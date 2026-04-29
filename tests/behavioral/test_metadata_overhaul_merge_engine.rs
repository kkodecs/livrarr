//! Behavioral contract tests for MergeEngine::merge covering priority resolution,
//! last-known-good preservation, provenance ownership semantics, conflict blocking,
//! merge status classification, external ID collection, and documented priority
//! model behavior for metadata enrichment.
#![allow(dead_code)]

use std::collections::HashMap;

use chrono::Utc;
use livrarr_db::SetFieldProvenanceRequest;
use livrarr_domain::{
    EnrichmentStatus, ExternalIdType, FieldProvenance, MetadataProvider as MetadataSource,
    NarrationType, OutcomeClass, ProvenanceSetter, UserId, Work, WorkField, WorkId,
};
use livrarr_metadata::{
    DefaultMergeEngine, EnrichmentMode, MergeEngine, MergeError, MergeInput, MergeOutput,
    NormalizedWorkDetail, PriorityModel, ReconstructedOutcome,
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

fn make_engine() -> Box<dyn MergeEngine> {
    Box::new(DefaultMergeEngine::new(default_priority_model()))
}

fn merge(engine: &(impl MergeEngine + ?Sized), input: MergeInput) -> MergeOutput {
    engine.merge(input).expect("merge should succeed")
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

fn has_external_id_update(output: &MergeOutput, id_type: ExternalIdType, id_value: &str) -> bool {
    output
        .external_id_updates
        .iter()
        .any(|req| req.id_type == id_type && req.id_value == id_value && req.work_id == WORK_ID)
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

#[test]
fn test_merge_engine_priority_first_non_none_provider_wins_custom_order() {
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(
        resolved(&output).subtitle.as_deref(),
        Some("goodreads subtitle")
    );
}

#[test]
fn test_merge_engine_last_known_good_preserves_current_value_when_no_provider_replacement() {
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("current description")
    );
}

#[test]
fn test_merge_engine_last_known_good_outputs_none_only_when_current_none_and_no_provider_value() {
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

    let output = merge(engine.as_ref(), input);

    assert!(resolved(&output).description.is_none());
}

#[test]
fn test_merge_engine_purity_same_inputs_same_observable_output() {
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

    let first = merge(engine.as_ref(), input.clone());
    let second = merge(engine.as_ref(), input);

    assert_eq!(first.conflict_detected, second.conflict_detected);
    assert_eq!(first.enrichment_status, second.enrichment_status);
    assert_eq!(resolved(&first).subtitle, resolved(&second).subtitle);
    assert_eq!(resolved(&first).description, resolved(&second).description);
    assert_eq!(resolved(&first).cover_url, resolved(&second).cover_url);
    assert_eq!(first.provenance_deletes, second.provenance_deletes);
    assert_eq!(
        first.external_id_updates.len(),
        second.external_id_updates.len()
    );
    assert_eq!(first.provenance_upserts.len(), 1);
    assert_eq!(second.provenance_upserts.len(), 1);
    assert_eq!(
        upsert_signature(&first.provenance_upserts[0]),
        upsert_signature(&second.provenance_upserts[0])
    );
}

#[test]
fn test_merge_engine_user_owned_field_skips_provider_replacement() {
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("user description")
    );
    assert_no_field_mutation(&output, WorkField::Description);
}

#[test]
fn test_merge_engine_user_cleared_sticky_empty_skips_provider_replacement() {
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

    let output = merge(engine.as_ref(), input);

    assert!(resolved(&output).description.is_none());
    assert_no_field_mutation(&output, WorkField::Description);
}

#[test]
fn test_merge_engine_provider_owned_field_is_replaced_by_priority_model() {
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(resolved(&output).subtitle.as_deref(), Some("new subtitle"));
}

#[test]
fn test_merge_engine_conflict_blocks_all_mutations() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: any conflict blocks field writes and clears all mutation collections
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

    let output = merge(engine.as_ref(), input);

    assert!(output.conflict_detected);
    assert!(output.work_update.is_none());
    assert!(output.provenance_upserts.is_empty());
    assert!(output.provenance_deletes.is_empty());
    assert!(output.external_id_updates.is_empty());
}

#[test]
fn test_merge_engine_conflict_sets_status_conflict() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: conflict detection sets enrichment status to Conflict
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(output.enrichment_status, EnrichmentStatus::Conflict);
}

