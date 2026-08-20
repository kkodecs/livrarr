use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

use crate::context::{
    EditionEvidenceCapability, HasAppConfigService, HasAuthorService, HasDiscoveryService,
    HasEditionRepository, HasHistoryService, HasIdentityRoadService, HasImportService,
    HasManualImportScan, HasManualImportService, HasMatchingService, HasWorkService,
};

pub trait ManualImportHandlerContext:
    HasMatchingService
    + HasManualImportService
    + HasManualImportScan
    + HasAppConfigService
    + HasAuthorService
    + HasWorkService
    + HasDiscoveryService
    + HasImportService
    + HasHistoryService
    + HasIdentityRoadService
    + HasEditionRepository
    + Clone
    + Send
    + Sync
    + 'static
{
}

impl<T> ManualImportHandlerContext for T where
    T: HasMatchingService
        + HasManualImportService
        + HasManualImportScan
        + HasAppConfigService
        + HasAuthorService
        + HasWorkService
        + HasDiscoveryService
        + HasImportService
        + HasHistoryService
        + HasIdentityRoadService
        + HasEditionRepository
        + Clone
        + Send
        + Sync
        + 'static
{
}
use crate::middleware::RequireAdmin;
use crate::ApiError;
use livrarr_domain::history_events;
use livrarr_domain::identity_layer::IdentityRoadService;
use livrarr_domain::services::{
    AppConfigService, AuthorService, DiscoveryService, HistoryService, ImportFileResult,
    ImportService, ImportSingleFileRequest, ManualImportService, MatchingService, RefreshSurface,
    WorkService,
};
use livrarr_domain::{classify_file, MediaType};
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ScanRequest {
    pub path: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResponse {
    pub scan_id: String,
    pub files: Vec<ScannedFile>,
    pub warnings: Vec<String>,
    pub ol_total: usize,
    pub ol_completed: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgressResponse {
    pub files: Vec<ScannedFile>,
    pub warnings: Vec<String>,
    pub ol_total: usize,
    pub ol_completed: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedFile {
    pub path: String,
    pub filename: String,
    /// Path relative to the scan root (e.g. `Author/Book/file.epub`).
    /// Gives nested files folder context in the scan results UI.
    pub rel_path: String,
    pub media_type: MediaType,
    pub size: i64,
    pub parsed: Option<ParsedFile>,
    #[serde(rename = "match")]
    pub ol_match: Option<SuggestedMatch>,
    pub existing_work_id: Option<i64>,
    pub has_existing_media_type: bool,
    pub routable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grouped_paths: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParsedFile {
    pub author: String,
    pub title: String,
    pub series: Option<String>,
    pub series_position: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// Embedded identifiers harvested from the file (EPUB OPF / M4B atoms)
    /// before any search (REQ-006), carried through the cluster→ParsedFile
    /// narrowing so the seam can seed identity. Populated during implementation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub isbn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub asin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestedMatch {
    pub ol_key: String,
    pub title: String,
    pub author: String,
    pub cover_url: Option<String>,
    pub existing_work_id: Option<i64>,
    // #97: carry the full discovery candidate so the import round-trip can reuse
    // the cached payload (candidate_id; REQ-014/015) and lock identity from any
    // anchor — not just an OpenLibrary key. All optional + camelCase so the
    // current frontend (which ignores them) keeps deserializing unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<livrarr_domain::identity::CandidateId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hc_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gr_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isbn_13: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

impl SuggestedMatch {
    /// Build a suggestion from a multi-provider discovery result, attaching the
    /// "already in library" work id resolved by the caller. `ol_key` falls back
    /// to empty (a non-OL result, e.g. Google Books) — the round-trip carries
    /// the other anchors so identity still locks at create.
    fn from_lookup(
        r: livrarr_domain::services::LookupResult,
        existing_work_id: Option<i64>,
    ) -> Self {
        Self {
            ol_key: r.ol_key.unwrap_or_default(),
            title: r.title,
            author: r.author_name,
            cover_url: r.cover_url,
            existing_work_id,
            candidate_id: r.candidate_id,
            hc_key: r.hc_key,
            gr_key: r.gr_key,
            asin: r.asin,
            isbn_13: r.isbn_13,
            year: r.year,
            source: r.source,
            language: r.language,
        }
    }
}

/// Snapshot returned by the scan accessor.
pub struct ScanSnapshot {
    pub files: Vec<ScannedFile>,
    pub warnings: Vec<String>,
    pub ol_total: usize,
    pub ol_completed: usize,
    pub user_id: i64,
}

/// Update for a single scanned file's OL match.
pub struct ScanFileUpdate {
    pub ol_match: Option<SuggestedMatch>,
    pub existing_work_id: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub author: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResponse {
    pub results: Vec<SuggestedMatch>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub items: Vec<ImportItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportItem {
    pub path: String,
    pub ol_key: String,
    pub title: String,
    pub author: String,
    pub delete_existing: bool,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub author_ol_key: Option<String>,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub isbn: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub series_name: Option<String>,
    #[serde(default)]
    pub series_position: Option<f64>,
    // #97: round-trip the picked candidate so add() can reuse its cached payload
    // (candidate_id) and lock identity from any anchor (HC/GR/ASIN), not only OL.
    #[serde(default)]
    pub candidate_id: Option<livrarr_domain::identity::CandidateId>,
    #[serde(default)]
    pub hc_key: Option<String>,
    #[serde(default)]
    pub gr_key: Option<String>,
    #[serde(default)]
    pub asin: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResponse {
    pub results: Vec<ImportResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub path: String,
    pub status: ImportStatus,
    pub work_id: Option<i64>,
    pub error: Option<String>,
    /// `Some` once file classification succeeds; `None` on the
    /// pre-classification failure paths (unrecognized media, missing root
    /// folder) — the type carries what each path truly knows.
    pub media_type: Option<MediaType>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ImportStatus {
    Imported,
    Skipped,
    Failed,
}

const MAX_MEDIA_FILES: usize = 2_000;
const MAX_ENTRIES_TRAVERSED: usize = 50_000;

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

pub async fn scan<S: ManualImportHandlerContext>(
    State(state): State<S>,
    RequireAdmin(auth): RequireAdmin,
    Json(req): Json<ScanRequest>,
) -> Result<Json<ScanResponse>, ApiError> {
    use crate::accessors::ManualImportScanAccessor;

    let path = PathBuf::from(&req.path);
    let precheck: Result<(), &'static str> = tokio::task::spawn_blocking({
        let path = path.clone();
        move || {
            if !path.exists() || !path.is_dir() {
                return Err("The file system path specified was not found.");
            }
            if std::fs::read_dir(&path).is_err() {
                return Err("The file system path specified was not found.");
            }
            Ok(())
        }
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;
    precheck.map_err(|msg| ApiError::BadRequest(msg.into()))?;

    let mut warnings = Vec::new();

    let (source_files, enumeration_warning) = tokio::task::spawn_blocking({
        let path = path.clone();
        move || enumerate_with_limits(&path)
    })
    .await
    .map_err(|e| ApiError::Internal(format!("spawn_blocking: {e}")))?;

    if let Some(w) = enumeration_warning {
        warnings.push(w);
    }

    if source_files.is_empty() {
        return Ok(Json(ScanResponse {
            scan_id: String::new(),
            files: vec![],
            warnings,
            ol_total: 0,
            ol_completed: 0,
        }));
    }

    // Group multi-file audiobooks into book-level items. Same-directory
    // multi-file books group by their shared directory; multi-disc books
    // (Book/CD1, Book/CD2, ...) collapse all discs into one book item.
    let scan_root = &path;
    let audio_files: Vec<(usize, PathBuf)> = source_files
        .iter()
        .enumerate()
        .filter(|(_, sf)| sf.media_type == MediaType::Audiobook)
        .map(|(i, sf)| (i, sf.path.clone()))
        .collect();
    let audio_groups = group_audio_files(&audio_files);

    struct ScanItem {
        display_name: String,
        /// Path relative to the scan root (e.g. `Author/Book/file.epub`),
        /// used to give nested files folder context in the UI.
        rel_path: String,
        primary_path: PathBuf,
        media_type: MediaType,
        grouped_paths: Option<Vec<PathBuf>>,
    }

    // Every audio-file index that landed in a group is skipped by the singleton
    // pass below; the rest become one row each.
    let grouped_indices: std::collections::HashSet<usize> = audio_groups
        .iter()
        .flat_map(|g| g.indices.iter().copied())
        .collect();

    let mut scan_items: Vec<ScanItem> = Vec::new();

    for group in &audio_groups {
        let rel = group
            .dir
            .strip_prefix(scan_root)
            .unwrap_or(&group.dir)
            .to_string_lossy()
            .to_string();
        let display = if rel.is_empty() {
            group
                .dir
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        } else {
            rel
        };
        let file_paths: Vec<PathBuf> = group
            .indices
            .iter()
            .map(|&i| source_files[i].path.clone())
            .collect();
        scan_items.push(ScanItem {
            display_name: display.clone(),
            rel_path: display,
            primary_path: group.dir.clone(),
            media_type: MediaType::Audiobook,
            grouped_paths: Some(file_paths),
        });
    }

    for (i, sf) in source_files.iter().enumerate() {
        if sf.media_type == MediaType::Audiobook && grouped_indices.contains(&i) {
            continue;
        }
        let filename = sf
            .path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let rel_path = scan_root_relative_path(scan_root, &sf.path, &filename);
        scan_items.push(ScanItem {
            display_name: filename,
            rel_path,
            primary_path: sf.path.clone(),
            media_type: sf.media_type,
            grouped_paths: None,
        });
    }

    // Local extraction via MatchingService.
    let scan_root_path = path.clone();
    let mut parsed_files: Vec<Option<ParsedFile>> = Vec::with_capacity(scan_items.len());
    for si in &scan_items {
        let input = livrarr_domain::services::MatchInput {
            file_path: Some(si.primary_path.clone()),
            grouped_paths: si.grouped_paths.clone(),
            parse_string: Some(si.display_name.clone()),
            media_type: Some(si.media_type),
            scan_root: Some(scan_root_path.clone()),
        };
        let clusters = state.matching_service().extract_and_reconcile(&input).await;
        let parsed = clusters.into_iter().next().map(|c| ParsedFile {
            author: c.author.unwrap_or_default(),
            title: c.title.unwrap_or_default(),
            series: c.series,
            series_position: c.series_position,
            language: c.language,
            isbn: c.isbn,
            asin: c.asin,
            year: c.year,
        });
        parsed_files.push(parsed);
    }

    // Sort by (author, series, series_position, title).
    let mut sort_indices: Vec<usize> = (0..scan_items.len()).collect();
    sort_indices.sort_by(|&a, &b| {
        let pa = parsed_files[a].as_ref();
        let pb = parsed_files[b].as_ref();
        let author_a = pa.map(|p| p.author.as_str()).unwrap_or("");
        let author_b = pb.map(|p| p.author.as_str()).unwrap_or("");
        let series_a = pa.and_then(|p| p.series.as_deref()).unwrap_or("");
        let series_b = pb.and_then(|p| p.series.as_deref()).unwrap_or("");
        let pos_a = pa.and_then(|p| p.series_position).unwrap_or(f64::MAX);
        let pos_b = pb.and_then(|p| p.series_position).unwrap_or(f64::MAX);
        let title_a = pa.map(|p| p.title.as_str()).unwrap_or("");
        let title_b = pb.map(|p| p.title.as_str()).unwrap_or("");
        author_a
            .cmp(author_b)
            .then(series_a.cmp(series_b))
            .then(
                pos_a
                    .partial_cmp(&pos_b)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(title_a.cmp(title_b))
    });

    let user_id = auth.user.id;
    let existing_works = state.manual_import_service().list_works(user_id).await?;
    let root_folders = state.manual_import_service().list_root_folders().await?;

    let pre_existing_work_ids: Vec<Option<i64>> = sort_indices
        .iter()
        .map(|&i| {
            let parsed = parsed_files.get(i).and_then(|p| p.as_ref());
            parsed.and_then(|p| {
                // Embedded identifiers give an exact key match (#149/#151):
                // a junk-titled file still matches its ISBN-anchored work.
                livrarr_matching::work_dedup::find_matching_work(
                    &existing_works,
                    &p.title,
                    &p.author,
                    &livrarr_matching::work_dedup::ProviderKeys {
                        isbn_13: p.isbn.as_deref(),
                        asin: p.asin.as_deref(),
                        ..Default::default()
                    },
                )
                .map(|w| w.id)
            })
        })
        .collect();

    let matched_work_ids: Vec<i64> = pre_existing_work_ids
        .iter()
        .flatten()
        .copied()
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let batch_items = if matched_work_ids.is_empty() {
        vec![]
    } else {
        state
            .manual_import_service()
            .list_library_items_by_work_ids(user_id, &matched_work_ids)
            .await
            .unwrap_or_default()
    };
    let mut items_by_work: std::collections::HashMap<i64, Vec<&livrarr_domain::LibraryItem>> =
        std::collections::HashMap::new();
    for item in &batch_items {
        items_by_work.entry(item.work_id).or_default().push(item);
    }

    let mut scanned_files = Vec::new();
    let mut ol_indices = Vec::new();

    for (loop_idx, &i) in sort_indices.iter().enumerate() {
        let si = &scan_items[i];
        let filename = si.display_name.clone();
        let parsed = parsed_files.get(i).cloned().flatten();

        let size: i64 = if let Some(ref paths) = si.grouped_paths {
            let mut total = 0u64;
            for p in paths {
                total += tokio::fs::metadata(p).await.map(|m| m.len()).unwrap_or(0);
            }
            total.try_into().unwrap_or(i64::MAX)
        } else {
            tokio::fs::metadata(&si.primary_path)
                .await
                .map(|m| m.len())
                .unwrap_or(0) as i64
        };

        let routable = root_folders.iter().any(|rf| rf.media_type == si.media_type);
        let existing_work_id = pre_existing_work_ids[loop_idx];

        let has_existing_media_type = existing_work_id
            .and_then(|wid| items_by_work.get(&wid))
            .map(|items| items.iter().any(|li| li.media_type == si.media_type))
            .unwrap_or(false);

        let check_path = si
            .grouped_paths
            .as_ref()
            .and_then(|p| p.first())
            .unwrap_or(&si.primary_path);
        let file_error = if tokio::fs::File::open(check_path).await.is_err() {
            Some("file not readable".to_string())
        } else {
            None
        };

        let grouped_path_strings = si
            .grouped_paths
            .as_ref()
            .map(|paths| paths.iter().map(|p| p.display().to_string()).collect());

        let display_filename = if let Some(ref paths) = si.grouped_paths {
            format!("{}/ ({} files)", filename, paths.len())
        } else {
            filename
        };

        // Scan-root-relative path for folder context in the UI. For grouped
        // audiobook dirs this is the directory path with a file-count suffix;
        // for singletons it is the file's path relative to the scan root.
        let rel_path = if let Some(ref paths) = si.grouped_paths {
            format!("{}/ ({} files)", si.rel_path, paths.len())
        } else {
            si.rel_path.clone()
        };

        let file_idx = scanned_files.len();
        if parsed.is_some() {
            ol_indices.push(file_idx);
        }

        scanned_files.push(ScannedFile {
            path: si.primary_path.display().to_string(),
            filename: display_filename,
            rel_path,
            media_type: si.media_type,
            size,
            parsed,
            ol_match: None,
            existing_work_id,
            has_existing_media_type,
            routable,
            error: file_error,
            grouped_paths: grouped_path_strings,
        });
    }

    let ol_total = ol_indices.len();
    let scan_id = uuid::Uuid::new_v4().to_string();

    state.manual_import_scan().insert_scan(
        scan_id.clone(),
        user_id,
        scanned_files.clone(),
        warnings.clone(),
        ol_total,
    );

    // Background discovery (#97): one multi-provider, author-grouped pass over
    // all parsed files instead of an OpenLibrary search per file. GB+OL are
    // queried once per author; each file's title is matched locally to its
    // author's corpus. Files already matched to an existing library work are
    // left untouched (don't let discovery override a good local match).
    let bg_state = state.clone();
    let bg_scan_id = scan_id.clone();
    tokio::spawn(async move {
        use crate::accessors::ManualImportScanAccessor;
        use livrarr_domain::services::EagerQuery;

        let scan = match bg_state.manual_import_scan().get_scan(&bg_scan_id) {
            Some(s) => s,
            None => return,
        };
        let user_id = scan.user_id;

        let mut queries: Vec<EagerQuery> = Vec::new();
        let mut already_done = 0usize;
        for &file_idx in &ol_indices {
            if scan
                .files
                .get(file_idx)
                .and_then(|f| f.existing_work_id)
                .is_some()
            {
                already_done += 1;
                continue;
            }
            if let Some(p) = scan.files.get(file_idx).and_then(|f| f.parsed.as_ref()) {
                // Strip a trailing parenthetical so the author corpus match keys
                // on the core title (same cleaning the old per-file OL search
                // used). No colon truncation: the raw parsed title (subtitle
                // included) feeds the identity authority, which already handles
                // subtitles — cutting it here would mangle the comparand before
                // it reaches best_candidate_index_lang / identity_absorb_match.
                let clean_title = match p.title.find('(') {
                    Some(paren) => p.title[..paren].trim().to_string(),
                    None => p.title.trim().to_string(),
                };
                queries.push(EagerQuery {
                    id: file_idx,
                    title: clean_title,
                    author: p.author.clone(),
                    language: p.language.clone(),
                    isbn: p.isbn.clone(),
                });
            } else {
                already_done += 1;
            }
        }

        // Files already matched to a library work (or unparseable) need no lookup;
        // count them toward completion now so the bar can reach 100%.
        for _ in 0..already_done {
            bg_state
                .manual_import_scan()
                .increment_ol_completed(&bg_scan_id);
        }

        let existing_works = bg_state
            .manual_import_service()
            .list_works(user_id)
            .await
            .unwrap_or_default();

        // Discover ONE author at a time, advancing the progress counter after each
        // author's files so the bar fills incrementally. Discovery is grouped by
        // author, so per-author is the honest granularity (not per-file); this is
        // the same set of provider calls as a single bulk pass — only the progress
        // reporting differs.
        let mut by_author: std::collections::HashMap<String, Vec<EagerQuery>> =
            std::collections::HashMap::new();
        for q in queries {
            by_author
                .entry(q.author.trim().to_lowercase())
                .or_default()
                .push(q);
        }

        let t_disc = std::time::Instant::now();
        let author_count = by_author.len();
        tracing::info!(
            authors = author_count,
            "perf discovery: start (per-author GB+OL)"
        );
        for group in by_author.into_values() {
            let n = group.len();
            let group_author = group.first().map(|q| q.author.clone()).unwrap_or_default();
            let t_g = std::time::Instant::now();
            let matches = bg_state
                .discovery_service()
                .eager_match_by_author(user_id, group)
                .await
                .unwrap_or_default();
            tracing::info!(
                ms = t_g.elapsed().as_millis() as u64,
                author = %group_author,
                files = n,
                hits = matches.len(),
                "perf discovery: author group"
            );

            for (file_idx, r) in matches {
                let existing_work_id = livrarr_matching::work_dedup::find_matching_work(
                    &existing_works,
                    &r.title,
                    &r.author_name,
                    &livrarr_matching::work_dedup::ProviderKeys {
                        ol_key: r.ol_key.as_deref(),
                        gr_key: r.gr_key.as_deref(),
                        isbn_13: r.isbn_13.as_deref(),
                        asin: r.asin.as_deref(),
                    },
                )
                .map(|w| w.id);
                bg_state.manual_import_scan().update_scan_file(
                    &bg_scan_id,
                    file_idx,
                    ScanFileUpdate {
                        ol_match: Some(SuggestedMatch::from_lookup(r, existing_work_id)),
                        existing_work_id,
                    },
                );
            }

            for _ in 0..n {
                bg_state
                    .manual_import_scan()
                    .increment_ol_completed(&bg_scan_id);
            }
        }
        tracing::info!(
            ms = t_disc.elapsed().as_millis() as u64,
            authors = author_count,
            "perf discovery: complete"
        );

        let st = bg_state.clone();
        let sid = bg_scan_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            st.manual_import_scan().remove_scan(&sid);
        });
    });

    Ok(Json(ScanResponse {
        scan_id,
        files: scanned_files,
        warnings,
        ol_total,
        ol_completed: 0,
    }))
}

pub async fn scan_progress<S: HasManualImportScan>(
    State(state): State<S>,
    RequireAdmin(auth): RequireAdmin,
    axum::extract::Path(scan_id): axum::extract::Path<String>,
) -> Result<Json<ScanProgressResponse>, ApiError> {
    use crate::accessors::ManualImportScanAccessor;

    let scan = state
        .manual_import_scan()
        .get_scan(&scan_id)
        .ok_or(ApiError::NotFound)?;

    // Verify the requesting user owns this scan.
    if scan.user_id != auth.user.id {
        return Err(ApiError::NotFound);
    }

    Ok(Json(ScanProgressResponse {
        files: scan.files,
        warnings: scan.warnings,
        ol_total: scan.ol_total,
        ol_completed: scan.ol_completed,
    }))
}

pub async fn search<S: HasDiscoveryService + HasManualImportScan + HasManualImportService>(
    State(state): State<S>,
    RequireAdmin(auth): RequireAdmin,
    Json(req): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, ApiError> {
    let term = if let Some(ref author) = req.author {
        format!("{} {}", req.query, author)
    } else {
        req.query.clone()
    };

    tracing::info!(query = %req.query, author = ?req.author, term = %term, "manual import search");

    let user_id = auth.user.id;

    // #97: route the manual re-search through the same multi-provider discovery
    // fan-out the Add Work box uses (Google Books / OpenLibrary / Hardcover /
    // Goodreads), not the legacy OpenLibrary-only path.
    let resp = state
        .discovery_service()
        .lookup_filtered(
            user_id,
            livrarr_domain::services::LookupRequest {
                term,
                lang_override: None,
            },
            false,
        )
        .await?;

    tracing::info!(
        results = resp.results.len(),
        "manual import search returned"
    );

    let existing_works = state.manual_import_service().list_works(user_id).await?;

    let results: Vec<SuggestedMatch> = resp
        .results
        .into_iter()
        .map(|r| {
            let existing_work_id = livrarr_matching::work_dedup::find_matching_work(
                &existing_works,
                &r.title,
                &r.author_name,
                &livrarr_matching::work_dedup::ProviderKeys {
                    ol_key: r.ol_key.as_deref(),
                    gr_key: r.gr_key.as_deref(),
                    isbn_13: r.isbn_13.as_deref(),
                    asin: r.asin.as_deref(),
                },
            )
            .map(|w| w.id);
            SuggestedMatch::from_lookup(r, existing_work_id)
        })
        .collect();

    Ok(Json(SearchResponse { results }))
}

pub async fn import<S: ManualImportHandlerContext>(
    State(state): State<S>,
    RequireAdmin(auth): RequireAdmin,
    Json(req): Json<ImportRequest>,
) -> Result<Json<ImportResponse>, ApiError> {
    let user_id = auth.user.id;
    let existing_works = state.manual_import_service().list_works(user_id).await?;
    let root_folders = state.manual_import_service().list_root_folders().await?;
    let media_mgmt = state
        .app_config_service()
        .get_media_management_config()
        .await?;
    // The user's default language, read once per import request: items whose
    // file and picked candidate both carry no language seed this value.
    let default_language = state.app_config_service().get_default_language().await?;

    let mut author_ol_cache: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();

    // One negative cover-host cache for the whole batch (dead-URL phase1
    // fast-fail): once a book's embedded cover URL fails to connect, later
    // books in this SAME request skip the doomed connect for that host. The
    // scope ends when the block below returns, so nothing survives past this
    // request — the next import (or a concurrent one on another task) starts
    // with an empty cache.
    let results = livrarr_domain::services::with_cover_host_cache(async {
        let mut results = Vec::new();
        for item in &req.items {
            let result = import_single_item(
                &state,
                user_id,
                item,
                &existing_works,
                &root_folders,
                &media_mgmt,
                &mut author_ol_cache,
                &default_language,
            )
            .await;

            // Per-file history write (REQ-003): the writer matches on the
            // outcome shape rather than trusting status alone, since a
            // successful classification is what guarantees work_id/media_type.
            match (&result.status, result.work_id, result.media_type) {
                (ImportStatus::Imported, Some(wid), Some(mt)) => {
                    let work_title = state
                        .work_service()
                        .get(user_id, wid)
                        .await
                        .map(|w| w.title)
                        .unwrap_or_default();
                    state
                        .history_service()
                        .record(
                            user_id,
                            history_events::imported_manual(
                                wid,
                                &work_title,
                                &result.path,
                                mt.as_str(),
                            ),
                        )
                        .await;
                }
                (ImportStatus::Imported, _, _) => {
                    warn!(
                        "manual import: Imported result missing work_id or media_type for {}",
                        result.path
                    );
                }
                (ImportStatus::Failed, Some(wid), mt) => {
                    let work_title = state
                        .work_service()
                        .get(user_id, wid)
                        .await
                        .map(|w| w.title)
                        .unwrap_or_default();
                    state
                        .history_service()
                        .record(
                            user_id,
                            history_events::import_failed_manual(
                                wid,
                                &work_title,
                                &result.path,
                                mt.map(|m| m.as_str()),
                                result.error.as_deref().unwrap_or_default(),
                            ),
                        )
                        .await;
                }
                (ImportStatus::Failed, None, mt) => {
                    state
                        .history_service()
                        .record(
                            user_id,
                            history_events::import_failed_unattached(
                                &result.path,
                                mt.map(|m| m.as_str()),
                                result.error.as_deref().unwrap_or_default(),
                            ),
                        )
                        .await;
                }
                (ImportStatus::Skipped, ..) => {}
            }

            results.push(result);
        }
        results
    })
    .await;

    // Background refresh for each imported work: fills in anchors (GR/HC/ASIN)
    // that initial enrichment misses because they require the identity road.
    for work_id in results.iter().filter_map(|r| r.work_id) {
        let s = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let _ = s
                .work_service()
                .refresh(user_id, work_id, RefreshSurface::Interactive)
                .await;
        });
    }

    Ok(Json(ImportResponse { results }))
}

// ---------------------------------------------------------------------------
// Import helpers
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn import_single_item<S: ManualImportHandlerContext>(
    state: &S,
    user_id: i64,
    item: &ImportItem,
    existing_works: &[livrarr_domain::Work],
    root_folders: &[livrarr_domain::RootFolder],
    _media_mgmt: &livrarr_domain::settings::MediaManagementConfig,
    author_ol_cache: &mut std::collections::HashMap<String, Option<String>>,
    default_language: &str,
) -> ImportResult {
    let source = PathBuf::from(&item.path);

    let media_type = match classify_file(&source) {
        Some(mt) => mt,
        None => {
            return ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Failed,
                work_id: None,
                error: Some("unrecognized media type".into()),
                media_type: None,
            };
        }
    };

    let root_folder = match root_folders.iter().find(|rf| rf.media_type == media_type) {
        Some(rf) => rf,
        None => {
            return ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Failed,
                work_id: None,
                error: Some(format!("no root folder configured for {:?}", media_type)),
                media_type: Some(media_type),
            };
        }
    };

    let work_id = match find_or_create_work(
        state,
        user_id,
        item,
        existing_works,
        author_ol_cache,
        default_language,
    )
    .await
    {
        Ok(id) => id,
        Err(e) => {
            warn!("manual import: work creation failed for {}: {e}", item.path);
            return ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Failed,
                work_id: None,
                error: Some(format!("work creation failed: {e}")),
                media_type: Some(media_type),
            };
        }
    };

    // Embedded file metadata is direct Edition evidence. Keep it downstream
    // of the one Work settlement but upstream of file attachment, and never
    // derive an Edition subtitle from the Work display title.
    let embedded_language = state
        .matching_service()
        .extract_and_reconcile(&livrarr_domain::services::MatchInput {
            file_path: Some(source.clone()),
            grouped_paths: None,
            parse_string: None,
            media_type: None,
            scan_root: None,
        })
        .await
        .into_iter()
        .find_map(|candidate| candidate.language);
    let edition_format = match media_type {
        MediaType::Ebook => livrarr_domain::identity_layer::EditionFormat::Ebook,
        MediaType::Audiobook => livrarr_domain::identity_layer::EditionFormat::Audiobook,
    };
    if let Err(error) = state
        .edition_repository()
        .apply_evidence(user_id, work_id, edition_format, embedded_language)
        .await
    {
        return ImportResult {
            path: item.path.clone(),
            status: ImportStatus::Failed,
            work_id: Some(work_id),
            error: Some(format!("edition evidence failed: {error}")),
            media_type: Some(media_type),
        };
    }

    let target_path = state.import_service().build_target_path(
        &root_folder.path,
        user_id,
        &item.author,
        &item.title,
        media_type,
        &source,
        &source,
    );

    let req = ImportSingleFileRequest {
        source,
        target_path,
        root_folder_path: root_folder.path.clone(),
        root_folder_id: root_folder.id,
        media_type,
        user_id,
        work_id,
        author_name: item.author.clone(),
        title: item.title.clone(),
        import_id: None,
    };

    match state.import_service().import_single_file(req).await {
        ImportFileResult::Ok => {
            info!("manual import: imported {} for work {}", item.path, work_id);
            ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Imported,
                work_id: Some(work_id),
                error: None,
                media_type: Some(media_type),
            }
        }
        ImportFileResult::Warning(w) => {
            info!(
                "manual import: imported {} for work {} with warning: {w}",
                item.path, work_id
            );
            ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Imported,
                work_id: Some(work_id),
                error: None,
                media_type: Some(media_type),
            }
        }
        ImportFileResult::Skipped(msg) => {
            info!(
                "manual import: skipped {} for work {} ({msg})",
                item.path, work_id
            );
            ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Skipped,
                work_id: Some(work_id),
                error: None,
                media_type: Some(media_type),
            }
        }
        ImportFileResult::Failed(e) => {
            warn!(
                "manual import: failed {} for work {}: {e}",
                item.path, work_id
            );
            ImportResult {
                path: item.path.clone(),
                status: ImportStatus::Failed,
                work_id: Some(work_id),
                error: Some(e),
                media_type: Some(media_type),
            }
        }
    }
}

fn find_existing_work<'a>(
    works: &'a [livrarr_domain::Work],
    ol_key: &str,
    title: &str,
    author: &str,
) -> Option<&'a livrarr_domain::Work> {
    livrarr_matching::work_dedup::find_matching_work(
        works,
        title,
        author,
        &livrarr_matching::work_dedup::ProviderKeys {
            ol_key: if ol_key.is_empty() {
                None
            } else {
                Some(ol_key)
            },
            ..Default::default()
        },
    )
}

