//! mod.io API client wrapper (API-key auth only).

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use modio::request::filter::prelude::*;
use modio::request::mods::filters::{Downloads, MaturityOption as MaturityFilter};
use modio::types::id::Id;
use modio::util::download::{Download, DownloadAction};
use modio::Client;
use serde::Serialize;
use tokio::io::{AsyncWriteExt, BufWriter};

use crate::config::KEYRING_SERVICE;

pub const KEYRING_USER: &str = "modio-api-key";

pub struct ModioClient {
    inner: Client,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModioGameHit {
    pub id: u32,
    pub name: String,
    pub name_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModioModHit {
    pub game_id: u32,
    pub mod_id: u64,
    pub name: String,
    pub name_id: String,
    pub summary: String,
    pub description: Option<String>,
    pub picture_url: Option<String>,
    pub author: Option<String>,
    pub downloads: u64,
    pub profile_url: String,
    pub tags: Vec<String>,
    pub has_dependencies: bool,
    pub primary_file_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModioModDetail {
    pub game_id: u32,
    pub mod_id: u64,
    pub name: String,
    pub name_id: String,
    pub summary: String,
    pub description: Option<String>,
    pub picture_url: Option<String>,
    pub author: Option<String>,
    pub downloads: u64,
    pub profile_url: String,
    pub tags: Vec<String>,
    pub has_dependencies: bool,
    pub primary_file_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModioFileInfo {
    pub file_id: u64,
    pub filename: String,
    pub version: Option<String>,
    pub filesize: u64,
    pub changelog: Option<String>,
    pub is_primary: bool,
    pub date_added: i64,
}

impl ModioClient {
    pub fn new(api_key: &str) -> Result<Self> {
        let key = api_key.trim();
        if key.is_empty() {
            bail!("mod.io API key is empty");
        }
        let inner = Client::builder(key.to_string())
            .build()
            .context("build mod.io client")?;
        Ok(Self { inner })
    }

    /// Cheap validation call (API key only — no OAuth).
    pub async fn validate(&self) -> Result<()> {
        let list = self
            .inner
            .get_games()
            .filter(with_limit(1))
            .await
            .context("mod.io get_games")?
            .data()
            .await
            .context("mod.io get_games body")?;
        let _ = list;
        Ok(())
    }

    /// Search games by title (fulltext). Used for catalog ID suggestions.
    pub async fn search_games(&self, query: &str, limit: u32) -> Result<Vec<ModioGameHit>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let filter = with_limit(limit.max(1) as usize).and(Fulltext::eq(q));
        let list = self
            .inner
            .get_games()
            .filter(filter)
            .await
            .context("mod.io get_games search")?
            .data()
            .await
            .context("mod.io get_games search body")?;
        Ok(list
            .data
            .into_iter()
            .map(|g| ModioGameHit {
                id: g.id.get() as u32,
                name: g.name,
                name_id: g.name_id,
            })
            .collect())
    }

    pub async fn search_mods(
        &self,
        game_id: u32,
        query: &str,
        offset: u32,
        count: u32,
        include_mature: bool,
    ) -> Result<(Vec<ModioModHit>, u32)> {
        let gid = Id::new(game_id as u64);
        let limit = count.max(1) as usize;
        let mut filter = with_limit(limit)
            .offset(offset as usize)
            .and(Downloads::desc());
        let q = query.trim();
        if !q.is_empty() {
            filter = filter.and(Fulltext::eq(q));
        }
        if !include_mature {
            // maturity_option == 0 means no maturity flags
            filter = filter.and(MaturityFilter::eq(0u8));
        }

        let list = self
            .inner
            .get_mods(gid)
            .filter(filter)
            .await
            .context("mod.io get_mods")?
            .data()
            .await
            .context("mod.io get_mods body")?;

        let total = list.total;
        let hits = list.data.into_iter().map(hit_from_mod).collect();
        Ok((hits, total))
    }

    pub async fn get_mod(&self, game_id: u32, mod_id: u64) -> Result<ModioModDetail> {
        let m = self
            .inner
            .get_mod(Id::new(game_id as u64), Id::new(mod_id))
            .await
            .context("mod.io get_mod")?
            .data()
            .await
            .context("mod.io get_mod body")?;
        Ok(detail_from_mod(m))
    }

    pub async fn list_files(
        &self,
        game_id: u32,
        mod_id: u64,
        primary_file_id: Option<u64>,
    ) -> Result<Vec<ModioFileInfo>> {
        let list = self
            .inner
            .get_files(Id::new(game_id as u64), Id::new(mod_id))
            .filter(with_limit(100))
            .await
            .context("mod.io get_files")?
            .data()
            .await
            .context("mod.io get_files body")?;

        Ok(list
            .data
            .into_iter()
            .map(|f| {
                let fid = f.id.get();
                ModioFileInfo {
                    file_id: fid,
                    filename: f.filename,
                    version: f.version,
                    filesize: f.filesize,
                    changelog: f.changelog,
                    is_primary: primary_file_id == Some(fid),
                    date_added: f.date_added.as_secs(),
                }
            })
            .collect())
    }

    pub async fn dependency_mod_ids(&self, game_id: u32, mod_id: u64) -> Result<Vec<u64>> {
        let list = self
            .inner
            .get_mod_dependencies(Id::new(game_id as u64), Id::new(mod_id))
            .await
            .context("mod.io get_mod_dependencies")?
            .data()
            .await
            .context("mod.io dependencies body")?;
        Ok(list.data.into_iter().map(|d| d.mod_id.get()).collect())
    }

    pub async fn download_file(
        &self,
        game_id: u32,
        mod_id: u64,
        file_id: Option<u64>,
        dest: &Path,
        on_progress: Option<&Arc<dyn Fn(u64, Option<u64>, u64) + Send + Sync>>,
    ) -> Result<()> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let action = if let Some(fid) = file_id {
            DownloadAction::File {
                game_id: Id::new(game_id as u64),
                mod_id: Id::new(mod_id),
                file_id: Id::new(fid),
            }
        } else {
            DownloadAction::Primary {
                game_id: Id::new(game_id as u64),
                mod_id: Id::new(mod_id),
            }
        };
        let mut chunked = self
            .inner
            .download(action)
            .chunked()
            .await
            .map_err(|e| anyhow::anyhow!("mod.io download: {e}"))?;
        let total = Some(chunked.info().filesize).filter(|n| *n > 0);

        let file = tokio::fs::File::create(dest).await?;
        let mut out = BufWriter::with_capacity(512 * 512, file);
        let mut downloaded: u64 = 0;
        let started = Instant::now();
        let mut last_report = started;
        while let Some(chunk) = chunked.data().await {
            let chunk = chunk.map_err(|e| anyhow::anyhow!("mod.io download: {e}"))?;
            out.write_all(&chunk).await?;
            downloaded += chunk.len() as u64;
            let now = Instant::now();
            if now.duration_since(last_report).as_millis() >= 150 {
                if let Some(cb) = on_progress {
                    let elapsed = started.elapsed().as_secs_f64().max(0.001);
                    cb(downloaded, total, (downloaded as f64 / elapsed) as u64);
                }
                last_report = now;
            }
        }
        out.flush().await?;
        if let Some(cb) = on_progress {
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            cb(
                downloaded,
                total.or(Some(downloaded)),
                (downloaded as f64 / elapsed) as u64,
            );
        }
        Ok(())
    }
}

fn hit_from_mod(m: modio::types::mods::Mod) -> ModioModHit {
    let primary = m.modfile.as_ref().map(|f| f.id.get());
    ModioModHit {
        game_id: m.game_id.get() as u32,
        mod_id: m.id.get(),
        name: m.name,
        name_id: m.name_id,
        summary: m.summary,
        description: m.description_plaintext.or(m.description),
        picture_url: Some(m.logo.thumb_640x360.to_string()),
        author: Some(m.submitted_by.username),
        downloads: m.stats.downloads_total as u64,
        profile_url: m.profile_url.to_string(),
        tags: m.tags.into_iter().map(|t| t.name).collect(),
        has_dependencies: m.dependencies,
        primary_file_id: primary,
    }
}

fn detail_from_mod(m: modio::types::mods::Mod) -> ModioModDetail {
    let hit = hit_from_mod(m);
    ModioModDetail {
        game_id: hit.game_id,
        mod_id: hit.mod_id,
        name: hit.name,
        name_id: hit.name_id,
        summary: hit.summary,
        description: hit.description,
        picture_url: hit.picture_url,
        author: hit.author,
        downloads: hit.downloads,
        profile_url: hit.profile_url,
        tags: hit.tags,
        has_dependencies: hit.has_dependencies,
        primary_file_id: hit.primary_file_id,
    }
}

pub fn store_api_key(key: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    entry.set_password(key.trim())?;
    Ok(())
}

pub fn load_api_key() -> Result<Option<String>> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    match entry.get_password() {
        Ok(k) if !k.is_empty() => Ok(Some(k)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => {
            log::warn!("mod.io keyring read failed: {e}");
            Ok(None)
        }
    }
}

pub fn clear_api_key() -> Result<()> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub fn store_api_key_file(path: &Path, key: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, key.trim())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}

pub fn load_api_key_file(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(path)?;
    let s = s.trim().to_string();
    if s.is_empty() {
        Ok(None)
    } else {
        Ok(Some(s))
    }
}

/// Known plugin_id → mod.io game id seeds (manual override still wins).
pub fn seed_modio_game_id(plugin_id: &str) -> Option<u32> {
    match plugin_id {
        // Deep Rock Galactic
        "deeprockgalactic" => Some(2475),
        // OpenXcom (generic manage)
        "openxcom" => Some(51),
        // Blade & Sorcery (official SDK / in-game manager)
        "bladeandsorcery" => Some(165),
        // BONELAB (CDN /mods/3541/...)
        "bonelab" => Some(3541),
        "snowrunner" => Some(123),
        "pavlov" => Some(3959),
        "spaceengineers" => Some(62),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_lookup() {
        assert_eq!(seed_modio_game_id("openxcom"), Some(51));
        assert_eq!(seed_modio_game_id("bladeandsorcery"), Some(165));
        assert_eq!(seed_modio_game_id("deeprockgalactic"), Some(2475));
        assert_eq!(seed_modio_game_id("bonelab"), Some(3541));
        assert_eq!(seed_modio_game_id("snowrunner"), Some(123));
        assert_eq!(seed_modio_game_id("pavlov"), Some(3959));
        assert_eq!(seed_modio_game_id("spaceengineers"), Some(62));
        assert_eq!(seed_modio_game_id("valheim"), None);
    }
}
