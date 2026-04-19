use std::sync::Arc;

use livrarr_handlers::accessors::ManualImportScanAccessor;
use livrarr_handlers::manual_import::{ScanFileUpdate, ScanSnapshot, ScannedFile};
use livrarr_handlers::WorkSearchResult;

use crate::state::{ManualImportScanMap, ManualImportScanState, OlRateLimiter};

#[derive(Clone)]
pub struct LiveManualImportScanService {
    pub scans: Arc<ManualImportScanMap>,
    pub ol_rate_limiter: Arc<OlRateLimiter>,
    pub http_client: livrarr_http::HttpClient,
}

impl ManualImportScanAccessor for LiveManualImportScanService {
    fn insert_scan(
        &self,
        scan_id: String,
        user_id: i64,
        files: Vec<ScannedFile>,
        warnings: Vec<String>,
        ol_total: usize,
    ) {
        self.scans.insert(
            scan_id,
            ManualImportScanState {
                files: std::sync::RwLock::new(files),
                warnings,
                ol_total,
                ol_completed: std::sync::atomic::AtomicUsize::new(0),
                user_id,
                created_at: std::time::Instant::now(),
            },
        );
    }

    fn get_scan(&self, scan_id: &str) -> Option<ScanSnapshot> {
        let entry = self.scans.get(scan_id)?;
        let files = entry.files.read().unwrap().clone();
        let ol_completed = entry
            .ol_completed
            .load(std::sync::atomic::Ordering::Relaxed);
        Some(ScanSnapshot {
            files,
            warnings: entry.warnings.clone(),
            ol_total: entry.ol_total,
            ol_completed,
            user_id: entry.user_id,
        })
    }

    fn update_scan_file(&self, scan_id: &str, file_idx: usize, update: ScanFileUpdate) {
        if let Some(entry) = self.scans.get(scan_id) {
            let mut files = entry.files.write().unwrap();
            if let Some(f) = files.get_mut(file_idx) {
                if let Some(ol_match) = update.ol_match {
                    f.ol_match = Some(ol_match);
                }
                if let Some(work_id) = update.existing_work_id {
                    f.existing_work_id = Some(work_id);
                }
            }
        }
    }

    fn increment_ol_completed(&self, scan_id: &str) {
        if let Some(entry) = self.scans.get(scan_id) {
            entry
                .ol_completed
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn remove_scan(&self, scan_id: &str) {
        self.scans.remove(scan_id);
    }

    async fn acquire_ol_permit(&self) {
        self.ol_rate_limiter.acquire().await;
    }

    async fn search_ol_works(
        &self,
        term: &str,
        _limit: u32,
    ) -> Result<Vec<WorkSearchResult>, String> {
        let resp = self
            .http_client
            .get("https://openlibrary.org/search.json")
            .query(&[
                ("q", term),
                ("limit", "10"),
                (
                    "fields",
                    "key,title,author_name,author_key,first_publish_year,cover_i",
                ),
            ])
            .send()
            .await
            .map_err(|e| format!("OpenLibrary request failed: {e}"))?;

        if !resp.status().is_success() {
            return Err(format!("OpenLibrary returned {}", resp.status()));
        }

        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("OpenLibrary parse error: {e}"))?;

        let docs = data
            .get("docs")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();

        let results: Vec<WorkSearchResult> = docs
            .iter()
            .filter_map(|doc| {
                let key = doc.get("key")?.as_str()?;
                let title = doc.get("title")?.as_str()?;
                let ol_key = key.trim_start_matches("/works/").to_string();

                let author_name = doc
                    .get("author_name")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|a| a.as_str())
                    .unwrap_or("Unknown")
                    .to_string();

                let author_ol_key = doc
                    .get("author_key")
                    .and_then(|a| a.as_array())
                    .and_then(|a| a.first())
                    .and_then(|a| a.as_str())
                    .map(|k| k.trim_start_matches("/authors/").to_string());

                let year = doc
                    .get("first_publish_year")
                    .and_then(|y| y.as_i64())
                    .map(|y| y as i32);

                let cover_url = doc
                    .get("cover_i")
                    .and_then(|c| c.as_i64())
                    .map(|c| format!("https://covers.openlibrary.org/b/id/{c}-L.jpg"));

                Some(WorkSearchResult {
                    ol_key: Some(ol_key),
                    title: title.to_string(),
                    author_name,
                    author_ol_key,
                    year,
                    cover_url,
                    description: None,
                    series_name: None,
                    series_position: None,
                    source: None,
                    source_type: None,
                    language: None,
                    detail_url: None,
                    rating: None,
                })
            })
            .collect();

        Ok(results)
    }
}
