#![allow(dead_code)]

use livrarr_behavioral::stubs::create_test_user;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::{
    CandidateId, CapturedIdentity, ConflictSource, IdentityConflictKind, IdentityMethod,
    IncomingConflictPayload, LatencyTier, NewIdentityConflict, PendingReason, Resolution, WorkSeed,
};
use livrarr_domain::services::{
    CallOperation, CallOutcomeClass, ProviderCallRecord, ProviderCallSink, WorkIdentityError,
};
use livrarr_domain::{IdentityStatus, UserId, Work};
use livrarr_metadata::async_resolver::complete_anchors;
use livrarr_metadata::english_identity_resolver::EnglishIdentityResolver;

struct StubResolver {
    calls: std::sync::atomic::AtomicUsize,
    result: std::sync::Mutex<Resolution>,
}

impl StubResolver {
    fn new(result: Resolution) -> Self {
        Self {
            calls: std::sync::atomic::AtomicUsize::new(0),
            result: std::sync::Mutex::new(result),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl EnglishIdentityResolver for StubResolver {
    async fn resolve(
        &self,
        _user_id: UserId,
        _seed: &WorkSeed,
        tier: LatencyTier,
    ) -> Result<Resolution, WorkIdentityError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(tier, LatencyTier::Background);
        Ok(self.result.lock().expect("resolver result").clone())
    }
}

#[derive(Default)]
struct RecordingSink {
    records: std::sync::Mutex<Vec<ProviderCallRecord>>,
}

impl RecordingSink {
    fn count(&self) -> usize {
        self.records.lock().expect("recording sink lock").len()
    }
}

impl ProviderCallSink for RecordingSink {
    fn record(&self, rec: ProviderCallRecord) {
        self.records.lock().expect("recording sink lock").push(rec);
    }
}

fn captured_with_gr() -> CapturedIdentity {
    CapturedIdentity {
        ol_key: Some("OL45883W".to_string()),
        gr_key: Some("234225".to_string()),
        hc_key: Some("HC-DUNE".to_string()),
        isbn_13: Some("9780441013593".to_string()),
        asin: Some("B000TEST12".to_string()),
        title: "Dune".to_string(),
        author_name: "Frank Herbert".to_string(),
        language: Some("en".to_string()),
    }
}

fn unresolved() -> Resolution {
    Resolution::Unresolved {
        captured: CapturedIdentity {
            gr_key: None,
            ..captured_with_gr()
        },
        reason: PendingReason::NoCandidates,
        candidate_id: None,
    }
}

async fn create_work_with_anchors(
    gr_key: Option<&str>,
    hc_key: Option<&str>,
    ol_key: Option<&str>,
    isbn_13: Option<&str>,
    asin: Option<&str>,
    status: IdentityStatus,
) -> (livrarr_db::sqlite::SqliteDb, i64, Work) {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (mut work, _) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Dune".to_string(),
            author_name: "Frank Herbert".to_string(),
            normalized_title: "dune".to_string(),
            normalized_author: "frank herbert".to_string(),
            ol_key: ol_key.map(str::to_string),
            gr_key: gr_key.map(str::to_string),
            isbn_13: isbn_13.map(str::to_string),
            asin: asin.map(str::to_string),
            language: Some("en".to_string()),
            monitor_ebook: true,
            ..Default::default()
        })
        .await
        .unwrap();
    db.set_identity_status(user_id, work.id, status)
        .await
        .unwrap();
    work.identity_status = status;
    work.hc_key = hc_key.map(str::to_string);
    (db, user_id, work)
}

#[tokio::test]
async fn test_mc_complete_anchors_resolved_gr_key_is_monotonic_and_reported() {
    // REQ-008 / AC-010
    let (db, user_id, work) = create_work_with_anchors(
        None,
        Some("HC-DUNE-OLD"),
        Some("OL45883W"),
        Some("9780441013593"),
        None,
        IdentityStatus::Confirmed,
    )
    .await;
    let resolver = StubResolver::new(Resolution::Resolved {
        identity: captured_with_gr(),
        method: IdentityMethod::IsbnDirect,
        candidate_id: CandidateId("candidate-complete".to_string()),
    });
    let sink: std::sync::Arc<dyn ProviderCallSink> = std::sync::Arc::new(RecordingSink::default());

    let report = complete_anchors(&resolver, &db, user_id, &work, &[], &sink)
        .await
        .unwrap();
    let after = db.get_work(user_id, work.id).await.unwrap();

    assert_eq!(resolver.call_count(), 1);
    assert!(report
        .resolved
        .iter()
        .any(|(kind, value)| kind == "gr_key" && value == "234225"));
    assert_eq!(after.gr_key.as_deref(), Some("234225"));
    assert_eq!(after.ol_key, work.ol_key);
    assert_eq!(after.isbn_13, work.isbn_13);
}

