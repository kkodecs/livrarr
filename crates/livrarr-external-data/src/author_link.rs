//! Road-scoped author transport: every author credited on ONE selected work
//! (OpenLibrary, Goodreads, Hardcover), OpenLibrary author-name search, and
//! OpenLibrary catalog paging — composed into the production
//! [`AuthorProviderGatewayImpl`].
//!
//! These adapters return contributors and candidates, never a chosen author.
//! Selection is the caller's guard, so a multi-contributor work reaches it
//! whole. Three shapes are kept apart on purpose: a readable record crediting
//! nobody is an empty success, an unreadable association shape is
//! `LayoutDrift`, and a transport failure keeps its retry timing.

use std::collections::{HashMap, HashSet};
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use livrarr_domain::identity_matching::canonical_author_key;
use livrarr_domain::services::{
    AuthorProviderGateway, FetchError, FetchRequest, HttpFetcher, HttpMethod, RateBucket,
    UserAgentProfile,
};
use livrarr_domain::{
    AuthorProvider, AuthorProviderError, AuthorRouteKey, OpenLibraryAuthorCandidate,
    OpenLibraryAuthorKey, OpenLibraryCatalogPage, ProviderAuthorRef, RequestPriority,
    CERTIFIED_AUTHOR_ROLE,
};
use lru::LruCache;
use tokio::sync::{broadcast, Semaphore};
use tokio::time::Instant;

use crate::goodreads;
use crate::hardcover::{hc_post, HardcoverError};
use crate::openlibrary::classify_ol_error;
use crate::provider_client::{GoodreadsClient, HardcoverClient, OpenLibraryClient};
use crate::types::ProviderFetchError;

/// How many Hardcover editions of one book are scanned for contributors.
/// Editions of the same book credit the same people; the cap bounds a
/// pathological catalogue without narrowing a real contributor set.
const HARDCOVER_EDITION_SCAN_LIMIT: u32 = 20;

/// Timeout and body cap for a keyed OpenLibrary record (work or author).
const OL_RECORD_TIMEOUT: Duration = Duration::from_secs(30);
const OL_RECORD_MAX_BODY: usize = 2 * 1024 * 1024;

/// Timeout and body cap for OpenLibrary author search — the interactive
/// add-author door's established budget, kept so background Tier 2 and the
/// door share one request shape.
const OL_AUTHOR_SEARCH_TIMEOUT: Duration = Duration::from_secs(10);
const OL_AUTHOR_SEARCH_MAX_BODY: usize = 512 * 1024;

/// One catalog request reads a batch, never one work per round-trip: the
/// author-works endpoint is a batch endpoint and OpenLibrary names N
/// single-record fetches as a bad-citizen pattern.
const OL_CATALOG_PAGE_LIMIT: u32 = 100;
const OL_CATALOG_TIMEOUT: Duration = Duration::from_secs(30);
const OL_CATALOG_MAX_BODY: usize = 2 * 1024 * 1024;

/// How long one hydrated OpenLibrary author name is trusted. Author records
/// change rarely and a stale display name is corrected by the next sweep, so
/// this trades a long window against repeat traffic on the shared OL bucket.
const OL_AUTHOR_NAME_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// How many hydrated names are retained. One sweep over a large library sees
/// far fewer distinct contributors than works, so this holds a whole pass.
const OL_AUTHOR_NAME_CACHE_CAPACITY: usize = 1024;

/// How many hydration requests may be outstanding at once. The shared
/// OpenLibrary bucket remains the pacing authority; this only bounds how many
/// slow requests one sweep can be waiting on.
const OL_HYDRATION_MAX_IN_FLIGHT: usize = 2;

/// One route per person, keeping every credit the provider made for them.
///
/// A provider can credit the same person twice on one book — Hardcover per
/// edition, Goodreads on both a primary and a secondary edge — and the credits
/// need not agree. A route is authorial if **any** occurrence certified it,
/// because a person who wrote one edition wrote the book; a route nobody
/// credited as an author keeps whatever label it was first seen with. The order
/// the provider credited people in is preserved.
fn aggregate_credits(refs: Vec<ProviderAuthorRef>) -> Vec<ProviderAuthorRef> {
    let mut order: Vec<String> = Vec::new();
    let mut by_route: HashMap<String, ProviderAuthorRef> = HashMap::new();
    for candidate in refs {
        let route = candidate.key.value();
        match by_route.entry(route.clone()) {
            std::collections::hash_map::Entry::Vacant(slot) => {
                order.push(route);
                slot.insert(candidate);
            }
            std::collections::hash_map::Entry::Occupied(mut slot) => {
                let held_is_authorial = slot.get().role.as_deref() == Some(CERTIFIED_AUTHOR_ROLE);
                let offer_is_authorial = candidate.role.as_deref() == Some(CERTIFIED_AUTHOR_ROLE);
                if offer_is_authorial && !held_is_authorial {
                    // The authorial occurrence is the one worth keeping, and it
                    // brings the name the provider used when crediting the
                    // person as an author.
                    slot.insert(candidate);
                }
            }
        }
    }
    order
        .into_iter()
        .filter_map(|route| by_route.remove(&route))
        .collect()
}

/// What one adapter's per-entry reads add up to.
///
/// Dropping a single unreadable entry is parse-drift discipline; dropping every
/// entry is the shape having moved. Answering `Ok([])` in that second case
/// would certify a non-empty response as "nobody was credited" and make the
/// keyed read terminally successful on fabricated emptiness (insight 62).
struct ContributorRead {
    refs: Vec<ProviderAuthorRef>,
    raw_entries: usize,
    role_dropped: usize,
}

impl ContributorRead {
    fn new() -> Self {
        Self {
            refs: Vec::new(),
            raw_entries: 0,
            role_dropped: 0,
        }
    }

    /// The aggregated credits, or drift when a moved role shape is the only
    /// reason this response looks empty.
    fn finish(self, provider: &str) -> Result<Vec<ProviderAuthorRef>, ProviderFetchError> {
        if self.raw_entries > 0 && self.refs.is_empty() && self.role_dropped > 0 {
            return Err(ProviderFetchError::LayoutDrift(format!(
                "{provider} credited {} contributor(s) but none carried a readable role",
                self.raw_entries
            )));
        }
        Ok(aggregate_credits(self.refs))
    }
}

impl<F: HttpFetcher> OpenLibraryClient<F> {
    /// Every author credited on one OpenLibrary work.
    ///
    /// A work record names its contributors by key. The name is read from the
    /// record when it carries one and hydrated from the author record when it
    /// does not, so a contributor never reaches the name guard nameless.
    pub async fn fetch_work_authors(
        &self,
        work_key: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, ProviderFetchError> {
        let key = work_key
            .trim()
            .trim_start_matches("/works/")
            .trim_start_matches('/');
        if key.is_empty() {
            return Err(ProviderFetchError::Permanent(
                "OpenLibrary work route is empty".to_string(),
            ));
        }

        let record = self
            .fetch_ol_json(
                &format!("https://openlibrary.org/works/{key}.json"),
                priority,
            )
            .await?;

        let entries = record
            .get("authors")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        let mut read = ContributorRead::new();
        for entry in &entries {
            read.raw_entries += 1;
            let Some(raw_key) = entry
                .pointer("/author/key")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|raw| !raw.is_empty())
            else {
                continue;
            };
            let Ok(route) = AuthorRouteKey::parse(AuthorProvider::OpenLibrary, raw_key) else {
                tracing::warn!(
                    work_key = %key,
                    author_key = %raw_key,
                    "OpenLibrary work contributor key is not a canonical author key"
                );
                continue;
            };
            let AuthorRouteKey::OpenLibrary(author_key) = &route else {
                continue;
            };

            let name = match entry
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                Some(name) => name.to_string(),
                None => self.hydrate_author_name(author_key, priority).await?,
            };

            read.refs.push(ProviderAuthorRef {
                key: route,
                name,
                role: Some(open_library_certified_role(entry)),
            });
        }

        read.finish("OpenLibrary")
    }

    /// The credited name on one OpenLibrary author record, served by the
    /// client's shared hydrator.
    ///
    /// A missing or blank name is a failure, never an empty name: an author
    /// ref with no name cannot be guarded, and pretending otherwise would put
    /// an unverifiable route in front of the guard.
    async fn hydrate_author_name(
        &self,
        author_key: &OpenLibraryAuthorKey,
        priority: RequestPriority,
    ) -> Result<String, ProviderFetchError> {
        self.hydrator()
            .name_for_key(self.fetcher(), author_key, priority)
            .await
    }

    /// One paced OpenLibrary GET, through the shared queue bucket and the
    /// established server identity.
    async fn fetch_ol_json(
        &self,
        url: &str,
        priority: RequestPriority,
    ) -> Result<serde_json::Value, ProviderFetchError> {
        fetch_ol_json(
            self.fetcher(),
            url,
            OL_RECORD_TIMEOUT,
            OL_RECORD_MAX_BODY,
            priority,
        )
        .await
    }
}

