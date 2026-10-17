//! Auto-install FromSoftware loaders (legacy Mod Engine, Mod Engine 2, me3) on deploy.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use walkdir::WalkDir;

use crate::config::Paths;
use crate::github_release::{self, GhAsset, GhRelease};
use crate::nexus::NexusClient;
use crate::tools::install::{self as tool_install};

use super::fromsoftware::{self, resolve_game_root};

/// Free-account Download Assist target for a curated legacy Mod Engine pack.
#[derive(Debug, Clone)]
pub struct LegacyAssistRequest {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub plugin_id: String,
    pub install_path: PathBuf,
    pub label: String,
}

/// Outcome of [`ensure_fromsoft_loaders`].
#[derive(Debug)]
pub enum EnsureLoadersOutcome {
    Done(Vec<String>),
    /// Open Download Assist for this request, wait for install, then finish deploy.
    NeedsAssist {
        warnings: Vec<String>,
        request: LegacyAssistRequest,
    },
}

const ME2_REPO: &str = "soulsmods/ModEngine2";
const ME3_REPO: &str = "garyttierney/me3";

/// DS2 Mod Engine binaries mirrored from a public mod pack (no Nexus download required).
const DS2_ME_RAW_BASE: &str = "https://raw.githubusercontent.com/vawser/Vestiges-DS2/master";

/// Nexus sources for legacy Mod Engine packs (domain, mod_id).
/// DSR uses community ModEngine2; DS1 uses katalash's DS3 Mod Engine as the
/// canonical ME1 binary distribution (same injector family).
fn legacy_nexus_source(plugin_id: &str) -> Option<(&'static str, u64)> {
    match plugin_id {
        "darksouls" => Some(("darksouls3", 332)),
        "darksoulsremastered" => Some(("darksoulsremastered", 790)),
        _ => None,
    }
}

pub fn is_fromsoft_plugin(plugin_id: &str) -> bool {
    matches!(
        plugin_id,
        "eldenring" | "darksouls3" | "darksouls2" | "darksoulsremastered" | "darksouls"
    )
}

fn is_legacy_plugin(plugin_id: &str) -> bool {
    matches!(
        plugin_id,
        "darksouls" | "darksoulsremastered" | "darksouls2"
    )
}

/// Download / cache missing loaders for a FromSoft title.
pub async fn ensure_fromsoft_loaders(
    paths: &Paths,
    plugin_id: &str,
    install_path: &Path,
    nexus: Option<&NexusClient>,
    nexus_premium: bool,
) -> EnsureLoadersOutcome {
    if !is_fromsoft_plugin(plugin_id) {
        return EnsureLoadersOutcome::Done(Vec::new());
    }
    if is_legacy_plugin(plugin_id) {
        match ensure_legacy_modengine(paths, plugin_id, install_path, nexus, nexus_premium).await {
            Ok(outcome) => outcome,
            Err(e) => EnsureLoadersOutcome::Done(vec![format!(
                "Could not auto-install legacy Mod Engine: {e:#}"
            )]),
        }
    } else {
        match ensure_modern_loaders(paths, plugin_id, install_path).await {
            Ok(extra) => EnsureLoadersOutcome::Done(extra),
            Err(e) => EnsureLoadersOutcome::Done(vec![format!(
                "Could not auto-install me3 / Mod Engine 2: {e:#}"
            )]),
        }
    }
}

/// After Assist installs the loader, copy cache → game if needed and build warnings.
pub fn finish_legacy_after_assist(
    paths: &Paths,
    plugin_id: &str,
    install_path: &Path,
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let cache = paths.fromsoft_legacy_modengine_dir(plugin_id);
    if !cache_has_dinput8(&cache) && !fromsoftware::legacy_modengine_present(install_path) {
        return Err(anyhow!("Mod Engine was not installed via Download Assist"));
    }
    let game_root = resolve_game_root(install_path);
    let installed = copy_legacy_into_game(&cache, &game_root)?;
    if installed {
        warnings.push(format!(
            "Installed legacy Mod Engine (dinput8.dll) into {}.",
            game_root.display()
        ));
        if cfg!(target_os = "linux") {
            warnings.push(
                "On Proton, add WINEDLLOVERRIDES=\"dinput8=n,b\" %command% to the game's Steam launch options so Mod Engine loads."
                    .into(),
            );
        }
    }
    Ok(warnings)
}

