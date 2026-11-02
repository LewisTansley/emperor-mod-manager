//! Proton `WINEDLLOVERRIDES` required by native injector DLLs.
//!
//! Scans the install (game root, FromSoftware `Game/`, Unreal `Binaries/Win64`
//! or `WinGDK`, No Man's Sky `Binaries/`) and enabled staged mods. Plugin trees
//! such as `BepInEx/plugins` are skipped so a normal mod DLL is not treated as a
//! Wine proxy.

use std::path::{Path, PathBuf};

use serde::Serialize;

use super::fromsoftware::{merge_fromsoft_winedll_overrides, winedll_has_native};
use super::unreal_engine::detect_ue_layout_with_preferred;

/// Proxy DLL stems Wine will not load from the game folder unless launch options
/// prefer the native copy.
const PROXY_DLLS: &[&str] = &[
    "winhttp",
    "version",
    "winmm",
    "dinput8",
    "dxgi",
    "d3d11",
    "d3d12",
    "dwmapi",
    "xinput1_3",
    "xinput1_4",
    "dsound",
    "opengl32",
];

/// Directory names that hold plugins, not the injector that sits next to the exe.
const SKIP_DIR_NAMES: &[&str] = &[
    "bepinex",
    "plugins",
    "patchers",
    "config",
    "core",
    "monomod",
    "cache",
    "mods",
    "melonloader",
    "userlibs",
    "userdata",
    "natives",
    "reframework",
    "pak_mods",
    "content",
    "binaries",
    "paks",
    "~mods",
    "logicmods",
    "data",
    "doorstop_libs",
    "crmods",
];

/// One enabled staged mod whose files may ship an injector.
pub struct StagedProxySource<'a> {
    pub name: &'a str,
    pub path: &'a Path,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WineDllNeed {
    pub dll: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WineOverrideAdvice {
    pub needs: Vec<WineDllNeed>,
    /// Full Steam launch options line, with existing Proton variables kept.
    pub launch_options: String,
    /// True when launch options use `dll,n,b` instead of `dll=n,b`.
    pub typo: bool,
    /// True when a Steam app id was resolved and launch options can be written.
    pub can_apply: bool,
    pub summary: String,
}

