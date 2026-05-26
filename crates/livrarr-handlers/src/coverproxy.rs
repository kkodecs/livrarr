use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::accessors::CoverProxyCacheAccessor;
use crate::context::{HasCoverCache, HasHmacKey, HasHttpClient};

// 5 MB — accommodates high-resolution covers from providers like Google Books.
// TODO(alpha6+): reduce stored cover resolution to limit on-disk footprint.
const MAX_IMAGE_SIZE: usize = 5 * 1024 * 1024;

#[derive(serde::Deserialize)]
pub struct CoverProxyQuery {
    pub url: String,
    #[serde(default)]
    pub sig: String,
}

pub async fn proxy_cover<S: HasCoverCache + HasHttpClient + HasHmacKey>(
    State(state): State<S>,
    Query(q): Query<CoverProxyQuery>,
) -> Response {
    let url = &q.url;

    // HMAC verification: if sig is present, verify it; if absent, fall back to allowlist
    if !q.sig.is_empty() {
        if !verify_proxy_sig(url, &q.sig, state.hmac_key()) {
            return (StatusCode::FORBIDDEN, "invalid signature").into_response();
        }
    } else if !is_allowed_cover_source(url) {
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
                    HeaderValue::from_static("public, max-age=604800"),
                ),
            ],
            data,
        )
            .into_response();
    }

    let resp = match state.http_client_safe().get(url).send().await {
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

    if let Some(declared) = resp.content_length() {
        if declared as usize > MAX_IMAGE_SIZE {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
    }

    let data = match resp.bytes().await {
        Ok(b) if b.len() <= MAX_IMAGE_SIZE => b.to_vec(),
        _ => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
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
                HeaderValue::from_static("public, max-age=604800"),
            ),
        ],
        data,
    )
        .into_response()
}

fn verify_proxy_sig(url: &str, sig: &str, key: &[u8]) -> bool {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let mut mac = match Hmac::<Sha256>::new_from_slice(key) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(url.as_bytes());
    let expected = data_encoding::HEXLOWER.encode(&mac.finalize().into_bytes());
    subtle::ConstantTimeEq::ct_eq(expected.as_bytes(), sig.as_bytes()).into()
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
