//! Read loader logs after a launch and turn common failures into plain findings.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::SystemTime;

use regex::Regex;
use serde::Serialize;

use crate::games::{GamePlugin, LogKind, LogSource};

const TAIL_BYTES: u64 = 256 * 1024;
const MAX_FINDINGS: usize = 8;

#[derive(Debug, Clone, Serialize)]
pub struct LoadFinding {
    pub severity: String,
    pub title: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub related_mod: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadDiagnosis {
    pub summary: String,
    pub findings: Vec<LoadFinding>,
    pub report: String,
    /// Label of the log the excerpt was taken from.
    pub excerpt_label: String,
    /// Filesystem path of that log, so the UI can open it.
    pub excerpt_path: String,
    /// Last lines of that log, shown even when nothing failed.
    pub excerpt: String,
}

pub struct DiagnoseInput<'a> {
    pub plugin: &'a dyn GamePlugin,
    pub install_path: &'a Path,
    pub game_title: &'a str,
    pub plugin_id: &'a str,
    pub app_version: &'a str,
    pub enabled_mods: &'a [String],
    pub deployed_paths: &'a [String],
    pub deployed_at: Option<SystemTime>,
    pub manager_log: &'a str,
}

struct ReadLog {
    source: LogSource,
    text: String,
    modified: SystemTime,
}

pub fn diagnose(input: &DiagnoseInput<'_>) -> LoadDiagnosis {
    let sources = input.plugin.load_log_sources(input.install_path);
    let read = read_sources(&sources);
    let mods = ModIndex {
        names: input.enabled_mods,
        deployed: input.deployed_paths,
    };

    if read.is_empty() {
        let summary = missing_loader_summary(input.plugin, input.install_path);
        return finish(input, &sources, &read, &summary, Vec::new());
    }

    let mut findings = Vec::new();
    let mut crash_summary = None;
    for log in &read {
        if findings.len() >= MAX_FINDINGS {
            break;
        }
        if log.source.kind == LogKind::Papyrus && !papyrus_is_current(log, &read) {
            continue;
        }
        for finding in findings_for(log.source.kind, &log.text, &mods) {
            if findings.len() >= MAX_FINDINGS {
                break;
            }
            if findings.iter().any(|existing: &LoadFinding| {
                existing.title == finding.title && existing.detail == finding.detail
            }) {
                continue;
            }
            if crash_summary.is_none() && log.source.kind == LogKind::CrashLog {
                crash_summary = Some(finding.title.clone());
            }
            findings.push(finding);
        }
    }

    let errors: Vec<&LoadFinding> = findings
        .iter()
        .filter(|finding| finding.severity == "error")
        .collect();
    let summary = if is_stale(&read, input.deployed_at) {
        "The newest loader log is from before the last deploy. Launch the game, then check again."
            .to_string()
    } else if let Some(title) = crash_summary {
        title
    } else if errors.is_empty() {
        if read.iter().all(|log| log.source.kind == LogKind::Player)
            || read.iter().all(|log| log.text.trim().is_empty())
        {
            "No loader log was found. Launch the game once so the loader can write a log, then check again."
                .to_string()
        } else if let Some(summary) = extender_finished_summary(&read, &sources) {
            summary
        } else if let Some(log) = excerpt_source(&read) {
            clean_summary(log, input.enabled_mods)
        } else {
            "The loader started and no plugin errors were found in the latest log.".to_string()
        }
    } else if errors.len() == 1 {
        errors[0].title.clone()
    } else {
        format!("Found {} load errors in the latest log.", errors.len())
    };

    finish(input, &sources, &read, &summary, findings)
}

fn finish(
    input: &DiagnoseInput<'_>,
    sources: &[LogSource],
    read: &[ReadLog],
    summary: &str,
    findings: Vec<LoadFinding>,
) -> LoadDiagnosis {
    let report = redact(&build_report(input, sources, read, summary, &findings));
    let (excerpt_label, excerpt_path, excerpt) = excerpt_of(read);
    LoadDiagnosis {
        summary: summary.to_string(),
        findings,
        report,
        excerpt_label,
        excerpt_path,
        excerpt,
    }
}

fn excerpt_source(read: &[ReadLog]) -> Option<&ReadLog> {
    read.iter()
        .find(|log| is_primary_log(log.source.kind) && !log.text.trim().is_empty())
        .or_else(|| read.iter().find(|log| !log.text.trim().is_empty()))
}

/// Loader and crash logs. Plugin and Papyrus files stay findings, not the excerpt.
fn is_primary_log(kind: LogKind) -> bool {
    !matches!(
        kind,
        LogKind::Player | LogKind::PluginLog | LogKind::Papyrus | LogKind::CrashDump
    )
}

fn excerpt_of(read: &[ReadLog]) -> (String, String, String) {
    let Some(log) = excerpt_source(read) else {
        return (String::new(), String::new(), String::new());
    };
    let text = tail_lines(&log.text, 40);
    if text.trim().is_empty() {
        return (String::new(), String::new(), String::new());
    }
    let file = log
        .source
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let label = if file.is_empty() || log.source.label.contains(&file) {
        log.source.label.clone()
    } else {
        format!("{} {file}", log.source.label)
    };
    (label, log.source.path.display().to_string(), text)
}

fn clean_summary(log: &ReadLog, enabled: &[String]) -> String {
    let status = status_line(&log.text).unwrap_or_else(|| "loader started".into());
    let (label, _, _) = excerpt_of(std::slice::from_ref(log));
    let mut summary = format!("{label}: {status}. No plugin errors.");
    if none_of_enabled_mods_appear(enabled, &log.text) {
        summary.push_str(" None of the enabled mods appear in this log.");
    }
    summary
}

