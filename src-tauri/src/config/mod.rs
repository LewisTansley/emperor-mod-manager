//! Path helpers and persistent app configuration.

use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};

pub const APP_NAME: &str = "emperor-mod-manager";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const KEYRING_SERVICE: &str = "emperor-mod-manager";
pub const KEYRING_USER: &str = "nexus-api-key";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    Light,
    Dark,
    System,
}

impl Default for ThemePreference {
    fn default() -> Self {
        Self::System
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstallClickBehavior {
    Stay,
    Downloads,
}

impl Default for InstallClickBehavior {
    fn default() -> Self {
        Self::Downloads
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub managed_games: Vec<ManagedGame>,
    pub adult_content: bool,
    pub last_active_game_id: Option<String>,
    /// When true, the free-download WebView auto-clicks Mod Manager / Slow Download once ready.
    pub autoclick_free_download: bool,
    pub theme: ThemePreference,
    /// Whether Install switches to the Downloads tab or keeps the current tab.
    pub install_click_behavior: InstallClickBehavior,
    /// Set after the first import of managed games from legacy nexus-manager config.
    #[serde(default)]
    pub legacy_games_merged: bool,
    /// Vulkan tool integration (lsfg-vk, AutoHDR-VK).
    #[serde(default)]
    pub tools: ToolsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    pub lsfg_vk_repo: String,
    pub autohdr_vk_repo: String,
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            lsfg_vk_repo: "LewisTansley/lsfg-vk".to_string(),
            autohdr_vk_repo: "LewisTansley/AutoHDR-VK".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GameToolOverrides {
    #[serde(default)]
    pub lsfg_vk: ToolGameState,
    #[serde(default)]
    pub autohdr_vk: ToolGameState,
    /// Windows/Linux executables used for profile matching (e.g. Game.exe).
    #[serde(default)]
    pub executables: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolGameState {
    #[serde(default)]
    pub enabled: bool,
    /// When true, inherit global tool settings instead of per-game overrides.
    #[serde(default = "default_use_global")]
    pub use_global_settings: bool,
}

fn default_use_global() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            managed_games: Vec::new(),
            adult_content: true,
            last_active_game_id: None,
            autoclick_free_download: true,
            theme: ThemePreference::System,
            install_click_behavior: InstallClickBehavior::Downloads,
            legacy_games_merged: false,
            tools: ToolsConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedGame {
    pub id: String,
    pub title: String,
    /// Nexus Mods domain; empty when Thunderstore-only.
    #[serde(default)]
    pub nexus_domain: String,
    pub install_path: String,
    pub launcher: String,
    pub plugin_id: String,
    #[serde(default)]
    pub cover_path: Option<String>,
    /// Optional Unreal project folder override (e.g. "Pal", "Phoenix").
    #[serde(default)]
    pub project_name: Option<String>,
    /// Thunderstore community identifier (e.g. "lethal-company").
    #[serde(default)]
    pub thunderstore_community: Option<String>,
    /// mod.io numeric game id when this title is catalogued there.
    #[serde(default)]
    pub modio_game_id: Option<u32>,
    /// Per-game Vulkan tool overrides (lsfg-vk, AutoHDR-VK).
    #[serde(default)]
    pub tool_overrides: Option<GameToolOverrides>,
}

pub struct Paths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    pub fn resolve() -> Result<Self> {
        let dirs = ProjectDirs::from("dev", "emperormodmanager", APP_NAME)
            .context("failed to resolve project directories")?;
        let paths = Self {
            config_dir: dirs.config_dir().to_path_buf(),
            data_dir: dirs.data_dir().to_path_buf(),
            cache_dir: dirs.cache_dir().to_path_buf(),
        };
        fs::create_dir_all(&paths.config_dir)?;
        fs::create_dir_all(&paths.data_dir)?;
        fs::create_dir_all(&paths.cache_dir)?;
        fs::create_dir_all(paths.downloads_dir())?;
        fs::create_dir_all(paths.assist_webview_dir())?;
        Ok(paths)
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.cache_dir.join("downloads")
    }

    /// Persistent WebKit profile for Download Assist (Nexus website cookies).
    pub fn assist_webview_dir(&self) -> PathBuf {
        self.data_dir.join("assist-webview")
    }

    pub fn game_data_dir(&self, game_id: &str) -> PathBuf {
        self.data_dir.join(game_id)
    }

    pub fn mods_dir(&self, game_id: &str) -> PathBuf {
        self.game_data_dir(game_id).join("mods")
    }

    pub fn loadorder_file(&self, game_id: &str) -> PathBuf {
        self.game_data_dir(game_id).join("loadorder.json")
    }

    pub fn collections_file(&self, game_id: &str) -> PathBuf {
        self.game_data_dir(game_id).join("collections.json")
    }

    pub fn deploy_manifest(&self, game_id: &str) -> PathBuf {
        self.game_data_dir(game_id).join("deployed.json")
    }

    /// Local library of Emperor share codes (not per-game staging).
    pub fn saved_collections_file(&self) -> PathBuf {
        self.data_dir.join("saved_collections.json")
    }

    pub fn tools_dir(&self) -> PathBuf {
        self.data_dir.join("tools")
    }

    pub fn tools_manifest_file(&self) -> PathBuf {
        self.tools_dir().join("manifest.json")
    }

    pub fn lsfg_vk_install_dir(&self) -> PathBuf {
        self.tools_dir().join("lsfg-vk")
    }

    pub fn autohdr_vk_install_dir(&self) -> PathBuf {
        self.tools_dir().join("autohdr-vk")
    }

    pub fn lsfg_vk_config_file(&self) -> PathBuf {
        self.lsfg_vk_install_dir().join("conf.toml")
    }

    pub fn autohdr_vk_config_file(&self) -> PathBuf {
        self.autohdr_vk_install_dir().join("conf.toml")
    }
}

pub fn load_config(paths: &Paths) -> Result<AppConfig> {
    let path = paths.config_file();
    if !path.exists() {
        let cfg = AppConfig::default();
        save_config(paths, &cfg)?;
        return Ok(cfg);
    }
    let raw =
        fs::read_to_string(&path).with_context(|| format!("reading config {}", path.display()))?;
    let cfg: AppConfig = toml::from_str(&raw).context("parsing config.toml")?;
    Ok(cfg)
}

pub fn save_config(paths: &Paths, cfg: &AppConfig) -> Result<()> {
    let path = paths.config_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = toml::to_string_pretty(cfg).context("serializing config")?;
    fs::write(&path, raw).with_context(|| format!("writing config {}", path.display()))?;
    Ok(())
}

pub fn ensure_game_dirs(paths: &Paths, game_id: &str) -> Result<()> {
    fs::create_dir_all(paths.mods_dir(game_id))?;
    Ok(())
}
