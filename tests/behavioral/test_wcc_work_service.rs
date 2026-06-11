#![allow(dead_code, unused_imports)]

//! Behavioral tests for work-creation-consistency WorkService::add seams.

mod common;

use assert_matches::assert_matches;
use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::{CreateUserDbRequest, UserDb};
use livrarr_domain::identity::{
    AnchorConfidence, AnchorSetter, AnchorType, CapturedIdentity, IdentityMethod, IdentityState,
    WorkCandidate, WorkIdentityAnchor, WorkSeedFields,
};
use livrarr_domain::services::{WorkIdentityRepository, WorkService};
use livrarr_domain::{ProvenanceSetter, UserId, UserRole, WorkId};
use livrarr_metadata::work_service::WorkServiceImpl;

async fn create_user(db: &SqliteDb, suffix: &str) -> UserId {
    db.create_user(CreateUserDbRequest {
        username: format!("wcc-work-service-{suffix}"),
        password_hash: "hash".to_string(),
        role: UserRole::User,
        api_key_hash: format!("wcc-work-service-key-{suffix}"),
    })
    .await
    .expect("test user should be created")
    .id
}

fn service(
    db: SqliteDb,
) -> WorkServiceImpl<
    SqliteDb,
    livrarr_metadata::work_service::StubNoEnrichment,
    StubHttpFetcher,
    livrarr_metadata::work_service::StubNoLlm,
    livrarr_metadata::DefaultMergeEngine,
    livrarr_metadata::work_service::StubTagService,
> {
    WorkServiceImpl::without_enrichment(
        db,
        StubHttpFetcher::new(),
        tempfile::tempdir()
            .expect("test data dir")
            .path()
            .to_path_buf(),
    )
}

fn candidate(title: &str, author: &str, anchors: CapturedIdentity) -> WorkCandidate {
    WorkCandidate {
        fields: WorkSeedFields {
            title: title.to_string(),
            author_name: author.to_string(),
            language: anchors.language.clone().unwrap_or_else(|| "en".to_string()),
            author_ol_key: None,
            year: Some(1965),
            cover_url: None,
            detail_url: None,
            description: None,
            series_name: None,
            series_position: None,
        },
        identity: IdentityState::Confirmed {
            anchors,
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
        provenance_setter: Some(ProvenanceSetter::Import),
        import_id: None,
        cover_manual: false,
    }
}

fn pending_candidate(title: &str, author: &str) -> WorkCandidate {
    WorkCandidate {
        fields: WorkSeedFields {
            title: title.to_string(),
            author_name: author.to_string(),
            language: "en".to_string(),
            author_ol_key: None,
            year: Some(1965),
            cover_url: None,
            detail_url: None,
            description: None,
            series_name: None,
            series_position: None,
        },
        identity: IdentityState::Pending {
            reason: livrarr_domain::identity::PendingReason::NoCandidates,
            seed_anchors: None,
            top_candidates: vec![],
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
    }
}

fn anchors(
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
        title: "Dune".to_string(),
        author_name: "Frank Herbert".to_string(),
        language: Some("en".to_string()),
    }
}

async fn anchor_value_exists(
    db: &SqliteDb,
    work_id: WorkId,
    anchor_type: &str,
    value: &str,
) -> bool {
    db.list_anchors(work_id)
        .await
        .expect("anchor listing should succeed")
        .into_iter()
        .any(|anchor| {
            anchor.anchor_type.as_str() == anchor_type
                && anchor.anchor_value == value
                && matches!(anchor.confidence, AnchorConfidence::Confirmed)
        })
}

async fn open_conflict_count(db: &SqliteDb, work_id: WorkId, kind: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_conflicts
         WHERE existing_work_id = ?1 AND kind = ?2 AND status = 'open'",
    )
    .bind(work_id)
    .bind(kind)
    .fetch_one(db.pool())
    .await
    .expect("conflict count query should succeed")
}

/// REQ-IDs: REQ-001, REQ-003
/// AC-IDs: AC-002
/// Directive: WorkService::add persists every identifier carried by a Readarr
/// candidate identity as confirmed anchors.
#[tokio::test]
async fn test_wcc_work_service_ac_002_add_persists_gr_isbn_and_asin_as_anchors() {
    let db = common::create_test_db().await;
    let user_id = create_user(&db, "ac002").await;
    let service = service(db.clone());

    let result = service
        .add(
            user_id,
            candidate(
                "Dune",
                "Frank Herbert",
                anchors(
                    None,
                    Some("234225"),
                    None,
                    Some("9780441013593"),
                    Some("B000N2HCP6"),
                ),
            ),
        )
        .await
        .expect("WorkService::add should accept federated Readarr identity");

    assert!(result.created);
    assert_eq!(
        db.find_work_by_anchor(user_id, &AnchorType::new(AnchorType::GR_WORK), "234225")
            .await
            .expect("gr anchor lookup should succeed"),
        Some(result.work.id)
    );
    assert!(anchor_value_exists(&db, result.work.id, AnchorType::GR_WORK, "234225").await);
    assert!(anchor_value_exists(&db, result.work.id, AnchorType::ISBN_13, "9780441013593").await);
    assert!(anchor_value_exists(&db, result.work.id, AnchorType::ASIN, "B000N2HCP6").await);
}

