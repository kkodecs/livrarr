//! Deterministic cleanup for work titles and author names at add-time.
//!
//! Runs at add-time before the values are stored on the `Work` record and
//! before provenance entries are written. The cleaned values are what get
//! locked as the identity anchor (`setter=User`) for LLM identity verification
//! at enrichment time.
//!
//! Design (from session 12 spec discussion):
//!   - LLM is optional. This module's deterministic cleanup must work
//!     standalone — LLM polish is a separate enhancement layer (deferred).
//!   - Aggressive cleanup is correct: divergence from provider strings is
//!     bridged by the LLM identity check (or fuzzy fallback). The locked
//!     title is for IDENTITY; provider strings are for ENRICHMENT.
//!
//! Title rules:
//!   1. Trim + collapse internal whitespace.
//!   2. Capitalization: title-case if input is all-uppercase or all-lowercase.
//!      Preserve mixed-case (stylized titles like "iCon").
//!   3. Strip trailing parenthetical when matching known patterns:
//!      series info, format markers, year markers, edition markers.
//!   4. Strip series-suffix after colon ("...: Book Two of the Expanse").
//!      Do NOT touch substantive descriptive subtitles
//!      ("The Power Broker: Robert Moses and the Fall of New York").
//!   5. Strip plain "A Novel" / "A Memoir" colon markers.
//!
//! Author rules:
//!   1. Trim + collapse internal whitespace.
//!   2. Capitalization fix when input is all-uppercase or all-lowercase.
//!   3. "Last, First" → "First Last" normalization.

use regex::Regex;
use std::sync::LazyLock;

/// Trailing parenthetical at the end of a title, e.g. "(Series, #1)".
static RE_TRAILING_PAREN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s*\(([^()]+)\)\s*$").unwrap());

/// Series info inside parens: "Series Name, #N" or "Series Name #N" or
/// "Book N of Series" (case-insensitive).
static RE_SERIES_PAREN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^.+,?\s*#\s*\d+(\.\d+)?$|^Book\s+\w+\s+of\s+.+$|^.+\s+series$").unwrap()
});

/// Format marker inside parens: "(Audiobook)", "(Unabridged)", etc.
static RE_FORMAT_PAREN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(audiobook|unabridged|abridged|ebook|kindle\s+edition|hardcover|paperback|mass\s+market|illustrated|annotated)$",
    )
    .unwrap()
});

/// Year inside parens: "(1963)" or "(1963 ed.)" or "(2010 reissue)".
static RE_YEAR_PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}(\s+\w+)?$").unwrap());

/// Edition marker inside parens: "(Deluxe Edition)", "(Anniversary Edition)".
static RE_EDITION_PAREN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(original|reissue|anniversary|deluxe|special|director'?s\s+cut|revised|updated|expanded|collector'?s|definitive)\s+edition$",
    )
    .unwrap()
});

/// Series-marker suffix after a colon. Handles "Book N", "Volume N", "Vol. N",
/// optionally followed by "of/in the Series" tail. Case-insensitive.
static RE_COLON_SERIES_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\s*:\s*(book|volume|vol\.?)\s+(\d+|[ivxlc]+|one|two|three|four|five|six|seven|eight|nine|ten)(\s+(of|in)\s+(the\s+)?.+)?$",
    )
    .unwrap()
});

/// Plain "A Novel" / "A Memoir" / "A Novella" markers after a colon.
static RE_COLON_NOVEL_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\s*:\s*a\s+(novel|memoir|novella|story|tale|poem)s?\s*$").unwrap()
});

/// Multiple consecutive whitespace characters.
static RE_WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// Small words that stay lowercase in title case unless first/last word.
const SMALL_WORDS: &[&str] = &[
    "a", "an", "and", "as", "at", "but", "by", "for", "from", "in", "into", "of", "on", "or",
    "the", "to", "vs", "vs.", "via", "with", "nor", "yet", "so", "if",
];

/// Clean a raw title to its canonical locked form.
///
/// See module docs for the rule set.
pub fn clean_title(raw: &str) -> String {
    let mut s = collapse_whitespace(raw);
    if s.is_empty() {
        return s;
    }

    s = fix_casing_if_needed(&s);
    s = strip_trailing_paren_if_match(&s);
    s = strip_colon_series_marker(&s);
    s = strip_colon_novel_marker(&s);
    s = collapse_whitespace(&s);
    s
}

