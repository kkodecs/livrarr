use std::collections::HashMap;

use livrarr_domain::settings::MetadataConfig;

/// Provider health status — tracks provider errors with TTL.
pub trait ProviderHealthAccessor: Send + Sync {
    fn statuses(&self) -> impl std::future::Future<Output = HashMap<String, String>> + Send;
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
    fn acquire_ol_permit(&self) -> impl std::future::Future<Output = ()> + Send;
    fn search_ol_works(
        &self,
        term: &str,
        limit: u32,
    ) -> impl std::future::Future<Output = Result<Vec<crate::WorkSearchResult>, String>> + Send;
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
