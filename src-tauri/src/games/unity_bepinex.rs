//! Shared deploy logic for Unity titles modded with BepInEx.
//!
//! Typical layout:
//! ```text
//! <Install>/BepInEx/core/                    # standard flat install
//! <Install>/BepInEx/plugins/
//! <Install>/BepInExPack/BepInEx/core/      # Thunderstore pack wrapper
//! <Install>/BepInExPack/BepInEx/plugins/
//! <Install>/winhttp.dll | version.dll        # Doorstop
//! ```

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

/// Roots that must not be peeled as archive wrappers for BepInEx mods.
pub const BEPINEX_PRESERVE_ROOTS: &[&str] = &[
    "BepInEx",
    "BepInExPack",
    "plugins",
    "patchers",
    "config",
    "core",
    "doorstop_libs",
    "dotnet",
    "MonoBleedingEdge",
];

/// Doorstop / proxy injector filenames commonly shipped with BepInEx packs.
const DOORSTOP_FILES: &[&str] = &[
    "winhttp.dll",
    "version.dll",
    "winmm.dll",
    "doorstop_config.ini",
    ".doorstop_version",
];

/// Subdirectories BepInEx itself loads from, relative to the `BepInEx/` root.
const BEPINEX_SUBDIRS: &[&str] = &["plugins", "patchers", "config", "core", "monomod", "cache"];

/// Thunderstore package metadata that BepInEx never loads. Kept for wrapped mods
/// (BepInEx.GUI reads `manifest.json` next to a plugin to show "TS Manifest"),
/// dropped when a mod merges into the shared `BepInEx/` tree.
const THUNDERSTORE_METADATA: &[&str] = &[
    "manifest.json",
    "icon.png",
    "readme.md",
    "changelog.md",
    "license",
    "license.md",
    "license.txt",
];

/// Seeded Unity / BepInEx titles with Nexus domain + Thunderstore community.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct BepInExTitleSeed {
    pub plugin_id: &'static str,
    pub display_name: &'static str,
    pub nexus_domain: &'static str,
    pub thunderstore_community: &'static str,
    pub match_names: &'static [&'static str],
    /// Community BepInEx pack `(namespace, name)`. `None` uses the runtime GitHub build.
    pub pack: Option<(&'static str, &'static str)>,
}

pub const BEPINEX_TITLE_SEEDS: &[BepInExTitleSeed] = &[
    BepInExTitleSeed {
        plugin_id: "lethalcompany",
        display_name: "Lethal Company",
        nexus_domain: "lethalcompany",
        thunderstore_community: "lethal-company",
        match_names: &["lethal company", "lethalcompany"],
        pack: Some(("BepInEx", "BepInExPack")),
    },
    BepInExTitleSeed {
        plugin_id: "valheim",
        display_name: "Valheim",
        nexus_domain: "valheim",
        thunderstore_community: "valheim",
        match_names: &["valheim"],
        pack: Some(("denikson", "BepInExPack_Valheim")),
    },
    BepInExTitleSeed {
        plugin_id: "riskofrain2",
        display_name: "Risk of Rain 2",
        nexus_domain: "riskofrain2",
        thunderstore_community: "riskofrain2",
        match_names: &["risk of rain 2", "riskofrain2", "ror2"],
        pack: Some(("bbepis", "BepInExPack")),
    },
    BepInExTitleSeed {
        plugin_id: "amongus",
        display_name: "Among Us",
        nexus_domain: "amongus",
        thunderstore_community: "among-us",
        match_names: &["among us", "amongus"],
        pack: Some(("BepInEx", "BepInExPack_AmongUs")),
    },
    BepInExTitleSeed {
        plugin_id: "contentwarning",
        display_name: "Content Warning",
        nexus_domain: "contentwarning",
        thunderstore_community: "content-warning",
        match_names: &["content warning", "contentwarning"],
        pack: Some(("BepInEx", "BepInExPack")),
    },
    BepInExTitleSeed {
        plugin_id: "gtfo",
        display_name: "GTFO",
        nexus_domain: "gtfo",
        thunderstore_community: "gtfo",
        match_names: &["gtfo"],
        pack: Some(("BepInEx", "BepInExPack_GTFO")),
    },
    BepInExTitleSeed {
        plugin_id: "timberborn",
        display_name: "Timberborn",
        nexus_domain: "timberborn",
        thunderstore_community: "timberborn",
        match_names: &["timberborn"],
        pack: Some(("BepInEx", "BepInExPack_Timberborn")),
    },
    BepInExTitleSeed {
        plugin_id: "cultofthelamb",
        display_name: "Cult of the Lamb",
        nexus_domain: "cultofthelamb",
        thunderstore_community: "cult-of-the-lamb",
        match_names: &["cult of the lamb", "cultofthelamb"],
        pack: Some(("BepInEx", "BepInExPack_CultOfTheLamb")),
    },
    BepInExTitleSeed {
        plugin_id: "againstthestorm",
        display_name: "Against the Storm",
        nexus_domain: "againstthestorm",
        thunderstore_community: "against-the-storm",
        match_names: &["against the storm", "againstthestorm"],
        pack: Some(("BepInEx", "BepInExPack")),
    },
    BepInExTitleSeed {
        plugin_id: "rounds",
        display_name: "ROUNDS",
        nexus_domain: "rounds",
        thunderstore_community: "rounds",
        match_names: &["rounds"],
        pack: Some(("BepInEx", "BepInExPack_ROUNDS")),
    },
    BepInExTitleSeed {
        plugin_id: "sonsoftheforest",
        display_name: "Sons of the Forest",
        nexus_domain: "sonsoftheforest",
        thunderstore_community: "sons-of-the-forest",
        match_names: &["sons of the forest", "sonsoftheforest"],
        pack: Some(("BepInEx", "BepInExPack_IL2CPP")),
    },
    BepInExTitleSeed {
        plugin_id: "subnauticabelowzero",
        display_name: "Subnautica: Below Zero",
        nexus_domain: "subnauticabelowzero",
        thunderstore_community: "subnautica-below-zero",
        match_names: &["below zero", "subnauticabelowzero"],
        pack: Some(("Subnautica_Modding", "BepInExPack_BelowZero")),
    },
    BepInExTitleSeed {
        plugin_id: "subnautica",
        display_name: "Subnautica",
        nexus_domain: "subnautica",
        thunderstore_community: "subnautica",
        match_names: &["subnautica"],
        pack: Some(("Subnautica_Modding", "BepInExPack_Subnautica")),
    },
    BepInExTitleSeed {
        plugin_id: "schedule1",
        display_name: "Schedule I",
        nexus_domain: "schedule1",
        thunderstore_community: "schedule-i",
        match_names: &["schedule 1", "schedule i", "schedule1"],
        pack: None,
    },
    BepInExTitleSeed {
        plugin_id: "repo",
        display_name: "R.E.P.O.",
        nexus_domain: "repo",
        thunderstore_community: "repo",
        match_names: &["r.e.p.o.", "r.e.p.o", "repo"],
        pack: Some(("BepInEx", "BepInExPack")),
    },
    BepInExTitleSeed {
        plugin_id: "peak",
        display_name: "PEAK",
        nexus_domain: "peak",
        thunderstore_community: "peak",
        match_names: &["peak"],
        pack: Some(("BepInEx", "BepInExPack")),
    },
    BepInExTitleSeed {
        plugin_id: "h3vr",
        display_name: "H3VR",
        nexus_domain: "h3vr",
        thunderstore_community: "h3vr",
        match_names: &[
            "hot dogs, horseshoes and hand grenades",
            "hot dogs, horseshoes",
            "hot dogs horseshoes",
            "h3vr",
        ],
        pack: Some(("BepInEx", "BepInExPack_H3VR")),
    },
    BepInExTitleSeed {
        plugin_id: "ultrakill",
        display_name: "ULTRAKILL",
        nexus_domain: "ultrakill",
        thunderstore_community: "ultrakill",
        match_names: &["ultrakill"],
        pack: Some(("BepInEx", "BepInExPack")),
    },
    BepInExTitleSeed {
        plugin_id: "atlyss",
        display_name: "ATLYSS",
        nexus_domain: "atlyss",
        thunderstore_community: "atlyss",
        match_names: &["atlyss"],
        pack: Some(("BepInEx", "BepInExPack")),
    },
    BepInExTitleSeed {
        plugin_id: "dysonsphereprogram",
        display_name: "Dyson Sphere Program",
        nexus_domain: "dysonsphereprogram",
        thunderstore_community: "dyson-sphere-program",
        match_names: &["dyson sphere program", "dysonsphereprogram"],
        pack: Some(("xiaoye97", "BepInEx")),
    },
    BepInExTitleSeed {
        plugin_id: "inscryption",
        display_name: "Inscryption",
        nexus_domain: "inscryption",
        thunderstore_community: "inscryption",
        match_names: &["inscryption"],
        pack: Some(("BepInEx", "BepInExPack_Inscryption")),
    },
    BepInExTitleSeed {
        plugin_id: "hollowknightsilksong",
        display_name: "Hollow Knight: Silksong",
        nexus_domain: "hollowknightsilksong",
        thunderstore_community: "hollow-knight-silksong",
        match_names: &[
            "silksong",
            "hollow knight: silksong",
            "hollowknightsilksong",
        ],
        pack: Some(("BepInEx", "BepInExPack_Silksong")),
    },
    BepInExTitleSeed {
        plugin_id: "chefrpg",
        display_name: "Chef RPG",
        nexus_domain: "chefrpg",
        thunderstore_community: "",
        match_names: &["chef rpg", "chefrpg"],
        pack: None,
    },
];

