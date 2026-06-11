#![allow(dead_code)]

use livrarr_behavioral::stubs::create_test_user;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    ApplyEnrichmentMergeRequest, CreateWorkDbRequest, ExternalIdDb, UpsertExternalIdRequest,
    WorkDb, WorkDbCreate,
};
use livrarr_domain::identity::{AnchorSetter, AnchorType};
use livrarr_domain::services::WorkIdentityRepository;
use livrarr_domain::{
    EnrichmentStatus, ExternalIdType, MergeResolved, MetadataProvider, OutcomeClass, Work,
};
use livrarr_external_data::NormalizedWorkDetail;
use livrarr_metadata::{
    DefaultMergeEngine, EnrichmentMode, MergeEngine, MergeInput, PriorityModel,
    ReconstructedOutcome,
};

fn work_request(user_id: i64) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: "Pan Tadeusz".to_string(),
        author_name: "Adam Mickiewicz".to_string(),
        normalized_title: "pan tadeusz".to_string(),
        normalized_author: "adam mickiewicz".to_string(),
        ol_key: Some("OL100W".to_string()),
        gr_key: Some("GR-CLEAN".to_string()),
        isbn_13: Some("9788306031153".to_string()),
        asin: Some("B000CLEAN".to_string()),
        description: Some("Polish epic poem".to_string()),
        language: Some("pl".to_string()),
        cover_url: Some("https://example.test/pan.jpg".to_string()),
        monitor_ebook: true,
        ..Default::default()
    }
}

fn wrong_book_payload() -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: Some("The Duke and I".to_string()),
        author_name: Some("Julia Quinn".to_string()),
        description: Some("English romance description".to_string()),
        cover_url: Some("https://example.test/bridgerton.jpg".to_string()),
        language: Some("en".to_string()),
        series_name: Some("Bridgertons".to_string()),
        gr_key: Some("GR-WRONG".to_string()),
        hc_key: Some("HC-WRONG".to_string()),
        ol_key: Some("OL-WRONG".to_string()),
        isbn_13: Some("9780062353597".to_string()),
        asin: Some("B00WRONG".to_string()),
        additional_isbns: vec!["9780062353597".to_string()],
        additional_asins: vec!["B00WRONG".to_string()],
        ..NormalizedWorkDetail::default()
    }
}

async fn seeded_work() -> (livrarr_db::sqlite::SqliteDb, i64, Work) {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (work, _) = db.create_work(work_request(user_id)).await.unwrap();
    db.confirm_anchor(
        work.id,
        AnchorType::new(AnchorType::HC_WORK),
        "HC-CLEAN",
        AnchorSetter::User,
    )
    .await
    .unwrap();
    db.upsert_external_id(
        user_id,
        UpsertExternalIdRequest {
            work_id: work.id,
            id_type: ExternalIdType::Asin,
            id_value: "B000CLEAN".to_string(),
        },
    )
    .await
    .unwrap();
    let work = db.get_work(user_id, work.id).await.unwrap();
    (db, user_id, work)
}

fn provider_results() -> std::collections::HashMap<MetadataProvider, ReconstructedOutcome> {
    std::collections::HashMap::from([(
        MetadataProvider::Goodreads,
        ReconstructedOutcome {
            class: OutcomeClass::Success,
            payload: Some(wrong_book_payload()),
        },
    )])
}

fn external_id_values(rows: Vec<livrarr_db::ExternalId>) -> Vec<(i64, ExternalIdType, String)> {
    rows.into_iter()
        .map(|row| (row.work_id, row.id_type, row.id_value))
        .collect()
}

