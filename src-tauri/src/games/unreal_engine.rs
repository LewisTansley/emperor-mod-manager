//! Shared deploy logic for Unreal Engine 4 / 5 titles.
//!
//! Typical layout:
//! ```text
//! <Install>/<Project>/Content/Paks/~mods/
//! <Install>/<Project>/Content/Paks/LogicMods/
//! <Install>/<Project>/Binaries/Win64|WinGDK/   # UE4SS, injectors
//! ```

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::{normalize_relative, GamePlugin, GamePluginInfo};

/// Roots that must not be peeled as archive wrappers for UE mods.
pub const UE_PRESERVE_ROOTS: &[&str] = &[
    "Binaries",
    "Content",
    "Engine",
    "LogicMods",
    "Paks",
    "ue4ss",
    "~mods",
];

/// Vortex-style markers that mean pak files in this pack are LogicMods.
const LOGICMOD_MARKERS: &[&str] = &[
    ".logicmod",
    ".ue4sslogicmod",
    "ue4sslogicmod.info",
];

/// Injector / companion DLLs that deploy next to the game binary.
const ROOT_INJECTOR_DLLS: &[&str] = &[
    "dwmapi.dll",
    "xinput1_3.dll",
    "xinput1_4.dll",
    "version.dll",
];

/// Optional overrides passed during deploy (generic UE + LogicMod markers).
#[derive(Debug, Clone, Default)]
pub struct DeployContext<'a> {
    /// Override auto-detected project folder name.
    pub project_name: Option<&'a str>,
    /// Staging content root (for LogicMod marker detection).
    pub content_root: Option<&'a Path>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UeLayout {
    pub project_name: String,
    pub binaries_platform: String,
}

impl UeLayout {
    pub fn project_dir(&self, install_path: &Path) -> PathBuf {
        install_path.join(&self.project_name)
    }

    pub fn paks_dir(&self, install_path: &Path) -> PathBuf {
        self.project_dir(install_path)
            .join("Content")
            .join("Paks")
    }

    pub fn mods_dir(&self, install_path: &Path) -> PathBuf {
        self.paks_dir(install_path).join("~mods")
    }

    pub fn logic_mods_dir(&self, install_path: &Path) -> PathBuf {
        self.paks_dir(install_path).join("LogicMods")
    }

    pub fn binaries_dir(&self, install_path: &Path) -> PathBuf {
        self.project_dir(install_path)
            .join("Binaries")
            .join(&self.binaries_platform)
    }
}

/// Detect an Unreal project folder under `install_path`.
pub fn detect_ue_layout(install_path: &Path) -> Option<UeLayout> {
    detect_ue_layout_with_preferred(install_path, None)
}

/// Prefer `preferred` when that project folder looks valid; otherwise scan.
pub fn detect_ue_layout_with_preferred(
    install_path: &Path,
    preferred: Option<&str>,
) -> Option<UeLayout> {
    if !install_path.is_dir() {
        return None;
    }

    if let Some(name) = preferred {
        if looks_like_ue_project(&install_path.join(name)) {
            return Some(layout_for_project(install_path, name));
        }
    }

    let Ok(entries) = std::fs::read_dir(install_path) else {
        return None;
    };

    let mut candidates: Vec<(String, PathBuf, i32)> = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.eq_ignore_ascii_case("Engine")
            || name_str.eq_ignore_ascii_case("Binaries")
            || name_str.starts_with('.')
        {
            continue;
        }
        let dir = entry.path();
        if !looks_like_ue_project(&dir) {
            continue;
        }
        let score = score_ue_project(&dir);
        candidates.push((name_str.into_owned(), dir, score));
    }

    candidates.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    let (name, dir, _) = candidates.into_iter().next()?;
    Some(layout_for_project_at(install_path, &name, &dir))
}

fn looks_like_ue_project(dir: &Path) -> bool {
    if dir.join("Content").join("Paks").is_dir() {
        return true;
    }
    if dir.join("Binaries").join("Win64").is_dir() || dir.join("Binaries").join("WinGDK").is_dir()
    {
        return true;
    }
    // Loose .uproject next to Content/
    if dir.join("Content").is_dir() {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                if name.to_string_lossy().to_lowercase().ends_with(".uproject") {
                    return true;
                }
            }
        }
    }
    false
}

