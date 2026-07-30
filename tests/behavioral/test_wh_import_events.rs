#![allow(dead_code, unused_imports)]

//! RED behavioral tests for work-history import/grab event writers.
//!
//! Tests-review disposition (r1, google R-3): the seven BLOCKED todo!()
//! placeholders claimed handler-context unreachability, but the suite's own
//! `test_wh_identity_events` builds exactly such a harness (a TestState with
//! narrow `Has*` impls driving handler fns directly), and
//! `test_consolidation_import_workflow` already drives the real
//! `ImportWorkflowImpl<SqliteDb>` grab road. All seven are realized here
//! against those real seams: the manual-import confirm loop and root-folder
//! adopt handler run with a scripted domain `ImportService` (the writer under
//! test is the HANDLER's match over the outcome, per ir-v2), and the batch
//! test drives the real grab-import road end to end.

use std::sync::{Arc, Mutex};

use axum::extract::{Path, State};
use livrarr_behavioral::stubs::{
    StubEnrichmentWorkflow, StubHttpFetcher, TagwriteChapterExtractor,
};
use livrarr_db::sqlite::SqliteDb;
use livrarr_db::test_helpers::create_test_db;
use livrarr_db::{
    CreateDownloadClientDbRequest, CreateGrabDbRequest, CreateHistoryEventDbRequest,
    CreateLibraryItemDbRequest, CreateUserDbRequest, CreateWorkDbRequest, DownloadClientDb, GrabDb,
    HistoryDb, LibraryItemDb, RootFolderDb, TagStatus, UserDb, WorkDb, WorkDbCreate,
};
use livrarr_domain::services::{
    AddAuthorRequest, AddAuthorResult, AdoptScannedFileRequest, AuthorLookupResult,
    AuthorMergeReport, AuthorService, AuthorServiceError, BibliographyResult, DiscoveryService,
    DownloadProtocol, EagerQuery, FetchResponse, GrabRequest, GrabSource, ImportFileOutcome,
    ImportFileResult, ImportGrabResult, ImportService, ImportSingleFileRequest, ImportWorkflow,
    ImportWorkflowError, LookupRequest, LookupResponse, LookupResult, MatchCluster, MatchInput,
    MatchingService, ReleaseService, ServiceError, SkipReason, UpdateAuthorRequest,
    WorkServiceError,
};
use livrarr_domain::{
    AuthType, Author, AuthorId, DownloadClientImplementation, EventType, GrabStatus, HistoryFilter,
    MediaType, UserId, UserRole,
};
use livrarr_download::release_service::ReleaseServiceImpl;
use livrarr_handlers::accessors::ManualImportScanAccessor;
use livrarr_handlers::context::{
    HasAppConfigService, HasAuthorService, HasDiscoveryService, HasFileService, HasHistoryService,
    HasImportService, HasManualImportScan, HasManualImportService, HasMatchingService,
    HasRootFolderService, HasWorkService,
};
use livrarr_handlers::manual_import::{self, ImportItem, ImportRequest, ImportStatus};
use livrarr_handlers::middleware::RequireAdmin;
use livrarr_handlers::root_folder;
use livrarr_handlers::AuthContext;
use livrarr_library::file_service::FileServiceImpl;
use livrarr_library::import_workflow::ImportWorkflowImpl;
use livrarr_metadata::work_service::WorkServiceImpl;
use livrarr_server::history_service::HistoryServiceImpl;
use livrarr_server::manual_import_service::ManualImportServiceImpl;
use livrarr_server::services::settings_service::LiveSettingsService;

fn trusted_origins() -> Arc<livrarr_http::ssrf::TrustedOrigins> {
    let origins = Arc::new(livrarr_http::ssrf::TrustedOrigins::new());
    origins.rebuild(&["http://indexer.test".to_string()]);
    origins
}

fn empty_history_filter() -> HistoryFilter {
    HistoryFilter {
        event_type: None,
        work_id: None,
        start_date: None,
        end_date: None,
    }
}

