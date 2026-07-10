use livrarr_domain::services::{
    is_known_dead_host, mark_cover_host_dead, FetchRequest, HttpFetcher, HttpMethod, RateBucket,
    UserAgentProfile,
};
use livrarr_domain::RequestPriority;

use livrarr_external_data::provider_util::upscale_cover_url;

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

/// GET `url` on `bucket` via the queue-routed, SSRF-safe fetcher. `None` on
/// any transport error or a non-2xx status — the silent-None style the
/// direct-`HttpClient` callers this replaces already used.
async fn get_ok<F: HttpFetcher>(
    fetcher: &F,
    url: &str,
    bucket: RateBucket,
    priority: RequestPriority,
) -> Option<livrarr_domain::services::FetchResponse> {
    let req = FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout: std::time::Duration::from_secs(30),
        rate_bucket: bucket,
        max_body_bytes: 10 * 1024 * 1024,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority,
    };
    let resp = fetcher.fetch_ssrf_safe(req).await.ok()?;
    (200..300).contains(&resp.status).then_some(resp)
}

/// Case-insensitive `Content-Length` lookup.
fn declared_content_length(headers: &[(String, String)]) -> Option<usize> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
}

/// Amazon/CdL return a tiny placeholder image for a missing cover — filter it
/// out. Prefer the declared `Content-Length`; if the header is absent, fall
/// back to the actual downloaded body length (available now that the
/// queue-routed fetcher always reads the full body, unlike the lazy
/// `reqwest::Response` this replaces). Either way: tiny = placeholder = filtered.
fn passes_placeholder_filter(resp: &livrarr_domain::services::FetchResponse) -> bool {
    let len = declared_content_length(&resp.headers).unwrap_or(resp.body.len());
    len > 1000
}

/// Attempt to resolve a cover image URL from an ISBN using OpenLibrary.
/// Used for English works only.
pub async fn resolve_cover_by_isbn_ol<F: HttpFetcher>(
    fetcher: &F,
    isbn: Option<&str>,
    priority: RequestPriority,
) -> Option<String> {
    let isbn = isbn?;
    if !is_valid_isbn(isbn) {
        return None;
    }

    let ol_url = format!("https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg?default=false");
    if get_ok(fetcher, &ol_url, RateBucket::OpenLibraryCovers, priority)
        .await
        .is_some()
    {
        return Some(format!(
            "https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg",
        ));
    }

    None
}

/// Resolve a cover using the Amazon direct ISBN-to-image URL.
/// No API key, no scraping — just URL construction from ISBN-10.
/// Returns the URL directly (Amazon returns 200 for valid ISBNs with covers,
/// or a 1x1 pixel for missing ones — we check Content-Length to filter).
pub async fn resolve_cover_by_isbn_amazon<F: HttpFetcher>(
    fetcher: &F,
    isbn: Option<&str>,
    priority: RequestPriority,
) -> Option<String> {
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

    if let Some(resp) = get_ok(fetcher, &url, RateBucket::None, priority).await {
        if passes_placeholder_filter(&resp) {
            return Some(url);
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
pub async fn resolve_cover_by_isbn_casadellibro<F: HttpFetcher>(
    fetcher: &F,
    isbn: Option<&str>,
    priority: RequestPriority,
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
    if let Some(resp) = get_ok(fetcher, &url, RateBucket::None, priority).await {
        if passes_placeholder_filter(&resp) {
            return Some(url);
        }
    }
    None
}

/// Resolve cover for foreign (non-English) works.
/// Chain: Casa del Libro ISBN → Amazon ISBN → nothing.
/// CdL covers are proxied through /api/v1/coverproxy to bypass their CDN browser-blocking.
pub async fn resolve_cover_foreign<F: HttpFetcher>(
    fetcher: &F,
    isbn: Option<&str>,
    priority: RequestPriority,
) -> Option<String> {
    if let Some(url) = resolve_cover_by_isbn_casadellibro(fetcher, isbn, priority).await {
        return Some(url);
    }
    resolve_cover_by_isbn_amazon(fetcher, isbn, priority).await
}

/// Resolve cover for English works (existing behavior).
/// Chain: OL ISBN → Amazon ISBN.
pub async fn resolve_cover_english<F: HttpFetcher>(
    fetcher: &F,
    isbn: Option<&str>,
    priority: RequestPriority,
) -> Option<String> {
    if let Some(url) = resolve_cover_by_isbn_ol(fetcher, isbn, priority).await {
        return Some(url);
    }
    resolve_cover_by_isbn_amazon(fetcher, isbn, priority).await
}

// =============================================================================
// Phase 1: synchronous cover acquisition (3s budget)
// =============================================================================

use std::time::Duration;

fn classify_cover_url(url: &str) -> &'static str {
    if url.contains("hardcover.app") || url.contains("assets.hardcover") {
        "hardcover"
    } else if url.contains("goodreads.com") || url.contains("gr-assets.com") {
        "goodreads"
    } else {
        "other"
    }
}

/// Lower-cased host for the phase1 negative cache. `None` on an unparseable
/// URL — `valid_url` already ran it through `validate_cover_url`, so this is
/// only reachable for a URL whose host `url::Url` itself can't extract
/// (host-less scheme edge cases); the cache is simply not consulted for it.
fn cover_url_host(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_lowercase))
}