fn status_line(text: &str) -> Option<String> {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return None;
    }
    const PREFERRED: &[&str] = &[
        "chainloader startup complete",
        "init complete",
        "plugins loaded",
        "mods loaded",
    ];
    if let Some(line) = lines.iter().rev().find(|line| {
        let lower = line.to_lowercase();
        PREFERRED.iter().any(|needle| lower.contains(needle))
    }) {
        return Some(compact_status(line));
    }
    lines.last().copied().map(compact_status)
}

fn compact_status(line: &str) -> String {
    if let Some(idx) = line.rfind(']') {
        let rest = line[idx + 1..].trim();
        if !rest.is_empty() {
            return rest.to_string();
        }
    }
    line.to_string()
}

fn none_of_enabled_mods_appear(enabled: &[String], text: &str) -> bool {
    let names: Vec<String> = enabled
        .iter()
        .map(|name| name.trim().to_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    if names.is_empty() {
        return false;
    }
    let lower = text.to_lowercase();
    names.iter().all(|name| !lower.contains(name.as_str()))
}

fn missing_loader_summary(plugin: &dyn GamePlugin, install_path: &Path) -> String {
    let warnings = plugin.preflight_warnings(install_path);
    if let Some(warning) = warnings.iter().find(|warning| {
        let lower = warning.to_lowercase();
        lower.contains("will not load")
            || lower.contains("not found")
            || lower.contains("never runs")
            || lower.contains("no crloader.log")
    }) {
        return warning.clone();
    }
    "No loader log was found. Launch the game once so the loader can write a log, then check again."
        .to_string()
}

fn is_stale(logs: &[ReadLog], deployed_at: Option<SystemTime>) -> bool {
    let Some(deployed_at) = deployed_at else {
        return false;
    };
    let Some(newest) = logs.iter().map(|log| log.modified).max() else {
        return false;
    };
    newest < deployed_at
}

fn read_sources(sources: &[LogSource]) -> Vec<ReadLog> {
    let mut read = Vec::new();
    for source in sources {
        if source.kind == LogKind::CrashDump {
            continue;
        }
        let Some((text, modified)) = read_tail(&source.path) else {
            continue;
        };
        read.push(ReadLog {
            source: source.clone(),
            text,
            modified,
        });
    }
    read.sort_by(|a, b| {
        let rank = |kind: LogKind| match kind {
            LogKind::CrashLog => 0,
            LogKind::PluginLog | LogKind::Papyrus => 2,
            LogKind::Player | LogKind::CrashDump => 3,
            _ => 1,
        };
        rank(a.source.kind)
            .cmp(&rank(b.source.kind))
            .then(b.modified.cmp(&a.modified))
    });
    read
}

fn read_tail(path: &Path) -> Option<(String, SystemTime)> {
    let mut file = File::open(path).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let len = meta.len();
    let start = len.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        if let Some(idx) = text.find('\n') {
            text.drain(..=idx);
        }
    }
    Some((text, modified))
}

struct ModIndex<'a> {
    names: &'a [String],
    deployed: &'a [String],
}

fn findings_for(kind: LogKind, text: &str, mods: &ModIndex<'_>) -> Vec<LoadFinding> {
    match kind {
        LogKind::BepInEx => bepinex_findings(text, mods),
        LogKind::MelonLoader => bracket_error_findings(text, mods, "MelonLoader", "[error]"),
        LogKind::Smapi => smapi_findings(text, mods),
        LogKind::ScriptExtender => script_extender_findings(text, mods),
        LogKind::CrashLog => crash_log_findings(text, mods),
        LogKind::ReFramework => excerpt_finding(text, "REFramework reported an error."),
        LogKind::Ue4ss => excerpt_finding(text, "UE4SS reported an error."),
        LogKind::Ds2Lighting => excerpt_finding(text, "Lighting Engine reported an error."),
        LogKind::CrLoader => excerpt_finding(text, "crloader reported an error."),
        LogKind::Player => excerpt_finding(text, "Unity Player.log reported an error."),
        LogKind::Console => excerpt_finding(text, "Project Zomboid console.txt reported an error."),
        LogKind::PluginLog => named_error_finding(text, mods, "A plugin log reported an error."),
        LogKind::Papyrus => papyrus_findings(text, mods),
        LogKind::CrashDump => Vec::new(),
    }
}

fn bepinex_findings(text: &str, mods: &ModIndex<'_>) -> Vec<LoadFinding> {
    let mut findings = Vec::new();
    let chain: Vec<&str> = text
        .lines()
        .filter(|line| is_chainloader_failure(line))
        .take(4)
        .collect();
    if !chain.is_empty() {
        findings.push(LoadFinding {
            severity: "error".into(),
            title: "BepInEx did not finish starting.".into(),
            detail: chain.join("\n"),
            related_mod: None,
        });
    }

    let mut seen_mods = Vec::new();
    for line in text.lines() {
        if is_chainloader_failure(line) || !is_load_exception(line) {
            continue;
        }
        let hints = hints_from_line(line);
        let related = match_mod(mods, &hints);
        if let Some(name) = &related {
            if seen_mods.iter().any(|seen| seen == name) {
                continue;
            }
            seen_mods.push(name.clone());
        } else if findings.iter().any(|finding| {
            finding.related_mod.is_none() && finding.title == "A plugin failed to load."
        }) {
            continue;
        }
        findings.push(LoadFinding {
            severity: "error".into(),
            title: match &related {
                Some(name) => format!("{name} failed to load."),
                None => "A plugin failed to load.".into(),
            },
            detail: line.trim().to_string(),
            related_mod: related,
        });
        if findings.len() >= MAX_FINDINGS {
            return findings;
        }
    }

    if findings.is_empty() {
        if let Some(excerpt) = error_excerpt(text) {
            let hints = hints_from_line(&excerpt);
            let related = match_mod(mods, &hints);
            findings.push(LoadFinding {
                severity: "error".into(),
                title: match &related {
                    Some(name) => format!("{name} reported an error."),
                    None => "BepInEx reported an error.".into(),
                },
                detail: excerpt,
                related_mod: related,
            });
        }
    }
    findings
}

