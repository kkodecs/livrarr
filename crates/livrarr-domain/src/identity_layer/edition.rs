//! Edition — first-class record owned by exactly one Work. IR v1 domain
//! module (ir-v1-identity-layer-rewrite.yaml:762-783) and IR v1 amendments
//! (2026-08-05, "Add Edition to the spine").

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::route::{IdentityProvider, WorkRoute};
use super::shared::{EditionId, FileRevision, SourcedValue};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditionFormat {
    Ebook,
    Audiobook,
    Physical,
    Other(String),
    Unknown,
}

/// The persisted two-state edition lifecycle flag. Distinct from the finer
/// `ActiveUnknown | ActiveKnown | EvidenceReview | Archived` conceptual view
/// documented in IR v1 `data_flow.state_machines` (entity: Edition), which is
/// a derived read over `format` + open `EditionEvidence` review cards, not a
/// second stored value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditionState {
    Active,
    Archived,
}

/// No `PartialEq`: `crate::CoverCandidate` (existing, additive-only) does not
/// implement it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edition {
    pub id: EditionId,
    pub user_id: crate::UserId,
    pub work_id: crate::WorkId,
    pub format: EditionFormat,
    pub language: Option<String>,
    pub subtitle: Option<SourcedValue<String>>,
    /// Every member has `owner = Edition(id)`.
    pub routes: Vec<WorkRoute>,
    pub covers: Vec<crate::CoverCandidate>,
    pub source_provider: Option<IdentityProvider>,
    pub provider_edition_id: Option<String>,
    pub state: EditionState,
}

/// A sanitized transient reason an EPUB revision could not be inspected.
/// The string is a stable code, never a raw path or parser error.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectionError(pub String);

/// The four-way local EPUB embedded-cover inspection outcome (ST-007).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EmbeddedCoverInspectionResult {
    Extracted {
        revision: FileRevision,
        bytes: Vec<u8>,
        media_type: String,
    },
    VerifiedNoCover {
        revision: FileRevision,
    },
    CouldNotInspect {
        revision: FileRevision,
        error: InspectionError,
    },
    FileGone,
}

/// The durable, byte-free projection of [`EmbeddedCoverInspectionResult`].
/// A separate enum (rather than reusing the result type) because the
/// invariant text is explicit that extracted bytes never enter the
/// persisted record — only `cover_candidate_id` points at the retained
/// materialized candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EmbeddedCoverInspectionOutcome {
    Extracted,
    VerifiedNoCover,
    CouldNotInspect,
    FileGone,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbeddedCoverInspectionRecord {
    pub user_id: crate::UserId,
    pub library_item_id: crate::LibraryItemId,
    pub revision: FileRevision,
    pub outcome: EmbeddedCoverInspectionOutcome,
    /// Points to retained materialized candidate, never stores cover bytes.
    pub cover_candidate_id: Option<i64>,
    pub sanitized_error_code: Option<String>,
    pub inspected_at: DateTime<Utc>,
}

/// Domain boundary for revision-exact local EPUB cover inspection.
#[trait_variant::make(Send)]
pub trait EmbeddedCoverInspector: Send + Sync {
    async fn inspect_revision(
        &self,
        item: crate::LibraryItem,
        revision: FileRevision,
        force: bool,
    ) -> Result<EmbeddedCoverInspectionResult, InspectionServiceError>;
}

/// Service-level error for [`EmbeddedCoverInspector::inspect_revision`].
/// Distinct from the per-inspection [`InspectionError`] payload carried
/// inside [`EmbeddedCoverInspectionResult::CouldNotInspect`].
#[derive(Debug, Clone, thiserror::Error)]
pub enum InspectionServiceError {
    #[error("database error: {0}")]
    Database(String),
    #[error("cancelled")]
    Cancelled,
}
