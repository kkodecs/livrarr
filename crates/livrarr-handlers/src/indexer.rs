use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use quick_xml::events::Event;
use quick_xml::Reader;

use crate::context::AppContext;
use crate::middleware::RequireAdmin;
use crate::{
    ApiError, CreateIndexerApiRequest, IndexerResponse, TestIndexerApiRequest,
    TestIndexerApiResponse, UpdateIndexerApiRequest,
};
use livrarr_domain::services::SettingsService;
use livrarr_domain::settings::{CreateIndexerParams, UpdateIndexerParams, UpdateProwlarrParams};
use livrarr_domain::{Indexer, IndexerId};

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

fn normalize_url(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn normalize_api_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

async fn test_indexer_caps<S: AppContext>(
    state: &S,
    url: &str,
    api_path: &str,
    api_key: Option<&str>,
) -> Result<TestIndexerApiResponse, ApiError> {
    let url = url.trim_end_matches('/');
    let api_path = if api_path.starts_with('/') {
        api_path.to_string()
    } else {
        format!("/{api_path}")
    };

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

    let resp = state
        .http_client_safe()
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

    let trimmed = body.trim_start();
    if trimmed.starts_with("<!DOCTYPE")
        || trimmed.starts_with("<html")
        || trimmed.starts_with("<HTML")
    {
        return Err(ApiError::BadGateway(
            "Indexer returned an HTML page instead of Torznab XML — check the URL and API key. If this site uses Cloudflare, route it through Prowlarr.".into(),
        ));
    }

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

pub async fn list<S: AppContext>(
    State(state): State<S>,
) -> Result<Json<Vec<IndexerResponse>>, ApiError> {
    let indexers = state.settings_service().list_indexers().await?;
    Ok(Json(indexers.iter().map(indexer_to_response).collect()))
}

pub async fn get<S: AppContext>(
    State(state): State<S>,
    Path(id): Path<IndexerId>,
) -> Result<Json<IndexerResponse>, ApiError> {
    let indexer = state.settings_service().get_indexer(id).await?;
    Ok(Json(indexer_to_response(&indexer)))
}

pub async fn create<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<CreateIndexerApiRequest>,
) -> Result<Json<IndexerResponse>, ApiError> {
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
        .settings_service()
        .create_indexer(CreateIndexerParams {
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

pub async fn update<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Path(id): Path<IndexerId>,
    Json(req): Json<UpdateIndexerApiRequest>,
) -> Result<Json<IndexerResponse>, ApiError> {
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

    if let Some(Some(ref k)) = req.api_key {
        if k.is_empty() {
            return Err(ApiError::BadRequest(
                "api_key must not be empty string; use null to clear".into(),
            ));
        }
    }

    let url = req.url.map(|u| normalize_url(&u));
    let api_path = req.api_path.map(|p| normalize_api_path(&p));

    let indexer = state
        .settings_service()
        .update_indexer(
            id,
            UpdateIndexerParams {
                name: req.name,
                url,
                api_path,
                api_key: req.api_key,
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

pub async fn delete<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Path(id): Path<IndexerId>,
) -> Result<(), ApiError> {
    state.settings_service().delete_indexer(id).await?;
    Ok(())
}

pub async fn test<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<TestIndexerApiRequest>,
) -> Result<Json<TestIndexerApiResponse>, ApiError> {
    let result = test_indexer_caps(&state, &req.url, &req.api_path, req.api_key.as_deref()).await?;
    Ok(Json(result))
}

pub async fn test_saved<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Path(id): Path<IndexerId>,
) -> Result<Json<TestIndexerApiResponse>, ApiError> {
    let indexer = state
        .settings_service()
        .get_indexer_with_credentials(id)
        .await?;

    let result = test_indexer_caps(
        &state,
        &indexer.url,
        &indexer.api_path,
        indexer.api_key.as_deref(),
    )
    .await?;

    if result.ok {
        state
            .settings_service()
            .set_supports_book_search(id, result.supports_book_search)
            .await?;
    }

    Ok(Json(result))
}

// ---------------------------------------------------------------------------
// Prowlarr Import
// ---------------------------------------------------------------------------

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

pub async fn import_from_prowlarr<S: AppContext>(
    State(state): State<S>,
    _admin: RequireAdmin,
    Json(req): Json<crate::ProwlarrImportRequest>,
) -> Result<Json<crate::ProwlarrImportResponse>, ApiError> {
    let saved = state
        .settings_service()
        .get_prowlarr_config()
        .await
        .unwrap_or_default();
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
        .http_client_safe()
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

    let existing = state.settings_service().list_indexers().await?;
    let existing_urls: std::collections::HashSet<String> = existing
        .iter()
        .map(|i| i.url.trim_end_matches('/').to_lowercase())
        .collect();

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut errors = Vec::new();

    for pi in prowlarr_indexers {
        let url = format!("{base}/{}", pi.id);
        if existing_urls.contains(&url.to_lowercase()) {
            skipped += 1;
            continue;
        }

        let api_key_val = Some(api_key.clone());
        let api_path = "/api".to_string();

        let protocol = match pi.protocol.as_str() {
            "usenet" => "usenet".to_string(),
            _ => "torrent".to_string(),
        };

        let priority = pi.priority.max(1);

        match state
            .settings_service()
            .create_indexer(CreateIndexerParams {
                name: pi.name.clone(),
                protocol,
                url,
                api_path: normalize_api_path(&api_path),
                api_key: api_key_val,
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

    if imported > 0 || skipped > 0 {
        let _ = state
            .settings_service()
            .update_prowlarr_config(UpdateProwlarrParams {
                url: Some(url),
                api_key: Some(Some(api_key)),
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
