//! CONTROL Resonant (f2g DLL Mod Loader + loose files under data/).

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{
    content_folder_wrap_name, normalize_relative, DeployContext, GamePlugin, GamePluginInfo,
};

pub const CONTROL_RESONANT_ROOT_DIRS: &[&str] = &["crmods", "data"];

/// Steam AppID (fallback when install path has no appmanifest).
const STEAM_APP_ID: &str = "3669870";

/// Loader + known install-root config files from loose-file overhauls.
const ROOT_FILES: &[&str] = &["winmm.dll", "crmods.txt", "renderer.ini", "nvngx.ini"];

pub struct ControlResonantPlugin;

impl GamePlugin for ControlResonantPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "controlresonant",
            display_name: "CONTROL Resonant",
            nexus_domain: "controlresonant",
            match_names: &["control resonant", "controlresonant"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        CONTROL_RESONANT_ROOT_DIRS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        !looks_like_install_layout(content_root) && looks_like_bare_dll_mod(content_root)
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    /// Always resolve through `resolve_deploy_root` so casing and crmods routing apply.
    fn deploys_to_install_root(&self, _content_root: &Path) -> bool {
        false
    }

    fn should_deploy_file(&self, relative: &Path) -> bool {
        !is_junk_deploy_file(relative)
    }

    fn prunable_dirs(&self, install_path: &Path) -> Vec<PathBuf> {
        vec![install_path.join("crmods")]
    }

    fn prepare_deploy(&self, install_path: &Path) -> Result<Vec<String>> {
        Ok(ensure_winmm_override(install_path))
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        let mut warnings = Vec::new();
        if !loader_present(install_path) {
            warnings.push(
                "f2g DLL Mod Loader (winmm.dll) not found in the game folder. \
                 DLL mods will not load until it is installed. On Proton/Steam Deck, \
                 add WINEDLLOVERRIDES=\"winmm=n,b\" %command% to launch options."
                    .into(),
            );
        }
        warnings.extend(winmm_override_warnings(install_path));
        warnings
    }

    fn staging_deploy_warnings(&self, content_root: &Path, mod_name: &str) -> Vec<String> {
        if looks_like_loader_pack(content_root) {
            vec![format!(
                "{mod_name} looks like the f2g DLL Mod Loader; winmm.dll deploys to the game root \
                 and DLL mods under crmods/."
            )]
        } else {
            Vec::new()
        }
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_control_resonant_deploy(
            install_path,
            relative,
            None,
        ))
    }

    fn resolve_deploy_root_ctx(
        &self,
        install_path: &Path,
        relative: &Path,
        ctx: &DeployContext<'_>,
    ) -> Result<PathBuf> {
        Ok(resolve_control_resonant_deploy(
            install_path,
            relative,
            ctx.content_root,
        ))
    }
}

fn loader_present(install_path: &Path) -> bool {
    install_path.join("winmm.dll").is_file()
}

fn looks_like_loader_pack(content_root: &Path) -> bool {
    content_root.join("winmm.dll").is_file() || content_root.join("crmods.txt").is_file()
}

fn dir_named_ignore_case(content_root: &Path, name: &str) -> bool {
    content_root.join(name).is_dir()
        || std::fs::read_dir(content_root).ok().is_some_and(|entries| {
            entries.flatten().any(|e| {
                e.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    && e.file_name().eq_ignore_ascii_case(name)
            })
        })
}

fn looks_like_install_layout(content_root: &Path) -> bool {
    if looks_like_loader_pack(content_root) {
        return true;
    }
    dir_named_ignore_case(content_root, "crmods")
        || dir_named_ignore_case(content_root, "data")
        || dir_named_ignore_case(content_root, "CRLooseFiles")
}

fn looks_like_bare_dll_mod(content_root: &Path) -> bool {
    if looks_like_install_layout(content_root) {
        return false;
    }
    // Only wrap when a DLL sits directly in the content root (e.g. peeled
    // WeaponWheel/WeaponWheel.dll). Nested packs (Outer/Inner/mod.dll) must not
    // wrap or crloader sees crmods/Outer/Inner/... (more than one subfolder).
    top_level_has_dll(content_root)
}

