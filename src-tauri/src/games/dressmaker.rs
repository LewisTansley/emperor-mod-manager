//! Dressmaker (Yuuma's Mod Loader).
//!
//! Every directory that contains `mod.json` is a content pack and deploys as
//! `Mods/<Pack>/mod.json`, including a sibling DLL. Wrapper folders above that
//! pack are dropped, and sibling packs in one download each get their own
//! folder. A DLL archive with no `mod.json` deploys under `BepInEx/plugins/`.
//! The loader itself (BepInEx + Doorstop) and game-folder overwrites
//! (`Dressmaker_Data/`, `GameAssembly.dll`) deploy at the install root.

use std::collections::{HashMap, HashSet, VecDeque};
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
            // The pack folder name comes from the directory that holds `mod.json`.
            StagedKind::ContentPack => false,
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
                file_belongs_to_content_pack(ctx.content_root, relative, ctx.source_dir)
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
            StagedKind::ContentPack => {
                content_pack_dest(install_path, ctx.content_root, relative, ctx.source_dir)
            }
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
    if !pack_dirs(content_root).is_empty() {
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

/// Directories that contain `mod.json`, relative to `content_root`.
///
/// An empty path means `mod.json` is at `content_root`.
fn pack_dirs(content_root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut queue = VecDeque::from([content_root.to_path_buf()]);
    while let Some(dir) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children = Vec::new();
        let mut has_manifest = false;
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
                has_manifest = true;
            }
        }
        if has_manifest {
            let rel = dir.strip_prefix(content_root).unwrap_or(Path::new(""));
            found.push(rel.to_path_buf());
        }
        queue.extend(children);
    }
    found
}

/// Staged path before an include filter peeled a prefix.
///
/// `source_dir` is the original parent. The path a plugin sees may only be the
/// file name once that prefix is gone.
fn staged_path(relative: &Path, source_dir: Option<&Path>) -> PathBuf {
    let normalized = normalize_relative(relative);
    match source_dir.map(normalize_relative) {
        Some(dir) if !dir.as_os_str().is_empty() => {
            if let Some(name) = normalized.file_name() {
                return dir.join(name);
            }
        }
        _ => {}
    }
    normalized
}

fn deepest_pack<'a>(packs: &'a [PathBuf], staged: &Path) -> Option<&'a Path> {
    packs
        .iter()
        .filter(|pack| staged.starts_with(pack))
        .max_by_key(|pack| pack.components().count())
        .map(PathBuf::as_path)
}

fn file_belongs_to_content_pack(
    content_root: Option<&Path>,
    relative: &Path,
    source_dir: Option<&Path>,
) -> bool {
    let Some(content_root) = content_root else {
        return true;
    };
    let packs = pack_dirs(content_root);
    if packs.is_empty() {
        return true;
    }
    deepest_pack(&packs, &staged_path(relative, source_dir)).is_some()
}

fn base_pack_name(content_root: &Path, pack_dir: &Path) -> String {
    let raw = if pack_dir.as_os_str().is_empty() {
        content_root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Pack".into())
    } else {
        pack_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Pack".into())
    };
    if raw.is_empty() || raw == "." || raw == ".." || raw.eq_ignore_ascii_case("Mods") {
        "Pack".into()
    } else {
        raw
    }
}

fn pack_names(content_root: &Path, packs: &[PathBuf]) -> HashMap<PathBuf, String> {
    let mut ordered = packs.to_vec();
    ordered.sort();
    let mut used = HashSet::new();
    let mut names = HashMap::new();
    for pack in ordered {
        let base = base_pack_name(content_root, &pack);
        let mut name = base.clone();
        let mut n = 2u32;
        while !used.insert(name.clone()) {
            name = format!("{base}_{n}");
            n += 1;
        }
        names.insert(pack, name);
    }
    names
}

fn push_pack_rest(out: &mut PathBuf, rest: &Path) {
    for component in rest.components() {
        let Component::Normal(part) = component else {
            continue;
        };
        if part.to_string_lossy().eq_ignore_ascii_case("mod.json") {
            out.push("mod.json");
        } else {
            out.push(part);
        }
    }
}

/// Place every directory that contains `mod.json` at `Mods/<Pack>/`.
fn content_pack_dest(
    install_path: &Path,
    content_root: Option<&Path>,
    relative: &Path,
    source_dir: Option<&Path>,
) -> PathBuf {
    let Some(content_root) = content_root else {
        return join_under(install_path, "Mods", relative);
    };
    let packs = pack_dirs(content_root);
    let staged = staged_path(relative, source_dir);
    let Some(pack_dir) = deepest_pack(&packs, &staged) else {
        return install_path.join("Mods");
    };
    let names = pack_names(content_root, &packs);
    let pack_name = names
        .get(pack_dir)
        .cloned()
        .unwrap_or_else(|| "Pack".into());
    let rest = if pack_dir.as_os_str().is_empty() {
        staged.clone()
    } else {
        staged
            .strip_prefix(pack_dir)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| staged.file_name().map(PathBuf::from).unwrap_or_default())
    };
    let mut out = install_path.join("Mods").join(pack_name);
    push_pack_rest(&mut out, &rest);
    out
}

