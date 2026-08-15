//! Installed-game detection via lib_game_detector.

use lib_game_detector::get_detector;
use serde::Serialize;

use crate::games::{match_plugin, GamePluginInfo};

#[derive(Debug, Clone, Serialize)]
pub struct DetectedGame {
    pub id: String,
    pub title: String,
    pub install_path: Option<String>,
    pub launcher: String,
    pub supported: bool,
    pub plugin_id: Option<String>,
    pub nexus_domain: Option<String>,
    pub cover_path: Option<String>,
}

fn launcher_label(source: &lib_game_detector::data::SupportedLaunchers) -> String {
    format!("{source}")
}

fn make_id(title: &str, launcher: &str, path: &Option<String>) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update(b"|");
    hasher.update(launcher.as_bytes());
    hasher.update(b"|");
    if let Some(p) = path {
        hasher.update(p.as_bytes());
    }
    hex::encode(hasher.finalize())[..16].to_string()
}

pub fn scan_games() -> Vec<DetectedGame> {
    let detector = get_detector();
    let games = detector.get_all_detected_games();
    let mut out = Vec::with_capacity(games.len());

    for game in games {
        let launcher = launcher_label(&game.source);
        let install_path = game
            .path_game_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());
        let plugin = match_plugin(&game.title, install_path.as_deref());
        let supported = plugin.is_some();
        let (plugin_id, nexus_domain) = match plugin {
            Some(GamePluginInfo {
                id, nexus_domain, ..
            }) => (Some(id.to_string()), Some(nexus_domain.to_string())),
            None => (None, None),
        };
        let cover_path = game
            .path_box_art
            .as_ref()
            .or(game.path_icon.as_ref())
            .map(|p| p.to_string_lossy().to_string());
        let id = make_id(&game.title, &launcher, &install_path);
        out.push(DetectedGame {
            id,
            title: game.title,
            install_path,
            launcher,
            supported,
            plugin_id,
            nexus_domain,
            cover_path,
        });
    }

    out.sort_by(|a, b| {
        b.supported
            .cmp(&a.supported)
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    out
}
