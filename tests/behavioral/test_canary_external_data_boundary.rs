//! Boundary tests for the `livrarr-external-data` extraction canary.

use std::path::Path;
use std::process::Command;

use livrarr_domain::OutcomeClass;
use livrarr_external_data::{goodreads, NormalizedWorkDetail, ProviderClient, ProviderOutcome};

/// REQ-IDs: REQ-005
/// AC-IDs: AC-004
/// Directive: contract types and provider entry points are imported directly
/// from `livrarr_external_data`, not through the metadata compatibility shim.
#[test]
fn test_ac004_external_data_contract_types_and_entry_points_are_public() {
    let detail = NormalizedWorkDetail {
        title: Some("Boundary Work".to_string()),
        ..NormalizedWorkDetail::default()
    };
    let outcome = ProviderOutcome::Success(Box::new(detail));

    assert_eq!(outcome.class(), OutcomeClass::Success);
    assert!(outcome.can_merge());

    let _discovery_search_entry_point = goodreads::search_goodreads;
    let _enrichment_fetch_entry_point = ProviderClient::fetch;
}

/// REQ-IDs: REQ-011
/// AC-IDs: AC-005
/// Directive: `livrarr-external-data` must not depend on policy, identity,
/// enrichment, or database crates.
#[test]
fn test_ac005_external_data_cargo_tree_has_no_reverse_edges() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("behavioral crate should be inside the workspace");
    let manifest_path = workspace_root.join("Cargo.toml");

    let output = Command::new("cargo")
        .args(["tree", "-p", "livrarr-external-data", "--manifest-path"])
        .arg(&manifest_path)
        .output()
        .expect("cargo tree should run for livrarr-external-data");

    assert!(
        output.status.success(),
        "cargo tree failed\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let forbidden = [
        "livrarr-metadata",
        "livrarr-identity",
        "livrarr-enrichment",
        "livrarr-db",
    ];
    let offending_lines = stdout
        .lines()
        .filter(|line| forbidden.iter().any(|crate_name| line.contains(crate_name)))
        .collect::<Vec<_>>();

    assert!(
        offending_lines.is_empty(),
        "livrarr-external-data has forbidden dependency edge(s):\n{}",
        offending_lines.join("\n")
    );
}
