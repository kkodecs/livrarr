use std::collections::HashSet;
use std::time::Duration;

use livrarr_db::{
    CreateGrabDbRequest, CreateHistoryEventDbRequest, DownloadClientDb, GrabDb, HistoryDb,
    IndexerDb, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::*;

use crate::release_search_cache::ReleaseSearchCache;

/// ReleaseService implementation — search indexers and grab releases.
pub struct ReleaseServiceImpl<D, H> {
    db: D,
    http: H,
    trusted_origins: std::sync::Arc<livrarr_http::ssrf::TrustedOrigins>,
    cache: ReleaseSearchCache,
}

impl<D, H> ReleaseServiceImpl<D, H> {
    pub fn new(
        db: D,
        http: H,
        trusted_origins: std::sync::Arc<livrarr_http::ssrf::TrustedOrigins>,
    ) -> Self {
        Self {
            db,
            http,
            trusted_origins,
            cache: ReleaseSearchCache::new(),
        }
    }
}

/// Derive media type from Torznab categories.
/// 7xxx = ebook, 3xxx = audiobook.
pub fn derive_media_type_from_categories(categories: &[i32]) -> Option<MediaType> {
    for cat in categories {
        let series = *cat / 1000;
        if series == 7 {
            return Some(MediaType::Ebook);
        }
        if series == 3 {
            return Some(MediaType::Audiobook);
        }
    }
    None
}

use livrarr_domain::torznab::{parse_torznab_xml, TorznabParseResult};

impl<D, H> ReleaseService for ReleaseServiceImpl<D, H>
where
    D: IndexerDb + WorkDb + DownloadClientDb + GrabDb + HistoryDb + Clone + Send + Sync + 'static,
    H: HttpFetcher + Clone + Send + Sync + 'static,
{
    async fn search(
        &self,
        user_id: UserId,
        req: SearchReleasesRequest,
    ) -> Result<ReleaseSearchResponse, ReleaseServiceError> {
        // Get work for search query
        let work = self
            .db
            .get_work(user_id, req.work_id)
            .await
            .map_err(|e| match e {
                DbError::NotFound { .. } => ReleaseServiceError::Db(e),
                other => ReleaseServiceError::Db(other),
            })?;

        // Get enabled indexers
        let indexers = self
            .db
            .list_enabled_interactive_indexers()
            .await
            .map_err(ReleaseServiceError::Db)?;

        if indexers.is_empty() {
            return Err(ReleaseServiceError::AllIndexersFailed);
        }

        let last_name = work
            .author_name
            .split_whitespace()
            .last()
            .unwrap_or("")
            .to_string();
        let query = if last_name.is_empty() {
            work.title.clone()
        } else {
            format!("{} {}", work.title, last_name)
        };

        let mut all_results: Vec<ReleaseResult> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut any_success = false;
        let mut any_live_attempted = false;
        let mut oldest_cache_age: Option<u64> = None;

        // Per indexer: `refresh` always goes live and rewrites the cache;
        // otherwise a fresh (<24h) cache entry is served with zero HTTP. A
        // miss goes live UNLESS `cache_only`, in which case that indexer
        // contributes nothing — silently, since a cold cache is the normal
        // day-one state, not an error.
        let mut live_indexers: Vec<&Indexer> = Vec::new();
        for indexer in &indexers {
            if !req.refresh {
                if let Some((cached, age)) = self
                    .cache
                    .get(&work.title, &work.author_name, indexer.id)
                    .await
                {
                    any_success = true;
                    oldest_cache_age = Some(oldest_cache_age.map_or(age, |a| a.max(age)));
                    all_results.extend(cached);
                    continue;
                }
            }

            if req.cache_only {
                continue;
            }

            live_indexers.push(indexer);
        }

        if !live_indexers.is_empty() {
            any_live_attempted = true;

            // Fan-out parallel requests with per-indexer 30s timeout
            let mut handles = tokio::task::JoinSet::new();

            for indexer in &live_indexers {
                let db = self.db.clone();
                let http = self.http.clone();
                let query = query.clone();
                let indexer_id = indexer.id;
                let indexer_name = indexer.name.clone();

                handles.spawn(async move {
                    // Fetch indexer with credentials for the API key
                    let indexer = match db.get_indexer_with_credentials(indexer_id).await {
                        Ok(i) => i,
                        Err(e) => {
                            return (
                                indexer_name,
                                indexer_id,
                                Err(format!("failed to load indexer: {e}")),
                            );
                        }
                    };

                    // Build Torznab search URL
                    let base_url = indexer.url.trim_end_matches('/');
                    let api_path = indexer.api_path.trim_start_matches('/');
                    let mut url = format!(
                        "{base_url}/{api_path}?t=search&q={}",
                        urlencoding::encode(&query)
                    );
                    if let Some(ref api_key) = indexer.api_key {
                        url.push_str(&format!("&apikey={}", urlencoding::encode(api_key)));
                    }
                    // Add categories
                    if !indexer.categories.is_empty() {
                        let cats: Vec<String> =
                            indexer.categories.iter().map(|c| c.to_string()).collect();
                        url.push_str(&format!("&cat={}", cats.join(",")));
                    }

                    let origin = livrarr_http::normalized_origin(&indexer.url)
                        .unwrap_or_else(|| indexer.url.clone());
                    let fetch_req = FetchRequest {
                        url,
                        method: HttpMethod::Get,
                        headers: vec![],
                        body: None,
                        timeout: Duration::from_secs(30),
                        rate_bucket: RateBucket::Indexer(origin),
                        max_body_bytes: 10 * 1024 * 1024,
                        anti_bot_check: false,
                        user_agent: UserAgentProfile::Server,
                        priority: RequestPriority::Normal,
                    };

                    match http.fetch(fetch_req).await {
                        Ok(resp) if resp.status == 200 => {
                            (indexer_name, indexer_id, Ok::<Vec<u8>, String>(resp.body))
                        }
                        Ok(resp) => (
                            indexer_name,
                            indexer_id,
                            Err::<Vec<u8>, String>(format!("HTTP {}", resp.status)),
                        ),
                        Err(e) => (
                            indexer_name,
                            indexer_id,
                            Err::<Vec<u8>, String>(format!("{e}")),
                        ),
                    }
                });
            }

            while let Some(join_result) = handles.join_next().await {
                match join_result {
                    Ok((indexer_name, indexer_id, Ok(body))) => {
                        any_success = true;
                        match parse_torznab_xml(&body) {
                            Ok(TorznabParseResult::Items(items)) => {
                                for item in &items {
                                    if item.guid.is_empty() {
                                        warnings.push(format!(
                                            "indexer {indexer_name}: skipped item missing guid (title: {})",
                                            if item.title.is_empty() { "<unknown>" } else { &item.title }
                                        ));
                                    } else if item.download_url.is_empty() {
                                        warnings.push(format!(
                                            "indexer {indexer_name}: skipped item missing downloadUrl (guid: {})",
                                            item.guid
                                        ));
                                    }
                                }
                                let results: Vec<ReleaseResult> = items
                                    .into_iter()
                                    .filter(|item| {
                                        !item.guid.is_empty() && !item.download_url.is_empty()
                                    })
                                    .map(|item| {
                                        let protocol = if item
                                            .enclosure_type
                                            .as_deref()
                                            .is_some_and(|t| t.contains("nzb"))
                                        {
                                            DownloadProtocol::Usenet
                                        } else {
                                            DownloadProtocol::Torrent
                                        };
                                        ReleaseResult {
                                            title: item.title,
                                            indexer: indexer_name.to_string(),
                                            size: item.size,
                                            guid: item.guid,
                                            download_url: item.download_url,
                                            seeders: item.seeders,
                                            leechers: item.leechers,
                                            publish_date: item.publish_date,
                                            protocol,
                                            categories: item.categories,
                                        }
                                    })
                                    .collect();
                                self.cache
                                    .put(
                                        &work.title,
                                        &work.author_name,
                                        indexer_id,
                                        results.clone(),
                                    )
                                    .await;
                                all_results.extend(results);
                            }
                            Ok(TorznabParseResult::Error { code, description }) => {
                                warnings.push(redact_secrets(&format!(
                                    "indexer {indexer_name}: error {code}: {description}"
                                )));
                            }
                            Err(e) => {
                                warnings
                                    .push(redact_secrets(&format!("indexer {indexer_name}: {e}")));
                            }
                        }
                    }
                    Ok((indexer_name, _indexer_id, Err(err_msg))) => {
                        warnings.push(redact_secrets(&format!(
                            "indexer {indexer_name}: {err_msg}"
                        )));
                    }
                    Err(join_err) => {
                        warnings.push(format!("indexer task panicked: {join_err}"));
                    }
                }
            }
        }

        // `AllIndexersFailed` is reserved for a mode that actually attempted
        // live fetches and had every attempt fail — never for a mode (like a
        // cold `cache_only` search) that contacted nobody.
        if !any_success && any_live_attempted {
            return Err(ReleaseServiceError::AllIndexersFailed);
        }

        // Dedup by (guid, indexer)
        let mut seen = HashSet::new();
        all_results.retain(|r| seen.insert((r.guid.clone(), r.indexer.clone())));

        // Sort: torrent by seeders desc, usenet by age asc, within tie by size desc
        all_results.sort_by(|a, b| {
            match (&a.protocol, &b.protocol) {
                (DownloadProtocol::Torrent, DownloadProtocol::Torrent) => {
                    // Seeders desc, then size desc
                    let sa = a.seeders.unwrap_or(0);
                    let sb = b.seeders.unwrap_or(0);
                    sb.cmp(&sa).then_with(|| b.size.cmp(&a.size))
                }
                (DownloadProtocol::Usenet, DownloadProtocol::Usenet) => {
                    // Age asc (newer first = publish_date desc), then size desc
                    let pa = a.publish_date.as_deref().unwrap_or("");
                    let pb = b.publish_date.as_deref().unwrap_or("");
                    pb.cmp(pa).then_with(|| b.size.cmp(&a.size))
                }
                (DownloadProtocol::Torrent, DownloadProtocol::Usenet) => std::cmp::Ordering::Less,
                (DownloadProtocol::Usenet, DownloadProtocol::Torrent) => {
                    std::cmp::Ordering::Greater
                }
            }
        });

        Ok(ReleaseSearchResponse {
            results: all_results,
            warnings,
            cache_age_seconds: oldest_cache_age,
            search_query: query,
        })
    }

    async fn grab(&self, user_id: UserId, req: GrabRequest) -> Result<Grab, ReleaseServiceError> {
        // SSRF validation: allow private IPs only for trusted origins
        // (user-configured indexers and download clients).
        if !self.trusted_origins.is_trusted(&req.download_url) {
            if let Err(e) = livrarr_http::ssrf::validate_url(&req.download_url).await {
                return Err(ReleaseServiceError::Ssrf(format!(
                    "download URL blocked: {e}"
                )));
            }
        }

        // Indexer-origin fetches during this grab (torrent-file, NZB,
        // Transmission URL fallback, magnet-redirect probe) key their pace
        // lane and cooldown on the CONFIGURED indexer's origin — looked up
        // by `req.indexer` (display name) — not on the download URL's host,
        // so an indexer that proxies or redirects through a different host
        // cannot evade its own bucket. Falls back to the download URL's own
        // origin when the name matches no configured indexer, so an ad-hoc
        // or renamed indexer still gets the grab instead of failing it.
        let configured_indexer = self
            .db
            .list_indexers()
            .await
            .ok()
            .and_then(|indexers| indexers.into_iter().find(|i| i.name == req.indexer));
        let indexer_origin = match configured_indexer {
            Some(indexer) => {
                livrarr_http::normalized_origin(&indexer.url).unwrap_or_else(|| indexer.url.clone())
            }
            None => livrarr_http::normalized_origin(&req.download_url)
                .unwrap_or_else(|| req.download_url.clone()),
        };

        // Determine client_type from protocol.
        // For Torrent, prefer the is_default_for_protocol client (could be qBittorrent or Transmission).
        let client_type = match req.protocol {
            DownloadProtocol::Torrent => "qbittorrent",
            DownloadProtocol::Usenet => "sabnzbd",
        };

        // Get download client
        let client = if let Some(client_id) = req.download_client_id {
            let c = self
                .db
                .get_download_client_with_credentials(client_id)
                .await
                .map_err(|e| match e {
                    DbError::NotFound { .. } => ReleaseServiceError::NoClient {
                        protocol: client_type.to_string(),
                    },
                    other => ReleaseServiceError::Db(other),
                })?;

            // Verify protocol match
            if c.implementation.protocol() != req.protocol {
                return Err(ReleaseServiceError::ClientProtocolMismatch {
                    protocol: protocol_str(&req.protocol).to_string(),
                });
            }
            c
        } else {
            // Use default client for protocol. For Torrent, check both qBittorrent and Transmission.
            let mut found = self
                .db
                .get_default_download_client(client_type)
                .await
                .map_err(ReleaseServiceError::Db)?;

            if found.is_none() && req.protocol == DownloadProtocol::Torrent {
                found = self
                    .db
                    .get_default_download_client("transmission")
                    .await
                    .map_err(ReleaseServiceError::Db)?;
            }

            found.ok_or_else(|| ReleaseServiceError::NoClient {
                protocol: protocol_str(&req.protocol).to_string(),
            })?
        };

        // Dispatch to download client via HTTP
        let dispatch_result = match req.protocol {
            DownloadProtocol::Torrent => match client.implementation {
                DownloadClientImplementation::Transmission => {
                    dispatch_transmission(&self.http, &client, &req.download_url, &indexer_origin)
                        .await
                }
                _ => {
                    dispatch_torrent(&self.http, &client, &req.download_url, &indexer_origin).await
                }
            },
            DownloadProtocol::Usenet => {
                dispatch_usenet(
                    &self.http,
                    &client,
                    &req.download_url,
                    &req.title,
                    &indexer_origin,
                )
                .await
            }
        };

        let download_id = match dispatch_result {
            Ok(id) => id,
            Err(e) => {
                return Err(ReleaseServiceError::ClientUnreachable(e));
            }
        };

        // Derive media type from categories
        let media_type = derive_media_type_from_categories(&req.categories);

        // Create grab record AFTER client confirms
        let grab = self
            .db
            .upsert_grab(CreateGrabDbRequest {
                user_id,
                work_id: req.work_id,
                download_client_id: client.id,
                title: req.title.clone(),
                indexer: req.indexer.clone(),
                guid: req.guid.clone(),
                size: Some(req.size),
                download_url: req.download_url.clone(),
                download_id: Some(download_id),
                status: GrabStatus::Sent,
                media_type,
            })
            .await
            .map_err(ReleaseServiceError::Db)?;

        // Create history event
        let _ = self
            .db
            .create_history_event(CreateHistoryEventDbRequest {
                user_id,
                work_id: Some(req.work_id),
                event_type: EventType::Grabbed,
                data: serde_json::json!({
                    "title": req.title,
                    "indexer": req.indexer,
                    "guid": req.guid,
                    "download_client_id": client.id,
                }),
            })
            .await;

        Ok(grab)
    }
}

enum TorrentDispatchSource {
    Magnet(String),
    File(Vec<u8>),
    Url(String),
}

/// Fetch the torrent resource at `download_url` and return both the info-hash
/// (best-effort) and the dispatch source the download client should use.
///
/// - Direct `magnet:` URI → `Magnet` (hash extracted from the URI).
/// - Normal fetch succeeds, body is a `magnet:` URI → `Magnet` (hash from it).
/// - Normal fetch succeeds, body is .torrent bytes → `File` (hash from the file;
///   bytes returned so the client can skip a second fetch).
/// - Normal fetch fails and a redirect-to-magnet is recovered → `Magnet`
///   (hash extracted from the recovered URI; dispatching the magnet avoids
///   re-hitting an indexer endpoint the client may not be able to reach).
/// - Normal fetch fails for any other reason → `Url` (original URL as
///   last-resort fallback, letting the download client fetch it directly).
///
/// `RateLimited`/`CircuitOpen` are NOT degraded to the `Url` fallback: the
/// indexer is cooling down, so this returns `Err` and the caller fails the
/// grab hard instead of handing the client a URL it would hit outside our
/// pacing — a delegated bypass of the cooldown.
async fn fetch_torrent_dispatch_source<H: HttpFetcher>(
    http: &H,
    download_url: &str,
    origin: &str,
) -> Result<(Option<String>, TorrentDispatchSource), FetchError> {
    use crate::{extract_torrent_hash, TorrentSource};

    if download_url.starts_with("magnet:") {
        let hash = extract_torrent_hash(&TorrentSource::Magnet(download_url.to_string())).ok();
        return Ok((
            hash,
            TorrentDispatchSource::Magnet(download_url.to_string()),
        ));
    }

    let resp = match http
        .fetch(FetchRequest {
            url: download_url.to_string(),
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(60),
            rate_bucket: RateBucket::Indexer(origin.to_string()),
            max_body_bytes: 4 * 1024 * 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Normal,
        })
        .await
    {
        Ok(r) => r,
        Err(e @ (FetchError::RateLimited | FetchError::CircuitOpen { .. })) => return Err(e),
        Err(_) => {
            // The fetch may have failed because the URL redirects to a magnet: URI.
            // reqwest's redirect-following client rejects non-HTTP schemes, so a
            // 301/302 Location: magnet:?xt=... response causes an Err rather than a
            // usable redirect. Some indexer proxies (e.g. Prowlarr) use this pattern.
            // Probe with a no-redirect request to recover the magnet if that's the case.
            if let Some(magnet) =
                probe_for_magnet_redirect(http, download_url, origin.to_string()).await
            {
                let hash = extract_torrent_hash(&TorrentSource::Magnet(magnet.clone())).ok();
                return Ok((hash, TorrentDispatchSource::Magnet(magnet)));
            }
            return Ok((None, TorrentDispatchSource::Url(download_url.to_string())));
        }
    };

    if !(200..300).contains(&resp.status) {
        return Ok((None, TorrentDispatchSource::Url(download_url.to_string())));
    }

    // Some indexers return a magnet URI as the response body
    if let Ok(text) = std::str::from_utf8(&resp.body) {
        let trimmed = text.trim();
        if trimmed.starts_with("magnet:") {
            let hash = extract_torrent_hash(&TorrentSource::Magnet(trimmed.to_string())).ok();
            return Ok((hash, TorrentDispatchSource::Magnet(trimmed.to_string())));
        }
    }

    let hash = extract_torrent_hash(&TorrentSource::TorrentFile {
        filename: "download.torrent".to_string(),
        data: resp.body.clone(),
    })
    .ok();
    Ok((hash, TorrentDispatchSource::File(resp.body)))
}

/// Probe a URL with a request whose redirects are NOT auto-followed, and
/// return the `Location` value if the response is a redirect to a `magnet:`
/// URI. Used to recover from redirect-to-magnet responses that cause the
/// normal redirect-following fetch to error on the non-HTTP scheme. Goes
/// through livrarr-http's fetcher (`fetch_no_redirect`) on the same indexer
/// bucket as the primary attempt, so it inherits pacing, cooldown, UA
/// policy, and SSRF posture like every other outbound call — no bespoke
/// `reqwest::Client` here.
async fn probe_for_magnet_redirect<H: HttpFetcher>(
    http: &H,
    url: &str,
    origin: String,
) -> Option<String> {
    let resp = http
        .fetch_no_redirect(FetchRequest {
            url: url.to_string(),
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(10),
            rate_bucket: RateBucket::Indexer(origin),
            max_body_bytes: 4096,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Normal,
        })
        .await
        .ok()?;

    if !(300..400).contains(&resp.status) {
        return None;
    }
    let location = resp
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("location"))
        .map(|(_, v)| v.clone())?;
    if location.starts_with("magnet:") {
        Some(location)
    } else {
        None
    }
}