pub fn seed_by_plugin_id(plugin_id: &str) -> Option<&'static BepInExTitleSeed> {
    BEPINEX_TITLE_SEEDS
        .iter()
        .find(|s| s.plugin_id.eq_ignore_ascii_case(plugin_id))
}

#[allow(dead_code)]
pub fn seed_by_nexus_domain(domain: &str) -> Option<&'static BepInExTitleSeed> {
    BEPINEX_TITLE_SEEDS
        .iter()
        .find(|s| s.nexus_domain.eq_ignore_ascii_case(domain))
}

pub fn thunderstore_community_for_plugin(plugin_id: &str) -> Option<&'static str> {
    if let Some(s) = seed_by_plugin_id(plugin_id) {
        if s.thunderstore_community.is_empty() {
            return None;
        }
        return Some(s.thunderstore_community);
    }
    extra_thunderstore_community(plugin_id)
}

/// Thunderstore communities for named titles that are not BepInEx.
fn extra_thunderstore_community(plugin_id: &str) -> Option<&'static str> {
    match plugin_id {
        "bladeandsorcery" => Some("blade-and-sorcery"),
        "bonelab" => Some("bonelab"),
        "boneworks" => Some("boneworks"),
        _ => None,
    }
}

const BEPINEX_REPO: &str = "BepInEx/BepInEx";
const MONO_TAG: &str = "v5.4.23.5";
const MONO_ASSET: &str = "BepInEx_win_x64_5.4.23.5.zip";
const MONO_CACHE: &str = "win-x64-5.4.23.5";
const IL2CPP_TAG: &str = "v6.0.0-pre.2";
const IL2CPP_ASSET: &str = "BepInEx-Unity.IL2CPP-win-x64-6.0.0-pre.2.zip";
const IL2CPP_CACHE: &str = "il2cpp-win-x64-6.0.0-pre.2";
const CHEF_RPG_APP_ID: &str = "1796790";

/// Where an automatic BepInEx install comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BepInExBuild {
    Thunderstore {
        community: &'static str,
        namespace: &'static str,
        name: &'static str,
    },
    Github {
        tag: &'static str,
        asset: &'static str,
        cache_id: &'static str,
        label: &'static str,
        il2cpp: bool,
    },
}

impl BepInExBuild {
    pub fn cache_id(self) -> String {
        match self {
            Self::Thunderstore {
                namespace, name, ..
            } => format!("ts-{namespace}-{name}"),
            Self::Github { cache_id, .. } => cache_id.to_string(),
        }
    }

    pub fn repo(self) -> &'static str {
        BEPINEX_REPO
    }
}

fn mono_github_build() -> BepInExBuild {
    BepInExBuild::Github {
        tag: MONO_TAG,
        asset: MONO_ASSET,
        cache_id: MONO_CACHE,
        label: "BepInEx 5.4.23.5",
        il2cpp: false,
    }
}

fn il2cpp_github_build() -> BepInExBuild {
    BepInExBuild::Github {
        tag: IL2CPP_TAG,
        asset: IL2CPP_ASSET,
        cache_id: IL2CPP_CACHE,
        label: "BepInEx 6.0.0-pre.2 (IL2CPP)",
        il2cpp: true,
    }
}

pub fn is_bepinex_auto_plugin(plugin_id: &str) -> bool {
    plugin_id.eq_ignore_ascii_case("bepinex") || seed_by_plugin_id(plugin_id).is_some()
}

/// Loader to install when this game has no BepInEx yet.
///
/// Schedule I's Thunderstore community uses MelonLoader, so it is not given a
/// BepInEx build. Chef RPG is pinned to the Mono 5.4.23.5 zip Nexus documents.
pub fn select_bepinex_build(plugin_id: &str, runtime: UnityRuntime) -> Option<BepInExBuild> {
    if !is_bepinex_auto_plugin(plugin_id) {
        return None;
    }
    if plugin_id.eq_ignore_ascii_case("schedule1") {
        return None;
    }
    if plugin_id.eq_ignore_ascii_case("chefrpg") {
        return Some(mono_github_build());
    }
    if let Some(seed) = seed_by_plugin_id(plugin_id) {
        if let Some((namespace, name)) = seed.pack {
            return Some(BepInExBuild::Thunderstore {
                community: seed.thunderstore_community,
                namespace,
                name,
            });
        }
    }
    Some(match runtime {
        UnityRuntime::Il2Cpp => il2cpp_github_build(),
        UnityRuntime::Mono | UnityRuntime::Unknown => mono_github_build(),
    })
}

/// A Chef RPG character/staff download: `UnassignedStaffData.dat` and no plugin DLL.
pub fn looks_like_chefrpg_save_pack(content_root: &Path) -> bool {
    if !content_root.is_dir() {
        return false;
    }
    let mut has_staff = false;
    let mut has_dll = false;
    for entry in walkdir::WalkDir::new(content_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if name.eq_ignore_ascii_case("UnassignedStaffData.dat") {
            has_staff = true;
        }
        if name.to_ascii_lowercase().ends_with(".dll") {
            has_dll = true;
        }
    }
    has_staff && !has_dll
}

/// Skip the download when BepInEx is already there, nothing enabled needs it,
/// or an enabled mod is itself the pack that deploy will install.
/// `build` is the plugin's loader choice; a save overwrite
/// ([`GamePlugin::allows_empty_deploy`]) does not count as needing a loader.
pub fn should_auto_install_bepinex(
    install_path: &Path,
    content_roots: &[&Path],
    build: Option<BepInExBuild>,
    allows_empty: impl Fn(&Path) -> bool,
) -> bool {
    if build.is_none() {
        return false;
    }
    if bepinex_present(install_path) {
        return false;
    }
    let mut needs = false;
    for root in content_roots {
        if looks_like_bepinex_pack(root) {
            return false;
        }
        if !allows_empty(root) {
            needs = true;
        }
    }
    needs
}

/// A generic BepInEx game whose executable is Chef RPG uses the Chef RPG plugin.
pub fn specialize_bepinex_plugin_id<'a>(plugin_id: &'a str, install_path: &Path) -> &'a str {
    if plugin_id == "bepinex" && install_path.join("Chef RPG.exe").is_file() {
        "chefrpg"
    } else {
        plugin_id
    }
}

pub fn bepinex_cache_ready(cache: &Path) -> bool {
    looks_like_bepinex_pack(cache) || cache.join("BepInEx").join("core").is_dir()
}

/// Copy a cached pack into the game root. Files are copied, not linked.
pub fn copy_bepinex_tree(cache: &Path, install_path: &Path) -> Result<()> {
    let plugin = BepInExPlugin;
    let root = super::normalize_staging_root(cache, &plugin)?;
    let from = if plugin.deploys_to_install_root(&root) {
        bepinex_pack_deploy_root(&root)
    } else {
        root
    };
    let mut copied = 0usize;
    for entry in walkdir::WalkDir::new(&from)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = super::normalize_relative(entry.path().strip_prefix(&from)?);
        let ctx = super::DeployContext {
            project_name: None,
            content_root: Some(&from),
            source_dir: rel.parent(),
        };
        if !plugin.should_deploy_file_ctx(&rel, &ctx) {
            continue;
        }
        let dest = plugin.resolve_deploy_root(install_path, &rel)?;
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if dest.exists() {
            std::fs::remove_file(&dest)?;
        }
        std::fs::copy(entry.path(), &dest)
            .with_context(|| format!("copy {} to {}", entry.path().display(), dest.display()))?;
        copied += 1;
    }
    if copied == 0 {
        anyhow::bail!(
            "BepInEx cache at {} had no loader files to copy",
            cache.display()
        );
    }
    Ok(())
}

fn find_named_file(dir: &Path, file_name: &str) -> Option<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .find(|e| {
            e.file_type().is_file()
                && e.file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(file_name)
        })
        .map(|e| e.path().to_path_buf())
}

fn chef_full_data(install_path: &Path) -> Option<PathBuf> {
    let local = super::user_data::appdata_local_dir(install_path, CHEF_RPG_APP_ID)?;
    let low = local.parent()?.join("LocalLow");
    Some(
        low.join("World 2 Studio Inc")
            .join("Chef RPG")
            .join("FullData"),
    )
}

