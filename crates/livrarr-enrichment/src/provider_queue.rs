//! `DefaultProviderQueue` — the centralized scatter-gather request queue (R-22).
//!
//! Responsibilities (covered by behavioral contract tests):
//!   - Parallel dispatch across applicable providers (`tokio::task::JoinSet`).
//!   - Panic isolation — a provider task panic becomes a per-provider
//!     `PermanentFailure { ProviderPanic }` outcome. Other providers complete normally.
//!   - Durable phase-1 outcome persistence in `provider_retry_state` ([I-11]).
//!   - Retry budget — `attempts == max_attempts - 1` plus a fresh `WillRetry`
//!     dispatch converts to `PermanentFailure { RetryBudgetExhausted }`.
//!   - Restart safety — providers with an existing phase-2 terminal retry-state
//!     row are skipped without being called.
//!   - Mode coercion — `Manual` and `HardRefresh` flip `WillRetry`
//!     to merge-eligible (`Conflict` always blocks).
//!   - Applicability — non-applicable providers are absent from outcomes entirely.
//!
//! Pacing, per-provider circuit breaking, and concurrency capping live at the
//! outbound queue (`livrarr_http::outbound_queue`), which paces and caps every
//! HTTP call regardless of caller. This queue does not pace or breaker-gate
//! dispatch itself — a call that needs to wait, waits at the outbound queue.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use livrarr_db::{DbError, ProviderCacheEntry, ProviderResponseCacheDb, ProviderRetryStateDb};
use livrarr_domain::identity_layer::{
    IdentityProvider, ProviderIdentityEvidence, ProviderIdentityEvidenceProvenance, RouteKey,
    RouteKind, WorkIdentityRepository, WorkRoute, WorkRouteState,
};
use livrarr_domain::services::{CallOperation, CallOutcomeClass, ProviderCallRecord};
use livrarr_domain::{
    AnchorQuery, Freshness, MetadataProvider, OutcomeClass, PermanentFailureReason, Work, WorkId,
};
use tokio::task::JoinSet;
use tracing::warn;

use crate::{
    EnrichmentContext, EnrichmentMode, NormalizedWorkDetail, PreviewFetchOutcome, ProviderOutcome,
    ProviderQueue, ProviderQueueConfig, ProviderQueueError, ScatterGatherResult, WillRetryReason,
};
use livrarr_external_data::provider_client::{
    IdentitySearchIdentifier, IdentitySearchProbe, ProviderClient,
};

#[derive(Default)]
struct SearchFallbackCapture {
    provider_identity: Vec<ProviderIdentityEvidence>,
    route_proposals: Vec<RouteKey>,
}

fn text_decisive_search_capture(
    provider: IdentityProvider,
    route: RouteKey,
) -> SearchFallbackCapture {
    SearchFallbackCapture {
        provider_identity: vec![ProviderIdentityEvidence {
            provider,
            route,
            work_core: None,
            provenance: ProviderIdentityEvidenceProvenance::TextDecisiveSearchFallback,
        }],
        route_proposals: Vec::new(),
    }
}

fn search_route_kind(provider: MetadataProvider) -> Option<(IdentityProvider, RouteKind)> {
    match provider {
        MetadataProvider::OpenLibrary => {
            Some((IdentityProvider::OpenLibrary, RouteKind::OpenLibraryWork))
        }
        MetadataProvider::Goodreads => {
            Some((IdentityProvider::Goodreads, RouteKind::GoodreadsWork))
        }
        MetadataProvider::Hardcover => {
            Some((IdentityProvider::Hardcover, RouteKind::HardcoverWork))
        }
        _ => None,
    }
}

fn edition_identifier_matches(route: &WorkRoute, identifier: &IdentitySearchIdentifier) -> bool {
    if route.state != WorkRouteState::Active || route.kind != identifier.kind {
        return false;
    }
    match identifier.kind {
        RouteKind::Isbn13Edition => {
            livrarr_domain::strip_isbn_punctuation(&route.provider_scoped_id)
                == livrarr_domain::strip_isbn_punctuation(&identifier.value)
        }
        RouteKind::AsinEdition | RouteKind::GoodreadsBookEdition => {
            route.provider_scoped_id.trim() == identifier.value.trim()
        }
        _ => false,
    }
}

