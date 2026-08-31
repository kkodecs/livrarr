//! Identity route capture for the add door: resolve a work's identity via
//! the multi-provider resolver and return the captured routes when the
//! full-length-match gate passes. Free-function, dependency-passing style
//! (no resolver struct exists).

use livrarr_domain::identity::{CapturedIdentity, IdentityMode, LatencyTier, Resolution, WorkSeed};
use livrarr_domain::identity_matching::{self, AuthorVerdict, IdEvidence, TitleVerdict};
use livrarr_domain::services::WorkIdentityError;
use livrarr_domain::{UserId, Work};

use crate::english_identity_resolver::EnglishIdentityResolver;

fn seed_from_work(work: &Work) -> WorkSeed {
    WorkSeed {
        ol_key: work.ol_key.clone(),
        gr_key: work.gr_key.clone(),
        hc_key: work.hc_key.clone(),
        isbn_13: work.isbn_13.clone(),
        asin: work.asin.clone(),
        title: Some(work.title.clone()),
        author_name: Some(work.author_name.clone()),
        language: work.language.clone(),
        series_name: work.series_name.clone(),
        year: work.year,
        user_confirmed: false,
    }
}

/// Capture the provider fan-out result for the F2 road without performing any
/// legacy scalar-anchor, badge, generation, conflict, or review write.
/// Ambiguous/conflicting results carry no routes across this seam; the next
/// convergence visit may retry them. Resolved and transient-unresolved
/// captures must still pass the shared title/author match gate.
pub async fn capture_identity_routes<R: EnglishIdentityResolver>(
    resolver: &R,
    user_id: UserId,
    work: &Work,
    mode: IdentityMode,
) -> Result<Option<CapturedIdentity>, WorkIdentityError> {
    let tier = match mode {
        IdentityMode::Interactive => LatencyTier::Interactive,
        IdentityMode::Background => LatencyTier::Background,
    };
    let resolution = resolver
        .resolve(user_id, &seed_from_work(work), tier)
        .await?;
    let captured = match resolution {
        Resolution::Resolved { identity, .. } => identity,
        Resolution::Unresolved { captured, .. } => captured,
        Resolution::NeedsConfirmation { .. } | Resolution::Conflict { .. } => return Ok(None),
    };
    Ok(flm_match(work, &captured).then_some(captured))
}

/// FLM (Fuzzy Livrarr Match): may the resolved identity's anchors auto-merge
/// onto the seed work? Routes through the one matching authority
/// (`title_id_trust`): exact-main title equality, or a one-sided-subtitle grey
/// corroborated by an independently agreeing hard ID (AC-004); a same-provider
/// work-key contradiction never merges. The author must Agree — an authorless
/// identity never auto-merges (today's equality bar, via the authority).
fn flm_match(work: &Work, identity: &CapturedIdentity) -> bool {
    if identity.title.is_empty() || identity.author_name.is_empty() {
        return false;
    }
    let title = identity_matching::title_verdict(
        &identity_matching::parse_title(&work.title),
        &identity_matching::parse_title(&identity.title),
    );
    let work_evidence = IdEvidence {
        ol_key: work.ol_key.as_deref(),
        gr_key: work.gr_key.as_deref(),
        hc_key: work.hc_key.as_deref(),
        isbn_13: work.isbn_13.as_deref(),
        asin: work.asin.as_deref(),
    };
    let identity_evidence = IdEvidence {
        ol_key: identity.ol_key.as_deref(),
        gr_key: identity.gr_key.as_deref(),
        hc_key: identity.hc_key.as_deref(),
        isbn_13: identity.isbn_13.as_deref(),
        asin: identity.asin.as_deref(),
    };
    if !identity_matching::title_id_trust(&title, &identity_evidence, &work_evidence) {
        if let TitleVerdict::Grey { cause, .. } = title {
            tracing::debug!(?cause, work_id = work.id, "flm declined grey identity");
        }
        return false;
    }
    matches!(
        identity_matching::author_verdict(
            std::slice::from_ref(&identity.author_name),
            std::slice::from_ref(&work.author_name),
        ),
        AuthorVerdict::Agree
    )
}