fn newest_gameplay_slot(install_path: &Path) -> Option<PathBuf> {
    let full = chef_full_data(install_path)?;
    let mut slots: Vec<PathBuf> = std::fs::read_dir(full)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir() && path.join("GameData.dat").is_file())
        .collect();
    slots.sort_by_key(|path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    });
    slots.pop()
}

fn install_one_chefrpg_save(install_path: &Path, content_root: &Path) -> Vec<String> {
    let Some(src) = find_named_file(content_root, "UnassignedStaffData.dat") else {
        return vec!["Chef RPG save mod is missing UnassignedStaffData.dat.".into()];
    };
    let Some(slot) = newest_gameplay_slot(install_path) else {
        return vec![
            "Chef RPG save mod was not deployed. It replaces FullData/SaveSlotN/UnassignedStaffData.dat, and no gameplay slot with GameData.dat was found."
                .into(),
        ];
    };
    let dest = slot.join("UnassignedStaffData.dat");
    let bak = slot.join("UnassignedStaffData.dat.bak");
    let slot_name = slot
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| slot.display().to_string());
    let had_previous = dest.is_file();
    if had_previous && !bak.is_file() {
        if let Err(e) = std::fs::copy(&dest, &bak) {
            return vec![format!(
                "Could not back up {} before replacing it ({e}).",
                dest.display()
            )];
        }
    }
    if let Some(parent) = dest.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return vec![format!("Could not create {slot_name} ({e}).")];
        }
    }
    if let Err(e) = std::fs::copy(&src, &dest) {
        return vec![format!(
            "Could not write UnassignedStaffData.dat into {slot_name} ({e})."
        )];
    }
    if had_previous {
        vec![format!(
            "Replaced UnassignedStaffData.dat in {slot_name}. Current staff must already be in unassigned slots. The previous file was kept as UnassignedStaffData.dat.bak."
        )]
    } else {
        vec![format!(
            "Installed UnassignedStaffData.dat into {slot_name}. Current staff must already be in unassigned slots."
        )]
    }
}

/// Copy Chef RPG staff overwrites into the newest gameplay save. Images and
/// readmes are left staged; they are not game files.
pub fn install_chefrpg_save_mods(install_path: &Path, content_roots: &[&Path]) -> Vec<String> {
    let mut warnings = Vec::new();
    for root in content_roots {
        if looks_like_chefrpg_save_pack(root) {
            warnings.extend(install_one_chefrpg_save(install_path, root));
        }
    }
    warnings
}

/// True when the install looks like a Unity game (or already has BepInEx).
pub fn looks_like_unity_install(install_path: &Path) -> bool {
    if !install_path.is_dir() {
        return false;
    }
    if install_path.join("BepInEx").is_dir() {
        return true;
    }
    if install_path.join("UnityPlayer.dll").is_file()
        || install_path.join("UnityPlayer.so").is_file()
        || install_path.join("UnityPlayer").is_file()
    {
        return true;
    }
    if install_path.join("GameAssembly.dll").is_file()
        || install_path.join("GameAssembly.so").is_file()
    {
        return true;
    }
    // Mono: <Name>_Data/Managed
    let Ok(entries) = std::fs::read_dir(install_path) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with("_Data") && entry.path().join("Managed").is_dir() {
            return true;
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnityRuntime {
    Mono,
    Il2Cpp,
    Unknown,
}

pub fn detect_unity_runtime(install_path: &Path) -> UnityRuntime {
    if install_path.join("GameAssembly.dll").is_file()
        || install_path.join("GameAssembly.so").is_file()
    {
        return UnityRuntime::Il2Cpp;
    }
    let Ok(entries) = std::fs::read_dir(install_path) else {
        return UnityRuntime::Unknown;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.ends_with("_Data") && entry.path().join("Managed").is_dir() {
            return UnityRuntime::Mono;
        }
    }
    UnityRuntime::Unknown
}

fn is_bepinex_core_dir(path: &Path) -> bool {
    path.join("core").is_dir()
}

/// Find the BepInEx tree that actually contains the loader (`core/`).
pub fn detect_bepinex_root(install_path: &Path) -> Option<PathBuf> {
    if !install_path.is_dir() {
        return None;
    }
    let standard = install_path.join("BepInEx");
    if is_bepinex_core_dir(&standard) {
        return Some(standard);
    }
    let pack = install_path.join("BepInExPack").join("BepInEx");
    if is_bepinex_core_dir(&pack) {
        return Some(pack);
    }
    let Ok(entries) = std::fs::read_dir(install_path) else {
        return None;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let candidate = entry.path().join("BepInEx");
        if is_bepinex_core_dir(&candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Plugins folder for the active BepInEx install, or the default when not installed yet.
pub fn bepinex_plugins_dir(install_path: &Path) -> PathBuf {
    detect_bepinex_root(install_path)
        .map(|root| root.join("plugins"))
        .unwrap_or_else(|| install_path.join("BepInEx").join("plugins"))
}

fn doorstop_present_in_dir(dir: &Path) -> bool {
    for name in DOORSTOP_FILES {
        if name.ends_with(".dll") && dir.join(name).is_file() {
            return true;
        }
    }
    dir.join("doorstop_libs").is_dir()
        || dir.join(".doorstop_version").is_file()
        || dir.join("doorstop_config.ini").is_file()
}

pub fn bepinex_present(install_path: &Path) -> bool {
    if detect_bepinex_root(install_path).is_some() {
        return true;
    }
    if doorstop_present_in_dir(install_path) {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(install_path) else {
        return false;
    };
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
            && doorstop_present_in_dir(&entry.path())
        {
            return true;
        }
    }
    false
}

/// True when staged content is a full BepInEx / Doorstop pack (deploy to install root).
fn is_bepinex_pack_tree(content_root: &Path) -> bool {
    content_root.join("BepInEx").join("core").is_dir()
}

fn nested_pack_dir(dir: &Path) -> bool {
    if dir.join("BepInEx").join("core").is_dir() {
        return true;
    }
    dir.join("BepInEx").is_dir()
        && (dir.join("doorstop_config.ini").is_file()
            || dir.join("winhttp.dll").is_file()
            || dir.join("version.dll").is_file())
}

pub fn looks_like_bepinex_pack(content_root: &Path) -> bool {
    if is_bepinex_pack_tree(content_root) {
        return true;
    }
    for name in DOORSTOP_FILES {
        if content_root.join(name).is_file() {
            return true;
        }
    }
    if content_root.join("doorstop_libs").is_dir() {
        return true;
    }
    // Thunderstore BepInExPack often nests under BepInExPack_*/BepInEx
    if let Ok(entries) = std::fs::read_dir(content_root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.starts_with("bepinexpack") && nested_pack_dir(&entry.path()) {
                return true;
            }
            if nested_pack_dir(&entry.path()) {
                return true;
            }
        }
    }
    false
}

fn is_bepinexpack_wrapper(dir: &Path) -> bool {
    nested_pack_dir(dir) || dir.join("dotnet").is_dir()
}

/// When a Thunderstore pack nests under `BepInExPack/`, deploy its contents to the game root.
pub fn bepinex_pack_deploy_root(content_root: &Path) -> PathBuf {
    let wrapper = content_root.join("BepInExPack");
    if wrapper.is_dir() && is_bepinexpack_wrapper(&wrapper) {
        return wrapper;
    }
    content_root.to_path_buf()
}

/// True when content looks like a plugin (DLL / plugin folder) rather than a full pack.
pub fn looks_like_bepinex_plugin(content_root: &Path) -> bool {
    if looks_like_bepinex_pack(content_root) {
        return false;
    }
    if content_root.join("plugins").is_dir() || content_root.join("patchers").is_dir() {
        return true;
    }
    // Single plugin folder with DLLs, or loose DLLs at root
    let Ok(entries) = std::fs::read_dir(content_root) else {
        return false;
    };
    let mut has_dll = false;
    let mut only_dirs_and_dlls = true;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            if name.ends_with(".dll") {
                has_dll = true;
            } else if name.ends_with(".json")
                || name.ends_with(".md")
                || name.ends_with(".txt")
                || name == "manifest.json"
                || name == "icon.png"
                || name == "readme.md"
            {
                // Thunderstore metadata — ignore
            } else {
                only_dirs_and_dlls = false;
            }
        } else if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            // Nested plugin folder
            if dir_contains_dll(&path) {
                has_dll = true;
            }
        }
    }
    has_dll && only_dirs_and_dlls
}

fn dir_contains_dll(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.ends_with(".dll") {
            return true;
        }
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) && dir_contains_dll(&entry.path())
        {
            return true;
        }
    }
    false
}

/// True when content is already laid out as `BepInEx/<plugins|patchers|config|...>`.
/// Such trees merge into the game's BepInEx root; wrapping them would bury a
/// patcher at `BepInEx/plugins/<mod>/BepInEx/patchers/...` where nothing loads it.
pub fn looks_like_bepinex_plugin_tree(content_root: &Path) -> bool {
    let root = content_root.join("BepInEx");
    if !root.is_dir() {
        return false;
    }
    BEPINEX_SUBDIRS.iter().any(|name| root.join(name).is_dir())
}