#[tokio::test]
async fn test_mc_merge_payload_anchors_do_not_change_inline_external_or_ledger_stores() {
    // REQ-007 / AC-009
    let (db, user_id, work) = seeded_work().await;
    let before_work = db.get_work(user_id, work.id).await.unwrap();
    let before_external_ids =
        external_id_values(db.list_external_ids(user_id, work.id).await.unwrap());
    let before_anchors = db.list_anchors(work.id).await.unwrap();

    let output = DefaultMergeEngine
        .merge(MergeInput {
            current_work: before_work.clone(),
            current_provenance: Vec::new(),
            provider_results: provider_results(),
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::for_language(before_work.language.as_deref()),
        })
        .await
        .unwrap();

    if let Some(update) = output.work_update {
        db.apply_enrichment_merge(ApplyEnrichmentMergeRequest {
            user_id,
            work_id: before_work.id,
            expected_merge_generation: db
                .get_merge_generation(user_id, before_work.id)
                .await
                .unwrap(),
            work_update: Some(update),
            new_enrichment_status: output.enrichment_status,
            provenance_upserts: output.provenance_upserts,
            provenance_deletes: output.provenance_deletes,
        })
        .await
        .unwrap();
    }

    let after_work = db.get_work(user_id, before_work.id).await.unwrap();
    assert_eq!(after_work.ol_key, before_work.ol_key);
    assert_eq!(after_work.gr_key, before_work.gr_key);
    assert_eq!(after_work.hc_key, before_work.hc_key);
    assert_eq!(after_work.isbn_13, before_work.isbn_13);
    assert_eq!(after_work.asin, before_work.asin);
    assert_eq!(
        external_id_values(db.list_external_ids(user_id, before_work.id).await.unwrap()),
        before_external_ids
    );
    assert_eq!(
        db.list_anchors(before_work.id).await.unwrap(),
        before_anchors
    );
}

#[tokio::test]
async fn test_mc_f1_foreign_fixture_keeps_anchors_and_series_name_unchanged() {
    // REQ-007/REQ-013 — the AC-008 write-site slice. The F1 wrong-book payload
    // (known-English text on a Polish work) must not land its fields: the wrong
    // book's series never reaches the update. The road-level AC-008 conjunction
    // is pinned per-seam (dispatch skip in test_mc_anchor_grounding, the
    // three-store guard above, completion-before-scatter in
    // test_mc_refresh_orchestration) and verified end-to-end against the
    // forensic DB at the Test stage.
    let (_db, _user_id, work) = seeded_work().await;
    let output = DefaultMergeEngine
        .merge(MergeInput {
            current_work: Work {
                series_name: None,
                language: Some("pl".to_string()),
                ..work
            },
            current_provenance: Vec::new(),
            provider_results: provider_results(),
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::foreign(),
        })
        .await
        .unwrap();

    if let Some(update) = output.work_update {
        assert!(
            update.0.series_name.is_none(),
            "AC-008: the wrong book's series_name must never land on the work"
        );
        assert_ne!(
            update.0.description.as_deref(),
            Some("English romance description"),
            "REQ-013: known-English description on a Polish work is dissent, not a write \
             (the last-known-good echo may re-write the CURRENT value; the offered one never lands)"
        );
    }
    assert!(
        output.dissents.iter().any(|d| d.provider == "goodreads"),
        "REQ-013/014: the excluded wrong-book contribution is recorded as dissent"
    );
}

#[tokio::test]
async fn test_mc_apply_merge_none_or_empty_cover_and_language_do_not_clobber_populated_values() {
    // REQ-009 / AC-011
    let (db, user_id, work) = seeded_work().await;
    let before = db.get_work(user_id, work.id).await.unwrap();

    db.apply_enrichment_merge(ApplyEnrichmentMergeRequest {
        user_id,
        work_id: work.id,
        expected_merge_generation: db.get_merge_generation(user_id, work.id).await.unwrap(),
        work_update: Some(MergeResolved(livrarr_db::UpdateWorkEnrichmentDbRequest {
            cover_url: Some(String::new()),
            language: None,
            enrichment_status: EnrichmentStatus::Enriched,
            ..Default::default()
        })),
        new_enrichment_status: EnrichmentStatus::Enriched,
        provenance_upserts: Vec::new(),
        provenance_deletes: Vec::new(),
    })
    .await
    .unwrap();

    let after = db.get_work(user_id, work.id).await.unwrap();
    assert_eq!(after.cover_url, before.cover_url);
    assert_eq!(after.language, before.language);
}
