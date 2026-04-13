//! M3 — String parsing via regex cascade (Readarr-inspired + extensions).

use once_cell::sync::Lazy;
use regex::Regex;

use super::types::{Confidence, Extraction, ExtractionSource};

/// Side-channel metadata extracted during cleaning.
#[derive(Debug, Default)]
pub struct SideChannel {
    pub year: Option<i32>,
    pub format: Option<String>,
    pub narrator: Option<String>,
    pub unabridged: Option<bool>,
    pub language: Option<String>,
}

/// Parse a release title / filename / torrent name into extraction(s).
/// Returns one extraction normally, or two if pattern 16 (ambiguous Title-Author) matches.
pub fn parse_string(input: &str) -> (Vec<Extraction>, SideChannel) {
    let (cleaned, side) = clean_input(input);
    if cleaned.trim().is_empty() {
        return (vec![], side);
    }

    for (i, pattern) in PATTERNS.iter().enumerate() {
        if let Some(cap) = pattern.regex.captures(&cleaned) {
            let author = cap.name("author").map(|m| m.as_str().trim().to_string());
            let book = cap.name("book").map(|m| m.as_str().trim().to_string());
            let year = cap
                .name("releaseyear")
                .and_then(|m| m.as_str().parse::<i32>().ok())
                .or(side.year);

            if i == 21 {
                // Pattern 22 (index 21): Title - Author is ambiguous.
                // Produce two hypotheses.
                let h1 = make_extraction(book.as_deref(), author.as_deref(), year);
                let h2 = make_extraction(author.as_deref(), book.as_deref(), year);
                return (vec![h1, h2], side);
            }

            if let Some(title) = book {
                if !title.is_empty() {
                    return (
                        vec![Extraction {
                            title: Some(title),
                            author,
                            year,
                            isbn: None,
                            language: side.language.clone(),
                            series: None,
                            series_position: None,
                            narrator: side.narrator.clone(),
                            asin: None,
                            confidence: Confidence::Medium,
                            source: ExtractionSource::String,
                        }],
                        side,
                    );
                }
            }
        }
    }

    // No regex matched — return cleaned string as title-only.
    (
        vec![Extraction {
            title: Some(cleaned.trim().to_string()),
            author: None,
            year: side.year,
            isbn: None,
            language: side.language.clone(),
            series: None,
            series_position: None,
            narrator: side.narrator.clone(),
            asin: None,
            confidence: Confidence::MediumLow,
            source: ExtractionSource::String,
        }],
        side,
    )
}

fn make_extraction(title: Option<&str>, author: Option<&str>, year: Option<i32>) -> Extraction {
    Extraction {
        title: title.map(|s| s.to_string()),
        author: author.map(|s| s.to_string()),
        year,
        isbn: None,
        language: None,
        series: None,
        series_position: None,
        narrator: None,
        asin: None,
        confidence: Confidence::Medium,
        source: ExtractionSource::String,
    }
}

// ---------------------------------------------------------------------------
// Input cleaning
// ---------------------------------------------------------------------------

static FILE_EXT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\.(epub|m4b|m4a|mp3|flac|ogg|wma|pdf|azw3?|mobi|cbz|cbr|nzb|torrent|zip|rar|7z)$",
    )
    .unwrap()
});
static WEBSITE_PREFIX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)^(?:\[\s*)?(?:www\.)?[-a-z0-9]{1,256}\.(?:[a-z]{2,6}\.[a-z]{2,6}|[a-z]{2,})\b(?:\s*\]|[ \-]{2,})[ \-]*").unwrap()
});
static WEBSITE_POSTFIX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\s*[-\s]*(?:www\.)?[-a-z0-9]{1,256}\.(?:com|net|org|info|me)\s*$").unwrap()
});
static GROUP_SUFFIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"-[A-Za-z0-9]{2,15}$").unwrap());
static QUALITY_TAG: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)[\[\(]\s*(epub|mobi|azw3?|pdf|flac|mp3|m4[ab]|ogg|wma|320kbps|192kbps|vbr|cbr|cbz)\s*[\]\)]").unwrap()
});
static YEAR_EXTRACT: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\(\[]?(\d{4})[\)\]]?").unwrap());
static NARRATOR_EXTRACT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(?:\((?:narrated|read) by ([^)]+)\)|\{([^}]+)\})").unwrap());
static ABRIDGED_EXTRACT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(unabridged|abridged)\b").unwrap());
static LANG_TAG: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\[(english|french|german|spanish|italian|portuguese|russian|chinese|japanese|korean|polish|dutch|swedish|czech|arabic|hebrew|hindi|turkish)\]").unwrap()
});

