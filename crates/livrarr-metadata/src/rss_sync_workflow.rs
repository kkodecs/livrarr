use livrarr_db::{
    CreateNotificationDbRequest, GrabDb, IndexerDb, LibraryItemDb, NotificationDb, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::*;
use std::sync::Arc;
use std::time::Duration;

/// Parsed RSS feed item.
#[derive(Debug, Clone)]
struct RssFeedItem {
    title: String,
    guid: String,
    download_url: String,
    size: i64,
    seeders: Option<i32>,
    publish_date: Option<String>,
    categories: Vec<i32>,
}

/// Match candidate: a monitored work to match against feed items.
#[derive(Debug, Clone)]
struct MatchCandidate {
    work: Work,
}

/// A scored match between a feed item and a work.
#[derive(Debug)]
struct ScoredMatch {
    feed_item: RssFeedItem,
    work: Work,
    score: f64,
    indexer: Indexer,
}

pub struct RssSyncWorkflowImpl<D, H, R> {
    db: Arc<D>,
    http: Arc<H>,
    release_service: Arc<R>,
}

impl<D, H, R> RssSyncWorkflowImpl<D, H, R> {
    pub fn new(db: Arc<D>, http: Arc<H>, release_service: Arc<R>) -> Self {
        Self {
            db,
            http,
            release_service,
        }
    }
}

impl<D, H, R> RssSyncWorkflow for RssSyncWorkflowImpl<D, H, R>
where
    D: IndexerDb + WorkDb + GrabDb + LibraryItemDb + NotificationDb + Send + Sync + 'static,
    H: HttpFetcher + Send + Sync + 'static,
    R: ReleaseService + Send + Sync + 'static,
{
    async fn run_sync(&self) -> Result<RssSyncReport, RssSyncError> {
        // 1. List enabled RSS indexers
        let indexers = self
            .db
            .list_enabled_rss_indexers()
            .await
            .map_err(RssSyncError::Db)?;

        if indexers.is_empty() {
            return Ok(RssSyncReport {
                feeds_checked: 0,
                releases_matched: 0,
                grabs_attempted: 0,
                grabs_succeeded: 0,
                warnings: vec![],
            });
        }

        let mut report = RssSyncReport {
            feeds_checked: 0,
            releases_matched: 0,
            grabs_attempted: 0,
            grabs_succeeded: 0,
            warnings: vec![],
        };

        // 2. Pre-query RSS state for all indexers (first-sync detection)
        let mut first_sync_indexers = std::collections::HashSet::new();
        for indexer in &indexers {
            let state = self
                .db
                .get_rss_state(indexer.id)
                .await
                .map_err(RssSyncError::Db)?;
            if state.is_none() {
                first_sync_indexers.insert(indexer.id);
            }
        }

        // 3. Fetch RSS feeds (sequential with timeout per indexer)
        let mut all_feed_items: Vec<(Indexer, Vec<RssFeedItem>)> = Vec::new();

        for indexer in &indexers {
            let feed_url = build_rss_url(indexer);

            let req = FetchRequest {
                url: feed_url,
                method: HttpMethod::Get,
                headers: vec![],
                body: None,
                timeout: Duration::from_secs(30),
                rate_bucket: RateBucket::Indexer(indexer.name.clone()),
                max_body_bytes: 5 * 1024 * 1024,
                anti_bot_check: false,
                user_agent: UserAgentProfile::Server,
            };

            match self.http.fetch(req).await {
                Ok(resp) if resp.status >= 400 => {
                    report.warnings.push(format!(
                        "Indexer '{}' returned HTTP {}",
                        indexer.name, resp.status
                    ));
                    continue;
                }
                Ok(resp) => {
                    report.feeds_checked += 1;
                    let items = parse_rss_feed(&resp.body);

                    // Update RSS state
                    if let Some(first_item) = items.first() {
                        let _ = self
                            .db
                            .upsert_rss_state(
                                indexer.id,
                                first_item.publish_date.as_deref(),
                                &first_item.guid,
                            )
                            .await;
                    }

                    all_feed_items.push((indexer.clone(), items));
                }
                Err(e) => {
                    report
                        .warnings
                        .push(format!("Indexer '{}' fetch failed: {}", indexer.name, e));
                    continue;
                }
            }
        }

        // 4. First-sync indexers: record state only, no grabs
        all_feed_items.retain(|(indexer, _)| !first_sync_indexers.contains(&indexer.id));

        // 5. Get all monitored works
        let monitored_works = self
            .db
            .list_monitored_works_all_users()
            .await
            .map_err(RssSyncError::Db)?;

        if monitored_works.is_empty() {
            return Ok(report);
        }

        // 6. Build match candidates
        let candidates: Vec<MatchCandidate> = monitored_works
            .iter()
            .map(|w| MatchCandidate { work: w.clone() })
            .collect();

        // 7. Phase 1: Match feed items against works
        let mut scored_matches: Vec<ScoredMatch> = Vec::new();

        for (indexer, items) in &all_feed_items {
            for item in items {
                // Find best work match for this release
                let mut best_match: Option<(f64, &MatchCandidate)> = None;

                for candidate in &candidates {
                    let score = compute_match_score(&item.title, &candidate.work);
                    if score >= 0.7 {
                        match best_match {
                            Some((best_score, _)) if score > best_score => {
                                best_match = Some((score, candidate));
                            }
                            None => {
                                best_match = Some((score, candidate));
                            }
                            _ => {}
                        }
                    }
                }

                if let Some((score, candidate)) = best_match {
                    scored_matches.push(ScoredMatch {
                        feed_item: item.clone(),
                        work: candidate.work.clone(),
                        score,
                        indexer: indexer.clone(),
                    });
                }
            }
        }

        report.releases_matched = scored_matches.len();

        // 8. Phase 2: Best release per (user, work, media_type)
        // Group by (user_id, work_id) and pick best
        let mut best_per_work: std::collections::HashMap<(UserId, WorkId), ScoredMatch> =
            std::collections::HashMap::new();

        for m in scored_matches {
            let key = (m.work.user_id, m.work.id);
            let dominated = best_per_work.get(&key).is_some_and(|existing| {
                // Higher score wins, then lower priority, then more seeders, then smaller size
                if m.score > existing.score {
                    true
                } else if (m.score - existing.score).abs() < f64::EPSILON {
                    if m.indexer.priority < existing.indexer.priority {
                        true
                    } else if m.indexer.priority == existing.indexer.priority {
                        let m_seeders = m.feed_item.seeders.unwrap_or(0);
                        let e_seeders = existing.feed_item.seeders.unwrap_or(0);
                        if m_seeders > e_seeders {
                            true
                        } else if m_seeders == e_seeders {
                            m.feed_item.size < existing.feed_item.size
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                } else {
                    false
                }
            });

            if dominated || !best_per_work.contains_key(&key) {
                best_per_work.insert(key, m);
            }
        }

        // 9. Grab candidates — filter and grab
        for ((user_id, work_id), m) in best_per_work {
            // Check: active grab exists?
            // Check for ebook monitoring
            if m.work.monitor_ebook {
                let has_grab = self
                    .db
                    .active_grab_exists(user_id, work_id, MediaType::Ebook)
                    .await
                    .map_err(RssSyncError::Db)?;
                if has_grab {
                    continue;
                }

                let has_item = self
                    .db
                    .work_has_library_item(user_id, work_id, MediaType::Ebook)
                    .await
                    .map_err(RssSyncError::Db)?;
                if has_item {
                    continue;
                }
            }

            // Check for audiobook monitoring
            if m.work.monitor_audiobook {
                let has_grab = self
                    .db
                    .active_grab_exists(user_id, work_id, MediaType::Audiobook)
                    .await
                    .map_err(RssSyncError::Db)?;
                if has_grab {
                    continue;
                }

                let has_item = self
                    .db
                    .work_has_library_item(user_id, work_id, MediaType::Audiobook)
                    .await
                    .map_err(RssSyncError::Db)?;
                if has_item {
                    continue;
                }
            }

            report.grabs_attempted += 1;

            let grab_req = GrabRequest {
                work_id,
                download_url: m.feed_item.download_url.clone(),
                title: m.feed_item.title.clone(),
                indexer: m.indexer.name.clone(),
                guid: m.feed_item.guid.clone(),
                size: m.feed_item.size,
                protocol: if m.indexer.protocol == "usenet" {
                    DownloadProtocol::Usenet
                } else {
                    DownloadProtocol::Torrent
                },
                categories: m.feed_item.categories.clone(),
                download_client_id: None,
                source: GrabSource::RssSync,
            };

            match self.release_service.grab(user_id, grab_req).await {
                Ok(_grab) => {
                    report.grabs_succeeded += 1;

                    // Notification: RssGrabbed
                    let _ = self
                        .db
                        .create_notification(CreateNotificationDbRequest {
                            user_id,
                            notification_type: NotificationType::RssGrabbed,
                            ref_key: Some(m.feed_item.guid.clone()),
                            message: format!(
                                "RSS auto-grabbed '{}' for '{}'",
                                m.feed_item.title, m.work.title
                            ),
                            data: serde_json::json!({
                                "work_id": work_id,
                                "title": m.feed_item.title,
                                "indexer": m.indexer.name,
                            }),
                        })
                        .await;
                }
                Err(e) => {
                    report
                        .warnings
                        .push(format!("Grab failed for '{}': {}", m.feed_item.title, e));

                    // Notification: RssGrabFailed
                    let _ = self
                        .db
                        .create_notification(CreateNotificationDbRequest {
                            user_id,
                            notification_type: NotificationType::RssGrabFailed,
                            ref_key: Some(m.feed_item.guid.clone()),
                            message: format!("RSS grab failed for '{}': {}", m.feed_item.title, e),
                            data: serde_json::json!({
                                "work_id": work_id,
                                "title": m.feed_item.title,
                                "indexer": m.indexer.name,
                                "error": e.to_string(),
                            }),
                        })
                        .await;
                }
            }
        }

        Ok(report)
    }
}

/// Build RSS feed URL for an indexer.
fn build_rss_url(indexer: &Indexer) -> String {
    let base = indexer.url.trim_end_matches('/');
    let api_path = indexer.api_path.trim_start_matches('/');
    let mut url = format!("{}/{}", base, api_path);

    // Add API key if present
    if let Some(ref key) = indexer.api_key {
        if url.contains('?') {
            url.push_str(&format!("&apikey={}", key));
        } else {
            url.push_str(&format!("?apikey={}", key));
        }
    }

    // Add RSS mode parameter
    if url.contains('?') {
        url.push_str("&t=search&cat=");
    } else {
        url.push_str("?t=search&cat=");
    }

    // Add categories
    let cats: Vec<String> = indexer.categories.iter().map(|c| c.to_string()).collect();
    url.push_str(&cats.join(","));

    url
}

/// Parse RSS/XML feed body into items. Minimal XML parsing.
fn parse_rss_feed(body: &[u8]) -> Vec<RssFeedItem> {
    let text = match std::str::from_utf8(body) {
        Ok(t) => t,
        Err(_) => return vec![],
    };

    let mut items = Vec::new();

    // Simple line-based XML parsing for RSS <item> elements
    // This is intentionally simple — production would use quick-xml
    let mut in_item = false;
    let mut title = String::new();
    let mut guid = String::new();
    let mut link = String::new();
    let mut size: i64 = 0;
    let mut pub_date = String::new();

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("<item") {
            in_item = true;
            title.clear();
            guid.clear();
            link.clear();
            size = 0;
            pub_date.clear();
        } else if trimmed == "</item>" && in_item {
            in_item = false;
            if !title.is_empty() && !guid.is_empty() {
                items.push(RssFeedItem {
                    title: title.clone(),
                    guid: guid.clone(),
                    download_url: if link.is_empty() {
                        guid.clone()
                    } else {
                        link.clone()
                    },
                    size,
                    seeders: None,
                    publish_date: if pub_date.is_empty() {
                        None
                    } else {
                        Some(pub_date.clone())
                    },
                    categories: vec![],
                });
            }
        } else if in_item {
            if let Some(content) = extract_xml_text(trimmed, "title") {
                title = content;
            } else if let Some(content) = extract_xml_text(trimmed, "guid") {
                guid = content;
            } else if let Some(content) = extract_xml_text(trimmed, "link") {
                link = content;
            } else if let Some(content) = extract_xml_text(trimmed, "pubDate") {
                pub_date = content;
            } else if trimmed.contains("length=") {
                // <enclosure length="12345" .../>
                if let Some(len_str) = extract_attr(trimmed, "length") {
                    size = len_str.parse().unwrap_or(0);
                }
            } else if let Some(content) = extract_xml_text(trimmed, "size") {
                size = content.parse().unwrap_or(0);
            }
        }
    }

    items
}

/// Extract text content between <tag>...</tag>.
fn extract_xml_text(line: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    if let Some(start) = line.find(&open) {
        if let Some(end) = line.find(&close) {
            let content_start = start + open.len();
            if content_start < end {
                return Some(line[content_start..end].to_string());
            }
        }
    }
    None
}

/// Extract attribute value from an XML element.
fn extract_attr(line: &str, attr: &str) -> Option<String> {
    let pattern = format!("{}=\"", attr);
    if let Some(start) = line.find(&pattern) {
        let val_start = start + pattern.len();
        if let Some(end) = line[val_start..].find('"') {
            return Some(line[val_start..val_start + end].to_string());
        }
    }
    None
}

/// Compute a simple title match score between a release title and a work.
/// Returns 0.0 to 1.0.
fn compute_match_score(release_title: &str, work: &Work) -> f64 {
    let release_lower = release_title.to_lowercase();
    let work_title_lower = work.title.to_lowercase();
    let author_lower = work.author_name.to_lowercase();

    // Normalize: remove punctuation for comparison
    let release_words: Vec<&str> = release_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    let title_words: Vec<&str> = work_title_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();

    if title_words.is_empty() {
        return 0.0;
    }

    // Count how many title words appear in release
    let title_hits = title_words
        .iter()
        .filter(|tw| release_words.contains(tw))
        .count();
    let title_score = title_hits as f64 / title_words.len() as f64;

    // Bonus for author name match
    let author_words: Vec<&str> = author_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect();
    let author_hits = if author_words.is_empty() {
        0
    } else {
        author_words
            .iter()
            .filter(|aw| release_words.contains(aw))
            .count()
    };
    let author_score = if author_words.is_empty() {
        0.0
    } else {
        author_hits as f64 / author_words.len() as f64
    };

    // Weighted combination: 70% title, 30% author
    title_score * 0.7 + author_score * 0.3
}
