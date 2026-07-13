use futures::stream::{self, StreamExt};
use livrarr_domain::seed::{lookup_term_to_seed, seed_carries_identifier};
use livrarr_domain::services::*;
use livrarr_domain::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct StubNoLlm;

impl LlmCaller for StubNoLlm {
    async fn call(&self, _req: LlmCallRequest) -> Result<LlmCallResponse, LlmError> {
        Err(LlmError::NotConfigured)
    }
}

pub(crate) struct CachedLookup {
    filtered: Vec<LookupResult>,
    raw: Vec<LookupResult>,
    raw_available: bool,
    created_at: Instant,
}

pub(crate) struct DiscoveryCtx<'a, C, H, L> {
    pub(crate) config: &'a C,
    pub(crate) http: &'a H,
    pub(crate) llm: &'a L,
    pub(crate) lookup_cache: &'a Arc<Mutex<HashMap<(String, String), CachedLookup>>>,
    pub(crate) resolver:
        &'a Option<Arc<crate::english_identity_resolver::LiveEnglishIdentityResolver>>,
}

// Every field is a reference, which is always `Copy` regardless of whether
// `C`/`H`/`L` themselves are — but `#[derive(Copy)]` on a generic struct
// naively adds `C: Copy, H: Copy, L: Copy` bounds (it doesn't look inside
// field types), which would wrongly require the real (non-`Copy`) service
// implementations to be `Copy`. Manual impls with no such bounds are the
// standard fix.
impl<'a, C, H, L> Clone for DiscoveryCtx<'a, C, H, L> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'a, C, H, L> Copy for DiscoveryCtx<'a, C, H, L> {}

pub struct DiscoveryServiceImpl<C, H, L = StubNoLlm> {
    config: C,
    http: H,
    llm: L,
    lookup_cache: Arc<Mutex<HashMap<(String, String), CachedLookup>>>,
    resolver: Option<Arc<crate::english_identity_resolver::LiveEnglishIdentityResolver>>,
}

impl<C, H, L> DiscoveryServiceImpl<C, H, L> {
    fn discovery_ctx(&self) -> DiscoveryCtx<'_, C, H, L> {
        DiscoveryCtx {
            config: &self.config,
            http: &self.http,
            llm: &self.llm,
            lookup_cache: &self.lookup_cache,
            resolver: &self.resolver,
        }
    }

    pub fn new(config: C, http: H, llm: L) -> Self {
        Self {
            config,
            http,
            llm,
            lookup_cache: Arc::new(Mutex::new(HashMap::new())),
            resolver: None,
        }
    }

    /// Inject the multi-provider identity resolver so `lookup_filtered` routes
    /// discovery through the federated fan-out (the #97 path) instead of the
    /// legacy sequential lookup chain.
    pub fn with_resolver(
        mut self,
        resolver: Arc<crate::english_identity_resolver::LiveEnglishIdentityResolver>,
    ) -> Self {
        self.resolver = Some(resolver);
        self
    }
}

impl<C, H, L> DiscoveryService for DiscoveryServiceImpl<C, H, L>
where
    C: livrarr_db::ConfigDb + Send + Sync,
    H: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
{
    async fn lookup(&self, req: LookupRequest) -> Result<Vec<LookupResult>, WorkServiceError> {
        lookup(self.discovery_ctx(), req).await
    }

    async fn lookup_filtered(
        &self,
        user_id: UserId,
        req: LookupRequest,
        raw: bool,
    ) -> Result<LookupResponse, WorkServiceError> {
        lookup_filtered(self.discovery_ctx(), user_id, req, raw).await
    }

    async fn eager_match_by_author(
        &self,
        user_id: UserId,
        queries: Vec<EagerQuery>,
    ) -> Result<Vec<(usize, LookupResult)>, WorkServiceError> {
        eager_match_by_author(self.discovery_ctx(), user_id, queries).await
    }
}

/// Take one provider's discovery result (relevance-ordered), logging a failure or
/// timeout rather than failing the whole search. Generic over the provider error
/// type so every provider lookup can share one helper.
fn take_lookup<E: std::fmt::Display>(
    provider: &str,
    term: &str,
    res: Result<Result<Vec<LookupResult>, E>, tokio::time::error::Elapsed>,
) -> Vec<LookupResult> {
    match res {
        Ok(Ok(found)) => found,
        Ok(Err(e)) => {
            tracing::warn!(provider, term, "discovery provider failed: {e}");
            Vec::new()
        }
        Err(_) => {
            tracing::warn!(provider, term, "discovery provider timed out");
            Vec::new()
        }
    }
}

/// Round-robin the per-provider lists in fixed-size chunks so the strongest
/// matches from every provider lead and quality degrades evenly down the combined
/// list — instead of a naive concat (good→bad, good→bad, …). With `chunk = 3` the
/// first rows are the top 3 of each provider, then the next 3 of each, and so on.
fn interleave_by(lists: Vec<Vec<LookupResult>>, chunk: usize) -> Vec<LookupResult> {
    let mut iters: Vec<_> = lists.into_iter().map(|l| l.into_iter()).collect();
    let mut out = Vec::new();
    loop {
        let mut any = false;
        for it in &mut iters {
            for _ in 0..chunk {
                match it.next() {
                    Some(item) => {
                        out.push(item);
                        any = true;
                    }
                    None => break,
                }
            }
        }
        if !any {
            break;
        }
    }
    out
}

/// Map a resolved/confirmable identity into a wire `LookupResult`, carrying the
/// federated anchors + the `candidate_id` payload handle (REQ-014/R-009) and
/// the contributing providers as the result's source attribution (#147 — a
/// source-less result renders chip-less in the search UI).
fn lookup_result_from_captured(
    captured: livrarr_domain::identity::CapturedIdentity,
    candidate_id: Option<livrarr_domain::identity::CandidateId>,
    cover_url: Option<String>,
    sources: &[livrarr_domain::MetadataProvider],
) -> LookupResult {
    let source = if sources.is_empty() {
        None
    } else {
        Some(
            sources
                .iter()
                .map(|p| p.record_key())
                .collect::<Vec<_>>()
                .join("+"),
        )
    };
    LookupResult {
        ol_key: captured.ol_key,
        title: captured.title,
        author_name: captured.author_name,
        author_ol_key: None,
        year: None,
        cover_url,
        description: None,
        series_name: None,
        series_position: None,
        source_type: source.clone(),
        source,
        language: captured.language,
        detail_url: None,
        rating: None,
        isbn_13: captured.isbn_13,
        candidate_id,
        hc_key: captured.hc_key,
        gr_key: captured.gr_key,
        asin: captured.asin,
    }
}

