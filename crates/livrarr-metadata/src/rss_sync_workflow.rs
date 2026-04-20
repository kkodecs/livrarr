use livrarr_db::{
    ConfigDb, CreateNotificationDbRequest, DownloadClientDb, GrabDb, IndexerDb, LibraryItemDb,
    NotificationDb, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::*;
use livrarr_matching::MatchCandidate as M4Candidate;
use livrarr_matching::MatchProvider;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

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
    D: IndexerDb
        + WorkDb
        + GrabDb
        + LibraryItemDb
        + NotificationDb
        + ConfigDb
        + DownloadClientDb
        + Send
        + Sync
        + 'static,
    H: HttpFetcher + Send + Sync + 'static,
    R: ReleaseService + Send + Sync + 'static,
{
    async fn run_sync(&self) -> Result<RssSyncReport, RssSyncError> {
        let config = self
            .db
            .get_indexer_config()
            .await
            .map_err(RssSyncError::Db)?;

        let media_mgmt = self
            .db
            .get_media_management_config()
            .await
            .map_err(RssSyncError::Db)?;

        let indexers = self
            .db
            .list_enabled_rss_indexers()
            .await
            .map_err(RssSyncError::Db)?;

        if indexers.is_empty() {
            return Ok(RssSyncReport::empty());
        }

        info!("RSS sync: starting");

        let mut report = RssSyncReport::empty();

        // Pre-query RSS state for first-sync detection.
        let mut pre_states: HashMap<i64, Option<IndexerRssState>> = HashMap::new();
        for indexer in &indexers {
            let state = self
                .db
                .get_rss_state(indexer.id)
                .await
                .map_err(RssSyncError::Db)?;
            pre_states.insert(indexer.id, state);
        }

        let first_sync_ids: HashSet<i64> = pre_states
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| *k)
            .collect();

        // Fetch RSS feeds.
        debug!("RSS sync: fetching from {} indexers", indexers.len());
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
                }
                Ok(resp) => {
                    report.feeds_checked += 1;
                    let items = parse_rss_feed(&resp.body);
                    debug!(
                        "RSS sync: fetched {} releases from {}",
                        items.len(),
                        indexer.name
                    );
                    all_feed_items.push((indexer.clone(), items));
                }
                Err(e) => {
                    report
                        .warnings
                        .push(format!("Indexer '{}' fetch failed: {}", indexer.name, e));
                }
            }
        }

        // Counters.
        let mut n_fetched: usize = 0;
        let mut n_parsed: usize = 0;
        let mut n_unparsed: usize = 0;
        let mut n_grabbed: usize = 0;
        let mut n_skipped: usize = 0;

        // Gap detection + state update per indexer.
        for (indexer, items) in &all_feed_items {
            n_fetched += items.len();
            let existing_state = pre_states.get(&indexer.id).and_then(|s| s.as_ref());
            let guidful: Vec<&RssFeedItem> = items.iter().filter(|r| !r.guid.is_empty()).collect();

            let mut dated_items: Vec<&RssFeedItem> = guidful
                .iter()
                .filter(|r| r.publish_date.is_some())
                .copied()
                .collect();
            dated_items.sort_by(|a, b| {
                parse_rfc2822_or_epoch(b.publish_date.as_deref().unwrap_or("")).cmp(
                    &parse_rfc2822_or_epoch(a.publish_date.as_deref().unwrap_or("")),
                )
            });

            // Gap detection (RSS-GAP-001).
            if let Some(es) = existing_state {
                if let (Some(ref stored_date), Some(oldest_dated)) =
                    (&es.last_publish_date, dated_items.last())
                {
                    if let Some(ref oldest_pub) = oldest_dated.publish_date {
                        if parse_rfc2822_or_epoch(oldest_pub) > parse_rfc2822_or_epoch(stored_date)
                        {
                            let stored_guid = es.last_guid.as_deref().unwrap_or("");
                            let guid_in_batch = guidful.iter().any(|r| r.guid == stored_guid);
                            if !guid_in_batch {
                                warn!(
                                    "RSS sync: gap detected for indexer {} — oldest item {} > stored {}",
                                    indexer.name, oldest_pub, stored_date
                                );
                            }
                        }
                    }
                }
            }

            // Update state to newest dated item.
            if let Some(newest) = dated_items.first() {
                let _ = self
                    .db
                    .upsert_rss_state(indexer.id, newest.publish_date.as_deref(), &newest.guid)
                    .await;
            } else if !guidful.is_empty() && existing_state.is_none() {
                let _ = self
                    .db
                    .upsert_rss_state(indexer.id, None, &guidful[0].guid)
                    .await;
            }

            if first_sync_ids.contains(&indexer.id) {
                info!(
                    "RSS sync: first sync for indexer {} — recording state, no grabs",
                    indexer.name
                );
            }
        }

        // Load monitored works.
        let monitored_works = self
            .db
            .list_monitored_works_all_users()
            .await
            .map_err(RssSyncError::Db)?;

        if monitored_works.is_empty() {
            info!("RSS sync: {n_fetched} releases, 0 matched (no monitored works)");
            return Ok(report);
        }

        // Build M4 candidates from works.
        let candidates: Vec<(Work, M4Candidate)> = monitored_works
            .iter()
            .map(|w| (w.clone(), work_to_candidate(w)))
            .collect();

        // Pre-check protocol availability (RSS-GRAB-003).
        let has_torrent_client = self
            .db
            .get_default_download_client("qbittorrent")
            .await
            .map(|c| c.is_some())
            .unwrap_or(false);
        let has_usenet_client = self
            .db
            .get_default_download_client("sabnzbd")
            .await
            .map(|c| c.is_some())
            .unwrap_or(false);

        let threshold = config.rss_match_threshold;
        let mut n_matched: usize = 0;

        // Phase 1: Per-release, find best work per (user, media_type).
        struct ReleaseMatch {
            user_id: i64,
            work_id: i64,
            media_type: MediaType,
            score: f64,
            indexer_priority: i32,
            feed_item: RssFeedItem,
            indexer_name: String,
            indexer_protocol: String,
        }
        let mut release_matches: Vec<ReleaseMatch> = Vec::new();

        for (indexer, items) in &all_feed_items {
            if first_sync_ids.contains(&indexer.id) {
                continue;
            }

            for item in items {
                if item.guid.is_empty() {
                    continue;
                }

                // RSS-FILTER-001: media types from categories.
                let media_types = media_types_from_categories(&item.categories);
                if media_types.is_empty() {
                    continue;
                }

                // RSS-GRAB-003: protocol eligibility.
                let protocol_eligible = match indexer.protocol.as_str() {
                    "usenet" => has_usenet_client,
                    _ => has_torrent_client,
                };
                if !protocol_eligible {
                    continue;
                }

                // Parse release title via M3.
                let parsed = livrarr_matching::parse_release_title(&item.title);
                if parsed.extractions.is_empty() {
                    n_unparsed += 1;
                    continue;
                }
                n_parsed += 1;

                // RSS-FILTER-005: format preference check per media type.
                let format_lower = parsed.format.as_deref().map(|f| f.to_lowercase());
                let mut format_eligible_types: Vec<MediaType> = Vec::new();
                for mt in &media_types {
                    let prefs = match mt {
                        MediaType::Ebook => &media_mgmt.preferred_ebook_formats,
                        MediaType::Audiobook => &media_mgmt.preferred_audiobook_formats,
                    };
                    if let Some(ref fmt) = format_lower {
                        if !prefs.is_empty() && !prefs.iter().any(|p| p.eq_ignore_ascii_case(fmt)) {
                            continue;
                        }
                    }
                    format_eligible_types.push(*mt);
                }
                if format_eligible_types.is_empty() {
                    n_skipped += 1;
                    continue;
                }

                // For each media type, find best work per user.
                for mt in &format_eligible_types {
                    let mut best_per_user: HashMap<i64, (i64, f64)> = HashMap::new();

                    for (work, cand) in &candidates {
                        let monitored_for_type = match mt {
                            MediaType::Ebook => work.monitor_ebook,
                            MediaType::Audiobook => work.monitor_audiobook,
                        };
                        if !monitored_for_type {
                            continue;
                        }

                        // RSS-FILTER-004: skip releases published before work was added.
                        if let Some(ref pub_date_str) = item.publish_date {
                            let published = chrono::DateTime::parse_from_rfc2822(pub_date_str)
                                .or_else(|_| chrono::DateTime::parse_from_rfc3339(pub_date_str));
                            if let Ok(pub_dt) = published {
                                if pub_dt < work.added_at {
                                    continue;
                                }
                            }
                        }

                        let best_score = livrarr_matching::best_match_score(&parsed, cand);

                        if best_score < threshold {
                            continue;
                        }

                        // RSS-MATCH-001: best score wins. Tie: lower work_id.
                        let is_better = match best_per_user.get(&work.user_id) {
                            None => true,
                            Some(&(existing_wid, existing_score)) => {
                                (best_score, std::cmp::Reverse(work.id))
                                    > (existing_score, std::cmp::Reverse(existing_wid))
                            }
                        };

                        if is_better {
                            best_per_user.insert(work.user_id, (work.id, best_score));
                        }
                    }

                    for (uid, (wid, score)) in &best_per_user {
                        n_matched += 1;
                        release_matches.push(ReleaseMatch {
                            user_id: *uid,
                            work_id: *wid,
                            media_type: *mt,
                            score: *score,
                            indexer_priority: indexer.priority,
                            feed_item: item.clone(),
                            indexer_name: indexer.name.clone(),
                            indexer_protocol: indexer.protocol.clone(),
                        });
                    }
                }
            }
        }

        debug!(
            "RSS sync: phase 1 complete — {} release-work matches",
            n_matched
        );

        // Phase 2: Best release per (user, work, media_type) (RSS-MATCH-002).
        struct GrabCandidate {
            user_id: i64,
            work_id: i64,
            media_type: MediaType,
            score: f64,
            indexer_priority: i32,
            feed_item: RssFeedItem,
            indexer_name: String,
            indexer_protocol: String,
        }

        let mut best_map: HashMap<(i64, i64, &str), GrabCandidate> = HashMap::new();

        for rm in release_matches {
            let mt_str = match rm.media_type {
                MediaType::Ebook => "ebook",
                MediaType::Audiobook => "audiobook",
            };
            let key = (rm.user_id, rm.work_id, mt_str);
            let seeders = rm.feed_item.seeders.unwrap_or(0);

            // RSS-MATCH-002: score desc, priority asc, seeders desc, size asc.
            let is_better = match best_map.get(&key) {
                None => true,
                Some(existing) => {
                    let e_seeders = existing.feed_item.seeders.unwrap_or(0);
                    (
                        rm.score,
                        std::cmp::Reverse(rm.indexer_priority),
                        seeders,
                        std::cmp::Reverse(rm.feed_item.size),
                    ) > (
                        existing.score,
                        std::cmp::Reverse(existing.indexer_priority),
                        e_seeders,
                        std::cmp::Reverse(existing.feed_item.size),
                    )
                }
            };

            if is_better {
                best_map.insert(
                    key,
                    GrabCandidate {
                        user_id: rm.user_id,
                        work_id: rm.work_id,
                        media_type: rm.media_type,
                        score: rm.score,
                        indexer_priority: rm.indexer_priority,
                        feed_item: rm.feed_item,
                        indexer_name: rm.indexer_name,
                        indexer_protocol: rm.indexer_protocol,
                    },
                );
            }
        }

        debug!(
            "RSS sync: phase 2 complete — {} grab candidates after dedup",
            best_map.len()
        );

        // Filter and grab.
        for gc in best_map.into_values() {
            // RSS-FILTER-002: active grab or library item exists?
            match self
                .db
                .active_grab_exists(gc.user_id, gc.work_id, gc.media_type)
                .await
            {
                Ok(true) => {
                    n_skipped += 1;
                    continue;
                }
                Err(e) => {
                    warn!("RSS sync: active_grab_exists error: {e}");
                    n_skipped += 1;
                    continue;
                }
                Ok(false) => {}
            }

            match self
                .db
                .work_has_library_item(gc.user_id, gc.work_id, gc.media_type)
                .await
            {
                Ok(true) => {
                    n_skipped += 1;
                    continue;
                }
                Err(e) => {
                    warn!("RSS sync: work_has_library_item error: {e}");
                    n_skipped += 1;
                    continue;
                }
                Ok(false) => {}
            }

            debug!(
                "RSS sync: grabbing '{}' for work {} via {}",
                gc.feed_item.title, gc.work_id, gc.indexer_name
            );

            report.grabs_attempted += 1;

            let grab_req = GrabRequest {
                work_id: gc.work_id,
                download_url: gc.feed_item.download_url.clone(),
                title: gc.feed_item.title.clone(),
                indexer: gc.indexer_name.clone(),
                guid: gc.feed_item.guid.clone(),
                size: gc.feed_item.size,
                protocol: if gc.indexer_protocol == "usenet" {
                    DownloadProtocol::Usenet
                } else {
                    DownloadProtocol::Torrent
                },
                categories: gc.feed_item.categories.clone(),
                download_client_id: None,
                source: GrabSource::RssSync,
            };

            match self.release_service.grab(gc.user_id, grab_req).await {
                Ok(_) => {
                    n_grabbed += 1;
                    report.grabs_succeeded += 1;

                    let _ = self
                        .db
                        .create_notification(CreateNotificationDbRequest {
                            user_id: gc.user_id,
                            notification_type: NotificationType::RssGrabbed,
                            ref_key: Some(format!("rss:{}", gc.feed_item.guid)),
                            message: format!(
                                "RSS grabbed: {} (score {:.2})",
                                gc.feed_item.title, gc.score
                            ),
                            data: serde_json::json!({
                                "title": gc.feed_item.title,
                                "indexer": gc.indexer_name,
                                "score": gc.score,
                                "workId": gc.work_id,
                            }),
                        })
                        .await;
                }
                Err(e) => {
                    warn!(
                        "RSS sync: grab failed for '{}': {:?}",
                        gc.feed_item.title, e
                    );

                    let _ = self
                        .db
                        .create_notification(CreateNotificationDbRequest {
                            user_id: gc.user_id,
                            notification_type: NotificationType::RssGrabFailed,
                            ref_key: Some(format!("rss-fail:{}", gc.feed_item.guid)),
                            message: format!("RSS grab failed: {}", gc.feed_item.title),
                            data: serde_json::json!({
                                "title": gc.feed_item.title,
                                "indexer": gc.indexer_name,
                                "error": format!("{:?}", e),
                                "workId": gc.work_id,
                            }),
                        })
                        .await;

                    n_skipped += 1;
                }
            }
        }

        report.releases_matched = n_matched;

        info!(
            "RSS sync: {n_fetched} releases, {n_parsed} parsed, {n_unparsed} unparseable, \
             {n_matched} matched, {n_grabbed} grabbed, {n_skipped} filtered"
        );

        Ok(report)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn work_to_candidate(work: &Work) -> M4Candidate {
    M4Candidate {
        title: work.title.clone(),
        author: work.author_name.clone(),
        year: work.year,
        work_key: String::new(),
        author_key: None,
        cover_url: None,
        series: work.series_name.clone(),
        series_position: work.series_position,
        provider: MatchProvider::OpenLibrary,
        score: 0.0,
    }
}

