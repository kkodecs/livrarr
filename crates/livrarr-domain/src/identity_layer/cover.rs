//! Cover placeholder/presentation/selection vocabulary. IR v1 domain module
//! (ir-v1-identity-layer-rewrite.yaml:994-1000) and `cover_policy`/`ui_contract`.

use serde::{Deserialize, Serialize};

/// Work-scoped (`FormatNeeded`) and slot-scoped (the other three) states in
/// one closed set, matching `ui_contract.cover_placeholders` exactly
/// (`work_scoped: Cover-found-format-needed`,
/// `slot_scoped: [Nowhere-to-look, Searching-for-a-cover, No-cover-found]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CoverPlaceholderState {
    FormatNeeded {
        candidates: Vec<crate::CoverCandidate>,
    },
    NowhereToLook,
    Searching,
    NoCoverFound,
}

/// One `cover_policy.target_slots` slot's presentation: the selected
/// candidate (if any) plus its placeholder state (if uncovered).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverSlotPresentation {
    pub selected: Option<crate::CoverCandidate>,
    pub placeholder: Option<CoverPlaceholderState>,
}

/// Two format slots plus at most one shared format-needed panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkCoverPresentation {
    /// FP-014: at most one work-scoped panel, shared by both slots.
    pub format_needed: Option<CoverPlaceholderState>,
    pub ebook: CoverSlotPresentation,
    pub audiobook: CoverSlotPresentation,
}

/// Materialization-ready cover selections and explicit cross-format fallback.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkCoverSelection {
    pub ebook: Option<crate::CoverCandidate>,
    pub audiobook: Option<crate::CoverCandidate>,
    pub audiobook_is_ebook_fallback: bool,
}
