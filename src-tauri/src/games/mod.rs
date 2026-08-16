//! Per-game deploy plugins.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;

mod baldursgate3;
mod cyberpunk2077;
mod daysgone;
mod fromsoftware;
mod re_engine;
mod stardewvalley;
mod unity_bepinex;
mod unreal_engine;
mod warhammer40kdarktide;

pub use baldursgate3::BaldursGate3Plugin;
pub use cyberpunk2077::Cyberpunk2077Plugin;
pub use daysgone::DaysGonePlugin;
pub use fromsoftware::{
    DarkSouls2Plugin, DarkSouls3Plugin, DarkSoulsPlugin, DarkSoulsRemasteredPlugin, EldenRingPlugin,
};
pub use re_engine::{
    ResidentEvil22019Plugin, ResidentEvil32020Plugin, ResidentEvil42023Plugin, ResidentEvil7Plugin,
    ResidentEvilRequiemPlugin, ResidentEvilVillagePlugin,
};
pub use stardewvalley::StardewValleyPlugin;
pub use unity_bepinex::{
    looks_like_unity_install, thunderstore_community_for_plugin, AgainstTheStormPlugin,
    AmongUsPlugin, BepInExPlugin, ContentWarningPlugin, CultOfTheLambPlugin, GtfoPlugin,
    LethalCompanyPlugin, RiskOfRain2Plugin, RoundsPlugin, TimberbornPlugin, ValheimPlugin,
};
pub use unreal_engine::{
    detect_ue_layout, layout_info, DeployContext, HogwartsLegacyPlugin, PalworldPlugin,
    Stalker2HeartOfChornobylPlugin, UeLayoutInfo, UnrealEnginePlugin,
};
pub use warhammer40kdarktide::Warhammer40kDarktidePlugin;

#[derive(Debug, Clone, Copy, Serialize)]
pub struct GamePluginInfo {
    pub id: &'static str,
    pub display_name: &'static str,
    pub nexus_domain: &'static str,
    pub match_names: &'static [&'static str],
}

pub trait GamePlugin: Send + Sync {
    fn info(&self) -> GamePluginInfo;

    /// Resolve where staged mod files should be linked inside the game install.
    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf>;

    /// Resolve with optional deploy context (UE project override, LogicMod markers).
    fn resolve_deploy_root_ctx(
        &self,
        install_path: &Path,
        relative: &Path,
        ctx: &DeployContext<'_>,
    ) -> Result<PathBuf> {
        let _ = ctx;
        self.resolve_deploy_root(install_path, relative)
    }

    /// Prefer a subdirectory name under Mods/ for folder-style games.
    fn prefers_mod_folder(&self) -> bool {
        false
    }

    /// Top-level staging folder names that must not be peeled as archive wrappers.
    fn preserve_staging_root_names(&self) -> &[&str] {
        &[]
    }

    /// When true, deploy under a sanitized mod-name folder (e.g. REDmod info.json packs).
    fn should_wrap_as_mod_folder(&self, _content_root: &Path) -> bool {
        false
    }

    /// Name of the directory used when wrapping a staged mod.
    fn wrap_mod_folder_name(&self, _content_root: &Path, staged_name: &str) -> String {
        sanitize_filename::sanitize(staged_name)
    }

    /// When true, link files at the game install root instead of via `resolve_deploy_root`
    /// (e.g. SMAPI installer packages).
    fn deploys_to_install_root(&self, _content_root: &Path) -> bool {
        false
    }

    /// Install-level warnings shown before/during deploy (e.g. missing mod loader).
    fn preflight_warnings(&self, _install_path: &Path) -> Vec<String> {
        Vec::new()
    }

    fn preflight_warnings_ctx(
        &self,
        install_path: &Path,
        ctx: &DeployContext<'_>,
    ) -> Vec<String> {
        let _ = ctx;
        self.preflight_warnings(install_path)
    }

    /// Per-staging-pack warnings (e.g. SMAPI installer deployed as a mod).
    fn staging_deploy_warnings(&self, _content_root: &Path, _mod_name: &str) -> Vec<String> {
        Vec::new()
    }

    /// Optional install prep before purge/link (rename paks, etc.). Returns warnings.
    fn prepare_deploy(&self, _install_path: &Path) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Called after files are linked. `enabled_mod_folders` are resolved wrap names
    /// for mods that should appear in a native load-order file (may be empty).
    /// Returns optional warning strings (deploy succeeds even if warnings are returned).
    fn after_deploy(
        &self,
        _install_path: &Path,
        _enabled_mod_folders: &[String],
    ) -> Result<Vec<String>> {
        Ok(vec![])
    }
}

