use futures::stream::{self, StreamExt};
use livrarr_db::{ConfigDb, CreateNotificationDbRequest, NotificationDb, WorkDb};
use livrarr_domain::services::*;
use livrarr_domain::*;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use chrono::Datelike;

/// OpenLibrary author works response — minimal parsing for monitor.
#[derive(Debug, serde::Deserialize)]
struct OlWorksResponse {
    entries: Vec<OlWorkEntry>,
}

#[derive(Debug, serde::Deserialize)]
struct OlWorkEntry {
    key: Option<String>,
    title: Option<String>,
    /// OL uses "first_publish_date" which may be a year string like "2024"
    /// or a full date like "January 1, 2024".
    first_publish_date: Option<String>,
    /// Credited authors — present in the payload the monitor already fetches;
    /// the anthology screen counts them (REQ-004a).
    authors: Option<Vec<OlEntryAuthor>>,
    /// OL cover IDs — first positive ID is the primary cover.
    covers: Option<Vec<i64>>,
}

/// One credited-author entry. The screen needs only the count, so nothing is
/// deserialized from it.
#[derive(Debug, serde::Deserialize)]
struct OlEntryAuthor {}

impl OlWorkEntry {
    fn ol_key(&self) -> Option<&str> {
        // key is like "/works/OL12345W" — extract "OL12345W"
        self.key.as_deref().and_then(|k| k.strip_prefix("/works/"))
    }

    fn publish_year(&self) -> Option<i32> {
        self.first_publish_date.as_deref().and_then(|d| {
            // Extract first 4-digit numeric token (matches handler behavior)
            d.split(|c: char| !c.is_ascii_digit())
                .find(|tok| tok.len() == 4)
                .and_then(|tok| tok.parse::<i32>().ok())
        })
    }

    /// Credited-author count; a missing or empty array counts as 1, so the
    /// anthology class never fires on it (REQ-004a).
    fn author_count(&self) -> usize {
        match self.authors.as_ref() {
            Some(v) if !v.is_empty() => v.len(),
            _ => 1,
        }
    }
}

/// Anthology threshold (REQ-004a): every junk anthology in the ST-007 sample
/// had ≥6 credited authors; the max observed on a clean work is 5.
const ANTHOLOGY_AUTHOR_THRESHOLD: usize = 6;

/// Title keywords that mark publisher bundles (REQ-004b).
const BUNDLE_KEYWORDS: [&str; 6] = [
    "omnibus",
    "box set",
    "boxed set",
    "collection set",
    "series set",
    "books in one",
];

/// High-precision study-guide keywords (REQ-004e). "Notes on" and
/// "Analysis of" are deliberately absent — they match real literary titles.
const SUMMARY_KEYWORDS: [&str; 6] = [
    "summary of",
    "study guide",
    "sparknotes",
    "cliffsnotes",
    "workbook",
    "quotes from",
];

/// Bundle vocabulary + connective stopwords stripped by the self-titled
/// bundle rule (REQ-004c).
const SELF_TITLED_STRIP: [&str; 13] = [
    "set",
    "box",
    "boxed",
    "collection",
    "books",
    "novels",
    "omnibus",
    "volume",
    "by",
    "the",
    "of",
    "a",
    "and",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JunkClass {
    Anthology,
    BundleKeyword,
    SelfTitledBundle,
    MalformedTitle,
    SummaryKeyword,
    MissingTitle,
}

impl JunkClass {
    fn as_str(self) -> &'static str {
        match self {
            JunkClass::Anthology => "anthology",
            JunkClass::BundleKeyword => "bundle_keyword",
            JunkClass::SelfTitledBundle => "self_titled_bundle",
            JunkClass::MalformedTitle => "malformed_title",
            JunkClass::SummaryKeyword => "summary_keyword",
            JunkClass::MissingTitle => "missing_title",
        }
    }
}

