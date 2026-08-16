//! Tauri IPC commands.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    assist::{AssistBounds, AssistContext},
    config::{self, AppConfig, ManagedGame, Paths, APP_NAME, APP_VERSION},
    detection::{self, DetectedGame},
    games, migration,
    modio_api::{self, ModioClient, ModioFileInfo, ModioModDetail},
    mods::{self, StagedMod},
    nexus::{
        self, stream_url_to_file, CollectionDetail, CollectionModFile, GameInfo, ModDetail,
        ModFileInfo, NexusClient, NexusUser, TransferControl, CANCELLED_MSG, PAUSED_MSG,
    },
    thunderstore::{self, ThunderstoreClient, TsPackageDetail},
};

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
    },
    Cdn {
        url: String,
        cookie_header: String,
        game_id: String,
        domain: String,
        mod_id: u64,
        file_id: u64,
        label: String,
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
    /// Cached Nexus game metadata (categories, counts) keyed by domain.
    pub game_info_cache: Mutex<HashMap<String, GameInfo>>,
    /// Cached Thunderstore community package lists.
    pub ts_package_cache: Mutex<HashMap<String, (Instant, Vec<thunderstore::TsPackage>)>>,
    /// Session cache of Nexus games list for catalog matching.
    pub nexus_games_cache: Mutex<Option<Vec<nexus::NexusGameEntry>>>,
    /// Session cache of Thunderstore communities for catalog matching.
    pub ts_communities_cache: Mutex<Option<Vec<thunderstore::TsCommunity>>>,
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
        Ok(Self {
            paths,
            config: Mutex::new(config),
            api_key: Mutex::new(api_key),
            modio_api_key: Mutex::new(modio_api_key),
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
            game_info_cache: Mutex::new(HashMap::new()),
            ts_package_cache: Mutex::new(HashMap::new()),
            nexus_games_cache: Mutex::new(None),
            ts_communities_cache: Mutex::new(None),
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
            if status == "cancelled"
                || status == "staged"
                || status == "failed"
                || status == "paused"
            {
                item.speed_bps = 0;
            }
        }
    }
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
    if let Ok(mut jobs) = state.download_jobs.lock() {
        if let Some(job) = jobs.get_mut(id) {
            job.dest = Some(dest);
            job.source = Some(source);
        }
    }
}

fn clear_download_job(state: &AppState, id: &str) {
    if let Ok(mut jobs) = state.download_jobs.lock() {
        jobs.remove(id);
    }
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

fn request_cancel_ids(state: &AppState, ids: &[String]) {
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

fn file_offset(path: &PathBuf) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
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
        title,
        nexus_domain: domain,
        install_path,
        launcher,
        plugin_id,
        cover_path,
        project_name,
        thunderstore_community: ts_community,
        modio_game_id: modio_id,
    };
    let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
    cfg.managed_games.retain(|g| g.id != id);
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
    let has_modio_api_key = state
        .modio_api_key
        .lock()
        .map_err(|e| e.to_string())?
        .is_some();
    let user = state.user.lock().map_err(|e| e.to_string())?.clone();
    Ok(serde_json::json!({
        "adult_content": cfg.adult_content,
        "autoclick_free_download": cfg.autoclick_free_download,
        "last_active_game_id": cfg.last_active_game_id,
        "theme": cfg.theme,
        "install_click_behavior": cfg.install_click_behavior,
        "has_api_key": has_key,
        "has_modio_api_key": has_modio_api_key,
        "user": user,
        "config_dir": state.paths.config_dir,
        "data_dir": state.paths.data_dir,
        "cache_dir": state.paths.cache_dir,
    }))
}

