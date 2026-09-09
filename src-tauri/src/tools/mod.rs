//! Vulkan tool integration (lsfg-vk, AutoHDR-VK).

mod autohdr_vk;
mod executables;
mod lsfg_vk;
mod manifest;

#[cfg(target_os = "linux")]
mod install;
#[cfg(target_os = "linux")]
mod steam_launch;
#[cfg(target_os = "linux")]
mod sync;

pub use autohdr_vk::{AutoHdrVkConfig, AutoHdrVkConfigDto, AutoHdrVkGlobal, AutoHdrVkProfile};
pub use lsfg_vk::{LsfgVkConfig, LsfgVkConfigDto, LsfgVkGlobal, LsfgVkProfile};
pub use manifest::{ToolInstallRecord, ToolsManifest};

use serde::Serialize;
use tauri::State;

use crate::{
    commands::AppState,
    config::GameToolOverrides,
};

#[derive(Debug, Clone, Serialize)]
pub struct ToolStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub latest_version: Option<String>,
    pub update_available: bool,
    pub config_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolsStatusResponse {
    pub platform_linux: bool,
    pub lsfg_vk: ToolStatus,
    pub autohdr_vk: ToolStatus,
    pub lsfg_vk_repo: String,
    pub autohdr_vk_repo: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GameToolsResponse {
    pub overrides: GameToolOverrides,
    pub executables: Vec<String>,
    pub steam_app_id: Option<String>,
    pub launch_options: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolUpdatesResponse {
    pub lsfg_vk: ToolStatus,
    pub autohdr_vk: ToolStatus,
}

#[cfg(target_os = "linux")]
fn tool_status_local(
    manifest: &ToolsManifest,
    tool_key: &str,
    config_path: std::path::PathBuf,
) -> ToolStatus {
    tool_status(manifest, tool_key, config_path, None)
}

#[cfg(target_os = "linux")]
fn tool_status(
    manifest: &ToolsManifest,
    tool_key: &str,
    config_path: std::path::PathBuf,
    latest: Option<&str>,
) -> ToolStatus {
    let record = match tool_key {
        "lsfg_vk" => manifest.lsfg_vk.as_ref(),
        "autohdr_vk" => manifest.autohdr_vk.as_ref(),
        _ => None,
    };
    let installed = record.is_some() || config_path.exists();
    let version = record.map(|r| r.version.clone());
    let path = record.map(|r| r.path.clone());
    let update_available = match (&version, latest) {
        (Some(installed), Some(latest)) => installed != latest,
        (None, Some(_)) => true,
        _ => false,
    };
    ToolStatus {
        installed,
        version,
        path,
        latest_version: latest.map(|s| s.to_string()),
        update_available,
        config_path: config_path.display().to_string(),
    }
}

fn stub_tool_status(config_path: String) -> ToolStatus {
    ToolStatus {
        installed: false,
        version: None,
        path: None,
        latest_version: None,
        update_available: false,
        config_path,
    }
}

#[tauri::command]
pub fn get_tools_status(state: State<'_, AppState>) -> Result<ToolsStatusResponse, String> {
    let (lsfg_vk_repo, autohdr_vk_repo) = {
        let cfg = state.config.lock().map_err(|e| e.to_string())?;
        (cfg.tools.lsfg_vk_repo.clone(), cfg.tools.autohdr_vk_repo.clone())
    };
    #[cfg(target_os = "linux")]
    {
        let manifest = ToolsManifest::load(&state.paths.tools_manifest_file())
            .map_err(|e| e.to_string())?;
        Ok(ToolsStatusResponse {
            platform_linux: true,
            lsfg_vk: tool_status_local(
                &manifest,
                "lsfg_vk",
                state.paths.lsfg_vk_config_file(),
            ),
            autohdr_vk: tool_status_local(
                &manifest,
                "autohdr_vk",
                state.paths.autohdr_vk_config_file(),
            ),
            lsfg_vk_repo,
            autohdr_vk_repo,
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(ToolsStatusResponse {
            platform_linux: false,
            lsfg_vk: stub_tool_status(state.paths.lsfg_vk_config_file().display().to_string()),
            autohdr_vk: stub_tool_status(
                state.paths.autohdr_vk_config_file().display().to_string(),
            ),
            lsfg_vk_repo,
            autohdr_vk_repo,
        })
    }
}

#[tauri::command]
pub async fn check_tool_updates(state: State<'_, AppState>) -> Result<ToolUpdatesResponse, String> {
    #[cfg(not(target_os = "linux"))]
    {
        return Err("Tools are only supported on Linux".to_string());
    }
    #[cfg(target_os = "linux")]
    {
        let (lsfg_vk_repo, autohdr_vk_repo) = {
            let cfg = state.config.lock().map_err(|e| e.to_string())?;
            (cfg.tools.lsfg_vk_repo.clone(), cfg.tools.autohdr_vk_repo.clone())
        };
        let manifest = ToolsManifest::load(&state.paths.tools_manifest_file())
            .map_err(|e| e.to_string())?;
        let lsfg_latest = crate::github_release::fetch_latest_release(&lsfg_vk_repo)
            .await
            .map_err(|e| e.to_string())?
            .map(|r| r.tag_name);
        let hdr_latest = crate::github_release::fetch_latest_release(&autohdr_vk_repo)
            .await
            .map_err(|e| e.to_string())?
            .map(|r| r.tag_name);
        Ok(ToolUpdatesResponse {
            lsfg_vk: tool_status(
                &manifest,
                "lsfg_vk",
                state.paths.lsfg_vk_config_file(),
                lsfg_latest.as_deref(),
            ),
            autohdr_vk: tool_status(
                &manifest,
                "autohdr_vk",
                state.paths.autohdr_vk_config_file(),
                hdr_latest.as_deref(),
            ),
        })
    }
}

#[tauri::command]
pub async fn install_tool(
    state: State<'_, AppState>,
    tool: String,
) -> Result<ToolsStatusResponse, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (state, tool);
        return Err("Tools are only supported on Linux".to_string());
    }
    #[cfg(target_os = "linux")]
    {
        let (repo, tool_key, install_dir, config_path) = {
            let cfg = state.config.lock().map_err(|e| e.to_string())?;
            let repo = match tool.as_str() {
                "lsfg_vk" => cfg.tools.lsfg_vk_repo.clone(),
                "autohdr_vk" => cfg.tools.autohdr_vk_repo.clone(),
                _ => return Err(format!("unknown tool: {tool}")),
            };
            let install_dir = match tool.as_str() {
                "lsfg_vk" => state.paths.lsfg_vk_install_dir(),
                "autohdr_vk" => state.paths.autohdr_vk_install_dir(),
                _ => unreachable!(),
            };
            let config_path = match tool.as_str() {
                "lsfg_vk" => state.paths.lsfg_vk_config_file(),
                "autohdr_vk" => state.paths.autohdr_vk_config_file(),
                _ => unreachable!(),
            };
            (repo, tool.as_str(), install_dir, config_path)
        };

        let release = crate::github_release::fetch_latest_release(&repo)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                format!(
                    "No releases found for {repo}. Build manually and copy to {}",
                    install_dir.display()
                )
            })?;
        let asset = crate::github_release::pick_linux_asset(&release, tool_key).ok_or_else(|| {
            format!("No Linux release asset found for {}", release.tag_name)
        })?;
        let bytes = crate::github_release::download_bytes(&asset.browser_download_url)
            .await
            .map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&install_dir).map_err(|e| e.to_string())?;
        install::extract_archive(&bytes, &asset.name, &install_dir).map_err(|e| e.to_string())?;
        install::normalize_install_root(&install_dir).map_err(|e| e.to_string())?;
        install::ensure_default_config(&install_dir, tool_key).map_err(|e| e.to_string())?;
        if !config_path.exists() {
            match tool_key {
                "lsfg_vk" => LsfgVkConfig::default_config()
                    .save(&config_path)
                    .map_err(|e| e.to_string())?,
                "autohdr_vk" => AutoHdrVkConfig::default_config()
                    .save(&config_path)
                    .map_err(|e| e.to_string())?,
                _ => {}
            }
        }
        install::register_install(
            &state.paths.tools_manifest_file(),
            tool_key,
            &install_dir,
            &release.tag_name,
        )
        .map_err(|e| e.to_string())?;
        get_tools_status(state)
    }
}

