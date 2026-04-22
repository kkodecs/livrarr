#![doc = "Behavioral tests for ProviderRetryStateDb using real SQLite `:memory:` with full migrations."]
#![doc = "Covers R-20 and R-22 retry-state transitions, normalized payload persistence, suppression-window"]
#![doc = "semantics, due-retry listing, terminal-provider-row listing, reset behavior, and user isolation."]
#![allow(dead_code)]

use std::collections::HashSet;

use assert_matches::assert_matches;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use livrarr_db::*;
use livrarr_domain::{MetadataProvider, OutcomeClass, UserId, UserRole, WorkId};

#[async_trait]
pub trait DbTestHarness: Send + Sync {
    type Db: ProviderRetryStateDb + WorkDb;
    async fn setup() -> Self;
    fn db(&self) -> &Self::Db;
    fn user_ids(&self) -> (UserId, UserId);
}

fn make_work_req(user_id: UserId, title: &str, author: &str) -> CreateWorkDbRequest {
    CreateWorkDbRequest {
        user_id,
        title: title.to_string(),
        author_name: author.to_string(),
        author_id: None,
        ol_key: None,
        year: Some(2024),
        cover_url: Some("https://example.test/cover.jpg".to_string()),
        ..Default::default()
    }
}

async fn make_work<DB: WorkDb>(db: &DB, user_id: UserId, title: &str) -> WorkId {
    db.create_work(make_work_req(user_id, title, &format!("{title} Author")))
        .await
        .unwrap()
        .id
}

fn all_providers() -> [MetadataProvider; 5] {
    [
        MetadataProvider::Hardcover,
        MetadataProvider::OpenLibrary,
        MetadataProvider::Goodreads,
        MetadataProvider::Audnexus,
        MetadataProvider::Llm,
    ]
}

fn provider_set(
    rows: &[(WorkId, Vec<MetadataProvider>)],
    work_id: WorkId,
) -> HashSet<MetadataProvider> {
    rows.iter()
        .find(|(wid, _)| *wid == work_id)
        .map(|(_, providers)| providers.iter().copied().collect())
        .unwrap_or_default()
}

async fn assert_terminal_provider_results_contain_no_conflicts<DB: ProviderRetryStateDb>(
    db: &DB,
    user_id: UserId,
    results: &[(WorkId, Vec<MetadataProvider>)],
) {
    for (work_id, providers) in results {
        for provider in providers {
            let state = db
                .get_retry_state(user_id, *work_id, *provider)
                .await
                .unwrap()
                .expect("returned terminal provider must have a stored retry state");

            assert_ne!(
                state.last_outcome,
                Some(OutcomeClass::Conflict),
                "Conflict must never appear in terminal provider list"
            );
        }
    }
}

async fn record_provider_state<DB: ProviderRetryStateDb>(
    db: &DB,
    user_id: UserId,
    work_id: WorkId,
    provider: MetadataProvider,
    outcome: OutcomeClass,
    at: chrono::DateTime<chrono::Utc>,
) {
    match outcome {
        OutcomeClass::Success => {
            db.record_terminal_outcome(
                user_id,
                work_id,
                provider,
                OutcomeClass::Success,
                Some(format!(r#"{{"provider":"{:?}"}}"#, provider)),
            )
            .await
            .unwrap();
        }
        OutcomeClass::NotFound => {
            db.record_terminal_outcome(user_id, work_id, provider, OutcomeClass::NotFound, None)
                .await
                .unwrap();
        }
        OutcomeClass::PermanentFailure => {
            db.record_terminal_outcome(
                user_id,
                work_id,
                provider,
                OutcomeClass::PermanentFailure,
                None,
            )
            .await
            .unwrap();
        }
        OutcomeClass::Conflict => {
            db.record_terminal_outcome(user_id, work_id, provider, OutcomeClass::Conflict, None)
                .await
                .unwrap();
        }
        OutcomeClass::WillRetry => {
            db.record_will_retry(user_id, work_id, provider, at)
                .await
                .unwrap();
        }
        OutcomeClass::Suppressed => {
            db.record_suppressed(user_id, work_id, provider, at)
                .await
                .unwrap();
        }
        OutcomeClass::NotConfigured => {
            db.record_terminal_outcome(
                user_id,
                work_id,
                provider,
                OutcomeClass::NotConfigured,
                None,
            )
            .await
            .unwrap();
        }
    }
}

async fn seed_all_non_conflict_terminal_rows<DB: ProviderRetryStateDb>(
    db: &DB,
    user_id: UserId,
    work_id: WorkId,
) {
    let at = Utc::now() + Duration::minutes(30);
    let rows = [
        (MetadataProvider::Hardcover, OutcomeClass::Success),
        (MetadataProvider::OpenLibrary, OutcomeClass::NotFound),
        (MetadataProvider::Goodreads, OutcomeClass::PermanentFailure),
        (MetadataProvider::Audnexus, OutcomeClass::Success),
        (MetadataProvider::Llm, OutcomeClass::NotFound),
    ];

    for (provider, outcome) in rows {
        record_provider_state(db, user_id, work_id, provider, outcome, at).await;
    }
}

async fn assert_get_retry_state_returns_none_before_any_record<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "none-before-any-record").await;

    let state = db
        .get_retry_state(user_id, work_id, MetadataProvider::Goodreads)
        .await
        .unwrap();

    assert!(state.is_none());
}

async fn assert_get_retry_state_returns_some_after_first_write<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "some-after-first-write").await;
    let next_attempt_at = Utc::now() + Duration::minutes(10);

    db.record_will_retry(
        user_id,
        work_id,
        MetadataProvider::Goodreads,
        next_attempt_at,
    )
    .await
    .unwrap();

    let state = db
        .get_retry_state(user_id, work_id, MetadataProvider::Goodreads)
        .await
        .unwrap();

    assert!(state.is_some());
}

async fn assert_record_will_retry_increments_attempts_by_one<DB: ProviderRetryStateDb + WorkDb>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "will-retry-increments-attempts").await;
    let first_next = Utc::now() + Duration::minutes(10);
    let second_next = Utc::now() + Duration::minutes(20);

    let first = db
        .record_will_retry(user_id, work_id, MetadataProvider::Hardcover, first_next)
        .await
        .unwrap();
    let second = db
        .record_will_retry(user_id, work_id, MetadataProvider::Hardcover, second_next)
        .await
        .unwrap();

    assert_eq!(first.attempts, 1);
    assert_eq!(second.attempts, 2);
    assert_eq!(second.last_outcome, Some(OutcomeClass::WillRetry));
    assert_eq!(second.next_attempt_at, Some(second_next));
}