#[test]
fn test_merge_engine_status_enriched_when_description_and_cover_present() {
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(output.enrichment_status, EnrichmentStatus::Enriched);
}

#[test]
fn test_merge_engine_status_partial_when_only_one_of_description_or_cover_is_present() {
    // REQ-ID: R-02, R-14 | Contract: MergeEngine::merge | Behavior: status is Partial when exactly one of description or cover_url is present
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(output.enrichment_status, EnrichmentStatus::Partial);
}

#[test]
fn test_merge_engine_status_failed_when_neither_description_nor_cover_is_present() {
    // REQ-ID: R-02, R-14 | Contract: MergeEngine::merge | Behavior: status is Failed when neither description nor cover_url is present
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(output.enrichment_status, EnrichmentStatus::Failed);
}

#[test]
fn test_merge_engine_successful_provider_field_produces_provenance_upsert() {
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

    let output = merge(engine.as_ref(), input);
    let upsert = provenance_upsert(&output, WorkField::Description)
        .expect("expected provenance upsert for description");

    assert_eq!(upsert.user_id, USER_ID);
    assert_eq!(upsert.work_id, WORK_ID);
    assert_eq!(upsert.field, WorkField::Description);
    assert_eq!(upsert.source, Some(MetadataSource::Hardcover));
    assert_eq!(upsert.setter, ProvenanceSetter::Provider);
    assert!(!upsert.cleared);
}

#[test]
fn test_merge_engine_provider_owned_field_without_replacement_produces_provenance_delete() {
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("existing provider description")
    );
    assert!(has_provenance_delete(&output, WorkField::Description));
}