/// REQ-004/REQ-005 quality screen: decides whether a bibliography entry looks
/// like a real primary work. Deterministic — no network, no LLM. Returns the
/// matched junk class, or `None` for a clean entry.
fn screen_entry(entry: &OlWorkEntry, author_name: &str) -> Option<JunkClass> {
    let title = match entry.title.as_deref() {
        Some(t) if !crate::title_cleanup::clean_title(t).is_empty() => t,
        _ => return Some(JunkClass::MissingTitle),
    };
    if entry.author_count() >= ANTHOLOGY_AUTHOR_THRESHOLD {
        return Some(JunkClass::Anthology);
    }
    // Malformed entries are screened, not repaired — no guessing a plausible
    // title (REQ-004d).
    if title.contains('\n') || title.contains("  ") {
        return Some(JunkClass::MalformedTitle);
    }
    let lower = title.to_lowercase();
    if BUNDLE_KEYWORDS.iter().any(|k| lower.contains(k)) || books_range_form(&lower) {
        return Some(JunkClass::BundleKeyword);
    }
    if SUMMARY_KEYWORDS.iter().any(|k| lower.contains(k)) {
        return Some(JunkClass::SummaryKeyword);
    }
    if is_self_titled_bundle(&lower, author_name) {
        return Some(JunkClass::SelfTitledBundle);
    }
    None
}

/// Matches "books 1-4" / "books 1–4" range forms (REQ-004b).
fn books_range_form(lower: &str) -> bool {
    let Some(idx) = lower.find("books") else {
        return false;
    };
    let rest = lower[idx + 5..].trim_start();
    let digits_end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    if digits_end == 0 {
        return false;
    }
    let mut after = rest[digits_end..].trim_start().chars();
    matches!(after.next(), Some('-' | '–' | '—'))
        && after
            .as_str()
            .trim_start()
            .starts_with(|c: char| c.is_ascii_digit())
}

/// REQ-004c: normalized title minus the author's name tokens, bundle
/// vocabulary, stopwords, and digits — an empty remainder means the title is
/// just a self-titled bundle label ("Jim Butcher Set"). A real title that
/// merely contains the author's name keeps a substantive token and passes
/// ("Persuasion by Jane Austen" → "persuasion").
fn is_self_titled_bundle(lower_title: &str, author_name: &str) -> bool {
    let author_lower = author_name.to_lowercase();
    let author_tokens: std::collections::HashSet<&str> = author_lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .collect();
    let mut saw_any = false;
    for tok in lower_title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        saw_any = true;
        if author_tokens.contains(tok)
            || SELF_TITLED_STRIP.contains(&tok)
            || tok.chars().all(|c| c.is_ascii_digit())
        {
            continue;
        }
        return false;
    }
    saw_any
}

pub struct NoIdentityRoad;

pub struct ConfiguredIdentityRoad<R>(Arc<R>);

#[trait_variant::make(AuthorMonitorIdentityRoad: Send)]
pub trait LocalAuthorMonitorIdentityRoad: Send + Sync {
    async fn settle(
        &self,
        request: livrarr_domain::identity_layer::IdentityRoadRequest,
    ) -> Option<
        Result<
            livrarr_domain::identity_layer::IdentityRoadOutcome,
            livrarr_domain::identity_layer::IdentityRoadError,
        >,
    >;
}

impl AuthorMonitorIdentityRoad for NoIdentityRoad {
    async fn settle(
        &self,
        _request: livrarr_domain::identity_layer::IdentityRoadRequest,
    ) -> Option<
        Result<
            livrarr_domain::identity_layer::IdentityRoadOutcome,
            livrarr_domain::identity_layer::IdentityRoadError,
        >,
    > {
        None
    }
}

impl<R> AuthorMonitorIdentityRoad for ConfiguredIdentityRoad<R>
where
    R: livrarr_domain::identity_layer::IdentityRoadService + Send + Sync,
{
    async fn settle(
        &self,
        request: livrarr_domain::identity_layer::IdentityRoadRequest,
    ) -> Option<
        Result<
            livrarr_domain::identity_layer::IdentityRoadOutcome,
            livrarr_domain::identity_layer::IdentityRoadError,
        >,
    > {
        Some(self.0.settle(request).await)
    }
}

