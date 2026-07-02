//! Behavioral contract tests for MergeEngine::merge covering priority resolution,
//! last-known-good preservation, provenance ownership semantics, conflict
//! dissent-isolation (REQ-014), merge status classification, and documented
//! priority model behavior for metadata enrichment.
#![allow(dead_code)]

use std::collections::HashMap;

use chrono::Utc;
use livrarr_db::SetFieldProvenanceRequest;
use livrarr_domain::{
    EnrichmentStatus, FieldProvenance, MetadataProvider as MetadataSource, NarrationType,
    OutcomeClass, ProvenanceSetter, UserId, Work, WorkField, WorkId,
};
use livrarr_external_data::NormalizedWorkDetail;
use livrarr_metadata::{
    DefaultMergeEngine, EnrichmentMode, MergeEngine, MergeError, MergeInput, MergeOutput,
    PriorityModel, ReconstructedOutcome,
};

const USER_ID: UserId = 7;
const WORK_ID: WorkId = 41;

fn default_priority_model() -> PriorityModel {
    PriorityModel {
        content: vec![MetadataSource::Hardcover, MetadataSource::OpenLibrary],
        description: vec![MetadataSource::Hardcover, MetadataSource::OpenLibrary],
        cover: vec![MetadataSource::Hardcover, MetadataSource::OpenLibrary],
        audio: vec![MetadataSource::Hardcover],
    }
}

fn make_engine() -> DefaultMergeEngine {
    DefaultMergeEngine::new(default_priority_model())
}

async fn merge(engine: &impl MergeEngine, input: MergeInput) -> MergeOutput {
    engine.merge(input).await.expect("merge should succeed")
}

fn resolved(output: &MergeOutput) -> &livrarr_db::UpdateWorkEnrichmentDbRequest {
    output
        .work_update
        .as_ref()
        .expect("expected work_update for non-conflict merge")
        .as_inner()
}

fn work_with(subtitle: Option<&str>, description: Option<&str>, cover_url: Option<&str>) -> Work {
    Work {
        identity_status: Default::default(),
        id: WORK_ID,
        user_id: USER_ID,
        subtitle: subtitle.map(str::to_owned),
        description: description.map(str::to_owned),
        cover_url: cover_url.map(str::to_owned),
        ..Default::default()
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

fn outcome(class: OutcomeClass) -> ReconstructedOutcome {
    ReconstructedOutcome {
        class,
        payload: None,
    }
}

fn custom_priority(
    content: Vec<MetadataSource>,
    description: Vec<MetadataSource>,
    cover: Vec<MetadataSource>,
) -> PriorityModel {
    PriorityModel {
        content,
        description,
        cover,
        audio: vec![MetadataSource::Audnexus],
    }
}

fn provenance(
    field: WorkField,
    setter: ProvenanceSetter,
    cleared: bool,
    source: Option<MetadataSource>,
) -> FieldProvenance {
    FieldProvenance {
        user_id: USER_ID,
        work_id: WORK_ID,
        field,
        source,
        set_at: Utc::now(),
        setter,
        cleared,
    }
}

fn user_owned(field: WorkField) -> FieldProvenance {
    provenance(field, ProvenanceSetter::User, false, None)
}

fn user_cleared(field: WorkField) -> FieldProvenance {
    provenance(field, ProvenanceSetter::User, true, None)
}

fn provider_owned(field: WorkField, source: MetadataSource) -> FieldProvenance {
    provenance(field, ProvenanceSetter::Provider, false, Some(source))
}

fn provenance_upsert(output: &MergeOutput, field: WorkField) -> Option<&SetFieldProvenanceRequest> {
    output
        .provenance_upserts
        .iter()
        .find(|req| req.field == field)
}

fn has_provenance_delete(output: &MergeOutput, field: WorkField) -> bool {
    output.provenance_deletes.contains(&field)
}

fn assert_no_field_mutation(output: &MergeOutput, field: WorkField) {
    assert!(
        provenance_upsert(output, field).is_none(),
        "field {field:?} should not receive a provenance upsert"
    );
    assert!(
        !has_provenance_delete(output, field),
        "field {field:?} should not receive a provenance delete"
    );
}

fn upsert_signature(
    req: &SetFieldProvenanceRequest,
) -> (
    UserId,
    WorkId,
    WorkField,
    Option<MetadataSource>,
    ProvenanceSetter,
    bool,
) {
    (
        req.user_id,
        req.work_id,
        req.field,
        req.source,
        req.setter,
        req.cleared,
    )
}

#[tokio::test]
async fn test_merge_engine_priority_first_non_none_provider_wins_custom_order() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: first non-None provider in priority order wins for a content field
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(
            Some("current subtitle"),
            Some("current description"),
            Some("current cover"),
        ),
        current_provenance: vec![],
        provider_results: HashMap::from([
            (
                MetadataSource::OpenLibrary,
                success(NormalizedWorkDetail {
                    subtitle: None,
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::Goodreads,
                success(NormalizedWorkDetail {
                    subtitle: Some("goodreads subtitle".to_string()),
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::Hardcover,
                success(NormalizedWorkDetail {
                    subtitle: Some("hardcover subtitle".to_string()),
                    ..empty_detail()
                }),
            ),
        ]),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![
                MetadataSource::OpenLibrary,
                MetadataSource::Goodreads,
                MetadataSource::Hardcover,
            ],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).subtitle.as_deref(),
        Some("goodreads subtitle")
    );
}

#[tokio::test]
async fn test_merge_engine_last_known_good_preserves_current_value_when_no_provider_replacement() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: preserves the current field value when no provider has a replacement
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, Some("current description"), Some("current cover")),
        current_provenance: vec![],
        provider_results: HashMap::from([(
            MetadataSource::Hardcover,
            success(NormalizedWorkDetail {
                description: None,
                ..empty_detail()
            }),
        )]),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![
                MetadataSource::Hardcover,
                MetadataSource::OpenLibrary,
                MetadataSource::Goodreads,
            ],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("current description")
    );
}

