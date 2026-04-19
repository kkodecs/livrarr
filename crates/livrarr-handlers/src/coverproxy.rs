use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::accessors::CoverProxyCacheAccessor;
use crate::context::AppContext;

const MAX_IMAGE_SIZE: usize = 500_000;

#[derive(serde::Deserialize)]
pub struct CoverProxyQuery {
    pub url: String,
}

pub async fn proxy_cover<S: AppContext>(
    State(state): State<S>,
    Query(q): Query<CoverProxyQuery>,
) -> Response {
    let url = &q.url;

    if !is_allowed_cover_source(url) {
        return (StatusCode::FORBIDDEN, "not an allowed cover source").into_response();
    }

    if let Some((data, content_type)) = state.cover_proxy_cache().get(url).await {
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

    let resp = match state.http_client().get(url).send().await {
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

    if !content_type.starts_with("image/") {
        return StatusCode::FORBIDDEN.into_response();
    }

    let data = match resp.bytes().await {
        Ok(b) if b.len() <= MAX_IMAGE_SIZE => b.to_vec(),
        _ => return StatusCode::BAD_GATEWAY.into_response(),
    };

    state
        .cover_proxy_cache()
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

fn is_allowed_cover_source(url: &str) -> bool {
    let parsed = match reqwest::Url::parse(url) {
        Ok(p) => p,
        Err(_) => return false,
    };

    if parsed.scheme() != "https" {
        return false;
    }

    let host = match parsed.host_str() {
        Some(h) => h.to_ascii_lowercase(),
        None => return false,
    };

    const ALLOWED_HOSTS: &[&str] = &[
        "images-na.ssl-images-amazon.com",
        "covers.openlibrary.org",
        "image.aladin.co.kr",
        "s.lubimyczytac.pl",
        "m.media-amazon.com",
        "books.google.com",
        "contents.kyobobook.co.kr",
        "i.gr-assets.com",
    ];

    if ALLOWED_HOSTS.iter().any(|h| *h == host) {
        return true;
    }

    if let Some(shard) = host.strip_prefix("imagessl") {
        if let Some(rest) = shard.strip_suffix(".casadellibro.com") {
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                return true;
            }
        }
    }

    false
}
