//! Behavioral contract tests for metadata-overhaul domain-facing enums and wrappers.
//! Covers OutcomeClass, MergeResolved<T>, ExternalIdType, PermanentFailureReason,
//! EnrichmentMode, and ProviderOutcome<T> contracts.

#![allow(dead_code)]

use std::collections::HashSet;
use std::fmt::Debug;
use std::mem::discriminant;

use livrarr_domain::services::{CoverService, CoverServiceError};
use livrarr_domain::{
    CoverCandidate, CoverMediaType, CoverTrust, ExternalIdType, MergeResolved, MetadataProvider,
    OutcomeClass, PermanentFailureReason, ProvenanceSetter, UserId, WillRetryReason, WorkField,
    WorkId,
};
use livrarr_external_data::ProviderOutcome;
use livrarr_metadata::EnrichmentMode;
use serde::Serialize;

fn fixed_timestamp() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339("2026-04-15T12:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc)
}

fn assert_merge_resolved_new_roundtrips<T>(value: T)
where
    T: Clone + Debug + PartialEq,
{
    let wrapped = MergeResolved::new(value.clone());
    assert_eq!(wrapped.into_inner(), value);
}

fn assert_provider_outcome_class<T>(outcome: ProviderOutcome<T>, expected: OutcomeClass) {
    assert_eq!(outcome.class(), expected);
}

fn assert_provider_outcome_can_merge<T>(outcome: ProviderOutcome<T>, expected: bool) {
    assert_eq!(outcome.can_merge(), expected);
}

fn assert_provider_outcome_can_merge_manual<T>(outcome: ProviderOutcome<T>, expected: bool) {
    assert_eq!(outcome.can_merge_manual(), expected);
}

fn assert_serializes_to_json<T>(value: T, expected_json: &str)
where
    T: Serialize,
{
    let actual_json = serde_json::to_string(&value).expect("value must serialize");
    assert_eq!(actual_json, expected_json);
}

fn provider_success() -> ProviderOutcome<String> {
    ProviderOutcome::Success(Box::new("normalized payload".to_string()))
}

fn provider_not_found() -> ProviderOutcome<String> {
    ProviderOutcome::NotFound
}

fn provider_will_retry() -> ProviderOutcome<String> {
    ProviderOutcome::WillRetry {
        reason: WillRetryReason::Timeout,
        next_attempt_at: fixed_timestamp(),
    }
}

fn provider_permanent_failure() -> ProviderOutcome<String> {
    ProviderOutcome::PermanentFailure {
        reason: PermanentFailureReason::InvalidResponse,
    }
}

fn provider_conflict() -> ProviderOutcome<String> {
    ProviderOutcome::Conflict {
        detail: "gr_key: 'show/123' -> 'show/456'".to_string(),
    }
}

#[test]
fn test_domain_outcome_class_is_phase2_terminal_matches_contract_matrix() {
    // REQ-ID: R-22 | Contract: OutcomeClass::is_phase2_terminal | Behavior: terminality matches the documented matrix for all five outcome classes
    let cases = [
        (OutcomeClass::Success, true),
        (OutcomeClass::NotFound, true),
        (OutcomeClass::WillRetry, false),
        (OutcomeClass::PermanentFailure, true),
        (OutcomeClass::Conflict, true),
    ];

    for (class, expected) in cases {
        assert_eq!(
            class.is_phase2_terminal(),
            expected,
            "unexpected phase-2 terminality for {:?}",
            class
        );
    }
}

#[test]
fn test_domain_outcome_class_can_merge_matches_contract_matrix() {
    // REQ-ID: R-22 | Contract: OutcomeClass::can_merge | Behavior: merge eligibility matches the documented matrix for all five outcome classes
    let cases = [
        (OutcomeClass::Success, true),
        (OutcomeClass::NotFound, true),
        (OutcomeClass::WillRetry, false),
        (OutcomeClass::PermanentFailure, true),
        (OutcomeClass::Conflict, false),
    ];

    for (class, expected) in cases {
        assert_eq!(
            class.can_merge(),
            expected,
            "unexpected merge eligibility for {:?}",
            class
        );
    }
}

