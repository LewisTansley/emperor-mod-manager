//! Per-game deploy plugins.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::Paths;
use crate::mods::options::ModOptionSet;
use crate::nexus::NexusClient;

mod baldursgate3;
mod bepinex_install;
mod bladeandsorcery;
mod controlresonant;
mod creation_engine;
mod cyberpunk2077;
mod daysgone;
mod dressmaker;
mod exe_version;
mod fromsoft_loaders;
mod fromsoftware;
mod helldivers2;
mod kingdomcome;
mod log_sources;
mod melonloader;
mod monsterhunterworld;
mod mountandblade;
mod mysummercar;
mod nomanssky;
mod projectzomboid;
mod re_engine;
mod rimworld;
mod sevendaystodie;
mod sims3;
mod sims4;
mod sims_framework;
mod snowrunner;
mod spaceengineers;
mod stardewvalley;
mod theforest;
mod unity_bepinex;
mod unreal_engine;
mod user_data;
mod warhammer40kdarktide;
mod wine_overrides;
mod witcher3;

pub use baldursgate3::BaldursGate3Plugin;
pub use bladeandsorcery::BladeAndSorceryPlugin;
pub use controlresonant::ControlResonantPlugin;
pub use creation_engine::{
    Fallout4Plugin, OblivionRemasteredPlugin, SkyrimSpecialEditionPlugin, StarfieldPlugin,
};
pub use cyberpunk2077::Cyberpunk2077Plugin;
pub use daysgone::DaysGonePlugin;
pub(crate) use dressmaker::plugin_id_for_install;
pub use dressmaker::DressmakerPlugin;
pub use exe_version::detect_install_version;
pub use fromsoft_loaders::install_legacy_from_archive;
pub use fromsoftware::{
    DarkSouls2Plugin, DarkSouls3Plugin, DarkSoulsPlugin, DarkSoulsRemasteredPlugin, EldenRingPlugin,
};
pub use helldivers2::Helldivers2Plugin;
pub use kingdomcome::{KingdomComeDeliverance2Plugin, KingdomComeDeliverancePlugin};
pub use melonloader::{BonelabPlugin, BoneworksPlugin};
pub use monsterhunterworld::MonsterHunterWorldPlugin;
pub use mountandblade::{MountAndBlade2BannerlordPlugin, MountAndBladeWarbandPlugin};
pub use mysummercar::MySummerCarPlugin;
pub use nomanssky::NoMansSkyPlugin;
pub use projectzomboid::ProjectZomboidPlugin;
pub use re_engine::{
    MonsterHunterRisePlugin, MonsterHunterWildsPlugin, ResidentEvil22019Plugin,
    ResidentEvil32020Plugin, ResidentEvil42023Plugin, ResidentEvil7Plugin,
    ResidentEvilRequiemPlugin, ResidentEvilVillagePlugin,
};
pub use rimworld::RimWorldPlugin;
pub use sevendaystodie::SevenDaysToDiePlugin;
pub use sims3::Sims3Plugin;
pub use sims4::Sims4Plugin;
pub use snowrunner::SnowRunnerPlugin;
pub use spaceengineers::SpaceEngineersPlugin;
pub use stardewvalley::StardewValleyPlugin;
pub use theforest::TheForestPlugin;
pub use unity_bepinex::{
    bepinex_pack_deploy_root, looks_like_unity_install, thunderstore_community_for_plugin,
    AgainstTheStormPlugin, AmongUsPlugin, AtlyssPlugin, BepInExBuild, BepInExPlugin, ChefRpgPlugin,
    ContentWarningPlugin, CultOfTheLambPlugin, DysonSphereProgramPlugin, GtfoPlugin, H3vrPlugin,
    HollowKnightSilksongPlugin, InscryptionPlugin, LethalCompanyPlugin, PeakPlugin, RepoPlugin,
    RiskOfRain2Plugin, RoundsPlugin, Schedule1Plugin, SonsOfTheForestPlugin,
    SubnauticaBelowZeroPlugin, SubnauticaPlugin, TimberbornPlugin, UltrakillPlugin, ValheimPlugin,
};
pub use unreal_engine::{
    detect_ue_layout, layout_info, DeepRockGalacticPlugin, DeployContext, HogwartsLegacyPlugin,
    MarvelRivalsPlugin, PalworldPlugin, PavlovPlugin, ReadyOrNotPlugin,
    Stalker2HeartOfChornobylPlugin, Subnautica2Plugin, UeLayoutInfo, UnrealEnginePlugin,
};
pub use warhammer40kdarktide::Warhammer40kDarktidePlugin;
pub use wine_overrides::{advise_wine_overrides, StagedProxySource, WineOverrideAdvice};
pub use witcher3::Witcher3Plugin;