/// Root-level Thunderstore metadata that must not be linked next to a merged tree.
fn is_thunderstore_metadata_root_file(relative: &Path) -> bool {
    let mut parts = relative.components();
    let Some(Component::Normal(first)) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    let name = first.to_string_lossy().to_lowercase();
    THUNDERSTORE_METADATA.contains(&name.as_str())
}

/// Skip Thunderstore metadata for mods that merge into the shared `BepInEx/` tree,
/// otherwise every package drops a `manifest.json` / `icon.png` into `BepInEx/plugins/`.
pub(crate) fn should_deploy_bepinex_file(content_root: Option<&Path>, relative: &Path) -> bool {
    let Some(content_root) = content_root else {
        return true;
    };
    if should_wrap_bepinex_mod(content_root) {
        return true;
    }
    !is_thunderstore_metadata_root_file(relative)
}

pub(crate) fn should_wrap_bepinex_mod(content_root: &Path) -> bool {
    // Thunderstore layouts that already use plugins/ or patchers/ must map via
    // resolve_bepinex_deploy (BepInEx/plugins/...) — wrapping would nest them as
    // BepInEx/plugins/<mod>/plugins/...
    if BEPINEX_SUBDIRS
        .iter()
        .any(|name| content_root.join(name).is_dir())
    {
        return false;
    }
    !looks_like_bepinex_pack(content_root)
        && looks_like_bepinex_plugin(content_root)
        && !looks_like_bepinex_plugin_tree(content_root)
}

fn dir_has_entries(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

fn doorstop_dll_at_root(install_path: &Path) -> bool {
    DOORSTOP_FILES
        .iter()
        .any(|name| name.ends_with(".dll") && install_path.join(name).is_file())
}

fn nested_doorstop_dir(install_path: &Path) -> Option<PathBuf> {
    let Ok(entries) = std::fs::read_dir(install_path) else {
        return None;
    };
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let path = entry.path();
        if DOORSTOP_FILES
            .iter()
            .any(|name| name.ends_with(".dll") && path.join(name).is_file())
        {
            return Some(path);
        }
    }
    None
}

fn doorstop_config_path(install_path: &Path) -> Option<PathBuf> {
    let root = install_path.join("doorstop_config.ini");
    if root.is_file() {
        return Some(root);
    }
    nested_doorstop_dir(install_path).map(|dir| dir.join("doorstop_config.ini"))
}

fn doorstop_target_assembly_warning(install_path: &Path, ini_path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(ini_path).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if !line.starts_with("target_assembly") {
            continue;
        }
        let target = line.split('=').nth(1)?.trim();
        if target.is_empty() {
            continue;
        }
        let relative = PathBuf::from(target.replace('\\', "/"));
        let resolved = install_path.join(relative);
        if !resolved.is_file() {
            return Some(format!(
                "doorstop_config.ini points to {} but that file was not found. Purge and redeploy the BepInEx pack.",
                resolved.display()
            ));
        }
    }
    None
}

/// BepInEx.GUI replaces the log console, but only when BepInEx is configured to
/// show one. A mod set that ships the GUI patcher with `Enabled = false` starts
/// with no visible console at all, which reads as "mods did not load".
fn gui_patcher_deployed(bepinex_root: &Path) -> bool {
    let patchers = bepinex_root.join("patchers");
    if !patchers.is_dir() {
        return false;
    }
    walkdir::WalkDir::new(&patchers)
        .max_depth(3)
        .into_iter()
        .filter_map(|e| e.ok())
        .any(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            name == "bepinex_gui.exe"
                || name == "bepinex.gui.loader.dll"
                || name == "bepinex.gui.patcher.dll"
        })
}

const CONSOLE_SECTION: &str = "[Logging.Console]";

fn console_enabled_config(existing: Option<&str>) -> Option<String> {
    let Some(existing) = existing else {
        // BepInEx merges its defaults into whatever is already here on startup.
        return Some(format!("{CONSOLE_SECTION}\nEnabled = true\n"));
    };
    let mut out = String::with_capacity(existing.len() + CONSOLE_SECTION.len() + 16);
    let mut in_section = false;
    let mut rewrote = false;
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed.eq_ignore_ascii_case(CONSOLE_SECTION);
        } else if in_section && trimmed.starts_with("Enabled") {
            if trimmed.split('=').nth(1).map(str::trim) == Some("true") {
                return None;
            }
            out.push_str("Enabled = true\n");
            rewrote = true;
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    if !rewrote {
        out.push_str(&format!("\n{CONSOLE_SECTION}\nEnabled = true\n"));
    }
    Some(out)
}

/// Turn the BepInEx console on when a GUI console patcher is installed.
pub fn enable_bepinex_console(install_path: &Path) -> Vec<String> {
    let Some(root) = detect_bepinex_root(install_path) else {
        return Vec::new();
    };
    if !gui_patcher_deployed(&root) {
        return Vec::new();
    }
    let config = root.join("config").join("BepInEx.cfg");
    let existing = std::fs::read_to_string(&config).ok();
    let Some(updated) = console_enabled_config(existing.as_deref()) else {
        return Vec::new();
    };
    // Write through a temp file: the deployed config may be a hardlink to the
    // staged pack, and editing in place would rewrite the staged copy too.
    let tmp = config.with_extension("cfg.emperor-tmp");
    let write = std::fs::create_dir_all(config.parent().unwrap_or(&root))
        .and_then(|()| std::fs::write(&tmp, updated))
        .and_then(|()| std::fs::rename(&tmp, &config));
    if let Err(e) = write {
        let _ = std::fs::remove_file(&tmp);
        return vec![format!(
            "Could not enable the BepInEx console in {}: {e}",
            config.display()
        )];
    }
    Vec::new()
}

/// Loader-owned directories whose empty leftovers are safe to remove on purge.
pub fn bepinex_prunable_dirs(install_path: &Path) -> Vec<PathBuf> {
    let root = detect_bepinex_root(install_path).unwrap_or_else(|| install_path.join("BepInEx"));
    ["plugins", "patchers", "monomod"]
        .iter()
        .map(|name| root.join(name))
        .filter(|dir| dir.is_dir())
        .collect()
}

pub fn bepinex_preflight_warnings(install_path: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    if !bepinex_present(install_path) {
        warnings.push(
            "BepInEx not found in the game folder. Mods will not load until BepInEx (or a BepInExPack) is installed."
                .into(),
        );
    }
    if let Some(root) = detect_bepinex_root(install_path) {
        let default_root = install_path.join("BepInEx");
        let stale_plugins = default_root.join("plugins");
        if root != default_root && stale_plugins.is_dir() && dir_has_entries(&stale_plugins) {
            warnings.push(format!(
                "Mods appear under {} but BepInEx loads from {}. Purge and redeploy.",
                stale_plugins.display(),
                root.join("plugins").display(),
            ));
        }
        if root == default_root {
            let nested_plugins = install_path
                .join("BepInExPack")
                .join("BepInEx")
                .join("plugins");
            if nested_plugins.is_dir() && dir_has_entries(&nested_plugins) {
                warnings.push(format!(
                    "Mods appear under {} but BepInEx loads from {}. Purge and redeploy.",
                    nested_plugins.display(),
                    root.join("plugins").display(),
                ));
            }
        }
    }
    if !doorstop_dll_at_root(install_path) {
        if let Some(nested) = nested_doorstop_dir(install_path) {
            warnings.push(format!(
                "Doorstop injector is under {} but must be next to the game executable. Purge and redeploy the BepInEx pack.",
                nested.display()
            ));
        }
    }
    if let Some(ini_path) = doorstop_config_path(install_path) {
        if ini_path.parent() != Some(install_path) {
            warnings.push(
                "doorstop_config.ini must be next to the game executable. Purge and redeploy the BepInEx pack."
                    .into(),
            );
        }
        if let Some(msg) = doorstop_target_assembly_warning(install_path, &ini_path) {
            warnings.push(msg);
        }
    }
    match detect_unity_runtime(install_path) {
        UnityRuntime::Il2Cpp => warnings.push(
            "IL2CPP Unity runtime detected. Ensure you installed the IL2CPP BepInEx pack for this game."
                .into(),
        ),
        UnityRuntime::Mono | UnityRuntime::Unknown => {}
    }
    warnings
}

pub fn bepinex_log_sources(install_path: &Path) -> Vec<super::LogSource> {
    let root = detect_bepinex_root(install_path).unwrap_or_else(|| install_path.join("BepInEx"));
    let mut sources = vec![super::LogSource {
        label: "BepInEx".into(),
        kind: super::LogKind::BepInEx,
        path: root.join("LogOutput.log"),
    }];
    let error_log = root.join("ErrorLog.log");
    if error_log.is_file() {
        sources.push(super::LogSource {
            label: "BepInEx errors".into(),
            kind: super::LogKind::BepInEx,
            path: error_log,
        });
    }
    sources.extend(super::log_sources::proton_player_logs(install_path));
    sources
}