fn score_ue_project(dir: &Path) -> i32 {
    let mut score = 0;
    if dir.join("Content").join("Paks").is_dir() {
        score += 10;
    }
    if dir.join("Binaries").join("Win64").is_dir() {
        score += 5;
    }
    if dir.join("Binaries").join("WinGDK").is_dir() {
        score += 4;
    }
    if dir.join("Content").is_dir() {
        score += 2;
    }
    score
}

fn layout_for_project(install_path: &Path, project_name: &str) -> UeLayout {
    layout_for_project_at(install_path, project_name, &install_path.join(project_name))
}

fn layout_for_project_at(install_path: &Path, project_name: &str, project_dir: &Path) -> UeLayout {
    let _ = install_path;
    let binaries_platform = detect_binaries_platform(project_dir);
    UeLayout {
        project_name: project_name.to_string(),
        binaries_platform,
    }
}

fn detect_binaries_platform(project_dir: &Path) -> String {
    let win64 = project_dir.join("Binaries").join("Win64");
    let wingdk = project_dir.join("Binaries").join("WinGDK");
    if win64.is_dir() {
        "Win64".into()
    } else if wingdk.is_dir() {
        "WinGDK".into()
    } else {
        "Win64".into()
    }
}

/// Resolve layout: preferred override, else detect, else preferred-as-literal (for tests / offline).
pub fn resolve_layout(
    install_path: &Path,
    preferred_project: Option<&str>,
) -> Result<UeLayout> {
    if let Some(layout) = detect_ue_layout_with_preferred(install_path, preferred_project) {
        return Ok(layout);
    }
    if let Some(name) = preferred_project {
        return Ok(UeLayout {
            project_name: name.to_string(),
            binaries_platform: "Win64".into(),
        });
    }
    bail!(
        "Could not detect an Unreal Engine project under {}. Set a project folder name in game settings.",
        install_path.display()
    )
}

pub fn staging_has_logicmod_marker(content_root: &Path) -> bool {
    if !content_root.is_dir() {
        return false;
    }
    // Shallow: markers at content root or one level down (common archive layouts).
    if dir_has_logicmod_marker(content_root) {
        return true;
    }
    let Ok(entries) = std::fs::read_dir(content_root) else {
        return false;
    };
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            if dir_has_logicmod_marker(&entry.path()) {
                return true;
            }
        }
    }
    false
}

fn dir_has_logicmod_marker(dir: &Path) -> bool {
    for marker in LOGICMOD_MARKERS {
        if dir.join(marker).is_file() {
            return true;
        }
    }
    false
}

fn is_pak_like(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".pak") || lower.ends_with(".ucas") || lower.ends_with(".utoc")
}

fn is_logicmod_marker_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    LOGICMOD_MARKERS
        .iter()
        .any(|m| lower == m.to_lowercase())
}

fn is_injector_dll(name: &str) -> bool {
    let lower = name.to_lowercase();
    ROOT_INJECTOR_DLLS.iter().any(|d| *d == lower.as_str())
}

