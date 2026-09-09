//! Steam launch option merge for Vulkan tool env vars.

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use regex::Regex;

const EMM_BEGIN: &str = "EMM_TOOLS_BEGIN";
const EMM_END: &str = "EMM_TOOLS_END";

#[derive(Debug, Clone, Default)]
pub struct LaunchEnvBlock {
    pub lsfg_vk: Option<LsfgLaunch>,
    pub autohdr_vk: bool,
}

#[derive(Debug, Clone)]
pub struct LsfgLaunch {
    pub config_path: String,
    pub profile: String,
}

pub fn build_launch_block(block: &LaunchEnvBlock) -> String {
    let mut vars = Vec::new();
    if let Some(lsfg) = &block.lsfg_vk {
        vars.push(format!("LSFGVK_CONFIG={}", lsfg.config_path));
        vars.push(format!("LSFGVK_PROFILE={}", lsfg.profile));
    }
    if block.autohdr_vk {
        vars.push("ENABLE_AUTOHDR=1".to_string());
    }
    if vars.is_empty() {
        return String::new();
    }
    format!("{EMM_BEGIN} {} {EMM_END} %command%", vars.join(" "))
}

pub fn merge_launch_options(existing: &str, block: &LaunchEnvBlock) -> String {
    let stripped = strip_emperor_block(existing);
    let emperor = build_launch_block(block);
    if emperor.is_empty() {
        return stripped.trim().to_string();
    }
    let base = stripped.trim();
    if base.is_empty() {
        return emperor;
    }
    if base.contains("%command%") {
        format!("{emperor} {base}")
    } else {
        format!("{emperor} {base} %command%")
    }
}

pub fn strip_emperor_block(opts: &str) -> String {
    let mut out = opts.to_string();
    while let Some(start) = out.find(EMM_BEGIN) {
        if let Some(end) = out[start..].find(EMM_END) {
            let end = start + end + EMM_END.len();
            out.replace_range(start..end, "");
        } else {
            break;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn steam_userdata_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
        let steam = PathBuf::from(&home).join(".steam/steam/userdata");
        if steam.is_dir() {
            if let Ok(entries) = fs::read_dir(&steam) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        dirs.push(entry.path());
                    }
                }
            }
        }
        let flatpak = PathBuf::from(&home)
            .join(".var/app/com.valvesoftware.Steam/.steam/steam/userdata");
        if flatpak.is_dir() {
            if let Ok(entries) = fs::read_dir(&flatpak) {
                for entry in entries.flatten() {
                    if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        dirs.push(entry.path());
                    }
                }
            }
        }
    }
    dirs
}

pub fn read_launch_options(app_id: &str) -> Result<Option<String>> {
    for userdata in steam_userdata_dirs() {
        let path = userdata.join("config/localconfig.vdf");
        if !path.exists() {
            continue;
        }
        if let Some(opts) = read_launch_from_vdf(&path, app_id)? {
            return Ok(Some(opts));
        }
    }
    Ok(None)
}

fn read_launch_from_vdf(path: &Path, app_id: &str) -> Result<Option<String>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let pattern = format!(
        r#"(?s)"{app_id}"\s*\{{.*?"LaunchOptions"\s+"([^"]*)""#
    );
    let re = Regex::new(&pattern).context("compiling launch options regex")?;
    Ok(re
        .captures(&content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string()))
}

pub fn write_launch_options(app_id: &str, launch_options: &str) -> Result<()> {
    let userdata_dirs = steam_userdata_dirs();
    if userdata_dirs.is_empty() {
        return Err(anyhow!("Steam userdata directory not found"));
    }
    let mut wrote = false;
    for userdata in userdata_dirs {
        let path = userdata.join("config/localconfig.vdf");
        if !path.exists() {
            continue;
        }
        if write_launch_to_vdf(&path, app_id, launch_options)? {
            wrote = true;
        }
    }
    if wrote {
        Ok(())
    } else {
        Err(anyhow!("could not update Steam launch options for app {app_id}"))
    }
}

fn write_launch_to_vdf(path: &Path, app_id: &str, launch_options: &str) -> Result<bool> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))?;
    let app_pattern = format!(r#"(?s)("{}")\s*(\{{)"#, regex::escape(app_id));
    let re_app = Regex::new(&app_pattern).context("compiling app block regex")?;
    let Some(caps) = re_app.captures(&content) else {
        return Ok(false);
    };
    let block_start = caps.get(0).unwrap().start();
    let open_brace = caps.get(2).unwrap().end();
    let block_end = find_matching_brace(&content, open_brace - 1)
        .ok_or_else(|| anyhow!("malformed VDF for app {app_id}"))?;
    let mut block = content[block_start..=block_end].to_string();

    let launch_re = Regex::new(r#"(?m)^\s*"LaunchOptions"\s+"[^"]*"\s*$"#)
        .context("compiling LaunchOptions regex")?;
    if launch_options.trim().is_empty() {
        block = launch_re.replace(&block, "").to_string();
    } else if launch_re.is_match(&block) {
        block = launch_re
            .replace(
                &block,
                format!(r#""LaunchOptions"		"{launch_options}""#),
            )
            .to_string();
    } else {
        let insert = format!(r#"
		"LaunchOptions"		"{launch_options}""#);
        let close = block.rfind('}').unwrap_or(block.len());
        block.insert_str(close, &insert);
    }

    let mut out = String::new();
    out.push_str(&content[..block_start]);
    out.push_str(&block);
    out.push_str(&content[block_end + 1..]);
    fs::write(path, out).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

fn find_matching_brace(content: &str, open_idx: usize) -> Option<usize> {
    let bytes = content.as_bytes();
    if bytes.get(open_idx) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate().skip(open_idx) {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}
