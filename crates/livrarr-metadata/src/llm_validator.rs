//! LLM-driven cross-provider identity verification + field conflict resolution.
//!
//! Sits between scatter-gather completion and merge-engine input in
//! `EnrichmentServiceImpl::enrich_work`. Reads each provider's payload along
//! with the work's locked identity anchor (title + author from
//! `setter=User` provenance), and decides per-provider:
//!   - Accept: payload describes the same work as the anchor — passes through
//!     to merge, possibly with individual fields nullified by the LLM
//!     (e.g. wrong-language description dropped).
//!   - Reject: payload describes a different work — outcome converts to
//!     `PermanentFailure` for this attempt; provider's other fields all
//!     dropped.
//!
//! When ALL `Success` outcomes are rejected, the EnrichmentService escalates
//! the work to `EnrichmentStatus::Conflict` (the IR's terminal "identity
//! drift detected" status, exit only via `reset_for_manual_refresh`).
//!
//! ## Privacy
//!
//! The prompt builder (see `build_prompt`) constructs the LLM input from:
//!   - The locked anchor (title + author + language) from the Work
//!   - Each provider's full normalized payload (all public metadata)
//!
//! It MUST NOT include:
//!   - work_id, user_id, author_id, or any internal DB identifier
//!   - filenames, paths, file sizes, checksums of Pete's media
//!   - import_id (could fingerprint Pete's external system)
//!   - detail_url (URL Pete chose to use — soft privacy concern)
//!   - added_at, monitor flags, or other Pete-private state
//!
//! See the project memory `feedback_llm_privacy.md` for the full rule set.
//!
//! ## LLM optionality
//!
//! Per project Principle 11, LLM is optional. If the user has not configured
//! an LLM endpoint/model/key, `EnrichmentServiceImpl` registers a `NoOpLlmValidator`
//! that returns inputs unchanged — the merge engine then runs as before this
//! work landed. With LLM configured, `GeminiLlmValidator` does the validation
//! call.

use std::collections::HashMap;
use std::time::Duration;

use livrarr_domain::{
    FieldProvenance, MetadataProvider, OutcomeClass, ProvenanceSetter, Work, WorkField,
};
use livrarr_http::HttpClient;
use serde::{Deserialize, Serialize};

use crate::{NormalizedWorkDetail, ReconstructedOutcome};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of an LLM validation pass over a single work's scatter-gather outcomes.
#[derive(Debug, Clone)]
pub struct ValidationOutcome {
    /// Per-provider reconstructed outcomes after LLM transformation:
    /// rejected providers have class=PermanentFailure with payload=None;
    /// accepted providers retain class=Success with payload optionally
    /// nullified field-by-field (e.g. wrong-language description dropped).
    pub reconstructed: HashMap<MetadataProvider, ReconstructedOutcome>,
    /// Per-provider rejection reasons (LLM's short explanation). Useful for
    /// surfacing in the work-detail UI when status=Conflict.
    pub rejections: HashMap<MetadataProvider, String>,
    /// True iff there was at least one provider in `Success` going INTO
    /// validation but ZERO surviving as `Success` AFTER. Signals work-level
    /// Conflict status.
    pub all_success_rejected: bool,
}

/// Errors from the LLM validator. Always recoverable — caller falls back
/// to passing the inputs through unmodified.
#[derive(Debug, thiserror::Error)]
pub enum LlmValidationError {
    #[error("LLM call timed out")]
    Timeout,
    #[error("LLM HTTP error: {0}")]
    Http(String),
    #[error("LLM returned malformed response: {0}")]
    MalformedResponse(String),
}

/// LLM validator trait. Implementations:
///   - `NoOpLlmValidator`: returns input unchanged. Used when LLM is not
///     configured.
///   - `GeminiLlmValidator`: calls Google's Gemini API with the validated
///     prompt+schema.
///
/// `trait_variant::make(Send)` per project convention.
#[trait_variant::make(Send)]
pub trait LlmValidator: Send + Sync {
    async fn validate(
        &self,
        work: &Work,
        provenance: &[FieldProvenance],
        reconstructed: HashMap<MetadataProvider, ReconstructedOutcome>,
    ) -> Result<ValidationOutcome, LlmValidationError>;
}