pub fn resolve_bepinex_deploy(install_path: &Path, relative: &Path) -> Result<PathBuf> {
    let normalized = normalize_relative(relative);
    let parts: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect();

    let bepinex_root =
        detect_bepinex_root(install_path).unwrap_or_else(|| install_path.join("BepInEx"));
    let plugins_dir = bepinex_plugins_dir(install_path);

    if parts.is_empty() {
        return Ok(plugins_dir);
    }

    let first = parts[0].to_string_lossy();
    let first_lower = first.to_lowercase();

    // Already rooted at BepInEx/...
    if first_lower == "bepinex" {
        let mut out = bepinex_root;
        for p in parts.iter().skip(1) {
            out.push(p);
        }
        return Ok(out);
    }

    // Doorstop / pack files at install root
    if DOORSTOP_FILES
        .iter()
        .any(|n| first_lower == n.to_lowercase())
        || first_lower == "doorstop_libs"
        || first_lower == "dotnet"
        || first_lower.starts_with("bepinexpack")
    {
        let mut out = install_path.to_path_buf();
        for p in &parts {
            out.push(p);
        }
        return Ok(out);
    }

    // plugins/, patchers/, config/, monomod/ without BepInEx prefix
    if BEPINEX_SUBDIRS.contains(&first_lower.as_str()) {
        let mut out = bepinex_root;
        for p in &parts {
            out.push(p);
        }
        return Ok(out);
    }

    // Default: <detected>/plugins/<relative>
    let mut out = plugins_dir;
    for p in &parts {
        out.push(p);
    }
    Ok(out)
}

macro_rules! bepinex_title_plugin {
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
                BEPINEX_PRESERVE_ROOTS
            }

            fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
                should_wrap_bepinex_mod(content_root)
            }

            fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
                content_folder_wrap_name(content_root, staged_name)
            }

            fn deploys_to_install_root(&self, content_root: &Path) -> bool {
                looks_like_bepinex_pack(content_root)
            }

            fn should_deploy_file_ctx(
                &self,
                relative: &Path,
                ctx: &crate::games::DeployContext<'_>,
            ) -> bool {
                should_deploy_bepinex_file(ctx.content_root, relative)
            }

            fn prunable_dirs(&self, install_path: &Path) -> Vec<PathBuf> {
                bepinex_prunable_dirs(install_path)
            }

            fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
                bepinex_preflight_warnings(install_path)
            }

            fn load_log_sources(&self, install_path: &Path) -> Vec<super::LogSource> {
                bepinex_log_sources(install_path)
            }

            fn staging_deploy_warnings(&self, content_root: &Path, mod_name: &str) -> Vec<String> {
                if looks_like_bepinex_pack(content_root) {
                    vec![format!(
                        "{mod_name} looks like a BepInEx pack; files were deployed to the game root."
                    )]
                } else {
                    Vec::new()
                }
            }

            fn resolve_deploy_root(
                &self,
                install_path: &Path,
                relative: &Path,
            ) -> Result<PathBuf> {
                resolve_bepinex_deploy(install_path, relative)
            }

            fn bepinex_loader(&self, install_path: &Path) -> Option<BepInExBuild> {
                select_bepinex_build($id, detect_unity_runtime(install_path))
            }

            fn after_deploy(
                &self,
                install_path: &Path,
                _enabled_mod_folders: &[String],
            ) -> Result<Vec<String>> {
                Ok(enable_bepinex_console(install_path))
            }
        }
    };
}

bepinex_title_plugin!(
    LethalCompanyPlugin,
    "lethalcompany",
    "Lethal Company",
    "lethalcompany",
    &["lethal company", "lethalcompany"]
);
bepinex_title_plugin!(ValheimPlugin, "valheim", "Valheim", "valheim", &["valheim"]);
bepinex_title_plugin!(
    RiskOfRain2Plugin,
    "riskofrain2",
    "Risk of Rain 2",
    "riskofrain2",
    &["risk of rain 2", "riskofrain2", "ror2"]
);
bepinex_title_plugin!(
    AmongUsPlugin,
    "amongus",
    "Among Us",
    "amongus",
    &["among us", "amongus"]
);
bepinex_title_plugin!(
    ContentWarningPlugin,
    "contentwarning",
    "Content Warning",
    "contentwarning",
    &["content warning", "contentwarning"]
);
bepinex_title_plugin!(GtfoPlugin, "gtfo", "GTFO", "gtfo", &["gtfo"]);
bepinex_title_plugin!(
    TimberbornPlugin,
    "timberborn",
    "Timberborn",
    "timberborn",
    &["timberborn"]
);
bepinex_title_plugin!(
    CultOfTheLambPlugin,
    "cultofthelamb",
    "Cult of the Lamb",
    "cultofthelamb",
    &["cult of the lamb", "cultofthelamb"]
);
bepinex_title_plugin!(
    AgainstTheStormPlugin,
    "againstthestorm",
    "Against the Storm",
    "againstthestorm",
    &["against the storm", "againstthestorm"]
);
bepinex_title_plugin!(RoundsPlugin, "rounds", "ROUNDS", "rounds", &["rounds"]);
bepinex_title_plugin!(
    SonsOfTheForestPlugin,
    "sonsoftheforest",
    "Sons of the Forest",
    "sonsoftheforest",
    &["sons of the forest", "sonsoftheforest"]
);
bepinex_title_plugin!(
    SubnauticaBelowZeroPlugin,
    "subnauticabelowzero",
    "Subnautica: Below Zero",
    "subnauticabelowzero",
    &["below zero", "subnauticabelowzero"]
);
bepinex_title_plugin!(
    SubnauticaPlugin,
    "subnautica",
    "Subnautica",
    "subnautica",
    &["subnautica"]
);
bepinex_title_plugin!(
    Schedule1Plugin,
    "schedule1",
    "Schedule I",
    "schedule1",
    &["schedule 1", "schedule i", "schedule1"]
);
bepinex_title_plugin!(
    RepoPlugin,
    "repo",
    "R.E.P.O.",
    "repo",
    &["r.e.p.o.", "r.e.p.o", "repo"]
);
bepinex_title_plugin!(PeakPlugin, "peak", "PEAK", "peak", &["peak"]);
bepinex_title_plugin!(
    H3vrPlugin,
    "h3vr",
    "H3VR",
    "h3vr",
    &[
        "hot dogs, horseshoes and hand grenades",
        "hot dogs, horseshoes",
        "hot dogs horseshoes",
        "h3vr",
    ]
);
bepinex_title_plugin!(
    UltrakillPlugin,
    "ultrakill",
    "ULTRAKILL",
    "ultrakill",
    &["ultrakill"]
);
bepinex_title_plugin!(AtlyssPlugin, "atlyss", "ATLYSS", "atlyss", &["atlyss"]);
bepinex_title_plugin!(
    DysonSphereProgramPlugin,
    "dysonsphereprogram",
    "Dyson Sphere Program",
    "dysonsphereprogram",
    &["dyson sphere program", "dysonsphereprogram"]
);
bepinex_title_plugin!(
    InscryptionPlugin,
    "inscryption",
    "Inscryption",
    "inscryption",
    &["inscryption"]
);
bepinex_title_plugin!(
    HollowKnightSilksongPlugin,
    "hollowknightsilksong",
    "Hollow Knight: Silksong",
    "hollowknightsilksong",
    &[
        "silksong",
        "hollow knight: silksong",
        "hollowknightsilksong"
    ]
);

/// Chef RPG. Plugin DLLs use the shared BepInEx layout. A staff download
/// (`UnassignedStaffData.dat` and no DLL) is a save overwrite, not a plugin.
pub struct ChefRpgPlugin;

impl GamePlugin for ChefRpgPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "chefrpg",
            display_name: "Chef RPG",
            nexus_domain: "chefrpg",
            match_names: &["chef rpg", "chefrpg"],
        }
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        BepInExPlugin.preserve_staging_root_names()
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        if looks_like_chefrpg_save_pack(content_root) {
            return false;
        }
        BepInExPlugin.should_wrap_as_mod_folder(content_root)
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        BepInExPlugin.wrap_mod_folder_name(content_root, staged_name)
    }

    fn deploys_to_install_root(&self, content_root: &Path) -> bool {
        if looks_like_chefrpg_save_pack(content_root) {
            return false;
        }
        BepInExPlugin.deploys_to_install_root(content_root)
    }

    fn should_deploy_file_ctx(
        &self,
        relative: &Path,
        ctx: &crate::games::DeployContext<'_>,
    ) -> bool {
        if ctx.content_root.is_some_and(looks_like_chefrpg_save_pack) {
            return false;
        }
        BepInExPlugin.should_deploy_file_ctx(relative, ctx)
    }

    fn allows_empty_deploy(&self, content_root: &Path) -> bool {
        looks_like_chefrpg_save_pack(content_root)
    }

    fn prunable_dirs(&self, install_path: &Path) -> Vec<PathBuf> {
        BepInExPlugin.prunable_dirs(install_path)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        BepInExPlugin.preflight_warnings(install_path)
    }

    fn load_log_sources(&self, install_path: &Path) -> Vec<super::LogSource> {
        BepInExPlugin.load_log_sources(install_path)
    }

    fn staging_deploy_warnings(&self, content_root: &Path, mod_name: &str) -> Vec<String> {
        BepInExPlugin.staging_deploy_warnings(content_root, mod_name)
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        BepInExPlugin.resolve_deploy_root(install_path, relative)
    }

    fn bepinex_loader(&self, _install_path: &Path) -> Option<BepInExBuild> {
        Some(mono_github_build())
    }

    fn deploy_staged_extra(
        &self,
        install_path: &Path,
        content_root: &Path,
        _mod_name: &str,
    ) -> Result<Vec<String>> {
        Ok(install_chefrpg_save_mods(install_path, &[content_root]))
    }

    fn after_deploy(
        &self,
        install_path: &Path,
        enabled_mod_folders: &[String],
    ) -> Result<Vec<String>> {
        BepInExPlugin.after_deploy(install_path, enabled_mod_folders)
    }
}

