//! Tauri IPC commands.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use walkdir::WalkDir;

use crate::{
    assist::{AssistBounds, AssistContext},
    config::{self, AppConfig, ManagedGame, Paths, APP_NAME, APP_VERSION},
    detection::{self, DetectedGame},
    games::{self, GamePlugin},
    migration,
    modio_api::{self, ModioClient, ModioFileInfo, ModioModDetail},
    mods::{
        self,
        options::{self, ModOptionSelection, ModOptionSet},
        CollectionKind, CollectionSource, InstalledCollection, ModSource, StagedMod,
        StagedModUpdate,
    },
    nexus::{
        self, stream_url_to_file, CollectionDetail, CollectionModFile, GameInfo, ModDetail,
        ModFileInfo, ModGalleryImage, NexusClient, NexusUser, TransferControl, CANCELLED_MSG,
        PAUSED_MSG,
    },
    share::{
        self, DetectImportResult, ExportShareResult, ImportCodeKind, SavedCollectionDetail,
        SavedCollectionEntry, ShareAssistFile, ShareDecodePreview, ShareImportResult,
        ShareModSource,
    },
    thunderstore::{self, ThunderstoreClient, TsDependency, TsPackageDetail},
};

#[derive(Clone)]
pub enum DownloadResumeSource {
    Api {
        game_id: String,
        domain: String,
        mod_id: u64,
        file_id: u64,
        label: String,
        version: Option<String>,
        nxm_key: Option<String>,
        nxm_expires: Option<u64>,
        replace_staged_id: Option<String>,
        file_name: Option<String>,
    },
    Cdn {
        url: String,
        cookie_header: String,
        game_id: String,
        domain: String,
        mod_id: u64,
        file_id: u64,
        label: String,
        version: Option<String>,
        replace_staged_id: Option<String>,
        file_name: Option<String>,
    },
    Thunderstore {
        game_id: String,
        community: String,
        namespace: String,
        name: String,
        version: Option<String>,
        display_name: String,
        enabled: Option<bool>,
    },
    Modio {
        game_id: String,
        modio_game_id: u32,
        mod_id: u64,
        file_id: u64,
        name: String,
        version: Option<String>,
        name_id: String,
    },
}

pub struct DownloadJob {
    pub cancel: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
    pub batch_id: Option<String>,
    pub dest: Option<PathBuf>,
    pub source: Option<DownloadResumeSource>,
}

pub struct AppState {
    pub paths: Paths,
    pub config: Mutex<AppConfig>,
    pub api_key: Mutex<Option<String>>,
    pub modio_api_key: Mutex<Option<String>>,
    pub user: Mutex<Option<NexusUser>>,
    pub downloads: Mutex<Vec<DownloadItem>>,
    pub download_jobs: Mutex<HashMap<String, DownloadJob>>,
    /// Restart metadata kept after failure so Retry can re-run without re-browsing.
    pub download_restart_meta: Mutex<HashMap<String, DownloadResumeSource>>,
    pub assist: Mutex<Option<AssistContext>>,
    /// Monotonic Assist window id (captured in Destroyed handlers).
    pub assist_window_gen: AtomicU64,
    /// Generation that was closed intentionally (success / cancel / replace).
    pub assist_closed_gen: AtomicU64,
    /// Debounce identical nxm emit URLs.
    pub last_nxm_emit: Mutex<Option<(String, Instant)>>,
    /// Debounce identical handle_nxm invocations.
    pub last_nxm_handled: Mutex<Option<(String, Instant)>>,
    /// Debounce identical CDN download intercepts.
    pub last_cdn_handled: Mutex<Option<(String, Instant)>>,
    /// Last embedded assist panel bounds (logical px, relative to main window).
    pub assist_bounds: Mutex<AssistBounds>,
    /// Whether the assist webview should be shown (Downloads tab). Bounds/open must not force-show.
    pub assist_desired_visible: AtomicBool,
    /// Nexus URL waiting until the Linux embed is attached. Flushed even while the overlay is hidden.
    pub assist_pending_url: Mutex<Option<String>>,
    /// Linux overlay stays hidden until the first non-blank assist document finishes loading.
    pub assist_hide_until_loaded: AtomicBool,
    /// Cached Nexus game metadata (categories, counts) keyed by domain.
    pub game_info_cache: Mutex<HashMap<String, GameInfo>>,
    /// Cached Thunderstore community package lists.
    pub ts_package_cache: Mutex<HashMap<String, (Instant, Vec<thunderstore::TsPackage>)>>,
    /// Session cache of Nexus games list for catalog matching.
    pub nexus_games_cache: Mutex<Option<Vec<nexus::NexusGameEntry>>>,
    /// Session cache of Thunderstore communities for catalog matching.
    pub ts_communities_cache: Mutex<Option<Vec<thunderstore::TsCommunity>>>,
    /// Caps simultaneous transfers. Installing a collection queues dozens of
    /// downloads at once, and saturating the connection is what makes CDN
    /// streams stall and end short in the first place.
    pub download_slots: tokio::sync::Semaphore,
    /// Same per-game lock the staging functions take.
    pub order_locks: mods::GameOrderLocks,
}

/// Enough to keep a fast link busy without starving any single transfer.
const MAX_CONCURRENT_DOWNLOADS: usize = 3;

/// Catalog identity for opening a download's mod page. Absent for loader installs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum DownloadPageTarget {
    Nexus {
        game_id: String,
        domain: String,
        mod_id: u64,
    },
    Thunderstore {
        game_id: String,
        community: String,
        namespace: String,
        package_name: String,
    },
    Modio {
        game_id: String,
        modio_game_id: u32,
        modio_mod_id: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadItem {
    pub id: String,
    pub label: String,
    pub status: String,
    pub error: Option<String>,
    pub bytes_downloaded: u64,
    pub bytes_total: Option<u64>,
    pub speed_bps: u64,
    pub batch_id: Option<String>,
    #[serde(default)]
    pub can_restart: bool,
    /// Chosen file name, when it is distinct from the mod title.
    #[serde(default)]
    pub file_name: Option<String>,
    /// Where to open this download's mod page. Missing for loader installs.
    #[serde(default)]
    pub page: Option<DownloadPageTarget>,
}

impl DownloadItem {
    fn new(
        id: String,
        label: String,
        status: &str,
        batch_id: Option<String>,
        page: Option<DownloadPageTarget>,
    ) -> Self {
        Self {
            id,
            label,
            status: status.into(),
            error: None,
            bytes_downloaded: 0,
            bytes_total: None,
            speed_bps: 0,
            batch_id,
            can_restart: false,
            file_name: None,
            page,
        }
    }
}

async fn modio_file_label(
    client: &ModioClient,
    game_id: u32,
    mod_id: u64,
    file_id: u64,
    primary_file_id: Option<u64>,
    mod_name: &str,
) -> Option<String> {
    let files = client
        .list_files(game_id, mod_id, primary_file_id)
        .await
        .ok()?;
    let archive = files
        .iter()
        .find(|f| f.file_id == file_id)
        .map(|f| f.filename.as_str());
    mods::pick_staged_file_name(mod_name, None, archive)
}

fn assign_download_file_name(state: &AppState, id: &str, file_name: Option<&str>) -> bool {
    let Some(name) = file_name.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    if let Ok(mut q) = state.downloads.lock() {
        if let Some(item) = q.iter_mut().find(|d| d.id == id) {
            if item
                .file_name
                .as_deref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                item.file_name = Some(name.to_string());
                return true;
            }
        }
    }
    false
}

impl AppState {
    pub fn new() -> anyhow::Result<Self> {
        let paths = Paths::resolve()?;
        let mut config = config::load_config(&paths)?;
        let recovery = migration::recover_legacy_data(&paths, &mut config)?;
        if !recovery.games_copied.is_empty() || recovery.mods_rewritten > 0 {
            log::info!(
                "Recovered legacy nexus-manager data: {} game(s), {} staging path(s) rewritten",
                recovery.games_copied.len(),
                recovery.mods_rewritten
            );
        }
        for warning in &recovery.warnings {
            log::warn!("{warning}");
        }
        let api_key = load_key_with_fallback(&paths)?;
        let modio_api_key = load_modio_key_with_fallback(&paths)?;
        let mut recent = load_recent_downloads(&paths);
        let recent_limit =
            config::clamp_recent_downloads_limit(config.recent_downloads_limit) as usize;
        let recent_before = recent.len();
        trim_finished_downloads(&mut recent, recent_limit);
        if recent.len() != recent_before {
            save_recent_downloads(&paths, &recent);
        }
        Ok(Self {
            paths,
            config: Mutex::new(config),
            api_key: Mutex::new(api_key),
            modio_api_key: Mutex::new(modio_api_key),
            user: Mutex::new(None),
            downloads: Mutex::new(recent),
            download_jobs: Mutex::new(HashMap::new()),
            download_restart_meta: Mutex::new(HashMap::new()),
            assist: Mutex::new(None),
            assist_window_gen: AtomicU64::new(0),
            assist_closed_gen: AtomicU64::new(0),
            last_nxm_emit: Mutex::new(None),
            last_nxm_handled: Mutex::new(None),
            last_cdn_handled: Mutex::new(None),
            assist_bounds: Mutex::new(AssistBounds::default()),
            assist_desired_visible: AtomicBool::new(false),
            assist_pending_url: Mutex::new(None),
            assist_hide_until_loaded: AtomicBool::new(false),
            game_info_cache: Mutex::new(HashMap::new()),
            ts_package_cache: Mutex::new(HashMap::new()),
            nexus_games_cache: Mutex::new(None),
            ts_communities_cache: Mutex::new(None),
            download_slots: tokio::sync::Semaphore::new(MAX_CONCURRENT_DOWNLOADS),
            order_locks: mods::shared_order_locks(),
        })
    }

    pub async fn get_game_cached(&self, domain: &str) -> Result<GameInfo, String> {
        if let Some(cached) = self
            .game_info_cache
            .lock()
            .map_err(|e| e.to_string())?
            .get(domain)
            .cloned()
        {
            return Ok(cached);
        }
        let client = self.client()?;
        let info = client.get_game(domain).await.map_err(|e| e.to_string())?;
        self.game_info_cache
            .lock()
            .map_err(|e| e.to_string())?
            .insert(domain.to_string(), info.clone());
        Ok(info)
    }

    pub fn client(&self) -> Result<NexusClient, String> {
        let key = self
            .api_key
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .ok_or_else(|| "No API key set. Add one in Setup.".to_string())?;
        NexusClient::new(key).map_err(|e| e.to_string())
    }

    pub fn is_premium(&self) -> bool {
        self.user
            .lock()
            .ok()
            .and_then(|u| u.clone())
            .map(|u| u.is_premium)
            .unwrap_or(false)
    }

    pub async fn ensure_premium_status(&self) -> bool {
        if self.is_premium() {
            return true;
        }
        let Ok(client) = self.client() else {
            return false;
        };
        match client.validate().await {
            Ok(u) => {
                let premium = u.is_premium;
                if let Ok(mut lock) = self.user.lock() {
                    *lock = Some(u);
                }
                premium
            }
            Err(_) => false,
        }
    }
}

fn load_key_with_fallback(paths: &Paths) -> anyhow::Result<Option<String>> {
    if let Ok(Some(k)) = nexus::load_api_key() {
        return Ok(Some(k));
    }
    let file = paths.config_dir.join("nexus_api_key");
    nexus::load_api_key_file(&file)
}

fn load_modio_key_with_fallback(paths: &Paths) -> anyhow::Result<Option<String>> {
    if let Ok(Some(k)) = modio_api::load_api_key() {
        return Ok(Some(k));
    }
    let file = paths.config_dir.join("modio_api_key");
    modio_api::load_api_key_file(&file)
}

fn is_active_download_status(status: &str) -> bool {
    status == "downloading" || status == "extracting" || status == "paused"
}

/// Terminal states that belong in the Recent list and are safe to forget.
fn is_finished_download_status(status: &str) -> bool {
    matches!(status, "done" | "staged" | "failed" | "cancelled")
}

fn nexus_page(game_id: &str, domain: &str, mod_id: u64) -> Option<DownloadPageTarget> {
    if game_id.is_empty() || domain.is_empty() || mod_id == 0 {
        return None;
    }
    Some(DownloadPageTarget::Nexus {
        game_id: game_id.to_string(),
        domain: domain.to_string(),
        mod_id,
    })
}

fn thunderstore_page(
    game_id: &str,
    community: &str,
    namespace: &str,
    package_name: &str,
) -> Option<DownloadPageTarget> {
    if game_id.is_empty() || community.is_empty() || namespace.is_empty() || package_name.is_empty()
    {
        return None;
    }
    Some(DownloadPageTarget::Thunderstore {
        game_id: game_id.to_string(),
        community: community.to_string(),
        namespace: namespace.to_string(),
        package_name: package_name.to_string(),
    })
}

fn modio_page(game_id: &str, modio_game_id: u32, modio_mod_id: u64) -> Option<DownloadPageTarget> {
    if game_id.is_empty() || modio_game_id == 0 || modio_mod_id == 0 {
        return None;
    }
    Some(DownloadPageTarget::Modio {
        game_id: game_id.to_string(),
        modio_game_id,
        modio_mod_id,
    })
}

/// Keep every in-progress row, plus the newest `limit` finished rows.
/// The vec is newest-first.
fn trim_finished_downloads(downloads: &mut Vec<DownloadItem>, limit: usize) {
    let mut kept_finished = 0usize;
    downloads.retain(|d| {
        if !is_finished_download_status(&d.status) {
            return true;
        }
        if kept_finished < limit {
            kept_finished += 1;
            true
        } else {
            false
        }
    });
}

fn load_recent_downloads(paths: &Paths) -> Vec<DownloadItem> {
    let path = paths.recent_downloads_file();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(mut items) = serde_json::from_str::<Vec<DownloadItem>>(&raw) else {
        log::warn!(
            "ignoring unreadable recent downloads file {}",
            path.display()
        );
        return Vec::new();
    };
    items.retain(|d| is_finished_download_status(&d.status));
    for item in &mut items {
        item.can_restart = false;
        item.speed_bps = 0;
    }
    items
}

fn save_recent_downloads(paths: &Paths, downloads: &[DownloadItem]) {
    let finished: Vec<DownloadItem> = downloads
        .iter()
        .filter(|d| is_finished_download_status(&d.status))
        .cloned()
        .map(|mut d| {
            d.can_restart = false;
            d.speed_bps = 0;
            d
        })
        .collect();
    let path = paths.recent_downloads_file();
    match serde_json::to_string_pretty(&finished) {
        Ok(raw) => {
            if let Err(e) = std::fs::write(&path, raw) {
                log::warn!("failed to save recent downloads: {e}");
            }
        }
        Err(e) => log::warn!("failed to serialize recent downloads: {e}"),
    }
}

fn recent_downloads_limit(state: &AppState) -> usize {
    state
        .config
        .lock()
        .ok()
        .map(|cfg| config::clamp_recent_downloads_limit(cfg.recent_downloads_limit) as usize)
        .unwrap_or(50)
}

fn commit_recent_downloads(state: &AppState) {
    let limit = recent_downloads_limit(state);
    let snapshot = {
        let Ok(mut q) = state.downloads.lock() else {
            return;
        };
        trim_finished_downloads(&mut q, limit);
        q.clone()
    };
    save_recent_downloads(&state.paths, &snapshot);
}

fn push_download(state: &AppState, item: DownloadItem) {
    if let Ok(mut q) = state.downloads.lock() {
        q.insert(0, item);
    }
    commit_recent_downloads(state);
}

fn update_download(state: &AppState, id: &str, status: &str, error: Option<String>) {
    if let Ok(mut q) = state.downloads.lock() {
        if let Some(item) = q.iter_mut().find(|d| d.id == id) {
            item.status = status.to_string();
            item.error = error;
            if status == "cancelled"
                || status == "staged"
                || status == "failed"
                || status == "paused"
            {
                item.speed_bps = 0;
            }
        }
    }
    commit_recent_downloads(state);
}

fn ensure_download_job(
    state: &AppState,
    id: &str,
    batch_id: Option<String>,
) -> (Arc<AtomicBool>, Arc<AtomicBool>) {
    if let Ok(mut jobs) = state.download_jobs.lock() {
        if let Some(job) = jobs.get(id) {
            return (job.cancel.clone(), job.pause.clone());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        jobs.insert(
            id.to_string(),
            DownloadJob {
                cancel: cancel.clone(),
                pause: pause.clone(),
                batch_id,
                dest: None,
                source: None,
            },
        );
        return (cancel, pause);
    }
    (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    )
}

fn set_download_resume(state: &AppState, id: &str, dest: PathBuf, source: DownloadResumeSource) {
    store_restart_meta(state, id, source.clone());
    if let Ok(mut jobs) = state.download_jobs.lock() {
        if let Some(job) = jobs.get_mut(id) {
            job.dest = Some(dest);
            job.source = Some(source);
        }
    }
}

fn store_restart_meta(state: &AppState, id: &str, source: DownloadResumeSource) {
    if let Ok(mut m) = state.download_restart_meta.lock() {
        m.insert(id.to_string(), source);
    }
    set_download_can_restart(state, id, true);
}

fn clear_restart_meta(state: &AppState, id: &str) {
    if let Ok(mut m) = state.download_restart_meta.lock() {
        m.remove(id);
    }
    set_download_can_restart(state, id, false);
}

fn set_download_can_restart(state: &AppState, id: &str, can: bool) {
    if let Ok(mut q) = state.downloads.lock() {
        if let Some(item) = q.iter_mut().find(|d| d.id == id) {
            item.can_restart = can;
        }
    }
}

fn reset_download_row(state: &AppState, id: &str) {
    if let Ok(mut q) = state.downloads.lock() {
        if let Some(item) = q.iter_mut().find(|d| d.id == id) {
            item.status = "downloading".into();
            item.error = None;
            item.bytes_downloaded = 0;
            item.bytes_total = None;
            item.speed_bps = 0;
        }
    }
    commit_recent_downloads(state);
}

fn finish_download_success(state: &AppState, id: &str) {
    clear_restart_meta(state, id);
    clear_download_job(state, id);
}

fn download_label(state: &AppState, id: &str) -> String {
    state
        .downloads
        .lock()
        .ok()
        .and_then(|q| q.iter().find(|d| d.id == id).map(|d| d.label.clone()))
        .unwrap_or_else(|| "Download".to_string())
}

fn clear_download_job(state: &AppState, id: &str) {
    if let Ok(mut jobs) = state.download_jobs.lock() {
        jobs.remove(id);
    }
}

/// True when `path` resolves under `downloads_dir` (symlinks / `..` normalized).
fn path_is_under_downloads(path: &Path, downloads_dir: &Path) -> bool {
    let downloads =
        std::fs::canonicalize(downloads_dir).unwrap_or_else(|_| downloads_dir.to_path_buf());
    let candidate = match std::fs::canonicalize(path) {
        Ok(p) => p,
        Err(_) => {
            if !path.is_absolute() {
                return false;
            }
            path.to_path_buf()
        }
    };
    candidate.starts_with(&downloads)
}

/// Delete a finished/failed download archive when it lives under the cache downloads dir.
/// No-ops for paths outside that dir (e.g. user-chosen import files).
fn remove_download_archive(path: &Path, downloads_dir: &Path) {
    if !path_is_under_downloads(path, downloads_dir) {
        return;
    }
    if path.is_dir() {
        let _ = std::fs::remove_dir_all(path);
    } else {
        let _ = std::fs::remove_file(path);
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ClearDownloadCacheResult {
    pub files_removed: u64,
    pub bytes_freed: u64,
}

/// Wipe entries under `downloads_dir` except paths in `protected` (active job dests).
fn clear_download_cache_dir(
    downloads_dir: &Path,
    protected: &HashSet<PathBuf>,
) -> Result<ClearDownloadCacheResult, String> {
    let mut files_removed = 0u64;
    let mut bytes_freed = 0u64;
    if !downloads_dir.is_dir() {
        return Ok(ClearDownloadCacheResult {
            files_removed,
            bytes_freed,
        });
    }
    let entries = std::fs::read_dir(downloads_dir).map_err(|e| e.to_string())?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let canon = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if protected.iter().any(|p| {
            let pc = std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
            pc == canon || canon.starts_with(&pc) || pc.starts_with(&canon)
        }) {
            continue;
        }
        let size = entry
            .metadata()
            .map(|m| {
                if m.is_dir() {
                    dir_size_approx(&path)
                } else {
                    m.len()
                }
            })
            .unwrap_or(0);
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(&path).is_ok()
        } else {
            std::fs::remove_file(&path).is_ok()
        };
        if removed {
            files_removed += 1;
            bytes_freed += size;
        }
    }
    Ok(ClearDownloadCacheResult {
        files_removed,
        bytes_freed,
    })
}

fn dir_size_approx(path: &Path) -> u64 {
    let mut total = 0u64;
    let Ok(rd) = std::fs::read_dir(path) else {
        return 0;
    };
    for entry in rd.flatten() {
        let meta = entry.metadata();
        if let Ok(m) = meta {
            if m.is_dir() {
                total += dir_size_approx(&entry.path());
            } else {
                total += m.len();
            }
        }
    }
    total
}

fn mark_cancelled(state: &AppState, id: &str) {
    update_download(state, id, "cancelled", Some("Cancelled".into()));
}

fn mark_paused(app: Option<&tauri::AppHandle>, state: &AppState, id: &str) {
    let (bytes_downloaded, bytes_total) = if let Ok(mut q) = state.downloads.lock() {
        if let Some(item) = q.iter_mut().find(|d| d.id == id) {
            if item.status == "cancelled" {
                return;
            }
            item.status = "paused".into();
            item.error = None;
            item.speed_bps = 0;
            (item.bytes_downloaded, item.bytes_total)
        } else {
            return;
        }
    } else {
        return;
    };
    if let Some(handle) = app {
        let _ = handle.emit(
            "download-progress",
            serde_json::json!({
                "id": id,
                "bytes_downloaded": bytes_downloaded,
                "bytes_total": bytes_total,
                "speed_bps": 0,
                "status": "paused",
            }),
        );
    }
}

fn report_download_progress(
    app: &tauri::AppHandle,
    state: &AppState,
    id: &str,
    bytes_downloaded: u64,
    bytes_total: Option<u64>,
    speed_bps: u64,
) {
    let status = if let Ok(mut q) = state.downloads.lock() {
        if let Some(item) = q.iter_mut().find(|d| d.id == id) {
            if item.status == "cancelled" || item.status == "paused" {
                return;
            }
            item.bytes_downloaded = bytes_downloaded;
            item.bytes_total = bytes_total;
            item.speed_bps = speed_bps;
            item.status.clone()
        } else {
            return;
        }
    } else {
        return;
    };
    let _ = app.emit(
        "download-progress",
        serde_json::json!({
            "id": id,
            "bytes_downloaded": bytes_downloaded,
            "bytes_total": bytes_total,
            "speed_bps": speed_bps,
            "status": status,
        }),
    );
}

fn is_cancel_err(msg: &str) -> bool {
    msg.contains(CANCELLED_MSG) || msg == "Cancelled"
}

fn is_pause_err(msg: &str) -> bool {
    msg.contains(PAUSED_MSG) || msg == "Paused"
}

fn request_cancel_ids(state: &AppState, ids: &[String], app: Option<&tauri::AppHandle>) {
    let paused_ids: Vec<String> = {
        let downloads = state.downloads.lock().ok();
        ids.iter()
            .filter(|id| {
                downloads
                    .as_ref()
                    .and_then(|q| q.iter().find(|d| d.id == **id))
                    .map(|d| d.status == "paused")
                    .unwrap_or(false)
            })
            .cloned()
            .collect()
    };

    if let Ok(mut jobs) = state.download_jobs.lock() {
        for id in ids {
            if let Some(job) = jobs.get_mut(id) {
                job.cancel.store(true, Ordering::SeqCst);
                job.pause.store(false, Ordering::SeqCst);
                if let Some(dest) = &job.dest {
                    let _ = std::fs::remove_file(dest);
                }
            }
            mark_cancelled(state, id);
            if paused_ids.iter().any(|p| p == id) {
                jobs.remove(id);
            }
            if let Some(handle) = app {
                let label = download_label(state, id);
                let _ = handle.emit(
                    "download-cancelled",
                    serde_json::json!({
                        "id": id,
                        "label": label,
                    }),
                );
            }
        }
    } else {
        for id in ids {
            mark_cancelled(state, id);
            if let Some(handle) = app {
                let label = download_label(state, id);
                let _ = handle.emit(
                    "download-cancelled",
                    serde_json::json!({
                        "id": id,
                        "label": label,
                    }),
                );
            }
        }
    }
}

fn progress_callback(
    app: Option<&tauri::AppHandle>,
    dl_id: String,
) -> Option<Arc<dyn Fn(u64, Option<u64>, u64) + Send + Sync>> {
    let handle = app?.clone();
    Some(Arc::new(move |downloaded, total, speed| {
        let st = handle.state::<AppState>();
        report_download_progress(&handle, &*st, &dl_id, downloaded, total, speed);
    }))
}

fn file_offset(path: &PathBuf) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// The extension worth carrying over from an upload's real name. Anything
/// containing a path separator or no extension at all is ignored, and the
/// result is lowercased so `is_loose_mod_file` matches.
fn archive_extension(file_name: Option<&str>) -> Option<String> {
    let name = file_name?;
    if name.contains('/') || name.contains('\\') {
        return None;
    }
    let ext = Path::new(name).extension()?.to_str()?.to_lowercase();
    if ext.is_empty() || ext.len() > 12 || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(ext)
}

#[tauri::command]
pub fn get_app_info() -> serde_json::Value {
    serde_json::json!({
        "name": APP_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "platform": std::env::consts::OS,
        "linux_only": false,
    })
}

#[tauri::command]
pub fn scan_games(app: AppHandle) -> Result<Vec<DetectedGame>, String> {
    let games = detection::scan_games();
    // Steam covers typically live under Program Files, outside the default $HOME
    // assetProtocol scope. Allow only the exact cover files we resolved.
    let scope = app.asset_protocol_scope();
    for game in &games {
        if let Some(cover) = game.cover_path.as_deref() {
            if let Err(e) = scope.allow_file(Path::new(cover)) {
                log::debug!("asset scope allow_file({cover}): {e}");
            }
        }
    }
    Ok(games)
}

#[tauri::command]
pub fn list_plugins() -> Vec<games::GamePluginInfo> {
    games::list_plugins()
}

#[tauri::command]
pub fn list_managed(state: State<'_, AppState>) -> Result<Vec<ManagedGame>, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    Ok(cfg.managed_games.clone())
}

#[tauri::command]
pub fn manage_game(
    state: State<'_, AppState>,
    id: String,
    title: String,
    nexus_domain: String,
    install_path: String,
    launcher: String,
    plugin_id: String,
    cover_path: Option<String>,
    project_name: Option<String>,
    thunderstore_community: Option<String>,
    modio_game_id: Option<u32>,
) -> Result<ManagedGame, String> {
    if install_path.is_empty() {
        return Err("Install path is required".into());
    }
    if games::plugin_by_id(&plugin_id).is_none() {
        return Err(format!("Unknown plugin: {plugin_id}"));
    }
    let domain = nexus_domain.trim().to_string();
    let ts_community = thunderstore_community
        .and_then(|s| {
            let t = s.trim().to_string();
            if t.is_empty() {
                None
            } else {
                Some(t)
            }
        })
        .or_else(|| games::thunderstore_community_for_plugin(&plugin_id).map(|s| s.to_string()));
    let modio_id = modio_game_id
        .filter(|&id| id > 0)
        .or_else(|| modio_api::seed_modio_game_id(&plugin_id));
    if domain.is_empty() && ts_community.is_none() && modio_id.is_none() {
        return Err(
            "Enter a Nexus Mods domain, Thunderstore community, and/or mod.io game ID.".into(),
        );
    }
    if plugin_id == "unreal" {
        let preferred = project_name.as_deref().filter(|s| !s.is_empty());
        if games::detect_ue_layout(std::path::Path::new(&install_path)).is_none()
            && preferred.is_none()
        {
            return Err(
                "Could not detect an Unreal project folder. Enter the project name (e.g. Pal, Phoenix)."
                    .into(),
            );
        }
    }
    if plugin_id == "bepinex"
        && !games::looks_like_unity_install(std::path::Path::new(&install_path))
    {
        return Err(
            "Install does not look like a Unity / BepInEx game (missing UnityPlayer, *_Data, or BepInEx)."
                .into(),
        );
    }
    config::ensure_game_dirs(&state.paths, &id).map_err(|e| e.to_string())?;
    let project_name = project_name.and_then(|s| {
        let t = s.trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    });
    let managed = ManagedGame {
        id: id.clone(),
        title: title.clone(),
        nexus_domain: domain,
        install_path: install_path.clone(),
        launcher: launcher.clone(),
        plugin_id,
        cover_path,
        project_name,
        thunderstore_community: ts_community,
        modio_game_id: modio_id,
        tool_overrides: None,
        game_version: None,
        ignored_requirements: Vec::new(),
    };
    let install_exists = Path::new(&install_path).is_dir();
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    if install_exists {
        cfg.managed_games.retain(|g| {
            g.id == id
                || !detection::reconcile::games_match(
                    &g.title,
                    &g.launcher,
                    &managed.title,
                    &managed.launcher,
                )
        });
    } else {
        cfg.managed_games.retain(|g| g.id != id);
    }
    cfg.managed_games.push(managed.clone());
    cfg.last_active_game_id = Some(id);
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())?;
    Ok(managed)
}