/// Shared UE path resolution.
pub fn resolve_ue_deploy(
    install_path: &Path,
    relative: &Path,
    preferred_project: Option<&str>,
    ctx: &DeployContext<'_>,
) -> Result<PathBuf> {
    let project = ctx.project_name.or(preferred_project);
    let layout = resolve_layout(install_path, project)?;
    let force_logic = ctx
        .content_root
        .map(staging_has_logicmod_marker)
        .unwrap_or(false);

    let normalized = normalize_relative(relative);
    let components = path_components_lower(&normalized);
    if components.is_empty() {
        return Ok(install_path.to_path_buf());
    }

    let originals = path_components_original(&normalized);
    let first = components[0].as_str();
    let project_lower = layout.project_name.to_lowercase();

    // Skip deploying marker files into the game tree.
    if components.len() == 1 && is_logicmod_marker_file(originals[0].as_str()) {
        return Ok(layout.logic_mods_dir(install_path).join(&originals[0]));
    }

    // Already install-relative under <Project>/...
    if first == project_lower {
        return Ok(join_with_originals(install_path, &originals));
    }

    // LogicMods (blueprint paks) — nest under Paks/LogicMods.
    if let Some(idx) = components.iter().position(|c| c == "logicmods") {
        let rest: PathBuf = originals.iter().skip(idx + 1).collect();
        let mut out = layout.logic_mods_dir(install_path);
        if !rest.as_os_str().is_empty() {
            out.push(rest);
        }
        return Ok(out);
    }

    // Flat UE pak / IoStore files.
    let is_single = components.len() == 1;
    let leaf = components.last().map(|s| s.as_str()).unwrap_or("");
    if is_single && is_pak_like(leaf) {
        let dest_dir = if force_logic {
            layout.logic_mods_dir(install_path)
        } else {
            layout.mods_dir(install_path)
        };
        return Ok(dest_dir.join(originals[0].as_str()));
    }

    // Paths already under Content/Paks/~mods or ~mods/...
    if components.iter().any(|c| c == "~mods") {
        if let Some(idx) = components.iter().position(|c| c == "~mods") {
            let rest: PathBuf = originals.iter().skip(idx + 1).collect();
            let mut out = layout.mods_dir(install_path);
            if !rest.as_os_str().is_empty() {
                out.push(rest);
            }
            return Ok(out);
        }
    }

    // UE4SS / binaries: ue4ss/, injectors, or Binaries/...
    let platform_lower = layout.binaries_platform.to_lowercase();
    if first == "ue4ss"
        || is_injector_dll(leaf)
        || first == "binaries"
        || components.iter().any(|c| c == "binaries")
    {
        return Ok(binaries_target(
            &layout,
            install_path,
            &components,
            &originals,
            &platform_lower,
        ));
    }

    // Default: majority of Nexus pak mods land in ~mods (or LogicMods with marker).
    if force_logic && is_pak_like(leaf) {
        return Ok(layout.logic_mods_dir(install_path).join(&normalized));
    }
    Ok(layout.mods_dir(install_path).join(&normalized))
}

fn binaries_target(
    layout: &UeLayout,
    install_path: &Path,
    components: &[String],
    originals: &[String],
    platform_lower: &str,
) -> PathBuf {
    let binaries = layout.binaries_dir(install_path);
    let project_lower = layout.project_name.to_lowercase();

    let mut start = 0;
    if components.first().map(|s| s.as_str()) == Some(project_lower.as_str()) {
        start = 1;
    }
    if components.get(start).map(|s| s.as_str()) == Some("binaries") {
        start += 1;
        if components.get(start).map(|s| s.as_str()) == Some(platform_lower)
            || components.get(start).map(|s| s.as_str()) == Some("win64")
            || components.get(start).map(|s| s.as_str()) == Some("wingdk")
        {
            start += 1;
        }
    }

    let rest: PathBuf = originals.iter().skip(start).collect();
    if rest.as_os_str().is_empty() {
        binaries
    } else {
        binaries.join(rest)
    }
}

pub fn ue_preflight_warnings(install_path: &Path, preferred_project: Option<&str>) -> Vec<String> {
    let mut warnings = Vec::new();
    let Ok(layout) = resolve_layout(install_path, preferred_project) else {
        warnings.push(
            "Could not detect Unreal project folder (expected <Project>/Content/Paks). Set project name in game settings.".into(),
        );
        return warnings;
    };

    let binaries = layout.binaries_dir(install_path);
    let ue4ss_dir = binaries.join("ue4ss");
    let has_ue4ss = ue4ss_dir.is_dir()
        || binaries.join("UE4SS.dll").is_file()
        || binaries.join("dwmapi.dll").is_file()
        || binaries.join("xinput1_3.dll").is_file()
        || binaries.join("xinput1_4.dll").is_file();

    if !has_ue4ss {
        warnings.push(format!(
            "UE4SS not detected under {}/Binaries/{}. LogicMods and script mods need UE4SS (install separately).",
            layout.project_name, layout.binaries_platform
        ));
    }

    let paks = layout.paks_dir(install_path);
    if install_path.is_dir() && !paks.exists() {
        // Only warn when install looks real (exists) but paks path missing.
        if layout.project_dir(install_path).is_dir() {
            warnings.push(format!(
                "Paks folder not found at {} — asset mods may not load until the game creates it.",
                paks.display()
            ));
        }
    }

    warnings
}

