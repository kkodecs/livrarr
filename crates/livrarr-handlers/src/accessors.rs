use livrarr_domain::settings::MetadataConfig;

/// Tracing log surface — the daily rolling file actually written and any
/// init failure captured at startup (REQ-003). The dated path is computed at
/// read time so the answer stays truthful across midnight rollover.
pub trait LogSurfaceAccessor: Send + Sync {
    fn status(&self) -> livrarr_domain::LogSurfaceStatus;
}

/// Live metadata config — hot-swappable config for enrichment components.
pub trait LiveMetadataConfigAccessor: Send + Sync {
    fn replace(&self, cfg: MetadataConfig);
}

/// RSS sync atomic guards — prevent concurrent RSS syncs.
pub trait RssSyncAccessor: Send + Sync {
    /// CAS false→true. Returns true if acquired.
    fn try_acquire(&self) -> bool;
    /// Set false (release the guard).
    fn release(&self);
    /// Store last-run timestamp (unix seconds).
    fn set_last_run(&self, ts: i64);
    /// Read the running flag.
    fn is_running(&self) -> bool;
    /// Read last-run timestamp (unix seconds, 0 = never).
    fn last_run_at(&self) -> i64;
}

/// System observability — log buffer + log level control.
pub trait SystemAccessor: Send + Sync {
    fn log_tail(&self, n: usize) -> Vec<String>;
    fn current_log_level(&self) -> String;
    fn set_log_level(&self, level: &str) -> Result<(), String>;
}

/// Manual import scan state — in-memory progressive scan results.
pub trait ManualImportScanAccessor: Send + Sync {
    fn insert_scan(
        &self,
        scan_id: String,
        user_id: i64,
        files: Vec<crate::manual_import::ScannedFile>,
        warnings: Vec<String>,
        ol_total: usize,
    );
    fn get_scan(&self, scan_id: &str) -> Option<crate::manual_import::ScanSnapshot>;
    fn update_scan_file(
        &self,
        scan_id: &str,
        file_idx: usize,
        update: crate::manual_import::ScanFileUpdate,
    );
    fn increment_ol_completed(&self, scan_id: &str);
    fn remove_scan(&self, scan_id: &str);
}

/// Cover proxy cache — get/put for proxied cover images.
pub trait CoverProxyCacheAccessor: Send + Sync {
    fn get(&self, url: &str)
        -> impl std::future::Future<Output = Option<(Vec<u8>, String)>> + Send;
    fn put(
        &self,
        url: String,
        data: Vec<u8>,
        content_type: String,
    ) -> impl std::future::Future<Output = ()> + Send;
}

/// Trusted origins rebuilder -- rebuilds the SSRF trusted-origins
/// allowlist from current indexer + download client URLs.
pub trait TrustedOriginsRebuilder: Send + Sync {
    fn rebuild(&self, urls: &[String]);
}