/// What `deploy_mods` knows about the install while a plugin prepares it.
pub struct RuntimePrep<'a> {
    pub paths: &'a Paths,
    pub install_path: &'a Path,
    pub nexus: Option<&'a NexusClient>,
    pub premium: bool,
    /// Normalized content roots of enabled staged mods.
    pub content_roots: &'a [&'a Path],
}

/// Nexus download the command should open in Download Assist, then poll.
#[derive(Debug, Clone)]
pub struct LoaderAssistRequest {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub plugin_id: String,
    pub install_path: PathBuf,
    pub label: String,
}

/// Result of [`GamePlugin::prepare_runtime`].
#[derive(Debug)]
pub enum RuntimePrepOutcome {
    Done(Vec<String>),
    NeedsAssist {
        warnings: Vec<String>,
        request: LoaderAssistRequest,
    },
}

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

    /// When true, a single top-level archive folder with this name is kept.
    fn preserves_staging_root(&self, name: &str) -> bool {
        self.preserve_staging_root_names()
            .iter()
            .any(|root| name.eq_ignore_ascii_case(root))
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

    /// When false, a staged file is not linked into the game (e.g. readmes and
    /// preview images that the game never loads). `relative` is the path inside
    /// the staged mod, before any mod-folder wrapping.
    fn should_deploy_file(&self, _relative: &Path) -> bool {
        true
    }

    /// Same as `should_deploy_file`, with the staging layout available.
    fn should_deploy_file_ctx(&self, relative: &Path, ctx: &DeployContext<'_>) -> bool {
        let _ = ctx;
        self.should_deploy_file(relative)
    }

    /// Directories owned by the mod loader whose empty leftovers should be pruned
    /// on purge (e.g. `BepInEx/plugins` folders from mods that are gone).
    fn prunable_dirs(&self, _install_path: &Path) -> Vec<PathBuf> {
        Vec::new()
    }

    /// Install-level warnings shown before/during deploy (e.g. missing mod loader).
    fn preflight_warnings(&self, _install_path: &Path) -> Vec<String> {
        Vec::new()
    }

    fn preflight_warnings_ctx(&self, install_path: &Path, ctx: &DeployContext<'_>) -> Vec<String> {
        let _ = ctx;
        self.preflight_warnings(install_path)
    }

    /// Per-staging-pack warnings (e.g. SMAPI installer deployed as a mod).
    fn staging_deploy_warnings(&self, _content_root: &Path, _mod_name: &str) -> Vec<String> {
        Vec::new()
    }

    /// When true, deploying zero files from this pack is expected (a save overwrite
    /// that is applied beside the install, not linked into it).
    fn allows_empty_deploy(&self, _content_root: &Path) -> bool {
        false
    }

    /// Options the mod declares for itself (Arsenal / HD2MM `manifest.json`).
    /// `None` means the mod is deployed whole, which is the norm.
    fn mod_options(&self, _content_root: &Path) -> Option<ModOptionSet> {
        None
    }

    /// Optional install prep before purge/link (rename paks, etc.). Returns warnings.
    fn prepare_deploy(&self, _install_path: &Path) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Plugin id to persist when the install is a more specific game than the
    /// saved type. The default is this plugin's own id.
    fn preferred_plugin_id(&self, _install_path: &Path) -> &'static str {
        self.info().id
    }

    /// Loader archive this plugin wants installed when the game does not have
    /// one yet. `None` means this plugin does not auto-install BepInEx.
    fn bepinex_loader(&self, _install_path: &Path) -> Option<BepInExBuild> {
        None
    }

    /// Download or cache anything the game needs before files are linked.
    /// The default installs BepInEx when [`Self::bepinex_loader`] returns a build.
    fn prepare_runtime<'a>(
        &'a self,
        ctx: &'a RuntimePrep<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<RuntimePrepOutcome>> + Send + 'a>> {
        let build = self.bepinex_loader(ctx.install_path);
        let install = unity_bepinex::should_auto_install_bepinex(
            ctx.install_path,
            ctx.content_roots,
            build,
            |root| self.allows_empty_deploy(root),
        );
        Box::pin(async move {
            let Some(build) = build.filter(|_| install) else {
                return Ok(RuntimePrepOutcome::Done(Vec::new()));
            };
            let warnings =
                bepinex_install::ensure_bepinex(ctx.paths, ctx.install_path, build).await;
            Ok(RuntimePrepOutcome::Done(warnings))
        })
    }

    /// Copy a loader into the game after Download Assist finishes.
    fn finish_runtime_prep(&self, _ctx: &RuntimePrep<'_>) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Whether the Assist download requested by `prepare_runtime` is on disk.
    fn loader_assist_ready(&self, _ctx: &RuntimePrep<'_>) -> bool {
        false
    }

    /// Place staged files that must not be linked into the game (save overwrites).
    fn deploy_staged_extra(
        &self,
        _install_path: &Path,
        _content_root: &Path,
        _mod_name: &str,
    ) -> Result<Vec<String>> {
        Ok(Vec::new())
    }

    /// Names this staged mod contributes to a native load-order file.
    ///
    /// The default is the wrap folder name when the mod is deployed under one
    /// folder, which is what `after_deploy` has always received.
    fn load_order_entries(&self, content_root: &Path, staged_name: &str) -> Vec<String> {
        let wrap = !self.deploys_to_install_root(content_root)
            && (self.prefers_mod_folder() || self.should_wrap_as_mod_folder(content_root));
        if !wrap {
            return Vec::new();
        }
        let name = self.wrap_mod_folder_name(content_root, staged_name);
        if name.is_empty() {
            Vec::new()
        } else {
            vec![name]
        }
    }

    /// Called after files are linked. `enabled_mod_folders` are load-order entries
    /// from `load_order_entries` (wrap folder names, unless a plugin overrides that).
    /// Returns optional warning strings (deploy succeeds even if warnings are returned).
    fn after_deploy(
        &self,
        _install_path: &Path,
        _enabled_mod_folders: &[String],
    ) -> Result<Vec<String>> {
        Ok(vec![])
    }

    /// Plugins left out of the load order because a master file is missing.
    fn plugin_skips(&self, _install_path: &Path) -> Vec<PluginSkip> {
        Vec::new()
    }

    /// Loader and crash logs this game writes. Missing files are included so a
    /// diagnosis can say the loader never ran.
    fn load_log_sources(&self, _install_path: &Path) -> Vec<LogSource> {
        Vec::new()
    }
}

