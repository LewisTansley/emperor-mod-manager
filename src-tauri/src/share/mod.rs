//! Self-contained Emperor share codes for cross-platform loadout sharing.

use std::io::{Read, Write};

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine;
use flate2::{read::DeflateDecoder, write::DeflateEncoder, Compression};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    config::{ManagedGame, Paths},
    mods::{LoadOrder, ModSource, StagedMod},
};

pub const SHARE_PREFIX: &str = "#emperor1\n";
const SOFT_SIZE_WARN: usize = 100 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportCodeKind {
    Emperor,
    ThunderstoreProfile,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShareModSource {
    Nexus,
    Thunderstore,
    Modio,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareGameHint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus_domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thunderstore_community: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modio_game_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareModEntry {
    pub s: ShareModSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mod_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareManifest {
    pub v: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub game: ShareGameHint,
    pub mods: Vec<ShareModEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportShareResult {
    pub code: String,
    pub warnings: Vec<String>,
    pub skipped: Vec<String>,
    pub mod_count: usize,
    pub saved: SavedCollectionEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareDecodePreview {
    pub name: Option<String>,
    pub game: ShareGameHint,
    pub mods: Vec<ShareModEntry>,
    pub nexus_count: usize,
    pub thunderstore_count: usize,
    pub modio_count: usize,
    pub code_len: usize,
    pub size_warning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectImportResult {
    pub kind: ImportCodeKind,
    pub preview: Option<ShareDecodePreview>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareAssistFile {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShareImportResult {
    pub collection_id: String,
    pub name: String,
    pub member_ids: Vec<String>,
    pub needs_assist: Vec<ShareAssistFile>,
    pub warnings: Vec<String>,
    /// Present when install finished without Assist (Premium or no Nexus files left).
    pub collection: Option<crate::mods::InstalledCollection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedCollectionEntry {
    pub id: String,
    pub name: String,
    pub code: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_game_id: Option<String>,
    pub game: ShareGameHint,
    pub mod_count: usize,
    pub nexus_count: usize,
    pub thunderstore_count: usize,
    pub modio_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SavedCollectionsStore {
    pub collections: Vec<SavedCollectionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedCollectionDetail {
    pub entry: SavedCollectionEntry,
    pub mods: Vec<ShareModEntry>,
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn content_hash(code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(code.as_bytes());
    hex::encode(hasher.finalize())[..16].to_string()
}

pub fn emperor_share_id(code: &str) -> String {
    format!("emperor-share:{}", content_hash(code))
}

pub fn encode_share_manifest(manifest: &ShareManifest) -> Result<String> {
    let json = serde_json::to_vec(manifest).context("serialize share manifest")?;
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&json).context("deflate share")?;
    let compressed = encoder.finish().context("finish deflate")?;
    let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(compressed);
    Ok(format!("{SHARE_PREFIX}{encoded}"))
}

pub fn decode_share_code(raw: &str) -> Result<ShareManifest> {
    let trimmed = raw.trim();
    let encoded = trimmed
        .strip_prefix(SHARE_PREFIX)
        .or_else(|| {
            trimmed
                .strip_prefix("#emperor1")
                .map(|s| s.trim_start_matches(['\n', '\r', ' ']))
        })
        .ok_or_else(|| anyhow!("not an Emperor share code (missing #emperor1 prefix)"))?;
    let compressed = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(encoded.trim().as_bytes())
        .or_else(|_| {
            base64::engine::general_purpose::STANDARD.decode(encoded.trim().as_bytes())
        })
        .context("decode share payload")?;
    let mut decoder = DeflateDecoder::new(compressed.as_slice());
    let mut json = Vec::new();
    decoder
        .read_to_end(&mut json)
        .context("inflate share payload")?;
    let manifest: ShareManifest =
        serde_json::from_slice(&json).context("parse share manifest")?;
    if manifest.v != 1 {
        bail!("unsupported share format version {}", manifest.v);
    }
    Ok(manifest)
}

pub fn preview_from_manifest(manifest: &ShareManifest, code_len: usize) -> ShareDecodePreview {
    let mut nexus_count = 0usize;
    let mut thunderstore_count = 0usize;
    let mut modio_count = 0usize;
    for m in &manifest.mods {
        match m.s {
            ShareModSource::Nexus => nexus_count += 1,
            ShareModSource::Thunderstore => thunderstore_count += 1,
            ShareModSource::Modio => modio_count += 1,
        }
    }
    ShareDecodePreview {
        name: manifest.name.clone(),
        game: manifest.game.clone(),
        mods: manifest.mods.clone(),
        nexus_count,
        thunderstore_count,
        modio_count,
        code_len,
        size_warning: code_len > SOFT_SIZE_WARN,
    }
}

pub fn detect_code_kind(raw: &str) -> ImportCodeKind {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return ImportCodeKind::Unknown;
    }
    if trimmed.starts_with("#emperor1") || decode_share_code(trimmed).is_ok() {
        return ImportCodeKind::Emperor;
    }
    if trimmed.starts_with("#r2modman") {
        return ImportCodeKind::ThunderstoreProfile;
    }
    // Legacy Thunderstore profile keys are short alphanumeric tokens (no whitespace).
    let compact: String = trimmed.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() >= 4
        && compact.len() <= 64
        && compact
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !compact.contains(':')
    {
        return ImportCodeKind::ThunderstoreProfile;
    }
    ImportCodeKind::Unknown
}

fn empty_opt(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub fn game_hint_from_managed(game: &ManagedGame) -> ShareGameHint {
    ShareGameHint {
        id: Some(game.id.clone()),
        plugin_id: Some(game.plugin_id.clone()),
        nexus_domain: empty_opt(&game.nexus_domain),
        thunderstore_community: game
            .thunderstore_community
            .as_ref()
            .and_then(|s| empty_opt(s)),
        modio_game_id: game.modio_game_id.filter(|id| *id > 0),
    }
}

pub fn catalogs_overlap(hint: &ShareGameHint, game: &ManagedGame) -> Result<(), String> {
    let mut any_share_catalog = false;
    let mut any_match = false;

    if let Some(d) = hint.nexus_domain.as_deref().filter(|s| !s.is_empty()) {
        any_share_catalog = true;
        let local = game.nexus_domain.trim();
        if !local.is_empty() {
            if !local.eq_ignore_ascii_case(d) {
                return Err(format!(
                    "Nexus domain mismatch: share is '{d}', game is '{local}'"
                ));
            }
            any_match = true;
        }
    }

    if let Some(c) = hint
        .thunderstore_community
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        any_share_catalog = true;
        if let Some(local) = game
            .thunderstore_community
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if !crate::thunderstore::community_slug_matches(c, local) {
                return Err(format!(
                    "Thunderstore community mismatch: share is '{c}', game is '{local}'"
                ));
            }
            any_match = true;
        }
    }

    if let Some(gid) = hint.modio_game_id.filter(|id| *id > 0) {
        any_share_catalog = true;
        if let Some(local) = game.modio_game_id.filter(|id| *id > 0) {
            if local != gid {
                return Err(format!(
                    "mod.io game id mismatch: share is {gid}, game is {local}"
                ));
            }
            any_match = true;
        }
    }

    if !any_share_catalog {
        return Err("Share code has no catalog identity (Nexus / Thunderstore / mod.io).".into());
    }
    if !any_match {
        return Err(
            "This share does not match any catalog configured on the selected game (Nexus domain, Thunderstore community, or mod.io id)."
                .into(),
        );
    }
    Ok(())
}

pub fn plugin_mismatch_warning(hint: &ShareGameHint, game: &ManagedGame) -> Option<String> {
    let share_plugin = hint.plugin_id.as_deref()?.trim();
    if share_plugin.is_empty() {
        return None;
    }
    if share_plugin != game.plugin_id {
        Some(format!(
            "Plugin differs: share is '{share_plugin}', game is '{}'. Deploy layout may differ.",
            game.plugin_id
        ))
    } else {
        None
    }
}

fn display_name(m: &StagedMod) -> String {
    if m.name.trim().is_empty() {
        m.id.clone()
    } else {
        m.name.clone()
    }
}

/// Convert enabled load-order mods into portable share entries.
pub fn build_manifest_from_loadorder(
    order: &LoadOrder,
    game: &ManagedGame,
    name: Option<String>,
) -> (ShareManifest, Vec<String>) {
    let mut mods = Vec::new();
    let mut skipped = Vec::new();

    let mut enabled: Vec<&StagedMod> = order.mods.iter().filter(|m| m.enabled).collect();
    enabled.sort_by_key(|m| m.order);

    for m in enabled {
        if m.id.starts_with("overlay_") {
            skipped.push(format!("{} (config overlay)", display_name(m)));
            continue;
        }
        match m.source {
            ModSource::Nexus => {
                if m.nexus_mod_id == 0 || m.nexus_file_id == 0 {
                    skipped.push(format!("{} (manual/imported Nexus archive)", display_name(m)));
                    continue;
                }
                mods.push(ShareModEntry {
                    s: ShareModSource::Nexus,
                    domain: Some(m.domain.clone()),
                    mod_id: Some(m.nexus_mod_id),
                    file_id: Some(m.nexus_file_id),
                    community: None,
                    namespace: None,
                    name: None,
                    game_id: None,
                    display_name: Some(display_name(m)),
                    version: m.version.clone(),
                });
            }
            ModSource::Thunderstore => {
                let (Some(ns), Some(pkg)) = (m.ts_namespace.as_deref(), m.ts_name.as_deref())
                else {
                    skipped.push(format!("{} (incomplete Thunderstore ids)", display_name(m)));
                    continue;
                };
                mods.push(ShareModEntry {
                    s: ShareModSource::Thunderstore,
                    domain: None,
                    mod_id: None,
                    file_id: None,
                    community: Some(m.domain.clone()),
                    namespace: Some(ns.to_string()),
                    name: Some(pkg.to_string()),
                    game_id: None,
                    display_name: Some(display_name(m)),
                    version: m.version.clone(),
                });
            }
            ModSource::Modio => {
                let (Some(gid), Some(mid)) = (m.modio_game_id, m.modio_mod_id) else {
                    skipped.push(format!("{} (incomplete mod.io ids)", display_name(m)));
                    continue;
                };
                mods.push(ShareModEntry {
                    s: ShareModSource::Modio,
                    domain: None,
                    mod_id: Some(mid),
                    file_id: m.modio_file_id,
                    community: None,
                    namespace: None,
                    name: None,
                    game_id: Some(gid),
                    display_name: Some(display_name(m)),
                    version: m.version.clone(),
                });
            }
        }
    }

    let manifest = ShareManifest {
        v: 1,
        name,
        game: game_hint_from_managed(game),
        mods,
    };
    (manifest, skipped)
}

pub fn entry_from_code(
    code: &str,
    name_override: Option<String>,
    source_game_id: Option<String>,
) -> Result<SavedCollectionEntry> {
    let manifest = decode_share_code(code)?;
    let preview = preview_from_manifest(&manifest, code.len());
    let name = name_override
        .or(manifest.name.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Shared loadout".into());
    let now = now_rfc3339();
    Ok(SavedCollectionEntry {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        code: if code.trim().starts_with("#emperor1") {
            code.trim().to_string()
        } else {
            encode_share_manifest(&manifest)?
        },
        created_at: now.clone(),
        updated_at: now,
        source_game_id,
        game: manifest.game,
        mod_count: preview.mods.len(),
        nexus_count: preview.nexus_count,
        thunderstore_count: preview.thunderstore_count,
        modio_count: preview.modio_count,
    })
}

pub fn load_saved_collections(paths: &Paths) -> Result<SavedCollectionsStore> {
    let file = paths.saved_collections_file();
    if !file.exists() {
        return Ok(SavedCollectionsStore::default());
    }
    let raw = std::fs::read_to_string(&file)
        .with_context(|| format!("reading {}", file.display()))?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save_saved_collections(paths: &Paths, store: &SavedCollectionsStore) -> Result<()> {
    let file = paths.saved_collections_file();
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(store)?;
    std::fs::write(&file, raw)?;
    Ok(())
}

pub fn add_saved_collection(
    paths: &Paths,
    entry: SavedCollectionEntry,
) -> Result<SavedCollectionEntry> {
    let mut store = load_saved_collections(paths)?;
    store.collections.insert(0, entry.clone());
    save_saved_collections(paths, &store)?;
    Ok(entry)
}

pub fn rename_saved_collection(paths: &Paths, id: &str, name: &str) -> Result<SavedCollectionEntry> {
    let mut store = load_saved_collections(paths)?;
    let entry = store
        .collections
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or_else(|| anyhow!("saved collection not found"))?;
    entry.name = name.trim().to_string();
    entry.updated_at = now_rfc3339();
    let cloned = entry.clone();
    save_saved_collections(paths, &store)?;
    Ok(cloned)
}

pub fn delete_saved_collection(paths: &Paths, id: &str) -> Result<()> {
    let mut store = load_saved_collections(paths)?;
    let before = store.collections.len();
    store.collections.retain(|c| c.id != id);
    if store.collections.len() == before {
        bail!("saved collection not found");
    }
    save_saved_collections(paths, &store)
}

pub fn get_saved_collection(paths: &Paths, id: &str) -> Result<SavedCollectionDetail> {
    let store = load_saved_collections(paths)?;
    let entry = store
        .collections
        .into_iter()
        .find(|c| c.id == id)
        .ok_or_else(|| anyhow!("saved collection not found"))?;
    let manifest = decode_share_code(&entry.code)?;
    Ok(SavedCollectionDetail {
        entry,
        mods: manifest.mods,
    })
}

pub fn mod_label(entry: &ShareModEntry) -> String {
    entry
        .display_name
        .clone()
        .or_else(|| entry.name.clone())
        .unwrap_or_else(|| match entry.s {
            ShareModSource::Nexus => format!(
                "Nexus {}:{}",
                entry.mod_id.unwrap_or(0),
                entry.file_id.unwrap_or(0)
            ),
            ShareModSource::Thunderstore => format!(
                "{}-{}",
                entry.namespace.as_deref().unwrap_or("?"),
                entry.name.as_deref().unwrap_or("?")
            ),
            ShareModSource::Modio => format!("mod.io {}", entry.mod_id.unwrap_or(0)),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::StagedMod;

    fn sample_manifest() -> ShareManifest {
        ShareManifest {
            v: 1,
            name: Some("Test pack".into()),
            game: ShareGameHint {
                id: Some("game1".into()),
                plugin_id: Some("unity_bepinex".into()),
                nexus_domain: Some("skyrimspecialedition".into()),
                thunderstore_community: Some("lethal-company".into()),
                modio_game_id: Some(42),
            },
            mods: vec![
                ShareModEntry {
                    s: ShareModSource::Nexus,
                    domain: Some("skyrimspecialedition".into()),
                    mod_id: Some(10),
                    file_id: Some(20),
                    community: None,
                    namespace: None,
                    name: None,
                    game_id: None,
                    display_name: Some("Cool Mod".into()),
                    version: Some("1.0".into()),
                },
                ShareModEntry {
                    s: ShareModSource::Thunderstore,
                    domain: None,
                    mod_id: None,
                    file_id: None,
                    community: Some("lethal-company".into()),
                    namespace: Some("Owner".into()),
                    name: Some("Pkg".into()),
                    game_id: None,
                    display_name: Some("Pkg".into()),
                    version: Some("2.0.0".into()),
                },
                ShareModEntry {
                    s: ShareModSource::Modio,
                    domain: None,
                    mod_id: Some(7),
                    file_id: Some(8),
                    community: None,
                    namespace: None,
                    name: None,
                    game_id: Some(42),
                    display_name: Some("Mio".into()),
                    version: None,
                },
            ],
        }
    }

    #[test]
    fn round_trip_encode_decode() {
        let m = sample_manifest();
        let code = encode_share_manifest(&m).unwrap();
        assert!(code.starts_with(SHARE_PREFIX));
        let back = decode_share_code(&code).unwrap();
        assert_eq!(back.v, 1);
        assert_eq!(back.name.as_deref(), Some("Test pack"));
        assert_eq!(back.mods.len(), 3);
        assert_eq!(back.mods[0].mod_id, Some(10));
        assert_eq!(back.mods[1].namespace.as_deref(), Some("Owner"));
        assert_eq!(back.mods[2].game_id, Some(42));
    }

    #[test]
    fn detect_kinds() {
        let code = encode_share_manifest(&sample_manifest()).unwrap();
        assert_eq!(detect_code_kind(&code), ImportCodeKind::Emperor);
        assert_eq!(
            detect_code_kind("AbCd1234"),
            ImportCodeKind::ThunderstoreProfile
        );
        assert_eq!(
            detect_code_kind("#r2modman\nAAAA"),
            ImportCodeKind::ThunderstoreProfile
        );
        assert_eq!(detect_code_kind("not a code!!!"), ImportCodeKind::Unknown);
        assert_eq!(detect_code_kind(""), ImportCodeKind::Unknown);
    }

    #[test]
    fn skips_non_portable() {
        let game = ManagedGame {
            id: "g".into(),
            title: "G".into(),
            nexus_domain: "domain".into(),
            install_path: "/tmp".into(),
            launcher: "steam".into(),
            plugin_id: "p".into(),
            cover_path: None,
            project_name: None,
            thunderstore_community: None,
            modio_game_id: None,
        };
        let order = LoadOrder {
            mods: vec![
                StagedMod {
                    id: "1_2".into(),
                    name: "Good".into(),
                    source: ModSource::Nexus,
                    nexus_mod_id: 1,
                    nexus_file_id: 2,
                    enabled: true,
                    order: 0,
                    domain: "domain".into(),
                    ..Default::default()
                },
                StagedMod {
                    id: "0_9".into(),
                    name: "Manual".into(),
                    source: ModSource::Nexus,
                    nexus_mod_id: 0,
                    nexus_file_id: 9,
                    enabled: true,
                    order: 1,
                    domain: "domain".into(),
                    ..Default::default()
                },
                StagedMod {
                    id: "disabled".into(),
                    name: "Off".into(),
                    source: ModSource::Nexus,
                    nexus_mod_id: 3,
                    nexus_file_id: 4,
                    enabled: false,
                    order: 2,
                    domain: "domain".into(),
                    ..Default::default()
                },
                StagedMod {
                    id: "overlay_x".into(),
                    name: "Overlay".into(),
                    source: ModSource::Thunderstore,
                    enabled: true,
                    order: 3,
                    ..Default::default()
                },
            ],
        };
        let (manifest, skipped) = build_manifest_from_loadorder(&order, &game, None);
        assert_eq!(manifest.mods.len(), 1);
        assert_eq!(manifest.mods[0].mod_id, Some(1));
        assert_eq!(skipped.len(), 2);
    }

    #[test]
    fn catalogs_overlap_ok() {
        let game = ManagedGame {
            id: "g".into(),
            title: "G".into(),
            nexus_domain: "skyrimspecialedition".into(),
            install_path: "/tmp".into(),
            launcher: "steam".into(),
            plugin_id: "p".into(),
            cover_path: None,
            project_name: None,
            thunderstore_community: Some("lethal-company".into()),
            modio_game_id: Some(42),
        };
        let hint = sample_manifest().game;
        assert!(catalogs_overlap(&hint, &game).is_ok());
    }

    #[test]
    fn catalogs_overlap_fail() {
        let game = ManagedGame {
            id: "g".into(),
            title: "G".into(),
            nexus_domain: "fallout4".into(),
            install_path: "/tmp".into(),
            launcher: "steam".into(),
            plugin_id: "p".into(),
            cover_path: None,
            project_name: None,
            thunderstore_community: None,
            modio_game_id: None,
        };
        let hint = ShareGameHint {
            id: None,
            plugin_id: None,
            nexus_domain: Some("skyrimspecialedition".into()),
            thunderstore_community: None,
            modio_game_id: None,
        };
        assert!(catalogs_overlap(&hint, &game).is_err());
    }
}