async fn run_identity_search_fallback(
    provider: MetadataProvider,
    client: ProviderClient,
    seed_title: String,
    seed_author: String,
    language: Option<String>,
    active_routes: Vec<WorkRoute>,
    priority: livrarr_domain::RequestPriority,
) -> SearchFallbackCapture {
    let Ok(mut candidates) = client
        .search_identity_candidates(&seed_title, &seed_author, language.as_deref(), priority)
        .await
    else {
        return SearchFallbackCapture::default();
    };
    let decision_candidates: Vec<livrarr_domain::identity_matching::SearchFallbackCandidate<'_>> =
        candidates
            .iter()
            .map(
                |candidate| livrarr_domain::identity_matching::SearchFallbackCandidate {
                    title: &candidate.title,
                    author: &candidate.author,
                    provider_work_id: &candidate.work_id,
                },
            )
            .collect();
    // REQ-027 is Same-tier only. The authority still types an explicitly
    // grey-enabled pick as Propose for other consumers; this queue never
    // broadens its own bar.
    let decision = livrarr_domain::identity_matching::classify_search_fallback(
        &seed_title,
        &seed_author,
        &decision_candidates,
        false,
    );
    let Some(index) = decision.candidate_index() else {
        return SearchFallbackCapture::default();
    };
    let text_decisive = matches!(
        decision,
        livrarr_domain::identity_matching::SearchFallbackDecision::AutoLink { .. }
    );
    drop(decision_candidates);
    let candidate = &mut candidates[index];
    let Some((identity_provider, work_kind)) = search_route_kind(provider) else {
        return SearchFallbackCapture::default();
    };
    let work_route = RouteKey {
        provider: identity_provider.clone(),
        kind: work_kind,
        value: candidate.work_id.trim().to_string(),
    };

    let can_corroborate = active_routes.iter().any(|route| {
        route.state == WorkRouteState::Active
            && matches!(
                route.kind,
                RouteKind::AsinEdition | RouteKind::Isbn13Edition | RouteKind::GoodreadsBookEdition
            )
    });
    if !can_corroborate {
        return if text_decisive {
            text_decisive_search_capture(identity_provider, work_route)
        } else {
            SearchFallbackCapture {
                provider_identity: Vec::new(),
                route_proposals: vec![work_route],
            }
        };
    }

    let mut goodreads_book_id = None;
    if let Some(probe) = candidate.probe.as_ref() {
        if let IdentitySearchProbe::GoodreadsBook { book_id } = probe {
            goodreads_book_id = Some(book_id.clone());
            candidate
                .edition_identifiers
                .push(IdentitySearchIdentifier {
                    kind: RouteKind::GoodreadsBookEdition,
                    value: book_id.clone(),
                });
        }
        // A probe is solely an opportunity to upgrade the result to
        // corroborated outcome (a). Failure retains a text-decisive (b), but a
        // proposal-grade pick remains an honest miss: the failed probe cannot
        // manufacture confidence for a card.
        let Ok(probed) = client
            .probe_identity_candidate(probe, language.as_deref(), priority)
            .await
        else {
            return if text_decisive {
                text_decisive_search_capture(identity_provider, work_route)
            } else {
                SearchFallbackCapture::default()
            };
        };
        candidate.edition_identifiers.extend(probed);
    }

    let corroborating_kind = candidate.edition_identifiers.iter().find_map(|identifier| {
        active_routes
            .iter()
            .any(|route| edition_identifier_matches(route, identifier))
            .then(|| identifier.kind.clone())
    });
    let Some(corroborating_kind) = corroborating_kind else {
        return if text_decisive {
            text_decisive_search_capture(identity_provider, work_route)
        } else {
            SearchFallbackCapture {
                provider_identity: Vec::new(),
                route_proposals: vec![work_route],
            }
        };
    };
    let provenance = ProviderIdentityEvidenceProvenance::SearchFallback {
        corroborating_kind: corroborating_kind.clone(),
    };
    let mut provider_identity = vec![ProviderIdentityEvidence {
        provider: identity_provider.clone(),
        route: work_route,
        work_core: None,
        provenance: provenance.clone(),
    }];
    if let Some(book_id) = goodreads_book_id {
        provider_identity.push(ProviderIdentityEvidence {
            provider: IdentityProvider::Goodreads,
            route: RouteKey {
                provider: IdentityProvider::Goodreads,
                kind: RouteKind::GoodreadsBookEdition,
                value: book_id,
            },
            work_core: None,
            provenance,
        });
    }
    SearchFallbackCapture {
        provider_identity,
        route_proposals: Vec::new(),
    }
}

/// REQ-006 anchor derivation: the anchor query each provider's enrichment
/// fetch uses, from the work's stored anchors. Empty/whitespace values count
/// as absent. Hardcover prefers ISBN (the working by-key path — see the HcKey
/// gap note in provider_client.rs); OpenLibrary prefers its own key.
fn derive_anchor_query(provider: MetadataProvider, work: &Work) -> Option<AnchorQuery> {
    fn present(v: &Option<String>) -> Option<String> {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    }
    match provider {
        MetadataProvider::GoogleBooks => present(&work.isbn_13).map(AnchorQuery::Isbn13),
        MetadataProvider::Goodreads => present(&work.gr_key).map(AnchorQuery::GrKey),
        MetadataProvider::Hardcover => present(&work.isbn_13)
            .map(AnchorQuery::Isbn13)
            .or_else(|| present(&work.hc_key).map(AnchorQuery::HcKey)),
        MetadataProvider::OpenLibrary => present(&work.ol_key)
            .map(AnchorQuery::OlKey)
            .or_else(|| present(&work.isbn_13).map(AnchorQuery::Isbn13)),
        MetadataProvider::Audnexus | MetadataProvider::Audible => {
            present(&work.asin).map(AnchorQuery::Asin)
        }
        // Never scatter providers; no anchor surface exists for them.
        MetadataProvider::Llm | MetadataProvider::Readarr => None,
    }
}

/// F2 cutover anchor derivation: production enrichment reads only active
/// identity routes. The returned query preserves the legacy provider-specific
/// preference order while keeping the route table authoritative.
fn derive_route_anchor_query(
    provider: MetadataProvider,
    routes: &[WorkRoute],
) -> Option<AnchorQuery> {
    let find = |identity_provider: IdentityProvider, kind: RouteKind| {
        routes
            .iter()
            .find(|route| {
                route.state == WorkRouteState::Active
                    && route.provider == identity_provider
                    && route.kind == kind
                    && !route.provider_scoped_id.trim().is_empty()
            })
            .map(|route| route.provider_scoped_id.trim().to_string())
    };

    match provider {
        // A Goodreads BookEdition id remains edition-scoped evidence, but it
        // can fetch that Book page. The payload parser returns its distinct
        // Book -> Work legacy id as fresh route evidence; the book id itself
        // is never promoted to a Work route.
        MetadataProvider::Goodreads => find(IdentityProvider::Goodreads, RouteKind::GoodreadsWork)
            .or_else(|| find(IdentityProvider::Goodreads, RouteKind::GoodreadsBookEdition))
            .map(AnchorQuery::GrKey),
        MetadataProvider::OpenLibrary => {
            find(IdentityProvider::OpenLibrary, RouteKind::OpenLibraryWork)
                .map(AnchorQuery::OlKey)
                .or_else(|| {
                    find(IdentityProvider::IsbnRegistry, RouteKind::Isbn13Edition)
                        .map(AnchorQuery::Isbn13)
                })
        }
        MetadataProvider::Hardcover => {
            find(IdentityProvider::IsbnRegistry, RouteKind::Isbn13Edition)
                .map(AnchorQuery::Isbn13)
                .or_else(|| {
                    find(IdentityProvider::Hardcover, RouteKind::HardcoverWork)
                        .map(AnchorQuery::HcKey)
                })
        }
        MetadataProvider::GoogleBooks => {
            find(IdentityProvider::IsbnRegistry, RouteKind::Isbn13Edition).map(AnchorQuery::Isbn13)
        }
        MetadataProvider::Audnexus | MetadataProvider::Audible => {
            find(IdentityProvider::Amazon, RouteKind::AsinEdition).map(AnchorQuery::Asin)
        }
        MetadataProvider::Llm | MetadataProvider::Readarr => None,
    }
}

