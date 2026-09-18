//! Space Engineers local mods under user data (`%APPDATA%/SpaceEngineers/Mods`), not the install.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

const STEAM_APP_ID: &str = "244850";

pub const SPACE_ENGINEERS_ROOT_DIRS: &[&str] = &["Mods"];

pub struct SpaceEngineersPlugin;

impl GamePlugin for SpaceEngineersPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "spaceengineers",
            display_name: "Space Engineers",
            nexus_domain: "spaceengineers",
            match_names: &["space engineers", "spaceengineers"],
        }
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        SPACE_ENGINEERS_ROOT_DIRS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        !content_root.join("Mods").is_dir()
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        if mods_dir(install_path).is_none() {
            vec![
                "Could not resolve Space Engineers user-data Mods folder (AppData or Proton compatdata). Mods may not appear until that path exists."
                    .into(),
            ]
        } else {
            Vec::new()
        }
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_se_deploy(install_path, relative))
    }
}

fn mods_dir(install_path: &Path) -> Option<PathBuf> {
    super::user_data::appdata_roaming_dir(install_path, STEAM_APP_ID)
        .map(|d| d.join("SpaceEngineers").join("Mods"))
}

fn resolve_se_deploy(install_path: &Path, relative: &Path) -> PathBuf {
    let root = mods_dir(install_path).unwrap_or_else(|| install_path.join("Mods"));
    let normalized = normalize_relative(relative);
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect();
    if originals.is_empty() {
        return root;
    }
    let first = originals[0].to_string_lossy();
    if first.eq_ignore_ascii_case("Mods") {
        let mut out = root;
        for orig in originals.into_iter().skip(1) {
            out.push(orig);
        }
        return out;
    }
    root.join(&normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_and_falls_back_to_install_mods_without_proton() {
        let dest = SpaceEngineersPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("CoolMod/mod.sbm"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/CoolMod/mod.sbm"));
    }

    #[test]
    fn proton_compatdata_when_under_steamapps() {
        let tmp = tempfile::tempdir().unwrap();
        let common = tmp
            .path()
            .join("steamapps")
            .join("common")
            .join("SpaceEngineers");
        std::fs::create_dir_all(&common).unwrap();
        let dest = SpaceEngineersPlugin
            .resolve_deploy_root(&common, Path::new("Cool/mod.sbm"))
            .unwrap();
        assert_eq!(
            dest,
            tmp.path()
                .join("steamapps")
                .join("compatdata")
                .join(STEAM_APP_ID)
                .join("pfx")
                .join("drive_c")
                .join("users")
                .join("steamuser")
                .join("AppData")
                .join("Roaming")
                .join("SpaceEngineers")
                .join("Mods")
                .join("Cool")
                .join("mod.sbm")
        );
    }
}
