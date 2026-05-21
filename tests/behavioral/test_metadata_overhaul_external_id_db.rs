#![allow(dead_code)]
//! Behavioral tests for `ExternalIdDb` against a real SQLite `:memory:` database
//! with full migrations applied.
//!
//! Covers R-06 contract behaviors for single upsert, batch upsert, listing,
//! uniqueness/no-op semantics for duplicates, accepted external ID types,
//! user isolation, and coexistence of different ID types on the same work.

use std::collections::HashSet;

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::*;
use livrarr_domain::ExternalIdType;

pub trait DbTestHarness: Send + Sync {
    type Db: ExternalIdDb + WorkDb + WorkDbCreate + Send + Sync;

    fn db(&self) -> &Self::Db;
    fn user_ids(&self) -> (UserId, UserId);
}

struct ExternalIdDbHarness {
    db: SqliteDb,
    user_ids: (UserId, UserId),
}

impl DbTestHarness for ExternalIdDbHarness {
    type Db = SqliteDb;

    fn db(&self) -> &Self::Db {
        &self.db
    }

    fn user_ids(&self) -> (UserId, UserId) {
        self.user_ids
    }
}

async fn setup_harness() -> impl DbTestHarness {
    use livrarr_db::{CreateUserDbRequest, UserDb};
    use livrarr_domain::UserRole;

    let db = SqliteDb::new_test().await;
    // User 1 is the placeholder admin from migration 001.
    // Create user 2 for cross-user isolation tests.
    db.create_user(CreateUserDbRequest {
        username: "external_id_user_2".to_string(),
        password_hash: "hash2".to_string(),
        role: UserRole::User,
        api_key_hash: "api_key_hash_2".to_string(),
    })
    .await
    .unwrap();
    ExternalIdDbHarness {
        db,
        user_ids: (1, 2),
    }
}

fn make_work_req(user_id: UserId, title: &str, author: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author.to_string(),
        normalized_title: livrarr_domain::normalize_for_matching(title),
        normalized_author: livrarr_domain::normalize_for_matching(author),
        author_id: None,
        ol_key: None,
        year: Some(2024),
        cover_url: Some("https://example.test/cover.jpg".to_string()),
        ..Default::default()
    }
}

async fn make_work_id<DB: WorkDb + WorkDbCreate>(db: &DB, user_id: UserId, title: &str) -> WorkId {
    db.create_work(make_work_req(user_id, title, "Contract Author"))
        .await
        .unwrap()
        .0
        .id
}

fn make_external_id_req(
    work_id: WorkId,
    id_type: ExternalIdType,
    id_value: &str,
) -> UpsertExternalIdRequest {
    UpsertExternalIdRequest {
        work_id,
        id_type,
        id_value: id_value.to_string(),
    }
}

fn external_id_set(ids: &[ExternalId]) -> std::collections::HashSet<(ExternalIdType, String)> {
    ids.iter()
        .map(|e| (e.id_type, e.id_value.clone()))
        .collect()
}

fn expected_external_id_set(rows: &[(ExternalIdType, &str)]) -> HashSet<(ExternalIdType, String)> {
    rows.iter()
        .map(|(id_type, id_value)| (*id_type, (*id_value).to_string()))
        .collect()
}

fn has_external_id(rows: &[ExternalId], id_type: ExternalIdType, id_value: &str) -> bool {
    rows.iter()
        .any(|row| row.id_type == id_type && row.id_value == id_value)
}

