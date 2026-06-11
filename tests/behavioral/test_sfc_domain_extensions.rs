#![allow(dead_code, unused_imports)]

//! Behavioral tests for search-fallback-chain domain extension contracts.

use assert_matches::assert_matches;
use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_domain::services::RateBucket;
use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractPreaddCoverCandidate {
    proxy_url: String,
    source: String,
    title: String,
    author_name: String,
}

fn contract_proxy_cover_url(raw_url: &str) -> String {
    if raw_url.starts_with('/') {
        raw_url.to_string()
    } else {
        format!("/api/v1/coverproxy?url={}", urlencoding::encode(raw_url))
    }
}

fn contract_normalize_isbn(isbn: &str) -> String {
    isbn.chars().filter(|c| c.is_alphanumeric()).collect()
}

fn not_yet_implemented() -> ! {
    todo!("search-fallback-chain domain extension implementation is not yet wired")
}

/// REQ-IDs: REQ-005, REQ-006, REQ-019
/// AC-IDs: AC-005, AC-014
/// Directive: PreaddCoverCandidate serializes to camelCase JSON with proxy_url field.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_domain_extensions_preadd_cover_candidate_serializes_camel_case_proxy_url() {
    let _http = StubHttpFetcher::new();
    let candidate = ContractPreaddCoverCandidate {
        proxy_url: "/api/v1/coverproxy?url=https%3A%2F%2Fimages.example%2Fcover.jpg".into(),
        source: "audible".into(),
        title: "Dune".into(),
        author_name: "Frank Herbert".into(),
    };

    let value = match serde_json::to_value(&candidate) {
        Ok(value) => value,
        Err(err) => panic!("candidate should serialize: {err}"),
    };

    assert_eq!(
        value["proxyUrl"],
        "/api/v1/coverproxy?url=https%3A%2F%2Fimages.example%2Fcover.jpg"
    );
    assert!(value.get("proxy_url").is_none());
    assert_eq!(value["authorName"], "Frank Herbert");
    not_yet_implemented();
}

/// REQ-IDs: REQ-006, REQ-019
/// AC-IDs: AC-014
/// Directive: external URL gets proxied.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_domain_extensions_proxy_cover_url_external_url_gets_proxied() {
    let proxied = contract_proxy_cover_url("https://m.media-amazon.com/images/I/dune.jpg");

    assert!(proxied.starts_with("/api/v1/coverproxy?url="));
    assert!(proxied.contains("https%3A%2F%2Fm.media-amazon.com"));
    assert!(!proxied.contains("https://m.media-amazon.com"));
    not_yet_implemented();
}

/// REQ-IDs: REQ-006, REQ-019
/// AC-IDs: AC-014
/// Directive: URL starting with / returned unchanged.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_domain_extensions_proxy_cover_url_local_path_returned_unchanged() {
    let local = contract_proxy_cover_url("/api/v1/work/41/cover");

    assert_eq!(local, "/api/v1/work/41/cover");
    assert_matches!(local.as_str(), "/api/v1/work/41/cover");
    not_yet_implemented();
}

/// REQ-IDs: REQ-011, REQ-012
/// AC-IDs: AC-010
/// Directive: '978-0-441-01359-3' -> '9780441013593'.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_domain_extensions_normalize_isbn_strips_hyphens() {
    assert_eq!(
        contract_normalize_isbn("978-0-441-01359-3"),
        "9780441013593"
    );
    not_yet_implemented();
}

/// REQ-IDs: REQ-011, REQ-012
/// AC-IDs: AC-010
/// Directive: '978 0441013593' -> '9780441013593'.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_domain_extensions_normalize_isbn_strips_spaces() {
    assert_eq!(contract_normalize_isbn("978 0441013593"), "9780441013593");
    not_yet_implemented();
}

/// REQ-IDs: REQ-011, REQ-012
/// AC-IDs: AC-010
/// Directive: already clean ISBN unchanged.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_domain_extensions_normalize_isbn_clean_isbn_unchanged() {
    assert_eq!(contract_normalize_isbn("9780441013593"), "9780441013593");
    not_yet_implemented();
}
