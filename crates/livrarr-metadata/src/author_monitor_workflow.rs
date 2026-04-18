use livrarr_db::{CreateNotificationDbRequest, NotificationDb, WorkDb};
use livrarr_domain::services::*;
use livrarr_domain::*;
use std::sync::Arc;
use std::time::Duration;

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
            // Try parsing as bare year first
            if let Ok(y) = d.trim().parse::<i32>() {
                return Some(y);
            }
            // Try extracting trailing 4-digit year (e.g. "January 1, 2024")
            d.split_whitespace()
                .rev()
                .find_map(|tok| tok.trim_matches(',').parse::<i32>().ok())
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
    async fn run_monitor(&self, user_id: UserId) -> Result<MonitorReport, MonitorError> {
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

        for (idx, author) in authors.iter().enumerate() {
            // Inter-author delay (skip before first)
            if idx > 0 {
                tokio::time::sleep(self.inter_author_delay).await;
            }

            let ol_key = match &author.ol_key {
                Some(k) => k.clone(),
                None => continue,
            };

            report.authors_checked += 1;

            // Fetch works from OL with retry on 429
            let works_response = match self.fetch_author_works(&ol_key).await {
                Ok(resp) => resp,
                Err(MonitorError::RateLimited) => {
                    // Already retried 3 times — log and continue
                    tracing::warn!(
                        author_id = author.id,
                        ol_key = %ol_key,
                        "OL rate limited after retries, skipping author"
                    );
                    continue;
                }
                Err(e) => {
                    tracing::warn!(
                        author_id = author.id,
                        ol_key = %ol_key,
                        error = %e,
                        "OL error for author, continuing to next"
                    );
                    continue;
                }
            };

            // Determine monitor_since year
            let monitor_since_year = author.monitor_since.map(|dt| dt.year()).unwrap_or(0);

            // Process each work entry
            for entry in &works_response.entries {
                let entry_ol_key = match entry.ol_key() {
                    Some(k) => k.to_string(),
                    None => continue,
                };

                let entry_title = match &entry.title {
                    Some(t) if !t.trim().is_empty() => t.trim().to_string(),
                    _ => continue,
                };

                // Extract publish year — skip if unparseable
                let publish_year = match entry.publish_year() {
                    Some(y) => y,
                    None => continue,
                };

                // Filter by monitor_since
                if publish_year < monitor_since_year {
                    continue;
                }

                // Check if work already exists
                let exists = self
                    .db
                    .work_exists_by_ol_key(user_id, &entry_ol_key)
                    .await
                    .map_err(MonitorError::Db)?;

                if exists {
                    continue;
                }

                report.new_works_found += 1;

                if author.monitor_new_items {
                    // Auto-add via WorkService
                    let add_req = AddWorkRequest {
                        title: entry_title.clone(),
                        author_name: Some(author.name.clone()),
                        isbn: None,
                        ol_key: Some(entry_ol_key.clone()),
                        hc_key: None,
                        detail_url: None,
                        cover_url: None,
                        media_type: None,
                        monitored: true,
                    };

                    match self.work_service.add(user_id, add_req).await {
                        Ok(_work) => {
                            report.works_added += 1;
                        }
                        Err(e) => {
                            tracing::warn!(
                                author_id = author.id,
                                ol_key = %entry_ol_key,
                                error = %e,
                                "failed to auto-add work, still creating notification"
                            );
                        }
                    }

                    // Create WorkAutoAdded notification
                    let notif_result = self
                        .db
                        .create_notification(CreateNotificationDbRequest {
                            user_id,
                            notification_type: NotificationType::WorkAutoAdded,
                            ref_key: Some(entry_ol_key.clone()),
                            message: format!("Auto-added '{}' by {}", entry_title, author.name),
                            data: serde_json::json!({
                                "author_id": author.id,
                                "author_name": author.name,
                                "ol_key": entry_ol_key,
                                "title": entry_title,
                                "year": publish_year,
                            }),
                        })
                        .await;

                    if notif_result.is_ok() {
                        report.notifications_created += 1;
                    }
                } else {
                    // Notification only — NewWorkDetected
                    let notif_result = self
                        .db
                        .create_notification(CreateNotificationDbRequest {
                            user_id,
                            notification_type: NotificationType::NewWorkDetected,
                            ref_key: Some(entry_ol_key.clone()),
                            message: format!(
                                "New work detected: '{}' by {}",
                                entry_title, author.name
                            ),
                            data: serde_json::json!({
                                "author_id": author.id,
                                "author_name": author.name,
                                "ol_key": entry_ol_key,
                                "title": entry_title,
                                "year": publish_year,
                            }),
                        })
                        .await;

                    if notif_result.is_ok() {
                        report.notifications_created += 1;
                    }
                }
            }
        }

        Ok(report)
    }
}

impl<D, W, H> AuthorMonitorWorkflowImpl<D, W, H>
where
    D: Send + Sync,
    H: HttpFetcher + Send + Sync,
{
    /// Fetch author works from OL with 429 backoff (up to 3 retries).
    async fn fetch_author_works(&self, ol_key: &str) -> Result<OlWorksResponse, MonitorError> {
        let url = format!(
            "https://openlibrary.org/authors/{}/works.json?limit=100",
            ol_key
        );

        let mut attempts = 0;
        const MAX_RETRIES: usize = 3;

        loop {
            let req = FetchRequest {
                url: url.clone(),
                method: HttpMethod::Get,
                headers: vec![],
                body: None,
                timeout: Duration::from_secs(30),
                rate_bucket: RateBucket::OpenLibrary,
                max_body_bytes: 2 * 1024 * 1024,
                anti_bot_check: false,
                user_agent: UserAgentProfile::Server,
            };

            match self.http.fetch(req).await {
                Ok(resp) if resp.status == 429 => {
                    attempts += 1;
                    if attempts > MAX_RETRIES {
                        return Err(MonitorError::RateLimited);
                    }
                    tracing::info!(ol_key, attempt = attempts, "OL 429 — backing off 60s");
                    tokio::time::sleep(self.backoff_duration).await;
                    continue;
                }
                Ok(resp) if resp.status >= 400 => {
                    return Err(MonitorError::ProviderFailed(format!(
                        "OL returned HTTP {}",
                        resp.status
                    )));
                }
                Ok(resp) => {
                    let parsed: OlWorksResponse =
                        serde_json::from_slice(&resp.body).map_err(|e| {
                            MonitorError::ProviderFailed(format!(
                                "failed to parse OL works response: {e}"
                            ))
                        })?;
                    return Ok(parsed);
                }
                Err(FetchError::RateLimited) => {
                    attempts += 1;
                    if attempts > MAX_RETRIES {
                        return Err(MonitorError::RateLimited);
                    }
                    tokio::time::sleep(self.backoff_duration).await;
                    continue;
                }
                Err(e) => {
                    return Err(MonitorError::ProviderFailed(format!("OL fetch error: {e}")));
                }
            }
        }
    }
}

// Need chrono for year extraction from monitor_since
use chrono::Datelike;