fn top_level_has_dll(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry.file_type().map(|t| t.is_file()).unwrap_or(false)
            && entry
                .file_name()
                .to_string_lossy()
                .to_lowercase()
                .ends_with(".dll")
    })
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

/// True when launch options contain a valid `winmm=…n…` override.
/// Catches the common typo `winmm,n,b` (comma) which Wine ignores.
pub fn launch_options_have_winmm_native(opts: &str) -> bool {
    // dll[=overrides]; accept winmm=n,b or winmm,version=n,b (comma-separated dlls).
    let lower = opts.to_lowercase();
    if lower.contains("winmm=") {
        // winmm=n,b or winmm=n
        if let Some(rest) = lower.split("winmm=").nth(1) {
            let token = rest.split_whitespace().next().unwrap_or("");
            return token.starts_with('n') || token.contains(",n") || token.starts_with("n,");
        }
    }
    // Multi-dll form: winmm,version=n,b
    if let Some(idx) = lower.find("winedlloverrides") {
        let slice = &lower[idx..];
        if let Some(eq) = slice.find('=') {
            let value = &slice[eq + 1..];
            let value = value.trim_start_matches(|c| c == '"' || c == '\\');
            // Find a segment that lists winmm among dll names before '='
            for part in value.split_whitespace() {
                let part = part.trim_matches('"');
                if let Some((names, order)) = part.split_once('=') {
                    let names = names.trim_matches('\\').trim_matches('"');
                    if names.split(',').any(|n| n.trim() == "winmm")
                        && order.split(',').any(|o| o.trim() == "n")
                    {
                        return true;
                    }
                }
            }
        }
    }
    false
}

fn resolve_steam_app_id(install_path: &Path) -> String {
    #[cfg(target_os = "linux")]
    {
        crate::tools::executables::steam_app_id_from_install(&install_path.to_string_lossy())
            .unwrap_or_else(|| STEAM_APP_ID.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = install_path;
        STEAM_APP_ID.to_string()
    }
}

/// Repair `winmm,n,b` → `winmm=n,b` and ensure an override exists on Linux.
fn ensure_winmm_override(install_path: &Path) -> Vec<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = install_path;
        Vec::new()
    }
    #[cfg(target_os = "linux")]
    {
        use crate::tools::steam_launch::{read_launch_options, write_launch_options};

        if !loader_present(install_path) {
            return Vec::new();
        }
        let app_id = resolve_steam_app_id(install_path);
        let existing = match read_launch_options(&app_id) {
            Ok(v) => v.unwrap_or_default(),
            Err(e) => {
                return vec![format!(
                    "Could not read Steam launch options for CONTROL Resonant ({e:#}). \
                     Set WINEDLLOVERRIDES=\"winmm=n,b\" %command% manually."
                )];
            }
        };

        if launch_options_have_winmm_native(&existing) {
            return Vec::new();
        }

        let repaired = repair_winmm_override(&existing);
        match write_launch_options(&app_id, &repaired) {
            Ok(()) => vec![
                "Updated Steam launch options to WINEDLLOVERRIDES=\"winmm=n,b\" \
                 (was missing or used a comma typo like winmm,n,b). Restart Steam if the game \
                 was already open, then launch again — crloader.log should appear next to winmm.dll."
                    .into(),
            ],
            Err(e) => vec![format!(
                "Steam launch options need WINEDLLOVERRIDES=\"winmm=n,b\" %command% \
                 (not winmm,n,b). Could not write them automatically ({e:#})."
            )],
        }
    }
}

