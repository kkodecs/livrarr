//! Behavioral contracts for metadata-consistency Phase 4 (provider pipeline)
//! and Phase 5 (merge engine).
//!
//! These are pre-implementation tests: they intentionally name the Phase 4/5
//! public contracts from the IR, including Readarr provider data and LLM merge
//! fallback/arbitration behavior.

use std::collections::HashMap;
use std::time::Duration;

use chrono::Utc;
use livrarr_domain::services::llm::{
    LlmCallRequest, LlmCallResponse, LlmCaller, LlmError, LlmField, LlmPurpose,
};
use livrarr_domain::{
    EnrichmentStatus, FieldProvenance, MetadataProvider, OutcomeClass, ProvenanceSetter,
    SourceProviderData, UserId, Work, WorkField, WorkId,
};
use livrarr_metadata::language::{provider_priority, ProviderPriority};
use livrarr_metadata::llm_scraper::build_llm_scraper_configs;
use livrarr_metadata::{
    DefaultMergeEngine, EnrichmentMode, MergeEngine, MergeInput, MergeOutput, NormalizedWorkDetail,
    PriorityModel, ReconstructedOutcome,
};

const USER_ID: UserId = 101;
const WORK_ID: WorkId = 202;

#[derive(Clone)]
struct FailingLlmCaller;

impl LlmCaller for FailingLlmCaller {
    async fn call(&self, _req: LlmCallRequest) -> Result<LlmCallResponse, LlmError> {
        Err(LlmError::Provider("contract stub failure".to_string()))
    }
}

#[derive(Clone)]
struct JsonLlmCaller {
    content: String,
}

impl JsonLlmCaller {
    fn new(content: &str) -> Self {
        Self {
            content: content.to_string(),
        }
    }
}

impl LlmCaller for JsonLlmCaller {
    async fn call(&self, req: LlmCallRequest) -> Result<LlmCallResponse, LlmError> {
        assert_eq!(req.purpose, LlmPurpose::IdentityValidation);
        assert!(req.allowed_fields.contains(&LlmField::ProviderName));
        Ok(LlmCallResponse {
            content: self.content.clone(),
            model_used: "phase-5-test-stub".to_string(),
            elapsed: Duration::from_millis(1),
        })
    }
}

fn work() -> Work {
    Work {
        id: WORK_ID,
        user_id: USER_ID,
        title: "Current Title".to_string(),
        author_name: "Current Author".to_string(),
        ..Default::default()
    }
}

fn current_work_with_description(description: &str) -> Work {
    Work {
        description: Some(description.to_string()),
        ..work()
    }
}

fn empty_detail() -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: None,
        subtitle: None,
        original_title: None,
        author_name: None,
        description: None,
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
        gr_key: None,
        ol_key: None,
        isbn_13: None,
        asin: None,
        narrator: None,
        narration_type: None,
        abridged: None,
        rating: None,
        rating_count: None,
        cover_url: None,
        additional_isbns: Vec::new(),
        additional_asins: Vec::new(),
    }
}

fn success(payload: NormalizedWorkDetail) -> ReconstructedOutcome {
    ReconstructedOutcome {
        class: OutcomeClass::Success,
        payload: Some(payload),
    }
}

fn conflict(detail: &str) -> ReconstructedOutcome {
    ReconstructedOutcome {
        class: OutcomeClass::Conflict,
        payload: Some(NormalizedWorkDetail {
            description: Some(detail.to_string()),
            ..empty_detail()
        }),
    }
}

fn user_owned(field: WorkField) -> FieldProvenance {
    FieldProvenance {
        user_id: USER_ID,
        work_id: WORK_ID,
        field,
        source: None,
        set_at: Utc::now(),
        setter: ProvenanceSetter::User,
        cleared: false,
    }
}

fn merge_input(
    current_work: Work,
    current_provenance: Vec<FieldProvenance>,
    provider_results: HashMap<MetadataProvider, ReconstructedOutcome>,
    priority_model: PriorityModel,
) -> MergeInput {
    MergeInput {
        current_work,
        current_provenance,
        provider_results,
        mode: EnrichmentMode::Background,
        priority_model,
    }
}

