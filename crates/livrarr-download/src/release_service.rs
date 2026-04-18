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

/// Parse Torznab XML response into ReleaseResult items.
/// Returns (results, warnings). Items missing guid or download_url are skipped with warning.
fn parse_torznab_xml(xml: &[u8], indexer_name: &str) -> (Vec<ReleaseResult>, Vec<String>) {
    let mut results = Vec::new();
    let mut warnings = Vec::new();

    let xml_str = match std::str::from_utf8(xml) {
        Ok(s) => s,
        Err(e) => {
            warnings.push(format!(
                "indexer {indexer_name}: invalid UTF-8 in response: {e}"
            ));
            return (results, warnings);
        }
    };

    // Simple XML parsing for Torznab responses.
    // Torznab XML format:
    // <rss><channel><item>
    //   <title>...</title>
    //   <guid>...</guid>
    //   <link>...</link> (or <enclosure url="...">)
    //   <size>...</size>
    //   <newznab:attr name="seeders" value="..."/>
    //   <newznab:attr name="peers" value="..."/>
    //   <newznab:attr name="category" value="..."/>
    //   <pubDate>...</pubDate>
    // </item></channel></rss>

    // Split into items
    let items: Vec<&str> = xml_str.split("<item>").skip(1).collect();

    for item_str in items {
        let item_end = item_str.find("</item>").unwrap_or(item_str.len());
        let item_xml = &item_str[..item_end];

        let title = extract_xml_element(item_xml, "title").unwrap_or_default();
        let guid = extract_xml_element(item_xml, "guid");
        let link = extract_xml_element(item_xml, "link");
        let enclosure_url = extract_xml_attr(item_xml, "enclosure", "url");
        let size_str = extract_xml_element(item_xml, "size")
            .or_else(|| extract_torznab_attr(item_xml, "size"));
        let seeders_str = extract_torznab_attr(item_xml, "seeders");
        let leechers_str = extract_torznab_attr(item_xml, "peers");
        let pub_date = extract_xml_element(item_xml, "pubDate");
        let categories = extract_torznab_categories(item_xml);

        // guid is required
        let guid = match guid {
            Some(g) if !g.is_empty() => g,
            _ => {
                warnings.push(format!(
                    "indexer {indexer_name}: skipped item missing guid (title: {})",
                    if title.is_empty() {
                        "<unknown>"
                    } else {
                        &title
                    }
                ));
                continue;
            }
        };

        // download_url: prefer enclosure url, fall back to link
        let download_url = enclosure_url.or(link);
        let download_url = match download_url {
            Some(u) if !u.is_empty() => u,
            _ => {
                warnings.push(format!(
                    "indexer {indexer_name}: skipped item missing downloadUrl (guid: {guid})"
                ));
                continue;
            }
        };

        let size = size_str.and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
        let seeders = seeders_str.and_then(|s| s.parse::<i32>().ok());
        let leechers = leechers_str.and_then(|s| s.parse::<i32>().ok());

        // Determine protocol from categories or URL
        let protocol = if download_url.starts_with("magnet:")
            || download_url.ends_with(".torrent")
            || categories.iter().any(|c| *c / 1000 == 7 || *c / 1000 == 5)
        {
            DownloadProtocol::Torrent
        } else {
            // Default to torrent for now; usenet NZBs typically have .nzb extension
            if download_url.ends_with(".nzb") {
                DownloadProtocol::Usenet
            } else {
                DownloadProtocol::Torrent
            }
        };

        results.push(ReleaseResult {
            title,
            indexer: indexer_name.to_string(),
            size,
            guid,
            download_url,
            seeders,
            leechers,
            publish_date: pub_date,
            protocol,
            categories,
        });
    }

    (results, warnings)
}

fn extract_xml_element(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    let content = &xml[start..end];
    // Handle CDATA
    let content = if let Some(inner) = content.strip_prefix("<![CDATA[") {
        inner.strip_suffix("]]>").unwrap_or(inner)
    } else {
        content
    };
    Some(content.to_string())
}

