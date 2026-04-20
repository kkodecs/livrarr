use std::time::Duration;

use crate::state::AppState;
use crate::{ApiError, ReleaseResponse};
use livrarr_domain::torznab::{parse_torznab_xml, TorznabParseResult};
use livrarr_domain::Indexer;
use livrarr_http::HttpClient;

const MAX_RESPONSE_BODY_BYTES: usize = 10 * 1024 * 1024;

fn clean_search_term(title: &str, author: &str) -> String {
    let mut t = title.to_string();

    let prefix = format!("{author}:");
    if t.starts_with(&prefix) {
        t = t[prefix.len()..].trim().to_string();
    }

    let colon = t.find(':');
    let paren = t.find('(');
    let split_at = match (colon, paren) {
        (Some(c), Some(p)) => Some(c.min(p)),
        (Some(c), None) => Some(c),
        (None, Some(p)) => Some(p),
        _ => None,
    };
    if let Some(pos) = split_at {
        if pos > 0 {
            t = t[..pos].trim().to_string();
        }
    }

    if t.to_lowercase().starts_with("the ") {
        t = t[4..].to_string();
    }

    t = t.replace(" & ", " ").replace('.', " ");
    t = t.split_whitespace().collect::<Vec<_>>().join(" ");

    t
}

pub(crate) async fn search_indexer(
    http: &HttpClient,
    indexer: &Indexer,
    title: &str,
    author: &str,
) -> Result<Vec<ReleaseResponse>, String> {
    let cats = indexer
        .categories
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(",");

    let clean_title = clean_search_term(title, author);
    let mut all_results: Vec<ReleaseResponse> = Vec::new();
    let mut seen_guids: std::collections::HashSet<String> = std::collections::HashSet::new();

    let add_results = |items: Vec<ReleaseResponse>,
                       results: &mut Vec<ReleaseResponse>,
                       seen: &mut std::collections::HashSet<String>| {
        for item in items {
            if seen.insert(item.guid.clone()) {
                results.push(item);
            }
        }
    };

    if indexer.supports_book_search {
        let url = build_torznab_url(
            &indexer.url,
            &indexer.api_path,
            indexer.api_key.as_deref(),
            &[
                ("t", "book"),
                ("author", author),
                ("title", &clean_title),
                ("cat", &cats),
                ("limit", "500"),
                ("extended", "1"),
            ],
        );
        if let Ok(items) = fetch_and_parse(http, &url, &indexer.name).await {
            add_results(items, &mut all_results, &mut seen_guids);
        }

        let url = build_torznab_url(
            &indexer.url,
            &indexer.api_path,
            indexer.api_key.as_deref(),
            &[
                ("t", "book"),
                ("title", &clean_title),
                ("cat", &cats),
                ("limit", "500"),
                ("extended", "1"),
            ],
        );
        if let Ok(items) = fetch_and_parse(http, &url, &indexer.name).await {
            add_results(items, &mut all_results, &mut seen_guids);
        }

        if !all_results.is_empty() {
            return Ok(all_results);
        }
    }

    let query = format!("{clean_title} {author}");
    let url = build_torznab_url(
        &indexer.url,
        &indexer.api_path,
        indexer.api_key.as_deref(),
        &[
            ("t", "search"),
            ("q", &query),
            ("cat", &cats),
            ("limit", "500"),
            ("extended", "1"),
        ],
    );
    if let Ok(items) = fetch_and_parse(http, &url, &indexer.name).await {
        add_results(items, &mut all_results, &mut seen_guids);
    }

    let query_rev = format!("{author} {clean_title}");
    let url = build_torznab_url(
        &indexer.url,
        &indexer.api_path,
        indexer.api_key.as_deref(),
        &[
            ("t", "search"),
            ("q", &query_rev),
            ("cat", &cats),
            ("limit", "500"),
            ("extended", "1"),
        ],
    );
    if let Ok(items) = fetch_and_parse(http, &url, &indexer.name).await {
        add_results(items, &mut all_results, &mut seen_guids);
    }

    if !all_results.is_empty() {
        return Ok(all_results);
    }

    let url = build_torznab_url(
        &indexer.url,
        &indexer.api_path,
        indexer.api_key.as_deref(),
        &[
            ("t", "search"),
            ("q", &clean_title),
            ("cat", &cats),
            ("limit", "500"),
            ("extended", "1"),
        ],
    );
    fetch_and_parse(http, &url, &indexer.name).await
}

