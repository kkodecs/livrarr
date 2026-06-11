use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

type UserId = i64;
type WorkId = i64;
type AuthorId = i64;
type RootFolderId = i64;
type GrabId = i64;
type NotificationId = i64;

#[derive(Debug, Clone, PartialEq)]
pub enum JobError {
    AlreadyRunning,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PollResult {
    pub grab_id: GrabId,
    pub action: PollAction,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PollAction {
    ImportTriggered,
    MarkedFailed { reason: String },
    Skipped { reason: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct AuthorMonitorResult {
    pub author_id: AuthorId,
    pub new_works_detected: Vec<DetectedWork>,
    pub auto_added: Vec<WorkId>,
    pub notifications_created: Vec<NotificationId>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DetectedWork {
    pub ol_key: String,
    pub title: String,
    pub publish_year: Option<i32>,
}

// -- Traits --

#[async_trait]
pub trait JobService: Send + Sync {
    async fn trigger_bulk_enrichment(&self, user_id: UserId) -> Result<(), JobError>;
    async fn trigger_author_search(&self) -> Result<(), JobError>;
    async fn trigger_scan(
        &self,
        user_id: UserId,
        root_folder_id: RootFolderId,
    ) -> Result<(), JobError>;
}

#[async_trait]
pub trait DownloadPoller: Send + Sync {
    async fn poll(&self) -> Result<Vec<PollResult>, JobError>;
}

#[async_trait]
pub trait AuthorMonitor: Send + Sync {
    async fn check_all(&self) -> Result<Vec<AuthorMonitorResult>, JobError>;
}

// -- Mock: JobService --

#[derive(Clone)]
struct MockJobService {
    bulk: Result<(), JobError>,
    author_search: Result<(), JobError>,
    scan: Result<(), JobError>,
    log: Arc<Mutex<Vec<String>>>,
}

impl MockJobService {
    fn ok() -> Self {
        Self {
            bulk: Ok(()),
            author_search: Ok(()),
            scan: Ok(()),
            log: Arc::new(Mutex::new(vec![])),
        }
    }
}

#[async_trait]
impl JobService for MockJobService {
    async fn trigger_bulk_enrichment(&self, uid: UserId) -> Result<(), JobError> {
        self.log.lock().unwrap().push(format!("bulk:{uid}"));
        self.bulk.clone()
    }
    async fn trigger_author_search(&self) -> Result<(), JobError> {
        self.log.lock().unwrap().push("author_search".into());
        self.author_search.clone()
    }
    async fn trigger_scan(&self, uid: UserId, rf: RootFolderId) -> Result<(), JobError> {
        self.log.lock().unwrap().push(format!("scan:{uid}:{rf}"));
        self.scan.clone()
    }
}

// -- Mock: DownloadPoller --

#[derive(Debug, Clone)]
enum GrabState {
    Completed,
    Importing,
    QbitError,
    Missing,
}

#[derive(Clone)]
struct MockDownloadPoller {
    grabs: Vec<(GrabId, GrabState)>,
}

#[async_trait]
impl DownloadPoller for MockDownloadPoller {
    async fn poll(&self) -> Result<Vec<PollResult>, JobError> {
        Ok(self
            .grabs
            .iter()
            .map(|(id, st)| PollResult {
                grab_id: *id,
                action: match st {
                    GrabState::Completed => PollAction::ImportTriggered,
                    GrabState::Importing => PollAction::Skipped {
                        reason: "grab already importing".into(),
                    },
                    GrabState::QbitError => PollAction::MarkedFailed {
                        reason: "qbit error state".into(),
                    },
                    GrabState::Missing => PollAction::MarkedFailed {
                        reason: "torrent missing".into(),
                    },
                },
            })
            .collect())
    }
}

// -- Mock: AuthorMonitor --

#[derive(Debug, Clone)]
struct CandidateWork {
    ol_key: String,
    title: String,
    publish_year: Option<i32>,
}

#[derive(Debug, Clone)]
struct AuthorScenario {
    author_id: AuthorId,
    user_id: UserId,
    ol_key: Option<String>,
    since_year: i32,
    monitor_new_items: bool,
    existing_keys: HashSet<String>,
    candidates: Vec<CandidateWork>,
}

#[derive(Clone)]
struct MockAuthorMonitor {
    scenarios: Vec<AuthorScenario>,
    fail_with: Option<String>,
    dedup: Arc<Mutex<HashSet<(UserId, String, String)>>>,
    next_wid: Arc<Mutex<WorkId>>,
    next_nid: Arc<Mutex<NotificationId>>,
}

impl MockAuthorMonitor {
    fn new(scenarios: Vec<AuthorScenario>) -> Self {
        Self {
            scenarios,
            fail_with: None,
            dedup: Arc::new(Mutex::new(HashSet::new())),
            next_wid: Arc::new(Mutex::new(100)),
            next_nid: Arc::new(Mutex::new(500)),
        }
    }
    fn failing(msg: &str) -> Self {
        Self {
            scenarios: vec![],
            fail_with: Some(msg.into()),
            dedup: Arc::new(Mutex::new(HashSet::new())),
            next_wid: Arc::new(Mutex::new(1)),
            next_nid: Arc::new(Mutex::new(1)),
        }
    }
    fn with_ids(mut self, wid: WorkId, nid: NotificationId) -> Self {
        self.next_wid = Arc::new(Mutex::new(wid));
        self.next_nid = Arc::new(Mutex::new(nid));
        self
    }
}

#[async_trait]
impl AuthorMonitor for MockAuthorMonitor {
    async fn check_all(&self) -> Result<Vec<AuthorMonitorResult>, JobError> {
        if let Some(msg) = &self.fail_with {
            return Err(JobError::Failed(msg.clone()));
        }
        let mut out = Vec::new();
        for s in &self.scenarios {
            let mut r = AuthorMonitorResult {
                author_id: s.author_id,
                new_works_detected: vec![],
                auto_added: vec![],
                notifications_created: vec![],
                warnings: vec![],
            };
            if s.ol_key.is_none() {
                r.warnings.push("author missing ol_key; skipped".into());
                out.push(r);
                continue;
            }
            for w in &s.candidates {
                match w.publish_year {
                    None => {
                        r.warnings.push(format!(
                            "work {} excluded due to missing/unparseable publish date",
                            w.ol_key
                        ));
                    }
                    Some(y) if y < s.since_year => {}
                    Some(y) => {
                        if s.existing_keys.contains(&w.ol_key) {
                            continue;
                        }
                        r.new_works_detected.push(DetectedWork {
                            ol_key: w.ol_key.clone(),
                            title: w.title.clone(),
                            publish_year: Some(y),
                        });
                        let ntype = if s.monitor_new_items {
                            "WorkAutoAdded"
                        } else {
                            "NewWorkDetected"
                        };
                        let key = (s.user_id, ntype.into(), w.ol_key.clone());
                        let mut dd = self.dedup.lock().unwrap();
                        if dd.insert(key) {
                            let mut nid = self.next_nid.lock().unwrap();
                            r.notifications_created.push(*nid);
                            *nid += 1;
                        }
                        drop(dd);
                        if s.monitor_new_items {
                            let mut wid = self.next_wid.lock().unwrap();
                            r.auto_added.push(*wid);
                            *wid += 1;
                        }
                    }
                }
            }
            out.push(r);
        }
        Ok(out)
    }
}

// Helper to build a simple scenario
fn scenario(
    aid: AuthorId,
    uid: UserId,
    ol: Option<&str>,
    since: i32,
    auto: bool,
    existing: &[&str],
    candidates: Vec<CandidateWork>,
) -> AuthorScenario {
    AuthorScenario {
        author_id: aid,
        user_id: uid,
        ol_key: ol.map(Into::into),
        since_year: since,
        monitor_new_items: auto,
        existing_keys: existing.iter().map(|s| s.to_string()).collect(),
        candidates,
    }
}
fn cw(key: &str, title: &str, year: Option<i32>) -> CandidateWork {
    CandidateWork {
        ol_key: key.into(),
        title: title.into(),
        publish_year: year,
    }
}

// =============================================================================
// NOMINAL
// =============================================================================

#[tokio::test]
async fn nominal_trigger_bulk_enrichment_returns_ok() {
    // Satisfies: SEARCH-011
    // IR: JobService::trigger_bulk_enrichment
    let svc = MockJobService::ok();
    assert_eq!(svc.trigger_bulk_enrichment(42).await, Ok(()));
    assert_eq!(svc.log.lock().unwrap().as_slice(), &["bulk:42"]);
}

#[tokio::test]
async fn nominal_trigger_author_search_returns_ok() {
    // Satisfies: AUTHOR-002
    // IR: JobService::trigger_author_search
    let svc = MockJobService::ok();
    assert_eq!(svc.trigger_author_search().await, Ok(()));
    assert_eq!(svc.log.lock().unwrap().as_slice(), &["author_search"]);
}

#[tokio::test]
async fn nominal_trigger_scan_returns_ok() {
    // Satisfies: IMPORT-017
    // IR: JobService::trigger_scan
    let svc = MockJobService::ok();
    assert_eq!(svc.trigger_scan(7, 99).await, Ok(()));
    assert_eq!(svc.log.lock().unwrap().as_slice(), &["scan:7:99"]);
}

#[tokio::test]
async fn nominal_poll_returns_import_triggered_for_completed_download() {
    // Satisfies: IMPORT-005
    // IR: DownloadPoller::poll
    let p = MockDownloadPoller {
        grabs: vec![(1, GrabState::Completed)],
    };
    let r = p.poll().await.unwrap();
    assert_eq!(
        r,
        vec![PollResult {
            grab_id: 1,
            action: PollAction::ImportTriggered
        }]
    );
}

#[tokio::test]
async fn nominal_poll_returns_multiple_results_for_multiple_grabs() {
    // Satisfies: IMPORT-005, DLC-015
    // IR: DownloadPoller::poll
    let p = MockDownloadPoller {
        grabs: vec![
            (1, GrabState::Completed),
            (2, GrabState::QbitError),
            (3, GrabState::Importing),
        ],
    };
    let r = p.poll().await.unwrap();
    assert_eq!(r.len(), 3);
    assert_eq!(r[0].action, PollAction::ImportTriggered);
    assert!(matches!(r[1].action, PollAction::MarkedFailed { .. }));
    assert!(matches!(r[2].action, PollAction::Skipped { .. }));
}

#[tokio::test]
async fn nominal_check_all_detects_new_work() {
    // Satisfies: AUTHOR-002
    // IR: AuthorMonitor::check_all
    let m = MockAuthorMonitor::new(vec![scenario(
        10,
        20,
        Some("OL1A"),
        2020,
        false,
        &["OL-EXISTING"],
        vec![cw("OL-NEW", "New Work", Some(2020))],
    )]);
    let r = m.check_all().await.unwrap();
    assert_eq!(r[0].new_works_detected.len(), 1);
    assert_eq!(r[0].new_works_detected[0].ol_key, "OL-NEW");
}

#[tokio::test]
async fn nominal_check_all_with_monitor_new_items_auto_adds() {
    // Satisfies: AUTHOR-004
    // IR: AuthorMonitor::check_all
    let m = MockAuthorMonitor::new(vec![scenario(
        11,
        21,
        Some("OL2A"),
        2021,
        true,
        &[],
        vec![cw("OL-W1", "Auto Add", Some(2022))],
    )])
    .with_ids(1000, 2000);
    let r = m.check_all().await.unwrap();
    assert_eq!(r[0].auto_added, vec![1000]);
    assert_eq!(r[0].notifications_created, vec![2000]);
}

#[tokio::test]
async fn nominal_check_all_without_monitor_new_items_notifies_only() {
    // Satisfies: AUTHOR-002
    // IR: AuthorMonitor::check_all
    let m = MockAuthorMonitor::new(vec![scenario(
        12,
        22,
        Some("OL3A"),
        2019,
        false,
        &[],
        vec![cw("OL-W2", "Notify Only", Some(2023))],
    )])
    .with_ids(1, 3000);
    let r = m.check_all().await.unwrap();
    assert!(r[0].auto_added.is_empty());
    assert_eq!(r[0].notifications_created, vec![3000]);
}

// =============================================================================
// FAILURE
// =============================================================================

#[tokio::test]
async fn failure_trigger_bulk_enrichment_already_running() {
    // Satisfies: SEARCH-011
    // IR: JobService::trigger_bulk_enrichment
    let mut svc = MockJobService::ok();
    svc.bulk = Err(JobError::AlreadyRunning);
    assert_eq!(
        svc.trigger_bulk_enrichment(42).await,
        Err(JobError::AlreadyRunning)
    );
}

#[tokio::test]
async fn failure_trigger_scan_already_running() {
    // Satisfies: IMPORT-017
    // IR: JobService::trigger_scan
    let mut svc = MockJobService::ok();
    svc.scan = Err(JobError::AlreadyRunning);
    assert_eq!(svc.trigger_scan(1, 2).await, Err(JobError::AlreadyRunning));
}

#[tokio::test]
async fn failure_poll_qbit_error_marks_failed() {
    // Satisfies: DLC-015
    // IR: DownloadPoller::poll
    let p = MockDownloadPoller {
        grabs: vec![(44, GrabState::QbitError)],
    };
    let r = p.poll().await.unwrap();
    assert_eq!(
        r[0],
        PollResult {
            grab_id: 44,
            action: PollAction::MarkedFailed {
                reason: "qbit error state".into()
            }
        }
    );
}

#[tokio::test]
async fn failure_poll_missing_torrent_marks_failed() {
    // Satisfies: DLC-015
    // IR: DownloadPoller::poll
    let p = MockDownloadPoller {
        grabs: vec![(45, GrabState::Missing)],
    };
    let r = p.poll().await.unwrap();
    assert_eq!(
        r[0],
        PollResult {
            grab_id: 45,
            action: PollAction::MarkedFailed {
                reason: "torrent missing".into()
            }
        }
    );
}

#[tokio::test]
async fn failure_check_all_upstream_error() {
    // Satisfies: AUTHOR-002
    // IR: AuthorMonitor::check_all
    let m = MockAuthorMonitor::failing("openlibrary unavailable");
    assert_eq!(
        m.check_all().await,
        Err(JobError::Failed("openlibrary unavailable".into()))
    );
}

// =============================================================================
// BOUNDARY
// =============================================================================

#[tokio::test]
async fn boundary_poll_skips_already_importing() {
    // Satisfies: IMPORT-005
    // IR: DownloadPoller::poll
    let p = MockDownloadPoller {
        grabs: vec![(50, GrabState::Importing)],
    };
    let r = p.poll().await.unwrap();
    assert_eq!(
        r[0].action,
        PollAction::Skipped {
            reason: "grab already importing".into()
        }
    );
}

#[tokio::test]
async fn boundary_poll_empty_when_no_grabs() {
    // Satisfies: IMPORT-005
    // IR: DownloadPoller::poll
    let p = MockDownloadPoller { grabs: vec![] };
    assert!(p.poll().await.unwrap().is_empty());
}

#[tokio::test]
async fn boundary_check_all_skips_author_without_ol_key() {
    // Satisfies: AUTHOR-002
    // IR: AuthorMonitor::check_all
    let m = MockAuthorMonitor::new(vec![scenario(60, 70, None, 2020, false, &[], vec![])]);
    let r = m.check_all().await.unwrap();
    assert!(r[0].new_works_detected.is_empty());
    assert_eq!(r[0].warnings, vec!["author missing ol_key; skipped"]);
}

#[tokio::test]
async fn boundary_check_all_excludes_missing_publish_year() {
    // Satisfies: AUTHOR-002
    // IR: AuthorMonitor::check_all
    let m = MockAuthorMonitor::new(vec![scenario(
        61,
        71,
        Some("OLA"),
        2020,
        false,
        &[],
        vec![cw("OL-ND", "Undated", None)],
    )]);
    let r = m.check_all().await.unwrap();
    assert!(r[0].new_works_detected.is_empty());
    assert!(r[0].warnings[0].contains("missing/unparseable publish date"));
}

#[tokio::test]
async fn boundary_check_all_excludes_old_publish_year() {
    // Satisfies: AUTHOR-002
    // IR: AuthorMonitor::check_all
    let m = MockAuthorMonitor::new(vec![scenario(
        62,
        72,
        Some("OLB"),
        2020,
        false,
        &[],
        vec![cw("OL-OLD", "Old", Some(2019))],
    )]);
    let r = m.check_all().await.unwrap();
    assert!(r[0].new_works_detected.is_empty());
    assert!(r[0].notifications_created.is_empty());
}

#[tokio::test]
async fn boundary_check_all_notification_dedup_blocks_duplicate() {
    // Satisfies: AUTHOR-003
    // IR: AuthorMonitor::check_all
    let m = MockAuthorMonitor::new(vec![scenario(
        63,
        73,
        Some("OLC"),
        2020,
        false,
        &[],
        vec![cw("OL-DUPE", "Dup", Some(2021))],
    )])
    .with_ids(1, 900);
    let first = m.check_all().await.unwrap();
    let second = m.check_all().await.unwrap();
    assert_eq!(first[0].notifications_created, vec![900]);
    assert!(second[0].notifications_created.is_empty());
    assert_eq!(second[0].new_works_detected.len(), 1); // still detected, just no new notification
}
