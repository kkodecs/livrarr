//! Background-tier convergence for identity-pending works + the conditional,
//! background-only LLM identity-verify (D-013/Q-001). Generalizes the
//! bulk_resolver / enrichment-retry machinery; mirrors that module's
//! free-function, dependency-passing style (no resolver struct exists).
//! See ir-v2 metadata-async-resolver (REQ-022/025/026).

use livrarr_domain::identity::{CapturedIdentity, NewIdentityConflict};
use livrarr_domain::services::{LlmCaller, WorkIdentityRepository, WorkServiceError};
use livrarr_domain::{UserId, Work};

use crate::english_identity_resolver::EnglishIdentityResolver;

/// Re-run resolve(.., Background) for an identity-pending work and APPLY the
/// result (merge + anchor-merge), advancing it toward the full anchor set
/// (REQ-022). A Tier-B dead-end transitions to NeedsReview rather than looping
/// (REQ-026); a user-resolved Conflict is never re-litigated (REQ-025). When an
/// LLM is configured and the deterministic layer stayed ambiguous, invokes
/// `llm_identity_verify` (D-013 — background only, never on the interactive path).
pub async fn converge_identity_pending<R: EnglishIdentityResolver, D: WorkIdentityRepository>(
    resolver: &R,
    db: &D,
    user_id: UserId,
    work: &Work,
) -> Result<(), WorkServiceError> {
    let _ = (resolver, db, user_id, work);
    todo!()
}

/// Conditional background LLM identity validation (D-013/Q-001): runs only when
/// an LLM is configured AND the deterministic layer left identity ambiguous.
/// Returns a conflict to raise on a confident mismatch, else `None`. Never
/// blocks an interactive create (REQ-017).
pub async fn llm_identity_verify<L: LlmCaller>(
    llm: &L,
    user_id: UserId,
    work: &Work,
    captured: &CapturedIdentity,
) -> Option<NewIdentityConflict> {
    let _ = (llm, user_id, work, captured);
    todo!()
}
