#![allow(dead_code, unused_imports)]
//! Behavioral tests for english-work-lifecycle `work_db` directives.
//!
//! The stubs below (all `#[ignore = "not yet implemented"]`) predate Phase 5
//! and are out of this unit's scope. The real, running tests at the bottom
//! of this file are Phase 5 Unit E additions pinning REQ-014/ST-04: the
//! add-time adopt lookup and the stored identity key now derive from the
//! same `identity_matching::identity_key` recipe on both sides, so a
//! junk-tailed ("A Novel") or accented variant of a stored title can adopt
//! the existing anchorless work — the shape ST-04 named as permanently
//! broken under the old `normalize_for_matching` (caller) vs weak
//! trim+lowercase (DB helper) mismatch is now dead. The key encodes the
//! full parse triple (main + subtitle + volumes), so series siblings keep
//! DISTINCT keys: both persist under the unique index, and a sibling never
//! adopts into its shelf-mate.

/// REQ-IDs: REQ-005
/// Directive: Given user U with work W1 (no anchor, title='Cold Days', author='Jim Butcher'); INSERT populates normalized_title_key='cold days' and normalized_author_key='butcher jim': find(U, 'cold days', 'jim butcher') == Some(W1).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_work_db_find_normalized_match_no_anchor_for_user_user_u_work_w1_no_anchor_title_cold_days(
) {
    todo!()
}

/// REQ-IDs: REQ-005
/// Directive: Given user U with work W1 that HAS a confirmed ol_work anchor: find(U, 'cold days', 'jim butcher') == None (anti-join excludes).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_work_db_find_normalized_match_no_anchor_for_user_user_u_work_w1_that_confirmed_ol_work_anchor(
) {
    todo!()
}

/// REQ-IDs: REQ-005
/// Directive: Given user V with the same work as U: find(U, 'cold days', 'jim butcher') == None (user-scoped).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_work_db_find_normalized_match_no_anchor_for_user_user_v_same_work_u_find_u_cold_days(
) {
    todo!()
}

/// REQ-IDs: REQ-005
/// Directive: Token-order invariance: find(U, 'cold days', 'jim butcher') == find(U, 'days cold', 'butcher jim') == Some(W1) (sort-and-join canonicalizes).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_work_db_find_normalized_match_no_anchor_for_user_token_order_invariance_find_u_cold_days_jim_butcher(
) {
    todo!()
}

/// REQ-IDs: REQ-005
/// Directive: Degenerate input: find(U, '', 'jim butcher') == None (empty title key short-circuits).
#[tokio::test]
#[ignore = "stage-5: requires implementation or integration infrastructure not available in unit tests"]
async fn test_ewl_work_db_find_normalized_match_no_anchor_for_user_degenerate_input_find_u_jim_butcher_equals_empty_title(
) {
    todo!()
}

/// REQ-IDs: REQ-005
/// Directive: Paren-strip equivalence: insert work with title='Cold Days (Dresden Files, #14)'; find(U, 'cold days', 'jim butcher') == Some(...) (text_norm clean_title strips the paren on the input side; the stored column was computed via the same pipeline, so they match).
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_work_db_find_normalized_match_no_anchor_for_user_paren_strip_equivalence_insert_work_title_cold_days_dresden(
) {
    todo!()
}

// ===========================================================================
// Phase 5 Unit E — REQ-014/ST-04: identity_key on both sides of the adopt
// lookup (real, running tests; see module doc comment above).
// ===========================================================================

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateUserDbRequest, CreateWorkDbRequest, UserDb, UserRole, WorkDb, WorkDbCreate,
};
use livrarr_domain::identity_matching::identity_key;

async fn setup_user(db: &SqliteDb) -> i64 {
    db.create_user(CreateUserDbRequest {
        username: "ewl-adopt-user".into(),
        password_hash: "hash".into(),
        role: UserRole::User,
        api_key_hash: "ewl-adopt-key".into(),
    })
    .await
    .unwrap()
    .id
}