pub fn all_plugins() -> Vec<&'static dyn GamePlugin> {
    // More specific match_names first (FromSoft + RE Engine remakes + UE + BepInEx titles).
    // Generic UnrealEnginePlugin / BepInExPlugin have empty match_names (detection via engine_hint).
    vec![
        &StardewValleyPlugin,
        &BaldursGate3Plugin,
        &Cyberpunk2077Plugin,
        &DaysGonePlugin,
        &Warhammer40kDarktidePlugin,
        &Stalker2HeartOfChornobylPlugin,
        &PalworldPlugin,
        &HogwartsLegacyPlugin,
        &UnrealEnginePlugin,
        &LethalCompanyPlugin,
        &ValheimPlugin,
        &RiskOfRain2Plugin,
        &AmongUsPlugin,
        &ContentWarningPlugin,
        &GtfoPlugin,
        &TimberbornPlugin,
        &CultOfTheLambPlugin,
        &AgainstTheStormPlugin,
        &RoundsPlugin,
        &BepInExPlugin,
        &EldenRingPlugin,
        &DarkSouls3Plugin,
        &DarkSouls2Plugin,
        &DarkSoulsRemasteredPlugin,
        &DarkSoulsPlugin,
        &ResidentEvilRequiemPlugin,
        &ResidentEvilVillagePlugin,
        &ResidentEvil42023Plugin,
        &ResidentEvil32020Plugin,
        &ResidentEvil22019Plugin,
        &ResidentEvil7Plugin,
    ]
}

pub fn plugin_by_id(id: &str) -> Option<&'static dyn GamePlugin> {
    all_plugins().into_iter().find(|p| p.info().id == id)
}

pub fn match_plugin(title: &str, install_path: Option<&str>) -> Option<GamePluginInfo> {
    let lower = title.to_lowercase();
    for plugin in all_plugins() {
        let info = plugin.info();
        if info.match_names.is_empty() {
            continue;
        }
        for name in info.match_names {
            if lower.contains(&name.to_lowercase()) {
                return Some(info);
            }
        }
    }

    // Bare "Resident Evil 4" matches the 2023 remake only when the install looks like RE Engine.
    if re_engine::is_ambiguous_re4_title(&lower) {
        if let Some(path) = install_path {
            if re_engine::is_re_engine_remake_install(Path::new(path)) {
                return Some(ResidentEvil42023Plugin.info());
            }
        }
    }

    None
}

pub fn list_plugins() -> Vec<GamePluginInfo> {
    all_plugins().into_iter().map(|p| p.info()).collect()
}

/// Turn archive-originated `\` separators into normal path components.
pub fn normalize_relative(relative: &Path) -> PathBuf {
    let raw = relative.to_string_lossy().replace('\\', "/");
    let trimmed = raw.trim_matches('/');
    if trimmed.is_empty() {
        return PathBuf::new();
    }
    PathBuf::from(trimmed)
}

/// Prefer a peeled content-folder name over the Nexus title when wrapping.
///
/// Staging directories are `{safe}_{mod_id}_{file_id}`; those names are never used
/// as the on-disk wrap folder.
pub fn content_folder_wrap_name(content_root: &Path, staged_name: &str) -> String {
    if let Some(name) = content_root.file_name().and_then(|n| n.to_str()) {
        let sanitized = sanitize_filename::sanitize(name);
        if !sanitized.is_empty() && !looks_like_staging_dir_name(&sanitized) {
            return sanitized;
        }
    }
    sanitize_filename::sanitize(staged_name)
}

fn looks_like_staging_dir_name(name: &str) -> bool {
    let mut parts = name.rsplitn(3, '_');
    let file_id = parts.next();
    let mod_id = parts.next();
    let rest = parts.next();
    matches!(
        (file_id, mod_id, rest),
        (Some(f), Some(m), Some(r))
            if !r.is_empty() && f.parse::<u64>().is_ok() && m.parse::<u64>().is_ok()
    )
}

