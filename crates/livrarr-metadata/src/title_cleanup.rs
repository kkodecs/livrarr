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

use livrarr_http::HttpClient;
use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;
use std::time::Duration;

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

// ---------------------------------------------------------------------------
// LLM-assisted polish at add-time (Option C from session 12 spec)
// ---------------------------------------------------------------------------

/// 2.5s ceiling on the LLM call per the design spec — keeps add-work fast.
const LLM_POLISH_TIMEOUT: Duration = Duration::from_millis(2_500);

const POLISH_SYSTEM_INSTRUCTION: &str = r#"You are a metadata normalization tool for a book library system. Given a raw book title and author from a search result, return cleaned canonical forms.

Return ONLY a JSON object with EXACTLY these fields (these names are required):
{
  "title": "<cleaned title string>",
  "author_name": "<cleaned author string>",
  "series_name": "<series name or null>",
  "series_position": <number or null>
}

Cleanup rules for "title":
- Apply proper Title Case. Capitalize the first letter of every significant word. Keep small words ("the", "of", "and", "a", "an", "to", "in", "on", "or", "for", "with", "but", "as", "by", "at", "from") lowercase EXCEPT when they are the first or last word of the title or subtitle.
- Examples: "the power broker: robert moses and the fall of new york" → "The Power Broker: Robert Moses and the Fall of New York". "DUNE" → "Dune". "the way of kings" → "The Way of Kings".
- Preserve intentionally stylized words that contain INTERNAL uppercase letters (e.g. "iCon", "iPhone", "MacBook", "eBay") — leave those words exactly as written.
- Strip trailing parentheticals matching: series info like "(Series Name, #N)", edition markers like "(Deluxe Edition)", "(Hardcover)", "(Paperback)", "(Audiobook)", "(Unabridged)", year markers like "(1963)".
- Strip series-marker suffixes after a colon like ": Book Two of the Expanse", ": Volume 1", ": A Novel". DO NOT strip substantive descriptive subtitles like "Robert Moses and the Fall of New York" or "Steve Jobs, The Greatest Second Act in the History of Business" — those ARE part of the work identity and must be preserved.
- Trim and collapse internal whitespace.

Cleanup rules for "author_name":
- Normalize "Last, First" → "First Last".
- Fix all-caps or all-lowercase author names to proper Name Case.
- Preserve initials and punctuation (e.g. "Robert A. Caro", "J.R.R. Tolkien").

Extract "series_name" and "series_position" ONLY when explicitly present in the raw title (e.g. inside parens like "(The Expanse, #2)"). Use null when not present.

Output ONLY the JSON object. No markdown code fences. No commentary."#;

/// Polished add-time output. `title` and `author_name` are the locked
/// identity anchor values. `series_name` and `series_position` are extracted
/// when present in the input — usable to populate the Work record at add-time.
#[derive(Debug, Clone)]
pub struct PolishedAddTime {
    pub title: String,
    pub author_name: String,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct LlmPolishResponse {
    title: String,
    author_name: String,
    #[serde(default)]
    series_name: Option<String>,
    #[serde(default)]
    series_position: Option<f64>,
}

/// Polish a raw title + author at add-time using the LLM if configured.
///
/// On any failure (LLM not configured, timeout, HTTP error, malformed
/// response), falls back to the deterministic `clean_title` / `clean_author`
/// pair. Per project Principle 11, LLM is value-add and never gatekeeps.
///
/// `llm_config` is `(endpoint, api_key, model)`. Pass None to skip the LLM
/// attempt entirely (purely deterministic path). The endpoint is the
/// OpenAI-compat base URL (e.g. `https://api.groq.com/openai/v1`,
/// `https://generativelanguage.googleapis.com/v1beta/openai`).
pub async fn polish_addtime(
    http: &HttpClient,
    llm_config: Option<(&str, &str, &str)>,
    raw_title: &str,
    raw_author: &str,
) -> PolishedAddTime {
    let deterministic = || PolishedAddTime {
        title: clean_title(raw_title),
        author_name: clean_author(raw_author),
        series_name: None,
        series_position: None,
    };

    let Some((endpoint, api_key, model)) = llm_config else {
        return deterministic();
    };

    match tokio::time::timeout(
        LLM_POLISH_TIMEOUT,
        call_polish_llm(http, endpoint, api_key, model, raw_title, raw_author),
    )
    .await
    {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => {
            tracing::info!(
                raw_title,
                raw_author,
                "LLM add-time polish failed, falling back to deterministic: {e}"
            );
            deterministic()
        }
        Err(_) => {
            tracing::info!(
                raw_title,
                raw_author,
                "LLM add-time polish timed out (>2.5s), falling back to deterministic"
            );
            deterministic()
        }
    }
}

async fn call_polish_llm(
    http: &HttpClient,
    endpoint: &str,
    api_key: &str,
    model: &str,
    raw_title: &str,
    raw_author: &str,
) -> Result<PolishedAddTime, String> {
    let url = format!("{}/chat/completions", endpoint.trim_end_matches('/'));
    let user_msg = format!("title_raw: {raw_title}\nauthor_raw: {raw_author}");
    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": POLISH_SYSTEM_INSTRUCTION},
            {"role": "user",   "content": user_msg},
        ],
        "temperature": 0.0,
        "response_format": {"type": "json_object"},
    });

    let resp = http
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("send: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("status {}", resp.status()));
    }
    let envelope: serde_json::Value = resp.json().await.map_err(|e| format!("envelope: {e}"))?;
    let content_raw = envelope
        .pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .ok_or("missing choices[0].message.content")?;
    // Tolerate code-fence wrapping that some providers add.
    let trimmed = content_raw.trim();
    let unfenced = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .unwrap_or(trimmed);
    let unfenced = unfenced.strip_suffix("```").unwrap_or(unfenced).trim();
    let parsed: LlmPolishResponse =
        serde_json::from_str(unfenced).map_err(|e| format!("inner parse: {e}"))?;
    Ok(PolishedAddTime {
        title: parsed.title,
        author_name: parsed.author_name,
        series_name: parsed.series_name,
        series_position: parsed.series_position,
    })
}

