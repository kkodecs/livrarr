use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::middleware::RequireAdmin;
use crate::state::AppState;
use crate::{
    ApiError, CreateIndexerApiRequest, IndexerResponse, TestIndexerApiRequest,
    TestIndexerApiResponse, UpdateIndexerApiRequest,
};
use livrarr_db::{ConfigDb, CreateIndexerDbRequest, IndexerDb, UpdateIndexerDbRequest};
use livrarr_domain::{Indexer, IndexerId};
use livrarr_http::HttpClient;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn indexer_to_response(indexer: &Indexer) -> IndexerResponse {
    IndexerResponse {
        id: indexer.id,
        name: indexer.name.clone(),
        protocol: indexer.protocol.clone(),
        url: indexer.url.clone(),
        api_path: indexer.api_path.clone(),
        api_key_set: indexer.api_key.is_some(),
        categories: indexer.categories.clone(),
        priority: indexer.priority,
        enable_automatic_search: indexer.enable_automatic_search,
        enable_interactive_search: indexer.enable_interactive_search,
        supports_book_search: indexer.supports_book_search,
        enable_rss: indexer.enable_rss,
        enabled: indexer.enabled,
        added_at: indexer.added_at,
    }
}

/// Normalize URL: strip trailing slashes.
fn normalize_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

/// Normalize api_path: ensure it starts with '/'.
fn normalize_api_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

