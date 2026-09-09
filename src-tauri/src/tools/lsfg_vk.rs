//! lsfg-vk configuration (conf.toml schema).

use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LsfgVkConfig {
    #[serde(default)]
    pub global: LsfgVkGlobal,
    #[serde(default, rename = "profile")]
    pub profiles: Vec<LsfgVkProfile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LsfgVkGlobal {
    #[serde(default)]
    pub dll: Option<String>,
    #[serde(default)]
    pub allow_fp16: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LsfgVkProfile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub active_in: Vec<String>,
    #[serde(default)]
    pub multiplier: u32,
    #[serde(default)]
    pub target_fps: u32,
    #[serde(default)]
    pub flow_scale: f32,
    #[serde(default)]
    pub performance_mode: bool,
    #[serde(default)]
    pub pacing: String,
    #[serde(default)]
    pub gpu: Option<String>,
}

impl Default for LsfgVkProfile {
    fn default() -> Self {
        Self {
            name: "Default".to_string(),
            active_in: Vec::new(),
            multiplier: 2,
            target_fps: 0,
            flow_scale: 1.0,
            performance_mode: false,
            pacing: "none".to_string(),
            gpu: None,
        }
    }
}

impl LsfgVkConfig {
    pub fn default_config() -> Self {
        Self {
            global: LsfgVkGlobal {
                dll: None,
                allow_fp16: true,
            },
            profiles: vec![LsfgVkProfile::default()],
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default_config());
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("reading lsfg-vk config {}", path.display()))?;
        let cfg: Self = toml::from_str(&raw).context("parsing lsfg-vk conf.toml")?;
        Ok(cfg)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self).context("serializing lsfg-vk conf.toml")?;
        fs::write(path, raw)
            .with_context(|| format!("writing lsfg-vk config {}", path.display()))?;
        Ok(())
    }

    pub fn profile_mut(&mut self, name: &str) -> &mut LsfgVkProfile {
        if let Some(idx) = self.profiles.iter().position(|p| p.name == name) {
            return &mut self.profiles[idx];
        }
        self.profiles.push(LsfgVkProfile {
            name: name.to_string(),
            ..LsfgVkProfile::default()
        });
        let last = self.profiles.len() - 1;
        &mut self.profiles[last]
    }

    pub fn remove_profile(&mut self, name: &str) {
        self.profiles.retain(|p| p.name != name);
    }

    pub fn default_profile(&self) -> LsfgVkProfile {
        self.profiles
            .first()
            .cloned()
            .unwrap_or_default()
    }

    pub fn to_dto(&self) -> LsfgVkConfigDto {
        LsfgVkConfigDto {
            global: self.global.clone(),
            profiles: self.profiles.clone(),
        }
    }

    pub fn from_dto(dto: LsfgVkConfigDto) -> Self {
        Self {
            global: dto.global,
            profiles: dto.profiles,
        }
    }
}

/// IPC/JSON shape (`profiles`); TOML on disk uses `[[profile]]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LsfgVkConfigDto {
    pub global: LsfgVkGlobal,
    pub profiles: Vec<LsfgVkProfile>,
}
