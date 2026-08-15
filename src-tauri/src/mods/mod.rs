//! Mod staging, extraction, load order, and deploy.

use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::{
    config::Paths,
    games::{normalize_relative, normalize_staging_root, plugin_by_id, GamePlugin},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedMod {
    pub id: String,
    pub name: String,
    pub nexus_mod_id: u64,
    pub nexus_file_id: u64,
    pub version: Option<String>,
    pub domain: String,
    pub staging_path: String,
    pub enabled: bool,
    pub order: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoadOrder {
    pub mods: Vec<StagedMod>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeployManifest {
    /// Absolute paths we created/linked during last deploy.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployResult {
    pub file_count: usize,
    pub enabled_mods: usize,
    pub warnings: Vec<String>,
}

pub fn load_loadorder(paths: &Paths, game_id: &str) -> Result<LoadOrder> {
    let file = paths.loadorder_file(game_id);
    if !file.exists() {
        return Ok(LoadOrder::default());
    }
    let raw = fs::read_to_string(&file)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save_loadorder(paths: &Paths, game_id: &str, order: &LoadOrder) -> Result<()> {
    crate::config::ensure_game_dirs(paths, game_id)?;
    let file = paths.loadorder_file(game_id);
    let raw = serde_json::to_string_pretty(order)?;
    fs::write(file, raw)?;
    Ok(())
}

pub fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;

    let ext = archive
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "zip" => extract_zip(archive, dest),
        "7z" => extract_7z(archive, dest),
        "rar" => extract_rar(archive, dest),
        other => {
            // Some Nexus files have no extension or odd names — try zip, 7z, then RAR magic
            if extract_zip(archive, dest).is_ok() {
                Ok(())
            } else if extract_7z(archive, dest).is_ok() {
                Ok(())
            } else if looks_like_rar(archive) {
                extract_rar(archive, dest)
            } else {
                bail!("unsupported archive type: .{other}");
            }
        }
    }
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(archive)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = match file.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue,
        };
        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
    }
    Ok(())
}

fn extract_7z(archive: &Path, dest: &Path) -> Result<()> {
    sevenz_rust::decompress_file(archive, dest).map_err(|e| anyhow::anyhow!("7z extract: {e}"))
}

/// RAR signature: `Rar!` followed by `0x1A 0x07` (RAR 1.5+ / RAR 5).
fn looks_like_rar(archive: &Path) -> bool {
    let Ok(mut file) = fs::File::open(archive) else {
        return false;
    };
    let mut magic = [0u8; 6];
    match file.read(&mut magic) {
        Ok(n) if n >= 6 => {}
        _ => return false,
    }
    magic[0..4] == *b"Rar!" && magic[4] == 0x1a && magic[5] == 0x07
}

fn extract_rar(archive: &Path, dest: &Path) -> Result<()> {
    let archive = unrar_ng::Archive::new(archive)
        .open_for_processing()
        .map_err(|e| anyhow::anyhow!("rar open: {e}"))?;
    archive
        .extract_all(dest)
        .map_err(|e| anyhow::anyhow!("rar extract: {e}"))?;
    Ok(())
}

pub fn stage_mod(
    paths: &Paths,
    game_id: &str,
    name: &str,
    domain: &str,
    mod_id: u64,
    file_id: u64,
    version: Option<String>,
    archive: &Path,
) -> Result<StagedMod> {
    crate::config::ensure_game_dirs(paths, game_id)?;
    let safe = sanitize_filename::sanitize(name);
    let staging = paths
        .mods_dir(game_id)
        .join(format!("{safe}_{mod_id}_{file_id}"));
    extract_archive(archive, &staging)?;

    let mut order = load_loadorder(paths, game_id)?;
    // Replace existing same file if present
    order
        .mods
        .retain(|m| !(m.nexus_mod_id == mod_id && m.nexus_file_id == file_id));
    let next_order = order.mods.iter().map(|m| m.order).max().unwrap_or(0) + 1;
    let staged = StagedMod {
        id: format!("{mod_id}_{file_id}"),
        name: name.to_string(),
        nexus_mod_id: mod_id,
        nexus_file_id: file_id,
        version,
        domain: domain.to_string(),
        staging_path: staging.to_string_lossy().to_string(),
        enabled: true,
        order: next_order,
    };
    order.mods.push(staged.clone());
    save_loadorder(paths, game_id, &order)?;
    Ok(staged)
}

