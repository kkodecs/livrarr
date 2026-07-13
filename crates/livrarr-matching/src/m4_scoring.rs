//! M4 — Fuzzy matching and scoring against OpenLibrary / Goodreads candidates.

use rapidfuzz::distance::levenshtein;
use unicode_normalization::UnicodeNormalization;

use livrarr_domain::identity_matching::{parse_title, title_verdict_with_positions, TitleVerdict};

use crate::types::{Extraction, MatchCandidate};

/// Compute the weighted composite match score between an extraction and a candidate.
/// Returns 0.0–1.0. Higher is better.
pub fn score_candidate(extraction: &Extraction, candidate: &MatchCandidate) -> f64 {
    let title_ext = extraction.title.as_deref().unwrap_or("");
    let author_ext = extraction.author.as_deref();
    let year_ext = extraction.year;
    let series_ext = extraction.series.as_deref();
    let seq_ext = extraction.series_position;

    let has_title = !title_ext.is_empty();
    let has_author = author_ext.is_some_and(|a| !a.is_empty());
    let has_year = year_ext.is_some() && candidate.year.is_some();
    let has_series = series_ext.is_some_and(|s| !s.is_empty()) && candidate.series.is_some();

    // Base weights.
    let mut w_title = if has_title { 0.45 } else { 0.0 };
    let mut w_author = if has_author { 0.40 } else { 0.0 };
    let mut w_year = if has_year { 0.10 } else { 0.0 };
    let mut w_series = if has_series { 0.05 } else { 0.0 };

    // Renormalize over available fields.
    let total_weight = w_title + w_author + w_year + w_series;
    if total_weight <= 0.0 {
        return 0.0;
    }
    w_title /= total_weight;
    w_author /= total_weight;
    w_year /= total_weight;
    w_series /= total_weight;

    // Compute similarities.
    let title_sim = if has_title {
        title_similarity_with_variants(
            title_ext,
            &candidate.title,
            seq_ext,
            candidate.series_position,
        )
    } else {
        0.0
    };

    let author_sim = if has_author {
        author_similarity(author_ext.unwrap(), &candidate.author)
    } else {
        0.0
    };

    let year_sim = if has_year {
        year_similarity(year_ext.unwrap(), candidate.year.unwrap())
    } else {
        0.0
    };

    let series_sim = if has_series {
        let s_sim = string_similarity(series_ext.unwrap(), candidate.series.as_deref().unwrap());
        let seq_match = match (seq_ext, candidate.series_position) {
            (Some(a), Some(b)) => (a - b).abs() < 0.01,
            _ => false,
        };
        if s_sim > 0.80 && seq_match {
            1.0
        } else {
            0.0
        }
    } else {
        0.0
    };

    (title_sim * w_title) + (author_sim * w_author) + (year_sim * w_year) + (series_sim * w_series)
}

/// Check hard gates. Returns true if this candidate should never be auto-confirmed.
pub fn fails_hard_gate(extraction: &Extraction, candidate: &MatchCandidate) -> bool {
    let title_ext = extraction.title.as_deref().unwrap_or("");
    let author_ext = extraction.author.as_deref();

    let title_sim = title_similarity_with_variants(
        title_ext,
        &candidate.title,
        extraction.series_position,
        candidate.series_position,
    );

    if author_ext.is_none() || author_ext.is_some_and(|a| a.is_empty()) {
        return true;
    }

    let author_sim = author_similarity(author_ext.unwrap(), &candidate.author);

    title_sim < 0.50 || author_sim < 0.40
}

// ---------------------------------------------------------------------------
// String similarity
// ---------------------------------------------------------------------------

/// Title similarity with variant folding layered on top of the plain string
/// similarity, via the identity authority's title parse (REQ-002). Folding
/// scores a pair 1.0 only when the parsed titles carry the same main title
/// with no vetoing or demoting tail evidence (`TitleVerdict::Same`) —
/// edition junk ("(Unabridged)") and diacritics fold away, but a subtitle
/// carried on only one side, disagreeing subtitles, or a series position
/// known on only one side do not: a missing position is not safe evidence
/// that two same-series releases share a volume. `pos_a`/`pos_b` are series
/// positions from caller-supplied metadata, folded in alongside any volume
/// markers the titles carry themselves. Folding never lowers a score: the
/// full-string similarity is the floor.
fn title_similarity_with_variants(a: &str, b: &str, pos_a: Option<f64>, pos_b: Option<f64>) -> f64 {
    let full = string_similarity(a, b);
    let parsed_a = parse_title(a);
    let parsed_b = parse_title(b);
    if title_verdict_with_positions(&parsed_a, pos_a, &parsed_b, pos_b) == TitleVerdict::Same {
        return 1.0;
    }
    full
}

