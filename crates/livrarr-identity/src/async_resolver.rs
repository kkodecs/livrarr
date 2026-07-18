//! Background-tier convergence for identity-pending works + the conditional,
//! background-only LLM identity-verify (D-013/Q-001). Generalizes the
//! bulk_resolver / enrichment-retry machinery; mirrors that module's
//! free-function, dependency-passing style (no resolver struct exists).
//! See ir-v2 metadata-async-resolver (REQ-022/025/026).

use std::collections::HashMap;
use std::time::Duration;

use livrarr_domain::identity::{
    AnchorType, CapturedIdentity, ConflictSource, IdentityConflictKind, IdentityMode,
    IdentityReport, IncomingConflictPayload, LatencyTier, NewIdentityConflict, Resolution,
    ResolverVerdictKind, WorkSeed,
};
use livrarr_domain::identity_matching::{self, AuthorVerdict, IdEvidence, TitleVerdict};
use livrarr_domain::services::{
    LlmCallRequest, LlmCaller, LlmPurpose, WorkIdentityError, WorkIdentityRepository,
};
use livrarr_domain::{IdentityStatus, UserId, Work};

use crate::english_identity_resolver::EnglishIdentityResolver;

/// A captured value for a slot whose works column is already populated is
/// never held as a pending guess: that slot's identifier is the one identity
/// and enrichment run on (mirrors `merge_missing_anchors`' monotonic fill),
/// and offering it for affirmation would invite replacing a settled
/// identifier with an unverified one.
fn anchor_slot_occupied(work: &Work, anchor_type: &str) -> bool {
    let value = match anchor_type {
        AnchorType::OL_WORK => work.ol_key.as_deref(),
        AnchorType::GR_WORK => work.gr_key.as_deref(),
        AnchorType::HC_WORK => work.hc_key.as_deref(),
        AnchorType::ISBN_13 => work.isbn_13.as_deref(),
        AnchorType::ASIN => work.asin.as_deref(),
        _ => None,
    };
    value.is_some_and(|v| !v.is_empty())
}

/// Conditional background LLM identity validation (D-013/Q-001): consulted ONLY
/// when the deterministic layer left identity ambiguous — i.e. no corroborating
/// work anchor. A multi-anchor identity needs no LLM check, so no call is made
/// (REQ-017: the LLM is never required). Returns a conflict to raise on a
/// confident mismatch, else `None`.
pub async fn llm_identity_verify<L: LlmCaller>(
    llm: &L,
    user_id: UserId,
    work: &Work,
    captured: &CapturedIdentity,
) -> Option<NewIdentityConflict> {
    // Deterministically corroborated (a work anchor is present) ⇒ nothing to
    // adjudicate; do not consult the LLM (D-013).
    let has_anchor =
        captured.ol_key.is_some() || captured.gr_key.is_some() || captured.hc_key.is_some();
    if has_anchor {
        return None;
    }

    let request = LlmCallRequest {
        system_template: "Validate whether the candidate identity is the same work. \
             Reply JSON: {\"identity_valid\": <bool>}."
            .to_string(),
        user_template: format!(
            "Work: \"{}\" by {}. Candidate: \"{}\" by {}.",
            work.title, work.author_name, captured.title, captured.author_name
        ),
        context: HashMap::new(),
        allowed_fields: &[],
        timeout: Duration::from_secs(30),
        purpose: LlmPurpose::IdentityValidation,
    };

    // LLM unavailable / unparseable ⇒ cannot adjudicate ⇒ no conflict (degrade
    // gracefully, REQ-017). Only a confident "invalid" verdict raises one.
    match llm.call(request).await {
        Ok(response) => {
            let valid = serde_json::from_str::<serde_json::Value>(&response.content)
                .ok()
                .and_then(|v| v.get("identity_valid").and_then(|b| b.as_bool()))
                .unwrap_or(true);
            if valid {
                None
            } else {
                Some(NewIdentityConflict {
                    user_id,
                    existing_work_id: work.id,
                    kind: IdentityConflictKind::QuorumTie,
                    incoming: incoming_from_captured(captured),
                    raised_by: ConflictSource::Refresh,
                    raised_source_path: None,
                })
            }
        }
        Err(_) => None,
    }
}

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

