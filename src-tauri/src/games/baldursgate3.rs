use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{GamePlugin, GamePluginInfo};

pub struct BaldursGate3Plugin;

impl GamePlugin for BaldursGate3Plugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "baldursgate3",
            display_name: "Baldur's Gate 3",
            nexus_domain: "baldursgate3",
            match_names: &[
                "baldur's gate 3",
                "baldurs gate 3",
                "baldur's gate iii",
                "bg3",
            ],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        true
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        let mut candidates = vec![
            install_path.join("Public").join("ModLibrary").join("Mods"),
            install_path.join("Mods"),
            // Proton / AppData-style path inside prefix is handled by install_path pointing at game root.
            dirs_fallback(install_path),
        ];
        if let Some(appdata) = native_bg3_mods_dir() {
            candidates.insert(0, appdata);
        }
        let root = candidates
            .into_iter()
            .find(|p| p.parent().map(|p| p.exists()).unwrap_or(false))
            .unwrap_or_else(|| install_path.join("Mods"));
        Ok(root.join(relative))
    }
}

fn dirs_fallback(install_path: &Path) -> PathBuf {
    install_path.join("Data").join("Mods")
}

/// Native Windows BG3 mods folder under LocalAppData.
fn native_bg3_mods_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
        let mods = local
            .join("Larian Studios")
            .join("Baldur's Gate 3")
            .join("Mods");
        if mods.parent().map(|p| p.exists()).unwrap_or(false) {
            return Some(mods);
        }
        None
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_install_relative_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("bg3");
        let mods = install.join("Mods");
        std::fs::create_dir_all(&mods).unwrap();
        let p = BaldursGate3Plugin;
        let dest = p
            .resolve_deploy_root(&install, Path::new("MyMod/pak"))
            .unwrap();
        assert!(dest.starts_with(&mods));
    }
}