/// REQ-009 provider-response cache key vocabulary: the `anchor_type` string
/// paired with the anchor value at the (provider, anchor_type, anchor) cache
/// key. Pinned by the behavioral suite.
fn anchor_cache_key(anchor: &AnchorQuery) -> (&'static str, &str) {
    match anchor {
        AnchorQuery::Isbn13(v) => ("isbn13", v.as_str()),
        AnchorQuery::GrKey(v) => ("gr_key", v.as_str()),
        AnchorQuery::HcKey(v) => ("hc_key", v.as_str()),
        AnchorQuery::OlKey(v) => ("ol_key", v.as_str()),
        AnchorQuery::Asin(v) => ("asin", v.as_str()),
    }
}

/// Pipeline-level skip record (REQ-001): emitted by the queue when it decides
/// not to call a provider, since no client call happens that could record
/// itself.
fn record_queue_skip(
    sink: &Option<Arc<dyn livrarr_domain::services::ProviderCallSink>>,
    provider: MetadataProvider,
    work_id: WorkId,
    outcome: CallOutcomeClass,
    detail: Option<&str>,
) {
    if let Some(sink) = sink {
        sink.record(ProviderCallRecord {
            provider: provider.record_key().to_string(),
            operation: CallOperation::Enrich,
            work_id: Some(work_id),
            started_at: Utc::now(),
            duration_ms: 0,
            outcome,
            detail: detail.map(str::to_string),
        });
    }
}

/// Pluggable applicability check. The queue calls this once per (provider, work)
/// at dispatch time; non-applicable providers are absent from `ScatterGatherResult.outcomes`
/// and never invoked.
pub type ApplicabilityRule = Arc<dyn Fn(MetadataProvider, &Work) -> bool + Send + Sync>;

/// Per-provider configuration registered with the queue.
struct ProviderEntry {
    client: ProviderClient,
    config: ProviderQueueConfig,
}

/// Builder for `DefaultProviderQueue`. The behavioral test harness uses this to
/// register one stub client per scenario; production wiring uses the same builder
/// to register real-network clients (in a follow-on session).
pub struct DefaultProviderQueueBuilder {
    providers: HashMap<MetadataProvider, ProviderEntry>,
    applicability: Option<ApplicabilityRule>,
    call_sink: Option<Arc<dyn livrarr_domain::services::ProviderCallSink>>,
    cache_ttl: chrono::Duration,
    cache_max_rows: i64,
    identity_routes_authoritative: bool,
}

impl Default for DefaultProviderQueueBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultProviderQueueBuilder {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
            applicability: None,
            call_sink: None,
            cache_ttl: chrono::Duration::days(7),
            cache_max_rows: 100_000,
            identity_routes_authoritative: false,
        }
    }

    /// Configure the persistent provider-response cache (REQ-009): entries
    /// older than `ttl` are stale for `Freshness::PreferCache` dispatches;
    /// the store is evicted oldest-first down to `max_rows` after writes.
    pub fn with_provider_cache(mut self, ttl: chrono::Duration, max_rows: i64) -> Self {
        self.cache_ttl = ttl;
        self.cache_max_rows = max_rows;
        self
    }

    /// Inject the call-record sink (REQ-001): the queue records pipeline-level
    /// skips (no anchor, policy) through it — no client call happens for those.
    pub fn with_call_sink(
        mut self,
        sink: Arc<dyn livrarr_domain::services::ProviderCallSink>,
    ) -> Self {
        self.call_sink = Some(sink);
        self
    }

    /// Enable post-F2 production dispatch. Every enrichment pass reads the
    /// Work's captured identity and derives provider queries from active routes;
    /// legacy scalar anchors are not consulted in this mode.
    pub fn with_identity_route_dispatch(mut self) -> Self {
        self.identity_routes_authoritative = true;
        self
    }

    pub fn add_provider(
        mut self,
        provider: MetadataProvider,
        client: ProviderClient,
        config: ProviderQueueConfig,
    ) -> Self {
        self.providers
            .insert(provider, ProviderEntry { client, config });
        self
    }

    pub fn with_applicability_rule(mut self, rule: ApplicabilityRule) -> Self {
        self.applicability = Some(rule);
        self
    }

    pub fn build<DB>(self, retry_db: Arc<DB>) -> DefaultProviderQueue<DB>
    where
        DB: ProviderRetryStateDb + ProviderResponseCacheDb + Send + Sync + 'static,
    {
        let applicability = self
            .applicability
            .unwrap_or_else(|| Arc::new(|_provider, _work| true));
        DefaultProviderQueue {
            providers: Arc::new(self.providers),
            applicability,
            retry_db,
            call_sink: self.call_sink,
            cache_ttl: self.cache_ttl,
            cache_max_rows: self.cache_max_rows,
            identity_routes_authoritative: self.identity_routes_authoritative,
        }
    }
}

/// Centralized scatter-gather provider request queue. See module-level docs.
pub struct DefaultProviderQueue<DB>
where
    DB: ProviderRetryStateDb + ProviderResponseCacheDb + Send + Sync + 'static,
{
    providers: Arc<HashMap<MetadataProvider, ProviderEntry>>,
    applicability: ApplicabilityRule,
    retry_db: Arc<DB>,
    #[allow(dead_code)] // read at green: REQ-006 skip records via REQ-001 sink
    call_sink: Option<Arc<dyn livrarr_domain::services::ProviderCallSink>>,
    /// REQ-009: entries older than this are stale for `Freshness::PreferCache`.
    cache_ttl: chrono::Duration,
    /// REQ-009: the store is evicted oldest-first down to this cap after a
    /// batch of real-fetch cache writes.
    cache_max_rows: i64,
    /// True only in the post-activation production composition and faithful
    /// production-router harnesses. Legacy queue unit tests keep their scalar
    /// fixtures until that older surface is retired.
    identity_routes_authoritative: bool,
}

/// Outcome of one provider's phase-1 dispatch, before terminal-budget conversion
/// and durable persistence.
enum DispatchedOutcome {
    /// Provider client returned an outcome normally.
    Returned(ProviderOutcome<NormalizedWorkDetail>),
    /// Provider client task panicked.
    Panicked,
    /// REQ-009: served from the persistent provider-response cache — no
    /// client fetch happened for this provider this pass.
    CachedSuccess(Box<NormalizedWorkDetail>),
}