fn winmm_override_warnings(install_path: &Path) -> Vec<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = install_path;
        Vec::new()
    }
    #[cfg(target_os = "linux")]
    {
        use crate::tools::steam_launch::read_launch_options;

        if !loader_present(install_path) {
            return Vec::new();
        }
        let app_id = resolve_steam_app_id(install_path);
        let existing = read_launch_options(&app_id)
            .ok()
            .flatten()
            .unwrap_or_default();
        if launch_options_have_winmm_native(&existing) {
            return Vec::new();
        }
        if existing.to_lowercase().contains("winmm,n") {
            return vec![
                "Steam launch options have WINEDLLOVERRIDES=\"winmm,n,b\" (comma). \
                 Wine needs an equals sign: WINEDLLOVERRIDES=\"winmm=n,b\". Without it, \
                 crloader never runs and no crloader.log is written."
                    .into(),
            ];
        }
        vec![
            "Proton needs WINEDLLOVERRIDES=\"winmm=n,b\" in Steam launch options or \
             the f2g loader never runs (no crloader.log)."
                .into(),
        ]
    }
}

/// Fix `winmm,n,b` typo and/or prepend a proper override.
pub fn repair_winmm_override(existing: &str) -> String {
    let mut opts = existing.to_string();
    // Common typo: winmm,n,b instead of winmm=n,b (not the multi-dll form winmm,version=n,b).
    opts = opts.replace("winmm,n,b", "winmm=n,b");
    opts = opts.replace("WINMM,N,B", "winmm=n,b");
    if launch_options_have_winmm_native(&opts) {
        return opts;
    }
    let trimmed = opts.trim();
    if trimmed.is_empty() {
        return "WINEDLLOVERRIDES=\"winmm=n,b\" %command%".into();
    }
    if trimmed.contains("%command%") {
        format!("WINEDLLOVERRIDES=\"winmm=n,b\" {trimmed}")
    } else {
        format!("WINEDLLOVERRIDES=\"winmm=n,b\" {trimmed} %command%")
    }
}

fn path_has_dll(relative: &Path) -> bool {
    relative.components().any(|c| match c {
        Component::Normal(s) => s.to_string_lossy().to_lowercase().ends_with(".dll"),
        _ => false,
    })
}

fn path_components(relative: &Path) -> Vec<std::ffi::OsString> {
    relative
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect()
}

fn canonical_root_file(lower: &str) -> Option<&'static str> {
    ROOT_FILES
        .iter()
        .copied()
        .find(|n| n.eq_ignore_ascii_case(lower))
}

/// Join under install, forcing the first segment to `canonical` when set.
fn join_canonical_first(
    install_path: &Path,
    parts: &[std::ffi::OsString],
    canonical_first: &str,
) -> PathBuf {
    let mut out = install_path.to_path_buf();
    out.push(canonical_first);
    for p in parts.iter().skip(1) {
        out.push(p);
    }
    out
}

fn join_under_crmods(install_path: &Path, under_crmods: &[std::ffi::OsString]) -> PathBuf {
    let clamped = clamp_crmods_rel(under_crmods);
    let mut out = install_path.join("crmods");
    for p in clamped {
        out.push(p);
    }
    out
}

/// crloader only scans `crmods/*.dll` and `crmods/<oneFolder>/*.dll`.
/// Collapse deeper trees to `lastDir/leaf` (companions hoist the same way).
pub fn clamp_crmods_rel(parts: &[std::ffi::OsString]) -> Vec<std::ffi::OsString> {
    if parts.is_empty() {
        return Vec::new();
    }
    let last = parts[parts.len() - 1].clone();
    let last_s = last.to_string_lossy();
    // Treat a final segment with a '.' as a file leaf; otherwise keep as a dir path.
    let last_is_file = last_s.contains('.');
    if !last_is_file {
        if parts.len() <= 1 {
            return parts.to_vec();
        }
        return vec![parts[parts.len() - 1].clone()];
    }
    let dirs = &parts[..parts.len() - 1];
    if dirs.len() <= 1 {
        return parts.to_vec();
    }
    vec![dirs[dirs.len() - 1].clone(), last]
}

fn is_junk_deploy_file(relative: &Path) -> bool {
    let Some(name) = relative.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_lowercase();
    lower.starts_with("changelog")
        || lower.starts_with("readme")
        || lower.starts_with("license")
        || lower.starts_with("sha256sums")
}

