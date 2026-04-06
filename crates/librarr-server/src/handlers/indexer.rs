use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::state::AppState;
use crate::{
    ApiError, CreateIndexerApiRequest, IndexerResponse, TestIndexerApiRequest,
    TestIndexerApiResponse, UpdateIndexerApiRequest,
};
use librarr_db::{CreateIndexerDbRequest, IndexerDb, UpdateIndexerDbRequest};
use librarr_domain::{Indexer, IndexerId};
use librarr_http::HttpClient;

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
    let has_ebook = found_categories.contains(&7020);
    let has_audiobook = found_categories.contains(&3030);
    if !has_ebook && !has_audiobook {
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
            enabled: req.enabled,
        })
        .await?;

    Ok(Json(indexer_to_response(&indexer)))
}

/// PUT /api/v1/indexer/:id
pub async fn update(
    State(state): State<AppState>,
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
                enabled: req.enabled,
            },
        )
        .await?;

    Ok(Json(indexer_to_response(&indexer)))
}

/// DELETE /api/v1/indexer/:id
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<IndexerId>,
) -> Result<(), ApiError> {
    state.db.delete_indexer(id).await?;
    Ok(())
}

/// POST /api/v1/indexer/test
pub async fn test(
    State(state): State<AppState>,
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
