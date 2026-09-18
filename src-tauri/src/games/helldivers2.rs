//! Helldivers 2 patch-archive mods.
//!
//! Multiple mods that patch the same archive id must be renamed to sequential
//! `.patch_N` (plus matching `.stream` / `.gpu_resources` sidecars) based on
//! deploy/load order.
//!
//! Mods packaged for Arsenal or HD2MM ship a `manifest.json` describing
//! togglable options; see [`crate::mods::options`] for how a selection turns
//! into the set of folders that actually deploy.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use serde::Deserialize;

use super::{normalize_relative, DeployContext, GamePlugin, GamePluginInfo};
use crate::mods::options::{option_id, sub_option_id, ModOption, ModOptionSet, ModSubOption};

pub const HD2_ROOT_DIRS: &[&str] = &["data", "Data"];

/// Packaging that the game never loads. Arsenal mods carry a manifest and
/// option artwork next to their patch files; linking those into `data/` would
/// litter the game folder.
const NON_GAME_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp", "md", "txt"];

const ROOT_INJECTORS: &[&str] = &[
    "dxgi.dll",
    "d3d11.dll",
    "d3d12.dll",
    "dinput8.dll",
    "version.dll",
    "winmm.dll",
    "opengl32.dll",
    "reshade.ini",
    "ReShade.ini",
];

struct PatchState {
    next_index: HashMap<String, u32>,
    assigned: HashMap<(String, String), u32>,
}

static PATCH_STATE: Mutex<Option<PatchState>> = Mutex::new(None);

/// `PATCH_STATE` is process-global, so tests that deploy Helldivers 2 mods have
/// to take turns or they hand each other the wrong patch indices.
#[cfg(test)]
pub(crate) static TEST_GATE: Mutex<()> = Mutex::new(());

pub struct Helldivers2Plugin;

impl GamePlugin for Helldivers2Plugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "helldivers2",
            display_name: "Helldivers 2",
            nexus_domain: "helldivers2",
            match_names: &["helldivers 2", "helldivers2"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        HD2_ROOT_DIRS
    }

    fn prepare_deploy(&self, _install_path: &Path) -> Result<Vec<String>> {
        reset_patch_state();
        Ok(Vec::new())
    }

    fn mod_options(&self, content_root: &Path) -> Option<ModOptionSet> {
        read_manifest(content_root)
    }

    fn should_deploy_file(&self, relative: &Path) -> bool {
        let Some(name) = relative.file_name().and_then(|n| n.to_str()) else {
            return true;
        };
        if name.eq_ignore_ascii_case("manifest.json") {
            return false;
        }
        relative
            .extension()
            .and_then(|e| e.to_str())
            .map(|ext| {
                !NON_GAME_EXTENSIONS
                    .iter()
                    .any(|n| ext.eq_ignore_ascii_case(n))
            })
            .unwrap_or(true)
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_hd2_deploy(install_path, relative, None, None))
    }

    fn resolve_deploy_root_ctx(
        &self,
        install_path: &Path,
        relative: &Path,
        ctx: &DeployContext<'_>,
    ) -> Result<PathBuf> {
        Ok(resolve_hd2_deploy(
            install_path,
            relative,
            ctx.content_root,
            ctx.source_dir,
        ))
    }
}

/// Arsenal / HD2MM V1 manifest. Every field past `Options` is metadata the
/// manager displays, so a missing or malformed one must not stop a deploy.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawManifest {
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    icon_path: Option<String>,
    #[serde(default)]
    options: Option<RawOptions>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawOptions {
    /// V1: objects with includes and optional sub-options.
    Modern(Vec<RawOption>),
    /// Legacy: bare folder names, one of which the user picks.
    Legacy(Vec<String>),
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawOption {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    include: Option<StringOrList>,
    #[serde(default)]
    sub_options: Option<Vec<RawOption>>,
}

/// Manifests in the wild write `Include` both ways.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StringOrList {
    One(String),
    Many(Vec<String>),
}

