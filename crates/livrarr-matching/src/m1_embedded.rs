//! M1 — Embedded metadata extraction from EPUB, M4B, MP3 files.

use std::path::Path;

use id3::TagLike;
use rbook::Ebook;

use crate::types::{Confidence, Extraction, ExtractionSource};

/// Extract metadata from a file's embedded tags.
/// Returns None only if no usable title can be extracted.
pub fn extract_embedded(
    path: &Path,
    grouped_paths: Option<&[std::path::PathBuf]>,
) -> Option<Extraction> {
    let ext = path.extension()?.to_str()?.to_lowercase();
    match ext.as_str() {
        "epub" => extract_epub(path),
        "m4b" | "m4a" => extract_m4b(path),
        "mp3" => extract_mp3(path, grouped_paths),
        _ => None,
    }
}

fn extract_epub(path: &Path) -> Option<Extraction> {
    use livrarr_domain::normalization::{normalize_isbn13, normalize_language};

    let book = rbook::Epub::new(path).ok()?;
    let metadata = book.metadata();

    let raw_title = metadata.title().map(|t| decode_xml_entities(t.value()));
    let raw_author = metadata
        .creators()
        .first()
        .map(|c| decode_xml_entities(c.value()));
    let raw_language = metadata.language().map(|l| l.value().to_string());

    let title = raw_title.and_then(|t| sanitize_title(&t, path))?;
    let author = raw_author.and_then(|a| sanitize_author(&a));

    // Harvest ISBN from dc:identifier elements, stripping common URI prefixes
    let isbn = metadata.get("identifier").iter().find_map(|el| {
        let raw = el.value();
        let stripped = raw
            .strip_prefix("ISBN:")
            .or_else(|| raw.strip_prefix("isbn:"))
            .or_else(|| raw.strip_prefix("urn:isbn:"))
            .or_else(|| raw.strip_prefix("URN:ISBN:"))
            .unwrap_or(raw);
        normalize_isbn13(stripped)
    });

    // Harvest the publication year from dc:date (best-effort; first 4-digit run).
    let year = metadata
        .get("date")
        .iter()
        .find_map(|el| parse_year(el.value()));

    // Normalize through the single authority; an unrecognized language is left
    // absent rather than stored raw (REQ-005). The widened ISO-639-1 table makes
    // the previous raw fallback unnecessary and avoids leaking a subtagged value.
    let language = raw_language.as_deref().and_then(normalize_language);

    let confidence = if author.is_some() {
        Confidence::High
    } else {
        Confidence::Medium
    };

    Some(Extraction {
        title: Some(title),
        author,
        year,
        isbn,
        language,
        series: None,
        series_position: None,
        narrator: None,
        asin: None,
        confidence,
        source: ExtractionSource::Embedded,
    })
}

fn extract_m4b(path: &Path) -> Option<Extraction> {
    use livrarr_domain::normalization::{normalize_asin, AsinNorm};
    use mp4ameta::FreeformIdent;

    let tag = mp4ameta::Tag::read_from_path(path).ok()?;

    let raw_title = tag.title().map(|s| s.to_string());
    let raw_author = tag.artist().map(|s| s.to_string());
    let raw_year = tag.year().and_then(|s| s.to_string().parse::<i32>().ok());

    // Language is left absent: mp4ameta 0.13's public `Tag` API only exposes the
    // iTunes `ilst` metadata atoms (title/artist/year/freeform), none of which is
    // a language tag. The track language lives in the `mdhd` media-header atom,
    // which the crate parses but keeps private (the `mdia`/`mdhd` modules are not
    // `pub`). A wrong language is worse than None downstream (it would force a
    // foreign audiobook onto the wrong edition), so we do not guess.
    let language = None;

    let title = raw_title.and_then(|t| sanitize_title(&t, path))?;
    let author = raw_author.and_then(|a| sanitize_author(&a));

    // Harvest ASIN from the iTunes freeform ----:com.apple.iTunes:ASIN atom
    let asin_ident = FreeformIdent::new_static("com.apple.iTunes", "ASIN");
    let raw_asin = tag.strings_of(&asin_ident).next().map(str::to_string);

    let (isbn, asin) = match raw_asin.as_deref() {
        Some(a) => match normalize_asin(a) {
            AsinNorm::Isbn13(isbn13) => (Some(isbn13), None),
            AsinNorm::Asin(a) => (None, Some(a)),
            AsinNorm::Invalid => (None, None),
        },
        None => (None, None),
    };

    let confidence = if author.is_some() {
        Confidence::High
    } else {
        Confidence::Medium
    };

    Some(Extraction {
        title: Some(title),
        author,
        year: raw_year,
        isbn,
        language,
        series: None,
        series_position: None,
        narrator: None,
        asin,
        confidence,
        source: ExtractionSource::Embedded,
    })
}

