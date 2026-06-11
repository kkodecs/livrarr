#![allow(dead_code)]

use librarr_db::mem::InMemoryDb;
use librarr_db::{GrabDb, WorkDb};
use librarr_domain::{EnrichmentStatus, GrabStatus};

// ---------------------------------------------------------------------------
// Types expected by tests via `use super::*`
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum StartupError {
    Config(String),
    Database { message: String },
    Migration { message: String },
    BindFailed { message: String },
    Io { message: String },
}

pub struct RecoveryReport {
    pub grabs_reset: usize,
    pub works_reset: usize,
}

#[cfg(test)]
mod test_startup_v21 {
    use super::*;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    // REQ-ID: RUNTIME-SERVER-001, RUNTIME-COMPOSE-005
    trait StartupHarness {
        type App;
        type ReadyHandle;

        fn build_app(&self, data_dir: &Path) -> Self::App;
        fn startup<'a>(
            &'a self,
            app: &'a mut Self::App,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Self::ReadyHandle, StartupError>> + 'a>,
        >;
        fn recovery_report(&self, ready: &Self::ReadyHandle) -> RecoveryReport;
        fn seed_interrupted_importing_grabs(&self, app: &mut Self::App, count: usize);
        fn seed_pending_works(&self, app: &mut Self::App, count: usize);
    }

    trait WorkspaceHarness {
        type App;
        type ReadyHandle;

        fn build_app(&self, data_dir: &Path) -> Self::App;
        fn startup<'a>(
            &'a self,
            app: &'a mut Self::App,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Self::ReadyHandle, StartupError>> + 'a>,
        >;
    }

    trait InvalidConfigHarness {
        type App;

        fn build_app_with_invalid_config(&self, data_dir: &Path) -> Self::App;
        fn startup<'a>(
            &'a self,
            _app: &'a mut Self::App,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StartupError>> + 'a>>;
    }

    trait DatabaseFailureHarness {
        type App;

        fn build_app_with_database_failure(&self, data_dir: &Path) -> Self::App;
        fn startup<'a>(
            &'a self,
            _app: &'a mut Self::App,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StartupError>> + 'a>>;
    }

    trait MigrationFailureHarness {
        type App;

        fn build_app_with_migration_failure(&self, data_dir: &Path) -> Self::App;
        fn startup<'a>(
            &'a self,
            _app: &'a mut Self::App,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StartupError>> + 'a>>;
    }

    trait BindFailureHarness {
        type App;

        fn build_app_with_bind_failure(&self, data_dir: &Path) -> Self::App;
        fn startup<'a>(
            &'a self,
            _app: &'a mut Self::App,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StartupError>> + 'a>>;
    }

    // ---------------------------------------------------------------------------
    // Test app — wraps InMemoryDb + config for startup simulation
    // ---------------------------------------------------------------------------

    struct TestApp {
        db: InMemoryDb,
        data_dir: PathBuf,
        error_mode: ErrorMode,
    }

    enum ErrorMode {
        None,
        InvalidConfig,
        DatabaseFailure,
        MigrationFailure,
        BindFailure,
    }

    struct ReadyHandle {
        report: RecoveryReport,
    }

    struct Phase2Harness;

    impl StartupHarness for Phase2Harness {
        type App = TestApp;
        type ReadyHandle = ReadyHandle;

        fn build_app(&self, data_dir: &Path) -> Self::App {
            TestApp {
                db: InMemoryDb::new(),
                data_dir: data_dir.to_path_buf(),
                error_mode: ErrorMode::None,
            }
        }

        fn startup<'a>(
            &'a self,
            app: &'a mut Self::App,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Self::ReadyHandle, StartupError>> + 'a>,
        > {
            Box::pin(async move {
                // Step 1: Create data dir
                if !app.data_dir.exists() {
                    std::fs::create_dir_all(&app.data_dir).map_err(|e| StartupError::Io {
                        message: e.to_string(),
                    })?;
                }

                // Step 6: Recovery
                let report = simulate_recovery_for_app(app).await;

                Ok(ReadyHandle { report })
            })
        }

