use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{MetadataProvider, UserId, WorkId};

// ---------------------------------------------------------------------------
// Anchor types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkIdentityAnchor {
    pub work_id: WorkId,
    pub anchor_type: AnchorType,
    pub anchor_value: String,
    pub confidence: AnchorConfidence,
    pub setter: AnchorSetter,
    pub set_at: DateTime<Utc>,
    pub superseded_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct AnchorType(String);

impl AnchorType {
    pub const OL_WORK: &str = "ol_work";
    pub const HC_WORK: &str = "hc_work";
    pub const ISBN_13: &str = "isbn_13";
    pub const ASIN: &str = "asin";
    pub const GR_WORK: &str = "gr_work";

    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnchorConfidence {
    Confirmed,
    Pending,
    Superseded,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnchorSetter {
    User,
    AutoIsbn,
    AutoSearch,
    Import,
    Redirect,
}

// ---------------------------------------------------------------------------
// Resolution types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolutionScore {
    pub title_jaccard: f64,
    pub author_overlap: u32,
    pub runner_up_delta: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    pub candidate_id: CandidateId,
    pub anchors: CapturedIdentity,
    pub cover_url: Option<String>,
    pub sources: Vec<MetadataProvider>,
    pub score: ResolutionScore,
    pub existing_work_id: Option<WorkId>,
}

#[derive(Debug, Clone)]
pub struct WorkSeed {
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
    pub isbn_13: Option<String>,
    pub asin: Option<String>,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub language: Option<String>,
    pub series_name: Option<String>,
    pub year: Option<i32>,
    /// The user's explicit pick — counts as the strongest vote in quorum.
    pub user_confirmed: bool,
}

/// Latency budget for an identity-resolution call. Determines discovery depth,
/// provider eligibility, and whether the caller blocks on background-only
/// providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatencyTier {
    /// Add Work and manual-import review — never blocks on background-only
    /// providers; an unresolved hard id falls through to confirmation.
    Interactive,
    /// List and Readarr import — per-item seconds budget; an unresolved item
    /// becomes identity-pending without a prompt.
    Bulk,
    /// Monitors and the async convergence resolver — unbounded.
    Background,
}

/// Opaque, browser-safe handle to the server-side cache of per-provider payloads
/// fetched during one resolution. User-scoped and consume-once.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CandidateId(pub String);

/// Federated identifier set for a work — the single home for both work anchors
/// (`ol_key`/`gr_key`/`hc_key`) and edition bridges (`isbn_13`/`asin`).
/// Materializes as [`WorkIdentityAnchor`] rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CapturedIdentity {
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
    pub isbn_13: Option<String>,
    pub asin: Option<String>,
    pub title: String,
    pub author_name: String,
    pub language: Option<String>,
}

impl CapturedIdentity {
    /// Fill any anchor or bridge slot this set lacks from `incoming`, never
    /// overwriting a slot already populated (convergence adds, never clobbers).
    pub fn merge_missing(&mut self, incoming: &CapturedIdentity) {
        if self.ol_key.is_none() {
            self.ol_key = incoming.ol_key.clone();
        }
        if self.gr_key.is_none() {
            self.gr_key = incoming.gr_key.clone();
        }
        if self.hc_key.is_none() {
            self.hc_key = incoming.hc_key.clone();
        }
        if self.isbn_13.is_none() {
            self.isbn_13 = incoming.isbn_13.clone();
        }
        if self.asin.is_none() {
            self.asin = incoming.asin.clone();
        }
    }
}

/// Raw, un-normalized signal a creation path harvested from its source, before
/// validation. [`WorkSeed::sanitized`] canonicalizes it into a [`WorkSeed`].
#[derive(Debug, Clone, Default)]
pub struct RawHarvest {
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
    pub isbn: Option<String>,
    pub asin: Option<String>,
    pub title: Option<String>,
    pub author_name: Option<String>,
    pub language: Option<String>,
    pub series_name: Option<String>,
    pub year: Option<i32>,
    pub user_confirmed: bool,
}

impl WorkSeed {
    /// Normalize and validate a raw harvest into a usable seed. A malformed
    /// identifier is dropped (treated as absent); a seed carrying no identifier
    /// and no title+author is rejected as empty.
    pub fn sanitized(raw: RawHarvest) -> Result<WorkSeed, crate::services::WorkIdentityError> {
        use crate::normalization::{
            normalize_asin, normalize_gr_key, normalize_isbn13, normalize_language, AsinNorm,
        };

        let isbn_from_field = raw.isbn.as_deref().and_then(normalize_isbn13);

        let (isbn_13, asin) = match raw.asin.as_deref() {
            Some(a) => match normalize_asin(a) {
                AsinNorm::Isbn13(i13) => (isbn_from_field.or(Some(i13)), None),
                AsinNorm::Asin(a) => (isbn_from_field, Some(a)),
                AsinNorm::Invalid => (isbn_from_field, None),
            },
            None => (isbn_from_field, None),
        };

        let gr_key = raw.gr_key.as_deref().and_then(normalize_gr_key);
        let ol_key = raw.ol_key.filter(|s| !s.trim().is_empty());
        let hc_key = raw.hc_key.filter(|s| !s.trim().is_empty());
        let language = raw.language.as_deref().and_then(normalize_language);

        let has_identifier = isbn_13.is_some()
            || asin.is_some()
            || gr_key.is_some()
            || ol_key.is_some()
            || hc_key.is_some();
        let has_title_author = raw.title.is_some() && raw.author_name.is_some();

        if !has_identifier && !has_title_author {
            return Err(crate::services::WorkIdentityError::EmptySeed);
        }

        Ok(WorkSeed {
            ol_key,
            gr_key,
            hc_key,
            isbn_13,
            asin,
            title: raw.title,
            author_name: raw.author_name,
            language,
            series_name: raw.series_name,
            year: raw.year,
            user_confirmed: raw.user_confirmed,
        })
    }
}

