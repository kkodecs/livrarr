//! Identity-grade matching authority: the single vocabulary for deciding
//! whether two records describe the same book.
//!
//! Pure functions only — no I/O, no async, no database access. Callers parse
//! each title once into a [`ParsedTitle`], then combine the per-dimension
//! verdicts ([`TitleVerdict`], [`AuthorVerdict`], [`LanguageVerdict`],
//! [`IdVerdict`]) at their own policy seat.
//!
//! Title model: parse, don't truncate. A title splits into a main title and a
//! tail (subtitle / series marker / edition junk). Only main titles can make
//! two records match. Tails can veto (conflicting volume numbers), demote to
//! grey (substantive disagreement or a tail on one side only), or be ignored
//! (edition junk). A tail that cannot be confidently classified is treated as
//! a true subtitle — the safest class: it can only demote, never veto, never
//! silently pass.

use std::sync::LazyLock;

use regex::Regex;

use crate::text_norm;
use crate::title_cleanup::{self, collapse_whitespace, normalize_last_first};

/// Token-set Jaccard floor below which two main titles are considered
/// different rather than grey candidates.
pub const TITLE_GREY_FLOOR: f64 = 0.75;

/// A series/volume marker extracted from a title tail or a trailing
/// parenthetical, e.g. "Book 3", "Vol. IV", "(The Dresden Files, #1)".
///
/// The marker's text never participates in title comparison; only the
/// extracted number carries signal.
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesMarker {
    /// Series name text accompanying the marker, when present
    /// ("The Dresden Files" in "The Dresden Files, Book 1"). Canonical
    /// lowercase form; never compared as title text.
    pub series: Option<String>,
    /// The extracted volume/position number.
    pub number: f64,
}

/// A title decomposed for identity comparison. All fields hold canonical
/// matching forms (lowercased, accent-stripped, punctuation-folded), not
/// display text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParsedTitle {
    /// Canonical main title — the only part that can make a match.
    pub main: String,
    /// True subtitle: substantive tail text that is neither a series marker
    /// nor recognized edition junk. Can only demote a pair to grey.
    pub subtitle: Option<String>,
    /// Series/volume markers extracted from the tail and trailing
    /// parentheticals. Conflicting numbers veto a match.
    pub series_markers: Vec<SeriesMarker>,
    /// Recognized edition-junk phrases ("a novel", "unabridged", …).
    /// Ignored for identity.
    pub junk: Vec<String>,
}

impl ParsedTitle {
    /// Volume numbers carried by this title's series markers.
    pub fn volume_numbers(&self) -> Vec<f64> {
        self.series_markers.iter().map(|m| m.number).collect()
    }
}

/// Identity-grade comparison of two parsed titles.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TitleVerdict {
    /// Cleaned main titles are exactly equal and no tail evidence conflicts.
    Same,
    /// Close but not certain: near-equal mains (token-set Jaccard at or above
    /// [`TITLE_GREY_FLOOR`]), or exact mains demoted by tail evidence
    /// (one-sided tail, disagreeing subtitles, one-sided volume info).
    /// `score` is the computed main-title token-set Jaccard.
    Grey { score: f64 },
    /// Below the grey floor, or no usable title on either side.
    Different,
    /// Conflicting volume numbers (from parsed tails or caller-supplied
    /// series positions): a hard stop regardless of title similarity.
    VetoVolume,
}

/// Identity-grade comparison of two credited-author lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorVerdict {
    /// At least one unambiguous full-name match. Extra credited names on
    /// either side are non-evidence and never subtract.
    Agree,
    /// Shared surname without a full-name match, or initials compatible with
    /// more than one candidate name.
    Grey,
    /// No overlap of any kind.
    Disagree,
    /// One or both sides carry no usable author names.
    Abstain,
}

/// Language compatibility between a work and an incoming payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanguageVerdict {
    /// Both sides declare a language and they differ: never merge.
    Veto,
    /// One side is silent and the declared side differs from the user's
    /// default language: eligible for review, never auto-apply.
    Grey,
    /// Declared and equal, silent-but-default, or both silent.
    Neutral,
}

/// Identifier evidence for one record. Work-level keys identify the work;
/// edition-level ids (ISBN/ASIN) identify a printing of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IdEvidence<'a> {
    pub ol_key: Option<&'a str>,
    pub gr_key: Option<&'a str>,
    pub hc_key: Option<&'a str>,
    pub isbn_13: Option<&'a str>,
    pub asin: Option<&'a str>,
}

