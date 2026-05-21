#![allow(dead_code)]
//! Behavioral tests for WorkDb merge extensions against real SQLite `:memory:`.
//! Covers apply_enrichment_merge nominal/CAS/conflict semantics, merge_generation,
//! reset_for_manual_refresh, list_conflict_works, and provenance deletes.

mod common;

use chrono::{Duration, Utc};
use common::create_test_db;
use livrarr_db::*;
use livrarr_domain::{
    ApplyMergeOutcome, EnrichmentStatus, ExternalIdType, MergeResolved, MetadataProvider,
    ProvenanceSetter, UserId, UserRole, Work, WorkField, WorkId,
};

const ORIGINAL_TITLE: &str = "Original Title";
const ORIGINAL_AUTHOR: &str = "Original Author";
const ORIGINAL_COVER_URL: &str = "https://example.test/original.jpg";

const MERGED_TITLE: &str = "Merged Title";
const MERGED_DESCRIPTION: &str = "Merged description";
const MERGED_GR_KEY: &str = "show/123";
const MERGED_COVER_URL: &str = "https://example.test/merged.jpg";

const STALE_TITLE: &str = "Stale Title";
const STALE_DESCRIPTION: &str = "Stale description";
const STALE_GR_KEY: &str = "show/999";
const STALE_COVER_URL: &str = "https://example.test/stale.jpg";

const SAMPLE_ISBN10: &str = "0123456789";
const SAMPLE_ISBN13: &str = "9781234567890";
const SAMPLE_ASIN: &str = "B00TEST123";

fn make_work_req(user_id: UserId, title: &str, author_name: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author_name.to_string(),
        normalized_title: livrarr_domain::normalize_for_matching(title),
        normalized_author: livrarr_domain::normalize_for_matching(author_name),
        author_id: None,
        ol_key: None,
        year: Some(2024),
        cover_url: Some(ORIGINAL_COVER_URL.to_string()),
        ..Default::default()
    }
}

fn resolved_enrichment_update(
    title: &str,
    author_name: &str,
    description: Option<&str>,
    gr_key: Option<&str>,
    cover_url: Option<&str>,
) -> MergeResolved<UpdateWorkEnrichmentDbRequest> {
    MergeResolved::new(UpdateWorkEnrichmentDbRequest {
        title: Some(title.to_string()),
        subtitle: None,
        original_title: None,
        author_name: Some(author_name.to_string()),
        description: description.map(str::to_string),
        year: None,
        series_name: None,
        series_position: None,
        genres: None,
        language: None,
        page_count: None,
        duration_seconds: None,
        publisher: None,
        publish_date: None,
        hc_key: None,
        gr_key: gr_key.map(str::to_string),
        ol_key: None,
        isbn_13: None,
        asin: None,
        narrator: None,
        narration_type: None,
        abridged: None,
        rating: None,
        rating_count: None,
        cover_url: cover_url.map(str::to_string),
        ..Default::default()
    })
}

fn sample_merge_request(
    user_id: UserId,
    work_id: WorkId,
    expected_merge_generation: i64,
) -> ApplyEnrichmentMergeRequest {
    ApplyEnrichmentMergeRequest {
        user_id,
        work_id,
        expected_merge_generation,
        work_update: Some(resolved_enrichment_update(
            MERGED_TITLE,
            ORIGINAL_AUTHOR,
            Some(MERGED_DESCRIPTION),
            Some(MERGED_GR_KEY),
            Some(MERGED_COVER_URL),
        )),
        new_enrichment_status: EnrichmentStatus::Enriched,
        provenance_upserts: vec![],
        provenance_deletes: vec![],
        external_id_updates: vec![],
    }
}

fn stale_merge_request(
    user_id: UserId,
    work_id: WorkId,
    expected_merge_generation: i64,
) -> ApplyEnrichmentMergeRequest {
    ApplyEnrichmentMergeRequest {
        user_id,
        work_id,
        expected_merge_generation,
        work_update: Some(resolved_enrichment_update(
            STALE_TITLE,
            ORIGINAL_AUTHOR,
            Some(STALE_DESCRIPTION),
            Some(STALE_GR_KEY),
            Some(STALE_COVER_URL),
        )),
        new_enrichment_status: EnrichmentStatus::Enriched,
        provenance_upserts: vec![],
        provenance_deletes: vec![],
        external_id_updates: vec![],
    }
}

