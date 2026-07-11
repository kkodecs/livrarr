//! Behavioral verification tests for Finding D1.
//!
//! D1: `preflight_and_merge_anchors` is non-blocking. When it detects
//! `ol_key=Y` conflicts with the existing work's confirmed `ol_key=X`, it:
//!   (1) raises a conflict record in `work_identity_conflicts`, then
//!   (2) still runs `merge_missing_anchors` — absorbing the incoming
//!       candidate's OTHER anchors (e.g. `gr_key=Z`) onto the existing work.
//!
//! Two tests, split by the existing anchor's setter:
//!   - machine-set (`AutoSearch`) existing anchor → the conflict IS raised and
//!     the non-conflicting anchor is still absorbed (the D1 behavior);
//!   - `User`-set existing anchor → `detect_conflicting_anchors` deliberately
//!     suppresses the conflict (the user outranks machine values — see the
//!     commented rule in `sqlite_work_identity.rs`, `detect_conflicting_anchors`,
//!     with its TODO(phase2-3) about future redirect awareness).

mod common;

use livrarr_db::{CreateWorkDbRequest, WorkDbCreate};
use livrarr_domain::identity::{
    AnchorConfidence, AnchorSetter, AnchorType, CapturedIdentity, ConflictSource,
    IdentityConflictKind,
};
use livrarr_domain::services::WorkIdentityRepository;
use livrarr_domain::WorkId;

async fn create_work(db: &livrarr_db::sqlite::SqliteDb, title: &str) -> WorkId {
    let (work, inserted) = db
        .create_work(CreateWorkDbRequest {
            user_id: 1,
            title: title.to_string(),
            author_name: "D1 Writer".to_string(),
            normalized_title: title.to_ascii_lowercase(),
            normalized_author: "d1 writer".to_string(),
            language: Some("en".to_string()),
            ..CreateWorkDbRequest::default()
        })
        .await
        .expect("work insert should succeed");
    assert!(inserted, "test work title should be unique");
    work.id
}

/// REQ-IDs: REQ-018, REQ-020, REQ-028
/// Finding: D1
/// Directive: When an incoming candidate carries `ol_key=Y` (conflicting with
/// the existing work's confirmed `ol_key=X`) plus `gr_key=Z` (a new anchor),
/// `preflight_and_merge_anchors` must both raise a conflict record AND absorb
/// `gr_key=Z` onto the existing work. The conflict is non-blocking: the merge
/// still runs for the non-conflicting anchors.
#[tokio::test]
async fn test_verify_d1_conflict_is_raised_and_gr_key_is_absorbed() {
    let db = common::create_test_db().await;

    // Step 1: create a work with confirmed ol_key=X. Machine-set (AutoSearch):
    // a User-set anchor suppresses conflict detection by design — that rule is
    // pinned by the companion test below.
    let work_id = create_work(&db, "D1 Conflict Book").await;
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::OL_WORK),
        "OLX001W",
        AnchorSetter::AutoSearch,
    )
    .await
    .expect("existing OL anchor should be confirmed");

    // Step 2: build an incoming CapturedIdentity with ol_key=Y (conflicts)
    // and gr_key=Z (new — should be absorbed by merge_missing_anchors).
    let incoming = CapturedIdentity {
        ol_key: Some("OLY002W".to_string()), // conflicts with OLX001W
        gr_key: Some("99999".to_string()),   // new — should be absorbed
        hc_key: None,
        isbn_13: None,
        asin: None,
        title: "D1 Conflict Book".to_string(),
        author_name: "D1 Writer".to_string(),
        language: Some("en".to_string()),
    };

    // Step 3a: detect + raise conflicts (first half of preflight_and_merge_anchors).
    let conflicts = db
        .detect_conflicting_anchors(work_id, &incoming, ConflictSource::ManualAdd)
        .await
        .expect("conflict detection should succeed");

    assert_eq!(
        conflicts.len(),
        1,
        "D1: exactly one OL conflict should be detected; got {:?}",
        conflicts.iter().map(|c| &c.kind).collect::<Vec<_>>()
    );
    assert_eq!(
        conflicts[0].kind,
        IdentityConflictKind::IncomingDifferentOlKey,
        "D1: conflict kind should be IncomingDifferentOlKey"
    );

    for conflict in conflicts {
        db.raise_identity_conflict(conflict)
            .await
            .expect("conflict row should be raised");
    }

    // Step 3b: merge_missing_anchors still runs despite the conflict (second half).
    db.merge_missing_anchors(work_id, &incoming)
        .await
        .expect("merge_missing_anchors should succeed even after conflict");

    // Step 4a: assert a conflict record exists in work_identity_conflicts.
    let conflict_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_conflicts \
         WHERE existing_work_id = ? AND kind = 'incoming_different_ol_key' AND status = 'open'",
    )
    .bind(work_id)
    .fetch_one(db.pool())
    .await
    .expect("conflict count query should succeed");

    assert_eq!(
        conflict_count, 1,
        "D1: expected 1 open conflict row for work {work_id}, found {conflict_count}"
    );

    // Step 4b: assert gr_key=Z was absorbed onto the existing work.
    let anchors = db
        .list_anchors(work_id)
        .await
        .expect("anchor list should succeed");

    let confirmed: Vec<(&str, &str)> = anchors
        .iter()
        .filter(|a| a.confidence == AnchorConfidence::Confirmed)
        .map(|a| (a.anchor_type.as_str(), a.anchor_value.as_str()))
        .collect();

    // Original OL anchor must be preserved (not overwritten).
    assert!(
        confirmed.contains(&(AnchorType::OL_WORK, "OLX001W")),
        "D1: original ol_key=OLX001W must still be confirmed; anchors={confirmed:?}"
    );

    // Conflicting OL anchor must NOT be absorbed.
    assert!(
        !confirmed.contains(&(AnchorType::OL_WORK, "OLY002W")),
        "D1: conflicting ol_key=OLY002W must NOT be absorbed; anchors={confirmed:?}"
    );

    // The non-conflicting GR anchor MUST be absorbed — this is the D1 behavior.
    assert!(
        confirmed.contains(&(AnchorType::GR_WORK, "99999")),
        "D1: gr_key=99999 must be absorbed despite the OL conflict; anchors={confirmed:?}"
    );
}

