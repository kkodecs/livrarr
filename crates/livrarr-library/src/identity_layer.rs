//! Identity-layer-rewrite (F2) local EPUB cover inspector. IR v1
//! `livrarr-library` module (ir-v1-identity-layer-rewrite.yaml:1294-1310).
//! Inspects owned EPUB revisions in `spawn_blocking` with zero provider
//! traffic (PROBE-ST007-EPUB).

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use livrarr_db::sqlite::SqliteDb;
use livrarr_domain::identity_layer::{
    EmbeddedCoverInspectionOutcome, EmbeddedCoverInspectionRecord, EmbeddedCoverInspectionResult,
    EmbeddedCoverInspector, FileRevision, InspectionError, InspectionServiceError,
};
use livrarr_domain::LibraryItem;

#[derive(Clone)]
pub struct EpubCoverInspector {
    db: SqliteDb,
    pub max_uncompressed_bytes: u64,
    pub max_entries: u32,
    inspection_attempts: Arc<AtomicU64>,
}

impl EpubCoverInspector {
    pub fn new(db: SqliteDb, max_uncompressed_bytes: u64, max_entries: u32) -> Self {
        Self {
            db,
            max_uncompressed_bytes,
            max_entries,
            inspection_attempts: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Number of actual blocking file inspections. Durable cache hits do not
    /// increment this counter, so callers can verify retry suppression.
    pub fn inspection_attempt_count(&self) -> u64 {
        self.inspection_attempts.load(Ordering::SeqCst)
    }

    /// Reads the durable exact-revision record first, performs zero
    /// provider requests, and persists Extracted/VerifiedNoCover/
    /// CouldNotInspect/FileGone before return.
    pub async fn inspect_revision(
        &self,
        item: LibraryItem,
        revision: FileRevision,
        force: bool,
    ) -> Result<EmbeddedCoverInspectionResult, InspectionServiceError> {
        if let Some(record) = self
            .db
            .read_embedded_cover_inspection(item.user_id, item.id, revision)
            .await
            .map_err(inspection_db_error)?
        {
            match record.outcome {
                EmbeddedCoverInspectionOutcome::VerifiedNoCover => {
                    return Ok(EmbeddedCoverInspectionResult::VerifiedNoCover { revision });
                }
                EmbeddedCoverInspectionOutcome::CouldNotInspect if !force => {
                    return Ok(EmbeddedCoverInspectionResult::CouldNotInspect {
                        revision,
                        error: InspectionError(
                            record
                                .sanitized_error_code
                                .unwrap_or_else(|| "inspection_failed".to_string()),
                        ),
                    });
                }
                EmbeddedCoverInspectionOutcome::FileGone if !force => {
                    return Ok(EmbeddedCoverInspectionResult::FileGone);
                }
                EmbeddedCoverInspectionOutcome::Extracted
                | EmbeddedCoverInspectionOutcome::CouldNotInspect
                | EmbeddedCoverInspectionOutcome::FileGone => {}
            }
        }

        let path = PathBuf::from(&item.path);
        let max_uncompressed_bytes = self.max_uncompressed_bytes;
        let max_entries = self.max_entries;
        self.inspection_attempts.fetch_add(1, Ordering::SeqCst);
        let inspected = tokio::task::spawn_blocking(move || {
            inspect_epub(path, revision, max_uncompressed_bytes, max_entries)
        })
        .await
        .map_err(|error| InspectionServiceError::Database(format!("inspection_task: {error}")))?;

        let record = inspection_record(&item, revision, &inspected);
        self.db
            .record_embedded_cover_inspection(record)
            .await
            .map_err(inspection_db_error)?;
        Ok(inspected)
    }
}

impl EmbeddedCoverInspector for EpubCoverInspector {
    async fn inspect_revision(
        &self,
        item: LibraryItem,
        revision: FileRevision,
        force: bool,
    ) -> Result<EmbeddedCoverInspectionResult, InspectionServiceError> {
        EpubCoverInspector::inspect_revision(self, item, revision, force).await
    }
}

fn inspection_db_error(
    error: livrarr_domain::identity_layer::IdentityRepositoryError,
) -> InspectionServiceError {
    InspectionServiceError::Database(error.to_string())
}

fn inspection_record(
    item: &LibraryItem,
    revision: FileRevision,
    result: &EmbeddedCoverInspectionResult,
) -> EmbeddedCoverInspectionRecord {
    let (outcome, sanitized_error_code) = match result {
        EmbeddedCoverInspectionResult::Extracted { .. } => {
            (EmbeddedCoverInspectionOutcome::Extracted, None)
        }
        EmbeddedCoverInspectionResult::VerifiedNoCover { .. } => {
            (EmbeddedCoverInspectionOutcome::VerifiedNoCover, None)
        }
        EmbeddedCoverInspectionResult::CouldNotInspect { error, .. } => (
            EmbeddedCoverInspectionOutcome::CouldNotInspect,
            Some(error.0.clone()),
        ),
        EmbeddedCoverInspectionResult::FileGone => (EmbeddedCoverInspectionOutcome::FileGone, None),
    };
    EmbeddedCoverInspectionRecord {
        user_id: item.user_id,
        library_item_id: item.id,
        revision,
        outcome,
        cover_candidate_id: None,
        sanitized_error_code,
        inspected_at: chrono::Utc::now(),
    }
}

fn inspect_epub(
    path: PathBuf,
    revision: FileRevision,
    max_uncompressed_bytes: u64,
    max_entries: u32,
) -> EmbeddedCoverInspectionResult {
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return EmbeddedCoverInspectionResult::FileGone;
        }
        Err(_) => return inspection_failure(revision, "file_unreadable"),
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(archive) => archive,
        Err(_) => return inspection_failure(revision, "invalid_zip"),
    };
    if archive.len() > max_entries as usize {
        return inspection_failure(revision, "archive_entry_limit");
    }
    let total_uncompressed = (0..archive.len()).try_fold(0u64, |total, index| {
        archive
            .by_index_raw(index)
            .ok()
            .and_then(|entry| total.checked_add(entry.size()))
    });
    if total_uncompressed.is_none_or(|total| total > max_uncompressed_bytes) {
        return inspection_failure(revision, "archive_size_limit");
    }

