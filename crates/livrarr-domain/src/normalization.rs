//! Single authority for identifier and language normalization and validation.
//!
//! Every work-creation path routes harvested identifiers through these helpers,
//! so a malformed value is treated as absent — never persisted and never sent to
//! a provider. ISBN conversion lives here so there is exactly one implementation
//! of each rule across the workspace.

/// Outcome of shape-guarding a raw ASIN-shaped value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsinNorm {
    /// The value was an ISBN-10 (shape + checksum valid) and has been folded to
    /// an ISBN-13. Store it as `isbn_13`, never as an `asin`.
    Isbn13(String),
    /// A genuine Amazon ASIN — retained for audiobook lookups.
    Asin(String),
    /// Malformed — treat as absent.
    Invalid,
}

/// Validate length + checksum and canonicalize to a 13-digit ISBN-13.
///
/// Accepts a checksum-valid ISBN-10 (converted to ISBN-13) or ISBN-13. Returns
/// `None` for any value that fails the length or checksum test.
pub fn normalize_isbn13(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| if c == 'x' { 'X' } else { c })
        .collect();

    match cleaned.len() {
        10 => {
            let (body, tail) = cleaned.split_at(9);
            if !body.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let check_char = tail.chars().next()?;
            if !check_char.is_ascii_digit() && check_char != 'X' {
                return None;
            }
            // Validate ISBN-10 mod-11 checksum: sum(d[i] * (10 - i)) ≡ 0 (mod 11)
            let check_val: u32 = if check_char == 'X' {
                10
            } else {
                check_char.to_digit(10)?
            };
            let body_sum: u32 = body
                .chars()
                .enumerate()
                .map(|(i, c)| c.to_digit(10).unwrap_or(0) * (10 - i as u32))
                .sum();
            if !(body_sum + check_val).is_multiple_of(11) {
                return None;
            }
            // Convert to ISBN-13: 978 prefix + 9-digit body + mod-10 check digit
            let prefix = format!("978{body}");
            let sum13: u32 = prefix
                .chars()
                .enumerate()
                .map(|(i, c)| {
                    let d = c.to_digit(10).unwrap();
                    if i % 2 == 0 {
                        d
                    } else {
                        d * 3
                    }
                })
                .sum();
            let check13 = (10 - (sum13 % 10)) % 10;
            Some(format!("{prefix}{check13}"))
        }
        13 => {
            if !cleaned.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            // Validate ISBN-13 mod-10 checksum: alternating weights 1/3
            let sum: u32 = cleaned
                .chars()
                .enumerate()
                .map(|(i, c)| {
                    let d = c.to_digit(10).unwrap();
                    if i % 2 == 0 {
                        d
                    } else {
                        d * 3
                    }
                })
                .sum();
            if !sum.is_multiple_of(10) {
                return None;
            }
            Some(cleaned)
        }
        _ => None,
    }
}

/// Shape-guard a raw ASIN.
///
/// An ISBN-10-shaped, checksum-valid value folds to [`AsinNorm::Isbn13`]; a
/// genuine Amazon ASIN yields [`AsinNorm::Asin`]; anything else is
/// [`AsinNorm::Invalid`].
pub fn normalize_asin(raw: &str) -> AsinNorm {
    let s: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect();

    if s.is_empty() {
        return AsinNorm::Invalid;
    }

    // ISBN-10 shape: 9 ASCII digits + (digit or X)
    let is_isbn10_shape = s.len() == 10
        && s[..9].chars().all(|c| c.is_ascii_digit())
        && s.chars()
            .nth(9)
            .is_some_and(|c| c.is_ascii_digit() || c == 'X');

    if is_isbn10_shape {
        // normalize_isbn13 validates the checksum; pass iff valid
        if let Some(isbn13) = normalize_isbn13(&s) {
            return AsinNorm::Isbn13(isbn13);
        }
        // Shape matches but checksum fails → genuine Amazon ASIN
        return AsinNorm::Asin(s);
    }

    // A canonical ASIN is exactly 10 alphanumeric characters (e.g. a B-prefixed
    // Kindle/Audible id). Anything else is not a usable identifier and is
    // treated as absent (REQ-029) rather than persisted or sent to a provider.
    if s.len() == 10 && s.chars().all(|c| c.is_ascii_alphanumeric()) {
        AsinNorm::Asin(s)
    } else {
        AsinNorm::Invalid
    }
}

/// Reduce a Goodreads key to its bare leading numeric segment
/// (`"123.Slug"` and `"123-slug"` both become `"123"`).
///
/// Returns `None` when there is no leading digit run.
pub fn normalize_gr_key(raw: &str) -> Option<String> {
    let digits: String = raw
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if digits.is_empty() {
        None
    } else {
        Some(digits)
    }
}

