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
//! - [`release_language_verdict`] — language gate for a background auto-grab decision

mod m1_embedded;
mod m2_path;
mod m3_string;
mod m4_scoring;
pub mod reconcile;
pub mod types;
pub mod work_dedup;

use std::path::Path;

pub use types::{
    Confidence, Extraction, ExtractionSource, MatchCandidate, MatchInput, MatchProvider,
};

/// Variant-title comparison key for Tier-A auto-match, built from the
/// identity authority's title parse (REQ-002) rather than a blind colon cut.
/// Two titles carry the same key exactly when they share a main title with
/// no vetoing or demoting tail evidence between them: edition junk
/// ("(Unabridged)") and diacritics fold away, but a true subtitle carried on
/// either side, or a series/volume marker carried on only one side, does
/// not — those cases keep distinct keys so genuinely different books (or a
/// sibling volume against a title with no volume information at all) never
/// collapse together. Degenerate input that parses to an empty main title
/// falls back to the plain normalization of the whole string.
pub fn normalize_title_variants(title: &str) -> String {
    use livrarr_domain::identity_matching::parse_title;

    let parsed = parse_title(title);
    if parsed.main.is_empty() {
        return m4_scoring::normalize(title);
    }

    let mut volumes = parsed.volume_numbers();
    volumes.sort_by(f64::total_cmp);
    volumes.dedup();
    let volume_key = volumes
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",");

    format!(
        "{}\u{1}{}\u{1}{}",
        parsed.main,
        parsed.subtitle.as_deref().unwrap_or(""),
        volume_key,
    )
}

/// Language compatibility between a parsed release and a candidate work, for
/// the Recognition matcher's background auto-grab decision (D7 recognition
/// corollary, REQ-011). Normalizes both sides through the same language
/// authority used to reconcile provider payloads (a release's declared-
/// language tag is a bare English word; a work's stored language is an ISO
/// 639-1 code) before deferring to the identity authority's language
/// comparison: a declared mismatch is [`LanguageVerdict::Veto`] (never
/// matches, in the background or interactively); a language-silent release
/// against a work whose language isn't the install default is
/// [`LanguageVerdict::Grey`] (never auto-matched in the background); anything
/// else is [`LanguageVerdict::Neutral`].
///
/// [`LanguageVerdict::Veto`]: livrarr_domain::identity_matching::LanguageVerdict::Veto
/// [`LanguageVerdict::Grey`]: livrarr_domain::identity_matching::LanguageVerdict::Grey
/// [`LanguageVerdict::Neutral`]: livrarr_domain::identity_matching::LanguageVerdict::Neutral
pub fn release_language_verdict(
    release_language: Option<&str>,
    work_language: Option<&str>,
    default_language: &str,
) -> livrarr_domain::identity_matching::LanguageVerdict {
    let release = livrarr_domain::normalize_language_opt(release_language);
    let work = livrarr_domain::normalize_language_opt(work_language);
    livrarr_domain::identity_matching::language_verdict(
        work.as_deref(),
        release.as_deref(),
        default_language,
    )
}

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
/// Returns the best (highest) score across extractions that pass the hard
/// gate (see [`fails_hard_gate`]) — an author-less extraction never wins a
/// score, however high `score_candidate` would otherwise put it (issue #142:
/// the candidate-substring fallback echoes the candidate's own title back as
/// the extraction, manufacturing a perfect title_sim with no author to check
/// it against). Range: 0.0–1.0; 0.0 when every extraction fails the gate.
pub fn best_match_score(parsed: &ParsedRelease, candidate: &MatchCandidate) -> f64 {
    parsed
        .extractions
        .iter()
        .filter(|ext| !m4_scoring::fails_hard_gate(ext, candidate))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(title: &str, author: &str) -> MatchCandidate {
        MatchCandidate {
            title: title.to_string(),
            author: author.to_string(),
            year: None,
            work_key: "OL1W".to_string(),
            author_key: None,
            cover_url: None,
            series: None,
            series_position: None,
            provider: MatchProvider::OpenLibrary,
            score: 0.0,
        }
    }

    // Issue #142: a monitored work's title landing as a bare substring of an
    // unrelated release title (a magazine mentioning the book, not the book
    // itself) must never auto-grab. The candidate-substring fallback finds
    // no author in either release, so the hard gate must zero both out.
    #[test]
    fn substring_title_hit_with_no_author_never_scores() {
        let candidates = [("The Civil War", "Bruce Catton")];
        let parsed = parse_release_title_with_candidates(
            "The.Civil.War.Monitor.Summer.2026.HYBRID.MAGAZINE.eBook-21A1",
            &candidates,
        );
        let score = best_match_score(&parsed, &candidate("The Civil War", "Bruce Catton"));
        assert_eq!(
            score, 0.0,
            "an author-less substring match must never clear the RSS threshold"
        );
    }

    #[test]
    fn short_title_inside_word_never_scores() {
        let candidates = [("Us", "Terrence Real")];
        let parsed = parse_release_title_with_candidates(
            "Brew.Your.Own.July-August.2026.HYBRID.MAGAZINE.eBook-21A1",
            &candidates,
        );
        let score = best_match_score(&parsed, &candidate("Us", "Terrence Real"));
        assert_eq!(
            score, 0.0,
            "'us' inside 'august' must never clear the RSS threshold"
        );
    }

    #[test]
    fn candidate_fallback_hit_with_real_author_still_scores() {
        let candidates = [("Project Hail Mary", "Andy Weir")];
        let parsed = parse_release_title_with_candidates(
            "Project.Hail.Mary.by.Andy.Weir.2026.EPUB",
            &candidates,
        );
        let score = best_match_score(&parsed, &candidate("Project Hail Mary", "Andy Weir"));
        assert!(
            score > 0.8,
            "a real title+author match must still score highly: got {score}"
        );
    }
}
