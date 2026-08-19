//! P3 guard separation, P8 text classification, and author-route inheritance.
//! IR v1 domain module (ir-v1-identity-layer-rewrite.yaml:937-983).

use std::collections::{BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

/// Opaque probe identifier (e.g. `"PROBE-ST006-E"`). IR v1 uses bare
/// `PROBE-ST006-*` strings as informal type annotations for opaque-type
/// private fields; represented as a lightweight id newtype.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ProbeId(pub String);

/// Lightweight identifier for one sampled text signal (distinct from the
/// opaque [`SampledTextSignal`] capability token itself).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SampledSignalId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostMatchGuardSet {
    pub one_sided_subtitle_recovery: bool,
    pub shared_edition_id_confirmation: bool,
    pub translation_same_text_signals: HashSet<SampledSignalId>,
}

/// Independently tunable guard against merging different canonical mains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MainTitleGuard(pub bool);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WrongMergeGuardSet {
    pub main_title_guard: MainTitleGuard,
    pub volume_conflict_guard: bool,
    pub author_disagreement_guard: bool,
    pub work_key_contradiction_guard: bool,
    pub audited_different_text_guard: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextSignalClass {
    Translation,
    Abridgement,
    Adaptation,
    StudyGuide,
    Omnibus,
}

/// Opaque, non-`Deserialize` capability: only provider adapters backed by
/// the accepted PROBE-ST006-E registry may construct one (FP-008). Fields
/// are private so only accepted provider-adapter probes can mint the token.
#[derive(Debug, Clone, Serialize)]
pub struct SampledTextSignal {
    probe_id: ProbeId,
    provider_path: String,
    class: TextSignalClass,
}

/// Opaque, non-`Deserialize` capability: only an adapter arm backed by
/// accepted PROBE-ST006-F evidence may construct one (FP-007).
#[derive(Debug, Clone, Serialize)]
pub struct AliasEquivalenceProof {
    provider: super::route::IdentityProvider,
    work_ids: BTreeSet<String>,
    probe_id: ProbeId,
}

#[derive(Debug, Clone, Serialize)]
pub enum TextIdentityVerdict {
    SameText(SampledTextSignal),
    DifferentText(SampledTextSignal),
    ReviewRequired,
}

/// Comparison input shaped as `CapturedIdentity` minus its
/// operational bookkeeping (`user_id`/`status`/`identity_generation`) — the
/// content actually being compared.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkIdentityEvidence {
    pub title: super::title::IdentityTitleTuple,
    pub primary_author_id: crate::AuthorId,
    pub routes: Vec<super::route::WorkRoute>,
}

/// Bundle of the four per-aspect verdicts in
/// `crate::identity_matching` (`title_verdict`/`author_verdict`/
/// `language_verdict`/`id_verdict`) used by directional policy.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DirectionalMatchVerdicts {
    pub title: crate::identity_matching::TitleVerdict,
    pub author: crate::identity_matching::AuthorVerdict,
    pub language: crate::identity_matching::LanguageVerdict,
    pub id: crate::identity_matching::IdVerdict,
}

/// F1 primary-author-route inheritance outcome. Reuses the existing
/// `crate::author_link` vocabulary (`RouteWriteOutcome`, `AuthorLinkCandidate`)
/// rather than redefining it.
#[derive(Debug, Clone)]
pub enum AuthorInheritanceOutcome {
    Linked(crate::RouteWriteOutcome),
    F1ReviewCandidate(crate::AuthorLinkCandidate),
    NoAuthorId,
}
