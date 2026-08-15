use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{normalize_relative, GamePlugin, GamePluginInfo};

/// Top-level folder that belongs at the S.T.A.L.K.E.R. 2 install root.
pub const STALKER2_ROOT_DIRS: &[&str] = &["Stalker2"];

pub struct Stalker2HeartOfChornobylPlugin;

impl GamePlugin for Stalker2HeartOfChornobylPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "stalker2heartofchornobyl",
            display_name: "S.T.A.L.K.E.R. 2: Heart of Chornobyl",
            nexus_domain: "stalker2heartofchornobyl",
            match_names: &[
                "s.t.a.l.k.e.r. 2",
                "stalker 2",
                "stalker2",
                "heart of chornobyl",
                "heart of chernobyl",
            ],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        STALKER2_ROOT_DIRS
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        let normalized = normalize_relative(relative);
        let components = path_components_lower(&normalized);
        if components.is_empty() {
            return Ok(install_path.to_path_buf());
        }

        let originals = path_components_original(&normalized);
        let first = components[0].as_str();

        // Already install-relative under Stalker2/...
        if first.eq_ignore_ascii_case("stalker2") {
            return Ok(join_with_originals(install_path, &originals));
        }

        // LogicMods (blueprint paks) — strip a leading LogicMods/ then nest under Paks/LogicMods.
        if let Some(idx) = components.iter().position(|c| c == "logicmods") {
            let rest: PathBuf = originals.iter().skip(idx + 1).collect();
            let mut out = paks_dir(install_path).join("LogicMods");
            if !rest.as_os_str().is_empty() {
                out.push(rest);
            }
            return Ok(out);
        }

        // Flat UE5 pak / IoStore files.
        let is_single = components.len() == 1;
        let leaf = components.last().map(|s| s.as_str()).unwrap_or("");
        if is_single && is_pak_like(leaf) {
            return Ok(mods_dir(install_path).join(originals[0].as_str()));
        }

        // Paths already under Content/Paks/~mods or ~mods/...
        if components.iter().any(|c| c == "~mods") {
            if let Some(idx) = components.iter().position(|c| c == "~mods") {
                let rest: PathBuf = originals.iter().skip(idx + 1).collect();
                let mut out = mods_dir(install_path);
                if !rest.as_os_str().is_empty() {
                    out.push(rest);
                }
                return Ok(out);
            }
        }

        // UE4SS / binaries: ue4ss/, dwmapi.dll, or Binaries/...
        if first == "ue4ss"
            || leaf == "dwmapi.dll"
            || first == "binaries"
            || components.iter().any(|c| c == "binaries")
        {
            return Ok(binaries_target(install_path, &components, &originals));
        }

        // Default: majority of Nexus pak mods land in ~mods.
        Ok(mods_dir(install_path).join(&normalized))
    }
}

fn paks_dir(install_path: &Path) -> PathBuf {
    install_path
        .join("Stalker2")
        .join("Content")
        .join("Paks")
}

fn mods_dir(install_path: &Path) -> PathBuf {
    paks_dir(install_path).join("~mods")
}

fn win64_dir(install_path: &Path) -> PathBuf {
    install_path
        .join("Stalker2")
        .join("Binaries")
        .join("Win64")
}

fn is_pak_like(name: &str) -> bool {
    name.ends_with(".pak") || name.ends_with(".ucas") || name.ends_with(".utoc")
}

fn binaries_target(
    install_path: &Path,
    components: &[String],
    originals: &[String],
) -> PathBuf {
    let win64 = win64_dir(install_path);

    // Strip a leading Stalker2/Binaries/Win64 (or Binaries/Win64) prefix if present.
    let mut start = 0;
    if components.first().map(|s| s.as_str()) == Some("stalker2") {
        start = 1;
    }
    if components.get(start).map(|s| s.as_str()) == Some("binaries") {
        start += 1;
        if components.get(start).map(|s| s.as_str()) == Some("win64") {
            start += 1;
        }
    }

    let rest: PathBuf = originals.iter().skip(start).collect();
    if rest.as_os_str().is_empty() {
        win64
    } else {
        win64.join(rest)
    }
}

fn path_components_lower(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect()
}

fn path_components_original(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn join_with_originals(install_path: &Path, originals: &[String]) -> PathBuf {
    let mut out = install_path.to_path_buf();
    for part in originals {
        out.push(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::GamePlugin;

    fn plugin() -> Stalker2HeartOfChornobylPlugin {
        Stalker2HeartOfChornobylPlugin
    }

    #[test]
    fn preserves_stalker2_relative_trees() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(
                install,
                Path::new("Stalker2/Content/Paks/~mods/Foo.pak")
            )
            .unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/~mods/Foo.pak")
        );
    }

    #[test]
    fn flat_pak_files_go_to_tildemods() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("Foo.pak")).unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/~mods/Foo.pak")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("Foo.ucas")).unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/~mods/Foo.ucas")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("Foo.utoc")).unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/~mods/Foo.utoc")
        );
    }

    #[test]
    fn logicmods_nested_under_paks() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("LogicMods/MyBp.pak"))
                .unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/LogicMods/MyBp.pak")
        );
        assert_eq!(
            p.resolve_deploy_root(
                install,
                Path::new("SomePack/LogicMods/MyBp.pak")
            )
            .unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/LogicMods/MyBp.pak")
        );
    }

    #[test]
    fn ue4ss_and_binaries_go_to_win64() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("ue4ss/Mods/Foo/scripts/main.lua"))
                .unwrap(),
            PathBuf::from("/game/Stalker2/Binaries/Win64/ue4ss/Mods/Foo/scripts/main.lua")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("dwmapi.dll"))
                .unwrap(),
            PathBuf::from("/game/Stalker2/Binaries/Win64/dwmapi.dll")
        );
        assert_eq!(
            p.resolve_deploy_root(
                install,
                Path::new("Binaries/Win64/ue4ss/UE4SS-settings.ini")
            )
            .unwrap(),
            PathBuf::from("/game/Stalker2/Binaries/Win64/ue4ss/UE4SS-settings.ini")
        );
    }

    #[test]
    fn tilde_mods_prefix_normalized() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("~mods/Bar.pak"))
                .unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/~mods/Bar.pak")
        );
    }

    #[test]
    fn default_lands_under_tildemods() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("MyMod/readme.txt"))
                .unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/~mods/MyMod/readme.txt")
        );
    }

    #[test]
    fn normalizes_windows_separators() {
        let install = Path::new("/game");
        let p = plugin();
        let weird = PathBuf::from(r"LogicMods\Bp.pak");
        assert_eq!(
            p.resolve_deploy_root(install, &weird).unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/LogicMods/Bp.pak")
        );
    }

    #[test]
    fn match_names_cover_common_titles() {
        let info = plugin().info();
        assert_eq!(info.nexus_domain, "stalker2heartofchornobyl");
        assert!(info.match_names.iter().any(|n| *n == "stalker 2"));
        assert!(info.match_names.iter().any(|n| *n == "s.t.a.l.k.e.r. 2"));
    }
}
