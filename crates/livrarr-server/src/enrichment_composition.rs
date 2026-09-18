//! Composition of the live enrichment pipeline: the provider queue, the merge
//! engine and the enrichment service the server serves requests from.
//!
//! The merge engine is built from the provider priorities stored in the
//! database (REQ-001). They are read once, here, after migrations and before
//! anything serves a request or runs background enrichment; the resulting
//! snapshot is immutable for the life of the process, so a priority edited
//! afterwards takes effect on the next restart. A load or validation failure is
//! returned, not swallowed — running a hardcoded order instead would hide a
//! misconfiguration behind plausible-looking results.

use std::sync::Arc;

use livrarr_db::ProviderPolicyDb;

/// The enrichment pipeline could not be composed because the stored provider
/// priorities could not be read or are not usable.
#[derive(Debug, thiserror::Error)]
pub enum EnrichmentCompositionError {
    #[error("reading provider priorities from the database failed: {0}")]
    LoadPolicy(#[from] livrarr_domain::DbError),
    #[error("stored provider priorities are not usable: {0}")]
    InvalidPolicy(#[from] livrarr_domain::services::ProviderPolicyError),
}

/// Build the live `DefaultProviderQueue` + `EnrichmentServiceImpl` from the
/// stored provider priorities and a startup-time snapshot of `MetadataConfig`.
///
/// What a restart does and does not gate:
/// * the provider priorities are read once here, so a priority row edited later
///   takes effect on the next restart;
/// * the Audnexus URL is the value captured in `cfg_snapshot`, so changing it
///   also needs a restart;
/// * Hardcover's token and enabled flag and the Google Books API key are NOT
///   captured here. Those clients hold `live_metadata_config` and read it per
///   fetch, so adding a token or a key takes effect on the next enrichment.
pub async fn build_enrichment_pipeline(
    db: &livrarr_db::sqlite::SqliteDb,
    http_fetcher: &livrarr_http::fetcher::HttpFetcherImpl,
    goodreads_client: livrarr_external_data::GoodreadsClient,
    live_metadata_config: &livrarr_external_data::live_config::LiveMetadataConfig,
    call_sink: &Arc<dyn livrarr_domain::services::ProviderCallSink>,
    transport_cache: &Arc<livrarr_external_data::transport_cache::TransportCache>,
    metadata_cache: &crate::config::MetadataCacheConfig,
) -> Result<
    (
        Arc<crate::state::LiveProviderQueue>,
        Arc<crate::state::LiveEnrichmentService>,
    ),
    EnrichmentCompositionError,
> {
    use livrarr_domain::MetadataProvider as P;
    use livrarr_metadata as m;

    let cfg_snapshot = live_metadata_config.snapshot();

    let queue_cfg = |provider| m::ProviderQueueConfig {
        provider,
        max_attempts: 5,
    };

    let mut builder = m::DefaultProviderQueueBuilder::new();

    // Audnexus — always available. URL is captured at startup; if you
    // want a custom audnexus_url to take effect live too, that's a
    // small follow-up (same LiveMetadataConfig pattern).
    builder = builder.add_provider(
        P::Audnexus,
        livrarr_external_data::ProviderClient::Audnexus(
            livrarr_external_data::AudnexusClient::new(
                http_fetcher.clone(),
                cfg_snapshot.audnexus_url.clone(),
            ),
        )
        .with_call_sink(call_sink.clone()),
        queue_cfg(P::Audnexus),
    );

    // OpenLibrary — always available, no credentials needed.
    builder = builder.add_provider(
        P::OpenLibrary,
        livrarr_external_data::ProviderClient::OpenLibrary(
            livrarr_external_data::OpenLibraryClient::new(http_fetcher.clone()),
        )
        .with_call_sink(call_sink.clone()),
        queue_cfg(P::OpenLibrary),
    );

    // Hardcover — always registered. The client itself reads the live
    // config per-fetch; if `hardcover_enabled=false` or the token is
    // empty, it returns NotFound without a network call. Enabling HC
    // via the UI takes effect on the next enrichment.
    builder = builder.add_provider(
        P::Hardcover,
        livrarr_external_data::ProviderClient::Hardcover(
            livrarr_external_data::HardcoverClient::new(
                http_fetcher.clone(),
                live_metadata_config.clone(),
            ),
        )
        .with_call_sink(call_sink.clone()),
        queue_cfg(P::Hardcover),
    );

    // Goodreads — always registered. The LLM extraction fallback for
    // foreign-language pages reads live config per-fetch.
    builder = builder.add_provider(
        P::Goodreads,
        livrarr_external_data::ProviderClient::Goodreads(goodreads_client)
            .with_call_sink(call_sink.clone()),
        queue_cfg(P::Goodreads),
    );

    // Google Books — always registered. Reads API key from live config per-fetch.
    builder = builder.add_provider(
        P::GoogleBooks,
        livrarr_external_data::ProviderClient::GoogleBooks(
            livrarr_external_data::GoogleBooksClient::new(
                http_fetcher.clone(),
                live_metadata_config.clone(),
            ),
        )
        .with_call_sink(call_sink.clone()),
        queue_cfg(P::GoogleBooks),
    );

    // Audible — always registered. Unauthenticated API, no config needed.
    builder = builder.add_provider(
        P::Audible,
        livrarr_external_data::ProviderClient::Audible(
            livrarr_external_data::audible::AudibleCatalogClient::new(http_fetcher.clone(), 5 * 60),
        )
        .with_call_sink(call_sink.clone()),
        queue_cfg(P::Audible),
    );

    builder = builder.with_applicability_rule(Arc::new(|provider, work| {
        if matches!(
            livrarr_external_data::language::provider_priority(work.language.as_deref()),
            livrarr_external_data::language::ProviderPriority::English
        ) {
            // REQ-002: Google Books enriches English works too. Its API key and
            // the identity requirements on its fetch still gate whether it
            // returns anything.
            return true;
        }
        // A foreign-language work never takes Hardcover or OpenLibrary
        // metadata (REQ-014/#133); Readarr source data has no foreign catalogue.
        matches!(
            provider,
            P::Goodreads | P::Audnexus | P::GoogleBooks | P::Audible
        )
    }));

    // Pipeline-level skip records (no anchor / policy) flow through the
    // queue's own sink seam (REQ-001).
    builder = builder.with_call_sink(call_sink.clone());

    // Persistent provider-response cache (REQ-009): TOML-configured TTL
    // and row cap, no env-var override (Servarr convention).
    builder = builder.with_provider_cache(
        chrono::Duration::days(metadata_cache.ttl_days as i64),
        metadata_cache.max_rows,
    );

    let db_arc = Arc::new(db.clone());
    let queue = Arc::new(builder.build(db_arc.clone()));

    // Merge engine: purely deterministic (REQ-005) — no LLM is consulted
    // anywhere in merge. Its provider order is the policy stored in the
    // database, read once here (REQ-001) and shared by the fresh-dispatch and
    // cached-reuse paths for the life of the process.
    let policy = db.load_provider_policy_snapshot().await?;
    policy.validate()?;
    log_loaded_policy(&policy);
    let merge_engine = Arc::new(m::DefaultMergeEngine::from_policy(Arc::new(policy)));

    let llm_configured = live_metadata_config.snapshot().llm_enabled;
    // Author-name observation (REQ-002): every successful provider payload's
    // author name is retained as a ranked variant, so the library can converge on
    // the spelling the providers agree on instead of the first one imported.
    let name_observer = Arc::new(
        livrarr_metadata::author_name_variant_observer::DbAuthorNameObservationSink::new(
            db_arc.clone(),
        ),
    );
    let service = Arc::new(
        m::EnrichmentServiceImpl::new(db_arc, queue.clone(), merge_engine, llm_configured)
            .with_transport_cache(transport_cache.clone())
            .with_call_sink(call_sink.clone())
            .with_author_name_observer(name_observer),
    );
    Ok((queue, service))
}

/// Record the provider order this process will enrich with. Provider names and
/// language codes only — the policy carries no credentials, and none are read
/// here.
fn log_loaded_policy(policy: &livrarr_domain::services::ProviderPolicySnapshot) {
    use livrarr_domain::services::ListKind;

    let order = |list: &livrarr_domain::services::ProviderList| {
        list.entries
            .iter()
            .map(|entry| entry.provider.record_key())
            .collect::<Vec<_>>()
            .join(" > ")
    };
    let mut languages: Vec<&String> = policy.by_language.keys().collect();
    languages.sort();
    tracing::info!(
        metadata = %order(policy.generic.list_for(ListKind::Ebook)),
        audio = %order(policy.generic.list_for(ListKind::Audiobook)),
        "loaded provider priorities for any language without its own list"
    );
    for language in languages {
        let group = &policy.by_language[language];
        tracing::info!(
            language = %language,
            metadata = %order(group.list_for(ListKind::Ebook)),
            audio = %order(group.list_for(ListKind::Audiobook)),
            "loaded provider priorities"
        );
    }
}
