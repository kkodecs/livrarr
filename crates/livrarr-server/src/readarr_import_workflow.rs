use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, error, info, warn};

use livrarr_db::{
    CreateAuthorDbRequest, CreateImportDbRequest, CreateLibraryItemDbRequest, CreateWorkDbRequest,
    UpdateWorkEnrichmentDbRequest, UpdateWorkUserFieldsDbRequest,
};
use livrarr_domain::readarr::*;
use livrarr_domain::services::{ReadarrImportWorkflow, ServiceError};
use livrarr_domain::{
    derive_sort_name, normalize_for_matching, sanitize_path_component, EnrichmentStatus, Import,
    MediaType, WorkId,
};

use livrarr_http::HttpClient;

use crate::readarr_client::{self, RdAuthor, RdBook, RdBookFile, RdRootFolder, ReadarrClient};
use crate::readarr_import_service::ReadarrImportService;
use crate::state::{LiveEnrichmentWorkflow, LiveWorkService, ReadarrImportServiceImpl};

const POST_IMPORT_ENRICH_CONCURRENCY: usize = 3;

// =============================================================================
// LiveReadarrImportWorkflow
// =============================================================================

#[derive(Clone)]
pub struct LiveReadarrImportWorkflow {
    http_client: HttpClient,
    readarr_import_service: Arc<ReadarrImportServiceImpl>,
    readarr_import_progress: Arc<tokio::sync::Mutex<ReadarrImportProgress>>,
    data_dir: Arc<std::path::PathBuf>,
    enrichment_workflow: Arc<LiveEnrichmentWorkflow>,
    work_service: Arc<LiveWorkService>,
}

impl LiveReadarrImportWorkflow {
    pub fn new(
        http_client: HttpClient,
        readarr_import_service: Arc<ReadarrImportServiceImpl>,
        readarr_import_progress: Arc<tokio::sync::Mutex<ReadarrImportProgress>>,
        data_dir: Arc<std::path::PathBuf>,
        enrichment_workflow: Arc<LiveEnrichmentWorkflow>,
        work_service: Arc<LiveWorkService>,
    ) -> Self {
        Self {
            http_client,
            readarr_import_service,
            readarr_import_progress,
            data_dir,
            enrichment_workflow,
            work_service,
        }
    }
}

impl ReadarrImportWorkflow for LiveReadarrImportWorkflow {
    async fn connect(
        &self,
        req: ReadarrConnectRequest,
    ) -> Result<ReadarrConnectResponse, ServiceError> {
        let client = ReadarrClient::new(&req.url, &req.api_key, self.http_client.inner().clone());
        let folders = client
            .root_folders()
            .await
            .map_err(|e| ServiceError::Internal(format!("Readarr connection failed: {e}")))?;

        let root_folders = folders
            .into_iter()
            .map(|f| ReadarrRootFolderInfo {
                id: f.id,
                name: f.name,
                path: f.path,
                accessible: f.accessible,
                free_space: f.free_space,
                total_space: f.total_space,
            })
            .collect();

        Ok(ReadarrConnectResponse { root_folders })
    }

