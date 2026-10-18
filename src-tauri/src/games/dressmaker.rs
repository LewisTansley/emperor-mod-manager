//! Dressmaker (Yuuma's Mod Loader).
//!
//! A staged tree with `mod.json` is a content pack and deploys as
//! `Mods/<Pack>/mod.json`, including a sibling DLL. Wrapper folders above that
//! pack are dropped. A DLL archive with no `mod.json` deploys under
//! `BepInEx/plugins/`. The loader itself (BepInEx + Doorstop) and game-folder
//! overwrites (`Dressmaker_Data/`, `GameAssembly.dll`) deploy at the install root.

use std::collections::VecDeque;
use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::unity_bepinex::{
    bepinex_plugins_dir, bepinex_prunable_dirs, looks_like_bepinex_pack, resolve_bepinex_deploy,
    should_deploy_bepinex_file, should_wrap_bepinex_mod,
};
use super::{
    content_folder_wrap_name, normalize_relative, DeployContext, GamePlugin, GamePluginInfo,
};

/// Steam AppID (fallback when the install path has no appmanifest).
#[cfg(target_os = "linux")]
const STEAM_APP_ID: &str = "4019220";

const PRESERVE_ROOTS: &[&str] = &[
    "Mods",
    "Dressmaker_Data",
    "BepInEx",
    "BepInExPack",
    "plugins",
    "patchers",
    "config",
    "core",
    "doorstop_libs",
    "dotnet",
    "MonoBleedingEdge",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StagedKind {
    /// Loader pack, game overwrite, or anything the loader does not recognize.
    InstallRoot,
    Plugin,
    ContentPack,
    Assets,
}

pub struct DressmakerPlugin;

impl GamePlugin for DressmakerPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "dressmaker",
            display_name: "Dressmaker",
            nexus_domain: "dressmaker",
            match_names: &["dressmaker"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        PRESERVE_ROOTS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        match staged_kind(content_root) {
            // `mod.json` already inside `Mods/` must not gain another folder.
            StagedKind::ContentPack => content_root.join("mod.json").is_file(),
            StagedKind::Plugin => should_wrap_bepinex_mod(content_root),
            StagedKind::InstallRoot | StagedKind::Assets => false,
        }
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn deploys_to_install_root(&self, content_root: &Path) -> bool {
        staged_kind(content_root) == StagedKind::InstallRoot
    }

    fn should_deploy_file_ctx(&self, relative: &Path, ctx: &DeployContext<'_>) -> bool {
        match ctx.content_root.map(staged_kind) {
            Some(StagedKind::Plugin) => should_deploy_bepinex_file(ctx.content_root, relative),
            Some(StagedKind::ContentPack) => {
                file_belongs_to_content_pack(ctx.content_root, relative)
            }
            _ => true,
        }
    }

    fn prunable_dirs(&self, install_path: &Path) -> Vec<PathBuf> {
        let mut dirs = vec![install_path.join("Mods")];
        dirs.extend(bepinex_prunable_dirs(install_path));
        dirs
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        if mod_loader_present(install_path) {
            Vec::new()
        } else {
            vec![
                "Yuuma's Dressmaker Mod Loader not found (expected BepInEx/plugins/DressmakerMods.dll). Content packs will not load until the loader is installed."
                    .into(),
            ]
        }
    }

    fn staging_deploy_warnings(&self, content_root: &Path, mod_name: &str) -> Vec<String> {
        if staged_kind(content_root) == StagedKind::InstallRoot
            && (looks_like_bepinex_pack(content_root) || looks_like_game_overwrite(content_root))
        {
            vec![format!(
                "{mod_name} looks like the Dressmaker Mod Loader or a game-folder overwrite; files were deployed to the game root."
            )]
        } else {
            Vec::new()
        }
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        self.resolve_deploy_root_ctx(
            install_path,
            relative,
            &DeployContext {
                project_name: None,
                content_root: None,
                source_dir: None,
            },
        )
    }

    fn resolve_deploy_root_ctx(
        &self,
        install_path: &Path,
        relative: &Path,
        ctx: &DeployContext<'_>,
    ) -> Result<PathBuf> {
        let kind = ctx
            .content_root
            .map(staged_kind)
            .unwrap_or(StagedKind::Plugin);
        Ok(match kind {
            StagedKind::ContentPack => content_pack_dest(install_path, ctx.content_root, relative),
            StagedKind::Assets => join_under(install_path, "Dressmaker_Data", relative),
            StagedKind::InstallRoot => install_path.join(normalize_relative(relative)),
            StagedKind::Plugin => return resolve_bepinex_deploy(install_path, relative),
        })
    }

    fn after_deploy(
        &self,
        install_path: &Path,
        _enabled_mod_folders: &[String],
    ) -> Result<Vec<String>> {
        Ok(ensure_winhttp_override(install_path))
    }
}

fn staged_kind(content_root: &Path) -> StagedKind {
    if looks_like_bepinex_pack(content_root) {
        return StagedKind::InstallRoot;
    }
    // The loader's own installer treats any tree with mod.json as a content pack,
    // even when a DLL sits beside it.
    if mod_json_dir(content_root).is_some() {
        return StagedKind::ContentPack;
    }
    if tree_has_file(content_root, |name| name.ends_with(".dll")) {
        return StagedKind::Plugin;
    }
    if looks_like_game_overwrite(content_root) {
        return StagedKind::InstallRoot;
    }
    if tree_has_file(content_root, |name| name.ends_with(".assets")) {
        return StagedKind::Assets;
    }
    StagedKind::InstallRoot
}

/// Directory that directly contains `mod.json`, relative to `content_root`.
///
/// `Some("")` means `mod.json` is at `content_root`. `None` means there is no pack.
fn mod_json_dir(content_root: &Path) -> Option<PathBuf> {
    let mut queue = VecDeque::from([content_root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.eq_ignore_ascii_case("__MACOSX") || name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                children.push(path);
            } else if name.eq_ignore_ascii_case("mod.json") {
                let rel = dir.strip_prefix(content_root).unwrap_or(Path::new(""));
                return Some(rel.to_path_buf());
            }
        }
        queue.extend(children);
    }
    None
}

