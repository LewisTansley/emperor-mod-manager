//! No Man's Sky (GAMEDATA/MODS/<mod>/). Legacy PCBANKS/MODS is not used.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

pub const NMS_ROOT_DIRS: &[&str] = &["GAMEDATA", "MODS"];

pub struct NoMansSkyPlugin;

impl GamePlugin for NoMansSkyPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "nomanssky",
            display_name: "No Man's Sky",
            nexus_domain: "nomanssky",
            match_names: &["no man's sky", "nomanssky", "no mans sky"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        NMS_ROOT_DIRS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        !looks_like_gamedata_tree(content_root)
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        if install_path.join("GAMEDATA").is_dir() {
            Vec::new()
        } else {
            vec![
                "GAMEDATA folder not found. No Man's Sky mods deploy under GAMEDATA/MODS (not PCBANKS/MODS)."
                    .into(),
            ]
        }
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_nms_deploy(install_path, relative))
    }
}

fn looks_like_gamedata_tree(content_root: &Path) -> bool {
    content_root.join("GAMEDATA").is_dir() || content_root.join("MODS").is_dir()
}

fn resolve_nms_deploy(install_path: &Path, relative: &Path) -> PathBuf {
    let mods = install_path.join("GAMEDATA").join("MODS");
    let normalized = normalize_relative(relative);
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect();
    if originals.is_empty() {
        return mods;
    }
    let first = originals[0].to_string_lossy();
    let first_lower = first.to_lowercase();
    if first_lower == "gamedata" {
        let mut out = install_path.to_path_buf();
        for orig in originals {
            out.push(orig);
        }
        return out;
    }
    if first_lower == "mods" {
        let mut out = mods;
        for orig in originals.into_iter().skip(1) {
            out.push(orig);
        }
        return out;
    }
    mods.join(&normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_loose_mod_under_gamedata_mods() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("foo.EXML"), b"x").unwrap();
        assert!(NoMansSkyPlugin.should_wrap_as_mod_folder(dir.path()));
        let dest = NoMansSkyPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("CoolMod/foo.EXML"))
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/GAMEDATA/MODS/CoolMod/foo.EXML")
        );
    }

    #[test]
    fn preserves_gamedata_and_mods_prefix() {
        let dest = NoMansSkyPlugin
            .resolve_deploy_root(
                Path::new("/game"),
                Path::new("GAMEDATA/MODS/Cool/foo.MBIN"),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/GAMEDATA/MODS/Cool/foo.MBIN")
        );
        let dest = NoMansSkyPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("MODS/Cool/foo.MBIN"))
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/GAMEDATA/MODS/Cool/foo.MBIN")
        );
    }

    #[test]
    fn warns_without_gamedata() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(NoMansSkyPlugin.preflight_warnings(empty.path()).len(), 1);
        std::fs::create_dir_all(empty.path().join("GAMEDATA")).unwrap();
        assert!(NoMansSkyPlugin.preflight_warnings(empty.path()).is_empty());
    }
}