/// A loader or engine log the diagnosis pass knows how to read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogKind {
    BepInEx,
    MelonLoader,
    Smapi,
    ReFramework,
    Ue4ss,
    ScriptExtender,
    CrashLog,
    /// A plugin's own text log next to the script extender log.
    PluginLog,
    /// Papyrus script log. Only useful when it is from the same launch.
    Papyrus,
    /// Binary minidump. Mentioned, never parsed.
    CrashDump,
    Ds2Lighting,
    CrLoader,
    Player,
    Console,
}

#[derive(Debug, Clone)]
pub struct LogSource {
    pub label: String,
    pub kind: LogKind,
    pub path: PathBuf,
}

/// A plugin omitted at deploy because one of its masters is not loadable.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginSkip {
    pub plugin: String,
    pub missing_master: String,
}

pub fn all_plugins() -> Vec<&'static dyn GamePlugin> {
    // More specific match_names first (FromSoft + RE Engine remakes + UE + BepInEx titles).
    // Generic UnrealEnginePlugin / BepInExPlugin have empty match_names (detection via engine_hint).
    vec![
        &StardewValleyPlugin,
        &BaldursGate3Plugin,
        &Cyberpunk2077Plugin,
        &Witcher3Plugin,
        &ControlResonantPlugin,
        &DaysGonePlugin,
        &Warhammer40kDarktidePlugin,
        &Helldivers2Plugin,
        &Stalker2HeartOfChornobylPlugin,
        &PalworldPlugin,
        &HogwartsLegacyPlugin,
        &ReadyOrNotPlugin,
        &Subnautica2Plugin,
        &MarvelRivalsPlugin,
        &DeepRockGalacticPlugin,
        &PavlovPlugin,
        &OblivionRemasteredPlugin,
        &UnrealEnginePlugin,
        &HollowKnightSilksongPlugin,
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
        &SonsOfTheForestPlugin,
        &SubnauticaBelowZeroPlugin,
        &SubnauticaPlugin,
        &Schedule1Plugin,
        &RepoPlugin,
        &PeakPlugin,
        &H3vrPlugin,
        &UltrakillPlugin,
        &AtlyssPlugin,
        &DysonSphereProgramPlugin,
        &InscryptionPlugin,
        &BonelabPlugin,
        &BoneworksPlugin,
        &BladeAndSorceryPlugin,
        &TheForestPlugin,
        &MySummerCarPlugin,
        &SevenDaysToDiePlugin,
        &ProjectZomboidPlugin,
        &RimWorldPlugin,
        &NoMansSkyPlugin,
        &Sims4Plugin,
        &Sims3Plugin,
        &SnowRunnerPlugin,
        &SpaceEngineersPlugin,
        &SkyrimSpecialEditionPlugin,
        &Fallout4Plugin,
        &StarfieldPlugin,
        &MountAndBlade2BannerlordPlugin,
        &MountAndBladeWarbandPlugin,
        &KingdomComeDeliverance2Plugin,
        &KingdomComeDeliverancePlugin,
        &DressmakerPlugin,
        &ChefRpgPlugin,
        &BepInExPlugin,
        &EldenRingPlugin,
        &DarkSouls3Plugin,
        &DarkSouls2Plugin,
        &DarkSoulsRemasteredPlugin,
        &DarkSoulsPlugin,
        &MonsterHunterWildsPlugin,
        &MonsterHunterRisePlugin,
        &MonsterHunterWorldPlugin,
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

/// Serializes tests that deploy Helldivers 2 mods, which share a process-global
/// patch index.
#[cfg(test)]
pub(crate) fn hd2_test_gate() -> std::sync::MutexGuard<'static, ()> {
    helldivers2::TEST_GATE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Lowercase a store title and drop trademark marks.
///
/// Steam and `lib_game_detector` report names like `The Sims™ 4`, which would
/// otherwise never match a `sims 4` entry in `match_names`.
pub fn normalize_title(title: &str) -> String {
    let stripped: String = title
        .chars()
        .map(|c| {
            if matches!(c, '™' | '®' | '©') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut out = String::with_capacity(stripped.len());
    for word in stripped.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&word.to_lowercase());
    }
    out
}

pub fn match_plugin(title: &str, install_path: Option<&str>) -> Option<GamePluginInfo> {
    let lower = normalize_title(title);
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
            let preserve = plugin.preserves_staging_root(&name);
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
        assert_eq!(content_folder_wrap_name(&named, "Cool Mod"), "Cool Mod");

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
    fn match_ignores_trademark_symbols_in_store_titles() {
        // Steam and lib_game_detector report titles with ™ / ®.
        assert_eq!(
            match_plugin("The Witcher® 3: Wild Hunt", None).unwrap().id,
            "witcher3"
        );
        assert_eq!(normalize_title("The Sims™ 4"), "the sims 4");
        assert_eq!(normalize_title("HELLDIVERS™ 2"), "helldivers 2");
        assert_eq!(match_plugin("The Sims™ 4", None).unwrap().id, "sims4");
        assert_eq!(match_plugin("The Sims™ 3", None).unwrap().id, "sims3");
    }

    #[test]
    fn match_re_engine_titles() {
        assert_eq!(
            match_plugin("Resident Evil 7 Biohazard", None).unwrap().id,
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
        let kept_dmf =
            normalize_staging_root(dmf_only.path(), &Warhammer40kDarktidePlugin).unwrap();
        assert_eq!(kept_dmf, dmf_only.path());

        let normal = tempfile::tempdir().unwrap();
        let wrapper = normal.path().join("scoreboard");
        std::fs::create_dir_all(&wrapper).unwrap();
        std::fs::write(wrapper.join("scoreboard.mod"), b"x").unwrap();
        let peeled = normalize_staging_root(normal.path(), &Warhammer40kDarktidePlugin).unwrap();
        assert_eq!(peeled, wrapper);
    }

    #[test]
    fn match_subnautica_family_and_forest_order() {
        assert_eq!(
            match_plugin("Subnautica 2", None).unwrap().id,
            "subnautica2"
        );
        assert_eq!(
            match_plugin("Subnautica: Below Zero", None).unwrap().id,
            "subnauticabelowzero"
        );
        assert_eq!(match_plugin("Subnautica", None).unwrap().id, "subnautica");
        assert_eq!(
            match_plugin("Sons of the Forest", None).unwrap().id,
            "sonsoftheforest"
        );
        assert_eq!(match_plugin("Ready or Not", None).unwrap().id, "readyornot");
        assert_eq!(match_plugin("Schedule I", None).unwrap().id, "schedule1");
        assert_eq!(
            match_plugin("Monster Hunter Wilds", None).unwrap().id,
            "monsterhunterwilds"
        );
        assert_eq!(
            match_plugin("Monster Hunter Rise", None).unwrap().id,
            "monsterhunterrise"
        );
        assert_eq!(match_plugin("The Forest", None).unwrap().id, "theforest");
        assert_eq!(
            match_plugin("The Witcher 3: Wild Hunt", None).unwrap().id,
            "witcher3"
        );
        assert_eq!(
            match_plugin("HELLDIVERS 2", None).unwrap().id,
            "helldivers2"
        );
        assert_eq!(
            match_plugin("Blade & Sorcery", None).unwrap().id,
            "bladeandsorcery"
        );
        assert_eq!(
            match_plugin("My Summer Car", None).unwrap().id,
            "mysummercar"
        );
        assert_eq!(match_plugin("Dressmaker", None).unwrap().id, "dressmaker");
        assert_eq!(
            match_plugin("CONTROL Resonant", None).unwrap().id,
            "controlresonant"
        );
        assert!(match_plugin("Control", None).is_none());
        assert!(match_plugin("Control Ultimate Edition", None).is_none());
        assert_eq!(
            match_plugin("Monster Hunter: World", None).unwrap().id,
            "monsterhunterworld"
        );
        assert_eq!(match_plugin("R.E.P.O.", None).unwrap().id, "repo");
        assert_eq!(match_plugin("PEAK", None).unwrap().id, "peak");
        assert_eq!(
            match_plugin("Hot Dogs, Horseshoes & Hand Grenades", None)
                .unwrap()
                .id,
            "h3vr"
        );
        assert_eq!(match_plugin("ULTRAKILL", None).unwrap().id, "ultrakill");
        assert_eq!(match_plugin("ATLYSS", None).unwrap().id, "atlyss");
        assert_eq!(
            match_plugin("Dyson Sphere Program", None).unwrap().id,
            "dysonsphereprogram"
        );
        assert_eq!(match_plugin("Inscryption", None).unwrap().id, "inscryption");
        assert_eq!(
            match_plugin("Hollow Knight: Silksong", None).unwrap().id,
            "hollowknightsilksong"
        );
        assert_eq!(
            match_plugin("7 Days to Die", None).unwrap().id,
            "7daystodie"
        );
        assert_eq!(match_plugin("RimWorld", None).unwrap().id, "rimworld");
        assert_eq!(match_plugin("No Man's Sky", None).unwrap().id, "nomanssky");
        assert_eq!(
            match_plugin("Marvel Rivals", None).unwrap().id,
            "marvelrivals"
        );
        assert_eq!(match_plugin("BONELAB", None).unwrap().id, "bonelab");
        assert_eq!(match_plugin("BONEWORKS", None).unwrap().id, "boneworks");
        assert_eq!(
            match_plugin("Deep Rock Galactic", None).unwrap().id,
            "deeprockgalactic"
        );
        assert_eq!(match_plugin("Pavlov VR", None).unwrap().id, "pavlov");
        assert_eq!(match_plugin("SnowRunner", None).unwrap().id, "snowrunner");
        assert_eq!(
            match_plugin("Space Engineers", None).unwrap().id,
            "spaceengineers"
        );
        assert_eq!(
            match_plugin("The Elder Scrolls V: Skyrim Special Edition", None)
                .unwrap()
                .id,
            "skyrimspecialedition"
        );
        assert_eq!(match_plugin("Fallout 4", None).unwrap().id, "fallout4");
        assert_eq!(match_plugin("Starfield", None).unwrap().id, "starfield");
        assert_eq!(
            match_plugin("The Elder Scrolls IV: Oblivion Remastered", None)
                .unwrap()
                .id,
            "oblivionremastered"
        );
    }
}
