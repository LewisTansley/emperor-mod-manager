//! Creation Engine titles: Skyrim SE, Fallout 4, Starfield, and Oblivion Remastered.
//!
//! v1: overlay into Data/ (or ObvData/Data + Paks/~mods for the remaster), rewrite
//! Plugins.txt, and warn when the matching script extender is missing.
//! Starfield also writes `StarfieldCustom.ini` in My Games so loose files load.
//! Not in v1: LOOT, FOMOD, BSA/BA2 extraction.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use super::{normalize_relative, GamePlugin, GamePluginInfo, PluginSkip};

const SKYRIM_SE_APP_ID: &str = "489830";
const FALLOUT4_APP_ID: &str = "377160";
const STARFIELD_APP_ID: &str = "1716740";
const OBLIVION_REMASTERED_APP_ID: &str = "2623190";

pub const CREATION_ENGINE_ROOT_DIRS: &[&str] = &["Data", "SKSE", "F4SE", "SFSE", "OBSE", "Scripts"];

pub const OBLIVION_REMASTERED_ROOT_DIRS: &[&str] = &[
    "OblivionRemastered",
    "Data",
    "Paks",
    "~mods",
    "Content",
    "Binaries",
    "OBSE",
    "Scripts",
];

const SKYRIM_SE_MASTERS: &[&str] = &[
    "Skyrim.esm",
    "Update.esm",
    "Dawnguard.esm",
    "HearthFires.esm",
    "Dragonborn.esm",
];

const FALLOUT4_MASTERS: &[&str] = &[
    "Fallout4.esm",
    "DLCRobot.esm",
    "DLCworkshop01.esm",
    "DLCCoast.esm",
    "DLCworkshop02.esm",
    "DLCworkshop03.esm",
    "DLCNukaWorld.esm",
];

const STARFIELD_MASTERS: &[&str] = &[
    "Starfield.esm",
    "Constellation.esm",
    "OldMars.esm",
    "ShatteredSpace.esm",
];

const OBLIVION_REMASTERED_MASTERS: &[&str] = &[
    "Oblivion.esm",
    "DLCShiveringIsles.esp",
    "Knights.esp",
    "AltarESPMain.esp",
    "AltarDeluxe.esp",
    "AltarESPLocal.esp",
];

const EXTENDER_LOADERS: &[&str] = &[
    "skse64_loader.exe",
    "skse64_loader",
    "f4se_loader.exe",
    "f4se_loader",
    "sfse_loader.exe",
    "sfse_loader",
    "obse64_loader.exe",
    "obse64_loader",
];

#[derive(Clone, Copy)]
struct CreationTitle {
    id: &'static str,
    display_name: &'static str,
    nexus_domain: &'static str,
    match_names: &'static [&'static str],
    steam_app_id: &'static str,
    local_appdata_game: &'static str,
    plugins_in_documents: bool,
    documents_game: &'static str,
    extender_names: &'static [&'static str],
    required_masters: &'static [&'static str],
    asterisk_enabled: bool,
}

const SKYRIM_SE: CreationTitle = CreationTitle {
    id: "skyrimspecialedition",
    display_name: "Skyrim Special Edition",
    nexus_domain: "skyrimspecialedition",
    match_names: &[
        "skyrim special edition",
        "skyrimspecialedition",
        "skyrim se",
        "skyrim ae",
        "skyrim anniversary",
    ],
    steam_app_id: SKYRIM_SE_APP_ID,
    local_appdata_game: "Skyrim Special Edition",
    plugins_in_documents: false,
    documents_game: "",
    extender_names: &["skse64_loader.exe", "skse64_loader", "skse64.dll"],
    required_masters: SKYRIM_SE_MASTERS,
    asterisk_enabled: true,
};

const FALLOUT4: CreationTitle = CreationTitle {
    id: "fallout4",
    display_name: "Fallout 4",
    nexus_domain: "fallout4",
    match_names: &["fallout 4", "fallout4"],
    steam_app_id: FALLOUT4_APP_ID,
    local_appdata_game: "Fallout4",
    plugins_in_documents: false,
    documents_game: "",
    extender_names: &["f4se_loader.exe", "f4se_loader", "f4se.dll"],
    required_masters: FALLOUT4_MASTERS,
    asterisk_enabled: true,
};

const STARFIELD: CreationTitle = CreationTitle {
    id: "starfield",
    display_name: "Starfield",
    nexus_domain: "starfield",
    match_names: &["starfield"],
    steam_app_id: STARFIELD_APP_ID,
    local_appdata_game: "Starfield",
    plugins_in_documents: true,
    documents_game: "Starfield",
    extender_names: &["sfse_loader.exe", "sfse_loader", "sfse.dll"],
    required_masters: STARFIELD_MASTERS,
    asterisk_enabled: true,
};

