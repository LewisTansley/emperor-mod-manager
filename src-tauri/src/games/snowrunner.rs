//! SnowRunner (preload/paks/client). Official discovery is the in-game mod.io browser.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{normalize_relative, GamePlugin, GamePluginInfo};

pub const SNOWRUNNER_ROOT_DIRS: &[&str] = &["preload", "en_us", "paks", "client"];

pub struct SnowRunnerPlugin;

impl GamePlugin for SnowRunnerPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "snowrunner",
            display_name: "SnowRunner",
            nexus_domain: "snowrunner",
            match_names: &["snowrunner", "snow runner"],
        }
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        SNOWRUNNER_ROOT_DIRS
    }

    fn preflight_warnings(&self, _install_path: &Path) -> Vec<String> {
        vec![
            "SnowRunner's native mod browser is in-game via mod.io. Linked paks go under preload/paks/client — verify in-game that the mod appeared."
                .into(),
        ]
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        Ok(resolve_snowrunner_deploy(install_path, relative))
    }
}

fn client_paks_dir(install_path: &Path) -> PathBuf {
    let en_us = install_path
        .join("en_us")
        .join("preload")
        .join("paks")
        .join("client");
    if en_us.is_dir() || install_path.join("en_us").join("preload").is_dir() {
        return en_us;
    }
    install_path
        .join("preload")
        .join("paks")
        .join("client")
}

fn resolve_snowrunner_deploy(install_path: &Path, relative: &Path) -> PathBuf {
    let client = client_paks_dir(install_path);
    let normalized = normalize_relative(relative);
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect();
    if originals.is_empty() {
        return client;
    }
    let first = originals[0].to_string_lossy().to_lowercase();
    if first == "preload" || first == "en_us" || first == "paks" || first == "client" {
        let mut out = install_path.to_path_buf();
        if first == "paks" {
            out.push("preload");
        } else if first == "client" {
            out.push("preload");
            out.push("paks");
        }
        for orig in originals {
            out.push(orig);
        }
        return out;
    }
    client.join(&normalized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_pak_goes_to_preload_client() {
        let dest = SnowRunnerPlugin
            .resolve_deploy_root(Path::new("/game"), Path::new("Cool.pak"))
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/preload/paks/client/Cool.pak")
        );
    }

    #[test]
    fn preserves_preload_tree() {
        let dest = SnowRunnerPlugin
            .resolve_deploy_root(
                Path::new("/game"),
                Path::new("preload/paks/client/Cool.pak"),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/preload/paks/client/Cool.pak")
        );
    }

    #[test]
    fn prefers_en_us_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("en_us").join("preload")).unwrap();
        let dest = SnowRunnerPlugin
            .resolve_deploy_root(tmp.path(), Path::new("Cool.pak"))
            .unwrap();
        assert_eq!(
            dest,
            tmp.path()
                .join("en_us")
                .join("preload")
                .join("paks")
                .join("client")
                .join("Cool.pak")
        );
    }
}
