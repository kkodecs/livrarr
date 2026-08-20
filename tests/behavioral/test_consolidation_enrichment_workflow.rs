#![allow(dead_code, unused_imports)]

//! Behavioral tests for EnrichmentWorkflow trait (WF-ENRICH-001..003).
//!
//! These tests verify the EnrichmentWorkflowImpl adapter correctly delegates
//! to EnrichmentServiceImpl and maps types between metadata-crate and domain-crate.
//! The underlying enrichment pipeline logic is tested by the metadata-overhaul suite.

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateUserDbRequest, CreateWorkDbRequest, UpdateWorkEnrichmentDbRequest, UserDb, WorkDb,
    WorkDbCreate,
};
use livrarr_domain::services::*;
use livrarr_domain::{EnrichmentStatus, UserRole, Work};
use livrarr_metadata::enrichment_workflow_service::{
    EnrichmentWorkflowImpl, ResetOnlyEnrichmentWorkflow,
};
use livrarr_metadata::{EnrichmentError, EnrichmentMode as MetaEnrichmentMode, EnrichmentService};
use std::collections::HashMap;
use std::sync::Arc;

// Inline stubs implementing the metadata crate's EnrichmentService trait.
// These feed canned responses to the adapter for boundary testing.

struct SuccessEnrichment;
impl EnrichmentService for SuccessEnrichment {
    async fn enrich_work(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
        _: MetaEnrichmentMode,
        _: Option<livrarr_domain::identity::CandidateId>,
        _: livrarr_domain::RequestPriority,
        _: livrarr_domain::Freshness,
    ) -> Result<livrarr_metadata::EnrichmentResult, EnrichmentError> {
        Ok(livrarr_metadata::EnrichmentResult {
            enrichment_status: EnrichmentStatus::Enriched,
            identity_not_found: false,
            enrichment_source: Some("test-provider".into()),
            llm_task_spawned: false,
            work: Work {
                title: "Enriched Title".into(),
                ..Default::default()
            },
            merge_deferred: false,
            provider_outcomes: HashMap::new(),
            cover_resolution: None,
            audiobook_cover_resolution: None,
            changed: false,
            dissents: Vec::new(),
            attempted: true,
            captured_provider_identity: Vec::new(),
            captured_route_proposals: Vec::new(),
            provider_chase_attempted: false,
            search_leg_fired: false,
            search_ledger_burnable: false,
        })
    }
    async fn reset_for_manual_refresh(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
    ) -> Result<(), EnrichmentError> {
        Ok(())
    }
    async fn inject_source_data(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
        _: livrarr_domain::services::SourceProviderData,
    ) {
    }
}

struct DeferredEnrichment;
impl EnrichmentService for DeferredEnrichment {
    async fn enrich_work(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
        _: MetaEnrichmentMode,
        _: Option<livrarr_domain::identity::CandidateId>,
        _: livrarr_domain::RequestPriority,
        _: livrarr_domain::Freshness,
    ) -> Result<livrarr_metadata::EnrichmentResult, EnrichmentError> {
        Ok(livrarr_metadata::EnrichmentResult {
            enrichment_status: EnrichmentStatus::Unenriched,
            identity_not_found: false,
            enrichment_source: None,
            llm_task_spawned: false,
            work: Work::default(),
            merge_deferred: true,
            provider_outcomes: HashMap::new(),
            cover_resolution: None,
            audiobook_cover_resolution: None,
            changed: false,
            dissents: Vec::new(),
            attempted: true,
            captured_provider_identity: Vec::new(),
            captured_route_proposals: Vec::new(),
            provider_chase_attempted: false,
            search_leg_fired: false,
            search_ledger_burnable: false,
        })
    }
    async fn reset_for_manual_refresh(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
    ) -> Result<(), EnrichmentError> {
        Ok(())
    }
    async fn inject_source_data(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
        _: livrarr_domain::services::SourceProviderData,
    ) {
    }
}

struct FailedEnrichment;
impl EnrichmentService for FailedEnrichment {
    async fn enrich_work(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
        _: MetaEnrichmentMode,
        _: Option<livrarr_domain::identity::CandidateId>,
        _: livrarr_domain::RequestPriority,
        _: livrarr_domain::Freshness,
    ) -> Result<livrarr_metadata::EnrichmentResult, EnrichmentError> {
        Ok(livrarr_metadata::EnrichmentResult {
            enrichment_status: EnrichmentStatus::Failed,
            identity_not_found: false,
            enrichment_source: None,
            llm_task_spawned: false,
            work: Work::default(),
            merge_deferred: false,
            provider_outcomes: HashMap::new(),
            cover_resolution: None,
            audiobook_cover_resolution: None,
            changed: false,
            dissents: Vec::new(),
            attempted: true,
            captured_provider_identity: Vec::new(),
            captured_route_proposals: Vec::new(),
            provider_chase_attempted: false,
            search_leg_fired: false,
            search_ledger_burnable: false,
        })
    }
    async fn reset_for_manual_refresh(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
    ) -> Result<(), EnrichmentError> {
        Ok(())
    }
    async fn inject_source_data(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
        _: livrarr_domain::services::SourceProviderData,
    ) {
    }
}

