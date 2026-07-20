//! Behavioral verification test for Finding A1 (T3).
//!
//! A1: The author monitor constructs `IdentityState::Confirmed { method:
//! TitleAuthorSearch, anchors: { ol_key: Some(...), gr_key: None } }` and
//! passes it directly to `work_service.add()` — no call to
//! `WorkService::resolve_identity()` ever occurs. This test drives that path
//! and proves:
//!
//! (a) `Confirmed/TitleAuthorSearch` is written to the DB for a work that was
//!     never identity-resolved through the multi-provider pipeline.
//!
//! (b) Whether the "adopt" step in `WorkService::add()` deduplicates an
//!     existing GR-only work against the incoming OL-keyed monitor candidate.
//!     The adopt query (`find_normalized_match_no_anchor_for_user`) matches
//!     works that have no confirmed `ol_work` anchor. A GR-only work has no
//!     `ol_work` anchor, so it IS eligible for adopt IF the normalized titles
//!     match.
//!
//! Expected outcomes (what this test documents, not what A1 claimed):
//!  - (a): PROVEN — Confirmed/TitleAuthorSearch IS written without resolution.
//!  - (b): adopt DOES deduplicate the GR-only work — no duplicate created.
//!    A1's "duplicate created" claim is therefore narrower than stated: the
//!    duplicate bug only fires when titles do NOT normalize the same (e.g.
//!    OL returns a cleaned/truncated title that doesn't match the GR work's
//!    stored normalized form).

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

async fn create_user(db: &SqliteDb) -> UserId {
    db.create_user(CreateUserDbRequest {
        username: "a1-test-user".to_string(),
        password_hash: "hash".to_string(),
        role: UserRole::User,
        api_key_hash: "a1-test-key".to_string(),
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

/// A GR-only candidate: no ol_key, gr_key set, method=UserSelected (simulating
/// a work added via GR search/import path). Same title+author as the monitor
/// candidate below.
fn gr_only_candidate() -> WorkCandidate {
    WorkCandidate {
        fields: WorkSeedFields {
            title: "Storm Front".to_string(),
            author_name: "Jim Butcher".to_string(),
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
                gr_key: Some("123456".to_string()),
                hc_key: None,
                isbn_13: None,
                asin: None,
                title: "Storm Front".to_string(),
                author_name: "Jim Butcher".to_string(),
                language: None,
            },
            method: IdentityMethod::UserSelected,
            score: None,
        },
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: Some(ProvenanceSetter::User),
        import_id: None,
        cover_manual: false,
        add_source: livrarr_domain::history_events::WorkAddSource::Search,
    }
}

/// The author-monitor candidate: exactly what `seed_author_monitor()` builds in
/// `author_monitor_workflow.rs`. OL key only, no GR key, method=TitleAuthorSearch.
/// This is Confirmed WITHOUT going through `resolve_identity()`.
fn monitor_ol_candidate() -> WorkCandidate {
    WorkCandidate {
        fields: WorkSeedFields {
            title: "Storm Front".to_string(),
            author_name: "Jim Butcher".to_string(),
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
                ol_key: Some("OL999W".to_string()),
                gr_key: None,
                hc_key: None,
                isbn_13: None,
                asin: None,
                title: "Storm Front".to_string(),
                author_name: "Jim Butcher".to_string(),
                language: None,
            },
            method: IdentityMethod::TitleAuthorSearch,
            score: None,
        },
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: Some(ProvenanceSetter::AutoAdded),
        import_id: None,
        cover_manual: false,
        add_source: livrarr_domain::history_events::WorkAddSource::AuthorMonitor,
    }
}

/// A monitor candidate with a DIFFERENT title to simulate a case where OL
/// returns a slightly different normalized title than the GR-added work. This
/// is the scenario where A1's duplicate claim would apply.
fn monitor_ol_candidate_different_title() -> WorkCandidate {
    WorkCandidate {
        fields: WorkSeedFields {
            title: "Storm Front: Dresden Files".to_string(), // OL often appends subtitle
            author_name: "Jim Butcher".to_string(),
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
                ol_key: Some("OL999W".to_string()),
                gr_key: None,
                hc_key: None,
                isbn_13: None,
                asin: None,
                title: "Storm Front: Dresden Files".to_string(),
                author_name: "Jim Butcher".to_string(),
                language: None,
            },
            method: IdentityMethod::TitleAuthorSearch,
            score: None,
        },
        candidate_id: None,
        source_provider_data: None,
        file_path: None,
        delete_existing_after_import: false,
        series_id: None,
        monitor_ebook: None,
        monitor_audiobook: None,
        provenance_setter: Some(ProvenanceSetter::AutoAdded),
        import_id: None,
        cover_manual: false,
        add_source: livrarr_domain::history_events::WorkAddSource::AuthorMonitor,
    }
}

