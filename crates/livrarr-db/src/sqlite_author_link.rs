//! Author-provider route, tombstone, candidate, name-variant, claim, and
//! key-attempt storage.
//!
//! Every author-route statement in the codebase lives here or in migrations
//! 078/079. Two rules shape the whole module:
//!
//! * **Claim discipline.** A worker mutation carries the claim token it was
//!   handed and only lands while that token still owns a live lease. Migration
//!   078's trigger clears the token whenever a work's identity or provider keys
//!   change, so a worker holding a pre-change snapshot loses authority before it
//!   can write; every token-checked entry point returns [`DbError::ClaimLost`].
//! * **User sovereignty.** A route removal is a durable tombstone. Only the
//!   private [`attach_route_as_user_tx`] — reachable from the two public user
//!   entry shapes and nothing else — may reactivate one.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use livrarr_domain::identity_matching::{canonical_author_key, AuthorVerdict};
use livrarr_domain::{
    AuthorCandidateAlternateNameEvidence, AuthorCompatibilityProjection, AuthorEvidenceFingerprint,
    AuthorId, AuthorKeyAttempt, AuthorKeyAttemptOutcome, AuthorKeyAttemptState,
    AuthorLinkCandidate, AuthorLinkCandidateReason, AuthorLinkProgress, AuthorLinkProgressUpdate,
    AuthorLinkReview, AuthorLinkState, AuthorLinkTrigger, AuthorNameSource, AuthorNameVariant,
    AuthorProvider, AuthorRoadInput, AuthorRoute, AuthorRouteEvidenceSource, AuthorRouteKey,
    AuthorRouteProvenance, AuthorRouteState, AuthorSweepProgress, OpenLibraryNameRole,
    ProviderAuthorNameObservation, RejectedAuthorRouteEvidence, RouteWriteOutcome,
    SettledAuthorWork, SettledWorkProviderKey, UserId, WorkId,
};
use sqlx::{Row, SqliteConnection};
use uuid::Uuid;

use crate::sqlite::SqliteDb;
use crate::sqlite_author::row_to_author;
use crate::sqlite_author_link_codec::*;
use crate::sqlite_common::{map_db_err, parse_dt};
use crate::sqlite_work::parse_identity_status;
use crate::{
    AuthorLinkClaim, AuthorLinkDb, AuthorNameVariantDb, AuthorRouteBackfillReport, DbError,
    GuardedRouteWrite,
};

/// One staging batch per transaction — legacy ingestion is a startup pass, not
/// a single library-wide transaction.
const LEGACY_BATCH: i64 = 500;

// ---------------------------------------------------------------------------
// Row readers
// ---------------------------------------------------------------------------

const ROUTE_COLUMNS: &str = "id, user_id, author_id, provider, route_value, state, provenance, \
                             evidence_work_id, created_at, verified_at, removed_at";

fn row_to_route(row: &sqlx::sqlite::SqliteRow) -> Result<AuthorRoute, DbError> {
    let provider: String = row.try_get("provider").map_err(map_db_err)?;
    let value: String = row.try_get("route_value").map_err(map_db_err)?;
    let state: String = row.try_get("state").map_err(map_db_err)?;
    let provenance: String = row.try_get("provenance").map_err(map_db_err)?;
    let created_at: String = row.try_get("created_at").map_err(map_db_err)?;
    let verified_at: Option<String> = row.try_get("verified_at").map_err(map_db_err)?;
    let removed_at: Option<String> = row.try_get("removed_at").map_err(map_db_err)?;
    Ok(AuthorRoute {
        id: row.try_get("id").map_err(map_db_err)?,
        user_id: row.try_get("user_id").map_err(map_db_err)?,
        author_id: row.try_get("author_id").map_err(map_db_err)?,
        key: parse_route_key(&provider, &value)?,
        state: parse_route_state(&state)?,
        provenance: parse_provenance(&provenance)?,
        evidence_work_id: row.try_get("evidence_work_id").map_err(map_db_err)?,
        created_at: parse_dt(&created_at)?,
        verified_at: verified_at.as_deref().map(parse_dt).transpose()?,
        removed_at: removed_at.as_deref().map(parse_dt).transpose()?,
    })
}

const CANDIDATE_COLUMNS: &str =
    "id, author_id, provider, route_value, candidate_name, reason, name_verdict, \
     primary_name_verdict, top_work_preview, catalog_evidence_state, corroborated_title_count, \
     settled_work_count, previously_removed, status, evidence_generation, observed_at";

/// Scalar half of a candidate; `alternate_name_evidence` is hydrated from the
/// child table by the caller that needs it.
fn row_to_candidate(row: &sqlx::sqlite::SqliteRow) -> Result<AuthorLinkCandidate, DbError> {
    let provider: String = row.try_get("provider").map_err(map_db_err)?;
    let value: String = row.try_get("route_value").map_err(map_db_err)?;
    let reason: String = row.try_get("reason").map_err(map_db_err)?;
    let name_verdict: String = row.try_get("name_verdict").map_err(map_db_err)?;
    let primary_name_verdict: String = row.try_get("primary_name_verdict").map_err(map_db_err)?;
    let catalog: String = row.try_get("catalog_evidence_state").map_err(map_db_err)?;
    let status: String = row.try_get("status").map_err(map_db_err)?;
    let observed_at: String = row.try_get("observed_at").map_err(map_db_err)?;
    Ok(AuthorLinkCandidate {
        id: row.try_get("id").map_err(map_db_err)?,
        author_id: row.try_get("author_id").map_err(map_db_err)?,
        key: parse_route_key(&provider, &value)?,
        candidate_name: row.try_get("candidate_name").map_err(map_db_err)?,
        reason: parse_candidate_reason(&reason)?,
        name_verdict: parse_verdict(&name_verdict)?,
        primary_name_verdict: parse_verdict(&primary_name_verdict)?,
        alternate_name_evidence: Vec::new(),
        top_work_preview: row.try_get("top_work_preview").map_err(map_db_err)?,
        catalog_evidence_state: parse_catalog_state(&catalog)?,
        corroborated_title_count: row
            .try_get::<i64, _>("corroborated_title_count")
            .map_err(map_db_err)?
            .max(0) as u32,
        settled_work_count: row
            .try_get::<i64, _>("settled_work_count")
            .map_err(map_db_err)?
            .max(0) as u32,
        previously_removed: row
            .try_get::<bool, _>("previously_removed")
            .map_err(map_db_err)?,
        status: parse_candidate_status(&status)?,
        evidence_generation: row.try_get("evidence_generation").map_err(map_db_err)?,
        observed_at: parse_dt(&observed_at)?,
    })
}

const ATTEMPT_COLUMNS: &str = "id, user_id, author_id, evidence_generation, work_id, provider, \
                               work_route, state, claim_token, attempt_count, next_attempt_at, \
                               last_error, updated_at";

fn row_to_attempt(row: &sqlx::sqlite::SqliteRow) -> Result<AuthorKeyAttempt, DbError> {
    let provider: String = row.try_get("provider").map_err(map_db_err)?;
    let state: String = row.try_get("state").map_err(map_db_err)?;
    let claim_token: Option<String> = row.try_get("claim_token").map_err(map_db_err)?;
    let next_attempt_at: Option<String> = row.try_get("next_attempt_at").map_err(map_db_err)?;
    let updated_at: String = row.try_get("updated_at").map_err(map_db_err)?;
    Ok(AuthorKeyAttempt {
        id: row.try_get("id").map_err(map_db_err)?,
        user_id: row.try_get("user_id").map_err(map_db_err)?,
        author_id: row.try_get("author_id").map_err(map_db_err)?,
        evidence_generation: row.try_get("evidence_generation").map_err(map_db_err)?,
        work_id: row.try_get("work_id").map_err(map_db_err)?,
        provider: parse_provider(&provider)?,
        work_route: row.try_get("work_route").map_err(map_db_err)?,
        state: parse_attempt_state(&state)?,
        claim_token: claim_token.and_then(|token| token.parse().ok()),
        attempt_count: row
            .try_get::<i64, _>("attempt_count")
            .map_err(map_db_err)?
            .max(0) as u32,
        next_attempt_at: next_attempt_at.as_deref().map(parse_dt).transpose()?,
        last_error: row.try_get("last_error").map_err(map_db_err)?,
        updated_at: parse_dt(&updated_at)?,
    })
}

fn row_to_name_variant(row: &sqlx::sqlite::SqliteRow) -> Result<AuthorNameVariant, DbError> {
    let source: String = row.try_get("source").map_err(map_db_err)?;
    let role: Option<String> = row.try_get("open_library_role").map_err(map_db_err)?;
    let user_selected_at: Option<String> = row.try_get("user_selected_at").map_err(map_db_err)?;
    let observed_at: String = row.try_get("observed_at").map_err(map_db_err)?;
    Ok(AuthorNameVariant {
        id: row.try_get("id").map_err(map_db_err)?,
        user_id: row.try_get("user_id").map_err(map_db_err)?,
        author_id: row.try_get("author_id").map_err(map_db_err)?,
        name: row.try_get("name").map_err(map_db_err)?,
        source: parse_name_source(&source)?,
        source_route_id: row.try_get("source_route_id").map_err(map_db_err)?,
        open_library_role: parse_ol_role(role.as_deref())?,
        user_selected_at: user_selected_at.as_deref().map(parse_dt).transpose()?,
        observed_at: parse_dt(&observed_at)?,
    })
}

// ---------------------------------------------------------------------------
// Transaction-internal helpers
// ---------------------------------------------------------------------------

/// The owning user of an author row, or `NotFound`.
async fn author_owner_tx(
    conn: &mut SqliteConnection,
    author_id: AuthorId,
) -> Result<UserId, DbError> {
    sqlx::query_scalar("SELECT user_id FROM authors WHERE id = ?")
        .bind(author_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(map_db_err)?
        .ok_or(DbError::NotFound { entity: "author" })
}

async fn require_author_owned_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    author_id: AuthorId,
) -> Result<(), DbError> {
    let owner = author_owner_tx(conn, author_id).await?;
    if owner != user_id {
        return Err(DbError::NotFound { entity: "author" });
    }
    Ok(())
}

/// A worker mutation is only allowed while the token it was handed still owns a
/// live lease on the author's progress row.
async fn verify_claim_tx(
    conn: &mut SqliteConnection,
    claim: &AuthorLinkClaim,
) -> Result<(), DbError> {
    let held: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM author_link_progress \
          WHERE author_id = ? AND user_id = ? AND claim_token = ? \
            AND lease_until IS NOT NULL AND lease_until > ?",
    )
    .bind(claim.author_id)
    .bind(claim.user_id)
    .bind(claim.claim_token.to_string())
    .bind(now_ts())
    .fetch_optional(&mut *conn)
    .await
    .map_err(map_db_err)?;
    held.map(|_| ()).ok_or(DbError::ClaimLost)
}

/// The evidence generation the author's persisted state is currently scoped to.
/// An author with no progress row has never been evaluated: generation zero.
async fn current_generation_tx(
    conn: &mut SqliteConnection,
    author_id: AuthorId,
) -> Result<i64, DbError> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT evidence_generation FROM author_link_progress WHERE author_id = ?",
    )
    .bind(author_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(map_db_err)?
    .unwrap_or(0))
}

/// Insert a progress row when absent, otherwise pull the existing row forward
/// under `trigger` without replacing it.
///
/// An automatic trigger leaves a live lease alone — the worker holding it is
/// evaluating the same question and finishing is better than restarting. A user
/// re-resolve is the exception: the user just changed the state the worker is
/// deciding from, so its claim is voided exactly as migration 078 voids it when
/// work evidence changes. The stale worker then loses its claim rather than
/// writing a route the user's action already answered.
async fn ensure_progress_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    author_id: AuthorId,
    trigger: AuthorLinkTrigger,
) -> Result<(), DbError> {
    let now = now_ts();
    let user_directed = matches!(trigger, AuthorLinkTrigger::UserReResolve);
    sqlx::query(
        "INSERT INTO author_link_progress \
             (author_id, user_id, state, next_attempt_at, trigger, updated_at) \
         VALUES (?, ?, 'queued', ?, ?, ?) \
         ON CONFLICT(author_id) DO UPDATE SET \
             state = CASE \
                 WHEN ? THEN 'queued' \
                 WHEN author_link_progress.lease_until IS NOT NULL \
                  AND author_link_progress.lease_until > excluded.updated_at \
                 THEN author_link_progress.state ELSE 'queued' END, \
             trigger = excluded.trigger, \
             next_attempt_at = MIN(author_link_progress.next_attempt_at, excluded.next_attempt_at), \
             claim_token = CASE WHEN ? THEN NULL ELSE author_link_progress.claim_token END, \
             lease_until = CASE WHEN ? THEN NULL ELSE author_link_progress.lease_until END, \
             updated_at = excluded.updated_at",
    )
    .bind(author_id)
    .bind(user_id)
    .bind(&now)
    .bind(trigger_str(trigger))
    .bind(&now)
    .bind(user_directed)
    .bind(user_directed)
    .bind(user_directed)
    .execute(&mut *conn)
    .await
    .map_err(map_db_err)?;
    Ok(())
}

