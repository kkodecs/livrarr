use livrarr_db::{CreateNotificationDbRequest, NotificationDb, WorkDb};
use livrarr_domain::services::*;
use livrarr_domain::*;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use chrono::Datelike;

/// OpenLibrary author works response — minimal parsing for monitor.
#[derive(Debug, serde::Deserialize)]
struct OlWorksResponse {
    entries: Vec<OlWorkEntry>,
}

#[derive(Debug, serde::Deserialize)]
struct OlWorkEntry {
    key: Option<String>,
    title: Option<String>,
    /// OL uses "first_publish_date" which may be a year string like "2024"
    /// or a full date like "January 1, 2024".
    first_publish_date: Option<String>,
}

impl OlWorkEntry {
    fn ol_key(&self) -> Option<&str> {
        // key is like "/works/OL12345W" — extract "OL12345W"
        self.key.as_deref().and_then(|k| k.strip_prefix("/works/"))
    }

    fn publish_year(&self) -> Option<i32> {
        self.first_publish_date.as_deref().and_then(|d| {
            // Extract first 4-digit numeric token (matches handler behavior)
            d.split(|c: char| !c.is_ascii_digit())
                .find(|tok| tok.len() == 4)
                .and_then(|tok| tok.parse::<i32>().ok())
        })
    }
}

pub struct AuthorMonitorWorkflowImpl<D, W, H> {
    db: Arc<D>,
    work_service: Arc<W>,
    http: Arc<H>,
    backoff_duration: Duration,
    inter_author_delay: Duration,
}

impl<D, W, H> AuthorMonitorWorkflowImpl<D, W, H> {
    pub fn new(db: Arc<D>, work_service: Arc<W>, http: Arc<H>) -> Self {
        Self {
            db,
            work_service,
            http,
            backoff_duration: Duration::from_secs(60),
            inter_author_delay: Duration::from_secs(1),
        }
    }

    pub fn with_backoff(mut self, backoff: Duration, inter_author: Duration) -> Self {
        self.backoff_duration = backoff;
        self.inter_author_delay = inter_author;
        self
    }
}

