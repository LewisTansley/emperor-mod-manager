//! Shared deploy logic for Capcom RE Engine Resident Evil titles.
//!
//! Mods are linked into the game install root for use with REFramework
//! (loose file loader + `pak_mods`). This does not perform Fluffy-style
//! PAK invalidation of stock `re_chunk_*.pak` archives.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{normalize_relative, GamePlugin, GamePluginInfo};

/// Top-level folders that belong at the RE Engine install root.
pub const RE_ENGINE_ROOT_DIRS: &[&str] = &["natives", "reframework", "pak_mods"];

/// REF injector / companion DLLs that deploy at the game install root.
const ROOT_DLLS: &[&str] = &["dinput8.dll", "openvr_api.dll", "openxr_loader.dll"];

/// Heuristic: install looks like an RE Engine remake (not classic MT Framework).
pub fn is_re_engine_remake_install(install_path: &Path) -> bool {
    if !install_path.is_dir() {
        return false;
    }
    if install_path.join("re4.exe").is_file()
        || install_path.join("re2.exe").is_file()
        || install_path.join("re3.exe").is_file()
        || install_path.join("re7.exe").is_file()
        || install_path.join("re8.exe").is_file()
        || install_path.join("re9.exe").is_file()
    {
        return true;
    }
    if install_path.join("re_chunk_000.pak").is_file() {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(install_path) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let lower = name.to_string_lossy().to_lowercase();
        if lower.starts_with("re_chunk_") && lower.ends_with(".pak") {
            return true;
        }
    }
    false
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
        if i == 0 && RE_ENGINE_ROOT_DIRS.iter().any(|r| *r == lower.as_str()) {
            out.push(lower.as_str());
        } else if let Some(orig) = originals.get(i) {
            out.push(orig);
        } else {
            out.push(lower.as_str());
        }
    }
    out
}

fn is_pak_leaf(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".pak") || lower.contains(".pak.")
}

fn is_natives_data_prefix(first: &str) -> bool {
    matches!(
        first,
        "stm" | "x64" | "wwise" | "streaming" | "helper" | "textures"
    )
}

fn resolve_re_engine_deploy(install_path: &Path, relative: &Path) -> PathBuf {
    let normalized = normalize_relative(relative);
    let components = path_components_lower(&normalized);
    if components.is_empty() {
        return install_path.to_path_buf();
    }

    let first = components[0].as_str();

    if RE_ENGINE_ROOT_DIRS.iter().any(|r| *r == first) {
        return join_canonical(install_path, &normalized, &components);
    }

    let is_single = components.len() == 1;
    let leaf = components.last().map(|s| s.as_str()).unwrap_or("");

    if is_single && ROOT_DLLS.iter().any(|d| *d == leaf) {
        return install_path.join(normalized.file_name().unwrap_or(normalized.as_os_str()));
    }

    if is_single && is_pak_leaf(leaf) {
        return install_path
            .join("pak_mods")
            .join(normalized.file_name().unwrap_or(normalized.as_os_str()));
    }

    // Loose natives trees sometimes omit the `natives/` prefix.
    if is_natives_data_prefix(first) {
        let mut out = install_path.join("natives");
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
        return out;
    }

    install_path.join(&normalized)
}

macro_rules! re_engine_plugin {
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
                false
            }

            fn preserve_staging_root_names(&self) -> &[&str] {
                RE_ENGINE_ROOT_DIRS
            }

            fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
                Ok(resolve_re_engine_deploy(install_path, relative))
            }
        }
    };
}

re_engine_plugin!(
    MonsterHunterWildsPlugin,
    "monsterhunterwilds",
    "Monster Hunter Wilds",
    "monsterhunterwilds",
    &[
        "monster hunter wilds",
        "monsterhunterwilds",
        "mh wilds",
    ]
);

re_engine_plugin!(
    MonsterHunterRisePlugin,
    "monsterhunterrise",
    "Monster Hunter Rise",
    "monsterhunterrise",
    &[
        "monster hunter rise",
        "monsterhunterrise",
        "mh rise",
    ]
);

re_engine_plugin!(
    ResidentEvilRequiemPlugin,
    "residentevilrequiem",
    "Resident Evil Requiem",
    "residentevilrequiem",
    &[
        "resident evil requiem",
        "residentevilrequiem",
        "biohazard requiem",
        "resident evil 9",
        "residentevil9",
    ]
);

re_engine_plugin!(
    ResidentEvilVillagePlugin,
    "residentevilvillage",
    "Resident Evil Village",
    "residentevilvillage",
    &[
        "resident evil village",
        "residentevilvillage",
        "biohazard village",
        "resident evil 8",
        "residentevil8",
        "biohazard 8",
    ]
);

re_engine_plugin!(
    ResidentEvil42023Plugin,
    "residentevil42023",
    "Resident Evil 4 (2023)",
    "residentevil42023",
    &[
        "biohazard re4",
        "residentevil42023",
        "resident evil 4 (2023)",
        "resident evil 4 2023",
        // Ambiguous bare "resident evil 4" is handled in match_plugin via install path.
    ]
);