/// Generic BepInEx plugin for any detected Unity install.
pub struct BepInExPlugin;

impl GamePlugin for BepInExPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "bepinex",
            display_name: "Unity / BepInEx (generic)",
            nexus_domain: "",
            match_names: &[],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        BEPINEX_PRESERVE_ROOTS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        should_wrap_bepinex_mod(content_root)
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn deploys_to_install_root(&self, content_root: &Path) -> bool {
        looks_like_bepinex_pack(content_root)
    }

    fn should_deploy_file_ctx(
        &self,
        relative: &Path,
        ctx: &crate::games::DeployContext<'_>,
    ) -> bool {
        should_deploy_bepinex_file(ctx.content_root, relative)
    }

    fn prunable_dirs(&self, install_path: &Path) -> Vec<PathBuf> {
        bepinex_prunable_dirs(install_path)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        bepinex_preflight_warnings(install_path)
    }

    fn load_log_sources(&self, install_path: &Path) -> Vec<super::LogSource> {
        bepinex_log_sources(install_path)
    }

    fn staging_deploy_warnings(&self, content_root: &Path, mod_name: &str) -> Vec<String> {
        if looks_like_bepinex_pack(content_root) {
            vec![format!(
                "{mod_name} looks like a BepInEx pack; files were deployed to the game root."
            )]
        } else {
            Vec::new()
        }
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        resolve_bepinex_deploy(install_path, relative)
    }

    fn preferred_plugin_id(&self, install_path: &Path) -> &'static str {
        let dressed = super::plugin_id_for_install(self.info().id, install_path);
        specialize_bepinex_plugin_id(dressed, install_path)
    }

    fn bepinex_loader(&self, install_path: &Path) -> Option<BepInExBuild> {
        select_bepinex_build(self.info().id, detect_unity_runtime(install_path))
    }

    fn after_deploy(
        &self,
        install_path: &Path,
        _enabled_mod_folders: &[String],
    ) -> Result<Vec<String>> {
        Ok(enable_bepinex_console(install_path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::GamePlugin;
    use std::fs;

    #[test]
    fn plugin_dll_wraps_under_plugins() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("CoolMod.dll"), b"x").unwrap();
        let plugin = BepInExPlugin;
        assert!(plugin.should_wrap_as_mod_folder(tmp.path()));
        assert!(!plugin.deploys_to_install_root(tmp.path()));
        let dest = plugin
            .resolve_deploy_root(Path::new("/game"), Path::new("CoolMod.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/BepInEx/plugins/CoolMod.dll"));
    }

    #[test]
    fn bepinex_tree_deploys_to_root() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("BepInEx").join("core")).unwrap();
        fs::write(tmp.path().join("winhttp.dll"), b"x").unwrap();
        let plugin = BepInExPlugin;
        assert!(plugin.deploys_to_install_root(tmp.path()));
        let dest = plugin
            .resolve_deploy_root(Path::new("/game"), Path::new("BepInEx/plugins/x.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/BepInEx/plugins/x.dll"));
    }

    #[test]
    fn patcher_tree_merges_into_bepinex_root() {
        let tmp = tempfile::tempdir().unwrap();
        let gui = tmp
            .path()
            .join("BepInEx")
            .join("patchers")
            .join("BepInEx.GUI");
        fs::create_dir_all(&gui).unwrap();
        fs::write(gui.join("BepInEx.GUI.Loader.dll"), b"x").unwrap();
        fs::write(tmp.path().join("manifest.json"), b"{}").unwrap();

        let plugin = RiskOfRain2Plugin;
        // Wrapping would bury the patcher at BepInEx/plugins/<mod>/BepInEx/patchers.
        assert!(!plugin.should_wrap_as_mod_folder(tmp.path()));
        assert!(!plugin.deploys_to_install_root(tmp.path()));
        let dest = plugin
            .resolve_deploy_root(
                Path::new("/game"),
                Path::new("BepInEx/patchers/BepInEx.GUI/BepInEx.GUI.Loader.dll"),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/BepInEx/patchers/BepInEx.GUI/BepInEx.GUI.Loader.dll")
        );

        let ctx = crate::games::DeployContext {
            content_root: Some(tmp.path()),
            ..Default::default()
        };
        assert!(!plugin.should_deploy_file_ctx(Path::new("manifest.json"), &ctx));
        assert!(plugin.should_deploy_file_ctx(
            Path::new("BepInEx/patchers/BepInEx.GUI/BepInEx.GUI.Loader.dll"),
            &ctx
        ));
    }

    #[test]
    fn wrapped_mods_keep_their_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("CoolMod.dll"), b"x").unwrap();
        fs::write(tmp.path().join("manifest.json"), b"{}").unwrap();
        let ctx = crate::games::DeployContext {
            content_root: Some(tmp.path()),
            ..Default::default()
        };
        // BepInEx.GUI shows the Thunderstore manifest that sits next to a plugin.
        assert!(RiskOfRain2Plugin.should_deploy_file_ctx(Path::new("manifest.json"), &ctx));
    }

    #[test]
    fn monomod_folder_maps_into_bepinex() {
        let dest = resolve_bepinex_deploy(Path::new("/game"), Path::new("monomod/Assembly.mm.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/BepInEx/monomod/Assembly.mm.dll"));
    }

    #[test]
    fn console_config_turns_the_console_on() {
        let existing = "[Logging.Console]\n\n## Enables showing a console.\nEnabled = false\n\n[Logging.Disk]\nEnabled = true\n";
        let updated = console_enabled_config(Some(existing)).unwrap();
        assert!(updated
            .contains("[Logging.Console]\n\n## Enables showing a console.\nEnabled = true\n"));
        // The disk logging section keeps its own value.
        assert!(updated.ends_with("[Logging.Disk]\nEnabled = true\n"));
        // Already enabled configs are left alone.
        assert!(console_enabled_config(Some(&updated)).is_none());
        assert!(console_enabled_config(None)
            .unwrap()
            .contains("Enabled = true"));
    }

    #[test]
    fn console_is_enabled_when_the_gui_patcher_is_deployed() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("BepInEx");
        fs::create_dir_all(root.join("core")).unwrap();
        fs::create_dir_all(root.join("config")).unwrap();
        fs::write(
            root.join("config").join("BepInEx.cfg"),
            "[Logging.Console]\nEnabled = false\n",
        )
        .unwrap();
        assert!(enable_bepinex_console(tmp.path()).is_empty());
        let cfg = || fs::read_to_string(root.join("config").join("BepInEx.cfg")).unwrap();
        assert!(cfg().contains("Enabled = false"), "no GUI installed yet");

        let gui = root.join("patchers").join("BepInEx.GUI");
        fs::create_dir_all(&gui).unwrap();
        fs::write(gui.join("bepinex_gui.exe"), b"x").unwrap();
        assert!(enable_bepinex_console(tmp.path()).is_empty());
        assert!(cfg().contains("Enabled = true"));
    }

    #[test]
    fn preserves_plugins_prefix() {
        let dest = resolve_bepinex_deploy(Path::new("/game"), Path::new("plugins/MyMod/MyMod.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/BepInEx/plugins/MyMod/MyMod.dll"));
    }

    #[test]
    fn detects_unity_data_folder() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("Game_Data").join("Managed")).unwrap();
        assert!(looks_like_unity_install(tmp.path()));
        assert_eq!(detect_unity_runtime(tmp.path()), UnityRuntime::Mono);
    }

    #[test]
    fn schedule1_dll_goes_to_plugins() {
        let dest = Schedule1Plugin
            .resolve_deploy_root(Path::new("/game"), Path::new("CoolMod.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/BepInEx/plugins/CoolMod.dll"));
        assert_eq!(
            thunderstore_community_for_plugin("schedule1"),
            Some("schedule-i")
        );
        assert_eq!(
            thunderstore_community_for_plugin("sonsoftheforest"),
            Some("sons-of-the-forest")
        );
        assert_eq!(
            thunderstore_community_for_plugin("bladeandsorcery"),
            Some("blade-and-sorcery")
        );
        assert_eq!(thunderstore_community_for_plugin("repo"), Some("repo"));
        assert_eq!(
            thunderstore_community_for_plugin("hollowknightsilksong"),
            Some("hollow-knight-silksong")
        );
        assert_eq!(
            thunderstore_community_for_plugin("dysonsphereprogram"),
            Some("dyson-sphere-program")
        );
        assert_eq!(
            thunderstore_community_for_plugin("bonelab"),
            Some("bonelab")
        );
        assert_eq!(
            thunderstore_community_for_plugin("boneworks"),
            Some("boneworks")
        );
    }

    #[test]
    fn detect_bepinexpack_nested_root() {
        let install = tempfile::tempdir().unwrap();
        fs::create_dir_all(
            install
                .path()
                .join("BepInExPack")
                .join("BepInEx")
                .join("core"),
        )
        .unwrap();
        assert_eq!(
            detect_bepinex_root(install.path()),
            Some(install.path().join("BepInExPack").join("BepInEx"))
        );
    }

    #[test]
    fn resolve_plugins_under_bepinexpack() {
        let install = tempfile::tempdir().unwrap();
        fs::create_dir_all(
            install
                .path()
                .join("BepInExPack")
                .join("BepInEx")
                .join("core"),
        )
        .unwrap();
        let dest = resolve_bepinex_deploy(install.path(), Path::new("CoolMod.dll")).unwrap();
        assert_eq!(
            dest,
            install
                .path()
                .join("BepInExPack")
                .join("BepInEx")
                .join("plugins")
                .join("CoolMod.dll")
        );
    }

    #[test]
    fn resolve_bepinex_prefixed_path_under_bepinexpack() {
        let install = tempfile::tempdir().unwrap();
        fs::create_dir_all(
            install
                .path()
                .join("BepInExPack")
                .join("BepInEx")
                .join("core"),
        )
        .unwrap();
        let dest =
            resolve_bepinex_deploy(install.path(), Path::new("BepInEx/plugins/MyMod/MyMod.dll"))
                .unwrap();
        assert_eq!(
            dest,
            install
                .path()
                .join("BepInExPack")
                .join("BepInEx")
                .join("plugins")
                .join("MyMod")
                .join("MyMod.dll")
        );
    }

    #[test]
    fn standard_layout_still_uses_root_bepinex() {
        let install = tempfile::tempdir().unwrap();
        fs::create_dir_all(install.path().join("BepInEx").join("core")).unwrap();
        let dest = resolve_bepinex_deploy(install.path(), Path::new("CoolMod.dll")).unwrap();
        assert_eq!(
            dest,
            install
                .path()
                .join("BepInEx")
                .join("plugins")
                .join("CoolMod.dll")
        );
    }

    #[test]
    fn plugin_tree_is_not_wrapped() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("BepInEx").join("plugins").join("MyMod")).unwrap();
        fs::write(
            tmp.path()
                .join("BepInEx")
                .join("plugins")
                .join("MyMod")
                .join("MyMod.dll"),
            b"x",
        )
        .unwrap();
        assert!(!should_wrap_bepinex_mod(tmp.path()));
    }

    #[test]
    fn loose_dll_is_wrapped() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("CoolMod.dll"), b"x").unwrap();
        assert!(should_wrap_bepinex_mod(tmp.path()));
    }

    #[test]
    fn top_level_plugins_dir_is_not_wrapped() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("plugins").join("R2API.Legacy")).unwrap();
        fs::write(
            tmp.path()
                .join("plugins")
                .join("R2API.Legacy")
                .join("R2API.dll"),
            b"x",
        )
        .unwrap();
        assert!(!should_wrap_bepinex_mod(tmp.path()));
        let dest = resolve_bepinex_deploy(
            Path::new("/game"),
            Path::new("plugins/R2API.Legacy/R2API.dll"),
        )
        .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/BepInEx/plugins/R2API.Legacy/R2API.dll")
        );
    }

    #[test]
    fn top_level_patchers_dir_is_not_wrapped() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("patchers").join("HookGen")).unwrap();
        fs::write(
            tmp.path()
                .join("patchers")
                .join("HookGen")
                .join("HookGen.dll"),
            b"x",
        )
        .unwrap();
        assert!(!should_wrap_bepinex_mod(tmp.path()));
    }

    #[test]
    fn riskofrain2_plugins_prefix_deploys_flat() {
        let dest = RiskOfRain2Plugin
            .resolve_deploy_root(
                Path::new("/ror2"),
                Path::new("plugins/HAND_Overclocked/HAND_Overclocked.dll"),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/ror2/BepInEx/plugins/HAND_Overclocked/HAND_Overclocked.dll")
        );
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("plugins").join("HAND_Overclocked")).unwrap();
        fs::write(
            tmp.path()
                .join("plugins")
                .join("HAND_Overclocked")
                .join("HAND_Overclocked.dll"),
            b"x",
        )
        .unwrap();
        assert!(!RiskOfRain2Plugin.should_wrap_as_mod_folder(tmp.path()));
    }

    #[test]
    fn plugin_tree_is_not_a_pack() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("BepInEx").join("plugins").join("MyMod")).unwrap();
        fs::write(
            tmp.path()
                .join("BepInEx")
                .join("plugins")
                .join("MyMod")
                .join("MyMod.dll"),
            b"x",
        )
        .unwrap();
        assert!(!looks_like_bepinex_pack(tmp.path()));
    }

    #[test]
    fn plugin_tree_does_not_deploy_to_install_root() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("BepInEx").join("plugins").join("MyMod")).unwrap();
        fs::write(
            tmp.path()
                .join("BepInEx")
                .join("plugins")
                .join("MyMod")
                .join("MyMod.dll"),
            b"x",
        )
        .unwrap();
        let plugin = BepInExPlugin;
        assert!(!plugin.deploys_to_install_root(tmp.path()));
    }

    #[test]
    fn pack_with_core_is_detected() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("BepInEx").join("core")).unwrap();
        fs::write(tmp.path().join("winhttp.dll"), b"x").unwrap();
        assert!(looks_like_bepinex_pack(tmp.path()));
    }

    #[test]
    fn bepinex_pack_deploy_root_flattens_wrapper() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("BepInExPack").join("BepInEx").join("core")).unwrap();
        fs::write(tmp.path().join("BepInExPack").join("winhttp.dll"), b"x").unwrap();
        fs::write(tmp.path().join("manifest.json"), b"{}").unwrap();
        assert_eq!(
            bepinex_pack_deploy_root(tmp.path()),
            tmp.path().join("BepInExPack")
        );
    }

    #[test]
    fn bepinex_pack_deploy_root_keeps_flat_pack() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir_all(tmp.path().join("BepInEx").join("core")).unwrap();
        fs::write(tmp.path().join("winhttp.dll"), b"x").unwrap();
        assert_eq!(bepinex_pack_deploy_root(tmp.path()), tmp.path());
    }

    #[test]
    fn preflight_warns_when_doorstop_is_nested() {
        let install = tempfile::tempdir().unwrap();
        fs::create_dir_all(
            install
                .path()
                .join("BepInExPack")
                .join("BepInEx")
                .join("core"),
        )
        .unwrap();
        fs::write(install.path().join("BepInExPack").join("winhttp.dll"), b"x").unwrap();
        fs::write(
            install
                .path()
                .join("BepInExPack")
                .join("doorstop_config.ini"),
            "target_assembly = BepInEx\\core\\Missing.dll\n",
        )
        .unwrap();
        let warnings = bepinex_preflight_warnings(install.path());
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("Doorstop injector is under")),
            "{warnings:?}"
        );
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("doorstop_config.ini must be next to the game executable")),
            "{warnings:?}"
        );
    }

    #[test]
    fn selects_thunderstore_pack_or_pinned_github_build() {
        assert!(matches!(
            select_bepinex_build("chefrpg", UnityRuntime::Mono),
            Some(BepInExBuild::Github { asset, .. }) if asset == "BepInEx_win_x64_5.4.23.5.zip"
        ));
        assert!(matches!(
            select_bepinex_build("lethalcompany", UnityRuntime::Il2Cpp),
            Some(BepInExBuild::Thunderstore { namespace, name, .. })
                if namespace == "BepInEx" && name == "BepInExPack"
        ));
        assert!(matches!(
            select_bepinex_build("valheim", UnityRuntime::Mono),
            Some(BepInExBuild::Thunderstore { namespace, name, .. })
                if namespace == "denikson" && name == "BepInExPack_Valheim"
        ));
        assert!(matches!(
            select_bepinex_build("bepinex", UnityRuntime::Il2Cpp),
            Some(BepInExBuild::Github { asset, il2cpp: true, .. })
                if asset == "BepInEx-Unity.IL2CPP-win-x64-6.0.0-pre.2.zip"
        ));
        assert!(matches!(
            select_bepinex_build("bepinex", UnityRuntime::Mono),
            Some(BepInExBuild::Github { asset, il2cpp: false, .. })
                if asset == "BepInEx_win_x64_5.4.23.5.zip"
        ));
        assert!(select_bepinex_build("schedule1", UnityRuntime::Il2Cpp).is_none());
        assert!(select_bepinex_build("eldenring", UnityRuntime::Mono).is_none());

        assert!(matches!(
            ChefRpgPlugin.bepinex_loader(Path::new("/unused")),
            Some(BepInExBuild::Github { asset, il2cpp: false, .. })
                if asset == "BepInEx_win_x64_5.4.23.5.zip"
        ));
        let il2cpp = tempfile::tempdir().unwrap();
        fs::write(il2cpp.path().join("GameAssembly.dll"), b"x").unwrap();
        assert!(matches!(
            BepInExPlugin.bepinex_loader(il2cpp.path()),
            Some(BepInExBuild::Github { asset, il2cpp: true, .. })
                if asset == "BepInEx-Unity.IL2CPP-win-x64-6.0.0-pre.2.zip"
        ));
        assert!(matches!(
            LethalCompanyPlugin.bepinex_loader(il2cpp.path()),
            Some(BepInExBuild::Thunderstore { namespace, name, .. })
                if namespace == "BepInEx" && name == "BepInExPack"
        ));
        let mono = tempfile::tempdir().unwrap();
        fs::create_dir_all(mono.path().join("Game_Data").join("Managed")).unwrap();
        assert!(matches!(
            BepInExPlugin.bepinex_loader(mono.path()),
            Some(BepInExBuild::Github { asset, il2cpp: false, .. })
                if asset == "BepInEx_win_x64_5.4.23.5.zip"
        ));
        assert!(Schedule1Plugin.bepinex_loader(il2cpp.path()).is_none());
    }

    #[test]
    fn skips_auto_install_when_loader_or_pack_is_already_present() {
        let install = tempfile::tempdir().unwrap();
        let dll = tempfile::tempdir().unwrap();
        fs::write(dll.path().join("CoolMod.dll"), b"x").unwrap();
        let roots = [dll.path()];
        let chef = ChefRpgPlugin;
        let wants = |plugin: &dyn GamePlugin, install: &Path, roots: &[&Path]| {
            should_auto_install_bepinex(install, roots, plugin.bepinex_loader(install), |root| {
                plugin.allows_empty_deploy(root)
            })
        };
        assert!(wants(&chef, install.path(), &roots));

        fs::write(install.path().join("winhttp.dll"), b"x").unwrap();
        assert!(!wants(&chef, install.path(), &roots));

        let fresh = tempfile::tempdir().unwrap();
        let pack = tempfile::tempdir().unwrap();
        fs::create_dir_all(pack.path().join("BepInEx").join("core")).unwrap();
        fs::write(pack.path().join("winhttp.dll"), b"x").unwrap();
        assert!(!wants(&chef, fresh.path(), &[pack.path()]));

        let save_only = tempfile::tempdir().unwrap();
        fs::write(save_only.path().join("UnassignedStaffData.dat"), b"staff").unwrap();
        assert!(!wants(&chef, fresh.path(), &[save_only.path()]));
        assert!(!wants(&Schedule1Plugin, fresh.path(), &roots));
    }

    #[tokio::test]
    async fn prepare_runtime_skips_download_when_bepinex_is_present() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("game");
        fs::create_dir_all(install.join("BepInEx").join("core")).unwrap();
        fs::write(install.join("winhttp.dll"), b"x").unwrap();
        let dll = tempfile::tempdir().unwrap();
        fs::write(dll.path().join("CoolMod.dll"), b"x").unwrap();
        let roots = [dll.path()];
        let paths = crate::config::Paths {
            config_dir: tmp.path().join("config"),
            data_dir: tmp.path().join("data"),
            cache_dir: tmp.path().join("cache"),
        };
        let ctx = crate::games::RuntimePrep {
            paths: &paths,
            install_path: &install,
            nexus: None,
            premium: false,
            content_roots: &roots,
        };
        let outcome = BepInExPlugin.prepare_runtime(&ctx).await.unwrap();
        match outcome {
            crate::games::RuntimePrepOutcome::Done(warnings) => {
                assert!(warnings.is_empty(), "{warnings:?}");
            }
            crate::games::RuntimePrepOutcome::NeedsAssist { .. } => {
                panic!("BepInEx install does not use Download Assist");
            }
        }
    }

    #[test]
    fn save_pack_lands_in_the_gameplay_slot_not_plugins() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("steamapps").join("common").join("Chef RPG");
        fs::create_dir_all(&install).unwrap();
        fs::write(install.join("Chef RPG.exe"), b"x").unwrap();
        let slot1 = tmp
            .path()
            .join("steamapps")
            .join("compatdata")
            .join("1796790")
            .join("pfx")
            .join("drive_c")
            .join("users")
            .join("steamuser")
            .join("AppData")
            .join("LocalLow")
            .join("World 2 Studio Inc")
            .join("Chef RPG")
            .join("FullData")
            .join("SaveSlot1");
        let slot0 = slot1.parent().unwrap().join("SaveSlot0");
        fs::create_dir_all(&slot1).unwrap();
        fs::create_dir_all(&slot0).unwrap();
        fs::write(slot1.join("GameData.dat"), b"game").unwrap();
        fs::write(slot1.join("UnassignedStaffData.dat"), b"old").unwrap();
        fs::write(slot0.join("SoundSettingsData.dat"), b"settings").unwrap();

        let staged = tempfile::tempdir().unwrap();
        fs::write(staged.path().join("UnassignedStaffData.dat"), b"new").unwrap();
        fs::write(staged.path().join("Read This.txt"), b"readme").unwrap();
        fs::write(staged.path().join("portrait.jpg"), b"jpg").unwrap();

        let plugin = ChefRpgPlugin;
        assert!(looks_like_chefrpg_save_pack(staged.path()));
        assert!(plugin.allows_empty_deploy(staged.path()));
        assert!(!plugin.should_deploy_file_ctx(
            Path::new("UnassignedStaffData.dat"),
            &super::super::DeployContext {
                project_name: None,
                content_root: Some(staged.path()),
                source_dir: None,
            }
        ));
        assert_eq!(specialize_bepinex_plugin_id("bepinex", &install), "chefrpg");
        assert_eq!(BepInExPlugin.preferred_plugin_id(&install), "chefrpg");

        let dll = tempfile::tempdir().unwrap();
        fs::write(dll.path().join("StaffMod.dll"), b"dll").unwrap();
        assert!(plugin.should_wrap_as_mod_folder(dll.path()));
        assert!(plugin.should_deploy_file_ctx(
            Path::new("StaffMod.dll"),
            &super::super::DeployContext {
                project_name: None,
                content_root: Some(dll.path()),
                source_dir: None,
            }
        ));
        let dll_dest = plugin
            .resolve_deploy_root(&install, Path::new("StaffMod.dll"))
            .unwrap();
        assert_eq!(
            dll_dest,
            install.join("BepInEx").join("plugins").join("StaffMod.dll")
        );

        let warnings = plugin
            .deploy_staged_extra(&install, staged.path(), "Dream Team")
            .unwrap();
        assert!(
            warnings.iter().any(|w| w.contains("SaveSlot1")),
            "{warnings:?}"
        );
        assert_eq!(
            fs::read(slot1.join("UnassignedStaffData.dat")).unwrap(),
            b"new"
        );
        assert_eq!(
            fs::read(slot1.join("UnassignedStaffData.dat.bak")).unwrap(),
            b"old"
        );
        assert!(!install.join("BepInEx").join("plugins").exists());
        assert!(!slot0.join("UnassignedStaffData.dat").exists());
    }

    #[test]
    fn copy_bepinex_tree_flattens_pack_and_skips_metadata() {
        let cache = tempfile::tempdir().unwrap();
        let inner = cache.path().join("BepInExPack");
        fs::create_dir_all(inner.join("BepInEx").join("core")).unwrap();
        fs::write(
            inner.join("BepInEx").join("core").join("BepInEx.dll"),
            b"core",
        )
        .unwrap();
        fs::write(inner.join("winhttp.dll"), b"door").unwrap();
        fs::write(inner.join("doorstop_config.ini"), b"enabled = true\n").unwrap();
        fs::write(cache.path().join("manifest.json"), b"{}").unwrap();
        fs::write(cache.path().join("icon.png"), b"png").unwrap();

        let install = tempfile::tempdir().unwrap();
        copy_bepinex_tree(cache.path(), install.path()).unwrap();
        assert!(install
            .path()
            .join("BepInEx")
            .join("core")
            .join("BepInEx.dll")
            .is_file());
        assert!(install.path().join("winhttp.dll").is_file());
        assert!(!install.path().join("winhttp.dll").is_symlink());
        assert!(!install.path().join("manifest.json").exists());
        assert!(!install.path().join("BepInExPack").exists());
    }
}