/// What one entry of an OpenLibrary work's `authors[]` credits this person as.
///
/// OpenLibrary spells an ordinary author credit as the structural type
/// `/type/author_role`; other role types name a different credit and travel
/// under their own stripped label. An entry with no type at all is still a
/// member of the work's **author list**, which is the credit the container
/// itself makes.
fn open_library_certified_role(entry: &serde_json::Value) -> String {
    match entry
        .pointer("/type/key")
        .and_then(|value| value.as_str())
        .map(|raw| raw.trim().trim_start_matches("/type/"))
        .filter(|label| !label.is_empty())
    {
        None | Some("author_role") => CERTIFIED_AUTHOR_ROLE.to_string(),
        Some(label) => label.to_string(),
    }
}

/// One paced OpenLibrary GET for any author-road request, through the shared
/// queue bucket and the established server identity.
async fn fetch_ol_json<F: HttpFetcher>(
    fetcher: &F,
    url: &str,
    timeout: Duration,
    max_body_bytes: usize,
    priority: RequestPriority,
) -> Result<serde_json::Value, ProviderFetchError> {
    let request = FetchRequest {
        url: url.to_string(),
        method: HttpMethod::Get,
        headers: vec![],
        body: None,
        timeout,
        rate_bucket: RateBucket::OpenLibrary,
        max_body_bytes,
        anti_bot_check: false,
        user_agent: UserAgentProfile::Server,
        priority,
    };
    let response = match fetcher.fetch(request).await {
        Ok(response) => response,
        Err(error) => return Err(map_ol_transport_error(error)),
    };
    if !(200..300).contains(&response.status) {
        return Err(map_ol_status(response.status, &response.headers));
    }
    serde_json::from_slice(&response.body).map_err(|error| {
        tracing::warn!(url, %error, "OpenLibrary response body is not JSON");
        ProviderFetchError::LayoutDrift(format!("OpenLibrary response is not JSON: {error}"))
    })
}

/// A transport failure keeps its class: a pause, a rate limit with its wait, or
/// a permanent refusal.
fn map_ol_transport_error(error: FetchError) -> ProviderFetchError {
    match error {
        FetchError::RateLimited => ProviderFetchError::Retryable {
            error: "OpenLibrary rate limited".to_string(),
            retry_not_before: None,
        },
        FetchError::CircuitOpen { retry_after } => ProviderFetchError::CircuitOpen(retry_after),
        FetchError::QueueFull { retry_after } => ProviderFetchError::QueueFull(retry_after),
        FetchError::HttpError { status, .. } => map_ol_status(status, &[]),
        other => ProviderFetchError::Retryable {
            error: format!("OpenLibrary transport failure: {other}"),
            retry_not_before: None,
        },
    }
}

/// A non-2xx OpenLibrary status, classified through the shared authority so
/// this surface cannot disagree with the enrichment surface about a status.
///
/// A retryable status carries the provider's own `Retry-After` when it sent a
/// readable one, so the road waits as long as OpenLibrary asked rather than
/// guessing.
fn map_ol_status(status: u16, headers: &[(String, String)]) -> ProviderFetchError {
    if status == 404 || status == 410 {
        return ProviderFetchError::Permanent(format!("OpenLibrary HTTP {status}"));
    }
    match classify_ol_error(status) {
        ProviderFetchError::RateLimited | ProviderFetchError::Transient => {
            ProviderFetchError::Retryable {
                error: format!("OpenLibrary HTTP {status}"),
                retry_not_before: retry_after_hint(headers),
            }
        }
        _ => ProviderFetchError::Permanent(format!("OpenLibrary HTTP {status}")),
    }
}

/// `Retry-After` in either documented form — delay-seconds or an HTTP-date —
/// resolved at response-receipt time into an absolute UTC lower bound.
///
/// An absent header is simply no hint. A header that is present but unreadable
/// is a provider-side oddity worth a warning, and still only costs the hint:
/// the caller's own backoff decides when to try again.
fn retry_after_hint(headers: &[(String, String)]) -> Option<DateTime<Utc>> {
    let raw = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .map(|(_, value)| value.trim())?;
    if let Ok(seconds) = raw.parse::<i64>() {
        if seconds >= 0 {
            return Some(Utc::now() + chrono::Duration::seconds(seconds));
        }
    }
    if let Ok(at) = DateTime::parse_from_rfc2822(raw) {
        return Some(at.with_timezone(&Utc));
    }
    if let Ok(at) = chrono::NaiveDateTime::parse_from_str(raw, "%a, %d %b %Y %H:%M:%S GMT") {
        return Some(DateTime::from_naive_utc_and_offset(at, Utc));
    }
    tracing::warn!(
        retry_after = raw,
        "provider Retry-After header is neither a delay nor an HTTP-date"
    );
    None
}

/// One hydrated OpenLibrary author name and when it stops being trusted.
struct CachedAuthorName {
    name: String,
    expires_at: Instant,
}

/// What a hydration publishes to every caller waiting on the same key.
type HydrationResult = Result<String, ProviderFetchError>;

/// The cache and the set of hydrations currently in flight, guarded together so
/// a caller decides "serve, join, or lead" in one atomic step.
struct HydrationState {
    names: LruCache<String, CachedAuthorName>,
    flights: HashMap<String, broadcast::Sender<HydrationResult>>,
}

impl HydrationState {
    /// A cached name that has not expired. An expired entry is dropped here so
    /// it cannot be served and cannot hold capacity.
    fn fresh(&mut self, key: &str) -> Option<String> {
        let hit = self
            .names
            .get(key)
            .map(|entry| (entry.name.clone(), entry.expires_at));
        match hit {
            Some((name, expires_at)) if expires_at > Instant::now() => Some(name),
            Some(_) => {
                self.names.pop(key);
                None
            }
            None => None,
        }
    }
}

/// Names for OpenLibrary author keys, fetched at most once per key while the
/// name stays fresh.
///
/// An OpenLibrary work record credits its authors by key and often carries no
/// name, and one author is credited on many works — so the naive adapter turns
/// one sweep into hundreds of `/authors/<key>.json` round-trips, exactly the
/// pattern OpenLibrary names as bad citizenship. Three things prevent that: a
/// positive TTL cache, same-key coalescing so concurrent lookups of one key
/// make one request, and a small in-flight cap so a sweep never has more than
/// two hydrations outstanding on the shared bucket.
///
/// Only validated names are cached. A failure, an empty name, and a malformed
/// record are all eligible for a real retry — durable absence must never be
/// manufactured from a transient answer.
pub struct OpenLibraryAuthorHydrator {
    state: Mutex<HydrationState>,
    in_flight: Semaphore,
}

impl Default for OpenLibraryAuthorHydrator {
    fn default() -> Self {
        Self::new()
    }
}

/// Removes this caller's flight registration however the caller leaves — including
/// a cancelled sweep. Without it a dropped leader would strand every waiter on a
/// hydration that will never publish.
struct FlightGuard<'a> {
    hydrator: &'a OpenLibraryAuthorHydrator,
    key: Option<String>,
}

impl FlightGuard<'_> {
    /// The leader retired the flight itself, under the same lock that published
    /// the name; there is nothing left to clean up.
    fn disarm(&mut self) {
        self.key = None;
    }
}

impl Drop for FlightGuard<'_> {
    fn drop(&mut self) {
        if let Some(key) = self.key.take() {
            self.hydrator.lock_state().flights.remove(&key);
        }
    }
}

impl OpenLibraryAuthorHydrator {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(HydrationState {
                names: LruCache::new(
                    NonZeroUsize::new(OL_AUTHOR_NAME_CACHE_CAPACITY)
                        .expect("hydration cache capacity is a nonzero constant"),
                ),
                flights: HashMap::new(),
            }),
            in_flight: Semaphore::new(OL_HYDRATION_MAX_IN_FLIGHT),
        }
    }

    /// The lock is only ever held across map and cache operations, never across
    /// a request, so a poisoned lock carries no half-written state.
    fn lock_state(&self) -> std::sync::MutexGuard<'_, HydrationState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The credited name for one canonical OpenLibrary author key.
    pub(crate) async fn name_for_key<F: HttpFetcher>(
        &self,
        fetcher: &F,
        author_key: &OpenLibraryAuthorKey,
        priority: RequestPriority,
    ) -> HydrationResult {
        let key = author_key.as_str();
        loop {
            let leader = {
                let mut state = self.lock_state();
                if let Some(name) = state.fresh(key) {
                    return Ok(name);
                }
                match state.flights.get(key) {
                    Some(sender) => Err(sender.subscribe()),
                    None => {
                        let (sender, _) = broadcast::channel(1);
                        state.flights.insert(key.to_string(), sender.clone());
                        Ok(sender)
                    }
                }
            };

            let sender = match leader {
                Ok(sender) => sender,
                Err(mut waiting) => match waiting.recv().await {
                    Ok(result) => return result,
                    // The leader left without publishing (a cancelled sweep).
                    // Compete to lead the next attempt rather than reporting a
                    // failure the provider never gave.
                    Err(_) => continue,
                },
            };

            let mut flight = FlightGuard {
                hydrator: self,
                key: Some(key.to_string()),
            };
            let outcome = {
                let _permit = self
                    .in_flight
                    .acquire()
                    .await
                    .expect("hydration semaphore is never closed");
                fetch_author_name(fetcher, key, priority).await
            };

            {
                let mut state = self.lock_state();
                if let Ok(name) = &outcome {
                    state.names.put(
                        key.to_string(),
                        CachedAuthorName {
                            name: name.clone(),
                            expires_at: Instant::now() + OL_AUTHOR_NAME_TTL,
                        },
                    );
                }
                state.flights.remove(key);
            }
            flight.disarm();
            // Fails only when every waiter has gone; the result is this
            // caller's regardless.
            let _ = sender.send(outcome.clone());
            return outcome;
        }
    }
}

