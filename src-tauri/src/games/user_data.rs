//! Shared resolution of per-user game data folders (`Documents`, `AppData`).
//!
//! Games that keep mods outside the install directory need the Windows user
//! profile that owns the save data. Three layouts are supported:
//!
//! ```text
//! Native Windows   %USERPROFILE%/Documents, %APPDATA%, %LOCALAPPDATA%
//! Steam Proton     <steamapps>/compatdata/<app id>/pfx/drive_c/users/steamuser
//! Wine prefix      <prefix>/drive_c/users/<name>   (Lutris / Bottles / Heroic)
//! ```

use std::path::{Path, PathBuf};

/// Wine user directories that never hold a real profile.
const RESERVED_PREFIX_USERS: &[&str] = &["public", "default", "default user", "all users"];

/// Walk up from a game install to the owning Steam `steamapps` directory.
pub fn find_steamapps_dir(install_path: &Path) -> Option<PathBuf> {
    for ancestor in install_path.ancestors() {
        if ancestor
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("steamapps"))
        {
            return Some(ancestor.to_path_buf());
        }
        // Installs are typically .../steamapps/common/<Game>.
        if ancestor
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("common"))
        {
            if let Some(parent) = ancestor.parent() {
                if parent
                    .file_name()
                    .is_some_and(|n| n.eq_ignore_ascii_case("steamapps"))
                {
                    return Some(parent.to_path_buf());
                }
            }
        }
    }
    None
}

/// `<steamapps>/compatdata/<app id>/pfx/drive_c/users/steamuser`.
pub fn proton_user_dir(install_path: &Path, steam_app_id: &str) -> Option<PathBuf> {
    if steam_app_id.is_empty() {
        return None;
    }
    let steamapps = find_steamapps_dir(install_path)?;
    Some(
        steamapps
            .join("compatdata")
            .join(steam_app_id)
            .join("pfx")
            .join("drive_c")
            .join("users")
            .join("steamuser"),
    )
}

/// `<prefix>/drive_c/users/<name>` for installs inside a plain Wine prefix.
///
/// Used by Lutris, Bottles, and Heroic, where an EA App / Origin install lives at
/// `<prefix>/drive_c/Program Files/...` instead of under `steamapps`.
pub fn wine_user_dir(install_path: &Path) -> Option<PathBuf> {
    let drive_c = install_path.ancestors().find(|ancestor| {
        ancestor
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("drive_c"))
    })?;
    let users = drive_c.join("users");
    if !users.is_dir() {
        return None;
    }
    for candidate in preferred_prefix_users() {
        let dir = users.join(&candidate);
        if dir.is_dir() {
            return Some(dir);
        }
    }
    // Fall back to the only real profile in the prefix, when there is exactly one.
    let mut profiles = std::fs::read_dir(&users)
        .ok()?
        .flatten()
        .filter(|entry| entry.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|name| {
                    let lower = name.to_lowercase();
                    !RESERVED_PREFIX_USERS.contains(&lower.as_str())
                })
        });
    let only = profiles.next()?;
    if profiles.next().is_some() {
        return None;
    }
    Some(only)
}

fn preferred_prefix_users() -> Vec<String> {
    let mut names = vec!["steamuser".to_string()];
    for var in ["USER", "USERNAME"] {
        if let Some(value) = std::env::var_os(var).and_then(|v| v.into_string().ok()) {
            let trimmed = value.trim().to_string();
            if !trimmed.is_empty() && !names.contains(&trimmed) {
                names.push(trimmed);
            }
        }
    }
    names
}

fn native_documents_dir() -> Option<PathBuf> {
    directories::UserDirs::new().and_then(|dirs| dirs.document_dir().map(|d| d.to_path_buf()))
}

/// The `Documents` folder that owns this install's save data.
pub fn documents_dir(install_path: &Path, steam_app_id: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(native) = native_documents_dir() {
            return Some(native);
        }
    }
    proton_user_dir(install_path, steam_app_id)
        .map(|user| user.join("Documents"))
        .or_else(|| wine_user_dir(install_path).map(|user| user.join("Documents")))
        .or_else(native_documents_dir)
}

/// The roaming `AppData` folder that owns this install's save data.
pub fn appdata_roaming_dir(install_path: &Path, steam_app_id: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(native) = std::env::var_os("APPDATA").map(PathBuf::from) {
            return Some(native);
        }
    }
    prefix_user_dir(install_path, steam_app_id).map(|user| user.join("AppData").join("Roaming"))
}

/// The local `AppData` folder that owns this install's save data.
pub fn appdata_local_dir(install_path: &Path, steam_app_id: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        if let Some(native) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            return Some(native);
        }
    }
    prefix_user_dir(install_path, steam_app_id).map(|user| user.join("AppData").join("Local"))
}

fn prefix_user_dir(install_path: &Path, steam_app_id: &str) -> Option<PathBuf> {
    proton_user_dir(install_path, steam_app_id).or_else(|| wine_user_dir(install_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steamapps_from_common_install() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("steamapps").join("common").join("Game");
        std::fs::create_dir_all(&install).unwrap();
        assert_eq!(
            find_steamapps_dir(&install),
            Some(tmp.path().join("steamapps"))
        );
    }

    #[test]
    fn proton_documents_for_steam_install() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("steamapps").join("common").join("Game");
        std::fs::create_dir_all(&install).unwrap();
        let docs = documents_dir(&install, "1234").unwrap();
        assert_eq!(
            docs,
            tmp.path()
                .join("steamapps")
                .join("compatdata")
                .join("1234")
                .join("pfx")
                .join("drive_c")
                .join("users")
                .join("steamuser")
                .join("Documents")
        );
    }

    #[test]
    fn wine_prefix_documents_prefers_steamuser() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("ea-app");
        let install = prefix
            .join("drive_c")
            .join("Program Files")
            .join("EA Games")
            .join("The Sims 4");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(prefix.join("drive_c").join("users").join("steamuser")).unwrap();
        std::fs::create_dir_all(prefix.join("drive_c").join("users").join("Public")).unwrap();

        let docs = documents_dir(&install, "").unwrap();
        assert_eq!(
            docs,
            prefix
                .join("drive_c")
                .join("users")
                .join("steamuser")
                .join("Documents")
        );
    }

    #[test]
    fn wine_prefix_falls_back_to_single_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("bottle");
        let install = prefix.join("drive_c").join("Games").join("Game");
        std::fs::create_dir_all(&install).unwrap();
        std::fs::create_dir_all(prefix.join("drive_c").join("users").join("simmer")).unwrap();
        std::fs::create_dir_all(prefix.join("drive_c").join("users").join("Public")).unwrap();

        assert_eq!(
            wine_user_dir(&install),
            Some(prefix.join("drive_c").join("users").join("simmer"))
        );
    }

    #[test]
    fn no_prefix_and_no_steamapps_yields_no_appdata() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("Game");
        std::fs::create_dir_all(&install).unwrap();
        #[cfg(not(windows))]
        assert_eq!(appdata_roaming_dir(&install, "1234"), None);
    }
}
