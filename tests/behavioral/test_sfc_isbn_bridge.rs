#![allow(dead_code, unused_imports)]

//! Behavioral tests for search-fallback-chain ISBN bridge provider contracts.

use assert_matches::assert_matches;
use livrarr_behavioral::stubs::{StubHttpFetcher, StubLlmCaller};
use livrarr_domain::Work;
use livrarr_external_data::{NormalizedWorkDetail, ProviderOutcome};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContractOutcome {
    Success,
    NotFound,
    NotConfigured,
}

fn not_yet_implemented() -> ! {
    todo!("search-fallback-chain ISBN bridge implementation is not yet wired")
}

/// REQ-IDs: REQ-011, REQ-012
/// AC-IDs: AC-010
/// Directive: ISBN -> ISBN lookup succeeds -> Success.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_openlibrary_isbn_lookup_success_returns_success() {
    let _http = StubHttpFetcher::with_ok(200, br#"{"works":[{"key":"/works/OL1W"}]}"#.to_vec());
    let attempted_paths = vec!["isbn"];
    let outcome = ContractOutcome::Success;

    assert_eq!(attempted_paths, vec!["isbn"]);
    assert_matches!(outcome, ContractOutcome::Success);
    not_yet_implemented();
}

/// REQ-IDs: REQ-011, REQ-012
/// AC-IDs: AC-010
/// Directive: ISBN -> ISBN 404 -> ol_key succeeds -> Success.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_openlibrary_isbn_404_falls_back_to_ol_key_success() {
    let _http = StubHttpFetcher::with_ok(404, b"{}".to_vec());
    let attempted_paths = vec!["isbn", "ol_key"];

    assert_eq!(attempted_paths, vec!["isbn", "ol_key"]);
    assert!(attempted_paths.contains(&"ol_key"));
    not_yet_implemented();
}

/// REQ-IDs: REQ-011, REQ-012
/// AC-IDs: AC-010
/// Directive: ISBN -> ISBN 404 -> no ol_key -> title+author succeeds -> Success.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_openlibrary_isbn_404_no_ol_key_title_author_success() {
    let _http = StubHttpFetcher::with_ok(404, b"{}".to_vec());
    let attempted_paths = vec!["isbn", "title_author"];

    assert_eq!(attempted_paths, vec!["isbn", "title_author"]);
    assert_matches!(ContractOutcome::Success, ContractOutcome::Success);
    not_yet_implemented();
}

/// REQ-IDs: REQ-011, REQ-012
/// AC-IDs: AC-010
/// Directive: all three fail -> NotFound.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_openlibrary_all_paths_fail_returns_not_found() {
    let _http = StubHttpFetcher::with_ok(404, b"{}".to_vec());
    let attempted_paths = ["isbn", "ol_key", "title_author"];

    assert_eq!(attempted_paths.len(), 3);
    assert_matches!(ContractOutcome::NotFound, ContractOutcome::NotFound);
    not_yet_implemented();
}

/// REQ-IDs: REQ-012
/// AC-IDs: AC-010
/// Directive: no ISBN, has ol_key -> existing behavior.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_openlibrary_no_isbn_has_ol_key_uses_existing_behavior() {
    let _http = StubHttpFetcher::new();
    let attempted_paths = vec!["ol_key"];

    assert_eq!(attempted_paths, vec!["ol_key"]);
    not_yet_implemented();
}

/// REQ-IDs: REQ-012
/// AC-IDs: AC-010
/// Directive: no ISBN, no ol_key -> title+author search.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_openlibrary_no_isbn_no_ol_key_uses_title_author() {
    let _http = StubHttpFetcher::new();
    let attempted_paths = vec!["title_author"];

    assert_eq!(attempted_paths, vec!["title_author"]);
    not_yet_implemented();
}

/// REQ-IDs: REQ-011, REQ-012
/// AC-IDs: AC-010
/// Directive: ISBN with hyphens is normalized before URL.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_openlibrary_hyphenated_isbn_normalized_before_url() {
    let _http = StubHttpFetcher::new();
    let url = "https://openlibrary.org/isbn/9780441013593.json";

    assert!(url.ends_with("/isbn/9780441013593.json"));
    assert!(!url.contains("978-0-441"));
    not_yet_implemented();
}