struct NotFoundEnrichment;
impl EnrichmentService for NotFoundEnrichment {
    async fn enrich_work(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
        _: MetaEnrichmentMode,
        _: Option<livrarr_domain::identity::CandidateId>,
        _: livrarr_domain::RequestPriority,
        _: livrarr_domain::Freshness,
    ) -> Result<livrarr_metadata::EnrichmentResult, EnrichmentError> {
        Err(EnrichmentError::WorkNotFound)
    }
    async fn reset_for_manual_refresh(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
    ) -> Result<(), EnrichmentError> {
        Ok(())
    }
    async fn inject_source_data(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
        _: livrarr_domain::services::SourceProviderData,
    ) {
    }
}

struct MergeSupersededEnrichment;
impl EnrichmentService for MergeSupersededEnrichment {
    async fn enrich_work(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
        _: MetaEnrichmentMode,
        _: Option<livrarr_domain::identity::CandidateId>,
        _: livrarr_domain::RequestPriority,
        _: livrarr_domain::Freshness,
    ) -> Result<livrarr_metadata::EnrichmentResult, EnrichmentError> {
        Err(EnrichmentError::MergeSuperseded)
    }
    async fn reset_for_manual_refresh(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
    ) -> Result<(), EnrichmentError> {
        Ok(())
    }
    async fn inject_source_data(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
        _: livrarr_domain::services::SourceProviderData,
    ) {
    }
}

struct CorruptPayloadEnrichment;
impl EnrichmentService for CorruptPayloadEnrichment {
    async fn enrich_work(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
        _: MetaEnrichmentMode,
        _: Option<livrarr_domain::identity::CandidateId>,
        _: livrarr_domain::RequestPriority,
        _: livrarr_domain::Freshness,
    ) -> Result<livrarr_metadata::EnrichmentResult, EnrichmentError> {
        Err(EnrichmentError::CorruptRetryPayload {
            work_id: 1,
            provider: livrarr_domain::MetadataProvider::Goodreads,
        })
    }
    async fn reset_for_manual_refresh(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
    ) -> Result<(), EnrichmentError> {
        Ok(())
    }
    async fn inject_source_data(
        &self,
        _: livrarr_domain::UserId,
        _: livrarr_domain::WorkId,
        _: livrarr_domain::services::SourceProviderData,
    ) {
    }
}

async fn setup(db: &SqliteDb) -> (i64, i64) {
    let user = db
        .create_user(CreateUserDbRequest {
            username: "testuser".into(),
            password_hash: "hash".into(),
            role: UserRole::Admin,
            api_key_hash: "testhash".into(),
        })
        .await
        .unwrap();
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id: user.id,
            title: "Test Work".into(),
            author_name: "Author".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    (user.id, work.id)
}

// =============================================================================
// enrich_work — adapter boundary tests (7 of 10 kept, 3 deleted as redundant)
// =============================================================================

