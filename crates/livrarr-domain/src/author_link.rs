use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::identity_matching::AuthorVerdict;
use crate::{Author, AuthorId, IdentityStatus, UserId, WorkId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorProvider {
    OpenLibrary,
    Goodreads,
    Hardcover,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OpenLibraryAuthorKey(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GoodreadsAuthorId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HardcoverAuthorId(u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorRouteKey {
    OpenLibrary(OpenLibraryAuthorKey),
    Goodreads(GoodreadsAuthorId),
    Hardcover(HardcoverAuthorId),
}

/// The OpenLibrary author-URL forms that reduce to a canonical author key.
const OPEN_LIBRARY_URL_PREFIXES: [&str; 4] = [
    "https://openlibrary.org",
    "http://openlibrary.org",
    "https://www.openlibrary.org",
    "http://www.openlibrary.org",
];

impl OpenLibraryAuthorKey {
    /// The canonical `OL<number>A` form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl GoodreadsAuthorId {
    /// The canonical positive decimal id.
    pub fn get(&self) -> u64 {
        self.0
    }
}

impl HardcoverAuthorId {
    /// The canonical positive decimal id.
    pub fn get(&self) -> u64 {
        self.0
    }
}

impl AuthorRouteKey {
    /// Parse a raw provider author-route value into its canonical typed form.
    ///
    /// Only the aliases documented for each provider are accepted. Malformed,
    /// zero, overflowed, and cross-provider values are rejected here, so no raw
    /// string reaches route uniqueness, tombstone lookup, or a route consumer.
    pub fn parse(provider: AuthorProvider, raw: &str) -> Result<Self, AuthorLinkError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(AuthorLinkError::InvalidRoute(
                "author route value is empty".to_string(),
            ));
        }
        match provider {
            AuthorProvider::OpenLibrary => {
                parse_open_library_author_key(trimmed).map(Self::OpenLibrary)
            }
            AuthorProvider::Goodreads => parse_numeric_author_id(provider, trimmed)
                .map(|id| Self::Goodreads(GoodreadsAuthorId(id))),
            AuthorProvider::Hardcover => parse_numeric_author_id(provider, trimmed)
                .map(|id| Self::Hardcover(HardcoverAuthorId(id))),
        }
    }

    /// The provider this route addresses.
    pub fn provider(&self) -> AuthorProvider {
        match self {
            Self::OpenLibrary(_) => AuthorProvider::OpenLibrary,
            Self::Goodreads(_) => AuthorProvider::Goodreads,
            Self::Hardcover(_) => AuthorProvider::Hardcover,
        }
    }

    /// The canonical stored value. Every alias of the same identifier parses to
    /// the same string, so this is what route uniqueness, tombstone lookup, and
    /// route consumers compare.
    pub fn value(&self) -> String {
        match self {
            Self::OpenLibrary(key) => key.as_str().to_string(),
            Self::Goodreads(id) => id.get().to_string(),
            Self::Hardcover(id) => id.get().to_string(),
        }
    }
}

/// `OL<number>A`, reached either directly, through an `/authors/` path, or
/// through an author URL that may carry a trailing display slug.
fn parse_open_library_author_key(trimmed: &str) -> Result<OpenLibraryAuthorKey, AuthorLinkError> {
    let path = OPEN_LIBRARY_URL_PREFIXES
        .iter()
        .find_map(|prefix| trimmed.strip_prefix(prefix))
        .unwrap_or(trimmed);
    let candidate = match path.strip_prefix("/authors/") {
        Some(after_prefix) => after_prefix.split('/').next().unwrap_or(after_prefix),
        None => path,
    };
    let digits = candidate
        .strip_prefix("OL")
        .and_then(|rest| rest.strip_suffix('A'))
        .filter(|digits| is_canonical_decimal(digits))
        .ok_or_else(|| {
            AuthorLinkError::InvalidRoute(format!(
                "{trimmed:?} is not an OpenLibrary author key (expected OL<number>A)"
            ))
        })?;
    if digits.parse::<u64>().is_err() {
        return Err(AuthorLinkError::InvalidRoute(format!(
            "{trimmed:?} exceeds the supported OpenLibrary author key range"
        )));
    }
    Ok(OpenLibraryAuthorKey(candidate.to_string()))
}

/// A positive decimal provider id. Leading zeros are an alias of the same id,
/// so the parsed number is the canonical value.
fn parse_numeric_author_id(
    provider: AuthorProvider,
    trimmed: &str,
) -> Result<u64, AuthorLinkError> {
    if !trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return Err(AuthorLinkError::InvalidRoute(format!(
            "{trimmed:?} is not a {provider:?} author id (expected a decimal number)"
        )));
    }
    match trimmed.parse::<u64>() {
        Ok(0) => Err(AuthorLinkError::InvalidRoute(format!(
            "{provider:?} author id must be greater than zero"
        ))),
        Ok(id) => Ok(id),
        Err(_) => Err(AuthorLinkError::InvalidRoute(format!(
            "{trimmed:?} exceeds the supported {provider:?} author id range"
        ))),
    }
}

/// A decimal run with no leading zero, which is how a canonical provider
/// identifier is written.
fn is_canonical_decimal(digits: &str) -> bool {
    !digits.is_empty() && !digits.starts_with('0') && digits.bytes().all(|b| b.is_ascii_digit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorRouteState {
    Active,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorRouteProvenance {
    LegacyUnguarded,
    Tier1Inherited,
    ReadarrGuarded,
    UserPicked,
    MergeCoalesced,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorRoute {
    pub id: i64,
    pub user_id: UserId,
    pub author_id: AuthorId,
    pub key: AuthorRouteKey,
    pub state: AuthorRouteState,
    pub provenance: AuthorRouteProvenance,
    pub evidence_work_id: Option<WorkId>,
    pub created_at: DateTime<Utc>,
    pub verified_at: Option<DateTime<Utc>>,
    pub removed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorNameSource {
    User,
    Goodreads,
    Hardcover,
    GoogleBooks,
    OpenLibrary,
    Readarr,
    Import,
    Legacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenLibraryNameRole {
    Primary,
    Alias,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorNameVariant {
    pub id: i64,
    pub user_id: UserId,
    pub author_id: AuthorId,
    pub name: String,
    pub source: AuthorNameSource,
    pub source_route_id: Option<i64>,
    pub open_library_role: Option<OpenLibraryNameRole>,
    pub user_selected_at: Option<DateTime<Utc>>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorCandidateCatalogState {
    Pending,
    Partial,
    Retrying,
    Complete,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorCandidateAlternateNameEvidence {
    pub name: String,
    pub verdict: AuthorVerdict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorLinkState {
    Linked,
    NeedsReview,
    Unlinked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorLinkProgressState {
    Queued,
    Running,
    ParkedNoSettledWork,
    ParkedNoEvidence,
    NeedsReview,
    Linked,
    RetryableFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorLinkCandidateReason {
    Tier2NameSearch,
    NameGuardFailed,
    ReadarrNameGuardFailed,
    Tombstoned,
    LegacyContradiction,
    OwnershipCollision,
    InvalidLegacyRoute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorRouteEvidenceSource {
    Tier1SettledWork,
    ReadarrImport,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorRouteEvidence {
    pub key: AuthorRouteKey,
    pub observed_name: String,
    pub evidence_work_id: Option<WorkId>,
    pub source: AuthorRouteEvidenceSource,
}

#[derive(Debug, Clone)]
struct AuthorNameGuardAgree;

impl AuthorNameGuardAgree {
    /// The witness carries no data — holding one *is* the assertion that
    /// [`guard_author_route`] returned `Agree`. Presenting it is how a route
    /// writer discharges the one standard of proof.
    fn presented(&self) {}
}

#[derive(Debug, Clone)]
pub struct AgreedAuthorRouteEvidence {
    evidence: AuthorRouteEvidence,
    agree_proof: AuthorNameGuardAgree,
}

impl AgreedAuthorRouteEvidence {
    pub fn evidence(&self) -> &AuthorRouteEvidence {
        &self.evidence
    }

    /// Consume the capability and hand back the evidence it proves.
    ///
    /// Taking `self` by value is what makes the guarded route writer's contract
    /// mechanical: it must hold — and give up — the private witness only
    /// [`guard_author_route`] can mint, so no caller can route unguarded
    /// evidence by asserting `Agree` on its own authority.
    pub fn into_agreed_evidence(self) -> AuthorRouteEvidence {
        self.agree_proof.presented();
        self.evidence
    }
}

#[derive(Debug, Clone)]
pub struct RejectedAuthorRouteEvidence {
    evidence: AuthorRouteEvidence,
    verdict: AuthorVerdict,
}

impl RejectedAuthorRouteEvidence {
    pub fn evidence(&self) -> &AuthorRouteEvidence {
        &self.evidence
    }

    pub fn verdict(&self) -> AuthorVerdict {
        self.verdict
    }
}

#[derive(Debug, Clone)]
pub enum AuthorRouteGuardResult {
    Agreed(AgreedAuthorRouteEvidence),
    Rejected(RejectedAuthorRouteEvidence),
}

/// The one standard of proof for an automatic author-route write.
///
/// The canonical [`crate::identity_matching::author_verdict`] authority compares
/// the provider's name for one author identifier against the author's full
/// current associated-name snapshot. `Agree` is the only verdict that mints
/// [`AgreedAuthorRouteEvidence`]; `Grey`, `Abstain`, and `Disagree` are retained
/// as review evidence with the verdict that produced them.
pub fn guard_author_route(
    current_display_names: &[String],
    provider_ref: ProviderAuthorRef,
    evidence_work_id: Option<WorkId>,
    source: AuthorRouteEvidenceSource,
) -> AuthorRouteGuardResult {
    let evidence = AuthorRouteEvidence {
        key: provider_ref.key,
        observed_name: provider_ref.name.trim().to_string(),
        evidence_work_id,
        source,
    };
    let provider_side = [evidence.observed_name.clone()];
    match crate::identity_matching::author_verdict(&provider_side, current_display_names) {
        AuthorVerdict::Agree => AuthorRouteGuardResult::Agreed(AgreedAuthorRouteEvidence {
            evidence,
            agree_proof: AuthorNameGuardAgree,
        }),
        verdict => {
            AuthorRouteGuardResult::Rejected(RejectedAuthorRouteEvidence { evidence, verdict })
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderAuthorRef {
    pub key: AuthorRouteKey,
    pub name: String,
    pub role: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorLinkTrigger {
    LegacyBackfill,
    AuthorCreated,
    AuthorAdopted,
    UserReResolve,
    EvidenceFingerprintChanged,
    DisplayNameDirty,
    RetryDue,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorLinkCursor {
    Tier1 {
        key_attempt_id: i64,
    },
    Tier2Search,
    Tier2Catalog {
        candidate: OpenLibraryAuthorKey,
        page: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorEvidenceFingerprint {
    pub settled_work_count: u32,
    pub settled_provider_key_count: u32,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettledAuthorWork {
    pub work_id: WorkId,
    pub title: String,
    pub identity_status: IdentityStatus,
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SettledWorkProviderKey {
    pub work_id: WorkId,
    pub provider: AuthorProvider,
    pub work_route: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKeyAttemptState {
    Pending,
    Running,
    Succeeded,
    Retryable,
    SkippedNotConfigured,
    SkippedPermanent,
    ParkedLayoutDrift,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKeyAttemptOutcome {
    Succeeded,
    Retryable {
        error: String,
        next_attempt_at: DateTime<Utc>,
    },
    SkippedNotConfigured,
    SkippedPermanent {
        error: String,
    },
    ParkedLayoutDrift {
        error: String,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorKeyAttempt {
    pub id: i64,
    pub user_id: UserId,
    pub author_id: AuthorId,
    pub evidence_generation: i64,
    pub work_id: WorkId,
    pub provider: AuthorProvider,
    pub work_route: String,
    pub state: AuthorKeyAttemptState,
    pub claim_token: Option<Uuid>,
    pub attempt_count: u32,
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorRoadInput {
    pub author: Author,
    pub active_routes: Vec<AuthorRoute>,
    pub settled_works: Vec<SettledAuthorWork>,
    pub name_variants: Vec<AuthorNameVariant>,
    pub evaluated_fingerprint: Option<AuthorEvidenceFingerprint>,
    pub live_fingerprint: AuthorEvidenceFingerprint,
    pub display_name_generation: i64,
    pub display_name_dirty: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorLinkProgressUpdate {
    pub state: AuthorLinkProgressState,
    pub tier: Option<u8>,
    pub cursor: Option<AuthorLinkCursor>,
    pub evaluated_fingerprint: AuthorEvidenceFingerprint,
    pub evidence_generation: i64,
    pub next_attempt_at: DateTime<Utc>,
    pub last_error: Option<String>,
    pub display_name_generation: i64,
    pub display_name_dirty: bool,
    pub would_have_linked_at_090: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorLinkCandidateStatus {
    Pending,
    Dismissed,
    Picked,
    Superseded,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenLibraryCatalogPage {
    pub titles: Vec<String>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorProviderError {
    UnsupportedProvider,
    NotConfigured,
    Retryable {
        error: String,
        retry_not_before: Option<DateTime<Utc>>,
    },
    Permanent(String),
    LayoutDrift(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorLinkError {
    NotFound,
    RouteOwnedByOtherAuthor(AuthorId),
    InvalidRoute(String),
    ClaimLost,
    Database(String),
    Provider(AuthorProviderError),
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorSweepTickSummary {
    pub claimed: u32,
    pub evaluated: u32,
    pub unchanged_fingerprint: u32,
    pub failed: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorCompatibilityProjection {
    pub ol_key: Option<String>,
    pub gr_key: Option<String>,
    pub hc_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorMonitorTarget {
    pub author: Author,
    pub ol_routes: Vec<AuthorRoute>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderAuthorNameObservation {
    pub source: AuthorNameSource,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthorLinkCandidate {
    pub id: i64,
    pub author_id: AuthorId,
    pub key: AuthorRouteKey,
    pub candidate_name: String,
    pub reason: AuthorLinkCandidateReason,
    pub name_verdict: AuthorVerdict,
    pub primary_name_verdict: AuthorVerdict,
    pub alternate_name_evidence: Vec<AuthorCandidateAlternateNameEvidence>,
    pub top_work_preview: Option<String>,
    pub catalog_evidence_state: AuthorCandidateCatalogState,
    pub corroborated_title_count: u32,
    pub settled_work_count: u32,
    pub previously_removed: bool,
    pub status: AuthorLinkCandidateStatus,
    pub evidence_generation: i64,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorLinkProgress {
    pub author_id: AuthorId,
    pub user_id: UserId,
    pub state: AuthorLinkProgressState,
    pub tier: Option<u8>,
    pub cursor: Option<AuthorLinkCursor>,
    pub evaluated_fingerprint: Option<AuthorEvidenceFingerprint>,
    pub evidence_generation: i64,
    pub display_name_generation: i64,
    pub display_name_dirty: bool,
    pub attempt_count: u32,
    pub next_attempt_at: DateTime<Utc>,
    pub lease_token: Option<Uuid>,
    pub lease_expires_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub would_have_linked_at_090: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum RouteWriteOutcome {
    Attached(AuthorRoute),
    AlreadyActive(AuthorRoute),
    LegacyProvenanceUpgraded(AuthorRoute),
    ParkedTombstoned(AuthorLinkCandidate),
    ParkedLegacyContradiction(AuthorLinkCandidate),
    ParkedOwnershipCollision(AuthorLinkCandidate),
    RejectedByNameGuard(AuthorLinkCandidate),
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorLinkReview {
    pub author: Author,
    pub link_state: AuthorLinkState,
    pub routes: Vec<AuthorRoute>,
    pub candidates: Vec<AuthorLinkCandidate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthorSweepProgress {
    pub total: u64,
    pub completed: u64,
    pub queued: u64,
    pub running: u64,
    pub parked: u64,
    pub needs_review: u64,
    pub retryable_failures: u64,
    pub key_retryable: u64,
    pub key_skipped: u64,
    pub key_layout_drift: u64,
    pub would_have_linked_at_090: u64,
    pub oldest_due_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenLibraryAuthorCandidate {
    pub route_key: OpenLibraryAuthorKey,
    pub name: String,
    pub alternate_names: Vec<String>,
    pub top_work: Option<String>,
    pub work_count: Option<u32>,
}