#[tauri::command]
pub fn scan_mod_orphans(state: State<'_, AppState>) -> Result<migration::OrphanScan, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    Ok(migration::scan_orphans(&state.paths, &cfg))
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
        return Err(
            "No catalog sources configured. Set a Nexus domain, Thunderstore community, and/or mod.io game ID."
                .into(),
        );
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

#[tauri::command]
pub async fn download_thunderstore_mod(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    game_id: String,
    community: String,
    namespace: String,
    name: String,
    version: Option<String>,
) -> Result<Vec<StagedMod>, String> {
    let client = ThunderstoreClient::new().map_err(|e| e.to_string())?;
    let order = client
        .resolve_install_order(&community, &namespace, &name, version.as_deref())
        .await
        .map_err(|e| e.to_string())?;

    let mut staged_all = Vec::new();
    for pkg in order {
        let ns = pkg.namespace.clone();
        let pkg_name = pkg.name.clone();
        if mods::has_thunderstore_package(&state.paths, &game_id, &ns, &pkg_name)
            .map_err(|e| e.to_string())?
        {
            continue;
        }
        let ver = pkg
            .versions
            .first()
            .ok_or_else(|| format!("No versions for {}", pkg.full_name))?;
        let label = format!("{}-{}", pkg.full_name, ver.version_number);
        let dl_id = uuid::Uuid::new_v4().to_string();
        let dest = state.paths.downloads_dir().join(format!(
            "ts_{}_{}.zip",
            dl_id,
            sanitize_filename::sanitize(&pkg_name)
        ));

        {
            let mut downloads = state.downloads.lock().map_err(|e| e.to_string())?;
            downloads.insert(
                0,
                DownloadItem::new(dl_id.clone(), label.clone(), "downloading", None),
            );
        }
        let _ = app.emit("downloads-changed", ());

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
                    source: None,
                },
            );
        }

        let control = TransferControl {
            cancel: cancel.clone(),
            pause: pause.clone(),
        };
        match client.download_version(ver, &dest, Some(&control)).await {
            Ok(()) => {
                let display = if pkg.name == name && pkg.namespace == namespace {
                    pkg.name.clone()
                } else {
                    format!("{} (dependency)", pkg.name)
                };
                let staged = mods::stage_thunderstore_mod(
                    &state.paths,
                    &game_id,
                    &community,
                    &ns,
                    &pkg_name,
                    &ver.version_number,
                    Some(pkg.uuid4.clone()),
                    &display,
                    &dest,
                )
                .map_err(|e| e.to_string())?;
                staged_all.push(staged);
                if let Ok(mut downloads) = state.downloads.lock() {
                    if let Some(d) = downloads.iter_mut().find(|d| d.id == dl_id) {
                        d.status = "done".into();
                    }
                }
                clear_download_job(&state, &dl_id);
                let _ = app.emit("downloads-changed", ());
            }
            Err(e) => {
                let msg = e.to_string();
                if let Ok(mut downloads) = state.downloads.lock() {
                    if let Some(d) = downloads.iter_mut().find(|d| d.id == dl_id) {
                        d.status = "failed".into();
                        d.error = Some(msg.clone());
                    }
                }
                clear_download_job(&state, &dl_id);
                let _ = app.emit("downloads-changed", ());
                return Err(msg);
            }
        }
    }
    Ok(staged_all)
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

    let mut queue: Vec<(u64, String, Option<u64>, Option<String>)> = Vec::new();
    if install_deps {
        match client.dependency_mod_ids(modio_game_id, mod_id).await {
            Ok(deps) => {
                for dep_id in deps {
                    if mods::has_modio_mod(&state.paths, &game_id, dep_id)
                        .map_err(|e| e.to_string())?
                    {
                        continue;
                    }
                    let detail = client
                        .get_mod(modio_game_id, dep_id)
                        .await
                        .map_err(|e| e.to_string())?;
                    queue.push((
                        dep_id,
                        format!("{} (dependency)", detail.name),
                        detail.primary_file_id,
                        None,
                    ));
                }
            }
            Err(e) => log::warn!("mod.io deps lookup failed: {e}"),
        }
    }
    queue.push((mod_id, name, file_id, version));

    let mut staged_all = Vec::new();
    for (mid, label, fid, ver) in queue {
        if mods::has_modio_mod(&state.paths, &game_id, mid).map_err(|e| e.to_string())?
            && mid != mod_id
        {
            continue;
        }
        let detail = client
            .get_mod(modio_game_id, mid)
            .await
            .map_err(|e| e.to_string())?;
        let resolved_file = fid
            .or(detail.primary_file_id)
            .ok_or_else(|| format!("No downloadable file for mod.io mod {mid}"))?;
        let dl_id = uuid::Uuid::new_v4().to_string();
        let dest = state.paths.downloads_dir().join(format!(
            "modio_{}_{}_{}.zip",
            dl_id,
            sanitize_filename::sanitize(&detail.name_id),
            resolved_file
        ));
        {
            let mut downloads = state.downloads.lock().map_err(|e| e.to_string())?;
            downloads.insert(
                0,
                DownloadItem::new(dl_id.clone(), label.clone(), "downloading", None),
            );
        }
        let _ = app.emit("downloads-changed", ());
        match client
            .download_file(modio_game_id, mid, Some(resolved_file), &dest)
            .await
        {
            Ok(()) => {
                let staged = mods::stage_modio_mod(
                    &state.paths,
                    &game_id,
                    modio_game_id,
                    mid,
                    resolved_file,
                    &detail.name,
                    ver,
                    &dest,
                )
                .map_err(|e| e.to_string())?;
                staged_all.push(staged);
                if let Ok(mut downloads) = state.downloads.lock() {
                    if let Some(d) = downloads.iter_mut().find(|d| d.id == dl_id) {
                        d.status = "done".into();
                    }
                }
                clear_download_job(&state, &dl_id);
                let _ = app.emit("downloads-changed", ());
            }
            Err(e) => {
                let msg = e.to_string();
                if let Ok(mut downloads) = state.downloads.lock() {
                    if let Some(d) = downloads.iter_mut().find(|d| d.id == dl_id) {
                        d.status = "failed".into();
                        d.error = Some(msg.clone());
                    }
                }
                clear_download_job(&state, &dl_id);
                let _ = app.emit("downloads-changed", ());
                return Err(msg);
            }
        }
    }
    Ok(staged_all)
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
        },
    );

    let start_offset = file_offset(&dest);
    let control = TransferControl {
        cancel: cancel.clone(),
        pause: pause.clone(),
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
            start_offset,
            Some(&control),
            on_progress.as_ref(),
        )
        .await;

    match download_result {
        Ok(()) => {}
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
    if pause.load(Ordering::SeqCst) {
        mark_paused(app, state, &dl_id);
        return Err("Paused".into());
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
            Err(e) if is_cancel_err(&e) || is_pause_err(&e) => break,
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
    let item = DownloadItem::new(dl_id.clone(), name.clone(), "downloading", batch_id.clone());
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
        },
    );

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

    log::info!("cdn download saved to {}", dest.display());

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
        .unwrap_or_else(|| (file_offset(&dest), Some(file_offset(&dest))));

    update_download(state, dl_id, "extracting", None);
    report_download_progress(app, state, dl_id, bytes_downloaded, bytes_total, 0);
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
    let (cancel, _pause) = ensure_download_job(&state, &dl_id, batch_id);
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
        game.project_name.as_deref(),
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
    request_cancel_ids(&state, &ids);
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
        let source = job
            .source
            .as_ref()
            .ok_or_else(|| "Missing resume metadata for this download".to_string())?;
        // Clone source fields without moving enum yet — we'll take ownership below.
        let source = match source {
            DownloadResumeSource::Api {
                game_id,
                domain,
                mod_id,
                file_id,
                label,
                version,
                nxm_key,
                nxm_expires,
            } => DownloadResumeSource::Api {
                game_id: game_id.clone(),
                domain: domain.clone(),
                mod_id: *mod_id,
                file_id: *file_id,
                label: label.clone(),
                version: version.clone(),
                nxm_key: nxm_key.clone(),
                nxm_expires: *nxm_expires,
            },
            DownloadResumeSource::Cdn {
                url,
                cookie_header,
                game_id,
                domain,
                mod_id,
                file_id,
                label,
            } => DownloadResumeSource::Cdn {
                url: url.clone(),
                cookie_header: cookie_header.clone(),
                game_id: game_id.clone(),
                domain: domain.clone(),
                mod_id: *mod_id,
                file_id: *file_id,
                label: label.clone(),
            },
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
                )
                .await;
                (res, label_clone, game_id_clone)
            }
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