fn extract_mp3(path: &Path, grouped_paths: Option<&[std::path::PathBuf]>) -> Option<Extraction> {
    let paths_to_read: Vec<&Path> = if let Some(group) = grouped_paths {
        group.iter().take(5).map(|p| p.as_path()).collect()
    } else {
        vec![path]
    };

    use livrarr_domain::normalization::normalize_language;

    let mut titles: Vec<String> = Vec::new();
    let mut artists: Vec<String> = Vec::new();
    let mut albums: Vec<String> = Vec::new();
    let mut years: Vec<i32> = Vec::new();
    let mut languages: Vec<String> = Vec::new();

    for p in &paths_to_read {
        if let Ok(tag) = id3::Tag::read_from_path(p) {
            if let Some(t) = tag.title() {
                titles.push(t.to_string());
            }
            if let Some(a) = tag.artist() {
                artists.push(a.to_string());
            }
            if let Some(al) = tag.album() {
                albums.push(al.to_string());
            }
            if let Some(y) = tag.year() {
                years.push(y);
            }
            // ID3 `TLAN` carries the language; normalize through the single
            // authority (unrecognized → dropped) so a foreign audiobook is not
            // treated as English downstream.
            if let Some(lang) = tag.text_for_frame_id("TLAN").and_then(normalize_language) {
                languages.push(lang);
            }
        }
    }

    let raw_title = {
        let most_common_album = most_common_non_garbage_title(&albums, path);
        let most_common_title = most_common_non_garbage_title(&titles, path);
        match (most_common_album, most_common_title) {
            (Some(a), _) => Some(a),
            (None, Some(t)) => Some(t),
            _ => None,
        }
    };

    let raw_author = most_common_non_garbage_author(&artists);

    let title = raw_title?;
    let author = raw_author;

    let year = most_common(&years).copied();
    let language = most_common(&languages).cloned();

    let confidence = if author.is_some() {
        Confidence::High
    } else {
        Confidence::Medium
    };

    Some(Extraction {
        title: Some(title),
        author,
        year,
        isbn: None,
        language,
        series: None,
        series_position: None,
        narrator: None,
        asin: None,
        confidence,
        source: ExtractionSource::Embedded,
    })
}

use livrarr_domain::decode_xml_entities;

// ---------------------------------------------------------------------------
// Sanity filters
// ---------------------------------------------------------------------------

use regex::Regex;
use std::sync::LazyLock as Lazy;

static GARBAGE_TITLE: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"(?i)^track\s*\d+$").unwrap(),
        Regex::new(
            r"(?i)^chapter\s+(\d+|one|two|three|four|five|six|seven|eight|nine|ten|[ivxlc]+)$",
        )
        .unwrap(),
        Regex::new(r"(?i)^ch\.\s*\d+$").unwrap(),
        Regex::new(r"(?i)^(disc|cd|part)\s*\d+$").unwrap(),
        Regex::new(r"(?i)^side\s*[ab]$").unwrap(),
        Regex::new(r"^\d{1,3}$").unwrap(),
        Regex::new(r"(?i)^(unknown|untitled|audiobook|full book)$").unwrap(),
        Regex::new(r"(?i)^https?://").unwrap(),
        Regex::new(r"(?i)\.(com|net|org)").unwrap(),
    ]
});