/// Record a set of observed names against one author and wake its display work.
///
/// The one body behind both public observation writes: the work-scoped form used
/// by enrichment and the author-scoped form used by an import that has the author
/// in hand. Both must leave a live worker lease alone and both must make the
/// author's display work immediately due, so there is one implementation of that
/// rule rather than two that can drift (FP-050).
async fn record_observed_names_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    author_id: AuthorId,
    observations: &[ProviderAuthorNameObservation],
) -> Result<u32, DbError> {
    let mut inserted = 0u32;
    for observation in observations {
        let landed = insert_name_variant_tx(
            conn,
            user_id,
            author_id,
            observation.source,
            &observation.name,
            None,
            None,
        )
        .await?;
        inserted += u32::from(landed);
    }

    if inserted > 0 {
        // One generation bump for the whole transaction, the author is due
        // now, and a live lease is left alone — the worker holding it will
        // fail its compare-and-set and the author stays dirty.
        sqlx::query(
            "UPDATE author_link_progress \
                SET display_name_generation = display_name_generation + 1, \
                    display_name_dirty = 1, \
                    next_attempt_at = MIN(next_attempt_at, ?), \
                    updated_at = ? \
              WHERE author_id = ?",
        )
        .bind(now_ts())
        .bind(now_ts())
        .bind(author_id)
        .execute(&mut *conn)
        .await
        .map_err(map_db_err)?;
    }

    Ok(inserted)
}

/// Record an observed name against an author without replacing an existing row.
/// Returns `true` only when a new distinct variant landed.
#[allow(clippy::too_many_arguments)]
async fn insert_name_variant_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    author_id: AuthorId,
    source: AuthorNameSource,
    name: &str,
    source_route_id: Option<i64>,
    role: Option<OpenLibraryNameRole>,
) -> Result<bool, DbError> {
    let name = name.trim();
    let canonical = canonical_author_key(name);
    if name.is_empty() || canonical.is_empty() {
        return Ok(false);
    }
    let inserted = sqlx::query(
        "INSERT INTO author_name_variants \
             (user_id, author_id, name, canonical_name, source, source_route_id, \
              open_library_role, observed_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(user_id, author_id, source, canonical_name) DO NOTHING",
    )
    .bind(user_id)
    .bind(author_id)
    .bind(name)
    .bind(&canonical)
    .bind(name_source_str(source))
    .bind(source_route_id)
    .bind(role.map(ol_role_str))
    .bind(now_ts())
    .execute(&mut *conn)
    .await
    .map_err(map_db_err)?;
    Ok(inserted.rows_affected() == 1)
}

/// Same as [`insert_name_variant_tx`], but an existing row's OpenLibrary role is
/// promoted (Primary over Alias over none) rather than left as it was. Used
/// where the writer genuinely establishes the OL search role — the user pick.
async fn upsert_name_variant_role_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    author_id: AuthorId,
    source: AuthorNameSource,
    name: &str,
    role: Option<OpenLibraryNameRole>,
) -> Result<(), DbError> {
    let name = name.trim();
    let canonical = canonical_author_key(name);
    if name.is_empty() || canonical.is_empty() {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO author_name_variants \
             (user_id, author_id, name, canonical_name, source, open_library_role, observed_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(user_id, author_id, source, canonical_name) DO UPDATE SET \
             open_library_role = CASE \
                 WHEN excluded.open_library_role = 'primary' THEN 'primary' \
                 WHEN author_name_variants.open_library_role IS NULL \
                     THEN excluded.open_library_role \
                 ELSE author_name_variants.open_library_role END",
    )
    .bind(user_id)
    .bind(author_id)
    .bind(name)
    .bind(&canonical)
    .bind(name_source_str(source))
    .bind(role.map(ol_role_str))
    .bind(now_ts())
    .execute(&mut *conn)
    .await
    .map_err(map_db_err)?;
    Ok(())
}

/// The fields a candidate carries when its reason is not a Tier-2 catalog read.
/// They are truthful not-run values, never a claim that a catalog read returned
/// nothing: Review omits catalog presentation entirely for these reasons.
struct NonTier2Candidate<'a> {
    author_id: AuthorId,
    key: &'a AuthorRouteKey,
    candidate_name: &'a str,
    reason: AuthorLinkCandidateReason,
    verdict: AuthorVerdict,
    previously_removed: bool,
}

async fn upsert_non_tier2_candidate_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    generation: i64,
    candidate: NonTier2Candidate<'_>,
) -> Result<AuthorLinkCandidate, DbError> {
    let provider = provider_str(candidate.key.provider());
    let value = candidate.key.value();
    let verdict = verdict_str(candidate.verdict);
    let reason = candidate_reason_str(candidate.reason);
    sqlx::query(
        "INSERT INTO author_link_candidates \
             (user_id, author_id, provider, route_value, candidate_name, reason, name_verdict, \
              primary_name_verdict, top_work_preview, catalog_evidence_state, \
              corroborated_title_count, settled_work_count, previously_removed, status, \
              evidence_generation, observed_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, 'pending', 0, 0, ?, 'pending', ?, ?) \
         ON CONFLICT(user_id, author_id, provider, route_value, reason, evidence_generation) \
         DO UPDATE SET \
             candidate_name = excluded.candidate_name, \
             name_verdict = excluded.name_verdict, \
             primary_name_verdict = excluded.primary_name_verdict, \
             previously_removed = excluded.previously_removed, \
             observed_at = excluded.observed_at",
    )
    .bind(user_id)
    .bind(candidate.author_id)
    .bind(provider)
    .bind(&value)
    .bind(candidate.candidate_name)
    .bind(reason)
    .bind(verdict)
    .bind(verdict)
    .bind(candidate.previously_removed)
    .bind(generation)
    .bind(now_ts())
    .execute(&mut *conn)
    .await
    .map_err(map_db_err)?;

    let row = sqlx::query(&format!(
        "SELECT {CANDIDATE_COLUMNS} FROM author_link_candidates \
          WHERE user_id = ? AND author_id = ? AND provider = ? AND route_value = ? \
            AND reason = ? AND evidence_generation = ?"
    ))
    .bind(user_id)
    .bind(candidate.author_id)
    .bind(provider)
    .bind(&value)
    .bind(reason)
    .bind(generation)
    .fetch_one(&mut *conn)
    .await
    .map_err(map_db_err)?;
    row_to_candidate(&row)
}

async fn load_route_by_id_tx(
    conn: &mut SqliteConnection,
    route_id: i64,
) -> Result<AuthorRoute, DbError> {
    let row = sqlx::query(&format!(
        "SELECT {ROUTE_COLUMNS} FROM author_provider_routes WHERE id = ?"
    ))
    .bind(route_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(map_db_err)?;
    row_to_route(&row)
}

/// Any route row for the user's canonical `(provider, value)` tuple — active or
/// tombstoned, this author's or another's. Route uniqueness is user-wide, so at
/// most one row can exist.
async fn find_route_for_tuple_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    key: &AuthorRouteKey,
) -> Result<Option<AuthorRoute>, DbError> {
    let row = sqlx::query(&format!(
        "SELECT {ROUTE_COLUMNS} FROM author_provider_routes \
          WHERE user_id = ? AND provider = ? AND route_value = ?"
    ))
    .bind(user_id)
    .bind(provider_str(key.provider()))
    .bind(key.value())
    .fetch_optional(&mut *conn)
    .await
    .map_err(map_db_err)?;
    row.as_ref().map(row_to_route).transpose()
}

/// **The sole user-attach transaction body.** Reachable only from
/// `attach_route_as_user` and `pick_candidate_as_user`, which is what makes
/// "only an explicit user selection clears a tombstone" mechanical rather than
/// a convention every future caller has to remember.
async fn attach_route_as_user_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    author_id: AuthorId,
    key: AuthorRouteKey,
) -> Result<AuthorRoute, DbError> {
    require_author_owned_tx(conn, user_id, author_id).await?;
    // Revalidate through the canonical parser: only the canonical tuple reaches
    // SQL, whatever alias the caller's value started life as.
    let key =
        AuthorRouteKey::parse(key.provider(), &key.value()).map_err(|_| DbError::Constraint {
            message: "author route value is not canonical".to_string(),
        })?;
    let now = now_ts();

    match find_route_for_tuple_tx(conn, user_id, &key).await? {
        Some(existing) if existing.author_id != author_id => Err(DbError::Constraint {
            message: format!(
                "author route is already held by author {}",
                existing.author_id
            ),
        }),
        Some(existing) => {
            sqlx::query(
                "UPDATE author_provider_routes \
                    SET state = 'active', provenance = 'user_picked', verified_at = ?, \
                        removed_at = NULL, removed_by_user_id = NULL \
                  WHERE id = ?",
            )
            .bind(&now)
            .bind(existing.id)
            .execute(&mut *conn)
            .await
            .map_err(map_db_err)?;
            load_route_by_id_tx(conn, existing.id).await
        }
        None => {
            let inserted = sqlx::query(
                "INSERT INTO author_provider_routes \
                     (user_id, author_id, provider, route_value, state, provenance, created_at, \
                      verified_at) \
                 VALUES (?, ?, ?, ?, 'active', 'user_picked', ?, ?)",
            )
            .bind(user_id)
            .bind(author_id)
            .bind(provider_str(key.provider()))
            .bind(key.value())
            .bind(&now)
            .bind(&now)
            .execute(&mut *conn)
            .await
            .map_err(map_db_err)?;
            load_route_by_id_tx(conn, inserted.last_insert_rowid()).await
        }
    }
}

/// Recompute the author's link state from what is actually persisted. A missing
/// progress row is left missing — repairing that invariant is
/// `ensure_enqueued`'s job, not a side effect of a route write.
async fn rederive_progress_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    author_id: AuthorId,
) -> Result<(), DbError> {
    let generation = match sqlx::query_scalar::<_, i64>(
        "SELECT evidence_generation FROM author_link_progress WHERE author_id = ?",
    )
    .bind(author_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(map_db_err)?
    {
        Some(generation) => generation,
        None => return Ok(()),
    };

    let pending: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM author_link_candidates \
          WHERE user_id = ? AND author_id = ? AND status = 'pending' AND evidence_generation = ?",
    )
    .bind(user_id)
    .bind(author_id)
    .bind(generation)
    .fetch_one(&mut *conn)
    .await
    .map_err(map_db_err)?;
    let active: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM author_provider_routes \
          WHERE user_id = ? AND author_id = ? AND state = 'active'",
    )
    .bind(user_id)
    .bind(author_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(map_db_err)?;

    let state = if pending > 0 {
        "needs_review"
    } else if active > 0 {
        "linked"
    } else {
        return Ok(());
    };
    sqlx::query("UPDATE author_link_progress SET state = ?, updated_at = ? WHERE author_id = ?")
        .bind(state)
        .bind(now_ts())
        .bind(author_id)
        .execute(&mut *conn)
        .await
        .map_err(map_db_err)?;
    Ok(())
}

/// Ordered alias-verdict evidence for a set of candidates, keyed by candidate id.
async fn load_alternate_evidence_tx(
    conn: &mut SqliteConnection,
    candidate_ids: &[i64],
) -> Result<HashMap<i64, Vec<AuthorCandidateAlternateNameEvidence>>, DbError> {
    let mut hydrated: HashMap<i64, Vec<AuthorCandidateAlternateNameEvidence>> = HashMap::new();
    for candidate_id in candidate_ids {
        let rows = sqlx::query(
            "SELECT name, verdict FROM author_link_candidate_alternate_name_evidence \
              WHERE candidate_id = ? ORDER BY ordinal",
        )
        .bind(candidate_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(map_db_err)?;
        let mut evidence = Vec::with_capacity(rows.len());
        for row in &rows {
            let verdict: String = row.try_get("verdict").map_err(map_db_err)?;
            evidence.push(AuthorCandidateAlternateNameEvidence {
                name: row.try_get("name").map_err(map_db_err)?,
                verdict: parse_verdict(&verdict)?,
            });
        }
        hydrated.insert(*candidate_id, evidence);
    }
    Ok(hydrated)
}

// ---------------------------------------------------------------------------
// Evidence fingerprint
// ---------------------------------------------------------------------------

const FNV_OFFSET_128: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
const FNV_PRIME_128: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;

fn fnv1a_128(bytes: &[u8]) -> u128 {
    let mut hash = FNV_OFFSET_128;
    for byte in bytes {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME_128);
    }
    hash
}

