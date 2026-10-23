//! Download a missing SMAPI loader before deploy.
//!
//! The official installer zip carries platform bundles in `install.dat` (a renamed
//! zip). This copies that bundle into the game folder. It does not run the
//! installer executable, which would pick a bundle from the host OS and install
//! the native Linux build into a Proton game.

use std::fs;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use walkdir::WalkDir;

use super::stardewvalley::{looks_like_smapi_installer, smapi_present};
use crate::config::Paths;
use crate::github_release::{self, GhAsset, GhRelease};
use crate::tools::install as tool_install;

const SMAPI_REPO: &str = "Pathoschild/SMAPI";
#[cfg(target_os = "linux")]
const STARDEW_STEAM_APP_ID: &str = "413150";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SmapiPlatform {
    Linux,
    Windows,
}

impl SmapiPlatform {
    fn cache_name(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Windows => "windows",
        }
    }

    fn install_dat_dirs(self) -> &'static [&'static str] {
        match self {
            Self::Linux => &["linux", "unix"],
            Self::Windows => &["windows"],
        }
    }
}

/// `Stardew Valley.exe` is the Windows game (native or Proton). A Linux install
/// has the `StardewValley` launcher and no `.exe`.
fn smapi_platform(install_path: &Path) -> SmapiPlatform {
    if install_path.join("Stardew Valley.exe").is_file() {
        SmapiPlatform::Windows
    } else {
        SmapiPlatform::Linux
    }
}

/// Skip the download when SMAPI is already installed, nothing enabled needs it,
/// or an enabled mod is itself the SMAPI pack that deploy will install.
pub fn should_auto_install_smapi(
    install_path: &Path,
    content_roots: &[&Path],
    allows_empty: impl Fn(&Path) -> bool,
) -> bool {
    if smapi_present(install_path) {
        return false;
    }
    let mut needs = false;
    for root in content_roots {
        if looks_like_smapi_installer(root) {
            return false;
        }
        if !allows_empty(root) {
            needs = true;
        }
    }
    needs
}

/// Install the latest SMAPI. A download failure is a warning.
pub async fn ensure_smapi(paths: &Paths, install_path: &Path) -> Vec<String> {
    match install_smapi(paths, install_path).await {
        Ok(messages) => messages,
        Err(e) => vec![format!("Could not auto-install SMAPI: {e:#}")],
    }
}

async fn install_smapi(paths: &Paths, install_path: &Path) -> Result<Vec<String>> {
    let platform = smapi_platform(install_path);
    let release = github_release::fetch_latest_release(SMAPI_REPO)
        .await?
        .ok_or_else(|| anyhow!("SMAPI has no GitHub release"))?;
    let bundle = cached_bundle(paths, &release, platform).await?;
    let mut messages = apply_bundle(&bundle, install_path, platform)?;
    if platform == SmapiPlatform::Windows {
        messages.extend(ensure_windows_launch(install_path));
    }
    Ok(messages)
}

async fn cached_bundle(
    paths: &Paths,
    release: &GhRelease,
    platform: SmapiPlatform,
) -> Result<PathBuf> {
    let cache = paths.smapi_cache_dir(&release.tag_name);
    let bundle = cache.join(platform.cache_name());
    if bundle_ready(&bundle, platform) {
        return Ok(bundle);
    }
    let outer = cache.join("installer");
    if find_install_dat(&outer, platform).is_none() {
        let asset = pick_smapi_installer(&release.assets)
            .ok_or_else(|| anyhow!("release {} has no SMAPI installer zip", release.tag_name))?;
        let bytes = github_release::download_bytes(&asset.browser_download_url).await?;
        tool_install::extract_archive(&bytes, &asset.name, &outer)?;
    }
    let dat = find_install_dat(&outer, platform).ok_or_else(|| {
        anyhow!(
            "SMAPI installer has no install.dat for {}",
            platform.cache_name()
        )
    })?;
    let dat_bytes = fs::read(&dat).with_context(|| format!("reading {}", dat.display()))?;
    tool_install::extract_archive(&dat_bytes, "install.zip", &bundle)?;
    if !bundle_ready(&bundle, platform) {
        anyhow::bail!(
            "downloaded SMAPI archive did not contain a {} loader",
            platform.cache_name()
        );
    }
    Ok(bundle)
}