    let container = match read_zip_member(
        &mut archive,
        "META-INF/container.xml",
        max_uncompressed_bytes,
    ) {
        Ok(bytes) => bytes,
        Err(code) => return inspection_failure(revision, code),
    };
    let opf_path = match parse_container_path(&container) {
        Ok(path) => path,
        Err(code) => return inspection_failure(revision, code),
    };
    let opf = match read_zip_member(&mut archive, &opf_path, max_uncompressed_bytes) {
        Ok(bytes) => bytes,
        Err(code) => return inspection_failure(revision, code),
    };
    let declaration = match parse_cover_declaration(&opf) {
        Ok(Some(declaration)) => declaration,
        Ok(None) => return EmbeddedCoverInspectionResult::VerifiedNoCover { revision },
        Err(code) => return inspection_failure(revision, code),
    };
    let cover_path = match resolve_archive_member(&opf_path, &declaration.href) {
        Some(path) => path,
        None => return inspection_failure(revision, "unsafe_cover_path"),
    };
    let bytes = match read_zip_member(&mut archive, &cover_path, max_uncompressed_bytes) {
        Ok(bytes) if !bytes.is_empty() => bytes,
        Ok(_) => return inspection_failure(revision, "empty_cover"),
        Err(_) => return inspection_failure(revision, "missing_cover_member"),
    };
    EmbeddedCoverInspectionResult::Extracted {
        revision,
        bytes,
        media_type: declaration.media_type,
    }
}

fn inspection_failure(revision: FileRevision, code: &str) -> EmbeddedCoverInspectionResult {
    EmbeddedCoverInspectionResult::CouldNotInspect {
        revision,
        error: InspectionError(code.to_string()),
    }
}