macro_rules! ce_title_plugin {
    ($struct:ident, $title:expr) => {
        pub struct $struct;

        impl GamePlugin for $struct {
            fn info(&self) -> GamePluginInfo {
                GamePluginInfo {
                    id: $title.id,
                    display_name: $title.display_name,
                    nexus_domain: $title.nexus_domain,
                    match_names: $title.match_names,
                }
            }

            fn preserve_staging_root_names(&self) -> &[&str] {
                CREATION_ENGINE_ROOT_DIRS
            }

            fn deploys_to_install_root(&self, content_root: &Path) -> bool {
                looks_like_script_extender(content_root)
            }

            fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
                extender_preflight(install_path, $title)
            }

            fn staging_deploy_warnings(&self, content_root: &Path, mod_name: &str) -> Vec<String> {
                if looks_like_script_extender(content_root) {
                    vec![format!(
                        "{mod_name} looks like a script extender; files were deployed to the game root."
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
                Ok(resolve_ce_deploy(install_path, relative))
            }

            fn after_deploy(
                &self,
                install_path: &Path,
                _enabled_mod_folders: &[String],
            ) -> Result<Vec<String>> {
                let mut warnings =
                    write_plugins_txt(install_path, $title, &install_path.join("Data"))?;
                if $title.id == "starfield" {
                    warnings.extend(ensure_starfield_custom_ini(install_path)?);
                }
                Ok(warnings)
            }

            fn plugin_skips(&self, install_path: &Path) -> Vec<PluginSkip> {
                data_dir_plugin_skips(&install_path.join("Data"))
            }
        }
    };
}

ce_title_plugin!(SkyrimSpecialEditionPlugin, SKYRIM_SE);
ce_title_plugin!(Fallout4Plugin, FALLOUT4);
ce_title_plugin!(StarfieldPlugin, STARFIELD);

pub struct OblivionRemasteredPlugin;

impl GamePlugin for OblivionRemasteredPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "oblivionremastered",
            display_name: "Oblivion Remastered",
            nexus_domain: "oblivionremastered",
            match_names: &[
                "oblivion remastered",
                "oblivionremastered",
                "tes iv: oblivion remastered",
            ],
        }
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        OBLIVION_REMASTERED_ROOT_DIRS
    }

    fn deploys_to_install_root(&self, content_root: &Path) -> bool {
        looks_like_script_extender(content_root)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        let mut warnings = extender_preflight(
            install_path,
            CreationTitle {
                id: "oblivionremastered",
                display_name: "Oblivion Remastered",
                nexus_domain: "oblivionremastered",
                match_names: &[],
                steam_app_id: OBLIVION_REMASTERED_APP_ID,
                local_appdata_game: "",
                plugins_in_documents: false,
                documents_game: "",
                extender_names: &["obse64_loader.exe", "obse64_loader"],
                required_masters: OBLIVION_REMASTERED_MASTERS,
                asterisk_enabled: false,
            },
        );
        if obr_data_dir(install_path).is_none() && install_path.is_dir() {
            warnings.push(
                "OblivionRemastered/Content/Dev/ObvData/Data not found. ESP mods deploy there; UE paks go to Content/Paks/~mods."
                    .into(),
            );
        }
        warnings
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_obr_deploy(install_path, relative))
    }

    fn after_deploy(
        &self,
        install_path: &Path,
        _enabled_mod_folders: &[String],
    ) -> Result<Vec<String>> {
        let Some(data) = obr_data_dir(install_path) else {
            return Ok(vec![
                "Skipped Plugins.txt: ObvData/Data folder was not found.".into(),
            ]);
        };
        write_plugins_file(
            &data.join("Plugins.txt"),
            &data,
            OBLIVION_REMASTERED_MASTERS,
            false,
        )
    }

    fn plugin_skips(&self, install_path: &Path) -> Vec<PluginSkip> {
        obr_data_dir(install_path)
            .map(|data| data_dir_plugin_skips(&data))
            .unwrap_or_default()
    }
}

fn looks_like_script_extender(content_root: &Path) -> bool {
    for name in EXTENDER_LOADERS {
        if content_root.join(name).is_file() {
            return true;
        }
    }
    let Ok(entries) = std::fs::read_dir(content_root) else {
        return false;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.starts_with("skse64") && name.ends_with(".dll") {
            return true;
        }
        if name.starts_with("f4se_") && name.ends_with(".dll") {
            return true;
        }
        if name.starts_with("sfse_") && name.ends_with(".dll") {
            return true;
        }
        if name.starts_with("obse") && name.ends_with(".dll") {
            return true;
        }
    }
    false
}

fn extender_preflight(install_path: &Path, title: CreationTitle) -> Vec<String> {
    let found = title
        .extender_names
        .iter()
        .any(|n| install_path.join(n).is_file())
        || looks_like_script_extender(install_path);
    if found {
        Vec::new()
    } else {
        vec![format!(
            "{} script extender not found in the game folder. SKSE/F4SE/SFSE/OBSE plugins will not load until the extender is installed.",
            title.display_name
        )]
    }
}

fn is_plugin_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".esp") || lower.ends_with(".esm") || lower.ends_with(".esl")
}

fn is_pak_like(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".pak") || lower.ends_with(".ucas") || lower.ends_with(".utoc")
}

