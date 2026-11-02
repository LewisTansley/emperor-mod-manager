//! SCS truck sims (ETS2, ATS): user-data `mod` folder with `.scs` archives.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;
use walkdir::WalkDir;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

pub const SCS_MOD_ROOT_DIRS: &[&str] = &["mod"];

struct ScsTitle {
    steam_app_id: &'static str,
    user_folder: &'static str,
    win_exe: &'static str,
}

const ETS2: ScsTitle = ScsTitle {
    steam_app_id: "227300",
    user_folder: "Euro Truck Simulator 2",
    win_exe: "eurotrucks2.exe",
};

const ATS: ScsTitle = ScsTitle {
    steam_app_id: "270880",
    user_folder: "American Truck Simulator",
    win_exe: "amtrucks.exe",
};

fn native_home() -> Option<PathBuf> {
    #[cfg(windows)]
    let from_env = std::env::var_os("USERPROFILE").map(PathBuf::from);
    #[cfg(not(windows))]
    let from_env = std::env::var_os("HOME").map(PathBuf::from);
    from_env.or_else(|| directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()))
}

fn is_windows_build(install_path: &Path, win_exe: &str) -> bool {
    if install_path.join(win_exe).is_file() {
        return true;
    }
    for entry in WalkDir::new(install_path)
        .max_depth(5)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(win_exe)
        {
            return true;
        }
    }
    false
}

fn windows_build_mod_dir(install_path: &Path, title: &ScsTitle) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        return super::user_data::documents_dir(install_path, title.steam_app_id)
            .map(|docs| docs.join(title.user_folder).join("mod"));
    }
    #[cfg(not(windows))]
    {
        super::user_data::proton_user_dir(install_path, title.steam_app_id)
            .or_else(|| super::user_data::wine_user_dir(install_path))
            .map(|user| {
                user.join("Documents")
                    .join(title.user_folder)
                    .join("mod")
            })
    }
}

fn native_linux_mod_dir(title: &ScsTitle) -> Option<PathBuf> {
    native_home().map(|home| {
        home.join(".local")
            .join("share")
            .join(title.user_folder)
            .join("mod")
    })
}

fn mod_dir(install_path: &Path, title: &ScsTitle) -> Option<PathBuf> {
    if is_windows_build(install_path, title.win_exe) {
        windows_build_mod_dir(install_path, title)
    } else {
        native_linux_mod_dir(title)
    }
}

fn resolve_scs_deploy(install_path: &Path, relative: &Path, title: &ScsTitle) -> PathBuf {
    let root = mod_dir(install_path, title).unwrap_or_else(|| install_path.join("mod"));
    let normalized = normalize_relative(relative);
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect();
    if originals.is_empty() {
        return root;
    }
    let first = originals[0].to_string_lossy();
    if first.eq_ignore_ascii_case("mod") {
        let mut out = root;
        for orig in originals.into_iter().skip(1) {
            out.push(orig);
        }
        return out;
    }
    if first.eq_ignore_ascii_case(title.user_folder) {
        let mut rest = originals.into_iter();
        rest.next();
        if rest
            .next()
            .is_some_and(|p| p.to_string_lossy().eq_ignore_ascii_case("mod"))
        {
            let mut out = root;
            for orig in rest {
                out.push(orig);
            }
            return out;
        }
    }
    root.join(&normalized)
}

fn looks_like_unpacked_scs_mod(content_root: &Path) -> bool {
    if !content_root.is_dir() {
        return false;
    }
    if root_has_scs(content_root) {
        return false;
    }
    content_root.read_dir().ok().is_some_and(|entries| {
        entries.flatten().any(|entry| {
            let path = entry.path();
            path.is_dir() || path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("scs"))
        })
    })
}

fn root_has_scs(dir: &Path) -> bool {
    let Ok(entries) = dir.read_dir() else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.path().is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("scs"))
    })
}

macro_rules! scs_truck_plugin {
    ($struct:ident, $id:expr, $display:expr, $domain:expr, $matches:expr, $title:expr) => {
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

            fn preserve_staging_root_names(&self) -> &[&str] {
                SCS_MOD_ROOT_DIRS
            }

            fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
                looks_like_unpacked_scs_mod(content_root)
            }

            fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
                content_folder_wrap_name(content_root, staged_name)
            }

            fn should_deploy_file(&self, relative: &Path) -> bool {
                relative
                    .extension()
                    .is_none_or(|ext| !ext.eq_ignore_ascii_case("exe"))
            }

            fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
                let mut out = vec![
                    "Enable deployed mods in the in-game Mod Manager — Emperor load order does not set SCS mod priority."
                        .into(),
                ];
                if mod_dir(install_path, &$title).is_none() {
                    out.push(format!(
                        "Could not resolve the {} user mod folder. Deploy may fall back to the install mod/ folder until the game has been launched once.",
                        $display
                    ));
                }
                out
            }

            fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
                Ok(resolve_scs_deploy(install_path, relative, &$title))
            }
        }
    };
}