#[test]
fn test_domain_outcome_class_all_can_merge_returns_true_when_all_outcomes_are_merge_eligible() {
    // REQ-ID: R-22 | Contract: OutcomeClass::all_can_merge | Behavior: returns true when every outcome class is merge-eligible
    let outcomes = [
        OutcomeClass::Success,
        OutcomeClass::NotFound,
        OutcomeClass::PermanentFailure,
    ];

    assert!(OutcomeClass::all_can_merge(&outcomes));
}

#[test]
fn test_domain_outcome_class_all_can_merge_returns_false_when_will_retry_is_present() {
    // REQ-ID: R-22 | Contract: OutcomeClass::all_can_merge | Behavior: returns false when a WillRetry outcome is present
    let outcomes = [OutcomeClass::Success, OutcomeClass::WillRetry];

    assert!(!OutcomeClass::all_can_merge(&outcomes));
}

#[test]
fn test_domain_outcome_class_all_can_merge_returns_false_when_conflict_is_present() {
    // REQ-ID: R-22 | Contract: OutcomeClass::all_can_merge | Behavior: returns false when a Conflict outcome is present
    let outcomes = [OutcomeClass::NotFound, OutcomeClass::Conflict];

    assert!(!OutcomeClass::all_can_merge(&outcomes));
}

#[test]
fn test_domain_outcome_class_conflict_invariant_is_terminal_and_not_mergeable() {
    // REQ-ID: R-22 | Contract: OutcomeClass::{is_phase2_terminal,can_merge} | Behavior: Conflict satisfies invariant I-9 by being terminal but not mergeable
    assert!(OutcomeClass::Conflict.is_phase2_terminal());
    assert!(!OutcomeClass::Conflict.can_merge());
}