/// Game spelling for Creation Engine directories. Archives often use `data` or
/// `plugins`; on a case-sensitive disk that becomes a second tree the game never reads.
fn canonical_ce_dir(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "data" => "Data",
        "interface" => "Interface",
        "meshes" => "Meshes",
        "textures" => "Textures",
        "strings" => "Strings",
        "music" => "Music",
        "sound" => "Sound",
        "scripts" => "Scripts",
        "skse" => "SKSE",
        "f4se" => "F4SE",
        "sfse" => "SFSE",
        "obse" => "OBSE",
        "plugins" => "Plugins",
        _ => return None,
    })
}

fn ci_child_names(parent: &Path, name: &str) -> Vec<std::ffi::OsString> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let file_name = entry.file_name();
            file_name
                .to_str()
                .is_some_and(|found| found.eq_ignore_ascii_case(name))
                .then_some(file_name)
        })
        .collect()
}

/// Prefer the single on-disk spelling so deploy does not create `data` next to `Data`.
/// Known folders fall back to the game's own spelling when nothing is there yet.
fn fold_ce_component(parent: &Path, name: &std::ffi::OsStr) -> std::ffi::OsString {
    let name_str = name.to_string_lossy();
    let matches = ci_child_names(parent, &name_str);
    if matches.len() == 1 {
        return matches
            .into_iter()
            .next()
            .unwrap_or_else(|| name.to_os_string());
    }
    if let Some(canon) = canonical_ce_dir(&name_str) {
        return std::ffi::OsString::from(canon);
    }
    name.to_os_string()
}

fn join_ce_components(root: &Path, components: &[std::ffi::OsString]) -> PathBuf {
    let mut out = root.to_path_buf();
    for name in components {
        out.push(fold_ce_component(&out, name));
    }
    out
}

fn path_components(relative: &Path) -> Vec<std::ffi::OsString> {
    normalize_relative(relative)
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect()
}

fn resolve_ce_deploy(install_path: &Path, relative: &Path) -> PathBuf {
    let originals = path_components(relative);
    if originals.is_empty() {
        return join_ce_components(install_path, &[std::ffi::OsString::from("Data")]);
    }
    let first_lower = originals[0].to_string_lossy().to_lowercase();
    if first_lower == "data" {
        return join_ce_components(install_path, &originals);
    }
    let mut components = Vec::with_capacity(originals.len() + 1);
    components.push(std::ffi::OsString::from("Data"));
    components.extend(originals);
    join_ce_components(install_path, &components)
}

fn obr_project_dir(install_path: &Path) -> PathBuf {
    if looks_like_obr_project(install_path) {
        return install_path.to_path_buf();
    }
    install_path.join("OblivionRemastered")
}

fn looks_like_obr_project(dir: &Path) -> bool {
    dir.file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("OblivionRemastered"))
        || dir.join("Content").join("Dev").join("ObvData").is_dir()
        || dir.join("Content").join("Paks").is_dir()
}

fn obr_data_dir(install_path: &Path) -> Option<PathBuf> {
    let project = obr_project_dir(install_path);
    let data = project
        .join("Content")
        .join("Dev")
        .join("ObvData")
        .join("Data");
    if data.is_dir() || project.join("Content").is_dir() {
        Some(data)
    } else if install_path
        .join("Content")
        .join("Dev")
        .join("ObvData")
        .join("Data")
        .is_dir()
    {
        Some(
            install_path
                .join("Content")
                .join("Dev")
                .join("ObvData")
                .join("Data"),
        )
    } else {
        None
    }
}

fn resolve_obr_deploy(install_path: &Path, relative: &Path) -> PathBuf {
    let project = obr_project_dir(install_path);
    let data = project
        .join("Content")
        .join("Dev")
        .join("ObvData")
        .join("Data");
    let mods = project.join("Content").join("Paks").join("~mods");
    let normalized = normalize_relative(relative);
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect();
    if originals.is_empty() {
        return data;
    }
    let lowers: Vec<String> = originals
        .iter()
        .map(|s| s.to_string_lossy().to_lowercase())
        .collect();
    let first = lowers[0].as_str();

    if first == "oblivionremastered" {
        let mut out = install_path.to_path_buf();
        for orig in originals {
            out.push(orig);
        }
        return out;
    }
    if first == "content" || first == "paks" || first == "~mods" || first == "binaries" {
        let mut out = project;
        if first == "paks" || first == "~mods" {
            out.push("Content");
            if first == "~mods" {
                out.push("Paks");
            }
        }
        for orig in originals {
            out.push(orig);
        }
        return out;
    }
    if is_pak_like(first) || lowers.iter().any(|s| s == "paks" || s == "~mods") {
        return mods.join(&normalized);
    }
    if first == "data" {
        let rest: Vec<_> = originals.into_iter().skip(1).collect();
        return join_ce_components(&data, &rest);
    }
    if first == "obse" || first == "scripts" {
        return join_ce_components(&data, &originals);
    }
    join_ce_components(&data, &originals)
}

