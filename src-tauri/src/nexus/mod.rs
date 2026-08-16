//! Nexus Mods REST v1 + GraphQL v2 client.

use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_RANGE, RANGE, USER_AGENT};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::config::{APP_NAME, APP_VERSION};

const REST_BASE: &str = "https://api.nexusmods.com/v1";
const GQL_URL: &str = "https://api.nexusmods.com/v2/graphql";

pub const CANCELLED_MSG: &str = "CANCELLED";
pub const PAUSED_MSG: &str = "PAUSED";

/// Cooperative cancel / pause flags for an in-flight HTTP transfer.
pub struct TransferControl {
    pub cancel: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
}

#[derive(Clone)]
pub struct NexusClient {
    http: reqwest::Client,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NexusUser {
    pub user_id: u64,
    pub key: String,
    pub name: String,
    pub is_premium: bool,
    pub is_supporter: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModSearchHit {
    pub mod_id: u64,
    pub name: String,
    pub summary: Option<String>,
    pub picture_url: Option<String>,
    pub downloads: Option<u64>,
    pub endorsements: Option<u64>,
    pub author: Option<String>,
    pub domain_name: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameCategory {
    pub category_id: u64,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameInfo {
    pub id: u64,
    pub name: String,
    pub domain_name: String,
    pub genre: Option<String>,
    pub forum_url: Option<String>,
    pub nexusmods_url: Option<String>,
    pub mods: Option<u64>,
    pub file_count: Option<u64>,
    pub downloads: Option<u64>,
    #[serde(default)]
    pub categories: Vec<GameCategory>,
}

/// Lightweight game row from `GET /v1/games.json` (catalog matching).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NexusGameEntry {
    pub id: u64,
    pub name: String,
    pub domain_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModDetail {
    pub mod_id: u64,
    pub name: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub picture_url: Option<String>,
    pub author: Option<String>,
    pub version: Option<String>,
    pub downloads: Option<u64>,
    pub endorsements: Option<u64>,
    pub created_timestamp: Option<u64>,
    pub updated_timestamp: Option<u64>,
    pub domain_name: String,
    #[serde(default)]
    pub category_id: Option<u64>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub uploaded_by: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub contains_adult_content: bool,
    #[serde(default)]
    pub uploaded_users_profile_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModFileInfo {
    pub file_id: u64,
    pub name: String,
    pub version: Option<String>,
    pub category_name: Option<String>,
    pub size_kb: Option<u64>,
    pub uploaded_timestamp: Option<u64>,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionHit {
    pub slug: String,
    pub name: String,
    pub summary: Option<String>,
    pub endorsements: Option<u64>,
    pub total_downloads: Option<u64>,
    pub domain_name: Option<String>,
    pub revision_number: Option<i64>,
    #[serde(default)]
    pub tile_image_url: Option<String>,
    pub author: Option<String>,
    pub category: Option<String>,
    pub overall_rating: Option<f64>,
    pub overall_rating_count: Option<u64>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub mod_count: Option<u64>,
    pub file_size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionDetail {
    pub slug: String,
    pub name: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub endorsements: Option<u64>,
    pub total_downloads: Option<u64>,
    pub domain_name: Option<String>,
    pub revision_number: Option<i64>,
    #[serde(default)]
    pub tile_image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionModFile {
    pub file_id: u64,
    pub optional: bool,
    pub mod_id: u64,
    pub mod_name: String,
    pub file_name: String,
    pub version: Option<String>,
    pub domain_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowseMeta {
    pub categories: Vec<String>,
    /// Searchable mod tags (`legacyTags` + mod facets).
    pub mod_tags: Vec<String>,
    /// Collection tags (`availableTags` / `specificTags` + collection facets).
    pub collection_tags: Vec<String>,
    pub game_versions: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct BrowseSearchOpts {
    pub sort: String,
    pub category: Option<String>,
    pub tags_include: Vec<String>,
    pub tags_exclude: Vec<String>,
    pub game_version: Option<String>,
    pub offset: u32,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModSearchPage {
    pub items: Vec<ModSearchHit>,
    pub nodes_count: u64,
    pub total_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionSearchPage {
    pub items: Vec<CollectionHit>,
    pub nodes_count: u64,
    pub total_count: u64,
}

const DEFAULT_MODS_PAGE_SIZE: u32 = 30;
const DEFAULT_COLLECTIONS_PAGE_SIZE: u32 = 25;

impl NexusClient {
    pub fn new(api_key: impl Into<String>) -> Result<Self> {
        let api_key = api_key.into();
        if api_key.trim().is_empty() {
            bail!("API key is empty");
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            "apikey",
            HeaderValue::from_str(api_key.trim()).context("invalid API key header")?,
        );
        headers.insert("Application-Name", HeaderValue::from_static(APP_NAME));
        headers.insert(
            "Application-Version",
            HeaderValue::from_str(APP_VERSION).unwrap_or(HeaderValue::from_static("0.1.0")),
        );
        headers.insert(
            USER_AGENT,
            HeaderValue::from_str(&format!("{APP_NAME}/{APP_VERSION}"))
                .unwrap_or(HeaderValue::from_static("emperor-mod-manager/0.1.0")),
        );
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;
        Ok(Self { http })
    }

    pub async fn validate(&self) -> Result<NexusUser> {
        let url = format!("{REST_BASE}/users/validate.json");
        let resp = self.http.get(&url).send().await?;
        self.check_rate_limit(&resp);
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("validate failed ({status}): {body}");
        }
        let user: NexusUser = resp.json().await?;
        Ok(user)
    }

    fn check_rate_limit(&self, resp: &reqwest::Response) {
        if let Some(remaining) = resp.headers().get("x-rl-hourly-remaining") {
            if let Ok(s) = remaining.to_str() {
                if let Ok(n) = s.parse::<i64>() {
                    if n < 10 {
                        log::warn!("Nexus hourly rate limit low: {n} remaining");
                    }
                }
            }
        }
    }

    pub async fn search_mods(
        &self,
        domain: &str,
        query: &str,
        adult_content: bool,
        opts: &BrowseSearchOpts,
    ) -> Result<ModSearchPage> {
        let sort_key = normalize_sort(&opts.sort);
        let count = if opts.count == 0 {
            DEFAULT_MODS_PAGE_SIZE
        } else {
            opts.count
        };
        let offset = opts.offset;

        // Unpaged REST trending shortcut only for the first empty trending page.
        if query.trim().is_empty()
            && sort_key == "trending"
            && !has_extra_filters(opts)
            && offset == 0
        {
            let items = self.trending_mods(domain).await?;
            let n = items.len() as u64;
            return Ok(ModSearchPage {
                items,
                nodes_count: n,
                total_count: n,
            });
        }

        let sort = mods_sort_json(&sort_key, !query.trim().is_empty());
        let filter = build_mods_filter(domain, query, adult_content, opts);

        let gql = r#"
        query SearchMods($filter: ModsFilter, $count: Int!, $offset: Int, $sort: [ModsSort!]) {
          mods(filter: $filter, count: $count, offset: $offset, sort: $sort) {
            nodesCount
            totalCount
            nodes {
              modId
              name
              summary
              pictureUrl
              downloads
              endorsements
              author
              category
              tags { name }
              game { domainName }
            }
          }
        }
        "#;

        let variables = json!({
            "filter": filter,
            "count": count,
            "offset": offset,
            "sort": [sort],
        });

        match self.graphql(gql, variables).await {
            Ok(data) => {
                let page = data.get("mods").cloned().unwrap_or(Value::Null);
                let nodes = page
                    .get("nodes")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let items: Vec<ModSearchHit> = nodes
                    .into_iter()
                    .filter_map(|n| parse_mod_hit(&n, domain))
                    .collect();
                let nodes_count = page
                    .get("nodesCount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(items.len() as u64);
                let total_count = page
                    .get("totalCount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(nodes_count);
                Ok(ModSearchPage {
                    items,
                    nodes_count,
                    total_count,
                })
            }
            Err(e) => {
                // REST fallback cannot honor offset / tag filters.
                if offset > 0 || has_extra_filters(opts) {
                    return Err(e);
                }
                log::warn!("GraphQL search failed, falling back to REST: {e}");
                let items = if query.trim().is_empty() {
                    self.trending_mods(domain).await?
                } else {
                    self.search_mods_rest(domain, query).await?
                };
                let n = items.len() as u64;
                Ok(ModSearchPage {
                    items,
                    nodes_count: n,
                    total_count: n,
                })
            }
        }
    }

    async fn search_mods_rest(&self, domain: &str, query: &str) -> Result<Vec<ModSearchHit>> {
        let url = format!("{REST_BASE}/games/{domain}/mods/search.json");
        let resp = self
            .http
            .get(&url)
            .query(&[("search", query), ("page", "1")])
            .send()
            .await?;
        self.check_rate_limit(&resp);
        if !resp.status().is_success() {
            let trending = self.trending_mods(domain).await?;
            let q = query.to_lowercase();
            return Ok(trending
                .into_iter()
                .filter(|m| m.name.to_lowercase().contains(&q))
                .collect());
        }
        let data: Value = resp.json().await?;
        let arr = data
            .as_array()
            .cloned()
            .or_else(|| data.get("results").and_then(|v| v.as_array()).cloned())
            .unwrap_or_default();
        Ok(arr
            .into_iter()
            .filter_map(|n| {
                Some(ModSearchHit {
                    mod_id: n.get("mod_id").or_else(|| n.get("id"))?.as_u64()?,
                    name: n.get("name")?.as_str()?.to_string(),
                    summary: n
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    picture_url: n
                        .get("picture_url")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    downloads: n.get("mod_downloads").and_then(|v| v.as_u64()),
                    endorsements: n.get("mod_endorsements").and_then(|v| v.as_u64()),
                    author: n.get("author").and_then(|v| v.as_str()).map(str::to_string),
                    domain_name: domain.to_string(),
                    category: None,
                    tags: Vec::new(),
                })
            })
            .collect())
    }

    pub async fn trending_mods(&self, domain: &str) -> Result<Vec<ModSearchHit>> {
        let url = format!("{REST_BASE}/games/{domain}/mods/trending.json");
        let resp = self.http.get(&url).send().await?;
        self.check_rate_limit(&resp);
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("trending failed ({status}): {body}");
        }
        let arr: Vec<Value> = resp.json().await?;
        Ok(arr
            .into_iter()
            .filter_map(|n| {
                Some(ModSearchHit {
                    mod_id: n.get("mod_id")?.as_u64()?,
                    name: n.get("name")?.as_str()?.to_string(),
                    summary: n
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    picture_url: n
                        .get("picture_url")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    downloads: None,
                    endorsements: n.get("endorsement_count").and_then(|v| v.as_u64()),
                    author: n.get("author").and_then(|v| v.as_str()).map(str::to_string),
                    domain_name: domain.to_string(),
                    category: None,
                    tags: Vec::new(),
                })
            })
            .collect())
    }

    pub async fn list_games(&self) -> Result<Vec<NexusGameEntry>> {
        let url = format!("{REST_BASE}/games.json");
        let resp = self.http.get(&url).send().await?;
        self.check_rate_limit(&resp);
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("list games failed ({status}): {body}");
        }
        let n: Value = resp.json().await?;
        let arr = n
            .as_array()
            .ok_or_else(|| anyhow!("games.json: expected array"))?;
        Ok(arr
            .iter()
            .filter_map(|g| {
                let domain_name = g.get("domain_name")?.as_str()?.to_string();
                let name = g
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(domain_name.as_str())
                    .to_string();
                let id = g.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                Some(NexusGameEntry {
                    id,
                    name,
                    domain_name,
                })
            })
            .collect())
    }

    pub async fn get_game(&self, domain: &str) -> Result<GameInfo> {
        let url = format!("{REST_BASE}/games/{domain}.json");
        let resp = self.http.get(&url).send().await?;
        self.check_rate_limit(&resp);
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("get game failed ({status}): {body}");
        }
        let n: Value = resp.json().await?;
        let domain_name = n
            .get("domain_name")
            .and_then(|v| v.as_str())
            .unwrap_or(domain)
            .to_string();
        let name = n
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(domain)
            .to_string();
        let id = n.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
        let categories = n
            .get("categories")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|c| {
                Some(GameCategory {
                    category_id: c.get("category_id")?.as_u64()?,
                    name: c.get("name")?.as_str()?.to_string(),
                })
            })
            .collect();
        Ok(GameInfo {
            id,
            name,
            domain_name,
            genre: n.get("genre").and_then(|v| v.as_str()).map(str::to_string),
            forum_url: n
                .get("forum_url")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            nexusmods_url: n
                .get("nexusmods_url")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            mods: n.get("mods").and_then(|v| v.as_u64()),
            file_count: n.get("file_count").and_then(|v| v.as_u64()),
            downloads: n.get("downloads").and_then(|v| v.as_u64()),
            categories,
        })
    }

    pub async fn get_mod(&self, domain: &str, mod_id: u64) -> Result<ModDetail> {
        let url = format!("{REST_BASE}/games/{domain}/mods/{mod_id}.json");
        let resp = self.http.get(&url).send().await?;
        self.check_rate_limit(&resp);
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("get mod failed ({status}): {body}");
        }
        let n: Value = resp.json().await?;
        let mod_id = n.get("mod_id").and_then(|v| v.as_u64()).unwrap_or(mod_id);
        let name = n
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("mod response missing name"))?
            .to_string();
        let domain_name = n
            .get("domain_name")
            .and_then(|v| v.as_str())
            .unwrap_or(domain)
            .to_string();
        let category_id = n.get("category_id").and_then(|v| v.as_u64());
        Ok(ModDetail {
            mod_id,
            name,
            summary: n
                .get("summary")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            description: n
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            picture_url: n
                .get("picture_url")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            author: n.get("author").and_then(|v| v.as_str()).map(str::to_string),
            version: n
                .get("version")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            downloads: n
                .get("mod_downloads")
                .and_then(|v| v.as_u64())
                .or_else(|| n.get("downloads").and_then(|v| v.as_u64())),
            endorsements: n.get("endorsement_count").and_then(|v| v.as_u64()),
            created_timestamp: n.get("created_timestamp").and_then(|v| v.as_u64()),
            updated_timestamp: n.get("updated_timestamp").and_then(|v| v.as_u64()),
            domain_name,
            category_id,
            category: None,
            uploaded_by: n
                .get("uploaded_by")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            status: n.get("status").and_then(|v| v.as_str()).map(str::to_string),
            contains_adult_content: n
                .get("contains_adult_content")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            uploaded_users_profile_url: n
                .get("uploaded_users_profile_url")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        })
    }

    pub async fn list_mod_files(&self, domain: &str, mod_id: u64) -> Result<Vec<ModFileInfo>> {
        let url = format!("{REST_BASE}/games/{domain}/mods/{mod_id}/files.json");
        let resp = self.http.get(&url).send().await?;
        self.check_rate_limit(&resp);
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("list files failed ({status}): {body}");
        }
        let data: Value = resp.json().await?;
        let files = data
            .get("files")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out: Vec<ModFileInfo> = files
            .into_iter()
            .filter_map(|n| {
                Some(ModFileInfo {
                    file_id: n.get("file_id")?.as_u64()?,
                    name: n.get("name")?.as_str()?.to_string(),
                    version: n
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    category_name: n
                        .get("category_name")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    size_kb: n.get("size_kb").and_then(|v| v.as_u64()),
                    uploaded_timestamp: n.get("uploaded_timestamp").and_then(|v| v.as_u64()),
                    is_primary: n
                        .get("is_primary")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                })
            })
            .collect();
        out.sort_by(|a, b| {
            b.uploaded_timestamp
                .cmp(&a.uploaded_timestamp)
                .then_with(|| b.file_id.cmp(&a.file_id))
        });
        Ok(out)
    }

    pub async fn download_file(
        &self,
        domain: &str,
        mod_id: u64,
        file_id: u64,
        dest: &Path,
        is_premium: bool,
        nxm_key: Option<&str>,
        nxm_expires: Option<u64>,
        start_offset: u64,
        control: Option<&TransferControl>,
        on_progress: Option<&Arc<dyn Fn(u64, Option<u64>, u64) + Send + Sync>>,
    ) -> Result<()> {
        let has_nxm = nxm_key.map(|k| !k.is_empty()).unwrap_or(false) && nxm_expires.is_some();
        if !is_premium && !has_nxm {
            bail!(
                "NEEDS_NXM: Free accounts need a site-issued download key. \
                 Use Download Assist (Mod Manager Download) or import a local archive. \
                 Premium unlocks one-click API downloads."
            );
        }
        let url =
            format!("{REST_BASE}/games/{domain}/mods/{mod_id}/files/{file_id}/download_link.json");
        let mut req = self.http.get(&url);
        if let (Some(key), Some(expires)) = (nxm_key, nxm_expires) {
            req = req.query(&[("key", key), ("expires", &expires.to_string())]);
        }
        let resp = req.send().await?;
        self.check_rate_limit(&resp);
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("download_link failed ({status}): {body}");
        }
        let links: Value = resp.json().await?;
        let download_url = links
            .as_array()
            .and_then(|a| a.first())
            .and_then(|v| v.get("URI"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("no download URI in response"))?
            .to_string();

        stream_url_to_file(
            &self.http,
            &download_url,
            dest,
            start_offset,
            None,
            control,
            on_progress,
        )
        .await
    }
}

/// Stream an HTTP URL to `dest`, optionally resuming with a Range request.
pub async fn stream_url_to_file(
    http: &reqwest::Client,
    download_url: &str,
    dest: &Path,
    start_offset: u64,
    extra_headers: Option<&HeaderMap>,
    control: Option<&TransferControl>,
    on_progress: Option<&Arc<dyn Fn(u64, Option<u64>, u64) + Send + Sync>>,
) -> Result<()> {
    if let Some(parent) = dest.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut offset = start_offset;
    let mut req = http.get(download_url);
    if let Some(headers) = extra_headers {
        req = req.headers(headers.clone());
    }
    if offset > 0 {
        req = req.header(RANGE, format!("bytes={offset}-"));
    }

    let resp = req.send().await?;
    let status = resp.status();

    if offset > 0 && status == StatusCode::OK {
        // Server ignored Range — restart from scratch.
        offset = 0;
    } else if offset > 0 && status == StatusCode::PARTIAL_CONTENT {
        // Resume from offset.
    } else if offset > 0
        && (status == StatusCode::FORBIDDEN
            || status == StatusCode::UNAUTHORIZED
            || status == StatusCode::GONE
            || status == StatusCode::NOT_FOUND)
    {
        let body = resp.text().await.unwrap_or_default();
        bail!("resume failed ({status}): link expired or unavailable. {body}");
    } else if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("download failed ({status}): {body}");
    }

    let content_len = resp.content_length();
    let total = if status == StatusCode::PARTIAL_CONTENT {
        parse_content_range_total(resp.headers().get(CONTENT_RANGE))
            .or_else(|| content_len.map(|n| n + offset))
    } else {
        content_len
    };

    let mut file = if offset > 0 {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(dest)
            .await?
    } else {
        tokio::fs::File::create(dest).await?
    };

    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = offset;
    let started = std::time::Instant::now();
    let mut last_report = started;
    // Bytes transferred this session (for speed).
    let session_start = offset;

    while let Some(chunk) = stream.next().await {
        if control.map(|c| c.cancel.load(Ordering::SeqCst)).unwrap_or(false) {
            drop(file);
            let _ = tokio::fs::remove_file(dest).await;
            bail!("{CANCELLED_MSG}");
        }
        if control.map(|c| c.pause.load(Ordering::SeqCst)).unwrap_or(false) {
            file.flush().await?;
            drop(file);
            if let Some(cb) = on_progress {
                cb(downloaded, total, 0);
            }
            bail!("{PAUSED_MSG}");
        }
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        let now = std::time::Instant::now();
        if now.duration_since(last_report).as_millis() >= 150
            || downloaded == total.unwrap_or(u64::MAX)
        {
            let elapsed = started.elapsed().as_secs_f64().max(0.001);
            let speed = ((downloaded - session_start) as f64 / elapsed) as u64;
            if let Some(cb) = on_progress {
                cb(downloaded, total, speed);
            }
            last_report = now;
        }
    }
    file.flush().await?;
    if let Some(cb) = on_progress {
        let elapsed = started.elapsed().as_secs_f64().max(0.001);
        let speed = ((downloaded - session_start) as f64 / elapsed) as u64;
        cb(downloaded, total.or(Some(downloaded)), speed);
    }
    Ok(())
}

fn parse_content_range_total(header: Option<&HeaderValue>) -> Option<u64> {
    let s = header?.to_str().ok()?;
    // e.g. "bytes 1000-1999/5000" or "bytes 1000-1999/*"
    let total = s.split('/').nth(1)?.trim();
    if total == "*" {
        return None;
    }
    total.parse().ok()
}

impl NexusClient {
    pub async fn search_collections(
        &self,
        domain: &str,
        query: &str,
        adult_content: bool,
        opts: &BrowseSearchOpts,
    ) -> Result<CollectionSearchPage> {
        let sort_key = normalize_sort(&opts.sort);
        let sort = collections_sort_json(&sort_key, !query.trim().is_empty());
        let filters = build_collections_filter(domain, query, adult_content, opts);
        let count = if opts.count == 0 {
            DEFAULT_COLLECTIONS_PAGE_SIZE
        } else {
            opts.count
        };
        let offset = opts.offset;

        let gql = r#"
        query SearchCollections($filters: CollectionsSearchFilter, $count: Int, $offset: Int, $sort: [CollectionsSearchSort!]) {
          collectionsV2(filter: $filters, count: $count, offset: $offset, sort: $sort) {
            nodesCount
            totalCount
            nodes {
              slug
              name
              summary
              endorsements
              totalDownloads
              overallRating
              overallRatingCount
              firstPublishedAt
              updatedAt
              category { name }
              user { name }
              game { domainName }
              latestPublishedRevision {
                revisionNumber
                modCount
                fileSize
                updatedAt
              }
              tileImage {
                url
                thumbnailUrl(size: med)
              }
            }
          }
        }
        "#;

        let data = self
            .graphql(
                gql,
                json!({
                    "filters": filters,
                    "count": count,
                    "offset": offset,
                    "sort": [sort],
                }),
            )
            .await?;

        let page = data.get("collectionsV2").cloned().unwrap_or(Value::Null);
        let nodes = page
            .get("nodes")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let items: Vec<CollectionHit> = nodes
            .into_iter()
            .filter_map(|n| {
                let tile = n.get("tileImage");
                let tile_image_url = tile
                    .and_then(|t| t.get("thumbnailUrl").and_then(|v| v.as_str()))
                    .or_else(|| tile.and_then(|t| t.get("url").and_then(|v| v.as_str())))
                    .map(str::to_string);
                let updated_at = n
                    .get("updatedAt")
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        n.pointer("/latestPublishedRevision/updatedAt")
                            .and_then(|v| v.as_str())
                    })
                    .map(str::to_string);
                let overall_rating = n
                    .get("overallRating")
                    .and_then(|v| v.as_f64())
                    .or_else(|| {
                        n.get("overallRating")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse().ok())
                    });
                Some(CollectionHit {
                    slug: n.get("slug")?.as_str()?.to_string(),
                    name: n.get("name")?.as_str()?.to_string(),
                    summary: n
                        .get("summary")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    endorsements: n.get("endorsements").and_then(|v| v.as_u64()),
                    total_downloads: n.get("totalDownloads").and_then(|v| v.as_u64()),
                    domain_name: n
                        .pointer("/game/domainName")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    revision_number: n
                        .pointer("/latestPublishedRevision/revisionNumber")
                        .and_then(|v| v.as_i64()),
                    tile_image_url,
                    author: n
                        .pointer("/user/name")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    category: n
                        .pointer("/category/name")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    overall_rating,
                    overall_rating_count: n.get("overallRatingCount").and_then(|v| v.as_u64()),
                    created_at: n
                        .get("firstPublishedAt")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    updated_at,
                    mod_count: n
                        .pointer("/latestPublishedRevision/modCount")
                        .and_then(|v| {
                            v.as_u64()
                                .or_else(|| v.as_i64().map(|i| i as u64))
                                .or_else(|| v.as_f64().map(|f| f as u64))
                        }),
                    file_size: n
                        .pointer("/latestPublishedRevision/fileSize")
                        .and_then(|v| {
                            v.as_u64()
                                .or_else(|| v.as_i64().map(|i| i as u64))
                                .or_else(|| v.as_f64().map(|f| f as u64))
                        }),
                })
            })
            .collect();

        let nodes_count = page
            .get("nodesCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(items.len() as u64);
        let total_count = page
            .get("totalCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(nodes_count);

        Ok(CollectionSearchPage {
            items,
            nodes_count,
            total_count,
        })
    }

    pub async fn browse_meta(&self, domain: &str, adult_content: bool) -> Result<BrowseMeta> {
        let game_gql = r#"
        query GameMeta($domainName: String!) {
          game(domainName: $domainName) {
            id
            availableTags { name }
            specificTags { name }
          }
        }
        "#;
        let game_data = self
            .graphql(game_gql, json!({ "domainName": domain }))
            .await?;
        let game = game_data.get("game").cloned().unwrap_or(Value::Null);
        let game_id = game.get("id").and_then(|v| v.as_i64()).or_else(|| {
            game.get("id")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
        });

        let mut collection_tags: Vec<String> = Vec::new();
        for key in ["availableTags", "specificTags"] {
            if let Some(arr) = game.get(key).and_then(|v| v.as_array()) {
                for t in arr {
                    if let Some(name) = t.get("name").and_then(|v| v.as_str()) {
                        push_unique(&mut collection_tags, name);
                    }
                }
            }
        }

        let mut mod_tags: Vec<String> = Vec::new();
        if let Some(gid) = game_id {
            let legacy_gql = r#"
            query LegacyTags($gameId: ID, $excludeAdult: Boolean) {
              legacyTags(gameId: $gameId, excludeAdult: $excludeAdult) {
                name
                searchable
              }
            }
            "#;
            let exclude_adult = if adult_content { Value::Null } else { json!(true) };
            match self
                .graphql(
                    legacy_gql,
                    json!({ "gameId": gid.to_string(), "excludeAdult": exclude_adult }),
                )
                .await
            {
                Ok(data) => {
                    if let Some(arr) = data.get("legacyTags").and_then(|v| v.as_array()) {
                        for t in arr {
                            let searchable = t
                                .get("searchable")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if !searchable {
                                continue;
                            }
                            if let Some(name) = t.get("name").and_then(|v| v.as_str()) {
                                push_unique(&mut mod_tags, name);
                            }
                        }
                    }
                }
                Err(e) => log::warn!("legacyTags query failed: {e}"),
            }
        }

        let mut categories = Vec::new();
        if let Some(gid) = game_id {
            let cat_gql = r#"
            query Cats($gameId: Int) {
              categories(gameId: $gameId, global: false) {
                name
              }
            }
            "#;
            match self.graphql(cat_gql, json!({ "gameId": gid })).await {
                Ok(data) => {
                    if let Some(arr) = data.get("categories").and_then(|v| v.as_array()) {
                        for c in arr {
                            if let Some(name) = c.get("name").and_then(|v| v.as_str()) {
                                push_unique(&mut categories, name);
                            }
                        }
                    }
                }
                Err(e) => log::warn!("categories query failed: {e}"),
            }
        }

        let mut game_versions = Vec::new();

        let collection_facet_gql = r#"
        query CollectionFacets($filters: CollectionsSearchFilter, $count: Int) {
          collectionsV2(
            filter: $filters
            count: $count
            facets: { gameVersion: [], tag: [], categoryName: [] }
          ) {
            facetsData
            nodesFacets {
              facet
              value
              count
            }
          }
        }
        "#;
        let mut collection_filters = json!({
            "gameDomain": [{ "value": domain, "op": "EQUALS" }],
        });
        apply_adult_content_filter(&mut collection_filters, adult_content, false);
        match self
            .graphql(
                collection_facet_gql,
                json!({ "filters": collection_filters, "count": 1 }),
            )
            .await
        {
            Ok(data) => {
                merge_facet_bucket(
                    &data,
                    "/collectionsV2/nodesFacets",
                    "/collectionsV2/facetsData",
                    &["gameVersion", "game_version"],
                    &mut game_versions,
                );
                merge_facet_bucket(
                    &data,
                    "/collectionsV2/nodesFacets",
                    "/collectionsV2/facetsData",
                    &["tag"],
                    &mut collection_tags,
                );
                merge_facet_bucket(
                    &data,
                    "/collectionsV2/nodesFacets",
                    "/collectionsV2/facetsData",
                    &["categoryName", "category_name"],
                    &mut categories,
                );
            }
            Err(e) => log::warn!("collection facets failed: {e}"),
        }

        let mods_facet_gql = r#"
        query ModFacets($filter: ModsFilter, $count: Int!) {
          mods(
            filter: $filter
            count: $count
            facets: { tag: [], categoryName: [] }
          ) {
            facetsData
            nodesFacets {
              facet
              value
              count
            }
          }
        }
        "#;
        let mut mods_filter = json!({
            "gameDomainName": [{ "value": domain, "op": "EQUALS" }],
        });
        apply_adult_content_filter(&mut mods_filter, adult_content, true);
        match self
            .graphql(
                mods_facet_gql,
                json!({ "filter": mods_filter, "count": 1 }),
            )
            .await
        {
            Ok(data) => {
                merge_facet_bucket(
                    &data,
                    "/mods/nodesFacets",
                    "/mods/facetsData",
                    &["tag"],
                    &mut mod_tags,
                );
                merge_facet_bucket(
                    &data,
                    "/mods/nodesFacets",
                    "/mods/facetsData",
                    &["categoryName", "category_name"],
                    &mut categories,
                );
            }
            Err(e) => log::warn!("mod facets failed: {e}"),
        }

        // Version-like collection tags as fallback for game-version filter
        if game_versions.is_empty() {
            for t in &collection_tags {
                if looks_like_version(t) {
                    push_unique(&mut game_versions, t);
                }
            }
        }

        mod_tags.sort_by_key(|s| s.to_lowercase());
        collection_tags.sort_by_key(|s| s.to_lowercase());
        categories.sort_by_key(|s| s.to_lowercase());
        game_versions.sort_by_key(|s| s.to_lowercase());

        Ok(BrowseMeta {
            categories,
            mod_tags,
            collection_tags,
            game_versions,
        })
    }

    pub async fn get_collection(
        &self,
        slug: &str,
        domain: Option<&str>,
        adult_content: bool,
    ) -> Result<CollectionDetail> {
        let gql = r#"
        query CollectionDetail($slug: String!, $viewAdultContent: Boolean, $domainName: String) {
          collection(slug: $slug, viewAdultContent: $viewAdultContent, domainName: $domainName) {
            slug
            name
            summary
            description
            endorsements
            totalDownloads
            game { domainName }
            latestPublishedRevision { revisionNumber }
            tileImage {
              url
              thumbnailUrl(size: med)
            }
          }
        }
        "#;

        let data = self
            .graphql(
                gql,
                json!({
                    "slug": slug,
                    "viewAdultContent": adult_content,
                    "domainName": domain,
                }),
            )
            .await?;

        let n = data
            .get("collection")
            .cloned()
            .ok_or_else(|| anyhow!("collection detail missing collection node"))?;
        if n.is_null() {
            bail!("collection not found: {slug}");
        }

        let tile = n.get("tileImage");
        let tile_image_url = tile
            .and_then(|t| t.get("thumbnailUrl").and_then(|v| v.as_str()))
            .or_else(|| tile.and_then(|t| t.get("url").and_then(|v| v.as_str())))
            .map(str::to_string);

        Ok(CollectionDetail {
            slug: n
                .get("slug")
                .and_then(|v| v.as_str())
                .unwrap_or(slug)
                .to_string(),
            name: n
                .get("name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("collection missing name"))?
                .to_string(),
            summary: n
                .get("summary")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            description: n
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            endorsements: n.get("endorsements").and_then(|v| v.as_u64()),
            total_downloads: n.get("totalDownloads").and_then(|v| v.as_u64()),
            domain_name: n
                .pointer("/game/domainName")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            revision_number: n
                .pointer("/latestPublishedRevision/revisionNumber")
                .and_then(|v| v.as_i64()),
            tile_image_url,
        })
    }

    pub async fn collection_mod_files(
        &self,
        slug: &str,
        revision: Option<i64>,
        adult_content: bool,
    ) -> Result<Vec<CollectionModFile>> {
        let gql = r#"
        query CollectionRevisionMods($revision: Int, $slug: String!, $viewAdultContent: Boolean) {
          collectionRevision(revision: $revision, slug: $slug, viewAdultContent: $viewAdultContent) {
            modFiles {
              fileId
              optional
              file {
                fileId
                name
                version
                mod {
                  modId
                  name
                  game { domainName }
                }
              }
            }
          }
        }
        "#;

        let data = self
            .graphql(
                gql,
                json!({
                    "slug": slug,
                    "revision": revision,
                    "viewAdultContent": adult_content,
                }),
            )
            .await?;

        let files = data
            .pointer("/collectionRevision/modFiles")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(files
            .into_iter()
            .filter_map(|n| {
                let file = n.get("file")?;
                let mod_obj = file.get("mod")?;
                Some(CollectionModFile {
                    file_id: file.get("fileId")?.as_u64()?,
                    optional: n.get("optional").and_then(|v| v.as_bool()).unwrap_or(false),
                    mod_id: mod_obj.get("modId")?.as_u64()?,
                    mod_name: mod_obj.get("name")?.as_str()?.to_string(),
                    file_name: file.get("name")?.as_str()?.to_string(),
                    version: file
                        .get("version")
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    domain_name: mod_obj
                        .pointer("/game/domainName")
                        .and_then(|v| v.as_str())?
                        .to_string(),
                })
            })
            .collect())
    }

    async fn graphql(&self, query: &str, variables: Value) -> Result<Value> {
        let resp = self
            .http
            .post(GQL_URL)
            .json(&json!({ "query": query, "variables": variables }))
            .send()
            .await?;
        self.check_rate_limit(&resp);
        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            bail!("GraphQL HTTP {status}: {body}");
        }
        if let Some(errors) = body.get("errors") {
            bail!("GraphQL errors: {errors}");
        }
        body.get("data")
            .cloned()
            .ok_or_else(|| anyhow!("GraphQL response missing data"))
    }
}

fn normalize_sort(sort: &str) -> String {
    let s = sort.trim().to_ascii_lowercase();
    match s.as_str() {
        "downloads" | "most_downloaded" | "most-downloaded" => "downloads".into(),
        "newest" | "created" | "created_at" | "createdat" => "createdAt".into(),
        "updated" | "recently_updated" | "updated_at" | "updatedat" => "updatedAt".into(),
        "relevance" | "best_match" | "best-match" => "relevance".into(),
        "rating" | "highest_rated" | "highest-rated" => "rating".into(),
        "trending" => "trending".into(),
        _ => "endorsements".into(),
    }
}

fn has_extra_filters(opts: &BrowseSearchOpts) -> bool {
    opts.category.as_ref().is_some_and(|s| !s.is_empty())
        || !opts.tags_include.is_empty()
        || !opts.tags_exclude.is_empty()
        || opts.game_version.as_ref().is_some_and(|s| !s.is_empty())
}

fn mods_sort_json(sort_key: &str, has_query: bool) -> Value {
    let key = if sort_key == "relevance" && !has_query {
        "endorsements"
    } else if sort_key == "trending" {
        "endorsements"
    } else if sort_key == "rating" {
        "endorsements"
    } else {
        sort_key
    };
    json!({ key: { "direction": "DESC" } })
}

fn collections_sort_json(sort_key: &str, has_query: bool) -> Value {
    let key = if sort_key == "relevance" && !has_query {
        "endorsements"
    } else if sort_key == "trending" {
        "endorsements"
    } else {
        sort_key
    };
    json!({ key: { "direction": "DESC" } })
}

/// Official Nexus clients only set adultContent when excluding adult results.
fn apply_adult_content_filter(filter: &mut Value, adult_content: bool, as_array: bool) {
    if adult_content {
        return;
    }
    let clause = json!({ "value": false, "op": "EQUALS" });
    filter["adultContent"] = if as_array {
        json!([clause])
    } else {
        clause
    };
}

fn tag_value(value: &str, op: &str) -> Value {
    json!({ "value": value, "op": op })
}

fn apply_tag_filters(filter: &mut Value, include: &[String], exclude: &[String]) {
    let includes: Vec<&String> = include.iter().filter(|t| !t.is_empty()).collect();
    let excludes: Vec<&String> = exclude.iter().filter(|t| !t.is_empty()).collect();
    let total = includes.len() + excludes.len();
    if total == 0 {
        return;
    }
    if total == 1 {
        if let Some(t) = includes.first() {
            filter["tag"] = json!([tag_value(t, "EQUALS")]);
        } else if let Some(t) = excludes.first() {
            filter["tag"] = json!([tag_value(t, "NOT_EQUALS")]);
        }
        return;
    }

    let mut nested: Vec<Value> = Vec::with_capacity(total);
    for t in includes {
        nested.push(json!({ "tag": [tag_value(t, "EQUALS")] }));
    }
    for t in excludes {
        nested.push(json!({ "tag": [tag_value(t, "NOT_EQUALS")] }));
    }
    filter["filter"] = Value::Array(nested);
    filter["op"] = json!("AND");
}

fn build_mods_filter(
    domain: &str,
    query: &str,
    adult_content: bool,
    opts: &BrowseSearchOpts,
) -> Value {
    let mut filter = json!({
        "gameDomainName": [{ "value": domain, "op": "EQUALS" }],
    });
    apply_adult_content_filter(&mut filter, adult_content, true);
    if !query.trim().is_empty() {
        // WILDCARD already applies leading/trailing wildcards; do not wrap with '*'.
        filter["name"] = json!([{ "value": query.trim(), "op": "WILDCARD" }]);
    }
    if let Some(cat) = opts.category.as_ref().filter(|s| !s.is_empty()) {
        filter["categoryName"] = json!([{ "value": cat, "op": "EQUALS" }]);
    }
    // ModsFilter has no gameVersion field; do not stuff versions into tag.
    apply_tag_filters(&mut filter, &opts.tags_include, &opts.tags_exclude);
    filter
}

fn build_collections_filter(
    domain: &str,
    query: &str,
    adult_content: bool,
    opts: &BrowseSearchOpts,
) -> Value {
    let mut filter = json!({
        "gameDomain": [{ "value": domain, "op": "EQUALS" }],
    });
    apply_adult_content_filter(&mut filter, adult_content, true);
    if !query.trim().is_empty() {
        filter["generalSearch"] = json!([{ "value": query.trim(), "op": "WILDCARD" }]);
    }
    if let Some(cat) = opts.category.as_ref().filter(|s| !s.is_empty()) {
        filter["categoryName"] = json!([{ "value": cat, "op": "EQUALS" }]);
    }
    if let Some(ver) = opts.game_version.as_ref().filter(|s| !s.is_empty()) {
        filter["gameVersion"] = json!([{ "value": ver, "op": "EQUALS" }]);
    }
    apply_tag_filters(&mut filter, &opts.tags_include, &opts.tags_exclude);
    filter
}

fn parse_mod_hit(n: &Value, domain: &str) -> Option<ModSearchHit> {
    let tags = n
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.get("name").and_then(|v| v.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(ModSearchHit {
        mod_id: n.get("modId")?.as_u64()?,
        name: n.get("name")?.as_str()?.to_string(),
        summary: n
            .get("summary")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        picture_url: n
            .get("pictureUrl")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        downloads: n.get("downloads").and_then(|v| v.as_u64()),
        endorsements: n.get("endorsements").and_then(|v| v.as_u64()),
        author: n.get("author").and_then(|v| v.as_str()).map(str::to_string),
        domain_name: n
            .pointer("/game/domainName")
            .and_then(|v| v.as_str())
            .unwrap_or(domain)
            .to_string(),
        category: n.get("category").and_then(|v| {
            v.as_str()
                .map(str::to_string)
                .or_else(|| v.get("name").and_then(|x| x.as_str()).map(str::to_string))
        }),
        tags,
    })
}

fn push_unique(out: &mut Vec<String>, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    if !out.iter().any(|x| x.eq_ignore_ascii_case(value)) {
        out.push(value.to_string());
    }
}

fn facet_name_matches(name: &str, aliases: &[&str]) -> bool {
    aliases
        .iter()
        .any(|alias| name.eq_ignore_ascii_case(alias))
}

/// Collect values for the given facet names from nodesFacets and/or facetsData.
///
/// `nodesFacets` entries are flat `{ facet, value, count }`.
/// `facetsData` is `{"facetName":{"facetValue":count}}` (nested object map).
fn merge_facet_bucket(
    data: &Value,
    nodes_path: &str,
    facets_data_path: &str,
    facet_aliases: &[&str],
    out: &mut Vec<String>,
) {
    if let Some(facets) = data.pointer(nodes_path).and_then(|v| v.as_array()) {
        for facet in facets {
            let name = facet.get("facet").and_then(|v| v.as_str()).unwrap_or("");
            if !facet_name_matches(name, facet_aliases) {
                continue;
            }
            if let Some(val) = facet.get("value").and_then(|v| v.as_str()) {
                push_unique(out, val);
            }
        }
    }

    if let Some(obj) = data.pointer(facets_data_path).and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if !facet_name_matches(k, facet_aliases) {
                continue;
            }
            match v {
                Value::Object(map) => {
                    for value_key in map.keys() {
                        push_unique(out, value_key);
                    }
                }
                Value::Array(arr) => {
                    for item in arr {
                        let val = item
                            .as_str()
                            .or_else(|| item.get("value").and_then(|x| x.as_str()));
                        if let Some(val) = val {
                            push_unique(out, val);
                        }
                    }
                }
                Value::String(s) => push_unique(out, s),
                _ => {}
            }
        }
    }
}

fn looks_like_version(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() || trimmed.len() > 24 {
        return false;
    }
    let has_digit = trimmed.chars().any(|c| c.is_ascii_digit());
    let ok_chars = trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '));
    has_digit && ok_chars && trimmed.contains('.')
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NxmLink {
    pub domain: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub key: Option<String>,
    pub expires: Option<u64>,
}

/// Parse `nxm://domain/mods/123/files/456?key=...&expires=...`
pub fn parse_nxm(url_str: &str) -> Result<NxmLink> {
    let url_str = url_str.trim();
    if !url_str.starts_with("nxm://") {
        bail!("not an nxm:// URL");
    }
    // url crate needs a standard scheme for query parsing; rewrite temporarily.
    let httpish = format!("https://{}", url_str.trim_start_matches("nxm://"));
    let parsed = url::Url::parse(&httpish).context("invalid nxm URL")?;
    let mut parts: Vec<&str> = parsed.path().split('/').filter(|p| !p.is_empty()).collect();
    // host is the domain name (first segment of nxm path)
    let domain = parsed
        .host_str()
        .ok_or_else(|| anyhow!("nxm URL missing game domain"))?
        .to_string();
    // path is /mods/{id}/files/{id}
    if parts.len() < 4 || parts[0] != "mods" || parts[2] != "files" {
        // Some parsers keep domain in path if host empty — handle path-only form
        let rest = url_str.trim_start_matches("nxm://");
        let path = rest.split('?').next().unwrap_or(rest);
        parts = path.split('/').filter(|p| !p.is_empty()).collect();
        if parts.len() < 5 || parts[1] != "mods" || parts[3] != "files" {
            bail!("unrecognized nxm path: {path}");
        }
        let mut key = None;
        let mut expires = None;
        if let Some(q) = rest.split('?').nth(1) {
            for pair in q.split('&') {
                let mut kv = pair.splitn(2, '=');
                let k = kv.next().unwrap_or("");
                let v = kv.next().unwrap_or("");
                match k {
                    "key" => key = Some(v.to_string()),
                    "expires" => expires = v.parse().ok(),
                    _ => {}
                }
            }
        }
        return Ok(NxmLink {
            domain: parts[0].to_string(),
            mod_id: parts[2].parse()?,
            file_id: parts[4].parse()?,
            key,
            expires,
        });
    }

    let mut key = None;
    let mut expires = None;
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "key" => key = Some(v.to_string()),
            "expires" => expires = v.parse().ok(),
            _ => {}
        }
    }

    Ok(NxmLink {
        domain,
        mod_id: parts[1].parse()?,
        file_id: parts[3].parse()?,
        key,
        expires,
    })
}

pub fn store_api_key(key: &str) -> Result<()> {
    let entry = keyring::Entry::new(crate::config::KEYRING_SERVICE, crate::config::KEYRING_USER)?;
    entry.set_password(key.trim())?;
    Ok(())
}

pub fn load_api_key() -> Result<Option<String>> {
    let entry = keyring::Entry::new(crate::config::KEYRING_SERVICE, crate::config::KEYRING_USER)?;
    match entry.get_password() {
        Ok(k) if !k.is_empty() => Ok(Some(k)),
        Ok(_) => Ok(None),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => {
            // Fall back to config file for headless / missing secret service
            log::warn!("keyring read failed: {e}");
            Ok(None)
        }
    }
}

pub fn clear_api_key() -> Result<()> {
    let entry = keyring::Entry::new(crate::config::KEYRING_SERVICE, crate::config::KEYRING_USER)?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// File-based API key fallback when keyring is unavailable.
pub fn store_api_key_file(path: &Path, key: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, key.trim())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(path)?.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(path, perms)?;
    }
    Ok(())
}

pub fn load_api_key_file(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }
    let s = std::fs::read_to_string(path)?;
    let s = s.trim().to_string();
    if s.is_empty() {
        Ok(None)
    } else {
        Ok(Some(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nxm_url() {
        let link =
            parse_nxm("nxm://stardewvalley/mods/2400/files/12345?key=abc&expires=1&user_id=2")
                .unwrap();
        assert_eq!(link.domain, "stardewvalley");
        assert_eq!(link.mod_id, 2400);
        assert_eq!(link.file_id, 12345);
        assert_eq!(link.key.as_deref(), Some("abc"));
        assert_eq!(link.expires, Some(1));
    }

    fn base_opts() -> BrowseSearchOpts {
        BrowseSearchOpts {
            sort: "endorsements".into(),
            ..Default::default()
        }
    }

    #[test]
    fn mods_filter_omits_adult_when_included() {
        let filter = build_mods_filter("skyrim", "", true, &base_opts());
        assert!(filter.get("adultContent").is_none());
    }

    #[test]
    fn mods_filter_excludes_adult_when_disabled() {
        let filter = build_mods_filter("skyrim", "", false, &base_opts());
        assert_eq!(
            filter["adultContent"],
            json!([{ "value": false, "op": "EQUALS" }])
        );
    }

    #[test]
    fn mods_filter_query_uses_bare_wildcard_value() {
        let filter = build_mods_filter("skyrim", "SKSE", true, &base_opts());
        assert_eq!(
            filter["name"],
            json!([{ "value": "SKSE", "op": "WILDCARD" }])
        );
        let value = filter["name"][0]["value"].as_str().unwrap();
        assert!(!value.contains('*'), "WILDCARD must not wrap query with '*'");
    }

    #[test]
    fn mods_filter_single_include_tag() {
        let mut opts = base_opts();
        opts.tags_include = vec!["Performance".into()];
        let filter = build_mods_filter("skyrim", "", true, &opts);
        assert_eq!(
            filter["tag"],
            json!([{ "value": "Performance", "op": "EQUALS" }])
        );
        assert!(filter.get("filter").is_none());
    }

    #[test]
    fn mods_filter_single_exclude_tag() {
        let mut opts = base_opts();
        opts.tags_exclude = vec!["NSFW".into()];
        let filter = build_mods_filter("skyrim", "", true, &opts);
        assert_eq!(
            filter["tag"],
            json!([{ "value": "NSFW", "op": "NOT_EQUALS" }])
        );
    }

    #[test]
    fn mods_filter_multi_include_uses_and() {
        let mut opts = base_opts();
        opts.tags_include = vec!["A".into(), "B".into()];
        let filter = build_mods_filter("skyrim", "", true, &opts);
        assert_eq!(filter["op"], json!("AND"));
        assert_eq!(
            filter["filter"],
            json!([
                { "tag": [{ "value": "A", "op": "EQUALS" }] },
                { "tag": [{ "value": "B", "op": "EQUALS" }] },
            ])
        );
    }

    #[test]
    fn mods_filter_mixed_include_exclude() {
        let mut opts = base_opts();
        opts.tags_include = vec!["A".into()];
        opts.tags_exclude = vec!["B".into()];
        let filter = build_mods_filter("skyrim", "", true, &opts);
        assert_eq!(filter["op"], json!("AND"));
        assert_eq!(
            filter["filter"],
            json!([
                { "tag": [{ "value": "A", "op": "EQUALS" }] },
                { "tag": [{ "value": "B", "op": "NOT_EQUALS" }] },
            ])
        );
    }

    #[test]
    fn mods_filter_ignores_game_version() {
        let mut opts = base_opts();
        opts.game_version = Some("1.6.0".into());
        let filter = build_mods_filter("skyrim", "", true, &opts);
        assert!(filter.get("tag").is_none());
        assert!(filter.get("gameVersion").is_none());
    }

    #[test]
    fn collections_filter_arrays_and_version() {
        let mut opts = base_opts();
        opts.game_version = Some("1.6.0".into());
        opts.tags_include = vec!["Gameplay".into()];
        let filter = build_collections_filter("skyrim", "quest", false, &opts);
        assert_eq!(
            filter["gameDomain"],
            json!([{ "value": "skyrim", "op": "EQUALS" }])
        );
        assert_eq!(
            filter["adultContent"],
            json!([{ "value": false, "op": "EQUALS" }])
        );
        assert_eq!(
            filter["gameVersion"],
            json!([{ "value": "1.6.0", "op": "EQUALS" }])
        );
        assert_eq!(
            filter["tag"],
            json!([{ "value": "Gameplay", "op": "EQUALS" }])
        );
    }

    #[test]
    fn has_extra_filters_detects_include_and_exclude() {
        let mut opts = base_opts();
        assert!(!has_extra_filters(&opts));
        opts.tags_include = vec!["A".into()];
        assert!(has_extra_filters(&opts));
        opts.tags_include.clear();
        opts.tags_exclude = vec!["B".into()];
        assert!(has_extra_filters(&opts));
    }

    #[test]
    fn page_size_defaults() {
        assert_eq!(DEFAULT_MODS_PAGE_SIZE, 30);
        assert_eq!(DEFAULT_COLLECTIONS_PAGE_SIZE, 25);
    }
}