macro_rules! external_id_db_tests {
    () => {
        #[tokio::test]
        async fn test_external_id_db_upsert_external_id_writes_new_row_nominal() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::upsert_external_id | Behavior: writes a new external ID row for the requested work
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work_id = make_work_id(db, u1, "Single Upsert Work").await;
            let req = make_external_id_req(work_id, ExternalIdType::Isbn13, "9781234567890");

            db.upsert_external_id(u1, req).await.unwrap();

            let rows = db.list_external_ids(u1, work_id).await.unwrap();
            assert_eq!(rows.len(), 1);

            let row = &rows[0];
            assert_eq!(row.user_id, u1);
            assert_eq!(row.work_id, work_id);
            assert_eq!(row.id_type, ExternalIdType::Isbn13);
            assert_eq!(row.id_value, "9781234567890");
        }

        #[tokio::test]
        async fn test_external_id_db_upsert_external_ids_batch_writes_multiple_rows_nominal() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::upsert_external_ids_batch | Behavior: successful batch write persists all provided external ID rows for a work
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work_id = make_work_id(db, u1, "Batch Upsert Work").await;
            let reqs = vec![
                make_external_id_req(work_id, ExternalIdType::Isbn13, "9781234567890"),
                make_external_id_req(work_id, ExternalIdType::Isbn10, "1234567890"),
                make_external_id_req(work_id, ExternalIdType::Asin, "B00TEST123"),
            ];

            db.upsert_external_ids_batch(u1, reqs).await.unwrap();

            let rows = db.list_external_ids(u1, work_id).await.unwrap();
            assert_eq!(rows.len(), 3);
            assert!(rows.iter().all(|row| row.user_id == u1 && row.work_id == work_id));
            assert_eq!(
                external_id_set(&rows),
                expected_external_id_set(&[
                    (ExternalIdType::Isbn13, "9781234567890"),
                    (ExternalIdType::Isbn10, "1234567890"),
                    (ExternalIdType::Asin, "B00TEST123"),
                ])
            );
        }

        #[tokio::test]
        async fn test_external_id_db_list_external_ids_returns_all_rows_for_work_nominal() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::list_external_ids | Behavior: returns every external ID stored for the requested work and excludes rows for other works
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work_id_one = make_work_id(db, u1, "List Target Work").await;
            let work_id_two = make_work_id(db, u1, "List Other Work").await;

            db.upsert_external_id(
                u1,
                make_external_id_req(work_id_one, ExternalIdType::Isbn13, "9781111111111"),
            )
            .await
            .unwrap();
            db.upsert_external_id(
                u1,
                make_external_id_req(work_id_one, ExternalIdType::Asin, "B00WORKONE"),
            )
            .await
            .unwrap();
            db.upsert_external_id(
                u1,
                make_external_id_req(work_id_two, ExternalIdType::Isbn10, "2222222222"),
            )
            .await
            .unwrap();

            let rows = db.list_external_ids(u1, work_id_one).await.unwrap();

            assert_eq!(rows.len(), 2);
            assert!(rows.iter().all(|row| row.work_id == work_id_one));
            assert_eq!(
                external_id_set(&rows),
                expected_external_id_set(&[
                    (ExternalIdType::Isbn13, "9781111111111"),
                    (ExternalIdType::Asin, "B00WORKONE"),
                ])
            );
        }

        #[tokio::test]
        async fn test_external_id_db_upsert_external_id_duplicate_is_noop_not_error_invariant() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::upsert_external_id | Behavior: upserting the same (work_id, id_type, id_value) twice succeeds without error
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work_id = make_work_id(db, u1, "Duplicate No-Error Work").await;
            let req = make_external_id_req(work_id, ExternalIdType::Isbn13, "9789999999999");

            db.upsert_external_id(u1, req.clone()).await.unwrap();
            let second = db.upsert_external_id(u1, req).await;

            assert!(second.is_ok());
        }

        #[tokio::test]
        async fn test_external_id_db_upsert_external_id_same_triple_twice_lists_exactly_one_row_invariant() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::upsert_external_id | Behavior: duplicate upsert of the same triple is a no-op and list returns exactly one row
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work_id = make_work_id(db, u1, "Duplicate Dedup Work").await;
            let req = make_external_id_req(work_id, ExternalIdType::Asin, "B00DEDUP01");

            db.upsert_external_id(u1, req.clone()).await.unwrap();
            db.upsert_external_id(u1, req).await.unwrap();

            let rows = db.list_external_ids(u1, work_id).await.unwrap();

            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].user_id, u1);
            assert_eq!(rows[0].work_id, work_id);
            assert_eq!(rows[0].id_type, ExternalIdType::Asin);
            assert_eq!(rows[0].id_value, "B00DEDUP01");
        }

        #[tokio::test]
        async fn test_external_id_db_upsert_external_id_same_id_pair_on_different_works_both_succeed_invariant() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::upsert_external_id | Behavior: the same (id_type, id_value) may be stored for different works because uniqueness is scoped per work
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work_id_one = make_work_id(db, u1, "Shared External ID Work One").await;
            let work_id_two = make_work_id(db, u1, "Shared External ID Work Two").await;
            let shared_type = ExternalIdType::Isbn13;
            let shared_value = "9788888888888";

            db.upsert_external_id(
                u1,
                make_external_id_req(work_id_one, shared_type, shared_value),
            )
            .await
            .unwrap();
            db.upsert_external_id(
                u1,
                make_external_id_req(work_id_two, shared_type, shared_value),
            )
            .await
            .unwrap();

            let rows_one = db.list_external_ids(u1, work_id_one).await.unwrap();
            let rows_two = db.list_external_ids(u1, work_id_two).await.unwrap();

            assert_eq!(rows_one.len(), 1);
            assert_eq!(rows_two.len(), 1);
            assert_eq!(rows_one[0].work_id, work_id_one);
            assert_eq!(rows_two[0].work_id, work_id_two);
            assert_eq!(rows_one[0].id_type, shared_type);
            assert_eq!(rows_two[0].id_type, shared_type);
            assert_eq!(rows_one[0].id_value, shared_value);
            assert_eq!(rows_two[0].id_value, shared_value);
        }

        #[tokio::test]
        async fn test_external_id_db_upsert_external_ids_batch_empty_vec_succeeds_boundary() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::upsert_external_ids_batch | Behavior: empty batch succeeds and writes no external ID rows
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work_id = make_work_id(db, u1, "Empty Batch Work").await;

            db.upsert_external_ids_batch(u1, Vec::new()).await.unwrap();

            let rows = db.list_external_ids(u1, work_id).await.unwrap();
            assert!(rows.is_empty());
        }

        #[tokio::test]
        async fn test_external_id_db_upsert_external_ids_batch_existing_duplicate_and_new_entry_succeeds_invariant() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::upsert_external_ids_batch | Behavior: a batch containing one already-existing triple and one new entry succeeds, preserves a single duplicate row, and inserts the new row
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work_id = make_work_id(db, u1, "Batch Existing Duplicate Plus New Work").await;

            db.upsert_external_id(
                u1,
                make_external_id_req(work_id, ExternalIdType::Isbn13, "9786666666666"),
            )
            .await
            .unwrap();

            let reqs = vec![
                make_external_id_req(work_id, ExternalIdType::Isbn13, "9786666666666"),
                make_external_id_req(work_id, ExternalIdType::Asin, "B00BATCHNEW"),
            ];

            let result = db.upsert_external_ids_batch(u1, reqs).await;

            assert!(result.is_ok());

            let rows = db.list_external_ids(u1, work_id).await.unwrap();
            assert_eq!(rows.len(), 2);
            assert_eq!(
                external_id_set(&rows),
                expected_external_id_set(&[
                    (ExternalIdType::Isbn13, "9786666666666"),
                    (ExternalIdType::Asin, "B00BATCHNEW"),
                ])
            );
        }

        #[tokio::test]
        async fn test_external_id_db_list_external_ids_returns_empty_vec_when_work_has_no_rows_boundary() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::list_external_ids | Behavior: listing external IDs for a work with no stored external IDs returns an empty vector
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work_id = make_work_id(db, u1, "No External IDs Work").await;

            let rows = db.list_external_ids(u1, work_id).await.unwrap();

            assert!(rows.is_empty());
        }

        #[tokio::test]
        async fn test_external_id_db_list_external_ids_does_not_expose_other_users_rows_isolation() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::list_external_ids | Behavior: list results are user-scoped; cross-user access may return either an empty vector or an error, and the owner's rows remain unchanged
            let h = setup_harness().await;
            let db = h.db();
            let (u1, u2) = h.user_ids();

            let work_id_user_two = make_work_id(db, u2, "User Two Work").await;

            db.upsert_external_id(
                u2,
                make_external_id_req(work_id_user_two, ExternalIdType::Asin, "B00USER2ID"),
            )
            .await
            .unwrap();

            let own_rows_before = db.list_external_ids(u2, work_id_user_two).await.unwrap();
            assert_eq!(own_rows_before.len(), 1);
            assert_eq!(own_rows_before[0].user_id, u2);
            assert_eq!(own_rows_before[0].id_value, "B00USER2ID");

            let cross_user_result = db.list_external_ids(u1, work_id_user_two).await;
            match cross_user_result {
                Ok(rows) => assert!(rows.is_empty()),
                Err(_) => {}
            }

            let own_rows_after = db.list_external_ids(u2, work_id_user_two).await.unwrap();
            assert_eq!(own_rows_after.len(), 1);
            assert_eq!(own_rows_after[0].user_id, u2);
            assert_eq!(own_rows_after[0].work_id, work_id_user_two);
            assert_eq!(own_rows_after[0].id_type, ExternalIdType::Asin);
            assert_eq!(own_rows_after[0].id_value, "B00USER2ID");
        }

        #[tokio::test]
        async fn test_external_id_db_upsert_external_id_accepts_all_external_id_types_type() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::upsert_external_id | Behavior: ExternalIdType::Isbn13, ExternalIdType::Isbn10, and ExternalIdType::Asin are all accepted
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work_id = make_work_id(db, u1, "All Types Work").await;

            db.upsert_external_id(
                u1,
                make_external_id_req(work_id, ExternalIdType::Isbn13, "9785555555555"),
            )
            .await
            .unwrap();
            db.upsert_external_id(
                u1,
                make_external_id_req(work_id, ExternalIdType::Isbn10, "5555555555"),
            )
            .await
            .unwrap();
            db.upsert_external_id(
                u1,
                make_external_id_req(work_id, ExternalIdType::Asin, "B00ALLTYP1"),
            )
            .await
            .unwrap();

            let rows = db.list_external_ids(u1, work_id).await.unwrap();

            assert_eq!(rows.len(), 3);
            assert!(has_external_id(&rows, ExternalIdType::Isbn13, "9785555555555"));
            assert!(has_external_id(&rows, ExternalIdType::Isbn10, "5555555555"));
            assert!(has_external_id(&rows, ExternalIdType::Asin, "B00ALLTYP1"));
        }

        #[tokio::test]
        async fn test_external_id_db_upsert_external_id_stores_same_work_different_id_types_together_boundary() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::upsert_external_id | Behavior: the same work may store multiple rows with the same id_value when id_type differs
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work_id = make_work_id(db, u1, "Coexisting Types Work").await;
            let shared_value = "1234567890";

            db.upsert_external_id(
                u1,
                make_external_id_req(work_id, ExternalIdType::Isbn10, shared_value),
            )
            .await
            .unwrap();
            db.upsert_external_id(
                u1,
                make_external_id_req(work_id, ExternalIdType::Asin, shared_value),
            )
            .await
            .unwrap();

            let rows = db.list_external_ids(u1, work_id).await.unwrap();

            assert_eq!(rows.len(), 2);
            assert!(has_external_id(&rows, ExternalIdType::Isbn10, shared_value));
            assert!(has_external_id(&rows, ExternalIdType::Asin, shared_value));
        }

        #[tokio::test]
        async fn test_external_id_db_upsert_external_id_nonexistent_work_returns_error_fk_boundary() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::upsert_external_id | Behavior: upserting an external ID for a non-existent work_id returns an error rather than creating an orphan row
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let nonexistent_work_id: WorkId = i64::MAX;
            let req = make_external_id_req(
                nonexistent_work_id,
                ExternalIdType::Isbn13,
                "9784044044044",
            );

            let result = db.upsert_external_id(u1, req).await;

            assert!(result.is_err());
        }

        #[tokio::test]
        async fn test_external_id_db_upsert_external_id_other_users_work_does_not_write_isolation() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::upsert_external_id | Behavior: cross-user upsert may return Err or Ok(()), but must not alter the owner's data or create visible rows for the caller
            let h = setup_harness().await;
            let db = h.db();
            let (user_a_id, user_b_id) = h.user_ids();

            let work_a_id = make_work_id(db, user_a_id, "Owned By User A").await;

            db.upsert_external_id(
                user_a_id,
                make_external_id_req(work_a_id, ExternalIdType::Asin, "B00OWNER01"),
            )
            .await
            .unwrap();

            let pre_seeded_ids = db.list_external_ids(user_a_id, work_a_id).await.unwrap();
            assert_eq!(pre_seeded_ids.len(), 1);

            let result = db
                .upsert_external_id(
                    user_b_id,
                    make_external_id_req(work_a_id, ExternalIdType::Asin, "B00XUSER01"),
                )
                .await;

            let _ = result;

            let user_a_rows = db.list_external_ids(user_a_id, work_a_id).await.unwrap();
            assert_eq!(
                external_id_set(&user_a_rows),
                external_id_set(&pre_seeded_ids),
                "User A data must be unchanged"
            );

            let user_b_rows = db.list_external_ids(user_b_id, work_a_id).await;
            match user_b_rows {
                Ok(rows) => assert!(rows.is_empty()),
                Err(_) => {}
            }
        }

        #[tokio::test]
        async fn test_external_id_db_upsert_external_ids_batch_duplicate_entries_same_batch_succeeds_and_stores_one_row_invariant() {
            // REQ-ID: R-06 | Contract: ExternalIdDb::upsert_external_ids_batch | Behavior: duplicate entries within the same batch are accepted as no-ops and result in exactly one stored row
            let h = setup_harness().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work_id = make_work_id(db, u1, "Batch Duplicate Entries Work").await;
            let reqs = vec![
                make_external_id_req(work_id, ExternalIdType::Isbn13, "9787777777777"),
                make_external_id_req(work_id, ExternalIdType::Isbn13, "9787777777777"),
            ];

            let result = db.upsert_external_ids_batch(u1, reqs).await;

            assert!(result.is_ok());

            let rows = db.list_external_ids(u1, work_id).await.unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].user_id, u1);
            assert_eq!(rows[0].work_id, work_id);
            assert_eq!(rows[0].id_type, ExternalIdType::Isbn13);
            assert_eq!(rows[0].id_value, "9787777777777");
        }
    };
}

external_id_db_tests!();
