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
}

impl<D, H> ReleaseServiceImpl<D, H> {
    pub fn new(db: D, http: H) -> Self {
        Self { db, http }
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

/// Simple SSRF check — reject private/loopback IPs in download URLs.
fn is_ssrf_url(url: &str) -> bool {
    // Parse out the host from the URL
    let host = if let Some(rest) = url.strip_prefix("http://") {
        rest
    } else if let Some(rest) = url.strip_prefix("https://") {
        rest
    } else {
        // Non-HTTP URLs (magnet, etc.) are not subject to SSRF
        return false;
    };

    // Extract host portion (before port/path)
    let host = host.split('/').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host);
    let host = host.to_lowercase();

    // Check for private/loopback addresses
    if host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "0.0.0.0" {
        return true;
    }

    // Check for private IP ranges
    if let Ok(ip) = host.parse::<std::net::Ipv4Addr>() {
        let octets = ip.octets();
        // 10.x.x.x
        if octets[0] == 10 {
            return true;
        }
        // 172.16.0.0 - 172.31.255.255
        if octets[0] == 172 && (16..=31).contains(&octets[1]) {
            return true;
        }
        // 192.168.x.x
        if octets[0] == 192 && octets[1] == 168 {
            return true;
        }
        // 169.254.x.x (link-local)
        if octets[0] == 169 && octets[1] == 254 {
            return true;
        }
    }

    false
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

        // Build search query from work title + author
        let query = if work.author_name.is_empty() {
            work.title.clone()
        } else {
            format!("{} {}", work.title, work.author_name)
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
                    url.push_str(&format!("&apikey={api_key}"));
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
        })
    }

    async fn grab(&self, user_id: UserId, req: GrabRequest) -> Result<Grab, ReleaseServiceError> {
        // SSRF validation on download_url
        if is_ssrf_url(&req.download_url) {
            return Err(ReleaseServiceError::Ssrf(
                "download URL points to private/loopback address".to_string(),
            ));
        }

        // Determine client_type from protocol
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
            if c.client_type() != client_type {
                return Err(ReleaseServiceError::ClientProtocolMismatch {
                    protocol: protocol_str(&req.protocol).to_string(),
                });
            }
            c
        } else {
            // Use default client for protocol
            self.db
                .get_default_download_client(client_type)
                .await
                .map_err(ReleaseServiceError::Db)?
                .ok_or_else(|| ReleaseServiceError::NoClient {
                    protocol: client_type.to_string(),
                })?
        };

        // Dispatch to download client via HTTP
        // For torrent: POST to qBit API
        // For usenet: POST to SABnzbd API
        let dispatch_result = match req.protocol {
            DownloadProtocol::Torrent => {
                dispatch_torrent(&self.http, &client, &req.download_url).await
            }
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

/// Best-effort torrent info_hash extraction before sending to qBit.
/// Handles: direct magnet URIs, body-text magnets, .torrent file bytes.
async fn fetch_and_extract_hash<H: HttpFetcher>(http: &H, download_url: &str) -> Option<String> {
    use crate::{extract_torrent_hash, TorrentSource};

    if download_url.starts_with("magnet:") {
        return extract_torrent_hash(&TorrentSource::Magnet(download_url.to_string())).ok();
    }

    let resp = http
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
        .ok()?;

    if !(200..300).contains(&resp.status) {
        return None;
    }

    if let Ok(text) = std::str::from_utf8(&resp.body) {
        let trimmed = text.trim();
        if trimmed.starts_with("magnet:") {
            return extract_torrent_hash(&TorrentSource::Magnet(trimmed.to_string())).ok();
        }
    }

    extract_torrent_hash(&TorrentSource::TorrentFile {
        filename: "download.torrent".to_string(),
        data: resp.body,
    })
    .ok()
}

/// Dispatch torrent to qBittorrent via HTTP API.
async fn dispatch_torrent<H: HttpFetcher>(
    http: &H,
    client: &DownloadClient,
    download_url: &str,
) -> Result<String, String> {
    let download_id = fetch_and_extract_hash(http, download_url)
        .await
        .unwrap_or_else(|| "pending".to_string());

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

    if auth_resp.status != 200 {
        return Err("qBit auth failed".to_string());
    }

    // Extract SID cookie
    let sid = auth_resp
        .headers
        .iter()
        .find(|(k, _)| k.to_lowercase() == "set-cookie")
        .and_then(|(_, v)| {
            v.split(';')
                .next()
                .filter(|c| c.starts_with("SID="))
                .map(|c| c.to_string())
        })
        .unwrap_or_default();

    // Add torrent via URL
    let add_url = format!("{base}/api/v2/torrents/add");
    let boundary = "----livrarr-boundary";
    let body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"urls\"\r\n\r\n{download_url}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"category\"\r\n\r\n{}\r\n--{boundary}--\r\n",
        client.category
    );

    let add_resp = http
        .fetch(FetchRequest {
            url: add_url,
            method: HttpMethod::Post,
            headers: vec![
                (
                    "Content-Type".into(),
                    format!("multipart/form-data; boundary={boundary}"),
                ),
                ("Cookie".into(), sid),
            ],
            body: Some(body.into_bytes()),
            timeout: Duration::from_secs(30),
            rate_bucket: RateBucket::None,
            max_body_bytes: 4096,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
        })
        .await
        .map_err(|e| format!("qBit add torrent failed: {e}"))?;

    if add_resp.status == 200 {
        Ok(download_id)
    } else if add_resp.status == 403 {
        Err("qBit auth expired".to_string())
    } else {
        Err(format!("qBit rejected torrent: HTTP {}", add_resp.status))
    }
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
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"{file_field}\"; filename=\"{file_name}\"\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(format!("Content-Type: {mime}\r\n\r\n").as_bytes());
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