/// Identifier-level comparison of two records.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdVerdict {
    /// A same-provider work key matches: the strongest positive evidence.
    WorkKeyEqual,
    /// A same-provider work key differs on the two sides: a hard veto that
    /// outranks any edition-id agreement (a shared ISBN with contradicting
    /// work keys is an ISBN collision, never a merge).
    WorkKeyContradiction,
    /// No work-key evidence either way, but a shared ISBN or ASIN bridges
    /// the pair as editions of one work.
    EditionBridge,
    /// No identifier evidence. Edition-id inequality lands here too: two
    /// editions of one work legitimately carry different ISBNs/ASINs, so
    /// inequality is no evidence and never vetoes.
    NoEvidence,
}

/// Decompose a raw title into its canonical matching parts.
pub fn parse_title(raw: &str) -> ParsedTitle {
    let mut markers: Vec<SeriesMarker> = Vec::new();
    let mut junk: Vec<String> = Vec::new();
    let mut subtitle_parts: Vec<String> = Vec::new();

    let mut rest = collapse_whitespace(raw);

    // Peel trailing parentheticals right-to-left, classifying each.
    loop {
        let found = title_cleanup::RE_TRAILING_PAREN
            .captures(&rest)
            .map(|caps| {
                (
                    caps.get(1)
                        .map(|m| m.as_str().trim().to_string())
                        .unwrap_or_default(),
                    caps.get(0).map(|m| m.start()).unwrap_or(0),
                )
            });
        let Some((inner, start)) = found else { break };
        classify_paren(&inner, &mut markers, &mut junk, &mut subtitle_parts);
        rest = rest[..start].trim_end().to_string();
        if rest.is_empty() {
            break;
        }
    }

    // Split main from tail at the first unambiguous separator.
    let (main_part, tail_part) = split_main_tail(&rest);
    let mut main_raw = main_part.to_string();
    let tail_owned = tail_part.map(str::to_string);

    // A trailing comma-led volume marker binds to the main ("Foo, Vol. 3").
    if let Some((prefix, marker)) = extract_comma_marker(&main_raw) {
        main_raw = prefix;
        markers.push(marker);
    }

    if let Some(tail) = tail_owned {
        classify_tail(&tail, &mut markers, &mut junk, &mut subtitle_parts);
    }

    let subtitle = {
        let joined = subtitle_parts.join(" ");
        let canonical = canonical_phrase(&joined);
        (!canonical.is_empty()).then_some(canonical)
    };

    ParsedTitle {
        main: canonical_phrase(&main_raw),
        subtitle,
        series_markers: markers,
        junk,
    }
}

/// Compare two parsed titles at identity grade, using only volume evidence
/// carried by the titles themselves.
pub fn title_verdict(a: &ParsedTitle, b: &ParsedTitle) -> TitleVerdict {
    title_verdict_with_positions(a, None, b, None)
}

/// Compare two parsed titles at identity grade, folding in caller-supplied
/// series positions (e.g. from series metadata) as additional volume
/// evidence.
pub fn title_verdict_with_positions(
    a: &ParsedTitle,
    a_position: Option<f64>,
    b: &ParsedTitle,
    b_position: Option<f64>,
) -> TitleVerdict {
    let mut volumes_a = a.volume_numbers();
    volumes_a.extend(a_position);
    let mut volumes_b = b.volume_numbers();
    volumes_b.extend(b_position);

    if volumes_conflict(&volumes_a, &volumes_b) {
        return TitleVerdict::VetoVolume;
    }
    if a.main.is_empty() || b.main.is_empty() {
        return TitleVerdict::Different;
    }

    let score = text_norm::jaccard(
        &text_norm::title_tokens(&a.main),
        &text_norm::title_tokens(&b.main),
    );

    if a.main == b.main {
        let one_sided_volume = volumes_a.is_empty() != volumes_b.is_empty();
        let subtitles_demote = match (&a.subtitle, &b.subtitle) {
            (Some(x), Some(y)) => x != y,
            (None, None) => false,
            _ => true,
        };
        if one_sided_volume || subtitles_demote {
            return TitleVerdict::Grey { score };
        }
        return TitleVerdict::Same;
    }

    if score >= TITLE_GREY_FLOOR {
        return TitleVerdict::Grey { score };
    }
    TitleVerdict::Different
}

