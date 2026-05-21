use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{UserId, WorkId};

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
pub struct OlCandidate {
    pub ol_key: String,
    pub title: String,
    pub author: String,
    pub year: Option<i32>,
    pub title_jaccard: f64,
    pub author_overlap: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdentityResolution {
    Confirmed {
        ol_key: String,
        method: IdentityMethod,
        score: ResolutionScore,
    },
    Pending {
        reason: PendingReason,
        top_candidates: Vec<OlCandidate>,
    },
    Conflict {
        incoming_ol_key: String,
        existing_work_id: WorkId,
        kind: IdentityConflictKind,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum IdentityState {
    Confirmed {
        ol_key: String,
        method: IdentityMethod,
        score: Option<ResolutionScore>,
    },
    Pending {
        reason: PendingReason,
        top_candidates: Vec<OlCandidate>,
    },
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
    pub title: String,
    pub author_name: String,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub top_candidates: Vec<OlCandidate>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IdentityConflictKind {
    IncomingDifferentOlKey,
    OlRedirectCollision,
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
    AuthorMonitor,
    Refresh,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolutionAction {
    KeepExisting,
    AcceptSeparate,
    ReplaceOlKey,
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
pub struct EnglishSeedFields {
    pub title: String,
    pub author_name: String,
    pub language: String,
    pub author_ol_key: Option<String>,
    pub year: Option<i32>,
    pub cover_url: Option<String>,
    pub detail_url: Option<String>,
    pub isbn: Option<String>,
    pub asin: Option<String>,
    pub description: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct EnglishWorkCandidate {
    pub fields: EnglishSeedFields,
    pub identity: IdentityState,
    pub source_provider_data: Option<crate::services::SourceProviderData>,
    pub file_path: Option<std::path::PathBuf>,
    pub delete_existing_after_import: bool,
}
