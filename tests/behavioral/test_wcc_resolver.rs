#![allow(dead_code, unused_imports)]

//! Behavioral tests for work-creation-consistency metadata identity resolver seams.
//!
//! The resolver fans out over a `HashMap<MetadataProvider, ProviderClient>` (the
//! same shape `fetch_internal_alternatives` uses); tests inject scriptable
//! `ProviderClient::Stub` clients to express per-provider scenarios.

use assert_matches::assert_matches;
use livrarr_domain::identity::*;
use livrarr_domain::{MetadataProvider, UserId, Work};
use livrarr_external_data::transport_cache::TransportCache;
use livrarr_external_data::{
    NormalizedWorkDetail, ProviderClient, ProviderOutcome, StubProviderClient,
};
use livrarr_metadata::english_identity_resolver::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const USER_ID: UserId = 42;
const DUNE_ISBN: &str = "9780441013593";

fn success(detail: NormalizedWorkDetail) -> ProviderOutcome<NormalizedWorkDetail> {
    ProviderOutcome::Success(Box::new(detail))
}

/// Build a resolver from a set of stub clients and an explicit config. Each stub
/// handle is cloned into the client map, so the caller keeps the original to read
/// its `call_count` after the resolve (the count is an `Arc`-shared counter).
fn make_resolver(
    stubs: Vec<StubProviderClient>,
    config: ResolverConfig,
) -> LiveEnglishIdentityResolver {
    let clients = stubs
        .into_iter()
        .map(|s| (s.provider, ProviderClient::Stub(s)))
        .collect::<HashMap<_, _>>();
    LiveEnglishIdentityResolver {
        clients,
        cache: Arc::new(TransportCache::new(Duration::from_secs(30))),
        config,
    }
}

/// Config with both prerequisite-gated providers enabled (GB key + LLM present),
/// so `select_providers` yields the full multi-provider set.
fn full_config() -> ResolverConfig {
    ResolverConfig {
        gb_key_present: true,
        llm_configured: false,
        ..ResolverConfig::default()
    }
}

fn isbn_seed() -> WorkSeed {
    WorkSeed {
        isbn_13: Some(DUNE_ISBN.to_string()),
        title: Some("Dune".to_string()),
        author_name: Some("Frank Herbert".to_string()),
        language: Some("en".to_string()),
        ol_key: None,
        gr_key: None,
        hc_key: None,
        asin: None,
        series_name: None,
        year: Some(1965),
        user_confirmed: false,
    }
}

fn title_seed(title: &str, author: &str) -> WorkSeed {
    WorkSeed {
        title: Some(title.to_string()),
        author_name: Some(author.to_string()),
        language: Some("en".to_string()),
        ol_key: None,
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
        series_name: None,
        year: None,
        user_confirmed: false,
    }
}

fn captured(title: &str, author: &str) -> CapturedIdentity {
    CapturedIdentity {
        title: title.to_string(),
        author_name: author.to_string(),
        language: Some("en".to_string()),
        ol_key: None,
        gr_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
    }
}

fn detail(title: &str, author: &str) -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: Some(title.to_string()),
        author_name: Some(author.to_string()),
        ..NormalizedWorkDetail::default()
    }
}

/// Directive: "the user's pick is the identity vote." A user-confirmed seed that
/// already carries a work anchor (the user picked a specific result) resolves
/// directly from the seed — the resolver does NOT fan out to providers, so the
/// interactive add is zero-network.
#[tokio::test]
async fn test_wcc_resolver_user_confirmed_work_anchor_skips_provider_fanout() {
    // OpenLibrary is always eligible, so without the fast-path it would be queried.
    let ol = StubProviderClient::new(MetadataProvider::OpenLibrary, ProviderOutcome::NotFound);
    let hc = StubProviderClient::new(
        MetadataProvider::Hardcover,
        success(NormalizedWorkDetail {
            hc_key: Some("HC-HOBBIT".to_string()),
            ..detail("The Hobbit", "J.R.R. Tolkien")
        }),
    );
    let resolver = make_resolver(vec![ol.clone(), hc.clone()], full_config());

    let seed = WorkSeed {
        gr_key: Some("5907".to_string()),
        title: Some("The Hobbit".to_string()),
        author_name: Some("J.R.R. Tolkien".to_string()),
        language: Some("en".to_string()),
        ol_key: None,
        hc_key: None,
        isbn_13: None,
        asin: None,
        series_name: None,
        year: None,
        user_confirmed: true,
    };

    let resolution = resolver
        .resolve(USER_ID, &seed, LatencyTier::Interactive)
        .await
        .expect("a user-confirmed work-anchored seed resolves");

    assert_eq!(
        ol.call_count(),
        0,
        "a user-confirmed work-anchored pick must not fan out to providers (zero-network add)"
    );
    assert_eq!(
        hc.call_count(),
        0,
        "no provider should be queried for a trusted pick"
    );
    assert_matches!(resolution, Resolution::Resolved { .. });
}