/// REQ-IDs: REQ-011, REQ-012
/// AC-IDs: AC-010
/// Directive: ISBN -> HC finds match with ISBN in isbns -> Success.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_hardcover_isbn_match_in_isbns_success() {
    let _http = StubHttpFetcher::with_ok(200, br#"{"data":{"search":{"hits":[]}}}"#.to_vec());
    let verified_isbn_match = true;

    assert!(verified_isbn_match);
    assert_matches!(ContractOutcome::Success, ContractOutcome::Success);
    not_yet_implemented();
}

/// REQ-IDs: REQ-011, REQ-012
/// AC-IDs: AC-010
/// Directive: ISBN -> HC no match -> falls back to title+author.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_hardcover_isbn_no_match_falls_back_to_title_author() {
    let _http = StubHttpFetcher::with_ok(200, br#"{"data":{"search":{"hits":[]}}}"#.to_vec());
    let attempted_paths = vec!["isbn", "title_author"];

    assert_eq!(attempted_paths, vec!["isbn", "title_author"]);
    not_yet_implemented();
}

/// REQ-IDs: REQ-012
/// AC-IDs: AC-010
/// Directive: no ISBN -> existing behavior.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_hardcover_no_isbn_uses_existing_title_author_behavior() {
    let _http = StubHttpFetcher::new();
    let attempted_paths = vec!["title_author"];

    assert_eq!(attempted_paths, vec!["title_author"]);
    not_yet_implemented();
}

/// REQ-IDs: REQ-012
/// AC-IDs: AC-010
/// Directive: HC not configured -> NotConfigured.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_hardcover_not_configured_returns_not_configured() {
    let _http = StubHttpFetcher::new();

    assert_matches!(
        ContractOutcome::NotConfigured,
        ContractOutcome::NotConfigured
    );
    not_yet_implemented();
}

/// REQ-IDs: REQ-012
/// AC-IDs: AC-010
/// Directive: has gr_key -> direct lookup, ISBN not tried.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_goodreads_gr_key_direct_lookup_skips_isbn() {
    let _http = StubHttpFetcher::new();
    let _llm = StubLlmCaller::configured(r#"{"gr_key":"123"}"#);
    let attempted_paths = vec!["gr_key"];

    assert_eq!(attempted_paths, vec!["gr_key"]);
    assert!(!attempted_paths.contains(&"isbn"));
    not_yet_implemented();
}

/// REQ-IDs: REQ-011, REQ-012
/// AC-IDs: AC-010
/// Directive: no gr_key, has ISBN -> ISBN search finds result.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_goodreads_no_gr_key_isbn_search_finds_result() {
    let _http = StubHttpFetcher::with_ok(200, b"<html>result</html>".to_vec());
    let _llm = StubLlmCaller::configured(r#"{"selected":0}"#);
    let query = "isbn:9780441013593";

    assert_eq!(query, "isbn:9780441013593");
    assert_matches!(ContractOutcome::Success, ContractOutcome::Success);
    not_yet_implemented();
}

/// REQ-IDs: REQ-012
/// AC-IDs: AC-010
/// Directive: no gr_key, has ISBN -> ISBN empty -> falls back to title+author.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_goodreads_isbn_empty_falls_back_to_title_author() {
    let _http = StubHttpFetcher::with_ok(200, b"<html></html>".to_vec());
    let _llm = StubLlmCaller::configured(r#"{"selected":0}"#);
    let attempted_paths = vec!["isbn", "title_author"];

    assert_eq!(attempted_paths, vec!["isbn", "title_author"]);
    not_yet_implemented();
}

/// REQ-IDs: REQ-012, REQ-017
/// AC-IDs: AC-010
/// Directive: no gr_key, no ISBN -> title+author search as before.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_isbn_bridge_goodreads_no_gr_key_no_isbn_title_author_as_before() {
    let _http = StubHttpFetcher::new();
    let _llm = StubLlmCaller::configured(r#"{"selected":0}"#);
    let attempted_paths = vec!["title_author"];

    assert_eq!(attempted_paths, vec!["title_author"]);
    not_yet_implemented();
}
