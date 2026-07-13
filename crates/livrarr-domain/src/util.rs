//! Pure helper functions: filesystem-safe path sanitization, sort-name and
//! series-suffix derivation, language/string normalization, file-extension
//! classification, XML entity decoding, and cover-URL proxying.

use crate::entities::MediaType;

/// Sanitizes a path component for filesystem use.
///
/// Satisfies: IMPORT-011
pub fn sanitize_path_component(input: &str, fallback: &str) -> String {
    const MAX_BYTES: usize = 255;
    const ELLIPSIS: &str = "...";

    fn sanitize_inner(s: &str) -> String {
        const ILLEGAL: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];

        // Strip control characters, replace illegal chars with underscore
        let sanitized: String = s
            .chars()
            .filter(|c| !c.is_control())
            .map(|c| if ILLEGAL.contains(&c) { '_' } else { c })
            .collect();

        // Trim trailing dots and spaces
        sanitized.trim_end_matches(['.', ' ']).to_string()
    }

    let trimmed = sanitize_inner(input);

    // "." / ".." or empty after sanitization -> sanitize fallback too
    let result = if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        let fb = sanitize_inner(fallback);
        if fb.is_empty() || fb == "." || fb == ".." {
            // Ultimate fallback if even the fallback is invalid
            return "_".to_string();
        }
        fb
    } else {
        trimmed
    };

    // Truncate to MAX_BYTES if needed
    if result.len() > MAX_BYTES {
        let max_content = MAX_BYTES - ELLIPSIS.len();
        // Find the last valid UTF-8 char boundary at or before max_content
        let mut end = max_content;
        while !result.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}{}", &result[..end], ELLIPSIS)
    } else {
        result
    }
}

/// Derives sort name from display name using a surname-as-last-word heuristic.
///
/// Note: Assumes the last whitespace-delimited word is the surname. This is
/// incorrect for some naming conventions (e.g., East Asian, Iberian, compound
/// surnames like "van der Berg"), but matches the Readarr/Servarr convention.
///
/// "Frank Herbert" -> "Herbert, Frank"
/// "J.R.R. Tolkien" -> "Tolkien, J.R.R."
/// Single-word name -> returned as-is.
pub fn derive_sort_name(display_name: &str) -> String {
    let trimmed = display_name.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Use rsplit_once to split at the last whitespace boundary.
    // This avoids collecting into an intermediate Vec.
    match trimmed.rsplit_once(char::is_whitespace) {
        Some((given, surname)) => format!("{}, {}", surname.trim(), given.trim()),
        None => trimmed.to_string(),
    }
}

/// Normalizes a string for scan matching. Applies the same character rules
/// as `sanitize_path_component` but replaces illegal chars with spaces
/// (for matching) instead of underscores (for filesystem). Also replaces
/// dots and underscores with spaces so that Livrarr-imported filenames
/// (which use underscores for illegal chars) match back to their DB titles.
///
/// Superseded in production by [`crate::identity_matching::identity_key`] (REQ-014):
/// this recipe keeps stopwords and accents (no leading-article drop, no
/// accent strip), which is exactly the mismatch ST-04 named. No production
/// call site uses this function anymore; it is retained because existing
/// test fixtures across the suite construct `normalized_title`/
/// `normalized_author` values with it.
///
/// Satisfies: SCAN-002, SCAN-003
pub fn normalize_for_matching(s: &str) -> String {
    const ILLEGAL: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let normalized: String = s
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| {
            if ILLEGAL.contains(&c) || c == '.' || c == '_' {
                ' '
            } else {
                c
            }
        })
        .collect();
    // Collapse multiple spaces and trim
    let mut result = String::with_capacity(normalized.len());
    let mut prev_space = true; // trim leading
    for c in normalized.chars() {
        if c == ' ' {
            if !prev_space {
                result.push(' ');
            }
            prev_space = true;
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    // Trim trailing space
    if result.ends_with(' ') {
        result.pop();
    }
    result.to_lowercase()
}

/// Marker prefix for stub series gr_keys (series rows created from work
/// metadata rather than Goodreads). A real GR series key is numeric, so the
/// prefix cannot collide. Stub keys are internal — API responses mask them.
pub const SERIES_STUB_KEY_PREFIX: &str = "stub:";

/// Sentinel `work_count` for stub series: any GR-backed series (real,
/// smaller roster) beats a stub under the "fewest books wins" assignment
/// guard; a stub never steals a work. Masked to 0 at the API boundary.
pub const SERIES_STUB_WORK_COUNT: i32 = i32::MAX;

pub fn is_series_stub_key(gr_key: &str) -> bool {
    gr_key.starts_with(SERIES_STUB_KEY_PREFIX)
}

/// Splits a positional suffix off a series name: `"The Wheel of Time, Book 3"`
/// → `("The Wheel of Time", Some(3.0))`. Recognized suffix forms after the
/// last comma: `Book N`, `#N`, `Vol N`, `Vol. N`, `Volume N` (N may be
/// fractional, e.g. `3.5`). A name with no recognized suffix is returned
/// trimmed, with `None`.
pub fn split_series_suffix(name: &str) -> (String, Option<f64>) {
    let trimmed = name.trim();
    if let Some((prefix, suffix)) = trimmed.rsplit_once(',') {
        let prefix = prefix.trim();
        let suffix = suffix.trim();
        if !prefix.is_empty() {
            let number_part = suffix
                .strip_prefix('#')
                .or_else(|| {
                    [
                        "Book", "book", "Volume", "volume", "Vol.", "vol.", "Vol", "vol",
                    ]
                    .iter()
                    .find_map(|kw| suffix.strip_prefix(kw))
                })
                .map(str::trim);
            if let Some(n) = number_part.and_then(|p| p.parse::<f64>().ok()) {
                if n.is_finite() {
                    return (prefix.to_string(), Some(n));
                }
            }
        }
    }
    (trimmed.to_string(), None)
}

/// Normalize a language value to an ISO 639-1 two-letter code.
///
/// Delegates to [`crate::normalization::normalize_language`] — the single
/// normalization authority (REQ-005) — and falls back to the trimmed,
/// lower-cased input for a value that authority does not recognize, preserving
/// this function's historical pass-through contract for its enrichment callers.
/// (Unlike the previous local table, this now also strips region subtags from
/// recognized languages, e.g. `"en-US"` → `"en"`.)
pub fn normalize_language(lang: &str) -> String {
    crate::normalization::normalize_language(lang).unwrap_or_else(|| lang.trim().to_lowercase())
}

/// Normalize an optional language value.
pub fn normalize_language_opt(lang: Option<&str>) -> Option<String> {
    lang.filter(|s| !s.is_empty()).map(normalize_language)
}

/// Classifies a file path into a MediaType based on extension.
pub fn classify_file(path: &std::path::Path) -> Option<MediaType> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    match ext.as_str() {
        "epub" | "mobi" | "azw3" | "pdf" => Some(MediaType::Ebook),
        "mp3" | "m4a" | "m4b" | "flac" | "ogg" | "wma" => Some(MediaType::Audiobook),
        _ => None,
    }
}