async fn setup_user(db: &SqliteDb) -> i64 {
    db.create_user(CreateUserDbRequest {
        username: "wh_import_user".into(),
        password_hash: "hash".into(),
        role: UserRole::Admin,
        api_key_hash: "api_hash".into(),
    })
    .await
    .unwrap()
    .id
}

async fn setup_work(db: &SqliteDb, user_id: i64, title: &str, author: &str) -> i64 {
    db.create_work(CreateWorkDbRequest {
        user_id,
        title: title.into(),
        author_name: author.into(),
        ..Default::default()
    })
    .await
    .unwrap()
    .0
    .id
}

async fn setup_qbit_client(db: &SqliteDb) -> i64 {
    db.create_download_client(CreateDownloadClientDbRequest {
        name: "qbit".into(),
        implementation: DownloadClientImplementation::QBittorrent,
        host: "qbit.test".into(),
        port: 8080,
        use_ssl: false,
        skip_ssl_validation: false,
        url_base: None,
        username: Some("user".into()),
        password: Some("pass".into()),
        category: "livrarr".into(),
        download_dir: None,
        enabled: true,
        api_key: None,
    })
    .await
    .unwrap()
    .id
}

/// Scripted responses for a full successful torrent grab: the torrent-file
/// fetch (minimal bencoded body — the indexer-citizenship pins' fixture),
/// then qBit auth (SID cookie), then the add call. The download URL must sit
/// on the trusted origin: `grab` SSRF-rejects untrusted URLs at entry, and a
/// raw `magnet:` URL never passes it (fixture fix at the tests-review
/// disposition — the original magnet fixture died at the unwrap and could
/// never reach the event writer).
fn grab_success_http() -> StubHttpFetcher {
    let http = StubHttpFetcher::with_response(Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: b"d4:infod4:name4:pinee".to_vec(),
    }));
    http.push_response(Ok(FetchResponse {
        status: 200,
        headers: vec![("set-cookie".into(), "SID=test-cookie; HttpOnly".into())],
        body: Vec::new(),
    }));
    http.push_response(Ok(FetchResponse {
        status: 200,
        headers: vec![],
        body: b"Ok.".to_vec(),
    }));
    http
}

// ---------------------------------------------------------------------------
// Handler-door harness (identity-events pattern): a TestState carrying real
// SqliteDb-backed services where the road reads/writes state, and scripted
// doubles where the road's outcome is the very thing the test controls.
// ---------------------------------------------------------------------------

type TestWorkService = WorkServiceImpl<SqliteDb, StubEnrichmentWorkflow, StubHttpFetcher>;

/// Scripted domain `ImportService`: the confirm loop's and adopt road's
/// outcome source. The handler writer under test matches over what this
/// returns; unrelated methods are unreachable by construction.
#[derive(Default)]
struct ScriptedImportService {
    single: Mutex<Option<ImportFileResult>>,
    adopt: Mutex<Option<Result<ImportFileOutcome, ImportWorkflowError>>>,
}

impl ImportService for ScriptedImportService {
    async fn import_grab(
        &self,
        _user_id: i64,
        _grab_id: i64,
    ) -> Result<ImportGrabResult, ServiceError> {
        unreachable!("import_grab is not driven through the handler harness")
    }

    async fn import_single_file(&self, _req: ImportSingleFileRequest) -> ImportFileResult {
        self.single
            .lock()
            .unwrap()
            .take()
            .expect("test must script the import_single_file result")
    }

    async fn adopt_scanned_file(
        &self,
        _user_id: i64,
        _req: AdoptScannedFileRequest,
    ) -> Result<ImportFileOutcome, ImportWorkflowError> {
        self.adopt
            .lock()
            .unwrap()
            .take()
            .expect("test must script the adopt_scanned_file result")
    }

    async fn reorganize_work_files(
        &self,
        _user_id: i64,
        _work_id: i64,
    ) -> Result<Vec<String>, ServiceError> {
        unreachable!("reorganize_work_files is not driven through the handler harness")
    }

