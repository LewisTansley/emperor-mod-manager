//! Thunderstore community package API client.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::atomic::Ordering;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::{Deserialize, Serialize};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::config::{APP_NAME, APP_VERSION};
use crate::nexus::{TransferControl, CANCELLED_MSG, PAUSED_MSG};
use walkdir::WalkDir;

const TS_BASE: &str = "https://thunderstore.io";

#[derive(Clone)]
pub struct ThunderstoreClient {
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TsPackageVersion {
    pub name: String,
    pub full_name: String,
    pub description: String,
    pub icon: String,
    pub version_number: String,
    pub dependencies: Vec<String>,
    pub download_url: String,
    pub downloads: u64,
    pub date_created: String,
    #[serde(default)]
    pub website_url: String,
    pub is_active: bool,
    pub uuid4: String,
    pub file_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TsPackage {
    pub name: String,
    pub full_name: String,
    pub owner: String,
    pub package_url: String,
    pub date_created: String,
    pub date_updated: String,
    pub uuid4: String,
    pub rating_score: i64,
    pub is_pinned: bool,
    pub is_deprecated: bool,
    pub has_nsfw_content: bool,
    pub categories: Vec<String>,
    pub versions: Vec<TsPackageVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TsCommunity {
    pub identifier: String,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct TsCommunityPage {
    pagination: TsCommunityPagination,
    results: Vec<TsCommunity>,
}

#[derive(Debug, Deserialize)]
struct TsCommunityPagination {
    next_link: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TsPackageDetail {
    pub community: String,
    pub namespace: String,
    pub name: String,
    pub full_name: String,
    pub package_url: String,
    pub uuid4: String,
    pub rating_score: i64,
    pub is_deprecated: bool,
    pub has_nsfw_content: bool,
    pub categories: Vec<String>,
    pub description: Option<String>,
    pub icon_url: Option<String>,
    pub downloads: u64,
    pub versions: Vec<TsPackageVersion>,
    pub latest_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct TsManifest {
    pub name: String,
    #[serde(default)]
    pub version_number: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
}

/// Parsed Thunderstore dependency string: `Namespace-Name-Version`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsDependency {
    pub namespace: String,
    pub name: String,
    pub version: String,
}

impl TsDependency {
    pub fn parse(raw: &str) -> Option<Self> {
        // Format is Owner-Name-X.Y.Z (name may contain hyphens).
        let parts: Vec<&str> = raw.split('-').collect();
        if parts.len() < 3 {
            return None;
        }
        // Prefer last segment with a '.' as the version start; else last segment.
        let ver_idx = parts
            .iter()
            .rposition(|p| p.contains('.'))
            .filter(|&i| i >= 2)
            .unwrap_or(parts.len() - 1);
        if ver_idx < 2 {
            return None;
        }
        let namespace = parts[0].to_string();
        let name = parts[1..ver_idx].join("-");
        let version = parts[ver_idx..].join("-");
        if namespace.is_empty() || name.is_empty() || version.is_empty() {
            return None;
        }
        Some(Self {
            namespace,
            name,
            version,
        })
    }

    pub fn key(&self) -> String {
        format!("{}-{}", self.namespace, self.name)
    }
}

impl ThunderstoreClient {
    pub fn new() -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&format!("{APP_NAME}/{APP_VERSION}"))?,
        );
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;
        Ok(Self { http })
    }

    pub async fn list_communities(&self) -> Result<Vec<TsCommunity>> {
        let mut url = format!("{TS_BASE}/api/experimental/community/");
        let mut out = Vec::new();
        loop {
            let resp = self
                .http
                .get(&url)
                .send()
                .await
                .with_context(|| format!("GET {url}"))?;
            if !resp.status().is_success() {
                bail!("Thunderstore list communities failed: {}", resp.status());
            }
            let page: TsCommunityPage = resp.json().await.context("decode community list")?;
            out.extend(page.results);
            match page.pagination.next_link {
                Some(next) if !next.is_empty() => url = next,
                _ => break,
            }
        }
        Ok(out)
    }

    pub async fn list_packages(&self, community: &str) -> Result<Vec<TsPackage>> {
        let url = format!("{TS_BASE}/c/{community}/api/v1/package/");
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            bail!("Thunderstore list packages failed: {}", resp.status());
        }
        let packages: Vec<TsPackage> = resp.json().await.context("decode package list")?;
        Ok(packages)
    }

    pub async fn get_package(
        &self,
        community: &str,
        namespace: &str,
        name: &str,
    ) -> Result<TsPackageDetail> {
        let packages = self.list_packages(community).await?;
        let pkg = packages
            .into_iter()
            .find(|p| {
                p.owner.eq_ignore_ascii_case(namespace) && p.name.eq_ignore_ascii_case(name)
            })
            .ok_or_else(|| anyhow!("Package {namespace}-{name} not found in {community}"))?;
        Ok(detail_from_package(community, pkg))
    }

    #[allow(dead_code)]
    pub async fn search_packages(
        &self,
        community: &str,
        query: &str,
        include_nsfw: bool,
        offset: usize,
        count: usize,
    ) -> Result<(Vec<TsPackageDetail>, usize)> {
        let mut packages = self.list_packages(community).await?;
        packages.retain(|p| !p.is_deprecated);
        if !include_nsfw {
            packages.retain(|p| !p.has_nsfw_content);
        }
        let q = query.trim().to_lowercase();
        if !q.is_empty() {
            packages.retain(|p| {
                p.name.to_lowercase().contains(&q)
                    || p.full_name.to_lowercase().contains(&q)
                    || p.owner.to_lowercase().contains(&q)
                    || p.versions
                        .first()
                        .map(|v| v.description.to_lowercase().contains(&q))
                        .unwrap_or(false)
                    || p.categories.iter().any(|c| c.to_lowercase().contains(&q))
            });
        }
        // Sort by latest version downloads desc, pinned first
        packages.sort_by(|a, b| {
            b.is_pinned
                .cmp(&a.is_pinned)
                .then_with(|| {
                    let da = a.versions.first().map(|v| v.downloads).unwrap_or(0);
                    let db = b.versions.first().map(|v| v.downloads).unwrap_or(0);
                    db.cmp(&da)
                })
                .then_with(|| b.rating_score.cmp(&a.rating_score))
        });
        let total = packages.len();
        let page: Vec<TsPackageDetail> = packages
            .into_iter()
            .skip(offset)
            .take(count)
            .map(|p| detail_from_package(community, p))
            .collect();
        Ok((page, total))
    }

    /// Resolve dependency closure (dependencies first). `root` is the leaf package.
    pub async fn resolve_install_order(
        &self,
        community: &str,
        namespace: &str,
        name: &str,
        version: Option<&str>,
    ) -> Result<Vec<TsPackageDetail>> {
        let packages = self.list_packages(community).await?;
        let by_key: HashMap<String, TsPackage> = packages
            .into_iter()
            .map(|p| (format!("{}-{}", p.owner, p.name).to_lowercase(), p))
            .collect();

        let root_key = format!("{namespace}-{name}").to_lowercase();
        let root_pkg = by_key
            .get(&root_key)
            .ok_or_else(|| anyhow!("Package {namespace}-{name} not found"))?;

        let mut ordered: Vec<TsPackageDetail> = Vec::new();
        let mut visiting: HashSet<String> = HashSet::new();
        let mut visited: HashSet<String> = HashSet::new();

        fn visit(
            key: &str,
            preferred_version: Option<&str>,
            by_key: &HashMap<String, TsPackage>,
            community: &str,
            visiting: &mut HashSet<String>,
            visited: &mut HashSet<String>,
            ordered: &mut Vec<TsPackageDetail>,
        ) -> Result<()> {
            let key_l = key.to_lowercase();
            if visited.contains(&key_l) {
                return Ok(());
            }
            if !visiting.insert(key_l.clone()) {
                bail!("Circular Thunderstore dependency involving {key}");
            }
            let pkg = by_key
                .get(&key_l)
                .ok_or_else(|| anyhow!("Missing dependency package {key}"))?;
            let ver = select_version(pkg, preferred_version)?;
            for dep_raw in &ver.dependencies {
                if let Some(dep) = TsDependency::parse(dep_raw) {
                    let dep_key = format!("{}-{}", dep.namespace, dep.name);
                    visit(
                        &dep_key,
                        Some(&dep.version),
                        by_key,
                        community,
                        visiting,
                        visited,
                        ordered,
                    )?;
                }
            }
            visiting.remove(&key_l);
            visited.insert(key_l);
            let mut detail = detail_from_package(community, pkg.clone());
            // Pin selected version as "latest" for download
            detail.latest_version = Some(ver.version_number.clone());
            if let Some(pos) = detail
                .versions
                .iter()
                .position(|v| v.version_number == ver.version_number)
            {
                let selected = detail.versions.remove(pos);
                detail.versions.insert(0, selected);
            }
            detail.description = Some(ver.description.clone());
            detail.icon_url = Some(ver.icon.clone());
            detail.downloads = ver.downloads;
            ordered.push(detail);
            Ok(())
        }

        visit(
            &root_key,
            version,
            &by_key,
            community,
            &mut visiting,
            &mut visited,
            &mut ordered,
        )?;

        // Ensure root is last (visit already does deps-first)
        let _ = root_pkg;
        Ok(ordered)
    }

    pub async fn download_version(
        &self,
        version: &TsPackageVersion,
        dest: &Path,
        control: Option<&TransferControl>,
    ) -> Result<()> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let resp = self
            .http
            .get(&version.download_url)
            .send()
            .await
            .with_context(|| format!("GET {}", version.download_url))?;
        if !resp.status().is_success() {
            bail!("Thunderstore download failed: {}", resp.status());
        }
        let mut stream = resp.bytes_stream();
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(dest)
            .await?;
        while let Some(chunk) = stream.next().await {
            if let Some(ctrl) = control {
                if ctrl.cancel.load(Ordering::Relaxed) {
                    bail!(CANCELLED_MSG);
                }
                if ctrl.pause.load(Ordering::Relaxed) {
                    bail!(PAUSED_MSG);
                }
            }
            let chunk = chunk?;
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        Ok(())
    }
}

fn select_version<'a>(
    pkg: &'a TsPackage,
    preferred: Option<&str>,
) -> Result<&'a TsPackageVersion> {
    if let Some(v) = preferred {
        if let Some(found) = pkg.versions.iter().find(|x| x.version_number == v) {
            return Ok(found);
        }
        // Prefer latest if exact version missing
    }
    pkg.versions
        .first()
        .ok_or_else(|| anyhow!("Package {} has no versions", pkg.full_name))
}

