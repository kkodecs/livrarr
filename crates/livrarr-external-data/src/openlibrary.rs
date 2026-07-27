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

/// Report the single successful outcome for a complete OpenLibrary operation.
/// Request helpers never call this: only the outer caller that knows no later
/// provider leg will run may clear the breaker history.
pub(crate) fn report_openlibrary_success() {
    outbound_queue::shared().report_outcome(RateBucket::OpenLibrary, BreakerSignal::Success);
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
    /// True only when every request leg that ran completed successfully.
    /// The outer provider operation uses this to decide whether it may report
    /// the single breaker Success for the complete operation.
    pub(crate) all_legs_succeeded: bool,
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
        // Past the genuine-absence check, every non-2xx is a provider-health
        // signal — a 403/401 storm must trip the breaker, not just a 5xx.
        outbound_queue::shared().report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        return Err(classify_ol_error(resp.status));
    }

    let data: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|e| {
        outbound_queue::shared().report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        ProviderFetchError::Other(format!("parse: {e}"))
    })?;
    // No success report here. This call has a second leg (editions, below) and
    // `record_success` clears every accumulated failure (breaker.rs), so
    // reporting the work leg's success up front meant a permanently refused
    // editions endpoint produced Success, Failure, Success, Failure… and never
    // reached the threshold. One outcome per operation, reported at the end.
    let mut leg_failed = false;

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
    let editions_resp = fetcher.fetch(editions_req).await;
    if let Err(ref e) = editions_resp {
        // The transport has already reported this to the breaker
        // (`HttpFetcherImpl::do_fetch`), so this leg must not report a second
        // failure — but it MUST mark the leg failed, or the operation's own
        // success report below clears the one the transport just filed and a
        // permanently timing-out editions endpoint never reaches the threshold.
        tracing::warn!(key = %key, error = %e, "OL editions: transport failure");
        leg_failed = true;
    }
    if let Ok(ed_resp) = editions_resp {
        // This response's status was never inspected: a 401/403/5xx body simply
        // failed to yield `entries` and was skipped in silence, so a refused
        // editions endpoint could be re-requested forever without the breaker
        // ever learning. The call stays best-effort — a missing ISBN is not
        // fatal to the work fetch — so a refusal is reported and skipped rather
        // than failing the whole call.
        if !(200..300).contains(&ed_resp.status) {
            // No genuine-absence exemption on this route. The work itself just
            // answered 200, so its own editions sub-route returning 404/410
            // means the ROUTE is gone — a work with no editions is a 200 with
            // an empty `entries` array, never a 404.
            tracing::warn!(status = ed_resp.status, key = %key, "OL editions: HTTP error");
            leg_failed = true;
            outbound_queue::shared()
                .report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        } else if let Ok(ed_data) = serde_json::from_slice::<serde_json::Value>(&ed_resp.body) {
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
        } else {
            // A 200 whose body will not parse is the provider serving garbage,
            // not a work without editions — the same "unreadable is not absent"
            // rule the detail path applies. Silently skipping it let a broken
            // edge report the operation healthy on every call.
            tracing::warn!(key = %key, "OL editions: 200 with an unreadable body");
            leg_failed = true;
            outbound_queue::shared()
                .report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        }
    }
    let isbn_13 = title_and_language_isbn
        .or(title_only_isbn)
        .or(fallback_isbn_13);

    // Do not report Success here. Every production caller may compose this
    // helper with an ISBN or search leg; the outermost caller owns the single
    // operation outcome and consults `all_legs_succeeded` below.

    Ok(OlDetailResult {
        title,
        description,
        isbn_13,
        cover_id,
        all_legs_succeeded: !leg_failed,
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
        // Same rule as the work-detail path above: any non-2xx that is not a
        // genuine absence is a health signal.
        outbound_queue::shared().report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        return Err(classify_ol_error(resp.status));
    }

    let data: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|e| {
        outbound_queue::shared().report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        ProviderFetchError::Other(format!("OL ISBN parse error: {e}"))
    })?;
    // This is only an ISBN leg. Its caller may continue with work detail,
    // editions, or fuzzy search, so Success belongs at that outer boundary.

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
        // This search path reported nothing to the breaker at all. No 404/410
        // exemption: `search.json` has no "this book is absent" status — an
        // empty result set is a 200 with an empty `docs` array. A 404 here means
        // the ROUTE moved or is blocked, which is a provider failure; exempting
        // it reported every queried book as missing while the status dot stayed
        // green.
        outbound_queue::shared().report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        return Err(format!("OpenLibrary returned {}", resp.status));
    }

    let data: serde_json::Value = serde_json::from_slice(&resp.body).map_err(|e| {
        outbound_queue::shared().report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        format!("OpenLibrary parse error: {e}")
    })?;

    // A parsed response — including a legitimately empty `docs` array — is a
    // healthy answer. Without this the C4 Failure report above could open the
    // breaker and nothing on this door could ever close it again:
    // `record_success` is the only transition out of HalfOpen.
    outbound_queue::shared().report_outcome(RateBucket::OpenLibrary, BreakerSignal::Success);

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

    /// A provider operation may report at most ONE breaker outcome, and a
    /// success reported by one leg must not erase a sibling leg's failures:
    /// `record_success` clears every accumulated failure, so `work=200,
    /// editions=403` repeating forever produced Success, Failure, Success,
    /// Failure… and the count never reached the production threshold. The
    /// refused editions endpoint was re-requested indefinitely with the
    /// breaker permanently closed.
    ///
    /// Runs at the PRODUCTION threshold deliberately — a test that forces the
    /// threshold to 1 cannot observe this shape at all.
    #[tokio::test]
    async fn a_refused_editions_endpoint_eventually_opens_the_openlibrary_breaker() {
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();

        let work_body = serde_json::json!({"title": "Test Work"}).to_string();
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_ok(200, work_body.clone().into_bytes());
        // Alternating legs: the work endpoint always answers, the editions
        // endpoint always refuses. Well past any sane threshold.
        for _ in 0..12 {
            fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
                status: 403,
                headers: vec![],
                body: vec![],
            }));
            fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
                status: 200,
                headers: vec![],
                body: work_body.clone().into_bytes(),
            }));
        }

        for _ in 0..12 {
            let _ = query_ol_detail(&fetcher, "OL123W", RequestPriority::Normal, None, None).await;
        }

        // Scoped so the permit this may hand back is released immediately.
        let tripped = {
            let admission = queue
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };

        assert!(
            tripped,
            "a permanently refused editions endpoint must eventually open the \
             OpenLibrary breaker — a sibling leg's success must not erase it"
        );
    }

    /// Response-derived editions failures must file their own breaker signal.
    /// A threshold-one breaker makes the assertion load-bearing: deleting the
    /// production `Failure` report for either branch leaves the breaker closed.
    // Bug reproduction: subtitle-matching finding 4 — response failures were
    // previously masked by a manually injected fifth failure in the test.
    #[tokio::test]
    async fn response_derived_editions_failures_open_threshold_one_breaker() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();
        let work_body = serde_json::json!({"title": "Test Work"}).to_string();
        let editions_legs = [
            (
                "a 404 on the editions sub-route",
                livrarr_domain::services::FetchResponse {
                    status: 404,
                    headers: vec![],
                    body: vec![],
                },
            ),
            (
                "an editions 200 whose body will not parse",
                livrarr_domain::services::FetchResponse {
                    status: 200,
                    headers: vec![],
                    body: b"<html>not json</html>".to_vec(),
                },
            ),
        ];

        for (label, editions_leg) in editions_legs {
            queue.reset_breaker_for_tests(RateBucket::OpenLibrary);
            queue.set_breaker_config_for_tests(
                RateBucket::OpenLibrary,
                CircuitBreakerConfig {
                    failure_threshold: 1,
                    evaluation_window_secs: 60,
                    open_duration_secs: 60,
                    half_open_probe_count: 1,
                },
            );

            let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
                200,
                work_body.clone().into_bytes(),
            );
            fetcher.push_response(Ok(editions_leg));
            let result =
                query_ol_detail(&fetcher, "OL123W", RequestPriority::Normal, None, None).await;
            assert!(
                result.is_ok(),
                "{label}: editions stay best-effort for the payload"
            );

            let tripped = {
                let admission = queue
                    .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                    .await;
                matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
            };
            assert!(
                tripped,
                "{label}: the operation itself must report Failure and open a threshold-one breaker"
            );
        }
    }

    #[tokio::test]
    async fn query_ol_detail_invalid_json_opens_threshold_one_breaker() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();
        queue.set_breaker_config_for_tests(
            RateBucket::OpenLibrary,
            CircuitBreakerConfig {
                failure_threshold: 1,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );

        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, b"not json".to_vec());
        let result = query_ol_detail(&fetcher, "OL123W", RequestPriority::Normal, None, None).await;
        assert!(
            matches!(result, Err(ProviderFetchError::Other(message)) if message.starts_with("parse:")),
            "the existing parse error must be preserved"
        );

        let admission = queue
            .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
            .await;
        assert!(
            matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
            "the unreadable completed response must open a threshold-one breaker"
        );
    }

    /// The recording fetcher returns a synthetic timeout without reproducing
    /// `HttpFetcherImpl::do_fetch`'s transport-level Failure report. This case
    /// can therefore pin only that the outer operation emits no Success that
    /// would erase the real transport signal in production.
    #[tokio::test]
    async fn a_timed_out_editions_leg_must_not_report_operation_success() {
        use livrarr_domain::services::FetchError;
        use livrarr_http::breaker::BreakerSignal;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();
        for _ in 0..4 {
            queue.report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        }

        let work_body = serde_json::json!({"title": "Test Work"}).to_string();
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_ok(200, work_body.into_bytes());
        fetcher.push_response(Err(FetchError::Timeout(std::time::Duration::from_secs(30))));
        let result = query_ol_detail(&fetcher, "OL123W", RequestPriority::Normal, None, None).await;
        assert!(result.is_ok(), "editions stay best-effort for the payload");

        // Stand in only for the transport signal omitted by this test double.
        queue.report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        let tripped = {
            let admission = queue
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        assert!(
            tripped,
            "the outer operation must not report Success after a timed-out editions leg"
        );
    }

    #[tokio::test]
    async fn isbn_lookup_invalid_json_opens_threshold_one_breaker() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();
        queue.set_breaker_config_for_tests(
            RateBucket::OpenLibrary,
            CircuitBreakerConfig {
                failure_threshold: 1,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );

        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, b"not json".to_vec());
        let result = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal).await;
        assert!(
            matches!(result, Err(ProviderFetchError::Other(message)) if message.starts_with("OL ISBN parse error:")),
            "the existing ISBN parse error must be preserved"
        );

        let admission = queue
            .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
            .await;
        assert!(
            matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
            "the unreadable completed ISBN response must open a threshold-one breaker"
        );
    }

    /// The ISBN request is only the first leg of seeded and anchor operations.
    /// It must not clear accumulated failures before the resolved work/detail
    /// legs run; the caller owns the operation-level Success.
    // Bug reproduction: subtitle-matching finding 1 — ISBN helper Success
    // erased a later editions Failure on every composite invocation.
    #[tokio::test]
    async fn isbn_lookup_defers_success_to_the_operation_boundary() {
        use livrarr_http::breaker::BreakerSignal;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();
        for _ in 0..4 {
            queue.report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        }

        let body = serde_json::json!({"works": [{"key": "/works/OL123W"}]}).to_string();
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, body.into_bytes());
        let key = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .expect("a parseable ISBN response is healthy");
        assert_eq!(key.as_deref(), Some("OL123W"));

        queue.report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        let tripped = {
            let admission = queue
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        assert!(
            tripped,
            "the ISBN helper must not report Success before later provider legs finish"
        );
    }

    /// `query_ol_detail` is also a request helper for every production Open
    /// Library client door. Even with healthy work and editions responses, its
    /// caller must own the single operation-level Success.
    #[tokio::test]
    async fn query_ol_detail_defers_success_to_the_operation_boundary() {
        use livrarr_http::breaker::BreakerSignal;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();
        for _ in 0..4 {
            queue.report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        }

        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(
            200,
            serde_json::json!({"title": "Test Work"})
                .to_string()
                .into_bytes(),
        );
        fetcher.push_response(Ok(livrarr_domain::services::FetchResponse {
            status: 200,
            headers: vec![],
            body: serde_json::json!({"entries": []}).to_string().into_bytes(),
        }));
        let result = query_ol_detail(&fetcher, "OL123W", RequestPriority::Normal, None, None).await;
        assert!(result.is_ok());

        queue.report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        let tripped = {
            let admission = queue
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        assert!(
            tripped,
            "the detail helper must not report Success before its caller boundary"
        );
    }

    #[tokio::test]
    async fn search_openlibrary_invalid_json_opens_threshold_one_breaker() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();
        queue.set_breaker_config_for_tests(
            RateBucket::OpenLibrary,
            CircuitBreakerConfig {
                failure_threshold: 1,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );

        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, b"not json".to_vec());
        let result = search_openlibrary(&fetcher, "anything", "en").await;
        assert!(
            matches!(result, Err(message) if message.starts_with("OpenLibrary parse error:")),
            "the existing discovery parse error must be preserved"
        );

        let admission = queue
            .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
            .await;
        assert!(
            matches!(admission, Err(AdmissionError::CircuitOpen { .. })),
            "the unreadable completed discovery response must open a threshold-one breaker"
        );
    }

    /// C4 added Failure reporting to this door but no Success, and
    /// `record_success` is the ONLY transition out of HalfOpen
    /// (`breaker.rs`). A door that can open a breaker but never close it
    /// leaves recovery to whichever unrelated code path happens to run next.
    /// A legitimate empty result set IS a healthy answer and must say so.
    #[tokio::test]
    async fn a_healthy_search_result_reports_operation_success() {
        use livrarr_http::breaker::BreakerSignal;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();

        // One below the production threshold of 5.
        for _ in 0..4 {
            queue.report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);
        }

        let canned = serde_json::json!({"docs": []}).to_string();
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(200, canned.into_bytes());
        let results = search_openlibrary(&fetcher, "anything", "en")
            .await
            .expect("a 200 with an empty docs array is a healthy answer");
        assert!(results.is_empty());

        queue.report_outcome(RateBucket::OpenLibrary, BreakerSignal::Failure);

        let tripped = {
            let admission = queue
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };
        assert!(
            !tripped,
            "a healthy search must report Success and clear the accumulated \
             failures — otherwise this door can open the breaker but never close it"
        );
    }

    /// `search.json` has no "this book is absent" status: an empty result set
    /// is a 200 with an empty `docs` array. A 404 there means the ROUTE moved
    /// or is blocked, which is a provider-health event — exempting it reported
    /// every queried book as missing while the provider status stayed green.
    #[tokio::test]
    async fn a_search_route_404_is_a_provider_failure_not_a_book_miss() {
        use livrarr_http::breaker::CircuitBreakerConfig;
        use livrarr_http::outbound_queue::{self, AdmissionError};

        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let queue = outbound_queue::shared();
        queue.set_breaker_config_for_tests(
            RateBucket::OpenLibrary,
            CircuitBreakerConfig {
                failure_threshold: 1,
                evaluation_window_secs: 60,
                open_duration_secs: 60,
                half_open_probe_count: 1,
            },
        );

        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(404, vec![]);
        let err = search_openlibrary(&fetcher, "anything", "en").await;
        assert!(err.is_err(), "a 404 search route must not yield results");

        let tripped = {
            let admission = queue
                .acquire(RateBucket::OpenLibrary, RequestPriority::Normal)
                .await;
            matches!(admission, Err(AdmissionError::CircuitOpen { .. }))
        };

        assert!(
            tripped,
            "a 404 from the search route is a provider failure, not a book miss"
        );
    }

    // -------------------------------------------------------------------
    // Door-routing: query_ol_detail goes through the HttpFetcher trait with
    // the OpenLibrary rate bucket, GET, no auth, for both the work-detail
    // call and the editions call.
    // -------------------------------------------------------------------

    #[tokio::test]
    async fn query_ol_detail_sends_openlibrary_bucket_get_for_both_calls() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(404, vec![]);

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::NotFound));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_timeout_to_transient() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher =
            crate::test_support::RecordingHttpFetcher::with_error(FetchError::RateLimited);

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::RateLimited));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_fetcher_queue_full_to_queue_full() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(429, vec![]);

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::RateLimited));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_http_5xx_to_transient() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(503, vec![]);

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Transient));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_http_403_to_other() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(400, vec![]);

        let err = query_ol_detail(&fetcher, "OL999W", RequestPriority::Normal, None, None)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Other(_)));
    }

    #[tokio::test]
    async fn query_ol_detail_maps_http_410_to_not_found() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(404, vec![]);

        let key = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap();

        assert_eq!(key, None);
    }

    #[tokio::test]
    async fn isbn_lookup_maps_http_410_to_ok_none() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(429, vec![]);

        let err = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::RateLimited));
    }

    #[tokio::test]
    async fn isbn_lookup_maps_http_5xx_to_err_transient() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(500, vec![]);

        let err = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Transient));
    }

    #[tokio::test]
    async fn isbn_lookup_maps_http_403_to_err_other() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(403, vec![]);

        let err = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Other(_)));
    }

    #[tokio::test]
    async fn isbn_lookup_maps_other_4xx_to_err_other() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = crate::test_support::RecordingHttpFetcher::with_ok(400, vec![]);

        let err = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
            .await
            .unwrap_err();

        assert!(matches!(err, ProviderFetchError::Other(_)));
    }

    #[tokio::test]
    async fn isbn_lookup_maps_network_error_to_transient() {
        let _guard = crate::test_support::lock_breaker(RateBucket::OpenLibrary).await;
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
