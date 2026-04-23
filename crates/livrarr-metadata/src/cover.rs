use livrarr_http::HttpClient;

/// Validate that an ISBN string contains only digits and an optional trailing X.
/// Rejects any input that could be used for URL injection.
fn is_valid_isbn(isbn: &str) -> bool {
    if isbn.is_empty() {
        return false;
    }
    let bytes = isbn.as_bytes();
    let (body, last) = bytes.split_at(bytes.len() - 1);
    body.iter().all(|b| b.is_ascii_digit())
        && (last[0].is_ascii_digit() || last[0] == b'X' || last[0] == b'x')
}

/// Attempt to resolve a cover image URL from an ISBN using OpenLibrary.
/// Used for English works only.
pub async fn resolve_cover_by_isbn_ol(http: &HttpClient, isbn: Option<&str>) -> Option<String> {
    let isbn = isbn?;
    if !is_valid_isbn(isbn) {
        return None;
    }

    let ol_url = format!("https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg?default=false");
    if let Ok(resp) = http.get(&ol_url).send().await {
        if resp.status().is_success() {
            return Some(format!(
                "https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg",
            ));
        }
    }

    None
}

/// Resolve a cover using the Amazon direct ISBN-to-image URL.
/// No API key, no scraping — just URL construction from ISBN-10.
/// Returns the URL directly (Amazon returns 200 for valid ISBNs with covers,
/// or a 1x1 pixel for missing ones — we check Content-Length to filter).
pub async fn resolve_cover_by_isbn_amazon(http: &HttpClient, isbn: Option<&str>) -> Option<String> {
    let isbn = isbn?;
    if !is_valid_isbn(isbn) {
        return None;
    }

    // Amazon needs ISBN-10. Convert ISBN-13 if needed.
    let isbn10 = if isbn.len() == 13 && isbn.starts_with("978") {
        isbn13_to_isbn10(isbn)?
    } else if isbn.len() == 10 {
        isbn.to_string()
    } else {
        return None;
    };

    let url =
        format!("https://images-na.ssl-images-amazon.com/images/P/{isbn10}.01._SCLZZZZZZZ_.jpg");

    if let Ok(resp) = http.get(&url).send().await {
        if resp.status().is_success() {
            // Amazon returns a tiny 1x1 GIF for missing covers — filter by Content-Length.
            // Real covers are >1KB.
            if let Some(len) = resp.content_length() {
                if len > 1000 {
                    return Some(url);
                }
            } else {
                // No Content-Length header — accept it (could be chunked)
                return Some(url);
            }
        }
    }

    None
}

/// Convert ISBN-13 (starting with 978) to ISBN-10.
fn isbn13_to_isbn10(isbn13: &str) -> Option<String> {
    if isbn13.len() != 13 || !isbn13.starts_with("978") {
        return None;
    }
    let body = &isbn13[3..12]; // 9 digits after "978", before check digit
    let sum: u32 = body
        .chars()
        .enumerate()
        .filter_map(|(i, c)| c.to_digit(10).map(|d| d * (10 - i as u32)))
        .sum();
    let check = (11 - (sum % 11)) % 11;
    let check_char = if check == 10 {
        'X'
    } else {
        char::from_digit(check, 10)?
    };
    Some(format!("{body}{check_char}"))
}

/// Resolve a cover using Casa del Libro's predictable ISBN-to-URL pattern.
/// Works for Spanish ISBNs (978-84-...) with very high hit rate.
pub async fn resolve_cover_by_isbn_casadellibro(
    http: &HttpClient,
    isbn: Option<&str>,
) -> Option<String> {
    let isbn = isbn?;
    if !is_valid_isbn(isbn) {
        return None;
    }
    let clean: String = isbn.chars().filter(|c| c.is_ascii_digit()).collect();
    if clean.len() != 13 {
        return None;
    }
    let last2 = &clean[11..13];
    let n = (clean.as_bytes()[12] - b'0') % 10; // last digit mod 10
    let url = format!("https://imagessl{n}.casadellibro.com/a/l/s5/{last2}/{clean}.webp");
    if let Ok(resp) = http.get(&url).send().await {
        if resp.status().is_success() {
            if let Some(len) = resp.content_length() {
                if len > 1000 {
                    return Some(url);
                }
            } else {
                return Some(url);
            }
        }
    }
    None
}

