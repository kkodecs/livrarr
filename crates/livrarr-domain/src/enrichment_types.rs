//! Metadata-enrichment and merge-engine vocabulary: provider identity, cover
//! candidates, field provenance, merge outcomes and dissent, and a few
//! adjacent value types (external id kind, queue progress/summary, log-surface
//! status).

use crate::entities::{UserId, WorkId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Which metadata provider produced a given field value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetadataProvider {
    Hardcover,
    OpenLibrary,
    Goodreads,
    Audnexus,
    Llm,
    /// Source data from a Readarr import. Treated as another provider
    /// input in the merge engine — ranked above OL, below GR.
    Readarr,
    GoogleBooks,
    Audible,
}

impl MetadataProvider {
    /// Canonical snake_case provider key used in call records and retry state
    /// (the REQ-001 reporting vocabulary) — never a display name.
    pub fn record_key(self) -> &'static str {
        match self {
            Self::Hardcover => "hardcover",
            Self::OpenLibrary => "openlibrary",
            Self::Goodreads => "goodreads",
            Self::Audnexus => "audnexus",
            Self::Llm => "llm",
            Self::Readarr => "readarr",
            Self::GoogleBooks => "google_books",
            Self::Audible => "audible",
        }
    }
}

/// Which cover slot: ebook or audiobook.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CoverMediaType {
    Ebook,
    Audiobook,
}

impl CoverMediaType {
    pub fn suffix(&self) -> &'static str {
        match self {
            CoverMediaType::Ebook => "",
            CoverMediaType::Audiobook => "_audio",
        }
    }
}

/// Source of a cover candidate — wraps MetadataProvider for standard providers,
/// adds EPUB and ISBN-based sources that don't participate in enrichment infra.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoverCandidateSource {
    Provider(MetadataProvider),
    Epub,
    IsbnOl,
    IsbnAmazon,
}

impl std::fmt::Display for CoverCandidateSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Provider(p) => write!(f, "{p:?}"),
            Self::Epub => write!(f, "epub"),
            Self::IsbnOl => write!(f, "isbn_ol"),
            Self::IsbnAmazon => write!(f, "isbn_amazon"),
        }
    }
}

/// A cover candidate with browser-safe proxied URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverCandidate {
    pub candidate_id: String,
    pub proxy_url: String,
    pub source: String,
    pub media_type: CoverMediaType,
    pub width: u32,
    pub height: u32,
    pub passes_quality_gate: bool,
}

/// Internal cover candidate with raw provider URL — never serialize to browser.
#[derive(Debug, Clone)]
pub struct InternalCoverCandidate {
    pub source: CoverCandidateSource,
    pub url: String,
    pub media_type: CoverMediaType,
    pub edition_title: Option<String>,
}

/// Request to select a cover from alternatives.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectCoverRequest {
    pub candidate_id: String,
    pub media_type: CoverMediaType,
}

/// Result of a cover resolution during enrichment merge.
#[derive(Debug, Clone)]
pub struct CoverResolution {
    pub url: String,
    pub source: String,
    pub media_type: CoverMediaType,
}

/// A named work field that can have per-provider provenance tracked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkField {
    Title,
    SortTitle,
    Subtitle,
    OriginalTitle,
    AuthorName,
    Description,
    Year,
    SeriesName,
    SeriesPosition,
    Genres,
    Language,
    PageCount,
    DurationSeconds,
    Publisher,
    PublishDate,
    OlKey,
    HcKey,
    GrKey,
    Isbn13,
    Asin,
    Narrator,
    NarrationType,
    Abridged,
    Rating,
    RatingCount,
    CoverUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceSetter {
    /// Field value was set by a metadata provider during enrichment.
    Provider,
    /// Field value was directly set or selected by the user (typing it,
    /// picking from search results, manually editing). Acts as the
    /// identity-lock anchor for LLM validation — providers returning data
    /// inconsistent with a User-set field have their payload rejected.
    User,
    /// Field value was set by the system in a contextless way (e.g.
    /// system-assigned defaults). Not a lock anchor.
    System,
    /// Field value originated from an automated add path (author-monitor
    /// auto-add or series auto-add) where the user did not per-work
    /// validate. Honest about provenance — NOT treated as a lock anchor
    /// for LLM identity verification. A user-confirm UX (future) can
    /// transition AutoAdded → User on confirm.
    AutoAdded,
    /// Field value originated from a bulk list import (CSV upload).
    Imported,
    /// Field value originated from an external system import (e.g., Readarr).
    /// Treated as provider-quality data, not user-sovereign.
    Import,
}

/// Provenance record for a single field value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldProvenance {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub field: WorkField,
    pub source: Option<MetadataProvider>,
    pub set_at: DateTime<Utc>,
    pub setter: ProvenanceSetter,
    pub cleared: bool,
}

/// Priority of a metadata request, used for queue ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RequestPriority {
    Low,
    Normal,
    High,
    Interactive,
}

