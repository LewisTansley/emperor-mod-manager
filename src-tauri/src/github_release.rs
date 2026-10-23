//! GitHub release checking and download (app + tools).

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Deserialize)]
pub struct GhRelease {
    pub tag_name: String,
    #[allow(dead_code)]
    pub prerelease: bool,
    #[serde(default)]
    pub html_url: Option<String>,
    pub assets: Vec<GhAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GhAsset {
    pub name: String,
    pub browser_download_url: String,
}

fn api_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("emperor-mod-manager")
        .timeout(Duration::from_secs(15))
        .build()
        .context("building HTTP client")
}

fn download_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent("emperor-mod-manager")
        .timeout(Duration::from_secs(60 * 30))
        .build()
        .context("building download HTTP client")
}

pub async fn fetch_release_by_tag(repo: &str, tag: &str) -> Result<Option<GhRelease>> {
    let url = format!("https://api.github.com/repos/{repo}/releases/tags/{tag}");
    let client = api_client()?;
    let resp = client.get(&url).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(anyhow!("GitHub API error {} for {url}", resp.status()));
    }
    Ok(Some(resp.json().await?))
}

pub async fn fetch_latest_release(repo: &str) -> Result<Option<GhRelease>> {
    let url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let client = api_client()?;
    let resp = client.get(&url).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return fetch_all_releases(repo).await;
    }
    if !resp.status().is_success() {
        return Err(anyhow!("GitHub API error {} for {url}", resp.status()));
    }
    Ok(Some(resp.json().await?))
}

async fn fetch_all_releases(repo: &str) -> Result<Option<GhRelease>> {
    let url = format!("https://api.github.com/repos/{repo}/releases");
    let client = api_client()?;
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let releases: Vec<GhRelease> = resp.json().await?;
    Ok(releases.into_iter().next())
}

pub fn pick_linux_asset<'a>(release: &'a GhRelease, tool: &str) -> Option<&'a GhAsset> {
    let patterns: &[&str] = match tool {
        "lsfg_vk" => &[
            "linux.tar.xz",
            "x86_64.tar.xz",
            "x86_64-linux.tar.xz",
            "x86_64.tar.zst",
        ],
        "autohdr_vk" => &["linux.tar.xz", "x86_64.tar.xz", "x86_64-linux.tar.xz"],
        _ => &[],
    };
    for pat in patterns {
        if let Some(a) = release.assets.iter().find(|a| a.name.contains(pat)) {
            return Some(a);
        }
    }
    release
        .assets
        .iter()
        .find(|a| a.name.ends_with(".tar.xz") || a.name.ends_with(".tar.zst"))
}

/// How the running copy should be replaced. Serialized for the settings screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateChannel {
    Pacman,
    Deb,
    Appimage,
    Portable,
    Nsis,
}

/// Asset that matches the install channel, not merely the operating system.
pub fn pick_update_asset<'a>(
    release: &'a GhRelease,
    channel: UpdateChannel,
) -> Option<&'a GhAsset> {
    match channel {
        UpdateChannel::Pacman => release.assets.iter().find(|a| a.name == "PKGBUILD"),
        UpdateChannel::Deb => pick_deb(release),
        UpdateChannel::Appimage => pick_appimage(release),
        UpdateChannel::Portable => release.assets.iter().find(|a| is_release_tarball(&a.name)),
        UpdateChannel::Nsis => pick_nsis(release),
    }
}

fn pick_appimage(release: &GhRelease) -> Option<&GhAsset> {
    release
        .assets
        .iter()
        .find(|a| a.name.ends_with(".AppImage"))
        .or_else(|| {
            release
                .assets
                .iter()
                .find(|a| a.name.to_ascii_lowercase().contains("appimage"))
        })
}

fn pick_deb(release: &GhRelease) -> Option<&GhAsset> {
    release
        .assets
        .iter()
        .filter(|a| a.name.to_ascii_lowercase().ends_with(".deb"))
        .min_by_key(|a| {
            let n = a.name.to_ascii_lowercase();
            if n.contains("amd64") || n.contains("x86_64") {
                0
            } else {
                1
            }
        })
}

fn pick_nsis(release: &GhRelease) -> Option<&GhAsset> {
    let setup = release.assets.iter().find(|a| {
        let n = a.name.to_ascii_lowercase();
        n.ends_with("-setup.exe") || (n.contains("setup") && n.ends_with(".exe"))
    });
    setup.or_else(|| {
        release.assets.iter().find(|a| {
            let n = a.name.to_ascii_lowercase();
            n.ends_with(".exe") && n != "emperor-mod-manager.exe" && !n.ends_with(".msi.exe")
        })
    })
}

fn is_release_tarball(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.starts_with("emperor-mod-manager-") && n.ends_with("-x86_64.tar.gz")
}

/// Prefer AppImage on Linux and NSIS setup.exe on Windows.
pub fn pick_app_asset<'a>(release: &'a GhRelease, os: &str) -> Option<&'a GhAsset> {
    match os {
        "linux" => pick_update_asset(release, UpdateChannel::Appimage),
        "windows" => pick_update_asset(release, UpdateChannel::Nsis),
        _ => None,
    }
}

pub async fn download_bytes(url: &str) -> Result<Vec<u8>> {
    let client = download_client()?;
    let resp = client
        .get(url)
        .send()
        .await
        .context("downloading release asset")?;
    if !resp.status().is_success() {
        return Err(anyhow!("download failed: {}", resp.status()));
    }
    Ok(resp.bytes().await?.to_vec())
}

