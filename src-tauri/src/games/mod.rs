//! Per-game deploy plugins.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

mod baldursgate3;
mod cyberpunk2077;
mod stardewvalley;

pub use baldursgate3::BaldursGate3Plugin;
pub use cyberpunk2077::Cyberpunk2077Plugin;
pub use stardewvalley::StardewValleyPlugin;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct GamePluginInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub nexus_domain: &'static str,
    pub match_names: &'static [&'static str],
}

pub trait GamePlugin: Send + Sync {
    fn info(&self) -> GamePluginInfo;

    /// Resolve where staged mod files should be linked inside the game install.
    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf>;

    /// Prefer a subdirectory name under Mods/ for folder-style games.
    fn prefers_mod_folder(&self) -> bool {
        false
    }

    /// Top-level staging folder names that must not be peeled as archive wrappers.
    fn preserve_staging_root_names(&self) -> &[&str] {
        &[]
    }

    /// When true, deploy under a sanitized mod-name folder (e.g. REDmod info.json packs).
    fn should_wrap_as_mod_folder(&self, _content_root: &Path) -> bool {
        false
    }
}

pub fn all_plugins() -> Vec<&'static dyn GamePlugin> {
    vec![
        &StardewValleyPlugin,
        &BaldursGate3Plugin,
        &Cyberpunk2077Plugin,
    ]
}

pub fn plugin_by_id(id: &str) -> Option<&'static dyn GamePlugin> {
    all_plugins().into_iter().find(|p| p.info().id == id)
}

pub fn match_plugin(title: &str, _install_path: Option<&str>) -> Option<GamePluginInfo> {
    let lower = title.to_lowercase();
    for plugin in all_plugins() {
        let info = plugin.info();
        for name in info.match_names {
            if lower.contains(&name.to_lowercase()) {
                return Some(info);
            }
        }
    }
    None
}

pub fn list_plugins() -> Vec<GamePluginInfo> {
    all_plugins().into_iter().map(|p| p.info()).collect()
}

/// Turn archive-originated `\` separators into normal path components.
pub fn normalize_relative(relative: &Path) -> PathBuf {
    let raw = relative.to_string_lossy().replace('\\', "/");
    let trimmed = raw.trim_matches('/');
    if trimmed.is_empty() {
        return PathBuf::new();
    }
    PathBuf::from(trimmed)
}

/// Strip a single top-level archive folder if the archive contained one root dir,
/// unless that folder is a meaningful game-root name for the plugin.
pub fn normalize_staging_root(staging_dir: &Path, plugin: &dyn GamePlugin) -> Result<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(staging_dir)
        .with_context(|| format!("read staging {}", staging_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            s != "__MACOSX" && !s.starts_with('.')
        })
        .collect();
    if entries.len() == 1 {
        let only = entries.remove(0);
        if only.file_type()?.is_dir() {
            let name = only.file_name();
            let name = name.to_string_lossy();
            let preserve = plugin
                .preserve_staging_root_names()
                .iter()
                .any(|root| name.eq_ignore_ascii_case(root));
            if !preserve {
                return Ok(only.path());
            }
        }
    }
    Ok(staging_dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberpunk2077::Cyberpunk2077Plugin;

    #[test]
    fn peels_wrapper_but_keeps_cyberpunk_roots() {
        let staging = tempfile::tempdir().unwrap();
        let wrapper = staging.path().join("Some Mod Pack");
        std::fs::create_dir_all(wrapper.join("bin").join("x64")).unwrap();
        std::fs::write(wrapper.join("bin").join("x64").join("a.dll"), b"x").unwrap();

        let root = normalize_staging_root(staging.path(), &Cyberpunk2077Plugin).unwrap();
        assert_eq!(root, wrapper);

        let bin_only = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(bin_only.path().join("bin").join("x64")).unwrap();
        std::fs::write(bin_only.path().join("bin").join("x64").join("a.dll"), b"x").unwrap();
        let kept = normalize_staging_root(bin_only.path(), &Cyberpunk2077Plugin).unwrap();
        assert_eq!(kept, bin_only.path());

        let red4 = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(red4.path().join("red4ext").join("plugins")).unwrap();
        let kept_red = normalize_staging_root(red4.path(), &Cyberpunk2077Plugin).unwrap();
        assert_eq!(kept_red, red4.path());
    }

    #[test]
    fn peels_for_folder_style_plugins() {
        let staging = tempfile::tempdir().unwrap();
        let wrapper = staging.path().join("MyCoolMod");
        std::fs::create_dir_all(&wrapper).unwrap();
        std::fs::write(wrapper.join("manifest.json"), "{}").unwrap();
        let root = normalize_staging_root(staging.path(), &StardewValleyPlugin).unwrap();
        assert_eq!(root, wrapper);
    }
}