/// Insert a work the way `WorkService::add()` does post-Phase-5: both
/// `normalized_title`/`normalized_author` computed via `identity_key`.
async fn insert_work_via_identity_key(db: &SqliteDb, user_id: i64, title: &str, author: &str) {
    let (normalized_title, normalized_author) = identity_key(title, author);
    db.create_work(CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author.to_string(),
        normalized_title,
        normalized_author,
        ..Default::default()
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn junk_tail_incoming_title_adopts_bare_stored_work() {
    // ST-04 was: the stored key came from a fuller recipe than the raw
    // query the adopt lookup passed in, so a variant-form incoming title
    // could never match. Post-fix, both sides route through identity_key,
    // and a JUNK-tail variant ("A Novel" — stripped by the parse, never in
    // the key) of a stored bare title now finds the existing anchorless
    // work. (A true-subtitle or volume-marker tail is NOT this shape: it
    // enters the key triple and correctly misses here — see the
    // sibling-adopt-guard test below.)
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    insert_work_via_identity_key(&db, user_id, "Storm Front", "Jim Butcher").await;

    let (query_title, query_author) = identity_key("Storm Front: A Novel", "Jim Butcher");
    let found = db
        .find_normalized_match_no_anchor_for_user(user_id, &query_title, &query_author)
        .await
        .unwrap();
    assert!(
        found.is_some(),
        "junk-tail incoming title must adopt the bare stored work (ST-04 dead)"
    );
    assert_eq!(found.unwrap().title, "Storm Front");
}

#[tokio::test]
async fn differently_cased_and_accented_incoming_title_adopts() {
    // The old recipe kept accents/case sensitivity effectively neutralized
    // only by lowercasing, not by accent-stripping; identity_key strips
    // accents on both sides, so this also now adopts.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    insert_work_via_identity_key(&db, user_id, "Cafe World", "J. Author").await;

    let (query_title, query_author) = identity_key("CAFÉ WORLD", "j. author");
    let found = db
        .find_normalized_match_no_anchor_for_user(user_id, &query_title, &query_author)
        .await
        .unwrap();
    assert!(found.is_some());
}

#[tokio::test]
async fn confirmed_anchor_excludes_from_adopt_lookup() {
    // The anti-join over work_identity_anchors (confidence='confirmed',
    // anchor_type='ol_work') is untouched by the identity_key routing fix —
    // pinned here so a future change can't silently regress it.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (normalized_title, normalized_author) = identity_key("Anchored Work", "Some Author");
    let (work, _created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Anchored Work".to_string(),
            author_name: "Some Author".to_string(),
            normalized_title,
            normalized_author,
            ol_key: Some("OL999W".to_string()),
            ..Default::default()
        })
        .await
        .unwrap();
    use livrarr_domain::identity::AnchorType;
    use livrarr_domain::services::WorkIdentityRepository;
    db.confirm_anchor(
        work.id,
        AnchorType::new(AnchorType::OL_WORK),
        "OL999W",
        livrarr_domain::identity::AnchorSetter::AutoSearch,
    )
    .await
    .unwrap();

    let (query_title, query_author) = identity_key("Anchored Work", "Some Author");
    let found = db
        .find_normalized_match_no_anchor_for_user(user_id, &query_title, &query_author)
        .await
        .unwrap();
    assert!(
        found.is_none(),
        "a work with a confirmed ol_work anchor must be excluded from the adopt lookup"
    );
}

#[tokio::test]
async fn series_siblings_both_persist_under_the_unique_key() {
    // The identity-key triple keeps series siblings distinct: two works
    // sharing a main title but differing in subtitle (and two differing in
    // volume) BOTH insert as their own rows — the ON CONFLICT DO NOTHING
    // backstop never swallows the second sibling.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;

    // Subtitle siblings.
    let (nt_a, na_a) = identity_key("Mistborn: The Final Empire", "Brandon Sanderson");
    let (work_a, created_a) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Mistborn: The Final Empire".to_string(),
            author_name: "Brandon Sanderson".to_string(),
            normalized_title: nt_a,
            normalized_author: na_a,
            ..Default::default()
        })
        .await
        .unwrap();
    let (nt_b, na_b) = identity_key("Mistborn: The Well of Ascension", "Brandon Sanderson");
    let (work_b, created_b) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Mistborn: The Well of Ascension".to_string(),
            author_name: "Brandon Sanderson".to_string(),
            normalized_title: nt_b,
            normalized_author: na_b,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(created_a && created_b, "both subtitle siblings must insert");
    assert_ne!(work_a.id, work_b.id);

    // Volume siblings.
    let (nt_c, na_c) = identity_key("History of Rome: Volume 1", "Mike Duncan");
    let (work_c, created_c) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "History of Rome: Volume 1".to_string(),
            author_name: "Mike Duncan".to_string(),
            normalized_title: nt_c,
            normalized_author: na_c,
            ..Default::default()
        })
        .await
        .unwrap();
    let (nt_d, na_d) = identity_key("History of Rome: Volume 2", "Mike Duncan");
    let (work_d, created_d) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "History of Rome: Volume 2".to_string(),
            author_name: "Mike Duncan".to_string(),
            normalized_title: nt_d,
            normalized_author: na_d,
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(created_c && created_d, "both volume siblings must insert");
    assert_ne!(work_c.id, work_d.id);
}

#[tokio::test]
async fn sibling_never_adopts_into_the_other_at_the_adopt_seat() {
    // The adopt lookup is exact key equality: a sibling's key (different
    // subtitle or volume segment) misses the stored work, so a sibling can
    // never adopt/absorb into its shelf-mate. It falls to the dedup
    // cascade, which lands grey (visible duplicate) — never a silent merge.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    insert_work_via_identity_key(
        &db,
        user_id,
        "Mistborn: The Final Empire",
        "Brandon Sanderson",
    )
    .await;
    insert_work_via_identity_key(&db, user_id, "Storm Front", "Jim Butcher").await;

    // Subtitle sibling misses.
    let (qt, qa) = identity_key("Mistborn: The Well of Ascension", "Brandon Sanderson");
    let found = db
        .find_normalized_match_no_anchor_for_user(user_id, &qt, &qa)
        .await
        .unwrap();
    assert!(
        found.is_none(),
        "a subtitle sibling must miss at the adopt seat"
    );

    // One-sided volume-marker pair misses too (the Dresden shape): the
    // marker enters the query key's volume segment, the stored bare title
    // has none.
    let (qt, qa) = identity_key("Storm Front: The Dresden Files, Book 1", "Jim Butcher");
    let found = db
        .find_normalized_match_no_anchor_for_user(user_id, &qt, &qa)
        .await
        .unwrap();
    assert!(
        found.is_none(),
        "a one-sided volume-marker title must miss at the adopt seat (grey at the cascade)"
    );
}