/// One keyed OpenLibrary author record, read for its credited name.
async fn fetch_author_name<F: HttpFetcher>(
    fetcher: &F,
    author_key: &str,
    priority: RequestPriority,
) -> Result<String, ProviderFetchError> {
    let record = fetch_ol_json(
        fetcher,
        &format!("https://openlibrary.org/authors/{author_key}.json"),
        OL_RECORD_TIMEOUT,
        OL_RECORD_MAX_BODY,
        priority,
    )
    .await?;
    record
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            tracing::warn!(author_key, "OpenLibrary author record carries no name");
            ProviderFetchError::Permanent(format!(
                "OpenLibrary author {author_key} record carries no name"
            ))
        })
}

impl<F: HttpFetcher> GoodreadsClient<F> {
    /// Every contributor credited on one Goodreads book.
    ///
    /// Only the selected book's own contributor edges (or its JSON-LD author
    /// entries) are followed. An unreadable association shape is `LayoutDrift`,
    /// so a Goodreads layout change can never look like an uncredited book.
    pub async fn fetch_work_authors(
        &self,
        book_key: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, ProviderFetchError> {
        let key = book_key.trim();
        if key.is_empty() {
            return Err(ProviderFetchError::Permanent(
                "Goodreads book route is empty".to_string(),
            ));
        }
        let url = goodreads::detail_url_for_gr_key(self.base_url(), key);
        let html = goodreads::fetch_goodreads_html(self.fetcher(), &url, priority)
            .await
            .map_err(map_goodreads_error)?;

        let read = goodreads::parse_book_contributors(&html).ok_or_else(|| {
            ProviderFetchError::LayoutDrift(format!(
                "Goodreads book {key} has no readable contributor association"
            ))
        })?;
        let from_author_field = read.source == goodreads::GoodreadsContributorSource::JsonLdAuthors;

        let mut contributors = ContributorRead::new();
        for contributor in read.contributors {
            contributors.raw_entries += 1;
            // The JSON-LD fallback reads the book's `author` field, so every
            // entry in it is an author credit by the field's own meaning. An
            // Apollo edge instead names its credit, and an edge that names none
            // has told us nothing we may act on.
            let role = if from_author_field {
                Some(CERTIFIED_AUTHOR_ROLE.to_string())
            } else {
                match contributor.role.as_deref() {
                    Some(label) => Some(goodreads_certified_role(label)),
                    None => {
                        tracing::warn!(
                            book_key = %key,
                            author_id = %contributor.raw_id,
                            "Goodreads contributor edge names no role — entry dropped"
                        );
                        contributors.role_dropped += 1;
                        continue;
                    }
                }
            };
            match AuthorRouteKey::parse(AuthorProvider::Goodreads, &contributor.raw_id) {
                Ok(route) => contributors.refs.push(ProviderAuthorRef {
                    key: route,
                    name: contributor.name,
                    role,
                }),
                Err(_) => tracing::warn!(
                    book_key = %key,
                    author_id = %contributor.raw_id,
                    "Goodreads contributor id is not a canonical author id"
                ),
            }
        }

        contributors.finish("Goodreads")
    }
}

/// Goodreads transport failures, keeping a pause distinct from a refusal and an
/// anti-bot block distinct from a missing page.
fn map_goodreads_error(error: goodreads::GoodreadsFetchError) -> ProviderFetchError {
    match error {
        goodreads::GoodreadsFetchError::CircuitOpen(retry_after) => {
            ProviderFetchError::CircuitOpen(retry_after)
        }
        goodreads::GoodreadsFetchError::QueueFull(retry_after) => {
            ProviderFetchError::QueueFull(retry_after)
        }
        goodreads::GoodreadsFetchError::HttpStatus(status) if status == 404 || status == 410 => {
            ProviderFetchError::Permanent(format!("Goodreads HTTP {status}"))
        }
        goodreads::GoodreadsFetchError::HttpStatus(status) => ProviderFetchError::Retryable {
            error: format!("Goodreads HTTP {status}"),
            retry_not_before: None,
        },
        goodreads::GoodreadsFetchError::SearchRouteFailure(status) => {
            ProviderFetchError::Retryable {
                error: format!("Goodreads route failure HTTP {status}"),
                retry_not_before: None,
            }
        }
        goodreads::GoodreadsFetchError::AntiBot => ProviderFetchError::Retryable {
            error: "Goodreads anti-bot challenge".to_string(),
            retry_not_before: None,
        },
        goodreads::GoodreadsFetchError::Network(detail)
        | goodreads::GoodreadsFetchError::SsrfRejected(detail) => ProviderFetchError::Retryable {
            error: detail,
            retry_not_before: None,
        },
        // The page fetch never parses, so this cannot originate here; a future
        // shared helper that does is drift, not an uncredited book.
        goodreads::GoodreadsFetchError::Parse => ProviderFetchError::LayoutDrift(
            "Goodreads page carried no readable payload".to_string(),
        ),
    }
}

impl<F: HttpFetcher> HardcoverClient<F> {
    /// Every contributor credited on one Hardcover book.
    ///
    /// Hardcover credits contributors per edition of a book, so the book's
    /// editions are the association this reads. A readable response with no
    /// contributions is an empty success; a response whose contribution or
    /// author shape has moved is `LayoutDrift`, never a silent empty answer.
    pub async fn fetch_work_authors(
        &self,
        book_key: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, ProviderFetchError> {
        let config = self.live_config().snapshot();
        if !config.hardcover_enabled {
            return Err(ProviderFetchError::NotConfigured);
        }
        let Some(token) = config
            .hardcover_api_token
            .as_deref()
            .map(|token| {
                token
                    .trim()
                    .trim_start_matches("Bearer ")
                    .trim_start_matches("bearer ")
            })
            .filter(|token| !token.is_empty())
        else {
            return Err(ProviderFetchError::NotConfigured);
        };

        let book_id: i64 = book_key.trim().parse().map_err(|_| {
            ProviderFetchError::Permanent(format!(
                "Hardcover book route {book_key:?} is not an integer id"
            ))
        })?;

        let query = format!(
            r#"query GetBookContributors($bookId: Int!) {{
            editions(where: {{book_id: {{_eq: $bookId}}}}, order_by: [{{users_read_count: desc}}], limit: {HARDCOVER_EDITION_SCAN_LIMIT}) {{
                contributions {{
                    contribution
                    author {{
                        id
                        name
                    }}
                }}
            }}
        }}"#
        );
        let body = serde_json::json!({
            "query": query,
            "variables": {"bookId": book_id}
        });
        let response = hc_post(self.fetcher(), body, token, priority)
            .await
            .map_err(map_hardcover_error)?;

        let editions = response
            .pointer("/data/editions")
            .and_then(|value| value.as_array())
            .ok_or_else(|| {
                ProviderFetchError::LayoutDrift(
                    "Hardcover response carries no editions array".to_string(),
                )
            })?;

        let mut read = ContributorRead::new();
        for edition in editions {
            let contributions = edition
                .get("contributions")
                .and_then(|value| value.as_array())
                .ok_or_else(|| {
                    ProviderFetchError::LayoutDrift(
                        "Hardcover edition carries no contributions array".to_string(),
                    )
                })?;
            for contribution in contributions {
                read.raw_entries += 1;
                let author = contribution
                    .get("author")
                    .filter(|value| value.is_object())
                    .ok_or_else(|| {
                        ProviderFetchError::LayoutDrift(
                            "Hardcover contribution carries no author object".to_string(),
                        )
                    })?;
                let id = author
                    .get("id")
                    .and_then(|value| value.as_i64())
                    .ok_or_else(|| {
                        ProviderFetchError::LayoutDrift(
                            "Hardcover contribution author carries no integer id".to_string(),
                        )
                    })?;
                let name = author
                    .get("name")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| {
                        ProviderFetchError::LayoutDrift(
                            "Hardcover contribution author carries no name".to_string(),
                        )
                    })?;
                let route = AuthorRouteKey::parse(AuthorProvider::Hardcover, &id.to_string())
                    .map_err(|_| {
                        ProviderFetchError::LayoutDrift(format!(
                            "Hardcover author id {id} is not a canonical author id"
                        ))
                    })?;
                let Some(role) = hardcover_certified_role(contribution) else {
                    tracing::warn!(
                        book_id,
                        author_id = id,
                        "Hardcover contribution field is absent or an unreadable shape — entry dropped"
                    );
                    read.role_dropped += 1;
                    continue;
                };
                read.refs.push(ProviderAuthorRef {
                    key: route,
                    name: name.to_string(),
                    role: Some(role),
                });
            }
        }