/// Resolve cover for foreign (non-English) works.
/// Chain: Casa del Libro ISBN → Amazon ISBN → nothing.
/// CdL covers are proxied through /api/v1/coverproxy to bypass their CDN browser-blocking.
pub async fn resolve_cover_foreign(http: &HttpClient, isbn: Option<&str>) -> Option<String> {
    if let Some(url) = resolve_cover_by_isbn_casadellibro(http, isbn).await {
        return Some(url);
    }
    resolve_cover_by_isbn_amazon(http, isbn).await
}

/// Resolve cover for English works (existing behavior).
/// Chain: OL ISBN → Amazon ISBN.
pub async fn resolve_cover_english(http: &HttpClient, isbn: Option<&str>) -> Option<String> {
    if let Some(url) = resolve_cover_by_isbn_ol(http, isbn).await {
        return Some(url);
    }
    resolve_cover_by_isbn_amazon(http, isbn).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isbn_validation() {
        assert!(is_valid_isbn("9780306406157"));
        assert!(is_valid_isbn("012000030X"));
        assert!(is_valid_isbn("012000030x"));
        assert!(!is_valid_isbn(""));
        assert!(!is_valid_isbn("978-0-306-40615-7")); // hyphens rejected
        assert!(!is_valid_isbn("978030640615X7")); // X not at end
        assert!(!is_valid_isbn("../../../etc/passwd"));
        assert!(!is_valid_isbn("9780306406157&extra=inject"));
    }

    #[test]
    fn isbn13_to_isbn10_valid() {
        assert_eq!(
            isbn13_to_isbn10("9782070612758"),
            Some("2070612759".to_string())
        );
        assert_eq!(
            isbn13_to_isbn10("9783522202022"),
            Some("3522202023".to_string())
        );
    }

    #[test]
    fn isbn13_to_isbn10_with_x_check() {
        // ISBN-13 9780306406157 → ISBN-10 0306406152
        assert_eq!(
            isbn13_to_isbn10("9780306406157"),
            Some("0306406152".to_string())
        );
        // ISBN-13 9780120000302 → ISBN-10 012000030X (check digit = 10 → X)
        assert_eq!(
            isbn13_to_isbn10("9780120000302"),
            Some("012000030X".to_string())
        );
    }

    #[test]
    fn isbn13_to_isbn10_invalid() {
        assert_eq!(isbn13_to_isbn10("1234567890"), None); // too short
        assert_eq!(isbn13_to_isbn10("9791234567890"), None); // 979 prefix
    }
}

// =============================================================================
// Phase 1: synchronous cover acquisition (3s budget)
// =============================================================================

use std::time::Duration;

use livrarr_domain::services::HttpFetcher;

fn classify_cover_url(url: &str) -> &'static str {
    if url.contains("hardcover.app") || url.contains("assets.hardcover") {
        "hardcover"
    } else if url.contains("goodreads.com") || url.contains("gr-assets.com") {
        "goodreads"
    } else {
        "other"
    }
}