/// Convert a resolver `Resolution` into wire lookup results: a Resolved identity
/// is a single auto-matched result; NeedsConfirmation becomes the candidate list;
/// Unresolved/Conflict yield no results.
fn lookup_results_from_resolution(
    resolution: livrarr_domain::identity::Resolution,
) -> Vec<LookupResult> {
    use livrarr_domain::identity::Resolution;
    match resolution {
        Resolution::Resolved {
            identity,
            candidate_id,
            ..
        } => vec![lookup_result_from_captured(
            identity,
            Some(candidate_id),
            None,
            &[],
        )],
        Resolution::NeedsConfirmation { candidates } => candidates
            .into_iter()
            .map(|c| {
                lookup_result_from_captured(
                    c.anchors,
                    Some(c.candidate_id),
                    c.cover_url,
                    &c.sources,
                )
            })
            .collect(),
        Resolution::Unresolved { .. } | Resolution::Conflict { .. } => Vec::new(),
    }
}

pub(crate) async fn lookup<C, H, L>(
    ctx: DiscoveryCtx<'_, C, H, L>,
    req: LookupRequest,
) -> Result<Vec<LookupResult>, WorkServiceError>
where
    C: livrarr_db::ConfigDb + Send + Sync,
    H: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
{
    let term = req.term.trim().to_string();
    if term.is_empty() {
        return Ok(vec![]);
    }

    let cfg = ctx.config.get_metadata_config().await.ok();
    let default_lang = cfg
        .as_ref()
        .and_then(|c| c.languages.first().cloned())
        .unwrap_or_else(|| "en".to_string());
    let lang = req.lang_override.as_deref().unwrap_or(&default_lang);

    if lang != "en" && !livrarr_external_data::language::is_supported_language(lang) {
        return Err(WorkServiceError::Enrichment(format!(
            "unsupported language: {lang}"
        )));
    }

    // #97 + WCC chunk A: query every provider in parallel and union the
    // results, instead of returning the first that answers. Goodreads joins
    // as a co-equal provider via its WAF-free autocomplete endpoint. Each
    // lookup is timeout-bounded so a slow scrape can't stall the search.
    let provider_timeout = Duration::from_secs(10);
    let (gb, ol, hc, gr) = tokio::join!(
        tokio::time::timeout(provider_timeout, lookup_google_books(ctx, &term, lang)),
        tokio::time::timeout(provider_timeout, lookup_openlibrary(ctx, &term, lang)),
        tokio::time::timeout(provider_timeout, lookup_hardcover(ctx, &term, lang)),
        tokio::time::timeout(provider_timeout, lookup_goodreads(ctx, &term, lang)),
    );

    // Cap each provider to its top 9 (relevance-ordered), then round-robin in
    // chunks of 3 so the strongest matches from every provider lead. Order is
    // language-aware: English leads with the anchor-id providers (Hardcover,
    // OpenLibrary), then Google Books, then Goodreads (scrape, often blocked)
    // last. Non-English leads with Google Books — the foreign-language
    // metadata provider — then OpenLibrary, Hardcover, Goodreads.
    const PER_PROVIDER: usize = 9;
    let mut lists = if lang == "en" {
        vec![
            take_lookup("Hardcover", &term, hc),
            take_lookup("OpenLibrary", &term, ol),
            take_lookup("GoogleBooks", &term, gb),
            take_lookup("Goodreads", &term, gr),
        ]
    } else {
        vec![
            take_lookup("GoogleBooks", &term, gb),
            take_lookup("OpenLibrary", &term, ol),
            take_lookup("Hardcover", &term, hc),
            take_lookup("Goodreads", &term, gr),
        ]
    };
    for l in &mut lists {
        l.truncate(PER_PROVIDER);
    }
    let merged = interleave_by(lists, 3);

    Ok(dedupe_lookup_results(merged))
}

