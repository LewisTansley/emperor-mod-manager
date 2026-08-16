//! Windows game detection via Steam libraryfolders + optional Heroic.
//!
//! Pure path/cover helpers also compile under `cfg(test)` on other hosts so CI
//! can cover Steam librarycache resolution without a Windows linker.

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;

#[cfg(target_os = "windows")]
use std::collections::HashSet;
#[cfg(target_os = "windows")]
use winreg::enums::HKEY_CURRENT_USER;
#[cfg(target_os = "windows")]
use winreg::RegKey;

use super::DetectedGame;

#[cfg(target_os = "windows")]
pub fn scan_games() -> Vec<DetectedGame> {
    let mut out = Vec::new();
    out.extend(scan_steam());
    out.extend(scan_heroic());

    out.sort_by(|a, b| {
        b.supported
            .cmp(&a.supported)
            .then_with(|| b.engine_hint.is_some().cmp(&a.engine_hint.is_some()))
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
    out
}

#[cfg(target_os = "windows")]
fn scan_steam() -> Vec<DetectedGame> {
    let Some(steam_root) = steam_install_path() else {
        log::info!("Steam install path not found in registry");
        return Vec::new();
    };
    let libraries = steam_library_paths(&steam_root);
    let mut out = Vec::new();
    let mut seen_installs = HashSet::new();
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
            if let Some(game) = parse_appmanifest(&path, &steamapps, &steam_root) {
                if let Some(install) = game.install_path.as_deref() {
                    let key = normalize_windows_path_key(Path::new(install));
                    if !seen_installs.insert(key) {
                        continue;
                    }
                }
                out.push(game);
            }
        }
    }
    out
}

#[cfg(target_os = "windows")]
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
        if p.is_dir() && !libs.iter().any(|e| same_windows_path(e, &p)) {
            libs.push(p);
        }
    }
    libs
}

/// Case- and separator-insensitive path identity for Windows filesystem paths.
fn same_windows_path(a: &Path, b: &Path) -> bool {
    normalize_windows_path_key(a) == normalize_windows_path_key(b)
}

fn normalize_windows_path_key(path: &Path) -> String {
    let s = path.to_string_lossy();
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '/' | '\\' => {
                if !out.ends_with('\\') {
                    out.push('\\');
                }
            }
            c => {
                for lower in c.to_lowercase() {
                    out.push(lower);
                }
            }
        }
    }
    // Trim trailing separators, but keep drive roots like `c:\`.
    while out.len() > 3 && out.ends_with('\\') {
        out.pop();
    }
    out
}

fn parse_appmanifest(acf: &Path, steamapps: &Path, steam_root: &Path) -> Option<DetectedGame> {
    let text = fs::read_to_string(acf).ok()?;
    let name = vdf_string(&text, "name")?;
    let installdir = vdf_string(&text, "installdir")?;
    let appid = vdf_string(&text, "appid");
    let install_path = steamapps.join("common").join(&installdir);
    if !install_path.is_dir() {
        return None;
    }
    let cover_path = appid
        .as_deref()
        .and_then(|id| resolve_steam_box_art(steam_root, id))
        .map(|p| p.to_string_lossy().to_string());
    Some(super::finish_detected(
        name,
        "Steam".to_string(),
        Some(install_path.to_string_lossy().to_string()),
        cover_path,
    ))
}

/// Locate Steam library capsule / box art under `appcache/librarycache`.
///
/// Supports the legacy flat layout (`{appid}_library_600x900.jpg`) and the newer
/// `{appid}/[hash/]library_600x900*.jpg` / `library_capsule.*` layouts.
fn resolve_steam_box_art(steam_root: &Path, app_id: &str) -> Option<PathBuf> {
    let app_id = app_id.trim();
    if app_id.is_empty() {
        return None;
    }
    let cache = steam_root.join("appcache").join("librarycache");
    if !cache.is_dir() {
        return None;
    }

    let legacy = [
        format!("{app_id}_library_600x900.jpg"),
        format!("{app_id}_library_600x900.png"),
        format!("{app_id}_library_capsule.jpg"),
        format!("{app_id}_library_capsule.png"),
    ];
    for name in legacy {
        let path = cache.join(&name);
        if path.is_file() {
            return Some(path);
        }
    }

    let app_dir = cache.join(app_id);
    if app_dir.is_dir() {
        if let Some(found) = find_cover_in_dir(&app_dir, 0) {
            return Some(found);
        }
    }

    // Last-resort icon from the flat cache.
    for name in [format!("{app_id}_icon.jpg"), format!("{app_id}_icon.png")] {
        let path = cache.join(&name);
        if path.is_file() {
            return Some(path);
        }
    }

    None
}

