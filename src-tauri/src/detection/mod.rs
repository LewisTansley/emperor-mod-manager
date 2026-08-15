//! Installed-game detection (Linux via lib_game_detector; Windows via Steam/Heroic).

use serde::Serialize;

use crate::games::{match_plugin, GamePluginInfo};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

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

pub(crate) fn make_id(title: &str, launcher: &str, path: &Option<String>) -> String {
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

pub(crate) fn finish_detected(
    title: String,
    launcher: String,
    install_path: Option<String>,
    cover_path: Option<String>,
) -> DetectedGame {
    let plugin = match_plugin(&title, install_path.as_deref());
    let supported = plugin.is_some();
    let (plugin_id, nexus_domain) = match plugin {
        Some(GamePluginInfo {
            id, nexus_domain, ..
        }) => (Some(id.to_string()), Some(nexus_domain.to_string())),
        None => (None, None),
    };
    let id = make_id(&title, &launcher, &install_path);
    DetectedGame {
        id,
        title,
        install_path,
        launcher,
        supported,
        plugin_id,
        nexus_domain,
        cover_path,
    }
}

pub fn scan_games() -> Vec<DetectedGame> {
    #[cfg(target_os = "linux")]
    {
        return linux::scan_games();
    }
    #[cfg(target_os = "windows")]
    {
        return windows::scan_games();
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        Vec::new()
    }
}
