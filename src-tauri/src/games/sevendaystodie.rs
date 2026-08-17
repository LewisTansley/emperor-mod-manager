//! 7 Days to Die (Mods/ + ModInfo.xml). EAC must be off for mods to load.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

pub const SEVEN_DAYS_ROOT_DIRS: &[&str] = &["Mods"];

pub struct SevenDaysToDiePlugin;

impl GamePlugin for SevenDaysToDiePlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "7daystodie",
            display_name: "7 Days to Die",
            nexus_domain: "7daystodie",
            match_names: &["7 days to die", "7daystodie", "seven days to die"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        SEVEN_DAYS_ROOT_DIRS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        looks_like_7dtd_mod(content_root)
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        if eac_present(install_path) {
            vec![
                "Easy Anti-Cheat files are present. Launch 7 Days to Die without EAC or mods will not load."
                    .into(),
            ]
        } else {
            Vec::new()
        }
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_mods_folder(install_path, relative))
    }
}

fn looks_like_7dtd_mod(content_root: &Path) -> bool {
    content_root.join("ModInfo.xml").is_file() || content_root.join("modinfo.xml").is_file()
}

fn eac_present(install_path: &Path) -> bool {
    install_path.join("EasyAntiCheat").is_dir()
        || install_path.join("EasyAntiCheat_EOS").is_dir()
        || install_path.join("EasyAntiCheat_EOS.so").is_file()
        || install_path.join("start_protected_game.exe").is_file()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_modinfo_and_deploys_under_mods() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ModInfo.xml"), "<xml/>").unwrap();
        assert!(SevenDaysToDiePlugin.should_wrap_as_mod_folder(dir.path()));
        let dest = SevenDaysToDiePlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("CoolMod/ModInfo.xml"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/CoolMod/ModInfo.xml"));
    }

    #[test]
    fn preserves_mods_prefix() {
        let dest = SevenDaysToDiePlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("Mods/CoolMod/ModInfo.xml"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/CoolMod/ModInfo.xml"));
    }

    #[test]
    fn warns_when_eac_present() {
        let empty = tempfile::tempdir().unwrap();
        assert!(SevenDaysToDiePlugin.preflight_warnings(empty.path()).is_empty());
        std::fs::create_dir_all(empty.path().join("EasyAntiCheat")).unwrap();
        assert_eq!(SevenDaysToDiePlugin.preflight_warnings(empty.path()).len(), 1);
    }
}