fn write_plugins_txt(
    install_path: &Path,
    title: CreationTitle,
    data_dir: &Path,
) -> Result<Vec<String>> {
    let Some(path) = plugins_txt_path(install_path, title) else {
        return Ok(Vec::new());
    };
    write_plugins_file(
        &path,
        data_dir,
        title.required_masters,
        title.asterisk_enabled,
    )
}

fn plugins_txt_path(install_path: &Path, title: CreationTitle) -> Option<PathBuf> {
    if title.plugins_in_documents {
        if let Some(dir) =
            documents_game_dir(install_path, title.steam_app_id, title.documents_game)
        {
            return Some(dir.join("Plugins.txt"));
        }
    }
    local_appdata_game_dir(install_path, title.steam_app_id, title.local_appdata_game)
        .map(|d| d.join("Plugins.txt"))
}

fn write_plugins_file(
    path: &Path,
    data_dir: &Path,
    required_masters: &[&str],
    asterisk: bool,
) -> Result<Vec<String>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut found = list_plugin_files(data_dir);
    for master in required_masters {
        if data_dir.join(master).is_file() && !found.iter().any(|f| f.eq_ignore_ascii_case(master))
        {
            found.push((*master).to_string());
        }
    }
    let (loadable, warnings, _) = loadable_plugins(data_dir, &found);
    let mut body = String::from("# Auto-generated by Emperor Mod Manager.\n");
    let mut emitted = std::collections::HashSet::<String>::new();
    for master in required_masters {
        if !loadable.iter().any(|f| f.eq_ignore_ascii_case(master)) {
            continue;
        }
        push_plugin_line(&mut body, master, asterisk);
        emitted.insert(master.to_lowercase());
    }
    let mut rest = loadable;
    rest.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    for name in rest {
        if emitted.contains(&name.to_lowercase()) {
            continue;
        }
        push_plugin_line(&mut body, &name, asterisk);
        emitted.insert(name.to_lowercase());
    }
    std::fs::write(path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(warnings)
}

fn data_dir_plugin_skips(data_dir: &Path) -> Vec<PluginSkip> {
    if !data_dir.is_dir() {
        return Vec::new();
    }
    let found = list_plugin_files(data_dir);
    let (_, _, skips) = loadable_plugins(data_dir, &found);
    skips
}