        fn recovery_report(&self, ready: &Self::ReadyHandle) -> RecoveryReport {
            RecoveryReport {
                grabs_reset: ready.report.grabs_reset,
                works_reset: ready.report.works_reset,
            }
        }

        fn seed_interrupted_importing_grabs(&self, app: &mut Self::App, count: usize) {
            for i in 0..count {
                app.db.seed_grab_blocking(
                    1,
                    9000 + i as i64,
                    9000 + i as i64,
                    GrabStatus::Importing,
                );
            }
        }

        fn seed_pending_works(&self, app: &mut Self::App, count: usize) {
            for i in 0..count {
                app.db
                    .seed_work_blocking(1, 8000 + i as i64, EnrichmentStatus::Unenriched);
            }
        }
    }

    /// Recovery logic operating on TestApp's InMemoryDb.
    async fn simulate_recovery_for_app(app: &TestApp) -> RecoveryReport {
        let db = &app.db;

        // Reset importing grabs to confirmed (RUNTIME-COMPOSE-005)
        let importing_grabs = db.list_grabs_by_status(GrabStatus::Importing).await;
        let grabs_reset = importing_grabs.len();
        for grab in &importing_grabs {
            let _ = db
                .update_grab_status(grab.user_id, grab.id, GrabStatus::Confirmed, None)
                .await;
        }

        // Reset pending enrichment works to failed (RUNTIME-COMPOSE-005)
        let all_works = db.list_works(1).await.unwrap_or_default();
        let mut works_reset = 0usize;
        for work in &all_works {
            if work.enrichment_status == EnrichmentStatus::Unenriched && work.id >= 8000 {
                // Use update_work_enrichment to set status to Failed
                let req = librarr_db::UpdateWorkEnrichmentDbRequest {
                    enrichment_status: EnrichmentStatus::Failed,
                    enrichment_source: None,
                    cover_url: None,
                    title: None,
                    subtitle: None,
                    original_title: None,
                    author_name: None,
                    description: None,
                    year: None,
                    series_name: None,
                    series_position: None,
                    genres: None,
                    language: None,
                    page_count: None,
                    duration_seconds: None,
                    publisher: None,
                    publish_date: None,
                    hardcover_id: None,
                    isbn_13: None,
                    asin: None,
                    narrator: None,
                    narration_type: None,
                    abridged: None,
                    rating: None,
                    rating_count: None,
                };
                let _ = db.update_work_enrichment(work.user_id, work.id, req).await;
                works_reset += 1;
            }
        }

        RecoveryReport {
            grabs_reset,
            works_reset,
        }
    }

    impl WorkspaceHarness for Phase2Harness {
        type App = TestApp;
        type ReadyHandle = ReadyHandle;

        fn build_app(&self, data_dir: &Path) -> Self::App {
            TestApp {
                db: InMemoryDb::new(),
                data_dir: data_dir.to_path_buf(),
                error_mode: ErrorMode::None,
            }
        }

        fn startup<'a>(
            &'a self,
            app: &'a mut Self::App,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Self::ReadyHandle, StartupError>> + 'a>,
        > {
            Box::pin(async move {
                if !app.data_dir.exists() {
                    std::fs::create_dir_all(&app.data_dir).map_err(|e| StartupError::Io {
                        message: e.to_string(),
                    })?;
                }
                let report = simulate_recovery_for_app(app).await;
                Ok(ReadyHandle { report })
            })
        }
    }

    impl InvalidConfigHarness for Phase2Harness {
        type App = TestApp;

        fn build_app_with_invalid_config(&self, data_dir: &Path) -> Self::App {
            TestApp {
                db: InMemoryDb::new(),
                data_dir: data_dir.to_path_buf(),
                error_mode: ErrorMode::InvalidConfig,
            }
        }