pub(crate) async fn lookup_filtered<C, H, L>(
    ctx: DiscoveryCtx<'_, C, H, L>,
    user_id: UserId,
    req: LookupRequest,
    raw: bool,
) -> Result<LookupResponse, WorkServiceError>
where
    C: livrarr_db::ConfigDb + Send + Sync,
    H: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
{
    let term = req.term.trim().to_lowercase();
    if term.is_empty() {
        return Ok(LookupResponse {
            results: vec![],
            filtered_count: 0,
            raw_count: 0,
            raw_available: false,
        });
    }

    let lang = req
        .lang_override
        .clone()
        .unwrap_or_else(|| "en".to_string());

    // Resolver path (#97 fix): route through the multi-provider fan-out ONLY
    // when the term identifies a specific book (an `isbn:` lookup or another
    // provider key). A bare-title term is a free-text discovery search — it
    // carries no identifier for the resolver to act on (resolve() abstains as
    // EmptySeed), so it falls through to the legacy provider search below.
    let seed = lookup_term_to_seed(&term, &lang);
    if seed_carries_identifier(&seed) {
        if let Some(resolver) = ctx.resolver.clone() {
            use livrarr_domain::services::IdentityResolver;
            let resolution = resolver
                .resolve(
                    user_id,
                    &seed,
                    livrarr_domain::identity::LatencyTier::Interactive,
                )
                .await
                .map_err(|e| WorkServiceError::Validation(format!("resolve failed: {e}")))?;
            let mut results = lookup_results_from_resolution(resolution);
            for r in &mut results {
                r.title = crate::title_cleanup::title_case(&r.title);
            }
            let count = results.len();
            return Ok(LookupResponse {
                results,
                filtered_count: count,
                raw_count: count,
                raw_available: false,
            });
        }
    }

    let cache_key = (term.clone(), lang.clone());

    // Check cache (15 min TTL)
    {
        let cache = ctx.lookup_cache.lock().unwrap();
        if let Some(cached) = cache.get(&cache_key) {
            if cached.created_at.elapsed() < Duration::from_secs(900) {
                let results = if raw || !cached.raw_available {
                    cached.raw.clone()
                } else {
                    cached.filtered.clone()
                };
                return Ok(LookupResponse {
                    filtered_count: cached.filtered.len(),
                    raw_count: cached.raw.len(),
                    raw_available: cached.raw_available,
                    results,
                });
            }
        }
    }

    let mut raw_results: Vec<LookupResult> = lookup(ctx, req).await?;
    for r in &mut raw_results {
        r.title = crate::title_cleanup::title_case(&r.title);
    }
    if raw_results.is_empty() {
        return Ok(LookupResponse {
            results: vec![],
            filtered_count: 0,
            raw_count: 0,
            raw_available: false,
        });
    }

    let raw_count = raw_results.len();

    // Keep the top 9 (the strongest cross-provider matches after the chunked
    // interleave) untouched; LLM-filter only the lower-ranked tail (item 10+)
    // for relevance to the query, so a genuine match in the head is never
    // dropped — only long-tail noise is pruned.
    const KEEP_HEAD: usize = 9;
    let (filtered, raw_available) = if raw_count > KEEP_HEAD {
        let tail = &raw_results[KEEP_HEAD..];
        match llm_filter_search(ctx, &term, tail).await {
            Some(keep) if keep.len() < tail.len() => {
                let mut filtered: Vec<LookupResult> = raw_results[..KEEP_HEAD].to_vec();
                filtered.extend(keep.into_iter().filter_map(|i| tail.get(i).cloned()));
                (filtered, true)
            }
            _ => (raw_results.clone(), false),
        }
    } else {
        (raw_results.clone(), false)
    };

    let filtered_count = filtered.len();

    // Cache both
    {
        let mut cache = ctx.lookup_cache.lock().unwrap();
        // Evict stale entries
        cache.retain(|_, v| v.created_at.elapsed() < Duration::from_secs(900));
        cache.insert(
            cache_key,
            CachedLookup {
                filtered: filtered.clone(),
                raw: raw_results.clone(),
                raw_available,
                created_at: Instant::now(),
            },
        );
    }

    let results = if raw || !raw_available {
        raw_results
    } else {
        filtered
    };

    Ok(LookupResponse {
        results,
        filtered_count,
        raw_count,
        raw_available,
    })
}

pub(crate) async fn eager_match_by_author<C, H, L>(
    ctx: DiscoveryCtx<'_, C, H, L>,
    _user_id: UserId,
    queries: Vec<EagerQuery>,
) -> Result<Vec<(usize, LookupResult)>, WorkServiceError>
where
    C: livrarr_db::ConfigDb + Send + Sync,
    H: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
{
    // Group files by author (case-insensitive). Manual imports cluster
    // heavily by author, so one author-scoped query per provider serves all
    // of that author's files instead of one search per title.
    let mut groups: HashMap<String, Vec<EagerQuery>> = HashMap::new();
    for q in queries {
        groups
            .entry(q.author.trim().to_lowercase())
            .or_default()
            .push(q);
    }

    let mut out: Vec<(usize, LookupResult)> = Vec::new();
    // Files the author-batch could not confidently match. Each gets a
    // per-file 4-way title+author fallback after the batch pass.
    let mut abstained: Vec<EagerQuery> = Vec::new();

    for group in groups.into_values() {
        let author = group[0].author.trim().to_string();
        if author.is_empty() {
            continue;
        }
        let lang = group
            .iter()
            .find_map(|q| q.language.clone())
            .unwrap_or_else(|| "en".to_string());

        // One author-scoped query per provider, in parallel. Google Books
        // (`inauthor:`) leads on coverage; OpenLibrary (`author:`) adds work
        // anchors. Each is timeout-bounded so a slow provider can't stall the
        // batch; a provider that errors or times out simply abstains. Google
        // Books returns empty without a fetch when unconfigured (no API key),
        // which makes the pass OpenLibrary-only for keyless installs.
        let gb_term = format!("inauthor:\"{author}\"");
        let ol_term = format!("author:\"{author}\"");
        let provider_timeout = Duration::from_secs(8);
        let gb_fut = async {
            let t = Instant::now();
            let r =
                tokio::time::timeout(provider_timeout, lookup_google_books(ctx, &gb_term, &lang))
                    .await;
            (r, t.elapsed().as_millis() as u64)
        };
        let ol_fut = async {
            let t = Instant::now();
            let r =
                tokio::time::timeout(provider_timeout, lookup_openlibrary(ctx, &ol_term, &lang))
                    .await;
            (r, t.elapsed().as_millis() as u64)
        };
        let ((gb, gb_ms), (ol, ol_ms)) = tokio::join!(gb_fut, ol_fut);
        tracing::info!(author = %author, gb_ms, ol_ms, "perf eager: provider fetch");

        // Union the author's corpus: Google Books first (coverage/covers),
        // then OpenLibrary (work anchors).
        let mut corpus: Vec<LookupResult> = Vec::new();
        if let Ok(Ok(mut r)) = gb {
            corpus.append(&mut r);
        }
        if let Ok(Ok(mut r)) = ol {
            corpus.append(&mut r);
        }
        if corpus.is_empty() {
            // The whole author corpus is empty (provider error/timeout, or no
            // author-facet hits). Every file in the group falls through to the
            // per-file 4-way fallback.
            abstained.extend(group);
            continue;
        }

        let cand_refs: Vec<(&str, &str)> = corpus
            .iter()
            .map(|c| (c.title.as_str(), c.author_name.as_str()))
            .collect();
        let cand_langs: Vec<Option<&str>> = corpus.iter().map(|c| c.language.as_deref()).collect();

        for q in group {
            // The file's *actual* language (None when unknown) drives the HARD
            // language filter on selection — NOT the per-author query `lang`,
            // which defaults to "en" and would otherwise force an unknown file
            // onto English-only candidates.
            let file_lang = q.language.as_deref();

            // ISBN first: a file's embedded ISBN-13 pins the exact edition in
            // the corpus (Google Books carries isbn_13; OpenLibrary does not),
            // beating any title heuristic. Fall back to the strict title+author
            // cascade when there's no ISBN or no ISBN hit in the corpus.
            let chosen = q
                .isbn
                .as_deref()
                .filter(|s| !s.is_empty())
                .and_then(|isbn| {
                    corpus
                        .iter()
                        .position(|c| c.isbn_13.as_deref() == Some(isbn))
                })
                .or_else(|| {
                    livrarr_matching::work_dedup::best_candidate_index_lang(
                        &cand_refs,
                        &cand_langs,
                        &q.title,
                        &q.author,
                        file_lang,
                    )
                });
            match chosen {
                Some(idx) => out.push((q.id, finalize_eager_pick(idx, &corpus, file_lang))),
                // No confident author-batch match: defer to the per-file
                // 4-way title+author fallback.
                None => abstained.push(q),
            }
        }
    }

    // Per-file fallback for abstained files (#6). The author-scoped batch
    // misses books that ARE findable by title on providers whose author
    // facet is incomplete (e.g. Hardcover returns a title but not an author
    // query). For each abstained file, run the SAME full 4-way discovery the
    // search box uses (`self.lookup`: Google Books + OpenLibrary + Hardcover
    // + Goodreads, parallel, interleaved, deduped) on `"<title> <author>"`,
    // then select with the SAME confident-match guard
    // (`best_candidate_index_lang`: HARD language guard + title/author match)
    // so a wrong book is never auto-picked. A fallback hit receives the same
    // anchor-graft + cover upgrade as a batch hit via `finalize_eager_pick`.
    // Fires only for abstained files (bounded) and runs with bounded
    // concurrency so several abstains don't serialize into many sequential
    // 4-way searches. Goodreads is in the 4-way but only on abstains, so its
    // volume stays low (anti-bot-safe).
    if !abstained.is_empty() {
        const FALLBACK_CONCURRENCY: usize = 4;
        let fallback_hits: Vec<Option<(usize, LookupResult)>> = stream::iter(abstained)
            .map(|q| async move {
                let file_lang = q.language.as_deref();
                let term = format!("{} {}", q.title, q.author);
                let req = LookupRequest {
                    term,
                    lang_override: q.language.clone(),
                };
                // A lookup error (e.g. unsupported language) is treated as an
                // abstain, mirroring how the batch treats a provider failure.
                let candidates = match lookup(ctx, req).await {
                    Ok(c) => c,
                    Err(_) => return None,
                };
                if candidates.is_empty() {
                    return None;
                }
                let cand_refs: Vec<(&str, &str)> = candidates
                    .iter()
                    .map(|c| (c.title.as_str(), c.author_name.as_str()))
                    .collect();
                let cand_langs: Vec<Option<&str>> =
                    candidates.iter().map(|c| c.language.as_deref()).collect();
                let chosen = livrarr_matching::work_dedup::best_candidate_index_lang(
                    &cand_refs,
                    &cand_langs,
                    &q.title,
                    &q.author,
                    file_lang,
                );
                chosen.map(|idx| (q.id, finalize_eager_pick(idx, &candidates, file_lang)))
            })
            .buffer_unordered(FALLBACK_CONCURRENCY)
            .collect()
            .await;
        out.extend(fallback_hits.into_iter().flatten());
    }

    Ok(out)
}