async fn assert_record_will_retry_clears_normalized_payload_json<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "will-retry-clears-payload").await;
    let payload = r#"{"title":"payload"}"#.to_string();

    db.record_terminal_outcome(
        user_id,
        work_id,
        MetadataProvider::Hardcover,
        OutcomeClass::Success,
        Some(payload.clone()),
    )
    .await
    .unwrap();

    let retry = db
        .record_will_retry(
            user_id,
            work_id,
            MetadataProvider::Hardcover,
            Utc::now() + Duration::minutes(15),
        )
        .await
        .unwrap();

    assert_eq!(retry.normalized_payload_json, None);

    let stored = db
        .get_retry_state(user_id, work_id, MetadataProvider::Hardcover)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(stored.normalized_payload_json, None);
}

async fn assert_record_will_retry_clears_first_suppressed_at<DB: ProviderRetryStateDb + WorkDb>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "will-retry-clears-first-suppressed-at").await;

    let suppressed = db
        .record_suppressed(
            user_id,
            work_id,
            MetadataProvider::OpenLibrary,
            Utc::now() + Duration::minutes(10),
        )
        .await
        .unwrap();

    assert!(suppressed.first_suppressed_at.is_some());

    let retry = db
        .record_will_retry(
            user_id,
            work_id,
            MetadataProvider::OpenLibrary,
            Utc::now() + Duration::minutes(20),
        )
        .await
        .unwrap();

    assert_eq!(retry.first_suppressed_at, None);
}

async fn assert_record_suppressed_increments_suppressed_passes_without_incrementing_attempts<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "suppressed-does-not-increment-attempts").await;

    db.record_will_retry(
        user_id,
        work_id,
        MetadataProvider::Goodreads,
        Utc::now() + Duration::minutes(5),
    )
    .await
    .unwrap();

    let until = Utc::now() + Duration::minutes(15);
    let suppressed = db
        .record_suppressed(user_id, work_id, MetadataProvider::Goodreads, until)
        .await
        .unwrap();

    assert_eq!(suppressed.attempts, 1);
    assert_eq!(suppressed.suppressed_passes, 1);
    assert_eq!(suppressed.last_outcome, Some(OutcomeClass::Suppressed));
    assert_eq!(suppressed.next_attempt_at, Some(until));
}

async fn assert_record_suppressed_sets_first_suppressed_at_when_none<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "suppressed-sets-first-window").await;
    let before = Utc::now();
    let state = db
        .record_suppressed(
            user_id,
            work_id,
            MetadataProvider::Audnexus,
            Utc::now() + Duration::minutes(10),
        )
        .await
        .unwrap();
    let after = Utc::now();

    let first = state
        .first_suppressed_at
        .expect("first_suppressed_at should be set on first suppression");

    assert!(first >= before - Duration::seconds(1));
    assert!(first <= after + Duration::seconds(1));
}

async fn assert_record_suppressed_preserves_existing_first_suppressed_at<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "suppressed-preserves-first-window").await;

    let first = db
        .record_suppressed(
            user_id,
            work_id,
            MetadataProvider::Audnexus,
            Utc::now() + Duration::minutes(10),
        )
        .await
        .unwrap()
        .first_suppressed_at
        .expect("first_suppressed_at should be set on first suppression");

    let second = db
        .record_suppressed(
            user_id,
            work_id,
            MetadataProvider::Audnexus,
            Utc::now() + Duration::minutes(20),
        )
        .await
        .unwrap();

    assert_eq!(second.first_suppressed_at, Some(first));
    assert_eq!(second.suppressed_passes, 2);
}

async fn assert_record_terminal_outcome_success_persists_normalized_payload_json<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "terminal-success-persists-payload").await;
    let payload = r#"{"title":"Persisted","gr_key":"show/123"}"#.to_string();

    db.record_terminal_outcome(
        user_id,
        work_id,
        MetadataProvider::Goodreads,
        OutcomeClass::Success,
        Some(payload.clone()),
    )
    .await
    .unwrap();

    let state = db
        .get_retry_state(user_id, work_id, MetadataProvider::Goodreads)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(state.last_outcome, Some(OutcomeClass::Success));
    assert_eq!(
        state.normalized_payload_json.as_deref(),
        Some(payload.as_str())
    );
    assert_eq!(state.next_attempt_at, None);
}

async fn assert_record_terminal_outcome_non_success_rejects_normalized_payload_json<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "terminal-non-success-rejects-payload").await;

    let result = db
        .record_terminal_outcome(
            user_id,
            work_id,
            MetadataProvider::Goodreads,
            OutcomeClass::NotFound,
            Some(r#"{"title":"invalid"}"#.to_string()),
        )
        .await;

    assert_matches!(result, Err(_));

    let state = db
        .get_retry_state(user_id, work_id, MetadataProvider::Goodreads)
        .await
        .unwrap();

    assert!(state
        .map(|s| {
            !(s.last_outcome == Some(OutcomeClass::NotFound) && s.normalized_payload_json.is_some())
        })
        .unwrap_or(true));
}

async fn assert_record_terminal_outcome_non_success_rejects_normalized_payload_json_for_all_terminal_non_success_classes<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    for outcome in [
        OutcomeClass::NotFound,
        OutcomeClass::PermanentFailure,
        OutcomeClass::Conflict,
    ] {
        let work_id = make_work(
            db,
            user_id,
            &format!("terminal-non-success-rejects-payload-{outcome:?}"),
        )
        .await;

        let result = db
            .record_terminal_outcome(
                user_id,
                work_id,
                MetadataProvider::Goodreads,
                outcome,
                Some(r#"{"title":"invalid"}"#.to_string()),
            )
            .await;

        assert_matches!(result, Err(_));

        let state = db
            .get_retry_state(user_id, work_id, MetadataProvider::Goodreads)
            .await
            .unwrap();

        assert!(state
            .map(|s| !(s.last_outcome == Some(outcome) && s.normalized_payload_json.is_some()))
            .unwrap_or(true));
    }
}

