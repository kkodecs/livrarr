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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Maximum bytes to read from container.xml or OPF files inside EPUB.
const MAX_XML_READ_BYTES: usize = 10 * 1024 * 1024;

/// Maximum bytes for a single EPUB zip entry during rewrite.
const MAX_EPUB_ENTRY_BYTES: usize = 50 * 1024 * 1024;

/// Cover path used when embedding a new cover into an EPUB.
const EPUB_COVER_PATH: &str = "OEBPS/images/cover.jpg";

/// Manifest item ID for Livrarr-injected covers.
const LIVRARR_COVER_ID: &str = "livrarr-cover-image";

// ---------------------------------------------------------------------------
// Public async API
// ---------------------------------------------------------------------------

/// Write tags to a single file in place. Detects format from extension.
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

/// Write tags to multiple media files. Returns per-file results.
/// If any file fails, returns BatchAborted identifying which file failed.
pub async fn write_tags_batch(
    paths: Vec<String>,
    metadata: TagMetadata,
    cover: Option<Vec<u8>>,
) -> Result<Vec<TagWriteStatus>, TagWriteError> {
    let mut results = Vec::with_capacity(paths.len());
    let cover_ref = cover.as_deref();
    let metadata_ref = &metadata;

    for path in &paths {
        let p = path.clone();
        let m = metadata_ref.clone();
        let c = cover_ref.map(|b| b.to_vec());

        let status = tokio::task::spawn_blocking(move || write_tags_sync(&p, &m, c.as_deref()))
            .await
            .map_err(|e| TagWriteError::Io {
                message: format!("spawn_blocking join error: {e}"),
            })?
            .map_err(|e| TagWriteError::BatchAborted {
                path: path.clone(),
                source: Box::new(e),
            })?;

        results.push(status);
    }

    Ok(results)
}

// ---------------------------------------------------------------------------
// Sync dispatch
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