    fn build_target_path(
        &self,
        _root_folder_path: &str,
        _user_id: i64,
        author: &str,
        title: &str,
        _media_type: MediaType,
        _source: &std::path::Path,
        _source_root: &std::path::Path,
    ) -> String {
        format!("{author}/{title}.epub")
    }
}

/// Author lookups on the confirm road are a best-effort OL enrichment step —
/// an empty result is a legal, network-free answer. Everything else on the
/// trait is unreachable from the driven roads.
struct ScriptedAuthorService;

impl AuthorService for ScriptedAuthorService {
    async fn add(
        &self,
        _user_id: UserId,
        _req: AddAuthorRequest,
    ) -> Result<AddAuthorResult, AuthorServiceError> {
        unreachable!("author add is not driven by these tests")
    }
    async fn merge(
        &self,
        _user_id: UserId,
        _survivor_id: AuthorId,
        _loser_id: AuthorId,
    ) -> Result<AuthorMergeReport, AuthorServiceError> {
        unreachable!("author merge is not driven by these tests")
    }
    async fn get(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<Author, AuthorServiceError> {
        unreachable!("author get is not driven by these tests")
    }
    async fn list(&self, _user_id: UserId) -> Result<Vec<Author>, AuthorServiceError> {
        unreachable!("author list is not driven by these tests")
    }
    async fn update(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _req: UpdateAuthorRequest,
    ) -> Result<Author, AuthorServiceError> {
        unreachable!("author update is not driven by these tests")
    }
    async fn delete(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<(), AuthorServiceError> {
        unreachable!("author delete is not driven by these tests")
    }
    async fn lookup(
        &self,
        _query: &str,
        _limit: u32,
    ) -> Result<Vec<AuthorLookupResult>, AuthorServiceError> {
        Ok(vec![])
    }
    async fn search(
        &self,
        _user_id: UserId,
        _query: &str,
    ) -> Result<Vec<Author>, AuthorServiceError> {
        unreachable!("author search is not driven by these tests")
    }
    async fn bibliography(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _raw: bool,
    ) -> Result<BibliographyResult, AuthorServiceError> {
        unreachable!("bibliography is not driven by these tests")
    }
    async fn refresh_bibliography(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
    ) -> Result<BibliographyResult, AuthorServiceError> {
        unreachable!("refresh_bibliography is not driven by these tests")
    }
    fn spawn_bibliography_refresh(&self, _author_id: i64, _user_id: i64) {}
    async fn lookup_authors(
        &self,
        _term: &str,
        _limit: u32,
    ) -> Result<Vec<AuthorLookupResult>, AuthorServiceError> {
        Ok(vec![])
    }
    async fn rename(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _name: String,
    ) -> Result<Author, AuthorServiceError> {
        unreachable!("author rename is not driven by these tests")
    }
    async fn select_name_variant(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _variant_id: i64,
    ) -> Result<Author, AuthorServiceError> {
        unreachable!("author name selection is not driven by these tests")
    }
    async fn set_monitoring(
        &self,
        _user_id: UserId,
        _author_id: AuthorId,
        _monitored: bool,
        _monitor_new_items: Option<bool>,
        _monitor_language: Option<String>,
    ) -> Result<Author, AuthorServiceError> {
        unreachable!("author monitoring is not driven by these tests")
    }
}

/// File-embedded metadata extraction: an empty read is the legal no-metadata
/// answer, so the fixture file never needs real embedded tags.
struct ScriptedMatchingService;

impl MatchingService for ScriptedMatchingService {
    async fn extract_and_reconcile(&self, _input: &MatchInput) -> Vec<MatchCluster> {
        vec![]
    }
}

/// Discovery is never consulted on the confirm/adopt roads — reaching it is a
/// test failure, not a stubbing gap.
struct PanicDiscoveryService;

impl DiscoveryService for PanicDiscoveryService {
    async fn lookup(&self, _req: LookupRequest) -> Result<Vec<LookupResult>, WorkServiceError> {
        unreachable!("discovery must not run on the manual-import confirm road")
    }
    async fn lookup_filtered(
        &self,
        _user_id: UserId,
        _req: LookupRequest,
        _raw: bool,
    ) -> Result<LookupResponse, WorkServiceError> {
        unreachable!("discovery must not run on the manual-import confirm road")
    }
    async fn eager_match_by_author(
        &self,
        _user_id: UserId,
        _queries: Vec<EagerQuery>,
    ) -> Result<Vec<(usize, LookupResult)>, WorkServiceError> {
        unreachable!("discovery must not run on the manual-import confirm road")
    }
}

/// Scan-progress state is a scan-endpoint concern; the confirm/adopt roads
/// never touch it.
struct NoScanState;

impl ManualImportScanAccessor for NoScanState {
    fn insert_scan(
        &self,
        _scan_id: String,
        _user_id: i64,
        _files: Vec<manual_import::ScannedFile>,
        _warnings: Vec<String>,
        _ol_total: usize,
    ) {
    }
    fn get_scan(&self, _scan_id: &str) -> Option<manual_import::ScanSnapshot> {
        None
    }
    fn update_scan_file(
        &self,
        _scan_id: &str,
        _file_idx: usize,
        _update: manual_import::ScanFileUpdate,
    ) {
    }
    fn increment_ol_completed(&self, _scan_id: &str) {}
    fn remove_scan(&self, _scan_id: &str) {}
}

#[derive(Clone)]
struct TestState {
    work_service: Arc<TestWorkService>,
    manual_import: Arc<ManualImportServiceImpl<SqliteDb>>,
    settings: Arc<LiveSettingsService<SqliteDb>>,
    history: Arc<HistoryServiceImpl<SqliteDb>>,
    files: Arc<FileServiceImpl<SqliteDb>>,
    imports: Arc<ScriptedImportService>,
    authors: Arc<ScriptedAuthorService>,
    matching: Arc<ScriptedMatchingService>,
    discovery: Arc<PanicDiscoveryService>,
    scan_state: Arc<NoScanState>,
}

fn test_state(db: &SqliteDb) -> TestState {
    TestState {
        work_service: Arc::new(WorkServiceImpl::new(
            db.clone(),
            StubEnrichmentWorkflow::succeeding(),
            StubHttpFetcher::new(),
            tempfile::tempdir().expect("test data dir").keep(),
        )),
        manual_import: Arc::new(ManualImportServiceImpl::new(db.clone())),
        settings: Arc::new(LiveSettingsService::new(db.clone())),
        history: Arc::new(HistoryServiceImpl::new(db.clone())),
        files: Arc::new(FileServiceImpl::new(db.clone())),
        imports: Arc::new(ScriptedImportService::default()),
        authors: Arc::new(ScriptedAuthorService),
        matching: Arc::new(ScriptedMatchingService),
        discovery: Arc::new(PanicDiscoveryService),
        scan_state: Arc::new(NoScanState),
    }
}

impl HasWorkService for TestState {
    type WorkSvc = TestWorkService;
    fn work_service(&self) -> &Self::WorkSvc {
        &self.work_service
    }
}

impl HasManualImportService for TestState {
    type ManualImportSvc = ManualImportServiceImpl<SqliteDb>;
    fn manual_import_service(&self) -> &Self::ManualImportSvc {
        &self.manual_import
    }
}

impl HasAppConfigService for TestState {
    type AppConfigSvc = LiveSettingsService<SqliteDb>;
    fn app_config_service(&self) -> &Self::AppConfigSvc {
        &self.settings
    }
}

impl HasRootFolderService for TestState {
    type RootFolderSvc = LiveSettingsService<SqliteDb>;
    fn root_folder_service(&self) -> &Self::RootFolderSvc {
        &self.settings
    }
}

impl HasHistoryService for TestState {
    type HistorySvc = HistoryServiceImpl<SqliteDb>;
    fn history_service(&self) -> &Self::HistorySvc {
        &self.history
    }
}

impl HasFileService for TestState {
    type FileSvc = FileServiceImpl<SqliteDb>;
    fn file_service(&self) -> &Self::FileSvc {
        &self.files
    }
}

impl HasImportService for TestState {
    type ImportSvc = ScriptedImportService;
    fn import_service(&self) -> &Self::ImportSvc {
        &self.imports
    }
}

impl HasAuthorService for TestState {
    type AuthorSvc = ScriptedAuthorService;
    fn author_service(&self) -> &Self::AuthorSvc {
        &self.authors
    }
}

impl HasMatchingService for TestState {
    type MatchingSvc = ScriptedMatchingService;
    fn matching_service(&self) -> &Self::MatchingSvc {
        &self.matching
    }
}

impl HasDiscoveryService for TestState {
    type DiscoverySvc = PanicDiscoveryService;
    fn discovery_service(&self) -> &Self::DiscoverySvc {
        &self.discovery
    }
}

impl HasManualImportScan for TestState {
    type ManualImportScan = NoScanState;
    fn manual_import_scan(&self) -> &Self::ManualImportScan {
        &self.scan_state
    }
}

async fn admin_auth(db: &SqliteDb, user_id: i64) -> RequireAdmin {
    RequireAdmin(AuthContext {
        user: db.get_user(user_id).await.expect("auth user"),
        auth_type: AuthType::Session,
        session_token_hash: Some("wh-import-session".to_string()),
    })
}

fn import_item(path: &str, ol_key: &str, title: &str, author: &str) -> ImportItem {
    ImportItem {
        path: path.to_string(),
        ol_key: ol_key.to_string(),
        title: title.to_string(),
        author: author.to_string(),
        delete_existing: false,
        language: Some("en".to_string()),
        author_ol_key: None,
        year: None,
        cover_url: None,
        isbn: None,
        description: None,
        series_name: None,
        series_position: None,
        candidate_id: None,
        hc_key: None,
        gr_key: None,
        asin: None,
    }
}

// ---------------------------------------------------------------------------
// Existing-writer migration: the grabbed event (REQ-013)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wh_grab_writes_one_grabbed_event_with_preserved_release_payload_plus_work_title() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id, "Work Title", "Work Author").await;
    let client_id = setup_qbit_client(&db).await;

    let svc = ReleaseServiceImpl::new(db.clone(), grab_success_http(), trusted_origins());
    svc.grab(
        user_id,
        GrabRequest {
            work_id,
            download_url: "http://indexer.test/wh-release.torrent".into(),
            title: "Release Title".into(),
            indexer: "Indexer Name".into(),
            guid: "guid-123".into(),
            size: 12345,
            protocol: DownloadProtocol::Torrent,
            categories: vec![7020],
            download_client_id: Some(client_id),
            source: GrabSource::Manual,
        },
    )
    .await
    .unwrap();

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    let grabbed: Vec<_> = history
        .iter()
        .filter(|event| event.event_type == EventType::Grabbed)
        .collect();

    assert_eq!(grabbed.len(), 1);
    assert_eq!(grabbed[0].work_id, Some(work_id));
    assert_eq!(grabbed[0].data["guid"], "guid-123");
    assert_eq!(grabbed[0].data["title"], "Release Title");
    assert_eq!(grabbed[0].data["indexer"], "Indexer Name");
    assert_eq!(grabbed[0].data["download_client_id"], client_id);
    assert_eq!(grabbed[0].data["work_title"], "Work Title");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wh_rss_road_grab_still_writes_exactly_one_grabbed_event() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id, "RSS Work", "RSS Author").await;
    let client_id = setup_qbit_client(&db).await;

