#![allow(dead_code, unused_imports)]

//! Behavioral tests for work-creation-consistency transport cache directives.

use livrarr_domain::{identity::CandidateId, MetadataProvider, UserId};
use livrarr_external_data::{transport_cache::TransportCache, NormalizedWorkDetail};
use std::collections::HashMap;
use std::time::Duration;

const USER_ID: UserId = 7;

fn payloads(title: &str) -> HashMap<MetadataProvider, NormalizedWorkDetail> {
    HashMap::from([(
        MetadataProvider::Hardcover,
        NormalizedWorkDetail {
            title: Some(title.to_string()),
            hc_key: Some("HC-1".to_string()),
            isbn_13: Some("9780441013593".to_string()),
            ..NormalizedWorkDetail::default()
        },
    )])
}

/// REQ-IDs: REQ-014
/// Directive: cache_put + cache_take is consume-once; the second take returns None.
#[test]
fn test_wcc_transport_cache_req_014_cache_take_consumes_entry_once() {
    let cache = TransportCache::new(Duration::from_secs(30));
    let candidate_id = CandidateId("candidate-consume-once".to_string());

    cache.cache_put(USER_ID, candidate_id.clone(), payloads("Dune"));
    let first = cache
        .cache_take(USER_ID, candidate_id.clone())
        .expect("first take should return stored payloads");
    let second = cache.cache_take(USER_ID, candidate_id);

    assert_eq!(
        first
            .get(&MetadataProvider::Hardcover)
            .and_then(|payload| payload.title.as_deref()),
        Some("Dune")
    );
    assert!(
        second.is_none(),
        "REQ-014: transport cache entries are consume-once"
    );
}

/// REQ-IDs: REQ-015
/// AC-IDs: AC-010
/// Directive: cache_take returns the exact stored payload map for in-process merge reuse.
#[test]
fn test_wcc_transport_cache_req_015_cache_take_returns_payloads_for_zero_network_merge() {
    let cache = TransportCache::new(Duration::from_secs(30));
    let candidate_id = CandidateId("candidate-payload".to_string());

    cache.cache_put(USER_ID, candidate_id.clone(), payloads("Already Retrieved"));
    let taken = cache
        .cache_take(USER_ID, candidate_id)
        .expect("payloads should be available to WorkService::add");

    assert_eq!(taken.len(), 1);
    assert_eq!(
        taken
            .get(&MetadataProvider::Hardcover)
            .and_then(|payload| payload.hc_key.as_deref()),
        Some("HC-1")
    );
    assert_eq!(
        taken
            .get(&MetadataProvider::Hardcover)
            .and_then(|payload| payload.isbn_13.as_deref()),
        Some("9780441013593")
    );
}

/// REQ-IDs: REQ-014
/// Directive: candidate payload cache is scoped by user_id and cannot be read cross-user.
#[test]
fn test_wcc_transport_cache_req_014_user_scoped_candidate_id_is_not_cross_user_readable() {
    let cache = TransportCache::new(Duration::from_secs(30));
    let candidate_id = CandidateId("same-browser-token".to_string());

    cache.cache_put(USER_ID, candidate_id.clone(), payloads("User Seven"));

    assert!(
        cache
            .cache_take(USER_ID + 1, candidate_id.clone())
            .is_none(),
        "REQ-014: another user must not read cached provider payloads"
    );
    assert!(
        cache.cache_take(USER_ID, candidate_id).is_some(),
        "cross-user miss must not consume the owning user's payload"
    );
}