/// Use the Dressmaker plugin when this install is Dressmaker, even if the
/// saved game type is still generic BepInEx. That plugin deploys packs under
/// `BepInEx/plugins/`, which the Mod Manager does not list.
pub(crate) fn plugin_id_for_install<'a>(plugin_id: &'a str, install_path: &Path) -> &'a str {
    if plugin_id == "bepinex" && install_path.join("Dressmaker.exe").is_file() {
        "dressmaker"
    } else {
        plugin_id
    }
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

        assert!(!plugin().should_wrap_as_mod_folder(&root));
        assert!(!plugin().deploys_to_install_root(&root));

        let dest = plugin()
            .resolve_deploy_root_ctx(Path::new("/game"), Path::new("mod.json"), &ctx(&root))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/MyPack/mod.json"));

        let texture = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("textures/cotton.png"),
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

        assert!(!plugin().should_wrap_as_mod_folder(&root));
        assert!(!plugin().deploys_to_install_root(&root));
        let dest = plugin()
            .resolve_deploy_root_ctx(Path::new("/game"), Path::new("Plugin.dll"), &ctx(&root))
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
        assert!(!plugin().should_wrap_as_mod_folder(&root));
        let dest = plugin()
            .resolve_deploy_root_ctx(Path::new("/game"), Path::new("mod.json"), &ctx(&root))
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
    fn sibling_packs_each_land_under_mods() {
        let staging = tempfile::tempdir().unwrap();
        std::fs::write(staging.path().join("README.txt"), b"hi").unwrap();
        for name in ["Cotton", "Linen"] {
            let pack = staging.path().join(name);
            std::fs::create_dir_all(&pack).unwrap();
            std::fs::write(pack.join("mod.json"), b"{}").unwrap();
        }

        let root = staging.path();
        assert!(!plugin().should_wrap_as_mod_folder(root));
        assert!(!plugin().should_deploy_file_ctx(Path::new("README.txt"), &ctx(root)));

        for name in ["Cotton", "Linen"] {
            let dest = plugin()
                .resolve_deploy_root_ctx(
                    Path::new("/game"),
                    &Path::new(name).join("mod.json"),
                    &ctx(root),
                )
                .unwrap();
            assert_eq!(dest, PathBuf::from(format!("/game/Mods/{name}/mod.json")));
        }
    }

    #[test]
    fn same_pack_folder_name_gets_a_suffix() {
        let staging = tempfile::tempdir().unwrap();
        for parent in ["A", "B"] {
            let pack = staging.path().join(parent).join("Pack");
            std::fs::create_dir_all(&pack).unwrap();
            std::fs::write(pack.join("mod.json"), b"{}").unwrap();
        }
        let root = staging.path();
        let first = plugin()
            .resolve_deploy_root_ctx(Path::new("/game"), Path::new("A/Pack/mod.json"), &ctx(root))
            .unwrap();
        let second = plugin()
            .resolve_deploy_root_ctx(Path::new("/game"), Path::new("B/Pack/mod.json"), &ctx(root))
            .unwrap();
        assert_eq!(first, PathBuf::from("/game/Mods/Pack/mod.json"));
        assert_eq!(second, PathBuf::from("/game/Mods/Pack_2/mod.json"));
    }

    #[test]
    fn remapped_include_still_lands_mod_json() {
        let staging = tempfile::tempdir().unwrap();
        let pack = staging.path().join("OptionA").join("Pack");
        std::fs::create_dir_all(&pack).unwrap();
        std::fs::write(pack.join("mod.json"), b"{}").unwrap();
        std::fs::write(pack.join("cotton.png"), b"x").unwrap();
        let root = staging.path();

        // Include filter peels `OptionA/Pack`, leaving the file name. The original
        // parent is still on the deploy context.
        let mapped = DeployContext {
            project_name: None,
            content_root: Some(root),
            source_dir: Some(Path::new("OptionA/Pack")),
        };
        assert!(plugin().should_deploy_file_ctx(Path::new("mod.json"), &mapped));
        assert!(plugin().should_deploy_file_ctx(Path::new("cotton.png"), &mapped));
        assert!(!plugin().should_deploy_file_ctx(Path::new("README.txt"), &ctx(root)));

        let dest = plugin()
            .resolve_deploy_root_ctx(Path::new("/game"), Path::new("mod.json"), &mapped)
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/Pack/mod.json"));
        let texture = plugin()
            .resolve_deploy_root_ctx(Path::new("/game"), Path::new("cotton.png"), &mapped)
            .unwrap();
        assert_eq!(texture, PathBuf::from("/game/Mods/Pack/cotton.png"));
    }

    #[test]
    fn manifest_filename_is_mod_json() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("MyPack");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("Mod.json"), b"{}").unwrap();

        let dest = plugin()
            .resolve_deploy_root_ctx(Path::new("/game"), Path::new("Mod.json"), &ctx(&root))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/MyPack/mod.json"));
    }

    #[test]
    fn bepinex_plugin_id_follows_dressmaker_exe() {
        let game = tempfile::tempdir().unwrap();
        assert_eq!(plugin_id_for_install("bepinex", game.path()), "bepinex");
        std::fs::write(game.path().join("Dressmaker.exe"), b"x").unwrap();
        assert_eq!(plugin_id_for_install("bepinex", game.path()), "dressmaker");
        assert_eq!(
            plugin_id_for_install("dressmaker", game.path()),
            "dressmaker"
        );
        assert_eq!(plugin_id_for_install("starfield", game.path()), "starfield");
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
