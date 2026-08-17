//! Mount & Blade II: Bannerlord and Mount & Blade: Warband (Modules/).

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

pub const MODULES_ROOT: &[&str] = &["Modules"];

fn resolve_modules_deploy(install_path: &Path, relative: &Path) -> PathBuf {
    let normalized = normalize_relative(relative);
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect();
    if originals.is_empty() {
        return install_path.join("Modules");
    }
    let first = originals[0].to_string_lossy();
    if first.eq_ignore_ascii_case("Modules") {
        let mut out = install_path.join("Modules");
        for orig in originals.into_iter().skip(1) {
            out.push(orig);
        }
        return out;
    }
    install_path.join("Modules").join(&normalized)
}

fn looks_like_bannerlord_module(content_root: &Path) -> bool {
    content_root.join("SubModule.xml").is_file()
}

fn blse_present(install_path: &Path) -> bool {
    let client = install_path.join("bin").join("Win64_Shipping_Client");
    for dir in [install_path, client.as_path()] {
        if dir.join("Bannerlord.BLSE.Shared.dll").is_file()
            || dir.join("Bannerlord.BLSE.Standalone.exe").is_file()
            || dir.join("Bannerlord.BLSE.LauncherEx.exe").is_file()
        {
            return true;
        }
    }
    if let Ok(entries) = std::fs::read_dir(install_path) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.starts_with("bannerlord.blse") {
                return true;
            }
        }
    }
    false
}

macro_rules! modules_plugin {
    ($struct:ident, $id:expr, $display:expr, $domain:expr, $matches:expr, $wrap:expr, $preflight:expr) => {
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
                $wrap
            }

            fn preserve_staging_root_names(&self) -> &[&str] {
                MODULES_ROOT
            }

            fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
                looks_like_bannerlord_module(content_root)
            }

            fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
                content_folder_wrap_name(content_root, staged_name)
            }

            fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
                $preflight(install_path)
            }

            fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
                Ok(resolve_modules_deploy(install_path, relative))
            }
        }
    };
}

fn bannerlord_preflight(install_path: &Path) -> Vec<String> {
    if blse_present(install_path) {
        Vec::new()
    } else {
        vec![
            "Bannerlord Software Extender (BLSE) was not found. Many mods will not load until BLSE is installed."
                .into(),
        ]
    }
}

fn warband_preflight(_install_path: &Path) -> Vec<String> {
    Vec::new()
}

modules_plugin!(
    MountAndBlade2BannerlordPlugin,
    "mountandblade2bannerlord",
    "Mount & Blade II: Bannerlord",
    "mountandblade2bannerlord",
    &[
        "bannerlord",
        "mount & blade ii",
        "mount and blade ii",
        "mountandblade2bannerlord",
    ],
    false,
    bannerlord_preflight
);

modules_plugin!(
    MountAndBladeWarbandPlugin,
    "mbwarband",
    "Mount & Blade: Warband",
    "mbwarband",
    &[
        "warband",
        "mount & blade: warband",
        "mount and blade warband",
        "mbwarband",
    ],
    true,
    warband_preflight
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::match_plugin;

    #[test]
    fn bannerlord_wraps_on_submodule() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("SubModule.xml"), "<xml/>").unwrap();
        assert!(MountAndBlade2BannerlordPlugin.should_wrap_as_mod_folder(dir.path()));
        let dest = MountAndBlade2BannerlordPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("MyMod/SubModule.xml"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Modules/MyMod/SubModule.xml"));
    }

    #[test]
    fn warband_prefers_modules_folder() {
        assert!(MountAndBladeWarbandPlugin.prefers_mod_folder());
        let dest = MountAndBladeWarbandPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("Native/module.ini"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Modules/Native/module.ini"));
    }

    #[test]
    fn match_bannerlord_not_warband() {
        assert_eq!(
            match_plugin("Mount & Blade II: Bannerlord", None)
                .unwrap()
                .id,
            "mountandblade2bannerlord"
        );
        assert_eq!(
            match_plugin("Mount & Blade: Warband", None).unwrap().id,
            "mbwarband"
        );
    }

    #[test]
    fn warns_without_blse() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            MountAndBlade2BannerlordPlugin
                .preflight_warnings(empty.path())
                .len(),
            1
        );
        std::fs::write(empty.path().join("Bannerlord.BLSE.Shared.dll"), b"x").unwrap();
        assert!(MountAndBlade2BannerlordPlugin
            .preflight_warnings(empty.path())
            .is_empty());
    }
}