#[tauri::command]
pub fn update_managed_game(
    state: State<'_, AppState>,
    id: String,
    nexus_domain: Option<String>,
    project_name: Option<String>,
    thunderstore_community: Option<String>,
    modio_game_id: Option<u32>,
) -> Result<ManagedGame, String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    let game = cfg
        .managed_games
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or_else(|| "Managed game not found".to_string())?;
    if let Some(domain) = nexus_domain {
        game.nexus_domain = domain.trim().to_string();
    }
    if let Some(project) = project_name {
        let t = project.trim().to_string();
        game.project_name = if t.is_empty() { None } else { Some(t) };
    }
    if let Some(community) = thunderstore_community {
        let t = community.trim().to_string();
        game.thunderstore_community = if t.is_empty() { None } else { Some(t) };
    }
    if let Some(mid) = modio_game_id {
        game.modio_game_id = if mid == 0 { None } else { Some(mid) };
    }
    if game.nexus_domain.is_empty()
        && game.thunderstore_community.is_none()
        && game.modio_game_id.is_none()
    {
        return Err(
            "Game needs a Nexus Mods domain, Thunderstore community, and/or mod.io game ID.".into(),
        );
    }
    let managed = game.clone();
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())?;
    Ok(managed)
}

#[tauri::command]
pub fn set_game_version(
    state: State<'_, AppState>,
    id: String,
    version: Option<String>,
) -> Result<ManagedGame, String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    let game = cfg
        .managed_games
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or_else(|| "Managed game not found".to_string())?;
    let trimmed = version.unwrap_or_default().trim().to_string();
    game.game_version = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    };
    let managed = game.clone();
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())?;
    Ok(managed)
}

#[tauri::command]
pub fn set_ignored_requirements(
    state: State<'_, AppState>,
    id: String,
    ignored: Vec<config::IgnoredRequirement>,
) -> Result<ManagedGame, String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    let game = cfg
        .managed_games
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or_else(|| "Managed game not found".to_string())?;
    let mut seen = HashSet::new();
    game.ignored_requirements = ignored
        .into_iter()
        .filter_map(|mut row| {
            row.key = row.key.trim().to_string();
            row.name = row.name.trim().to_string();
            if row.key.is_empty() || row.name.is_empty() || !seen.insert(row.key.clone()) {
                None
            } else {
                Some(row)
            }
        })
        .collect();
    let managed = game.clone();
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())?;
    Ok(managed)
}

#[tauri::command]
pub fn detect_ue_layout(
    install_path: String,
    project_name: Option<String>,
) -> Result<Option<games::UeLayoutInfo>, String> {
    let preferred = project_name.as_deref().filter(|s| !s.is_empty());
    match games::layout_info(std::path::Path::new(&install_path), preferred) {
        Ok(info) => Ok(Some(info)),
        Err(_) => Ok(None),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogSuggestion {
    pub nexus_domain: Option<String>,
    pub nexus_name: Option<String>,
    pub thunderstore_community: Option<String>,
    pub thunderstore_name: Option<String>,
    pub modio_game_id: Option<u32>,
    pub modio_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CatalogSuggestRequest {
    pub id: String,
    pub title: String,
}

const CATALOG_MATCH_MIN_SCORE: u32 = 50;

fn empty_catalog_suggestion() -> CatalogSuggestion {
    CatalogSuggestion {
        nexus_domain: None,
        nexus_name: None,
        thunderstore_community: None,
        thunderstore_name: None,
        modio_game_id: None,
        modio_name: None,
    }
}

fn match_catalog_lists(
    title: &str,
    nexus_games: &[nexus::NexusGameEntry],
    ts_communities: &[thunderstore::TsCommunity],
) -> CatalogSuggestion {
    let mut suggestion = empty_catalog_suggestion();
    let title = title.trim();
    if title.is_empty() {
        return suggestion;
    }
    if let Some(hit) = best_catalog_match(
        title,
        nexus_games,
        |g| g.name.as_str(),
        |g| g.domain_name.as_str(),
    ) {
        suggestion.nexus_domain = Some(hit.domain_name.clone());
        suggestion.nexus_name = Some(hit.name.clone());
    }
    if let Some(hit) = best_catalog_match(
        title,
        ts_communities,
        |c| c.name.as_str(),
        |c| c.identifier.as_str(),
    ) {
        suggestion.thunderstore_community = Some(hit.identifier.clone());
        suggestion.thunderstore_name = Some(hit.name.clone());
    }
    suggestion
}

async fn load_nexus_games_cached(state: &AppState) -> Result<Vec<nexus::NexusGameEntry>, String> {
    let cached = state
        .nexus_games_cache
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    if let Some(list) = cached {
        return Ok(list);
    }
    let Ok(client) = state.client() else {
        return Ok(Vec::new());
    };
    match client.list_games().await {
        Ok(list) => {
            if let Ok(mut lock) = state.nexus_games_cache.lock() {
                *lock = Some(list.clone());
            }
            Ok(list)
        }
        Err(e) => {
            log::warn!("suggest_catalog_ids: Nexus list_games failed: {e:#}");
            Ok(Vec::new())
        }
    }
}

async fn load_ts_communities_cached(
    state: &AppState,
) -> Result<Vec<thunderstore::TsCommunity>, String> {
    let cached = state
        .ts_communities_cache
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    if let Some(list) = cached {
        return Ok(list);
    }
    match ThunderstoreClient::new() {
        Ok(client) => match client.list_communities().await {
            Ok(list) => {
                if let Ok(mut lock) = state.ts_communities_cache.lock() {
                    *lock = Some(list.clone());
                }
                Ok(list)
            }
            Err(e) => {
                log::warn!("suggest_catalog_ids: Thunderstore communities failed: {e:#}");
                Ok(Vec::new())
            }
        },
        Err(e) => {
            log::warn!("suggest_catalog_ids: Thunderstore client: {e:#}");
            Ok(Vec::new())
        }
    }
}

async fn enrich_modio_suggestion(
    state: &AppState,
    title: &str,
    suggestion: &mut CatalogSuggestion,
) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }
    let modio_key = match state.modio_api_key.lock() {
        Ok(lock) => lock.clone(),
        Err(_) => return,
    };
    let Some(key) = modio_key else {
        return;
    };
    match ModioClient::new(&key) {
        Ok(client) => match client.search_games(title, 25).await {
            Ok(games) => {
                if let Some(hit) =
                    best_catalog_match(title, &games, |g| g.name.as_str(), |g| g.name_id.as_str())
                {
                    suggestion.modio_game_id = Some(hit.id);
                    suggestion.modio_name = Some(hit.name.clone());
                }
            }
            Err(e) => {
                log::warn!("suggest_catalog_ids: mod.io search_games failed: {e:#}");
            }
        },
        Err(e) => {
            log::warn!("suggest_catalog_ids: mod.io client: {e:#}");
        }
    }
}

fn normalize_catalog_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() {
            out.push(lower);
            prev_space = false;
        } else if !prev_space && !out.is_empty() {
            out.push(' ');
            prev_space = true;
        }
    }
    out.trim().to_string()
}

fn slug_compact(normalized: &str) -> String {
    normalized.chars().filter(|c| *c != ' ').collect()
}

fn slug_hyphen(normalized: &str) -> String {
    normalized
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn catalog_match_score(query: &str, candidate_name: &str, candidate_id: &str) -> u32 {
    let q = normalize_catalog_text(query);
    if q.is_empty() {
        return 0;
    }
    let name = normalize_catalog_text(candidate_name);
    let id_norm = normalize_catalog_text(candidate_id);
    let q_compact = slug_compact(&q);
    let q_hyphen = slug_hyphen(&q);
    let name_compact = slug_compact(&name);
    let id_compact = slug_compact(&id_norm);
    let id_hyphen = slug_hyphen(&id_norm);

    if !name.is_empty() && name == q {
        return 100;
    }
    if !id_norm.is_empty() && (id_norm == q || id_compact == q_compact || id_hyphen == q_hyphen) {
        return 95;
    }
    if !name_compact.is_empty() && name_compact == q_compact {
        return 90;
    }
    if (!name.is_empty() && (name.contains(&q) || q.contains(&name)))
        || (!id_compact.is_empty()
            && (id_compact.contains(&q_compact) || q_compact.contains(&id_compact)))
    {
        let shorter = q_compact
            .len()
            .min(name_compact.len().max(id_compact.len()));
        let longer = q_compact
            .len()
            .max(name_compact.len())
            .max(id_compact.len());
        if longer > 0 && shorter * 100 / longer >= 60 {
            return 70;
        }
    }
    let q_tokens: Vec<&str> = q.split_whitespace().filter(|t| t.len() > 1).collect();
    if q_tokens.is_empty() {
        return 0;
    }
    let cand = format!("{name} {id_norm}");
    let hit = q_tokens.iter().filter(|t| cand.contains(*t)).count();
    if hit * 100 / q_tokens.len() >= 70 {
        return 55;
    }
    0
}

fn best_catalog_match<'a, T>(
    title: &str,
    items: &'a [T],
    name_fn: impl Fn(&T) -> &str,
    id_fn: impl Fn(&T) -> &str,
) -> Option<&'a T> {
    let mut best: Option<(&T, u32)> = None;
    for item in items {
        let score = catalog_match_score(title, name_fn(item), id_fn(item));
        if score < CATALOG_MATCH_MIN_SCORE {
            continue;
        }
        if best.map(|(_, s)| score > s).unwrap_or(true) {
            best = Some((item, score));
        }
    }
    best.map(|(item, _)| item)
}

#[tauri::command]
pub async fn suggest_catalog_ids(
    state: State<'_, AppState>,
    title: String,
) -> Result<CatalogSuggestion, String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Ok(empty_catalog_suggestion());
    }
    let nexus_games = load_nexus_games_cached(&state).await?;
    let ts_communities = load_ts_communities_cached(&state).await?;
    let mut suggestion = match_catalog_lists(&title, &nexus_games, &ts_communities);
    enrich_modio_suggestion(&state, &title, &mut suggestion).await;
    Ok(suggestion)
}

#[tauri::command]
pub async fn suggest_catalog_ids_batch(
    state: State<'_, AppState>,
    requests: Vec<CatalogSuggestRequest>,
) -> Result<HashMap<String, CatalogSuggestion>, String> {
    let mut out = HashMap::new();
    if requests.is_empty() {
        return Ok(out);
    }
    let nexus_games = load_nexus_games_cached(&state).await?;
    let ts_communities = load_ts_communities_cached(&state).await?;
    let has_modio = state
        .modio_api_key
        .lock()
        .map_err(|e| e.to_string())?
        .is_some();

    for req in requests {
        let id = req.id.trim().to_string();
        if id.is_empty() {
            continue;
        }
        let mut suggestion = match_catalog_lists(&req.title, &nexus_games, &ts_communities);
        if has_modio {
            enrich_modio_suggestion(&state, &req.title, &mut suggestion).await;
        }
        out.insert(id, suggestion);
    }
    Ok(out)
}

#[tauri::command]
pub fn get_game_health(
    state: State<'_, AppState>,
) -> Result<Vec<detection::reconcile::GameHealth>, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    let detected = detection::scan_games();
    Ok(detection::reconcile::compute_game_health(
        &cfg.managed_games,
        &detected,
    ))
}

#[tauri::command]
pub fn relink_managed_game(
    state: State<'_, AppState>,
    old_id: String,
    new_id: String,
) -> Result<ManagedGame, String> {
    let detected = detection::scan_games();
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    detection::reconcile::relink_managed_game(&state.paths, &mut cfg, &old_id, &new_id, &detected)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn unmanage_game(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.managed_games.retain(|g| g.id != id);
    if cfg.last_active_game_id.as_deref() == Some(&id) {
        cfg.last_active_game_id = cfg.managed_games.first().map(|g| g.id.clone());
    }
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())?;
    migration::remove_game_from_legacy_config(&id).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let has_key = state.api_key.lock().map_err(|e| e.to_string())?.is_some();
    let has_modio_api_key = state
        .modio_api_key
        .lock()
        .map_err(|e| e.to_string())?
        .is_some();
    let user = state.user.lock().map_err(|e| e.to_string())?.clone();
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    let initial_setup_complete = config::resolve_initial_setup(cfg.initial_setup_complete, has_key);
    if cfg.initial_setup_complete.is_none() {
        cfg.initial_setup_complete = Some(initial_setup_complete);
        config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())?;
    }
    Ok(serde_json::json!({
        "adult_content": cfg.adult_content,
        "game_version_warnings": cfg.game_version_warnings,
        "autoclick_free_download": cfg.autoclick_free_download,
        "last_active_game_id": cfg.last_active_game_id,
        "theme": cfg.theme,
        "install_click_behavior": cfg.install_click_behavior,
        "recent_downloads_limit": config::clamp_recent_downloads_limit(cfg.recent_downloads_limit),
        "has_api_key": has_key,
        "has_modio_api_key": has_modio_api_key,
        "initial_setup_complete": initial_setup_complete,
        "nexus_website_signed_in": cfg.nexus_website_signed_in,
        "user": user,
        "config_dir": state.paths.config_dir,
        "data_dir": state.paths.data_dir,
        "cache_dir": state.paths.cache_dir,
        "platform_linux": cfg!(target_os = "linux"),
    }))
}

