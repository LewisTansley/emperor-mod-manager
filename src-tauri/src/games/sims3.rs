//! The Sims 3 — mods live in the user-data "framework" folder, not the install.
//!
//! ```text
//! Documents/Electronic Arts/The Sims 3/Mods/Resource.cfg
//! Documents/Electronic Arts/The Sims 3/Mods/Packages/<ModName>/...
//! Documents/Electronic Arts/The Sims 3/Mods/Overrides/...
//! Documents/Electronic Arts/The Sims 3/DCCache/*.dbc
//! Documents/Electronic Arts/The Sims 3/Downloads/*.sims3pack
//! ```
//!
//! `Resource.cfg` is what makes the game read `Mods/` at all, so it is written
//! when missing. Ordinary `.package` mods are wrapped in `Packages/<ModName>/`;
//! archives that already ship a framework tree keep their own layout, which is how
//! core mods reach the higher-priority `Overrides/` folder.
//!
//! `.sims3pack` files cannot be read by the game — only the Sims 3 Launcher can
//! install them — so they are placed in `Downloads/` for the Launcher to pick up.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{
    content_folder_wrap_name, normalize_relative,
    sims_framework::{clear_caches, clear_dir_files, ensure_resource_cfg},
    GamePlugin, GamePluginInfo,
};

const STEAM_APP_ID: &str = "47890";

pub const SIMS3_ROOT_DIRS: &[&str] = &["Mods", "Packages", "Overrides", "DCCache"];

/// Only these are read by the game or the Launcher.
const LOADABLE_EXTENSIONS: &[&str] = &["package", "dbc", "sims3pack"];

/// The community-standard framework file (Overrides outranks Packages).
const RESOURCE_CFG: &str = "Priority 501
DirectoryFiles Files/... autoupdate

Priority 1000
PackedFile Overrides/*.package
PackedFile Overrides/*/*.package
PackedFile Overrides/*/*/*.package
PackedFile Overrides/*/*/*/*.package
PackedFile Overrides/*/*/*/*/*.package

Priority 500
PackedFile Packages/*.package
PackedFile Packages/*/*.package
PackedFile Packages/*/*/*.package
PackedFile Packages/*/*/*/*.package
PackedFile Packages/*/*/*/*/*.package

Priority 500
PackedFile DCCache/*.dbc
";

/// Depth marker that tells us an existing cfg already scans nested folders.
const DEEPEST_PACKED_FILE: &str = "Packages/*/*.package";

/// Caches the game rebuilds; stale ones make new mods look broken.
const STALE_CACHES: &[&str] = &[
    "CASPartCache.package",
    "compositorCache.package",
    "scriptCache.package",
    "simCompositorCache.package",
    "socialCache.package",
];

pub struct Sims3Plugin;