async fn assert_record_terminal_outcome_success_requires_normalized_payload_json<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "terminal-success-requires-payload").await;

    let result = db
        .record_terminal_outcome(
            user_id,
            work_id,
            MetadataProvider::Goodreads,
            OutcomeClass::Success,
            None,
        )
        .await;

    assert_matches!(result, Err(_));

    let state = db
        .get_retry_state(user_id, work_id, MetadataProvider::Goodreads)
        .await
        .unwrap();

    assert!(state
        .map(|s| {
            !(s.last_outcome == Some(OutcomeClass::Success) && s.normalized_payload_json.is_none())
        })
        .unwrap_or(true));
}

async fn assert_record_terminal_outcome_rejects_non_terminal_outcome_classes<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    for outcome in [OutcomeClass::WillRetry, OutcomeClass::Suppressed] {
        let work_id = make_work(
            db,
            user_id,
            &format!("terminal-method-rejects-non-terminal-{outcome:?}"),
        )
        .await;

        let result = db
            .record_terminal_outcome(user_id, work_id, MetadataProvider::Goodreads, outcome, None)
            .await;

        assert_matches!(result, Err(_));

        let state = db
            .get_retry_state(user_id, work_id, MetadataProvider::Goodreads)
            .await
            .unwrap();

        assert!(state.is_none());
    }
}

async fn assert_record_terminal_outcome_clears_next_attempt_at_and_first_suppressed_at<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "terminal-clears-scheduling-window").await;

    let suppressed = db
        .record_suppressed(
            user_id,
            work_id,
            MetadataProvider::OpenLibrary,
            Utc::now() + Duration::minutes(10),
        )
        .await
        .unwrap();

    assert!(suppressed.next_attempt_at.is_some());
    assert!(suppressed.first_suppressed_at.is_some());

    db.record_terminal_outcome(
        user_id,
        work_id,
        MetadataProvider::OpenLibrary,
        OutcomeClass::PermanentFailure,
        None,
    )
    .await
    .unwrap();

    let state = db
        .get_retry_state(user_id, work_id, MetadataProvider::OpenLibrary)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(state.last_outcome, Some(OutcomeClass::PermanentFailure));
    assert_eq!(state.next_attempt_at, None);
    assert_eq!(state.first_suppressed_at, None);
}

async fn assert_not_configured_reset_flow<DB: ProviderRetryStateDb + WorkDb>(
    db: &DB,
    user_id: UserId,
) {
    let work = make_work(db, user_id, "not-configured-reset").await;

    // Step 1: Record NotConfigured terminal outcome for Hardcover.
    db.record_terminal_outcome(
        user_id,
        work,
        MetadataProvider::Hardcover,
        OutcomeClass::NotConfigured,
        None,
    )
    .await
    .unwrap();

    // Also record a normal terminal for another provider (should be untouched).
    db.record_terminal_outcome(
        user_id,
        work,
        MetadataProvider::OpenLibrary,
        OutcomeClass::Success,
        Some(r#"{"provider":"OpenLibrary"}"#.to_string()),
    )
    .await
    .unwrap();

    // Verify both rows exist.
    let states = db.list_retry_states(user_id, work).await.unwrap();
    assert_eq!(states.len(), 2, "should have 2 retry state rows");

    let hc = states
        .iter()
        .find(|s| s.provider == MetadataProvider::Hardcover)
        .expect("Hardcover row must exist");
    assert_eq!(hc.last_outcome, Some(OutcomeClass::NotConfigured));

    // Step 2: Reset NotConfigured outcomes for Hardcover (simulates config save).
    let deleted = db
        .reset_not_configured_outcomes(MetadataProvider::Hardcover)
        .await
        .unwrap();
    assert_eq!(deleted, 1, "should delete exactly 1 NotConfigured row");

    // Step 3: Verify Hardcover row is gone but OpenLibrary row remains.
    let states_after = db.list_retry_states(user_id, work).await.unwrap();
    assert_eq!(states_after.len(), 1, "only OpenLibrary row should remain");
    assert_eq!(states_after[0].provider, MetadataProvider::OpenLibrary);

    // Step 4: Second reset is a no-op.
    let deleted_again = db
        .reset_not_configured_outcomes(MetadataProvider::Hardcover)
        .await
        .unwrap();
    assert_eq!(deleted_again, 0, "no rows to delete on second call");
}

async fn assert_reset_all_retry_states_deletes_all_rows_for_work<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let work_a = make_work(db, user_id, "reset-clears-work-a").await;
    let work_b = make_work(db, user_id, "reset-preserves-work-b").await;

    db.record_will_retry(
        user_id,
        work_a,
        MetadataProvider::Hardcover,
        Utc::now() + Duration::minutes(10),
    )
    .await
    .unwrap();
    db.record_suppressed(
        user_id,
        work_a,
        MetadataProvider::Goodreads,
        Utc::now() + Duration::minutes(20),
    )
    .await
    .unwrap();
    db.record_will_retry(
        user_id,
        work_b,
        MetadataProvider::OpenLibrary,
        Utc::now() + Duration::minutes(30),
    )
    .await
    .unwrap();

    assert_eq!(
        db.list_retry_states(user_id, work_a).await.unwrap().len(),
        2
    );

    db.reset_all_retry_states(user_id, work_a).await.unwrap();

    assert!(db
        .list_retry_states(user_id, work_a)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(
        db.list_retry_states(user_id, work_b).await.unwrap().len(),
        1
    );
}

async fn assert_list_works_due_for_retry_returns_due_will_retry_and_suppressed_only<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let due_will_retry = make_work(db, user_id, "due-will-retry").await;
    let due_suppressed = make_work(db, user_id, "due-suppressed").await;
    let future_will_retry = make_work(db, user_id, "future-will-retry").await;
    let terminal_success = make_work(db, user_id, "terminal-success").await;

    let base = Utc::now();
    let due_at = base + Duration::minutes(10);
    let future_at = base + Duration::minutes(30);
    let query_now = due_at + Duration::seconds(1);

    db.record_will_retry(user_id, due_will_retry, MetadataProvider::Hardcover, due_at)
        .await
        .unwrap();
    db.record_suppressed(
        user_id,
        due_suppressed,
        MetadataProvider::OpenLibrary,
        due_at,
    )
    .await
    .unwrap();
    db.record_will_retry(
        user_id,
        future_will_retry,
        MetadataProvider::Goodreads,
        future_at,
    )
    .await
    .unwrap();
    db.record_terminal_outcome(
        user_id,
        terminal_success,
        MetadataProvider::Audnexus,
        OutcomeClass::Success,
        Some(r#"{"title":"done"}"#.to_string()),
    )
    .await
    .unwrap();

    let got: HashSet<_> = db
        .list_works_due_for_retry(user_id, query_now)
        .await
        .unwrap()
        .into_iter()
        .collect();

    let expected: HashSet<_> = [
        (due_will_retry, MetadataProvider::Hardcover),
        (due_suppressed, MetadataProvider::OpenLibrary),
    ]
    .into_iter()
    .collect();

    assert_eq!(got, expected);
}

async fn assert_list_works_due_for_retry_includes_rows_when_query_now_equals_next_attempt_at<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let due_will_retry = make_work(db, user_id, "due-boundary-will-retry").await;
    let due_suppressed = make_work(db, user_id, "due-boundary-suppressed").await;
    let next_attempt_at = Utc::now() + Duration::minutes(10);
    let query_now = next_attempt_at;

    db.record_will_retry(
        user_id,
        due_will_retry,
        MetadataProvider::Hardcover,
        next_attempt_at,
    )
    .await
    .unwrap();
    db.record_suppressed(
        user_id,
        due_suppressed,
        MetadataProvider::OpenLibrary,
        next_attempt_at,
    )
    .await
    .unwrap();

    let got: HashSet<_> = db
        .list_works_due_for_retry(user_id, query_now)
        .await
        .unwrap()
        .into_iter()
        .collect();

    let expected: HashSet<_> = [
        (due_will_retry, MetadataProvider::Hardcover),
        (due_suppressed, MetadataProvider::OpenLibrary),
    ]
    .into_iter()
    .collect();

    assert_eq!(got, expected);
}

