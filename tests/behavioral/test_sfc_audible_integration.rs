#![allow(dead_code, unused_imports)]

//! Behavioral tests for search-fallback-chain Audible integration contracts.

use assert_matches::assert_matches;
use livrarr_behavioral::stubs::StubHttpFetcher;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContractProvider {
    Hardcover,
    Goodreads,
    Audnexus,
    Audible,
}

fn audio_priority() -> Vec<ContractProvider> {
    vec![ContractProvider::Audible, ContractProvider::Audnexus]
}

fn content_priority() -> Vec<ContractProvider> {
    vec![
        ContractProvider::Hardcover,
        ContractProvider::Goodreads,
        ContractProvider::Audible,
    ]
}

fn not_yet_implemented() -> ! {
    todo!("search-fallback-chain Audible integration implementation is not yet wired")
}

/// REQ-IDs: REQ-014
/// AC-IDs: AC-011
/// Directive: Audible is highest priority for audio fields above Audnexus.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_integration_audio_fields_prioritize_audible_above_audnexus() {
    let priority = audio_priority();

    assert_eq!(priority[0], ContractProvider::Audible);
    assert_eq!(priority[1], ContractProvider::Audnexus);
    not_yet_implemented();
}

/// REQ-IDs: REQ-014
/// AC-IDs: AC-011
/// Directive: Audible is lower priority than HC/GR for non-audio fields.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_integration_content_fields_prioritize_hc_gr_above_audible() {
    let priority = content_priority();

    assert_eq!(priority[0], ContractProvider::Hardcover);
    assert_eq!(priority[1], ContractProvider::Goodreads);
    assert_eq!(priority[2], ContractProvider::Audible);
    not_yet_implemented();
}

/// REQ-IDs: REQ-018
/// AC-IDs: AC-011
/// Directive: Audible rate bucket interval is 150ms.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_integration_rate_bucket_audible_interval_150ms() {
    let _http = StubHttpFetcher::new();
    let interval = Duration::from_millis(150);

    assert_eq!(interval, Duration::from_millis(150));
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-015
/// AC-IDs: AC-012
/// Directive: Audible provider registration dispatches for English and foreign works.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_integration_registration_dispatches_for_all_work_languages() {
    let english_providers = [ContractProvider::Hardcover, ContractProvider::Audible];
    let foreign_providers = [ContractProvider::Audible];

    assert!(english_providers.contains(&ContractProvider::Audible));
    assert!(foreign_providers.contains(&ContractProvider::Audible));
    not_yet_implemented();
}

/// REQ-IDs: REQ-006, REQ-019
/// AC-IDs: AC-005, AC-014
/// Directive: Audible is eligible for pre-add cover alternatives.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_integration_cover_eligibility_includes_audible() {
    let eligible = [ContractProvider::Hardcover, ContractProvider::Audible];

    assert!(eligible.contains(&ContractProvider::Audible));
    assert_matches!(eligible[1], ContractProvider::Audible);
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: After enrichment, a work on Audible has asin, narrator, and duration_seconds populated.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_integration_enrichment_populates_audio_fields() {
    let asin = Some("B000FC0PBC");
    let narrator = Some(vec!["Scott Brick"]);
    let duration_seconds = Some(75_600);

    assert_eq!(asin, Some("B000FC0PBC"));
    assert_eq!(narrator.as_ref().map(|n| n.len()), Some(1));
    assert_eq!(duration_seconds, Some(75_600));
    not_yet_implemented();
}