#[tokio::test]
async fn test_merge_engine_last_known_good_outputs_none_only_when_current_none_and_no_provider_value(
) {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: outputs None only when the current field is already None and providers have no value
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, None, Some("current cover")),
        current_provenance: vec![],
        provider_results: HashMap::new(),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert!(resolved(&output).description.is_none());
}

#[tokio::test]
async fn test_merge_engine_purity_same_inputs_same_observable_output() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: same input produces the same observable output on repeated calls
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, None, Some("current cover")),
        current_provenance: vec![],
        provider_results: HashMap::from([(
            MetadataSource::Hardcover,
            success(NormalizedWorkDetail {
                description: Some("provider description".to_string()),
                ..empty_detail()
            }),
        )]),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let first = merge(&engine, input.clone()).await;
    let second = merge(&engine, input).await;

    assert_eq!(first.enrichment_status, second.enrichment_status);
    assert_eq!(resolved(&first).subtitle, resolved(&second).subtitle);
    assert_eq!(resolved(&first).description, resolved(&second).description);
    assert_eq!(resolved(&first).cover_url, resolved(&second).cover_url);
    assert_eq!(first.provenance_deletes, second.provenance_deletes);
    assert_eq!(first.provenance_upserts.len(), 1);
    assert_eq!(second.provenance_upserts.len(), 1);
    assert_eq!(
        upsert_signature(&first.provenance_upserts[0]),
        upsert_signature(&second.provenance_upserts[0])
    );
}

#[tokio::test]
async fn test_merge_engine_user_owned_field_skips_provider_replacement() {
    // REQ-ID: R-02, R-18 | Contract: MergeEngine::merge | Behavior: user-owned fields are skipped even when providers supply data
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, Some("user description"), Some("current cover")),
        current_provenance: vec![user_owned(WorkField::Description)],
        provider_results: HashMap::from([(
            MetadataSource::Hardcover,
            success(NormalizedWorkDetail {
                description: Some("provider description".to_string()),
                ..empty_detail()
            }),
        )]),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("user description")
    );
    assert_no_field_mutation(&output, WorkField::Description);
}

#[tokio::test]
async fn test_merge_engine_user_cleared_sticky_empty_skips_provider_replacement() {
    // REQ-ID: R-02, R-18 | Contract: MergeEngine::merge | Behavior: user-cleared sticky empty fields are preserved and skipped
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, None, Some("current cover")),
        current_provenance: vec![user_cleared(WorkField::Description)],
        provider_results: HashMap::from([(
            MetadataSource::Hardcover,
            success(NormalizedWorkDetail {
                description: Some("provider description".to_string()),
                ..empty_detail()
            }),
        )]),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert!(resolved(&output).description.is_none());
    assert_no_field_mutation(&output, WorkField::Description);
}