impl GamePlugin for Sims3Plugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "sims3",
            display_name: "The Sims 3",
            nexus_domain: "thesims3",
            match_names: &["the sims 3", "sims 3", "sims3"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        true
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        SIMS3_ROOT_DIRS
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn should_deploy_file(&self, relative: &Path) -> bool {
        has_extension(relative, LOADABLE_EXTENSIONS)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        let Some(user_dir) = user_data_dir(install_path) else {
            return vec![
                "Could not resolve the Sims 3 user-data folder (Documents/Electronic Arts/The Sims 3). Mods will not load until that path exists."
                    .into(),
            ];
        };
        if user_dir.is_dir() {
            Vec::new()
        } else {
            vec![format!(
                "Sims 3 user-data folder not found at {}. Run the game once so it creates the folder, then deploy again.",
                user_dir.display()
            )]
        }
    }

    fn staging_deploy_warnings(&self, content_root: &Path, mod_name: &str) -> Vec<String> {
        if contains_sims3pack(content_root) {
            vec![format!(
                "{mod_name} contains .sims3pack files, which the game cannot read. They were placed in Downloads/ — open the Sims 3 Launcher and install them from its Downloads tab. Removing this mod later will not undo what the Launcher installed."
            )]
        } else {
            Vec::new()
        }
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        let user_dir = user_data_dir(install_path).unwrap_or_else(|| install_path.to_path_buf());
        let mods = user_dir.join("Mods");
        let relative = normalize_relative(relative);

        // A framework root anywhere in the path wins: the author chose the layout.
        if let Some((root, rest)) = split_at_framework_root(&relative) {
            let base = match root.as_str() {
                "mods" => mods,
                "packages" => mods.join("Packages"),
                "overrides" => mods.join("Overrides"),
                _ => user_dir.join("DCCache"),
            };
            return Ok(base.join(rest));
        }

        // Launcher-only and cache formats are flat folders, so drop any nesting.
        if has_extension(&relative, &["sims3pack"]) {
            return Ok(user_dir.join("Downloads").join(file_name_of(&relative)));
        }
        if has_extension(&relative, &["dbc"]) {
            return Ok(user_dir.join("DCCache").join(file_name_of(&relative)));
        }

        Ok(mods.join("Packages").join(&relative))
    }

    fn after_deploy(
        &self,
        install_path: &Path,
        _enabled_mod_folders: &[String],
    ) -> Result<Vec<String>> {
        let Some(user_dir) = user_data_dir(install_path) else {
            return Ok(Vec::new());
        };
        let mods = user_dir.join("Mods");
        let warnings = ensure_resource_cfg(&mods, RESOURCE_CFG, DEEPEST_PACKED_FILE)?;
        // The framework expects both folders to exist even when one is empty.
        for folder in ["Packages", "Overrides"] {
            std::fs::create_dir_all(mods.join(folder))?;
        }
        let stale: Vec<PathBuf> = STALE_CACHES
            .iter()
            .map(|name| user_dir.join(name))
            .collect();
        clear_caches(&stale);
        clear_dir_files(&user_dir.join("WorldCaches"));
        Ok(warnings)
    }
}

fn user_data_dir(install_path: &Path) -> Option<PathBuf> {
    super::user_data::documents_dir(install_path, STEAM_APP_ID)
        .map(|docs| docs.join("Electronic Arts").join("The Sims 3"))
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            extensions
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
}

fn file_name_of(path: &Path) -> PathBuf {
    path.file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| path.to_path_buf())
}

/// Split a staged path at the first framework folder, returning it lowercased
/// along with everything below it.
fn split_at_framework_root(relative: &Path) -> Option<(String, PathBuf)> {
    let names: Vec<String> = relative
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let index = names.iter().position(|name| {
        SIMS3_ROOT_DIRS
            .iter()
            .any(|root| name.eq_ignore_ascii_case(root))
    })?;
    let rest = names[index + 1..]
        .iter()
        .fold(PathBuf::new(), |mut acc, name| {
            acc.push(name);
            acc
        });
    Some((names[index].to_lowercase(), rest))
}

