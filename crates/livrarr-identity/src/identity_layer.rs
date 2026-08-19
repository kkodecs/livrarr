//! Identity-layer-rewrite (F2) deterministic identity orchestration. IR v1
//! `livrarr-identity` module (ir-v1-identity-layer-rewrite.yaml:1180-1202).
//! Owns no SQL and calls no enrichment API (sibling seam, FP-032).

use livrarr_domain::identity_layer::{
    evaluate_match, AliasEquivalenceProof, CapturedIdentity, DirectionalMatchVerdicts,
    IdentityConflictClass, IdentityEvidenceBundle, IdentityRoadInteraction, IdentityRoadOrigin,
    LostMatchGuardSet, ParkedRouteCandidate, RouteKey, RouteKind, RouteOwner, SampledTextSignal,
    TextIdentityVerdict, WorkIdentityEvidence, WorkRouteState, WrongMergeGuardSet,
};
use livrarr_external_data::identity_layer::ProviderRouteEvidence;

/// Complete pure-policy input: caller scope, door context, ranked evidence,
/// normalized candidate identity, accepted capabilities, and guard sets.
#[derive(Debug, Clone)]
pub struct IdentityDecisionRequest {
    pub user_id: livrarr_domain::UserId,
    pub origin: IdentityRoadOrigin,
    pub interaction: IdentityRoadInteraction,
    pub evidence: IdentityEvidenceBundle,
    pub existing: Option<CapturedIdentity>,
    pub incoming: WorkIdentityEvidence,
    pub text_signal: Option<SampledTextSignal>,
    pub alias_proof: Option<AliasEquivalenceProof>,
    pub capability_claim: Option<IdentityCapabilityClaim>,
    pub lost_match: LostMatchGuardSet,
    pub wrong_merge: WrongMergeGuardSet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityCapabilityClaim {
    WhoseText,
    ProviderAlias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionEvidenceTier {
    UserChoice,
    OwnedFile,
    ProviderIdentity,
    MinimumTitleAuthors,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityDecisionSettlement {
    Decide,
    Review,
    Defer,
}

#[derive(Debug, Clone)]
pub struct IdentityDecision {
    pub match_verdicts: DirectionalMatchVerdicts,
    pub conflict: Option<ParkedRouteCandidate>,
    pub text_verdict: TextIdentityVerdict,
    pub settlement: IdentityDecisionSettlement,
    pub selected_tier: DecisionEvidenceTier,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum IdentityEngineError {
    #[error("blocked on probe")]
    ProbeBlocked,
    #[error("invalid evidence")]
    InvalidEvidence,
}

#[trait_variant::make(Send)]
pub trait IdentityEngine: Send + Sync {
    async fn decide(
        &self,
        request: IdentityDecisionRequest,
    ) -> Result<IdentityDecision, IdentityEngineError>;

    /// Pure/deterministic — no I/O, so not part of the trait's async surface
    /// despite living alongside `decide`. IR v1 gives it `errors: []` and a
    /// bare `Option<...>` output (no `Result`).
    fn classify_route_conflict(
        &self,
        existing: CapturedIdentity,
        candidate: ProviderRouteEvidence,
    ) -> Option<IdentityConflictClass>;
}

/// Production, deterministic identity policy. It is deliberately stateless:
/// repository and provider I/O remain at the road and adapter boundaries.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicIdentityEngine;

impl IdentityEngine for DeterministicIdentityEngine {
    async fn decide(
        &self,
        request: IdentityDecisionRequest,
    ) -> Result<IdentityDecision, IdentityEngineError> {
        validate_request(&request)?;
        match request.capability_claim {
            Some(IdentityCapabilityClaim::WhoseText) if request.text_signal.is_none() => {
                return Err(IdentityEngineError::ProbeBlocked);
            }
            Some(IdentityCapabilityClaim::ProviderAlias) if request.alias_proof.is_none() => {
                return Err(IdentityEngineError::ProbeBlocked);
            }
            _ => {}
        }

        let selected_tier =
            evidence_tier(&request.evidence).ok_or(IdentityEngineError::InvalidEvidence)?;
        let match_verdicts = request.existing.as_ref().map_or_else(
            || {
                evaluate_match(
                    request.incoming.clone(),
                    request.incoming.clone(),
                    request.lost_match.clone(),
                    request.wrong_merge.clone(),
                )
            },
            |existing| {
                evaluate_match(
                    captured_evidence(existing),
                    request.incoming.clone(),
                    request.lost_match.clone(),
                    request.wrong_merge.clone(),
                )
            },
        );
        let mut conflict_class = request.existing.as_ref().and_then(|existing| {
            request.incoming.routes.iter().find_map(|route| {
                let candidate = ProviderRouteEvidence {
                    provider: route.provider.clone(),
                    kind: route.kind.clone(),
                    provider_scoped_id: route.provider_scoped_id.clone(),
                };
                self.classify_route_conflict(existing.clone(), candidate)
            })
        });
        // A resolved provider fan-out is precisely how different providers'
        // native work ids become routes of one already-selected Work. On the
        // three machine capture origins, exact main + primary-author agreement
        // discharges class C; same-provider disagreement and ownership
        // collisions remain hard conflicts. Human creation/re-key doors retain
        // the conservative review behavior.
        if matches!(
            conflict_class,
            Some(IdentityConflictClass::CrossProviderWorkKeyDisagreement)
        ) && matches!(
            request.origin,
            IdentityRoadOrigin::EnrichmentPass
                | IdentityRoadOrigin::ManualRefresh
                | IdentityRoadOrigin::ConvergenceVisit
        ) && matches!(
            match_verdicts.title,
            livrarr_domain::identity_matching::TitleVerdict::Same
        ) && matches!(
            match_verdicts.author,
            livrarr_domain::identity_matching::AuthorVerdict::Agree
        ) {
            conflict_class = None;
        }
        let conflict = conflict_class.and_then(|_| {
            request
                .incoming
                .routes
                .first()
                .map(|route| ParkedRouteCandidate {
                    route: RouteKey {
                        provider: route.provider.clone(),
                        kind: route.kind.clone(),
                        value: route.provider_scoped_id.clone(),
                    },
                    proposed_owner: RouteOwner::Work(
                        request
                            .existing
                            .as_ref()
                            .map_or(0, |identity| identity.own_work_id),
                    ),
                })
        });
        let authority_certain = request.existing.is_none()
            || (matches!(
                match_verdicts.title,
                livrarr_domain::identity_matching::TitleVerdict::Same
            ) && matches!(
                match_verdicts.author,
                livrarr_domain::identity_matching::AuthorVerdict::Agree
            ) && !matches!(
                match_verdicts.id,
                livrarr_domain::identity_matching::IdVerdict::WorkKeyContradiction
            ) && conflict.is_none());
        let settlement = if authority_certain {
            IdentityDecisionSettlement::Decide
        } else if request.interaction == IdentityRoadInteraction::HumanWatching {
            IdentityDecisionSettlement::Review
        } else {
            IdentityDecisionSettlement::Defer
        };
        let text_verdict = request.text_signal.map_or(
            TextIdentityVerdict::ReviewRequired,
            TextIdentityVerdict::SameText,
        );

        Ok(IdentityDecision {
            match_verdicts,
            conflict,
            text_verdict,
            settlement,
            selected_tier,
        })
    }

    fn classify_route_conflict(
        &self,
        existing: CapturedIdentity,
        candidate: ProviderRouteEvidence,
    ) -> Option<IdentityConflictClass> {
        let active: Vec<_> = existing
            .active_routes
            .iter()
            .filter(|route| matches!(route.state, WorkRouteState::Active))
            .collect();
        if active.iter().any(|route| {
            route.provider == candidate.provider
                && route.kind == candidate.kind
                && route.provider_scoped_id == candidate.provider_scoped_id
                && route.resolved_work_id != existing.own_work_id
        }) {
            return Some(IdentityConflictClass::RouteOwnedByDifferentWork);
        }
        if is_work_kind(&candidate.kind)
            && active.iter().any(|route| {
                route.provider == candidate.provider
                    && route.kind == candidate.kind
                    && route.provider_scoped_id != candidate.provider_scoped_id
            })
        {
            // ProviderRouteEvidence carries no alias capability. Raw ids can
            // therefore never bypass class A; accepted proof is consumed by
            // decide before a future adapter emits an uncontested candidate.
            return Some(IdentityConflictClass::SameProviderWorkIdDisagreement);
        }
        if is_work_kind(&candidate.kind)
            && active
                .iter()
                .any(|route| is_work_kind(&route.kind) && route.provider != candidate.provider)
        {
            return Some(IdentityConflictClass::CrossProviderWorkKeyDisagreement);
        }
        None
    }
}

fn validate_request(request: &IdentityDecisionRequest) -> Result<(), IdentityEngineError> {
    if request.user_id <= 0
        || request.incoming.title.normalized_main.is_empty()
        || request.incoming.primary_author_id <= 0
        || request
            .existing
            .as_ref()
            .is_some_and(|existing| existing.user_id != request.user_id)
        || request
            .incoming
            .routes
            .iter()
            .any(|route| route.user_id != request.user_id)
    {
        return Err(IdentityEngineError::InvalidEvidence);
    }
    Ok(())
}

fn evidence_tier(evidence: &IdentityEvidenceBundle) -> Option<DecisionEvidenceTier> {
    if evidence.user_choice.is_some() {
        Some(DecisionEvidenceTier::UserChoice)
    } else if !evidence.owned_files.is_empty() {
        Some(DecisionEvidenceTier::OwnedFile)
    } else if !evidence.provider_identity.is_empty() {
        Some(DecisionEvidenceTier::ProviderIdentity)
    } else if evidence.minimum.is_some() {
        Some(DecisionEvidenceTier::MinimumTitleAuthors)
    } else {
        None
    }
}

fn captured_evidence(identity: &CapturedIdentity) -> WorkIdentityEvidence {
    WorkIdentityEvidence {
        title: identity.identity_title.clone(),
        primary_author_id: identity.primary_author_id,
        routes: identity.active_routes.clone(),
    }
}

fn is_work_kind(kind: &RouteKind) -> bool {
    matches!(
        kind,
        RouteKind::OpenLibraryWork | RouteKind::GoodreadsWork | RouteKind::HardcoverWork
    )
}