pub fn set_enabled(paths: &Paths, game_id: &str, mod_uid: &str, enabled: bool) -> Result<()> {
    let mut order = load_loadorder(paths, game_id)?;
    let Some(m) = order.mods.iter_mut().find(|m| m.id == mod_uid) else {
        bail!("mod not found: {mod_uid}");
    };
    m.enabled = enabled;
    save_loadorder(paths, game_id, &order)
}

pub fn set_load_order(paths: &Paths, game_id: &str, ordered_ids: &[String]) -> Result<()> {
    let mut order = load_loadorder(paths, game_id)?;
    let mut by_id: HashMap<String, StagedMod> =
        order.mods.drain(..).map(|m| (m.id.clone(), m)).collect();
    let mut new_mods = Vec::new();
    for (i, id) in ordered_ids.iter().enumerate() {
        if let Some(mut m) = by_id.remove(id) {
            m.order = (i as u32) + 1;
            new_mods.push(m);
        }
    }
    // Append any leftovers
    for (_, mut m) in by_id {
        m.order = (new_mods.len() as u32) + 1;
        new_mods.push(m);
    }
    order.mods = new_mods;
    save_loadorder(paths, game_id, &order)
}

pub fn remove_mod(paths: &Paths, game_id: &str, mod_uid: &str) -> Result<()> {
    let mut order = load_loadorder(paths, game_id)?;
    if let Some(pos) = order.mods.iter().position(|m| m.id == mod_uid) {
        let m = order.mods.remove(pos);
        let staging = PathBuf::from(&m.staging_path);
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
        save_loadorder(paths, game_id, &order)?;
    }
    Ok(())
}

pub fn remove_all_mods(paths: &Paths, game_id: &str) -> Result<()> {
    purge_deploy(paths, game_id)?;
    let order = load_loadorder(paths, game_id)?;
    for m in &order.mods {
        let staging = PathBuf::from(&m.staging_path);
        if staging.exists() {
            fs::remove_dir_all(&staging)?;
        }
    }
    save_loadorder(paths, game_id, &LoadOrder::default())?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkKind {
    HardlinkOrSymlink,
    Copied,
}

fn link_or_copy(src: &Path, dest: &Path) -> Result<LinkKind> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        if dest.is_dir() {
            fs::remove_dir_all(dest)?;
        } else {
            fs::remove_file(dest)?;
        }
    }
    // Hardlink files; directories are created normally and children linked.
    if src.is_dir() {
        fs::create_dir_all(dest)?;
        return Ok(LinkKind::HardlinkOrSymlink);
    }
    match fs::hard_link(src, dest) {
        Ok(()) => Ok(LinkKind::HardlinkOrSymlink),
        Err(_) => match symlink_file(src, dest) {
            Ok(()) => Ok(LinkKind::HardlinkOrSymlink),
            Err(e) => {
                log::debug!(
                    "symlink failed for {} -> {} ({e}); copying instead",
                    src.display(),
                    dest.display()
                );
                fs::copy(src, dest)?;
                Ok(LinkKind::Copied)
            }
        },
    }
}

#[cfg(unix)]
fn symlink_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dest)
}

#[cfg(windows)]
fn symlink_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(src, dest)
}

#[cfg(not(any(unix, windows)))]
fn symlink_file(_src: &Path, dest: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!("symlink unsupported on this platform for {}", dest.display()),
    ))
}

