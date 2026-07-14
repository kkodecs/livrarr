#![allow(dead_code, unused_imports)]

//! Behavioral tests for search-fallback-chain Audible provider contracts.

use assert_matches::assert_matches;
use chrono::{Duration as ChronoDuration, Utc};
use livrarr_behavioral::stubs::StubHttpFetcher;
use livrarr_domain::services::{FetchError, FetchResponse};
use livrarr_domain::Work;
use livrarr_external_data::{NormalizedWorkDetail, ProviderOutcome};

#[derive(Debug, Clone)]
struct ContractAudibleProduct {
    asin: Option<String>,
    title: Option<String>,
    author: Option<String>,
    narrators: Option<Vec<String>>,
    series_name: Option<String>,
    series_position: Option<String>,
    runtime_length_min: Option<i32>,
    publisher: Option<String>,
    cover_url: Option<String>,
    language: Option<String>,
}

fn contract_score_provider_candidates(
    seed_title: &str,
    seed_author: &str,
    candidates: &[(String, String)],
    min_title_jaccard: f64,
    min_author_overlap: u32,
) -> Option<usize> {
    use livrarr_domain::text_norm::{author_tokens, jaccard, title_tokens};

    let seed_title_tokens = title_tokens(seed_title);
    let seed_author_tokens = author_tokens(seed_author);
    candidates
        .iter()
        .enumerate()
        .filter_map(|(idx, (title, author))| {
            let title_score = jaccard(&seed_title_tokens, &title_tokens(title));
            let overlap = seed_author_tokens
                .intersection(&author_tokens(author))
                .count() as u32;
            if title_score >= min_title_jaccard && overlap >= min_author_overlap {
                Some((idx, title_score, overlap))
            } else {
                None
            }
        })
        .max_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.2.cmp(&b.2))
        })
        .map(|(idx, _, _)| idx)
}

fn contract_map_audible_to_detail(product: &ContractAudibleProduct) -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: product.title.clone(),
        author_name: product.author.clone(),
        narrator: product
            .narrators
            .clone()
            .filter(|narrators| !narrators.is_empty()),
        duration_seconds: product.runtime_length_min.map(|minutes| minutes * 60),
        series_name: product.series_name.clone(),
        series_position: product
            .series_position
            .as_deref()
            .and_then(|position| position.parse::<f64>().ok()),
        cover_url: product.cover_url.as_ref().map(|url| {
            if url.starts_with("http://") {
                url.replacen("http://", "https://", 1)
            } else {
                url.clone()
            }
        }),
        publisher: product.publisher.clone(),
        asin: product.asin.clone(),
        language: product.language.clone(),
        ..NormalizedWorkDetail::default()
    }
}