/// Whether an enrichment pass may satisfy provider fetches from the
/// persistent provider-response cache (REQ-009). Orthogonal to
/// [`RequestPriority`]: priority orders the outbound queue, freshness decides
/// whether a fetch consults the cache at all (D-004).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// A fresh cached payload (age < TTL) satisfies the fetch with zero
    /// provider HTTP. Background flows: convergence, re-adds, list import,
    /// monitors.
    PreferCache,
    /// Ignore cached payloads; make the real fetch and overwrite the cache
    /// entry. User-triggered Refresh / Refresh All.
    Bypass,
}

/// Normalization class for a field or work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormalizationClass {
    /// Rich text fields (HTML, markdown).
    RichText,
    /// Plain display text fields.
    DisplayText,
    /// Structured identifier fields.
    Identifier,
    /// Work-level: English-language merge strategy.
    English,
    /// Work-level: foreign-language merge strategy.
    ForeignLanguage,
}

/// Outcome class returned by a provider for a single field or whole work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeClass {
    /// Provider returned a usable value.
    Success,
    /// Provider returned no match for this work.
    NotFound,
    /// Provider is not configured — retriable when config changes.
    NotConfigured,
    /// Provider returned data that will be retried.
    WillRetry,
    /// Provider returned an error that will not resolve on retry.
    PermanentFailure,
    /// Provider returned data that conflicts with existing confirmed data.
    Conflict,
}

/// Per-field / per-provider merge dissent (REQ-013/014): an excluded
/// contribution, recorded queryably. A dissent isolates at provider or field
/// granularity and never discards the work's merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldDissent {
    pub work_id: WorkId,
    /// Canonical provider key (e.g. "google_books").
    pub provider: String,
    /// Work column name. Payload-level dissent records one row per affected
    /// field.
    pub field: String,
    pub offered_value: String,
    pub winning_value: Option<String>,
    pub reason: DissentReason,
    pub merge_generation: i64,
    pub recorded_at: DateTime<Utc>,
}

/// Why a contribution was excluded from the merge (REQ-013/014).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DissentReason {
    /// The provider appears to describe a different book.
    PayloadMismatch,
    /// Contradictory value for one field.
    FieldConflict,
    /// Known-incompatible payload language on a foreign work (REQ-013).
    LanguageIncompatible,
}

/// Anchor-only enrichment fetch vocabulary (REQ-006). The input type admits no
/// title or author, so no enrichment fetch can construct a text search
/// (AC-007). Text search exists only behind the lookup/identity surfaces.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AnchorQuery {
    Isbn13(String),
    GrKey(String),
    HcKey(String),
    OlKey(String),
    Asin(String),
}

/// Tracing-init outcome surfaced on the status page (REQ-003): the daily
/// rolling file actually written and any init failure — never swallowed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LogSurfaceStatus {
    /// The daily rolling file the appender actually writes.
    pub active_path: std::path::PathBuf,
    /// Log-dir creation/write failure captured at startup (#102's vector).
    pub init_error: Option<String>,
}

impl OutcomeClass {
    pub fn is_phase2_terminal(&self) -> bool {
        matches!(
            self,
            OutcomeClass::Success
                | OutcomeClass::NotFound
                | OutcomeClass::PermanentFailure
                | OutcomeClass::Conflict
                | OutcomeClass::NotConfigured
        )
    }

    pub fn can_merge(&self) -> bool {
        matches!(
            self,
            OutcomeClass::Success | OutcomeClass::NotFound | OutcomeClass::PermanentFailure
        )
    }

    pub fn all_can_merge(outcomes: &[OutcomeClass]) -> bool {
        outcomes.iter().all(|o| o.can_merge())
    }
}

/// Reason a provider will be retried later.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WillRetryReason {
    Timeout,
    RateLimit,
    ServerError,
    AntiBotBlock,
    /// The outbound queue's breaker was Open for this provider's bucket
    /// (R-11): a pause, not a step toward a retry-budget dead-end — must
    /// never consume the attempt budget.
    CircuitOpen,
    /// The outbound queue's admission cap rejected the request (D3): same
    /// class as `CircuitOpen` — a local, transport-level pause, never a
    /// provider verdict — so it must never consume the attempt budget
    /// either.
    QueueFull,
}

/// Reason a provider permanently failed for this work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermanentFailureReason {
    ProviderPanic,
    RetryBudgetExhausted,
    InvalidResponse,
    Unsupported,
    IdentityMismatch,
}

/// Result of applying an enrichment merge to the work record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApplyMergeOutcome {
    Applied,
    NoChange,
    Deferred,
    Superseded,
}

/// A resolved value from a merge (newtype wrapper).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeResolved<T>(pub T);

impl<T> MergeResolved<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn into_inner(self) -> T {
        self.0
    }

    pub fn as_inner(&self) -> &T {
        &self.0
    }

    pub fn as_inner_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

/// Typed external identifier kind for a work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalIdType {
    Isbn10,
    Isbn13,
    Asin,
    OpenLibraryWork,
    OpenLibraryEdition,
    GoodreadsBook,
    HardcoverBook,
    GoogleBooksVolume,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueProgress {
    pub percent: f64,
    pub eta: Option<i64>,
    pub download_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueSummary {
    pub total: i64,
    pub downloading: i64,
    pub importing: i64,
}