pub fn detail_from_package(community: &str, pkg: TsPackage) -> TsPackageDetail {
    let latest = pkg.versions.first();
    TsPackageDetail {
        community: community.to_string(),
        namespace: pkg.owner.clone(),
        name: pkg.name.clone(),
        full_name: pkg.full_name.clone(),
        package_url: pkg.package_url.clone(),
        uuid4: pkg.uuid4.clone(),
        rating_score: pkg.rating_score,
        is_deprecated: pkg.is_deprecated,
        has_nsfw_content: pkg.has_nsfw_content,
        categories: pkg.categories.clone(),
        description: latest.map(|v| v.description.clone()),
        icon_url: latest.map(|v| v.icon.clone()),
        downloads: latest.map(|v| v.downloads).unwrap_or(0),
        latest_version: latest.map(|v| v.version_number.clone()),
        versions: pkg.versions,
    }
}

pub fn is_modpack_category(categories: &[String]) -> bool {
    categories
        .iter()
        .any(|c| c.to_lowercase().contains("modpack"))
}

const PROFILE_DATA_PREFIX: &str = "#r2modman\n";

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileManifest {
    #[serde(rename = "profileName", alias = "name")]
    pub profile_name: String,
    #[serde(default)]
    pub mods: Vec<ProfileMod>,
    #[serde(default, alias = "game_slug")]
    pub community: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfileMod {
    pub name: String,
    #[serde(alias = "versionNumber")]
    pub version: ProfileVersion,
    #[serde(default = "profile_mod_enabled")]
    pub enabled: bool,
}

fn profile_mod_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ProfileVersion {
    Parts {
        major: u64,
        minor: u64,
        patch: u64,
    },
    String(String),
}

impl ProfileVersion {
    pub fn as_string(&self) -> String {
        match self {
            ProfileVersion::Parts {
                major,
                minor,
                patch,
            } => format!("{major}.{minor}.{patch}"),
            ProfileVersion::String(s) => s.clone(),
        }
    }
}

/// `Owner-PackageName` without a version suffix.
pub fn parse_package_ident(name: &str) -> Option<(String, String)> {
    let (ns, rest) = name.split_once('-')?;
    if ns.is_empty() || rest.is_empty() {
        return None;
    }
    Some((ns.to_string(), rest.to_string()))
}

pub fn community_slug_matches(profile: &str, community: &str) -> bool {
    let norm = |s: &str| {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect::<String>()
    };
    norm(profile) == norm(community)
}

impl ThunderstoreClient {
    pub async fn fetch_legacy_profile(&self, key: &str) -> Result<Vec<u8>> {
        let key = key.trim();
        let url = format!("{TS_BASE}/api/experimental/legacyprofile/get/{key}/");
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            bail!("profile code is expired or invalid");
        }
        if !resp.status().is_success() {
            bail!("Thunderstore profile fetch failed: {}", resp.status());
        }
        let text = resp.text().await.context("read profile payload")?;
        let encoded = text
            .strip_prefix(PROFILE_DATA_PREFIX)
            .or_else(|| text.trim_start().strip_prefix(PROFILE_DATA_PREFIX))
            .ok_or_else(|| anyhow!("invalid profile data"))?;
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(encoded.trim())
            .context("decode profile payload")
    }
}

