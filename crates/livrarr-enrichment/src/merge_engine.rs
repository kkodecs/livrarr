//! The field-merge engine: computes a field-level merge from per-provider
//! outcomes.
//!
//! `MergeEngine::merge` is the payload-policy chokepoint both the network
//! enrichment path (`enrich_work`) and the cached-payload reuse path
//! (`merge_from_cached`) funnel through: the language-routing drop
//! (`drop_language_incompatible_providers`) and the Goodreads cover gate
//! (`apply_gr_cover_gate`) run here before `merge_impl` walks each field's
//! provider priority order (`PriorityModel`/`MERGE_FIELDS`) to resolve the
//! final value.

use std::collections::HashMap;

use livrarr_db::{
    ApplyEnrichmentMergeRequest, SetFieldProvenanceRequest, UpdateWorkEnrichmentDbRequest,
};
use livrarr_domain::{
    DissentReason, EnrichmentStatus, FieldProvenance, MergeResolved, NarrationType, Work, WorkField,
};
use livrarr_external_data::NormalizedWorkDetail;

use crate::cover_resolution;
use crate::{EnrichmentMode, ReconstructedOutcome};

/// TEMP(pk-tdd): priority order per field group for merge resolution.
#[derive(Debug, Clone)]
pub struct PriorityModel {
    pub content: Vec<livrarr_domain::MetadataProvider>,
    pub description: Vec<livrarr_domain::MetadataProvider>,
    pub cover: Vec<livrarr_domain::MetadataProvider>,
    pub audio: Vec<livrarr_domain::MetadataProvider>,
}

impl PriorityModel {
    /// English content/description order is unchanged by N2 (cover-only
    /// consolidation); `cover`/`audio` are derived from the single rank table
    /// (S1) — see `cover_rank::CoverRankModel::EbookEnglish`/`Audiobook`.
    pub fn english() -> Self {
        use livrarr_domain::MetadataProvider as P;
        Self {
            content: vec![
                P::Hardcover,
                P::Goodreads,
                P::Readarr,
                P::OpenLibrary,
                P::Audible,
            ],
            description: vec![
                P::Hardcover,
                P::Goodreads,
                P::Readarr,
                P::OpenLibrary,
                P::Audible,
            ],
            cover: crate::cover_rank::rank_table(crate::cover_rank::CoverRankModel::EbookEnglish)
                .to_vec(),
            audio: crate::cover_rank::rank_table(crate::cover_rank::CoverRankModel::Audiobook)
                .to_vec(),
        }
    }

    /// Foreign content/description order is unchanged by N2; `cover`/`audio`
    /// are derived from the single rank table (S1) — see
    /// `cover_rank::CoverRankModel::EbookForeign`/`Audiobook`.
    pub fn foreign() -> Self {
        use livrarr_domain::MetadataProvider as P;
        Self {
            content: vec![
                P::GoogleBooks,
                P::Goodreads,
                P::Hardcover,
                P::Readarr,
                P::OpenLibrary,
                P::Audible,
            ],
            description: vec![
                P::GoogleBooks,
                P::Goodreads,
                P::Hardcover,
                P::Readarr,
                P::OpenLibrary,
                P::Audible,
            ],
            cover: crate::cover_rank::rank_table(crate::cover_rank::CoverRankModel::EbookForeign)
                .to_vec(),
            audio: crate::cover_rank::rank_table(crate::cover_rank::CoverRankModel::Audiobook)
                .to_vec(),
        }
    }

    /// Select model based on work language.
    pub fn for_language(language: Option<&str>) -> Self {
        match livrarr_external_data::language::provider_priority(language) {
            livrarr_external_data::language::ProviderPriority::English => Self::english(),
            livrarr_external_data::language::ProviderPriority::Foreign => Self::foreign(),
        }
    }
}

/// TEMP(pk-tdd): inputs to MergeEngine::merge.
#[derive(Debug, Clone)]
pub struct MergeInput {
    pub current_work: Work,
    pub current_provenance: Vec<FieldProvenance>,
    pub provider_results: HashMap<livrarr_domain::MetadataProvider, ReconstructedOutcome>,
    pub mode: EnrichmentMode,
    pub priority_model: PriorityModel,
}

