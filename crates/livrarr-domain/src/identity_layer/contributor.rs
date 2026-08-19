//! Contributor and work-shape-extension vocabulary. IR v1 domain module
//! (ir-v1-identity-layer-rewrite.yaml:809-847) and `contributor_policy`.

use serde::{Deserialize, Serialize};

use super::shared::{DefaultEdition, EvidenceProvenance, RouteKey, SourcedValue};

/// A single sourced role label for one `WorkContributor`. IR v1 does not give
/// `SourcedRole` its own field list; aliased to the same `value` +
/// `provenance` + `observed_at` shape used pervasively elsewhere in IR v1
/// for "sourced" evidence (e.g. `alternative_titles: Vec<SourcedValue<String>>`).
pub type SourcedRole = SourcedValue<String>;

/// Deterministic pre-id grouping key for co-created Author identities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributorPartition {
    pub normalized_full_author_identity_name: String,
    pub sorted_provider_route_set: Vec<String>,
    pub sorted_exact_source_name_set: Vec<String>,
}

/// Stable opaque Author reference carried by contributor-order review actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorRef(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkContributor {
    pub user_id: crate::UserId,
    pub work_id: crate::WorkId,
    pub author_id: crate::AuthorId,
    pub ordinal: u32,
    pub roles: Vec<SourcedRole>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkReference {
    Local(crate::WorkId),
    Unresolved(RouteKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkRelationshipKind {
    Contains,
    PartOf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkRelationship {
    pub user_id: crate::UserId,
    pub from_work_id: crate::WorkId,
    pub kind: WorkRelationshipKind,
    pub target: WorkReference,
    pub provenance: EvidenceProvenance,
}

/// Work-level identity-shape extensions (IR v1 amendment 2026-08-05, "Amend
/// Work"). `contributors` is documented `nonempty Vec<WorkContributor>`;
/// represented as a plain `Vec` (no non-empty-vec dependency in
/// `approved_libraries`) with the invariant enforced at construction time,
/// not by the type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkShapeExtensions {
    pub alternative_titles: Vec<SourcedValue<String>>,
    pub compilation: Option<SourcedValue<bool>>,
    /// Contains and part-of stored as reciprocal local rows when both Works exist.
    pub relationships: Vec<WorkRelationship>,
    pub canonical_pointer: Option<SourcedValue<WorkReference>>,
    pub default_editions: Vec<DefaultEdition>,
    pub contributors: Vec<WorkContributor>,
    pub subject_people: Vec<SourcedValue<String>>,
    pub subject_places: Vec<SourcedValue<String>>,
    pub subject_times: Vec<SourcedValue<String>>,
    pub subject_topics: Vec<SourcedValue<String>>,
}
