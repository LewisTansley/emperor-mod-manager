//! Proton `WINEDLLOVERRIDES` required by native injector DLLs.
//!
//! Scans the install (game root, FromSoftware `Game/`, Unreal `Binaries/Win64`
//! or `WinGDK`) and enabled staged mods. Plugin trees such as `BepInEx/plugins`
//! are skipped so a normal mod DLL is not treated as a Wine proxy.

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
        if winedll_has_native(launch_options, dll) {
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
        .filter(|need| comma_typo(launch_options, &need.dll))
        .map(|need| need.dll.clone())
        .collect();
    let typo = !typo_dlls.is_empty();
    let typo_refs: Vec<&str> = typo_dlls.iter().map(String::as_str).collect();

    let mut merge_dlls = Vec::new();
    for dll in PROXY_DLLS {
        let required = needs.iter().any(|need| need.dll == *dll);
        let already = winedll_has_native(launch_options, dll);
        if required || already {
            merge_dlls.push(*dll);
        }
    }
    let launch_options = merge_fromsoft_winedll_overrides(launch_options, &merge_dlls);
    let summary = summary(&typo_refs, can_apply, steam_note);

    Some(WineOverrideAdvice {
        needs,
        launch_options,
        typo,
        can_apply,
        summary,
    })
}

fn summary(typo_dlls: &[&str], can_apply: bool, steam_note: Option<&str>) -> String {
    let mut summary = String::from(
        "Proton skips these native DLLs unless launch options tell Wine to load them. \
         Without that, the injector never starts and the mods do not run.",
    );
    if let Some(dll) = typo_dlls.first() {
        summary.push_str(&format!(
            " The current options use a comma ({dll},n,b). Wine needs an equals sign ({dll}=n,b)."
        ));
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

fn comma_typo(opts: &str, dll: &str) -> bool {
    let needle = format!("{},n,b", dll.to_ascii_lowercase());
    opts.to_lowercase().contains(&needle)
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