/// Advice for the staging page, or `None` when every required DLL already has a
/// native override.
pub fn advise_wine_overrides(
    install_path: &Path,
    project_name: Option<&str>,
    staged: &[StagedProxySource<'_>],
    launch_options: &str,
    can_apply: bool,
    steam_note: Option<&str>,
) -> Option<WineOverrideAdvice> {
    let found = collect_proxy_dlls(install_path, project_name, staged);
    let mut needs = Vec::new();
    for dll in PROXY_DLLS {
        if dll_override_satisfied(launch_options, dll) {
            continue;
        }
        let mut reasons = Vec::new();
        for (found_dll, reason) in &found {
            if found_dll.eq_ignore_ascii_case(dll) && !reasons.iter().any(|r| r == reason) {
                reasons.push(reason.clone());
            }
        }
        if reasons.is_empty() {
            continue;
        }
        needs.push(WineDllNeed {
            dll: (*dll).to_string(),
            reason: reasons.join(", "),
        });
    }
    if needs.is_empty() {
        return None;
    }

    let typo_dlls: Vec<String> = needs
        .iter()
        .filter(|need| override_typo(launch_options, &need.dll))
        .map(|need| need.dll.clone())
        .collect();
    let typo = !typo_dlls.is_empty();
    let typo_refs: Vec<&str> = typo_dlls.iter().map(String::as_str).collect();

    let mut merge_dlls = Vec::new();
    for dll in PROXY_DLLS {
        let required = needs.iter().any(|need| need.dll == *dll);
        let already = dll_override_satisfied(launch_options, dll);
        if required || already {
            merge_dlls.push(*dll);
        }
    }
    let merged_options = merge_fromsoft_winedll_overrides(launch_options, &merge_dlls);
    let summary = summary(&typo_refs, launch_options, can_apply, steam_note);

    Some(WineOverrideAdvice {
        needs,
        launch_options: merged_options,
        typo,
        can_apply,
        summary,
    })
}

fn summary(
    typo_dlls: &[&str],
    launch_options: &str,
    can_apply: bool,
    steam_note: Option<&str>,
) -> String {
    let mut summary = String::from(
        "Proton skips these native DLLs unless launch options tell Wine to load them. \
         Without that, the injector never starts and the mods do not run.",
    );
    if let Some(dll) = typo_dlls.first() {
        if *dll == "winmm" && winmm_native_only(launch_options) {
            summary.push_str(
                " winmm needs WINEDLLOVERRIDES=\"winmm=n,b\" (native and builtin); \
                 winmm=n alone breaks NMS-style winmm proxies.",
            );
        } else if comma_typo(launch_options, dll) {
            summary.push_str(&format!(
                " The current options use a comma ({dll},n,b). Wine needs an equals sign ({dll}=n,b)."
            ));
        }
    }
    if let Some(note) = steam_note {
        summary.push(' ');
        summary.push_str(note);
    } else if !can_apply {
        summary.push_str(
            " This install has no Steam app id. Copy the line into the launcher's launch options.",
        );
    }
    summary
}

fn dll_override_satisfied(opts: &str, dll: &str) -> bool {
    if dll == "winmm" {
        launch_options_have_winmm_proxy_override(opts)
    } else {
        winedll_has_native(opts, dll)
    }
}

fn override_typo(opts: &str, dll: &str) -> bool {
    if dll == "winmm" {
        return comma_typo(opts, dll) || winmm_native_only(opts);
    }
    comma_typo(opts, dll)
}

fn comma_typo(opts: &str, dll: &str) -> bool {
    let needle = format!("{},n,b", dll.to_ascii_lowercase());
    opts.to_lowercase().contains(&needle)
}

/// True when launch options load a mod `winmm.dll` proxy (native **and** builtin).
pub fn launch_options_have_winmm_proxy_override(opts: &str) -> bool {
    if comma_typo(opts, "winmm") {
        return false;
    }
    if !launch_options_have_winmm_native(opts) {
        return false;
    }
    winmm_has_native_and_builtin(opts)
}

/// True when launch options mention a native `winmm` override (may omit builtin `b`).
pub fn launch_options_have_winmm_native(opts: &str) -> bool {
    let lower = opts.to_lowercase();
    if lower.contains("winmm=") {
        if let Some(rest) = lower.split("winmm=").nth(1) {
            let token = rest.split_whitespace().next().unwrap_or("");
            return token.starts_with('n') || token.contains(",n") || token.starts_with("n,");
        }
    }
    if let Some(idx) = lower.find("winedlloverrides") {
        let slice = &lower[idx..];
        if let Some(eq) = slice.find('=') {
            let value = &slice[eq + 1..];
            let value = value.trim_start_matches(|c| c == '"' || c == '\\');
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

fn winmm_has_native_and_builtin(opts: &str) -> bool {
    let lower = opts.to_lowercase();
    if let Some(rest) = lower.split("winmm=").nth(1) {
        let token = rest.split_whitespace().next().unwrap_or("").trim_matches('"');
        return token.contains('n') && token.contains('b');
    }
    if let Some(idx) = lower.find("winedlloverrides") {
        let slice = &lower[idx..];
        if let Some(eq) = slice.find('=') {
            let value = &slice[eq + 1..];
            let value = value.trim_start_matches(|c| c == '"' || c == '\\');
            for part in value.split_whitespace() {
                let part = part.trim_matches('"');
                if let Some((names, order)) = part.split_once('=') {
                    let names = names.trim_matches('\\').trim_matches('"');
                    if names.split(',').any(|n| n.trim() == "winmm") {
                        return order.split(',').any(|o| o.trim() == "n")
                            && order.split(',').any(|o| o.trim() == "b");
                    }
                }
            }
        }
    }
    false
}

fn winmm_native_only(opts: &str) -> bool {
    launch_options_have_winmm_native(opts) && !winmm_has_native_and_builtin(opts)
}

/// Path to a deployed mod `winmm.dll` proxy (game root or NMS `Binaries/`).
pub fn winmm_proxy_at(install_path: &Path) -> Option<PathBuf> {
    let root = install_path.join("winmm.dll");
    if root.is_file() {
        return Some(root);
    }
    let binaries = install_path.join("Binaries").join("winmm.dll");
    if binaries.is_file() {
        return Some(binaries);
    }
    None
}

/// Fix `winmm,n,b` → `winmm=n,b` and ensure an override exists on Linux.
pub fn repair_winmm_override(existing: &str) -> String {
    let mut opts = existing.to_string();
    opts = opts.replace("winmm,n,b", "winmm=n,b");
    opts = opts.replace("WINMM,N,B", "winmm=n,b");
    if launch_options_have_winmm_proxy_override(&opts) {
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

/// Write Steam launch options when a mod `winmm.dll` is present. Linux only.
pub fn ensure_winmm_launch_options(install_path: &Path, success_hint: &str) -> Vec<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (install_path, success_hint);
        Vec::new()
    }
    #[cfg(target_os = "linux")]
    {
        use crate::tools::steam_launch::{read_launch_options, write_launch_options};

        if winmm_proxy_at(install_path).is_none() {
            return Vec::new();
        }
        let app_id = crate::tools::executables::steam_app_id_from_install(
            &install_path.to_string_lossy(),
        );
        let Some(app_id) = app_id else {
            return vec![
                "A mod winmm.dll is deployed but this install has no Steam app id. \
                 Set WINEDLLOVERRIDES=\"winmm=n,b\" %command% in launch options manually."
                    .into(),
            ];
        };
        let existing = match read_launch_options(&app_id) {
            Ok(v) => v.unwrap_or_default(),
            Err(e) => {
                return vec![format!(
                    "Could not read Steam launch options ({e:#}). \
                     Set WINEDLLOVERRIDES=\"winmm=n,b\" %command% manually."
                )];
            }
        };

        if launch_options_have_winmm_proxy_override(&existing) {
            return Vec::new();
        }

        let repaired = repair_winmm_override(&existing);
        match write_launch_options(&app_id, &repaired) {
            Ok(()) => vec![format!(
                "Updated Steam launch options to WINEDLLOVERRIDES=\"winmm=n,b\" \
                 (was missing, used winmm=n only, or had a comma typo). Restart Steam if the \
                 game was already open, then launch again. {success_hint}"
            )],
            Err(e) => vec![format!(
                "Steam launch options need WINEDLLOVERRIDES=\"winmm=n,b\" %command%. \
                 Could not write them automatically ({e:#})."
            )],
        }
    }
}

/// Preflight-style warnings when `winmm.dll` is present but launch options are wrong.
pub fn winmm_launch_option_warnings(install_path: &Path, loader_label: &str) -> Vec<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (install_path, loader_label);
        Vec::new()
    }
    #[cfg(target_os = "linux")]
    {
        use crate::tools::steam_launch::read_launch_options;

        if winmm_proxy_at(install_path).is_none() {
            return Vec::new();
        }
        let app_id = crate::tools::executables::steam_app_id_from_install(
            &install_path.to_string_lossy(),
        );
        let Some(app_id) = app_id else {
            return vec![format!(
                "Proton needs WINEDLLOVERRIDES=\"winmm=n,b\" in Steam launch options or \
                 {loader_label} never loads."
            )];
        };
        let existing = read_launch_options(&app_id)
            .ok()
            .flatten()
            .unwrap_or_default();
        if launch_options_have_winmm_proxy_override(&existing) {
            return Vec::new();
        }
        if winmm_native_only(&existing) {
            return vec![format!(
                "Steam launch options use winmm=n without builtin (b). \
                 NMS-style winmm proxies need WINEDLLOVERRIDES=\"winmm=n,b\" or {loader_label} \
                 will not run."
            )];
        }
        if comma_typo(&existing, "winmm") {
            return vec![format!(
                "Steam launch options have WINEDLLOVERRIDES=\"winmm,n,b\" (comma). \
                 Wine needs WINEDLLOVERRIDES=\"winmm=n,b\" or {loader_label} never runs."
            )];
        }
        vec![format!(
            "Proton needs WINEDLLOVERRIDES=\"winmm=n,b\" in Steam launch options or \
             {loader_label} never runs."
        )]
    }
}

fn collect_proxy_dlls(
    install_path: &Path,
    project_name: Option<&str>,
    staged: &[StagedProxySource<'_>],
) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for dir in install_scan_dirs(install_path, project_name) {
        let reason = install_reason(&dir);
        for dll in direct_proxy_dlls(&dir) {
            push_unique(&mut found, dll, reason.clone());
        }
    }
    for source in staged.iter().filter(|source| source.enabled) {
        let reason = if source.name.trim().is_empty() {
            "staged mod".to_string()
        } else {
            source.name.to_string()
        };
        if source.path.is_file() {
            if let Some(dll) = source
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(proxy_dll_name)
            {
                push_unique(&mut found, dll.to_string(), reason);
            }
            continue;
        }
        let mut dlls = Vec::new();
        for dir in staging_scan_dirs(source.path) {
            for dll in direct_proxy_dlls(&dir) {
                if !dlls.iter().any(|existing| existing == &dll) {
                    dlls.push(dll);
                }
            }
        }
        for dll in dlls {
            push_unique(&mut found, dll, reason.clone());
        }
    }
    found
}

fn push_unique(found: &mut Vec<(String, String)>, dll: String, reason: String) {
    if found
        .iter()
        .any(|(existing_dll, existing_reason)| existing_dll == &dll && existing_reason == &reason)
    {
        return;
    }
    found.push((dll, reason));
}

fn install_scan_dirs(install_path: &Path, project_name: Option<&str>) -> Vec<PathBuf> {
    let mut dirs = vec![install_path.to_path_buf()];
    if let Some(game) = child_dir_ci(install_path, "Game") {
        if game != install_path {
            dirs.push(game);
        }
    }
    if let Some(layout) = detect_ue_layout_with_preferred(install_path, project_name) {
        let binaries = layout.binaries_dir(install_path);
        if binaries.is_dir() && !dirs.iter().any(|dir| dir == &binaries) {
            dirs.push(binaries);
        }
    }
    if let Some(binaries) = child_dir_ci(install_path, "Binaries") {
        if binaries.is_dir() && !dirs.iter().any(|dir| dir == &binaries) {
            dirs.push(binaries);
        }
    }
    dirs
}

fn install_reason(dir: &Path) -> String {
    if child_dir_ci(dir, "MelonLoader").is_some() {
        return "MelonLoader".into();
    }
    if file_exists_ci(dir, "modengine.ini") {
        return "Mod Engine".into();
    }
    if child_dir_ci(dir, "reframework").is_some() {
        return "REFramework".into();
    }
    if file_exists_ci(dir, "doorstop_config.ini")
        || file_exists_ci(dir, ".doorstop_version")
        || child_dir_ci(dir, "doorstop_libs").is_some()
        || child_dir_ci(dir, "BepInEx").is_some()
    {
        return "Doorstop / BepInEx".into();
    }
    if dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("Binaries"))
        && dir
            .parent()
            .is_some_and(|parent| child_dir_ci(parent, "GAMEDATA").is_some())
    {
        return "No Man's Sky mod loader (Binaries/)".into();
    }
    "in the game folder".into()
}

/// Content root, a `BepInExPack*` wrapper, and one other single top-level folder.
fn staging_scan_dirs(content_root: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![content_root.to_path_buf()];
    let Ok(entries) = std::fs::read_dir(content_root) else {
        return dirs;
    };
    let children: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    for child in &children {
        if is_bepinex_pack_dir(child) {
            dirs.push(child.clone());
        }
    }
    let wrappers: Vec<&PathBuf> = children
        .iter()
        .filter(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            !should_skip_dir(name) && !is_bepinex_pack_name(name)
        })
        .collect();
    if wrappers.len() == 1 {
        let wrapper = wrappers[0];
        dirs.push(wrapper.clone());
        if let Ok(inner) = std::fs::read_dir(wrapper) {
            for entry in inner.flatten() {
                let path = entry.path();
                if path.is_dir() && is_bepinex_pack_dir(&path) {
                    dirs.push(path);
                }
            }
        }
    }
    if let Some(binaries) = child_dir_ci(content_root, "Binaries") {
        if !dirs.iter().any(|dir| dir == &binaries) {
            dirs.push(binaries);
        }
    }
    dirs
}

