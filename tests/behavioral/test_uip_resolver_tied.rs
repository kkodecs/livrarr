//! Behavioral tests for the unified-identity-path resolver conflict payload.
//!
//! These characterize only the QuorumTie payload projection: winner selection and
//! clustering are covered by the WCC resolver matrix and must remain unchanged.

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
        language: Some("en".to_string()),
        ..NormalizedWorkDetail::default()
    }
}

fn has_ol_key(captured: &[CapturedIdentity], key: &str) -> bool {
    captured
        .iter()
        .any(|identity| identity.ol_key.as_deref() == Some(key))
}

fn has_no_work_anchor(identity: &CapturedIdentity) -> bool {
    identity.ol_key.is_none() && identity.gr_key.is_none() && identity.hc_key.is_none()
}

#[test]
fn run_quorum_anchored_tie_projects_every_tied_cluster() {
    let responders = HashMap::from([
        (
            MetadataProvider::OpenLibrary,
            NormalizedWorkDetail {
                ol_key: Some("OL-WORK-X".to_string()),
                ..detail("Dune", "Frank Herbert")
            },
        ),
        (
            MetadataProvider::Hardcover,
            NormalizedWorkDetail {
                ol_key: Some("OL-WORK-Y".to_string()),
                ..detail("Dune", "Frank Herbert")
            },
        ),
    ]);

    match run_quorum(&responders, &isbn_seed()) {
        Resolution::Conflict { captured, tied, .. } => {
            assert!(
                captured.ol_key.as_deref() == Some("OL-WORK-X")
                    || captured.ol_key.as_deref() == Some("OL-WORK-Y"),
                "the representative captured set should come from one tied anchored cluster, got {captured:?}"
            );
            assert_eq!(
                tied.len(),
                2,
                "a two-cluster QuorumTie must expose both tied captured sets"
            );
            assert!(
                has_ol_key(&tied, "OL-WORK-X"),
                "tied clusters must include the OL-WORK-X captured set, got {tied:?}"
            );
            assert!(
                has_ol_key(&tied, "OL-WORK-Y"),
                "tied clusters must include the OL-WORK-Y captured set, got {tied:?}"
            );
        }
        other => panic!("expected anchored QuorumTie Conflict, got {other:?}"),
    }
}

#[test]
fn run_quorum_tied_cluster_projects_later_work_anchor_when_bridge_member_sorts_first() {
    let responders = HashMap::from([
        (
            MetadataProvider::GoogleBooks,
            NormalizedWorkDetail {
                isbn_13: Some("9780441478125".to_string()),
                ..detail("The Left Hand of Darkness", "Ursula K. Le Guin")
            },
        ),
        (
            MetadataProvider::OpenLibrary,
            NormalizedWorkDetail {
                ol_key: Some("OL-LEFT-HAND".to_string()),
                isbn_13: Some("9780441478125".to_string()),
                ..detail("The Left Hand of Darkness", "Ursula K. Le Guin")
            },
        ),
        (
            MetadataProvider::Goodreads,
            NormalizedWorkDetail {
                gr_key: Some("GR-DISPOSSESSED".to_string()),
                ..detail("The Dispossessed", "Ursula K. Le Guin")
            },
        ),
        (
            MetadataProvider::Hardcover,
            NormalizedWorkDetail {
                hc_key: Some("HC-DISPOSSESSED".to_string()),
                ..detail("The Dispossessed", "Ursula K. Le Guin")
            },
        ),
    ]);

    match run_quorum(&responders, &isbn_seed()) {
        Resolution::Conflict { tied, .. } => {
            assert_eq!(
                tied.len(),
                2,
                "a two-cluster QuorumTie must expose both tied captured sets"
            );
            assert!(
                has_ol_key(&tied, "OL-LEFT-HAND"),
                "the tied projection must include the OL work key from the later member of the GoogleBooks/OpenLibrary cluster, got {tied:?}"
            );
        }
        other => panic!("expected anchored QuorumTie Conflict, got {other:?}"),
    }
}

#[test]
fn run_quorum_anchorless_tie_projects_anchorless_tied_clusters() {
    let responders = HashMap::from([
        (
            MetadataProvider::GoogleBooks,
            NormalizedWorkDetail {
                isbn_13: Some("9780143111580".to_string()),
                ..detail("Kindred", "Octavia Butler")
            },
        ),
        (
            MetadataProvider::OpenLibrary,
            NormalizedWorkDetail {
                asin: Some("B000FC0PBC".to_string()),
                ..detail("Parable of the Sower", "Octavia Butler")
            },
        ),
    ]);

    match run_quorum(&responders, &isbn_seed()) {
        Resolution::Conflict { captured, tied, .. } => {
            assert!(
                has_no_work_anchor(&captured),
                "the representative captured set should be anchorless, got {captured:?}"
            );
            assert_eq!(
                tied.len(),
                2,
                "an anchorless two-cluster QuorumTie must expose both tied captured sets"
            );
            assert!(
                tied.iter().all(has_no_work_anchor),
                "anchorless tied clusters must not carry OL/GR/HC work anchors, got {tied:?}"
            );
        }
        other => panic!("expected anchorless QuorumTie Conflict, got {other:?}"),
    }
}

#[test]
fn run_quorum_majority_winner_remains_resolved_without_tied_payload() {
    let responders = HashMap::from([
        (
            MetadataProvider::OpenLibrary,
            NormalizedWorkDetail {
                ol_key: Some("OL-DUNE".to_string()),
                ..detail("Dune", "Frank Herbert")
            },
        ),
        (
            MetadataProvider::Hardcover,
            NormalizedWorkDetail {
                hc_key: Some("HC-DUNE".to_string()),
                ..detail("Dune", "Frank Herbert")
            },
        ),
        (
            MetadataProvider::Goodreads,
            NormalizedWorkDetail {
                gr_key: Some("GR-FOUNDATION".to_string()),
                ..detail("Foundation", "Isaac Asimov")
            },
        ),
    ]);

    assert_matches!(
        run_quorum(&responders, &isbn_seed()),
        Resolution::Resolved { identity, .. }
            if identity.ol_key.as_deref() == Some("OL-DUNE")
                && identity.hc_key.as_deref() == Some("HC-DUNE")
                && identity.gr_key.is_none()
    );
}