    let svc = ReleaseServiceImpl::new(db.clone(), grab_success_http(), trusted_origins());
    svc.grab(
        user_id,
        GrabRequest {
            work_id,
            download_url: "http://indexer.test/wh-rss-release.torrent".into(),
            title: "RSS Release".into(),
            indexer: "RSS Indexer".into(),
            guid: "rss-guid".into(),
            size: 67890,
            protocol: DownloadProtocol::Torrent,
            categories: vec![7020],
            download_client_id: Some(client_id),
            source: GrabSource::RssSync,
        },
    )
    .await
    .unwrap();

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    assert_eq!(
        history
            .iter()
            .filter(|event| event.event_type == EventType::Grabbed)
            .count(),
        1
    );
}

// ---------------------------------------------------------------------------
// Grab-road batch import: payload migration (REQ-013)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wh_grab_road_import_batch_payload_keeps_existing_keys_and_adds_work_title() {
    // Realized at disposition: ImportWorkflowImpl is generic over the db alone,
    // and the suite already drives import_grab end to end
    // (test_consolidation_import_workflow) — this mirrors that harness.
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let work_id = setup_work(&db, user_id, "Batch Work", "Batch Author").await;
    let client_id = setup_qbit_client(&db).await;

    let source_dir = tempfile::tempdir().expect("source dir");
    std::fs::write(source_dir.path().join("book.epub"), b"grab import fixture")
        .expect("source file");
    let library_dir = tempfile::tempdir().expect("library dir");
    db.create_root_folder(library_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .expect("root folder");

    let grab = db
        .upsert_grab(CreateGrabDbRequest {
            user_id,
            work_id,
            download_client_id: client_id,
            title: "Batch Release".into(),
            indexer: "Batch Indexer".into(),
            guid: "wh-batch-guid".into(),
            size: None,
            download_url: "magnet:?xt=urn:btih:feedfacefeedfacefeedfacefeedfacefeedface".into(),
            download_id: Some("hash-batch".into()),
            status: GrabStatus::Confirmed,
            media_type: None,
        })
        .await
        .expect("grab");
    db.set_grab_content_path(user_id, grab.id, source_dir.path().to_str().unwrap())
        .await
        .expect("content path");

    let wf = ImportWorkflowImpl::new(
        db.clone(),
        Arc::new(tokio::sync::Semaphore::new(2)),
        Arc::new(std::path::PathBuf::from("/tmp/livrarr-wh-batch")),
        Arc::new(TagwriteChapterExtractor),
    );
    let result = wf.import_grab(user_id, grab.id).await.expect("grab import");
    assert_eq!(result.final_status, GrabStatus::Imported);

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    let imported: Vec<_> = history
        .iter()
        .filter(|event| event.event_type == EventType::Imported)
        .collect();
    assert_eq!(imported.len(), 1, "one batch event per grab import");
    let data = &imported[0].data;
    assert_eq!(
        data["title"], "Batch Release",
        "release-title key preserved"
    );
    assert_eq!(data["imported"], 1);
    assert_eq!(data["failed"], 0);
    assert_eq!(data["skipped"], 0);
    assert_eq!(
        data["work_title"], "Batch Work",
        "REQ-013: the migrated batch payload gains the work-title snapshot"
    );
    assert_eq!(data["work_author"], "Batch Author");
}