/// TEMP(pk-tdd): output of MergeEngine::merge.
#[derive(Debug, Clone)]
pub struct MergeOutput {
    pub work_update: Option<MergeResolved<UpdateWorkEnrichmentDbRequest>>,
    pub provenance_upserts: Vec<SetFieldProvenanceRequest>,
    pub provenance_deletes: Vec<WorkField>,
    pub enrichment_status: EnrichmentStatus,
    pub enrichment_source: Option<String>,
    pub cover_resolution: Option<livrarr_domain::CoverResolution>,
    pub audiobook_cover_resolution: Option<livrarr_domain::CoverResolution>,
    /// Per-field/per-provider dissents (REQ-014): contributions excluded at
    /// provider or field granularity; the merge proceeds with the rest.
    pub dissents: Vec<livrarr_domain::FieldDissent>,
}

/// TEMP(pk-tdd): error from MergeEngine::merge.
#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error("priority model has no providers for required field groups")]
    EmptyPriorityModel,
}

/// Merge engine — computes field-level merge from provider outcomes.
///
/// Async because the LLM arbitration path makes a network call.
/// The deterministic fallback is purely synchronous — the async overhead
/// is negligible compared to the prior scatter-gather.
#[trait_variant::make(Send)]
pub trait MergeEngine: Send + Sync {
    async fn merge(&self, inputs: MergeInput) -> Result<MergeOutput, MergeError>;

    /// Merge from already-fetched per-provider payloads — zero provider network
    /// calls (REQ-014/015). The add path reuses the payloads the resolver cached
    /// during discovery instead of re-querying. See ir-v2 metadata-merge-reuse.
    async fn merge_from_cached(
        &self,
        work: Work,
        payloads: HashMap<livrarr_domain::MetadataProvider, NormalizedWorkDetail>,
        current_provenance: Vec<FieldProvenance>,
        language: Option<&str>,
    ) -> Result<MergeOutput, MergeError>;
}

/// Deterministic merge engine (REQ-004/REQ-005, P-C): pure and zero-LLM. The
/// per-merge priority model is taken from `MergeInput`; the engine is stateless.
pub struct DefaultMergeEngine;

/// Build the DB apply-request from a computed merge output, rewriting the
/// per-row ids to the target (user_id, work_id). Shared by the network
/// enrichment path (`enrich_work`) and the cached-payload reuse path in
/// `WorkService::add` (REQ-014/015) so both produce byte-identical writes.
pub fn build_apply_request(
    merge_output: &MergeOutput,
    user_id: livrarr_domain::UserId,
    work_id: livrarr_domain::WorkId,
    expected_merge_generation: i64,
) -> ApplyEnrichmentMergeRequest {
    let provenance_upserts = merge_output
        .provenance_upserts
        .iter()
        .map(|p| SetFieldProvenanceRequest {
            user_id,
            work_id,
            ..p.clone()
        })
        .collect();
    ApplyEnrichmentMergeRequest {
        user_id,
        work_id,
        expected_merge_generation,
        work_update: merge_output.work_update.clone(),
        new_enrichment_status: merge_output.enrichment_status,
        provenance_upserts,
        provenance_deletes: merge_output.provenance_deletes.clone(),
    }
}

impl DefaultMergeEngine {
    /// Construct the deterministic merge engine. `priority_model` is accepted for
    /// call-site compatibility; the per-merge model comes from `MergeInput`.
    pub fn new(_priority_model: PriorityModel) -> Self {
        Self
    }
}

impl DefaultMergeEngine {
    /// Compatibility constructor for call sites that previously supplied an LLM
    /// caller. The merge is purely deterministic now (REQ-005/D-010), so the
    /// caller and its configured flag are accepted and discarded.
    pub fn new_with_llm<L>(_priority_model: PriorityModel, _llm: L, _llm_configured: bool) -> Self
    where
        L: livrarr_domain::services::LlmCaller + Send + Sync,
    {
        Self
    }
}

impl MergeEngine for DefaultMergeEngine {
    async fn merge(&self, inputs: MergeInput) -> Result<MergeOutput, MergeError> {
        // REQ-005/D-010: the merge is purely deterministic — ZERO LLM, even when a
        // caller is configured. Language routing (REQ-014/#133) is enforced here at
        // the single chokepoint both the cached and network entry paths funnel through,
        // so a foreign work can never take English OpenLibrary/Hardcover metadata.
        // The Goodreads cover gate (REQ-017) is the second chokepoint policy enforced
        // here, so a mismatched-title GR cover can never win on either entry path.
        // `had_providers` is captured BEFORE the policy drops: providers that were
        // attempted but all excluded yield a status-only output (REQ-014), while an
        // empty dispatch re-materializes current state as before.
        let had_providers = !inputs.provider_results.is_empty();
        let inputs = drop_language_incompatible_providers(inputs);
        let inputs = apply_gr_cover_gate(inputs);
        merge_impl(inputs, had_providers)
    }

