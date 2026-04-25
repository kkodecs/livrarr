//! Book matching engine — extracts metadata from files/paths/strings and matches against
//! OpenLibrary (with Goodreads fallback for foreign-language titles).
//!
//! Pipeline: Extract (M1+M2+M3) → Reconcile → Match (M4) → Confirm
//!
//! ## Public API
//!
//! - [`parse_release_title`] — parse a release/torrent title into extractions + side-channel
//! - [`best_match_score`] — score parsed extractions against a candidate, return best score
//! - [`extract_and_reconcile`] — full pipeline for file-based matching (manual import)
//! - [`should_auto_confirm`] / [`should_try_combinatorial`] — post-scoring decisions

mod m1_embedded;
mod m2_path;
mod m3_string;
mod m4_scoring;
pub mod reconcile;
pub mod types;
pub mod work_dedup;

use std::path::Path;

pub use types::{
    Confidence, DuplicateClass, Extraction, ExtractionSource, MatchCandidate, MatchInput,
    MatchProvider, MatchResult,
};

/// Parsed output from a release title string (M3 + side-channel metadata).
#[derive(Debug)]
pub struct ParsedRelease {
    pub extractions: Vec<Extraction>,
    pub format: Option<String>,
    pub year: Option<i32>,
    pub narrator: Option<String>,
    pub unabridged: Option<bool>,
    pub language: Option<String>,
}

/// Parse a release title / torrent name into structured extractions and side-channel metadata.
/// This is the primary entry point for RSS sync and search matching.
pub fn parse_release_title(title: &str) -> ParsedRelease {
    let (extractions, side) = m3_string::parse_string(title);
    ParsedRelease {
        extractions,
        format: side.format,
        year: side.year,
        narrator: side.narrator,
        unabridged: side.unabridged,
        language: side.language,
    }
}

/// Parse a release title with candidate-aware fallback.
/// When regex patterns fail, scans for known title/author substrings in the input.
pub fn parse_release_title_with_candidates(
    title: &str,
    candidates: &[(&str, &str)],
) -> ParsedRelease {
    let (extractions, side) = m3_string::parse_string_with_candidates(title, candidates);
    ParsedRelease {
        extractions,
        format: side.format,
        year: side.year,
        narrator: side.narrator,
        unabridged: side.unabridged,
        language: side.language,
    }
}

/// Score parsed extractions against a single candidate.
/// Returns the best (highest) score across all extractions. Range: 0.0–1.0.
pub fn best_match_score(parsed: &ParsedRelease, candidate: &MatchCandidate) -> f64 {
    parsed
        .extractions
        .iter()
        .map(|ext| m4_scoring::score_candidate(ext, candidate))
        .fold(0.0_f64, f64::max)
}

/// Run extraction and reconciliation on a single input (full file-based pipeline).
/// M1 file I/O runs inside spawn_blocking to avoid stalling the Tokio executor.
/// Returns ranked clusters for the caller to score against OL/Goodreads.
pub async fn extract_and_reconcile(input: &MatchInput) -> Vec<reconcile::Cluster> {
    let mut all_extractions: Vec<Extraction> = Vec::new();

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

    if let Some(ref path) = input.file_path {
        let m2_path = path.to_path_buf();
        let scan_root = input.scan_root.as_deref().unwrap_or(Path::new("/"));
        let path_extractions = m2_path::extract_from_path(&m2_path, scan_root);
        all_extractions.extend(path_extractions);
    }

    let parse_str = input.parse_string.clone().or_else(|| {
        input.file_path.as_ref().and_then(|p| {
            p.file_name()
                .and_then(|f| f.to_str())
                .map(|s| s.to_string())
        })
    });
    if let Some(ref s) = parse_str {
        let (string_extractions, _side) = m3_string::parse_string(s);
        all_extractions.extend(string_extractions);
    }

    reconcile::reconcile(all_extractions)
}

/// After scoring clusters against OL/Goodreads, determine auto-confirm status.
pub fn should_auto_confirm(
    cluster: &reconcile::Cluster,
    best_score: f64,
    is_synthetic: bool,
) -> bool {
    if is_synthetic {
        return false;
    }
    cluster.confidence >= Confidence::High && best_score >= 0.90
}

/// Check if combinatorial fallback should be triggered.
pub fn should_try_combinatorial(best_score: f64) -> bool {
    best_score < 0.80
}

/// Check hard gates on a candidate. Returns true if this candidate should never be auto-confirmed.
pub fn fails_hard_gate(extraction: &Extraction, candidate: &MatchCandidate) -> bool {
    m4_scoring::fails_hard_gate(extraction, candidate)
}

/// Compute string similarity (used for external scoring comparisons).
pub fn string_similarity(a: &str, b: &str) -> f64 {
    m4_scoring::string_similarity(a, b)
}

/// Compute author similarity with name canonicalization.
pub fn author_similarity(a: &str, b: &str) -> f64 {
    m4_scoring::author_similarity(a, b)
}
