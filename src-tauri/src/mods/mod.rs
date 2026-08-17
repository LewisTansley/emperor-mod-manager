//! Mod staging, extraction, load order, and deploy.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::{
    config::Paths,
    games::{
        normalize_relative, normalize_staging_root, plugin_by_id, DeployContext, GamePlugin,
    },
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
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CollectionSource {
    #[default]
    Nexus,
    Thunderstore,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CollectionKind {
    #[default]
    Collection,
    Modpack,
    Profile,
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
        m.depends_on.retain(|s| !s.is_empty() && seen.insert(s.clone()));
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
    if let Some(existing) = store
        .collections
        .iter_mut()
        .find(|c| c.id == collection.id)
    {
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
        c.mod_ids.retain(|id| order.mods.iter().any(|m| &m.id == id));
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
        ..Default::default()
    };
    order.mods.push(staged.clone());
    save_loadorder(paths, game_id, &order)?;
    Ok(staged)
}

pub fn extract_archive(archive: &Path, dest: &Path) -> Result<()> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;

    let ext = archive
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "zip" => extract_zip(archive, dest),
        "7z" => extract_7z(archive, dest),
        "rar" => extract_rar(archive, dest),
        other => {
            // Some Nexus files have no extension or odd names — try zip, 7z, then RAR magic
            if extract_zip(archive, dest).is_ok() {
                Ok(())
            } else if extract_7z(archive, dest).is_ok() {
                Ok(())
            } else if looks_like_rar(archive) {
                extract_rar(archive, dest)
            } else {
                bail!("unsupported archive type: .{other}");
            }
        }
    }
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let file = fs::File::open(archive)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = match file.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue,
        };
        if file.name().ends_with('/') {
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

/// RAR signature: `Rar!` followed by `0x1A 0x07` (RAR 1.5+ / RAR 5).
fn looks_like_rar(archive: &Path) -> bool {
    let Ok(mut file) = fs::File::open(archive) else {
        return false;
    };
    let mut magic = [0u8; 6];
    match file.read(&mut magic) {
        Ok(n) if n >= 6 => {}
        _ => return false,
    }
    magic[0..4] == *b"Rar!" && magic[4] == 0x1a && magic[5] == 0x07
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
    crate::config::ensure_game_dirs(paths, game_id)?;
    let safe = sanitize_filename::sanitize(name);
    let staging = paths
        .mods_dir(game_id)
        .join(format!("{safe}_{mod_id}_{file_id}"));
    extract_archive(archive, &staging)?;

    let mut order = load_loadorder(paths, game_id)?;
    let old = take_nexus_row(&mut order, mod_id, file_id);
    let provenance = provenance_from_removed(old, &staging);
    let next_order = provenance
        .order
        .unwrap_or_else(|| order.mods.iter().map(|m| m.order).max().unwrap_or(0) + 1);
    let staged = StagedMod {
        id: format!("{mod_id}_{file_id}"),
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
    };
    order.mods.push(staged.clone());
    save_loadorder(paths, game_id, &order)?;
    Ok(staged)
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
    Ok(order.mods.iter().any(|m| {
        m.source == ModSource::Modio && m.modio_mod_id == Some(modio_mod_id)
    }))
}

pub fn set_enabled(paths: &Paths, game_id: &str, mod_uid: &str, enabled: bool) -> Result<()> {
    let mut order = load_loadorder(paths, game_id)?;
    let Some(m) = order.mods.iter_mut().find(|m| m.id == mod_uid) else {
        bail!("mod not found: {mod_uid}");
    };
    m.enabled = enabled;
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

pub fn remove_all_mods(paths: &Paths, game_id: &str) -> Result<()> {
    purge_deploy(paths, game_id)?;
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
    HardlinkOrSymlink,
    Copied,
}

fn link_or_copy(src: &Path, dest: &Path) -> Result<LinkKind> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    if dest.exists() {
        if dest.is_dir() {
            fs::remove_dir_all(dest)?;
        } else {
            fs::remove_file(dest)?;
        }
    }
    // Hardlink files; directories are created normally and children linked.
    if src.is_dir() {
        fs::create_dir_all(dest)?;
        return Ok(LinkKind::HardlinkOrSymlink);
    }
    match fs::hard_link(src, dest) {
        Ok(()) => Ok(LinkKind::HardlinkOrSymlink),
        Err(_) => match symlink_file(src, dest) {
            Ok(()) => Ok(LinkKind::HardlinkOrSymlink),
            Err(e) => {
                log::debug!(
                    "symlink failed for {} -> {} ({e}); copying instead",
                    src.display(),
                    dest.display()
                );
                fs::copy(src, dest)?;
                Ok(LinkKind::Copied)
            }
        },
    }
}

#[cfg(unix)]
fn symlink_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(src, dest)
}

#[cfg(windows)]
fn symlink_file(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(src, dest)
}

#[cfg(not(any(unix, windows)))]
fn symlink_file(_src: &Path, dest: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        format!("symlink unsupported on this platform for {}", dest.display()),
    ))
}