pub struct AuthorMonitorWorkflowImpl<D, W, H, R = NoIdentityRoad> {
    db: Arc<D>,
    work_service: Arc<W>,
    http: Arc<H>,
    identity_road: R,
    backoff_duration: Duration,
    inter_author_delay: Duration,
    running: AtomicBool,
}

impl<D, W, H> AuthorMonitorWorkflowImpl<D, W, H, NoIdentityRoad> {
    pub fn new(db: Arc<D>, work_service: Arc<W>, http: Arc<H>) -> Self {
        Self {
            db,
            work_service,
            http,
            identity_road: NoIdentityRoad,
            backoff_duration: Duration::from_secs(60),
            inter_author_delay: Duration::from_secs(1),
            running: AtomicBool::new(false),
        }
    }
}

impl<D, W, H, R> AuthorMonitorWorkflowImpl<D, W, H, ConfiguredIdentityRoad<R>> {
    pub fn with_identity_road(
        db: Arc<D>,
        work_service: Arc<W>,
        http: Arc<H>,
        identity_road: Arc<R>,
    ) -> Self {
        Self {
            db,
            work_service,
            http,
            identity_road: ConfiguredIdentityRoad(identity_road),
            backoff_duration: Duration::from_secs(60),
            inter_author_delay: Duration::from_secs(1),
            running: AtomicBool::new(false),
        }
    }
}

impl<D, W, H, R> AuthorMonitorWorkflowImpl<D, W, H, R> {
    pub fn with_backoff(mut self, backoff: Duration, inter_author: Duration) -> Self {
        self.backoff_duration = backoff;
        self.inter_author_delay = inter_author;
        self
    }
}

impl<D, W, H, R> AuthorMonitorWorkflow for AuthorMonitorWorkflowImpl<D, W, H, R>
where
    D: WorkDb + livrarr_db::AuthorDb + NotificationDb + ConfigDb + Send + Sync + 'static,
    W: WorkService + Send + Sync + 'static,
    H: HttpFetcher + Send + Sync + 'static,
    R: AuthorMonitorIdentityRoad + Send + Sync + 'static,
{
    async fn run_monitor(
        &self,
        user_id: UserId,
        cancel: CancellationToken,
    ) -> Result<MonitorReport, MonitorError> {
        if self.running.swap(true, Ordering::AcqRel) {
            tracing::info!("author monitor already running, skipping");
            return Err(MonitorError::AlreadyRunning);
        }
        let result = self.run_monitor_inner(user_id, cancel).await;
        self.running.store(false, Ordering::Release);
        result
    }
}