fn path_components_lower(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect()
}

fn path_components_original(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn join_with_originals(install_path: &Path, originals: &[String]) -> PathBuf {
    let mut out = install_path.to_path_buf();
    for part in originals {
        out.push(part);
    }
    out
}

macro_rules! ue_title_plugin {
    ($struct:ident, $id:expr, $display:expr, $domain:expr, $matches:expr, $project:expr, $preserve:expr) => {
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
                $preserve
            }

            fn resolve_deploy_root(
                &self,
                install_path: &Path,
                relative: &Path,
            ) -> Result<PathBuf> {
                resolve_ue_deploy(
                    install_path,
                    relative,
                    Some($project),
                    &DeployContext::default(),
                )
            }

            fn resolve_deploy_root_ctx(
                &self,
                install_path: &Path,
                relative: &Path,
                ctx: &DeployContext<'_>,
            ) -> Result<PathBuf> {
                resolve_ue_deploy(install_path, relative, Some($project), ctx)
            }

            fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
                ue_preflight_warnings(install_path, Some($project))
            }

            fn preflight_warnings_ctx(
                &self,
                install_path: &Path,
                ctx: &DeployContext<'_>,
            ) -> Vec<String> {
                let preferred = ctx.project_name.or(Some($project));
                ue_preflight_warnings(install_path, preferred)
            }
        }
    };
}

// --- Tier A title plugins ---

ue_title_plugin!(
    Stalker2HeartOfChornobylPlugin,
    "stalker2heartofchornobyl",
    "S.T.A.L.K.E.R. 2: Heart of Chornobyl",
    "stalker2heartofchornobyl",
    &[
        "s.t.a.l.k.e.r. 2",
        "stalker 2",
        "stalker2",
        "heart of chornobyl",
        "heart of chernobyl",
    ],
    "Stalker2",
    &[
        "Stalker2",
        "Binaries",
        "Content",
        "Engine",
        "LogicMods",
        "Paks",
        "ue4ss",
        "~mods",
    ]
);

ue_title_plugin!(
    PalworldPlugin,
    "palworld",
    "Palworld",
    "palworld",
    &["palworld"],
    "Pal",
    &[
        "Pal",
        "Binaries",
        "Content",
        "Engine",
        "LogicMods",
        "Paks",
        "ue4ss",
        "~mods",
    ]
);

ue_title_plugin!(
    HogwartsLegacyPlugin,
    "hogwartslegacy",
    "Hogwarts Legacy",
    "hogwartslegacy",
    &["hogwarts legacy", "hogwartslegacy"],
    "Phoenix",
    &[
        "Phoenix",
        "Binaries",
        "Content",
        "Engine",
        "LogicMods",
        "Paks",
        "ue4ss",
        "~mods",
    ]
);

ue_title_plugin!(
    ReadyOrNotPlugin,
    "readyornot",
    "Ready or Not",
    "readyornot",
    &["ready or not", "readyornot"],
    "ReadyOrNot",
    &[
        "ReadyOrNot",
        "Binaries",
        "Content",
        "Engine",
        "LogicMods",
        "Paks",
        "ue4ss",
        "~mods",
    ]
);

ue_title_plugin!(
    Subnautica2Plugin,
    "subnautica2",
    "Subnautica 2",
    "subnautica2",
    &["subnautica 2", "subnautica2"],
    "Subnautica2",
    &[
        "Subnautica2",
        "Binaries",
        "Content",
        "Engine",
        "LogicMods",
        "Paks",
        "ue4ss",
        "~mods",
    ]
);

ue_title_plugin!(
    DeepRockGalacticPlugin,
    "deeprockgalactic",
    "Deep Rock Galactic",
    "deeprockgalactic",
    &["deep rock galactic", "deeprockgalactic"],
    "FSD",
    &[
        "FSD",
        "Binaries",
        "Content",
        "Engine",
        "LogicMods",
        "Paks",
        "ue4ss",
        "~mods",
    ]
);

