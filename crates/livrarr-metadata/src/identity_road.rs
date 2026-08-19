//! The identity road — `IdentityRoadServiceImpl::settle` is the only
//! production entry for all six Work-creation doors and every re-key
//! continuation (`architecture_decisions.identity_road_chokepoint`). IR v1
//! `livrarr-metadata` module (ir-v1-identity-layer-rewrite.yaml:1251-1292).

use livrarr_domain::identity_layer::{
    evaluate_match, title_parts_from_provider, AuthorInheritanceOutcome, CapturedIdentity,
    DirectionalMatchVerdicts, EditionId, EditionRepository, EvidenceProvenance, IdentityRoadError,
    IdentityRoadInteraction, IdentityRoadOrigin, IdentityRoadOutcome, IdentityRoadRequest,
    IdentityRoadService, IdentityTitleTuple, LostMatchGuardSet, MainTitleGuard,
    ProviderIdentityEvidence, ReviewActor, ReviewResolutionCommand, RouteKey, RouteOwner,
    RouteProvenance, SettlementCommit, SettlementReviewCard, UserIdentityChoice, WorkContributor,
    WorkIdentityEvidence, WorkIdentityRepository, WorkRoute, WorkRouteState, WrongMergeGuardSet,
};
use livrarr_domain::services::AuthorLinkWorkflow;
use livrarr_domain::{
    guard_author_route, history_events::WorkAddSource, AuthorId, AuthorLinkError,
    AuthorRouteEvidenceSource, AuthorRouteGuardResult, ProviderAuthorRef, UserId, WorkId,
};
use livrarr_enrichment::identity_layer::EnrichmentApplyOutcome;
use livrarr_identity::identity_layer::{
    IdentityDecisionRequest, IdentityDecisionSettlement, IdentityEngine, IdentityEngineError,
};

/// IR v1 names `ProposedWorkIdentity` (`reconcile_complete_group`'s input)
/// without a field list. Reconstructed from `reconciliation_policy` — the
/// candidate identity a create/re-key commit checks against every other
/// active Work sharing the same broad main+primary group. See
/// STUBS-REPORT.md.
#[derive(Debug, Clone)]
pub struct ProposedWorkIdentity {
    pub user_id: UserId,
    pub identity_title: IdentityTitleTuple,
    pub primary_author_id: AuthorId,
    /// Reuses `merge_field_policy.singular_fields`' already-named
    /// `text_distinction` concept.
    pub text_distinction: Option<String>,
}

