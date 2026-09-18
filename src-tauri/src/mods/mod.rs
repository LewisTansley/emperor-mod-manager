//! Mod staging, extraction, load order, and deploy.

pub mod options;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::{
    config::Paths,
    games::{
        bepinex_pack_deploy_root, normalize_relative, normalize_staging_root, plugin_by_id,
        DeployContext, GamePlugin,
    },
    mods::options::{IncludeFilter, ModOptionSelection},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ModSource {
    #[default]
    Nexus,
    Thunderstore,
    Modio,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedMod {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub source: ModSource,
    #[serde(default)]
    pub nexus_mod_id: u64,
    #[serde(default)]
    pub nexus_file_id: u64,
    pub version: Option<String>,
    /// Nexus domain, or Thunderstore community id.
    pub domain: String,
    pub staging_path: String,
    pub enabled: bool,
    pub order: u32,
    /// Thunderstore package namespace (owner).
    #[serde(default)]
    pub ts_namespace: Option<String>,
    /// Thunderstore package name (without namespace).
    #[serde(default)]
    pub ts_name: Option<String>,
    /// Thunderstore package UUID (optional).
    #[serde(default)]
    pub ts_package_uuid: Option<String>,
    #[serde(default)]
    pub modio_game_id: Option<u32>,
    #[serde(default)]
    pub modio_mod_id: Option<u64>,
    #[serde(default)]
    pub modio_file_id: Option<u64>,
    /// Collections that include this mod as a member or recorded pack dependency.
    #[serde(default)]
    pub collection_ids: Vec<String>,
    /// True when the user installed this outside a collection (browse/import).
    #[serde(default = "default_true")]
    pub independent: bool,
    /// Staged mod ids this mod requires.
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// Files present in staging right after extraction. Staging is immutable, so
    /// a smaller count means something outside Emperor deleted staged files and
    /// the package has to be downloaded again.
    #[serde(default)]
    pub staged_file_count: Option<usize>,
    /// Which of the mod's own options are on. `None` means the mod declares no
    /// options, or the user has not touched the defaults yet.
    #[serde(default)]
    pub option_selection: Option<ModOptionSelection>,
}

fn default_true() -> bool {
    true
}

impl Default for StagedMod {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            source: ModSource::Nexus,
            nexus_mod_id: 0,
            nexus_file_id: 0,
            version: None,
            domain: String::new(),
            staging_path: String::new(),
            enabled: true,
            order: 0,
            ts_namespace: None,
            ts_name: None,
            ts_package_uuid: None,
            modio_game_id: None,
            modio_mod_id: None,
            modio_file_id: None,
            collection_ids: Vec::new(),
            independent: true,
            depends_on: Vec::new(),
            staged_file_count: None,
            option_selection: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CollectionSource {
    #[default]
    Nexus,
    Thunderstore,
    Emperor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CollectionKind {
    #[default]
    Collection,
    Modpack,
    Profile,
    Share,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledCollection {
    pub id: String,
    pub source: CollectionSource,
    pub kind: CollectionKind,
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub package_name: Option<String>,
    #[serde(default)]
    pub community: Option<String>,
    #[serde(default)]
    pub revision: Option<i64>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub profile_code: Option<String>,
    #[serde(default)]
    pub mod_ids: Vec<String>,
    #[serde(default)]
    pub installed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstalledCollections {
    pub collections: Vec<InstalledCollection>,
}

#[derive(Debug, Clone, Default)]
struct StagingProvenance {
    collection_ids: Vec<String>,
    independent: bool,
    depends_on: Vec<String>,
    enabled: bool,
    order: Option<u32>,
    option_selection: Option<ModOptionSelection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LoadOrder {
    pub mods: Vec<StagedMod>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeployManifest {
    /// Absolute paths we created/linked during last deploy.
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeployResult {
    pub file_count: usize,
    pub enabled_mods: usize,
    pub warnings: Vec<String>,
}

pub fn load_loadorder(paths: &Paths, game_id: &str) -> Result<LoadOrder> {
    let file = paths.loadorder_file(game_id);
    if !file.exists() {
        return Ok(LoadOrder::default());
    }
    let raw = fs::read_to_string(&file)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save_loadorder(paths: &Paths, game_id: &str, order: &LoadOrder) -> Result<()> {
    crate::config::ensure_game_dirs(paths, game_id)?;
    let file = paths.loadorder_file(game_id);
    let raw = serde_json::to_string_pretty(order)?;
    fs::write(file, raw)?;
    Ok(())
}

pub fn load_collections(paths: &Paths, game_id: &str) -> Result<InstalledCollections> {
    let file = paths.collections_file(game_id);
    if !file.exists() {
        return Ok(InstalledCollections::default());
    }
    let raw = fs::read_to_string(&file)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save_collections(
    paths: &Paths,
    game_id: &str,
    collections: &InstalledCollections,
) -> Result<()> {
    crate::config::ensure_game_dirs(paths, game_id)?;
    let file = paths.collections_file(game_id);
    let raw = serde_json::to_string_pretty(collections)?;
    fs::write(file, raw)?;
    Ok(())
}

pub fn thunderstore_mod_id(namespace: &str, name: &str) -> String {
    format!("ts_{namespace}_{name}")
}

pub fn modio_identity_id(modio_game_id: u32, modio_mod_id: u64) -> String {
    format!("modio_{modio_game_id}_{modio_mod_id}")
}

pub fn nexus_collection_id(slug: &str) -> String {
    format!("nexus:{slug}")
}

pub fn thunderstore_modpack_id(community: &str, namespace: &str, name: &str) -> String {
    format!("ts-modpack:{community}:{namespace}-{name}")
}

pub fn thunderstore_profile_id(code: &str) -> String {
    format!("ts-profile:{code}")
}

fn parse_version_parts(raw: &str) -> Option<Vec<u64>> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    let mut saw_digit = false;
    for token in s.split(|c: char| !c.is_ascii_digit()) {
        if token.is_empty() {
            continue;
        }
        saw_digit = true;
        parts.push(token.parse().ok()?);
    }
    if saw_digit {
        Some(parts)
    } else {
        None
    }
}

/// True when `incoming` should replace `existing` (newest-wins).
pub fn incoming_is_newer(existing: Option<&str>, incoming: &str) -> bool {
    let incoming = incoming.trim();
    if incoming.is_empty() {
        return false;
    }
    let Some(existing) = existing.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    if existing == incoming {
        return false;
    }
    match (parse_version_parts(existing), parse_version_parts(incoming)) {
        (Some(a), Some(b)) => {
            let n = a.len().max(b.len());
            for i in 0..n {
                let x = a.get(i).copied().unwrap_or(0);
                let y = b.get(i).copied().unwrap_or(0);
                if y != x {
                    return y > x;
                }
            }
            false
        }
        (None, None) => true,
        (Some(_), None) => false,
        (None, Some(_)) => true,
    }
}

/// Minimal Nexus file metadata for update candidate selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NexusFileMeta {
    pub file_id: u64,
    pub version: Option<String>,
    pub category_name: Option<String>,
    pub uploaded_timestamp: Option<u64>,
    pub is_primary: bool,
}

/// Available remote update for a staged mod.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedModUpdate {
    pub staged_id: String,
    pub available_version: Option<String>,
    pub source: ModSource,
    #[serde(default)]
    pub nexus_file_id: Option<u64>,
    #[serde(default)]
    pub ts_version: Option<String>,
    #[serde(default)]
    pub modio_file_id: Option<u64>,
}

fn category_key(name: Option<&str>) -> String {
    name.unwrap_or("").trim().to_lowercase()
}

fn is_main_category(name: Option<&str>) -> bool {
    let k = category_key(name);
    k.is_empty() || k.contains("main")
}

fn nexus_file_is_newer(candidate: &NexusFileMeta, baseline: &NexusFileMeta) -> bool {
    match (candidate.uploaded_timestamp, baseline.uploaded_timestamp) {
        (Some(ta), Some(tb)) if ta != tb => ta > tb,
        _ => candidate.file_id > baseline.file_id,
    }
}

fn newest_nexus_file<'a, F>(files: &'a [NexusFileMeta], pred: F) -> Option<&'a NexusFileMeta>
where
    F: Fn(&NexusFileMeta) -> bool,
{
    files.iter().filter(|f| pred(f)).max_by(|a, b| {
        a.uploaded_timestamp
            .cmp(&b.uploaded_timestamp)
            .then_with(|| a.file_id.cmp(&b.file_id))
    })
}

/// Pick a newer Nexus file that should replace the staged file, if any.
pub fn nexus_update_candidate<'a>(
    staged_file_id: u64,
    staged_version: Option<&str>,
    files: &'a [NexusFileMeta],
) -> Option<&'a NexusFileMeta> {
    if files.is_empty() {
        return None;
    }
    let staged = files.iter().find(|f| f.file_id == staged_file_id);
    let candidate = if let Some(staged) = staged {
        let cat = category_key(staged.category_name.as_deref());
        newest_nexus_file(files, |f| category_key(f.category_name.as_deref()) == cat)
            .or_else(|| newest_nexus_file(files, |f| f.is_primary))
    } else {
        newest_nexus_file(files, |f| f.is_primary)
            .or_else(|| newest_nexus_file(files, |f| is_main_category(f.category_name.as_deref())))
            .or_else(|| newest_nexus_file(files, |_| true))
    }?;

    if candidate.file_id == staged_file_id {
        return None;
    }

    let version_newer = candidate
        .version
        .as_deref()
        .filter(|v| !v.is_empty())
        .map(|v| incoming_is_newer(staged_version, v))
        .unwrap_or(false);
    let upload_newer = match staged {
        Some(s) => nexus_file_is_newer(candidate, s),
        None => true,
    };
    if version_newer || upload_newer {
        Some(candidate)
    } else {
        None
    }
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.iter().any(|s| s == &value) {
        list.push(value);
    }
}

fn rewrite_depends_on(order: &mut LoadOrder, old_id: &str, new_id: &str) {
    if old_id == new_id {
        return;
    }
    for m in &mut order.mods {
        for dep in &mut m.depends_on {
            if dep == old_id {
                *dep = new_id.to_string();
            }
        }
        let mut seen = HashSet::new();
        m.depends_on
            .retain(|s| !s.is_empty() && seen.insert(s.clone()));
    }
}

fn provenance_from_removed(old: Option<StagedMod>, new_staging: &Path) -> StagingProvenance {
    let Some(old) = old else {
        return StagingProvenance {
            independent: true,
            enabled: true,
            ..Default::default()
        };
    };
    let old_path = PathBuf::from(&old.staging_path);
    if old_path != new_staging && old_path.exists() {
        let _ = fs::remove_dir_all(&old_path);
    }
    StagingProvenance {
        collection_ids: old.collection_ids,
        independent: old.independent,
        depends_on: old.depends_on,
        enabled: old.enabled,
        order: Some(old.order),
        // Option ids are slugs of the manifest entries, so an update that keeps
        // the same options keeps the user's picks. Anything the new manifest
        // dropped is discarded when the selection is normalized.
        option_selection: old.option_selection,
    }
}

fn take_row_by_id(order: &mut LoadOrder, id: &str) -> Option<StagedMod> {
    order
        .mods
        .iter()
        .position(|m| m.id == id)
        .map(|pos| order.mods.remove(pos))
}

fn take_nexus_row(order: &mut LoadOrder, mod_id: u64, file_id: u64) -> Option<StagedMod> {
    order
        .mods
        .iter()
        .position(|m| {
            m.source == ModSource::Nexus && m.nexus_mod_id == mod_id && m.nexus_file_id == file_id
        })
        .map(|pos| order.mods.remove(pos))
}

fn take_modio_row(
    order: &mut LoadOrder,
    modio_game_id: u32,
    modio_mod_id: u64,
) -> Option<StagedMod> {
    order
        .mods
        .iter()
        .position(|m| {
            m.source == ModSource::Modio
                && m.modio_game_id == Some(modio_game_id)
                && m.modio_mod_id == Some(modio_mod_id)
        })
        .map(|pos| order.mods.remove(pos))
}

pub fn find_thunderstore_package<'a>(
    order: &'a LoadOrder,
    namespace: &str,
    name: &str,
) -> Option<&'a StagedMod> {
    order.mods.iter().find(|m| {
        m.source == ModSource::Thunderstore
            && m.ts_namespace.as_deref() == Some(namespace)
            && m.ts_name.as_deref() == Some(name)
    })
}

pub fn find_modio_mod<'a>(
    order: &'a LoadOrder,
    modio_game_id: u32,
    modio_mod_id: u64,
) -> Option<&'a StagedMod> {
    order.mods.iter().find(|m| {
        m.source == ModSource::Modio
            && m.modio_game_id == Some(modio_game_id)
            && m.modio_mod_id == Some(modio_mod_id)
    })
}

pub fn find_nexus_file<'a>(
    order: &'a LoadOrder,
    nexus_mod_id: u64,
    nexus_file_id: u64,
) -> Option<&'a StagedMod> {
    order.mods.iter().find(|m| {
        m.source == ModSource::Nexus
            && m.nexus_mod_id == nexus_mod_id
            && m.nexus_file_id == nexus_file_id
    })
}

