#![allow(dead_code)]

pub use livrarr_domain::*;

use std::path::Path;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Metadata for tag writing.
#[derive(Debug, Clone)]
pub struct TagMetadata {
    pub title: String,
    pub subtitle: Option<String>,
    pub author: String,
    pub narrator: Option<Vec<String>>,
    pub year: Option<i32>,
    pub genre: Option<Vec<String>>,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub isbn: Option<String>,
    pub language: Option<String>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
}

#[derive(Debug)]
pub enum TagWriteStatus {
    Written,
    Unsupported,
    NoData,
}

#[derive(Debug, thiserror::Error)]
pub enum TagWriteError {
    #[error("file not found: {path}")]
    FileNotFound { path: String },
    #[error("EPUB tag write failed: {message}")]
    EpubFailed { message: String },
    #[error("M4B tag write failed: {message}")]
    M4bFailed { message: String },
    #[error("MP3 tag write failed: {message}")]
    Mp3Failed { message: String },
    #[error("batch write aborted at {path}: {source}")]
    BatchAborted {
        path: String,
        source: Box<TagWriteError>,
    },
    #[error("I/O error: {message}")]
    Io { message: String },
}

// ---------------------------------------------------------------------------
// Public async API — hides spawn_blocking internally
// ---------------------------------------------------------------------------

/// Write tags to a single file in place. Detects format from extension (case-insensitive).
/// Caller manages .tmp lifecycle; this function modifies the file directly.
///
/// Satisfies: TAG-001, TAG-003, TAG-004, TAG-005, TAG-008, TAG-V21-007
pub async fn write_tags(
    file_path: String,
    metadata: TagMetadata,
    cover: Option<Vec<u8>>,
) -> Result<TagWriteStatus, TagWriteError> {
    tokio::task::spawn_blocking(move || write_tags_sync(&file_path, &metadata, cover.as_deref()))
        .await
        .map_err(|e| TagWriteError::Io {
            message: format!("spawn_blocking join error: {e}"),
        })?
}

/// Write tags to multiple MP3 files in place. Shared metadata/cover for all files.
/// If any file fails, returns BatchAborted with context. Caller handles cleanup.
///
/// Satisfies: TAG-006
pub async fn write_tags_batch(
    paths: Vec<String>,
    metadata: TagMetadata,
    cover: Option<Vec<u8>>,
) -> Result<Vec<TagWriteStatus>, TagWriteError> {
    tokio::task::spawn_blocking(move || write_tags_batch_sync(&paths, &metadata, cover.as_deref()))
        .await
        .map_err(|e| TagWriteError::Io {
            message: format!("spawn_blocking join error: {e}"),
        })?
}

// ---------------------------------------------------------------------------
// Sync implementations (called inside spawn_blocking)
// ---------------------------------------------------------------------------

fn write_tags_sync(
    file_path: &str,
    metadata: &TagMetadata,
    cover: Option<&[u8]>,
) -> Result<TagWriteStatus, TagWriteError> {
    let path = Path::new(file_path);
    if !path.exists() {
        return Err(TagWriteError::FileNotFound {
            path: file_path.to_string(),
        });
    }

    if metadata.title.is_empty() && metadata.author.is_empty() {
        return Ok(TagWriteStatus::NoData);
    }

    // Caller passes .tmp files (e.g., "book.epub.tmp"). Strip .tmp to detect real format.
    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let name_lower = file_name.to_lowercase();
    let name_for_ext = if name_lower.ends_with(".tmp") {
        &file_name[..file_name.len() - 4]
    } else {
        file_name
    };
    let ext = Path::new(name_for_ext)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "epub" => {
            write_epub(path, metadata, cover)?;
            Ok(TagWriteStatus::Written)
        }
        "m4b" => {
            write_m4b(path, metadata, cover)?;
            Ok(TagWriteStatus::Written)
        }
        "mp3" => {
            write_mp3(path, metadata, cover)?;
            Ok(TagWriteStatus::Written)
        }
        _ => Ok(TagWriteStatus::Unsupported),
    }
}