/// REQ-IDs: REQ-008, REQ-011
/// AC-IDs: AC-001
/// Directive: an HC-resolvable ISBN absent from OpenLibrary still yields a non-empty
/// HC-sourced resolution (the #97 regression) — the resolver fans out, it does not
/// stop at a single provider.
#[tokio::test]
async fn test_wcc_resolver_ac_001_hardcover_candidate_when_openlibrary_absent() {
    let ol = StubProviderClient::new(MetadataProvider::OpenLibrary, ProviderOutcome::NotFound);
    let hc = StubProviderClient::new(
        MetadataProvider::Hardcover,
        success(NormalizedWorkDetail {
            hc_key: Some("HC-DUNE".to_string()),
            isbn_13: Some(DUNE_ISBN.to_string()),
            ..detail("Dune", "Frank Herbert")
        }),
    );
    let resolver = make_resolver(vec![ol, hc], full_config());

    let resolution = resolver
        .resolve(USER_ID, &isbn_seed(), LatencyTier::Interactive)
        .await
        .expect("resolver should treat an OL miss as provider abstention, not Err");

    assert_matches!(
        resolution,
        Resolution::Resolved { identity, .. }
            if identity.hc_key.as_deref() == Some("HC-DUNE")
                && identity.isbn_13.as_deref() == Some(DUNE_ISBN)
    );
}

/// REQ-IDs: REQ-008, REQ-021
/// AC-IDs: AC-030
/// Directive: prerequisite-lacking providers are excluded without narrowing an ISBN
/// seed to one provider.
#[test]
fn test_wcc_resolver_ac_030_select_providers_excludes_prerequisite_lacking_but_keeps_multi_provider_isbn(
) {
    let resolver = make_resolver(vec![], full_config());
    let selected = resolver.select_providers(&isbn_seed(), LatencyTier::Interactive);

    assert!(selected.contains(&MetadataProvider::OpenLibrary));
    assert!(selected.contains(&MetadataProvider::Hardcover));
    assert!(selected.contains(&MetadataProvider::GoogleBooks));
    assert!(!selected.contains(&MetadataProvider::Audnexus));
    assert!(
        selected.len() >= 2,
        "REQ-008: an ISBN seed must not be narrowed to a single hardcoded provider"
    );
}

/// REQ-IDs: REQ-011, REQ-018, REQ-022
/// AC-IDs: AC-032
/// Directive: an ISBN resolving only to Google Books carries no *work* anchor, so
/// it cannot be `Confirmed` — but an ISBN is a usable *provisional* identity, not
/// a non-identity. `resolve` returns `Resolved` (ISBN carried, no work anchor);
/// the badge layer (`IdentityState::derived_identity_status`) renders that
/// `Provisional`. It is NOT left Unresolved/Pending, and NOT a Confirmed lock —
/// consistent with the no-responder Tier-A path. ("ISBN is a bridge, not a lock"
/// means Provisional, not Pending.)
#[tokio::test]
async fn test_wcc_resolver_ac_032_isbn_google_books_only_resolves_provisional_not_confirmed_or_pending(
) {
    let ol = StubProviderClient::new(MetadataProvider::OpenLibrary, ProviderOutcome::NotFound);
    let hc = StubProviderClient::new(MetadataProvider::Hardcover, ProviderOutcome::NotFound);
    let gb = StubProviderClient::new(
        MetadataProvider::GoogleBooks,
        success(NormalizedWorkDetail {
            isbn_13: Some(DUNE_ISBN.to_string()),
            ..detail("Dune", "Frank Herbert")
        }),
    );
    let resolver = make_resolver(vec![ol, hc, gb], full_config());

    let resolution = resolver
        .resolve(USER_ID, &isbn_seed(), LatencyTier::Interactive)
        .await
        .expect("GB-only ISBN should resolve without error");

    // Resolved with the ISBN bridge and no work anchor → Provisional downstream.
    assert_matches!(
        resolution,
        Resolution::Resolved { identity, method, .. }
            if identity.isbn_13.as_deref() == Some(DUNE_ISBN)
                && identity.ol_key.is_none()
                && identity.hc_key.is_none()
                && identity.gr_key.is_none()
                && method == IdentityMethod::IsbnDirect
    );
}

