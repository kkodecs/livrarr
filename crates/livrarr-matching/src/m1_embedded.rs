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

    let confidence = if author.is_some() {
        Confidence::High
    } else {
        Confidence::Medium
    };

    Some(Extraction {
        title: Some(title),
        author,
        year: None,
        isbn: None,
        language: raw_language,
        series: None,
        series_position: None,
        narrator: None,
        asin: None,
        confidence,
        source: ExtractionSource::Embedded,
    })
}

fn extract_m4b(path: &Path) -> Option<Extraction> {
    let tag = mp4ameta::Tag::read_from_path(path).ok()?;

    let raw_title = tag.title().map(|s| s.to_string());
    let raw_author = tag.artist().map(|s| s.to_string());
    let raw_year = tag.year().and_then(|s| s.to_string().parse::<i32>().ok());

    let title = raw_title.and_then(|t| sanitize_title(&t, path))?;
    let author = raw_author.and_then(|a| sanitize_author(&a));

    let confidence = if author.is_some() {
        Confidence::High
    } else {
        Confidence::Medium
    };

    Some(Extraction {
        title: Some(title),
        author,
        year: raw_year,
        isbn: None,
        language: None,
        series: None,
        series_position: None,
        narrator: None,
        asin: None,
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

    let mut titles: Vec<String> = Vec::new();
    let mut artists: Vec<String> = Vec::new();
    let mut albums: Vec<String> = Vec::new();
    let mut years: Vec<i32> = Vec::new();

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
        language: None,
        series: None,
        series_position: None,
        narrator: None,
        asin: None,
        confidence,
        source: ExtractionSource::Embedded,
    })
}

fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
}

// ---------------------------------------------------------------------------
// Sanity filters
// ---------------------------------------------------------------------------

use once_cell::sync::Lazy;
use regex::Regex;

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
