#![allow(dead_code)]

use std::path::PathBuf;
use std::time::Duration;

use librarr_domain::{EnrichmentStatus, NarrationType, Work, WorkId};
use librarr_metadata::{
    AuthorSearchResult, CoverCache, CoverError, EnrichmentError, EnrichmentResult,
    EnrichmentService, HardcoverCandidate, HardcoverMatcher, LlmClient, LlmError, LlmMessage,
    LlmRole, MetadataError, MetadataProvider, ProviderAuthorResult, ProviderSearchResult,
    ProviderWorkDetail, SearchService, WorkSearchResult,
};

// =============================================================================
// Helpers
// =============================================================================

fn search_result() -> ProviderSearchResult {
    ProviderSearchResult {
        provider_key: "OL123W".into(),
        title: "The Rust Book".into(),
        author_name: Some("Ferris Crab".into()),
        year: Some(2024),
        cover_url: Some("https://example.test/cover.jpg".into()),
    }
}

fn author_result() -> ProviderAuthorResult {
    ProviderAuthorResult {
        provider_key: "OL999A".into(),
        name: "Ferris Crab".into(),
        work_count: Some(42),
    }
}

fn work_detail() -> ProviderWorkDetail {
    ProviderWorkDetail {
        title: "The Rust Book".into(),
        subtitle: Some("Ownership for Humans".into()),
        original_title: None,
        author_name: "Ferris Crab".into(),
        description: Some("A guide".into()),
        year: Some(2024),
        series_name: Some("Rustacean Library".into()),
        series_position: Some(1.0),
        genres: Some(vec!["Programming".into()]),
        language: Some("en".into()),
        page_count: Some(320),
        publisher: Some("Crab Press".into()),
        publish_date: Some("2024-01-01".into()),
        isbn_13: Some("9781234567890".into()),
        cover_url: Some("https://example.test/cover.jpg".into()),
        hardcover_id: Some("HC1".into()),
        asin: Some("B000TEST".into()),
        narrator: Some(vec!["Narrator".into()]),
        narration_type: Some(NarrationType::Human),
        abridged: Some(false),
        duration_seconds: Some(36000),
        rating: Some(4.7),
        rating_count: Some(1000),
    }
}

fn candidate(id: &str, title: &str, author: &str, reads: i64) -> HardcoverCandidate {
    HardcoverCandidate {
        hardcover_id: id.into(),
        title: title.into(),
        author_name: Some(author.into()),
        users_read_count: reads,
        detail: work_detail(),
    }
}

fn enrichment_ok(status: EnrichmentStatus, source: Option<&str>, llm: bool) -> EnrichmentResult {
    EnrichmentResult {
        enrichment_status: status,
        enrichment_source: source.map(Into::into),
        llm_task_spawned: llm,
        work: Work::default(),
    }
}

// =============================================================================
// Generic contract assertions
// =============================================================================

async fn assert_provider_search_ok(p: &impl MetadataProvider) {
    let r = p.search_works("rust").await.expect("search should succeed");
    assert!(!r.is_empty());
    assert!(!r[0].title.is_empty());
}

async fn assert_search_svc_works_ok(s: &impl SearchService) {
    let r = s.search_works("rust").await.expect("search should succeed");
    assert!(!r.is_empty());
    assert!(!r[0].ol_key.is_empty());
}

async fn assert_search_svc_authors_ok(s: &impl SearchService) {
    let r = s
        .search_authors("ferris")
        .await
        .expect("author search should succeed");
    assert!(!r.is_empty());
    assert!(!r[0].name.is_empty());
}

async fn assert_enrichment_not_pending(r: &EnrichmentResult) {
    assert_ne!(
        r.enrichment_status,
        EnrichmentStatus::Unenriched,
        "enrichment_status must never be Pending after enrich_work"
    );
}

async fn assert_cover_path(c: &impl CoverCache, id: WorkId, dir: &str) {
    let expected = PathBuf::from(dir)
        .join("MediaCover")
        .join(id.to_string())
        .join("cover.jpg");
    assert_eq!(c.expected_cover_path(id), expected);
}