fn clean_input(input: &str) -> (String, SideChannel) {
    let mut s = input.to_string();
    let mut side = SideChannel::default();

    // Extract side-channel metadata before stripping.
    if let Some(cap) = NARRATOR_EXTRACT.captures(&s) {
        side.narrator = cap
            .get(1)
            .or_else(|| cap.get(2))
            .map(|m| m.as_str().to_string());
    }
    if let Some(cap) = ABRIDGED_EXTRACT.captures(&s) {
        side.unabridged = Some(cap[1].eq_ignore_ascii_case("unabridged"));
    }
    if let Some(cap) = LANG_TAG.captures(&s) {
        side.language = Some(cap[1].to_string());
    }
    // Extract year from parenthesized or bracketed form.
    if let Some(cap) = YEAR_EXTRACT.captures(&s) {
        let y: i32 = cap[1].parse().unwrap_or(0);
        if (1800..=2030).contains(&y) {
            side.year = Some(y);
        }
    }
    // Extract format.
    if let Some(cap) = QUALITY_TAG.captures(&s) {
        side.format = Some(cap[1].to_uppercase());
    }

    // Now strip.
    s = FILE_EXT.replace(&s, "").to_string();
    s = WEBSITE_PREFIX.replace(&s, "").to_string();
    s = WEBSITE_POSTFIX.replace(&s, "").to_string();
    s = QUALITY_TAG.replace_all(&s, "").to_string();
    s = NARRATOR_EXTRACT.replace_all(&s, "").to_string();
    s = ABRIDGED_EXTRACT.replace_all(&s, "").to_string();
    s = LANG_TAG.replace_all(&s, "").to_string();

    // Replace underscores and dots with spaces (but not within 4-digit years).
    s = s.replace('_', " ");
    // Dots: replace with spaces, then fix any digit-space-digit sequences back.
    // (Preserve "2015.01" style patterns.)
    static DIGIT_SPACE_DIGIT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(\d) (\d)").unwrap());
    static MULTI_SPACE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s{2,}").unwrap());

    s = s.replace('.', " ");
    s = DIGIT_SPACE_DIGIT.replace_all(&s, "$1.$2").to_string();

    // Strip group suffix (but only if it looks like a scene tag, not part of a name).
    if s.contains(" - ") || s.contains(" – ") {
        s = GROUP_SUFFIX.replace(&s, "").to_string();
    }

    // Collapse whitespace.
    s = MULTI_SPACE.replace_all(&s, " ").to_string();

    (s.trim().to_string(), side)
}

// ---------------------------------------------------------------------------
// Regex cascade — 22 patterns
// ---------------------------------------------------------------------------

struct Pattern {
    regex: Regex,
}

