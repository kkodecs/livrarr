//! identity-edit fix unit — red-first pins.
//!
//! Contract: `docs/design-identity-edit-fixes.md` (r1), which repairs defects the
//! `a7f03540` merge knowingly carried. Every test here fails on that commit and passes
//! once its fix lands. Numbering follows the design's F-items.
//!
//! All surfaces referenced already exist, so this file compiles ungated — its RED points
//! at behavior, never at a missing type.

use livrarr_behavioral::stubs::create_test_user;
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{backfill_work_identity_ledger, CreateWorkDbRequest, WorkDbCreate};
use livrarr_domain::identity::{AnchorSetter, AnchorType};
use livrarr_domain::identity_edit::classify_identifier_input;
use livrarr_domain::services::WorkIdentityRepository;
use livrarr_domain::{normalize_for_matching, UserId};

async fn create_work(db: &SqliteDb, user_id: UserId, title: &str) -> i64 {
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: title.to_string(),
            author_name: "Test Author".to_string(),
            normalized_title: normalize_for_matching(title),
            normalized_author: normalize_for_matching("Test Author"),
            ..Default::default()
        })
        .await
        .expect("create test work");
    work.id
}

async fn confirmed_owner_count(db: &SqliteDb, work_id: i64, anchor_type: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors \
         WHERE work_id = ?1 AND anchor_type = ?2 AND confidence = 'confirmed'",
    )
    .bind(work_id)
    .bind(anchor_type)
    .fetch_one(db.pool())
    .await
    .expect("count confirmed anchors")
}

// ---------------------------------------------------------------------------
// F3 — startup backfill must see an owner that exists only in the ledger
// ---------------------------------------------------------------------------

/// Design: `docs/design-identity-edit-fixes.md` F3.
///
/// Constructed-state justification (CLAUDE.md "Tests drive the real door"): the door
/// under test IS `backfill_work_identity_ledger` running against arbitrary pre-existing
/// data — that is its entire contract. The legacy shape it must survive (a confirmed
/// ledger row whose denormalized column is NULL) is by definition one today's writers no
/// longer produce, so it cannot be reached forward through them. The ledger row itself is
/// still written by the real production writer (`confirm_anchor`); only the legacy column
/// state is set directly.
///
/// RED on a7f03540: the pass builds its owner map from works that have a non-NULL legacy
/// column, so the true owner is invisible, the column-only work is elected, and the
/// second confirmed insert violates `uniq_user_confirmed_work_anchor` — backfill returns
/// Err and **startup fails**.
#[tokio::test]
async fn f3_backfill_preserves_a_confirmed_owner_that_exists_only_in_the_ledger() {
    let db = create_test_db().await;
    let user = create_test_user(&db).await;

    // Lower id, legacy column only, no ledger row — the work "lowest id wins" would elect.
    let column_only = create_work(&db, user, "Column Only").await;
    sqlx::query("UPDATE works SET gr_key = '777123' WHERE id = ?")
        .bind(column_only)
        .execute(db.pool())
        .await
        .expect("seed legacy column");

    // Higher id, real confirmed ledger row, every legacy column NULL.
    let ledger_owner = create_work(&db, user, "Ledger Owner").await;
    db.confirm_anchor(
        ledger_owner,
        AnchorType::new(AnchorType::GR_WORK),
        "777123",
        AnchorSetter::User,
    )
    .await
    .expect("real writer confirms the ledger row");
    sqlx::query("UPDATE works SET gr_key = NULL WHERE id = ?")
        .bind(ledger_owner)
        .execute(db.pool())
        .await
        .expect("reproduce the legacy ledger-only shape");

    backfill_work_identity_ledger(db.pool())
        .await
        .expect("backfill must not fail startup when the owner is ledger-only");

    assert_eq!(
        confirmed_owner_count(&db, ledger_owner, AnchorType::GR_WORK).await,
        1,
        "the existing ledger-only owner must be preserved"
    );
    assert_eq!(
        confirmed_owner_count(&db, column_only, AnchorType::GR_WORK).await,
        0,
        "a column-only work must not be elected owner over an existing ledger owner"
    );

    let kept: Option<String> = sqlx::query_scalar("SELECT gr_key FROM works WHERE id = ?")
        .bind(column_only)
        .fetch_one(db.pool())
        .await
        .expect("loser column");
    assert_eq!(
        kept.as_deref(),
        Some("777123"),
        "the loser keeps its column — visible and clearable, never silently dropped"
    );
}

// ---------------------------------------------------------------------------
// F4a — a pending row is state whatever its value
// ---------------------------------------------------------------------------

