//! Background-tier convergence for identity-pending works + the conditional,
//! background-only LLM identity-verify (D-013/Q-001). Generalizes the
//! bulk_resolver / enrichment-retry machinery; mirrors that module's
//! free-function, dependency-passing style (no resolver struct exists).
//! See ir-v2 metadata-async-resolver (REQ-022/025/026).

use std::collections::HashMap;
use std::time::Duration;

use livrarr_domain::identity::{
    CapturedIdentity, ConflictSource, IdentityConflictKind, IncomingConflictPayload, LatencyTier,
    NewIdentityConflict, PendingReason, Resolution, WorkSeed,
};
use livrarr_domain::services::{
    AnchorCompletionReport, LlmCallRequest, LlmCaller, LlmPurpose, ProviderCallSink,
    WorkIdentityError, WorkIdentityRepository, WorkServiceError,
};
use livrarr_domain::{IdentityStatus, UserId, Work};

use crate::english_identity_resolver::EnglishIdentityResolver;

/// Re-run `resolve(.., Background)` for an identity-pending work and APPLY the
/// result (anchor-merge), advancing it toward the full federated anchor set
/// (REQ-022). A Tier-B dead-end transitions to NeedsReview rather than looping
/// (REQ-026); a user-resolved Conflict is never re-litigated (REQ-025).
pub async fn converge_identity_pending<R: EnglishIdentityResolver, D: WorkIdentityRepository>(
    resolver: &R,
    db: &D,
    user_id: UserId,
    work: &Work,
) -> Result<(), WorkServiceError> {
    // REQ-025: a work whose identity is terminal — an open anchor `Conflict` or an
    // unverifiable `NotFound` (the LLM rejected all payloads) — is never re-litigated
    // by a background pass; it waits for the user (reset_for_manual_refresh).
    if matches!(
        work.identity_status,
        IdentityStatus::Conflict | IdentityStatus::NotFound
    ) {
        return Ok(());
    }

    let seed = seed_from_work(work);
    let resolution = resolver
        .resolve(user_id, &seed, LatencyTier::Background)
        .await
        .map_err(|e| WorkServiceError::Enrichment(e.to_string()))?;

    match resolution {
        // REQ-022/028: fill every anchor the work lacks; `confirm_anchor` keeps the
        // denormalized works columns in sync. Additive — an existing anchor is
        // never clobbered.
        Resolution::Resolved { identity, .. } => {
            db.merge_missing_anchors(work.id, &identity)
                .await
                .map_err(|e| WorkServiceError::Enrichment(e.to_string()))?;
        }
        // A transient abstention (provider down) leaves the work pending for a
        // later pass but still absorbs any partial anchors it did carry (REQ-025
        // no-clobber); a deterministic dead-end surfaces as needs-review (REQ-026).
        Resolution::Unresolved {
            reason, captured, ..
        } => {
            if is_terminal_pending(reason) {
                db.set_needs_review(work.id)
                    .await
                    .map_err(|e| WorkServiceError::Enrichment(e.to_string()))?;
            } else {
                db.merge_missing_anchors(work.id, &captured)
                    .await
                    .map_err(|e| WorkServiceError::Enrichment(e.to_string()))?;
            }
        }
        // A non-interactive Tier-B item with no resolving identifier cannot be
        // auto-confirmed — surface it for the user rather than loop (REQ-026).
        Resolution::NeedsConfirmation { .. } => {
            db.set_needs_review(work.id)
                .await
                .map_err(|e| WorkServiceError::Enrichment(e.to_string()))?;
        }
        // A genuine identity conflict is raised by `WorkService::add`'s create-time
        // preflight (which covers all match paths, r5 C-R012); a background pass
        // leaves the existing work's identity untouched (REQ-025).
        Resolution::Conflict { .. } => {}
    }

    Ok(())
}