fn conflict_merge_request(
    user_id: UserId,
    work_id: WorkId,
    expected_merge_generation: i64,
) -> ApplyEnrichmentMergeRequest {
    ApplyEnrichmentMergeRequest {
        user_id,
        work_id,
        expected_merge_generation,
        work_update: None,
        new_enrichment_status: EnrichmentStatus::Conflict,
        provenance_upserts: vec![],
        provenance_deletes: vec![],
        external_id_updates: vec![],
    }
}

fn provider_provenance(
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

fn user_provenance(
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

fn sample_provenance_upserts(user_id: UserId, work_id: WorkId) -> Vec<SetFieldProvenanceRequest> {
    vec![
        provider_provenance(
            user_id,
            work_id,
            WorkField::Title,
            MetadataProvider::Goodreads,
        ),
        provider_provenance(
            user_id,
            work_id,
            WorkField::GrKey,
            MetadataProvider::Goodreads,
        ),
    ]
}

fn sample_external_ids(work_id: WorkId) -> Vec<UpsertExternalIdRequest> {
    vec![
        UpsertExternalIdRequest {
            work_id,
            id_type: ExternalIdType::Isbn13,
            id_value: SAMPLE_ISBN13.to_string(),
        },
        UpsertExternalIdRequest {
            work_id,
            id_type: ExternalIdType::Asin,
            id_value: SAMPLE_ASIN.to_string(),
        },
    ]
}

fn sample_isbn10_external_ids(work_id: WorkId) -> Vec<UpsertExternalIdRequest> {
    vec![UpsertExternalIdRequest {
        work_id,
        id_type: ExternalIdType::Isbn10,
        id_value: SAMPLE_ISBN10.to_string(),
    }]
}

fn external_id_tuples(ids: &[ExternalId]) -> Vec<(ExternalIdType, String)> {
    let mut tuples: Vec<(ExternalIdType, String)> = ids
        .iter()
        .map(|id| (id.id_type, id.id_value.clone()))
        .collect();
    tuples.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then_with(|| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)))
    });
    tuples
}

async fn seed_users<DB: UserDb>(db: &DB) -> (UserId, UserId) {
    let u1 = db
        .create_user(CreateUserDbRequest {
            username: "merge_user_1".to_string(),
            password_hash: "hash1".to_string(),
            role: UserRole::Admin,
            api_key_hash: "api1".to_string(),
        })
        .await
        .unwrap();

    let u2 = db
        .create_user(CreateUserDbRequest {
            username: "merge_user_2".to_string(),
            password_hash: "hash2".to_string(),
            role: UserRole::User,
            api_key_hash: "api2".to_string(),
        })
        .await
        .unwrap();

    (u1.id, u2.id)
}

async fn create_new_work<DB: WorkDb + WorkDbCreate>(db: &DB, user_id: UserId) -> Work {
    db.create_work(make_work_req(user_id, ORIGINAL_TITLE, ORIGINAL_AUTHOR))
        .await
        .unwrap()
        .0
}

async fn create_merged_work<DB: WorkDb + WorkDbCreate>(db: &DB, user_id: UserId) -> Work {
    let work = create_new_work(db, user_id).await;
    let outcome = db
        .apply_enrichment_merge(sample_merge_request(user_id, work.id, 0))
        .await
        .unwrap();
    assert_eq!(outcome, ApplyMergeOutcome::Applied);
    db.get_work(user_id, work.id).await.unwrap()
}

async fn create_conflict_work<DB: WorkDb + WorkDbCreate>(db: &DB, user_id: UserId) -> Work {
    let work = create_new_work(db, user_id).await;
    let outcome = db
        .apply_enrichment_merge(conflict_merge_request(user_id, work.id, 0))
        .await
        .unwrap();
    assert_eq!(outcome, ApplyMergeOutcome::Applied);
    db.get_work(user_id, work.id).await.unwrap()
}

async fn seed_retry_rows<DB: ProviderRetryStateDb>(db: &DB, user_id: UserId, work_id: WorkId) {
    db.record_will_retry(
        user_id,
        work_id,
        MetadataProvider::Goodreads,
        Utc::now() + Duration::hours(1),
    )
    .await
    .unwrap();

    db.record_suppressed(
        user_id,
        work_id,
        MetadataProvider::OpenLibrary,
        Utc::now() + Duration::hours(2),
    )
    .await
    .unwrap();
}