impl<D, W, H> AuthorMonitorWorkflow for AuthorMonitorWorkflowImpl<D, W, H>
where
    D: WorkDb + livrarr_db::AuthorDb + NotificationDb + Send + Sync + 'static,
    W: WorkService + Send + Sync + 'static,
    H: HttpFetcher + Send + Sync + 'static,
{
    async fn run_monitor(&self, cancel: CancellationToken) -> Result<MonitorReport, MonitorError> {
        let authors = self
            .db
            .list_monitored_authors()
            .await
            .map_err(MonitorError::Db)?;

        let mut report = MonitorReport {
            authors_checked: 0,
            new_works_found: 0,
            works_added: 0,
            notifications_created: 0,
        };

        // Index-based loop with retry map for 429 handling (matches handler).
        let mut i = 0;
        let mut retry_counts: HashMap<usize, u32> = HashMap::new();
        let mut rate_limit_notified = false;

        while i < authors.len() {
            let author = &authors[i];
            let ol_key = match &author.ol_key {
                Some(k) => k.clone(),
                None => {
                    i += 1;
                    continue;
                }
            };

            // Only count each author once (not on retries)
            if !retry_counts.contains_key(&i) {
                report.authors_checked += 1;
            }

            // Fetch OL author works
            let works_url = format!(
                "https://openlibrary.org/authors/{}/works.json?limit=100",
                ol_key
            );

            let req = FetchRequest {
                url: works_url,
                method: HttpMethod::Get,
                headers: vec![],
                body: None,
                timeout: Duration::from_secs(30),
                rate_bucket: RateBucket::OpenLibrary,
                max_body_bytes: 2 * 1024 * 1024,
                anti_bot_check: false,
                user_agent: UserAgentProfile::Server,
            };

            let fetch_result = self.http.fetch(req).await;

            // Handle fetch error
            let resp = match fetch_result {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!(
                        author_id = author.id,
                        author_name = %author.name,
                        error = %e,
                        "author monitor: OL request failed, skipping"
                    );
                    i += 1;
                    continue;
                }
            };

            // Handle 429 with backoff and retry
            if resp.status == 429 {
                let retries = retry_counts.entry(i).or_insert(0);
                *retries += 1;
                if *retries > 3 {
                    tracing::warn!(
                        author_id = author.id,
                        author_name = %author.name,
                        "author monitor: OL 429 — max retries exceeded, skipping"
                    );
                    i += 1;
                    continue;
                }
                tracing::warn!(
                    author_id = author.id,
                    author_name = %author.name,
                    attempt = *retries,
                    "author monitor: OL 429 — backing off (attempt {}/3)",
                    retries
                );

                // Rate-limit notification on first 429 per run
                if !rate_limit_notified {
                    rate_limit_notified = true;
                    if let Err(e) = self
                        .db
                        .create_notification(CreateNotificationDbRequest {
                            user_id: 1, // legacy system pseudo-user
                            notification_type: NotificationType::RateLimitHit,
                            ref_key: Some("author_monitor".into()),
                            message: "Open Library rate limit hit during author monitoring".into(),
                            data: serde_json::Value::Null,
                        })
                        .await
                    {
                        tracing::warn!("create_notification failed: {e}");
                    }
                }

                // Cancellation-aware backoff
                tokio::select! {
                    _ = tokio::time::sleep(self.backoff_duration) => {},
                    _ = cancel.cancelled() => { return Ok(report); },
                }
                // Retry same author (don't increment i).
                continue;
            }

            // Handle non-success HTTP
            if resp.status >= 400 {
                tracing::warn!(
                    author_id = author.id,
                    author_name = %author.name,
                    status = resp.status,
                    "author monitor: OL returned non-success status, skipping"
                );
                i += 1;
                continue;
            }

            // Parse JSON response
            let works_response: OlWorksResponse = match serde_json::from_slice(&resp.body) {
                Ok(parsed) => parsed,
                Err(e) => {
                    tracing::warn!(
                        author_id = author.id,
                        author_name = %author.name,
                        error = %e,
                        "author monitor: OL parse error, skipping"
                    );
                    i += 1;
                    continue;
                }
            };

            // Determine monitor_since year
            let monitor_since_year = author.monitor_since.map(|dt| dt.year()).unwrap_or(0);

            // Get existing work OL keys for dedup
            let existing_ol_keys = self
                .db
                .list_works_by_author_ol_keys(author.user_id, &ol_key)
                .await
                .unwrap_or_default();

            // Process each work entry
            for entry in &works_response.entries {
                let stripped_ol_key = match entry.ol_key() {
                    Some(k) => k.to_string(),
                    None => continue,
                };

                // Dedup against existing works (compare stripped keys — WorkService
                // stores the stripped form, so existing_ol_keys are stripped too)
                if existing_ol_keys.contains(&stripped_ol_key) {
                    continue;
                }

                // Extract publish year — skip if unparseable
                let year = match entry.publish_year() {
                    Some(y) => y,
                    None => {
                        tracing::debug!(
                            ol_key = %stripped_ol_key,
                            "author monitor: skipping work — unparseable date"
                        );
                        continue;
                    }
                };

                // Filter by monitor_since
                if year < monitor_since_year {
                    continue;
                }

                let raw_title = entry.title.as_deref().unwrap_or("Unknown").to_string();
                let work_title = crate::title_cleanup::clean_title(&raw_title);
                let cleaned_author = crate::title_cleanup::clean_author(&author.name);

                tracing::info!(
                    author_id = author.id,
                    year = year,
                    "author monitor: new work detected"
                );

                report.new_works_found += 1;

                if author.monitor_new_items {
                    // Auto-add via WorkService
                    let add_req = AddWorkRequest {
                        title: work_title.clone(),
                        author_name: cleaned_author.clone(),
                        author_ol_key: Some(ol_key.clone()),
                        ol_key: Some(stripped_ol_key.clone()),
                        gr_key: None,
                        year: Some(year),
                        cover_url: None,
                        metadata_source: None,
                        language: None,
                        detail_url: None,
                        series_name: None,
                        series_position: None,
                        defer_enrichment: false,
                        provenance_setter: Some(ProvenanceSetter::AutoAdded),
                    };

                    match self.work_service.add(author.user_id, add_req).await {
                        Ok(_work) => {
                            report.works_added += 1;
                        }
                        Err(e) => {
                            tracing::warn!(
                                author_id = author.id,
                                ol_key = %stripped_ol_key,
                                error = %e,
                                "author monitor: failed to auto-add work"
                            );
                        }
                    }

                    // WorkAutoAdded notification
                    if let Err(e) = self
                        .db
                        .create_notification(CreateNotificationDbRequest {
                            user_id: author.user_id,
                            notification_type: NotificationType::WorkAutoAdded,
                            ref_key: Some(stripped_ol_key.clone()),
                            message: format!(
                                "New work '{}' by {} auto-added to your library",
                                work_title, author.name
                            ),
                            data: serde_json::json!({
                                "title": work_title,
                                "author": author.name,
                                "year": year,
                                "ol_key": stripped_ol_key,
                            }),
                        })
                        .await
                    {
                        tracing::warn!("create_notification failed: {e}");
                    } else {
                        report.notifications_created += 1;
                    }
                } else {
                    // Notification only — NewWorkDetected
                    if let Err(e) = self
                        .db
                        .create_notification(CreateNotificationDbRequest {
                            user_id: author.user_id,
                            notification_type: NotificationType::NewWorkDetected,
                            ref_key: Some(stripped_ol_key.clone()),
                            message: format!(
                                "New work '{}' by {} detected",
                                work_title, author.name
                            ),
                            data: serde_json::json!({
                                "title": work_title,
                                "author": author.name,
                                "year": year,
                                "ol_key": stripped_ol_key,
                            }),
                        })
                        .await
                    {
                        tracing::warn!("create_notification failed: {e}");
                    } else {
                        report.notifications_created += 1;
                    }
                }
            }

            // Rate limit respect: 1s delay between authors (cancellation-aware).
            tokio::select! {
                _ = tokio::time::sleep(self.inter_author_delay) => {},
                _ = cancel.cancelled() => { return Ok(report); },
            }
            i += 1;
        }

        Ok(report)
    }
}