// ---------------------------------------------------------------------------
// Manual-import confirm loop (REQ-003)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn wh_manual_success_writes_one_per_file_imported_event() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let books = tempfile::tempdir().expect("books dir");
    db.create_root_folder(books.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .expect("root folder");
    let state = test_state(&db);
    *state.imports.single.lock().unwrap() = Some(ImportFileResult::Ok);

    let source = books.path().join("Manual Author - Manual Work.epub");
    let response = manual_import::import(
        State(state.clone()),
        admin_auth(&db, user_id).await,
        axum::Json(ImportRequest {
            items: vec![import_item(
                source.to_str().unwrap(),
                "OL-WH-MANUAL-1W",
                "Manual Work",
                "Manual Author",
            )],
        }),
    )
    .await
    .expect("manual import confirm");
    assert_eq!(response.0.results.len(), 1);
    assert!(matches!(
        response.0.results[0].status,
        ImportStatus::Imported
    ));
    let work_id = response.0.results[0]
        .work_id
        .expect("imported result carries the resolved work");

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    let imported: Vec<_> = history
        .iter()
        .filter(|event| event.event_type == EventType::Imported)
        .collect();
    assert_eq!(
        imported.len(),
        1,
        "one per-file imported event for the confirmed file"
    );
    assert_eq!(imported[0].work_id, Some(work_id));
    assert_eq!(imported[0].data["path"], source.to_str().unwrap());
    assert_eq!(imported[0].data["media_type"], "ebook");
    assert!(
        imported[0].data["work_title"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "per-file payload snapshots the work title"
    );
    assert!(
        history
            .iter()
            .all(|event| event.event_type != EventType::ImportFailed),
        "a clean confirm writes no importFailed"
    );
}

#[tokio::test]
async fn wh_manual_early_failure_writes_unattached_import_failed_without_work_title() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let state = test_state(&db);
    // Nothing scripted: the road must fail at classification, before any
    // work creation or import-service call.

    let response = manual_import::import(
        State(state.clone()),
        admin_auth(&db, user_id).await,
        axum::Json(ImportRequest {
            items: vec![import_item(
                "mystery-file.xyz",
                "",
                "Unknown Work",
                "Unknown Author",
            )],
        }),
    )
    .await
    .expect("confirm reports per-file failures, not an HTTP error");
    assert!(matches!(response.0.results[0].status, ImportStatus::Failed));
    assert_eq!(response.0.results[0].work_id, None);

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    let failed: Vec<_> = history
        .iter()
        .filter(|event| event.event_type == EventType::ImportFailed)
        .collect();
    assert_eq!(
        failed.len(),
        1,
        "the early (unrecognized-media) failure records the unattached importFailed shape"
    );
    assert_eq!(
        failed[0].work_id, None,
        "no work was ever resolved — the row is unattached"
    );
    assert_eq!(failed[0].data["path"], "mystery-file.xyz");
    assert!(
        failed[0].data["error"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "unattached failure payload names the error"
    );
    assert!(
        failed[0].data.get("work_title").is_none(),
        "unattached failures carry no work_title key"
    );
}

#[tokio::test]
async fn wh_manual_late_failure_writes_attached_import_failed_with_work_title() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let books = tempfile::tempdir().expect("books dir");
    db.create_root_folder(books.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .expect("root folder");
    let state = test_state(&db);
    *state.imports.single.lock().unwrap() = Some(ImportFileResult::Failed(
        "scripted copy failure".to_string(),
    ));

    let source = books.path().join("Late Failure Work.epub");
    let response = manual_import::import(
        State(state.clone()),
        admin_auth(&db, user_id).await,
        axum::Json(ImportRequest {
            items: vec![import_item(
                source.to_str().unwrap(),
                "OL-WH-MANUAL-LATE-1W",
                "Late Failure Work",
                "Manual Author",
            )],
        }),
    )
    .await
    .expect("manual import confirm");
    assert!(matches!(response.0.results[0].status, ImportStatus::Failed));
    let work_id = response.0.results[0]
        .work_id
        .expect("the late failure happened after the work was resolved");

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    let failed: Vec<_> = history
        .iter()
        .filter(|event| event.event_type == EventType::ImportFailed)
        .collect();
    assert_eq!(
        failed.len(),
        1,
        "a late failure records the work-attached importFailed shape"
    );
    assert_eq!(failed[0].work_id, Some(work_id));
    assert_eq!(failed[0].data["path"], source.to_str().unwrap());
    assert!(
        failed[0].data["error"]
            .as_str()
            .is_some_and(|e| e.contains("scripted copy failure")),
        "attached failure payload carries the import error"
    );
    assert!(
        failed[0].data["work_title"]
            .as_str()
            .is_some_and(|s| !s.is_empty()),
        "attached failures snapshot the work title"
    );
}

// ---------------------------------------------------------------------------
// Root-folder adopt road (REQ-003 "or adopted")
// ---------------------------------------------------------------------------

/// Root layout the scan walk expects for ebooks: `{root}/{user_id}/{Author}/{Title}.epub`.
async fn adopt_fixture(db: &SqliteDb, user_id: i64) -> (tempfile::TempDir, i64, i64, String) {
    let root_dir = tempfile::tempdir().expect("root dir");
    let author_dir = root_dir
        .path()
        .join(user_id.to_string())
        .join("Adopt Author");
    std::fs::create_dir_all(&author_dir).expect("author dir");
    let file_path = author_dir.join("Adopt Work.epub");
    std::fs::write(&file_path, b"adopt fixture bytes").expect("epub file");
    let root = db
        .create_root_folder(root_dir.path().to_str().unwrap(), MediaType::Ebook)
        .await
        .expect("root folder");
    let work_id = setup_work(db, user_id, "Adopt Work", "Adopt Author").await;
    (
        root_dir,
        root.id,
        work_id,
        file_path.to_string_lossy().into_owned(),
    )
}

#[tokio::test]
async fn wh_adopt_scanned_file_writes_imported_event_for_adopted_or_imported_outcome() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_root_dir, root_id, work_id, file_path) = adopt_fixture(&db, user_id).await;
    let state = test_state(&db);
    *state.imports.adopt.lock().unwrap() = Some(Ok(ImportFileOutcome::Adopted {
        item_id: 4242,
        path: "Adopt Author/Adopt Work.epub".into(),
    }));

    let result = root_folder::scan(
        State(state.clone()),
        admin_auth(&db, user_id).await,
        Path(root_id),
    )
    .await
    .expect("scan");
    assert_eq!(result.0.matched, 1);

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    let imported: Vec<_> = history
        .iter()
        .filter(|event| event.event_type == EventType::Imported)
        .collect();
    assert_eq!(
        imported.len(),
        1,
        "one per-file imported event per adoption"
    );
    assert_eq!(imported[0].work_id, Some(work_id));
    assert_eq!(imported[0].data["path"], file_path);
    assert_eq!(imported[0].data["work_title"], "Adopt Work");
}