fn extract_xml_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    let tag_open = format!("<{tag} ");
    let start = xml.find(&tag_open)?;
    let tag_str = &xml[start..];
    let end = tag_str.find('>')? + start;
    let tag_content = &xml[start..=end];

    let attr_prefix = format!("{attr}=\"");
    let attr_start = tag_content.find(&attr_prefix)? + attr_prefix.len();
    let attr_end = tag_content[attr_start..].find('"')? + attr_start;
    Some(tag_content[attr_start..attr_end].to_string())
}

fn extract_torznab_attr(xml: &str, name: &str) -> Option<String> {
    // Match both newznab:attr and torznab:attr
    let patterns = [
        format!("newznab:attr name=\"{name}\" value=\""),
        format!("torznab:attr name=\"{name}\" value=\""),
    ];
    for pattern in &patterns {
        if let Some(start) = xml.find(pattern.as_str()) {
            let val_start = start + pattern.len();
            if let Some(end) = xml[val_start..].find('"') {
                return Some(xml[val_start..val_start + end].to_string());
            }
        }
    }
    None
}

fn extract_torznab_categories(xml: &str) -> Vec<i32> {
    let mut cats = Vec::new();
    let patterns = [
        "newznab:attr name=\"category\" value=\"",
        "torznab:attr name=\"category\" value=\"",
    ];
    for pattern in &patterns {
        let mut search_from = 0;
        while let Some(pos) = xml[search_from..].find(pattern) {
            let val_start = search_from + pos + pattern.len();
            if let Some(end) = xml[val_start..].find('"') {
                if let Ok(cat) = xml[val_start..val_start + end].parse::<i32>() {
                    cats.push(cat);
                }
            }
            search_from = val_start;
        }
    }
    cats
}

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
                    let (mut results, mut parse_warnings) = parse_torznab_xml(&body, &indexer_name);
                    all_results.append(&mut results);
                    warnings.append(&mut parse_warnings);
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

/// Dispatch torrent to qBittorrent via HTTP API.
async fn dispatch_torrent<H: HttpFetcher>(
    http: &H,
    client: &DownloadClient,
    download_url: &str,
) -> Result<String, String> {
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
        // qBit doesn't return a download ID on add; use a placeholder
        // The actual torrent hash is resolved later by the poller
        Ok("pending".to_string())
    } else if add_resp.status == 403 {
        Err("qBit auth expired".to_string())
    } else {
        Err(format!("qBit rejected torrent: HTTP {}", add_resp.status))
    }
}

/// Dispatch NZB to SABnzbd via HTTP API.
async fn dispatch_usenet<H: HttpFetcher>(
    http: &H,
    client: &DownloadClient,
    download_url: &str,
    title: &str,
) -> Result<String, String> {
    let scheme = if client.use_ssl { "https" } else { "http" };
    let url_base = client.url_base.as_deref().unwrap_or("");
    let api_key = client.api_key.as_deref().unwrap_or("");
    let url = format!(
        "{}://{}:{}{}/api?mode=addurl&name={}&nzbname={}&cat={}&apikey={}&output=json",
        scheme,
        client.host,
        client.port,
        url_base,
        urlencoded(download_url),
        urlencoded(title),
        urlencoded(&client.category),
        api_key,
    );

    let resp = http
        .fetch(FetchRequest {
            url,
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(30),
            rate_bucket: RateBucket::None,
            max_body_bytes: 4096,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
        })
        .await
        .map_err(|e| format!("SABnzbd unreachable: {e}"))?;

    if resp.status == 200 {
        // Parse SABnzbd JSON response for nzo_id
        let body = String::from_utf8_lossy(&resp.body);
        if let Some(nzo_start) = body.find("nzo_") {
            let nzo_end = body[nzo_start..]
                .find('"')
                .unwrap_or(body[nzo_start..].len());
            Ok(body[nzo_start..nzo_start + nzo_end].to_string())
        } else {
            Ok("pending".to_string())
        }
    } else {
        Err(format!("SABnzbd rejected NZB: HTTP {}", resp.status))
    }
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
