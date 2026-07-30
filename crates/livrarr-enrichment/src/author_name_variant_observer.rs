use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use livrarr_db::{AuthorNameVariantDb, WorkDb};
use livrarr_domain::identity_matching::canonical_author_key;
use livrarr_domain::{
    AuthorNameSource, MetadataProvider, ProviderAuthorNameObservation, UserId, WorkId,
};
use livrarr_external_data::NormalizedWorkDetail;

/// Records the author names successful provider payloads carried.
///
/// Record-only and infallible by contract (FP-041): enrichment's own result is
/// never changed by what happens here, and a storage failure is a warning, not an
/// error the caller has to handle. It writes name variants and nothing else — no
/// route, no rename, no work identity.
pub struct AuthorNameVariantObserver;

impl AuthorNameVariantObserver {
    /// Record every usable name from one work's successful provider outcomes.
    ///
    /// Names are trimmed and reduced to one row per (source, canonical spelling),
    /// so the same person spelled the same way by one provider is a single
    /// observation however many payloads carried it. An empty set writes nothing
    /// at all.
    pub async fn record_observed_author_names(
        db: &(impl AuthorNameVariantDb + WorkDb),
        user_id: UserId,
        work_id: WorkId,
        observations: &[ProviderAuthorNameObservation],
    ) {
        let mut seen: Vec<(AuthorNameSource, String)> = Vec::new();
        let mut usable: Vec<ProviderAuthorNameObservation> = Vec::new();
        for observation in observations {
            let name = observation.name.trim();
            let canonical = canonical_author_key(name);
            if name.is_empty() || canonical.is_empty() {
                continue;
            }
            // The true source is kept: a Google Books or Readarr spelling ranks
            // differently from a Goodreads one, and collapsing them would throw
            // that away.
            let key = (observation.source, canonical);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            usable.push(ProviderAuthorNameObservation {
                source: observation.source,
                name: name.to_string(),
            });
        }
        if usable.is_empty() {
            return;
        }
        if let Err(error) = db.record_observed_names(user_id, work_id, &usable).await {
            tracing::warn!(
                work_id,
                %error,
                "author name observation not recorded; enrichment result is unaffected"
            );
        }
    }
}

/// The author name a successful provider payload carried, if that provider is
/// one whose names are a ranked source.
///
/// Audnexus, Audible, and the LLM have no author-name source of their own, so a
/// name arriving on one of their payloads is not recorded as an observation.
pub fn observed_author_name(
    provider: MetadataProvider,
    detail: &NormalizedWorkDetail,
) -> Option<ProviderAuthorNameObservation> {
    let source = match provider {
        MetadataProvider::Goodreads => AuthorNameSource::Goodreads,
        MetadataProvider::Hardcover => AuthorNameSource::Hardcover,
        MetadataProvider::GoogleBooks => AuthorNameSource::GoogleBooks,
        MetadataProvider::OpenLibrary => AuthorNameSource::OpenLibrary,
        MetadataProvider::Readarr => AuthorNameSource::Readarr,
        MetadataProvider::Audnexus | MetadataProvider::Audible | MetadataProvider::Llm => {
            return None
        }
    };
    let name = detail.author_name.as_deref()?.trim();
    if name.is_empty() {
        return None;
    }
    Some(ProviderAuthorNameObservation {
        source,
        name: name.to_string(),
    })
}

/// The dyn-safe seam the enrichment spine records observations through.
///
/// `enrich_work` is generic over a DB type whose trait bound is fixed by
/// existing callers, so the name-variant capability arrives as an optional
/// handle the composition root installs — the same shape as the provider-call
/// sink beside it. A composition that does not install one records nothing.
pub trait AuthorNameObservationSink: Send + Sync {
    fn record<'a>(
        &'a self,
        user_id: UserId,
        work_id: WorkId,
        observations: Vec<ProviderAuthorNameObservation>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>>;
}

/// The production sink: the observer over the real repository.
pub struct DbAuthorNameObservationSink<D> {
    db: Arc<D>,
}

impl<D> DbAuthorNameObservationSink<D> {
    pub fn new(db: Arc<D>) -> Self {
        Self { db }
    }
}

impl<D> AuthorNameObservationSink for DbAuthorNameObservationSink<D>
where
    D: AuthorNameVariantDb + WorkDb + Send + Sync + 'static,
{
    fn record<'a>(
        &'a self,
        user_id: UserId,
        work_id: WorkId,
        observations: Vec<ProviderAuthorNameObservation>,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            AuthorNameVariantObserver::record_observed_author_names(
                self.db.as_ref(),
                user_id,
                work_id,
                &observations,
            )
            .await;
        })
    }
}
