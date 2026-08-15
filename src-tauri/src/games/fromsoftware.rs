//! Shared deploy logic for FromSoftware titles (Dark Souls trilogy + Elden Ring).

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{normalize_relative, GamePlugin, GamePluginInfo};

/// Top-level FromSoft asset / overlay folders that must not be peeled as archive wrappers.
pub const FROMSOFT_ROOT_DIRS: &[&str] = &[
    "action",
    "chr",
    "event",
    "expression",
    "facegen",
    "font",
    "map",
    "menu",
    "mod",
    "mods",
    "movie",
    "msg",
    "mtd",
    "other",
    "param",
    "parts",
    "script",
    "sfx",
    "shader",
    "sound",
];

/// Known game executables under a Steam `Game/` subdirectory.
const GAME_SUBDIR_EXES: &[&str] = &[
    "eldenring.exe",
    "darksoulsiii.exe",
    "darksoulsii.exe",
    "darksouls.exe",
    "darksoulsremastered.exe",
];

/// Prefer `install_path/Game` when that subdirectory holds the real game root.
pub fn resolve_game_root(install_path: &Path) -> PathBuf {
    let game_subdir = install_path.join("Game");
    if !game_subdir.is_dir() {
        return install_path.to_path_buf();
    }
    if has_known_exe(&game_subdir) || !has_known_exe(install_path) {
        return game_subdir;
    }
    install_path.to_path_buf()
}

fn has_known_exe(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let lower = name.to_string_lossy().to_lowercase();
        if GAME_SUBDIR_EXES.iter().any(|exe| *exe == lower.as_str()) {
            return true;
        }
    }
    false
}

fn resolve_mods_deploy(install_path: &Path, relative: &Path) -> PathBuf {
    let root = resolve_game_root(install_path);
    let normalized = normalize_relative(relative);
    root.join("mods").join(normalized)
}

macro_rules! fromsoft_plugin {
    ($struct:ident, $id:expr, $display:expr, $domain:expr, $matches:expr) => {
        pub struct $struct;

        impl GamePlugin for $struct {
            fn info(&self) -> GamePluginInfo {
                GamePluginInfo {
                    id: $id,
                    display_name: $display,
                    nexus_domain: $domain,
                    match_names: $matches,
                }
            }

            fn prefers_mod_folder(&self) -> bool {
                true
            }

            fn preserve_staging_root_names(&self) -> &[&str] {
                FROMSOFT_ROOT_DIRS
            }

            fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
                Ok(resolve_mods_deploy(install_path, relative))
            }
        }
    };
}

fromsoft_plugin!(
    EldenRingPlugin,
    "eldenring",
    "Elden Ring",
    "eldenring",
    &["elden ring", "eldenring"]
);

fromsoft_plugin!(
    DarkSouls3Plugin,
    "darksouls3",
    "Dark Souls 3",
    "darksouls3",
    &["dark souls iii", "dark souls 3", "darksouls3", "darksoulsiii"]
);

fromsoft_plugin!(
    DarkSouls2Plugin,
    "darksouls2",
    "Dark Souls 2",
    "darksouls2",
    &[
        "scholar of the first sin",
        "dark souls ii",
        "dark souls 2",
        "darksouls2",
        "darksoulsii",
    ]
);

fromsoft_plugin!(
    DarkSoulsRemasteredPlugin,
    "darksoulsremastered",
    "Dark Souls Remastered",
    "darksoulsremastered",
    &["dark souls remastered", "darksoulsremastered"]
);

fromsoft_plugin!(
    DarkSoulsPlugin,
    "darksouls",
    "Dark Souls",
    "darksouls",
    &[
        "prepare to die",
        "dark souls: prepare",
        "dark souls prepare",
        "darksouls",
        "dark souls",
    ]
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::{match_plugin, normalize_staging_root};

    #[test]
    fn prefers_game_subdir_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("ELDEN RING");
        let game = install.join("Game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("eldenring.exe"), b"x").unwrap();
        assert_eq!(resolve_game_root(&install), game);
    }

    #[test]
    fn keeps_install_when_no_game_subdir() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("DARK SOULS REMASTERED");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("DarkSoulsRemastered.exe"), b"x").unwrap();
        assert_eq!(resolve_game_root(&install), install);
    }

    #[test]
    fn prefers_game_subdir_without_exe_heuristic() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("DARK SOULS III");
        let game = install.join("Game");
        std::fs::create_dir_all(&game).unwrap();
        // No exe written — still prefer Game/ when install root has no known exe.
        assert_eq!(resolve_game_root(&install), game);
    }

    #[test]
    fn deploy_lands_under_mods() {
        let install = Path::new("/games/ELDEN RING");
        let p = EldenRingPlugin;
        // Without a real Game/ on disk, resolve_game_root keeps install_path.
        let dest = p
            .resolve_deploy_root(install, Path::new("parts/a.partsbnd.dcx"))
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/games/ELDEN RING/mods/parts/a.partsbnd.dcx")
        );
    }

    #[test]
    fn deploy_uses_game_subdir_when_on_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("ELDEN RING");
        let game = install.join("Game");
        std::fs::create_dir_all(&game).unwrap();
        let p = EldenRingPlugin;
        let dest = p
            .resolve_deploy_root(&install, Path::new("MyMod/regulation.bin"))
            .unwrap();
        assert_eq!(dest, game.join("mods").join("MyMod").join("regulation.bin"));
    }

    #[test]
    fn peels_wrapper_but_keeps_parts() {
        let staging = tempfile::tempdir().unwrap();
        let wrapper = staging.path().join("Cool Armor Pack");
        std::fs::create_dir_all(wrapper.join("parts")).unwrap();
        std::fs::write(wrapper.join("parts").join("a.partsbnd.dcx"), b"x").unwrap();
        let root = normalize_staging_root(staging.path(), &EldenRingPlugin).unwrap();
        assert_eq!(root, wrapper);

        let parts_only = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(parts_only.path().join("parts")).unwrap();
        let kept = normalize_staging_root(parts_only.path(), &EldenRingPlugin).unwrap();
        assert_eq!(kept, parts_only.path());
    }

    #[test]
    fn title_matching_does_not_collide() {
        assert_eq!(
            match_plugin("ELDEN RING", None).unwrap().id,
            "eldenring"
        );
        assert_eq!(
            match_plugin("DARK SOULS III", None).unwrap().id,
            "darksouls3"
        );
        assert_eq!(
            match_plugin("Dark Souls II Scholar of the First Sin", None)
                .unwrap()
                .id,
            "darksouls2"
        );
        assert_eq!(
            match_plugin("DARK SOULS REMASTERED", None).unwrap().id,
            "darksoulsremastered"
        );
        assert_eq!(
            match_plugin("Dark Souls: Prepare to Die Edition", None)
                .unwrap()
                .id,
            "darksouls"
        );
    }
}