pub fn merge_mod_meta(
    paths: &Paths,
    game_id: &str,
    mod_uid: &str,
    extra_collection_ids: &[String],
    extra_depends_on: &[String],
    independent: Option<bool>,
    enabled: Option<bool>,
) -> Result<Option<StagedMod>> {
    let mut order = load_loadorder(paths, game_id)?;
    let Some(m) = order.mods.iter_mut().find(|m| m.id == mod_uid) else {
        return Ok(None);
    };
    for id in extra_collection_ids {
        push_unique(&mut m.collection_ids, id.clone());
    }
    for id in extra_depends_on {
        push_unique(&mut m.depends_on, id.clone());
    }
    if let Some(flag) = independent {
        m.independent = flag;
    }
    if let Some(flag) = enabled {
        m.enabled = flag;
    }
    let cloned = m.clone();
    save_loadorder(paths, game_id, &order)?;
    Ok(Some(cloned))
}

pub fn record_installed_collection(
    paths: &Paths,
    game_id: &str,
    mut collection: InstalledCollection,
    new_mod_ids: &[String],
) -> Result<InstalledCollection> {
    if collection.installed_at.is_empty() {
        collection.installed_at = chrono::Utc::now().to_rfc3339();
    }
    let mut order = load_loadorder(paths, game_id)?;
    let new_set: HashSet<&str> = new_mod_ids.iter().map(|s| s.as_str()).collect();
    for m in &mut order.mods {
        if collection.mod_ids.iter().any(|id| id == &m.id) {
            push_unique(&mut m.collection_ids, collection.id.clone());
            if new_set.contains(m.id.as_str()) {
                m.independent = false;
            }
        }
    }
    save_loadorder(paths, game_id, &order)?;

    let mut store = load_collections(paths, game_id)?;
    if let Some(existing) = store.collections.iter_mut().find(|c| c.id == collection.id) {
        let mut ids = existing.mod_ids.clone();
        for id in &collection.mod_ids {
            push_unique(&mut ids, id.clone());
        }
        collection.mod_ids = ids;
        *existing = collection.clone();
    } else {
        store.collections.push(collection.clone());
    }
    save_collections(paths, game_id, &store)?;
    Ok(collection)
}

fn drop_collection_from_mods(order: &mut LoadOrder, collection_id: &str) {
    for m in &mut order.mods {
        m.collection_ids.retain(|id| id != collection_id);
    }
}

fn reachable_mod_ids(order: &LoadOrder, store: &InstalledCollections) -> HashSet<String> {
    let mut keep: HashSet<String> = HashSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    for m in &order.mods {
        if m.independent && keep.insert(m.id.clone()) {
            queue.push_back(m.id.clone());
        }
    }
    for c in &store.collections {
        for id in &c.mod_ids {
            if order.mods.iter().any(|m| &m.id == id) && keep.insert(id.clone()) {
                queue.push_back(id.clone());
            }
        }
    }
    let by_id: HashMap<&str, &StagedMod> = order.mods.iter().map(|m| (m.id.as_str(), m)).collect();
    while let Some(id) = queue.pop_front() {
        if let Some(m) = by_id.get(id.as_str()) {
            for dep in &m.depends_on {
                if by_id.contains_key(dep.as_str()) && keep.insert(dep.clone()) {
                    queue.push_back(dep.clone());
                }
            }
        }
    }
    keep
}

fn delete_staged_row(m: &StagedMod) -> Result<()> {
    let staging = PathBuf::from(&m.staging_path);
    if staging.exists() {
        fs::remove_dir_all(&staging)?;
    }
    Ok(())
}

pub fn uninstall_collection(paths: &Paths, game_id: &str, collection_id: &str) -> Result<usize> {
    let mut store = load_collections(paths, game_id)?;
    if !store.collections.iter().any(|c| c.id == collection_id) {
        bail!("collection not found: {collection_id}");
    }
    store.collections.retain(|c| c.id != collection_id);
    let mut order = load_loadorder(paths, game_id)?;
    drop_collection_from_mods(&mut order, collection_id);
    let keep = reachable_mod_ids(&order, &store);
    let mut removed = 0usize;
    let mut remaining = Vec::new();
    for m in order.mods.drain(..) {
        if keep.contains(&m.id) {
            remaining.push(m);
        } else {
            delete_staged_row(&m)?;
            removed += 1;
        }
    }
    order.mods = remaining;
    for c in &mut store.collections {
        c.mod_ids
            .retain(|id| order.mods.iter().any(|m| &m.id == id));
    }
    save_loadorder(paths, game_id, &order)?;
    save_collections(paths, game_id, &store)?;
    Ok(removed)
}

pub fn list_installed_collections(
    paths: &Paths,
    game_id: &str,
) -> Result<Vec<InstalledCollection>> {
    Ok(load_collections(paths, game_id)?.collections)
}

fn copy_dir_all(src: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;
    for entry in WalkDir::new(src) {
        let entry = entry?;
        let rel = entry.path().strip_prefix(src).unwrap_or(entry.path());
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&out)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), &out)?;
        }
    }
    Ok(())
}

pub fn stage_overlay_dir(
    paths: &Paths,
    game_id: &str,
    id: &str,
    name: &str,
    source_dir: &Path,
) -> Result<StagedMod> {
    crate::config::ensure_game_dirs(paths, game_id)?;
    let safe = sanitize_filename::sanitize(name);
    let staging = paths.mods_dir(game_id).join(format!("{safe}_{id}"));
    copy_dir_all(source_dir, &staging)?;

    let mut order = load_loadorder(paths, game_id)?;
    let old = take_row_by_id(&mut order, id);
    let provenance = provenance_from_removed(old, &staging);
    let next_order = provenance
        .order
        .unwrap_or_else(|| order.mods.iter().map(|m| m.order).max().unwrap_or(0) + 1);
    let staged = StagedMod {
        id: id.to_string(),
        name: name.to_string(),
        source: ModSource::Thunderstore,
        version: None,
        domain: "profile".into(),
        staging_path: staging.to_string_lossy().to_string(),
        enabled: provenance.enabled,
        order: next_order,
        ts_namespace: Some("profile".into()),
        ts_name: Some(name.to_string()),
        collection_ids: provenance.collection_ids,
        independent: provenance.independent,
        depends_on: provenance.depends_on,
        option_selection: provenance.option_selection,
        ..Default::default()
    };
    order.mods.push(staged.clone());
    save_loadorder(paths, game_id, &order)?;
    Ok(staged)
}

/// Mod payloads that are single files rather than archives (e.g. Sims `.package`
/// downloads). These are copied into staging as-is.
pub const LOOSE_MOD_EXTENSIONS: &[&str] = &["package", "ts4script", "dbc", "sims3pack"];

pub fn is_loose_mod_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| {
            LOOSE_MOD_EXTENSIONS
                .iter()
                .any(|known| ext.eq_ignore_ascii_case(known))
        })
}

fn stage_loose_file(file: &Path, dest: &Path) -> Result<()> {
    let name = file
        .file_name()
        .map(|n| sanitize_filename::sanitize(n.to_string_lossy().as_ref()))
        .filter(|n| !n.is_empty())
        .context("mod file has no usable file name")?;
    let target = dest.join(name);
    fs::copy(file, &target)
        .with_context(|| format!("copy {} to {}", file.display(), target.display()))?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArchiveKind {
    Zip,
    SevenZ,
    Rar,
    /// A Sims `.package`; a payload, not a container.
    Dbpf,
}

impl ArchiveKind {
    fn label(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::SevenZ => "7z",
            Self::Rar => "rar",
            Self::Dbpf => "Sims package",
        }
    }
}

/// Identify a file by its header. Downloads are saved under a synthesized name,
/// so the extension says what the app guessed rather than what the file is.
fn detect_archive_kind(path: &Path) -> Option<ArchiveKind> {
    let mut file = fs::File::open(path).ok()?;
    let mut magic = [0u8; 8];
    let read = file.read(&mut magic).ok()?;
    let magic = &magic[..read];
    if magic.starts_with(b"PK\x03\x04")
        || magic.starts_with(b"PK\x05\x06")
        || magic.starts_with(b"PK\x07\x08")
    {
        return Some(ArchiveKind::Zip);
    }
    if magic.starts_with(b"7z\xBC\xAF\x27\x1C") {
        return Some(ArchiveKind::SevenZ);
    }
    if magic.starts_with(b"Rar!\x1a\x07") {
        return Some(ArchiveKind::Rar);
    }
    if magic.starts_with(b"DBPF") {
        return Some(ArchiveKind::Dbpf);
    }
    None
}

fn extract_by_kind(kind: ArchiveKind, archive: &Path, dest: &Path) -> Result<()> {
    match kind {
        ArchiveKind::Zip => extract_zip(archive, dest),
        ArchiveKind::SevenZ => extract_7z(archive, dest),
        ArchiveKind::Rar => extract_rar(archive, dest),
        ArchiveKind::Dbpf => stage_loose_file(archive, dest),
    }
}

/// Wrap a failure with what the file actually is, since the caller only knows
/// the name the app invented for it.
fn unreadable_archive(
    archive: &Path,
    detected: Option<ArchiveKind>,
    cause: anyhow::Error,
) -> anyhow::Error {
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| archive.display().to_string());
    let size = fs::metadata(archive).map(|m| m.len()).unwrap_or(0);
    match detected {
        Some(kind) => anyhow::anyhow!(
            "could not read {name} ({size} bytes, looks like {}): {cause:#}. \
             The download may be incomplete — remove it and download again.",
            kind.label()
        ),
        None => anyhow::anyhow!(
            "could not read {name} ({size} bytes): not a zip, 7z, or rar archive. \
             The download may be incomplete — remove it and download again."
        ),
    }
}

pub fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;

    // The header wins over the extension: Nexus downloads land as `.bin` and
    // mod.io downloads as `.zip` regardless of what they really contain.
    let detected = detect_archive_kind(archive);
    if let Some(kind) = detected {
        // A loose payload named like an archive is still a loose payload, but a
        // `.ts4script` is a real zip, so the extension decides between them.
        if kind == ArchiveKind::Zip && is_loose_mod_file(archive) {
            return stage_loose_file(archive, dest);
        }
        extract_by_kind(kind, archive, dest)
            .map_err(|e| unreadable_archive(archive, detected, e))?;
        repair_backslash_entries(dest)?;
        return Ok(());
    }

    if is_loose_mod_file(archive) {
        return stage_loose_file(archive, dest);
    }

    // Headerless or unrecognized: fall back to whatever the name claims.
    let ext = archive
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let result = match ext.as_str() {
        "zip" => extract_zip(archive, dest),
        "7z" => extract_7z(archive, dest),
        "rar" => extract_rar(archive, dest),
        _ => Err(anyhow::anyhow!("unrecognized file header")),
    };
    result.map_err(|e| unreadable_archive(archive, detected, e))?;
    repair_backslash_entries(dest)?;
    Ok(())
}

