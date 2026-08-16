//! Linux startup hardening before GTK/Tauri initializes.
//!
//! GTK aborts hard when icon loading hits a write error (e.g. full `/tmp` /
//! disk quota) during `tao::Window::setup_signals`. Prefer a writable TMPDIR
//! under the user cache when the default temp dir cannot accept writes.

use std::fs;
use std::path::PathBuf;

const MIN_PROBE: &[u8] = b"emm-tmp-ok";

/// Ensure `TMPDIR` can accept small writes before GTK starts.
///
/// Returns the TMPDIR in effect after any fallback.
pub fn ensure_writable_tmpdir() -> PathBuf {
    let current = std::env::temp_dir();
    if probe_writable(&current) {
        return current;
    }

    let fallback = fallback_tmpdir();
    if let Err(e) = fs::create_dir_all(&fallback) {
        eprintln!(
            "warning: default temp dir {:?} is not writable and could not create fallback {:?}: {e}",
            current, fallback
        );
        return current;
    }

    if !probe_writable(&fallback) {
        eprintln!(
            "warning: default temp dir {:?} is not writable and fallback {:?} also failed writes; GTK may abort on icon load",
            current, fallback
        );
        return current;
    }

    eprintln!(
        "warning: default temp dir {:?} is not writable (disk full/quota); using {:?}",
        current, fallback
    );
    // SAFETY: called once from main before any threads / GTK init.
    unsafe {
        std::env::set_var("TMPDIR", &fallback);
    }
    fallback
}

fn fallback_tmpdir() -> PathBuf {
    if let Some(dirs) =
        directories::ProjectDirs::from("dev", "emperormodmanager", crate::config::APP_NAME)
    {
        return dirs.cache_dir().join("tmp");
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".cache/emperor-mod-manager/tmp")
}

fn probe_writable(dir: &std::path::Path) -> bool {
    let path = dir.join(format!(".emm-tmp-probe-{}", std::process::id()));
    match fs::write(&path, MIN_PROBE) {
        Ok(()) => {
            let _ = fs::remove_file(&path);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn probe_rejects_unwritable_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut perms = fs::metadata(dir.path()).unwrap().permissions();
        perms.set_mode(0o555);
        fs::set_permissions(dir.path(), perms).unwrap();
        assert!(!probe_writable(dir.path()));
    }

    #[test]
    fn probe_accepts_writable_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(probe_writable(dir.path()));
    }
}
