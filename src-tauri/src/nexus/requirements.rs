//! Nexus GraphQL mod requirements: Nexus mods to queue, and off-site links.

use std::collections::{HashMap, HashSet, VecDeque};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{ModFileInfo, NexusClient};

const PAGE_SIZE: u64 = 50;
const MAX_REQUIREMENT_MODS: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NexusRequirementDownload {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub name: String,
    pub version: Option<String>,
    /// File chooser title or archive name, when it differs from the mod title.
    #[serde(default)]
    pub file_name: Option<String>,
    pub notes: Option<String>,
    pub url: String,
    /// Staged mod ids that need this requirement.
    pub required_by: Vec<String>,
    pub required_by_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalRequirementLink {
    pub name: String,
    pub url: String,
    pub notes: Option<String>,
    pub required_by: Vec<String>,
    pub required_by_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DlcRequirementNote {
    pub name: String,
    pub notes: Option<String>,
    pub required_by: Vec<String>,
    pub required_by_names: Vec<String>,
}

/// A staged mod that already satisfies a Nexus requirement. Enabled and disabled
/// both count, so the UI can warn when an unticked copy is still required.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedRequirementNote {
    pub staged_id: String,
    pub name: String,
    pub enabled: bool,
    pub required_by: Vec<String>,
    pub required_by_names: Vec<String>,
}

/// Staged Nexus mod already on disk for this game.
#[derive(Debug, Clone)]
pub struct StagedNexusMod {
    pub staged_id: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MissingRequirements {
    pub nexus: Vec<NexusRequirementDownload>,
    pub external: Vec<ExternalRequirementLink>,
    pub dlc: Vec<DlcRequirementNote>,
    /// Staged mods that match a Nexus requirement. Not offered as downloads.
    #[serde(default)]
    pub staged: Vec<StagedRequirementNote>,
    #[serde(default)]
    pub error: Option<String>,
}

/// A staged (or previewed) mod whose requirements should be resolved.
#[derive(Debug, Clone)]
pub struct RequirementRoot {
    pub staged_id: String,
    pub name: String,
    pub game_id: u64,
    pub mod_id: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequirementNode {
    pub external: bool,
    pub game_id: u64,
    pub mod_id: u64,
    pub name: String,
    pub notes: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequirementDlc {
    pub name: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequirementPage {
    pub nodes: Vec<RequirementNode>,
    pub total_count: u64,
    pub dlc: Vec<RequirementDlc>,
}

pub fn page_is_complete(fetched: usize, total: u64) -> bool {
    total == 0 || fetched as u64 >= total
}

fn json_u64(v: Option<&Value>) -> Option<u64> {
    let v = v?;
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

fn opt_text(v: Option<&Value>) -> Option<String> {
    v.and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// One GraphQL `mod.modRequirements` payload (`data` object, not the HTTP envelope).
pub fn parse_requirements_data(data: &Value) -> Result<RequirementPage> {
    let reqs = data
        .pointer("/mod/modRequirements")
        .ok_or_else(|| anyhow!("mod requirements missing"))?;
    let page = reqs.get("nexusRequirements").unwrap_or(&Value::Null);
    let nodes: Vec<RequirementNode> = page
        .get("nodes")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|n| {
                    Some(RequirementNode {
                        external: n
                            .get("externalRequirement")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        game_id: json_u64(n.get("gameId"))?,
                        mod_id: json_u64(n.get("modId")).unwrap_or(0),
                        name: opt_text(n.get("modName")).unwrap_or_else(|| "Requirement".into()),
                        notes: opt_text(n.get("notes")),
                        url: opt_text(n.get("url")).unwrap_or_default(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let total_count = json_u64(page.get("totalCount")).unwrap_or(nodes.len() as u64);
    let dlc = reqs
        .get("dlcRequirements")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|n| {
                    let name = opt_text(n.pointer("/gameExpansion/name"))?;
                    Some(RequirementDlc {
                        name,
                        notes: opt_text(n.get("notes")),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(RequirementPage {
        nodes,
        total_count,
        dlc,
    })
}

pub fn append_requirement_page(into: &mut RequirementPage, page: RequirementPage) {
    if into.nodes.is_empty() {
        into.dlc = page.dlc;
        into.total_count = page.total_count;
    }
    into.nodes.extend(page.nodes);
}

fn category_key(name: Option<&str>) -> String {
    name.unwrap_or("").trim().to_lowercase()
}

fn is_historical_file(file: &ModFileInfo) -> bool {
    matches!(
        category_key(file.category_name.as_deref()).as_str(),
        "old_version" | "old version" | "archived"
    )
}

/// Current primary file, else a Main-category file, else the first current file.
pub fn pick_main_mod_file(files: &[ModFileInfo]) -> Option<&ModFileInfo> {
    let current: Vec<&ModFileInfo> = files.iter().filter(|f| !is_historical_file(f)).collect();
    current
        .iter()
        .copied()
        .find(|f| f.is_primary)
        .or_else(|| {
            current
                .iter()
                .copied()
                .find(|f| category_key(f.category_name.as_deref()) == "main")
        })
        .or_else(|| current.first().copied())
}

fn nexus_page_url(domain: &str, mod_id: u64, given: &str) -> String {
    if !given.is_empty() {
        return given.to_string();
    }
    format!("https://www.nexusmods.com/{domain}/mods/{mod_id}")
}

fn push_unique(list: &mut Vec<String>, value: &str) {
    if value.is_empty() || list.iter().any(|s| s == value) {
        return;
    }
    list.push(value.to_string());
}

fn remember_root(required_by: &mut Vec<String>, names: &mut Vec<String>, root: &RequirementRoot) {
    push_unique(required_by, &root.staged_id);
    push_unique(names, &root.name);
}

fn external_key(node: &RequirementNode) -> String {
    if node.mod_id == 0 {
        format!("url:{}", node.url)
    } else {
        format!("{}:{}", node.game_id, node.mod_id)
    }
}

struct Listed {
    nexus: HashMap<(u64, u64), usize>,
    external: HashMap<String, usize>,
    dlc: HashMap<String, usize>,
    staged: HashMap<(u64, u64), usize>,
}

impl Listed {
    fn new() -> Self {
        Self {
            nexus: HashMap::new(),
            external: HashMap::new(),
            dlc: HashMap::new(),
            staged: HashMap::new(),
        }
    }
}

/// Dependency-first missing requirements for `roots`.
///
/// `pages` is keyed by `(game_id, mod_id)`. `files` is keyed by `(domain, mod_id)`.
/// `staged` is `(domain lowercase, mod_id)` already installed for this game.
/// Those mods are recorded on `staged` notes and are not offered as downloads.
pub fn assemble_missing(
    roots: &[RequirementRoot],
    pages: &HashMap<(u64, u64), RequirementPage>,
    files: &HashMap<(String, u64), Vec<ModFileInfo>>,
    domains: &HashMap<u64, String>,
    staged: &HashMap<(String, u64), StagedNexusMod>,
) -> MissingRequirements {
    let mut out = MissingRequirements::default();
    let mut listed = Listed::new();
    let mut emitted: HashSet<(u64, u64)> = HashSet::new();
    let mut visiting: HashSet<(u64, u64)> = HashSet::new();
    let mut linked: HashSet<(String, u64, u64)> = HashSet::new();

    for root in roots {
        if let Some(page) = pages.get(&(root.game_id, root.mod_id)) {
            for dlc in &page.dlc {
                add_dlc(&mut out, &mut listed, dlc, root);
            }
            for node in &page.nodes {
                walk_node(
                    node,
                    root,
                    pages,
                    files,
                    domains,
                    staged,
                    &mut out,
                    &mut listed,
                    &mut emitted,
                    &mut visiting,
                    &mut linked,
                );
            }
        }
    }
    out
}

fn add_dlc(
    out: &mut MissingRequirements,
    listed: &mut Listed,
    dlc: &RequirementDlc,
    root: &RequirementRoot,
) {
    let key = dlc.name.to_lowercase();
    if let Some(idx) = listed.dlc.get(&key).copied() {
        let row = &mut out.dlc[idx];
        remember_root(&mut row.required_by, &mut row.required_by_names, root);
        return;
    }
    let mut required_by = Vec::new();
    let mut required_by_names = Vec::new();
    remember_root(&mut required_by, &mut required_by_names, root);
    listed.dlc.insert(key, out.dlc.len());
    out.dlc.push(DlcRequirementNote {
        name: dlc.name.clone(),
        notes: dlc.notes.clone(),
        required_by,
        required_by_names,
    });
}

fn walk_node(
    node: &RequirementNode,
    root: &RequirementRoot,
    pages: &HashMap<(u64, u64), RequirementPage>,
    files: &HashMap<(String, u64), Vec<ModFileInfo>>,
    domains: &HashMap<u64, String>,
    staged: &HashMap<(String, u64), StagedNexusMod>,
    out: &mut MissingRequirements,
    listed: &mut Listed,
    emitted: &mut HashSet<(u64, u64)>,
    visiting: &mut HashSet<(u64, u64)>,
    linked: &mut HashSet<(String, u64, u64)>,
) {
    if node.external {
        let link = format!(
            "{}:{}:{}:{}",
            root.staged_id, node.game_id, node.mod_id, node.url
        );
        if !linked.insert((link, 0, 0)) {
            return;
        }
        add_external(out, listed, node, None, root);
        return;
    }
    let key = (node.game_id, node.mod_id);
    if !linked.insert((root.staged_id.clone(), node.game_id, node.mod_id)) {
        return;
    }
    if visiting.contains(&key) {
        return;
    }
    let first = emitted.insert(key);
    if first {
        visiting.insert(key);
        if let Some(page) = pages.get(&key) {
            for dlc in &page.dlc {
                add_dlc(out, listed, dlc, root);
            }
            for child in &page.nodes {
                walk_node(
                    child, root, pages, files, domains, staged, out, listed, emitted, visiting,
                    linked,
                );
            }
        }
        visiting.remove(&key);
        emit_nexus_or_link(out, listed, node, files, domains, staged, root);
    } else {
        attach_existing(out, listed, node, root);
        if let Some(page) = pages.get(&key) {
            for dlc in &page.dlc {
                add_dlc(out, listed, dlc, root);
            }
            for child in &page.nodes {
                walk_node(
                    child, root, pages, files, domains, staged, out, listed, emitted, visiting,
                    linked,
                );
            }
        }
    }
}

fn lookup_staged<'a>(
    domains: &HashMap<u64, String>,
    staged: &'a HashMap<(String, u64), StagedNexusMod>,
    game_id: u64,
    mod_id: u64,
) -> Option<&'a StagedNexusMod> {
    let domain = domains.get(&game_id)?;
    staged.get(&(domain.to_lowercase(), mod_id))
}

fn add_staged(
    out: &mut MissingRequirements,
    listed: &mut Listed,
    hit: &StagedNexusMod,
    game_id: u64,
    mod_id: u64,
    root: &RequirementRoot,
) {
    let key = (game_id, mod_id);
    if let Some(idx) = listed.staged.get(&key).copied() {
        let row = &mut out.staged[idx];
        remember_root(&mut row.required_by, &mut row.required_by_names, root);
        return;
    }
    let mut required_by = Vec::new();
    let mut required_by_names = Vec::new();
    remember_root(&mut required_by, &mut required_by_names, root);
    listed.staged.insert(key, out.staged.len());
    out.staged.push(StagedRequirementNote {
        staged_id: hit.staged_id.clone(),
        name: hit.name.clone(),
        enabled: hit.enabled,
        required_by,
        required_by_names,
    });
}

fn emit_nexus_or_link(
    out: &mut MissingRequirements,
    listed: &mut Listed,
    node: &RequirementNode,
    files: &HashMap<(String, u64), Vec<ModFileInfo>>,
    domains: &HashMap<u64, String>,
    staged: &HashMap<(String, u64), StagedNexusMod>,
    root: &RequirementRoot,
) {
    let Some(domain) = domains.get(&node.game_id).cloned() else {
        add_external(out, listed, node, None, root);
        return;
    };
    if let Some(hit) = lookup_staged(domains, staged, node.game_id, node.mod_id) {
        add_staged(out, listed, hit, node.game_id, node.mod_id, root);
        return;
    }
    let chosen = files
        .get(&(domain.clone(), node.mod_id))
        .and_then(|list| pick_main_mod_file(list));
    let Some(file) = chosen else {
        add_external(out, listed, node, Some(domain), root);
        return;
    };
    let key = (node.game_id, node.mod_id);
    if listed.nexus.contains_key(&key) {
        attach_existing(out, listed, node, root);
        return;
    }
    let mut required_by = Vec::new();
    let mut required_by_names = Vec::new();
    remember_root(&mut required_by, &mut required_by_names, root);
    listed.nexus.insert(key, out.nexus.len());
    out.nexus.push(NexusRequirementDownload {
        url: nexus_page_url(&domain, node.mod_id, &node.url),
        domain,
        mod_id: node.mod_id,
        file_id: file.file_id,
        name: node.name.clone(),
        version: file.version.clone(),
        file_name: crate::mods::pick_staged_file_name(
            &node.name,
            Some(&file.name),
            file.file_name.as_deref(),
        ),
        notes: node.notes.clone(),
        required_by,
        required_by_names,
    });
}

fn add_external(
    out: &mut MissingRequirements,
    listed: &mut Listed,
    node: &RequirementNode,
    domain: Option<String>,
    root: &RequirementRoot,
) {
    let key = external_key(node);
    if let Some(idx) = listed.external.get(&key).copied() {
        let row = &mut out.external[idx];
        remember_root(&mut row.required_by, &mut row.required_by_names, root);
        return;
    }
    if node.mod_id != 0 && listed.nexus.contains_key(&(node.game_id, node.mod_id)) {
        attach_existing(out, listed, node, root);
        return;
    }
    let url = match &domain {
        Some(domain) => nexus_page_url(domain, node.mod_id, &node.url),
        None => node.url.clone(),
    };
    let mut required_by = Vec::new();
    let mut required_by_names = Vec::new();
    remember_root(&mut required_by, &mut required_by_names, root);
    listed.external.insert(key, out.external.len());
    out.external.push(ExternalRequirementLink {
        name: node.name.clone(),
        url,
        notes: node.notes.clone(),
        required_by,
        required_by_names,
    });
}

fn locatable_mod(game_id: u64, mod_id: u64) -> bool {
    game_id != 0 && mod_id != 0
}

fn graphql_error_list(err: &str) -> Option<Vec<Value>> {
    let marker = "GraphQL errors:";
    let at = err.find(marker)?;
    let rest = err[at + marker.len()..].trim_start();
    let start = rest.find('[')?;
    let mut de = serde_json::Deserializer::from_str(&rest[start..]);
    let value = Value::deserialize(&mut de).ok()?;
    value.as_array().cloned()
}

/// Nexus could not locate this mod (`extensions.code` NOT_FOUND, message "Mod not found").
fn is_mod_not_found(err: &str) -> bool {
    graphql_error_list(err).is_some_and(|errors| {
        errors.iter().any(|error| {
            error.pointer("/extensions/code").and_then(|v| v.as_str()) == Some("NOT_FOUND")
                && error.get("message").and_then(|v| v.as_str()) == Some("Mod not found")
        })
    })
}

fn root_names_for<'a>(roots: &'a [RequirementRoot], game_id: u64, mod_id: u64) -> Vec<&'a str> {
    let mut names = Vec::new();
    for root in roots {
        if root.game_id == game_id && root.mod_id == mod_id && !names.contains(&root.name.as_str())
        {
            names.push(root.name.as_str());
        }
    }
    names
}

fn record_requirement_failure(
    pages: &mut HashMap<(u64, u64), RequirementPage>,
    errors: &mut Vec<String>,
    roots: &[RequirementRoot],
    game_id: u64,
    mod_id: u64,
    err: &str,
) {
    if is_mod_not_found(err) {
        pages.insert((game_id, mod_id), RequirementPage::default());
        let names = root_names_for(roots, game_id, mod_id);
        if !names.is_empty() {
            errors.push(format!(
                "Could not load requirements for {}. It was not found on Nexus.",
                names.join(", ")
            ));
        }
        return;
    }
    log::warn!("requirements for mod {mod_id}: {err}");
    errors.push(format!("Could not load requirements for mod {mod_id}."));
}

fn record_file_failure(errors: &mut Vec<String>, mod_id: u64, err: &str) {
    log::warn!("files for mod {mod_id}: {err}");
    errors.push(format!("Could not list files for mod {mod_id}."));
}

fn missing_error(errors: &[String]) -> Option<String> {
    if errors.is_empty() {
        None
    } else {
        Some(errors.join(" "))
    }
}

fn attach_existing(
    out: &mut MissingRequirements,
    listed: &Listed,
    node: &RequirementNode,
    root: &RequirementRoot,
) {
    let key = (node.game_id, node.mod_id);
    if let Some(idx) = listed.nexus.get(&key).copied() {
        let row = &mut out.nexus[idx];
        remember_root(&mut row.required_by, &mut row.required_by_names, root);
    }
    if let Some(idx) = listed.external.get(&external_key(node)).copied() {
        let row = &mut out.external[idx];
        remember_root(&mut row.required_by, &mut row.required_by_names, root);
    }
    if let Some(idx) = listed.staged.get(&(node.game_id, node.mod_id)).copied() {
        let row = &mut out.staged[idx];
        remember_root(&mut row.required_by, &mut row.required_by_names, root);
    }
}

impl NexusClient {
    async fn fetch_requirements_page(
        &self,
        game_id: u64,
        mod_id: u64,
        offset: u64,
    ) -> Result<RequirementPage> {
        let query = r#"
        query ModRequirements($modId: ID!, $gameId: ID!, $offset: Int, $count: Int) {
          mod(modId: $modId, gameId: $gameId) {
            modRequirements {
              dlcRequirements { notes gameExpansion { name } }
              nexusRequirements(count: $count, offset: $offset) {
                totalCount
                nodes { externalRequirement gameId modId modName notes url }
              }
            }
          }
        }
        "#;
        let data = self
            .graphql(
                query,
                json!({
                    "modId": mod_id.to_string(),
                    "gameId": game_id.to_string(),
                    "offset": offset,
                    "count": PAGE_SIZE,
                }),
            )
            .await
            .with_context(|| format!("requirements for mod {mod_id}"))?;
        parse_requirements_data(&data)
    }

    pub async fn mod_requirements(&self, game_id: u64, mod_id: u64) -> Result<RequirementPage> {
        let mut merged = RequirementPage::default();
        let mut offset = 0u64;
        loop {
            let page = self
                .fetch_requirements_page(game_id, mod_id, offset)
                .await?;
            let fetched = page.nodes.len() as u64;
            let total = page.total_count;
            append_requirement_page(&mut merged, page);
            if fetched == 0 || page_is_complete(merged.nodes.len(), total) {
                break;
            }
            offset += fetched;
        }
        Ok(merged)
    }
}

/// Fetch the requirement tree and drop anything already staged.
pub async fn collect_missing(
    client: &NexusClient,
    roots: &[RequirementRoot],
    domains: &HashMap<u64, String>,
    staged: &HashMap<(String, u64), StagedNexusMod>,
) -> MissingRequirements {
    let mut pages: HashMap<(u64, u64), RequirementPage> = HashMap::new();
    let mut errors = Vec::new();
    let mut queue: VecDeque<(u64, u64)> = VecDeque::new();
    for root in roots {
        if locatable_mod(root.game_id, root.mod_id) {
            queue.push_back((root.game_id, root.mod_id));
        }
    }
    let mut queued: HashSet<(u64, u64)> = HashSet::new();

    while let Some((game_id, mod_id)) = queue.pop_front() {
        if !queued.insert((game_id, mod_id)) {
            continue;
        }
        if queued.len() > MAX_REQUIREMENT_MODS {
            errors.push(format!(
                "Stopped after {MAX_REQUIREMENT_MODS} requirement mods."
            ));
            break;
        }
        match client.mod_requirements(game_id, mod_id).await {
            Ok(page) => {
                for node in &page.nodes {
                    if !node.external && locatable_mod(node.game_id, node.mod_id) {
                        queue.push_back((node.game_id, node.mod_id));
                    }
                }
                pages.insert((game_id, mod_id), page);
            }
            Err(e) => record_requirement_failure(
                &mut pages,
                &mut errors,
                roots,
                game_id,
                mod_id,
                &format!("{e:#}"),
            ),
        }
    }

    let mut files: HashMap<(String, u64), Vec<ModFileInfo>> = HashMap::new();
    let mut needed: Vec<(String, u64)> = Vec::new();
    for page in pages.values() {
        for node in &page.nodes {
            if node.external {
                continue;
            }
            let Some(domain) = domains.get(&node.game_id) else {
                continue;
            };
            if staged.contains_key(&(domain.to_lowercase(), node.mod_id)) {
                continue;
            }
            let key = (domain.clone(), node.mod_id);
            if needed.iter().any(|k| k == &key) {
                continue;
            }
            needed.push(key);
        }
    }
    for (domain, mod_id) in needed {
        match client.list_mod_files(&domain, mod_id).await {
            Ok(list) => {
                files.insert((domain, mod_id), list);
            }
            Err(e) => record_file_failure(&mut errors, mod_id, &format!("{e:#}")),
        }
    }

    let mut missing = assemble_missing(roots, &pages, &files, domains, staged);
    missing.error = missing_error(&errors);
    missing
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn file(id: u64, category: &str, primary: bool) -> ModFileInfo {
        ModFileInfo {
            file_id: id,
            name: format!("file {id}"),
            file_name: None,
            version: Some(format!("1.{id}")),
            category_name: Some(category.into()),
            size_kb: None,
            uploaded_timestamp: Some(id),
            is_primary: primary,
        }
    }

    fn node(external: bool, game: u64, id: u64, name: &str, url: &str) -> RequirementNode {
        RequirementNode {
            external,
            game_id: game,
            mod_id: id,
            name: name.into(),
            notes: None,
            url: url.into(),
        }
    }

    fn staged_hit(id: &str, name: &str, enabled: bool) -> StagedNexusMod {
        StagedNexusMod {
            staged_id: id.into(),
            name: name.into(),
            enabled,
        }
    }

    fn root(id: &str, name: &str, game: u64, mod_id: u64) -> RequirementRoot {
        RequirementRoot {
            staged_id: id.into(),
            name: name.into(),
            game_id: game,
            mod_id,
        }
    }

    #[test]
    fn parses_nexus_external_and_dlc() {
        let data = json!({
            "mod": {
                "modRequirements": {
                    "dlcRequirements": [
                        { "notes": "Needs the expansion", "gameExpansion": { "name": "Far Harbor" } }
                    ],
                    "nexusRequirements": {
                        "totalCount": 2,
                        "nodes": [
                            {
                                "externalRequirement": false,
                                "gameId": "110",
                                "modId": "42",
                                "modName": "Address Library",
                                "notes": null,
                                "url": "https://www.nexusmods.com/skyrimspecialedition/mods/42"
                            },
                            {
                                "externalRequirement": true,
                                "gameId": 110,
                                "modId": "0",
                                "modName": "SKSE",
                                "notes": "Install the matching build",
                                "url": "https://skse.silverlock.org/"
                            }
                        ]
                    }
                }
            }
        });
        let page = parse_requirements_data(&data).unwrap();
        assert_eq!(page.total_count, 2);
        assert_eq!(page.nodes.len(), 2);
        assert!(!page.nodes[0].external);
        assert_eq!(page.nodes[0].mod_id, 42);
        assert_eq!(page.nodes[0].game_id, 110);
        assert!(page.nodes[1].external);
        assert_eq!(page.nodes[1].url, "https://skse.silverlock.org/");
        assert_eq!(page.dlc[0].name, "Far Harbor");
        assert_eq!(page.dlc[0].notes.as_deref(), Some("Needs the expansion"));
    }

    #[test]
    fn pagination_appends_until_total_count() {
        let mut merged = RequirementPage::default();
        append_requirement_page(
            &mut merged,
            RequirementPage {
                nodes: vec![node(false, 1, 2, "A", "")],
                total_count: 2,
                dlc: vec![RequirementDlc {
                    name: "DLC".into(),
                    notes: None,
                }],
            },
        );
        assert!(!page_is_complete(merged.nodes.len(), merged.total_count));
        append_requirement_page(
            &mut merged,
            RequirementPage {
                nodes: vec![node(true, 1, 3, "B", "https://example.com")],
                total_count: 2,
                dlc: vec![],
            },
        );
        assert!(page_is_complete(merged.nodes.len(), merged.total_count));
        assert_eq!(merged.nodes.len(), 2);
        assert_eq!(merged.dlc.len(), 1);
        assert!(page_is_complete(0, 0));
    }

    #[test]
    fn picks_primary_then_main_then_first_current_file() {
        let old = file(1, "OLD_VERSION", false);
        let main = file(2, "MAIN", false);
        let primary = file(3, "OPTIONAL", true);
        assert_eq!(
            pick_main_mod_file(&[old.clone(), main.clone(), primary.clone()])
                .unwrap()
                .file_id,
            3
        );
        assert_eq!(
            pick_main_mod_file(&[old.clone(), main.clone()])
                .unwrap()
                .file_id,
            2
        );
        assert_eq!(
            pick_main_mod_file(&[old.clone(), file(4, "OPTIONAL", false)])
                .unwrap()
                .file_id,
            4
        );
        assert!(pick_main_mod_file(&[old]).is_none());
    }

    #[test]
    fn walks_children_first_and_skips_cycles_and_staged_mods() {
        let mut pages = HashMap::new();
        pages.insert(
            (1, 10),
            RequirementPage {
                nodes: vec![node(false, 1, 20, "Parent req", "")],
                total_count: 1,
                dlc: vec![RequirementDlc {
                    name: "Expansion".into(),
                    notes: None,
                }],
            },
        );
        pages.insert(
            (1, 20),
            RequirementPage {
                nodes: vec![
                    node(false, 1, 30, "Leaf", ""),
                    node(false, 1, 40, "Already installed", ""),
                ],
                total_count: 2,
                dlc: vec![],
            },
        );
        pages.insert(
            (1, 30),
            RequirementPage {
                nodes: vec![node(false, 1, 20, "Parent req", "")],
                total_count: 1,
                dlc: vec![],
            },
        );
        pages.insert((1, 40), RequirementPage::default());
        let mut files = HashMap::new();
        files.insert(("skyrim".into(), 20), vec![file(200, "MAIN", true)]);
        files.insert(("skyrim".into(), 30), vec![file(300, "MAIN", true)]);
        files.insert(("skyrim".into(), 40), vec![file(400, "MAIN", true)]);
        let mut domains = HashMap::new();
        domains.insert(1, "skyrim".into());
        let mut staged = HashMap::new();
        staged.insert(
            ("skyrim".into(), 40),
            staged_hit("40_1", "Already installed", true),
        );

        let missing = assemble_missing(
            &[root("10_1", "My Mod", 1, 10)],
            &pages,
            &files,
            &domains,
            &staged,
        );
        let ids: Vec<u64> = missing.nexus.iter().map(|n| n.mod_id).collect();
        assert_eq!(ids, vec![30, 20]);
        assert_eq!(missing.nexus[0].file_id, 300);
        assert_eq!(missing.nexus[0].required_by, vec!["10_1".to_string()]);
        assert!(missing.nexus.iter().all(|n| n.mod_id != 40));
        assert_eq!(missing.staged.len(), 1);
        assert_eq!(missing.staged[0].staged_id, "40_1");
        assert!(missing.staged[0].enabled);
        assert_eq!(missing.staged[0].required_by, vec!["10_1".to_string()]);
        assert_eq!(missing.dlc[0].name, "Expansion");
        assert!(missing.external.is_empty());
    }

    #[test]
    fn staged_requirements_are_noted_and_not_offered_as_downloads() {
        let mut pages = HashMap::new();
        pages.insert(
            (1, 10),
            RequirementPage {
                nodes: vec![
                    node(false, 1, 20, "Disabled req", ""),
                    node(false, 1, 30, "Enabled req", ""),
                ],
                total_count: 2,
                dlc: vec![],
            },
        );
        pages.insert((1, 20), RequirementPage::default());
        pages.insert((1, 30), RequirementPage::default());
        let mut files = HashMap::new();
        files.insert(("skyrim".into(), 20), vec![file(200, "MAIN", true)]);
        files.insert(("skyrim".into(), 30), vec![file(300, "MAIN", true)]);
        let mut domains = HashMap::new();
        domains.insert(1, "skyrim".into());
        let mut staged = HashMap::new();
        staged.insert(("skyrim".into(), 20), staged_hit("20_1", "SkyUI", false));
        staged.insert(
            ("skyrim".into(), 30),
            staged_hit("30_1", "Address Library", true),
        );
        let missing = assemble_missing(
            &[root("10_1", "First", 1, 10), root("11_1", "Second", 1, 10)],
            &pages,
            &files,
            &domains,
            &staged,
        );
        assert!(missing.nexus.is_empty());
        assert_eq!(missing.staged.len(), 2);
        let disabled = missing
            .staged
            .iter()
            .find(|n| n.staged_id == "20_1")
            .unwrap();
        assert!(!disabled.enabled);
        assert_eq!(disabled.name, "SkyUI");
        assert_eq!(
            disabled.required_by,
            vec!["10_1".to_string(), "11_1".to_string()]
        );
        assert_eq!(
            disabled.required_by_names,
            vec!["First".to_string(), "Second".to_string()]
        );
        let enabled = missing
            .staged
            .iter()
            .find(|n| n.staged_id == "30_1")
            .unwrap();
        assert!(enabled.enabled);
        assert_eq!(enabled.name, "Address Library");
    }

    #[test]
    fn a_second_staged_mod_is_recorded_on_a_shared_requirement() {
        let mut pages = HashMap::new();
        let shared = RequirementPage {
            nodes: vec![node(false, 1, 20, "Shared", "")],
            total_count: 1,
            dlc: vec![],
        };
        pages.insert((1, 10), shared.clone());
        pages.insert((1, 11), shared);
        pages.insert((1, 20), RequirementPage::default());
        let mut files = HashMap::new();
        files.insert(("skyrim".into(), 20), vec![file(200, "MAIN", false)]);
        let mut domains = HashMap::new();
        domains.insert(1, "skyrim".into());
        let missing = assemble_missing(
            &[root("10_1", "First", 1, 10), root("11_1", "Second", 1, 11)],
            &pages,
            &files,
            &domains,
            &HashMap::new(),
        );
        assert_eq!(missing.nexus.len(), 1);
        assert_eq!(
            missing.nexus[0].required_by,
            vec!["10_1".to_string(), "11_1".to_string()]
        );
        assert_eq!(
            missing.nexus[0].required_by_names,
            vec!["First".to_string(), "Second".to_string()]
        );
    }

    #[test]
    fn missing_file_or_unknown_game_becomes_a_link() {
        let mut pages = HashMap::new();
        pages.insert(
            (1, 10),
            RequirementPage {
                nodes: vec![
                    node(
                        false,
                        1,
                        20,
                        "No file",
                        "https://www.nexusmods.com/skyrim/mods/20",
                    ),
                    node(false, 9, 21, "Other game", "https://example.com/other"),
                    node(true, 1, 0, "SKSE", "https://skse.silverlock.org/"),
                ],
                total_count: 3,
                dlc: vec![],
            },
        );
        pages.insert((1, 20), RequirementPage::default());
        pages.insert((9, 21), RequirementPage::default());
        let mut domains = HashMap::new();
        domains.insert(1, "skyrim".into());
        let missing = assemble_missing(
            &[root("", "Preview", 1, 10)],
            &pages,
            &HashMap::new(),
            &domains,
            &HashMap::new(),
        );
        assert!(missing.nexus.is_empty());
        assert_eq!(missing.external.len(), 3);
        assert_eq!(missing.external[2].url, "https://skse.silverlock.org/");
        assert!(missing.external[0].required_by.is_empty());
        assert_eq!(
            missing.external[0].required_by_names,
            vec!["Preview".to_string()]
        );
    }

    const MOD_NOT_FOUND: &str = r#"Mod 9863: requirements for mod 9863: GraphQL errors: [{"message":"Mod not found","locations":[{"column":11,"line":3}],"path":["mod"],"extensions":{"code":"NOT_FOUND","detail":[{"attribute":"uid","code":"NOT_FOUND","entity":"Mod","message":"Mod not found","type":"LOCATE_ERROR","value":null}]}}]"#;

    #[test]
    fn nested_mod_not_found_keeps_the_requirement_and_hides_graphql() {
        let roots = [root("1", "Parent", 1704, 100)];
        let mut pages = HashMap::new();
        pages.insert(
            (1704, 100),
            RequirementPage {
                nodes: vec![node(
                    false,
                    1704,
                    9863,
                    "Missing child",
                    "https://www.nexusmods.com/skyrimspecialedition/mods/9863",
                )],
                total_count: 1,
                dlc: vec![],
            },
        );
        let mut errors = Vec::new();
        record_requirement_failure(&mut pages, &mut errors, &roots, 1704, 9863, MOD_NOT_FOUND);
        let mut files = HashMap::new();
        files.insert(
            ("skyrimspecialedition".into(), 9863),
            vec![file(1, "MAIN", true)],
        );
        let mut domains = HashMap::new();
        domains.insert(1704, "skyrimspecialedition".into());
        let mut missing = assemble_missing(&roots, &pages, &files, &domains, &HashMap::new());
        missing.error = missing_error(&errors);
        assert!(missing.error.is_none());
        assert_eq!(missing.nexus.len(), 1);
        assert_eq!(missing.nexus[0].mod_id, 9863);
        assert!(pages.get(&(1704, 9863)).unwrap().nodes.is_empty());
    }

    #[test]
    fn root_mod_not_found_is_a_short_sentence() {
        let roots = [root("1", "Parent", 1704, 9863)];
        let mut pages = HashMap::new();
        let mut errors = Vec::new();
        record_requirement_failure(&mut pages, &mut errors, &roots, 1704, 9863, MOD_NOT_FOUND);
        let message = missing_error(&errors).unwrap();
        assert_eq!(
            message,
            "Could not load requirements for Parent. It was not found on Nexus."
        );
        assert!(!message.contains("GraphQL"));
        assert!(!message.contains("NOT_FOUND"));
        assert!(!message.contains('{'));
        assert!(pages.contains_key(&(1704, 9863)));
    }

    #[test]
    fn other_requirement_and_file_failures_hide_raw_errors() {
        let mut pages = HashMap::new();
        let mut errors = Vec::new();
        let graphql = r#"requirements for mod 5: GraphQL errors: [{"message":"boom","extensions":{"code":"INTERNAL"}}]"#;
        record_requirement_failure(&mut pages, &mut errors, &[], 1, 5, graphql);
        record_file_failure(
            &mut errors,
            5,
            r#"list files failed (404): {"error":"nope"}"#,
        );
        let message = missing_error(&errors).unwrap();
        assert_eq!(
            message,
            "Could not load requirements for mod 5. Could not list files for mod 5."
        );
        assert!(!message.contains("GraphQL"));
        assert!(!message.contains("boom"));
        assert!(!message.contains("nope"));
        assert!(pages.is_empty());
    }

    #[test]
    fn zero_ids_are_not_locatable_mods() {
        assert!(!locatable_mod(0, 9863));
        assert!(!locatable_mod(1704, 0));
        assert!(locatable_mod(1704, 9863));
    }
}