    /// Merge from already-fetched per-provider payloads — zero provider network
    /// calls (REQ-014/015). Wraps each payload as a ReconstructedOutcome and runs
    /// the deterministic merge. The foreign-work OpenLibrary/Hardcover drop
    /// (REQ-027) is enforced centrally in `merge`, so this cached path and the
    /// network path share one language-routing policy (#133).
    async fn merge_from_cached(
        &self,
        work: Work,
        payloads: HashMap<livrarr_domain::MetadataProvider, NormalizedWorkDetail>,
        current_provenance: Vec<FieldProvenance>,
        language: Option<&str>,
    ) -> Result<MergeOutput, MergeError> {
        let provider_results = payloads
            .into_iter()
            .map(|(provider, detail)| {
                (
                    provider,
                    ReconstructedOutcome {
                        class: livrarr_domain::OutcomeClass::Success,
                        payload: Some(detail),
                    },
                )
            })
            .collect();
        let input = MergeInput {
            current_work: work,
            current_provenance,
            provider_results,
            mode: EnrichmentMode::Manual,
            priority_model: PriorityModel::for_language(language),
        };
        self.merge(input).await
    }
}

// =============================================================================
// Merge implementation helpers
// =============================================================================

/// Enforce P2 (a book's language is sacred): a foreign-language work must never
/// take English-centric OpenLibrary/Hardcover metadata (#133 / REQ-027). Called
/// once at the `MergeEngine::merge` chokepoint, so the rule is caller-independent —
/// `PriorityModel::foreign()` still lists OL/HC as fallbacks, so reordering alone
/// is insufficient; the providers must be removed from the inputs. OL/HC anchors
/// are captured upstream at the identity resolver (language-agnostic), so only
/// metadata contribution is affected, not identity.
fn drop_language_incompatible_providers(mut inputs: MergeInput) -> MergeInput {
    use livrarr_domain::MetadataProvider as P;
    let is_foreign = matches!(
        livrarr_external_data::language::provider_priority(inputs.current_work.language.as_deref()),
        livrarr_external_data::language::ProviderPriority::Foreign
    );
    if is_foreign {
        inputs
            .provider_results
            .retain(|provider, _| !matches!(provider, P::OpenLibrary | P::Hardcover));
    }
    inputs
}

/// Enforce the Goodreads cover gate (REQ-017): for an English work with an OL
/// key, a Goodreads payload's cover_url only survives if its title clears the
/// deterministic Jaccard threshold against the work title. Called once at the
/// `MergeEngine::merge` chokepoint so the cached-reuse path and the network
/// path share one cover-gating policy — the same centralization
/// `drop_language_incompatible_providers` applies to the language-routing
/// policy (#133).
fn apply_gr_cover_gate(mut inputs: MergeInput) -> MergeInput {
    if inputs.current_work.language.as_deref() == Some("en") && inputs.current_work.ol_key.is_some()
    {
        if let Some(gr_outcome) = inputs
            .provider_results
            .get_mut(&livrarr_domain::MetadataProvider::Goodreads)
        {
            if let Some(ref mut payload) = gr_outcome.payload {
                if payload.cover_url.is_some() {
                    let anchor = crate::cover_gate::OlAnchor {
                        title: &inputs.current_work.title,
                        author_name: &inputs.current_work.author_name,
                        year: inputs.current_work.year,
                        isbn: inputs.current_work.isbn_13.as_deref(),
                        ol_key: inputs.current_work.ol_key.as_deref().unwrap_or(""),
                    };
                    let candidate = crate::cover_gate::GrCandidate {
                        title: payload.title.as_deref().unwrap_or(""),
                        author_name: payload.author_name.as_deref().unwrap_or(""),
                        year: payload.year,
                        isbn: None,
                        gr_key: payload.gr_key.as_deref().unwrap_or(""),
                    };
                    // REQ-005/REQ-016 (zero LLM): the merge is deterministic and
                    // LLM-free, so a borderline title is a strip either way — the
                    // gate itself has no LLM tier to select (D10).
                    let outcome = crate::cover_gate::evaluate_gr_cover_gate(&anchor, &candidate);
                    match outcome {
                        crate::cover_gate::CoverGateOutcome::Apply { .. } => {}
                        other => {
                            tracing::info!(
                                work_id = inputs.current_work.id,
                                ?other,
                                "cover gate: stripping GR cover_url"
                            );
                            payload.cover_url = None;
                            payload.gr_key = None;
                        }
                    }
                }
            }
        }
    }
    inputs
}

/// Field category for priority model lookup.
enum FieldCategory {
    Content,
    Description,
    Cover,
    Audio,
}