async fn assert_list_retry_states_returns_all_rows_for_work<DB: ProviderRetryStateDb + WorkDb>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(db, user_id, "list-retry-states-all-providers").await;
    let at = Utc::now() + Duration::minutes(30);

    let seeded = [
        (MetadataProvider::Hardcover, OutcomeClass::Success),
        (MetadataProvider::OpenLibrary, OutcomeClass::NotFound),
        (MetadataProvider::Goodreads, OutcomeClass::PermanentFailure),
        (MetadataProvider::Audnexus, OutcomeClass::WillRetry),
        (MetadataProvider::Llm, OutcomeClass::Suppressed),
    ];

    for (provider, outcome) in seeded {
        record_provider_state(db, user_id, work_id, provider, outcome, at).await;
    }

    let got = db.list_retry_states(user_id, work_id).await.unwrap();

    assert_eq!(got.len(), seeded.len());

    let got_pairs: HashSet<_> = got
        .into_iter()
        .map(|row| (row.work_id, row.provider, row.last_outcome))
        .collect();

    let expected_pairs: HashSet<_> = seeded
        .into_iter()
        .map(|(provider, outcome)| (work_id, provider, Some(outcome)))
        .collect();

    assert_eq!(got_pairs, expected_pairs);
}

async fn assert_list_retry_states_isolates_by_user_id<DB: ProviderRetryStateDb + WorkDb>(
    db: &DB,
    user1: UserId,
    user2: UserId,
) {
    let user1_work = make_work(db, user1, "list-retry-states-user1-work").await;
    let user2_work = make_work(db, user2, "list-retry-states-user2-work").await;
    let at = Utc::now() + Duration::minutes(30);

    record_provider_state(
        db,
        user1,
        user1_work,
        MetadataProvider::Hardcover,
        OutcomeClass::Success,
        at,
    )
    .await;
    record_provider_state(
        db,
        user1,
        user1_work,
        MetadataProvider::Goodreads,
        OutcomeClass::WillRetry,
        at,
    )
    .await;
    record_provider_state(
        db,
        user2,
        user2_work,
        MetadataProvider::OpenLibrary,
        OutcomeClass::NotFound,
        at,
    )
    .await;
    record_provider_state(
        db,
        user2,
        user2_work,
        MetadataProvider::Audnexus,
        OutcomeClass::Suppressed,
        at,
    )
    .await;

    let user1_rows = db.list_retry_states(user1, user1_work).await.unwrap();
    let user2_rows = db.list_retry_states(user2, user2_work).await.unwrap();
    let wrong_user_for_user1_work = db.list_retry_states(user2, user1_work).await.unwrap();
    let wrong_user_for_user2_work = db.list_retry_states(user1, user2_work).await.unwrap();

    let user1_got: HashSet<_> = user1_rows
        .into_iter()
        .map(|row| (row.work_id, row.provider, row.last_outcome))
        .collect();
    let user2_got: HashSet<_> = user2_rows
        .into_iter()
        .map(|row| (row.work_id, row.provider, row.last_outcome))
        .collect();

    let user1_expected: HashSet<_> = [
        (
            user1_work,
            MetadataProvider::Hardcover,
            Some(OutcomeClass::Success),
        ),
        (
            user1_work,
            MetadataProvider::Goodreads,
            Some(OutcomeClass::WillRetry),
        ),
    ]
    .into_iter()
    .collect();

    let user2_expected: HashSet<_> = [
        (
            user2_work,
            MetadataProvider::OpenLibrary,
            Some(OutcomeClass::NotFound),
        ),
        (
            user2_work,
            MetadataProvider::Audnexus,
            Some(OutcomeClass::Suppressed),
        ),
    ]
    .into_iter()
    .collect();

    assert_eq!(user1_got, user1_expected);
    assert_eq!(user2_got, user2_expected);
    assert!(wrong_user_for_user1_work.is_empty());
    assert!(wrong_user_for_user2_work.is_empty());
}

async fn assert_list_works_with_terminal_provider_rows_includes_only_all_non_conflict_terminal_works<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let terminal_work = make_work(db, user_id, "all-terminal-included").await;
    let non_terminal_work = make_work(db, user_id, "non-terminal-excluded").await;

    seed_all_non_conflict_terminal_rows(db, user_id, terminal_work).await;

    db.record_terminal_outcome(
        user_id,
        non_terminal_work,
        MetadataProvider::Hardcover,
        OutcomeClass::Success,
        Some(r#"{"title":"done"}"#.to_string()),
    )
    .await
    .unwrap();
    db.record_will_retry(
        user_id,
        non_terminal_work,
        MetadataProvider::OpenLibrary,
        Utc::now() + Duration::minutes(30),
    )
    .await
    .unwrap();

    let got = db
        .list_works_with_terminal_provider_rows(user_id)
        .await
        .unwrap();

    assert_terminal_provider_results_contain_no_conflicts(db, user_id, &got).await;

    assert!(got.iter().any(|(wid, _)| *wid == terminal_work));
    assert!(!got.iter().any(|(wid, _)| *wid == non_terminal_work));

    let expected_terminal_providers: HashSet<_> = all_providers().into_iter().collect();
    let actual_terminal_providers = provider_set(&got, terminal_work);

    assert_eq!(actual_terminal_providers, expected_terminal_providers);

    for (work_id, providers) in &got {
        assert!(!providers.is_empty());
        if *work_id == terminal_work {
            let set: HashSet<_> = providers.iter().copied().collect();
            assert_eq!(set, expected_terminal_providers);
        }
    }
}