/// Compare two credited-author lists at identity grade.
pub fn author_verdict(a: &[String], b: &[String]) -> AuthorVerdict {
    let ca: Vec<CanonicalName> = a.iter().filter_map(|n| canonical_author_name(n)).collect();
    let cb: Vec<CanonicalName> = b.iter().filter_map(|n| canonical_author_name(n)).collect();
    if ca.is_empty() || cb.is_empty() {
        return AuthorVerdict::Abstain;
    }

    // A full-name match pair counts only when it is unambiguous on both
    // sides; a name compatible with several candidates is grey evidence.
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut row_counts = vec![0usize; ca.len()];
    let mut col_counts = vec![0usize; cb.len()];
    for (i, x) in ca.iter().enumerate() {
        for (j, y) in cb.iter().enumerate() {
            if full_name_match(x, y) {
                row_counts[i] += 1;
                col_counts[j] += 1;
                pairs.push((i, j));
            }
        }
    }
    if pairs
        .iter()
        .any(|&(i, j)| row_counts[i] == 1 && col_counts[j] == 1)
    {
        return AuthorVerdict::Agree;
    }
    if !pairs.is_empty() {
        return AuthorVerdict::Grey;
    }
    let shared_surname = ca.iter().any(|x| cb.iter().any(|y| x.surname == y.surname));
    if shared_surname {
        AuthorVerdict::Grey
    } else {
        AuthorVerdict::Disagree
    }
}

/// Compare a work's language against an incoming payload's declared
/// language, in the context of the user's default language.
pub fn language_verdict(
    work: Option<&str>,
    payload: Option<&str>,
    user_default: &str,
) -> LanguageVerdict {
    let norm = |v: Option<&str>| v.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let default = user_default.trim().to_lowercase();
    match (norm(work), norm(payload)) {
        (Some(w), Some(p)) if w != p => LanguageVerdict::Veto,
        (Some(_), Some(_)) | (None, None) => LanguageVerdict::Neutral,
        (Some(declared), None) | (None, Some(declared)) => {
            if declared == default {
                LanguageVerdict::Neutral
            } else {
                LanguageVerdict::Grey
            }
        }
    }
}

/// Compare the identifier evidence of two records.
pub fn id_verdict(a: &IdEvidence, b: &IdEvidence) -> IdVerdict {
    fn present(v: Option<&str>) -> Option<&str> {
        v.map(str::trim).filter(|s| !s.is_empty())
    }
    fn eq(x: Option<&str>, y: Option<&str>) -> bool {
        matches!((present(x), present(y)), (Some(p), Some(q)) if p == q)
    }
    fn differs(x: Option<&str>, y: Option<&str>) -> bool {
        matches!((present(x), present(y)), (Some(p), Some(q)) if p != q)
    }

    if eq(a.ol_key, b.ol_key) || eq(a.gr_key, b.gr_key) || eq(a.hc_key, b.hc_key) {
        return IdVerdict::WorkKeyEqual;
    }
    if differs(a.ol_key, b.ol_key) || differs(a.gr_key, b.gr_key) || differs(a.hc_key, b.hc_key) {
        return IdVerdict::WorkKeyContradiction;
    }
    if eq(a.isbn_13, b.isbn_13) || eq(a.asin, b.asin) {
        return IdVerdict::EditionBridge;
    }
    IdVerdict::NoEvidence
}

// --- title parsing internals ---

/// Volume-marker phrase: optional "Series Name, " prefix, a volume token,
/// a number token, and an optional "of/in the Series" suffix. Anchored to
/// the whole string.
static RE_SERIES_VOLUME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(?:(.+?),\s*)?(?:book|volume|vol\.?|part|no\.|#)\s*([a-z0-9.]+)(?:\s+(?:of|in)\s+(?:the\s+)?(.+?))?$",
    )
    .unwrap()
});

/// Hash-number series form without a comma: "Series Name #4" or "#4".
static RE_HASH_SERIES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:(.+?)\s+)?#\s*([0-9.]+)$").unwrap());

/// A bare number standing alone as a tail.
static RE_BARE_NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+(\.\d+)?$").unwrap());

/// Trailing comma-led volume marker bound to a main title ("Foo, Vol. 3").
static RE_COMMA_VOLUME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i),\s*(?:book|volume|vol\.?|part|no\.|#)\s*([a-z0-9.]+)\s*$").unwrap()
});

/// Edition-junk phrases matched whole against a normalized tail. Extending
/// this list requires a matching trap-corpus test.
const JUNK_VOCAB: &[&str] = &[
    "a novel",
    "a memoir",
    "a novella",
    "a story",
    "a tale",
    "a poem",
    "unabridged",
    "abridged",
    "large print",
    "annotated",
    "illustrated",
    "complete",
    "tie in",
    "special edition",
    "deluxe edition",
    "collectors edition",
    "anniversary edition",
    "expanded edition",
    "audiobook",
    "ebook",
    "kindle edition",
    "hardcover",
    "paperback",
    "mass market",
    "original edition",
    "reissue edition",
    "revised edition",
    "updated edition",
    "definitive edition",
    "directors cut edition",
];

