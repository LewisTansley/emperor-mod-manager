//! Shared deploy logic for Unity titles modded with BepInEx.
//!
//! Typical layout:
//! ```text
//! <Install>/BepInEx/core/
//! <Install>/BepInEx/plugins/
//! <Install>/BepInEx/patchers/
//! <Install>/BepInEx/config/
//! <Install>/winhttp.dll | version.dll | doorstop_libs/   # Doorstop
//! <Install>/GameAssembly.dll                             # IL2CPP
//! <Install>/*_Data/Managed/                              # Mono
//! ```

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

/// Roots that must not be peeled as archive wrappers for BepInEx mods.
pub const BEPINEX_PRESERVE_ROOTS: &[&str] = &[
    "BepInEx",
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
    "changelog.txt",
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
}

pub const BEPINEX_TITLE_SEEDS: &[BepInExTitleSeed] = &[
    BepInExTitleSeed {
        plugin_id: "lethalcompany",
        display_name: "Lethal Company",
        nexus_domain: "lethalcompany",
        thunderstore_community: "lethal-company",
        match_names: &["lethal company", "lethalcompany"],
    },
    BepInExTitleSeed {
        plugin_id: "valheim",
        display_name: "Valheim",
        nexus_domain: "valheim",
        thunderstore_community: "valheim",
        match_names: &["valheim"],
    },
    BepInExTitleSeed {
        plugin_id: "riskofrain2",
        display_name: "Risk of Rain 2",
        nexus_domain: "riskofrain2",
        thunderstore_community: "riskofrain2",
        match_names: &["risk of rain 2", "riskofrain2", "ror2"],
    },
    BepInExTitleSeed {
        plugin_id: "amongus",
        display_name: "Among Us",
        nexus_domain: "amongus",
        thunderstore_community: "among-us",
        match_names: &["among us", "amongus"],
    },
    BepInExTitleSeed {
        plugin_id: "contentwarning",
        display_name: "Content Warning",
        nexus_domain: "contentwarning",
        thunderstore_community: "content-warning",
        match_names: &["content warning", "contentwarning"],
    },
    BepInExTitleSeed {
        plugin_id: "gtfo",
        display_name: "GTFO",
        nexus_domain: "gtfo",
        thunderstore_community: "gtfo",
        match_names: &["gtfo"],
    },
    BepInExTitleSeed {
        plugin_id: "timberborn",
        display_name: "Timberborn",
        nexus_domain: "timberborn",
        thunderstore_community: "timberborn",
        match_names: &["timberborn"],
    },
    BepInExTitleSeed {
        plugin_id: "cultofthelamb",
        display_name: "Cult of the Lamb",
        nexus_domain: "cultofthelamb",
        thunderstore_community: "cult-of-the-lamb",
        match_names: &["cult of the lamb", "cultofthelamb"],
    },
    BepInExTitleSeed {
        plugin_id: "againstthestorm",
        display_name: "Against the Storm",
        nexus_domain: "againstthestorm",
        thunderstore_community: "against-the-storm",
        match_names: &["against the storm", "againstthestorm"],
    },
    BepInExTitleSeed {
        plugin_id: "rounds",
        display_name: "ROUNDS",
        nexus_domain: "rounds",
        thunderstore_community: "rounds",
        match_names: &["rounds"],
    },
    BepInExTitleSeed {
        plugin_id: "sonsoftheforest",
        display_name: "Sons of the Forest",
        nexus_domain: "sonsoftheforest",
        thunderstore_community: "sons-of-the-forest",
        match_names: &["sons of the forest", "sonsoftheforest"],
    },
    BepInExTitleSeed {
        plugin_id: "subnauticabelowzero",
        display_name: "Subnautica: Below Zero",
        nexus_domain: "subnauticabelowzero",
        thunderstore_community: "subnautica-below-zero",
        match_names: &["below zero", "subnauticabelowzero"],
    },
    BepInExTitleSeed {
        plugin_id: "subnautica",
        display_name: "Subnautica",
        nexus_domain: "subnautica",
        thunderstore_community: "subnautica",
        match_names: &["subnautica"],
    },
    BepInExTitleSeed {
        plugin_id: "schedule1",
        display_name: "Schedule I",
        nexus_domain: "schedule1",
        thunderstore_community: "schedule-i",
        match_names: &["schedule 1", "schedule i", "schedule1"],
    },
    BepInExTitleSeed {
        plugin_id: "repo",
        display_name: "R.E.P.O.",
        nexus_domain: "repo",
        thunderstore_community: "repo",
        match_names: &["r.e.p.o.", "r.e.p.o", "repo"],
    },
    BepInExTitleSeed {
        plugin_id: "peak",
        display_name: "PEAK",
        nexus_domain: "peak",
        thunderstore_community: "peak",
        match_names: &["peak"],
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
    },
    BepInExTitleSeed {
        plugin_id: "ultrakill",
        display_name: "ULTRAKILL",
        nexus_domain: "ultrakill",
        thunderstore_community: "ultrakill",
        match_names: &["ultrakill"],
    },
    BepInExTitleSeed {
        plugin_id: "atlyss",
        display_name: "ATLYSS",
        nexus_domain: "atlyss",
        thunderstore_community: "atlyss",
        match_names: &["atlyss"],
    },
    BepInExTitleSeed {
        plugin_id: "dysonsphereprogram",
        display_name: "Dyson Sphere Program",
        nexus_domain: "dysonsphereprogram",
        thunderstore_community: "dyson-sphere-program",
        match_names: &["dyson sphere program", "dysonsphereprogram"],
    },
    BepInExTitleSeed {
        plugin_id: "inscryption",
        display_name: "Inscryption",
        nexus_domain: "inscryption",
        thunderstore_community: "inscryption",
        match_names: &["inscryption"],
    },
    BepInExTitleSeed {
        plugin_id: "hollowknightsilksong",
        display_name: "Hollow Knight: Silksong",
        nexus_domain: "hollowknightsilksong",
        thunderstore_community: "hollow-knight-silksong",
        match_names: &["silksong", "hollow knight: silksong", "hollowknightsilksong"],
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

pub fn bepinex_present(install_path: &Path) -> bool {
    let core = install_path.join("BepInEx").join("core");
    if core.is_dir() {
        return true;
    }
    for name in DOORSTOP_FILES {
        if name.ends_with(".dll") && install_path.join(name).is_file() {
            return true;
        }
    }
    install_path.join("doorstop_libs").is_dir()
        || install_path.join(".doorstop_version").is_file()
        || install_path.join("doorstop_config.ini").is_file()
}

/// True when staged content is a full BepInEx / Doorstop pack (deploy to install root).
pub fn looks_like_bepinex_pack(content_root: &Path) -> bool {
    if content_root.join("BepInEx").is_dir() {
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
            if name.starts_with("bepinexpack") && entry.path().join("BepInEx").is_dir() {
                return true;
            }
            if entry.path().join("BepInEx").is_dir()
                && (entry.path().join("doorstop_config.ini").is_file()
                    || entry.path().join("winhttp.dll").is_file()
                    || entry.path().join("version.dll").is_file())
            {
                return true;
            }
        }
    }
    false
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

pub fn bepinex_preflight_warnings(install_path: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    if !bepinex_present(install_path) {
        warnings.push(
            "BepInEx not found in the game folder. Mods will not load until BepInEx (or a BepInExPack) is installed."
                .into(),
        );
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

pub fn resolve_bepinex_deploy(install_path: &Path, relative: &Path) -> Result<PathBuf> {
    let normalized = normalize_relative(relative);
    let parts: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect();

    if parts.is_empty() {
        return Ok(install_path.join("BepInEx").join("plugins"));
    }

    let first = parts[0].to_string_lossy();
    let first_lower = first.to_lowercase();

    // Already rooted at BepInEx/...
    if first_lower == "bepinex" {
        let mut out = install_path.to_path_buf();
        for p in &parts {
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

    // plugins/ or patchers/ without BepInEx prefix
    if first_lower == "plugins" || first_lower == "patchers" || first_lower == "config" {
        let mut out = install_path.join("BepInEx");
        for p in &parts {
            out.push(p);
        }
        return Ok(out);
    }

    // Default: BepInEx/plugins/<relative>
    let mut out = install_path.join("BepInEx").join("plugins");
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
                !looks_like_bepinex_pack(content_root) && looks_like_bepinex_plugin(content_root)
            }

            fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
                content_folder_wrap_name(content_root, staged_name)
            }

            fn deploys_to_install_root(&self, content_root: &Path) -> bool {
                looks_like_bepinex_pack(content_root)
            }

            fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
                bepinex_preflight_warnings(install_path)
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
bepinex_title_plugin!(
    ValheimPlugin,
    "valheim",
    "Valheim",
    "valheim",
    &["valheim"]
);
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
bepinex_title_plugin!(
    PeakPlugin,
    "peak",
    "PEAK",
    "peak",
    &["peak"]
);
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
bepinex_title_plugin!(
    AtlyssPlugin,
    "atlyss",
    "ATLYSS",
    "atlyss",
    &["atlyss"]
);
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
    &["silksong", "hollow knight: silksong", "hollowknightsilksong"]
);

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
        !looks_like_bepinex_pack(content_root) && looks_like_bepinex_plugin(content_root)
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn deploys_to_install_root(&self, content_root: &Path) -> bool {
        looks_like_bepinex_pack(content_root)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        bepinex_preflight_warnings(install_path)
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
        assert_eq!(
            dest,
            PathBuf::from("/game/BepInEx/plugins/CoolMod.dll")
        );
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
    fn preserves_plugins_prefix() {
        let dest = resolve_bepinex_deploy(
            Path::new("/game"),
            Path::new("plugins/MyMod/MyMod.dll"),
        )
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
        assert_eq!(thunderstore_community_for_plugin("bonelab"), Some("bonelab"));
        assert_eq!(
            thunderstore_community_for_plugin("boneworks"),
            Some("boneworks")
        );
    }
}