pub(crate) fn build_torznab_url(
    base: &str,
    api_path: &str,
    api_key: Option<&str>,
    params: &[(&str, &str)],
) -> String {
    let base_with_path = format!("{base}{api_path}");
    let separator = if base_with_path.contains('?') {
        '&'
    } else {
        '?'
    };
    let mut url = format!("{base_with_path}{separator}");
    for (i, (k, v)) in params.iter().enumerate() {
        if i > 0 {
            url.push('&');
        }
        url.push_str(k);
        url.push('=');
        url.push_str(&urlencoding::encode(v));
    }
    if let Some(key) = api_key {
        if !key.is_empty() {
            url.push_str("&apikey=");
            url.push_str(&urlencoding::encode(key));
        }
    }
    url
}

fn redact_url(url: &str) -> String {
    let mut result = url.to_string();
    if let Some(pos) = result.find("apikey=") {
        let end = result[pos..]
            .find('&')
            .map(|i| pos + i)
            .unwrap_or(result.len());
        result = format!("{}apikey=[REDACTED]{}", &result[..pos], &result[end..]);
    }
    if let Some(pos) = result.find("q=") {
        let end = result[pos..]
            .find('&')
            .map(|i| pos + i)
            .unwrap_or(result.len());
        result = format!("{}q=[REDACTED]{}", &result[..pos], &result[end..]);
    }
    result
}

pub(crate) async fn fetch_and_parse(
    http: &HttpClient,
    url: &str,
    indexer_name: &str,
) -> Result<Vec<ReleaseResponse>, String> {
    tracing::debug!(indexer = %indexer_name, url = %redact_url(url), "searching indexer");

    let resp = http
        .get(url)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.without_url().to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    if let Some(cl) = resp.content_length() {
        if cl as usize > MAX_RESPONSE_BODY_BYTES {
            return Err(format!(
                "indexer response too large: {cl} bytes (max {MAX_RESPONSE_BODY_BYTES})"
            ));
        }
    }

    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| e.without_url().to_string())?
    {
        if buf.len() + chunk.len() > MAX_RESPONSE_BODY_BYTES {
            return Err(format!(
                "indexer response exceeded {MAX_RESPONSE_BODY_BYTES} bytes"
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    let xml = String::from_utf8(buf).map_err(|e| format!("invalid UTF-8 in response: {e}"))?;

    match parse_torznab_xml(xml.as_bytes())? {
        TorznabParseResult::Items(items) => {
            let results = items
                .into_iter()
                .filter(|item| !item.guid.is_empty() && !item.download_url.is_empty())
                .map(|item| {
                    let protocol = if item
                        .enclosure_type
                        .as_deref()
                        .is_some_and(|t| t.contains("nzb"))
                    {
                        "usenet"
                    } else {
                        "torrent"
                    }
                    .to_string();
                    ReleaseResponse {
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
            Ok(results)
        }
        TorznabParseResult::Error { code, description } => {
            Err(format!("Torznab error {code}: {description}"))
        }
    }
}

pub(crate) fn qbit_base_url(client: &livrarr_domain::DownloadClient) -> String {
    if client.host.starts_with("http://") || client.host.starts_with("https://") {
        let url_base = client.url_base.as_deref().unwrap_or("");
        return format!("{}{url_base}", client.host.trim_end_matches('/'));
    }

    let scheme = if client.use_ssl { "https" } else { "http" };
    let url_base = client.url_base.as_deref().unwrap_or("");
    if client.port == 80 || client.port == 443 {
        format!("{scheme}://{}{url_base}", client.host)
    } else {
        format!("{scheme}://{}:{}{url_base}", client.host, client.port)
    }
}

pub(crate) async fn qbit_login(
    state: &AppState,
    base_url: &str,
    client: &livrarr_domain::DownloadClient,
) -> Result<String, ApiError> {
    let username = client.username.as_deref().unwrap_or("");
    let password = client.password.as_deref().unwrap_or("");

    if username.is_empty() && password.is_empty() {
        return Ok(String::new());
    }

    let login_url = format!("{base_url}/api/v2/auth/login");
    let resp = state
        .http_client
        .post(&login_url)
        .form(&[("username", username), ("password", password)])
        .send()
        .await
        .map_err(|e| ApiError::BadGateway(format!("qBittorrent login failed: {e}")))?;

    let sid = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .find_map(|v| {
            let s = v.to_str().ok()?;
            s.split(';')
                .find(|part| part.trim().starts_with("SID="))
                .map(|part| part.trim().trim_start_matches("SID=").to_string())
        })
        .unwrap_or_default();

    let body = resp.text().await.unwrap_or_default();
    if body.contains("Fails") {
        return Err(ApiError::BadGateway(
            "qBittorrent authentication failed".into(),
        ));
    }

    Ok(sid)
}