impl StringOrList {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(s) => vec![s],
            Self::Many(v) => v,
        }
    }
}

/// `manifest.json` written by any casing; archives are built on Windows.
fn find_manifest(content_root: &Path) -> Option<PathBuf> {
    let direct = content_root.join("manifest.json");
    if direct.is_file() {
        return Some(direct);
    }
    std::fs::read_dir(content_root)
        .ok()?
        .filter_map(|e| e.ok())
        .find(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.eq_ignore_ascii_case("manifest.json"))
        })
        .map(|e| e.path())
}

/// Resolve a manifest-relative image to an absolute path the asset protocol can
/// serve. Dropped when it does not exist, so the UI never shows a broken image.
fn resolve_image(content_root: &Path, raw: Option<String>) -> Option<String> {
    let raw = raw?;
    let rel = normalize_relative(Path::new(&raw));
    if rel.as_os_str().is_empty() || rel.components().any(|c| c == Component::ParentDir) {
        return None;
    }
    let full = content_root.join(rel);
    full.is_file().then(|| full.to_string_lossy().into_owned())
}

fn read_manifest(content_root: &Path) -> Option<ModOptionSet> {
    let path = find_manifest(content_root)?;
    let raw = std::fs::read_to_string(&path).ok()?;
    let manifest: RawManifest = match serde_json::from_str(&raw) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("ignoring malformed {}: {e}", path.display());
            return None;
        }
    };

    let options = match manifest.options {
        Some(RawOptions::Modern(list)) => modern_options(content_root, list),
        Some(RawOptions::Legacy(names)) => legacy_options(content_root, names),
        None => Vec::new(),
    };

    Some(ModOptionSet {
        description: manifest.description.filter(|d| !d.trim().is_empty()),
        icon: resolve_image(content_root, manifest.icon_path),
        options,
    })
}

fn modern_options(content_root: &Path, list: Vec<RawOption>) -> Vec<ModOption> {
    let mut out = Vec::new();
    for (index, raw) in list.into_iter().enumerate() {
        let name = raw
            .name
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("Option {}", index + 1));
        let id = option_id(index, &name);
        let sub_options: Vec<ModSubOption> = raw
            .sub_options
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(sub_index, sub)| {
                let sub_name = sub
                    .name
                    .filter(|n| !n.trim().is_empty())
                    .unwrap_or_else(|| format!("Variant {}", sub_index + 1));
                ModSubOption {
                    id: sub_option_id(&id, sub_index, &sub_name),
                    name: sub_name,
                    description: sub.description.filter(|d| !d.trim().is_empty()),
                    image: resolve_image(content_root, sub.image),
                    include: sub.include.map(StringOrList::into_vec).unwrap_or_default(),
                }
            })
            .collect();

        let include = raw.include.map(StringOrList::into_vec).unwrap_or_default();
        // An option that contributes nothing is a packaging mistake, not a
        // choice worth showing.
        if include.is_empty() && sub_options.is_empty() {
            continue;
        }
        out.push(ModOption {
            id,
            name,
            description: raw.description.filter(|d| !d.trim().is_empty()),
            image: resolve_image(content_root, raw.image),
            include,
            sub_options,
        });
    }
    out
}

/// Legacy manifests list folder names that are alternatives to each other, so
/// they become one always-on option holding every variant.
fn legacy_options(content_root: &Path, names: Vec<String>) -> Vec<ModOption> {
    let sub_options: Vec<ModSubOption> = names
        .into_iter()
        .filter(|n| !n.trim().is_empty())
        .enumerate()
        .map(|(index, folder)| ModSubOption {
            id: sub_option_id("0-variant", index, &folder),
            name: folder.clone(),
            description: None,
            image: resolve_image(content_root, Some(format!("{folder}.png"))),
            include: vec![folder],
        })
        .collect();
    if sub_options.is_empty() {
        return Vec::new();
    }
    vec![ModOption {
        id: "0-variant".into(),
        name: "Variant".into(),
        description: Some("Pick which version of this mod to install.".into()),
        image: None,
        include: Vec::new(),
        sub_options,
    }]
}

