//! identity-edit — DURABLE red-first pins (CC suite, runnable today).
//!
//! Contract: docs/design-identity-edit.md (r4, both-family PASS). This file references
//! ONLY surfaces that exist on current main, so it compiles now and fails RED now:
//! every test observes `works.identity_generation` (migration 076) or the 076 index
//! swap, none of which exist yet. Green requires: 076 applied by the test-DB migrator
//! AND every existing identity writer advancing the generation per the design's
//! "Claims and delayed completion" section.
//!
//! The new-surface suite (preview/commit/clear routes, classification authority,
//! writer race matrix, startup backfill) lives in test_identity_edit.rs behind the
//! `identity_edit_red` feature — see its header for the staged-red protocol.

use livrarr_behavioral::stubs::{create_second_test_user, create_test_user};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::{AnchorSetter, AnchorType};
use livrarr_domain::services::WorkIdentityRepository;
use livrarr_domain::{normalize_for_matching, IdentityStatus, UserId};
use sqlx::Row;

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

/// The observable the whole feature hangs on. RED today: the column does not exist,
/// so this query errors and the calling test fails.
async fn generation(db: &SqliteDb, work_id: i64) -> i64 {
    sqlx::query_scalar("SELECT identity_generation FROM works WHERE id = ?")
        .bind(work_id)
        .fetch_one(db.pool())
        .await
        .expect("works.identity_generation must exist (migration 076)")
}

/// Codex-merged shim: writer-bump tests install the future coordination column when an
/// old-schema fixture lacks it, so their RED points at the missing WRITER bump (the
/// contract under test), not at the missing column — the schema tests above keep the
/// column itself red. No-ops once migration 076 exists.
async fn ensure_identity_generation_column(db: &SqliteDb) {
    let columns = sqlx::query("PRAGMA table_info(works)")
        .fetch_all(db.pool())
        .await
        .expect("inspect works columns");
    let exists = columns
        .iter()
        .any(|row| row.get::<String, _>("name") == "identity_generation");
    if !exists {
        sqlx::query("ALTER TABLE works ADD COLUMN identity_generation INTEGER NOT NULL DEFAULT 0")
            .execute(db.pool())
            .await
            .expect("install future coordination column in an old-schema fixture");
    }
}

// ---------------------------------------------------------------------------
// Migration 076 schema (AC-21)
// ---------------------------------------------------------------------------

/// REQ-IDs: AC-21
/// Directive: works.identity_generation exists as INTEGER NOT NULL DEFAULT 0.
#[tokio::test]
async fn ac21_works_identity_generation_column_exists_not_null_default_zero() {
    let db = create_test_db().await;
    let row: Option<(String, i64, Option<String>)> = sqlx::query_as(
        "SELECT type, \"notnull\", dflt_value FROM pragma_table_info('works') \
         WHERE name = 'identity_generation'",
    )
    .fetch_optional(db.pool())
    .await
    .expect("pragma_table_info query");
    let (ty, notnull, dflt) =
        row.expect("identity_generation column must exist on works (migration 076)");
    assert_eq!(ty.to_uppercase(), "INTEGER");
    assert_eq!(notnull, 1, "identity_generation must be NOT NULL");
    assert_eq!(dflt.as_deref(), Some("0"));

    let user = create_test_user(&db).await;
    let work = create_work(&db, user, "Fresh Work").await;
    assert_eq!(
        generation(&db, work).await,
        0,
        "fresh work starts at generation 0"
    );
}

/// REQ-IDs: AC-21
/// Directive: the live all-type per-user index (044's uniq_user_confirmed_ol_anchor —
/// per-user over (user_id, anchor_type, anchor_value), NOT 041's global: 042 dropped
/// that and made 044's same-name CREATE real) is gone; the work-keys-only per-user
/// index exists.
#[tokio::test]
async fn ac21_index_swap_044_all_type_index_gone_per_user_work_key_index_present() {
    let db = create_test_db().await;
    let old: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='index' AND name='uniq_user_confirmed_ol_anchor'",
    )
    .fetch_optional(db.pool())
    .await
    .expect("sqlite_master query");
    assert!(
        old.is_none(),
        "044's all-type per-user index must be dropped by 076"
    );

    let new_sql: Option<String> = sqlx::query_scalar(
        "SELECT sql FROM sqlite_master WHERE type='index' AND name='uniq_user_confirmed_work_anchor'",
    )
    .fetch_optional(db.pool())
    .await
    .expect("sqlite_master query");
    let sql = new_sql.expect("076 must create uniq_user_confirmed_work_anchor");
    let sql_lc = sql.to_lowercase();
    assert!(sql_lc.contains("user_id"), "index must be per-user: {sql}");
    for key in ["ol_work", "gr_work", "hc_work"] {
        assert!(
            sql_lc.contains(key),
            "index predicate must include {key}: {sql}"
        );
    }
    assert!(
        !sql_lc.contains("isbn_13") && !sql_lc.contains("asin"),
        "bridges must NOT be in the uniqueness predicate: {sql}"
    );
}

