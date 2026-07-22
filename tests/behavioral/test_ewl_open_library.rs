#![allow(dead_code, unused_imports)]
//! Behavioral tests for english-work-lifecycle `open_library` directives.

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_domain::services::FetchError;
use livrarr_domain::RequestPriority;
use livrarr_external_data::openlibrary::{isbn_lookup, query_ol_detail};
use livrarr_external_data::types::ProviderFetchError;

/// REQ-IDs: REQ-003, REQ-010
/// Directive: 200 normal: OlResult { resolved_key=ol_key, redirected_from=None }.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_fetch_work_200_normal_olresult_resolved_key_ol_key_redirected() {
    todo!()
}

/// REQ-IDs: REQ-003, REQ-010
/// Directive: 301 to /works/OL2W.json: OlResult { resolved_key='OL2W', redirected_from=Some('OL1W'), data=follow body }; structured log fired.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_fetch_work_301_works_ol2w_json_olresult_resolved_key_ol2w_redirected(
) {
    todo!()
}

/// REQ-IDs: REQ-003, REQ-010
/// Directive: In-payload location='/works/OL2W': same as 301 case.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_fetch_work_payload_location_works_ol2w_same_301_case() {
    todo!()
}

/// REQ-IDs: REQ-003, REQ-010
/// Directive: Two consecutive 429s with Retry-After 5: sleeps 5s, retries once, second 429 → WillRetry(RateLimited).
///
/// Unit A scope note: the approved Unit A design fixes OL's 429 handling to
/// a fixed 6h+jitter backoff (mirroring `google_books::map_http_error`), not
/// a `Retry-After`-driven sleep-and-retry loop — no such loop exists at any
/// layer of this stack (verified: `HttpFetcherImpl::do_fetch` intercepts a
/// 429 and returns immediately). This test verifies the part that IS Unit
/// A's invariant: `query_ol_detail` ("fetch work") must surface a live 429 as
/// a retryable `ProviderFetchError::RateLimited`, not swallow it, making
/// exactly one HTTP call.
#[tokio::test]
async fn test_ewl_open_library_fetch_work_consecutive_429s_retry_after_5_sleeps_5s_retries_once() {
    let fetcher = StubHttpFetcher::with_error(FetchError::RateLimited);

    let err = query_ol_detail(&fetcher, "OL1W", RequestPriority::Normal, None, None)
        .await
        .unwrap_err();

    assert!(matches!(err, ProviderFetchError::RateLimited));
    assert_eq!(fetcher.call_count(), 1);
}

/// REQ-IDs: REQ-003, REQ-010
/// Directive: Rate-limit pacing: 100 sequential calls anonymous take >= 100s (1 rps cap).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_fetch_work_rate_limit_pacing_100_sequential_calls_anonymous_take_ge()
{
    todo!()
}

/// REQ-IDs: REQ-003, REQ-010
/// Directive: Rate-limit pacing: 100 sequential calls identified take >= 33s (3 rps cap).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_fetch_work_rate_limit_pacing_100_sequential_calls_identified_take_ge(
) {
    todo!()
}

/// REQ-IDs: REQ-003, REQ-010
/// Directive: UA assertion: anonymous → 'Livrarr/<v>'; identified → 'Livrarr/<v> (<email>)'.
#[tokio::test]
#[ignore = "stage-5: requires implementation or integration infrastructure not available in unit tests"]
async fn test_ewl_open_library_fetch_work_ua_assertion_anonymous_livrarr_v_identified_livrarr_v_email(
) {
    todo!()
}

/// REQ-IDs: REQ-003, REQ-010
/// Directive: Redirect loop (Location → same key): WillRetry; circuit breaker logs.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_fetch_work_redirect_loop_location_same_key_willretry_circuit_breaker_logs(
) {
    todo!()
}

/// REQ-IDs: REQ-003
/// Directive: Mock OL returns {works: [{key: '/works/OL1W'}]}: returns Ok(Some('OL1W')).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_isbn_to_work_mock_ol_works_key_works_ol1w_ol1w() {
    todo!()
}

/// REQ-IDs: REQ-003
/// Directive: Mock OL 404: returns Ok(None).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_isbn_to_work_mock_ol_404() {
    todo!()
}

/// REQ-IDs: REQ-003
/// Directive: Mock OL 200 with no works field: returns Ok(None).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_isbn_to_work_mock_ol_200_no_works_field() {
    todo!()
}

/// REQ-IDs: REQ-003
/// Directive: Mock OL two 429s: returns Err(RateLimited).
#[tokio::test]
async fn test_ewl_open_library_isbn_to_work_mock_ol_429s_err_ratelimited() {
    let fetcher = StubHttpFetcher::with_error(FetchError::RateLimited);

    let err = isbn_lookup(&fetcher, "9781234567890", RequestPriority::Normal)
        .await
        .unwrap_err();

    assert!(matches!(err, ProviderFetchError::RateLimited));
}

/// REQ-IDs: REQ-003
/// Directive: Mock OL returns 5 docs: returns 5 OlSearchHit objects in order.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_search_works_mock_ol_5_docs_5_olsearchhit_objects_order() {
    todo!()
}

/// REQ-IDs: REQ-003
/// Directive: Mock OL returns 0 docs: returns empty Vec.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_search_works_mock_ol_0_docs_empty_vec() {
    todo!()
}

/// REQ-IDs: REQ-003
/// Directive: Mock OL 500: returns Err(Transient).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_search_works_mock_ol_500_err_transient() {
    todo!()
}

/// REQ-IDs: REQ-013
/// Directive: Closed by default; opens after configured consecutive failures (existing test infrastructure).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_open_library_circuit_state_closed_default_opens_after_configured_consecutive_failures_existing_test(
) {
    todo!()
}