// ---------------------------------------------------------------------------
// EitherLlmValidator — enum dispatch, used by AppState
// ---------------------------------------------------------------------------

/// Concrete enum so the production `AppState` can hold the validator without
/// resorting to `Box<dyn>`. The trait is `trait_variant::make(Send)` and is
/// therefore not dyn-compatible (same constraint as `ProviderClient`).
#[derive(Clone)]
pub enum EitherLlmValidator {
    NoOp(NoOpLlmValidator),
    Gemini(GeminiLlmValidator),
}

impl EitherLlmValidator {
    pub fn noop() -> Self {
        Self::NoOp(NoOpLlmValidator::new())
    }

    pub fn gemini(http: HttpClient, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self::Gemini(GeminiLlmValidator::new(http, api_key, model))
    }
}

impl LlmValidator for EitherLlmValidator {
    async fn validate(
        &self,
        work: &Work,
        provenance: &[FieldProvenance],
        reconstructed: HashMap<MetadataProvider, ReconstructedOutcome>,
    ) -> Result<ValidationOutcome, LlmValidationError> {
        match self {
            Self::NoOp(v) => v.validate(work, provenance, reconstructed).await,
            Self::Gemini(v) => v.validate(work, provenance, reconstructed).await,
        }
    }
}

// ---------------------------------------------------------------------------
// NoOpLlmValidator
// ---------------------------------------------------------------------------

/// Pass-through validator. Returns inputs unchanged. Used when LLM is not
/// configured — preserves legacy behavior (priority-based merge with no
/// identity check).
#[derive(Clone, Default)]
pub struct NoOpLlmValidator;

impl NoOpLlmValidator {
    pub fn new() -> Self {
        Self
    }
}

impl LlmValidator for NoOpLlmValidator {
    async fn validate(
        &self,
        _work: &Work,
        _provenance: &[FieldProvenance],
        reconstructed: HashMap<MetadataProvider, ReconstructedOutcome>,
    ) -> Result<ValidationOutcome, LlmValidationError> {
        Ok(ValidationOutcome {
            reconstructed,
            rejections: HashMap::new(),
            all_success_rejected: false,
        })
    }
}

// ---------------------------------------------------------------------------
// GeminiLlmValidator
// ---------------------------------------------------------------------------

/// Gemini-backed LLM validator. Uses the Gemini API directly via HTTPS.
///
/// Default model is `gemini-3.1-flash-lite-preview` per session-12 empirical
/// validation (~1.5-2s per call; strong quality on identity-mismatch and
/// language-guard cases).
#[derive(Clone)]
pub struct GeminiLlmValidator {
    http: HttpClient,
    api_key: String,
    model: String,
    timeout: Duration,
}