async fn find_or_create_work<
    S: HasAuthorService
        + HasWorkService
        + HasManualImportService
        + HasMatchingService
        + HasIdentityRoadService,
>(
    state: &S,
    user_id: i64,
    item: &ImportItem,
    existing_works: &[livrarr_domain::Work],
    author_ol_cache: &mut std::collections::HashMap<String, Option<String>>,
    _default_language: &str,
) -> Result<i64, ApiError> {
    if let Some(work) = find_existing_work(existing_works, &item.ol_key, &item.title, &item.author)
    {
        let author = state
            .author_service()
            .add(
                user_id,
                livrarr_domain::services::AddAuthorRequest {
                    name: item.author.clone(),
                    sort_name: None,
                    ol_key: item.author_ol_key.clone(),
                    monitored: true,
                },
            )
            .await?;
        settle_manual_import(state, user_id, item, author.author().id, Some(work.id)).await?;
        return Ok(work.id);
    }

    let cache_key = item.author.to_lowercase();
    let author_ol_key = if let Some(cached) = author_ol_cache.get(&cache_key) {
        cached.clone()
    } else {
        let result = match state.author_service().lookup_authors(&item.author, 1).await {
            Ok(results) => results.into_iter().next().map(|r| r.ol_key),
            Err(e) => {
                tracing::warn!(author = %item.author, error = %e, "OL author lookup failed");
                None
            }
        };
        author_ol_cache.insert(cache_key, result.clone());
        result
    };

    let author = state
        .author_service()
        .add(
            user_id,
            livrarr_domain::services::AddAuthorRequest {
                name: item.author.clone(),
                sort_name: None,
                ol_key: item.author_ol_key.clone().or(author_ol_key.clone()),
                monitored: true,
            },
        )
        .await?;
    settle_manual_import(state, user_id, item, author.author().id, None).await
}

