#![allow(dead_code, unused_imports)]

//! Behavioral tests for english-work-lifecycle bulk resolver directives.

use livrarr_domain::identity::*;
use livrarr_metadata::bulk_resolver::resolve_bulk;
use livrarr_metadata::english_identity_resolver::{EnglishIdentityResolver, WorkSeed};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct TrackingResolver {
    in_flight: AtomicUsize,
    max_in_flight: AtomicUsize,
}
impl TrackingResolver {
    fn new() -> Self {
        Self {
            in_flight: AtomicUsize::new(0),
            max_in_flight: AtomicUsize::new(0),
        }
    }
}
impl EnglishIdentityResolver for TrackingResolver {
    async fn resolve(
        &self,
        _user_id: livrarr_domain::UserId,
        seed: &WorkSeed,
        _tier: LatencyTier,
    ) -> Result<Resolution, livrarr_domain::services::WorkIdentityError> {
        let current = self.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_in_flight.fetch_max(current, Ordering::SeqCst);
        let title = seed.title.clone().unwrap_or_default();
        if title.contains("slow") {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
        let author_name = seed.author_name.clone().unwrap_or_default();
        if title.contains("malformed") {
            Ok(Resolution::Unresolved {
                captured: CapturedIdentity {
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    isbn_13: None,
                    asin: None,
                    title,
                    author_name,
                    language: None,
                },
                reason: PendingReason::MalformedResponse,
                candidate_id: None,
            })
        } else {
            Ok(Resolution::Resolved {
                identity: CapturedIdentity {
                    ol_key: Some(title.clone()),
                    gr_key: None,
                    hc_key: None,
                    isbn_13: None,
                    asin: None,
                    title: title.clone(),
                    author_name,
                    language: None,
                },
                method: IdentityMethod::TitleAuthorSearch,
                candidate_id: CandidateId(String::new()),
            })
        }
    }
}
fn seeds(n: usize) -> Vec<WorkSeed> {
    (0..n)
        .map(|i| WorkSeed {
            ol_key: None,
            gr_key: None,
            hc_key: None,
            isbn_13: None,
            asin: None,
            title: Some(format!("OL{i}W")),
            author_name: Some("Author".into()),
            language: None,
            series_name: None,
            year: None,
            user_confirmed: false,
        })
        .collect()
}

/// REQ-IDs: REQ-002, REQ-013
/// Directive: 10 seeds with cap 4 never exceed four in-flight resolver calls.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_bulk_resolver_resolve_bulk_max_counter_le_4() {
    let r = TrackingResolver::new();
    let _ = resolve_bulk(&r, 1, seeds(10), 4).await;
    assert!(r.max_in_flight.load(Ordering::SeqCst) <= 4);
}
/// REQ-IDs: REQ-002, REQ-013
/// Directive: Output ordering matches input ordering.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_bulk_resolver_resolve_bulk_output_ordering_matches_input_ordering() {
    let r = TrackingResolver::new();
    let out = resolve_bulk(&r, 1, seeds(10), 4).await;
    for (i, item) in out.into_iter().enumerate() {
        assert!(
            matches!(item, Resolution::Resolved { identity, .. } if identity.ol_key.as_deref() == Some(format!("OL{i}W").as_str()))
        );
    }
}
/// REQ-IDs: REQ-002, REQ-013
/// Directive: Empty seed list returns empty Vec without invoking resolver.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_bulk_resolver_resolve_bulk_empty_vec_no_resolver_invocation() {
    let r = TrackingResolver::new();
    assert!(resolve_bulk(&r, 1, vec![], 4).await.is_empty());
    assert_eq!(r.max_in_flight.load(Ordering::SeqCst), 0);
}
/// REQ-IDs: REQ-002, REQ-013
/// Directive: concurrency=1 serializes resolver futures.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_bulk_resolver_resolve_bulk_concurrency_1_one_future_at_moment() {
    let r = TrackingResolver::new();
    let mut s = seeds(4);
    for seed in &mut s {
        if let Some(t) = seed.title.as_mut() {
            t.push_str("-slow");
        }
    }
    let _ = resolve_bulk(&r, 1, s, 1).await;
    assert_eq!(r.max_in_flight.load(Ordering::SeqCst), 1);
}
/// REQ-IDs: REQ-002, REQ-013
/// Directive: A malformed result at index 2 does not poison sibling seeds.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_bulk_resolver_resolve_bulk_malformed_index_2_siblings_normal() {
    let r = TrackingResolver::new();
    let mut s = seeds(5);
    s[2].title = Some("malformed".into());
    let out = resolve_bulk(&r, 1, s, 4).await;
    assert!(matches!(
        out[2],
        Resolution::Unresolved {
            reason: PendingReason::MalformedResponse,
            ..
        }
    ));
    assert!(matches!(out[4], Resolution::Resolved { .. }));
}
/// REQ-IDs: REQ-002, REQ-013
/// Directive: 1000 seeds with concurrency 4 preserve length and cap in-flight work.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_bulk_resolver_resolve_bulk_1000_seed_cap() {
    let r = TrackingResolver::new();
    let out = resolve_bulk(&r, 1, seeds(1000), 4).await;
    assert_eq!(out.len(), 1000);
    assert!(r.max_in_flight.load(Ordering::SeqCst) <= 4);
}
/// REQ-IDs: REQ-002, REQ-013
/// Directive: concurrency=0 clamps to one, avoiding a zero-permit deadlock.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_bulk_resolver_resolve_bulk_concurrency_0_clamps_to_1() {
    let r = TrackingResolver::new();
    let _ = resolve_bulk(&r, 1, seeds(3), 0).await;
    assert!(r.max_in_flight.load(Ordering::SeqCst) >= 1);
    assert!(r.max_in_flight.load(Ordering::SeqCst) <= 1);
}
/// REQ-IDs: REQ-002, REQ-013
/// Directive: Resolver-level transient parse failure maps to Pending without poisoning siblings.
#[tokio::test]
#[ignore = "not yet implemented"]
async fn test_ewl_bulk_resolver_resolve_bulk_bug_isolation_err_semantics() {
    let r = TrackingResolver::new();
    let mut s = seeds(5);
    s[2].title = Some("malformed".into());
    let out = resolve_bulk(&r, 1, s, 2).await;
    assert!(matches!(out[0], Resolution::Resolved { .. }));
    assert!(matches!(
        out[2],
        Resolution::Unresolved {
            reason: PendingReason::MalformedResponse,
            ..
        }
    ));
    assert!(matches!(out[4], Resolution::Resolved { .. }));
}