pub fn purge_deploy(paths: &Paths, game_id: &str) -> Result<()> {
    let manifest_path = paths.deploy_manifest(game_id);
    if !manifest_path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(&manifest_path)?;
    let manifest: DeployManifest = serde_json::from_str(&raw)?;
    // Remove files first, then empty dirs (reverse sort by path length)
    let mut entries = manifest.paths;
    entries.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for p in entries {
        let path = PathBuf::from(&p);
        if path.is_file() || path.is_symlink() {
            let _ = fs::remove_file(&path);
        } else if path.is_dir() {
            let _ = fs::remove_dir(&path); // only if empty
        }
    }
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&DeployManifest::default())?,
    )?;
    Ok(())
}

pub fn deploy(
    paths: &Paths,
    game_id: &str,
    plugin_id: &str,
    install_path: &Path,
) -> Result<DeployResult> {
    let plugin = plugin_by_id(plugin_id).context("unknown game plugin")?;
    let mut warnings = Vec::new();

    if !install_path.is_dir() {
        bail!(
            "Game install path does not exist or is not a directory: {}",
            install_path.display()
        );
    }

    warnings.extend(plugin.prepare_deploy(install_path)?);
    warnings.extend(plugin.preflight_warnings(install_path));

    purge_deploy(paths, game_id)?;

    let mut order = load_loadorder(paths, game_id)?;
    order.mods.sort_by_key(|m| m.order);

    let mut deployed = DeployManifest::default();
    let mut count = 0usize;
    let mut copied_files = 0usize;
    let enabled: Vec<_> = order.mods.iter().filter(|m| m.enabled).cloned().collect();
    let enabled_mods = enabled.len();
    let mut enabled_mod_folders = Vec::new();

    for staged in &enabled {
        let staging = PathBuf::from(&staged.staging_path);
        if !staging.exists() {
            let msg = format!(
                "Missing staging for {}: {}",
                staged.name,
                staging.display()
            );
            log::warn!("{msg}");
            warnings.push(msg);
            continue;
        }
        let root = normalize_staging_root(&staging, plugin)?;
        warnings.extend(plugin.staging_deploy_warnings(&root, &staged.name));
        let to_root = plugin.deploys_to_install_root(&root);
        let wrap = !to_root
            && (plugin.prefers_mod_folder() || plugin.should_wrap_as_mod_folder(&root));
        if wrap {
            let folder_name = plugin.wrap_mod_folder_name(&root, &staged.name);
            if !folder_name.is_empty() {
                enabled_mod_folders.push(folder_name);
            }
        }
        let (files, copied) =
            deploy_tree(plugin, install_path, &root, &staged.name, &mut deployed)?;
        if files == 0 {
            warnings.push(format!("No files deployed from {}", staged.name));
        }
        if copied > 0 {
            copied_files += copied;
        }
        count += files;
    }

    if copied_files > 0 {
        warnings.push(format!(
            "{copied_files} file(s) were copied instead of hardlinked/symlinked (cross-volume or missing symlink privilege). On Windows, enable Developer Mode for symlink deploy."
        ));
    }

    if enabled_mods == 0 {
        warnings.push("No enabled mods to deploy.".into());
    } else if count == 0 {
        warnings.push("Deploy finished with 0 files linked.".into());
    }

    warnings.extend(plugin.after_deploy(install_path, &enabled_mod_folders)?);

    crate::config::ensure_game_dirs(paths, game_id)?;
    fs::write(
        paths.deploy_manifest(game_id),
        serde_json::to_string_pretty(&deployed)?,
    )?;
    Ok(DeployResult {
        file_count: count,
        enabled_mods,
        warnings,
    })
}

