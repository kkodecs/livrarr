use std::time::Duration;

use axum::extract::{Query, State};
use axum::Json;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::state::AppState;
use crate::{
    ApiError, AuthContext, GrabApiRequest, GrabStatus, ReleaseResponse, ReleaseSearchResponse,
};

/// Maximum size for .torrent file downloads (10 MB).
const MAX_DOWNLOAD_BYTES: usize = 10 * 1024 * 1024;
/// Maximum size for an indexer XML response body (10 MB).
/// Protects against hostile or misconfigured indexers sending unbounded XML.
const MAX_RESPONSE_BODY_BYTES: usize = 10 * 1024 * 1024;
use livrarr_db::{
    CreateGrabDbRequest, CreateHistoryEventDbRequest, DownloadClientDb, GrabDb, HistoryDb,
};
use livrarr_domain::{EventType, Indexer};

#[derive(serde::Deserialize)]
pub struct SearchQuery {
    #[serde(rename = "workId")]
    pub work_id: Option<i64>,
    #[serde(default)]
    pub refresh: bool,
    /// If true, only return cached results — never hit indexers.
    #[serde(default, rename = "cacheOnly")]
    pub cache_only: bool,
}

/// GET /api/v1/release?workId=...  — searches all enabled Torznab indexers
pub async fn search(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(q): Query<SearchQuery>,
) -> Result<Json<ReleaseSearchResponse>, ApiError> {
    use livrarr_domain::services::{ReleaseService, SearchReleasesRequest};

    let work_id = match q.work_id {
        Some(id) => id,
        None => {
            return Ok(Json(ReleaseSearchResponse {
                results: vec![],
                warnings: vec![],
                cache_age_seconds: None,
            }))
        }
    };

    let svc_response = state
        .release_service
        .search(
            ctx.user.id,
            SearchReleasesRequest {
                work_id,
                refresh: q.refresh,
                cache_only: q.cache_only,
            },
        )
        .await?;

    let results = svc_response
        .results
        .into_iter()
        .map(|r| ReleaseResponse {
            title: r.title,
            indexer: r.indexer,
            size: r.size,
            guid: r.guid,
            download_url: r.download_url,
            seeders: r.seeders,
            leechers: r.leechers,
            publish_date: r.publish_date,
            protocol: r.protocol.to_string(),
            categories: r.categories,
        })
        .collect();

    let warnings = svc_response
        .warnings
        .into_iter()
        .map(|w| {
            let (indexer, error) = w
                .strip_prefix("indexer ")
                .and_then(|s| s.split_once(": "))
                .map(|(i, e)| (i.to_string(), e.to_string()))
                .unwrap_or_else(|| (String::new(), w));
            crate::SearchWarning { indexer, error }
        })
        .collect();

    Ok(Json(ReleaseSearchResponse {
        results,
        warnings,
        cache_age_seconds: svc_response.cache_age_seconds,
    }))
}

/// Clean a title for search: strip subtitle (after colon or parenthetical),
/// strip author prefix, remove leading "the ", replace non-word chars, remove accents.
/// Mirrors Readarr's `SplitBookTitle` + `GetQueryTitle`.
fn clean_search_term(title: &str, author: &str) -> String {
    let mut t = title.to_string();

    // Strip "Author: Title" prefix.
    let prefix = format!("{author}:");
    if t.starts_with(&prefix) {
        t = t[prefix.len()..].trim().to_string();
    }

    // Strip subtitle after colon or parenthetical (whichever comes first).
    let colon = t.find(':');
    let paren = t.find('(');
    let split_at = match (colon, paren) {
        (Some(c), Some(p)) => Some(c.min(p)),
        (Some(c), None) => Some(c),
        (None, Some(p)) => Some(p),
        _ => None,
    };
    if let Some(pos) = split_at {
        if pos > 0 {
            t = t[..pos].trim().to_string();
        }
    }

    // Strip leading "the ".
    if t.to_lowercase().starts_with("the ") {
        t = t[4..].to_string();
    }

    // Replace & with space, . with space.
    t = t.replace(" & ", " ").replace('.', " ");

    // Collapse whitespace.
    t = t.split_whitespace().collect::<Vec<_>>().join(" ");

    t
}

