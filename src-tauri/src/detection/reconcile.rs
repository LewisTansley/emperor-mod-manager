//! Reconcile managed games with current install detection (moves, uninstalls).

use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::config::{self, AppConfig, ManagedGame, Paths};

use super::DetectedGame;

pub fn normalize_game_key(title: &str, launcher: &str) -> String {
    format!(
        "{}|{}",
        title.trim().to_lowercase(),
        launcher.trim().to_lowercase()
    )
}

pub fn games_match(a_title: &str, a_launcher: &str, b_title: &str, b_launcher: &str) -> bool {
    normalize_game_key(a_title, a_launcher) == normalize_game_key(b_title, b_launcher)
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GameHealthStatus {
    Ok,
    Missing,
    RelocateCandidate,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameHealth {
    pub game_id: String,
    pub status: GameHealthStatus,
    pub new_install_path: Option<String>,
    pub relocate_target_id: Option<String>,
}

pub fn compute_game_health(managed: &[ManagedGame], detected: &[DetectedGame]) -> Vec<GameHealth> {
    managed
        .iter()
        .map(|game| health_for_managed(game, detected))
        .collect()
}

fn health_for_managed(game: &ManagedGame, detected: &[DetectedGame]) -> GameHealth {
    if Path::new(&game.install_path).is_dir() {
        return GameHealth {
            game_id: game.id.clone(),
            status: GameHealthStatus::Ok,
            new_install_path: None,
            relocate_target_id: None,
        };
    }

    if let Some(candidate) = detected.iter().find(|d| {
        games_match(&game.title, &game.launcher, &d.title, &d.launcher)
    }) {
        return GameHealth {
            game_id: game.id.clone(),
            status: GameHealthStatus::RelocateCandidate,
            new_install_path: candidate.install_path.clone(),
            relocate_target_id: Some(candidate.id.clone()),
        };
    }

    GameHealth {
        game_id: game.id.clone(),
        status: GameHealthStatus::Missing,
        new_install_path: None,
        relocate_target_id: None,
    }
}

pub fn relink_managed_game(
    paths: &Paths,
    config: &mut AppConfig,
    old_id: &str,
    new_id: &str,
    detected: &[DetectedGame],
) -> Result<ManagedGame> {
    let old_game = config
        .managed_games
        .iter()
        .find(|g| g.id == old_id)
        .cloned()
        .context("Managed game not found")?;

    let detection = detected
        .iter()
        .find(|d| d.id == new_id)
        .context("Detection not found for relink target")?;

    if !games_match(
        &old_game.title,
        &old_game.launcher,
        &detection.title,
        &detection.launcher,
    ) {
        anyhow::bail!("Relink target does not match managed game title and launcher");
    }

    let new_install_path = detection
        .install_path
        .clone()
        .filter(|p| !p.is_empty())
        .context("Relink target has no install path")?;

    if !Path::new(&new_install_path).is_dir() {
        anyhow::bail!("Relink target install path does not exist");
    }

    migrate_game_data_dir(paths, old_id, new_id)?;

    let mut updated = old_game;
    updated.id = new_id.to_string();
    updated.install_path = new_install_path;
    if detection.cover_path.is_some() {
        updated.cover_path = detection.cover_path.clone();
    }

    config.managed_games.retain(|g| {
        g.id != old_id
            && g.id != new_id
            && !games_match(&g.title, &g.launcher, &updated.title, &updated.launcher)
    });
    config.managed_games.push(updated.clone());

    if config.last_active_game_id.as_deref() == Some(old_id) {
        config.last_active_game_id = Some(new_id.to_string());
    }

    config::save_config(paths, config)?;
    Ok(updated)
}

fn migrate_game_data_dir(paths: &Paths, old_id: &str, new_id: &str) -> Result<()> {
    if old_id == new_id {
        return Ok(());
    }

    let old_dir = paths.game_data_dir(old_id);
    let new_dir = paths.game_data_dir(new_id);

    if !old_dir.is_dir() {
        return Ok(());
    }

    if !new_dir.exists() {
        fs::rename(&old_dir, &new_dir).with_context(|| {
            format!(
                "moving game data {} to {}",
                old_dir.display(),
                new_dir.display()
            )
        })?;
        return Ok(());
    }

    if game_data_dir_is_empty(&new_dir) {
        copy_dir_contents(&old_dir, &new_dir)?;
        fs::remove_dir_all(&old_dir).with_context(|| format!("removing old game data {}", old_dir.display()))?;
        return Ok(());
    }

    copy_dir_contents(&old_dir, &new_dir)?;
    fs::remove_dir_all(&old_dir).with_context(|| format!("removing old game data {}", old_dir.display()))?;
    Ok(())
}

fn game_data_dir_is_empty(dir: &Path) -> bool {
    !dir.join("loadorder.json").exists()
        && !dir.join("deployed.json").exists()
        && dir.join("mods")
            .read_dir()
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(true)
}

fn copy_dir_contents(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            if destination_path.is_dir() {
                copy_dir_contents(&source_path, &destination_path)?;
            } else if !destination_path.exists() {
                copy_dir_contents(&source_path, &destination_path)?;
            }
        } else if source_path.is_file() && !destination_path.exists() {
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "copying game data {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use tempfile::tempdir;

    fn sample_managed(id: &str, title: &str, path: &str) -> ManagedGame {
        ManagedGame {
            id: id.to_string(),
            title: title.to_string(),
            nexus_domain: "example".to_string(),
            install_path: path.to_string(),
            launcher: "Steam".to_string(),
            plugin_id: "example".to_string(),
            cover_path: None,
            project_name: None,
            thunderstore_community: None,
            modio_game_id: None,
            tool_overrides: None,
        }
    }

    fn sample_detected(id: &str, title: &str, path: &str) -> DetectedGame {
        DetectedGame {
            id: id.to_string(),
            title: title.to_string(),
            install_path: Some(path.to_string()),
            launcher: "Steam".to_string(),
            supported: true,
            plugin_id: Some("example".to_string()),
            nexus_domain: Some("example".to_string()),
            cover_path: None,
            engine_hint: None,
        }
    }

    #[test]
    fn normalize_game_key_is_case_insensitive() {
        assert_eq!(
            normalize_game_key("  Cyberpunk 2077 ", "Steam"),
            normalize_game_key("cyberpunk 2077", "steam")
        );
    }

    #[test]
    fn health_ok_when_install_exists() {
        let root = tempdir().unwrap();
        let install = root.path().join("game");
        fs::create_dir_all(&install).unwrap();
        let managed = sample_managed("old", "Game", &install.to_string_lossy());
        let health = health_for_managed(&managed, &[]);
        assert_eq!(health.status, GameHealthStatus::Ok);
    }

    #[test]
    fn health_missing_when_path_gone_and_no_detection() {
        let managed = sample_managed("old", "Game", "/nonexistent/path");
        let health = health_for_managed(&managed, &[]);
        assert_eq!(health.status, GameHealthStatus::Missing);
    }

    #[test]
    fn health_relocate_candidate_when_path_moved() {
        let root = tempdir().unwrap();
        let new_path = root.path().join("new");
        fs::create_dir_all(&new_path).unwrap();
        let managed = sample_managed("old", "Game", "/nonexistent/path");
        let detected = vec![sample_detected("new", "Game", &new_path.to_string_lossy())];
        let health = health_for_managed(&managed, &detected);
        assert_eq!(health.status, GameHealthStatus::RelocateCandidate);
        assert_eq!(health.relocate_target_id.as_deref(), Some("new"));
    }

    #[test]
    fn manage_dedupes_same_title_launcher() {
        let mut games = vec![
            sample_managed("old_id", "Game", "/old"),
            sample_managed("other", "Other", "/other"),
        ];
        let new_entry = sample_managed("new_id", "Game", "/new");
        games.retain(|g| {
            g.id == new_entry.id
                || !games_match(
                    &g.title,
                    &g.launcher,
                    &new_entry.title,
                    &new_entry.launcher,
                )
        });
        games.push(new_entry);
        assert_eq!(games.len(), 2);
        assert!(games.iter().any(|g| g.id == "new_id"));
        assert!(!games.iter().any(|g| g.id == "old_id"));
    }

    #[test]
    fn relink_migrates_data_dir_and_updates_config() {
        let root = tempdir().unwrap();
        let paths = Paths {
            config_dir: root.path().join("config"),
            data_dir: root.path().join("data"),
            cache_dir: root.path().join("cache"),
        };
        fs::create_dir_all(&paths.config_dir).unwrap();
        fs::create_dir_all(&paths.data_dir).unwrap();

        let old_install = root.path().join("old_install");
        let new_install = root.path().join("new_install");
        fs::create_dir_all(&new_install).unwrap();

        let old_data = paths.game_data_dir("old_id");
        fs::create_dir_all(old_data.join("mods").join("Example_1_1")).unwrap();
        fs::write(old_data.join("loadorder.json"), "[]").unwrap();

        let mut config = AppConfig::default();
        config.managed_games.push(sample_managed(
            "old_id",
            "Game",
            &old_install.to_string_lossy(),
        ));
        config.last_active_game_id = Some("old_id".to_string());

        let detected = vec![sample_detected(
            "new_id",
            "Game",
            &new_install.to_string_lossy(),
        )];

        let updated = relink_managed_game(&paths, &mut config, "old_id", "new_id", &detected).unwrap();
        assert_eq!(updated.id, "new_id");
        assert_eq!(updated.install_path, new_install.to_string_lossy());
        assert_eq!(config.managed_games.len(), 1);
        assert_eq!(config.last_active_game_id.as_deref(), Some("new_id"));
        assert!(!paths.game_data_dir("old_id").exists());
        assert!(paths.game_data_dir("new_id").join("loadorder.json").is_file());
    }
}
