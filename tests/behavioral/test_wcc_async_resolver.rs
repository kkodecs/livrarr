#![allow(dead_code, unused_imports)]

//! Behavioral tests for work-creation-consistency async resolver directives.

use assert_matches::assert_matches;
use livrarr_behavioral::stubs::create_test_user;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{CreateWorkDbRequest, WorkDb, WorkDbCreate};
use livrarr_domain::identity::*;
use livrarr_domain::services::{
    LlmCallRequest, LlmCallResponse, LlmCaller, LlmError, WorkIdentityError, WorkServiceError,
};
use livrarr_domain::{EnrichmentStatus, UserId, Work, WorkId};
use livrarr_metadata::async_resolver::{converge_identity_pending, llm_identity_verify};
use livrarr_metadata::english_identity_resolver::EnglishIdentityResolver;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

const USER_ID: UserId = 81;
const WORK_ID: WorkId = 91;

struct StubResolver {
    calls: AtomicUsize,
    result: Mutex<Resolution>,
}

impl StubResolver {
    fn new(result: Resolution) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            result: Mutex::new(result),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl EnglishIdentityResolver for StubResolver {
    async fn resolve(
        &self,
        _user_id: UserId,
        _seed: &WorkSeed,
        tier: LatencyTier,
    ) -> Result<Resolution, WorkIdentityError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(tier, LatencyTier::Background);
        Ok(self.result.lock().expect("resolver result").clone())
    }
}

#[derive(Clone, Default)]
struct CountingLlm {
    calls: std::sync::Arc<AtomicUsize>,
    configured: bool,
}

impl CountingLlm {
    fn not_configured() -> Self {
        Self {
            calls: std::sync::Arc::new(AtomicUsize::new(0)),
            configured: false,
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl LlmCaller for CountingLlm {
    async fn call(&self, _req: LlmCallRequest) -> Result<LlmCallResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if !self.configured {
            return Err(LlmError::NotConfigured);
        }

        Ok(LlmCallResponse {
            content: "{\"identity_valid\":true}".to_string(),
            model_used: "wcc-test".to_string(),
            elapsed: Duration::from_millis(1),
        })
    }
}

fn captured() -> CapturedIdentity {
    CapturedIdentity {
        ol_key: Some("OL45883W".to_string()),
        gr_key: Some("234225".to_string()),
        hc_key: Some("HC-DUNE".to_string()),
        isbn_13: Some("9780441013593".to_string()),
        asin: None,
        title: "Dune".to_string(),
        author_name: "Frank Herbert".to_string(),
        language: Some("en".to_string()),
    }
}

fn pending_work() -> Work {
    Work {
        identity_status: Default::default(),
        id: WORK_ID,
        user_id: USER_ID,
        title: "Dune".to_string(),
        author_name: "Frank Herbert".to_string(),
        ol_key: Some("OL45883W".to_string()),
        enrichment_status: EnrichmentStatus::Unenriched,
        ..Work::default()
    }
}

/// REQ-IDs: REQ-022, REQ-023
/// AC-IDs: AC-023
/// Directive: background convergence resolves a monitor-seeded work toward the full federated anchor set.
#[tokio::test]
async fn test_wcc_async_resolver_ac_023_converges_monitor_seeded_work_to_full_anchor_set() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (seeded, created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Dune".to_string(),
            author_name: "Frank Herbert".to_string(),
            normalized_title: "dune".to_string(),
            normalized_author: "frank herbert".to_string(),
            author_id: None,
            ol_key: Some("OL45883W".to_string()),
            gr_key: None,
            year: Some(1965),
            cover_url: None,
            language: Some("en".to_string()),
            import_id: None,
            series_id: None,
            isbn_13: None,
            asin: None,
            description: None,
            series_name: None,
            series_position: None,
            monitor_ebook: true,
            monitor_audiobook: false,
            source_provider_json: None,
            cover_manual: false,
        })
        .await
        .expect("seed monitor-created work");
    assert!(created);

    let resolver = StubResolver::new(Resolution::Resolved {
        identity: captured(),
        method: IdentityMethod::IsbnDirect,
        candidate_id: CandidateId("candidate-bg".to_string()),
    });
    let pending = Work {
        identity_status: Default::default(),
        enrichment_status: EnrichmentStatus::Unenriched,
        ..seeded
    };

