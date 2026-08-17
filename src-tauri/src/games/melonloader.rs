//! Shared deploy logic for Unity titles modded with MelonLoader (BONELAB, Boneworks).
//!
//! Typical layout:
//! ```text
//! <Install>/MelonLoader/
//! <Install>/Mods/
//! <Install>/Plugins/
//! <Install>/UserLibs/
//! <Install>/UserData/
//! <Install>/version.dll | dobby.dll
//! ```

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

pub const MELONLOADER_PRESERVE_ROOTS: &[&str] = &[
    "MelonLoader",
    "Mods",
    "Plugins",
    "UserLibs",
    "UserData",
];

const LOADER_FILES: &[&str] = &[
    "version.dll",
    "dobby.dll",
    "winhttp.dll",
    "winmm.dll",
    "doorstop_config.ini",
    ".doorstop_version",
];

pub struct BonelabPlugin;
pub struct BoneworksPlugin;

impl GamePlugin for BonelabPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "bonelab",
            display_name: "BONELAB",
            nexus_domain: "bonelab",
            match_names: &["bonelab", "bone lab"],
        }
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        MELONLOADER_PRESERVE_ROOTS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        !looks_like_melonloader_pack(content_root) && looks_like_melon_mod(content_root)
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn deploys_to_install_root(&self, content_root: &Path) -> bool {
        looks_like_melonloader_pack(content_root)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        melonloader_preflight(install_path)
    }

    fn staging_deploy_warnings(&self, content_root: &Path, mod_name: &str) -> Vec<String> {
        melon_staging_warnings(content_root, mod_name)
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_melon_deploy(install_path, relative))
    }
}

impl GamePlugin for BoneworksPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "boneworks",
            display_name: "BONEWORKS",
            nexus_domain: "boneworks",
            match_names: &["boneworks", "bone works"],
        }
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        MELONLOADER_PRESERVE_ROOTS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        !looks_like_melonloader_pack(content_root) && looks_like_melon_mod(content_root)
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn deploys_to_install_root(&self, content_root: &Path) -> bool {
        looks_like_melonloader_pack(content_root)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        melonloader_preflight(install_path)
    }

    fn staging_deploy_warnings(&self, content_root: &Path, mod_name: &str) -> Vec<String> {
        melon_staging_warnings(content_root, mod_name)
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_melon_deploy(install_path, relative))
    }
}

pub fn melonloader_present(install_path: &Path) -> bool {
    install_path.join("MelonLoader").is_dir()
        || LOADER_FILES
            .iter()
            .any(|n| n.ends_with(".dll") && install_path.join(n).is_file())
}

pub fn looks_like_melonloader_pack(content_root: &Path) -> bool {
    if content_root.join("MelonLoader").is_dir() {
        return true;
    }
    LOADER_FILES.iter().any(|n| content_root.join(n).is_file())
}

pub fn looks_like_melon_mod(content_root: &Path) -> bool {
    if looks_like_melonloader_pack(content_root) {
        return false;
    }
    if content_root.join("Mods").is_dir() || content_root.join("Plugins").is_dir() {
        return true;
    }
    dir_contains_dll(content_root)
}

fn dir_contains_dll(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.ends_with(".dll") {
            return true;
        }
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) && dir_contains_dll(&entry.path())
        {
            return true;
        }
    }
    false
}

fn melonloader_preflight(install_path: &Path) -> Vec<String> {
    if melonloader_present(install_path) {
        Vec::new()
    } else {
        vec![
            "MelonLoader not found in the game folder. Mods will not load until MelonLoader is installed."
                .into(),
        ]
    }
}

fn melon_staging_warnings(content_root: &Path, mod_name: &str) -> Vec<String> {
    if looks_like_melonloader_pack(content_root) {
        vec![format!(
            "{mod_name} looks like a MelonLoader pack; files were deployed to the game root."
        )]
    } else {
        Vec::new()
    }
}

pub fn resolve_melon_deploy(install_path: &Path, relative: &Path) -> PathBuf {
    let normalized = normalize_relative(relative);
    let parts: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect();
    if parts.is_empty() {
        return install_path.join("Mods");
    }
    let first = parts[0].to_string_lossy();
    let first_lower = first.to_lowercase();
    if first_lower == "melonloader"
        || LOADER_FILES.iter().any(|n| first_lower == n.to_lowercase())
        || first_lower == "doorstop_libs"
    {
        let mut out = install_path.to_path_buf();
        for p in &parts {
            out.push(p);
        }
        return out;
    }
    if first_lower == "mods"
        || first_lower == "plugins"
        || first_lower == "userlibs"
        || first_lower == "userdata"
    {
        let mut out = install_path.to_path_buf();
        for p in &parts {
            out.push(p);
        }
        return out;
    }
    let mut out = install_path.join("Mods");
    for p in &parts {
        out.push(p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_dll_wraps_under_mods() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("CoolMod.dll"), b"x").unwrap();
        assert!(BonelabPlugin.should_wrap_as_mod_folder(tmp.path()));
        assert!(!BonelabPlugin.deploys_to_install_root(tmp.path()));
        let dest = BonelabPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("CoolMod.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/CoolMod.dll"));
    }

    #[test]
    fn loader_pack_deploys_to_root() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("MelonLoader")).unwrap();
        std::fs::write(tmp.path().join("version.dll"), b"x").unwrap();
        assert!(BonelabPlugin.deploys_to_install_root(tmp.path()));
        let dest = BonelabPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("MelonLoader/net6/x.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/MelonLoader/net6/x.dll"));
    }

    #[test]
    fn preserves_mods_and_plugins_prefix() {
        let dest = resolve_melon_deploy(Path::new("/game"), Path::new("Plugins/Helper.dll"));
        assert_eq!(dest, PathBuf::from("/game/Plugins/Helper.dll"));
        let dest = resolve_melon_deploy(Path::new("/game"), Path::new("UserData/cfg.json"));
        assert_eq!(dest, PathBuf::from("/game/UserData/cfg.json"));
    }

    #[test]
    fn warns_without_loader() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(BoneworksPlugin.preflight_warnings(empty.path()).len(), 1);
        std::fs::create_dir_all(empty.path().join("MelonLoader")).unwrap();
        assert!(BoneworksPlugin.preflight_warnings(empty.path()).is_empty());
    }
}