fn is_chainloader_failure(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("could not run preloader")
        || lower.contains("chainloader startup failed")
        || lower.contains("failed to start chainloader")
        || lower.contains("preloader failed")
}

fn is_load_exception(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("filenotfoundexception")
        || lower.contains("missingmethodexception")
        || lower.contains("typeloadexception")
        || lower.contains("dllnotfoundexception")
        || lower.contains("reflectiontypeloadexception")
}

fn bracket_error_findings(
    text: &str,
    mods: &ModIndex<'_>,
    loader: &str,
    marker: &str,
) -> Vec<LoadFinding> {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| line.to_lowercase().contains(marker))
        .take(8)
        .collect();
    if lines.is_empty() {
        return excerpt_finding(text, &format!("{loader} reported an error."));
    }
    let hints = lines
        .iter()
        .flat_map(|line| hints_from_line(line))
        .collect::<Vec<_>>();
    let related = match_mod(mods, &hints);
    vec![LoadFinding {
        severity: "error".into(),
        title: match &related {
            Some(name) => format!("{name} failed to load."),
            None => format!("{loader} reported an error."),
        },
        detail: lines.join("\n"),
        related_mod: related,
    }]
}

fn smapi_findings(text: &str, mods: &ModIndex<'_>) -> Vec<LoadFinding> {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            lower.contains("failed to load")
                || lower.contains("could not be loaded")
                || lower.contains("couldn't be loaded")
                || lower.contains("these mods could not")
        })
        .take(10)
        .collect();
    if lines.is_empty() {
        return excerpt_finding(text, "SMAPI reported an error.");
    }
    let hints = lines
        .iter()
        .flat_map(|line| hints_from_line(line))
        .collect::<Vec<_>>();
    let related = match_mod(mods, &hints);
    vec![LoadFinding {
        severity: "error".into(),
        title: match &related {
            Some(name) => format!("{name} failed to load."),
            None => "SMAPI could not load a mod.".into(),
        },
        detail: lines.join("\n"),
        related_mod: related,
    }]
}

struct ExtenderPlugin {
    dll: String,
    name: String,
    status: String,
    line: String,
}

fn extender_plugin_re() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?i)^plugin\s+(\S+)\s+\(([0-9A-Fa-f]+)\s+(.+?)\s+([0-9A-Fa-f]+)\)\s+(.+?)\s+\(handle\s+\d+\)\s*$",
        )
        .unwrap()
    })
}

fn extender_plugins(text: &str) -> Vec<ExtenderPlugin> {
    let mut plugins = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(caps) = extender_plugin_re().captures(trimmed) else {
            continue;
        };
        plugins.push(ExtenderPlugin {
            dll: caps.get(1).map(|m| m.as_str()).unwrap_or("").to_string(),
            name: caps
                .get(3)
                .map(|m| m.as_str().trim())
                .unwrap_or("")
                .to_string(),
            status: caps
                .get(5)
                .map(|m| m.as_str().trim())
                .unwrap_or("")
                .to_string(),
            line: trimmed.to_string(),
        });
    }
    plugins
}

fn plugin_loaded(status: &str) -> bool {
    status.eq_ignore_ascii_case("loaded correctly")
}

fn plugin_hints(dll: &str, name: &str) -> Vec<String> {
    let mut hints = Vec::new();
    if let Some(stem) = Path::new(dll).file_stem() {
        let stem = stem.to_string_lossy().into_owned();
        if !stem.is_empty() {
            hints.push(stem);
        }
    }
    if !dll.is_empty() {
        hints.push(dll.to_string());
    }
    if !name.is_empty() {
        hints.push(name.to_string());
    }
    hints
}

fn plugin_label(dll: &str, name: &str, related: Option<&str>) -> String {
    if let Some(related) = related {
        return related.to_string();
    }
    if !name.is_empty() {
        return name.to_string();
    }
    Path::new(dll)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| dll.to_string())
}

fn extender_folder(label: &str) -> &str {
    label
        .split_whitespace()
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("The script extender")
}

/// Papyrus from an earlier launch must not be blamed for this one.
fn papyrus_is_current(papyrus: &ReadLog, read: &[ReadLog]) -> bool {
    let Some(extender_mtime) = read
        .iter()
        .filter(|log| log.source.kind == LogKind::ScriptExtender)
        .map(|log| log.modified)
        .max()
    else {
        return false;
    };
    papyrus.modified >= extender_mtime
}

/// SFSE/SKSE/F4SE stop at `init complete`. A crash after that is not in this file.
fn extender_finished_summary(read: &[ReadLog], sources: &[LogSource]) -> Option<String> {
    if read.iter().any(|log| log.source.kind == LogKind::CrashLog) {
        return None;
    }
    let log = read.iter().find(|log| {
        log.source.kind == LogKind::ScriptExtender
            && log.text.to_lowercase().contains("init complete")
    })?;
    let loaded = extender_plugins(&log.text)
        .iter()
        .filter(|plugin| plugin_loaded(&plugin.status))
        .count();
    let folder = extender_folder(&log.source.label);
    let plugins = if loaded == 1 { "plugin" } else { "plugins" };
    let dump = sources
        .iter()
        .any(|source| source.kind == LogKind::CrashDump && source.path.is_file());
    let missing = if dump {
        format!(
            "A crash dump (.dmp) was written, but it does not name the mod. Crash Logger writes crash-*.log in {folder}/Crashlogs, which does."
        )
    } else {
        format!(
            "{folder} does not write a crash stack. Crash Logger writes crash-*.log in {folder}/Crashlogs, which names the faulting DLL."
        )
    };
    Some(format!(
        "{folder} finished starting and loaded {loaded} {plugins}. The log ends at init complete, before the game loads data, so the crash happened after that. {missing}"
    ))
}

