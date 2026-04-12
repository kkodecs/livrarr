//! M2 — Path-based metadata extraction (Audiobookshelf-inspired with signal-based classification).

use std::path::Path;

use once_cell::sync::Lazy;
use regex::Regex;

use super::types::{Confidence, Extraction, ExtractionSource};

/// Directories to skip during path parsing (case-insensitive).
const IGNORE_DIRS: &[&str] = &[
    "books", "ebooks", "audiobooks", "fiction", "non-fiction", "nonfiction",
    "sci-fi", "fantasy", "to import", "downloads", "complete", "unsorted",
    "new", "incoming", "media", "library", "audio", "text",
];

/// Patterns for noise directories to collapse (multi-disc, parts).
static NOISE_DIR: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(disc|cd|part|disk)\s*\d+$").unwrap()
});

/// Series vocabulary that strongly signals a directory is a series name.
static SERIES_VOCAB: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(series|saga|chronicles|cycle|trilogy|duology|quartet|collection)\b").unwrap()
});

/// Sequence indicators in a child title that signal the parent is a series.
static CHILD_SEQUENCE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)(^|\s)(book|vol\.?|volume|#)\s*\d|^\d{1,3}[\.\s]\s*-?\s*\w").unwrap()
});

// Supplementary metadata patterns for title component.
static ASIN_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[([A-Z0-9]{10})\]").unwrap());
static NARRATOR_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\{([^}]+)\}").unwrap());
static YEAR_PREFIX_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\(?(\d{4})\)?\s*-\s*(.+)").unwrap());
static SEQUENCE_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(?:vol\.?\s*|volume\s*|book\s*|#)?(\d{1,3}(?:\.\d{1,2})?)\s*[\.\-]\s*(.+)").unwrap()
});

/// Extract metadata from filesystem path structure.
/// `scan_root` is the base directory the user selected for scanning.
/// Returns one or two extractions (two if parent classification is ambiguous).
pub fn extract_from_path(path: &Path, scan_root: &Path) -> Vec<Extraction> {
    let rel = match path.strip_prefix(scan_root) {
        Ok(r) => r,
        Err(_) => path,
    };

    // Split into components, filter noise and ignored dirs.
    let components: Vec<String> = rel
        .components()
        .filter_map(|c| {
            let s = c.as_os_str().to_str()?;
            Some(s.to_string())
        })
        .collect();

    if components.is_empty() {
        return vec![];
    }

    // Collapse noise directories and strip ignored roots.
    let cleaned: Vec<&str> = components
        .iter()
        .map(|s| s.as_str())
        .filter(|s| !NOISE_DIR.is_match(s))
        .filter(|s| !IGNORE_DIRS.contains(&s.to_lowercase().as_str()))
        .collect();

    if cleaned.is_empty() {
        return vec![];
    }

    // Leaf = candidate title (strip extension).
    let leaf_raw = cleaned[cleaned.len() - 1];
    let leaf = strip_extension(leaf_raw);

    // Extract supplementary metadata from title.
    let (title, sup) = extract_supplementary(&leaf);

    if title.trim().is_empty() {
        return vec![];
    }

    let parent = if cleaned.len() >= 2 { Some(cleaned[cleaned.len() - 2]) } else { None };
    let grandparent = if cleaned.len() >= 3 { Some(cleaned[cleaned.len() - 3]) } else { None };

    match parent {
        None => {
            // Flat file, title only.
            vec![make_extraction(
                &title, None, sup.series.as_deref(), sup.sequence, sup.year,
                sup.narrator.as_deref(), sup.asin.as_deref(), Confidence::MediumLow,
            )]
        }
        Some(parent_dir) => {
            let (author_score, series_score) = classify_parent(parent_dir, &title);

            if series_score >= 3 {
                // Parent is series. Grandparent is author (if available).
                let author = grandparent.map(|s| s.to_string());
                let series = Some(parent_dir.to_string());
                let conf = if author.is_some() { Confidence::MediumHigh } else { Confidence::Medium };
                vec![make_extraction(
                    &title, author.as_deref(), series.as_deref(),
                    sup.sequence, sup.year, sup.narrator.as_deref(), sup.asin.as_deref(), conf,
                )]
            } else if author_score >= 3 || (author_score > series_score) {
                // Parent is author.
                vec![make_extraction(
                    &title, Some(parent_dir), sup.series.as_deref(),
                    sup.sequence, sup.year, sup.narrator.as_deref(), sup.asin.as_deref(),
                    Confidence::MediumHigh,
                )]
            } else {
                // Ambiguous — produce two hypotheses.
                let as_author = make_extraction(
                    &title, Some(parent_dir), sup.series.as_deref(),
                    sup.sequence, sup.year, sup.narrator.as_deref(), sup.asin.as_deref(),
                    Confidence::Medium,
                );
                let as_series = make_extraction(
                    &title, grandparent, Some(parent_dir),
                    sup.sequence, sup.year, sup.narrator.as_deref(), sup.asin.as_deref(),
                    Confidence::Medium,
                );
                vec![as_author, as_series]
            }
        }
    }
}

