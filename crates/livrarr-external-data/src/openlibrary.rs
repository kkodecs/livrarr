//! OpenLibrary REST client, consumed via `ProviderClient::OpenLibrary` (queue
//! dispatch and the identity-resolution fan-out) and by the discovery path
//! (`search_openlibrary`).

use livrarr_domain::identity_matching;
use livrarr_domain::seed::iso639_1_to_3;
use livrarr_domain::services::{
    FetchError, FetchRequest, HttpFetcher, HttpMethod, LookupResult, RateBucket, UserAgentProfile,
};
use livrarr_domain::RequestPriority;
use livrarr_http::breaker::BreakerSignal;
use livrarr_http::outbound_queue;

use crate::types::ProviderFetchError;

/// Classify a non-2xx, non-404/410 OpenLibrary response status (Unit A). The
/// single place this decision is made, shared by `isbn_lookup` and
/// `query_ol_detail`, so the anchor (`detail_by_key`) and seeded entry paths
/// built on top of them cannot classify the same status differently.
pub(crate) fn classify_ol_error(status: u16) -> ProviderFetchError {
    match status {
        429 => ProviderFetchError::RateLimited,
        500..=599 => ProviderFetchError::Transient,
        _ => ProviderFetchError::Other(format!("HTTP {status}")),
    }
}

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

pub async fn query_ol_detail<F: HttpFetcher>(
    fetcher: &F,
    ol_key: &str,
    priority: RequestPriority,
    preferred_language: Option<&str>,
    preferred_title: Option<&str>,
) -> Result<OlDetailResult, ProviderFetchError> {
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
        priority,
    };
    let resp = match fetcher.fetch(req).await {
        Ok(r) => r,
        Err(FetchError::RateLimited) => return Err(ProviderFetchError::RateLimited),
        Err(FetchError::CircuitOpen { retry_after }) => {
            return Err(ProviderFetchError::CircuitOpen(retry_after));
        }
        Err(FetchError::QueueFull { retry_after }) => {
            return Err(ProviderFetchError::QueueFull(retry_after));
        }
        // A transport layer that represents an HTTP status as a distinct
        // error (rather than a normal response) still carries a real status
        // — classify it exactly like one, so a wrapped 429/5xx doesn't
        // silently outrank the un-wrapped case.
        Err(FetchError::HttpError { status, .. }) => return Err(classify_ol_error(status)),
        Err(e) => {
            tracing::debug!(ol_key = %ol_key, error = %e, "OL work detail: transport failure");
            return Err(ProviderFetchError::Transient);
        }
    };

    if !(200..300).contains(&resp.status) {
        if resp.status == 404 || resp.status == 410 {
            return Err(ProviderFetchError::NotFound);
        }
        if (500..600).contains(&resp.status) {
            outbound_queue::shared()
                .report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        }
        return Err(classify_ol_error(resp.status));
    }

    let data: serde_json::Value = serde_json::from_slice(&resp.body)
        .map_err(|e| ProviderFetchError::Other(format!("parse: {e}")))?;
    outbound_queue::shared().report_outcome(RateBucket::OpenLibrary, BreakerSignal::Success);

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

    // Fetch editions for ISBN. A work's edition list spans every language AND
    // every variant (Young Readers / abridged / anniversary retellings all
    // ride the same OL work) — picking the first edition with any ISBN can
    // land on a different product entirely (the live bug: a language-only
    // preference still picked "Code Breaker -- Young Readers Edition" over
    // the actual book). Preference order: an edition whose OWN title matches
    // the one we're resolving for (via the shared title-matching authority —
    // catches the Young Readers/abridged case regardless of language), tie-
    // broken by a matching language when more than one title-matching
    // edition exists; otherwise the first edition carrying any ISBN, exactly
    // as before — the safety net when nothing lines up.
    let preferred_ol_lang = preferred_language.map(iso639_1_to_3);
    let preferred_parsed_title = preferred_title
        .filter(|t| !t.trim().is_empty())
        .map(identity_matching::parse_title);
    let mut title_and_language_isbn = None;
    let mut title_only_isbn = None;
    let mut fallback_isbn_13 = None;
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
        priority,
    };
    if let Ok(ed_resp) = fetcher.fetch(editions_req).await {
        if let Ok(ed_data) = serde_json::from_slice::<serde_json::Value>(&ed_resp.body) {
            if let Some(entries) = ed_data.get("entries").and_then(|e| e.as_array()) {
                for entry in entries {
                    let Some(isbn) = entry
                        .get("isbn_13")
                        .and_then(|i| i.as_array())
                        .and_then(|isbns| isbns.first())
                        .and_then(|v| v.as_str())
                    else {
                        continue;
                    };
                    if fallback_isbn_13.is_none() {
                        fallback_isbn_13 = Some(isbn.to_string());
                    }

                    let title_matches = match (
                        &preferred_parsed_title,
                        entry.get("title").and_then(|t| t.as_str()),
                    ) {
                        (Some(want), Some(edition_title)) if !edition_title.trim().is_empty() => {
                            identity_matching::title_verdict(
                                want,
                                &identity_matching::parse_title(edition_title),
                            ) == identity_matching::TitleVerdict::Same
                        }
                        _ => false,
                    };
                    if !title_matches {
                        continue;
                    }
                    if title_only_isbn.is_none() {
                        title_only_isbn = Some(isbn.to_string());
                    }

                    let matches_language = preferred_ol_lang.is_some_and(|want| {
                        let want_key = format!("/languages/{want}");
                        entry
                            .get("languages")
                            .and_then(|l| l.as_array())
                            .is_some_and(|langs| {
                                langs.iter().any(|l| {
                                    l.get("key").and_then(|k| k.as_str()) == Some(want_key.as_str())
                                })
                            })
                    });
                    if matches_language {
                        title_and_language_isbn = Some(isbn.to_string());
                        break;
                    }
                }
            }
        }
    }
    let isbn_13 = title_and_language_isbn
        .or(title_only_isbn)
        .or(fallback_isbn_13);

    Ok(OlDetailResult {
        title,
        description,
        isbn_13,
        cover_id,
    })
}