/// Plugins the game can open: every `MAST` exists on disk, and was not itself skipped.
///
/// A plugin we cannot parse is kept. Creation Engine quits at startup when a listed
/// plugin's master is missing, so those are omitted and reported.
fn loadable_plugins(
    data_dir: &Path,
    names: &[String],
) -> (Vec<String>, Vec<String>, Vec<PluginSkip>) {
    let present: std::collections::HashSet<String> =
        names.iter().map(|name| name.to_lowercase()).collect();
    let parsed: Vec<Option<Vec<String>>> = names
        .iter()
        .map(|name| plugin_masters(&data_dir.join(name)))
        .collect();
    let mut dropped = std::collections::HashSet::<String>::new();
    loop {
        let mut changed = false;
        for (name, masters) in names.iter().zip(parsed.iter()) {
            let key = name.to_lowercase();
            if dropped.contains(&key) {
                continue;
            }
            let Some(masters) = masters else {
                continue;
            };
            let blocked = masters.iter().any(|master| {
                let master_key = master.to_lowercase();
                !present.contains(&master_key) || dropped.contains(&master_key)
            });
            if blocked {
                dropped.insert(key);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut warnings = Vec::new();
    let mut skips = Vec::new();
    let mut loadable = Vec::new();
    for (name, masters) in names.iter().zip(parsed.iter()) {
        let key = name.to_lowercase();
        if !dropped.contains(&key) {
            loadable.push(name.clone());
            continue;
        }
        let Some(masters) = masters else {
            continue;
        };
        let Some(master) = masters.iter().find(|master| {
            let master_key = master.to_lowercase();
            !present.contains(&master_key) || dropped.contains(&master_key)
        }) else {
            continue;
        };
        let master_key = master.to_lowercase();
        if present.contains(&master_key) {
            warnings.push(format!(
                "Skipped {name}: master {master} was left out because its own masters are missing."
            ));
        } else {
            warnings.push(format!("Skipped {name}: missing master {master}."));
        }
        skips.push(PluginSkip {
            plugin: name.clone(),
            missing_master: master.clone(),
        });
    }
    (loadable, warnings, skips)
}

/// `TES4` master filenames. `None` when the file is not a readable plugin header.
fn plugin_masters(path: &Path) -> Option<Vec<String>> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).ok()?;
    let mut header = [0u8; 24];
    file.read_exact(&mut header).ok()?;
    if &header[0..4] != b"TES4" {
        return None;
    }
    let data_size = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
    // A corrupt size must not pull an entire master file into memory.
    if data_size > 1_048_576 {
        return None;
    }
    let mut body = vec![0u8; data_size];
    file.read_exact(&mut body).ok()?;

    // Subrecord sizes stay 16-bit even when the localized flag (0x80) is set.
    // Starfield.esm is flagged 0x81 and still stores HEDR/MAST that way.
    let mut offset = 0;
    let mut masters = Vec::new();
    while offset + 6 <= body.len() {
        let kind = &body[offset..offset + 4];
        let len = u16::from_le_bytes(body[offset + 4..offset + 6].try_into().ok()?) as usize;
        let payload_at = offset + 6;
        if len > body.len() - payload_at {
            break;
        }
        if kind == b"MAST" {
            let payload = &body[payload_at..payload_at + len];
            let end = payload
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(payload.len());
            let name = String::from_utf8_lossy(&payload[..end]).trim().to_string();
            if !name.is_empty() {
                masters.push(name);
            }
        }
        offset = payload_at + len;
    }
    Some(masters)
}

fn push_plugin_line(body: &mut String, name: &str, asterisk: bool) {
    if asterisk {
        body.push('*');
    }
    body.push_str(name);
    body.push('\n');
}

fn list_plugin_files(data_dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(data_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if is_plugin_file(&name) {
            out.push(name.into_owned());
        }
    }
    out
}

fn local_appdata_game_dir(
    install_path: &Path,
    steam_app_id: &str,
    game_folder: &str,
) -> Option<PathBuf> {
    if game_folder.is_empty() {
        return None;
    }
    super::user_data::appdata_local_dir(install_path, steam_app_id).map(|p| p.join(game_folder))
}

fn documents_game_dir(
    install_path: &Path,
    steam_app_id: &str,
    game_folder: &str,
) -> Option<PathBuf> {
    if game_folder.is_empty() {
        return None;
    }
    super::user_data::documents_dir(install_path, steam_app_id)
        .map(|docs| docs.join("My Games").join(game_folder))
}

const STARFIELD_CUSTOM_INI: &str = "StarfieldCustom.ini";

/// Create or update My Games `StarfieldCustom.ini`. The game does not read the
/// copy under the install `Data/` folder; that file is only a source of extra keys.
fn ensure_starfield_custom_ini(install_path: &Path) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let Some(dir) = documents_game_dir(
        install_path,
        STARFIELD.steam_app_id,
        STARFIELD.documents_game,
    ) else {
        warnings.push(
            "Could not find Documents/My Games/Starfield. StarfieldCustom.ini was not written, so loose files will not load."
                .into(),
        );
        return Ok(warnings);
    };

    let dest = dir.join(STARFIELD_CUSTOM_INI);
    let existing = std::fs::read_to_string(&dest).ok();
    let data_dir = existing_named_dir(install_path, "Data");
    let data_ini = data_dir
        .as_ref()
        .and_then(|dir| read_named_file_ci(dir, STARFIELD_CUSTOM_INI));
    let merged = merge_starfield_custom_ini(existing.as_deref(), data_ini.as_deref());
    let unchanged = existing
        .as_deref()
        .is_some_and(|text| ini_text_eq(text, &merged));
    if !unchanged {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        std::fs::write(&dest, &merged).with_context(|| format!("write {}", dest.display()))?;
    }

    for name in ci_child_names(&dir, "Data") {
        let shadow = dir.join(name);
        if shadow.is_dir() && directory_has_entries(&shadow) {
            warnings.push(format!(
                "{} overrides the game install Data folder, so deployed loose files may not load.",
                shadow.display()
            ));
            break;
        }
    }
    Ok(warnings)
}

fn existing_named_dir(parent: &Path, name: &str) -> Option<PathBuf> {
    let matches = ci_child_names(parent, name);
    let path = if matches.len() == 1 {
        parent.join(&matches[0])
    } else {
        parent.join(name)
    };
    path.is_dir().then_some(path)
}

fn read_named_file_ci(dir: &Path, name: &str) -> Option<String> {
    for found in ci_child_names(dir, name) {
        let path = dir.join(found);
        if path.is_file() {
            return std::fs::read_to_string(path).ok();
        }
    }
    None
}

fn directory_has_entries(path: &Path) -> bool {
    std::fs::read_dir(path)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
}

fn strip_ini_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

fn ini_text_eq(existing: &str, merged: &str) -> bool {
    let existing = strip_ini_bom(existing)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let merged = strip_ini_bom(merged)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    existing == merged
}

fn ini_section(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    let name = trimmed.strip_prefix('[')?.strip_suffix(']')?.trim();
    (!name.is_empty()).then_some(name)
}

fn ini_assignment(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with(';')
        || trimmed.starts_with('#')
        || trimmed.starts_with('[')
    {
        return None;
    }
    let (key, value) = trimmed.split_once('=')?;
    let key = key.trim();
    (!key.is_empty()).then_some((key, value.trim()))
}

fn section_has_key(lines: &[String], section: &str, key: &str) -> bool {
    let mut current = "";
    for line in lines {
        if let Some(name) = ini_section(line) {
            current = name;
            continue;
        }
        if current.eq_ignore_ascii_case(section) {
            if let Some((found, _)) = ini_assignment(line) {
                if found.eq_ignore_ascii_case(key) {
                    return true;
                }
            }
        }
    }
    false
}

