//! Behavioral contract tests for ProvenanceDb against a real SQLite :memory: database with full migrations, covering field provenance CRUD, batch behavior, upsert semantics, user isolation, and cascade invariants for R-18.

use assert_matches::assert_matches;

use async_trait::async_trait;
use livrarr_db::*;

#[async_trait]
pub trait DbTestHarness: Send + Sync {
    type Db: ProvenanceDb + WorkDb + UserDb;

    async fn setup() -> Self;
    fn db(&self) -> &Self::Db;
    fn user_ids(&self) -> (UserId, UserId);
    fn work_ids(&self) -> (WorkId, WorkId);
}

fn make_user_req(username: &str, role: UserRole, suffix: &str) -> CreateUserDbRequest {
    CreateUserDbRequest {
        username: username.to_string(),
        password_hash: format!("password-hash-{suffix}"),
        role,
        api_key_hash: format!("api-key-hash-{suffix}"),
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

fn make_provider_req(
    user_id: UserId,
    work_id: WorkId,
    field: WorkField,
    source: MetadataProvider,
) -> SetFieldProvenanceRequest {
    SetFieldProvenanceRequest {
        user_id,
        work_id,
        field,
        source: Some(source),
        setter: ProvenanceSetter::Provider,
        cleared: false,
    }
}

fn make_user_cleared_req(
    user_id: UserId,
    work_id: WorkId,
    field: WorkField,
) -> SetFieldProvenanceRequest {
    SetFieldProvenanceRequest {
        user_id,
        work_id,
        field,
        source: None,
        setter: ProvenanceSetter::User,
        cleared: true,
    }
}

fn make_user_set_req(
    user_id: UserId,
    work_id: WorkId,
    field: WorkField,
) -> SetFieldProvenanceRequest {
    SetFieldProvenanceRequest {
        user_id,
        work_id,
        field,
        source: None,
        setter: ProvenanceSetter::User,
        cleared: false,
    }
}

fn make_system_req(
    user_id: UserId,
    work_id: WorkId,
    field: WorkField,
) -> SetFieldProvenanceRequest {
    SetFieldProvenanceRequest {
        user_id,
        work_id,
        field,
        source: None,
        setter: ProvenanceSetter::System,
        cleared: false,
    }
}

fn find_field(rows: &[FieldProvenance], field: WorkField) -> Option<&FieldProvenance> {
    rows.iter().find(|row| row.field == field)
}

fn require_field(rows: &[FieldProvenance], field: WorkField) -> &FieldProvenance {
    find_field(rows, field).unwrap_or_else(|| panic!("expected provenance for field {field:?}"))
}

macro_rules! provenance_db_tests {
    ($harness:ty) => {
        #[tokio::test]
        async fn test_provenance_db_set_get_returns_written_record() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::set_field_provenance + ProvenanceDb::get_field_provenance | Behavior: writing a provenance record makes it retrievable for the same user, work, and field
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance(make_provider_req(
                u1,
                w1,
                WorkField::Title,
                MetadataProvider::OpenLibrary,
            ))
            .await
            .unwrap();

            let got = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap()
                .expect("provenance should exist after successful write");

            assert_eq!(got.user_id, u1);
            assert_eq!(got.work_id, w1);
            assert_eq!(got.field, WorkField::Title);
            assert!(matches!(got.source, Some(MetadataProvider::OpenLibrary)));
            assert_eq!(got.setter, ProvenanceSetter::Provider);
            assert!(!got.cleared);
        }

        #[tokio::test]
        async fn test_provenance_db_get_returns_none_for_nonexistent_field() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::get_field_provenance | Behavior: querying a field with no provenance returns None
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            let got = db
                .get_field_provenance(u1, w1, WorkField::Description)
                .await
                .unwrap();

            assert!(got.is_none());
        }

        #[tokio::test]
        async fn test_provenance_db_list_returns_empty_for_work_without_provenance() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::list_work_provenance | Behavior: listing provenance for a work with no records returns an empty vec
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            let rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert!(rows.is_empty());
        }

        #[tokio::test]
        async fn test_provenance_db_list_returns_all_records_for_work() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::list_work_provenance | Behavior: listing provenance returns every stored field record for the specified work
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance(make_provider_req(
                u1,
                w1,
                WorkField::Title,
                MetadataProvider::OpenLibrary,
            ))
            .await
            .unwrap();
            db.set_field_provenance(make_provider_req(
                u1,
                w1,
                WorkField::Description,
                MetadataProvider::Goodreads,
            ))
            .await
            .unwrap();

            let rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert_eq!(rows.len(), 2);
            let title = require_field(&rows, WorkField::Title);
            let description = require_field(&rows, WorkField::Description);

            assert_eq!(title.user_id, u1);
            assert_eq!(title.work_id, w1);
            assert!(matches!(title.source, Some(MetadataProvider::OpenLibrary)));
            assert_eq!(title.setter, ProvenanceSetter::Provider);
            assert!(!title.cleared);

            assert_eq!(description.user_id, u1);
            assert_eq!(description.work_id, w1);
            assert!(matches!(
                description.source,
                Some(MetadataProvider::Goodreads)
            ));
            assert_eq!(description.setter, ProvenanceSetter::Provider);
            assert!(!description.cleared);
        }

        #[tokio::test]
        async fn test_provenance_db_set_batch_writes_all_requested_fields() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::set_field_provenance_batch | Behavior: a successful batch write persists provenance for each requested field
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance_batch(vec![
                make_provider_req(u1, w1, WorkField::Title, MetadataProvider::OpenLibrary),
                make_provider_req(u1, w1, WorkField::CoverUrl, MetadataProvider::Goodreads),
                make_user_cleared_req(u1, w1, WorkField::Subtitle),
            ])
            .await
            .unwrap();

            let rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert_eq!(rows.len(), 3);
            assert!(find_field(&rows, WorkField::Title).is_some());
            assert!(find_field(&rows, WorkField::CoverUrl).is_some());
            assert!(find_field(&rows, WorkField::Subtitle).is_some());
        }

        #[tokio::test]
        async fn test_provenance_db_set_batch_is_atomic_when_any_request_fails() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::set_field_provenance_batch | Behavior: batch writes are atomic and do not partially persist when one request fails
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, w2) = h.work_ids();
            let missing_work_id = std::cmp::max(w1, w2) + 1_000_000;

            let result = db
                .set_field_provenance_batch(vec![
                    make_provider_req(u1, w1, WorkField::Title, MetadataProvider::OpenLibrary),
                    make_provider_req(
                        u1,
                        missing_work_id,
                        WorkField::Description,
                        MetadataProvider::Goodreads,
                    ),
                ])
                .await;

            assert_matches!(result, Err(_));

            let title = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap();
            let all_rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert!(title.is_none());
            assert!(all_rows.is_empty());
        }

        #[tokio::test]
        async fn test_provenance_db_set_batch_rejects_entire_batch_on_invariant_violation() {
            // REQ-ID: R-18 | Contract: SetFieldProvenanceRequest invariant + ProvenanceDb::set_field_provenance_batch | Behavior: if any batch item violates provenance invariants, the entire batch is rejected atomically and no rows are written
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            let result = db
                .set_field_provenance_batch(vec![
                    make_provider_req(u1, w1, WorkField::Title, MetadataProvider::OpenLibrary),
                    SetFieldProvenanceRequest {
                        user_id: u1,
                        work_id: w1,
                        field: WorkField::Description,
                        source: None,
                        setter: ProvenanceSetter::Provider,
                        cleared: false,
                    },
                ])
                .await;

            assert_matches!(result, Err(_));

            let title = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap();
            let description = db
                .get_field_provenance(u1, w1, WorkField::Description)
                .await
                .unwrap();
            let rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert!(title.is_none());
            assert!(description.is_none());
            assert!(rows.is_empty());
        }

        #[tokio::test]
        async fn test_provenance_db_set_batch_empty_is_ok_and_preserves_db_state() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::set_field_provenance_batch | Behavior: an empty batch succeeds and leaves existing provenance unchanged
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance(make_provider_req(
                u1,
                w1,
                WorkField::Title,
                MetadataProvider::OpenLibrary,
            ))
            .await
            .unwrap();

            let before = db.list_work_provenance(u1, w1).await.unwrap();

            db.set_field_provenance_batch(vec![]).await.unwrap();

            let after = db.list_work_provenance(u1, w1).await.unwrap();

            assert_eq!(before.len(), 1);
            assert_eq!(after.len(), 1);
            assert_eq!(after[0].field, WorkField::Title);
            assert!(matches!(
                after[0].source,
                Some(MetadataProvider::OpenLibrary)
            ));
            assert_eq!(after[0].setter, ProvenanceSetter::Provider);
            assert!(!after[0].cleared);
        }

        #[tokio::test]
        async fn test_provenance_db_delete_batch_removes_only_requested_fields() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::delete_field_provenance_batch | Behavior: deleting specific fields removes only those provenance records and leaves others intact
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance_batch(vec![
                make_provider_req(u1, w1, WorkField::Title, MetadataProvider::OpenLibrary),
                make_provider_req(u1, w1, WorkField::Description, MetadataProvider::Goodreads),
                make_provider_req(u1, w1, WorkField::CoverUrl, MetadataProvider::Hardcover),
            ])
            .await
            .unwrap();

            db.delete_field_provenance_batch(
                u1,
                w1,
                vec![WorkField::Title, WorkField::Description],
            )
            .await
            .unwrap();

            let title = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap();
            let description = db
                .get_field_provenance(u1, w1, WorkField::Description)
                .await
                .unwrap();
            let cover = db
                .get_field_provenance(u1, w1, WorkField::CoverUrl)
                .await
                .unwrap();
            let rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert!(title.is_none());
            assert!(description.is_none());
            assert!(cover.is_some());
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].field, WorkField::CoverUrl);
            assert!(matches!(rows[0].source, Some(MetadataProvider::Hardcover)));
        }

        #[tokio::test]
        async fn test_provenance_db_delete_batch_empty_is_ok_and_preserves_db_state() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::delete_field_provenance_batch | Behavior: deleting an empty field batch succeeds and leaves existing provenance unchanged
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance_batch(vec![
                make_provider_req(u1, w1, WorkField::Title, MetadataProvider::OpenLibrary),
                make_provider_req(u1, w1, WorkField::Description, MetadataProvider::Goodreads),
            ])
            .await
            .unwrap();

            let before = db.list_work_provenance(u1, w1).await.unwrap();

            db.delete_field_provenance_batch(u1, w1, vec![])
                .await
                .unwrap();

            let after = db.list_work_provenance(u1, w1).await.unwrap();

            assert_eq!(before.len(), 2);
            assert_eq!(after.len(), 2);
            assert!(find_field(&after, WorkField::Title).is_some());
            assert!(find_field(&after, WorkField::Description).is_some());
        }

        #[tokio::test]
        async fn test_provenance_db_delete_batch_nonexistent_field_is_idempotent() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::delete_field_provenance_batch | Behavior: deleting a field with no provenance succeeds and does not alter other records
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance(make_provider_req(
                u1,
                w1,
                WorkField::Title,
                MetadataProvider::OpenLibrary,
            ))
            .await
            .unwrap();

            db.delete_field_provenance_batch(u1, w1, vec![WorkField::Description])
                .await
                .unwrap();

            let title = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap();
            let description = db
                .get_field_provenance(u1, w1, WorkField::Description)
                .await
                .unwrap();
            let rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert!(title.is_some());
            assert!(description.is_none());
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].field, WorkField::Title);
        }

        #[tokio::test]
        async fn test_provenance_db_clear_work_removes_all_records() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::clear_work_provenance | Behavior: clearing work provenance removes every provenance record for the work
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance_batch(vec![
                make_provider_req(u1, w1, WorkField::Title, MetadataProvider::OpenLibrary),
                make_provider_req(u1, w1, WorkField::Description, MetadataProvider::Goodreads),
            ])
            .await
            .unwrap();

            db.clear_work_provenance(u1, w1).await.unwrap();

            let title = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap();
            let description = db
                .get_field_provenance(u1, w1, WorkField::Description)
                .await
                .unwrap();
            let rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert!(title.is_none());
            assert!(description.is_none());
            assert!(rows.is_empty());
        }

        #[tokio::test]
        async fn test_provenance_db_set_upserts_existing_work_field_record() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::set_field_provenance | Behavior: setting provenance twice for the same work and field updates the existing record instead of creating a duplicate
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance(make_provider_req(
                u1,
                w1,
                WorkField::Title,
                MetadataProvider::OpenLibrary,
            ))
            .await
            .unwrap();
            db.set_field_provenance(make_provider_req(
                u1,
                w1,
                WorkField::Title,
                MetadataProvider::Goodreads,
            ))
            .await
            .unwrap();

            let got = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap()
                .expect("upserted provenance should exist");
            let rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert!(matches!(got.source, Some(MetadataProvider::Goodreads)));
            assert_eq!(got.setter, ProvenanceSetter::Provider);
            assert_eq!(
                rows.iter()
                    .filter(|row| row.field == WorkField::Title)
                    .count(),
                1
            );
            assert_eq!(rows.len(), 1);
        }

        #[tokio::test]
        async fn test_provenance_db_set_accepts_cleared_true_for_user_setter() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::set_field_provenance | Behavior: cleared=true is valid when the setter is User and the stored record preserves that user-cleared state
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance(make_user_cleared_req(u1, w1, WorkField::Subtitle))
                .await
                .unwrap();

            let got = db
                .get_field_provenance(u1, w1, WorkField::Subtitle)
                .await
                .unwrap()
                .expect("user-cleared provenance should exist");

            assert_eq!(got.user_id, u1);
            assert_eq!(got.work_id, w1);
            assert_eq!(got.field, WorkField::Subtitle);
            assert_eq!(got.setter, ProvenanceSetter::User);
            assert!(got.cleared);
            assert!(got.source.is_none());
        }

        #[tokio::test]
        async fn test_provenance_db_set_accepts_user_setter_with_source_none_and_cleared_false() {
            // REQ-ID: R-18 | Contract: SetFieldProvenanceRequest invariant + ProvenanceDb::set_field_provenance | Behavior: source=None with setter=User and cleared=false is valid and persists a user-owned non-cleared provenance row
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            let result = db
                .set_field_provenance(make_user_set_req(u1, w1, WorkField::OriginalTitle))
                .await;

            assert!(result.is_ok());

            let got = db
                .get_field_provenance(u1, w1, WorkField::OriginalTitle)
                .await
                .unwrap()
                .expect("user-set provenance should exist");
            let rows = db.list_work_provenance(u1, w1).await.unwrap();
            let row = require_field(&rows, WorkField::OriginalTitle);

            assert_eq!(got.user_id, u1);
            assert_eq!(got.work_id, w1);
            assert_eq!(got.field, WorkField::OriginalTitle);
            assert_eq!(got.setter, ProvenanceSetter::User);
            assert!(got.source.is_none());
            assert!(!got.cleared);

            assert_eq!(row.user_id, u1);
            assert_eq!(row.work_id, w1);
            assert_eq!(row.field, WorkField::OriginalTitle);
            assert_eq!(row.setter, ProvenanceSetter::User);
            assert!(row.source.is_none());
            assert!(!row.cleared);
        }

        #[tokio::test]
        async fn test_provenance_db_set_accepts_system_setter_with_no_source() {
            // REQ-ID: R-18 | Contract: FieldProvenance invariant + ProvenanceDb::set_field_provenance | Behavior: source=None is valid when setter=System and is stored as a system-owned provenance record
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance(make_system_req(u1, w1, WorkField::SortTitle))
                .await
                .unwrap();

            let got = db
                .get_field_provenance(u1, w1, WorkField::SortTitle)
                .await
                .unwrap()
                .expect("system-owned provenance should exist");

            assert_eq!(got.user_id, u1);
            assert_eq!(got.work_id, w1);
            assert_eq!(got.field, WorkField::SortTitle);
            assert!(got.source.is_none());
            assert_eq!(got.setter, ProvenanceSetter::System);
            assert!(!got.cleared);
        }

        #[tokio::test]
        async fn test_provenance_db_set_rejects_system_setter_with_source_some() {
            // REQ-ID: R-18 | Contract: SetFieldProvenanceRequest invariant + ProvenanceDb::set_field_provenance | Behavior: source=None is required when setter=System, so source=Some(_) is rejected and no record is written
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            let result = db
                .set_field_provenance(SetFieldProvenanceRequest {
                    user_id: u1,
                    work_id: w1,
                    field: WorkField::SortTitle,
                    source: Some(MetadataProvider::OpenLibrary),
                    setter: ProvenanceSetter::System,
                    cleared: false,
                })
                .await;

            assert_matches!(result, Err(_));

            let got = db
                .get_field_provenance(u1, w1, WorkField::SortTitle)
                .await
                .unwrap();
            let rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert!(got.is_none());
            assert!(rows.is_empty());
        }

        #[tokio::test]
        async fn test_provenance_db_set_rejects_provider_setter_with_no_source() {
            // REQ-ID: R-18 | Contract: SetFieldProvenanceRequest invariant + ProvenanceDb::set_field_provenance | Behavior: source=Some(_) is required when setter=Provider, so source=None is rejected and no record is written
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            let result = db
                .set_field_provenance(SetFieldProvenanceRequest {
                    user_id: u1,
                    work_id: w1,
                    field: WorkField::Title,
                    source: None,
                    setter: ProvenanceSetter::Provider,
                    cleared: false,
                })
                .await;

            assert_matches!(result, Err(_));

            let got = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap();
            let rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert!(got.is_none());
            assert!(rows.is_empty());
        }

        #[tokio::test]
        async fn test_provenance_db_set_rejects_user_setter_with_source_some() {
            // REQ-ID: R-18 | Contract: SetFieldProvenanceRequest invariant + ProvenanceDb::set_field_provenance | Behavior: source=None is required when setter=User, so source=Some(_) is rejected and no record is written
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            let result = db
                .set_field_provenance(SetFieldProvenanceRequest {
                    user_id: u1,
                    work_id: w1,
                    field: WorkField::Subtitle,
                    source: Some(MetadataProvider::Hardcover),
                    setter: ProvenanceSetter::User,
                    cleared: false,
                })
                .await;

            assert_matches!(result, Err(_));

            let got = db
                .get_field_provenance(u1, w1, WorkField::Subtitle)
                .await
                .unwrap();
            let rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert!(got.is_none());
            assert!(rows.is_empty());
        }

        #[tokio::test]
        async fn test_provenance_db_set_rejects_cleared_true_for_provider_setter() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::set_field_provenance | Behavior: cleared=true is rejected when the setter is not User and no record is written
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            let result = db
                .set_field_provenance(SetFieldProvenanceRequest {
                    user_id: u1,
                    work_id: w1,
                    field: WorkField::Subtitle,
                    source: Some(MetadataProvider::OpenLibrary),
                    setter: ProvenanceSetter::Provider,
                    cleared: true,
                })
                .await;

            assert_matches!(result, Err(_));

            let got = db
                .get_field_provenance(u1, w1, WorkField::Subtitle)
                .await
                .unwrap();
            let rows = db.list_work_provenance(u1, w1).await.unwrap();

            assert!(got.is_none());
            assert!(rows.is_empty());
        }

        #[tokio::test]
        async fn test_provenance_db_queries_are_isolated_by_user_id() {
            // REQ-ID: R-18 | Contract: ProvenanceDb::set_field_provenance + ProvenanceDb::get_field_provenance + ProvenanceDb::list_work_provenance | Behavior: provenance records remain isolated per user and are not visible through another user's queries
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, u2) = h.user_ids();
            let (w1, w2) = h.work_ids();

            db.set_field_provenance(make_provider_req(
                u1,
                w1,
                WorkField::Title,
                MetadataProvider::OpenLibrary,
            ))
            .await
            .unwrap();
            db.set_field_provenance(make_provider_req(
                u2,
                w2,
                WorkField::Title,
                MetadataProvider::Goodreads,
            ))
            .await
            .unwrap();

            let u1_own = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap()
                .expect("u1 should see its own provenance");
            let u2_own = db
                .get_field_provenance(u2, w2, WorkField::Title)
                .await
                .unwrap()
                .expect("u2 should see its own provenance");
            let u1_cross = db
                .get_field_provenance(u1, w2, WorkField::Title)
                .await
                .unwrap();
            let u2_cross = db
                .get_field_provenance(u2, w1, WorkField::Title)
                .await
                .unwrap();
            let u1_rows = db.list_work_provenance(u1, w1).await.unwrap();
            let u2_rows = db.list_work_provenance(u2, w2).await.unwrap();

            assert_eq!(u1_own.user_id, u1);
            assert!(matches!(u1_own.source, Some(MetadataProvider::OpenLibrary)));
            assert_eq!(u2_own.user_id, u2);
            assert!(matches!(u2_own.source, Some(MetadataProvider::Goodreads)));
            assert!(u1_cross.is_none());
            assert!(u2_cross.is_none());
            assert_eq!(u1_rows.len(), 1);
            assert_eq!(u1_rows[0].user_id, u1);
            assert_eq!(u2_rows.len(), 1);
            assert_eq!(u2_rows[0].user_id, u2);
        }

        #[tokio::test]
        async fn test_provenance_db_cascades_on_work_delete() {
            // REQ-ID: R-18 | Contract: work_metadata_provenance FK ON DELETE CASCADE from works(id) + ProvenanceDb::get_field_provenance + ProvenanceDb::list_work_provenance | Behavior: deleting a work removes existing provenance rows and subsequent reads confirm they are gone
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance_batch(vec![
                make_provider_req(u1, w1, WorkField::Title, MetadataProvider::OpenLibrary),
                make_user_cleared_req(u1, w1, WorkField::Subtitle),
            ])
            .await
            .unwrap();

            let before_rows = db.list_work_provenance(u1, w1).await.unwrap();
            let before_title = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap();
            let before_subtitle = db
                .get_field_provenance(u1, w1, WorkField::Subtitle)
                .await
                .unwrap();

            assert_eq!(before_rows.len(), 2);
            assert!(before_title.is_some());
            assert!(before_subtitle.is_some());

            db.delete_work(u1, w1).await.unwrap();

            let after_rows = db.list_work_provenance(u1, w1).await.unwrap();
            let after_title = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap();
            let after_subtitle = db
                .get_field_provenance(u1, w1, WorkField::Subtitle)
                .await
                .unwrap();

            assert!(after_rows.is_empty());
            assert!(after_title.is_none());
            assert!(after_subtitle.is_none());
        }

        #[tokio::test]
        async fn test_provenance_db_cascades_on_user_delete() {
            // REQ-ID: R-18 | Contract: work_metadata_provenance FK ON DELETE CASCADE from users(id) + ProvenanceDb::get_field_provenance + ProvenanceDb::list_work_provenance | Behavior: deleting a user removes existing provenance rows and subsequent reads confirm they are gone
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();
            let (w1, _) = h.work_ids();

            db.set_field_provenance_batch(vec![
                make_provider_req(u1, w1, WorkField::Title, MetadataProvider::OpenLibrary),
                make_user_cleared_req(u1, w1, WorkField::Subtitle),
            ])
            .await
            .unwrap();

            let before_rows = db.list_work_provenance(u1, w1).await.unwrap();
            let before_title = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap();
            let before_subtitle = db
                .get_field_provenance(u1, w1, WorkField::Subtitle)
                .await
                .unwrap();

            assert_eq!(before_rows.len(), 2);
            assert!(before_title.is_some());
            assert!(before_subtitle.is_some());

            db.delete_user(u1).await.unwrap();

            let after_rows = db.list_work_provenance(u1, w1).await.unwrap();
            let after_title = db
                .get_field_provenance(u1, w1, WorkField::Title)
                .await
                .unwrap();
            let after_subtitle = db
                .get_field_provenance(u1, w1, WorkField::Subtitle)
                .await
                .unwrap();

            assert!(after_rows.is_empty());
            assert!(after_title.is_none());
            assert!(after_subtitle.is_none());
        }
    };
}