async fn settle_manual_import<S: HasIdentityRoadService>(
    state: &S,
    user_id: i64,
    item: &ImportItem,
    author_id: i64,
    existing_work_id: Option<i64>,
) -> Result<i64, ApiError> {
    let evidence = owned_file_evidence(&item.path).await?;
    let minimum = livrarr_domain::identity_layer::MinimumWorkEvidence {
        title: item.title.clone(),
        authors: vec![author_id],
    };
    let provider_identity = import_provider_evidence(item);
    let user_choice = existing_work_id.map_or_else(
        || livrarr_domain::identity_layer::UserIdentityChoice::ExplicitCreate(minimum.clone()),
        livrarr_domain::identity_layer::UserIdentityChoice::ExistingWork,
    );
    let outcome = state
        .identity_road_service()
        .settle(livrarr_domain::identity_layer::IdentityRoadRequest {
            user_id,
            origin: livrarr_domain::identity_layer::IdentityRoadOrigin::CreationDoor(
                livrarr_domain::identity_layer::DoorKind::ManualImport,
            ),
            evidence: livrarr_domain::identity_layer::IdentityEvidenceBundle {
                user_choice: Some(user_choice),
                owned_files: vec![evidence],
                provider_identity,
                minimum: existing_work_id.is_none().then_some(minimum),
            },
            interaction: livrarr_domain::identity_layer::IdentityRoadInteraction::HumanWatching,
            existing_work_id,
        })
        .await
        .map_err(|error| ApiError::BadRequest(error.to_string()))?;
    match outcome {
        livrarr_domain::identity_layer::IdentityRoadOutcome::Settled { work_id, .. } => Ok(work_id),
        livrarr_domain::identity_layer::IdentityRoadOutcome::ReviewPending {
            review_id,
            unattached,
            ..
        } => Err(ApiError::Conflict {
            reason: if unattached {
                format!("manual import parked unattached review card {review_id}")
            } else {
                format!("manual import requires review card {review_id}")
            },
        }),
        livrarr_domain::identity_layer::IdentityRoadOutcome::Deferred { reason } => {
            Err(ApiError::Conflict { reason: reason.0 })
        }
        livrarr_domain::identity_layer::IdentityRoadOutcome::Rejected { reason } => {
            Err(ApiError::BadRequest(reason.to_string()))
        }
    }
}

