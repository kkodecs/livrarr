//! M4 — Fuzzy matching and scoring against OpenLibrary / Goodreads candidates.

use rapidfuzz::distance::levenshtein;
use unicode_normalization::UnicodeNormalization;

use super::types::{Extraction, MatchCandidate};

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
        string_similarity(title_ext, &candidate.title)
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

    let title_sim = string_similarity(title_ext, &candidate.title);

    // Title-only extraction can never auto-confirm.
    if author_ext.is_none() || author_ext.is_some_and(|a| a.is_empty()) {
        return true;
    }

    let author_sim = author_similarity(author_ext.unwrap(), &candidate.author);

    title_sim < 0.50 || author_sim < 0.40
}

// ---------------------------------------------------------------------------
// String similarity
// ---------------------------------------------------------------------------

/// Compute similarity between two strings.
/// Returns max of normalized Levenshtein and token-set Levenshtein.
pub fn string_similarity(a: &str, b: &str) -> f64 {
    let na = normalize(a);
    let nb = normalize(b);

    if na.is_empty() && nb.is_empty() {
        return 1.0;
    }
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
pub fn year_similarity(extracted: i32, candidate: i32) -> f64 {
    let diff = (extracted - candidate).unsigned_abs();
    if diff == 0 {
        1.0
    } else if diff <= 1 {
        0.8
    } else if diff <= 3 {
        0.5
    } else if extracted > candidate {
        // Extracted year is newer (e.g., audiobook release vs original publication).
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
    // NFKD decomposition.
    let decomposed: String = s.nfkd().collect();

    // Strip combining marks (category M) but keep base characters.
    let stripped: String = decomposed
        .chars()
        .filter(|c| !unicode_is_combining_mark(*c))
        .collect();

    let mut result = stripped.to_lowercase();

    // & → and
    result = result.replace('&', " and ");

    // Handle articles: "The X" / "X, The" → "X"
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

    // Strip non-alphanumeric, keeping CJK/Arabic/Cyrillic/spaces.
    result = result
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .collect();

    // Collapse whitespace.
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Canonicalize author names: "Last, First" → "First Last".
fn canonicalize_author(author: &str) -> String {
    // Handle "Last, First" format.
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

fn unicode_is_combining_mark(c: char) -> bool {
    // Unicode General Category M (Mark): Mn, Mc, Me
    matches!(unicode_general_category(c),
        '\u{0300}'..='\u{036F}'  // Combining Diacritical Marks
        | '\u{0483}'..='\u{0489}' // Cyrillic combining
        | '\u{0591}'..='\u{05BD}' // Hebrew
        | '\u{0610}'..='\u{061A}' // Arabic
        | '\u{064B}'..='\u{065F}' // Arabic
        | '\u{0670}'
        | '\u{06D6}'..='\u{06DC}' // Arabic
        | '\u{0730}'..='\u{074A}' // Syriac
        | '\u{0900}'..='\u{0903}' // Devanagari
        | '\u{093A}'..='\u{094F}' // Devanagari
        | '\u{0951}'..='\u{0957}' // Devanagari
        | '\u{0981}'..='\u{0983}' // Bengali
        | '\u{FE00}'..='\u{FE0F}' // Variation selectors
        | '\u{FE20}'..='\u{FE2F}' // Combining half marks
        | '\u{20D0}'..='\u{20FF}' // Combining for symbols
    )
}

// This is a simplified check — we use char ranges for common combining marks.
// For full Unicode correctness, we'd use the `unicode-general-category` crate,
// but this covers Latin, Cyrillic, Arabic, Hebrew, and Devanagari diacritics
// which are our primary use cases.
fn unicode_general_category(c: char) -> char {
    c // passthrough — the match is done in the caller
}