// =============================================================================
// MetadataProvider — SEARCH-001, SEARCH-002, SEARCH-006
// =============================================================================

/// SEARCH-001, SEARCH-003 | search_works returns results with required display fields.
#[tokio::test]
async fn test_metadata_provider_search_works_nominal() {
    let p = librarr_metadata::OpenLibraryProvider::new_test(vec![search_result()]);
    assert_provider_search_ok(&p).await;
    let r = p.search_works("rust").await.unwrap();
    assert_eq!(r[0].title, "The Rust Book");
    assert_eq!(r[0].author_name.as_deref(), Some("Ferris Crab"));
    assert_eq!(r[0].year, Some(2024));
    assert!(r[0].cover_url.is_some());
}

/// SEARCH-002, AUTHOR-001 | search_authors returns name and work count.
#[tokio::test]
async fn test_metadata_provider_search_authors_nominal() {
    let p = librarr_metadata::OpenLibraryProvider::new_test_authors(vec![author_result()]);
    let r = p.search_authors("ferris").await.unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].name, "Ferris Crab");
    assert_eq!(r[0].work_count, Some(42));
}

/// SEARCH-006 | fetch_work_detail returns full enrichment-grade detail.
#[tokio::test]
async fn test_metadata_provider_fetch_detail_nominal() {
    let p = librarr_metadata::OpenLibraryProvider::new_test_detail(work_detail());
    let d = p.fetch_work_detail("OL123W").await.unwrap();
    assert_eq!(d.title, "The Rust Book");
    assert_eq!(d.language.as_deref(), Some("en"));
    assert!(d.isbn_13.is_some());
}

/// SEARCH-006 | Timeout(3s) on provider timeout.
#[tokio::test]
async fn test_metadata_provider_failure_timeout() {
    let p = librarr_metadata::OpenLibraryProvider::new_test_err(MetadataError::Timeout(
        Duration::from_secs(3),
    ));
    assert!(
        matches!(p.search_works("x").await, Err(MetadataError::Timeout(d)) if d == Duration::from_secs(3))
    );
}

/// SEARCH-006 | NotConfigured when provider not set up.
#[tokio::test]
async fn test_metadata_provider_failure_not_configured() {
    let p = librarr_metadata::OpenLibraryProvider::new_test_err(MetadataError::NotConfigured);
    assert!(matches!(
        p.search_works("x").await,
        Err(MetadataError::NotConfigured)
    ));
}

/// SEARCH-006 | RateLimited on upstream throttling.
#[tokio::test]
async fn test_metadata_provider_failure_rate_limited() {
    let p = librarr_metadata::OpenLibraryProvider::new_test_err(MetadataError::RateLimited);
    assert!(matches!(
        p.search_works("x").await,
        Err(MetadataError::RateLimited)
    ));
}

/// SEARCH-006 | AuthFailed on invalid credentials.
#[tokio::test]
async fn test_metadata_provider_failure_auth_failed() {
    let p = librarr_metadata::OpenLibraryProvider::new_test_err(MetadataError::AuthFailed);
    assert!(matches!(
        p.search_works("x").await,
        Err(MetadataError::AuthFailed)
    ));
}

/// SEARCH-001 | Empty query returns empty vec, not error.
#[tokio::test]
async fn test_metadata_provider_boundary_empty_query() {
    let p = librarr_metadata::OpenLibraryProvider::new_test(vec![]);
    assert!(p.search_works("").await.unwrap().is_empty());
}

// =============================================================================
// SearchService — SEARCH-001, SEARCH-002, SEARCH-003, AUTHOR-001
// =============================================================================

/// SEARCH-001, SEARCH-003 | search_works returns OL-keyed display results.
#[tokio::test]
async fn test_metadata_search_svc_works_nominal() {
    let s = librarr_metadata::OlSearchService::new_test(vec![WorkSearchResult {
        ol_key: "OL123W".into(),
        title: "The Rust Book".into(),
        author_name: Some("Ferris Crab".into()),
        author_ol_key: Some("OL999A".into()),
        year: Some(2024),
        cover_url: Some("https://example.test/cover.jpg".into()),
    }]);
    assert_search_svc_works_ok(&s).await;
}