pub fn extract_profile_zip(bytes: &[u8], dest: &Path) -> Result<ProfileManifest> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).context("open profile zip")?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let raw_name = file.name();
        let is_dir = raw_name.ends_with('/') || raw_name.ends_with('\\');
        let rel = match crate::mods::safe_archive_entry_path(raw_name.trim_end_matches(['/', '\\']))
        {
            Some(p) => p,
            None => continue,
        };
        let outpath = dest.join(rel);
        if is_dir {
            fs::create_dir_all(&outpath)?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
    }
    crate::mods::repair_backslash_entries(dest)?;
    let manifest_path = dest.join("export.r2x");
    let reader = fs::File::open(&manifest_path).context("missing export.r2x in profile")?;
    let manifest: ProfileManifest =
        serde_yaml::from_reader(reader).context("parse export.r2x")?;
    let _ = fs::remove_file(&manifest_path);
    Ok(manifest)
}

pub fn profile_has_overlay_files(root: &Path) -> bool {
    WalkDir::new(root).into_iter().flatten().any(|e| {
        e.file_type().is_file()
            && e.file_name()
                .to_str()
                .map(|n| n != "export.r2x")
                .unwrap_or(true)
    })
}

#[allow(dead_code)]
pub fn parse_manifest(staging_root: &Path) -> Option<TsManifest> {
    let path = staging_root.join("manifest.json");
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

/// BFS helper kept for tests / future incremental resolve.
#[allow(dead_code)]
pub fn dependency_keys(deps: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut q: VecDeque<&str> = deps.iter().map(|s| s.as_str()).collect();
    while let Some(raw) = q.pop_front() {
        if let Some(d) = TsDependency::parse(raw) {
            out.push(d.key());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_dependency() {
        let d = TsDependency::parse("BepInEx-BepInExPack-5.4.2100").unwrap();
        assert_eq!(d.namespace, "BepInEx");
        assert_eq!(d.name, "BepInExPack");
        assert_eq!(d.version, "5.4.2100");
    }

    #[test]
    fn parse_hyphenated_name() {
        let d = TsDependency::parse("Owner-Cool-Mod-1.2.3").unwrap();
        assert_eq!(d.namespace, "Owner");
        assert_eq!(d.name, "Cool-Mod");
        assert_eq!(d.version, "1.2.3");
    }

    #[test]
    fn parse_package_ident_owner_name() {
        let (ns, name) = parse_package_ident("bbepis-BepInExPack").unwrap();
        assert_eq!(ns, "bbepis");
        assert_eq!(name, "BepInExPack");
    }

    #[test]
    fn community_slug_ignores_hyphens() {
        assert!(community_slug_matches("lethalcompany", "lethal-company"));
        assert!(!community_slug_matches("valheim", "lethal-company"));
    }
}
