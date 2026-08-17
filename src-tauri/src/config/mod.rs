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