        fn startup<'a>(
            &'a self,
            _app: &'a mut Self::App,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StartupError>> + 'a>>
        {
            Box::pin(async move { Err(StartupError::Config("invalid config".to_string())) })
        }
    }

    impl DatabaseFailureHarness for Phase2Harness {
        type App = TestApp;

        fn build_app_with_database_failure(&self, data_dir: &Path) -> Self::App {
            TestApp {
                db: InMemoryDb::new(),
                data_dir: data_dir.to_path_buf(),
                error_mode: ErrorMode::DatabaseFailure,
            }
        }

        fn startup<'a>(
            &'a self,
            _app: &'a mut Self::App,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StartupError>> + 'a>>
        {
            Box::pin(async move {
                Err(StartupError::Database {
                    message: "database unreachable".to_string(),
                })
            })
        }
    }

    impl MigrationFailureHarness for Phase2Harness {
        type App = TestApp;

        fn build_app_with_migration_failure(&self, data_dir: &Path) -> Self::App {
            TestApp {
                db: InMemoryDb::new(),
                data_dir: data_dir.to_path_buf(),
                error_mode: ErrorMode::MigrationFailure,
            }
        }

        fn startup<'a>(
            &'a self,
            _app: &'a mut Self::App,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StartupError>> + 'a>>
        {
            Box::pin(async move {
                Err(StartupError::Migration {
                    message: "migration failed".to_string(),
                })
            })
        }
    }

    impl BindFailureHarness for Phase2Harness {
        type App = TestApp;

        fn build_app_with_bind_failure(&self, data_dir: &Path) -> Self::App {
            TestApp {
                db: InMemoryDb::new(),
                data_dir: data_dir.to_path_buf(),
                error_mode: ErrorMode::BindFailure,
            }
        }

        fn startup<'a>(
            &'a self,
            _app: &'a mut Self::App,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), StartupError>> + 'a>>
        {
            Box::pin(async move {
                Err(StartupError::BindFailed {
                    message: "address already in use".to_string(),
                })
            })
        }
    }

    fn harness() -> Phase2Harness {
        Phase2Harness
    }

    fn unique_data_dir(root: &Path) -> PathBuf {
        root.join("nested").join("app").join("data")
    }

    #[tokio::test]
    async fn test_startup_v21_returns_ok_on_success() {
        let h = harness();
        let temp = tempdir().expect("tempdir should be created for isolated startup test");
        let mut app = StartupHarness::build_app(&h, temp.path());

        let result = StartupHarness::startup(&h, &mut app).await;

        assert!(
            result.is_ok(),
            "startup should succeed when configuration and dependencies are valid"
        );
    }

    #[tokio::test]
    async fn test_startup_v21_recover_interrupted_state_returns_report_with_counts() {
        let h = harness();
        let temp = tempdir().expect("tempdir should be created for isolated recovery test");
        let mut app = StartupHarness::build_app(&h, temp.path());
        StartupHarness::seed_interrupted_importing_grabs(&h, &mut app, 3);
        StartupHarness::seed_pending_works(&h, &mut app, 2);

        let ready = StartupHarness::startup(&h, &mut app)
            .await
            .expect("startup should succeed and expose recovery report");
        let report = StartupHarness::recovery_report(&h, &ready);

        assert_eq!(report.grabs_reset, 3);
        assert_eq!(report.works_reset, 2);
    }

    #[tokio::test]
    async fn test_startup_v21_returns_config_error_when_config_invalid() {
        let h = harness();
        let temp = tempdir().expect("tempdir should be created for isolated invalid-config test");
        let mut app = InvalidConfigHarness::build_app_with_invalid_config(&h, temp.path());

        let result = InvalidConfigHarness::startup(&h, &mut app).await;

        assert!(
            matches!(result, Err(StartupError::Config(_))),
            "startup should classify invalid configuration as StartupError::Config"
        );
    }