pub fn decode_xml_entities(s: &str) -> String {
    // `&amp;` must decode LAST: decoding it first turns a literal `&amp;quot;`
    // into `&quot;`, which the later pass then wrongly decodes again —
    // corrupting any payload (e.g. attribute-encoded JSON) that carries
    // entity text inside strings.
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

/// Proxy an external cover URL through the internal cover proxy endpoint.
/// URLs already starting with '/' are returned as-is (already local).
pub fn proxy_cover_url(url: &str) -> String {
    if url.starts_with('/') {
        return url.to_string();
    }
    format!("/api/v1/coverproxy?url={}", urlencoding::encode(url))
}

/// Reverse `proxy_cover_url`: recover the canonical external URL from the
/// internal cover-proxy display form (`/api/v1/coverproxy?url=<encoded-url>`).
/// Values that are not in proxied form are returned unchanged.
///
/// The search results the UI renders carry covers in proxied form so `<img>`
/// tags can fetch them. When the user picks one of those covers, the persisted
/// value must be the real provider URL, not the proxied display string — a
/// proxied (leading-`/`) value is not a usable cover source.
pub fn unproxy_cover_url(url: &str) -> String {
    match url.strip_prefix("/api/v1/coverproxy?url=") {
        Some(encoded) => urlencoding::decode(encoded)
            .map(|s| s.into_owned())
            .unwrap_or_else(|_| url.to_string()),
        None => url.to_string(),
    }
}

/// Strip all non-alphanumeric characters from an ISBN (hyphens, spaces, etc.).
pub fn strip_isbn_punctuation(isbn: &str) -> String {
    isbn.chars().filter(|c| c.is_alphanumeric()).collect()
}

#[cfg(test)]
mod series_suffix_tests {
    use super::split_series_suffix;

    #[test]
    fn strips_book_n() {
        assert_eq!(
            split_series_suffix("The Wheel of Time, Book 3"),
            ("The Wheel of Time".to_string(), Some(3.0))
        );
    }

    #[test]
    fn strips_hash_n() {
        assert_eq!(
            split_series_suffix("Dresden Files, #12"),
            ("Dresden Files".to_string(), Some(12.0))
        );
    }

    #[test]
    fn strips_fractional_position() {
        assert_eq!(
            split_series_suffix("Saga, Book 3.5"),
            ("Saga".to_string(), Some(3.5))
        );
    }

    #[test]
    fn strips_volume_forms() {
        assert_eq!(
            split_series_suffix("Foo, Volume 2"),
            ("Foo".to_string(), Some(2.0))
        );
        assert_eq!(
            split_series_suffix("Foo, Vol. 4"),
            ("Foo".to_string(), Some(4.0))
        );
    }

    #[test]
    fn plain_name_untouched() {
        assert_eq!(
            split_series_suffix("The Green Bone Saga"),
            ("The Green Bone Saga".to_string(), None)
        );
    }

    #[test]
    fn comma_without_positional_suffix_untouched() {
        assert_eq!(
            split_series_suffix("Hello, World"),
            ("Hello, World".to_string(), None)
        );
        assert_eq!(
            split_series_suffix("The Series, 3"),
            ("The Series, 3".to_string(), None)
        );
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(
            split_series_suffix("  Uplift Saga  "),
            ("Uplift Saga".to_string(), None)
        );
    }
}
