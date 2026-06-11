#![allow(dead_code, unused_imports)]

//! Behavioral tests for search-fallback-chain async add handler contracts.

use assert_matches::assert_matches;
use livrarr_behavioral::stubs::StubEnrichmentWorkflow;
use livrarr_domain::identity::{WorkCandidate, WorkSeed, WorkSeedFields};
use livrarr_domain::{EnrichmentStatus, Work};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ContractAddWorkRequest {
    title: String,
    author_name: String,
    cover_url: Option<String>,
    cover_manual: bool,
    isbn_13: Option<String>,
}

fn not_yet_implemented() -> ! {
    todo!("search-fallback-chain async add handler implementation is not yet wired")
}

/// REQ-IDs: REQ-004, REQ-007
/// AC-IDs: AC-004
/// Directive: selected search result is held for cover choice before create request is submitted.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_async_add_handler_selected_result_requires_cover_choice_before_add() {
    let selected_result_has_default_cover = true;
    let add_button_visible = true;
    let create_submitted_before_cover_picker = false;

    assert!(selected_result_has_default_cover);
    assert!(add_button_visible);
    assert!(!create_submitted_before_cover_picker);
    not_yet_implemented();
}

/// REQ-IDs: REQ-008
/// AC-IDs: AC-006
/// Directive: cover_manual=true -> work.cover_manual=true in DB after add.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_async_add_handler_cover_manual_true_persists_manual_cover_in_db() {
    let req = ContractAddWorkRequest {
        title: "Dune".into(),
        author_name: "Frank Herbert".into(),
        cover_url: Some("https://images.example/dune.jpg".into()),
        cover_manual: true,
        isbn_13: None,
    };
    let cover_is_manual = req.cover_manual && req.cover_url.is_some();

    assert!(cover_is_manual);
    not_yet_implemented();
}

/// REQ-IDs: REQ-007, REQ-008
/// AC-IDs: AC-007
/// Directive: cover_manual=false -> work.cover_manual=false.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_async_add_handler_cover_manual_false_persists_non_manual_cover_in_db() {
    let req = ContractAddWorkRequest {
        title: "Dune".into(),
        author_name: "Frank Herbert".into(),
        cover_url: None,
        cover_manual: false,
        isbn_13: None,
    };
    let cover_is_manual = req.cover_manual && req.cover_url.is_some();

    assert!(!cover_is_manual);
    not_yet_implemented();
}

/// REQ-IDs: REQ-011
/// AC-IDs: AC-010
/// Directive: isbn_13 from request -> work.isbn_13 in DB.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_async_add_handler_isbn_13_from_request_persists_to_work() {
    let req = ContractAddWorkRequest {
        title: "Dune".into(),
        author_name: "Frank Herbert".into(),
        cover_url: None,
        cover_manual: false,
        isbn_13: Some("9780441013593".into()),
    };

    assert_eq!(req.isbn_13.as_deref(), Some("9780441013593"));
    not_yet_implemented();
}

/// REQ-IDs: REQ-011
/// AC-IDs: AC-010
/// Directive: isbn_13 from request -> EnglishSeed.isbn populated for identity resolver.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_async_add_handler_isbn_13_from_request_populates_english_seed() {
    let seed = WorkSeed {
        ol_key: None,
        gr_key: None,
        hc_key: None,
        isbn_13: Some("9780441013593".into()),
        asin: None,
        title: Some("Dune".into()),
        author_name: Some("Frank Herbert".into()),
        language: None,
        series_name: None,
        year: None,
        user_confirmed: false,
    };

    assert_eq!(seed.isbn_13.as_deref(), Some("9780441013593"));
    assert_eq!(seed.title.as_deref(), Some("Dune"));
    not_yet_implemented();
}

/// REQ-IDs: REQ-009
/// AC-IDs: AC-008
/// Directive: user-initiated add returns Unenriched status.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_async_add_handler_user_initiated_add_returns_unenriched_status() {
    let _workflow = StubEnrichmentWorkflow::succeeding();
    let status = EnrichmentStatus::Unenriched;

    assert_eq!(status, EnrichmentStatus::Unenriched);
    assert_matches!(status, EnrichmentStatus::Unenriched);
    not_yet_implemented();
}

/// REQ-IDs: REQ-010
/// AC-IDs: AC-009
/// Directive: work detail response exposes terminal enrichment status before metadata batch is visible.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_async_add_handler_work_detail_poll_returns_single_terminal_batch() {
    let poll_statuses = [EnrichmentStatus::Unenriched, EnrichmentStatus::Enriched];
    let partial_metadata_visible_before_terminal = false;

    assert_eq!(poll_statuses[0], EnrichmentStatus::Unenriched);
    assert_eq!(poll_statuses[1], EnrichmentStatus::Enriched);
    assert!(!partial_metadata_visible_before_terminal);
    not_yet_implemented();
}
