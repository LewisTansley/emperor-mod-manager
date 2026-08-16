//! Non-destructive recovery of data written before the nexus-manager rename.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::Serialize;

use crate::{
    config::{self, AppConfig, Paths},
    mods::{self, LoadOrder},
};

const LEGACY_QUALIFIER: &str = "nexusmanager";
const LEGACY_APP_NAME: &str = "nexus-manager";

#[derive(Debug, Clone, Default, Serialize)]
pub struct RecoveryReport {
    pub games_copied: Vec<String>,
    pub mods_rewritten: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct OrphanScan {
    pub legacy_game_ids: Vec<String>,
    pub missing_staging: Vec<String>,
    pub untracked_staging: Vec<String>,
    pub deploy_without_loadorder: Vec<String>,
}

fn legacy_paths() -> Option<(PathBuf, PathBuf)> {
    let dirs = ProjectDirs::from("dev", LEGACY_QUALIFIER, LEGACY_APP_NAME)?;
    Some((
        dirs.config_dir().to_path_buf(),
        dirs.data_dir().to_path_buf(),
    ))
}

fn has_mod_data(dir: &Path) -> bool {
    dir.join("loadorder.json").is_file() || dir.join("mods").is_dir()
}

fn game_dirs(data_dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = fs::read_dir(data_dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            (path.is_dir() && has_mod_data(&path))
                .then(|| (entry.file_name().to_string_lossy().into_owned(), path))
        })
        .collect()
}

fn copy_dir(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &destination_path)?;
        } else if source_path.is_file() {
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "copying legacy data {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn game_dir_is_empty(dir: &Path) -> bool {
    !dir.join("loadorder.json").exists()
        && !dir.join("deployed.json").exists()
        && dir
            .join("mods")
            .read_dir()
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
}

fn rewrite_staging_paths(paths: &Paths, game_id: &str, legacy_game_dir: &Path) -> Result<usize> {
    let mut order: LoadOrder = mods::load_loadorder(paths, game_id)?;
    let old_root = legacy_game_dir.join("mods");
    let new_root = paths.mods_dir(game_id);
    let mut rewritten = 0;
    for staged in &mut order.mods {
        let old_path = Path::new(&staged.staging_path);
        if let Ok(relative) = old_path.strip_prefix(&old_root) {
            staged.staging_path = new_root.join(relative).to_string_lossy().into_owned();
            rewritten += 1;
        }
    }
    if rewritten > 0 {
        mods::save_loadorder(paths, game_id, &order)?;
    }
    Ok(rewritten)
}

fn merge_legacy_games(
    paths: &Paths,
    config: &mut AppConfig,
    legacy_config_dir: &Path,
) -> Result<()> {
    let file = legacy_config_dir.join("config.toml");
    if !file.is_file() {
        return Ok(());
    }
    let legacy: AppConfig =
        toml::from_str(&fs::read_to_string(&file)?).context("parsing legacy config.toml")?;
    let mut changed = false;
    for game in legacy.managed_games {
        if !config
            .managed_games
            .iter()
            .any(|current| current.id == game.id)
        {
            config.managed_games.push(game);
            changed = true;
        }
    }
    if changed {
        config::save_config(paths, config)?;
    }
    Ok(())
}

/// Copy legacy game data into the current app data directory and fix absolute staging paths.
///
/// Copying rather than moving is intentional: existing game installs can still symlink into the
/// legacy tree until the user completes a successful deploy from the migrated tree.
pub fn recover_legacy_data(paths: &Paths, config: &mut AppConfig) -> Result<RecoveryReport> {
    let mut report = RecoveryReport::default();
    let Some((legacy_config_dir, legacy_data_dir)) = legacy_paths() else {
        return Ok(report);
    };
    if !legacy_data_dir.is_dir() {
        return Ok(report);
    }

    merge_legacy_games(paths, config, &legacy_config_dir)?;
    for (game_id, legacy_game_dir) in game_dirs(&legacy_data_dir) {
        let destination = paths.game_data_dir(&game_id);
        if !destination.exists() || game_dir_is_empty(&destination) {
            copy_dir(&legacy_game_dir, &destination)?;
            report.games_copied.push(game_id.clone());
        }
        if paths.loadorder_file(&game_id).is_file() {
            report.mods_rewritten += rewrite_staging_paths(paths, &game_id, &legacy_game_dir)?;
        }
    }
    Ok(report)
}

pub fn scan_orphans(paths: &Paths, config: &AppConfig) -> OrphanScan {
    let mut scan = OrphanScan::default();
    if let Some((_, legacy_data_dir)) = legacy_paths() {
        scan.legacy_game_ids = game_dirs(&legacy_data_dir)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
    }

    for game in &config.managed_games {
        let Ok(order) = mods::load_loadorder(paths, &game.id) else {
            continue;
        };
        if order.mods.is_empty() && paths.deploy_manifest(&game.id).is_file() {
            scan.deploy_without_loadorder.push(game.id.clone());
        }
        let tracked: Vec<PathBuf> = order
            .mods
            .iter()
            .map(|staged| PathBuf::from(&staged.staging_path))
            .collect();
        for path in &tracked {
            if !path.exists() {
                scan.missing_staging
                    .push(format!("{}: {}", game.title, path.display()));
            }
        }
        let Ok(entries) = fs::read_dir(paths.mods_dir(&game.id)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() && !tracked.iter().any(|tracked_path| tracked_path == &path) {
                scan.untracked_staging
                    .push(format!("{}: {}", game.title, path.display()));
            }
        }
    }
    scan
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::StagedMod;
    use tempfile::tempdir;

    #[test]
    fn recognizes_mod_data_directories() {
        let root =
            std::env::temp_dir().join(format!("emperor-migration-test-{}", std::process::id()));
        let game = root.join("game");
        fs::create_dir_all(game.join("mods")).unwrap();
        assert!(has_mod_data(&game));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rewrites_legacy_absolute_staging_paths() {
        let root = tempdir().unwrap();
        let paths = Paths {
            config_dir: root.path().join("config"),
            data_dir: root.path().join("current"),
            cache_dir: root.path().join("cache"),
        };
        let game_id = "game";
        let legacy_game = root.path().join("legacy").join(game_id);
        let legacy_staging = legacy_game.join("mods").join("Example_1_2");
        fs::create_dir_all(&legacy_staging).unwrap();

        let mut mod_entry = StagedMod::default();
        mod_entry.staging_path = legacy_staging.to_string_lossy().into_owned();
        mods::save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![mod_entry],
            },
        )
        .unwrap();

        assert_eq!(rewrite_staging_paths(&paths, game_id, &legacy_game).unwrap(), 1);
        let recovered = mods::load_loadorder(&paths, game_id).unwrap();
        assert_eq!(
            recovered.mods[0].staging_path,
            paths
                .mods_dir(game_id)
                .join("Example_1_2")
                .to_string_lossy()
        );
    }
}
