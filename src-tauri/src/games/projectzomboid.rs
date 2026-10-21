//! Project Zomboid local mods.
//!
//! Build 42 reads `mod.info` from `common/` or a version folder (`42`, `42.21`),
//! not from the mod root. Mods live in the user `Zomboid/mods` folder, and the
//! game only loads ids listed in `mods/default.txt`.

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

const STEAM_APP_ID: &str = "108600";
const MANAGED_IDS_FILE: &str = ".emperor-mods.txt";
/// Build 42 version folders score at least this (`42` → 42_000).
const B42_MIN_SCORE: u32 = 42_000;

pub struct ProjectZomboidPlugin;

impl GamePlugin for ProjectZomboidPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "projectzomboid",
            display_name: "Project Zomboid",
            nexus_domain: "projectzomboid",
            match_names: &["project zomboid", "projectzomboid"],
        }
    }

    fn preserves_staging_root(&self, name: &str) -> bool {
        name.eq_ignore_ascii_case("mods")
            || name.eq_ignore_ascii_case("common")
            || version_score(name).is_some()
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        winning_mod_info(content_root).is_some()
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn should_deploy_file(&self, relative: &Path) -> bool {
        relative
            .file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| !name.eq_ignore_ascii_case("workshop.txt"))
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        if zomboid_user_dir(install_path).is_some() {
            Vec::new()
        } else {
            vec![
                "Could not resolve the Project Zomboid user folder (Zomboid/mods). Mods deploy to the install mods folder and may not load until the game has been launched once."
                    .into(),
            ]
        }
    }

    fn staging_deploy_warnings(&self, content_root: &Path, mod_name: &str) -> Vec<String> {
        mod_directories(content_root)
            .into_iter()
            .filter(|dir| is_build41_only(dir))
            .map(|dir| {
                let label = dir
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| mod_name.to_string());
                format!("{label} only has a Build 41 mod.info. Build 42 will not list it.")
            })
            .collect()
    }

    fn load_order_entries(&self, content_root: &Path, _staged_name: &str) -> Vec<String> {
        mod_directories(content_root)
            .into_iter()
            .filter_map(|dir| mod_id_of(&dir))
            .collect()
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_mods_folder(install_path, relative))
    }

    fn after_deploy(
        &self,
        install_path: &Path,
        enabled_mod_folders: &[String],
    ) -> Result<Vec<String>> {
        let mods = mods_dir(install_path);
        std::fs::create_dir_all(&mods).with_context(|| format!("create {}", mods.display()))?;
        write_default_list(&mods, enabled_mod_folders)?;
        Ok(Vec::new())
    }

    fn load_log_sources(&self, install_path: &Path) -> Vec<super::LogSource> {
        let Some(user) = zomboid_user_dir(install_path) else {
            return Vec::new();
        };
        vec![super::LogSource {
            label: "console.txt".into(),
            kind: super::LogKind::Console,
            path: user.join("console.txt"),
        }]
    }
}

fn is_windows_build(install_path: &Path) -> bool {
    install_path.join("ProjectZomboid64.exe").is_file()
        || install_path.join("ProjectZomboid32.exe").is_file()
}

fn native_home() -> Option<PathBuf> {
    #[cfg(windows)]
    let from_env = std::env::var_os("USERPROFILE").map(PathBuf::from);
    #[cfg(not(windows))]
    let from_env = std::env::var_os("HOME").map(PathBuf::from);
    from_env.or_else(|| directories::UserDirs::new().map(|dirs| dirs.home_dir().to_path_buf()))
}

fn zomboid_user_dir(install_path: &Path) -> Option<PathBuf> {
    if is_windows_build(install_path) {
        return windows_build_user_dir(install_path);
    }
    native_home().map(|home| home.join("Zomboid"))
}

#[cfg(windows)]
fn windows_build_user_dir(_install_path: &Path) -> Option<PathBuf> {
    native_home().map(|home| home.join("Zomboid"))
}