fn deploy_tree(
    plugin: &dyn GamePlugin,
    install_path: &Path,
    content_root: &Path,
    mod_name: &str,
    deployed: &mut DeployManifest,
) -> Result<(usize, usize)> {
    let mut n = 0usize;
    let mut copied = 0usize;
    let to_root = plugin.deploys_to_install_root(content_root);
    let wrap = !to_root
        && (plugin.prefers_mod_folder() || plugin.should_wrap_as_mod_folder(content_root));
    let folder_name = plugin.wrap_mod_folder_name(content_root, mod_name);

    for entry in WalkDir::new(content_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path == content_root {
            continue;
        }
        let rel = path.strip_prefix(content_root)?;
        let rel = normalize_relative(rel);
        let deploy_rel: PathBuf = if wrap {
            Path::new(&folder_name).join(&rel)
        } else {
            rel
        };
        let dest = if to_root {
            install_path.join(&deploy_rel)
        } else {
            plugin.resolve_deploy_root(install_path, &deploy_rel)?
        };
        if entry.file_type().is_dir() {
            fs::create_dir_all(&dest)?;
            deployed.paths.push(dest.to_string_lossy().to_string());
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        if link_or_copy(path, &dest)? == LinkKind::Copied {
            copied += 1;
        }
        deployed.paths.push(dest.to_string_lossy().to_string());
        n += 1;
    }
    Ok((n, copied))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;

    fn test_paths() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_dir: tmp.path().join("config"),
            data_dir: tmp.path().join("data"),
            cache_dir: tmp.path().join("cache"),
        };
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        std::fs::create_dir_all(&paths.cache_dir).unwrap();
        (tmp, paths)
    }

    #[test]
    fn deploy_cyberpunk_preserves_bin_and_red4ext() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_test";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("CET_1_1");
        std::fs::create_dir_all(staging.join("bin").join("x64").join("plugins")).unwrap();
        std::fs::write(
            staging.join("bin").join("x64").join("version.dll"),
            b"dll",
        )
        .unwrap();
        std::fs::create_dir_all(staging.join("red4ext").join("plugins")).unwrap();
        std::fs::write(staging.join("red4ext").join("RED4ext.dll"), b"dll").unwrap();

        let order = LoadOrder {
            mods: vec![StagedMod {
                id: "1_1".into(),
                name: "CET".into(),
                nexus_mod_id: 1,
                nexus_file_id: 1,
                version: None,
                domain: "cyberpunk2077".into(),
                staging_path: staging.to_string_lossy().into(),
                enabled: true,
                order: 1,
            }],
        };
        save_loadorder(&paths, game_id, &order).unwrap();

        let result = deploy(&paths, game_id, "cyberpunk2077", install.path()).unwrap();
        assert!(result.file_count >= 2, "warnings: {:?}", result.warnings);
        assert!(install.path().join("bin/x64/version.dll").exists());
        assert!(install.path().join("red4ext/RED4ext.dll").exists());
        assert!(!install.path().join("mods/bin/x64/version.dll").exists());
        assert!(!install.path().join("mods/red4ext/RED4ext.dll").exists());
    }

    #[test]
    fn deploy_peels_wrapper_then_keeps_roots() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_wrap";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("Wrap_2_2");
        let inner = staging.join("Cyber Engine Tweaks");
        std::fs::create_dir_all(inner.join("bin").join("x64")).unwrap();
        std::fs::write(inner.join("bin").join("x64").join("global.ini"), b"x").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "2_2".into(),
                    name: "CET Wrap".into(),
                    nexus_mod_id: 2,
                    nexus_file_id: 2,
                    version: None,
                    domain: "cyberpunk2077".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                }],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "cyberpunk2077", install.path()).unwrap();
        assert_eq!(result.file_count, 1, "{:?}", result.warnings);
        assert!(install.path().join("bin/x64/global.ini").exists());
    }

    #[test]
    fn deploy_flat_archive_and_backslash_paths() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_flat";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("Flat_3_3");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("Cool.archive"), b"a").unwrap();
        // Literal backslash in filename as produced by some Windows zips on Linux.
        std::fs::write(staging.join(r"r6\scripts\Mod\mod.reds"), b"reds").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "3_3".into(),
                    name: "Flat".into(),
                    nexus_mod_id: 3,
                    nexus_file_id: 3,
                    version: None,
                    domain: "cyberpunk2077".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                }],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "cyberpunk2077", install.path()).unwrap();
        assert!(result.file_count >= 2, "{:?}", result.warnings);
        assert!(install
            .path()
            .join("archive/pc/mod/Cool.archive")
            .exists());
        assert!(install.path().join("r6/scripts/Mod/mod.reds").exists());
    }

    #[test]
    fn deploy_zero_enabled_reports_warning() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_empty";
        let install = tempfile::tempdir().unwrap();
        save_loadorder(&paths, game_id, &LoadOrder::default()).unwrap();
        let result = deploy(&paths, game_id, "cyberpunk2077", install.path()).unwrap();
        assert_eq!(result.file_count, 0);
        assert_eq!(result.enabled_mods, 0);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn deploy_missing_staging_warns() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_missing";
        let install = tempfile::tempdir().unwrap();
        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "9_9".into(),
                    name: "Gone".into(),
                    nexus_mod_id: 9,
                    nexus_file_id: 9,
                    version: None,
                    domain: "cyberpunk2077".into(),
                    staging_path: paths
                        .mods_dir(game_id)
                        .join("does_not_exist")
                        .to_string_lossy()
                        .into(),
                    enabled: true,
                    order: 1,
                }],
            },
        )
        .unwrap();
        let result = deploy(&paths, game_id, "cyberpunk2077", install.path()).unwrap();
        assert_eq!(result.file_count, 0);
        assert!(result.warnings.iter().any(|w| w.contains("Missing staging")));
    }

    #[test]
    fn deploy_redmod_wraps_info_json() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_redmod";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("Red_4_4");
        let inner = staging.join("MyRedMod");
        std::fs::create_dir_all(inner.join("archives")).unwrap();
        std::fs::write(inner.join("info.json"), r#"{"name":"MyRedMod"}"#).unwrap();
        std::fs::write(inner.join("archives").join("x.archive"), b"a").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "4_4".into(),
                    name: "Nexus REDmod Title".into(),
                    nexus_mod_id: 4,
                    nexus_file_id: 4,
                    version: None,
                    domain: "cyberpunk2077".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                }],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "cyberpunk2077", install.path()).unwrap();
        assert!(result.file_count >= 2, "{:?}", result.warnings);
        assert!(install.path().join("mods/MyRedMod/info.json").exists());
        assert!(!install.path().join("mods/Nexus REDmod Title").exists());
    }

    fn stage_stardew(
        paths: &Paths,
        game_id: &str,
        id: &str,
        name: &str,
        staging: PathBuf,
    ) {
        save_loadorder(
            paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: id.into(),
                    name: name.into(),
                    nexus_mod_id: 1,
                    nexus_file_id: 1,
                    version: None,
                    domain: "stardewvalley".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                }],
            },
        )
        .unwrap();
    }

    #[test]
    fn deploy_stardew_wraps_manifest_mod() {
        let (_tmp, paths) = test_paths();
        let game_id = "sdv_wrap";
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("smapi-internal")).unwrap();

        let staging = paths.mods_dir(game_id).join("Cool_1_1");
        let inner = staging.join("CoolMod");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("manifest.json"), r#"{"UniqueID":"A.Cool"}"#).unwrap();
        std::fs::write(inner.join("Cool.dll"), b"dll").unwrap();
        stage_stardew(&paths, game_id, "1_1", "Cool Mod", staging);

        let result = deploy(&paths, game_id, "stardewvalley", install.path()).unwrap();
        assert!(result.file_count >= 2, "{:?}", result.warnings);
        assert!(install.path().join("Mods/CoolMod/manifest.json").exists());
        assert!(install.path().join("Mods/CoolMod/Cool.dll").exists());
        assert!(!install.path().join("Mods/Cool Mod/manifest.json").exists());
        assert!(!result.warnings.iter().any(|w| w.contains("SMAPI not found")));
    }

    #[test]
    fn deploy_stardew_mods_prefix_no_double_nest() {
        let (_tmp, paths) = test_paths();
        let game_id = "sdv_mods";
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("smapi-internal")).unwrap();

        let staging = paths.mods_dir(game_id).join("Pack_2_2");
        std::fs::create_dir_all(staging.join("Mods").join("CoolMod")).unwrap();
        std::fs::write(
            staging.join("Mods").join("CoolMod").join("manifest.json"),
            "{}",
        )
        .unwrap();
        stage_stardew(&paths, game_id, "2_2", "Pack", staging);

        let result = deploy(&paths, game_id, "stardewvalley", install.path()).unwrap();
        assert_eq!(result.file_count, 1, "{:?}", result.warnings);
        assert!(install.path().join("Mods/CoolMod/manifest.json").exists());
        assert!(!install
            .path()
            .join("Mods/Pack/Mods/CoolMod/manifest.json")
            .exists());
    }

    #[test]
    fn deploy_stardew_multi_mod_siblings() {
        let (_tmp, paths) = test_paths();
        let game_id = "sdv_multi";
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("smapi-internal")).unwrap();

        let staging = paths.mods_dir(game_id).join("Bundle_3_3");
        for name in ["ModA", "ModB"] {
            let dir = staging.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("manifest.json"), "{}").unwrap();
        }
        stage_stardew(&paths, game_id, "3_3", "Bundle", staging);

        let result = deploy(&paths, game_id, "stardewvalley", install.path()).unwrap();
        assert_eq!(result.file_count, 2, "{:?}", result.warnings);
        assert!(install.path().join("Mods/ModA/manifest.json").exists());
        assert!(install.path().join("Mods/ModB/manifest.json").exists());
    }

    #[test]
    fn deploy_stardew_warns_without_smapi() {
        let (_tmp, paths) = test_paths();
        let game_id = "sdv_nosmapi";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("M_4_4");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("manifest.json"), "{}").unwrap();
        stage_stardew(&paths, game_id, "4_4", "M", staging);

        let result = deploy(&paths, game_id, "stardewvalley", install.path()).unwrap();
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("SMAPI not found")));
        assert!(install.path().join("Mods/M/manifest.json").exists());
    }

    #[test]
    fn deploy_stardew_smapi_installer_to_root() {
        let (_tmp, paths) = test_paths();
        let game_id = "sdv_smapi";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("SMAPI_5_5");
        let inner = staging.join("SMAPI 4.0");
        std::fs::create_dir_all(inner.join("smapi-internal")).unwrap();
        std::fs::write(inner.join("StardewModdingAPI.exe"), b"exe").unwrap();
        std::fs::write(inner.join("smapi-internal").join("config.json"), b"{}").unwrap();
        stage_stardew(&paths, game_id, "5_5", "SMAPI", staging);

        let result = deploy(&paths, game_id, "stardewvalley", install.path()).unwrap();
        assert!(result.file_count >= 2, "{:?}", result.warnings);
        assert!(install.path().join("StardewModdingAPI.exe").exists());
        assert!(install.path().join("smapi-internal/config.json").exists());
        assert!(!install
            .path()
            .join("Mods/SMAPI/StardewModdingAPI.exe")
            .exists());
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("SMAPI installer")));
    }

    #[test]
    fn deploy_darktide_writes_load_order_and_skips_dmf() {
        let (_tmp, paths) = test_paths();
        let game_id = "dt_test";
        let install = tempfile::tempdir().unwrap();
        // Already patched so we do not spawn dtkit-patch.
        std::fs::create_dir_all(install.path().join("bundle")).unwrap();
        std::fs::write(
            install.path().join("bundle/bundle_database.data"),
            b"xxxpatch_999yyy",
        )
        .unwrap();

        let holy_light_staging = paths.mods_dir(game_id).join("Holy Light_1_1");
        let holy_light_inner = holy_light_staging.join("HolyLight");
        std::fs::create_dir_all(&holy_light_inner).unwrap();
        std::fs::write(holy_light_inner.join("HolyLight.mod"), b"mod").unwrap();

        let dmf_staging = paths.mods_dir(game_id).join("DMF_2_2");
        std::fs::create_dir_all(dmf_staging.join("dmf")).unwrap();
        std::fs::write(dmf_staging.join("dmf").join("dmf.lua"), b"lua").unwrap();

        let health_staging = paths.mods_dir(game_id).join("Healthbars_3_3");
        let health_inner = health_staging.join("healthbars");
        std::fs::create_dir_all(&health_inner).unwrap();
        std::fs::write(health_inner.join("healthbars.mod"), b"mod").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![
                    StagedMod {
                        id: "1_1".into(),
                        name: "Holy Light".into(),
                        nexus_mod_id: 1,
                        nexus_file_id: 1,
                        version: None,
                        domain: "warhammer40kdarktide".into(),
                        staging_path: holy_light_staging.to_string_lossy().into(),
                        enabled: true,
                        order: 1,
                    },
                    StagedMod {
                        id: "2_2".into(),
                        name: "Darktide Mod Framework".into(),
                        nexus_mod_id: 2,
                        nexus_file_id: 2,
                        version: None,
                        domain: "warhammer40kdarktide".into(),
                        staging_path: dmf_staging.to_string_lossy().into(),
                        enabled: true,
                        order: 2,
                    },
                    StagedMod {
                        id: "3_3".into(),
                        name: "Healthbars".into(),
                        nexus_mod_id: 3,
                        nexus_file_id: 3,
                        version: None,
                        domain: "warhammer40kdarktide".into(),
                        staging_path: health_staging.to_string_lossy().into(),
                        enabled: false,
                        order: 3,
                    },
                ],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "warhammer40kdarktide", install.path()).unwrap();
        assert!(result.file_count >= 2, "{:?}", result.warnings);
        assert!(install.path().join("mods/HolyLight/HolyLight.mod").exists());
        assert!(!install.path().join("mods/Holy Light").exists());
        assert!(install.path().join("mods/dmf/dmf.lua").exists());
        assert!(!install.path().join("mods/Healthbars").exists());

        let order_txt =
            std::fs::read_to_string(install.path().join("mods/mod_load_order.txt")).unwrap();
        assert!(order_txt.contains("HolyLight\n"));
        assert!(!order_txt.contains("Holy Light\n"));
        assert!(!order_txt.lines().any(|l| l.trim() == "dmf"));
        assert!(!order_txt.contains("Healthbars"));
        assert!(!order_txt.contains("Darktide Mod Framework"));
    }

    #[test]
    fn deploy_darktide_loader_root_layout() {
        let (_tmp, paths) = test_paths();
        let game_id = "dt_loader";
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("bundle")).unwrap();
        std::fs::write(
            install.path().join("bundle/bundle_database.data"),
            b"xxxpatch_999yyy",
        )
        .unwrap();

        let staging = paths.mods_dir(game_id).join("Loader_1_1");
        std::fs::create_dir_all(staging.join("tools")).unwrap();
        std::fs::create_dir_all(staging.join("binaries")).unwrap();
        std::fs::create_dir_all(staging.join("mods")).unwrap();
        std::fs::write(staging.join("tools/dtkit-patch.exe"), b"fake").unwrap();
        std::fs::write(staging.join("binaries/mod_loader"), b"x").unwrap();
        std::fs::write(staging.join("toggle_darktide_mods.bat"), b"bat").unwrap();
        std::fs::write(staging.join("mods/mod_load_order.txt"), b"-- template\n").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "1_1".into(),
                    name: "Darktide Mod Loader".into(),
                    nexus_mod_id: 19,
                    nexus_file_id: 1,
                    version: None,
                    domain: "warhammer40kdarktide".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                }],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "warhammer40kdarktide", install.path()).unwrap();
        assert!(result.file_count >= 3, "{:?}", result.warnings);
        assert!(install.path().join("tools/dtkit-patch.exe").exists());
        assert!(install.path().join("binaries/mod_loader").exists());
        assert!(install.path().join("toggle_darktide_mods.bat").exists());
        // Loader is not wrapped → not listed in load order (rewritten empty aside from header).
        let order_txt =
            std::fs::read_to_string(install.path().join("mods/mod_load_order.txt")).unwrap();
        assert!(!order_txt
            .lines()
            .any(|l| !l.starts_with("--") && !l.trim().is_empty()));
    }
}