fn reset_patch_state() {
    let mut guard = PATCH_STATE.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(PatchState {
        next_index: HashMap::new(),
        assigned: HashMap::new(),
    });
}

/// A `.patch_N` and its `.stream` / `.gpu_resources` sidecars are one unit and
/// must share an index, so the key is the folder they were staged in rather
/// than the mod as a whole: two enabled options can each patch the same archive
/// and those two patches need distinct indices or one overwrites the other.
fn assign_patch_index(
    content_root: Option<&Path>,
    source_dir: Option<&Path>,
    archive_id: &str,
) -> u32 {
    let mut guard = PATCH_STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(|| PatchState {
        next_index: HashMap::new(),
        assigned: HashMap::new(),
    });
    let mut root_key = content_root
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    if let Some(dir) = source_dir.filter(|d| !d.as_os_str().is_empty()) {
        root_key.push('/');
        root_key.push_str(&dir.to_string_lossy());
    }
    let key = (root_key, archive_id.to_string());
    if let Some(&idx) = state.assigned.get(&key) {
        return idx;
    }
    let idx = state.next_index.get(archive_id).copied().unwrap_or(0);
    state
        .next_index
        .insert(archive_id.to_string(), idx.saturating_add(1));
    state.assigned.insert(key, idx);
    idx
}

/// `{archive}.patch_{n}` or `{archive}.patch{n}`, plus optional `.stream` / `.gpu_resources`.
fn parse_patch_name(name: &str) -> Option<(String, String)> {
    let lower = name.to_lowercase();
    let (stem, suffix) = if let Some(rest) = lower.strip_suffix(".gpu_resources") {
        (rest, ".gpu_resources")
    } else if let Some(rest) = lower.strip_suffix(".stream") {
        (rest, ".stream")
    } else {
        (lower.as_str(), "")
    };

    if let Some(idx) = stem.rfind(".patch_") {
        let archive = &stem[..idx];
        let num = &stem[idx + ".patch_".len()..];
        if !archive.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
            return Some((archive.to_string(), suffix.to_string()));
        }
    }
    if let Some(idx) = stem.rfind(".patch") {
        let archive = &stem[..idx];
        let num = &stem[idx + ".patch".len()..];
        if !archive.is_empty() && !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
            return Some((archive.to_string(), suffix.to_string()));
        }
    }
    None
}