fn media_types_from_categories(categories: &[i32]) -> Vec<MediaType> {
    let mut types = Vec::new();
    if categories.contains(&7020) {
        types.push(MediaType::Ebook);
    }
    if categories.contains(&3030) {
        types.push(MediaType::Audiobook);
    }
    types
}

fn parse_rfc2822_or_epoch(s: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc2822(s)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or(chrono::DateTime::<chrono::Utc>::MIN_UTC)
}

fn build_rss_url(indexer: &Indexer) -> String {
    let base = indexer.url.trim_end_matches('/');
    let api_path = indexer.api_path.trim_start_matches('/');
    let mut url = format!("{}/{}", base, api_path);

    if let Some(ref key) = indexer.api_key {
        if url.contains('?') {
            url.push_str(&format!("&apikey={}", key));
        } else {
            url.push_str(&format!("?apikey={}", key));
        }
    }

    if url.contains('?') {
        url.push_str("&t=search&cat=");
    } else {
        url.push_str("?t=search&cat=");
    }

    let cats: Vec<String> = indexer.categories.iter().map(|c| c.to_string()).collect();
    url.push_str(&cats.join(","));

    url
}

fn parse_rss_feed(body: &[u8]) -> Vec<RssFeedItem> {
    use livrarr_domain::torznab::{parse_torznab_xml, TorznabParseResult};

    let parse_result = match parse_torznab_xml(body) {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    let items = match parse_result {
        TorznabParseResult::Items(items) => items,
        TorznabParseResult::Error { .. } => return vec![],
    };
    items
        .into_iter()
        .filter(|item| !item.title.is_empty() && !item.guid.is_empty())
        .map(|item| RssFeedItem {
            title: item.title,
            guid: item.guid.clone(),
            download_url: if item.download_url.is_empty() {
                item.guid
            } else {
                item.download_url
            },
            size: item.size,
            seeders: item.seeders,
            publish_date: item.publish_date,
            categories: item.categories,
        })
        .collect()
}