fn read_zip_member(
    archive: &mut zip::ZipArchive<std::fs::File>,
    name: &str,
    max_bytes: u64,
) -> Result<Vec<u8>, &'static str> {
    let entry = archive
        .by_name(name)
        .map_err(|_| "missing_archive_member")?;
    if entry.size() > max_bytes {
        return Err("archive_member_limit");
    }
    let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
    entry
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| "archive_member_read")?;
    if bytes.len() as u64 > max_bytes {
        return Err("archive_member_limit");
    }
    Ok(bytes)
}

fn parse_container_path(xml: &[u8]) -> Result<String, &'static str> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_reader(xml);
    let mut buffer = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Empty(element)) | Ok(Event::Start(element))
                if element.local_name().as_ref() == b"rootfile" =>
            {
                for attribute in element.attributes() {
                    let attribute = attribute.map_err(|_| "invalid_container_xml")?;
                    if attribute.key.local_name().as_ref() == b"full-path" {
                        let path = attribute
                            .unescape_value()
                            .map_err(|_| "invalid_container_xml")?
                            .into_owned();
                        return normalize_archive_path(Path::new(&path)).ok_or("unsafe_opf_path");
                    }
                }
            }
            Ok(Event::Eof) => return Err("missing_rootfile"),
            Err(_) => return Err("invalid_container_xml"),
            _ => {}
        }
        buffer.clear();
    }
}

struct CoverDeclaration {
    href: String,
    media_type: String,
}

fn parse_cover_declaration(opf: &[u8]) -> Result<Option<CoverDeclaration>, &'static str> {
    use quick_xml::events::Event;

    let mut reader = quick_xml::Reader::from_reader(opf);
    let mut buffer = Vec::new();
    let mut cover_id = None;
    let mut items = Vec::new();
    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Empty(element)) | Ok(Event::Start(element)) => {
                if element.local_name().as_ref() == b"meta" {
                    let mut name = None;
                    let mut content = None;
                    for attribute in element.attributes() {
                        let attribute = attribute.map_err(|_| "invalid_opf_xml")?;
                        let value = attribute
                            .unescape_value()
                            .map_err(|_| "invalid_opf_xml")?
                            .into_owned();
                        match attribute.key.local_name().as_ref() {
                            b"name" => name = Some(value),
                            b"content" => content = Some(value),
                            _ => {}
                        }
                    }
                    if name.as_deref() == Some("cover") {
                        cover_id = content;
                    }
                } else if element.local_name().as_ref() == b"item" {
                    let mut id = None;
                    let mut href = None;
                    let mut media_type = None;
                    let mut cover_image = false;
                    for attribute in element.attributes() {
                        let attribute = attribute.map_err(|_| "invalid_opf_xml")?;
                        let value = attribute
                            .unescape_value()
                            .map_err(|_| "invalid_opf_xml")?
                            .into_owned();
                        match attribute.key.local_name().as_ref() {
                            b"id" => id = Some(value),
                            b"href" => href = Some(value),
                            b"media-type" => media_type = Some(value),
                            b"properties" => {
                                cover_image = value
                                    .split_whitespace()
                                    .any(|property| property == "cover-image")
                            }
                            _ => {}
                        }
                    }
                    items.push((id, href, media_type, cover_image));
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => return Err("invalid_opf_xml"),
            _ => {}
        }
        buffer.clear();
    }

    let selected = items.iter().find(|(id, _, _, cover_image)| {
        *cover_image
            || cover_id
                .as_ref()
                .is_some_and(|cover| id.as_ref() == Some(cover))
    });
    let Some((_, href, media_type, _)) = selected else {
        return Ok(None);
    };
    let href = href.clone().ok_or("cover_href_missing")?;
    let media_type = media_type
        .clone()
        .filter(|value| value.starts_with("image/"))
        .ok_or("cover_media_type_invalid")?;
    Ok(Some(CoverDeclaration { href, media_type }))
}

fn resolve_archive_member(opf_path: &str, href: &str) -> Option<String> {
    let base = Path::new(opf_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    normalize_archive_path(&base.join(href))
}

fn normalize_archive_path(path: &Path) -> Option<String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!normalized.as_os_str().is_empty()).then(|| normalized.to_string_lossy().into_owned())
}
