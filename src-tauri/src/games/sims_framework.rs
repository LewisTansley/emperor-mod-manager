//! Pieces shared by the two Sims plugins: `Resource.cfg` upkeep and cache clearing.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const RESOURCE_CFG_NAME: &str = "Resource.cfg";

/// Find a file in `dir` ignoring case.
///
/// Sims installs run through Wine, where the prefix is case-insensitive but the
/// underlying Linux filesystem is not: a `resource.cfg` fetched from a CC site and
/// a game-authored `Resource.cfg` are the same file to the game, and we must not
/// create a second one.
pub fn find_file_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|found| found.eq_ignore_ascii_case(name))
        })
}

/// Write `contents` as `Resource.cfg` when the Mods folder has none.
///
/// An existing file is never rewritten — it may carry hand-tuned priorities — but
/// one that cannot reach nested folders earns a warning naming the missing line.
pub fn ensure_resource_cfg(
    mods_dir: &Path,
    contents: &str,
    depth_marker: &str,
) -> Result<Vec<String>> {
    if let Some(existing) = find_file_ci(mods_dir, RESOURCE_CFG_NAME) {
        let raw = std::fs::read_to_string(&existing)
            .with_context(|| format!("read {}", existing.display()))?;
        if !raw.contains(depth_marker) {
            return Ok(vec![format!(
                "{} does not scan nested folders (no `PackedFile {depth_marker}`), so mods in subfolders may not load. Add that line, or delete the file to have it regenerated.",
                existing.display()
            )]);
        }
        return Ok(Vec::new());
    }
    std::fs::create_dir_all(mods_dir).with_context(|| format!("create {}", mods_dir.display()))?;
    let cfg = mods_dir.join(RESOURCE_CFG_NAME);
    std::fs::write(&cfg, contents).with_context(|| format!("write {}", cfg.display()))?;
    Ok(Vec::new())
}

/// Delete caches the game rebuilds on next launch.
///
/// Stale caches are the most common reason a freshly installed mod appears to do
/// nothing, so this runs after every deploy. Failures are ignored: a cache we
/// could not remove is a slower first launch, not a broken deploy.
pub fn clear_caches(files: &[PathBuf]) {
    for file in files {
        if file.is_file() {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// Delete every file directly inside `dir`, leaving the folder in place.
pub fn clear_dir_files(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_file()).unwrap_or(false) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_existing_cfg_regardless_of_case() {
        let tmp = tempfile::tempdir().unwrap();
        let lower = tmp.path().join("resource.cfg");
        std::fs::write(&lower, "PackedFile */*.package\n").unwrap();

        let warnings = ensure_resource_cfg(tmp.path(), "generated", "*/*.package").unwrap();
        assert!(warnings.is_empty());
        assert!(!tmp.path().join(RESOURCE_CFG_NAME).exists());
        assert_eq!(
            std::fs::read_to_string(&lower).unwrap(),
            "PackedFile */*.package\n"
        );
    }

    #[test]
    fn writes_cfg_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let mods = tmp.path().join("Mods");
        let warnings = ensure_resource_cfg(&mods, "generated", "marker").unwrap();
        assert!(warnings.is_empty());
        assert_eq!(
            std::fs::read_to_string(mods.join(RESOURCE_CFG_NAME)).unwrap(),
            "generated"
        );
    }

    #[test]
    fn clear_dir_files_keeps_subfolders() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("stale.dat"), b"x").unwrap();
        std::fs::create_dir_all(tmp.path().join("keep")).unwrap();

        clear_dir_files(tmp.path());
        assert!(!tmp.path().join("stale.dat").exists());
        assert!(tmp.path().join("keep").is_dir());
    }
}