/// Read existing terminal state for restart safety. None = no row, or row is non-terminal.
async fn existing_terminal_outcome<DB: ProviderRetryStateDb + Send + Sync>(
    db: &DB,
    user_id: livrarr_domain::UserId,
    work_id: WorkId,
    provider: MetadataProvider,
) -> Result<Option<OutcomeClass>, DbError> {
    let state = db.get_retry_state(user_id, work_id, provider).await?;
    Ok(state
        .and_then(|s| s.last_outcome)
        .filter(|o| o.is_phase2_terminal()))
}

impl<DB> ProviderQueue for DefaultProviderQueue<DB>
where
    DB: ProviderRetryStateDb
        + ProviderResponseCacheDb
        + WorkIdentityRepository
        + Send
        + Sync
        + 'static,
{
    async fn preview_fetch(
        &self,
        provider: MetadataProvider,
        query: AnchorQuery,
        language: Option<String>,
        priority: livrarr_domain::RequestPriority,
    ) -> PreviewFetchOutcome {
        // Direct client fetch over the registry — no cache, no retry-state,
        // no budget (identity-edit r4 §Preview seam). An unregistered
        // provider is truthfully NotConfigured.
        let Some(entry) = self.providers.get(&provider) else {
            return PreviewFetchOutcome::NotConfigured;
        };
        match entry
            .client
            .fetch_by_anchor(query, language.as_deref(), priority)
            .await
        {
            ProviderOutcome::Success(detail) => PreviewFetchOutcome::Resolved(detail),
            ProviderOutcome::NotFound => PreviewFetchOutcome::NotFound,
            ProviderOutcome::NotConfigured => PreviewFetchOutcome::NotConfigured,
            ProviderOutcome::WillRetry { .. }
            | ProviderOutcome::PermanentFailure { .. }
            | ProviderOutcome::Conflict { .. } => PreviewFetchOutcome::Unavailable,
        }
    }

    async fn dispatch_enrichment(
        &self,
        work: &Work,
        context: EnrichmentContext,
    ) -> Result<ScatterGatherResult, ProviderQueueError> {
        let mut outcomes: HashMap<MetadataProvider, ProviderOutcome<NormalizedWorkDetail>> =
            HashMap::new();
        let captured_identity = if self.identity_routes_authoritative {
            Some(
                self.retry_db
                    .read_captured_identity(work.user_id, work.id)
                    .await
                    .map_err(|error| ProviderQueueError::IdentityRouteRead(error.to_string()))?,
            )
        } else {
            None
        };
        let route_plan = captured_identity.clone().map(|identity| {
            use crate::identity_layer::EnrichmentService as _;
            crate::identity_layer::RouteDrivenEnrichmentService::from_arc(
                self.retry_db.clone(),
                identity.clone(),
                context.priority,
            )
            .plan_from_routes(identity)
        });
        let search_seed = if let Some(identity) = captured_identity.as_ref().filter(|identity| {
            !identity.active_routes.iter().any(|route| {
                route.state == WorkRouteState::Active
                    && matches!(
                        route.kind,
                        RouteKind::OpenLibraryWork
                            | RouteKind::GoodreadsWork
                            | RouteKind::HardcoverWork
                    )
            })
        }) {
            let author_name = self
                .retry_db
                .read_primary_author_names(work.user_id, identity.primary_author_id)
                .await
                .map_err(|error| ProviderQueueError::IdentityRouteRead(error.to_string()))?
                .into_iter()
                .next();
            author_name.map(|author| (identity.identity_title.main.clone(), author))
        } else {
            None
        };

        // Partition providers into: skip (not applicable / anchor-less /
        // restart-resumed) and dispatch. The dispatch tuple carries the
        // derived anchor query (REQ-006).
        struct DispatchEntry {
            provider: MetadataProvider,
            client: ProviderClient,
            config: ProviderQueueConfig,
            anchor: AnchorQuery,
        }
        struct SearchDispatchEntry {
            provider: MetadataProvider,
            client: ProviderClient,
        }
        let mut to_dispatch: Vec<DispatchEntry> = Vec::new();
        let mut to_search: Vec<SearchDispatchEntry> = Vec::new();
        // REQ-009: pre-populated with cache hits before the scatter phase —
        // a hit occupies its provider's slot here so the spawn loop below
        // never fetches it, yet it still reaches the same finalization
        // (budget rules + durable persistence) as a real fetch.
        let mut dispatched: HashMap<MetadataProvider, DispatchedOutcome> = HashMap::new();

        for (provider, entry) in self.providers.iter() {
            let provider = *provider;

            if !(self.applicability)(provider, work) {
                // Policy skip (e.g. the language applicability rule): recorded
                // by this layer since no client call happens (REQ-001).
                record_queue_skip(
                    &self.call_sink,
                    provider,
                    work.id,
                    CallOutcomeClass::SkippedPolicy,
                    Some("not_applicable"),
                );
                continue;
            }

            // REQ-006: enrichment fetches only by stored anchor. No anchor for
            // this provider → no fetch, a SkippedNoAnchor record, a NotFound
            // outcome (anchor acquisition is the identity track's job).
            let anchor = route_plan.as_ref().map_or_else(
                || derive_anchor_query(provider, work),
                |plan| derive_route_anchor_query(provider, &plan.usable_routes),
            );
            let search_eligible = search_seed.is_some()
                && search_route_kind(provider).is_some()
                && entry.client.identity_search_available();
            let Some(anchor) = anchor else {
                if search_eligible {
                    to_search.push(SearchDispatchEntry {
                        provider,
                        client: entry.client.clone(),
                    });
                    // Search is a route-finding side leg, not an enrichment
                    // payload. Keep the provider outcome terminal while its
                    // typed evidence travels on the handoff-only channel.
                    outcomes.insert(provider, ProviderOutcome::NotFound);
                    continue;
                }
                record_queue_skip(
                    &self.call_sink,
                    provider,
                    work.id,
                    CallOutcomeClass::SkippedNoAnchor,
                    None,
                );
                outcomes.insert(provider, ProviderOutcome::NotFound);
                continue;
            };

            // Spec v10 dead-anchor predicate: an applicable OL/GR/HC provider
            // with a derivable anchor joins the search leg only when no active
            // work-level route exists, provider search is available, and the
            // anchor's current standing is terminal `not_found`. `will_retry`
            // is non-terminal and therefore remains anchor-first. Explicit
            // retry-state reset after a re-key/generation change removes this
            // standing and likewise restores anchor-first dispatch.
            let terminal =
                existing_terminal_outcome(self.retry_db.as_ref(), work.user_id, work.id, provider)
                    .await?;
            if terminal == Some(OutcomeClass::NotFound) && search_eligible {
                to_search.push(SearchDispatchEntry {
                    provider,
                    client: entry.client.clone(),
                });
                outcomes.insert(provider, ProviderOutcome::NotFound);
                continue;
            }

            // Restart safety for every other terminal standing.
            if terminal.is_some() {
                continue;
            }

            // REQ-009: a fresh cache hit satisfies this provider without a
            // client fetch. Bypass, a stale row, a missing row, or damaged
            // payload all fall through to the normal fetch below.
            if context.freshness == Freshness::PreferCache {
                if let Some(detail) = self.cached_success(provider, &anchor).await {
                    dispatched.insert(provider, DispatchedOutcome::CachedSuccess(Box::new(detail)));
                }
            }

            to_dispatch.push(DispatchEntry {
                provider,
                client: entry.client.clone(),
                config: entry.config.clone(),
                anchor,
            });
        }

        let priority = context.priority;
        let language = work.language.clone();
        let active_routes = captured_identity
            .as_ref()
            .map(|identity| identity.active_routes.clone())
            .unwrap_or_default();
        let mut provider_chase_attempted = false;
        let mut search_provider_identity = Vec::new();
        let mut search_route_proposals = Vec::new();
        if let Some((seed_title, seed_author)) = search_seed {
            let mut search_set = JoinSet::new();
            for entry in to_search {
                provider_chase_attempted = true;
                search_set.spawn(run_identity_search_fallback(
                    entry.provider,
                    entry.client,
                    seed_title.clone(),
                    seed_author.clone(),
                    language.clone(),
                    active_routes.clone(),
                    priority,
                ));
            }
            while let Some(joined) = search_set.join_next().await {
                match joined {
                    Ok(capture) => {
                        search_provider_identity.extend(capture.provider_identity);
                        search_route_proposals.extend(capture.route_proposals);
                    }
                    Err(error) => warn!("identity search fallback task failed: {error}"),
                }
            }
        }

        // Phase 1: scatter — spawn each provider call not already served from
        // cache. Panic isolation via JoinSet. Pacing, concurrency capping, and
        // circuit breaking happen at the outbound queue (every HTTP call
        // routes through it); this layer only dispatches.
        let mut set: JoinSet<(MetadataProvider, DispatchedOutcome)> = JoinSet::new();
        for d in &to_dispatch {
            if dispatched.contains_key(&d.provider) {
                continue; // REQ-009 cache hit — no client fetch, no call record.
            }
            let provider = d.provider;
            let client = d.client.clone();
            let anchor = d.anchor.clone();
            let language = language.clone();
            provider_chase_attempted = true;
            set.spawn(async move {
                let outcome = client
                    .fetch_by_anchor(anchor, language.as_deref(), priority)
                    .await;
                (provider, DispatchedOutcome::Returned(outcome))
            });
        }

        // Phase 1: gather — collect outcomes, mapping panics to ProviderPanic.
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok((provider, outcome)) => {
                    dispatched.insert(provider, outcome);
                }
                Err(join_err) if join_err.is_panic() => {
                    // Recover the provider id by id() — we can't, JoinError doesn't
                    // expose the provider tag. Use the task id we wrapped earlier.
                    // Workaround: panicked tasks need a separate path. Spawn with
                    // metadata wasn't possible above, so we rebuild using JoinHandle
                    // tracking. Since we can't recover the provider here, we mark
                    // any missing providers as panicked at the end of the gather phase.
                    warn!("provider task panicked (id mapping resolved post-gather)");
                }
                Err(join_err) => {
                    warn!("provider task join error (non-panic): {join_err}");
                }
            }
        }

        // Reconcile: any to_dispatch provider that didn't show up in `dispatched`
        // panicked or was canceled — treat as ProviderPanic per IR.
        for d in &to_dispatch {
            dispatched
                .entry(d.provider)
                .or_insert(DispatchedOutcome::Panicked);
        }

        // For each dispatched outcome, apply budget rules and persist phase-1
        // state durably ([I-11]). Then build the in-memory result outcome.
        let mut wrote_to_cache = false;
        for d in &to_dispatch {
            let provider = d.provider;
            let raw = dispatched
                .remove(&provider)
                .expect("dispatched entry must exist after reconciliation");

            let (final_outcome, cache_served) = match raw {
                DispatchedOutcome::Panicked => (
                    ProviderOutcome::PermanentFailure {
                        reason: PermanentFailureReason::ProviderPanic,
                    },
                    false,
                ),
                DispatchedOutcome::Returned(outcome) => (
                    self.apply_budget_rules(work, provider, &d.config, outcome)
                        .await?,
                    false,
                ),
                DispatchedOutcome::CachedSuccess(detail) => (
                    self.apply_budget_rules(
                        work,
                        provider,
                        &d.config,
                        ProviderOutcome::Success(detail),
                    )
                    .await?,
                    true,
                ),
            };

            // Durable persistence.
            self.persist_phase1_outcome(work, provider, &final_outcome)
                .await?;

            // REQ-009 / D-003: only a REAL fetch success is cache-worthy — a
            // cache-served success must never re-stamp its own row, and
            // non-Success outcomes are never cached.
            if !cache_served {
                if let ProviderOutcome::Success(payload) = &final_outcome {
                    self.cache_write(provider, &d.anchor, payload.as_ref())
                        .await;
                    wrote_to_cache = true;
                }
            }

            outcomes.insert(provider, final_outcome);
        }

        if wrote_to_cache {
            if let Err(e) = self
                .retry_db
                .evict_provider_cache_to_cap(self.cache_max_rows)
                .await
            {
                warn!("provider-response cache eviction failed: {e}");
            }
        }

        let conflict_present = outcomes
            .values()
            .any(|o| matches!(o, ProviderOutcome::Conflict { .. }));
        let merge_eligible = !conflict_present;
        let deferred = if conflict_present {
            false
        } else {
            match context.mode {
                EnrichmentMode::Background => outcomes.values().any(|o| !o.can_merge()),
                EnrichmentMode::Manual | EnrichmentMode::HardRefresh => false,
            }
        };

        Ok(ScatterGatherResult {
            work_id: work.id,
            outcomes,
            merge_eligible,
            deferred,
            provider_chase_attempted,
            search_provider_identity,
            search_route_proposals,
        })
    }
}

