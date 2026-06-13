//! ListService implementation — CSV imports from Goodreads and Hardcover.
//!
//! 5-step workflow: preview -> confirm (batched) -> complete -> undo -> list.
//! All business logic lives here. Handlers: validate -> call ONE service -> map result.

use chrono::Utc;
use futures::stream::{self, StreamExt};
use tracing::{info, warn};

use livrarr_db::ListImportDb;
use livrarr_domain::services::*;
use livrarr_domain::UserId;

use livrarr_external_data::parsers::{self, CsvSource, ParseError};

// ---------------------------------------------------------------------------
// ListServiceImpl
// ---------------------------------------------------------------------------

pub struct ListServiceImpl<D, W, H, B> {
    pub db: D,
    pub work_service: W,
    pub http: H,
    pub bibliography_trigger: B,
}

impl<D, W, H, B> ListServiceImpl<D, W, H, B> {
    pub fn new(db: D, work_service: W, http: H, bibliography_trigger: B) -> Self {
        Self {
            db,
            work_service,
            http,
            bibliography_trigger,
        }
    }
}

// ---------------------------------------------------------------------------
// Row -> identity candidate (resolved synchronously through the one road)
// ---------------------------------------------------------------------------

impl<D, W, H, B> ListServiceImpl<D, W, H, B>
where
    W: WorkService,
{
    /// Resolve a confirmed preview row's identity through the shared
    /// [`WorkService::resolve_identity`] — the same road the interactive Add-Work
    /// door takes (P-A/P-B). The row's harvested anchors (Goodreads Book Id, ISBN)
    /// resolve to a canonical work anchor synchronously: `user_confirmed = false`
    /// forces the multi-provider fan-out that finds the OpenLibrary work key,
    /// because a list row is not a per-book identity confirmation. A resolution
    /// miss lands `Pending` and is still added — the import is never blocked
    /// (REQ-013); a pending row converges later via "retry all incomplete".
    async fn resolve_candidate_from_row(
        &self,
        user_id: UserId,
        row: &livrarr_db::ListImportPreviewRow,
        language: Option<&str>,
    ) -> livrarr_domain::identity::WorkCandidate {
        use livrarr_domain::identity::{
            CapturedIdentity, IdentityState, LatencyTier, PendingReason, RawHarvest,
        };
        use livrarr_domain::normalization::{normalize_gr_key, normalize_isbn13};

        let gr_key = row.goodreads_book_id.as_deref().and_then(normalize_gr_key);
        let isbn_13 = row
            .isbn_13
            .as_deref()
            .or(row.isbn_10.as_deref())
            .and_then(normalize_isbn13);

        // Background lane: a foreground interactive add always outranks bulk
        // import (REQ-010).
        let resolved = self
            .work_service
            .resolve_identity(
                user_id,
                RawHarvest {
                    ol_key: None,
                    gr_key: gr_key.clone(),
                    hc_key: None,
                    isbn: isbn_13.clone(),
                    asin: None,
                    title: Some(row.title.clone()),
                    author_name: Some(row.author.clone()),
                    language: Some("en".to_string()),
                    series_name: None,
                    year: row.year,
                    user_confirmed: false,
                },
                LatencyTier::Background,
            )
            .await;

        // A resolver infrastructure error never blocks the import: seed a Pending
        // candidate that a later user-triggered "retry all incomplete" converges.
        let (identity, candidate_id) = match resolved {
            Ok(r) => (r.identity, r.candidate_id),
            Err(e) => {
                warn!(error = %e, title = %row.title, "list import: identity resolve failed; seeding Pending");
                (
                    IdentityState::Pending {
                        reason: PendingReason::NoCandidates,
                        seed_anchors: Some(CapturedIdentity {
                            ol_key: None,
                            gr_key,
                            hc_key: None,
                            isbn_13,
                            asin: None,
                            title: row.title.clone(),
                            author_name: row.author.clone(),
                            language: None,
                        }),
                        top_candidates: vec![],
                    },
                    None,
                )
            }
        };

        livrarr_domain::seed::seed_list_import(
            livrarr_domain::seed::SeedInput {
                title: row.title.clone(),
                author_name: row.author.clone(),
                language: livrarr_domain::seed::SeedLanguage::resolve(language),
                author_ol_key: None,
                year: row.year,
                cover_url: None,
                detail_url: None,
                description: None,
                series_name: None,
                series_position: None,
            },
            identity,
            candidate_id,
        )
    }
}