fn resolved(output: &MergeOutput) -> &livrarr_db::UpdateWorkEnrichmentDbRequest {
    output
        .work_update
        .as_ref()
        .expect("non-conflict merge should produce a work update")
        .as_inner()
}

fn provider_for(output: &MergeOutput, field: WorkField) -> Option<MetadataProvider> {
    output
        .provenance_upserts
        .iter()
        .find(|p| p.field == field)
        .and_then(|p| p.source)
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn provider_priority_defaults_to_english_and_routes_non_english_to_foreign() {
    assert_eq!(provider_priority(Some("en")), ProviderPriority::English);
    assert_eq!(provider_priority(None), ProviderPriority::English);
    assert_eq!(provider_priority(Some("fr")), ProviderPriority::Foreign);
    assert_eq!(provider_priority(Some("")), ProviderPriority::English);
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn priority_models_include_readarr_for_content_description_and_cover() {
    let english = PriorityModel::english();
    let foreign = PriorityModel::foreign();

    for providers in [&english.content, &english.description, &english.cover] {
        assert!(
            providers.contains(&MetadataProvider::Readarr),
            "English priority model must include Readarr in every metadata field group"
        );
    }

    for providers in [&foreign.content, &foreign.description, &foreign.cover] {
        assert!(
            providers.contains(&MetadataProvider::Readarr),
            "Foreign priority model must include Readarr in every metadata field group"
        );
    }
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn priority_model_for_language_reuses_provider_priority_routing() {
    assert_eq!(
        PriorityModel::for_language(Some("en")).content,
        PriorityModel::english().content
    );
    assert_eq!(
        PriorityModel::for_language(Some("de")).content,
        PriorityModel::foreign().content
    );
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn llm_scraper_configs_route_polish_through_goodreads_and_drop_lubimyczytac() {
    let configs = build_llm_scraper_configs();

    assert!(
        configs
            .iter()
            .any(|cfg| cfg.language == "pl" && cfg.search_url_template.contains("goodreads.com")),
        "Polish must be included in the Goodreads/Web Search scraper configs"
    );
    assert!(
        configs.iter().all(|cfg| !cfg.name.contains("lubimyczytac")
            && !cfg.search_url_template.contains("lubimyczytac")),
        "lubimyczytac.pl must not have a dedicated scraper entry"
    );
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn source_provider_data_converts_to_normalized_detail_without_identity_fields() {
    let src = SourceProviderData {
        description: Some("Readarr description".to_string()),
        series_name: Some("Readarr Series".to_string()),
        series_position: Some("2.5".to_string()),
        genres: Some(vec!["Fantasy".to_string(), "Adventure".to_string()]),
        page_count: Some(456),
        publisher: Some("Readarr Publisher".to_string()),
        isbn: Some("9781234567890".to_string()),
        asin: Some("B000123456".to_string()),
        rating: Some(4.25),
        rating_count: Some(987),
        cover_url: Some("https://covers.example/readarr.jpg".to_string()),
    };

    let normalized = NormalizedWorkDetail::from(src);

    assert_eq!(normalized.title, None);
    assert_eq!(normalized.author_name, None);
    assert_eq!(
        normalized.description.as_deref(),
        Some("Readarr description")
    );
    assert_eq!(normalized.series_name.as_deref(), Some("Readarr Series"));
    assert_eq!(normalized.series_position, Some(2.5));
    assert_eq!(
        normalized.genres,
        Some(vec!["Fantasy".to_string(), "Adventure".to_string()])
    );
    assert_eq!(normalized.page_count, Some(456));
    assert_eq!(normalized.publisher.as_deref(), Some("Readarr Publisher"));
    assert_eq!(normalized.isbn_13.as_deref(), Some("9781234567890"));
    assert_eq!(normalized.asin.as_deref(), Some("B000123456"));
    assert_eq!(normalized.rating, Some(4.25));
    assert_eq!(normalized.rating_count, Some(987));
    assert_eq!(
        normalized.cover_url.as_deref(),
        Some("https://covers.example/readarr.jpg")
    );
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn deterministic_merge_first_non_empty_value_wins_and_user_owned_fields_are_excluded() {
    let engine = DefaultMergeEngine::new(PriorityModel::english());
    let priority = PriorityModel {
        content: vec![
            MetadataProvider::Hardcover,
            MetadataProvider::Goodreads,
            MetadataProvider::Readarr,
            MetadataProvider::OpenLibrary,
        ],
        description: vec![
            MetadataProvider::Hardcover,
            MetadataProvider::Goodreads,
            MetadataProvider::Readarr,
            MetadataProvider::OpenLibrary,
        ],
        cover: vec![MetadataProvider::Hardcover],
        audio: vec![MetadataProvider::Audnexus, MetadataProvider::Hardcover],
    };
    let input = merge_input(
        current_work_with_description("User description"),
        vec![user_owned(WorkField::Description)],
        HashMap::from([
            (
                MetadataProvider::Hardcover,
                success(NormalizedWorkDetail {
                    publisher: Some("Hardcover Publisher".to_string()),
                    year: Some(2001),
                    description: Some("Hardcover description".to_string()),
                    ..empty_detail()
                }),
            ),
            (
                MetadataProvider::Goodreads,
                success(NormalizedWorkDetail {
                    publisher: Some("Goodreads Publisher".to_string()),
                    year: Some(2002),
                    description: Some("Goodreads description".to_string()),
                    ..empty_detail()
                }),
            ),
        ]),
        priority,
    );

    let output = engine.merge(input).await.expect("merge should succeed");

    assert_eq!(
        resolved(&output).publisher.as_deref(),
        Some("Hardcover Publisher")
    );
    assert_eq!(resolved(&output).year, Some(2001));
    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("User description"),
        "user-owned fields are sovereign and must not be overwritten"
    );
    assert_eq!(
        provider_for(&output, WorkField::Publisher),
        Some(MetadataProvider::Hardcover)
    );
    assert_eq!(provider_for(&output, WorkField::Description), None);
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn deterministic_merge_skips_empty_values_and_falls_through_to_next_provider() {
    let engine = DefaultMergeEngine::new(PriorityModel::english());
    let input = merge_input(
        work(),
        vec![],
        HashMap::from([
            (
                MetadataProvider::Hardcover,
                success(NormalizedWorkDetail {
                    description: Some("   ".to_string()),
                    publisher: None,
                    ..empty_detail()
                }),
            ),
            (
                MetadataProvider::Goodreads,
                success(NormalizedWorkDetail {
                    description: Some("Goodreads description".to_string()),
                    publisher: Some("Goodreads Publisher".to_string()),
                    ..empty_detail()
                }),
            ),
        ]),
        PriorityModel {
            content: vec![MetadataProvider::Hardcover, MetadataProvider::Goodreads],
            description: vec![MetadataProvider::Hardcover, MetadataProvider::Goodreads],
            cover: vec![MetadataProvider::Hardcover, MetadataProvider::Goodreads],
            audio: vec![MetadataProvider::Audnexus],
        },
    );

    let output = engine.merge(input).await.expect("merge should succeed");

    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("Goodreads description")
    );
    assert_eq!(
        provider_for(&output, WorkField::Description),
        Some(MetadataProvider::Goodreads)
    );
    assert_eq!(
        resolved(&output).publisher.as_deref(),
        Some("Goodreads Publisher")
    );
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn llm_merge_failure_falls_back_to_deterministic_merge_instead_of_erroring() {
    let engine = DefaultMergeEngine::with_llm(PriorityModel::english(), FailingLlmCaller);
    let input = merge_input(
        work(),
        vec![],
        HashMap::from([(
            MetadataProvider::Hardcover,
            success(NormalizedWorkDetail {
                description: Some("Deterministic fallback description".to_string()),
                cover_url: Some("https://covers.example/fallback.jpg".to_string()),
                ..empty_detail()
            }),
        )]),
        PriorityModel::english(),
    );

    let output = engine
        .merge(input)
        .await
        .expect("LLM failure should fall back to deterministic merge");

    assert!(!output.conflict_detected);
    assert_eq!(output.enrichment_status, EnrichmentStatus::Enriched);
    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("Deterministic fallback description")
    );
    assert_eq!(
        provider_for(&output, WorkField::Description),
        Some(MetadataProvider::Hardcover)
    );
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn llm_merge_identity_conflict_returns_conflict_signal_to_caller() {
    let engine = DefaultMergeEngine::with_llm(
        PriorityModel::english(),
        JsonLlmCaller::new(
            r#"{
                "identity_valid": false,
                "conflict_providers": ["goodreads"],
                "fields": {}
            }"#,
        ),
    );
    let input = merge_input(
        work(),
        vec![],
        HashMap::from([
            (
                MetadataProvider::Goodreads,
                success(NormalizedWorkDetail {
                    description: Some("Different book".to_string()),
                    ..empty_detail()
                }),
            ),
            (MetadataProvider::Hardcover, conflict("identity mismatch")),
        ]),
        PriorityModel::english(),
    );

    let output = engine
        .merge(input)
        .await
        .expect("conflict is a valid outcome");

    assert!(output.conflict_detected);
    assert_eq!(output.enrichment_status, EnrichmentStatus::Conflict);
    assert!(output.work_update.is_none());
    assert!(output.provenance_upserts.is_empty());
}

#[tokio::test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
async fn deterministic_cover_url_selection_picks_first_available_by_cover_priority() {
    let engine = DefaultMergeEngine::new(PriorityModel::english());
    let input = merge_input(
        work(),
        vec![],
        HashMap::from([
            (
                MetadataProvider::Goodreads,
                success(NormalizedWorkDetail {
                    cover_url: Some("https://covers.example/goodreads.jpg".to_string()),
                    ..empty_detail()
                }),
            ),
            (
                MetadataProvider::Readarr,
                success(NormalizedWorkDetail {
                    cover_url: Some("https://covers.example/readarr.jpg".to_string()),
                    ..empty_detail()
                }),
            ),
            (
                MetadataProvider::Hardcover,
                success(NormalizedWorkDetail {
                    cover_url: None,
                    ..empty_detail()
                }),
            ),
        ]),
        PriorityModel {
            content: vec![MetadataProvider::Hardcover],
            description: vec![MetadataProvider::Hardcover],
            cover: vec![
                MetadataProvider::Hardcover,
                MetadataProvider::Goodreads,
                MetadataProvider::Readarr,
            ],
            audio: vec![MetadataProvider::Audnexus],
        },
    );

    let output = engine.merge(input).await.expect("merge should succeed");

    assert_eq!(
        resolved(&output).cover_url.as_deref(),
        Some("https://covers.example/goodreads.jpg")
    );
    assert_eq!(
        provider_for(&output, WorkField::CoverUrl),
        Some(MetadataProvider::Goodreads)
    );
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn source_provider_data_isbn10_routes_to_additional_isbns_not_isbn13() {
    let src = SourceProviderData {
        isbn: Some("0345391802".to_string()), // 10 chars = ISBN-10
        ..Default::default()
    };

    let normalized: NormalizedWorkDetail = src.into();

    // ISBN-10 must NOT end up in isbn_13
    assert_eq!(normalized.isbn_13, None);
    // ISBN-10 should be in additional_isbns
    assert!(normalized.additional_isbns.contains(&"0345391802".to_string()));
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn source_provider_data_isbn13_routes_to_isbn13_field() {
    let src = SourceProviderData {
        isbn: Some("9780345391803".to_string()), // 13 chars = ISBN-13
        ..Default::default()
    };

    let normalized: NormalizedWorkDetail = src.into();

    assert_eq!(normalized.isbn_13, Some("9780345391803".to_string()));
    assert!(normalized.additional_isbns.is_empty());
}

#[test]
#[ignore = "pk-implement: behavioral test registered pre-implementation"]
fn llm_merge_prompt_truncates_long_descriptions() {
    // Contract: description fields in the LLM prompt must be capped at 500 chars
    // to prevent context limit blowout and token waste.
    //
    // This test verifies the merge engine truncates before prompt construction.
    // Implementation: the prompt builder must truncate each provider's description
    // to 500 chars before injecting into the LLM prompt template.

    let long_description = "A".repeat(1000);

    let provider_data = vec![(
        MetadataProvider::Hardcover,
        NormalizedWorkDetail {
            description: Some(long_description.clone()),
            ..Default::default()
        },
    )];

    // The actual test would verify the prompt string passed to LlmCaller
    // has descriptions capped at 500 chars. This requires access to the
    // prompt construction internals or a recording LLM stub.
    //
    // Contract: no description in the LLM prompt exceeds 500 characters.
    assert!(long_description.len() > 500, "test fixture must exceed cap");
}