    let result = converge_identity_pending(&resolver, &db, user_id, &pending).await;
    let converged = db
        .get_work(user_id, pending.id)
        .await
        .expect("fetch work after convergence");

    assert!(result.is_ok());
    assert_eq!(resolver.call_count(), 1);
    assert_eq!(converged.ol_key.as_deref(), Some("OL45883W"));
    assert_eq!(
        converged.gr_key.as_deref(),
        Some("234225"),
        "AC-023/REQ-022: background convergence must persist the missing GR anchor"
    );
    assert_eq!(
        converged.hc_key.as_deref(),
        Some("HC-DUNE"),
        "AC-023/REQ-022: background convergence must persist the missing HC anchor"
    );
    assert_eq!(
        converged.isbn_13.as_deref(),
        Some("9780441013593"),
        "AC-023/REQ-022: background convergence must persist the ISBN bridge"
    );
}

/// REQ-IDs: REQ-026
/// AC-IDs: AC-036
/// Directive: a non-interactive Tier-B item that cannot deterministically match transitions to NeedsReview.
#[tokio::test]
async fn test_wcc_async_resolver_ac_036_tier_b_dead_end_becomes_needs_review_not_infinite_pending()
{
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;
    let (seeded, _created) = db
        .create_work(CreateWorkDbRequest {
            user_id,
            title: "Unresolvable Title".to_string(),
            author_name: "Unknown Author".to_string(),
            normalized_title: "unresolvable title".to_string(),
            normalized_author: "unknown author".to_string(),
            author_id: None,
            ol_key: None,
            gr_key: None,
            year: None,
            cover_url: None,
            language: Some("en".to_string()),
            import_id: None,
            series_id: None,
            isbn_13: None,
            asin: None,
            description: None,
            series_name: None,
            series_position: None,
            monitor_ebook: true,
            monitor_audiobook: false,
            source_provider_json: None,
            cover_manual: false,
        })
        .await
        .expect("seed tier-B identity-pending work");

    let unresolved = Resolution::Unresolved {
        captured: CapturedIdentity {
            ol_key: None,
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: "Unresolvable Title".to_string(),
            author_name: "Unknown Author".to_string(),
            language: Some("en".to_string()),
        },
        reason: PendingReason::NoCandidates,
        candidate_id: None,
    };
    let resolver = StubResolver::new(unresolved);
    let pending = Work {
        identity_status: Default::default(),
        enrichment_status: EnrichmentStatus::Unenriched,
        ..seeded
    };

    let result = converge_identity_pending(&resolver, &db, user_id, &pending).await;
    let converged = db
        .get_work(user_id, pending.id)
        .await
        .expect("fetch work after convergence");

    assert!(result.is_ok());
    assert_eq!(resolver.call_count(), 1);
    assert_eq!(
        converged.identity_status,
        livrarr_domain::IdentityStatus::NeedsReview,
        "AC-036/REQ-026: an unresolvable Tier-B item must transition to NeedsReview on the identity track, not loop"
    );
}

/// REQ-IDs: REQ-025
/// AC-IDs: AC-038
/// Directive: partial failure does not clear existing anchors; user-resolved conflicts are not silently changed.
#[tokio::test]
async fn test_wcc_async_resolver_ac_038_partial_failure_no_clobber_after_user_resolution() {
    let db = create_test_db().await;
    let unresolved = Resolution::Unresolved {
        captured: CapturedIdentity {
            ol_key: Some("OL45883W".to_string()),
            gr_key: None,
            hc_key: None,
            isbn_13: Some("9780441013593".to_string()),
            asin: None,
            title: "Dune".to_string(),
            author_name: "Frank Herbert".to_string(),
            language: Some("en".to_string()),
        },
        reason: PendingReason::OlUnavailable,
        candidate_id: Some(CandidateId("candidate-partial".to_string())),
    };
    let resolver = StubResolver::new(unresolved);
    let mut work = pending_work();
    work.gr_key = Some("234225".to_string());
    work.identity_status = livrarr_domain::IdentityStatus::Conflict;

    let result = converge_identity_pending(&resolver, &db, USER_ID, &work).await;

    assert!(result.is_ok());
    assert_eq!(
        resolver.call_count(),
        0,
        "AC-038: user-resolved or conflicted works must not be re-litigated by background convergence"
    );
}