        read.finish("Hardcover")
    }
}

/// What one Hardcover contribution credits this person as, or `None` when the
/// field cannot be read at all.
///
/// Hardcover leaves `contribution` empty for the people who wrote the book and
/// names every other credit in free text. A field that is *absent*, or present
/// in a shape this reader does not know, is not the same as an empty one: it is
/// a shape that moved, and guessing it into an author credit is exactly the
/// silent failure the role gate exists to prevent.
fn hardcover_certified_role(contribution: &serde_json::Value) -> Option<String> {
    match contribution.get("contribution") {
        Some(serde_json::Value::Null) => Some(CERTIFIED_AUTHOR_ROLE.to_string()),
        Some(serde_json::Value::String(label)) => Some(label.clone()),
        _ => None,
    }
}

/// What one Goodreads contributor edge credits this person as.
///
/// The site spells an author credit two ways — plain "Author" on a primary
/// edge, "Goodreads Author" when the person has a claimed profile — and both
/// mean the same thing. Every other credit travels verbatim.
fn goodreads_certified_role(label: &str) -> String {
    if label.eq_ignore_ascii_case("author") || label.eq_ignore_ascii_case("goodreads author") {
        return CERTIFIED_AUTHOR_ROLE.to_string();
    }
    label.to_string()
}

/// Hardcover failures: a pause stays a pause, and a readable-but-refused
/// GraphQL answer is permanent rather than drift.
fn map_hardcover_error(error: HardcoverError) -> ProviderFetchError {
    match error {
        HardcoverError::CircuitOpen(retry_after) => ProviderFetchError::CircuitOpen(retry_after),
        HardcoverError::Http(detail) if is_retryable_hardcover_failure(&detail) => {
            ProviderFetchError::Retryable {
                error: detail,
                retry_not_before: None,
            }
        }
        HardcoverError::Http(detail) => ProviderFetchError::Permanent(detail),
        HardcoverError::NoResults => {
            ProviderFetchError::Permanent("Hardcover returned no book".to_string())
        }
        HardcoverError::NoMatch(detail) => ProviderFetchError::Permanent(detail),
    }
}

/// A rate limit or a 5xx is Hardcover's own temporary state, so it retries;
/// every other refusal is an answer about this request and does not.
fn is_retryable_hardcover_failure(detail: &str) -> bool {
    detail
        .strip_prefix("HTTP ")
        .and_then(|status| status.trim().parse::<u16>().ok())
        .is_some_and(|status| status == 429 || (500..600).contains(&status))
}

/// OpenLibrary authors whose name matches `query`.
///
/// The one author-name search implementation: the interactive add-author door
/// and the background linking road both come through here, differing only in
/// the priority they hand the shared queue. Candidates are evidence for a
/// person to read — this never mints a route and never writes state.
pub async fn open_library_author_search<F: HttpFetcher>(
    fetcher: &F,
    query: &str,
    limit: u32,
    priority: RequestPriority,
) -> Result<Vec<OpenLibraryAuthorCandidate>, ProviderFetchError> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(vec![]);
    }
    let url = format!(
        "https://openlibrary.org/search/authors.json?q={}&limit={limit}",
        urlencoding::encode(trimmed)
    );
    let payload = fetch_ol_json(
        fetcher,
        &url,
        OL_AUTHOR_SEARCH_TIMEOUT,
        OL_AUTHOR_SEARCH_MAX_BODY,
        priority,
    )
    .await?;

    let Some(docs) = payload.get("docs").and_then(|value| value.as_array()) else {
        tracing::warn!(
            query = trimmed,
            "OpenLibrary author search carries no docs array"
        );
        return Err(ProviderFetchError::Permanent(
            "OpenLibrary author search response carries no docs array".to_string(),
        ));
    };

    let mut candidates = Vec::with_capacity(docs.len());
    for doc in docs {
        match parse_author_candidate(doc) {
            Some(candidate) => candidates.push(candidate),
            None => tracing::warn!(
                query = trimmed,
                "OpenLibrary author search document is not a readable author candidate"
            ),
        }
    }
    // A page of documents none of which is readable is drift, not an author
    // nobody has heard of. Reporting it as "no candidates" would park the
    // author with a confident empty answer.
    if candidates.is_empty() && !docs.is_empty() {
        return Err(ProviderFetchError::Permanent(
            "OpenLibrary author search returned only unreadable documents".to_string(),
        ));
    }
    Ok(candidates)
}

/// One search document as a candidate, or `None` when it cannot be read.
fn parse_author_candidate(doc: &serde_json::Value) -> Option<OpenLibraryAuthorCandidate> {
    let raw_key = doc
        .get("key")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|raw| !raw.is_empty())?;
    let route_key = match AuthorRouteKey::parse(AuthorProvider::OpenLibrary, raw_key) {
        Ok(AuthorRouteKey::OpenLibrary(key)) => key,
        _ => return None,
    };
    let name = doc
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())?
        .to_string();

    // One spelling family contributes one alias. Deduplicating on the shared
    // canonical form — including against the primary name — keeps a candidate
    // from producing several verdict rows that all say the same thing.
    let mut seen = HashSet::new();
    seen.insert(alias_fingerprint(&name));
    let alternate_names = doc
        .get("alternate_names")
        .and_then(|value| value.as_array())
        .map(|aliases| {
            aliases
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::trim)
                .filter(|alias| !alias.is_empty())
                .filter(|alias| seen.insert(alias_fingerprint(alias)))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    Some(OpenLibraryAuthorCandidate {
        route_key,
        name,
        alternate_names,
        top_work: doc
            .get("top_work")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string),
        work_count: doc
            .get("work_count")
            .and_then(|value| value.as_u64())
            .and_then(|count| u32::try_from(count).ok()),
    })
}

/// What makes two spellings the same alias. The shared canonical author key
/// where it produces one — a script it cannot canonicalize falls back to the
/// alias itself, so unrelated non-Latin spellings never collapse together.
fn alias_fingerprint(alias: &str) -> String {
    let canonical = canonical_author_key(alias);
    if canonical.is_empty() {
        alias.to_lowercase()
    } else {
        canonical
    }
}

/// One page of an OpenLibrary author's catalog.
///
/// The cursor is the offset the next page starts at, so an interrupted walk
/// resumes exactly where it stopped. A page that reads cleanly and lists
/// nothing is a successful empty page; a page that cannot be read is an error,
/// never a catalog with no titles in it.
pub async fn open_library_catalog_page<F: HttpFetcher>(
    fetcher: &F,
    author_route: &OpenLibraryAuthorKey,
    cursor: Option<&str>,
    priority: RequestPriority,
) -> Result<OpenLibraryCatalogPage, ProviderFetchError> {
    let offset = match cursor {
        None => 0u32,
        Some(raw) => raw.trim().parse::<u32>().map_err(|_| {
            ProviderFetchError::Permanent(format!(
                "OpenLibrary catalog cursor {raw:?} is not an offset"
            ))
        })?,
    };
    let url = format!(
        "https://openlibrary.org/authors/{}/works.json?limit={OL_CATALOG_PAGE_LIMIT}&offset={offset}",
        author_route.as_str()
    );
    let payload = fetch_ol_json(
        fetcher,
        &url,
        OL_CATALOG_TIMEOUT,
        OL_CATALOG_MAX_BODY,
        priority,
    )
    .await?;

    let Some(entries) = payload.get("entries").and_then(|value| value.as_array()) else {
        tracing::warn!(
            author_key = author_route.as_str(),
            "OpenLibrary author works response carries no entries array"
        );
        return Err(ProviderFetchError::Permanent(
            "OpenLibrary author works response carries no entries array".to_string(),
        ));
    };

    let mut titles = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry
            .get("title")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|title| !title.is_empty())
        {
            Some(title) => titles.push(title.to_string()),
            None => tracing::warn!(
                author_key = author_route.as_str(),
                "OpenLibrary catalog entry carries no title"
            ),
        }
    }
    if titles.is_empty() && !entries.is_empty() {
        return Err(ProviderFetchError::Permanent(
            "OpenLibrary author works page carries no readable entry".to_string(),
        ));
    }

    // The provider's own `next` link is the authority on whether more exists;
    // a response without a links object falls back to "a full batch may have
    // more behind it". An empty page ends the walk either way — a cursor that
    // did not advance would read the same page forever.
    let has_next = payload
        .pointer("/links/next")
        .and_then(|value| value.as_str())
        .is_some()
        || (payload.get("links").is_none() && entries.len() as u32 >= OL_CATALOG_PAGE_LIMIT);
    let next_cursor =
        (has_next && !entries.is_empty()).then(|| (offset as usize + entries.len()).to_string());

    Ok(OpenLibraryCatalogPage {
        titles,
        next_cursor,
    })
}