fn content_has_dll_under_crmods(content_root: Option<&Path>) -> Option<bool> {
    let root = content_root?;
    // Prefer an actual crmods/CRMods child if present.
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            if entry.file_name().eq_ignore_ascii_case("crmods") {
                return Some(dir_contains_dll(&entry.path()));
            }
        }
    }
    // Content root itself may be the peeled CRMods folder.
    if root
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("crmods"))
    {
        return Some(dir_contains_dll(root));
    }
    None
}

pub fn resolve_control_resonant_deploy(
    install_path: &Path,
    relative: &Path,
    content_root: Option<&Path>,
) -> PathBuf {
    let normalized = normalize_relative(relative);
    let parts = path_components(&normalized);
    if parts.is_empty() {
        return install_path.join("crmods");
    }

    let first_lower = parts[0].to_string_lossy().to_lowercase();

    // Known root files (loader / overhaul configs).
    if let Some(canon) = canonical_root_file(&first_lower) {
        return join_canonical_first(install_path, &parts, canon);
    }

    // Bare CRLooseFiles → data/CRLooseFiles/
    if first_lower == "crloosefiles" {
        let mut out = install_path.join("data").join("CRLooseFiles");
        for p in parts.iter().skip(1) {
            out.push(p);
        }
        return out;
    }

    // data/… — canonicalize first segment only.
    if first_lower == "data" {
        return join_canonical_first(install_path, &parts, "data");
    }

    // crmods / CRMods — DLL loader folder vs data/CRMods loose files.
    if first_lower == "crmods" {
        // Explicit: tree has no DLLs → loose-file pack under data/CRMods.
        if !path_has_dll(&normalized) && content_has_dll_under_crmods(content_root) == Some(false) {
            let mut out = install_path.join("data").join("CRMods");
            for p in parts.iter().skip(1) {
                out.push(p);
            }
            return out;
        }
        // DLL present in path/tree, or ambiguous → crloader folder (lowercase),
        // clamped to at most one subdirectory.
        return join_under_crmods(install_path, &parts[1..]);
    }

    // Bare DLL packs and their sidecars (ini/cfg/cache/…) go under crmods/,
    // clamped so Outer/Inner/mod.dll → crmods/Inner/mod.dll.
    join_under_crmods(install_path, &parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin() -> ControlResonantPlugin {
        ControlResonantPlugin
    }

    #[test]
    fn resolves_crmods_prefix() {
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("crmods/crheal.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/crmods/crheal.dll"));
    }

    #[test]
    fn canonicalizes_crmods_casing() {
        let dest =
            resolve_control_resonant_deploy(Path::new("/game"), Path::new("CRMods/x.dll"), None);
        assert_eq!(dest, PathBuf::from("/game/crmods/x.dll"));
    }

    #[test]
    fn resolves_data_loose_files() {
        let dest = plugin()
            .resolve_deploy_root(
                Path::new("/game"),
                Path::new("data/CRLooseFiles/tweakables.xml"),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/data/CRLooseFiles/tweakables.xml")
        );
    }

    #[test]
    fn bare_crloosefiles_go_under_data() {
        let dest = resolve_control_resonant_deploy(
            Path::new("/game"),
            Path::new("CRLooseFiles/tweakables.xml"),
            None,
        );
        assert_eq!(
            dest,
            PathBuf::from("/game/data/CRLooseFiles/tweakables.xml")
        );
    }

    #[test]
    fn non_dll_crmods_tree_goes_under_data() {
        let staging = tempfile::tempdir().unwrap();
        let crmods = staging.path().join("CRMods");
        std::fs::create_dir_all(&crmods).unwrap();
        std::fs::write(crmods.join("note.txt"), b"x").unwrap();

        let dest = resolve_control_resonant_deploy(
            Path::new("/game"),
            Path::new("CRMods/note.txt"),
            Some(staging.path()),
        );
        assert_eq!(dest, PathBuf::from("/game/data/CRMods/note.txt"));
    }

    #[test]
    fn bare_dll_goes_under_crmods() {
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("crheal.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/crmods/crheal.dll"));
    }

    #[test]
    fn wrapped_bare_pack_keeps_mod_folder() {
        let staging = tempfile::tempdir().unwrap();
        let mod_dir = staging.path().join("WeaponWheel");
        std::fs::create_dir_all(&mod_dir).unwrap();
        std::fs::write(mod_dir.join("WeaponWheel.dll"), b"x").unwrap();
        assert!(plugin().should_wrap_as_mod_folder(&mod_dir));
        assert_eq!(
            plugin().wrap_mod_folder_name(&mod_dir, "WeaponWheel_1_2"),
            "WeaponWheel"
        );
        // After wrap, relative is ModName/file.dll
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("WeaponWheel/WeaponWheel.dll"))
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/crmods/WeaponWheel/WeaponWheel.dll")
        );
    }

    #[test]
    fn nested_only_dll_pack_does_not_wrap() {
        // Outer/Inner/mod.dll — wrapping would create crmods/Outer/Inner/...
        let staging = tempfile::tempdir().unwrap();
        let outer = staging.path().join("VAXIS Mod");
        let inner = outer.join("VAXIS Inner");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("hid.dll"), b"x").unwrap();
        std::fs::write(outer.join("CHANGELOG.txt"), b"x").unwrap();
        assert!(!plugin().should_wrap_as_mod_folder(&outer));
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("VAXIS Inner/hid.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/crmods/VAXIS Inner/hid.dll"));
    }

    #[test]
    fn clamps_over_nested_crmods_paths() {
        // Two+ folders under crmods → keep last folder + leaf (crloader limit).
        assert_eq!(
            resolve_control_resonant_deploy(
                Path::new("/game"),
                Path::new("Outer/VAXIS Graphics/avifil32.dll"),
                None,
            ),
            PathBuf::from("/game/crmods/VAXIS Graphics/avifil32.dll")
        );
        assert_eq!(
            resolve_control_resonant_deploy(
                Path::new("/game"),
                Path::new("Outer/VAXIS Graphics/vaxis_graphics_image.menu.json"),
                None,
            ),
            PathBuf::from("/game/crmods/VAXIS Graphics/vaxis_graphics_image.menu.json")
        );
        assert_eq!(
            resolve_control_resonant_deploy(Path::new("/game"), Path::new("A/B/C/hid.dll"), None,),
            PathBuf::from("/game/crmods/C/hid.dll")
        );
        assert_eq!(
            resolve_control_resonant_deploy(
                Path::new("/game"),
                Path::new("crmods/Outer/Inner/mod.dll"),
                None,
            ),
            PathBuf::from("/game/crmods/Inner/mod.dll")
        );
        // One level stays put.
        assert_eq!(
            resolve_control_resonant_deploy(
                Path::new("/game"),
                Path::new("WeaponWheel/WeaponWheel.dll"),
                None,
            ),
            PathBuf::from("/game/crmods/WeaponWheel/WeaponWheel.dll")
        );
    }

    #[test]
    fn skips_junk_deploy_files() {
        let p = plugin();
        assert!(!p.should_deploy_file(Path::new("CHANGELOG.txt")));
        assert!(!p.should_deploy_file(Path::new("VAXIS/README.md")));
        assert!(!p.should_deploy_file(Path::new("LICENSE")));
        assert!(!p.should_deploy_file(Path::new("SHA256SUMS.txt")));
        assert!(p.should_deploy_file(Path::new("VAXIS Graphics/avifil32.dll")));
        assert!(p.should_deploy_file(Path::new("crheal.ini")));
    }

    #[test]
    fn vaxis_style_pack_one_level() {
        let staging = tempfile::tempdir().unwrap();
        let root = staging.path();
        std::fs::create_dir_all(root.join("VAXIS Graphics")).unwrap();
        std::fs::create_dir_all(root.join("VAXIS PhysX")).unwrap();
        std::fs::write(root.join("VAXIS Graphics").join("avifil32.dll"), b"x").unwrap();
        std::fs::write(root.join("VAXIS PhysX").join("iphlpapi.dll"), b"x").unwrap();
        std::fs::write(root.join("CHANGELOG.txt"), b"x").unwrap();
        assert!(!plugin().should_wrap_as_mod_folder(root));
        assert_eq!(
            resolve_control_resonant_deploy(
                Path::new("/game"),
                Path::new("VAXIS Graphics/avifil32.dll"),
                Some(root),
            ),
            PathBuf::from("/game/crmods/VAXIS Graphics/avifil32.dll")
        );
        assert_eq!(
            resolve_control_resonant_deploy(
                Path::new("/game"),
                Path::new("VAXIS PhysX/iphlpapi.dll"),
                Some(root),
            ),
            PathBuf::from("/game/crmods/VAXIS PhysX/iphlpapi.dll")
        );
    }

    #[test]
    fn detects_winmm_override_syntax() {
        assert!(launch_options_have_winmm_native(
            r#"WINEDLLOVERRIDES="winmm=n,b" %command%"#
        ));
        assert!(launch_options_have_winmm_native(
            r#"WINEDLLOVERRIDES="winmm,version=n,b" %command%"#
        ));
        assert!(!launch_options_have_winmm_native(
            r#"WINEDLLOVERRIDES="winmm,n,b" %command%"#
        ));
        assert!(!launch_options_have_winmm_native(
            "PROTON_ENABLE_WAYLAND=1 %command%"
        ));
    }

    #[test]
    fn repairs_winmm_comma_typo() {
        let fixed = repair_winmm_override(
            r#"PROTON_ENABLE_WAYLAND=1 WINEDLLOVERRIDES="winmm,n,b" %command%"#,
        );
        assert!(launch_options_have_winmm_native(&fixed));
        assert!(fixed.contains("winmm=n,b"));
        assert!(!fixed.contains("winmm,n,b"));
    }

    #[test]
    fn crmods_layout_does_not_wrap() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("crmods").join("WeaponWheel")).unwrap();
        std::fs::write(
            dir.path()
                .join("crmods")
                .join("WeaponWheel")
                .join("WeaponWheel.dll"),
            b"x",
        )
        .unwrap();
        assert!(!plugin().should_wrap_as_mod_folder(dir.path()));
        let dest = plugin()
            .resolve_deploy_root(
                Path::new("/game"),
                Path::new("crmods/WeaponWheel/WeaponWheel.dll"),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/crmods/WeaponWheel/WeaponWheel.dll")
        );
    }

    #[test]
    fn loader_does_not_bypass_resolve() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("winmm.dll"), b"x").unwrap();
        std::fs::write(dir.path().join("extra.dll"), b"x").unwrap();
        assert!(!plugin().deploys_to_install_root(dir.path()));
        assert_eq!(
            plugin()
                .resolve_deploy_root(Path::new("/game"), Path::new("winmm.dll"))
                .unwrap(),
            PathBuf::from("/game/winmm.dll")
        );
        assert_eq!(
            plugin()
                .resolve_deploy_root(Path::new("/game"), Path::new("extra.dll"))
                .unwrap(),
            PathBuf::from("/game/crmods/extra.dll")
        );
    }

    #[test]
    fn root_ini_deploys_to_install_root() {
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("renderer.ini"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/renderer.ini"));
    }

    #[test]
    fn warns_without_loader() {
        let empty = tempfile::tempdir().unwrap();
        let missing = plugin().preflight_warnings(empty.path());
        assert!(
            missing
                .iter()
                .any(|w| w.contains("winmm.dll") && w.contains("not found")),
            "{missing:?}"
        );
        std::fs::write(empty.path().join("winmm.dll"), b"x").unwrap();
        let with_loader = plugin().preflight_warnings(empty.path());
        assert!(
            !with_loader
                .iter()
                .any(|w| w.contains("winmm.dll") && w.contains("not found")),
            "{with_loader:?}"
        );
    }

    #[test]
    fn prunable_crmods() {
        let dirs = plugin().prunable_dirs(Path::new("/game"));
        assert_eq!(dirs, vec![PathBuf::from("/game/crmods")]);
    }
}