const MARVEL_PRESERVE: &[&str] = &[
    "Marvel",
    "MarvelGame",
    "Binaries",
    "Content",
    "Engine",
    "LogicMods",
    "Paks",
    "ue4ss",
    "~mods",
];

const PAVLOV_PRESERVE: &[&str] = &[
    "Pavlov",
    "Binaries",
    "Content",
    "Engine",
    "LogicMods",
    "Paks",
    "ue4ss",
    "~mods",
];

/// Steam layout is often `MarvelRivals/MarvelGame/Marvel/...`.
pub fn marvel_ue_install_root(install_path: &Path) -> PathBuf {
    if looks_like_ue_project(&install_path.join("Marvel")) {
        return install_path.to_path_buf();
    }
    let nested = install_path.join("MarvelGame");
    if looks_like_ue_project(&nested.join("Marvel")) {
        return nested;
    }
    install_path.to_path_buf()
}

pub struct MarvelRivalsPlugin;

impl GamePlugin for MarvelRivalsPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "marvelrivals",
            display_name: "Marvel Rivals",
            nexus_domain: "marvelrivals",
            match_names: &["marvel rivals", "marvelrivals"],
        }
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        MARVEL_PRESERVE
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        let root = marvel_ue_install_root(install_path);
        resolve_ue_deploy(&root, relative, Some("Marvel"), &DeployContext::default())
    }

    fn resolve_deploy_root_ctx(
        &self,
        install_path: &Path,
        relative: &Path,
        ctx: &DeployContext<'_>,
    ) -> Result<PathBuf> {
        let root = marvel_ue_install_root(install_path);
        resolve_ue_deploy(&root, relative, Some("Marvel"), ctx)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        let root = marvel_ue_install_root(install_path);
        let mut warnings = ue_preflight_warnings(&root, Some("Marvel"));
        warnings.push(
            "Marvel Rivals paks need a UTOC signature bypass (or equivalent) before ~mods will load."
                .into(),
        );
        warnings
    }

    fn preflight_warnings_ctx(
        &self,
        install_path: &Path,
        ctx: &DeployContext<'_>,
    ) -> Vec<String> {
        let root = marvel_ue_install_root(install_path);
        let preferred = ctx.project_name.or(Some("Marvel"));
        let mut warnings = ue_preflight_warnings(&root, preferred);
        warnings.push(
            "Marvel Rivals paks need a UTOC signature bypass (or equivalent) before ~mods will load."
                .into(),
        );
        warnings
    }
}

pub struct PavlovPlugin;

impl GamePlugin for PavlovPlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "pavlov",
            display_name: "Pavlov VR",
            nexus_domain: "pavlov",
            match_names: &["pavlov vr", "pavlov"],
        }
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        PAVLOV_PRESERVE
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        resolve_ue_deploy(
            install_path,
            relative,
            Some("Pavlov"),
            &DeployContext::default(),
        )
    }

    fn resolve_deploy_root_ctx(
        &self,
        install_path: &Path,
        relative: &Path,
        ctx: &DeployContext<'_>,
    ) -> Result<PathBuf> {
        resolve_ue_deploy(install_path, relative, Some("Pavlov"), ctx)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        let mut warnings = ue_preflight_warnings(install_path, Some("Pavlov"));
        warnings.push(
            "Official Pavlov VR map/mod discovery is the in-game mod.io browser. Linked paks go under Pavlov/Content/Paks/~mods — verify in-game that the content appeared."
                .into(),
        );
        warnings
    }

    fn preflight_warnings_ctx(
        &self,
        install_path: &Path,
        ctx: &DeployContext<'_>,
    ) -> Vec<String> {
        let preferred = ctx.project_name.or(Some("Pavlov"));
        let mut warnings = ue_preflight_warnings(install_path, preferred);
        warnings.push(
            "Official Pavlov VR map/mod discovery is the in-game mod.io browser. Linked paks go under Pavlov/Content/Paks/~mods — verify in-game that the content appeared."
                .into(),
        );
        warnings
    }
}