fn not_yet_implemented() -> ! {
    todo!("search-fallback-chain Audible provider implementation is not yet wired")
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: exact match returns Some(0).
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_provider_score_exact_match_returns_first_candidate() {
    let candidates = vec![("Dune".to_string(), "Frank Herbert".to_string())];

    assert_eq!(
        contract_score_provider_candidates("Dune", "Frank Herbert", &candidates, 0.75, 1),
        Some(0)
    );
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: best candidate selected by jaccard then overlap.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_provider_score_best_candidate_by_jaccard_then_overlap() {
    let candidates = vec![
        ("Dune Messiah".to_string(), "Frank Herbert".to_string()),
        ("Dune".to_string(), "Brian Herbert".to_string()),
        ("Dune".to_string(), "Frank Herbert".to_string()),
    ];

    assert_eq!(
        contract_score_provider_candidates("Dune", "Frank Herbert", &candidates, 0.75, 1),
        Some(2)
    );
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: all below threshold returns None.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_provider_score_below_threshold_returns_none() {
    let candidates = vec![("Foundation".to_string(), "Isaac Asimov".to_string())];

    assert_eq!(
        contract_score_provider_candidates("Dune", "Frank Herbert", &candidates, 0.75, 1),
        None
    );
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: empty candidates returns None.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_provider_score_empty_candidates_returns_none() {
    assert_eq!(
        contract_score_provider_candidates("Dune", "Frank Herbert", &[], 0.75, 1),
        None
    );
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: missing author still scores on title.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_provider_score_missing_author_scores_on_title_when_overlap_not_required() {
    let candidates = vec![("Dune".to_string(), "".to_string())];

    assert_eq!(
        contract_score_provider_candidates("Dune", "Frank Herbert", &candidates, 0.75, 0),
        Some(0)
    );
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: ASIN direct lookup returns Success when match is good.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_audible_provider_fetch_asin_direct_lookup_success_when_match_good() {
    let http = StubHttpFetcher::with_ok(200, br#"{"asin":"B000FC0PBC","title":"Dune"}"#.to_vec());
    let outcome: ProviderOutcome<NormalizedWorkDetail> =
        ProviderOutcome::Success(Box::new(NormalizedWorkDetail {
            asin: Some("B000FC0PBC".into()),
            title: Some("Dune".into()),
            ..NormalizedWorkDetail::default()
        }));

    assert_eq!(http.call_count(), 0);
    assert_matches!(outcome, ProviderOutcome::Success(_));
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: ASIN lookup miss falls through to title+author search.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_audible_provider_fetch_asin_lookup_miss_falls_through_to_search() {
    let http = StubHttpFetcher::new();
    let attempted_paths = ["asin_lookup", "title_author_search"];

    assert_eq!(http.call_count(), 0);
    assert_eq!(attempted_paths, ["asin_lookup", "title_author_search"]);
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: ASIN lookup title mismatch falls through to search.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_audible_provider_fetch_asin_title_mismatch_falls_through_to_search() {
    let _http = StubHttpFetcher::with_ok(200, br#"{"title":"Foundation"}"#.to_vec());
    let attempted_paths = ["asin_lookup", "title_author_search"];

    assert!(attempted_paths.contains(&"title_author_search"));
    assert_eq!(attempted_paths[0], "asin_lookup");
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: title+author search returns best-scored candidate.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_audible_provider_fetch_title_author_search_returns_best_scored_candidate() {
    let _http = StubHttpFetcher::with_ok(200, b"{}".to_vec());
    let candidates = vec![
        ("Dune Messiah".to_string(), "Frank Herbert".to_string()),
        ("Dune".to_string(), "Frank Herbert".to_string()),
    ];

    assert_eq!(
        contract_score_provider_candidates("Dune", "Frank Herbert", &candidates, 0.75, 1),
        Some(1)
    );
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: no candidates above threshold returns NotFound.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_audible_provider_fetch_no_candidate_above_threshold_returns_not_found() {
    let _http = StubHttpFetcher::with_ok(200, b"{}".to_vec());
    let outcome: ProviderOutcome<NormalizedWorkDetail> = ProviderOutcome::NotFound;

    assert_matches!(outcome, ProviderOutcome::NotFound);
    not_yet_implemented();
}

/// REQ-IDs: REQ-013
/// AC-IDs: AC-011
/// Directive: network error returns WillRetry.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_sfc_audible_provider_fetch_network_error_returns_will_retry() {
    let _http = StubHttpFetcher::with_error(FetchError::Connection("reset".into()));
    let outcome: ProviderOutcome<NormalizedWorkDetail> = ProviderOutcome::WillRetry {
        reason: livrarr_domain::WillRetryReason::ServerError,
        next_attempt_at: Utc::now() + ChronoDuration::seconds(300),
    };

    assert_matches!(outcome, ProviderOutcome::WillRetry { .. });
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: maps all fields from canned AudibleProduct JSON.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_provider_map_all_fields_from_canned_product() {
    let detail = contract_map_audible_to_detail(&ContractAudibleProduct {
        asin: Some("B000FC0PBC".into()),
        title: Some("Dune".into()),
        author: Some("Frank Herbert".into()),
        narrators: Some(vec!["Scott Brick".into()]),
        series_name: Some("Dune".into()),
        series_position: Some("1".into()),
        runtime_length_min: Some(1260),
        publisher: Some("Audible Studios".into()),
        cover_url: Some("https://images.example/dune.jpg".into()),
        language: Some("en".into()),
    });

    assert_eq!(detail.asin.as_deref(), Some("B000FC0PBC"));
    assert_eq!(
        detail.narrator.as_deref(),
        Some(&["Scott Brick".to_string()][..])
    );
    assert_eq!(detail.duration_seconds, Some(75_600));
    not_yet_implemented();
}

/// REQ-IDs: REQ-013
/// AC-IDs: AC-011
/// Directive: handles None authors, narrators, series, images (no panic).
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_provider_map_handles_missing_optional_collections() {
    let detail = contract_map_audible_to_detail(&ContractAudibleProduct {
        asin: None,
        title: Some("Dune".into()),
        author: None,
        narrators: None,
        series_name: None,
        series_position: None,
        runtime_length_min: None,
        publisher: None,
        cover_url: None,
        language: None,
    });

    assert_eq!(detail.title.as_deref(), Some("Dune"));
    assert!(detail.narrator.is_none());
    assert!(detail.cover_url.is_none());
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: duration converts minutes to seconds.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_provider_map_duration_minutes_to_seconds() {
    let detail = contract_map_audible_to_detail(&ContractAudibleProduct {
        runtime_length_min: Some(42),
        asin: None,
        title: None,
        author: None,
        narrators: None,
        series_name: None,
        series_position: None,
        publisher: None,
        cover_url: None,
        language: None,
    });

    assert_eq!(detail.duration_seconds, Some(2520));
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-019
/// AC-IDs: AC-014
/// Directive: http:// cover URL upgraded to https://.
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_provider_map_http_cover_url_upgraded_to_https() {
    let detail = contract_map_audible_to_detail(&ContractAudibleProduct {
        cover_url: Some("http://images.example/dune.jpg".into()),
        asin: None,
        title: None,
        author: None,
        narrators: None,
        series_name: None,
        series_position: None,
        runtime_length_min: None,
        publisher: None,
        language: None,
    });

    assert_eq!(
        detail.cover_url.as_deref(),
        Some("https://images.example/dune.jpg")
    );
    not_yet_implemented();
}

/// REQ-IDs: REQ-013, REQ-014
/// AC-IDs: AC-011
/// Directive: empty narrators list -> narrator = None (not empty vec).
#[test]
#[ignore = "not yet implemented"]
fn test_sfc_audible_provider_map_empty_narrators_list_to_none() {
    let detail = contract_map_audible_to_detail(&ContractAudibleProduct {
        narrators: Some(vec![]),
        asin: None,
        title: None,
        author: None,
        series_name: None,
        series_position: None,
        runtime_length_min: None,
        publisher: None,
        cover_url: None,
        language: None,
    });

    assert!(detail.narrator.is_none());
    not_yet_implemented();
}

// The SFC `score_provider_candidates` canary was removed with the matching-conformance
// unit: that function is deleted (REQ-001/AC-001 — one shared `pick_best_candidate`),
// and this canary asserted nothing (`let _ = ...`). The rest of this stalled
// search-fallback-chain scaffolding remains for the separate orphan-test triage.