async fn llm_filter_search<C, H, L>(
    ctx: DiscoveryCtx<'_, C, H, L>,
    query: &str,
    results: &[LookupResult],
) -> Option<Vec<usize>>
where
    C: livrarr_db::ConfigDb + Send + Sync,
    H: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
{
    let mut listing = String::new();
    for (i, r) in results.iter().enumerate() {
        listing.push_str(&format!(
            "{}: \"{}\" by {} ({})\n",
            i,
            r.title,
            r.author_name,
            r.year.map(|y| y.to_string()).unwrap_or_default(),
        ));
    }

    let system = "You are a librarian assistant. Clean up book search results.";
    let user_prompt = format!(
        "A user searched a book database for: \"{query}\"\n\n\
         These are lower-ranked results for that query:\n\n\
         {listing}\n\
         Clean up this list:\n\
         1. Remove items not relevant to the query \"{query}\"\n\
         2. Remove non-book items (study guides, journals, blank notebooks, merchandise, board games)\n\
         3. Remove duplicate editions of the same work — keep the one with the best metadata\n\
         4. Remove comic/manga adaptations, movie tie-in editions, and abridged versions\n\
         5. Remove anthologies and compilations unless they are a well-known standalone work\n\
         6. Keep results that are legitimate different works even if titles are similar\n\n\
         Return a JSON array of the original indices to keep, e.g. [0, 2, 5].\n\
         Return ONLY the JSON array, no other text."
    );

    let mut context = HashMap::new();
    context.insert(LlmField::BibliographyHtml, LlmValue::Text(listing));

    let req = LlmCallRequest {
        system_template: system.to_string(),
        user_template: user_prompt,
        context,
        allowed_fields: &[LlmField::BibliographyHtml],
        timeout: Duration::from_secs(30),
        purpose: LlmPurpose::SearchResultCleanup,
    };

    let resp = ctx.llm.call(req).await.ok()?;

    let json_str = crate::strip_llm_fences(&resp.content);

    let indices: Vec<usize> = serde_json::from_str(json_str).ok()?;
    let max_idx = results.len();
    let valid: Vec<usize> = indices.into_iter().filter(|&i| i < max_idx).collect();

    if valid.is_empty() {
        return None;
    }

    Some(valid)
}