/// Normalize a language tag to its ISO 639-1 two-letter code, stripping any
/// region subtag (`"en-US"` becomes `"en"`).
///
/// Returns `None` when the input maps to no known language.
pub fn normalize_language(raw: &str) -> Option<String> {
    // Strip region/script subtag: "en-US" → "en", "pt-BR" → "pt"
    let primary = raw
        .trim()
        .split(['-', '_'])
        .next()
        .unwrap_or("")
        .to_lowercase();

    if primary.is_empty() {
        return None;
    }

    // Map full language names and 3-letter codes to their ISO 639-1 code.
    let mapped = match primary.as_str() {
        "en" | "english" | "eng" => Some("en"),
        "fr" | "french" | "français" | "fra" | "fre" => Some("fr"),
        "de" | "german" | "deutsch" | "deu" | "ger" => Some("de"),
        "es" | "spanish" | "español" | "spa" => Some("es"),
        "pl" | "polish" | "polski" | "pol" => Some("pl"),
        "nl" | "dutch" | "nederlands" | "nld" | "dut" => Some("nl"),
        "it" | "italian" | "italiano" | "ita" => Some("it"),
        "pt" | "portuguese" | "português" | "por" => Some("pt"),
        "ja" | "japanese" | "日本語" | "jpn" => Some("ja"),
        "ko" | "korean" | "한국어" | "kor" => Some("ko"),
        "zh" | "chinese" | "中文" | "zho" | "chi" => Some("zh"),
        "ru" | "russian" | "русский" | "rus" => Some("ru"),
        "sv" | "swedish" | "svenska" | "swe" => Some("sv"),
        "no" | "norwegian" | "norsk" | "nor" => Some("no"),
        "da" | "danish" | "dansk" | "dan" => Some("da"),
        "fi" | "finnish" | "suomi" | "fin" => Some("fi"),
        "cs" | "czech" | "čeština" | "ces" | "cze" => Some("cs"),
        "tr" | "turkish" | "türkçe" | "tur" => Some("tr"),
        "ar" | "arabic" | "العربية" | "ara" => Some("ar"),
        "hi" | "hindi" | "हिन्दी" | "hin" => Some("hi"),
        "ro" | "romanian" | "română" | "ron" | "rum" => Some("ro"),
        "hu" | "hungarian" | "magyar" | "hun" => Some("hu"),
        _ => None,
    };
    if let Some(code) = mapped {
        return Some(code.to_string());
    }

    // Fall back to recognizing any bare ISO 639-1 two-letter code as itself, so a
    // work tagged with a less-common language (e.g. "el", "uk", "he", "th") is
    // still identified and routed correctly. Without this, an unlisted language
    // normalizes to None and is treated as unresolved → English-eligible, which
    // would defeat the REQ-027 foreign-language routing (OL/HC must not enrich a
    // non-English work).
    const ISO_639_1: &[&str] = &[
        "aa", "ab", "ae", "af", "ak", "am", "an", "ar", "as", "av", "ay", "az", "ba", "be", "bg",
        "bh", "bi", "bm", "bn", "bo", "br", "bs", "ca", "ce", "ch", "co", "cr", "cs", "cu", "cv",
        "cy", "da", "de", "dv", "dz", "ee", "el", "en", "eo", "es", "et", "eu", "fa", "ff", "fi",
        "fj", "fo", "fr", "fy", "ga", "gd", "gl", "gn", "gu", "gv", "ha", "he", "hi", "ho", "hr",
        "ht", "hu", "hy", "hz", "ia", "id", "ie", "ig", "ii", "ik", "io", "is", "it", "iu", "ja",
        "jv", "ka", "kg", "ki", "kj", "kk", "kl", "km", "kn", "ko", "kr", "ks", "ku", "kv", "kw",
        "ky", "la", "lb", "lg", "li", "ln", "lo", "lt", "lu", "lv", "mg", "mh", "mi", "mk", "ml",
        "mn", "mr", "ms", "mt", "my", "na", "nb", "nd", "ne", "ng", "nl", "nn", "no", "nr", "nv",
        "ny", "oc", "oj", "om", "or", "os", "pa", "pi", "pl", "ps", "pt", "qu", "rm", "rn", "ro",
        "ru", "rw", "sa", "sc", "sd", "se", "sg", "si", "sk", "sl", "sm", "sn", "so", "sq", "sr",
        "ss", "st", "su", "sv", "sw", "ta", "te", "tg", "th", "ti", "tk", "tl", "tn", "to", "tr",
        "ts", "tt", "tw", "ty", "ug", "uk", "ur", "uz", "ve", "vi", "vo", "wa", "wo", "xh", "yi",
        "yo", "za", "zh", "zu",
    ];
    if primary.len() == 2 && ISO_639_1.contains(&primary.as_str()) {
        return Some(primary);
    }

    None
}
