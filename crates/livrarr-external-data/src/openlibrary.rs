//! OpenLibrary REST client, consumed via `ProviderClient::OpenLibrary` (queue
//! dispatch and the identity-resolution fan-out) and by the discovery path
//! (`search_openlibrary`).

use livrarr_domain::seed::iso639_1_to_3;
use livrarr_domain::services::{
    FetchError, FetchRequest, HttpFetcher, HttpMethod, LookupResult, RateBucket, UserAgentProfile,
};
use livrarr_domain::RequestPriority;

/// Parsed subset of an OpenLibrary work detail + first edition with ISBN.
#[derive(Debug, Clone)]
pub struct OlDetailResult {
    /// The work's title — identity arbitration clusters responders by
    /// title/author when no shared key exists, so a title-less payload is
    /// unclusterable and its ol_key gets discarded (#148).
    pub title: Option<String>,
    pub description: Option<String>,
    pub isbn_13: Option<String>,
    pub cover_id: Option<i64>,
}

/// Fetch work detail + first edition ISBN for an OpenLibrary work key.
///
/// `ol_key` accepts either bare keys (`OL12345W`) or path-prefixed forms
/// (`/works/OL12345W`).
pub async fn query_ol_detail<F: HttpFetcher>(
    fetcher: &F,
    ol_key: &str,
) -> Result<OlDetailResult, String> {
    let key = ol_key.trim_start_matches("/works/").trim_start_matches('/');

    let url = format!("https://openlibrary.org/works/{key}.json");
    let req = FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: std::time::Duration::from_secs(30),
        rate_bucket: RateBucket::OpenLibrary,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority: RequestPriority::Normal,
    };
    let resp = fetcher
        .fetch(req)
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !(200..300).contains(&resp.status) {
        return Err(format!("HTTP {}", resp.status));
    }

    let data: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("parse: {e}"))?;

    let title = data
        .get("title")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let description = data.get("description").and_then(|d| {
        d.as_str().map(|s| s.to_string()).or_else(|| {
            d.get("value")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
    });

    let cover_id = data
        .get("covers")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.iter().find_map(|v| v.as_i64()))
        .filter(|&id| id > 0);

    // Fetch editions for ISBN.
    let mut isbn_13 = None;
    let editions_url = format!("https://openlibrary.org/works/{key}/editions.json?limit=10");
    let editions_req = FetchRequest {
        url: editions_url,
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: std::time::Duration::from_secs(30),
        rate_bucket: RateBucket::OpenLibrary,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority: RequestPriority::Normal,
    };
    if let Ok(ed_resp) = fetcher.fetch(editions_req).await {
        if let Ok(ed_data) = serde_json::from_slice::<serde_json::Value>(&ed_resp.body) {
            if let Some(entries) = ed_data.get("entries").and_then(|e| e.as_array()) {
                for entry in entries {
                    if let Some(isbns) = entry.get("isbn_13").and_then(|i| i.as_array()) {
                        if let Some(isbn) = isbns.first().and_then(|v| v.as_str()) {
                            isbn_13 = Some(isbn.to_string());
                            break;
                        }
                    }
                }
            }
        }
    }

    Ok(OlDetailResult {
        title,
        description,
        isbn_13,
        cover_id,
    })
}

/// Resolve an ISBN-13 to its OpenLibrary work key via the ISBN lookup
/// endpoint. Any non-success status maps to `Ok(None)` ("no work found"),
/// never a hard error — including a fetcher-intercepted HTTP 429
/// (`FetchError::RateLimited`), which the pre-fetcher raw-`HttpClient` code
/// also folded into the same "no work found" outcome (it only ever checked
/// `resp.status().is_success()`, never treated any particular status as
/// retry-worthy). Only a transport-level failure (network, timeout, body)
/// is a hard `Err`.
pub async fn isbn_lookup<F: HttpFetcher>(
    fetcher: &F,
    isbn: &str,
) -> Result<Option<String>, String> {
    let url = format!("https://openlibrary.org/isbn/{isbn}.json");
    let req = FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: std::time::Duration::from_secs(30),
        rate_bucket: RateBucket::OpenLibrary,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority: RequestPriority::Normal,
    };
    let resp = match fetcher.fetch(req).await {
        Ok(r) => r,
        Err(FetchError::RateLimited) => return Ok(None),
        Err(e) => return Err(format!("OL ISBN fetch failed: {e}")),
    };

    if !(200..300).contains(&resp.status) {
        return Ok(None);
    }

    let data: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("OL ISBN parse error: {e}"))?;

    let ol_work_key = data
        .get("works")
        .and_then(|w| w.as_array())
        .and_then(|arr| arr.first())
        .and_then(|w| w.get("key"))
        .and_then(|k| k.as_str())
        .map(|k| k.strip_prefix("/works/").unwrap_or(k).to_string());

    Ok(ol_work_key)
}