/// Companion to D1: a `User`-set confirmed anchor outranks machine values, so a
/// differing incoming value raises NO conflict — `detect_conflicting_anchors`
/// deliberately drops it (see the commented suppression rule + TODO(phase2-3)
/// in `sqlite_work_identity.rs`). The original anchor must survive untouched.
#[tokio::test]
async fn test_verify_d1_user_set_anchor_suppresses_conflict_by_design() {
    let db = common::create_test_db().await;

    let work_id = create_work(&db, "D1 User Anchor Book").await;
    db.confirm_anchor(
        work_id,
        AnchorType::new(AnchorType::OL_WORK),
        "OLX001W",
        AnchorSetter::User,
    )
    .await
    .expect("existing OL anchor should be confirmed");

    let incoming = CapturedIdentity {
        ol_key: Some("OLY002W".to_string()), // differs — but the user outranks it
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
        title: "D1 User Anchor Book".to_string(),
        author_name: "D1 Writer".to_string(),
        language: Some("en".to_string()),
    };

    let conflicts = db
        .detect_conflicting_anchors(work_id, &incoming, ConflictSource::ManualAdd)
        .await
        .expect("conflict detection should succeed");
    assert!(
        conflicts.is_empty(),
        "a User-set anchor must suppress the conflict by design; got {:?}",
        conflicts.iter().map(|c| &c.kind).collect::<Vec<_>>()
    );

    let anchors = db
        .list_anchors(work_id)
        .await
        .expect("anchor list should succeed");
    let confirmed: Vec<(&str, &str)> = anchors
        .iter()
        .filter(|a| a.confidence == AnchorConfidence::Confirmed)
        .map(|a| (a.anchor_type.as_str(), a.anchor_value.as_str()))
        .collect();
    assert!(
        confirmed.contains(&(AnchorType::OL_WORK, "OLX001W")),
        "the User-set ol_key must survive untouched; anchors={confirmed:?}"
    );
    assert!(
        !confirmed.contains(&(AnchorType::OL_WORK, "OLY002W")),
        "the differing machine value must NOT replace the User anchor; anchors={confirmed:?}"
    );
}