/// Length-prefixed so no field boundary is ambiguous: `"ab" + "c"` and
/// `"a" + "bc"` must not hash alike.
fn push_field(buffer: &mut Vec<u8>, field: &str) {
    buffer.extend_from_slice(&(field.len() as u64).to_be_bytes());
    buffer.extend_from_slice(field.as_bytes());
}

/// One settled work's contribution to its author's evidence fingerprint.
struct SettledTuple {
    work_id: WorkId,
    identity_status: String,
    provider_keys: Vec<(AuthorProvider, String)>,
}

/// The settled-evidence tuples an author's fingerprint is computed over, in the
/// canonical sorted order.
struct SettledTuples {
    works: Vec<SettledTuple>,
    provider_key_count: u32,
}

fn provider_tag(provider: AuthorProvider) -> &'static str {
    provider_str(provider)
}

fn fingerprint_of(tuples: &SettledTuples) -> AuthorEvidenceFingerprint {
    let mut buffer = Vec::new();
    for work in &tuples.works {
        push_field(&mut buffer, &work.work_id.to_string());
        push_field(&mut buffer, &work.identity_status);
        for (provider, key) in &work.provider_keys {
            push_field(&mut buffer, provider_tag(*provider));
            push_field(&mut buffer, key);
        }
    }
    AuthorEvidenceFingerprint {
        settled_work_count: tuples.works.len() as u32,
        settled_provider_key_count: tuples.provider_key_count,
        content_hash: format!("{:032x}", fnv1a_128(&buffer)),
    }
}

/// Confirmed and Provisional works only — a Pending, NeedsReview, Conflict, or
/// NotFound work has no settled identity to inherit an author route from.
const SETTLED_WORK_SQL: &str = "SELECT id, title, identity_status, ol_key, gr_key, hc_key \
                                  FROM works \
                                 WHERE user_id = ? AND author_id = ? \
                                   AND identity_status IN ('confirmed', 'provisional') \
                                 ORDER BY id";

async fn load_settled_tuples_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    author_id: AuthorId,
) -> Result<SettledTuples, DbError> {
    let rows = sqlx::query(SETTLED_WORK_SQL)
        .bind(user_id)
        .bind(author_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(map_db_err)?;

    let mut works = Vec::with_capacity(rows.len());
    let mut provider_key_count = 0u32;
    for row in &rows {
        let mut provider_keys = Vec::new();
        for (provider, column) in [
            (AuthorProvider::OpenLibrary, "ol_key"),
            (AuthorProvider::Goodreads, "gr_key"),
            (AuthorProvider::Hardcover, "hc_key"),
        ] {
            let raw: Option<String> = row.try_get(column).map_err(map_db_err)?;
            if let Some(value) = raw.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                provider_keys.push((provider, value.to_string()));
                provider_key_count += 1;
            }
        }
        provider_keys.sort_by(|a, b| provider_tag(a.0).cmp(provider_tag(b.0)).then(a.1.cmp(&b.1)));
        works.push(SettledTuple {
            work_id: row.try_get("id").map_err(map_db_err)?,
            identity_status: row.try_get("identity_status").map_err(map_db_err)?,
            provider_keys,
        });
    }
    works.sort_by(|a, b| {
        a.work_id
            .cmp(&b.work_id)
            .then(a.identity_status.cmp(&b.identity_status))
    });
    Ok(SettledTuples {
        works,
        provider_key_count,
    })
}

async fn load_settled_works_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    author_id: AuthorId,
) -> Result<Vec<SettledAuthorWork>, DbError> {
    let rows = sqlx::query(SETTLED_WORK_SQL)
        .bind(user_id)
        .bind(author_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(map_db_err)?;
    let mut works = Vec::with_capacity(rows.len());
    for row in &rows {
        let status: String = row.try_get("identity_status").map_err(map_db_err)?;
        works.push(SettledAuthorWork {
            work_id: row.try_get("id").map_err(map_db_err)?,
            title: row.try_get("title").map_err(map_db_err)?,
            identity_status: parse_identity_status(&status)?,
            ol_key: row.try_get("ol_key").map_err(map_db_err)?,
            gr_key: row.try_get("gr_key").map_err(map_db_err)?,
            hc_key: row.try_get("hc_key").map_err(map_db_err)?,
        });
    }
    Ok(works)
}

// ---------------------------------------------------------------------------
// Legacy cutover reporting
// ---------------------------------------------------------------------------

/// One honest census of the legacy-to-route cutover, computed the same way for
/// ingestion and for the startup verification so the two can never disagree.
///
/// A route row counts whatever its current state is: the cutover question is
/// "did this legacy value become a canonical route", and a user who later
/// removes that route must not brick the next startup.
async fn backfill_report(db: &SqliteDb) -> Result<AuthorRouteBackfillReport, DbError> {
    let rows = sqlx::query(
        "SELECT id, user_id, ol_key, gr_key, hc_key FROM authors \
          WHERE (ol_key IS NOT NULL AND TRIM(ol_key) <> '') \
             OR (gr_key IS NOT NULL AND TRIM(gr_key) <> '') \
             OR (hc_key IS NOT NULL AND TRIM(hc_key) <> '')",
    )
    .fetch_all(db.pool())
    .await
    .map_err(map_db_err)?;

    let route_rows: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT user_id, provider, route_value FROM author_provider_routes")
            .fetch_all(db.pool())
            .await
            .map_err(map_db_err)?;
    let routes: HashSet<(i64, String, String)> = route_rows.into_iter().collect();

    let mut legacy_values = 0u64;
    let mut canonical_routes = 0u64;
    let mut missing_routes = 0u64;
    let mut invalid_values = 0u64;
    for row in &rows {
        let user_id: UserId = row.try_get("user_id").map_err(map_db_err)?;
        for (provider, column) in [
            (AuthorProvider::OpenLibrary, "ol_key"),
            (AuthorProvider::Goodreads, "gr_key"),
            (AuthorProvider::Hardcover, "hc_key"),
        ] {
            let raw: Option<String> = row.try_get(column).map_err(map_db_err)?;
            let Some(raw) = raw.as_deref().map(str::trim).filter(|v| !v.is_empty()) else {
                continue;
            };
            legacy_values += 1;
            match AuthorRouteKey::parse(provider, raw) {
                Ok(key) => {
                    let tuple = (user_id, provider_str(provider).to_string(), key.value());
                    if routes.contains(&tuple) {
                        canonical_routes += 1;
                    } else {
                        missing_routes += 1;
                    }
                }
                Err(_) => invalid_values += 1,
            }
        }
    }

    let missing_progress_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM authors a \
          WHERE NOT EXISTS (SELECT 1 FROM author_link_progress p WHERE p.author_id = a.id)",
    )
    .fetch_one(db.pool())
    .await
    .map_err(map_db_err)?;

    Ok(AuthorRouteBackfillReport {
        legacy_values,
        canonical_routes,
        missing_routes,
        invalid_values,
        missing_progress_rows: missing_progress_rows.max(0) as u64,
    })
}

// ---------------------------------------------------------------------------
// AuthorLinkDb
// ---------------------------------------------------------------------------