impl<D, W, H, R> AuthorMonitorWorkflowImpl<D, W, H, R>
where
    D: WorkDb + livrarr_db::AuthorDb + NotificationDb + ConfigDb + Send + Sync + 'static,
    W: WorkService + Send + Sync + 'static,
    H: HttpFetcher + Send + Sync + 'static,
    R: AuthorMonitorIdentityRoad + Send + Sync + 'static,
{
    async fn run_monitor_inner(
        &self,
        user_id: UserId,
        cancel: CancellationToken,
    ) -> Result<MonitorReport, MonitorError> {
        // One target per monitored author, carrying every active OpenLibrary
        // route (FP-038). Plural routes widen the feed a person is watched
        // through; they never turn one author into several monitor entries, and
        // they never multiply what gets added or reported.
        let targets = self
            .db
            .list_author_monitor_targets(user_id)
            .await
            .map_err(MonitorError::Db)?;

        // The user's default language, read once per monitor run: new works
        // whose author has no monitor_language choice seed this value.
        let default_language = self
            .db
            .get_default_language()
            .await
            .map_err(MonitorError::Db)?;

        let mut report = MonitorReport {
            authors_checked: 0,
            new_works_found: 0,
            works_added: 0,
            notifications_created: 0,
            entries_screened: 0,
        };

        // Index-based loop with retry map for 429 handling (matches handler).
        let mut i = 0;
        let mut retry_counts: HashMap<usize, u32> = HashMap::new();
        let mut rate_limit_notified = false;

        while i < targets.len() {
            let author = &targets[i].author;
            let ol_routes: Vec<String> = targets[i]
                .ol_routes
                .iter()
                .map(|route| route.key.value())
                .collect();

            // Only count each author once (not on retries)
            if !retry_counts.contains_key(&i) {
                report.authors_checked += 1;
            }

            // Every active route's feed is read, then unioned by canonical OL
            // work key, and only then does anything get screened, added, or
            // notified (FP-038). First route wins a duplicate, so the seed's
            // author key is stable across runs. A route that fails is warned
            // about and its siblings' results stand; only a rate limit pauses the
            // whole author, because that is a signal about the provider, not
            // about one feed.
            let mut union: Vec<OlWorkEntry> = Vec::new();
            let mut union_keys: Vec<String> = Vec::new();
            let mut seed_route: Vec<String> = Vec::new();
            let mut successful_routes = 0usize;
            let mut rate_limited = false;

            for ol_key in &ol_routes {
                let works_url = format!(
                    "https://openlibrary.org/authors/{}/works.json?limit=100",
                    ol_key
                );

                let req = FetchRequest {
                    url: works_url,
                    method: HttpMethod::Get,
                    headers: vec![],
                    body: None,
                    timeout: Duration::from_secs(30),
                    rate_bucket: RateBucket::OpenLibrary,
                    max_body_bytes: 2 * 1024 * 1024,
                    anti_bot_check: false,
                    user_agent: UserAgentProfile::Server,
                    // Low: background author monitor scan (B4 table).
                    priority: RequestPriority::Low,
                };

                let resp = match self.http.fetch(req).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!(
                            author_id = author.id,
                            author_name = %author.name,
                            %ol_key,
                            error = %e,
                            "author monitor: OL request failed for one route, keeping siblings"
                        );
                        continue;
                    }
                };

                if resp.status == 429 {
                    rate_limited = true;
                    break;
                }

                if resp.status >= 400 {
                    tracing::warn!(
                        author_id = author.id,
                        author_name = %author.name,
                        %ol_key,
                        status = resp.status,
                        "author monitor: OL returned non-success status for one route, keeping siblings"
                    );
                    continue;
                }

                let works_response: OlWorksResponse = match serde_json::from_slice(&resp.body) {
                    Ok(parsed) => parsed,
                    Err(e) => {
                        tracing::warn!(
                            author_id = author.id,
                            author_name = %author.name,
                            %ol_key,
                            error = %e,
                            "author monitor: OL parse error for one route, keeping siblings"
                        );
                        continue;
                    }
                };

                successful_routes += 1;
                for entry in works_response.entries {
                    // No canonical work key is no identity to union on: keep the
                    // entry and let the screen and the local dedup judge it.
                    let identity = entry.ol_key().map(str::to_string);
                    if let Some(key) = identity {
                        if union_keys.contains(&key) {
                            continue;
                        }
                        union_keys.push(key);
                    }
                    union.push(entry);
                    seed_route.push(ol_key.clone());
                }
            }

            if rate_limited {
                let retries = retry_counts.entry(i).or_insert(0);
                *retries += 1;
                if *retries > 3 {
                    tracing::warn!(
                        author_id = author.id,
                        author_name = %author.name,
                        "author monitor: OL 429 — max retries exceeded, skipping"
                    );
                    i += 1;
                    continue;
                }
                tracing::warn!(
                    author_id = author.id,
                    author_name = %author.name,
                    attempt = *retries,
                    "author monitor: OL 429 — backing off (attempt {}/3)",
                    retries
                );

                // Rate-limit notification on first 429 per run — notify the affected author's owner.
                if !rate_limit_notified {
                    rate_limit_notified = true;
                    if let Err(e) = self
                        .db
                        .create_notification(CreateNotificationDbRequest {
                            user_id: author.user_id,
                            notification_type: NotificationType::RateLimitHit,
                            ref_key: Some("author_monitor".into()),
                            message: "Open Library rate limit hit during author monitoring".into(),
                            data: serde_json::Value::Null,
                        })
                        .await
                    {
                        tracing::warn!("create_notification failed: {e}");
                    }
                }

                // Cancellation-aware backoff
                tokio::select! {
                    _ = tokio::time::sleep(self.backoff_duration) => {},
                    _ = cancel.cancelled() => { return Ok(report); },
                }
                // Retry same author (don't increment i).
                continue;
            }

            if successful_routes == 0 {
                tracing::warn!(
                    author_id = author.id,
                    author_name = %author.name,
                    routes = ol_routes.len(),
                    "author monitor: every OpenLibrary route failed, skipping author"
                );
                i += 1;
                continue;
            }

            let works_response = OlWorksResponse { entries: union };

            // Determine monitor_since year
            let monitor_since_year = author.monitor_since.map(|dt| dt.year()).unwrap_or(0);

            // Get existing work provider keys for dedup (checks both OL and GR keys)
            let existing_keys = self
                .db
                .list_work_provider_keys_by_author(author.user_id, author.id)
                .await
                .unwrap_or_default();

            let cleaned_author = crate::title_cleanup::clean_author(&author.name);

            // M9 bounded concurrency: filter eligible entries serially (cheap
            // CPU-only dedup + year filter), then auto-add or notify with
            // buffer_unordered(5) so up to 5 work_service.add() calls run in
            // parallel. Each future owns its own request and returns deltas;
            // the serial post-pass folds into `report`.
            let cleaned_author_ref = &cleaned_author;
            let author_ref = &author;
            let identity_road_ref = &self.identity_road;
            let default_language_ref = &default_language;
            let mut entries_screened = 0usize;
            let eligible: Vec<(String, i32, String, Option<String>, String)> = works_response
                .entries
                .iter()
                .enumerate()
                .filter_map(|(idx, entry)| {
                    let stripped_ol_key = entry.ol_key()?.to_string();
                    // The route that first surfaced this work seeds it, so the
                    // seed's author key does not depend on which feed answered
                    // fastest.
                    let seed_ol_key = seed_route[idx].clone();
                    if existing_keys
                        .iter()
                        .any(|(ol, _gr)| ol.as_deref() == Some(stripped_ol_key.as_str()))
                    {
                        return None;
                    }
                    // REQ-004/REQ-005 quality screen — before the year gate (so a
                    // screened entry never counts as "found") and before the
                    // auto-add/notification fork (a reject produces neither).
                    if let Some(class) = screen_entry(entry, &author.name) {
                        entries_screened += 1;
                        tracing::debug!(
                            ol_key = %stripped_ol_key,
                            title = ?entry.title,
                            class = class.as_str(),
                            "author monitor: entry screened out"
                        );
                        return None;
                    }
                    let year = match entry.publish_year() {
                        Some(y) => y,
                        None => {
                            tracing::trace!(
                                ol_key = %stripped_ol_key,
                                raw_date = ?entry.first_publish_date,
                                "author monitor: skipping work — no publish date"
                            );
                            return None;
                        }
                    };
                    if year < monitor_since_year {
                        return None;
                    }
                    let raw_title = entry.title.as_deref().unwrap_or("Unknown").to_string();
                    let work_title = crate::title_cleanup::clean_title(&raw_title);
                    let cover_url = entry
                        .covers
                        .as_deref()
                        .and_then(|cs| cs.iter().find(|&&id| id > 0))
                        .map(|id| format!("https://covers.openlibrary.org/b/id/{id}-L.jpg"));
                    Some((stripped_ol_key, year, work_title, cover_url, seed_ol_key))
                })
                .collect();

            report.entries_screened += entries_screened;
            report.new_works_found += eligible.len();

            struct EntryOutcome {
                works_added: bool,
                notifications_created: bool,
            }

            let outcomes: Vec<EntryOutcome> = stream::iter(eligible.into_iter())
                .map(
                    |(stripped_ol_key, year, work_title, cover_url, seed_ol_key)| async move {
                        tracing::info!(
                            author_id = author_ref.id,
                            year = year,
                            "author monitor: new work detected"
                        );

                        if author_ref.monitor_new_items {
                            let identity_title =
                                match livrarr_domain::identity_layer::title_parts_from_provider(
                                    work_title.clone(),
                                    None,
                                ) {
                                    Ok(title) => title,
                                    Err(error) => {
                                        tracing::warn!(author_id = author_ref.id, %error, "author monitor: invalid provider title");
                                        return EntryOutcome {
                                            works_added: false,
                                            notifications_created: false,
                                        };
                                    }
                                };
                            let road_request =
                                livrarr_domain::identity_layer::IdentityRoadRequest {
                                    user_id: author_ref.user_id,
                                    origin: livrarr_domain::identity_layer::IdentityRoadOrigin::CreationDoor(
                                        livrarr_domain::identity_layer::DoorKind::AuthorMonitor,
                                    ),
                                    evidence: livrarr_domain::identity_layer::IdentityEvidenceBundle {
                                        user_choice: None,
                                        owned_files: Vec::new(),
                                        provider_identity: vec![livrarr_domain::identity_layer::ProviderIdentityEvidence {
                                            provider: livrarr_domain::identity_layer::IdentityProvider::OpenLibrary,
                                            route: livrarr_domain::identity_layer::RouteKey {
                                                provider: livrarr_domain::identity_layer::IdentityProvider::OpenLibrary,
                                                kind: livrarr_domain::identity_layer::RouteKind::OpenLibraryWork,
                                                value: stripped_ol_key.clone(),
                                            },
                                            work_core: Some(livrarr_domain::identity_layer::ProviderWorkIdentityCore {
                                                identity_title,
                                                primary_author_id: author_ref.id,
                                            }),
                                            provenance: Default::default(),
                                        }],
                                        minimum: None,
                                    },
                                    interaction: livrarr_domain::identity_layer::IdentityRoadInteraction::MachineAlone,
                                    existing_work_id: None,
                                };
                            if let Some(road_result) = identity_road_ref.settle(road_request).await {
                                let work_id = match road_result {
                                    Ok(livrarr_domain::identity_layer::IdentityRoadOutcome::Settled {
                                        work_id,
                                        ..
                                    }) => work_id,
                                    Ok(other) => {
                                        tracing::warn!(author_id = author_ref.id, outcome = ?other, "author monitor: identity road did not settle work");
                                        return EntryOutcome {
                                            works_added: false,
                                            notifications_created: false,
                                        };
                                    }
                                    Err(error) => {
                                        tracing::warn!(author_id = author_ref.id, %error, "author monitor: identity road failed");
                                        return EntryOutcome {
                                            works_added: false,
                                            notifications_created: false,
                                        };
                                    }
                                };
                                let ws = self.work_service.clone();
                                let uid = author_ref.user_id;
                                tokio::spawn(async move {
                                    let _ = ws.converge_work(uid, work_id, 3).await;
                                });
                                let notif_ok = self
                                    .db
                                    .create_notification(CreateNotificationDbRequest {
                                        user_id: author_ref.user_id,
                                        notification_type: NotificationType::WorkAutoAdded,
                                        ref_key: Some(stripped_ol_key.clone()),
                                        message: format!(
                                            "New work '{}' by {} auto-added to your library",
                                            work_title, author_ref.name
                                        ),
                                        data: serde_json::json!({
                                            "title": work_title,
                                            "author": author_ref.name,
                                            "year": year,
                                            "ol_key": stripped_ol_key,
                                        }),
                                    })
                                    .await
                                    .map_err(|error| {
                                        tracing::warn!("create_notification failed: {error}")
                                    })
                                    .is_ok();
                                return EntryOutcome {
                                    works_added: true,
                                    notifications_created: notif_ok,
                                };
                            }

                            use livrarr_domain::identity::{IdentityMethod, IdentityState};
                            use livrarr_domain::seed::{
                                seed_author_monitor, SeedInput, SeedLanguage,
                            };
                            let candidate = seed_author_monitor(
                                SeedInput {
                                    title: work_title.clone(),
                                    author_name: cleaned_author_ref.clone(),
                                    language: SeedLanguage::resolve(
                                        author_ref.monitor_language.as_deref(),
                                        default_language_ref,
                                    ),
                                    author_ol_key: Some(seed_ol_key.clone()),
                                    year: Some(year),
                                    cover_url,
                                    detail_url: None,
                                    description: None,
                                    series_name: None,
                                    series_position: None,
                                },
                                IdentityState::Confirmed {
                                    anchors: livrarr_domain::identity::CapturedIdentity {
                                        ol_key: Some(stripped_ol_key.clone()),
                                        gr_key: None,
                                        hc_key: None,
                                        isbn_13: None,
                                        asin: None,
                                        title: work_title.clone(),
                                        author_name: cleaned_author_ref.clone(),
                                        language: None,
                                    },
                                    method: IdentityMethod::TitleAuthorSearch,
                                    score: None,
                                },
                            );
                            match self.work_service.add(author_ref.user_id, candidate).await {
                                Ok(work_result) => {
                                    let ws = self.work_service.clone();
                                    let (uid, wid) = (author_ref.user_id, work_result.work.id);
                                    tokio::spawn(async move {
                                        let _ = ws.converge_work(uid, wid, 3).await;
                                    });
                                    let notif_ok = self
                                        .db
                                        .create_notification(CreateNotificationDbRequest {
                                            user_id: author_ref.user_id,
                                            notification_type: NotificationType::WorkAutoAdded,
                                            ref_key: Some(stripped_ol_key.clone()),
                                            message: format!(
                                                "New work '{}' by {} auto-added to your library",
                                                work_title, author_ref.name
                                            ),
                                            data: serde_json::json!({
                                                "title": work_title,
                                                "author": author_ref.name,
                                                "year": year,
                                                "ol_key": stripped_ol_key,
                                            }),
                                        })
                                        .await
                                        .map_err(|e| {
                                            tracing::warn!("create_notification failed: {e}")
                                        })
                                        .is_ok();
                                    EntryOutcome {
                                        works_added: true,
                                        notifications_created: notif_ok,
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        author_id = author_ref.id,
                                        ol_key = %stripped_ol_key,
                                        error = %e,
                                        "author monitor: failed to auto-add work"
                                    );
                                    EntryOutcome {
                                        works_added: false,
                                        notifications_created: false,
                                    }
                                }
                            }
                        } else {
                            let notif_ok = self
                                .db
                                .create_notification(CreateNotificationDbRequest {
                                    user_id: author_ref.user_id,
                                    notification_type: NotificationType::NewWorkDetected,
                                    ref_key: Some(stripped_ol_key.clone()),
                                    message: format!(
                                        "New work '{}' by {} detected",
                                        work_title, author_ref.name
                                    ),
                                    data: serde_json::json!({
                                        "title": work_title,
                                        "author": author_ref.name,
                                        "year": year,
                                        "ol_key": stripped_ol_key,
                                    }),
                                })
                                .await
                                .map_err(|e| tracing::warn!("create_notification failed: {e}"))
                                .is_ok();
                            EntryOutcome {
                                works_added: false,
                                notifications_created: notif_ok,
                            }
                        }
                    },
                )
                .buffer_unordered(5)
                .collect()
                .await;

            for outcome in outcomes {
                if outcome.works_added {
                    report.works_added += 1;
                }
                if outcome.notifications_created {
                    report.notifications_created += 1;
                }
            }

            // Rate limit respect: 1s delay between authors (cancellation-aware).
            tokio::select! {
                _ = tokio::time::sleep(self.inter_author_delay) => {},
                _ = cancel.cancelled() => { return Ok(report); },
            }
            i += 1;
        }

        Ok(report)
    }
}