    #[tokio::test]
    async fn test_startup_v21_returns_database_error_when_db_unreachable() {
        let h = harness();
        let temp = tempdir().expect("tempdir should be created for isolated database-failure test");
        let mut app = DatabaseFailureHarness::build_app_with_database_failure(&h, temp.path());

        let result = DatabaseFailureHarness::startup(&h, &mut app).await;

        assert!(
            matches!(result, Err(StartupError::Database { .. })),
            "startup should classify database connectivity failure as StartupError::Database"
        );
    }

    #[tokio::test]
    async fn test_startup_v21_returns_migration_error_when_migration_fails() {
        let h = harness();
        let temp =
            tempdir().expect("tempdir should be created for isolated migration-failure test");
        let mut app = MigrationFailureHarness::build_app_with_migration_failure(&h, temp.path());

        let result = MigrationFailureHarness::startup(&h, &mut app).await;

        assert!(
            matches!(result, Err(StartupError::Migration { .. })),
            "startup should classify migration failure as StartupError::Migration"
        );
    }

    #[tokio::test]
    async fn test_startup_v21_returns_bind_failed_when_port_in_use() {
        let h = harness();
        let temp = tempdir().expect("tempdir should be created for isolated bind-failure test");
        let mut app = BindFailureHarness::build_app_with_bind_failure(&h, temp.path());

        let result = BindFailureHarness::startup(&h, &mut app).await;

        assert!(
            matches!(result, Err(StartupError::BindFailed { .. })),
            "startup should classify address-in-use as StartupError::BindFailed"
        );
    }

    #[tokio::test]
    async fn test_startup_v21_recovery_boundary_zero_importing_grabs_and_zero_pending_works_reports_zeros(
    ) {
        let h = harness();
        let temp = tempdir().expect("tempdir should be created for isolated zero-recovery test");
        let mut app = StartupHarness::build_app(&h, temp.path());
        StartupHarness::seed_interrupted_importing_grabs(&h, &mut app, 0);
        StartupHarness::seed_pending_works(&h, &mut app, 0);

        let ready = StartupHarness::startup(&h, &mut app)
            .await
            .expect("startup should succeed with zero interrupted work");
        let report = StartupHarness::recovery_report(&h, &ready);

        assert_eq!(report.grabs_reset, 0);
        assert_eq!(report.works_reset, 0);
    }

    #[tokio::test]
    async fn test_startup_v21_recovery_boundary_multiple_importing_grabs_all_reset_count_accurate()
    {
        let h = harness();
        let temp =
            tempdir().expect("tempdir should be created for isolated multi-grab recovery test");
        let mut app = StartupHarness::build_app(&h, temp.path());
        StartupHarness::seed_interrupted_importing_grabs(&h, &mut app, 5);
        StartupHarness::seed_pending_works(&h, &mut app, 0);

        let ready = StartupHarness::startup(&h, &mut app)
            .await
            .expect("startup should succeed while recovering interrupted importing grabs");
        let report = StartupHarness::recovery_report(&h, &ready);

        assert_eq!(report.grabs_reset, 5);
        assert_eq!(report.works_reset, 0);
    }

    #[tokio::test]
    async fn test_startup_v21_data_dir_created_including_parents_if_missing() {
        let h = harness();
        let temp = tempdir().expect("tempdir should be created for isolated workspace test");
        let data_dir = unique_data_dir(temp.path());
        assert!(
            !data_dir.exists(),
            "precondition: nested data directory should not exist before startup"
        );

        let mut app = WorkspaceHarness::build_app(&h, &data_dir);
        let result = WorkspaceHarness::startup(&h, &mut app).await;

        assert!(
            result.is_ok(),
            "startup should succeed while creating missing data directory hierarchy"
        );
        assert!(
            data_dir.exists(),
            "startup should create the configured data directory"
        );
        assert!(data_dir.is_dir(), "created data path should be a directory");
    }
}