/// Where a new key in `section` belongs: after that section's lines, before the next header.
fn section_insert_at(lines: &[String], section: &str) -> Option<usize> {
    let mut header = None;
    for (index, line) in lines.iter().enumerate() {
        if let Some(name) = ini_section(line) {
            if header.is_some() {
                return Some(index);
            }
            if name.eq_ignore_ascii_case(section) {
                header = Some(index);
            }
        }
    }
    header.map(|_| lines.len())
}

fn insert_ini_key(lines: &mut Vec<String>, section: &str, key: &str, value: &str) {
    let line = format!("{key}={value}");
    if let Some(mut at) = section_insert_at(lines, section) {
        if at < lines.len() && ini_section(&lines[at]).is_some() {
            while at > 0 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
        }
        lines.insert(at, line);
        return;
    }
    if lines.last().is_some_and(|last| !last.trim().is_empty()) {
        lines.push(String::new());
    }
    lines.push(format!("[{section}]"));
    lines.push(line);
}

fn force_ini_key(lines: &mut Vec<String>, section: &str, key: &str, value: &str) {
    let mut current = "";
    let mut found = None;
    for (index, line) in lines.iter().enumerate() {
        if let Some(name) = ini_section(line) {
            current = name;
            continue;
        }
        if current.eq_ignore_ascii_case(section) {
            if let Some((found_key, found_value)) = ini_assignment(line) {
                if found_key.eq_ignore_ascii_case(key) {
                    found = Some((index, found_value.to_string()));
                    break;
                }
            }
        }
    }
    if let Some((index, existing)) = found {
        if existing != value {
            lines[index] = format!("{key}={value}");
        }
    } else {
        insert_ini_key(lines, section, key, value);
    }
}

fn ensure_ini_key(lines: &mut Vec<String>, section: &str, key: &str, value: &str) {
    if !section_has_key(lines, section, key) {
        insert_ini_key(lines, section, key, value);
    }
}

fn overlay_missing_ini_keys(lines: &mut Vec<String>, overlay: &str) {
    let mut current: Option<String> = None;
    for line in overlay.lines() {
        if let Some(name) = ini_section(line) {
            current = Some(name.to_string());
            continue;
        }
        let Some(section) = current.as_deref() else {
            continue;
        };
        let Some((key, value)) = ini_assignment(line) else {
            continue;
        };
        if !section_has_key(lines, section, key) {
            insert_ini_key(lines, section, key, value);
        }
    }
}

fn ini_lines(text: &str) -> Vec<String> {
    let text = strip_ini_bom(text);
    if text.is_empty() {
        Vec::new()
    } else {
        text.lines().map(str::to_string).collect()
    }
}

