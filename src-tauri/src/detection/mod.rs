//! Installed-game detection (Linux via lib_game_detector; Windows via Steam/Heroic).

use std::path::Path;

use serde::Serialize;

use crate::games::{match_plugin, GamePluginInfo};

#[cfg(target_os = "linux")]
mod linux;
// Compile under `test` on non-Windows so pure Steam path/cover helpers are covered in CI.
#[cfg(any(target_os = "windows", test))]
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
    /// When set (e.g. "unreal"), the install can be managed via a generic engine plugin.
    pub engine_hint: Option<String>,
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

    let engine_hint = if supported {
        None
    } else {
        install_path.as_deref().and_then(|p| {
            let path = Path::new(p);
            if crate::games::detect_ue_layout(path).is_some() {
                Some("unreal".to_string())
            } else if crate::games::looks_like_unity_install(path) {
                Some("bepinex".to_string())
            } else {
                None
            }
        })
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
        engine_hint,
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