#[cfg(not(windows))]
fn windows_build_user_dir(install_path: &Path) -> Option<PathBuf> {
    super::user_data::proton_user_dir(install_path, STEAM_APP_ID)
        .or_else(|| super::user_data::wine_user_dir(install_path))
        .map(|user| user.join("Zomboid"))
}

fn mods_dir(install_path: &Path) -> PathBuf {
    zomboid_user_dir(install_path)
        .map(|dir| dir.join("mods"))
        .unwrap_or_else(|| install_path.join("mods"))
}

fn resolve_mods_folder(install_path: &Path, relative: &Path) -> PathBuf {
    let root = mods_dir(install_path);
    let normalized = normalize_relative(relative);
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_os_string()),
            _ => None,
        })
        .collect();
    if originals.is_empty() {
        return root;
    }
    if originals[0].to_string_lossy().eq_ignore_ascii_case("mods") {
        let mut out = root;
        for part in originals.into_iter().skip(1) {
            out.push(part);
        }
        return out;
    }
    root.join(&normalized)
}

/// `42` → 42000, `42.21` → 42021. Other numeric names are not version folders.
fn version_score(name: &str) -> Option<u32> {
    let mut parts = name.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    if !(41..=99).contains(&major) {
        return None;
    }
    let minor = match parts.next() {
        None => 0,
        Some(value) => {
            if value.is_empty() || !value.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            value.parse::<u32>().ok()?
        }
    };
    if parts.next().is_some() {
        return None;
    }
    Some(major.saturating_mul(1000).saturating_add(minor.min(999)))
}

fn best_version_mod_info(dir: &Path, min_score: u32) -> Option<PathBuf> {
    let mut best: Option<(u32, String, PathBuf)> = None;
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(score) = version_score(&name) else {
            continue;
        };
        if score < min_score {
            continue;
        }
        let info = entry.path().join("mod.info");
        if !info.is_file() {
            continue;
        }
        let replace = match &best {
            None => true,
            Some((best_score, best_name, _)) => {
                score > *best_score || (score == *best_score && name > *best_name)
            }
        };
        if replace {
            best = Some((score, name, info));
        }
    }
    best.map(|(_, _, path)| path)
}

/// Version folder first, then `common/mod.info`, then a Build 41 root file.
fn winning_mod_info(dir: &Path) -> Option<PathBuf> {
    if let Some(path) = best_version_mod_info(dir, B42_MIN_SCORE) {
        return Some(path);
    }
    let common = dir.join("common").join("mod.info");
    if common.is_file() {
        return Some(common);
    }
    let root = dir.join("mod.info");
    if root.is_file() {
        return Some(root);
    }
    best_version_mod_info(dir, 0)
}

fn is_build41_only(dir: &Path) -> bool {
    if dir.join("common").join("mod.info").is_file()
        || best_version_mod_info(dir, B42_MIN_SCORE).is_some()
    {
        return false;
    }
    dir.join("mod.info").is_file() || best_version_mod_info(dir, 0).is_some()
}

fn read_mod_id(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}');
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with(';')
            || line.starts_with("//")
        {
            continue;
        }
        let (key, value) = line.split_once('=')?;
        if key.trim().eq_ignore_ascii_case("id") {
            let id = value.trim().trim_matches('"').trim().to_string();
            if !id.is_empty() {
                return Some(id);
            }
        }
    }
    None
}

fn mod_id_of(dir: &Path) -> Option<String> {
    let info = winning_mod_info(dir)?;
    if let Some(id) = read_mod_id(&info) {
        return Some(id);
    }
    dir.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

fn mod_directories(root: &Path) -> Vec<PathBuf> {
    if winning_mod_info(root).is_some() {
        return vec![root.to_path_buf()];
    }
    let mut found = Vec::new();
    scan_mod_directories(root, 0, &mut found);
    found
}

fn scan_mod_directories(dir: &Path, depth: u32, found: &mut Vec<PathBuf>) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<_> = entries.flatten().collect();
    children.sort_by_key(|entry| entry.file_name());
    for entry in children {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if name.starts_with('.') || name == "__macosx" || name == "common" || name == "media" {
            continue;
        }
        if version_score(&name).is_some() {
            continue;
        }
        let path = entry.path();
        if winning_mod_info(&path).is_some() {
            found.push(path);
        } else if depth < 2 {
            scan_mod_directories(&path, depth + 1, found);
        }
    }
}

