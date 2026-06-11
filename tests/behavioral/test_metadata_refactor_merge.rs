use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use livrarr_db::UpdateWorkEnrichmentDbRequest;
use livrarr_domain::{
    services::{LlmCallRequest, LlmCallResponse, LlmCaller, LlmError},
    EnrichmentStatus, FieldProvenance, MetadataProvider, ProvenanceSetter, UserId, Work, WorkField,
};
use livrarr_external_data::NormalizedWorkDetail;
use livrarr_metadata::{
    DefaultMergeEngine, EnrichmentMode, MergeEngine, MergeInput, PriorityModel,
    ReconstructedOutcome,
};

#[derive(Clone, Default)]
struct SpyLlmCaller {
    calls: Arc<AtomicUsize>,
}

impl SpyLlmCaller {
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl LlmCaller for SpyLlmCaller {
    async fn call(&self, _req: LlmCallRequest) -> Result<LlmCallResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmCallResponse {
            content: r#"{"identity_valid":true,"conflict_providers":[],"fields":{}}"#.to_string(),
            model_used: "spy".to_string(),
            elapsed: Duration::from_millis(1),
        })
    }
}

fn work() -> Work {
    Work {
        id: 7,
        user_id: 11,
        title: "Contract Book".to_string(),
        author_name: "Contract Author".to_string(),
        language: Some("en".to_string()),
        gr_key: Some("existing-gr".to_string()),
        series_name: Some("User Series".to_string()),
        series_position: Some(2.0),
        cover_url: Some("https://covers.example.test/user.jpg".to_string()),
        cover_manual: true,
        ..Default::default()
    }
}

fn detail() -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: Some("Contract Book".to_string()),
        author_name: Some("Contract Author".to_string()),
        description: Some("Provider description".to_string()),
        series_name: Some("Provider Series".to_string()),
        series_position: Some(9.0),
        isbn_13: Some("9780000000002".to_string()),
        cover_url: Some("https://covers.example.test/provider.jpg".to_string()),
        ..Default::default()
    }
}

fn outcome(payload: NormalizedWorkDetail) -> ReconstructedOutcome {
    ReconstructedOutcome {
        class: livrarr_domain::OutcomeClass::Success,
        payload: Some(payload),
    }
}

fn merge_input(
    current_work: Work,
    provider_results: HashMap<MetadataProvider, ReconstructedOutcome>,
    provenance: Vec<FieldProvenance>,
) -> MergeInput {
    MergeInput {
        current_work,
        current_provenance: provenance,
        provider_results,
        mode: EnrichmentMode::Manual,
        priority_model: PriorityModel::english(),
    }
}

fn update(output: livrarr_metadata::MergeOutput) -> UpdateWorkEnrichmentDbRequest {
    output
        .work_update
        .expect("merge should produce a work update")
        .into_inner()
}

#[tokio::test]
async fn full_enrichment_merge_makes_zero_llm_calls() {
    // AC-003
    let spy = SpyLlmCaller::default();
    let engine = DefaultMergeEngine::new_with_llm(PriorityModel::english(), spy.clone(), true);
    let input = merge_input(
        work(),
        HashMap::from([(MetadataProvider::Hardcover, outcome(detail()))]),
        vec![],
    );

    let output = engine.merge(input).await.expect("merge should complete");

    assert_eq!(output.enrichment_status, EnrichmentStatus::Enriched);
    assert_eq!(
        spy.call_count(),
        0,
        "metadata-refactor forbids LLM calls during merge even when an LLM caller is configured"
    );
}

#[tokio::test]
async fn user_set_series_and_cover_survive_refresh() {
    // AC-005
    let engine = DefaultMergeEngine::new(PriorityModel::english());
    let provenance = vec![
        FieldProvenance {
            user_id: 11 as UserId,
            work_id: 7,
            field: WorkField::SeriesName,
            source: None,
            set_at: chrono::Utc::now(),
            setter: ProvenanceSetter::User,
            cleared: false,
        },
        FieldProvenance {
            user_id: 11 as UserId,
            work_id: 7,
            field: WorkField::SeriesPosition,
            source: None,
            set_at: chrono::Utc::now(),
            setter: ProvenanceSetter::User,
            cleared: false,
        },
        FieldProvenance {
            user_id: 11 as UserId,
            work_id: 7,
            field: WorkField::CoverUrl,
            source: None,
            set_at: chrono::Utc::now(),
            setter: ProvenanceSetter::User,
            cleared: false,
        },
    ];

    let output = engine
        .merge(merge_input(
            work(),
            HashMap::from([(MetadataProvider::Hardcover, outcome(detail()))]),
            provenance,
        ))
        .await
        .expect("refresh merge should complete");
    let update = update(output);

    assert_eq!(update.series_name, Some("User Series".to_string()));
    assert_eq!(update.series_position, Some(2.0));
    assert_eq!(
        update.cover_url,
        Some("https://covers.example.test/user.jpg".to_string())
    );
}

#[tokio::test]
async fn cover_selection_uses_provider_priority_not_pixel_dimensions() {
    // AC-012
    let engine = DefaultMergeEngine::new(PriorityModel::english());
    let mut high_priority_small = detail();
    high_priority_small.cover_url = Some("https://covers.example.test/hc-small.jpg".to_string());
    let mut low_priority_large = detail();
    low_priority_large.cover_url = Some("https://covers.example.test/gr-large.jpg".to_string());

    let output = engine
        .merge(merge_input(
            work(),
            HashMap::from([
                (MetadataProvider::Hardcover, outcome(high_priority_small)),
                (MetadataProvider::Goodreads, outcome(low_priority_large)),
            ]),
            vec![],
        ))
        .await
        .expect("cover merge should complete");

    assert_eq!(
        update(output).cover_url,
        Some("https://covers.example.test/hc-small.jpg".to_string()),
        "higher-priority provider must win even when a lower-priority provider has a larger image"
    );
}