/// Dispatch torrent to qBittorrent via HTTP API.
///
/// For magnet URIs: sends via `urls=` field (qBit handles natively).
/// For .torrent URLs: fetches the file server-side and sends bytes via
/// `torrents=` multipart field, so qBit doesn't need to reach the indexer.
async fn dispatch_torrent<H: HttpFetcher>(
    http: &H,
    client: &DownloadClient,
    download_url: &str,
    origin: &str,
) -> Result<String, String> {
    let (hash, dispatch_source) = fetch_torrent_dispatch_source(http, download_url, origin)
        .await
        .map_err(|e| format!("indexer cooling down — retry later: {e}"))?;
    let download_id = hash.unwrap_or_else(|| "pending".to_string());

    let scheme = if client.use_ssl { "https" } else { "http" };
    let url_base = client.url_base.as_deref().unwrap_or("");
    let base = format!("{}://{}:{}{}", scheme, client.host, client.port, url_base);

    // First authenticate
    let auth_url = format!("{base}/api/v2/auth/login");
    let auth_body = format!(
        "username={}&password={}",
        urlencoding::encode(client.username.as_deref().unwrap_or("")),
        urlencoding::encode(client.password.as_deref().unwrap_or("")),
    );

    let auth_resp = http
        .fetch(FetchRequest {
            url: auth_url,
            method: HttpMethod::Post,
            headers: vec![(
                "Content-Type".into(),
                "application/x-www-form-urlencoded".into(),
            )],
            body: Some(auth_body.into_bytes()),
            timeout: Duration::from_secs(10),
            rate_bucket: RateBucket::None,
            max_body_bytes: 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Normal,
        })
        .await
        .map_err(|e| format!("qBit auth failed: {e}"))?;

    if !(200..300).contains(&auth_resp.status) {
        return Err("qBit auth failed".to_string());
    }

    let auth_cookie = auth_resp
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
        .and_then(|(_, v)| {
            let cookie = v.split(';').next()?.trim();
            let name = cookie.split('=').next()?;

            if name == "SID" || name == "QBT_SID" || name.starts_with("QBT_SID_") {
                Some(cookie.to_string())
            } else {
                None
            }
        })
        .unwrap_or_default();

    // Build multipart body: use `torrents=` for file bytes, `urls=` for magnets
    let add_url = format!("{base}/api/v2/torrents/add");
    let boundary = "----livrarr-boundary";
    let safe_category = sanitize_multipart_value(&client.category);

    let body = match &dispatch_source {
        TorrentDispatchSource::File(file_bytes) => {
            // Non-magnet: send .torrent file bytes via multipart file field
            let mut buf = Vec::with_capacity(file_bytes.len() + 512);
            buf.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"torrents\"; filename=\"torrent.torrent\"\r\nContent-Type: application/x-bittorrent\r\n\r\n"
                )
                .as_bytes(),
            );
            buf.extend_from_slice(file_bytes);
            buf.extend_from_slice(
                format!(
                    "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"category\"\r\n\r\n{safe_category}\r\n--{boundary}--\r\n"
                )
                .as_bytes(),
            );
            buf
        }
        TorrentDispatchSource::Magnet(s) | TorrentDispatchSource::Url(s) => {
            // Magnet URI (direct, recovered, or body-text) or fallback URL: send via urls= text field
            let safe_url = sanitize_multipart_value(s);
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"urls\"\r\n\r\n{safe_url}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"category\"\r\n\r\n{safe_category}\r\n--{boundary}--\r\n",
            )
            .into_bytes()
        }
    };

    let add_resp = http
        .fetch(FetchRequest {
            url: add_url,
            method: HttpMethod::Post,
            headers: vec![
                (
                    "Content-Type".into(),
                    format!("multipart/form-data; boundary={boundary}"),
                ),
                ("Cookie".into(), auth_cookie),
            ],
            body: Some(body),
            timeout: Duration::from_secs(30),
            rate_bucket: RateBucket::None,
            max_body_bytes: 4096,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Normal,
        })
        .await
        .map_err(|e| format!("qBit add torrent failed: {e}"))?;

    let body_text = String::from_utf8_lossy(&add_resp.body);

    if (200..300).contains(&add_resp.status) {
        if body_text.contains("Fails") {
            return Err(format!("qBit add failed: {}", body_text.trim()));
        }
        return Ok(download_id);
    }

    match add_resp.status {
        403 => Err("qBit auth expired".to_string()),
        s => Err(format!(
            "qBit rejected torrent: HTTP {s}: {}",
            body_text.trim()
        )),
    }
}

