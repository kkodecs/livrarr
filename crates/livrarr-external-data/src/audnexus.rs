//! Audnexus REST client, consumed via `ProviderClient::Audnexus` (queue
//! dispatch and the identity-resolution fan-out).

use livrarr_http::HttpClient;
use std::num::NonZeroUsize;
use std::sync::Arc;
use tokio::sync::Mutex;

const CACHE_CAP: usize = 512;

struct CachedResponse {
    last_modified: String,
    body: serde_json::Value,
}

#[derive(Clone)]
pub struct AudnexusCache(Arc<Mutex<lru::LruCache<String, CachedResponse>>>);

impl Default for AudnexusCache {
    fn default() -> Self {
        Self(Arc::new(Mutex::new(lru::LruCache::new(
            NonZeroUsize::new(CACHE_CAP).unwrap(),
        ))))
    }
}

impl AudnexusCache {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Parsed subset of the Audnexus book detail response — narrators, runtime,
/// ASIN, and cover URL.
#[derive(Debug, Clone)]
pub struct AudnexusResult {
    pub narrators: Vec<String>,
    pub narrators_empty: bool,
    pub duration_seconds: Option<i32>,
    pub asin: Option<String>,
    pub cover_url: Option<String>,
}

/// Query Audnexus, preferring lookup by ASIN and falling back to title+author search.
///
/// Returns `Ok(Some(_))` on a parseable hit, `Ok(None)` if no match, `Err(_)` on
/// transport or parse errors. The error string is opaque — callers that need
/// failure-class discrimination (timeout vs 5xx vs DNS) should inspect the
/// underlying `reqwest::Error` themselves.
pub async fn query_audnexus(
    http: &HttpClient,
    base_url: &str,
    asin: Option<&str>,
    title: &str,
    author: &str,
    cache: &AudnexusCache,
) -> Result<Option<AudnexusResult>, String> {
    let base = base_url.trim_end_matches('/');

    // Try by ASIN first.
    if let Some(asin) = asin {
        if let Some(result) = query_audnexus_by_asin(http, base_url, asin, cache).await? {
            return Ok(Some(result));
        }
    }

    // Fallback: search by title + author.
    let url = format!(
        "{base}/books?title={}&author={}",
        urlencoding(title),
        urlencoding(author),
    );
    if let Some(result) = cached_fetch(http, &url, cache).await? {
        let book = if result.is_array() {
            result.as_array().and_then(|a| a.first()).cloned()
        } else {
            Some(result)
        };
        return match book {
            Some(b) => Ok(Some(parse_audnexus(&b, None))),
            None => Ok(None),
        };
    }

    Ok(None)
}

/// ASIN-only Audnexus lookup — the anchor tier of [`query_audnexus`], exposed
/// separately for the anchor-grounded enrichment surface (REQ-006): no
/// title/author fallback exists on this path.
pub async fn query_audnexus_by_asin(
    http: &HttpClient,
    base_url: &str,
    asin: &str,
    cache: &AudnexusCache,
) -> Result<Option<AudnexusResult>, String> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/books/{asin}");
    match cached_fetch(http, &url, cache).await? {
        Some(result) => Ok(Some(parse_audnexus(&result, Some(asin)))),
        None => Ok(None),
    }
}

async fn cached_fetch(
    http: &HttpClient,
    url: &str,
    cache: &AudnexusCache,
) -> Result<Option<serde_json::Value>, String> {
    let cached_last_modified = {
        let mut guard = cache.0.lock().await;
        guard.get(url).map(|c| c.last_modified.clone())
    };

    let mut req = http.get(url);
    if let Some(ref lm) = cached_last_modified {
        req = req.header("If-Modified-Since", lm);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let status = resp.status();

    if status.as_u16() == 304 {
        let mut guard = cache.0.lock().await;
        if let Some(cached) = guard.get(url) {
            tracing::debug!(%url, "audnexus 304 — reusing cached response");
            return Ok(Some(cached.body.clone()));
        }
    }

    if !status.is_success() {
        return Ok(None);
    }

    let last_modified = resp
        .headers()
        .get("Last-Modified")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;

    if let Some(lm) = last_modified {
        let mut guard = cache.0.lock().await;
        guard.put(
            url.to_string(),
            CachedResponse {
                last_modified: lm,
                body: data.clone(),
            },
        );
    }

    Ok(Some(data))
}

/// Parse a single Audnexus book JSON object into `AudnexusResult`.
pub fn parse_audnexus(data: &serde_json::Value, asin_hint: Option<&str>) -> AudnexusResult {
    let narrators = data
        .get("narrators")
        .and_then(|n| n.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|n| {
                    n.get("name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let duration_seconds = data
        .get("runtimeLengthSec")
        .or_else(|| data.get("runtime_length_sec"))
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);

    let asin = data
        .get("asin")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| asin_hint.map(|s| s.to_string()));

    let cover_url = data
        .get("image")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let narrators_empty = narrators.is_empty();

    AudnexusResult {
        narrators,
        narrators_empty,
        duration_seconds,
        asin,
        cover_url,
    }
}

/// Minimal query-string encoder: escapes only the five characters its
/// callers exercise. Switching to the `urlencoding` crate is a deliberate
/// non-change — it would alter encoding semantics.
fn urlencoding(s: &str) -> String {
    s.replace(' ', "+")
        .replace('&', "%26")
        .replace('=', "%3D")
        .replace('?', "%3F")
        .replace('#', "%23")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_audnexus_extracts_narrators_and_runtime() {
        let json = serde_json::json!({
            "asin": "B07ABCDEFG",
            "narrators": [{"name": "Jane Smith"}, {"name": "John Doe"}],
            "runtimeLengthSec": 36000,
        });
        let result = parse_audnexus(&json, None);
        assert_eq!(result.asin.as_deref(), Some("B07ABCDEFG"));
        assert_eq!(result.narrators, vec!["Jane Smith", "John Doe"]);
        assert_eq!(result.duration_seconds, Some(36000));
        assert!(!result.narrators_empty);
    }

    #[test]
    fn parse_audnexus_falls_back_to_asin_hint_when_response_omits_asin() {
        let json = serde_json::json!({"narrators": [], "runtime_length_sec": 1800});
        let result = parse_audnexus(&json, Some("B07HINT123"));
        assert_eq!(result.asin.as_deref(), Some("B07HINT123"));
        assert_eq!(result.duration_seconds, Some(1800));
        assert!(result.narrators_empty);
    }
}
