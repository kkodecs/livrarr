//! Behavioral verification test for Finding D2.
//!
//! D2 as stated: "two adds of the same book with only isbn_13 will NOT dedup —
//! two separate works are created."
//!
//! Actual behavior (as proven by these tests):
//!
//! Case A — identical title+author: the anchor loop skips ISBN but
//! `find_by_normalized_match` catches the duplicate. One work, no error.
//! D2 is refuted for this case.
//!
//! Case B — same ISBN, different title string (subtitle variant): the anchor
//! loop skips ISBN; the normalized-match fallback fails (strings differ). The
//! second add attempts to write the same ISBN anchor for a new work and hits a
//! UNIQUE constraint error (`work_identity_anchors.user_id, anchor_type,
//! anchor_value`). The call returns `Err(Validation(...))` rather than
//! silently creating a second work.
//!
//! The real D2 finding is therefore: **same ISBN + different title produces an
//! unhandled Validation error instead of a graceful dedup.** The user-facing
//! symptom is a 500 / error response, not a silent duplicate.
//!
//! Test 1 (PASS): same ISBN + identical title → dedups cleanly via normalized
//!   fallback. D2 does not apply here.
//!
//! Test 2 (FAIL on the graceful-dedup assertion, PASS on the error
//!   observation): same ISBN + subtitle-decorated title → second add errors
//!   with a Validation/UNIQUE constraint rather than deduping. The test asserts
//!   the correct behavior (no error, 1 work) to pin the bug surface; it fails
//!   because the system errors instead of deduping.

mod common;

use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{CreateUserDbRequest, UserDb};
use livrarr_domain::identity::{
    CapturedIdentity, IdentityMethod, IdentityState, WorkCandidate, WorkSeedFields,
};
use livrarr_domain::services::{WorkFilter, WorkService};
use livrarr_domain::{ProvenanceSetter, UserId, UserRole};
use livrarr_metadata::work_service::WorkServiceImpl;

async fn create_user(db: &SqliteDb, suffix: &str) -> UserId {
    db.create_user(CreateUserDbRequest {
        username: format!("d2-{suffix}"),
        password_hash: "hash".to_string(),
        role: UserRole::User,
        api_key_hash: format!("d2-key-{suffix}"),
    })
    .await
    .expect("test user should be created")
    .id
}

fn service(
    db: SqliteDb,
) -> WorkServiceImpl<SqliteDb, livrarr_metadata::work_service::StubNoEnrichment, StubHttpFetcher> {
    WorkServiceImpl::without_enrichment(
        db,
        StubHttpFetcher::new(),
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
    )
}

fn candidate_with_title(isbn: &str, title: &str, author: &str) -> WorkCandidate {
    WorkCandidate {
        fields: WorkSeedFields {
            title: title.to_string(),
            author_name: author.to_string(),
            language: "en".to_string(),
            author_ol_key: None,
            year: Some(2000),
            cover_url: None,
            detail_url: None,
            description: None,
            series_name: None,
            series_position: None,
        },
        identity: IdentityState::Confirmed {
            anchors: CapturedIdentity {
                ol_key: None,
                gr_key: None,
                hc_key: None,
                isbn_13: Some(isbn.to_string()),
                asin: None,
                title: title.to_string(),
                author_name: author.to_string(),
                language: None,
            },
            method: IdentityMethod::IsbnDirect,
            score: None,
        },
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: Some(ProvenanceSetter::Import),
        import_id: None,
        cover_manual: false,
        add_source: livrarr_domain::history_events::WorkAddSource::Search,
    }
}

fn all_works_filter() -> WorkFilter {
    WorkFilter {
        author_id: None,
        monitored: None,
        enrichment_status: None,
        media_type: None,
        language: None,
        sort_by: None,
        sort_dir: None,
    }
}

