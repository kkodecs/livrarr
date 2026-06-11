#![allow(dead_code, unused_imports)]

//! Behavioral tests for search-fallback-chain ASIN merge write contract.

use assert_matches::assert_matches;
use livrarr_db::test_helpers::create_test_db;
use livrarr_domain::OutcomeClass;

fn contract_resolved_asin_write(
    current_asin: Option<&str>,
    resolved_asin: Option<&str>,
    provider_outcome: OutcomeClass,
) -> Option<String> {
    match (resolved_asin, provider_outcome) {
        (Some(asin), _) => Some(asin.to_string()),
        (None, OutcomeClass::WillRetry) => current_asin.map(str::to_string),
        (None, _) => None,
    }
}

fn not_yet_implemented() -> ! {
    todo!("search-fallback-chain ASIN merge write implementation is not yet wired")
}

/// REQ-IDs: REQ-016
/// AC-IDs: AC-013
/// Directive: Re-enriching a work that has an ASIN retains the ASIN when Audnexus/Audible returns WillRetry.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_asin_fix_reenrich_will_retry_preserves_existing_asin() {
    let _db = create_test_db().await;
    let written = contract_resolved_asin_write(Some("B000FC0PBC"), None, OutcomeClass::WillRetry);

    assert_eq!(written.as_deref(), Some("B000FC0PBC"));
    assert_matches!(OutcomeClass::WillRetry, OutcomeClass::WillRetry);
    not_yet_implemented();
}

/// REQ-IDs: REQ-016
/// AC-IDs: AC-013
/// Directive: merge-resolved value must be written faithfully, not bare SET asin = ?.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_asin_fix_writes_merge_resolved_asin_value_faithfully() {
    let _db = create_test_db().await;
    let written =
        contract_resolved_asin_write(Some("OLDASIN"), Some("B000FC0PBC"), OutcomeClass::Success);

    assert_eq!(written.as_deref(), Some("B000FC0PBC"));
    assert_ne!(written.as_deref(), Some("OLDASIN"));
    not_yet_implemented();
}

/// REQ-IDs: REQ-016
/// AC-IDs: AC-013
/// Directive: resolved explicit None with no current value writes NULL.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_asin_fix_explicit_none_without_current_value_writes_null() {
    let _db = create_test_db().await;
    let written = contract_resolved_asin_write(None, None, OutcomeClass::NotFound);

    assert!(written.is_none());
    not_yet_implemented();
}