#[tokio::test]
async fn test_mc_complete_anchors_unconfirmable_leaves_anchor_absent_and_reports_not_found() {
    // REQ-008 / AC-010
    let (db, user_id, work) = create_work_with_anchors(
        None,
        None,
        Some("OL45883W"),
        Some("9780441013593"),
        None,
        IdentityStatus::Confirmed,
    )
    .await;
    let resolver = StubResolver::new(unresolved());
    let sink: std::sync::Arc<dyn ProviderCallSink> = std::sync::Arc::new(RecordingSink::default());

    let report = complete_anchors(&resolver, &db, user_id, &work, &[], &sink)
        .await
        .unwrap();
    let after = db.get_work(user_id, work.id).await.unwrap();

    assert_eq!(after.gr_key, None);
    assert!(report
        .skipped
        .iter()
        .any(|(provider, reason)| provider == "goodreads" && reason == "not_found"));
}

#[tokio::test]
async fn test_mc_complete_anchors_suppressed_missing_providers_make_zero_resolver_calls() {
    // REQ-008 / AC-010
    let (db, user_id, work) = create_work_with_anchors(
        None,
        None,
        Some("OL45883W"),
        Some("9780441013593"),
        None,
        IdentityStatus::Confirmed,
    )
    .await;
    let resolver = StubResolver::new(unresolved());
    let sink_impl = std::sync::Arc::new(RecordingSink::default());
    let sink: std::sync::Arc<dyn ProviderCallSink> = sink_impl.clone();

    let report = complete_anchors(
        &resolver,
        &db,
        user_id,
        &work,
        &[
            "goodreads".to_string(),
            "hardcover".to_string(),
            "audnexus".to_string(),
        ],
        &sink,
    )
    .await
    .unwrap();

    assert_eq!(resolver.call_count(), 0);
    assert_eq!(sink_impl.count(), 0);
    assert!(!report.skipped.is_empty());
    assert!(report
        .skipped
        .iter()
        .all(|(_, reason)| reason == "suppressed"));
}

#[tokio::test]
async fn test_mc_complete_anchors_complete_or_conflict_work_returns_empty_report_without_calls() {
    // REQ-008 / AC-010
    let (db, user_id, complete_work) = create_work_with_anchors(
        Some("234225"),
        Some("HC-DUNE"),
        Some("OL45883W"),
        Some("9780441013593"),
        Some("B000TEST12"),
        IdentityStatus::Confirmed,
    )
    .await;
    let resolver = StubResolver::new(unresolved());
    let sink: std::sync::Arc<dyn ProviderCallSink> = std::sync::Arc::new(RecordingSink::default());

    let complete_report = complete_anchors(&resolver, &db, user_id, &complete_work, &[], &sink)
        .await
        .unwrap();

    let conflict_work = Work {
        identity_status: IdentityStatus::Conflict,
        gr_key: None,
        ..complete_work
    };
    let conflict_report = complete_anchors(&resolver, &db, user_id, &conflict_work, &[], &sink)
        .await
        .unwrap();

    assert_eq!(resolver.call_count(), 0);
    assert!(complete_report.resolved.is_empty());
    assert!(complete_report.skipped.is_empty());
    assert!(conflict_report.resolved.is_empty());
    assert!(conflict_report.skipped.is_empty());
    let _ = (CallOperation::Identity, CallOutcomeClass::SkippedPolicy);
}

fn quorum_tie() -> Resolution {
    Resolution::Conflict {
        conflict: NewIdentityConflict {
            user_id: 0,
            existing_work_id: 0,
            kind: IdentityConflictKind::QuorumTie,
            incoming: IncomingConflictPayload {
                ol_key: None,
                gr_key: None,
                hc_key: Some("HC-DUNE".to_string()),
                isbn_13: None,
                asin: None,
                title: "Dune".to_string(),
                author_name: "Frank Herbert".to_string(),
                year: None,
                cover_url: None,
                top_candidates: Vec::new(),
            },
            raised_by: ConflictSource::ManualAdd,
            raised_source_path: None,
        },
        captured: CapturedIdentity {
            gr_key: None,
            ..captured_with_gr()
        },
        tied: Vec::new(),
    }
}

/// #148 / PO decision 2026-06-11: a quorum tie (or needs-confirmation) is the
/// arbitration's failure, not the provider lacking the work — completion
/// skips with reason "ambiguous" (no terminal parking; the not-found reason
/// is reserved for a genuinely empty-handed resolution) and writes nothing.
#[tokio::test]
async fn test_mc_complete_anchors_ambiguous_resolution_skips_without_parking_reason() {
    let (db, user_id, work) = create_work_with_anchors(
        None,
        None,
        Some("OL45883W"),
        Some("9780441013593"),
        None,
        IdentityStatus::Confirmed,
    )
    .await;
    let resolver = StubResolver::new(quorum_tie());
    let sink: std::sync::Arc<dyn ProviderCallSink> = std::sync::Arc::new(RecordingSink::default());

    let report = complete_anchors(&resolver, &db, user_id, &work, &[], &sink)
        .await
        .unwrap();
    let after = db.get_work(user_id, work.id).await.unwrap();

    assert_eq!(resolver.call_count(), 1);
    assert!(report.resolved.is_empty());
    assert_eq!(after.gr_key, None);
    assert!(report
        .skipped
        .iter()
        .any(|(provider, reason)| provider == "goodreads" && reason == "ambiguous"));
    assert!(!report
        .skipped
        .iter()
        .any(|(_, reason)| reason == "not_found"));
}
