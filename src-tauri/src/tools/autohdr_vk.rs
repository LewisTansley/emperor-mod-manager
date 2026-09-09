//! AutoHDR-VK configuration (conf.toml schema).

use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AutoHdrVkConfig {
    #[serde(default)]
    pub global: AutoHdrVkGlobal,
    #[serde(default, rename = "profile")]
    pub profiles: Vec<AutoHdrVkProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoHdrVkGlobal {
    #[serde(default = "default_intensity")]
    pub intensity: f32,
    #[serde(default = "default_color_intensity")]
    pub color_intensity: f32,
    #[serde(default = "default_expansion_shape")]
    pub expansion_shape: f32,
    #[serde(default)]
    pub black_floor: f32,
    #[serde(default = "default_highlight_stretch")]
    pub highlight_stretch: f32,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default = "default_true")]
    pub set_hdr_metadata: bool,
    #[serde(default = "default_true")]
    pub prefer_hdr_swapchain: bool,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_intensity() -> f32 {
    0.5
}
fn default_color_intensity() -> f32 {
    0.33
}
fn default_expansion_shape() -> f32 {
    0.55
}
fn default_highlight_stretch() -> f32 {
    0.45
}
fn default_encoding() -> String {
    "auto".to_string()
}
fn default_true() -> bool {
    true
}

impl Default for AutoHdrVkGlobal {
    fn default() -> Self {
        Self {
            intensity: default_intensity(),
            color_intensity: default_color_intensity(),
            expansion_shape: default_expansion_shape(),
            black_floor: 0.0,
            highlight_stretch: default_highlight_stretch(),
            encoding: default_encoding(),
            set_hdr_metadata: true,
            prefer_hdr_swapchain: true,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoHdrVkProfile {
    pub exe: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub intensity: Option<f32>,
    #[serde(default)]
    pub color_intensity: Option<f32>,
    #[serde(default)]
    pub expansion_shape: Option<f32>,
    #[serde(default)]
    pub black_floor: Option<f32>,
    #[serde(default)]
    pub highlight_stretch: Option<f32>,
}

impl AutoHdrVkConfig {
    pub fn default_config() -> Self {
        Self {
            global: AutoHdrVkGlobal::default(),
            profiles: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default_config());
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("reading autohdr-vk config {}", path.display()))?;
        let cfg: Self = toml::from_str(&raw).context("parsing autohdr-vk conf.toml")?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self).context("serializing autohdr-vk conf.toml")?;
        fs::write(path, raw)
            .with_context(|| format!("writing autohdr-vk config {}", path.display()))?;
        Ok(())
    }

    pub fn profile_mut(&mut self, exe: &str) -> &mut AutoHdrVkProfile {
        let exe_lower = exe.to_lowercase();
        if let Some(idx) = self
            .profiles
            .iter()
            .position(|p| p.exe.eq_ignore_ascii_case(&exe_lower))
        {
            return &mut self.profiles[idx];
        }
        self.profiles.push(AutoHdrVkProfile {
            exe: exe.to_string(),
            enabled: true,
            intensity: None,
            color_intensity: None,
            expansion_shape: None,
            black_floor: None,
            highlight_stretch: None,
        });
        let last = self.profiles.len() - 1;
        &mut self.profiles[last]
    }

    pub fn remove_profile(&mut self, exe: &str) {
        self.profiles
            .retain(|p| !p.exe.eq_ignore_ascii_case(exe));
    }

    pub fn to_dto(&self) -> AutoHdrVkConfigDto {
        AutoHdrVkConfigDto {
            global: self.global.clone(),
            profiles: self.profiles.clone(),
        }
    }

    pub fn from_dto(dto: AutoHdrVkConfigDto) -> Self {
        Self {
            global: dto.global,
            profiles: dto.profiles,
        }
    }
}

/// IPC/JSON shape (`profiles`); TOML on disk uses `[[profile]]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoHdrVkConfigDto {
    pub global: AutoHdrVkGlobal,
    pub profiles: Vec<AutoHdrVkProfile>,
}