/// Resolve an ISBN-13 to its OpenLibrary work key via the ISBN lookup
/// endpoint. A genuine "no work found" (HTTP 404/410) maps to `Ok(None)` so
/// callers may fall through to a weaker tier. A live 429 or 5xx is retryable,
/// not a permanent miss (Unit A): both surface as a typed `Err` —
/// `RateLimited` / `Transient` — so the caller can schedule a real retry
/// instead of silently dropping the ISBN. This includes a fetcher-intercepted
/// HTTP 429 (`FetchError::RateLimited`), which the pre-Unit-A code folded
/// into the same "no work found" outcome. Any other transport-level failure
/// (network, timeout, body) is a hard `Err` too.
pub async fn isbn_lookup<F: HttpFetcher>(
    fetcher: &F,
    isbn: &str,
    priority: RequestPriority,
) -> Result<Option<String>, ProviderFetchError> {
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
        priority,
    };
    let resp = match fetcher.fetch(req).await {
        Ok(r) => r,
        Err(FetchError::RateLimited) => return Err(ProviderFetchError::RateLimited),
        Err(FetchError::CircuitOpen { retry_after }) => {
            return Err(ProviderFetchError::CircuitOpen(retry_after));
        }
        Err(FetchError::QueueFull { retry_after }) => {
            return Err(ProviderFetchError::QueueFull(retry_after));
        }
        Err(FetchError::HttpError { status, .. }) => return Err(classify_ol_error(status)),
        Err(e) => {
            tracing::debug!(isbn = isbn, error = %e, "OL ISBN fetch: transport failure");
            return Err(ProviderFetchError::Transient);
        }
    };

    if !(200..300).contains(&resp.status) {
        if resp.status == 404 || resp.status == 410 {
            return Ok(None);
        }
        if (500..600).contains(&resp.status) {
            outbound_queue::shared()
                .report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        }
        return Err(classify_ol_error(resp.status));
    }

    let data: serde_json::Value = serde_json::from_slice(&resp.body)
        .map_err(|e| ProviderFetchError::Other(format!("OL ISBN parse error: {e}")))?;
    outbound_queue::shared().report_outcome(RateBucket::OpenLibrary, BreakerSignal::Success);

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

        let result = query_ol_detail(&fetcher, "OL123W", RequestPriority::Normal, None, None)
            .await
            .unwrap();

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

        query_ol_detail(
            &fetcher,
            "/works/OL42W",
            RequestPriority::Normal,
            None,
            None,
        )
        .await
        .unwrap();

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
    async fn query_ol_detail_maps_http_404_to_not_found() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(404, vec![]);

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::NotFound));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_timeout_to_transient() {
        // Unit A: a transport-level timeout is retryable (Transient), not an
        // opaque permanent failure.
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            livrarr_domain::services::FetchError::Timeout(std::time::Duration::from_secs(30)),
        );

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Transient));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_wrapped_http_status_to_same_classification_as_raw_status() {
        // Some transport layers represent an HTTP status as a distinct
        // `FetchError::HttpError` rather than a normal response. A 429/5xx
        // wrapped this way must classify identically to the un-wrapped case.
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            livrarr_domain::services::FetchError::HttpError {
                status: 503,
                classification: "server_error".to_string(),
            },
        );

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Transient));
    }

    // -------------------------------------------------------------------
    // Unit A: a live 429/5xx on the work-detail call must be retryable, not
    // folded into an opaque `Other` (which the anchor path `detail_by_key`
    // used to collapse straight into a permanent `NotFound`).
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn query_ol_detail_maps_fetcher_rate_limited_to_rate_limited() {
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::RateLimited);

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::RateLimited));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_fetcher_queue_full_to_queue_full() {
        // D3/#6: the outbound queue's local admission cap (no HTTP attempted)
        // must surface as a typed QueueFull, not fall into the generic
        // Transient catch-all (which would silently consume retry budget).
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::QueueFull {
                retry_after: std::time::Duration::from_secs(1),
            });

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::QueueFull(_)));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_http_429_status_to_rate_limited() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(429, vec![]);

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::RateLimited));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_http_5xx_to_transient() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(503, vec![]);

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Transient));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_http_403_to_other() {
        // OL is keyless: a 403 has no credential to fix. It is NOT
        // `NotFound` (unchanged-behavior tests cover that) and the crate
        // does not construct `NotConfigured` for OL at all — the caller
        // (`provider_client::ol_error_outcome`) turns this into an explicit
        // `PermanentFailure`, never a silent "not configured."
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(403, vec![]);

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Other(_)));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_other_4xx_to_other() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(400, vec![]);

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Other(_)));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_http_410_to_not_found() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(410, vec![]);

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::NotFound));
    }

    #[test]
    fn classify_ol_error_maps_429_to_rate_limited() {
        assert!(matches!(
            classify_ol_error(429),
            ProviderFetchError::RateLimited
        ));
    }

    #[test]
    fn classify_ol_error_maps_5xx_to_transient() {
        assert!(matches!(
            classify_ol_error(500),
            ProviderFetchError::Transient
        ));
        assert!(matches!(
            classify_ol_error(503),
            ProviderFetchError::Transient
        ));
        assert!(matches!(
            classify_ol_error(599),
            ProviderFetchError::Transient
        ));
    }

    #[test]
    fn classify_ol_error_maps_403_and_other_4xx_to_other() {
        assert!(matches!(
            classify_ol_error(403),
            ProviderFetchError::Other(_)
        ));
        assert!(matches!(
            classify_ol_error(400),
            ProviderFetchError::Other(_)
        ));
        assert!(matches!(
            classify_ol_error(451),
            ProviderFetchError::Other(_)
        ));
    }

    // -------------------------------------------------------------------
    // ISBN selection: a work's edition list spans every language AND every
    // variant (Young Readers/abridged retellings ride the same OL work) —
    // the live bug reproduced below with the real edition shape for this
    // work: the Portuguese edition sorts first, then an English "Young
    // Readers Edition" (a different, abridged product), then the actual
    // English edition further down.
    // -------------------------------------------------------------------

    fn code_breaker_editions_canned() -> serde_json::Value {
        serde_json::json!({
            "entries": [
                {
                    "title": "A decodificadora",
                    "isbn_13": ["9786555601824"],
                    "languages": [{"key": "/languages/por"}]
                },
                {
                    "title": "Code Breaker -- Young Readers Edition",
                    "isbn_13": ["9781665910682"],
                    "languages": [{"key": "/languages/eng"}]
                },
                {
                    "title": "The Code Breaker",
                    "isbn_13": ["9781982115852"],
                    "languages": [{"key": "/languages/eng"}]
                }
            ]
        })
    }

    #[tokio::test]
    async fn query_ol_detail_prefers_title_and_language_match_over_wrong_title_same_language_edition(
    ) {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            code_breaker_editions_canned().to_string().into_bytes(),
        );

        let result = query_ol_detail(
            &fetcher,
            "OL24217656W",
            RequestPriority::Normal,
            Some("en"),
            Some("The Code Breaker"),
        )
        .await
        .unwrap();

        assert_eq!(
            result.isbn_13.as_deref(),
            Some("9781982115852"),
            "must skip the language-matching but wrongly-titled Young Readers edition"
        );
    }

    #[tokio::test]
    async fn query_ol_detail_prefers_title_match_even_when_edition_has_no_language_tag() {
        let canned = serde_json::json!({
            "entries": [
                {
                    "title": "Code Breaker -- Young Readers Edition",
                    "isbn_13": ["9781665910682"],
                    "languages": [{"key": "/languages/eng"}]
                },
                {
                    "title": "The Code Breaker",
                    "isbn_13": ["9781797147338"],
                    "languages": []
                }
            ]
        });
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            canned.to_string().into_bytes(),
        );

        let result = query_ol_detail(
            &fetcher,
            "OL24217656W",
            RequestPriority::Normal,
            Some("en"),
            Some("The Code Breaker"),
        )
        .await
        .unwrap();

        assert_eq!(
            result.isbn_13.as_deref(),
            Some("9781797147338"),
            "a matching title outranks a matching-language wrong title, even with no language tag of its own"
        );
    }

    #[tokio::test]
    async fn query_ol_detail_falls_back_to_first_isbn_when_no_title_given() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            code_breaker_editions_canned().to_string().into_bytes(),
        );

        let result = query_ol_detail(&fetcher, "OL24217656W", RequestPriority::Normal, None, None)
            .await
            .unwrap();

        assert_eq!(result.isbn_13.as_deref(), Some("9786555601824"));
    }

    #[tokio::test]
    async fn query_ol_detail_falls_back_to_first_isbn_when_no_edition_title_matches() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            code_breaker_editions_canned().to_string().into_bytes(),
        );

        let result = query_ol_detail(
            &fetcher,
            "OL24217656W",
            RequestPriority::Normal,
            Some("en"),
            Some("Some Entirely Different Title"),
        )
        .await
        .unwrap();

        assert_eq!(result.isbn_13.as_deref(), Some("9786555601824"));
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

        let key = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap();

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

        let key = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap();

        assert_eq!(key, None);
    }

    #[tokio::test]
    async fn isbn_lookup_maps_http_410_to_ok_none() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(410, vec![]);

        let key = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap();

        assert_eq!(key, None);
    }

    // -------------------------------------------------------------------
    // Unit A: a live 429/5xx must be retryable, not folded into "no work
    // found" (the bug: `isbn_lookup` used to fold a 429 into `Ok(None)`,
    // which `fetch_by_anchor_query`'s `Isbn13` arm reads as a genuine miss
    // — permanently dropping the book's metadata on a transient rate-limit).
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn isbn_lookup_maps_fetcher_rate_limited_to_err_rate_limited() {
        // The fetcher intercepts HTTP 429 as a transport-level
        // `FetchError::RateLimited` before a status is ever seen. This must
        // now surface as `Err(RateLimited)` so the caller can schedule a
        // real retry (`WillRetry { RateLimit }`) instead of treating a live
        // rate-limit as a permanent "no work found."
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::RateLimited);

        let err = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::RateLimited));
    }

    #[tokio::test]
    async fn isbn_lookup_maps_fetcher_queue_full_to_queue_full() {
        // D3/#6: same local-admission-cap rule as query_ol_detail above —
        // must not fold into the generic Transient (budget-consuming) path.
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::QueueFull {
                retry_after: std::time::Duration::from_secs(1),
            });

        let err = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::QueueFull(_)));
    }

    #[tokio::test]
    async fn isbn_lookup_maps_http_429_status_to_err_rate_limited() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(429, vec![]);

        let err = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::RateLimited));
    }

    #[tokio::test]
    async fn isbn_lookup_maps_http_5xx_to_err_transient() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(500, vec![]);

        let err = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Transient));
    }

    #[tokio::test]
    async fn isbn_lookup_maps_http_403_to_err_other() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(403, vec![]);

        let err = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Other(_)));
    }

    #[tokio::test]
    async fn isbn_lookup_maps_other_4xx_to_err_other() {
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(400, vec![]);

        let err = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Other(_)));
    }

    #[tokio::test]
    async fn isbn_lookup_maps_network_error_to_transient() {
        // Unit A: a connection failure is retryable (Transient), not an
        // opaque permanent failure.
        let fetcher = crate::test_support::RecordingHttpFetcher::with_error(
            FetchError::Connection("refused".to_string()),
        );

        let err = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Transient));
    }
}