/// The one identity authority (REQ-001): resolve a work's identity, map the
/// verdict × current badge to a final `IdentityStatus` monotonically (REQ-003/004),
/// perform the badge + anchor writes itself (REQ-008), and return an audit
/// `IdentityReport`. Respects terminal states (REQ-006), idempotent (REQ-007),
/// never terminalizes a transient `Unresolved` (ST-002), writes zero enrichment
/// (REQ-004). It is the one identity road (REQ-001): every resolving door + the
/// background convergence loop routes through it (the add-door anchorless
/// leg, refresh, retry-incomplete, convergence); author-monitor asserts a
/// hard key and is the deliberate exception (RE-009). `mode` is the sole patience knob (REQ-005); `source` attributes any
/// raised conflict (the engine is shared by all doors, so it cannot assume one).
pub async fn settle_identity<R: EnglishIdentityResolver, D: WorkIdentityRepository>(
    resolver: &R,
    repo: &D,
    user_id: UserId,
    work: &Work,
    mode: IdentityMode,
    source: ConflictSource,
) -> Result<IdentityReport, WorkIdentityError> {
    let prior = work.identity_status;

    // REQ-006 terminal guard: Conflict / NotFound / NeedsReview are terminal —
    // no resolve runs, no write, verdict None (idempotent no-op; AC-010/AC-012).
    if matches!(
        prior,
        IdentityStatus::Conflict | IdentityStatus::NotFound | IdentityStatus::NeedsReview
    ) {
        return Ok(IdentityReport {
            prior_status: prior,
            final_status: prior,
            anchors_merged: Vec::new(),
            verdict: None,
        });
    }

    let seed = seed_from_work(work);
    let tier = match mode {
        IdentityMode::Interactive => LatencyTier::Interactive,
        IdentityMode::Background => LatencyTier::Background,
    };
    let resolution = resolver.resolve(user_id, &seed, tier).await?;

    let mut anchors_merged = Vec::new();
    let mut final_status = prior;
    let verdict;

    match resolution {
        Resolution::Resolved { identity, .. } => {
            // FLM gate: auto-confirm when the resolved title/author match the seed.
            // Only if both fail do we hold anchors as pending for user review.
            let verified = flm_match(work, &identity);
            if verified {
                anchors_merged = repo
                    .merge_missing_anchors(work.id, &identity)
                    .await?
                    .iter()
                    .map(|t| t.as_str().to_string())
                    .collect();
                let target = if has_work_anchor(&identity) {
                    IdentityStatus::Confirmed
                } else if identity.isbn_13.is_some() || identity.asin.is_some() {
                    IdentityStatus::Provisional
                } else {
                    IdentityStatus::Pending
                };
                if target == IdentityStatus::Confirmed && prior != IdentityStatus::Confirmed {
                    repo.set_identity_confirmed(work.id).await?;
                    final_status = IdentityStatus::Confirmed;
                } else if target == IdentityStatus::Provisional && prior == IdentityStatus::Pending
                {
                    repo.set_identity_provisional(work.id).await?;
                    final_status = IdentityStatus::Provisional;
                }
            } else {
                for (anchor_type, value) in [
                    (AnchorType::OL_WORK, identity.ol_key.as_deref()),
                    (AnchorType::GR_WORK, identity.gr_key.as_deref()),
                    (AnchorType::HC_WORK, identity.hc_key.as_deref()),
                    (AnchorType::ISBN_13, identity.isbn_13.as_deref()),
                    (AnchorType::ASIN, identity.asin.as_deref()),
                ] {
                    if let Some(v) = value {
                        if anchor_slot_occupied(work, anchor_type) {
                            tracing::debug!(
                                work_id = work.id,
                                anchor_type,
                                "pending guess dropped: slot already settled"
                            );
                        } else {
                            repo.record_pending_anchor(work.id, AnchorType::new(anchor_type), v)
                                .await?;
                        }
                    }
                }
            }
            verdict = ResolverVerdictKind::Resolved;
        }
        Resolution::Unresolved { captured, .. } => {
            // ST-002: transient — absorb anchors when title/author match, else hold pending.
            let verified = flm_match(work, &captured);
            if verified {
                anchors_merged = repo
                    .merge_missing_anchors(work.id, &captured)
                    .await?
                    .iter()
                    .map(|t| t.as_str().to_string())
                    .collect();
            } else {
                for (anchor_type, value) in [
                    (AnchorType::OL_WORK, captured.ol_key.as_deref()),
                    (AnchorType::GR_WORK, captured.gr_key.as_deref()),
                    (AnchorType::HC_WORK, captured.hc_key.as_deref()),
                    (AnchorType::ISBN_13, captured.isbn_13.as_deref()),
                    (AnchorType::ASIN, captured.asin.as_deref()),
                ] {
                    if let Some(v) = value {
                        if anchor_slot_occupied(work, anchor_type) {
                            tracing::debug!(
                                work_id = work.id,
                                anchor_type,
                                "pending guess dropped: slot already settled"
                            );
                        } else {
                            repo.record_pending_anchor(work.id, AnchorType::new(anchor_type), v)
                                .await?;
                        }
                    }
                }
            }
            verdict = ResolverVerdictKind::Unresolved;
        }
        Resolution::NeedsConfirmation { candidates } => {
            // REQ-005: ambiguous candidates — mode decides ONLY on a Pending work.
            // Background surfaces NeedsReview; Interactive leaves it Pending for a
            // user pick. A settled work is never downgraded by a weak verdict.
            if prior == IdentityStatus::Pending && matches!(mode, IdentityMode::Background) {
                // Persist the ranked candidates behind this park (REQ-010),
                // queryable per work — previously discarded on this exact path.
                repo.record_review_candidates(work.id, &candidates).await?;
                repo.set_needs_review(work.id).await?;
                final_status = IdentityStatus::NeedsReview;
            }
            verdict = ResolverVerdictKind::NeedsConfirmation;
        }
        Resolution::Conflict {
            conflict,
            captured,
            tied,
        } => {
            if prior == IdentityStatus::Pending {
                // A fresh Pending work has no established anchor — the tie itself
                // is the conflict to surface (AC-007).
                repo.raise_identity_conflict(conflict).await?;
                final_status = IdentityStatus::Conflict;
            } else {
                // Settled work (REQ-003 From-Provisional/From-Confirmed, D-Q008):
                // contradiction-based, never kind-based — check the established
                // anchor against the representative AND every tied cluster (AC-018).
                let mut contradictions = Vec::new();
                for candidate in std::iter::once(&captured).chain(tied.iter()) {
                    contradictions.extend(
                        repo.detect_conflicting_anchors(work.id, candidate, source)
                            .await?,
                    );
                }
                if !contradictions.is_empty() {
                    for c in contradictions {
                        repo.raise_identity_conflict(c).await?;
                    }
                    final_status = IdentityStatus::Conflict;
                }
            }
            verdict = ResolverVerdictKind::Conflict;
        }
    }

    Ok(IdentityReport {
        prior_status: prior,
        final_status,
        anchors_merged,
        verdict: Some(verdict),
    })
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

/// A captured identity carries a work anchor iff one of OL/GR/HC is present and
/// non-empty (the REQ-003 Confirmed-vs-Provisional split).
fn has_work_anchor(identity: &CapturedIdentity) -> bool {
    [
        identity.ol_key.as_deref(),
        identity.gr_key.as_deref(),
        identity.hc_key.as_deref(),
    ]
    .iter()
    .any(|a| a.map(|s| !s.is_empty()).unwrap_or(false))
}

fn incoming_from_captured(c: &CapturedIdentity) -> IncomingConflictPayload {
    IncomingConflictPayload {
        ol_key: c.ol_key.clone(),
        gr_key: c.gr_key.clone(),
        hc_key: c.hc_key.clone(),
        isbn_13: c.isbn_13.clone(),
        asin: c.asin.clone(),
        title: c.title.clone(),
        author_name: c.author_name.clone(),
        year: None,
        cover_url: None,
        top_candidates: Vec::new(),
    }
}