/// Normalize a zip/archive entry name into a safe relative path.
/// Converts Windows `\` separators; rejects `..` and absolute paths.
pub fn safe_archive_entry_path(name: &str) -> Option<PathBuf> {
    let normalized = name.replace('\\', "/");
    let trimmed = normalized.trim_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let mut out = PathBuf::new();
    for comp in Path::new(trimmed).components() {
        match comp {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        return None;
    }
    Some(out)
}

fn archive_entry_is_dir(raw_name: &str) -> bool {
    raw_name.ends_with('/') || raw_name.ends_with('\\')
}

/// Move files whose names contain literal `\` into a normal directory tree.
///
/// Windows-built zips sometimes extract on Linux as flat names like
/// `BepInEx\plugins\Mod.dll` instead of nested folders.
pub fn repair_backslash_entries(root: &Path) -> Result<()> {
    if !root.is_dir() {
        return Ok(());
    }

    let entries: Vec<_> = fs::read_dir(root)?.filter_map(|e| e.ok()).collect();
    let mut repairs: Vec<(PathBuf, PathBuf)> = Vec::new();
    for entry in &entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.contains('\\') {
            continue;
        }
        let rel = normalize_relative(Path::new(&*name_str));
        if rel.as_os_str().is_empty() {
            continue;
        }
        repairs.push((entry.path(), root.join(rel)));
    }

    repairs.sort_by_key(|(_, dest)| std::cmp::Reverse(dest.components().count()));
    for (src, dest) in repairs {
        if !src.exists() {
            continue;
        }
        let meta = fs::metadata(&src)?;
        if meta.is_file() && meta.len() == 0 && dest.is_dir() {
            fs::remove_file(&src)?;
            continue;
        }
        if dest.exists() && dest.is_dir() && meta.is_file() {
            let file_name = src
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            fs::rename(&src, dest.join(file_name)).with_context(|| {
                format!(
                    "repair backslash file {} into {}",
                    src.display(),
                    dest.display()
                )
            })?;
            continue;
        }
        if dest.exists() && dest.is_file() && meta.is_file() && meta.len() > 0 {
            fs::remove_file(&dest)?;
        } else if dest.exists() && dest.is_file() && meta.is_file() && meta.len() == 0 {
            fs::remove_file(&src)?;
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::rename(&src, &dest).with_context(|| {
            format!(
                "repair backslash path {} -> {}",
                src.display(),
                dest.display()
            )
        })?;
    }

    for entry in fs::read_dir(root)?.filter_map(|e| e.ok()) {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            repair_backslash_entries(&entry.path())?;
        }
    }
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(archive)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let raw_name = file.name();
        let is_dir = archive_entry_is_dir(raw_name);
        let rel = match safe_archive_entry_path(raw_name.trim_end_matches(['/', '\\'])) {
            Some(p) => p,
            None => continue,
        };
        let outpath = dest.join(rel);
        if is_dir {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
    }
    Ok(())
}

fn extract_7z(archive: &Path, dest: &Path) -> Result<()> {
    sevenz_rust::decompress_file(archive, dest).map_err(|e| anyhow::anyhow!("7z extract: {e}"))
}

fn extract_rar(archive: &Path, dest: &Path) -> Result<()> {
    let archive = unrar_ng::Archive::new(archive)
        .open_for_processing()
        .map_err(|e| anyhow::anyhow!("rar open: {e}"))?;
    archive
        .extract_all(dest)
        .map_err(|e| anyhow::anyhow!("rar extract: {e}"))?;
    Ok(())
}

pub fn stage_mod(
    paths: &Paths,
    game_id: &str,
    name: &str,
    domain: &str,
    mod_id: u64,
    file_id: u64,
    version: Option<String>,
    archive: &Path,
) -> Result<StagedMod> {
    stage_mod_inner(
        paths, game_id, None, name, domain, mod_id, file_id, version, archive,
    )
}

/// Stage a Nexus archive, replacing a specific staged row (e.g. updating to a new file id).
pub fn stage_mod_replacing(
    paths: &Paths,
    game_id: &str,
    replace_id: &str,
    name: &str,
    domain: &str,
    mod_id: u64,
    file_id: u64,
    version: Option<String>,
    archive: &Path,
) -> Result<StagedMod> {
    stage_mod_inner(
        paths,
        game_id,
        Some(replace_id),
        name,
        domain,
        mod_id,
        file_id,
        version,
        archive,
    )
}

fn stage_mod_inner(
    paths: &Paths,
    game_id: &str,
    replace_id: Option<&str>,
    name: &str,
    domain: &str,
    mod_id: u64,
    file_id: u64,
    version: Option<String>,
    archive: &Path,
) -> Result<StagedMod> {
    crate::config::ensure_game_dirs(paths, game_id)?;
    let safe = sanitize_filename::sanitize(name);
    let staging = paths
        .mods_dir(game_id)
        .join(format!("{safe}_{mod_id}_{file_id}"));
    extract_archive(archive, &staging)?;

    let mut order = load_loadorder(paths, game_id)?;
    let old = if let Some(id) = replace_id {
        take_row_by_id(&mut order, id)
    } else {
        take_nexus_row(&mut order, mod_id, file_id)
    };
    let old_id = old.as_ref().map(|m| m.id.clone());
    let provenance = provenance_from_removed(old, &staging);
    let new_id = format!("{mod_id}_{file_id}");
    if let Some(old_id) = old_id {
        rewrite_depends_on(&mut order, &old_id, &new_id);
    }
    let next_order = provenance
        .order
        .unwrap_or_else(|| order.mods.iter().map(|m| m.order).max().unwrap_or(0) + 1);
    let staged = StagedMod {
        id: new_id,
        name: name.to_string(),
        source: ModSource::Nexus,
        nexus_mod_id: mod_id,
        nexus_file_id: file_id,
        version,
        domain: domain.to_string(),
        staging_path: staging.to_string_lossy().to_string(),
        enabled: provenance.enabled,
        order: next_order,
        ts_namespace: None,
        ts_name: None,
        ts_package_uuid: None,
        modio_game_id: None,
        modio_mod_id: None,
        modio_file_id: None,
        collection_ids: provenance.collection_ids,
        independent: provenance.independent,
        depends_on: provenance.depends_on,
        staged_file_count: Some(count_staged_files(&staging)),
        option_selection: provenance.option_selection,
    };
    order.mods.push(staged.clone());
    save_loadorder(paths, game_id, &order)?;
    Ok(staged)
}