#[tauri::command]
pub fn complete_initial_setup(state: State<'_, AppState>) -> Result<(), String> {
    let has_key = state.api_key.lock().map_err(|e| e.to_string())?.is_some();
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    if !has_key || !cfg.nexus_website_signed_in {
        return Err("Save a Nexus API key and finish website sign-in before leaving setup.".into());
    }
    cfg.initial_setup_complete = Some(true);
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn scan_mod_orphans(state: State<'_, AppState>) -> Result<migration::OrphanScan, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    Ok(migration::scan_orphans(&state.paths, &cfg))
}

#[tauri::command]
pub fn clear_download_cache(
    state: State<'_, AppState>,
) -> Result<ClearDownloadCacheResult, String> {
    let downloads_dir = state.paths.downloads_dir();
    let protected: HashSet<PathBuf> = state
        .download_jobs
        .lock()
        .map_err(|e| e.to_string())?
        .values()
        .filter_map(|j| j.dest.clone())
        .collect();
    clear_download_cache_dir(&downloads_dir, &protected)
}

#[tauri::command]
pub fn recover_legacy_mod_data(
    state: State<'_, AppState>,
) -> Result<migration::RecoveryReport, String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    migration::recover_legacy_data(&state.paths, &mut cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_adult_content(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.adult_content = enabled;
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_game_version_warnings(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.game_version_warnings = enabled;
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_theme(state: State<'_, AppState>, theme: config::ThemePreference) -> Result<(), String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.theme = theme;
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_autoclick_free_download(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.autoclick_free_download = enabled;
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_install_click_behavior(
    state: State<'_, AppState>,
    behavior: config::InstallClickBehavior,
) -> Result<(), String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.install_click_behavior = behavior;
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_recent_downloads_limit(state: State<'_, AppState>, limit: u32) -> Result<(), String> {
    let limit = config::clamp_recent_downloads_limit(limit);
    {
        let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
        cfg.recent_downloads_limit = limit;
        config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())?;
    }
    commit_recent_downloads(&state);
    Ok(())
}

#[tauri::command]
pub fn set_active_game(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    if !cfg.managed_games.iter().any(|g| g.id == id) {
        return Err("Game is not managed".into());
    }
    cfg.last_active_game_id = Some(id);
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_api_key(state: State<'_, AppState>, key: String) -> Result<NexusUser, String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("API key cannot be empty".into());
    }
    if let Err(e) = nexus::store_api_key(&key) {
        log::warn!("keyring store failed: {e}");
        let file = state.paths.config_dir.join("nexus_api_key");
        nexus::store_api_key_file(&file, &key).map_err(|e| e.to_string())?;
    }
    *state.api_key.lock().map_err(|e| e.to_string())? = Some(key.clone());
    let client = NexusClient::new(key).map_err(|e| e.to_string())?;
    let user = client.validate().await.map_err(|e| e.to_string())?;
    *state.user.lock().map_err(|e| e.to_string())? = Some(user.clone());
    Ok(user)
}

#[tauri::command]
pub async fn validate_user(state: State<'_, AppState>) -> Result<NexusUser, String> {
    let client = state.client()?;
    let user = client.validate().await.map_err(|e| e.to_string())?;
    *state.user.lock().map_err(|e| e.to_string())? = Some(user.clone());
    Ok(user)
}

#[tauri::command]
pub fn clear_api_key(state: State<'_, AppState>) -> Result<(), String> {
    let _ = nexus::clear_api_key();
    let file = state.paths.config_dir.join("nexus_api_key");
    let _ = std::fs::remove_file(file);
    *state.api_key.lock().map_err(|e| e.to_string())? = None;
    *state.user.lock().map_err(|e| e.to_string())? = None;
    Ok(())
}

#[tauri::command]
pub async fn set_modio_api_key(state: State<'_, AppState>, key: String) -> Result<(), String> {
    let key = key.trim().to_string();
    if key.is_empty() {
        return Err("mod.io API key cannot be empty".into());
    }
    let client = ModioClient::new(&key).map_err(|e| e.to_string())?;
    client.validate().await.map_err(|e| e.to_string())?;
    if let Err(e) = modio_api::store_api_key(&key) {
        log::warn!("mod.io keyring store failed: {e}");
        let file = state.paths.config_dir.join("modio_api_key");
        modio_api::store_api_key_file(&file, &key).map_err(|e| e.to_string())?;
    }
    *state.modio_api_key.lock().map_err(|e| e.to_string())? = Some(key);
    Ok(())
}

#[tauri::command]
pub fn clear_modio_api_key(state: State<'_, AppState>) -> Result<(), String> {
    let _ = modio_api::clear_api_key();
    let file = state.paths.config_dir.join("modio_api_key");
    let _ = std::fs::remove_file(file);
    *state.modio_api_key.lock().map_err(|e| e.to_string())? = None;
    Ok(())
}

#[tauri::command]
pub async fn search_mods(
    state: State<'_, AppState>,
    domain: String,
    query: String,
    sort: Option<String>,
    category: Option<String>,
    tags_include: Option<Vec<String>>,
    tags_exclude: Option<Vec<String>>,
    game_version: Option<String>,
    offset: Option<u32>,
    count: Option<u32>,
) -> Result<nexus::ModSearchPage, String> {
    let adult = state
        .config
        .lock()
        .map_err(|e| e.to_string())?
        .adult_content;
    let client = state.client()?;
    let opts = nexus::BrowseSearchOpts {
        sort: sort.unwrap_or_else(|| "endorsements".into()),
        category,
        tags_include: tags_include.unwrap_or_default(),
        tags_exclude: tags_exclude.unwrap_or_default(),
        game_version,
        offset: offset.unwrap_or(0),
        count: count.unwrap_or(0),
    };
    client
        .search_mods(&domain, &query, adult, &opts)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogHit {
    pub source: String,
    pub id: String,
    pub name: String,
    pub summary: Option<String>,
    pub picture_url: Option<String>,
    pub author: Option<String>,
    pub downloads: Option<u64>,
    pub endorsements: Option<u64>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    // Nexus
    pub mod_id: Option<u64>,
    pub domain_name: Option<String>,
    // Thunderstore
    pub community: Option<String>,
    pub namespace: Option<String>,
    pub package_name: Option<String>,
    pub full_name: Option<String>,
    pub package_url: Option<String>,
    pub rating_score: Option<i64>,
    pub latest_version: Option<String>,
    // mod.io
    pub modio_game_id: Option<u32>,
    pub modio_mod_id: Option<u64>,
    pub profile_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CatalogSearchPage {
    pub items: Vec<CatalogHit>,
    pub total_count: u32,
    pub next_offset: u32,
    pub has_more: bool,
    pub nexus_available: bool,
    pub thunderstore_available: bool,
    pub modio_available: bool,
}

fn catalog_from_nexus(hit: nexus::ModSearchHit) -> CatalogHit {
    CatalogHit {
        source: "nexus".into(),
        id: format!("nexus:{}:{}", hit.domain_name, hit.mod_id),
        name: hit.name,
        summary: hit.summary,
        picture_url: hit.picture_url,
        author: hit.author,
        downloads: hit.downloads,
        endorsements: hit.endorsements,
        category: hit.category,
        tags: hit.tags,
        mod_id: Some(hit.mod_id),
        domain_name: Some(hit.domain_name),
        community: None,
        namespace: None,
        package_name: None,
        full_name: None,
        package_url: None,
        rating_score: None,
        latest_version: None,
        modio_game_id: None,
        modio_mod_id: None,
        profile_url: None,
    }
}

fn catalog_from_ts(detail: TsPackageDetail) -> CatalogHit {
    CatalogHit {
        source: "thunderstore".into(),
        id: format!(
            "thunderstore:{}:{}:{}",
            detail.community, detail.namespace, detail.name
        ),
        name: detail.name.clone(),
        summary: detail.description.clone(),
        picture_url: detail.icon_url.clone(),
        author: Some(detail.namespace.clone()),
        downloads: Some(detail.downloads),
        endorsements: None,
        category: detail.categories.first().cloned(),
        tags: detail.categories.clone(),
        mod_id: None,
        domain_name: None,
        community: Some(detail.community),
        namespace: Some(detail.namespace),
        package_name: Some(detail.name),
        full_name: Some(detail.full_name),
        package_url: Some(detail.package_url),
        rating_score: Some(detail.rating_score),
        latest_version: detail.latest_version,
        modio_game_id: None,
        modio_mod_id: None,
        profile_url: None,
    }
}

fn catalog_from_modio(hit: modio_api::ModioModHit) -> CatalogHit {
    CatalogHit {
        source: "modio".into(),
        id: format!("modio:{}:{}", hit.game_id, hit.mod_id),
        name: hit.name,
        summary: Some(hit.summary),
        picture_url: hit.picture_url,
        author: hit.author,
        downloads: Some(hit.downloads),
        endorsements: None,
        category: hit.tags.first().cloned(),
        tags: hit.tags,
        mod_id: None,
        domain_name: None,
        community: None,
        namespace: None,
        package_name: None,
        full_name: Some(hit.name_id),
        package_url: None,
        rating_score: None,
        latest_version: None,
        modio_game_id: Some(hit.game_id),
        modio_mod_id: Some(hit.mod_id),
        profile_url: Some(hit.profile_url),
    }
}

fn interleave_catalog_many(sources: Vec<Vec<CatalogHit>>) -> Vec<CatalogHit> {
    let mut iters: Vec<_> = sources
        .into_iter()
        .map(|v| v.into_iter().peekable())
        .collect();
    let mut out = Vec::new();
    loop {
        let mut progressed = false;
        for it in iters.iter_mut() {
            if let Some(item) = it.next() {
                out.push(item);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    out
}

fn catalog_source_unavailable_error(
    filter: &str,
    has_nexus: bool,
    has_ts: bool,
    has_modio: bool,
    collections: bool,
) -> String {
    let mut configured = Vec::new();
    if has_nexus {
        configured.push("Nexus");
    }
    if has_ts {
        configured.push("Thunderstore");
    }
    if has_modio && !collections {
        configured.push("mod.io");
    }
    if configured.is_empty() {
        if collections {
            return "Collections require a Nexus Mods domain or Thunderstore community for this game."
                .into();
        }
        return "No catalog sources configured. Set a Nexus domain, Thunderstore community, and/or mod.io game ID."
            .into();
    }
    let filter_label = match filter {
        "nexus" => "Nexus",
        "thunderstore" => "Thunderstore",
        "modio" => "mod.io",
        other => other,
    };
    let joined = configured.join(" / ");
    format!(
        "No results for the {filter_label} filter — this game only has {joined} configured. Switch source filter to All or {joined}."
    )
}

#[tauri::command]
pub async fn search_catalog(
    state: State<'_, AppState>,
    game_id: String,
    query: String,
    source_filter: Option<String>,
    sort: Option<String>,
    category: Option<String>,
    tags_include: Option<Vec<String>>,
    tags_exclude: Option<Vec<String>>,
    game_version: Option<String>,
    offset: Option<u32>,
    count: Option<u32>,
) -> Result<CatalogSearchPage, String> {
    let (nexus_domain, ts_community, modio_game_id, adult) = {
        let cfg = state.config.lock().map_err(|e| e.to_string())?;
        let game = cfg
            .managed_games
            .iter()
            .find(|g| g.id == game_id)
            .ok_or_else(|| "Managed game not found".to_string())?;
        (
            if game.nexus_domain.is_empty() {
                None
            } else {
                Some(game.nexus_domain.clone())
            },
            game.thunderstore_community.clone(),
            game.modio_game_id,
            cfg.adult_content,
        )
    };

    let filter = source_filter.unwrap_or_else(|| "all".into()).to_lowercase();
    let want_nexus = (filter == "all" || filter == "nexus") && nexus_domain.is_some();
    let want_ts = (filter == "all" || filter == "thunderstore") && ts_community.is_some();
    let want_modio = (filter == "all" || filter == "modio") && modio_game_id.is_some();
    if !want_nexus && !want_ts && !want_modio {
        return Err(catalog_source_unavailable_error(
            &filter,
            nexus_domain.is_some(),
            ts_community.is_some(),
            modio_game_id.is_some(),
            false,
        ));
    }

    let offset = offset.unwrap_or(0) as usize;
    let count = {
        let c = count.unwrap_or(24) as usize;
        if c == 0 {
            24
        } else {
            c
        }
    };
    let sort = sort.unwrap_or_else(|| {
        if query.trim().is_empty() {
            "downloads".into()
        } else {
            "relevance".into()
        }
    });

    let mut nexus_hits = Vec::new();
    let mut nexus_total = 0u32;
    let mut nexus_available = false;
    if want_nexus {
        if let Some(domain) = &nexus_domain {
            match state.client() {
                Ok(client) => {
                    nexus_available = true;
                    let nexus_sort = if sort == "downloads" || sort == "relevance" {
                        if query.trim().is_empty() {
                            "downloads".into()
                        } else {
                            "endorsements".into()
                        }
                    } else {
                        sort.clone()
                    };
                    let opts = nexus::BrowseSearchOpts {
                        sort: nexus_sort,
                        category: category.clone(),
                        tags_include: tags_include.clone().unwrap_or_default(),
                        tags_exclude: tags_exclude.clone().unwrap_or_default(),
                        game_version: game_version.clone(),
                        offset: offset as u32,
                        count: count as u32,
                    };
                    match client.search_mods(domain, &query, adult, &opts).await {
                        Ok(page) => {
                            nexus_total = page.total_count as u32;
                            nexus_hits = page.items.into_iter().map(catalog_from_nexus).collect();
                        }
                        Err(e) => log::warn!("Nexus catalog search failed: {e}"),
                    }
                }
                Err(e) => log::warn!("Nexus client unavailable for catalog: {e}"),
            }
        }
    }

    let mut ts_hits = Vec::new();
    let mut ts_total = 0u32;
    let mut thunderstore_available = false;
    if want_ts {
        if let Some(community) = &ts_community {
            thunderstore_available = true;
            let client = ThunderstoreClient::new().map_err(|e| e.to_string())?;
            let cached = {
                let cache = state.ts_package_cache.lock().map_err(|e| e.to_string())?;
                cache.get(community).and_then(|(at, pkgs)| {
                    if at.elapsed() < Duration::from_secs(15 * 60) {
                        Some(pkgs.clone())
                    } else {
                        None
                    }
                })
            };
            let (page, total) = if let Some(packages) = cached {
                filter_ts_packages(community, &packages, &query, adult, offset, count)
            } else {
                let packages = client
                    .list_packages(community)
                    .await
                    .map_err(|e| e.to_string())?;
                if let Ok(mut cache) = state.ts_package_cache.lock() {
                    cache.insert(community.clone(), (Instant::now(), packages.clone()));
                }
                filter_ts_packages(community, &packages, &query, adult, offset, count)
            };
            ts_total = total as u32;
            ts_hits = page.into_iter().map(catalog_from_ts).collect();
        }
    }

    let mut modio_hits = Vec::new();
    let mut modio_total = 0u32;
    let mut modio_available = false;
    if want_modio {
        if let Some(gid) = modio_game_id {
            let key = state
                .modio_api_key
                .lock()
                .map_err(|e| e.to_string())?
                .clone();
            match key {
                Some(key) => {
                    modio_available = true;
                    match ModioClient::new(&key) {
                        Ok(client) => {
                            match client
                                .search_mods(gid, &query, offset as u32, count as u32, adult)
                                .await
                            {
                                Ok((hits, total)) => {
                                    modio_total = total;
                                    modio_hits = hits.into_iter().map(catalog_from_modio).collect();
                                }
                                Err(e) => log::warn!("mod.io catalog search failed: {e}"),
                            }
                        }
                        Err(e) => log::warn!("mod.io client unavailable: {e}"),
                    }
                }
                None => log::warn!("mod.io game id set but API key missing"),
            }
        }
    }

    let items = if filter == "all" {
        let mut parts = Vec::new();
        if want_nexus {
            parts.push(nexus_hits);
        }
        if want_ts {
            parts.push(ts_hits);
        }
        if want_modio {
            parts.push(modio_hits);
        }
        interleave_catalog_many(parts)
    } else if filter == "thunderstore" {
        ts_hits
    } else if filter == "modio" {
        modio_hits
    } else {
        nexus_hits
    };

    let total_count = nexus_total
        .saturating_add(ts_total)
        .saturating_add(modio_total);
    let has_more = (offset + count) < nexus_total as usize
        || (offset + count) < ts_total as usize
        || (offset + count) < modio_total as usize;

    Ok(CatalogSearchPage {
        items,
        total_count,
        next_offset: (offset + count) as u32,
        has_more,
        nexus_available,
        thunderstore_available,
        modio_available,
    })
}

fn filter_ts_packages(
    community: &str,
    packages: &[thunderstore::TsPackage],
    query: &str,
    include_nsfw: bool,
    offset: usize,
    count: usize,
) -> (Vec<TsPackageDetail>, usize) {
    let mut packages: Vec<_> = packages
        .iter()
        .filter(|p| !p.is_deprecated)
        .cloned()
        .collect();
    if !include_nsfw {
        packages.retain(|p| !p.has_nsfw_content);
    }
    let q = query.trim().to_lowercase();
    if !q.is_empty() {
        packages.retain(|p| {
            p.name.to_lowercase().contains(&q)
                || p.full_name.to_lowercase().contains(&q)
                || p.owner.to_lowercase().contains(&q)
                || p.versions
                    .first()
                    .map(|v| v.description.to_lowercase().contains(&q))
                    .unwrap_or(false)
                || p.categories.iter().any(|c| c.to_lowercase().contains(&q))
        });
    }
    packages.sort_by(|a, b| {
        b.is_pinned.cmp(&a.is_pinned).then_with(|| {
            let da = a.versions.first().map(|v| v.downloads).unwrap_or(0);
            let db = b.versions.first().map(|v| v.downloads).unwrap_or(0);
            db.cmp(&da)
        })
    });
    let total = packages.len();
    let page = packages
        .into_iter()
        .skip(offset)
        .take(count)
        .map(|p| {
            // Rebuild detail similarly to thunderstore::detail_from_package
            let latest = p.versions.first();
            TsPackageDetail {
                community: community.to_string(),
                namespace: p.owner.clone(),
                name: p.name.clone(),
                full_name: p.full_name.clone(),
                package_url: p.package_url.clone(),
                uuid4: p.uuid4.clone(),
                rating_score: p.rating_score,
                is_deprecated: p.is_deprecated,
                has_nsfw_content: p.has_nsfw_content,
                categories: p.categories.clone(),
                description: latest.map(|v| v.description.clone()),
                icon_url: latest.map(|v| v.icon.clone()),
                downloads: latest.map(|v| v.downloads).unwrap_or(0),
                latest_version: latest.map(|v| v.version_number.clone()),
                versions: p.versions,
            }
        })
        .collect();
    (page, total)
}

fn filter_ts_modpacks(
    community: &str,
    packages: &[thunderstore::TsPackage],
    query: &str,
    include_nsfw: bool,
    offset: usize,
    count: usize,
) -> (Vec<TsPackageDetail>, usize) {
    let packs: Vec<_> = packages
        .iter()
        .filter(|p| thunderstore::is_modpack_category(&p.categories))
        .cloned()
        .collect();
    filter_ts_packages(community, &packs, query, include_nsfw, offset, count)
}

fn snapshot_mod_ids(
    paths: &crate::config::Paths,
    game_id: &str,
) -> Result<HashSet<String>, String> {
    let order = mods::load_loadorder(paths, game_id).map_err(|e| e.to_string())?;
    Ok(order.mods.into_iter().map(|m| m.id).collect())
}

async fn load_ts_packages_cached(
    state: &AppState,
    community: &str,
) -> Result<Vec<thunderstore::TsPackage>, String> {
    let cached = {
        let cache = state.ts_package_cache.lock().map_err(|e| e.to_string())?;
        cache.get(community).and_then(|(at, pkgs)| {
            if at.elapsed() < Duration::from_secs(15 * 60) {
                Some(pkgs.clone())
            } else {
                None
            }
        })
    };
    if let Some(packages) = cached {
        return Ok(packages);
    }
    let client = ThunderstoreClient::new().map_err(|e| e.to_string())?;
    let packages = client
        .list_packages(community)
        .await
        .map_err(|e| e.to_string())?;
    if let Ok(mut cache) = state.ts_package_cache.lock() {
        cache.insert(community.to_string(), (Instant::now(), packages.clone()));
    }
    Ok(packages)
}

fn collection_hit_from_ts(detail: TsPackageDetail) -> nexus::CollectionHit {
    let slug = format!("{}-{}", detail.namespace, detail.name);
    let dep_count = detail
        .versions
        .first()
        .map(|v| v.dependencies.len() as u64 + 1);
    nexus::CollectionHit {
        slug,
        name: detail.name.clone(),
        summary: detail.description.clone(),
        endorsements: None,
        total_downloads: Some(detail.downloads),
        domain_name: None,
        revision_number: None,
        tile_image_url: detail.icon_url.clone(),
        author: Some(detail.namespace.clone()),
        category: detail.categories.first().cloned(),
        overall_rating: Some(detail.rating_score as f64),
        overall_rating_count: None,
        created_at: None,
        updated_at: None,
        mod_count: dep_count,
        file_size: detail.versions.first().map(|v| v.file_size),
        source: "thunderstore".into(),
        id: mods::thunderstore_modpack_id(&detail.community, &detail.namespace, &detail.name),
        community: Some(detail.community),
        namespace: Some(detail.namespace),
        package_name: Some(detail.name),
        latest_version: detail.latest_version,
    }
}

fn apply_ts_depends_on(
    paths: &crate::config::Paths,
    game_id: &str,
    packages: &[TsPackageDetail],
) -> Result<(), String> {
    let known: HashSet<String> = packages
        .iter()
        .map(|p| mods::thunderstore_mod_id(&p.namespace, &p.name))
        .collect();
    for pkg in packages {
        let id = mods::thunderstore_mod_id(&pkg.namespace, &pkg.name);
        let deps: Vec<String> = pkg
            .versions
            .first()
            .map(|v| {
                v.dependencies
                    .iter()
                    .filter_map(|raw| TsDependency::parse(raw))
                    .map(|d| mods::thunderstore_mod_id(&d.namespace, &d.name))
                    .filter(|dep_id| known.contains(dep_id))
                    .collect()
            })
            .unwrap_or_default();
        mods::merge_mod_meta(paths, game_id, &id, &[], &deps, None, None)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// What to do when the package is already staged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StagePolicy {
    /// Keep what is installed unless the incoming build is newer.
    KeepNewer,
    /// Install exactly this version, downgrading if needed. Dependency pins ask
    /// for this: a mod set only loads with the versions it was tested against.
    MatchResolved,
    /// Download again even if the same version is recorded (repairing staging).
    Force,
}

/// Stage `pkg` at its first listed version.
async fn stage_thunderstore_package(
    app: &tauri::AppHandle,
    state: &AppState,
    client: &ThunderstoreClient,
    game_id: &str,
    community: &str,
    pkg: &TsPackageDetail,
    display_name: &str,
    enabled: Option<bool>,
    reuse_dl_id: Option<String>,
    policy: StagePolicy,
) -> Result<StagedMod, String> {
    let ns = pkg.namespace.clone();
    let pkg_name = pkg.name.clone();
    let ver = pkg
        .versions
        .first()
        .ok_or_else(|| format!("No versions for {}", pkg.full_name))?;
    let incoming_ver = &ver.version_number;
    let order = mods::load_loadorder(&state.paths, game_id).map_err(|e| e.to_string())?;
    if let Some(existing) = mods::find_thunderstore_package(&order, &ns, &pkg_name) {
        let keep_existing = match policy {
            StagePolicy::KeepNewer => {
                !mods::incoming_is_newer(existing.version.as_deref(), incoming_ver)
            }
            StagePolicy::MatchResolved => {
                existing.version.as_deref() == Some(incoming_ver.as_str())
            }
            StagePolicy::Force => false,
        };
        if keep_existing {
            let id = existing.id.clone();
            let apply_enabled = match enabled {
                Some(true) => Some(true),
                _ => None,
            };
            return mods::merge_mod_meta(&state.paths, game_id, &id, &[], &[], None, apply_enabled)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Missing staged package {id}"));
        }
    }

    let label = format!("{}-{}", pkg.full_name, incoming_ver);
    let is_new = reuse_dl_id.is_none();
    let dl_id = reuse_dl_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let dest = state.paths.downloads_dir().join(format!(
        "ts_{}_{}.zip",
        dl_id,
        sanitize_filename::sanitize(&pkg_name)
    ));

    if is_new {
        push_download(
            state,
            DownloadItem::new(
                dl_id.clone(),
                label.clone(),
                "downloading",
                None,
                thunderstore_page(game_id, community, &ns, &pkg_name),
            ),
        );
    } else {
        reset_download_row(state, &dl_id);
    }
    let _ = app.emit("downloads-changed", ());

    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    let ts_source = DownloadResumeSource::Thunderstore {
        game_id: game_id.to_string(),
        community: community.to_string(),
        namespace: ns.clone(),
        name: pkg_name.clone(),
        version: Some(incoming_ver.clone()),
        display_name: display_name.to_string(),
        enabled,
    };
    {
        let mut jobs = state.download_jobs.lock().map_err(|e| e.to_string())?;
        jobs.insert(
            dl_id.clone(),
            DownloadJob {
                cancel: cancel.clone(),
                pause: pause.clone(),
                batch_id: None,
                dest: Some(dest.clone()),
                source: Some(ts_source.clone()),
            },
        );
    }
    store_restart_meta(state, &dl_id, ts_source);

    let control = TransferControl {
        cancel: cancel.clone(),
        pause: pause.clone(),
    };
    let on_progress = progress_callback(Some(app), dl_id.clone());
    match client
        .download_version(ver, &dest, Some(&control), on_progress.as_ref())
        .await
    {
        Ok(()) => {
            update_download(state, &dl_id, "extracting", None);
            let _ = app.emit("downloads-changed", ());
            let ts_file_name = mods::pick_staged_file_name(
                display_name,
                None,
                crate::nexus::filename_from_url(&ver.download_url).as_deref(),
            );
            assign_download_file_name(state, &dl_id, ts_file_name.as_deref());
            match mods::stage_thunderstore_mod(
                &state.paths,
                game_id,
                community,
                &ns,
                &pkg_name,
                incoming_ver,
                Some(pkg.uuid4.clone()),
                display_name,
                ts_file_name,
                Some(ver.icon.clone()).filter(|icon| !icon.trim().is_empty()),
                &dest,
            ) {
                Ok(staged) => {
                    if let Some(false) = enabled {
                        let _ = mods::merge_mod_meta(
                            &state.paths,
                            game_id,
                            &staged.id,
                            &[],
                            &[],
                            None,
                            Some(false),
                        );
                    }
                    update_download(state, &dl_id, "done", None);
                    remove_download_archive(&dest, &state.paths.downloads_dir());
                    finish_download_success(state, &dl_id);
                    let _ = app.emit("downloads-changed", ());
                    Ok(staged)
                }
                Err(e) => {
                    let msg = e.to_string();
                    remove_download_archive(&dest, &state.paths.downloads_dir());
                    update_download(state, &dl_id, "failed", Some(msg.clone()));
                    clear_download_job(state, &dl_id);
                    let _ = app.emit("downloads-changed", ());
                    Err(msg)
                }
            }
        }
        Err(e) => {
            let msg = e.to_string();
            remove_download_archive(&dest, &state.paths.downloads_dir());
            update_download(state, &dl_id, "failed", Some(msg.clone()));
            clear_download_job(state, &dl_id);
            let _ = app.emit("downloads-changed", ());
            Err(msg)
        }
    }
}

#[tauri::command]
pub async fn get_thunderstore_package(
    community: String,
    namespace: String,
    name: String,
) -> Result<TsPackageDetail, String> {
    let client = ThunderstoreClient::new().map_err(|e| e.to_string())?;
    client
        .get_package(&community, &namespace, &name)
        .await
        .map_err(|e| e.to_string())
}

/// Resolve and stage a Thunderstore package plus its full dependency closure.
/// Returns staged mods (deps first) and the resolved package details for `depends_on` wiring.
async fn stage_thunderstore_with_deps(
    app: &tauri::AppHandle,
    state: &AppState,
    client: &ThunderstoreClient,
    game_id: &str,
    community: &str,
    namespace: &str,
    name: &str,
    version: Option<&str>,
    root_enabled: Option<bool>,
    root_display: Option<&str>,
) -> Result<(Vec<StagedMod>, Vec<TsPackageDetail>), String> {
    let order = client
        .resolve_install_order(community, namespace, name, version)
        .await
        .map_err(|e| e.to_string())?;

    let mut staged_all = Vec::new();
    for pkg in &order {
        let is_root = pkg.name == name && pkg.namespace == namespace;
        let display = if is_root {
            root_display
                .map(|s| s.to_string())
                .unwrap_or_else(|| pkg.name.clone())
        } else {
            format!("{} (dependency)", pkg.name)
        };
        let enabled = if is_root { root_enabled } else { None };
        let staged = stage_thunderstore_package(
            app,
            state,
            client,
            game_id,
            community,
            pkg,
            &display,
            enabled,
            None,
            // Never downgrade the package the user asked for, only its deps.
            if is_root {
                StagePolicy::KeepNewer
            } else {
                StagePolicy::MatchResolved
            },
        )
        .await?;
        staged_all.push(staged);
    }
    apply_ts_depends_on(&state.paths, game_id, &order)?;
    Ok((staged_all, order))
}

#[tauri::command]
pub async fn download_thunderstore_mod(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    game_id: String,
    community: String,
    namespace: String,
    name: String,
    version: Option<String>,
    record_as_modpack: Option<bool>,
) -> Result<Vec<StagedMod>, String> {
    let client = ThunderstoreClient::new().map_err(|e| e.to_string())?;
    let before = snapshot_mod_ids(&state.paths, &game_id)?;
    let (staged_all, order) = stage_thunderstore_with_deps(
        &app,
        &state,
        &client,
        &game_id,
        &community,
        &namespace,
        &name,
        version.as_deref(),
        None,
        None,
    )
    .await?;

    if record_as_modpack.unwrap_or(false) {
        let member_ids: Vec<String> = order
            .iter()
            .map(|p| mods::thunderstore_mod_id(&p.namespace, &p.name))
            .collect();
        let new_ids: Vec<String> = member_ids
            .iter()
            .filter(|id| !before.contains(*id))
            .cloned()
            .collect();
        let root = order
            .iter()
            .find(|p| p.namespace == namespace && p.name == name);
        let collection = InstalledCollection {
            id: mods::thunderstore_modpack_id(&community, &namespace, &name),
            source: CollectionSource::Thunderstore,
            kind: CollectionKind::Modpack,
            name: root.map(|p| p.name.clone()).unwrap_or_else(|| name.clone()),
            slug: Some(format!("{namespace}-{name}")),
            namespace: Some(namespace.clone()),
            package_name: Some(name.clone()),
            community: Some(community.clone()),
            revision: None,
            version: root.and_then(|p| p.latest_version.clone()),
            profile_code: None,
            mod_ids: member_ids,
            installed_at: String::new(),
            game_versions: None,
        };
        mods::record_installed_collection(&state.paths, &game_id, collection, &new_ids)
            .map_err(|e| e.to_string())?;
    }

    Ok(staged_all)
}

#[tauri::command]
pub async fn import_thunderstore_profile(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    game_id: String,
    code: String,
) -> Result<InstalledCollection, String> {
    let community = {
        let cfg = state.config.lock().map_err(|e| e.to_string())?;
        let game = cfg
            .managed_games
            .iter()
            .find(|g| g.id == game_id)
            .ok_or_else(|| "Managed game not found".to_string())?;
        game.thunderstore_community
            .clone()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "Set a Thunderstore community for this game first.".to_string())?
    };
    let code = code.trim().to_string();
    if code.is_empty() {
        return Err("Paste a Thunderstore / r2modman profile code.".into());
    }

    let client = ThunderstoreClient::new().map_err(|e| e.to_string())?;
    let bytes = client
        .fetch_legacy_profile(&code)
        .await
        .map_err(|e| e.to_string())?;
    let temp = state
        .paths
        .downloads_dir()
        .join(format!("ts_profile_{}", sanitize_filename::sanitize(&code)));
    let manifest = thunderstore::extract_profile_zip(&bytes, &temp).map_err(|e| e.to_string())?;
    if let Some(slug) = manifest.community.as_deref() {
        if !slug.is_empty() && !thunderstore::community_slug_matches(slug, &community) {
            let _ = std::fs::remove_dir_all(&temp);
            return Err(format!(
                "This profile is for '{slug}', but this game is set to Thunderstore community '{community}'."
            ));
        }
    }

    let packages = load_ts_packages_cached(&state, &community).await?;
    let by_key: HashMap<String, thunderstore::TsPackage> = packages
        .into_iter()
        .map(|p| (format!("{}-{}", p.owner, p.name).to_lowercase(), p))
        .collect();

    let before = snapshot_mod_ids(&state.paths, &game_id)?;
    let mut member_ids = Vec::new();
    let mut seen_pkg_ids: HashSet<String> = HashSet::new();
    let mut details_for_deps = Vec::new();

    for m in &manifest.mods {
        let Some((ns, pkg_name)) = thunderstore::parse_package_ident(&m.name) else {
            log::warn!("Skipping unreadable profile package {}", m.name);
            continue;
        };
        let key = format!("{ns}-{pkg_name}").to_lowercase();
        if !by_key.contains_key(&key) {
            log::warn!("Profile package {ns}-{pkg_name} not found in {community}");
            continue;
        }
        let ver = m.version.as_string();
        let enabled = if m.enabled { None } else { Some(false) };
        match stage_thunderstore_with_deps(
            &app,
            &state,
            &client,
            &game_id,
            &community,
            &ns,
            &pkg_name,
            Some(&ver),
            enabled,
            Some(&pkg_name),
        )
        .await
        {
            Ok((staged, order)) => {
                for s in staged {
                    if seen_pkg_ids.insert(s.id.clone()) {
                        member_ids.push(s.id);
                    }
                }
                for detail in order {
                    let id = mods::thunderstore_mod_id(&detail.namespace, &detail.name);
                    if details_for_deps.iter().all(|d: &TsPackageDetail| {
                        mods::thunderstore_mod_id(&d.namespace, &d.name) != id
                    }) {
                        details_for_deps.push(detail);
                    }
                }
            }
            Err(e) => {
                log::warn!("Failed to stage profile package {ns}-{pkg_name}: {e}");
            }
        }
    }

    apply_ts_depends_on(&state.paths, &game_id, &details_for_deps)?;

    let overlay_id = format!(
        "overlay_{}",
        mods::thunderstore_profile_id(&code).replace(':', "_")
    );
    if thunderstore::profile_has_overlay_files(&temp) {
        let overlay_name = format!("{} configs", manifest.profile_name);
        let staged =
            mods::stage_overlay_dir(&state.paths, &game_id, &overlay_id, &overlay_name, &temp)
                .map_err(|e| e.to_string())?;
        member_ids.push(staged.id);
    }
    let _ = std::fs::remove_dir_all(&temp);

    let new_ids: Vec<String> = member_ids
        .iter()
        .filter(|id| !before.contains(*id))
        .cloned()
        .collect();
    let collection = InstalledCollection {
        id: mods::thunderstore_profile_id(&code),
        source: CollectionSource::Thunderstore,
        kind: CollectionKind::Profile,
        name: manifest.profile_name,
        slug: None,
        namespace: None,
        package_name: None,
        community: Some(community),
        revision: None,
        version: None,
        profile_code: Some(code),
        mod_ids: member_ids,
        installed_at: String::new(),
        game_versions: None,
    };
    mods::record_installed_collection(&state.paths, &game_id, collection, &new_ids)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_modio_mod(
    state: State<'_, AppState>,
    game_id: u32,
    mod_id: u64,
) -> Result<ModioModDetail, String> {
    let key = state
        .modio_api_key
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or_else(|| "mod.io API key required".to_string())?;
    let client = ModioClient::new(&key).map_err(|e| e.to_string())?;
    client
        .get_mod(game_id, mod_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn modio_files(
    state: State<'_, AppState>,
    game_id: u32,
    mod_id: u64,
) -> Result<Vec<ModioFileInfo>, String> {
    let key = state
        .modio_api_key
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or_else(|| "mod.io API key required".to_string())?;
    let client = ModioClient::new(&key).map_err(|e| e.to_string())?;
    let detail = client
        .get_mod(game_id, mod_id)
        .await
        .map_err(|e| e.to_string())?;
    client
        .list_files(game_id, mod_id, detail.primary_file_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn download_modio_mod(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    game_id: String,
    modio_game_id: u32,
    mod_id: u64,
    file_id: Option<u64>,
    name: String,
    version: Option<String>,
    install_deps: Option<bool>,
) -> Result<Vec<StagedMod>, String> {
    let key = state
        .modio_api_key
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or_else(|| "mod.io API key required".to_string())?;
    let client = ModioClient::new(&key).map_err(|e| e.to_string())?;
    let install_deps = install_deps.unwrap_or(true);

    let mut queue: Vec<(u64, String, Option<u64>, Option<String>, bool)> = Vec::new();
    let mut dep_ids: Vec<u64> = Vec::new();
    if install_deps {
        match client.dependency_mod_ids(modio_game_id, mod_id).await {
            Ok(deps) => {
                for dep_id in deps {
                    dep_ids.push(dep_id);
                    let detail = client
                        .get_mod(modio_game_id, dep_id)
                        .await
                        .map_err(|e| e.to_string())?;
                    queue.push((
                        dep_id,
                        format!("{} (dependency)", detail.name),
                        detail.primary_file_id,
                        None,
                        true,
                    ));
                }
            }
            Err(e) => log::warn!("mod.io deps lookup failed: {e}"),
        }
    }
    queue.push((mod_id, name, file_id, version, false));

    let mut staged_all = Vec::new();
    for (mid, label, fid, ver, _is_dep) in queue {
        let detail = client
            .get_mod(modio_game_id, mid)
            .await
            .map_err(|e| e.to_string())?;
        let resolved_file = fid
            .or(detail.primary_file_id)
            .ok_or_else(|| format!("No downloadable file for mod.io mod {mid}"))?;
        let incoming_ver = ver.clone().or(None);
        let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
        if let Some(existing) = mods::find_modio_mod(&order, modio_game_id, mid) {
            let same_file = existing.modio_file_id == Some(resolved_file);
            let keep = mods::keep_existing_modio_file(
                same_file,
                existing.version.as_deref(),
                incoming_ver.as_deref(),
            );
            if keep {
                staged_all.push(existing.clone());
                continue;
            }
        }
        let dl_id = uuid::Uuid::new_v4().to_string();
        let dest = state.paths.downloads_dir().join(format!(
            "modio_{}_{}_{}.zip",
            dl_id,
            sanitize_filename::sanitize(&detail.name_id),
            resolved_file
        ));
        {
            push_download(
                &state,
                DownloadItem::new(
                    dl_id.clone(),
                    label.clone(),
                    "downloading",
                    None,
                    modio_page(&game_id, modio_game_id, mid),
                ),
            );
        }
        let modio_source = DownloadResumeSource::Modio {
            game_id: game_id.clone(),
            modio_game_id,
            mod_id: mid,
            file_id: resolved_file,
            name: label.clone(),
            version: ver.clone(),
            name_id: detail.name_id.clone(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        {
            let mut jobs = state.download_jobs.lock().map_err(|e| e.to_string())?;
            jobs.insert(
                dl_id.clone(),
                DownloadJob {
                    cancel: cancel.clone(),
                    pause: pause.clone(),
                    batch_id: None,
                    dest: Some(dest.clone()),
                    source: Some(modio_source.clone()),
                },
            );
        }
        store_restart_meta(&state, &dl_id, modio_source);
        let _ = app.emit("downloads-changed", ());
        let on_progress = progress_callback(Some(&app), dl_id.clone());
        match client
            .download_file(
                modio_game_id,
                mid,
                Some(resolved_file),
                &dest,
                on_progress.as_ref(),
            )
            .await
        {
            Ok(()) => {
                update_download(&state, &dl_id, "extracting", None);
                let _ = app.emit("downloads-changed", ());
                let modio_label = modio_file_label(
                    &client,
                    modio_game_id,
                    mid,
                    resolved_file,
                    detail.primary_file_id,
                    &detail.name,
                )
                .await;
                assign_download_file_name(&state, &dl_id, modio_label.as_deref());
                match mods::stage_modio_mod(
                    &state.paths,
                    &game_id,
                    modio_game_id,
                    mid,
                    resolved_file,
                    &detail.name,
                    ver,
                    modio_label,
                    detail.picture_url.clone(),
                    &dest,
                ) {
                    Ok(staged) => {
                        staged_all.push(staged);
                        update_download(&state, &dl_id, "done", None);
                        remove_download_archive(&dest, &state.paths.downloads_dir());
                        finish_download_success(&state, &dl_id);
                        let _ = app.emit("downloads-changed", ());
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        remove_download_archive(&dest, &state.paths.downloads_dir());
                        update_download(&state, &dl_id, "failed", Some(msg.clone()));
                        clear_download_job(&state, &dl_id);
                        let _ = app.emit("downloads-changed", ());
                        return Err(msg);
                    }
                }
            }
            Err(e) => {
                let msg = e.to_string();
                remove_download_archive(&dest, &state.paths.downloads_dir());
                update_download(&state, &dl_id, "failed", Some(msg.clone()));
                clear_download_job(&state, &dl_id);
                let _ = app.emit("downloads-changed", ());
                return Err(msg);
            }
        }
    }

    let root_id = mods::modio_identity_id(modio_game_id, mod_id);
    let dep_staged: Vec<String> = dep_ids
        .iter()
        .map(|id| mods::modio_identity_id(modio_game_id, *id))
        .collect();
    let _ = mods::merge_mod_meta(
        &state.paths,
        &game_id,
        &root_id,
        &[],
        &dep_staged,
        None,
        None,
    );
    Ok(staged_all)
}

fn staging_contains_filename(root: &Path, file_name: &str) -> bool {
    if !root.is_dir() {
        return false;
    }
    WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .any(|entry| {
            entry.file_type().is_file()
                && entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(file_name)
        })
}

fn staged_nexus_mods(mods: &[StagedMod]) -> HashMap<(String, u64), nexus::StagedNexusMod> {
    let mut out = HashMap::new();
    for m in mods {
        if m.source != ModSource::Nexus || m.nexus_mod_id == 0 || m.domain.is_empty() {
            continue;
        }
        out.entry((m.domain.to_lowercase(), m.nexus_mod_id))
            .or_insert_with(|| nexus::StagedNexusMod {
                staged_id: m.id.clone(),
                name: m.name.clone(),
                enabled: m.enabled,
            });
    }
    out
}

struct RequirementSeed {
    staged_id: String,
    name: String,
    domain: String,
    mod_id: u64,
}

fn seed_from_staged(m: &StagedMod) -> Option<RequirementSeed> {
    if m.source != ModSource::Nexus || m.nexus_mod_id == 0 || m.domain.is_empty() {
        return None;
    }
    Some(RequirementSeed {
        staged_id: m.id.clone(),
        name: m.name.clone(),
        domain: m.domain.clone(),
        mod_id: m.nexus_mod_id,
    })
}

async fn collect_requirement_seeds(
    state: &AppState,
    seeds: Vec<RequirementSeed>,
    staged: HashMap<(String, u64), nexus::StagedNexusMod>,
) -> nexus::MissingRequirements {
    if seeds.is_empty() {
        return nexus::MissingRequirements::default();
    }
    let client = match state.client() {
        Ok(client) => client,
        Err(e) => {
            return nexus::MissingRequirements {
                error: Some(e),
                ..Default::default()
            };
        }
    };
    let games = match load_nexus_games_cached(state).await {
        Ok(games) => games,
        Err(e) => {
            return nexus::MissingRequirements {
                error: Some(e),
                ..Default::default()
            };
        }
    };
    let mut by_id: HashMap<u64, String> = HashMap::new();
    let mut by_domain: HashMap<String, u64> = HashMap::new();
    for game in games {
        by_domain.insert(game.domain_name.to_lowercase(), game.id);
        by_id.insert(game.id, game.domain_name);
    }
    let mut roots = Vec::new();
    let mut errors = Vec::new();
    for seed in seeds {
        let key = seed.domain.to_lowercase();
        let game_id = if let Some(id) = by_domain.get(&key).copied() {
            id
        } else {
            match state.get_game_cached(&seed.domain).await {
                Ok(info) => {
                    by_domain.insert(key, info.id);
                    by_id.insert(info.id, info.domain_name);
                    info.id
                }
                Err(e) => {
                    errors.push(format!("{}: {e}", seed.name));
                    continue;
                }
            }
        };
        roots.push(nexus::RequirementRoot {
            staged_id: seed.staged_id,
            name: seed.name,
            game_id,
            mod_id: seed.mod_id,
        });
    }
    if roots.is_empty() {
        return nexus::MissingRequirements {
            error: (!errors.is_empty()).then(|| errors.join(" ")),
            ..Default::default()
        };
    }
    let mut missing = nexus::collect_missing(&client, &roots, &by_id, &staged).await;
    if !errors.is_empty() {
        let extra = errors.join(" ");
        missing.error = Some(match missing.error.take() {
            Some(existing) => format!("{existing} {extra}"),
            None => extra,
        });
    }
    missing
}

#[tauri::command]
pub async fn missing_nexus_requirements(
    state: State<'_, AppState>,
    game_id: String,
    staged_id: Option<String>,
) -> Result<nexus::MissingRequirements, String> {
    let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    let staged = staged_nexus_mods(&order.mods);
    let seeds = match staged_id.as_deref().filter(|id| !id.is_empty()) {
        Some(id) => {
            let staged_mod = order
                .mods
                .iter()
                .find(|m| m.id == id)
                .ok_or_else(|| format!("Staged mod not found: {id}"))?;
            seed_from_staged(staged_mod)
                .ok_or_else(|| "Requirements are available for Nexus mods.".to_string())
                .map(|seed| vec![seed])?
        }
        None => order.mods.iter().filter_map(seed_from_staged).collect(),
    };
    Ok(collect_requirement_seeds(&state, seeds, staged).await)
}

#[derive(Debug, Clone, Serialize)]
pub struct GameVersionReport {
    pub installed: Option<String>,
    pub override_version: Option<String>,
    pub current: Option<String>,
    /// `override`, `detected`, or `unknown`.
    pub source: String,
    pub mods: Vec<GameVersionModAdvice>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameVersionModAdvice {
    pub staged_id: String,
    pub status: mods::game_version::ModVersionStatus,
    pub supported: Vec<String>,
    pub message: String,
}

#[tauri::command]
pub async fn check_mod_game_versions(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<GameVersionReport, String> {
    let (install_path, override_version, nexus_domain, adult) = {
        let cfg = state.config.lock().map_err(|e| e.to_string())?;
        let game = cfg
            .managed_games
            .iter()
            .find(|g| g.id == game_id)
            .ok_or_else(|| "Managed game not found".to_string())?;
        (
            game.install_path.clone(),
            game.game_version
                .clone()
                .filter(|version| !version.trim().is_empty()),
            game.nexus_domain.clone(),
            cfg.adult_content,
        )
    };
    let installed = games::detect_install_version(Path::new(&install_path));
    let current = override_version.clone().or_else(|| installed.clone());
    let source = if override_version.is_some() {
        "override"
    } else if installed.is_some() {
        "detected"
    } else {
        "unknown"
    };

    let mut order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    let mut collections =
        mods::list_installed_collections(&state.paths, &game_id).map_err(|e| e.to_string())?;
    refresh_version_support(
        &state,
        &game_id,
        &nexus_domain,
        adult,
        &mut order,
        &mut collections,
    )
    .await;

    let mods = order
        .mods
        .iter()
        .map(|staged| {
            let tags = staged.supported_game_versions.clone().unwrap_or_default();
            let mut collection_versions = Vec::new();
            for collection in &collections {
                if collection.mod_ids.iter().any(|id| id == &staged.id) {
                    if let Some(versions) = &collection.game_versions {
                        for version in versions {
                            if !collection_versions
                                .iter()
                                .any(|existing: &String| existing.eq_ignore_ascii_case(version))
                            {
                                collection_versions.push(version.clone());
                            }
                        }
                    }
                }
            }
            let advice = mods::game_version::advise(&mods::game_version::AdviseInput {
                domain: &nexus_domain,
                current: current.as_deref(),
                tags: &tags,
                file_name: staged.file_name.as_deref(),
                mod_version: staged.version.as_deref(),
                collection_versions: &collection_versions,
            });
            GameVersionModAdvice {
                staged_id: staged.id.clone(),
                status: advice.status,
                supported: advice.supported,
                message: advice.message,
            }
        })
        .collect();

    Ok(GameVersionReport {
        installed,
        override_version,
        current,
        source: source.into(),
        mods,
    })
}

async fn refresh_version_support(
    state: &AppState,
    game_id: &str,
    nexus_domain: &str,
    adult: bool,
    order: &mut mods::LoadOrder,
    collections: &mut [InstalledCollection],
) {
    let client = match state.client() {
        Ok(client) => client,
        Err(_) => return,
    };
    let mut by_domain: HashMap<String, Vec<(String, u64)>> = HashMap::new();
    for staged in &order.mods {
        if staged.source != ModSource::Nexus || staged.nexus_mod_id == 0 {
            continue;
        }
        if staged.supported_game_versions.is_some()
            && staged.version_support_generation >= mods::game_version::VERSION_SUPPORT_GENERATION
        {
            continue;
        }
        let domain = if staged.domain.trim().is_empty() {
            nexus_domain.to_string()
        } else {
            staged.domain.clone()
        };
        if domain.is_empty() {
            continue;
        }
        by_domain
            .entry(domain)
            .or_default()
            .push((staged.id.clone(), staged.nexus_mod_id));
    }
    let mut tag_updates: HashMap<String, Vec<String>> = HashMap::new();
    for (domain, rows) in &by_domain {
        let ids: Vec<u64> = rows.iter().map(|(_, id)| *id).collect();
        match client.mod_version_tags(domain, &ids).await {
            Ok(found) => {
                for (staged_id, mod_id) in rows {
                    tag_updates.insert(
                        staged_id.clone(),
                        found.get(mod_id).cloned().unwrap_or_default(),
                    );
                }
            }
            Err(e) => log::warn!("mod version tags failed for {domain}: {e}"),
        }
    }
    if !tag_updates.is_empty() {
        match mods::cache_supported_game_versions(&state.paths, game_id, &tag_updates) {
            Ok(()) => {
                for staged in &mut order.mods {
                    if let Some(versions) = tag_updates.get(&staged.id) {
                        staged.supported_game_versions = Some(versions.clone());
                        staged.version_support_generation =
                            mods::game_version::VERSION_SUPPORT_GENERATION;
                    }
                }
            }
            Err(e) => log::warn!("could not cache mod version tags: {e}"),
        }
    }

    let mut collection_updates: HashMap<String, Vec<String>> = HashMap::new();
    for collection in collections.iter() {
        if collection.source != CollectionSource::Nexus || collection.game_versions.is_some() {
            continue;
        }
        let Some(slug) = collection.slug.as_deref().filter(|slug| !slug.is_empty()) else {
            continue;
        };
        match client
            .collection_game_versions(slug, collection.revision, adult)
            .await
        {
            Ok(versions) => {
                collection_updates.insert(collection.id.clone(), versions);
            }
            Err(e) => log::warn!("collection game versions failed for {slug}: {e}"),
        }
    }
    if collection_updates.is_empty() {
        return;
    }
    match mods::cache_collection_game_versions(&state.paths, game_id, &collection_updates) {
        Ok(()) => {
            for collection in collections.iter_mut() {
                if let Some(versions) = collection_updates.get(&collection.id) {
                    collection.game_versions = Some(versions.clone());
                }
            }
        }
        Err(e) => log::warn!("could not cache collection game versions: {e}"),
    }
}

#[tauri::command]
pub async fn nexus_mod_requirements(
    state: State<'_, AppState>,
    game_id: String,
    domain: String,
    mod_id: u64,
    name: String,
) -> Result<nexus::MissingRequirements, String> {
    let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    let staged = staged_nexus_mods(&order.mods);
    let existing = mods::find_nexus_mod(&order, &domain, mod_id);
    let seed = RequirementSeed {
        staged_id: existing.map(|m| m.id.clone()).unwrap_or_default(),
        name: existing
            .map(|m| m.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or(name),
        domain,
        mod_id,
    };
    Ok(collect_requirement_seeds(&state, vec![seed], staged).await)
}

#[tauri::command]
pub fn link_staged_requirements(
    state: State<'_, AppState>,
    game_id: String,
    requirement_id: String,
    required_by: Vec<String>,
) -> Result<(), String> {
    for parent in required_by {
        if parent.is_empty() || parent == requirement_id {
            continue;
        }
        mods::merge_mod_meta(
            &state.paths,
            &game_id,
            &parent,
            &[],
            &[requirement_id.clone()],
            None,
            None,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_game(state: State<'_, AppState>, domain: String) -> Result<GameInfo, String> {
    state.get_game_cached(&domain).await
}

#[tauri::command]
pub async fn get_mod(
    state: State<'_, AppState>,
    domain: String,
    mod_id: u64,
) -> Result<ModDetail, String> {
    let client = state.client()?;
    let mut detail = client
        .get_mod(&domain, mod_id)
        .await
        .map_err(|e| e.to_string())?;
    if detail.category.is_none() {
        if let Some(category_id) = detail.category_id {
            if let Ok(game) = state.get_game_cached(&domain).await {
                detail.category = game
                    .categories
                    .iter()
                    .find(|c| c.category_id == category_id)
                    .map(|c| c.name.clone());
            }
        }
    }
    match client.mod_page_gallery(&domain, mod_id).await {
        Ok(images) => detail.image_urls = images,
        Err(e) => log::warn!("nexus mod gallery unavailable for {domain}/{mod_id}: {e}"),
    }
    if detail.image_urls.is_empty() {
        if let Some(url) = detail
            .picture_url
            .clone()
            .filter(|url| url.starts_with("http://") || url.starts_with("https://"))
        {
            detail
                .image_urls
                .push(ModGalleryImage { url, caption: None });
        }
    }
    Ok(detail)
}

#[tauri::command]
pub async fn mod_files(
    state: State<'_, AppState>,
    domain: String,
    mod_id: u64,
) -> Result<Vec<ModFileInfo>, String> {
    let client = state.client()?;
    client
        .list_mod_files(&domain, mod_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn search_collections(
    state: State<'_, AppState>,
    domain: String,
    query: String,
    sort: Option<String>,
    category: Option<String>,
    tags_include: Option<Vec<String>>,
    tags_exclude: Option<Vec<String>>,
    game_version: Option<String>,
    offset: Option<u32>,
    count: Option<u32>,
) -> Result<nexus::CollectionSearchPage, String> {
    let adult = state
        .config
        .lock()
        .map_err(|e| e.to_string())?
        .adult_content;
    let client = state.client()?;
    let opts = nexus::BrowseSearchOpts {
        sort: sort.unwrap_or_else(|| "endorsements".into()),
        category,
        tags_include: tags_include.unwrap_or_default(),
        tags_exclude: tags_exclude.unwrap_or_default(),
        game_version,
        offset: offset.unwrap_or(0),
        count: count.unwrap_or(0),
    };
    client
        .search_collections(&domain, &query, adult, &opts)
        .await
        .map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectionCatalogPage {
    pub items: Vec<nexus::CollectionHit>,
    pub total_count: u32,
    pub next_offset: u32,
    pub has_more: bool,
    pub nexus_available: bool,
    pub thunderstore_available: bool,
}

fn interleave_collection_hits(
    sources: Vec<Vec<nexus::CollectionHit>>,
) -> Vec<nexus::CollectionHit> {
    let mut iters: Vec<_> = sources
        .into_iter()
        .map(|v| v.into_iter().peekable())
        .collect();
    let mut out = Vec::new();
    loop {
        let mut progressed = false;
        for it in iters.iter_mut() {
            if let Some(item) = it.next() {
                out.push(item);
                progressed = true;
            }
        }
        if !progressed {
            break;
        }
    }
    out
}

#[tauri::command]
pub async fn search_collection_catalog(
    state: State<'_, AppState>,
    game_id: String,
    query: String,
    source_filter: Option<String>,
    sort: Option<String>,
    category: Option<String>,
    tags_include: Option<Vec<String>>,
    tags_exclude: Option<Vec<String>>,
    game_version: Option<String>,
    offset: Option<u32>,
    count: Option<u32>,
) -> Result<CollectionCatalogPage, String> {
    let (nexus_domain, ts_community, adult) = {
        let cfg = state.config.lock().map_err(|e| e.to_string())?;
        let game = cfg
            .managed_games
            .iter()
            .find(|g| g.id == game_id)
            .ok_or_else(|| "Managed game not found".to_string())?;
        (
            if game.nexus_domain.is_empty() {
                None
            } else {
                Some(game.nexus_domain.clone())
            },
            game.thunderstore_community.clone(),
            cfg.adult_content,
        )
    };
    let filter = source_filter.unwrap_or_else(|| "all".into()).to_lowercase();
    let want_nexus = (filter == "all" || filter == "nexus") && nexus_domain.is_some();
    let want_ts = (filter == "all" || filter == "thunderstore") && ts_community.is_some();
    if !want_nexus && !want_ts {
        return Err(catalog_source_unavailable_error(
            &filter,
            nexus_domain.is_some(),
            ts_community.is_some(),
            false,
            true,
        ));
    }

    let offset = offset.unwrap_or(0) as usize;
    let count = {
        let c = count.unwrap_or(25) as usize;
        if c == 0 {
            25
        } else {
            c
        }
    };

    let mut nexus_hits = Vec::new();
    let mut nexus_total = 0u32;
    let mut nexus_available = false;
    if want_nexus {
        if let Some(domain) = &nexus_domain {
            match state.client() {
                Ok(client) => {
                    nexus_available = true;
                    let opts = nexus::BrowseSearchOpts {
                        sort: sort.unwrap_or_else(|| "endorsements".into()),
                        category,
                        tags_include: tags_include.unwrap_or_default(),
                        tags_exclude: tags_exclude.unwrap_or_default(),
                        game_version,
                        offset: offset as u32,
                        count: count as u32,
                    };
                    match client
                        .search_collections(domain, &query, adult, &opts)
                        .await
                    {
                        Ok(page) => {
                            nexus_total = page.total_count as u32;
                            nexus_hits = page.items;
                        }
                        Err(e) => log::warn!("Nexus collection search failed: {e}"),
                    }
                }
                Err(e) => log::warn!("Nexus client unavailable for collections: {e}"),
            }
        }
    }

    let mut ts_hits = Vec::new();
    let mut ts_total = 0u32;
    let mut thunderstore_available = false;
    if want_ts {
        if let Some(community) = &ts_community {
            thunderstore_available = true;
            let packages = load_ts_packages_cached(&state, community).await?;
            let (page, total) =
                filter_ts_modpacks(community, &packages, &query, adult, offset, count);
            ts_total = total as u32;
            ts_hits = page.into_iter().map(collection_hit_from_ts).collect();
        }
    }

    let items = interleave_collection_hits(vec![nexus_hits, ts_hits]);
    let total_count = nexus_total.saturating_add(ts_total);
    let has_more = (offset + count) < nexus_total as usize || (offset + count) < ts_total as usize;

    Ok(CollectionCatalogPage {
        items,
        total_count,
        next_offset: (offset + count) as u32,
        has_more,
        nexus_available,
        thunderstore_available,
    })
}

#[tauri::command]
pub async fn browse_meta(
    state: State<'_, AppState>,
    domain: String,
) -> Result<nexus::BrowseMeta, String> {
    let adult = state
        .config
        .lock()
        .map_err(|e| e.to_string())?
        .adult_content;
    let client = state.client()?;
    client
        .browse_meta(&domain, adult)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_collection(
    state: State<'_, AppState>,
    slug: String,
    domain: Option<String>,
) -> Result<CollectionDetail, String> {
    let adult = state
        .config
        .lock()
        .map_err(|e| e.to_string())?
        .adult_content;
    let client = state.client()?;
    client
        .get_collection(&slug, domain.as_deref(), adult)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn collection_files(
    state: State<'_, AppState>,
    slug: String,
    revision: Option<i64>,
) -> Result<Vec<CollectionModFile>, String> {
    let adult = state
        .config
        .lock()
        .map_err(|e| e.to_string())?
        .adult_content;
    let client = state.client()?;
    client
        .collection_mod_files(&slug, revision, adult)
        .await
        .map_err(|e| e.to_string())
}

async fn download_and_stage_inner(
    app: Option<&tauri::AppHandle>,
    state: &AppState,
    game_id: &str,
    domain: &str,
    mod_id: u64,
    file_id: u64,
    label: &str,
    version: Option<String>,
    nxm_key: Option<String>,
    nxm_expires: Option<u64>,
    existing_dl_id: Option<String>,
    batch_id: Option<String>,
    replace_staged_id: Option<String>,
    chosen_file_name: Option<String>,
) -> Result<StagedMod, String> {
    let client = state.client()?;
    let is_premium = state.ensure_premium_status().await;

    let (dl_id, is_new) = match existing_dl_id {
        Some(id) => (id, false),
        None => (uuid::Uuid::new_v4().to_string(), true),
    };
    if is_new {
        push_download(
            state,
            DownloadItem::new(
                dl_id.clone(),
                label.to_string(),
                "downloading",
                batch_id.clone(),
                nexus_page(game_id, domain, mod_id),
            ),
        );
    } else {
        update_download(state, &dl_id, "downloading", None);
        if let Ok(mut q) = state.downloads.lock() {
            if let Some(item) = q.iter_mut().find(|d| d.id == dl_id) {
                if item.batch_id.is_none() {
                    item.batch_id = batch_id.clone();
                }
            }
        }
    }

    let (cancel, pause) = ensure_download_job(state, &dl_id, batch_id.clone());
    if cancel.load(Ordering::SeqCst) {
        mark_cancelled(state, &dl_id);
        clear_download_job(state, &dl_id);
        return Err("Cancelled".into());
    }
    pause.store(false, Ordering::SeqCst);

    let dest = state
        .paths
        .downloads_dir()
        .join(format!("{domain}_{mod_id}_{file_id}.bin"));

    set_download_resume(
        state,
        &dl_id,
        dest.clone(),
        DownloadResumeSource::Api {
            game_id: game_id.to_string(),
            domain: domain.to_string(),
            mod_id,
            file_id,
            label: label.to_string(),
            version: version.clone(),
            nxm_key: nxm_key.clone(),
            nxm_expires,
            replace_staged_id: replace_staged_id.clone(),
            file_name: chosen_file_name.clone(),
        },
    );

    if let Some(early) = mods::pick_staged_file_name(label, chosen_file_name.as_deref(), None) {
        if assign_download_file_name(state, &dl_id, Some(&early)) {
            if let Some(handle) = app {
                let _ = handle.emit("downloads-changed", ());
            }
        }
    }

    let start_offset = file_offset(&dest);
    let control = TransferControl {
        cancel: cancel.clone(),
        pause: pause.clone(),
    };
    let on_progress = progress_callback(app, dl_id.clone());

    // Held for the transfer only; extraction is disk-bound and should not
    // occupy a network slot.
    let _slot = state
        .download_slots
        .acquire()
        .await
        .map_err(|e| e.to_string())?;

    let download_result = client
        .download_file(
            domain,
            mod_id,
            file_id,
            &dest,
            is_premium,
            nxm_key.as_deref(),
            nxm_expires,
            start_offset,
            Some(&control),
            on_progress.as_ref(),
        )
        .await;

    let cdn_name = match download_result {
        Ok(name) => name,
        Err(e) => {
            let msg = e.to_string();
            if is_pause_err(&msg) || pause.load(Ordering::SeqCst) {
                mark_paused(app, state, &dl_id);
                return Err("Paused".into());
            }
            if is_cancel_err(&msg) || cancel.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&dest);
                mark_cancelled(state, &dl_id);
                clear_download_job(state, &dl_id);
                return Err("Cancelled".into());
            }
            // The partial is the resume base for the next attempt, and the
            // path is derived from the file id, so keeping a broken one makes
            // every retry fail the same way. Start clean instead.
            let _ = std::fs::remove_file(&dest);
            update_download(state, &dl_id, "failed", Some(msg.clone()));
            clear_download_job(state, &dl_id);
            return Err(msg);
        }
    };

    if cancel.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(&dest);
        mark_cancelled(state, &dl_id);
        clear_download_job(state, &dl_id);
        return Err("Cancelled".into());
    }
    if pause.load(Ordering::SeqCst) {
        mark_paused(app, state, &dl_id);
        return Err("Paused".into());
    }

    drop(_slot);

    // Downloading to a stable `.bin` name keeps pause/resume working, but
    // staging needs the real extension: a Sims `.package` is recognized by name,
    // not by header. Rename only now that the transfer is verified complete.
    let dest = match archive_extension(cdn_name.as_deref()) {
        Some(ext) => {
            let renamed = state
                .paths
                .downloads_dir()
                .join(format!("{domain}_{mod_id}_{file_id}.{ext}"));
            match std::fs::rename(&dest, &renamed) {
                Ok(()) => renamed,
                Err(e) => {
                    log::warn!("could not rename {} to {ext}: {e}", dest.display());
                    dest
                }
            }
        }
        None => dest,
    };

    update_download(state, &dl_id, "extracting", None);
    if let Some(handle) = app {
        let _ = handle.emit(
            "download-progress",
            serde_json::json!({
                "id": dl_id,
                "bytes_downloaded": state.downloads.lock().ok()
                    .and_then(|q| q.iter().find(|d| d.id == dl_id).map(|d| d.bytes_downloaded))
                    .unwrap_or(0),
                "bytes_total": state.downloads.lock().ok()
                    .and_then(|q| q.iter().find(|d| d.id == dl_id).and_then(|d| d.bytes_total)),
                "speed_bps": 0,
                "status": "extracting",
            }),
        );
    }

    let stored_file_name =
        mods::pick_staged_file_name(label, chosen_file_name.as_deref(), cdn_name.as_deref());
    if assign_download_file_name(state, &dl_id, stored_file_name.as_deref()) {
        if let Some(handle) = app {
            let _ = handle.emit("downloads-changed", ());
        }
    }

    match if let Some(replace_id) = replace_staged_id.as_deref() {
        mods::stage_mod_replacing(
            &state.paths,
            game_id,
            replace_id,
            label,
            domain,
            mod_id,
            file_id,
            version,
            stored_file_name.clone(),
            &dest,
        )
    } else {
        mods::stage_mod(
            &state.paths,
            game_id,
            label,
            domain,
            mod_id,
            file_id,
            version,
            stored_file_name,
            &dest,
        )
    } {
        Ok(staged) => {
            if cancel.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&dest);
                mark_cancelled(state, &dl_id);
                clear_download_job(state, &dl_id);
                return Err("Cancelled".into());
            }
            update_download(state, &dl_id, "staged", None);
            let _ = std::fs::remove_file(&dest);
            finish_download_success(state, &dl_id);
            Ok(staged)
        }
        Err(e) => {
            let msg = e.to_string();
            // An archive that would not extract is worthless; never resume onto it.
            let _ = std::fs::remove_file(&dest);
            if cancel.load(Ordering::SeqCst) {
                mark_cancelled(state, &dl_id);
            } else {
                update_download(state, &dl_id, "failed", Some(msg.clone()));
            }
            clear_download_job(state, &dl_id);
            Err(if cancel.load(Ordering::SeqCst) {
                "Cancelled".into()
            } else {
                msg
            })
        }
    }
}

/// Download via Nexus API / NXM key, then install as a FromSoft legacy loader.
async fn download_and_install_loader_inner(
    app: Option<&tauri::AppHandle>,
    state: &AppState,
    game_id: &str,
    domain: &str,
    mod_id: u64,
    file_id: u64,
    label: &str,
    nxm_key: Option<String>,
    nxm_expires: Option<u64>,
    existing_dl_id: Option<String>,
    batch_id: Option<String>,
    plugin_id: &str,
    install_path: &Path,
) -> Result<(), String> {
    let client = state.client()?;
    let is_premium = state.ensure_premium_status().await;

    let (dl_id, is_new) = match existing_dl_id {
        Some(id) => (id, false),
        None => (uuid::Uuid::new_v4().to_string(), true),
    };
    if is_new {
        push_download(
            state,
            DownloadItem::new(
                dl_id.clone(),
                label.to_string(),
                "downloading",
                batch_id.clone(),
                None,
            ),
        );
    } else {
        update_download(state, &dl_id, "downloading", None);
        if let Ok(mut q) = state.downloads.lock() {
            if let Some(item) = q.iter_mut().find(|d| d.id == dl_id) {
                if item.batch_id.is_none() {
                    item.batch_id = batch_id.clone();
                }
            }
        }
    }

    let (cancel, pause) = ensure_download_job(state, &dl_id, batch_id.clone());
    if cancel.load(Ordering::SeqCst) {
        mark_cancelled(state, &dl_id);
        clear_download_job(state, &dl_id);
        return Err("Cancelled".into());
    }
    pause.store(false, Ordering::SeqCst);

    let dest = state
        .paths
        .downloads_dir()
        .join(format!("{domain}_{mod_id}_{file_id}.bin"));

    set_download_resume(
        state,
        &dl_id,
        dest.clone(),
        DownloadResumeSource::Api {
            game_id: game_id.to_string(),
            domain: domain.to_string(),
            mod_id,
            file_id,
            label: label.to_string(),
            version: None,
            nxm_key: nxm_key.clone(),
            nxm_expires,
            replace_staged_id: None,
            file_name: None,
        },
    );

    let start_offset = file_offset(&dest);
    let control = TransferControl {
        cancel: cancel.clone(),
        pause: pause.clone(),
    };
    let on_progress = progress_callback(app, dl_id.clone());

    let _slot = state
        .download_slots
        .acquire()
        .await
        .map_err(|e| e.to_string())?;

    let download_result = client
        .download_file(
            domain,
            mod_id,
            file_id,
            &dest,
            is_premium,
            nxm_key.as_deref(),
            nxm_expires,
            start_offset,
            Some(&control),
            on_progress.as_ref(),
        )
        .await;

    let cdn_name = match download_result {
        Ok(name) => name,
        Err(e) => {
            let msg = e.to_string();
            if is_pause_err(&msg) || pause.load(Ordering::SeqCst) {
                mark_paused(app, state, &dl_id);
                return Err("Paused".into());
            }
            if is_cancel_err(&msg) || cancel.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&dest);
                mark_cancelled(state, &dl_id);
                clear_download_job(state, &dl_id);
                return Err("Cancelled".into());
            }
            let _ = std::fs::remove_file(&dest);
            update_download(state, &dl_id, "failed", Some(msg.clone()));
            clear_download_job(state, &dl_id);
            return Err(msg);
        }
    };

    if cancel.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(&dest);
        mark_cancelled(state, &dl_id);
        clear_download_job(state, &dl_id);
        return Err("Cancelled".into());
    }
    if pause.load(Ordering::SeqCst) {
        mark_paused(app, state, &dl_id);
        return Err("Paused".into());
    }

    drop(_slot);

    let dest = match archive_extension(cdn_name.as_deref()) {
        Some(ext) => {
            let renamed = state
                .paths
                .downloads_dir()
                .join(format!("{domain}_{mod_id}_{file_id}.{ext}"));
            match std::fs::rename(&dest, &renamed) {
                Ok(()) => renamed,
                Err(e) => {
                    log::warn!("could not rename {} to {ext}: {e}", dest.display());
                    dest
                }
            }
        }
        None => dest,
    };

    update_download(state, &dl_id, "extracting", None);
    if let Some(handle) = app {
        let _ = handle.emit(
            "download-progress",
            serde_json::json!({
                "id": dl_id,
                "bytes_downloaded": state.downloads.lock().ok()
                    .and_then(|q| q.iter().find(|d| d.id == dl_id).map(|d| d.bytes_downloaded))
                    .unwrap_or(0),
                "bytes_total": state.downloads.lock().ok()
                    .and_then(|q| q.iter().find(|d| d.id == dl_id).and_then(|d| d.bytes_total)),
                "speed_bps": 0,
                "status": "extracting",
            }),
        );
    }

    match games::install_legacy_from_archive(&state.paths, plugin_id, install_path, &dest) {
        Ok(()) => {
            if cancel.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&dest);
                mark_cancelled(state, &dl_id);
                clear_download_job(state, &dl_id);
                return Err("Cancelled".into());
            }
            update_download(state, &dl_id, "staged", None);
            let _ = std::fs::remove_file(&dest);
            finish_download_success(state, &dl_id);
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = std::fs::remove_file(&dest);
            if cancel.load(Ordering::SeqCst) {
                mark_cancelled(state, &dl_id);
            } else {
                update_download(state, &dl_id, "failed", Some(msg.clone()));
            }
            clear_download_job(state, &dl_id);
            Err(if cancel.load(Ordering::SeqCst) {
                "Cancelled".into()
            } else {
                msg
            })
        }
    }
}

#[tauri::command]
pub async fn download_mod(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    game_id: String,
    domain: String,
    mod_id: u64,
    file_id: u64,
    name: String,
    version: Option<String>,
    replace_staged_id: Option<String>,
    file_name: Option<String>,
) -> Result<StagedMod, String> {
    if !state.ensure_premium_status().await {
        return Err(
            "NEEDS_NXM: Free account — open Download Assist to use Mod Manager Download \
             (or enable autoclick), or import a local archive."
                .into(),
        );
    }
    download_and_stage_inner(
        Some(&app),
        &*state,
        &game_id,
        &domain,
        mod_id,
        file_id,
        &name,
        version,
        None,
        None,
        None,
        None,
        replace_staged_id,
        file_name,
    )
    .await
}

#[tauri::command]
pub async fn install_collection(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    game_id: String,
    slug: String,
    revision: Option<i64>,
    include_optional: bool,
    name: Option<String>,
) -> Result<usize, String> {
    if !state.ensure_premium_status().await {
        return Err("Collection batch install requires Nexus Premium. \
             Free accounts can install mods one-by-one via Browse → Download Assist."
            .into());
    }
    let adult = state
        .config
        .lock()
        .map_err(|e| e.to_string())?
        .adult_content;
    let client = state.client()?;
    let files = client
        .collection_mod_files(&slug, revision, adult)
        .await
        .map_err(|e| e.to_string())?;

    let batch_id = uuid::Uuid::new_v4().to_string();
    let before = snapshot_mod_ids(&state.paths, &game_id)?;
    let mut installed = 0usize;
    let mut member_ids = Vec::new();
    for file in files {
        if file.optional && !include_optional {
            continue;
        }
        // If the whole batch was cancelled, stop installing further mods.
        let batch_cancelled = state
            .download_jobs
            .lock()
            .ok()
            .map(|jobs| {
                jobs.values().any(|j| {
                    j.batch_id.as_deref() == Some(batch_id.as_str())
                        && j.cancel.load(Ordering::SeqCst)
                })
            })
            .unwrap_or(false);
        if batch_cancelled {
            break;
        }
        match download_and_stage_inner(
            Some(&app),
            &*state,
            &game_id,
            &file.domain_name,
            file.mod_id,
            file.file_id,
            &file.mod_name,
            file.version,
            None,
            None,
            None,
            Some(batch_id.clone()),
            None,
            Some(file.file_name),
        )
        .await
        {
            Ok(staged) => {
                member_ids.push(staged.id);
                installed += 1;
            }
            Err(e) if is_cancel_err(&e) || is_pause_err(&e) => break,
            Err(e) => return Err(e),
        }
    }
    if !member_ids.is_empty() {
        let new_ids: Vec<String> = member_ids
            .iter()
            .filter(|id| !before.contains(*id))
            .cloned()
            .collect();
        let collection = InstalledCollection {
            id: mods::nexus_collection_id(&slug),
            source: CollectionSource::Nexus,
            kind: CollectionKind::Collection,
            name: name
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| slug.clone()),
            slug: Some(slug),
            namespace: None,
            package_name: None,
            community: None,
            revision,
            version: None,
            profile_code: None,
            mod_ids: member_ids,
            installed_at: String::new(),
            game_versions: None,
        };
        mods::record_installed_collection(&state.paths, &game_id, collection, &new_ids)
            .map_err(|e| e.to_string())?;
    }
    Ok(installed)
}

/// Queues a keyed free-account (or premium) download and closes Assist immediately.
/// Staging continues in the background; listen for `download-finished` / `download-failed`.
#[tauri::command]
pub async fn handle_nxm(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    url: String,
) -> Result<DownloadItem, String> {
    let url = url.trim().to_string();
    if let Ok(mut last) = state.last_nxm_handled.lock() {
        if let Some((ref prev, at)) = *last {
            if prev == &url && at.elapsed() < Duration::from_secs(1) {
                return Err("DUPLICATE_NXM".into());
            }
        }
        *last = Some((url.clone(), Instant::now()));
    }

    let link = nexus::parse_nxm(&url).map_err(|e| e.to_string())?;
    let assist = crate::assist::peek_assist_context(&state);
    let batch_id = assist.as_ref().and_then(|a| a.batch_id.clone());
    let replace_staged_id = assist.as_ref().and_then(|a| a.replace_staged_id.clone());
    let staged_version = assist.as_ref().and_then(|a| a.version.clone());
    let staged_file_name = assist.as_ref().and_then(|a| a.file_name.clone());
    let loader_purpose = match assist.as_ref().map(|a| &a.purpose) {
        Some(crate::assist::AssistPurpose::FromsoftLegacyLoader {
            plugin_id,
            install_path,
        }) => Some((plugin_id.clone(), install_path.clone())),
        _ => None,
    };
    let game = {
        let cfg = state.config.lock().map_err(|e| e.to_string())?;
        cfg.managed_games
            .iter()
            .find(|g| g.nexus_domain == link.domain)
            .cloned()
            .or_else(|| {
                assist.as_ref().and_then(|a| {
                    cfg.managed_games
                        .iter()
                        .find(|g| g.id == a.game_id)
                        .cloned()
                })
            })
            .or_else(|| {
                cfg.last_active_game_id
                    .as_ref()
                    .and_then(|id| cfg.managed_games.iter().find(|g| &g.id == id).cloned())
            })
            .ok_or_else(|| {
                format!(
                    "No managed game for Nexus domain '{}'. Manage a matching game first.",
                    link.domain
                )
            })?
    };

    let name = assist
        .as_ref()
        .map(|a| a.label.clone())
        .unwrap_or_else(|| format!("{} file {}", link.mod_id, link.file_id));

    let is_premium = state.ensure_premium_status().await;
    let has_nxm =
        link.key.as_ref().map(|k| !k.is_empty()).unwrap_or(false) && link.expires.is_some();
    if !is_premium && !has_nxm {
        return Err(
            "nxm:// link missing download key/expires. Sign in on the Assist window and \
             click Mod Manager Download again."
                .into(),
        );
    }

    // Close Assist as soon as the keyed download can start; do not wait for staging.
    let _ = crate::assist::close_assist_intentionally(&app, &state);

    let dl_id = uuid::Uuid::new_v4().to_string();
    let page = if loader_purpose.is_some() {
        None
    } else {
        nexus_page(&game.id, &link.domain, link.mod_id)
    };
    let item = DownloadItem::new(
        dl_id.clone(),
        name.clone(),
        "downloading",
        batch_id.clone(),
        page,
    );
    push_download(&state, item.clone());
    let _ = ensure_download_job(&state, &dl_id, batch_id.clone());
    log::info!(
        "download queued: {name} id={dl_id} domain={domain} mod={mod_id} file={file_id}",
        domain = link.domain,
        mod_id = link.mod_id,
        file_id = link.file_id
    );

    let _ = app.emit(
        "assist-download-started",
        serde_json::json!({
            "id": dl_id,
            "label": name,
            "mod_id": link.mod_id,
            "file_id": link.file_id,
            "domain": link.domain,
            "batch_id": batch_id,
        }),
    );

    let app_bg = app.clone();
    let game_id = game.id.clone();
    let domain = link.domain.clone();
    let mod_id = link.mod_id;
    let file_id = link.file_id;
    let key = link.key.clone();
    let expires = link.expires;
    let label = name.clone();
    let queued_id = dl_id.clone();
    let batch_id_bg = batch_id.clone();
    let replace_staged_id_bg = replace_staged_id;
    let staged_version_bg = staged_version;
    let file_name_bg = staged_file_name;

    tauri::async_runtime::spawn(async move {
        let state = app_bg.state::<AppState>();
        if let Some((plugin_id, install_path)) = loader_purpose {
            let result = download_and_install_loader_inner(
                Some(&app_bg),
                &*state,
                &game_id,
                &domain,
                mod_id,
                file_id,
                &label,
                key,
                expires,
                Some(queued_id.clone()),
                batch_id_bg,
                &plugin_id,
                Path::new(&install_path),
            )
            .await;
            match result {
                Ok(()) => {
                    log::info!("nxm loader install finished: {label}");
                    let _ = app_bg.emit(
                        "fromsoft-loader-installed",
                        serde_json::json!({
                            "id": queued_id,
                            "label": label,
                            "game_id": game_id,
                            "plugin_id": plugin_id,
                        }),
                    );
                    let _ = app_bg.emit(
                        "download-finished",
                        serde_json::json!({
                            "id": queued_id,
                            "label": label,
                            "game_id": game_id,
                            "mod_id": "",
                        }),
                    );
                }
                Err(e) => {
                    if is_pause_err(&e) {
                        log::info!("nxm loader download paused: {label}");
                    } else if is_cancel_err(&e) {
                        log::info!("nxm loader download cancelled: {label}");
                        let _ = app_bg.emit(
                            "download-cancelled",
                            serde_json::json!({
                                "id": queued_id,
                                "label": label,
                            }),
                        );
                    } else {
                        log::error!("nxm loader download failed ({label}): {e}");
                        let _ = app_bg.emit(
                            "download-failed",
                            serde_json::json!({
                                "id": queued_id,
                                "label": label,
                                "error": e,
                            }),
                        );
                    }
                }
            }
            return;
        }

        let result = download_and_stage_inner(
            Some(&app_bg),
            &*state,
            &game_id,
            &domain,
            mod_id,
            file_id,
            &label,
            staged_version_bg,
            key,
            expires,
            Some(queued_id.clone()),
            batch_id_bg,
            replace_staged_id_bg,
            file_name_bg,
        )
        .await;
        match result {
            Ok(staged) => {
                log::info!("download finished: {label}");
                let _ = app_bg.emit(
                    "download-finished",
                    serde_json::json!({
                        "id": queued_id,
                        "label": label,
                        "game_id": game_id,
                        "mod_id": staged.id,
                    }),
                );
            }
            Err(e) => {
                if is_pause_err(&e) {
                    log::info!("download paused: {label}");
                } else if is_cancel_err(&e) {
                    log::info!("download cancelled: {label}");
                    let _ = app_bg.emit(
                        "download-cancelled",
                        serde_json::json!({
                            "id": queued_id,
                            "label": label,
                        }),
                    );
                } else {
                    log::error!("download failed ({label}): {e}");
                    let _ = app_bg.emit(
                        "download-failed",
                        serde_json::json!({
                            "id": queued_id,
                            "label": label,
                            "error": e,
                        }),
                    );
                }
            }
        }
    });

    Ok(item)
}

#[tauri::command]
pub fn import_mod_archive(
    state: State<'_, AppState>,
    game_id: String,
    path: String,
    name: Option<String>,
    domain: Option<String>,
    mod_id: Option<u64>,
    file_id: Option<u64>,
) -> Result<StagedMod, String> {
    let archive = PathBuf::from(&path);
    if !archive.is_file() {
        return Err(format!("Archive not found: {path}"));
    }
    let label = name.unwrap_or_else(|| {
        archive
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Imported mod".into())
    });
    let domain = domain.unwrap_or_else(|| {
        state
            .config
            .lock()
            .ok()
            .and_then(|c| {
                c.managed_games
                    .iter()
                    .find(|g| g.id == game_id)
                    .map(|g| g.nexus_domain.clone())
            })
            .unwrap_or_else(|| "unknown".into())
    });
    let mod_id = mod_id.unwrap_or(0);
    let file_id = file_id.unwrap_or_else(|| {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(1)
    });

    let imported_name = archive.file_name().map(|s| s.to_string_lossy().to_string());
    mods::stage_mod(
        &state.paths,
        &game_id,
        &label,
        &domain,
        mod_id,
        file_id,
        None,
        mods::pick_staged_file_name(&label, None, imported_name.as_deref()),
        &archive,
    )
    .map_err(|e| e.to_string())
}

/// Intercept a Nexus CDN URL from Download Assist, fetch with webview cookies, stage in background.
pub async fn start_assist_cdn_download(
    app: &tauri::AppHandle,
    state: &AppState,
    cdn_url: &str,
) -> Result<DownloadItem, String> {
    let cdn_url = cdn_url.trim();
    if let Ok(mut last) = state.last_cdn_handled.lock() {
        if let Some((ref prev, at)) = *last {
            if prev == cdn_url && at.elapsed() < Duration::from_secs(1) {
                return Err("DUPLICATE_CDN".into());
            }
        }
        *last = Some((cdn_url.to_string(), Instant::now()));
    }

    let ctx = crate::assist::peek_assist_context(state)
        .ok_or_else(|| "No active Download Assist context".to_string())?;
    let loader_purpose = match &ctx.purpose {
        crate::assist::AssistPurpose::FromsoftLegacyLoader {
            plugin_id,
            install_path,
        } => Some((plugin_id.clone(), install_path.clone())),
        crate::assist::AssistPurpose::StageMod => None,
        crate::assist::AssistPurpose::WebsiteLogin => {
            return Err("Nexus website login is not a download".into());
        }
    };

    let parsed = url::Url::parse(cdn_url).map_err(|e| e.to_string())?;
    if !crate::assist::is_nexus_cdn_download(&parsed) {
        return Err(format!("Not a Nexus CDN download URL: {cdn_url}"));
    }

    let cookie_header = if let Some(w) = app.get_webview(crate::assist::ASSIST_LABEL) {
        match w.cookies_for_url(parsed.clone()) {
            Ok(cookies) => {
                log::info!(
                    "cdn download: {} session cookies from assist webview",
                    cookies.len()
                );
                cookies
                    .iter()
                    .map(|c| format!("{}={}", c.name(), c.value()))
                    .collect::<Vec<_>>()
                    .join("; ")
            }
            Err(e) => {
                log::warn!("cdn download: failed to read assist cookies: {e}");
                String::new()
            }
        }
    } else {
        log::warn!("cdn download: assist webview gone before cookie read");
        String::new()
    };

    let batch_id = ctx.batch_id.clone();
    let dl_id = uuid::Uuid::new_v4().to_string();
    let page = if loader_purpose.is_some() {
        None
    } else {
        nexus_page(&ctx.game_id, &ctx.domain, ctx.mod_id)
    };
    let item = DownloadItem::new(
        dl_id.clone(),
        ctx.label.clone(),
        "downloading",
        batch_id.clone(),
        page,
    );
    push_download(state, item.clone());
    let _ = ensure_download_job(state, &dl_id, batch_id.clone());
    log::info!(
        "cdn download queued: {} id={dl_id} domain={} mod={} file={}",
        ctx.label,
        ctx.domain,
        ctx.mod_id,
        ctx.file_id
    );

    let _ = app.emit(
        "assist-download-started",
        serde_json::json!({
            "id": dl_id,
            "label": ctx.label,
            "mod_id": ctx.mod_id,
            "file_id": ctx.file_id,
            "domain": ctx.domain,
            "source": "cdn",
            "batch_id": batch_id,
        }),
    );

    let _ = crate::assist::close_assist_intentionally(app, state);

    let app_bg = app.clone();
    let game_id = ctx.game_id.clone();
    let label = ctx.label.clone();
    let domain = ctx.domain.clone();
    let mod_id = ctx.mod_id;
    let file_id = ctx.file_id;
    let replace_staged_id = ctx.replace_staged_id.clone();
    let staged_version = ctx.version.clone();
    let staged_file_name = ctx.file_name.clone();
    let queued_id = dl_id.clone();
    let cdn_url = cdn_url.to_string();
    let cookie_header = cookie_header;
    let batch_id_bg = batch_id.clone();

    tauri::async_runtime::spawn(async move {
        let state = app_bg.state::<AppState>();
        if let Some((plugin_id, install_path)) = loader_purpose {
            let result = fetch_cdn_and_install_loader(
                &app_bg,
                &*state,
                &cdn_url,
                &cookie_header,
                &game_id,
                &label,
                &domain,
                mod_id,
                file_id,
                &queued_id,
                batch_id_bg,
                &plugin_id,
                Path::new(&install_path),
            )
            .await;
            match result {
                Ok(()) => {
                    log::info!("cdn loader install finished: {label}");
                    let _ = app_bg.emit(
                        "fromsoft-loader-installed",
                        serde_json::json!({
                            "id": queued_id,
                            "label": label,
                            "game_id": game_id,
                            "plugin_id": plugin_id,
                        }),
                    );
                    let _ = app_bg.emit(
                        "download-finished",
                        serde_json::json!({
                            "id": queued_id,
                            "label": label,
                            "game_id": game_id,
                            "mod_id": "",
                        }),
                    );
                }
                Err(e) => {
                    if is_pause_err(&e) {
                        log::info!("cdn loader download paused: {label}");
                    } else if is_cancel_err(&e) {
                        log::info!("cdn loader download cancelled: {label}");
                        let _ = app_bg.emit(
                            "download-cancelled",
                            serde_json::json!({
                                "id": queued_id,
                                "label": label,
                            }),
                        );
                    } else {
                        log::error!("cdn loader download failed ({label}): {e}");
                        let _ = app_bg.emit(
                            "download-failed",
                            serde_json::json!({
                                "id": queued_id,
                                "label": label,
                                "error": e,
                            }),
                        );
                    }
                }
            }
            return;
        }

        let result = fetch_and_stage_cdn(
            &app_bg,
            &*state,
            &cdn_url,
            &cookie_header,
            &game_id,
            &label,
            &domain,
            mod_id,
            file_id,
            &queued_id,
            batch_id_bg,
            staged_version,
            replace_staged_id,
            staged_file_name,
        )
        .await;
        match result {
            Ok(staged) => {
                log::info!("cdn download finished: {label}");
                let _ = app_bg.emit(
                    "download-finished",
                    serde_json::json!({
                        "id": queued_id,
                        "label": label,
                        "game_id": game_id,
                        "mod_id": staged.id,
                    }),
                );
            }
            Err(e) => {
                if is_pause_err(&e) {
                    log::info!("cdn download paused: {label}");
                } else if is_cancel_err(&e) {
                    log::info!("cdn download cancelled: {label}");
                    let _ = app_bg.emit(
                        "download-cancelled",
                        serde_json::json!({
                            "id": queued_id,
                            "label": label,
                        }),
                    );
                } else {
                    log::error!("cdn download failed ({label}): {e}");
                    let _ = app_bg.emit(
                        "download-failed",
                        serde_json::json!({
                            "id": queued_id,
                            "label": label,
                            "error": e,
                        }),
                    );
                }
            }
        }
    });

    Ok(item)
}

/// Download a CDN URL into the downloads dir (shared by stage + loader install).
async fn download_cdn_archive(
    app: &tauri::AppHandle,
    state: &AppState,
    cdn_url: &str,
    cookie_header: &str,
    game_id: &str,
    label: &str,
    domain: &str,
    mod_id: u64,
    file_id: u64,
    dl_id: &str,
    batch_id: Option<String>,
    version: Option<String>,
    replace_staged_id: Option<String>,
    chosen_file_name: Option<String>,
) -> Result<(PathBuf, Arc<AtomicBool>), String> {
    let (cancel, pause) = ensure_download_job(state, dl_id, batch_id.clone());
    if cancel.load(Ordering::SeqCst) {
        mark_cancelled(state, dl_id);
        clear_download_job(state, dl_id);
        return Err("Cancelled".into());
    }
    pause.store(false, Ordering::SeqCst);

    let existing_dest = state
        .download_jobs
        .lock()
        .ok()
        .and_then(|jobs| jobs.get(dl_id).and_then(|j| j.dest.clone()));

    let dest = if let Some(path) = existing_dest {
        path
    } else {
        // Prefer a stable path so pause/resume can reuse the same file.
        let filename = sanitize_filename::sanitize(format!(
            "{domain}_{mod_id}_{file_id}_{}.bin",
            &dl_id[..8.min(dl_id.len())]
        ));
        let path = state.paths.downloads_dir().join(filename);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        path
    };

    set_download_resume(
        state,
        dl_id,
        dest.clone(),
        DownloadResumeSource::Cdn {
            url: cdn_url.to_string(),
            cookie_header: cookie_header.to_string(),
            game_id: game_id.to_string(),
            domain: domain.to_string(),
            mod_id,
            file_id,
            label: label.to_string(),
            version: version.clone(),
            replace_staged_id: replace_staged_id.clone(),
            file_name: chosen_file_name.clone(),
        },
    );
    let archive_name = crate::nexus::filename_from_url(cdn_url);
    if let Some(early) =
        mods::pick_staged_file_name(label, chosen_file_name.as_deref(), archive_name.as_deref())
    {
        if assign_download_file_name(state, dl_id, Some(&early)) {
            let _ = app.emit("downloads-changed", ());
        }
    }

    let client = reqwest::Client::builder()
        .user_agent(format!("{APP_NAME}/{APP_VERSION}"))
        .build()
        .map_err(|e| e.to_string())?;

    let mut headers = reqwest::header::HeaderMap::new();
    if !cookie_header.is_empty() {
        headers.insert(
            reqwest::header::COOKIE,
            reqwest::header::HeaderValue::from_str(cookie_header).map_err(|e| e.to_string())?,
        );
    }

    let start_offset = file_offset(&dest);
    let control = TransferControl {
        cancel: cancel.clone(),
        pause: pause.clone(),
    };
    let on_progress = progress_callback(Some(app), dl_id.to_string());

    log::info!(
        "cdn download start offset={start_offset} for {cdn_url} -> {}",
        dest.display()
    );

    // Held for the transfer only; extraction is disk-bound and should not
    // occupy a network slot.
    let slot = state
        .download_slots
        .acquire()
        .await
        .map_err(|e| e.to_string())?;

    let download_result = stream_url_to_file(
        &client,
        cdn_url,
        &dest,
        start_offset,
        if headers.is_empty() {
            None
        } else {
            Some(&headers)
        },
        Some(&control),
        on_progress.as_ref(),
    )
    .await;

    match download_result {
        Ok(()) => {}
        Err(e) => {
            let msg = e.to_string();
            if is_pause_err(&msg) || pause.load(Ordering::SeqCst) {
                mark_paused(Some(app), state, dl_id);
                return Err("Paused".into());
            }
            if is_cancel_err(&msg) || cancel.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&dest);
                mark_cancelled(state, dl_id);
                clear_download_job(state, dl_id);
                return Err("Cancelled".into());
            }
            let fail_msg = if msg.contains("link expired") {
                format!("{msg} Cancel and re-queue via Download Assist to get a fresh link.")
            } else {
                msg.clone()
            };
            // Same reasoning as the API path: a failed transfer must not become
            // the resume base for the next attempt.
            let _ = std::fs::remove_file(&dest);
            update_download(state, dl_id, "failed", Some(fail_msg.clone()));
            clear_download_job(state, dl_id);
            return Err(fail_msg);
        }
    }

    if cancel.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(&dest);
        mark_cancelled(state, dl_id);
        clear_download_job(state, dl_id);
        return Err("Cancelled".into());
    }
    if pause.load(Ordering::SeqCst) {
        mark_paused(Some(app), state, dl_id);
        return Err("Paused".into());
    }

    drop(slot);

    // Same as the API path: give staging the real extension now that the
    // transfer is known complete.
    let dest = match archive_extension(nexus::filename_from_url(cdn_url).as_deref()) {
        Some(ext) => {
            let renamed = dest.with_extension(&ext);
            match std::fs::rename(&dest, &renamed) {
                Ok(()) => renamed,
                Err(e) => {
                    log::warn!("could not rename {} to {ext}: {e}", dest.display());
                    dest
                }
            }
        }
        None => dest,
    };

    log::info!("cdn download saved to {}", dest.display());
    Ok((dest, cancel))
}

fn report_cdn_extracting(app: &tauri::AppHandle, state: &AppState, dl_id: &str, dest: &PathBuf) {
    let (bytes_downloaded, bytes_total) = state
        .downloads
        .lock()
        .ok()
        .and_then(|q| {
            q.iter().find(|d| d.id == dl_id).map(|d| {
                (
                    d.bytes_downloaded,
                    d.bytes_total.or(Some(d.bytes_downloaded)),
                )
            })
        })
        .unwrap_or_else(|| (file_offset(dest), Some(file_offset(dest))));

    update_download(state, dl_id, "extracting", None);
    report_download_progress(app, state, dl_id, bytes_downloaded, bytes_total, 0);
}

async fn fetch_and_stage_cdn(
    app: &tauri::AppHandle,
    state: &AppState,
    cdn_url: &str,
    cookie_header: &str,
    game_id: &str,
    label: &str,
    domain: &str,
    mod_id: u64,
    file_id: u64,
    dl_id: &str,
    batch_id: Option<String>,
    version: Option<String>,
    replace_staged_id: Option<String>,
    chosen_file_name: Option<String>,
) -> Result<StagedMod, String> {
    let archive_name = crate::nexus::filename_from_url(cdn_url);
    let stored_file_name =
        mods::pick_staged_file_name(label, chosen_file_name.as_deref(), archive_name.as_deref());
    let (dest, cancel) = download_cdn_archive(
        app,
        state,
        cdn_url,
        cookie_header,
        game_id,
        label,
        domain,
        mod_id,
        file_id,
        dl_id,
        batch_id,
        version.clone(),
        replace_staged_id.clone(),
        chosen_file_name,
    )
    .await?;

    report_cdn_extracting(app, state, dl_id, &dest);
    match if let Some(replace_id) = replace_staged_id.as_deref() {
        mods::stage_mod_replacing(
            &state.paths,
            game_id,
            replace_id,
            label,
            domain,
            mod_id,
            file_id,
            version.clone(),
            stored_file_name.clone(),
            &dest,
        )
    } else {
        mods::stage_mod(
            &state.paths,
            game_id,
            label,
            domain,
            mod_id,
            file_id,
            version,
            stored_file_name,
            &dest,
        )
    } {
        Ok(staged) => {
            if cancel.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&dest);
                mark_cancelled(state, dl_id);
                clear_download_job(state, dl_id);
                return Err("Cancelled".into());
            }
            update_download(state, dl_id, "staged", None);
            let _ = std::fs::remove_file(&dest);
            finish_download_success(state, dl_id);
            Ok(staged)
        }
        Err(e) => {
            let msg = e.to_string();
            // An archive that would not extract is worthless; never resume onto it.
            let _ = std::fs::remove_file(&dest);
            if cancel.load(Ordering::SeqCst) {
                mark_cancelled(state, dl_id);
                clear_download_job(state, dl_id);
                Err("Cancelled".into())
            } else {
                update_download(state, dl_id, "failed", Some(msg.clone()));
                clear_download_job(state, dl_id);
                Err(msg)
            }
        }
    }
}

/// Same CDN download as [`fetch_and_stage_cdn`], but installs a FromSoft loader.
async fn fetch_cdn_and_install_loader(
    app: &tauri::AppHandle,
    state: &AppState,
    cdn_url: &str,
    cookie_header: &str,
    game_id: &str,
    label: &str,
    domain: &str,
    mod_id: u64,
    file_id: u64,
    dl_id: &str,
    batch_id: Option<String>,
    plugin_id: &str,
    install_path: &Path,
) -> Result<(), String> {
    let (dest, cancel) = download_cdn_archive(
        app,
        state,
        cdn_url,
        cookie_header,
        game_id,
        label,
        domain,
        mod_id,
        file_id,
        dl_id,
        batch_id,
        None,
        None,
        None,
    )
    .await?;

    report_cdn_extracting(app, state, dl_id, &dest);
    match games::install_legacy_from_archive(&state.paths, plugin_id, install_path, &dest) {
        Ok(()) => {
            if cancel.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&dest);
                mark_cancelled(state, dl_id);
                clear_download_job(state, dl_id);
                return Err("Cancelled".into());
            }
            update_download(state, dl_id, "staged", None);
            let _ = std::fs::remove_file(&dest);
            finish_download_success(state, dl_id);
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            let _ = std::fs::remove_file(&dest);
            if cancel.load(Ordering::SeqCst) {
                mark_cancelled(state, dl_id);
                clear_download_job(state, dl_id);
                Err("Cancelled".into())
            } else {
                update_download(state, dl_id, "failed", Some(msg.clone()));
                clear_download_job(state, dl_id);
                Err(msg)
            }
        }
    }
}

#[tauri::command]
pub fn import_assist_download(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<Option<StagedMod>, String> {
    let ctx = crate::assist::peek_assist_context(&state)
        .ok_or_else(|| "No active Download Assist context".to_string())?;
    let dl_id = uuid::Uuid::new_v4().to_string();
    let batch_id = ctx.batch_id.clone();
    let page = if matches!(
        ctx.purpose,
        crate::assist::AssistPurpose::FromsoftLegacyLoader { .. }
    ) {
        None
    } else {
        nexus_page(&ctx.game_id, &ctx.domain, ctx.mod_id)
    };
    push_download(
        &state,
        DownloadItem::new(
            dl_id.clone(),
            ctx.label.clone(),
            "extracting",
            batch_id.clone(),
            page,
        ),
    );
    let (cancel, _pause) = ensure_download_job(&state, &dl_id, batch_id);
    log::info!("slow download queued: {} id={dl_id}", ctx.label);
    if cancel.load(Ordering::SeqCst) {
        mark_cancelled(&state, &dl_id);
        clear_download_job(&state, &dl_id);
        return Err("Cancelled".into());
    }

    if let crate::assist::AssistPurpose::FromsoftLegacyLoader {
        plugin_id,
        install_path,
    } = &ctx.purpose
    {
        let archive = PathBuf::from(&path);
        match games::install_legacy_from_archive(
            &state.paths,
            plugin_id,
            Path::new(install_path),
            &archive,
        ) {
            Ok(()) => {
                remove_download_archive(&archive, &state.paths.downloads_dir());
                if cancel.load(Ordering::SeqCst) {
                    mark_cancelled(&state, &dl_id);
                    clear_download_job(&state, &dl_id);
                    return Err("Cancelled".into());
                }
                update_download(&state, &dl_id, "staged", None);
                clear_download_job(&state, &dl_id);
                let _ = crate::assist::close_assist_intentionally(&app, &state);
                let _ = app.emit(
                    "fromsoft-loader-installed",
                    serde_json::json!({
                        "id": dl_id,
                        "label": ctx.label,
                        "game_id": ctx.game_id,
                        "plugin_id": plugin_id,
                    }),
                );
                let _ = app.emit(
                    "download-finished",
                    serde_json::json!({
                        "id": dl_id,
                        "label": ctx.label,
                        "game_id": ctx.game_id,
                        "mod_id": "",
                    }),
                );
                Ok(None)
            }
            Err(e) => {
                let msg = e.to_string();
                remove_download_archive(Path::new(&path), &state.paths.downloads_dir());
                if cancel.load(Ordering::SeqCst) {
                    mark_cancelled(&state, &dl_id);
                    clear_download_job(&state, &dl_id);
                    Err("Cancelled".into())
                } else {
                    update_download(&state, &dl_id, "failed", Some(msg.clone()));
                    clear_download_job(&state, &dl_id);
                    let _ = app.emit(
                        "download-failed",
                        serde_json::json!({
                            "id": dl_id,
                            "label": ctx.label,
                            "error": msg,
                        }),
                    );
                    Err(msg)
                }
            }
        }
    } else {
        let imported_name = Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().to_string());
        let stored_file_name = mods::pick_staged_file_name(
            &ctx.label,
            ctx.file_name.as_deref(),
            imported_name.as_deref(),
        );
        assign_download_file_name(&state, &dl_id, stored_file_name.as_deref());
        match if let Some(replace_id) = ctx.replace_staged_id.as_deref() {
            mods::stage_mod_replacing(
                &state.paths,
                &ctx.game_id,
                replace_id,
                &ctx.label,
                &ctx.domain,
                ctx.mod_id,
                ctx.file_id,
                ctx.version.clone(),
                stored_file_name.clone(),
                &PathBuf::from(&path),
            )
        } else {
            mods::stage_mod(
                &state.paths,
                &ctx.game_id,
                &ctx.label,
                &ctx.domain,
                ctx.mod_id,
                ctx.file_id,
                ctx.version.clone(),
                stored_file_name,
                &PathBuf::from(&path),
            )
        } {
            Ok(staged) => {
                let archive = PathBuf::from(&path);
                remove_download_archive(&archive, &state.paths.downloads_dir());
                if cancel.load(Ordering::SeqCst) {
                    mark_cancelled(&state, &dl_id);
                    clear_download_job(&state, &dl_id);
                    return Err("Cancelled".into());
                }
                update_download(&state, &dl_id, "staged", None);
                clear_download_job(&state, &dl_id);
                let _ = crate::assist::close_assist_intentionally(&app, &state);
                let _ = app.emit(
                    "download-finished",
                    serde_json::json!({
                        "id": dl_id,
                        "label": ctx.label,
                        "game_id": ctx.game_id,
                        "mod_id": staged.id,
                    }),
                );
                Ok(Some(staged))
            }
            Err(e) => {
                let msg = e.to_string();
                remove_download_archive(Path::new(&path), &state.paths.downloads_dir());
                if cancel.load(Ordering::SeqCst) {
                    mark_cancelled(&state, &dl_id);
                    clear_download_job(&state, &dl_id);
                    Err("Cancelled".into())
                } else {
                    update_download(&state, &dl_id, "failed", Some(msg.clone()));
                    clear_download_job(&state, &dl_id);
                    let _ = app.emit(
                        "download-failed",
                        serde_json::json!({
                            "id": dl_id,
                            "label": ctx.label,
                            "error": msg,
                        }),
                    );
                    Err(msg)
                }
            }
        }
    }
}

#[tauri::command]
pub fn list_mods(state: State<'_, AppState>, game_id: String) -> Result<Vec<StagedMod>, String> {
    let mut order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    order.mods.sort_by_key(|m| m.order);
    Ok(order.mods)
}

struct PreparedWineOverrides {
    advice: Option<games::WineOverrideAdvice>,
    app_id: Option<String>,
}

fn prepare_wine_overrides(
    state: &AppState,
    game_id: &str,
) -> Result<PreparedWineOverrides, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (state, game_id);
        return Ok(PreparedWineOverrides {
            advice: None,
            app_id: None,
        });
    }
    #[cfg(target_os = "linux")]
    {
        let game = managed_game(state, game_id)?;
        let order = mods::load_loadorder(&state.paths, game_id).map_err(|e| e.to_string())?;
        let staged: Vec<games::StagedProxySource<'_>> = order
            .mods
            .iter()
            .map(|staged| games::StagedProxySource {
                name: staged.name.as_str(),
                path: Path::new(&staged.staging_path),
                enabled: staged.enabled,
            })
            .collect();
        let app_id = crate::tools::executables::steam_app_id_from_install(&game.install_path);
        let (launch_options, can_apply, steam_note) = match &app_id {
            None => (String::new(), false, None),
            Some(app_id) => match crate::tools::steam_launch::read_launch_options(app_id) {
                Ok(options) => (options.unwrap_or_default(), true, None),
                Err(err) => (
                    String::new(),
                    false,
                    Some(format!(
                        "Steam launch options could not be read ({err:#}). The line below does not include them, so merge it in by hand."
                    )),
                ),
            },
        };
        let advice = games::advise_wine_overrides(
            Path::new(&game.install_path),
            game.project_name.as_deref(),
            &staged,
            &launch_options,
            can_apply,
            steam_note.as_deref(),
        );
        Ok(PreparedWineOverrides { advice, app_id })
    }
}

#[tauri::command]
pub fn wine_override_advice(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<Option<games::WineOverrideAdvice>, String> {
    Ok(prepare_wine_overrides(&state, &game_id)?.advice)
}

#[tauri::command]
pub fn apply_wine_overrides(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<Option<games::WineOverrideAdvice>, String> {
    let prepared = prepare_wine_overrides(&state, &game_id)?;
    let Some(advice) = prepared.advice else {
        return Ok(None);
    };
    if !advice.can_apply {
        return Err("No Steam app id for this install. Copy the launch options instead.".into());
    }
    let app_id = prepared.app_id.ok_or_else(|| {
        "No Steam app id for this install. Copy the launch options instead.".to_string()
    })?;
    crate::tools::steam_launch::write_launch_options(&app_id, &advice.launch_options)
        .map_err(|e| e.to_string())?;
    Ok(prepare_wine_overrides(&state, &game_id)?.advice)
}

enum PosterLookup {
    Empty(String),
    Nexus {
        id: String,
        domain: String,
        mod_id: u64,
    },
    Thunderstore {
        id: String,
        community: String,
        namespace: String,
        name: String,
    },
    Modio {
        id: String,
        game_id: u32,
        mod_id: u64,
    },
}

fn poster_lookup(staged: &StagedMod) -> Option<PosterLookup> {
    if staged.picture_url.is_some() {
        return None;
    }
    Some(match staged.source {
        ModSource::Nexus => {
            let domain = staged.domain.trim();
            if staged.nexus_mod_id == 0
                || domain.is_empty()
                || domain.eq_ignore_ascii_case("unknown")
            {
                PosterLookup::Empty(staged.id.clone())
            } else {
                PosterLookup::Nexus {
                    id: staged.id.clone(),
                    domain: domain.to_string(),
                    mod_id: staged.nexus_mod_id,
                }
            }
        }
        ModSource::Thunderstore => {
            let community = staged.domain.trim();
            let namespace = staged.ts_namespace.as_deref().unwrap_or("").trim();
            let name = staged.ts_name.as_deref().unwrap_or("").trim();
            if community.is_empty()
                || community.eq_ignore_ascii_case("profile")
                || namespace.is_empty()
                || name.is_empty()
                || namespace.eq_ignore_ascii_case("profile")
            {
                PosterLookup::Empty(staged.id.clone())
            } else {
                PosterLookup::Thunderstore {
                    id: staged.id.clone(),
                    community: community.to_string(),
                    namespace: namespace.to_string(),
                    name: name.to_string(),
                }
            }
        }
        ModSource::Modio => match (staged.modio_game_id, staged.modio_mod_id) {
            (Some(game_id), Some(mod_id)) if game_id > 0 && mod_id > 0 => PosterLookup::Modio {
                id: staged.id.clone(),
                game_id,
                mod_id,
            },
            _ => PosterLookup::Empty(staged.id.clone()),
        },
    })
}

fn catalog_picture(url: Option<String>) -> String {
    url.map(|value| value.trim().to_string())
        .filter(|value| value.starts_with("http://") || value.starts_with("https://"))
        .unwrap_or_default()
}

#[tauri::command]
pub async fn fill_staged_pictures(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<Vec<StagedMod>, String> {
    let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    let mut updates: HashMap<String, String> = HashMap::new();
    let mut nexus = Vec::new();
    let mut thunderstore: HashMap<String, Vec<(String, String, String)>> = HashMap::new();
    let mut modio = Vec::new();
    for staged in &order.mods {
        match poster_lookup(staged) {
            Some(PosterLookup::Empty(id)) => {
                updates.insert(id, String::new());
            }
            Some(PosterLookup::Nexus { id, domain, mod_id }) => {
                nexus.push((id, domain, mod_id));
            }
            Some(PosterLookup::Thunderstore {
                id,
                community,
                namespace,
                name,
            }) => {
                thunderstore
                    .entry(community)
                    .or_default()
                    .push((id, namespace, name));
            }
            Some(PosterLookup::Modio {
                id,
                game_id,
                mod_id,
            }) => {
                modio.push((id, game_id, mod_id));
            }
            None => {}
        }
    }

    if let Ok(client) = state.client() {
        const CONCURRENCY: usize = 4;
        for chunk in nexus.chunks(CONCURRENCY) {
            let futs = chunk.iter().map(|(id, domain, mod_id)| {
                let client = state.client();
                let id = id.clone();
                let domain = domain.clone();
                let mod_id = *mod_id;
                async move {
                    let Ok(client) = client else {
                        return None;
                    };
                    match client.get_mod(&domain, mod_id).await {
                        Ok(detail) => Some((id, catalog_picture(detail.picture_url))),
                        Err(e) => {
                            log::warn!(
                                "staged poster lookup failed for nexus {domain}/{mod_id}: {e}"
                            );
                            None
                        }
                    }
                }
            });
            for found in futures_util::future::join_all(futs)
                .await
                .into_iter()
                .flatten()
            {
                updates.insert(found.0, found.1);
            }
        }
        let _ = client;
    }

    if !thunderstore.is_empty() {
        match ThunderstoreClient::new() {
            Ok(client) => {
                for (community, rows) in thunderstore {
                    match client.list_packages(&community).await {
                        Ok(packages) => {
                            for (id, namespace, name) in rows {
                                let icon = packages.iter().find(|pkg| {
                                    pkg.owner.eq_ignore_ascii_case(&namespace)
                                        && pkg.name.eq_ignore_ascii_case(&name)
                                });
                                let url = icon.and_then(|pkg| {
                                    pkg.versions.first().map(|ver| ver.icon.clone())
                                });
                                updates.insert(id, catalog_picture(url));
                            }
                        }
                        Err(e) => {
                            log::warn!(
                                "staged poster lookup failed for thunderstore {community}: {e}"
                            );
                        }
                    }
                }
            }
            Err(e) => log::warn!("thunderstore client unavailable for staged posters: {e}"),
        }
    }

    if !modio.is_empty() {
        let key = state
            .modio_api_key
            .lock()
            .ok()
            .and_then(|guard| guard.clone());
        if let Some(key) = key {
            match ModioClient::new(&key) {
                Ok(client) => {
                    const CONCURRENCY: usize = 4;
                    for chunk in modio.chunks(CONCURRENCY) {
                        let futs = chunk.iter().map(|(id, game_id, mod_id)| {
                            let id = id.clone();
                            let game_id = *game_id;
                            let mod_id = *mod_id;
                            let client = ModioClient::new(&key);
                            async move {
                                let Ok(client) = client else {
                                    return None;
                                };
                                match client.get_mod(game_id, mod_id).await {
                                    Ok(detail) => Some((id, catalog_picture(detail.picture_url))),
                                    Err(e) => {
                                        log::warn!(
                                            "staged poster lookup failed for mod.io {game_id}/{mod_id}: {e}"
                                        );
                                        None
                                    }
                                }
                            }
                        });
                        for found in futures_util::future::join_all(futs)
                            .await
                            .into_iter()
                            .flatten()
                        {
                            updates.insert(found.0, found.1);
                        }
                    }
                    let _ = client;
                }
                Err(e) => log::warn!("mod.io client unavailable for staged posters: {e}"),
            }
        }
    }

    if updates.is_empty() {
        let mut mods = order.mods;
        mods.sort_by_key(|m| m.order);
        return Ok(mods);
    }
    mods::apply_staged_pictures(&state.paths, &game_id, &updates).map_err(|e| e.to_string())
}

async fn detect_update_for_staged(
    state: &AppState,
    game_id: &str,
    staged: &StagedMod,
) -> Option<StagedModUpdate> {
    match staged.source {
        ModSource::Nexus => {
            if staged.nexus_mod_id == 0 || staged.domain.is_empty() {
                return None;
            }
            let client = state.client().ok()?;
            let files = client
                .list_mod_files(&staged.domain, staged.nexus_mod_id)
                .await
                .ok()?;
            let metas: Vec<_> = files
                .iter()
                .map(|f| mods::NexusFileMeta {
                    file_id: f.file_id,
                    version: f.version.clone(),
                    category_name: f.category_name.clone(),
                    uploaded_timestamp: f.uploaded_timestamp,
                    is_primary: f.is_primary,
                    name: f.name.clone(),
                    file_name: mods::pick_staged_file_name(
                        &staged.name,
                        Some(&f.name),
                        f.file_name.as_deref(),
                    ),
                })
                .collect();
            if staged
                .file_name
                .as_deref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
            {
                if let Some(label) = metas
                    .iter()
                    .find(|f| f.file_id == staged.nexus_file_id)
                    .and_then(|f| f.file_name.clone())
                {
                    let _ = mods::remember_file_name_if_missing(
                        &state.paths,
                        game_id,
                        &staged.id,
                        &label,
                    );
                }
            }
            let candidate = mods::nexus_update_candidate(
                staged.nexus_file_id,
                staged.version.as_deref(),
                &metas,
            )?;
            Some(StagedModUpdate {
                staged_id: staged.id.clone(),
                available_version: candidate.version.clone(),
                source: ModSource::Nexus,
                nexus_file_id: Some(candidate.file_id),
                ts_version: None,
                modio_file_id: None,
                file_name: candidate.file_name.clone(),
            })
        }
        ModSource::Thunderstore => {
            let ns = staged.ts_namespace.as_deref()?;
            let name = staged.ts_name.as_deref()?;
            if staged.domain.is_empty() {
                return None;
            }
            let client = ThunderstoreClient::new().ok()?;
            let detail = client.get_package(&staged.domain, ns, name).await.ok()?;
            let latest = detail
                .latest_version
                .or_else(|| detail.versions.first().map(|v| v.version_number.clone()))?;
            if !mods::incoming_is_newer(staged.version.as_deref(), &latest) {
                return None;
            }
            let archive_name = detail
                .versions
                .iter()
                .find(|v| v.version_number == latest)
                .and_then(|v| crate::nexus::filename_from_url(&v.download_url));
            Some(StagedModUpdate {
                staged_id: staged.id.clone(),
                available_version: Some(latest.clone()),
                source: ModSource::Thunderstore,
                nexus_file_id: None,
                ts_version: Some(latest),
                modio_file_id: None,
                file_name: mods::pick_staged_file_name(&staged.name, None, archive_name.as_deref()),
            })
        }
        ModSource::Modio => {
            let game_id = staged.modio_game_id?;
            let mod_id = staged.modio_mod_id?;
            let key = state.modio_api_key.lock().ok()?.clone()?;
            let client = ModioClient::new(&key).ok()?;
            let detail = client.get_mod(game_id, mod_id).await.ok()?;
            let primary = detail.primary_file_id?;
            if staged.modio_file_id == Some(primary) {
                return None;
            }
            let files = client.list_files(game_id, mod_id, Some(primary)).await.ok();
            let matched = files
                .as_ref()
                .and_then(|fs| fs.iter().find(|f| f.file_id == primary));
            let available_version = matched.and_then(|f| f.version.clone());
            let file_name = mods::pick_staged_file_name(
                &staged.name,
                None,
                matched.map(|f| f.filename.as_str()),
            );
            Some(StagedModUpdate {
                staged_id: staged.id.clone(),
                available_version,
                source: ModSource::Modio,
                nexus_file_id: None,
                ts_version: None,
                modio_file_id: Some(primary),
                file_name,
            })
        }
    }
}

#[tauri::command]
pub async fn check_staged_mod_updates(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<Vec<StagedModUpdate>, String> {
    let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    let mods_list = order.mods;
    let mut out = Vec::new();
    const CONCURRENCY: usize = 3;
    for chunk in mods_list.chunks(CONCURRENCY) {
        let futs: Vec<_> = chunk
            .iter()
            .map(|m| detect_update_for_staged(&state, &game_id, m))
            .collect();
        let results = futures_util::future::join_all(futs).await;
        for upd in results.into_iter().flatten() {
            out.push(upd);
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn update_staged_mod(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    game_id: String,
    staged_id: String,
) -> Result<StagedMod, String> {
    let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    let staged = order
        .mods
        .iter()
        .find(|m| m.id == staged_id)
        .cloned()
        .ok_or_else(|| format!("Staged mod not found: {staged_id}"))?;
    let update = detect_update_for_staged(&state, &game_id, &staged)
        .await
        .ok_or_else(|| "No update available for this mod".to_string())?;

    match staged.source {
        ModSource::Nexus => {
            let file_id = update
                .nexus_file_id
                .ok_or_else(|| "Missing Nexus update file id".to_string())?;
            if !state.ensure_premium_status().await {
                return Err(
                    "NEEDS_NXM: Free account — open Download Assist to use Mod Manager Download \
                     (or enable autoclick), or import a local archive."
                        .into(),
                );
            }
            download_and_stage_inner(
                Some(&app),
                &*state,
                &game_id,
                &staged.domain,
                staged.nexus_mod_id,
                file_id,
                &staged.name,
                update.available_version.clone(),
                None,
                None,
                None,
                None,
                Some(staged.id.clone()),
                update.file_name.clone(),
            )
            .await
        }
        ModSource::Thunderstore => {
            let ns = staged
                .ts_namespace
                .clone()
                .ok_or_else(|| "Missing Thunderstore namespace".to_string())?;
            let name = staged
                .ts_name
                .clone()
                .ok_or_else(|| "Missing Thunderstore package name".to_string())?;
            let client = ThunderstoreClient::new().map_err(|e| e.to_string())?;
            let pkg = client
                .get_package(&staged.domain, &ns, &name)
                .await
                .map_err(|e| e.to_string())?;
            stage_thunderstore_package(
                &app,
                &*state,
                &client,
                &game_id,
                &staged.domain,
                &pkg,
                &staged.name,
                Some(staged.enabled),
                None,
                StagePolicy::KeepNewer,
            )
            .await
        }
        ModSource::Modio => {
            let modio_game_id = staged
                .modio_game_id
                .ok_or_else(|| "Missing mod.io game id".to_string())?;
            let mod_id = staged
                .modio_mod_id
                .ok_or_else(|| "Missing mod.io mod id".to_string())?;
            let file_id = update.modio_file_id;
            let staged_all = download_modio_mod(
                app,
                state,
                game_id,
                modio_game_id,
                mod_id,
                file_id,
                staged.name.clone(),
                update.available_version.clone(),
                Some(false),
            )
            .await?;
            staged_all
                .into_iter()
                .find(|m| m.modio_mod_id == Some(mod_id))
                .ok_or_else(|| "mod.io update staged nothing".to_string())
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct NexusCollectionFileRef {
    pub mod_id: u64,
    pub file_id: u64,
}

#[tauri::command]
pub fn list_installed_collections(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<Vec<InstalledCollection>, String> {
    mods::list_installed_collections(&state.paths, &game_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn uninstall_collection(
    state: State<'_, AppState>,
    game_id: String,
    collection_id: String,
) -> Result<usize, String> {
    mods::uninstall_collection(&state.paths, &game_id, &collection_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn record_nexus_collection(
    state: State<'_, AppState>,
    game_id: String,
    slug: String,
    name: String,
    revision: Option<i64>,
    files: Vec<NexusCollectionFileRef>,
    existing_mod_ids: Option<Vec<String>>,
) -> Result<InstalledCollection, String> {
    let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    let mut member_ids = Vec::new();
    for file in &files {
        if let Some(staged) = mods::find_nexus_file(&order, file.mod_id, file.file_id) {
            member_ids.push(staged.id.clone());
        }
    }
    if member_ids.is_empty() {
        return Err("No staged mods matched this collection yet.".into());
    }
    let preexisting: HashSet<String> = existing_mod_ids.unwrap_or_default().into_iter().collect();
    let new_ids: Vec<String> = member_ids
        .iter()
        .filter(|id| !preexisting.contains(*id))
        .cloned()
        .collect();
    let collection = InstalledCollection {
        id: mods::nexus_collection_id(&slug),
        source: CollectionSource::Nexus,
        kind: CollectionKind::Collection,
        name,
        slug: Some(slug),
        namespace: None,
        package_name: None,
        community: None,
        revision,
        version: None,
        profile_code: None,
        mod_ids: member_ids,
        installed_at: String::new(),
        game_versions: None,
    };
    mods::record_installed_collection(&state.paths, &game_id, collection, &new_ids)
        .map_err(|e| e.to_string())
}

fn managed_game(state: &AppState, game_id: &str) -> Result<ManagedGame, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.managed_games
        .iter()
        .find(|g| g.id == game_id)
        .cloned()
        .ok_or_else(|| "Managed game not found".to_string())
}

#[tauri::command]
pub fn export_share_code(
    state: State<'_, AppState>,
    game_id: String,
    name: Option<String>,
) -> Result<ExportShareResult, String> {
    let game = managed_game(&state, &game_id)?;
    let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    let (manifest, skipped) = share::build_manifest_from_loadorder(&order, &game, name.clone());
    if manifest.mods.is_empty() {
        return Err(if skipped.is_empty() {
            "No enabled mods to share.".into()
        } else {
            format!(
                "No portable enabled mods to share. Skipped: {}",
                skipped.join("; ")
            )
        });
    }
    let code = share::encode_share_manifest(&manifest).map_err(|e| e.to_string())?;
    let mut warnings = Vec::new();
    if !skipped.is_empty() {
        warnings.push(format!(
            "Skipped {} non-portable mod(s): {}",
            skipped.len(),
            skipped.join("; ")
        ));
    }
    if code.len() > 100 * 1024 {
        warnings.push(format!(
            "Share code is large ({:.1} KB); some chat apps may truncate it.",
            code.len() as f64 / 1024.0
        ));
    }
    let saved = share::entry_from_code(&code, name, Some(game_id)).map_err(|e| e.to_string())?;
    let saved = share::add_saved_collection(&state.paths, saved).map_err(|e| e.to_string())?;
    Ok(ExportShareResult {
        code,
        warnings,
        skipped,
        mod_count: manifest.mods.len(),
        saved,
    })
}

#[tauri::command]
pub fn decode_share_code(code: String) -> Result<ShareDecodePreview, String> {
    let code = code.trim().to_string();
    let manifest = share::decode_share_code(&code).map_err(|e| e.to_string())?;
    Ok(share::preview_from_manifest(&manifest, code.len()))
}

#[tauri::command]
pub fn detect_import_code(code: String) -> Result<DetectImportResult, String> {
    let code = code.trim().to_string();
    let kind = share::detect_code_kind(&code);
    match kind {
        ImportCodeKind::Emperor => {
            let manifest = share::decode_share_code(&code).map_err(|e| e.to_string())?;
            let preview = share::preview_from_manifest(&manifest, code.len());
            Ok(DetectImportResult {
                kind,
                message: format!(
                    "Emperor share: {} mod(s)",
                    preview.mods.len()
                ),
                preview: Some(preview),
            })
        }
        ImportCodeKind::ThunderstoreProfile => Ok(DetectImportResult {
            kind,
            message: "Thunderstore / r2modman profile code".into(),
            preview: None,
        }),
        ImportCodeKind::Unknown => Ok(DetectImportResult {
            kind,
            message: "Unrecognized code. Paste an Emperor share (#emperor1) or r2modman / Gale profile code.".into(),
            preview: None,
        }),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImportCodeResult {
    Emperor { result: ShareImportResult },
    ThunderstoreProfile { collection: InstalledCollection },
}

#[tauri::command]
pub async fn import_code(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    game_id: String,
    code: String,
    name: Option<String>,
) -> Result<ImportCodeResult, String> {
    let code = code.trim().to_string();
    match share::detect_code_kind(&code) {
        ImportCodeKind::Emperor => {
            let result = import_share_code(app, state, game_id, code, name).await?;
            Ok(ImportCodeResult::Emperor { result })
        }
        ImportCodeKind::ThunderstoreProfile => {
            let collection = import_thunderstore_profile(app, state, game_id, code).await?;
            Ok(ImportCodeResult::ThunderstoreProfile { collection })
        }
        ImportCodeKind::Unknown => Err(
            "Unrecognized code. Paste an Emperor share (#emperor1) or r2modman / Gale profile code."
                .into(),
        ),
    }
}

#[tauri::command]
pub async fn import_share_code(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    game_id: String,
    code: String,
    name: Option<String>,
) -> Result<ShareImportResult, String> {
    let game = managed_game(&state, &game_id)?;
    let code = code.trim().to_string();
    let manifest = share::decode_share_code(&code).map_err(|e| e.to_string())?;
    share::catalogs_overlap(&manifest.game, &game)?;
    let mut warnings = Vec::new();
    if let Some(w) = share::plugin_mismatch_warning(&manifest.game, &game) {
        warnings.push(w);
    }

    let collection_name = name
        .or(manifest.name.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Shared loadout".into());
    let collection_id = share::emperor_share_id(&code);
    let before = snapshot_mod_ids(&state.paths, &game_id)?;
    let is_premium = state.ensure_premium_status().await;
    let modio_key = state
        .modio_api_key
        .lock()
        .map_err(|e| e.to_string())?
        .clone();

    let mut member_ids = Vec::new();
    let mut needs_assist = Vec::new();
    let mut modio_failures = 0usize;
    let ts_client = ThunderstoreClient::new().map_err(|e| e.to_string())?;

    for entry in &manifest.mods {
        match entry.s {
            ShareModSource::Nexus => {
                let domain = entry
                    .domain
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(game.nexus_domain.as_str());
                let mod_id = entry.mod_id.unwrap_or(0);
                let file_id = entry.file_id.unwrap_or(0);
                if mod_id == 0 || file_id == 0 {
                    warnings.push(format!(
                        "Skipped invalid Nexus entry {}",
                        share::mod_label(entry)
                    ));
                    continue;
                }
                let order =
                    mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
                if let Some(existing) = mods::find_nexus_file(&order, mod_id, file_id) {
                    member_ids.push(existing.id.clone());
                    continue;
                }
                let label = share::mod_label(entry);
                if is_premium {
                    match download_and_stage_inner(
                        Some(&app),
                        &*state,
                        &game_id,
                        domain,
                        mod_id,
                        file_id,
                        &label,
                        entry.version.clone(),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                    )
                    .await
                    {
                        Ok(staged) => member_ids.push(staged.id),
                        Err(e) => {
                            warnings.push(format!("{label}: {e}"));
                        }
                    }
                } else {
                    needs_assist.push(ShareAssistFile {
                        domain: domain.to_string(),
                        mod_id,
                        file_id,
                        name: label,
                        version: entry.version.clone(),
                    });
                }
            }
            ShareModSource::Thunderstore => {
                let community = entry
                    .community
                    .as_deref()
                    .or(game.thunderstore_community.as_deref())
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        "Share includes Thunderstore mods but this game has no community set."
                            .to_string()
                    })?;
                let ns = entry
                    .namespace
                    .as_deref()
                    .ok_or_else(|| "Thunderstore entry missing namespace".to_string())?;
                let pkg_name = entry
                    .name
                    .as_deref()
                    .ok_or_else(|| "Thunderstore entry missing package name".to_string())?;
                let label = share::mod_label(entry);
                let version = entry.version.as_deref().filter(|s| !s.is_empty());
                match stage_thunderstore_with_deps(
                    &app,
                    &state,
                    &ts_client,
                    &game_id,
                    community,
                    ns,
                    pkg_name,
                    version,
                    None,
                    Some(&label),
                )
                .await
                {
                    Ok((staged, _)) => {
                        for s in staged {
                            if !member_ids.contains(&s.id) {
                                member_ids.push(s.id);
                            }
                        }
                    }
                    Err(e) => warnings.push(format!("{label}: {e}")),
                }
            }
            ShareModSource::Modio => {
                let Some(key) = modio_key.as_deref() else {
                    modio_failures += 1;
                    warnings.push(format!(
                        "{}: mod.io API key required",
                        share::mod_label(entry)
                    ));
                    continue;
                };
                let gid = entry.game_id.or(game.modio_game_id).unwrap_or(0);
                let mid = entry.mod_id.unwrap_or(0);
                if gid == 0 || mid == 0 {
                    warnings.push(format!(
                        "Skipped invalid mod.io entry {}",
                        share::mod_label(entry)
                    ));
                    continue;
                }
                let order =
                    mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
                if let Some(existing) = mods::find_modio_mod(&order, gid, mid) {
                    member_ids.push(existing.id.clone());
                    continue;
                }
                let client = ModioClient::new(key).map_err(|e| e.to_string())?;
                let detail = client.get_mod(gid, mid).await.map_err(|e| e.to_string())?;
                let resolved_file = entry
                    .file_id
                    .or(detail.primary_file_id)
                    .ok_or_else(|| format!("No downloadable file for mod.io mod {mid}"))?;
                let label = share::mod_label(entry);
                let dl_id = uuid::Uuid::new_v4().to_string();
                let dest = state.paths.downloads_dir().join(format!(
                    "modio_{}_{}_{}.zip",
                    dl_id,
                    sanitize_filename::sanitize(&detail.name_id),
                    resolved_file
                ));
                {
                    push_download(
                        &state,
                        DownloadItem::new(
                            dl_id.clone(),
                            label.clone(),
                            "downloading",
                            None,
                            modio_page(&game_id, gid, mid),
                        ),
                    );
                }
                let _ = app.emit("downloads-changed", ());
                let on_progress = progress_callback(Some(&app), dl_id.clone());
                match client
                    .download_file(gid, mid, Some(resolved_file), &dest, on_progress.as_ref())
                    .await
                {
                    Ok(()) => {
                        update_download(&state, &dl_id, "extracting", None);
                        let _ = app.emit("downloads-changed", ());
                        let modio_label = modio_file_label(
                            &client,
                            gid,
                            mid,
                            resolved_file,
                            detail.primary_file_id,
                            &detail.name,
                        )
                        .await;
                        assign_download_file_name(&state, &dl_id, modio_label.as_deref());
                        match mods::stage_modio_mod(
                            &state.paths,
                            &game_id,
                            gid,
                            mid,
                            resolved_file,
                            &detail.name,
                            entry.version.clone(),
                            modio_label,
                            detail.picture_url.clone(),
                            &dest,
                        ) {
                            Ok(staged) => {
                                member_ids.push(staged.id);
                                update_download(&state, &dl_id, "done", None);
                                remove_download_archive(&dest, &state.paths.downloads_dir());
                                clear_download_job(&state, &dl_id);
                                let _ = app.emit("downloads-changed", ());
                            }
                            Err(e) => {
                                modio_failures += 1;
                                let msg = e.to_string();
                                remove_download_archive(&dest, &state.paths.downloads_dir());
                                update_download(&state, &dl_id, "failed", Some(msg.clone()));
                                clear_download_job(&state, &dl_id);
                                let _ = app.emit("downloads-changed", ());
                                warnings.push(format!("{label}: {msg}"));
                            }
                        }
                    }
                    Err(e) => {
                        modio_failures += 1;
                        let msg = e.to_string();
                        remove_download_archive(&dest, &state.paths.downloads_dir());
                        update_download(&state, &dl_id, "failed", Some(msg.clone()));
                        clear_download_job(&state, &dl_id);
                        let _ = app.emit("downloads-changed", ());
                        warnings.push(format!("{label}: {msg}"));
                    }
                }
            }
        }
    }

    if member_ids.is_empty() && needs_assist.is_empty() {
        if modio_failures > 0 {
            return Err(
                "Could not install any mods. Set a mod.io API key in Setup and try again.".into(),
            );
        }
        return Err(if warnings.is_empty() {
            "No mods could be installed from this share code.".into()
        } else {
            format!("No mods installed. {}", warnings.join("; "))
        });
    }

    // Deduplicate member ids while preserving order.
    let mut seen = HashSet::new();
    member_ids.retain(|id| seen.insert(id.clone()));

    let collection = if needs_assist.is_empty() {
        let new_ids: Vec<String> = member_ids
            .iter()
            .filter(|id| !before.contains(*id))
            .cloned()
            .collect();
        let col = InstalledCollection {
            id: collection_id.clone(),
            source: CollectionSource::Emperor,
            kind: CollectionKind::Share,
            name: collection_name.clone(),
            slug: Some(content_slug_from_id(&collection_id)),
            namespace: None,
            package_name: None,
            community: None,
            revision: None,
            version: None,
            profile_code: Some(code),
            mod_ids: member_ids.clone(),
            installed_at: String::new(),
            game_versions: None,
        };
        Some(
            mods::record_installed_collection(&state.paths, &game_id, col, &new_ids)
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };

    Ok(ShareImportResult {
        collection_id,
        name: collection_name,
        member_ids,
        needs_assist,
        warnings,
        collection,
    })
}

fn content_slug_from_id(collection_id: &str) -> String {
    collection_id
        .strip_prefix("emperor-share:")
        .unwrap_or(collection_id)
        .to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmperorShareFileRef {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
}

#[tauri::command]
pub fn record_emperor_share(
    state: State<'_, AppState>,
    game_id: String,
    collection_id: String,
    name: String,
    code: Option<String>,
    files: Vec<EmperorShareFileRef>,
    member_ids: Option<Vec<String>>,
    existing_mod_ids: Option<Vec<String>>,
) -> Result<InstalledCollection, String> {
    let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    let mut ids = member_ids.unwrap_or_default();
    for file in &files {
        if let Some(staged) = mods::find_nexus_file(&order, file.mod_id, file.file_id) {
            if !ids.iter().any(|id| id == &staged.id) {
                ids.push(staged.id.clone());
            }
        }
    }
    if ids.is_empty() {
        return Err("No staged mods matched this share yet.".into());
    }
    let preexisting: HashSet<String> = existing_mod_ids.unwrap_or_default().into_iter().collect();
    let new_ids: Vec<String> = ids
        .iter()
        .filter(|id| !preexisting.contains(*id))
        .cloned()
        .collect();
    let collection = InstalledCollection {
        id: collection_id.clone(),
        source: CollectionSource::Emperor,
        kind: CollectionKind::Share,
        name,
        slug: Some(content_slug_from_id(&collection_id)),
        namespace: None,
        package_name: None,
        community: None,
        revision: None,
        version: None,
        profile_code: code,
        mod_ids: ids,
        installed_at: String::new(),
        game_versions: None,
    };
    mods::record_installed_collection(&state.paths, &game_id, collection, &new_ids)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_saved_collections(
    state: State<'_, AppState>,
) -> Result<Vec<SavedCollectionEntry>, String> {
    Ok(share::load_saved_collections(&state.paths)
        .map_err(|e| e.to_string())?
        .collections)
}

#[tauri::command]
pub fn save_collection_code(
    state: State<'_, AppState>,
    code: String,
    name: Option<String>,
    source_game_id: Option<String>,
) -> Result<SavedCollectionEntry, String> {
    let kind = share::detect_code_kind(code.trim());
    if kind != ImportCodeKind::Emperor {
        return Err(
            "Only Emperor share codes (#emperor1) can be saved to the local library.".into(),
        );
    }
    let entry =
        share::entry_from_code(code.trim(), name, source_game_id).map_err(|e| e.to_string())?;
    share::add_saved_collection(&state.paths, entry).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_saved_collection(
    state: State<'_, AppState>,
    id: String,
) -> Result<SavedCollectionDetail, String> {
    share::get_saved_collection(&state.paths, &id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn rename_saved_collection(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Result<SavedCollectionEntry, String> {
    if name.trim().is_empty() {
        return Err("Name cannot be empty.".into());
    }
    share::rename_saved_collection(&state.paths, &id, &name).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_saved_collection(state: State<'_, AppState>, id: String) -> Result<(), String> {
    share::delete_saved_collection(&state.paths, &id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_mod_enabled(
    state: State<'_, AppState>,
    game_id: String,
    mod_id: String,
    enabled: bool,
) -> Result<(), String> {
    mods::set_enabled(&state.paths, &game_id, &mod_id, enabled).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_load_order(
    state: State<'_, AppState>,
    game_id: String,
    ordered_ids: Vec<String>,
) -> Result<(), String> {
    mods::set_load_order(&state.paths, &game_id, &ordered_ids).map_err(|e| e.to_string())
}

#[derive(Debug, Serialize)]
pub struct ModOptionsView {
    pub set: ModOptionSet,
    /// What is in effect right now: the stored picks, or the defaults when the
    /// user has not opened the dialog yet.
    pub selection: ModOptionSelection,
}

fn plugin_for_game(state: &AppState, game_id: &str) -> Result<&'static dyn GamePlugin, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    let plugin_id = cfg
        .managed_games
        .iter()
        .find(|g| g.id == game_id)
        .map(|g| g.plugin_id.clone())
        .ok_or_else(|| "Managed game not found".to_string())?;
    drop(cfg);
    games::plugin_by_id(&plugin_id).ok_or_else(|| format!("Unknown game plugin: {plugin_id}"))
}

#[tauri::command]
pub fn diagnose_load(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<crate::diagnostics::LoadDiagnosis, String> {
    let game = {
        let cfg = state.config.lock().map_err(|e| e.to_string())?;
        cfg.managed_games
            .iter()
            .find(|g| g.id == game_id)
            .cloned()
            .ok_or_else(|| "Managed game not found".to_string())?
    };
    let plugin = games::plugin_by_id(&game.plugin_id)
        .ok_or_else(|| format!("Unknown game plugin: {}", game.plugin_id))?;
    let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    let enabled_mods: Vec<String> = order
        .mods
        .iter()
        .filter(|staged| staged.enabled)
        .map(|staged| staged.name.clone())
        .collect();
    let manifest_path = state.paths.deploy_manifest(&game_id);
    let deployed_paths = std::fs::read_to_string(&manifest_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<mods::DeployManifest>(&raw).ok())
        .map(|manifest| manifest.paths)
        .unwrap_or_default();
    let deployed_at = std::fs::metadata(&manifest_path)
        .and_then(|meta| meta.modified())
        .ok();
    let manager_log = crate::app_log::recent_warn_lines(&state.paths.log_file());
    Ok(crate::diagnostics::diagnose(
        &crate::diagnostics::DiagnoseInput {
            plugin,
            install_path: Path::new(&game.install_path),
            game_title: &game.title,
            plugin_id: &game.plugin_id,
            app_version: APP_VERSION,
            enabled_mods: &enabled_mods,
            deployed_paths: &deployed_paths,
            deployed_at,
            manager_log: &manager_log,
        },
    ))
}

/// The staged content root a plugin inspects, i.e. what deploy would use.
fn staged_content_root(staged: &StagedMod, plugin: &dyn GamePlugin) -> Option<PathBuf> {
    let staging = PathBuf::from(&staged.staging_path);
    if !staging.is_dir() {
        return None;
    }
    games::normalize_staging_root(&staging, plugin).ok()
}

fn enabled_deploy_roots(
    paths: &crate::config::Paths,
    game_id: &str,
    plugin_id: &str,
) -> Vec<PathBuf> {
    let Some(plugin) = games::plugin_by_id(plugin_id) else {
        return Vec::new();
    };
    let Ok(order) = mods::load_loadorder(paths, game_id) else {
        return Vec::new();
    };
    order
        .mods
        .iter()
        .filter(|staged| staged.enabled)
        .filter_map(|staged| staged_content_root(staged, plugin))
        .collect()
}

#[tauri::command]
pub fn get_mod_options(
    state: State<'_, AppState>,
    game_id: String,
    mod_id: String,
) -> Result<Option<ModOptionsView>, String> {
    let plugin = plugin_for_game(&state, &game_id)?;
    let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    let staged = order
        .mods
        .iter()
        .find(|m| m.id == mod_id)
        .ok_or_else(|| format!("Mod not found: {mod_id}"))?;
    let Some(root) = staged_content_root(staged, plugin) else {
        return Ok(None);
    };
    let Some(set) = plugin.mod_options(&root).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let selection = options::effective_selection(&set, staged.option_selection.as_ref());
    Ok(Some(ModOptionsView { set, selection }))
}

#[tauri::command]
pub fn set_mod_options(
    state: State<'_, AppState>,
    game_id: String,
    mod_id: String,
    selection: ModOptionSelection,
) -> Result<ModOptionSelection, String> {
    let plugin = plugin_for_game(&state, &game_id)?;
    let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    let staged = order
        .mods
        .iter()
        .find(|m| m.id == mod_id)
        .ok_or_else(|| format!("Mod not found: {mod_id}"))?;
    let root = staged_content_root(staged, plugin)
        .ok_or_else(|| format!("Staging folder is missing for {}", staged.name))?;
    let set = plugin
        .mod_options(&root)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{} does not declare any options", staged.name))?;

    // Store the repaired selection rather than what the UI sent, so a stale
    // dialog cannot persist ids the manifest no longer has.
    let normalized = options::normalize_selection(&set, &selection);
    mods::set_mod_options(&state.paths, &game_id, &mod_id, normalized.clone())
        .map_err(|e| e.to_string())?;
    Ok(normalized)
}

/// Ids of staged mods that declare options, so the Mods list can show the
/// button without asking about every row one at a time.
#[tauri::command]
pub fn list_mods_with_options(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<Vec<String>, String> {
    let plugin = plugin_for_game(&state, &game_id)?;
    let order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    Ok(order
        .mods
        .iter()
        .filter(|staged| {
            staged_content_root(staged, plugin)
                .and_then(|root| plugin.mod_options(&root))
                .is_some_and(|set| !set.is_empty())
        })
        .map(|staged| staged.id.clone())
        .collect())
}

#[tauri::command]
pub fn remove_mod(
    state: State<'_, AppState>,
    game_id: String,
    mod_id: String,
) -> Result<(), String> {
    mods::remove_mod(&state.paths, &game_id, &mod_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_all_mods(state: State<'_, AppState>, game_id: String) -> Result<(), String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    let install_path = cfg
        .managed_games
        .iter()
        .find(|g| g.id == game_id)
        .map(|g| g.install_path.clone());
    drop(cfg);
    mods::remove_all_mods(
        &state.paths,
        &game_id,
        install_path.as_deref().map(Path::new),
    )
    .map_err(|e| e.to_string())
}

/// Download Thunderstore packages whose staging lost files, so a deploy links a
/// complete mod instead of a partial one. Returns warnings for what could not
/// be repaired.
async fn repair_damaged_staging(
    app: &tauri::AppHandle,
    state: &AppState,
    game_id: &str,
) -> Vec<String> {
    let Ok(order) = mods::load_loadorder(&state.paths, game_id) else {
        return Vec::new();
    };
    let damaged: Vec<StagedMod> = mods::damaged_staging(&order).into_iter().cloned().collect();
    if damaged.is_empty() {
        return Vec::new();
    }
    let client = match ThunderstoreClient::new() {
        Ok(client) => client,
        Err(e) => return vec![format!("Could not repair staged mods: {e}")],
    };

    let mut warnings = Vec::new();
    for staged in damaged {
        let (Some(ns), Some(name)) = (staged.ts_namespace.clone(), staged.ts_name.clone()) else {
            warnings.push(format!(
                "{} is missing staged files and has to be reinstalled by hand.",
                staged.name
            ));
            continue;
        };
        let mut pkg = match client.get_package(&staged.domain, &ns, &name).await {
            Ok(pkg) => pkg,
            Err(e) => {
                warnings.push(format!("Could not re-download {}: {e}", staged.name));
                continue;
            }
        };
        // Repair to the version that is recorded, not to the newest release.
        if let Some(pos) = staged
            .version
            .as_deref()
            .and_then(|want| pkg.versions.iter().position(|v| v.version_number == want))
        {
            let wanted = pkg.versions.remove(pos);
            pkg.versions.insert(0, wanted);
        }
        if let Err(e) = stage_thunderstore_package(
            app,
            state,
            &client,
            game_id,
            &staged.domain,
            &pkg,
            &staged.name,
            Some(staged.enabled),
            None,
            StagePolicy::Force,
        )
        .await
        {
            warnings.push(format!("Could not re-download {}: {e}", staged.name));
        }
    }
    warnings
}

#[tauri::command]
pub async fn deploy_mods(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    game_id: String,
) -> Result<mods::DeployResult, String> {
    let game = {
        let cfg = state.config.lock().map_err(|e| e.to_string())?;
        cfg.managed_games
            .iter()
            .find(|g| g.id == game_id)
            .cloned()
            .ok_or_else(|| "Managed game not found".to_string())?
    };
    let repair_warnings = repair_damaged_staging(&app, &state, &game_id).await;
    let install = PathBuf::from(&game.install_path);
    let Some(saved_plugin) = games::plugin_by_id(&game.plugin_id) else {
        return Err(format!("Unknown game plugin: {}", game.plugin_id));
    };
    // The saved plugin can name a more specific title for this install.
    // The plugin decides; this command only persists the id.
    let preferred = saved_plugin.preferred_plugin_id(&install);
    let mut plugin_id = game.plugin_id.clone();
    let mut plugin_warning = None;
    let plugin: &dyn GamePlugin = if preferred != game.plugin_id {
        if let Some(next) = games::plugin_by_id(preferred) {
            let display = next.info().display_name;
            plugin_id = preferred.to_string();
            match state.config.lock() {
                Ok(mut cfg) => {
                    if let Some(saved) = cfg.managed_games.iter_mut().find(|g| g.id == game_id) {
                        saved.plugin_id = plugin_id.clone();
                    }
                    if let Err(e) = config::save_config(&state.paths, &cfg) {
                        plugin_warning = Some(format!(
                            "{display} was detected, but the saved game type could not be updated ({e:#})."
                        ));
                    }
                }
                Err(e) => {
                    plugin_warning = Some(format!(
                        "{display} was detected, but the saved game type could not be updated ({e})."
                    ));
                }
            }
            next
        } else {
            saved_plugin
        }
    } else {
        saved_plugin
    };
    let nexus = state.client().ok();
    let premium = state.is_premium();
    let content_roots = enabled_deploy_roots(&state.paths, &game_id, &plugin_id);
    let content_refs: Vec<&Path> = content_roots.iter().map(PathBuf::as_path).collect();
    let prep = games::RuntimePrep {
        paths: &state.paths,
        install_path: &install,
        nexus: nexus.as_ref(),
        premium,
        content_roots: &content_refs,
    };
    let mut loader_warnings = match plugin.prepare_runtime(&prep).await {
        Ok(games::RuntimePrepOutcome::Done(warnings)) => warnings,
        Ok(games::RuntimePrepOutcome::NeedsAssist { warnings, request }) => {
            let mut warnings = warnings;
            if let Err(e) =
                wait_for_loader_assist(&app, &state, &game_id, plugin, &prep, &request).await
            {
                warnings.push(e);
            } else if let Ok(extra) = plugin.finish_runtime_prep(&prep) {
                warnings.extend(extra);
            }
            warnings
        }
        Err(e) => vec![format!("{e:#}")],
    };
    let mut result = mods::deploy(
        &state.paths,
        &game_id,
        &plugin_id,
        &install,
        game.project_name.as_deref(),
    )
    .map_err(|e| e.to_string())?;
    if let Some(warning) = plugin_warning {
        result.warnings.insert(0, warning);
    }
    result.warnings.splice(0..0, loader_warnings.drain(..));
    result.warnings.splice(0..0, repair_warnings);
    if !result.plugin_skips.is_empty() {
        if let Ok(order) = mods::load_loadorder(&state.paths, &game_id) {
            let mut owners: Vec<StagedMod> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            for skip in &result.plugin_skips {
                for staged in &order.mods {
                    if !staged.enabled
                        || staged.source != ModSource::Nexus
                        || staged.nexus_mod_id == 0
                        || seen.contains(&staged.id)
                    {
                        continue;
                    }
                    if staging_contains_filename(Path::new(&staged.staging_path), &skip.plugin) {
                        seen.insert(staged.id.clone());
                        owners.push(staged.clone());
                    }
                }
            }
            let seeds = owners.iter().filter_map(seed_from_staged).collect();
            result.missing_requirements =
                collect_requirement_seeds(&state, seeds, staged_nexus_mods(&order.mods)).await;
        }
    }
    Ok(result)
}

async fn wait_for_loader_assist(
    app: &tauri::AppHandle,
    state: &AppState,
    game_id: &str,
    plugin: &dyn GamePlugin,
    prep: &games::RuntimePrep<'_>,
    request: &games::LoaderAssistRequest,
) -> Result<(), String> {
    crate::assist::open_download_assist_inner(
        app,
        state,
        game_id.to_string(),
        request.domain.clone(),
        request.mod_id,
        request.file_id,
        request.label.clone(),
        None,
        None,
        None,
        None,
        crate::assist::AssistPurpose::FromsoftLegacyLoader {
            plugin_id: request.plugin_id.clone(),
            install_path: request.install_path.display().to_string(),
        },
    )
    .await?;

    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        if plugin.loader_assist_ready(prep) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    Err(format!(
        "Timed out waiting for {} (180s). Complete the download in Assist and deploy again.",
        request.label
    ))
}

#[tauri::command]
pub fn purge_mods(state: State<'_, AppState>, game_id: String) -> Result<(), String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    let game = cfg
        .managed_games
        .iter()
        .find(|g| g.id == game_id)
        .map(|g| (g.install_path.clone(), g.plugin_id.clone()));
    drop(cfg);
    let install_path = game.as_ref().map(|(path, _)| path.clone());
    mods::purge_deploy(
        &state.paths,
        &game_id,
        install_path.as_deref().map(Path::new),
    )
    .map_err(|e| e.to_string())?;
    if let Some((install_path, plugin_id)) = game {
        if let Some(plugin) = games::plugin_by_id(&plugin_id) {
            mods::prune_empty_loader_dirs(plugin, Path::new(&install_path));
        }
    }
    Ok(())
}

#[tauri::command]
pub fn list_downloads(state: State<'_, AppState>) -> Result<Vec<DownloadItem>, String> {
    Ok(state.downloads.lock().map_err(|e| e.to_string())?.clone())
}

#[tauri::command]
pub fn clear_recent_downloads(state: State<'_, AppState>, ids: Vec<String>) -> Result<(), String> {
    let removed_ids: Vec<String> = {
        let downloads = state.downloads.lock().map_err(|e| e.to_string())?;
        downloads
            .iter()
            .filter(|d| ids.iter().any(|id| id == &d.id))
            .filter(|d| is_finished_download_status(&d.status))
            .map(|d| d.id.clone())
            .collect()
    };
    for id in &removed_ids {
        clear_restart_meta(&state, id);
        clear_download_job(&state, id);
    }
    let mut downloads = state.downloads.lock().map_err(|e| e.to_string())?;
    downloads.retain(|d| !removed_ids.iter().any(|id| id == &d.id));
    drop(downloads);
    commit_recent_downloads(&state);
    Ok(())
}

#[tauri::command]
pub fn remove_download(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut downloads = state.downloads.lock().map_err(|e| e.to_string())?;
    if let Some(item) = downloads.iter().find(|d| d.id == id) {
        if is_active_download_status(&item.status) {
            return Err("Cannot remove an active download".into());
        }
    }
    downloads.retain(|d| d.id != id);
    drop(downloads);
    clear_restart_meta(&state, &id);
    clear_download_job(&state, &id);
    commit_recent_downloads(&state);
    Ok(())
}

#[tauri::command]
pub fn cancel_download(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let active = state
        .downloads
        .lock()
        .ok()
        .and_then(|q| {
            q.iter().find(|d| d.id == id).map(|d| {
                d.status == "downloading" || d.status == "extracting" || d.status == "paused"
            })
        })
        .unwrap_or(false);
    if !active
        && state
            .download_jobs
            .lock()
            .ok()
            .map(|j| !j.contains_key(&id))
            .unwrap_or(true)
    {
        return Ok(());
    }
    request_cancel_ids(&state, &[id], Some(&app));
    Ok(())
}

#[tauri::command]
pub fn cancel_download_batch(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    batch_id: String,
) -> Result<(), String> {
    let ids: Vec<String> = {
        let downloads = state.downloads.lock().map_err(|e| e.to_string())?;
        let jobs = state.download_jobs.lock().map_err(|e| e.to_string())?;
        let mut ids: Vec<String> = downloads
            .iter()
            .filter(|d| {
                d.batch_id.as_deref() == Some(batch_id.as_str())
                    && (d.status == "downloading"
                        || d.status == "extracting"
                        || d.status == "paused")
            })
            .map(|d| d.id.clone())
            .collect();
        for (id, job) in jobs.iter() {
            if job.batch_id.as_deref() == Some(batch_id.as_str()) && !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        ids
    };
    request_cancel_ids(&state, &ids, Some(&app));
    Ok(())
}

#[tauri::command]
pub fn pause_download(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let status = state
        .downloads
        .lock()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.status.clone());
    if status.as_deref() != Some("downloading") {
        return Err("Only in-progress downloads can be paused".into());
    }
    if let Ok(mut jobs) = state.download_jobs.lock() {
        if let Some(job) = jobs.get_mut(&id) {
            job.pause.store(true, Ordering::SeqCst);
            return Ok(());
        }
    }
    Err("Download job not found".into())
}

#[tauri::command]
pub fn resume_download(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let status = state
        .downloads
        .lock()
        .map_err(|e| e.to_string())?
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.status.clone());
    if status.as_deref() != Some("paused") {
        return Err("Only paused downloads can be resumed".into());
    }

    let (source, dest, batch_id, cancel, pause) = {
        let mut jobs = state.download_jobs.lock().map_err(|e| e.to_string())?;
        let job = jobs
            .get_mut(&id)
            .ok_or_else(|| "Download job not found".to_string())?;
        let source = match job.source.as_ref() {
            Some(
                DownloadResumeSource::Thunderstore { .. } | DownloadResumeSource::Modio { .. },
            ) => {
                return Err("Cannot resume this download — use Restart instead.".into());
            }
            Some(source) => source.clone(),
            None => return Err("Missing resume metadata for this download".to_string()),
        };
        let dest = job
            .dest
            .clone()
            .ok_or_else(|| "Missing download path for resume".to_string())?;
        job.pause.store(false, Ordering::SeqCst);
        job.cancel.store(false, Ordering::SeqCst);
        (
            source,
            dest,
            job.batch_id.clone(),
            job.cancel.clone(),
            job.pause.clone(),
        )
    };

    if cancel.load(Ordering::SeqCst) {
        return Err("Download was cancelled".into());
    }
    let _ = dest;
    let _ = pause;

    update_download(&state, &id, "downloading", None);

    let app_bg = app.clone();
    let queued_id = id.clone();

    tauri::async_runtime::spawn(async move {
        let state = app_bg.state::<AppState>();
        let result = match source {
            DownloadResumeSource::Api {
                game_id,
                domain,
                mod_id,
                file_id,
                label,
                version,
                nxm_key,
                nxm_expires,
                replace_staged_id,
                file_name,
            } => {
                let label_clone = label.clone();
                let game_id_clone = game_id.clone();
                let res = download_and_stage_inner(
                    Some(&app_bg),
                    &*state,
                    &game_id,
                    &domain,
                    mod_id,
                    file_id,
                    &label,
                    version,
                    nxm_key,
                    nxm_expires,
                    Some(queued_id.clone()),
                    batch_id,
                    replace_staged_id,
                    file_name,
                )
                .await;
                (res, label_clone, game_id_clone)
            }
            DownloadResumeSource::Cdn {
                url,
                cookie_header,
                game_id,
                domain,
                mod_id,
                file_id,
                label,
                version,
                replace_staged_id,
                file_name,
            } => {
                let label_clone = label.clone();
                let game_id_clone = game_id.clone();
                let res = fetch_and_stage_cdn(
                    &app_bg,
                    &*state,
                    &url,
                    &cookie_header,
                    &game_id,
                    &label,
                    &domain,
                    mod_id,
                    file_id,
                    &queued_id,
                    batch_id,
                    version,
                    replace_staged_id,
                    file_name,
                )
                .await;
                (res, label_clone, game_id_clone)
            }
            DownloadResumeSource::Thunderstore { .. } | DownloadResumeSource::Modio { .. } => (
                Err("Cannot resume this download — use Restart instead.".into()),
                String::new(),
                String::new(),
            ),
        };

        match result {
            (Ok(staged), label, game_id) => {
                log::info!("download finished after resume: {label}");
                let _ = app_bg.emit(
                    "download-finished",
                    serde_json::json!({
                        "id": queued_id,
                        "label": label,
                        "game_id": game_id,
                        "mod_id": staged.id,
                    }),
                );
            }
            (Err(e), label, _) => {
                if is_pause_err(&e) {
                    log::info!("download paused: {label}");
                } else if is_cancel_err(&e) {
                    log::info!("download cancelled: {label}");
                    let _ = app_bg.emit(
                        "download-cancelled",
                        serde_json::json!({
                            "id": queued_id,
                            "label": label,
                        }),
                    );
                } else {
                    log::error!("download failed after resume ({label}): {e}");
                    let _ = app_bg.emit(
                        "download-failed",
                        serde_json::json!({
                            "id": queued_id,
                            "label": label,
                            "error": e,
                        }),
                    );
                }
            }
        }
    });

    Ok(())
}

fn take_restart_source(state: &AppState, id: &str) -> Option<DownloadResumeSource> {
    state
        .download_restart_meta
        .lock()
        .ok()
        .and_then(|m| m.get(id).cloned())
        .or_else(|| {
            state
                .download_jobs
                .lock()
                .ok()
                .and_then(|j| j.get(id).and_then(|job| job.source.clone()))
        })
}

async fn emit_download_outcome(
    app: &tauri::AppHandle,
    queued_id: &str,
    label: &str,
    game_id: &str,
    result: Result<StagedMod, String>,
) {
    match result {
        Ok(staged) => {
            log::info!("download finished: {label}");
            let _ = app.emit(
                "download-finished",
                serde_json::json!({
                    "id": queued_id,
                    "label": label,
                    "game_id": game_id,
                    "mod_id": staged.id,
                }),
            );
        }
        Err(e) => {
            if is_pause_err(&e) {
                log::info!("download paused: {label}");
            } else if is_cancel_err(&e) {
                log::info!("download cancelled: {label}");
                let _ = app.emit(
                    "download-cancelled",
                    serde_json::json!({
                        "id": queued_id,
                        "label": label,
                    }),
                );
            } else {
                log::error!("download failed ({label}): {e}");
                let _ = app.emit(
                    "download-failed",
                    serde_json::json!({
                        "id": queued_id,
                        "label": label,
                        "error": e,
                    }),
                );
            }
        }
    }
}

async fn run_modio_download_restart(
    app: &tauri::AppHandle,
    state: &AppState,
    dl_id: &str,
    game_id: &str,
    modio_game_id: u32,
    mod_id: u64,
    file_id: u64,
    label: &str,
    version: Option<String>,
    name_id: &str,
) -> Result<StagedMod, String> {
    let dest = state.paths.downloads_dir().join(format!(
        "modio_{}_{}_{}.zip",
        dl_id,
        sanitize_filename::sanitize(name_id),
        file_id
    ));
    let _ = std::fs::remove_file(&dest);
    reset_download_row(state, dl_id);

    let modio_source = DownloadResumeSource::Modio {
        game_id: game_id.to_string(),
        modio_game_id,
        mod_id,
        file_id,
        name: label.to_string(),
        version: version.clone(),
        name_id: name_id.to_string(),
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));
    {
        let mut jobs = state.download_jobs.lock().map_err(|e| e.to_string())?;
        jobs.insert(
            dl_id.to_string(),
            DownloadJob {
                cancel: cancel.clone(),
                pause: pause.clone(),
                batch_id: None,
                dest: Some(dest.clone()),
                source: Some(modio_source.clone()),
            },
        );
    }
    store_restart_meta(state, dl_id, modio_source);

    let key = state
        .modio_api_key
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or_else(|| "mod.io API key required".to_string())?;
    let client = ModioClient::new(&key).map_err(|e| e.to_string())?;

    let on_progress = progress_callback(Some(app), dl_id.to_string());
    match client
        .download_file(
            modio_game_id,
            mod_id,
            Some(file_id),
            &dest,
            on_progress.as_ref(),
        )
        .await
    {
        Ok(()) => {
            update_download(state, dl_id, "extracting", None);
            let _ = app.emit("downloads-changed", ());
            let modio_label = modio_file_label(
                &client,
                modio_game_id,
                mod_id,
                file_id,
                Some(file_id),
                label,
            )
            .await;
            assign_download_file_name(state, dl_id, modio_label.as_deref());
            match mods::stage_modio_mod(
                &state.paths,
                game_id,
                modio_game_id,
                mod_id,
                file_id,
                label,
                version,
                modio_label,
                None,
                &dest,
            ) {
                Ok(staged) => {
                    update_download(state, dl_id, "done", None);
                    remove_download_archive(&dest, &state.paths.downloads_dir());
                    finish_download_success(state, dl_id);
                    let _ = app.emit("downloads-changed", ());
                    Ok(staged)
                }
                Err(e) => {
                    let msg = e.to_string();
                    remove_download_archive(&dest, &state.paths.downloads_dir());
                    update_download(state, dl_id, "failed", Some(msg.clone()));
                    clear_download_job(state, dl_id);
                    let _ = app.emit("downloads-changed", ());
                    Err(msg)
                }
            }
        }
        Err(e) => {
            let msg = e.to_string();
            remove_download_archive(&dest, &state.paths.downloads_dir());
            update_download(state, dl_id, "failed", Some(msg.clone()));
            clear_download_job(state, dl_id);
            let _ = app.emit("downloads-changed", ());
            Err(msg)
        }
    }
}

#[tauri::command]
pub fn restart_download(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let source = take_restart_source(&state, &id)
        .ok_or_else(|| "No restart metadata for this download".to_string())?;

    if let Ok(mut jobs) = state.download_jobs.lock() {
        if let Some(job) = jobs.get_mut(&id) {
            job.cancel.store(true, Ordering::SeqCst);
            if let Some(dest) = &job.dest {
                let _ = std::fs::remove_file(dest);
            }
        }
    }

    reset_download_row(&state, &id);
    store_restart_meta(&state, &id, source.clone());

    if let Ok(mut jobs) = state.download_jobs.lock() {
        if let Some(job) = jobs.get_mut(&id) {
            job.cancel.store(false, Ordering::SeqCst);
            job.pause.store(false, Ordering::SeqCst);
        }
    }

    let app_bg = app.clone();
    let queued_id = id.clone();

    tauri::async_runtime::spawn(async move {
        let state = app_bg.state::<AppState>();
        let result = match source {
            DownloadResumeSource::Api {
                game_id,
                domain,
                mod_id,
                file_id,
                label,
                version,
                replace_staged_id,
                file_name,
                ..
            } => {
                let label_clone = label.clone();
                let game_id_clone = game_id.clone();
                let batch_id = state
                    .download_jobs
                    .lock()
                    .ok()
                    .and_then(|j| j.get(&queued_id).and_then(|job| job.batch_id.clone()));
                let res = download_and_stage_inner(
                    Some(&app_bg),
                    &*state,
                    &game_id,
                    &domain,
                    mod_id,
                    file_id,
                    &label,
                    version,
                    None,
                    None,
                    Some(queued_id.clone()),
                    batch_id,
                    replace_staged_id,
                    file_name,
                )
                .await;
                (res, label_clone, game_id_clone)
            }
            DownloadResumeSource::Cdn {
                url,
                cookie_header,
                game_id,
                domain,
                mod_id,
                file_id,
                label,
                version,
                replace_staged_id,
                file_name,
            } => {
                let label_clone = label.clone();
                let game_id_clone = game_id.clone();
                let batch_id = state
                    .download_jobs
                    .lock()
                    .ok()
                    .and_then(|j| j.get(&queued_id).and_then(|job| job.batch_id.clone()));
                let res = fetch_and_stage_cdn(
                    &app_bg,
                    &*state,
                    &url,
                    &cookie_header,
                    &game_id,
                    &label,
                    &domain,
                    mod_id,
                    file_id,
                    &queued_id,
                    batch_id,
                    version,
                    replace_staged_id,
                    file_name,
                )
                .await;
                (res, label_clone, game_id_clone)
            }
            DownloadResumeSource::Thunderstore {
                game_id,
                community,
                namespace,
                name,
                display_name,
                enabled,
                ..
            } => {
                let label_clone = display_name.clone();
                let game_id_clone = game_id.clone();
                let res = async {
                    let client = ThunderstoreClient::new().map_err(|e| e.to_string())?;
                    let pkg = client
                        .get_package(&community, &namespace, &name)
                        .await
                        .map_err(|e| e.to_string())?;
                    stage_thunderstore_package(
                        &app_bg,
                        &*state,
                        &client,
                        &game_id,
                        &community,
                        &pkg,
                        &display_name,
                        enabled,
                        Some(queued_id.clone()),
                        StagePolicy::KeepNewer,
                    )
                    .await
                }
                .await;
                (res, label_clone, game_id_clone)
            }
            DownloadResumeSource::Modio {
                game_id,
                modio_game_id,
                mod_id,
                file_id,
                name,
                version,
                name_id,
            } => {
                let label_clone = name.clone();
                let game_id_clone = game_id.clone();
                let res = run_modio_download_restart(
                    &app_bg,
                    &*state,
                    &queued_id,
                    &game_id,
                    modio_game_id,
                    mod_id,
                    file_id,
                    &name,
                    version,
                    &name_id,
                )
                .await;
                (res, label_clone, game_id_clone)
            }
        };

        emit_download_outcome(&app_bg, &queued_id, &result.1, &result.2, result.0).await;
    });

    Ok(())
}

#[tauri::command]
pub fn force_reset_download(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let label = download_label(&state, &id);
    if let Ok(mut jobs) = state.download_jobs.lock() {
        if let Some(job) = jobs.get_mut(&id) {
            job.cancel.store(true, Ordering::SeqCst);
            if let Some(dest) = &job.dest {
                let _ = std::fs::remove_file(dest);
            }
        }
        jobs.remove(&id);
    }
    update_download(&state, &id, "failed", Some("Reset by user".into()));
    clear_restart_meta(&state, &id);
    let _ = app.emit(
        "download-failed",
        serde_json::json!({
            "id": id,
            "label": label,
            "error": "Reset by user",
        }),
    );
    Ok(())
}

#[cfg(test)]
mod download_naming_tests {
    use super::archive_extension;

    #[test]
    fn a_real_upload_name_supplies_the_extension() {
        assert_eq!(
            archive_extension(Some("Cool Mod-1034-1-0.7z")).as_deref(),
            Some("7z")
        );
        // Lowercased so the loose-file check matches.
        assert_eq!(
            archive_extension(Some("Career.PACKAGE")).as_deref(),
            Some("package")
        );
    }

    #[test]
    fn nothing_usable_leaves_the_download_name_alone() {
        assert_eq!(archive_extension(None), None);
        assert_eq!(archive_extension(Some("no-extension")), None);
        // A separator would let a crafted name escape the downloads directory.
        assert_eq!(archive_extension(Some("../../etc/passwd.zip")), None);
        assert_eq!(archive_extension(Some("mod.tar gz")), None);
    }
}

#[cfg(test)]
mod download_cache_cleanup_tests {
    use super::{clear_download_cache_dir, path_is_under_downloads, remove_download_archive};
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;

    fn temp_downloads() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let downloads = tmp.path().join("downloads");
        fs::create_dir_all(&downloads).unwrap();
        (tmp, downloads)
    }

    #[test]
    fn path_under_downloads_accepts_nested_files() {
        let (_tmp, downloads) = temp_downloads();
        let archive = downloads.join("ts_abc_mod.zip");
        fs::write(&archive, b"zip").unwrap();
        assert!(path_is_under_downloads(&archive, &downloads));
    }

    #[test]
    fn path_under_downloads_rejects_outside_paths() {
        let (_tmp, downloads) = temp_downloads();
        let outside = _tmp.path().join("elsewhere.zip");
        fs::write(&outside, b"zip").unwrap();
        assert!(!path_is_under_downloads(&outside, &downloads));
    }

    #[test]
    fn remove_download_archive_only_deletes_under_cache() {
        let (_tmp, downloads) = temp_downloads();
        let inside = downloads.join("keep-me-not.zip");
        let outside = _tmp.path().join("user-import.zip");
        fs::write(&inside, b"a").unwrap();
        fs::write(&outside, b"b").unwrap();

        remove_download_archive(&inside, &downloads);
        remove_download_archive(&outside, &downloads);

        assert!(!inside.exists());
        assert!(outside.exists());
    }

    #[test]
    fn clear_download_cache_skips_protected_active_dest() {
        let (_tmp, downloads) = temp_downloads();
        let stale = downloads.join("stale.zip");
        let active = downloads.join("active.zip");
        fs::write(&stale, b"aaaa").unwrap();
        fs::write(&active, b"bbbbbbbb").unwrap();

        let mut protected = HashSet::new();
        protected.insert(active.clone());

        let result = clear_download_cache_dir(&downloads, &protected).unwrap();
        assert_eq!(result.files_removed, 1);
        assert_eq!(result.bytes_freed, 4);
        assert!(!stale.exists());
        assert!(active.exists());
    }

    #[test]
    fn clear_download_cache_empty_dir_is_noop() {
        let (_tmp, downloads) = temp_downloads();
        let result = clear_download_cache_dir(&downloads, &HashSet::new()).unwrap();
        assert_eq!(result.files_removed, 0);
        assert_eq!(result.bytes_freed, 0);
    }
}

#[cfg(test)]
mod catalog_filter_tests {
    use super::catalog_source_unavailable_error;

    #[test]
    fn no_sources_uses_generic_message() {
        let msg = catalog_source_unavailable_error("all", false, false, false, false);
        assert!(msg.contains("No catalog sources configured"), "{msg}");
    }

    #[test]
    fn filter_mismatch_mentions_configured_source() {
        let msg = catalog_source_unavailable_error("modio", true, false, false, false);
        assert!(msg.contains("mod.io filter"), "{msg}");
        assert!(msg.contains("Nexus"), "{msg}");
        assert!(!msg.contains("No catalog sources configured"), "{msg}");
    }

    #[test]
    fn collections_no_sources() {
        let msg = catalog_source_unavailable_error("all", false, false, false, true);
        assert!(msg.contains("Collections require"), "{msg}");
    }

    #[test]
    fn collections_filter_mismatch() {
        let msg = catalog_source_unavailable_error("thunderstore", true, false, false, true);
        assert!(msg.contains("Thunderstore filter"), "{msg}");
        assert!(msg.contains("Nexus"), "{msg}");
    }
}

#[cfg(test)]
mod recent_download_limit_tests {
    use super::{trim_finished_downloads, DownloadItem};

    fn row(id: &str, status: &str) -> DownloadItem {
        DownloadItem::new(id.into(), id.into(), status, None, None)
    }

    #[test]
    fn limit_drops_oldest_finished_and_keeps_active() {
        let mut q = vec![
            row("new-done", "done"),
            row("active", "downloading"),
            row("mid", "failed"),
            row("paused", "paused"),
            row("old", "cancelled"),
        ];
        trim_finished_downloads(&mut q, 2);
        let ids: Vec<_> = q.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["new-done", "active", "mid", "paused"]);
    }

    #[test]
    fn shorter_list_is_left_untouched() {
        let mut q = vec![row("a", "done"), row("b", "downloading")];
        trim_finished_downloads(&mut q, 5);
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].id, "a");
        assert_eq!(q[1].status, "downloading");
    }
}