#[cfg(test)]
mod screen_tests {
    use super::*;

    /// Raw OL JSON in the sampled shape (ST-007), exercising the extended
    /// deserialization — not pre-parsed structs (AC-008).
    fn entry_json(title: &str, author_count: usize) -> OlWorkEntry {
        let authors: Vec<serde_json::Value> = (0..author_count)
            .map(|i| {
                serde_json::json!({
                    "author": { "key": format!("/authors/OL{i}A") },
                    "type": { "key": "/type/author_role" }
                })
            })
            .collect();
        let raw = serde_json::json!({
            "key": "/works/OL1W",
            "title": title,
            "authors": authors,
        });
        serde_json::from_value(raw).expect("entry deserializes")
    }

    #[test]
    fn sampled_junk_is_screened() {
        // Verbatim ST-007 junk exemplars, by class.
        let cases = [
            (entry_json("Blood Lite", 23), JunkClass::Anthology),
            (entry_json("Urban Enemies", 6), JunkClass::Anthology), // threshold boundary
            (
                entry_json("Jim Butcher Box Set", 1),
                JunkClass::BundleKeyword,
            ),
            (
                entry_json("Jim Butcher's the Dresden Files Omnibus Volume 2", 1),
                JunkClass::BundleKeyword,
            ),
            (
                entry_json(
                    "Jim Butcher The Dresden Files Series 5 Books Collection Set",
                    1,
                ),
                JunkClass::BundleKeyword,
            ),
            (
                entry_json("Jim Butcher - Dresden Files : Books 1-4", 1),
                JunkClass::BundleKeyword,
            ),
            (
                entry_json("Jim Butcher Set", 1),
                JunkClass::SelfTitledBundle,
            ),
            (
                entry_json(
                    "Ghost Story\n            \n                Dresden Files",
                    1,
                ),
                JunkClass::MalformedTitle,
            ),
            (
                entry_json("1984 SparkNotes Literature Guide", 1),
                JunkClass::SummaryKeyword,
            ),
        ];
        for (entry, expected) in cases {
            let got = screen_entry(&entry, "Jim Butcher");
            assert_eq!(
                got,
                Some(expected),
                "title {:?} should screen as {expected:?}",
                entry.title
            );
        }
    }