pub fn legacy_loader_ready(paths: &Paths, plugin_id: &str, install_path: &Path) -> bool {
    cache_has_dinput8(&paths.fromsoft_legacy_modengine_dir(plugin_id))
        || fromsoftware::legacy_modengine_present(install_path)
}

async fn ensure_legacy_modengine(
    paths: &Paths,
    plugin_id: &str,
    install_path: &Path,
    nexus: Option<&NexusClient>,
    nexus_premium: bool,
) -> Result<EnsureLoadersOutcome> {
    let mut warnings = Vec::new();
    let game_root = resolve_game_root(install_path);

    // DS2: warm the tools cache only. Copying into Game is deferred to after_deploy so
    // Lighting Engine / Seamless-only deploys do not leave a conflicting dinput8.dll.
    if plugin_id == "darksouls2" {
        let cache = paths.fromsoft_legacy_modengine_dir(plugin_id);
        if !cache_has_dinput8(&cache) {
            match download_ds2_modengine_github(&cache).await {
                Ok(()) => {}
                Err(e) => warnings.push(format!(
                    "Could not cache Mod Engine for Dark Souls 2 (needed only for loose asset mods): {e:#}"
                )),
            }
        }
        return Ok(EnsureLoadersOutcome::Done(warnings));
    }

    if fromsoftware::legacy_modengine_present(install_path) {
        return Ok(EnsureLoadersOutcome::Done(warnings));
    }

    let cache = paths.fromsoft_legacy_modengine_dir(plugin_id);
    if !cache_has_dinput8(&cache) {
        if let Some((domain, mod_id)) = legacy_nexus_source(plugin_id) {
            match (nexus, nexus_premium) {
                (Some(client), true) => {
                    download_legacy_from_nexus(client, domain, mod_id, &cache).await?;
                }
                (Some(client), false) => {
                    let files = client.list_mod_files(domain, mod_id).await?;
                    let file = files
                        .iter()
                        .find(|f| f.is_primary)
                        .or_else(|| files.first())
                        .ok_or_else(|| anyhow!("Nexus mod {domain}/{mod_id} has no files"))?;
                    warnings.push(
                        "Download Assist opened for Mod Engine — use Mod Manager Download or Slow Download, then wait for deploy to continue."
                            .into(),
                    );
                    return Ok(EnsureLoadersOutcome::NeedsAssist {
                        warnings,
                        request: LegacyAssistRequest {
                            domain: domain.to_string(),
                            mod_id,
                            file_id: file.file_id,
                            plugin_id: plugin_id.to_string(),
                            install_path: install_path.to_path_buf(),
                            label: format!("Mod Engine ({plugin_id})"),
                        },
                    });
                }
                (None, _) => {
                    warnings.push(
                        "No Nexus API key configured; cannot auto-download legacy Mod Engine. Add an API key in Settings or install dinput8.dll manually."
                            .into(),
                    );
                    return Ok(EnsureLoadersOutcome::Done(warnings));
                }
            }
        } else {
            warnings.push(format!(
                "No known Mod Engine download source for {plugin_id}; install dinput8.dll manually."
            ));
            return Ok(EnsureLoadersOutcome::Done(warnings));
        }
    }

    let installed = copy_legacy_into_game(&cache, &game_root)?;
    if installed {
        warnings.push(format!(
            "Installed legacy Mod Engine (dinput8.dll) into {}.",
            game_root.display()
        ));
        if cfg!(target_os = "linux") {
            warnings.push(
                "On Proton, add WINEDLLOVERRIDES=\"dinput8=n,b\" %command% to the game's Steam launch options so Mod Engine loads."
                    .into(),
            );
        }
    }
    Ok(EnsureLoadersOutcome::Done(warnings))
}

/// Extract a downloaded Mod Engine archive into the tools cache and Game folder.
pub fn install_legacy_from_archive(
    paths: &Paths,
    plugin_id: &str,
    install_path: &Path,
    archive: &Path,
) -> Result<()> {
    let cache = paths.fromsoft_legacy_modengine_dir(plugin_id);
    let extract_to = cache.join("_extract");
    crate::mods::extract_archive(archive, &extract_to)?;
    promote_injector_files(&extract_to, &cache)?;
    let _ = std::fs::remove_dir_all(&extract_to);
    let game_root = resolve_game_root(install_path);
    let _ = copy_legacy_into_game(&cache, &game_root)?;
    Ok(())
}

