//! The Forest (ModAPI). Sons of the Forest is a separate BepInEx title.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

pub const THE_FOREST_ROOT_DIRS: &[&str] = &["Mods"];

pub struct TheForestPlugin;

impl GamePlugin for TheForestPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "theforest",
            display_name: "The Forest",
            nexus_domain: "theforest",
            match_names: &["the forest", "theforest"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        true
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        THE_FOREST_ROOT_DIRS
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        if modapi_present(install_path) {
            Vec::new()
        } else {
            vec![
                "ModAPI not found in the game folder. Mods will not load until ModAPI is installed."
                    .into(),
            ]
        }
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_mods_folder(install_path, relative))
    }
}

fn resolve_mods_folder(install_path: &Path, relative: &Path) -> PathBuf {
    let normalized = normalize_relative(relative);
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect();
    if originals.is_empty() {
        return install_path.join("Mods");
    }
    let first = originals[0].to_string_lossy();
    if first.eq_ignore_ascii_case("Mods") {
        let mut out = install_path.join("Mods");
        for orig in originals.into_iter().skip(1) {
            out.push(orig);
        }
        return out;
    }
    install_path.join("Mods").join(&normalized)
}

fn modapi_present(install_path: &Path) -> bool {
    install_path.join("ModAPI.exe").is_file()
        || install_path.join("ModAPI.dll").is_file()
        || install_path.join("TheForestModAPI.dll").is_file()
        || install_path.join("Mods").join("ModAPI.dll").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deploys_under_mods() {
        let dest = TheForestPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("CoolMod/mod.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/CoolMod/mod.dll"));
    }

    #[test]
    fn preserves_mods_prefix() {
        let dest = TheForestPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("Mods/CoolMod/mod.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/CoolMod/mod.dll"));
    }

    #[test]
    fn warns_without_modapi() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(TheForestPlugin.preflight_warnings(empty.path()).len(), 1);
        std::fs::write(empty.path().join("ModAPI.exe"), b"x").unwrap();
        assert!(TheForestPlugin.preflight_warnings(empty.path()).is_empty());
    }
}