/// Map a WorkField to its priority model category.
fn field_category(field: WorkField) -> FieldCategory {
    match field {
        WorkField::Description => FieldCategory::Description,
        WorkField::CoverUrl => FieldCategory::Cover,
        WorkField::DurationSeconds
        | WorkField::Narrator
        | WorkField::NarrationType
        | WorkField::Abridged
        | WorkField::Asin => FieldCategory::Audio,
        // Everything else is content
        _ => FieldCategory::Content,
    }
}

/// Get the priority list for a field from the priority model.
fn priority_list_for(field: WorkField, pm: &PriorityModel) -> &[livrarr_domain::MetadataProvider] {
    match field_category(field) {
        FieldCategory::Content => &pm.content,
        FieldCategory::Description => &pm.description,
        FieldCategory::Cover => &pm.cover,
        FieldCategory::Audio => &pm.audio,
    }
}

/// Represents a resolved field value — either a string-like option, or typed data.
/// We use an enum to handle the different field value types uniformly.
#[derive(Debug, Clone)]
enum FieldValue {
    Str(Option<String>),
    Int(Option<i32>),
    Float(Option<f64>),
    Bool(Option<bool>),
    Strings(Option<Vec<String>>),
    NarrationType(Option<NarrationType>),
}

impl FieldValue {
    fn is_some(&self) -> bool {
        match self {
            Self::Str(v) => v.is_some(),
            Self::Int(v) => v.is_some(),
            Self::Float(v) => v.is_some(),
            Self::Bool(v) => v.is_some(),
            Self::Strings(v) => v.is_some(),
            Self::NarrationType(v) => v.is_some(),
        }
    }
}

/// Extract a field value from NormalizedWorkDetail.
fn extract_provider_field(field: WorkField, detail: &NormalizedWorkDetail) -> FieldValue {
    match field {
        WorkField::Title => FieldValue::Str(non_blank(&detail.title)),
        WorkField::SortTitle => FieldValue::Str(None), // not in NormalizedWorkDetail
        WorkField::Subtitle => FieldValue::Str(non_blank(&detail.subtitle)),
        WorkField::OriginalTitle => FieldValue::Str(non_blank(&detail.original_title)),
        WorkField::AuthorName => FieldValue::Str(non_blank(&detail.author_name)),
        WorkField::Description => FieldValue::Str(non_blank(&detail.description)),
        WorkField::Year => FieldValue::Int(detail.year),
        WorkField::SeriesName => FieldValue::Str(non_blank(&detail.series_name)),
        WorkField::SeriesPosition => FieldValue::Float(detail.series_position),
        WorkField::Genres => FieldValue::Strings(non_empty_vec(&detail.genres)),
        WorkField::Language => FieldValue::Str(non_blank(&detail.language)),
        WorkField::PageCount => FieldValue::Int(detail.page_count),
        WorkField::DurationSeconds => FieldValue::Int(detail.duration_seconds),
        WorkField::Publisher => FieldValue::Str(non_blank(&detail.publisher)),
        WorkField::PublishDate => FieldValue::Str(non_blank(&detail.publish_date)),
        WorkField::OlKey => FieldValue::Str(non_blank(&detail.ol_key)),
        WorkField::HcKey => FieldValue::Str(non_blank(&detail.hc_key)),
        WorkField::GrKey => FieldValue::Str(non_blank(&detail.gr_key)),
        WorkField::Isbn13 => FieldValue::Str(non_blank(&detail.isbn_13)),
        WorkField::Asin => FieldValue::Str(non_blank(&detail.asin)),
        WorkField::Narrator => FieldValue::Strings(non_empty_vec(&detail.narrator)),
        WorkField::NarrationType => FieldValue::NarrationType(detail.narration_type),
        WorkField::Abridged => FieldValue::Bool(detail.abridged),
        WorkField::Rating => FieldValue::Float(detail.rating),
        WorkField::RatingCount => FieldValue::Int(detail.rating_count),
        WorkField::CoverUrl => FieldValue::Str(non_blank(&detail.cover_url)),
    }
}

