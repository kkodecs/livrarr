use crate::state::AppState;
use crate::AuthorSearchResult;

/// Search OpenLibrary for authors by name. Reusable by handlers and manual import.
pub async fn lookup_ol_authors(
    http: &livrarr_http::HttpClient,
    term: &str,
    limit: u32,
) -> Result<Vec<AuthorSearchResult>, String> {
    let resp = http
        .get("https://openlibrary.org/search/authors.json")
        .query(&[("q", term), ("limit", &limit.to_string())])
        .send()
        .await
        .map_err(|e| format!("OpenLibrary request failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("OpenLibrary returned {}", resp.status()));
    }

    let data: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("OpenLibrary parse error: {e}"))?;

    let docs = data
        .get("docs")
        .and_then(|d| d.as_array())
        .cloned()
        .unwrap_or_default();

    Ok(docs
        .iter()
        .filter_map(|doc| {
            let key = doc.get("key")?.as_str()?;
            let name = doc.get("name")?.as_str()?;
            let ol_key = key.trim_start_matches("/authors/").to_string();

            Some(AuthorSearchResult {
                ol_key,
                name: name.to_string(),
                sort_name: None,
            })
        })
        .collect())
}

/// Spawn a background task to fetch and cache an author's bibliography.
pub fn spawn_bibliography_fetch(state: AppState, author_id: i64, user_id: i64) {
    use livrarr_domain::services::AuthorService;
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        match state
            .author_service
            .refresh_bibliography(user_id, author_id)
            .await
        {
            Ok(result) => {
                tracing::info!(
                    author_id,
                    entries = result.entries.len(),
                    "background bibliography fetch complete"
                );
            }
            Err(e) => {
                tracing::debug!(author_id, "background bibliography fetch skipped: {e}");
            }
        }
    });
}
