//! Behavioral contract tests for ProviderQueue (R-22), covering scatter-gather dispatch,
//! durable phase-1 persistence into provider_retry_state [I-11], panic isolation,
//! restart safety, and manual-mode coercion semantics.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use livrarr_db::{
    CreateUserDbRequest, CreateWorkDbRequest, ProviderRetryState, ProviderRetryStateDb, UserDb,
    WorkDbCreate,
};
use livrarr_domain::{
    MetadataProvider, OutcomeClass, PermanentFailureReason, RequestPriority, UserRole, Work,
};
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::{
    DefaultProviderQueue, DefaultProviderQueueBuilder, EnrichmentContext, EnrichmentMode,
    ProviderQueue, ProviderQueueConfig, ScatterGatherResult,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderQueueScenario {
    NominalPopulatedOutcomes,
    Phase1DurabilityMixed,
    SuccessPayloadDurability,
    DeferredBackgroundWillRetry,
    AllCanMergeBackground,
    ConflictBlocksMerge,
    ProviderPanicIsolation,
    RestartSkipsTerminal,
    ManualCoercesWillRetry,
    ManualCoercesSuppressed,
    RetryBudgetExhausted,
    SuppressionExhausted,
    SuppressionPreservesFirstTimestamp,
    ProviderNotApplicableSkipped,
    NonSuccessTerminalClearsPayload,
    HardRefreshCoercesWillRetry,
    HardRefreshCoercesSuppressed,
}

// DEFERRED: normalization pipeline tests require NormalizedWorkDetail type.

#[trait_variant::make(Send)]
pub trait ProviderQueueTestHarness: Send + Sync {
    type Queue: ProviderQueue;
    type RetryDb: ProviderRetryStateDb;

    async fn setup(scenario: ProviderQueueScenario) -> Self;

    fn queue(&self) -> &Self::Queue;

    fn retry_db(&self) -> &Self::RetryDb;

    fn work(&self) -> &Work;

    fn call_count(&self, provider: MetadataProvider) -> usize;

    fn provider_config(&self, provider: MetadataProvider) -> ProviderQueueConfig;

    /// The `RequestPriority` the stub for `provider` last received via
    /// `fetch_by_anchor` (B4) — `None` if the provider was never dispatched.
    fn last_priority(&self, provider: MetadataProvider) -> Option<RequestPriority>;
}

fn background_context() -> EnrichmentContext {
    EnrichmentContext {
        priority: RequestPriority::Low,
        mode: EnrichmentMode::Background,
    }
}

fn manual_context() -> EnrichmentContext {
    EnrichmentContext {
        priority: RequestPriority::High,
        mode: EnrichmentMode::Manual,
    }
}

fn hard_refresh_context() -> EnrichmentContext {
    EnrichmentContext {
        priority: RequestPriority::High,
        mode: EnrichmentMode::HardRefresh,
    }
}

fn has_outcome_class(result: &ScatterGatherResult, class: OutcomeClass) -> bool {
    result
        .outcomes
        .values()
        .any(|outcome| outcome.class() == class)
}

fn provider_with_outcome_class(
    result: &ScatterGatherResult,
    class: OutcomeClass,
) -> MetadataProvider {
    result
        .outcomes
        .iter()
        .find_map(|(provider, outcome)| (outcome.class() == class).then_some(*provider))
        .expect("scenario must include the requested outcome class")
}

async fn retry_state_for<DB: ProviderRetryStateDb>(
    db: &DB,
    work: &Work,
    provider: MetadataProvider,
) -> ProviderRetryState {
    db.get_retry_state(work.user_id, work.id, provider)
        .await
        .unwrap()
        .expect("retry state row must exist for provider")
}

fn success_payload_value(
    result: &ScatterGatherResult,
    provider: MetadataProvider,
) -> serde_json::Value {
    match result
        .outcomes
        .get(&provider)
        .expect("success provider must be present in outcomes")
    {
        ProviderOutcome::Success(payload) => serde_json::to_value(payload.as_ref()).unwrap(),
        _ => panic!("provider was expected to be a success outcome"),
    }
}

fn stored_payload_value(state: &ProviderRetryState) -> serde_json::Value {
    let payload = state
        .normalized_payload_json
        .as_deref()
        .expect("success outcome must persist normalized_payload_json");
    serde_json::from_str::<serde_json::Value>(payload).unwrap()
}

macro_rules! provider_queue_contract_tests {
    ($harness:ty) => {
        #[tokio::test]

        async fn test_provider_queue_dispatch_returns_populated_outcomes_nominal() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: nominal dispatch returns ScatterGatherResult with populated outcomes map
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::NominalPopulatedOutcomes,
            )
            .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            assert_eq!(result.work_id, h.work().id);
            assert!(!result.outcomes.is_empty());
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_persists_phase1_outcomes_before_return() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: all phase-1 outcomes are durable in provider_retry_state before dispatch returns
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::Phase1DurabilityMixed,
            )
            .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            let states = h
                .retry_db()
                .list_retry_states(h.work().user_id, h.work().id)
                .await
                .unwrap();

            assert_eq!(states.len(), result.outcomes.len());

            for (provider, outcome) in &result.outcomes {
                let state = states
                    .iter()
                    .find(|state| state.provider == *provider)
                    .expect("every returned provider outcome must have a durable retry-state row");
                assert_eq!(state.last_outcome, Some(outcome.class()));
            }

            if has_outcome_class(&result, OutcomeClass::WillRetry) {
                let provider = provider_with_outcome_class(&result, OutcomeClass::WillRetry);
                let state = retry_state_for(h.retry_db(), h.work(), provider).await;
                assert_eq!(state.last_outcome, Some(OutcomeClass::WillRetry));
                assert_eq!(state.attempts, 1);
                assert_eq!(state.suppressed_passes, 0);
            }

            if has_outcome_class(&result, OutcomeClass::Suppressed) {
                let provider = provider_with_outcome_class(&result, OutcomeClass::Suppressed);
                let state = retry_state_for(h.retry_db(), h.work(), provider).await;
                assert_eq!(state.last_outcome, Some(OutcomeClass::Suppressed));
                assert_eq!(state.attempts, 0);
                assert_eq!(state.suppressed_passes, 1);
            }
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_persists_normalized_payload_json_on_success() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: success outcomes persist normalized_payload_json in provider_retry_state
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::SuccessPayloadDurability,
            )
            .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            let success_provider = provider_with_outcome_class(&result, OutcomeClass::Success);
            let state = retry_state_for(h.retry_db(), h.work(), success_provider).await;

            assert_eq!(state.last_outcome, Some(OutcomeClass::Success));
            assert!(state.normalized_payload_json.is_some());
            assert_eq!(
                stored_payload_value(&state),
                success_payload_value(&result, success_provider)
            );
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_clears_normalized_payload_json_on_non_success_terminal_outcome() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: non-Success terminal outcomes do not retain normalized_payload_json in provider_retry_state
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::NonSuccessTerminalClearsPayload,
            )
            .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            let terminal_provider = result
                .outcomes
                .iter()
                .find_map(|(provider, outcome)| {
                    matches!(
                        outcome,
                        ProviderOutcome::NotFound
                            | ProviderOutcome::PermanentFailure { .. }
                            | ProviderOutcome::Conflict { .. }
                    )
                    .then_some(*provider)
                })
                .expect("scenario must include a non-success terminal outcome");

            let state = retry_state_for(h.retry_db(), h.work(), terminal_provider).await;
            assert!(state
                .last_outcome
                .expect("terminal retry state must persist last_outcome")
                .is_phase2_terminal());
            assert_ne!(state.last_outcome, Some(OutcomeClass::Success));
            assert!(state.normalized_payload_json.is_none());
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_sets_deferred_true_when_will_retry_present() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: deferred is true when any outcome is not can_merge() via WillRetry
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::DeferredBackgroundWillRetry,
            )
            .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            assert!(has_outcome_class(&result, OutcomeClass::WillRetry));
            assert!(result.deferred);
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_sets_deferred_false_when_all_outcomes_can_merge() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: deferred is false when all outcomes are can_merge()
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::AllCanMergeBackground,
            )
            .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            assert!(result.outcomes.values().all(|outcome| outcome.can_merge()));
            assert!(!result.deferred);
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_sets_merge_eligible_false_when_conflict_present() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: merge_eligible is false when any outcome is Conflict
            let h =
                <$harness as ProviderQueueTestHarness>::setup(ProviderQueueScenario::ConflictBlocksMerge)
                    .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            assert!(has_outcome_class(&result, OutcomeClass::Conflict));
            assert!(!result.merge_eligible);
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_sets_deferred_false_when_conflict_present() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: conflict outcome does not defer merge and returns deferred=false
            let h =
                <$harness as ProviderQueueTestHarness>::setup(ProviderQueueScenario::ConflictBlocksMerge)
                    .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            assert!(has_outcome_class(&result, OutcomeClass::Conflict));
            assert!(!result.deferred);
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_converts_provider_panic_without_aborting_others() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: provider panic becomes PermanentFailure{ProviderPanic} and other providers still complete
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::ProviderPanicIsolation,
            )
            .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            assert!(matches!(
                result.outcomes.get(&MetadataProvider::Hardcover),
                Some(ProviderOutcome::PermanentFailure {
                    reason: PermanentFailureReason::ProviderPanic
                })
            ));
            assert!(matches!(
                result.outcomes.get(&MetadataProvider::OpenLibrary),
                Some(ProviderOutcome::Success(_))
            ));
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_skips_terminal_providers_on_restart() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: providers with existing phase-2 terminal retry state are skipped on restart
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::RestartSkipsTerminal,
            )
            .await;

            let _ = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            assert_eq!(h.call_count(MetadataProvider::Hardcover), 0);
            assert!(h.call_count(MetadataProvider::OpenLibrary) > 0);

            let state = retry_state_for(h.retry_db(), h.work(), MetadataProvider::Hardcover).await;
            assert!(state
                .last_outcome
                .expect("preexisting retry state must retain a terminal outcome")
                .is_phase2_terminal());
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_manual_coerces_will_retry_to_merge_eligible() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: Manual mode coerces WillRetry to merge-eligible for that request
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::ManualCoercesWillRetry,
            )
            .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), manual_context())
                .await
                .unwrap();

            assert!(has_outcome_class(&result, OutcomeClass::WillRetry));
            assert!(result.merge_eligible);
            assert!(!result.deferred);
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_manual_coerces_suppressed_to_merge_eligible() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: Manual mode coerces Suppressed to merge-eligible for that request
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::ManualCoercesSuppressed,
            )
            .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), manual_context())
                .await
                .unwrap();

            assert!(has_outcome_class(&result, OutcomeClass::Suppressed));
            assert!(result.merge_eligible);
            assert!(!result.deferred);
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_manual_mode_does_not_coerce_conflict() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: Manual mode does not coerce Conflict; Conflict always blocks merge
            let h =
                <$harness as ProviderQueueTestHarness>::setup(ProviderQueueScenario::ConflictBlocksMerge)
                    .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), manual_context())
                .await
                .unwrap();

            assert!(has_outcome_class(&result, OutcomeClass::Conflict));
            assert!(!result.merge_eligible);
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_hard_refresh_coerces_will_retry_to_merge_eligible() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: HardRefresh mode coerces WillRetry to merge-eligible for that request
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::HardRefreshCoercesWillRetry,
            )
            .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), hard_refresh_context())
                .await
                .unwrap();

            assert!(has_outcome_class(&result, OutcomeClass::WillRetry));
            assert!(result.merge_eligible);
            assert!(!result.deferred);
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_hard_refresh_coerces_suppressed_to_merge_eligible() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: HardRefresh mode coerces Suppressed to merge-eligible for that request
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::HardRefreshCoercesSuppressed,
            )
            .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), hard_refresh_context())
                .await
                .unwrap();

            assert!(has_outcome_class(&result, OutcomeClass::Suppressed));
            assert!(result.merge_eligible);
            assert!(!result.deferred);
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_hard_refresh_mode_does_not_coerce_conflict() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: HardRefresh mode does not coerce Conflict; Conflict always blocks merge
            let h =
                <$harness as ProviderQueueTestHarness>::setup(ProviderQueueScenario::ConflictBlocksMerge)
                    .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), hard_refresh_context())
                .await
                .unwrap();

            assert!(has_outcome_class(&result, OutcomeClass::Conflict));
            assert!(!result.merge_eligible);
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_records_permanent_failure_when_retry_budget_exhausted() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: only the dispatch from a preexisting attempts=max_attempts-1 retry state converts the provider to PermanentFailure{RetryBudgetExhausted}
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::RetryBudgetExhausted,
            )
            .await;

            let pre_states = h
                .retry_db()
                .list_retry_states(h.work().user_id, h.work().id)
                .await
                .unwrap();

            let pre_exhausted_provider = pre_states
                .iter()
                .find_map(|state| {
                    (state.last_outcome == Some(OutcomeClass::WillRetry) && state.attempts > 0)
                        .then_some(state.provider)
                })
                .expect("scenario must pre-seed a provider one retry away from exhaustion");

            let config = h.provider_config(pre_exhausted_provider);
            let pre_state = retry_state_for(h.retry_db(), h.work(), pre_exhausted_provider).await;
            assert_eq!(pre_state.last_outcome, Some(OutcomeClass::WillRetry));
            assert_eq!(pre_state.attempts, config.max_attempts - 1);
            assert_ne!(pre_state.last_outcome, Some(OutcomeClass::PermanentFailure));

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            assert!(matches!(
                result.outcomes.get(&pre_exhausted_provider),
                Some(ProviderOutcome::PermanentFailure {
                    reason: PermanentFailureReason::RetryBudgetExhausted
                })
            ));

            let state = retry_state_for(h.retry_db(), h.work(), pre_exhausted_provider).await;
            assert_eq!(state.last_outcome, Some(OutcomeClass::PermanentFailure));
            assert!(state.normalized_payload_json.is_none());
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_records_permanent_failure_when_suppression_budget_exhausted() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: only the dispatch from a preexisting suppressed_passes=max_suppressed_passes-1 state converts the provider to PermanentFailure{SuppressionExhausted}, preserving the original suppression-window start until terminalization
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::SuppressionExhausted,
            )
            .await;

            let pre_states = h
                .retry_db()
                .list_retry_states(h.work().user_id, h.work().id)
                .await
                .unwrap();

            let pre_exhausted_provider = pre_states
                .iter()
                .find_map(|state| {
                    (state.last_outcome == Some(OutcomeClass::Suppressed)
                        && state.suppressed_passes > 0
                        && state.first_suppressed_at.is_some())
                    .then_some(state.provider)
                })
                .expect("scenario must pre-seed a provider one suppressed pass away from exhaustion");

            let config = h.provider_config(pre_exhausted_provider);
            let pre_state = retry_state_for(h.retry_db(), h.work(), pre_exhausted_provider).await;
            let original_first_suppressed_at = pre_state
                .first_suppressed_at
                .expect("suppression window must already be in progress before final dispatch");

            assert_eq!(pre_state.last_outcome, Some(OutcomeClass::Suppressed));
            assert_eq!(
                pre_state.suppressed_passes,
                config.max_suppressed_passes - 1
            );
            let window_check = pre_state.first_suppressed_at.unwrap();
            let elapsed = chrono::Utc::now() - window_check;
            assert!(
                elapsed.num_seconds() < config.max_suppression_window_secs as i64,
                "first_suppressed_at must be within window to test budget exhaustion, not window expiry"
            );
            assert_ne!(pre_state.last_outcome, Some(OutcomeClass::PermanentFailure));

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            assert!(matches!(
                result.outcomes.get(&pre_exhausted_provider),
                Some(ProviderOutcome::PermanentFailure {
                    reason: PermanentFailureReason::SuppressionExhausted
                })
            ));

            let state = retry_state_for(h.retry_db(), h.work(), pre_exhausted_provider).await;
            assert_eq!(state.last_outcome, Some(OutcomeClass::PermanentFailure));
            assert!(state.normalized_payload_json.is_none());
            assert!(state.first_suppressed_at.is_none());
            assert!(original_first_suppressed_at <= chrono::Utc::now());
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_preserves_first_suppressed_at_on_subsequent_suppressions() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: subsequent Suppressed outcomes increment suppressed_passes without resetting first_suppressed_at
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::SuppressionPreservesFirstTimestamp,
            )
            .await;

            let pre_states = h
                .retry_db()
                .list_retry_states(h.work().user_id, h.work().id)
                .await
                .unwrap();

            let provider = pre_states
                .iter()
                .find_map(|state| {
                    (state.last_outcome == Some(OutcomeClass::Suppressed)
                        && state.suppressed_passes == 1
                        && state.first_suppressed_at.is_some())
                    .then_some(state.provider)
                })
                .expect("scenario must pre-seed a suppressed provider with an existing first_suppressed_at");

            let pre_state = retry_state_for(h.retry_db(), h.work(), provider).await;
            let known_past_timestamp = pre_state
                .first_suppressed_at
                .expect("scenario must include known past first_suppressed_at");
            assert_eq!(pre_state.suppressed_passes, 1);

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            assert!(matches!(
                result.outcomes.get(&provider),
                Some(ProviderOutcome::Suppressed { .. })
            ));

            let state = retry_state_for(h.retry_db(), h.work(), provider).await;
            assert_eq!(state.last_outcome, Some(OutcomeClass::Suppressed));
            assert_eq!(state.suppressed_passes, 2);
            assert_eq!(state.first_suppressed_at, Some(known_past_timestamp));
        }

        #[tokio::test]

        async fn test_provider_queue_dispatch_skips_not_applicable_provider() {
            // REQ-ID: R-22 | Contract: ProviderQueue::dispatch_enrichment | Behavior: a provider that is not applicable to the work is skipped without being called, and is absent from outcomes entirely
            let h = <$harness as ProviderQueueTestHarness>::setup(
                ProviderQueueScenario::ProviderNotApplicableSkipped,
            )
            .await;

            let result = h
                .queue()
                .dispatch_enrichment(h.work(), background_context())
                .await
                .unwrap();

            let skipped_provider = MetadataProvider::Hardcover;

            assert_eq!(h.call_count(skipped_provider), 0);
            assert!(!result.outcomes.contains_key(&skipped_provider));
            assert!(!matches!(
                result.outcomes.get(&skipped_provider),
                Some(ProviderOutcome::Suppressed { .. } | ProviderOutcome::WillRetry { .. })
            ));
            assert!(!matches!(
                result.outcomes.get(&skipped_provider),
                Some(
                    ProviderOutcome::Success(_)
                        | ProviderOutcome::NotFound
                        | ProviderOutcome::PermanentFailure { .. }
                        | ProviderOutcome::Conflict { .. }
                )
            ));
        }
    };
}