fn collapse_whitespace(s: &str) -> String {
    RE_WHITESPACE.replace_all(s.trim(), " ").to_string()
}

/// Apply per-word title-case. Words with INTERNAL uppercase letters
/// (e.g. "iCon", "iPhone", "MacBook", "FBI", "X-Men") are preserved as-is
/// so stylized forms survive. Other words are lower-cased then capitalized
/// (with small-word lowercase rule for non-edge positions).
fn fix_casing_if_needed(s: &str) -> String {
    title_case(s)
}

pub fn title_case(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace().collect();
    let last_idx = words.len().saturating_sub(1);
    words
        .iter()
        .enumerate()
        .map(|(i, w)| title_case_word(w, i == 0 || i == last_idx))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Casing rule for a single word:
///   - Stylized words (have at least one lowercase AND at least one
///     non-leading uppercase letter) are preserved verbatim — covers
///     "iCon", "iPhone", "MacBook", "McDonald", "X-Men".
///   - Everything else (all-lowercase, all-uppercase, or first-cap-only) is
///     lowercased then re-capitalized via small-word rules. This normalizes
///     "york" → "York", "DUNE" → "Dune", "POWER" → "Power".
///   - All-uppercase acronyms ("FBI") get normalized too — acceptable
///     trade-off; user can manually edit if they meant the acronym.
fn title_case_word(word: &str, edge: bool) -> String {
    let chars: Vec<char> = word.chars().collect();
    let has_lower = chars.iter().any(|c| c.is_lowercase());
    let has_internal_upper = chars
        .iter()
        .skip(1)
        .any(|c| c.is_alphabetic() && c.is_uppercase());
    if has_lower && has_internal_upper {
        return word.to_string();
    }
    // Tokens with periods are dotted initials ("J.R.R.", "U.S.A."),
    // honorifics ("Mr.", "Dr.", "Jr."), or abbreviations — preserve.
    if word.contains('.') {
        return word.to_string();
    }
    let lower = word.to_lowercase();
    if !edge && SMALL_WORDS.contains(&lower.as_str()) {
        return lower;
    }
    capitalize_first(&lower)
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
    // Only fire on exactly one comma — multi-comma forms are ambiguous.
    if s.matches(',').count() != 1 {
        return s.to_string();
    }
    let (last, first_etc) = s.split_once(',').unwrap();
    let last = last.trim();
    let first_etc = first_etc.trim();
    if last.is_empty() || first_etc.is_empty() {
        return s.to_string();
    }
    // Don't re-arrange if `first_etc` looks like a name suffix.
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
    fn author_per_word_casing_normalizes_lowercase_words() {
        // Per-word casing fixes lowercase-word-by-lowercase-word; only words
        // with internal uppercase (like "iPad" or "MacBook") are preserved.
        assert_eq!(clean_author("danah boyd"), "Danah Boyd");
        assert_eq!(clean_author("danah Boyd"), "Danah Boyd");
    }

    #[test]
    fn title_mixed_case_with_lowercase_words_normalized() {
        // The motivating bug: titles where some words are uppercase and
        // some are lowercase used to be preserved as-is. Per-word casing
        // now fixes the lowercase ones.
        assert_eq!(
            clean_title("The power broker: Robert Moses and the fall of New York"),
            "The Power Broker: Robert Moses and the Fall of New York"
        );
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
