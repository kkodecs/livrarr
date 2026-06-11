#![allow(dead_code)]

use chrono::{Duration, Utc};
use livrarr_behavioral::stubs::create_test_user;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateWorkDbRequest, FieldDissentDb, ProviderCallRecordDb, RetentionPolicy, WorkDbCreate,
};
use livrarr_domain::services::{
    CallOperation, CallOutcomeClass, NoopCallSink, ProviderCallRecord, ProviderCallSink,
};
use livrarr_domain::{DissentReason, FieldDissent};

#[derive(Default)]
struct RecordingSink {
    records: std::sync::Mutex<Vec<ProviderCallRecord>>,
}

impl RecordingSink {
    fn records(&self) -> Vec<ProviderCallRecord> {
        self.records.lock().expect("recording sink lock").clone()
    }
}

impl ProviderCallSink for RecordingSink {
    fn record(&self, rec: ProviderCallRecord) {
        self.records.lock().expect("recording sink lock").push(rec);
    }
}

fn call_record(
    provider: &str,
    operation: CallOperation,
    outcome: CallOutcomeClass,
    started_at: chrono::DateTime<Utc>,
    duration_ms: i64,
    detail: Option<&str>,
) -> ProviderCallRecord {
    ProviderCallRecord {
        provider: provider.to_string(),
        operation,
        work_id: Some(101),
        started_at,
        duration_ms,
        outcome,
        detail: detail.map(str::to_string),
    }
}

fn dissent(work_id: i64, provider: &str, field: &str, generation: i64) -> FieldDissent {
    FieldDissent {
        work_id,
        provider: provider.to_string(),
        field: field.to_string(),
        offered_value: format!("{provider}-{field}-offered"),
        winning_value: Some("winner".to_string()),
        reason: DissentReason::FieldConflict,
        merge_generation: generation,
        recorded_at: Utc::now(),
    }
}

#[tokio::test]
async fn test_mc_provider_call_records_persist_and_aggregate_network_latency_only() {
    // REQ-001 / AC-001; REQ-002 / AC-003
    let db = create_test_db().await;
    let now = Utc::now();
    let records = vec![
        call_record(
            "goodreads",
            CallOperation::Lookup,
            CallOutcomeClass::Success,
            now - Duration::minutes(10),
            100,
            None,
        ),
        call_record(
            "goodreads",
            CallOperation::Identity,
            CallOutcomeClass::NotFound,
            now - Duration::minutes(9),
            300,
            None,
        ),
        call_record(
            "goodreads",
            CallOperation::Enrich,
            CallOutcomeClass::SkippedNoAnchor,
            now - Duration::minutes(8),
            1,
            None,
        ),
        call_record(
            "goodreads",
            CallOperation::Cover,
            CallOutcomeClass::Cached,
            now - Duration::minutes(7),
            1,
            None,
        ),
        call_record(
            "google_books",
            CallOperation::Enrich,
            CallOutcomeClass::Error,
            now - Duration::minutes(6),
            500,
            Some("http_500"),
        ),
        call_record(
            "google_books",
            CallOperation::Enrich,
            CallOutcomeClass::Success,
            now - Duration::minutes(5),
            200,
            None,
        ),
    ];

    db.record_provider_calls(records).await.unwrap();
    let stats = db.query_provider_stats_24h().await.unwrap();

    let gr = stats
        .iter()
        .find(|row| row.provider == "goodreads")
        .expect("goodreads stats row");
    assert_eq!(gr.calls_24h, 4);
    assert_eq!(gr.median_latency_ms, 200);
    assert_eq!(gr.last_success, Some(now - Duration::minutes(10)));

    let gb = stats
        .iter()
        .find(|row| row.provider == "google_books")
        .expect("google_books stats row");
    assert_eq!(gb.calls_24h, 2);
    assert_eq!(gb.success_rate, 0.5);
    assert_eq!(
        gb.last_error.as_ref().map(|(detail, _)| detail.as_str()),
        Some("http_500")
    );
}

#[tokio::test]
async fn test_mc_provider_call_eviction_keeps_retention_bounds_oldest_first() {
    // REQ-001 / AC-002
    let db = create_test_db().await;
    let now = Utc::now();
    let records = (0..6)
        .map(|idx| {
            call_record(
                "hardcover",
                CallOperation::Enrich,
                CallOutcomeClass::Success,
                now - Duration::days(idx),
                100 + idx,
                None,
            )
        })
        .collect();

    db.record_provider_calls(records).await.unwrap();
    let deleted = db
        .evict_call_records(RetentionPolicy {
            max_age_days: 3,
            max_records: 2,
        })
        .await
        .unwrap();

    assert_eq!(deleted, 4);
    let stats = db.query_provider_stats_24h().await.unwrap();
    let hc = stats
        .iter()
        .find(|row| row.provider == "hardcover")
        .expect("hardcover stats row");
    assert!(hc.calls_24h <= 2);
}

#[tokio::test]
async fn test_mc_field_dissents_query_newest_generation_only() {
    // REQ-014 / AC-016
    let db = create_test_db().await;
    // work_field_dissents carries FK references to users/works (migration 060,
    // the 029 schema pattern) — seed the referenced rows.
    let user_id = create_test_user(&db).await;
    let (work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Dissent Fixture".to_string(),
            author_name: "Fixture Author".to_string(),
            normalized_title: "dissent fixture".to_string(),
            normalized_author: "fixture author".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();
    let work_id = work.id;

    db.record_field_dissents(
        user_id,
        work_id,
        vec![
            dissent(work_id, "goodreads", "description", 1),
            dissent(work_id, "google_books", "series_name", 1),
        ],
    )
    .await
    .unwrap();
    db.record_field_dissents(
        user_id,
        work_id,
        vec![dissent(work_id, "hardcover", "description", 2)],
    )
    .await
    .unwrap();

    let rows = db.list_field_dissents(user_id, work_id).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].provider, "hardcover");
    assert_eq!(rows[0].merge_generation, 2);
}

#[tokio::test]
async fn test_mc_call_sink_contract_and_no_metadata_source_column() {
    // REQ-001 / AC-001; REQ-018 / AC-020
    let db = create_test_db().await;
    let sink = RecordingSink::default();
    let rec = call_record(
        "openlibrary",
        CallOperation::Enrich,
        CallOutcomeClass::SkippedPolicy,
        Utc::now(),
        0,
        Some("foreign_policy"),
    );
    sink.record(rec.clone());
    NoopCallSink.record(rec.clone());

    assert_eq!(sink.records(), vec![rec.clone()]);
    db.record_provider_calls(vec![rec]).await.unwrap();

    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('works')")
        .fetch_all(db.pool())
        .await
        .unwrap();
    assert!(
        !columns.iter().any(|name| name == "metadata_source"),
        "AC-020: migration 061 must remove works.metadata_source"
    );
}