fn is_junk_phrase(normalized: &str) -> bool {
    JUNK_VOCAB.contains(&normalized)
}

/// Lowercased, accent-stripped, apostrophe-dropped, punctuation-folded form
/// used for junk-vocabulary comparison.
fn normalize_vocab(s: &str) -> String {
    let stripped = text_norm::strip_combining_marks(s);
    let lower = stripped.to_lowercase();
    let no_apostrophe: String = lower.chars().filter(|c| !matches!(c, '\'' | '’')).collect();
    no_apostrophe
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Canonical comparison form: accent-stripped, lowercased, punctuation
/// folded to token boundaries, leading article dropped. CJK text folds to
/// its bare character sequence instead (bigram comparison happens at
/// scoring time).
fn canonical_phrase(s: &str) -> String {
    if text_norm::has_cjk(s) {
        return s
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
            .to_lowercase();
    }
    let stripped = text_norm::strip_combining_marks(s);
    let lower = stripped.to_lowercase();
    let mut tokens: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.len() > 1 && matches!(tokens[0], "a" | "an" | "the") {
        tokens.remove(0);
    }
    tokens.join(" ")
}

/// Split at the first unambiguous separator: a colon, a spaced dash, or an
/// em-dash. A hyphen inside a word never splits.
fn split_main_tail(s: &str) -> (&str, Option<&str>) {
    let candidates = [
        s.find(':').map(|i| (i, ':'.len_utf8())),
        s.find(" - ").map(|i| (i, " - ".len())),
        s.find('—').map(|i| (i, '—'.len_utf8())),
    ];
    let best = candidates.into_iter().flatten().min_by_key(|&(i, _)| i);
    match best {
        Some((i, len)) => {
            let main = s[..i].trim_end();
            let tail = s[i + len..].trim_start();
            (main, (!tail.is_empty()).then_some(tail))
        }
        None => (s, None),
    }
}

/// Parse a volume number token: digits (with an optional decimal part),
/// a spelled cardinal (one..twenty), or a roman numeral (i..xx — the forms
/// the existing series-marker stripper recognizes).
fn parse_volume_number(token: &str) -> Option<f64> {
    let t = token.trim().trim_end_matches('.');
    if t.is_empty() {
        return None;
    }
    if t.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return t.parse::<f64>().ok();
    }
    let lower = t.to_lowercase();
    const SPELLED: [&str; 20] = [
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    if let Some(i) = SPELLED.iter().position(|w| *w == lower) {
        return Some((i + 1) as f64);
    }
    const ROMAN: [&str; 20] = [
        "i", "ii", "iii", "iv", "v", "vi", "vii", "viii", "ix", "x", "xi", "xii", "xiii", "xiv",
        "xv", "xvi", "xvii", "xviii", "xix", "xx",
    ];
    ROMAN
        .iter()
        .position(|w| *w == lower)
        .map(|i| (i + 1) as f64)
}

/// Recognize a whole string as a series-volume marker and extract its
/// number. Bare numbers qualify only below four integer digits (a four
/// digit standalone number reads as a year, not a volume).
fn parse_series_marker(text: &str) -> Option<SeriesMarker> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if RE_BARE_NUMBER.is_match(t) {
        let int_digits = t.split('.').next().unwrap_or("").len();
        if int_digits <= 3 {
            return t.parse::<f64>().ok().map(|number| SeriesMarker {
                series: None,
                number,
            });
        }
        return None;
    }
    if let Some(caps) = RE_SERIES_VOLUME.captures(t) {
        if let Some(number) = caps.get(2).and_then(|m| parse_volume_number(m.as_str())) {
            let series = caps
                .get(1)
                .or(caps.get(3))
                .map(|m| canonical_phrase(m.as_str()))
                .filter(|s| !s.is_empty());
            return Some(SeriesMarker { series, number });
        }
    }
    if let Some(caps) = RE_HASH_SERIES.captures(t) {
        if let Some(number) = caps.get(2).and_then(|m| parse_volume_number(m.as_str())) {
            let series = caps
                .get(1)
                .map(|m| canonical_phrase(m.as_str()))
                .filter(|s| !s.is_empty());
            return Some(SeriesMarker { series, number });
        }
    }
    None
}

