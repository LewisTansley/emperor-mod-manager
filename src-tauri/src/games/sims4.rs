//! The Sims 4 — mods live in user data, not the install.
//!
//! ```text
//! Documents/Electronic Arts/The Sims 4/Mods/<ModName>/...
//! Documents/Electronic Arts/The Sims 4/Mods/Resource.cfg
//! ```
//!
//! Two loader rules drive the layout:
//! - `.package` files load up to five folders below `Mods`, as declared by
//!   `Resource.cfg`.
//! - `.ts4script` files load at most **one** folder below `Mods`, regardless of
//!   `Resource.cfg`.
//!
//! Every mod is therefore wrapped in `Mods/<ModName>/`: one level satisfies both
//! rules and keeps mods from overwriting each other. Scripts nested deeper inside
//! an archive are flattened up into the wrap folder.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{
    content_folder_wrap_name, normalize_relative,
    sims_framework::{clear_caches, ensure_resource_cfg},
    GamePlugin, GamePluginInfo,
};

const STEAM_APP_ID: &str = "1222670";

pub const SIMS4_ROOT_DIRS: &[&str] = &["Mods"];

/// Only these are read by the game; everything else in an archive is noise.
const LOADABLE_EXTENSIONS: &[&str] = &["package", "ts4script"];

/// Written when the Mods folder has no `Resource.cfg` of its own.
const RESOURCE_CFG: &str = "Priority 500
PackedFile *.package
PackedFile */*.package
PackedFile */*/*.package
PackedFile */*/*/*.package
PackedFile */*/*/*/*.package
";

/// Depth marker that tells us an existing cfg already scans nested folders.
const DEEPEST_PACKED_FILE: &str = "*/*/*/*/*.package";

pub struct Sims4Plugin;

impl GamePlugin for Sims4Plugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "sims4",
            display_name: "The Sims 4",
            nexus_domain: "thesims4",
            match_names: &["the sims 4", "sims 4", "sims4"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        true
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        SIMS4_ROOT_DIRS
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn should_deploy_file(&self, relative: &Path) -> bool {
        is_loadable(relative)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        let Some(user_dir) = user_data_dir(install_path) else {
            return vec![
                "Could not resolve the Sims 4 user-data folder (Documents/Electronic Arts/The Sims 4). Mods will not load until that path exists."
                    .into(),
            ];
        };
        let mut warnings = Vec::new();
        if !user_dir.is_dir() {
            warnings.push(format!(
                "Sims 4 user-data folder not found at {}. Run the game once so it creates the folder, then deploy again.",
                user_dir.display()
            ));
        }
        if !script_mods_enabled(&user_dir) {
            warnings.push(
                "Script mods appear to be disabled. Enable Game Options → Other → Enable Custom Content and Mods plus Script Mods Allowed, or .ts4script mods will not run."
                    .into(),
            );
        }
        warnings
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        let root = mods_dir(install_path);
        let relative = strip_mods_prefix(&normalize_relative(relative));
        if relative.as_os_str().is_empty() {
            return Ok(root);
        }
        Ok(root.join(flatten_script_path(&relative)))
    }

    fn after_deploy(
        &self,
        install_path: &Path,
        _enabled_mod_folders: &[String],
    ) -> Result<Vec<String>> {
        let Some(user_dir) = user_data_dir(install_path) else {
            return Ok(Vec::new());
        };
        let warnings =
            ensure_resource_cfg(&user_dir.join("Mods"), RESOURCE_CFG, DEEPEST_PACKED_FILE)?;
        // The thumbnail cache goes stale whenever CC changes; the game rebuilds it.
        clear_caches(&[user_dir.join("localthumbcache.package")]);
        Ok(warnings)
    }
}

fn user_data_dir(install_path: &Path) -> Option<PathBuf> {
    super::user_data::documents_dir(install_path, STEAM_APP_ID)
        .map(|docs| docs.join("Electronic Arts").join("The Sims 4"))
}