/// Try to get a cover on disk within 3 seconds. Returns the cover file mtime on success.
///
/// Strategy: try HC GraphQL search first (2s), fall back to request cover URL.
/// Only one download runs — no concurrent writes.
pub async fn fetch_phase1_cover<H: HttpFetcher>(
    http_fetcher: &H,
    hc_http: &HttpClient,
    title: &str,
    author: &str,
    request_cover_url: Option<&str>,
    hc_token: Option<&str>,
    covers_dir: &std::path::Path,
    work_id: i64,
) -> Option<i64> {
    let start = tokio::time::Instant::now();
    let deadline = start + Duration::from_secs(3);

    let unproxied = request_cover_url.map(crate::work_service::unproxy_cover_url);
    let valid_url = unproxied
        .as_deref()
        .filter(|u| crate::llm_scraper::validate_cover_url(u, "").is_some());

    // If request already has an HC/GR cover, download directly (it's already good)
    if let Some(url) = valid_url {
        let source = classify_cover_url(url);
        if source == "hardcover" || source == "goodreads" {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if tokio::time::timeout(
                remaining,
                crate::work_service::download_cover_to_disk(
                    http_fetcher,
                    url,
                    covers_dir,
                    work_id,
                    "",
                ),
            )
            .await
            .is_ok()
            {
                tracing::info!(
                    work_id,
                    source,
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    "phase1 cover acquired"
                );
                return cover_file_mtime(covers_dir, work_id);
            }
        }
    }

    // Try HC search (Tier 1 only, no LLM)
    if let Some(token) = hc_token {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining > Duration::from_millis(500) {
            let hc_timeout = remaining.min(Duration::from_secs(2));
            match tokio::time::timeout(
                hc_timeout,
                fast_hc_cover_search(hc_http, title, author, token),
            )
            .await
            {
                Ok(Ok(Some(hc_url))) => {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining > Duration::from_millis(200) {
                        if tokio::time::timeout(
                            remaining,
                            crate::work_service::download_cover_to_disk(
                                http_fetcher,
                                &hc_url,
                                covers_dir,
                                work_id,
                                "",
                            ),
                        )
                        .await
                        .is_ok()
                        {
                            tracing::info!(
                                work_id,
                                source = "hardcover",
                                elapsed_ms = start.elapsed().as_millis() as u64,
                                "phase1 cover acquired"
                            );
                            return cover_file_mtime(covers_dir, work_id);
                        }
                    }
                }
                Ok(Ok(None)) => {}
                Ok(Err(e)) => {
                    tracing::debug!(work_id, "phase1 HC search failed: {e}");
                }
                Err(_) => {
                    tracing::debug!(work_id, "phase1 HC search timed out");
                }
            }
        }
    }

    // Fall back to request cover URL (OL or whatever the lookup returned)
    if let Some(url) = valid_url {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining > Duration::from_millis(200) {
            if tokio::time::timeout(
                remaining,
                crate::work_service::download_cover_to_disk(
                    http_fetcher,
                    url,
                    covers_dir,
                    work_id,
                    "",
                ),
            )
            .await
            .is_ok()
            {
                tracing::info!(
                    work_id,
                    source = "request_url",
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    "phase1 cover acquired"
                );
                return cover_file_mtime(covers_dir, work_id);
            }
        }
    }

    tracing::info!(
        work_id,
        elapsed_ms = start.elapsed().as_millis() as u64,
        "phase1 cover miss"
    );
    None
}

async fn fast_hc_cover_search(
    http: &HttpClient,
    title: &str,
    author: &str,
    token: &str,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    use crate::hardcover::HARDCOVER_API_URL;

    let query = r#"query SearchBooks($query: String!) {
        search(query: $query, query_type: "books", per_page: 10) { results }
    }"#;

    let clean_title = title
        .rfind('(')
        .filter(|_| title.ends_with(')'))
        .map(|i| title[..i].trim())
        .unwrap_or(title);
    let body = serde_json::json!({
        "query": query,
        "variables": {"query": format!("\"{clean_title}\"")}
    });

    let resp = http
        .post(HARDCOVER_API_URL)
        .header("Authorization", format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(format!("HC HTTP {}", resp.status()).into());
    }

    let data: serde_json::Value = resp.json().await?;
    let hits = data
        .pointer("/data/search/results/hits")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let title_lower = clean_title.trim().to_lowercase();
    let author_lower = author.trim().to_lowercase();
    let mut best_cover: Option<String> = None;
    let mut best_urc: i64 = -1;

    for hit in &hits {
        let doc = match hit.get("document") {
            Some(d) => d,
            None => continue,
        };
        let doc_title = doc
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim()
            .to_lowercase();
        if doc_title != title_lower {
            continue;
        }
        let authors: Vec<String> = doc
            .get("author_names")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.trim().to_lowercase())
                    .collect()
            })
            .unwrap_or_default();
        if !authors.iter().any(|a| a == &author_lower) {
            continue;
        }
        let urc = doc
            .get("users_read_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        if urc > best_urc {
            best_urc = urc;
            best_cover = doc
                .pointer("/image/url")
                .and_then(|v| v.as_str())
                .map(String::from);
        }
    }

    Ok(best_cover)
}

pub fn cover_file_mtime(covers_dir: &std::path::Path, work_id: i64) -> Option<i64> {
    let path = covers_dir.join(format!("{work_id}.jpg"));
    std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        })
}