/// IR v1 names `PairwiseIdentityOutcome` (`CompleteGroupReconciliation.pairwise_outcomes`)
/// without a field list. Reconstructed from
/// `reconciliation_policy.evaluation_order`'s
/// `pairwise_authority_certain_predicate` step.
#[derive(Debug, Clone, Copy)]
pub struct PairwiseIdentityOutcome {
    pub left_work_id: WorkId,
    pub right_work_id: WorkId,
    pub verdicts: DirectionalMatchVerdicts,
    pub same_text: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompleteGroupReconciliationAction {
    AutoMerge,
    CommitDifferent,
    Review,
}

#[derive(Debug, Clone)]
pub struct CompleteGroupReconciliation {
    pub broad_main_author_candidates: Vec<WorkId>,
    pub exact_tuple_author_group: Vec<WorkId>,
    pub pairwise_outcomes: Vec<PairwiseIdentityOutcome>,
    pub action: CompleteGroupReconciliationAction,
    pub review_card: Option<SettlementReviewCard>,
}

/// Generic/enum dispatch only (FP-029) — never `Box<dyn Service>`.
pub struct IdentityRoadServiceImpl<I, R, E, A>
where
    I: IdentityEngine + Send + Sync + 'static,
    R: WorkIdentityRepository + Send + Sync + 'static,
    E: EditionRepository + Send + Sync + 'static,
    A: AuthorLinkWorkflow + Send + Sync + 'static,
{
    pub identity_engine: I,
    pub identity_repository: R,
    pub edition_repository: E,
    pub author_link_workflow: A,
}

impl<I, R, E, A> IdentityRoadServiceImpl<I, R, E, A>
where
    I: IdentityEngine + Send + Sync + 'static,
    R: WorkIdentityRepository + Send + Sync + 'static,
    E: EditionRepository + Send + Sync + 'static,
    A: AuthorLinkWorkflow + Send + Sync + 'static,
{
    pub async fn settle(
        &self,
        request: IdentityRoadRequest,
    ) -> Result<IdentityRoadOutcome, IdentityRoadError> {
        validate_road_request(&request)?;
        if matches!(
            request.origin,
            IdentityRoadOrigin::CreationDoor(
                livrarr_domain::identity_layer::DoorKind::ManualImport
            )
        ) && request.existing_work_id.is_none()
            && request.evidence.provider_identity.is_empty()
            && request.evidence.minimum.is_some()
        {
            let provenance = request_provenance(&request);
            let card = self
                .identity_repository
                .commit_unattached_import_review(request.user_id, request.evidence)
                .await
                .map_err(map_repository_error)?;
            return Ok(IdentityRoadOutcome::ReviewPending {
                review_id: card.id,
                kind: card.kind,
                unattached: true,
                expected_generation: card.generation,
                provenance,
            });
        }
        let mut existing_work_id = selected_existing_work(&request)?;
        let mut existing = match existing_work_id {
            Some(work_id) => Some(
                self.identity_repository
                    .read_captured_identity(request.user_id, work_id)
                    .await
                    .map_err(map_repository_error)?,
            ),
            None => None,
        };
        let (mut identity_title, primary_author_id) = candidate_core(&request, existing.as_ref())?;
        let text_distinction = existing
            .as_ref()
            .map(|identity| identity.text_distinction.clone())
            .filter(|distinction| distinction != "common");
        identity_title.provenance = request_provenance(&request);
        let incoming_routes = normalize_provider_routes(
            request.user_id,
            existing_work_id.unwrap_or_default(),
            &request.evidence.provider_identity,
            &request,
        )?;
        let incoming = WorkIdentityEvidence {
            title: identity_title.clone(),
            primary_author_id,
            routes: incoming_routes.clone(),
        };
        let decision = self
            .identity_engine
            .decide(IdentityDecisionRequest {
                user_id: request.user_id,
                origin: request.origin.clone(),
                interaction: request.interaction,
                evidence: request.evidence.clone(),
                existing: existing.clone(),
                incoming: incoming.clone(),
                text_signal: None,
                alias_proof: None,
                capability_claim: None,
                lost_match: strict_lost_guards(),
                wrong_merge: strict_wrong_merge_guards(),
            })
            .await
            .map_err(map_engine_error)?;

        if decision.settlement == IdentityDecisionSettlement::Defer {
            return Ok(IdentityRoadOutcome::Deferred {
                reason: livrarr_domain::identity_layer::DeferReason(
                    "identity_authority_uncertain".to_string(),
                ),
            });
        }

        let mut expected_generation = existing
            .as_ref()
            .map_or(0, |identity| identity.identity_generation);
        let mut routes = existing
            .as_ref()
            .map_or_else(Vec::new, |identity| identity.active_routes.clone());
        let mut review_cards = Vec::new();
        let mut absorbed_work_ids = Vec::new();

        // The three human re-key continuations deliberately originate a typed
        // card. Their handlers either return it (empty merge choice) or resolve
        // it immediately through the same road; no handler writes identity
        // tables on the side.
        match &request.origin {
            IdentityRoadOrigin::AffirmPendingRoute => {
                let candidate = incoming_routes
                    .first()
                    .map(
                        |route| livrarr_domain::identity_layer::ParkedRouteCandidate {
                            route: RouteKey {
                                provider: route.provider.clone(),
                                kind: route.kind.clone(),
                                value: route.provider_scoped_id.clone(),
                            },
                            proposed_owner: RouteOwner::Work(existing_work_id.unwrap_or_default()),
                        },
                    )
                    .ok_or(IdentityRoadError::InvalidDoorEvidence)?;
                review_cards.push(SettlementReviewCard::PendingRoute {
                    work_id: existing_work_id.unwrap_or_default(),
                    candidate,
                });
            }
            IdentityRoadOrigin::WorkUpdateRekey => {
                review_cards.push(SettlementReviewCard::GroupIdentity {
                    work_ids: existing_work_id.into_iter().collect(),
                    proposed_identity: Some(incoming.clone()),
                    merge_choices: Vec::new(),
                });
            }
            IdentityRoadOrigin::ManualWorkMerge {
                loser_work_id,
                choices,
            } => {
                let loser = self
                    .identity_repository
                    .read_captured_identity(request.user_id, *loser_work_id)
                    .await
                    .map_err(map_repository_error)?;
                let mut proposed_identity = incoming.clone();
                proposed_identity.routes = routes.clone();
                proposed_identity.routes.extend(loser.active_routes);
                review_cards.push(SettlementReviewCard::GroupIdentity {
                    work_ids: vec![existing_work_id.unwrap_or_default(), *loser_work_id],
                    proposed_identity: Some(proposed_identity),
                    merge_choices: choices.clone(),
                });
            }
            _ if decision.settlement == IdentityDecisionSettlement::Review => {
                if let Some(candidate) = decision.conflict {
                    review_cards.push(SettlementReviewCard::PendingRoute {
                        work_id: existing_work_id.unwrap_or_default(),
                        candidate,
                    });
                } else {
                    review_cards.push(SettlementReviewCard::GroupIdentity {
                        work_ids: existing_work_id.into_iter().collect(),
                        proposed_identity: Some(incoming.clone()),
                        merge_choices: Vec::new(),
                    });
                }
            }
            _ => {
                let reconciliation = self
                    .reconcile_complete_group(ProposedWorkIdentity {
                        user_id: request.user_id,
                        identity_title: identity_title.clone(),
                        primary_author_id,
                        text_distinction: text_distinction.clone(),
                    })
                    .await?;
                match reconciliation.action {
                    CompleteGroupReconciliationAction::AutoMerge => {
                        if existing_work_id.is_none() {
                            if let Some(winner) =
                                reconciliation.broad_main_author_candidates.first().copied()
                            {
                                let winner_identity = self
                                    .identity_repository
                                    .read_captured_identity(request.user_id, winner)
                                    .await
                                    .map_err(map_repository_error)?;
                                expected_generation = winner_identity.identity_generation;
                                routes = winner_identity.active_routes.clone();
                                existing_work_id = Some(winner);
                                existing = Some(winner_identity);
                            }
                        }
                        let winner = existing_work_id.unwrap_or_default();
                        absorbed_work_ids = reconciliation
                            .broad_main_author_candidates
                            .into_iter()
                            .filter(|work_id| *work_id != winner)
                            .collect();
                        merge_routes(&mut routes, incoming_routes.clone());
                    }
                    CompleteGroupReconciliationAction::CommitDifferent => {
                        merge_routes(&mut routes, incoming_routes.clone());
                    }
                    CompleteGroupReconciliationAction::Review => {
                        // A creation door can discover an established broad
                        // group only after the decision phase. Review parks
                        // the incoming tuple/routes; it must therefore claim
                        // and update that established Work, not create the
                        // proposal as a second Work behind a ReviewPending
                        // response.
                        if existing_work_id.is_none() {
                            if let Some(anchor) =
                                reconciliation.broad_main_author_candidates.first().copied()
                            {
                                let anchor_identity = self
                                    .identity_repository
                                    .read_captured_identity(request.user_id, anchor)
                                    .await
                                    .map_err(map_repository_error)?;
                                expected_generation = anchor_identity.identity_generation;
                                routes = anchor_identity.active_routes.clone();
                                existing_work_id = Some(anchor);
                                existing = Some(anchor_identity);
                            }
                        }
                        review_cards.push(SettlementReviewCard::GroupIdentity {
                            work_ids: reconciliation.broad_main_author_candidates,
                            proposed_identity: Some(incoming.clone()),
                            merge_choices: Vec::new(),
                        });
                    }
                }
            }
        }

        // Card origination claims a generation but does not prematurely apply
        // the requested tuple or route; the continuation performs that write.
        let commit_title = if review_cards.is_empty() {
            identity_title
        } else {
            existing
                .as_ref()
                .map_or(identity_title, |captured| captured.identity_title.clone())
        };

        // Captured-route handoffs commonly rediscover the exact graph already
        // stored for a Work. Such a visit is observation, not settlement: it
        // must not claim a generation or manufacture an audit event. Limit
        // this fast no-op to machine capture continuations; human doors and
        // review origination retain their explicit audited transitions.
        let machine_capture = matches!(
            request.origin,
            IdentityRoadOrigin::EnrichmentPass
                | IdentityRoadOrigin::ManualRefresh
                | IdentityRoadOrigin::ConvergenceVisit
        );
        if machine_capture
            && review_cards.is_empty()
            && absorbed_work_ids.is_empty()
            && existing.as_ref().is_some_and(|captured| {
                captured.identity_title == commit_title
                    && captured.primary_author_id == primary_author_id
                    && captured.text_distinction == text_distinction.as_deref().unwrap_or("common")
                    && captured.active_routes == routes
            })
        {
            if let Some(captured) = existing {
                self.inherit_primary_author_route(captured.clone(), None)
                    .await
                    .map_err(|error| IdentityRoadError::Database(format!("{error:?}")))?;
                return Ok(settled_outcome(captured, false, 0, 0));
            }
        }

        let committed = self
            .identity_repository
            .commit_settlement(SettlementCommit {
                user_id: request.user_id,
                existing_work_id,
                add_source: creation_add_source(&request.origin),
                identity_title: commit_title,
                text_distinction,
                contributors: vec![WorkContributor {
                    user_id: request.user_id,
                    work_id: existing_work_id.unwrap_or_default(),
                    author_id: primary_author_id,
                    ordinal: 0,
                    roles: Vec::new(),
                }],
                routes,
                absorbed_work_ids,
                expected_generation,
                review_cards,
            })
            .await
            .map_err(map_repository_error)?;
        if let Some(card) = committed.review_cards.first() {
            return Ok(IdentityRoadOutcome::ReviewPending {
                review_id: card.id,
                kind: card.kind,
                unattached: false,
                expected_generation: card.generation,
                provenance: request_provenance(&request),
            });
        }
        // The author-link workflow is downstream of the committed Work. There
        // is no safe provider Author ref in a Work-route-only request, so this
        // still runs the typed no-evidence branch rather than fabricating one.
        self.inherit_primary_author_route(committed.identity.clone(), None)
            .await
            .map_err(|error| IdentityRoadError::Database(format!("{error:?}")))?;
        Ok(settled_outcome(committed.identity, committed.created, 0, 0))
    }

    pub async fn reconcile_complete_group(
        &self,
        candidate: ProposedWorkIdentity,
    ) -> Result<CompleteGroupReconciliation, IdentityRoadError> {
        if candidate.user_id <= 0
            || candidate.primary_author_id <= 0
            || candidate.identity_title.normalized_main.is_empty()
        {
            return Err(IdentityRoadError::InvalidDoorEvidence);
        }
        let identities = self
            .identity_repository
            .list_captured_identities_in_group(
                candidate.user_id,
                candidate.identity_title.normalized_main.clone(),
                candidate.primary_author_id,
            )
            .await
            .map_err(map_repository_error)?;
        let broad_main_author_candidates = identities
            .iter()
            .map(|identity| identity.own_work_id)
            .collect::<Vec<_>>();
        let exact_tuple_author_group = identities
            .iter()
            .filter(|identity| identity.identity_title == candidate.identity_title)
            .map(|identity| identity.own_work_id)
            .collect::<Vec<_>>();
        let candidate_evidence = WorkIdentityEvidence {
            title: candidate.identity_title.clone(),
            primary_author_id: candidate.primary_author_id,
            routes: Vec::new(),
        };
        let mut pairwise_outcomes = Vec::new();
        for current in &identities {
            let verdicts = evaluate_match(
                candidate_evidence.clone(),
                evidence_from_captured(current),
                strict_lost_guards(),
                strict_wrong_merge_guards(),
            );
            pairwise_outcomes.push(PairwiseIdentityOutcome {
                left_work_id: 0,
                right_work_id: current.own_work_id,
                verdicts,
                same_text: authority_certain(&verdicts),
            });
        }
        for (index, left) in identities.iter().enumerate() {
            for right in identities.iter().skip(index + 1) {
                let verdicts = evaluate_match(
                    evidence_from_captured(left),
                    evidence_from_captured(right),
                    strict_lost_guards(),
                    strict_wrong_merge_guards(),
                );
                pairwise_outcomes.push(PairwiseIdentityOutcome {
                    left_work_id: left.own_work_id,
                    right_work_id: right.own_work_id,
                    verdicts,
                    same_text: authority_certain(&verdicts),
                });
            }
        }
        let cohort_has_audited_distinction = identities
            .iter()
            .any(|identity| identity.text_distinction != "common");
        let action = if candidate.text_distinction.is_some() || pairwise_outcomes.is_empty() {
            CompleteGroupReconciliationAction::CommitDifferent
        } else if cohort_has_audited_distinction {
            CompleteGroupReconciliationAction::Review
        } else if pairwise_outcomes.iter().all(|outcome| outcome.same_text) {
            CompleteGroupReconciliationAction::AutoMerge
        } else {
            CompleteGroupReconciliationAction::Review
        };
        let review_card = (action == CompleteGroupReconciliationAction::Review).then(|| {
            SettlementReviewCard::GroupIdentity {
                work_ids: broad_main_author_candidates.clone(),
                proposed_identity: Some(candidate_evidence),
                merge_choices: Vec::new(),
            }
        });
        Ok(CompleteGroupReconciliation {
            broad_main_author_candidates,
            exact_tuple_author_group,
            pairwise_outcomes,
            action,
            review_card,
        })
    }

    /// `Linked` when the F1 guard agrees, `F1ReviewCandidate` when it
    /// rejects, `NoAuthorId` when absent/unreadable.
    pub async fn inherit_primary_author_route(
        &self,
        settled_work: CapturedIdentity,
        author_evidence: Option<ProviderAuthorRef>,
    ) -> Result<AuthorInheritanceOutcome, AuthorLinkError> {
        let Some(author_evidence) = author_evidence.filter(|value| !value.name.trim().is_empty())
        else {
            return Ok(AuthorInheritanceOutcome::NoAuthorId);
        };
        let names = self
            .identity_repository
            .read_primary_author_names(settled_work.user_id, settled_work.primary_author_id)
            .await
            .map_err(|error| AuthorLinkError::Database(error.to_string()))?;
        match guard_author_route(
            &names,
            author_evidence,
            Some(settled_work.own_work_id),
            AuthorRouteEvidenceSource::Tier1SettledWork,
        ) {
            AuthorRouteGuardResult::Agreed(evidence) => self
                .author_link_workflow
                .submit_evidence(
                    settled_work.user_id,
                    settled_work.primary_author_id,
                    evidence,
                )
                .await
                .map(AuthorInheritanceOutcome::Linked),
            AuthorRouteGuardResult::Rejected(evidence) => self
                .author_link_workflow
                .record_readarr_rejection(
                    settled_work.user_id,
                    settled_work.primary_author_id,
                    evidence,
                )
                .await
                .map(AuthorInheritanceOutcome::F1ReviewCandidate),
            AuthorRouteGuardResult::NonAuthorial(_)
            | AuthorRouteGuardResult::UnlabeledMismatchDropped(_) => {
                Ok(AuthorInheritanceOutcome::NoAuthorId)
            }
        }
    }

    /// Resubmits `outcome.captured_route_evidence` through `settle` with the
    /// named existing-work origin. `trigger` reuses `IdentityRoadOrigin`
    /// directly — its `EnrichmentPass`/`ManualRefresh`/`ConvergenceVisit`
    /// variants are exactly IR v1's inline `trigger` type text.
    pub async fn apply_captured_enrichment_routes(
        &self,
        user_id: UserId,
        work_id: WorkId,
        trigger: IdentityRoadOrigin,
        outcome: EnrichmentApplyOutcome,
    ) -> Result<IdentityRoadOutcome, IdentityRoadError> {
        if !matches!(
            trigger,
            IdentityRoadOrigin::EnrichmentPass
                | IdentityRoadOrigin::ManualRefresh
                | IdentityRoadOrigin::ConvergenceVisit
        ) || user_id <= 0
            || work_id <= 0
            || outcome.presentation.subtitle.user_id != user_id
            || outcome.presentation.subtitle.work_id != work_id
        {
            return Err(IdentityRoadError::InvalidDoorEvidence);
        }
        let mut provider_identity = Vec::with_capacity(outcome.captured_route_evidence.len());
        for captured in outcome.captured_route_evidence {
            if captured.provider_scoped_id.trim().is_empty() {
                return Err(IdentityRoadError::ProviderBoundary);
            }
            provider_identity.push(ProviderIdentityEvidence {
                provider: captured.provider.clone(),
                route: RouteKey {
                    provider: captured.provider,
                    kind: captured.kind,
                    value: captured.provider_scoped_id,
                },
                work_core: None,
                provenance: Default::default(),
            });
        }
        let handoff = livrarr_domain::identity_layer::CapturedRouteHandoff {
            metadata_generation: outcome.metadata_generation,
            provider_identity,
            route_proposals: Vec::new(),
        };
        if let Some(settled) = self
            .apply_captured_route_handoff_authority(user_id, work_id, trigger, handoff)
            .await?
        {
            return Ok(settled);
        }
        let snapshot = self
            .identity_repository
            .read_captured_identity(user_id, work_id)
            .await
            .map_err(map_repository_error)?;
        Ok(settled_outcome(snapshot, false, 0, 0))
    }

    /// Generation-checked successor seam used by every production machine
    /// handoff. Empty fresh evidence is an observation-only no-op and never
    /// calls `settle`.
    pub async fn apply_captured_route_handoff_authority(
        &self,
        user_id: UserId,
        work_id: WorkId,
        trigger: IdentityRoadOrigin,
        handoff: livrarr_domain::identity_layer::CapturedRouteHandoff,
    ) -> Result<Option<IdentityRoadOutcome>, IdentityRoadError> {
        if !matches!(
            trigger,
            IdentityRoadOrigin::EnrichmentPass
                | IdentityRoadOrigin::ManualRefresh
                | IdentityRoadOrigin::ConvergenceVisit
        ) || user_id <= 0
            || work_id <= 0
        {
            return Err(IdentityRoadError::InvalidDoorEvidence);
        }
        let snapshot = self
            .identity_repository
            .read_captured_identity(user_id, work_id)
            .await
            .map_err(map_repository_error)?;
        if snapshot.identity_generation != handoff.metadata_generation {
            return Err(IdentityRoadError::StaleGeneration);
        }
        if handoff.provider_identity.is_empty() && handoff.route_proposals.is_empty() {
            return Ok(None);
        }
        if handoff.provider_identity.iter().any(|evidence| {
            evidence.route.value.trim().is_empty() || evidence.provider != evidence.route.provider
        }) {
            return Err(IdentityRoadError::ProviderBoundary);
        }
        if handoff.route_proposals.iter().any(|route| {
            route.value.trim().is_empty()
                || !matches!(
                    route.kind,
                    livrarr_domain::identity_layer::RouteKind::OpenLibraryWork
                        | livrarr_domain::identity_layer::RouteKind::GoodreadsWork
                        | livrarr_domain::identity_layer::RouteKind::HardcoverWork
                )
        }) {
            return Err(IdentityRoadError::ProviderBoundary);
        }
        // A corroborated route settles immediately. Any sibling proposals in
        // the same pass are obsolete once the graph has a Work-level route.
        if handoff.provider_identity.is_empty() {
            let mut proposals = handoff.route_proposals;
            proposals.sort_by_key(|route| {
                format!("{:?}:{:?}:{}", route.provider, route.kind, route.value)
            });
            proposals.dedup();
            let mut first = None;
            for route in proposals {
                let provider = route.provider.clone();
                let minted = self
                    .identity_repository
                    .commit_pending_route_review(
                        user_id,
                        work_id,
                        snapshot.identity_generation,
                        livrarr_domain::identity_layer::ParkedRouteCandidate {
                            route,
                            proposed_owner: RouteOwner::Work(work_id),
                        },
                    )
                    .await
                    .map_err(map_repository_error)?;
                first.get_or_insert((minted, provider));
            }
            return Ok(
                first.map(|(card, provider)| IdentityRoadOutcome::ReviewPending {
                    review_id: card.id,
                    kind: card.kind,
                    unattached: false,
                    expected_generation: card.generation,
                    provenance: EvidenceProvenance::Provider(provider),
                }),
            );
        }
        self.settle(IdentityRoadRequest {
            user_id,
            origin: trigger,
            evidence: livrarr_domain::identity_layer::IdentityEvidenceBundle {
                user_choice: None,
                owned_files: Vec::new(),
                provider_identity: handoff.provider_identity,
                minimum: None,
            },
            interaction: IdentityRoadInteraction::MachineAlone,
            existing_work_id: Some(work_id),
        })
        .await
        .map(Some)
    }

    /// Validates card kind, scope, allowed action, and expected generation,
    /// then persists continuation plus actor audit atomically.
    pub async fn resolve_review(
        &self,
        actor: ReviewActor,
        command: ReviewResolutionCommand,
    ) -> Result<IdentityRoadOutcome, IdentityRoadError> {
        self.resolve_review_with_cancel(actor, command, tokio_util::sync::CancellationToken::new())
            .await
    }

    pub async fn resolve_review_with_cancel(
        &self,
        actor: ReviewActor,
        command: ReviewResolutionCommand,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<IdentityRoadOutcome, IdentityRoadError> {
        if cancel.is_cancelled() {
            return Err(IdentityRoadError::Cancelled);
        }
        let pending = self
            .identity_repository
            .load_pending_review(actor.clone(), command.card_id())
            .await
            .map_err(map_repository_error)?;
        if pending.kind != command.kind() {
            return Err(IdentityRoadError::ReviewKindMismatch);
        }
        if pending.kind != livrarr_domain::identity_layer::ReviewKind::PendingRoute
            && pending.generation != command.expected_generation()
        {
            return Err(IdentityRoadError::StaleGeneration);
        }
        let committed = self
            .identity_repository
            .commit_review_continuation(actor, command, cancel)
            .await
            .map_err(map_repository_error)?;
        let livrarr_domain::identity_layer::ReviewContinuationOutcome {
            card_id,
            kind,
            identity,
            library_items_moved,
            grabs_moved,
            ..
        } = committed;
        identity.map_or_else(
            || {
                Ok(IdentityRoadOutcome::Deferred {
                    reason: livrarr_domain::identity_layer::DeferReason(format!(
                        "resolved_{:?}_review_{}",
                        kind, card_id
                    )),
                })
            },
            |identity| {
                Ok(settled_outcome(
                    identity,
                    false,
                    library_items_moved,
                    grabs_moved,
                ))
            },
        )
    }

    pub fn mint_edition_evidence_review(
        &self,
        edition_id: EditionId,
        evidence_ids: Vec<i64>,
    ) -> SettlementReviewCard {
        SettlementReviewCard::EditionEvidence {
            edition_id,
            evidence_ids,
        }
    }
}

fn selected_existing_work(
    request: &IdentityRoadRequest,
) -> Result<Option<WorkId>, IdentityRoadError> {
    let selected = match request.evidence.user_choice.as_ref() {
        Some(UserIdentityChoice::ExistingWork(work_id)) => Some(*work_id),
        _ => request.existing_work_id,
    };
    if selected.is_some_and(|work_id| work_id <= 0) {
        return Err(IdentityRoadError::InvalidDoorEvidence);
    }
    Ok(selected)
}

fn candidate_core(
    request: &IdentityRoadRequest,
    existing: Option<&CapturedIdentity>,
) -> Result<(IdentityTitleTuple, AuthorId), IdentityRoadError> {
    if let Some(minimum) = request.evidence.minimum.as_ref() {
        let author_id = minimum
            .authors
            .first()
            .copied()
            .filter(|author_id| *author_id > 0)
            .ok_or(IdentityRoadError::InvalidDoorEvidence)?;
        let title = title_parts_from_provider(minimum.title.clone(), None)
            .map_err(|_| IdentityRoadError::InvalidDoorEvidence)?;
        return Ok((title, author_id));
    }
    if let Some(existing) = existing {
        return Ok((existing.identity_title.clone(), existing.primary_author_id));
    }
    let minimum = match request.evidence.user_choice.as_ref() {
        Some(UserIdentityChoice::ExplicitCreate(minimum)) => Some(minimum),
        _ => request.evidence.minimum.as_ref(),
    };
    if let Some(minimum) = minimum {
        let author_id = minimum
            .authors
            .first()
            .copied()
            .filter(|author_id| *author_id > 0)
            .ok_or(IdentityRoadError::InvalidDoorEvidence)?;
        let title = title_parts_from_provider(minimum.title.clone(), None)
            .map_err(|_| IdentityRoadError::InvalidDoorEvidence)?;
        return Ok((title, author_id));
    }
    let mut cores = request
        .evidence
        .provider_identity
        .iter()
        .map(|evidence| evidence.work_core.as_ref());
    let core = cores
        .next()
        .flatten()
        .filter(|core| {
            core.primary_author_id > 0
                && !core.identity_title.main.trim().is_empty()
                && !core.identity_title.normalized_main.trim().is_empty()
        })
        .ok_or(IdentityRoadError::InvalidDoorEvidence)?;
    if cores.any(|candidate| candidate != Some(core)) {
        return Err(IdentityRoadError::InvalidDoorEvidence);
    }
    Ok((core.identity_title.clone(), core.primary_author_id))
}

fn validate_road_request(request: &IdentityRoadRequest) -> Result<(), IdentityRoadError> {
    if request.user_id <= 0 {
        return Err(IdentityRoadError::InvalidDoorEvidence);
    }
    let evidence = &request.evidence;
    let has_choice = evidence.user_choice.is_some();
    let has_file = !evidence.owned_files.is_empty();
    let has_provider = !evidence.provider_identity.is_empty();
    let has_minimum = evidence.minimum.is_some();
    let minimum_mixed_with_route = has_minimum && (has_file || has_provider);
    let valid = match request.origin {
        IdentityRoadOrigin::CreationDoor(door) if request.existing_work_id.is_none() => {
            match door {
                livrarr_domain::identity_layer::DoorKind::DirectAdd => {
                    request.interaction == IdentityRoadInteraction::HumanWatching
                        && has_choice
                        && !has_file
                }
                livrarr_domain::identity_layer::DoorKind::ManualImport => {
                    request.interaction == IdentityRoadInteraction::HumanWatching
                        && has_choice
                        && has_file
                }
                livrarr_domain::identity_layer::DoorKind::ListImport => {
                    request.interaction == IdentityRoadInteraction::HumanWatching
                        && !has_file
                        && (has_choice || has_provider || has_minimum)
                        && !minimum_mixed_with_route
                }
                livrarr_domain::identity_layer::DoorKind::AuthorMonitor => {
                    request.interaction == IdentityRoadInteraction::MachineAlone
                        && !has_choice
                        && !has_file
                        && has_provider
                        && !has_minimum
                }
                livrarr_domain::identity_layer::DoorKind::SeriesMonitor => {
                    request.interaction == IdentityRoadInteraction::MachineAlone
                        && !has_choice
                        && !has_file
                        && (has_provider ^ has_minimum)
                }
                livrarr_domain::identity_layer::DoorKind::ReadarrImport => {
                    request.interaction == IdentityRoadInteraction::MachineAlone
                        && !has_choice
                        && (has_file || (has_provider && !has_minimum))
                }
            }
        }
        IdentityRoadOrigin::EnrichmentPass
        | IdentityRoadOrigin::ManualRefresh
        | IdentityRoadOrigin::ConvergenceVisit => {
            request.existing_work_id.is_some()
                && request.interaction == IdentityRoadInteraction::MachineAlone
                && !has_choice
                && !has_file
                && has_provider
                && !has_minimum
        }
        IdentityRoadOrigin::WorkUpdateRekey => {
            request.existing_work_id.is_some()
                && request.interaction == IdentityRoadInteraction::HumanWatching
                && !has_choice
                && !has_file
                && !has_provider
                && has_minimum
        }
        IdentityRoadOrigin::ManualWorkMerge { loser_work_id, .. } => {
            request.existing_work_id.is_some()
                && loser_work_id > 0
                && Some(loser_work_id) != request.existing_work_id
                && request.interaction == IdentityRoadInteraction::HumanWatching
                && !has_choice
                && !has_file
                && !has_provider
                && has_minimum
        }
        IdentityRoadOrigin::AffirmPendingRoute => {
            request.existing_work_id.is_some()
                && request.interaction == IdentityRoadInteraction::HumanWatching
                && !has_choice
                && !has_file
                && has_provider
                && !has_minimum
        }
        IdentityRoadOrigin::CreationDoor(
            livrarr_domain::identity_layer::DoorKind::ManualImport,
        ) => {
            request.existing_work_id.is_some()
                && request.interaction == IdentityRoadInteraction::HumanWatching
                && matches!(
                    evidence.user_choice,
                    Some(UserIdentityChoice::ExistingWork(work_id))
                        if Some(work_id) == request.existing_work_id
                )
                && has_file
        }
        IdentityRoadOrigin::CreationDoor(livrarr_domain::identity_layer::DoorKind::ListImport) => {
            request.existing_work_id.is_some()
                && request.interaction == IdentityRoadInteraction::HumanWatching
                && matches!(
                    evidence.user_choice,
                    Some(UserIdentityChoice::ExistingWork(work_id))
                        if Some(work_id) == request.existing_work_id
                )
                && !has_file
                && has_minimum
        }
        IdentityRoadOrigin::CreationDoor(_) => false,
    };
    valid
        .then_some(())
        .ok_or(IdentityRoadError::InvalidDoorEvidence)
}

fn normalize_provider_routes(
    user_id: UserId,
    work_id: WorkId,
    evidence: &[ProviderIdentityEvidence],
    request: &IdentityRoadRequest,
) -> Result<Vec<WorkRoute>, IdentityRoadError> {
    evidence
        .iter()
        .map(|item| {
            if item.provider != item.route.provider || item.route.value.trim().is_empty() {
                return Err(IdentityRoadError::ProviderBoundary);
            }
            let provenance = route_provenance(request, item);
            let user_confirmed = matches!(provenance, RouteProvenance::UserChoice);
            Ok(WorkRoute {
                id: 0,
                user_id,
                owner: RouteOwner::Work(work_id),
                resolved_work_id: work_id,
                provider: item.provider.clone(),
                kind: item.route.kind.clone(),
                provider_scoped_id: item.route.value.clone(),
                state: WorkRouteState::Active,
                provenance,
                user_confirmed,
                observed_at: chrono::Utc::now(),
            })
        })
        .collect()
}

fn route_provenance(
    request: &IdentityRoadRequest,
    evidence: &ProviderIdentityEvidence,
) -> RouteProvenance {
    if matches!(request.origin, IdentityRoadOrigin::AffirmPendingRoute)
        || (matches!(
            request.origin,
            IdentityRoadOrigin::CreationDoor(livrarr_domain::identity_layer::DoorKind::DirectAdd)
                | IdentityRoadOrigin::CreationDoor(
                    livrarr_domain::identity_layer::DoorKind::ListImport
                )
        ) && request.evidence.user_choice.is_some())
    {
        return RouteProvenance::UserChoice;
    }
    if let Some(owned) = request.evidence.owned_files.first() {
        return RouteProvenance::OwnedFile {
            library_item_id: Some(owned.library_item_id),
            file_revision: owned.file_revision,
        };
    }
    match &evidence.provenance {
        livrarr_domain::identity_layer::ProviderIdentityEvidenceProvenance::AnchorPayload => {
            RouteProvenance::Provider(evidence.provider.clone())
        }
        livrarr_domain::identity_layer::ProviderIdentityEvidenceProvenance::SearchFallback {
            corroborating_kind,
        } => RouteProvenance::SearchFallback {
            provider: evidence.provider.clone(),
            corroborating_kind: corroborating_kind.clone(),
        },
        livrarr_domain::identity_layer::ProviderIdentityEvidenceProvenance::TextDecisiveSearchFallback => {
            RouteProvenance::TextDecisiveSearchFallback {
                provider: evidence.provider.clone(),
            }
        }
    }
}

fn request_provenance(request: &IdentityRoadRequest) -> EvidenceProvenance {
    if matches!(
        request.origin,
        IdentityRoadOrigin::WorkUpdateRekey
            | IdentityRoadOrigin::ManualWorkMerge { .. }
            | IdentityRoadOrigin::AffirmPendingRoute
    ) || request.evidence.user_choice.is_some()
    {
        EvidenceProvenance::User
    } else if !request.evidence.owned_files.is_empty() {
        EvidenceProvenance::OwnedFile
    } else if let Some(provider) = request.evidence.provider_identity.first() {
        EvidenceProvenance::Provider(provider.provider.clone())
    } else {
        EvidenceProvenance::Provider(livrarr_domain::identity_layer::IdentityProvider::Other(
            "minimum".to_string(),
        ))
    }
}

fn creation_add_source(origin: &IdentityRoadOrigin) -> Option<WorkAddSource> {
    let IdentityRoadOrigin::CreationDoor(door) = origin else {
        return None;
    };
    Some(match door {
        livrarr_domain::identity_layer::DoorKind::DirectAdd => WorkAddSource::Search,
        livrarr_domain::identity_layer::DoorKind::ManualImport => WorkAddSource::FileImport,
        livrarr_domain::identity_layer::DoorKind::ListImport => WorkAddSource::ListImport,
        livrarr_domain::identity_layer::DoorKind::AuthorMonitor => WorkAddSource::AuthorMonitor,
        livrarr_domain::identity_layer::DoorKind::SeriesMonitor => WorkAddSource::SeriesMonitor,
        livrarr_domain::identity_layer::DoorKind::ReadarrImport => WorkAddSource::Readarr,
    })
}

fn merge_routes(routes: &mut Vec<WorkRoute>, incoming: Vec<WorkRoute>) {
    for route in incoming {
        if !routes.iter().any(|existing| {
            existing.provider == route.provider
                && existing.kind == route.kind
                && existing.provider_scoped_id == route.provider_scoped_id
        }) {
            routes.push(route);
        }
    }
}

fn strict_lost_guards() -> LostMatchGuardSet {
    LostMatchGuardSet {
        one_sided_subtitle_recovery: true,
        shared_edition_id_confirmation: true,
        translation_same_text_signals: Default::default(),
    }
}

fn strict_wrong_merge_guards() -> WrongMergeGuardSet {
    WrongMergeGuardSet {
        main_title_guard: MainTitleGuard(true),
        volume_conflict_guard: true,
        author_disagreement_guard: true,
        work_key_contradiction_guard: true,
        audited_different_text_guard: true,
    }
}

fn evidence_from_captured(identity: &CapturedIdentity) -> WorkIdentityEvidence {
    WorkIdentityEvidence {
        title: identity.identity_title.clone(),
        primary_author_id: identity.primary_author_id,
        routes: identity.active_routes.clone(),
    }
}

fn authority_certain(verdicts: &DirectionalMatchVerdicts) -> bool {
    matches!(
        verdicts.title,
        livrarr_domain::identity_matching::TitleVerdict::Same
    ) && matches!(
        verdicts.author,
        livrarr_domain::identity_matching::AuthorVerdict::Agree
    ) && !matches!(
        verdicts.id,
        livrarr_domain::identity_matching::IdVerdict::WorkKeyContradiction
    )
}

fn settled_outcome(
    identity: CapturedIdentity,
    created: bool,
    library_items_moved: usize,
    grabs_moved: usize,
) -> IdentityRoadOutcome {
    IdentityRoadOutcome::Settled {
        work_id: identity.own_work_id,
        created,
        routes: identity.active_routes,
        status: identity.status,
        library_items_moved,
        grabs_moved,
    }
}

fn map_engine_error(error: IdentityEngineError) -> IdentityRoadError {
    match error {
        IdentityEngineError::ProbeBlocked => IdentityRoadError::ProviderBoundary,
        IdentityEngineError::InvalidEvidence => IdentityRoadError::InvalidDoorEvidence,
    }
}

fn map_repository_error(
    error: livrarr_domain::identity_layer::IdentityRepositoryError,
) -> IdentityRoadError {
    match error {
        livrarr_domain::identity_layer::IdentityRepositoryError::NotFound => {
            IdentityRoadError::NotFound
        }
        livrarr_domain::identity_layer::IdentityRepositoryError::StaleGeneration => {
            IdentityRoadError::StaleGeneration
        }
        livrarr_domain::identity_layer::IdentityRepositoryError::ReviewKindMismatch => {
            IdentityRoadError::ReviewKindMismatch
        }
        livrarr_domain::identity_layer::IdentityRepositoryError::UnauthorizedScope => {
            IdentityRoadError::UnauthorizedScope
        }
        livrarr_domain::identity_layer::IdentityRepositoryError::InvalidResolution => {
            IdentityRoadError::InvalidResolution
        }
        livrarr_domain::identity_layer::IdentityRepositoryError::ReviewProposalInvalidated(
            reason,
        ) => IdentityRoadError::ReviewProposalInvalidated(reason),
        livrarr_domain::identity_layer::IdentityRepositoryError::Cancelled => {
            IdentityRoadError::Cancelled
        }
        other => IdentityRoadError::Database(other.to_string()),
    }
}

impl<I, R, E, A> IdentityRoadService for IdentityRoadServiceImpl<I, R, E, A>
where
    I: IdentityEngine + Send + Sync + 'static,
    R: WorkIdentityRepository + Send + Sync + 'static,
    E: EditionRepository + Send + Sync + 'static,
    A: AuthorLinkWorkflow + Send + Sync + 'static,
{
    async fn settle(
        &self,
        request: IdentityRoadRequest,
    ) -> Result<IdentityRoadOutcome, IdentityRoadError> {
        IdentityRoadServiceImpl::settle(self, request).await
    }

    async fn resolve_review(
        &self,
        actor: ReviewActor,
        command: ReviewResolutionCommand,
    ) -> Result<IdentityRoadOutcome, IdentityRoadError> {
        IdentityRoadServiceImpl::resolve_review(self, actor, command).await
    }

    async fn apply_captured_route_handoff(
        &self,
        user_id: UserId,
        work_id: WorkId,
        trigger: IdentityRoadOrigin,
        handoff: livrarr_domain::identity_layer::CapturedRouteHandoff,
    ) -> Result<Option<IdentityRoadOutcome>, IdentityRoadError> {
        self.apply_captured_route_handoff_authority(user_id, work_id, trigger, handoff)
            .await
    }
}