// =============================================================================
// Real harness — backed by SqliteDb (`:memory:` with full migrations) and
// `DefaultProviderQueue` registered with `StubProviderClient` per scenario.
// =============================================================================

fn default_config(provider: MetadataProvider) -> ProviderQueueConfig {
    ProviderQueueConfig {
        provider,
        max_attempts: 3,
        max_suppressed_passes: 3,
        max_suppression_window_secs: 3600,
    }
}

fn empty_normalized() -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: None,
        subtitle: None,
        original_title: None,
        author_name: None,
        description: None,
        year: None,
        series_name: None,
        series_position: None,
        genres: None,
        language: None,
        page_count: None,
        duration_seconds: None,
        publisher: None,
        publish_date: None,
        hc_key: None,
        gr_key: None,
        ol_key: None,
        isbn_13: None,
        asin: None,
        narrator: None,
        narration_type: None,
        abridged: None,
        rating: None,
        rating_count: None,
        cover_url: None,
        additional_isbns: Vec::new(),
        additional_asins: Vec::new(),
    }
}

fn payload_for(provider: MetadataProvider) -> NormalizedWorkDetail {
    let mut p = empty_normalized();
    p.title = Some(format!("title-{provider:?}"));
    p.description = Some(format!("desc-{provider:?}"));
    p
}

