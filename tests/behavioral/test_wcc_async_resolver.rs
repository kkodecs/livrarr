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
use livrarr_metadata::async_resolver::llm_identity_verify;
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

fn hard_provenance(identity: &CapturedIdentity) -> AnchorProvenance {
    AnchorProvenance {
        ol_key: identity.ol_key.as_ref().map(|_| MatchBasis::Hard),
        gr_key: identity.gr_key.as_ref().map(|_| MatchBasis::Hard),
        hc_key: identity.hc_key.as_ref().map(|_| MatchBasis::Hard),
        isbn_13: identity.isbn_13.as_ref().map(|_| MatchBasis::Hard),
        asin: identity.asin.as_ref().map(|_| MatchBasis::Hard),
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

// NOTE (id-completeness cutover): the four `converge_identity_pending` directive
// tests (AC-023, AC-036, AC-038, AC-017) were removed when that legacy function
// was deleted. Its behavior is now the engine's: hard/fuzzy harvest +
// monotonic badge in `settle_identity` (covered by test_id_completeness +
// test_unified_identity_path) and the dead-end -> NeedsReview termination in
// `converge_work` (covered by test_id_completeness converge_work_terminal).

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