#[test]
fn test_domain_merge_resolved_new_wraps_value_without_transformation() {
    // REQ-ID: R-02 | Contract: MergeResolved::new | Behavior: new wraps the provided value and preserves it for later extraction
    assert_merge_resolved_new_roundtrips(vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn test_domain_merge_resolved_into_inner_returns_wrapped_value() {
    // REQ-ID: R-02 | Contract: MergeResolved::into_inner | Behavior: into_inner returns the exact wrapped value
    let wrapped = MergeResolved("canonical-id".to_string());

    assert_eq!(wrapped.into_inner(), "canonical-id".to_string());
}

#[test]
fn test_domain_merge_resolved_as_inner_borrows_wrapped_value() {
    // REQ-ID: R-02 | Contract: MergeResolved::as_inner | Behavior: as_inner returns a shared reference to the wrapped value
    let wrapped = MergeResolved(vec![1_u8, 2_u8, 3_u8]);

    assert_eq!(wrapped.as_inner(), &vec![1_u8, 2_u8, 3_u8]);
}

#[test]
fn test_domain_external_id_type_serializes_isbn13_in_snake_case() {
    // REQ-ID: R-06 | Contract: ExternalIdType serde serialization | Behavior: Isbn13 serializes using snake_case naming
    assert_serializes_to_json(ExternalIdType::Isbn13, "\"isbn13\"");
}

#[test]
fn test_domain_external_id_type_serializes_isbn10_in_snake_case() {
    // REQ-ID: R-06 | Contract: ExternalIdType serde serialization | Behavior: Isbn10 serializes using snake_case naming
    assert_serializes_to_json(ExternalIdType::Isbn10, "\"isbn10\"");
}

#[test]
fn test_domain_external_id_type_serializes_asin_in_snake_case() {
    // REQ-ID: R-06 | Contract: ExternalIdType serde serialization | Behavior: Asin serializes using snake_case naming
    assert_serializes_to_json(ExternalIdType::Asin, "\"asin\"");
}

#[test]
fn test_domain_permanent_failure_reason_declares_all_documented_variants() {
    // REQ-ID: R-20/R-21/R-22 | Contract: PermanentFailureReason | Behavior: all documented permanent failure reasons are constructible and distinct
    let reasons = [
        PermanentFailureReason::ProviderPanic,
        PermanentFailureReason::RetryBudgetExhausted,
        PermanentFailureReason::InvalidResponse,
        PermanentFailureReason::Unsupported,
        PermanentFailureReason::IdentityMismatch,
    ];

    let unique: HashSet<_> = reasons.iter().map(discriminant).collect();

    assert_eq!(reasons.len(), 5);
    assert_eq!(unique.len(), 5);
}

#[test]
fn test_domain_enrichment_mode_declares_three_distinct_modes() {
    // REQ-ID: R-22 | Contract: EnrichmentMode | Behavior: Background, Manual, and HardRefresh are all distinct modes and Manual/HardRefresh are not Background
    let modes = [
        EnrichmentMode::Background,
        EnrichmentMode::Manual,
        EnrichmentMode::HardRefresh,
    ];
    let unique: HashSet<_> = modes.iter().map(discriminant).collect();

    assert_eq!(unique.len(), 3);
    assert_ne!(EnrichmentMode::Background, EnrichmentMode::Manual);
    assert_ne!(EnrichmentMode::Background, EnrichmentMode::HardRefresh);
    assert_ne!(EnrichmentMode::Manual, EnrichmentMode::HardRefresh);
}

#[test]
fn test_domain_provider_outcome_class_maps_each_variant_to_its_outcome_class() {
    // REQ-ID: R-22 | Contract: ProviderOutcome::class | Behavior: each ProviderOutcome variant maps to the corresponding OutcomeClass
    let cases = [
        (provider_success(), OutcomeClass::Success),
        (provider_not_found(), OutcomeClass::NotFound),
        (provider_will_retry(), OutcomeClass::WillRetry),
        (provider_permanent_failure(), OutcomeClass::PermanentFailure),
        (provider_conflict(), OutcomeClass::Conflict),
    ];

    for (outcome, expected) in cases {
        assert_provider_outcome_class(outcome, expected);
    }
}

#[test]
fn test_domain_provider_outcome_can_merge_matches_outcome_contract_matrix() {
    // REQ-ID: R-22 | Contract: ProviderOutcome::can_merge | Behavior: merge eligibility matches the documented matrix for all ProviderOutcome variants
    let cases = [
        (provider_success(), true),
        (provider_not_found(), true),
        (provider_will_retry(), false),
        (provider_permanent_failure(), true),
        (provider_conflict(), false),
    ];

    for (outcome, expected) in cases {
        assert_provider_outcome_can_merge(outcome, expected);
    }
}

#[test]
fn test_domain_provider_outcome_can_merge_manual_matches_manual_mode_contract_matrix() {
    // REQ-ID: R-22 | Contract: ProviderOutcome::can_merge_manual | Behavior: manual-mode merge eligibility coerces WillRetry but still blocks Conflict
    let cases = [
        (provider_success(), true),
        (provider_not_found(), true),
        (provider_will_retry(), true),
        (provider_permanent_failure(), true),
        (provider_conflict(), false),
    ];

    for (outcome, expected) in cases {
        assert_provider_outcome_can_merge_manual(outcome, expected);
    }
}

#[test]
fn test_domain_provider_outcome_success_variant_boxes_payload() {
    // REQ-ID: R-22 | Contract: ProviderOutcome::Success | Behavior: Success stores its payload in Box<T> and yields the boxed inner value when matched
    let outcome: ProviderOutcome<String> =
        ProviderOutcome::Success(Box::new("boxed-payload".to_string()));

    match outcome {
        ProviderOutcome::Success(inner) => {
            assert_eq!(*inner, "boxed-payload".to_string());
        }
        _ => panic!("expected Success variant with boxed payload"),
    }
}

#[test]
fn test_domain_metadata_provider_serializes_each_variant_in_snake_case() {
    // REQ-ID: R-18/R-22 | Contract: MetadataProvider serde serialization | Behavior: each provider variant serializes to its exact snake_case string
    let cases = [
        (MetadataProvider::Hardcover, "\"hardcover\""),
        (MetadataProvider::OpenLibrary, "\"open_library\""),
        (MetadataProvider::Goodreads, "\"goodreads\""),
        (MetadataProvider::Audnexus, "\"audnexus\""),
        (MetadataProvider::Llm, "\"llm\""),
    ];

    for (provider, expected_json) in cases {
        assert_serializes_to_json(provider, expected_json);
    }
}

#[test]
fn test_domain_work_field_serializes_each_variant_in_snake_case() {
    // REQ-ID: R-18 | Contract: WorkField serde serialization | Behavior: each work field variant serializes to its exact snake_case string
    let cases = [
        (WorkField::Title, "\"title\""),
        (WorkField::Subtitle, "\"subtitle\""),
        (WorkField::OriginalTitle, "\"original_title\""),
        (WorkField::AuthorName, "\"author_name\""),
        (WorkField::Description, "\"description\""),
        (WorkField::Year, "\"year\""),
        (WorkField::SeriesName, "\"series_name\""),
        (WorkField::SeriesPosition, "\"series_position\""),
        (WorkField::Genres, "\"genres\""),
        (WorkField::Language, "\"language\""),
        (WorkField::PageCount, "\"page_count\""),
        (WorkField::DurationSeconds, "\"duration_seconds\""),
        (WorkField::Publisher, "\"publisher\""),
        (WorkField::PublishDate, "\"publish_date\""),
        (WorkField::HcKey, "\"hc_key\""),
        (WorkField::GrKey, "\"gr_key\""),
        (WorkField::Isbn13, "\"isbn13\""),
        (WorkField::Asin, "\"asin\""),
        (WorkField::Narrator, "\"narrator\""),
        (WorkField::NarrationType, "\"narration_type\""),
        (WorkField::Abridged, "\"abridged\""),
        (WorkField::Rating, "\"rating\""),
        (WorkField::RatingCount, "\"rating_count\""),
        (WorkField::CoverUrl, "\"cover_url\""),
        (WorkField::OlKey, "\"ol_key\""),
        (WorkField::SortTitle, "\"sort_title\""),
    ];

    for (field, expected_json) in cases {
        assert_serializes_to_json(field, expected_json);
    }
}

#[test]
fn test_domain_provenance_setter_serializes_each_variant_in_snake_case() {
    // REQ-ID: R-18/R-02 | Contract: ProvenanceSetter serde serialization | Behavior: each setter variant serializes to its exact snake_case string
    let cases = [
        (ProvenanceSetter::User, "\"user\""),
        (ProvenanceSetter::Provider, "\"provider\""),
        (ProvenanceSetter::System, "\"system\""),
    ];

    for (setter, expected_json) in cases {
        assert_serializes_to_json(setter, expected_json);
    }
}

#[test]
fn test_domain_outcome_class_serializes_each_variant_in_snake_case() {
    // REQ-ID: R-22 | Contract: OutcomeClass serde serialization | Behavior: each outcome class variant serializes to its exact snake_case string
    let cases = [
        (OutcomeClass::Success, "\"success\""),
        (OutcomeClass::NotFound, "\"not_found\""),
        (OutcomeClass::WillRetry, "\"will_retry\""),
        (OutcomeClass::PermanentFailure, "\"permanent_failure\""),
        (OutcomeClass::Conflict, "\"conflict\""),
    ];

    for (class, expected_json) in cases {
        assert_serializes_to_json(class, expected_json);
    }
}

#[test]
fn test_domain_will_retry_reason_serializes_each_variant_in_snake_case() {
    // REQ-ID: R-22 | Contract: WillRetryReason serde serialization | Behavior: each will-retry reason variant serializes to its exact snake_case string
    let cases = [
        (WillRetryReason::Timeout, "\"timeout\""),
        (WillRetryReason::RateLimit, "\"rate_limit\""),
        (WillRetryReason::ServerError, "\"server_error\""),
        (WillRetryReason::AntiBotBlock, "\"anti_bot_block\""),
    ];

    for (reason, expected_json) in cases {
        assert_serializes_to_json(reason, expected_json);
    }
}

#[test]
fn test_domain_permanent_failure_reason_serializes_each_variant_in_snake_case() {
    // REQ-ID: R-20/R-21/R-22 | Contract: PermanentFailureReason serde serialization | Behavior: each permanent-failure reason variant serializes to its exact snake_case string
    let cases = [
        (PermanentFailureReason::ProviderPanic, "\"provider_panic\""),
        (
            PermanentFailureReason::RetryBudgetExhausted,
            "\"retry_budget_exhausted\"",
        ),
        (
            PermanentFailureReason::InvalidResponse,
            "\"invalid_response\"",
        ),
        (PermanentFailureReason::Unsupported, "\"unsupported\""),
        (
            PermanentFailureReason::IdentityMismatch,
            "\"identity_mismatch\"",
        ),
    ];

    for (reason, expected_json) in cases {
        assert_serializes_to_json(reason, expected_json);
    }
}

#[test]
/// REQ-ID: REQ-001 | Contract: CoverTrust::allows_replacement_by | Behavior: evaluates all trust-tier replacement combinations.
fn test_multi_cover_cover_trust_replacement_matrix_matches_hierarchy() {
    let cases = [
        (CoverTrust::Unvalidated, CoverTrust::Unvalidated, true),
        (CoverTrust::Unvalidated, CoverTrust::Validated, true),
        (CoverTrust::Unvalidated, CoverTrust::User, true),
        (CoverTrust::Validated, CoverTrust::Unvalidated, false),
        (CoverTrust::Validated, CoverTrust::Validated, true),
        (CoverTrust::Validated, CoverTrust::User, true),
        (CoverTrust::User, CoverTrust::Unvalidated, false),
        (CoverTrust::User, CoverTrust::Validated, false),
        (CoverTrust::User, CoverTrust::User, false),
    ];

    for (current, incoming, expected) in cases {
        assert_eq!(
            current.allows_replacement_by(incoming),
            expected,
            "unexpected replacement decision for current={current:?}, incoming={incoming:?}"
        );
    }
}

#[test]
/// REQ-ID: REQ-004 | Contract: CoverTrust::allows_replacement_by | Behavior: User trust is a permanent lock against automatic replacement.
fn test_multi_cover_user_trust_never_allows_automatic_replacement() {
    for incoming in [
        CoverTrust::Unvalidated,
        CoverTrust::Validated,
        CoverTrust::User,
    ] {
        assert!(
            !CoverTrust::User.allows_replacement_by(incoming),
            "User trust must reject incoming {incoming:?}"
        );
    }
}

#[test]
/// REQ-ID: REQ-001 | Contract: CoverTrust::allows_replacement_by | Behavior: Unvalidated trust can be replaced by any incoming trust tier.
fn test_multi_cover_unvalidated_trust_allows_any_incoming_replacement() {
    for incoming in [
        CoverTrust::Unvalidated,
        CoverTrust::Validated,
        CoverTrust::User,
    ] {
        assert!(
            CoverTrust::Unvalidated.allows_replacement_by(incoming),
            "Unvalidated trust must accept incoming {incoming:?}"
        );
    }
}

#[test]
/// REQ-ID: REQ-007 | Contract: CoverMediaType::suffix | Behavior: ebook uses the primary cover path suffix and audiobook uses the audio slot suffix.
fn test_multi_cover_media_type_suffix_matches_storage_convention() {
    assert_eq!(CoverMediaType::Ebook.suffix(), "");
    assert_eq!(CoverMediaType::Audiobook.suffix(), "_audio");
}

/// Minimal valid 1x1 white JPEG (107 bytes). Used for upload tests that must
/// pass image decode validation per REQ-020.
const MINIMAL_JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x01, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x08, 0x06, 0x06, 0x07, 0x06, 0x05, 0x08,
    0x07, 0x07, 0x07, 0x09, 0x09, 0x08, 0x0A, 0x0C, 0x14, 0x0D, 0x0C, 0x0B, 0x0B, 0x0C, 0x19, 0x12,
    0x13, 0x0F, 0x14, 0x1D, 0x1A, 0x1F, 0x1E, 0x1D, 0x1A, 0x1C, 0x1C, 0x20, 0x24, 0x2E, 0x27, 0x20,
    0x22, 0x2C, 0x23, 0x1C, 0x1C, 0x28, 0x37, 0x29, 0x2C, 0x30, 0x31, 0x34, 0x34, 0x34, 0x1F, 0x27,
    0x39, 0x3D, 0x38, 0x32, 0x3C, 0x2E, 0x33, 0x34, 0x32, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01,
    0x00, 0x01, 0x01, 0x01, 0x11, 0x00, 0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01,
    0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04,
    0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03,
    0x03, 0x02, 0x04, 0x03, 0x05, 0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00,
    0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32,
    0x81, 0x91, 0xA1, 0x08, 0x23, 0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72,
    0x82, 0x09, 0x0A, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35,
    0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55,
    0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75,
    0x76, 0x77, 0x78, 0x79, 0x7A, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94,
    0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2,
    0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9,
    0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6,
    0xE7, 0xE8, 0xE9, 0xEA, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xDA,
    0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0x7B, 0x94, 0x11, 0x00, 0x00, 0x00, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xD9,
];

#[derive(Clone, Default)]
struct ContractOnlyCoverService;

impl CoverService for ContractOnlyCoverService {
    async fn fetch_alternatives(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
    ) -> Result<Vec<CoverCandidate>, CoverServiceError> {
        Ok(vec![CoverCandidate {
            candidate_id: "goodreads:ebook".to_string(),
            proxy_url: "/api/v1/coverproxy?provider=goodreads&sig=test-signature".to_string(),
            source: "Goodreads".to_string(),
            media_type: CoverMediaType::Ebook,
            width: 800,
            height: 1200,
            passes_quality_gate: true,
        }])
    }

    async fn select_cover(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        candidate_id: &str,
        media_type: CoverMediaType,
    ) -> Result<(), CoverServiceError> {
        assert_eq!(candidate_id, "goodreads:ebook");
        assert_eq!(media_type, CoverMediaType::Ebook);
        Ok(())
    }

    async fn upload_cover(
        &self,
        _user_id: UserId,
        _work_id: WorkId,
        data: &[u8],
        media_type: CoverMediaType,
    ) -> Result<(), CoverServiceError> {
        assert!(
            data.len() >= 2 && data[0] == 0xFF && data[1] == 0xD8,
            "data must be valid JPEG"
        );
        assert_eq!(media_type, CoverMediaType::Audiobook);
        Ok(())
    }
}

#[tokio::test]
#[ignore = "not yet implemented"]
/// REQ-ID: REQ-016/REQ-021 | Contract: CoverService::fetch_alternatives | Behavior: alternatives expose opaque IDs and proxied URLs, not raw provider URLs.
async fn test_multi_cover_service_fetch_alternatives_returns_browser_safe_candidates() {
    let service = ContractOnlyCoverService;

    let candidates = service.fetch_alternatives(7, 41).await.unwrap();

    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    assert_eq!(candidate.candidate_id, "goodreads:ebook");
    assert_eq!(candidate.media_type, CoverMediaType::Ebook);
    assert!(candidate.proxy_url.starts_with("/api/v1/coverproxy"));
    assert!(
        !candidate.proxy_url.contains("https://images.example.test"),
        "raw provider URLs must not appear in alternatives"
    );
    assert!(candidate.passes_quality_gate);
}

#[tokio::test]
#[ignore = "not yet implemented"]
/// REQ-ID: REQ-015/REQ-021 | Contract: CoverService::select_cover | Behavior: selection accepts an opaque candidate ID and media slot.
async fn test_multi_cover_service_select_cover_accepts_candidate_id_not_url() {
    let service = ContractOnlyCoverService;

    service
        .select_cover(7, 41, "goodreads:ebook", CoverMediaType::Ebook)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "not yet implemented"]
/// REQ-ID: REQ-020 | Contract: CoverService::upload_cover | Behavior: valid image data is accepted for the requested independent cover slot.
async fn test_multi_cover_service_upload_cover_accepts_valid_image_for_requested_slot() {
    let service = ContractOnlyCoverService;

    service
        .upload_cover(7, 41, MINIMAL_JPEG, CoverMediaType::Audiobook)
        .await
        .unwrap();
}