fn future_ts(secs: i64) -> chrono::DateTime<chrono::Utc> {
    Utc::now() + chrono::Duration::seconds(secs)
}

pub struct StubProviderQueueHarness {
    db: livrarr_db::sqlite::SqliteDb,
    work: Work,
    queue: DefaultProviderQueue<livrarr_db::sqlite::SqliteDb>,
    configs: HashMap<MetadataProvider, ProviderQueueConfig>,
    clients: HashMap<MetadataProvider, StubProviderClient>,
}

impl StubProviderQueueHarness {
    async fn create_db_and_work() -> (livrarr_db::sqlite::SqliteDb, Work) {
        let db = livrarr_db::create_test_db().await;
        let user_id = db
            .create_user(CreateUserDbRequest {
                username: "queue_test_user".to_string(),
                password_hash: "hash".to_string(),
                role: UserRole::Admin,
                api_key_hash: "apikey".to_string(),
            })
            .await
            .unwrap()
            .id;
        let (work, _) = db
            .create_work(CreateWorkDbRequest {
                user_id,
                title: "Queue Test Work".to_string(),
                author_name: "Test Author".to_string(),
                author_id: None,
                // REQ-006: dispatch fetches only by stored anchor — the
                // fixture carries one per scatter provider so every stub
                // still dispatches.
                ol_key: Some("OL777W".to_string()),
                gr_key: Some("777".to_string()),
                isbn_13: Some("9780000000777".to_string()),
                asin: Some("B000QUEUE77".to_string()),
                year: Some(2024),
                cover_url: Some("https://example.test/cover.jpg".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        (db, work)
    }

    fn add_stub(
        builder: DefaultProviderQueueBuilder,
        configs: &mut HashMap<MetadataProvider, ProviderQueueConfig>,
        clients: &mut HashMap<MetadataProvider, StubProviderClient>,
        provider: MetadataProvider,
        outcome: ProviderOutcome<NormalizedWorkDetail>,
    ) -> DefaultProviderQueueBuilder {
        let stub = StubProviderClient::new(provider, outcome);
        let cfg = default_config(provider);
        clients.insert(provider, stub.clone());
        configs.insert(provider, cfg.clone());
        builder.add_provider(provider, ProviderClient::Stub(stub), cfg)
    }

    fn add_panic(
        builder: DefaultProviderQueueBuilder,
        configs: &mut HashMap<MetadataProvider, ProviderQueueConfig>,
        clients: &mut HashMap<MetadataProvider, StubProviderClient>,
        provider: MetadataProvider,
    ) -> DefaultProviderQueueBuilder {
        let stub = StubProviderClient::with_panic(provider);
        let cfg = default_config(provider);
        clients.insert(provider, stub.clone());
        configs.insert(provider, cfg.clone());
        builder.add_provider(provider, ProviderClient::Stub(stub), cfg)
    }
}

impl ProviderQueueTestHarness for StubProviderQueueHarness {
    type Queue = DefaultProviderQueue<livrarr_db::sqlite::SqliteDb>;
    type RetryDb = livrarr_db::sqlite::SqliteDb;

    async fn setup(scenario: ProviderQueueScenario) -> Self {
        let (db, work) = Self::create_db_and_work().await;
        let mut configs: HashMap<MetadataProvider, ProviderQueueConfig> = HashMap::new();
        let mut clients: HashMap<MetadataProvider, StubProviderClient> = HashMap::new();
        let mut builder = DefaultProviderQueueBuilder::new();

        match scenario {
            ProviderQueueScenario::NominalPopulatedOutcomes => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::Success(Box::new(payload_for(MetadataProvider::Hardcover))),
                );
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::OpenLibrary,
                    ProviderOutcome::Success(Box::new(payload_for(MetadataProvider::OpenLibrary))),
                );
            }
            ProviderQueueScenario::Phase1DurabilityMixed => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::Success(Box::new(payload_for(MetadataProvider::Hardcover))),
                );
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::OpenLibrary,
                    ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::Timeout,
                        next_attempt_at: future_ts(600),
                    },
                );
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Goodreads,
                    ProviderOutcome::Suppressed {
                        until: future_ts(600),
                    },
                );
            }
            ProviderQueueScenario::SuccessPayloadDurability => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::Success(Box::new(payload_for(MetadataProvider::Hardcover))),
                );
            }
            ProviderQueueScenario::DeferredBackgroundWillRetry => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::Timeout,
                        next_attempt_at: future_ts(600),
                    },
                );
            }
            ProviderQueueScenario::AllCanMergeBackground => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::Success(Box::new(payload_for(MetadataProvider::Hardcover))),
                );
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::OpenLibrary,
                    ProviderOutcome::NotFound,
                );
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Goodreads,
                    ProviderOutcome::PermanentFailure {
                        reason: PermanentFailureReason::Unsupported,
                    },
                );
            }
            ProviderQueueScenario::ConflictBlocksMerge => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::Conflict {
                        detail: "gr_key drift".to_string(),
                    },
                );
            }
            ProviderQueueScenario::ProviderPanicIsolation => {
                builder = Self::add_panic(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                );
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::OpenLibrary,
                    ProviderOutcome::Success(Box::new(payload_for(MetadataProvider::OpenLibrary))),
                );
            }
            ProviderQueueScenario::RestartSkipsTerminal => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::Success(Box::new(payload_for(MetadataProvider::Hardcover))),
                );
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::OpenLibrary,
                    ProviderOutcome::Success(Box::new(payload_for(MetadataProvider::OpenLibrary))),
                );
                // Pre-seed HC with a terminal outcome so the queue skips it on
                // dispatch (restart safety).
                db.record_terminal_outcome(
                    work.user_id,
                    work.id,
                    MetadataProvider::Hardcover,
                    OutcomeClass::Success,
                    Some(serde_json::to_string(&payload_for(MetadataProvider::Hardcover)).unwrap()),
                )
                .await
                .unwrap();
            }
            ProviderQueueScenario::ManualCoercesWillRetry => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::Timeout,
                        next_attempt_at: future_ts(600),
                    },
                );
            }
            ProviderQueueScenario::ManualCoercesSuppressed => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::Suppressed {
                        until: future_ts(600),
                    },
                );
            }
            ProviderQueueScenario::RetryBudgetExhausted => {
                // Pre-seed HC at attempts=max-1 with last_outcome=WillRetry. A
                // fresh WillRetry from the client this dispatch must convert to
                // PermanentFailure(RetryBudgetExhausted).
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::Timeout,
                        next_attempt_at: future_ts(600),
                    },
                );
                let cfg = configs.get(&MetadataProvider::Hardcover).unwrap().clone();
                for _ in 0..(cfg.max_attempts - 1) {
                    db.record_will_retry(
                        work.user_id,
                        work.id,
                        MetadataProvider::Hardcover,
                        future_ts(600),
                    )
                    .await
                    .unwrap();
                }
            }
            ProviderQueueScenario::SuppressionExhausted => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::Suppressed {
                        until: future_ts(600),
                    },
                );
                let cfg = configs.get(&MetadataProvider::Hardcover).unwrap().clone();
                for _ in 0..(cfg.max_suppressed_passes - 1) {
                    db.record_suppressed(
                        work.user_id,
                        work.id,
                        MetadataProvider::Hardcover,
                        future_ts(600),
                    )
                    .await
                    .unwrap();
                }
            }
            ProviderQueueScenario::SuppressionPreservesFirstTimestamp => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::Suppressed {
                        until: future_ts(600),
                    },
                );
                // One prior suppression seeds first_suppressed_at.
                db.record_suppressed(
                    work.user_id,
                    work.id,
                    MetadataProvider::Hardcover,
                    future_ts(600),
                )
                .await
                .unwrap();
            }
            ProviderQueueScenario::ProviderNotApplicableSkipped => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::Success(Box::new(payload_for(MetadataProvider::Hardcover))),
                );
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::OpenLibrary,
                    ProviderOutcome::Success(Box::new(payload_for(MetadataProvider::OpenLibrary))),
                );
                builder = builder.with_applicability_rule(Arc::new(
                    |provider: MetadataProvider, _work: &Work| {
                        provider != MetadataProvider::Hardcover
                    },
                ));
            }
            ProviderQueueScenario::NonSuccessTerminalClearsPayload => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::NotFound,
                );
            }
            ProviderQueueScenario::HardRefreshCoercesWillRetry => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::WillRetry {
                        reason: livrarr_domain::WillRetryReason::Timeout,
                        next_attempt_at: future_ts(600),
                    },
                );
            }
            ProviderQueueScenario::HardRefreshCoercesSuppressed => {
                builder = Self::add_stub(
                    builder,
                    &mut configs,
                    &mut clients,
                    MetadataProvider::Hardcover,
                    ProviderOutcome::Suppressed {
                        until: future_ts(600),
                    },
                );
            }
        }

        let queue = builder.build(Arc::new(db.clone()));

        Self {
            db,
            work,
            queue,
            configs,
            clients,
        }
    }

    fn queue(&self) -> &Self::Queue {
        &self.queue
    }

    fn retry_db(&self) -> &Self::RetryDb {
        &self.db
    }

    fn work(&self) -> &Work {
        &self.work
    }

    fn call_count(&self, provider: MetadataProvider) -> usize {
        self.clients
            .get(&provider)
            .map(|c| c.call_count())
            .unwrap_or(0)
    }

    fn provider_config(&self, provider: MetadataProvider) -> ProviderQueueConfig {
        self.configs
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| default_config(provider))
    }

    fn last_priority(&self, provider: MetadataProvider) -> Option<RequestPriority> {
        self.clients.get(&provider).and_then(|c| c.last_priority())
    }
}

