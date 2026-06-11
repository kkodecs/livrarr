#![allow(dead_code, unused_imports)]

//! Behavioral tests for search-fallback-chain GB-first discovery contracts.

use assert_matches::assert_matches;
use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_domain::services::LookupResult;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContractLookupResult {
    title: String,
    author_name: String,
    cover_url: Option<String>,
    year_displayed: bool,
    source: String,
    isbn_13: Option<String>,
}

fn not_yet_implemented() -> ! {
    todo!("search-fallback-chain GB-first discovery implementation is not yet wired")
}

/// REQ-IDs: REQ-001, REQ-002
/// AC-IDs: AC-001
/// Directive: GB returns results -> returns immediately.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_gb_first_discovery_lookup_gb_results_return_immediately() {
    let _http = StubHttpFetcher::with_ok(200, b"{}".to_vec());
    let results = [ContractLookupResult {
        title: "Dune".into(),
        author_name: "Frank Herbert".into(),
        cover_url: Some("https://books.google.com/cover.jpg".into()),
        year_displayed: false,
        source: "google_books".into(),
        isbn_13: Some("9780441013593".into()),
    }];
    let attempted_providers = vec!["google_books"];

    assert_eq!(results[0].title, "Dune");
    assert_eq!(results[0].author_name, "Frank Herbert");
    assert!(results[0].cover_url.is_some());
    assert!(!results[0].year_displayed);
    assert_eq!(attempted_providers, vec!["google_books"]);
    not_yet_implemented();
}

/// REQ-IDs: REQ-002
/// AC-IDs: AC-002
/// Directive: GB empty -> OL returns -> OL results.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_gb_first_discovery_lookup_gb_empty_falls_back_to_ol_results() {
    let _http = StubHttpFetcher::with_ok(200, b"{}".to_vec());
    let attempted_providers = vec!["google_books", "open_library"];
    let result_source = "open_library";

    assert_eq!(attempted_providers, vec!["google_books", "open_library"]);
    assert_eq!(result_source, "open_library");
    not_yet_implemented();
}

/// REQ-IDs: REQ-002
/// AC-IDs: AC-002
/// Directive: GB+OL empty -> HC returns -> HC results.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_gb_first_discovery_lookup_gb_ol_empty_falls_back_to_hc_results() {
    let _http = StubHttpFetcher::with_ok(200, b"{}".to_vec());
    let attempted_providers = vec!["google_books", "open_library", "hardcover"];

    assert_eq!(
        attempted_providers,
        vec!["google_books", "open_library", "hardcover"]
    );
    assert_matches!(attempted_providers.last(), Some(&"hardcover"));
    not_yet_implemented();
}

/// REQ-IDs: REQ-002
/// AC-IDs: AC-002
/// Directive: all empty -> empty vec.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_gb_first_discovery_lookup_all_empty_returns_empty_vec() {
    let _http = StubHttpFetcher::with_ok(200, b"{}".to_vec());
    let results: Vec<ContractLookupResult> = vec![];

    assert!(results.is_empty());
    not_yet_implemented();
}

/// REQ-IDs: REQ-002, REQ-003
/// AC-IDs: AC-003
/// Directive: GB error -> falls through to OL.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_gb_first_discovery_lookup_gb_error_falls_through_to_ol() {
    let _http = StubHttpFetcher::with_ok(403, b"quota exhausted".to_vec());
    let attempted_providers = vec!["google_books", "open_library"];
    let warning_logged = true;

    assert!(warning_logged);
    assert_eq!(attempted_providers, vec!["google_books", "open_library"]);
    not_yet_implemented();
}

/// REQ-IDs: REQ-001, REQ-011
/// AC-IDs: AC-001, AC-010
/// Directive: GB result with ISBN-13 -> isbn_13 populated.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_gb_first_discovery_google_books_isbn_13_populated() {
    let result = ContractLookupResult {
        title: "Dune".into(),
        author_name: "Frank Herbert".into(),
        cover_url: None,
        year_displayed: false,
        source: "google_books".into(),
        isbn_13: Some("9780441013593".into()),
    };

    assert_eq!(result.source, "google_books");
    assert_eq!(result.isbn_13.as_deref(), Some("9780441013593"));
    not_yet_implemented();
}

/// REQ-IDs: REQ-001, REQ-011
/// AC-IDs: AC-010
/// Directive: GB result with ISBN-10 only -> isbn_13 via conversion.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_gb_first_discovery_google_books_isbn_10_converted_to_isbn_13() {
    let converted = "9780441172719";

    assert_eq!(converted.len(), 13);
    assert!(converted.starts_with("978"));
    not_yet_implemented();
}

/// REQ-IDs: REQ-001, REQ-011
/// AC-IDs: AC-001
/// Directive: no identifiers -> isbn_13 is None.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_gb_first_discovery_google_books_no_identifiers_isbn_13_none() {
    let result = ContractLookupResult {
        title: "Dune".into(),
        author_name: "Frank Herbert".into(),
        cover_url: None,
        year_displayed: false,
        source: "google_books".into(),
        isbn_13: None,
    };

    assert!(result.isbn_13.is_none());
    not_yet_implemented();
}

/// REQ-IDs: REQ-002, REQ-011
/// AC-IDs: AC-002, AC-010
/// Directive: HC configured -> returns mapped results with isbn_13.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_gb_first_discovery_hardcover_configured_returns_mapped_isbn_13_results() {
    let _http = StubHttpFetcher::with_ok(200, b"{}".to_vec());
    let result = ContractLookupResult {
        title: "Dune".into(),
        author_name: "Frank Herbert".into(),
        cover_url: Some("https://images.hardcover.app/dune.jpg".into()),
        year_displayed: false,
        source: "hardcover".into(),
        isbn_13: Some("9780441013593".into()),
    };

    assert_eq!(result.source, "hardcover");
    assert_eq!(result.isbn_13.as_deref(), Some("9780441013593"));
    not_yet_implemented();
}

/// REQ-IDs: REQ-002
/// AC-IDs: AC-002
/// Directive: HC not configured -> empty vec.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_gb_first_discovery_hardcover_not_configured_returns_empty_vec() {
    let _http = StubHttpFetcher::new();
    let results: Vec<ContractLookupResult> = vec![];

    assert!(results.is_empty());
    not_yet_implemented();
}

/// REQ-IDs: REQ-002
/// AC-IDs: AC-002
/// Directive: HC error -> Err propagated for fallback.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_gb_first_discovery_hardcover_error_propagated_for_fallback() {
    let _http = StubHttpFetcher::with_ok(500, b"server error".to_vec());
    let fallback_should_continue = true;

    assert!(fallback_should_continue);
    not_yet_implemented();
}