impl AuthorLinkDb for SqliteDb {
    async fn ensure_enqueued(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        trigger: AuthorLinkTrigger,
    ) -> Result<(), DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        require_author_owned_tx(&mut tx, user_id, author_id).await?;
        ensure_progress_tx(&mut tx, user_id, author_id, trigger).await?;
        tx.commit().await.map_err(map_db_err)?;
        Ok(())
    }

    async fn create_or_adopt_author(
        &self,
        request: crate::CreateAuthorGateRequest,
    ) -> Result<(livrarr_domain::Author, bool), DbError> {
        self.create_or_adopt_author_gate(request).await
    }

    async fn ensure_missing_progress_rows(&self, limit: u32) -> Result<u32, DbError> {
        let rows: Vec<(i64, i64, String)> = sqlx::query_as(
            "SELECT a.id, a.user_id, a.name FROM authors a \
              WHERE NOT EXISTS (SELECT 1 FROM author_link_progress p WHERE p.author_id = a.id) \
              ORDER BY a.id LIMIT ?",
        )
        .bind(limit)
        .fetch_all(self.pool())
        .await
        .map_err(map_db_err)?;

        let mut inserted = 0u32;
        for (author_id, user_id, name) in rows {
            let mut tx = self.pool().begin().await.map_err(map_db_err)?;
            let landed = sqlx::query(
                "INSERT INTO author_link_progress \
                     (author_id, user_id, state, next_attempt_at, trigger, updated_at) \
                 VALUES (?, ?, 'queued', ?, 'legacy_backfill', ?) \
                 ON CONFLICT(author_id) DO NOTHING",
            )
            .bind(author_id)
            .bind(user_id)
            .bind(now_ts())
            .bind(now_ts())
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
            insert_name_variant_tx(
                &mut tx,
                user_id,
                author_id,
                AuthorNameSource::Legacy,
                &name,
                None,
                None,
            )
            .await?;
            tx.commit().await.map_err(map_db_err)?;
            inserted += u32::from(landed.rows_affected() == 1);
        }
        Ok(inserted)
    }

    async fn claim_due(
        &self,
        now: DateTime<Utc>,
        lease_until: DateTime<Utc>,
        limit: u32,
    ) -> Result<Vec<AuthorLinkClaim>, DbError> {
        let now_str = ts(now);
        let lease_str = ts(lease_until);
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;

        // A lease-free or expired row qualifies when its due time has arrived
        // OR a name observation dirtied it — including from an otherwise
        // terminal state, which is how display convergence is never stranded.
        let due: Vec<(i64, i64, Option<String>, i64)> = sqlx::query_as(
            "SELECT author_id, user_id, cursor, display_name_generation \
               FROM author_link_progress \
              WHERE (lease_until IS NULL OR lease_until <= ?) \
                AND (next_attempt_at <= ? OR display_name_dirty = 1) \
              ORDER BY next_attempt_at, author_id LIMIT ?",
        )
        .bind(&now_str)
        .bind(&now_str)
        .bind(limit)
        .fetch_all(&mut *tx)
        .await
        .map_err(map_db_err)?;

        let mut claims = Vec::with_capacity(due.len());
        for (author_id, user_id, cursor, display_name_generation) in due {
            let claim_token = Uuid::new_v4();
            let updated = sqlx::query(
                "UPDATE author_link_progress \
                    SET claim_token = ?, lease_until = ?, state = 'running', updated_at = ? \
                  WHERE author_id = ? AND (lease_until IS NULL OR lease_until <= ?)",
            )
            .bind(claim_token.to_string())
            .bind(&lease_str)
            .bind(&now_str)
            .bind(author_id)
            .bind(&now_str)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
            if updated.rows_affected() != 1 {
                continue;
            }
            claims.push(AuthorLinkClaim {
                author_id,
                user_id,
                claim_token,
                lease_expires_at: lease_until,
                cursor: cursor.as_deref().and_then(cursor_from_string),
                display_name_generation,
            });
        }

        tx.commit().await.map_err(map_db_err)?;
        Ok(claims)
    }

    async fn load_road_input(&self, claim: AuthorLinkClaim) -> Result<AuthorRoadInput, DbError> {
        let mut conn = self.pool().acquire().await.map_err(map_db_err)?;
        verify_claim_tx(&mut conn, &claim).await?;

        let author_row = sqlx::query("SELECT * FROM authors WHERE id = ? AND user_id = ?")
            .bind(claim.author_id)
            .bind(claim.user_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(map_db_err)?;
        let author = row_to_author(author_row)?;

        let route_rows = sqlx::query(&format!(
            "SELECT {ROUTE_COLUMNS} FROM author_provider_routes \
              WHERE user_id = ? AND author_id = ? AND state = 'active'"
        ))
        .bind(claim.user_id)
        .bind(claim.author_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(map_db_err)?;
        let mut active_routes = route_rows
            .iter()
            .map(row_to_route)
            .collect::<Result<Vec<_>, _>>()?;
        active_routes.sort_by_key(|route| (provenance_rank(route.provenance), route.id));

        let variant_rows = sqlx::query(
            "SELECT * FROM author_name_variants WHERE user_id = ? AND author_id = ? ORDER BY id",
        )
        .bind(claim.user_id)
        .bind(claim.author_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(map_db_err)?;
        let name_variants = variant_rows
            .iter()
            .map(row_to_name_variant)
            .collect::<Result<Vec<_>, _>>()?;

        let progress: (Option<String>, i64, bool) = sqlx::query_as(
            "SELECT evaluated_fingerprint, display_name_generation, display_name_dirty \
               FROM author_link_progress WHERE author_id = ?",
        )
        .bind(claim.author_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(map_db_err)?;

        let settled_works =
            load_settled_works_tx(&mut conn, claim.user_id, claim.author_id).await?;
        let tuples = load_settled_tuples_tx(&mut conn, claim.user_id, claim.author_id).await?;

        Ok(AuthorRoadInput {
            author,
            active_routes,
            settled_works,
            name_variants,
            evaluated_fingerprint: progress.0.as_deref().and_then(fingerprint_from_string),
            live_fingerprint: fingerprint_of(&tuples),
            display_name_generation: progress.1,
            display_name_dirty: progress.2,
        })
    }

    async fn load_progress(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorLinkProgress, DbError> {
        let row = sqlx::query(
            "SELECT state, tier, cursor, evaluated_fingerprint, evidence_generation, \
                    display_name_generation, display_name_dirty, attempt_count, next_attempt_at, \
                    claim_token, lease_until, last_error, would_have_linked_at_090, updated_at \
               FROM author_link_progress WHERE author_id = ? AND user_id = ?",
        )
        .bind(author_id)
        .bind(user_id)
        .fetch_optional(self.pool())
        .await
        .map_err(map_db_err)?
        .ok_or(DbError::NotFound {
            entity: "author link progress",
        })?;

        let state: String = row.try_get("state").map_err(map_db_err)?;
        let cursor: Option<String> = row.try_get("cursor").map_err(map_db_err)?;
        let fingerprint: Option<String> =
            row.try_get("evaluated_fingerprint").map_err(map_db_err)?;
        let next_attempt_at: String = row.try_get("next_attempt_at").map_err(map_db_err)?;
        let claim_token: Option<String> = row.try_get("claim_token").map_err(map_db_err)?;
        let lease_until: Option<String> = row.try_get("lease_until").map_err(map_db_err)?;
        let updated_at: String = row.try_get("updated_at").map_err(map_db_err)?;
        Ok(AuthorLinkProgress {
            author_id,
            user_id,
            state: parse_progress_state(&state)?,
            tier: row
                .try_get::<Option<i64>, _>("tier")
                .map_err(map_db_err)?
                .map(|tier| tier.clamp(0, i64::from(u8::MAX)) as u8),
            cursor: cursor.as_deref().and_then(cursor_from_string),
            evaluated_fingerprint: fingerprint.as_deref().and_then(fingerprint_from_string),
            evidence_generation: row.try_get("evidence_generation").map_err(map_db_err)?,
            display_name_generation: row.try_get("display_name_generation").map_err(map_db_err)?,
            display_name_dirty: row.try_get("display_name_dirty").map_err(map_db_err)?,
            attempt_count: row
                .try_get::<i64, _>("attempt_count")
                .map_err(map_db_err)?
                .max(0) as u32,
            next_attempt_at: parse_dt(&next_attempt_at)?,
            lease_token: claim_token.and_then(|token| token.parse().ok()),
            lease_expires_at: lease_until.as_deref().map(parse_dt).transpose()?,
            last_error: row.try_get("last_error").map_err(map_db_err)?,
            would_have_linked_at_090: row
                .try_get("would_have_linked_at_090")
                .map_err(map_db_err)?,
            updated_at: parse_dt(&updated_at)?,
        })
    }

    async fn begin_evidence_generation(
        &self,
        claim: AuthorLinkClaim,
        evidence_generation: i64,
    ) -> Result<(), DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        verify_claim_tx(&mut tx, &claim).await?;

        // MAX, not assignment: a generation only ever moves forward, so a stale
        // worker cannot reopen a question a newer run already retired.
        sqlx::query(
            "UPDATE author_link_progress \
                SET evidence_generation = MAX(evidence_generation, ?), cursor = NULL, \
                    updated_at = ? \
              WHERE author_id = ? AND user_id = ?",
        )
        .bind(evidence_generation)
        .bind(now_ts())
        .bind(claim.author_id)
        .bind(claim.user_id)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;

        sqlx::query(
            "UPDATE author_link_candidates \
                SET status = 'superseded', resolved_at = ? \
              WHERE user_id = ? AND author_id = ? AND status = 'pending' \
                AND evidence_generation < ?",
        )
        .bind(now_ts())
        .bind(claim.user_id)
        .bind(claim.author_id)
        .bind(evidence_generation)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;

        tx.commit().await.map_err(map_db_err)?;
        Ok(())
    }

    async fn compute_evidence_fingerprint(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorEvidenceFingerprint, DbError> {
        let mut conn = self.pool().acquire().await.map_err(map_db_err)?;
        require_author_owned_tx(&mut conn, user_id, author_id).await?;
        let tuples = load_settled_tuples_tx(&mut conn, user_id, author_id).await?;
        Ok(fingerprint_of(&tuples))
    }

    async fn prepare_key_attempts(
        &self,
        claim: AuthorLinkClaim,
        evidence_generation: i64,
        keys: Vec<SettledWorkProviderKey>,
    ) -> Result<Vec<AuthorKeyAttempt>, DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        verify_claim_tx(&mut tx, &claim).await?;

        // Work routes are *work* keys (OL…W, GR book id, HC book id), not
        // author routes, so the canonical form here is the trimmed value; the
        // author-route parser would reject every one of them.
        let mut canonical: Vec<(WorkId, AuthorProvider, String)> = keys
            .iter()
            .filter_map(|key| {
                let route = key.work_route.trim();
                (!route.is_empty()).then(|| (key.work_id, key.provider, route.to_string()))
            })
            .collect();
        canonical.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(provider_tag(a.1).cmp(provider_tag(b.1)))
                .then(a.2.cmp(&b.2))
        });
        canonical.dedup();

        let now = now_ts();
        for (work_id, provider, work_route) in &canonical {
            sqlx::query(
                "INSERT INTO author_link_key_attempts \
                     (user_id, author_id, evidence_generation, work_id, provider, work_route, \
                      state, updated_at) \
                 VALUES (?, ?, ?, ?, ?, ?, 'pending', ?) \
                 ON CONFLICT(user_id, author_id, evidence_generation, work_id, provider, \
                             work_route) DO NOTHING",
            )
            .bind(claim.user_id)
            .bind(claim.author_id)
            .bind(evidence_generation)
            .bind(work_id)
            .bind(provider_str(*provider))
            .bind(work_route)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
        }

        // This claim owns the author's only live lease, so every Running row in
        // this generation is either already ours or was abandoned by a claim
        // that has since lost authority. Both are reclaimable; terminal rows and
        // not-yet-due retries are not.
        sqlx::query(
            "UPDATE author_link_key_attempts \
                SET state = 'running', claim_token = ?, updated_at = ? \
              WHERE user_id = ? AND author_id = ? AND evidence_generation = ? \
                AND (state = 'pending' OR state = 'running' \
                     OR (state = 'retryable' AND next_attempt_at IS NOT NULL \
                         AND next_attempt_at <= ?))",
        )
        .bind(claim.claim_token.to_string())
        .bind(&now)
        .bind(claim.user_id)
        .bind(claim.author_id)
        .bind(evidence_generation)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;

        let rows = sqlx::query(&format!(
            "SELECT {ATTEMPT_COLUMNS} FROM author_link_key_attempts \
              WHERE user_id = ? AND author_id = ? AND evidence_generation = ? \
                AND state = 'running' AND claim_token = ? \
              ORDER BY work_id, provider, work_route"
        ))
        .bind(claim.user_id)
        .bind(claim.author_id)
        .bind(evidence_generation)
        .bind(claim.claim_token.to_string())
        .fetch_all(&mut *tx)
        .await
        .map_err(map_db_err)?;
        let attempts = rows
            .iter()
            .map(row_to_attempt)
            .collect::<Result<Vec<_>, _>>()?;

        tx.commit().await.map_err(map_db_err)?;
        Ok(attempts)
    }

    async fn complete_key_attempt(
        &self,
        claim: AuthorLinkClaim,
        key_attempt_id: i64,
        outcome: AuthorKeyAttemptOutcome,
    ) -> Result<(), DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        verify_claim_tx(&mut tx, &claim).await?;

        // A permanent skip carries an operator-visible code because "we will
        // never call this key again" is a diagnosis, not a silent state.
        let (state, next_attempt_at, error, diagnostic) = match &outcome {
            AuthorKeyAttemptOutcome::Succeeded => {
                (AuthorKeyAttemptState::Succeeded, None, None, None)
            }
            AuthorKeyAttemptOutcome::Retryable {
                error,
                next_attempt_at,
            } => (
                AuthorKeyAttemptState::Retryable,
                Some(ts(*next_attempt_at)),
                Some(error.clone()),
                None,
            ),
            AuthorKeyAttemptOutcome::SkippedNotConfigured => (
                AuthorKeyAttemptState::SkippedNotConfigured,
                None,
                None,
                Some("not_configured"),
            ),
            AuthorKeyAttemptOutcome::SkippedPermanent { error } => (
                AuthorKeyAttemptState::SkippedPermanent,
                None,
                Some(error.clone()),
                Some("unsupported_provider"),
            ),
            AuthorKeyAttemptOutcome::ParkedLayoutDrift { error } => (
                AuthorKeyAttemptState::ParkedLayoutDrift,
                None,
                Some(error.clone()),
                Some("layout_drift"),
            ),
        };

        let updated = sqlx::query(
            "UPDATE author_link_key_attempts \
                SET state = ?, next_attempt_at = ?, last_error = ?, diagnostic_code = ?, \
                    attempt_count = attempt_count + 1, claim_token = NULL, updated_at = ? \
              WHERE id = ? AND user_id = ? AND author_id = ? AND state = 'running' \
                AND claim_token = ?",
        )
        .bind(attempt_state_str(state))
        .bind(next_attempt_at)
        .bind(error)
        .bind(diagnostic)
        .bind(now_ts())
        .bind(key_attempt_id)
        .bind(claim.user_id)
        .bind(claim.author_id)
        .bind(claim.claim_token.to_string())
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;
        if updated.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "author link key attempt",
            });
        }

        tx.commit().await.map_err(map_db_err)?;
        Ok(())
    }

    async fn apply_guarded_route(
        &self,
        write: GuardedRouteWrite,
    ) -> Result<RouteWriteOutcome, DbError> {
        let GuardedRouteWrite {
            claim_token,
            author_id,
            evidence,
        } = write;
        // Consuming the opaque capability is the write's standard of proof: only
        // `guard_author_route` can mint one, so no caller can assert Agree here.
        let evidence = evidence.into_agreed_evidence();

        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        let user_id = author_owner_tx(&mut tx, author_id).await?;
        if let Some(claim_token) = claim_token {
            verify_claim_tx(
                &mut tx,
                &AuthorLinkClaim {
                    author_id,
                    user_id,
                    claim_token,
                    lease_expires_at: Utc::now(),
                    cursor: None,
                    display_name_generation: 0,
                },
            )
            .await?;
        }

        let key = evidence.key.clone();
        let provenance = match evidence.source {
            AuthorRouteEvidenceSource::Tier1SettledWork => AuthorRouteProvenance::Tier1Inherited,
            AuthorRouteEvidenceSource::ReadarrImport => AuthorRouteProvenance::ReadarrGuarded,
        };
        let generation = current_generation_tx(&mut tx, author_id).await?;
        let now = now_ts();

        let outcome = match find_route_for_tuple_tx(&mut tx, user_id, &key).await? {
            // Another author already holds this canonical tuple. Both rows stay
            // exactly as they are; the collision becomes review evidence.
            Some(existing) if existing.author_id != author_id => {
                let candidate = upsert_non_tier2_candidate_tx(
                    &mut tx,
                    user_id,
                    generation,
                    NonTier2Candidate {
                        author_id,
                        key: &key,
                        candidate_name: &evidence.observed_name,
                        reason: AuthorLinkCandidateReason::OwnershipCollision,
                        verdict: AuthorVerdict::Agree,
                        previously_removed: false,
                    },
                )
                .await?;
                RouteWriteOutcome::ParkedOwnershipCollision(candidate)
            }
            // The user removed this exact route. Automation parks; only an
            // explicit user re-pick may ever clear the tombstone.
            Some(existing) if existing.state == AuthorRouteState::Removed => {
                let candidate = upsert_non_tier2_candidate_tx(
                    &mut tx,
                    user_id,
                    generation,
                    NonTier2Candidate {
                        author_id,
                        key: &key,
                        candidate_name: &evidence.observed_name,
                        reason: AuthorLinkCandidateReason::Tombstoned,
                        verdict: AuthorVerdict::Agree,
                        previously_removed: true,
                    },
                )
                .await?;
                RouteWriteOutcome::ParkedTombstoned(candidate)
            }
            Some(existing) => {
                let upgraded = existing.provenance == AuthorRouteProvenance::LegacyUnguarded;
                if upgraded {
                    // Provenance and verification time only. The row's identity
                    // and history are exactly what they were — the guarded
                    // evidence proves the value, it does not replace the row.
                    sqlx::query(
                        "UPDATE author_provider_routes SET provenance = ?, verified_at = ? \
                          WHERE id = ?",
                    )
                    .bind(provenance_str(provenance))
                    .bind(&now)
                    .bind(existing.id)
                    .execute(&mut *tx)
                    .await
                    .map_err(map_db_err)?;
                } else {
                    sqlx::query("UPDATE author_provider_routes SET verified_at = ? WHERE id = ?")
                        .bind(&now)
                        .bind(existing.id)
                        .execute(&mut *tx)
                        .await
                        .map_err(map_db_err)?;
                }
                insert_name_variant_tx(
                    &mut tx,
                    user_id,
                    author_id,
                    name_source_for_provider(key.provider()),
                    &evidence.observed_name,
                    Some(existing.id),
                    None,
                )
                .await?;
                let route = load_route_by_id_tx(&mut tx, existing.id).await?;
                if upgraded {
                    RouteWriteOutcome::LegacyProvenanceUpgraded(route)
                } else {
                    RouteWriteOutcome::AlreadyActive(route)
                }
            }
            None => {
                // A different, still-unverified legacy value for the same
                // provider is a contradiction, not something automation gets to
                // overwrite: both values stay visible and the user decides.
                let contradicting: Option<i64> = sqlx::query_scalar(
                    "SELECT id FROM author_provider_routes \
                      WHERE user_id = ? AND author_id = ? AND provider = ? AND state = 'active' \
                        AND provenance = 'legacy_unguarded' \
                      ORDER BY id LIMIT 1",
                )
                .bind(user_id)
                .bind(author_id)
                .bind(provider_str(key.provider()))
                .fetch_optional(&mut *tx)
                .await
                .map_err(map_db_err)?;

                if contradicting.is_some() {
                    let candidate = upsert_non_tier2_candidate_tx(
                        &mut tx,
                        user_id,
                        generation,
                        NonTier2Candidate {
                            author_id,
                            key: &key,
                            candidate_name: &evidence.observed_name,
                            reason: AuthorLinkCandidateReason::LegacyContradiction,
                            verdict: AuthorVerdict::Agree,
                            previously_removed: false,
                        },
                    )
                    .await?;
                    RouteWriteOutcome::ParkedLegacyContradiction(candidate)
                } else {
                    let inserted = sqlx::query(
                        "INSERT INTO author_provider_routes \
                             (user_id, author_id, provider, route_value, state, provenance, \
                              evidence_work_id, created_at, verified_at) \
                         VALUES (?, ?, ?, ?, 'active', ?, ?, ?, ?)",
                    )
                    .bind(user_id)
                    .bind(author_id)
                    .bind(provider_str(key.provider()))
                    .bind(key.value())
                    .bind(provenance_str(provenance))
                    .bind(evidence.evidence_work_id)
                    .bind(&now)
                    .bind(&now)
                    .execute(&mut *tx)
                    .await
                    .map_err(map_db_err)?;
                    let route_id = inserted.last_insert_rowid();
                    insert_name_variant_tx(
                        &mut tx,
                        user_id,
                        author_id,
                        name_source_for_provider(key.provider()),
                        &evidence.observed_name,
                        Some(route_id),
                        None,
                    )
                    .await?;
                    RouteWriteOutcome::Attached(load_route_by_id_tx(&mut tx, route_id).await?)
                }
            }
        };

        tx.commit().await.map_err(map_db_err)?;
        Ok(outcome)
    }

    async fn record_candidates(
        &self,
        claim: AuthorLinkClaim,
        candidates: Vec<AuthorLinkCandidate>,
    ) -> Result<(), DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        verify_claim_tx(&mut tx, &claim).await?;
        if candidates.is_empty() {
            tx.commit().await.map_err(map_db_err)?;
            return Ok(());
        }

        let generation = candidates
            .iter()
            .map(|candidate| candidate.evidence_generation)
            .max()
            .unwrap_or(0);
        let stored = current_generation_tx(&mut tx, claim.author_id).await?;
        if generation > stored {
            sqlx::query(
                "UPDATE author_link_progress SET evidence_generation = ?, updated_at = ? \
                  WHERE author_id = ?",
            )
            .bind(generation)
            .bind(now_ts())
            .bind(claim.author_id)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
            // Older-generation evidence is superseded, never deleted: the audit
            // trail of what a prior run believed stays readable.
            sqlx::query(
                "UPDATE author_link_candidates \
                    SET status = 'superseded', resolved_at = ? \
                  WHERE user_id = ? AND author_id = ? AND status = 'pending' \
                    AND evidence_generation < ?",
            )
            .bind(now_ts())
            .bind(claim.user_id)
            .bind(claim.author_id)
            .bind(generation)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;
        }

        for candidate in &candidates {
            let provider = provider_str(candidate.key.provider());
            let value = candidate.key.value();
            let reason = candidate_reason_str(candidate.reason);
            let previously_removed: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM author_provider_routes \
                                WHERE user_id = ? AND provider = ? AND route_value = ? \
                                  AND state = 'removed')",
            )
            .bind(claim.user_id)
            .bind(provider)
            .bind(&value)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_db_err)?;

            sqlx::query(
                "INSERT INTO author_link_candidates \
                     (user_id, author_id, provider, route_value, candidate_name, reason, \
                      name_verdict, primary_name_verdict, top_work_preview, \
                      catalog_evidence_state, corroborated_title_count, settled_work_count, \
                      previously_removed, status, evidence_generation, observed_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?) \
                 ON CONFLICT(user_id, author_id, provider, route_value, reason, \
                             evidence_generation) DO UPDATE SET \
                     candidate_name = excluded.candidate_name, \
                     name_verdict = excluded.name_verdict, \
                     primary_name_verdict = excluded.primary_name_verdict, \
                     top_work_preview = excluded.top_work_preview, \
                     catalog_evidence_state = excluded.catalog_evidence_state, \
                     corroborated_title_count = excluded.corroborated_title_count, \
                     settled_work_count = excluded.settled_work_count, \
                     previously_removed = excluded.previously_removed, \
                     observed_at = excluded.observed_at",
            )
            .bind(claim.user_id)
            .bind(candidate.author_id)
            .bind(provider)
            .bind(&value)
            .bind(&candidate.candidate_name)
            .bind(reason)
            .bind(verdict_str(candidate.name_verdict))
            .bind(verdict_str(candidate.primary_name_verdict))
            .bind(&candidate.top_work_preview)
            .bind(catalog_state_str(candidate.catalog_evidence_state))
            .bind(i64::from(candidate.corroborated_title_count))
            .bind(i64::from(candidate.settled_work_count))
            .bind(previously_removed)
            .bind(candidate.evidence_generation)
            .bind(ts(candidate.observed_at))
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;

            let candidate_id: i64 = sqlx::query_scalar(
                "SELECT id FROM author_link_candidates \
                  WHERE user_id = ? AND author_id = ? AND provider = ? AND route_value = ? \
                    AND reason = ? AND evidence_generation = ?",
            )
            .bind(claim.user_id)
            .bind(candidate.author_id)
            .bind(provider)
            .bind(&value)
            .bind(reason)
            .bind(candidate.evidence_generation)
            .fetch_one(&mut *tx)
            .await
            .map_err(map_db_err)?;

            sqlx::query(
                "DELETE FROM author_link_candidate_alternate_name_evidence WHERE candidate_id = ?",
            )
            .bind(candidate_id)
            .execute(&mut *tx)
            .await
            .map_err(map_db_err)?;

            let mut seen = HashSet::new();
            let mut ordinal = 0i64;
            for alternate in &candidate.alternate_name_evidence {
                let canonical = canonical_author_key(&alternate.name);
                if canonical.is_empty() || !seen.insert(canonical.clone()) {
                    continue;
                }
                sqlx::query(
                    "INSERT INTO author_link_candidate_alternate_name_evidence \
                         (candidate_id, ordinal, name, canonical_name, verdict) \
                     VALUES (?, ?, ?, ?, ?)",
                )
                .bind(candidate_id)
                .bind(ordinal)
                .bind(alternate.name.trim())
                .bind(&canonical)
                .bind(verdict_str(alternate.verdict))
                .execute(&mut *tx)
                .await
                .map_err(map_db_err)?;
                ordinal += 1;
            }
        }

        tx.commit().await.map_err(map_db_err)?;
        Ok(())
    }

    async fn record_readarr_rejection(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        rejected: RejectedAuthorRouteEvidence,
    ) -> Result<AuthorLinkCandidate, DbError> {
        let evidence = rejected.evidence();
        if !matches!(evidence.source, AuthorRouteEvidenceSource::ReadarrImport) {
            return Err(DbError::Constraint {
                message: "only Readarr import evidence may use the claimless rejection seam"
                    .to_string(),
            });
        }
        if !matches!(evidence.key, AuthorRouteKey::Goodreads(_)) {
            return Err(DbError::Constraint {
                message: "Readarr author evidence must carry a canonical Goodreads key".to_string(),
            });
        }

        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        require_author_owned_tx(&mut tx, user_id, author_id).await?;
        // No worker claim is taken or required: a Readarr import is not the
        // author road, and it must never disturb its lease.
        let generation = current_generation_tx(&mut tx, author_id).await?;

        insert_name_variant_tx(
            &mut tx,
            user_id,
            author_id,
            AuthorNameSource::Readarr,
            &evidence.observed_name,
            None,
            None,
        )
        .await?;
        let candidate = upsert_non_tier2_candidate_tx(
            &mut tx,
            user_id,
            generation,
            NonTier2Candidate {
                author_id,
                key: &evidence.key,
                candidate_name: &evidence.observed_name,
                reason: AuthorLinkCandidateReason::ReadarrNameGuardFailed,
                verdict: rejected.verdict(),
                previously_removed: false,
            },
        )
        .await?;

        tx.commit().await.map_err(map_db_err)?;
        Ok(candidate)
    }

    async fn advance_progress(
        &self,
        claim: AuthorLinkClaim,
        update: AuthorLinkProgressUpdate,
    ) -> Result<(), DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        verify_claim_tx(&mut tx, &claim).await?;

        let now = now_ts();
        let next_attempt_at = ts(update.next_attempt_at);
        // Clearing the dirty flag is a compare-and-set against the generation
        // this worker was handed. An observation that landed mid-run keeps the
        // author dirty and immediately due rather than being silently dropped.
        sqlx::query(
            "UPDATE author_link_progress \
                SET state = ?, tier = ?, cursor = ?, evaluated_fingerprint = ?, \
                    evidence_generation = ?, last_error = ?, \
                    would_have_linked_at_090 = ?, \
                    attempt_count = attempt_count + 1, \
                    display_name_dirty = CASE \
                        WHEN display_name_generation = ? THEN ? ELSE 1 END, \
                    next_attempt_at = CASE \
                        WHEN display_name_generation = ? THEN ? ELSE MIN(next_attempt_at, ?) END, \
                    claim_token = NULL, lease_until = NULL, updated_at = ? \
              WHERE author_id = ? AND user_id = ?",
        )
        .bind(progress_state_str(update.state))
        .bind(update.tier.map(i64::from))
        .bind(update.cursor.as_ref().map(cursor_to_string))
        .bind(fingerprint_to_string(&update.evaluated_fingerprint))
        .bind(update.evidence_generation)
        .bind(&update.last_error)
        .bind(update.would_have_linked_at_090)
        .bind(claim.display_name_generation)
        .bind(update.display_name_dirty)
        .bind(claim.display_name_generation)
        .bind(&next_attempt_at)
        .bind(&now)
        .bind(&now)
        .bind(claim.author_id)
        .bind(claim.user_id)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;

        tx.commit().await.map_err(map_db_err)?;
        Ok(())
    }

    async fn pick_candidate_as_user(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<AuthorRoute, DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;

        let row = sqlx::query(&format!(
            "SELECT {CANDIDATE_COLUMNS} FROM author_link_candidates \
              WHERE id = ? AND user_id = ? AND status = 'pending'"
        ))
        .bind(candidate_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_db_err)?
        .ok_or(DbError::NotFound {
            entity: "author link candidate",
        })?;
        let candidate = row_to_candidate(&row)?;
        let generation = current_generation_tx(&mut tx, candidate.author_id).await?;
        if candidate.evidence_generation != generation {
            return Err(DbError::NotFound {
                entity: "author link candidate",
            });
        }

        let route =
            attach_route_as_user_tx(&mut tx, user_id, candidate.author_id, candidate.key.clone())
                .await?;

        let now = now_ts();
        sqlx::query(
            "UPDATE author_link_candidates SET status = 'picked', resolved_at = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(candidate.id)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;
        // Competing current-generation candidates for the same author lost:
        // the user answered the question they were asking.
        sqlx::query(
            "UPDATE author_link_candidates SET status = 'superseded', resolved_at = ? \
              WHERE user_id = ? AND author_id = ? AND evidence_generation = ? \
                AND status = 'pending' AND id <> ?",
        )
        .bind(&now)
        .bind(user_id)
        .bind(candidate.author_id)
        .bind(generation)
        .bind(candidate.id)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;

        // The picked record's whole name set is retained: the primary carries
        // the OL search role Primary, every canonical-distinct alias Alias. A
        // display choice is a separate, explicit User variant.
        let source = name_source_for_provider(candidate.key.provider());
        let ol = matches!(candidate.key, AuthorRouteKey::OpenLibrary(_));
        upsert_name_variant_role_tx(
            &mut tx,
            user_id,
            candidate.author_id,
            source,
            &candidate.candidate_name,
            ol.then_some(OpenLibraryNameRole::Primary),
        )
        .await?;
        let alternates = load_alternate_evidence_tx(&mut tx, &[candidate.id]).await?;
        for alternate in alternates.get(&candidate.id).into_iter().flatten() {
            upsert_name_variant_role_tx(
                &mut tx,
                user_id,
                candidate.author_id,
                source,
                &alternate.name,
                ol.then_some(OpenLibraryNameRole::Alias),
            )
            .await?;
        }

        rederive_progress_tx(&mut tx, user_id, candidate.author_id).await?;
        tx.commit().await.map_err(map_db_err)?;
        Ok(route)
    }

    async fn attach_route_as_user(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        key: AuthorRouteKey,
    ) -> Result<AuthorRoute, DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        let route = attach_route_as_user_tx(&mut tx, user_id, author_id, key).await?;
        rederive_progress_tx(&mut tx, user_id, author_id).await?;
        tx.commit().await.map_err(map_db_err)?;
        Ok(route)
    }

    async fn dismiss_candidate_as_user(
        &self,
        user_id: UserId,
        candidate_id: i64,
    ) -> Result<(), DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;

        let author_id: AuthorId = sqlx::query_scalar(
            "SELECT author_id FROM author_link_candidates \
              WHERE id = ? AND user_id = ? AND status = 'pending'",
        )
        .bind(candidate_id)
        .bind(user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(map_db_err)?
        .ok_or(DbError::NotFound {
            entity: "author link candidate",
        })?;

        let generation = current_generation_tx(&mut tx, author_id).await?;
        let dismissed = sqlx::query(
            "UPDATE author_link_candidates SET status = 'dismissed', resolved_at = ? \
              WHERE id = ? AND user_id = ? AND status = 'pending' AND evidence_generation = ?",
        )
        .bind(now_ts())
        .bind(candidate_id)
        .bind(user_id)
        .bind(generation)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;
        if dismissed.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "author link candidate",
            });
        }

        rederive_progress_tx(&mut tx, user_id, author_id).await?;
        tx.commit().await.map_err(map_db_err)?;
        Ok(())
    }

    async fn list_review(&self, user_id: UserId) -> Result<Vec<AuthorLinkReview>, DbError> {
        let mut conn = self.pool().acquire().await.map_err(map_db_err)?;

        // Only the current evidence generation is reviewable: a superseded run's
        // question is not one the user should still be asked.
        let author_ids: Vec<AuthorId> = sqlx::query_scalar(
            "SELECT DISTINCT c.author_id FROM author_link_candidates c \
               JOIN author_link_progress p ON p.author_id = c.author_id \
              WHERE c.user_id = ? AND c.status = 'pending' \
                AND c.evidence_generation = p.evidence_generation \
              ORDER BY c.author_id",
        )
        .bind(user_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(map_db_err)?;

        let mut reviews = Vec::with_capacity(author_ids.len());
        for author_id in author_ids {
            let author_row = sqlx::query("SELECT * FROM authors WHERE id = ? AND user_id = ?")
                .bind(author_id)
                .bind(user_id)
                .fetch_one(&mut *conn)
                .await
                .map_err(map_db_err)?;
            let author = row_to_author(author_row)?;

            let route_rows = sqlx::query(&format!(
                "SELECT {ROUTE_COLUMNS} FROM author_provider_routes \
                  WHERE user_id = ? AND author_id = ? AND state = 'active'"
            ))
            .bind(user_id)
            .bind(author_id)
            .fetch_all(&mut *conn)
            .await
            .map_err(map_db_err)?;
            let mut routes = route_rows
                .iter()
                .map(row_to_route)
                .collect::<Result<Vec<_>, _>>()?;
            routes.sort_by_key(|route| (provenance_rank(route.provenance), route.id));

            let candidate_rows = sqlx::query(&format!(
                "SELECT {CANDIDATE_COLUMNS} FROM author_link_candidates c \
                  WHERE c.user_id = ? AND c.author_id = ? AND c.status = 'pending' \
                    AND c.evidence_generation = \
                        (SELECT evidence_generation FROM author_link_progress WHERE author_id = ?)"
            ))
            .bind(user_id)
            .bind(author_id)
            .bind(author_id)
            .fetch_all(&mut *conn)
            .await
            .map_err(map_db_err)?;
            let mut candidates = candidate_rows
                .iter()
                .map(row_to_candidate)
                .collect::<Result<Vec<_>, _>>()?;

            let ids: Vec<i64> = candidates.iter().map(|candidate| candidate.id).collect();
            let mut alternates = load_alternate_evidence_tx(&mut conn, &ids).await?;
            for candidate in &mut candidates {
                candidate.alternate_name_evidence =
                    alternates.remove(&candidate.id).unwrap_or_default();
            }
            // Observed evidence strength first, then settled evidence ahead of
            // unfinished evidence, then name strength, then the canonical value.
            candidates.sort_by(|a, b| {
                b.corroborated_title_count
                    .cmp(&a.corroborated_title_count)
                    .then(
                        catalog_state_rank(a.catalog_evidence_state)
                            .cmp(&catalog_state_rank(b.catalog_evidence_state)),
                    )
                    .then(verdict_rank(a.name_verdict).cmp(&verdict_rank(b.name_verdict)))
                    .then(a.key.value().cmp(&b.key.value()))
            });

            let link_state = if candidates.is_empty() {
                if routes.is_empty() {
                    AuthorLinkState::Unlinked
                } else {
                    AuthorLinkState::Linked
                }
            } else {
                AuthorLinkState::NeedsReview
            };

            reviews.push(AuthorLinkReview {
                author,
                link_state,
                routes,
                candidates,
            });
        }
        Ok(reviews)
    }

    async fn sweep_progress(&self, user_id: UserId) -> Result<AuthorSweepProgress, DbError> {
        let mut conn = self.pool().acquire().await.map_err(map_db_err)?;

        let progress: (i64, i64, i64, i64, i64, i64, i64, i64, Option<String>) = sqlx::query_as(
            "SELECT COUNT(*), \
                    SUM(state = 'linked'), \
                    SUM(state = 'queued'), \
                    SUM(state = 'running'), \
                    SUM(state IN ('parked_no_settled_work', 'parked_no_evidence')), \
                    SUM(state = 'needs_review'), \
                    SUM(state = 'retryable_failure'), \
                    SUM(would_have_linked_at_090), \
                    MIN(next_attempt_at) \
               FROM author_link_progress WHERE user_id = ?",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(map_db_err)?;

        // Old-generation attempts are audit history, never live sweep state.
        let attempts: (i64, i64, i64) = sqlx::query_as(
            "SELECT SUM(k.state = 'retryable'), \
                    SUM(k.state IN ('skipped_not_configured', 'skipped_permanent')), \
                    SUM(k.state = 'parked_layout_drift') \
               FROM author_link_key_attempts k \
               JOIN author_link_progress p ON p.author_id = k.author_id \
              WHERE k.user_id = ? AND k.evidence_generation = p.evidence_generation",
        )
        .bind(user_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(map_db_err)?;

        Ok(AuthorSweepProgress {
            total: progress.0.max(0) as u64,
            completed: progress.1.max(0) as u64,
            queued: progress.2.max(0) as u64,
            running: progress.3.max(0) as u64,
            parked: progress.4.max(0) as u64,
            needs_review: progress.5.max(0) as u64,
            retryable_failures: progress.6.max(0) as u64,
            key_retryable: attempts.0.max(0) as u64,
            key_skipped: attempts.1.max(0) as u64,
            key_layout_drift: attempts.2.max(0) as u64,
            would_have_linked_at_090: progress.7.max(0) as u64,
            oldest_due_at: progress.8.as_deref().map(parse_dt).transpose()?,
        })
    }

    async fn remove_route_as_user(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        route_id: i64,
    ) -> Result<(), DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        // The row is retained, not deleted: the tombstone is the durable record
        // that the user said no, and it outlives every automatic re-discovery.
        let removed = sqlx::query(
            "UPDATE author_provider_routes \
                SET state = 'removed', removed_at = ?, removed_by_user_id = ? \
              WHERE id = ? AND user_id = ? AND author_id = ? AND state = 'active'",
        )
        .bind(now_ts())
        .bind(user_id)
        .bind(route_id)
        .bind(user_id)
        .bind(author_id)
        .execute(&mut *tx)
        .await
        .map_err(map_db_err)?;
        if removed.rows_affected() == 0 {
            return Err(DbError::NotFound {
                entity: "author provider route",
            });
        }

        rederive_progress_tx(&mut tx, user_id, author_id).await?;
        tx.commit().await.map_err(map_db_err)?;
        Ok(())
    }

    async fn list_active_routes(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        provider: Option<AuthorProvider>,
    ) -> Result<Vec<AuthorRoute>, DbError> {
        let mut conn = self.pool().acquire().await.map_err(map_db_err)?;
        require_author_owned_tx(&mut conn, user_id, author_id).await?;

        let rows = sqlx::query(&format!(
            "SELECT {ROUTE_COLUMNS} FROM author_provider_routes \
              WHERE user_id = ? AND author_id = ? AND state = 'active' \
                AND (? IS NULL OR provider = ?)"
        ))
        .bind(user_id)
        .bind(author_id)
        .bind(provider.map(provider_str))
        .bind(provider.map(provider_str))
        .fetch_all(&mut *conn)
        .await
        .map_err(map_db_err)?;
        let mut routes = rows
            .iter()
            .map(row_to_route)
            .collect::<Result<Vec<_>, _>>()?;
        routes.sort_by_key(|route| (provenance_rank(route.provenance), route.id));
        Ok(routes)
    }

    async fn list_routes_for_view(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<AuthorRoute>, DbError> {
        let mut conn = self.pool().acquire().await.map_err(map_db_err)?;
        require_author_owned_tx(&mut conn, user_id, author_id).await?;

        let rows = sqlx::query(&format!(
            "SELECT {ROUTE_COLUMNS} FROM author_provider_routes \
              WHERE user_id = ? AND author_id = ?"
        ))
        .bind(user_id)
        .bind(author_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(map_db_err)?;
        let mut routes = rows
            .iter()
            .map(row_to_route)
            .collect::<Result<Vec<_>, _>>()?;
        // Active rows first in the same provenance order `list_active_routes`
        // uses, then the removal history — one stable panel order.
        routes.sort_by_key(|route| {
            (
                route.state != AuthorRouteState::Active,
                provenance_rank(route.provenance),
                route.id,
            )
        });
        Ok(routes)
    }

    async fn has_active_route(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        provider: AuthorProvider,
    ) -> Result<bool, DbError> {
        // Route rows only — a frozen legacy scalar is not linkage.
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM author_provider_routes \
                            WHERE user_id = ? AND author_id = ? AND provider = ? \
                              AND state = 'active')",
        )
        .bind(user_id)
        .bind(author_id)
        .bind(provider_str(provider))
        .fetch_one(self.pool())
        .await
        .map_err(map_db_err)
    }

    async fn compatibility_projection(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorCompatibilityProjection, DbError> {
        let routes = self.list_active_routes(user_id, author_id, None).await?;
        // One value per provider for the old scalar-shaped API surface. It is a
        // projection, never the route set, and never a mutation input.
        let pick = |provider: AuthorProvider| {
            routes
                .iter()
                .find(|route| route.key.provider() == provider)
                .map(|route| route.key.value())
        };
        Ok(AuthorCompatibilityProjection {
            ol_key: pick(AuthorProvider::OpenLibrary),
            gr_key: pick(AuthorProvider::Goodreads),
            hc_key: pick(AuthorProvider::Hardcover),
        })
    }

    async fn ingest_legacy_routes(&self) -> Result<AuthorRouteBackfillReport, DbError> {
        loop {
            let staged: Vec<(i64, i64, i64, String, String)> = sqlx::query_as(
                "SELECT id, user_id, author_id, provider, raw_value \
                   FROM author_route_legacy_staging WHERE status = 'pending' \
                  ORDER BY id LIMIT ?",
            )
            .bind(LEGACY_BATCH)
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
            if staged.is_empty() {
                break;
            }

            let mut tx = self.pool().begin().await.map_err(map_db_err)?;
            for (id, user_id, author_id, provider, raw_value) in staged {
                let now = now_ts();
                let provider = parse_provider(&provider)?;
                match AuthorRouteKey::parse(provider, &raw_value) {
                    Ok(key) => {
                        sqlx::query(
                            "INSERT INTO author_provider_routes \
                                 (user_id, author_id, provider, route_value, state, provenance, \
                                  created_at) \
                             VALUES (?, ?, ?, ?, 'active', 'legacy_unguarded', ?) \
                             ON CONFLICT(user_id, provider, route_value) DO NOTHING",
                        )
                        .bind(user_id)
                        .bind(author_id)
                        .bind(provider_str(provider))
                        .bind(key.value())
                        .bind(&now)
                        .execute(&mut *tx)
                        .await
                        .map_err(map_db_err)?;
                        sqlx::query(
                            "UPDATE author_route_legacy_staging \
                                SET status = 'ingested', diagnostic = NULL, updated_at = ? \
                              WHERE id = ?",
                        )
                        .bind(&now)
                        .bind(id)
                        .execute(&mut *tx)
                        .await
                        .map_err(map_db_err)?;
                    }
                    Err(error) => {
                        // A value that cannot be canonicalized is reported, never
                        // guessed into a route and never erased.
                        sqlx::query(
                            "UPDATE author_route_legacy_staging \
                                SET status = 'invalid', diagnostic = ?, updated_at = ? \
                              WHERE id = ?",
                        )
                        .bind(format!("{error:?}"))
                        .bind(&now)
                        .bind(id)
                        .execute(&mut *tx)
                        .await
                        .map_err(map_db_err)?;
                    }
                }
            }
            tx.commit().await.map_err(map_db_err)?;
        }

        loop {
            let staged: Vec<(i64, i64, String)> = sqlx::query_as(
                "SELECT author_id, user_id, name FROM author_name_legacy_staging \
                  WHERE status = 'pending' ORDER BY author_id LIMIT ?",
            )
            .bind(LEGACY_BATCH)
            .fetch_all(self.pool())
            .await
            .map_err(map_db_err)?;
            if staged.is_empty() {
                break;
            }

            let mut tx = self.pool().begin().await.map_err(map_db_err)?;
            for (author_id, user_id, name) in staged {
                let canonical = canonical_author_key(&name);
                insert_name_variant_tx(
                    &mut tx,
                    user_id,
                    author_id,
                    AuthorNameSource::Legacy,
                    &name,
                    None,
                    None,
                )
                .await?;
                sqlx::query(
                    "UPDATE author_name_legacy_staging \
                        SET status = ?, canonical_name = ?, diagnostic = ?, updated_at = ? \
                      WHERE author_id = ?",
                )
                .bind(if canonical.is_empty() {
                    "invalid"
                } else {
                    "ingested"
                })
                .bind((!canonical.is_empty()).then_some(canonical.clone()))
                .bind(canonical.is_empty().then_some("name does not canonicalize"))
                .bind(now_ts())
                .bind(author_id)
                .execute(&mut *tx)
                .await
                .map_err(map_db_err)?;
            }
            tx.commit().await.map_err(map_db_err)?;
        }

        // Cutover completeness: every author leaves this pass enqueued, so no
        // pre-F1 row is invisible to the sweep afterwards.
        let now = now_ts();
        sqlx::query(
            "INSERT INTO author_link_progress \
                 (author_id, user_id, state, next_attempt_at, trigger, updated_at) \
             SELECT a.id, a.user_id, 'queued', ?, 'legacy_backfill', ? FROM authors a \
              WHERE NOT EXISTS (SELECT 1 FROM author_link_progress p WHERE p.author_id = a.id)",
        )
        .bind(&now)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(map_db_err)?;

        backfill_report(self).await
    }

    async fn verify_cutover_ready(&self) -> Result<AuthorRouteBackfillReport, DbError> {
        backfill_report(self).await
    }
}

// ---------------------------------------------------------------------------
// Author merge: link-state fold
// ---------------------------------------------------------------------------

/// The parts of a loser name variant the merge fold decides with.
struct VariantFold {
    id: i64,
    source: String,
    canonical_name: String,
    open_library_role: Option<String>,
    user_selected_at: Option<String>,
}

async fn load_variant_folds_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    author_id: AuthorId,
) -> Result<Vec<VariantFold>, DbError> {
    let rows = sqlx::query(
        "SELECT id, source, canonical_name, open_library_role, user_selected_at \
           FROM author_name_variants WHERE user_id = ? AND author_id = ? ORDER BY id",
    )
    .bind(user_id)
    .bind(author_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(map_db_err)?;
    rows.iter()
        .map(|row| {
            Ok(VariantFold {
                id: row.try_get("id").map_err(map_db_err)?,
                source: row.try_get("source").map_err(map_db_err)?,
                canonical_name: row.try_get("canonical_name").map_err(map_db_err)?,
                open_library_role: row.try_get("open_library_role").map_err(map_db_err)?,
                user_selected_at: row.try_get("user_selected_at").map_err(map_db_err)?,
            })
        })
        .collect()
}

/// Fold the loser's author-link state onto the survivor, inside the caller's
/// merge transaction and before the loser row is deleted.
///
/// Merge is monotonic for routes and monitoring, and it never resurrects a
/// tombstone: a survivor route the user removed stays removed no matter what
/// the loser carried.
///
/// Route rows move rather than coalesce. `UNIQUE(user_id, provider,
/// route_value)` is user-wide, so a survivor and a loser under the same user
/// can never hold the same canonical tuple — the same-key precedence case IR v2
/// describes is unreachable state, and is deliberately not implemented (F-011).
pub(crate) async fn fold_author_link_state_tx(
    conn: &mut SqliteConnection,
    user_id: UserId,
    survivor_id: AuthorId,
    loser_id: AuthorId,
) -> Result<(), DbError> {
    sqlx::query(
        "UPDATE author_provider_routes SET author_id = ? WHERE user_id = ? AND author_id = ?",
    )
    .bind(survivor_id)
    .bind(user_id)
    .bind(loser_id)
    .execute(&mut *conn)
    .await
    .map_err(map_db_err)?;

    // Staged legacy values move too, or a merge that happens *before* the
    // cutover ingestion silently destroys the loser's provider linkage: the
    // staging rows are `ON DELETE CASCADE` on the author, and the startup
    // duplicate-author repair (`pool::backfill_author_identity`) runs before
    // `ingest_legacy_routes`. The survivor's own staged value wins a provider
    // collision — the same precedence the retired scalar COALESCE had — and the
    // loser's row is then dropped rather than left to violate
    // `UNIQUE(user_id, author_id, provider)`.
    sqlx::query(
        "DELETE FROM author_route_legacy_staging \
          WHERE user_id = ? AND author_id = ? AND provider IN ( \
                SELECT provider FROM author_route_legacy_staging \
                 WHERE user_id = ? AND author_id = ?)",
    )
    .bind(user_id)
    .bind(loser_id)
    .bind(user_id)
    .bind(survivor_id)
    .execute(&mut *conn)
    .await
    .map_err(map_db_err)?;
    sqlx::query(
        "UPDATE author_route_legacy_staging SET author_id = ?, updated_at = ? \
          WHERE user_id = ? AND author_id = ?",
    )
    .bind(survivor_id)
    .bind(now_ts())
    .bind(user_id)
    .bind(loser_id)
    .execute(&mut *conn)
    .await
    .map_err(map_db_err)?;

    // Names: every distinct associated name survives the merge. A duplicate
    // canonical OpenLibrary name keeps the strongest role it was ever observed
    // with — Primary over Alias over "no role asserted".
    let loser_variants = load_variant_folds_tx(conn, user_id, loser_id).await?;
    for loser in loser_variants {
        let survivor_row = sqlx::query(
            "SELECT id, open_library_role, user_selected_at FROM author_name_variants \
              WHERE user_id = ? AND author_id = ? AND source = ? AND canonical_name = ?",
        )
        .bind(user_id)
        .bind(survivor_id)
        .bind(&loser.source)
        .bind(&loser.canonical_name)
        .fetch_optional(&mut *conn)
        .await
        .map_err(map_db_err)?;

        match survivor_row {
            Some(row) => {
                let survivor_variant_id: i64 = row.try_get("id").map_err(map_db_err)?;
                let survivor_role: Option<String> =
                    row.try_get("open_library_role").map_err(map_db_err)?;
                let survivor_selected_at: Option<String> =
                    row.try_get("user_selected_at").map_err(map_db_err)?;
                let loser_role = parse_ol_role(loser.open_library_role.as_deref())?;
                let kept_role = if ol_role_rank(loser_role)
                    > ol_role_rank(parse_ol_role(survivor_role.as_deref())?)
                {
                    loser_role.map(ol_role_str)
                } else {
                    survivor_role.as_deref()
                };
                sqlx::query(
                    "UPDATE author_name_variants \
                        SET open_library_role = ?, user_selected_at = COALESCE(?, ?) \
                      WHERE id = ?",
                )
                .bind(kept_role)
                .bind(&survivor_selected_at)
                .bind(&loser.user_selected_at)
                .bind(survivor_variant_id)
                .execute(&mut *conn)
                .await
                .map_err(map_db_err)?;
                sqlx::query("DELETE FROM author_name_variants WHERE id = ?")
                    .bind(loser.id)
                    .execute(&mut *conn)
                    .await
                    .map_err(map_db_err)?;
            }
            None => {
                sqlx::query("UPDATE author_name_variants SET author_id = ? WHERE id = ?")
                    .bind(survivor_id)
                    .bind(loser.id)
                    .execute(&mut *conn)
                    .await
                    .map_err(map_db_err)?;
            }
        }
    }

    // The survivor's progress row is the one that survives; the loser's — and
    // its generation-scoped key attempts — cascade away with the loser author.
    ensure_progress_tx(conn, user_id, survivor_id, AuthorLinkTrigger::AuthorAdopted).await?;
    let survivor_generation = current_generation_tx(conn, survivor_id).await?;

    // Open questions travel to the survivor and are restamped into its current
    // generation so they stay reviewable; a question the survivor is already
    // asking is dropped rather than duplicated.
    let loser_candidates: Vec<(i64, String, String, String, String, i64)> = sqlx::query_as(
        "SELECT id, provider, route_value, reason, status, evidence_generation \
           FROM author_link_candidates WHERE user_id = ? AND author_id = ? ORDER BY id",
    )
    .bind(user_id)
    .bind(loser_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(map_db_err)?;
    for (id, provider, route_value, reason, status, generation) in loser_candidates {
        let target_generation = if status == "pending" {
            survivor_generation
        } else {
            generation
        };
        let collides: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM author_link_candidates \
                            WHERE user_id = ? AND author_id = ? AND provider = ? \
                              AND route_value = ? AND reason = ? AND evidence_generation = ?)",
        )
        .bind(user_id)
        .bind(survivor_id)
        .bind(&provider)
        .bind(&route_value)
        .bind(&reason)
        .bind(target_generation)
        .fetch_one(&mut *conn)
        .await
        .map_err(map_db_err)?;

        if collides {
            sqlx::query("DELETE FROM author_link_candidates WHERE id = ?")
                .bind(id)
                .execute(&mut *conn)
                .await
                .map_err(map_db_err)?;
        } else {
            sqlx::query(
                "UPDATE author_link_candidates SET author_id = ?, evidence_generation = ? \
                  WHERE id = ?",
            )
            .bind(survivor_id)
            .bind(target_generation)
            .bind(id)
            .execute(&mut *conn)
            .await
            .map_err(map_db_err)?;
        }
    }

    rederive_progress_tx(conn, user_id, survivor_id).await?;
    Ok(())
}

/// The one atomic author create/adopt gate every F1 add path goes through.
///
/// The name says what it guarantees: a committed F1 author *always* has a
/// progress row and its initial name variant, so no add door can leave an
/// author that the sweep will never look at. The route field family is absent
/// from the request by construction — an explicitly selected route is a
/// separate, user-sovereign step after this gate commits.
pub(crate) async fn create_or_adopt_author_tx(
    conn: &mut SqliteConnection,
    request: &crate::CreateAuthorGateRequest,
) -> Result<(livrarr_domain::Author, bool), DbError> {
    let canonical = canonical_author_key(&request.name);
    let normalized_name = (!canonical.is_empty()).then_some(canonical);
    let now = Utc::now().to_rfc3339();

    // Same named ON CONFLICT convergence as `create_author`: a concurrent
    // same-key insert lands on DO NOTHING and every racer re-selects the one
    // winning row (issue #175).
    let inserted = sqlx::query(
        "INSERT INTO authors (user_id, name, sort_name, import_id, added_at, normalized_name) \
         VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(user_id, normalized_name) WHERE normalized_name IS NOT NULL DO NOTHING",
    )
    .bind(request.user_id)
    .bind(&request.name)
    .bind(&request.sort_name)
    .bind(&request.import_id)
    .bind(&now)
    .bind(&normalized_name)
    .execute(&mut *conn)
    .await
    .map_err(map_db_err)?;

    let (author, created) = if inserted.rows_affected() == 1 {
        let row = sqlx::query("SELECT * FROM authors WHERE id = ?")
            .bind(inserted.last_insert_rowid())
            .fetch_one(&mut *conn)
            .await
            .map_err(map_db_err)?;
        (row_to_author(row)?, true)
    } else {
        let row = sqlx::query("SELECT * FROM authors WHERE user_id = ? AND normalized_name = ?")
            .bind(request.user_id)
            .bind(&normalized_name)
            .fetch_one(&mut *conn)
            .await
            .map_err(map_db_err)?;
        (row_to_author(row)?, false)
    };

    insert_name_variant_tx(
        conn,
        request.user_id,
        author.id,
        request.initial_name_source,
        &author.name,
        None,
        None,
    )
    .await?;
    ensure_progress_tx(conn, request.user_id, author.id, request.trigger).await?;
    Ok((author, created))
}

fn verdict_rank(verdict: AuthorVerdict) -> i64 {
    match verdict {
        AuthorVerdict::Agree => 0,
        AuthorVerdict::Grey => 1,
        AuthorVerdict::Abstain => 2,
        AuthorVerdict::Disagree => 3,
    }
}

// ---------------------------------------------------------------------------
// AuthorNameVariantDb
// ---------------------------------------------------------------------------

impl AuthorNameVariantDb for SqliteDb {
    async fn record_observed_names(
        &self,
        user_id: UserId,
        work_id: WorkId,
        observations: &[ProviderAuthorNameObservation],
    ) -> Result<u32, DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;

        let author_id: AuthorId =
            sqlx::query_scalar("SELECT author_id FROM works WHERE id = ? AND user_id = ?")
                .bind(work_id)
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(map_db_err)?
                .flatten()
                .ok_or(DbError::NotFound { entity: "work" })?;

        let inserted = record_observed_names_tx(&mut tx, user_id, author_id, observations).await?;
        tx.commit().await.map_err(map_db_err)?;
        Ok(inserted)
    }

    async fn record_author_observed_names(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        observations: &[ProviderAuthorNameObservation],
    ) -> Result<u32, DbError> {
        let mut tx = self.pool().begin().await.map_err(map_db_err)?;
        require_author_owned_tx(&mut tx, user_id, author_id).await?;
        let inserted = record_observed_names_tx(&mut tx, user_id, author_id, observations).await?;
        tx.commit().await.map_err(map_db_err)?;
        Ok(inserted)
    }

    async fn list_name_variants(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<AuthorNameVariant>, DbError> {
        let mut conn = self.pool().acquire().await.map_err(map_db_err)?;
        require_author_owned_tx(&mut conn, user_id, author_id).await?;

        let rows = sqlx::query(
            "SELECT * FROM author_name_variants WHERE user_id = ? AND author_id = ? ORDER BY id",
        )
        .bind(user_id)
        .bind(author_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(map_db_err)?;
        rows.iter().map(row_to_name_variant).collect()
    }
}

/// The three reads and one write U6 added to this module, over a real migrated
/// SQLite database and the real writers.
///
/// They exist because the behavioural suite reaches these seams only through the
/// author-detail door, which cannot show what the route-history read does with a
/// *removed* row, nor what the author-scoped observation write does when the
/// caller has no work in hand. Same in-crate precedent as
/// `sqlite_author::display_name_origin_tests`.
#[cfg(test)]
mod route_history_and_variant_tests {
    use super::*;
    use crate::test_helpers::create_test_db;
    use crate::{AuthorDb, CreateAuthorDbRequest, CreateUserDbRequest, UserDb};
    use livrarr_domain::UserRole;

    async fn seed(db: &SqliteDb, username: &str) -> (i64, i64) {
        let user = db
            .create_user(CreateUserDbRequest {
                username: username.into(),
                password_hash: "hash".into(),
                role: UserRole::User,
                api_key_hash: format!("{username}-key"),
            })
            .await
            .expect("user");
        let (author, _) = db
            .create_author(CreateAuthorDbRequest {
                user_id: user.id,
                name: "Routed Author".into(),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                hc_key: None,
                import_id: None,
            })
            .await
            .expect("author");
        (user.id, author.id)
    }

    /// The panel read shows a removed route, and shows it after the active ones.
    /// The active-only read still cannot see it — that is what keeps a tombstone
    /// out of every linkage answer.
    #[tokio::test]
    async fn route_history_keeps_removed_rows_after_the_active_ones() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed(&db, "route-history").await;

        let kept = AuthorRouteKey::parse(AuthorProvider::OpenLibrary, "OL9001A").expect("key");
        let removed = AuthorRouteKey::parse(AuthorProvider::Goodreads, "9002").expect("key");
        db.attach_route_as_user(user_id, author_id, kept)
            .await
            .expect("attach kept");
        let removed_route = db
            .attach_route_as_user(user_id, author_id, removed)
            .await
            .expect("attach removed");
        db.remove_route_as_user(user_id, author_id, removed_route.id)
            .await
            .expect("remove");

        let panel = db
            .list_routes_for_view(user_id, author_id)
            .await
            .expect("route history");
        assert_eq!(panel.len(), 2);
        assert_eq!(panel[0].key.value(), "OL9001A");
        assert_eq!(panel[0].state, AuthorRouteState::Active);
        assert_eq!(panel[1].key.value(), "9002");
        assert_eq!(panel[1].state, AuthorRouteState::Removed);
        assert!(panel[1].removed_at.is_some());

        let active = db
            .list_active_routes(user_id, author_id, None)
            .await
            .expect("active routes");
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].key.value(), "OL9001A");
    }

    /// Both new reads are user-scoped: another user's id gets nothing back, not
    /// someone else's author.
    #[tokio::test]
    async fn the_new_reads_refuse_another_users_author() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed(&db, "owner").await;
        let (other_user_id, _) = seed(&db, "other").await;

        db.attach_route_as_user(
            user_id,
            author_id,
            AuthorRouteKey::parse(AuthorProvider::OpenLibrary, "OL9003A").expect("key"),
        )
        .await
        .expect("attach");

        assert!(db
            .list_routes_for_view(other_user_id, author_id)
            .await
            .is_err());
        assert!(db
            .list_name_variants(other_user_id, author_id)
            .await
            .is_err());
        assert!(db
            .record_author_observed_names(
                other_user_id,
                author_id,
                &[ProviderAuthorNameObservation {
                    source: AuthorNameSource::Readarr,
                    name: "Someone Else".into(),
                }],
            )
            .await
            .is_err());
    }

    /// The author-scoped observation write records the name and wakes the
    /// author's display work, exactly like the work-scoped form — and a repeat of
    /// the same spelling from the same source adds nothing.
    #[tokio::test]
    async fn an_author_scoped_observation_records_the_name_and_makes_display_work_due() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed(&db, "author-observe").await;
        db.ensure_enqueued(user_id, author_id, AuthorLinkTrigger::AuthorCreated)
            .await
            .expect("progress row");
        sqlx::query(
            "UPDATE author_link_progress \
                SET display_name_dirty = 0, next_attempt_at = '2999-01-01T00:00:00.000Z' \
              WHERE author_id = ?",
        )
        .bind(author_id)
        .execute(db.pool())
        .await
        .expect("park the author");

        let observation = |name: &str| ProviderAuthorNameObservation {
            source: AuthorNameSource::Readarr,
            name: name.to_string(),
        };
        let inserted = db
            .record_author_observed_names(user_id, author_id, &[observation("Readarr Spelling")])
            .await
            .expect("observation");
        assert_eq!(inserted, 1);

        let variants = db
            .list_name_variants(user_id, author_id)
            .await
            .expect("variants");
        assert!(variants
            .iter()
            .any(|v| v.name == "Readarr Spelling" && v.source == AuthorNameSource::Readarr));

        let (dirty, due): (i64, i64) = sqlx::query_as(
            "SELECT display_name_dirty, julianday(next_attempt_at) <= julianday('now') \
               FROM author_link_progress WHERE author_id = ?",
        )
        .bind(author_id)
        .fetch_one(db.pool())
        .await
        .expect("progress");
        assert_eq!((dirty, due), (1, 1));

        // A repeat of the same spelling is not a new observation.
        let repeat = db
            .record_author_observed_names(user_id, author_id, &[observation("Readarr Spelling")])
            .await
            .expect("repeat observation");
        assert_eq!(repeat, 0);
    }
    /// A merge that happens **before** the cutover ingestion must not destroy the
    /// loser's provider linkage.
    ///
    /// The startup duplicate-author repair (`pool::backfill_author_identity`) runs
    /// before `ingest_legacy_routes`, and the staging rows are `ON DELETE CASCADE`
    /// on the author — so the loser's staged legacy value has to move to the
    /// survivor or it is gone with no way to recover it. The survivor's own staged
    /// value wins a provider collision, the same precedence the retired scalar
    /// COALESCE had.
    #[tokio::test]
    async fn merge_moves_the_losers_staged_legacy_route_to_the_survivor() {
        let db = create_test_db().await;
        let (user_id, survivor_id) = seed(&db, "merge-staging").await;
        let (_, loser_id) = {
            let (author, _) = db
                .create_author(CreateAuthorDbRequest {
                    user_id,
                    name: "Routed Author Duplicate".into(),
                    sort_name: None,
                    ol_key: None,
                    gr_key: None,
                    hc_key: None,
                    import_id: None,
                })
                .await
                .expect("loser author");
            (user_id, author.id)
        };

        // Migration 079 stages every nonempty scalar at migration time; this is
        // the same row it would have written for each of these two authors.
        let stage = |author_id: AuthorId, provider: &'static str, raw: &'static str| {
            sqlx::query(
                "INSERT INTO author_route_legacy_staging \
                     (user_id, author_id, provider, raw_value, status, staged_at, updated_at) \
                 VALUES (?, ?, ?, ?, 'pending', ?, ?)",
            )
            .bind(user_id)
            .bind(author_id)
            .bind(provider)
            .bind(raw)
            .bind(now_ts())
            .bind(now_ts())
        };
        stage(survivor_id, "open_library", "OL-SURVIVOR")
            .execute(db.pool())
            .await
            .expect("stage survivor OL");
        stage(loser_id, "open_library", "OL-LOSER")
            .execute(db.pool())
            .await
            .expect("stage loser OL");
        stage(loser_id, "goodreads", "5150")
            .execute(db.pool())
            .await
            .expect("stage loser GR");

        db.merge_authors(user_id, survivor_id, loser_id)
            .await
            .expect("merge");

        let staged: Vec<(String, String)> = sqlx::query_as(
            "SELECT provider, raw_value FROM author_route_legacy_staging \
              WHERE author_id = ? ORDER BY provider",
        )
        .bind(survivor_id)
        .fetch_all(db.pool())
        .await
        .expect("survivor staging");
        assert_eq!(
            staged,
            vec![
                ("goodreads".to_string(), "5150".to_string()),
                ("open_library".to_string(), "OL-SURVIVOR".to_string()),
            ],
            "the loser's unique provider moves; a collision keeps the survivor's own value"
        );

        // The frozen scalar columns stay frozen either way.
        let survivor = db.get_author(user_id, survivor_id).await.expect("survivor");
        assert_eq!(survivor.ol_key, None);
        assert_eq!(survivor.gr_key, None);
    }
}
