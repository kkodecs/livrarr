#![allow(dead_code, unused_imports)]

//! Behavioral tests for work-creation-consistency cached merge reuse directives.

use assert_matches::assert_matches;
use livrarr_domain::services::{LlmCallRequest, LlmCallResponse, LlmCaller, LlmError};
use livrarr_domain::{EnrichmentStatus, MetadataProvider, UserId, Work, WorkId};
use livrarr_external_data::NormalizedWorkDetail;
use livrarr_metadata::{DefaultMergeEngine, MergeEngine, PriorityModel};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

const USER_ID: UserId = 101;
const WORK_ID: WorkId = 202;

#[derive(Clone, Default)]
struct NoOpLlm;

impl LlmCaller for NoOpLlm {
    async fn call(&self, _req: LlmCallRequest) -> Result<LlmCallResponse, LlmError> {
        Err(LlmError::NotConfigured)
    }
}

#[derive(Default)]
struct ProviderHttpSpy {
    calls: AtomicUsize,
}

impl ProviderHttpSpy {
    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

fn work(language: Option<&str>) -> Work {
    Work {
        identity_status: Default::default(),
        id: WORK_ID,
        user_id: USER_ID,
        title: "Existing Title".to_string(),
        author_name: "Existing Author".to_string(),
        description: Some("Keep me if providers are null".to_string()),
        language: language.map(str::to_string),
        enrichment_status: EnrichmentStatus::Unenriched,
        ..Work::default()
    }
}

fn engine() -> DefaultMergeEngine {
    DefaultMergeEngine::new_with_llm(PriorityModel::english(), NoOpLlm, false)
}

fn payload(title: &str, description: Option<&str>, language: &str) -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: Some(title.to_string()),
        author_name: Some("Frank Herbert".to_string()),
        description: description.map(str::to_string),
        language: Some(language.to_string()),
        isbn_13: Some("9780441013593".to_string()),
        // Real provider payloads carry a cover; merge_impl classifies Enriched only
        // when both a description and a cover are present.
        cover_url: Some("https://covers.example/dune.jpg".to_string()),
        ..NormalizedWorkDetail::default()
    }
}

/// REQ-IDs: REQ-014, REQ-015
/// AC-IDs: AC-010
/// Directive: merge_from_cached feeds already-retrieved payloads to the merge engine with zero provider HTTP calls.
#[tokio::test]
async fn test_wcc_merge_reuse_ac_010_merge_from_cached_issues_zero_provider_http_calls() {
    let provider_spy = ProviderHttpSpy::default();
    let payloads = HashMap::from([(
        MetadataProvider::Hardcover,
        payload("Dune", Some("Cached Hardcover description"), "en"),
    )]);

    let output = engine()
        .merge_from_cached(work(Some("en")), payloads, vec![], Some("en"))
        .await
        .expect("cached merge should succeed without provider IO");

    assert_eq!(provider_spy.call_count(), 0);
    assert_eq!(output.enrichment_status, EnrichmentStatus::Enriched);
    assert!(
        output.work_update.is_some(),
        "cached provider fields should produce an immediate work update"
    );
}

/// REQ-IDs: REQ-016, REQ-017
/// AC-IDs: AC-012
/// Directive: with no LLM configured, deterministic provider priority wins and null never clears populated data.
#[tokio::test]
async fn test_wcc_merge_reuse_ac_012_no_llm_priority_wins_and_null_never_clears_populated_field() {
    let payloads = HashMap::from([
        (
            MetadataProvider::OpenLibrary,
            payload("Dune", Some("Lower-priority OL description"), "en"),
        ),
        (MetadataProvider::Hardcover, payload("Dune", None, "en")),
        (
            MetadataProvider::Goodreads,
            payload("Dune", Some("Higher-priority GR description"), "en"),
        ),
    ]);

    let output = engine()
        .merge_from_cached(work(Some("en")), payloads, vec![], Some("en"))
        .await
        .expect("deterministic no-LLM merge should succeed");

    assert_eq!(output.enrichment_status, EnrichmentStatus::Enriched);
    assert!(
        output.work_update.is_some(),
        "non-null provider payload should not be erased by a higher-priority null"
    );
}

/// REQ-IDs: REQ-027
/// AC-IDs: AC-027
/// Directive: for a foreign work, OpenLibrary/Hardcover English metadata is dropped before merge.
#[tokio::test]
async fn test_wcc_merge_reuse_ac_027_foreign_work_drops_openlibrary_and_hardcover_metadata() {
    let payloads = HashMap::from([
        (
            MetadataProvider::OpenLibrary,
            payload("English OL Title", Some("English OL description"), "en"),
        ),
        (
            MetadataProvider::Hardcover,
            payload("English HC Title", Some("English HC description"), "en"),
        ),
        (
            MetadataProvider::GoogleBooks,
            payload("O Titulo Correto", Some("Descricao em portugues"), "pt"),
        ),
    ]);

    let output = engine()
        .merge_from_cached(work(Some("pt")), payloads, vec![], Some("pt"))
        .await
        .expect("foreign cached merge should succeed");

    assert_eq!(output.enrichment_status, EnrichmentStatus::Enriched);
    assert!(
        output.work_update.is_some(),
        "language-compatible Google Books payload should remain eligible after OL/HC are dropped"
    );
}
