//! My Summer Car (MSCLoader, not BepInEx).

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

pub const MSC_ROOT_DIRS: &[&str] = &["Mods"];

const LOADER_MARKERS: &[&str] = &[
    "MSCLoader.dll",
    "winhttp.dll",
    "doorstop_config.ini",
    "doorstop_libs",
];

pub struct MySummerCarPlugin;

impl GamePlugin for MySummerCarPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "mysummercar",
            display_name: "My Summer Car",
            nexus_domain: "mysummercar",
            match_names: &["my summer car", "mysummercar"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        MSC_ROOT_DIRS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        !looks_like_mscloader_installer(content_root) && looks_like_msc_mod(content_root)
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn deploys_to_install_root(&self, content_root: &Path) -> bool {
        looks_like_mscloader_installer(content_root)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        if mscloader_present(install_path) {
            Vec::new()
        } else {
            vec![
                "MSCLoader not found in the game folder. Mods will not load until MSCLoader is installed."
                    .into(),
            ]
        }
    }

    fn staging_deploy_warnings(&self, content_root: &Path, mod_name: &str) -> Vec<String> {
        if looks_like_mscloader_installer(content_root) {
            vec![format!(
                "{mod_name} looks like the MSCLoader installer; files were deployed to the game root (not Mods/)."
            )]
        } else {
            Vec::new()
        }
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        let normalized = normalize_relative(relative);
        let originals: Vec<_> = normalized
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_os_string()),
                _ => None,
            })
            .collect();
        if originals.is_empty() {
            return Ok(install_path.join("Mods"));
        }
        let first = originals[0].to_string_lossy();
        if first.eq_ignore_ascii_case("Mods") {
            let mut out = install_path.join("Mods");
            for orig in originals.into_iter().skip(1) {
                out.push(orig);
            }
            return Ok(out);
        }
        Ok(install_path.join("Mods").join(&normalized))
    }
}

fn mscloader_present(install_path: &Path) -> bool {
    install_path.join("MSCLoader.dll").is_file()
        || install_path.join("Mods").join("MSCLoader.dll").is_file()
}

fn looks_like_mscloader_installer(content_root: &Path) -> bool {
    let has_loader = content_root.join("MSCLoader.dll").is_file();
    if !has_loader {
        return false;
    }
    LOADER_MARKERS.iter().any(|m| {
        let p = content_root.join(m);
        p.is_file() || p.is_dir()
    }) && (content_root.join("winhttp.dll").is_file()
        || content_root.join("doorstop_config.ini").is_file()
        || content_root.join("doorstop_libs").is_dir())
}

fn looks_like_msc_mod(content_root: &Path) -> bool {
    if looks_like_mscloader_installer(content_root) {
        return false;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin() -> MySummerCarPlugin {
        MySummerCarPlugin
    }

    #[test]
    fn wraps_mod_dlls_under_mods() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CoolMod.dll"), b"x").unwrap();
        assert!(plugin().should_wrap_as_mod_folder(dir.path()));
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("CoolMod/CoolMod.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/CoolMod/CoolMod.dll"));
    }

    #[test]
    fn loader_deploys_to_install_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("MSCLoader.dll"), b"x").unwrap();
        std::fs::write(dir.path().join("winhttp.dll"), b"x").unwrap();
        assert!(plugin().deploys_to_install_root(dir.path()));
        assert!(!plugin().should_wrap_as_mod_folder(dir.path()));
    }

    #[test]
    fn warns_without_mscloader() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(plugin().preflight_warnings(empty.path()).len(), 1);
        std::fs::write(empty.path().join("MSCLoader.dll"), b"x").unwrap();
        assert!(plugin().preflight_warnings(empty.path()).is_empty());
    }
}