fn script_extender_findings(text: &str, mods: &ModIndex<'_>) -> Vec<LoadFinding> {
    let plugins = extender_plugins(text);
    if plugins.is_empty() {
        return excerpt_finding(text, "The script extender reported an error.");
    }
    let mut findings = Vec::new();
    for plugin in &plugins {
        if plugin_loaded(&plugin.status) {
            continue;
        }
        let related = match_mod(mods, &plugin_hints(&plugin.dll, &plugin.name));
        let label = plugin_label(&plugin.dll, &plugin.name, related.as_deref());
        findings.push(LoadFinding {
            severity: "error".into(),
            title: format!("{label} failed to load."),
            detail: plugin.line.clone(),
            related_mod: related,
        });
    }
    let loaded: Vec<&ExtenderPlugin> = plugins
        .iter()
        .filter(|plugin| plugin_loaded(&plugin.status))
        .collect();
    if !loaded.is_empty() {
        let detail = loaded
            .iter()
            .map(|plugin| {
                let related = match_mod(mods, &plugin_hints(&plugin.dll, &plugin.name));
                let shown = plugin_label(&plugin.dll, &plugin.name, related.as_deref());
                format!("{} — {shown}", plugin.dll)
            })
            .collect::<Vec<_>>()
            .join("\n");
        findings.push(LoadFinding {
            severity: "info".into(),
            title: "Loaded plugins".into(),
            detail,
            related_mod: None,
        });
    }
    findings
}

fn crash_log_findings(text: &str, mods: &ModIndex<'_>) -> Vec<LoadFinding> {
    let lines: Vec<&str> = text.lines().collect();
    let exception = lines
        .iter()
        .find(|line| is_crash_exception_line(line))
        .map(|line| line.trim().to_string());
    let mut detail = Vec::new();
    if let Some(line) = &exception {
        detail.push(line.clone());
    }
    for line in call_stack_excerpt(&lines) {
        if detail.iter().any(|existing| existing == &line) {
            continue;
        }
        detail.push(line);
        if detail.len() >= 7 {
            break;
        }
    }
    if detail.is_empty() {
        return excerpt_finding(text, "The game crashed.");
    }
    let hints = detail
        .iter()
        .flat_map(|line| hints_from_line(line))
        .collect::<Vec<_>>();
    let related = match_mod(mods, &hints);
    let title = match &related {
        Some(name) => format!("{name} crashed the game."),
        None => crash_title(exception.as_deref()),
    };
    vec![LoadFinding {
        severity: "error".into(),
        title,
        detail: detail.join("\n"),
        related_mod: related,
    }]
}

fn crash_title(exception: Option<&str>) -> String {
    let Some(line) = exception else {
        return "The game crashed.".into();
    };
    match exception_code(line) {
        Some(code) => format!("The game crashed ({code})."),
        None => "The game crashed.".into(),
    }
}

fn exception_code(line: &str) -> Option<String> {
    let lower = line.to_lowercase();
    let start = lower.find("exception_")?;
    let rest = &line[start..];
    let end = rest
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .unwrap_or(rest.len());
    let code = &rest[..end];
    (!code.is_empty()).then(|| code.to_string())
}

fn is_crash_exception_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("unhandled exception")
        || lower.contains("exception_")
        || lower.trim_start().starts_with("exception:")
}

fn call_stack_excerpt(lines: &[&str]) -> Vec<String> {
    let header = lines.iter().position(|line| {
        let lower = line.to_lowercase();
        lower.contains("probable call stack")
            || lower.contains("callstack")
            || lower.contains("call stack")
    });
    let mut out = Vec::new();
    if let Some(header) = header {
        for line in lines.iter().skip(header + 1) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                if !out.is_empty() {
                    break;
                }
                continue;
            }
            if is_crash_section(trimmed) {
                break;
            }
            out.push(trimmed.to_string());
            if out.len() >= 6 {
                break;
            }
        }
    }
    if out.is_empty() {
        for line in lines {
            let trimmed = line.trim();
            if trimmed.to_lowercase().contains(".dll") && !is_crash_exception_line(trimmed) {
                out.push(trimmed.to_string());
                if out.len() >= 6 {
                    break;
                }
            }
        }
    }
    out
}

fn is_crash_section(line: &str) -> bool {
    line.ends_with(':') && !line.to_lowercase().contains(".dll")
}

fn named_error_finding(text: &str, mods: &ModIndex<'_>, fallback: &str) -> Vec<LoadFinding> {
    let Some(excerpt) = error_excerpt(text) else {
        return Vec::new();
    };
    let hints = excerpt
        .lines()
        .flat_map(hints_from_line)
        .collect::<Vec<_>>();
    let related = match_mod(mods, &hints);
    let title = match &related {
        Some(name) => format!("{name} reported an error."),
        None => fallback.to_string(),
    };
    vec![LoadFinding {
        severity: "error".into(),
        title,
        detail: excerpt,
        related_mod: related,
    }]
}