#[tokio::test]
async fn test_merge_engine_provider_owned_field_is_replaced_by_priority_model() {
    // REQ-ID: R-02, R-18 | Contract: MergeEngine::merge | Behavior: hard refresh allows populated provider-owned fields to be replaced by priority order
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(
            Some("old subtitle"),
            Some("current description"),
            Some("current cover"),
        ),
        current_provenance: vec![provider_owned(
            WorkField::Subtitle,
            MetadataSource::Hardcover,
        )],
        provider_results: HashMap::from([
            (
                MetadataSource::Goodreads,
                success(NormalizedWorkDetail {
                    subtitle: Some("new subtitle".to_string()),
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::Hardcover,
                success(NormalizedWorkDetail {
                    subtitle: Some("stale subtitle".to_string()),
                    ..empty_detail()
                }),
            ),
        ]),
        mode: EnrichmentMode::HardRefresh,
        priority_model: custom_priority(
            vec![MetadataSource::Goodreads, MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(resolved(&output).subtitle.as_deref(), Some("new subtitle"));
}

#[tokio::test]
async fn test_merge_engine_conflict_is_isolated_and_clean_providers_merge() {
    // REQ-014 (metadata-correctness) | Contract: MergeEngine::merge | Behavior: a
    // Conflict provider is excluded (dissent-isolated); clean providers'
    // contributions merge normally — the old whole-merge block is retired.
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, Some("current description"), Some("current cover")),
        current_provenance: vec![],
        provider_results: HashMap::from([
            (MetadataSource::Goodreads, outcome(OutcomeClass::Conflict)),
            (
                MetadataSource::Hardcover,
                success(NormalizedWorkDetail {
                    description: Some("provider description".to_string()),
                    ..empty_detail()
                }),
            ),
        ]),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("provider description"),
        "REQ-014: a dissenting provider never blocks the clean contributions"
    );
    assert!(provenance_upsert(&output, WorkField::Description).is_some());
}

#[tokio::test]
async fn test_merge_engine_conflict_alone_writes_no_fields() {
    // REQ-014 (metadata-correctness) | Contract: MergeEngine::merge | Behavior: a
    // sole Conflict provider contributes nothing — no field writes, no
    // provenance. The Unenriched-on-conflict sentinel is retired; identity
    // conflicts live in IdentityStatus, not in enrichment status.
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, Some("current description"), Some("current cover")),
        current_provenance: vec![],
        provider_results: HashMap::from([(
            MetadataSource::Goodreads,
            outcome(OutcomeClass::Conflict),
        )]),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert!(
        output.work_update.is_none(),
        "REQ-014: zero surviving contributions → nothing to write (status-only apply)"
    );
    assert!(output.provenance_upserts.is_empty());
}

#[tokio::test]
async fn test_merge_engine_status_enriched_when_description_and_cover_present() {
    // REQ-ID: R-02, R-14 | Contract: MergeEngine::merge | Behavior: status is Enriched when merged output has both description and cover_url
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, Some("preserved description"), None),
        current_provenance: vec![],
        provider_results: HashMap::from([(
            MetadataSource::Goodreads,
            success(NormalizedWorkDetail {
                cover_url: Some("https://example.test/gr-cover.jpg".to_string()),
                ..empty_detail()
            }),
        )]),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(output.enrichment_status, EnrichmentStatus::Enriched);
}

#[tokio::test]
async fn test_merge_engine_status_enriched_when_description_present_without_cover() {
    // REQ-ID: REQ-019 | Contract: MergeEngine::merge | Behavior: description present (no cover) classifies Enriched — cover never gates
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, Some("description only"), None),
        current_provenance: vec![],
        provider_results: HashMap::new(),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(output.enrichment_status, EnrichmentStatus::Enriched);
    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("description only"),
        "description metadata should be preserved when it classifies the merge as Enriched"
    );
    assert!(
        resolved(&output).cover_url.is_none(),
        "missing counterpart metadata should remain absent rather than being fabricated"
    );
}

#[tokio::test]
async fn test_merge_engine_status_thin_when_no_meaningful_text_present() {
    // REQ-ID: REQ-019/REQ-014 | Contract: MergeEngine::merge | Behavior: a successful textless merge is Thin, not Failed
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, None, None),
        current_provenance: vec![],
        provider_results: HashMap::new(),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(output.enrichment_status, EnrichmentStatus::Thin);
}

#[tokio::test]
async fn test_merge_engine_successful_provider_field_produces_provenance_upsert() {
    // REQ-ID: R-02, R-18 | Contract: MergeEngine::merge | Behavior: merged provider-owned field values produce provenance upsert entries
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, None, Some("current cover")),
        current_provenance: vec![],
        provider_results: HashMap::from([(
            MetadataSource::Hardcover,
            success(NormalizedWorkDetail {
                description: Some("hardcover description".to_string()),
                ..empty_detail()
            }),
        )]),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;
    let upsert = provenance_upsert(&output, WorkField::Description)
        .expect("expected provenance upsert for description");

    assert_eq!(upsert.user_id, USER_ID);
    assert_eq!(upsert.work_id, WORK_ID);
    assert_eq!(upsert.field, WorkField::Description);
    assert_eq!(upsert.source, Some(MetadataSource::Hardcover));
    assert_eq!(upsert.setter, ProvenanceSetter::Provider);
    assert!(!upsert.cleared);
}

#[tokio::test]
async fn test_merge_engine_provider_owned_field_without_replacement_produces_provenance_delete() {
    // REQ-ID: R-02, R-18 | Contract: MergeEngine::merge | Behavior: provider-owned fields with no replacement preserve value and produce provenance delete entries
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(
            None,
            Some("existing provider description"),
            Some("current cover"),
        ),
        current_provenance: vec![provider_owned(
            WorkField::Description,
            MetadataSource::Hardcover,
        )],
        provider_results: HashMap::new(),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("existing provider description")
    );
    assert!(has_provenance_delete(&output, WorkField::Description));
}

#[tokio::test]
async fn test_merge_engine_hard_refresh_replaces_provider_owned_populated_field() {
    // REQ-ID: R-02, R-18 | Contract: MergeEngine::merge | Behavior: hard refresh treats provider-owned populated fields as replaceable candidates
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(
            Some("provider managed subtitle"),
            Some("current description"),
            Some("current cover"),
        ),
        current_provenance: vec![provider_owned(
            WorkField::Subtitle,
            MetadataSource::Hardcover,
        )],
        provider_results: HashMap::from([(
            MetadataSource::Hardcover,
            success(NormalizedWorkDetail {
                subtitle: Some("hard refreshed subtitle".to_string()),
                ..empty_detail()
            }),
        )]),
        mode: EnrichmentMode::HardRefresh,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).subtitle.as_deref(),
        Some("hard refreshed subtitle")
    );
}