fn pick_smapi_installer(assets: &[GhAsset]) -> Option<&GhAsset> {
    assets.iter().find(|asset| {
        let name = asset.name.to_ascii_lowercase();
        name.starts_with("smapi-")
            && name.ends_with("-installer.zip")
            && !name.contains("developer")
    })
}

fn find_install_dat(root: &Path, platform: SmapiPlatform) -> Option<PathBuf> {
    if !root.is_dir() {
        return None;
    }
    let mut found = Vec::new();
    for entry in WalkDir::new(root)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if !entry.file_name().eq_ignore_ascii_case("install.dat") {
            continue;
        }
        let Some(parent) = entry.path().parent().and_then(|path| path.file_name()) else {
            continue;
        };
        found.push((parent.to_os_string(), entry.path().to_path_buf()));
    }
    for dir in platform.install_dat_dirs() {
        if let Some((_, path)) = found
            .iter()
            .find(|(parent, _)| parent.eq_ignore_ascii_case(dir))
        {
            return Some(path.clone());
        }
    }
    None
}

fn bundle_ready(bundle: &Path, platform: SmapiPlatform) -> bool {
    match platform {
        SmapiPlatform::Linux => {
            bundle.join("StardewModdingAPI").is_file() && bundle.join("unix-launcher.sh").is_file()
        }
        SmapiPlatform::Windows => bundle.join("StardewModdingAPI.exe").is_file(),
    }
}

fn apply_bundle(
    bundle: &Path,
    install_path: &Path,
    platform: SmapiPlatform,
) -> Result<Vec<String>> {
    copy_bundle(bundle, install_path)?;
    let mut messages = vec!["Installed SMAPI.".to_string()];
    if !copy_deps_json(install_path)? {
        messages.push(
            "Stardew Valley.deps.json was not found, so StardewModdingAPI.deps.json was not created. SMAPI may fail to resolve native libraries."
                .into(),
        );
    }
    if platform == SmapiPlatform::Linux {
        swap_unix_launcher(install_path)?;
        messages.push("Replaced the StardewValley launcher with SMAPI.".into());
    }
    Ok(messages)
}

fn copy_bundle(bundle: &Path, install_path: &Path) -> Result<()> {
    let mut copied = 0usize;
    for entry in WalkDir::new(bundle)
        .into_iter()
        .filter_map(|entry| entry.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(bundle).with_context(|| {
            format!(
                "bundle path {} is outside {}",
                entry.path().display(),
                bundle.display()
            )
        })?;
        if !should_copy_bundle_file(rel) {
            continue;
        }
        let dest = install_path.join(rel);
        if is_smapi_config(rel) && dest.is_file() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(entry.path(), &dest)
            .with_context(|| format!("copy {} to {}", entry.path().display(), dest.display()))?;
        copied += 1;
    }
    if copied == 0 {
        anyhow::bail!(
            "SMAPI bundle at {} had no loader files to copy",
            bundle.display()
        );
    }
    Ok(())
}

fn should_copy_bundle_file(rel: &Path) -> bool {
    let mut comps = rel.components();
    let Some(Component::Normal(first)) = comps.next() else {
        return false;
    };
    if !first.to_string_lossy().eq_ignore_ascii_case("Mods") {
        return true;
    }
    let Some(Component::Normal(mod_name)) = comps.next() else {
        return false;
    };
    let name = mod_name.to_string_lossy();
    name.eq_ignore_ascii_case("ConsoleCommands") || name.eq_ignore_ascii_case("SaveBackup")
}

fn is_smapi_config(rel: &Path) -> bool {
    let mut comps = rel.components();
    match (comps.next(), comps.next(), comps.next()) {
        (Some(Component::Normal(dir)), Some(Component::Normal(file)), None) => {
            dir.to_string_lossy().eq_ignore_ascii_case("smapi-internal")
                && file.to_string_lossy().eq_ignore_ascii_case("config.json")
        }
        _ => false,
    }
}

