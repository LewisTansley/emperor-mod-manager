//! Windows game detection via Steam libraryfolders + optional Heroic.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use winreg::enums::HKEY_CURRENT_USER;
use winreg::RegKey;

use super::DetectedGame;

pub fn scan_games() -> Vec<DetectedGame> {
    let mut out = Vec::new();
    out.extend(scan_steam());
    out.extend(scan_heroic());

    out.sort_by(|a, b| {
        b.supported
            .cmp(&a.supported)
            .then_with(|| {
                b.engine_hint
                    .is_some()
                    .cmp(&a.engine_hint.is_some())
            })
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    out
}

fn scan_steam() -> Vec<DetectedGame> {
    let Some(steam_root) = steam_install_path() else {
        log::info!("Steam install path not found in registry");
        return Vec::new();
    };
    let libraries = steam_library_paths(&steam_root);
    let mut out = Vec::new();
    for lib in libraries {
        let steamapps = lib.join("steamapps");
        let Ok(entries) = fs::read_dir(&steamapps) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }
            if let Some(game) = parse_appmanifest(&path, &steamapps) {
                out.push(game);
            }
        }
    }
    out
}

fn steam_install_path() -> Option<PathBuf> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let steam = hkcu.open_subkey(r"Software\Valve\Steam").ok()?;
    let path: String = steam.get_value("SteamPath").ok()?;
    let path = PathBuf::from(path.replace('/', "\\"));
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}

fn steam_library_paths(steam_root: &Path) -> Vec<PathBuf> {
    let mut libs = vec![steam_root.to_path_buf()];
    let vdf = steam_root.join("steamapps").join("libraryfolders.vdf");
    let Ok(text) = fs::read_to_string(&vdf) else {
        return libs;
    };
    // Match "path" "C:\\..." entries in libraryfolders.vdf (VDF is loosely structured).
    let re = Regex::new(r#""path"\s+"([^"]+)""#).expect("libraryfolders path regex");
    for cap in re.captures_iter(&text) {
        let raw = cap[1].replace("\\\\", "\\");
        let p = PathBuf::from(raw);
        if p.is_dir() && !libs.iter().any(|e| e == &p) {
            libs.push(p);
        }
    }
    libs
}

fn parse_appmanifest(acf: &Path, steamapps: &Path) -> Option<DetectedGame> {
    let text = fs::read_to_string(acf).ok()?;
    let name = vdf_string(&text, "name")?;
    let installdir = vdf_string(&text, "installdir")?;
    let install_path = steamapps.join("common").join(&installdir);
    if !install_path.is_dir() {
        return None;
    }
    Some(super::finish_detected(
        name,
        "Steam".to_string(),
        Some(install_path.to_string_lossy().to_string()),
        None,
    ))
}

fn vdf_string(text: &str, key: &str) -> Option<String> {
    let pattern = format!(r#""{key}"\s+"([^"]*)""#);
    let re = Regex::new(&pattern).ok()?;
    re.captures(text)
        .map(|c| c[1].to_string())
        .filter(|s| !s.is_empty())
}

/// Heroic Games Launcher stores installed games under %APPDATA%/heroic/store_cache
/// and config in legendary/gog configs. We read `installed.json` style paths when present.
fn scan_heroic() -> Vec<DetectedGame> {
    let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) else {
        return Vec::new();
    };
    let heroic = appdata.join("heroic");
    if !heroic.is_dir() {
        return Vec::new();
    }

    let mut out = Vec::new();

    // Legendary (Epic) installed.json
    let legendary = heroic
        .join("legendaryConfig")
        .join("legendary")
        .join("installed.json");
    if let Ok(text) = fs::read_to_string(&legendary) {
        if let Ok(map) = serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(&text) {
            for (_id, val) in map {
                let title = val
                    .get("title")
                    .or_else(|| val.get("app_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string();
                let install_path = val
                    .get("install_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if install_path
                    .as_ref()
                    .map(|p| Path::new(p).is_dir())
                    .unwrap_or(false)
                {
                    out.push(super::finish_detected(
                        title,
                        "Heroic".to_string(),
                        install_path,
                        None,
                    ));
                }
            }
        }
    }

    // GOG installed games list used by Heroic
    let gog = heroic.join("gog_store").join("installed.json");
    if let Ok(text) = fs::read_to_string(&gog) {
        if let Ok(list) = serde_json::from_str::<Vec<serde_json::Value>>(&text) {
            for val in list {
                let title = val
                    .get("title")
                    .or_else(|| val.get("appName"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("Unknown")
                    .to_string();
                let install_path = val
                    .get("install_path")
                    .or_else(|| val.get("installPath"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if install_path
                    .as_ref()
                    .map(|p| Path::new(p).is_dir())
                    .unwrap_or(false)
                {
                    out.push(super::finish_detected(
                        title,
                        "Heroic".to_string(),
                        install_path,
                        None,
                    ));
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vdf_string_parses_name() {
        let sample = r#"
"AppState"
{
	"appid"		"413150"
	"name"		"Stardew Valley"
	"installdir"		"Stardew Valley"
}
"#;
        assert_eq!(
            vdf_string(sample, "name").as_deref(),
            Some("Stardew Valley")
        );
        assert_eq!(
            vdf_string(sample, "installdir").as_deref(),
            Some("Stardew Valley")
        );
    }
}