/// Classify a parent directory as author or series.
/// Returns (author_score, series_score).
fn classify_parent(parent: &str, child_title: &str) -> (i32, i32) {
    let mut author_score = 0i32;
    let mut series_score = 0i32;

    // Strong author: comma-separated name form ("Last, First").
    if parent.contains(',') && parent.split(',').count() == 2 {
        let parts: Vec<&str> = parent.split(',').map(|s| s.trim()).collect();
        if !parts[0].is_empty() && !parts[1].is_empty()
            && !parts[0].chars().any(|c| c.is_ascii_digit())
        {
            author_score += 3;
        }
    }

    // Moderate author: 1-4 word tokens, no digits.
    let tokens: Vec<&str> = parent.split_whitespace().collect();
    if (1..=4).contains(&tokens.len()) && !parent.chars().any(|c| c.is_ascii_digit()) {
        author_score += 1;
    }

    // Strong series: child title has sequence indicators.
    if CHILD_SEQUENCE.is_match(child_title) {
        series_score += 3;
    }

    // Strong series: parent matches series vocabulary.
    if SERIES_VOCAB.is_match(parent) {
        series_score += 3;
    }

    // Moderate series: parent contains digits (not in a year pattern).
    static YEAR_ONLY: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d{4}$").unwrap());
    if parent.chars().any(|c| c.is_ascii_digit()) && !YEAR_ONLY.is_match(parent) {
        series_score += 1;
    }

    (author_score, series_score)
}

// ---------------------------------------------------------------------------
// Supplementary metadata extraction
// ---------------------------------------------------------------------------

struct Supplementary {
    series: Option<String>,
    sequence: Option<f64>,
    year: Option<i32>,
    narrator: Option<String>,
    asin: Option<String>,
}

/// Extract supplementary metadata from a title string, returning the cleaned title.
fn extract_supplementary(title: &str) -> (String, Supplementary) {
    let mut t = title.to_string();
    let mut sup = Supplementary {
        series: None, sequence: None, year: None, narrator: None, asin: None,
    };

    // ASIN: [B0015T963C]
    if let Some(cap) = ASIN_RE.captures(&t) {
        sup.asin = Some(cap[1].to_string());
        t = ASIN_RE.replace(&t, "").trim().to_string();
    }

    // Narrator: {name}
    if let Some(cap) = NARRATOR_RE.captures(&t) {
        sup.narrator = Some(cap[1].to_string());
        t = NARRATOR_RE.replace(&t, "").trim().to_string();
    }

    // Year prefix: (2024) - Title or 2024 - Title
    if let Some(cap) = YEAR_PREFIX_RE.captures(&t) {
        sup.year = cap[1].parse().ok();
        t = cap[2].to_string();
    }

    // Sequence prefix: Book 2 - Title, Vol. 3 Title, 01 - Title
    if let Some(cap) = SEQUENCE_RE.captures(&t) {
        sup.sequence = cap[1].parse().ok();
        t = cap[2].to_string();
    }

    // Subtitle: everything after first " - " (after stripping above).
    // We keep the full title but note the split point.
    // (Series detection may use the pre-subtitle portion.)

    (t.trim().to_string(), sup)
}

fn strip_extension(name: &str) -> String {
    static EXT_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)\.(epub|m4b|m4a|mp3|flac|ogg|wma|pdf|azw3?|mobi|cbz|cbr)$").unwrap()
    });
    EXT_RE.replace(name, "").to_string()
}

/// Basic sanity check for author values from directory names.
fn sanitize_path_author(author: &str) -> Option<String> {
    let trimmed = author.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Reject obvious non-author directory names.
    let lower = trimmed.to_lowercase();
    if matches!(
        lower.as_str(),
        "unknown" | "various" | "misc" | "other" | "temp" | "tmp"
    ) {
        return None;
    }
    Some(trimmed.to_string())
}

fn make_extraction(
    title: &str,
    author: Option<&str>,
    series: Option<&str>,
    sequence: Option<f64>,
    year: Option<i32>,
    narrator: Option<&str>,
    asin: Option<&str>,
    confidence: Confidence,
) -> Extraction {
    Extraction {
        title: Some(title.to_string()),
        author: author.and_then(sanitize_path_author),
        year,
        isbn: None,
        language: None,
        series: series.map(|s| s.to_string()),
        series_position: sequence,
        narrator: narrator.map(|s| s.to_string()),
        asin: asin.map(|s| s.to_string()),
        confidence,
        source: ExtractionSource::Path,
    }
}