fn mods_dir(install_path: &Path) -> PathBuf {
    match user_data_dir(install_path) {
        Some(user_dir) => user_dir.join("Mods"),
        None => install_path.join("Mods"),
    }
}

fn is_loadable(relative: &Path) -> bool {
    relative
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            LOADABLE_EXTENSIONS
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
}

fn is_script(relative: &Path) -> bool {
    relative
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("ts4script"))
}

fn path_component_names(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

/// Drop everything up to and including a `Mods` component.
///
/// Archives that ship their own `Mods/` tree keep that layout, whether or not the
/// staged mod was also wrapped (`<ModName>/Mods/...`).
fn strip_mods_prefix(relative: &Path) -> PathBuf {
    let names = path_component_names(relative);
    let mods_at = names
        .iter()
        .position(|name| name.eq_ignore_ascii_case("Mods"));
    let kept = match mods_at {
        Some(index) => &names[index + 1..],
        None => &names[..],
    };
    kept.iter().fold(PathBuf::new(), |mut acc, name| {
        acc.push(name);
        acc
    })
}

/// Pull a `.ts4script` up to one folder below `Mods`, folding the directories it
/// came from into the file name so two scripts can never collide.
fn flatten_script_path(relative: &Path) -> PathBuf {
    if !is_script(relative) {
        return relative.to_path_buf();
    }
    let names = path_component_names(relative);
    // `<ModName>/script.ts4script` is already at the deepest allowed level.
    if names.len() <= 2 {
        return relative.to_path_buf();
    }
    let (wrap, rest) = names.split_first().expect("checked len");
    Path::new(wrap).join(rest.join("_"))
}

/// `Options.ini` is written by the game; we only read it.
fn script_mods_enabled(user_dir: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(user_dir.join("Options.ini")) else {
        // No Options.ini yet (game never launched) — the preflight already warns
        // about the missing user-data folder, so don't add a second warning.
        return true;
    };
    for line in raw.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("scriptmodsenabled") {
            return value.trim() != "0";
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steam_install(tmp: &Path) -> PathBuf {
        let install = tmp.join("steamapps").join("common").join("The Sims 4");
        std::fs::create_dir_all(&install).unwrap();
        install
    }

    fn proton_user_dir(tmp: &Path) -> PathBuf {
        tmp.join("steamapps")
            .join("compatdata")
            .join(STEAM_APP_ID)
            .join("pfx")
            .join("drive_c")
            .join("users")
            .join("steamuser")
            .join("Documents")
            .join("Electronic Arts")
            .join("The Sims 4")
    }

    #[test]
    fn packages_land_one_folder_below_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let install = steam_install(tmp.path());
        let dest = Sims4Plugin
            .resolve_deploy_root(&install, Path::new("MC Command Center/mccc.package"))
            .unwrap();
        assert_eq!(
            dest,
            proton_user_dir(tmp.path())
                .join("Mods")
                .join("MC Command Center")
                .join("mccc.package")
        );
    }

    #[test]
    fn nested_packages_keep_their_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let install = steam_install(tmp.path());
        let dest = Sims4Plugin
            .resolve_deploy_root(&install, Path::new("CC Pack/Hair/Female/wavy.package"))
            .unwrap();
        assert_eq!(
            dest,
            proton_user_dir(tmp.path())
                .join("Mods")
                .join("CC Pack")
                .join("Hair")
                .join("Female")
                .join("wavy.package")
        );
    }

    #[test]
    fn deep_scripts_are_flattened_into_the_wrap_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let install = steam_install(tmp.path());
        let mods = proton_user_dir(tmp.path()).join("Mods");

        // Already one deep: untouched.
        assert_eq!(
            Sims4Plugin
                .resolve_deploy_root(&install, Path::new("MCCC/mc_cmd_center.ts4script"))
                .unwrap(),
            mods.join("MCCC").join("mc_cmd_center.ts4script")
        );

        // Too deep to load: folded into the file name, so siblings cannot collide.
        assert_eq!(
            Sims4Plugin
                .resolve_deploy_root(&install, Path::new("MCCC/scripts/core/mod.ts4script"))
                .unwrap(),
            mods.join("MCCC").join("scripts_core_mod.ts4script")
        );
        assert_eq!(
            Sims4Plugin
                .resolve_deploy_root(&install, Path::new("MCCC/extras/core/mod.ts4script"))
                .unwrap(),
            mods.join("MCCC").join("extras_core_mod.ts4script")
        );
    }

    #[test]
    fn archives_shipping_their_own_mods_folder_keep_that_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let install = steam_install(tmp.path());
        let dest = Sims4Plugin
            .resolve_deploy_root(&install, Path::new("Some Pack/Mods/CoolMod/thing.package"))
            .unwrap();
        assert_eq!(
            dest,
            proton_user_dir(tmp.path())
                .join("Mods")
                .join("CoolMod")
                .join("thing.package")
        );
    }

    #[test]
    fn only_loadable_files_deploy() {
        assert!(Sims4Plugin.should_deploy_file(Path::new("Mod/thing.package")));
        assert!(Sims4Plugin.should_deploy_file(Path::new("Mod/thing.TS4SCRIPT")));
        assert!(!Sims4Plugin.should_deploy_file(Path::new("Mod/README.txt")));
        assert!(!Sims4Plugin.should_deploy_file(Path::new("Mod/preview.jpg")));
        assert!(!Sims4Plugin.should_deploy_file(Path::new("Mod/loose_script.py")));
    }

    #[test]
    fn after_deploy_writes_resource_cfg_and_clears_thumbnail_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let install = steam_install(tmp.path());
        let user_dir = proton_user_dir(tmp.path());
        std::fs::create_dir_all(&user_dir).unwrap();
        let cache = user_dir.join("localthumbcache.package");
        std::fs::write(&cache, b"stale").unwrap();

        let warnings = Sims4Plugin.after_deploy(&install, &[]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");

        let cfg = std::fs::read_to_string(user_dir.join("Mods").join("Resource.cfg")).unwrap();
        assert!(cfg.contains("PackedFile */*/*/*/*.package"));
        assert!(!cache.exists(), "stale thumbnail cache should be removed");
    }

    #[test]
    fn after_deploy_keeps_a_custom_resource_cfg_but_warns_when_shallow() {
        let tmp = tempfile::tempdir().unwrap();
        let install = steam_install(tmp.path());
        let mods = proton_user_dir(tmp.path()).join("Mods");
        std::fs::create_dir_all(&mods).unwrap();
        let cfg = mods.join("Resource.cfg");
        std::fs::write(&cfg, "Priority 500\nPackedFile *.package\n").unwrap();

        let warnings = Sims4Plugin.after_deploy(&install, &[]).unwrap();
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("does not scan nested folders"));
        assert_eq!(
            std::fs::read_to_string(&cfg).unwrap(),
            "Priority 500\nPackedFile *.package\n",
            "a user-authored cfg must not be overwritten"
        );
    }

    #[test]
    fn warns_when_script_mods_are_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let install = steam_install(tmp.path());
        let user_dir = proton_user_dir(tmp.path());
        std::fs::create_dir_all(&user_dir).unwrap();

        std::fs::write(user_dir.join("Options.ini"), "scriptmodsenabled = 0\n").unwrap();
        let warnings = Sims4Plugin.preflight_warnings(&install);
        assert!(
            warnings.iter().any(|w| w.contains("Script mods")),
            "{warnings:?}"
        );

        std::fs::write(user_dir.join("Options.ini"), "scriptmodsenabled = 1\n").unwrap();
        assert!(Sims4Plugin.preflight_warnings(&install).is_empty());
    }
}
