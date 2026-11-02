//! No Man's Sky (GAMEDATA/MODS/<mod>/). Legacy PCBANKS/MODS is not used.
//! Some mods (e.g. Absolute Flight) also ship `Binaries/winmm.dll` at the install root.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

pub const NMS_ROOT_DIRS: &[&str] = &["GAMEDATA", "MODS", "Binaries"];

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
        !looks_like_merge_install_layout(content_root)
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        let mut out = Vec::new();
        if !install_path.join("GAMEDATA").is_dir() {
            out.push(
                "GAMEDATA folder not found. No Man's Sky mods deploy under GAMEDATA/MODS (not PCBANKS/MODS)."
                    .into(),
            );
        }
        if install_path
            .join("GAMEDATA")
            .join("PCBANKS")
            .join("MODS")
            .is_dir()
        {
            out.push(
                "GAMEDATA/PCBANKS/MODS is a deprecated imposter folder (since 5.50). \
                 Mods must be under GAMEDATA/MODS only."
                    .into(),
            );
        }
        if mods_all_disabled(install_path) {
            out.push(
                "Binaries/SETTINGS/GCMODSETTINGS.MXML has DisableAllMods enabled. \
                 Delete that file or set DisableAllMods to false, then restart the game."
                    .into(),
            );
        }
        if wrong_binaries_mods_path(install_path) {
            out.push(
                "Found GAMEDATA/MODS/Binaries from an older deploy. Purge, redeploy, \
                 then delete that folder if it remains."
                    .into(),
            );
        }
        out.extend(super::wine_overrides::winmm_launch_option_warnings(
            install_path,
            "winmm-based mods (e.g. Absolute Flight)",
        ));
        out
    }

    fn staging_deploy_warnings(&self, content_root: &Path, _mod_name: &str) -> Vec<String> {
        let mut out = Vec::new();
        if content_root.join("Binaries").join("winmm.dll").is_file()
            && cfg!(target_os = "linux")
        {
            out.push(
                "This mod ships Binaries/winmm.dll. On Proton set \
                 WINEDLLOVERRIDES=\"winmm=n,b\" %command% (not winmm=n). Force Proton in Steam \
                 Compatibility — the native Linux build will not load this DLL."
                    .into(),
            );
        }
        out
    }

    fn after_deploy(
        &self,
        install_path: &Path,
        _enabled_mod_folders: &[String],
    ) -> Result<Vec<String>> {
        let mut out = Vec::new();
        if wrong_binaries_mods_path(install_path) {
            out.push(
                "Remove the leftover GAMEDATA/MODS/Binaries folder (from a previous deploy), \
                 then purge and deploy again so winmm.dll lands in Binaries/ at the game root."
                    .into(),
            );
        }
        out.extend(super::wine_overrides::ensure_winmm_launch_options(
            install_path,
            "Check in-game Controls for Absolute Flight setup when the hook loads.",
        ));
        Ok(out)
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_nms_deploy(install_path, relative))
    }
}

fn looks_like_merge_install_layout(content_root: &Path) -> bool {
    content_root.join("GAMEDATA").is_dir()
        || content_root.join("MODS").is_dir()
        || content_root.join("Binaries").is_dir()
}

fn wrong_binaries_mods_path(install_path: &Path) -> bool {
    install_path
        .join("GAMEDATA")
        .join("MODS")
        .join("Binaries")
        .is_dir()
}

fn mods_all_disabled(install_path: &Path) -> bool {
    let path = install_path
        .join("Binaries")
        .join("SETTINGS")
        .join("GCMODSETTINGS.MXML");
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let lower = text.to_lowercase();
    lower.contains("disableallmods") && lower.contains("true")
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
    if first_lower == "gamedata" || first_lower == "binaries" {
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
    use crate::games::normalize_staging_root;

    #[test]
    fn wraps_loose_mod_under_gamedata_mods() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("foo.EXML"), b"x").unwrap();
        assert!(NoMansSkyPlugin.should_wrap_as_mod_folder(dir.path()));
        let dest = NoMansSkyPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("CoolMod/foo.EXML"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/GAMEDATA/MODS/CoolMod/foo.EXML"));
    }

    #[test]
    fn preserves_gamedata_and_mods_prefix() {
        let dest = NoMansSkyPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("GAMEDATA/MODS/Cool/foo.MBIN"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/GAMEDATA/MODS/Cool/foo.MBIN"));
        let dest = NoMansSkyPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("MODS/Cool/foo.MBIN"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/GAMEDATA/MODS/Cool/foo.MBIN"));
    }

    #[test]
    fn routes_binaries_to_install_root() {
        let dest = NoMansSkyPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("Binaries/winmm.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Binaries/winmm.dll"));
        let dest = NoMansSkyPlugin
            .resolve_deploy_root(
                Path::new("/game"),
                Path::new("Binaries/SETTINGS/GCMODSETTINGS.MXML"),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/Binaries/SETTINGS/GCMODSETTINGS.MXML")
        );
    }

    #[test]
    fn absolute_flight_layout() {
        let dest = NoMansSkyPlugin
            .resolve_deploy_root(
                Path::new("/game"),
                Path::new("GAMEDATA/MODS/AbsoluteFlight/GCSPACESHIPGLOBALS.GLOBAL.EXML"),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from(
                "/game/GAMEDATA/MODS/AbsoluteFlight/GCSPACESHIPGLOBALS.GLOBAL.EXML"
            )
        );
        let dest = NoMansSkyPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("Binaries/winmm.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Binaries/winmm.dll"));
    }

    #[test]
    fn merge_layout_skips_mod_folder_wrap() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("Binaries")).unwrap();
        std::fs::create_dir_all(dir.path().join("GAMEDATA/MODS/AbsoluteFlight")).unwrap();
        assert!(!NoMansSkyPlugin.should_wrap_as_mod_folder(dir.path()));
    }

    #[test]
    fn preserves_binaries_staging_root() {
        let staging = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(staging.path().join("Binaries")).unwrap();
        std::fs::write(staging.path().join("Binaries/winmm.dll"), b"x").unwrap();
        let root = normalize_staging_root(staging.path(), &NoMansSkyPlugin).unwrap();
        assert_eq!(root, staging.path());
    }

    #[test]
    fn warns_without_gamedata() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(NoMansSkyPlugin.preflight_warnings(empty.path()).len(), 1);
        std::fs::create_dir_all(empty.path().join("GAMEDATA")).unwrap();
        assert!(NoMansSkyPlugin.preflight_warnings(empty.path()).is_empty());
    }

    #[test]
    fn preflight_disable_all_mods() {
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("GAMEDATA")).unwrap();
        let settings = install.path().join("Binaries/SETTINGS");
        std::fs::create_dir_all(&settings).unwrap();
        std::fs::write(
            settings.join("GCMODSETTINGS.MXML"),
            "<DisableAllMods>true</DisableAllMods>",
        )
        .unwrap();
        let warnings = NoMansSkyPlugin.preflight_warnings(install.path());
        assert!(warnings.iter().any(|w| w.contains("DisableAllMods")));
    }

    #[test]
    fn preflight_wrong_binaries_under_mods() {
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("GAMEDATA")).unwrap();
        std::fs::create_dir_all(install.path().join("GAMEDATA/MODS/Binaries")).unwrap();
        let warnings = NoMansSkyPlugin.preflight_warnings(install.path());
        assert!(warnings.iter().any(|w| w.contains("GAMEDATA/MODS/Binaries")));
    }

}