/// The production [`AuthorProviderGateway`]: the three keyed contributor
/// adapters plus OpenLibrary author search and catalog paging, behind one
/// domain trait.
///
/// It classifies, it does not decide. Every failure comes back keyed to the one
/// call that produced it, so the road can park one provider key and keep
/// walking the rest.
pub struct AuthorProviderGatewayImpl<F: HttpFetcher = livrarr_http::fetcher::HttpFetcherImpl> {
    open_library: OpenLibraryClient<F>,
    goodreads: GoodreadsClient<F>,
    hardcover: HardcoverClient<F>,
}

impl<F: HttpFetcher> AuthorProviderGatewayImpl<F> {
    pub fn new(
        open_library: OpenLibraryClient<F>,
        goodreads: GoodreadsClient<F>,
        hardcover: HardcoverClient<F>,
    ) -> Self {
        Self {
            open_library,
            goodreads,
            hardcover,
        }
    }
}

impl<F: HttpFetcher> AuthorProviderGateway for AuthorProviderGatewayImpl<F> {
    async fn fetch_work_authors(
        &self,
        provider: AuthorProvider,
        work_route: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, AuthorProviderError> {
        let refs = match provider {
            AuthorProvider::OpenLibrary => {
                self.open_library
                    .fetch_work_authors(work_route, priority)
                    .await
            }
            AuthorProvider::Goodreads => {
                self.goodreads
                    .fetch_work_authors(work_route, priority)
                    .await
            }
            AuthorProvider::Hardcover => {
                self.hardcover
                    .fetch_work_authors(work_route, priority)
                    .await
            }
        };
        refs.map_err(map_gateway_error)
    }

    async fn search_open_library_authors(
        &self,
        query: String,
        limit: u32,
        priority: RequestPriority,
    ) -> Result<Vec<OpenLibraryAuthorCandidate>, AuthorProviderError> {
        open_library_author_search(self.open_library.fetcher(), &query, limit, priority)
            .await
            .map_err(map_gateway_error)
    }

    async fn fetch_open_library_catalog_page(
        &self,
        author_route: OpenLibraryAuthorKey,
        cursor: Option<String>,
        priority: RequestPriority,
    ) -> Result<OpenLibraryCatalogPage, AuthorProviderError> {
        open_library_catalog_page(
            self.open_library.fetcher(),
            &author_route,
            cursor.as_deref(),
            priority,
        )
        .await
        .map_err(map_gateway_error)
    }
}

/// An adapter failure as the road reads it.
///
/// The two local pauses — an open breaker and a full outbound queue — are
/// retryable with the wait the queue itself reported, so the road parks the key
/// until then instead of treating a pause as a provider verdict.
fn map_gateway_error(error: ProviderFetchError) -> AuthorProviderError {
    match error {
        ProviderFetchError::NotConfigured => AuthorProviderError::NotConfigured,
        ProviderFetchError::Retryable {
            error,
            retry_not_before,
        } => AuthorProviderError::Retryable {
            error,
            retry_not_before,
        },
        ProviderFetchError::Permanent(detail) => AuthorProviderError::Permanent(detail),
        ProviderFetchError::LayoutDrift(detail) => AuthorProviderError::LayoutDrift(detail),
        ProviderFetchError::CircuitOpen(retry_after) => AuthorProviderError::Retryable {
            error: format!(
                "provider circuit is open for another {}s",
                retry_after.as_secs()
            ),
            retry_not_before: absolute_retry_hint(retry_after),
        },
        ProviderFetchError::QueueFull(retry_after) => AuthorProviderError::Retryable {
            error: format!(
                "outbound queue is full for another {}s",
                retry_after.as_secs()
            ),
            retry_not_before: absolute_retry_hint(retry_after),
        },
        ProviderFetchError::RateLimited => AuthorProviderError::Retryable {
            error: "provider rate limited this request".to_string(),
            retry_not_before: None,
        },
        ProviderFetchError::Transient => AuthorProviderError::Retryable {
            error: "provider transport failed transiently".to_string(),
            retry_not_before: None,
        },
        ProviderFetchError::NotFound => {
            AuthorProviderError::Permanent("provider has no record for this key".to_string())
        }
        ProviderFetchError::Other(detail) => AuthorProviderError::Permanent(detail),
    }
}

/// A local pause expressed as the absolute time it ends.
fn absolute_retry_hint(retry_after: Duration) -> Option<DateTime<Utc>> {
    chrono::Duration::from_std(retry_after)
        .ok()
        .map(|wait| Utc::now() + wait)
}

#[cfg(test)]
mod tests {
    use super::*;

    use livrarr_domain::services::FetchResponse;
    use livrarr_domain::settings::MetadataConfig;

    use crate::live_config::LiveMetadataConfig;
    use crate::test_support::{lock_breaker, RecordingHttpFetcher};

    fn ol_key(raw: &str) -> OpenLibraryAuthorKey {
        match AuthorRouteKey::parse(AuthorProvider::OpenLibrary, raw) {
            Ok(AuthorRouteKey::OpenLibrary(key)) => key,
            other => panic!("expected an OpenLibrary author key, got {other:?}"),
        }
    }

    fn ok(body: &str) -> Result<FetchResponse, FetchError> {
        Ok(FetchResponse {
            status: 200,
            headers: vec![],
            body: body.as_bytes().to_vec(),
        })
    }

    fn status(status: u16, headers: Vec<(String, String)>) -> Result<FetchResponse, FetchError> {
        Ok(FetchResponse {
            status,
            headers,
            body: vec![],
        })
    }

    fn queued(responses: Vec<Result<FetchResponse, FetchError>>) -> RecordingHttpFetcher {
        let fetcher = RecordingHttpFetcher::new();
        for response in responses {
            fetcher.push_response(response);
        }
        fetcher
    }

    fn metadata_config(hardcover_enabled: bool, token: Option<&str>) -> MetadataConfig {
        MetadataConfig {
            hardcover_enabled,
            hardcover_api_token: token.map(str::to_string),
            llm_enabled: false,
            llm_provider: None,
            llm_endpoint: None,
            llm_api_key: None,
            llm_model: None,
            audnexus_url: "https://api.audnex.us".to_string(),
            languages: vec!["en".to_string()],
            google_books_api_key: None,
        }
    }

    fn gateway(
        fetcher: RecordingHttpFetcher,
        config: MetadataConfig,
    ) -> AuthorProviderGatewayImpl<RecordingHttpFetcher> {
        let http = livrarr_http::HttpClient::builder()
            .user_agent("livrarr-author-gateway-test")
            .build()
            .expect("test HTTP client");
        AuthorProviderGatewayImpl::new(
            OpenLibraryClient::new(fetcher.clone()),
            GoodreadsClient::new(fetcher.clone(), http, "https://www.goodreads.com"),
            HardcoverClient::new(fetcher, LiveMetadataConfig::new(config)),
        )
    }

    /// A fetcher that suspends once before answering, so two callers of the same
    /// key are genuinely in flight together. Without the suspension the first
    /// call would finish before the second was ever polled and the coalescing
    /// this exists to prove would never be exercised.
    #[derive(Clone)]
    struct SuspendingFetcher {
        inner: RecordingHttpFetcher,
    }

    impl HttpFetcher for SuspendingFetcher {
        async fn fetch(
            &self,
            request: FetchRequest,
        ) -> Result<FetchResponse, livrarr_domain::services::FetchError> {
            tokio::task::yield_now().await;
            self.inner.fetch(request).await
        }

        async fn fetch_ssrf_safe(
            &self,
            request: FetchRequest,
        ) -> Result<FetchResponse, livrarr_domain::services::FetchError> {
            tokio::task::yield_now().await;
            self.inner.fetch_ssrf_safe(request).await
        }
    }

    const WORK_WITHOUT_NAMES: &str = r#"{"authors":[{"author":{"key":"/authors/OL7001A"}}]}"#;
    const AUTHOR_RECORD: &str = r#"{"name":"Hydrated Name"}"#;

