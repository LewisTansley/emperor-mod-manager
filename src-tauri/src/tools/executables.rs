//! Detect game executables and Steam app IDs from install paths.

use std::{
    fs,
    path::{Path, PathBuf},
};

use regex::Regex;
use walkdir::WalkDir;

/// Scan install directory for Windows executables (Proton games).
pub fn detect_executables(install_path: &str, max: usize) -> Vec<String> {
    let root = Path::new(install_path);
    if !root.is_dir() {
        return Vec::new();
    }
    let mut found = Vec::new();
    for entry in WalkDir::new(root).max_depth(4).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if !name.ends_with(".exe") {
            continue;
        }
        let lower = name.to_lowercase();
        if lower.contains("unins")
            || lower.contains("setup")
            || lower.contains("redist")
            || lower.contains("crash")
            || lower.contains("launcher")
            && !lower.contains("shipping")
        {
            continue;
        }
        let exe = name.to_string();
        if !found.iter().any(|e: &String| e.eq_ignore_ascii_case(&exe)) {
            found.push(exe);
        }
        if found.len() >= max {
            break;
        }
    }
    found.sort_by_key(|a| a.to_lowercase());
    found
}

/// Resolve Steam app ID from install path via appmanifest.
pub fn steam_app_id_from_install(install_path: &str) -> Option<String> {
    let install = Path::new(install_path);
    let steamapps = find_steamapps_dir(install)?;
    let common = steamapps.join("common");
    let _ = common;
    let game_name = install.file_name()?.to_str()?;
    let acf_path = find_appmanifest_for_game(&steamapps, game_name)?;
    parse_appmanifest_appid(&acf_path)
}

fn find_steamapps_dir(install_path: &Path) -> Option<PathBuf> {
    let mut cur = Some(install_path);
    while let Some(p) = cur {
        if p.file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("steamapps"))
        {
            return Some(p.to_path_buf());
        }
        cur = p.parent();
    }
    None
}

fn find_appmanifest_for_game(steamapps: &Path, game_folder: &str) -> Option<PathBuf> {
    let re = Regex::new(r#"^\s*"installdir"\s+"([^"]+)"\s*$"#).ok()?;
    for entry in fs::read_dir(steamapps).ok()? {
        let entry = entry.ok()?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
            continue;
        }
        let content = fs::read_to_string(entry.path()).ok()?;
        for line in content.lines() {
            if let Some(cap) = re.captures(line) {
                if cap.get(1).is_some_and(|m| m.as_str() == game_folder) {
                    return Some(entry.path());
                }
            }
        }
    }
    None
}

fn parse_appmanifest_appid(acf_path: &Path) -> Option<String> {
    let content = fs::read_to_string(acf_path).ok()?;
    let re = Regex::new(r#"^\s*"appid"\s+"(\d+)"\s*$"#).ok()?;
    for line in content.lines() {
        if let Some(cap) = re.captures(line) {
            return cap.get(1).map(|m| m.as_str().to_string());
        }
    }
    None
}

pub fn profile_name_for_game(game_id: &str) -> String {
    format!("emm-{game_id}")
}