async fn assert_list_works_with_terminal_provider_rows_returns_terminal_provider_set_for_full_terminal_work<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let terminal_work = make_work(db, user_id, "all-terminal-provider-set-full").await;

    seed_all_non_conflict_terminal_rows(db, user_id, terminal_work).await;

    let got = db
        .list_works_with_terminal_provider_rows(user_id)
        .await
        .unwrap();

    assert_terminal_provider_results_contain_no_conflicts(db, user_id, &got).await;

    let expected: HashSet<_> = all_providers().into_iter().collect();
    let actual = provider_set(&got, terminal_work);

    assert_eq!(actual, expected);
}

async fn assert_list_works_with_terminal_provider_rows_includes_partial_provider_set_when_all_rows_are_terminal<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let partial_terminal_work = make_work(db, user_id, "all-terminal-partial-provider-set").await;

    db.record_terminal_outcome(
        user_id,
        partial_terminal_work,
        MetadataProvider::Hardcover,
        OutcomeClass::Success,
        Some(r#"{"title":"done"}"#.to_string()),
    )
    .await
    .unwrap();
    db.record_terminal_outcome(
        user_id,
        partial_terminal_work,
        MetadataProvider::OpenLibrary,
        OutcomeClass::NotFound,
        None,
    )
    .await
    .unwrap();

    let got = db
        .list_works_with_terminal_provider_rows(user_id)
        .await
        .unwrap();

    assert_terminal_provider_results_contain_no_conflicts(db, user_id, &got).await;

    assert!(got.iter().any(|(wid, _)| *wid == partial_terminal_work));
}

async fn assert_list_works_with_terminal_provider_rows_returns_only_terminal_non_conflict_providers_for_partial_work<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let partial_terminal_work = make_work(db, user_id, "all-terminal-partial-provider-vec").await;

    db.record_terminal_outcome(
        user_id,
        partial_terminal_work,
        MetadataProvider::Hardcover,
        OutcomeClass::Success,
        Some(r#"{"title":"done"}"#.to_string()),
    )
    .await
    .unwrap();
    db.record_terminal_outcome(
        user_id,
        partial_terminal_work,
        MetadataProvider::OpenLibrary,
        OutcomeClass::NotFound,
        None,
    )
    .await
    .unwrap();

    let got = db
        .list_works_with_terminal_provider_rows(user_id)
        .await
        .unwrap();

    assert_terminal_provider_results_contain_no_conflicts(db, user_id, &got).await;

    let actual = provider_set(&got, partial_terminal_work);
    let expected: HashSet<_> = [MetadataProvider::Hardcover, MetadataProvider::OpenLibrary]
        .into_iter()
        .collect();

    assert_eq!(actual, expected);
}

async fn assert_list_works_with_terminal_provider_rows_works_with_terminal_provider_rows_for_single_work_with_three_terminal_non_conflict_rows<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let work_id = make_work(
        db,
        user_id,
        "terminal-three-providers-success-notfound-permanentfailure",
    )
    .await;

    db.record_terminal_outcome(
        user_id,
        work_id,
        MetadataProvider::Hardcover,
        OutcomeClass::Success,
        Some(r#"{"title":"done"}"#.to_string()),
    )
    .await
    .unwrap();
    db.record_terminal_outcome(
        user_id,
        work_id,
        MetadataProvider::OpenLibrary,
        OutcomeClass::NotFound,
        None,
    )
    .await
    .unwrap();
    db.record_terminal_outcome(
        user_id,
        work_id,
        MetadataProvider::Goodreads,
        OutcomeClass::PermanentFailure,
        None,
    )
    .await
    .unwrap();

    let got = db
        .list_works_with_terminal_provider_rows(user_id)
        .await
        .unwrap();

    assert_terminal_provider_results_contain_no_conflicts(db, user_id, &got).await;

    assert!(got.iter().any(|(wid, _)| *wid == work_id));

    let providers = got
        .iter()
        .find(|(wid, _)| *wid == work_id)
        .map(|(_, providers)| providers.clone())
        .expect("work should be present in terminal-provider-row listing");

    assert_eq!(providers.len(), 3);

    let provider_set: HashSet<_> = providers.iter().copied().collect();
    let expected: HashSet<_> = [
        MetadataProvider::Hardcover,
        MetadataProvider::OpenLibrary,
        MetadataProvider::Goodreads,
    ]
    .into_iter()
    .collect();

    assert_eq!(provider_set, expected);
    assert!(!provider_set.contains(&MetadataProvider::Audnexus));
    assert!(!provider_set.contains(&MetadataProvider::Llm));
}