// ---------------------------------------------------------------------------
// ListService trait implementation
// ---------------------------------------------------------------------------

impl<D, W, H, B> ListService for ListServiceImpl<D, W, H, B>
where
    D: ListImportDb + livrarr_db::WorkDb + Send + Sync,
    W: WorkService + Send + Sync,
    H: HttpFetcher + Send + Sync,
    B: BibliographyTrigger + Send + Sync,
{
    async fn preview(
        &self,
        user_id: UserId,
        bytes: Vec<u8>,
    ) -> Result<ListPreviewResponse, ListServiceError> {
        if bytes.is_empty() {
            return Err(ListServiceError::Parse("uploaded file is empty".into()));
        }
        if bytes.len() > 20 * 1024 * 1024 {
            return Err(ListServiceError::Parse("file too large (max 20MB)".into()));
        }

        // CSV parsing can be CPU-intensive for large files — run off async executor.
        let (source, rows) = tokio::task::spawn_blocking(move || -> Result<_, ListServiceError> {
            let stripped = parsers::strip_bom_pub(&bytes);
            let mut rdr = csv::ReaderBuilder::new()
                .flexible(true)
                .from_reader(stripped);

            let headers = rdr
                .headers()
                .map_err(|e| ListServiceError::Parse(format!("invalid CSV: {e}")))?
                .clone();

            let source = parsers::detect_csv_source(&headers).map_err(|e| match e {
                ParseError::UnknownFormat {
                    detected_headers, ..
                } => ListServiceError::Parse(format!(
                    "unrecognized CSV format. Detected headers: {}",
                    detected_headers.join(", ")
                )),
                other => ListServiceError::Parse(other.to_string()),
            })?;

            let rows = match source {
                CsvSource::Goodreads => parsers::parse_goodreads_csv(&bytes),
                CsvSource::Hardcover => parsers::parse_hardcover_csv(&bytes),
            }
            .map_err(|e| ListServiceError::Parse(e.to_string()))?;

            Ok((source, rows))
        })
        .await
        .map_err(|e| ListServiceError::Parse(format!("CSV parse task failed: {e}")))??;

        let source_str = match source {
            CsvSource::Goodreads => "goodreads",
            CsvSource::Hardcover => "hardcover",
        };

        // Generate preview_id.
        let preview_id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();

        let existing_works = self
            .db
            .list_works(user_id)
            .await
            .map_err(ListServiceError::Db)?;
        let existing_keys: std::collections::HashSet<String> = existing_works
            .iter()
            .map(|w| normalize_for_dedup(&w.title, &w.author_name))
            .collect();

        let mut preview_rows = Vec::with_capacity(rows.len());

        for row in &rows {
            let status = if row.title.is_empty() {
                "parse_error"
            } else if existing_keys.contains(&normalize_for_dedup(&row.title, &row.author)) {
                "already_exists"
            } else {
                "new"
            };

            // Persist to preview table.
            if let Err(e) = self
                .db
                .insert_list_import_preview_row(
                    &preview_id,
                    user_id,
                    row.row_index as i64,
                    &row.title,
                    &row.author,
                    row.isbn_13.as_deref(),
                    row.isbn_10.as_deref(),
                    row.goodreads_book_id.as_deref(),
                    row.year,
                    row.status.map(|s| format!("{s:?}")).as_deref(),
                    row.rating,
                    status,
                    source_str,
                    &now,
                )
                .await
            {
                return Err(ListServiceError::Db(e));
            }

            preview_rows.push(ListPreviewRow {
                row_index: row.row_index,
                title: row.title.clone(),
                author: row.author.clone(),
                isbn_13: row.isbn_13.clone(),
                isbn_10: row.isbn_10.clone(),
                year: row.year,
                source_status: row.status.map(|s| format!("{s:?}")),
                source_rating: row.rating,
                preview_status: status.to_string(),
            });
        }

        info!(
            user_id,
            source = source_str,
            rows = preview_rows.len(),
            preview_id = %preview_id,
            "list import preview created"
        );

        Ok(ListPreviewResponse {
            preview_id,
            source: source_str.to_string(),
            total_rows: preview_rows.len(),
            rows: preview_rows,
        })
    }

    async fn confirm(
        &self,
        user_id: UserId,
        preview_id: &str,
        import_id: Option<&str>,
        row_indices: &[usize],
        language: Option<&str>,
    ) -> Result<ListConfirmResponse, ListServiceError> {
        // Validate preview exists for this user.
        let preview_count = self
            .db
            .count_list_import_previews(preview_id, user_id)
            .await
            .map_err(ListServiceError::Db)?;

        if preview_count == 0 {
            return Err(ListServiceError::Parse(
                "preview not found or expired".into(),
            ));
        }

        // Get or create import record.
        let resolved_import_id = if let Some(id) = import_id {
            // Validate ownership and status.
            let record = self
                .db
                .get_list_import_record(id)
                .await
                .map_err(ListServiceError::Db)?
                .ok_or(ListServiceError::NotFound)?;

            if record.user_id != user_id {
                return Err(ListServiceError::NotFound);
            }
            if record.status != "running" {
                return Err(ListServiceError::Conflict(format!(
                    "import is {}, not running",
                    record.status
                )));
            }
            id.to_string()
        } else {
            // Get source from preview.
            let source = self
                .db
                .get_list_import_source(preview_id, user_id)
                .await
                .map_err(ListServiceError::Db)?;

            // Create new import record.
            let id = uuid::Uuid::new_v4().to_string();
            let now = Utc::now().to_rfc3339();
            self.db
                .create_list_import_record(&id, user_id, &source, &now)
                .await
                .map_err(ListServiceError::Db)?;
            id
        };

        // Process rows with M9 bounded concurrency. Each row's pipeline
        // (preview lookup -> OL lookup -> work_service.add) runs in its own
        // future; up to 5 run in parallel. work_service.add is itself
        // synchronous and fully enriches before returning, so the
        // concurrency is between distinct rows, not within one.
        //
        // Each future produces a self-contained RowOutcome. The serial
        // post-pass folds outcomes into running totals.
        struct RowOutcome {
            result: ListConfirmRowResult,
            works_created: bool,
            new_author_id: Option<i64>,
        }

        let resolved_id_ref = &resolved_import_id;
        let outcomes: Vec<RowOutcome> = stream::iter(row_indices.iter().copied())
            .map(|row_idx| async move {
                let row = match self
                    .db
                    .get_list_import_preview_row(preview_id, user_id, row_idx as i64)
                    .await
                {
                    Ok(Some(r)) => r,
                    Ok(None) => {
                        return RowOutcome {
                            result: ListConfirmRowResult {
                                row_index: row_idx,
                                status: "add_failed".into(),
                                message: Some("row not found in preview".into()),
                            },
                            works_created: false,
                            new_author_id: None,
                        };
                    }
                    Err(e) => {
                        return RowOutcome {
                            result: ListConfirmRowResult {
                                row_index: row_idx,
                                status: "add_failed".into(),
                                message: Some(format!("{e}")),
                            },
                            works_created: false,
                            new_author_id: None,
                        };
                    }
                };

                // Resolve identity synchronously through the shared one-road
                // resolver (same path as the interactive Add-Work door), then add.
                // A resolvable row lands Confirmed; a miss lands Pending without
                // blocking the import.
                let add_req = self
                    .resolve_candidate_from_row(user_id, &row, language)
                    .await;

                match self.work_service.add(user_id, add_req).await {
                    Ok(add_result) if !add_result.created => RowOutcome {
                        result: ListConfirmRowResult {
                            row_index: row_idx,
                            status: "already_exists".into(),
                            message: None,
                        },
                        works_created: false,
                        new_author_id: None,
                    },
                    Ok(add_result) => {
                        if let Err(e) = self
                            .db
                            .tag_work_with_import(user_id, add_result.work.id, resolved_id_ref)
                            .await
                        {
                            warn!(
                                user_id,
                                work_id = add_result.work.id,
                                import_id = %resolved_id_ref,
                                "tag_work_with_import failed (non-fatal): {e}"
                            );
                        }
                        let new_author_id = if add_result.author_created {
                            add_result.author_id
                        } else {
                            None
                        };
                        RowOutcome {
                            result: ListConfirmRowResult {
                                row_index: row_idx,
                                status: "added".into(),
                                message: None,
                            },
                            works_created: true,
                            new_author_id,
                        }
                    }
                    Err(e) => {
                        warn!(row_idx, error = %e, "list import: add_work failed");
                        RowOutcome {
                            result: ListConfirmRowResult {
                                row_index: row_idx,
                                status: "add_failed".into(),
                                message: Some(format!("{e}")),
                            },
                            works_created: false,
                            new_author_id: None,
                        }
                    }
                }
            })
            .buffer_unordered(5)
            .collect()
            .await;

        // Fold outcomes into final state. Preserve input order for results.
        let mut results = Vec::with_capacity(row_indices.len());
        let mut works_created: i64 = 0;
        let mut new_author_ids: Vec<i64> = Vec::new();
        // buffer_unordered yields out-of-order; sort by row_index to get a
        // stable response.
        let mut outcomes = outcomes;
        outcomes.sort_by_key(|o| o.result.row_index);
        for outcome in outcomes {
            if outcome.works_created {
                works_created += 1;
            }
            if let Some(aid) = outcome.new_author_id {
                if !new_author_ids.contains(&aid) {
                    new_author_ids.push(aid);
                }
            }
            results.push(outcome.result);
        }

        // Update import counters (non-fatal if this fails).
        if let Err(e) = self
            .db
            .increment_list_import_works_created(&resolved_import_id, works_created)
            .await
        {
            warn!(
                import_id = %resolved_import_id,
                "increment_list_import_works_created failed (non-fatal): {e}"
            );
        }

        // Trigger bibliography for newly created authors.
        for author_id in new_author_ids {
            self.bibliography_trigger.trigger(author_id, user_id);
        }

        info!(
            user_id,
            import_id = %resolved_import_id,
            batch_size = row_indices.len(),
            works_created,
            "list import confirm batch processed"
        );

        Ok(ListConfirmResponse {
            import_id: resolved_import_id,
            results,
        })
    }

    async fn complete(&self, user_id: UserId, import_id: &str) -> Result<(), ListServiceError> {
        let now = Utc::now().to_rfc3339();
        let rows_affected = self
            .db
            .complete_list_import(import_id, user_id, &now)
            .await
            .map_err(ListServiceError::Db)?;

        if rows_affected == 0 {
            return Err(ListServiceError::NotFound);
        }

        info!(user_id, import_id = %import_id, "list import completed");
        Ok(())
    }

    async fn undo(
        &self,
        user_id: UserId,
        import_id: &str,
    ) -> Result<ListUndoResponse, ListServiceError> {
        // Validate import exists and belongs to user.
        let status = self
            .db
            .get_list_import_status_for_user(import_id, user_id)
            .await
            .map_err(ListServiceError::Db)?
            .ok_or(ListServiceError::NotFound)?;

        if status == "undone" {
            return Err(ListServiceError::Conflict("import already undone".into()));
        }

        // Enumerate works created by this import and delete via WorkService.
        let work_ids = self
            .db
            .list_works_by_import(import_id, user_id)
            .await
            .map_err(ListServiceError::Db)?;

        let mut works_removed: usize = 0;
        let mut works_skipped: usize = 0;

        for work_id in &work_ids {
            match self.work_service.delete(user_id, *work_id).await {
                Ok(()) => works_removed += 1,
                Err(e) => {
                    warn!(
                        user_id,
                        work_id,
                        import_id = %import_id,
                        "undo: work delete failed (skipping): {e}"
                    );
                    works_skipped += 1;
                }
            }
        }

        // Mark import as undone.
        if let Err(e) = self.db.mark_list_import_undone(import_id).await {
            warn!(import_id = %import_id, "mark_list_import_undone failed: {e}");
        }

        info!(
            user_id,
            import_id = %import_id,
            works_removed,
            works_skipped,
            "list import undone"
        );

        Ok(ListUndoResponse {
            works_removed,
            works_skipped,
        })
    }

    async fn list_imports(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ListImportSummary>, ListServiceError> {
        let rows = self
            .db
            .list_list_imports(user_id)
            .await
            .map_err(ListServiceError::Db)?;

        let summaries = rows
            .into_iter()
            .map(|r| ListImportSummary {
                id: r.id,
                source: r.source,
                status: r.status,
                started_at: r.started_at,
                completed_at: r.completed_at,
                works_created: r.works_created,
            })
            .collect();

        Ok(summaries)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn normalize_for_dedup(title: &str, author: &str) -> String {
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(|c| c.to_lowercase())
            .collect()
    };
    format!("{}::{}", norm(title), norm(author))
}

// ---------------------------------------------------------------------------
// No-op BibliographyTrigger for tests
// ---------------------------------------------------------------------------

/// No-op bibliography trigger for unit/behavioral tests.
pub struct NoOpBibliographyTrigger;

impl BibliographyTrigger for NoOpBibliographyTrigger {
    fn trigger(&self, _author_id: i64, _user_id: UserId) {
        // no-op
    }
}