#[tokio::test]
async fn wh_adopt_skipped_outcome_records_zero_events() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_root_dir, root_id, _work_id, _file_path) = adopt_fixture(&db, user_id).await;
    let state = test_state(&db);
    *state.imports.adopt.lock().unwrap() = Some(Ok(ImportFileOutcome::Skipped {
        reason: SkipReason::AlreadyImported,
    }));

    let result = root_folder::scan(
        State(state.clone()),
        admin_auth(&db, user_id).await,
        Path(root_id),
    )
    .await
    .expect("scan");
    assert_eq!(result.0.matched, 1, "a skip still counts as matched");

    assert!(
        db.list_history(user_id, empty_history_filter())
            .await
            .unwrap()
            .is_empty(),
        "a Skipped adopt outcome imports nothing and records nothing"
    );
}

#[tokio::test]
async fn wh_adopt_error_writes_one_import_failed_event() {
    let db = create_test_db().await;
    let user_id = setup_user(&db).await;
    let (_root_dir, root_id, work_id, file_path) = adopt_fixture(&db, user_id).await;
    let state = test_state(&db);
    *state.imports.adopt.lock().unwrap() = Some(Err(ImportWorkflowError::PathCollision(
        "Adopt Author/Adopt Work.epub".into(),
    )));

    let result = root_folder::scan(
        State(state.clone()),
        admin_auth(&db, user_id).await,
        Path(root_id),
    )
    .await
    .expect("scan");
    assert_eq!(
        result.0.errors.len(),
        1,
        "the adopt error surfaces in the scan report"
    );

    let history = db
        .list_history(user_id, empty_history_filter())
        .await
        .unwrap();
    let failed: Vec<_> = history
        .iter()
        .filter(|event| event.event_type == EventType::ImportFailed)
        .collect();
    assert_eq!(failed.len(), 1, "an adopt error records one importFailed");
    assert_eq!(failed[0].work_id, Some(work_id));
    assert_eq!(failed[0].data["path"], file_path);
    assert!(
        failed[0].data["error"]
            .as_str()
            .is_some_and(|e| !e.is_empty()),
        "adopt-failure payload names the error"
    );
}