async fn setup_sqlite() -> (
    impl WorkDb + WorkDbCreate + UserDb + ProviderRetryStateDb + ProvenanceDb + ExternalIdDb,
    UserId,
    UserId,
) {
    let db = create_test_db().await;
    let (u1, u2) = seed_users(&db).await;
    (db, u1, u2)
}

macro_rules! work_db_merge_tests {
    ($setup:path) => {
        #[tokio::test]
        async fn test_work_db_merge_get_merge_generation_starts_at_zero_on_new_work() {
            // REQ-ID: R-22 | Contract: WorkDb::get_merge_generation | Behavior: returns 0 for a newly created work
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            let generation = db.get_merge_generation(u1, work.id).await.unwrap();

            assert_eq!(generation, 0);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_returns_applied_when_generation_matches() {
            // REQ-ID: R-02, R-22 | Contract: WorkDb::apply_enrichment_merge | Behavior: returns Applied when expected merge_generation matches the current generation
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            let outcome = db
                .apply_enrichment_merge(sample_merge_request(u1, work.id, 0))
                .await
                .unwrap();

            assert_eq!(outcome, ApplyMergeOutcome::Applied);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_writes_work_fields_when_work_update_present() {
            // REQ-ID: R-02, R-21 | Contract: WorkDb::apply_enrichment_merge | Behavior: writes work metadata fields when work_update is Some and CAS matches, including straight-assignment NULL overwrite semantics for None fields
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            db.apply_enrichment_merge(sample_merge_request(u1, work.id, 0))
                .await
                .unwrap();

            let got = db.get_work(u1, work.id).await.unwrap();

            assert_eq!(got.title, MERGED_TITLE);
            assert_eq!(got.description.as_deref(), Some(MERGED_DESCRIPTION));
            assert_eq!(got.gr_key.as_deref(), Some(MERGED_GR_KEY));
            assert_eq!(got.cover_url.as_deref(), Some(MERGED_COVER_URL));
            assert_eq!(got.year, None);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_updates_enrichment_status_on_apply() {
            // REQ-ID: R-02, R-22 | Contract: WorkDb::apply_enrichment_merge | Behavior: updates enrichment_status when the merge is applied
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            db.apply_enrichment_merge(sample_merge_request(u1, work.id, 0))
                .await
                .unwrap();

            let got = db.get_work(u1, work.id).await.unwrap();

            assert_eq!(got.enrichment_status, EnrichmentStatus::Enriched);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_increments_generation_by_one_on_apply() {
            // REQ-ID: R-22 | Contract: WorkDb::apply_enrichment_merge | Behavior: increments merge_generation by exactly 1 when the merge is applied
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            let before = db.get_merge_generation(u1, work.id).await.unwrap();
            db.apply_enrichment_merge(sample_merge_request(u1, work.id, before))
                .await
                .unwrap();
            let after = db.get_merge_generation(u1, work.id).await.unwrap();

            assert_eq!(before, 0);
            assert_eq!(after, before + 1);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_writes_provenance_upserts_on_apply() {
            // REQ-ID: R-02 | Contract: WorkDb::apply_enrichment_merge | Behavior: writes provenance_upserts when the merge is applied
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            let mut req = sample_merge_request(u1, work.id, 0);
            req.provenance_upserts = sample_provenance_upserts(u1, work.id);

            db.apply_enrichment_merge(req).await.unwrap();

            let title_provenance = db
                .get_field_provenance(u1, work.id, WorkField::Title)
                .await
                .unwrap()
                .unwrap();
            let gr_key_provenance = db
                .get_field_provenance(u1, work.id, WorkField::GrKey)
                .await
                .unwrap()
                .unwrap();

            assert_eq!(title_provenance.source, Some(MetadataProvider::Goodreads));
            assert_eq!(title_provenance.setter, ProvenanceSetter::Provider);
            assert_eq!(gr_key_provenance.source, Some(MetadataProvider::Goodreads));
            assert_eq!(gr_key_provenance.setter, ProvenanceSetter::Provider);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_writes_external_id_updates_on_apply() {
            // REQ-ID: R-21 | Contract: WorkDb::apply_enrichment_merge | Behavior: writes external_id_updates when the merge is applied
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            let mut req = sample_merge_request(u1, work.id, 0);
            req.external_id_updates = sample_external_ids(work.id);

            db.apply_enrichment_merge(req).await.unwrap();

            let ids = db.list_external_ids(u1, work.id).await.unwrap();

            assert!(ids
                .iter()
                .any(|id| id.id_type == ExternalIdType::Isbn13 && id.id_value == SAMPLE_ISBN13));
            assert!(ids
                .iter()
                .any(|id| id.id_type == ExternalIdType::Asin && id.id_value == SAMPLE_ASIN));
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_returns_superseded_when_generation_is_stale() {
            // REQ-ID: R-02, R-22 | Contract: WorkDb::apply_enrichment_merge | Behavior: returns Superseded when expected merge_generation does not match the current generation
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            db.apply_enrichment_merge(sample_merge_request(u1, work.id, 0))
                .await
                .unwrap();

            let outcome = db
                .apply_enrichment_merge(stale_merge_request(u1, work.id, 0))
                .await
                .unwrap();

            assert_eq!(outcome, ApplyMergeOutcome::Superseded);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_returns_superseded_when_expected_generation_is_in_future() {
            // REQ-ID: R-02, R-22 | Contract: WorkDb::apply_enrichment_merge | Behavior: returns Superseded when expected merge_generation is greater than the current generation
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            let current = db.get_merge_generation(u1, work.id).await.unwrap();
            let outcome = db
                .apply_enrichment_merge(sample_merge_request(u1, work.id, current + 1))
                .await
                .unwrap();

            assert_eq!(outcome, ApplyMergeOutcome::Superseded);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_superseded_does_not_modify_work_fields() {
            // REQ-ID: R-02, R-22 | Contract: WorkDb::apply_enrichment_merge | Behavior: leaves work metadata columns unchanged when CAS is superseded
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            db.apply_enrichment_merge(sample_merge_request(u1, work.id, 0))
                .await
                .unwrap();
            let before = db.get_work(u1, work.id).await.unwrap();

            db.apply_enrichment_merge(stale_merge_request(u1, work.id, 0))
                .await
                .unwrap();
            let after = db.get_work(u1, work.id).await.unwrap();

            assert_eq!(after.title, before.title);
            assert_eq!(after.description, before.description);
            assert_eq!(after.gr_key, before.gr_key);
            assert_eq!(after.cover_url, before.cover_url);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_superseded_does_not_increment_generation() {
            // REQ-ID: R-22 | Contract: WorkDb::apply_enrichment_merge | Behavior: does not increment merge_generation when CAS is superseded
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            db.apply_enrichment_merge(sample_merge_request(u1, work.id, 0))
                .await
                .unwrap();
            let before = db.get_merge_generation(u1, work.id).await.unwrap();

            db.apply_enrichment_merge(stale_merge_request(u1, work.id, 0))
                .await
                .unwrap();
            let after = db.get_merge_generation(u1, work.id).await.unwrap();

            assert_eq!(before, 1);
            assert_eq!(after, before);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_superseded_writes_nothing_to_side_tables() {
            // REQ-ID: R-02, R-22 | Contract: WorkDb::apply_enrichment_merge | Behavior: writes nothing to provenance or external_ids when CAS is superseded
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            db.apply_enrichment_merge(sample_merge_request(u1, work.id, 0))
                .await
                .unwrap();

            let mut stale = stale_merge_request(u1, work.id, 0);
            stale.provenance_upserts = sample_provenance_upserts(u1, work.id);
            stale.external_id_updates = sample_external_ids(work.id);

            let outcome = db.apply_enrichment_merge(stale).await.unwrap();

            let title_provenance = db
                .get_field_provenance(u1, work.id, WorkField::Title)
                .await
                .unwrap();
            let external_ids = db.list_external_ids(u1, work.id).await.unwrap();

            assert_eq!(outcome, ApplyMergeOutcome::Superseded);
            assert!(title_provenance.is_none());
            assert!(external_ids.is_empty());
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_conflict_path_sets_status_conflict() {
            // REQ-ID: R-02, R-21 | Contract: WorkDb::apply_enrichment_merge | Behavior: sets enrichment_status to Conflict when called with work_update=None and new_status=Conflict
            let (db, u1, _) = $setup().await;
            let work = create_merged_work(&db, u1).await;

            db.apply_enrichment_merge(conflict_merge_request(u1, work.id, 1))
                .await
                .unwrap();

            let got = db.get_work(u1, work.id).await.unwrap();

            assert_eq!(got.enrichment_status, EnrichmentStatus::Conflict);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_conflict_path_increments_generation() {
            // REQ-ID: R-22 | Contract: WorkDb::apply_enrichment_merge | Behavior: increments merge_generation on the conflict status-only path
            let (db, u1, _) = $setup().await;
            let work = create_merged_work(&db, u1).await;

            let before = db.get_merge_generation(u1, work.id).await.unwrap();
            db.apply_enrichment_merge(conflict_merge_request(u1, work.id, before))
                .await
                .unwrap();
            let after = db.get_merge_generation(u1, work.id).await.unwrap();

            assert_eq!(before, 1);
            assert_eq!(after, before + 1);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_conflict_path_does_not_modify_metadata_columns() {
            // REQ-ID: R-02, R-21 | Contract: WorkDb::apply_enrichment_merge | Behavior: does not modify work metadata columns on the conflict status-only path
            let (db, u1, _) = $setup().await;
            let work = create_merged_work(&db, u1).await;
            let before = db.get_work(u1, work.id).await.unwrap();

            db.apply_enrichment_merge(conflict_merge_request(u1, work.id, 1))
                .await
                .unwrap();
            let after = db.get_work(u1, work.id).await.unwrap();

            assert_eq!(after.title, before.title);
            assert_eq!(after.author_name, before.author_name);
            assert_eq!(after.description, before.description);
            assert_eq!(after.gr_key, before.gr_key);
            assert_eq!(after.cover_url, before.cover_url);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_conflict_path_with_empty_side_tables_sets_conflict_status() {
            // REQ-ID: R-02, R-21 | Contract: ApplyEnrichmentMergeRequest + WorkDb::apply_enrichment_merge | Behavior: caller supplies empty provenance/external-id mutations on conflict path and the work ends in Conflict status
            let (db, u1, _) = $setup().await;
            let work = create_merged_work(&db, u1).await;

            let req = conflict_merge_request(u1, work.id, 1);

            let outcome = db.apply_enrichment_merge(req).await.unwrap();
            let got = db.get_work(u1, work.id).await.unwrap();

            assert_eq!(outcome, ApplyMergeOutcome::Applied);
            assert_eq!(got.enrichment_status, EnrichmentStatus::Conflict);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_conflict_path_with_empty_side_tables_increments_generation() {
            // REQ-ID: R-02, R-21, R-22 | Contract: ApplyEnrichmentMergeRequest + WorkDb::apply_enrichment_merge | Behavior: caller conflict request with empty side-table mutations applies status-only write and increments merge_generation
            let (db, u1, _) = $setup().await;
            let work = create_merged_work(&db, u1).await;
            let before_generation = db.get_merge_generation(u1, work.id).await.unwrap();

            let req = conflict_merge_request(u1, work.id, before_generation);

            let outcome = db.apply_enrichment_merge(req).await.unwrap();
            let after_generation = db.get_merge_generation(u1, work.id).await.unwrap();
            let got = db.get_work(u1, work.id).await.unwrap();

            assert_eq!(outcome, ApplyMergeOutcome::Applied);
            assert_eq!(got.enrichment_status, EnrichmentStatus::Conflict);
            assert_eq!(after_generation, before_generation + 1);
        }

        #[tokio::test]
        async fn test_work_db_merge_reset_for_manual_refresh_sets_status_pending() {
            // REQ-ID: R-22 | Contract: WorkDb::reset_for_manual_refresh | Behavior: sets enrichment_status to Pending and clears enriched_at
            let (db, u1, _) = $setup().await;
            let work = create_merged_work(&db, u1).await;

            db.reset_for_manual_refresh(u1, work.id).await.unwrap();

            let got = db.get_work(u1, work.id).await.unwrap();

            assert_eq!(got.enrichment_status, EnrichmentStatus::Unenriched);
            assert!(got.enriched_at.is_none());
        }

        #[tokio::test]
        async fn test_work_db_merge_reset_for_manual_refresh_increments_generation() {
            // REQ-ID: R-22 | Contract: WorkDb::reset_for_manual_refresh | Behavior: increments merge_generation by 1
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            let before = db.get_merge_generation(u1, work.id).await.unwrap();
            db.reset_for_manual_refresh(u1, work.id).await.unwrap();
            let after = db.get_merge_generation(u1, work.id).await.unwrap();

            assert_eq!(before, 0);
            assert_eq!(after, before + 1);
        }

        #[tokio::test]
        async fn test_work_db_merge_reset_for_manual_refresh_deletes_retry_state_rows() {
            // REQ-ID: R-22 | Contract: WorkDb::reset_for_manual_refresh | Behavior: deletes provider_retry_state rows for the work
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            seed_retry_rows(&db, u1, work.id).await;
            let before = db.list_retry_states(u1, work.id).await.unwrap();
            db.reset_for_manual_refresh(u1, work.id).await.unwrap();
            let after = db.list_retry_states(u1, work.id).await.unwrap();

            assert_eq!(before.len(), 2);
            assert!(after.is_empty());
        }

        #[tokio::test]
        async fn test_work_db_merge_reset_for_manual_refresh_preserves_provenance_rows() {
            // REQ-ID: R-22 | Contract: WorkDb::reset_for_manual_refresh | Behavior: does not touch work_metadata_provenance rows
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            db.set_field_provenance(user_provenance(u1, work.id, WorkField::Description))
                .await
                .unwrap();

            db.reset_for_manual_refresh(u1, work.id).await.unwrap();

            let provenance = db
                .get_field_provenance(u1, work.id, WorkField::Description)
                .await
                .unwrap()
                .unwrap();

            assert_eq!(provenance.field, WorkField::Description);
            assert_eq!(provenance.setter, ProvenanceSetter::User);
            assert!(!provenance.cleared);
        }

        #[tokio::test]
        async fn test_work_db_merge_list_conflict_works_returns_only_conflicts_for_user() {
            // REQ-ID: R-21 | Contract: WorkDb::list_conflict_works | Behavior: returns only works with enrichment_status=Conflict for the requested user
            let (db, u1, u2) = $setup().await;

            let conflict_u1 = create_conflict_work(&db, u1).await;
            let non_conflict_u1 = db
                .create_work(make_work_req(u1, "Non Conflict Work", "Different Author"))
                .await
                .unwrap()
                .0;
            let conflict_u2 = create_conflict_work(&db, u2).await;

            let conflicts = db.list_conflict_works(u1).await.unwrap();
            let ids: Vec<WorkId> = conflicts.iter().map(|w| w.id).collect();

            assert!(ids.contains(&conflict_u1.id));
            assert!(!ids.contains(&non_conflict_u1.id));
            assert!(!ids.contains(&conflict_u2.id));
            assert_eq!(ids.len(), 1);
        }

        #[tokio::test]
        async fn test_work_db_merge_apply_enrichment_merge_provenance_deletes_remove_rows() {
            // REQ-ID: R-02 | Contract: WorkDb::apply_enrichment_merge | Behavior: removes provenance rows listed in provenance_deletes
            let (db, u1, _) = $setup().await;
            let work = create_new_work(&db, u1).await;

            db.set_field_provenance(provider_provenance(
                u1,
                work.id,
                WorkField::Title,
                MetadataProvider::Goodreads,
            ))
            .await
            .unwrap();
            db.set_field_provenance(provider_provenance(
                u1,
                work.id,
                WorkField::Description,
                MetadataProvider::OpenLibrary,
            ))
            .await
            .unwrap();

            let mut req = sample_merge_request(u1, work.id, 0);
            req.provenance_deletes = vec![WorkField::Description];

            db.apply_enrichment_merge(req).await.unwrap();

            let title_provenance = db
                .get_field_provenance(u1, work.id, WorkField::Title)
                .await
                .unwrap();
            let description_provenance = db
                .get_field_provenance(u1, work.id, WorkField::Description)
                .await
                .unwrap();

            assert!(title_provenance.is_some());
            assert!(description_provenance.is_none());
        }
    };
}

work_db_merge_tests!(setup_sqlite);