/// Try to get a cover on disk within 3 seconds. Returns the cover file mtime on success.
///
/// Branch A: download existing URL immediately (any host, with GR upscaling).
/// Branch B: HC search only when no URL was provided (recovery path).
#[allow(clippy::too_many_arguments)]
pub async fn fetch_phase1_cover<H: HttpFetcher>(
    http_fetcher: &H,
    title: &str,
    author: &str,
    request_cover_url: Option<&str>,
    hc_token: Option<&str>,
    covers_dir: &std::path::Path,
    work_id: i64,
) -> Option<i64> {
    let start = tokio::time::Instant::now();
    let deadline = start + Duration::from_secs(3);

    let unproxied = request_cover_url.map(livrarr_domain::unproxy_cover_url);
    let valid_url = unproxied
        .as_deref()
        .filter(|u| livrarr_external_data::provider_util::validate_cover_url(u, "").is_some());

    // Branch A: download existing URL directly (any provider)
    if let Some(url) = valid_url {
        let source = classify_cover_url(url);
        let host = cover_url_host(url);

        // The same dead host tends to repeat across a whole import batch
        // (books from one source share a host) — once this run has seen this
        // host fail to connect, later books skip straight to a miss instead
        // of burning their own 3s budget on a doomed connect.
        if let Some(h) = host.as_deref() {
            if is_known_dead_host(h) {
                tracing::debug!(
                    work_id,
                    host = h,
                    "phase1 cover skip: host failed to connect earlier this run"
                );
                return None;
            }
        }

        let preferred = upscale_cover_url(url);
        let preferred_changed = preferred != url;
        let mut saw_connect_failure = false;

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining > Duration::from_millis(200) {
            let result = tokio::time::timeout(
                remaining,
                livrarr_materialize::download_cover_to_disk(
                    http_fetcher,
                    &preferred,
                    covers_dir,
                    work_id,
                    "",
                    RequestPriority::High,
                    true,
                ),
            )
            .await;

            if matches!(result, Ok(Ok(_))) {
                tracing::info!(
                    work_id,
                    source,
                    elapsed_ms = start.elapsed().as_millis() as u64,
                    "phase1 cover acquired"
                );
                return cover_file_mtime(covers_dir, work_id);
            }
            if let Ok(Err(e)) = &result {
                saw_connect_failure |= e.is_connect_failure();
            }
        }

        if preferred_changed {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining > Duration::from_millis(200) {
                let result = tokio::time::timeout(
                    remaining,
                    livrarr_materialize::download_cover_to_disk(
                        http_fetcher,
                        url,
                        covers_dir,
                        work_id,
                        "",
                        RequestPriority::High,
                        true,
                    ),
                )
                .await;

                if matches!(result, Ok(Ok(_))) {
                    tracing::info!(
                        work_id,
                        source = "request_url_fallback",
                        elapsed_ms = start.elapsed().as_millis() as u64,
                        "phase1 cover acquired (original URL fallback)"
                    );
                    return cover_file_mtime(covers_dir, work_id);
                }
                if let Ok(Err(e)) = &result {
                    saw_connect_failure |= e.is_connect_failure();
                }
            }
        }

        // Only a transport-level connect/timeout failure marks the host dead
        // (DownloadCoverError::is_connect_failure) — a 404 or a decode
        // rejection means the host answered fine and must not poison a later
        // book that has a working cover URL on the same host.
        if saw_connect_failure {
            if let Some(h) = host {
                mark_cover_host_dead(&h);
            }
        }

        tracing::info!(
            work_id,
            elapsed_ms = start.elapsed().as_millis() as u64,
            "phase1 cover miss (direct download failed)"
        );
        return None;
    }

    // Branch B: no URL provided — HC search as recovery. Not the direct-
    // download leg the negative host cache targets (a batch-wide dead host
    // comes from a per-book embedded URL, not HC's own asset host), so this
    // download keeps the normal connect budget and skips the cache.
    if let Some(token) = hc_token {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining > Duration::from_millis(500) {
            let hc_timeout = remaining.min(Duration::from_secs(2));
            match tokio::time::timeout(
                hc_timeout,
                fast_hc_cover_search(http_fetcher, title, author, token, RequestPriority::High),
            )
            .await
            {
                Ok(Ok(Some(hc_url))) => {
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    if remaining > Duration::from_millis(200)
                        && matches!(
                            tokio::time::timeout(
                                remaining,
                                livrarr_materialize::download_cover_to_disk(
                                    http_fetcher,
                                    &hc_url,
                                    covers_dir,
                                    work_id,
                                    "",
                                    RequestPriority::High,
                                    false,
                                ),
                            )
                            .await,
                            Ok(Ok(_))
                        )
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

    tracing::info!(
        work_id,
        elapsed_ms = start.elapsed().as_millis() as u64,
        "phase1 cover miss"
    );
    None
}

async fn fast_hc_cover_search<F: HttpFetcher>(
    fetcher: &F,
    title: &str,
    author: &str,
    token: &str,
    priority: RequestPriority,
) -> Result<Option<String>, Box<dyn std::error::Error + Send + Sync>> {
    use livrarr_external_data::hardcover::{hc_extract_hits, hc_post, hc_search_body};

    let clean_title = title
        .rfind('(')
        .filter(|_| title.ends_with(')'))
        .map(|i| title[..i].trim())
        .unwrap_or(title);
    let body = hc_search_body(10, &format!("\"{clean_title}\""));
    let data = hc_post(fetcher, body, token, priority).await?;
    let hits = hc_extract_hits(&data);

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
    cover_file_mtime_with_suffix(covers_dir, work_id, "")
}

pub fn audiobook_cover_file_mtime(covers_dir: &std::path::Path, work_id: i64) -> Option<i64> {
    cover_file_mtime_with_suffix(covers_dir, work_id, "_audio")
}

fn cover_file_mtime_with_suffix(
    covers_dir: &std::path::Path,
    work_id: i64,
    suffix: &str,
) -> Option<i64> {
    let path = covers_dir.join(format!("{work_id}{suffix}.jpg"));
    std::fs::metadata(&path)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64
        })
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
        assert!(!is_valid_isbn("978-0-306-40615-7"));
        assert!(!is_valid_isbn("978030640615X7"));
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
        assert_eq!(
            isbn13_to_isbn10("9780306406157"),
            Some("0306406152".to_string())
        );
        assert_eq!(
            isbn13_to_isbn10("9780120000302"),
            Some("012000030X".to_string())
        );
    }

    #[test]
    fn isbn13_to_isbn10_invalid() {
        assert_eq!(isbn13_to_isbn10("1234567890"), None);
        assert_eq!(isbn13_to_isbn10("9791234567890"), None);
    }
}

/// Behavioral coverage for the dead-host fast-fail: (a) a connect failure
/// marks the host and a later book on that host skips its fetch, (b) a live
/// URL is unaffected, (c) the cache never leaks across separate runs or
/// outside any run scope, and (d) only a connect-class failure marks a host
/// — a plain 404 from an otherwise-live host must not poison it.
#[cfg(test)]
mod phase1_fast_fail_tests {
    use super::*;
    use livrarr_domain::services::{with_cover_host_cache, FetchError, FetchResponse};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fetcher whose `fetch_ssrf_safe` answers per-host: hosts in
    /// `dead_hosts` always fail with a connect-class `FetchError`; URLs in
    /// `not_found_urls` return a live 404; everything else returns a 200.
    /// Body bytes are never a real JPEG — `download_cover_to_disk` tolerates
    /// that (dims come back `None`, non-fatal); only the status/error matters
    /// for this test's assertions.
    struct StubFetcher {
        dead_hosts: Vec<String>,
        not_found_urls: Vec<String>,
        call_count: AtomicUsize,
    }

    impl StubFetcher {
        fn new(dead_hosts: &[&str]) -> Self {
            Self {
                dead_hosts: dead_hosts.iter().map(|s| s.to_string()).collect(),
                not_found_urls: Vec::new(),
                call_count: AtomicUsize::new(0),
            }
        }

        fn with_not_found(not_found_urls: &[&str]) -> Self {
            Self {
                dead_hosts: Vec::new(),
                not_found_urls: not_found_urls.iter().map(|s| s.to_string()).collect(),
                call_count: AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    impl HttpFetcher for StubFetcher {
        async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
            self.fetch_ssrf_safe(req).await
        }

        async fn fetch_ssrf_safe(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            let host = cover_url_host(&req.url);
            if host
                .as_deref()
                .map(|h| self.dead_hosts.iter().any(|d| d == h))
                .unwrap_or(false)
            {
                return Err(FetchError::Connection("simulated dead host".into()));
            }
            if self.not_found_urls.contains(&req.url) {
                return Ok(FetchResponse {
                    status: 404,
                    headers: vec![],
                    body: vec![],
                });
            }
            Ok(FetchResponse {
                status: 200,
                headers: vec![],
                body: b"fake-cover-bytes".to_vec(),
            })
        }
    }

    #[tokio::test]
    async fn dead_host_failure_marks_host_and_second_book_with_same_host_skips_fetch() {
        let stub = StubFetcher::new(&["dead.example.com"]);
        let dir = tempfile::tempdir().expect("tempdir");

        with_cover_host_cache(async {
            let r1 = fetch_phase1_cover(
                &stub,
                "Book One",
                "Author",
                Some("https://dead.example.com/cover1.jpg"),
                None,
                dir.path(),
                101,
            )
            .await;
            assert_eq!(r1, None, "dead host: phase1 must report a miss");
            assert_eq!(
                stub.calls(),
                1,
                "book 1 attempts exactly once (no GR/HC upscaling for this host, so no 2nd attempt)"
            );

            let r2 = fetch_phase1_cover(
                &stub,
                "Book Two",
                "Author",
                Some("https://dead.example.com/cover2.jpg"),
                None,
                dir.path(),
                102,
            )
            .await;
            assert_eq!(r2, None);
            assert_eq!(
                stub.calls(),
                1,
                "book 2 must skip the fetch entirely — the host is already known dead"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn live_url_fetch_is_unaffected() {
        let stub = StubFetcher::new(&["dead.example.com"]);
        let dir = tempfile::tempdir().expect("tempdir");

        with_cover_host_cache(async {
            let r = fetch_phase1_cover(
                &stub,
                "Live Book",
                "Author",
                Some("https://good.example.com/cover.jpg"),
                None,
                dir.path(),
                201,
            )
            .await;
            assert!(r.is_some(), "a live host must still acquire a cover");
            assert_eq!(stub.calls(), 1);
        })
        .await;
    }

    #[tokio::test]
    async fn dead_host_cache_does_not_leak_across_separate_runs() {
        let stub = StubFetcher::new(&["dead.example.com"]);
        let dir = tempfile::tempdir().expect("tempdir");

        with_cover_host_cache(async {
            let _ = fetch_phase1_cover(
                &stub,
                "Run1 Book",
                "Author",
                Some("https://dead.example.com/c1.jpg"),
                None,
                dir.path(),
                301,
            )
            .await;
        })
        .await;
        assert_eq!(stub.calls(), 1, "run 1 attempted once");

        with_cover_host_cache(async {
            let _ = fetch_phase1_cover(
                &stub,
                "Run2 Book",
                "Author",
                Some("https://dead.example.com/c2.jpg"),
                None,
                dir.path(),
                302,
            )
            .await;
        })
        .await;
        assert_eq!(
            stub.calls(),
            2,
            "a fresh run must attempt again — no cross-run leak"
        );
    }

    #[tokio::test]
    async fn without_any_run_scope_the_check_is_a_noop_every_book_attempts() {
        // Every WorkService::add caller other than manual import (direct add,
        // list import, Readarr import, author monitor, background retry)
        // never wraps itself in with_cover_host_cache — must never panic,
        // must always attempt the fetch (fail-open).
        let stub = StubFetcher::new(&["dead.example.com"]);
        let dir = tempfile::tempdir().expect("tempdir");

        let _ = fetch_phase1_cover(
            &stub,
            "No Scope 1",
            "Author",
            Some("https://dead.example.com/c1.jpg"),
            None,
            dir.path(),
            401,
        )
        .await;
        let _ = fetch_phase1_cover(
            &stub,
            "No Scope 2",
            "Author",
            Some("https://dead.example.com/c2.jpg"),
            None,
            dir.path(),
            402,
        )
        .await;
        assert_eq!(
            stub.calls(),
            2,
            "no active run scope — no caching, both books attempted"
        );
    }

    #[tokio::test]
    async fn a_404_on_one_book_does_not_poison_a_later_book_on_the_same_host() {
        // The host is live (it answers with a real 404, not a connect
        // failure) — a later book with a working URL on that SAME host must
        // still be attempted, not silently skipped.
        let stub = StubFetcher::with_not_found(&["https://flaky.example.com/missing.jpg"]);
        let dir = tempfile::tempdir().expect("tempdir");

        with_cover_host_cache(async {
            let r1 = fetch_phase1_cover(
                &stub,
                "Missing Cover",
                "Author",
                Some("https://flaky.example.com/missing.jpg"),
                None,
                dir.path(),
                501,
            )
            .await;
            assert_eq!(r1, None);

            let r2 = fetch_phase1_cover(
                &stub,
                "Working Cover",
                "Author",
                Some("https://flaky.example.com/present.jpg"),
                None,
                dir.path(),
                502,
            )
            .await;
            assert!(
                r2.is_some(),
                "same host, different URL that works — must not have been skipped"
            );
            assert_eq!(
                stub.calls(),
                2,
                "both books attempted — the 404 never marked the host dead"
            );
        })
        .await;
    }
}