/// Compute similarity between two strings.
/// Returns max of normalized Levenshtein and token-set Levenshtein. An empty
/// string on either side — including both sides empty — carries no positive
/// evidence and scores 0.0: it can never satisfy a similarity bar or gate.
pub fn string_similarity(a: &str, b: &str) -> f64 {
    let na = normalize(a);
    let nb = normalize(b);

    if na.is_empty() || nb.is_empty() {
        return 0.0;
    }

    let lev_sim = levenshtein_sim(&na, &nb);
    let token_sim = token_set_similarity(&na, &nb);

    lev_sim.max(token_sim)
}

/// Compute author similarity with name canonicalization.
pub fn author_similarity(a: &str, b: &str) -> f64 {
    let ca = canonicalize_author(a);
    let cb = canonicalize_author(b);
    string_similarity(&ca, &cb)
}

/// Year similarity with asymmetric penalty.
#[allow(clippy::if_same_then_else)]
pub fn year_similarity(extracted: i32, candidate: i32) -> f64 {
    let diff = (extracted - candidate).unsigned_abs();
    if diff == 0 {
        1.0
    } else if diff <= 1 {
        0.8
    } else if diff <= 3 {
        0.5
    } else if extracted > candidate {
        // Newer extraction (e.g. audiobook release vs original publication) — lenient
        0.5
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------
// Normalization
// ---------------------------------------------------------------------------

/// Normalize a string for comparison:
/// NFKD, strip combining marks (preserving CJK/Arabic/Cyrillic base chars),
/// lowercase, strip non-alphanumeric (preserving CJK/Arabic/Cyrillic),
/// normalize articles, & → and.
pub fn normalize(s: &str) -> String {
    let decomposed: String = s.nfkd().collect();

    let stripped: String = decomposed
        .chars()
        .filter(|c| !unicode_normalization::char::is_combining_mark(*c))
        .collect();

    let mut result = stripped.to_lowercase();

    result = result.replace('&', " and ");

    for article in &["the ", "a ", "an "] {
        if result.starts_with(article) {
            result = result[article.len()..].to_string();
        }
    }
    for article in &[", the", ", a", ", an"] {
        if result.ends_with(article) {
            result = result[..result.len() - article.len()].to_string();
        }
    }

    result = result
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect();

    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn canonicalize_author(author: &str) -> String {
    if author.contains(',') {
        let parts: Vec<&str> = author.splitn(2, ',').map(|s| s.trim()).collect();
        if parts.len() == 2 && !parts[1].is_empty() {
            return format!("{} {}", parts[1], parts[0]);
        }
    }
    author.to_string()
}

fn levenshtein_sim(a: &str, b: &str) -> f64 {
    let max_len = a.chars().count().max(b.chars().count());
    if max_len == 0 {
        return 1.0;
    }
    let dist = levenshtein::distance(a.chars(), b.chars());
    1.0 - (dist as f64 / max_len as f64)
}

fn token_set_similarity(a: &str, b: &str) -> f64 {
    let mut ta: Vec<&str> = a.split_whitespace().collect();
    let mut tb: Vec<&str> = b.split_whitespace().collect();
    ta.sort_unstable();
    tb.sort_unstable();
    let sa = ta.join(" ");
    let sb = tb.join(" ");
    levenshtein_sim(&sa, &sb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_empty_strings_score_zero_not_one() {
        assert_eq!(string_similarity("", ""), 0.0);
    }

    #[test]
    fn one_sided_empty_still_scores_zero() {
        assert_eq!(string_similarity("Dune", ""), 0.0);
        assert_eq!(string_similarity("", "Dune"), 0.0);
    }

    #[test]
    fn junk_only_difference_still_folds_to_one() {
        assert_eq!(
            title_similarity_with_variants("Dune (Unabridged)", "Dune", None, None),
            1.0
        );
    }

    #[test]
    fn embedded_volume_marker_vs_missing_does_not_force_one() {
        let a = "Mistborn: The Final Empire, Book 1";
        let b = "Mistborn";
        let full = string_similarity(a, b);
        assert!(full < 1.0, "fixture must not already be a full match");
        assert_eq!(title_similarity_with_variants(a, b, None, None), full);
    }

    #[test]
    fn known_position_on_one_side_blocks_junk_fold() {
        let a = "The Way of Kings (Unabridged)";
        let b = "The Way of Kings";
        let full = string_similarity(a, b);
        assert!(full < 1.0, "fixture must not already be a full match");
        assert_eq!(
            title_similarity_with_variants(a, b, Some(1.0), None),
            full,
            "a missing series position is not evidence the volumes agree"
        );
    }

    #[test]
    fn known_matching_positions_still_fold() {
        assert_eq!(
            title_similarity_with_variants(
                "The Way of Kings (Unabridged)",
                "The Way of Kings",
                Some(1.0),
                Some(1.0),
            ),
            1.0
        );
    }

    #[test]
    fn conflicting_known_positions_never_fold() {
        let a = "Mistborn (Unabridged)";
        let b = "Mistborn";
        let full = string_similarity(a, b);
        assert!(full < 1.0, "fixture must not already be a full match");
        assert_eq!(
            title_similarity_with_variants(a, b, Some(1.0), Some(2.0)),
            full
        );
    }
}
