use std::path::{Path, PathBuf};

use anyhow::Result;

use super::{GamePlugin, GamePluginInfo};

pub struct StardewValleyPlugin;

impl GamePlugin for StardewValleyPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "stardewvalley",
            display_name: "Stardew Valley",
            nexus_domain: "stardewvalley",
            match_names: &["stardew valley", "stardewvalley"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        true
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        // SMAPI mods live in <game>/Mods/<ModName>/...
        Ok(install_path.join("Mods").join(relative))
    }
}
