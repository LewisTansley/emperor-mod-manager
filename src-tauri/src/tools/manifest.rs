//! Installed tool versions tracked by Emperor.

use std::{fs, path::Path};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolsManifest {
    #[serde(default)]
    pub lsfg_vk: Option<ToolInstallRecord>,
    #[serde(default)]
    pub autohdr_vk: Option<ToolInstallRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInstallRecord {
    pub version: String,
    pub path: String,
    pub installed_at: DateTime<Utc>,
}

impl ToolsManifest {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)
            .with_context(|| format!("reading tools manifest {}", path.display()))?;
        serde_json::from_str(&raw).context("parsing tools manifest")
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self).context("serializing tools manifest")?;
        fs::write(path, raw)
            .with_context(|| format!("writing tools manifest {}", path.display()))?;
        Ok(())
    }
}
