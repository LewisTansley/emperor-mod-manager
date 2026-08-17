//! Blade & Sorcery SDK mods under StreamingAssets/Mods.

use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

pub const BAS_PRESERVE_ROOTS: &[&str] = &["Mods", "Default", "StreamingAssets"];

pub struct BladeAndSorceryPlugin;

impl GamePlugin for BladeAndSorceryPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "bladeandsorcery",
            display_name: "Blade & Sorcery",
            nexus_domain: "bladeandsorcery",
            match_names: &[
                "blade & sorcery",
                "blade and sorcery",
                "bladeandsorcery",
            ],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        BAS_PRESERVE_ROOTS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        looks_like_sdk_mod(content_root)
    }

    fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
        content_folder_wrap_name(content_root, staged_name)
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        let streaming = streaming_assets_dir(install_path);
        let normalized = normalize_relative(relative);
        let originals: Vec<_> = normalized
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_os_string()),
                _ => None,
            })
            .collect();
        if originals.is_empty() {
            return Ok(streaming.join("Mods"));
        }

        let first = originals[0].to_string_lossy();
        let first_lower = first.to_lowercase();
        if first_lower == "streamingassets" {
            let mut out = data_dir(install_path);
            for orig in originals {
                out.push(orig);
            }
            return Ok(out);
        }
        if first_lower == "mods" || first_lower == "default" {
            let mut out = streaming;
            for orig in originals {
                out.push(orig);
            }
            return Ok(out);
        }

        let dest_root = if looks_like_overwrite_relative(&originals) {
            streaming.join("Default")
        } else {
            streaming.join("Mods")
        };
        Ok(dest_root.join(&normalized))
    }
}

fn looks_like_sdk_mod(content_root: &Path) -> bool {
    content_root.join("manifest.json").is_file()
}

fn looks_like_overwrite_relative(originals: &[std::ffi::OsString]) -> bool {
    originals.iter().any(|s| {
        let n = s.to_string_lossy().to_lowercase();
        n == "default" || n.ends_with(".asset") || n.ends_with(".bundle")
    }) && !originals.iter().any(|s| s.to_string_lossy().eq_ignore_ascii_case("Mods"))
}

fn data_dir(install_path: &Path) -> PathBuf {
    if let Ok(entries) = std::fs::read_dir(install_path) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with("_Data") && entry.path().join("StreamingAssets").is_dir() {
                return entry.path();
            }
        }
    }
    install_path.join("BladeAndSorcery_Data")
}

fn streaming_assets_dir(install_path: &Path) -> PathBuf {
    data_dir(install_path).join("StreamingAssets")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin() -> BladeAndSorceryPlugin {
        BladeAndSorceryPlugin
    }

    #[test]
    fn sdk_mod_wraps_under_streamingassets_mods() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("manifest.json"), "{}").unwrap();
        assert!(plugin().should_wrap_as_mod_folder(dir.path()));
        let dest = plugin()
            .resolve_deploy_root(
                Path::new("/game"),
                Path::new("MyMod/manifest.json"),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/BladeAndSorcery_Data/StreamingAssets/Mods/MyMod/manifest.json")
        );
    }

    #[test]
    fn preserves_mods_prefix() {
        let dest = plugin()
            .resolve_deploy_root(
                Path::new("/game"),
                Path::new("Mods/MyMod/manifest.json"),
            )
            .unwrap();
        assert_eq!(
            dest,
            PathBuf::from("/game/BladeAndSorcery_Data/StreamingAssets/Mods/MyMod/manifest.json")
        );
    }

    #[test]
    fn uses_detected_data_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("BladeAndSorcery_Data");
        std::fs::create_dir_all(data.join("StreamingAssets").join("Mods")).unwrap();
        let dest = plugin()
            .resolve_deploy_root(tmp.path(), Path::new("Cool/manifest.json"))
            .unwrap();
        assert_eq!(
            dest,
            data.join("StreamingAssets").join("Mods").join("Cool").join("manifest.json")
        );
    }
}
