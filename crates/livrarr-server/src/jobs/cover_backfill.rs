use std::path::PathBuf;

use livrarr_db::sqlite::SqliteDb;
use livrarr_http::fetcher::HttpFetcherImpl;
use livrarr_metadata::work_service::download_cover_to_disk;

pub async fn run_cover_backfill(db: SqliteDb, covers_dir: PathBuf) {
    let http = match HttpFetcherImpl::new() {
        Ok(h) => h,
        Err(e) => {
            tracing::error!(error = %e, "cover backfill: failed to create HTTP client");
            return;
        }
    };

    let works = match sqlx::query_as::<_, (i64, String)>(
        "SELECT id, cover_url FROM works WHERE cover_url IS NOT NULL AND cover_url != ''",
    )
    .fetch_all(db.pool())
    .await
    {
        Ok(w) => w,
        Err(e) => {
            tracing::error!(error = %e, "cover backfill: failed to query works");
            return;
        }
    };

    let mut downloaded = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;

    for (work_id, cover_url) in &works {
        let cover_path = covers_dir.join(format!("{work_id}.jpg"));
        if cover_path.exists() {
            skipped += 1;
            continue;
        }

        match download_cover_to_disk(&http, cover_url, &covers_dir, *work_id, "").await {
            Ok(()) => {
                downloaded += 1;
                tracing::debug!(work_id, "cover backfill: downloaded");
            }
            Err(e) => {
                failed += 1;
                tracing::warn!(work_id, error = %e, "cover backfill: failed");
            }
        }
    }

    if downloaded > 0 || failed > 0 {
        tracing::info!(downloaded, skipped, failed, "cover backfill: complete");
    }
}
