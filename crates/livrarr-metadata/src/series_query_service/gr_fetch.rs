use std::time::Duration;

use livrarr_db::{SeriesCacheEntry, SeriesRosterEntry};
use livrarr_domain::services::*;
use livrarr_domain::*;

async fn fetch_gr_html<F: HttpFetcher>(
    fetcher: &F,
    url: &str,
) -> Result<String, SeriesServiceError> {
    let req = FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers: vec![("Accept-Language".into(), "en-US,en;q=0.9".into())],
        body: None,
        timeout: Duration::from_secs(15),
        rate_bucket: RateBucket::Goodreads,
        max_body_bytes: 5 * 1024 * 1024,
        anti_bot_check: true,
        user_agent: UserAgentProfile::Browser,
        // Parked (B4 table): this one fn serves BOTH the background series
        // monitor AND the interactive roster-expand door — stays Normal
        // rather than picking a value that's wrong for one of the two.
        priority: RequestPriority::Normal,
    };
    let resp = fetcher
        .fetch(req)
        .await
        .map_err(|_| SeriesServiceError::GoodreadsUnavailable)?;
    if resp.status != 200 {
        return Err(SeriesServiceError::GoodreadsUnavailable);
    }
    String::from_utf8(resp.body).map_err(|_| SeriesServiceError::GoodreadsUnavailable)
}

/// Fetch + parse a GR series' detail pages (ST-008 road: paged, ≤10 pages,
/// 1s pacing) and keep the PRIMARY-works roster — the same set `work_count`
/// counts. On the 2026-07 React layout the page lists primaries FIRST and
/// the header states their count; omnibuses, split editions, and
/// translations follow (measured on series 108562 and 43318). No primary
/// count means the header drifted: return an empty roster (loud, never a
/// guess) rather than adopt GR's full 27-entry edition soup. Shared by the
/// monitor worker and the first-expand roster fetch (REQ-010).
pub(super) async fn fetch_series_roster_pages<F: HttpFetcher>(
    fetcher: &F,
    series_gr_key: &str,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Vec<livrarr_external_data::goodreads::GoodreadsSeriesBook>, SeriesServiceError> {
    let cancelled = |c: Option<&tokio_util::sync::CancellationToken>| {
        c.map(|t| t.is_cancelled()).unwrap_or(false)
    };
    let mut collected = Vec::new();
    let mut primary_count: Option<usize> = None;
    let mut page = 1;

    loop {
        let url = if page == 1 {
            format!("https://www.goodreads.com/series/{}", series_gr_key)
        } else {
            format!(
                "https://www.goodreads.com/series/{}?page={}",
                series_gr_key, page
            )
        };

        let html = fetch_gr_html(fetcher, &url).await?;
        let parsed = livrarr_external_data::goodreads::parse_series_detail_html(&html);

        if page == 1 {
            primary_count = parsed.primary_count;
        }
        if parsed.books.is_empty() {
            break;
        }
        collected.extend(parsed.books);

        let Some(needed) = primary_count else { break };
        if collected.len() >= needed || !parsed.has_next || page >= 10 {
            break;
        }

        page += 1;
        match cancel {
            Some(c) => {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    _ = c.cancelled() => break,
                }
            }
            None => tokio::time::sleep(Duration::from_secs(1)).await,
        }
        if cancelled(cancel) {
            break;
        }
    }

    let Some(needed) = primary_count else {
        if !collected.is_empty() {
            tracing::warn!(
                series_gr_key,
                books = collected.len(),
                "GR series page parsed books but carries no primary count — refusing to guess the roster"
            );
        }
        return Ok(Vec::new());
    };
    // Fewer books than the header declared means a later page was unreadable
    // (or the pagination walk stopped short): a PARTIAL roster is drift, not
    // truth — returning empty routes it into the same no-write guards, so a
    // stored full roster is never replaced by a partial one (review R-3).
    if collected.len() < needed {
        tracing::warn!(
            series_gr_key,
            collected = collected.len(),
            declared = needed,
            "GR roster: fewer books than the declared primary count — refusing a partial roster"
        );
        return Ok(Vec::new());
    }
    collected.truncate(needed);
    let before = collected.len();
    collected.retain(|b| !livrarr_external_data::goodreads::is_collection_title(&b.title));
    if collected.len() < before {
        tracing::warn!(
            series_gr_key,
            screened = before - collected.len(),
            "GR roster: screened collection-shaped titles inside the primary window"
        );
    }
    Ok(collected)
}

pub(super) fn to_roster_entries(
    books: &[livrarr_external_data::goodreads::GoodreadsSeriesBook],
) -> Vec<SeriesRosterEntry> {
    books
        .iter()
        .map(|b| SeriesRosterEntry {
            title: b.title.clone(),
            gr_key: b.gr_key.clone(),
            position: b.position,
            year: b.year,
        })
        .collect()
}

pub(super) async fn fetch_author_series_pages<F: HttpFetcher>(
    fetcher: &F,
    gr_author_id: &str,
    cancel: Option<&tokio_util::sync::CancellationToken>,
) -> Result<Vec<SeriesCacheEntry>, SeriesServiceError> {
    let cancelled = |c: Option<&tokio_util::sync::CancellationToken>| {
        c.map(|t| t.is_cancelled()).unwrap_or(false)
    };
    // Primary: HTML series list page (has proper gr_keys for monitoring)
    let mut all_entries = Vec::new();
    let mut page = 1;

    loop {
        let url = format!(
            "https://www.goodreads.com/series/list?id={}&page={}",
            gr_author_id, page
        );

        let html = fetch_gr_html(fetcher, &url).await?;
        let (entries, has_next) = livrarr_external_data::goodreads::parse_series_list_html(&html);

        if entries.is_empty() {
            break;
        }

        all_entries.extend(entries.into_iter().map(|e| SeriesCacheEntry {
            name: e.name,
            gr_key: e.gr_key,
            book_count: e.book_count,
            // GR gives no language signal at all (#112) — classified
            // separately, after the fetch, via Google Books.
            language: None,
        }));

        if !has_next || page >= 10 {
            break;
        }

        page += 1;
        match cancel {
            Some(c) => {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                    _ = c.cancelled() => break,
                }
            }
            None => tokio::time::sleep(Duration::from_secs(1)).await,
        }
        if cancelled(cancel) {
            break;
        }
    }

    // No GR `/search` fallback (REQ-005/ST-012): the series-list pages are
    // the only road. An empty result is an honest empty result — series
    // names from work metadata surface via DB stubs instead.
    Ok(all_entries)
}
