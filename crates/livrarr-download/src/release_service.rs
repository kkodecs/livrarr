use std::collections::HashSet;
use std::time::Duration;

use livrarr_db::{
    CreateGrabDbRequest, CreateHistoryEventDbRequest, DownloadClientDb, GrabDb, HistoryDb,
    IndexerDb, WorkDb,
};
use livrarr_domain::services::*;
use livrarr_domain::*;

/// ReleaseService implementation — search indexers and grab releases.
pub struct ReleaseServiceImpl<D, H> {
    db: D,
    http: H,
    trusted_origins: std::sync::Arc<livrarr_http::ssrf::TrustedOrigins>,
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

        // Fan-out parallel requests with per-indexer 30s timeout
        let mut handles = tokio::task::JoinSet::new();

        for indexer in &indexers {
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
                        return (indexer_name, Err(format!("failed to load indexer: {e}")));
                    }
                };

                // Build Torznab search URL
                let base_url = indexer.url.trim_end_matches('/');
                let api_path = indexer.api_path.trim_start_matches('/');
                let mut url = format!("{base_url}/{api_path}?t=search&q={}", urlencoded(&query));
                if let Some(ref api_key) = indexer.api_key {
                    url.push_str(&format!("&apikey={}", urlencoded(api_key)));
                }
                // Add categories
                if !indexer.categories.is_empty() {
                    let cats: Vec<String> =
                        indexer.categories.iter().map(|c| c.to_string()).collect();
                    url.push_str(&format!("&cat={}", cats.join(",")));
                }

                let fetch_req = FetchRequest {
                    url,
                    method: HttpMethod::Get,
                    headers: vec![],
                    body: None,
                    timeout: Duration::from_secs(30),
                    rate_bucket: RateBucket::Indexer(indexer_name.clone()),
                    max_body_bytes: 10 * 1024 * 1024,
                    anti_bot_check: false,
                    user_agent: UserAgentProfile::Server,
                };

                match http.fetch(fetch_req).await {
                    Ok(resp) if resp.status == 200 => {
                        (indexer_name, Ok::<Vec<u8>, String>(resp.body))
                    }
                    Ok(resp) => (
                        indexer_name,
                        Err::<Vec<u8>, String>(format!("HTTP {}", resp.status)),
                    ),
                    Err(e) => (indexer_name, Err::<Vec<u8>, String>(format!("{e}"))),
                }
            });
        }

        let mut all_results: Vec<ReleaseResult> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut any_success = false;

        while let Some(join_result) = handles.join_next().await {
            match join_result {
                Ok((indexer_name, Ok(body))) => {
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
                            all_results.extend(results);
                        }
                        Ok(TorznabParseResult::Error { code, description }) => {
                            warnings.push(format!(
                                "indexer {indexer_name}: error {code}: {description}"
                            ));
                        }
                        Err(e) => {
                            warnings.push(format!("indexer {indexer_name}: {e}"));
                        }
                    }
                }
                Ok((indexer_name, Err(err_msg))) => {
                    // Don't expose API keys in warnings
                    let safe_msg = if err_msg.contains("apikey=") {
                        "request failed".to_string()
                    } else {
                        err_msg
                    };
                    warnings.push(format!("indexer {indexer_name}: {safe_msg}"));
                }
                Err(join_err) => {
                    warnings.push(format!("indexer task panicked: {join_err}"));
                }
            }
        }

        if !any_success {
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
            cache_age_seconds: None,
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
                    dispatch_transmission(&self.http, &client, &req.download_url).await
                }
                _ => dispatch_torrent(&self.http, &client, &req.download_url).await,
            },
            DownloadProtocol::Usenet => {
                dispatch_usenet(&self.http, &client, &req.download_url, &req.title).await
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

/// Best-effort torrent info_hash extraction before sending to download client.
/// Handles: direct magnet URIs, body-text magnets, .torrent file bytes,
/// and redirect-to-magnet responses (e.g. Prowlarr proxying a torrent indexer).
///
/// Returns `(hash, Option<torrent_bytes>)`:
/// - For magnets: hash extracted from URI, no bytes.
/// - For .torrent URLs: hash extracted from file, bytes returned for reuse
///   (avoids double-fetch when the download client also needs the file).
async fn fetch_and_extract_hash<H: HttpFetcher>(
    http: &H,
    download_url: &str,
) -> (Option<String>, Option<Vec<u8>>) {
    use crate::{extract_torrent_hash, TorrentSource};

    if download_url.starts_with("magnet:") {
        let hash = extract_torrent_hash(&TorrentSource::Magnet(download_url.to_string())).ok();
        return (hash, None);
    }

    let resp = match http
        .fetch(FetchRequest {
            url: download_url.to_string(),
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(60),
            rate_bucket: RateBucket::None,
            max_body_bytes: 4 * 1024 * 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
        })
        .await
    {
        Ok(r) => r,
        Err(_) => {
            // The fetch may have failed because the URL redirects to a magnet: URI.
            // reqwest's redirect-following client rejects non-HTTP schemes, so a
            // 301/302 Location: magnet:?xt=... response causes an Err rather than a
            // usable redirect. Some indexer proxies (e.g. Prowlarr) use this pattern.
            // Probe with a no-redirect client to recover the magnet if that's the case.
            if let Some(magnet) = probe_for_magnet_redirect(download_url).await {
                let hash = extract_torrent_hash(&TorrentSource::Magnet(magnet)).ok();
                return (hash, None);
            }
            return (None, None);
        }
    };

    if !(200..300).contains(&resp.status) {
        return (None, None);
    }

    // Some indexers return a magnet URI as the response body
    if let Ok(text) = std::str::from_utf8(&resp.body) {
        let trimmed = text.trim();
        if trimmed.starts_with("magnet:") {
            let hash = extract_torrent_hash(&TorrentSource::Magnet(trimmed.to_string())).ok();
            return (hash, None);
        }
    }

    let hash = extract_torrent_hash(&TorrentSource::TorrentFile {
        filename: "download.torrent".to_string(),
        data: resp.body.clone(),
    })
    .ok();
    (hash, Some(resp.body))
}

/// Probe a URL with a no-redirect client and return the Location value if it is
/// a magnet: URI. Used to recover from redirect-to-magnet responses that cause
/// the redirect-following client to error on the non-HTTP scheme.
async fn probe_for_magnet_redirect(url: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .ok()?;
    let resp = client.get(url).send().await.ok()?;
    if !resp.status().is_redirection() {
        return None;
    }
    let location = resp
        .headers()
        .get(reqwest::header::LOCATION)?
        .to_str()
        .ok()?;
    if location.starts_with("magnet:") {
        Some(location.to_string())
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
) -> Result<String, String> {
    let (hash, torrent_bytes) = fetch_and_extract_hash(http, download_url).await;
    let download_id = hash.unwrap_or_else(|| "pending".to_string());

    let scheme = if client.use_ssl { "https" } else { "http" };
    let url_base = client.url_base.as_deref().unwrap_or("");
    let base = format!("{}://{}:{}{}", scheme, client.host, client.port, url_base);

    // First authenticate
    let auth_url = format!("{base}/api/v2/auth/login");
    let auth_body = format!(
        "username={}&password={}",
        urlencoded(client.username.as_deref().unwrap_or("")),
        urlencoded(client.password.as_deref().unwrap_or("")),
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

    let body = if let Some(ref file_bytes) = torrent_bytes {
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
    } else {
        // Magnet URI: send via urls= text field
        let safe_url = sanitize_multipart_value(download_url);
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"urls\"\r\n\r\n{safe_url}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"category\"\r\n\r\n{safe_category}\r\n--{boundary}--\r\n",
        )
        .into_bytes()
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
) -> Result<String, String> {
    let (hash, torrent_bytes) = fetch_and_extract_hash(http, download_url).await;
    let download_id = hash.unwrap_or_else(|| "pending".to_string());

    let scheme = if client.use_ssl { "https" } else { "http" };
    let url_base = client.url_base.as_deref().unwrap_or("");
    let base = format!("{}://{}:{}{}", scheme, client.host, client.port, url_base);
    let rpc_url = format!("{base}/transmission/rpc");

    // Build torrent-add request. Use filename for magnet URIs, metainfo for .torrent files.
    let mut args = serde_json::json!({});
    if download_url.starts_with("magnet:") {
        args["filename"] = serde_json::Value::String(download_url.to_string());
    } else if let Some(ref file_bytes) = torrent_bytes {
        // Reuse bytes already fetched by fetch_and_extract_hash
        use data_encoding::BASE64;
        args["metainfo"] = serde_json::Value::String(BASE64.encode(file_bytes));
    } else {
        // Fallback: fetch_and_extract_hash failed to get bytes, try direct fetch
        let torrent_resp = http
            .fetch(FetchRequest {
                url: download_url.to_string(),
                method: HttpMethod::Get,
                headers: vec![],
                body: None,
                timeout: Duration::from_secs(30),
                rate_bucket: RateBucket::None,
                max_body_bytes: 10 * 1024 * 1024,
                anti_bot_check: false,
                user_agent: UserAgentProfile::Server,
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
) -> Result<String, String> {
    // Step 1: Download NZB from indexer.
    let nzb_resp = http
        .fetch(FetchRequest {
            url: download_url.to_string(),
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(60),
            rate_bucket: RateBucket::None,
            max_body_bytes: 16 * 1024 * 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
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

/// Minimal URL encoding for query parameters.
fn urlencoded(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(b as char);
            }
            b' ' => result.push('+'),
            _ => {
                result.push('%');
                result.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                result.push(char::from(b"0123456789ABCDEF"[(b & 0x0f) as usize]));
            }
        }
    }
    result
}

fn protocol_str(p: &DownloadProtocol) -> &'static str {
    match p {
        DownloadProtocol::Torrent => "torrent",
        DownloadProtocol::Usenet => "usenet",
    }
}