#[tokio::test]
async fn test_merge_engine_manual_mode_preserves_last_known_good_for_will_retry() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: manual mode coerces WillRetry to merge-eligible while preserving last-known-good
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, Some("current description"), Some("current cover")),
        current_provenance: vec![],
        provider_results: HashMap::from([(
            MetadataSource::Hardcover,
            outcome(OutcomeClass::WillRetry),
        )]),
        mode: EnrichmentMode::Manual,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("current description")
    );
}

#[tokio::test]
async fn test_merge_engine_manual_mode_preserves_last_known_good_for_suppressed() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: manual mode coerces Suppressed to merge-eligible while preserving last-known-good
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, Some("current description"), Some("current cover")),
        current_provenance: vec![],
        provider_results: HashMap::from([(
            MetadataSource::Goodreads,
            outcome(OutcomeClass::Suppressed),
        )]),
        mode: EnrichmentMode::Manual,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).cover_url.as_deref(),
        Some("current cover")
    );
}

#[tokio::test]
async fn test_merge_engine_hard_refresh_preserves_last_known_good_for_will_retry() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: hard refresh coerces WillRetry to merge-eligible while preserving the current last-known-good field value
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(
            Some("current subtitle"),
            Some("current description"),
            Some("current cover"),
        ),
        current_provenance: vec![provider_owned(
            WorkField::Description,
            MetadataSource::Hardcover,
        )],
        provider_results: HashMap::from([(
            MetadataSource::Hardcover,
            outcome(OutcomeClass::WillRetry),
        )]),
        mode: EnrichmentMode::HardRefresh,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("current description")
    );
}

#[tokio::test]
async fn test_merge_engine_hard_refresh_suppressed_coercion_is_observable() {
    // REQ-ID: R-02, R-18 | Contract: MergeEngine::merge | Behavior: hard refresh coerces Suppressed to merge-eligible. HardRefresh coercion: Suppressed→merge_eligible=true. Observable via work_update.is_some()
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(
            Some("current subtitle"),
            Some("current description"),
            Some("current cover"),
        ),
        current_provenance: vec![provider_owned(
            WorkField::Description,
            MetadataSource::Hardcover,
        )],
        provider_results: HashMap::from([(
            MetadataSource::Hardcover,
            outcome(OutcomeClass::Suppressed),
        )]),
        mode: EnrichmentMode::HardRefresh,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert!(
        output.work_update.is_some(),
        "Suppressed must be coerced in HardRefresh mode"
    );
}

#[tokio::test]
async fn test_merge_engine_hard_refresh_suppressed_preserves_last_known_good_value() {
    // REQ-ID: R-02, R-18 | Contract: MergeEngine::merge | Behavior: hard refresh preserves the last-known-good populated field value for a coerced Suppressed outcome
    let engine = make_engine();

    let input = MergeInput {
        current_work: Work {
            identity_status: Default::default(),
            id: WORK_ID,
            user_id: USER_ID,
            title: "current title".to_string(),
            subtitle: Some("current subtitle".to_string()),
            description: Some("current description".to_string()),
            cover_url: Some("current cover".to_string()),
            ..Default::default()
        },
        current_provenance: vec![provider_owned(WorkField::Title, MetadataSource::Hardcover)],
        provider_results: HashMap::from([(
            MetadataSource::Hardcover,
            outcome(OutcomeClass::Suppressed),
        )]),
        mode: EnrichmentMode::HardRefresh,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(resolved(&output).title.as_deref(), Some("current title"));
}

#[tokio::test]
async fn test_merge_engine_english_priority_model_uses_documented_provider_order() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: English priority model uses HC→GR→OL for content, HC→OL→GR for description, and HC→GR→OL for cover
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, None, None),
        current_provenance: vec![],
        provider_results: HashMap::from([
            (
                MetadataSource::Hardcover,
                success(NormalizedWorkDetail {
                    subtitle: Some("hc content".to_string()),
                    cover_url: Some("https://example.test/hc-cover.jpg".to_string()),
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::Goodreads,
                success(NormalizedWorkDetail {
                    subtitle: Some("gr content".to_string()),
                    description: Some("gr description".to_string()),
                    cover_url: Some("https://example.test/gr-cover.jpg".to_string()),
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::OpenLibrary,
                success(NormalizedWorkDetail {
                    subtitle: Some("ol content".to_string()),
                    description: Some("ol description".to_string()),
                    cover_url: Some("https://example.test/ol-cover.jpg".to_string()),
                    ..empty_detail()
                }),
            ),
        ]),
        mode: EnrichmentMode::Background,
        priority_model: PriorityModel::english(),
    };

    let output = merge(&engine, input).await;

    assert_eq!(resolved(&output).subtitle.as_deref(), Some("hc content"));
    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("gr description")
    );
    let cover = output
        .cover_resolution
        .as_ref()
        .expect("should resolve a cover");
    assert_eq!(cover.url, "https://example.test/hc-cover.jpg");
}

#[tokio::test]
async fn test_merge_engine_foreign_priority_model_uses_gr_only() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: foreign priority model uses GR-only; OL excluded
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, None, None),
        current_provenance: vec![],
        provider_results: HashMap::from([
            (
                MetadataSource::Goodreads,
                success(NormalizedWorkDetail {
                    subtitle: Some("gr subtitle".to_string()),
                    description: Some("gr description".to_string()),
                    cover_url: Some("https://example.test/gr-cover.jpg".to_string()),
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::OpenLibrary,
                success(NormalizedWorkDetail {
                    subtitle: Some("ol subtitle".to_string()),
                    description: Some("ol description".to_string()),
                    cover_url: Some("https://example.test/ol-cover.jpg".to_string()),
                    ..empty_detail()
                }),
            ),
        ]),
        mode: EnrichmentMode::Background,
        priority_model: PriorityModel::foreign(),
    };

    let output = merge(&engine, input).await;

    assert_eq!(resolved(&output).subtitle.as_deref(), Some("gr subtitle"));
    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("gr description")
    );
    let cover = output
        .cover_resolution
        .as_ref()
        .expect("should resolve a cover");
    assert_eq!(cover.url, "https://example.test/gr-cover.jpg");
}

