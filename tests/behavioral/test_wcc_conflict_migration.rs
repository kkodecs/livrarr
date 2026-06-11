#![allow(dead_code, unused_imports, clippy::type_complexity)]

//! Behavioral tests for work-creation-consistency conflict-store federation.

#[path = "common.rs"]
mod common;

use chrono::Utc;
use livrarr_db::{CreateWorkDbRequest, WorkDbCreate};
use livrarr_domain::identity::{ConflictSource, IdentityConflictKind};
use livrarr_domain::{UserId, UserRole, WorkId};

async fn create_user(db: &livrarr_db::sqlite::SqliteDb, username: &str) -> UserId {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "INSERT INTO users (username, password_hash, role, api_key_hash, created_at, updated_at) \
         VALUES (?, 'hash', 'user', ?, ?, ?)",
    )
    .bind(username)
    .bind(format!("{username}-key"))
    .bind(&now)
    .bind(&now)
    .execute(db.pool())
    .await
    .expect("test user should insert");
    result.last_insert_rowid()
}

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
    assert!(inserted);
    work.id
}

async fn recreate_legacy_conflict_table(db: &livrarr_db::sqlite::SqliteDb) {
    sqlx::raw_sql(
        r#"
        DROP TABLE IF EXISTS work_identity_conflicts;
        CREATE TABLE work_identity_conflicts (
            id                    INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id               INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            existing_work_id      INTEGER NOT NULL REFERENCES works(id) ON DELETE CASCADE,
            kind                  TEXT NOT NULL CHECK (kind IN ('incoming_different_ol_key', 'ol_redirect_collision')),
            incoming_payload_json TEXT NOT NULL,
            raised_at             TEXT NOT NULL,
            raised_by             TEXT NOT NULL CHECK (raised_by IN ('manual_add', 'manual_import', 'list_import', 'readarr_import', 'author_monitor', 'refresh')),
            raised_source_path    TEXT,
            status                TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'resolved', 'dismissed')),
            resolved_at           TEXT,
            resolution_action     TEXT CHECK (resolution_action IN ('keep_existing', 'accept_separate', 'replace_ol_key', 'merge') OR resolution_action IS NULL),
            resolution_notes      TEXT
        );
        CREATE INDEX idx_identity_conflicts_user_status
            ON work_identity_conflicts(user_id, status);
        CREATE INDEX idx_identity_conflicts_work
            ON work_identity_conflicts(existing_work_id);
        "#,
    )
    .execute(db.pool())
    .await
    .expect("legacy conflict table should be recreated");
}

/// REQ-IDs: REQ-020, cR-001
/// Directive: migration 052 preserves rows for multiple users and maps replace_ol_key to replace_anchor.
#[tokio::test]

async fn test_wcc_conflict_migration_req_020_cr_001_migration_052_preserves_rows_and_maps_replace_anchor(
) {
    let db = common::create_test_db().await;
    let user_one = create_user(&db, "wcc-conflict-user-one").await;
    let user_two = create_user(&db, "wcc-conflict-user-two").await;
    let work_one = create_work(&db, user_one, "Legacy Conflict One").await;
    let work_two = create_work(&db, user_two, "Legacy Conflict Two").await;

    recreate_legacy_conflict_table(&db).await;

    sqlx::query(
        "INSERT INTO work_identity_conflicts \
         (id, user_id, existing_work_id, kind, incoming_payload_json, raised_at, raised_by, status, resolved_at, resolution_action, resolution_notes) \
         VALUES \
         (101, ?, ?, 'incoming_different_ol_key', '{\"ol_key\":\"OLX1W\"}', '2026-05-29T00:00:00Z', 'manual_add', 'open', NULL, 'replace_ol_key', 'keep the new anchor'), \
         (202, ?, ?, 'ol_redirect_collision', '{\"ol_key\":\"OLX2W\"}', '2026-05-29T00:01:00Z', 'list_import', 'resolved', '2026-05-29T00:02:00Z', 'replace_ol_key', 'resolved legacy row')",
    )
    .bind(user_one)
    .bind(work_one)
    .bind(user_two)
    .bind(work_two)
    .execute(db.pool())
    .await
    .expect("legacy conflict rows should seed");

    let before_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM work_identity_conflicts")
        .fetch_one(db.pool())
        .await
        .expect("pre-migration count should query");
    assert_eq!(before_count, 2);

    sqlx::raw_sql(include_str!(
        "../../crates/livrarr-db/migrations/052_federate_identity_conflicts.sql"
    ))
    .execute(db.pool())
    .await
    .expect("migration 052 should run");

    let after_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM work_identity_conflicts")
        .fetch_one(db.pool())
        .await
        .expect("post-migration count should query");
    assert_eq!(after_count, before_count);

    let rows: Vec<(i64, UserId, WorkId, String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, user_id, existing_work_id, status, resolution_action, incoming_payload_json, resolved_at \
         FROM work_identity_conflicts ORDER BY id",
    )
    .fetch_all(db.pool())
    .await
    .expect("post-migration rows should query");

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, 101);
    assert_eq!(rows[0].1, user_one);
    assert_eq!(rows[0].2, work_one);
    assert_eq!(rows[0].3, "open");
    assert_eq!(rows[0].4, "replace_anchor");
    assert_eq!(rows[0].5, "{\"ol_key\":\"OLX1W\"}");
    assert_eq!(rows[0].6, None);

    assert_eq!(rows[1].0, 202);
    assert_eq!(rows[1].1, user_two);
    assert_eq!(rows[1].2, work_two);
    assert_eq!(rows[1].3, "resolved");
    assert_eq!(rows[1].4, "replace_anchor");
    assert_eq!(rows[1].5, "{\"ol_key\":\"OLX2W\"}");
    assert_eq!(rows[1].6.as_deref(), Some("2026-05-29T00:02:00Z"));
}

/// REQ-IDs: REQ-020, AC-035
/// Directive: raise_identity_conflict stores federated GR/HC conflict kinds as observable rows.
#[tokio::test]

async fn test_wcc_conflict_repo_req_020_ac_035_raise_identity_conflict_accepts_federated_gr_key_kind(
) {
    let db = common::create_test_db().await;
    let user_id = create_user(&db, "wcc-conflict-raise").await;
    let work_id = create_work(&db, user_id, "Federated Conflict Row").await;

    let conflict_id = db
        .create_identity_conflict(
            user_id,
            work_id,
            IdentityConflictKind::IncomingDifferentGrKey,
            r#"{"gr_key":"12345","title":"Federated Conflict Row","author_name":"Case Writer"}"#,
            Utc::now(),
            ConflictSource::SeriesMonitor,
            Some("/series/42"),
        )
        .await
        .expect("federated GR conflict kind should insert");

    assert!(conflict_id > 0);

    let row: (String, String, String, String, Option<String>) = sqlx::query_as(
        "SELECT kind, raised_by, status, incoming_payload_json, raised_source_path \
         FROM work_identity_conflicts WHERE id = ?",
    )
    .bind(conflict_id)
    .fetch_one(db.pool())
    .await
    .expect("conflict row should be queryable");

    assert_eq!(row.0, "incoming_different_gr_key");
    assert_eq!(row.1, "series_monitor");
    assert_eq!(row.2, "open");
    assert!(row.3.contains("\"gr_key\":\"12345\""));
    assert_eq!(row.4.as_deref(), Some("/series/42"));
}