#[test]
fn test_merge_engine_success_provider_additional_ids_produce_external_id_updates() {
    // REQ-ID: R-02, R-06 | Contract: MergeEngine::merge | Behavior: success-provider additional ISBNs and ASINs are emitted as external ID updates
    let engine = make_engine();

    let input = MergeInput {
        current_work: work_with(None, Some("current description"), Some("current cover")),
        current_provenance: vec![],
        provider_results: HashMap::from([
            (
                MetadataSource::Goodreads,
                success(NormalizedWorkDetail {
                    additional_isbns: vec!["9781234567890".to_string()],
                    additional_asins: vec!["B00TEST123".to_string()],
                    ..empty_detail()
                }),
            ),
            (
                MetadataSource::Hardcover,
                success(NormalizedWorkDetail {
                    additional_isbns: vec!["9781111111111".to_string()],
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

    let output = merge(engine.as_ref(), input);

    assert!(has_external_id_update(
        &output,
        ExternalIdType::Isbn13,
        "9781234567890"
    ));
    assert!(has_external_id_update(
        &output,
        ExternalIdType::Isbn13,
        "9781111111111"
    ));
    assert!(has_external_id_update(
        &output,
        ExternalIdType::Asin,
        "B00TEST123"
    ));
}

#[test]
fn test_merge_engine_hard_refresh_replaces_provider_owned_populated_field() {
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(
        resolved(&output).subtitle.as_deref(),
        Some("hard refreshed subtitle")
    );
}

#[test]
fn test_merge_engine_manual_mode_preserves_last_known_good_for_will_retry() {
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

    let output = merge(engine.as_ref(), input);

    assert!(!output.conflict_detected);
    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("current description")
    );
}

#[test]
fn test_merge_engine_manual_mode_preserves_last_known_good_for_suppressed() {
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

    let output = merge(engine.as_ref(), input);

    assert!(!output.conflict_detected);
    assert_eq!(
        resolved(&output).cover_url.as_deref(),
        Some("current cover")
    );
}

#[test]
fn test_merge_engine_hard_refresh_preserves_last_known_good_for_will_retry() {
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

    let output = merge(engine.as_ref(), input);

    assert!(!output.conflict_detected);
    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("current description")
    );
}

#[test]
fn test_merge_engine_hard_refresh_suppressed_coercion_is_observable() {
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

    let output = merge(engine.as_ref(), input);

    assert!(!output.conflict_detected);
    assert!(
        output.work_update.is_some(),
        "Suppressed must be coerced in HardRefresh mode"
    );
}

#[test]
fn test_merge_engine_hard_refresh_suppressed_preserves_last_known_good_value() {
    // REQ-ID: R-02, R-18 | Contract: MergeEngine::merge | Behavior: hard refresh preserves the last-known-good populated field value for a coerced Suppressed outcome
    let engine = make_engine();

    let input = MergeInput {
        current_work: Work {
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

    let output = merge(engine.as_ref(), input);

    assert!(!output.conflict_detected);
    assert_eq!(resolved(&output).title.as_deref(), Some("current title"));
}

#[test]
fn test_merge_engine_english_priority_model_uses_documented_provider_order() {
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(resolved(&output).subtitle.as_deref(), Some("hc content"));
    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("ol description")
    );
    assert_eq!(
        resolved(&output).cover_url.as_deref(),
        Some("https://example.test/hc-cover.jpg")
    );
}

#[test]
fn test_merge_engine_foreign_priority_model_uses_gr_only() {
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(resolved(&output).subtitle.as_deref(), Some("gr subtitle"));
    assert_eq!(
        resolved(&output).description.as_deref(),
        Some("gr description")
    );
    assert_eq!(
        resolved(&output).cover_url.as_deref(),
        Some("https://example.test/gr-cover.jpg")
    );
}

#[test]
fn test_merge_engine_whitespace_only_high_priority_value_does_not_block_fallback() {
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(resolved(&output).subtitle.as_deref(), Some("Valid"));
    let upsert = provenance_upsert(&output, WorkField::Subtitle)
        .expect("expected provenance upsert for subtitle");
    assert_eq!(upsert.source, Some(MetadataSource::Goodreads));
}

#[test]
fn test_merge_engine_audio_fields_use_audio_priority_model_not_content_priority() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: narrator, duration_seconds, asin, and narration_type are resolved from PriorityModel.audio rather than PriorityModel.content
    let engine = make_engine();

    let input = MergeInput {
        current_work: Work {
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(
        resolved(&output).narrator.as_ref(),
        Some(&vec!["Audio Narrator".to_string()])
    );
    assert_eq!(resolved(&output).duration_seconds, Some(2222));
    assert_eq!(resolved(&output).asin.as_deref(), Some("AUDIOASIN2"));
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

    let asin_upsert =
        provenance_upsert(&output, WorkField::Asin).expect("expected provenance upsert for asin");
    assert_eq!(asin_upsert.source, Some(MetadataSource::Audnexus));

    let narration_type_upsert = provenance_upsert(&output, WorkField::NarrationType)
        .expect("expected provenance upsert for narration_type");
    assert_eq!(narration_type_upsert.source, Some(MetadataSource::Audnexus));
}

#[test]
fn test_merge_engine_cover_manual_bypasses_provider_cover_logic() {
    // REQ-ID: R-02, R-18 | Contract: MergeEngine::merge | Behavior: when cover_manual is true provider cover values are ignored, existing cover is preserved, and cover provenance is unchanged
    let engine = make_engine();

    let input = MergeInput {
        current_work: Work {
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

    let output = merge(engine.as_ref(), input);

    assert_eq!(
        resolved(&output).cover_url.as_deref(),
        Some("https://example.test/manual-cover.jpg")
    );
    assert_no_field_mutation(&output, WorkField::CoverUrl);
}

#[test]
fn test_merge_engine_empty_priority_model_returns_error() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: merge fails when the required priority list for a field category is empty
    let engine = make_engine();

    let result = engine.merge(MergeInput {
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
    });

    assert!(matches!(result, Err(MergeError::EmptyPriorityModel)));
}

#[test]
fn test_merge_engine_empty_description_priority_model_returns_error_for_description_field() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: merge fails with EmptyPriorityModel when description priority is empty for a description merge decision
    let engine = make_engine();

    let result = engine.merge(MergeInput {
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
    });

    assert!(matches!(result, Err(MergeError::EmptyPriorityModel)));
}

#[test]
fn test_merge_engine_empty_audio_priority_model_returns_error_for_audio_field() {
    // REQ-ID: R-02 | Contract: MergeEngine::merge | Behavior: merge fails with EmptyPriorityModel when audio priority is empty for an audio-field merge decision
    let engine = make_engine();

    let result = engine.merge(MergeInput {
        current_work: Work {
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
    });

    assert!(matches!(result, Err(MergeError::EmptyPriorityModel)));
}