    /// A work record that credits a contributor by key alone is hydrated once,
    /// and a second work crediting the same person is served from the cache.
    #[tokio::test]
    async fn hydration_fetches_one_author_key_once_across_works() {
        let _guard = lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = queued(vec![
            ok(WORK_WITHOUT_NAMES),
            ok(AUTHOR_RECORD),
            ok(WORK_WITHOUT_NAMES),
        ]);
        let client = OpenLibraryClient::new(fetcher.clone());

        let first = client
            .fetch_work_authors("OL7000W".to_string(), RequestPriority::Low)
            .await
            .expect("first work");
        let second = client
            .fetch_work_authors("OL7002W".to_string(), RequestPriority::Low)
            .await
            .expect("second work");

        assert_eq!(first[0].name, "Hydrated Name");
        assert_eq!(second[0].name, "Hydrated Name");
        assert_eq!(
            fetcher.call_count(),
            3,
            "two work records plus one hydration; the second work reuses the cached name"
        );
        let requests = fetcher.requests();
        assert_eq!(
            requests[1].url,
            "https://openlibrary.org/authors/OL7001A.json"
        );
        assert_eq!(requests[1].rate_bucket, RateBucket::OpenLibrary);
        assert_eq!(requests[1].priority, RequestPriority::Low);
    }

    /// Two callers wanting the same key at the same time make one request and
    /// both receive the leader's answer.
    #[tokio::test]
    async fn concurrent_hydration_of_one_key_makes_one_request() {
        let _guard = lock_breaker(RateBucket::OpenLibrary).await;
        let inner = queued(vec![ok(AUTHOR_RECORD)]);
        let fetcher = SuspendingFetcher {
            inner: inner.clone(),
        };
        let hydrator = OpenLibraryAuthorHydrator::new();
        let key = ol_key("OL7001A");

        let (left, right) = tokio::join!(
            hydrator.name_for_key(&fetcher, &key, RequestPriority::Low),
            hydrator.name_for_key(&fetcher, &key, RequestPriority::Low),
        );

        assert_eq!(left.expect("leader"), "Hydrated Name");
        assert_eq!(right.expect("joiner"), "Hydrated Name");
        assert_eq!(
            inner.call_count(),
            1,
            "the joiner must not issue its own request"
        );
    }

    /// A failure is never remembered as an answer: the next pass asks again and
    /// takes the real name.
    #[tokio::test]
    async fn a_failed_hydration_is_not_negatively_cached() {
        let _guard = lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = queued(vec![status(503, vec![]), ok(AUTHOR_RECORD)]);
        let hydrator = OpenLibraryAuthorHydrator::new();
        let key = ol_key("OL7001A");

        let failed = hydrator
            .name_for_key(&fetcher, &key, RequestPriority::Low)
            .await;
        assert!(
            matches!(failed, Err(ProviderFetchError::Retryable { .. })),
            "a 503 is retryable, got {failed:?}"
        );

        let recovered = hydrator
            .name_for_key(&fetcher, &key, RequestPriority::Low)
            .await
            .expect("retry after a transient failure");
        assert_eq!(recovered, "Hydrated Name");
        assert_eq!(fetcher.call_count(), 2);
    }

