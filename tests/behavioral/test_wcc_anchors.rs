#![allow(dead_code, unused_imports)]

//! Behavioral tests for work-creation-consistency federated anchor persistence.

#[path = "common.rs"]
mod common;

use livrarr_db::{CreateWorkDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::{AnchorConfidence, AnchorSetter, AnchorType, CapturedIdentity};
use livrarr_domain::services::WorkIdentityRepository;
use livrarr_domain::{UserId, WorkId};

async fn create_work(db: &livrarr_db::sqlite::SqliteDb, user_id: UserId, title: &str) -> WorkId {
    let (work, inserted) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: title.to_string(),
            author_name: "Case Writer".to_string(),
            normalized_title: title.to_ascii_lowercase(),
            normalized_author: "case writer".to_string(),
            language: Some("en".to_string()),
            ..CreateWorkDbRequest::default()
        })
        .await
        .expect("work insert should succeed");
    assert!(inserted, "test work title should be unique");
    work.id
}

fn incoming_identity(
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
        title: "Federated Anchor Book".to_string(),
        author_name: "Case Writer".to_string(),
        language: Some("en".to_string()),
    }
}

/// REQ-IDs: REQ-001, REQ-003, AC-002
/// Directive: confirm_anchor persists an HC work anchor and find_work_by_anchor can resolve it.
#[tokio::test]

async fn test_wcc_anchors_req_001_req_003_ac_002_confirm_anchor_hc_work_is_findable() {
    let db = common::create_test_db().await;
    let work_id = create_work(&db, 1, "Confirm HC Anchor").await;

    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::HC_WORK),
        "HC-12345",
        AnchorSetter::Import,
    )
    .await
    .expect("HC anchor should be confirmed");

    let found = db
        .find_work_by_anchor(1, &AnchorType::new(AnchorType::HC_WORK), "HC-12345")
        .await
        .expect("HC anchor lookup should succeed");
    assert_eq!(found, Some(work_id));
}

/// REQ-IDs: REQ-028, AC-028
/// Directive: merge_missing_anchors adds incoming gr_key while preserving existing ol_key.
#[tokio::test]

async fn test_wcc_anchors_req_028_ac_028_merge_missing_anchors_adds_gr_without_overwriting_ol() {
    let db = common::create_test_db().await;
    let work_id = create_work(&db, 1, "Merge Missing Anchors").await;

    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::OL_WORK),
        "OL111W",
        AnchorSetter::User,
    )
    .await
    .expect("existing OL anchor should be confirmed");

    db.merge_missing_anchors(
        work_id,
        &incoming_identity(
            Some("OL222W"),
            Some("12345"),
            Some("HC-999"),
            Some("9780439139601"),
            Some("B000000001"),
        ),
    )
    .await
    .expect("missing anchors should merge additively");

    let anchors = db.list_anchors(work_id).await.expect("anchors should list");
    let confirmed: Vec<_> = anchors
        .iter()
        .filter(|a| a.confidence == AnchorConfidence::Confirmed)
        .map(|a| (a.anchor_type.as_str(), a.anchor_value.as_str()))
        .collect();

    assert!(confirmed.contains(&(AnchorType::OL_WORK, "OL111W")));
    assert!(confirmed.contains(&(AnchorType::GR_WORK, "12345")));
    assert!(confirmed.contains(&(AnchorType::HC_WORK, "HC-999")));
    assert!(!confirmed.contains(&(AnchorType::OL_WORK, "OL222W")));
}

/// REQ-IDs: REQ-018, REQ-020, AC-015, AC-034, AC-035
/// Directive: detect_conflicting_anchors reports federated same-type anchor conflicts
/// for machine-set anchors (AutoSearch/AutoIsbn/etc.), but respects User-set anchors
/// — a User pick is the top of the confidence hierarchy and must not be overridden by
/// a machine result.
#[tokio::test]