/// Extract current field value from the Work struct.
fn extract_current_field(field: WorkField, work: &Work) -> FieldValue {
    match field {
        WorkField::Title => FieldValue::Str(non_blank_owned(&work.title)),
        WorkField::SortTitle => FieldValue::Str(work.sort_title.clone()),
        WorkField::Subtitle => FieldValue::Str(work.subtitle.clone()),
        WorkField::OriginalTitle => FieldValue::Str(work.original_title.clone()),
        WorkField::AuthorName => FieldValue::Str(non_blank_owned(&work.author_name)),
        WorkField::Description => FieldValue::Str(work.description.clone()),
        WorkField::Year => FieldValue::Int(work.year),
        WorkField::SeriesName => FieldValue::Str(work.series_name.clone()),
        WorkField::SeriesPosition => FieldValue::Float(work.series_position),
        WorkField::Genres => FieldValue::Strings(work.genres.clone()),
        WorkField::Language => FieldValue::Str(non_blank(&work.language)),
        WorkField::PageCount => FieldValue::Int(work.page_count),
        WorkField::DurationSeconds => FieldValue::Int(work.duration_seconds),
        WorkField::Publisher => FieldValue::Str(work.publisher.clone()),
        WorkField::PublishDate => FieldValue::Str(work.publish_date.clone()),
        WorkField::OlKey => FieldValue::Str(work.ol_key.clone()),
        WorkField::HcKey => FieldValue::Str(work.hc_key.clone()),
        WorkField::GrKey => FieldValue::Str(work.gr_key.clone()),
        WorkField::Isbn13 => FieldValue::Str(work.isbn_13.clone()),
        WorkField::Asin => FieldValue::Str(work.asin.clone()),
        WorkField::Narrator => FieldValue::Strings(work.narrator.clone()),
        WorkField::NarrationType => FieldValue::NarrationType(work.narration_type),
        WorkField::Abridged => FieldValue::Bool(Some(work.abridged)),
        WorkField::Rating => FieldValue::Float(work.rating),
        WorkField::RatingCount => FieldValue::Int(work.rating_count),
        WorkField::CoverUrl => FieldValue::Str(work.cover_url.clone()),
    }
}

/// Returns None if the string is None or whitespace-only after trimming.
fn non_blank(s: &Option<String>) -> Option<String> {
    s.as_ref().filter(|v| !v.trim().is_empty()).cloned()
}

/// Returns None if the list is None or empty — an empty offer is no offer.
fn non_empty_vec(v: &Option<Vec<String>>) -> Option<Vec<String>> {
    v.as_ref().filter(|list| !list.is_empty()).cloned()
}

/// Returns None if the owned string is empty or whitespace-only.
fn non_blank_owned(s: &str) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s.to_owned())
    }
}

/// Lowercase name for a MetadataProvider (for enrichment_source).
fn provider_name(p: livrarr_domain::MetadataProvider) -> &'static str {
    p.record_key()
}

/// The ordered list of fields that we merge. SortTitle is excluded because
/// NormalizedWorkDetail and UpdateWorkEnrichmentDbRequest don't carry it.
/// Anchor fields (ol_key/hc_key/gr_key/isbn_13/asin) are deliberately absent
/// (REQ-007): provider anchor values are never read by the merge — anchors
/// move exclusively via the identity track.
const MERGE_FIELDS: &[WorkField] = &[
    WorkField::Title,
    WorkField::Subtitle,
    WorkField::OriginalTitle,
    WorkField::AuthorName,
    WorkField::Description,
    WorkField::Year,
    WorkField::SeriesName,
    WorkField::SeriesPosition,
    WorkField::Genres,
    WorkField::Language,
    WorkField::PageCount,
    WorkField::DurationSeconds,
    WorkField::Publisher,
    WorkField::PublishDate,
    WorkField::Narrator,
    WorkField::NarrationType,
    WorkField::Abridged,
    WorkField::Rating,
    WorkField::RatingCount,
];

/// A field participates in the REQ-013 language-dissent pass unless it is
/// audio-category. Audio fields (duration/narrator/narration_type/abridged)
/// come only from Audible/Audnexus, and a foreign-language work can
/// legitimately have a different-language audiobook edition — guarding them
/// would strip correct sole-source audio metadata. Fail-closed: every other
/// field (including ones added later) is gated by default.
fn is_language_dissent_field(field: WorkField) -> bool {
    !matches!(field_category(field), FieldCategory::Audio)
}

/// Display text for a field value, used for dissent rows; `None` when absent.
fn field_value_text(value: &FieldValue) -> Option<String> {
    match value {
        FieldValue::Str(v) => v.clone(),
        FieldValue::Int(v) => v.map(|n| n.to_string()),
        FieldValue::Float(v) => v.map(|n| n.to_string()),
        FieldValue::Bool(v) => v.map(|b| b.to_string()),
        FieldValue::Strings(v) => v.as_ref().map(|s| s.join(", ")),
        FieldValue::NarrationType(v) => v.map(|n| format!("{n:?}")),
    }
}