    /// An empty name is a failure, not a nameless contributor — a ref with no
    /// name cannot be guarded.
    #[tokio::test]
    async fn an_empty_author_name_is_a_failure_and_is_not_cached() {
        let _guard = lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = queued(vec![ok(r#"{"name":"   "}"#), ok(AUTHOR_RECORD)]);
        let hydrator = OpenLibraryAuthorHydrator::new();
        let key = ol_key("OL7001A");

        let empty = hydrator
            .name_for_key(&fetcher, &key, RequestPriority::Low)
            .await;
        assert!(matches!(empty, Err(ProviderFetchError::Permanent(_))));

        let named = hydrator
            .name_for_key(&fetcher, &key, RequestPriority::Low)
            .await
            .expect("a later valid record");
        assert_eq!(named, "Hydrated Name");
        assert_eq!(fetcher.call_count(), 2);
    }

    /// A cached name stops being served once its TTL has passed.
    #[tokio::test]
    async fn a_cached_name_expires_and_is_fetched_again() {
        let _guard = lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = queued(vec![ok(AUTHOR_RECORD), ok(r#"{"name":"Renamed"}"#)]);
        let hydrator = OpenLibraryAuthorHydrator::new();
        let key = ol_key("OL7001A");

        tokio::time::pause();
        let first = hydrator
            .name_for_key(&fetcher, &key, RequestPriority::Low)
            .await
            .expect("first hydration");
        let cached = hydrator
            .name_for_key(&fetcher, &key, RequestPriority::Low)
            .await
            .expect("cached hydration");
        assert_eq!(first, "Hydrated Name");
        assert_eq!(cached, "Hydrated Name");
        assert_eq!(fetcher.call_count(), 1);

        tokio::time::advance(OL_AUTHOR_NAME_TTL + Duration::from_secs(1)).await;
        let refreshed = hydrator
            .name_for_key(&fetcher, &key, RequestPriority::Low)
            .await
            .expect("hydration after expiry");
        assert_eq!(refreshed, "Renamed");
        assert_eq!(fetcher.call_count(), 2);
    }

    const SEARCH_RESPONSE: &str = r#"{"numFound":1,"docs":[{
        "key":"OL7100A",
        "name":"Ursula K. Le Guin",
        "alternate_names":["Ursula K. Le Guin","Ursula Le Guin","","アーシュラ・K・ル・グイン"],
        "top_work":"The Left Hand of Darkness",
        "work_count":265
    }]}"#;

    /// Search evidence reaches review whole: aliases in response order, one per
    /// spelling family, plus the headline work and catalogue size.
    #[tokio::test]
    async fn author_search_preserves_distinct_aliases_and_top_work() {
        let _guard = lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = queued(vec![ok(SEARCH_RESPONSE)]);

        let candidates =
            open_library_author_search(&fetcher, "Ursula K. Le Guin", 10, RequestPriority::Low)
                .await
                .expect("author search");

        assert_eq!(candidates.len(), 1);
        let candidate = &candidates[0];
        assert_eq!(candidate.route_key.as_str(), "OL7100A");
        assert_eq!(candidate.name, "Ursula K. Le Guin");
        assert_eq!(
            candidate.alternate_names,
            ["Ursula Le Guin", "アーシュラ・K・ル・グイン"],
            "the primary name's own spelling and blanks contribute no alias"
        );
        assert_eq!(
            candidate.top_work.as_deref(),
            Some("The Left Hand of Darkness")
        );
        assert_eq!(candidate.work_count, Some(265));

        let requests = fetcher.requests();
        assert_eq!(requests[0].rate_bucket, RateBucket::OpenLibrary);
        assert_eq!(requests[0].priority, RequestPriority::Low);
        assert!(requests[0].url.contains("search/authors.json"));
        assert!(requests[0].url.contains("limit=10"));
    }

    /// A page of documents none of which is readable is drift, not an author
    /// nobody has heard of.
    #[tokio::test]
    async fn author_search_reports_unreadable_documents_rather_than_no_candidates() {
        let _guard = lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = queued(vec![ok(r#"{"docs":[{"nope":1},{"key":"not-a-key"}]}"#)]);

        let outcome =
            open_library_author_search(&fetcher, "Someone", 10, RequestPriority::Low).await;
        assert!(
            matches!(outcome, Err(ProviderFetchError::Permanent(_))),
            "got {outcome:?}"
        );
    }

    /// A genuinely empty result set is a successful empty answer.
    #[tokio::test]
    async fn author_search_returns_an_empty_result_set_as_success() {
        let _guard = lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = queued(vec![ok(r#"{"numFound":0,"docs":[]}"#)]);

        let candidates =
            open_library_author_search(&fetcher, "Nobody At All", 10, RequestPriority::Low)
                .await
                .expect("empty search is a success");
        assert!(candidates.is_empty());
    }

    /// The catalog walk carries its own cursor forward, and a readable page
    /// listing nothing ends it without ever looking like a failure.
    #[tokio::test]
    async fn catalog_paging_carries_the_cursor_and_ends_on_an_empty_page() {
        let _guard = lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = queued(vec![
            ok(
                r#"{"links":{"next":"/authors/OL7100A/works.json?limit=100&offset=100"},
                   "size":102,"entries":[{"title":"First"},{"title":"Second"}]}"#,
            ),
            ok(r#"{"links":{"self":"/authors/OL7100A/works.json"},"size":102,"entries":[]}"#),
        ]);
        let key = ol_key("OL7100A");

        let first = open_library_catalog_page(&fetcher, &key, None, RequestPriority::Low)
            .await
            .expect("first page");
        assert_eq!(first.titles, ["First", "Second"]);
        assert_eq!(first.next_cursor.as_deref(), Some("2"));

        let second = open_library_catalog_page(
            &fetcher,
            &key,
            first.next_cursor.as_deref(),
            RequestPriority::Low,
        )
        .await
        .expect("empty page is a success");
        assert!(second.titles.is_empty());
        assert_eq!(second.next_cursor, None);

        let requests = fetcher.requests();
        assert!(requests[0].url.contains("offset=0"));
        assert!(requests[1].url.contains("offset=2"));
        assert!(requests
            .iter()
            .all(|request| request.rate_bucket == RateBucket::OpenLibrary
                && request.priority == RequestPriority::Low));
    }

    /// A failed catalog read never becomes a catalog with nothing in it — that
    /// would read as evidence against a candidate rather than an absence of it.
    #[tokio::test]
    async fn a_failed_catalog_read_is_never_an_empty_page() {
        let _guard = lock_breaker(RateBucket::OpenLibrary).await;
        let key = ol_key("OL7100A");

        let unreadable = queued(vec![ok(r#"{"size":3}"#)]);
        let drift = open_library_catalog_page(&unreadable, &key, None, RequestPriority::Low).await;
        assert!(
            matches!(drift, Err(ProviderFetchError::Permanent(_))),
            "a response with no entries array is drift, got {drift:?}"
        );

        let rate_limited = queued(vec![status(
            429,
            vec![("Retry-After".to_string(), "120".to_string())],
        )]);
        let paused =
            open_library_catalog_page(&rate_limited, &key, None, RequestPriority::Low).await;
        match paused {
            Err(ProviderFetchError::Retryable {
                retry_not_before, ..
            }) => {
                let hint = retry_not_before.expect("Retry-After is preserved as an absolute time");
                let waited = (hint - Utc::now()).num_seconds();
                assert!(
                    (110..=120).contains(&waited),
                    "expected roughly 120s of wait, got {waited}s"
                );
            }
            other => panic!("expected a retryable pause, got {other:?}"),
        }
    }

    /// An HTTP-date `Retry-After` is honoured as well as a delay in seconds.
    #[test]
    fn retry_after_reads_both_documented_forms() {
        let delay = retry_after_hint(&[("retry-after".to_string(), "45".to_string())])
            .expect("delay seconds");
        assert!((40..=45).contains(&(delay - Utc::now()).num_seconds()));

        let dated = retry_after_hint(&[(
            "Retry-After".to_string(),
            "Wed, 21 Oct 2015 07:28:00 GMT".to_string(),
        )])
        .expect("HTTP-date");
        assert_eq!(dated.to_rfc3339(), "2015-10-21T07:28:00+00:00");

        assert!(retry_after_hint(&[]).is_none());
        assert!(
            retry_after_hint(&[("Retry-After".to_string(), "soonish".to_string())]).is_none(),
            "an unreadable header costs the hint, not the retry"
        );
    }

    /// The gateway dispatches each provider to its own keyed adapter.
    #[tokio::test]
    async fn the_gateway_routes_each_provider_to_its_own_adapter() {
        let _guard = lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = queued(vec![ok(
            r#"{"authors":[{"author":{"key":"/authors/OL7001A"},"name":"Named Inline"}]}"#,
        )]);
        let gateway = gateway(fetcher.clone(), metadata_config(false, None));

        let refs = gateway
            .fetch_work_authors(
                AuthorProvider::OpenLibrary,
                "OL7000W".to_string(),
                RequestPriority::Low,
            )
            .await
            .expect("OpenLibrary dispatch");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "Named Inline");
        assert!(fetcher.requests()[0]
            .url
            .starts_with("https://openlibrary.org/works/"));
    }

    /// Hardcover with no token is a configuration answer, not a provider
    /// failure, and it costs no request.
    #[tokio::test]
    async fn the_gateway_reports_a_tokenless_hardcover_as_not_configured() {
        let _guard = lock_breaker(RateBucket::Hardcover).await;
        let fetcher = RecordingHttpFetcher::new();
        let gateway = gateway(fetcher.clone(), metadata_config(true, None));

        let outcome = gateway
            .fetch_work_authors(
                AuthorProvider::Hardcover,
                "42".to_string(),
                RequestPriority::Low,
            )
            .await;
        assert!(
            matches!(outcome, Err(AuthorProviderError::NotConfigured)),
            "got {outcome:?}"
        );
        assert_eq!(fetcher.call_count(), 0);
    }

    /// A Hardcover response whose contributor association has moved is drift,
    /// never a book nobody wrote.
    #[tokio::test]
    async fn the_gateway_reports_an_unreadable_hardcover_association_as_layout_drift() {
        let _guard = lock_breaker(RateBucket::Hardcover).await;
        let fetcher = queued(vec![ok(
            r#"{"data":{"editions":[{"contributions":[{"author":{"name":"No Id"}}]}]}}"#,
        )]);
        let gateway = gateway(fetcher, metadata_config(true, Some("token")));

        let outcome = gateway
            .fetch_work_authors(
                AuthorProvider::Hardcover,
                "42".to_string(),
                RequestPriority::Low,
            )
            .await;
        assert!(
            matches!(outcome, Err(AuthorProviderError::LayoutDrift(_))),
            "got {outcome:?}"
        );
    }

    /// A local pause — an open breaker or a full queue — reaches the road as a
    /// retry with the wait the queue itself reported.
    #[test]
    fn a_local_pause_maps_to_a_retry_with_its_own_deadline() {
        let mapped = map_gateway_error(ProviderFetchError::CircuitOpen(Duration::from_secs(90)));
        match mapped {
            AuthorProviderError::Retryable {
                retry_not_before, ..
            } => {
                let hint = retry_not_before.expect("a pause knows when it ends");
                assert!((80..=90).contains(&(hint - Utc::now()).num_seconds()));
            }
            other => panic!("expected a retryable pause, got {other:?}"),
        }

        let queue_full = map_gateway_error(ProviderFetchError::QueueFull(Duration::from_secs(5)));
        assert!(matches!(
            queue_full,
            AuthorProviderError::Retryable {
                retry_not_before: Some(_),
                ..
            }
        ));
    }

    // -----------------------------------------------------------------------
    // Role certification — what each provider actually said about a credit
    // -----------------------------------------------------------------------

    fn hardcover(fetcher: RecordingHttpFetcher) -> HardcoverClient<RecordingHttpFetcher> {
        HardcoverClient::new(
            fetcher,
            LiveMetadataConfig::new(metadata_config(true, Some("token"))),
        )
    }

    fn goodreads(fetcher: RecordingHttpFetcher) -> GoodreadsClient<RecordingHttpFetcher> {
        let http = livrarr_http::HttpClient::builder()
            .user_agent("livrarr-author-gateway-test")
            .build()
            .expect("test HTTP client");
        GoodreadsClient::new(fetcher, http, "https://www.goodreads.com")
    }

    /// One Goodreads book page in the current layout, with the contributor
    /// edges spelled the way the live Apollo cache spells them.
    fn goodreads_page(edges: &str) -> String {
        format!(
            r#"<html><script id="__NEXT_DATA__" type="application/json">
            {{"props":{{"pageProps":{{"apolloState":{{
                "Book:kca://book/1":{{{edges}}},
                "Contributor:kca://author/1":{{"name":"First Person","legacyId":31}},
                "Contributor:kca://author/2":{{"name":"Second Person","legacyId":32}}
            }}}}}}}}</script></html>"#
        )
    }

    /// Hardcover says "author" by sending the contribution field with no value.
    /// A named contribution is a different credit and keeps its own label.
    #[tokio::test]
    async fn hardcover_certifies_a_null_contribution_as_author_and_a_label_verbatim() {
        let _guard = lock_breaker(RateBucket::Hardcover).await;
        let fetcher = queued(vec![ok(r#"{"data":{"editions":[{"contributions":[
                {"contribution":null,"author":{"id":41,"name":"The Author"}},
                {"contribution":"Narrated by","author":{"id":42,"name":"The Narrator"}}
            ]}]}}"#)]);

        let refs = hardcover(fetcher)
            .fetch_work_authors("42".to_string(), RequestPriority::Low)
            .await
            .expect("Hardcover contributor read");

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].name, "The Author");
        assert_eq!(refs[0].role.as_deref(), Some("author"));
        assert_eq!(refs[1].name, "The Narrator");
        assert_eq!(refs[1].role.as_deref(), Some("Narrated by"));
    }

    /// A contribution field that is absent, or present in a shape this reader
    /// does not know, is not a credit it can certify — the entry is dropped
    /// with a warning rather than guessed into an author.
    #[tokio::test]
    async fn hardcover_drops_an_entry_whose_contribution_shape_is_unreadable() {
        let _guard = lock_breaker(RateBucket::Hardcover).await;
        let fetcher = queued(vec![ok(r#"{"data":{"editions":[{"contributions":[
                {"author":{"id":41,"name":"No Contribution Field"}},
                {"contribution":{"role":"author"},"author":{"id":42,"name":"Unknown Shape"}},
                {"contribution":null,"author":{"id":43,"name":"The Author"}}
            ]}]}}"#)]);

        let refs = hardcover(fetcher)
            .fetch_work_authors("42".to_string(), RequestPriority::Low)
            .await
            .expect("a mixed response still answers");

        assert_eq!(refs.len(), 1, "only the readable credit survives");
        assert_eq!(refs[0].name, "The Author");
        assert_eq!(refs[0].role.as_deref(), Some("author"));
    }

    /// Every entry unreadable is the shape moving, not a book nobody wrote.
    /// Answering `Ok([])` here would make the keyed read terminally successful
    /// on fabricated emptiness (insight 62).
    #[tokio::test]
    async fn hardcover_reports_drift_when_every_contribution_shape_is_unreadable() {
        let _guard = lock_breaker(RateBucket::Hardcover).await;
        let fetcher = queued(vec![ok(r#"{"data":{"editions":[{"contributions":[
                {"author":{"id":41,"name":"First Person"}},
                {"author":{"id":42,"name":"Second Person"}}
            ]}]}}"#)]);

        let outcome = hardcover(fetcher)
            .fetch_work_authors("42".to_string(), RequestPriority::Low)
            .await;

        assert!(
            matches!(outcome, Err(ProviderFetchError::LayoutDrift(_))),
            "got {outcome:?}"
        );
    }

    /// A book that genuinely credits nobody is still a readable answer.
    #[tokio::test]
    async fn hardcover_keeps_a_genuinely_empty_contribution_list_a_success() {
        let _guard = lock_breaker(RateBucket::Hardcover).await;
        let fetcher = queued(vec![ok(r#"{"data":{"editions":[{"contributions":[]}]}}"#)]);

        let refs = hardcover(fetcher)
            .fetch_work_authors("42".to_string(), RequestPriority::Low)
            .await
            .expect("an empty credit list is readable");

        assert!(refs.is_empty());
    }

    /// One person credited on two editions is one route. Any edition that
    /// credited them as the author makes the route authorial, whichever
    /// edition came back first.
    #[tokio::test]
    async fn hardcover_aggregates_one_person_across_editions_in_either_order() {
        let _guard = lock_breaker(RateBucket::Hardcover).await;
        let narrator_first = queued(vec![ok(r#"{"data":{"editions":[
                {"contributions":[{"contribution":"Narrated by","author":{"id":41,"name":"Both Credits"}}]},
                {"contributions":[{"contribution":null,"author":{"id":41,"name":"Both Credits"}}]}
            ]}}"#)]);
        let refs = hardcover(narrator_first)
            .fetch_work_authors("42".to_string(), RequestPriority::Low)
            .await
            .expect("narrator credit first");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].role.as_deref(), Some("author"));

        let author_first = queued(vec![ok(r#"{"data":{"editions":[
                {"contributions":[{"contribution":null,"author":{"id":41,"name":"Both Credits"}}]},
                {"contributions":[{"contribution":"Narrated by","author":{"id":41,"name":"Both Credits"}}]}
            ]}}"#)]);
        let refs = hardcover(author_first)
            .fetch_work_authors("42".to_string(), RequestPriority::Low)
            .await
            .expect("author credit first");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].role.as_deref(), Some("author"));
    }

    /// The order the provider credited people in is the order they come back
    /// in, and a person credited only as a narrator keeps that label.
    #[tokio::test]
    async fn hardcover_aggregation_preserves_first_seen_order_and_other_labels() {
        let _guard = lock_breaker(RateBucket::Hardcover).await;
        let fetcher = queued(vec![ok(r#"{"data":{"editions":[{"contributions":[
                {"contribution":"Narrated by","author":{"id":41,"name":"Only Narrator"}},
                {"contribution":null,"author":{"id":42,"name":"The Author"}},
                {"contribution":"Illustrated by","author":{"id":41,"name":"Only Narrator"}}
            ]}]}}"#)]);

        let refs = hardcover(fetcher)
            .fetch_work_authors("42".to_string(), RequestPriority::Low)
            .await
            .expect("ordered contributor read");

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].name, "Only Narrator");
        assert_eq!(
            refs[0].role.as_deref(),
            Some("Narrated by"),
            "a non-authorial route keeps the label it was first credited with"
        );
        assert_eq!(refs[1].name, "The Author");
        assert_eq!(refs[1].role.as_deref(), Some("author"));
    }

    /// Goodreads names the credit on the edge. "Author" — in either of the two
    /// spellings the site uses — is an author credit; anything else is that
    /// other credit, verbatim.
    #[tokio::test]
    async fn goodreads_certifies_author_edges_and_keeps_other_roles_verbatim() {
        let _guard = lock_breaker(RateBucket::Goodreads).await;
        let page = goodreads_page(
            r#""primaryContributorEdge":{"node":{"__ref":"Contributor:kca://author/1"},"role":"Goodreads Author"},
               "secondaryContributorEdges":[{"node":{"__ref":"Contributor:kca://author/2"},"role":"Translator"}]"#,
        );
        let fetcher = queued(vec![ok(&page)]);

        let refs = goodreads(fetcher)
            .fetch_work_authors("9300".to_string(), RequestPriority::Low)
            .await
            .expect("Goodreads contributor read");

        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].name, "First Person");
        assert_eq!(refs[0].role.as_deref(), Some("author"));
        assert_eq!(refs[1].name, "Second Person");
        assert_eq!(refs[1].role.as_deref(), Some("Translator"));
    }

    /// An edge with no role said nothing about the credit. Dropping it is the
    /// only reading that cannot invent an author.
    #[tokio::test]
    async fn goodreads_drops_an_edge_that_names_no_role() {
        let _guard = lock_breaker(RateBucket::Goodreads).await;
        let page = goodreads_page(
            r#""primaryContributorEdge":{"node":{"__ref":"Contributor:kca://author/1"},"role":"Author"},
               "secondaryContributorEdges":[{"node":{"__ref":"Contributor:kca://author/2"},"role":"   "}]"#,
        );
        let fetcher = queued(vec![ok(&page)]);

        let refs = goodreads(fetcher)
            .fetch_work_authors("9300".to_string(), RequestPriority::Low)
            .await
            .expect("a mixed response still answers");

        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, "First Person");
        assert_eq!(refs[0].role.as_deref(), Some("author"));
    }

    /// Every edge roleless is the layout having moved.
    #[tokio::test]
    async fn goodreads_reports_drift_when_no_edge_names_a_role() {
        let _guard = lock_breaker(RateBucket::Goodreads).await;
        let page = goodreads_page(
            r#""primaryContributorEdge":{"node":{"__ref":"Contributor:kca://author/1"}},
               "secondaryContributorEdges":[{"node":{"__ref":"Contributor:kca://author/2"}}]"#,
        );
        let fetcher = queued(vec![ok(&page)]);

        let outcome = goodreads(fetcher)
            .fetch_work_authors("9300".to_string(), RequestPriority::Low)
            .await;

        assert!(
            matches!(outcome, Err(ProviderFetchError::LayoutDrift(_))),
            "got {outcome:?}"
        );
    }

    /// The JSON-LD fallback has no roles because the field it reads *is* the
    /// author list — every entry in it is an author credit.
    #[tokio::test]
    async fn goodreads_json_ld_entries_are_author_credits() {
        let _guard = lock_breaker(RateBucket::Goodreads).await;
        let fetcher = queued(vec![ok(r#"<html><script type="application/ld+json">
               {"author":[{"@type":"Person","name":"First Person","url":"/author/show/31"},
                          {"@type":"Person","name":"Second Person","url":"/author/show/32"}]}
               </script></html>"#)]);

        let refs = goodreads(fetcher)
            .fetch_work_authors("9300".to_string(), RequestPriority::Low)
            .await
            .expect("JSON-LD contributor read");

        assert_eq!(refs.len(), 2);
        assert!(refs
            .iter()
            .all(|entry| entry.role.as_deref() == Some("author")));
    }

    /// Open Library spells an ordinary author credit `/type/author_role`, and
    /// an entry with no type at all is still in the work's author list.
    #[tokio::test]
    async fn open_library_certifies_author_role_and_a_missing_type_as_author() {
        let _guard = lock_breaker(RateBucket::OpenLibrary).await;
        let fetcher = queued(vec![ok(r#"{"authors":[
                {"type":{"key":"/type/author_role"},
                 "author":{"key":"/authors/OL7001A"},"name":"Typed Author"},
                {"author":{"key":"/authors/OL7002A"},"name":"Untyped Author"},
                {"type":{"key":"/type/translator_role"},
                 "author":{"key":"/authors/OL7003A"},"name":"Translator Person"}
            ]}"#)]);

        let refs = OpenLibraryClient::new(fetcher)
            .fetch_work_authors("OL7000W".to_string(), RequestPriority::Low)
            .await
            .expect("OpenLibrary contributor read");

        assert_eq!(refs.len(), 3);
        assert_eq!(refs[0].role.as_deref(), Some("author"));
        assert_eq!(
            refs[1].role.as_deref(),
            Some("author"),
            "the authors[] container is the work's author list"
        );
        assert_eq!(refs[2].role.as_deref(), Some("translator_role"));
    }
}
