//! Audnexus REST client.
//!
//! Lifted out of `livrarr-server/src/handlers/enrichment.rs` so the same code
//! can serve both:
//!   - the existing inline enrichment pipeline (still on the legacy direct path), and
//!   - `ProviderClient::Audnexus`, the first real-network adapter wired through
//!     `DefaultProviderQueue` (Phase 1.5 tracer).
//!
//! Behavior is unchanged from the original implementation. Only the location
//! moves; existing call sites import from here.

use livrarr_http::HttpClient;

/// Parsed subset of the Audnexus book detail response — narrators, runtime,
/// and ASIN are the only fields the enrichment pipeline consumes today.
#[derive(Debug, Clone)]
pub struct AudnexusResult {
    pub narrators: Vec<String>,
    pub narrators_empty: bool,
    pub duration_seconds: Option<i32>,
    pub asin: Option<String>,
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
) -> Result<Option<AudnexusResult>, String> {
    let base = base_url.trim_end_matches('/');

    // Try by ASIN first.
    if let Some(asin) = asin {
        let url = format!("{base}/books/{asin}");
        if let Ok(resp) = http.get(&url).send().await {
            if resp.status().is_success() {
                if let Ok(data) = resp.json::<serde_json::Value>().await {
                    return Ok(Some(parse_audnexus(&data, Some(asin))));
                }
            }
        }
    }

    // Fallback: search by title + author.
    let url = format!(
        "{base}/books?title={}&author={}",
        urlencoding(title),
        urlencoding(author),
    );
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;

    let book = if data.is_array() {
        data.as_array().and_then(|a| a.first()).cloned()
    } else {
        Some(data)
    };

    match book {
        Some(b) => Ok(Some(parse_audnexus(&b, None))),
        None => Ok(None),
    }
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

    let narrators_empty = narrators.is_empty();

    AudnexusResult {
        narrators,
        narrators_empty,
        duration_seconds,
        asin,
    }
}

/// Pre-existing minimal query-string encoder. Only escapes the five characters
/// the legacy callers actually exercised. Carried forward verbatim from the
/// original `livrarr-server/src/handlers/enrichment.rs` to preserve behavior
/// during the lift; switching to the `urlencoding` crate is a separate change
/// because it would alter encoding semantics.
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