/// Design: `docs/design-identity-edit-fixes.md` F4a.
///
/// The `clear_identity_slot` trait contract
/// (`crates/livrarr-domain/src/services/work.rs:529-530`) defines an empty slot as "no
/// confirmed row, no nonempty column, and **no pending row**" — presence, not value.
///
/// Constructed-state justification: today's `record_pending_anchor` rejects an empty
/// value outright (`crates/livrarr-db/src/sqlite_work_identity.rs:849-851`), so this row
/// shape is reachable only from legacy or corrupted data — not forward through any
/// current writer. **Scope note:** that makes F4a a robustness repair against the stated
/// contract, not a bug a user can hit on data this build produced. The clear itself runs
/// through the real production repository door.
///
/// RED on a7f03540: clear filters the empty value out, reports `EmptySlot`, and leaves
/// the row behind — so the slot can never be cleaned up.
#[tokio::test]
async fn f4a_clear_removes_a_pending_row_whose_value_is_empty() {
    let db = create_test_db().await;
    let user = create_test_user(&db).await;
    let work = create_work(&db, user, "Stuck Pending").await;

    sqlx::query(
        "INSERT INTO work_identity_anchors \
         (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id) \
         VALUES (?1, 'isbn_13', '', 'pending', 'auto_search', '2026-01-01T00:00:00Z', ?2)",
    )
    .bind(work)
    .bind(user)
    .execute(db.pool())
    .await
    .expect("seed the legacy empty pending row");

    db.apply_identity_clear(work, user, AnchorType::new(AnchorType::ISBN_13))
        .await
        .expect("a slot holding a pending row is not empty and must clear");

    let left: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM work_identity_anchors \
         WHERE work_id = ?1 AND anchor_type = 'isbn_13'",
    )
    .bind(work)
    .fetch_one(db.pool())
    .await
    .expect("count remaining rows");
    assert_eq!(left, 0, "clear must delete the pending row it reported on");
}

// ---------------------------------------------------------------------------
// F5b — URL classification must match the real host, not any substring
// ---------------------------------------------------------------------------

/// Design: `docs/design-identity-edit-fixes.md` F5b.
///
/// RED on a7f03540: `url_segment` lowercases the whole input and tests
/// `contains("goodreads.com")`, so a provider domain appearing anywhere — a query
/// parameter, a path, a fragment — is accepted as if it were the URL's host.
#[test]
fn f5b_a_provider_domain_outside_the_host_does_not_classify() {
    // The domain appears only in a query parameter of an unrelated host.
    let err = classify_identifier_input(
        "https://evil.example/?next=goodreads.com/book/show/12345",
        None,
    );
    assert!(
        err.is_err(),
        "a non-Goodreads host must not classify as Goodreads: {err:?}"
    );

    // Same shape for the other two providers.
    assert!(classify_identifier_input(
        "https://evil.example/?u=openlibrary.org/works/OL123W",
        None
    )
    .is_err());
    assert!(
        classify_identifier_input("https://evil.example/?u=amazon.com/dp/B00TEST123", None)
            .is_err()
    );
}

/// Real provider URLs must keep classifying — the F5b fix must not narrow them.
#[test]
fn f5b_real_provider_urls_still_classify() {
    let (slot, value) =
        classify_identifier_input("https://www.goodreads.com/book/show/12345", None)
            .expect("canonical Goodreads URL");
    assert_eq!(slot.as_str(), AnchorType::GR_WORK);
    assert_eq!(value, "12345");

    let (slot, value) =
        classify_identifier_input("https://openlibrary.org/works/OL123W/Some-Title", None)
            .expect("canonical OpenLibrary URL");
    assert_eq!(slot.as_str(), AnchorType::OL_WORK);
    assert_eq!(value, "OL123W");

    // Regional Amazon domains are in scope; the trailing-dot pattern must survive.
    let (slot, _) = classify_identifier_input("https://www.amazon.co.uk/dp/B00TEST123", None)
        .expect("regional Amazon URL");
    assert_eq!(slot.as_str(), AnchorType::ASIN);

    // A subdomain of the provider is still the provider.
    let (slot, value) = classify_identifier_input("https://m.goodreads.com/book/show/999", None)
        .expect("provider subdomain");
    assert_eq!(slot.as_str(), AnchorType::GR_WORK);
    assert_eq!(value, "999");
}

/// A lookalike domain must not pass as the provider.
#[test]
fn f5b_lookalike_domains_do_not_classify() {
    assert!(
        classify_identifier_input("https://notgoodreads.com/book/show/12345", None).is_err(),
        "a suffix-only lookalike must not classify"
    );
    assert!(
        classify_identifier_input("https://goodreads.com.evil.example/book/show/12345", None)
            .is_err(),
        "the provider domain as a left label of another host must not classify"
    );
}