fn file_belongs_to_content_pack(content_root: Option<&Path>, relative: &Path) -> bool {
    let Some(content_root) = content_root else {
        return true;
    };
    let Some(pack_dir) = mod_json_dir(content_root) else {
        return true;
    };
    if pack_dir.as_os_str().is_empty() {
        return true;
    }
    normalize_relative(relative).starts_with(&pack_dir)
}

/// Place the directory that contains `mod.json` at `Mods/<Pack>/`.
fn content_pack_dest(install_path: &Path, content_root: Option<&Path>, relative: &Path) -> PathBuf {
    let Some(content_root) = content_root else {
        return join_under(install_path, "Mods", relative);
    };
    let Some(pack_dir) = mod_json_dir(content_root) else {
        return join_under(install_path, "Mods", relative);
    };
    if pack_dir.as_os_str().is_empty() {
        // `mod.json` is at the content root, so deploy already wrapped one folder.
        return join_under(install_path, "Mods", relative);
    }
    let pack_name = pack_dir
        .file_name()
        .map(|n| n.to_os_string())
        .filter(|n| !n.to_string_lossy().eq_ignore_ascii_case("Mods"))
        .unwrap_or_else(|| "Pack".into());
    let normalized = normalize_relative(relative);
    let mut out = install_path.join("Mods").join(&pack_name);
    if let Ok(rest) = normalized.strip_prefix(&pack_dir) {
        if !rest.as_os_str().is_empty() {
            out.push(rest);
        }
    }
    out
}

fn looks_like_game_overwrite(content_root: &Path) -> bool {
    content_root.join("GameAssembly.dll").is_file() || content_root.join("Dressmaker_Data").is_dir()
}

fn mod_loader_present(install_path: &Path) -> bool {
    bepinex_plugins_dir(install_path)
        .join("DressmakerMods.dll")
        .is_file()
}

fn tree_has_file(dir: &Path, pred: impl Fn(&str) -> bool) -> bool {
    fn walk(dir: &Path, pred: &impl Fn(&str) -> bool) -> bool {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return false;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.eq_ignore_ascii_case("__MACOSX") || name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if walk(&path, pred) {
                    return true;
                }
            } else if pred(&name.to_lowercase()) {
                return true;
            }
        }
        false
    }
    walk(dir, &pred)
}

/// Join `relative` under `folder`, keeping a single copy of that folder name.
fn join_under(install_path: &Path, folder: &str, relative: &Path) -> PathBuf {
    let normalized = normalize_relative(relative);
    let mut parts = normalized.components().filter_map(|c| match c {
        Component::Normal(s) => Some(s.to_os_string()),
        _ => None,
    });
    let Some(first) = parts.next() else {
        return install_path.join(folder);
    };
    let mut out = install_path.to_path_buf();
    if first.to_string_lossy().eq_ignore_ascii_case(folder) {
        out.push(folder);
    } else {
        out.push(folder);
        out.push(first);
    }
    for part in parts {
        out.push(part);
    }
    out
}