async fn lookup_goodreads<C, H, L>(
    ctx: DiscoveryCtx<'_, C, H, L>,
    term: &str,
    // Autocomplete is language-agnostic; the real GR language comes from
    // enrichment (detail-page JSON-LD `inLanguage`), not from discovery.
    _lang: &str,
) -> Result<Vec<LookupResult>, WorkServiceError>
where
    C: livrarr_db::ConfigDb + Send + Sync,
    H: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
{
    // Discovery uses the WAF-free `/book/auto_complete` JSON endpoint. The
    // HTML `/search` page is AWS-WAF 202-challenged (dead); autocomplete
    // returns structured title/author/cover/rating/id with no LLM. Query the
    // term as-is — adding the author demotes the canonical book (author-in-
    // title substring matches rank study guides / adaptations first).
    let url = format!(
        "https://www.goodreads.com/book/auto_complete?format=json&q={}",
        urlencoding::encode(term)
    );

    let fetch_req = FetchRequest {
        url,
        method: HttpMethod::Get,
        headers: vec![("Accept".into(), "application/json".into())],
        body: None,
        timeout: std::time::Duration::from_secs(10),
        rate_bucket: RateBucket::Goodreads,
        max_body_bytes: 2 * 1024 * 1024,
        anti_bot_check: true,
        user_agent: UserAgentProfile::Browser,
        priority: RequestPriority::Normal,
    };

    let resp = match ctx.http.fetch(fetch_req).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("Goodreads autocomplete fetch failed: {e}");
            return Ok(vec![]);
        }
    };

    // 200 is the only "door open" status; a 202 challenge / 4xx / 5xx are
    // transient blocks for discovery — the other providers carry the search.
    if resp.status != 200 {
        tracing::warn!(
            status = resp.status,
            "Goodreads autocomplete returned non-200"
        );
        return Ok(vec![]);
    }

    let body = String::from_utf8_lossy(&resp.body);
    // A non-array body (WAF interstitial / format change) parses to empty.
    let parsed = livrarr_external_data::goodreads::parse_autocomplete_json(&body);

    let results = parsed
        .into_iter()
        .map(|r| {
            let full_url = if r.detail_url.starts_with('/') {
                format!("https://www.goodreads.com{}", r.detail_url)
            } else {
                r.detail_url.clone()
            };
            let validated_url = if livrarr_external_data::goodreads::validate_detail_url(&full_url)
            {
                Some(full_url)
            } else {
                None
            };
            // Canonical Goodreads work anchor from the structured endpoint,
            // normalized to the bare numeric id (the domain canonical form per
            // normalize_gr_key) so it persists and matches consistently.
            let gr_key = validated_url
                .as_deref()
                .and_then(livrarr_external_data::goodreads::extract_gr_key)
                .and_then(|k| livrarr_domain::normalization::normalize_gr_key(&k));
            LookupResult {
                ol_key: None,
                title: r.title,
                author_name: r.author.unwrap_or_default(),
                author_ol_key: None,
                year: r.year,
                cover_url: r.cover_url,
                description: None,
                series_name: r.series_name,
                series_position: r.series_position,
                source: Some("goodreads".to_string()),
                source_type: Some("goodreads".to_string()),
                // Discovery has no language — don't fabricate it from the query
                // term (#11 / 三体=es). Enrichment supplies the real one.
                language: None,
                detail_url: validated_url,
                rating: r.rating,
                isbn_13: None,
                candidate_id: None,
                hc_key: None,
                gr_key,
                asin: None,
            }
        })
        .collect();

    Ok(results)
}

async fn lookup_openlibrary<C, H, L>(
    ctx: DiscoveryCtx<'_, C, H, L>,
    term: &str,
    lang: &str,
) -> Result<Vec<LookupResult>, WorkServiceError>
where
    C: livrarr_db::ConfigDb + Send + Sync,
    H: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
{
    livrarr_external_data::openlibrary::search_openlibrary(ctx.http, term, lang)
        .await
        .map_err(WorkServiceError::Enrichment)
}

async fn lookup_google_books<C, H, L>(
    ctx: DiscoveryCtx<'_, C, H, L>,
    term: &str,
    lang: &str,
) -> Result<Vec<LookupResult>, WorkServiceError>
where
    C: livrarr_db::ConfigDb + Send + Sync,
    H: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
{
    let lang_norm = lang.split('-').next().unwrap_or(lang).to_lowercase();

    let api_key = match ctx.config.get_metadata_config().await {
        Ok(cfg) => match cfg
            .google_books_api_key
            .as_deref()
            .filter(|s| !s.is_empty())
        {
            Some(k) => k.to_string(),
            None => {
                tracing::debug!(term = %term, "GoogleBooks: no API key configured; skipping");
                return Ok(vec![]);
            }
        },
        Err(_) => return Ok(vec![]),
    };

    let url = format!(
        "https://www.googleapis.com/books/v1/volumes\
         ?q={}&langRestrict={}&maxResults=20",
        urlencoding::encode(term),
        urlencoding::encode(&lang_norm),
    );

    let volumes = livrarr_external_data::google_books::fetch_gb_volumes(
        ctx.http,
        &api_key,
        url,
        RequestPriority::Interactive,
    )
    .await
    .map_err(WorkServiceError::Enrichment)?;

    let results = volumes
        .iter()
        .filter_map(|vol| {
            let vi = vol.volume_info.as_ref()?;
            let title = vi.title.as_ref()?.clone();
            let author_name = vi
                .authors
                .as_ref()
                .and_then(|a| a.first())
                .cloned()
                .unwrap_or_else(|| "Unknown".to_string());
            let year = vi
                .published_date
                .as_deref()
                .and_then(|d| d.get(..4))
                .and_then(|y| y.parse::<i32>().ok());
            let cover_url = vi
                .image_links
                .as_ref()
                .and_then(livrarr_external_data::google_books::normalize_cover_url);
            // REQ-011: never stamp the query language onto a result — a
            // payload without one stays language-unknown (#11, GB path).
            let language = vi.language.clone();

            Some(LookupResult {
                ol_key: None,
                title,
                author_name,
                author_ol_key: None,
                year,
                cover_url,
                description: None,
                series_name: None,
                series_position: None,
                source: Some("google_books".into()),
                source_type: Some("search".into()),
                language,
                detail_url: None,
                rating: None,
                isbn_13: livrarr_external_data::google_books::extract_isbn13(
                    &vi.industry_identifiers,
                ),
                candidate_id: None,
                hc_key: None,
                gr_key: None,
                asin: None,
            })
        })
        .collect();

    Ok(results)
}

