//! In-app updates from the emperor-mod-manager GitHub Releases page.
//!
//! The running binary decides the channel: an AppImage is replaced in place, a
//! pacman or apt package is installed with polkit, and a user-writable copy is
//! swapped from the release tarball. Anything else is left alone.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::commands::AppState;
use crate::config::APP_VERSION;
use crate::github_release::{self, GhRelease, UpdateChannel};

const APP_REPO: &str = "LewisTansley/emperor-mod-manager";
const ARCH_PKGNAME: &str = "emperor-mod-manager-bin";
const REPO_URL: &str = "https://github.com/LewisTansley/emperor-mod-manager";
const PROGRESS_EVENT: &str = "app-update-progress";

#[derive(Debug, Clone, Serialize)]
pub struct AppUpdateStatus {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub asset_name: Option<String>,
    pub release_url: Option<String>,
    pub install_via: Option<UpdateChannel>,
}

#[derive(Debug, Clone)]
struct InstallProbe {
    appimage_env: Option<String>,
    exe_path: PathBuf,
    exe_writable: bool,
    pacman_owner: Option<String>,
    dpkg_owner: Option<String>,
}

/// Classify the running install. AppImage wins, then the package that owns the
/// binary, then a copy the user can overwrite themselves.
fn classify_install(probe: &InstallProbe) -> Option<UpdateChannel> {
    let appimage_env = probe.appimage_env.as_ref().is_some_and(|s| !s.is_empty());
    let appimage_file = probe
        .exe_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("AppImage"));
    if appimage_env || appimage_file {
        return Some(UpdateChannel::Appimage);
    }
    if probe.pacman_owner.as_deref() == Some(ARCH_PKGNAME) {
        return Some(UpdateChannel::Pacman);
    }
    if probe.dpkg_owner.as_ref().is_some_and(|s| !s.is_empty()) {
        return Some(UpdateChannel::Deb);
    }
    if probe.exe_writable {
        return Some(UpdateChannel::Portable);
    }
    None
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
        _ => latest.trim().trim_start_matches('v') != current.trim().trim_start_matches('v'),
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

fn status_from_release(
    release: Option<&GhRelease>,
    install_via: Option<UpdateChannel>,
) -> AppUpdateStatus {
    let current_version = APP_VERSION.to_string();
    let Some(release) = release else {
        return AppUpdateStatus {
            current_version,
            latest_version: None,
            update_available: false,
            asset_name: None,
            release_url: None,
            install_via,
        };
    };
    let latest = normalize_tag(&release.tag_name);
    let update_available = is_newer(&latest, &current_version);
    let asset = install_via.and_then(|via| github_release::pick_update_asset(release, via));
    AppUpdateStatus {
        current_version,
        latest_version: Some(latest),
        update_available,
        asset_name: asset.map(|a| a.name.clone()),
        release_url: Some(release_url(release)),
        install_via,
    }
}

fn missing_asset(via: UpdateChannel) -> String {
    match via {
        UpdateChannel::Pacman => "No PKGBUILD asset found on the latest GitHub release".into(),
        UpdateChannel::Deb => "No .deb asset found on the latest GitHub release".into(),
        UpdateChannel::Appimage => "No .AppImage asset found on the latest GitHub release".into(),
        UpdateChannel::Portable => "No Linux tarball found on the latest GitHub release".into(),
        UpdateChannel::Nsis => {
            "No NSIS setup (.exe) asset found on the latest GitHub release".into()
        }
    }
}

fn emit_progress(app: &AppHandle, message: &str) {
    let _ = app.emit(PROGRESS_EVENT, message.to_string());
}