struct ModEntry {
    canonical: String,
    token: String,
}

fn canonical_mod_id(token: &str) -> String {
    token.trim().trim_start_matches('\\').trim().to_string()
}

fn parse_mod_entry(line: &str) -> Option<ModEntry> {
    let trimmed = line.trim().strip_suffix(',').unwrap_or(line.trim()).trim();
    let rest = trimmed.strip_prefix("mod")?.trim_start();
    let rest = rest.strip_prefix('=')?.trim().trim_matches('"').trim();
    if rest.is_empty() {
        return None;
    }
    let canonical = canonical_mod_id(rest);
    if canonical.is_empty() {
        return None;
    }
    Some(ModEntry {
        canonical,
        token: rest.to_string(),
    })
}

fn find_block_inner<'a>(text: &'a str, name: &str) -> Option<&'a str> {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index + name.len() <= bytes.len() {
        if text[index..].starts_with(name) {
            let before_ok = index == 0 || !bytes[index - 1].is_ascii_alphanumeric();
            let after = index + name.len();
            let after_ok = after >= bytes.len() || !bytes[after].is_ascii_alphanumeric();
            if before_ok && after_ok {
                let rest = &text[after..];
                if rest.trim_start().starts_with('{') {
                    let brace_at = after + rest.find('{')?;
                    let inner_start = brace_at + 1;
                    let rel_end = text[inner_start..].find('}')?;
                    return Some(&text[inner_start..inner_start + rel_end]);
                }
            }
        }
        index += 1;
    }
    None
}

fn render_default_txt(mods: &[ModEntry], maps_inner: &str) -> String {
    let mut body = String::from("VERSION = 1,\n\nmods\n{\n");
    for entry in mods {
        body.push_str("\tmod = ");
        body.push_str(&entry.token);
        body.push_str(",\n");
    }
    body.push_str("}\n\nmaps\n{");
    if maps_inner.is_empty() {
        body.push('\n');
    } else {
        body.push_str(maps_inner);
        if !maps_inner.ends_with('\n') {
            body.push('\n');
        }
    }
    body.push_str("}\n");
    body
}