fn promote_injector_files(extract_to: &Path, cache: &Path) -> Result<()> {
    std::fs::create_dir_all(cache)?;
    if let Some(dll) = find_named_file(extract_to, "dinput8.dll") {
        std::fs::copy(&dll, cache.join("dinput8.dll"))?;
    } else if let Some(dll) = find_named_file(extract_to, "modengine.dll") {
        std::fs::copy(&dll, cache.join("dinput8.dll"))?;
    } else {
        return Err(anyhow!("Mod Engine archive has no dinput8.dll"));
    }
    if let Some(ini) = find_named_file(extract_to, "modengine.ini") {
        std::fs::copy(&ini, cache.join("modengine.ini"))?;
    } else if !cache.join("modengine.ini").is_file() {
        std::fs::write(
            cache.join("modengine.ini"),
            fromsoftware::legacy_modengine_stub_ini().replace(
                "Generated by Emperor Mod Manager",
                "Cached by Emperor Mod Manager",
            ),
        )?;
    }
    Ok(())
}

fn cache_has_dinput8(cache: &Path) -> bool {
    find_named_file(cache, "dinput8.dll").is_some()
}

async fn download_ds2_modengine_github(cache: &Path) -> Result<()> {
    std::fs::create_dir_all(cache).with_context(|| format!("create {}", cache.display()))?;
    for name in ["dinput8.dll", "modengine.ini"] {
        let url = format!("{DS2_ME_RAW_BASE}/{name}");
        let dest = cache.join(name);
        github_release::download_to_path(&url, &dest)
            .await
            .with_context(|| format!("download {name} from Vestiges-DS2"))?;
    }
    Ok(())
}

async fn download_legacy_from_nexus(
    client: &NexusClient,
    domain: &str,
    mod_id: u64,
    cache: &Path,
) -> Result<()> {
    let files = client.list_mod_files(domain, mod_id).await?;
    let file = files
        .iter()
        .find(|f| f.is_primary)
        .or_else(|| files.first())
        .ok_or_else(|| anyhow!("Nexus mod {domain}/{mod_id} has no files"))?;
    let archive_name = file
        .file_name
        .clone()
        .unwrap_or_else(|| format!("modengine-{mod_id}.bin"));
    let download_dir = cache.join("_download");
    std::fs::create_dir_all(&download_dir)?;
    let archive_path = download_dir.join(&archive_name);
    client
        .download_file(
            domain,
            mod_id,
            file.file_id,
            &archive_path,
            true,
            None,
            None,
            0,
            None,
            None,
        )
        .await?;
    let extract_to = cache.join("_extract");
    crate::mods::extract_archive(&archive_path, &extract_to)?;
    promote_injector_files(&extract_to, cache)?;
    let _ = std::fs::remove_dir_all(&download_dir);
    let _ = std::fs::remove_dir_all(&extract_to);
    Ok(())
}

/// Returns true when `dinput8.dll` was newly copied into the game root.
fn copy_legacy_into_game(cache: &Path, game_root: &Path) -> Result<bool> {
    std::fs::create_dir_all(game_root)
        .with_context(|| format!("create {}", game_root.display()))?;
    let dll_src = find_named_file(cache, "dinput8.dll")
        .ok_or_else(|| anyhow!("cached Mod Engine missing dinput8.dll"))?;
    let dll_dest = game_root.join("dinput8.dll");
    let mut copied_dll = false;
    if !dll_dest.is_file() {
        std::fs::copy(&dll_src, &dll_dest)
            .with_context(|| format!("copy dinput8.dll to {}", dll_dest.display()))?;
        copied_dll = true;
    }
    let ini_dest = game_root.join("modengine.ini");
    if !ini_dest.is_file() {
        if let Some(ini_src) = find_named_file(cache, "modengine.ini") {
            std::fs::copy(&ini_src, &ini_dest)?;
        } else {
            std::fs::write(&ini_dest, fromsoftware::legacy_modengine_stub_ini())?;
        }
    }
    Ok(copied_dll)
}