/// REQ-IDs: REQ-010
/// AC-IDs: AC-024
/// Directive: a resolving ISBN that no provider matches falls through to Tier-A by
/// its identifier — never to a fuzzy title/author confirmation list. Each provider
/// is fetched exactly once (no fuzzy second pass at the resolver level).
#[tokio::test]
async fn test_wcc_resolver_ac_024_resolving_isbn_does_not_fall_through_to_fuzzy_search() {
    let ol = StubProviderClient::new(MetadataProvider::OpenLibrary, ProviderOutcome::NotFound);
    let resolver = make_resolver(vec![ol.clone()], full_config());

    let resolution = resolver
        .resolve(USER_ID, &isbn_seed(), LatencyTier::Interactive)
        .await
        .expect("a resolving ISBN must not error when providers miss");

    assert_matches!(
        resolution,
        Resolution::Resolved { method, .. } if method == IdentityMethod::IsbnDirect
    );
    assert!(
        !matches!(resolution, Resolution::NeedsConfirmation { .. }),
        "AC-024: a resolving ISBN must not fall through to a fuzzy-search confirmation list"
    );
    assert_eq!(
        ol.call_count(),
        1,
        "AC-024: each provider is fetched once; no fuzzy second pass"
    );
}

/// REQ-IDs: REQ-021, REQ-023
/// AC-IDs: AC-018
/// Directive: Interactive resolve returns without awaiting a slow/background-only
/// provider.
#[tokio::test]
async fn test_wcc_resolver_ac_018_interactive_returns_without_awaiting_slow_provider() {
    let slow = StubProviderClient::new(
        MetadataProvider::OpenLibrary,
        success(NormalizedWorkDetail {
            ol_key: Some("OL45883W".to_string()),
            ..detail("Dune", "Frank Herbert")
        }),
    )
    .with_delay(Duration::from_secs(5));
    let resolver = make_resolver(
        vec![slow.clone()],
        ResolverConfig {
            call_timeout: Duration::from_millis(25),
            ..ResolverConfig::default()
        },
    );

    let started = Instant::now();
    let resolution = resolver
        .resolve(USER_ID, &isbn_seed(), LatencyTier::Interactive)
        .await
        .expect("AC-018: a slow provider must not fail interactive resolve");

    assert!(
        started.elapsed() < Duration::from_millis(250),
        "AC-018: interactive resolve must return inside the foreground latency budget"
    );
    assert_matches!(
        &resolution,
        Resolution::Resolved { .. }
            | Resolution::NeedsConfirmation { .. }
            | Resolution::Unresolved { .. }
    );
    assert!(
        slow.call_count() <= 1,
        "AC-018/REQ-021: a slow provider must not be repeatedly awaited in Interactive tier"
    );
}

/// REQ-IDs: REQ-018, REQ-025
/// AC-IDs: AC-022
/// Directive: one provider timeout/abstention still yields a Resolution from the
/// hard identifier and no Conflict.
#[tokio::test]
async fn test_wcc_resolver_ac_022_timeout_abstains_and_resolution_still_built() {
    let abstaining = StubProviderClient::new(
        MetadataProvider::OpenLibrary,
        success(NormalizedWorkDetail {
            ol_key: Some("OL45883W".to_string()),
            ..detail("Dune", "Frank Herbert")
        }),
    )
    .with_delay(Duration::from_secs(5));
    let resolver = make_resolver(
        vec![abstaining.clone()],
        ResolverConfig {
            call_timeout: Duration::from_millis(25),
            ..ResolverConfig::default()
        },
    );

    let resolution = resolver
        .resolve(USER_ID, &isbn_seed(), LatencyTier::Bulk)
        .await
        .expect("AC-022: a provider timeout is an abstention, not resolver Err");

    assert_matches!(
        &resolution,
        Resolution::Resolved { identity, .. }
            if identity.isbn_13.as_deref() == Some(DUNE_ISBN)
                || identity.ol_key.is_some()
                || identity.hc_key.is_some()
    );
    assert!(
        !matches!(resolution, Resolution::Conflict { .. }),
        "AC-022/REQ-025: abstainers must not count as quorum disagreement"
    );
    assert_eq!(
        abstaining.call_count(),
        1,
        "AC-022: the abstaining provider double should be exercised by the real resolve seam"
    );
}