provider_queue_contract_tests!(StubProviderQueueHarness);

// =============================================================================
// B4: dispatch_enrichment threads context.priority to the provider client
// (packet-b4-priorities.md item 5 — "enrichment dispatch sends context.priority").
// =============================================================================

#[tokio::test]
async fn test_provider_queue_dispatch_sends_manual_context_priority_to_client() {
    let h = <StubProviderQueueHarness as ProviderQueueTestHarness>::setup(
        ProviderQueueScenario::NominalPopulatedOutcomes,
    )
    .await;

    let _ = h
        .queue()
        .dispatch_enrichment(h.work(), manual_context())
        .await
        .unwrap();

    assert_eq!(
        h.last_priority(MetadataProvider::Hardcover),
        Some(RequestPriority::High),
        "manual_context's High priority must reach the dispatched provider client"
    );
}

#[tokio::test]
async fn test_provider_queue_dispatch_sends_background_context_priority_to_client() {
    let h = <StubProviderQueueHarness as ProviderQueueTestHarness>::setup(
        ProviderQueueScenario::NominalPopulatedOutcomes,
    )
    .await;

    let _ = h
        .queue()
        .dispatch_enrichment(h.work(), background_context())
        .await
        .unwrap();

    assert_eq!(
        h.last_priority(MetadataProvider::Hardcover),
        Some(RequestPriority::Low),
        "background_context's Low priority must reach the dispatched provider client"
    );
}