async fn ensure_modern_loaders(
    paths: &Paths,
    plugin_id: &str,
    install_path: &Path,
) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let game_root = resolve_game_root(install_path);

    match ensure_modengine2(paths).await {
        Ok(true) => warnings.push(format!(
            "Installed Mod Engine 2 into {}.",
            paths.modengine2_install_dir().display()
        )),
        Ok(false) => {}
        Err(e) => warnings.push(format!("Mod Engine 2 install skipped: {e:#}")),
    }

    match ensure_me3(paths).await {
        Ok(true) => warnings.push(format!(
            "Installed me3 into {}.",
            paths.me3_install_dir().display()
        )),
        Ok(false) => {}
        Err(e) => warnings.push(format!("me3 install skipped: {e:#}")),
    }

    write_modern_launch_helpers(paths, plugin_id, &game_root)?;
    Ok(warnings)
}

async fn ensure_modengine2(paths: &Paths) -> Result<bool> {
    let dest = paths.modengine2_install_dir();
    if find_me2_launcher(&dest).is_some() {
        return Ok(false);
    }
    let release = github_release::fetch_latest_release(ME2_REPO)
        .await?
        .ok_or_else(|| anyhow!("no Mod Engine 2 releases found"))?;
    let asset = pick_me2_asset(&release).ok_or_else(|| anyhow!("no win64 zip on ME2 release"))?;
    let bytes = github_release::download_bytes(&asset.browser_download_url).await?;
    tool_install::extract_archive(&bytes, &asset.name, &dest)?;
    tool_install::normalize_install_root(&dest)?;
    if find_me2_launcher(&dest).is_none() {
        return Err(anyhow!(
            "Mod Engine 2 archive extracted but modengine2_launcher.exe was not found"
        ));
    }
    Ok(true)
}

async fn ensure_me3(paths: &Paths) -> Result<bool> {
    let dest = paths.me3_install_dir();
    if me3_binary(&dest).is_some() || me3_on_path() {
        return Ok(false);
    }
    let release = github_release::fetch_latest_release(ME3_REPO)
        .await?
        .ok_or_else(|| anyhow!("no me3 releases found"))?;
    let asset = pick_me3_asset(&release, std::env::consts::OS)
        .ok_or_else(|| anyhow!("no me3 portable asset for this OS"))?;
    let bytes = github_release::download_bytes(&asset.browser_download_url).await?;
    tool_install::extract_archive(&bytes, &asset.name, &dest)?;
    tool_install::normalize_install_root(&dest)?;
    // Some archives nest bin/me3
    if me3_binary(&dest).is_none() {
        return Err(anyhow!(
            "me3 archive extracted but me3 binary was not found"
        ));
    }
    Ok(true)
}

pub fn pick_me2_asset(release: &GhRelease) -> Option<&GhAsset> {
    release
        .assets
        .iter()
        .find(|a| {
            let n = a.name.to_ascii_lowercase();
            n.contains("win64") && n.ends_with(".zip")
        })
        .or_else(|| {
            release
                .assets
                .iter()
                .find(|a| a.name.to_ascii_lowercase().ends_with(".zip"))
        })
}

pub fn pick_me3_asset<'a>(release: &'a GhRelease, os: &str) -> Option<&'a GhAsset> {
    match os {
        "linux" => release.assets.iter().find(|a| {
            let n = a.name.to_ascii_lowercase();
            n.contains("linux") && (n.ends_with(".tar.gz") || n.ends_with(".tgz"))
        }),
        "windows" => release.assets.iter().find(|a| {
            let n = a.name.to_ascii_lowercase();
            n.contains("windows") && n.ends_with(".zip") && !n.contains("installer")
        }),
        _ => None,
    }
}

fn find_me2_launcher(dir: &Path) -> Option<PathBuf> {
    find_named_file(dir, "modengine2_launcher.exe")
}

fn me3_binary(dir: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    let names = ["me3.exe", "me3"];
    #[cfg(not(windows))]
    let names = ["me3"];
    for name in names {
        if let Some(p) = find_named_file(dir, name) {
            return Some(p);
        }
    }
    None
}

fn me3_on_path() -> bool {
    Command::new("me3")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn me2_game_type(plugin_id: &str) -> &'static str {
    match plugin_id {
        "darksouls3" => "ds3",
        _ => "er",
    }
}