static PATTERNS: Lazy<Vec<Pattern>> = Lazy::new(|| {
    vec![
        // 1. Book by Author [...] (MyAnonamouse)
        Pattern { regex: Regex::new(r"^(?P<book>.+)\bby\b(?P<author>.+?)(?:\[|\()").unwrap() },

        // 2. (Genre) [Source] Author - Book - Year (ruTracker)
        Pattern { regex: Regex::new(r"^(?:\(.+?\))(?:\W*(?:\[.+?\]))?\W*(?P<author>.+?)(?: - )(?P<book>.+?)(?: - )(?P<releaseyear>\d{4})").unwrap() },

        // 3. Author - Book - Version - Source - Year (scene)
        Pattern { regex: Regex::new(r"^(?P<author>.+?)[-](?P<book>.+?)[-](?:[\(\[]?)(?:.+?(?:Edition)?)(?:[\)\]]?)[-](?:\d?CD|WEB).+?(?P<releaseyear>\d{4})").unwrap() },

        // 4. Author - Book - Source - Year
        Pattern { regex: Regex::new(r"^(?P<author>.+?)[-](?P<book>.+?)[-](?:\d?CD|WEB).+?(?P<releaseyear>\d{4})").unwrap() },

        // 5. Author - Book (Year) strict
        Pattern { regex: Regex::new(r"^(?:(?P<author>.+?)(?: - )+)(?P<book>.+?)\W*(?:\(|\[).+?(?P<releaseyear>\d{4})").unwrap() },

        // 6. Author - Book (Year) loose
        Pattern { regex: Regex::new(r"^(?:(?P<author>.+?)(?: - )+)(?P<book>.+?)\W*(?:\(|\[)(?P<releaseyear>\d{4})").unwrap() },

        // 7. Author - Book - Year [something]
        Pattern { regex: Regex::new(r"^(?:(?P<author>.+?)(?: - )+)(?P<book>.+?)\W*(?: - )(?P<releaseyear>\d{4})\W*(?:\(|\[)").unwrap() },

        // 8. Author - Book [something] or Author - Book (something)
        Pattern { regex: Regex::new(r"^(?:(?P<author>.+?)(?: - )+)(?P<book>.+?)\W*(?:\(|\[)").unwrap() },

        // 9. Author - Book Year
        Pattern { regex: Regex::new(r"^(?:(?P<author>.+?)(?: - )+)(?P<book>.+?)\W*(?P<releaseyear>\d{4})").unwrap() },

        // 10. Author-Book (Year) strict (hyphen no space)
        Pattern { regex: Regex::new(r"^(?:(?P<author>.+?)(?:-)+)(?P<book>.+?)\W*(?:\(|\[).+?(?P<releaseyear>\d{4})").unwrap() },

        // 11. Author-Book (Year) loose (hyphen no space)
        Pattern { regex: Regex::new(r"^(?:(?P<author>.+?)(?:-)+)(?P<book>.+?)\W*(?:\(|\[)(?P<releaseyear>\d{4})").unwrap() },

        // 12. Author-Book [something] (hyphen no space)
        Pattern { regex: Regex::new(r"^(?:(?P<author>.+?)(?:-)+)(?P<book>.+?)\W*(?:\(|\[)").unwrap() },

        // 13. Author-Book-something-Year
        Pattern { regex: Regex::new(r"^(?:(?P<author>.+?)(?:-)+)(?P<book>.+?)(?:-.+?)(?P<releaseyear>\d{4})").unwrap() },

        // 14. Author-Book Year (hyphen no space)
        Pattern { regex: Regex::new(r"^(?:(?P<author>.+?)(?:-)+)(?:(?P<book>.+?)(?:-)+)(?P<releaseyear>\d{4})").unwrap() },

        // 15. Author - Year - Book
        Pattern { regex: Regex::new(r"^(?:(?P<author>.+?)(?:-))(?P<releaseyear>\d{4})(?:-)(?P<book>[^-]+)").unwrap() },

        // 16. Author - Series # - Title
        Pattern { regex: Regex::new(r"^(?P<author>.+?)\s+-\s+.+?\s+\d+\s+-\s+(?P<book>.+)$").unwrap() },

        // 17. Author - [Series ##] - Title
        Pattern { regex: Regex::new(r"^(?P<author>.+?)\s+-\s+\[.+?\s*\d+\]\s+-\s+(?P<book>.+)$").unwrap() },

        // 18. Author - Title {Narrator}
        Pattern { regex: Regex::new(r"^(?P<author>.+?)\s+-\s+(?P<book>.+?)\s+\{.+?\}$").unwrap() },

        // 19. Author - Title (Narrated by ...)
        Pattern { regex: Regex::new(r"^(?P<author>.+?)\s+-\s+(?P<book>.+?)\s+\((?:Narrated|Read) by .+?\)$").unwrap() },

        // 20. Title (Author)
        Pattern { regex: Regex::new(r"^(?P<book>.+?)\s+\((?P<author>[^)]+)\)\s*$").unwrap() },

        // 21. [Author] Title (Asian LN / private trackers)
        Pattern { regex: Regex::new(r"^\[(?P<author>[^\]]+)\]\s*(?P<book>.+)$").unwrap() },

        // 22. Title - Author (reversed) — AMBIGUOUS: produces two hypotheses
        // Placed last among hyphen patterns so more specific patterns get priority.
        Pattern { regex: Regex::new(r"^(?P<book>.+?)\s+-\s+(?P<author>.+)$").unwrap() },
    ]
});