/// Search a single indexer with tiered fallback (mirrors Readarr strategy).
pub(crate) async fn search_indexer(
    http: &livrarr_http::HttpClient,
    indexer: &Indexer,
    title: &str,
    author: &str,
) -> Result<Vec<ReleaseResponse>, String> {
    let cats = indexer
        .categories
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let clean_title = clean_search_term(title, author);
    let mut all_results: Vec<ReleaseResponse> = Vec::new();
    let mut seen_guids: std::collections::HashSet<String> = std::collections::HashSet::new();

    let add_results = |items: Vec<ReleaseResponse>,
                       results: &mut Vec<ReleaseResponse>,
                       seen: &mut std::collections::HashSet<String>| {
        for item in items {
            if seen.insert(item.guid.clone()) {
                results.push(item);
            }
        }
    };

    // Tier 1: structured book search (if supported)
    if indexer.supports_book_search {
        // 1a: author + title
        let url = build_torznab_url(
            &indexer.url,
            &indexer.api_path,
            indexer.api_key.as_deref(),
            &[
                ("t", "book"),
                ("author", author),
                ("title", &clean_title),
                ("cat", &cats),
                ("limit", "500"),
                ("extended", "1"),
            ],
        );
        if let Ok(items) = fetch_and_parse(http, &url, &indexer.name).await {
            add_results(items, &mut all_results, &mut seen_guids);
        }

        // 1b: title only
        let url = build_torznab_url(
            &indexer.url,
            &indexer.api_path,
            indexer.api_key.as_deref(),
            &[
                ("t", "book"),
                ("title", &clean_title),
                ("cat", &cats),
                ("limit", "500"),
                ("extended", "1"),
            ],
        );
        if let Ok(items) = fetch_and_parse(http, &url, &indexer.name).await {
            add_results(items, &mut all_results, &mut seen_guids);
        }

        if !all_results.is_empty() {
            return Ok(all_results);
        }
    }

    // Tier 2: freetext — title + author
    let query = format!("{clean_title} {author}");
    let url = build_torznab_url(
        &indexer.url,
        &indexer.api_path,
        indexer.api_key.as_deref(),
        &[
            ("t", "search"),
            ("q", &query),
            ("cat", &cats),
            ("limit", "500"),
            ("extended", "1"),
        ],
    );
    if let Ok(items) = fetch_and_parse(http, &url, &indexer.name).await {
        add_results(items, &mut all_results, &mut seen_guids);
    }

    // Tier 2b: author + title (reversed)
    let query_rev = format!("{author} {clean_title}");
    let url = build_torznab_url(
        &indexer.url,
        &indexer.api_path,
        indexer.api_key.as_deref(),
        &[
            ("t", "search"),
            ("q", &query_rev),
            ("cat", &cats),
            ("limit", "500"),
            ("extended", "1"),
        ],
    );
    if let Ok(items) = fetch_and_parse(http, &url, &indexer.name).await {
        add_results(items, &mut all_results, &mut seen_guids);
    }

    if !all_results.is_empty() {
        return Ok(all_results);
    }

    // Tier 3: title only (last resort)
    let url = build_torznab_url(
        &indexer.url,
        &indexer.api_path,
        indexer.api_key.as_deref(),
        &[
            ("t", "search"),
            ("q", &clean_title),
            ("cat", &cats),
            ("limit", "500"),
            ("extended", "1"),
        ],
    );
    fetch_and_parse(http, &url, &indexer.name).await
}

pub(crate) fn build_torznab_url(
    base: &str,
    api_path: &str,
    api_key: Option<&str>,
    params: &[(&str, &str)],
) -> String {
    let base_with_path = format!("{base}{api_path}");
    let separator = if base_with_path.contains('?') {
        '&'
    } else {
        '?'
    };
    let mut url = format!("{base_with_path}{separator}");
    for (i, (k, v)) in params.iter().enumerate() {
        if i > 0 {
            url.push('&');
        }
        url.push_str(k);
        url.push('=');
        url.push_str(&urlencoding::encode(v));
    }
    if let Some(key) = api_key {
        if !key.is_empty() {
            url.push_str("&apikey=");
            url.push_str(&urlencoding::encode(key));
        }
    }
    url
}