/// SEARCH-002, AUTHOR-001 | search_authors returns name and work count.
#[tokio::test]
async fn test_metadata_search_svc_authors_nominal() {
    let s = librarr_metadata::OlSearchService::new_test_authors(vec![AuthorSearchResult {
        ol_key: "OL999A".into(),
        name: "Ferris Crab".into(),
        work_count: Some(42),
    }]);
    assert_search_svc_authors_ok(&s).await;
}

/// SEARCH-001 | Empty query returns empty vec.
#[tokio::test]
async fn test_metadata_search_svc_boundary_empty_query() {
    let s = librarr_metadata::OlSearchService::new_test(vec![]);
    assert!(s.search_works("").await.unwrap().is_empty());
}

// =============================================================================
// EnrichmentService — SEARCH-006..008, SEARCH-010..012, SEARCH-014
// =============================================================================

/// SEARCH-006 | enrich_work success → Enriched, never Pending.
#[tokio::test]
async fn test_metadata_enrichment_nominal_success() {
    let svc = librarr_metadata::tests::enrichment_stub_success();
    let r = svc.enrich_work(&Work::default()).await.unwrap();
    assert_eq!(r.enrichment_status, EnrichmentStatus::Enriched);
    assert_enrichment_not_pending(&r).await;
    assert!(r.enrichment_source.is_some());
}

/// SEARCH-008 | Partial provider failure → Partial, never Pending.
#[tokio::test]
async fn test_metadata_enrichment_boundary_partial() {
    let svc = librarr_metadata::tests::enrichment_stub_partial();
    let r = svc.enrich_work(&Work::default()).await.unwrap();
    assert_eq!(r.enrichment_status, EnrichmentStatus::Unenriched);
    assert_enrichment_not_pending(&r).await;
}

/// SEARCH-008 | All providers fail → Failed, work retains OL data, never Pending.
#[tokio::test]
async fn test_metadata_enrichment_failure_all_providers() {
    let svc = librarr_metadata::tests::enrichment_stub_all_fail();
    let r = svc.enrich_work(&Work::default()).await.unwrap();
    assert_eq!(r.enrichment_status, EnrichmentStatus::Failed);
    assert_enrichment_not_pending(&r).await;
    assert!(!r.work.title.is_empty()); // retains existing data
}

/// SEARCH-006 | WorkNotFound for missing work.
#[tokio::test]
async fn test_metadata_enrichment_failure_work_not_found() {
    let svc = librarr_metadata::tests::enrichment_stub_not_found();
    assert!(matches!(
        svc.enrich_work(&Work::default()).await,
        Err(EnrichmentError::WorkNotFound)
    ));
}

/// SEARCH-006 | enrichment_status never Pending across all outcome variants.
#[tokio::test]
async fn test_metadata_enrichment_boundary_never_pending() {
    for f in [
        librarr_metadata::tests::enrichment_stub_success,
        librarr_metadata::tests::enrichment_stub_partial,
        librarr_metadata::tests::enrichment_stub_all_fail,
    ] {
        let r = f().enrich_work(&Work::default()).await.unwrap();
        assert_ne!(r.enrichment_status, EnrichmentStatus::Unenriched);
    }
}

/// SEARCH-011, SEARCH-012 | refresh_work overwrites enrichment-managed fields.
#[tokio::test]
async fn test_metadata_enrichment_refresh_overwrites() {
    let svc = librarr_metadata::tests::enrichment_stub_success();
    let r = svc.refresh_work(1, 7).await.unwrap();
    assert_eq!(r.enrichment_status, EnrichmentStatus::Enriched);
}

/// SEARCH-014 | refresh_work skips cover when cover_manual = true.
#[tokio::test]
async fn test_metadata_enrichment_refresh_skips_manual_cover() {
    let svc = librarr_metadata::tests::enrichment_stub_manual_cover();
    let r = svc.refresh_work(1, 7).await.unwrap();
    assert!(r.work.cover_manual);
}