/// Generic UE plugin: any install with detected (or overridden) project folder.
pub struct UnrealEnginePlugin;

impl GamePlugin for UnrealEnginePlugin {
    fn info(&self) -> GamePluginInfo {
        GamePluginInfo {
            id: "unreal",
            display_name: "Unreal Engine (generic)",
            nexus_domain: "",
            match_names: &[],
        }
    }

    fn prefers_mod_folder(&self) -> bool {
        false
    }

    fn preserve_staging_root_names(&self) -> &[&str] {
        UE_PRESERVE_ROOTS
    }

    fn resolve_deploy_root(&self, install_path: &Path, relative: &Path) -> Result<PathBuf> {
        resolve_ue_deploy(install_path, relative, None, &DeployContext::default())
    }

    fn resolve_deploy_root_ctx(
        &self,
        install_path: &Path,
        relative: &Path,
        ctx: &DeployContext<'_>,
    ) -> Result<PathBuf> {
        resolve_ue_deploy(install_path, relative, None, ctx)
    }

    fn preflight_warnings(&self, install_path: &Path) -> Vec<String> {
        ue_preflight_warnings(install_path, None)
    }

    fn preflight_warnings_ctx(
        &self,
        install_path: &Path,
        ctx: &DeployContext<'_>,
    ) -> Vec<String> {
        ue_preflight_warnings(install_path, ctx.project_name)
    }
}

/// Serialize-friendly layout for the frontend.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UeLayoutInfo {
    pub project_name: String,
    pub binaries_platform: String,
    pub paks_dir: String,
    pub binaries_dir: String,
}