fn find_cover_in_dir(dir: &Path, depth: usize) -> Option<PathBuf> {
    // Prefer portrait library art, then capsule, searching one hash subdirectory deep.
    const MAX_DEPTH: usize = 1;
    let mut capsule: Option<PathBuf> = None;
    let Ok(entries) = fs::read_dir(dir) else {
        return None;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < MAX_DEPTH {
                if let Some(found) = find_cover_in_dir(&path, depth + 1) {
                    // Nested portrait wins immediately; nested capsule only if nothing else.
                    let name = found
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    if name.contains("library_600x900") {
                        return Some(found);
                    }
                    if capsule.is_none() && name.contains("library_capsule") {
                        capsule = Some(found);
                    }
                }
            }
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let lower = name.to_ascii_lowercase();
        if !(lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png")) {
            continue;
        }
        if lower.contains("library_600x900") {
            return Some(path);
        }
        if capsule.is_none() && lower.contains("library_capsule") {
            capsule = Some(path);
        }
    }
    capsule
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
#[cfg(target_os = "windows")]
fn scan_heroic() -> Vec<DetectedGame> {
    let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) else {
        return Vec::new();
    };
    let heroic = appdata.join("heroic");
    if !heroic.is_dir() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut seen_installs = HashSet::new();

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
                    if let Some(ref p) = install_path {
                        let key = normalize_windows_path_key(Path::new(p));
                        if !seen_installs.insert(key) {
                            continue;
                        }
                    }
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
                    if let Some(ref p) = install_path {
                        let key = normalize_windows_path_key(Path::new(p));
                        if !seen_installs.insert(key) {
                            continue;
                        }
                    }
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
        assert_eq!(vdf_string(sample, "appid").as_deref(), Some("413150"));
    }

    #[test]
    fn same_windows_path_ignores_case_and_separators() {
        assert!(same_windows_path(
            Path::new(r"c:\program files (x86)\steam"),
            Path::new(r"C:\Program Files (x86)\Steam"),
        ));
        assert!(same_windows_path(
            Path::new(r"c:/steam"),
            Path::new(r"C:\Steam\"),
        ));
        assert!(!same_windows_path(
            Path::new(r"C:\Steam"),
            Path::new(r"D:\Steam"),
        ));
    }

    #[test]
    fn normalize_trims_trailing_separators() {
        assert_eq!(
            normalize_windows_path_key(Path::new(r"C:\Steam\")),
            normalize_windows_path_key(Path::new(r"c:\steam"))
        );
    }

    #[test]
    fn steam_library_paths_dedupes_cased_root() {
        let root = tempfile::tempdir().unwrap();
        let steamapps = root.path().join("steamapps");
        fs::create_dir_all(&steamapps).unwrap();

        // Simulate VDF listing the same root with different casing/separators.
        let root_str = root.path().to_string_lossy().replace('\\', "\\\\");
        let vdf = format!(
            r#""libraryfolders"
{{
	"0"
	{{
		"path"		"{root_str}"
	}}
}}
"#
        );
        fs::write(steamapps.join("libraryfolders.vdf"), vdf).unwrap();

        // Feed a differently-cased / differently-separated "registry" root that
        // still points at the same temp directory via a second PathBuf spelling.
        let alt_spelling = PathBuf::from(
            root.path()
                .to_string_lossy()
                .replace('/', "\\")
                .to_ascii_lowercase(),
        );
        // If the filesystem is case-sensitive (Linux CI), create a symlink-free
        // path that still compares equal via normalize when separators differ.
        let libs = steam_library_paths(if alt_spelling.is_dir() {
            &alt_spelling
        } else {
            root.path()
        });
        assert_eq!(
            libs.len(),
            1,
            "primary Steam library must appear once: {libs:?}"
        );
    }

    #[test]
    fn resolve_steam_box_art_legacy_flat() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("appcache").join("librarycache");
        fs::create_dir_all(&cache).unwrap();
        let cover = cache.join("413150_library_600x900.jpg");
        fs::write(&cover, b"fake").unwrap();

        assert_eq!(
            resolve_steam_box_art(root.path(), "413150").as_deref(),
            Some(cover.as_path())
        );
    }

    #[test]
    fn resolve_steam_box_art_nested_hash_dir() {
        let root = tempfile::tempdir().unwrap();
        let nested = root
            .path()
            .join("appcache")
            .join("librarycache")
            .join("413150")
            .join("abcdef0123456789");
        fs::create_dir_all(&nested).unwrap();
        let cover = nested.join("library_600x900.jpg");
        fs::write(&cover, b"fake").unwrap();

        assert_eq!(
            resolve_steam_box_art(root.path(), "413150").as_deref(),
            Some(cover.as_path())
        );
    }

    #[test]
    fn resolve_steam_box_art_prefers_portrait_over_capsule() {
        let root = tempfile::tempdir().unwrap();
        let app_dir = root
            .path()
            .join("appcache")
            .join("librarycache")
            .join("730");
        fs::create_dir_all(&app_dir).unwrap();
        let capsule = app_dir.join("library_capsule.jpg");
        let portrait = app_dir.join("library_600x900.jpg");
        fs::write(&capsule, b"cap").unwrap();
        fs::write(&portrait, b"port").unwrap();

        assert_eq!(
            resolve_steam_box_art(root.path(), "730").as_deref(),
            Some(portrait.as_path())
        );
    }

    #[test]
    fn resolve_steam_box_art_missing_returns_none() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("appcache").join("librarycache")).unwrap();
        assert!(resolve_steam_box_art(root.path(), "999").is_none());
    }

    #[test]
    fn parse_appmanifest_sets_cover_from_appid() {
        let root = tempfile::tempdir().unwrap();
        let steamapps = root.path().join("steamapps");
        let common = steamapps.join("common").join("Stardew Valley");
        let cache = root.path().join("appcache").join("librarycache");
        fs::create_dir_all(&common).unwrap();
        fs::create_dir_all(&cache).unwrap();
        let cover = cache.join("413150_library_600x900.jpg");
        fs::write(&cover, b"fake").unwrap();

        let acf = steamapps.join("appmanifest_413150.acf");
        fs::write(
            &acf,
            r#"
"AppState"
{
	"appid"		"413150"
	"name"		"Stardew Valley"
	"installdir"		"Stardew Valley"
}
"#,
        )
        .unwrap();

        let game = parse_appmanifest(&acf, &steamapps, root.path()).expect("game");
        assert_eq!(game.title, "Stardew Valley");
        assert_eq!(
            game.cover_path.as_deref(),
            Some(cover.to_string_lossy().as_ref())
        );
    }
}
