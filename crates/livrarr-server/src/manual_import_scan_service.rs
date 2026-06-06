use std::sync::Arc;

use livrarr_handlers::accessors::ManualImportScanAccessor;
use livrarr_handlers::manual_import::{ScanFileUpdate, ScanSnapshot, ScannedFile};

use crate::state::{ManualImportScanMap, ManualImportScanState};

#[derive(Clone)]
pub struct LiveManualImportScanService {
    pub scans: Arc<ManualImportScanMap>,
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
}