async fn assert_list_works_with_terminal_provider_rows_isolated_by_user_id<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user1: UserId,
    user2: UserId,
) {
    let user1_terminal = make_work(db, user1, "all-terminal-user1-only").await;
    let user2_terminal = make_work(db, user2, "all-terminal-user2-only").await;

    db.record_terminal_outcome(
        user1,
        user1_terminal,
        MetadataProvider::Hardcover,
        OutcomeClass::Success,
        Some(r#"{"title":"user1"}"#.to_string()),
    )
    .await
    .unwrap();
    db.record_terminal_outcome(
        user1,
        user1_terminal,
        MetadataProvider::OpenLibrary,
        OutcomeClass::NotFound,
        None,
    )
    .await
    .unwrap();

    db.record_terminal_outcome(
        user2,
        user2_terminal,
        MetadataProvider::Goodreads,
        OutcomeClass::Success,
        Some(r#"{"title":"user2"}"#.to_string()),
    )
    .await
    .unwrap();
    db.record_terminal_outcome(
        user2,
        user2_terminal,
        MetadataProvider::Audnexus,
        OutcomeClass::PermanentFailure,
        None,
    )
    .await
    .unwrap();

    let user1_got = db
        .list_works_with_terminal_provider_rows(user1)
        .await
        .unwrap();
    let user2_got = db
        .list_works_with_terminal_provider_rows(user2)
        .await
        .unwrap();

    assert_terminal_provider_results_contain_no_conflicts(db, user1, &user1_got).await;
    assert_terminal_provider_results_contain_no_conflicts(db, user2, &user2_got).await;

    assert!(user1_got.iter().any(|(wid, _)| *wid == user1_terminal));
    assert!(!user1_got.iter().any(|(wid, _)| *wid == user2_terminal));
    assert!(user2_got.iter().any(|(wid, _)| *wid == user2_terminal));
    assert!(!user2_got.iter().any(|(wid, _)| *wid == user1_terminal));

    let user1_expected: HashSet<_> = [MetadataProvider::Hardcover, MetadataProvider::OpenLibrary]
        .into_iter()
        .collect();
    let user2_expected: HashSet<_> = [MetadataProvider::Goodreads, MetadataProvider::Audnexus]
        .into_iter()
        .collect();

    for (work_id, providers) in &user1_got {
        let set: HashSet<_> = providers.iter().copied().collect();
        if *work_id == user1_terminal {
            assert_eq!(set, user1_expected);
        }
    }

    for (work_id, providers) in &user2_got {
        let set: HashSet<_> = providers.iter().copied().collect();
        if *work_id == user2_terminal {
            assert_eq!(set, user2_expected);
        }
    }
}

async fn assert_list_works_with_terminal_provider_rows_excludes_conflict_rows<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let included = make_work(db, user_id, "terminal-included-without-conflict").await;
    let conflict = make_work(db, user_id, "terminal-excluded-with-conflict").await;
    let at = Utc::now() + Duration::minutes(30);

    seed_all_non_conflict_terminal_rows(db, user_id, included).await;

    for provider in all_providers() {
        let outcome = if provider == MetadataProvider::Goodreads {
            OutcomeClass::Conflict
        } else {
            OutcomeClass::Success
        };
        record_provider_state(db, user_id, conflict, provider, outcome, at).await;
    }

    let got = db
        .list_works_with_terminal_provider_rows(user_id)
        .await
        .unwrap();

    assert_terminal_provider_results_contain_no_conflicts(db, user_id, &got).await;

    assert!(got.iter().any(|(wid, _)| *wid == included));
    assert!(!got.iter().any(|(wid, _)| *wid == conflict));

    let expected_included: HashSet<_> = all_providers().into_iter().collect();
    for (work_id, providers) in &got {
        let set: HashSet<_> = providers.iter().copied().collect();
        if *work_id == included {
            assert_eq!(set, expected_included);
        }
    }
}

async fn assert_list_works_with_terminal_provider_rows_excludes_work_with_any_conflict_row<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let non_conflict_terminal =
        make_work(db, user_id, "terminal-included-no-conflict-control").await;
    let mixed_terminal_with_conflict =
        make_work(db, user_id, "terminal-excluded-any-conflict-row").await;

    seed_all_non_conflict_terminal_rows(db, user_id, non_conflict_terminal).await;

    db.record_terminal_outcome(
        user_id,
        mixed_terminal_with_conflict,
        MetadataProvider::Hardcover,
        OutcomeClass::Success,
        Some(r#"{"title":"done"}"#.to_string()),
    )
    .await
    .unwrap();
    db.record_terminal_outcome(
        user_id,
        mixed_terminal_with_conflict,
        MetadataProvider::OpenLibrary,
        OutcomeClass::PermanentFailure,
        None,
    )
    .await
    .unwrap();
    db.record_terminal_outcome(
        user_id,
        mixed_terminal_with_conflict,
        MetadataProvider::Goodreads,
        OutcomeClass::NotFound,
        None,
    )
    .await
    .unwrap();
    db.record_terminal_outcome(
        user_id,
        mixed_terminal_with_conflict,
        MetadataProvider::Audnexus,
        OutcomeClass::Conflict,
        None,
    )
    .await
    .unwrap();

    let got = db
        .list_works_with_terminal_provider_rows(user_id)
        .await
        .unwrap();

    assert_terminal_provider_results_contain_no_conflicts(db, user_id, &got).await;

    assert!(got.iter().any(|(wid, _)| *wid == non_conflict_terminal));
    assert!(!got
        .iter()
        .any(|(wid, _)| *wid == mixed_terminal_with_conflict));
}

async fn assert_list_works_with_terminal_provider_rows_excludes_works_with_no_rows<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let included = make_work(db, user_id, "terminal-included-with-rows").await;
    let never_attempted = make_work(db, user_id, "terminal-excluded-no-rows").await;

    seed_all_non_conflict_terminal_rows(db, user_id, included).await;

    let got = db
        .list_works_with_terminal_provider_rows(user_id)
        .await
        .unwrap();

    assert_terminal_provider_results_contain_no_conflicts(db, user_id, &got).await;

    assert!(got.iter().any(|(wid, _)| *wid == included));
    assert!(!got.iter().any(|(wid, _)| *wid == never_attempted));
}

async fn assert_list_works_with_terminal_provider_rows_excludes_in_flight_rows<
    DB: ProviderRetryStateDb + WorkDb,
>(
    db: &DB,
    user_id: UserId,
) {
    let included = make_work(db, user_id, "terminal-included-all-terminal").await;
    let retrying = make_work(db, user_id, "terminal-excluded-will-retry").await;
    let suppressed = make_work(db, user_id, "terminal-excluded-suppressed").await;
    let at = Utc::now() + Duration::minutes(30);

    seed_all_non_conflict_terminal_rows(db, user_id, included).await;

    for provider in all_providers() {
        let outcome = if provider == MetadataProvider::OpenLibrary {
            OutcomeClass::WillRetry
        } else {
            OutcomeClass::Success
        };
        record_provider_state(db, user_id, retrying, provider, outcome, at).await;
    }

    for provider in all_providers() {
        let outcome = if provider == MetadataProvider::Audnexus {
            OutcomeClass::Suppressed
        } else {
            OutcomeClass::Success
        };
        record_provider_state(db, user_id, suppressed, provider, outcome, at).await;
    }

    let got = db
        .list_works_with_terminal_provider_rows(user_id)
        .await
        .unwrap();

    assert_terminal_provider_results_contain_no_conflicts(db, user_id, &got).await;

    assert!(got.iter().any(|(wid, _)| *wid == included));
    assert!(!got.iter().any(|(wid, _)| *wid == retrying));
    assert!(!got.iter().any(|(wid, _)| *wid == suppressed));

    let expected_included: HashSet<_> = all_providers().into_iter().collect();
    for (work_id, providers) in &got {
        let set: HashSet<_> = providers.iter().copied().collect();
        if *work_id == included {
            assert_eq!(set, expected_included);
        }
    }
}

