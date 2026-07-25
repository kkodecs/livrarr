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

/// How a resolver matched a given anchor. `Hard` — the provider's raw record
/// shares a hard identifier the seed already carries, so the anchor is safe to
/// sync into `works.*`. `Fuzzy` — the anchor came from a title/author match
/// only, so it is held as a pending guess until the user affirms it (REQ-004).
/// An in-memory routing signal from the resolver to the harvest write-step; the
/// ledger persists `AnchorConfidence`/`AnchorSetter`, not this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchBasis {
    Hard,
    Fuzzy,
}

/// Per-anchor [`MatchBasis`] sidecar produced alongside a [`CapturedIdentity`]
/// by the resolver's cluster projection. Mirrors the five anchor fields of
/// `CapturedIdentity`; `None` means the captured identity carries no such
/// anchor. Rides only on the harvest-writing resolution verdicts.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AnchorProvenance {
    pub ol_key: Option<MatchBasis>,
    pub gr_key: Option<MatchBasis>,
    pub hc_key: Option<MatchBasis>,
    pub isbn_13: Option<MatchBasis>,
    pub asin: Option<MatchBasis>,
}

/// A durable per-(work, anchor) retry counter: how many background convergence
/// attempts have failed to obtain a missing anchor. At or above the configured
/// threshold the anchor is a dead-end and is no longer chased; a successful
/// harvest clears the counter (REQ-009).
#[derive(Debug, Clone, PartialEq)]
pub struct AnchorDeadEnd {
    pub work_id: WorkId,
    pub anchor_type: AnchorType,
    pub attempt_count: u32,
    pub last_attempt_at: DateTime<Utc>,
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
        /// Per-anchor hard-vs-fuzzy basis for `identity`, computed by the cluster
        /// projection. Drives the harvest write-step's safe/guessed split
        /// (REQ-003/004): hard anchors sync to `works.*`, fuzzy ones are held.
        provenance: AnchorProvenance,
    },
    /// Tier B/C — no resolving hard id; each candidate carries its own
    /// `candidate_id`.
    NeedsConfirmation { candidates: Vec<Candidate> },
    /// Terminal — a quorum tie or a conflicting same-kind anchor.
    Conflict {
        conflict: NewIdentityConflict,
        captured: CapturedIdentity,
        /// Captured anchors of every tied cluster at a quorum tie (Q-008), so a
        /// settled work can be checked for a contradiction sitting on a
        /// non-representative cluster (AC-018). `captured` stays the
        /// representative; `tied` carries the full set. Empty when not a tie.
        tied: Vec<CapturedIdentity>,
    },
    /// Transient — a provider abstained; `Some(candidate_id)` if any payload was
    /// fetched so the create can still reuse it. Converges later.
    Unresolved {
        captured: CapturedIdentity,
        reason: PendingReason,
        candidate_id: Option<CandidateId>,
        /// Per-anchor basis for the partial anchors `captured` carries (an
        /// Unresolved verdict still harvests only-missing anchors). Default
        /// (all-`None`) when nothing was captured.
        provenance: AnchorProvenance,
    },
}

/// Caller patience for the identity engine (`settle_identity`) — the only
/// caller-visible knob (REQ-005). Exactly two modes (Q-003): `Interactive`
/// (a person is waiting) and `Background` (unattended). Maps onto `LatencyTier`
/// for the resolve call (Interactive→Interactive, Background→Background); the
/// resolver's `Bulk` tier is deferred background pacing (§4), not a mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityMode {
    Interactive,
    Background,
}

/// Which raw `Resolution` verdict the engine acted on, for audit/logging
/// (REQ-008). The engine maps the raw four variants (ST-002), never the legacy
/// `LowConfidence` / `PendingReason` buckets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolverVerdictKind {
    Resolved,
    NeedsConfirmation,
    Conflict,
    Unresolved,
}

/// Audit-only report of a `settle_identity` run (REQ-008): the badge before and
/// after, which anchor types were newly merged, and which verdict drove it
/// (`None` when the terminal guard skipped resolution — REQ-006). The engine has
/// already performed every write; no caller persists anything from this report.
#[derive(Debug, Clone, PartialEq)]
pub struct IdentityReport {
    pub prior_status: crate::IdentityStatus,
    pub final_status: crate::IdentityStatus,
    pub anchors_merged: Vec<String>,
    pub verdict: Option<ResolverVerdictKind>,
    /// The post-resolve completion lost its `identity_generation` claim — a
    /// user edit/clear (or another identity writer) won while the provider
    /// await was in flight. Nothing was written; the caller discards this
    /// resolution, re-reads the work, and skips its dead-end accounting.
    pub superseded: bool,
}

/// The shared identity outcome a creation path receives from
/// [`crate::services::WorkService::resolve_identity`]: the resolved badge plus
/// the bits a door needs to finish building its `WorkCandidate`. Every door uses
/// this instead of hand-deriving identity, so all paths agree on what a book IS
/// (P1 / REQ-014). It collapses the four [`Resolution`] verdicts into what a
/// caller actually needs.
#[derive(Debug, Clone)]
pub struct ResolvedIdentity {
    /// The badge to place on the `WorkCandidate` — `Confirmed` only when the
    /// resolver corroborated a work anchor; otherwise `Pending`.
    pub identity: IdentityState,
    /// Payload-cache handle from this resolve, so `add()` can reuse the fetched
    /// payloads without a second network round (REQ-015). `None` when nothing was
    /// fetched.
    pub candidate_id: Option<CandidateId>,
    /// The language the resolver determined, if any. A door SHOULD prefer this
    /// over a hardcoded default so a foreign work is never stamped English (#8).
    pub language: Option<String>,
    /// `Some` when the resolver raised an identity conflict. The door decides how
    /// to surface it — the interactive add shows the existing work; batch paths
    /// skip or notify. `identity` is left `Pending` in this case.
    pub conflict: Option<NewIdentityConflict>,
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

    /// Derive the persisted identity-confidence badge ([`crate::IdentityStatus`])
    /// from this resolution state's confirmed anchors (REQ-014/016, D-013): a work
    /// anchor (OL/GR/HC work key) is `Confirmed`; an ISBN/ASIN bridge with no work
    /// anchor is the de-facto `Provisional`; otherwise `Pending`. A `Pending`
    /// resolution stays `Pending` even when it carries unresolved seed anchors —
    /// only a `Confirmed` resolution yields a settled badge. `Conflict` and
    /// `NeedsReview` are written by their own paths (an open conflict row;
    /// resolution exhaustion), not derived from a fresh candidate.
    pub fn derived_identity_status(&self) -> crate::IdentityStatus {
        let nonempty = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.is_empty());
        match self.anchors() {
            Some(a) if nonempty(&a.ol_key) || nonempty(&a.gr_key) || nonempty(&a.hc_key) => {
                crate::IdentityStatus::Confirmed
            }
            Some(a) if nonempty(&a.isbn_13) || nonempty(&a.asin) => {
                crate::IdentityStatus::Provisional
            }
            _ => crate::IdentityStatus::Pending,
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
    /// The user-triggered "retry incomplete" re-processing path (insertion C).
    ManualRetry,
    /// The background convergence loop re-processing a work.
    Convergence,
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
    /// The creation door that built this candidate (the `added` event's source
    /// label). Stamped exclusively by the per-door `seed_*` constructors in
    /// [`crate::seed`] — callers never pass it, so a door cannot mislabel itself.
    pub add_source: crate::history_events::WorkAddSource,
}