pub fn purge_deploy(paths: &Paths, game_id: &str) -> Result<()> {
    let manifest_path = paths.deploy_manifest(game_id);
    if !manifest_path.exists() {
        return Ok(());
    }
    let raw = fs::read_to_string(&manifest_path)?;
    let manifest: DeployManifest = serde_json::from_str(&raw)?;
    // Remove files first, then empty dirs (reverse sort by path length)
    let mut entries = manifest.paths;
    entries.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for p in entries {
        let path = PathBuf::from(&p);
        if path.is_file() || path.is_symlink() {
            let _ = fs::remove_file(&path);
        } else if path.is_dir() {
            let _ = fs::remove_dir(&path); // only if empty
        }
    }
    fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&DeployManifest::default())?,
    )?;
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
    };

    if !install_path.is_dir() {
        bail!(
            "Game install path does not exist or is not a directory: {}",
            install_path.display()
        );
    }

    warnings.extend(plugin.prepare_deploy(install_path)?);
    warnings.extend(plugin.preflight_warnings_ctx(install_path, &base_ctx));

    purge_deploy(paths, game_id)?;

    let mut order = load_loadorder(paths, game_id)?;
    order.mods.sort_by_key(|m| m.order);

    let mut deployed = DeployManifest::default();
    let mut count = 0usize;
    let mut copied_files = 0usize;
    let enabled: Vec<_> = order.mods.iter().filter(|m| m.enabled).cloned().collect();
    let enabled_mods = enabled.len();
    let mut enabled_mod_folders = Vec::new();

    for staged in &enabled {
        let staging = PathBuf::from(&staged.staging_path);
        if !staging.exists() {
            let msg = format!(
                "Missing staging for {}: {}",
                staged.name,
                staging.display()
            );
            log::warn!("{msg}");
            warnings.push(msg);
            continue;
        }
        let root = normalize_staging_root(&staging, plugin)?;
        warnings.extend(plugin.staging_deploy_warnings(&root, &staged.name));
        let to_root = plugin.deploys_to_install_root(&root);
        let wrap = !to_root
            && (plugin.prefers_mod_folder() || plugin.should_wrap_as_mod_folder(&root));
        if wrap {
            let folder_name = plugin.wrap_mod_folder_name(&root, &staged.name);
            if !folder_name.is_empty() {
                enabled_mod_folders.push(folder_name);
            }
        }
        let (files, copied) = deploy_tree(
            plugin,
            install_path,
            &root,
            &staged.name,
            project_name,
            &mut deployed,
        )?;
        if files == 0 {
            warnings.push(format!("No files deployed from {}", staged.name));
        }
        if copied > 0 {
            copied_files += copied;
        }
        count += files;
    }

    if copied_files > 0 {
        warnings.push(format!(
            "{copied_files} file(s) were copied instead of hardlinked/symlinked (cross-volume or missing symlink privilege). On Windows, enable Developer Mode for symlink deploy."
        ));
    }

    if enabled_mods == 0 {
        warnings.push("No enabled mods to deploy.".into());
    } else if count == 0 {
        warnings.push("Deploy finished with 0 files linked.".into());
    }

    warnings.extend(plugin.after_deploy(install_path, &enabled_mod_folders)?);

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

fn deploy_tree(
    plugin: &dyn GamePlugin,
    install_path: &Path,
    content_root: &Path,
    mod_name: &str,
    project_name: Option<&str>,
    deployed: &mut DeployManifest,
) -> Result<(usize, usize)> {
    let mut n = 0usize;
    let mut copied = 0usize;
    let to_root = plugin.deploys_to_install_root(content_root);
    let wrap = !to_root
        && (plugin.prefers_mod_folder() || plugin.should_wrap_as_mod_folder(content_root));
    let folder_name = plugin.wrap_mod_folder_name(content_root, mod_name);
    let ctx = DeployContext {
        project_name,
        content_root: Some(content_root),
    };

    for entry in WalkDir::new(content_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path == content_root {
            continue;
        }
        let rel = path.strip_prefix(content_root)?;
        let rel = normalize_relative(rel);
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
            fs::create_dir_all(&dest)?;
            deployed.paths.push(dest.to_string_lossy().to_string());
            continue;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        if link_or_copy(path, &dest)? == LinkKind::Copied {
            copied += 1;
        }
        deployed.paths.push(dest.to_string_lossy().to_string());
        n += 1;
    }
    Ok((n, copied))
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

    #[test]
    fn deploy_cyberpunk_preserves_bin_and_red4ext() {
        let (_tmp, paths) = test_paths();
        let game_id = "cp_test";
        let install = tempfile::tempdir().unwrap();

        let staging = paths.mods_dir(game_id).join("CET_1_1");
        std::fs::create_dir_all(staging.join("bin").join("x64").join("plugins")).unwrap();
        std::fs::write(
            staging.join("bin").join("x64").join("version.dll"),
            b"dll",
        )
        .unwrap();
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
        assert!(install
            .path()
            .join("archive/pc/mod/Cool.archive")
            .exists());
        assert!(install.path().join("r6/scripts/Mod/mod.reds").exists());
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
        assert!(result.warnings.iter().any(|w| w.contains("Missing staging")));
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

    fn stage_stardew(
        paths: &Paths,
        game_id: &str,
        id: &str,
        name: &str,
        staging: PathBuf,
    ) {
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
        assert!(!result.warnings.iter().any(|w| w.contains("SMAPI not found")));
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

        let result = deploy(&paths, game_id, "warhammer40kdarktide", install.path(), None).unwrap();
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

        let result = deploy(&paths, game_id, "warhammer40kdarktide", install.path(), None).unwrap();
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

    #[test]
    fn uninstall_keeps_shared_requirements() {
        let (_tmp, paths) = test_paths();
        let game_id = "col_test";
        crate::config::ensure_game_dirs(&paths, game_id).unwrap();
        let staging_root = paths.mods_dir(game_id);
        for id in ["A", "B", "Z", "W"] {
            std::fs::create_dir_all(staging_root.join(id)).unwrap();
        }
        let staged = |id: &str, independent: bool, depends_on: Vec<String>, collection_ids: Vec<String>| {
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