async fn owned_file_evidence(
    path: &str,
) -> Result<livrarr_domain::identity_layer::OwnedFileEvidence, ApiError> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| ApiError::BadRequest(format!("read import evidence: {error}")))?;
    let metadata = tokio::fs::metadata(path)
        .await
        .map_err(|error| ApiError::BadRequest(format!("stat import evidence: {error}")))?;
    let modified_ns = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos() as i128);
    let sha256: [u8; 32] = Sha256::digest(&bytes).into();
    Ok(livrarr_domain::identity_layer::OwnedFileEvidence {
        library_item_id: 0,
        file_revision: livrarr_domain::identity_layer::FileRevision {
            size_bytes: metadata.len(),
            modified_ns,
            sha256,
        },
    })
}

fn import_provider_evidence(
    item: &ImportItem,
) -> Vec<livrarr_domain::identity_layer::ProviderIdentityEvidence> {
    use livrarr_domain::identity_layer::{
        IdentityProvider, ProviderIdentityEvidence, RouteKey, RouteKind,
    };
    [
        (!item.ol_key.trim().is_empty()).then_some((
            IdentityProvider::OpenLibrary,
            RouteKind::OpenLibraryWork,
            item.ol_key.as_str(),
        )),
        item.gr_key
            .as_deref()
            .filter(|value| !value.is_empty())
            // Discovery/manual-import `gr_key` is sourced from a Goodreads
            // `/book/show` identifier embedded in or attached to the file.
            .map(|value| {
                (
                    IdentityProvider::Goodreads,
                    RouteKind::GoodreadsBookEdition,
                    value,
                )
            }),
        item.hc_key
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| (IdentityProvider::Hardcover, RouteKind::HardcoverWork, value)),
        item.isbn
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| {
                (
                    IdentityProvider::IsbnRegistry,
                    RouteKind::Isbn13Edition,
                    value,
                )
            }),
        item.asin
            .as_deref()
            .filter(|value| !value.is_empty())
            .map(|value| (IdentityProvider::Amazon, RouteKind::AsinEdition, value)),
    ]
    .into_iter()
    .flatten()
    .map(|(provider, kind, value)| ProviderIdentityEvidence {
        provider: provider.clone(),
        route: RouteKey {
            provider,
            kind,
            value: value.to_string(),
        },
        work_core: None,
        provenance: Default::default(),
    })
    .collect()
}