/// Pull a trailing comma-led volume marker off a main title.
fn extract_comma_marker(s: &str) -> Option<(String, SeriesMarker)> {
    let caps = RE_COMMA_VOLUME.captures(s)?;
    let number = parse_volume_number(caps.get(1)?.as_str())?;
    let start = caps.get(0)?.start();
    Some((
        s[..start].trim_end().to_string(),
        SeriesMarker {
            series: None,
            number,
        },
    ))
}

/// Classify a colon/dash tail: series marker, junk, or true subtitle.
/// Anything unrecognized is a true subtitle — the class that can only
/// demote, never veto, never silently pass.
fn classify_tail(
    tail: &str,
    markers: &mut Vec<SeriesMarker>,
    junk: &mut Vec<String>,
    subtitle_parts: &mut Vec<String>,
) {
    if let Some(marker) = parse_series_marker(tail) {
        markers.push(marker);
        return;
    }
    let normalized = normalize_vocab(tail);
    if !normalized.is_empty() && is_junk_phrase(&normalized) {
        junk.push(normalized);
        return;
    }
    subtitle_parts.push(tail.to_string());
}

/// Classify a trailing parenthetical: year and recognized series/format/
/// edition tags are junk (number-carrying series tags become markers);
/// anything unrecognized is a true subtitle.
fn classify_paren(
    inner: &str,
    markers: &mut Vec<SeriesMarker>,
    junk: &mut Vec<String>,
    subtitle_parts: &mut Vec<String>,
) {
    if inner.is_empty() {
        return;
    }
    let normalized = normalize_vocab(inner);
    if title_cleanup::RE_YEAR_PAREN.is_match(inner) {
        junk.push(normalized);
        return;
    }
    if let Some(marker) = parse_series_marker(inner) {
        markers.push(marker);
        return;
    }
    if title_cleanup::RE_SERIES_PAREN.is_match(inner)
        || title_cleanup::RE_FORMAT_PAREN.is_match(inner)
        || title_cleanup::RE_EDITION_PAREN.is_match(inner)
        || is_junk_phrase(&normalized)
    {
        junk.push(normalized);
        return;
    }
    subtitle_parts.push(inner.to_string());
}

// --- verdict internals ---

/// Both sides carry volume evidence and share no number.
fn volumes_conflict(a: &[f64], b: &[f64]) -> bool {
    !a.is_empty() && !b.is_empty() && !a.iter().any(|x| b.iter().any(|y| (x - y).abs() < 1e-9))
}

/// Parenthesized role tags in author credits ("(narrator)", "(translator)").
static RE_AUTHOR_PAREN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\([^)]*\)").unwrap());

/// One credited name in canonical comparison form: lowercased,
/// accent-stripped, order-normalized tokens with name suffixes dropped.
/// The last token is the surname; the rest are given-name tokens (a
/// single-character token is an initial).
struct CanonicalName {
    given: Vec<String>,
    surname: String,
}

fn canonical_author_name(raw: &str) -> Option<CanonicalName> {
    let no_parens = RE_AUTHOR_PAREN.replace_all(raw, " ");
    let reordered = normalize_last_first(&collapse_whitespace(&no_parens));
    let stripped = text_norm::strip_combining_marks(&reordered);
    let lower = stripped.to_lowercase();
    let mut tokens: Vec<String> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .filter(|t| !text_norm::AUTHOR_SUFFIX_STOPWORDS.contains(t))
        .map(String::from)
        .collect();
    let surname = tokens.pop()?;
    Some(CanonicalName {
        given: tokens,
        surname,
    })
}

/// An initial is compatible with any name sharing its first character;
/// full words must match exactly.
fn given_token_compatible(a: &str, b: &str) -> bool {
    let a_initial = a.chars().count() == 1;
    let b_initial = b.chars().count() == 1;
    if a_initial || b_initial {
        a.chars().next() == b.chars().next()
    } else {
        a == b
    }
}

