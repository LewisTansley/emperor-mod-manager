//! RimWorld (Mods/<ModName>/About/About.xml). Steam Workshop is out of scope.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

pub const RIMWORLD_ROOT_DIRS: &[&str] = &["Mods"];

pub struct RimWorldPlugin;

impl GamePlugin for RimWorldPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "rimworld",
            display_name: "RimWorld",
            nexus_domain: "rimworld",
            match_names: &["rimworld"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        RIMWORLD_ROOT_DIRS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        looks_like_rimworld_mod(content_root)
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_mods_folder(install_path, relative))
    }
}

fn looks_like_rimworld_mod(content_root: &Path) -> bool {
    content_root.join("About").join("About.xml").is_file()
        || content_root.join("About").join("about.xml").is_file()
}

/// Mods live next to the game binary, or inside the macOS `.app` bundle.
pub fn mods_root(install_path: &Path) -> PathBuf {
    let name = install_path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if name.ends_with(".app") {
        return install_path.join("Mods");
    }
    let bundled = install_path.join("RimWorldMac.app");
    if bundled.is_dir() {
        return bundled.join("Mods");
    }
    install_path.join("Mods")
}

fn resolve_mods_folder(install_path: &Path, relative: &Path) -> PathBuf {
    let root = mods_root(install_path);
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
    fn wraps_about_xml_and_deploys_under_mods() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("About")).unwrap();
        std::fs::write(dir.path().join("About").join("About.xml"), "<xml/>").unwrap();
        assert!(RimWorldPlugin.should_wrap_as_mod_folder(dir.path()));
        let dest = RimWorldPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("CoolMod/About/About.xml"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/CoolMod/About/About.xml"));
    }

    #[test]
    fn uses_macos_app_bundle_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("RimWorldMac.app");
        std::fs::create_dir_all(&app).unwrap();
        let dest = RimWorldPlugin
            .resolve_deploy_root(&app, Path::new("Cool/About/About.xml"))
            .unwrap();
        assert_eq!(dest, app.join("Mods").join("Cool").join("About").join("About.xml"));
    }
}
