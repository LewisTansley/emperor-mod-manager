use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

pub const STARDEW_ROOT_DIRS: &[&str] = &["Mods"];

pub struct StardewValleyPlugin;

impl GamePlugin for StardewValleyPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "stardewvalley",
            display_name: "Stardew Valley",
            nexus_domain: "stardewvalley",
            match_names: &["stardew valley", "stardewvalley", "stardew"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        STARDEW_ROOT_DIRS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        // Single SMAPI mod with manifest at the (possibly peeled) root.
        !looks_like_smapi_installer(content_root) && content_root.join("manifest.json").is_file()
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        // Prefer the author folder from the archive (after peel) over the Nexus title.
        content_folder_wrap_name(content_root, staged_name)
    }

    fn deploys_to_install_root(&self, content_root: &Path) -> bool {
        looks_like_smapi_installer(content_root)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        if smapi_present(install_path) {
            Vec::new()
        } else {
            vec![
                "SMAPI not found in the game folder. Mods will not load until SMAPI is installed."
                    .into(),
            ]
        }
    }

    fn staging_deploy_warnings(&self, content_root: &Path, mod_name: &str) -> Vec<String> {
        if looks_like_smapi_installer(content_root) {
            vec![format!(
                "{mod_name} looks like the SMAPI installer; files were deployed to the game root (not Mods/). Prefer the official SMAPI installer when possible."
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

        // Archives that already include Mods/<ModName>/...
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

/// True when the game install already has SMAPI.
pub fn smapi_present(install_path: &Path) -> bool {
    install_path.join("StardewModdingAPI.exe").is_file()
        || install_path.join("StardewModdingAPI.dll").is_file()
        || install_path.join("StardewModdingAPI").is_file()
        || install_path.join("smapi-internal").is_dir()
}

/// True when staged content looks like SMAPI itself (not a normal SMAPI mod).
pub fn looks_like_smapi_installer(content_root: &Path) -> bool {
    content_root.join("StardewModdingAPI.exe").is_file()
        || content_root.join("StardewModdingAPI.dll").is_file()
        || content_root.join("StardewModdingAPI").is_file()
        || content_root.join("smapi-internal").is_dir()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::GamePlugin;

    fn plugin() -> StardewValleyPlugin {
        StardewValleyPlugin
    }

    #[test]
    fn wraps_when_manifest_at_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), "{}").unwrap();
        assert!(plugin().should_wrap_as_mod_folder(dir.path()));
    }

    #[test]
    fn wrap_name_prefers_peeled_folder_over_nexus_title() {
        let staging = tempfile::tempdir().unwrap();
        let content = staging.path().join("CoolMod");
        std::fs::create_dir_all(&content).unwrap();
        std::fs::write(content.join("manifest.json"), "{}").unwrap();
        assert_eq!(
            plugin().wrap_mod_folder_name(&content, "Cool Mod"),
            "CoolMod"
        );
    }

    #[test]
    fn does_not_wrap_multi_mod_or_smapi() {
        let multi = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(multi.path().join("ModA")).unwrap();
        std::fs::write(multi.path().join("ModA").join("manifest.json"), "{}").unwrap();
        assert!(!plugin().should_wrap_as_mod_folder(multi.path()));

        let smapi = tempfile::tempdir().unwrap();
        std::fs::write(smapi.path().join("StardewModdingAPI.exe"), b"x").unwrap();
        std::fs::write(smapi.path().join("manifest.json"), "{}").unwrap();
        assert!(!plugin().should_wrap_as_mod_folder(smapi.path()));
        assert!(plugin().deploys_to_install_root(smapi.path()));
    }

    #[test]
    fn resolve_puts_plain_under_mods() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("CoolMod/manifest.json"))
                .unwrap(),
            PathBuf::from("/game/Mods/CoolMod/manifest.json")
        );
    }

    #[test]
    fn resolve_preserves_mods_prefix() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("Mods/CoolMod/manifest.json"))
                .unwrap(),
            PathBuf::from("/game/Mods/CoolMod/manifest.json")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("mods/CoolMod/i18n/default.json"))
                .unwrap(),
            PathBuf::from("/game/Mods/CoolMod/i18n/default.json")
        );
    }

    #[test]
    fn smapi_markers() {
        let install = tempfile::tempdir().unwrap();
        assert!(!smapi_present(install.path()));
        std::fs::create_dir_all(install.path().join("smapi-internal")).unwrap();
        assert!(smapi_present(install.path()));
        assert!(plugin().preflight_warnings(install.path()).is_empty());

        let empty = tempfile::tempdir().unwrap();
        assert_eq!(plugin().preflight_warnings(empty.path()).len(), 1);
    }

    #[test]
    fn staging_warning_for_smapi_installer() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("StardewModdingAPI.dll"), b"x").unwrap();
        let warns = plugin().staging_deploy_warnings(dir.path(), "SMAPI");
        assert_eq!(warns.len(), 1);
        assert!(warns[0].contains("SMAPI"));
    }
}
