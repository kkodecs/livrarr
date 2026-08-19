//! Identity-layer-rewrite (F2) thin adapter over `livrarr-domain`'s matching
//! authority — replaces the V5/V8 dedup and V7 fast-cover title comparisons'
//! private sameness recipes (FP-010). IR v1 `livrarr-matching` module
//! (ir-v1-identity-layer-rewrite.yaml:1238-1249).
//!
//! DELIBERATE SHADOW NAME: `crate::work_dedup::find_matching_work` already
//! exists with an unrelated, incompatible signature
//! (`fn(&[Work], &str, &str, &ProviderKeys) -> Option<&Work>` — the exact
//! V5-era recipe this IR is retiring). This module's `find_matching_work` is
//! a *different* free function at a different path
//! (`livrarr_matching::identity_layer::find_matching_work`); neither is
//! re-exported at the crate root, so there is no ambiguity. See
//! STUBS-REPORT.md.

use livrarr_domain::identity_layer::{
    evaluate_match, DirectionalMatchVerdicts, LostMatchGuardSet, MainTitleGuard,
    WorkIdentityEvidence, WrongMergeGuardSet,
};

/// IR v1 names `WorkMatchAuthorityInputs` without a field list. The V5/V7/V8
/// call sites this module replaces are all pairwise "is this the same work"
/// comparisons, so this wraps the same left/right evidence
/// `evaluate_match` takes. See STUBS-REPORT.md.
#[derive(Debug, Clone)]
pub struct WorkMatchAuthorityInputs {
    pub left: WorkIdentityEvidence,
    pub right: WorkIdentityEvidence,
}

/// IR v1 names `MatchAuthorityOutcome` without a field list. Wraps the raw
/// verdicts plus the boolean the V5/V7/V8 call sites actually branch on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MatchAuthorityOutcome {
    pub verdicts: DirectionalMatchVerdicts,
    pub is_match: bool,
}

pub fn find_matching_work(authority_inputs: WorkMatchAuthorityInputs) -> MatchAuthorityOutcome {
    // INFERRED: The laid-down pairwise adapter input omitted the IR's caller-supplied
    // guard sets. Use the ratified strict guards so this adapter owns no threshold
    // and all sameness still comes from the domain matching authority.
    let verdicts = evaluate_match(
        authority_inputs.left,
        authority_inputs.right,
        LostMatchGuardSet {
            one_sided_subtitle_recovery: true,
            shared_edition_id_confirmation: true,
            translation_same_text_signals: Default::default(),
        },
        WrongMergeGuardSet {
            main_title_guard: MainTitleGuard(true),
            volume_conflict_guard: true,
            author_disagreement_guard: true,
            work_key_contradiction_guard: true,
            audited_different_text_guard: true,
        },
    );
    let is_match = matches!(
        verdicts.title,
        livrarr_domain::identity_matching::TitleVerdict::Same
    ) && matches!(
        verdicts.author,
        livrarr_domain::identity_matching::AuthorVerdict::Agree
    ) && !matches!(
        verdicts.id,
        livrarr_domain::identity_matching::IdVerdict::WorkKeyContradiction
    );

    MatchAuthorityOutcome { verdicts, is_match }
}
