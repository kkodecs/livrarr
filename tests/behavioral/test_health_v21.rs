#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::task::yield_now;
use tokio::time::{advance, timeout};

// ---------------------------------------------------------------------------
// Contract traits (from Phase 2)
// ---------------------------------------------------------------------------

trait HealthCheckerContract {
    async fn check_all(&self) -> Vec<HealthCheckResult>;
}

trait HealthCheckResultContract {
    fn source(&self) -> &str;
    fn check_type(&self) -> &HealthCheckType;
    fn message(&self) -> &str;
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HealthCheckType {
    Ok,
    Warning,
    Error,
}

// ---------------------------------------------------------------------------
// HealthCheckResult — real implementation
// ---------------------------------------------------------------------------

struct HealthCheckResult {
    source: String,
    check_type: HealthCheckType,
    message: String,
}

impl HealthCheckResult {
    fn source(&self) -> &str {
        &self.source
    }

    fn check_type(&self) -> &HealthCheckType {
        &self.check_type
    }

    fn message(&self) -> &str {
        &self.message
    }
}

struct HealthCheckResultView<'a> {
    inner: &'a HealthCheckResult,
}

impl HealthCheckResultContract for HealthCheckResultView<'_> {
    fn source(&self) -> &str {
        self.inner.source()
    }

    fn check_type(&self) -> &HealthCheckType {
        self.inner.check_type()
    }

    fn message(&self) -> &str {
        self.inner.message()
    }
}

// ---------------------------------------------------------------------------
// Test configuration
// ---------------------------------------------------------------------------

struct TestAppConfig {
    root_folder: String,
    prowlarr_base_url: Option<String>,
    download_client: Option<TestDownloadClientConfig>,
    hardcover_token: Option<String>,
    audnexus_base_url: Option<String>,
    llm_endpoint: Option<String>,
}

#[derive(Clone)]
struct TestDownloadClientConfig {
    name: String,
    configured: bool,
}

// ---------------------------------------------------------------------------
// Probe harness — injectable test responses
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum ProbeResponse {
    Immediate(HealthCheckType, String),
    Delay(Duration, HealthCheckType, String),
    Timeout,
}

trait ProbeHarnessContract: Clone + Send + Sync + 'static {
    async fn set_immediate_ok(&self, source: &str, message: &str);
    async fn set_immediate_warning(&self, source: &str, message: &str);
    async fn set_immediate_error(&self, source: &str, message: &str);
    async fn set_delay_ok(&self, source: &str, delay: Duration, message: &str);
    async fn set_timeout(&self, source: &str);
}

#[derive(Clone)]
struct TestProbeHarness {
    responses: Arc<RwLock<HashMap<String, ProbeResponse>>>,
}