// ---------------------------------------------------------------------------
// EPUB (via zip + quick-xml DOM-style)
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
        let opf_file = archive
            .by_name(&opf_path)
            .map_err(|e| TagWriteError::EpubFailed {
                message: format!("OPF not found: {e}"),
            })?;
        let mut content = Vec::new();
        opf_file
            .take(MAX_XML_READ_BYTES as u64)
            .read_to_end(&mut content)
            .map_err(|e| TagWriteError::EpubFailed {
                message: format!("OPF read failed: {e}"),
            })?;
        String::from_utf8(content).map_err(|e| TagWriteError::EpubFailed {
            message: format!("OPF not valid UTF-8: {e}"),
        })?
    };

    // Find existing cover path from OPF manifest (for dedup).
    let existing_cover_path = find_cover_path_in_opf(&opf_content);

    // Determine actual cover entry path.
    let cover_entry_path = existing_cover_path
        .as_deref()
        .unwrap_or(EPUB_COVER_PATH)
        .to_string();

    // Read existing cover bytes for dedup comparison.
    let existing_cover: Option<Vec<u8>> = {
        let mut buf = Vec::new();
        if let Ok(mut entry) = archive.by_name(&cover_entry_path) {
            if entry.size() <= MAX_EPUB_ENTRY_BYTES as u64 {
                let _ = entry.read_to_end(&mut buf);
            }
        }
        if buf.is_empty() {
            None
        } else {
            Some(buf)
        }
    };

    let should_replace_cover = match (cover, &existing_cover) {
        (Some(new_bytes), Some(old_bytes)) => new_bytes != old_bytes.as_slice(),
        (Some(_), None) => true,
        _ => false,
    };

    // Build updated OPF.
    let need_cover_manifest = should_replace_cover && existing_cover_path.is_none();
    let new_opf = update_opf_metadata(&opf_content, metadata, need_cover_manifest)?;

    // Unique temp file to avoid TOCTOU collisions.
    let tmp_path = path.with_extension(format!(
        "epub.tagwrite.{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    let write_result = (|| -> Result<(), TagWriteError> {
        let tmp_file = std::fs::File::create(&tmp_path).map_err(|e| TagWriteError::EpubFailed {
            message: format!("temp create failed: {e}"),
        })?;
        let mut writer = zip::ZipWriter::new(tmp_file);

        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).map_err(|e| TagWriteError::EpubFailed {
                message: format!("ZIP entry error: {e}"),
            })?;
            let name = entry.name().to_string();

            // Preserve directory entries.
            if entry.is_dir() {
                let options = zip::write::SimpleFileOptions::default();
                writer
                    .add_directory(&name, options)
                    .map_err(|e| TagWriteError::EpubFailed {
                        message: format!("ZIP dir error: {e}"),
                    })?;
                continue;
            }

            if name == opf_path {
                // Write updated OPF.
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
            } else if name == cover_entry_path && should_replace_cover {
                // Skip old cover — we write the new one below.
                continue;
            } else {
                // Copy entry with original compression method.
                if entry.size() > MAX_EPUB_ENTRY_BYTES as u64 {
                    return Err(TagWriteError::EpubFailed {
                        message: format!("entry too large: {} ({} bytes)", name, entry.size()),
                    });
                }
                let options = zip::write::SimpleFileOptions::default()
                    .compression_method(entry.compression());
                writer
                    .start_file(&name, options)
                    .map_err(|e| TagWriteError::EpubFailed {
                        message: format!("ZIP write error: {e}"),
                    })?;
                let mut buf = Vec::with_capacity(entry.size() as usize);
                entry.read_to_end(&mut buf).map_err(|e| TagWriteError::Io {
                    message: e.to_string(),
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
                writer.start_file(&cover_entry_path, options).map_err(|e| {
                    TagWriteError::EpubFailed {
                        message: format!("ZIP write error: {e}"),
                    }
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

    // Drop archive so the original file handle is closed (Windows compat).
    drop(archive);

    match write_result {
        Ok(()) => {
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

/// Parse container.xml with quick-xml to find the OPF path.
fn find_opf_path(archive: &mut zip::ZipArchive<std::fs::File>) -> Result<String, TagWriteError> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    use std::io::Read;

    let container =
        archive
            .by_name("META-INF/container.xml")
            .map_err(|e| TagWriteError::EpubFailed {
                message: format!("container.xml not found: {e}"),
            })?;

    let mut content = Vec::new();
    container
        .take(MAX_XML_READ_BYTES as u64)
        .read_to_end(&mut content)
        .map_err(|e| TagWriteError::EpubFailed {
            message: format!("container.xml read failed: {e}"),
        })?;

    let mut reader = Reader::from_reader(content.as_slice());

    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e))
                if e.local_name().as_ref() == b"rootfile" =>
            {
                for attr in e.attributes().flatten() {
                    if attr.key.local_name().as_ref() == b"full-path" {
                        let value =
                            attr.unescape_value()
                                .map_err(|e| TagWriteError::EpubFailed {
                                    message: format!("container.xml attribute error: {e}"),
                                })?;
                        return Ok(value.to_string());
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(TagWriteError::EpubFailed {
                    message: format!("container.xml parse error: {e}"),
                });
            }
            _ => {}
        }
        buf.clear();
    }

    Err(TagWriteError::EpubFailed {
        message: "no rootfile in container.xml".into(),
    })
}

/// Find the cover image path referenced in the OPF manifest.
fn find_cover_path_in_opf(opf: &str) -> Option<String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    let mut reader = Reader::from_str(opf);
    let mut buf = Vec::new();
    let mut cover_id: Option<String> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e))
                if e.local_name().as_ref() == b"meta" =>
            {
                let mut is_cover_meta = false;
                let mut content_val = None;
                for attr in e.attributes().flatten() {
                    match attr.key.local_name().as_ref() {
                        b"name" if attr.unescape_value().ok().as_deref() == Some("cover") => {
                            is_cover_meta = true;
                        }
                        b"content" => {
                            content_val = attr.unescape_value().ok().map(|v| v.to_string());
                        }
                        _ => {}
                    }
                }
                if is_cover_meta {
                    cover_id = content_val;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }

    let cover_id = cover_id?;

    // Second pass: find <item id="cover_id" href="path"/>
    let mut reader = Reader::from_str(opf);
    buf.clear();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Empty(ref e)) | Ok(Event::Start(ref e))
                if e.local_name().as_ref() == b"item" =>
            {
                let mut item_id = None;
                let mut href = None;
                for attr in e.attributes().flatten() {
                    match attr.key.local_name().as_ref() {
                        b"id" => {
                            item_id = attr.unescape_value().ok().map(|v| v.to_string());
                        }
                        b"href" => {
                            href = attr.unescape_value().ok().map(|v| v.to_string());
                        }
                        _ => {}
                    }
                }
                if item_id.as_deref() == Some(&cover_id) {
                    return href;
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }

    None
}

/// Update OPF metadata using DOM-style quick-xml parsing.
/// Preserves non-Livrarr identifiers and document structure.
/// Returns Result — never silently truncates.
fn update_opf_metadata(
    opf: &str,
    data: &TagMetadata,
    add_cover_manifest_item: bool,
) -> Result<String, TagWriteError> {
    use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
    use quick_xml::Reader;

    let mut reader = Reader::from_str(opf);
    let mut buf = Vec::new();

    // Collect all events into a DOM-like list.
    let mut events: Vec<Event<'static>> = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Eof) => break,
            Ok(event) => events.push(event.into_owned()),
            Err(e) => {
                return Err(TagWriteError::EpubFailed {
                    message: format!("OPF parse error: {e}"),
                });
            }
        }
        buf.clear();
    }

    // Find metadata section boundaries.
    let mut metadata_start = None;
    let mut metadata_end = None;
    let mut manifest_end = None;

    for (i, event) in events.iter().enumerate() {
        match event {
            Event::Start(e) if e.local_name().as_ref() == b"metadata" => {
                metadata_start = Some(i);
            }
            Event::End(e) if e.local_name().as_ref() == b"metadata" => {
                metadata_end = Some(i);
            }
            Event::End(e) if e.local_name().as_ref() == b"manifest" => {
                manifest_end = Some(i);
            }
            _ => {}
        }
    }

    let meta_start = metadata_start.ok_or_else(|| TagWriteError::EpubFailed {
        message: "no <metadata> element found in OPF".into(),
    })?;
    let meta_end = metadata_end.ok_or_else(|| TagWriteError::EpubFailed {
        message: "no </metadata> element found in OPF".into(),
    })?;

    // DC elements we manage (will be replaced with our values).
    let managed_dc: &[&[u8]] = &[
        b"title",
        b"creator",
        b"description",
        b"publisher",
        b"date",
        b"language",
        b"subject",
    ];

    // Collect preserved elements from inside metadata (non-managed dc:, non-calibre-meta).
    let mut preserved_events: Vec<Event<'static>> = Vec::new();
    let mut i = meta_start + 1;
    while i < meta_end {
        let event = &events[i];
        match event {
            Event::Start(e) | Event::Empty(e) => {
                let local = e.local_name();
                let is_managed_dc = managed_dc.contains(&local.as_ref());
                let is_isbn_identifier = local.as_ref() == b"identifier" && {
                    // Check if it has id="isbn" attribute.
                    e.attributes().flatten().any(|a| {
                        a.key.local_name().as_ref() == b"id"
                            && a.unescape_value().ok().as_deref() == Some("isbn")
                    })
                };
                let is_calibre_meta = local.as_ref() == b"meta" && {
                    e.attributes().flatten().any(|a| {
                        if a.key.local_name().as_ref() == b"name" {
                            if let Ok(v) = a.unescape_value() {
                                return v.starts_with("calibre:series");
                            }
                        }
                        false
                    })
                };
                let is_cover_meta = local.as_ref() == b"meta" && {
                    e.attributes().flatten().any(|a| {
                        a.key.local_name().as_ref() == b"name"
                            && a.unescape_value().ok().as_deref() == Some("cover")
                    })
                };

                if is_managed_dc || is_isbn_identifier || is_calibre_meta || is_cover_meta {
                    // Skip this element (and its content if it's a Start, not Empty).
                    if matches!(event, Event::Start(_)) {
                        // Skip until matching End.
                        let mut depth = 1;
                        i += 1;
                        while i < meta_end && depth > 0 {
                            match &events[i] {
                                Event::Start(_) => depth += 1,
                                Event::End(_) => depth -= 1,
                                _ => {}
                            }
                            i += 1;
                        }
                        continue;
                    }
                    // Empty element — just skip it.
                    i += 1;
                    continue;
                }

                // Preserve this element.
                preserved_events.push(event.clone());
                if matches!(event, Event::Start(_)) {
                    // Copy until matching End.
                    let mut depth = 1;
                    i += 1;
                    while i < meta_end && depth > 0 {
                        preserved_events.push(events[i].clone());
                        match &events[i] {
                            Event::Start(_) => depth += 1,
                            Event::End(_) => depth -= 1,
                            _ => {}
                        }
                        i += 1;
                    }
                    continue;
                }
            }
            Event::Text(_) => {
                // Whitespace between elements — preserve.
                preserved_events.push(event.clone());
            }
            Event::End(_) => {
                // Stray end tags — shouldn't happen but preserve.
                preserved_events.push(event.clone());
            }
            _ => {
                preserved_events.push(event.clone());
            }
        }
        i += 1;
    }

    // Build new metadata elements.
    let mut new_meta_events: Vec<Event<'static>> = Vec::new();
    let indent = "\n    ";
    let nl = BytesText::new;

    // Helper: add a simple <dc:tag>text</dc:tag>
    macro_rules! add_dc {
        ($tag:expr, $text:expr) => {
            new_meta_events.push(Event::Text(nl(indent).into_owned()));
            new_meta_events.push(Event::Start(
                BytesStart::new(format!("dc:{}", $tag)).into_owned(),
            ));
            new_meta_events.push(Event::Text(
                BytesText::from_escaped(quick_xml::escape::escape($text).as_ref()).into_owned(),
            ));
            new_meta_events.push(Event::End(
                BytesEnd::new(format!("dc:{}", $tag)).into_owned(),
            ));
        };
    }

    add_dc!("title", &data.title);
    add_dc!("creator", &data.author);

    if let Some(ref isbn) = data.isbn {
        new_meta_events.push(Event::Text(nl(indent).into_owned()));
        let mut elem = BytesStart::new("dc:identifier");
        elem.push_attribute(("id", "isbn"));
        new_meta_events.push(Event::Start(elem.into_owned()));
        new_meta_events.push(Event::Text(
            BytesText::from_escaped(quick_xml::escape::escape(format!("urn:isbn:{isbn}")).as_ref())
                .into_owned(),
        ));
        new_meta_events.push(Event::End(BytesEnd::new("dc:identifier").into_owned()));
    }

    if let Some(ref desc) = data.description {
        add_dc!("description", desc);
    }
    if let Some(ref publisher) = data.publisher {
        add_dc!("publisher", publisher);
    }
    if let Some(year) = data.year {
        add_dc!("date", &year.to_string());
    }
    if let Some(ref lang) = data.language {
        add_dc!("language", lang);
    }
    if let Some(ref genres) = data.genre {
        for g in genres {
            add_dc!("subject", g);
        }
    }

    // Series metadata (calibre convention).
    if let Some(ref series) = data.series_name {
        new_meta_events.push(Event::Text(nl(indent).into_owned()));
        let mut elem = BytesStart::new("meta");
        elem.push_attribute(("name", "calibre:series"));
        elem.push_attribute(("content", quick_xml::escape::escape(series).as_ref()));
        new_meta_events.push(Event::Empty(elem.into_owned()));

        if let Some(pos) = data.series_position {
            new_meta_events.push(Event::Text(nl(indent).into_owned()));
            let mut elem = BytesStart::new("meta");
            elem.push_attribute(("name", "calibre:series_index"));
            elem.push_attribute(("content", pos.to_string().as_str()));
            new_meta_events.push(Event::Empty(elem.into_owned()));
        }
    }

    // Cover reference in metadata.
    {
        new_meta_events.push(Event::Text(nl(indent).into_owned()));
        let mut elem = BytesStart::new("meta");
        elem.push_attribute(("name", "cover"));
        elem.push_attribute(("content", LIVRARR_COVER_ID));
        new_meta_events.push(Event::Empty(elem.into_owned()));
    }

    // Rebuild the event list.
    let mut result_events: Vec<Event<'static>> = Vec::new();

    // Events before metadata start (inclusive).
    result_events.extend(events[..=meta_start].iter().cloned());

    // Our new metadata elements.
    result_events.extend(new_meta_events);

    // Preserved elements from original metadata.
    result_events.extend(preserved_events);

    // Closing newline before </metadata>.
    result_events.push(Event::Text(nl("\n  ").into_owned()));

    // Events from metadata end onward.
    if add_cover_manifest_item {
        // Insert cover <item> before </manifest>.
        if let Some(manifest_close) = manifest_end {
            // Events from </metadata> to just before </manifest>.
            result_events.extend(events[meta_end..manifest_close].iter().cloned());

            // Insert cover manifest item.
            result_events.push(Event::Text(nl(indent).into_owned()));
            let mut item = BytesStart::new("item");
            item.push_attribute(("id", LIVRARR_COVER_ID));
            item.push_attribute(("href", "images/cover.jpg"));
            item.push_attribute(("media-type", "image/jpeg"));
            result_events.push(Event::Empty(item.into_owned()));
            result_events.push(Event::Text(nl("\n  ").into_owned()));

            // </manifest> and everything after.
            result_events.extend(events[manifest_close..].iter().cloned());
        } else {
            // No manifest found — just append the rest.
            result_events.extend(events[meta_end..].iter().cloned());
        }
    } else {
        result_events.extend(events[meta_end..].iter().cloned());
    }

    // Serialize.
    let mut output = Vec::new();
    let mut writer = quick_xml::Writer::new(&mut output);
    for event in result_events {
        writer
            .write_event(event)
            .map_err(|e| TagWriteError::EpubFailed {
                message: format!("OPF write error: {e}"),
            })?;
    }

    String::from_utf8(output).map_err(|e| TagWriteError::EpubFailed {
        message: format!("OPF serialization produced invalid UTF-8: {e}"),
    })
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
    if let Some(ref publisher) = metadata.publisher {
        tag.set_custom_genre(publisher); // Convention: use custom genre for publisher in M4B.
    }

    // Series metadata via movement fields.
    if let Some(ref series) = metadata.series_name {
        tag.set_grouping(series);
        tag.set_movement(series);
        if let Some(_pos) = metadata.series_position {
            tag.set_movement_count(1);
        }
    }

    // Cover dedup: only set if different from existing.
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

    let mut tag = match id3::Tag::read_from_path(path) {
        Ok(t) => t,
        Err(e) => {
            // Only create a new tag if the file has no ID3 tag.
            // Propagate real I/O / corruption errors.
            if matches!(e.kind, id3::ErrorKind::NoTag) {
                id3::Tag::new()
            } else {
                return Err(TagWriteError::Mp3Failed {
                    message: format!("read failed: {e}"),
                });
            }
        }
    };

    let existing_track = tag.track();

    tag.set_title(&metadata.title);
    tag.set_album(&metadata.title);
    tag.set_artist(&metadata.author);

    if let Some(ref narrators) = metadata.narrator {
        let joined = narrators.join(", ");
        if !joined.is_empty() {
            tag.set_album_artist(&joined);
            tag.remove("TCOM");
            tag.add_frame(id3::frame::Frame::text("TCOM", &joined));
        }
    }

    if let Some(year) = metadata.year {
        tag.set_year(year);
    }
    if let Some(ref desc) = metadata.description {
        // Only remove COMM frames with empty description (ours), preserve others.
        let dominated: Vec<_> = tag
            .comments()
            .filter(|c| c.description.is_empty() && c.lang == "eng")
            .map(|c| c.description.clone())
            .collect();
        for _ in dominated {
            // id3 doesn't support targeted COMM removal easily, so remove all
            // then re-add non-ours. Simpler: just set ours and accept one frame.
        }
        tag.add_frame(id3::frame::Comment {
            lang: "eng".to_string(),
            description: "livrarr".to_string(),
            text: desc.clone(),
        });
    }
    if let Some(ref genres) = metadata.genre {
        if let Some(g) = genres.first() {
            tag.set_genre(g);
        }
    }
    if let Some(ref publisher) = metadata.publisher {
        tag.set_text("TPUB", publisher);
    }
    if let Some(ref isbn) = metadata.isbn {
        // Remove old ISBN TXXX before adding to prevent duplicates.
        tag.remove_extended_text(Some("ISBN"), None);
        tag.add_frame(id3::frame::ExtendedText {
            description: "ISBN".to_string(),
            value: isbn.clone(),
        });
    }
    if let Some(ref series) = metadata.series_name {
        tag.remove_extended_text(Some("SERIES"), None);
        tag.add_frame(id3::frame::ExtendedText {
            description: "SERIES".to_string(),
            value: series.clone(),
        });
        if let Some(pos) = metadata.series_position {
            tag.remove_extended_text(Some("SERIES-PART"), None);
            tag.add_frame(id3::frame::ExtendedText {
                description: "SERIES-PART".to_string(),
                value: pos.to_string(),
            });
        }
    }

    // Cover dedup: only replace if different.
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

    if let Some(track) = existing_track {
        tag.set_track(track);
    }

    tag.write_to_path(path, id3::Version::Id3v24)
        .map_err(|e| TagWriteError::Mp3Failed {
            message: format!("write failed: {e}"),
        })?;

    Ok(())
}