/// REQ-IDs: AC-21
/// Directive: uniqueness semantics. Baseline truth (corrected 2026-07-24 by dual-suite
/// red-run divergence): the LIVE index is already per-user over ALL anchor types (044;
/// 042 dropped 041's global, so 044's CREATE was real). The 076 delta is therefore
/// exactly ONE arm: same-user shared BRIDGES become legal. Cross-user sharing and the
/// same-user work-key rejection are invariants that hold both before and after — pinned
/// here so 076 cannot regress them.
#[tokio::test]
async fn ac21_uniqueness_semantics_bridge_freedom_is_the_076_delta() {
    let db = create_test_db().await;
    let user_a = create_test_user(&db).await;
    let user_b = create_second_test_user(&db).await;
    let a1 = create_work(&db, user_a, "Alpha Book").await;
    let a2 = create_work(&db, user_a, "Beta Book").await;
    let b1 = create_work(&db, user_b, "Gamma Book").await;

    db.confirm_anchor(
        a1,
        AnchorType::new(AnchorType::GR_WORK),
        "777001",
        AnchorSetter::User,
    )
    .await
    .expect("first confirm");
    // Invariant (pre- and post-076): work keys are shareable across users.
    db.confirm_anchor(
        b1,
        AnchorType::new(AnchorType::GR_WORK),
        "777001",
        AnchorSetter::User,
    )
    .await
    .expect("cross-user same work key is legal (invariant since 044)");
    // Invariant (pre- and post-076): same-user duplicate work key rejected.
    assert!(
        db.confirm_anchor(
            a2,
            AnchorType::new(AnchorType::GR_WORK),
            "777001",
            AnchorSetter::User
        )
        .await
        .is_err(),
        "same-user duplicate work key must violate the per-user unique index"
    );
    // THE 076 delta (RED today — 044's live index still covers bridges):
    db.confirm_anchor(
        a1,
        AnchorType::new(AnchorType::ISBN_13),
        "9780441013593",
        AnchorSetter::User,
    )
    .await
    .expect("bridge on first work");
    db.confirm_anchor(
        a2,
        AnchorType::new(AnchorType::ISBN_13),
        "9780441013593",
        AnchorSetter::User,
    )
    .await
    .expect("same-user shared bridge must be accepted post-076");
}

// ---------------------------------------------------------------------------
// Generation advancement at the existing write chokepoints
// (design §Durable identity generation — every committed identity mutation advances it)
// ---------------------------------------------------------------------------

/// REQ-IDs: AC-6 (bump half), design §Claims
/// Directive: confirm_anchor (the chokepoint) advances identity_generation.
#[tokio::test]
async fn chokepoint_confirm_anchor_advances_generation() {
    let db = create_test_db().await;
    let user = create_test_user(&db).await;
    let work = create_work(&db, user, "Bump Confirm").await;
    ensure_identity_generation_column(&db).await;
    let before = generation(&db, work).await;
    db.confirm_anchor(
        work,
        AnchorType::new(AnchorType::OL_WORK),
        "OL123W",
        AnchorSetter::User,
    )
    .await
    .expect("confirm");
    let after = generation(&db, work).await;
    assert!(
        after > before,
        "confirm_anchor_in_tx must bump ({before} -> {after})"
    );
}

/// Post-cutover writer coverage: manual refresh must not read, rewrite, or
/// advance the retired legacy status/generation pair.
#[tokio::test]
async fn reset_for_manual_refresh_freezes_legacy_notfound_and_generation() {
    let db = create_test_db().await;
    let user = create_test_user(&db).await;
    let work = create_work(&db, user, "Bump Reset").await;
    ensure_identity_generation_column(&db).await;
    db.confirm_anchor(
        work,
        AnchorType::new(AnchorType::OL_WORK),
        "OL777W",
        AnchorSetter::Import,
    )
    .await
    .expect("seed anchor");
    livrarr_db::test_helpers::set_identity_status_fixture(
        &db,
        user,
        work,
        IdentityStatus::NotFound,
    )
    .await
    .expect("park not_found");
    let before = generation(&db, work).await;
    db.reset_for_manual_refresh(user, work)
        .await
        .expect("manual refresh reset");
    let status: String = sqlx::query_scalar("SELECT identity_status FROM works WHERE id = ?")
        .bind(work)
        .fetch_one(db.pool())
        .await
        .expect("status read");
    assert_eq!(status, "not_found", "the retired badge remains frozen");
    let after = generation(&db, work).await;
    assert_eq!(after, before, "refresh is not an identity settlement");
}