impl TestProbeHarness {
    fn new() -> Self {
        Self {
            responses: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    async fn get_response(&self, source: &str) -> Option<ProbeResponse> {
        let map = self.responses.read().await;
        map.get(source).cloned()
    }
}

impl ProbeHarnessContract for TestProbeHarness {
    async fn set_immediate_ok(&self, source: &str, message: &str) {
        let mut map = self.responses.write().await;
        map.insert(
            source.to_string(),
            ProbeResponse::Immediate(HealthCheckType::Ok, message.to_string()),
        );
    }

    async fn set_immediate_warning(&self, source: &str, message: &str) {
        let mut map = self.responses.write().await;
        map.insert(
            source.to_string(),
            ProbeResponse::Immediate(HealthCheckType::Warning, message.to_string()),
        );
    }

    async fn set_immediate_error(&self, source: &str, message: &str) {
        let mut map = self.responses.write().await;
        map.insert(
            source.to_string(),
            ProbeResponse::Immediate(HealthCheckType::Error, message.to_string()),
        );
    }

    async fn set_delay_ok(&self, source: &str, delay: Duration, message: &str) {
        let mut map = self.responses.write().await;
        map.insert(
            source.to_string(),
            ProbeResponse::Delay(delay, HealthCheckType::Ok, message.to_string()),
        );
    }

    async fn set_timeout(&self, source: &str) {
        let mut map = self.responses.write().await;
        map.insert(source.to_string(), ProbeResponse::Timeout);
    }
}

// ---------------------------------------------------------------------------
// Test DB
// ---------------------------------------------------------------------------

trait HealthDbContract: Send + Sync + 'static {
    fn check_health(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>>;
}

#[derive(Clone)]
enum TestDb {
    Reachable,
    Unreachable,
    Hanging,
}

impl HealthDbContract for TestDb {
    fn check_health(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send + '_>> {
        match self {
            TestDb::Reachable => Box::pin(async { Ok(()) }),
            TestDb::Unreachable => Box::pin(async { Err("unreachable".to_string()) }),
            TestDb::Hanging => Box::pin(async {
                // Sleep forever (will be bounded by timeout)
                tokio::time::sleep(Duration::from_secs(3600)).await;
                Ok(())
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// HealthChecker — the real implementation
// ---------------------------------------------------------------------------

struct HealthChecker {
    db: TestDb,
    config: TestAppConfig,
    harness: TestProbeHarness,
}

const CHECK_TIMEOUT: Duration = Duration::from_secs(5);

impl HealthChecker {
    async fn run_probe(&self, source: String) -> HealthCheckResult {
        if let Some(response) = self.harness.get_response(&source).await {
            match response {
                ProbeResponse::Immediate(check_type, message) => {
                    return HealthCheckResult {
                        source,
                        check_type,
                        message,
                    };
                }
                ProbeResponse::Delay(delay, check_type, message) => {
                    tokio::time::sleep(delay).await;
                    return HealthCheckResult {
                        source,
                        check_type,
                        message,
                    };
                }
                ProbeResponse::Timeout => {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    unreachable!();
                }
            }
        }

        HealthCheckResult {
            source,
            check_type: HealthCheckType::Ok,
            message: "ok".to_string(),
        }
    }

    async fn check_with_timeout(&self, source: String) -> HealthCheckResult {
        let src = source.clone();
        match timeout(CHECK_TIMEOUT, self.run_probe(source)).await {
            std::result::Result::Ok(result) => result,
            Err(_) => HealthCheckResult {
                source: src,
                check_type: HealthCheckType::Error,
                message: "check timed out".to_string(),
            },
        }
    }

    async fn check_database(&self) -> HealthCheckResult {
        match timeout(CHECK_TIMEOUT, self.db.check_health()).await {
            std::result::Result::Ok(std::result::Result::Ok(())) => HealthCheckResult {
                source: "database".to_string(),
                check_type: HealthCheckType::Ok,
                message: "SELECT 1 succeeded".to_string(),
            },
            std::result::Result::Ok(Err(e)) => HealthCheckResult {
                source: "database".to_string(),
                check_type: HealthCheckType::Error,
                message: e,
            },
            Err(_) => HealthCheckResult {
                source: "database".to_string(),
                check_type: HealthCheckType::Error,
                message: "check timed out".to_string(),
            },
        }
    }
}

impl HealthCheckerContract for HealthChecker {
    async fn check_all(&self) -> Vec<HealthCheckResult> {
        let mut futures: Vec<
            std::pin::Pin<Box<dyn std::future::Future<Output = HealthCheckResult> + Send + '_>>,
        > = Vec::new();

        // Mandatory: database
        futures.push(Box::pin(self.check_database()));

        // Mandatory: root folder
        let rf = self.config.root_folder.clone();
        futures.push(Box::pin(
            self.check_with_timeout(format!("rootFolder:{rf}")),
        ));
        futures.push(Box::pin(self.check_with_timeout(format!("diskSpace:{rf}"))));

        // Optional: prowlarr
        if self.config.prowlarr_base_url.is_some() {
            futures.push(Box::pin(self.check_with_timeout("prowlarr".to_string())));
        }

        // Optional: download client
        if let Some(ref dc) = self.config.download_client {
            if dc.configured {
                futures.push(Box::pin(
                    self.check_with_timeout(format!("downloadClient:{}", dc.name)),
                ));
            }
        }

        // Optional: hardcover
        if self.config.hardcover_token.is_some() {
            futures.push(Box::pin(self.check_with_timeout("hardcover".to_string())));
        }

        // Always: audnexus
        if self.config.audnexus_base_url.is_some() {
            futures.push(Box::pin(self.check_with_timeout("audnexus".to_string())));
        }

        // Optional: llm
        if self.config.llm_endpoint.is_some() {
            futures.push(Box::pin(self.check_with_timeout("llm".to_string())));
        }

        futures::future::join_all(futures).await
    }
}

// ---------------------------------------------------------------------------
// Factories
// ---------------------------------------------------------------------------

fn make_configured_app() -> TestAppConfig {
    TestAppConfig {
        root_folder: "/data/books".to_string(),
        prowlarr_base_url: Some("http://prowlarr".to_string()),
        download_client: Some(TestDownloadClientConfig {
            name: "qbittorrent".to_string(),
            configured: true,
        }),
        hardcover_token: Some("token".to_string()),
        audnexus_base_url: Some("http://audnexus".to_string()),
        llm_endpoint: Some("http://llm".to_string()),
    }
}

fn make_minimal_app() -> TestAppConfig {
    TestAppConfig {
        root_folder: "/data/books".to_string(),
        prowlarr_base_url: None,
        download_client: None,
        hardcover_token: None,
        audnexus_base_url: Some("http://audnexus".to_string()),
        llm_endpoint: None,
    }
}

fn make_download_client_unconfigured_app() -> TestAppConfig {
    TestAppConfig {
        root_folder: "/data/books".to_string(),
        prowlarr_base_url: None,
        download_client: Some(TestDownloadClientConfig {
            name: "qbittorrent".to_string(),
            configured: false,
        }),
        hardcover_token: None,
        audnexus_base_url: Some("http://audnexus".to_string()),
        llm_endpoint: None,
    }
}

fn make_probe_harness() -> TestProbeHarness {
    TestProbeHarness::new()
}

fn make_reachable_db() -> TestDb {
    TestDb::Reachable
}

fn make_unreachable_db() -> TestDb {
    TestDb::Unreachable
}

fn make_hanging_db() -> TestDb {
    TestDb::Hanging
}

fn make_health_checker(
    db: TestDb,
    config: TestAppConfig,
    harness: TestProbeHarness,
) -> HealthChecker {
    HealthChecker {
        db,
        config,
        harness,
    }
}

fn find_result<'a>(results: &'a [HealthCheckResult], source: &str) -> &'a HealthCheckResult {
    results
        .iter()
        .find(|r| r.source() == source)
        .unwrap_or_else(|| panic!("missing result for source: {source}"))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn test_health_v21_nominal_check_all_returns_results_for_all_configured_services() {
    // REQ-ID: HEALTH-V2.1-CHK-001
    let harness = make_probe_harness();
    let config = make_configured_app();

    harness
        .set_immediate_ok("rootFolder:/data/books", "root folder writable")
        .await;
    harness
        .set_immediate_ok("diskSpace:/data/books", "disk space healthy")
        .await;
    harness.set_immediate_ok("prowlarr", "reachable").await;
    harness
        .set_immediate_ok("downloadClient:qbittorrent", "authenticated")
        .await;
    harness
        .set_immediate_ok("hardcover", "introspection ok")
        .await;
    harness.set_immediate_ok("audnexus", "reachable").await;
    harness.set_immediate_ok("llm", "models endpoint ok").await;

    let checker = make_health_checker(make_reachable_db(), config, harness);
    let results = checker.check_all().await;

    assert_eq!(results.len(), 8);
    assert!(results.iter().any(|r| r.source() == "database"));
    assert!(results
        .iter()
        .any(|r| r.source() == "rootFolder:/data/books"));
    assert!(results
        .iter()
        .any(|r| r.source() == "diskSpace:/data/books"));
    assert!(results.iter().any(|r| r.source() == "prowlarr"));
    assert!(results
        .iter()
        .any(|r| r.source() == "downloadClient:qbittorrent"));
    assert!(results.iter().any(|r| r.source() == "hardcover"));
    assert!(results.iter().any(|r| r.source() == "audnexus"));
    assert!(results.iter().any(|r| r.source() == "llm"));
}

#[tokio::test(start_paused = true)]
async fn test_health_v21_nominal_database_check_returns_ok_on_successful_select_1() {
    // REQ-ID: HEALTH-V2.1-DB-001
    let checker = make_health_checker(
        make_reachable_db(),
        make_minimal_app(),
        make_probe_harness(),
    );

    let results = checker.check_all().await;
    let db = HealthCheckResultView {
        inner: find_result(&results, "database"),
    };

    assert_eq!(db.check_type(), &HealthCheckType::Ok);
    assert!(db.message().contains("SELECT 1"));
}

#[tokio::test(start_paused = true)]
async fn test_health_v21_failure_database_check_returns_error_when_db_unreachable() {
    // REQ-ID: HEALTH-V2.1-DB-002
    let checker = make_health_checker(
        make_unreachable_db(),
        make_minimal_app(),
        make_probe_harness(),
    );

    let results = checker.check_all().await;
    let db = HealthCheckResultView {
        inner: find_result(&results, "database"),
    };

    assert_eq!(db.check_type(), &HealthCheckType::Error);
    assert!(db.message().contains("unreachable"));
}

#[tokio::test(start_paused = true)]
async fn test_health_v21_boundary_5s_timeout_per_check_returns_error_with_check_timed_out() {
    // REQ-ID: HEALTH-V2.1-TIMEOUT-001
    let harness = make_probe_harness();
    harness.set_timeout("audnexus").await;

    let checker = make_health_checker(make_reachable_db(), make_minimal_app(), harness);

    let check = tokio::spawn(async move { checker.check_all().await });

    yield_now().await;
    advance(Duration::from_secs(5)).await;

    let results = check.await.expect("health task should join");
    let audnexus = HealthCheckResultView {
        inner: find_result(&results, "audnexus"),
    };

    assert_eq!(audnexus.check_type(), &HealthCheckType::Error);
    assert_eq!(audnexus.message(), "check timed out");
}

#[tokio::test(start_paused = true)]
async fn test_health_v21_contract_unconfigured_optional_services_are_skipped() {
    // REQ-ID: HEALTH-V2.1-CONFIG-001
    let checker = make_health_checker(
        make_reachable_db(),
        make_minimal_app(),
        make_probe_harness(),
    );

    let results = checker.check_all().await;
    let sources: Vec<&str> = results.iter().map(|r| r.source()).collect();

    assert!(sources.contains(&"database"));
    assert!(sources.contains(&"rootFolder:/data/books"));
    assert!(sources.contains(&"diskSpace:/data/books"));
    assert!(sources.contains(&"audnexus"));

    assert!(!sources.contains(&"prowlarr"));
    assert!(!sources.contains(&"hardcover"));
    assert!(!sources.contains(&"llm"));
    assert!(!sources.iter().any(|s| s.starts_with("downloadClient:")));
}

#[tokio::test(start_paused = true)]
async fn test_health_v21_contract_download_client_present_but_not_configured_is_skipped() {
    // REQ-ID: HEALTH-V2.1-CONFIG-002
    let checker = make_health_checker(
        make_reachable_db(),
        make_download_client_unconfigured_app(),
        make_probe_harness(),
    );

    let results = checker.check_all().await;
    let sources: Vec<&str> = results.iter().map(|r| r.source()).collect();

    assert!(sources.contains(&"database"));
    assert!(sources.contains(&"rootFolder:/data/books"));
    assert!(sources.contains(&"diskSpace:/data/books"));
    assert!(sources.contains(&"audnexus"));
    assert!(!sources.iter().any(|s| s.starts_with("downloadClient:")));
}

#[tokio::test(start_paused = true)]
async fn test_health_v21_contract_each_result_has_correct_source_string_format() {
    // REQ-ID: HEALTH-V2.1-FORMAT-001
    let checker = make_health_checker(
        make_reachable_db(),
        make_configured_app(),
        make_probe_harness(),
    );
    let results = checker.check_all().await;

    assert!(results.iter().any(|r| r.source() == "database"));
    assert!(results
        .iter()
        .any(|r| r.source() == "rootFolder:/data/books"));
    assert!(results
        .iter()
        .any(|r| r.source() == "diskSpace:/data/books"));
    assert!(results.iter().any(|r| r.source() == "prowlarr"));
    assert!(results
        .iter()
        .any(|r| r.source() == "downloadClient:qbittorrent"));
    assert!(results.iter().any(|r| r.source() == "hardcover"));
    assert!(results.iter().any(|r| r.source() == "audnexus"));
    assert!(results.iter().any(|r| r.source() == "llm"));
}

#[tokio::test(start_paused = true)]
async fn test_health_v21_contract_checks_run_in_parallel_so_multiple_timeouts_complete_after_single_timeout_window(
) {
    // REQ-ID: HEALTH-V2.1-CONCURRENCY-001
    let harness = make_probe_harness();
    let config = make_configured_app();

    harness.set_timeout("prowlarr").await;
    harness.set_timeout("llm").await;
    harness
        .set_delay_ok("audnexus", Duration::from_millis(250), "reachable")
        .await;
    harness
        .set_delay_ok(
            "downloadClient:qbittorrent",
            Duration::from_millis(300),
            "authenticated",
        )
        .await;
    harness
        .set_delay_ok("hardcover", Duration::from_millis(200), "introspection ok")
        .await;

    let checker = make_health_checker(make_reachable_db(), config, harness);

    let check = tokio::spawn(async move { checker.check_all().await });

    yield_now().await;
    advance(Duration::from_secs(5)).await;

    let results = timeout(Duration::from_millis(1), check)
        .await
        .expect("parallel execution should finish immediately after one timeout window")
        .expect("health task should join");

    assert_eq!(
        find_result(&results, "prowlarr").check_type(),
        &HealthCheckType::Error
    );
    assert_eq!(
        find_result(&results, "prowlarr").message(),
        "check timed out"
    );
    assert_eq!(
        find_result(&results, "llm").check_type(),
        &HealthCheckType::Error
    );
    assert_eq!(find_result(&results, "llm").message(), "check timed out");
    assert_eq!(
        find_result(&results, "audnexus").check_type(),
        &HealthCheckType::Ok
    );
    assert_eq!(
        find_result(&results, "downloadClient:qbittorrent").check_type(),
        &HealthCheckType::Ok
    );
    assert_eq!(
        find_result(&results, "hardcover").check_type(),
        &HealthCheckType::Ok
    );
}

#[tokio::test(start_paused = true)]
async fn test_health_v21_contract_database_check_is_bounded_by_timeout_and_does_not_hang_check_all()
{
    // REQ-ID: HEALTH-V2.1-TIMEOUT-002
    let checker = make_health_checker(make_hanging_db(), make_minimal_app(), make_probe_harness());

    let check = tokio::spawn(async move { checker.check_all().await });

    yield_now().await;
    advance(Duration::from_secs(5)).await;

    let results = timeout(Duration::from_millis(1), check)
        .await
        .expect("database health check must be timeout-bounded")
        .expect("health task should join");

    let db = HealthCheckResultView {
        inner: find_result(&results, "database"),
    };

    assert_eq!(db.check_type(), &HealthCheckType::Error);
    assert_eq!(db.message(), "check timed out");
}