/// Redact apikey from URL for logging.
fn redact_url(url: &str) -> String {
    let mut result = url.to_string();
    // Redact API key.
    if let Some(pos) = result.find("apikey=") {
        let end = result[pos..]
            .find('&')
            .map(|i| pos + i)
            .unwrap_or(result.len());
        result = format!("{}apikey=[REDACTED]{}", &result[..pos], &result[end..]);
    }
    // Redact search query (contains book title).
    if let Some(pos) = result.find("q=") {
        let end = result[pos..]
            .find('&')
            .map(|i| pos + i)
            .unwrap_or(result.len());
        result = format!("{}q=[REDACTED]{}", &result[..pos], &result[end..]);
    }
    result
}

pub(crate) async fn fetch_and_parse(
    http: &livrarr_http::HttpClient,
    url: &str,
    indexer_name: &str,
) -> Result<Vec<ReleaseResponse>, String> {
    tracing::debug!(indexer = %indexer_name, url = %redact_url(url), "searching indexer");

    let resp = http
        .get(url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    // Reject oversized responses up-front when the server advertises Content-Length.
    if let Some(cl) = resp.content_length() {
        if cl as usize > MAX_RESPONSE_BODY_BYTES {
            return Err(format!(
                "indexer response too large: {cl} bytes (max {MAX_RESPONSE_BODY_BYTES})"
            ));
        }
    }

    // Read body incrementally via `chunk()` so a server that lies about
    // Content-Length (or omits it) still cannot exhaust memory — abort as
    // soon as the cap is exceeded.
    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| e.without_url().to_string())?
    {
        if buf.len() + chunk.len() > MAX_RESPONSE_BODY_BYTES {
            return Err(format!(
                "indexer response exceeded {MAX_RESPONSE_BODY_BYTES} bytes"
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    let xml = String::from_utf8(buf).map_err(|e| format!("invalid UTF-8 in response: {e}"))?;
    parse_torznab_xml(&xml, indexer_name)
}

fn parse_torznab_xml(xml: &str, indexer_name: &str) -> Result<Vec<ReleaseResponse>, String> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut results = Vec::new();

    let mut in_item = false;
    let mut current_title = String::new();
    let mut current_guid = String::new();
    let mut current_download_url = String::new();
    let mut current_size: i64 = 0;
    let mut current_seeders: Option<i32> = None;
    let mut current_leechers: Option<i32> = None;
    let mut current_pub_date: Option<String> = None;
    let mut current_categories: Vec<i32> = Vec::new();
    let mut current_tag: Vec<u8> = Vec::new();
    let mut current_enclosure_type = String::new();

    loop {
        match reader.read_event() {
            Ok(ref event @ (Event::Start(_) | Event::Empty(_))) => {
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!(),
                };
                let local = e.local_name();
                let is_start = matches!(event, Event::Start(_));

                match local.as_ref() {
                    b"error" => {
                        let code = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.local_name().as_ref() == b"code")
                            .and_then(|a| a.unescape_value().ok()?.parse::<i32>().ok())
                            .unwrap_or(0);
                        let desc = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.local_name().as_ref() == b"description")
                            .and_then(|a| a.unescape_value().ok().map(|v| v.to_string()))
                            .unwrap_or_default();
                        return Err(format!("Torznab error {code}: {desc}"));
                    }
                    b"item" if is_start => {
                        in_item = true;
                        current_title.clear();
                        current_guid.clear();
                        current_download_url.clear();
                        current_size = 0;
                        current_seeders = None;
                        current_leechers = None;
                        current_pub_date = None;
                        current_categories.clear();
                        current_enclosure_type.clear();
                    }
                    b"enclosure" if in_item => {
                        for attr in e.attributes().flatten() {
                            match attr.key.local_name().as_ref() {
                                b"url" => {
                                    if let Ok(val) = attr.unescape_value() {
                                        current_download_url = val.to_string();
                                    }
                                }
                                b"length" if current_size == 0 => {
                                    if let Ok(val) = attr.unescape_value() {
                                        current_size = val.parse().unwrap_or(0);
                                    }
                                }
                                b"type" => {
                                    if let Ok(val) = attr.unescape_value() {
                                        current_enclosure_type = val.to_string();
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    // <torznab:attr> / <newznab:attr> — local name is "attr"
                    b"attr" if in_item => {
                        let mut attr_name = String::new();
                        let mut attr_value = String::new();
                        for attr in e.attributes().flatten() {
                            match attr.key.local_name().as_ref() {
                                b"name" => {
                                    if let Ok(v) = attr.unescape_value() {
                                        attr_name = v.to_string();
                                    }
                                }
                                b"value" => {
                                    if let Ok(v) = attr.unescape_value() {
                                        attr_value = v.to_string();
                                    }
                                }
                                _ => {}
                            }
                        }
                        match attr_name.as_str() {
                            "seeders" => current_seeders = attr_value.parse().ok(),
                            "peers" | "leechers" => {
                                if current_leechers.is_none() {
                                    current_leechers = attr_value.parse().ok();
                                }
                            }
                            "size" if current_size == 0 => {
                                current_size = attr_value.parse().unwrap_or(0);
                            }
                            "category" => {
                                if let Ok(cat) = attr_value.parse::<i32>() {
                                    current_categories.push(cat);
                                }
                            }
                            _ => {}
                        }
                    }
                    _ if in_item && is_start => {
                        current_tag = local.as_ref().to_vec();
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(ref e)) if in_item => {
                if let Ok(text) = e.unescape() {
                    match current_tag.as_slice() {
                        b"title" => current_title.push_str(&text),
                        b"guid" => current_guid.push_str(&text),
                        b"size" if current_size == 0 => {
                            current_size = text.parse().unwrap_or(0);
                        }
                        b"pubDate" => {
                            current_pub_date
                                .get_or_insert_with(String::new)
                                .push_str(&text);
                        }
                        _ => {}
                    }
                }
            }
            Ok(Event::End(ref e)) => {
                if e.local_name().as_ref() == b"item" && in_item {
                    in_item = false;
                    if current_guid.is_empty() || current_download_url.is_empty() {
                        tracing::warn!(
                            indexer = %indexer_name,
                            title = %current_title,
                            "skipping item missing guid or downloadUrl"
                        );
                        continue;
                    }
                    // Detect protocol from enclosure type (USE-GRAB-001).
                    let protocol = if current_enclosure_type.contains("nzb") {
                        "usenet"
                    } else {
                        "torrent"
                    }
                    .to_string();

                    results.push(ReleaseResponse {
                        title: std::mem::take(&mut current_title),
                        indexer: indexer_name.to_string(),
                        size: current_size,
                        guid: std::mem::take(&mut current_guid),
                        download_url: std::mem::take(&mut current_download_url),
                        seeders: current_seeders,
                        leechers: current_leechers,
                        publish_date: current_pub_date.take(),
                        protocol,
                        categories: std::mem::take(&mut current_categories),
                    });
                }
                current_tag.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("XML parse error: {e}")),
            _ => {}
        }
    }

    Ok(results)
}

/// POST /api/v1/release/grab — route to qBittorrent or SABnzbd based on protocol
pub async fn grab(
    State(state): State<AppState>,
    ctx: AuthContext,
    Json(req): Json<GrabApiRequest>,
) -> Result<(), ApiError> {
    use livrarr_domain::services::{DownloadProtocol, GrabRequest, GrabSource, ReleaseService};

    let protocol = match req.protocol.as_deref() {
        Some("usenet") => DownloadProtocol::Usenet,
        _ => DownloadProtocol::Torrent,
    };

    state
        .release_service
        .grab(
            ctx.user.id,
            GrabRequest {
                work_id: req.work_id,
                download_url: req.download_url,
                title: req.title,
                indexer: req.indexer,
                guid: req.guid,
                size: req.size,
                protocol,
                categories: req.categories,
                download_client_id: req.download_client_id,
                source: GrabSource::Manual,
            },
        )
        .await?;

    Ok(())
}

/// Request for internal grab (used by both API handler and RSS job).
pub struct InternalGrabRequest {
    pub user_id: livrarr_domain::UserId,
    pub work_id: livrarr_domain::WorkId,
    pub download_url: String,
    pub title: String,
    pub indexer: String,
    pub guid: String,
    pub size: i64,
    pub protocol: String,
    pub categories: Vec<i32>,
    pub download_client_id: Option<livrarr_domain::DownloadClientId>,
    pub source: String,
}

#[derive(Debug)]
pub enum GrabError {
    NoClient { protocol: String },
    ClientProtocolMismatch { protocol: String },
    ClientUnreachable { message: String },
    Ssrf(String),
    Db(livrarr_domain::DbError),
}

impl From<livrarr_domain::DbError> for GrabError {
    fn from(e: livrarr_domain::DbError) -> Self {
        GrabError::Db(e)
    }
}

/// Core grab logic shared between HTTP handler and RSS job.
pub async fn do_grab_internal(
    state: &AppState,
    req: InternalGrabRequest,
) -> Result<livrarr_domain::Grab, GrabError> {
    // SSRF protection
    livrarr_http::ssrf::validate_url(&req.download_url)
        .await
        .map_err(|e| GrabError::Ssrf(e.to_string()))?;

    let protocol = req.protocol.as_str();

    // Get download client: specified, or default for protocol.
    let client_type = match protocol {
        "usenet" => "sabnzbd",
        _ => "qbittorrent",
    };

    let client = if let Some(client_id) = req.download_client_id {
        let c = state.db.get_download_client(client_id).await?;
        if c.client_type() != client_type {
            return Err(GrabError::ClientProtocolMismatch {
                protocol: protocol.to_string(),
            });
        }
        c
    } else {
        state
            .db
            .get_default_download_client(client_type)
            .await?
            .ok_or_else(|| GrabError::NoClient {
                protocol: protocol.to_string(),
            })?
    };

    // Build a temporary GrabApiRequest for the existing helpers.
    let api_req = GrabApiRequest {
        work_id: req.work_id,
        download_url: req.download_url.clone(),
        title: req.title.clone(),
        indexer: req.indexer.clone(),
        guid: req.guid.clone(),
        size: req.size,
        download_client_id: req.download_client_id,
        protocol: Some(req.protocol.clone()),
        categories: req.categories.clone(),
    };

    let download_id = match client.client_type() {
        "sabnzbd" => grab_sabnzbd(state, &client, &api_req).await.map_err(|e| {
            GrabError::ClientUnreachable {
                message: e.to_string(),
            }
        })?,
        _ => grab_qbittorrent(state, &client, &api_req)
            .await
            .map_err(|e| GrabError::ClientUnreachable {
                message: e.to_string(),
            })?,
    };

    // Derive media type from categories: 7020 = ebook, 3030 = audiobook.
    let media_type = if req.categories.contains(&7020) {
        Some(crate::MediaType::Ebook)
    } else if req.categories.contains(&3030) {
        Some(crate::MediaType::Audiobook)
    } else if req.categories.iter().any(|&c| (7000..8000).contains(&c)) {
        Some(crate::MediaType::Ebook)
    } else if req.categories.iter().any(|&c| (3000..4000).contains(&c)) {
        Some(crate::MediaType::Audiobook)
    } else {
        None
    };

    // Record grab in DB.
    let grab_record = state
        .db
        .upsert_grab(CreateGrabDbRequest {
            user_id: req.user_id,
            work_id: req.work_id,
            download_client_id: client.id,
            title: req.title.clone(),
            indexer: req.indexer.clone(),
            guid: req.guid,
            size: Some(req.size),
            download_url: req.download_url,
            download_id,
            status: GrabStatus::Sent,
            media_type,
        })
        .await?;

    // Record history event.
    if let Err(e) = state
        .db
        .create_history_event(CreateHistoryEventDbRequest {
            user_id: req.user_id,
            work_id: Some(req.work_id),
            event_type: EventType::Grabbed,
            data: serde_json::json!({
                "title": req.title,
                "indexer": req.indexer,
                "downloadClient": client.name,
                "protocol": protocol,
                "source": req.source,
            }),
        })
        .await
    {
        tracing::warn!("create_history_event failed: {e}");
    }

    Ok(grab_record)
}

/// USE-GRAB-002: Grab via SABnzbd — download NZB, push via addfile multipart.
async fn grab_sabnzbd(
    state: &AppState,
    client: &livrarr_domain::DownloadClient,
    req: &GrabApiRequest,
) -> Result<Option<String>, ApiError> {
    let base_url = super::download_client::client_base_url(client);
    let api_key = client.api_key.as_deref().unwrap_or("");

    // Step 1: Download NZB from indexer into memory (SSRF-safe client).
    let nzb_resp = state
        .http_client_safe
        .get(&req.download_url)
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("Failed to download NZB from indexer: {e}")))?;

    if !nzb_resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "Indexer returned {} when fetching NZB",
            nzb_resp.status()
        )));
    }

    let nzb_bytes = nzb_resp
        .bytes()
        .await
        .map_err(|e| ApiError::BadGateway(format!("Failed to read NZB bytes: {e}")))?;

    // Step 2: Push to SABnzbd via multipart addfile.
    let filename = format!("{}.nzb", req.title.replace('/', "_"));
    let file_part = reqwest::multipart::Part::bytes(nzb_bytes.to_vec())
        .file_name(filename)
        .mime_str("application/x-nzb")
        .unwrap();

    let form = reqwest::multipart::Form::new()
        .text("mode", "addfile")
        .text("cat", client.category.clone())
        .text("apikey", api_key.to_string())
        .text("output", "json")
        .part("name", file_part);

    let sab_url = format!("{base_url}/api");
    let resp = state
        .http_client
        .post(&sab_url)
        .multipart(form)
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("SABnzbd addfile failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "SABnzbd returned {}",
            resp.status()
        )));
    }

    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| ApiError::BadGateway(format!("SABnzbd response parse error: {e}")))?;

    // Check for SABnzbd rejection (e.g., duplicate).
    if body.get("status").and_then(|s| s.as_bool()) == Some(false) {
        let error = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown error");
        return Err(ApiError::BadGateway(format!(
            "SABnzbd rejected NZB: {error}"
        )));
    }

    // Extract nzo_id.
    let nzo_id = body
        .get("nzo_ids")
        .and_then(|ids| ids.as_array())
        .and_then(|ids| ids.first())
        .and_then(|id| id.as_str())
        .map(|s| s.to_string());

    if let Some(ref id) = nzo_id {
        tracing::info!("grab: sent NZB to SABnzbd, nzo_id={id}");
    } else {
        tracing::warn!("grab: SABnzbd accepted NZB but returned no nzo_id");
    }

    Ok(nzo_id)
}

