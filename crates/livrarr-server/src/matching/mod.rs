//! Book matching engine — extracts metadata from files/paths/strings and matches against
//! OpenLibrary (with Goodreads fallback for foreign-language titles).
//!
//! Pipeline: Extract (M1+M2+M3) → Reconcile → Match (M4) → Confirm

pub mod m1_embedded;
pub mod m2_path;
pub mod m3_string;
pub mod m4_scoring;
pub mod reconcile;
pub mod types;

use std::path::Path;

use types::*;

/// Run extraction and reconciliation on a single input.
/// M1 file I/O runs inside spawn_blocking to avoid stalling the Tokio executor.
/// Returns ranked clusters for the caller to score against OL/Goodreads.
pub async fn extract_and_reconcile(input: &MatchInput) -> Vec<reconcile::Cluster> {
    let mut all_extractions: Vec<Extraction> = Vec::new();

    // --- Run applicable extraction methods ---

    // M1: Embedded metadata (if file on disk with supported format).
    // Runs in spawn_blocking because tag reading does synchronous file I/O.
    if let Some(ref path) = input.file_path {
        let p = path.clone();
        let grouped = input.grouped_paths.clone();
        let m1_result = tokio::task::spawn_blocking(move || {
            m1_embedded::extract_embedded(&p, grouped.as_deref())
        })
        .await
        .ok()
        .flatten();
        if let Some(extraction) = m1_result {
            all_extractions.push(extraction);
        }
    }

    // M2: Path parsing (if file path has directory structure).
    // For grouped audiobooks, primary_path is already the book container directory
    // (set by the scan grouping code), so we use it directly.
    if let Some(ref path) = input.file_path {
        let m2_path = path.to_path_buf();
        let scan_root = input
            .scan_root
            .as_deref()
            .unwrap_or(Path::new("/"));
        let path_extractions = m2_path::extract_from_path(&m2_path, scan_root);
        all_extractions.extend(path_extractions);
    }

    // M3: String parsing (on filename, folder name, or explicit parse string).
    let parse_str = input.parse_string.clone().or_else(|| {
        input.file_path.as_ref().and_then(|p| {
            p.file_name().and_then(|f| f.to_str()).map(|s| s.to_string())
        })
    });
    if let Some(ref s) = parse_str {
        let (string_extractions, _side) = m3_string::parse_string(s);
        all_extractions.extend(string_extractions);
    }

    // --- Reconcile ---
    reconcile::reconcile(all_extractions)
}

/// After scoring clusters against OL/Goodreads, determine auto-confirm status.
pub fn should_auto_confirm(
    cluster: &reconcile::Cluster,
    best_score: f64,
    is_synthetic: bool,
) -> bool {
    // Synthetic pairs never auto-confirm.
    if is_synthetic {
        return false;
    }
    // Must have high extraction confidence AND high match score.
    cluster.confidence >= Confidence::High && best_score >= 0.90
}

/// Check if combinatorial fallback should be triggered.
pub fn should_try_combinatorial(best_score: f64) -> bool {
    best_score < 0.80
}