#[tauri::command]
pub fn get_lsfg_vk_config(state: State<'_, AppState>) -> Result<LsfgVkConfigDto, String> {
    LsfgVkConfig::load(&state.paths.lsfg_vk_config_file())
        .map(|c| c.to_dto())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_lsfg_vk_config(state: State<'_, AppState>, config: LsfgVkConfigDto) -> Result<(), String> {
    std::fs::create_dir_all(state.paths.lsfg_vk_install_dir()).map_err(|e| e.to_string())?;
    LsfgVkConfig::from_dto(config)
        .save(&state.paths.lsfg_vk_config_file())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_autohdr_vk_config(state: State<'_, AppState>) -> Result<AutoHdrVkConfigDto, String> {
    AutoHdrVkConfig::load(&state.paths.autohdr_vk_config_file())
        .map(|c| c.to_dto())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_autohdr_vk_config(
    state: State<'_, AppState>,
    config: AutoHdrVkConfigDto,
) -> Result<(), String> {
    std::fs::create_dir_all(state.paths.autohdr_vk_install_dir()).map_err(|e| e.to_string())?;
    AutoHdrVkConfig::from_dto(config)
        .save(&state.paths.autohdr_vk_config_file())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn detect_game_executables(
    state: State<'_, AppState>,
    game_id: String,
) -> Result<Vec<String>, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    let game = cfg
        .managed_games
        .iter()
        .find(|g| g.id == game_id)
        .ok_or_else(|| format!("game not found: {game_id}"))?;
    let overrides = game.tool_overrides.clone().unwrap_or_default();
    if !overrides.executables.is_empty() {
        return Ok(overrides.executables.clone());
    }
    #[cfg(target_os = "linux")]
    {
        return Ok(executables::detect_executables(&game.install_path, 8));
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(Vec::new())
    }
}

#[tauri::command]
pub fn get_game_tools(state: State<'_, AppState>, game_id: String) -> Result<GameToolsResponse, String> {
    let cfg = state.config.lock().map_err(|e| e.to_string())?;
    let game = cfg
        .managed_games
        .iter()
        .find(|g| g.id == game_id)
        .ok_or_else(|| format!("game not found: {game_id}"))?;
    let overrides = game.tool_overrides.clone().unwrap_or_default();
    let executables = if !overrides.executables.is_empty() {
        overrides.executables.clone()
    } else {
        #[cfg(target_os = "linux")]
        {
            executables::detect_executables(&game.install_path, 8)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Vec::new()
        }
    };
    #[cfg(target_os = "linux")]
    let steam_app_id = executables::steam_app_id_from_install(&game.install_path);
    #[cfg(not(target_os = "linux"))]
    let steam_app_id = None;
    #[cfg(target_os = "linux")]
    let launch_options = steam_app_id
        .as_ref()
        .and_then(|id| steam_launch::read_launch_options(id).ok().flatten());
    #[cfg(not(target_os = "linux"))]
    let launch_options = None;
    Ok(GameToolsResponse {
        overrides,
        executables,
        steam_app_id,
        launch_options,
    })
}

#[derive(serde::Deserialize)]
pub struct SetGameToolsRequest {
    pub game_id: String,
    pub overrides: GameToolOverrides,
}

#[tauri::command]
pub fn set_game_tools(
    state: State<'_, AppState>,
    request: SetGameToolsRequest,
) -> Result<GameToolsResponse, String> {
    {
        let mut cfg = state.config.lock().map_err(|e| e.to_string())?;
        let game = cfg
            .managed_games
            .iter_mut()
            .find(|g| g.id == request.game_id)
            .ok_or_else(|| format!("game not found: {}", request.game_id))?;
        game.tool_overrides = Some(request.overrides.clone());
    }
    #[cfg(target_os = "linux")]
    {
        let game = {
            let cfg = state.config.lock().map_err(|e| e.to_string())?;
            cfg.managed_games
                .iter()
                .find(|g| g.id == request.game_id)
                .cloned()
                .ok_or_else(|| format!("game not found: {}", request.game_id))?
        };
        sync::sync_game_tools(&state.paths, &game).map_err(|e| e.to_string())?;
    }
    get_game_tools(state, request.game_id)
}

#[tauri::command]
pub fn sync_game_tools(state: State<'_, AppState>, game_id: String) -> Result<GameToolsResponse, String> {
    #[cfg(target_os = "linux")]
    {
        let game = {
            let cfg = state.config.lock().map_err(|e| e.to_string())?;
            cfg.managed_games
                .iter()
                .find(|g| g.id == game_id)
                .cloned()
                .ok_or_else(|| format!("game not found: {game_id}"))?
        };
        sync::sync_game_tools(&state.paths, &game).map_err(|e| e.to_string())?;
    }
    get_game_tools(state, game_id)
}