// ---------------------------------------------------------------------------
// File enumeration
// ---------------------------------------------------------------------------

struct EnumeratedFile {
    path: PathBuf,
    media_type: MediaType,
}

fn enumerate_with_limits(dir: &Path) -> (Vec<EnumeratedFile>, Option<String>) {
    let mut files = Vec::new();
    let mut entries_traversed = 0usize;
    let mut warning = None;

    enumerate_recursive(dir, &mut files, &mut entries_traversed, &mut warning);

    (files, warning)
}

fn enumerate_recursive(
    dir: &Path,
    files: &mut Vec<EnumeratedFile>,
    entries_traversed: &mut usize,
    warning: &mut Option<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        *entries_traversed += 1;

        if *entries_traversed > MAX_ENTRIES_TRAVERSED {
            *warning = Some(format!(
                "Traversal limit reached ({MAX_ENTRIES_TRAVERSED} entries). Some files may not be shown."
            ));
            return;
        }

        if files.len() >= MAX_MEDIA_FILES {
            *warning = Some(format!(
                "Found more than {MAX_MEDIA_FILES} media files. Showing first {MAX_MEDIA_FILES}."
            ));
            return;
        }

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with('.') {
            continue;
        }

        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };

        if ft.is_dir() {
            enumerate_recursive(&path, files, entries_traversed, warning);
            if warning.is_some() {
                return;
            }
        } else if ft.is_file() {
            if let Some(media_type) = classify_file(&path) {
                files.push(EnumeratedFile { path, media_type });
            }
        }
    }
}

