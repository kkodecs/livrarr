use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use livrarr_domain::services::{
    cover_bucket_for_host, FetchError, FetchRequest, HttpFetcher, HttpMethod, RateBucket,
    UserAgentProfile,
};
use livrarr_domain::RequestPriority;

use crate::accessors::CoverProxyCacheAccessor;
use crate::context::{HasCoverCache, HasHmacKey, HasHttpFetcher};

// 5 MB — accommodates high-resolution covers from providers like Google Books.
// TODO(alpha6+): reduce stored cover resolution to limit on-disk footprint.
const MAX_IMAGE_SIZE: usize = 5 * 1024 * 1024;

#[derive(serde::Deserialize)]
pub struct CoverProxyQuery {
    pub url: String,
    #[serde(default)]
    pub sig: String,
}

pub async fn proxy_cover<S: HasCoverCache + HasHttpFetcher + HasHmacKey>(
    State(state): State<S>,
    Query(q): Query<CoverProxyQuery>,
) -> Response {
    let url = &q.url;
    // Parsed once — reused for both the allowlist check below and the
    // pacing-bucket lookup, instead of re-parsing the URL twice.
    let host = parse_https_host(url);

    // HMAC verification: if sig is present, verify it; if absent, fall back to allowlist
    if !q.sig.is_empty() {
        if !verify_proxy_sig(url, &q.sig, state.hmac_key()) {
            return (StatusCode::FORBIDDEN, "invalid signature").into_response();
        }
    } else if !host.as_deref().is_some_and(is_allowed_host) {
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

    let bucket = host
        .as_deref()
        .map(cover_bucket_for_host)
        .unwrap_or(RateBucket::None);

    // Runtime URL (insight 37) — SSRF-safe fetch is mandatory here.
    let req = FetchRequest {
        url: url.clone(),
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: std::time::Duration::from_secs(30),
        rate_bucket: bucket,
        max_body_bytes: MAX_IMAGE_SIZE,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        // The user is looking at the cover grid right now (plan §B3).
        priority: RequestPriority::Interactive,
    };

    let resp = match state.http_fetcher().fetch_ssrf_safe(req).await {
        Ok(r) => r,
        Err(FetchError::BodyTooLarge { .. }) => {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response()
        }
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };

    if !(200..300).contains(&resp.status) {
        return StatusCode::NOT_FOUND.into_response();
    }

    let content_type = resp
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "image/jpeg".to_string());

    if !content_type.starts_with("image/") {
        return StatusCode::FORBIDDEN.into_response();
    }

    if let Some(declared) = resp
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
    {
        if declared > MAX_IMAGE_SIZE {
            return StatusCode::PAYLOAD_TOO_LARGE.into_response();
        }
    }

    if resp.body.len() > MAX_IMAGE_SIZE {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    }
    let data = resp.body;

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

/// Parse `url` and return its lowercased host IFF the scheme is `https`.
fn parse_https_host(url: &str) -> Option<String> {
    let parsed = reqwest::Url::parse(url).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    parsed.host_str().map(|h| h.to_ascii_lowercase())
}

/// V5: every unified-rank provider's asset host (S1) must be allowed here so
/// pre-add/alternatives previews never 403. Verified against live code +
/// production evidence, not assumed: Goodreads (`i.gr-assets.com`,
/// amazon-family), Hardcover (`assets.hardcover.app`), Google Books
/// (`books.google.com`, and `books.googleusercontent.com` — the second host
/// `work_service::cover_source_rank`'s host classifier has always
/// recognized as Google Books but which this allowlist was missing),
/// OpenLibrary (`covers.openlibrary.org`), Audible and Audnexus (both
/// confirmed live to serve from `m.media-amazon.com` — Audnexus mirrors
/// Audible's own catalog images). Readarr has no asset host of its own (it
/// relays whichever provider's URL it imported).
fn is_allowed_host(host: &str) -> bool {
    const ALLOWED_HOSTS: &[&str] = &[
        "images-na.ssl-images-amazon.com",
        "covers.openlibrary.org",
        "image.aladin.co.kr",
        "s.lubimyczytac.pl",
        "m.media-amazon.com",
        "books.google.com",
        "books.googleusercontent.com",
        "contents.kyobobook.co.kr",
        "i.gr-assets.com",
        "assets.hardcover.app",
    ];

    if ALLOWED_HOSTS.contains(&host) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_every_unified_rank_provider_asset_host() {
        // S1/V5: Goodreads, Hardcover, Google Books (both known hosts),
        // OpenLibrary, Audible + Audnexus (both amazon-hosted, live-verified).
        for host in [
            "i.gr-assets.com",
            "images-na.ssl-images-amazon.com",
            "m.media-amazon.com",
            "assets.hardcover.app",
            "books.google.com",
            "books.googleusercontent.com",
            "covers.openlibrary.org",
        ] {
            assert!(
                is_allowed_host(host),
                "{host} must be an allowed cover host"
            );
        }
    }

    #[test]
    fn rejects_unrecognized_host() {
        assert!(!is_allowed_host("evil.example.com"));
    }
}