static GARBAGE_AUTHOR: Lazy<Vec<Regex>> = Lazy::new(|| {
    vec![
        Regex::new(r"(?i)^(unknown|unknown author|various|various authors|va)$").unwrap(),
        Regex::new(r"(?i)^(author|calibre|administrator|admin)$").unwrap(),
        Regex::new(r"(?i)^(read by|narrated by)").unwrap(),
        Regex::new(r"(?i)^(microsoft|amazon|google)").unwrap(),
        Regex::new(r"(?i)^https?://").unwrap(),
        Regex::new(r"(?i)\.(com|net)").unwrap(),
    ]
});

fn sanitize_title(title: &str, path: &Path) -> Option<String> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
        if trimmed.eq_ignore_ascii_case(stem) && GARBAGE_TITLE.iter().any(|re| re.is_match(trimmed))
        {
            return None;
        }
    }
    if GARBAGE_TITLE.iter().any(|re| re.is_match(trimmed)) {
        return None;
    }
    Some(trimmed.to_string())
}

fn sanitize_author(author: &str) -> Option<String> {
    let trimmed = author.trim();
    if trimmed.is_empty() {
        return None;
    }
    if GARBAGE_AUTHOR.iter().any(|re| re.is_match(trimmed)) {
        return None;
    }
    Some(trimmed.to_string())
}

fn most_common_non_garbage_title(values: &[String], path: &Path) -> Option<String> {
    let clean: Vec<String> = values
        .iter()
        .filter_map(|v| sanitize_title(v, path))
        .collect();
    most_common(&clean).cloned()
}

fn most_common_non_garbage_author(values: &[String]) -> Option<String> {
    let clean: Vec<String> = values.iter().filter_map(|v| sanitize_author(v)).collect();
    most_common(&clean).cloned()
}

fn most_common<T: Eq + std::hash::Hash>(values: &[T]) -> Option<&T> {
    if values.is_empty() {
        return None;
    }
    let mut counts = std::collections::HashMap::new();
    for v in values {
        *counts.entry(v).or_insert(0u32) += 1;
    }
    counts.into_iter().max_by_key(|(_, c)| *c).map(|(v, _)| v)
}

/// Best-effort publication year from a date string: the first standalone run of
/// exactly four ASCII digits (handles `"2021"`, `"2021-03-01"`, `"March 2021"`).
/// Mirrors the OpenLibrary `first_publish_date` parsing convention.
fn parse_year(raw: &str) -> Option<i32> {
    raw.split(|c: char| !c.is_ascii_digit())
        .find(|tok| tok.len() == 4)
        .and_then(|tok| tok.parse::<i32>().ok())
        .filter(|y| (1000..=2999).contains(y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Write an ID3v2.4 tag (title + optional TLAN) into a fresh `.mp3` file in a
    /// unique temp dir, returning the path. No real audio data is needed:
    /// `id3::Tag::read_from_path` decodes a bare ID3 header by magic.
    fn write_mp3_with_tags(name: &str, title: &str, tlan: Option<&str>) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("livrarr-m1-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.mp3"));
        // The file must exist before `write_to_path` (it opens read+write).
        std::fs::File::create(&path)
            .unwrap()
            .write_all(&[])
            .unwrap();

        let mut tag = id3::Tag::new();
        tag.set_text("TIT2", title);
        if let Some(l) = tlan {
            tag.set_text("TLAN", l);
        }
        tag.write_to_path(&path, id3::Version::Id3v24).unwrap();
        path
    }

    #[test]
    fn mp3_tlan_yields_normalized_language() {
        // A German MP3 (TLAN="ger") surfaces normalized "de".
        let path = write_mp3_with_tags("german", "Der Steppenwolf", Some("ger"));
        let ex = extract_mp3(&path, None).expect("extraction");
        assert_eq!(ex.language.as_deref(), Some("de"));
    }

    #[test]
    fn mp3_tlan_unrecognized_is_dropped() {
        // An unrecognized TLAN value normalizes to None (not stored raw).
        let path = write_mp3_with_tags("bogus", "Some Title", Some("zzz"));
        let ex = extract_mp3(&path, None).expect("extraction");
        assert_eq!(ex.language, None);
    }

    #[test]
    fn mp3_without_tlan_has_no_language() {
        let path = write_mp3_with_tags("notag", "Some Title", None);
        let ex = extract_mp3(&path, None).expect("extraction");
        assert_eq!(ex.language, None);
    }
}
