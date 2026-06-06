#![allow(dead_code, unused_imports)]

//! Behavioral tests for work-creation-consistency domain seed and identity state.

#[path = "common.rs"]
mod common;

use livrarr_db::{CreateWorkDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::{CapturedIdentity, RawHarvest, WorkSeed};
use livrarr_domain::services::WorkIdentityError;
use livrarr_domain::EnrichmentStatus;

fn captured_identity(
    ol_key: Option<&str>,
    gr_key: Option<&str>,
    hc_key: Option<&str>,
    isbn_13: Option<&str>,
    asin: Option<&str>,
) -> CapturedIdentity {
    CapturedIdentity {
        ol_key: ol_key.map(str::to_string),
        gr_key: gr_key.map(str::to_string),
        hc_key: hc_key.map(str::to_string),
        isbn_13: isbn_13.map(str::to_string),
        asin: asin.map(str::to_string),
        title: "The Identity Book".to_string(),
        author_name: "Case Writer".to_string(),
        language: Some("en".to_string()),
    }
}

/// REQ-IDs: REQ-029, AC-037
/// Directive: malformed ISBN in RawHarvest is dropped from WorkSeed.
#[test]

fn test_wcc_seed_req_029_ac_037_sanitized_malformed_isbn_is_absent() {
    let seed = WorkSeed::sanitized(RawHarvest {
        isbn: Some("9780439139602".to_string()),
        title: Some("The Identity Book".to_string()),
        author_name: Some("Case Writer".to_string()),
        ..RawHarvest::default()
    })
    .expect("title and author still make the seed usable");

    assert_eq!(seed.isbn_13, None);
    assert_eq!(seed.title.as_deref(), Some("The Identity Book"));
    assert_eq!(seed.author_name.as_deref(), Some("Case Writer"));
}

/// REQ-IDs: REQ-004, REQ-029, AC-004, AC-029
/// Directive: sanitized ASIN folds checksum-valid ISBN-10 to isbn_13 and retains checksum-invalid ASIN.
#[test]

fn test_wcc_seed_req_004_req_029_ac_004_ac_029_sanitized_asin_isbn10_split() {
    let isbn_asin = WorkSeed::sanitized(RawHarvest {
        asin: Some("0439139600".to_string()),
        title: Some("Valid Print ASIN".to_string()),
        author_name: Some("Case Writer".to_string()),
        ..RawHarvest::default()
    })
    .expect("valid print ASIN should produce a usable seed");

    assert_eq!(isbn_asin.isbn_13.as_deref(), Some("9780439139601"));
    assert_eq!(isbn_asin.asin, None);

    let retained_asin = WorkSeed::sanitized(RawHarvest {
        asin: Some("0439139601".to_string()),
        title: Some("Audio ASIN".to_string()),
        author_name: Some("Case Writer".to_string()),
        ..RawHarvest::default()
    })
    .expect("checksum-invalid ISBN-10-shaped ASIN is retained as ASIN");

    assert_eq!(retained_asin.isbn_13, None);
    assert_eq!(retained_asin.asin.as_deref(), Some("0439139601"));
}

/// REQ-IDs: REQ-029
/// Directive: a RawHarvest with no identifier and no title+author is rejected as EmptySeed.
#[test]

fn test_wcc_seed_req_029_sanitized_empty_seed_returns_empty_seed_error() {
    let err = WorkSeed::sanitized(RawHarvest {
        isbn: Some("9780439139602".to_string()),
        asin: Some("".to_string()),
        gr_key: Some("not-a-key".to_string()),
        ..RawHarvest::default()
    })
    .expect_err("no normalized identifier and no title+author should be rejected");

    assert!(matches!(err, WorkIdentityError::EmptySeed));
}

/// REQ-IDs: REQ-028, AC-028
/// Directive: CapturedIdentity::merge_missing adds missing anchors and never overwrites existing ones.
#[test]

fn test_wcc_identity_req_028_ac_028_merge_missing_adds_gr_key_without_overwriting_ol_key() {
    let mut existing = captured_identity(Some("OL111W"), None, None, Some("9780439139601"), None);
    let incoming = captured_identity(
        Some("OL222W"),
        Some("12345"),
        Some("HC-999"),
        Some("9780000000002"),
        Some("B000000001"),
    );

    existing.merge_missing(&incoming);

    assert_eq!(existing.ol_key.as_deref(), Some("OL111W"));
    assert_eq!(existing.gr_key.as_deref(), Some("12345"));
    assert_eq!(existing.hc_key.as_deref(), Some("HC-999"));
    assert_eq!(existing.isbn_13.as_deref(), Some("9780439139601"));
    assert_eq!(existing.asin.as_deref(), Some("B000000001"));
}

/// REQ-IDs: REQ-026
/// Directive: IdentityStatus::NeedsReview round-trips through serde and SQLite storage.
/// (The needs-review state moved from EnrichmentStatus to the identity track.)
#[tokio::test]
async fn test_wcc_states_req_026_needs_review_round_trips_through_serde_and_db() {
    use livrarr_domain::IdentityStatus;
    let json = serde_json::to_string(&IdentityStatus::NeedsReview).unwrap();
    assert_eq!(json, "\"needs_review\"");
    assert_eq!(
        serde_json::from_str::<IdentityStatus>(&json).unwrap(),
        IdentityStatus::NeedsReview
    );

    let db = common::create_test_db().await;
    let (work, inserted) = db
        .create_work(CreateWorkDbRequest {
            user_id: 1,
            title: "Needs Review Work".to_string(),
            author_name: "Case Writer".to_string(),
            normalized_title: "needs review work".to_string(),
            normalized_author: "case writer".to_string(),
            language: Some("en".to_string()),
            ..CreateWorkDbRequest::default()
        })
        .await
        .expect("work insert should succeed");
    assert!(inserted);

    sqlx::query("UPDATE works SET identity_status = 'needs_review' WHERE id = ?")
        .bind(work.id)
        .execute(db.pool())
        .await
        .expect("needs_review status should be writable");

    let reloaded = db
        .get_work(1, work.id)
        .await
        .expect("work should reload after status update");
    assert_eq!(reloaded.identity_status, IdentityStatus::NeedsReview);
}
