//! Download a missing BepInEx loader before deploy.

use std::path::Path;

use anyhow::{anyhow, Result};

use super::unity_bepinex::{bepinex_cache_ready, copy_bepinex_tree, BepInExBuild};
use crate::config::Paths;
use crate::github_release;
use crate::thunderstore::ThunderstoreClient;
use crate::tools::install as tool_install;

/// Install one chosen BepInEx build. A download failure is a warning.
pub async fn ensure_bepinex(
    paths: &Paths,
    install_path: &Path,
    build: BepInExBuild,
) -> Vec<String> {
    match install_build(paths, install_path, build).await {
        Ok(message) => vec![message],
        Err(e) => vec![format!("Could not auto-install BepInEx: {e:#}")],
    }
}

async fn install_build(paths: &Paths, install_path: &Path, build: BepInExBuild) -> Result<String> {
    let cache = paths.bepinex_cache_dir(&build.cache_id());
    if !bepinex_cache_ready(&cache) {
        download_build(&cache, build).await?;
    }
    if !bepinex_cache_ready(&cache) {
        anyhow::bail!("downloaded BepInEx archive did not contain a loader");
    }
    copy_bepinex_tree(&cache, install_path)?;
    let mut message = match build {
        BepInExBuild::Github { label, .. } => format!("Installed {label}."),
        BepInExBuild::Thunderstore {
            namespace, name, ..
        } => format!("Installed BepInEx pack {namespace}-{name}."),
    };
    if matches!(build, BepInExBuild::Github { il2cpp: true, .. }) {
        message.push_str(" The first launch generates interop assemblies.");
    }
    Ok(message)
}

async fn download_build(cache: &Path, build: BepInExBuild) -> Result<()> {
    let (bytes, name) = match build {
        BepInExBuild::Github { tag, asset, .. } => {
            let release = github_release::fetch_release_by_tag(build.repo(), tag)
                .await?
                .ok_or_else(|| anyhow!("GitHub release {tag} was not found"))?;
            let found = release
                .assets
                .iter()
                .find(|item| item.name == asset)
                .ok_or_else(|| anyhow!("release {tag} has no {asset}"))?;
            let bytes = github_release::download_bytes(&found.browser_download_url).await?;
            (bytes, found.name.clone())
        }
        BepInExBuild::Thunderstore {
            community,
            namespace,
            name,
            ..
        } => {
            let client = ThunderstoreClient::new()?;
            let detail = client.get_package(community, namespace, name).await?;
            let version = detail.versions.first().ok_or_else(|| {
                anyhow!("BepInEx pack {namespace}-{name} has no published versions")
            })?;
            let bytes = github_release::download_bytes(&version.download_url).await?;
            (
                bytes,
                format!("{namespace}-{name}-{}.zip", version.version_number),
            )
        }
    };
    tool_install::extract_archive(&bytes, &name, cache)?;
    Ok(())
}