fn papyrus_findings(text: &str, mods: &ModIndex<'_>) -> Vec<LoadFinding> {
    let lines: Vec<&str> = text
        .lines()
        .filter(|line| line.to_lowercase().contains("error:"))
        .take(8)
        .collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let hints = lines
        .iter()
        .flat_map(|line| {
            hints_from_line(line)
                .into_iter()
                .chain(script_plugin_hints(line))
        })
        .collect::<Vec<_>>();
    let related = match_mod(mods, &hints);
    let title = match &related {
        Some(name) => format!("{name} reported a script error."),
        None => "Papyrus reported a script error.".into(),
    };
    vec![LoadFinding {
        severity: "error".into(),
        title,
        detail: lines.join("\n"),
        related_mod: related,
    }]
}

fn script_plugin_hints(line: &str) -> Vec<String> {
    let mut hints = Vec::new();
    for caps in script_plugin_name().captures_iter(line) {
        let raw = caps
            .get(1)
            .or_else(|| caps.get(2))
            .map(|m| m.as_str().trim())
            .unwrap_or("");
        if raw.is_empty() {
            continue;
        }
        hints.push(raw.to_string());
        if let Some(stem) = Path::new(raw).file_stem() {
            hints.push(stem.to_string_lossy().into_owned());
        }
    }
    hints
}

fn script_plugin_name() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"(?i)"([^"]+)\.(?:esp|esm|esl)"|\b([A-Za-z0-9_.\-]+)\.(?:esp|esm|esl)\b"#)
            .unwrap()
    })
}

fn excerpt_finding(text: &str, title: &str) -> Vec<LoadFinding> {
    let Some(excerpt) = error_excerpt(text) else {
        return Vec::new();
    };
    vec![LoadFinding {
        severity: "error".into(),
        title: title.to_string(),
        detail: excerpt,
        related_mod: None,
    }]
}

fn error_excerpt(text: &str) -> Option<String> {
    let lines: Vec<String> = text
        .lines()
        .filter(|line| looks_like_error_line(line))
        .take(12)
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.chars().count() > 240 {
                trimmed.chars().take(240).collect()
            } else {
                trimmed.to_string()
            }
        })
        .collect();
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

fn looks_like_error_line(line: &str) -> bool {
    let lower = line.to_lowercase();
    lower.contains("[error")
        || lower.contains("[fatal")
        || lower.contains("exception")
        || lower.contains("failed to load")
        || lower.contains("could not load")
        || lower.contains("couldn't load")
        || lower.contains("could not run")
        || lower.contains("crash!!!")
        || lower.contains("fatal error")
}

fn hints_from_line(line: &str) -> Vec<String> {
    let mut hints = Vec::new();
    if let Some(caps) = bepinex_header().captures(line) {
        let name = caps.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        if !name.is_empty() && !is_loader_label(name) {
            hints.push(name.to_string());
        }
    }
    for caps in dll_name().captures_iter(line) {
        if let Some(matched) = caps.get(1) {
            let raw = matched.as_str().trim();
            hints.push(raw.to_string());
            if let Some(stem) = Path::new(raw).file_stem() {
                hints.push(stem.to_string_lossy().into_owned());
            }
        }
    }
    hints
}

fn bepinex_header() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\[(?:Error|Fatal)\s*:\s*([^\]]+)\]").unwrap())
}

fn dll_name() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b([A-Za-z0-9_.\-]+)\.dll\b").unwrap())
}

fn is_loader_label(name: &str) -> bool {
    let lower = name.trim().to_lowercase();
    matches!(
        lower.as_str(),
        "bepinex" | "chainloader" | "preloader" | "unity doorstop"
    )
}

fn match_mod(mods: &ModIndex<'_>, hints: &[String]) -> Option<String> {
    let named: Vec<(String, String)> = mods
        .names
        .iter()
        .map(|name| (normalize_key(name), name.clone()))
        .filter(|(key, _)| key.len() >= 3)
        .collect();
    for hint in hints {
        let key = normalize_key(hint);
        if key.len() < 3 {
            continue;
        }
        if let Some((_, name)) = named.iter().find(|(candidate, _)| candidate == &key) {
            return Some(name.clone());
        }
        for path in mods.deployed {
            let file = Path::new(path);
            let stem = file
                .file_stem()
                .map(|value| normalize_key(&value.to_string_lossy()))
                .unwrap_or_default();
            let parent = file
                .parent()
                .and_then(|dir| dir.file_name())
                .map(|value| normalize_key(&value.to_string_lossy()))
                .unwrap_or_default();
            if stem == key || parent == key {
                if let Some((_, name)) = named.iter().find(|(candidate, _)| {
                    candidate == &key || candidate == &stem || candidate == &parent
                }) {
                    return Some(name.clone());
                }
            }
        }
    }
    let mut best: Option<(usize, String)> = None;
    for hint in hints {
        let key = normalize_key(hint);
        for (candidate, name) in &named {
            let shorter = key.len().min(candidate.len());
            if shorter < 4 {
                continue;
            }
            if key.contains(candidate.as_str()) || candidate.contains(&key) {
                if best
                    .as_ref()
                    .map(|(len, _)| candidate.len() > *len)
                    .unwrap_or(true)
                {
                    best = Some((candidate.len(), name.clone()));
                }
            }
        }
    }
    best.map(|(_, name)| name)
}

