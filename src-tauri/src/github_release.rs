//! GitHub release checking and download (app + tools).

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures_util::StreamExt;
use serde::Deserialize;
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

/// Prefer AppImage on Linux and NSIS setup.exe on Windows.
pub fn pick_app_asset<'a>(release: &'a GhRelease, os: &str) -> Option<&'a GhAsset> {
    match os {
        "linux" => release
            .assets
            .iter()
            .find(|a| a.name.ends_with(".AppImage"))
            .or_else(|| {
                release
                    .assets
                    .iter()
                    .find(|a| a.name.to_ascii_lowercase().contains("appimage"))
            }),
        "windows" => {
            let setup = release.assets.iter().find(|a| {
                let n = a.name.to_ascii_lowercase();
                n.ends_with("-setup.exe") || (n.contains("setup") && n.ends_with(".exe"))
            });
            setup.or_else(|| {
                release.assets.iter().find(|a| {
                    let n = a.name.to_ascii_lowercase();
                    n.ends_with(".exe")
                        && n != "emperor-mod-manager.exe"
                        && !n.ends_with(".msi.exe")
                })
            })
        }
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
}