/// Grab via qBittorrent — existing torrent flow extracted to helper.
async fn grab_qbittorrent(
    state: &AppState,
    client: &livrarr_domain::DownloadClient,
    req: &GrabApiRequest,
) -> Result<Option<String>, ApiError> {
    let base_url = qbit_base_url(client);
    let sid = qbit_login(state, &base_url, client).await?;

    // Fetch the download once, classify, extract hash, and reuse the same bytes
    // when posting to qBit (avoids a second download from the indexer).
    let fetched = fetch_torrent_source(&state.http_client_safe, &req.download_url).await;

    let download_id = fetched
        .as_ref()
        .and_then(|src| livrarr_download::extract_torrent_hash(src).ok());

    if let Some(ref hash) = download_id {
        tracing::info!("grab: extracted hash {hash} from download URL");
    } else {
        tracing::warn!("grab: could not extract hash from download URL");
    }

    // Add torrent to qBit — use the bytes we already fetched for .torrent files,
    // or the magnet URI for magnets.
    let add_url = format!("{base_url}/api/v2/torrents/add");
    let mut form =
        reqwest::multipart::Form::new().text("category", client.category.as_str().to_string());

    match fetched {
        Some(livrarr_download::TorrentSource::Magnet(uri)) => {
            form = form.text("urls", uri);
        }
        Some(livrarr_download::TorrentSource::TorrentFile { filename, data }) => {
            let part = reqwest::multipart::Part::bytes(data)
                .file_name(filename)
                .mime_str("application/x-bittorrent")
                .map_err(|e| ApiError::BadGateway(format!("qBittorrent mime error: {e}")))?;
            form = form.part("torrents", part);
        }
        _ => {
            // Fall back to URL if we couldn't fetch/classify.
            form = form.text("urls", req.download_url.clone());
        }
    }

    let add_resp = state
        .http_client
        .post(&add_url)
        .header("Cookie", format!("SID={sid}"))
        .multipart(form)
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("qBittorrent add failed: {e}")))?;

    if !add_resp.status().is_success() {
        let body = add_resp.text().await.unwrap_or_default();
        return Err(ApiError::BadGateway(format!(
            "qBittorrent add torrent failed: {body}"
        )));
    }

    Ok(download_id)
}