/// Files currently present in a staging folder.
pub fn count_staged_files(dir: &Path) -> usize {
    WalkDir::new(dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .count()
}

/// Staged mods that lost files after extraction. Older builds symlinked staging
/// into the game folder, so a loader deleting its own plugin directory deleted
/// the staged originals too; those packages must be downloaded again.
pub fn damaged_staging(order: &LoadOrder) -> Vec<&StagedMod> {
    order
        .mods
        .iter()
        .filter(|m| {
            let Some(expected) = m.staged_file_count else {
                return false;
            };
            let staging = Path::new(&m.staging_path);
            staging.is_dir() && count_staged_files(staging) < expected
        })
        .collect()
}

pub fn stage_thunderstore_mod(
    paths: &Paths,
    game_id: &str,
    community: &str,
    namespace: &str,
    name: &str,
    version: &str,
    package_uuid: Option<String>,
    display_name: &str,
    archive: &Path,
) -> Result<StagedMod> {
    crate::config::ensure_game_dirs(paths, game_id)?;
    let safe = sanitize_filename::sanitize(display_name);
    let ns_safe = sanitize_filename::sanitize(namespace);
    let name_safe = sanitize_filename::sanitize(name);
    let ver_safe = sanitize_filename::sanitize(version);
    let staging = paths
        .mods_dir(game_id)
        .join(format!("{safe}_{ns_safe}_{name_safe}_{ver_safe}"));
    extract_archive(archive, &staging)?;

    let mut order = load_loadorder(paths, game_id)?;
    let id = thunderstore_mod_id(namespace, name);
    let old = take_row_by_id(&mut order, &id);
    let provenance = provenance_from_removed(old, &staging);
    let next_order = provenance
        .order
        .unwrap_or_else(|| order.mods.iter().map(|m| m.order).max().unwrap_or(0) + 1);
    let staged = StagedMod {
        id,
        name: display_name.to_string(),
        source: ModSource::Thunderstore,
        nexus_mod_id: 0,
        nexus_file_id: 0,
        version: Some(version.to_string()),
        domain: community.to_string(),
        staging_path: staging.to_string_lossy().to_string(),
        enabled: provenance.enabled,
        order: next_order,
        ts_namespace: Some(namespace.to_string()),
        ts_name: Some(name.to_string()),
        ts_package_uuid: package_uuid,
        modio_game_id: None,
        modio_mod_id: None,
        modio_file_id: None,
        collection_ids: provenance.collection_ids,
        independent: provenance.independent,
        depends_on: provenance.depends_on,
        staged_file_count: Some(count_staged_files(&staging)),
        option_selection: provenance.option_selection,
    };
    order.mods.push(staged.clone());
    save_loadorder(paths, game_id, &order)?;
    Ok(staged)
}

pub fn stage_modio_mod(
    paths: &Paths,
    game_id: &str,
    modio_game_id: u32,
    modio_mod_id: u64,
    modio_file_id: u64,
    name: &str,
    version: Option<String>,
    archive: &Path,
) -> Result<StagedMod> {
    crate::config::ensure_game_dirs(paths, game_id)?;
    let safe = sanitize_filename::sanitize(name);
    let staging = paths.mods_dir(game_id).join(format!(
        "{safe}_{modio_game_id}_{modio_mod_id}_{modio_file_id}"
    ));
    extract_archive(archive, &staging)?;

    let mut order = load_loadorder(paths, game_id)?;
    let id = modio_identity_id(modio_game_id, modio_mod_id);
    let old = take_modio_row(&mut order, modio_game_id, modio_mod_id);
    let old_id = old.as_ref().map(|m| m.id.clone());
    let provenance = provenance_from_removed(old, &staging);
    if let Some(old_id) = old_id {
        rewrite_depends_on(&mut order, &old_id, &id);
    }
    let next_order = provenance
        .order
        .unwrap_or_else(|| order.mods.iter().map(|m| m.order).max().unwrap_or(0) + 1);
    let staged = StagedMod {
        id,
        name: name.to_string(),
        source: ModSource::Modio,
        nexus_mod_id: 0,
        nexus_file_id: 0,
        version,
        domain: format!("modio:{modio_game_id}"),
        staging_path: staging.to_string_lossy().to_string(),
        enabled: provenance.enabled,
        order: next_order,
        ts_namespace: None,
        ts_name: None,
        ts_package_uuid: None,
        modio_game_id: Some(modio_game_id),
        modio_mod_id: Some(modio_mod_id),
        modio_file_id: Some(modio_file_id),
        collection_ids: provenance.collection_ids,
        independent: provenance.independent,
        depends_on: provenance.depends_on,
        staged_file_count: Some(count_staged_files(&staging)),
        option_selection: provenance.option_selection,
    };
    order.mods.push(staged.clone());
    save_loadorder(paths, game_id, &order)?;
    Ok(staged)
}

/// True when a Thunderstore package (any version) is already staged.
#[allow(dead_code)]
pub fn has_thunderstore_package(
    paths: &Paths,
    game_id: &str,
    namespace: &str,
    name: &str,
) -> Result<bool> {
    let order = load_loadorder(paths, game_id)?;
    Ok(order.mods.iter().any(|m| {
        m.source == ModSource::Thunderstore
            && m.ts_namespace.as_deref() == Some(namespace)
            && m.ts_name.as_deref() == Some(name)
    }))
}

#[allow(dead_code)]
pub fn has_modio_mod(paths: &Paths, game_id: &str, modio_mod_id: u64) -> Result<bool> {
    let order = load_loadorder(paths, game_id)?;
    Ok(order
        .mods
        .iter()
        .any(|m| m.source == ModSource::Modio && m.modio_mod_id == Some(modio_mod_id)))
}

pub fn set_enabled(paths: &Paths, game_id: &str, mod_uid: &str, enabled: bool) -> Result<()> {
    let mut order = load_loadorder(paths, game_id)?;
    let Some(m) = order.mods.iter_mut().find(|m| m.id == mod_uid) else {
        bail!("mod not found: {mod_uid}");
    };
    m.enabled = enabled;
    save_loadorder(paths, game_id, &order)
}

pub fn set_mod_options(
    paths: &Paths,
    game_id: &str,
    mod_uid: &str,
    selection: ModOptionSelection,
) -> Result<()> {
    let mut order = load_loadorder(paths, game_id)?;
    let Some(m) = order.mods.iter_mut().find(|m| m.id == mod_uid) else {
        bail!("mod not found: {mod_uid}");
    };
    m.option_selection = Some(selection);
    save_loadorder(paths, game_id, &order)
}

pub fn set_load_order(paths: &Paths, game_id: &str, ordered_ids: &[String]) -> Result<()> {
    let mut order = load_loadorder(paths, game_id)?;
    let mut by_id: HashMap<String, StagedMod> =
        order.mods.drain(..).map(|m| (m.id.clone(), m)).collect();
    let mut new_mods = Vec::new();
    for (i, id) in ordered_ids.iter().enumerate() {
        if let Some(mut m) = by_id.remove(id) {
            m.order = (i as u32) + 1;
            new_mods.push(m);
        }
    }
    // Append any leftovers
    for (_, mut m) in by_id {
        m.order = (new_mods.len() as u32) + 1;
        new_mods.push(m);
    }
    order.mods = new_mods;
    save_loadorder(paths, game_id, &order)
}

pub fn remove_mod(paths: &Paths, game_id: &str, mod_uid: &str) -> Result<()> {
    let mut order = load_loadorder(paths, game_id)?;
    if let Some(pos) = order.mods.iter().position(|m| m.id == mod_uid) {
        let m = order.mods.remove(pos);
        delete_staged_row(&m)?;
        save_loadorder(paths, game_id, &order)?;
        let mut store = load_collections(paths, game_id)?;
        for c in &mut store.collections {
            c.mod_ids.retain(|id| id != mod_uid);
        }
        save_collections(paths, game_id, &store)?;
    }
    Ok(())
}

pub fn remove_all_mods(paths: &Paths, game_id: &str, install_path: Option<&Path>) -> Result<()> {
    purge_deploy(paths, game_id, install_path)?;
    let order = load_loadorder(paths, game_id)?;
    for m in &order.mods {
        delete_staged_row(m)?;
    }
    save_loadorder(paths, game_id, &LoadOrder::default())?;
    save_collections(paths, game_id, &InstalledCollections::default())?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkKind {
    Hardlinked,
    Copied,
}

/// Remove a deploy destination without following symlinks.
fn remove_deploy_path(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || meta.is_file() {
        fs::remove_file(path)?;
    } else if meta.is_dir() {
        fs::remove_dir_all(path)?;
    }
    Ok(())
}

/// Ensure `dest` can be created as a directory (remove blocking files/symlinks).
fn ensure_deploy_dir(dest: &Path) -> Result<()> {
    if dest.exists() {
        let meta = fs::symlink_metadata(dest)?;
        if meta.file_type().is_symlink() || meta.is_file() {
            fs::remove_file(dest)?;
        }
    }
    fs::create_dir_all(dest)?;
    Ok(())
}

fn ensure_deploy_parent(dest: &Path) -> Result<()> {
    let Some(parent) = dest.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    if parent.exists() {
        let meta = fs::symlink_metadata(parent)?;
        if meta.file_type().is_symlink() || meta.is_file() {
            fs::remove_file(parent)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink_points_into(path: &Path, root: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.file_type().is_symlink() {
        return false;
    }
    let Ok(target) = fs::read_link(path) else {
        return false;
    };
    let resolved = if target.is_absolute() {
        target
    } else {
        path.parent().map(|p| p.join(&target)).unwrap_or(target)
    };
    resolved.starts_with(root)
}

#[cfg(windows)]
fn symlink_points_into(path: &Path, root: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.file_type().is_symlink() {
        return false;
    }
    let Ok(target) = fs::read_link(path) else {
        return false;
    };
    let resolved = if target.is_absolute() {
        target
    } else {
        path.parent().map(|p| p.join(&target)).unwrap_or(target)
    };
    resolved.starts_with(root)
}

#[cfg(not(any(unix, windows)))]
fn symlink_points_into(_path: &Path, _root: &Path) -> bool {
    false
}

fn prune_empty_dirs_up(install_path: &Path, mut dir: PathBuf) {
    while dir.starts_with(install_path) && dir != install_path {
        let is_empty = fs::read_dir(&dir)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if is_empty {
            let _ = fs::remove_dir(&dir);
            if !dir.pop() {
                break;
            }
        } else {
            break;
        }
    }
}

/// Remove deployed symlinks under the game folder that point into app data.
/// Covers failed deploys where `deployed.json` was never written.
pub fn purge_install_symlinks(install_path: &Path, data_dir: &Path) -> Result<usize> {
    if !install_path.is_dir() {
        return Ok(0);
    }
    let mut symlinks: Vec<PathBuf> = Vec::new();
    for entry in WalkDir::new(install_path)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if symlink_points_into(path, data_dir) {
            symlinks.push(path.to_path_buf());
        }
    }
    symlinks.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    let mut removed = 0usize;
    for path in symlinks {
        if fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            if let Some(parent) = path.parent() {
                fs::remove_file(&path)?;
                prune_empty_dirs_up(install_path, parent.to_path_buf());
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// Remove empty leftover directories under loader-owned folders. Mods that were
/// purged or removed leave their (now empty) folders behind, which pile up in
/// `BepInEx/plugins` and make an install look modded when it is not.
pub fn prune_empty_loader_dirs(plugin: &dyn GamePlugin, install_path: &Path) -> usize {
    let mut removed = 0usize;
    for root in plugin.prunable_dirs(install_path) {
        let mut dirs: Vec<PathBuf> = WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_dir() && e.path() != root)
            .map(|e| e.path().to_path_buf())
            .collect();
        // Deepest first so nested leftovers collapse in one pass.
        dirs.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
        for dir in dirs {
            let is_empty = fs::read_dir(&dir)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(false);
            if is_empty && fs::remove_dir(&dir).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

/// Manifest paths that no longer exist, i.e. files a game or loader deleted
/// after deploy. Returned paths are relative to the install when possible.
fn missing_deployed_paths(deployed: &DeployManifest, install_path: &Path) -> Vec<String> {
    deployed
        .paths
        .iter()
        .filter(|p| !Path::new(p).exists())
        .map(|p| {
            Path::new(p)
                .strip_prefix(install_path)
                .unwrap_or(Path::new(p))
                .to_string_lossy()
                .to_string()
        })
        .collect()
}

fn link_or_copy(src: &Path, dest: &Path) -> Result<LinkKind> {
    ensure_deploy_parent(dest)?;
    if dest.exists() {
        remove_deploy_path(dest)?;
    }
    if let Some(parent) = dest.parent() {
        ensure_deploy_dir(parent)?;
    }
    // Hardlink files; directories are created normally and children linked.
    if src.is_dir() {
        ensure_deploy_dir(dest)?;
        return Ok(LinkKind::Hardlinked);
    }
    // Never symlink into a game folder: a mod loader that deletes its own
    // directory (BepInEx prunes BepInEx/plugins/RoR2BepInExPack on startup)
    // follows the link and destroys the staged original.
    match fs::hard_link(src, dest) {
        Ok(()) => Ok(LinkKind::Hardlinked),
        Err(e) => {
            log::debug!(
                "hardlink failed for {} -> {} ({e}); copying instead",
                src.display(),
                dest.display()
            );
            fs::copy(src, dest)?;
            Ok(LinkKind::Copied)
        }
    }
}

pub fn purge_deploy(paths: &Paths, game_id: &str, install_path: Option<&Path>) -> Result<()> {
    let manifest_path = paths.deploy_manifest(game_id);
    if manifest_path.exists() {
        let raw = fs::read_to_string(&manifest_path)?;
        let manifest: DeployManifest = serde_json::from_str(&raw)?;
        // Remove files first, then empty dirs (reverse sort by path length)
        let mut entries = manifest.paths;
        entries.sort_by_key(|p| std::cmp::Reverse(p.len()));
        for p in entries {
            let path = PathBuf::from(&p);
            if path.is_symlink() || path.is_file() {
                let _ = fs::remove_file(&path);
            } else if path.is_dir() {
                let _ = fs::remove_dir(&path); // only if empty
            }
        }
        fs::write(
            &manifest_path,
            serde_json::to_string_pretty(&DeployManifest::default())?,
        )?;
    }
    if let Some(install) = install_path {
        purge_install_symlinks(install, &paths.data_dir)?;
    }
    Ok(())
}

pub fn deploy(
    paths: &Paths,
    game_id: &str,
    plugin_id: &str,
    install_path: &Path,
    project_name: Option<&str>,
) -> Result<DeployResult> {
    let plugin = plugin_by_id(plugin_id).context("unknown game plugin")?;
    let mut warnings = Vec::new();
    let base_ctx = DeployContext {
        project_name,
        content_root: None,
        source_dir: None,
    };

    if !install_path.is_dir() {
        bail!(
            "Game install path does not exist or is not a directory: {}",
            install_path.display()
        );
    }

    warnings.extend(plugin.prepare_deploy(install_path)?);

    purge_deploy(paths, game_id, Some(install_path))?;
    prune_empty_loader_dirs(plugin, install_path);

    let mut order = load_loadorder(paths, game_id)?;
    order.mods.sort_by_key(|m| m.order);

    let mut deployed = DeployManifest::default();
    let mut count = 0usize;
    let mut copied_files = 0usize;
    let mut skipped_files = 0usize;
    let enabled: Vec<_> = order.mods.iter().filter(|m| m.enabled).cloned().collect();
    let enabled_mods = enabled.len();
    let mut enabled_mod_folders = Vec::new();

    for staged in &enabled {
        let staging = PathBuf::from(&staged.staging_path);
        if !staging.exists() {
            let msg = format!("Missing staging for {}: {}", staged.name, staging.display());
            log::warn!("{msg}");
            warnings.push(msg);
            continue;
        }
        if let Some(expected) = staged.staged_file_count {
            let actual = count_staged_files(&staging);
            if actual < expected {
                warnings.push(format!(
                    "{} is missing {} of {expected} staged file(s); download it again before deploying.",
                    staged.name,
                    expected - actual
                ));
            }
        }
        if let Err(e) = repair_backslash_entries(&staging) {
            warnings.push(format!(
                "Could not normalize Windows-style paths for {}: {e:#}",
                staged.name
            ));
        }
        let root = normalize_staging_root(&staging, plugin)?;
        warnings.extend(plugin.staging_deploy_warnings(&root, &staged.name));
        let filter = plugin
            .mod_options(&root)
            .map(|set| IncludeFilter::build(&set, staged.option_selection.as_ref()))
            .filter(|f| !f.is_passthrough());
        let to_root = plugin.deploys_to_install_root(&root);
        let wrap =
            !to_root && (plugin.prefers_mod_folder() || plugin.should_wrap_as_mod_folder(&root));
        if wrap {
            let folder_name = plugin.wrap_mod_folder_name(&root, &staged.name);
            if !folder_name.is_empty() {
                enabled_mod_folders.push(folder_name);
            }
        }
        let deploy_root = if to_root {
            bepinex_pack_deploy_root(&root)
        } else {
            root.clone()
        };
        let stats = deploy_tree(
            plugin,
            install_path,
            &deploy_root,
            &staged.name,
            project_name,
            filter.as_ref(),
            &mut deployed,
        )?;
        if stats.linked == 0 {
            let msg = if filter.is_some() {
                format!(
                    "No files deployed from {}; check its options, every one may be turned off.",
                    staged.name
                )
            } else {
                format!("No files deployed from {}", staged.name)
            };
            warnings.push(msg);
        }
        copied_files += stats.copied;
        skipped_files += stats.skipped;
        count += stats.linked;
    }

    if copied_files > 0 {
        warnings.push(format!(
            "{copied_files} file(s) were copied instead of hardlinked because staging and the game are on different drives. Keep them on one drive to save space."
        ));
    }

    if skipped_files > 0 {
        warnings.push(format!(
            "{skipped_files} file(s) were skipped because this game does not load them (documentation, previews, or loose archives)."
        ));
    }

    if enabled_mods == 0 {
        warnings.push("No enabled mods to deploy.".into());
    } else if count == 0 {
        warnings.push("Deploy finished with 0 files linked.".into());
    }

    warnings.extend(plugin.after_deploy(install_path, &enabled_mod_folders)?);

    // Warn about the tree we just produced, not the one we started from: a pack
    // that failed to land leaves an install that cannot load anything, and a
    // pre-purge check would report problems the deploy has already fixed.
    warnings.extend(plugin.preflight_warnings_ctx(install_path, &base_ctx));
    let missing = missing_deployed_paths(&deployed, install_path);
    if !missing.is_empty() {
        warnings.push(format!(
            "{} deployed file(s) are already missing from the game folder, starting with {}. Purge and deploy again.",
            missing.len(),
            missing[0]
        ));
    }

    crate::config::ensure_game_dirs(paths, game_id)?;
    fs::write(
        paths.deploy_manifest(game_id),
        serde_json::to_string_pretty(&deployed)?,
    )?;
    Ok(DeployResult {
        file_count: count,
        enabled_mods,
        warnings,
    })
}

#[derive(Debug, Default, Clone, Copy)]
struct TreeStats {
    linked: usize,
    copied: usize,
    skipped: usize,
}

fn deploy_tree(
    plugin: &dyn GamePlugin,
    install_path: &Path,
    content_root: &Path,
    mod_name: &str,
    project_name: Option<&str>,
    filter: Option<&IncludeFilter>,
    deployed: &mut DeployManifest,
) -> Result<TreeStats> {
    let mut stats = TreeStats::default();
    let to_root = plugin.deploys_to_install_root(content_root);
    let wrap =
        !to_root && (plugin.prefers_mod_folder() || plugin.should_wrap_as_mod_folder(content_root));
    let folder_name = plugin.wrap_mod_folder_name(content_root, mod_name);

    for entry in WalkDir::new(content_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path == content_root {
            continue;
        }
        let source_rel = normalize_relative(path.strip_prefix(content_root)?);
        // Options decide membership before the plugin decides layout, and they
        // rewrite the path: an include folder is a container, so its contents
        // deploy as if they had been at the mod root all along.
        let rel = match filter {
            Some(f) => match f.map(&source_rel, entry.file_type().is_dir()) {
                Some(mapped) => mapped,
                None => continue,
            },
            None => source_rel.clone(),
        };
        let ctx = DeployContext {
            project_name,
            content_root: Some(content_root),
            source_dir: source_rel.parent(),
        };
        if entry.file_type().is_file() && !plugin.should_deploy_file_ctx(&rel, &ctx) {
            stats.skipped += 1;
            continue;
        }
        let deploy_rel: PathBuf = if wrap {
            Path::new(&folder_name).join(&rel)
        } else {
            rel
        };
        let dest = if to_root {
            install_path.join(&deploy_rel)
        } else {
            plugin.resolve_deploy_root_ctx(install_path, &deploy_rel, &ctx)?
        };
        if entry.file_type().is_dir() {
            ensure_deploy_dir(&dest)?;
            deployed.paths.push(dest.to_string_lossy().to_string());
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        if link_or_copy(path, &dest)? == LinkKind::Copied {
            stats.copied += 1;
        }
        deployed.paths.push(dest.to_string_lossy().to_string());
        stats.linked += 1;
    }
    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;

    fn test_paths() -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_dir: tmp.path().join("config"),
            data_dir: tmp.path().join("data"),
            cache_dir: tmp.path().join("cache"),
        };
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::create_dir_all(&paths.data_dir).unwrap();
        std::fs::create_dir_all(&paths.cache_dir).unwrap();
        (tmp, paths)
    }

    /// A one-entry zip, returned as raw bytes so tests can truncate it.
    fn zip_bytes() -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        let mut zip = zip::ZipWriter::new(&mut buf);
        zip.start_file::<_, ()>("readme.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, b"hello").unwrap();
        zip.finish().unwrap();
        buf.into_inner()
    }

    /// Downloads land under a synthesized `.bin` name, so every extraction path
    /// has to work from the header alone.
    #[test]
    fn a_zip_named_bin_still_extracts() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("helldivers2_383_35491.bin");
        std::fs::write(&archive, zip_bytes()).unwrap();

        let dest = tmp.path().join("out");
        extract_archive(&archive, &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("readme.txt")).unwrap(), b"hello");
    }

    #[test]
    fn a_sims_package_named_bin_is_staged_as_a_loose_file() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("sims4_99_1.bin");
        let mut body = b"DBPF".to_vec();
        body.extend_from_slice(&[0u8; 64]);
        std::fs::write(&archive, &body).unwrap();

        let dest = tmp.path().join("out");
        extract_archive(&archive, &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("sims4_99_1.bin")).unwrap(), body);
    }

    /// A `.ts4script` is a real zip, so the loose-file rule must not swallow it.
    #[test]
    fn a_ts4script_is_copied_whole_rather_than_unpacked() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("mod.ts4script");
        std::fs::write(&archive, zip_bytes()).unwrap();

        let dest = tmp.path().join("out");
        extract_archive(&archive, &dest).unwrap();
        assert!(dest.join("mod.ts4script").exists());
        assert!(!dest.join("readme.txt").exists());
    }

    /// The bug this whole change exists for: a download cut short used to be
    /// reported as "unsupported archive type: .bin", which sent people looking
    /// for a format problem that was never there.
    #[test]
    fn a_truncated_zip_reports_the_file_not_a_bogus_archive_type() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("helldivers2_383_35491.bin");
        let full = zip_bytes();
        std::fs::write(&archive, &full[..full.len() / 2]).unwrap();

        let err = extract_archive(&archive, &tmp.path().join("out")).unwrap_err();
        let msg = err.to_string();
        assert!(!msg.contains("unsupported archive type"), "{msg}");
        assert!(msg.contains("helldivers2_383_35491.bin"), "{msg}");
        assert!(msg.contains("looks like zip"), "{msg}");
        assert!(msg.contains("download again"), "{msg}");
    }

    #[test]
    fn random_bytes_fail_with_the_size_included() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("garbage.bin");
        std::fs::write(&archive, vec![0x42u8; 4096]).unwrap();

        let err = extract_archive(&archive, &tmp.path().join("out")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("garbage.bin"), "{msg}");
        assert!(msg.contains("4096 bytes"), "{msg}");
        assert!(msg.contains("not a zip, 7z, or rar"), "{msg}");
    }

    #[test]
    fn headers_are_recognized_regardless_of_name() {
        let tmp = tempfile::tempdir().unwrap();
        let cases: &[(&str, &[u8], ArchiveKind)] = &[
            ("a.bin", b"PK\x03\x04rest", ArchiveKind::Zip),
            ("b.zip", b"7z\xBC\xAF\x27\x1C\x00\x04", ArchiveKind::SevenZ),
            ("c.bin", b"Rar!\x1a\x07\x01\x00", ArchiveKind::Rar),
            ("d.bin", b"DBPF\x02\x00\x00\x00", ArchiveKind::Dbpf),
        ];
        for (name, magic, expected) in cases {
            let path = tmp.path().join(name);
            std::fs::write(&path, magic).unwrap();
            assert_eq!(detect_archive_kind(&path), Some(*expected), "{name}");
        }

        let plain = tmp.path().join("e.bin");
        std::fs::write(&plain, b"nothing here").unwrap();
        assert_eq!(detect_archive_kind(&plain), None);
    }

    /// An Arsenal-style pack: root file, a plain option, and a two-variant group.
    fn stage_hd2_option_mod(paths: &Paths, game_id: &str) -> PathBuf {
        let staging = paths.mods_dir(game_id).join("ArsenalMod_1_1");
        std::fs::create_dir_all(staging.join("Head")).unwrap();
        std::fs::create_dir_all(staging.join("Body").join("Blue")).unwrap();
        std::fs::create_dir_all(staging.join("Body").join("Red")).unwrap();
        std::fs::create_dir_all(staging.join("Unused")).unwrap();
        std::fs::write(
            staging.join("manifest.json"),
            r#"{
                "Version": 1,
                "Name": "Arsenal Mod",
                "Description": "",
                "Options": [
                    { "Name": "Head", "Include": ["Head"] },
                    { "Name": "Body", "SubOptions": [
                        { "Name": "Blue", "Include": ["Body/Blue"] },
                        { "Name": "Red", "Include": ["Body/Red"] }
                    ] }
                ]
            }"#,
        )
        .unwrap();
        std::fs::write(staging.join("always.patch_0"), b"root").unwrap();
        std::fs::write(staging.join("Head").join("aaa.patch_0"), b"head").unwrap();
        std::fs::write(
            staging.join("Body").join("Blue").join("bbb.patch_0"),
            b"blue",
        )
        .unwrap();
        std::fs::write(staging.join("Body").join("Red").join("bbb.patch_0"), b"red").unwrap();
        std::fs::write(staging.join("Unused").join("ccc.patch_0"), b"unused").unwrap();
        staging
    }

    fn hd2_order(staging: &Path, selection: Option<ModOptionSelection>) -> LoadOrder {
        LoadOrder {
            mods: vec![StagedMod {
                id: "1_1".into(),
                name: "Arsenal Mod".into(),
                domain: "helldivers2".into(),
                staging_path: staging.to_string_lossy().into(),
                enabled: true,
                order: 1,
                option_selection: selection,
                ..Default::default()
            }],
        }
    }

    #[test]
    fn hd2_options_default_to_everything_on_with_the_first_variant() {
        let _gate = crate::games::hd2_test_gate();
        let (_tmp, paths) = test_paths();
        let game_id = "hd2_default";
        let install = tempfile::tempdir().unwrap();
        let staging = stage_hd2_option_mod(&paths, game_id);
        save_loadorder(&paths, game_id, &hd2_order(&staging, None)).unwrap();

        deploy(&paths, game_id, "helldivers2", install.path(), None).unwrap();

        let data = install.path().join("data");
        assert!(data.join("always.patch_0").exists());
        assert!(data.join("aaa.patch_0").exists());
        // Blue is the first sub-option, so Red must not be there.
        assert_eq!(std::fs::read(data.join("bbb.patch_0")).unwrap(), b"blue");
        // A folder the manifest never includes is packaging, not content.
        assert!(!data.join("ccc.patch_0").exists());
        // Include folders are containers; they leave no shell behind.
        assert!(!data.join("Head").exists());
        assert!(!data.join("Body").exists());
        assert!(!data.join("manifest.json").exists());
    }

    #[test]
    fn hd2_selection_picks_the_other_variant_and_drops_disabled_options() {
        let _gate = crate::games::hd2_test_gate();
        let (_tmp, paths) = test_paths();
        let game_id = "hd2_selected";
        let install = tempfile::tempdir().unwrap();
        let staging = stage_hd2_option_mod(&paths, game_id);
        let selection = ModOptionSelection {
            enabled_options: vec!["1-body".into()],
            sub_choice: [("1-body".to_string(), "1-body.1-red".to_string())]
                .into_iter()
                .collect(),
        };
        save_loadorder(&paths, game_id, &hd2_order(&staging, Some(selection))).unwrap();

        deploy(&paths, game_id, "helldivers2", install.path(), None).unwrap();

        let data = install.path().join("data");
        assert!(data.join("always.patch_0").exists());
        assert!(!data.join("aaa.patch_0").exists());
        assert_eq!(std::fs::read(data.join("bbb.patch_0")).unwrap(), b"red");
    }

    #[test]
    fn hd2_option_selection_survives_a_reinstall() {
        let (_tmp, paths) = test_paths();
        let game_id = "hd2_update";
        let staging = stage_hd2_option_mod(&paths, game_id);
        let selection = ModOptionSelection {
            enabled_options: vec!["1-body".into()],
            sub_choice: [("1-body".to_string(), "1-body.1-red".to_string())]
                .into_iter()
                .collect(),
        };
        let mut order = hd2_order(&staging, Some(selection.clone()));
        order.mods[0].nexus_mod_id = 1;
        order.mods[0].nexus_file_id = 1;
        save_loadorder(&paths, game_id, &order).unwrap();

        let archive = _tmp.path().join("update.zip");
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file::<_, ()>("abc.patch_0", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, b"x").unwrap();
        zip.finish().unwrap();

        let staged = stage_mod_replacing(
            &paths,
            game_id,
            "1_1",
            "Arsenal Mod",
            "helldivers2",
            1,
            2,
            None,
            &archive,
        )
        .unwrap();
        assert_eq!(staged.option_selection, Some(selection));
    }

    #[test]
    fn deploy_cyberpunk_preserves_bin_and_red4ext() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_test";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("CET_1_1");
        std::fs::create_dir_all(staging.join("bin").join("x64").join("plugins")).unwrap();
        std::fs::write(staging.join("bin").join("x64").join("version.dll"), b"dll").unwrap();
        std::fs::create_dir_all(staging.join("red4ext").join("plugins")).unwrap();
        std::fs::write(staging.join("red4ext").join("RED4ext.dll"), b"dll").unwrap();

        let order = LoadOrder {
            mods: vec![StagedMod {
                id: "1_1".into(),
                name: "CET".into(),
                nexus_mod_id: 1,
                nexus_file_id: 1,
                version: None,
                domain: "cyberpunk2077".into(),
                staging_path: staging.to_string_lossy().into(),
                enabled: true,
                order: 1,
                ..Default::default()
            }],
        };
        save_loadorder(&paths, game_id, &order).unwrap();

        let result = deploy(&paths, game_id, "cyberpunk2077", install.path(), None).unwrap();
        assert!(result.file_count >= 2, "warnings: {:?}", result.warnings);
        assert!(install.path().join("bin/x64/version.dll").exists());
        assert!(install.path().join("red4ext/RED4ext.dll").exists());
        assert!(!install.path().join("mods/bin/x64/version.dll").exists());
        assert!(!install.path().join("mods/red4ext/RED4ext.dll").exists());
    }

    #[test]
    fn deploy_peels_wrapper_then_keeps_roots() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_wrap";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("Wrap_2_2");
        let inner = staging.join("Cyber Engine Tweaks");
        std::fs::create_dir_all(inner.join("bin").join("x64")).unwrap();
        std::fs::write(inner.join("bin").join("x64").join("global.ini"), b"x").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "2_2".into(),
                    name: "CET Wrap".into(),
                    nexus_mod_id: 2,
                    nexus_file_id: 2,
                    version: None,
                    domain: "cyberpunk2077".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "cyberpunk2077", install.path(), None).unwrap();
        assert_eq!(result.file_count, 1, "{:?}", result.warnings);
        assert!(install.path().join("bin/x64/global.ini").exists());
    }

    #[test]
    fn deploy_flat_archive_and_backslash_paths() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_flat";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("Flat_3_3");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("Cool.archive"), b"a").unwrap();
        // Literal backslash in filename as produced by some Windows zips on Linux.
        std::fs::write(staging.join(r"r6\scripts\Mod\mod.reds"), b"reds").unwrap();
        repair_backslash_entries(&staging).unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "3_3".into(),
                    name: "Flat".into(),
                    nexus_mod_id: 3,
                    nexus_file_id: 3,
                    version: None,
                    domain: "cyberpunk2077".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "cyberpunk2077", install.path(), None).unwrap();
        assert!(result.file_count >= 2, "{:?}", result.warnings);
        assert!(install.path().join("archive/pc/mod/Cool.archive").exists());
        assert!(install.path().join("r6/scripts/Mod/mod.reds").exists());
    }

    #[test]
    fn repair_backslash_entries_builds_tree() {
        let staging = tempfile::tempdir().unwrap();
        std::fs::write(staging.path().join(r"BepInEx\plugins\"), b"").unwrap();
        std::fs::write(
            staging
                .path()
                .join(r"BepInEx\plugins\DarkMode\DarkMode.dll"),
            b"dll",
        )
        .unwrap();

        repair_backslash_entries(staging.path()).unwrap();

        assert!(staging.path().join("BepInEx/plugins").is_dir());
        assert!(staging
            .path()
            .join("BepInEx/plugins/DarkMode/DarkMode.dll")
            .is_file());
    }

    #[test]
    fn deploy_bepinex_repaired_backslash_plugin() {
        let (_tmp, paths) = test_paths();
        let game_id = "bw_bepinex";
        let install = tempfile::tempdir().unwrap();
        let staging = paths.mods_dir(game_id).join("DarkMode_1");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join(r"BepInEx\plugins\"), b"").unwrap();
        std::fs::write(
            staging.join(r"BepInEx\plugins\DarkMode\DarkMode.dll"),
            b"dll",
        )
        .unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "1".into(),
                    name: "DarkMode".into(),
                    source: ModSource::Thunderstore,
                    domain: "big-walk".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "bepinex", install.path(), None).unwrap();
        assert!(result.file_count >= 1, "{:?}", result.warnings);
        let plugins = install.path().join("BepInEx/plugins");
        assert!(
            plugins.is_dir(),
            "plugins should be a directory, not a symlink to a file"
        );
        assert!(
            plugins.join("DarkMode/DarkMode.dll").exists()
                || plugins.join("DarkMode/DarkMode/DarkMode.dll").exists()
        );
    }

    #[test]
    fn stages_bare_package_file_without_extracting() {
        let tmp = tempfile::tempdir().unwrap();
        let download = tmp.path().join("CoolHair.package");
        std::fs::write(&download, b"DBPF").unwrap();
        let staging = tmp.path().join("staged");

        extract_archive(&download, &staging).unwrap();
        assert_eq!(
            std::fs::read(staging.join("CoolHair.package")).unwrap(),
            b"DBPF"
        );
    }

    #[test]
    fn safe_archive_entry_path_normalizes_backslashes() {
        assert_eq!(
            safe_archive_entry_path(r"BepInEx\plugins\Mod.dll"),
            Some(PathBuf::from("BepInEx/plugins/Mod.dll"))
        );
        assert!(safe_archive_entry_path(r"..\escape").is_none());
    }

    #[test]
    #[cfg(unix)]
    fn purge_install_symlinks_without_manifest() {
        let (_tmp, paths) = test_paths();
        let install = tempfile::tempdir().unwrap();
        let staging = paths.mods_dir("bw").join("Mod_1");
        std::fs::create_dir_all(staging.join("BepInEx/plugins/Mod")).unwrap();
        std::fs::write(staging.join("BepInEx/plugins/Mod/Mod.dll"), b"dll").unwrap();
        std::fs::write(staging.join("README.md"), b"readme").unwrap();
        std::fs::create_dir_all(install.path().join("BepInEx")).unwrap();
        std::os::unix::fs::symlink(
            staging.join("BepInEx/plugins"),
            install.path().join("BepInEx/plugins"),
        )
        .unwrap();
        std::os::unix::fs::symlink(staging.join("README.md"), install.path().join("README.md"))
            .unwrap();

        let removed = purge_install_symlinks(install.path(), &paths.data_dir).unwrap();
        assert!(removed >= 2, "removed {removed}");
        assert!(!install.path().join("BepInEx/plugins").exists());
        assert!(!install.path().join("README.md").exists());
        assert!(staging.join("BepInEx/plugins/Mod/Mod.dll").is_file());
    }

    #[test]
    #[cfg(unix)]
    fn deploy_after_orphaned_symlinks() {
        let (_tmp, paths) = test_paths();
        let game_id = "bw_orphan";
        let install = tempfile::tempdir().unwrap();
        let staging = paths.mods_dir(game_id).join("BigDart_1");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("BigDart.dll"), b"dll").unwrap();
        std::fs::create_dir_all(install.path().join("BepInEx")).unwrap();
        std::os::unix::fs::symlink(&staging, install.path().join("BepInEx/plugins")).unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "1".into(),
                    name: "BigDart".into(),
                    source: ModSource::Thunderstore,
                    domain: "big-walk".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();

        purge_deploy(&paths, game_id, Some(install.path())).unwrap();
        let result = deploy(&paths, game_id, "bepinex", install.path(), None).unwrap();
        assert!(result.file_count >= 1, "{:?}", result.warnings);
        let plugins = install.path().join("BepInEx/plugins");
        assert!(plugins.is_dir());
        assert!(
            plugins.join("BigDart.dll").is_file()
                || plugins.join("BigDart/BigDart.dll").is_file()
                || plugins.join("BigDart_1/BigDart.dll").is_file()
        );
    }

    #[test]
    fn deploy_bepinex_plugins_under_bepinexpack() {
        let (_tmp, paths) = test_paths();
        let game_id = "bw_pack";
        let install = tempfile::tempdir().unwrap();

        let pack_staging = paths.mods_dir(game_id).join("BepInExPack_1");
        std::fs::create_dir_all(
            pack_staging
                .join("BepInExPack")
                .join("BepInEx")
                .join("core"),
        )
        .unwrap();
        std::fs::write(pack_staging.join("BepInExPack").join("winhttp.dll"), b"x").unwrap();
        std::fs::write(
            pack_staging.join("BepInExPack").join("doorstop_config.ini"),
            b"target_assembly = BepInEx\\core\\BepInEx.Unity.IL2CPP.dll\n",
        )
        .unwrap();

        let mod_staging = paths.mods_dir(game_id).join("BigDart_1");
        std::fs::create_dir_all(&mod_staging).unwrap();
        std::fs::write(mod_staging.join("BigDart.dll"), b"dll").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![
                    StagedMod {
                        id: "pack".into(),
                        name: "BepInExPack".into(),
                        source: ModSource::Thunderstore,
                        domain: "big-walk".into(),
                        staging_path: pack_staging.to_string_lossy().into(),
                        enabled: true,
                        order: 1,
                        ..Default::default()
                    },
                    StagedMod {
                        id: "dart".into(),
                        name: "BigDart".into(),
                        source: ModSource::Thunderstore,
                        domain: "big-walk".into(),
                        staging_path: mod_staging.to_string_lossy().into(),
                        enabled: true,
                        order: 2,
                        ..Default::default()
                    },
                ],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "bepinex", install.path(), None).unwrap();
        assert!(result.file_count >= 2, "{:?}", result.warnings);
        assert!(install.path().join("winhttp.dll").is_file());
        assert!(install.path().join("BepInEx").join("core").is_dir());
        let plugins = install.path().join("BepInEx").join("plugins");
        assert!(plugins.is_dir(), "plugins should be under flat BepInEx");
        assert!(
            plugins.join("BigDart.dll").is_file()
                || plugins.join("BigDart/BigDart.dll").is_file()
                || plugins.join("BigDart_1/BigDart.dll").is_file()
        );
        assert!(!install
            .path()
            .join("BepInExPack")
            .join("winhttp.dll")
            .exists());
    }

    #[test]
    fn deploy_bepinexpack_flattens_thunderstore_layout() {
        let (_tmp, paths) = test_paths();
        let game_id = "bw_flat_pack";
        let install = tempfile::tempdir().unwrap();

        let pack_staging = paths.mods_dir(game_id).join("BepInExPack_1");
        std::fs::create_dir_all(
            pack_staging
                .join("BepInExPack")
                .join("BepInEx")
                .join("core"),
        )
        .unwrap();
        std::fs::write(pack_staging.join("BepInExPack").join("winhttp.dll"), b"x").unwrap();
        std::fs::write(pack_staging.join("manifest.json"), b"{}").unwrap();
        std::fs::write(pack_staging.join("icon.png"), b"png").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "pack".into(),
                    name: "BepInExPack".into(),
                    source: ModSource::Thunderstore,
                    domain: "big-walk".into(),
                    staging_path: pack_staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();

        deploy(&paths, game_id, "bepinex", install.path(), None).unwrap();
        assert!(install.path().join("winhttp.dll").is_file());
        assert!(install.path().join("BepInEx").join("core").is_dir());
        assert!(!install.path().join("manifest.json").exists());
        assert!(!install.path().join("icon.png").exists());
        assert!(!install
            .path()
            .join("BepInExPack")
            .join("winhttp.dll")
            .exists());
    }

    #[test]
    fn deploy_ror2_plugins_prefix_without_wrap() {
        let (_tmp, paths) = test_paths();
        let game_id = "ror2_plugins";
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("BepInEx").join("core")).unwrap();

        let staging = paths.mods_dir(game_id).join("R2API_1");
        std::fs::create_dir_all(staging.join("plugins").join("R2API.Legacy")).unwrap();
        std::fs::write(
            staging
                .join("plugins")
                .join("R2API.Legacy")
                .join("R2API.dll"),
            b"dll",
        )
        .unwrap();
        std::fs::write(staging.join("manifest.json"), b"{}").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "r2api".into(),
                    name: "R2API".into(),
                    source: ModSource::Thunderstore,
                    domain: "riskofrain2".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "riskofrain2", install.path(), None).unwrap();
        assert!(result.file_count >= 1, "{:?}", result.warnings);
        let dll = install
            .path()
            .join("BepInEx")
            .join("plugins")
            .join("R2API.Legacy")
            .join("R2API.dll");
        assert!(
            dll.is_file(),
            "expected flat plugins layout, got {:?}",
            std::fs::read_dir(install.path().join("BepInEx").join("plugins"))
                .map(|d| d
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name())
                    .collect::<Vec<_>>())
                .ok()
        );
        assert!(
            !install
                .path()
                .join("BepInEx")
                .join("plugins")
                .join("R2API")
                .join("plugins")
                .exists()
                && !install
                    .path()
                    .join("BepInEx")
                    .join("plugins")
                    .join("R2API_1")
                    .join("plugins")
                    .exists(),
            "must not double-wrap plugins/"
        );
    }

    #[test]
    fn deploy_bepinex_plugin_tree_under_bepinexpack() {
        let (_tmp, paths) = test_paths();
        let game_id = "bw_plugin_tree";
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("BepInEx").join("core")).unwrap();

        let mod_staging = paths.mods_dir(game_id).join("DarkMode_1");
        std::fs::create_dir_all(
            mod_staging
                .join("BepInEx")
                .join("plugins")
                .join("BigWalk.DarkMode"),
        )
        .unwrap();
        std::fs::write(
            mod_staging
                .join("BepInEx")
                .join("plugins")
                .join("BigWalk.DarkMode")
                .join("BigWalk.DarkMode.dll"),
            b"dll",
        )
        .unwrap();
        std::fs::write(mod_staging.join("manifest.json"), b"{}").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "darkmode".into(),
                    name: "DarkMode".into(),
                    source: ModSource::Thunderstore,
                    domain: "big-walk".into(),
                    staging_path: mod_staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "bepinex", install.path(), None).unwrap();
        assert!(result.file_count >= 1, "{:?}", result.warnings);
        let correct = install
            .path()
            .join("BepInEx")
            .join("plugins")
            .join("BigWalk.DarkMode")
            .join("BigWalk.DarkMode.dll");
        assert!(
            correct.is_file(),
            "expected plugin at {}",
            correct.display()
        );
    }

    #[test]
    fn deploy_never_symlinks_staging_into_the_game() {
        let (_tmp, paths) = test_paths();
        let game_id = "bw_no_symlinks";
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("BepInEx").join("core")).unwrap();

        let staging = paths.mods_dir(game_id).join("DarkMode_1");
        std::fs::create_dir_all(staging.join("plugins")).unwrap();
        std::fs::write(staging.join("plugins").join("DarkMode.dll"), b"dll").unwrap();
        std::fs::write(staging.join("plugins").join("config.json"), b"{}").unwrap();
        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "darkmode".into(),
                    name: "DarkMode".into(),
                    source: ModSource::Thunderstore,
                    domain: "big-walk".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();

        deploy(&paths, game_id, "bepinex", install.path(), None).unwrap();

        // A loader that deletes its own plugin folder must not reach staging.
        let plugins = install.path().join("BepInEx").join("plugins");
        for name in ["DarkMode.dll", "config.json"] {
            let deployed = plugins.join(name);
            assert!(
                !std::fs::symlink_metadata(&deployed)
                    .unwrap()
                    .file_type()
                    .is_symlink(),
                "{name} was symlinked into the game folder"
            );
        }
        std::fs::remove_dir_all(&plugins).unwrap();
        assert!(staging.join("plugins").join("DarkMode.dll").is_file());
        assert!(staging.join("plugins").join("config.json").is_file());
    }

    #[test]
    fn deploy_warns_when_staged_files_went_missing() {
        let (_tmp, paths) = test_paths();
        let game_id = "bw_damaged";
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("BepInEx").join("core")).unwrap();

        let staging = paths.mods_dir(game_id).join("Pack_1");
        std::fs::create_dir_all(staging.join("plugins")).unwrap();
        std::fs::write(staging.join("plugins").join("Pack.dll"), b"dll").unwrap();
        let order = LoadOrder {
            mods: vec![StagedMod {
                id: "pack".into(),
                name: "Pack".into(),
                source: ModSource::Thunderstore,
                domain: "big-walk".into(),
                staging_path: staging.to_string_lossy().into(),
                enabled: true,
                order: 1,
                // Extraction produced five files; four are gone.
                staged_file_count: Some(5),
                ..Default::default()
            }],
        };
        save_loadorder(&paths, game_id, &order).unwrap();

        assert_eq!(damaged_staging(&order).len(), 1);
        let result = deploy(&paths, game_id, "bepinex", install.path(), None).unwrap();
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("Pack is missing 4 of 5")),
            "{:?}",
            result.warnings
        );
    }

    #[test]
    fn purge_removes_empty_plugin_folders_left_behind() {
        let (_tmp, paths) = test_paths();
        let game_id = "bw_prune";
        let install = tempfile::tempdir().unwrap();
        let plugins = install.path().join("BepInEx").join("plugins");
        std::fs::create_dir_all(install.path().join("BepInEx").join("core")).unwrap();
        std::fs::create_dir_all(plugins.join("GoneMod").join("nested")).unwrap();
        std::fs::create_dir_all(plugins.join("StillHere")).unwrap();
        std::fs::write(plugins.join("StillHere").join("mod.dll"), b"dll").unwrap();

        purge_deploy(&paths, game_id, Some(install.path())).unwrap();
        let removed = prune_empty_loader_dirs(
            crate::games::plugin_by_id("bepinex").unwrap(),
            install.path(),
        );

        assert_eq!(removed, 2);
        assert!(!plugins.join("GoneMod").exists());
        assert!(plugins.join("StillHere").join("mod.dll").is_file());
    }

    #[test]
    fn deploy_zero_enabled_reports_warning() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_empty";
        let install = tempfile::tempdir().unwrap();
        save_loadorder(&paths, game_id, &LoadOrder::default()).unwrap();
        let result = deploy(&paths, game_id, "cyberpunk2077", install.path(), None).unwrap();
        assert_eq!(result.file_count, 0);
        assert_eq!(result.enabled_mods, 0);
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn deploy_missing_staging_warns() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_missing";
        let install = tempfile::tempdir().unwrap();
        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "9_9".into(),
                    name: "Gone".into(),
                    nexus_mod_id: 9,
                    nexus_file_id: 9,
                    version: None,
                    domain: "cyberpunk2077".into(),
                    staging_path: paths
                        .mods_dir(game_id)
                        .join("does_not_exist")
                        .to_string_lossy()
                        .into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();
        let result = deploy(&paths, game_id, "cyberpunk2077", install.path(), None).unwrap();
        assert_eq!(result.file_count, 0);
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("Missing staging")));
    }

    #[test]
    fn deploy_redmod_wraps_info_json() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_redmod";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("Red_4_4");
        let inner = staging.join("MyRedMod");
        std::fs::create_dir_all(inner.join("archives")).unwrap();
        std::fs::write(inner.join("info.json"), r#"{"name":"MyRedMod"}"#).unwrap();
        std::fs::write(inner.join("archives").join("x.archive"), b"a").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "4_4".into(),
                    name: "Nexus REDmod Title".into(),
                    nexus_mod_id: 4,
                    nexus_file_id: 4,
                    version: None,
                    domain: "cyberpunk2077".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "cyberpunk2077", install.path(), None).unwrap();
        assert!(result.file_count >= 2, "{:?}", result.warnings);
        assert!(install.path().join("mods/MyRedMod/info.json").exists());
        assert!(!install.path().join("mods/Nexus REDmod Title").exists());
    }

    fn stage_stardew(paths: &Paths, game_id: &str, id: &str, name: &str, staging: PathBuf) {
        save_loadorder(
            paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: id.into(),
                    name: name.into(),
                    nexus_mod_id: 1,
                    nexus_file_id: 1,
                    version: None,
                    domain: "stardewvalley".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();
    }

    #[test]
    fn deploy_stardew_wraps_manifest_mod() {
        let (_tmp, paths) = test_paths();
        let game_id = "sdv_wrap";
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("smapi-internal")).unwrap();

        let staging = paths.mods_dir(game_id).join("Cool_1_1");
        let inner = staging.join("CoolMod");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(inner.join("manifest.json"), r#"{"UniqueID":"A.Cool"}"#).unwrap();
        std::fs::write(inner.join("Cool.dll"), b"dll").unwrap();
        stage_stardew(&paths, game_id, "1_1", "Cool Mod", staging);

        let result = deploy(&paths, game_id, "stardewvalley", install.path(), None).unwrap();
        assert!(result.file_count >= 2, "{:?}", result.warnings);
        assert!(install.path().join("Mods/CoolMod/manifest.json").exists());
        assert!(install.path().join("Mods/CoolMod/Cool.dll").exists());
        assert!(!install.path().join("Mods/Cool Mod/manifest.json").exists());
        assert!(!result
            .warnings
            .iter()
            .any(|w| w.contains("SMAPI not found")));
    }

    #[test]
    fn deploy_stardew_mods_prefix_no_double_nest() {
        let (_tmp, paths) = test_paths();
        let game_id = "sdv_mods";
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("smapi-internal")).unwrap();

        let staging = paths.mods_dir(game_id).join("Pack_2_2");
        std::fs::create_dir_all(staging.join("Mods").join("CoolMod")).unwrap();
        std::fs::write(
            staging.join("Mods").join("CoolMod").join("manifest.json"),
            "{}",
        )
        .unwrap();
        stage_stardew(&paths, game_id, "2_2", "Pack", staging);

        let result = deploy(&paths, game_id, "stardewvalley", install.path(), None).unwrap();
        assert_eq!(result.file_count, 1, "{:?}", result.warnings);
        assert!(install.path().join("Mods/CoolMod/manifest.json").exists());
        assert!(!install
            .path()
            .join("Mods/Pack/Mods/CoolMod/manifest.json")
            .exists());
    }

    #[test]
    fn deploy_stardew_multi_mod_siblings() {
        let (_tmp, paths) = test_paths();
        let game_id = "sdv_multi";
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("smapi-internal")).unwrap();

        let staging = paths.mods_dir(game_id).join("Bundle_3_3");
        for name in ["ModA", "ModB"] {
            let dir = staging.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("manifest.json"), "{}").unwrap();
        }
        stage_stardew(&paths, game_id, "3_3", "Bundle", staging);

        let result = deploy(&paths, game_id, "stardewvalley", install.path(), None).unwrap();
        assert_eq!(result.file_count, 2, "{:?}", result.warnings);
        assert!(install.path().join("Mods/ModA/manifest.json").exists());
        assert!(install.path().join("Mods/ModB/manifest.json").exists());
    }

    #[test]
    fn deploy_sims4_wraps_mods_and_skips_unloadable_files() {
        let (_tmp, paths) = test_paths();
        let game_id = "sims4_deploy";
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp
            .path()
            .join("steamapps")
            .join("common")
            .join("The Sims 4");
        std::fs::create_dir_all(&install).unwrap();
        let user_dir = tmp
            .path()
            .join("steamapps")
            .join("compatdata")
            .join("1222670")
            .join("pfx")
            .join("drive_c")
            .join("users")
            .join("steamuser")
            .join("Documents")
            .join("Electronic Arts")
            .join("The Sims 4");
        std::fs::create_dir_all(&user_dir).unwrap();

        let staging = paths.mods_dir(game_id).join("MCCC_7_7");
        std::fs::create_dir_all(staging.join("scripts")).unwrap();
        std::fs::write(staging.join("mccc.package"), b"pkg").unwrap();
        std::fs::write(staging.join("scripts").join("core.ts4script"), b"zip").unwrap();
        std::fs::write(staging.join("README.txt"), b"docs").unwrap();
        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "7_7".into(),
                    name: "MCCC".into(),
                    nexus_mod_id: 7,
                    nexus_file_id: 7,
                    version: None,
                    domain: "thesims4".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "sims4", &install, None).unwrap();
        let mods = user_dir.join("Mods");
        assert_eq!(result.file_count, 2, "{:?}", result.warnings);
        assert!(mods.join("MCCC").join("mccc.package").exists());
        // Flattened up from scripts/ so the game can still load it.
        assert!(mods.join("MCCC").join("scripts_core.ts4script").exists());
        assert!(!mods.join("MCCC").join("README.txt").exists());
        assert!(mods.join("Resource.cfg").is_file());
        assert!(
            result.warnings.iter().any(|w| w.contains("skipped")),
            "{:?}",
            result.warnings
        );

        // Purge must clean the user-data folder, not just the install.
        purge_deploy(&paths, game_id, Some(&install)).unwrap();
        assert!(!mods.join("MCCC").join("mccc.package").exists());
    }

    #[test]
    fn deploy_sims3_routes_packages_and_sims3packs() {
        let (_tmp, paths) = test_paths();
        let game_id = "sims3_deploy";
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp
            .path()
            .join("steamapps")
            .join("common")
            .join("The Sims 3");
        std::fs::create_dir_all(&install).unwrap();
        let user_dir = tmp
            .path()
            .join("steamapps")
            .join("compatdata")
            .join("47890")
            .join("pfx")
            .join("drive_c")
            .join("users")
            .join("steamuser")
            .join("Documents")
            .join("Electronic Arts")
            .join("The Sims 3");
        std::fs::create_dir_all(&user_dir).unwrap();

        let staging = paths.mods_dir(game_id).join("HairSet_8_8");
        std::fs::create_dir_all(staging.join("extras")).unwrap();
        std::fs::write(staging.join("hair.package"), b"pkg").unwrap();
        std::fs::write(staging.join("extras").join("bonus.sims3pack"), b"s3p").unwrap();
        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "8_8".into(),
                    name: "Hair Set".into(),
                    nexus_mod_id: 8,
                    nexus_file_id: 8,
                    version: None,
                    domain: "thesims3".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();

        let result = deploy(&paths, game_id, "sims3", &install, None).unwrap();
        assert_eq!(result.file_count, 2, "{:?}", result.warnings);
        assert!(user_dir
            .join("Mods")
            .join("Packages")
            .join("Hair Set")
            .join("hair.package")
            .exists());
        // The Launcher only reads the top level of Downloads.
        assert!(user_dir.join("Downloads").join("bonus.sims3pack").exists());
        assert!(user_dir.join("Mods").join("Resource.cfg").is_file());
        assert!(
            result
                .warnings
                .iter()
                .any(|w| w.contains("Sims 3 Launcher")),
            "{:?}",
            result.warnings
        );
    }

    #[test]
    fn deploy_stardew_warns_without_smapi() {
        let (_tmp, paths) = test_paths();
        let game_id = "sdv_nosmapi";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("M_4_4");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("manifest.json"), "{}").unwrap();
        stage_stardew(&paths, game_id, "4_4", "M", staging);

        let result = deploy(&paths, game_id, "stardewvalley", install.path(), None).unwrap();
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("SMAPI not found")));
        assert!(install.path().join("Mods/M/manifest.json").exists());
    }

    #[test]
    fn deploy_stardew_smapi_installer_to_root() {
        let (_tmp, paths) = test_paths();
        let game_id = "sdv_smapi";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("SMAPI_5_5");
        let inner = staging.join("SMAPI 4.0");
        std::fs::create_dir_all(inner.join("smapi-internal")).unwrap();
        std::fs::write(inner.join("StardewModdingAPI.exe"), b"exe").unwrap();
        std::fs::write(inner.join("smapi-internal").join("config.json"), b"{}").unwrap();
        stage_stardew(&paths, game_id, "5_5", "SMAPI", staging);

        let result = deploy(&paths, game_id, "stardewvalley", install.path(), None).unwrap();
        assert!(result.file_count >= 2, "{:?}", result.warnings);
        assert!(install.path().join("StardewModdingAPI.exe").exists());
        assert!(install.path().join("smapi-internal/config.json").exists());
        assert!(!install
            .path()
            .join("Mods/SMAPI/StardewModdingAPI.exe")
            .exists());
        assert!(result
            .warnings
            .iter()
            .any(|w| w.contains("SMAPI installer")));
    }

    #[test]
    fn deploy_darktide_writes_load_order_and_skips_dmf() {
        let (_tmp, paths) = test_paths();
        let game_id = "dt_test";
        let install = tempfile::tempdir().unwrap();
        // Already patched so we do not spawn dtkit-patch.
        std::fs::create_dir_all(install.path().join("bundle")).unwrap();
        std::fs::write(
            install.path().join("bundle/bundle_database.data"),
            b"xxxpatch_999yyy",
        )
        .unwrap();

        let holy_light_staging = paths.mods_dir(game_id).join("Holy Light_1_1");
        let holy_light_inner = holy_light_staging.join("HolyLight");
        std::fs::create_dir_all(&holy_light_inner).unwrap();
        std::fs::write(holy_light_inner.join("HolyLight.mod"), b"mod").unwrap();

        let dmf_staging = paths.mods_dir(game_id).join("DMF_2_2");
        std::fs::create_dir_all(dmf_staging.join("dmf")).unwrap();
        std::fs::write(dmf_staging.join("dmf").join("dmf.lua"), b"lua").unwrap();

        let health_staging = paths.mods_dir(game_id).join("Healthbars_3_3");
        let health_inner = health_staging.join("healthbars");
        std::fs::create_dir_all(&health_inner).unwrap();
        std::fs::write(health_inner.join("healthbars.mod"), b"mod").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![
                    StagedMod {
                        id: "1_1".into(),
                        name: "Holy Light".into(),
                        nexus_mod_id: 1,
                        nexus_file_id: 1,
                        version: None,
                        domain: "warhammer40kdarktide".into(),
                        staging_path: holy_light_staging.to_string_lossy().into(),
                        enabled: true,
                        order: 1,
                        ..Default::default()
                    },
                    StagedMod {
                        id: "2_2".into(),
                        name: "Darktide Mod Framework".into(),
                        nexus_mod_id: 2,
                        nexus_file_id: 2,
                        version: None,
                        domain: "warhammer40kdarktide".into(),
                        staging_path: dmf_staging.to_string_lossy().into(),
                        enabled: true,
                        order: 2,
                        ..Default::default()
                    },
                    StagedMod {
                        id: "3_3".into(),
                        name: "Healthbars".into(),
                        nexus_mod_id: 3,
                        nexus_file_id: 3,
                        version: None,
                        domain: "warhammer40kdarktide".into(),
                        staging_path: health_staging.to_string_lossy().into(),
                        enabled: false,
                        order: 3,
                        ..Default::default()
                    },
                ],
            },
        )
        .unwrap();

        let result = deploy(
            &paths,
            game_id,
            "warhammer40kdarktide",
            install.path(),
            None,
        )
        .unwrap();
        assert!(result.file_count >= 2, "{:?}", result.warnings);
        assert!(install.path().join("mods/HolyLight/HolyLight.mod").exists());
        assert!(!install.path().join("mods/Holy Light").exists());
        assert!(install.path().join("mods/dmf/dmf.lua").exists());
        assert!(!install.path().join("mods/Healthbars").exists());

        let order_txt =
            std::fs::read_to_string(install.path().join("mods/mod_load_order.txt")).unwrap();
        assert!(order_txt.contains("HolyLight\n"));
        assert!(!order_txt.contains("Holy Light\n"));
        assert!(!order_txt.lines().any(|l| l.trim() == "dmf"));
        assert!(!order_txt.contains("Healthbars"));
        assert!(!order_txt.contains("Darktide Mod Framework"));
    }

    #[test]
    fn deploy_darktide_loader_root_layout() {
        let (_tmp, paths) = test_paths();
        let game_id = "dt_loader";
        let install = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(install.path().join("bundle")).unwrap();
        std::fs::write(
            install.path().join("bundle/bundle_database.data"),
            b"xxxpatch_999yyy",
        )
        .unwrap();

        let staging = paths.mods_dir(game_id).join("Loader_1_1");
        std::fs::create_dir_all(staging.join("tools")).unwrap();
        std::fs::create_dir_all(staging.join("binaries")).unwrap();
        std::fs::create_dir_all(staging.join("mods")).unwrap();
        std::fs::write(staging.join("tools/dtkit-patch.exe"), b"fake").unwrap();
        std::fs::write(staging.join("binaries/mod_loader"), b"x").unwrap();
        std::fs::write(staging.join("toggle_darktide_mods.bat"), b"bat").unwrap();
        std::fs::write(staging.join("mods/mod_load_order.txt"), b"-- template\n").unwrap();

        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![StagedMod {
                    id: "1_1".into(),
                    name: "Darktide Mod Loader".into(),
                    nexus_mod_id: 19,
                    nexus_file_id: 1,
                    version: None,
                    domain: "warhammer40kdarktide".into(),
                    staging_path: staging.to_string_lossy().into(),
                    enabled: true,
                    order: 1,
                    ..Default::default()
                }],
            },
        )
        .unwrap();

        let result = deploy(
            &paths,
            game_id,
            "warhammer40kdarktide",
            install.path(),
            None,
        )
        .unwrap();
        assert!(result.file_count >= 3, "{:?}", result.warnings);
        assert!(install.path().join("tools/dtkit-patch.exe").exists());
        assert!(install.path().join("binaries/mod_loader").exists());
        assert!(install.path().join("toggle_darktide_mods.bat").exists());
        // Loader is not wrapped → not listed in load order (rewritten empty aside from header).
        let order_txt =
            std::fs::read_to_string(install.path().join("mods/mod_load_order.txt")).unwrap();
        assert!(!order_txt
            .lines()
            .any(|l| !l.starts_with("--") && !l.trim().is_empty()));
    }

    #[test]
    fn incoming_version_newest_wins() {
        assert!(incoming_is_newer(None, "1.2.3"));
        assert!(!incoming_is_newer(Some("1.2.3"), "1.2.3"));
        assert!(incoming_is_newer(Some("1.2.3"), "1.2.10"));
        assert!(!incoming_is_newer(Some("1.2.10"), "1.2.3"));
        assert!(incoming_is_newer(Some("build"), "other"));
        assert!(!incoming_is_newer(Some("1.0.0"), "weird"));
        assert!(incoming_is_newer(Some("weird"), "1.0.0"));
    }

    fn nexus_file(
        file_id: u64,
        version: &str,
        category: &str,
        uploaded: u64,
        is_primary: bool,
    ) -> NexusFileMeta {
        NexusFileMeta {
            file_id,
            version: Some(version.into()),
            category_name: Some(category.into()),
            uploaded_timestamp: Some(uploaded),
            is_primary,
        }
    }

    #[test]
    fn nexus_same_file_no_update() {
        let files = vec![nexus_file(10, "1.0.0", "MAIN", 100, true)];
        assert!(nexus_update_candidate(10, Some("1.0.0"), &files).is_none());
    }

    #[test]
    fn nexus_newer_same_category_is_update() {
        let files = vec![
            nexus_file(10, "1.0.0", "MAIN", 100, false),
            nexus_file(20, "1.1.0", "MAIN", 200, true),
        ];
        let c = nexus_update_candidate(10, Some("1.0.0"), &files).unwrap();
        assert_eq!(c.file_id, 20);
        assert_eq!(c.version.as_deref(), Some("1.1.0"));
    }

    #[test]
    fn nexus_optional_not_replaced_by_main() {
        let files = vec![
            nexus_file(10, "1.0.0", "OPTIONAL", 100, false),
            nexus_file(20, "2.0.0", "MAIN", 200, true),
            nexus_file(11, "1.0.1", "OPTIONAL", 150, false),
        ];
        let c = nexus_update_candidate(10, Some("1.0.0"), &files).unwrap();
        assert_eq!(c.file_id, 11);
    }

    #[test]
    fn nexus_missing_staged_falls_back_to_primary() {
        let files = vec![
            nexus_file(20, "2.0.0", "MAIN", 200, true),
            nexus_file(30, "1.0.0", "OPTIONAL", 300, false),
        ];
        let c = nexus_update_candidate(99, Some("1.0.0"), &files).unwrap();
        assert_eq!(c.file_id, 20);
    }

    #[test]
    fn uninstall_keeps_shared_requirements() {
        let (_tmp, paths) = test_paths();
        let game_id = "col_test";
        crate::config::ensure_game_dirs(&paths, game_id).unwrap();
        let staging_root = paths.mods_dir(game_id);
        for id in ["A", "B", "Z", "W"] {
            std::fs::create_dir_all(staging_root.join(id)).unwrap();
        }
        let staged =
            |id: &str, independent: bool, depends_on: Vec<String>, collection_ids: Vec<String>| {
                StagedMod {
                    id: id.into(),
                    name: id.into(),
                    staging_path: staging_root.join(id).to_string_lossy().into(),
                    independent,
                    depends_on,
                    collection_ids,
                    ..Default::default()
                }
            };
        save_loadorder(
            &paths,
            game_id,
            &LoadOrder {
                mods: vec![
                    staged("A", false, vec!["Z".into()], vec!["col-a".into()]),
                    staged("Z", false, vec![], vec!["col-a".into()]),
                    staged("W", true, vec!["Z".into()], vec![]),
                    staged("B", false, vec![], vec!["col-b".into()]),
                ],
            },
        )
        .unwrap();
        save_collections(
            &paths,
            game_id,
            &InstalledCollections {
                collections: vec![
                    InstalledCollection {
                        id: "col-a".into(),
                        source: CollectionSource::Nexus,
                        kind: CollectionKind::Collection,
                        name: "A".into(),
                        slug: Some("a".into()),
                        namespace: None,
                        package_name: None,
                        community: None,
                        revision: None,
                        version: None,
                        profile_code: None,
                        mod_ids: vec!["A".into(), "Z".into()],
                        installed_at: "t".into(),
                    },
                    InstalledCollection {
                        id: "col-b".into(),
                        source: CollectionSource::Nexus,
                        kind: CollectionKind::Collection,
                        name: "B".into(),
                        slug: Some("b".into()),
                        namespace: None,
                        package_name: None,
                        community: None,
                        revision: None,
                        version: None,
                        profile_code: None,
                        mod_ids: vec!["B".into()],
                        installed_at: "t".into(),
                    },
                ],
            },
        )
        .unwrap();

        let removed = uninstall_collection(&paths, game_id, "col-a").unwrap();
        assert_eq!(removed, 1);
        let order = load_loadorder(&paths, game_id).unwrap();
        let ids: Vec<_> = order.mods.iter().map(|m| m.id.as_str()).collect();
        assert!(ids.contains(&"Z"));
        assert!(ids.contains(&"W"));
        assert!(ids.contains(&"B"));
        assert!(!ids.contains(&"A"));
    }
}
