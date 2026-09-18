//! Creation Engine titles: Skyrim SE, Fallout 4, Starfield, and Oblivion Remastered.
//!
//! v1: overlay into Data/ (or ObvData/Data + Paks/~mods for the remaster), rewrite
//! Plugins.txt, and warn when the matching script extender is missing.
//! Not in v1: LOOT, FOMOD, BSA/BA2 extraction.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use super::{normalize_relative, GamePlugin, GamePluginInfo};

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
                write_plugins_txt(
                    install_path,
                    $title,
                    &install_path.join("Data"),
                )?;
                Ok(Vec::new())
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
        )?;
        Ok(Vec::new())
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

fn resolve_ce_deploy(install_path: &Path, relative: &Path) -> PathBuf {
    let data = install_path.join("Data");
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
    let first = originals[0].to_string_lossy();
    let first_lower = first.to_lowercase();
    if first_lower == "data" {
        let mut out = install_path.to_path_buf();
        for orig in originals {
            out.push(orig);
        }
        return out;
    }
    if first_lower == "skse"
        || first_lower == "f4se"
        || first_lower == "sfse"
        || first_lower == "obse"
        || first_lower == "scripts"
    {
        let mut out = data;
        for orig in originals {
            out.push(orig);
        }
        return out;
    }
    if is_plugin_file(&first_lower)
        || first_lower.ends_with(".bsa")
        || first_lower.ends_with(".ba2")
    {
        return data.join(&normalized);
    }
    data.join(&normalized)
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
        let mut out = data;
        for orig in originals.into_iter().skip(1) {
            out.push(orig);
        }
        return out;
    }
    if first == "obse" || first == "scripts" {
        let mut out = data;
        for orig in originals {
            out.push(orig);
        }
        return out;
    }
    data.join(&normalized)
}

fn write_plugins_txt(install_path: &Path, title: CreationTitle, data_dir: &Path) -> Result<()> {
    let Some(path) = plugins_txt_path(install_path, title) else {
        return Ok(());
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
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let found = list_plugin_files(data_dir);
    let mut body = String::from("# Auto-generated by Emperor Mod Manager.\n");
    let mut emitted = std::collections::HashSet::<String>::new();
    for master in required_masters {
        if found.iter().any(|f| f.eq_ignore_ascii_case(master)) || data_dir.join(master).is_file() {
            push_plugin_line(&mut body, master, asterisk);
            emitted.insert(master.to_lowercase());
        }
    }
    let mut rest = found;
    rest.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    for name in rest {
        if emitted.contains(&name.to_lowercase()) {
            continue;
        }
        push_plugin_line(&mut body, &name, asterisk);
        emitted.insert(name.to_lowercase());
    }
    std::fs::write(path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
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
        assert_eq!(dest, PathBuf::from("/game/Data/meshes/x.nif"));
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
}