fn normalize_key(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn build_report(
    input: &DiagnoseInput<'_>,
    sources: &[LogSource],
    read: &[ReadLog],
    summary: &str,
    findings: &[LoadFinding],
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "Emperor Mod Manager {}\nGame: {} ({})\n\n",
        input.app_version, input.game_title, input.plugin_id
    ));
    out.push_str("Enabled mods:\n");
    if input.enabled_mods.is_empty() {
        out.push_str("- (none enabled)\n");
    } else {
        for name in input.enabled_mods {
            out.push_str(&format!("- {name}\n"));
        }
    }
    out.push_str(&format!("\nSummary:\n{summary}\n"));
    if !findings.is_empty() {
        out.push_str("\nFindings:\n");
        for finding in findings {
            out.push_str(&format!("- {}\n{}\n", finding.title, finding.detail));
            if let Some(name) = &finding.related_mod {
                out.push_str(&format!("Likely mod: {name}\n"));
            }
        }
    }
    out.push_str("\nLogs checked:\n");
    if sources.is_empty() {
        out.push_str("- (this game has no known loader log)\n");
    }
    for source in sources {
        let found = read.iter().any(|log| log.source.path == source.path)
            || (source.kind == LogKind::CrashDump && source.path.is_file());
        let state = if found { "found" } else { "missing" };
        out.push_str(&format!(
            "- {} ({state}): {}\n",
            source.label,
            source.path.display()
        ));
    }
    if let Some(log) = excerpt_source(read) {
        out.push_str(&format!(
            "\nLog excerpt ({}, {}):\n{}\n",
            log.source.label,
            log.source.path.display(),
            tail_lines(&log.text, 80)
        ));
    } else if let Some(log) = read.first() {
        out.push_str(&format!(
            "\nLog excerpt ({}, {}):\n{}\n",
            log.source.label,
            log.source.path.display(),
            tail_lines(&log.text, 80)
        ));
    }
    if !input.manager_log.trim().is_empty() {
        out.push_str(&format!("\nManager log:\n{}\n", input.manager_log.trim()));
    }
    out
}

fn tail_lines(text: &str, count: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(count);
    lines[start..].join("\n")
}