fn copy_deps_json(install_path: &Path) -> Result<bool> {
    let src = install_path.join("Stardew Valley.deps.json");
    if !src.is_file() {
        return Ok(false);
    }
    let dest = install_path.join("StardewModdingAPI.deps.json");
    fs::copy(&src, &dest)
        .with_context(|| format!("copy {} to {}", src.display(), dest.display()))?;
    Ok(true)
}

fn swap_unix_launcher(install_path: &Path) -> Result<()> {
    let launcher = install_path.join("StardewValley");
    let backup = install_path.join("StardewValley-original");
    let script = install_path.join("unix-launcher.sh");
    if !script.is_file() {
        anyhow::bail!("SMAPI bundle did not contain unix-launcher.sh");
    }

    // A previous attempt may already have renamed the vanilla launcher. Put it
    // back before backing it up again, so the SMAPI script is not saved as the
    // original game launcher.
    if backup.is_file() {
        if launcher.exists() {
            fs::remove_file(&launcher)
                .with_context(|| format!("removing {}", launcher.display()))?;
        }
        fs::rename(&backup, &launcher)
            .with_context(|| format!("restoring {} to {}", backup.display(), launcher.display()))?;
    }

    if launcher.exists() {
        fs::rename(&launcher, &backup).with_context(|| {
            format!("backing up {} to {}", launcher.display(), backup.display())
        })?;
    }
    fs::rename(&script, &launcher)
        .with_context(|| format!("installing SMAPI launcher at {}", launcher.display()))?;
    make_executable(&launcher)?;
    let binary = install_path.join("StardewModdingAPI");
    if binary.is_file() {
        make_executable(&binary)?;
    }
    Ok(())
}

fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(path)
            .with_context(|| format!("reading permissions for {}", path.display()))?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(path, perms)
            .with_context(|| format!("making {} executable", path.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// Place `StardewModdingAPI.exe` immediately before the only `%command%`.
pub fn smapi_launch_options(existing: &str, exe: &Path) -> String {
    let trimmed = existing.trim();
    if trimmed
        .to_ascii_lowercase()
        .contains("stardewmoddingapi.exe")
    {
        return trimmed.to_string();
    }
    let quoted = format!("\"{}\"", exe.display());
    let prefix = trimmed.strip_suffix("%command%").unwrap_or(trimmed).trim();
    if prefix.is_empty() {
        format!("{quoted} %command%")
    } else {
        format!("{prefix} {quoted} %command%")
    }
}

fn ensure_windows_launch(install_path: &Path) -> Vec<String> {
    #[cfg(not(target_os = "linux"))]
    {
        let exe = install_path.join("StardewModdingAPI.exe");
        let manual = smapi_launch_options("", &exe);
        vec![format!(
            "Set Steam launch options to {manual} so the game starts with SMAPI."
        )]
    }
    #[cfg(target_os = "linux")]
    {
        use crate::tools::steam_launch::{read_launch_options, write_launch_options};

        let exe = install_path.join("StardewModdingAPI.exe");
        let app_id =
            crate::tools::executables::steam_app_id_from_install(&install_path.to_string_lossy())
                .unwrap_or_else(|| STARDEW_STEAM_APP_ID.to_string());
        let existing = match read_launch_options(&app_id) {
            Ok(value) => value.unwrap_or_default(),
            Err(e) => {
                let manual = smapi_launch_options("", &exe);
                return vec![format!(
                    "Could not read Steam launch options ({e:#}). Set {manual} manually."
                )];
            }
        };
        if existing
            .to_ascii_lowercase()
            .contains("stardewmoddingapi.exe")
        {
            return Vec::new();
        }
        let updated = smapi_launch_options(&existing, &exe);
        match write_launch_options(&app_id, &updated) {
            Ok(()) => vec![format!(
                "Updated Steam launch options to {updated} so the game starts with SMAPI."
            )],
            Err(e) => vec![format!(
                "Steam launch options need {updated}. Could not write them automatically ({e:#})."
            )],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::GamePlugin;

    fn write_file(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    fn linux_bundle(root: &Path) {
        write_file(&root.join("StardewModdingAPI"), "smapi-bin");
        write_file(&root.join("StardewModdingAPI.dll"), "dll");
        write_file(
            &root.join("unix-launcher.sh"),
            "#!/bin/sh\nexec ./StardewModdingAPI\n",
        );
        write_file(&root.join("smapi-internal/config.json"), "bundle-config");
        write_file(&root.join("smapi-internal/metadata.json"), "meta");
        write_file(&root.join("Mods/ConsoleCommands/manifest.json"), "{}");
        write_file(&root.join("Mods/SaveBackup/manifest.json"), "{}");
        write_file(&root.join("Mods/ShouldNotCopy/manifest.json"), "{}");
    }

    #[test]
    fn skips_when_smapi_is_present_or_nothing_needs_it() {
        let install = tempfile::tempdir().unwrap();
        let mod_dir = tempfile::tempdir().unwrap();
        write_file(&mod_dir.path().join("manifest.json"), "{}");
        let roots = [mod_dir.path()];
        assert!(should_auto_install_smapi(install.path(), &roots, |_| false));

        write_file(&install.path().join("StardewModdingAPI.dll"), "x");
        assert!(!should_auto_install_smapi(install.path(), &roots, |_| {
            false
        }));

        let fresh = tempfile::tempdir().unwrap();
        assert!(!should_auto_install_smapi(fresh.path(), &[], |_| false));

        let installer = tempfile::tempdir().unwrap();
        write_file(&installer.path().join("StardewModdingAPI.exe"), "x");
        assert!(!should_auto_install_smapi(
            fresh.path(),
            &[installer.path()],
            |_| false
        ));
    }

    #[test]
    fn linux_install_restores_original_launcher_and_copies_deps() {
        let install = tempfile::tempdir().unwrap();
        write_file(&install.path().join("StardewValley"), "already-smapi");
        write_file(
            &install.path().join("StardewValley-original"),
            "vanilla-launcher",
        );
        write_file(
            &install.path().join("Stardew Valley.deps.json"),
            "{\"name\":\"game\"}",
        );
        write_file(
            &install.path().join("smapi-internal/config.json"),
            "keep-me",
        );

        let bundle = tempfile::tempdir().unwrap();
        linux_bundle(bundle.path());
        let messages = apply_bundle(bundle.path(), install.path(), SmapiPlatform::Linux).unwrap();
        assert!(messages.iter().any(|message| message == "Installed SMAPI."));

        assert_eq!(
            fs::read_to_string(install.path().join("StardewValley")).unwrap(),
            "#!/bin/sh\nexec ./StardewModdingAPI\n"
        );
        assert_eq!(
            fs::read_to_string(install.path().join("StardewValley-original")).unwrap(),
            "vanilla-launcher"
        );
        assert!(!install.path().join("unix-launcher.sh").exists());
        assert_eq!(
            fs::read_to_string(install.path().join("StardewModdingAPI.deps.json")).unwrap(),
            "{\"name\":\"game\"}"
        );
        assert_eq!(
            fs::read_to_string(install.path().join("smapi-internal/config.json")).unwrap(),
            "keep-me"
        );
        assert!(install
            .path()
            .join("Mods/ConsoleCommands/manifest.json")
            .is_file());
        assert!(install
            .path()
            .join("Mods/SaveBackup/manifest.json")
            .is_file());
        assert!(!install.path().join("Mods/ShouldNotCopy").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let launcher = fs::metadata(install.path().join("StardewValley"))
                .unwrap()
                .permissions()
                .mode();
            let binary = fs::metadata(install.path().join("StardewModdingAPI"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(launcher & 0o755, 0o755);
            assert_eq!(binary & 0o755, 0o755);
        }
    }

    #[test]
    fn windows_install_copies_the_exe_and_leaves_the_unix_launcher() {
        let install = tempfile::tempdir().unwrap();
        write_file(&install.path().join("Stardew Valley.exe"), "game");
        write_file(&install.path().join("StardewValley"), "native-launcher");
        write_file(&install.path().join("Stardew Valley.deps.json"), "{}");

        let bundle = tempfile::tempdir().unwrap();
        write_file(&bundle.path().join("StardewModdingAPI.exe"), "smapi-exe");
        write_file(&bundle.path().join("StardewModdingAPI.dll"), "dll");
        write_file(
            &bundle.path().join("Mods/ConsoleCommands/manifest.json"),
            "{}",
        );

        apply_bundle(bundle.path(), install.path(), SmapiPlatform::Windows).unwrap();

        assert_eq!(
            fs::read_to_string(install.path().join("StardewModdingAPI.exe")).unwrap(),
            "smapi-exe"
        );
        assert_eq!(
            fs::read_to_string(install.path().join("StardewValley")).unwrap(),
            "native-launcher"
        );
        assert!(!install.path().join("StardewValley-original").exists());
        assert!(install.path().join("StardewModdingAPI.deps.json").is_file());
        assert_eq!(smapi_platform(install.path()), SmapiPlatform::Windows);
    }

    #[test]
    fn launch_options_keep_one_command_and_an_existing_prefix() {
        let exe = Path::new("/games/Stardew Valley/StardewModdingAPI.exe");
        assert_eq!(
            smapi_launch_options("", exe),
            "\"/games/Stardew Valley/StardewModdingAPI.exe\" %command%"
        );
        let existing = "EMM_TOOLS_BEGIN ENABLE_AUTOHDR=1 EMM_TOOLS_END %command%";
        assert_eq!(
            smapi_launch_options(existing, exe),
            "EMM_TOOLS_BEGIN ENABLE_AUTOHDR=1 EMM_TOOLS_END \"/games/Stardew Valley/StardewModdingAPI.exe\" %command%"
        );
        let already = smapi_launch_options(existing, exe);
        assert_eq!(smapi_launch_options(&already, exe), already);
        assert_eq!(already.matches("%command%").count(), 1);
    }

    #[test]
    fn prefers_linux_install_dat_over_the_older_unix_folder() {
        let root = tempfile::tempdir().unwrap();
        write_file(&root.path().join("internal/unix/install.dat"), "old");
        write_file(&root.path().join("internal/linux/install.dat"), "new");
        write_file(&root.path().join("internal/windows/install.dat"), "win");
        assert_eq!(
            fs::read_to_string(find_install_dat(root.path(), SmapiPlatform::Linux).unwrap())
                .unwrap(),
            "new"
        );
        assert_eq!(
            fs::read_to_string(find_install_dat(root.path(), SmapiPlatform::Windows).unwrap())
                .unwrap(),
            "win"
        );
    }

    #[test]
    fn picks_the_installer_zip_and_not_a_developer_pack() {
        let assets = vec![
            GhAsset {
                name: "SMAPI-4.3.2-for-developers.zip".into(),
                browser_download_url: "https://example.invalid/dev".into(),
            },
            GhAsset {
                name: "SMAPI-4.3.2-installer.zip".into(),
                browser_download_url: "https://example.invalid/installer".into(),
            },
        ];
        let picked = pick_smapi_installer(&assets).unwrap();
        assert_eq!(picked.name, "SMAPI-4.3.2-installer.zip");
    }

    #[tokio::test]
    async fn prepare_runtime_skips_download_when_smapi_is_present() {
        let tmp = tempfile::tempdir().unwrap();
        let install = tmp.path().join("game");
        fs::create_dir_all(install.join("smapi-internal")).unwrap();
        let staged = tempfile::tempdir().unwrap();
        write_file(&staged.path().join("manifest.json"), "{}");
        let roots = [staged.path()];
        let paths = Paths {
            config_dir: tmp.path().join("config"),
            data_dir: tmp.path().join("data"),
            cache_dir: tmp.path().join("cache"),
        };
        let ctx = crate::games::RuntimePrep {
            paths: &paths,
            install_path: &install,
            nexus: None,
            premium: false,
            content_roots: &roots,
        };
        let outcome = crate::games::StardewValleyPlugin
            .prepare_runtime(&ctx)
            .await
            .unwrap();
        match outcome {
            crate::games::RuntimePrepOutcome::Done(warnings) => {
                assert!(warnings.is_empty(), "{warnings:?}");
            }
            crate::games::RuntimePrepOutcome::NeedsAssist { .. } => {
                panic!("SMAPI install does not use Download Assist");
            }
        }
    }
}