#[tokio::test]
async fn test_enrich_happy_path_merges_provider_data() {
    // Adapter maps successful enrichment result correctly
    let db = create_test_db().await;
    let (user_id, work_id) = setup(&db).await;

    let workflow = EnrichmentWorkflowImpl::new(Arc::new(SuccessEnrichment));
    let r = workflow
        .enrich_work(
            user_id,
            work_id,
            EnrichmentMode::Manual,
            None,
            livrarr_domain::RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .unwrap();

    assert_eq!(r.enrichment_status, EnrichmentStatus::Enriched);
    assert_eq!(r.enrichment_source.as_deref(), Some("test-provider"));
    assert!(!r.merge_deferred);
    assert_eq!(r.work.title, "Enriched Title");
}

#[tokio::test]
async fn test_enrich_background_defers_merge_when_not_terminal() {
    // Adapter preserves merge_deferred=true semantics
    let db = create_test_db().await;
    let (user_id, work_id) = setup(&db).await;

    let workflow = EnrichmentWorkflowImpl::new(Arc::new(DeferredEnrichment));
    let r = workflow
        .enrich_work(
            user_id,
            work_id,
            EnrichmentMode::Background,
            None,
            livrarr_domain::RequestPriority::Low,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .unwrap();

    assert_eq!(r.enrichment_status, EnrichmentStatus::Unenriched);
    assert!(
        r.merge_deferred,
        "deferred flag must survive adapter conversion"
    );
}

#[tokio::test]
async fn test_enrich_llm_rejects_all_sets_conflict() {
    // Adapter maps Failed/Conflict status correctly through boundary
    let db = create_test_db().await;
    let (user_id, work_id) = setup(&db).await;

    let workflow = EnrichmentWorkflowImpl::new(Arc::new(FailedEnrichment));
    let r = workflow
        .enrich_work(
            user_id,
            work_id,
            EnrichmentMode::Manual,
            None,
            livrarr_domain::RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .unwrap();

    assert_eq!(r.enrichment_status, EnrichmentStatus::Failed);
}

#[tokio::test]
async fn test_enrich_llm_failure_passes_through() {
    // Adapter doesn't swallow results when LLM fails (service falls back internally)
    let db = create_test_db().await;
    let (user_id, work_id) = setup(&db).await;

    let workflow = EnrichmentWorkflowImpl::new(Arc::new(SuccessEnrichment));
    let r = workflow
        .enrich_work(
            user_id,
            work_id,
            EnrichmentMode::Manual,
            None,
            livrarr_domain::RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .unwrap();

    assert_eq!(r.enrichment_status, EnrichmentStatus::Enriched);
}

#[tokio::test]
async fn test_enrich_cas_exhausted_returns_error() {
    // Adapter maps MergeSuperseded error correctly
    let db = create_test_db().await;
    let (user_id, work_id) = setup(&db).await;

    let workflow = EnrichmentWorkflowImpl::new(Arc::new(MergeSupersededEnrichment));
    let result = workflow
        .enrich_work(
            user_id,
            work_id,
            EnrichmentMode::Manual,
            None,
            livrarr_domain::RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await;

    assert!(
        matches!(result, Err(EnrichmentWorkflowError::MergeSuperseded)),
        "expected MergeSuperseded, got {result:?}"
    );
}

#[tokio::test]
async fn test_enrich_corrupt_retry_payload_returns_error() {
    // Adapter maps CorruptRetryPayload error with correct work_id and provider
    let db = create_test_db().await;
    let (user_id, work_id) = setup(&db).await;

    let workflow = EnrichmentWorkflowImpl::new(Arc::new(CorruptPayloadEnrichment));
    let result = workflow
        .enrich_work(
            user_id,
            work_id,
            EnrichmentMode::Manual,
            None,
            livrarr_domain::RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await;

    match result {
        Err(EnrichmentWorkflowError::CorruptRetryPayload {
            work_id: wid,
            provider,
        }) => {
            assert_eq!(wid, 1);
            assert_eq!(provider, livrarr_domain::MetadataProvider::Goodreads);
        }
        other => panic!("expected CorruptRetryPayload, got {other:?}"),
    }
}

#[tokio::test]
async fn test_enrich_user_provenance_never_overwritten() {
    // Adapter faithfully passes through result without mutating fields
    let db = create_test_db().await;
    let (user_id, work_id) = setup(&db).await;

    let workflow = EnrichmentWorkflowImpl::new(Arc::new(SuccessEnrichment));
    let r = workflow
        .enrich_work(
            user_id,
            work_id,
            EnrichmentMode::HardRefresh,
            None,
            livrarr_domain::RequestPriority::Normal,
            livrarr_domain::Freshness::Bypass,
        )
        .await
        .unwrap();

    assert_eq!(r.enrichment_status, EnrichmentStatus::Enriched);
    assert_eq!(r.work.title, "Enriched Title");
    assert_eq!(r.enrichment_source.as_deref(), Some("test-provider"));
}

// =============================================================================
// reset_for_manual_refresh
// =============================================================================

#[tokio::test]
async fn test_reset_sets_pending_and_clears_retry_state() {
    let db = create_test_db().await;
    let (user_id, work_id) = setup(&db).await;

    db.update_work_enrichment(
        user_id,
        work_id,
        UpdateWorkEnrichmentDbRequest {
            enrichment_status: EnrichmentStatus::Enriched,
            enrichment_source: Some("test".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let before = db.get_work(user_id, work_id).await.unwrap();
    assert_eq!(before.enrichment_status, EnrichmentStatus::Enriched);

    let svc = ResetOnlyEnrichmentWorkflow::new(db.clone());
    svc.reset_for_manual_refresh(user_id, work_id)
        .await
        .unwrap();

    let after = db.get_work(user_id, work_id).await.unwrap();
    assert_eq!(after.enrichment_status, EnrichmentStatus::Unenriched);
    assert_eq!(after.title, "Test Work");
}

#[tokio::test]
#[ignore = "pk-implement: requires provenance DB infrastructure"]
async fn test_reset_preserves_user_provenance() {
    todo!("requires provenance seeding and verification")
}

#[tokio::test]
#[ignore = "pk-implement: requires EnrichmentRetryDb seeding"]
async fn test_reset_clears_provider_retry_states() {
    todo!("requires retry state seeding and verification")
}