pub fn layout_info(install_path: &Path, preferred: Option<&str>) -> Result<UeLayoutInfo> {
    let layout = resolve_layout(install_path, preferred)
        .with_context(|| format!("detect UE layout at {}", install_path.display()))?;
    Ok(UeLayoutInfo {
        project_name: layout.project_name.clone(),
        binaries_platform: layout.binaries_platform.clone(),
        paks_dir: layout.paks_dir(install_path).to_string_lossy().into(),
        binaries_dir: layout.binaries_dir(install_path).to_string_lossy().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::GamePlugin;

    #[test]
    fn stalker2_preserves_relative_trees() {
        let install = Path::new("/game");
        let p = Stalker2HeartOfChornobylPlugin;
        assert_eq!(
            p.resolve_deploy_root(
                install,
                Path::new("Stalker2/Content/Paks/~mods/Foo.pak")
            )
            .unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/~mods/Foo.pak")
        );
    }

    #[test]
    fn flat_pak_files_go_to_tildemods() {
        let install = Path::new("/game");
        let p = Stalker2HeartOfChornobylPlugin;
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("Foo.pak")).unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/~mods/Foo.pak")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("Foo.ucas")).unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/~mods/Foo.ucas")
        );
    }

    #[test]
    fn logicmods_nested_under_paks() {
        let install = Path::new("/game");
        let p = Stalker2HeartOfChornobylPlugin;
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("LogicMods/MyBp.pak"))
                .unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/LogicMods/MyBp.pak")
        );
    }

    #[test]
    fn ue4ss_and_binaries_go_to_win64() {
        let install = Path::new("/game");
        let p = Stalker2HeartOfChornobylPlugin;
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("ue4ss/Mods/Foo/scripts/main.lua"))
                .unwrap(),
            PathBuf::from("/game/Stalker2/Binaries/Win64/ue4ss/Mods/Foo/scripts/main.lua")
        );
        assert_eq!(
            p.resolve_deploy_root(install, Path::new("dwmapi.dll"))
                .unwrap(),
            PathBuf::from("/game/Stalker2/Binaries/Win64/dwmapi.dll")
        );
    }

    #[test]
    fn logicmod_marker_routes_flat_pak() {
        let staging = tempfile::tempdir().unwrap();
        std::fs::write(staging.path().join(".logicmod"), b"").unwrap();
        let install = Path::new("/game");
        let ctx = DeployContext {
            project_name: None,
            content_root: Some(staging.path()),
        };
        assert_eq!(
            resolve_ue_deploy(install, Path::new("MyBp.pak"), Some("Stalker2"), &ctx).unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/LogicMods/MyBp.pak")
        );
    }

    #[test]
    fn detect_project_folder() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("Pal");
        std::fs::create_dir_all(project.join("Content").join("Paks")).unwrap();
        std::fs::create_dir_all(project.join("Binaries").join("Win64")).unwrap();
        let layout = detect_ue_layout(root.path()).unwrap();
        assert_eq!(layout.project_name, "Pal");
        assert_eq!(layout.binaries_platform, "Win64");
    }

    #[test]
    fn detect_wingdk_platform() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("Pal");
        std::fs::create_dir_all(project.join("Content").join("Paks")).unwrap();
        std::fs::create_dir_all(project.join("Binaries").join("WinGDK")).unwrap();
        let layout = detect_ue_layout(root.path()).unwrap();
        assert_eq!(layout.binaries_platform, "WinGDK");
    }

    #[test]
    fn project_override_in_context() {
        let install = Path::new("/game");
        let ctx = DeployContext {
            project_name: Some("CustomGame"),
            content_root: None,
        };
        assert_eq!(
            UnrealEnginePlugin
                .resolve_deploy_root_ctx(install, Path::new("Foo.pak"), &ctx)
                .unwrap(),
            PathBuf::from("/game/CustomGame/Content/Paks/~mods/Foo.pak")
        );
    }

    #[test]
    fn palworld_and_hogwarts_match() {
        assert_eq!(PalworldPlugin.info().id, "palworld");
        assert_eq!(HogwartsLegacyPlugin.info().nexus_domain, "hogwartslegacy");
    }

    #[test]
    fn ready_or_not_and_subnautica2_flat_pak() {
        let install = Path::new("/game");
        assert_eq!(
            ReadyOrNotPlugin
                .resolve_deploy_root(install, Path::new("Foo.pak"))
                .unwrap(),
            PathBuf::from("/game/ReadyOrNot/Content/Paks/~mods/Foo.pak")
        );
        assert_eq!(
            Subnautica2Plugin
                .resolve_deploy_root(install, Path::new("Foo.ucas"))
                .unwrap(),
            PathBuf::from("/game/Subnautica2/Content/Paks/~mods/Foo.ucas")
        );
    }

    #[test]
    fn deeprock_and_pavlov_flat_pak() {
        let install = Path::new("/game");
        assert_eq!(
            DeepRockGalacticPlugin
                .resolve_deploy_root(install, Path::new("mod_P.pak"))
                .unwrap(),
            PathBuf::from("/game/FSD/Content/Paks/~mods/mod_P.pak")
        );
        assert_eq!(
            PavlovPlugin
                .resolve_deploy_root(install, Path::new("Map.pak"))
                .unwrap(),
            PathBuf::from("/game/Pavlov/Content/Paks/~mods/Map.pak")
        );
    }

    #[test]
    fn marvel_rivals_nested_marvelgame_root() {
        let tmp = tempfile::tempdir().unwrap();
        let marvel = tmp.path().join("MarvelGame").join("Marvel");
        std::fs::create_dir_all(marvel.join("Content").join("Paks")).unwrap();
        let dest = MarvelRivalsPlugin
            .resolve_deploy_root(tmp.path(), Path::new("Skin.pak"))
            .unwrap();
        assert_eq!(
            dest,
            marvel.join("Content").join("Paks").join("~mods").join("Skin.pak")
        );
        let warns = MarvelRivalsPlugin.preflight_warnings(tmp.path());
        assert!(warns.iter().any(|w| w.contains("signature bypass")));
    }

    #[test]
    fn normalizes_windows_separators() {
        let install = Path::new("/game");
        let weird = PathBuf::from(r"LogicMods\Bp.pak");
        assert_eq!(
            Stalker2HeartOfChornobylPlugin
                .resolve_deploy_root(install, &weird)
                .unwrap(),
            PathBuf::from("/game/Stalker2/Content/Paks/LogicMods/Bp.pak")
        );
    }
}
