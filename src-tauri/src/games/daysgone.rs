use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{GamePlugin, GamePluginInfo};

/// Steam AppID used for Proton compatdata path resolution.
const STEAM_APP_ID: &str = "1259420";

pub const DAYS_GONE_ROOT_DIRS: &[&str] = &["Paks", "BendGame"];

pub struct DaysGonePlugin;

impl GamePlugin for DaysGonePlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "daysgone",
            display_name: "Days Gone",
            nexus_domain: "daysgone",
            match_names: &["days gone", "daysgone"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        DAYS_GONE_ROOT_DIRS
    }

    fn prepare_deploy(&self, install_path: &Path) -> Result<Vec<String>> {
        prepare_modding(install_path)
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        let root = resolve_paks_root(install_path);
        Ok(root.join(relative))
    }
}

fn resolve_paks_root(install_path: &Path) -> PathBuf {
    let mut candidates: Vec<PathBuf> = Vec::new();

    if let Some(native) = native_paks_dir() {
        candidates.push(native);
    }
    if let Some(proton) = proton_paks_dir(install_path) {
        candidates.push(proton);
    }
    candidates.push(
        install_path
            .join("BendGame")
            .join("Saved")
            .join("Paks"),
    );

    candidates
        .into_iter()
        .find(|p| paks_candidate_usable(p))
        .unwrap_or_else(|| {
            install_path
                .join("BendGame")
                .join("Saved")
                .join("Paks")
        })
}

fn paks_candidate_usable(paks: &Path) -> bool {
    if paks.is_dir() {
        return true;
    }
    // Prefer a path whose Saved / BendGame / Local parent already exists so we can create Paks.
    paks.ancestors()
        .skip(1)
        .take(3)
        .any(|ancestor| ancestor.exists())
}

/// Native Windows LocalAppData Paks folder.
fn native_paks_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
        Some(
            local
                .join("BendGame")
                .join("Saved")
                .join("Paks"),
        )
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Derive Proton AppData Paks from a Steam install under steamapps/common.
fn proton_paks_dir(install_path: &Path) -> Option<PathBuf> {
    let steamapps = find_steamapps_dir(install_path)?;
    let paks = steamapps
        .join("compatdata")
        .join(STEAM_APP_ID)
        .join("pfx")
        .join("drive_c")
        .join("users")
        .join("steamuser")
        .join("AppData")
        .join("Local")
        .join("BendGame")
        .join("Saved")
        .join("Paks");
    Some(paks)
}

fn find_steamapps_dir(install_path: &Path) -> Option<PathBuf> {
    for ancestor in install_path.ancestors() {
        if ancestor
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("steamapps"))
        {
            return Some(ancestor.to_path_buf());
        }
        // install is typically .../steamapps/common/Days Gone
        if ancestor
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("common"))
        {
            if let Some(parent) = ancestor.parent() {
                if parent
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("steamapps"))
                {
                    return Some(parent.to_path_buf());
                }
            }
        }
    }
    None
}

fn sfpaks_dir(install_path: &Path) -> PathBuf {
    install_path
        .join("BendGame")
        .join("Content")
        .join("sfpaks")
}

fn sfpakss_dir(install_path: &Path) -> PathBuf {
    install_path
        .join("BendGame")
        .join("Content")
        .join("sfpakss")
}

/// One-time remastered mod enable: rename startuppackages.pak when present.
pub fn prepare_modding(install_path: &Path) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let sfpaks = sfpaks_dir(install_path);
    let enabled_name = "startuppackages_modsenabled.pak";
    let original = sfpaks.join("startuppackages.pak");
    let enabled = sfpaks.join(enabled_name);

    if enabled.is_file() {
        return Ok(warnings);
    }

    if original.is_file() {
        std::fs::rename(&original, &enabled)?;
        warnings.push(format!(
            "Enabled Days Gone .pak mods by renaming {} to {}.",
            original.display(),
            enabled.display()
        ));
        return Ok(warnings);
    }

    // Older community method: rename entire sfpaks folder.
    if !sfpaks.exists() && sfpakss_dir(install_path).is_dir() {
        return Ok(warnings);
    }

    warnings.push(
        "Days Gone may not load .pak mods yet. If mods do not appear in-game, rename \
         BendGame/Content/sfpaks/startuppackages.pak to startuppackages_modsenabled.pak \
         (or rename the sfpaks folder)."
            .into(),
    );
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_proton_paks_from_steam_install() {
        let tmp = tempfile::tempdir().unwrap();
        let steamapps = tmp.path().join("steamapps");
        let install = steamapps.join("common").join("Days Gone");
        let proton_saved = steamapps
            .join("compatdata")
            .join(STEAM_APP_ID)
            .join("pfx")
            .join("drive_c")
            .join("users")
            .join("steamuser")
            .join("AppData")
            .join("Local")
            .join("BendGame")
            .join("Saved");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(&proton_saved).unwrap();

        let dest = DaysGonePlugin
            .resolve_deploy_root(&install, Path::new("500-Test_P.pak"))
            .unwrap();
        assert_eq!(
            dest,
            proton_saved.join("Paks").join("500-Test_P.pak")
        );
    }

    #[test]
    fn falls_back_to_install_relative_paks() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("Days Gone");
        std::fs::create_dir_all(install.join("BendGame").join("Saved")).unwrap();

        let dest = DaysGonePlugin
            .resolve_deploy_root(&install, Path::new("mod.pak"))
            .unwrap();
        assert_eq!(
            dest,
            install
                .join("BendGame")
                .join("Saved")
                .join("Paks")
                .join("mod.pak")
        );
    }

    #[test]
    fn prepare_deploy_renames_startuppackages_once() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("Days Gone");
        let sfpaks = install.join("BendGame").join("Content").join("sfpaks");
        std::fs::create_dir_all(&sfpaks).unwrap();
        let original = sfpaks.join("startuppackages.pak");
        std::fs::write(&original, b"pak").unwrap();

        let plugin = DaysGonePlugin;
        let first = plugin.prepare_deploy(&install).unwrap();
        assert!(first.iter().any(|w| w.contains("Enabled Days Gone")));
        assert!(!original.exists());
        assert!(sfpaks.join("startuppackages_modsenabled.pak").is_file());

        let second = plugin.prepare_deploy(&install).unwrap();
        assert!(second.is_empty());
    }

    #[test]
    fn prepare_deploy_accepts_legacy_sfpakss() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("Days Gone");
        let sfpakss = install.join("BendGame").join("Content").join("sfpakss");
        std::fs::create_dir_all(&sfpakss).unwrap();

        let warnings = DaysGonePlugin.prepare_deploy(&install).unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn prepare_deploy_warns_when_unconfigured() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("Days Gone");
        std::fs::create_dir_all(&install).unwrap();

        let warnings = DaysGonePlugin.prepare_deploy(&install).unwrap();
        assert!(warnings.iter().any(|w| w.contains("may not load")));
    }
}
