//! Kingdom Come: Deliverance 1 and 2 (CryEngine Mods/ + mod_order.txt).

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};

use super::{content_folder_wrap_name, normalize_relative, GamePlugin, GamePluginInfo};

pub const KCD_ROOT_DIRS: &[&str] = &["Mods"];

fn resolve_kcd_deploy(install_path: &Path, relative: &Path) -> PathBuf {
    let normalized = normalize_relative(relative);
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_os_string()),
            _ => None,
        })
        .collect();
    if originals.is_empty() {
        return install_path.join("Mods");
    }
    let first = originals[0].to_string_lossy();
    if first.eq_ignore_ascii_case("Mods") {
        let mut out = install_path.join("Mods");
        for orig in originals.into_iter().skip(1) {
            out.push(orig);
        }
        return out;
    }
    install_path.join("Mods").join(&normalized)
}

fn looks_like_kcd_mod(content_root: &Path) -> bool {
    if content_root.join("mod.manifest").is_file() {
        return true;
    }
    let data = content_root.join("Data");
    if data.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&data) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_lowercase();
                if name.ends_with(".pak") {
                    return true;
                }
            }
        }
    }
    false
}

pub fn write_mod_order(install_path: &Path, enabled_mod_folders: &[String]) -> Result<()> {
    let mods_dir = install_path.join("Mods");
    std::fs::create_dir_all(&mods_dir)
        .with_context(|| format!("create {}", mods_dir.display()))?;
    let path = mods_dir.join("mod_order.txt");
    let mut body = String::new();
    for name in enabled_mod_folders {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("mod_order.txt") {
            continue;
        }
        body.push_str(trimmed);
        body.push('\n');
    }
    std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

macro_rules! kcd_plugin {
    ($struct:ident, $id:expr, $display:expr, $domain:expr, $matches:expr) => {
        pub struct $struct;

        impl GamePlugin for $struct {
            fn info(&self) -> GamePluginInfo {
                GamePluginInfo {
                    id: $id,
                    display_name: $display,
                    nexus_domain: $domain,
                    match_names: $matches,
                }
            }

            fn prefers_mod_folder(&self) -> bool {
                false
            }

            fn preserve_staging_root_names(&self) -> &[&str] {
                KCD_ROOT_DIRS
            }

            fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
                looks_like_kcd_mod(content_root)
            }

            fn wrap_mod_folder_name(&self, content_root: &Path, staged_name: &str) -> String {
                content_folder_wrap_name(content_root, staged_name)
            }

            fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
                Ok(resolve_kcd_deploy(install_path, relative))
            }

            fn after_deploy(
                &self,
                install_path: &Path,
                enabled_mod_folders: &[String],
            ) -> Result<Vec<String>> {
                write_mod_order(install_path, enabled_mod_folders)?;
                Ok(Vec::new())
            }
        }
    };
}

kcd_plugin!(
    KingdomComeDeliverance2Plugin,
    "kingdomcomedeliverance2",
    "Kingdom Come: Deliverance 2",
    "kingdomcomedeliverance2",
    &[
        "kingdom come deliverance 2",
        "kingdom come: deliverance 2",
        "kingdom come deliverance ii",
        "kingdom come: deliverance ii",
        "kingdomcomedeliverance2",
        "kcd2",
    ]
);

kcd_plugin!(
    KingdomComeDeliverancePlugin,
    "kingdomcomedeliverance",
    "Kingdom Come: Deliverance",
    "kingdomcomedeliverance",
    &[
        "kingdom come deliverance",
        "kingdom come: deliverance",
        "kingdomcomedeliverance",
    ]
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::match_plugin;

    #[test]
    fn wraps_on_manifest_or_pak() {
        let manifest = tempfile::tempdir().unwrap();
        std::fs::write(manifest.path().join("mod.manifest"), "<kcd_mod/>").unwrap();
        assert!(KingdomComeDeliverance2Plugin.should_wrap_as_mod_folder(manifest.path()));

        let pak = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(pak.path().join("Data")).unwrap();
        std::fs::write(pak.path().join("Data").join("mod.pak"), b"x").unwrap();
        assert!(KingdomComeDeliverancePlugin.should_wrap_as_mod_folder(pak.path()));
    }

    #[test]
    fn deploys_under_mods() {
        let dest = KingdomComeDeliverance2Plugin
            .resolve_deploy_root(Path::new("/game"), Path::new("MyMod/mod.manifest"))
            .unwrap();
        assert_eq!(dest, PathBuf::from("/game/Mods/MyMod/mod.manifest"));
    }

    #[test]
    fn writes_whitelist_mod_order() {
        let tmp = tempfile::tempdir().unwrap();
        write_mod_order(tmp.path(), &["modB".into(), "modA".into()]).unwrap();
        let raw = std::fs::read_to_string(tmp.path().join("Mods").join("mod_order.txt")).unwrap();
        assert_eq!(raw, "modB\nmodA\n");
    }

    #[test]
    fn match_kcd2_before_kcd1() {
        assert_eq!(
            match_plugin("Kingdom Come: Deliverance 2", None)
                .unwrap()
                .id,
            "kingdomcomedeliverance2"
        );
        assert_eq!(
            match_plugin("Kingdom Come: Deliverance", None)
                .unwrap()
                .id,
            "kingdomcomedeliverance"
        );
    }
}