fn write_tags_batch_sync(
    paths: &[String],
    metadata: &TagMetadata,
    cover: Option<&[u8]>,
) -> Result<Vec<TagWriteStatus>, TagWriteError> {
    let mut results = Vec::with_capacity(paths.len());

    for path in paths {
        match write_tags_sync(path, metadata, cover) {
            Ok(status) => results.push(status),
            Err(e) => {
                return Err(TagWriteError::BatchAborted {
                    path: path.clone(),
                    source: Box::new(e),
                });
            }
        }
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// EPUB (via zip + regex_lite)
// ---------------------------------------------------------------------------

fn write_epub(
    path: &Path,
    metadata: &TagMetadata,
    cover: Option<&[u8]>,
) -> Result<(), TagWriteError> {
    use std::io::{Read, Write};

    let file = std::fs::File::open(path).map_err(|e| TagWriteError::EpubFailed {
        message: format!("open failed: {e}"),
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| TagWriteError::EpubFailed {
        message: format!("invalid EPUB: {e}"),
    })?;

    let opf_path = find_opf_path(&mut archive)?;

    let opf_content = {
        let mut opf_file = archive
            .by_name(&opf_path)
            .map_err(|e| TagWriteError::EpubFailed {
                message: format!("OPF not found: {e}"),
            })?;
        let mut content = String::new();
        opf_file
            .read_to_string(&mut content)
            .map_err(|e| TagWriteError::EpubFailed {
                message: format!("OPF read failed: {e}"),
            })?;
        content
    };

    let new_opf = update_opf_metadata(&opf_content, metadata);

    // Rewrite EPUB via internal temp (same-dir for atomic rename).
    let tmp_path = path.with_extension("epub.tagwrite");
    let write_result = (|| -> Result<(), TagWriteError> {
        let tmp_file = std::fs::File::create(&tmp_path).map_err(|e| TagWriteError::EpubFailed {
            message: format!("temp create failed: {e}"),
        })?;
        let mut writer = zip::ZipWriter::new(tmp_file);

        let cover_name = "OEBPS/images/cover.jpg";

        // Read existing cover bytes for dedup comparison (TAG-V21-007).
        let existing_cover: Option<Vec<u8>> = (0..archive.len()).find_map(|i| {
            let mut entry = archive.by_index(i).ok()?;
            if entry.name() == cover_name {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf).ok()?;
                Some(buf)
            } else {
                None
            }
        });

        // Determine if we need to replace the cover.
        let should_replace_cover = match (cover, &existing_cover) {
            (Some(new_bytes), Some(old_bytes)) => new_bytes != old_bytes.as_slice(),
            (Some(_), None) => true, // no existing cover, add new
            _ => false,              // no new cover provided
        };

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| TagWriteError::EpubFailed {
                message: format!("ZIP entry error: {e}"),
            })?;
            let name = entry.name().to_string();

            if name == opf_path {
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated);
                writer
                    .start_file(&name, options)
                    .map_err(|e| TagWriteError::EpubFailed {
                        message: format!("ZIP write error: {e}"),
                    })?;
                writer
                    .write_all(new_opf.as_bytes())
                    .map_err(|e| TagWriteError::Io {
                        message: e.to_string(),
                    })?;
            } else if name == cover_name && should_replace_cover {
                // Skip old cover — we'll write the new one after the loop.
                continue;
            } else {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf).map_err(|e| TagWriteError::Io {
                    message: e.to_string(),
                })?;
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(entry.compression());
                writer
                    .start_file(&name, options)
                    .map_err(|e| TagWriteError::EpubFailed {
                        message: format!("ZIP write error: {e}"),
                    })?;
                writer.write_all(&buf).map_err(|e| TagWriteError::Io {
                    message: e.to_string(),
                })?;
            }
        }

        // Write new cover if needed.
        if should_replace_cover {
            if let Some(cover_bytes) = cover {
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored);
                writer
                    .start_file(cover_name, options)
                    .map_err(|e| TagWriteError::EpubFailed {
                        message: format!("ZIP write error: {e}"),
                    })?;
                writer
                    .write_all(cover_bytes)
                    .map_err(|e| TagWriteError::Io {
                        message: e.to_string(),
                    })?;
            }
        }

        writer.finish().map_err(|e| TagWriteError::EpubFailed {
            message: format!("ZIP finish error: {e}"),
        })?;
        Ok(())
    })();

    match write_result {
        Ok(()) => {
            // EPUB rewrite succeeded — replace original with rewritten file.
            std::fs::rename(&tmp_path, path).map_err(|e| {
                let _ = std::fs::remove_file(&tmp_path);
                TagWriteError::Io {
                    message: format!("EPUB rename failed: {e}"),
                }
            })?;
            Ok(())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp_path);
            Err(e)
        }
    }
}