fn render_ini(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// My Games text wins. Keys that only exist in the install `Data` ini are filled in.
/// Archive invalidation is always forced. Message-of-the-day keys are added only when absent.
fn merge_starfield_custom_ini(my_games: Option<&str>, data_ini: Option<&str>) -> String {
    let mut lines = ini_lines(my_games.unwrap_or(""));
    if let Some(data) = data_ini {
        overlay_missing_ini_keys(&mut lines, strip_ini_bom(data));
    }
    force_ini_key(&mut lines, "Archive", "bInvalidateOlderFiles", "1");
    force_ini_key(&mut lines, "Archive", "sResourceDataDirsFinal", "");
    ensure_ini_key(&mut lines, "General", "bEnableMessageOfTheDay", "0");
    ensure_ini_key(&mut lines, "General", "sPhotoModeFolder", "Photos");
    render_ini(&lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skyrim_esp_goes_to_data() {
        let dest = SkyrimSpecialEditionPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("Cool.esp"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Data/Cool.esp"));
        let dest = SkyrimSpecialEditionPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("Data/meshes/x.nif"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Data/Meshes/x.nif"));
        let dest = SkyrimSpecialEditionPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("SKSE/Plugins/foo.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Data/SKSE/Plugins/foo.dll"));
    }

    #[test]
    fn writes_plugins_txt_with_asterisks_and_masters_first() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Skyrim.esm"), b"x").unwrap();
        std::fs::write(data.join("Cool.esp"), b"x").unwrap();
        std::fs::write(data.join("Update.esm"), b"x").unwrap();
        let plugins = tmp.path().join("Plugins.txt");
        write_plugins_file(&plugins, &data, SKYRIM_SE_MASTERS, true).unwrap();
        let text = std::fs::read_to_string(&plugins).unwrap();
        let skyrim = text.find("*Skyrim.esm").unwrap();
        let update = text.find("*Update.esm").unwrap();
        let cool = text.find("*Cool.esp").unwrap();
        assert!(skyrim < update && update < cool);
    }

    #[test]
    fn fallout4_and_starfield_ids() {
        assert_eq!(Fallout4Plugin.info().id, "fallout4");
        assert_eq!(StarfieldPlugin.info().nexus_domain, "starfield");
        assert!(StarfieldPlugin
            .preflight_warnings(tempfile::tempdir().unwrap().path())
            .iter()
            .any(|w| w.contains("script extender")));
    }

    #[test]
    fn oblivion_remastered_splits_esp_and_paks() {
        let dest = OblivionRemasteredPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("MyMod.esp"))
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/OblivionRemastered/Content/Dev/ObvData/Data/MyMod.esp")
        );
        let dest = OblivionRemasteredPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("Cool.pak"))
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/OblivionRemastered/Content/Paks/~mods/Cool.pak")
        );
        let dest = OblivionRemasteredPlugin
            .resolve_deploy_root(
                Path::new("/game"),
                Path::new("OblivionRemastered/Content/Paks/~mods/A.pak"),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/OblivionRemastered/Content/Paks/~mods/A.pak")
        );
    }

    #[test]
    fn oblivion_plugins_txt_no_asterisk() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Oblivion.esm"), b"x").unwrap();
        std::fs::write(data.join("AltarESPMain.esp"), b"x").unwrap();
        std::fs::write(data.join("My.esp"), b"x").unwrap();
        let plugins = data.join("Plugins.txt");
        write_plugins_file(&plugins, &data, OBLIVION_REMASTERED_MASTERS, false).unwrap();
        let text = std::fs::read_to_string(&plugins).unwrap();
        assert!(text.contains("Oblivion.esm\n"));
        assert!(!text.contains("*Oblivion.esm"));
        assert!(text.find("AltarESPMain.esp").unwrap() < text.find("My.esp").unwrap());
    }

    #[test]
    fn skse_pack_deploys_to_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("skse64_loader.exe"), b"x").unwrap();
        assert!(SkyrimSpecialEditionPlugin.deploys_to_install_root(dir.path()));
    }

    fn tes4(masters: &[&str]) -> Vec<u8> {
        tes4_flags(masters, 1)
    }

    fn tes4_flags(masters: &[&str], flags: u32) -> Vec<u8> {
        let mut subs = Vec::new();
        for master in masters {
            let bytes = master.as_bytes();
            subs.extend_from_slice(b"MAST");
            subs.extend_from_slice(&((bytes.len() + 1) as u16).to_le_bytes());
            subs.extend_from_slice(bytes);
            subs.push(0);
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"TES4");
        out.extend_from_slice(&(subs.len() as u32).to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&[0u8; 12]);
        out.extend_from_slice(&subs);
        out
    }

    #[test]
    fn skips_plugins_whose_masters_are_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Starfield.esm"), tes4(&[])).unwrap();
        std::fs::write(data.join("BetterNPCs.esm"), tes4(&["Starfield.esm"])).unwrap();
        std::fs::write(
            data.join("BetterShipPartFlips.esm"),
            tes4(&["Starfield.esm", "ShipBuilderCategories.esm"]),
        )
        .unwrap();
        std::fs::write(
            data.join("BSPF-Patch.esm"),
            tes4(&["BetterShipPartFlips.esm"]),
        )
        .unwrap();
        let plugins = tmp.path().join("Plugins.txt");
        let warnings = write_plugins_file(&plugins, &data, STARFIELD_MASTERS, true).unwrap();
        let text = std::fs::read_to_string(&plugins).unwrap();
        assert!(text.contains("*Starfield.esm\n"));
        assert!(text.contains("*BetterNPCs.esm\n"));
        assert!(!text.contains("BetterShipPartFlips.esm"));
        assert!(!text.contains("BSPF-Patch.esm"));
        assert!(warnings.iter().any(|w| {
            w.contains("BetterShipPartFlips.esm") && w.contains("ShipBuilderCategories.esm")
        }));
        assert!(warnings.iter().any(|w| w.contains("BSPF-Patch.esm")));
    }

    #[test]
    fn plugin_skips_name_the_missing_master() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Skyrim.esm"), tes4(&[])).unwrap();
        std::fs::write(data.join("Patch.esp"), tes4(&["Skyrim.esm", "Missing.esm"])).unwrap();
        let skips = SkyrimSpecialEditionPlugin.plugin_skips(tmp.path());
        assert!(skips
            .iter()
            .any(|s| { s.plugin == "Patch.esp" && s.missing_master == "Missing.esm" }));
    }

    #[test]
    fn localized_plugin_masters_stay_sixteen_bit() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("Starfield.esm"), tes4_flags(&[], 0x81)).unwrap();
        std::fs::write(
            data.join("Localized.esm"),
            tes4_flags(&["Starfield.esm", "Missing.esm"], 0x81),
        )
        .unwrap();
        let plugins = tmp.path().join("Plugins.txt");
        let warnings = write_plugins_file(&plugins, &data, &["Starfield.esm"], true).unwrap();
        let text = std::fs::read_to_string(&plugins).unwrap();
        assert!(text.contains("*Starfield.esm\n"));
        assert!(!text.contains("Localized.esm"));
        assert!(warnings.iter().any(|w| w.contains("Missing.esm")));
    }

    #[test]
    fn folds_creation_engine_directory_case() {
        let dest = StarfieldPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("data/interface/x.swf"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Data/Interface/x.swf"));
        let dest = StarfieldPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("data/SFSE/plugins/a.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Data/SFSE/Plugins/a.dll"));
        let dest = OblivionRemasteredPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("data/obse/plugins/a.dll"))
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/OblivionRemastered/Content/Dev/ObvData/Data/OBSE/Plugins/a.dll")
        );
    }

    #[test]
    fn data_prefix_reuses_existing_data_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("Data");
        std::fs::create_dir(&data).unwrap();
        let dest = StarfieldPlugin
            .resolve_deploy_root(tmp.path(), Path::new("data/foo.esp"))
            .unwrap();
        assert_eq!(dest, data.join("foo.esp"));
    }

    #[test]
    fn starfield_custom_ini_created_from_nothing() {
        let text = merge_starfield_custom_ini(None, None);
        assert_eq!(
            text,
            "\
[Archive]
bInvalidateOlderFiles=1
sResourceDataDirsFinal=

[General]
bEnableMessageOfTheDay=0
sPhotoModeFolder=Photos
"
        );
    }

    #[test]
    fn starfield_custom_ini_keeps_comments_and_corrects_archive() {
        let text = merge_starfield_custom_ini(
            Some(
                "\
; keep me
[Archive]
bInvalidateOlderFiles=1
sResourceDataDirsFinal=STRINGS\\

[General]
SIntroSequence=
",
            ),
            None,
        );
        assert_eq!(
            text,
            "\
; keep me
[Archive]
bInvalidateOlderFiles=1
sResourceDataDirsFinal=

[General]
SIntroSequence=
bEnableMessageOfTheDay=0
sPhotoModeFolder=Photos
"
        );
    }

    #[test]
    fn starfield_custom_ini_fills_data_keys_without_replacing_my_games() {
        let text = merge_starfield_custom_ini(
            Some("[General]\nuGridsToLoad=5\n"),
            Some("[General]\nuGridsToLoad=7\nSIntroSequence=\n"),
        );
        assert!(text.contains("uGridsToLoad=5\n"));
        assert!(!text.contains("uGridsToLoad=7"));
        assert!(text.contains("SIntroSequence=\n"));
        assert!(text.contains("bInvalidateOlderFiles=1\n"));
        assert!(text.contains("sResourceDataDirsFinal=\n"));
    }

    #[test]
    fn starfield_custom_ini_already_correct_is_unchanged() {
        let original = "\
[Archive]
bInvalidateOlderFiles=1
sResourceDataDirsFinal=

[General]
bEnableMessageOfTheDay=0
sPhotoModeFolder=Photos
";
        assert_eq!(merge_starfield_custom_ini(Some(original), None), original);
    }

    #[test]
    fn starfield_custom_ini_adds_motd_keys_only_when_missing() {
        let text = merge_starfield_custom_ini(
            Some(
                "\
[Archive]
bInvalidateOlderFiles=1
sResourceDataDirsFinal=

[General]
bEnableMessageOfTheDay=1
",
            ),
            None,
        );
        assert!(text.contains("bEnableMessageOfTheDay=1\n"));
        assert!(text.contains("sPhotoModeFolder=Photos\n"));
        assert!(!text.contains("bEnableMessageOfTheDay=0"));
    }

    #[test]
    fn starfield_deploy_writes_custom_ini_and_warns_when_my_games_data_shadows() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp
            .path()
            .join("steamapps")
            .join("common")
            .join("Starfield");
        std::fs::create_dir_all(install.join("Data")).unwrap();
        std::fs::write(
            install.join("Data").join("StarfieldCustom.ini"),
            "[General]\nSIntroSequence=\n",
        )
        .unwrap();
        let my_games = tmp
            .path()
            .join("steamapps")
            .join("compatdata")
            .join(STARFIELD_APP_ID)
            .join("pfx")
            .join("drive_c")
            .join("users")
            .join("steamuser")
            .join("Documents")
            .join("My Games")
            .join("Starfield");
        let shadow = my_games.join("Data");
        std::fs::create_dir_all(&shadow).unwrap();
        std::fs::write(shadow.join("motd.dds"), b"x").unwrap();

        let warnings = ensure_starfield_custom_ini(&install).unwrap();
        let ini = my_games.join("StarfieldCustom.ini");
        let text = std::fs::read_to_string(&ini).unwrap();
        assert!(text.contains("SIntroSequence=\n"));
        assert!(text.contains("bInvalidateOlderFiles=1\n"));
        assert!(text.contains("sResourceDataDirsFinal=\n"));
        assert!(warnings.iter().any(|w| w.contains("overrides")));

        let mut perms = std::fs::metadata(&ini).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&ini, perms).unwrap();
        ensure_starfield_custom_ini(&install).unwrap();
        assert_eq!(std::fs::read_to_string(&ini).unwrap(), text);
    }
}
