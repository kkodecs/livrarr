//! The author-link cutover gate, the recurring sweep tick, and the Readarr
//! keyed-evidence helper.
//!
//! These three are the server's whole share of author linking. The gate runs
//! once at startup and decides whether the route ledger is complete enough to
//! serve from; the tick is the only thing that drives the road automatically;
//! the Readarr helper turns one import record's own claim about an author into
//! guarded evidence — or into nothing.

use livrarr_db::{sqlite::SqliteDb, AuthorLinkDb, AuthorRouteBackfillReport};
use livrarr_domain::services::AuthorLinkWorkflow;
use livrarr_domain::{
    guard_author_route, AuthorProvider, AuthorRouteEvidenceSource, AuthorRouteGuardResult,
    AuthorRouteKey, ProviderAuthorRef, ProviderCredit,
};
use tokio_util::sync::CancellationToken;

use crate::readarr_client::RdAuthor;
use crate::state::AppState;

/// How many authors one sweep tick claims. Bounded so a tick is a short piece of
/// work that can be interrupted and resumed, never a library-sized transaction.
const DEFAULT_SWEEP_BATCH: u32 = 25;

/// Startup refused to proceed. It carries the report so the operator can see
/// exactly what is missing rather than a bare failure.
#[derive(Debug)]
pub struct StartupError {
    pub message: String,
    pub report: Option<AuthorRouteBackfillReport>,
}

impl std::fmt::Display for StartupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for StartupError {}

/// Prove the route ledger can be the only authority before anything reads it.
///
/// Every author's legacy provider key is turned into a canonical route row, and
/// then the result is checked. If a single author would be invisible to the
/// route-only consumers — a missing route, a value that is not a canonical
/// provider key, or an author with no progress row — startup stops here. A
/// half-finished cutover would make migrated authors silently lose their
/// bibliography, series, and monitoring (FP-042).
///
/// Ingestion is idempotent, so a restart re-runs it harmlessly.
pub async fn verify_author_link_cutover_before_serving(
    db: &SqliteDb,
) -> Result<AuthorRouteBackfillReport, StartupError> {
    let ingested = db.ingest_legacy_routes().await.map_err(|e| StartupError {
        message: format!("author-link legacy route ingestion failed: {e}"),
        report: None,
    })?;
    tracing::info!(
        legacy_values = ingested.legacy_values,
        canonical_routes = ingested.canonical_routes,
        invalid_values = ingested.invalid_values,
        "author-link cutover: legacy routes ingested"
    );

    let report = db.verify_cutover_ready().await.map_err(|e| StartupError {
        message: format!("author-link cutover verification failed: {e}"),
        report: None,
    })?;

    if report.missing_routes > 0 || report.invalid_values > 0 || report.missing_progress_rows > 0 {
        return Err(StartupError {
            message: format!(
                "author-link cutover incomplete: {} missing routes, {} invalid values, \
                 {} authors without a link task",
                report.missing_routes, report.invalid_values, report.missing_progress_rows
            ),
            report: Some(report),
        });
    }

    tracing::info!(
        legacy_values = report.legacy_values,
        canonical_routes = report.canonical_routes,
        "author-link cutover verified: route consumers have complete canonical backing"
    );
    Ok(report)
}

/// One bounded pass of the author-link sweep.
///
/// It claims a batch of due authors and hands each to the road. Cancellation is
/// honoured before claiming, so a shutdown stops taking new work and lets the
/// leases it already holds expire — nothing committed is replayed or rolled back.
pub async fn author_link_sweep_tick(
    state: AppState,
    cancel: CancellationToken,
) -> Result<(), String> {
    if !state.config.author_link.enabled {
        tracing::debug!("author-link sweep disabled by configuration");
        return Ok(());
    }
    if cancel.is_cancelled() {
        return Ok(());
    }

    let batch_size =
        u32::try_from(state.config.author_link.batch_size).unwrap_or(DEFAULT_SWEEP_BATCH);
    match state.run_due(batch_size, cancel).await {
        Ok(summary) => {
            if summary.claimed > 0 {
                tracing::info!(
                    claimed = summary.claimed,
                    evaluated = summary.evaluated,
                    unchanged_fingerprint = summary.unchanged_fingerprint,
                    failed = summary.failed,
                    "author-link sweep tick complete"
                );
            }
            Ok(())
        }
        Err(e) => Err(format!("author-link sweep tick failed: {e:?}")),
    }
}

/// What one Readarr author record proves about its own Goodreads id.
///
/// Readarr hands us an author name and a Goodreads author id on the same record.
/// That pairing is evidence — the id and the name came from one source that
/// claims they belong together — so it goes through the same name guard every
/// other automatic route write goes through. It is *not* a user's pick, and
/// Readarr is never allowed to prove itself: the names it is compared against are
/// snapshotted before its own observation is recorded, or the comparison would
/// eventually be the Readarr name against itself.
///
/// `None` means there is nothing to judge: no id, an id that is not a canonical
/// Goodreads author id, or no usable name. The caller still keeps the author
/// enqueued in every one of those cases.
pub fn readarr_author_route_evidence(
    rd_author: &RdAuthor,
    pre_observation_associated_names: &[String],
) -> Option<AuthorRouteGuardResult> {
    let raw_id = rd_author.foreign_author_id.as_deref().map(str::trim)?;
    if raw_id.is_empty() {
        return None;
    }
    let key = match AuthorRouteKey::parse(AuthorProvider::Goodreads, raw_id) {
        Ok(key) => key,
        Err(e) => {
            tracing::warn!(
                readarr_author_id = rd_author.id,
                %raw_id,
                ?e,
                "readarr import: foreignAuthorId is not a canonical Goodreads author id"
            );
            return None;
        }
    };
    let name = rd_author.author_name.as_deref().map(str::trim)?;
    if name.is_empty() {
        return None;
    }
    Some(guard_author_route(
        pre_observation_associated_names,
        ProviderAuthorRef {
            key,
            name: name.to_string(),
            // A Readarr *author* record asserting its own Goodreads author id
            // is an author claim by what the record is — an assertion, not a
            // placement. That is this door's own shape knowledge, certified here
            // exactly as each provider gateway certifies its own.
            credit: ProviderCredit::AssertedAuthor,
        },
        None,
        AuthorRouteEvidenceSource::ReadarrImport,
    ))
}
