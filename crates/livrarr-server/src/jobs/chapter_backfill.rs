use std::path::PathBuf;

use livrarr_db::sqlite::SqliteDb;
use livrarr_db::ChapterDb;

pub async fn run_chapter_backfill(db: SqliteDb) {
    let items = match db.list_unscanned_audiobook_items().await {
        Ok(items) => items,
        Err(e) => {
            tracing::error!(error = %e, "chapter backfill: failed to list unscanned items");
            return;
        }
    };

    if items.is_empty() {
        tracing::info!("chapter backfill: no items to scan");
        return;
    }

    tracing::info!(count = items.len(), "chapter backfill: starting");
    let mut chapters_found: u32 = 0;
    let mut no_chapters: u32 = 0;
    let mut parse_errors: u32 = 0;
    let mut io_errors: u32 = 0;

    for (item_id, path_str) in &items {
        let path = PathBuf::from(path_str);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        if ext.as_str() != "m4b" {
            let _ = db
                .update_chapter_scan_result(*item_id, "no_chapters", None)
                .await;
            no_chapters += 1;
            continue;
        }

        let item_id_val = *item_id;
        let path_clone = path.clone();
        let result = tokio::task::spawn_blocking(move || {
            livrarr_tagwrite::extract_m4b_chapters(&path_clone)
        })
        .await;

        match result {
            Ok(Ok(extraction)) => {
                let dur = extraction.duration_secs;
                if extraction.chapters.is_empty() {
                    let _ = db
                        .update_chapter_scan_result(item_id_val, "no_chapters", dur)
                        .await;
                    no_chapters += 1;
                } else {
                    let mut chs = Vec::new();
                    let extracted = &extraction.chapters;
                    for (i, ch) in extracted.iter().enumerate() {
                        let title = if ch.title.is_empty() {
                            format!("Chapter {}", i + 1)
                        } else {
                            ch.title.clone()
                        };
                        let end_time = if i + 1 < extracted.len() {
                            extracted[i + 1].start_time_secs
                        } else {
                            match dur {
                                Some(d) if d > ch.start_time_secs => d,
                                _ => {
                                    tracing::warn!(
                                        item_id = item_id_val,
                                        "backfill: last chapter has no valid end time — dropping"
                                    );
                                    continue;
                                }
                            }
                        };
                        chs.push(livrarr_domain::AudiobookChapter {
                            id: 0,
                            library_item_id: item_id_val,
                            chapter_index: i as i32,
                            title,
                            start_time_secs: ch.start_time_secs,
                            end_time_secs: end_time,
                        });
                    }
                    if chs.is_empty() {
                        let _ = db
                            .update_chapter_scan_result(item_id_val, "no_chapters", dur)
                            .await;
                        no_chapters += 1;
                    } else {
                        match db.replace_chapters(item_id_val, &chs).await {
                            Ok(()) => {
                                let _ = db
                                    .update_chapter_scan_result(item_id_val, "scanned", dur)
                                    .await;
                                chapters_found += 1;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    item_id = item_id_val,
                                    error = %e,
                                    "backfill: replace_chapters failed — leaving scan_status NULL for retry"
                                );
                                io_errors += 1;
                            }
                        }
                    }
                }
            }
            Ok(Err(livrarr_tagwrite::ChapterExtractionError::ParseError(_))) => {
                tracing::warn!(
                    item_id = item_id_val,
                    "backfill: corrupt M4B — marking parse_error"
                );
                let _ = db
                    .update_chapter_scan_result(item_id_val, "parse_error", None)
                    .await;
                parse_errors += 1;
            }
            Ok(Err(livrarr_tagwrite::ChapterExtractionError::IoError(e))) => {
                tracing::warn!(item_id = item_id_val, error = %e, "backfill: I/O error — will retry");
                io_errors += 1;
            }
            Err(e) => {
                tracing::warn!(item_id = item_id_val, error = %e, "backfill: task panicked");
                io_errors += 1;
            }
        }
    }

    tracing::info!(
        chapters_found,
        no_chapters,
        parse_errors,
        io_errors,
        "chapter backfill: complete"
    );
}