    #[test]
    fn sampled_real_titles_pass() {
        // False-positive guard (AC-009): real titles from the sampled feeds,
        // including the author-name-containment case and a co-authored work
        // at the threshold boundary (5 credited authors).
        let author = "Jane Austen";
        for (title, count) in [
            ("Persuasion by Jane Austen", 1),
            ("Mansfield Park (Jane Austen Novels Book 5)", 1),
            ("Pride and Prejudice", 1),
            ("Storm Front", 1),
            ("Dead Beat", 1),
            ("Working Together", 5),
        ] {
            let entry = entry_json(title, count);
            assert_eq!(
                screen_entry(&entry, author),
                None,
                "real title {title:?} must pass the screen"
            );
        }
    }

    #[test]
    fn title_less_entry_is_screened_not_unknown() {
        // AC-010: no work named "Unknown" can be created.
        let raw = serde_json::json!({ "key": "/works/OL2W" });
        let entry: OlWorkEntry = serde_json::from_value(raw).unwrap();
        assert_eq!(
            screen_entry(&entry, "Jim Butcher"),
            Some(JunkClass::MissingTitle)
        );
    }

    #[test]
    fn missing_authors_array_counts_as_one() {
        let raw = serde_json::json!({ "key": "/works/OL3W", "title": "Storm Front" });
        let entry: OlWorkEntry = serde_json::from_value(raw).unwrap();
        assert_eq!(entry.author_count(), 1);
        assert_eq!(screen_entry(&entry, "Jim Butcher"), None);
    }
}