re_engine_plugin!(
    ResidentEvil32020Plugin,
    "residentevil32020",
    "Resident Evil 3 (2020)",
    "residentevil32020",
    &[
        "resident evil 3",
        "residentevil32020",
        "residentevil3",
        "biohazard re3",
        "biohazard 3",
    ]
);

re_engine_plugin!(
    ResidentEvil22019Plugin,
    "residentevil22019",
    "Resident Evil 2 (2019)",
    "residentevil22019",
    &[
        "resident evil 2",
        "residentevil22019",
        "residentevil2",
        "biohazard re2",
        "biohazard 2",
    ]
);

re_engine_plugin!(
    ResidentEvil7Plugin,
    "residentevil7",
    "Resident Evil 7",
    "residentevil7",
    &[
        "resident evil 7",
        "residentevil7",
        "biohazard 7",
        "resident evil vii",
        "biohazard vii",
    ]
);

/// True when the title is a bare RE4 remake candidate (not already matched by BIOHAZARD RE4).
pub fn is_ambiguous_re4_title(title_lower: &str) -> bool {
    let t = title_lower.to_lowercase();
    if t.contains("biohazard re4") || t.contains("2023") || t.contains("remake") {
        return false;
    }
    // Classic / other titles we must not claim without a remake install heuristic.
    if t.contains("revelations") || t.contains("outbreak") {
        return false;
    }
    t.contains("resident evil 4") || t.contains("residentevil4")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::{normalize_staging_root, GamePlugin};

    fn plugin() -> ResidentEvil7Plugin {
        ResidentEvil7Plugin
    }

    #[test]
    fn preserves_natives_reframework_pak_mods() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("natives/STM/a.tex"))
                .unwrap(),
            PathBuf::from("/game/natives/STM/a.tex")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("reframework/autorun/mod.lua"))
                .unwrap(),
            PathBuf::from("/game/reframework/autorun/mod.lua")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("pak_mods/mod.pak"))
                .unwrap(),
            PathBuf::from("/game/pak_mods/mod.pak")
        );
    }

    #[test]
    fn flat_pak_goes_to_pak_mods() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("CoolMod.pak"))
                .unwrap(),
            PathBuf::from("/game/pak_mods/CoolMod.pak")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("CoolMod.pak.001"))
                .unwrap(),
            PathBuf::from("/game/pak_mods/CoolMod.pak.001")
        );
    }

    #[test]
    fn dinput8_deploys_to_install_root() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("dinput8.dll"))
                .unwrap(),
            PathBuf::from("/game/dinput8.dll")
        );
    }

    #[test]
    fn case_insensitive_root_prefixes() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("NATIVES/STM/x.bin"))
                .unwrap(),
            PathBuf::from("/game/natives/STM/x.bin")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("REFramework/plugins/a.dll"))
                .unwrap(),
            PathBuf::from("/game/reframework/plugins/a.dll")
        );
    }

    #[test]
    fn stm_prefix_wraps_under_natives() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("STM/Gui/ui.tex"))
                .unwrap(),
            PathBuf::from("/game/natives/STM/Gui/ui.tex")
        );
    }

    #[test]
    fn default_lands_at_install_root() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("readme.txt"))
                .unwrap(),
            PathBuf::from("/game/readme.txt")
        );
    }

    #[test]
    fn peels_wrapper_but_keeps_re_engine_roots() {
        let staging = tempfile::tempdir().unwrap();
        let wrapper = staging.path().join("Some Mod Pack");
        std::fs::create_dir_all(wrapper.join("natives").join("STM")).unwrap();
        std::fs::write(wrapper.join("natives").join("STM").join("a.bin"), b"x").unwrap();

        let root = normalize_staging_root(staging.path(), &ResidentEvil7Plugin).unwrap();
        assert_eq!(root, wrapper);

        let natives_only = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(natives_only.path().join("natives").join("STM")).unwrap();
        let kept = normalize_staging_root(natives_only.path(), &ResidentEvil7Plugin).unwrap();
        assert_eq!(kept, natives_only.path());
    }

    #[test]
    fn remake_install_heuristic() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_re_engine_remake_install(dir.path()));
        std::fs::write(dir.path().join("re4.exe"), b"x").unwrap();
        assert!(is_re_engine_remake_install(dir.path()));

        let pak_dir = tempfile::tempdir().unwrap();
        std::fs::write(pak_dir.path().join("re_chunk_000.pak"), b"x").unwrap();
        assert!(is_re_engine_remake_install(pak_dir.path()));
    }

    #[test]
    fn ambiguous_re4_title_detection() {
        assert!(is_ambiguous_re4_title("resident evil 4"));
        assert!(is_ambiguous_re4_title("Resident Evil 4"));
        assert!(!is_ambiguous_re4_title("resident evil 4 biohazard re4"));
        assert!(!is_ambiguous_re4_title("biohazard re4"));
        assert!(!is_ambiguous_re4_title("resident evil revelations 2"));
    }

    #[test]
    fn normalizes_windows_separators() {
        let install = Path::new("/game");
        let p = plugin();
        let weird = PathBuf::from(r"natives\STM\file.bin");
        assert_eq!(
            p.resolve_deploy_root(install, &weird).unwrap(),
            PathBuf::from("/game/natives/STM/file.bin")
        );
    }
}
