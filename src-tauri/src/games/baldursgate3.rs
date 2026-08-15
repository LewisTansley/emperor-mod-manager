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
        // Prefer Public/ModLibrary/Mods when present (native), else Mods under install.
        let candidates = [
            install_path.join("Public").join("ModLibrary").join("Mods"),
            install_path.join("Mods"),
            // Proton / AppData-style path inside prefix is handled by install_path pointing at game root;
            // users managing BG3 often point at the game directory; drop packs into Mods.
            dirs_fallback(install_path),
        ];
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
