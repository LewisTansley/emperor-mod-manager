//! Helldivers 2 patch-archive mods.
//!
//! Multiple mods that patch the same archive id must be renamed to sequential
//! `.patch_N` (plus matching `.stream` / `.gpu_resources` sidecars) based on
//! deploy/load order.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;

use super::{normalize_relative, DeployContext, GamePlugin, GamePluginInfo};

pub const HD2_ROOT_DIRS: &[&str] = &["data", "Data"];

const ROOT_INJECTORS: &[&str] = &[
    "dxgi.dll",
    "d3d11.dll",
    "d3d12.dll",
    "dinput8.dll",
    "version.dll",
    "winmm.dll",
    "opengl32.dll",
    "reshade.ini",
    "ReShade.ini",
];

struct PatchState {
    next_index: HashMap<String, u32>,
    assigned: HashMap<(String, String), u32>,
}

static PATCH_STATE: Mutex<Option<PatchState>> = Mutex::new(None);

pub struct Helldivers2Plugin;

impl GamePlugin for Helldivers2Plugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "helldivers2",
            display_name: "Helldivers 2",
            nexus_domain: "helldivers2",
            match_names: &["helldivers 2", "helldivers2"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        HD2_ROOT_DIRS
    }

    fn prepare_deploy(&self, _install_path: &Path) -> Result<Vec<String>> {
        reset_patch_state();
        Ok(Vec::new())
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_hd2_deploy(
            install_path,
            relative,
            None,
        ))
    }

    fn resolve_deploy_root_ctx(
        &self,
        install_path: &Path,
        relative: &Path,
        ctx: &DeployContext<'_>,
    ) -> Result<PathBuf> {
        Ok(resolve_hd2_deploy(
            install_path,
            relative,
            ctx.content_root,
        ))
    }
}

fn reset_patch_state() {
    let mut guard = PATCH_STATE.lock().unwrap_or_else(|e| e.into_inner());
    *guard = Some(PatchState {
        next_index: HashMap::new(),
        assigned: HashMap::new(),
    });
}

fn assign_patch_index(content_root: Option<&Path>, archive_id: &str) -> u32 {
    let mut guard = PATCH_STATE.lock().unwrap_or_else(|e| e.into_inner());
    let state = guard.get_or_insert_with(|| PatchState {
        next_index: HashMap::new(),
        assigned: HashMap::new(),
    });
    let root_key = content_root
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let key = (root_key, archive_id.to_string());
    if let Some(&idx) = state.assigned.get(&key) {
        return idx;
    }
    let idx = state.next_index.get(archive_id).copied().unwrap_or(0);
    state
        .next_index
        .insert(archive_id.to_string(), idx.saturating_add(1));
    state.assigned.insert(key, idx);
    idx
}

/// `{archive}.patch_{n}` or `{archive}.patch{n}`, plus optional `.stream` / `.gpu_resources`.
fn parse_patch_name(name: &str) -> Option<(String, String)> {
    let lower = name.to_lowercase();
    let (stem, suffix) = if let Some(rest) = lower.strip_suffix(".gpu_resources") {
        (rest, ".gpu_resources")
    } else if let Some(rest) = lower.strip_suffix(".stream") {
        (rest, ".stream")
    } else {
        (lower.as_str(), "")
    };

    if let Some(idx) = stem.rfind(".patch_") {
        let archive = &stem[..idx];
        let num = &stem[idx + ".patch_".len()..];
        if !archive.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
            return Some((archive.to_string(), suffix.to_string()));
        }
    }
    if let Some(idx) = stem.rfind(".patch") {
        let archive = &stem[..idx];
        let num = &stem[idx + ".patch".len()..];
        if !archive.is_empty() && !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
            return Some((archive.to_string(), suffix.to_string()));
        }
    }
    None
}

fn resolve_hd2_deploy(
    install_path: &Path,
    relative: &Path,
    content_root: Option<&Path>,
) -> PathBuf {
    let normalized = normalize_relative(relative);
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if originals.is_empty() {
        return install_path.join("data");
    }

    let first_lower = originals[0].to_lowercase();
    let leaf = originals.last().map(|s| s.as_str()).unwrap_or("");

    if originals.len() == 1 && ROOT_INJECTORS.iter().any(|n| n.eq_ignore_ascii_case(leaf))
    {
        return install_path.join(leaf);
    }

    let skip_data = first_lower == "data";
    let rest: &[String] = if skip_data {
        &originals[1..]
    } else {
        &originals
    };

    if rest.len() == 1 && rest[0].to_lowercase().ends_with(".dl-bin") {
        return install_path
            .join("data")
            .join("game")
            .join(&rest[0]);
    }
    if rest.len() >= 2 && rest[0].eq_ignore_ascii_case("game") {
        let mut out = install_path.join("data").join("game");
        for part in &rest[1..] {
            out.push(part);
        }
        return out;
    }

    if let Some((archive_id, suffix)) = parse_patch_name(leaf) {
        let idx = assign_patch_index(content_root, &archive_id);
        let renamed = format!("{archive_id}.patch_{idx}{suffix}");
        return install_path.join("data").join(renamed);
    }

    let mut out = install_path.join("data");
    for part in rest {
        out.push(part);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static TEST_GATE: StdMutex<()> = StdMutex::new(());

    fn plugin() -> Helldivers2Plugin {
        Helldivers2Plugin
    }

    #[test]
    fn two_mods_same_archive_get_sequential_indices() {
        let _gate = TEST_GATE.lock().unwrap_or_else(|e| e.into_inner());
        plugin().prepare_deploy(Path::new("/game")).unwrap();

        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let ctx_a = DeployContext {
            project_name: None,
            content_root: Some(a.path()),
        };
        let ctx_b = DeployContext {
            project_name: None,
            content_root: Some(b.path()),
        };

        let dest_a = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("9ba626afa44a3aa3.patch_0"),
                &ctx_a,
            )
            .unwrap();
        let dest_a_stream = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("9ba626afa44a3aa3.patch_0.stream"),
                &ctx_a,
            )
            .unwrap();
        let dest_b = plugin()
            .resolve_deploy_root_ctx(
                Path::new("/game"),
                Path::new("9ba626afa44a3aa3.patch_0"),
                &ctx_b,
            )
            .unwrap();

        assert_eq!(
            dest_a,
            PathBuf::from("/game/data/9ba626afa44a3aa3.patch_0")
        );
        assert_eq!(
            dest_a_stream,
            PathBuf::from("/game/data/9ba626afa44a3aa3.patch_0.stream")
        );
        assert_eq!(
            dest_b,
            PathBuf::from("/game/data/9ba626afa44a3aa3.patch_1")
        );
    }

    #[test]
    fn dl_bin_goes_to_data_game() {
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("foo.dl-bin"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/data/game/foo.dl-bin"));
    }

    #[test]
    fn injector_at_install_root() {
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("dxgi.dll"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/dxgi.dll"));
    }

    #[test]
    fn data_prefix_stripped() {
        let _gate = TEST_GATE.lock().unwrap_or_else(|e| e.into_inner());
        plugin().prepare_deploy(Path::new("/game")).unwrap();
        let dest = plugin()
            .resolve_deploy_root(Path::new("/game"), Path::new("data/abc.patch_0"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/data/abc.patch_0"));
    }
}