fn write_default_list(mods_root: &Path, enabled_ids: &[String]) -> Result<()> {
    let list_path = mods_root.join("default.txt");
    let sidecar_path = mods_root.join(MANAGED_IDS_FILE);
    let existing = std::fs::read_to_string(&list_path).unwrap_or_default();
    let maps_inner = find_block_inner(&existing, "maps").unwrap_or("\n");
    let mut kept: Vec<ModEntry> = find_block_inner(&existing, "mods")
        .map(|inner| inner.lines().filter_map(parse_mod_entry).collect())
        .unwrap_or_default();

    let previous = std::fs::read_to_string(&sidecar_path).unwrap_or_default();
    let previous_ids: Vec<String> = previous
        .lines()
        .map(canonical_mod_id)
        .filter(|id| !id.is_empty())
        .collect();

    let mut enabled = Vec::new();
    for id in enabled_ids {
        let canonical = canonical_mod_id(id);
        if canonical.is_empty() || enabled.iter().any(|seen| seen == &canonical) {
            continue;
        }
        enabled.push(canonical);
    }

    kept.retain(|entry| {
        !previous_ids.iter().any(|id| id == &entry.canonical)
            && !enabled.iter().any(|id| id == &entry.canonical)
    });
    for id in &enabled {
        kept.push(ModEntry {
            canonical: id.clone(),
            token: id.clone(),
        });
    }

    let body = render_default_txt(&kept, maps_inner);
    std::fs::write(&list_path, body).with_context(|| format!("write {}", list_path.display()))?;
    let sidecar = enabled.join("\n");
    let sidecar = if sidecar.is_empty() {
        String::new()
    } else {
        format!("{sidecar}\n")
    };
    std::fs::write(&sidecar_path, sidecar)
        .with_context(|| format!("write {}", sidecar_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::{normalize_staging_root, GamePlugin};

    fn plugin() -> ProjectZomboidPlugin {
        ProjectZomboidPlugin
    }

    fn write_info(dir: &Path, id: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("mod.info"), format!("name=Mod\nid={id}\n")).unwrap();
    }

    #[test]
    fn matches_project_zomboid_title() {
        assert_eq!(
            crate::games::match_plugin("Project Zomboid", None)
                .unwrap()
                .id,
            "projectzomboid"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn b42_wraps_under_proton_zomboid_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp
            .path()
            .join("steamapps")
            .join("common")
            .join("ProjectZomboid");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("ProjectZomboid64.exe"), b"x").unwrap();

        let staged = tempfile::tempdir().unwrap();
        write_info(&staged.path().join("42"), "CoolMod");
        assert!(plugin().should_wrap_as_mod_folder(staged.path()));

        let dest = plugin()
            .resolve_deploy_root(&install, Path::new("CoolMod/42/mod.info"))
            .unwrap();
        assert_eq!(
            dest,
            tmp.path()
                .join("steamapps")
                .join("compatdata")
                .join(STEAM_APP_ID)
                .join("pfx")
                .join("drive_c")
                .join("users")
                .join("steamuser")
                .join("Zomboid")
                .join("mods")
                .join("CoolMod")
                .join("42")
                .join("mod.info")
        );
        assert!(plugin().preflight_warnings(&install).is_empty());
    }

    #[test]
    fn b41_root_mod_info_wraps_and_warns() {
        let dir = tempfile::tempdir().unwrap();
        write_info(dir.path(), "OldMod");
        assert!(plugin().should_wrap_as_mod_folder(dir.path()));
        let warnings = plugin().staging_deploy_warnings(dir.path(), "Old Mod");
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Build 41"));
    }

    #[test]
    fn pack_does_not_wrap_and_keeps_both_folders() {
        let dir = tempfile::tempdir().unwrap();
        write_info(&dir.path().join("ModA").join("42"), "ModAId");
        write_info(&dir.path().join("ModB").join("common"), "ModBId");
        assert!(!plugin().should_wrap_as_mod_folder(dir.path()));
        assert_eq!(
            plugin().load_order_entries(dir.path(), "pack"),
            vec!["ModAId".to_string(), "ModBId".to_string()]
        );

        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("ProjectZomboid");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("ProjectZomboid64.exe"), b"x").unwrap();
        let dest = plugin()
            .resolve_deploy_root(&install, Path::new("mods/ModA/42/mod.info"))
            .unwrap();
        let expected_root = if cfg!(windows) {
            native_home().unwrap().join("Zomboid").join("mods")
        } else {
            install.join("mods")
        };
        assert_eq!(dest, expected_root.join("ModA").join("42").join("mod.info"));
    }

    #[test]
    fn version_and_mods_roots_are_not_peeled() {
        let versioned = tempfile::tempdir().unwrap();
        write_info(&versioned.path().join("42.21"), "Versioned");
        assert_eq!(
            normalize_staging_root(versioned.path(), &plugin()).unwrap(),
            versioned.path()
        );

        let packed = tempfile::tempdir().unwrap();
        write_info(
            &packed.path().join("mods").join("MyMod").join("42"),
            "Packed",
        );
        assert_eq!(
            normalize_staging_root(packed.path(), &plugin()).unwrap(),
            packed.path()
        );
    }

    #[test]
    fn version_folder_id_wins_over_common() {
        let dir = tempfile::tempdir().unwrap();
        write_info(&dir.path().join("common"), "CommonId");
        write_info(&dir.path().join("42.13"), "OldId");
        write_info(&dir.path().join("42.21"), "NewId");
        std::fs::write(dir.path().join("mod.info"), "id=RootId\n").unwrap();
        assert_eq!(
            plugin().load_order_entries(dir.path(), "staged"),
            vec!["NewId".to_string()]
        );
        assert!(plugin()
            .staging_deploy_warnings(dir.path(), "staged")
            .is_empty());
    }

    #[test]
    fn default_txt_keeps_workshop_mods_and_drops_disabled_ids() {
        let mods = tempfile::tempdir().unwrap();
        std::fs::write(
            mods.path().join("default.txt"),
            "VERSION = 1,\n\nmods\n{\n\tmod = WorkshopMod,\n\tmod = \\OldManaged,\n}\n\nmaps\n{\n\tmap = Muldraugh, KY,\n}\n",
        )
        .unwrap();
        std::fs::write(mods.path().join(MANAGED_IDS_FILE), "OldManaged\n").unwrap();

        write_default_list(mods.path(), &["NewMod".into(), "Other".into()]).unwrap();

        let body = std::fs::read_to_string(mods.path().join("default.txt")).unwrap();
        assert!(body.contains("mod = WorkshopMod,"));
        assert!(body.contains("mod = NewMod,"));
        assert!(body.contains("mod = Other,"));
        assert!(!body.contains("OldManaged"));
        assert!(body.contains("map = Muldraugh, KY,"));
        let sidecar = std::fs::read_to_string(mods.path().join(MANAGED_IDS_FILE)).unwrap();
        assert_eq!(sidecar, "NewMod\nOther\n");

        write_default_list(mods.path(), &["Other".into()]).unwrap();
        let body = std::fs::read_to_string(mods.path().join("default.txt")).unwrap();
        assert!(body.contains("mod = WorkshopMod,"));
        assert!(!body.contains("NewMod"));
        assert!(body.contains("mod = Other,"));
        assert!(body.contains("map = Muldraugh, KY,"));
    }

    #[cfg(not(windows))]
    #[test]
    fn native_install_uses_home_even_when_proton_prefix_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp
            .path()
            .join("steamapps")
            .join("common")
            .join("ProjectZomboid");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("ProjectZomboid64"), b"x").unwrap();
        let dest = plugin()
            .resolve_deploy_root(&install, Path::new("Cool/mod.info"))
            .unwrap();
        let home = std::env::var("HOME").unwrap();
        assert_eq!(
            dest,
            PathBuf::from(home)
                .join("Zomboid")
                .join("mods")
                .join("Cool")
                .join("mod.info")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn windows_build_without_prefix_falls_back_to_install_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("ProjectZomboid");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("ProjectZomboid64.exe"), b"x").unwrap();
        let dest = plugin()
            .resolve_deploy_root(&install, Path::new("Cool/42/mod.info"))
            .unwrap();
        assert_eq!(
            dest,
            install
                .join("mods")
                .join("Cool")
                .join("42")
                .join("mod.info")
        );
        assert_eq!(plugin().preflight_warnings(&install).len(), 1);
    }

    #[test]
    fn skips_workshop_txt() {
        assert!(!plugin().should_deploy_file(Path::new("workshop.txt")));
        assert!(plugin().should_deploy_file(Path::new("42/mod.info")));
    }

    #[cfg(not(windows))]
    #[test]
    fn console_log_points_at_proton_user_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp
            .path()
            .join("steamapps")
            .join("common")
            .join("ProjectZomboid");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::write(install.join("ProjectZomboid64.exe"), b"x").unwrap();
        let sources = plugin().load_log_sources(&install);
        assert_eq!(sources.len(), 1);
        assert!(sources[0].path.ends_with("Zomboid/console.txt"));
        assert_eq!(sources[0].kind, super::super::LogKind::Console);
    }
}
