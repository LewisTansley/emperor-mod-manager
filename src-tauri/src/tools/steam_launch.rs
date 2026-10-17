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
        let flatpak =
            PathBuf::from(&home).join(".var/app/com.valvesoftware.Steam/.steam/steam/userdata");
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
    // Prefer accounts that already have a non-empty LaunchOptions line for this app.
    let mut fallback = None;
    for userdata in steam_userdata_dirs() {
        let path = userdata.join("config/localconfig.vdf");
        if !path.exists() {
            continue;
        }
        if let Some(opts) = read_launch_from_vdf(&path, app_id)? {
            if !opts.trim().is_empty() {
                return Ok(Some(opts));
            }
            if fallback.is_none() {
                fallback = Some(opts);
            }
        }
    }
    Ok(fallback)
}

fn read_launch_from_vdf(path: &Path, app_id: &str) -> Result<Option<String>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(extract_launch_options_for_app(&content, app_id))
}

/// Escape-aware LaunchOptions parser for a Steam app block.
pub fn extract_launch_options_for_app(content: &str, app_id: &str) -> Option<String> {
    let app_pattern = format!(r#"(?s)("{}")\s*(\{{)"#, regex::escape(app_id));
    let re_app = Regex::new(&app_pattern).ok()?;
    let caps = re_app.captures(content)?;
    let open_brace = caps.get(2)?.end();
    let block_end = find_matching_brace(content, open_brace - 1)?;
    let block = &content[caps.get(0)?.start()..=block_end];
    extract_launch_options_value(block)
}

/// Parse `"LaunchOptions" "…"` allowing `\"` escapes inside the value.
pub fn extract_launch_options_value(block: &str) -> Option<String> {
    let key = "\"LaunchOptions\"";
    let idx = block.find(key)?;
    let after = block[idx + key.len()..].trim_start();
    if !after.starts_with('"') {
        return None;
    }
    Some(unescape_vdf_quoted(&after[1..])?.0)
}

fn unescape_vdf_quoted(s: &str) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some((_, '\\')) => out.push('\\'),
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                Some((_, '"')) => out.push('"'),
                Some((_, other)) => out.push(other),
                None => break,
            }
        } else if c == '"' {
            return Some((out, i + 1));
        } else {
            out.push(c);
        }
    }
    None
}

pub fn escape_vdf_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out
}

pub fn write_launch_options(app_id: &str, launch_options: &str) -> Result<()> {
    let userdata_dirs = steam_userdata_dirs();
    if userdata_dirs.is_empty() {
        return Err(anyhow!("Steam userdata directory not found"));
    }
    // Prefer the userdata that already has a real LaunchOptions value for this app
    // so we do not corrupt idle/secondary Steam accounts.
    let mut preferred = Vec::new();
    let mut others = Vec::new();
    for userdata in &userdata_dirs {
        let path = userdata.join("config/localconfig.vdf");
        if !path.exists() {
            continue;
        }
        match read_launch_from_vdf(&path, app_id) {
            Ok(Some(opts)) if !opts.trim().is_empty() => preferred.push(path),
            Ok(Some(_)) | Ok(None) => {
                // Still a candidate if the app block exists (write_launch_to_vdf returns false otherwise).
                others.push(path);
            }
            Err(_) => others.push(path),
        }
    }
    let mut wrote = false;
    for path in preferred.into_iter().chain(others) {
        if write_launch_to_vdf(&path, app_id, launch_options)? {
            wrote = true;
            // One successful preferred/secondary write is enough when preferred was used;
            // still write all preferred accounts (usually one), skip blank secondary corruption
            // by stopping after the first successful write if it came from preferred set.
            break;
        }
    }
    if wrote {
        Ok(())
    } else {
        Err(anyhow!(
            "could not update Steam launch options for app {app_id}"
        ))
    }
}

fn write_launch_to_vdf(path: &Path, app_id: &str, launch_options: &str) -> Result<bool> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let Some(updated) = replace_launch_options_in_vdf(&content, app_id, launch_options) else {
        return Ok(false);
    };
    fs::write(path, updated).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// Replace or insert LaunchOptions for `app_id`. Returns None when the app block is missing.