async fn lookup_hardcover<C, H, L>(
    ctx: DiscoveryCtx<'_, C, H, L>,
    term: &str,
    _lang: &str,
) -> Result<Vec<LookupResult>, WorkServiceError>
where
    C: livrarr_db::ConfigDb + Send + Sync,
    H: HttpFetcher + Send + Sync,
    L: LlmCaller + Send + Sync,
{
    let cfg = match ctx.config.get_metadata_config().await {
        Ok(c) => c,
        Err(_) => return Ok(vec![]),
    };

    if !cfg.hardcover_enabled {
        return Ok(vec![]);
    }

    let token = match cfg
        .hardcover_api_token
        .as_deref()
        .map(|t| {
            t.trim()
                .trim_start_matches("Bearer ")
                .trim_start_matches("bearer ")
        })
        .filter(|t| !t.is_empty())
    {
        Some(t) => t.to_string(),
        None => return Ok(vec![]),
    };

    let body = livrarr_external_data::hardcover::hc_search_body(15, term);
    let body_bytes = serde_json::to_vec(&body)
        .map_err(|e| WorkServiceError::Enrichment(format!("HC serialize: {e}")))?;

    let resp = ctx
        .http
        .fetch(livrarr_domain::services::FetchRequest {
            url: livrarr_external_data::hardcover::HARDCOVER_API_URL.to_string(),
            method: livrarr_domain::services::HttpMethod::Post,
            headers: vec![
                ("Authorization".into(), format!("Bearer {token}")),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: Some(body_bytes),
            timeout: std::time::Duration::from_secs(10),
            rate_bucket: livrarr_domain::services::RateBucket::Hardcover,
            max_body_bytes: 2 * 1024 * 1024,
            anti_bot_check: false,
            user_agent: livrarr_domain::services::UserAgentProfile::Server,
            priority: livrarr_domain::RequestPriority::Normal,
        })
        .await
        .map_err(|e| WorkServiceError::Enrichment(format!("HC search: {e}")))?;

    if resp.status >= 400 {
        return Err(WorkServiceError::Enrichment(format!(
            "HC search HTTP {}",
            resp.status
        )));
    }

    let data: serde_json::Value = serde_json::from_slice(&resp.body)
        .map_err(|e| WorkServiceError::Enrichment(format!("HC parse: {e}")))?;

    let hits = livrarr_external_data::hardcover::hc_extract_hits(&data);

    let results: Vec<LookupResult> = hits
        .iter()
        .filter_map(|hit| {
            let doc = hit.get("document")?;
            let title = doc.get("title")?.as_str()?.to_string();
            let author_name = doc
                .get("author_names")
                .and_then(|a| a.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();

            let cover_url = doc
                .pointer("/image/url")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            // The Hardcover work id (same extraction the HC client uses) is a
            // work anchor, so a picked HC result is trusted (zero-network add)
            // instead of falling back to an ISBN re-resolve.
            let hc_key = doc
                .get("id")
                .map(|v| v.to_string().trim_matches('"').to_string());

            Some(LookupResult {
                ol_key: None,
                title,
                author_name,
                author_ol_key: None,
                year: None,
                cover_url,
                description: None,
                series_name: None,
                series_position: None,
                source: Some("hardcover".into()),
                source_type: Some("search".into()),
                language: None,
                detail_url: None,
                rating: None,
                isbn_13: None,
                candidate_id: None,
                hc_key,
                gr_key: None,
                asin: None,
            })
        })
        .collect();

    Ok(results)
}

/// Collapse duplicate works merged from multiple discovery providers. Prefers an
/// ISBN-13 match; otherwise keys on normalized title + author. First occurrence
/// wins, so provider order (Google Books, OpenLibrary, Hardcover) breaks ties.
fn dedupe_lookup_results(results: Vec<LookupResult>) -> Vec<LookupResult> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(results.len());
    for r in results {
        let key = match r
            .isbn_13
            .as_deref()
            .and_then(livrarr_domain::normalization::normalize_isbn13)
        {
            Some(isbn) => format!("isbn:{isbn}"),
            None => format!(
                "ta:{}|{}",
                r.title.trim().to_lowercase(),
                r.author_name.trim().to_lowercase()
            ),
        };
        if seen.insert(key) {
            out.push(r);
        }
    }
    out
}

/// Rank a cover by the quality of its hosting source — derived from the SAME
/// unified rank table the live picker and comparator consume (S1), not an ad
/// hoc tier scheme. Higher is better. `foreign` selects the ebook-foreign vs
/// ebook-english order (the seed-cover selection this feeds is always an
/// ebook concern). A host mapping to a known provider ranks by that
/// provider's table position; an unrecognized non-empty host ranks above
/// nothing but below every known provider; empty ranks lowest.
fn cover_source_rank(url: &str, foreign: bool) -> u8 {
    if url.is_empty() {
        return 0;
    }
    match livrarr_enrichment::cover_rank::provider_for_cover_host(url) {
        Some(provider) => {
            let model = livrarr_enrichment::cover_rank::CoverRankModel::for_ebook(foreign);
            let len = livrarr_enrichment::cover_rank::rank_table(model).len();
            let idx = livrarr_enrichment::cover_rank::rank_index(provider, model);
            (len - idx + 1) as u8
        }
        None => 1,
    }
}

/// Finalize a confident eager-match pick: take the selected candidate from a
/// candidate corpus and apply the two consistency upgrades that every eager hit
/// receives — an anchor-graft (so an ISBN/Google-Books-only pick gains a work
/// anchor and can be created Confirmed) and a best-source cover upgrade. Both
/// upgrades enforce the same HARD language guard (`file_lang`, when known): an
/// anchor or cover is never borrowed across languages. Shared by the
/// author-batch pass and the per-file 4-way fallback so both treat a hit
/// identically.
fn finalize_eager_pick(
    idx: usize,
    corpus: &[LookupResult],
    file_lang: Option<&str>,
) -> LookupResult {
    let mut result = corpus[idx].clone();
    // The pick is often a Google Books / ISBN hit, which carries a cover + ISBN
    // but NO work anchor. Graft an anchor from a same-title candidate in the
    // corpus so the work can be created Confirmed (and enrich directly) rather
    // than landing ISBN-only and relying on background convergence.
    let has_anchor = result.ol_key.is_some() || result.gr_key.is_some() || result.hc_key.is_some();
    if !has_anchor {
        let norm = livrarr_matching::work_dedup::normalize_title_for_match(&result.title);
        // HARD language guard (#8): when the file's language is known, only graft
        // an anchor from a same-language candidate — never lend a
        // different-language work's anchor.
        let want_lang = file_lang.and_then(livrarr_domain::normalization::normalize_language);
        if let Some(anchored) = corpus.iter().find(|c| {
            (c.ol_key.is_some() || c.gr_key.is_some() || c.hc_key.is_some())
                && livrarr_matching::work_dedup::normalize_title_for_match(&c.title) == norm
                && livrarr_matching::work_dedup::authors_match(&c.author_name, &result.author_name)
                && match want_lang {
                    Some(ref want) => {
                        c.language
                            .as_deref()
                            .and_then(livrarr_domain::normalization::normalize_language)
                            == Some(want.clone())
                    }
                    None => true,
                }
        }) {
            result.ol_key = anchored.ol_key.clone();
            result.author_ol_key = anchored.author_ol_key.clone();
            if result.gr_key.is_none() {
                result.gr_key = anchored.gr_key.clone();
            }
            if result.hc_key.is_none() {
                result.hc_key = anchored.hc_key.clone();
            }
        }
    }
    // Cover-quality upgrade: the matched work/edition stays as selected, but its
    // cover is replaced with the best-source cover among same-work corpus
    // candidates (e.g. a Google Books full-res image instead of an OpenLibrary
    // `-M` thumbnail). The same language guard as the anchor-graft applies so a
    // cover is never borrowed across languages.
    if let Some(better) = best_same_work_cover(&result, corpus, file_lang) {
        result.cover_url = Some(better);
    }
    result
}

/// Among `corpus` candidates that represent the SAME work as `selected`, return
/// the best-quality cover URL by source rank. "Same work" = matching normalized
/// title + author; when `want_lang` is set (the file's known language) only
/// same-language candidates are considered, so a cover is never borrowed across
/// languages. Returns `None` when no same-work candidate has a cover that
/// outranks the selected candidate's own cover (stable: ties keep the original).
fn best_same_work_cover(
    selected: &LookupResult,
    corpus: &[LookupResult],
    want_lang: Option<&str>,
) -> Option<String> {
    let norm = livrarr_matching::work_dedup::normalize_title_for_match(&selected.title);
    let want = want_lang.and_then(livrarr_domain::normalization::normalize_language);
    // S1: one rank table drives this too — the file's own language (when
    // known) picks ebook-english vs ebook-foreign, the same classifier the
    // picker and comparator use.
    let foreign = matches!(
        livrarr_external_data::language::provider_priority(want_lang),
        livrarr_external_data::language::ProviderPriority::Foreign
    );
    let mut best_url: Option<&str> = selected.cover_url.as_deref().filter(|u| !u.is_empty());
    let mut best_rank = best_url.map(|u| cover_source_rank(u, foreign)).unwrap_or(0);

    for c in corpus {
        let url = match c.cover_url.as_deref().filter(|u| !u.is_empty()) {
            Some(u) => u,
            None => continue,
        };
        if livrarr_matching::work_dedup::normalize_title_for_match(&c.title) != norm {
            continue;
        }
        if !livrarr_matching::work_dedup::authors_match(&c.author_name, &selected.author_name) {
            continue;
        }
        // HARD language guard: when the file's language is known, only consider
        // same-language candidates for the cover upgrade.
        if let Some(ref want) = want {
            let cand = c
                .language
                .as_deref()
                .and_then(livrarr_domain::normalization::normalize_language);
            if cand.as_ref() != Some(want) {
                continue;
            }
        }
        let rank = cover_source_rank(url, foreign);
        if rank > best_rank {
            best_rank = rank;
            best_url = Some(url);
        }
    }

    match best_url {
        Some(u) if Some(u) != selected.cover_url.as_deref() => Some(u.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    fn lr(title: &str, author: &str, isbn: Option<&str>) -> LookupResult {
        LookupResult {
            ol_key: None,
            title: title.into(),
            author_name: author.into(),
            author_ol_key: None,
            year: None,
            cover_url: None,
            description: None,
            series_name: None,
            series_position: None,
            source: None,
            source_type: None,
            language: None,
            detail_url: None,
            rating: None,
            isbn_13: isbn.map(|s| s.into()),
            candidate_id: None,
            hc_key: None,
            gr_key: None,
            asin: None,
        }
    }

    #[test]
    fn dedupe_keeps_distinct_works_from_all_providers() {
        // A Hardcover-only book must survive a merge where Google Books already
        // returned results (the #97 regression); duplicates collapse to one.
        let merged = dedupe_lookup_results(vec![
            lr("Google Result", "Author A", Some("9780000000001")),
            lr("Hardcover Only", "Author B", Some("9780000000002")),
            lr("Google Result", "Author A", Some("9780000000001")), // dup by isbn
            lr("No ISBN Book", "Author C", None),
            lr("No ISBN Book", "Author C", None), // dup by title+author
        ]);
        let titles: Vec<&str> = merged.iter().map(|r| r.title.as_str()).collect();
        assert!(
            titles.contains(&"Hardcover Only"),
            "HC-only book was dropped"
        );
        assert_eq!(merged.len(), 3, "expected 3 distinct works after dedupe");
    }

    #[test]
    fn interleave_round_robins_in_chunks() {
        // chunk=2: first 2 of A, first 2 of B, then A's remainder — so each
        // provider's strongest hits lead and quality degrades evenly.
        let a = vec![
            lr("A0", "x", None),
            lr("A1", "x", None),
            lr("A2", "x", None),
        ];
        let b = vec![lr("B0", "y", None), lr("B1", "y", None)];
        let out = interleave_by(vec![a, b], 2);
        let titles: Vec<&str> = out.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, vec!["A0", "A1", "B0", "B1", "A2"]);
    }

    #[test]
    fn interleave_handles_uneven_and_empty_lists() {
        let a = vec![lr("A0", "x", None)];
        let empty: Vec<LookupResult> = vec![];
        let c = vec![lr("C0", "z", None), lr("C1", "z", None)];
        let out = interleave_by(vec![a, empty, c], 3);
        let titles: Vec<&str> = out.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(titles, vec!["A0", "C0", "C1"]);
    }

    #[test]
    fn take_lookup_passes_ok_and_swallows_err() {
        let ok: Result<Result<Vec<LookupResult>, String>, tokio::time::error::Elapsed> =
            Ok(Ok(vec![lr("Hit", "a", None)]));
        assert_eq!(take_lookup("P", "t", ok).len(), 1);

        // A provider error degrades to an empty contribution, never failing the
        // whole search (the timeout arm behaves identically).
        let err: Result<Result<Vec<LookupResult>, String>, tokio::time::error::Elapsed> =
            Ok(Err("provider boom".to_string()));
        assert!(take_lookup("P", "t", err).is_empty());
    }

    fn lr_cover(
        title: &str,
        author: &str,
        lang: Option<&str>,
        cover: Option<&str>,
    ) -> LookupResult {
        LookupResult {
            language: lang.map(|s| s.into()),
            cover_url: cover.map(|s| s.into()),
            ..lr(title, author, None)
        }
    }

    #[test]
    fn cover_rank_prefers_high_res_sources_over_openlibrary() {
        let gb = "https://books.google.com/books/content?id=abc&img=1";
        let ol = "https://covers.openlibrary.org/b/id/123-L.jpg";
        let amazon = "https://images-amazon.com/images/I/x.jpg";
        let hc = "https://assets.hardcover.app/cover.jpg";
        assert!(cover_source_rank(gb, false) > cover_source_rank(ol, false));
        assert!(cover_source_rank(amazon, false) > cover_source_rank(ol, false));
        assert!(cover_source_rank(hc, false) > cover_source_rank(ol, false));
        assert!(cover_source_rank(ol, false) > cover_source_rank("", false));
    }

    #[test]
    fn cover_rank_unknown_host_ranks_above_empty_below_known_providers() {
        let unknown = "https://random-cdn.example.com/x.jpg";
        let ol = "https://covers.openlibrary.org/b/id/123-L.jpg";
        assert!(cover_source_rank(unknown, false) > cover_source_rank("", false));
        assert!(cover_source_rank(ol, false) > cover_source_rank(unknown, false));
    }

    #[test]
    fn cover_rank_foreign_prefers_googlebooks_over_goodreads_hosted_amazon() {
        // S1: foreign order is GB -> GR -> ... — the opposite of English,
        // where GR/amazon-family outranks GB.
        let gb = "https://books.google.com/books/content?id=abc&img=1";
        let amazon = "https://images-amazon.com/images/I/x.jpg";
        assert!(cover_source_rank(gb, true) > cover_source_rank(amazon, true));
        assert!(cover_source_rank(amazon, false) > cover_source_rank(gb, false));
    }

    #[test]
    fn cover_upgrade_picks_google_over_openlibrary_for_same_work() {
        let selected = lr_cover(
            "The Hobbit",
            "Tolkien",
            None,
            Some("https://covers.openlibrary.org/b/id/123-M.jpg"),
        );
        let corpus = vec![
            selected.clone(),
            lr_cover(
                "The Hobbit",
                "Tolkien",
                None,
                Some("https://books.google.com/books/content?id=hobbit"),
            ),
        ];
        let better = best_same_work_cover(&selected, &corpus, None);
        assert_eq!(
            better.as_deref(),
            Some("https://books.google.com/books/content?id=hobbit")
        );
    }

    #[test]
    fn cover_upgrade_keeps_openlibrary_when_only_source() {
        let selected = lr_cover(
            "The Hobbit",
            "Tolkien",
            None,
            Some("https://covers.openlibrary.org/b/id/123-L.jpg"),
        );
        let corpus = vec![selected.clone()];
        // No higher-ranked same-work cover exists, so no upgrade is returned.
        assert_eq!(best_same_work_cover(&selected, &corpus, None), None);
    }

    #[test]
    fn cover_upgrade_does_not_borrow_other_language_cover() {
        // German pick; an English same-title edition has a Google cover, but the
        // known file language is German, so its cover must NOT be borrowed.
        let selected = lr_cover(
            "Der Hobbit",
            "Tolkien",
            Some("de"),
            Some("https://covers.openlibrary.org/b/id/123-M.jpg"),
        );
        let corpus = vec![
            selected.clone(),
            lr_cover(
                "Der Hobbit",
                "Tolkien",
                Some("en"),
                Some("https://books.google.com/books/content?id=eng"),
            ),
        ];
        assert_eq!(best_same_work_cover(&selected, &corpus, Some("de")), None);
    }

    // ---- Phase 5 Unit E: anchor-graft/cover-borrow seat pinned under the
    // identity authority (REQ-014, site 7 — normalize_title_for_match and
    // authors_match now route through identity_matching) -------------------

    #[test]
    fn cover_upgrade_recognizes_same_work_across_a_subtitle() {
        // The bare title (no subtitle) and a subtitled variant of the same
        // book must still be treated as the same work for cover-borrowing —
        // normalize_title_for_match now derives from parse_title's main
        // title (site 7), which drops any tail exactly as the old colon-cut
        // did for this shape, so this pins that the routing didn't regress
        // the seat's basic case.
        let selected = lr_cover(
            "The Hobbit",
            "Tolkien",
            None,
            Some("https://covers.openlibrary.org/b/id/123-M.jpg"),
        );
        let corpus = vec![
            selected.clone(),
            lr_cover(
                "The Hobbit: There and Back Again",
                "Tolkien",
                None,
                Some("https://books.google.com/books/content?id=hobbit"),
            ),
        ];
        let better = best_same_work_cover(&selected, &corpus, None);
        assert_eq!(
            better.as_deref(),
            Some("https://books.google.com/books/content?id=hobbit"),
            "a subtitled same-work candidate must still be recognized for the cover upgrade"
        );
    }

    #[test]
    fn finalize_eager_pick_grafts_anchor_from_subtitled_same_work_candidate() {
        // Mirrors the cover-upgrade case for the anchor-graft half of
        // finalize_eager_pick: an anchorless pick (e.g. an ISBN-only Google
        // Books hit) borrows a work anchor from a same-work candidate whose
        // title carries a subtitle the pick's title doesn't.
        let anchorless = LookupResult {
            ol_key: None,
            ..lr("The Hobbit", "Tolkien", Some("9780000000001"))
        };
        let anchored = LookupResult {
            ol_key: Some("/works/OL1W".to_string()),
            ..lr("The Hobbit: There and Back Again", "Tolkien", None)
        };
        let corpus = vec![anchorless.clone(), anchored];
        let result = finalize_eager_pick(0, &corpus, None);
        assert_eq!(result.ol_key.as_deref(), Some("/works/OL1W"));
    }
}