impl<DB> DefaultProviderQueue<DB>
where
    DB: ProviderRetryStateDb + ProviderResponseCacheDb + Send + Sync + 'static,
{
    /// REQ-009: consult the persistent provider-response cache for a fresh
    /// (age < `cache_ttl`) success payload. `None` on any miss — no row, a
    /// stale row, a DB read error, or a payload that fails to deserialize.
    /// Cache damage is never the dispatch's problem: warn-log and let the
    /// caller fall through to a normal fetch.
    async fn cached_success(
        &self,
        provider: MetadataProvider,
        anchor: &AnchorQuery,
    ) -> Option<NormalizedWorkDetail> {
        let (anchor_type, anchor_value) = anchor_cache_key(anchor);
        let entry = match self
            .retry_db
            .get_provider_cache_entry(provider, anchor_type, anchor_value)
            .await
        {
            Ok(Some(entry)) => entry,
            Ok(None) => return None,
            Err(e) => {
                warn!("provider-response cache read failed for {provider:?}: {e}");
                return None;
            }
        };
        if Utc::now() - entry.fetched_at >= self.cache_ttl {
            return None;
        }
        match serde_json::from_str::<NormalizedWorkDetail>(&entry.payload_json) {
            Ok(detail) => Some(detail),
            Err(e) => {
                warn!(
                    "provider-response cache payload for {provider:?} failed to deserialize: {e}"
                );
                None
            }
        }
    }

    /// REQ-009: write a real fetch's success payload into the persistent
    /// provider-response cache, stamping `fetched_at` at write time. Never
    /// fails the dispatch — a write error is logged and the pass proceeds
    /// with the outcome it already has.
    async fn cache_write(
        &self,
        provider: MetadataProvider,
        anchor: &AnchorQuery,
        payload: &NormalizedWorkDetail,
    ) {
        let (anchor_type, anchor_value) = anchor_cache_key(anchor);
        let payload_json = match serde_json::to_string(payload) {
            Ok(json) => json,
            Err(e) => {
                warn!("provider-response cache payload for {provider:?} failed to serialize: {e}");
                return;
            }
        };
        let entry = ProviderCacheEntry {
            provider,
            anchor_type: anchor_type.to_string(),
            anchor: anchor_value.to_string(),
            payload_json,
            fetched_at: Utc::now(),
        };
        if let Err(e) = self.retry_db.upsert_provider_cache_entry(entry).await {
            warn!("provider-response cache write failed for {provider:?}: {e}");
        }
    }

    /// Apply retry budget conversion. Reads the existing retry-state row
    /// to know prior `attempts`.
    async fn apply_budget_rules(
        &self,
        work: &Work,
        provider: MetadataProvider,
        config: &ProviderQueueConfig,
        outcome: ProviderOutcome<NormalizedWorkDetail>,
    ) -> Result<ProviderOutcome<NormalizedWorkDetail>, ProviderQueueError> {
        match outcome {
            ProviderOutcome::WillRetry {
                reason,
                next_attempt_at,
            } => {
                // R-11/D3: a breaker-open OR admission-queue-full pass is a
                // PAUSE (the provider is temporarily down, or the local
                // outbound queue is momentarily oversubscribed) — never a
                // step toward a retry-budget dead-end. Neither may consume
                // the attempt nor the suppression budget. Return unchanged.
                if matches!(
                    reason,
                    WillRetryReason::CircuitOpen | WillRetryReason::QueueFull
                ) {
                    return Ok(ProviderOutcome::WillRetry {
                        reason,
                        next_attempt_at,
                    });
                }
                let prior = self
                    .retry_db
                    .get_retry_state(work.user_id, work.id, provider)
                    .await?;
                let prior_attempts = prior.as_ref().map(|s| s.attempts).unwrap_or(0);
                if prior_attempts.saturating_add(1) >= config.max_attempts {
                    Ok(ProviderOutcome::PermanentFailure {
                        reason: PermanentFailureReason::RetryBudgetExhausted,
                    })
                } else {
                    Ok(ProviderOutcome::WillRetry {
                        reason,
                        next_attempt_at,
                    })
                }
            }
            other => Ok(other),
        }
    }

    /// Persist the per-provider phase-1 outcome to `provider_retry_state` ([I-11]).
    /// Success outcomes carry `normalized_payload_json`; non-Success terminal
    /// outcomes clear it.
    async fn persist_phase1_outcome(
        &self,
        work: &Work,
        provider: MetadataProvider,
        outcome: &ProviderOutcome<NormalizedWorkDetail>,
    ) -> Result<(), ProviderQueueError> {
        match outcome {
            ProviderOutcome::Success(payload) => {
                let json = serde_json::to_string(payload.as_ref())
                    .expect("NormalizedWorkDetail is always JSON-serializable");
                self.retry_db
                    .record_terminal_outcome(
                        work.user_id,
                        work.id,
                        provider,
                        OutcomeClass::Success,
                        Some(json),
                    )
                    .await?;
            }
            ProviderOutcome::NotFound => {
                self.retry_db
                    .record_terminal_outcome(
                        work.user_id,
                        work.id,
                        provider,
                        OutcomeClass::NotFound,
                        None,
                    )
                    .await?;
            }
            ProviderOutcome::NotConfigured => {
                self.retry_db
                    .record_terminal_outcome(
                        work.user_id,
                        work.id,
                        provider,
                        OutcomeClass::NotConfigured,
                        None,
                    )
                    .await?;
            }
            ProviderOutcome::PermanentFailure { .. } => {
                self.retry_db
                    .record_terminal_outcome(
                        work.user_id,
                        work.id,
                        provider,
                        OutcomeClass::PermanentFailure,
                        None,
                    )
                    .await?;
            }
            ProviderOutcome::Conflict { .. } => {
                self.retry_db
                    .record_terminal_outcome(
                        work.user_id,
                        work.id,
                        provider,
                        OutcomeClass::Conflict,
                        None,
                    )
                    .await?;
            }
            ProviderOutcome::WillRetry {
                reason,
                next_attempt_at,
            } => {
                // R-11/D3: a breaker-open OR admission-queue-full pass
                // persists via `record_will_retry_paused` (same row shape,
                // `attempts` NOT incremented) — a paused provider must not
                // spend retry budget while its breaker is open OR the local
                // outbound queue is momentarily full.
                if matches!(
                    *reason,
                    WillRetryReason::CircuitOpen | WillRetryReason::QueueFull
                ) {
                    self.retry_db
                        .record_will_retry_paused(work.user_id, work.id, provider, *next_attempt_at)
                        .await?;
                } else {
                    self.retry_db
                        .record_will_retry(work.user_id, work.id, provider, *next_attempt_at)
                        .await?;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod identity_route_dispatch_tests {
    use std::sync::Arc;

    use chrono::Utc;
    use livrarr_db::{AuthorDb, CreateAuthorDbRequest, CreateUserDbRequest, UserDb, WorkDb};
    use livrarr_domain::identity_layer::{
        EvidenceProvenance, IdentityProvider, IdentityTitleTuple, RouteKind, RouteOwner,
        RouteProvenance, SettlementCommit, WorkContributor, WorkIdentityRepository, WorkRoute,
        WorkRouteState,
    };
    use livrarr_domain::{Freshness, MetadataProvider, RequestPriority, UserRole};
    use livrarr_external_data::{ProviderClient, ProviderOutcome, StubProviderClient};

    use crate::provider_queue::DefaultProviderQueueBuilder;
    use crate::{EnrichmentContext, EnrichmentMode, ProviderQueue, ProviderQueueConfig};

    // Bug reproduction: identity-layer-rewrite — post-F2 Works carry provider
    // routes only in identity_routes; live dispatch must not require works.gr_key.
    #[tokio::test]
    async fn post_f2_goodreads_route_dispatches_without_legacy_scalar() {
        let db = livrarr_db::create_test_db().await;
        let user_id = db
            .create_user(CreateUserDbRequest {
                username: "f2_route_dispatch_user".to_string(),
                password_hash: "hash".to_string(),
                role: UserRole::Admin,
                api_key_hash: "apikey".to_string(),
            })
            .await
            .expect("create route-dispatch user")
            .id;
        let (author, _) = db
            .create_author(CreateAuthorDbRequest {
                user_id,
                name: "Route Dispatch Author".to_string(),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                hc_key: None,
                import_id: None,
            })
            .await
            .expect("create route-dispatch author");
        let settled = WorkIdentityRepository::commit_settlement(
            &db,
            SettlementCommit {
                user_id,
                existing_work_id: None,
                add_source: None,
                identity_title: IdentityTitleTuple {
                    main: "Route Dispatch Book".to_string(),
                    subtitle: None,
                    volume: None,
                    normalized_main: "route dispatch book".to_string(),
                    normalized_subtitle: String::new(),
                    normalized_volume: String::new(),
                    provenance: EvidenceProvenance::User,
                },
                text_distinction: None,
                contributors: vec![WorkContributor {
                    user_id,
                    work_id: 0,
                    author_id: author.id,
                    ordinal: 0,
                    roles: Vec::new(),
                }],
                routes: vec![WorkRoute {
                    id: 0,
                    user_id,
                    owner: RouteOwner::Work(0),
                    resolved_work_id: 0,
                    provider: IdentityProvider::Goodreads,
                    kind: RouteKind::GoodreadsWork,
                    provider_scoped_id: "12345".to_string(),
                    state: WorkRouteState::Active,
                    provenance: RouteProvenance::UserChoice,
                    user_confirmed: true,
                    observed_at: Utc::now(),
                }],
                absorbed_work_ids: Vec::new(),
                expected_generation: 0,
                review_cards: Vec::new(),
            },
        )
        .await
        .expect("settle route-dispatch work");
        let work = db
            .get_work(user_id, settled.identity.own_work_id)
            .await
            .expect("read settled work");
        assert_eq!(work.gr_key, None, "fixture must have no legacy GR scalar");

        let stub = StubProviderClient::new(MetadataProvider::Goodreads, ProviderOutcome::NotFound);
        let queue = DefaultProviderQueueBuilder::new()
            .with_identity_route_dispatch()
            .add_provider(
                MetadataProvider::Goodreads,
                ProviderClient::Stub(stub.clone()),
                ProviderQueueConfig {
                    provider: MetadataProvider::Goodreads,
                    max_attempts: 1,
                },
            )
            .build(Arc::new(db));
        let result = queue
            .dispatch_enrichment(
                &work,
                EnrichmentContext {
                    priority: RequestPriority::Normal,
                    mode: EnrichmentMode::Manual,
                    freshness: Freshness::Bypass,
                },
            )
            .await
            .expect("dispatch from captured route");

        assert_eq!(stub.call_count(), 1, "captured GR route must dispatch once");
        assert!(matches!(
            result.outcomes.get(&MetadataProvider::Goodreads),
            Some(ProviderOutcome::NotFound)
        ));
    }
}

#[cfg(test)]
mod circuit_open_budget_tests {
    //! R-11: a breaker-open `WillRetry { CircuitOpen }` must never convert to
    //! `PermanentFailure` at the retry-attempt-budget boundary, and its
    //! persistence must never bump `attempts` (see the `record_will_retry_paused`
    //! db-level tests in `livrarr-db`). This is the one spot budget conversion
    //! happens (`apply_budget_rules`), driven end-to-end through
    //! `dispatch_enrichment` with a scripted `StubProviderClient`.

    use std::sync::Arc;

    use livrarr_db::{
        CreateUserDbRequest, CreateWorkDbRequest, ProviderRetryStateDb, UserDb, WorkDbCreate,
    };
    use livrarr_domain::{Freshness, MetadataProvider, RequestPriority, UserRole, WillRetryReason};
    use livrarr_external_data::{ProviderClient, ProviderOutcome, StubProviderClient};

    use crate::provider_queue::DefaultProviderQueueBuilder;
    use crate::{EnrichmentContext, EnrichmentMode, ProviderQueue, ProviderQueueConfig};

    fn config(max_attempts: u32) -> ProviderQueueConfig {
        ProviderQueueConfig {
            provider: MetadataProvider::OpenLibrary,
            max_attempts,
        }
    }

    async fn seed_db_and_work() -> (livrarr_db::sqlite::SqliteDb, livrarr_domain::Work) {
        let db = livrarr_db::create_test_db().await;
        let user_id = db
            .create_user(CreateUserDbRequest {
                username: "circuit_open_budget_user".to_string(),
                password_hash: "hash".to_string(),
                role: UserRole::Admin,
                api_key_hash: "apikey".to_string(),
            })
            .await
            .unwrap()
            .id;
        let (work, _) = db
            .create_work(CreateWorkDbRequest {
                user_id,
                title: "Budget Book".to_string(),
                author_name: "Budget Author".to_string(),
                // OpenLibrary's REQ-006 anchor gate requires ol_key or isbn_13
                // before the queue will dispatch to the client at all.
                ol_key: Some("OL1W".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        (db, work)
    }

    /// Prior REAL retries (non-CircuitOpen) parked one short of the
    /// `max_attempts` boundary — the next WillRetry{ServerError} pass would
    /// normally convert to PermanentFailure{RetryBudgetExhausted}. A
    /// WillRetry{CircuitOpen} pass at the exact same boundary must NOT.
    #[tokio::test]
    async fn will_retry_circuit_open_survives_the_max_attempts_boundary() {
        let (db, work) = seed_db_and_work().await;
        let max_attempts = 3;
        for _ in 0..(max_attempts - 1) {
            db.record_will_retry(
                work.user_id,
                work.id,
                MetadataProvider::OpenLibrary,
                chrono::Utc::now() + chrono::Duration::seconds(60),
            )
            .await
            .unwrap();
        }
        let prior = db
            .get_retry_state(work.user_id, work.id, MetadataProvider::OpenLibrary)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(prior.attempts, max_attempts - 1);

        let client = ProviderClient::Stub(StubProviderClient::new(
            MetadataProvider::OpenLibrary,
            ProviderOutcome::WillRetry {
                reason: WillRetryReason::CircuitOpen,
                next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(60),
            },
        ));
        let db = Arc::new(db);
        let queue = DefaultProviderQueueBuilder::new()
            .add_provider(MetadataProvider::OpenLibrary, client, config(max_attempts))
            .build(db.clone());

        let ctx = EnrichmentContext {
            priority: RequestPriority::Normal,
            mode: EnrichmentMode::Background,
            freshness: Freshness::PreferCache,
        };
        let result = queue.dispatch_enrichment(&work, ctx).await.unwrap();
        let outcome = result.outcomes.get(&MetadataProvider::OpenLibrary).unwrap();

        assert!(
            matches!(
                outcome,
                ProviderOutcome::WillRetry {
                    reason: WillRetryReason::CircuitOpen,
                    ..
                }
            ),
            "a breaker-open pass at the max_attempts boundary must stay WillRetry{{CircuitOpen}}, \
             not convert to PermanentFailure — got {outcome:?}"
        );

        // record_will_retry_paused must have been used, not record_will_retry:
        // the prior attempts count is untouched by the CircuitOpen pass.
        let after = db
            .get_retry_state(work.user_id, work.id, MetadataProvider::OpenLibrary)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after.attempts,
            max_attempts - 1,
            "a breaker-open pass must not increment attempts"
        );
    }

    /// D3: `WillRetry{QueueFull}` (the outbound queue's admission cap
    /// rejected the request — no HTTP attempted) must survive the
    /// `max_attempts` boundary exactly like `WillRetry{CircuitOpen}` does
    /// above — same budget-exempt class, same non-incrementing persistence
    /// path (`record_will_retry_paused`).
    #[tokio::test]
    async fn will_retry_queue_full_survives_the_max_attempts_boundary() {
        let (db, work) = seed_db_and_work().await;
        let max_attempts = 3;
        for _ in 0..(max_attempts - 1) {
            db.record_will_retry(
                work.user_id,
                work.id,
                MetadataProvider::OpenLibrary,
                chrono::Utc::now() + chrono::Duration::seconds(60),
            )
            .await
            .unwrap();
        }
        let prior = db
            .get_retry_state(work.user_id, work.id, MetadataProvider::OpenLibrary)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(prior.attempts, max_attempts - 1);

        let client = ProviderClient::Stub(StubProviderClient::new(
            MetadataProvider::OpenLibrary,
            ProviderOutcome::WillRetry {
                reason: WillRetryReason::QueueFull,
                next_attempt_at: chrono::Utc::now() + chrono::Duration::seconds(60),
            },
        ));
        let db = Arc::new(db);
        let queue = DefaultProviderQueueBuilder::new()
            .add_provider(MetadataProvider::OpenLibrary, client, config(max_attempts))
            .build(db.clone());

        let ctx = EnrichmentContext {
            priority: RequestPriority::Normal,
            mode: EnrichmentMode::Background,
            freshness: Freshness::PreferCache,
        };
        let result = queue.dispatch_enrichment(&work, ctx).await.unwrap();
        let outcome = result.outcomes.get(&MetadataProvider::OpenLibrary).unwrap();

        assert!(
            matches!(
                outcome,
                ProviderOutcome::WillRetry {
                    reason: WillRetryReason::QueueFull,
                    ..
                }
            ),
            "a queue-full pass at the max_attempts boundary must stay WillRetry{{QueueFull}}, \
             not convert to PermanentFailure — got {outcome:?}"
        );

        // record_will_retry_paused must have been used, not record_will_retry:
        // the prior attempts count is untouched by the QueueFull pass.
        let after = db
            .get_retry_state(work.user_id, work.id, MetadataProvider::OpenLibrary)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after.attempts,
            max_attempts - 1,
            "a queue-full pass must not increment attempts"
        );
    }
}