/// snake_case column name for a work field — the serde vocabulary shared
/// with the provenance store.
fn work_field_name(field: WorkField) -> String {
    serde_json::to_value(field)
        .expect("WorkField serialization is infallible")
        .as_str()
        .expect("WorkField serializes to a string")
        .to_string()
}

/// Core merge implementation. `had_providers` reflects the pre-drop input set
/// (see `MergeEngine::merge`): only a merge that attempted providers and lost
/// them all to exclusion produces a status-only (`work_update: None`) output.
fn merge_impl(inputs: MergeInput, had_providers: bool) -> Result<MergeOutput, MergeError> {
    let pm = &inputs.priority_model;

    // 1. Validate priority model: if ANY category is empty, error.
    if pm.content.is_empty()
        || pm.description.is_empty()
        || pm.cover.is_empty()
        || pm.audio.is_empty()
    {
        return Err(MergeError::EmptyPriorityModel);
    }

    // 2. REQ-014 (#110): a provider with a Conflict outcome is excluded from
    // the merge and its offered contribution is recorded as PayloadMismatch
    // dissent rows — the merge always proceeds with the remaining providers.
    // (Identity-level conflicts keep their separate IdentityStatus flow.)

    // 3. Determine which providers are merge-eligible based on mode.
    let eligible_providers: HashMap<
        livrarr_domain::MetadataProvider,
        Option<&NormalizedWorkDetail>,
    > = inputs
        .provider_results
        .iter()
        .filter(|(_, outcome)| {
            match inputs.mode {
                EnrichmentMode::Background => outcome.class.can_merge(),
                EnrichmentMode::Manual | EnrichmentMode::HardRefresh => {
                    // Conflict providers are dissent-isolated, never merged.
                    outcome.class != livrarr_domain::OutcomeClass::Conflict
                }
            }
        })
        .map(|(provider, outcome)| (*provider, outcome.payload.as_ref()))
        .collect();

    // Build a provenance lookup: field → FieldProvenance
    let prov_map: HashMap<WorkField, &FieldProvenance> = inputs
        .current_provenance
        .iter()
        .map(|fp| (fp.field, fp))
        .collect();

    let user_id = inputs.current_work.user_id;
    let work_id = inputs.current_work.id;

    // Dissent seeds: (provider, field, offered value, reason). Materialized
    // into FieldDissent rows after the field loop, because winning values
    // need the final resolved state.
    let mut dissent_seeds: Vec<(
        livrarr_domain::MetadataProvider,
        WorkField,
        String,
        DissentReason,
    )> = Vec::new();

    // REQ-014: every field a Conflict provider offered becomes a
    // PayloadMismatch dissent row (its whole contribution is excluded).
    for (provider, outcome) in &inputs.provider_results {
        if outcome.class != livrarr_domain::OutcomeClass::Conflict {
            continue;
        }
        let Some(ref detail) = outcome.payload else {
            continue; // nothing offered, nothing to record
        };
        for &field in MERGE_FIELDS {
            if let Some(offered) = field_value_text(&extract_provider_field(field, detail)) {
                dissent_seeds.push((*provider, field, offered, DissentReason::PayloadMismatch));
            }
        }
    }

    // REQ-013: on a foreign-language work, an eligible payload whose language
    // is KNOWN and incompatible contributes no text fields — each suppressed
    // value becomes a LanguageIncompatible dissent. Unknown (None) payload
    // language is unaffected; non-text fields are unaffected.
    let work_is_foreign = matches!(
        livrarr_external_data::language::provider_priority(inputs.current_work.language.as_deref()),
        livrarr_external_data::language::ProviderPriority::Foreign
    );
    let work_lang = inputs
        .current_work
        .language
        .as_deref()
        .map(livrarr_domain::normalize_language);
    let language_incompatible: std::collections::HashSet<livrarr_domain::MetadataProvider> =
        if work_is_foreign {
            eligible_providers
                .iter()
                .filter_map(|(provider, detail)| {
                    let payload_lang = non_blank(&(*detail)?.language)
                        .map(|l| livrarr_domain::normalize_language(&l))?;
                    (Some(&payload_lang) != work_lang.as_ref()).then_some(*provider)
                })
                .collect()
        } else {
            std::collections::HashSet::new()
        };
    for provider in &language_incompatible {
        if let Some(Some(detail)) = eligible_providers.get(provider) {
            for &field in MERGE_FIELDS
                .iter()
                .filter(|&&f| is_language_dissent_field(f))
            {
                if let Some(offered) = field_value_text(&extract_provider_field(field, detail)) {
                    dissent_seeds.push((
                        *provider,
                        field,
                        offered,
                        DissentReason::LanguageIncompatible,
                    ));
                }
            }
        }
    }

    // 4. Resolve each field.
    let mut provenance_upserts = Vec::new();
    let mut provenance_deletes = Vec::new();
    let mut resolved_values: HashMap<WorkField, FieldValue> = HashMap::new();
    let mut contributing_providers: Vec<livrarr_domain::MetadataProvider> = Vec::new();

    for &field in MERGE_FIELDS {
        // 4a. Identity fields are locked at add-time — never overwrite a non-empty
        // title/author/language. Language is identity-sovereign (P2): set once at
        // identity from real data, only a user changes it. A provider may FILL a
        // blank language but never override a set one.
        if field == WorkField::Title
            || field == WorkField::AuthorName
            || field == WorkField::Language
        {
            let current = extract_current_field(field, &inputs.current_work);
            if current.is_some() {
                resolved_values.insert(field, current);
                continue;
            }
        }

        // 4c. User-owned skip
        if let Some(fp) = prov_map.get(&field) {
            if fp.setter == livrarr_domain::ProvenanceSetter::User {
                let current = extract_current_field(field, &inputs.current_work);
                resolved_values.insert(field, current);
                continue;
            }
        }

        // 4c. Find winning provider by priority order
        let priority_list = priority_list_for(field, pm);
        let mut winner: Option<(livrarr_domain::MetadataProvider, FieldValue)> = None;

        for &provider in priority_list {
            // REQ-013: language-incompatible providers never win text fields
            // (their offered values are already dissent seeds).
            if is_language_dissent_field(field) && language_incompatible.contains(&provider) {
                continue;
            }
            if let Some(Some(detail)) = eligible_providers.get(&provider) {
                let val = extract_provider_field(field, detail);
                if val.is_some() {
                    winner = Some((provider, val));
                    break;
                }
            }
        }

        if let Some((provider, val)) = winner {
            // Provider wins — set value and generate provenance upsert
            resolved_values.insert(field, val);
            provenance_upserts.push(SetFieldProvenanceRequest {
                user_id,
                work_id,
                field,
                source: Some(provider),
                setter: livrarr_domain::ProvenanceSetter::Provider,
                cleared: false,
            });
            if !contributing_providers.contains(&provider) {
                contributing_providers.push(provider);
            }
        } else {
            // No winning provider — last-known-good
            let current = extract_current_field(field, &inputs.current_work);

            // If the field was provider-owned and current value exists,
            // generate a provenance delete (old provider no longer claims it).
            if current.is_some() {
                if let Some(fp) = prov_map.get(&field) {
                    if fp.setter == livrarr_domain::ProvenanceSetter::Provider {
                        provenance_deletes.push(field);
                    }
                }
            }

            resolved_values.insert(field, current);
        }
    }

    // 5. Build UpdateWorkEnrichmentDbRequest from resolved values.
    let get_str = |f: WorkField| -> Option<String> {
        match resolved_values.get(&f) {
            Some(FieldValue::Str(v)) => v.clone(),
            _ => None,
        }
    };
    let get_int = |f: WorkField| -> Option<i32> {
        match resolved_values.get(&f) {
            Some(FieldValue::Int(v)) => *v,
            _ => None,
        }
    };
    let get_float = |f: WorkField| -> Option<f64> {
        match resolved_values.get(&f) {
            Some(FieldValue::Float(v)) => *v,
            _ => None,
        }
    };
    let get_bool = |f: WorkField| -> Option<bool> {
        match resolved_values.get(&f) {
            Some(FieldValue::Bool(v)) => *v,
            _ => None,
        }
    };
    let get_strings = |f: WorkField| -> Option<Vec<String>> {
        match resolved_values.get(&f) {
            Some(FieldValue::Strings(v)) => v.clone(),
            _ => None,
        }
    };
    let get_narration_type = |f: WorkField| -> Option<NarrationType> {
        match resolved_values.get(&f) {
            Some(FieldValue::NarrationType(v)) => *v,
            _ => None,
        }
    };

    let merged_description = get_str(WorkField::Description);

    // 5b. Cover resolution (separate from generic field merge). REQ-006: covers
    // are chosen by provider PRIORITY (something-beats-nothing, no size ranking).
    // REQ-008: a user-locked cover (provenance Setter=User) is never resolved
    // over, so materialize neither downloads nor writes a replacement.
    let outcomes_ref: HashMap<livrarr_domain::MetadataProvider, &ReconstructedOutcome> = inputs
        .provider_results
        .iter()
        .map(|(p, o)| (*p, o))
        .collect();
    let cover_user_locked = prov_map
        .get(&WorkField::CoverUrl)
        .is_some_and(|fp| fp.setter == livrarr_domain::ProvenanceSetter::User && !fp.cleared);
    let cover_resolution = if cover_user_locked {
        None
    } else {
        cover_resolution::resolve_cover(
            &inputs.current_work,
            livrarr_domain::CoverMediaType::Ebook,
            &pm.cover,
            &eligible_providers,
            &outcomes_ref,
        )
    };
    let audiobook_cover_resolution = cover_resolution::resolve_cover(
        &inputs.current_work,
        livrarr_domain::CoverMediaType::Audiobook,
        &pm.audio,
        &eligible_providers,
        &outcomes_ref,
    );
    // 6. Status classification (REQ-019): Enriched iff >=1 meaningful text field
    // is present; otherwise Thin ("we know the book, found no info"). The cover
    // is a lazy backfill asset and never gates completion; title/author are
    // identity (present from creation) and are not an enrichment signal.
    let has_meaningful_text = merged_description.is_some()
        || get_str(WorkField::Subtitle).is_some()
        || get_str(WorkField::SeriesName).is_some()
        || get_strings(WorkField::Genres).is_some_and(|g| !g.is_empty())
        || get_str(WorkField::Publisher).is_some();
    let enrichment_status = if has_meaningful_text {
        EnrichmentStatus::Enriched
    } else {
        EnrichmentStatus::Thin
    };

    // 7. enrichment_source: comma-joined lowercased provider names.
    let enrichment_source = if contributing_providers.is_empty() {
        None
    } else {
        let names: Vec<&str> = contributing_providers
            .iter()
            .map(|p| provider_name(*p))
            .collect();
        Some(names.join(","))
    };

    let work_update = UpdateWorkEnrichmentDbRequest {
        title: get_str(WorkField::Title),
        subtitle: get_str(WorkField::Subtitle),
        original_title: get_str(WorkField::OriginalTitle),
        author_name: get_str(WorkField::AuthorName),
        description: merged_description,
        year: get_int(WorkField::Year),
        series_name: get_str(WorkField::SeriesName),
        series_position: get_float(WorkField::SeriesPosition),
        genres: get_strings(WorkField::Genres),
        language: get_str(WorkField::Language).map(|s| livrarr_domain::normalize_language(&s)),
        page_count: get_int(WorkField::PageCount),
        duration_seconds: get_int(WorkField::DurationSeconds),
        publisher: get_str(WorkField::Publisher),
        publish_date: get_str(WorkField::PublishDate),
        narrator: get_strings(WorkField::Narrator),
        narration_type: get_narration_type(WorkField::NarrationType),
        abridged: get_bool(WorkField::Abridged),
        rating: get_float(WorkField::Rating),
        rating_count: get_int(WorkField::RatingCount),
        enrichment_status,
        enrichment_source: enrichment_source.clone(),
        // REQ-006: persist the priority-resolved cover URL; fall back to the
        // existing cover when no provider supplied one (non-destructive) or when
        // the user locked it (cover_resolution is None above).
        cover_url: cover_resolution
            .as_ref()
            .map(|c| c.url.clone())
            .or_else(|| inputs.current_work.cover_url.clone()),
    };

    // 8. Materialize dissent rows (REQ-014): winning values come from the
    // final resolved state. merge_generation 0 is a placeholder — the
    // persisting caller stamps the applied generation (the engine has no
    // db-generation knowledge).
    let recorded_at = chrono::Utc::now();
    let dissents: Vec<livrarr_domain::FieldDissent> = dissent_seeds
        .into_iter()
        .map(
            |(provider, field, offered, reason)| livrarr_domain::FieldDissent {
                work_id,
                provider: provider.record_key().to_string(),
                field: work_field_name(field),
                offered_value: offered,
                winning_value: resolved_values.get(&field).and_then(field_value_text),
                reason,
                merge_generation: 0,
                recorded_at,
            },
        )
        .collect();

    // REQ-014: providers were attempted but ALL were excluded (conflicted or
    // policy-dropped) — there is nothing to write, the apply is status-only.
    // With ANY eligible provider — or an empty dispatch — the update is
    // emitted as before (the last-known-good echo preserves current values
    // under the db's direct binds).
    let work_update = if had_providers && eligible_providers.is_empty() {
        None
    } else {
        Some(MergeResolved::new(work_update))
    };

    Ok(MergeOutput {
        work_update,
        provenance_upserts,
        provenance_deletes,
        enrichment_status,
        enrichment_source,
        cover_resolution,
        audiobook_cover_resolution,
        dissents,
    })
}