/// Refresh-time identity anchor completion (REQ-008), built on the same
/// convergence shape as [`converge_identity_pending`]: deterministic matching
/// first, LLM disambiguation where the provider requires it (Goodreads),
/// never first-hit adoption — a failed completion leaves the anchor absent.
///
/// Differences from plain convergence: (a) the caller supplies
/// `suppressed_providers` from `provider_retry_state` (identity stays db-free;
/// boundedness is the caller's existing retry-suppression semantics); (b) the
/// outcome returns as an [`AnchorCompletionReport`] for recording and scatter
/// gating; (c) it runs for anchor-incomplete Confirmed works, not just
/// Pending. Anchor persistence goes through
/// [`WorkIdentityRepository::merge_missing_anchors`] — monotonic, established
/// anchors untouched. Works with all five anchors or in identity Conflict
/// return an empty report (nothing to complete / paused pending the user).
pub async fn complete_anchors<R: EnglishIdentityResolver, D: WorkIdentityRepository>(
    resolver: &R,
    repo: &D,
    user_id: UserId,
    work: &Work,
    suppressed_providers: &[String],
    sink: &std::sync::Arc<dyn ProviderCallSink>,
) -> Result<AnchorCompletionReport, WorkIdentityError> {
    // Identity-operation call records originate at the client layer (REQ-001);
    // this surface only shapes the report.
    let _ = sink;

    // A Conflict identity is paused pending the user; a complete anchor set
    // has nothing to complete (REQ-008).
    if work.identity_status == IdentityStatus::Conflict {
        return Ok(AnchorCompletionReport::default());
    }
    let missing = missing_anchors(work);
    if missing.is_empty() {
        return Ok(AnchorCompletionReport::default());
    }

    let is_suppressed = |provider: &str| suppressed_providers.iter().any(|s| s == provider);

    // REQ-008 boundedness: when every missing anchor's source provider is
    // under retry suppression, complete without resolver work — zero network.
    if missing.iter().all(|(_, provider)| is_suppressed(provider)) {
        return Ok(AnchorCompletionReport {
            resolved: Vec::new(),
            skipped: missing
                .iter()
                .map(|(_, p)| (p.to_string(), "suppressed".to_string()))
                .collect(),
        });
    }

    let seed = seed_from_work(work);
    let resolution = resolver
        .resolve(user_id, &seed, LatencyTier::Background)
        .await?;

    // Skip-reason vocabulary: "not_found" ONLY when the resolution came back
    // empty-handed (no candidates) — the caller records it as a terminal
    // outcome that suppresses the next attempt (REQ-008 boundedness). A
    // RESOLVED pass that simply lacked one provider's anchor is
    // "unresolvable", and a tie or ambiguous outcome is "ambiguous" — the
    // arbitration's failure, not the provider lacking the work (PO decision
    // 2026-06-11, #148): neither records a terminal, so a later pass with
    // better data may still complete the anchor.
    let (identity, absent_reason) = match resolution {
        // Monotonic: merge_missing_anchors appends only absent anchor types
        // via confirm_anchor — established anchors untouched (AC-010).
        Resolution::Resolved { identity, .. } => {
            repo.merge_missing_anchors(work.id, &identity).await?;
            (Some(identity), "unresolvable")
        }
        // No anchor writes on any non-resolved outcome: completion never
        // adopts a fuzzy or partial result (a skipped provider beats a wrong
        // merge).
        Resolution::Unresolved { .. } => (None, "not_found"),
        Resolution::NeedsConfirmation { .. } | Resolution::Conflict { .. } => (None, "ambiguous"),
    };

    let mut report = AnchorCompletionReport::default();
    for (anchor, provider) in missing {
        match identity.as_ref().and_then(|i| anchor_value(i, anchor)) {
            Some(value) => report.resolved.push((anchor.to_string(), value)),
            None => {
                let reason = if is_suppressed(provider) {
                    "suppressed"
                } else {
                    absent_reason
                };
                report
                    .skipped
                    .push((provider.to_string(), reason.to_string()));
            }
        }
    }
    Ok(report)
}

/// The five identity anchors and their canonical source-provider keys (the
/// `MetadataProvider::record_key` vocabulary), for suppression checks and
/// skip reporting.
fn missing_anchors(work: &Work) -> Vec<(&'static str, &'static str)> {
    let mut missing = Vec::new();
    if work.ol_key.is_none() {
        missing.push(("ol_key", "openlibrary"));
    }
    if work.gr_key.is_none() {
        missing.push(("gr_key", "goodreads"));
    }
    if work.hc_key.is_none() {
        missing.push(("hc_key", "hardcover"));
    }
    if work.isbn_13.is_none() {
        missing.push(("isbn_13", "google_books"));
    }
    if work.asin.is_none() {
        missing.push(("asin", "audnexus"));
    }
    missing
}

fn anchor_value(identity: &CapturedIdentity, anchor: &str) -> Option<String> {
    match anchor {
        "ol_key" => identity.ol_key.clone(),
        "gr_key" => identity.gr_key.clone(),
        "hc_key" => identity.hc_key.clone(),
        "isbn_13" => identity.isbn_13.clone(),
        "asin" => identity.asin.clone(),
        _ => None,
    }
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

fn is_terminal_pending(reason: PendingReason) -> bool {
    matches!(
        reason,
        PendingReason::NoCandidates | PendingReason::LowConfidence
    )
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
