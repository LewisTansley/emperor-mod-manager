//! `emperor://` deep links that open a mod detail page in the app.
//!
//! Formats:
//! - `emperor://mod/nexus/{domain}/{modId}`
//! - `emperor://mod/thunderstore/{community}/{namespace}/{package}`
//! - `emperor://mod/modio/{gameId}/{modId}`

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use url::Url;

use crate::commands::AppState;

/// Parsed payload emitted to the frontend as `emperor-open-mod`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum EmperorOpenMod {
    Nexus { domain: String, mod_id: u32 },
    Thunderstore {
        community: String,
        namespace: String,
        package_name: String,
    },
    Modio { game_id: u32, mod_id: u32 },
}

impl EmperorOpenMod {
    pub fn build_url(&self) -> String {
        match self {
            Self::Nexus { domain, mod_id } => {
                format!("emperor://mod/nexus/{domain}/{mod_id}")
            }
            Self::Thunderstore {
                community,
                namespace,
                package_name,
            } => {
                format!("emperor://mod/thunderstore/{community}/{namespace}/{package_name}")
            }
            Self::Modio { game_id, mod_id } => {
                format!("emperor://mod/modio/{game_id}/{mod_id}")
            }
        }
    }
}

/// Parse an `emperor://mod/...` URL into a structured payload.
pub fn parse_emperor_mod_url(raw: &str) -> Option<EmperorOpenMod> {
    let raw = raw.trim();
    if !raw.to_ascii_lowercase().starts_with("emperor://") {
        return None;
    }
    let url = Url::parse(raw).ok()?;
    if url.scheme() != "emperor" {
        return None;
    }

    let host = url.host_str().unwrap_or("");
    let mut segments: Vec<String> = url
        .path_segments()?
        .filter(|s| !s.is_empty())
        .map(percent_decode_str)
        .collect::<Option<Vec<_>>>()?;

    if host.eq_ignore_ascii_case("mod") {
        // emperor://mod/nexus/... → host=mod, path=/nexus/...
    } else if host.is_empty() {
        // emperor:///mod/nexus/... → path=/mod/nexus/...
        if segments
            .first()
            .is_some_and(|s| s.eq_ignore_ascii_case("mod"))
        {
            segments.remove(0);
        } else {
            return None;
        }
    } else {
        return None;
    }

    if segments.is_empty() {
        return None;
    }

    match segments[0].to_ascii_lowercase().as_str() {
        "nexus" => {
            if segments.len() != 3 {
                return None;
            }
            let domain = segments[1].clone();
            if domain.is_empty() {
                return None;
            }
            let mod_id: u32 = segments[2].parse().ok()?;
            if mod_id == 0 {
                return None;
            }
            Some(EmperorOpenMod::Nexus { domain, mod_id })
        }
        "thunderstore" => {
            if segments.len() != 4 {
                return None;
            }
            let community = segments[1].clone();
            let namespace = segments[2].clone();
            let package_name = segments[3].clone();
            if community.is_empty() || namespace.is_empty() || package_name.is_empty() {
                return None;
            }
            Some(EmperorOpenMod::Thunderstore {
                community,
                namespace,
                package_name,
            })
        }
        "modio" => {
            if segments.len() != 3 {
                return None;
            }
            let game_id: u32 = segments[1].parse().ok()?;
            let mod_id: u32 = segments[2].parse().ok()?;
            if game_id == 0 || mod_id == 0 {
                return None;
            }
            Some(EmperorOpenMod::Modio { game_id, mod_id })
        }
        _ => None,
    }
}

fn percent_decode_str(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                if i + 2 >= bytes.len() {
                    return None;
                }
                let hi = from_hex(bytes[i + 1])?;
                let lo = from_hex(bytes[i + 2])?;
                out.push((hi << 4) | lo);
                i += 3;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn from_hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Store + emit `emperor-open-mod`, debouncing identical URLs within 1s.
pub fn emit_emperor_mod_url(app: &AppHandle, url: &str) {
    let url = url.trim();
    let Some(payload) = parse_emperor_mod_url(url) else {
        log::warn!("ignored unrecognized emperor deep link: {url}");
        return;
    };

    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut last) = state.last_emperor_mod_emit.lock() {
            if let Some((ref prev, at)) = *last {
                if prev == url && at.elapsed() < Duration::from_secs(1) {
                    log::debug!("debounce duplicate emperor-open-mod emit: {url}");
                    return;
                }
            }
            *last = Some((url.to_string(), Instant::now()));
        }
        if let Ok(mut pending) = state.pending_emperor_mod.lock() {
            *pending = Some(payload.clone());
        }
    }

    log::info!("emperor-open-mod: {url}");
    let _ = app.emit("emperor-open-mod", payload);
}

/// Take (and clear) any pending cold-start emperor mod link.
pub fn take_pending_emperor_mod(state: &AppState) -> Option<EmperorOpenMod> {
    state
        .pending_emperor_mod
        .lock()
        .ok()
        .and_then(|mut g| g.take())
}

#[tauri::command]
pub fn take_pending_emperor_mod_link(
    state: tauri::State<'_, AppState>,
) -> Option<EmperorOpenMod> {
    take_pending_emperor_mod(&state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nexus() {
        let link = parse_emperor_mod_url("emperor://mod/nexus/skyrimspecialedition/12604").unwrap();
        assert_eq!(
            link,
            EmperorOpenMod::Nexus {
                domain: "skyrimspecialedition".into(),
                mod_id: 12604,
            }
        );
        assert_eq!(
            link.build_url(),
            "emperor://mod/nexus/skyrimspecialedition/12604"
        );
    }

    #[test]
    fn parses_thunderstore() {
        let link = parse_emperor_mod_url(
            "emperor://mod/thunderstore/lethal-company/BepInEx/BepInExPack",
        )
        .unwrap();
        assert_eq!(
            link,
            EmperorOpenMod::Thunderstore {
                community: "lethal-company".into(),
                namespace: "BepInEx".into(),
                package_name: "BepInExPack".into(),
            }
        );
    }

    #[test]
    fn parses_modio() {
        let link = parse_emperor_mod_url("emperor://mod/modio/2248/12345").unwrap();
        assert_eq!(
            link,
            EmperorOpenMod::Modio {
                game_id: 2248,
                mod_id: 12345,
            }
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_emperor_mod_url("nxm://skyrim/mods/1/files/2").is_none());
        assert!(parse_emperor_mod_url("emperor://mod/nexus/onlydomain").is_none());
        assert!(parse_emperor_mod_url("emperor://mod/nexus/domain/0").is_none());
        assert!(parse_emperor_mod_url("emperor://other/nexus/domain/1").is_none());
        assert!(parse_emperor_mod_url("emperor://mod/unknown/a/b").is_none());
        assert!(parse_emperor_mod_url("https://example.com/mod").is_none());
    }

    #[test]
    fn parses_percent_encoded_domain() {
        let link =
            parse_emperor_mod_url("emperor://mod/nexus/stardew%20valley/2400").unwrap();
        assert_eq!(
            link,
            EmperorOpenMod::Nexus {
                domain: "stardew valley".into(),
                mod_id: 2400,
            }
        );
    }
}