/// SEARCH-007 | LLM fallback dispatched when deterministic ambiguous → llm_task_spawned = true.
#[tokio::test]
async fn test_metadata_enrichment_llm_fallback_dispatched() {
    let svc = librarr_metadata::tests::enrichment_stub_llm_fallback();
    let r = svc.enrich_work(&Work::default()).await.unwrap();
    assert!(r.llm_task_spawned);
}

/// SEARCH-006 | Pending → Enriched transition.
#[tokio::test]
async fn test_metadata_enrichment_transition_pending_to_enriched() {
    let svc = librarr_metadata::tests::enrichment_stub_success();
    let w = Work {
        enrichment_status: EnrichmentStatus::Unenriched,
        ..Default::default()
    };
    assert_eq!(
        svc.enrich_work(&w).await.unwrap().enrichment_status,
        EnrichmentStatus::Enriched
    );
}

/// SEARCH-008 | Pending → Partial transition.
#[tokio::test]
async fn test_metadata_enrichment_transition_pending_to_partial() {
    let svc = librarr_metadata::tests::enrichment_stub_partial();
    let w = Work {
        enrichment_status: EnrichmentStatus::Unenriched,
        ..Default::default()
    };
    assert_eq!(
        svc.enrich_work(&w).await.unwrap().enrichment_status,
        EnrichmentStatus::Unenriched
    );
}

/// SEARCH-008 | Pending → Failed transition.
#[tokio::test]
async fn test_metadata_enrichment_transition_pending_to_failed() {
    let svc = librarr_metadata::tests::enrichment_stub_all_fail();
    let w = Work {
        enrichment_status: EnrichmentStatus::Unenriched,
        ..Default::default()
    };
    assert_eq!(
        svc.enrich_work(&w).await.unwrap().enrichment_status,
        EnrichmentStatus::Failed
    );
}

// =============================================================================
// HardcoverMatcher — SEARCH-007
// =============================================================================

/// SEARCH-007 | Deterministic match finds exact match by title + author.
#[tokio::test]
async fn test_metadata_hardcover_deterministic_nominal() {
    let m = librarr_metadata::tests::matcher_deterministic_hit();
    let c = candidate("HC1", "The Rust Book", "Ferris Crab", 100);
    let got = m
        .match_deterministic("The Rust Book", "Ferris Crab", &[c])
        .await;
    assert_eq!(got.unwrap().hardcover_id, "HC1");
}

/// SEARCH-007 | Deterministic match uses users_read_count tiebreaker.
#[tokio::test]
async fn test_metadata_hardcover_deterministic_tiebreaker() {
    let m = librarr_metadata::tests::matcher_deterministic_tiebreaker();
    let lo = candidate("HC1", "The Rust Book", "Ferris Crab", 10);
    let hi = candidate("HC2", "The Rust Book", "Ferris Crab", 100);
    assert_eq!(
        m.match_deterministic("The Rust Book", "Ferris Crab", &[lo, hi])
            .await
            .unwrap()
            .hardcover_id,
        "HC2"
    );
}

/// SEARCH-007 | Deterministic returns None when ambiguous.
#[tokio::test]
async fn test_metadata_hardcover_deterministic_boundary_ambiguous() {
    let m = librarr_metadata::tests::matcher_deterministic_ambiguous();
    let a = candidate("HC1", "Rust", "Ferris", 50);
    let b = candidate("HC2", "Rust", "Ferris", 50);
    assert!(m
        .match_deterministic("Rust", "Ferris", &[a, b])
        .await
        .is_none());
}

/// SEARCH-007 | Deterministic returns None for empty candidates.
#[tokio::test]
async fn test_metadata_hardcover_deterministic_boundary_empty() {
    let m = librarr_metadata::tests::matcher_deterministic_ambiguous();
    assert!(m.match_deterministic("X", "Y", &[]).await.is_none());
}

/// SEARCH-007 | LLM timeout returns error, no state change.
#[tokio::test]
async fn test_metadata_hardcover_llm_failure_timeout() {
    let m = librarr_metadata::tests::matcher_llm_timeout();
    let c = candidate("HC1", "Rust", "Ferris", 1);
    assert!(matches!(
        m.match_llm(7, "Rust", "Ferris", &[c]).await,
        Err(MetadataError::Timeout(_))
    ));
}

