use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::state::AppState;

/// In-memory cover cache with 5-minute TTL. Shared across requests.
pub struct CoverProxyCache {
    entries: RwLock<HashMap<String, CacheEntry>>,
}

struct CacheEntry {
    data: Vec<u8>,
    content_type: String,
    fetched_at: Instant,
}

const CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes
const MAX_CACHE_ENTRIES: usize = 200;
const MAX_IMAGE_SIZE: usize = 500_000; // 500KB

impl CoverProxyCache {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    async fn get(&self, url: &str) -> Option<(Vec<u8>, String)> {
        let cache = self.entries.read().await;
        let entry = cache.get(url)?;
        if entry.fetched_at.elapsed() < CACHE_TTL {
            Some((entry.data.clone(), entry.content_type.clone()))
        } else {
            None
        }
    }

    async fn put(&self, url: String, data: Vec<u8>, content_type: String) {
        let mut cache = self.entries.write().await;
        // Evict expired entries if cache is getting large
        if cache.len() >= MAX_CACHE_ENTRIES {
            cache.retain(|_, e| e.fetched_at.elapsed() < CACHE_TTL);
        }
        cache.insert(
            url,
            CacheEntry {
                data,
                content_type,
                fetched_at: Instant::now(),
            },
        );
    }
}

#[derive(serde::Deserialize)]
pub struct CoverProxyQuery {
    pub url: String,
}

/// GET /api/v1/coverproxy?url=https://...
///
/// Proxies an external cover image through the server.
/// Used for cover sources that block direct browser requests (e.g., Casa del Libro CDN).
/// Caches in memory for 5 minutes.
pub async fn proxy_cover(
    State(state): State<AppState>,
    Query(q): Query<CoverProxyQuery>,
) -> Response {
    let url = &q.url;

    // Only proxy image URLs from known cover sources
    if !is_allowed_cover_source(url) {
        return (StatusCode::FORBIDDEN, "not an allowed cover source").into_response();
    }

    // Check cache
    if let Some((data, content_type)) = state.cover_proxy_cache.get(url).await {
        return (
            StatusCode::OK,
            [
                (
                    header::CONTENT_TYPE,
                    HeaderValue::from_str(&content_type)
                        .unwrap_or(HeaderValue::from_static("image/jpeg")),
                ),
                (
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=300"),
                ),
            ],
            data,
        )
            .into_response();
    }

    // Fetch from source
    let resp = match state.http_client.get(url).send().await {
        Ok(r) => r,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    if !resp.status().is_success() {
        return StatusCode::NOT_FOUND.into_response();
    }

    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();

    // Only proxy images
    if !content_type.starts_with("image/") {
        return StatusCode::FORBIDDEN.into_response();
    }

    let data = match resp.bytes().await {
        Ok(b) if b.len() <= MAX_IMAGE_SIZE => b.to_vec(),
        _ => return StatusCode::BAD_GATEWAY.into_response(),
    };

    // Cache it
    state
        .cover_proxy_cache
        .put(url.clone(), data.clone(), content_type.clone())
        .await;

    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_str(&content_type)
                    .unwrap_or(HeaderValue::from_static("image/jpeg")),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=300"),
            ),
        ],
        data,
    )
        .into_response()
}

/// Only allow proxying from known cover image sources.
fn is_allowed_cover_source(url: &str) -> bool {
    let allowed = [
        "imagessl",                        // Casa del Libro CDN
        "images-na.ssl-images-amazon.com", // Amazon covers
        "covers.openlibrary.org",          // OL covers (English)
        "image.aladin.co.kr",              // Aladin (Korean)
        "s.lubimyczytac.pl",               // lubimyczytac (Polish)
        "m.media-amazon.com",              // Amazon media
        "books.google.com",                // Google Books
        "contents.kyobobook.co.kr",        // Kyobo (Korean)
        "i.gr-assets.com",                 // Goodreads covers
    ];
    url.starts_with("https://") && allowed.iter().any(|a| url.contains(a))
}