/// Strip a single top-level archive folder if the archive contained one root dir,
/// unless that folder is a meaningful game-root name for the plugin.
pub fn normalize_staging_root(staging_dir: &Path, plugin: &dyn GamePlugin) -> Result<PathBuf> {
    let mut entries: Vec<_> = std::fs::read_dir(staging_dir)
        .with_context(|| format!("read staging {}", staging_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            s != "__MACOSX" && !s.starts_with('.')
        })
        .collect();
    if entries.len() == 1 {
        let only = entries.remove(0);
        if only.file_type()?.is_dir() {
            let name = only.file_name();
            let name = name.to_string_lossy();
            let preserve = plugin
                .preserve_staging_root_names()
                .iter()
                .any(|root| name.eq_ignore_ascii_case(root));
            if !preserve {
                return Ok(only.path());
            }
        }
    }
    Ok(staging_dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use cyberpunk2077::Cyberpunk2077Plugin;

    #[test]
    fn peels_wrapper_but_keeps_cyberpunk_roots() {
        let staging = tempfile::tempdir().unwrap();
        let wrapper = staging.path().join("Some Mod Pack");
        std::fs::create_dir_all(wrapper.join("bin").join("x64")).unwrap();
        std::fs::write(wrapper.join("bin").join("x64").join("a.dll"), b"x").unwrap();

        let root = normalize_staging_root(staging.path(), &Cyberpunk2077Plugin).unwrap();
        assert_eq!(root, wrapper);

        let bin_only = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(bin_only.path().join("bin").join("x64")).unwrap();
        std::fs::write(bin_only.path().join("bin").join("x64").join("a.dll"), b"x").unwrap();
        let kept = normalize_staging_root(bin_only.path(), &Cyberpunk2077Plugin).unwrap();
        assert_eq!(kept, bin_only.path());

        let red4 = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(red4.path().join("red4ext").join("plugins")).unwrap();
        let kept_red = normalize_staging_root(red4.path(), &Cyberpunk2077Plugin).unwrap();
        assert_eq!(kept_red, red4.path());
    }

    #[test]
    fn content_folder_wrap_name_skips_staging_suffix() {
        let staging = tempfile::tempdir().unwrap();
        let named = staging.path().join("CoolMod_12_34");
        std::fs::create_dir_all(&named).unwrap();
        assert_eq!(
            content_folder_wrap_name(&named, "Cool Mod"),
            "Cool Mod"
        );

        let peeled = staging.path().join("CoolMod");
        std::fs::create_dir_all(&peeled).unwrap();
        assert_eq!(content_folder_wrap_name(&peeled, "Cool Mod"), "CoolMod");
    }

    #[test]
    fn peels_for_folder_style_plugins() {
        let staging = tempfile::tempdir().unwrap();
        let wrapper = staging.path().join("MyCoolMod");
        std::fs::create_dir_all(&wrapper).unwrap();
        std::fs::write(wrapper.join("manifest.json"), "{}").unwrap();
        let root = normalize_staging_root(staging.path(), &StardewValleyPlugin).unwrap();
        assert_eq!(root, wrapper);
    }

    #[test]
    fn match_re_engine_titles() {
        assert_eq!(
            match_plugin("Resident Evil 7 Biohazard", None)
                .unwrap()
                .id,
            "residentevil7"
        );
        assert_eq!(
            match_plugin("Resident Evil Village", None).unwrap().id,
            "residentevilvillage"
        );
        assert_eq!(
            match_plugin("Resident Evil Requiem", None).unwrap().id,
            "residentevilrequiem"
        );
        assert_eq!(
            match_plugin("Resident Evil 2", None).unwrap().id,
            "residentevil22019"
        );
        assert_eq!(
            match_plugin("Resident Evil 3", None).unwrap().id,
            "residentevil32020"
        );
        assert_eq!(
            match_plugin("RESIDENT EVIL 4 BIOHAZARD RE4", None)
                .unwrap()
                .id,
            "residentevil42023"
        );

        // Bare RE4 without remake install markers stays unsupported.
        assert!(match_plugin("Resident Evil 4", None).is_none());
        let remake = tempfile::tempdir().unwrap();
        std::fs::write(remake.path().join("re4.exe"), b"x").unwrap();
        assert_eq!(
            match_plugin("Resident Evil 4", Some(remake.path().to_str().unwrap()))
                .unwrap()
                .id,
            "residentevil42023"
        );
    }

    #[test]
    fn preserves_stardew_mods_root() {
        let staging = tempfile::tempdir().unwrap();
        let mods = staging.path().join("Mods");
        std::fs::create_dir_all(mods.join("CoolMod")).unwrap();
        std::fs::write(mods.join("CoolMod").join("manifest.json"), "{}").unwrap();
        let root = normalize_staging_root(staging.path(), &StardewValleyPlugin).unwrap();
        assert_eq!(root, staging.path());
    }

    #[test]
    fn darktide_preserves_loader_and_dmf_roots() {
        let loader = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(loader.path().join("tools")).unwrap();
        std::fs::create_dir_all(loader.path().join("mods")).unwrap();
        let kept = normalize_staging_root(loader.path(), &Warhammer40kDarktidePlugin).unwrap();
        assert_eq!(kept, loader.path());

        let dmf_only = tempfile::tempdir().unwrap();
        let dmf = dmf_only.path().join("dmf");
        std::fs::create_dir_all(&dmf).unwrap();
        std::fs::write(dmf.join("dmf.lua"), b"x").unwrap();
        let kept_dmf = normalize_staging_root(dmf_only.path(), &Warhammer40kDarktidePlugin).unwrap();
        assert_eq!(kept_dmf, dmf_only.path());

        let normal = tempfile::tempdir().unwrap();
        let wrapper = normal.path().join("scoreboard");
        std::fs::create_dir_all(&wrapper).unwrap();
        std::fs::write(wrapper.join("scoreboard.mod"), b"x").unwrap();
        let peeled = normalize_staging_root(normal.path(), &Warhammer40kDarktidePlugin).unwrap();
        assert_eq!(peeled, wrapper);
    }
}