/// Search OpenLibrary `search.json` for books matching `term`.
///
/// `lang` is the ISO 639-1 language code (e.g. `"en"`, `"fr"`); for non-English
/// searches an `&language=` filter is appended. Returns one [`LookupResult`] per
/// document — OL key, title, first author, first-publish year, and cover URL.
///
/// Extracted from `work_service::lookup_openlibrary` (M-004 / Phase 2 dedup).
/// The `provider_client.rs` title+author search that returns [`NormalizedWorkDetail`]
/// for enrichment is a separate shape and must stay separate.
pub async fn search_openlibrary<H: HttpFetcher + Send + Sync>(
    http: &H,
    term: &str,
    lang: &str,
) -> Result<Vec<LookupResult>, String> {
    let lang_param = if lang != "en" {
        let ol_lang = iso639_1_to_3(lang);
        format!("&language={}", urlencoding::encode(ol_lang))
    } else {
        String::new()
    };
    let url = format!(
        "https://openlibrary.org/search.json?q={}&limit=50&fields=key,title,author_name,author_key,first_publish_year,cover_i{lang_param}",
        urlencoding::encode(term)
    );

    let fetch_req = FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: std::time::Duration::from_secs(10),
        rate_bucket: RateBucket::OpenLibrary,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority: RequestPriority::Normal,
    };

    let resp = http
        .fetch(fetch_req)
        .await
        .map_err(|e| format!("OpenLibrary request failed: {e}"))?;

    if resp.status >= 400 {
        return Err(format!("OpenLibrary returned {}", resp.status));
    }

    let data: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("OpenLibrary parse error: {e}"))?;

    let docs = data
        .get("docs")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    let results = docs
        .iter()
        .filter_map(|doc| {
            let key = doc.get("key")?.as_str()?;
            let title = doc.get("title")?.as_str()?;
            let ol_key = key.trim_start_matches("/works/").to_string();

            let author_name = doc
                .get("author_name")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|a| a.as_str())
                .unwrap_or("Unknown")
                .to_string();

            let author_ol_key = doc
                .get("author_key")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|a| a.as_str())
                .map(|k| k.trim_start_matches("/authors/").to_string());

            let year = doc
                .get("first_publish_year")
                .and_then(|y| y.as_i64())
                .map(|y| y as i32);

            let cover_url = doc
                .get("cover_i")
                .and_then(|c| c.as_i64())
                .map(|c| format!("https://covers.openlibrary.org/b/id/{c}-L.jpg"));

            Some(LookupResult {
                ol_key: Some(ol_key),
                title: title.to_string(),
                author_name,
                author_ol_key,
                year,
                cover_url,
                description: None,
                series_name: None,
                series_position: None,
                source: Some("openlibrary".to_string()),
                source_type: Some("openlibrary".to_string()),
                language: Some(lang.to_string()),
                detail_url: None,
                rating: None,
                isbn_13: None,
                candidate_id: None,
                hc_key: None,
                gr_key: None,
                asin: None,
            })
        })
        .collect();

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------
    // Door-routing: query_ol_detail goes through the HttpFetcher trait with
    // the OpenLibrary rate bucket, GET, no auth, for both the work-detail
    // call and the editions call.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn query_ol_detail_sends_openlibrary_bucket_get_for_both_calls() {
        // A single canned body serves both requests (RecordingHttpFetcher
        // repeats its one queued response) — it carries work-detail fields
        // but no `entries` key, so the editions parse naturally finds
        // nothing and isbn_13 stays None.
        let canned = serde_json::json!({
            "title": "Test Work",
            "description": "A description",
            "covers": [12345]
        });
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let result = query_ol_detail(&fetcher, "OL123W").await.unwrap();

        assert_eq!(result.title.as_deref(), Some("Test Work"));
        assert_eq!(result.description.as_deref(), Some("A description"));
        assert_eq!(result.cover_id, Some(12345));
        assert_eq!(result.isbn_13, None);

        let reqs = fetcher.requests();
        assert_eq!(
            reqs.len(),
            2,
            "expects a works-detail call and an editions call"
        );
        for req in reqs.iter() {
            assert_eq!(req.rate_bucket, RateBucket::OpenLibrary);
            assert_eq!(req.method, HttpMethod::Get);
            assert!(matches!(req.user_agent, UserAgentProfile::Server));
            assert!(req.headers.is_empty());
            assert!(!req.anti_bot_check);
            assert_eq!(req.timeout, std::time::Duration::from_secs(30));
        }
        assert_eq!(reqs[0].url, "https://openlibrary.org/works/OL123W.json");
        assert_eq!(
            reqs[1].url,
            "https://openlibrary.org/works/OL123W/editions.json?limit=10"
        );
    }

    #[tokio::test]
    async fn query_ol_detail_strips_works_prefix_from_key() {
        let canned = serde_json::json!({"title": "T"});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        query_ol_detail(&fetcher, "/works/OL42W").await.unwrap();

        let reqs = fetcher.requests();
        assert_eq!(reqs[0].url, "https://openlibrary.org/works/OL42W.json");
    }

    // -------------------------------------------------------------------
    // Error mapping: non-2xx status and transport failures on the
    // work-detail call surface as an `Err(String)`; the editions call's
    // failures are always swallowed (isbn_13 stays None), matching the
    // pre-fetcher behavior of `if let Ok(..) = ...`.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn query_ol_detail_maps_http_404_to_error() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(404, vec![]);

        let err = query_ol_detail(&fetcher, "OL999W").await.unwrap_err();

        assert_eq!(err, "HTTP 404");
    }

    #[tokio::test]
    async fn query_ol_detail_maps_fetch_error_to_error_string() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            livrarr_domain::services::FetchError::Timeout(std::time::Duration::from_secs(30)),
        );

        let err = query_ol_detail(&fetcher, "OL999W").await.unwrap_err();

        assert!(err.contains("request failed"));
    }

    // -------------------------------------------------------------------
    // isbn_lookup: door-routing + the RateLimited-is-not-an-error nuance.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn isbn_lookup_sends_openlibrary_bucket_get_isbn_url() {
        let canned = serde_json::json!({"works": [{"key": "/works/OL42W"}]});
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let key = isbn_lookup(&fetcher, "9781234567890").await.unwrap();

        assert_eq!(key.as_deref(), Some("OL42W"));
        let reqs = fetcher.requests();
        assert_eq!(reqs.len(), 1);
        let req = &reqs[0];
        assert_eq!(req.url, "https://openlibrary.org/isbn/9781234567890.json");
        assert_eq!(req.rate_bucket, RateBucket::OpenLibrary);
        assert_eq!(req.method, HttpMethod::Get);
        assert!(matches!(req.user_agent, UserAgentProfile::Server));
        assert!(!req.anti_bot_check);
    }

    #[tokio::test]
    async fn isbn_lookup_maps_http_404_to_ok_none() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(404, vec![]);

        let key = isbn_lookup(&fetcher, "9781234567890").await.unwrap();

        assert_eq!(key, None);
    }

    #[tokio::test]
    async fn isbn_lookup_maps_fetcher_rate_limited_to_ok_none_not_error() {
        // The pre-fetcher code only ever checked `resp.status().is_success()`
        // — a 429 fell into the same "no work found" bucket as any other
        // non-success status, never a hard error. The fetcher now intercepts
        // 429 as a transport-level `FetchError::RateLimited` before a status
        // is ever seen; this must still land on `Ok(None)`, not `Err`
        // (an `Err` here would incorrectly surface as `WillRetry` one layer
        // up, in `fetch_by_anchor_query`).
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::RateLimited);

        let key = isbn_lookup(&fetcher, "9781234567890").await.unwrap();

        assert_eq!(key, None);
    }

    #[tokio::test]
    async fn isbn_lookup_maps_network_error_to_err() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            FetchError::Connection("refused".to_string()),
        );

        let err = isbn_lookup(&fetcher, "9781234567890").await.unwrap_err();

        assert!(err.contains("OL ISBN fetch failed"));
    }
}