pub async fn download_to_path(url: &str, dest: &Path) -> Result<()> {
    let client = download_client()?;
    let resp = client
        .get(url)
        .send()
        .await
        .context("downloading release asset")?;
    if !resp.status().is_success() {
        return Err(anyhow!("download failed: {}", resp.status()));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = dest.with_extension(format!(
        "{}.part",
        dest.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("download")
    ));
    let mut file = tokio::fs::File::create(&tmp)
        .await
        .with_context(|| format!("creating {}", tmp.display()))?;
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("reading download stream")?;
        file.write_all(&chunk).await.context("writing download")?;
    }
    file.flush().await.context("flushing download")?;
    drop(file);
    if dest.exists() {
        std::fs::remove_file(dest).ok();
    }
    std::fs::rename(&tmp, dest)
        .with_context(|| format!("moving download to {}", dest.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pick_app_asset_prefers_appimage_and_setup() {
        let release = GhRelease {
            tag_name: "v0.4.0".into(),
            prerelease: false,
            html_url: None,
            assets: vec![
                GhAsset {
                    name: "emperor-mod-manager".into(),
                    browser_download_url: "https://example/bin".into(),
                },
                GhAsset {
                    name: "emperor-mod-manager_0.4.0_amd64.AppImage".into(),
                    browser_download_url: "https://example/app".into(),
                },
                GhAsset {
                    name: "emperor-mod-manager_0.4.0_amd64.deb".into(),
                    browser_download_url: "https://example/deb".into(),
                },
                GhAsset {
                    name: "emperor-mod-manager.exe".into(),
                    browser_download_url: "https://example/bare".into(),
                },
                GhAsset {
                    name: "Emperor Mod Manager_0.4.0_x64-setup.exe".into(),
                    browser_download_url: "https://example/setup".into(),
                },
            ],
        };
        assert_eq!(
            pick_app_asset(&release, "linux").map(|a| a.name.as_str()),
            Some("emperor-mod-manager_0.4.0_amd64.AppImage")
        );
        assert_eq!(
            pick_app_asset(&release, "windows").map(|a| a.name.as_str()),
            Some("Emperor Mod Manager_0.4.0_x64-setup.exe")
        );
    }

    fn sample_release() -> GhRelease {
        GhRelease {
            tag_name: "v0.4.0".into(),
            prerelease: false,
            html_url: None,
            assets: vec![
                GhAsset {
                    name: "PKGBUILD".into(),
                    browser_download_url: "https://example/pkgbuild".into(),
                },
                GhAsset {
                    name: "emperor-mod-manager-0.4.0-x86_64.tar.gz".into(),
                    browser_download_url: "https://example/tar".into(),
                },
                GhAsset {
                    name: "SHA256SUMS".into(),
                    browser_download_url: "https://example/sums".into(),
                },
                GhAsset {
                    name: "emperor-mod-manager_0.4.0_amd64.AppImage".into(),
                    browser_download_url: "https://example/app".into(),
                },
                GhAsset {
                    name: "Emperor.Mod.Manager_0.4.0_amd64.deb".into(),
                    browser_download_url: "https://example/deb".into(),
                },
                GhAsset {
                    name: "notes.deb.txt".into(),
                    browser_download_url: "https://example/not-deb".into(),
                },
                GhAsset {
                    name: "Emperor Mod Manager_0.4.0_x64-setup.exe".into(),
                    browser_download_url: "https://example/setup".into(),
                },
            ],
        }
    }

    #[test]
    fn pick_update_asset_follows_install_channel() {
        let release = sample_release();
        assert_eq!(
            pick_update_asset(&release, UpdateChannel::Pacman).map(|a| a.name.as_str()),
            Some("PKGBUILD")
        );
        assert_eq!(
            pick_update_asset(&release, UpdateChannel::Deb).map(|a| a.name.as_str()),
            Some("Emperor.Mod.Manager_0.4.0_amd64.deb")
        );
        assert_eq!(
            pick_update_asset(&release, UpdateChannel::Appimage).map(|a| a.name.as_str()),
            Some("emperor-mod-manager_0.4.0_amd64.AppImage")
        );
        assert_eq!(
            pick_update_asset(&release, UpdateChannel::Portable).map(|a| a.name.as_str()),
            Some("emperor-mod-manager-0.4.0-x86_64.tar.gz")
        );
        assert_eq!(
            pick_update_asset(&release, UpdateChannel::Nsis).map(|a| a.name.as_str()),
            Some("Emperor Mod Manager_0.4.0_x64-setup.exe")
        );
    }

    #[test]
    fn update_channel_serializes_for_settings() {
        assert_eq!(
            serde_json::to_string(&UpdateChannel::Appimage).unwrap(),
            "\"appimage\""
        );
        assert_eq!(
            serde_json::to_string(&UpdateChannel::Nsis).unwrap(),
            "\"nsis\""
        );
        assert_eq!(
            serde_json::to_string(&UpdateChannel::Pacman).unwrap(),
            "\"pacman\""
        );
    }

    #[test]
    fn pick_deb_prefers_amd64_over_an_earlier_deb() {
        let release = GhRelease {
            tag_name: "v0.4.0".into(),
            prerelease: false,
            html_url: None,
            assets: vec![
                GhAsset {
                    name: "Emperor.Mod.Manager_0.4.0_arm64.deb".into(),
                    browser_download_url: "https://example/arm".into(),
                },
                GhAsset {
                    name: "Emperor.Mod.Manager_0.4.0_amd64.deb".into(),
                    browser_download_url: "https://example/amd".into(),
                },
            ],
        };
        assert_eq!(
            pick_update_asset(&release, UpdateChannel::Deb).map(|a| a.name.as_str()),
            Some("Emperor.Mod.Manager_0.4.0_amd64.deb")
        );
    }
}
