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

impl AuthorRouteKey {
    pub fn parse(provider: AuthorProvider, raw: &str) -> Result<Self, AuthorLinkError> {
        todo!()
    }
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

#[derive(Debug, Clone)]
pub struct AgreedAuthorRouteEvidence {
    evidence: AuthorRouteEvidence,
    agree_proof: AuthorNameGuardAgree,
}

impl AgreedAuthorRouteEvidence {
    pub fn evidence(&self) -> &AuthorRouteEvidence {
        todo!()
    }
}

#[derive(Debug, Clone)]
pub struct RejectedAuthorRouteEvidence {
    evidence: AuthorRouteEvidence,
    verdict: AuthorVerdict,
}

impl RejectedAuthorRouteEvidence {
    pub fn evidence(&self) -> &AuthorRouteEvidence {
        todo!()
    }

    pub fn verdict(&self) -> AuthorVerdict {
        todo!()
    }
}

#[derive(Debug, Clone)]
pub enum AuthorRouteGuardResult {
    Agreed(AgreedAuthorRouteEvidence),
    Rejected(RejectedAuthorRouteEvidence),
}

pub fn guard_author_route(
    current_display_names: &[String],
    provider_ref: ProviderAuthorRef,
    evidence_work_id: Option<WorkId>,
    source: AuthorRouteEvidenceSource,
) -> AuthorRouteGuardResult {
    todo!()
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