/// REQ-IDs: REQ-017
/// AC-IDs: D-013
/// Directive: with no LLM configured, identity verification is not invoked and deterministic flow remains available.
#[tokio::test]
async fn test_wcc_async_resolver_d_013_llm_identity_verify_not_called_without_llm_configuration() {
    let llm = CountingLlm::not_configured();
    let conflict = llm_identity_verify(&llm, USER_ID, &pending_work(), &captured()).await;

    assert!(conflict.is_none());
    assert_eq!(
        llm.call_count(),
        0,
        "D-013/REQ-017: absence of an LLM must not trigger an LLM call"
    );
}

/// REQ-IDs: REQ-022
/// AC-IDs: AC-017
/// Directive: the SAME book seeded through two DIFFERENT creation paths (one
/// carrying only an `ol_key`, the other only a `gr_key`) converges to an
/// IDENTICAL federated anchor set — the six creation paths differ only in seed,
/// never in resolved identity (the cross-path convergence-equality of REQ-022).
#[tokio::test]
async fn test_wcc_async_resolver_ac_017_two_paths_same_book_converge_identical() {
    let db = create_test_db().await;
    let user_id = create_test_user(&db).await;

    let seed_work = |ol: Option<&str>, gr: Option<&str>| CreateWorkDbRequest {
        user_id,
        title: "Dune".to_string(),
        author_name: "Frank Herbert".to_string(),
        normalized_title: "dune".to_string(),
        normalized_author: "frank herbert".to_string(),
        author_id: None,
        ol_key: ol.map(str::to_string),
        gr_key: gr.map(str::to_string),
        year: Some(1965),
        cover_url: None,
        language: Some("en".to_string()),
        import_id: None,
        series_id: None,
        isbn_13: None,
        asin: None,
        description: None,
        series_name: None,
        series_position: None,
        monitor_ebook: true,
        monitor_audiobook: false,
        source_provider_json: None,
        cover_manual: false,
    };

    // Path 1 — author-monitor: seeds only the ol_key.
    let (work_a, _) = db
        .create_work(seed_work(Some("OL45883W"), None))
        .await
        .expect("seed author-monitor work (ol_key only)");
    // Path 2 — series-monitor: seeds only the gr_key, same book.
    let (work_b, _) = db
        .create_work(seed_work(None, Some("234225")))
        .await
        .expect("seed series-monitor work (gr_key only)");

    // Both converge against the same fully-resolved identity (the resolver is
    // seed-independent — REQ-022): the differently-seeded paths must end identical.
    let resolution = Resolution::Resolved {
        identity: captured(),
        method: IdentityMethod::IsbnDirect,
        candidate_id: CandidateId("candidate-conv".to_string()),
    };
    for work in [&work_a, &work_b] {
        let pending = Work {
            identity_status: Default::default(),
            enrichment_status: EnrichmentStatus::Unenriched,
            ..work.clone()
        };
        converge_identity_pending(
            &StubResolver::new(resolution.clone()),
            &db,
            user_id,
            &pending,
        )
        .await
        .expect("convergence");
    }

    let a = db.get_work(user_id, work_a.id).await.expect("fetch A");
    let b = db.get_work(user_id, work_b.id).await.expect("fetch B");

    let anchors = |w: &Work| {
        (
            w.ol_key.clone(),
            w.gr_key.clone(),
            w.hc_key.clone(),
            w.isbn_13.clone(),
        )
    };
    assert_eq!(
        anchors(&a),
        anchors(&b),
        "AC-017/REQ-022: the same book seeded via different paths must converge to identical anchors"
    );
    assert_eq!(
        anchors(&a),
        (
            Some("OL45883W".to_string()),
            Some("234225".to_string()),
            Some("HC-DUNE".to_string()),
            Some("9780441013593".to_string()),
        ),
        "AC-017: both paths converge to the full federated anchor set"
    );
}