/// Fetch a download URL once and classify it as a magnet URI or .torrent file.
/// Returns the `TorrentSource` so callers can both extract the info_hash and
/// reuse the fetched bytes (avoiding a second download).
async fn fetch_torrent_source(
    http: &livrarr_http::HttpClient,
    download_url: &str,
) -> Option<livrarr_download::TorrentSource> {
    // SSRF protection handled by the safe client's DNS resolver.

    // If it's already a magnet link, return directly.
    if download_url.starts_with("magnet:") {
        return Some(livrarr_download::TorrentSource::Magnet(
            download_url.to_string(),
        ));
    }

    // Fetch the URL — may redirect to magnet or return .torrent bytes.
    let resp = http.get(download_url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }

    // Check for magnet redirect.
    let final_url = resp.url().to_string();
    if final_url.starts_with("magnet:") {
        return Some(livrarr_download::TorrentSource::Magnet(final_url));
    }

    // Enforce size limit on .torrent downloads.
    if let Some(content_length) = resp.content_length() {
        if content_length as usize > MAX_DOWNLOAD_BYTES {
            tracing::warn!(
                content_length,
                "download URL content-length exceeds MAX_DOWNLOAD_BYTES"
            );
            return None;
        }
    }

    let bytes = resp.bytes().await.ok()?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        tracing::warn!(
            size = bytes.len(),
            "download URL response exceeds MAX_DOWNLOAD_BYTES"
        );
        return None;
    }

    // Try as magnet text in body.
    if let Ok(text) = std::str::from_utf8(&bytes) {
        if text.trim().starts_with("magnet:") {
            return Some(livrarr_download::TorrentSource::Magnet(
                text.trim().to_string(),
            ));
        }
    }

    Some(livrarr_download::TorrentSource::TorrentFile {
        filename: "download.torrent".to_string(),
        data: bytes.to_vec(),
    })
}

