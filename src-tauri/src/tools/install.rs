//! Extract and install tool release archives.

use std::{
    fs,
    io::{copy, Cursor},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use walkdir::WalkDir;

use super::manifest::{ToolInstallRecord, ToolsManifest};

pub fn extract_archive(bytes: &[u8], name: &str, dest: &Path) -> Result<()> {
    if dest.exists() {
        fs::remove_dir_all(dest).ok();
    }
    fs::create_dir_all(dest)?;

    if name.ends_with(".tar.xz") {
        extract_tar_xz(bytes, dest)?;
    } else if name.ends_with(".tar.zst") {
        return Err(anyhow!(
            "zstd archives are not supported yet; use .tar.xz release"
        ));
    } else if name.ends_with(".zip") {
        extract_zip(bytes, dest)?;
    } else {
        return Err(anyhow!("unsupported archive format: {name}"));
    }
    Ok(())
}

fn extract_tar_xz(bytes: &[u8], dest: &Path) -> Result<()> {
    let decoder = xz2::read::XzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(decoder);
    archive.unpack(dest).context("extracting tar.xz")?;
    Ok(())
}

fn extract_zip(bytes: &[u8], dest: &Path) -> Result<()> {
    let reader = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(reader).context("opening zip")?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).context("reading zip entry")?;
        let outpath = match file.enclosed_name() {
            Some(p) => dest.join(p),
            None => continue,
        };
        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = fs::File::create(&outpath)?;
            copy(&mut file, &mut outfile)?;
        }
    }
    Ok(())
}

/// Flatten single top-level directory if present (common in release tarballs).
pub fn normalize_install_root(install_dir: &Path) -> Result<PathBuf> {
    let mut entries: Vec<_> = fs::read_dir(install_dir)
        .context("reading install dir")?
        .filter_map(|e| e.ok())
        .collect();
    if entries.len() == 1 && entries[0].file_type().map(|t| t.is_dir()).unwrap_or(false) {
        let inner = entries.remove(0).path();
        for entry in fs::read_dir(&inner)? {
            let entry = entry?;
            let dest = install_dir.join(entry.file_name());
            if dest.exists() {
                if dest.is_dir() {
                    fs::remove_dir_all(&dest)?;
                } else {
                    fs::remove_file(&dest)?;
                }
            }
            fs::rename(entry.path(), dest)?;
        }
        fs::remove_dir(&inner)?;
    }
    Ok(install_dir.to_path_buf())
}

pub fn register_install(
    manifest_path: &Path,
    tool_key: &str,
    install_dir: &Path,
    version: &str,
) -> Result<()> {
    let mut manifest = ToolsManifest::load(manifest_path)?;
    let record = ToolInstallRecord {
        version: version.to_string(),
        path: install_dir.display().to_string(),
        installed_at: Utc::now(),
    };
    match tool_key {
        "lsfg_vk" => manifest.lsfg_vk = Some(record),
        "autohdr_vk" => manifest.autohdr_vk = Some(record),
        _ => return Err(anyhow!("unknown tool: {tool_key}")),
    }
    manifest.save(manifest_path)?;
    Ok(())
}

/// Copy example conf if missing after install.
pub fn ensure_default_config(install_dir: &Path, tool_key: &str) -> Result<()> {
    let conf = install_dir.join("conf.toml");
    if conf.exists() {
        return Ok(());
    }
    let example = install_dir.join("conf.example.toml");
    if example.exists() {
        fs::copy(&example, &conf)?;
        return Ok(());
    }
    if tool_key == "lsfg_vk" {
        super::lsfg_vk::LsfgVkConfig::default_config().save(&conf)?;
    } else if tool_key == "autohdr_vk" {
        super::autohdr_vk::AutoHdrVkConfig::default_config().save(&conf)?;
    }
    Ok(())
}

/// Find layer .so in install tree.
pub fn find_layer_library(install_dir: &Path, needle: &str) -> Option<PathBuf> {
    for entry in WalkDir::new(install_dir).max_depth(6) {
        let entry = entry.ok()?;
        if entry.file_type().is_file() {
            let name = entry.file_name().to_string_lossy();
            if name.contains(needle) && name.ends_with(".so") {
                return Some(entry.path().to_path_buf());
            }
        }
    }
    None
}
