//! Cutover-run/report vocabulary. IR v1 domain module
//! (ir-v1-identity-layer-rewrite.yaml:1001-1015) and `migration_plan`.
//! Reports contain aggregate counts and fingerprints, never user payloads.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityMigrationReport {
    /// Schema version of the frozen legacy-source snapshot used by rehearsal
    /// and pre-activation comparison.
    pub source_schema_version: u32,
    /// Fingerprint only of the frozen legacy-source domain, excluding staged
    /// and resolution rows.
    pub source_fingerprint: [u8; 32],
    /// Rehearsal value on snapshot reports; recomputed fresh from the
    /// resolved graph on the activation report.
    pub canonical_output_fingerprint: [u8; 32],
    pub mapped_route_count: u64,
    pub edition_count: u64,
    pub repair_cards: u64,
    pub group_cards: u64,
    pub field_cards: u64,
    pub contributor_cards: u64,
    pub index_ready: bool,
    pub trivially_empty: bool,
    pub legacy_work_count: u64,
}

/// `IdentityCutoverService::ensure_authority_ready`'s three outcomes, named
/// directly in its IR v1 output-type prose: "Active for an existing marker,
/// ActivatedFresh only after a transaction proves works and all legacy
/// identity/review sources empty, CutoverRequired for any non-empty inactive
/// database."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentityAuthorityReadiness {
    Active,
    ActivatedFresh,
    CutoverRequired,
}
