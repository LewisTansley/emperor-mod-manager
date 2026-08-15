use std::path::{Component, Path, PathBuf};

use anyhow::Result;

use super::{normalize_relative, GamePlugin, GamePluginInfo};

/// Top-level folders that belong at the Cyberpunk 2077 install root.
pub const CYBERPUNK_ROOT_DIRS: &[&str] = &["archive", "bin", "engine", "mods", "r6", "red4ext"];

pub struct Cyberpunk2077Plugin;

impl GamePlugin for Cyberpunk2077Plugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "cyberpunk2077",
            display_name: "Cyberpunk 2077",
            nexus_domain: "cyberpunk2077",
            match_names: &["cyberpunk 2077", "cyberpunk2077"],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        CYBERPUNK_ROOT_DIRS
    }

    fn should_wrap_as_mod_folder(&self, content_root: &Path) -> bool {
        // REDmod packs use mods/<ModName>/info.json after a wrapper folder is peeled.
        content_root.join("info.json").is_file()
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        let normalized = normalize_relative(relative);
        let components = path_components_lower(&normalized);
        if components.is_empty() {
            return Ok(install_path.to_path_buf());
        }

        let first = components[0].as_str();

        // Known install-root trees: rejoin with canonical lowercase segment names.
        if CYBERPUNK_ROOT_DIRS.iter().any(|r| *r == first) {
            return Ok(join_canonical(install_path, &normalized, &components));
        }

        // Flat archive / ArchiveXL files (and only those — not nested dirs named oddly).
        let is_single = components.len() == 1;
        let leaf = components.last().map(|s| s.as_str()).unwrap_or("");
        if is_single && (leaf.ends_with(".archive") || leaf.ends_with(".xl")) {
            return Ok(install_path
                .join("archive")
                .join("pc")
                .join("mod")
                .join(normalized.file_name().unwrap_or(normalized.as_os_str())));
        }

        // Nested path that already points at archive/pc/mod/...
        if components.len() >= 3
            && components[0] == "archive"
            && components[1] == "pc"
            && components[2] == "mod"
        {
            return Ok(join_canonical(install_path, &normalized, &components));
        }

        // Default: REDmod / leftover content under mods/
        Ok(install_path.join("mods").join(&normalized))
    }
}

fn path_components_lower(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect()
}

fn join_canonical(install_path: &Path, normalized: &Path, lower_components: &[String]) -> PathBuf {
    let mut out = install_path.to_path_buf();
    let originals: Vec<_> = normalized
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();

    for (i, lower) in lower_components.iter().enumerate() {
        if i == 0 && CYBERPUNK_ROOT_DIRS.iter().any(|r| *r == lower.as_str()) {
            out.push(lower.as_str());
        } else if i < 3
            && lower_components.first().map(|s| s.as_str()) == Some("archive")
            && matches!(lower.as_str(), "archive" | "pc" | "mod")
            && matches!(i, 0 | 1 | 2)
        {
            out.push(lower.as_str());
        } else if let Some(orig) = originals.get(i) {
            out.push(orig);
        } else {
            out.push(lower.as_str());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::GamePlugin;

    fn plugin() -> Cyberpunk2077Plugin {
        Cyberpunk2077Plugin
    }

    #[test]
    fn preserves_bin_red4ext_engine_r6_archive() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("bin/x64/version.dll"))
                .unwrap(),
            PathBuf::from("/game/bin/x64/version.dll")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("red4ext/plugins/Foo/foo.dll"))
                .unwrap(),
            PathBuf::from("/game/red4ext/plugins/Foo/foo.dll")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("engine/config/base/scripts.ini"))
                .unwrap(),
            PathBuf::from("/game/engine/config/base/scripts.ini")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("r6/scripts/Mod/mod.reds"))
                .unwrap(),
            PathBuf::from("/game/r6/scripts/Mod/mod.reds")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("archive/pc/mod"))
                .unwrap(),
            PathBuf::from("/game/archive/pc/mod")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("archive/pc/mod/Foo.archive"))
                .unwrap(),
            PathBuf::from("/game/archive/pc/mod/Foo.archive")
        );
    }

    #[test]
    fn flat_archive_files_go_to_archive_pc_mod() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("Foo.archive"))
                .unwrap(),
            PathBuf::from("/game/archive/pc/mod/Foo.archive")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("Foo.archive.xl"))
                .unwrap(),
            PathBuf::from("/game/archive/pc/mod/Foo.archive.xl")
        );
    }

    #[test]
    fn does_not_flatten_archive_directory_to_mod_mod() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("archive/pc/mod"))
                .unwrap(),
            PathBuf::from("/game/archive/pc/mod")
        );
    }

    #[test]
    fn normalizes_windows_separators() {
        let install = Path::new("/game");
        let p = plugin();
        let weird = PathBuf::from(r"r6\scripts\RedHttpClient\RedHttpClient.reds");
        assert_eq!(
            p.resolve_deploy_root(install, &weird).unwrap(),
            PathBuf::from("/game/r6/scripts/RedHttpClient/RedHttpClient.reds")
        );
    }

    #[test]
    fn case_insensitive_root_prefixes() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("BIN/x64/plugins/a.asi"))
                .unwrap(),
            PathBuf::from("/game/bin/x64/plugins/a.asi")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("R6/scripts/x.reds"))
                .unwrap(),
            PathBuf::from("/game/r6/scripts/x.reds")
        );
    }

    #[test]
    fn default_lands_under_mods() {
        let install = Path::new("/game");
        let p = plugin();
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("some/loose/file.txt"))
                .unwrap(),
            PathBuf::from("/game/mods/some/loose/file.txt")
        );
    }

    #[test]
    fn redmod_wrap_detection() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("info.json"), "{}").unwrap();
        assert!(plugin().should_wrap_as_mod_folder(dir.path()));
        let empty = tempfile::tempdir().unwrap();
        assert!(!plugin().should_wrap_as_mod_folder(empty.path()));
    }

    #[test]
    fn normalize_relative_splits_backslashes() {
        let p = PathBuf::from(r"bin\x64\version.dll");
        assert_eq!(
            normalize_relative(&p),
            PathBuf::from("bin/x64/version.dll")
        );
    }
}
