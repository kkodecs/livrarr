//! OpenLibrary REST client (work-detail enrichment).
//!
//! Lifted out of `livrarr-server/src/handlers/enrichment.rs` so the same code
//! can serve the legacy direct path AND `ProviderClient::OpenLibrary` behind
//! `DefaultProviderQueue`. Behavior unchanged from the original.

use livrarr_http::HttpClient;

/// Parsed subset of an OpenLibrary work detail + first edition with ISBN.
#[derive(Debug, Clone)]
pub struct OlDetailResult {
    pub description: Option<String>,
    pub isbn_13: Option<String>,
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

    let description = data.get("description").and_then(|d| {
        d.as_str().map(|s| s.to_string()).or_else(|| {
            d.get("value")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
    });

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
        description,
        isbn_13,
    })
}