fn direct_proxy_dlls(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dlls = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(dll) = proxy_dll_name(name) else {
            continue;
        };
        if !dlls.iter().any(|existing| existing == dll) {
            dlls.push(dll.to_string());
        }
    }
    dlls
}

fn proxy_dll_name(file_name: &str) -> Option<&'static str> {
    let lower = file_name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".dll")?;
    PROXY_DLLS.iter().copied().find(|dll| *dll == stem)
}

fn is_bepinex_pack_dir(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_bepinex_pack_name)
}

fn is_bepinex_pack_name(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("bepinexpack")
}

fn should_skip_dir(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SKIP_DIR_NAMES.iter().any(|skip| *skip == lower)
}

fn child_dir_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(parent).ok()?;
    entries.flatten().map(|entry| entry.path()).find(|path| {
        path.is_dir()
            && path
                .file_name()
                .and_then(|found| found.to_str())
                .is_some_and(|found| found.eq_ignore_ascii_case(name))
    })
}

fn file_exists_ci(dir: &Path, name: &str) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        path.is_file()
            && path
                .file_name()
                .and_then(|found| found.to_str())
                .is_some_and(|found| found.eq_ignore_ascii_case(name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source<'a>(name: &'a str, path: &'a Path, enabled: bool) -> StagedProxySource<'a> {
        StagedProxySource {
            name,
            path,
            enabled,
        }
    }

    #[test]
    fn staged_bepinex_pack_winhttp_is_attributed_to_the_mod() {
        let install = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let pack = staging.path().join("BepInExPack");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("winhttp.dll"), b"x").unwrap();

        let advice = advise_wine_overrides(
            install.path(),
            None,
            &[source("BepInEx", staging.path(), true)],
            "",
            true,
            None,
        )
        .unwrap();
        assert_eq!(advice.needs.len(), 1);
        assert_eq!(advice.needs[0].dll, "winhttp");
        assert_eq!(advice.needs[0].reason, "BepInEx");
        assert!(advice.launch_options.contains("winhttp=n,b"));
    }

    #[test]
    fn plugin_dll_under_bepinex_plugins_is_ignored() {
        let install = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let plugins = staging.path().join("BepInEx").join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("winhttp.dll"), b"x").unwrap();
        std::fs::write(plugins.join("Foo.dll"), b"x").unwrap();

        assert!(advise_wine_overrides(
            install.path(),
            None,
            &[source("Some Plugin", staging.path(), true)],
            "",
            true,
            None,
        )
        .is_none());
    }

    #[test]
    fn install_root_dinput8_is_required() {
        let install = tempfile::tempdir().unwrap();
        std::fs::write(install.path().join("dinput8.dll"), b"x").unwrap();

        let advice = advise_wine_overrides(install.path(), None, &[], "", true, None).unwrap();
        assert_eq!(advice.needs[0].dll, "dinput8");
        assert_eq!(advice.needs[0].reason, "in the game folder");

        std::fs::write(install.path().join("modengine.ini"), b"x").unwrap();
        let labeled = advise_wine_overrides(install.path(), None, &[], "", true, None).unwrap();
        assert_eq!(labeled.needs[0].reason, "Mod Engine");
    }

    #[test]
    fn already_correct_options_produce_no_advice() {
        let install = tempfile::tempdir().unwrap();
        std::fs::write(install.path().join("dinput8.dll"), b"x").unwrap();
        assert!(advise_wine_overrides(
            install.path(),
            None,
            &[],
            r#"WINEDLLOVERRIDES="dinput8=n,b" %command%"#,
            true,
            None,
        )
        .is_none());
    }

    #[test]
    fn comma_typo_is_reported() {
        let install = tempfile::tempdir().unwrap();
        std::fs::write(install.path().join("dinput8.dll"), b"x").unwrap();
        let advice = advise_wine_overrides(
            install.path(),
            None,
            &[],
            r#"WINEDLLOVERRIDES="dinput8,n,b" %command%"#,
            true,
            None,
        )
        .unwrap();
        assert!(advice.typo);
        assert!(advice.summary.contains("equals sign"));
        assert!(advice.summary.contains("dinput8=n,b"));
        assert!(advice.launch_options.contains("dinput8=n,b"));
    }

    #[test]
    fn merge_keeps_existing_proton_variable() {
        let install = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        std::fs::write(staging.path().join("winhttp.dll"), b"x").unwrap();
        let advice = advise_wine_overrides(
            install.path(),
            None,
            &[source("Doorstop", staging.path(), true)],
            "PROTON_LOG=1 %command%",
            true,
            None,
        )
        .unwrap();
        assert!(advice.launch_options.contains("PROTON_LOG=1"));
        assert!(advice.launch_options.contains("winhttp=n,b"));
        assert!(advice.launch_options.contains("%command%"));
    }

    #[test]
    fn disabled_staged_mod_is_ignored() {
        let install = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        std::fs::write(staging.path().join("winhttp.dll"), b"x").unwrap();
        assert!(advise_wine_overrides(
            install.path(),
            None,
            &[source("Doorstop", staging.path(), false)],
            "",
            true,
            None,
        )
        .is_none());
    }

    #[test]
    fn nms_binaries_winmm_on_install_and_staging() {
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("GAMEDATA/MODS")).unwrap();
        std::fs::create_dir_all(install.path().join("Binaries")).unwrap();
        std::fs::write(install.path().join("Binaries/winmm.dll"), b"x").unwrap();

        let advice = advise_wine_overrides(install.path(), None, &[], "", true, None).unwrap();
        assert_eq!(advice.needs.len(), 1);
        assert_eq!(advice.needs[0].dll, "winmm");
        assert!(advice.needs[0].reason.contains("Binaries"));

        let empty_install = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(staging.path().join("Binaries")).unwrap();
        std::fs::create_dir_all(staging.path().join("GAMEDATA/MODS/AbsoluteFlight")).unwrap();
        std::fs::write(staging.path().join("Binaries/winmm.dll"), b"x").unwrap();
        let staged_advice = advise_wine_overrides(
            empty_install.path(),
            None,
            &[source("Absolute Flight", staging.path(), true)],
            "",
            true,
            None,
        )
        .unwrap();
        assert_eq!(staged_advice.needs[0].dll, "winmm");
        assert_eq!(staged_advice.needs[0].reason, "Absolute Flight");
    }

    #[test]
    fn winmm_n_only_is_not_satisfied() {
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("GAMEDATA")).unwrap();
        std::fs::create_dir_all(install.path().join("Binaries")).unwrap();
        std::fs::write(install.path().join("Binaries/winmm.dll"), b"x").unwrap();
        let advice = advise_wine_overrides(
            install.path(),
            None,
            &[],
            r#"WINEDLLOVERRIDES="winmm=n" %command%"#,
            true,
            None,
        )
        .unwrap();
        assert_eq!(advice.needs[0].dll, "winmm");
        assert!(advice.typo || advice.summary.contains("winmm=n,b"));
    }

    #[test]
    fn winmm_nb_satisfied_produces_no_advice() {
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("GAMEDATA")).unwrap();
        std::fs::create_dir_all(install.path().join("Binaries")).unwrap();
        std::fs::write(install.path().join("Binaries/winmm.dll"), b"x").unwrap();
        assert!(advise_wine_overrides(
            install.path(),
            None,
            &[],
            r#"WINEDLLOVERRIDES="winmm=n,b" %command%"#,
            true,
            None,
        )
        .is_none());
    }

    #[test]
    fn game_subdir_and_unreal_binaries_are_scanned() {
        let install = tempfile::tempdir().unwrap();
        let game = install.path().join("Game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("dxgi.dll"), b"x").unwrap();

        let advice = advise_wine_overrides(install.path(), None, &[], "", true, None).unwrap();
        assert_eq!(advice.needs[0].dll, "dxgi");

        let ue = tempfile::tempdir().unwrap();
        let binaries = ue.path().join("Pal").join("Binaries").join("Win64");
        std::fs::create_dir_all(&binaries).unwrap();
        std::fs::write(binaries.join("dwmapi.dll"), b"x").unwrap();
        let advice = advise_wine_overrides(ue.path(), Some("Pal"), &[], "", false, None).unwrap();
        assert_eq!(advice.needs[0].dll, "dwmapi");
        assert!(!advice.can_apply);
        assert!(advice.summary.contains("no Steam app id"));
    }
}