async fn assert_retry_state_is_isolated_by_user_id<DB: ProviderRetryStateDb + WorkDb>(
    db: &DB,
    user1: UserId,
    user2: UserId,
) {
    let work1 = make_work(db, user1, "user-one-retry-state").await;
    let work2 = make_work(db, user2, "user-two-retry-state").await;
    let due_at = Utc::now() + Duration::minutes(10);
    let query_now = due_at + Duration::seconds(1);

    db.record_will_retry(user1, work1, MetadataProvider::Hardcover, due_at)
        .await
        .unwrap();
    db.record_suppressed(user2, work2, MetadataProvider::OpenLibrary, due_at)
        .await
        .unwrap();

    let wrong_user_state = db
        .get_retry_state(user2, work1, MetadataProvider::Hardcover)
        .await
        .unwrap();
    assert!(wrong_user_state.is_none());

    let user1_due: HashSet<_> = db
        .list_works_due_for_retry(user1, query_now)
        .await
        .unwrap()
        .into_iter()
        .collect();
    let user2_due: HashSet<_> = db
        .list_works_due_for_retry(user2, query_now)
        .await
        .unwrap()
        .into_iter()
        .collect();

    let expected_user1: HashSet<_> = [(work1, MetadataProvider::Hardcover)].into_iter().collect();
    let expected_user2: HashSet<_> = [(work2, MetadataProvider::OpenLibrary)]
        .into_iter()
        .collect();

    assert_eq!(user1_due, expected_user1);
    assert_eq!(user2_due, expected_user2);
}