/// True when launch options already force the native `winhttp` override.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) fn launch_options_have_winhttp_native(opts: &str) -> bool {
    let lower = opts.to_lowercase();
    if lower.contains("winhttp=") {
        if let Some(rest) = lower.split("winhttp=").nth(1) {
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
                    if names.split(',').any(|n| n.trim() == "winhttp")
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

/// Prepend `winhttp=n,b` unless launch options already load Doorstop's proxy.
#[cfg_attr(not(any(target_os = "linux", test)), allow(dead_code))]
pub(crate) fn repair_winhttp_override(existing: &str) -> String {
    let mut opts = existing.to_string();
    opts = opts.replace("winhttp,n,b", "winhttp=n,b");
    opts = opts.replace("WINHTTP,N,B", "winhttp=n,b");
    if launch_options_have_winhttp_native(&opts) {
        return opts;
    }
    let trimmed = opts.trim();
    if trimmed.is_empty() {
        return "WINEDLLOVERRIDES=\"winhttp=n,b\" %command%".into();
    }
    if trimmed.contains("%command%") {
        format!("WINEDLLOVERRIDES=\"winhttp=n,b\" {trimmed}")
    } else {
        format!("WINEDLLOVERRIDES=\"winhttp=n,b\" {trimmed} %command%")
    }
}

fn ensure_winhttp_override(install_path: &Path) -> Vec<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = install_path;
        Vec::new()
    }
    #[cfg(target_os = "linux")]
    {
        use crate::tools::steam_launch::{read_launch_options, write_launch_options};

        if !install_path.join("winhttp.dll").is_file() {
            return Vec::new();
        }
        let app_id =
            crate::tools::executables::steam_app_id_from_install(&install_path.to_string_lossy())
                .unwrap_or_else(|| STEAM_APP_ID.to_string());
        let existing = match read_launch_options(&app_id) {
            Ok(v) => v.unwrap_or_default(),
            Err(e) => {
                return vec![format!(
                    "Could not read Steam launch options for Dressmaker ({e:#}). \
                     Set WINEDLLOVERRIDES=\"winhttp=n,b\" %command% manually."
                )];
            }
        };
        if launch_options_have_winhttp_native(&existing) {
            return Vec::new();
        }
        let repaired = repair_winhttp_override(&existing);
        match write_launch_options(&app_id, &repaired) {
            Ok(()) => vec![
                "Updated Steam launch options to WINEDLLOVERRIDES=\"winhttp=n,b\" \
                 so Proton loads Dressmaker's Doorstop (winhttp.dll)."
                    .into(),
            ],
            Err(e) => vec![format!(
                "Steam launch options need WINEDLLOVERRIDES=\"winhttp=n,b\" %command%. \
                 Could not write them automatically ({e:#})."
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin() -> DressmakerPlugin {
        DressmakerPlugin
    }

    fn ctx(root: &Path) -> DeployContext<'_> {
        DeployContext {
            project_name: None,
            content_root: Some(root),
            source_dir: None,
        }
    }

    #[test]
    fn content_pack_wraps_under_mods() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("MyPack");
        std::fs::create_dir_all(root.join("textures")).unwrap();
        std::fs::write(root.join("mod.json"), b"{}").unwrap();
        std::fs::write(root.join("textures").join("cotton.png"), b"x").unwrap();

        assert!(plugin().should_wrap_as_mod_folder(&root));
        assert!(!plugin().deploys_to_install_root(&root));
        let folder = plugin().wrap_mod_folder_name(&root, "Cotton Pack");
        assert_eq!(folder, "MyPack");

        let dest = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                &Path::new(&folder).join("mod.json"),
                &ctx(&root),
            )
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/MyPack/mod.json"));

        let texture = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                &Path::new(&folder).join("textures").join("cotton.png"),
                &ctx(&root),
            )
            .unwrap();
        assert_eq!(
            texture,
            PathBuf::from("/game/Mods/MyPack/textures/cotton.png")
        );
    }

    #[test]
    fn preserves_mods_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let pack = dir.path().join("Mods").join("MyPack");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("mod.json"), b"{}").unwrap();

        assert!(!plugin().should_wrap_as_mod_folder(dir.path()));
        let dest = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("Mods/MyPack/mod.json"),
                &ctx(dir.path()),
            )
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/MyPack/mod.json"));
    }

    #[test]
    fn plugin_dll_deploys_under_bepinex_plugins() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("CoolMod.dll"), b"x").unwrap();

        assert!(plugin().should_wrap_as_mod_folder(dir.path()));
        assert!(!plugin().deploys_to_install_root(dir.path()));
        let dest = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("CoolMod/CoolMod.dll"),
                &ctx(dir.path()),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/BepInEx/plugins/CoolMod/CoolMod.dll")
        );
    }

    #[test]
    fn loader_pack_deploys_to_install_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("BepInEx").join("core")).unwrap();
        std::fs::write(dir.path().join("winhttp.dll"), b"x").unwrap();
        std::fs::create_dir_all(dir.path().join("BepInEx").join("plugins")).unwrap();
        std::fs::write(
            dir.path()
                .join("BepInEx")
                .join("plugins")
                .join("DressmakerMods.dll"),
            b"x",
        )
        .unwrap();

        assert!(plugin().deploys_to_install_root(dir.path()));
        assert!(!plugin().should_wrap_as_mod_folder(dir.path()));
    }

    #[test]
    fn assets_deploy_under_dressmaker_data() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("sharedassets0.assets"), b"x").unwrap();

        assert!(!plugin().deploys_to_install_root(dir.path()));
        assert!(!plugin().should_wrap_as_mod_folder(dir.path()));
        let dest = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("sharedassets0.assets"),
                &ctx(dir.path()),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/Dressmaker_Data/sharedassets0.assets")
        );
    }

    #[test]
    fn dll_stays_inside_content_pack() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("MyPack");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("mod.json"), b"{}").unwrap();
        std::fs::write(root.join("Plugin.dll"), b"x").unwrap();

        assert!(plugin().should_wrap_as_mod_folder(&root));
        assert!(!plugin().deploys_to_install_root(&root));
        let dest = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("MyPack/Plugin.dll"),
                &ctx(&root),
            )
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/MyPack/Plugin.dll"));
    }

    #[test]
    fn pack_folder_lands_directly_under_mods() {
        let staging = tempfile::tempdir().unwrap();
        let pack = staging.path().join("Pack");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("mod.json"), b"{}").unwrap();

        let root = super::super::normalize_staging_root(staging.path(), &plugin()).unwrap();
        assert_eq!(root, pack);
        assert!(plugin().should_wrap_as_mod_folder(&root));
        let folder = plugin().wrap_mod_folder_name(&root, "Ignored Title");
        let dest = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                &Path::new(&folder).join("mod.json"),
                &ctx(&root),
            )
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/Pack/mod.json"));
    }

    #[test]
    fn nested_pack_beside_readme_drops_wrapper() {
        let staging = tempfile::tempdir().unwrap();
        std::fs::write(staging.path().join("README.txt"), b"hi").unwrap();
        let pack = staging.path().join("Outer").join("Pack");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("mod.json"), b"{}").unwrap();
        std::fs::write(pack.join("cotton.png"), b"x").unwrap();

        let root = super::super::normalize_staging_root(staging.path(), &plugin()).unwrap();
        assert_eq!(root, staging.path());
        assert!(!plugin().should_wrap_as_mod_folder(&root));
        assert!(!plugin().should_deploy_file_ctx(Path::new("README.txt"), &ctx(&root)));

        let dest = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("Outer/Pack/mod.json"),
                &ctx(&root),
            )
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/Pack/mod.json"));

        let texture = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("Outer/Pack/cotton.png"),
                &ctx(&root),
            )
            .unwrap();
        assert_eq!(texture, PathBuf::from("/game/Mods/Pack/cotton.png"));
    }

    #[test]
    fn warns_without_mod_loader() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(plugin().preflight_warnings(empty.path()).len(), 1);

        let plugins = empty.path().join("BepInEx").join("plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(plugins.join("DressmakerMods.dll"), b"x").unwrap();
        assert!(plugin().preflight_warnings(empty.path()).is_empty());
    }

    #[test]
    fn repair_winhttp_override_prepends_and_keeps_command() {
        assert_eq!(
            repair_winhttp_override(""),
            "WINEDLLOVERRIDES=\"winhttp=n,b\" %command%"
        );
        assert_eq!(
            repair_winhttp_override("PROTON_LOG=1 %command%"),
            "WINEDLLOVERRIDES=\"winhttp=n,b\" PROTON_LOG=1 %command%"
        );
        assert_eq!(
            repair_winhttp_override("WINEDLLOVERRIDES=\"winhttp=n,b\" %command%"),
            "WINEDLLOVERRIDES=\"winhttp=n,b\" %command%"
        );
        assert_eq!(
            repair_winhttp_override("WINEDLLOVERRIDES=\"winhttp,n,b\" %command%"),
            "WINEDLLOVERRIDES=\"winhttp=n,b\" %command%"
        );
    }
}