/// Part (a): Confirms that `Confirmed/TitleAuthorSearch` is stamped on the work
/// without any call to `resolve_identity`. The service is `without_enrichment`
/// so there is no resolver, yet the add() succeeds and the DB reflects the
/// Confirmed status — proving the monitor pre-stamps Confirmed directly.
///
/// NOTE: `WorkService::add()` itself does not store `identity_status` directly —
/// it stores via `derived_identity_status()` which gives Confirmed when a work
/// anchor (ol/gr/hc) is present. The work's `identity_status` column IS written
/// as "confirmed" because the OL key is present. The method column comes from
/// the anchor setter metadata. So this test confirms the structural fact: a
/// monitor-added work gets `identity_status = Confirmed` from a candidate that
/// was NEVER fed through `resolve_identity()`.
#[tokio::test]
async fn test_verify_a1_monitor_stamps_confirmed_without_resolve() {
    let db = common::create_test_db().await;
    let user_id = create_user(&db).await;
    let svc = service(db);

    let result = svc
        .add(user_id, monitor_ol_candidate())
        .await
        .expect("monitor-path add should succeed");

    assert!(
        result.created,
        "A1(a): monitor-path add should create a new work (first add)"
    );

    // Verify the work exists and was created as Confirmed despite no resolver call.
    let works = svc
        .list(user_id, all_works_filter())
        .await
        .expect("list should succeed");

    assert_eq!(works.len(), 1, "exactly one work should exist");
    let work = &works[0];

    // identity_status must be Confirmed because an ol_key anchor was written —
    // but that anchor was NOT cross-provider-resolved. The monitor just stamped it.
    assert_eq!(
        work.identity_status,
        livrarr_domain::IdentityStatus::Confirmed,
        "A1(a) PROVEN: work has identity_status=Confirmed stamped by the monitor \
         without going through resolve_identity(). OL key='OL999W' was written as \
         a confirmed anchor directly from the bibliography, not from a resolver."
    );
}

/// Part (b) - same title: when the monitor adds a work with an OL key and the
/// SAME normalized title as an existing GR-only work, the adopt step
/// (`find_normalized_match_no_anchor_for_user`) catches it because the GR-only
/// work has no confirmed `ol_work` anchor. Only ONE work should exist.
///
/// This refutes A1's "duplicate created" claim for the same-title case.
#[tokio::test]
async fn test_verify_a1_gr_only_work_is_adopted_same_title() {
    let db = common::create_test_db().await;
    let user_id = create_user(&db).await;
    let svc = service(db);

    // Step 1: Create a GR-only work (has gr_work anchor, no ol_work anchor).
    let gr_result = svc
        .add(user_id, gr_only_candidate())
        .await
        .expect("GR-only add should succeed");

    assert!(gr_result.created, "GR-only work should be created");
    let gr_work_id = gr_result.work.id;

    // Step 2: Author monitor adds same book with OL key only. Same title => adopt.
    let monitor_result = svc
        .add(user_id, monitor_ol_candidate())
        .await
        .expect("monitor-path add should succeed");

    // Step 3: Check total works count.
    let works = svc
        .list(user_id, all_works_filter())
        .await
        .expect("list should succeed");

    // The adopt step should have caught the GR-only work (no ol_work anchor,
    // same normalized title+author). Only one work should exist.
    assert_eq!(
        works.len(),
        1,
        "A1(b) adopt path: GR-only work with same title should be adopted, \
         not duplicated. works.len()={}, monitor_result.created={}, \
         gr_work_id={}, monitor_work_id={}",
        works.len(),
        monitor_result.created,
        gr_work_id,
        monitor_result.work.id
    );

    assert_eq!(
        monitor_result.work.id, gr_work_id,
        "A1(b): the adopted work should be the original GR-only work (same id)"
    );

    assert!(
        !monitor_result.created,
        "A1(b): adopt path should return created=false (existing work returned)"
    );
}

/// Part (b) - different title: when the monitor's OL-sourced title is different
/// from the stored GR work's normalized title, the adopt step MISSES it. A
/// second work IS created — this is where A1's duplicate claim holds.
///
/// Example: GR has "Storm Front", OL returns "Storm Front: Dresden Files"
/// (subtitle appended). Different normalized forms → no adopt match → duplicate.
#[tokio::test]
async fn test_verify_a1_gr_only_work_not_deduped_different_title() {
    let db = common::create_test_db().await;
    let user_id = create_user(&db).await;
    let svc = service(db);

    // Step 1: Create a GR-only work titled "Storm Front".
    let gr_result = svc
        .add(user_id, gr_only_candidate())
        .await
        .expect("GR-only add should succeed");

    assert!(gr_result.created, "GR-only work should be created");

    // Step 2: Monitor adds same book but with OL's subtitle-appended title.
    // Normalized form differs from the GR work's stored normalized_title.
    let monitor_result = svc
        .add(user_id, monitor_ol_candidate_different_title())
        .await
        .expect("monitor-path add (different title) should succeed");

    // Step 3: Count works.
    let works = svc
        .list(user_id, all_works_filter())
        .await
        .expect("list should succeed");

    // A1 CONFIRMED for the different-title case: the GR-only work is NOT
    // deduped when the monitor brings a title with different normalization.
    // Two works are created.
    assert_eq!(
        works.len(),
        2,
        "A1(b) confirmed for different-title case: expected 2 works (GR work + \
         monitor OL work with different title), but got {}. If this fails with 1, \
         the normalized forms happen to match and adopt DID catch it.",
        works.len()
    );

    assert!(
        monitor_result.created,
        "A1(b): monitor adds a SECOND work when the title normalization differs. \
         This is the duplicate that A1 warns about."
    );
}
