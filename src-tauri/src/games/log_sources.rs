//! Shared loader-log locations. Game plugins call these from `load_log_sources`.

use std::fs;
use std::path::{Path, PathBuf};

use super::{LogKind, LogSource};

const SKIP_DIRS: &[&str] = &[
    "natives", "content", "engine", "paks", "pak", "redist", "movies", "~mods",
];

/// Unity `Player.log` under a known Proton prefix, at most a few levels under `LocalLow`.
pub(crate) fn proton_player_logs(install_path: &Path) -> Vec<LogSource> {
    let Some(app_id) =
        crate::tools::executables::steam_app_id_from_install(&install_path.to_string_lossy())
    else {
        return Vec::new();
    };
    let Some(user) = super::user_data::proton_user_dir(install_path, &app_id) else {
        return Vec::new();
    };
    let local_low = user.join("AppData").join("LocalLow");
    let mut found = Vec::new();
    collect_matching(&local_low, 3, &mut found, &|name| {
        name.eq_ignore_ascii_case("Player.log")
    });
    found.sort_by_key(|path| fs::metadata(path).and_then(|meta| meta.modified()).ok());
    found.reverse();
    found.truncate(2);
    found
        .into_iter()
        .map(|path| LogSource {
            label: "Unity Player.log".into(),
            kind: LogKind::Player,
            path,
        })
        .collect()
}

pub(crate) fn reframework_log_sources(install_path: &Path) -> Vec<LogSource> {
    let mut paths = Vec::new();
    collect_matching(install_path, 2, &mut paths, &|name| {
        let lower = name.to_lowercase();
        lower == "reframework.log" || lower.ends_with("_framework_log.txt")
    });
    if paths.is_empty() {
        paths.push(install_path.join("reframework.log"));
    }
    paths
        .into_iter()
        .map(|path| LogSource {
            label: "REFramework".into(),
            kind: LogKind::ReFramework,
            path,
        })
        .collect()
}

/// `UE4SS.log` next to the game binary (`<Project>/Binaries/<Platform>/ue4ss/`).
pub(crate) fn ue4ss_log_sources(install_path: &Path) -> Vec<LogSource> {
    let mut paths = Vec::new();
    consider_ue4ss(&mut paths, install_path);
    if let Ok(entries) = fs::read_dir(install_path) {
        for entry in entries.flatten() {
            let project = entry.path();
            if !project.is_dir() {
                continue;
            }
            consider_ue4ss(&mut paths, &project);
            let binaries = project.join("Binaries");
            if !binaries.is_dir() {
                continue;
            }
            if let Ok(platforms) = fs::read_dir(&binaries) {
                for platform in platforms.flatten() {
                    consider_ue4ss(&mut paths, &platform.path());
                }
            }
        }
    }
    if paths.is_empty() {
        paths.push(install_path.join("ue4ss").join("UE4SS.log"));
    }
    paths
        .into_iter()
        .map(|path| LogSource {
            label: "UE4SS".into(),
            kind: LogKind::Ue4ss,
            path,
        })
        .collect()
}

fn consider_ue4ss(paths: &mut Vec<PathBuf>, dir: &Path) {
    for candidate in [dir.join("UE4SS.log"), dir.join("ue4ss").join("UE4SS.log")] {
        if candidate.is_file() {
            paths.push(candidate);
        }
    }
}

fn collect_matching(
    root: &Path,
    max_depth: usize,
    out: &mut Vec<PathBuf>,
    pred: &dyn Fn(&str) -> bool,
) {
    fn walk(
        dir: &Path,
        depth: usize,
        max_depth: usize,
        out: &mut Vec<PathBuf>,
        pred: &dyn Fn(&str) -> bool,
    ) {
        if out.len() >= 8 || depth > max_depth {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            if out.len() >= 8 {
                return;
            }
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let is_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
            let is_file = entry
                .file_type()
                .map(|kind| kind.is_file())
                .unwrap_or(false);
            if is_file && pred(&name) {
                out.push(path);
            } else if is_dir && depth < max_depth {
                if SKIP_DIRS.iter().any(|skip| name.eq_ignore_ascii_case(skip)) {
                    continue;
                }
                walk(&path, depth + 1, max_depth, out, pred);
            }
        }
    }
    walk(root, 0, max_depth, out, pred);
}