/// Build qBit base URL from download client config.
pub(crate) fn qbit_base_url(client: &livrarr_domain::DownloadClient) -> String {
    // If host already has scheme, use it directly.
    if client.host.starts_with("http://") || client.host.starts_with("https://") {
        let url_base = client.url_base.as_deref().unwrap_or("");
        return format!("{}{url_base}", client.host.trim_end_matches('/'));
    }

    let scheme = if client.use_ssl { "https" } else { "http" };
    let url_base = client.url_base.as_deref().unwrap_or("");
    if client.port == 80 || client.port == 443 {
        format!("{scheme}://{}{url_base}", client.host)
    } else {
        format!("{scheme}://{}:{}{url_base}", client.host, client.port)
    }
}

/// Authenticate to qBittorrent and return SID cookie.
pub(crate) async fn qbit_login(
    state: &AppState,
    base_url: &str,
    client: &livrarr_domain::DownloadClient,
) -> Result<String, ApiError> {
    let username = client.username.as_deref().unwrap_or("");
    let password = client.password.as_deref().unwrap_or("");

    // Anonymous mode — no auth needed.
    if username.is_empty() && password.is_empty() {
        return Ok(String::new());
    }

    let login_url = format!("{base_url}/api/v2/auth/login");
    let resp = state
        .http_client
        .post(&login_url)
        .form(&[("username", username), ("password", password)])
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("qBittorrent login failed: {e}")))?;

    // Extract SID from Set-Cookie header before consuming body.
    let sid = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            s.split(';')
                .find(|part| part.trim().starts_with("SID="))
                .map(|part| part.trim().trim_start_matches("SID=").to_string())
        })
        .unwrap_or_default();

    let body = resp.text().await.unwrap_or_default();
    if body.contains("Fails") {
        return Err(ApiError::BadGateway(
            "qBittorrent authentication failed".into(),
        ));
    }

    Ok(sid)
}