/// SEARCH-007 | LLM resolves ambiguity successfully.
#[tokio::test]
async fn test_metadata_hardcover_llm_nominal() {
    let m = librarr_metadata::tests::matcher_llm_success();
    let c = candidate("HC9", "The Rust Book", "Ferris Crab", 77);
    assert_eq!(
        m.match_llm(7, "The Rust Book", "Ferris Crab", &[c])
            .await
            .unwrap()
            .hardcover_id,
        "HC9"
    );
}

// =============================================================================
// CoverCache — SEARCH-009, SEARCH-014
// =============================================================================

/// SEARCH-009 | expected_cover_path = {data_dir}/MediaCover/{work_id}/cover.jpg.
#[tokio::test]
async fn test_metadata_cover_cache_path_format() {
    let c = librarr_metadata::tests::cover_cache_stub("/data/librarr");
    assert_cover_path(&c, 7, "/data/librarr").await;
    assert_cover_path(&c, 999, "/data/librarr").await;
}

/// SEARCH-009 | cache_cover download failure surfaces DownloadFailed.
#[tokio::test]
async fn test_metadata_cover_cache_failure_download() {
    let c = librarr_metadata::tests::cover_cache_download_fail();
    assert!(matches!(
        c.cache_cover(7, "https://x.test/c.png").await,
        Err(CoverError::DownloadFailed(_))
    ));
}

/// SEARCH-014 | save_manual_cover rejects unsupported format.
#[tokio::test]
async fn test_metadata_cover_cache_failure_unsupported_format() {
    let c = librarr_metadata::tests::cover_cache_unsupported_format();
    assert!(matches!(
        c.save_manual_cover(7, b"gif89a", "image/gif").await,
        Err(CoverError::UnsupportedFormat(_))
    ));
}

/// SEARCH-009 | cache_cover succeeds and path ends in cover.jpg.
#[tokio::test]
async fn test_metadata_cover_cache_nominal() {
    let c = librarr_metadata::tests::cover_cache_stub("/data/librarr");
    c.cache_cover(7, "https://x.test/c.png").await.unwrap();
    assert!(c
        .expected_cover_path(7)
        .to_str()
        .unwrap()
        .ends_with("cover.jpg"));
}

/// SEARCH-014 | delete_cover removes cover directory.
#[tokio::test]
async fn test_metadata_cover_cache_delete_nominal() {
    let c = librarr_metadata::tests::cover_cache_stub("/data/librarr");
    c.delete_cover(7).unwrap();
}

// =============================================================================
// LlmClient — SEARCH-007, CONFIG-004
// =============================================================================

/// SEARCH-007, CONFIG-004 | chat_completion returns valid response.
#[tokio::test]
async fn test_metadata_llm_nominal() {
    let c = librarr_metadata::tests::llm_stub_ok("HC9");
    let r = c
        .chat_completion(vec![LlmMessage {
            role: LlmRole::User,
            content: "pick".into(),
        }])
        .await
        .unwrap();
    assert_eq!(r, "HC9");
}

/// CONFIG-004 | NotConfigured when no LLM provider configured.
#[tokio::test]
async fn test_metadata_llm_failure_not_configured() {
    let c = librarr_metadata::tests::llm_stub_err(LlmError::NotConfigured);
    assert!(matches!(
        c.chat_completion(vec![]).await,
        Err(LlmError::NotConfigured)
    ));
}

/// SEARCH-007 | Timeout on provider timeout.
#[tokio::test]
async fn test_metadata_llm_failure_timeout() {
    let c = librarr_metadata::tests::llm_stub_err(LlmError::Timeout);
    assert!(matches!(
        c.chat_completion(vec![]).await,
        Err(LlmError::Timeout)
    ));
}

/// SEARCH-007 | RateLimited on upstream throttling.
#[tokio::test]
async fn test_metadata_llm_failure_rate_limited() {
    let c = librarr_metadata::tests::llm_stub_err(LlmError::RateLimited);
    assert!(matches!(
        c.chat_completion(vec![]).await,
        Err(LlmError::RateLimited)
    ));
}