fn redact(text: &str) -> String {
    let Some(base) = directories::BaseDirs::new() else {
        return text.to_string();
    };
    let home = base.home_dir().display().to_string();
    if home.is_empty() {
        return text.to_string();
    }
    let mut out = text.to_string();
    for prefix in [
        home.clone(),
        home.replace('\\', "/"),
        home.replace('/', "\\"),
    ] {
        if prefix.is_empty() {
            continue;
        }
        out = out.replace(&format!("{prefix}/"), "~/");
        out = out.replace(&format!("{prefix}\\"), "~\\");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::BepInExPlugin;
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::Duration;

    fn input<'a>(
        install: &'a Path,
        enabled: &'a [String],
        deployed: &'a [String],
        deployed_at: Option<SystemTime>,
    ) -> DiagnoseInput<'a> {
        DiagnoseInput {
            plugin: &BepInExPlugin,
            install_path: install,
            game_title: "Lethal Company",
            plugin_id: "lethalcompany",
            app_version: "0.0.0-test",
            enabled_mods: enabled,
            deployed_paths: deployed,
            deployed_at,
            manager_log: "",
        }
    }

    #[test]
    fn chainloader_failure_is_a_finding() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("BepInEx").join("LogOutput.log");
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(
            &log,
            "[Error  :   BepInEx] Could not run preloader!\n[Error  :Chainloader] Chainloader startup failed\n",
        )
        .unwrap();
        let enabled = Vec::new();
        let deployed = Vec::new();
        let diagnosis = diagnose(&input(dir.path(), &enabled, &deployed, None));
        assert!(
            diagnosis.summary.contains("did not finish starting"),
            "{}",
            diagnosis.summary
        );
        assert!(diagnosis.findings.iter().any(|finding| {
            finding
                .detail
                .to_lowercase()
                .contains("chainloader startup failed")
        }));
    }

    #[test]
    fn plugin_exception_names_the_matching_mod() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("BepInEx").join("LogOutput.log");
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(
            &log,
            "[Error  :  BetterUI] FileNotFoundException: Could not load BetterUI.dll\n",
        )
        .unwrap();
        let enabled = vec!["BetterUI".to_string()];
        let deployed = vec![dir
            .path()
            .join("BepInEx/plugins/BetterUI/BetterUI.dll")
            .display()
            .to_string()];
        let diagnosis = diagnose(&input(dir.path(), &enabled, &deployed, None));
        assert!(
            diagnosis
                .findings
                .iter()
                .any(|finding| finding.related_mod.as_deref() == Some("BetterUI")),
            "{:?}",
            diagnosis.findings
        );
    }

    #[test]
    fn log_older_than_deploy_asks_for_a_new_launch() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("BepInEx").join("LogOutput.log");
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        let mut file = fs::File::create(&log).unwrap();
        writeln!(file, "[Message: BepInEx] Chainloader startup complete").unwrap();
        file.flush().unwrap();
        let old = SystemTime::now() - Duration::from_secs(7200);
        file.set_modified(old).unwrap();
        drop(file);
        let enabled = Vec::new();
        let deployed = Vec::new();
        let diagnosis = diagnose(&input(
            dir.path(),
            &enabled,
            &deployed,
            Some(SystemTime::now()),
        ));
        assert!(
            diagnosis.summary.contains("before the last deploy"),
            "{}",
            diagnosis.summary
        );
    }

    #[test]
    fn missing_loader_log_reuses_the_preflight_warning() {
        let dir = tempfile::tempdir().unwrap();
        let enabled = Vec::new();
        let deployed = Vec::new();
        let diagnosis = diagnose(&input(dir.path(), &enabled, &deployed, None));
        assert!(
            diagnosis.summary.contains("BepInEx not found")
                && diagnosis.summary.to_lowercase().contains("will not load"),
            "{}",
            diagnosis.summary
        );
        assert!(diagnosis.findings.is_empty());
    }

    #[test]
    fn clean_loader_log_reports_no_plugin_errors() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("BepInEx").join("LogOutput.log");
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(&log, "[Message: BepInEx] Chainloader startup complete\n").unwrap();
        let enabled = Vec::new();
        let deployed = Vec::new();
        let diagnosis = diagnose(&input(dir.path(), &enabled, &deployed, None));
        assert!(
            diagnosis.summary.contains("BepInEx LogOutput.log")
                && diagnosis.summary.contains("Chainloader startup complete")
                && diagnosis.summary.contains("No plugin errors"),
            "{}",
            diagnosis.summary
        );
        assert!(diagnosis.findings.is_empty());
        assert!(
            diagnosis.excerpt.contains("Chainloader startup complete"),
            "{}",
            diagnosis.excerpt
        );
        assert_eq!(diagnosis.excerpt_label, "BepInEx LogOutput.log");
        assert!(
            diagnosis.excerpt_path.ends_with("LogOutput.log"),
            "{}",
            diagnosis.excerpt_path
        );
    }

    #[test]
    fn clean_loader_log_notes_enabled_mods_missing_from_the_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("BepInEx").join("LogOutput.log");
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(&log, "[Message: BepInEx] Chainloader startup complete\n").unwrap();
        let enabled = vec!["BetterUI".to_string()];
        let deployed = Vec::new();
        let diagnosis = diagnose(&input(dir.path(), &enabled, &deployed, None));
        assert!(
            diagnosis
                .summary
                .contains("None of the enabled mods appear in this log"),
            "{}",
            diagnosis.summary
        );
    }

    fn starfield_input<'a>(
        install: &'a Path,
        enabled: &'a [String],
        deployed: &'a [String],
    ) -> DiagnoseInput<'a> {
        DiagnoseInput {
            plugin: &crate::games::StarfieldPlugin,
            install_path: install,
            game_title: "Starfield",
            plugin_id: "starfield",
            app_version: "0.0.0-test",
            enabled_mods: enabled,
            deployed_paths: deployed,
            deployed_at: None,
            manager_log: "",
        }
    }

    fn starfield_install(tmp: &Path) -> PathBuf {
        let install = tmp.join("steamapps").join("common").join("Starfield");
        fs::create_dir_all(&install).unwrap();
        install
    }

    fn starfield_sfse(tmp: &Path) -> PathBuf {
        tmp.join("steamapps/compatdata/1716740/pfx/drive_c/users/steamuser/Documents/My Games/Starfield/SFSE")
    }

    #[test]
    fn crash_log_names_the_matching_mod_ahead_of_a_clean_sfse_log() {
        let tmp = tempfile::tempdir().unwrap();
        let install = starfield_install(tmp.path());
        let sfse = starfield_sfse(tmp.path());
        let logs = sfse.join("Logs");
        let crashes = sfse.join("Crashlogs");
        fs::create_dir_all(&logs).unwrap();
        fs::create_dir_all(&crashes).unwrap();
        fs::write(
            logs.join("sfse.txt"),
            "SFSE runtime: initialize\ninit complete\n",
        )
        .unwrap();
        fs::write(
            crashes.join("crash_2026-01-01.log"),
            "\
Unhandled exception \"EXCEPTION_ACCESS_VIOLATION\" at 0x7FF600000000 BetterShips.dll+001234

PROBABLE CALL STACK:
\t[0] 0x7FF600000000 BetterShips.dll+001234
\t[1] 0x7FF600001000 Starfield.exe+00ABCDEF

REGISTERS:
\tRAX 0
",
        )
        .unwrap();
        let enabled = vec!["Better Ships".to_string()];
        let deployed = Vec::new();
        let diagnosis = diagnose(&starfield_input(&install, &enabled, &deployed));
        assert!(
            diagnosis.summary.contains("Better Ships"),
            "{}",
            diagnosis.summary
        );
        assert!(
            !diagnosis.summary.contains("no plugin errors"),
            "{}",
            diagnosis.summary
        );
        assert!(diagnosis.findings.iter().any(|finding| {
            finding.related_mod.as_deref() == Some("Better Ships")
                && finding.detail.contains("BetterShips.dll")
        }));
    }

    #[test]
    fn missing_sfse_log_reports_no_loader() {
        let tmp = tempfile::tempdir().unwrap();
        let install = starfield_install(tmp.path());
        fs::write(install.join("sfse_loader.exe"), b"x").unwrap();
        let enabled = Vec::new();
        let deployed = Vec::new();
        let diagnosis = diagnose(&starfield_input(&install, &enabled, &deployed));
        assert!(
            diagnosis.summary.contains("No loader log was found"),
            "{}",
            diagnosis.summary
        );
        assert!(
            diagnosis.report.contains("sfse.txt") && diagnosis.report.contains("missing"),
            "{}",
            diagnosis.report
        );
        assert!(diagnosis.findings.is_empty());
    }

    #[test]
    fn sfse_init_complete_names_loaded_plugins_and_missing_crash_log() {
        let tmp = tempfile::tempdir().unwrap();
        let install = starfield_install(tmp.path());
        let logs = starfield_sfse(tmp.path()).join("Logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(
            logs.join("sfse.txt"),
            "\
loading plugin \"SeamlessGravJumps\"
plugin SeamlessGravJumps.dll (00000001 SeamlessGravJumps 00000000) loaded correctly (handle 6)
plugin StarfieldEngineFixes.dll (00000001 Starfield Engine Fixes - SFSE Plugin by LarannKiar 15020000) loaded correctly (handle 11)
plugin Luma.dll (00000001 Luma 00004E20) loaded correctly (handle 4)
init complete
",
        )
        .unwrap();
        let enabled = vec!["Starfield Engine Fixes".to_string(), "Luma".to_string()];
        let deployed = Vec::new();
        let diagnosis = diagnose(&starfield_input(&install, &enabled, &deployed));
        assert!(
            diagnosis.summary.contains("SFSE")
                && diagnosis.summary.contains("after")
                && diagnosis.summary.contains("does not write a crash stack")
                && diagnosis.summary.contains("Crash Logger")
                && diagnosis.summary.contains("3 plugins"),
            "{}",
            diagnosis.summary
        );
        assert!(
            !diagnosis.summary.contains("No plugin errors"),
            "{}",
            diagnosis.summary
        );
        let loaded = diagnosis
            .findings
            .iter()
            .find(|finding| finding.title == "Loaded plugins")
            .expect("loaded plugin finding");
        assert_eq!(loaded.severity, "info");
        assert!(loaded.detail.contains("SeamlessGravJumps.dll"));
        assert!(loaded
            .detail
            .contains("StarfieldEngineFixes.dll — Starfield Engine Fixes"));
        assert!(loaded.detail.contains("Luma.dll — Luma"));
    }

    #[test]
    fn sfse_disabled_plugin_names_the_matching_mod() {
        let tmp = tempfile::tempdir().unwrap();
        let install = starfield_install(tmp.path());
        let logs = starfield_sfse(tmp.path()).join("Logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(
            logs.join("sfse.txt"),
            "plugin Foo.dll (00000001 Foo 00000001) disabled, requires newer script extender 0 (handle 1)\n",
        )
        .unwrap();
        let enabled = vec!["Foo".to_string()];
        let deployed = Vec::new();
        let diagnosis = diagnose(&starfield_input(&install, &enabled, &deployed));
        assert!(
            diagnosis.summary.contains("Foo failed to load"),
            "{}",
            diagnosis.summary
        );
        assert!(diagnosis.findings.iter().any(|finding| {
            finding.related_mod.as_deref() == Some("Foo")
                && finding
                    .detail
                    .contains("disabled, requires newer script extender")
        }));
    }

    #[test]
    fn plugin_log_names_the_mod_without_replacing_the_extender_excerpt() {
        let tmp = tempfile::tempdir().unwrap();
        let install = starfield_install(tmp.path());
        let logs = starfield_sfse(tmp.path()).join("Logs");
        fs::create_dir_all(&logs).unwrap();
        fs::write(&logs.join("sfse.txt"), "init complete\n").unwrap();
        fs::write(
            logs.join("BetterShips.log"),
            "Failed to load BetterShips.dll\n",
        )
        .unwrap();
        let enabled = vec!["Better Ships".to_string()];
        let deployed = Vec::new();
        let diagnosis = diagnose(&starfield_input(&install, &enabled, &deployed));
        assert!(
            diagnosis.excerpt_path.ends_with("sfse.txt"),
            "{}",
            diagnosis.excerpt_path
        );
        assert!(
            !diagnosis.excerpt.contains("BetterShips"),
            "{}",
            diagnosis.excerpt
        );
        assert!(diagnosis.findings.iter().any(|finding| {
            finding.related_mod.as_deref() == Some("Better Ships")
                && finding.detail.contains("BetterShips.dll")
        }));
    }

    #[test]
    fn crash_dump_without_a_text_log_names_crash_logger() {
        let tmp = tempfile::tempdir().unwrap();
        let install = starfield_install(tmp.path());
        let sfse = starfield_sfse(tmp.path());
        let logs = sfse.join("Logs");
        let crashes = sfse.join("Crashlogs");
        fs::create_dir_all(&logs).unwrap();
        fs::create_dir_all(&crashes).unwrap();
        fs::write(&logs.join("sfse.txt"), "init complete\n").unwrap();
        fs::write(crashes.join("Starfield.exe.dmp"), b"MDMP").unwrap();
        let enabled = Vec::new();
        let deployed = Vec::new();
        let diagnosis = diagnose(&starfield_input(&install, &enabled, &deployed));
        assert!(
            diagnosis.summary.contains(".dmp") && diagnosis.summary.contains("Crash Logger"),
            "{}",
            diagnosis.summary
        );
        assert!(
            !diagnosis.summary.contains("does not write a crash stack"),
            "{}",
            diagnosis.summary
        );
    }

    #[test]
    fn older_papyrus_log_is_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let install = starfield_install(tmp.path());
        let sfse = starfield_sfse(tmp.path());
        let logs = sfse.join("Logs");
        fs::create_dir_all(&logs).unwrap();
        let sfse_log = logs.join("sfse.txt");
        fs::write(&sfse_log, "init complete\n").unwrap();
        let papyrus = sfse.parent().unwrap().join("Logs").join("Script");
        fs::create_dir_all(&papyrus).unwrap();
        let papyrus_log = papyrus.join("Papyrus.0.log");
        fs::write(
            &papyrus_log,
            "[10/20/2026] error: File \"OldMod.esp\" exploded\n",
        )
        .unwrap();
        let now = SystemTime::now();
        fs::File::options()
            .write(true)
            .open(&sfse_log)
            .unwrap()
            .set_modified(now)
            .unwrap();
        fs::File::options()
            .write(true)
            .open(&papyrus_log)
            .unwrap()
            .set_modified(now - Duration::from_secs(7200))
            .unwrap();
        let enabled = vec!["OldMod".to_string()];
        let deployed = Vec::new();
        let diagnosis = diagnose(&starfield_input(&install, &enabled, &deployed));
        assert!(
            diagnosis
                .findings
                .iter()
                .all(|finding| !finding.detail.contains("OldMod")),
            "{:?}",
            diagnosis.findings
        );
        assert!(
            !diagnosis.summary.contains("OldMod"),
            "{}",
            diagnosis.summary
        );
    }
}