#[tokio::test]
async fn test_merge_engine_whitespace_only_high_priority_value_does_not_block_fallback() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: empty and whitespace-only strings are treated as no value so a lower-priority valid string wins
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, Some("current description"), Some("current cover")),
        current_provenance: vec![],
        provider_results: HashMap::from([
            (
                MetadataSource::Hardcover,
                success(NormalizedWorkDetail {
                    subtitle: Some("   ".to_string()),
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::Goodreads,
                success(NormalizedWorkDetail {
                    subtitle: Some("Valid".to_string()),
                    ..empty_detail()
                }),
            ),
        ]),
        mode: EnrichmentMode::Background,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover, MetadataSource::Goodreads],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(resolved(&output).subtitle.as_deref(), Some("Valid"));
    let upsert = provenance_upsert(&output, WorkField::Subtitle)
        .expect("expected provenance upsert for subtitle");
    assert_eq!(upsert.source, Some(MetadataSource::Goodreads));
}

#[tokio::test]
async fn test_merge_engine_empty_genres_high_priority_does_not_block_fallback() {
    // REQ-ID: M-013 | Contract: MergeEngine::merge | Behavior: an empty genres list from a high-priority provider is not a value, so a lower-priority provider's real genres win
    let engine = make_engine();

    let input = MergeInput {
        current_work: Work {
            identity_status: Default::default(),
            id: WORK_ID,
            user_id: USER_ID,
            language: Some("en".to_string()),
            ..Default::default()
        },
        current_provenance: vec![],
        provider_results: HashMap::from([
            (
                MetadataSource::Hardcover,
                success(NormalizedWorkDetail {
                    genres: Some(vec![]),
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::Goodreads,
                success(NormalizedWorkDetail {
                    genres: Some(vec!["Fantasy".to_string(), "Adventure".to_string()]),
                    ..empty_detail()
                }),
            ),
        ]),
        mode: EnrichmentMode::Background,
        priority_model: PriorityModel::english(),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).genres.as_ref(),
        Some(&vec!["Fantasy".to_string(), "Adventure".to_string()])
    );
    let upsert = provenance_upsert(&output, WorkField::Genres)
        .expect("expected provenance upsert for genres");
    assert_eq!(upsert.source, Some(MetadataSource::Goodreads));
}

#[tokio::test]
async fn test_merge_engine_all_empty_genres_retains_current_genres() {
    // REQ-ID: M-013 | Contract: MergeEngine::merge | Behavior: when every provider offers an empty genres list, the current stored genres are retained (last-known-good; no erasure)
    let engine = make_engine();

    let input = MergeInput {
        current_work: Work {
            identity_status: Default::default(),
            id: WORK_ID,
            user_id: USER_ID,
            genres: Some(vec!["Mystery".to_string()]),
            ..Default::default()
        },
        current_provenance: vec![],
        provider_results: HashMap::from([
            (
                MetadataSource::Hardcover,
                success(NormalizedWorkDetail {
                    genres: Some(vec![]),
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::Goodreads,
                success(NormalizedWorkDetail {
                    genres: Some(vec![]),
                    ..empty_detail()
                }),
            ),
        ]),
        mode: EnrichmentMode::Background,
        priority_model: PriorityModel::english(),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).genres.as_ref(),
        Some(&vec!["Mystery".to_string()])
    );
}

#[tokio::test]
async fn test_merge_engine_empty_narrator_does_not_block_fallback() {
    // REQ-ID: M-013 | Contract: MergeEngine::merge | Behavior: an empty narrator list from a high-priority audio provider is not a value, so a lower-priority provider's real narrator wins
    let engine = make_engine();

    let input = MergeInput {
        current_work: Work {
            identity_status: Default::default(),
            id: WORK_ID,
            user_id: USER_ID,
            ..Default::default()
        },
        current_provenance: vec![],
        provider_results: HashMap::from([
            (
                MetadataSource::Audible,
                success(NormalizedWorkDetail {
                    narrator: Some(vec![]),
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::Audnexus,
                success(NormalizedWorkDetail {
                    narrator: Some(vec!["Real Narrator".to_string()]),
                    ..empty_detail()
                }),
            ),
        ]),
        mode: EnrichmentMode::Background,
        priority_model: PriorityModel::english(),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).narrator.as_ref(),
        Some(&vec!["Real Narrator".to_string()])
    );
    let upsert = provenance_upsert(&output, WorkField::Narrator)
        .expect("expected provenance upsert for narrator");
    assert_eq!(upsert.source, Some(MetadataSource::Audnexus));
}

#[tokio::test]
async fn test_merge_engine_audio_fields_use_audio_priority_model_not_content_priority() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: narrator, duration_seconds, asin, and narration_type are resolved from PriorityModel.audio rather than PriorityModel.content
    let engine = make_engine();

    let input = MergeInput {
        current_work: Work {
            identity_status: Default::default(),
            id: WORK_ID,
            user_id: USER_ID,
            ..Default::default()
        },
        current_provenance: vec![],
        provider_results: HashMap::from([
            (
                MetadataSource::Hardcover,
                success(NormalizedWorkDetail {
                    narrator: Some(vec!["Content Narrator".to_string()]),
                    duration_seconds: Some(1111),
                    asin: Some("CONTENTASIN1".to_string()),
                    narration_type: Some(NarrationType::Abridged),
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::Audnexus,
                success(NormalizedWorkDetail {
                    narrator: Some(vec!["Audio Narrator".to_string()]),
                    duration_seconds: Some(2222),
                    asin: Some("AUDIOASIN2".to_string()),
                    narration_type: Some(NarrationType::Unabridged),
                    ..empty_detail()
                }),
            ),
        ]),
        mode: EnrichmentMode::Background,
        priority_model: PriorityModel {
            content: vec![MetadataSource::Hardcover, MetadataSource::Audnexus],
            description: vec![MetadataSource::Hardcover],
            cover: vec![MetadataSource::Goodreads],
            audio: vec![MetadataSource::Audnexus, MetadataSource::Hardcover],
        },
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).narrator.as_ref(),
        Some(&vec!["Audio Narrator".to_string()])
    );
    assert_eq!(resolved(&output).duration_seconds, Some(2222));
    assert_eq!(
        resolved(&output).narration_type,
        Some(NarrationType::Unabridged)
    );

    let narrator_upsert = provenance_upsert(&output, WorkField::Narrator)
        .expect("expected provenance upsert for narrator");
    assert_eq!(narrator_upsert.source, Some(MetadataSource::Audnexus));

    let duration_upsert = provenance_upsert(&output, WorkField::DurationSeconds)
        .expect("expected provenance upsert for duration_seconds");
    assert_eq!(duration_upsert.source, Some(MetadataSource::Audnexus));

    let narration_type_upsert = provenance_upsert(&output, WorkField::NarrationType)
        .expect("expected provenance upsert for narration_type");
    assert_eq!(narration_type_upsert.source, Some(MetadataSource::Audnexus));
}

#[tokio::test]
async fn test_merge_engine_cover_manual_no_longer_blocks_provider_cover() {
    // REQ-008 (metadata-refactor): cover_manual is no longer a cover lock — provenance
    // Setter=User is the single lock mechanism (metadata-refactor AC-005). The cover
    // provenance here is Provider-owned, so the highest-priority provider cover wins;
    // the legacy cover_manual bypass is removed. Cover selection generates no field
    // provenance (cover is resolved separately from MERGE_FIELDS).
    let engine = make_engine();

    let input = MergeInput {
        current_work: Work {
            identity_status: Default::default(),
            id: WORK_ID,
            user_id: USER_ID,
            cover_url: Some("https://example.test/manual-cover.jpg".to_string()),
            cover_manual: true,
            ..Default::default()
        },
        current_provenance: vec![provider_owned(
            WorkField::CoverUrl,
            MetadataSource::Goodreads,
        )],
        provider_results: HashMap::from([
            (
                MetadataSource::Goodreads,
                success(NormalizedWorkDetail {
                    cover_url: Some("https://example.test/gr-cover.jpg".to_string()),
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::Hardcover,
                success(NormalizedWorkDetail {
                    cover_url: Some("https://example.test/hc-cover.jpg".to_string()),
                    ..empty_detail()
                }),
            ),
        ]),
        mode: EnrichmentMode::HardRefresh,
        priority_model: custom_priority(
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Hardcover],
            vec![MetadataSource::Goodreads, MetadataSource::Hardcover],
        ),
    };

    let output = merge(&engine, input).await;

    assert_eq!(
        resolved(&output).cover_url.as_deref(),
        Some("https://example.test/gr-cover.jpg")
    );
    assert_no_field_mutation(&output, WorkField::CoverUrl);
}

#[tokio::test]
async fn test_merge_engine_empty_priority_model_returns_error() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: merge fails when the required priority list for a field category is empty
    let engine = make_engine();

    let result = engine
        .merge(MergeInput {
            current_work: work_with(None, Some("current description"), Some("current cover")),
            current_provenance: vec![],
            provider_results: HashMap::from([(
                MetadataSource::Goodreads,
                success(NormalizedWorkDetail {
                    subtitle: Some("provider subtitle".to_string()),
                    ..empty_detail()
                }),
            )]),
            mode: EnrichmentMode::Background,
            priority_model: PriorityModel {
                content: vec![],
                description: vec![MetadataSource::Hardcover],
                cover: vec![MetadataSource::Goodreads],
                audio: vec![MetadataSource::Audnexus],
            },
        })
        .await;

    assert!(matches!(result, Err(MergeError::EmptyPriorityModel)));
}

#[tokio::test]
async fn test_merge_engine_empty_description_priority_model_returns_error_for_description_field() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: merge fails with EmptyPriorityModel when description priority is empty for a description merge decision
    let engine = make_engine();

    let result = engine
        .merge(MergeInput {
            current_work: work_with(None, Some("current description"), Some("current cover")),
            current_provenance: vec![],
            provider_results: HashMap::from([(
                MetadataSource::Hardcover,
                success(NormalizedWorkDetail {
                    description: Some("provider description".to_string()),
                    ..empty_detail()
                }),
            )]),
            mode: EnrichmentMode::Background,
            priority_model: PriorityModel {
                content: vec![MetadataSource::Hardcover],
                description: vec![],
                cover: vec![MetadataSource::Goodreads],
                audio: vec![MetadataSource::Audnexus],
            },
        })
        .await;

    assert!(matches!(result, Err(MergeError::EmptyPriorityModel)));
}

#[tokio::test]
async fn test_merge_engine_empty_audio_priority_model_returns_error_for_audio_field() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: merge fails with EmptyPriorityModel when audio priority is empty for an audio-field merge decision
    let engine = make_engine();

    let result = engine
        .merge(MergeInput {
            current_work: Work {
                identity_status: Default::default(),
                id: WORK_ID,
                user_id: USER_ID,
                ..Default::default()
            },
            current_provenance: vec![],
            provider_results: HashMap::from([(
                MetadataSource::Audnexus,
                success(NormalizedWorkDetail {
                    narrator: Some(vec!["Audio Narrator".to_string()]),
                    ..empty_detail()
                }),
            )]),
            mode: EnrichmentMode::Background,
            priority_model: PriorityModel {
                content: vec![MetadataSource::Hardcover],
                description: vec![MetadataSource::Hardcover],
                cover: vec![MetadataSource::Goodreads],
                audio: vec![],
            },
        })
        .await;

    assert!(matches!(result, Err(MergeError::EmptyPriorityModel)));
}

/// REQ-IDs: REQ-027
/// #133 regression: the NETWORK merge path (`MergeEngine::merge` fed reconstructed
/// `provider_results`) must drop OpenLibrary/Hardcover English metadata for a
/// foreign work — not only the cached `merge_from_cached` path. Before the fix,
/// `PriorityModel::foreign()`'s fallback list let Hardcover win a field that the
/// language-compatible providers (GB/GR) left empty.
#[tokio::test]
async fn test_merge_network_path_foreign_drops_english_openlibrary_and_hardcover() {
    let engine = make_engine();

    let work = Work {
        identity_status: Default::default(),
        id: WORK_ID,
        user_id: USER_ID,
        title: "Le Petit Prince".to_string(),
        author_name: "Antoine de Saint-Exupéry".to_string(),
        language: Some("fr".to_string()),
        ..Default::default()
    };

    // Only the English OpenLibrary/Hardcover editions carry a description; the
    // language-compatible providers contributed nothing for this field, so on the
    // unpatched network path Hardcover would win it.
    let provider_results = HashMap::from([
        (
            MetadataSource::OpenLibrary,
            success(NormalizedWorkDetail {
                description: Some("English OpenLibrary description".to_string()),
                ..empty_detail()
            }),
        ),
        (
            MetadataSource::Hardcover,
            success(NormalizedWorkDetail {
                description: Some("English Hardcover description".to_string()),
                ..empty_detail()
            }),
        ),
    ]);

    let output = merge(
        &engine,
        MergeInput {
            current_work: work,
            current_provenance: vec![],
            provider_results,
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::foreign(),
        },
    )
    .await;

    assert!(
        output.work_update.is_none(),
        "foreign work must not take English OpenLibrary/Hardcover description on the network \
         path (#133/REQ-027); with zero surviving contributions there is nothing to write"
    );
    assert!(
        provenance_upsert(&output, WorkField::Description).is_none(),
        "no provider should win Description for a foreign work when only OL/HC supplied it"
    );
}

/// P2 (language is sovereign): a work's already-set language is identity-locked —
/// a provider returning a DIFFERENT language must never override it (only a user
/// changes language). Guards the "one language home" fix.
#[tokio::test]
async fn test_merge_language_is_identity_locked_provider_cannot_override() {
    let engine = make_engine();

    let work = Work {
        identity_status: Default::default(),
        id: WORK_ID,
        user_id: USER_ID,
        title: "Le Petit Prince".to_string(),
        author_name: "Antoine de Saint-Exupéry".to_string(),
        language: Some("fr".to_string()),
        ..Default::default()
    };

    // A provider reports a different language; it must not win.
    let provider_results = HashMap::from([(
        MetadataSource::GoogleBooks,
        success(NormalizedWorkDetail {
            description: Some("Descripción en español".to_string()),
            language: Some("es".to_string()),
            ..empty_detail()
        }),
    )]);

    let output = merge(
        &engine,
        MergeInput {
            current_work: work,
            current_provenance: vec![],
            provider_results,
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::foreign(),
        },
    )
    .await;

    let resolved = resolved(&output);
    assert_eq!(
        resolved.language.as_deref(),
        Some("fr"),
        "a set language must not be overridden by a provider (P2)"
    );
}

/// Fill-blank companion: when the work has NO language yet, a provider may supply
/// one — the lock only prevents overriding a set value, never filling a blank.
#[tokio::test]
async fn test_merge_language_fills_when_blank() {
    let engine = make_engine();

    let work = Work {
        identity_status: Default::default(),
        id: WORK_ID,
        user_id: USER_ID,
        title: "Some Book".to_string(),
        author_name: "An Author".to_string(),
        language: None,
        ..Default::default()
    };

    let provider_results = HashMap::from([(
        MetadataSource::GoogleBooks,
        success(NormalizedWorkDetail {
            description: Some("Une description".to_string()),
            language: Some("fr".to_string()),
            ..empty_detail()
        }),
    )]);

    let output = merge(
        &engine,
        MergeInput {
            current_work: work,
            current_provenance: vec![],
            provider_results,
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::foreign(),
        },
    )
    .await;

    let resolved = resolved(&output);
    assert_eq!(
        resolved.language.as_deref(),
        Some("fr"),
        "a provider may fill a blank language"
    );
}

/// REQ-ID: M-012 | Contract: MergeEngine::merge | Behavior: the Goodreads cover
/// gate (REQ-017) runs at the merge chokepoint for every caller — a Goodreads
/// cover whose title fails the deterministic Jaccard check must not win the
/// cover field even though Goodreads outranks OpenLibrary in the English cover
/// priority list; a Goodreads cover whose title clears the threshold may win.
#[tokio::test]
async fn test_merge_engine_gr_cover_gate_at_chokepoint() {
    let engine = make_engine();

    let base_work = Work {
        identity_status: Default::default(),
        id: WORK_ID,
        user_id: USER_ID,
        title: "The Name of the Wind".to_string(),
        author_name: "Patrick Rothfuss".to_string(),
        language: Some("en".to_string()),
        ol_key: Some("OL15832982W".to_string()),
        ..Default::default()
    };

    // Mismatched GR title: the gate strips the GR cover, so OpenLibrary's cover wins.
    let mismatched_output = merge(
        &engine,
        MergeInput {
            current_work: base_work.clone(),
            current_provenance: vec![],
            provider_results: HashMap::from([
                (
                    MetadataSource::Goodreads,
                    success(NormalizedWorkDetail {
                        title: Some("A Darkness at Sethanon".to_string()),
                        cover_url: Some("https://example.test/gr-cover.jpg".to_string()),
                        ..empty_detail()
                    }),
                ),
                (
                    MetadataSource::OpenLibrary,
                    success(NormalizedWorkDetail {
                        cover_url: Some("https://example.test/ol-cover.jpg".to_string()),
                        ..empty_detail()
                    }),
                ),
            ]),
            mode: EnrichmentMode::Background,
            priority_model: PriorityModel::english(),
        },
    )
    .await;

    let mismatched_cover = mismatched_output
        .cover_resolution
        .as_ref()
        .expect("should resolve a cover");
    assert_eq!(
        mismatched_cover.url, "https://example.test/ol-cover.jpg",
        "mismatched GR title must not win the cover gate"
    );

    // Matching GR title (Jaccard >= 0.6): the gate applies, GR outranks OL for cover.
    let matching_output = merge(
        &engine,
        MergeInput {
            current_work: base_work,
            current_provenance: vec![],
            provider_results: HashMap::from([
                (
                    MetadataSource::Goodreads,
                    success(NormalizedWorkDetail {
                        title: Some("The Name of the Wind".to_string()),
                        cover_url: Some("https://example.test/gr-cover.jpg".to_string()),
                        ..empty_detail()
                    }),
                ),
                (
                    MetadataSource::OpenLibrary,
                    success(NormalizedWorkDetail {
                        cover_url: Some("https://example.test/ol-cover.jpg".to_string()),
                        ..empty_detail()
                    }),
                ),
            ]),
            mode: EnrichmentMode::Background,
            priority_model: PriorityModel::english(),
        },
    )
    .await;

    let matching_cover = matching_output
        .cover_resolution
        .as_ref()
        .expect("should resolve a cover");
    assert_eq!(
        matching_cover.url, "https://example.test/gr-cover.jpg",
        "matching GR title should be allowed to win the cover gate"
    );
}