// A transient resolver verdict, not stored in bulk; the size spread between the
// payload-bearing variants and the lightweight ones is acceptable here.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Resolution {
    /// Tier A — a hard identifier fixed the identity; auto-match. `candidate_id`
    /// handles the payloads fetched this resolve, for `add()` to reuse.
    Resolved {
        identity: CapturedIdentity,
        method: IdentityMethod,
        candidate_id: CandidateId,
    },
    /// Tier B/C — no resolving hard id; each candidate carries its own
    /// `candidate_id`.
    NeedsConfirmation { candidates: Vec<Candidate> },
    /// Terminal — a quorum tie or a conflicting same-kind anchor.
    Conflict {
        conflict: NewIdentityConflict,
        captured: CapturedIdentity,
    },
    /// Transient — a provider abstained; `Some(candidate_id)` if any payload was
    /// fetched so the create can still reuse it. Converges later.
    Unresolved {
        captured: CapturedIdentity,
        reason: PendingReason,
        candidate_id: Option<CandidateId>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdentityState {
    Confirmed {
        anchors: CapturedIdentity,
        method: IdentityMethod,
        score: Option<ResolutionScore>,
    },
    Pending {
        reason: PendingReason,
        /// Source anchors seeded at create for a bulk/monitor path not yet
        /// cross-provider-resolved — persisted now (REQ-001/006), converged later
        /// (REQ-022). `None` for a genuinely identifier-less pending work.
        seed_anchors: Option<CapturedIdentity>,
        top_candidates: Vec<Candidate>,
    },
}

impl IdentityState {
    /// The confirmed federated anchor set, or `None` when identity is pending.
    pub fn anchors(&self) -> Option<&CapturedIdentity> {
        match self {
            IdentityState::Confirmed { anchors, .. } => Some(anchors),
            IdentityState::Pending { .. } => None,
        }
    }

    /// The anchors to persist at create: a Confirmed work's full set, or a
    /// Pending work's seed anchors (a bulk/monitor path that carries source
    /// identifiers but is not yet cross-provider-resolved). `None` when neither.
    pub fn seed_or_confirmed_anchors(&self) -> Option<&CapturedIdentity> {
        match self {
            IdentityState::Confirmed { anchors, .. } => Some(anchors),
            IdentityState::Pending { seed_anchors, .. } => seed_anchors.as_ref(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityMethod {
    UserSelected,
    IsbnDirect,
    TitleAuthorSearch,
    Redirect,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingReason {
    LowConfidence,
    NoCandidates,
    OlUnavailable,
    MalformedResponse,
}

// ---------------------------------------------------------------------------
// Conflict types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IdentityConflict {
    pub id: i64,
    pub user_id: UserId,
    pub existing_work_id: WorkId,
    pub kind: IdentityConflictKind,
    pub incoming: IncomingConflictPayload,
    pub raised_at: DateTime<Utc>,
    pub raised_by: ConflictSource,
    pub raised_source_path: Option<String>,
    pub status: ConflictStatus,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolution_action: Option<ConflictResolutionAction>,
    pub resolution_notes: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewIdentityConflict {
    pub user_id: UserId,
    pub existing_work_id: WorkId,
    pub kind: IdentityConflictKind,
    pub incoming: IncomingConflictPayload,
    pub raised_by: ConflictSource,
    pub raised_source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IncomingConflictPayload {
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
    pub isbn_13: Option<String>,
    pub asin: Option<String>,
    pub title: String,
    pub author_name: String,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub top_candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityConflictKind {
    IncomingDifferentOlKey,
    IncomingDifferentGrKey,
    IncomingDifferentHcKey,
    OlRedirectCollision,
    QuorumTie,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStatus {
    Open,
    Resolved,
    Dismissed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictSource {
    ManualAdd,
    ManualImport,
    ListImport,
    ReadarrImport,
    SeriesMonitor,
    AuthorMonitor,
    Refresh,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolutionAction {
    KeepExisting,
    AcceptSeparate,
    ReplaceAnchor,
    Merge,
}

// ---------------------------------------------------------------------------
// Consistency check output
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum ConsistencyDivergence {
    CacheAhead {
        work_id: WorkId,
        cache: Option<String>,
        anchor: Option<String>,
    },
    AnchorAhead {
        work_id: WorkId,
        anchor: String,
    },
}

// ---------------------------------------------------------------------------
// English work candidate (unified creation contract)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct WorkSeedFields {
    pub title: String,
    pub author_name: String,
    pub language: String,
    pub author_ol_key: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub detail_url: Option<String>,
    pub description: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct WorkCandidate {
    pub fields: WorkSeedFields,
    /// The sole authoritative home for all identifiers (work anchors and edition
    /// bridges). `WorkService::add` persists identifiers only from here.
    pub identity: IdentityState,
    /// Echoed from the chosen candidate / resolved result; lets `add()` reuse
    /// cached payloads. `None` means enrich from the network.
    pub candidate_id: Option<CandidateId>,
    pub source_provider_data: Option<crate::services::SourceProviderData>,
    pub file_path: Option<std::path::PathBuf>,
    pub delete_existing_after_import: bool,
    pub series_id: Option<i64>,
    pub monitor_ebook: Option<bool>,
    pub monitor_audiobook: Option<bool>,
    pub provenance_setter: Option<crate::ProvenanceSetter>,
    pub import_id: Option<String>,
    pub cover_manual: bool,
    pub skip_sync_enrichment: bool,
}