pub fn write_modern_launch_helpers(paths: &Paths, plugin_id: &str, game_root: &Path) -> Result<()> {
    std::fs::create_dir_all(game_root)?;
    let me3_bin = me3_binary(&paths.me3_install_dir())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "me3".into());
    let profile = game_root.join("emperor.me3");
    let me2_launcher = find_me2_launcher(&paths.modengine2_install_dir());
    let config = game_root.join("config_emperor.toml");

    // Shell helper (Linux / Git Bash).
    let sh = format!(
        "#!/bin/sh\ncd \"$(dirname \"$0\")\" || exit 1\nexec \"{me3}\" launch --profile \"{profile}\"\n",
        me3 = shell_escape(&me3_bin),
        profile = shell_escape(&profile.display().to_string()),
    );
    let sh_path = game_root.join("launch_emperor_me3.sh");
    std::fs::write(&sh_path, sh)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&sh_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&sh_path, perms)?;
    }

    // Windows batch for me3.
    let bat_me3 = format!(
        "@echo off\r\ncd /d \"%~dp0\"\r\n\"{me3}\" launch --profile \"{profile}\"\r\n",
        me3 = me3_bin.replace('/', "\\"),
        profile = profile.display().to_string().replace('/', "\\"),
    );
    std::fs::write(game_root.join("launch_emperor_me3.bat"), bat_me3)?;

    if let Some(launcher) = me2_launcher {
        let bat_me2 = format!(
            "@echo off\r\ncd /d \"%~dp0\"\r\n\"{launcher}\" -t {game} -c \"{config}\"\r\n",
            launcher = launcher.display().to_string().replace('/', "\\"),
            game = me2_game_type(plugin_id),
            config = config.display().to_string().replace('/', "\\"),
        );
        std::fs::write(game_root.join("launch_emperor_me2.bat"), bat_me2)?;
    }

    Ok(())
}

fn shell_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
}