async fn test_wcc_anchors_req_018_req_020_ac_015_ac_034_ac_035_detect_conflicting_ol_anchor() {
    let db = common::create_test_db().await;
    let work_id = create_work(&db, 1, "Detect Conflicting Anchors").await;

    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::ISBN_13),
        "9780439139601",
        AnchorSetter::AutoIsbn,
    )
    .await
    .expect("ISBN bridge anchor should be confirmed");

    // User-set anchor: a differing incoming OL key must NOT generate a conflict.
    // The user already made the identity call; a machine result cannot override it.
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::OL_WORK),
        "OL111W",
        AnchorSetter::User,
    )
    .await
    .expect("existing OL anchor should be confirmed");

    let conflicts_user_set = db
        .detect_conflicting_anchors(
            work_id,
            &incoming_identity(
                Some("OL222W"),
                Some("12345"),
                None,
                Some("9780439139601"),
                None,
            ),
            livrarr_domain::identity::ConflictSource::ManualAdd,
        )
        .await
        .expect("conflict detection should succeed");

    assert_eq!(
        conflicts_user_set.len(),
        0,
        "User-set OL anchor must not generate a conflict; got: {conflicts_user_set:?}"
    );

    // AutoSearch-set anchor: a differing incoming OL key MUST generate a conflict.
    let work_id2 = create_work(&db, 1, "Detect Conflicting Anchors AutoSearch").await;
    db.confirm_anchor(
        work_id2,
        AnchorType::new(AnchorType::OL_WORK),
        "OL333W", // different value from work_id's anchor to avoid cross-work user-uniqueness
        AnchorSetter::AutoSearch,
    )
    .await
    .expect("existing OL anchor should be confirmed");

    let conflicts_auto = db
        .detect_conflicting_anchors(
            work_id2,
            &incoming_identity(
                Some("OL222W"),
                Some("12345"),
                None,
                Some("9780439139601"),
                None,
            ),
            livrarr_domain::identity::ConflictSource::ManualAdd,
        )
        .await
        .expect("conflict detection should succeed");

    assert_eq!(conflicts_auto.len(), 1);
    assert_eq!(
        conflicts_auto[0].kind,
        livrarr_domain::identity::IdentityConflictKind::IncomingDifferentOlKey
    );
    assert_eq!(conflicts_auto[0].existing_work_id, work_id2);
    // incoming OL key differs from the existing "OL333W"
    assert_eq!(conflicts_auto[0].incoming.ol_key.as_deref(), Some("OL222W"));
    assert_eq!(conflicts_auto[0].incoming.gr_key.as_deref(), Some("12345"));
}

/// REQ-IDs: REQ-002, AC-003
/// Directive: backfill_gr_numeric rewrites slug-form works.gr_key and creates bare-numeric GR anchors idempotently.
#[tokio::test]

async fn test_wcc_anchors_req_002_ac_003_backfill_gr_numeric_rewrites_works_and_backfills_anchor() {
    let db = common::create_test_db().await;
    let slug_work_id = create_work(&db, 1, "Backfill Slug GR").await;
    let bare_work_id = create_work(&db, 1, "Backfill Bare GR").await;

    sqlx::query("UPDATE works SET gr_key = ? WHERE id = ?")
        .bind("12345.Some_Slug")
        .bind(slug_work_id)
        .execute(db.pool())
        .await
        .expect("slug GR key should seed works row");
    sqlx::query("UPDATE works SET gr_key = ? WHERE id = ?")
        .bind("67890")
        .bind(bare_work_id)
        .execute(db.pool())
        .await
        .expect("bare GR key should seed works row");

    db.backfill_gr_numeric()
        .await
        .expect("GR numeric backfill should succeed");
    db.backfill_gr_numeric()
        .await
        .expect("GR numeric backfill should be idempotent");

    let slug_after: String = sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
        .bind(slug_work_id)
        .fetch_one(db.pool())
        .await
        .expect("slug work should reload");
    let bare_after: String = sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
        .bind(bare_work_id)
        .fetch_one(db.pool())
        .await
        .expect("bare work should reload");

    assert_eq!(slug_after, "12345");
    assert_eq!(bare_after, "67890");

    let slug_anchor_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors \
         WHERE work_id = ? AND anchor_type = 'gr_work' AND anchor_value = '12345'",
    )
    .bind(slug_work_id)
    .fetch_one(db.pool())
    .await
    .expect("slug work GR anchor count should query");
    let bare_anchor_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors \
         WHERE work_id = ? AND anchor_type = 'gr_work' AND anchor_value = '67890'",
    )
    .bind(bare_work_id)
    .fetch_one(db.pool())
    .await
    .expect("bare work GR anchor count should query");

    assert_eq!(slug_anchor_count, 1);
    assert_eq!(bare_anchor_count, 1);
}