    async fn preview(
        &self,
        user_id: i64,
        req: ReadarrImportRequest,
    ) -> Result<ReadarrPreviewResponse, ServiceError> {
        let client = ReadarrClient::new(&req.url, &req.api_key, self.http_client.inner().clone());

        let data = fetch_all_readarr_data(&client).await?;

        let _readarr_root = data
            .root_folders
            .iter()
            .find(|f| f.id == req.readarr_root_folder_id)
            .map(|f| f.path.clone())
            .ok_or_else(|| ServiceError::Internal("Invalid Readarr root folder ID".into()))?;

        let livrarr_root = self
            .readarr_import_service
            .get_root_folder(req.livrarr_root_folder_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        let planner = ImportPlanner::new(&data, &livrarr_root.path, req.files_only, user_id);
        let existing_authors = self
            .readarr_import_service
            .list_authors(user_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        let existing_works = self
            .readarr_import_service
            .list_works(user_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        Ok(planner.preview(&existing_authors, &existing_works))
    }

    async fn start(
        &self,
        user_id: i64,
        req: ReadarrImportRequest,
    ) -> Result<ReadarrStartResponse, ServiceError> {
        let import_id = uuid::Uuid::new_v4().to_string();

        self.readarr_import_service
            .create_import(CreateImportDbRequest {
                id: import_id.clone(),
                user_id,
                source: "readarr".to_string(),
                source_url: Some(req.url.clone()),
                target_root_folder_id: Some(req.livrarr_root_folder_id),
            })
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        {
            let mut prog = self.readarr_import_progress.lock().await;
            *prog = ReadarrImportProgress {
                running: true,
                import_id: Some(import_id.clone()),
                phase: "fetching".to_string(),
                ..Default::default()
            };
        }

        let http_client = self.http_client.clone();
        let readarr_import_service = self.readarr_import_service.clone();
        let readarr_import_progress = self.readarr_import_progress.clone();
        let data_dir = self.data_dir.clone();
        let enrichment_workflow = self.enrichment_workflow.clone();
        let work_service = self.work_service.clone();
        let id = import_id.clone();

        tokio::spawn(async move {
            let runner = ImportRunner::new(
                http_client,
                readarr_import_service.clone(),
                readarr_import_progress.clone(),
                data_dir,
                &id,
                user_id,
                req,
                enrichment_workflow,
                work_service,
            );
            if let Err(e) = runner.run().await {
                error!(import_id = %id, "Readarr import failed: {e}");
                let _ = readarr_import_service
                    .update_import_status(&id, "failed")
                    .await;
            }

            let mut prog = readarr_import_progress.lock().await;
            prog.running = false;
            prog.phase = "done".to_string();
        });

        Ok(ReadarrStartResponse { import_id })
    }

    async fn progress(&self) -> ReadarrImportProgress {
        self.readarr_import_progress.lock().await.clone()
    }

    async fn history(&self, user_id: i64) -> Result<ReadarrHistoryResponse, ServiceError> {
        let imports = self
            .readarr_import_service
            .list_imports(user_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;
        let records = imports.iter().map(import_to_record).collect();
        Ok(ReadarrHistoryResponse { imports: records })
    }

    async fn undo(
        &self,
        user_id: i64,
        import_id: String,
    ) -> Result<ReadarrUndoResponse, ServiceError> {
        let imp = self
            .readarr_import_service
            .get_import(&import_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?
            .ok_or_else(|| ServiceError::Internal("Import not found".into()))?;

        if imp.user_id != user_id {
            return Err(ServiceError::Internal("Forbidden".into()));
        }
        if imp.status == "running" {
            return Err(ServiceError::Internal(
                "Cannot undo a running import".into(),
            ));
        }

        let items = self
            .readarr_import_service
            .list_library_items_by_import(&import_id)
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        let root_folder_path: Option<String> = if let Some(rf_id) = imp.target_root_folder_id {
            self.readarr_import_service
                .get_root_folder(rf_id)
                .await
                .ok()
                .map(|rf| rf.path)
        } else {
            None
        };

        let mut files_deleted = 0i64;
        let mut files_skipped = 0i64;

        for item in &items {
            let full_path = if let Some(ref root) = root_folder_path {
                PathBuf::from(root).join(&item.path)
            } else {
                PathBuf::from(&item.path)
            };
            if full_path.exists() {
                match std::fs::metadata(&full_path) {
                    Ok(meta) if meta.len() as i64 == item.file_size => {
                        match std::fs::remove_file(&full_path) {
                            Ok(()) => {
                                files_deleted += 1;
                                info!(path = %item.path, "Undo: deleted file");
                            }
                            Err(e) => {
                                warn!(path = %item.path, "Undo: failed to delete: {e}");
                                files_skipped += 1;
                            }
                        }
                    }
                    Ok(meta) => {
                        warn!(
                            path = %item.path,
                            expected = item.file_size,
                            actual = meta.len(),
                            "Undo: skipping file with size mismatch"
                        );
                        files_skipped += 1;
                    }
                    Err(e) => {
                        warn!(path = %item.path, "Undo: cannot stat file: {e}");
                        files_skipped += 1;
                    }
                }
            }

            if let Err(e) = self
                .readarr_import_service
                .delete_library_item_by_id(item.id)
                .await
            {
                warn!(id = item.id, "Undo: failed to delete library item: {e}");
            }
        }

        let orphan_work_ids = self
            .readarr_import_service
            .list_orphan_work_ids_by_import(&import_id)
            .await
            .unwrap_or_default();

        let works_deleted = self
            .readarr_import_service
            .delete_orphan_works_by_import(&import_id)
            .await
            .unwrap_or(0);

        for wid in &orphan_work_ids {
            livrarr_metadata::work_service::delete_cover_files(&self.data_dir, user_id, *wid).await;
        }

        let authors_deleted = self
            .readarr_import_service
            .delete_orphan_authors_by_import(&import_id)
            .await
            .unwrap_or(0);

        self.readarr_import_service
            .update_import_status(&import_id, "undone")
            .await
            .map_err(|e| ServiceError::Internal(e.to_string()))?;

        Ok(ReadarrUndoResponse {
            files_deleted,
            files_skipped,
            works_deleted,
            authors_deleted,
        })
    }
}

// =============================================================================
// Shared helpers
// =============================================================================

fn import_to_record(imp: &Import) -> ReadarrImportRecord {
    ReadarrImportRecord {
        id: imp.id.clone(),
        source: imp.source.clone(),
        status: imp.status.clone(),
        started_at: imp.started_at.to_rfc3339(),
        completed_at: imp.completed_at.map(|d| d.to_rfc3339()),
        authors_created: imp.authors_created,
        works_created: imp.works_created,
        files_imported: imp.files_imported,
        files_skipped: imp.files_skipped,
        source_url: imp.source_url.clone(),
    }
}

fn resolve_media_type(quality_id: Option<i32>, path: &str) -> Option<MediaType> {
    if let Some(qid) = quality_id {
        if let Some(mt_str) = readarr_client::quality_to_media_type(qid) {
            return match mt_str {
                "ebook" => Some(MediaType::Ebook),
                "audiobook" => Some(MediaType::Audiobook),
                _ => None,
            };
        }
    }
    if let Some(mt_str) = readarr_client::media_type_from_extension(path) {
        return match mt_str {
            "ebook" => Some(MediaType::Ebook),
            "audiobook" => Some(MediaType::Audiobook),
            _ => None,
        };
    }
    None
}

fn extract_quality_id(bf: &RdBookFile) -> Option<i32> {
    bf.quality.as_ref()?.quality.as_ref().map(|q| q.id)
}

fn parse_series_title(series_title: &str) -> (Option<String>, Option<f64>) {
    let segment = series_title
        .split(';')
        .next()
        .unwrap_or(series_title)
        .trim();
    if segment.is_empty() {
        return (None, None);
    }
    let re = regex::Regex::new(r"^(.*?)(?:\s+#([\d.]+))?$").unwrap();
    if let Some(caps) = re.captures(segment) {
        let name = caps.get(1).map(|m| m.as_str().trim().to_string());
        let pos = caps.get(2).and_then(|m| m.as_str().parse::<f64>().ok());
        let name = name.filter(|n| !n.is_empty());
        (name, pos)
    } else {
        (Some(segment.to_string()), None)
    }
}

fn extract_year(date_str: &str) -> Option<i32> {
    date_str.get(..4)?.parse::<i32>().ok()
}

fn extract_cover_url(images: &Option<Vec<readarr_client::RdImage>>) -> Option<String> {
    let imgs = images.as_ref()?;
    for img in imgs {
        if img.cover_type.as_deref() == Some("cover") {
            if let Some(ref url) = img.remote_url {
                if !url.is_empty() {
                    return Some(url.clone());
                }
            }
            if let Some(ref url) = img.url {
                if !url.is_empty() {
                    return Some(url.clone());
                }
            }
        }
    }
    for img in imgs {
        if let Some(ref url) = img.remote_url {
            if !url.is_empty() {
                return Some(url.clone());
            }
        }
        if let Some(ref url) = img.url {
            if !url.is_empty() {
                return Some(url.clone());
            }
        }
    }
    None
}

fn build_dest_path(
    root: &str,
    user_id: i64,
    author_name: &str,
    title: &str,
    source_path: &str,
) -> PathBuf {
    let ext = Path::new(source_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin");
    let author_dir = sanitize_path_component(author_name, "Unknown Author");
    let file_stem = sanitize_path_component(title, "Unknown Title");
    PathBuf::from(root)
        .join(user_id.to_string())
        .join(author_dir)
        .join(format!("{file_stem}.{ext}"))
}

fn validate_source_path(source: &str, readarr_root: &str) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(source)
        .map_err(|e| format!("cannot canonicalize source path: {e}"))?;
    let root_canonical = std::fs::canonicalize(readarr_root)
        .map_err(|e| format!("cannot canonicalize readarr root: {e}"))?;
    if !canonical.starts_with(&root_canonical) {
        return Err(format!(
            "source path {canonical:?} is not under readarr root {root_canonical:?}"
        ));
    }
    Ok(canonical)
}

fn apply_path_translation(
    path: &str,
    container_path: Option<&str>,
    host_path: Option<&str>,
) -> String {
    match (container_path, host_path) {
        (Some(cp), Some(hp)) if !cp.is_empty() && !hp.is_empty() => {
            let cp = cp.trim_end_matches('/');
            let hp = hp.trim_end_matches('/');
            if let Some(suffix) = path.strip_prefix(cp) {
                format!("{hp}{suffix}")
            } else {
                path.to_string()
            }
        }
        _ => path.to_string(),
    }
}

fn materialize_file(source: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir failed: {e}"))?;
    }
    if std::fs::hard_link(source, dest).is_ok() {
        return Ok(());
    }
    let temp = dest.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
    match std::fs::copy(source, &temp) {
        Ok(copied) => {
            let source_size = std::fs::metadata(source)
                .map_err(|e| format!("cannot stat source: {e}"))?
                .len();
            if copied != source_size {
                let _ = std::fs::remove_file(&temp);
                return Err(format!(
                    "copy size mismatch: copied {copied} vs source {source_size}"
                ));
            }
            std::fs::rename(&temp, dest).map_err(|e| {
                let _ = std::fs::remove_file(&temp);
                format!("rename failed: {e}")
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(format!("copy failed: {e}"))
        }
    }
}

fn media_type_str(mt: MediaType) -> &'static str {
    match mt {
        MediaType::Ebook => "ebook",
        MediaType::Audiobook => "audiobook",
    }
}

// =============================================================================
// Readarr data bundle (fetched once, shared by preview and import)
// =============================================================================

struct ReadarrData {
    authors: Vec<RdAuthor>,
    books: Vec<RdBook>,
    book_files: Vec<RdBookFile>,
    root_folders: Vec<RdRootFolder>,
}

async fn fetch_all_readarr_data(client: &ReadarrClient) -> Result<ReadarrData, ServiceError> {
    let root_folders = client
        .root_folders()
        .await
        .map_err(|e| ServiceError::Internal(format!("Readarr root folders: {e}")))?;
    let authors = client
        .authors()
        .await
        .map_err(|e| ServiceError::Internal(format!("Readarr authors: {e}")))?;
    let books = client
        .books()
        .await
        .map_err(|e| ServiceError::Internal(format!("Readarr books: {e}")))?;
    let author_ids: Vec<i64> = authors.iter().map(|a| a.id).collect();
    use futures::stream::{self, StreamExt};
    let file_results: Vec<(i64, Result<Vec<RdBookFile>, _>)> = stream::iter(
        author_ids
            .into_iter()
            .map(|aid| async move { (aid, client.book_files_by_author(aid).await) }),
    )
    .buffer_unordered(10)
    .collect()
    .await;
    let mut book_files: Vec<RdBookFile> = Vec::new();
    for (aid, res) in file_results {
        match res {
            Ok(files) => book_files.extend(files),
            Err(e) => {
                return Err(ServiceError::Internal(format!(
                    "Readarr book files (author {aid}): {e}"
                )));
            }
        }
    }
    Ok(ReadarrData {
        authors,
        books,
        book_files,
        root_folders,
    })
}

// =============================================================================
// ImportPlanner — shared logic for preview and the plan phase of import
// =============================================================================

struct ImportPlanner<'a> {
    author_map: HashMap<i64, &'a RdAuthor>,
    book_files_by_book: HashMap<i64, Vec<&'a RdBookFile>>,
    livrarr_root_path: &'a str,
    books: &'a [RdBook],
    files_only: bool,
    user_id: i64,
}

impl<'a> ImportPlanner<'a> {
    fn new(
        data: &'a ReadarrData,
        livrarr_root_path: &'a str,
        files_only: bool,
        user_id: i64,
    ) -> Self {
        let author_map: HashMap<i64, &RdAuthor> = data.authors.iter().map(|a| (a.id, a)).collect();
        let mut book_files_by_book: HashMap<i64, Vec<&RdBookFile>> = HashMap::new();
        for bf in &data.book_files {
            book_files_by_book.entry(bf.book_id).or_default().push(bf);
        }
        Self {
            author_map,
            book_files_by_book,
            livrarr_root_path,
            books: &data.books,
            files_only,
            user_id,
        }
    }

    fn preview(
        &self,
        existing_authors: &[livrarr_domain::Author],
        existing_works: &[livrarr_domain::Work],
    ) -> ReadarrPreviewResponse {
        let mut skipped_items: Vec<ReadarrSkippedItem> = Vec::new();
        let mut import_files: Vec<ReadarrPreviewFileItem> = Vec::new();
        let mut authors_to_create = 0i64;
        let mut works_to_create = 0i64;
        let mut works_existing = 0i64;
        let mut files_to_skip = 0i64;

        let mut author_names_seen: HashMap<String, bool> = HashMap::new();
        for a in existing_authors {
            author_names_seen.insert(normalize_for_matching(&a.name), true);
        }

        for book in self.books {
            let author_name = self
                .author_map
                .get(&book.author_id)
                .and_then(|a| a.author_name.as_deref())
                .unwrap_or("");
            let title = book.title.as_deref().unwrap_or("");

            if author_name.is_empty() {
                skipped_items.push(ReadarrSkippedItem {
                    title: title.to_string(),
                    author: String::new(),
                    reason: "No author".to_string(),
                });
                continue;
            }

            if self.files_only && !self.book_files_by_book.contains_key(&book.id) {
                continue;
            }

            let norm_author = normalize_for_matching(author_name);
            if !author_names_seen.contains_key(&norm_author) {
                author_names_seen.insert(norm_author.clone(), false);
                authors_to_create += 1;
            }

            let is_existing = self.is_work_existing(book, &norm_author, title, existing_works);

            let work_status = if is_existing { "existing" } else { "new" };
            if is_existing {
                works_existing += 1;
            } else {
                works_to_create += 1;
            }

            self.classify_book_files(
                book,
                author_name,
                title,
                work_status,
                &mut import_files,
                &mut skipped_items,
                &mut files_to_skip,
            );
        }

        let authors_existing = self
            .author_map
            .values()
            .filter(|a| {
                let name = a.author_name.as_deref().unwrap_or("");
                let norm = normalize_for_matching(name);
                author_names_seen.get(&norm) == Some(&true)
            })
            .count() as i64;

        ReadarrPreviewResponse {
            authors_to_create,
            authors_existing,
            works_to_create,
            works_existing,
            files_to_import: import_files.len() as i64,
            files_to_skip,
            skipped_items,
            import_files,
        }
    }

    fn is_work_existing(
        &self,
        book: &RdBook,
        norm_author: &str,
        title: &str,
        existing_works: &[livrarr_domain::Work],
    ) -> bool {
        let edition = book.monitored_edition();
        let isbn = edition
            .and_then(|e| e.isbn13.as_deref())
            .filter(|s| !s.is_empty());
        let asin = edition
            .and_then(|e| e.asin.as_deref())
            .filter(|s| !s.is_empty());
        let year = book.release_date.as_deref().and_then(extract_year);
        let norm_title = normalize_for_matching(title);

        if let Some(isbn_val) = isbn {
            existing_works
                .iter()
                .any(|w| w.isbn_13.as_deref() == Some(isbn_val))
        } else if let Some(asin_val) = asin {
            existing_works
                .iter()
                .any(|w| w.asin.as_deref() == Some(asin_val))
        } else {
            existing_works.iter().any(|w| {
                normalize_for_matching(&w.author_name) == norm_author
                    && normalize_for_matching(&w.title) == norm_title
                    && w.year == year
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn classify_book_files(
        &self,
        book: &RdBook,
        author_name: &str,
        title: &str,
        work_status: &str,
        import_files: &mut Vec<ReadarrPreviewFileItem>,
        skipped_items: &mut Vec<ReadarrSkippedItem>,
        files_to_skip: &mut i64,
    ) {
        let files = self.book_files_by_book.get(&book.id);
        let file_list: Vec<&&RdBookFile> = files.map(|f| f.iter().collect()).unwrap_or_default();

        let audiobook_count = file_list
            .iter()
            .filter(|f| {
                resolve_media_type(extract_quality_id(f), &f.path) == Some(MediaType::Audiobook)
            })
            .count();

        if audiobook_count > 1 {
            skipped_items.push(ReadarrSkippedItem {
                title: title.to_string(),
                author: author_name.to_string(),
                reason: format!("Multi-file audiobook ({audiobook_count} files)"),
            });
            *files_to_skip += audiobook_count as i64;

            for f in file_list.iter().filter(|f| {
                resolve_media_type(extract_quality_id(f), &f.path) != Some(MediaType::Audiobook)
            }) {
                self.classify_single_file(
                    f,
                    author_name,
                    title,
                    work_status,
                    import_files,
                    skipped_items,
                    files_to_skip,
                );
            }
        } else {
            for f in &file_list {
                self.classify_single_file(
                    f,
                    author_name,
                    title,
                    work_status,
                    import_files,
                    skipped_items,
                    files_to_skip,
                );
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn classify_single_file(
        &self,
        f: &RdBookFile,
        author_name: &str,
        title: &str,
        work_status: &str,
        import_files: &mut Vec<ReadarrPreviewFileItem>,
        skipped_items: &mut Vec<ReadarrSkippedItem>,
        files_to_skip: &mut i64,
    ) {
        let qid = extract_quality_id(f);
        match resolve_media_type(qid, &f.path) {
            None => {
                *files_to_skip += 1;
                skipped_items.push(ReadarrSkippedItem {
                    title: title.to_string(),
                    author: author_name.to_string(),
                    reason: format!("Unknown format: {}", f.path),
                });
            }
            Some(mt) => {
                let dest = build_dest_path(
                    self.livrarr_root_path,
                    self.user_id,
                    author_name,
                    title,
                    &f.path,
                );
                if dest.exists() {
                    *files_to_skip += 1;
                    skipped_items.push(ReadarrSkippedItem {
                        title: title.to_string(),
                        author: author_name.to_string(),
                        reason: "Destination already exists".to_string(),
                    });
                } else {
                    import_files.push(ReadarrPreviewFileItem {
                        title: title.to_string(),
                        author: author_name.to_string(),
                        path: f.path.clone(),
                        media_type: media_type_str(mt).to_string(),
                        work_status: work_status.to_string(),
                    });
                }
            }
        }
    }
}

// =============================================================================
// ImportRunner — executes the import in a background task
// =============================================================================

struct ImportRunner {
    http_client: HttpClient,
    readarr_import_service: Arc<ReadarrImportServiceImpl>,
    readarr_import_progress: Arc<tokio::sync::Mutex<ReadarrImportProgress>>,
    data_dir: Arc<std::path::PathBuf>,
    import_id: String,
    user_id: i64,
    req: ReadarrImportRequest,
    author_map_rd: HashMap<i64, i64>,
    work_map_rd: HashMap<i64, i64>,
    authors_created: i64,
    works_created: i64,
    files_imported: i64,
    files_skipped: i64,
    enrichment_workflow: Arc<LiveEnrichmentWorkflow>,
    work_service: Arc<LiveWorkService>,
    pending_enrich_work_ids: Vec<WorkId>,
}

impl ImportRunner {
    #[allow(clippy::too_many_arguments)]
    fn new(
        http_client: HttpClient,
        readarr_import_service: Arc<ReadarrImportServiceImpl>,
        readarr_import_progress: Arc<tokio::sync::Mutex<ReadarrImportProgress>>,
        data_dir: Arc<std::path::PathBuf>,
        import_id: &str,
        user_id: i64,
        req: ReadarrImportRequest,
        enrichment_workflow: Arc<LiveEnrichmentWorkflow>,
        work_service: Arc<LiveWorkService>,
    ) -> Self {
        Self {
            http_client,
            readarr_import_service,
            readarr_import_progress,
            data_dir,
            import_id: import_id.to_string(),
            user_id,
            req,
            author_map_rd: HashMap::new(),
            work_map_rd: HashMap::new(),
            authors_created: 0,
            works_created: 0,
            files_imported: 0,
            files_skipped: 0,
            enrichment_workflow,
            work_service,
            pending_enrich_work_ids: Vec::new(),
        }
    }

    fn progress(&self) -> &Arc<Mutex<ReadarrImportProgress>> {
        &self.readarr_import_progress
    }

    async fn run(mut self) -> Result<(), String> {
        let client = ReadarrClient::new(
            &self.req.url,
            &self.req.api_key,
            self.http_client.inner().clone(),
        );
        let data = fetch_all_readarr_data(&client)
            .await
            .map_err(|e| format!("fetch failed: {e}"))?;

        let readarr_root_raw = data
            .root_folders
            .iter()
            .find(|f| f.id == self.req.readarr_root_folder_id)
            .map(|f| f.path.clone())
            .ok_or_else(|| "Invalid Readarr root folder ID".to_string())?;
        let readarr_root = apply_path_translation(
            &readarr_root_raw,
            self.req.container_path.as_deref(),
            self.req.host_path.as_deref(),
        );

        let livrarr_root = self
            .readarr_import_service
            .get_root_folder(self.req.livrarr_root_folder_id)
            .await
            .map_err(|e| format!("get livrarr root folder: {e}"))?;

        let author_map: HashMap<i64, &RdAuthor> = data.authors.iter().map(|a| (a.id, a)).collect();
        let mut book_files_by_book: HashMap<i64, Vec<&RdBookFile>> = HashMap::new();
        for bf in &data.book_files {
            book_files_by_book.entry(bf.book_id).or_default().push(bf);
        }

        let active_book_ids: HashSet<i64> = if self.req.files_only {
            book_files_by_book.keys().copied().collect()
        } else {
            data.books.iter().map(|b| b.id).collect()
        };

        let active_books: Vec<&RdBook> = data
            .books
            .iter()
            .filter(|b| active_book_ids.contains(&b.id))
            .collect();

        {
            let mut prog = self.progress().lock().await;
            prog.phase = "processing".to_string();
            prog.authors_total = data.authors.len() as i64;
            prog.works_total = active_books.len() as i64;
            prog.files_total = data
                .book_files
                .iter()
                .filter(|f| active_book_ids.contains(&f.book_id))
                .count() as i64;
        }

        self.process_authors(&data.authors, &data.books, &active_book_ids)
            .await?;
        self.process_works(
            &active_books,
            &author_map,
            &book_files_by_book,
            &livrarr_root.path,
        )
        .await?;
        self.process_files(
            &data.book_files,
            &active_book_ids,
            &author_map,
            &data.books,
            &book_files_by_book,
            &readarr_root,
            &livrarr_root.path,
        )
        .await?;

        let _ = self
            .readarr_import_service
            .update_import_counts(
                &self.import_id,
                self.authors_created,
                self.works_created,
                self.files_imported,
                self.files_skipped,
            )
            .await;

        self.readarr_import_service
            .set_import_completed(&self.import_id)
            .await
            .map_err(|e| format!("set completed: {e}"))?;

        info!(
            import_id = %self.import_id,
            self.authors_created,
            self.works_created,
            self.files_imported,
            self.files_skipped,
            "Readarr import completed"
        );

        if !self.pending_enrich_work_ids.is_empty() {
            let work_ids = std::mem::take(&mut self.pending_enrich_work_ids);
            let enrichment_workflow = self.enrichment_workflow.clone();
            let work_service = self.work_service.clone();
            let user_id = self.user_id;
            let import_id = self.import_id.clone();
            let count = work_ids.len();

            tokio::spawn(async move {
                run_post_import_enrichment(
                    &import_id,
                    user_id,
                    work_ids,
                    enrichment_workflow,
                    work_service,
                )
                .await;
            });

            debug!(import_id = %self.import_id, count, "spawned post-import enrichment");
        }

        Ok(())
    }

    async fn process_authors(
        &mut self,
        rd_authors: &[RdAuthor],
        rd_books: &[RdBook],
        active_book_ids: &HashSet<i64>,
    ) -> Result<(), String> {
        let existing_authors = self
            .readarr_import_service
            .list_authors(self.user_id)
            .await
            .map_err(|e| format!("list authors: {e}"))?;

        for rd_author in rd_authors {
            let name = rd_author.author_name.as_deref().unwrap_or("").trim();
            if name.is_empty() {
                continue;
            }

            if self.req.files_only {
                let has_files = rd_books
                    .iter()
                    .filter(|b| b.author_id == rd_author.id)
                    .any(|b| active_book_ids.contains(&b.id));
                if !has_files {
                    let mut prog = self.progress().lock().await;
                    prog.authors_processed += 1;
                    continue;
                }
            }

            let norm = normalize_for_matching(name);
            let matches: Vec<_> = existing_authors
                .iter()
                .filter(|a| normalize_for_matching(&a.name) == norm)
                .collect();

            let livrarr_author_id = if matches.len() == 1 {
                matches[0].id
            } else {
                let sort_name = rd_author
                    .sort_name
                    .as_deref()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| derive_sort_name(name));

                match self
                    .readarr_import_service
                    .create_author(CreateAuthorDbRequest {
                        user_id: self.user_id,
                        name: name.to_string(),
                        sort_name: Some(sort_name),
                        ol_key: None,
                        gr_key: None,
                        hc_key: None,
                        import_id: Some(self.import_id.clone()),
                    })
                    .await
                {
                    Ok(a) => {
                        self.authors_created += 1;
                        a.id
                    }
                    Err(e) => {
                        warn!(name = %name, "Failed to create author: {e}");
                        let mut prog = self.progress().lock().await;
                        prog.errors.push(format!("Author '{name}': {e}"));
                        continue;
                    }
                }
            };

            self.author_map_rd.insert(rd_author.id, livrarr_author_id);

            {
                let mut prog = self.progress().lock().await;
                prog.authors_processed += 1;
            }
        }
        Ok(())
    }

    async fn process_works(
        &mut self,
        active_books: &[&RdBook],
        author_map: &HashMap<i64, &RdAuthor>,
        book_files_by_book: &HashMap<i64, Vec<&RdBookFile>>,
        livrarr_root_path: &str,
    ) -> Result<(), String> {
        let all_works = self
            .readarr_import_service
            .list_works(self.user_id)
            .await
            .map_err(|e| format!("list works: {e}"))?;

        for rd_book in active_books {
            let author_name = author_map
                .get(&rd_book.author_id)
                .and_then(|a| a.author_name.as_deref())
                .unwrap_or("");
            let title = rd_book.title.as_deref().unwrap_or("").trim();

            if author_name.is_empty() {
                let mut prog = self.progress().lock().await;
                prog.works_processed += 1;
                prog.errors
                    .push(format!("Book '{title}': skipped (no author)"));
                continue;
            }

            let livrarr_author_id = self.author_map_rd.get(&rd_book.author_id).copied();

            let edition = rd_book.monitored_edition();
            let isbn = edition
                .and_then(|e| e.isbn13.as_deref())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let asin = edition
                .and_then(|e| e.asin.as_deref())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let publisher = edition
                .and_then(|e| e.publisher.as_deref())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let language = edition
                .and_then(|e| e.language.as_deref())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let year = rd_book.release_date.as_deref().and_then(extract_year);

            let norm_title = normalize_for_matching(title);
            let norm_author = normalize_for_matching(author_name);

            let existing_work = if let Some(ref isbn_val) = isbn {
                all_works
                    .iter()
                    .find(|w| w.isbn_13.as_deref() == Some(isbn_val))
            } else if let Some(ref asin_val) = asin {
                all_works
                    .iter()
                    .find(|w| w.asin.as_deref() == Some(asin_val))
            } else {
                all_works.iter().find(|w| {
                    normalize_for_matching(&w.author_name) == norm_author
                        && normalize_for_matching(&w.title) == norm_title
                        && w.year == year
                })
            };

            let work_id = if let Some(ew) = existing_work {
                ew.id
            } else {
                match self
                    .create_work(
                        rd_book,
                        author_name,
                        title,
                        livrarr_author_id,
                        isbn.clone(),
                        asin.clone(),
                        publisher,
                        language,
                        year,
                        book_files_by_book,
                        livrarr_root_path,
                    )
                    .await
                {
                    Some(id) => id,
                    None => {
                        let mut prog = self.progress().lock().await;
                        prog.works_processed += 1;
                        continue;
                    }
                }
            };

            self.work_map_rd.insert(rd_book.id, work_id);

            {
                let mut prog = self.progress().lock().await;
                prog.works_processed += 1;
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_work(
        &mut self,
        rd_book: &RdBook,
        author_name: &str,
        title: &str,
        livrarr_author_id: Option<i64>,
        isbn: Option<String>,
        asin: Option<String>,
        publisher: Option<String>,
        language: Option<String>,
        year: Option<i32>,
        book_files_by_book: &HashMap<i64, Vec<&RdBookFile>>,
        _livrarr_root_path: &str,
    ) -> Option<i64> {
        let edition = rd_book.monitored_edition();
        let description = rd_book
            .overview
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| {
                edition
                    .and_then(|e| e.overview.as_deref())
                    .filter(|s| !s.is_empty())
            })
            .map(|s| s.to_string());

        let (series_name, series_position) = rd_book
            .series_title
            .as_deref()
            .map(parse_series_title)
            .unwrap_or((None, None));

        let genres = rd_book.genres.clone();
        let page_count = rd_book
            .page_count
            .or_else(|| edition.and_then(|e| e.page_count));
        let rating = rd_book.ratings.as_ref().and_then(|r| r.value);
        let rating_count = rd_book.ratings.as_ref().and_then(|r| r.votes);
        let cover_url = extract_cover_url(&rd_book.images);

        let book_files_list = book_files_by_book.get(&rd_book.id);
        let has_ebook_file = book_files_list
            .map(|fs| {
                fs.iter().any(|f| {
                    resolve_media_type(extract_quality_id(f), &f.path) == Some(MediaType::Ebook)
                })
            })
            .unwrap_or(false);
        let has_audiobook_file = book_files_list
            .map(|fs| {
                fs.iter().any(|f| {
                    resolve_media_type(extract_quality_id(f), &f.path) == Some(MediaType::Audiobook)
                })
            })
            .unwrap_or(false);

        let monitor_ebook = has_ebook_file || rd_book.monitored.unwrap_or(false);
        let monitor_audiobook = has_audiobook_file;

        match self
            .readarr_import_service
            .create_work(CreateWorkDbRequest {
                user_id: self.user_id,
                title: title.to_string(),
                author_name: author_name.to_string(),
                author_id: livrarr_author_id,
                ol_key: None,
                gr_key: None,
                year,
                cover_url: cover_url.clone(),
                metadata_source: Some("readarr".to_string()),
                detail_url: None,
                language: language.clone(),
                import_id: Some(self.import_id.clone()),
                series_id: None,
                series_name: None,
                series_position: None,
                monitor_ebook: false,
                monitor_audiobook: false,
            })
            .await
        {
            Ok(w) => {
                self.works_created += 1;

                let _ = self
                    .readarr_import_service
                    .update_work_enrichment(
                        self.user_id,
                        w.id,
                        UpdateWorkEnrichmentDbRequest {
                            title: None,
                            subtitle: None,
                            original_title: None,
                            author_name: None,
                            description,
                            year: None,
                            series_name,
                            series_position,
                            genres,
                            language: None,
                            page_count,
                            duration_seconds: None,
                            publisher,
                            publish_date: rd_book.release_date.clone(),
                            ol_key: None,
                            gr_key: None,
                            hc_key: None,
                            isbn_13: isbn,
                            asin,
                            narrator: None,
                            narration_type: None,
                            abridged: None,
                            rating,
                            rating_count,
                            enrichment_status: EnrichmentStatus::Pending,
                            enrichment_source: Some("readarr".to_string()),
                            cover_url: None,
                        },
                    )
                    .await;

                let _ = self
                    .readarr_import_service
                    .update_work_user_fields(
                        self.user_id,
                        w.id,
                        UpdateWorkUserFieldsDbRequest {
                            title: None,
                            author_name: None,
                            series_name: None,
                            series_position: None,
                            monitor_ebook: Some(monitor_ebook),
                            monitor_audiobook: Some(monitor_audiobook),
                        },
                    )
                    .await;

                if let Some(ref url) = cover_url {
                    let covers_dir = self.data_dir.join("covers");
                    let _ = tokio::fs::create_dir_all(&covers_dir).await;
                    if let Ok(resp) = self.http_client.get(url).send().await {
                        if resp.status().is_success() {
                            if let Ok(bytes) = resp.bytes().await {
                                let path = covers_dir.join(format!("{}.jpg", w.id));
                                let _ = tokio::fs::write(&path, &bytes).await;
                            }
                        }
                    }
                }

                self.pending_enrich_work_ids.push(w.id);

                Some(w.id)
            }
            Err(e) => {
                warn!(title = %title, "Failed to create work: {e}");
                let mut prog = self.progress().lock().await;
                prog.errors.push(format!("Work '{title}': {e}"));
                None
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn process_files(
        &mut self,
        rd_book_files: &[RdBookFile],
        active_book_ids: &HashSet<i64>,
        author_map: &HashMap<i64, &RdAuthor>,
        rd_books: &[RdBook],
        book_files_by_book: &HashMap<i64, Vec<&RdBookFile>>,
        readarr_root: &str,
        livrarr_root_path: &str,
    ) -> Result<(), String> {
        for rd_file in rd_book_files
            .iter()
            .filter(|f| active_book_ids.contains(&f.book_id))
        {
            let work_id = match self.work_map_rd.get(&rd_file.book_id) {
                Some(id) => *id,
                None => {
                    self.files_skipped += 1;
                    let mut prog = self.progress().lock().await;
                    prog.files_processed += 1;
                    prog.files_skipped += 1;
                    continue;
                }
            };

            let author_name = rd_file
                .author_id
                .and_then(|aid| author_map.get(&aid))
                .and_then(|a| a.author_name.as_deref())
                .unwrap_or("Unknown Author");

            let title = rd_books
                .iter()
                .find(|b| b.id == rd_file.book_id)
                .and_then(|b| b.title.as_deref())
                .unwrap_or("Unknown Title");

            let qid = extract_quality_id(rd_file);
            let media_type = match resolve_media_type(qid, &rd_file.path) {
                Some(mt) => mt,
                None => {
                    self.files_skipped += 1;
                    let mut prog = self.progress().lock().await;
                    prog.files_processed += 1;
                    prog.files_skipped += 1;
                    continue;
                }
            };

            if media_type == MediaType::Audiobook {
                let book_audio_count = book_files_by_book
                    .get(&rd_file.book_id)
                    .map(|fs| {
                        fs.iter()
                            .filter(|f| {
                                resolve_media_type(extract_quality_id(f), &f.path)
                                    == Some(MediaType::Audiobook)
                            })
                            .count()
                    })
                    .unwrap_or(0);
                if book_audio_count > 1 {
                    self.files_skipped += 1;
                    let mut prog = self.progress().lock().await;
                    prog.files_processed += 1;
                    prog.files_skipped += 1;
                    continue;
                }
            }

            let translated_path = apply_path_translation(
                &rd_file.path,
                self.req.container_path.as_deref(),
                self.req.host_path.as_deref(),
            );
            let source = match validate_source_path(&translated_path, readarr_root) {
                Ok(p) => p,
                Err(e) => {
                    warn!(path = %rd_file.path, "Source path validation failed: {e}");
                    self.files_skipped += 1;
                    let mut prog = self.progress().lock().await;
                    prog.files_processed += 1;
                    prog.files_skipped += 1;
                    prog.errors.push(format!("File '{}': {e}", rd_file.path));
                    continue;
                }
            };

            let dest = build_dest_path(
                livrarr_root_path,
                self.user_id,
                author_name,
                title,
                &rd_file.path,
            );

            if dest.exists() {
                self.files_skipped += 1;
                let mut prog = self.progress().lock().await;
                prog.files_processed += 1;
                prog.files_skipped += 1;
                continue;
            }

            if let Err(e) = materialize_file(&source, &dest) {
                warn!(src = %rd_file.path, dest = %dest.display(), "File materialization failed: {e}");
                self.files_skipped += 1;
                let mut prog = self.progress().lock().await;
                prog.files_processed += 1;
                prog.files_skipped += 1;
                prog.errors.push(format!("File '{}': {e}", rd_file.path));
                continue;
            }

            let rel_path = dest
                .strip_prefix(livrarr_root_path)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| dest.to_string_lossy().to_string());

            match self
                .readarr_import_service
                .create_library_item(CreateLibraryItemDbRequest {
                    user_id: self.user_id,
                    work_id,
                    root_folder_id: self.req.livrarr_root_folder_id,
                    path: rel_path,
                    media_type,
                    file_size: rd_file.size,
                    import_id: Some(self.import_id.clone()),
                })
                .await
            {
                Ok(_) => {
                    self.files_imported += 1;
                }
                Err(crate::readarr_import_service::ReadarrImportError::Db(
                    livrarr_domain::DbError::Constraint { .. },
                )) => {
                    self.files_skipped += 1;
                }
                Err(e) => {
                    warn!(path = %rd_file.path, "Failed to create library item: {e}");
                    self.files_skipped += 1;
                    let mut prog = self.progress().lock().await;
                    prog.errors
                        .push(format!("LibraryItem for '{}': {e}", rd_file.path));
                }
            }

            {
                let mut prog = self.progress().lock().await;
                prog.files_processed += 1;
            }
        }
        Ok(())
    }
}

// =============================================================================
// Post-import enrichment (bounded concurrency, detached from import completion)
// =============================================================================

async fn run_post_import_enrichment(
    import_id: &str,
    user_id: i64,
    work_ids: Vec<WorkId>,
    enrichment_workflow: Arc<LiveEnrichmentWorkflow>,
    work_service: Arc<LiveWorkService>,
) {
    let gate = Arc::new(Semaphore::new(POST_IMPORT_ENRICH_CONCURRENCY));
    let mut join_set = tokio::task::JoinSet::new();

    for work_id in &work_ids {
        let permit = match gate.clone().acquire_owned().await {
            Ok(p) => p,
            Err(_) => break,
        };
        let ew = enrichment_workflow.clone();
        let ws = work_service.clone();
        let wid = *work_id;

        join_set.spawn(async move {
            let _permit = permit;
            enrich_one_imported_work(user_id, wid, &ew, &ws).await;
        });
    }

    while let Some(joined) = join_set.join_next().await {
        if let Err(e) = joined {
            warn!(%e, "post-import enrichment task panicked");
        }
    }

    debug!(
        import_id,
        count = work_ids.len(),
        "post-import enrichment finished"
    );
}

async fn enrich_one_imported_work(
    user_id: i64,
    work_id: WorkId,
    enrichment_workflow: &LiveEnrichmentWorkflow,
    work_service: &LiveWorkService,
) {
    use livrarr_domain::services::{EnrichmentMode, EnrichmentWorkflow, WorkService};

    match tokio::time::timeout(
        std::time::Duration::from_secs(30),
        enrichment_workflow.enrich_work(user_id, work_id, EnrichmentMode::Background),
    )
    .await
    {
        Ok(Ok(result)) => {
            if !result.work.cover_manual {
                if let Some(ref cover_url) = result.work.cover_url {
                    if let Err(e) = work_service
                        .download_cover_from_url(user_id, work_id, cover_url)
                        .await
                    {
                        warn!(work_id, %e, "cover download failed after post-import enrichment");
                    }
                }
            }
        }
        Ok(Err(e)) => {
            warn!(work_id, %e, "post-import enrich_work failed");
        }
        Err(_) => {
            warn!(work_id, "post-import enrich_work timed out");
        }
    }
}