fn find_named_file(root: &Path, name: &str) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }
    for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
        {
            return Some(entry.path().to_path_buf());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn recognizes_fromsoft_plugins() {
        assert!(is_fromsoft_plugin("darksouls2"));
        assert!(is_fromsoft_plugin("eldenring"));
        assert!(!is_fromsoft_plugin("stardewvalley"));
    }

    #[test]
    fn picks_me2_win64_zip() {
        let release = GhRelease {
            tag_name: "release-2.1.0".into(),
            prerelease: false,
            html_url: None,
            assets: vec![
                GhAsset {
                    name: "notes.txt".into(),
                    browser_download_url: "https://example/notes".into(),
                },
                GhAsset {
                    name: "ModEngine-2.1.0.0-win64.zip".into(),
                    browser_download_url: "https://example/me2.zip".into(),
                },
            ],
        };
        assert_eq!(
            pick_me2_asset(&release).map(|a| a.name.as_str()),
            Some("ModEngine-2.1.0.0-win64.zip")
        );
    }

    #[test]
    fn picks_me3_portable_by_os() {
        let release = GhRelease {
            tag_name: "v0.13.0".into(),
            prerelease: false,
            html_url: None,
            assets: vec![
                GhAsset {
                    name: "me3_installer.exe".into(),
                    browser_download_url: "https://example/installer".into(),
                },
                GhAsset {
                    name: "me3-windows-amd64.zip".into(),
                    browser_download_url: "https://example/win".into(),
                },
                GhAsset {
                    name: "me3-linux-amd64.tar.gz".into(),
                    browser_download_url: "https://example/linux".into(),
                },
            ],
        };
        assert_eq!(
            pick_me3_asset(&release, "linux").map(|a| a.name.as_str()),
            Some("me3-linux-amd64.tar.gz")
        );
        assert_eq!(
            pick_me3_asset(&release, "windows").map(|a| a.name.as_str()),
            Some("me3-windows-amd64.zip")
        );
    }

    #[test]
    fn skips_legacy_when_dinput8_present() {
        let tmp = tempfile::tempdir().unwrap();
        let game = tmp.path().join("Game");
        std::fs::create_dir_all(&game).unwrap();
        std::fs::write(game.join("dinput8.dll"), b"x").unwrap();
        assert!(fromsoftware::legacy_modengine_present(tmp.path()));
    }

    #[test]
    fn copy_legacy_does_not_overwrite_existing_dinput8() {
        let cache = tempfile::tempdir().unwrap();
        let game = tempfile::tempdir().unwrap();
        std::fs::write(cache.path().join("dinput8.dll"), b"new").unwrap();
        std::fs::write(cache.path().join("modengine.ini"), b"ini").unwrap();
        std::fs::write(game.path().join("dinput8.dll"), b"old").unwrap();
        assert!(!copy_legacy_into_game(cache.path(), game.path()).unwrap());
        assert_eq!(
            std::fs::read(game.path().join("dinput8.dll")).unwrap(),
            b"old"
        );
    }

    #[test]
    fn install_legacy_from_archive_fills_cache_and_game() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_dir: tmp.path().join("cfg"),
            data_dir: tmp.path().join("data"),
            cache_dir: tmp.path().join("cache"),
        };
        let install = tmp.path().join("game");
        let game_root = install.join("Game");
        std::fs::create_dir_all(&game_root).unwrap();

        let archive = tmp.path().join("me.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file::<_, ()>("dinput8.dll", opts).unwrap();
            zip.write_all(b"dll-bytes").unwrap();
            zip.start_file::<_, ()>("modengine.ini", opts).unwrap();
            zip.write_all(b"ini-bytes").unwrap();
            zip.finish().unwrap();
        }

        install_legacy_from_archive(&paths, "darksouls", &install, &archive).unwrap();

        let cache = paths.fromsoft_legacy_modengine_dir("darksouls");
        assert_eq!(
            std::fs::read(cache.join("dinput8.dll")).unwrap(),
            b"dll-bytes"
        );
        assert_eq!(
            std::fs::read(game_root.join("dinput8.dll")).unwrap(),
            b"dll-bytes"
        );
        assert!(game_root.join("modengine.ini").is_file());
        assert!(legacy_loader_ready(&paths, "darksouls", &install));
    }

    #[test]
    fn install_legacy_from_archive_does_not_overwrite_game_dll() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_dir: tmp.path().join("cfg"),
            data_dir: tmp.path().join("data"),
            cache_dir: tmp.path().join("cache"),
        };
        let install = tmp.path().join("game");
        let game_root = install.join("Game");
        std::fs::create_dir_all(&game_root).unwrap();
        std::fs::write(game_root.join("dinput8.dll"), b"existing").unwrap();

        let archive = tmp.path().join("me.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = zip::write::SimpleFileOptions::default();
            zip.start_file::<_, ()>("dinput8.dll", opts).unwrap();
            zip.write_all(b"new-dll").unwrap();
            zip.start_file::<_, ()>("modengine.ini", opts).unwrap();
            zip.write_all(b"ini").unwrap();
            zip.finish().unwrap();
        }

        install_legacy_from_archive(&paths, "darksoulsremastered", &install, &archive).unwrap();
        assert_eq!(
            std::fs::read(game_root.join("dinput8.dll")).unwrap(),
            b"existing"
        );
        assert_eq!(
            std::fs::read(
                paths
                    .fromsoft_legacy_modengine_dir("darksoulsremastered")
                    .join("dinput8.dll")
            )
            .unwrap(),
            b"new-dll"
        );
    }

    #[test]
    fn writes_launch_helpers() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_dir: tmp.path().join("cfg"),
            data_dir: tmp.path().join("data"),
            cache_dir: tmp.path().join("cache"),
        };
        std::fs::create_dir_all(paths.me3_install_dir()).unwrap();
        std::fs::write(paths.me3_install_dir().join("me3"), b"x").unwrap();
        let me2 = paths.modengine2_install_dir();
        std::fs::create_dir_all(&me2).unwrap();
        std::fs::write(me2.join("modengine2_launcher.exe"), b"x").unwrap();

        let game = tmp.path().join("Game");
        std::fs::create_dir_all(&game).unwrap();
        write_modern_launch_helpers(&paths, "eldenring", &game).unwrap();
        let sh = std::fs::read_to_string(game.join("launch_emperor_me3.sh")).unwrap();
        assert!(sh.contains("launch --profile"));
        assert!(game.join("launch_emperor_me3.bat").is_file());
        let me2_bat = std::fs::read_to_string(game.join("launch_emperor_me2.bat")).unwrap();
        assert!(me2_bat.contains("-t er"));
        assert!(me2_bat.contains("config_emperor.toml"));
    }
}