/// Clean a raw author name to its canonical locked form.
pub fn clean_author(raw: &str) -> String {
    let mut s = collapse_whitespace(raw);
    if s.is_empty() {
        return s;
    }

    // "Last, First" → "First Last" before casing fix so the cased output
    // applies to the rearranged form.
    s = normalize_last_first(&s);
    s = fix_casing_if_needed(&s);
    s = collapse_whitespace(&s);
    s
}

fn collapse_whitespace(s: &str) -> String {
    RE_WHITESPACE.replace_all(s.trim(), " ").to_string()
}

/// Apply title-case if the input has no mixed casing (i.e. all-caps or
/// all-lowercase). Preserve mixed-case input as-is so stylized forms
/// (e.g. "iCon", "MASH", "eBook") survive.
fn fix_casing_if_needed(s: &str) -> String {
    let has_lower = s.chars().any(|c| c.is_lowercase());
    let has_upper = s.chars().any(|c| c.is_uppercase());
    let needs_fix = !(has_lower && has_upper);
    if !needs_fix {
        return s.to_string();
    }
    title_case(s)
}

/// Convert "all caps" or "all lowercase" string to title case, respecting
/// small-word lowercasing rules.
fn title_case(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace().collect();
    let last_idx = words.len().saturating_sub(1);
    words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let lower = w.to_lowercase();
            if i != 0 && i != last_idx && SMALL_WORDS.contains(&lower.as_str()) {
                lower
            } else {
                capitalize_first(&lower)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn capitalize_first(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    if let Some(c) = chars.next() {
        out.extend(c.to_uppercase());
    }
    out.push_str(chars.as_str());
    out
}

fn strip_trailing_paren_if_match(s: &str) -> String {
    let Some(cap) = RE_TRAILING_PAREN.captures(s) else {
        return s.to_string();
    };
    let inner = cap.get(1).unwrap().as_str().trim();
    let matched = RE_SERIES_PAREN.is_match(inner)
        || RE_FORMAT_PAREN.is_match(inner)
        || RE_YEAR_PAREN.is_match(inner)
        || RE_EDITION_PAREN.is_match(inner);
    if matched {
        RE_TRAILING_PAREN.replace(s, "").trim().to_string()
    } else {
        s.to_string()
    }
}

fn strip_colon_series_marker(s: &str) -> String {
    RE_COLON_SERIES_MARKER.replace(s, "").trim().to_string()
}

fn strip_colon_novel_marker(s: &str) -> String {
    RE_COLON_NOVEL_MARKER.replace(s, "").trim().to_string()
}

/// "Last, First" → "First Last". Preserves suffixes like "Jr.", "III".
fn normalize_last_first(s: &str) -> String {
    // Only fire on a single comma — "Last, First Middle" form.
    // Multi-comma forms ("Last, First, Jr.") are not safely invertible.
    let parts: Vec<&str> = s.splitn(2, ',').collect();
    if parts.len() != 2 {
        return s.to_string();
    }
    let last = parts[0].trim();
    let first_etc = parts[1].trim();
    if last.is_empty() || first_etc.is_empty() {
        return s.to_string();
    }
    // Don't re-arrange if `first_etc` looks like a name suffix
    // ("Jr.", "Sr.", "II", "III", "IV").
    let first_lower = first_etc.to_ascii_lowercase();
    if matches!(
        first_lower.as_str(),
        "jr." | "jr" | "sr." | "sr" | "ii" | "iii" | "iv" | "v"
    ) {
        return s.to_string();
    }
    format!("{first_etc} {last}")
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Title cases ───────────────────────────────────────────────────────────

    #[test]
    fn all_caps_becomes_title_case() {
        assert_eq!(clean_title("DUNE"), "Dune");
    }

    #[test]
    fn all_lowercase_becomes_title_case() {
        assert_eq!(clean_title("the way of kings"), "The Way of Kings");
    }

    #[test]
    fn mixed_case_stylized_preserved() {
        assert_eq!(clean_title("iCon: Steve Jobs"), "iCon: Steve Jobs");
        assert_eq!(clean_title("eBay for Dummies"), "eBay for Dummies");
    }

    #[test]
    fn small_words_stay_lowercase_mid_title() {
        // Title-case rule: "of", "the" stay lowercase in the middle.
        assert_eq!(clean_title("the path to power"), "The Path to Power");
    }

    #[test]
    fn series_paren_stripped() {
        assert_eq!(
            clean_title("The Way of Kings (The Stormlight Archive, #1)"),
            "The Way of Kings"
        );
    }

    #[test]
    fn format_paren_stripped() {
        assert_eq!(clean_title("Dune (Audiobook)"), "Dune");
        assert_eq!(clean_title("Dune (Unabridged)"), "Dune");
        assert_eq!(clean_title("Dune (Hardcover)"), "Dune");
        assert_eq!(clean_title("Dune (Kindle Edition)"), "Dune");
    }

    #[test]
    fn year_paren_stripped() {
        assert_eq!(clean_title("Cat's Cradle (1963)"), "Cat's Cradle");
    }

    #[test]
    fn edition_paren_stripped() {
        assert_eq!(clean_title("Dune (Deluxe Edition)"), "Dune");
        assert_eq!(
            clean_title("Brave New World (Anniversary Edition)"),
            "Brave New World"
        );
    }

    #[test]
    fn unknown_paren_preserved() {
        // Not a recognized pattern — keep the parenthetical.
        assert_eq!(
            clean_title("1984 (Signet Classics)"),
            "1984 (Signet Classics)"
        );
    }

    #[test]
    fn colon_series_marker_stripped() {
        assert_eq!(
            clean_title("Caliban's War: Book Two of the Expanse"),
            "Caliban's War"
        );
        assert_eq!(
            clean_title("Master of the Senate: Volume III"),
            "Master of the Senate"
        );
        assert_eq!(
            clean_title("Master of the Senate: Vol. III of The Years of Lyndon Johnson"),
            "Master of the Senate"
        );
    }

    #[test]
    fn colon_novel_marker_stripped() {
        assert_eq!(clean_title("Norwegian Wood: A Novel"), "Norwegian Wood");
        assert_eq!(
            clean_title("My Father at 100: A Memoir"),
            "My Father at 100"
        );
    }

    #[test]
    fn substantive_subtitle_preserved() {
        assert_eq!(
            clean_title("The Power Broker: Robert Moses and the Fall of New York"),
            "The Power Broker: Robert Moses and the Fall of New York"
        );
    }

    #[test]
    fn whitespace_normalized() {
        assert_eq!(clean_title("  Dune    "), "Dune");
        assert_eq!(clean_title("Dune\n\tSequel"), "Dune Sequel");
    }

    #[test]
    fn empty_input() {
        assert_eq!(clean_title(""), "");
        assert_eq!(clean_title("   "), "");
    }

    #[test]
    fn already_clean_no_op() {
        assert_eq!(clean_title("Norwegian Wood"), "Norwegian Wood");
        assert_eq!(
            clean_title("The Hitchhiker's Guide to the Galaxy"),
            "The Hitchhiker's Guide to the Galaxy"
        );
    }

    // ── Author cases ──────────────────────────────────────────────────────────

    #[test]
    fn author_all_caps_fixed() {
        assert_eq!(clean_author("FRANK HERBERT"), "Frank Herbert");
    }

    #[test]
    fn author_last_first_normalized() {
        assert_eq!(clean_author("Murakami, Haruki"), "Haruki Murakami");
        assert_eq!(clean_author("Caro, Robert A."), "Robert A. Caro");
    }

    #[test]
    fn author_already_normal() {
        assert_eq!(clean_author("Brandon Sanderson"), "Brandon Sanderson");
        assert_eq!(clean_author("J.R.R. Tolkien"), "J.R.R. Tolkien");
    }

    #[test]
    fn author_mixed_case_preserved() {
        // Mixed-case stylized author handles preserved.
        assert_eq!(clean_author("danah boyd"), "Danah Boyd"); // all-lowercase → fixed
        assert_eq!(clean_author("danah Boyd"), "danah Boyd"); // mixed → preserved
    }

    #[test]
    fn author_empty() {
        assert_eq!(clean_author(""), "");
        assert_eq!(clean_author("   "), "");
    }

    #[test]
    fn author_with_suffix_not_swapped() {
        // "Smith, Jr." should NOT become "Jr. Smith".
        assert_eq!(clean_author("Smith, Jr."), "Smith, Jr.");
    }
}