fn current_install_channel() -> Option<UpdateChannel> {
    #[cfg(target_os = "windows")]
    {
        return Some(UpdateChannel::Nsis);
    }
    #[cfg(target_os = "linux")]
    {
        return classify_install(&probe_current());
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn probe_current() -> InstallProbe {
    let exe_path = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("emperor-mod-manager"));
    InstallProbe {
        appimage_env: std::env::var("APPIMAGE").ok().filter(|s| !s.is_empty()),
        exe_writable: is_user_writable(&exe_path),
        pacman_owner: query_pacman_owner(&exe_path),
        dpkg_owner: query_dpkg_owner(&exe_path),
        exe_path,
    }
}

#[cfg(target_os = "linux")]
fn current_euid() -> Option<u32> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in status.lines() {
        let Some(rest) = line.strip_prefix("Uid:") else {
            continue;
        };
        // real, effective, saved, filesystem
        return rest.split_whitespace().nth(1)?.parse().ok();
    }
    None
}

#[cfg(target_os = "linux")]
fn is_user_writable(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    let mode = meta.mode();
    if mode & 0o002 != 0 {
        return true;
    }
    current_euid().is_some_and(|uid| meta.uid() == uid && mode & 0o200 != 0)
}

#[cfg(target_os = "linux")]
fn query_pacman_owner(exe: &Path) -> Option<String> {
    let output = Command::new("pacman")
        .args(["-Qo", &exe.display().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let rest = text.split("owned by ").nth(1)?;
    let name = rest.split_whitespace().next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

#[cfg(target_os = "linux")]
fn query_dpkg_owner(exe: &Path) -> Option<String> {
    let output = Command::new("dpkg")
        .args(["-S", &exe.display().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let name = text.split(':').next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Refuse a PKGBUILD that is not this app's release package. `makepkg` still
/// verifies the tarball checksum published in the release asset.
fn validate_pkgbuild(text: &str) -> Result<(), String> {
    let pkgname_ok = text.lines().any(|line| {
        matches!(
            line.trim(),
            "pkgname=emperor-mod-manager-bin"
                | "pkgname='emperor-mod-manager-bin'"
                | "pkgname=\"emperor-mod-manager-bin\""
        )
    });
    if !pkgname_ok {
        return Err("PKGBUILD pkgname is not emperor-mod-manager-bin".into());
    }

    let url_ok = text.lines().any(|line| {
        let line = line.trim();
        line.starts_with("url=") && line.contains(REPO_URL)
    });
    if !url_ok {
        return Err("PKGBUILD url is not the emperor-mod-manager repository".into());
    }

    let source = text
        .lines()
        .find(|line| line.trim().starts_with("source="))
        .ok_or_else(|| "PKGBUILD has no source".to_string())?;
    let uses_release = source.contains("LewisTansley/emperor-mod-manager")
        || source.contains("$url/releases/download/");
    if !uses_release {
        return Err("PKGBUILD source is not the GitHub release tarball".into());
    }
    if let Some(url_at) = source.find("https://").or_else(|| source.find("http://")) {
        let rest = &source[url_at..];
        if !rest.starts_with(REPO_URL) {
            return Err("PKGBUILD source URL is not the emperor-mod-manager repository".into());
        }
    }
    Ok(())
}

fn asset_file_name(name: &str) -> String {
    Path::new(name)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty() && n != "." && n != "..")
        .unwrap_or_else(|| "update-asset".into())
}

#[tauri::command]
pub async fn check_app_update() -> Result<AppUpdateStatus, String> {
    let release = github_release::fetch_latest_release(APP_REPO)
        .await
        .map_err(|e| e.to_string())?;
    Ok(status_from_release(
        release.as_ref(),
        current_install_channel(),
    ))
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
    let via = current_install_channel().ok_or_else(|| {
        "This install cannot be updated from inside the app. It is not an AppImage, a pacman or apt package, or a user-writable copy.".to_string()
    })?;

    emit_progress(&app, "Downloading…");

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

    let asset =
        github_release::pick_update_asset(&release, via).ok_or_else(|| missing_asset(via))?;

    let updates_dir = state.paths.cache_dir.join("app-updates").join(&latest);
    std::fs::create_dir_all(&updates_dir).map_err(|e| e.to_string())?;
    let dest = updates_dir.join(asset_file_name(&asset.name));

    github_release::download_to_path(&asset.browser_download_url, &dest)
        .await
        .map_err(|e| e.to_string())?;

    dispatch_install(&app, via, &updates_dir, &dest).await
}

async fn dispatch_install(
    app: &AppHandle,
    via: UpdateChannel,
    workdir: &Path,
    downloaded: &Path,
) -> Result<AppInstallResult, String> {
    #[cfg(target_os = "windows")]
    {
        let _ = workdir;
        if via != UpdateChannel::Nsis {
            return Err("In-app updates on Windows use the NSIS installer.".into());
        }
        return apply_windows(app, downloaded);
    }
    #[cfg(target_os = "linux")]
    {
        return match via {
            UpdateChannel::Pacman => apply_arch(app, workdir, downloaded).await,
            UpdateChannel::Deb => apply_deb(app, downloaded).await,
            UpdateChannel::Appimage => apply_appimage(app, downloaded).await,
            UpdateChannel::Portable => apply_portable(app, downloaded).await,
            UpdateChannel::Nsis => Err("The Windows installer cannot run on Linux.".into()),
        };
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        let _ = (app, via, workdir, downloaded);
        Err("In-app updates are not supported on this OS".into())
    }
}

#[cfg(target_os = "linux")]
async fn apply_arch(
    app: &AppHandle,
    workdir: &Path,
    pkgbuild: &Path,
) -> Result<AppInstallResult, String> {
    if current_euid() == Some(0) {
        return Err(
            "Run Emperor Mod Manager as your normal user. makepkg refuses to run as root.".into(),
        );
    }
    let text = std::fs::read_to_string(pkgbuild).map_err(|e| e.to_string())?;
    validate_pkgbuild(&text)?;
    if pkgbuild.file_name().and_then(|n| n.to_str()) != Some("PKGBUILD") {
        return Err("Downloaded asset is not named PKGBUILD".into());
    }
    // Capture the path before pacman replaces the inode. /proc/self/exe then
    // points at the deleted binary and cannot be spawned.
    let relaunch_at = running_exe()?;

    ensure_base_devel(app).await?;
    clear_previous_packages(workdir)?;

    emit_progress(app, "Building package…");
    let build_dir = workdir.to_path_buf();
    blocking(move || {
        // --nodeps keeps the build from calling sudo. pacman -U installs
        // missing runtime dependencies when the package is added.
        // PKGDEST is pinned so a system makepkg.conf cannot hide the package.
        let mut cmd = Command::new("makepkg");
        cmd.args(["--noconfirm", "--clean", "--nodeps"])
            .current_dir(&build_dir)
            .env("PKGDEST", &build_dir);
        run_command(cmd, "makepkg")
    })
    .await?;

    let package = find_pkg_archive(workdir)?;
    let installable = copy_world_readable(&package)?;
    emit_progress(app, "Waiting for administrator approval…");
    let package_path = installable.display().to_string();
    blocking(move || elevate("pacman", &["-U".into(), "--noconfirm".into(), package_path])).await?;
    let _ = std::fs::remove_file(&installable);

    relaunch(app, &relaunch_at)?;
    Ok(AppInstallResult {
        message: "Update installed with pacman. Relaunching…".into(),
        path: Some(relaunch_at.display().to_string()),
        will_exit: true,
    })
}

#[cfg(target_os = "linux")]
async fn ensure_base_devel(app: &AppHandle) -> Result<(), String> {
    if ["makepkg", "fakeroot", "bsdtar"]
        .iter()
        .all(|tool| command_on_path(tool))
    {
        return Ok(());
    }
    emit_progress(app, "Waiting for administrator approval…");
    blocking(|| {
        elevate(
            "pacman",
            &[
                "-S".into(),
                "--needed".into(),
                "--noconfirm".into(),
                "base-devel".into(),
            ],
        )
    })
    .await?;
    if ["makepkg", "fakeroot", "bsdtar"]
        .iter()
        .all(|tool| command_on_path(tool))
    {
        return Ok(());
    }
    Err(
        "base-devel is still missing after installation (need makepkg, fakeroot, and bsdtar)."
            .into(),
    )
}

#[cfg(target_os = "linux")]
fn clear_previous_packages(dir: &Path) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_pkg_archive(&name) {
            std::fs::remove_file(entry.path()).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn is_pkg_archive(name: &str) -> bool {
    name.contains(".pkg.tar.") && !name.ends_with(".sig")
}

#[cfg(target_os = "linux")]
fn find_pkg_archive(dir: &Path) -> Result<PathBuf, String> {
    let entries = std::fs::read_dir(dir).map_err(|e| e.to_string())?;
    let mut packages = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_pkg_archive(&name) {
            packages.push((name, entry.path()));
        }
    }
    let non_debug: Vec<_> = packages
        .iter()
        .filter(|(name, _)| !name.contains("-debug-"))
        .collect();
    if non_debug.len() == 1 {
        return Ok(non_debug[0].1.clone());
    }
    if packages.len() == 1 {
        return Ok(packages[0].1.clone());
    }
    Err(format!(
        "expected one pacman package in {}, found {}",
        dir.display(),
        packages.len()
    ))
}

#[cfg(target_os = "linux")]
async fn apply_deb(app: &AppHandle, deb: &Path) -> Result<AppInstallResult, String> {
    let relaunch_at = running_exe()?;
    let installable = copy_world_readable(deb)?;
    emit_progress(app, "Waiting for administrator approval…");
    let package_path = installable.display().to_string();
    blocking(move || {
        elevate(
            "apt-get",
            &[
                "install".into(),
                "-y".into(),
                "-o".into(),
                "Dpkg::Options::=--force-confdef".into(),
                "-o".into(),
                "Dpkg::Options::=--force-confold".into(),
                package_path,
            ],
        )
    })
    .await?;
    let _ = std::fs::remove_file(&installable);

    relaunch(app, &relaunch_at)?;
    Ok(AppInstallResult {
        message: "Update installed with apt. Relaunching…".into(),
        path: Some(relaunch_at.display().to_string()),
        will_exit: true,
    })
}

#[cfg(target_os = "linux")]
async fn apply_appimage(app: &AppHandle, downloaded: &Path) -> Result<AppInstallResult, String> {
    let current = appimage_path().ok_or_else(|| {
        "Could not find the AppImage path to replace. Launch the AppImage directly and try again."
            .to_string()
    })?;
    replace_executable(&current, downloaded)?;
    relaunch(app, &current)?;
    Ok(AppInstallResult {
        message: "Update installed. Relaunching…".into(),
        path: Some(current.display().to_string()),
        will_exit: true,
    })
}

#[cfg(target_os = "linux")]
fn appimage_path() -> Option<PathBuf> {
    if let Some(raw) = std::env::var_os("APPIMAGE") {
        let path = PathBuf::from(raw);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    let exe = std::env::current_exe().ok()?;
    if exe
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("AppImage"))
    {
        return Some(exe);
    }
    None
}

#[cfg(target_os = "linux")]
async fn apply_portable(app: &AppHandle, tarball: &Path) -> Result<AppInstallResult, String> {
    let current = running_exe()?;
    if !is_user_writable(&current) {
        return Err(format!(
            "Cannot replace {} — it is not writable by this user.",
            current.display()
        ));
    }
    let extracted = tarball.with_file_name("emperor-mod-manager.next");
    extract_release_binary(tarball, &extracted)?;
    replace_executable(&current, &extracted)?;
    relaunch(app, &current)?;
    Ok(AppInstallResult {
        message: "Update installed. Relaunching…".into(),
        path: Some(current.display().to_string()),
        will_exit: true,
    })
}

/// Pull `emperor-mod-manager` out of `emperor-mod-manager-<version>-x86_64.tar.gz`.
fn extract_release_binary(archive_path: &Path, dest: &Path) -> Result<(), String> {
    let file = std::fs::File::open(archive_path).map_err(|e| e.to_string())?;
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|e| format!("reading tarball: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("reading tarball entry: {e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry
            .path()
            .map_err(|e| format!("reading tarball path: {e}"))?;
        let name_ok = path.file_name().and_then(|n| n.to_str()) == Some("emperor-mod-manager");
        if !name_ok || path.components().count() != 2 {
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        entry
            .unpack(dest)
            .map_err(|e| format!("unpacking binary: {e}"))?;
        return Ok(());
    }
    Err("Release tarball does not contain emperor-mod-manager".into())
}

#[cfg(target_os = "linux")]
fn replace_executable(current: &Path, downloaded: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let parent = current
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let file_name = current
        .file_name()
        .ok_or_else(|| "Invalid install path".to_string())?;
    let staging = parent.join(format!("{}.new", file_name.to_string_lossy()));
    let backup = parent.join(format!("{}.old", file_name.to_string_lossy()));

    std::fs::copy(downloaded, &staging).map_err(|e| e.to_string())?;
    let mut perms = std::fs::metadata(&staging)
        .map_err(|e| e.to_string())?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&staging, perms).map_err(|e| e.to_string())?;

    if backup.exists() {
        std::fs::remove_file(&backup).ok();
    }
    if current.exists() {
        std::fs::rename(current, &backup).map_err(|e| e.to_string())?;
    }
    if let Err(e) = std::fs::rename(&staging, current) {
        let _ = std::fs::rename(&backup, current);
        return Err(e.to_string());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn running_exe() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("Could not resolve the running binary: {e}"))
}

#[cfg(target_os = "linux")]
fn relaunch(app: &AppHandle, binary: &Path) -> Result<(), String> {
    emit_progress(app, "Relaunching…");
    Command::new(binary)
        .spawn()
        .map_err(|e| format!("Failed to relaunch {}: {e}", binary.display()))?;
    schedule_exit(app);
    Ok(())
}

/// Root can read this even when the cache directory is mode 0700.
#[cfg(target_os = "linux")]
fn copy_world_readable(src: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;

    let name = src
        .file_name()
        .ok_or_else(|| "Update package has no file name".to_string())?;
    let dest = std::env::temp_dir().join(format!(
        "emm-update-{}-{}",
        std::process::id(),
        name.to_string_lossy()
    ));
    std::fs::copy(src, &dest).map_err(|e| e.to_string())?;
    let mut perms = std::fs::metadata(&dest)
        .map_err(|e| e.to_string())?
        .permissions();
    perms.set_mode(0o644);
    std::fs::set_permissions(&dest, perms).map_err(|e| e.to_string())?;
    Ok(dest)
}

#[cfg(target_os = "linux")]
fn command_on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(name).is_file())
}

#[cfg(target_os = "linux")]
fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

#[cfg(target_os = "linux")]
fn elevate(program: &str, args: &[String]) -> Result<(), String> {
    let sudo_hint = format!(
        "sudo {} {}",
        shell_quote(program),
        args.iter()
            .map(|arg| shell_quote(arg))
            .collect::<Vec<_>>()
            .join(" ")
    );
    if command_on_path("pkexec") {
        return run_elevated("pkexec", program, args, &sudo_hint);
    }
    if command_on_path("run0") {
        return run_elevated("run0", program, args, &sudo_hint);
    }
    Err(format!(
        "Could not ask for administrator permission (pkexec and run0 are not installed). Run: {sudo_hint}"
    ))
}

#[cfg(target_os = "linux")]
fn run_elevated(
    elevator: &str,
    program: &str,
    args: &[String],
    sudo_hint: &str,
) -> Result<(), String> {
    let output = Command::new(elevator)
        .arg(program)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to start {elevator}: {e}. Run: {sudo_hint}"))?;
    if output.status.success() {
        return Ok(());
    }
    let code = output.status.code().unwrap_or(-1);
    if code == 126 || code == 127 {
        return Err(format!(
            "Administrator approval was denied. You can retry, or run: {sudo_hint}"
        ));
    }
    let detail = error_tail(&String::from_utf8_lossy(&output.stderr));
    let detail = if detail.is_empty() {
        error_tail(&String::from_utf8_lossy(&output.stdout))
    } else {
        detail
    };
    if detail.is_empty() {
        Err(format!(
            "{elevator} {program} failed ({code}). Run: {sudo_hint}"
        ))
    } else {
        Err(format!(
            "{elevator} {program} failed ({code}): {detail}. Run: {sudo_hint}"
        ))
    }
}

#[cfg(target_os = "linux")]
fn run_command(mut cmd: Command, label: &str) -> Result<(), String> {
    let output = cmd
        .output()
        .map_err(|e| format!("Failed to start {label}: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let detail = error_tail(&String::from_utf8_lossy(&output.stderr));
    let detail = if detail.is_empty() {
        error_tail(&String::from_utf8_lossy(&output.stdout))
    } else {
        detail
    };
    Err(format!(
        "{label} exited with {}: {detail}",
        output.status.code().unwrap_or(-1)
    ))
}

#[cfg(target_os = "linux")]
fn error_tail(text: &str) -> String {
    const MAX: usize = 600;
    let text = text.trim();
    if text.len() <= MAX {
        return text.to_string();
    }
    let mut start = text.len() - MAX;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

#[cfg(target_os = "linux")]
async fn blocking<T, F>(f: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("update task failed: {e}"))?
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
    use crate::github_release::{GhAsset, GhRelease};

    fn probe(
        path: &str,
        appimage: Option<&str>,
        writable: bool,
        pacman: Option<&str>,
        dpkg: Option<&str>,
    ) -> InstallProbe {
        InstallProbe {
            appimage_env: appimage.map(str::to_string),
            exe_path: PathBuf::from(path),
            exe_writable: writable,
            pacman_owner: pacman.map(str::to_string),
            dpkg_owner: dpkg.map(str::to_string),
        }
    }

    #[test]
    fn classify_prefers_appimage_over_package_managers() {
        let found = classify_install(&probe(
            "/usr/bin/emperor-mod-manager",
            Some("/opt/Emperor.AppImage"),
            false,
            Some(ARCH_PKGNAME),
            Some("emperor-mod-manager"),
        ));
        assert_eq!(found, Some(UpdateChannel::Appimage));
    }

    #[test]
    fn classify_appimage_suffix_without_env() {
        let found = classify_install(&probe(
            "/home/user/Emperor.Mod.Manager.AppImage",
            None,
            true,
            None,
            None,
        ));
        assert_eq!(found, Some(UpdateChannel::Appimage));
    }

    #[test]
    fn classify_pacman_package() {
        let found = classify_install(&probe(
            "/usr/bin/emperor-mod-manager",
            None,
            false,
            Some(ARCH_PKGNAME),
            None,
        ));
        assert_eq!(found, Some(UpdateChannel::Pacman));
    }

    #[test]
    fn classify_ignores_unrelated_pacman_owner() {
        let found = classify_install(&probe(
            "/usr/bin/emperor-mod-manager",
            None,
            false,
            Some("some-other-package"),
            Some("emperor-mod-manager"),
        ));
        assert_eq!(found, Some(UpdateChannel::Deb));
    }

    #[test]
    fn classify_deb_package() {
        let found = classify_install(&probe(
            "/usr/bin/emperor-mod-manager",
            None,
            false,
            None,
            Some("emperor-mod-manager"),
        ));
        assert_eq!(found, Some(UpdateChannel::Deb));
    }

    #[test]
    fn classify_writable_copy_is_portable() {
        let found = classify_install(&probe(
            "/home/user/apps/emperor-mod-manager",
            None,
            true,
            None,
            None,
        ));
        assert_eq!(found, Some(UpdateChannel::Portable));
    }

    #[test]
    fn classify_system_binary_without_owner_is_unsupported() {
        let found = classify_install(&probe(
            "/usr/local/bin/emperor-mod-manager",
            Some(""),
            false,
            None,
            Some(""),
        ));
        assert_eq!(found, None);
    }

    #[test]
    fn newer_detects_semver() {
        assert!(is_newer("0.4.0", "0.3.0"));
        assert!(is_newer("v0.3.1", "0.3.0"));
        assert!(!is_newer("0.3.0", "0.3.0"));
        assert!(!is_newer("0.2.9", "0.3.0"));
        assert!(is_newer("1.0.0-beta", "0.9.0"));
    }

    const VALID_PKGBUILD: &str = r#"
pkgname=emperor-mod-manager-bin
url="https://github.com/LewisTansley/emperor-mod-manager"
source=("$pkgname-$pkgver.tar.gz::$url/releases/download/v$pkgver/emperor-mod-manager-$pkgver-x86_64.tar.gz")
"#;

    #[test]
    fn pkgbuild_accepts_release_template() {
        validate_pkgbuild(VALID_PKGBUILD).unwrap();
        validate_pkgbuild("pkgname=\"emperor-mod-manager-bin\"\nurl=\"https://github.com/LewisTansley/emperor-mod-manager\"\nsource=(\"$url/releases/download/v1/x.tar.gz\")\n").unwrap();
    }

    #[test]
    fn pkgbuild_rejects_wrong_pkgname() {
        let text = VALID_PKGBUILD.replace("pkgname=emperor-mod-manager-bin", "pkgname=evil");
        assert!(validate_pkgbuild(&text).is_err());
    }

    #[test]
    fn pkgbuild_rejects_wrong_url() {
        let text = VALID_PKGBUILD.replace(REPO_URL, "https://github.com/evil/emperor-mod-manager");
        assert!(validate_pkgbuild(&text).is_err());
    }

    #[test]
    fn pkgbuild_rejects_foreign_source_url() {
        let text = r#"
pkgname=emperor-mod-manager-bin
url="https://github.com/LewisTansley/emperor-mod-manager"
source=("https://evil.example/emperor-mod-manager.tar.gz")
"#;
        assert!(validate_pkgbuild(text).is_err());
    }

    #[test]
    fn status_uses_the_channel_asset() {
        let release = GhRelease {
            tag_name: "v99.0.0".into(),
            prerelease: false,
            html_url: None,
            assets: vec![
                GhAsset {
                    name: "PKGBUILD".into(),
                    browser_download_url: "https://example/pkgbuild".into(),
                },
                GhAsset {
                    name: "emperor-mod-manager_99.0.0_amd64.AppImage".into(),
                    browser_download_url: "https://example/app".into(),
                },
            ],
        };
        let status = status_from_release(Some(&release), Some(UpdateChannel::Pacman));
        assert!(status.update_available);
        assert_eq!(status.asset_name.as_deref(), Some("PKGBUILD"));
        assert_eq!(status.install_via, Some(UpdateChannel::Pacman));
    }

    #[test]
    fn extract_release_binary_finds_nested_binary() {
        let dir = tempfile::tempdir().unwrap();
        let archive_path = dir.path().join("pkg.tar.gz");
        let file = std::fs::File::create(&archive_path).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        {
            let mut builder = tar::Builder::new(&mut encoder);
            let bytes = b"#!/bin/true\n";
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(
                    &mut header,
                    "emperor-mod-manager-1.0.0-x86_64/emperor-mod-manager",
                    &bytes[..],
                )
                .unwrap();
            let mut license = tar::Header::new_gnu();
            license.set_size(1);
            license.set_cksum();
            builder
                .append_data(
                    &mut license,
                    "emperor-mod-manager-1.0.0-x86_64/LICENSE",
                    &b"x"[..],
                )
                .unwrap();
            builder.finish().unwrap();
        }
        encoder.finish().unwrap();

        let dest = dir.path().join("out-bin");
        extract_release_binary(&archive_path, &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"#!/bin/true\n");
    }
}
