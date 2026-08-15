//! Tauri IPC commands.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use tauri::{Emitter, Manager, State};

use crate::{
    assist::{AssistBounds, AssistContext},
    config::{self, AppConfig, ManagedGame, Paths, APP_NAME, APP_VERSION},
    detection::{self, DetectedGame},
    games,
    mods::{self, StagedMod},
    nexus::{
        self, CollectionHit, CollectionModFile, ModDetail, ModFileInfo, ModSearchHit, NexusClient,
        NexusUser, TransferControl,
    },
};

const CANCELLED_MSG: &str = "CANCELLED";

pub struct DownloadJob {
    pub cancel: Arc<AtomicBool>,
    pub batch_id: Option<String>,
}

pub struct AppState {
    pub paths: Paths,
    pub config: Mutex<AppConfig>,
    pub api_key: Mutex<Option<String>>,
    pub user: Mutex<Option<NexusUser>>,
    pub downloads: Mutex<Vec<DownloadItem>>,
    pub download_jobs: Mutex<HashMap<String, DownloadJob>>,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadItem {
    pub id: String,
    pub label: String,
    pub status: String,
    pub error: Option<String>,
    pub bytes_downloaded: u64,
    pub bytes_total: Option<u64>,
    pub speed_bps: u64,
    pub batch_id: Option<String>,
}

impl DownloadItem {
    fn new(id: String, label: String, status: &str, batch_id: Option<String>) -> Self {
        Self {
            id,
            label,
            status: status.into(),
            error: None,
            bytes_downloaded: 0,
            bytes_total: None,
            speed_bps: 0,
            batch_id,
        }
    }
}

impl AppState {
    pub fn new() -> anyhow::Result<Self> {
        let paths = Paths::resolve()?;
        let config = config::load_config(&paths)?;
        let api_key = load_key_with_fallback(&paths)?;
        Ok(Self {
            paths,
            config: Mutex::new(config),
            api_key: Mutex::new(api_key),
            user: Mutex::new(None),
            downloads: Mutex::new(Vec::new()),
            download_jobs: Mutex::new(HashMap::new()),
            assist: Mutex::new(None),
            assist_window_gen: AtomicU64::new(0),
            assist_closed_gen: AtomicU64::new(0),
            last_nxm_emit: Mutex::new(None),
            last_nxm_handled: Mutex::new(None),
            last_cdn_handled: Mutex::new(None),
            assist_bounds: Mutex::new(AssistBounds::default()),
            assist_desired_visible: AtomicBool::new(false),
        })
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

fn push_download(state: &AppState, item: DownloadItem) {
    if let Ok(mut q) = state.downloads.lock() {
        q.insert(0, item);
        if q.len() > 100 {
            q.truncate(100);
        }
    }
}

fn update_download(state: &AppState, id: &str, status: &str, error: Option<String>) {
    if let Ok(mut q) = state.downloads.lock() {
        if let Some(item) = q.iter_mut().find(|d| d.id == id) {
            item.status = status.to_string();
            item.error = error;
            if status == "cancelled" || status == "staged" || status == "failed" {
                item.speed_bps = 0;
            }
        }
    }
}

fn ensure_download_job(
    state: &AppState,
    id: &str,
    batch_id: Option<String>,
) -> Arc<AtomicBool> {
    if let Ok(mut jobs) = state.download_jobs.lock() {
        if let Some(job) = jobs.get(id) {
            return job.cancel.clone();
        }
        let cancel = Arc::new(AtomicBool::new(false));
        jobs.insert(
            id.to_string(),
            DownloadJob {
                cancel: cancel.clone(),
                batch_id,
            },
        );
        return cancel;
    }
    Arc::new(AtomicBool::new(false))
}

fn clear_download_job(state: &AppState, id: &str) {
    if let Ok(mut jobs) = state.download_jobs.lock() {
        jobs.remove(id);
    }
}

fn mark_cancelled(state: &AppState, id: &str) {
    update_download(state, id, "cancelled", Some("Cancelled".into()));
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
            if item.status == "cancelled" {
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

fn request_cancel_ids(state: &AppState, ids: &[String]) {
    if let Ok(mut jobs) = state.download_jobs.lock() {
        for id in ids {
            if let Some(job) = jobs.get_mut(id) {
                job.cancel.store(true, Ordering::SeqCst);
            }
            mark_cancelled(state, id);
        }
    } else {
        for id in ids {
            mark_cancelled(state, id);
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

#[tauri::command]
pub fn get_app_info() -> serde_json::Value {
    serde_json::json!({
        "name": APP_NAME,
        "version": env!("CARGO_PKG_VERSION"),
        "linux_only": true,
    })
}

#[tauri::command]
pub fn scan_games() -> Result<Vec<DetectedGame>, String> {
    Ok(detection::scan_games())
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
) -> Result<ManagedGame, String> {
    if install_path.is_empty() {
        return Err("Install path is required".into());
    }
    if games::plugin_by_id(&plugin_id).is_none() {
        return Err(format!("Unknown plugin: {plugin_id}"));
    }
    config::ensure_game_dirs(&state.paths, &id).map_err(|e| e.to_string())?;
    let managed = ManagedGame {
        id: id.clone(),
        title,
        nexus_domain,
        install_path,
        launcher,
        plugin_id,
        cover_path,
    };
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.managed_games.retain(|g| g.id != id);
    cfg.managed_games.push(managed.clone());
    cfg.last_active_game_id = Some(id);
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())?;
    Ok(managed)
}

#[tauri::command]
pub fn unmanage_game(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.managed_games.retain(|g| g.id != id);
    if cfg.last_active_game_id.as_deref() == Some(&id) {
        cfg.last_active_game_id = cfg.managed_games.first().map(|g| g.id.clone());
    }
    config::save_config(&state.paths, &cfg).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    let has_key = state.api_key.lock().map_err(|e| e.to_string())?.is_some();
    let user = state.user.lock().map_err(|e| e.to_string())?.clone();
    Ok(serde_json::json!({
        "adult_content": cfg.adult_content,
        "autoclick_free_download": cfg.autoclick_free_download,
        "last_active_game_id": cfg.last_active_game_id,
        "theme": cfg.theme,
        "has_api_key": has_key,
        "user": user,
        "config_dir": state.paths.config_dir,
        "data_dir": state.paths.data_dir,
        "cache_dir": state.paths.cache_dir,
    }))
}

#[tauri::command]
pub fn set_adult_content(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.adult_content = enabled;
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
pub async fn search_mods(
    state: State<'_, AppState>,
    domain: String,
    query: String,
    sort: Option<String>,
    category: Option<String>,
    tags: Option<Vec<String>>,
    game_version: Option<String>,
) -> Result<Vec<ModSearchHit>, String> {
    let adult = state
        .config
        .lock()
        .map_err(|e| e.to_string())?
        .adult_content;
    let client = state.client()?;
    let opts = nexus::BrowseSearchOpts {
        sort: sort.unwrap_or_else(|| "endorsements".into()),
        category,
        tags: tags.unwrap_or_default(),
        game_version,
    };
    client
        .search_mods(&domain, &query, adult, &opts)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_mod(
    state: State<'_, AppState>,
    domain: String,
    mod_id: u64,
) -> Result<ModDetail, String> {
    let client = state.client()?;
    client
        .get_mod(&domain, mod_id)
        .await
        .map_err(|e| e.to_string())
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
    tags: Option<Vec<String>>,
    game_version: Option<String>,
) -> Result<Vec<CollectionHit>, String> {
    let adult = state
        .config
        .lock()
        .map_err(|e| e.to_string())?
        .adult_content;
    let client = state.client()?;
    let opts = nexus::BrowseSearchOpts {
        sort: sort.unwrap_or_else(|| "endorsements".into()),
        category,
        tags: tags.unwrap_or_default(),
        game_version,
    };
    client
        .search_collections(&domain, &query, adult, &opts)
        .await
        .map_err(|e| e.to_string())
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
            DownloadItem::new(dl_id.clone(), label.to_string(), "downloading", batch_id.clone()),
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

    let cancel = ensure_download_job(state, &dl_id, batch_id.clone());
    if cancel.load(Ordering::SeqCst) {
        mark_cancelled(state, &dl_id);
        clear_download_job(state, &dl_id);
        return Err("Cancelled".into());
    }

    let dest = state
        .paths
        .downloads_dir()
        .join(format!("{domain}_{mod_id}_{file_id}.bin"));

    let control = TransferControl {
        cancel: cancel.clone(),
    };
    let on_progress = progress_callback(app, dl_id.clone());

    let download_result = client
        .download_file(
            domain,
            mod_id,
            file_id,
            &dest,
            is_premium,
            nxm_key.as_deref(),
            nxm_expires,
            Some(&control),
            on_progress.as_ref(),
        )
        .await;

    match download_result {
        Ok(()) => {}
        Err(e) => {
            let msg = e.to_string();
            if is_cancel_err(&msg) || cancel.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&dest);
                mark_cancelled(state, &dl_id);
                clear_download_job(state, &dl_id);
                return Err("Cancelled".into());
            }
            update_download(state, &dl_id, "failed", Some(msg.clone()));
            clear_download_job(state, &dl_id);
            return Err(msg);
        }
    }

    if cancel.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(&dest);
        mark_cancelled(state, &dl_id);
        clear_download_job(state, &dl_id);
        return Err("Cancelled".into());
    }

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

    match mods::stage_mod(
        &state.paths,
        game_id,
        label,
        domain,
        mod_id,
        file_id,
        version,
        &dest,
    ) {
        Ok(staged) => {
            if cancel.load(Ordering::SeqCst) {
                mark_cancelled(state, &dl_id);
                clear_download_job(state, &dl_id);
                return Err("Cancelled".into());
            }
            update_download(state, &dl_id, "staged", None);
            let _ = std::fs::remove_file(&dest);
            clear_download_job(state, &dl_id);
            Ok(staged)
        }
        Err(e) => {
            let msg = e.to_string();
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
    let mut installed = 0usize;
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
        )
        .await
        {
            Ok(_) => installed += 1,
            Err(e) if is_cancel_err(&e) => break,
            Err(e) => return Err(e),
        }
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
    let item = DownloadItem::new(
        dl_id.clone(),
        name.clone(),
        "downloading",
        batch_id.clone(),
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

    tauri::async_runtime::spawn(async move {
        let state = app_bg.state::<AppState>();
        let result = download_and_stage_inner(
            Some(&app_bg),
            &*state,
            &game_id,
            &domain,
            mod_id,
            file_id,
            &label,
            None,
            key,
            expires,
            Some(queued_id.clone()),
            batch_id_bg,
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
                if is_cancel_err(&e) {
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

    mods::stage_mod(
        &state.paths,
        &game_id,
        &label,
        &domain,
        mod_id,
        file_id,
        None,
        &archive,
    )
    .map_err(|e| e.to_string())
}

fn filename_from_cdn_url(cdn_url: &str) -> String {
    url::Url::parse(cdn_url)
        .ok()
        .and_then(|u| {
            u.path()
                .rsplit('/')
                .find(|s| !s.is_empty())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty())
        .map(|s| sanitize_filename::sanitize(s))
        .unwrap_or_else(|| "nexus-download.zip".into())
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
    let item = DownloadItem::new(
        dl_id.clone(),
        ctx.label.clone(),
        "downloading",
        batch_id.clone(),
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
    let queued_id = dl_id.clone();
    let cdn_url = cdn_url.to_string();
    let cookie_header = cookie_header;
    let batch_id_bg = batch_id.clone();

    tauri::async_runtime::spawn(async move {
        let state = app_bg.state::<AppState>();
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
                if is_cancel_err(&e) {
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
) -> Result<StagedMod, String> {
    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;

    let cancel = ensure_download_job(state, dl_id, batch_id);
    if cancel.load(Ordering::SeqCst) {
        mark_cancelled(state, dl_id);
        clear_download_job(state, dl_id);
        return Err("Cancelled".into());
    }

    let client = reqwest::Client::builder()
        .user_agent(format!("{APP_NAME}/{APP_VERSION}"))
        .build()
        .map_err(|e| e.to_string())?;

    let mut req = client.get(cdn_url);
    if !cookie_header.is_empty() {
        req = req.header("Cookie", cookie_header);
    }

    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    log::info!("cdn download HTTP {status} for {cdn_url}");
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        update_download(
            state,
            dl_id,
            "failed",
            Some(format!("HTTP {status}: {body}")),
        );
        clear_download_job(state, dl_id);
        return Err(format!("CDN download failed ({status})"));
    }

    let total = resp.content_length();
    let mut filename = resp
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.split("filename=")
                .nth(1)
                .map(|s| s.trim_matches('"').trim_matches('\''))
        })
        .map(sanitize_filename::sanitize)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| filename_from_cdn_url(cdn_url));

    let dest = state.paths.downloads_dir().join(&filename);
    if dest.exists() {
        let stem = dest
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "nexus-download".into());
        let ext = dest
            .extension()
            .map(|s| format!(".{}", s.to_string_lossy()))
            .unwrap_or_default();
        filename = sanitize_filename::sanitize(format!("{stem}-{}", &dl_id[..8])) + &ext;
    }
    let dest = state.paths.downloads_dir().join(&filename);

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let mut file = tokio::fs::File::create(&dest)
        .await
        .map_err(|e| e.to_string())?;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    let started = Instant::now();
    let mut last_report = started;
    while let Some(chunk) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            drop(file);
            let _ = tokio::fs::remove_file(&dest).await;
            mark_cancelled(state, dl_id);
            clear_download_job(state, dl_id);
            return Err("Cancelled".into());
        }
        let chunk = chunk.map_err(|e| e.to_string())?;
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        let now = Instant::now();
        if now.duration_since(last_report).as_millis() >= 150
            || downloaded == total.unwrap_or(u64::MAX)
        {
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            let speed = (downloaded as f64 / elapsed) as u64;
            report_download_progress(app, state, dl_id, downloaded, total, speed);
            last_report = now;
        }
    }
    file.flush().await.map_err(|e| e.to_string())?;
    log::info!("cdn download saved to {}", dest.display());

    if cancel.load(Ordering::SeqCst) {
        let _ = std::fs::remove_file(&dest);
        mark_cancelled(state, dl_id);
        clear_download_job(state, dl_id);
        return Err("Cancelled".into());
    }

    update_download(state, dl_id, "extracting", None);
    report_download_progress(app, state, dl_id, downloaded, total.or(Some(downloaded)), 0);
    match mods::stage_mod(
        &state.paths,
        game_id,
        label,
        domain,
        mod_id,
        file_id,
        None,
        &dest,
    ) {
        Ok(staged) => {
            if cancel.load(Ordering::SeqCst) {
                mark_cancelled(state, dl_id);
                clear_download_job(state, dl_id);
                return Err("Cancelled".into());
            }
            update_download(state, dl_id, "staged", None);
            let _ = std::fs::remove_file(&dest);
            clear_download_job(state, dl_id);
            Ok(staged)
        }
        Err(e) => {
            let msg = e.to_string();
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
) -> Result<StagedMod, String> {
    let ctx = crate::assist::peek_assist_context(&state)
        .ok_or_else(|| "No active Download Assist context".to_string())?;
    let dl_id = uuid::Uuid::new_v4().to_string();
    let batch_id = ctx.batch_id.clone();
    push_download(
        &state,
        DownloadItem::new(
            dl_id.clone(),
            ctx.label.clone(),
            "extracting",
            batch_id.clone(),
        ),
    );
    let cancel = ensure_download_job(&state, &dl_id, batch_id);
    log::info!("slow download queued: {} id={dl_id}", ctx.label);
    if cancel.load(Ordering::SeqCst) {
        mark_cancelled(&state, &dl_id);
        clear_download_job(&state, &dl_id);
        return Err("Cancelled".into());
    }
    match mods::stage_mod(
        &state.paths,
        &ctx.game_id,
        &ctx.label,
        &ctx.domain,
        ctx.mod_id,
        ctx.file_id,
        None,
        &PathBuf::from(&path),
    ) {
        Ok(staged) => {
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
            Ok(staged)
        }
        Err(e) => {
            let msg = e.to_string();
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

#[tauri::command]
pub fn list_mods(state: State<'_, AppState>, game_id: String) -> Result<Vec<StagedMod>, String> {
    let mut order = mods::load_loadorder(&state.paths, &game_id).map_err(|e| e.to_string())?;
    order.mods.sort_by_key(|m| m.order);
    Ok(order.mods)
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
    mods::remove_all_mods(&state.paths, &game_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn deploy_mods(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<mods::DeployResult, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    let game = cfg
        .managed_games
        .iter()
        .find(|g| g.id == game_id)
        .cloned()
        .ok_or_else(|| "Managed game not found".to_string())?;
    drop(cfg);
    mods::deploy(
        &state.paths,
        &game_id,
        &game.plugin_id,
        &PathBuf::from(&game.install_path),
    )
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn purge_mods(state: State<'_, AppState>, game_id: String) -> Result<(), String> {
    mods::purge_deploy(&state.paths, &game_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_downloads(state: State<'_, AppState>) -> Result<Vec<DownloadItem>, String> {
    Ok(state.downloads.lock().map_err(|e| e.to_string())?.clone())
}

#[tauri::command]
pub fn cancel_download(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let active = state
        .downloads
        .lock()
        .ok()
        .and_then(|q| {
            q.iter()
                .find(|d| d.id == id)
                .map(|d| d.status == "downloading" || d.status == "extracting")
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
    request_cancel_ids(&state, &[id]);
    Ok(())
}

#[tauri::command]
pub fn cancel_download_batch(state: State<'_, AppState>, batch_id: String) -> Result<(), String> {
    let ids: Vec<String> = {
        let downloads = state.downloads.lock().map_err(|e| e.to_string())?;
        let jobs = state.download_jobs.lock().map_err(|e| e.to_string())?;
        let mut ids: Vec<String> = downloads
            .iter()
            .filter(|d| {
                d.batch_id.as_deref() == Some(batch_id.as_str())
                    && (d.status == "downloading" || d.status == "extracting")
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
    request_cancel_ids(&state, &ids);
    Ok(())
}