/// Dispatch torrent to Transmission via JSON-RPC.
async fn dispatch_transmission<H: HttpFetcher>(
    http: &H,
    client: &DownloadClient,
    download_url: &str,
    origin: &str,
) -> Result<String, String> {
    let (hash, dispatch_source) = fetch_torrent_dispatch_source(http, download_url, origin)
        .await
        .map_err(|e| format!("indexer cooling down — retry later: {e}"))?;
    let download_id = hash.unwrap_or_else(|| "pending".to_string());

    let scheme = if client.use_ssl { "https" } else { "http" };
    let url_base = client.url_base.as_deref().unwrap_or("");
    let base = format!("{}://{}:{}{}", scheme, client.host, client.port, url_base);
    let rpc_url = format!("{base}/transmission/rpc");

    // Build torrent-add request. Use filename for magnet URIs, metainfo for .torrent files.
    let mut args = serde_json::json!({});
    match dispatch_source {
        TorrentDispatchSource::Magnet(magnet) => {
            args["filename"] = serde_json::Value::String(magnet);
        }
        TorrentDispatchSource::File(file_bytes) => {
            // Reuse bytes already fetched by fetch_torrent_dispatch_source
            use data_encoding::BASE64;
            args["metainfo"] = serde_json::Value::String(BASE64.encode(&file_bytes));
        }
        TorrentDispatchSource::Url(url) => {
            // Fallback: fetch_torrent_dispatch_source failed to get bytes, try direct fetch
            let torrent_resp = http
                .fetch(FetchRequest {
                    url,
                    method: HttpMethod::Get,
                    headers: vec![],
                    body: None,
                    timeout: Duration::from_secs(30),
                    rate_bucket: RateBucket::Indexer(origin.to_string()),
                    max_body_bytes: 10 * 1024 * 1024,
                    anti_bot_check: false,
                    user_agent: UserAgentProfile::Server,
                    priority: RequestPriority::Normal,
                })
                .await
                .map_err(|e| format!("failed to download .torrent: {e}"))?;

            if !(200..300).contains(&torrent_resp.status) {
                return Err(format!(
                    "torrent download returned HTTP {}",
                    torrent_resp.status
                ));
            }

            use data_encoding::BASE64;
            args["metainfo"] = serde_json::Value::String(BASE64.encode(&torrent_resp.body));
        }
    }

    if let Some(ref dir) = client.download_dir {
        args["download-dir"] = serde_json::Value::String(dir.clone());
    }

    let rpc_body = serde_json::json!({"method": "torrent-add", "arguments": args});

    // Session-ID handshake: first request may return 409
    let mut session_id: Option<String> = None;
    for attempt in 0..2 {
        let mut headers = vec![("Content-Type".into(), "application/json".into())];
        if let (Some(u), Some(p)) = (client.username.as_deref(), client.password.as_deref()) {
            use data_encoding::BASE64;
            let cred = BASE64.encode(format!("{u}:{p}").as_bytes());
            headers.push(("Authorization".into(), format!("Basic {cred}")));
        }
        if let Some(ref sid) = session_id {
            headers.push(("X-Transmission-Session-Id".into(), sid.clone()));
        }

        let resp = http
            .fetch(FetchRequest {
                url: rpc_url.clone(),
                method: HttpMethod::Post,
                headers,
                body: Some(serde_json::to_vec(&rpc_body).unwrap()),
                timeout: Duration::from_secs(30),
                rate_bucket: RateBucket::None,
                max_body_bytes: 64 * 1024,
                anti_bot_check: false,
                user_agent: UserAgentProfile::Server,
                priority: RequestPriority::Normal,
            })
            .await
            .map_err(|e| format!("Transmission RPC failed: {e}"))?;

        if resp.status == 409 {
            if attempt == 1 {
                return Err("Transmission CSRF handshake failed".to_string());
            }
            session_id = resp
                .headers
                .iter()
                .find(|(k, _)| k.to_lowercase() == "x-transmission-session-id")
                .map(|(_, v)| v.clone());
            continue;
        }

        if resp.status == 401 {
            return Err("Transmission auth failed".to_string());
        }

        if resp.status != 200 {
            return Err(format!("Transmission rejected: HTTP {}", resp.status));
        }

        let body: serde_json::Value = serde_json::from_slice(&resp.body)
            .map_err(|e| format!("Transmission parse error: {e}"))?;

        let result = body.get("result").and_then(|v| v.as_str()).unwrap_or("");
        if result != "success" {
            return Err(format!("Transmission error: {result}"));
        }

        // Extract hash from response (torrent-added or torrent-duplicate)
        let hash = body
            .pointer("/arguments/torrent-added/hashString")
            .or_else(|| body.pointer("/arguments/torrent-duplicate/hashString"))
            .and_then(|v| v.as_str())
            .unwrap_or(&download_id);

        return Ok(hash.to_string());
    }

    Err("Transmission dispatch failed".to_string())
}