pub fn replace_launch_options_in_vdf(
    content: &str,
    app_id: &str,
    launch_options: &str,
) -> Option<String> {
    let app_pattern = format!(r#"(?s)("{}")\s*(\{{)"#, regex::escape(app_id));
    let re_app = Regex::new(&app_pattern).ok()?;
    let caps = re_app.captures(content)?;
    let block_start = caps.get(0)?.start();
    let open_brace = caps.get(2)?.end();
    let block_end = find_matching_brace(content, open_brace - 1)?;
    let mut block = content[block_start..=block_end].to_string();
    block = set_launch_options_in_block(&block, launch_options);

    let mut out = String::new();
    out.push_str(&content[..block_start]);
    out.push_str(&block);
    out.push_str(&content[block_end + 1..]);
    Some(out)
}

fn set_launch_options_in_block(block: &str, launch_options: &str) -> String {
    let key = "\"LaunchOptions\"";
    if let Some(idx) = block.find(key) {
        let after_key = idx + key.len();
        let rest = &block[after_key..];
        let trimmed_start = rest.len() - rest.trim_start().len();
        let value_start = after_key + trimmed_start;
        if !block[value_start..].starts_with('"') {
            return block.to_string();
        }
        let Some((_, consumed)) = unescape_vdf_quoted(&block[value_start + 1..]) else {
            return block.to_string();
        };
        let value_end = value_start + 1 + consumed;
        if launch_options.trim().is_empty() {
            // Drop the whole LaunchOptions line (from prior newline if present).
            let line_start = block[..idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let mut line_end = value_end;
            if block[line_end..].starts_with('\n') {
                line_end += 1;
            }
            let mut out = String::new();
            out.push_str(&block[..line_start]);
            out.push_str(&block[line_end..]);
            return out;
        }
        let escaped = escape_vdf_string(launch_options);
        let mut out = String::new();
        out.push_str(&block[..value_start]);
        out.push('"');
        out.push_str(&escaped);
        out.push('"');
        out.push_str(&block[value_end..]);
        return out;
    }

    if launch_options.trim().is_empty() {
        return block.to_string();
    }
    let escaped = escape_vdf_string(launch_options);
    let insert = format!("\n\t\t\"LaunchOptions\"\t\t\"{escaped}\"");
    let close = block.rfind('}').unwrap_or(block.len());
    let mut out = String::new();
    out.push_str(&block[..close]);
    out.push_str(&insert);
    out.push_str(&block[close..]);
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_escaped_winedlloverrides_quotes() {
        let vdf = r#"
"UserLocalConfigStore"
{
	"Software"
	{
		"Valve"
		{
			"Steam"
			{
				"apps"
				{
					"335670"
					{
						"LaunchOptions"		"WINEDLLOVERRIDES=\"dinput8=n,b\" %command%"
					}
				}
			}
		}
	}
}
"#;
        let opts = extract_launch_options_for_app(vdf, "335670").unwrap();
        assert_eq!(opts, r#"WINEDLLOVERRIDES="dinput8=n,b" %command%"#);
        assert!(opts.contains("dinput8=n,b"));
    }

    #[test]
    fn writes_unquoted_override_round_trip() {
        let vdf = r#"
"UserLocalConfigStore"
{
	"apps"
	{
		"335670"
		{
			"LaunchOptions"		"PROTON_ENABLE_WAYLAND=1 %command%"
		}
	}
}
"#;
        let updated =
            replace_launch_options_in_vdf(vdf, "335670", "WINEDLLOVERRIDES=dinput8=n,b %command%")
                .unwrap();
        let opts = extract_launch_options_for_app(&updated, "335670").unwrap();
        assert_eq!(opts, "WINEDLLOVERRIDES=dinput8=n,b %command%");
        // Value stored without needing nested quote escapes when unquoted.
        assert!(updated.contains(r#""LaunchOptions"		"WINEDLLOVERRIDES=dinput8=n,b %command%""#));
    }

    #[test]
    fn round_trips_quoted_value_with_escapes() {
        let vdf = r#"
"apps"
{
	"1"
	{
		"LaunchOptions"		"old"
	}
}
"#;
        let written = r#"WINEDLLOVERRIDES="dinput8=n,b" %command%"#;
        let updated = replace_launch_options_in_vdf(vdf, "1", written).unwrap();
        assert!(updated.contains(r#"WINEDLLOVERRIDES=\"dinput8=n,b\""#));
        let opts = extract_launch_options_for_app(&updated, "1").unwrap();
        assert_eq!(opts, written);
    }

    #[test]
    fn escape_vdf_string_escapes_quotes_and_backslashes() {
        assert_eq!(
            escape_vdf_string(r#"WINEDLLOVERRIDES="dinput8=n,b""#),
            r#"WINEDLLOVERRIDES=\"dinput8=n,b\""#
        );
    }
}