fn contains_sims3pack(content_root: &Path) -> bool {
    walkdir::WalkDir::new(content_root)
        .into_iter()
        .flatten()
        .any(|entry| entry.file_type().is_file() && has_extension(entry.path(), &["sims3pack"]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steam_install(tmp: &Path) -> PathBuf {
        let install = tmp.join("steamapps").join("common").join("The Sims 3");
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
            .join("The Sims 3")
    }

    #[test]
    fn packages_wrap_under_the_packages_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let install = steam_install(tmp.path());
        let dest = Sims3Plugin
            .resolve_deploy_root(&install, Path::new("NRaas Overwatch/overwatch.package"))
            .unwrap();
        assert_eq!(
            dest,
            proton_user_dir(tmp.path())
                .join("Mods")
                .join("Packages")
                .join("NRaas Overwatch")
                .join("overwatch.package")
        );
    }

    #[test]
    fn framework_archives_keep_overrides_and_packages() {
        let tmp = tempfile::tempdir().unwrap();
        let install = steam_install(tmp.path());
        let mods = proton_user_dir(tmp.path()).join("Mods");

        assert_eq!(
            Sims3Plugin
                .resolve_deploy_root(&install, Path::new("Pack/Mods/Overrides/core.package"))
                .unwrap(),
            mods.join("Overrides").join("core.package")
        );
        assert_eq!(
            Sims3Plugin
                .resolve_deploy_root(&install, Path::new("Pack/Overrides/core.package"))
                .unwrap(),
            mods.join("Overrides").join("core.package")
        );
        assert_eq!(
            Sims3Plugin
                .resolve_deploy_root(&install, Path::new("Pack/Mods/Packages/mod.package"))
                .unwrap(),
            mods.join("Packages").join("mod.package")
        );
    }

    #[test]
    fn launcher_and_cache_formats_go_to_their_flat_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let install = steam_install(tmp.path());
        let user_dir = proton_user_dir(tmp.path());

        assert_eq!(
            Sims3Plugin
                .resolve_deploy_root(&install, Path::new("Set/extras/hair.sims3pack"))
                .unwrap(),
            user_dir.join("Downloads").join("hair.sims3pack")
        );
        assert_eq!(
            Sims3Plugin
                .resolve_deploy_root(&install, Path::new("Set/store.dbc"))
                .unwrap(),
            user_dir.join("DCCache").join("store.dbc")
        );
    }

    #[test]
    fn only_loadable_files_deploy() {
        assert!(Sims3Plugin.should_deploy_file(Path::new("Mod/thing.package")));
        assert!(Sims3Plugin.should_deploy_file(Path::new("Mod/thing.Sims3Pack")));
        assert!(Sims3Plugin.should_deploy_file(Path::new("Mod/store.dbc")));
        assert!(!Sims3Plugin.should_deploy_file(Path::new("Mod/readme.pdf")));
        assert!(!Sims3Plugin.should_deploy_file(Path::new("Mod/preview.png")));
    }

    #[test]
    fn after_deploy_writes_framework_and_clears_caches() {
        let tmp = tempfile::tempdir().unwrap();
        let install = steam_install(tmp.path());
        let user_dir = proton_user_dir(tmp.path());
        std::fs::create_dir_all(user_dir.join("WorldCaches")).unwrap();
        std::fs::write(user_dir.join("scriptCache.package"), b"stale").unwrap();
        std::fs::write(user_dir.join("WorldCaches").join("Sunset.bin"), b"stale").unwrap();

        let warnings = Sims3Plugin.after_deploy(&install, &[]).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");

        let cfg = std::fs::read_to_string(user_dir.join("Mods").join("Resource.cfg")).unwrap();
        assert!(cfg.contains("PackedFile Overrides/*.package"));
        assert!(cfg.contains("PackedFile Packages/*/*.package"));
        assert!(cfg.contains("PackedFile DCCache/*.dbc"));
        assert!(user_dir.join("Mods").join("Packages").is_dir());
        assert!(user_dir.join("Mods").join("Overrides").is_dir());
        assert!(!user_dir.join("scriptCache.package").exists());
        assert!(!user_dir.join("WorldCaches").join("Sunset.bin").exists());
        assert!(user_dir.join("WorldCaches").is_dir());
    }

    #[test]
    fn sims3pack_mods_warn_about_the_launcher() {
        let staging = tempfile::tempdir().unwrap();
        std::fs::write(staging.path().join("hair.sims3pack"), b"x").unwrap();
        let warnings = Sims3Plugin.staging_deploy_warnings(staging.path(), "Hair Set");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Sims 3 Launcher"));

        let plain = tempfile::tempdir().unwrap();
        std::fs::write(plain.path().join("mod.package"), b"x").unwrap();
        assert!(Sims3Plugin
            .staging_deploy_warnings(plain.path(), "Mod")
            .is_empty());
    }
}
