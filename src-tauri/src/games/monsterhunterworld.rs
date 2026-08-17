//! Monster Hunter: World (Stracker's Loader + nativePC overlay).
//! Rise and Wilds use the shared RE Engine plugin instead.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{normalize_relative, GamePlugin, GamePluginInfo};

pub const MHW_ROOT_DIRS: &[&str] = &["nativepc", "nativePC", "reframework"];

const ROOT_DLLS: &[&str] = &["dinput8.dll", "openvr_api.dll", "openxr_loader.dll"];

pub struct MonsterHunterWorldPlugin;

impl GamePlugin for MonsterHunterWorldPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "monsterhunterworld",
            display_name: "Monster Hunter: World",
            nexus_domain: "monsterhunterworld",
            match_names: &[
                "monster hunter: world",
                "monster hunter world",
                "monsterhunterworld",
                "mh world",
            ],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        MHW_ROOT_DIRS
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        if strackers_loader_present(install_path) {
            Vec::new()
        } else {
            vec![
                "Stracker's Loader was not found. NativePC mods will not load until it is installed."
                    .into(),
            ]
        }
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_mhw_deploy(install_path, relative))
    }
}

fn strackers_loader_present(install_path: &Path) -> bool {
    let plugins = install_path.join("nativePC").join("plugins");
    if plugins.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&plugins) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.contains("stracker")
                    || (name.contains("loader") && name.ends_with(".dll"))
                {
                    return true;
                }
            }
        }
    }
    install_path.join("dinput8.dll").is_file()
}

fn resolve_mhw_deploy(install_path: &Path, relative: &Path) -> PathBuf {
    let normalized = normalize_relative(relative);
    let components = path_components_lower(&normalized);
    if components.is_empty() {
        return install_path.join("nativePC");
    }

    let first = components[0].as_str();
    if first == "nativepc" || first == "reframework" {
        return join_canonical(install_path, &normalized, &components);
    }

    let is_single = components.len() == 1;
    let leaf = components.last().map(|s| s.as_str()).unwrap_or("");
    if is_single && ROOT_DLLS.iter().any(|d| *d == leaf) {
        return install_path.join(normalized.file_name().unwrap_or(normalized.as_os_str()));
    }

    let mut out = install_path.join("nativePC");
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    for orig in originals {
        out.push(orig);
    }
    out
}

fn path_components_lower(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect()
}

fn join_canonical(install_path: &Path, normalized: &Path, lower_components: &[String]) -> PathBuf {
    let mut out = install_path.to_path_buf();
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();

    for (i, lower) in lower_components.iter().enumerate() {
        if i == 0 && *lower == "nativepc" {
            out.push("nativePC");
        } else if i == 0 && *lower == "reframework" {
            out.push("reframework");
        } else if let Some(orig) = originals.get(i) {
            out.push(orig);
        } else {
            out.push(lower.as_str());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin() -> MonsterHunterWorldPlugin {
        MonsterHunterWorldPlugin
    }

    #[test]
    fn nativepc_and_reframework_at_root() {
        let install = Path::new("/game");
        assert_eq!(
            plugin()
                .resolve_deploy_root(install, Path::new("nativePC/plugins/foo.dll"))
                .unwrap(),
            PathBuf::from("/game/nativePC/plugins/foo.dll")
        );
        assert_eq!(
            plugin()
                .resolve_deploy_root(install, Path::new("reframework/autorun/mod.lua"))
                .unwrap(),
            PathBuf::from("/game/reframework/autorun/mod.lua")
        );
    }

    #[test]
    fn loose_files_default_to_nativepc() {
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("pl/armor/a.mod3"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/nativePC/pl/armor/a.mod3"));
    }

    #[test]
    fn dinput8_at_install_root() {
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("dinput8.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/dinput8.dll"));
    }

    #[test]
    fn warns_without_loader() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(plugin().preflight_warnings(empty.path()).len(), 1);
        std::fs::write(empty.path().join("dinput8.dll"), b"x").unwrap();
        assert!(plugin().preflight_warnings(empty.path()).is_empty());
    }
}