/// Compute a scan-root-relative display path for a file (e.g.
/// `Author/Book/file.epub`), giving nested files folder context in the scan
/// results UI. Falls back to the bare filename when `file_path` is not under
/// `scan_root`.
fn scan_root_relative_path(scan_root: &Path, file_path: &Path, filename: &str) -> String {
    file_path
        .strip_prefix(scan_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| filename.to_string())
}

/// A group of audio files that belong to a single book item in the scan
/// results. `dir` is the directory the group is keyed on — either the immediate
/// parent directory (same-dir multi-file books) or the parent "book" directory
/// when its children are disc/part subfolders (multi-disc books). `indices`
/// point into the original audio-file slice.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioGroup {
    dir: PathBuf,
    indices: Vec<usize>,
}

/// Return true if `name` looks like a disc/part divider directory, e.g.
/// `cd1`, `cd 1`, `cd-1`, `disc 2`, `disc-2`, `disk 3`, `part 1`, `pt 1`,
/// `vol 2`, `volume 2`. Matching is case-insensitive and tolerates a separator
/// (space, hyphen, underscore) and leading zeros between the keyword and the
/// number. The remainder after the keyword must be a non-empty run of digits so
/// that ordinary folders like `cdrom` or `partials` are not misread as dividers.
fn is_disc_part_dir_name(name: &str) -> bool {
    // Longer keywords first so `volume` wins over `vol` and `disk` is tried
    // before nothing — prefix stripping is greedy on the first match.
    const KEYWORDS: &[&str] = &["volume", "disc", "disk", "part", "vol", "cd", "pt"];
    let lower = name.trim().to_lowercase();
    for kw in KEYWORDS {
        if let Some(rest) = lower.strip_prefix(kw) {
            // Allow one optional separator between keyword and number.
            let rest = rest
                .strip_prefix(' ')
                .or_else(|| rest.strip_prefix('-'))
                .or_else(|| rest.strip_prefix('_'))
                .unwrap_or(rest);
            let rest = rest.trim();
            if !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

/// Group audio files into book-level items.
///
/// Input is `(original_index, path)` for every audio file found in the scan.
/// The result is the set of grouped items (each with ≥2 files); any audio file
/// whose index is absent from every group is a singleton row handled by the
/// caller.
///
/// Two collapsing rules apply, in order:
///
/// 1. **Multi-disc / multi-part books.** When a directory's audio-containing
///    immediate child directories are *all* disc/part dividers (e.g.
///    `Book/CD1`, `Book/CD2`), every audio file beneath that book directory is
///    collapsed into one group keyed on the book directory. The requirement
///    that *every* audio-bearing child be a divider is the guard against
///    merging two genuinely separate books that merely sit side by side — real
///    book folders are not named `cd1`/`disc 2`.
///
/// 2. **Same-directory multi-file books.** Any remaining directory holding ≥2
///    *chapter-style* audio files (and not already absorbed by rule 1) becomes
///    one group keyed on that directory. Self-contained containers (`.m4b`)
///    never join these groups: one m4b is one complete book, so multiple m4bs
///    sharing a directory (e.g. loose files in a downloads root) are separate
///    books. Disc-divider layouts still collapse m4bs under rule 1 — there the
///    explicit `CD1`/`Part 2` naming signals a single title.
fn group_audio_files(audio: &[(usize, PathBuf)]) -> Vec<AudioGroup> {
    use std::collections::BTreeMap;

    // Bucket by immediate parent directory, preserving input order within each
    // bucket. BTreeMap gives deterministic iteration for stable output.
    let mut dir_files: BTreeMap<PathBuf, Vec<usize>> = BTreeMap::new();
    for (idx, path) in audio {
        if let Some(parent) = path.parent() {
            dir_files
                .entry(parent.to_path_buf())
                .or_default()
                .push(*idx);
        }
    }

    // Identify candidate book directories: the parents of any divider-named
    // directory that holds audio.
    let mut book_dirs: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
    for dir in dir_files.keys() {
        let is_divider = dir
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(is_disc_part_dir_name);
        if is_divider {
            if let Some(book_dir) = dir.parent() {
                book_dirs.insert(book_dir.to_path_buf());
            }
        }
    }

    let mut grouped: Vec<AudioGroup> = Vec::new();
    let mut absorbed_dirs: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    // Rule 1: collapse multi-disc books.
    for book_dir in &book_dirs {
        // Every audio-bearing immediate child of book_dir must be a divider for
        // us to treat book_dir as a single multi-disc title.
        let mut audio_children: Vec<&PathBuf> = dir_files
            .keys()
            .filter(|d| d.parent() == Some(book_dir.as_path()))
            .collect();
        audio_children.sort();
        let all_dividers = audio_children.iter().all(|d| {
            d.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(is_disc_part_dir_name)
        });
        if !all_dividers || audio_children.is_empty() {
            continue;
        }

        // Collect every audio file under the disc children, in deterministic
        // (sorted-dir, input) order.
        let mut indices: Vec<usize> = Vec::new();
        for child in &audio_children {
            if let Some(files) = dir_files.get(*child) {
                indices.extend(files.iter().copied());
            }
        }
        if indices.len() >= 2 {
            for child in &audio_children {
                absorbed_dirs.insert((*child).clone());
            }
            grouped.push(AudioGroup {
                dir: book_dir.clone(),
                indices,
            });
        }
        // If only a single file lives under the disc dirs, leave it for rule 2
        // / singleton handling (don't form a one-file multi-disc group).
    }

    // Rule 2: same-directory multi-file books for everything not absorbed.
    // Self-contained containers stay out: two m4bs in one directory are two
    // books, not chapters of one.
    let idx_to_path: std::collections::HashMap<usize, &PathBuf> =
        audio.iter().map(|(i, p)| (*i, p)).collect();
    for (dir, indices) in &dir_files {
        if absorbed_dirs.contains(dir) {
            continue;
        }
        let groupable: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|i| {
                idx_to_path
                    .get(i)
                    .is_none_or(|p| !is_self_contained_audio(p))
            })
            .collect();
        if groupable.len() >= 2 {
            grouped.push(AudioGroup {
                dir: dir.clone(),
                indices: groupable,
            });
        }
    }

    grouped
}

/// Self-contained audiobook container: one file is one complete book, so
/// same-directory grouping (rule 2) must never merge multiple of these.
fn is_self_contained_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("m4b"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_path_for_nested_file_is_scan_root_relative() {
        let scan_root = Path::new("/import");
        let file = Path::new("/import/Brandon Sanderson/Mistborn/book.epub");
        let rel = scan_root_relative_path(scan_root, file, "book.epub");
        assert_eq!(rel, "Brandon Sanderson/Mistborn/book.epub");
    }

    #[test]
    fn rel_path_for_top_level_file_is_just_filename() {
        let scan_root = Path::new("/import");
        let file = Path::new("/import/book.epub");
        let rel = scan_root_relative_path(scan_root, file, "book.epub");
        assert_eq!(rel, "book.epub");
    }

    #[test]
    fn rel_path_falls_back_to_filename_when_not_under_root() {
        let scan_root = Path::new("/import");
        let file = Path::new("/elsewhere/book.epub");
        let rel = scan_root_relative_path(scan_root, file, "book.epub");
        assert_eq!(rel, "book.epub");
    }

    fn pbs(paths: &[&str]) -> Vec<(usize, PathBuf)> {
        paths
            .iter()
            .enumerate()
            .map(|(i, p)| (i, PathBuf::from(p)))
            .collect()
    }

    #[test]
    fn disc_part_dir_name_matches_common_patterns() {
        for ok in [
            "cd1",
            "cd 1",
            "cd-1",
            "cd_1",
            "CD1",
            "Cd 02",
            "disc1",
            "disc 2",
            "disc-2",
            "Disc 03",
            "disk3",
            "disk 3",
            "part1",
            "part 3",
            "Part 10",
            "pt1",
            "pt 2",
            "vol2",
            "vol 2",
            "volume2",
            "volume 12",
        ] {
            assert!(is_disc_part_dir_name(ok), "expected divider: {ok}");
        }
        for no in [
            "cdrom",
            "partials",
            "Book Title",
            "disco",
            "cd",
            "part",
            "vol",
            "Chapter 1",
            "cda",
            "volume",
            "",
        ] {
            assert!(!is_disc_part_dir_name(no), "expected NOT divider: {no}");
        }
    }

    #[test]
    fn multi_disc_book_collapses_to_one_group() {
        let files = pbs(&[
            "/import/Author/Book/CD1/track01.mp3",
            "/import/Author/Book/CD1/track02.mp3",
            "/import/Author/Book/CD2/track01.mp3",
            "/import/Author/Book/CD2/track02.mp3",
        ]);
        let groups = group_audio_files(&files);
        assert_eq!(groups.len(), 1, "multi-disc book should be ONE group");
        assert_eq!(groups[0].dir, PathBuf::from("/import/Author/Book"));
        assert_eq!(groups[0].indices.len(), 4);
    }

    #[test]
    fn two_adjacent_separate_books_stay_separate() {
        // Two distinct book folders, each with its own tracks — not discs of one
        // title. Must NOT merge.
        let files = pbs(&[
            "/import/Author/Book One/track01.mp3",
            "/import/Author/Book One/track02.mp3",
            "/import/Author/Book Two/track01.mp3",
            "/import/Author/Book Two/track02.mp3",
        ]);
        let mut groups = group_audio_files(&files);
        groups.sort_by(|a, b| a.dir.cmp(&b.dir));
        assert_eq!(groups.len(), 2, "two separate books should stay separate");
        assert_eq!(groups[0].dir, PathBuf::from("/import/Author/Book One"));
        assert_eq!(groups[1].dir, PathBuf::from("/import/Author/Book Two"));
    }

    #[test]
    fn flat_directory_of_tracks_is_one_group() {
        let files = pbs(&[
            "/import/Author/Book/track01.mp3",
            "/import/Author/Book/track02.mp3",
            "/import/Author/Book/track03.mp3",
        ]);
        let groups = group_audio_files(&files);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].dir, PathBuf::from("/import/Author/Book"));
        assert_eq!(groups[0].indices.len(), 3);
    }

    #[test]
    fn single_file_is_not_grouped() {
        let files = pbs(&["/import/Author/Book.m4b"]);
        let groups = group_audio_files(&files);
        assert!(groups.is_empty(), "lone file should remain a singleton");
    }

    #[test]
    fn book_with_disc_dirs_plus_stray_audio_does_not_collapse() {
        // The book dir has a non-divider audio-bearing child alongside the disc
        // dirs, so we cannot safely treat it as a single multi-disc title. The
        // disc dirs still group individually (rule 2).
        let files = pbs(&[
            "/import/Author/Book/CD1/track01.mp3",
            "/import/Author/Book/CD1/track02.mp3",
            "/import/Author/Book/Bonus/extra01.mp3",
            "/import/Author/Book/Bonus/extra02.mp3",
        ]);
        let mut groups = group_audio_files(&files);
        groups.sort_by(|a, b| a.dir.cmp(&b.dir));
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].dir, PathBuf::from("/import/Author/Book/Bonus"));
        assert_eq!(groups[1].dir, PathBuf::from("/import/Author/Book/CD1"));
    }

    #[test]
    fn disc_dirs_with_one_file_each_still_collapse() {
        // Each disc holds a single (large) file — common for m4b-per-disc.
        let files = pbs(&[
            "/import/Author/Book/Disc 1/disc1.m4b",
            "/import/Author/Book/Disc 2/disc2.m4b",
        ]);
        let groups = group_audio_files(&files);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].dir, PathBuf::from("/import/Author/Book"));
        assert_eq!(groups[0].indices.len(), 2);
    }

    #[test]
    fn loose_m4bs_in_one_directory_stay_separate() {
        // Unrelated self-contained audiobooks dumped loose in a downloads root
        // must NOT fuse into one phantom work keyed on the shared directory.
        let files = pbs(&[
            "/import/A Game of Thrones (Unabridged).m4b",
            "/import/Fourth Wing.m4b",
            "/import/Project Hail Mary NL.m4b",
            "/import/The Man from the Future.m4b",
        ]);
        let groups = group_audio_files(&files);
        assert!(
            groups.is_empty(),
            "self-contained m4bs must stay singletons"
        );
    }

    #[test]
    fn mixed_directory_groups_chapters_but_not_m4b() {
        let files = pbs(&[
            "/import/Book/track01.mp3",
            "/import/Book/track02.mp3",
            "/import/Book/complete.m4b",
        ]);
        let groups = group_audio_files(&files);
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].indices,
            vec![0, 1],
            "m4b stays out of the chapter group"
        );
    }
}
