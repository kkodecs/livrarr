//! OpenLibrary REST client, consumed via `ProviderClient::OpenLibrary` (queue
//! dispatch and the identity-resolution fan-out) and by the discovery path
//! (`search_openlibrary`).

use livrarr_domain::seed::iso639_1_to_3;
use livrarr_domain::services::{
    FetchRequest, HttpFetcher, HttpMethod, LookupResult, RateBucket, UserAgentProfile,
};
use livrarr_domain::RequestPriority;
use livrarr_http::HttpClient;

/// Parsed subset of an OpenLibrary work detail + first edition with ISBN.
#[derive(Debug, Clone)]
pub struct OlDetailResult {
    /// The work's title — identity arbitration clusters responders by
    /// title/author when no shared key exists, so a title-less payload is
    /// unclusterable and its ol_key gets discarded (#148).
    pub title: Option<String>,
    pub description: Option<String>,
    pub isbn_13: Option<String>,
    pub cover_id: Option<i64>,
}

/// Fetch work detail + first edition ISBN for an OpenLibrary work key.
///
/// `ol_key` accepts either bare keys (`OL12345W`) or path-prefixed forms
/// (`/works/OL12345W`).
pub async fn query_ol_detail(http: &HttpClient, ol_key: &str) -> Result<OlDetailResult, String> {
    let key = ol_key.trim_start_matches("/works/").trim_start_matches('/');

    let url = format!("https://openlibrary.org/works/{key}.json");
    let resp = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let data: serde_json::Value = resp.json().await.map_err(|e| format!("parse: {e}"))?;

    let title = data
        .get("title")
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let description = data.get("description").and_then(|d| {
        d.as_str().map(|s| s.to_string()).or_else(|| {
            d.get("value")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
    });

    let cover_id = data
        .get("covers")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.iter().find_map(|v| v.as_i64()))
        .filter(|&id| id > 0);

    // Fetch editions for ISBN.
    let mut isbn_13 = None;
    let editions_url = format!("https://openlibrary.org/works/{key}/editions.json?limit=10");
    if let Ok(ed_resp) = http.get(&editions_url).send().await {
        if let Ok(ed_data) = ed_resp.json::<serde_json::Value>().await {
            if let Some(entries) = ed_data.get("entries").and_then(|e| e.as_array()) {
                for entry in entries {
                    if let Some(isbns) = entry.get("isbn_13").and_then(|i| i.as_array()) {
                        if let Some(isbn) = isbns.first().and_then(|v| v.as_str()) {
                            isbn_13 = Some(isbn.to_string());
                            break;
                        }
                    }
                }
            }
        }
    }

    Ok(OlDetailResult {
        title,
        description,
        isbn_13,
        cover_id,
    })
}

/// Search OpenLibrary `search.json` for books matching `term`.
///
/// `lang` is the ISO 639-1 language code (e.g. `"en"`, `"fr"`); for non-English
/// searches an `&language=` filter is appended. Returns one [`LookupResult`] per
/// document — OL key, title, first author, first-publish year, and cover URL.
///
/// Extracted from `work_service::lookup_openlibrary` (M-004 / Phase 2 dedup).
/// The `provider_client.rs` title+author search that returns [`NormalizedWorkDetail`]
/// for enrichment is a separate shape and must stay separate.
pub async fn search_openlibrary<H: HttpFetcher + Send + Sync>(
    http: &H,
    term: &str,
    lang: &str,
) -> Result<Vec<LookupResult>, String> {
    let lang_param = if lang != "en" {
        let ol_lang = iso639_1_to_3(lang);
        format!("&language={}", urlencoding::encode(ol_lang))
    } else {
        String::new()
    };
    let url = format!(
        "https://openlibrary.org/search.json?q={}&limit=50&fields=key,title,author_name,author_key,first_publish_year,cover_i{lang_param}",
        urlencoding::encode(term)
    );

    let fetch_req = FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: std::time::Duration::from_secs(10),
        rate_bucket: RateBucket::OpenLibrary,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority: RequestPriority::Normal,
    };

    let resp = http
        .fetch(fetch_req)
        .await
        .map_err(|e| format!("OpenLibrary request failed: {e}"))?;

    if resp.status >= 400 {
        return Err(format!("OpenLibrary returned {}", resp.status));
    }

    let data: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("OpenLibrary parse error: {e}"))?;

    let docs = data
        .get("docs")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    let results = docs
        .iter()
        .filter_map(|doc| {
            let key = doc.get("key")?.as_str()?;
            let title = doc.get("title")?.as_str()?;
            let ol_key = key.trim_start_matches("/works/").to_string();

            let author_name = doc
                .get("author_name")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|a| a.as_str())
                .unwrap_or("Unknown")
                .to_string();

            let author_ol_key = doc
                .get("author_key")
                .and_then(|a| a.as_array())
                .and_then(|a| a.first())
                .and_then(|a| a.as_str())
                .map(|k| k.trim_start_matches("/authors/").to_string());

            let year = doc
                .get("first_publish_year")
                .and_then(|y| y.as_i64())
                .map(|y| y as i32);

            let cover_url = doc
                .get("cover_i")
                .and_then(|c| c.as_i64())
                .map(|c| format!("https://covers.openlibrary.org/b/id/{c}-L.jpg"));

            Some(LookupResult {
                ol_key: Some(ol_key),
                title: title.to_string(),
                author_name,
                author_ol_key,
                year,
                cover_url,
                description: None,
                series_name: None,
                series_position: None,
                source: Some("openlibrary".to_string()),
                source_type: Some("openlibrary".to_string()),
                language: Some(lang.to_string()),
                detail_url: None,
                rating: None,
                isbn_13: None,
                candidate_id: None,
                hc_key: None,
                gr_key: None,
                asin: None,
            })
        })
        .collect();

    Ok(results)
}