/// REQ-IDs: REQ-018, REQ-020
/// AC-IDs: AC-015, AC-034, AC-035
/// Directive: WorkService::add records observable conflict rows when the same
/// normalized title+author arrives with a different federated work anchor.
#[tokio::test]
async fn test_wcc_work_service_ac_015_034_035_same_title_author_different_gr_key_records_conflict()
{
    let db = common::create_test_db().await;
    let user_id = create_user(&db, "conflicting-gr").await;
    let service = service(db.clone());

    let existing = service
        .add(
            user_id,
            candidate(
                "Dune",
                "Frank Herbert",
                anchors(
                    Some("/works/OL27448W"),
                    Some("234225"),
                    Some("hc_dune"),
                    Some("9780441013593"),
                    None,
                ),
            ),
        )
        .await
        .expect("initial confirmed work should be created");

    let second = service
        .add(
            user_id,
            candidate(
                "  dune  ",
                "FRANK HERBERT",
                anchors(
                    Some("/works/OL27448W"),
                    Some("999999"),
                    Some("hc_dune"),
                    Some("9780441013593"),
                    None,
                ),
            ),
        )
        .await
        .expect("conflicting add should return an observable result");

    assert!(!second.created);
    assert_eq!(second.work.id, existing.work.id);
    assert_eq!(
        open_conflict_count(&db, existing.work.id, "incoming_different_gr_key").await,
        1
    );
}

/// REQ-IDs: REQ-018, REQ-020
/// AC-IDs: AC-015, AC-034, AC-035
/// Directive: WorkService::add records observable conflict rows for conflicting
/// Hardcover anchors, not just OpenLibrary keys.
#[tokio::test]
async fn test_wcc_work_service_ac_015_034_035_same_title_author_different_hc_key_records_conflict()
{
    let db = common::create_test_db().await;
    let user_id = create_user(&db, "conflicting-hc").await;
    let service = service(db.clone());

    let existing = service
        .add(
            user_id,
            candidate(
                "Dune",
                "Frank Herbert",
                anchors(
                    Some("/works/OL27448W"),
                    Some("234225"),
                    Some("hc_dune"),
                    Some("9780441013593"),
                    None,
                ),
            ),
        )
        .await
        .expect("initial confirmed work should be created");

    let second = service
        .add(
            user_id,
            candidate(
                "Dune",
                "Frank Herbert",
                anchors(
                    Some("/works/OL27448W"),
                    Some("234225"),
                    Some("hc_other_dune"),
                    Some("9780441013593"),
                    None,
                ),
            ),
        )
        .await
        .expect("conflicting add should return an observable result");

    assert!(!second.created);
    assert_eq!(second.work.id, existing.work.id);
    assert_eq!(
        open_conflict_count(&db, existing.work.id, "incoming_different_hc_key").await,
        1
    );
}

/// REQ-IDs: REQ-028
/// AC-IDs: AC-028
/// Directive: WorkService::add adopts an anchorless normalized match and merges
/// the incoming federated anchor onto the existing Work.
#[tokio::test]
async fn test_wcc_work_service_ac_028_anchorless_existing_match_adopts_incoming_gr_anchor() {
    let db = common::create_test_db().await;
    let user_id = create_user(&db, "adopt-gr").await;
    let service = service(db.clone());

    let existing = service
        .add(user_id, pending_candidate("Dune", "Frank Herbert"))
        .await
        .expect("anchorless existing work should be created");

    let adopted = service
        .add(
            user_id,
            candidate(
                "dune",
                "frank herbert",
                anchors(None, Some("234225"), None, None, None),
            ),
        )
        .await
        .expect("incoming anchored candidate should adopt existing normalized match");

    assert!(!adopted.created);
    assert_eq!(adopted.work.id, existing.work.id);
    assert_eq!(
        db.find_work_by_anchor(user_id, &AnchorType::new(AnchorType::GR_WORK), "234225")
            .await
            .expect("merged gr anchor lookup should succeed"),
        Some(existing.work.id)
    );
    assert!(anchor_value_exists(&db, existing.work.id, AnchorType::GR_WORK, "234225").await);
}
