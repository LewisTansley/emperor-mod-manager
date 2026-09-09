//! In-app updates from the emperor-mod-manager GitHub Releases page.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::commands::AppState;
use crate::config::APP_VERSION;
use crate::github_release::{self, GhRelease};

const APP_REPO: &str = "LewisTansley/emperor-mod-manager";

#[derive(Debug, Clone, Serialize)]
pub struct AppUpdateStatus {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub asset_name: Option<String>,
    pub release_url: Option<String>,
}

fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let s = raw.trim().trim_start_matches('v');
    let base = s.split(['-', '+']).next().unwrap_or(s);
    let mut parts = base.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(l), Some(c)) => l > c,
        _ => {
            latest.trim().trim_start_matches('v') != current.trim().trim_start_matches('v')
        }
    }
}

fn normalize_tag(tag: &str) -> String {
    tag.trim().trim_start_matches('v').to_string()
}

fn release_url(release: &GhRelease) -> String {
    release.html_url.clone().unwrap_or_else(|| {
        format!(
            "https://github.com/{APP_REPO}/releases/tag/{}",
            release.tag_name
        )
    })
}

fn status_from_release(release: Option<&GhRelease>) -> AppUpdateStatus {
    let current_version = APP_VERSION.to_string();
    let Some(release) = release else {
        return AppUpdateStatus {
            current_version,
            latest_version: None,
            update_available: false,
            asset_name: None,
            release_url: None,
        };
    };
    let latest = normalize_tag(&release.tag_name);
    let update_available = is_newer(&latest, &current_version);
    let asset = github_release::pick_app_asset(release, std::env::consts::OS);
    AppUpdateStatus {
        current_version,
        latest_version: Some(latest),
        update_available,
        asset_name: asset.map(|a| a.name.clone()),
        release_url: Some(release_url(release)),
    }
}

#[tauri::command]
pub async fn check_app_update() -> Result<AppUpdateStatus, String> {
    let release = github_release::fetch_latest_release(APP_REPO)
        .await
        .map_err(|e| e.to_string())?;
    Ok(status_from_release(release.as_ref()))
}

#[derive(Debug, Clone, Serialize)]
pub struct AppInstallResult {
    pub message: String,
    pub path: Option<String>,
    pub will_exit: bool,
}

#[tauri::command]
pub async fn install_app_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<AppInstallResult, String> {
    let release = github_release::fetch_latest_release(APP_REPO)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No GitHub releases found for emperor-mod-manager".to_string())?;

    let latest = normalize_tag(&release.tag_name);
    if !is_newer(&latest, APP_VERSION) {
        return Err(format!(
            "Already on the latest version ({APP_VERSION}); nothing to install"
        ));
    }

    let os = std::env::consts::OS;
    let asset = github_release::pick_app_asset(&release, os).ok_or_else(|| {
        match os {
            "linux" => {
                "No .AppImage asset found on the latest GitHub release".to_string()
            }
            "windows" => {
                "No NSIS setup (.exe) asset found on the latest GitHub release".to_string()
            }
            other => format!("In-app updates are not supported on {other}"),
        }
    })?;

    let updates_dir = state.paths.cache_dir.join("app-updates");
    std::fs::create_dir_all(&updates_dir).map_err(|e| e.to_string())?;
    let dest = updates_dir.join(&asset.name);

    github_release::download_to_path(&asset.browser_download_url, &dest)
        .await
        .map_err(|e| e.to_string())?;

    apply_update(&app, &dest).await
}

async fn apply_update(app: &AppHandle, downloaded: &Path) -> Result<AppInstallResult, String> {
    #[cfg(target_os = "linux")]
    {
        return apply_linux(app, downloaded).await;
    }
    #[cfg(target_os = "windows")]
    {
        return apply_windows(app, downloaded);
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = app;
        Err(format!(
            "Downloaded to {}, but automatic install is not supported on this OS",
            downloaded.display()
        ))
    }
}

#[cfg(target_os = "linux")]
async fn apply_linux(app: &AppHandle, downloaded: &Path) -> Result<AppInstallResult, String> {
    use std::os::unix::fs::PermissionsExt;

    let meta = std::fs::metadata(downloaded).map_err(|e| e.to_string())?;
    let mut perms = meta.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(downloaded, perms).map_err(|e| e.to_string())?;

    if let Ok(appimage) = std::env::var("APPIMAGE") {
        let current = PathBuf::from(&appimage);
        let parent = current
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let file_name = current
            .file_name()
            .ok_or_else(|| "Invalid APPIMAGE path".to_string())?;
        let staging = parent.join(format!(
            "{}.new",
            file_name.to_string_lossy()
        ));
        let backup = parent.join(format!(
            "{}.old",
            file_name.to_string_lossy()
        ));

        std::fs::copy(downloaded, &staging).map_err(|e| e.to_string())?;
        let meta = std::fs::metadata(&staging).map_err(|e| e.to_string())?;
        let mut perms = meta.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&staging, perms).map_err(|e| e.to_string())?;

        if backup.exists() {
            std::fs::remove_file(&backup).ok();
        }
        if current.exists() {
            std::fs::rename(&current, &backup).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&staging, &current).map_err(|e| {
            // Best-effort rollback
            let _ = std::fs::rename(&backup, &current);
            e.to_string()
        })?;

        Command::new(&current)
            .spawn()
            .map_err(|e| format!("Failed to launch updated AppImage: {e}"))?;

        let path = current.display().to_string();
        schedule_exit(app);
        return Ok(AppInstallResult {
            message: "Update installed. Relaunching…".into(),
            path: Some(path),
            will_exit: true,
        });
    }

    // Not running as AppImage: open the downloaded file for the user.
    tauri_plugin_opener::open_path(downloaded, None::<&str>)
        .map_err(|e| format!("Downloaded update but failed to open it: {e}"))?;

    Ok(AppInstallResult {
        message: format!(
            "Downloaded {}. Opened the new AppImage — replace your existing install, then relaunch.",
            downloaded
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| downloaded.display().to_string())
        ),
        path: Some(downloaded.display().to_string()),
        will_exit: false,
    })
}

#[cfg(target_os = "windows")]
fn apply_windows(app: &AppHandle, downloaded: &Path) -> Result<AppInstallResult, String> {
    use std::os::windows::process::CommandExt;

    // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP — survive parent exit.
    const DETACHED: u32 = 0x00000008 | 0x00000200;

    Command::new(downloaded)
        .creation_flags(DETACHED)
        .spawn()
        .map_err(|e| format!("Failed to start installer: {e}"))?;

    let path = downloaded.display().to_string();
    schedule_exit(app);
    Ok(AppInstallResult {
        message: "Installer started. Emperor Mod Manager will close so files can be replaced."
            .into(),
        path: Some(path),
        will_exit: true,
    })
}

fn schedule_exit(app: &AppHandle) {
    let handle = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        handle.exit(0);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_detects_semver() {
        assert!(is_newer("0.4.0", "0.3.0"));
        assert!(is_newer("v0.3.1", "0.3.0"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
        assert!(is_newer("1.0.0-beta", "0.9.0"));
    }
}