/// Full-name match: equal surnames plus pairwise-compatible given names.
/// Surplus given tokens (an extra middle name on one side) do not block.
/// A surname-only name never fully matches a name that carries given
/// names — that is shared-surname (grey) territory.
fn full_name_match(a: &CanonicalName, b: &CanonicalName) -> bool {
    if a.surname != b.surname {
        return false;
    }
    match (a.given.is_empty(), b.given.is_empty()) {
        (true, true) => true,
        (true, false) | (false, true) => false,
        (false, false) => a
            .given
            .iter()
            .zip(b.given.iter())
            .all(|(x, y)| given_token_compatible(x, y)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- parse_title: splitting and tail classification ---

    #[test]
    fn hyphenated_word_never_splits() {
        let p = parse_title("Catch-22");
        assert_eq!(p.main, "catch 22");
        assert!(p.subtitle.is_none());
        assert!(p.series_markers.is_empty());
    }

    #[test]
    fn spaced_dash_and_em_dash_split_as_separators() {
        let spaced = parse_title("Foo - And Other Stories");
        assert_eq!(spaced.main, "foo");
        assert!(spaced.subtitle.is_some());

        let em = parse_title("Foo—And Other Stories");
        assert_eq!(em.main, "foo");
        assert!(em.subtitle.is_some());
    }

    #[test]
    fn colon_book_spelled_ordinal_is_series_marker() {
        let p = parse_title("Foo: Book Three");
        assert_eq!(p.main, "foo");
        assert!(p.subtitle.is_none());
        assert_eq!(p.volume_numbers(), vec![3.0]);
    }

    #[test]
    fn comma_vol_abbreviation_is_series_marker() {
        let p = parse_title("Foo, Vol. 3");
        assert_eq!(p.main, "foo");
        assert!(p.subtitle.is_none());
        assert_eq!(p.volume_numbers(), vec![3.0]);
    }

    #[test]
    fn a_novel_tail_is_junk() {
        let p = parse_title("Foo: A Novel");
        assert_eq!(p.main, "foo");
        assert!(p.subtitle.is_none());
        assert!(p.series_markers.is_empty());
        assert!(!p.junk.is_empty());
    }

    #[test]
    fn substantive_tail_is_true_subtitle() {
        let p = parse_title("Foo: And Other Stories");
        assert_eq!(p.main, "foo");
        assert!(p.subtitle.is_some());
        assert!(p.series_markers.is_empty());
    }

    #[test]
    fn unclassifiable_tail_lands_as_subtitle() {
        let p = parse_title("Foo: Xyzzy Plugh Fnord");
        assert_eq!(p.main, "foo");
        assert!(p.subtitle.is_some());
        assert!(p.series_markers.is_empty());
        assert!(p.junk.is_empty());
    }

    #[test]
    fn paren_series_suffix_is_series_marker() {
        let p = parse_title("The Way of Kings (The Stormlight Archive, #1)");
        assert_eq!(p.main, "way of kings");
        assert!(p.subtitle.is_none());
        assert_eq!(p.volume_numbers(), vec![1.0]);
    }

    #[test]
    fn series_name_comma_book_n_tail_is_series_marker() {
        let p = parse_title("Storm Front: The Dresden Files, Book 1");
        assert_eq!(p.main, "storm front");
        assert_eq!(p.volume_numbers(), vec![1.0]);
        assert!(p.subtitle.is_none());
    }

    #[test]
    fn unabridged_paren_is_junk_not_subtitle() {
        let p = parse_title("Dune (Unabridged)");
        assert_eq!(p.main, "dune");
        assert!(p.subtitle.is_none());
        assert!(!p.junk.is_empty());
    }

    // --- title_verdict: volume vetoes ---

    #[test]
    fn conflicting_tail_volumes_veto() {
        let a = parse_title("History of Rome: Volume 1");
        let b = parse_title("History of Rome: Volume 2");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::VetoVolume);
    }

    #[test]
    fn conflicting_caller_positions_veto_without_tails() {
        let a = parse_title("History of Rome");
        let b = parse_title("History of Rome");
        assert_eq!(
            title_verdict_with_positions(&a, Some(1.0), &b, Some(2.0)),
            TitleVerdict::VetoVolume
        );
    }

    #[test]
    fn position_missing_sibling_is_not_same() {
        let a = parse_title("History of Rome");
        let b = parse_title("History of Rome");
        let v = title_verdict_with_positions(&a, Some(1.0), &b, None);
        assert_ne!(v, TitleVerdict::Same);
        assert!(matches!(v, TitleVerdict::Grey { .. }), "got {v:?}");
    }

    #[test]
    fn matching_volumes_do_not_veto() {
        let a = parse_title("History of Rome: Volume 1");
        let b = parse_title("History of Rome, Vol. 1");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Same);
    }

    // --- title_verdict: tails demote, junk is ignored ---

    #[test]
    fn one_sided_series_tail_is_grey_never_same() {
        let a = parse_title("Storm Front");
        let b = parse_title("Storm Front: The Dresden Files, Book 1");
        let v = title_verdict(&a, &b);
        assert_ne!(v, TitleVerdict::Same);
        assert!(matches!(v, TitleVerdict::Grey { .. }), "got {v:?}");
    }

    #[test]
    fn disagreeing_subtitles_never_auto_same() {
        let a = parse_title("Mistborn: The Final Empire");
        let b = parse_title("Mistborn: The Well of Ascension");
        let v = title_verdict(&a, &b);
        assert_ne!(v, TitleVerdict::Same);
        assert!(matches!(v, TitleVerdict::Grey { .. }), "got {v:?}");
    }

    #[test]
    fn one_sided_true_subtitle_is_grey() {
        let a = parse_title("Foo");
        let b = parse_title("Foo: And Other Stories");
        let v = title_verdict(&a, &b);
        assert!(matches!(v, TitleVerdict::Grey { .. }), "got {v:?}");
    }

    #[test]
    fn junk_only_tail_difference_is_ignored() {
        let a = parse_title("Dune: A Novel");
        let b = parse_title("Dune");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Same);
    }

    #[test]
    fn agreeing_subtitles_stay_same() {
        let a = parse_title("The Power Broker: Robert Moses and the Fall of New York");
        let b = parse_title("The Power Broker: Robert Moses and the Fall of New York");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Same);
    }

    // --- title_verdict: lookalikes and colon-cut regression guards ---

    #[test]
    fn study_guide_lookalike_never_same() {
        let real = parse_title("Storm Front");
        let guide = parse_title("Study Guide: Storm Front by Jim Butcher");
        let summary = parse_title("Summary of Storm Front");
        assert_ne!(title_verdict(&real, &guide), TitleVerdict::Same);
        assert_ne!(title_verdict(&real, &summary), TitleVerdict::Same);
    }

    #[test]
    fn colon_truncation_lookalikes_are_not_same() {
        // Titles today's first-colon truncation would wrongly equate.
        let a = parse_title("Star Wars: A New Hope");
        let b = parse_title("Star Wars: The Empire Strikes Back");
        assert_ne!(title_verdict(&a, &b), TitleVerdict::Same);
    }

    #[test]
    fn both_titles_empty_never_same() {
        let a = parse_title("");
        let b = parse_title("");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Different);
    }

    #[test]
    fn one_empty_title_cannot_pass_any_bar() {
        let a = parse_title("");
        let b = parse_title("Dune");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Different);
    }

    // --- title_verdict: cleaning baseline and the grey band ---

    #[test]
    fn leading_article_and_accents_fold_into_exact_same() {
        let a = parse_title("The Hobbit");
        let b = parse_title("Hobbit");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Same);

        let c = parse_title("Café");
        let d = parse_title("Cafe");
        assert_eq!(title_verdict(&c, &d), TitleVerdict::Same);
    }

    #[test]
    fn near_titles_land_grey_with_real_score() {
        let a = parse_title("The Wise Man's Fear");
        let b = parse_title("The Wise Man's Fear Chronicle");
        match title_verdict(&a, &b) {
            TitleVerdict::Grey { score } => {
                assert!(score >= TITLE_GREY_FLOOR, "score {score}");
                assert!(score < 1.0, "score {score}");
            }
            other => panic!("expected Grey, got {other:?}"),
        }
    }

    #[test]
    fn unrelated_titles_are_different() {
        let a = parse_title("Storm Front");
        let b = parse_title("The Name of the Wind");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Different);
    }

    #[test]
    fn cjk_titles_compare_via_bigrams() {
        let a = parse_title("三体");
        let b = parse_title("三体");
        assert_eq!(title_verdict(&a, &b), TitleVerdict::Same);

        let c = parse_title("キッチン");
        let d = parse_title("ノルウェイの森");
        assert_ne!(title_verdict(&c, &d), TitleVerdict::Same);
    }

    // --- author_verdict ---

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn shared_surname_only_is_grey_never_agree() {
        let v = author_verdict(&names(&["John Smith"]), &names(&["Jane Smith"]));
        assert_eq!(v, AuthorVerdict::Grey);
    }

    #[test]
    fn extra_credited_name_is_non_evidence() {
        let v = author_verdict(
            &names(&["Jim Butcher"]),
            &names(&["Jim Butcher", "James Marsters"]),
        );
        assert_eq!(v, AuthorVerdict::Agree);
    }

    #[test]
    fn last_first_and_initials_normalize_to_agree() {
        let v = author_verdict(&names(&["Rowling, J. K."]), &names(&["J.K. Rowling"]));
        assert_eq!(v, AuthorVerdict::Agree);
    }

    #[test]
    fn initials_compatible_with_expanded_name_agree() {
        let v = author_verdict(
            &names(&["J.K. Rowling"]),
            &names(&["Joanne Kathleen Rowling"]),
        );
        assert_eq!(v, AuthorVerdict::Agree);
    }

    #[test]
    fn initials_compatible_with_two_candidates_is_grey() {
        let v = author_verdict(&names(&["J. Smith"]), &names(&["John Smith", "Jane Smith"]));
        assert_eq!(v, AuthorVerdict::Grey);
    }

    #[test]
    fn zero_overlap_is_disagree() {
        let v = author_verdict(&names(&["Frank Herbert"]), &names(&["Ursula Le Guin"]));
        assert_eq!(v, AuthorVerdict::Disagree);
    }

    #[test]
    fn empty_author_list_abstains() {
        assert_eq!(
            author_verdict(&[], &names(&["Frank Herbert"])),
            AuthorVerdict::Abstain
        );
        assert_eq!(author_verdict(&[], &[]), AuthorVerdict::Abstain);
    }

    #[test]
    fn accented_and_plain_forms_agree() {
        let v = author_verdict(
            &names(&["Gabriel García Márquez"]),
            &names(&["Gabriel Garcia Marquez"]),
        );
        assert_eq!(v, AuthorVerdict::Agree);
    }

    // --- language_verdict ---

    #[test]
    fn declared_language_mismatch_vetoes() {
        assert_eq!(
            language_verdict(Some("en"), Some("fr"), "en"),
            LanguageVerdict::Veto
        );
    }

    #[test]
    fn silent_payload_on_non_default_work_is_grey() {
        assert_eq!(
            language_verdict(Some("fr"), None, "en"),
            LanguageVerdict::Grey
        );
    }

    #[test]
    fn silent_payload_on_default_language_work_is_neutral() {
        assert_eq!(
            language_verdict(Some("en"), None, "en"),
            LanguageVerdict::Neutral
        );
    }

    #[test]
    fn equal_or_both_silent_is_neutral() {
        assert_eq!(
            language_verdict(Some("fr"), Some("fr"), "en"),
            LanguageVerdict::Neutral
        );
        assert_eq!(language_verdict(None, None, "en"), LanguageVerdict::Neutral);
    }

    // --- id_verdict ---

    #[test]
    fn work_key_equality_wins() {
        let a = IdEvidence {
            ol_key: Some("OL1W"),
            ..Default::default()
        };
        let b = IdEvidence {
            ol_key: Some("OL1W"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::WorkKeyEqual);
    }

    #[test]
    fn shared_isbn_with_work_key_contradiction_is_collision() {
        let a = IdEvidence {
            gr_key: Some("111"),
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        let b = IdEvidence {
            gr_key: Some("222"),
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::WorkKeyContradiction);
    }

    #[test]
    fn differing_asin_is_no_penalty() {
        let a = IdEvidence {
            ol_key: Some("OL1W"),
            asin: Some("B001"),
            ..Default::default()
        };
        let b = IdEvidence {
            ol_key: Some("OL1W"),
            asin: Some("B002"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::WorkKeyEqual);

        let c = IdEvidence {
            isbn_13: Some("9780000000001"),
            asin: Some("B001"),
            ..Default::default()
        };
        let d = IdEvidence {
            isbn_13: Some("9780000000001"),
            asin: Some("B002"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&c, &d), IdVerdict::EditionBridge);
    }

    #[test]
    fn shared_isbn_alone_is_an_edition_bridge() {
        let a = IdEvidence {
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        let b = IdEvidence {
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::EditionBridge);
    }

    #[test]
    fn edition_id_inequality_is_no_evidence() {
        let a = IdEvidence {
            isbn_13: Some("9780000000001"),
            ..Default::default()
        };
        let b = IdEvidence {
            isbn_13: Some("9780000000002"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::NoEvidence);
    }

    #[test]
    fn no_identifiers_is_no_evidence() {
        let a = IdEvidence::default();
        let b = IdEvidence {
            ol_key: Some("OL9W"),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::NoEvidence);
        assert_eq!(id_verdict(&a, &a), IdVerdict::NoEvidence);
    }

    #[test]
    fn blank_keys_are_absent_not_equal() {
        let a = IdEvidence {
            ol_key: Some("  "),
            ..Default::default()
        };
        let b = IdEvidence {
            ol_key: Some("  "),
            ..Default::default()
        };
        assert_eq!(id_verdict(&a, &b), IdVerdict::NoEvidence);
    }
}