/// Dispatch NZB to SABnzbd: download NZB from indexer, push via multipart addfile.
async fn dispatch_usenet<H: HttpFetcher>(
    http: &H,
    client: &DownloadClient,
    download_url: &str,
    title: &str,
    origin: &str,
) -> Result<String, String> {
    // Step 1: Download NZB from indexer.
    let nzb_resp = http
        .fetch(FetchRequest {
            url: download_url.to_string(),
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(60),
            rate_bucket: RateBucket::Indexer(origin.to_string()),
            max_body_bytes: 16 * 1024 * 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Normal,
        })
        .await
        .map_err(|e| format!("failed to download NZB from indexer: {e}"))?;

    if !(200..300).contains(&nzb_resp.status) {
        return Err(format!(
            "indexer returned HTTP {} when fetching NZB",
            nzb_resp.status
        ));
    }

    let nzb_bytes = nzb_resp.body;

    // Step 2: Build multipart addfile request for SABnzbd.
    let scheme = if client.use_ssl { "https" } else { "http" };
    let url_base = client.url_base.as_deref().unwrap_or("");
    let api_key = client.api_key.as_deref().unwrap_or("");
    let sab_url = format!(
        "{}://{}:{}{}/api",
        scheme, client.host, client.port, url_base
    );

    let filename = format!("{}.nzb", title.replace('/', "_"));
    let boundary = "----livrarr-sab-boundary";
    let (content_type, body) = build_multipart_addfile(
        boundary,
        &[
            ("mode", "addfile"),
            ("cat", &client.category),
            ("apikey", api_key),
            ("output", "json"),
        ],
        "name",
        &filename,
        "application/x-nzb",
        &nzb_bytes,
    );

    // Step 3: POST to SABnzbd.
    let resp = http
        .fetch(FetchRequest {
            url: sab_url,
            method: HttpMethod::Post,
            headers: vec![("Content-Type".into(), content_type)],
            body: Some(body),
            timeout: Duration::from_secs(30),
            rate_bucket: RateBucket::None,
            max_body_bytes: 4096,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority: RequestPriority::Normal,
        })
        .await
        .map_err(|e| format!("SABnzbd unreachable: {e}"))?;

    if !(200..300).contains(&resp.status) {
        return Err(format!("SABnzbd returned HTTP {}", resp.status));
    }

    // Step 4: Parse JSON response properly.
    let body_json: serde_json::Value = serde_json::from_slice(&resp.body)
        .map_err(|e| format!("SABnzbd response parse error: {e}"))?;

    if body_json.get("status").and_then(|s| s.as_bool()) == Some(false) {
        let error = body_json
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("unknown error");
        return Err(format!("SABnzbd rejected NZB: {error}"));
    }

    let nzo_id = body_json
        .get("nzo_ids")
        .and_then(|ids| ids.as_array())
        .and_then(|ids| ids.first())
        .and_then(|id| id.as_str())
        .map(str::to_owned);

    match nzo_id {
        Some(id) => Ok(id),
        None => Ok("pending".to_string()),
    }
}

/// Strip CR and LF from a string to prevent CRLF injection in multipart headers/values.
fn sanitize_multipart_value(s: &str) -> String {
    s.chars().filter(|&c| c != '\r' && c != '\n').collect()
}

fn build_multipart_addfile(
    boundary: &str,
    fields: &[(&str, &str)],
    file_field: &str,
    file_name: &str,
    mime: &str,
    file_bytes: &[u8],
) -> (String, Vec<u8>) {
    let mut body = Vec::new();

    for (name, value) in fields {
        let safe_name = sanitize_multipart_value(name);
        let safe_value = sanitize_multipart_value(value);
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{safe_name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(safe_value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    let safe_file_field = sanitize_multipart_value(file_field);
    let safe_file_name = sanitize_multipart_value(file_name);
    let safe_mime = sanitize_multipart_value(mime);
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{safe_file_field}\"; filename=\"{safe_file_name}\"\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {safe_mime}\r\n\r\n").as_bytes());
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(b"\r\n");

    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn protocol_str(p: &DownloadProtocol) -> &'static str {
    match p {
        DownloadProtocol::Torrent => "torrent",
        DownloadProtocol::Usenet => "usenet",
    }
}