struct RealSqliteHarness {
    db: livrarr_db::sqlite::SqliteDb,
    u1: UserId,
    u2: UserId,
    w1: WorkId,
    w2: WorkId,
}

#[async_trait]
impl DbTestHarness for RealSqliteHarness {
    type Db = livrarr_db::sqlite::SqliteDb;

    async fn setup() -> Self {
        let db = livrarr_db::create_test_db().await;

        let u1 = db
            .create_user(make_user_req("prov_user_1", UserRole::Admin, "1"))
            .await
            .unwrap();
        let u2 = db
            .create_user(make_user_req("prov_user_2", UserRole::User, "2"))
            .await
            .unwrap();

        let w1 = db
            .create_work(make_work_req(u1.id, "Provenance Work One", "Author One"))
            .await
            .unwrap()
            .0
            .id;
        let w2 = db
            .create_work(make_work_req(u2.id, "Provenance Work Two", "Author Two"))
            .await
            .unwrap()
            .0
            .id;

        Self {
            db,
            u1: u1.id,
            u2: u2.id,
            w1,
            w2,
        }
    }

    fn db(&self) -> &Self::Db {
        &self.db
    }

    fn user_ids(&self) -> (UserId, UserId) {
        (self.u1, self.u2)
    }

    fn work_ids(&self) -> (WorkId, WorkId) {
        (self.w1, self.w2)
    }
}

provenance_db_tests!(RealSqliteHarness);