impl GeminiLlmValidator {
    pub fn new(http: HttpClient, api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            http,
            api_key: api_key.into(),
            model: model.into(),
            timeout: Duration::from_millis(2_500),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl LlmValidator for GeminiLlmValidator {
    async fn validate(
        &self,
        work: &Work,
        provenance: &[FieldProvenance],
        mut reconstructed: HashMap<MetadataProvider, ReconstructedOutcome>,
    ) -> Result<ValidationOutcome, LlmValidationError> {
        // Skip validation when there is no User-set anchor to validate
        // against. AutoAdded works run unanchored — no LLM call.
        if !has_user_anchor(provenance) {
            return Ok(ValidationOutcome {
                reconstructed,
                rejections: HashMap::new(),
                all_success_rejected: false,
            });
        }

        // Skip when there are no Success payloads to validate.
        let success_provider_count = reconstructed
            .values()
            .filter(|o| o.class == OutcomeClass::Success && o.payload.is_some())
            .count();
        if success_provider_count == 0 {
            return Ok(ValidationOutcome {
                reconstructed,
                rejections: HashMap::new(),
                all_success_rejected: false,
            });
        }

        let prompt_input = build_prompt(work, &reconstructed);
        let response = call_gemini(
            &self.http,
            &self.api_key,
            &self.model,
            &prompt_input,
            self.timeout,
        )
        .await?;

        let mut rejections: HashMap<MetadataProvider, String> = HashMap::new();
        let mut surviving_success = 0usize;

        for (provider_str, verdict) in &response.providers {
            let Some(provider) = parse_provider(provider_str) else {
                continue;
            };
            let entry = reconstructed.get_mut(&provider);
            let Some(entry) = entry else { continue };

            // Only re-classify Success outcomes. NotFound / WillRetry / etc.
            // pass through unchanged regardless of the LLM's verdict.
            if entry.class != OutcomeClass::Success {
                continue;
            }

            match verdict.verdict.as_str() {
                "reject" => {
                    rejections.insert(provider, verdict.reason.clone());
                    entry.class = OutcomeClass::PermanentFailure;
                    entry.payload = None;
                }
                "accept" => {
                    surviving_success += 1;
                    // Apply per-field nullifications. The LLM's `merged`
                    // field for that provider may have certain fields set
                    // to null indicating "drop this field even though the
                    // payload is otherwise accepted" (e.g. wrong-language
                    // description).
                    if let (Some(payload), Some(per_field)) =
                        (entry.payload.as_mut(), verdict.dropped_fields.as_ref())
                    {
                        apply_dropped_fields(payload, per_field);
                    }
                }
                // "absent" or anything else — unchanged.
                _ => {
                    surviving_success += 1;
                }
            }
        }

        let all_success_rejected = success_provider_count > 0 && surviving_success == 0;

        Ok(ValidationOutcome {
            reconstructed,
            rejections,
            all_success_rejected,
        })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn has_user_anchor(provenance: &[FieldProvenance]) -> bool {
    provenance.iter().any(|fp| {
        fp.setter == ProvenanceSetter::User
            && (fp.field == WorkField::Title || fp.field == WorkField::AuthorName)
            && !fp.cleared
    })
}

fn parse_provider(s: &str) -> Option<MetadataProvider> {
    match s {
        "hardcover" => Some(MetadataProvider::Hardcover),
        "openlibrary" => Some(MetadataProvider::OpenLibrary),
        "goodreads" => Some(MetadataProvider::Goodreads),
        "audnexus" => Some(MetadataProvider::Audnexus),
        _ => None,
    }
}

fn provider_key(p: MetadataProvider) -> &'static str {
    match p {
        MetadataProvider::Hardcover => "hardcover",
        MetadataProvider::OpenLibrary => "openlibrary",
        MetadataProvider::Goodreads => "goodreads",
        MetadataProvider::Audnexus => "audnexus",
        MetadataProvider::Llm => "llm",
    }
}

fn apply_dropped_fields(payload: &mut NormalizedWorkDetail, dropped: &[String]) {
    for field in dropped {
        match field.as_str() {
            "description" => payload.description = None,
            "year" => payload.year = None,
            "page_count" => payload.page_count = None,
            "publisher" => payload.publisher = None,
            "publish_date" => payload.publish_date = None,
            "isbn_13" => payload.isbn_13 = None,
            "asin" => payload.asin = None,
            "series_name" => payload.series_name = None,
            "series_position" => payload.series_position = None,
            "genres" => payload.genres = None,
            "language" => payload.language = None,
            "rating" => payload.rating = None,
            "rating_count" => payload.rating_count = None,
            "cover_url" => payload.cover_url = None,
            "subtitle" => payload.subtitle = None,
            "original_title" => payload.original_title = None,
            // title and author_name intentionally NOT droppable here — they
            // are the anchor fields and are immutable per design.
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Prompt construction (privacy-audited)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct PromptInput<'a> {
    anchor: AnchorBlock<'a>,
    providers: HashMap<&'static str, ProviderBlock>,
}

#[derive(Debug, Serialize)]
struct AnchorBlock<'a> {
    title: &'a str,
    author_name: &'a str,
    language: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct ProviderBlock {
    title: Option<String>,
    author: Option<String>,
    description: Option<String>,
    year: Option<i32>,
    page_count: Option<i32>,
    publisher: Option<String>,
    publish_date: Option<String>,
    isbn_13: Option<String>,
    asin: Option<String>,
    series_name: Option<String>,
    series_position: Option<f64>,
    genres: Option<Vec<String>>,
    language: Option<String>,
    rating: Option<f64>,
    rating_count: Option<i32>,
}

fn build_prompt<'a>(
    work: &'a Work,
    reconstructed: &HashMap<MetadataProvider, ReconstructedOutcome>,
) -> PromptInput<'a> {
    // PRIVACY AUDIT: the only Work fields read here are title, author_name,
    // and language — all of which are public metadata that the user
    // selected/typed at add-time. NO work_id, user_id, import_id,
    // detail_url, added_at, monitor flags, or filesystem paths.
    let anchor = AnchorBlock {
        title: &work.title,
        author_name: &work.author_name,
        language: work.language.as_deref(),
    };

    let mut providers: HashMap<&'static str, ProviderBlock> = HashMap::new();
    for (provider, outcome) in reconstructed {
        if outcome.class != OutcomeClass::Success {
            continue;
        }
        let Some(payload) = outcome.payload.as_ref() else {
            continue;
        };
        providers.insert(
            provider_key(*provider),
            ProviderBlock {
                title: payload.title.clone(),
                author: payload.author_name.clone(),
                description: payload.description.clone(),
                year: payload.year,
                page_count: payload.page_count,
                publisher: payload.publisher.clone(),
                publish_date: payload.publish_date.clone(),
                isbn_13: payload.isbn_13.clone(),
                asin: payload.asin.clone(),
                series_name: payload.series_name.clone(),
                series_position: payload.series_position,
                genres: payload.genres.clone(),
                language: payload.language.clone(),
                rating: payload.rating,
                rating_count: payload.rating_count,
            },
        );
    }

    PromptInput { anchor, providers }
}

// ---------------------------------------------------------------------------
// Gemini API call
// ---------------------------------------------------------------------------

const GEMINI_API_BASE: &str = "https://generativelanguage.googleapis.com/v1beta/models";

const SYSTEM_INSTRUCTION: &str = r#"You are a metadata enrichment validator for a book library system.

INPUT: a LOCKED IDENTITY ANCHOR (title + author the user validated at add-time, immutable) plus per-provider PAYLOADS from Hardcover, OpenLibrary, Goodreads, Audnexus.

YOUR JOB:
1. For each provider, decide if its payload is about the SAME WORK as the anchor. Tolerate cosmetic differences (capitalization, punctuation, series suffix in title, edition markers, "Last, First" vs "First Last" in author). REJECT if the title/author indicates a DIFFERENT WORK (different book entirely, different author, wrong-edition cases like a series-mate substituted for the anchor work).
2. For accepted providers: optionally nullify individual fields in the payload that are clearly wrong (e.g. description in a language different from the anchor language hint — drop it; LLM-detected language mismatch on subtitle — drop it). Surface dropped field names in `dropped_fields`. The provider's other fields pass through.
3. NEVER suggest changes to title or author — they are locked.

OUTPUT JSON SCHEMA:
- providers: per-provider verdict
  - verdict: "accept" or "reject"
  - reason: short human-readable string
  - dropped_fields: array of field names to nullify (only meaningful when verdict=accept)
- notes: brief flags about anomalies"#;

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    providers: HashMap<String, ProviderVerdict>,
    #[allow(dead_code)]
    #[serde(default)]
    notes: String,
}

#[derive(Debug, Deserialize)]
struct ProviderVerdict {
    verdict: String,
    reason: String,
    #[serde(default)]
    dropped_fields: Option<Vec<String>>,
}

async fn call_gemini(
    http: &HttpClient,
    api_key: &str,
    model: &str,
    prompt_input: &PromptInput<'_>,
    timeout: Duration,
) -> Result<GeminiResponse, LlmValidationError> {
    let url = format!("{GEMINI_API_BASE}/{model}:generateContent?key={api_key}");
    let user_msg = serde_json::to_string(prompt_input)
        .map_err(|e| LlmValidationError::MalformedResponse(format!("prompt encode: {e}")))?;

    let body = serde_json::json!({
        "systemInstruction": {"parts": [{"text": SYSTEM_INSTRUCTION}]},
        "contents": [{"role": "user", "parts": [{"text": user_msg}]}],
        "generationConfig": {
            "temperature": 0.0,
            "responseMimeType": "application/json",
            "responseSchema": {
                "type": "OBJECT",
                "properties": {
                    "providers": {
                        "type": "OBJECT",
                        "properties": {
                            "hardcover":   {"type": "OBJECT", "properties": {"verdict": {"type": "STRING", "enum": ["accept","reject"]}, "reason": {"type": "STRING"}, "dropped_fields": {"type": "ARRAY", "items": {"type": "STRING"}, "nullable": true}}, "required": ["verdict","reason"]},
                            "openlibrary": {"type": "OBJECT", "properties": {"verdict": {"type": "STRING", "enum": ["accept","reject"]}, "reason": {"type": "STRING"}, "dropped_fields": {"type": "ARRAY", "items": {"type": "STRING"}, "nullable": true}}, "required": ["verdict","reason"]},
                            "goodreads":   {"type": "OBJECT", "properties": {"verdict": {"type": "STRING", "enum": ["accept","reject"]}, "reason": {"type": "STRING"}, "dropped_fields": {"type": "ARRAY", "items": {"type": "STRING"}, "nullable": true}}, "required": ["verdict","reason"]},
                            "audnexus":    {"type": "OBJECT", "properties": {"verdict": {"type": "STRING", "enum": ["accept","reject"]}, "reason": {"type": "STRING"}, "dropped_fields": {"type": "ARRAY", "items": {"type": "STRING"}, "nullable": true}}, "required": ["verdict","reason"]}
                        }
                    },
                    "notes": {"type": "STRING"}
                },
                "required": ["providers"]
            }
        }
    });

    let resp = tokio::time::timeout(
        timeout,
        http.post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send(),
    )
    .await
    .map_err(|_| LlmValidationError::Timeout)?
    .map_err(|e| LlmValidationError::Http(format!("send: {e}")))?;

    if !resp.status().is_success() {
        return Err(LlmValidationError::Http(format!(
            "status {}",
            resp.status()
        )));
    }

    let envelope: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| LlmValidationError::MalformedResponse(format!("envelope parse: {e}")))?;

    let content_str = envelope
        .pointer("/candidates/0/content/parts/0/text")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            LlmValidationError::MalformedResponse(
                "missing candidates[0].content.parts[0].text".to_string(),
            )
        })?;

    serde_json::from_str::<GeminiResponse>(content_str).map_err(|e| {
        LlmValidationError::MalformedResponse(format!("inner parse: {e} | body: {content_str}"))
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use livrarr_domain::{EnrichmentStatus, MetadataProvider, NarrationType};

    fn fake_work(title: &str, author: &str, language: Option<&str>) -> Work {
        Work {
            id: 0,
            user_id: 0,
            title: title.to_string(),
            sort_title: None,
            subtitle: None,
            original_title: None,
            author_name: author.to_string(),
            author_id: None,
            description: None,
            year: None,
            series_id: None,
            series_name: None,
            series_position: None,
            genres: None,
            language: language.map(String::from),
            page_count: None,
            duration_seconds: None,
            publisher: None,
            publish_date: None,
            ol_key: None,
            hc_key: None,
            gr_key: None,
            isbn_13: None,
            asin: None,
            narrator: None,
            narration_type: None as Option<NarrationType>,
            abridged: false,
            rating: None,
            rating_count: None,
            enrichment_status: EnrichmentStatus::Pending,
            enrichment_retry_count: 0,
            enriched_at: None,
            enrichment_source: None,
            cover_url: None,
            cover_manual: false,
            monitor_ebook: false,
            monitor_audiobook: false,
            import_id: None,
            added_at: chrono::Utc::now(),
            metadata_source: None,
            detail_url: None,
        }
    }

    fn fake_provenance(field: WorkField, setter: ProvenanceSetter) -> FieldProvenance {
        FieldProvenance {
            user_id: 1,
            work_id: 1,
            field,
            source: None,
            set_at: chrono::Utc::now(),
            setter,
            cleared: false,
        }
    }

    fn fake_payload() -> NormalizedWorkDetail {
        NormalizedWorkDetail {
            title: Some("Anything".to_string()),
            subtitle: None,
            original_title: None,
            author_name: Some("Anyone".to_string()),
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

    #[tokio::test]
    async fn noop_validator_passes_through_unchanged() {
        let work = fake_work("Title", "Author", Some("en"));
        let prov = vec![fake_provenance(WorkField::Title, ProvenanceSetter::User)];
        let mut reconstructed = HashMap::new();
        reconstructed.insert(
            MetadataProvider::Hardcover,
            ReconstructedOutcome {
                class: OutcomeClass::Success,
                payload: Some(fake_payload()),
            },
        );

        let validator = NoOpLlmValidator::new();
        let result = validator
            .validate(&work, &prov, reconstructed.clone())
            .await
            .unwrap();
        assert_eq!(result.reconstructed.len(), 1);
        assert_eq!(
            result
                .reconstructed
                .get(&MetadataProvider::Hardcover)
                .unwrap()
                .class,
            OutcomeClass::Success
        );
        assert!(result.rejections.is_empty());
        assert!(!result.all_success_rejected);
    }

    #[test]
    fn has_user_anchor_detects_user_set_title() {
        let prov = vec![
            fake_provenance(WorkField::Title, ProvenanceSetter::User),
            fake_provenance(WorkField::Year, ProvenanceSetter::Provider),
        ];
        assert!(has_user_anchor(&prov));
    }

    #[test]
    fn has_user_anchor_rejects_only_provider_set() {
        let prov = vec![
            fake_provenance(WorkField::Title, ProvenanceSetter::Provider),
            fake_provenance(WorkField::AuthorName, ProvenanceSetter::AutoAdded),
        ];
        assert!(!has_user_anchor(&prov));
    }

    #[test]
    fn has_user_anchor_rejects_cleared_user_field() {
        let mut fp = fake_provenance(WorkField::Title, ProvenanceSetter::User);
        fp.cleared = true;
        assert!(!has_user_anchor(&[fp]));
    }

    #[test]
    fn build_prompt_includes_anchor_and_provider_payloads() {
        let work = fake_work("Caliban's War", "James S.A. Corey", Some("en"));
        let mut reconstructed = HashMap::new();
        let mut payload = fake_payload();
        payload.title = Some("Caliban's War (Expanse #2)".to_string());
        payload.year = Some(2012);
        reconstructed.insert(
            MetadataProvider::Hardcover,
            ReconstructedOutcome {
                class: OutcomeClass::Success,
                payload: Some(payload),
            },
        );

        let p = build_prompt(&work, &reconstructed);
        assert_eq!(p.anchor.title, "Caliban's War");
        assert_eq!(p.anchor.author_name, "James S.A. Corey");
        assert_eq!(p.anchor.language, Some("en"));
        assert!(p.providers.contains_key("hardcover"));
        let hc = p.providers.get("hardcover").unwrap();
        assert_eq!(hc.title.as_deref(), Some("Caliban's War (Expanse #2)"));
        assert_eq!(hc.year, Some(2012));
    }

    #[test]
    fn apply_dropped_fields_nullifies_listed_fields_only() {
        let mut payload = fake_payload();
        payload.description = Some("desc".to_string());
        payload.year = Some(2020);
        payload.page_count = Some(300);

        apply_dropped_fields(
            &mut payload,
            &["description".to_string(), "year".to_string()],
        );

        assert!(payload.description.is_none());
        assert!(payload.year.is_none());
        assert_eq!(payload.page_count, Some(300));
    }

    #[test]
    fn apply_dropped_fields_does_not_drop_title_or_author() {
        let mut payload = fake_payload();
        payload.title = Some("orig".to_string());
        payload.author_name = Some("auth".to_string());

        apply_dropped_fields(
            &mut payload,
            &["title".to_string(), "author_name".to_string()],
        );

        assert!(
            payload.title.is_some(),
            "title is intentionally not droppable"
        );
        assert!(
            payload.author_name.is_some(),
            "author_name is intentionally not droppable"
        );
    }
}