fn find_opf_path(archive: &mut zip::ZipArchive<std::fs::File>) -> Result<String, TagWriteError> {
    use std::io::Read;

    let mut container =
        archive
            .by_name("META-INF/container.xml")
            .map_err(|_| TagWriteError::EpubFailed {
                message: "container.xml not found".into(),
            })?;
    let mut content = String::new();
    container
        .read_to_string(&mut content)
        .map_err(|e| TagWriteError::EpubFailed {
            message: format!("container.xml read failed: {e}"),
        })?;

    let re = regex_lite::Regex::new(r#"full-path="([^"]+)""#).unwrap();
    re.captures(&content)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .ok_or_else(|| TagWriteError::EpubFailed {
            message: "no rootfile in container.xml".into(),
        })
}

fn update_opf_metadata(opf: &str, data: &TagMetadata) -> String {
    let mut result = opf.to_string();

    let mut dc_elements = Vec::new();

    dc_elements.push(format!(
        "    <dc:title>{}</dc:title>",
        xml_escape(&data.title)
    ));
    dc_elements.push(format!(
        "    <dc:creator>{}</dc:creator>",
        xml_escape(&data.author)
    ));

    if let Some(ref isbn) = data.isbn {
        dc_elements.push(format!(
            "    <dc:identifier id=\"isbn\">urn:isbn:{}</dc:identifier>",
            xml_escape(isbn)
        ));
    }
    if let Some(ref desc) = data.description {
        dc_elements.push(format!(
            "    <dc:description>{}</dc:description>",
            xml_escape(desc)
        ));
    }
    if let Some(ref publisher) = data.publisher {
        dc_elements.push(format!(
            "    <dc:publisher>{}</dc:publisher>",
            xml_escape(publisher)
        ));
    }
    if let Some(year) = data.year {
        dc_elements.push(format!("    <dc:date>{year}</dc:date>"));
    }
    if let Some(ref lang) = data.language {
        dc_elements.push(format!(
            "    <dc:language>{}</dc:language>",
            xml_escape(lang)
        ));
    }
    if let Some(ref genres) = data.genre {
        for g in genres {
            dc_elements.push(format!("    <dc:subject>{}</dc:subject>", xml_escape(g)));
        }
    }

    if let Some(ref series) = data.series_name {
        dc_elements.push(format!(
            "    <meta name=\"calibre:series\" content=\"{}\"/>",
            xml_escape(series)
        ));
        if let Some(pos) = data.series_position {
            dc_elements.push(format!(
                "    <meta name=\"calibre:series_index\" content=\"{pos}\"/>"
            ));
        }
    }

    let new_metadata = dc_elements.join("\n");

    let re_metadata = regex_lite::Regex::new(r"(?s)<metadata[^>]*>(.*?)</metadata>").unwrap();
    if let Some(cap) = re_metadata.captures(&result) {
        let existing = &cap[1];

        let re_dc = regex_lite::Regex::new(
            r"(?m)^\s*<dc:(title|creator|identifier|description|publisher|date|language|subject)[^>]*>.*?</dc:\w+>\s*$",
        )
        .unwrap();
        let re_calibre = regex_lite::Regex::new(
            r#"(?m)^\s*<meta\s+name="calibre:(series|series_index)"[^/]*/>\s*$"#,
        )
        .unwrap();

        let cleaned = re_dc.replace_all(existing, "").to_string();
        let cleaned = re_calibre.replace_all(&cleaned, "").to_string();

        let new_block = format!("{}\n{}", new_metadata, cleaned.trim());
        let full_match = &cap[0];
        let tag_start = &full_match[..full_match.find('>').unwrap() + 1];
        result = re_metadata
            .replace(
                &result,
                format!("{}\n{}\n  </metadata>", tag_start, new_block),
            )
            .to_string();
    }

    result
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

// ---------------------------------------------------------------------------
// M4B (via mp4ameta)
// ---------------------------------------------------------------------------

fn write_m4b(
    path: &Path,
    metadata: &TagMetadata,
    cover: Option<&[u8]>,
) -> Result<(), TagWriteError> {
    let mut tag = mp4ameta::Tag::read_from_path(path).map_err(|e| TagWriteError::M4bFailed {
        message: format!("read failed: {e}"),
    })?;

    tag.set_title(&metadata.title);
    tag.set_album(&metadata.title);
    tag.set_artist(&metadata.author);

    if let Some(ref narrators) = metadata.narrator {
        let joined = narrators.join(", ");
        if !joined.is_empty() {
            tag.set_album_artist(&joined);
            tag.set_composer(&joined);
        }
    }

    if let Some(year) = metadata.year {
        tag.set_year(year.to_string());
    }
    if let Some(ref genres) = metadata.genre {
        if let Some(g) = genres.first() {
            tag.set_genre(g);
        }
    }
    if let Some(ref desc) = metadata.description {
        tag.set_comment(desc);
    }

    // Series metadata (TAG-004: grouping for series).
    if let Some(ref series) = metadata.series_name {
        tag.set_grouping(series);
    }

    // Cover dedup (TAG-V21-007): only set if different from existing.
    if let Some(cover_bytes) = cover {
        let existing_cover = tag.artwork().map(|img| img.data.to_vec());
        if existing_cover.as_deref() != Some(cover_bytes) {
            tag.set_artwork(mp4ameta::Img::jpeg(cover_bytes.to_vec()));
        }
    }

    tag.write_to_path(path)
        .map_err(|e| TagWriteError::M4bFailed {
            message: format!("write failed: {e}"),
        })?;

    Ok(())
}

// ---------------------------------------------------------------------------
// MP3 (via id3)
// ---------------------------------------------------------------------------

fn write_mp3(
    path: &Path,
    metadata: &TagMetadata,
    cover: Option<&[u8]>,
) -> Result<(), TagWriteError> {
    use id3::TagLike;

    let mut tag = id3::Tag::read_from_path(path).unwrap_or_else(|_| id3::Tag::new());

    // Preserve existing TRCK (TAG-004/005).
    let existing_track = tag.track();

    tag.set_title(&metadata.title);
    tag.set_album(&metadata.title);
    tag.set_artist(&metadata.author);

    if let Some(ref narrators) = metadata.narrator {
        let joined = narrators.join(", ");
        if !joined.is_empty() {
            tag.set_album_artist(&joined);
            // Clear existing TCOM frames before adding to prevent duplicates on re-enrichment.
            tag.remove("TCOM");
            tag.add_frame(id3::frame::Frame::text("TCOM", &joined));
        }
    }

    if let Some(year) = metadata.year {
        tag.set_year(year);
    }
    if let Some(ref desc) = metadata.description {
        // Clear existing COMM frames before adding to prevent duplicates on re-enrichment.
        tag.remove("COMM");
        tag.add_frame(id3::frame::Comment {
            lang: "eng".to_string(),
            description: String::new(),
            text: desc.clone(),
        });
    }
    if let Some(ref genres) = metadata.genre {
        if let Some(g) = genres.first() {
            tag.set_genre(g);
        }
    }

    // Cover dedup (TAG-V21-007): only replace if different.
    if let Some(cover_bytes) = cover {
        let existing = tag
            .pictures()
            .find(|p| p.picture_type == id3::frame::PictureType::CoverFront)
            .map(|p| p.data.clone());
        if existing.as_deref() != Some(cover_bytes) {
            tag.remove_picture_by_type(id3::frame::PictureType::CoverFront);
            tag.add_frame(id3::frame::Picture {
                mime_type: "image/jpeg".to_string(),
                picture_type: id3::frame::PictureType::CoverFront,
                description: String::new(),
                data: cover_bytes.to_vec(),
            });
        }
    }

    // Restore preserved track number.
    if let Some(track) = existing_track {
        tag.set_track(track);
    }

    tag.write_to_path(path, id3::Version::Id3v24)
        .map_err(|e| TagWriteError::Mp3Failed {
            message: format!("write failed: {e}"),
        })?;

    Ok(())
}