#[macro_export]
macro_rules! retry_state_db_tests {
    ($harness:ty) => {
        #[tokio::test]
        async fn test_retry_state_db_get_retry_state_returns_none_before_any_record() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::get_retry_state | Behavior: returns None before any retry state is recorded
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_get_retry_state_returns_none_before_any_record(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_get_retry_state_returns_some_after_first_write() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::get_retry_state | Behavior: returns Some after the first state write
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_get_retry_state_returns_some_after_first_write(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_record_will_retry_increments_attempts_by_one() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::record_will_retry | Behavior: increments attempts by exactly 1 on each WillRetry transition
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_record_will_retry_increments_attempts_by_one(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_record_will_retry_clears_normalized_payload_json() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::record_will_retry | Behavior: clears normalized_payload_json when transitioning to WillRetry
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_record_will_retry_clears_normalized_payload_json(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_record_will_retry_clears_first_suppressed_at() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::record_will_retry | Behavior: clears first_suppressed_at when transitioning from Suppressed to WillRetry
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_record_will_retry_clears_first_suppressed_at(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_record_suppressed_increments_suppressed_passes_without_incrementing_attempts() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::record_suppressed | Behavior: increments suppressed_passes while leaving attempts unchanged
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_record_suppressed_increments_suppressed_passes_without_incrementing_attempts(
                db, u1,
            )
            .await;
        }

        #[tokio::test]
        async fn test_retry_state_db_record_suppressed_sets_first_suppressed_at_when_none() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::record_suppressed | Behavior: sets first_suppressed_at on the first suppression in a window
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_record_suppressed_sets_first_suppressed_at_when_none(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_record_suppressed_preserves_existing_first_suppressed_at() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::record_suppressed | Behavior: preserves the original first_suppressed_at on subsequent suppressions
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_record_suppressed_preserves_existing_first_suppressed_at(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_record_terminal_outcome_success_persists_normalized_payload_json() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::record_terminal_outcome | Behavior: Success persists normalized_payload_json for deferred merge reconstruction
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_record_terminal_outcome_success_persists_normalized_payload_json(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_record_terminal_outcome_non_success_rejects_normalized_payload_json() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::record_terminal_outcome | Behavior: non-Success terminal outcomes must not accept normalized_payload_json
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_record_terminal_outcome_non_success_rejects_normalized_payload_json(db, u1)
                .await;
        }

        #[tokio::test]
        async fn test_retry_state_db_record_terminal_outcome_non_success_rejects_normalized_payload_json_for_all_terminal_non_success_classes() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::record_terminal_outcome | Behavior: NotFound, PermanentFailure, and Conflict must each reject normalized_payload_json
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_record_terminal_outcome_non_success_rejects_normalized_payload_json_for_all_terminal_non_success_classes(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_record_terminal_outcome_success_requires_normalized_payload_json() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::record_terminal_outcome | Behavior: Success without normalized_payload_json is invalid and must not persist an invalid state
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_record_terminal_outcome_success_requires_normalized_payload_json(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_record_terminal_outcome_rejects_non_terminal_outcome_classes() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::record_terminal_outcome | Behavior: WillRetry and Suppressed are non-terminal classes and must be rejected
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_record_terminal_outcome_rejects_non_terminal_outcome_classes(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_record_terminal_outcome_clears_next_attempt_at_and_first_suppressed_at() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::record_terminal_outcome | Behavior: terminal outcomes clear retry scheduling and the suppression window start
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_record_terminal_outcome_clears_next_attempt_at_and_first_suppressed_at(db, u1)
                .await;
        }

        #[tokio::test]
        async fn test_retry_state_db_reset_all_retry_states_deletes_all_rows_for_work() {
            // REQ-ID: R-20 | Contract: ProviderRetryStateDb::reset_all_retry_states | Behavior: deletes all retry-state rows for the specified work
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_reset_all_retry_states_deletes_all_rows_for_work(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_works_due_for_retry_returns_due_will_retry_and_suppressed_only() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::list_works_due_for_retry | Behavior: returns only pairs due at or before now whose last outcome is WillRetry or Suppressed
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_list_works_due_for_retry_returns_due_will_retry_and_suppressed_only(db, u1)
                .await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_works_due_for_retry_includes_rows_when_query_now_equals_next_attempt_at() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::list_works_due_for_retry | Behavior: next_attempt_at equality is inclusive, so rows due exactly at now are returned
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_list_works_due_for_retry_includes_rows_when_query_now_equals_next_attempt_at(
                db, u1,
            )
            .await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_retry_states_returns_all_rows_for_work() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::list_retry_states | Behavior: returns every provider retry-state row recorded for the specified work
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_list_retry_states_returns_all_rows_for_work(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_retry_states_isolates_by_user_id() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::list_retry_states | Behavior: returns only rows owned by the requested user_id
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, u2) = h.user_ids();

            assert_list_retry_states_isolates_by_user_id(db, u1, u2).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_works_with_terminal_provider_rows_includes_only_all_non_conflict_terminal_works() {
            // REQ-ID: R-20 | Contract: ProviderRetryStateDb::list_works_with_terminal_provider_rows | Behavior: includes works whose existing provider rows are all non-Conflict terminal outcomes and excludes works with any non-terminal row
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_list_works_with_terminal_provider_rows_includes_only_all_non_conflict_terminal_works(
                db, u1,
            )
            .await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_works_with_terminal_provider_rows_returns_terminal_provider_set_for_full_terminal_work() {
            // REQ-ID: R-20 | Contract: ProviderRetryStateDb::list_works_with_terminal_provider_rows | Behavior: returned provider vec lists the providers whose rows are terminal non-Conflict outcomes for a fully terminal work
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_list_works_with_terminal_provider_rows_returns_terminal_provider_set_for_full_terminal_work(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_works_with_terminal_provider_rows_includes_partial_provider_set_when_all_rows_are_terminal() {
            // REQ-ID: R-20 | Contract: ProviderRetryStateDb::list_works_with_terminal_provider_rows | Behavior: a work with a nonzero partial provider set still qualifies when all existing rows are non-Conflict terminal outcomes
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_list_works_with_terminal_provider_rows_includes_partial_provider_set_when_all_rows_are_terminal(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_works_with_terminal_provider_rows_returns_only_terminal_non_conflict_providers_for_partial_work() {
            // REQ-ID: R-20 | Contract: ProviderRetryStateDb::list_works_with_terminal_provider_rows | Behavior: returned provider vec contains exactly the terminal non-Conflict providers recorded for a partial provider set
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_list_works_with_terminal_provider_rows_returns_only_terminal_non_conflict_providers_for_partial_work(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_works_with_terminal_provider_rows_works_with_terminal_provider_rows_for_single_work_with_three_terminal_non_conflict_rows() {
            // REQ-ID: R-20 | Contract: ProviderRetryStateDb::list_works_with_terminal_provider_rows | Behavior: a single work with Success, NotFound, and PermanentFailure rows appears once with exactly those 3 providers and no Conflict provider in the vec
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_list_works_with_terminal_provider_rows_works_with_terminal_provider_rows_for_single_work_with_three_terminal_non_conflict_rows(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_works_with_terminal_provider_rows_isolated_by_user_id() {
            // REQ-ID: R-20 | Contract: ProviderRetryStateDb::list_works_with_terminal_provider_rows | Behavior: terminal-provider-row listing is isolated by user_id
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, u2) = h.user_ids();

            assert_list_works_with_terminal_provider_rows_isolated_by_user_id(db, u1, u2).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_works_with_terminal_provider_rows_excludes_conflict_rows() {
            // REQ-ID: R-20 | Contract: ProviderRetryStateDb::list_works_with_terminal_provider_rows | Behavior: excludes works with any Conflict provider row and returned provider vecs for included works contain only terminal non-Conflict providers
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_list_works_with_terminal_provider_rows_excludes_conflict_rows(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_works_with_terminal_provider_rows_excludes_work_with_any_conflict_row() {
            // REQ-ID: R-20 | Contract: ProviderRetryStateDb::list_works_with_terminal_provider_rows | Behavior: a work with Success, PermanentFailure, and NotFound rows must still be excluded entirely when any provider row is Conflict
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_list_works_with_terminal_provider_rows_excludes_work_with_any_conflict_row(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_works_with_terminal_provider_rows_excludes_works_with_no_rows() {
            // REQ-ID: R-20 | Contract: ProviderRetryStateDb::list_works_with_terminal_provider_rows | Behavior: excludes works that have no provider retry-state rows
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_list_works_with_terminal_provider_rows_excludes_works_with_no_rows(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_list_works_with_terminal_provider_rows_excludes_in_flight_rows() {
            // REQ-ID: R-20 | Contract: ProviderRetryStateDb::list_works_with_terminal_provider_rows | Behavior: excludes works with any WillRetry or Suppressed provider row and returned provider vecs for included works contain only terminal non-Conflict providers
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_list_works_with_terminal_provider_rows_excludes_in_flight_rows(db, u1).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_isolates_retry_state_by_user_id() {
            // REQ-ID: R-20, R-22 | Contract: ProviderRetryStateDb::{get_retry_state,list_works_due_for_retry} | Behavior: different user_ids see separate retry-state records and queries
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, u2) = h.user_ids();

            assert_retry_state_is_isolated_by_user_id(db, u1, u2).await;
        }

        #[tokio::test]
        async fn test_retry_state_db_not_configured_reset_flow() {
            // Contract: NotConfigured terminal outcome is persisted and reset_not_configured_outcomes
            // deletes only matching rows, leaving other providers untouched.
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert_not_configured_reset_flow(db, u1).await;
        }
    };
}

struct SqliteHarness {
    db: livrarr_db::sqlite::SqliteDb,
    u1: UserId,
    u2: UserId,
}

#[async_trait]
impl DbTestHarness for SqliteHarness {
    type Db = livrarr_db::sqlite::SqliteDb;

    async fn setup() -> Self {
        let db = livrarr_db::create_test_db().await;

        let u1 = db
            .create_user(CreateUserDbRequest {
                username: "retry_state_user_1".to_string(),
                password_hash: "hash1".to_string(),
                role: UserRole::Admin,
                api_key_hash: "apikey1".to_string(),
            })
            .await
            .unwrap()
            .id;

        let u2 = db
            .create_user(CreateUserDbRequest {
                username: "retry_state_user_2".to_string(),
                password_hash: "hash2".to_string(),
                role: UserRole::User,
                api_key_hash: "apikey2".to_string(),
            })
            .await
            .unwrap()
            .id;

        Self { db, u1, u2 }
    }

    fn db(&self) -> &Self::Db {
        &self.db
    }

    fn user_ids(&self) -> (UserId, UserId) {
        (self.u1, self.u2)
    }
}

retry_state_db_tests!(SqliteHarness);
