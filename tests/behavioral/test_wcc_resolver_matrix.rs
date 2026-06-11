#![allow(dead_code, unused_imports)]

//! Synthetic decision-matrix for `run_quorum` (WCC chunk-D characterization).
//!
//! Densifies the anchored-cluster winner rule beyond `test_wcc_resolver.rs`:
//! direct `run_quorum` calls over hand-built per-provider responses, no network,
//! deterministic. Cases marked [RED-until-D] fail on the trunk's current
//! `run_quorum` (which lets an anchorless cluster win) and go green when the
//! `has_work_anchor` filter lands. The rest are green now and lock behavior the
//! D change must preserve.

use assert_matches::assert_matches;
use livrarr_domain::identity::*;
use livrarr_domain::MetadataProvider;
use livrarr_external_data::NormalizedWorkDetail;
use livrarr_metadata::english_identity_resolver::*;
use std::collections::HashMap;

const ISBN: &str = "9780441013593";

fn isbn_seed() -> WorkSeed {
    WorkSeed {
        isbn_13: Some(ISBN.to_string()),
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

fn detail(title: &str, author: &str) -> NormalizedWorkDetail {
    NormalizedWorkDetail {
        title: Some(title.to_string()),
        author_name: Some(author.to_string()),
        ..NormalizedWorkDetail::default()
    }
}

/// Case 1 — a single anchored provider resolves trivially.
#[test]
fn matrix_single_anchored_resolves() {
    let mut r = HashMap::new();
    r.insert(
        MetadataProvider::OpenLibrary,
        NormalizedWorkDetail {
            ol_key: Some("OL-D".to_string()),
            isbn_13: Some(ISBN.to_string()),
            ..detail("Dune", "Frank Herbert")
        },
    );
    assert_matches!(
        run_quorum(&r, &isbn_seed()),
        Resolution::Resolved { identity, .. } if identity.ol_key.as_deref() == Some("OL-D")
    );
}

/// Case 2 — three providers, two anchored agree and one anchored dissents:
/// resolve to the two-provider agreement, drop the outlier (AC-013 majority).
#[test]
fn matrix_three_way_majority_drops_outlier() {
    let mut r = HashMap::new();
    r.insert(
        MetadataProvider::OpenLibrary,
        NormalizedWorkDetail {
            ol_key: Some("OL-D".to_string()),
            ..detail("Dune", "Frank Herbert")
        },
    );
    r.insert(
        MetadataProvider::Hardcover,
        NormalizedWorkDetail {
            hc_key: Some("HC-D".to_string()),
            ..detail("Dune", "Frank Herbert")
        },
    );
    r.insert(
        MetadataProvider::Goodreads,
        NormalizedWorkDetail {
            gr_key: Some("GR-X".to_string()),
            ..detail("Foundation", "Isaac Asimov")
        },
    );
    assert_matches!(
        run_quorum(&r, &isbn_seed()),
        Resolution::Resolved { identity, .. }
            if identity.ol_key.as_deref() == Some("OL-D")
                && identity.hc_key.as_deref() == Some("HC-D")
                && identity.gr_key.is_none()
    );
}

/// Case 3 — every responder is anchorless (ISBN-only): no cluster carries a work
/// anchor, so the anchorless clusters compete and the winner is `Resolved` with
/// the ISBN and no work anchor. An ISBN is a usable *provisional* identity, not a
/// non-identity — it is resolved, not held (see case 3b for the badge it derives).
#[test]
fn matrix_all_anchorless_resolves_with_isbn_no_anchor() {
    let mut r = HashMap::new();
    r.insert(
        MetadataProvider::GoogleBooks,
        NormalizedWorkDetail {
            isbn_13: Some(ISBN.to_string()),
            ..detail("Dune", "Frank Herbert")
        },
    );
    r.insert(
        MetadataProvider::OpenLibrary,
        NormalizedWorkDetail {
            isbn_13: Some(ISBN.to_string()),
            ..detail("Dune", "Frank Herbert")
        },
    );
    assert_matches!(
        run_quorum(&r, &isbn_seed()),
        Resolution::Resolved { identity, .. }
            if identity.isbn_13.as_deref() == Some(ISBN)
                && identity.ol_key.is_none()
                && identity.hc_key.is_none()
                && identity.gr_key.is_none()
    );
}

/// Case 3b — the governing cross-layer invariant: an ISBN-only resolved identity
/// (no work anchor) derives the `Provisional` badge — never `Confirmed`, never
/// `Pending` (`IdentityState::derived_identity_status`, identity.rs). This is what
/// case 3's anchorless `Resolved` relies on to render correctly; pinning it here
/// closes the resolver→status layer gap.
#[test]
fn matrix_isbn_only_identity_derives_provisional() {
    let anchors = CapturedIdentity {
        isbn_13: Some(ISBN.to_string()),
        ol_key: None,
        gr_key: None,
        hc_key: None,
        asin: None,
        title: "Dune".to_string(),
        author_name: "Frank Herbert".to_string(),
        language: Some("en".to_string()),
    };
    let state = IdentityState::Confirmed {
        anchors,
        method: IdentityMethod::IsbnDirect,
        score: None,
    };
    assert_eq!(
        state.derived_identity_status(),
        livrarr_domain::IdentityStatus::Provisional
    );
}

/// Case 4 — an anchored provider and an anchorless one agree on the work: resolve
/// to the anchor, the edition bridge rides along (corroboration adds, never locks).
#[test]
fn matrix_anchorless_corroborates_anchored() {
    let mut r = HashMap::new();
    r.insert(
        MetadataProvider::OpenLibrary,
        NormalizedWorkDetail {
            ol_key: Some("OL-D".to_string()),
            ..detail("Dune", "Frank Herbert")
        },
    );
    r.insert(
        MetadataProvider::GoogleBooks,
        NormalizedWorkDetail {
            isbn_13: Some(ISBN.to_string()),
            ..detail("Dune", "Frank Herbert")
        },
    );
    assert_matches!(
        run_quorum(&r, &isbn_seed()),
        Resolution::Resolved { identity, .. }
            if identity.ol_key.as_deref() == Some("OL-D")
                && identity.isbn_13.as_deref() == Some(ISBN)
    );
}

/// Case 5 — [RED-until-D] an anchored provider and an anchorless one return
/// *different* works: drop the anchorless dissenter and resolve to the anchor —
/// it must NOT be a Conflict (the trunk's current rule wrongly ties them 1-vs-1).
#[test]
fn matrix_anchorless_dissenter_dropped_not_conflict() {
    let mut r = HashMap::new();
    r.insert(
        MetadataProvider::OpenLibrary,
        NormalizedWorkDetail {
            ol_key: Some("OL-D".to_string()),
            ..detail("Dune", "Frank Herbert")
        },
    );
    r.insert(
        MetadataProvider::GoogleBooks,
        NormalizedWorkDetail {
            isbn_13: Some(ISBN.to_string()),
            ..detail("Some Unrelated Title", "Other Author")
        },
    );
    let res = run_quorum(&r, &isbn_seed());
    assert!(
        !matches!(res, Resolution::Conflict { .. }),
        "an anchorless dissenter must be dropped, not raise a Conflict"
    );
    assert_matches!(
        res,
        Resolution::Resolved { identity, .. } if identity.ol_key.as_deref() == Some("OL-D")
    );
}

/// Case 6 — two *anchored* providers return different works → terminal Conflict
/// (REQ-018/020): the anchored-cluster competition has no majority, so it is a
/// genuine identity conflict, not a silent merge. (Also asserted via the resolver
/// in `test_wcc_resolver::ac_020_033`; kept here so the standalone matrix fully
/// characterizes the winner rule.)
#[test]
fn matrix_two_anchored_different_works_conflict() {
    let mut r = HashMap::new();
    r.insert(
        MetadataProvider::OpenLibrary,
        NormalizedWorkDetail {
            ol_key: Some("OL-A".to_string()),
            ..detail("Dune", "Frank Herbert")
        },
    );
    r.insert(
        MetadataProvider::Hardcover,
        NormalizedWorkDetail {
            hc_key: Some("HC-B".to_string()),
            ..detail("Foundation", "Isaac Asimov")
        },
    );
    assert_matches!(run_quorum(&r, &isbn_seed()), Resolution::Conflict { .. });
}

/// Case 7 — AC-017 at the resolver layer: the same provider evidence resolves to
/// the same work anchors regardless of which anchor the seed started from. A seed
/// carrying only `ol_key` and one carrying only `hc_key`, over identical anchored
/// responders, yield identical resolved anchors — the resolver is seed-independent
/// (the `run_quorum` side of REQ-022 cross-path convergence). Complements the
/// convergence-merge AC-017 in `test_wcc_async_resolver`, which pins the DB merge.
#[test]
fn matrix_same_evidence_different_seed_anchor_resolves_identical() {
    let responders = HashMap::from([(
        MetadataProvider::OpenLibrary,
        NormalizedWorkDetail {
            ol_key: Some("OL-D".to_string()),
            hc_key: Some("HC-D".to_string()),
            isbn_13: Some(ISBN.to_string()),
            ..detail("Dune", "Frank Herbert")
        },
    )]);

    let mut seed_ol = isbn_seed();
    seed_ol.isbn_13 = None;
    seed_ol.ol_key = Some("OL-D".to_string());
    let mut seed_hc = isbn_seed();
    seed_hc.isbn_13 = None;
    seed_hc.hc_key = Some("HC-D".to_string());

    let resolved_anchors = |seed: &WorkSeed| match run_quorum(&responders, seed) {
        Resolution::Resolved { identity, .. } => (
            identity.ol_key,
            identity.hc_key,
            identity.gr_key,
            identity.isbn_13,
        ),
        other => panic!("expected Resolved, got {other:?}"),
    };

    assert_eq!(resolved_anchors(&seed_ol), resolved_anchors(&seed_hc));
}