/// D2 — Part 1 (EXPECTED TO PASS): common case covered by normalized fallback.
///
/// Same ISBN, identical title+author. The anchor loop skips ISBN, but
/// `find_by_normalized_match` deduplicates correctly. Documents that the
/// happy-path same-title-author case is not broken.
#[tokio::test]
async fn test_verify_d2_same_isbn_identical_title_author_dedups_via_normalized_fallback() {
    let db = common::create_test_db().await;
    let user_id = create_user(&db, "d2-common").await;
    let service = service(db);

    let isbn = "9780593099322";
    let title = "Storm Front";
    let author = "Jim Butcher";

    let first = service
        .add(user_id, candidate_with_title(isbn, title, author))
        .await
        .expect("first add should succeed");
    assert!(first.created, "first add should create a new work");

    let second = service
        .add(user_id, candidate_with_title(isbn, title, author))
        .await
        .expect("second add should not error");

    let works = service
        .list(user_id, all_works_filter())
        .await
        .expect("list should succeed");

    assert_eq!(
        works.len(),
        1,
        "same ISBN + identical title+author: normalized-match fallback should dedup to 1 work, \
         but got {}. second.created={}, ids: {:?} vs {:?}",
        works.len(),
        second.created,
        second.work.id,
        first.work.id,
    );
    assert!(
        !second.created,
        "second add must return created=false when deduped"
    );
    assert_eq!(
        second.work.id, first.work.id,
        "second add must return the same work id as the first"
    );
}

/// D2 — Part 2 (EXPECTED TO FAIL — proves the real D2 bug surface).
///
/// Same ISBN `9780593099322` (Storm Front), but the second add has a
/// subtitle-decorated title. This defeats `find_by_normalized_match`. The
/// anchor loop also skips ISBN. The correct behavior: dedup gracefully to the
/// existing work (return created=false, same work id, no error).
///
/// Actual behavior: the second add attempts to write the same ISBN anchor for a
/// new work and hits a UNIQUE constraint on `work_identity_anchors`, returning
/// `Err(Validation("anchor write failed: ...UNIQUE constraint failed..."))`.
///
/// The test asserts correct behavior (Ok, 1 work). It FAILS because the system
/// errors with a Validation/UNIQUE constraint instead of deduping.
#[tokio::test]
#[ignore = "Finding D2 (red-by-design gate): same-ISBN add with a subtitle-variant title errors with a UNIQUE-constraint Validation (500) instead of graceful dedup. Standalone dedup bug; fix pending (must respect AC-020 wrong-book guard). Run with --ignored."]
async fn test_verify_d2_same_isbn_different_title_must_dedup_not_error() {
    let db = common::create_test_db().await;
    let user_id = create_user(&db, "d2-subtitle").await;
    let service = service(db);

    let isbn = "9780593099322";
    let author = "Jim Butcher";

    let first = service
        .add(user_id, candidate_with_title(isbn, "Storm Front", author))
        .await
        .expect("first add should succeed");
    assert!(first.created, "first add should create a new work");

    // Same ISBN, subtitle-decorated title as a different source might provide.
    // Correct behavior: dedup to the existing work, no error.
    // D2 bug: returns Err(Validation("anchor write failed: UNIQUE constraint failed"))
    // because the anchor loop skips ISBN and the normalized-match misses the
    // subtitle variant, so add() tries to insert the same ISBN anchor for a
    // new work and the DB constraint fires.
    let second_result = service
        .add(
            user_id,
            candidate_with_title(isbn, "Storm Front: The Dresden Files, Book 1", author),
        )
        .await;

    // Assert the correct behavior: no error, returns the existing work.
    // This assertion fails today — second_result is Err(Validation(...)).
    let second = second_result.expect(
        "D2 BUG: second add of the same ISBN with a subtitle-decorated title must dedup \
         gracefully (return existing work, created=false) rather than erroring with a \
         UNIQUE constraint violation on work_identity_anchors. \
         The anchor loop skips ISBN (bridge-anchor policy) and the normalized-match \
         fallback cannot reconcile 'storm front' with \
         'storm front the dresden files book 1', so the bug path fires.",
    );

    assert!(
        !second.created,
        "second add of the same ISBN must return created=false (dedup), not create a new work"
    );
    assert_eq!(
        second.work.id, first.work.id,
        "second add must return the same work as the first (dedup by ISBN)"
    );

    let works = service
        .list(user_id, all_works_filter())
        .await
        .expect("list should succeed");
    assert_eq!(works.len(), 1, "only 1 work should exist after dedup");
}