/// Test indexer capabilities by fetching Torznab caps XML.
///
/// Builds URL: `{url}{api_path}?t=caps` + `&apikey={key}` if key is non-empty.
/// Parses XML looking for:
///   - `<searching>` -> `<book-search available="yes">` -> supports_book_search
///   - `<categories>` -> `<category id="...">` + `<subcat id="...">` -> category IDs
async fn test_indexer_caps(
    http_client: &HttpClient,
    url: &str,
    api_path: &str,
    api_key: Option<&str>,
) -> Result<TestIndexerApiResponse, ApiError> {
    // Normalize URL and api_path for test (same as create/update).
    let url = url.trim_end_matches('/');
    let api_path = if api_path.starts_with('/') {
        api_path.to_string()
    } else {
        format!("/{api_path}")
    };

    // Build caps URL with proper encoding.
    let base_with_path = format!("{url}{api_path}");
    let separator = if base_with_path.contains('?') {
        '&'
    } else {
        '?'
    };
    let mut caps_url = format!("{base_with_path}{separator}t=caps");
    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        caps_url.push_str("&apikey=");
        caps_url.push_str(&urlencoding::encode(key));
    }

    // Fetch with 10s timeout.
    let resp = http_client
        .inner()
        .get(&caps_url)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| {
            ApiError::BadGateway(format!("Failed to connect to indexer: {}", e.without_url()))
        })?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "Indexer returned HTTP {}",
            resp.status()
        )));
    }

    let body = resp.text().await.map_err(|e| {
        ApiError::BadGateway(format!(
            "Failed to read indexer response: {}",
            e.without_url()
        ))
    })?;

    // Detect HTML responses (login pages, Cloudflare challenges, etc.)
    let trimmed = body.trim_start();
    if trimmed.starts_with("<!DOCTYPE")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<HTML")
    {
        return Err(ApiError::BadGateway(
            "Indexer returned an HTML page instead of Torznab XML — check the URL and API key. If this site uses Cloudflare, route it through Prowlarr.".into(),
        ));
    }

    // Parse the XML.
    let mut reader = Reader::from_str(&body);
    reader.config_mut().trim_text(true);

    let mut supports_book_search = false;
    let mut found_categories = Vec::new();
    let mut warnings = Vec::new();

    let mut in_searching = false;
    let mut in_categories = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e) | Event::Empty(ref e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"searching" => {
                        in_searching = true;
                    }
                    b"book-search" if in_searching => {
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == b"available" {
                                let val = attr.unescape_value().unwrap_or_default();
                                if val.eq_ignore_ascii_case("yes") {
                                    supports_book_search = true;
                                }
                            }
                        }
                    }
                    b"categories" => {
                        in_categories = true;
                    }
                    b"category" if in_categories => {
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == b"id" {
                                if let Ok(val) =
                                    attr.unescape_value().unwrap_or_default().parse::<i32>()
                                {
                                    found_categories.push(val);
                                }
                            }
                        }
                    }
                    b"subcat" if in_categories => {
                        for attr in e.attributes().flatten() {
                            if attr.key.local_name().as_ref() == b"id" {
                                if let Ok(val) =
                                    attr.unescape_value().unwrap_or_default().parse::<i32>()
                                {
                                    found_categories.push(val);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let local = e.local_name();
                match local.as_ref() {
                    b"searching" => in_searching = false,
                    b"categories" => in_categories = false,
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(ApiError::BadGateway(format!(
                    "Failed to parse caps XML: {e}"
                )));
            }
            _ => {}
        }
    }

    // Check for expected categories (7020 = ebook, 3030 = audiobook).
    // Skip this warning if book search is already supported — Prowlarr proxies
    // report book-search available but may not list individual categories.
    let has_ebook = found_categories.contains(&7020);
    let has_audiobook = found_categories.contains(&3030);
    if !supports_book_search && !has_ebook && !has_audiobook {
        warnings
            .push("No book-relevant categories (7020, 3030) found in indexer capabilities".into());
    }

    Ok(TestIndexerApiResponse {
        ok: true,
        supports_book_search,
        warnings,
        error: None,
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /api/v1/indexer
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<IndexerResponse>>, ApiError> {
    let indexers = state.db.list_indexers().await?;
    Ok(Json(indexers.iter().map(indexer_to_response).collect()))
}

/// GET /api/v1/indexer/:id
pub async fn get(
    State(state): State<AppState>,
    Path(id): Path<IndexerId>,
) -> Result<Json<IndexerResponse>, ApiError> {
    let indexer = state.db.get_indexer(id).await?;
    Ok(Json(indexer_to_response(&indexer)))
}

/// POST /api/v1/indexer
pub async fn create(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(req): Json<CreateIndexerApiRequest>,
) -> Result<Json<IndexerResponse>, ApiError> {
    // Validate.
    if req.name.is_empty() {
        return Err(ApiError::BadRequest("name is required".into()));
    }
    if req.url.is_empty() {
        return Err(ApiError::BadRequest("url is required".into()));
    }
    if req.priority < 1 {
        return Err(ApiError::BadRequest("priority must be at least 1".into()));
    }

    let url = normalize_url(&req.url);
    let api_path = normalize_api_path(&req.api_path);

    let indexer = state
        .db
        .create_indexer(CreateIndexerDbRequest {
            name: req.name,
            protocol: req.protocol,
            url,
            api_path,
            api_key: req.api_key,
            categories: req.categories,
            priority: req.priority,
            enable_automatic_search: req.enable_automatic_search,
            enable_interactive_search: req.enable_interactive_search,
            enable_rss: req.enable_rss,
            enabled: req.enabled,
        })
        .await?;

    Ok(Json(indexer_to_response(&indexer)))
}

/// PUT /api/v1/indexer/:id
pub async fn update(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<IndexerId>,
    Json(req): Json<UpdateIndexerApiRequest>,
) -> Result<Json<IndexerResponse>, ApiError> {
    // Validate fields that are present.
    if let Some(ref name) = req.name {
        if name.is_empty() {
            return Err(ApiError::BadRequest("name is required".into()));
        }
    }
    if let Some(ref url) = req.url {
        if url.is_empty() {
            return Err(ApiError::BadRequest("url is required".into()));
        }
    }
    if let Some(priority) = req.priority {
        if priority < 1 {
            return Err(ApiError::BadRequest("priority must be at least 1".into()));
        }
    }

    // Normalize url/api_path if provided.
    let url = req.url.map(|u| normalize_url(&u));
    let api_path = req.api_path.map(|p| normalize_api_path(&p));

    // Treat empty api_key as None (keep existing).
    let api_key = req.api_key.filter(|k| !k.is_empty());

    let indexer = state
        .db
        .update_indexer(
            id,
            UpdateIndexerDbRequest {
                name: req.name,
                url,
                api_path,
                api_key,
                categories: req.categories,
                priority: req.priority,
                enable_automatic_search: req.enable_automatic_search,
                enable_interactive_search: req.enable_interactive_search,
                enable_rss: req.enable_rss,
                enabled: req.enabled,
            },
        )
        .await?;

    Ok(Json(indexer_to_response(&indexer)))
}

/// DELETE /api/v1/indexer/:id
pub async fn delete(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<IndexerId>,
) -> Result<(), ApiError> {
    state.db.delete_indexer(id).await?;
    Ok(())
}

/// POST /api/v1/indexer/test
pub async fn test(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(req): Json<TestIndexerApiRequest>,
) -> Result<Json<TestIndexerApiResponse>, ApiError> {
    let result = test_indexer_caps(
        &state.http_client,
        &req.url,
        &req.api_path,
        req.api_key.as_deref(),
    )
    .await?;
    Ok(Json(result))
}

/// POST /api/v1/indexer/:id/test
pub async fn test_saved(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Path(id): Path<IndexerId>,
) -> Result<Json<TestIndexerApiResponse>, ApiError> {
    let indexer = state.db.get_indexer(id).await?;

    let result = test_indexer_caps(
        &state.http_client,
        &indexer.url,
        &indexer.api_path,
        indexer.api_key.as_deref(),
    )
    .await?;

    // Persist book search capability if the test succeeded.
    if result.ok {
        state
            .db
            .set_supports_book_search(id, result.supports_book_search)
            .await?;
    }

    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// Prowlarr Import
// ---------------------------------------------------------------------------

/// Prowlarr API indexer response shape.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProwlarrIndexer {
    id: i64,
    name: String,
    #[serde(default = "default_torrent")]
    protocol: String,
    #[serde(default = "default_true")]
    enable_automatic_search: bool,
    #[serde(default = "default_true")]
    enable_interactive_search: bool,
    #[serde(default = "default_priority")]
    priority: i32,
}

fn default_true() -> bool {
    true
}

fn default_torrent() -> String {
    "torrent".to_string()
}

fn default_priority() -> i32 {
    25
}

/// POST /api/v1/indexer/import/prowlarr
pub async fn import_from_prowlarr(
    State(state): State<AppState>,
    _admin: RequireAdmin,
    Json(req): Json<crate::ProwlarrImportRequest>,
) -> Result<Json<crate::ProwlarrImportResponse>, ApiError> {
    // Fall back to saved Prowlarr config for missing fields.
    let saved = state.db.get_prowlarr_config().await.unwrap_or_default();
    let url = if req.url.is_empty() {
        saved
            .url
            .ok_or_else(|| ApiError::BadRequest("url is required".into()))?
    } else {
        req.url.clone()
    };
    let api_key = if req.api_key.is_empty() {
        saved
            .api_key
            .ok_or_else(|| ApiError::BadRequest("apiKey is required".into()))?
    } else {
        req.api_key.clone()
    };

    let base = url.trim_end_matches('/');
    let fetch_url = format!("{base}/api/v1/indexer");

    let resp = state
        .http_client
        .inner()
        .get(&fetch_url)
        .header("X-Api-Key", &api_key)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| {
            ApiError::BadGateway(format!(
                "Failed to connect to Prowlarr: {}",
                e.without_url()
            ))
        })?;

    if !resp.status().is_success() {
        return Err(ApiError::BadGateway(format!(
            "Prowlarr returned HTTP {}",
            resp.status()
        )));
    }

    let prowlarr_indexers: Vec<ProwlarrIndexer> = resp.json().await.map_err(|e| {
        ApiError::BadGateway(format!(
            "Failed to parse Prowlarr response: {}",
            e.without_url()
        ))
    })?;

    // Load existing indexers for duplicate detection (by normalized URL).
    let existing = state.db.list_indexers().await?;
    let existing_urls: std::collections::HashSet<String> = existing
        .iter()
        .map(|i| i.url.trim_end_matches('/').to_lowercase())
        .collect();

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut errors = Vec::new();

    for pi in prowlarr_indexers {
        // Use Prowlarr proxy URL instead of the direct indexer URL.
        // This routes searches through Prowlarr, which handles Cloudflare
        // and other connection issues that our HTTP client can't.
        let url = format!("{base}/{}", pi.id);
        if existing_urls.contains(&url.to_lowercase()) {
            skipped += 1;
            continue;
        }

        // Use Prowlarr's API key (not the individual indexer's key).
        let api_key = Some(api_key.clone());
        let api_path = "/api".to_string();

        let protocol = match pi.protocol.as_str() {
            "usenet" => "usenet".to_string(),
            _ => "torrent".to_string(),
        };

        // Map Prowlarr priority (1-50, default 25) to Livrarr priority (1+).
        let priority = pi.priority.max(1);

        match state
            .db
            .create_indexer(CreateIndexerDbRequest {
                name: pi.name.clone(),
                protocol,
                url,
                api_path: normalize_api_path(&api_path),
                api_key,
                categories: vec![7020, 3030],
                priority,
                enable_automatic_search: pi.enable_automatic_search,
                enable_interactive_search: pi.enable_interactive_search,
                enable_rss: true,
                enabled: true,
            })
            .await
        {
            Ok(_) => imported += 1,
            Err(e) => errors.push(format!("{}: {e}", pi.name)),
        }
    }

    // Persist Prowlarr creds on successful import so the form can be pre-filled.
    if imported > 0 || skipped > 0 {
        let _ = state
            .db
            .update_prowlarr_config(livrarr_db::UpdateProwlarrConfigRequest {
                url: Some(url),
                api_key: Some(api_key),
                enabled: Some(true),
            })
            .await;
    }

    Ok(Json(crate::ProwlarrImportResponse {
        imported,
        skipped,
        errors,
    }))
}