scs_truck_plugin!(
    EuroTruckSimulator2Plugin,
    "eurotrucksimulator2",
    "Euro Truck Simulator 2",
    "eurotrucksimulator2",
    &[
        "euro truck simulator 2",
        "eurotrucksimulator2",
        "ets2",
    ],
    ETS2
);

scs_truck_plugin!(
    AmericanTruckSimulatorPlugin,
    "americantrucksimulator",
    "American Truck Simulator",
    "americantrucksimulator",
    &[
        "american truck simulator",
        "americantrucksimulator",
        "ats",
    ],
    ATS
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::match_plugin;

    fn protonless_win_install() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("game");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("eurotrucks2.exe"), b"x").unwrap();
        tmp
    }

    #[test]
    fn flat_scs_goes_to_mod() {
        let tmp = protonless_win_install();
        let install = tmp.path().join("game");
        let dest = EuroTruckSimulator2Plugin
            .resolve_deploy_root(&install, Path::new("Cool.scs"))
            .unwrap();
        assert_eq!(dest, install.join("mod").join("Cool.scs"));
    }

    #[test]
    fn preserves_mod_prefix() {
        let tmp = protonless_win_install();
        let install = tmp.path().join("game");
        let dest = EuroTruckSimulator2Plugin
            .resolve_deploy_root(&install, Path::new("mod/Cool.scs"))
            .unwrap();
        assert_eq!(dest, install.join("mod").join("Cool.scs"));
    }

    #[test]
    fn strips_game_folder_prefix() {
        let tmp = protonless_win_install();
        let install = tmp.path().join("game");
        let dest = EuroTruckSimulator2Plugin
            .resolve_deploy_root(
                &install,
                Path::new("Euro Truck Simulator 2/mod/Cool.scs"),
            )
            .unwrap();
        assert_eq!(dest, install.join("mod").join("Cool.scs"));
    }

    #[test]
    fn proton_compatdata_ets2() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp
            .path()
            .join("steamapps")
            .join("common")
            .join("Euro Truck Simulator 2");
        std::fs::create_dir_all(install.join("bin").join("win_x64")).unwrap();
        std::fs::write(install.join("bin/win_x64/eurotrucks2.exe"), b"x").unwrap();
        let docs = tmp
            .path()
            .join("steamapps")
            .join("compatdata")
            .join("227300")
            .join("pfx")
            .join("drive_c")
            .join("users")
            .join("steamuser")
            .join("Documents")
            .join("Euro Truck Simulator 2")
            .join("mod");
        std::fs::create_dir_all(&docs).unwrap();
        let dest = EuroTruckSimulator2Plugin
            .resolve_deploy_root(&install, Path::new("Cool.scs"))
            .unwrap();
        assert_eq!(dest, docs.join("Cool.scs"));
    }

    #[test]
    fn proton_compatdata_ats() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp
            .path()
            .join("steamapps")
            .join("common")
            .join("American Truck Simulator");
        std::fs::create_dir_all(install.join("bin").join("win_x64")).unwrap();
        std::fs::write(install.join("bin/win_x64/amtrucks.exe"), b"x").unwrap();
        let docs = tmp
            .path()
            .join("steamapps")
            .join("compatdata")
            .join("270880")
            .join("pfx")
            .join("drive_c")
            .join("users")
            .join("steamuser")
            .join("Documents")
            .join("American Truck Simulator")
            .join("mod");
        std::fs::create_dir_all(&docs).unwrap();
        let dest = AmericanTruckSimulatorPlugin
            .resolve_deploy_root(&install, Path::new("Cool.scs"))
            .unwrap();
        assert_eq!(dest, docs.join("Cool.scs"));
    }

    #[test]
    fn native_linux_mod_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("Euro Truck Simulator 2");
        std::fs::create_dir_all(install.join("bin").join("linux_x64")).unwrap();
        std::fs::write(install.join("bin/linux_x64/eurotrucks2"), b"x").unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", &home);
        let dest = EuroTruckSimulator2Plugin
            .resolve_deploy_root(&install, Path::new("Cool.scs"))
            .unwrap();
        if let Some(prev) = prev {
            std::env::set_var("HOME", prev);
        } else {
            std::env::remove_var("HOME");
        }
        assert_eq!(
            dest,
            home.join(".local")
                .join("share")
                .join("Euro Truck Simulator 2")
                .join("mod")
                .join("Cool.scs")
        );
    }

    #[test]
    fn skips_exe_deploy() {
        assert!(!EuroTruckSimulator2Plugin.should_deploy_file(Path::new("installer.exe")));
        assert!(EuroTruckSimulator2Plugin.should_deploy_file(Path::new("Cool.scs")));
    }

    #[test]
    fn match_ets2_not_ets1() {
        assert_eq!(
            match_plugin("Euro Truck Simulator 2", None).unwrap().id,
            "eurotrucksimulator2"
        );
        assert!(match_plugin("Euro Truck Simulator", None).is_none());
        assert_eq!(
            match_plugin("American Truck Simulator", None).unwrap().id,
            "americantrucksimulator"
        );
    }
}