fn resolve_hd2_deploy(
    install_path: &Path,
    relative: &Path,
    content_root: Option<&Path>,
    source_dir: Option<&Path>,
) -> PathBuf {
    let normalized = normalize_relative(relative);
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if originals.is_empty() {
        return install_path.join("data");
    }

    let first_lower = originals[0].to_lowercase();
    let leaf = originals.last().map(|s| s.as_str()).unwrap_or("");

    if originals.len() == 1 && ROOT_INJECTORS.iter().any(|n| n.eq_ignore_ascii_case(leaf)) {
        return install_path.join(leaf);
    }

    let skip_data = first_lower == "data";
    let rest: &[String] = if skip_data {
        &originals[1..]
    } else {
        &originals
    };

    if rest.len() == 1 && rest[0].to_lowercase().ends_with(".dl-bin") {
        return install_path.join("data").join("game").join(&rest[0]);
    }
    if rest.len() >= 2 && rest[0].eq_ignore_ascii_case("game") {
        let mut out = install_path.join("data").join("game");
        for part in &rest[1..] {
            out.push(part);
        }
        return out;
    }

    if let Some((archive_id, suffix)) = parse_patch_name(leaf) {
        let idx = assign_patch_index(content_root, source_dir, &archive_id);
        let renamed = format!("{archive_id}.patch_{idx}{suffix}");
        return install_path.join("data").join(renamed);
    }

    let mut out = install_path.join("data");
    for part in rest {
        out.push(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin() -> Helldivers2Plugin {
        Helldivers2Plugin
    }

    #[test]
    fn two_mods_same_archive_get_sequential_indices() {
        let _gate = TEST_GATE.lock().unwrap_or_else(|e| e.into_inner());
        plugin().prepare_deploy(Path::new("/game")).unwrap();

        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let ctx_a = DeployContext {
            content_root: Some(a.path()),
            ..Default::default()
        };
        let ctx_b = DeployContext {
            content_root: Some(b.path()),
            ..Default::default()
        };

        let dest_a = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("9ba626afa44a3aa3.patch_0"),
                &ctx_a,
            )
            .unwrap();
        let dest_a_stream = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("9ba626afa44a3aa3.patch_0.stream"),
                &ctx_a,
            )
            .unwrap();
        let dest_b = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("9ba626afa44a3aa3.patch_0"),
                &ctx_b,
            )
            .unwrap();

        assert_eq!(dest_a, PathBuf::from("/game/data/9ba626afa44a3aa3.patch_0"));
        assert_eq!(
            dest_a_stream,
            PathBuf::from("/game/data/9ba626afa44a3aa3.patch_0.stream")
        );
        assert_eq!(dest_b, PathBuf::from("/game/data/9ba626afa44a3aa3.patch_1"));
    }

    #[test]
    fn dl_bin_goes_to_data_game() {
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("foo.dl-bin"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/data/game/foo.dl-bin"));
    }

    #[test]
    fn injector_at_install_root() {
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("dxgi.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/dxgi.dll"));
    }

    #[test]
    fn data_prefix_stripped() {
        let _gate = TEST_GATE.lock().unwrap_or_else(|e| e.into_inner());
        plugin().prepare_deploy(Path::new("/game")).unwrap();
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("data/abc.patch_0"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/data/abc.patch_0"));
    }

    #[test]
    fn two_options_patching_one_archive_get_distinct_indices() {
        // Options are peeled off before the plugin sees the path, so both
        // arrive as the same name; only source_dir keeps them apart.
        let _gate = TEST_GATE.lock().unwrap_or_else(|e| e.into_inner());
        plugin().prepare_deploy(Path::new("/game")).unwrap();
        let root = tempfile::tempdir().unwrap();

        let dest = |dir: &str, name: &str| {
            let source_dir = PathBuf::from(dir);
            let ctx = DeployContext {
                content_root: Some(root.path()),
                source_dir: Some(&source_dir),
                ..Default::default()
            };
            plugin()
                .resolve_deploy_root_ctx(Path::new("/game"), Path::new(name), &ctx)
                .unwrap()
        };

        assert_eq!(
            dest("Head", "abc.patch_0"),
            PathBuf::from("/game/data/abc.patch_0")
        );
        // Sidecars share their patch file's folder, so they share its index.
        assert_eq!(
            dest("Head", "abc.patch_0.stream"),
            PathBuf::from("/game/data/abc.patch_0.stream")
        );
        assert_eq!(
            dest("Body/Blue", "abc.patch_0"),
            PathBuf::from("/game/data/abc.patch_1")
        );
    }

    #[test]
    fn manifest_and_artwork_are_not_linked_into_data() {
        let p = plugin();
        assert!(!p.should_deploy_file(Path::new("manifest.json")));
        assert!(!p.should_deploy_file(Path::new("icon.png")));
        assert!(!p.should_deploy_file(Path::new("Body/Blue.jpg")));
        assert!(!p.should_deploy_file(Path::new("README.md")));
        assert!(p.should_deploy_file(Path::new("abc.patch_0")));
        assert!(p.should_deploy_file(Path::new("abc.patch_0.stream")));
        assert!(p.should_deploy_file(Path::new("foo.dl-bin")));
    }

    fn write_manifest(dir: &Path, body: &str) {
        std::fs::write(dir.join("manifest.json"), body).unwrap();
    }

    #[test]
    fn parses_v1_manifest_options_and_sub_options() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(
            root.path(),
            r#"{
                "Version": 1,
                "Guid": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                "Name": "Wiki Manifest",
                "Description": "An example mod.",
                "IconPath": "icon.png",
                "Options": [
                    { "Name": "Head", "Description": "Swaps the head.", "Include": ["Head"] },
                    { "Name": "Body", "SubOptions": [
                        { "Name": "Blue", "Include": ["Body/Blue"] },
                        { "Name": "Red", "Include": ["Body/Red"] }
                    ] }
                ]
            }"#,
        );
        std::fs::write(root.path().join("icon.png"), b"x").unwrap();

        let set = plugin().mod_options(root.path()).unwrap();
        assert_eq!(set.description.as_deref(), Some("An example mod."));
        assert_eq!(
            set.icon.as_deref(),
            Some(root.path().join("icon.png").to_string_lossy().as_ref())
        );
        assert_eq!(set.options.len(), 2);

        assert_eq!(set.options[0].name, "Head");
        assert_eq!(set.options[0].include, vec!["Head"]);
        assert!(set.options[0].sub_options.is_empty());

        assert_eq!(set.options[1].name, "Body");
        assert_eq!(set.options[1].sub_options.len(), 2);
        assert_eq!(set.options[1].sub_options[0].name, "Blue");
        assert_eq!(set.options[1].sub_options[1].include, vec!["Body/Red"]);
        // Ids stay distinct across options and sub-options.
        assert_ne!(set.options[0].id, set.options[1].id);
        assert_ne!(
            set.options[1].sub_options[0].id,
            set.options[1].sub_options[1].id
        );
    }

    #[test]
    fn legacy_manifest_becomes_one_group_of_variants() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(
            root.path(),
            r#"{
                "Guid": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
                "Name": "Legacy",
                "Description": "",
                "Options": ["Option A", "Option B"]
            }"#,
        );

        let set = plugin().mod_options(root.path()).unwrap();
        assert_eq!(set.options.len(), 1);
        let group = &set.options[0];
        assert_eq!(group.sub_options.len(), 2);
        assert!(group.include.is_empty());
        assert_eq!(group.sub_options[0].include, vec!["Option A"]);
        assert_eq!(group.sub_options[1].include, vec!["Option B"]);
    }

    #[test]
    fn manifest_quirks_do_not_break_parsing() {
        let no_manifest = tempfile::tempdir().unwrap();
        assert!(plugin().mod_options(no_manifest.path()).is_none());

        let broken = tempfile::tempdir().unwrap();
        write_manifest(broken.path(), "{ not json");
        assert!(plugin().mod_options(broken.path()).is_none());

        // No Options at all: the whole mod deploys, so the set is empty.
        let plain = tempfile::tempdir().unwrap();
        write_manifest(plain.path(), r#"{ "Version": 1, "Name": "Plain" }"#);
        assert!(plugin().mod_options(plain.path()).unwrap().is_empty());

        // A scalar Include, and an option contributing nothing at all.
        let loose = tempfile::tempdir().unwrap();
        write_manifest(
            loose.path(),
            r#"{
                "Version": 1,
                "Options": [
                    { "Name": "Solo", "Include": "Solo" },
                    { "Name": "Empty" }
                ]
            }"#,
        );
        let set = plugin().mod_options(loose.path()).unwrap();
        assert_eq!(set.options.len(), 1);
        assert_eq!(set.options[0].include, vec!["Solo"]);
    }

    #[test]
    fn missing_option_artwork_is_dropped() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(
            root.path(),
            r#"{
                "Version": 1,
                "IconPath": "gone.png",
                "Options": [{ "Name": "Head", "Image": "also-gone.png", "Include": ["Head"] }]
            }"#,
        );
        let set = plugin().mod_options(root.path()).unwrap();
        assert!(set.icon.is_none());
        assert!(set.options[0].image.is_none());
    }
}