/// REQ-IDs: REQ-018, REQ-020
/// AC-IDs: AC-020, AC-033
/// Directive: same ISBN returning different works conflicts; edition-variant titles
/// corroborate.
#[test]
fn test_wcc_resolver_ac_020_033_run_quorum_conflict_and_edition_variant_corrobates() {
    let mut responders = HashMap::new();
    responders.insert(
        MetadataProvider::OpenLibrary,
        NormalizedWorkDetail {
            ol_key: Some("OL-DUNE".to_string()),
            isbn_13: Some(DUNE_ISBN.to_string()),
            ..detail("Dune", "Frank Herbert")
        },
    );
    responders.insert(
        MetadataProvider::Hardcover,
        NormalizedWorkDetail {
            hc_key: Some("HC-OTHER".to_string()),
            isbn_13: Some(DUNE_ISBN.to_string()),
            ..detail("Different Book", "Other Author")
        },
    );

    let conflict = run_quorum(&responders, &isbn_seed());
    assert_matches!(conflict, Resolution::Conflict { .. });

    responders.insert(
        MetadataProvider::GoogleBooks,
        NormalizedWorkDetail {
            isbn_13: Some(DUNE_ISBN.to_string()),
            ..detail("Dune (Illustrated Edition)", "Frank Herbert")
        },
    );
    let corroborated = run_quorum(&responders, &isbn_seed());
    assert_matches!(
        corroborated,
        Resolution::Resolved { identity, .. }
            if identity.title == "Dune" && identity.author_name == "Frank Herbert"
    );
}

/// REQ-IDs: REQ-024, REQ-014
/// AC-IDs: AC-021
/// Directive: GR payload verification is payload-inspection only: match passes,
/// mismatch/title-None fails.
#[test]
fn test_wcc_resolver_ac_021_verify_gr_payload_strips_mismatch_and_antibot_payload() {
    let captured = captured("Dune", "Frank Herbert");

    let matching = NormalizedWorkDetail {
        gr_key: Some("234225".to_string()),
        ..detail("Dune", "Frank Herbert")
    };
    let mismatched = NormalizedWorkDetail {
        gr_key: Some("234225".to_string()),
        ..detail("Dune Messiah", "Frank Herbert")
    };
    let antibot = NormalizedWorkDetail {
        gr_key: Some("234225".to_string()),
        title: None,
        author_name: Some("Frank Herbert".to_string()),
        ..NormalizedWorkDetail::default()
    };

    assert!(verify_gr_payload(&matching, &captured));
    assert!(!verify_gr_payload(&mismatched, &captured));
    assert!(!verify_gr_payload(&antibot, &captured));
}

/// REQ-IDs: REQ-010, REQ-014
/// AC-IDs: AC-024
/// Directive: resolving identifiers are carried into the transient Work and no
/// persisted Work is required.
#[test]
fn test_wcc_resolver_ac_024_build_transient_work_from_seed_carries_identifiers_without_persisted_id(
) {
    let seed = WorkSeed {
        ol_key: Some("OL45883W".to_string()),
        gr_key: Some("234225".to_string()),
        hc_key: Some("HC-DUNE".to_string()),
        asin: Some("B000N2HCP6".to_string()),
        ..isbn_seed()
    };

    let transient: Work = build_transient_work_from_seed(&seed, USER_ID);

    assert_eq!(
        transient.id, 0,
        "transient discovery Work must never be a persisted row"
    );
    assert_eq!(transient.user_id, USER_ID);
    assert_eq!(transient.isbn_13.as_deref(), Some(DUNE_ISBN));
    assert_eq!(transient.asin.as_deref(), Some("B000N2HCP6"));
    assert_eq!(transient.ol_key.as_deref(), Some("OL45883W"));
    assert_eq!(transient.gr_key.as_deref(), Some("234225"));
    assert_eq!(transient.hc_key.as_deref(), Some("HC-DUNE"));
}
