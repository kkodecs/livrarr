//! SQLite persistence and activation control for the identity-v2 domain.
//! The module owns the new repository traits while legacy identity storage
//! remains available until the installation-wide authority marker is active.

use livrarr_domain::identity_layer::{
    CapturedIdentity, CoverPlaceholderState, CoverSlotPresentation, Edition,
    EditionEvidenceCommand, EditionEvidenceOutcome, EditionFormat, EditionId, EditionRepository,
    EditionRepositoryError, EditionState, EditionWorkEvidenceCommand,
    EmbeddedCoverInspectionOutcome, EmbeddedCoverInspectionRecord, EvidenceProvenance,
    FileRevision, IdentityAuthorityReadiness, IdentityCutoverService, IdentityEvidenceBundle,
    IdentityMigrationError, IdentityMigrationReport, IdentityProvider, IdentityRepositoryError,
    IdentityStatus, MachineSubtitleProjection, MintedReviewCard, PendingReviewCard,
    ResolveIdentityConflictCommand, ReviewActor, ReviewContinuationOutcome, ReviewKind,
    ReviewResolutionCommand, RouteKey, RouteKind, RouteOwner, RouteProvenance, SettlementCommit,
    SettlementCommitOutcome, SettlementReviewCard, SnapshotDatabase, WorkContributor,
    WorkCoverPresentation, WorkIdentityPresentation, WorkIdentityRepository, WorkRoute,
    WorkRouteState,
};
use livrarr_domain::{history_events, AuthorId, LibraryItemId, UserId, WorkId};
use sha2::{Digest, Sha256};
use sqlx::{QueryBuilder, Row, Sqlite, SqliteConnection, SqlitePool, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

use crate::sqlite::SqliteDb;

/// The active route fields that can change a provider's derived anchor set.
/// Provenance, observation time, and confirmation decorate a route but do not
/// change the graph the enrichment planner consumes.
pub(crate) type ActiveRouteGraph = Vec<(String, Option<i64>, Option<i64>, String, String, String)>;

pub(crate) async fn read_active_route_graph(
    conn: &mut SqliteConnection,
    user_id: UserId,
    work_id: WorkId,
) -> Result<ActiveRouteGraph, sqlx::Error> {
    sqlx::query_as(
        "SELECT owner_type, work_id, edition_id, provider, kind, provider_scoped_id \
           FROM identity_routes \
          WHERE user_id=?1 AND resolved_work_id=?2 AND state='active' \
          ORDER BY provider, kind, provider_scoped_id, owner_type, work_id, edition_id",
    )
    .bind(user_id)
    .bind(work_id)
    .fetch_all(conn)
    .await
}

/// Option B from fix round 18: standing is deliberately coarse (all providers
/// for the Work), but it is invalidated only when the active route graph has
/// actually changed. This keeps same-anchor `not_found` durable while making
/// every newly derived anchor fetchable on its next pass.
pub(crate) async fn invalidate_retry_state_if_route_graph_changed(
    conn: &mut SqliteConnection,
    user_id: UserId,
    work_id: WorkId,
    before: &ActiveRouteGraph,
) -> Result<bool, sqlx::Error> {
    let after = read_active_route_graph(conn, user_id, work_id).await?;
    if &after == before {
        return Ok(false);
    }
    sqlx::query("DELETE FROM provider_retry_state WHERE user_id=?1 AND work_id=?2")
        .bind(user_id)
        .bind(work_id)
        .execute(conn)
        .await?;
    Ok(true)
}

// ---------------------------------------------------------------------------
// Types (IR v1 `livrarr-db` public_api.types).
// ---------------------------------------------------------------------------

/// Declarative fixture labels used to seed deterministic legacy work groups.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LegacyWorkFixture {
    pub label: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LegacyIdentityRows {
    pub label: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LegacyBadgeRouteCase {
    pub label: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LegacyMonitoringFixture {
    pub label: String,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LegacyIdentityFixture {
    pub works_and_authors: Vec<LegacyWorkFixture>,
    pub legacy_routes_ledgers_and_reviews: LegacyIdentityRows,
    pub legacy_badge_route_matrix: Vec<LegacyBadgeRouteCase>,
    pub monitoring_flags: Vec<LegacyMonitoringFixture>,
}

/// Test-only access; the marker is guaranteed `NotRun`/inactive.
pub struct PreCutoverIdentityTestDb {
    pub db: SqliteDb,
    pub path: PathBuf,
    pub(crate) _tempdir: tempfile::TempDir,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IdentityRouteRow {
    pub user_id: UserId,
    pub work_id: Option<WorkId>,
    pub edition_id: Option<EditionId>,
    pub provider: String,
    pub kind: String,
    pub route_value: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorkContributorRoleRow {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub author_id: AuthorId,
    pub role: String,
    pub provenance: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorkSubjectRow {
    pub id: i64,
    pub user_id: UserId,
    pub work_id: WorkId,
    pub subject_kind: String,
    pub value: String,
    pub provenance: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EditionCoverCandidateRow {
    pub user_id: UserId,
    pub edition_id: EditionId,
    pub candidate_id: String,
    pub source: String,
    pub media_type: String,
    pub proxy_url: String,
    pub width: i64,
    pub height: i64,
    pub passes_quality_gate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct WorkCoverSelectionRow {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub format: String,
    pub candidate_id: Option<String>,
    pub source: Option<String>,
    pub fallback_from_format: Option<String>,
    pub provenance: Option<String>,
    pub computed_at_generation: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IdentityReviewCardRow {
    pub id: i64,
    pub user_id: UserId,
    pub work_id: Option<WorkId>,
    pub kind: String,
    pub generation: i64,
    pub status: String,
    pub payload: String,
    pub created_at: String,
    pub resolved_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IdentityRouteArchiveRow {
    pub id: i64,
    pub user_id: UserId,
    pub provider: String,
    pub kind: String,
    pub route_value: String,
    pub former_owner_type: String,
    pub former_owner_id: i64,
    pub reason: String,
    pub audit_id: Option<i64>,
    pub archived_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IdentityMergeArchiveRow {
    pub id: i64,
    pub user_id: UserId,
    pub winner_work_id: WorkId,
    pub loser_work_id: WorkId,
    pub preserved_fields: String,
    pub audit_id: Option<i64>,
    pub archived_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IdentityAuditEventRow {
    pub id: i64,
    pub user_id: UserId,
    pub work_id: Option<WorkId>,
    pub event_kind: String,
    pub actor: String,
    pub payload: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IdentityProviderAttemptRow {
    pub id: i64,
    pub user_id: UserId,
    pub work_id: WorkId,
    pub provider: String,
    pub route_kind: String,
    pub route_value: String,
    pub attempt_key: String,
    pub outcome: String,
    pub observed_at: String,
}

/// Installation-wide marker type: `IdentityCutoverRun.scope` carries no
/// per-user payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InstallationWide;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IdentityCutoverMode {
    Rehearsal,
    Apply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IdentityCutoverBranch {
    Snapshot,
    TriviallyEmpty,
}

/// Durable states for a persisted rehearsal or activation attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum IdentityCutoverRunStatus {
    Running,
    Blocked,
    Ready,
    Activated,
    Failed,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IdentityCutoverRun {
    pub id: i64,
    pub scope: InstallationWide,
    pub mode: IdentityCutoverMode,
    pub branch: IdentityCutoverBranch,
    pub source_fingerprint: [u8; 32],
    pub output_fingerprint: [u8; 32],
    pub status: IdentityCutoverRunStatus,
    pub report: IdentityMigrationReport,
}

/// One generation-claimed transfer of a typed route to a Work or Edition.
#[derive(Debug, Clone)]
pub struct TransferRouteCommand {
    pub user_id: UserId,
    pub route: RouteKey,
    pub target_owner: RouteOwner,
    pub expected_generation: i64,
}

/// Coherent subtitle, cover, and badge projections produced by one claim.
#[derive(Debug, Clone)]
pub struct WorkProjectionSnapshot {
    pub work_id: WorkId,
    pub subtitle: MachineSubtitleProjection,
    pub covers: WorkCoverPresentation,
    pub status: IdentityStatus,
    pub generation: i64,
}

// ---------------------------------------------------------------------------
// Domain trait impls on the existing `SqliteDb` (NEW shadow
// `WorkIdentityRepository`; `EditionRepository`; `IdentityCutoverService`).
// ---------------------------------------------------------------------------

impl WorkIdentityRepository for SqliteDb {
    async fn read_captured_identity(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<CapturedIdentity, IdentityRepositoryError> {
        let row = sqlx::query(
            "SELECT title, subtitle, identity_volume, \
                    CASE WHEN normalized_identity_main IS NULL \
                              OR trim(normalized_identity_main) = '' \
                              OR normalized_identity_main = '__UNMIGRATED__' \
                         THEN normalized_title ELSE normalized_identity_main END \
                        AS normalized_identity_main, \
                    COALESCE(normalized_identity_subtitle, '') AS normalized_identity_subtitle, \
                    COALESCE(normalized_identity_volume, '') AS normalized_identity_volume, \
                    COALESCE(primary_author_id, author_id, \
                        (SELECT a.id FROM authors a WHERE a.user_id = works.user_id \
                         AND lower(a.name) = lower(works.author_name) LIMIT 1)) \
                        AS primary_author_id, \
                    COALESCE(text_distinction, 'common') AS text_distinction, \
                    identity_status_v2, identity_generation, \
                    identity_title_provenance \
               FROM works WHERE user_id = ?1 AND id = ?2",
        )
        .bind(user_id)
        .bind(work_id)
        .fetch_optional(self.pool())
        .await
        .map_err(repo_db)?
        .ok_or(IdentityRepositoryError::NotFound)?;

        let provenance = row
            .try_get::<String, _>("identity_title_provenance")
            .ok()
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or(EvidenceProvenance::Migrated);
        let primary_author_id = row
            .try_get::<Option<i64>, _>("primary_author_id")
            .map_err(repo_decode)?
            .ok_or(IdentityRepositoryError::NotFound)?;
        let route_rows = sqlx::query(
            "SELECT id, user_id, owner_type, work_id, edition_id, resolved_work_id, \
                    provider, kind, provider_scoped_id, state, provenance, \
                    user_confirmed, observed_at \
               FROM identity_routes \
              WHERE user_id = ?1 AND resolved_work_id = ?2 AND state = 'active' \
              ORDER BY id",
        )
        .bind(user_id)
        .bind(work_id)
        .fetch_all(self.pool())
        .await
        .map_err(repo_db)?;
        let mut active_routes = Vec::with_capacity(route_rows.len());
        for route in route_rows {
            active_routes.push(decode_route(&route)?);
        }

        Ok(CapturedIdentity {
            user_id,
            own_work_id: work_id,
            identity_title: livrarr_domain::identity_layer::IdentityTitleTuple {
                main: row.try_get("title").map_err(repo_decode)?,
                subtitle: row.try_get("subtitle").map_err(repo_decode)?,
                volume: row.try_get("identity_volume").map_err(repo_decode)?,
                normalized_main: row
                    .try_get("normalized_identity_main")
                    .map_err(repo_decode)?,
                normalized_subtitle: row
                    .try_get("normalized_identity_subtitle")
                    .map_err(repo_decode)?,
                normalized_volume: row
                    .try_get("normalized_identity_volume")
                    .map_err(repo_decode)?,
                provenance,
            },
            primary_author_id,
            text_distinction: row.try_get("text_distinction").map_err(repo_decode)?,
            active_routes,
            status: decode_identity_status(
                row.try_get::<String, _>("identity_status_v2")
                    .map_err(repo_decode)?
                    .as_str(),
            )?,
            identity_generation: row.try_get("identity_generation").map_err(repo_decode)?,
        })
    }

    async fn read_identity_presentations(
        &self,
        user_id: UserId,
        work_ids: &[WorkId],
    ) -> Result<Vec<WorkIdentityPresentation>, IdentityRepositoryError> {
        if work_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut query = QueryBuilder::<Sqlite>::new(
            "SELECT id, identity_status_v2 FROM works WHERE user_id = ",
        );
        query.push_bind(user_id).push(" AND id IN (");
        {
            let mut separated = query.separated(", ");
            for work_id in work_ids {
                separated.push_bind(work_id);
            }
        }
        query.push(") ORDER BY id");
        let rows = query
            .build()
            .fetch_all(self.pool())
            .await
            .map_err(repo_db)?;
        let mut presentations = BTreeMap::new();
        for row in rows {
            let work_id = row.try_get("id").map_err(repo_decode)?;
            let status = decode_identity_status(
                row.try_get::<String, _>("identity_status_v2")
                    .map_err(repo_decode)?
                    .as_str(),
            )?;
            presentations.insert(
                work_id,
                WorkIdentityPresentation {
                    work_id,
                    status,
                    identifiers: Default::default(),
                },
            );
        }

        let mut route_query = QueryBuilder::<Sqlite>::new(
            "SELECT id, user_id, owner_type, work_id, edition_id, resolved_work_id, \
                    provider, kind, provider_scoped_id, state, provenance, \
                    user_confirmed, observed_at FROM identity_routes \
             WHERE user_id = ",
        );
        route_query
            .push_bind(user_id)
            .push(" AND state = 'active' AND resolved_work_id IN (");
        {
            let mut separated = route_query.separated(", ");
            for work_id in work_ids {
                separated.push_bind(work_id);
            }
        }
        route_query.push(") ORDER BY resolved_work_id, id");
        let route_rows = route_query
            .build()
            .fetch_all(self.pool())
            .await
            .map_err(repo_db)?;
        let mut routes_by_work: BTreeMap<WorkId, Vec<WorkRoute>> = BTreeMap::new();
        for row in route_rows {
            let route = decode_route(&row)?;
            routes_by_work
                .entry(route.resolved_work_id)
                .or_default()
                .push(route);
        }
        for (work_id, routes) in routes_by_work {
            if let Some(presentation) = presentations.get_mut(&work_id) {
                presentation.identifiers =
                    livrarr_domain::identity_layer::project_work_identifiers(&routes);
            }
        }
        Ok(presentations.into_values().collect())
    }

    async fn list_captured_identities_in_group(
        &self,
        user_id: UserId,
        normalized_main: String,
        primary_author_id: AuthorId,
    ) -> Result<Vec<CapturedIdentity>, IdentityRepositoryError> {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM works \
              WHERE user_id = ?1 AND normalized_identity_main = ?2 \
                AND primary_author_id = ?3 ORDER BY id",
        )
        .bind(user_id)
        .bind(normalized_main)
        .bind(primary_author_id)
        .fetch_all(self.pool())
        .await
        .map_err(repo_db)?;
        let mut identities = Vec::with_capacity(ids.len());
        for work_id in ids {
            identities.push(self.read_captured_identity(user_id, work_id).await?);
        }
        Ok(identities)
    }

    async fn read_primary_author_names(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<String>, IdentityRepositoryError> {
        let canonical: String =
            sqlx::query_scalar("SELECT name FROM authors WHERE user_id = ?1 AND id = ?2")
                .bind(user_id)
                .bind(author_id)
                .fetch_optional(self.pool())
                .await
                .map_err(repo_db)?
                .ok_or(IdentityRepositoryError::NotFound)?;
        let variants: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM author_name_variants \
              WHERE user_id = ?1 AND author_id = ?2 ORDER BY id",
        )
        .bind(user_id)
        .bind(author_id)
        .fetch_all(self.pool())
        .await
        .map_err(repo_db)?;
        let mut names = vec![canonical];
        for variant in variants {
            if !names.contains(&variant) {
                names.push(variant);
            }
        }
        Ok(names)
    }

    async fn commit_settlement(
        &self,
        command: SettlementCommit,
    ) -> Result<SettlementCommitOutcome, IdentityRepositoryError> {
        let primary = command
            .contributors
            .iter()
            .filter(|contributor| contributor.ordinal == 0)
            .collect::<Vec<_>>();
        if primary.len() != 1 {
            return Err(IdentityRepositoryError::AtomicRollback);
        }
        let primary_author_id = primary[0].author_id;
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(repo_db)?;
        commit_settlement_failpoint("begin")?;
        let route_graph_before = match command.existing_work_id {
            Some(work_id) => Some(
                read_active_route_graph(&mut tx, command.user_id, work_id)
                    .await
                    .map_err(repo_db)?,
            ),
            // A new Work cannot already own provider retry standing.
            None => None,
        };

        // An inline pending-route affirmation is a settle+resolve sequence.
        // Reject an already-owned candidate before the settlement mutates the
        // Work generation, writes its audit, or mints the continuation card.
        // Keeping this read inside the settlement transaction also closes the
        // window between the handler's informative owner preflight and the
        // first write claim.
        for card in &command.review_cards {
            let SettlementReviewCard::PendingRoute { candidate, .. } = card else {
                continue;
            };
            let provider = serde_json::to_string(&candidate.route.provider).map_err(repo_json)?;
            let kind = serde_json::to_string(&candidate.route.kind).map_err(repo_json)?;
            let owner: Option<i64> = sqlx::query_scalar(
                "SELECT resolved_work_id FROM identity_routes \
                  WHERE user_id=?1 AND provider=?2 AND kind=?3 \
                    AND provider_scoped_id=?4 AND state='active' LIMIT 1",
            )
            .bind(command.user_id)
            .bind(provider)
            .bind(kind)
            .bind(&candidate.route.value)
            .fetch_optional(&mut *tx)
            .await
            .map_err(repo_db)?;
            if owner.is_some_and(|owner| Some(owner) != command.existing_work_id) {
                return Err(IdentityRepositoryError::RouteOwnershipCollision);
            }
        }
        let author_name: String =
            sqlx::query_scalar("SELECT name FROM authors WHERE user_id = ?1 AND id = ?2")
                .bind(command.user_id)
                .bind(primary_author_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(repo_db)?
                .ok_or(IdentityRepositoryError::NotFound)?;

        let provenance = serde_json::to_string(&command.identity_title.provenance)
            .map_err(|error| IdentityRepositoryError::Database(error.to_string()))?;
        let (work_id, created, generation, birth_date) =
            if let Some(work_id) = command.existing_work_id {
                let current: Option<i64> = sqlx::query_scalar(
                    "SELECT identity_generation FROM works WHERE user_id = ?1 AND id = ?2",
                )
                .bind(command.user_id)
                .bind(work_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(repo_db)?;
                let current = current.ok_or(IdentityRepositoryError::NotFound)?;
                if current != command.expected_generation {
                    return Err(IdentityRepositoryError::StaleGeneration);
                }
                let generation = current + 1;
                let result = sqlx::query(
                    "UPDATE works \
                    SET title = ?1, subtitle = ?2, author_name = ?3, author_id = ?4, \
                        normalized_title = ?5, normalized_author = ?6, \
                        normalized_identity_main = ?5, normalized_identity_subtitle = ?7, \
                        normalized_identity_volume = ?8, primary_author_id = ?4, \
                        identity_status_v2 = 'not_connected', identity_generation = ?9, \
                        identity_title_provenance = ?10, identity_volume = ?11, \
                        text_distinction = COALESCE(?12, text_distinction) \
                  WHERE user_id = ?13 AND id = ?14 AND identity_generation = ?15",
                )
                .bind(&command.identity_title.main)
                .bind(&command.identity_title.subtitle)
                .bind(&author_name)
                .bind(primary_author_id)
                .bind(&command.identity_title.normalized_main)
                .bind(author_name.to_lowercase())
                .bind(&command.identity_title.normalized_subtitle)
                .bind(&command.identity_title.normalized_volume)
                .bind(generation)
                .bind(&provenance)
                .bind(&command.identity_title.volume)
                .bind(&command.text_distinction)
                .bind(command.user_id)
                .bind(work_id)
                .bind(current)
                .execute(&mut *tx)
                .await
                .map_err(map_settlement_sql)?;
                if result.rows_affected() != 1 {
                    return Err(IdentityRepositoryError::StaleGeneration);
                }
                (work_id, false, generation, None)
            } else {
                if command.expected_generation != 0 {
                    return Err(IdentityRepositoryError::StaleGeneration);
                }
                let added_at = chrono::Utc::now().to_rfc3339();
                let result = sqlx::query(
                    "INSERT INTO works \
                    (user_id, title, subtitle, author_name, author_id, normalized_title, \
                     normalized_author, added_at, normalized_identity_main, \
                     normalized_identity_subtitle, normalized_identity_volume, \
                     text_distinction, identity_status_v2, primary_author_id, \
                     identity_generation, identity_title_provenance, identity_volume) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?6, ?9, ?10, \
                         ?11, 'not_connected', ?5, 1, ?12, ?13)",
                )
                .bind(command.user_id)
                .bind(&command.identity_title.main)
                .bind(&command.identity_title.subtitle)
                .bind(&author_name)
                .bind(primary_author_id)
                .bind(&command.identity_title.normalized_main)
                .bind(author_name.to_lowercase())
                .bind(&added_at)
                .bind(&command.identity_title.normalized_subtitle)
                .bind(&command.identity_title.normalized_volume)
                .bind(command.text_distinction.as_deref().unwrap_or("common"))
                .bind(&provenance)
                .bind(&command.identity_title.volume)
                .execute(&mut *tx)
                .await
                .map_err(map_settlement_sql)?;
                (result.last_insert_rowid(), true, 1, Some(added_at))
            };
        commit_settlement_failpoint("work")?;

        // The v2 INSERT above is the Work's moment of truth. Its birth fact
        // participates in this same transaction so a committed Work can never
        // exist without its one live `added` event. Payload construction stays
        // at the typed domain chokepoint used by every other history writer.
        if created {
            if let (Some(source), Some(date)) = (command.add_source, birth_date.as_deref()) {
                let draft = history_events::added(
                    work_id,
                    &command.identity_title.main,
                    Some(&author_name),
                    source,
                );
                sqlx::query(
                    "INSERT INTO history (user_id, work_id, event_type, data, date) \
                     VALUES (?1, ?2, 'added', ?3, ?4)",
                )
                .bind(command.user_id)
                .bind(work_id)
                .bind(serde_json::to_string(&draft.data).map_err(repo_json)?)
                .bind(date)
                .execute(&mut *tx)
                .await
                .map_err(repo_db)?;
            }
        }

        for loser_work_id in command
            .absorbed_work_ids
            .iter()
            .copied()
            .filter(|loser| *loser != work_id)
        {
            absorb_work_into(&mut tx, command.user_id, work_id, loser_work_id).await?;
        }
        merge_contributors(&mut tx, command.user_id, work_id, &command.contributors).await?;
        commit_settlement_failpoint("contributors")?;

        for route in &command.routes {
            let mut route = route.clone();
            materialize_edition_route_owner(&mut tx, command.user_id, work_id, &mut route).await?;
            insert_route(&mut tx, command.user_id, work_id, &route).await?;
        }
        cancel_satisfied_pending_route_cards(&mut tx, command.user_id, work_id, &command.routes)
            .await?;
        let route_state = sqlx::query(
            "SELECT COUNT(*) AS route_count, COALESCE(MAX(user_confirmed), 0) AS confirmed \
               FROM identity_routes \
              WHERE user_id = ?1 AND resolved_work_id = ?2 AND state = 'active'",
        )
        .bind(command.user_id)
        .bind(work_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_db)?;
        let status = if route_state
            .try_get::<i64, _>("confirmed")
            .map_err(repo_decode)?
            > 0
        {
            IdentityStatus::UserConfirmed
        } else if route_state
            .try_get::<i64, _>("route_count")
            .map_err(repo_decode)?
            > 0
        {
            IdentityStatus::Connected
        } else {
            IdentityStatus::NotConnected
        };
        sqlx::query("UPDATE works SET identity_status_v2 = ?1 WHERE user_id = ?2 AND id = ?3")
            .bind(encode_identity_status(status))
            .bind(command.user_id)
            .bind(work_id)
            .execute(&mut *tx)
            .await
            .map_err(repo_db)?;
        if let Some(before) = route_graph_before.as_ref() {
            invalidate_retry_state_if_route_graph_changed(
                &mut tx,
                command.user_id,
                work_id,
                before,
            )
            .await
            .map_err(repo_db)?;
        }
        commit_settlement_failpoint("routes")?;

        let search_fallback_kinds: BTreeSet<String> = command
            .routes
            .iter()
            .filter_map(|route| match &route.provenance {
                RouteProvenance::SearchFallback {
                    corroborating_kind, ..
                } => Some(format!("{corroborating_kind:?}")),
                _ => None,
            })
            .collect();
        let has_text_decisive_search = command.routes.iter().any(|route| {
            matches!(
                route.provenance,
                RouteProvenance::TextDecisiveSearchFallback { .. }
            )
        });
        let audit_payload = if search_fallback_kinds.is_empty() && !has_text_decisive_search {
            format!("generation={generation}")
        } else if search_fallback_kinds.is_empty() {
            format!("generation={generation};origin=search-fallback;basis=TEXT-DECISIVE")
        } else {
            let mut payload = format!(
                "generation={generation};origin=search-fallback;corroborating_kind={}",
                search_fallback_kinds
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(",")
            );
            if has_text_decisive_search {
                payload.push_str(";additional_basis=TEXT-DECISIVE");
            }
            payload
        };
        let audit = sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'settlement', 'identity-engine', ?3, ?4)",
        )
        .bind(command.user_id)
        .bind(work_id)
        .bind(audit_payload)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        let audit_id = audit.last_insert_rowid();
        let mut review_cards = Vec::with_capacity(command.review_cards.len());
        for card in &command.review_cards {
            let kind = card.kind();
            if let Some(proposal_key) = group_identity_proposal_key(card)? {
                let pending = sqlx::query(
                    "SELECT id, payload FROM identity_review_cards \
                      WHERE user_id=?1 AND kind=?2 AND status='pending' ORDER BY id",
                )
                .bind(command.user_id)
                .bind(ReviewKind::GroupIdentity.storage_code())
                .fetch_all(&mut *tx)
                .await
                .map_err(repo_db)?;
                let mut reusable_id = None;
                for row in pending {
                    let card_id: i64 = row.try_get("id").map_err(repo_decode)?;
                    let payload: String = row.try_get("payload").map_err(repo_decode)?;
                    let pending_card: SettlementReviewCard =
                        serde_json::from_str(&payload).map_err(repo_json)?;
                    if group_identity_proposal_key(&pending_card)?.as_ref() == Some(&proposal_key) {
                        reusable_id = Some(card_id);
                        break;
                    }
                }
                if let Some(id) = reusable_id {
                    review_cards.push(MintedReviewCard {
                        id,
                        kind,
                        generation,
                    });
                    continue;
                }
            }
            let row = sqlx::query(
                "INSERT INTO identity_review_cards \
                    (user_id, work_id, kind, generation, status, payload, created_at) \
                 VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6)",
            )
            .bind(command.user_id)
            .bind(
                review_card_work_id(card)
                    .filter(|candidate| *candidate > 0)
                    .or(Some(work_id)),
            )
            .bind(kind.storage_code())
            .bind(generation)
            .bind(serde_json::to_string(card).map_err(repo_json)?)
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&mut *tx)
            .await
            .map_err(repo_db)?;
            review_cards.push(MintedReviewCard {
                id: row.last_insert_rowid(),
                kind,
                generation,
            });
        }
        commit_settlement_failpoint("reviews")?;
        tx.commit().await.map_err(repo_db)?;

        let identity = self
            .read_captured_identity(command.user_id, work_id)
            .await?;
        Ok(SettlementCommitOutcome {
            identity,
            created,
            audit_id,
            review_cards,
        })
    }

    async fn commit_unattached_import_review(
        &self,
        user_id: UserId,
        evidence: IdentityEvidenceBundle,
    ) -> Result<MintedReviewCard, IdentityRepositoryError> {
        let card = SettlementReviewCard::ImportIdentity {
            work_id: None,
            evidence,
        };
        let kind = card.kind();
        let kind_code = kind.storage_code();
        let payload = serde_json::to_string(&card).map_err(repo_json)?;
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(repo_db)?;
        let existing = sqlx::query(
            "SELECT id, generation FROM identity_review_cards \
              WHERE user_id = ?1 AND work_id IS NULL AND kind = ?2 \
                AND status = 'pending' AND payload = ?3 ORDER BY id LIMIT 1",
        )
        .bind(user_id)
        .bind(kind_code)
        .bind(&payload)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_db)?;
        if let Some(row) = existing {
            return Ok(MintedReviewCard {
                id: row.try_get("id").map_err(repo_decode)?,
                kind,
                generation: row.try_get("generation").map_err(repo_decode)?,
            });
        }
        let created_at = chrono::Utc::now().to_rfc3339();
        let inserted = sqlx::query(
            "INSERT INTO identity_review_cards \
                (user_id, work_id, kind, generation, status, payload, created_at) \
             VALUES (?1, NULL, ?2, 0, 'pending', ?3, ?4)",
        )
        .bind(user_id)
        .bind(kind_code)
        .bind(&payload)
        .bind(&created_at)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        let card_id = inserted.last_insert_rowid();
        sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, NULL, 'unattached-import-review', 'identity-road', ?2, ?3)",
        )
        .bind(user_id)
        .bind(format!("card_id={card_id}"))
        .bind(created_at)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        tx.commit().await.map_err(repo_db)?;
        Ok(MintedReviewCard {
            id: card_id,
            kind,
            generation: 0,
        })
    }

    async fn commit_pending_route_review(
        &self,
        user_id: UserId,
        work_id: WorkId,
        expected_generation: i64,
        candidate: livrarr_domain::identity_layer::ParkedRouteCandidate,
    ) -> Result<MintedReviewCard, IdentityRepositoryError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(repo_db)?;
        let current: Option<(i64, String, String)> = sqlx::query_as(
            "SELECT identity_generation, title, author_name \
               FROM works WHERE user_id=?1 AND id=?2",
        )
        .bind(user_id)
        .bind(work_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_db)?;
        let (current_generation, work_title, work_author) =
            current.ok_or(IdentityRepositoryError::NotFound)?;
        if current_generation != expected_generation {
            return Err(IdentityRepositoryError::StaleGeneration);
        }
        let card = SettlementReviewCard::PendingRoute { work_id, candidate };
        let proposal_key =
            pending_route_proposal_key(&card).ok_or(IdentityRepositoryError::InvalidResolution)?;
        let pending = sqlx::query(
            "SELECT id, generation, payload FROM identity_review_cards \
              WHERE user_id=?1 AND work_id=?2 AND kind=?3 AND status='pending' ORDER BY id",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(ReviewKind::PendingRoute.storage_code())
        .fetch_all(&mut *tx)
        .await
        .map_err(repo_db)?;
        for row in pending {
            let payload: String = row.try_get("payload").map_err(repo_decode)?;
            let pending_card: SettlementReviewCard =
                serde_json::from_str(&payload).map_err(repo_json)?;
            if pending_route_proposal_key(&pending_card).as_ref() == Some(&proposal_key) {
                return Ok(MintedReviewCard {
                    id: row.try_get("id").map_err(repo_decode)?,
                    kind: ReviewKind::PendingRoute,
                    // The row keeps its mint generation as history; callers
                    // receive the generation that is actionable now.
                    generation: expected_generation,
                });
            }
        }
        let created_at = chrono::Utc::now().to_rfc3339();
        let inserted = sqlx::query(
            "INSERT INTO identity_review_cards \
                (user_id, work_id, kind, generation, status, payload, created_at) \
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6)",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(ReviewKind::PendingRoute.storage_code())
        .bind(expected_generation)
        .bind(serde_json::to_string(&card).map_err(repo_json)?)
        .bind(&created_at)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        let card_id = inserted.last_insert_rowid();
        let SettlementReviewCard::PendingRoute { candidate, .. } = &card else {
            unreachable!("commit_pending_route_review always constructs PendingRoute")
        };
        let provider_name = review_notification_provider_name(&candidate.route.provider);
        let message = format!(
            "Review needed: link '{work_title}' — a possible {provider_name} match was found"
        );
        sqlx::query(
            "INSERT INTO notifications \
                (user_id, type, ref_key, message, data, created_at) \
             VALUES (?1, 'identityReviewNeeded', ?2, ?3, ?4, ?5)",
        )
        .bind(user_id)
        .bind(format!("identity-review-card:{card_id}"))
        .bind(message)
        .bind(
            serde_json::to_string(&serde_json::json!({
                "cardId": card_id,
                "workId": work_id,
                "title": work_title,
                "author": work_author,
                "provider": provider_name,
            }))
            .map_err(repo_json)?,
        )
        .bind(&created_at)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'pending-route-proposal', 'identity-road', ?3, ?4)",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(format!("card_id={card_id};origin=search-fallback"))
        .bind(created_at)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        tx.commit().await.map_err(repo_db)?;
        Ok(MintedReviewCard {
            id: card_id,
            kind: ReviewKind::PendingRoute,
            generation: expected_generation,
        })
    }

    async fn load_pending_review(
        &self,
        actor: ReviewActor,
        card_id: i64,
    ) -> Result<PendingReviewCard, IdentityRepositoryError> {
        let row = sqlx::query(
            "SELECT c.id, c.user_id, c.work_id, c.kind, c.payload, \
                    w.title AS work_title, w.author_name AS work_author, \
                    CASE WHEN c.kind IN ('GroupIdentity', 'PendingRoute') AND w.id IS NOT NULL \
                         THEN w.identity_generation ELSE c.generation END AS generation \
               FROM identity_review_cards c \
               LEFT JOIN works w ON w.user_id = c.user_id AND w.id = c.work_id \
              WHERE c.id = ?1 AND c.status = 'pending'",
        )
        .bind(card_id)
        .fetch_optional(self.pool())
        .await
        .map_err(repo_db)?
        .ok_or(IdentityRepositoryError::NotFound)?;
        let pending = decode_pending_review(&row)?;
        authorize_review_actor(&actor, pending.user_id)?;
        Ok(pending)
    }

    async fn list_pending_reviews(
        &self,
        actor: ReviewActor,
    ) -> Result<Vec<PendingReviewCard>, IdentityRepositoryError> {
        let rows = match &actor {
            ReviewActor::AuthenticatedUser { user_id } => sqlx::query(
                "SELECT c.id, c.user_id, c.work_id, c.kind, c.payload, \
                        w.title AS work_title, w.author_name AS work_author, \
                        CASE WHEN c.kind IN ('GroupIdentity', 'PendingRoute') AND w.id IS NOT NULL \
                             THEN w.identity_generation ELSE c.generation END AS generation \
                   FROM identity_review_cards c \
                   LEFT JOIN works w ON w.user_id = c.user_id AND w.id = c.work_id \
                  WHERE c.user_id = ?1 AND c.status = 'pending' ORDER BY c.id",
            )
            .bind(*user_id)
            .fetch_all(self.pool())
            .await
            .map_err(repo_db)?,
            ReviewActor::CutoverOperator { .. } => sqlx::query(
                "SELECT c.id, c.user_id, c.work_id, c.kind, c.payload, \
                        w.title AS work_title, w.author_name AS work_author, \
                        CASE WHEN c.kind IN ('GroupIdentity', 'PendingRoute') AND w.id IS NOT NULL \
                             THEN w.identity_generation ELSE c.generation END AS generation \
                   FROM identity_review_cards c \
                   LEFT JOIN works w ON w.user_id = c.user_id AND w.id = c.work_id \
                  WHERE c.status = 'pending' ORDER BY c.id",
            )
            .fetch_all(self.pool())
            .await
            .map_err(repo_db)?,
        };
        rows.into_iter()
            .map(|row| {
                let pending = decode_pending_review(&row)?;
                authorize_review_actor(&actor, pending.user_id)?;
                Ok(pending)
            })
            .collect()
    }

    async fn dismiss_pending_review(
        &self,
        actor: ReviewActor,
        card_id: i64,
    ) -> Result<(), IdentityRepositoryError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(repo_db)?;
        let row = sqlx::query(
            "SELECT c.id, c.user_id, c.work_id, c.kind, c.generation, c.payload, \
                    w.title AS work_title, w.author_name AS work_author \
               FROM identity_review_cards c \
               LEFT JOIN works w ON w.user_id = c.user_id AND w.id = c.work_id \
              WHERE c.id = ?1 AND c.status = 'pending'",
        )
        .bind(card_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_db)?
        .ok_or(IdentityRepositoryError::NotFound)?;
        let pending = decode_pending_review(&row)?;
        authorize_review_actor(&actor, pending.user_id)?;
        let now = chrono::Utc::now().to_rfc3339();
        let updated = sqlx::query(
            "UPDATE identity_review_cards SET status='cancelled', resolved_at=?1 \
              WHERE id=?2 AND user_id=?3 AND status='pending'",
        )
        .bind(&now)
        .bind(card_id)
        .bind(pending.user_id)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        if updated.rows_affected() != 1 {
            return Err(IdentityRepositoryError::NotFound);
        }
        sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'review-dismissal', ?3, ?4, ?5)",
        )
        .bind(pending.user_id)
        .bind(pending.work_id)
        .bind(serde_json::to_string(&actor).map_err(repo_json)?)
        .bind(format!(
            "card_id={card_id};kind={}",
            pending.kind.storage_code()
        ))
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        tx.commit().await.map_err(repo_db)?;
        Ok(())
    }

    async fn load_pending_conflict_review(
        &self,
        actor: ReviewActor,
        conflict_id: i64,
    ) -> Result<PendingReviewCard, IdentityRepositoryError> {
        let kind = ReviewKind::IdentityConflict.storage_code();
        let rows = match &actor {
            ReviewActor::AuthenticatedUser { user_id } => sqlx::query(
                "SELECT c.id, c.user_id, c.work_id, c.kind, c.generation, c.payload, \
                        w.title AS work_title, w.author_name AS work_author \
                   FROM identity_review_cards c \
                   LEFT JOIN works w ON w.user_id = c.user_id AND w.id = c.work_id \
                  WHERE c.user_id = ?1 AND c.status = 'pending' AND c.kind = ?2 ORDER BY c.id",
            )
            .bind(*user_id)
            .bind(kind)
            .fetch_all(self.pool())
            .await
            .map_err(repo_db)?,
            ReviewActor::CutoverOperator { .. } => sqlx::query(
                "SELECT c.id, c.user_id, c.work_id, c.kind, c.generation, c.payload, \
                        w.title AS work_title, w.author_name AS work_author \
                   FROM identity_review_cards c \
                   LEFT JOIN works w ON w.user_id = c.user_id AND w.id = c.work_id \
                  WHERE c.status = 'pending' AND c.kind = ?1 ORDER BY c.id",
            )
            .bind(kind)
            .fetch_all(self.pool())
            .await
            .map_err(repo_db)?,
        };
        for row in rows {
            let pending = decode_pending_review(&row)?;
            if matches!(
                pending.payload,
                SettlementReviewCard::IdentityConflict {
                    conflict_id: candidate,
                    ..
                } if candidate == conflict_id
            ) {
                authorize_review_actor(&actor, pending.user_id)?;
                return Ok(pending);
            }
        }
        Err(IdentityRepositoryError::NotFound)
    }

    async fn commit_review_continuation(
        &self,
        actor: ReviewActor,
        command: ReviewResolutionCommand,
        cancel: CancellationToken,
    ) -> Result<ReviewContinuationOutcome, IdentityRepositoryError> {
        if cancel.is_cancelled() {
            return Err(IdentityRepositoryError::Cancelled);
        }
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(repo_db)?;
        let row = sqlx::query(
            "SELECT c.id, c.user_id, c.work_id, c.kind, c.payload, \
                    w.title AS work_title, w.author_name AS work_author, \
                    CASE WHEN c.kind IN ('GroupIdentity', 'PendingRoute') AND w.id IS NOT NULL \
                         THEN w.identity_generation ELSE c.generation END AS generation \
               FROM identity_review_cards c \
               LEFT JOIN works w ON w.user_id = c.user_id AND w.id = c.work_id \
              WHERE c.id = ?1 AND c.status = 'pending'",
        )
        .bind(command.card_id())
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_db)?
        .ok_or(IdentityRepositoryError::NotFound)?;
        let pending = decode_pending_review(&row)?;
        authorize_review_actor(&actor, pending.user_id)?;
        if command.kind() != pending.kind {
            return Err(IdentityRepositoryError::ReviewKindMismatch);
        }
        let is_pending_route = pending.kind == ReviewKind::PendingRoute;
        if is_pending_route {
            let work_exists = match pending.work_id {
                Some(work_id) => sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS(SELECT 1 FROM works WHERE user_id=?1 AND id=?2)",
                )
                .bind(pending.user_id)
                .bind(work_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(repo_db)?,
                None => false,
            };
            if !work_exists {
                return Err(IdentityRepositoryError::ReviewProposalInvalidated(
                    "work no longer exists".to_string(),
                ));
            }
        } else if command.expected_generation() != pending.generation {
            return Err(IdentityRepositoryError::StaleGeneration);
        }
        validate_review_resolution(&pending, &command)?;
        validate_group_identity_proposal(&mut tx, &pending, &command).await?;
        if cancel.is_cancelled() {
            return Err(IdentityRepositoryError::Cancelled);
        }
        let route_graph_before = match pending.work_id {
            Some(work_id) => Some(
                read_active_route_graph(&mut tx, pending.user_id, work_id)
                    .await
                    .map_err(repo_db)?,
            ),
            None => None,
        };

        let mut pending_route_noop = false;
        if let (
            Some(work_id),
            SettlementReviewCard::PendingRoute { candidate, .. },
            ReviewResolutionCommand::PendingRoute { .. },
        ) = (pending.work_id, &pending.payload, &command)
        {
            let provider = serde_json::to_string(&candidate.route.provider).map_err(repo_json)?;
            let kind = serde_json::to_string(&candidate.route.kind).map_err(repo_json)?;
            let current_owner: Option<i64> = sqlx::query_scalar(
                "SELECT resolved_work_id FROM identity_routes \
                  WHERE user_id=?1 AND provider=?2 AND kind=?3 \
                    AND provider_scoped_id=?4 AND state='active' LIMIT 1",
            )
            .bind(pending.user_id)
            .bind(provider)
            .bind(kind)
            .bind(candidate.route.value.trim())
            .fetch_optional(&mut *tx)
            .await
            .map_err(repo_db)?;
            if current_owner.is_some_and(|owner| owner != work_id) {
                return Err(IdentityRepositoryError::ReviewProposalInvalidated(
                    "proposed route is now owned by a different work".to_string(),
                ));
            }
            pending_route_noop = current_owner == Some(work_id);
        }

        let mut result_work_id = pending.work_id;
        let mut moved = AbsorptionCounts::default();
        if let (
            Some(work_id),
            SettlementReviewCard::GroupIdentity {
                work_ids,
                proposed_identity,
                merge_choices,
            },
            ReviewResolutionCommand::GroupIdentity { action, .. },
        ) = (pending.work_id, &pending.payload, &command)
        {
            match action {
                livrarr_domain::identity_layer::GroupIdentityAction::DifferentFromAll => {
                    if let Some(proposed) = proposed_identity.as_ref() {
                        let author_name: String = sqlx::query_scalar(
                            "SELECT name FROM authors WHERE user_id = ?1 AND id = ?2",
                        )
                        .bind(pending.user_id)
                        .bind(proposed.primary_author_id)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(repo_db)?
                        .ok_or(IdentityRepositoryError::NotFound)?;
                        let provenance =
                            serde_json::to_string(&EvidenceProvenance::User).map_err(repo_json)?;
                        sqlx::query(
                        "UPDATE works SET title = ?1, subtitle = ?2, identity_volume = ?3, \
                                normalized_title = ?4, normalized_identity_main = ?4, \
                                normalized_identity_subtitle = ?5, normalized_identity_volume = ?6, \
                                author_name = ?7, author_id = ?8, primary_author_id = ?8, \
                                normalized_author = ?9, identity_title_provenance = ?10, \
                                text_distinction = ?11 \
                          WHERE user_id = ?12 AND id = ?13",
                    )
                    .bind(&proposed.title.main)
                    .bind(&proposed.title.subtitle)
                    .bind(&proposed.title.volume)
                    .bind(&proposed.title.normalized_main)
                    .bind(&proposed.title.normalized_subtitle)
                    .bind(&proposed.title.normalized_volume)
                    .bind(&author_name)
                    .bind(proposed.primary_author_id)
                    .bind(author_name.to_lowercase())
                    .bind(provenance)
                    .bind(format!("different:review:{}", pending.id))
                    .bind(pending.user_id)
                    .bind(work_id)
                    .execute(&mut *tx)
                    .await
                        .map_err(map_settlement_sql)?;
                        merge_contributors(
                            &mut tx,
                            pending.user_id,
                            work_id,
                            &[WorkContributor {
                                user_id: pending.user_id,
                                work_id,
                                author_id: proposed.primary_author_id,
                                ordinal: 0,
                                roles: Vec::new(),
                            }],
                        )
                        .await?;
                        for route in &proposed.routes {
                            let mut route = route.clone();
                            materialize_edition_route_owner(
                                &mut tx,
                                pending.user_id,
                                work_id,
                                &mut route,
                            )
                            .await?;
                            insert_route(&mut tx, pending.user_id, work_id, &route).await?;
                        }
                        let confirmed: bool = sqlx::query_scalar(
                            "SELECT EXISTS(SELECT 1 FROM identity_routes \
                              WHERE user_id=?1 AND resolved_work_id=?2 \
                                AND state='active' AND user_confirmed=1)",
                        )
                        .bind(pending.user_id)
                        .bind(work_id)
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(repo_db)?;
                        let connected: bool = sqlx::query_scalar(
                            "SELECT EXISTS(SELECT 1 FROM identity_routes \
                              WHERE user_id=?1 AND resolved_work_id=?2 AND state='active')",
                        )
                        .bind(pending.user_id)
                        .bind(work_id)
                        .fetch_one(&mut *tx)
                        .await
                        .map_err(repo_db)?;
                        let status = if confirmed {
                            "user_confirmed"
                        } else if connected {
                            "connected"
                        } else {
                            "not_connected"
                        };
                        sqlx::query(
                            "UPDATE works SET identity_status_v2=?1 WHERE user_id=?2 AND id=?3",
                        )
                        .bind(status)
                        .bind(pending.user_id)
                        .bind(work_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(repo_db)?;
                    } else {
                        sqlx::query(
                            "UPDATE works SET text_distinction = ?1 \
                              WHERE user_id = ?2 AND id = ?3",
                        )
                        .bind(format!("different:review:{}", pending.id))
                        .bind(pending.user_id)
                        .bind(work_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(map_settlement_sql)?;
                    }
                }
                livrarr_domain::identity_layer::GroupIdentityAction::AttachOrMerge { anchor } => {
                    if !work_ids.contains(anchor) || *anchor != work_id {
                        return Err(IdentityRepositoryError::InvalidResolution);
                    }
                    apply_manual_merge_fields(
                        &mut tx,
                        pending.user_id,
                        *anchor,
                        work_ids,
                        merge_choices,
                    )
                    .await?;
                    for loser in work_ids.iter().copied().filter(|id| id != anchor) {
                        moved += absorb_work_into(&mut tx, pending.user_id, *anchor, loser).await?;
                    }
                    result_work_id = Some(*anchor);
                }
            }
        }

        if !pending_route_noop {
            if let (
                Some(work_id),
                SettlementReviewCard::PendingRoute { candidate, .. },
                ReviewResolutionCommand::PendingRoute { .. },
            ) = (pending.work_id, &pending.payload, &command)
            {
                let owner = match candidate.proposed_owner {
                    RouteOwner::Work(id) if id > 0 => RouteOwner::Work(id),
                    RouteOwner::Work(_) => RouteOwner::Work(work_id),
                    RouteOwner::Edition(id) => RouteOwner::Edition(id),
                };
                let route = WorkRoute {
                    id: 0,
                    user_id: pending.user_id,
                    owner,
                    resolved_work_id: work_id,
                    provider: candidate.route.provider.clone(),
                    kind: candidate.route.kind.clone(),
                    provider_scoped_id: candidate.route.value.clone(),
                    state: WorkRouteState::Active,
                    provenance: RouteProvenance::UserChoice,
                    user_confirmed: true,
                    observed_at: chrono::Utc::now(),
                };
                insert_route(&mut tx, pending.user_id, work_id, &route).await?;
                if let Some((anchor_type, column)) = legacy_route_slot(&candidate.route.kind) {
                    let sql =
                        format!("UPDATE works SET {column} = ?1 WHERE user_id = ?2 AND id = ?3");
                    sqlx::query(&sql)
                        .bind(&candidate.route.value)
                        .bind(pending.user_id)
                        .bind(work_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(repo_db)?;
                    sqlx::query(
                        "INSERT INTO work_identity_anchors \
                        (work_id, anchor_type, anchor_value, confidence, setter, set_at, user_id) \
                     VALUES (?1, ?2, ?3, 'confirmed', 'user', ?4, ?5) \
                     ON CONFLICT(work_id, anchor_type, anchor_value) DO UPDATE SET \
                         confidence = 'confirmed', setter = 'user', set_at = excluded.set_at",
                    )
                    .bind(work_id)
                    .bind(anchor_type)
                    .bind(&candidate.route.value)
                    .bind(chrono::Utc::now().to_rfc3339())
                    .bind(pending.user_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(repo_db)?;
                }
                sqlx::query(
                    "UPDATE works SET identity_status_v2 = 'user_confirmed' \
                  WHERE user_id = ?1 AND id = ?2",
                )
                .bind(pending.user_id)
                .bind(work_id)
                .execute(&mut *tx)
                .await
                .map_err(repo_db)?;
            }
        }

        let generation = if pending_route_noop {
            pending.generation
        } else if let Some(work_id) = result_work_id {
            let updated = sqlx::query(
                "UPDATE works SET identity_generation = identity_generation + 1 \
                  WHERE user_id = ?1 AND id = ?2 AND identity_generation = ?3",
            )
            .bind(pending.user_id)
            .bind(work_id)
            .bind(pending.generation)
            .execute(&mut *tx)
            .await
            .map_err(repo_db)?;
            if updated.rows_affected() != 1 {
                return Err(IdentityRepositoryError::StaleGeneration);
            }
            pending.generation + 1
        } else {
            pending.generation
        };
        if let (Some(work_id), Some(before)) = (result_work_id, route_graph_before.as_ref()) {
            invalidate_retry_state_if_route_graph_changed(
                &mut tx,
                pending.user_id,
                work_id,
                before,
            )
            .await
            .map_err(repo_db)?;
        }
        let now = chrono::Utc::now().to_rfc3339();
        let audit = sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'review-resolution', ?3, ?4, ?5)",
        )
        .bind(pending.user_id)
        .bind(pending.work_id)
        .bind(serde_json::to_string(&actor).map_err(repo_json)?)
        .bind(serde_json::to_string(&command).map_err(repo_json)?)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        let audit_id = audit.last_insert_rowid();
        let resolved = sqlx::query(
            "UPDATE identity_review_cards SET status = 'resolved', resolved_at = ?1 \
              WHERE id = ?2 AND user_id = ?3 AND status = 'pending'",
        )
        .bind(&now)
        .bind(pending.id)
        .bind(pending.user_id)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        if resolved.rows_affected() != 1 {
            return Err(IdentityRepositoryError::NotFound);
        }
        if cancel.is_cancelled() {
            return Err(IdentityRepositoryError::Cancelled);
        }
        tx.commit().await.map_err(repo_db)?;
        let identity = match result_work_id {
            Some(work_id) => Some(
                self.read_captured_identity(pending.user_id, work_id)
                    .await?,
            ),
            None => None,
        };
        Ok(ReviewContinuationOutcome {
            card_id: pending.id,
            kind: pending.kind,
            generation,
            audit_id,
            identity,
            library_items_moved: moved.library_items,
            grabs_moved: moved.grabs,
        })
    }

    async fn resolve_conflict_atomically(
        &self,
        command: ResolveIdentityConflictCommand,
    ) -> Result<CapturedIdentity, IdentityRepositoryError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(repo_db)?;
        let row = sqlx::query(
            "SELECT current_work_id, expected_generation, status, candidate_kind, \
                    proposed_owner_type, proposed_owner_id \
               FROM identity_conflicts_v2 WHERE user_id = ?1 AND id = ?2",
        )
        .bind(command.user_id)
        .bind(command.conflict_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_db)?
        .ok_or(IdentityRepositoryError::NotFound)?;
        let work_id: i64 = row.try_get("current_work_id").map_err(repo_decode)?;
        let recorded_generation: i64 = row.try_get("expected_generation").map_err(repo_decode)?;
        let status: String = row.try_get("status").map_err(repo_decode)?;
        if status != "pending" || recorded_generation != command.expected_generation {
            return Err(IdentityRepositoryError::StaleGeneration);
        }
        let work_generation: Option<i64> = sqlx::query_scalar(
            "SELECT identity_generation FROM works WHERE user_id = ?1 AND id = ?2",
        )
        .bind(command.user_id)
        .bind(work_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_db)?;
        if work_generation != Some(command.expected_generation) {
            return Err(IdentityRepositoryError::StaleGeneration);
        }
        let candidate_kind: livrarr_domain::identity_layer::RouteKind = serde_json::from_str(
            &row.try_get::<String, _>("candidate_kind")
                .map_err(repo_decode)?,
        )
        .map_err(repo_json)?;
        let edition_scoped = matches!(
            candidate_kind,
            livrarr_domain::identity_layer::RouteKind::Isbn13Edition
                | livrarr_domain::identity_layer::RouteKind::AsinEdition
                | livrarr_domain::identity_layer::RouteKind::GoodreadsBookEdition
                | livrarr_domain::identity_layer::RouteKind::Undeclared {
                    scope: livrarr_domain::identity_layer::RouteScope::Edition,
                    ..
                }
        );
        if edition_scoped {
            let proposed_owner_type: String =
                row.try_get("proposed_owner_type").map_err(repo_decode)?;
            let proposed_owner_id: i64 = row.try_get("proposed_owner_id").map_err(repo_decode)?;
            let (target_work_id, target_edition) = match &command.resolution {
                livrarr_domain::identity_layer::IdentityConflictResolution::Reject { .. } => {
                    (work_id, None)
                }
                livrarr_domain::identity_layer::IdentityConflictResolution::Accept {
                    target_edition,
                    ..
                } => {
                    let proposed_work_id = if proposed_owner_type == "edition" {
                        sqlx::query_scalar(
                            "SELECT work_id FROM editions WHERE user_id = ?1 AND id = ?2 AND state = 'active'",
                        )
                        .bind(command.user_id)
                        .bind(proposed_owner_id)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(repo_db)?
                        .ok_or(IdentityRepositoryError::NotFound)?
                    } else {
                        proposed_owner_id
                    };
                    (proposed_work_id, *target_edition)
                }
                livrarr_domain::identity_layer::IdentityConflictResolution::DifferentWork {
                    winning_work_id,
                    target_edition,
                    ..
                } => (*winning_work_id, *target_edition),
            };
            match target_edition {
                Some(edition_id) => {
                    let belongs: Option<i64> = sqlx::query_scalar(
                        "SELECT id FROM editions WHERE user_id = ?1 AND id = ?2 \
                           AND work_id = ?3 AND state = 'active'",
                    )
                    .bind(command.user_id)
                    .bind(edition_id)
                    .bind(target_work_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(repo_db)?;
                    if belongs.is_none() {
                        return Err(IdentityRepositoryError::StillAmbiguous);
                    }
                }
                None => {
                    let eligible: i64 = sqlx::query_scalar(
                        "SELECT COUNT(*) FROM editions WHERE user_id = ?1 AND work_id = ?2 AND state = 'active'",
                    )
                    .bind(command.user_id)
                    .bind(target_work_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(repo_db)?;
                    if eligible != 1 {
                        return Err(IdentityRepositoryError::StillAmbiguous);
                    }
                }
            }
        }
        let audit = sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'conflict-resolution', 'authenticated-user', ?3, ?4)",
        )
        .bind(command.user_id)
        .bind(work_id)
        .bind(serde_json::to_string(&command.resolution).map_err(repo_json)?)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        sqlx::query(
            "UPDATE identity_conflicts_v2 \
                SET status = 'resolved', resolution = ?1, audit_id = ?2 \
              WHERE user_id = ?3 AND id = ?4 AND status = 'pending'",
        )
        .bind(serde_json::to_string(&command.resolution).map_err(repo_json)?)
        .bind(audit.last_insert_rowid())
        .bind(command.user_id)
        .bind(command.conflict_id)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        let updated = sqlx::query(
            "UPDATE works SET identity_generation = identity_generation + 1 \
              WHERE user_id = ?1 AND id = ?2 AND identity_generation = ?3",
        )
        .bind(command.user_id)
        .bind(work_id)
        .bind(command.expected_generation)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        if updated.rows_affected() != 1 {
            return Err(IdentityRepositoryError::StaleGeneration);
        }
        tx.commit().await.map_err(repo_db)?;
        self.read_captured_identity(command.user_id, work_id).await
    }
}

impl EditionRepository for SqliteDb {
    async fn apply_evidence(
        &self,
        command: EditionEvidenceCommand,
    ) -> Result<EditionEvidenceOutcome, EditionRepositoryError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(edition_db)?;
        let row = sqlx::query(
            "SELECT work_id, format, language, subtitle, subtitle_provenance, \
                    source_provider, provider_edition_id, state \
               FROM editions WHERE user_id = ?1 AND id = ?2",
        )
        .bind(command.user_id)
        .bind(command.edition_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(edition_db)?
        .ok_or_else(|| EditionRepositoryError::Database("edition not found".to_string()))?;
        let existing_format: EditionFormat = decode_json_column(&row, "format")
            .map_err(|error| EditionRepositoryError::Database(error.to_string()))?;
        let existing_language: Option<String> = row
            .try_get("language")
            .map_err(|error| EditionRepositoryError::Database(error.to_string()))?;
        let format_conflict = command.format.as_ref().is_some_and(|incoming| {
            existing_format != EditionFormat::Unknown && incoming != &existing_format
        });
        let language_conflict = command.language.as_ref().is_some_and(|incoming| {
            existing_language
                .as_ref()
                .is_some_and(|existing| incoming != existing)
        });
        if format_conflict || language_conflict {
            sqlx::query(
                "INSERT INTO identity_review_cards \
                    (user_id, work_id, kind, generation, status, payload, created_at) \
                 VALUES (?1, ?2, ?3, 0, 'pending', ?4, ?5)",
            )
            .bind(command.user_id)
            .bind(row.try_get::<i64, _>("work_id").map_err(edition_decode)?)
            .bind(ReviewKind::EditionEvidence.storage_code())
            .bind(
                serde_json::to_string(&SettlementReviewCard::EditionEvidence {
                    edition_id: command.edition_id,
                    evidence_ids: Vec::new(),
                })
                .map_err(edition_json)?,
            )
            .bind(chrono::Utc::now().to_rfc3339())
            .execute(&mut *tx)
            .await
            .map_err(edition_db)?;
            tx.commit().await.map_err(edition_db)?;
            return Err(EditionRepositoryError::ContradictoryEvidenceParked);
        }
        let format = command.format.unwrap_or(existing_format);
        let language = command.language.or(existing_language);
        sqlx::query(
            "UPDATE editions SET format = ?1, language = ?2 \
              WHERE user_id = ?3 AND id = ?4",
        )
        .bind(serde_json::to_string(&format).map_err(edition_json)?)
        .bind(&language)
        .bind(command.user_id)
        .bind(command.edition_id)
        .execute(&mut *tx)
        .await
        .map_err(edition_db)?;
        tx.commit().await.map_err(edition_db)?;

        let work_id = row.try_get("work_id").map_err(edition_decode)?;
        let subtitle_value: Option<String> = row.try_get("subtitle").map_err(edition_decode)?;
        let subtitle_provenance: Option<String> =
            row.try_get("subtitle_provenance").map_err(edition_decode)?;
        let subtitle = subtitle_value.map(|value| livrarr_domain::identity_layer::SourcedValue {
            value,
            provenance: subtitle_provenance
                .and_then(|encoded| serde_json::from_str(&encoded).ok())
                .unwrap_or(EvidenceProvenance::Migrated),
            observed_at: chrono::Utc::now(),
        });
        let source_provider = row
            .try_get::<Option<String>, _>("source_provider")
            .map_err(edition_decode)?
            .and_then(|encoded| serde_json::from_str(&encoded).ok());
        let state = match row
            .try_get::<String, _>("state")
            .map_err(edition_decode)?
            .as_str()
        {
            "active" => EditionState::Active,
            "archived" => EditionState::Archived,
            other => {
                return Err(EditionRepositoryError::Database(format!(
                    "invalid edition state {other}"
                )))
            }
        };
        let route_rows = sqlx::query(
            "SELECT id, user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
                    provider_scoped_id, state, provenance, user_confirmed, observed_at \
               FROM identity_routes WHERE user_id = ?1 AND edition_id = ?2 ORDER BY id",
        )
        .bind(command.user_id)
        .bind(command.edition_id)
        .fetch_all(self.pool())
        .await
        .map_err(edition_db)?;
        let routes = route_rows
            .iter()
            .map(decode_route)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| EditionRepositoryError::Database(error.to_string()))?;
        Ok(EditionEvidenceOutcome {
            edition: Edition {
                id: command.edition_id,
                user_id: command.user_id,
                work_id,
                format,
                language,
                subtitle,
                routes,
                covers: vec![],
                source_provider,
                provider_edition_id: row.try_get("provider_edition_id").map_err(edition_decode)?,
                state,
            },
        })
    }

    async fn apply_work_evidence(
        &self,
        command: EditionWorkEvidenceCommand,
    ) -> Result<EditionEvidenceOutcome, EditionRepositoryError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(edition_db)?;
        let generation: i64 = sqlx::query_scalar(
            "SELECT identity_generation FROM works WHERE user_id = ?1 AND id = ?2",
        )
        .bind(command.user_id)
        .bind(command.work_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(edition_db)?
        .ok_or_else(|| EditionRepositoryError::Database("work not found".to_string()))?;
        let existing = sqlx::query(
            "SELECT id, format, language FROM editions \
              WHERE user_id = ?1 AND work_id = ?2 AND state = 'active' ORDER BY id LIMIT 1",
        )
        .bind(command.user_id)
        .bind(command.work_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(edition_db)?;
        let edition_id = if let Some(row) = existing {
            let edition_id: i64 = row.try_get("id").map_err(edition_decode)?;
            let existing_format: EditionFormat = decode_json_column(&row, "format")
                .map_err(|error| EditionRepositoryError::Database(error.to_string()))?;
            let existing_language: Option<String> =
                row.try_get("language").map_err(edition_decode)?;
            let format_conflict =
                existing_format != EditionFormat::Unknown && existing_format != command.format;
            let language_conflict = command.language.as_ref().is_some_and(|incoming| {
                existing_language
                    .as_ref()
                    .is_some_and(|current| incoming != current)
            });
            if format_conflict || language_conflict {
                sqlx::query(
                    "INSERT INTO identity_review_cards \
                        (user_id, work_id, kind, generation, status, payload, created_at) \
                     VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6)",
                )
                .bind(command.user_id)
                .bind(command.work_id)
                .bind(ReviewKind::EditionEvidence.storage_code())
                .bind(generation)
                .bind(
                    serde_json::to_string(&SettlementReviewCard::EditionEvidence {
                        edition_id,
                        evidence_ids: Vec::new(),
                    })
                    .map_err(edition_json)?,
                )
                .bind(chrono::Utc::now().to_rfc3339())
                .execute(&mut *tx)
                .await
                .map_err(edition_db)?;
                tx.commit().await.map_err(edition_db)?;
                return Err(EditionRepositoryError::ContradictoryEvidenceParked);
            }
            sqlx::query(
                "UPDATE editions SET format = ?1, language = COALESCE(?2, language) \
                  WHERE user_id = ?3 AND work_id = ?4 AND id = ?5",
            )
            .bind(serde_json::to_string(&command.format).map_err(edition_json)?)
            .bind(&command.language)
            .bind(command.user_id)
            .bind(command.work_id)
            .bind(edition_id)
            .execute(&mut *tx)
            .await
            .map_err(edition_db)?;
            edition_id
        } else {
            sqlx::query(
                "INSERT INTO editions (user_id, work_id, format, language, state) \
                 VALUES (?1, ?2, ?3, ?4, 'active')",
            )
            .bind(command.user_id)
            .bind(command.work_id)
            .bind(serde_json::to_string(&command.format).map_err(edition_json)?)
            .bind(&command.language)
            .execute(&mut *tx)
            .await
            .map_err(edition_db)?
            .last_insert_rowid()
        };
        tx.commit().await.map_err(edition_db)?;
        Ok(EditionEvidenceOutcome {
            edition: Edition {
                id: edition_id,
                user_id: command.user_id,
                work_id: command.work_id,
                format: command.format,
                language: command.language,
                subtitle: None,
                routes: Vec::new(),
                covers: Vec::new(),
                source_provider: match command.provenance {
                    EvidenceProvenance::Provider(provider) => Some(provider),
                    _ => None,
                },
                provider_edition_id: None,
                state: EditionState::Active,
            },
        })
    }
}

impl IdentityCutoverService for SqliteDb {
    async fn rehearse(
        &self,
        snapshot: SnapshotDatabase,
        cancel: CancellationToken,
    ) -> Result<IdentityMigrationReport, IdentityMigrationError> {
        if cancel.is_cancelled() {
            return Err(IdentityMigrationError::Cancelled);
        }
        if !snapshot.path.is_file() {
            return Err(IdentityMigrationError::NotSnapshot);
        }
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&snapshot.path)
            .create_if_missing(false)
            .pragma("foreign_keys", "ON")
            .pragma("busy_timeout", "5000");
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|_| IdentityMigrationError::NotSnapshot)?;
        let version: Option<String> =
            sqlx::query_scalar("SELECT value FROM _livrarr_meta WHERE key = 'schema_version'")
                .fetch_optional(&pool)
                .await
                .map_err(|_| IdentityMigrationError::SchemaMismatch)?;
        if version.as_deref() != Some("83") {
            return Err(IdentityMigrationError::SchemaMismatch);
        }
        let snapshot_db = SqliteDb::new(pool);
        let report = build_migration_report(&snapshot_db).await?;
        persist_cutover_report(self, IdentityCutoverMode::Rehearsal, &report).await?;
        Ok(report)
    }

    async fn apply(
        &self,
        approved_report: IdentityMigrationReport,
        cancel: CancellationToken,
    ) -> Result<IdentityMigrationReport, IdentityMigrationError> {
        if cancel.is_cancelled() {
            return Err(IdentityMigrationError::Cancelled);
        }
        self.run_identity_cutover(IdentityCutoverMode::Apply, Some(approved_report))
            .await
    }

    async fn ensure_authority_ready(
        &self,
        cancel: CancellationToken,
    ) -> Result<IdentityAuthorityReadiness, IdentityMigrationError> {
        if cancel.is_cancelled() {
            return Err(IdentityMigrationError::Cancelled);
        }
        self.ensure_identity_authority_ready().await
    }
}

// ---------------------------------------------------------------------------
// `SqliteDb` inherent methods IR v1 lists separately from the trait surface
// above (different signatures — e.g. no `CancellationToken` on the
// no-arg `ensure_identity_authority_ready`).
// ---------------------------------------------------------------------------

impl SqliteDb {
    /// Record one route-native convergence visit for an edition-only bridge.
    /// The Work generation scopes the attempt series, so any later identity
    /// change automatically opens a fresh bounded series without deleting
    /// historical observations.
    pub async fn record_identity_convergence_attempt(
        &self,
        user_id: UserId,
        work_id: WorkId,
        identity_generation: i64,
    ) -> Result<u32, IdentityRepositoryError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(repo_db)?;
        let current: Option<i64> =
            sqlx::query_scalar("SELECT identity_generation FROM works WHERE user_id=?1 AND id=?2")
                .bind(user_id)
                .bind(work_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(repo_db)?;
        let current = current.ok_or(IdentityRepositoryError::NotFound)?;
        if current != identity_generation {
            return Err(IdentityRepositoryError::StaleGeneration);
        }
        let generation_key = identity_generation.to_string();
        let prior: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM identity_provider_attempts \
              WHERE user_id=?1 AND work_id=?2 \
                AND provider='livrarr-convergence' \
                AND route_kind='bridge-upgrade' AND route_value=?3",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(&generation_key)
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_db)?;
        let attempt = prior + 1;
        sqlx::query(
            "INSERT INTO identity_provider_attempts \
                (user_id, work_id, provider, route_kind, route_value, \
                 attempt_key, outcome, observed_at) \
             VALUES (?1, ?2, 'livrarr-convergence', 'bridge-upgrade', ?3, ?4, \
                     'no-route-change', ?5)",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(generation_key)
        .bind(format!("attempt-{attempt}"))
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        tx.commit().await.map_err(repo_db)?;
        u32::try_from(attempt).map_err(|error| {
            IdentityRepositoryError::Database(format!(
                "identity convergence attempt count overflow: {error}"
            ))
        })
    }

    pub async fn transfer_route(
        &self,
        command: TransferRouteCommand,
    ) -> Result<CapturedIdentity, IdentityRepositoryError> {
        let provider = serde_json::to_string(&command.route.provider).map_err(repo_json)?;
        let kind = serde_json::to_string(&command.route.kind).map_err(repo_json)?;
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(repo_db)?;
        let routes = sqlx::query(
            "SELECT id, resolved_work_id FROM identity_routes \
              WHERE user_id = ?1 AND provider = ?2 AND kind = ?3 \
                AND provider_scoped_id = ?4 AND state = 'active'",
        )
        .bind(command.user_id)
        .bind(&provider)
        .bind(&kind)
        .bind(&command.route.value)
        .fetch_all(&mut *tx)
        .await
        .map_err(repo_db)?;
        if routes.is_empty() {
            return Err(IdentityRepositoryError::NotFound);
        }
        if routes.len() != 1 {
            return Err(IdentityRepositoryError::StillAmbiguous);
        }
        let route_id: i64 = routes[0].try_get("id").map_err(repo_decode)?;
        let source_work_id: i64 = routes[0].try_get("resolved_work_id").map_err(repo_decode)?;
        let (owner_type, work_id, edition_id, target_work_id) = match command.target_owner {
            RouteOwner::Work(work_id) => {
                let exists: Option<i64> =
                    sqlx::query_scalar("SELECT id FROM works WHERE user_id = ?1 AND id = ?2")
                        .bind(command.user_id)
                        .bind(work_id)
                        .fetch_optional(&mut *tx)
                        .await
                        .map_err(repo_db)?;
                if exists.is_none() {
                    return Err(IdentityRepositoryError::NotFound);
                }
                ("work", Some(work_id), None, work_id)
            }
            RouteOwner::Edition(edition_id) => {
                let work_id: Option<i64> = sqlx::query_scalar(
                    "SELECT work_id FROM editions WHERE user_id = ?1 AND id = ?2",
                )
                .bind(command.user_id)
                .bind(edition_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(repo_db)?;
                let work_id = work_id.ok_or(IdentityRepositoryError::NotFound)?;
                ("edition", None, Some(edition_id), work_id)
            }
        };
        let source_route_graph_before =
            read_active_route_graph(&mut tx, command.user_id, source_work_id)
                .await
                .map_err(repo_db)?;
        let target_route_graph_before = if target_work_id == source_work_id {
            None
        } else {
            Some(
                read_active_route_graph(&mut tx, command.user_id, target_work_id)
                    .await
                    .map_err(repo_db)?,
            )
        };
        let source_generation: Option<i64> = sqlx::query_scalar(
            "SELECT identity_generation FROM works WHERE user_id = ?1 AND id = ?2",
        )
        .bind(command.user_id)
        .bind(source_work_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(repo_db)?;
        if source_generation != Some(command.expected_generation) {
            return Err(IdentityRepositoryError::StaleGeneration);
        }
        transfer_route_failpoint("before-owner-update")?;
        sqlx::query(
            "UPDATE identity_routes \
                SET owner_type = ?1, work_id = ?2, edition_id = ?3, resolved_work_id = ?4 \
              WHERE user_id = ?5 AND id = ?6 AND state = 'active'",
        )
        .bind(owner_type)
        .bind(work_id)
        .bind(edition_id)
        .bind(target_work_id)
        .bind(command.user_id)
        .bind(route_id)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        let changed = sqlx::query(
            "UPDATE works SET identity_generation = identity_generation + 1 \
              WHERE user_id = ?1 AND id = ?2 AND identity_generation = ?3",
        )
        .bind(command.user_id)
        .bind(source_work_id)
        .bind(command.expected_generation)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        if changed.rows_affected() != 1 {
            return Err(IdentityRepositoryError::StaleGeneration);
        }
        invalidate_retry_state_if_route_graph_changed(
            &mut tx,
            command.user_id,
            source_work_id,
            &source_route_graph_before,
        )
        .await
        .map_err(repo_db)?;
        if let Some(before) = target_route_graph_before.as_ref() {
            invalidate_retry_state_if_route_graph_changed(
                &mut tx,
                command.user_id,
                target_work_id,
                before,
            )
            .await
            .map_err(repo_db)?;
        }
        sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'route-transfer', 'authenticated-user', ?3, ?4)",
        )
        .bind(command.user_id)
        .bind(target_work_id)
        .bind(format!(
            "route_id={route_id};from={source_work_id};to={target_work_id}"
        ))
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        transfer_route_failpoint("before-commit")?;
        tx.commit().await.map_err(repo_db)?;
        self.read_captured_identity(command.user_id, target_work_id)
            .await
    }

    pub async fn recompute_work_projections(
        &self,
        work_id: WorkId,
        expected_generation: i64,
    ) -> Result<WorkProjectionSnapshot, IdentityRepositoryError> {
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(repo_db)?;
        let row = sqlx::query("SELECT user_id, identity_generation FROM works WHERE id = ?1")
            .bind(work_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(repo_db)?
            .ok_or(IdentityRepositoryError::NotFound)?;
        let user_id: i64 = row.try_get("user_id").map_err(repo_decode)?;
        let generation: i64 = row.try_get("identity_generation").map_err(repo_decode)?;
        if generation != expected_generation {
            return Err(IdentityRepositoryError::StaleGeneration);
        }
        let route_stats = sqlx::query(
            "SELECT COUNT(*) AS route_count, \
                    COALESCE(MAX(user_confirmed), 0) AS any_confirmed \
               FROM identity_routes \
              WHERE user_id = ?1 AND resolved_work_id = ?2 AND state = 'active'",
        )
        .bind(user_id)
        .bind(work_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(repo_db)?;
        let route_count: i64 = route_stats.try_get("route_count").map_err(repo_decode)?;
        let any_confirmed: i64 = route_stats.try_get("any_confirmed").map_err(repo_decode)?;
        let status = if any_confirmed > 0 {
            IdentityStatus::UserConfirmed
        } else if route_count > 0 {
            IdentityStatus::Connected
        } else {
            IdentityStatus::NotConnected
        };
        let status_text = encode_identity_status(status);
        let new_generation = generation + 1;
        let updated = sqlx::query(
            "UPDATE works SET identity_status_v2 = ?1, identity_generation = ?2 \
              WHERE user_id = ?3 AND id = ?4 AND identity_generation = ?5",
        )
        .bind(status_text)
        .bind(new_generation)
        .bind(user_id)
        .bind(work_id)
        .bind(generation)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        if updated.rows_affected() != 1 {
            return Err(IdentityRepositoryError::StaleGeneration);
        }
        let subtitle = MachineSubtitleProjection {
            user_id,
            work_id,
            value: None,
            edition_id: None,
            provenance: None,
            computed_at_generation: new_generation,
        };
        sqlx::query(
            "INSERT INTO machine_subtitle_projections \
                (user_id, work_id, value, edition_id, provenance, computed_at_generation) \
             VALUES (?1, ?2, NULL, NULL, NULL, ?3) \
             ON CONFLICT (user_id, work_id) DO UPDATE SET \
                value = excluded.value, edition_id = excluded.edition_id, \
                provenance = excluded.provenance, \
                computed_at_generation = excluded.computed_at_generation",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(new_generation)
        .execute(&mut *tx)
        .await
        .map_err(repo_db)?;
        tx.commit().await.map_err(repo_db)?;
        Ok(WorkProjectionSnapshot {
            work_id,
            subtitle,
            covers: empty_cover_presentation(),
            status,
            generation: new_generation,
        })
    }

    pub async fn record_embedded_cover_inspection(
        &self,
        record: EmbeddedCoverInspectionRecord,
    ) -> Result<(), IdentityRepositoryError> {
        sqlx::query(
            "INSERT INTO embedded_cover_inspections \
                (user_id, library_item_id, revision_size_bytes, revision_modified_ns, \
                 revision_sha256, outcome, cover_candidate_id, sanitized_error_code, inspected_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
             ON CONFLICT (user_id, library_item_id, revision_size_bytes, \
                          revision_modified_ns, revision_sha256) DO UPDATE SET \
                outcome = excluded.outcome, \
                cover_candidate_id = excluded.cover_candidate_id, \
                sanitized_error_code = excluded.sanitized_error_code, \
                inspected_at = excluded.inspected_at",
        )
        .bind(record.user_id)
        .bind(record.library_item_id)
        .bind(i64::try_from(record.revision.size_bytes).map_err(|_| {
            IdentityRepositoryError::Database("file revision size exceeds SQLite i64".to_string())
        })?)
        .bind(record.revision.modified_ns.to_string())
        .bind(record.revision.sha256.to_vec())
        .bind(encode_inspection_outcome(record.outcome))
        .bind(record.cover_candidate_id)
        .bind(record.sanitized_error_code)
        .bind(record.inspected_at.to_rfc3339())
        .execute(self.pool())
        .await
        .map_err(repo_db)?;
        Ok(())
    }

    pub async fn read_embedded_cover_inspection(
        &self,
        user_id: UserId,
        library_item_id: LibraryItemId,
        revision: FileRevision,
    ) -> Result<Option<EmbeddedCoverInspectionRecord>, IdentityRepositoryError> {
        let size = i64::try_from(revision.size_bytes).map_err(|_| {
            IdentityRepositoryError::Database("file revision size exceeds SQLite i64".to_string())
        })?;
        let row = sqlx::query(
            "SELECT outcome, cover_candidate_id, sanitized_error_code, inspected_at \
               FROM embedded_cover_inspections \
              WHERE user_id = ?1 AND library_item_id = ?2 \
                AND revision_size_bytes = ?3 AND revision_modified_ns = ?4 \
                AND revision_sha256 = ?5",
        )
        .bind(user_id)
        .bind(library_item_id)
        .bind(size)
        .bind(revision.modified_ns.to_string())
        .bind(revision.sha256.to_vec())
        .fetch_optional(self.pool())
        .await
        .map_err(repo_db)?;
        row.map(|row| {
            let inspected_at = chrono::DateTime::parse_from_rfc3339(
                &row.try_get::<String, _>("inspected_at")
                    .map_err(repo_decode)?,
            )
            .map_err(|error| IdentityRepositoryError::Database(error.to_string()))?
            .with_timezone(&chrono::Utc);
            Ok(EmbeddedCoverInspectionRecord {
                user_id,
                library_item_id,
                revision,
                outcome: decode_inspection_outcome(
                    &row.try_get::<String, _>("outcome").map_err(repo_decode)?,
                )?,
                cover_candidate_id: row.try_get("cover_candidate_id").map_err(repo_decode)?,
                sanitized_error_code: row.try_get("sanitized_error_code").map_err(repo_decode)?,
                inspected_at,
            })
        })
        .transpose()
    }

    pub async fn run_identity_cutover(
        &self,
        mode: IdentityCutoverMode,
        approved_report: Option<IdentityMigrationReport>,
    ) -> Result<IdentityMigrationReport, IdentityMigrationError> {
        let report = build_migration_report(self).await?;
        if matches!(mode, IdentityCutoverMode::Apply) {
            let active: Option<String> = sqlx::query_scalar(
                "SELECT value FROM _livrarr_meta WHERE key = 'identity_authority_v2'",
            )
            .fetch_optional(self.pool())
            .await
            .map_err(migration_db)?;
            if active.as_deref() == Some("active") {
                return Err(IdentityMigrationError::RehearsalMismatch);
            }
            if let Some(approved) = approved_report.as_ref() {
                if approved.source_schema_version != report.source_schema_version
                    || approved.source_fingerprint != report.source_fingerprint
                {
                    return Err(IdentityMigrationError::RehearsalMismatch);
                }
                reuse_staged_rows(self, approved).await?;
            }
            stage_legacy_identity_rows(self, &report).await?;
        }
        persist_cutover_report(self, mode, &report).await?;
        Ok(report)
    }

    pub async fn ensure_identity_authority_ready(
        &self,
    ) -> Result<IdentityAuthorityReadiness, IdentityMigrationError> {
        let marker: Option<String> = sqlx::query_scalar(
            "SELECT value FROM _livrarr_meta WHERE key = 'identity_authority_v2'",
        )
        .fetch_optional(self.pool())
        .await
        .map_err(migration_db)?;
        if marker.as_deref() == Some("active") {
            return Ok(IdentityAuthorityReadiness::Active);
        }
        let mut tx = crate::pool::begin_write(self.pool())
            .await
            .map_err(migration_db)?;
        let legacy_work_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM works")
            .fetch_one(&mut *tx)
            .await
            .map_err(migration_db)?;
        let legacy_review_count: i64 = sqlx::query_scalar(
            "SELECT (SELECT COUNT(*) FROM work_identity_conflicts \
                     WHERE status = 'open') + \
                    (SELECT COUNT(*) FROM identity_review_cards \
                     WHERE status = 'pending')",
        )
        .fetch_one(&mut *tx)
        .await
        .unwrap_or(0);
        let latest_apply: Option<(i64, String, i64, i64)> = sqlx::query_as(
            "SELECT run.id, run.status, report.index_ready, report.blocker_count \
               FROM identity_cutover_runs AS run \
               JOIN identity_cutover_reports AS report ON report.run_id = run.id \
              WHERE run.mode = 'apply' ORDER BY run.id DESC LIMIT 1",
        )
        .fetch_optional(&mut *tx)
        .await
        .map_err(migration_db)?;
        let ready_run_id =
            latest_apply
                .as_ref()
                .and_then(|(run_id, status, index_ready, blocker_count)| {
                    (status == "ready" && *index_ready == 1 && *blocker_count == 0)
                        .then_some(*run_id)
                });
        if legacy_review_count != 0 || (legacy_work_count != 0 && ready_run_id.is_none()) {
            return Ok(IdentityAuthorityReadiness::CutoverRequired);
        }
        sqlx::query("DROP INDEX IF EXISTS idx_works_identity")
            .execute(&mut *tx)
            .await
            .map_err(migration_db)?;
        sqlx::query("DROP INDEX IF EXISTS idx_works_user_normalized")
            .execute(&mut *tx)
            .await
            .map_err(migration_db)?;
        readiness_index_failpoint()?;
        activation_index_failpoint()?;
        if let Some(run_id) = ready_run_id {
            let updated = sqlx::query(
                "UPDATE identity_cutover_runs \
                    SET status = 'activated', updated_at = ?1 \
                  WHERE id = ?2 AND mode = 'apply' AND status = 'ready'",
            )
            .bind(chrono::Utc::now().to_rfc3339())
            .bind(run_id)
            .execute(&mut *tx)
            .await
            .map_err(migration_db)?;
            if updated.rows_affected() != 1 {
                return Err(IdentityMigrationError::RehearsalMismatch);
            }
        }
        sqlx::query(
            "INSERT INTO _livrarr_meta (key, value) VALUES ('identity_authority_v2', 'active') \
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        )
        .execute(&mut *tx)
        .await
        .map_err(migration_db)?;
        sqlx::query(
            "CREATE UNIQUE INDEX idx_works_identity_v2 ON works \
                (user_id, normalized_identity_main, normalized_identity_subtitle, \
                 normalized_identity_volume, primary_author_id, text_distinction)",
        )
        .execute(&mut *tx)
        .await
        .map_err(|error| {
            if error_is_unique(&error) {
                IdentityMigrationError::Collision
            } else {
                migration_db(error)
            }
        })?;
        tx.commit().await.map_err(migration_db)?;
        Ok(IdentityAuthorityReadiness::ActivatedFresh)
    }
}

fn repo_db(error: sqlx::Error) -> IdentityRepositoryError {
    IdentityRepositoryError::Database(error.to_string())
}

fn repo_decode(error: sqlx::Error) -> IdentityRepositoryError {
    IdentityRepositoryError::Database(error.to_string())
}

fn repo_json(error: serde_json::Error) -> IdentityRepositoryError {
    IdentityRepositoryError::Database(error.to_string())
}

fn edition_db(error: sqlx::Error) -> EditionRepositoryError {
    EditionRepositoryError::Database(error.to_string())
}

fn edition_decode(error: sqlx::Error) -> EditionRepositoryError {
    EditionRepositoryError::Database(error.to_string())
}

fn edition_json(error: serde_json::Error) -> EditionRepositoryError {
    EditionRepositoryError::Database(error.to_string())
}

fn migration_db(error: sqlx::Error) -> IdentityMigrationError {
    IdentityMigrationError::Database(error.to_string())
}

fn error_is_unique(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .is_some_and(|database| database.is_unique_violation())
}

fn map_settlement_sql(error: sqlx::Error) -> IdentityRepositoryError {
    if error_is_unique(&error) {
        IdentityRepositoryError::KeyCollision
    } else {
        repo_db(error)
    }
}

fn decode_json_column<T: serde::de::DeserializeOwned>(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<T, IdentityRepositoryError> {
    let encoded: String = row.try_get(column).map_err(repo_decode)?;
    serde_json::from_str(&encoded).map_err(repo_json)
}

fn decode_route(row: &sqlx::sqlite::SqliteRow) -> Result<WorkRoute, IdentityRepositoryError> {
    let owner_type: String = row.try_get("owner_type").map_err(repo_decode)?;
    let owner = match owner_type.as_str() {
        "work" => RouteOwner::Work(
            row.try_get::<Option<i64>, _>("work_id")
                .map_err(repo_decode)?
                .ok_or_else(|| {
                    IdentityRepositoryError::Database(
                        "work route is missing its work owner".to_string(),
                    )
                })?,
        ),
        "edition" => RouteOwner::Edition(
            row.try_get::<Option<i64>, _>("edition_id")
                .map_err(repo_decode)?
                .ok_or_else(|| {
                    IdentityRepositoryError::Database(
                        "edition route is missing its edition owner".to_string(),
                    )
                })?,
        ),
        other => {
            return Err(IdentityRepositoryError::Database(format!(
                "invalid route owner type {other}"
            )))
        }
    };
    let state = match row
        .try_get::<String, _>("state")
        .map_err(repo_decode)?
        .as_str()
    {
        "active" => WorkRouteState::Active,
        "retired" => WorkRouteState::Retired { audit_id: 0 },
        other => {
            return Err(IdentityRepositoryError::Database(format!(
                "invalid route state {other}"
            )))
        }
    };
    let observed_at = chrono::DateTime::parse_from_rfc3339(
        &row.try_get::<String, _>("observed_at")
            .map_err(repo_decode)?,
    )
    .map_err(|error| IdentityRepositoryError::Database(error.to_string()))?
    .with_timezone(&chrono::Utc);
    Ok(WorkRoute {
        id: row.try_get("id").map_err(repo_decode)?,
        user_id: row.try_get("user_id").map_err(repo_decode)?,
        owner,
        resolved_work_id: row.try_get("resolved_work_id").map_err(repo_decode)?,
        provider: decode_json_column(row, "provider")?,
        kind: decode_json_column(row, "kind")?,
        provider_scoped_id: row.try_get("provider_scoped_id").map_err(repo_decode)?,
        state,
        provenance: decode_json_column(row, "provenance")?,
        user_confirmed: row
            .try_get::<i64, _>("user_confirmed")
            .map_err(repo_decode)?
            != 0,
        observed_at,
    })
}

/// Cancel pending route decisions made obsolete by routes this settlement has
/// made active. Card state and its audit participate in the settlement's write
/// transaction, so readers can never observe the route without the queue
/// cleanup (REQ-027 v9 card lifecycle).
async fn cancel_satisfied_pending_route_cards(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: UserId,
    work_id: WorkId,
    active_routes: &[WorkRoute],
) -> Result<usize, IdentityRepositoryError> {
    if active_routes.is_empty() {
        return Ok(0);
    }
    let active_keys: Result<BTreeSet<String>, IdentityRepositoryError> = active_routes
        .iter()
        .map(|route| {
            serde_json::to_string(&(
                &route.provider,
                &route.kind,
                route.provider_scoped_id.trim(),
            ))
            .map_err(repo_json)
        })
        .collect();
    let active_keys = active_keys?;
    let pending = sqlx::query(
        "SELECT id, payload FROM identity_review_cards \
          WHERE user_id=?1 AND work_id=?2 AND kind='PendingRoute' \
            AND status='pending' ORDER BY id",
    )
    .bind(user_id)
    .bind(work_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_db)?;
    let now = chrono::Utc::now().to_rfc3339();
    let mut cancelled = 0usize;
    for row in pending {
        let card_id: i64 = row.try_get("id").map_err(repo_decode)?;
        let payload: String = row.try_get("payload").map_err(repo_decode)?;
        let card: SettlementReviewCard = serde_json::from_str(&payload).map_err(repo_json)?;
        let Some(key) = pending_route_proposal_key(&card) else {
            return Err(IdentityRepositoryError::Database(
                "PendingRoute row has a non-PendingRoute payload".to_string(),
            ));
        };
        if !active_keys.contains(&key) {
            continue;
        }
        let updated = sqlx::query(
            "UPDATE identity_review_cards SET status='cancelled', resolved_at=?1 \
              WHERE id=?2 AND user_id=?3 AND status='pending'",
        )
        .bind(&now)
        .bind(card_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_db)?;
        if updated.rows_affected() != 1 {
            continue;
        }
        sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'pending-route-satisfied', 'identity-engine', ?3, ?4)",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(format!("card_id={card_id};route_key={key}"))
        .bind(&now)
        .execute(&mut **tx)
        .await
        .map_err(repo_db)?;
        cancelled += 1;
    }
    Ok(cancelled)
}

async fn insert_route(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: UserId,
    work_id: WorkId,
    route: &WorkRoute,
) -> Result<(), IdentityRepositoryError> {
    let (owner_type, owner_work_id, edition_id) = match route.owner {
        RouteOwner::Work(_) => ("work", Some(work_id), None),
        RouteOwner::Edition(edition_id) => {
            let edition_work: Option<i64> =
                sqlx::query_scalar("SELECT work_id FROM editions WHERE user_id = ?1 AND id = ?2")
                    .bind(user_id)
                    .bind(edition_id)
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(repo_db)?;
            if edition_work != Some(work_id) {
                return Err(IdentityRepositoryError::NotFound);
            }
            ("edition", None, Some(edition_id))
        }
    };
    let state = match route.state {
        WorkRouteState::Active => "active",
        WorkRouteState::Retired { .. } => "retired",
    };
    let provider = serde_json::to_string(&route.provider).map_err(repo_json)?;
    let kind = serde_json::to_string(&route.kind).map_err(repo_json)?;
    let provenance = serde_json::to_string(&route.provenance).map_err(repo_json)?;
    let existing = sqlx::query(
        "SELECT id, provenance, user_confirmed FROM identity_routes \
          WHERE user_id = ?1 AND resolved_work_id = ?2 AND provider = ?3 \
            AND kind = ?4 AND provider_scoped_id = ?5 LIMIT 1",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(&provider)
    .bind(&kind)
    .bind(&route.provider_scoped_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(repo_db)?;
    if let Some(existing) = existing {
        let route_id: i64 = existing.try_get("id").map_err(repo_decode)?;
        let existing_provenance: RouteProvenance = serde_json::from_str(
            &existing
                .try_get::<String, _>("provenance")
                .map_err(repo_decode)?,
        )
        .map_err(repo_json)?;
        let provenance = if matches!(
            &existing_provenance,
            RouteProvenance::UserChoice | RouteProvenance::OwnedFile { .. }
        ) {
            serde_json::to_string(&existing_provenance).map_err(repo_json)?
        } else {
            provenance
        };
        let user_confirmed = route.user_confirmed
            || existing
                .try_get::<i64, _>("user_confirmed")
                .map_err(repo_decode)?
                != 0;
        sqlx::query(
            "UPDATE identity_routes SET owner_type = ?1, work_id = ?2, edition_id = ?3, \
                    state = ?4, provenance = ?5, user_confirmed = ?6, observed_at = ?7 \
              WHERE id = ?8 AND user_id = ?9",
        )
        .bind(owner_type)
        .bind(owner_work_id)
        .bind(edition_id)
        .bind(state)
        .bind(provenance)
        .bind(i64::from(user_confirmed))
        .bind(route.observed_at.to_rfc3339())
        .bind(route_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_db)?;
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO identity_routes \
            (user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
             provider_scoped_id, state, provenance, user_confirmed, observed_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )
    .bind(user_id)
    .bind(owner_type)
    .bind(owner_work_id)
    .bind(edition_id)
    .bind(work_id)
    .bind(provider)
    .bind(kind)
    .bind(&route.provider_scoped_id)
    .bind(state)
    .bind(provenance)
    .bind(i64::from(route.user_confirmed))
    .bind(route.observed_at.to_rfc3339())
    .execute(&mut **tx)
    .await
    .map_err(|error| {
        if error_is_unique(&error) {
            IdentityRepositoryError::RouteOwnershipCollision
        } else {
            repo_db(error)
        }
    })?;
    Ok(())
}

fn route_kind_requires_edition(kind: &RouteKind) -> bool {
    matches!(
        kind,
        RouteKind::Isbn13Edition | RouteKind::AsinEdition | RouteKind::GoodreadsBookEdition
    )
}

/// Settlement owns the taxonomy boundary: edition-scoped identifiers can
/// never persist as Work-owned routes. The Edition row and its route owner are
/// materialized inside the same transaction and generation claim as the rest
/// of the identity settlement.
async fn materialize_edition_route_owner(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: UserId,
    work_id: WorkId,
    route: &mut WorkRoute,
) -> Result<(), IdentityRepositoryError> {
    if !route_kind_requires_edition(&route.kind) {
        return Ok(());
    }
    if let RouteOwner::Edition(_) = route.owner {
        return Ok(());
    }
    let provider = serde_json::to_string(&route.provider).map_err(repo_json)?;
    let kind = serde_json::to_string(&route.kind).map_err(repo_json)?;
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT edition_id FROM identity_routes \
          WHERE user_id=?1 AND provider=?2 AND kind=?3 AND provider_scoped_id=?4 \
            AND state='active' AND edition_id IS NOT NULL LIMIT 1",
    )
    .bind(user_id)
    .bind(&provider)
    .bind(&kind)
    .bind(&route.provider_scoped_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(repo_db)?;
    let edition_id = if let Some(edition_id) = existing {
        edition_id
    } else {
        sqlx::query(
            "INSERT INTO editions \
                (user_id, work_id, format, source_provider, provider_edition_id, state) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'active')",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(serde_json::to_string(&EditionFormat::Unknown).map_err(repo_json)?)
        .bind(provider)
        .bind(&route.provider_scoped_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_db)?
        .last_insert_rowid()
    };
    route.owner = RouteOwner::Edition(edition_id);
    Ok(())
}

/// Fold every user-authored/dependent row into `winner_work_id` before the
/// loser is deleted. This is shared by automatic broad-group reconciliation
/// and the typed manual-merge review continuation.
#[derive(Debug, Clone, Copy, Default)]
struct AbsorptionCounts {
    library_items: usize,
    grabs: usize,
}

impl std::ops::AddAssign for AbsorptionCounts {
    fn add_assign(&mut self, rhs: Self) {
        self.library_items += rhs.library_items;
        self.grabs += rhs.grabs;
    }
}

async fn absorb_work_into(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: UserId,
    winner_work_id: WorkId,
    loser_work_id: WorkId,
) -> Result<AbsorptionCounts, IdentityRepositoryError> {
    if winner_work_id == loser_work_id {
        return Ok(AbsorptionCounts::default());
    }
    let owned: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM works WHERE user_id = ?1 AND id IN (?2, ?3)")
            .bind(user_id)
            .bind(winner_work_id)
            .bind(loser_work_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(repo_db)?;
    if owned != 2 {
        return Err(IdentityRepositoryError::NotFound);
    }

    let mut moved = AbsorptionCounts::default();
    for table in ["library_items", "grabs", "history", "bookmarks"] {
        let sql = format!("UPDATE {table} SET work_id = ?1 WHERE user_id = ?2 AND work_id = ?3");
        let result = sqlx::query(&sql)
            .bind(winner_work_id)
            .bind(user_id)
            .bind(loser_work_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_db)?;
        let rows = usize::try_from(result.rows_affected()).unwrap_or(usize::MAX);
        match table {
            "library_items" => moved.library_items += rows,
            "grabs" => moved.grabs += rows,
            _ => {}
        }
    }

    // Contributor roles are copied before the loser contributor is removed;
    // the winner keeps its order and newly adopted Authors append.
    let loser_authors: Vec<i64> = sqlx::query_scalar(
        "SELECT author_id FROM work_contributors \
          WHERE user_id = ?1 AND work_id = ?2 ORDER BY ordinal, author_id",
    )
    .bind(user_id)
    .bind(loser_work_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_db)?;
    for author_id in loser_authors {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM work_contributors \
              WHERE user_id = ?1 AND work_id = ?2 AND author_id = ?3",
        )
        .bind(user_id)
        .bind(winner_work_id)
        .bind(author_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(repo_db)?;
        if exists == 0 {
            let ordinal: i64 = sqlx::query_scalar(
                "SELECT COALESCE(MAX(ordinal), -1) + 1 FROM work_contributors \
                  WHERE user_id = ?1 AND work_id = ?2",
            )
            .bind(user_id)
            .bind(winner_work_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(repo_db)?;
            sqlx::query(
                "INSERT INTO work_contributors (user_id, work_id, author_id, ordinal) \
                 VALUES (?1, ?2, ?3, ?4)",
            )
            .bind(user_id)
            .bind(winner_work_id)
            .bind(author_id)
            .bind(ordinal)
            .execute(&mut **tx)
            .await
            .map_err(repo_db)?;
        }
        sqlx::query(
            "INSERT OR IGNORE INTO work_contributor_roles \
                (user_id, work_id, author_id, role, provenance, observed_at) \
             SELECT user_id, ?1, author_id, role, provenance, observed_at \
               FROM work_contributor_roles \
              WHERE user_id = ?2 AND work_id = ?3 AND author_id = ?4",
        )
        .bind(winner_work_id)
        .bind(user_id)
        .bind(loser_work_id)
        .bind(author_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_db)?;
    }

    // Coalesce identical route evidence before repointing. Retired rows remain
    // durable; active ownership is never duplicated.
    let loser_routes = sqlx::query(
        "SELECT id, provider, kind, provider_scoped_id, user_confirmed \
           FROM identity_routes WHERE user_id = ?1 AND resolved_work_id = ?2 ORDER BY id",
    )
    .bind(user_id)
    .bind(loser_work_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_db)?;
    for row in loser_routes {
        let route_id: i64 = row.try_get("id").map_err(repo_decode)?;
        let provider: String = row.try_get("provider").map_err(repo_decode)?;
        let kind: String = row.try_get("kind").map_err(repo_decode)?;
        let value: String = row.try_get("provider_scoped_id").map_err(repo_decode)?;
        let confirmed: i64 = row.try_get("user_confirmed").map_err(repo_decode)?;
        let winner_route: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM identity_routes WHERE user_id = ?1 AND resolved_work_id = ?2 \
                AND provider = ?3 AND kind = ?4 AND provider_scoped_id = ?5 LIMIT 1",
        )
        .bind(user_id)
        .bind(winner_work_id)
        .bind(&provider)
        .bind(&kind)
        .bind(&value)
        .fetch_optional(&mut **tx)
        .await
        .map_err(repo_db)?;
        if let Some(winner_route_id) = winner_route {
            sqlx::query(
                "UPDATE identity_routes SET user_confirmed = MAX(user_confirmed, ?1), \
                        provenance = ?2 WHERE id = ?3 AND user_id = ?4",
            )
            .bind(confirmed)
            .bind(serde_json::to_string(&RouteProvenance::MergeCoalesced).map_err(repo_json)?)
            .bind(winner_route_id)
            .bind(user_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_db)?;
            sqlx::query("DELETE FROM identity_routes WHERE id = ?1 AND user_id = ?2")
                .bind(route_id)
                .bind(user_id)
                .execute(&mut **tx)
                .await
                .map_err(repo_db)?;
        } else {
            sqlx::query(
                "UPDATE identity_routes SET resolved_work_id = ?1, \
                        work_id = CASE WHEN owner_type = 'work' THEN ?1 ELSE work_id END, \
                        provenance = ?2 WHERE id = ?3 AND user_id = ?4",
            )
            .bind(winner_work_id)
            .bind(serde_json::to_string(&RouteProvenance::MergeCoalesced).map_err(repo_json)?)
            .bind(route_id)
            .bind(user_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_db)?;
        }
    }

    let default_editions = sqlx::query(
        "SELECT format, edition_id, provenance FROM work_default_editions \
          WHERE user_id = ?1 AND work_id = ?2",
    )
    .bind(user_id)
    .bind(loser_work_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_db)?;
    sqlx::query("DELETE FROM work_default_editions WHERE user_id = ?1 AND work_id = ?2")
        .bind(user_id)
        .bind(loser_work_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_db)?;
    sqlx::query("UPDATE editions SET work_id = ?1 WHERE user_id = ?2 AND work_id = ?3")
        .bind(winner_work_id)
        .bind(user_id)
        .bind(loser_work_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_db)?;
    for row in default_editions {
        sqlx::query(
            "INSERT OR IGNORE INTO work_default_editions \
                (user_id, work_id, format, edition_id, provenance) VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(user_id)
        .bind(winner_work_id)
        .bind(row.try_get::<String, _>("format").map_err(repo_decode)?)
        .bind(row.try_get::<i64, _>("edition_id").map_err(repo_decode)?)
        .bind(
            row.try_get::<String, _>("provenance")
                .map_err(repo_decode)?,
        )
        .execute(&mut **tx)
        .await
        .map_err(repo_db)?;
    }

    sqlx::query(
        "INSERT OR IGNORE INTO work_subjects \
            (user_id, work_id, subject_kind, value, provenance, observed_at) \
         SELECT user_id, ?1, subject_kind, value, provenance, observed_at \
           FROM work_subjects WHERE user_id = ?2 AND work_id = ?3",
    )
    .bind(winner_work_id)
    .bind(user_id)
    .bind(loser_work_id)
    .execute(&mut **tx)
    .await
    .map_err(repo_db)?;
    sqlx::query("DELETE FROM work_subjects WHERE user_id = ?1 AND work_id = ?2")
        .bind(user_id)
        .bind(loser_work_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_db)?;

    for table in [
        "identity_review_cards",
        "identity_conflicts_v2",
        "identity_provider_attempts",
        "identity_audit_events",
    ] {
        let column = if table == "identity_conflicts_v2" {
            "current_work_id"
        } else {
            "work_id"
        };
        let sql = format!("UPDATE {table} SET {column} = ?1 WHERE user_id = ?2 AND {column} = ?3");
        sqlx::query(&sql)
            .bind(winner_work_id)
            .bind(user_id)
            .bind(loser_work_id)
            .execute(&mut **tx)
            .await
            .map_err(repo_db)?;
    }
    sqlx::query(
        "UPDATE work_relationships SET from_work_id = ?1 \
          WHERE user_id = ?2 AND from_work_id = ?3",
    )
    .bind(winner_work_id)
    .bind(user_id)
    .bind(loser_work_id)
    .execute(&mut **tx)
    .await
    .map_err(repo_db)?;
    sqlx::query(
        "UPDATE work_relationships SET target_work_id = ?1 \
          WHERE user_id = ?2 AND target_work_id = ?3",
    )
    .bind(winner_work_id)
    .bind(user_id)
    .bind(loser_work_id)
    .execute(&mut **tx)
    .await
    .map_err(repo_db)?;

    sqlx::query(
        "INSERT INTO identity_merge_archives \
            (user_id, winner_work_id, loser_work_id, preserved_fields, archived_at) \
         VALUES (?1, ?2, ?3, 'all-dependent-rows', ?4)",
    )
    .bind(user_id)
    .bind(winner_work_id)
    .bind(loser_work_id)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&mut **tx)
    .await
    .map_err(repo_db)?;
    sqlx::query("DELETE FROM works WHERE user_id = ?1 AND id = ?2")
        .bind(user_id)
        .bind(loser_work_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_db)?;
    Ok(moved)
}

async fn merge_contributors(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: UserId,
    work_id: WorkId,
    requested: &[WorkContributor],
) -> Result<(), IdentityRepositoryError> {
    let existing = sqlx::query(
        "SELECT author_id, ordinal FROM work_contributors \
          WHERE user_id = ?1 AND work_id = ?2 ORDER BY ordinal DESC, author_id DESC",
    )
    .bind(user_id)
    .bind(work_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(repo_db)?;
    let existing_authors = existing
        .iter()
        .map(|row| row.try_get::<i64, _>("author_id").map_err(repo_decode))
        .collect::<Result<Vec<_>, _>>()?;
    for row in &existing {
        let author_id: i64 = row.try_get("author_id").map_err(repo_decode)?;
        let ordinal: i64 = row.try_get("ordinal").map_err(repo_decode)?;
        sqlx::query(
            "UPDATE work_contributors SET ordinal = ?1 \
              WHERE user_id = ?2 AND work_id = ?3 AND author_id = ?4",
        )
        .bind(ordinal + 1_000_000)
        .bind(user_id)
        .bind(work_id)
        .bind(author_id)
        .execute(&mut **tx)
        .await
        .map_err(repo_db)?;
    }
    let mut ordered = requested
        .iter()
        .map(|contributor| contributor.author_id)
        .collect::<Vec<_>>();
    for author_id in existing_authors {
        if !ordered.contains(&author_id) {
            ordered.push(author_id);
        }
    }
    for (ordinal, author_id) in ordered.iter().copied().enumerate() {
        sqlx::query(
            "INSERT INTO work_contributors (user_id, work_id, author_id, ordinal) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(user_id, work_id, author_id) DO UPDATE SET ordinal = excluded.ordinal",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(author_id)
        .bind(ordinal as i64)
        .execute(&mut **tx)
        .await
        .map_err(repo_db)?;
    }
    for contributor in requested {
        for role in &contributor.roles {
            sqlx::query(
                "INSERT OR IGNORE INTO work_contributor_roles \
                    (user_id, work_id, author_id, role, provenance, observed_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            )
            .bind(user_id)
            .bind(work_id)
            .bind(contributor.author_id)
            .bind(&role.value)
            .bind(serde_json::to_string(&role.provenance).map_err(repo_json)?)
            .bind(role.observed_at.to_rfc3339())
            .execute(&mut **tx)
            .await
            .map_err(repo_db)?;
        }
    }
    Ok(())
}

async fn apply_manual_merge_fields(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: UserId,
    winner_work_id: WorkId,
    work_ids: &[WorkId],
    choices: &[livrarr_domain::services::MergeFieldChoiceEntry],
) -> Result<(), IdentityRepositoryError> {
    let loser_work_id = work_ids
        .iter()
        .copied()
        .find(|work_id| *work_id != winner_work_id)
        .ok_or(IdentityRepositoryError::InvalidResolution)?;
    let winner = sqlx::query(
        "SELECT series_name, series_position, monitor_ebook, monitor_audiobook \
           FROM works WHERE user_id = ?1 AND id = ?2",
    )
    .bind(user_id)
    .bind(winner_work_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(repo_db)?
    .ok_or(IdentityRepositoryError::NotFound)?;
    let loser = sqlx::query(
        "SELECT series_name, series_position, monitor_ebook, monitor_audiobook \
           FROM works WHERE user_id = ?1 AND id = ?2",
    )
    .bind(user_id)
    .bind(loser_work_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(repo_db)?
    .ok_or(IdentityRepositoryError::NotFound)?;
    let choice = |field| {
        choices
            .iter()
            .find(|entry| entry.field == field)
            .map(|entry| entry.choice)
    };
    use livrarr_domain::services::{MergeFieldChoice, MergeableField};
    let winner_series_name: Option<String> = winner.try_get("series_name").map_err(repo_decode)?;
    let loser_series_name: Option<String> = loser.try_get("series_name").map_err(repo_decode)?;
    let series_name = match choice(MergeableField::SeriesName) {
        Some(MergeFieldChoice::KeepSurvivor) => winner_series_name,
        Some(MergeFieldChoice::TakeLoser) => loser_series_name,
        None => winner_series_name.or(loser_series_name),
    };
    let winner_series_position: Option<f64> =
        winner.try_get("series_position").map_err(repo_decode)?;
    let loser_series_position: Option<f64> =
        loser.try_get("series_position").map_err(repo_decode)?;
    let series_position = match choice(MergeableField::SeriesPosition) {
        Some(MergeFieldChoice::KeepSurvivor) => winner_series_position,
        Some(MergeFieldChoice::TakeLoser) => loser_series_position,
        None => winner_series_position.or(loser_series_position),
    };
    let monitor_ebook = winner
        .try_get::<i64, _>("monitor_ebook")
        .map_err(repo_decode)?
        != 0
        || loser
            .try_get::<i64, _>("monitor_ebook")
            .map_err(repo_decode)?
            != 0;
    let monitor_audiobook = winner
        .try_get::<i64, _>("monitor_audiobook")
        .map_err(repo_decode)?
        != 0
        || loser
            .try_get::<i64, _>("monitor_audiobook")
            .map_err(repo_decode)?
            != 0;
    sqlx::query(
        "UPDATE works SET series_name = ?1, series_position = ?2, monitor_ebook = ?3, \
                monitor_audiobook = ?4 WHERE user_id = ?5 AND id = ?6",
    )
    .bind(series_name)
    .bind(series_position)
    .bind(monitor_ebook)
    .bind(monitor_audiobook)
    .bind(user_id)
    .bind(winner_work_id)
    .execute(&mut **tx)
    .await
    .map_err(repo_db)?;
    Ok(())
}

fn legacy_route_slot(
    kind: &livrarr_domain::identity_layer::RouteKind,
) -> Option<(&'static str, &'static str)> {
    use livrarr_domain::identity_layer::RouteKind;
    match kind {
        RouteKind::OpenLibraryWork => Some(("ol_work", "ol_key")),
        RouteKind::GoodreadsWork => Some(("gr_work", "gr_key")),
        RouteKind::HardcoverWork => Some(("hc_work", "hc_key")),
        RouteKind::Isbn13Edition => Some(("isbn_13", "isbn_13")),
        RouteKind::AsinEdition => Some(("asin", "asin")),
        RouteKind::GoodreadsBookEdition | RouteKind::Undeclared { .. } => None,
    }
}

fn decode_pending_review(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<PendingReviewCard, IdentityRepositoryError> {
    let kind_code = row.try_get::<String, _>("kind").map_err(repo_decode)?;
    let kind = ReviewKind::from_storage_code(&kind_code).ok_or_else(|| {
        IdentityRepositoryError::Database(format!("unknown review kind {kind_code}"))
    })?;
    let payload: SettlementReviewCard =
        serde_json::from_str(&row.try_get::<String, _>("payload").map_err(repo_decode)?)
            .map_err(repo_json)?;
    if payload.kind() != kind {
        return Err(IdentityRepositoryError::Database(
            "review kind does not match typed payload".to_string(),
        ));
    }
    Ok(PendingReviewCard {
        id: row.try_get("id").map_err(repo_decode)?,
        user_id: row.try_get("user_id").map_err(repo_decode)?,
        work_id: row.try_get("work_id").map_err(repo_decode)?,
        work_title: row.try_get("work_title").map_err(repo_decode)?,
        work_author: row.try_get("work_author").map_err(repo_decode)?,
        kind,
        generation: row.try_get("generation").map_err(repo_decode)?,
        payload,
    })
}

fn review_notification_provider_name(provider: &IdentityProvider) -> String {
    match provider {
        // Product copy intentionally follows spec v10's provider spellings.
        IdentityProvider::OpenLibrary => "OpenLibrary".to_string(),
        IdentityProvider::Goodreads => "Goodreads".to_string(),
        IdentityProvider::Hardcover => "Hardcover".to_string(),
        IdentityProvider::IsbnRegistry => "ISBN".to_string(),
        IdentityProvider::Amazon => "Amazon".to_string(),
        IdentityProvider::Other(name) => name.clone(),
    }
}

fn authorize_review_actor(
    actor: &ReviewActor,
    user_id: UserId,
) -> Result<(), IdentityRepositoryError> {
    match actor {
        ReviewActor::AuthenticatedUser {
            user_id: actor_user_id,
        } if *actor_user_id == user_id => Ok(()),
        ReviewActor::CutoverOperator {
            installation_id,
            invocation_id,
        } if !installation_id.trim().is_empty() && !invocation_id.trim().is_empty() => Ok(()),
        _ => Err(IdentityRepositoryError::UnauthorizedScope),
    }
}

fn validate_review_resolution(
    pending: &PendingReviewCard,
    command: &ReviewResolutionCommand,
) -> Result<(), IdentityRepositoryError> {
    use livrarr_domain::identity_layer::{
        EditionEvidenceAction, FieldResolutionAction, GroupIdentityAction,
    };
    let valid = match (&pending.payload, command) {
        (
            SettlementReviewCard::IdentityConflict { .. },
            ReviewResolutionCommand::IdentityConflict { .. },
        )
        | (
            SettlementReviewCard::PendingRoute { .. },
            ReviewResolutionCommand::PendingRoute { .. },
        )
        | (
            SettlementReviewCard::ImportIdentity { .. },
            ReviewResolutionCommand::ImportIdentity { .. },
        )
        | (
            SettlementReviewCard::MigrationRepair { .. },
            ReviewResolutionCommand::MigrationRepair { .. },
        )
        | (
            SettlementReviewCard::InvariantRepair { .. },
            ReviewResolutionCommand::InvariantRepair { .. },
        ) => true,
        (
            SettlementReviewCard::GroupIdentity { work_ids, .. },
            ReviewResolutionCommand::GroupIdentity { action, .. },
        ) => match action {
            GroupIdentityAction::DifferentFromAll => true,
            GroupIdentityAction::AttachOrMerge { anchor } => work_ids.contains(anchor),
        },
        (
            SettlementReviewCard::FieldResolution { evidence_ids, .. },
            ReviewResolutionCommand::FieldResolution { action, .. },
        ) => match action {
            FieldResolutionAction::ExplicitAbsence => true,
            FieldResolutionAction::ChoosePreservedValue { evidence_id } => {
                evidence_ids.contains(evidence_id)
            }
        },
        (
            SettlementReviewCard::ContributorOrder { .. },
            ReviewResolutionCommand::ContributorOrder { order, primary, .. },
        ) => !order.is_empty() && order.contains(primary),
        (
            SettlementReviewCard::EditionEvidence { evidence_ids, .. },
            ReviewResolutionCommand::EditionEvidence { action, .. },
        ) => match action {
            EditionEvidenceAction::ChooseDirectEvidence { evidence_id } => {
                evidence_ids.contains(evidence_id)
            }
            EditionEvidenceAction::RetainUnknownOrAbsent
            | EditionEvidenceAction::ArchiveEmptyShell => true,
        },
        _ => false,
    };
    valid
        .then_some(())
        .ok_or(IdentityRepositoryError::InvalidResolution)
}

async fn validate_group_identity_proposal(
    tx: &mut Transaction<'_, Sqlite>,
    pending: &PendingReviewCard,
    command: &ReviewResolutionCommand,
) -> Result<(), IdentityRepositoryError> {
    let (
        SettlementReviewCard::GroupIdentity {
            work_ids,
            proposed_identity,
            ..
        },
        ReviewResolutionCommand::GroupIdentity { action, .. },
    ) = (&pending.payload, command)
    else {
        return Ok(());
    };
    if matches!(
        action,
        livrarr_domain::identity_layer::GroupIdentityAction::DifferentFromAll
    ) {
        return Ok(());
    }

    let anchor = pending.work_id.ok_or_else(|| {
        IdentityRepositoryError::ReviewProposalInvalidated(
            "proposed merge anchor no longer exists".to_string(),
        )
    })?;
    if !work_ids.contains(&anchor) {
        return Err(IdentityRepositoryError::InvalidResolution);
    }
    for work_id in work_ids {
        let exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM works WHERE user_id=?1 AND id=?2)")
                .bind(pending.user_id)
                .bind(work_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(repo_db)?;
        if !exists {
            return Err(IdentityRepositoryError::ReviewProposalInvalidated(
                "proposed merge work no longer exists".to_string(),
            ));
        }
    }

    if let Some(proposed) = proposed_identity {
        for route in &proposed.routes {
            let provider = serde_json::to_string(&route.provider).map_err(repo_json)?;
            let kind = serde_json::to_string(&route.kind).map_err(repo_json)?;
            let current_owner: Option<i64> = sqlx::query_scalar(
                "SELECT resolved_work_id FROM identity_routes \
                  WHERE user_id=?1 AND provider=?2 AND kind=?3 \
                    AND provider_scoped_id=?4 AND state='active' LIMIT 1",
            )
            .bind(pending.user_id)
            .bind(provider)
            .bind(kind)
            .bind(&route.provider_scoped_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(repo_db)?;
            if !current_owner.is_some_and(|owner| work_ids.contains(&owner)) {
                return Err(IdentityRepositoryError::ReviewProposalInvalidated(
                    "proposed route no longer belongs to the merge group".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn review_card_work_id(
    card: &livrarr_domain::identity_layer::SettlementReviewCard,
) -> Option<WorkId> {
    use livrarr_domain::identity_layer::SettlementReviewCard;
    match card {
        SettlementReviewCard::IdentityConflict { work_id, .. }
        | SettlementReviewCard::PendingRoute { work_id, .. }
        | SettlementReviewCard::FieldResolution { work_id, .. }
        | SettlementReviewCard::ContributorOrder { work_id, .. } => Some(*work_id),
        SettlementReviewCard::ImportIdentity { work_id, .. }
        | SettlementReviewCard::InvariantRepair { work_id, .. } => *work_id,
        SettlementReviewCard::GroupIdentity { work_ids, .. } => work_ids.first().copied(),
        SettlementReviewCard::EditionEvidence { .. }
        | SettlementReviewCard::MigrationRepair { .. } => None,
    }
}

/// Stable semantic identity for an unresolved group proposal. Operational
/// route fields (row/owner ids, provenance, observation time) and title
/// presentation/provenance are deliberately excluded: retriggered capture of
/// the same evidence must reuse the existing pending decision.
fn group_identity_proposal_key(
    card: &SettlementReviewCard,
) -> Result<Option<String>, IdentityRepositoryError> {
    let SettlementReviewCard::GroupIdentity {
        work_ids,
        proposed_identity,
        merge_choices,
    } = card
    else {
        return Ok(None);
    };

    let mut work_ids = work_ids.clone();
    work_ids.sort_unstable();
    work_ids.dedup();
    let proposed = proposed_identity
        .as_ref()
        .map(|identity| {
            let routes: Result<BTreeSet<String>, IdentityRepositoryError> = identity
                .routes
                .iter()
                .map(|route| {
                    serde_json::to_string(&(
                        &route.provider,
                        &route.kind,
                        &route.provider_scoped_id,
                    ))
                    .map_err(repo_json)
                })
                .collect();
            Ok((
                identity.title.normalized_main.clone(),
                identity.title.normalized_subtitle.clone(),
                identity.title.normalized_volume.clone(),
                identity.primary_author_id,
                routes?,
            ))
        })
        .transpose()?;
    let choices: Result<BTreeSet<String>, IdentityRepositoryError> = merge_choices
        .iter()
        .map(|choice| serde_json::to_string(choice).map_err(repo_json))
        .collect();
    serde_json::to_string(&(work_ids, proposed, choices?))
        .map(Some)
        .map_err(repo_json)
}

/// Stable semantic identity for a pending route proposal. Row ids and the
/// operational owner representation are excluded; user/work scope is already
/// fixed by the card row query.
fn pending_route_proposal_key(card: &SettlementReviewCard) -> Option<String> {
    let SettlementReviewCard::PendingRoute { candidate, .. } = card else {
        return None;
    };
    serde_json::to_string(&(
        &candidate.route.provider,
        &candidate.route.kind,
        candidate.route.value.trim(),
    ))
    .ok()
}

const IDENTITY_DEDUP_RESIDUE_HEAL_GENERATION: i64 = 1;

const IDENTITY_ROUND10_RESIDUE_HEAL_GENERATION: i64 = 1;

const IDENTITY_ROUND11_ATTEMPT_REHEAL_GENERATION: i64 = 1;

const IDENTITY_ROUND15_SEARCH_LEDGER_RESET_GENERATION: i64 = 1;

const IDENTITY_ROUND15_GR_COVER_RESELECT_GENERATION: i64 = 1;

const IDENTITY_ROUND21_GOODREADS_NAMESPACE_HEAL_GENERATION: i64 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentityRound10ResidueHealReport {
    pub bridge_attempts_cleared: usize,
    pub dishonest_enriched_reclassified: usize,
    pub failed_readarr_authors_deleted: usize,
}

/// Re-open convergence work that the former self-feeding handoff charged,
/// reclassify only pre-fix `enriched` stamps with no source and no real
/// provider call, and remove the narrowly-scoped 2026-08-17 Readarr author
/// residue. All predicates and the completion marker commit atomically.
pub async fn heal_identity_round10_residue(
    pool: &SqlitePool,
) -> Result<IdentityRound10ResidueHealReport, String> {
    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|error| format!("begin identity round-10 residue heal: {error}"))?;
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta \
          WHERE key='identity_round10_residue_heal_generation'",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| format!("read identity round-10 residue marker: {error}"))?;
    if marker
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|generation| generation >= IDENTITY_ROUND10_RESIDUE_HEAL_GENERATION)
    {
        tx.rollback()
            .await
            .map_err(|error| format!("close completed round-10 heal: {error}"))?;
        return Ok(IdentityRound10ResidueHealReport::default());
    }

    let bridge_attempts_cleared = sqlx::query(
        "DELETE FROM identity_provider_attempts \
          WHERE provider='livrarr-convergence' AND route_kind='bridge-upgrade'",
    )
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("clear dishonest convergence attempts: {error}"))?
    .rows_affected() as usize;

    let dishonest_enriched_reclassified = sqlx::query(
        "UPDATE works AS w SET enrichment_status='failed' \
          WHERE w.enrichment_status='enriched' AND w.enrichment_source IS NULL \
            AND NOT EXISTS (SELECT 1 FROM provider_call_records p \
                             WHERE p.work_id=w.id \
                               AND p.outcome NOT IN ('skipped_no_anchor','skipped_policy'))",
    )
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("reclassify dishonest enrichment stamps: {error}"))?
    .rows_affected() as usize;

    let failed_readarr_authors_deleted = sqlx::query(
        "DELETE FROM authors AS a \
          WHERE datetime(a.added_at)>=datetime('2026-08-17T00:00:00Z') \
            AND datetime(a.added_at)<datetime('2026-08-18T00:00:00Z') \
            AND a.import_id IS NOT NULL \
            AND EXISTS (SELECT 1 FROM imports i WHERE i.id=a.import_id \
                         AND i.user_id=a.user_id AND i.source='readarr') \
            AND a.monitored=0 AND a.monitor_new_items=0 \
            AND a.monitor_since IS NULL AND a.monitor_language IS NULL \
            AND NOT EXISTS (SELECT 1 FROM works w WHERE w.user_id=a.user_id \
                             AND (w.author_id=a.id OR w.primary_author_id=a.id)) \
            AND NOT EXISTS (SELECT 1 FROM work_contributors wc \
                             WHERE wc.user_id=a.user_id AND wc.author_id=a.id) \
            AND NOT EXISTS (SELECT 1 FROM series s \
                             WHERE s.user_id=a.user_id AND s.author_id=a.id) \
            AND NOT EXISTS (SELECT 1 FROM author_provider_routes r \
                             WHERE r.user_id=a.user_id AND r.author_id=a.id \
                               AND (r.provenance='user_picked' \
                                    OR r.removed_by_user_id IS NOT NULL)) \
            AND NOT EXISTS (SELECT 1 FROM author_name_variants n \
                             WHERE n.user_id=a.user_id AND n.author_id=a.id \
                               AND (n.source='user' OR n.user_selected_at IS NOT NULL)) \
            AND NOT EXISTS (SELECT 1 FROM author_link_candidates c \
                             WHERE c.user_id=a.user_id AND c.author_id=a.id \
                               AND c.status='picked')",
    )
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("delete failed Readarr author residue: {error}"))?
    .rows_affected() as usize;

    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) \
         VALUES ('identity_round10_residue_heal_generation', ?1) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(IDENTITY_ROUND10_RESIDUE_HEAL_GENERATION.to_string())
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("stamp identity round-10 residue marker: {error}"))?;
    tx.commit()
        .await
        .map_err(|error| format!("commit identity round-10 residue heal: {error}"))?;
    Ok(IdentityRound10ResidueHealReport {
        bridge_attempts_cleared,
        dishonest_enriched_reclassified,
        failed_readarr_authors_deleted,
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentityRound11AttemptRehealReport {
    pub bridge_attempts_cleared: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentityRound15SearchLedgerResetReport {
    pub edition_only_works_reopened: usize,
    pub edition_only_attempts_cleared: usize,
    pub zero_route_works_reopened: usize,
    pub zero_route_attempts_cleared: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentityRound21GoodreadsNamespaceHealReport {
    pub owned_file_routes_relabelled: usize,
    pub migrated_gr_key_routes_relabelled: usize,
    pub editions_created: usize,
    pub works_advanced: usize,
    pub retry_works_reset: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentityRound15GrCoverReselectTarget {
    pub user_id: UserId,
    pub work_id: WorkId,
    pub ebook: bool,
    pub audiobook: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityRound15GrCoverReselectPlan {
    pub targets: Vec<IdentityRound15GrCoverReselectTarget>,
    pub ebook_slots: usize,
    pub audiobook_slots: usize,
    pub manual_ebook_slots_preserved: usize,
    pub manual_audiobook_slots_preserved: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentityRound15GrCoverSlotsCleared {
    pub ebook: bool,
    pub audiobook: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentityRound15GrCoverReselectCompletion {
    pub queued_works: usize,
    pub automatic_target_works: usize,
}

/// Repair only Goodreads Work routes whose provenance proves that the value
/// came from the legacy Book-page namespace. SearchFallback and
/// TextDecisiveSearchFallback routes are intentionally excluded: their values
/// are genuine autocomplete `workId`s. Route re-homing, generation changes,
/// retry invalidation, audit rows, and the one-shot marker are one transaction.
pub async fn heal_identity_round21_goodreads_namespace(
    pool: &SqlitePool,
) -> Result<IdentityRound21GoodreadsNamespaceHealReport, String> {
    #[derive(Debug, Clone, Copy)]
    enum ProvenBookSource {
        OwnedFile,
        MigratedGrKey,
    }

    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|error| format!("begin identity round-21 Goodreads namespace heal: {error}"))?;
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta \
          WHERE key='identity_round21_goodreads_book_namespace_heal'",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| format!("read identity round-21 Goodreads namespace marker: {error}"))?;
    if marker
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|generation| {
            generation >= IDENTITY_ROUND21_GOODREADS_NAMESPACE_HEAL_GENERATION
        })
    {
        tx.rollback()
            .await
            .map_err(|error| format!("close completed round-21 namespace heal: {error}"))?;
        return Ok(IdentityRound21GoodreadsNamespaceHealReport::default());
    }

    let goodreads = serde_json::to_string(&IdentityProvider::Goodreads)
        .map_err(|error| format!("encode round-21 Goodreads provider: {error}"))?;
    let work_kind = serde_json::to_string(&RouteKind::GoodreadsWork)
        .map_err(|error| format!("encode round-21 Goodreads Work kind: {error}"))?;
    let book_kind = serde_json::to_string(&RouteKind::GoodreadsBookEdition)
        .map_err(|error| format!("encode round-21 Goodreads Book kind: {error}"))?;
    let unknown_format = serde_json::to_string(&EditionFormat::Unknown)
        .map_err(|error| format!("encode round-21 unknown Edition format: {error}"))?;
    let candidates = sqlx::query(
        "SELECT id, user_id, resolved_work_id, provider_scoped_id, provenance \
           FROM identity_routes \
          WHERE provider=?1 AND kind=?2 AND state='active' \
          ORDER BY user_id, resolved_work_id, id",
    )
    .bind(&goodreads)
    .bind(&work_kind)
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| format!("list round-21 Goodreads namespace candidates: {error}"))?;

    let mut report = IdentityRound21GoodreadsNamespaceHealReport::default();
    let mut route_graphs: BTreeMap<(UserId, WorkId), ActiveRouteGraph> = BTreeMap::new();
    let mut work_counts: BTreeMap<(UserId, WorkId), (usize, usize)> = BTreeMap::new();

    for row in candidates {
        let provenance_json: String = row
            .try_get("provenance")
            .map_err(|error| format!("decode round-21 route provenance: {error}"))?;
        let provenance: RouteProvenance = serde_json::from_str(&provenance_json)
            .map_err(|error| format!("parse round-21 route provenance: {error}"))?;
        let source = match provenance {
            RouteProvenance::OwnedFile { .. } => ProvenBookSource::OwnedFile,
            RouteProvenance::Migrated { ref legacy_field } if legacy_field == "gr_key" => {
                ProvenBookSource::MigratedGrKey
            }
            _ => continue,
        };
        let route_id: i64 = row
            .try_get("id")
            .map_err(|error| format!("decode round-21 route id: {error}"))?;
        let user_id: UserId = row
            .try_get("user_id")
            .map_err(|error| format!("decode round-21 route user: {error}"))?;
        let work_id: WorkId = row
            .try_get("resolved_work_id")
            .map_err(|error| format!("decode round-21 route work: {error}"))?;
        let value: String = row
            .try_get("provider_scoped_id")
            .map_err(|error| format!("decode round-21 Goodreads Book id: {error}"))?;

        if let std::collections::btree_map::Entry::Vacant(entry) =
            route_graphs.entry((user_id, work_id))
        {
            let before = read_active_route_graph(&mut tx, user_id, work_id)
                .await
                .map_err(|error| format!("snapshot round-21 route graph: {error}"))?;
            entry.insert(before);
        }

        let active_book_routes = sqlx::query(
            "SELECT id, resolved_work_id, edition_id FROM identity_routes \
              WHERE user_id=?1 AND provider=?2 AND kind=?3 AND provider_scoped_id=?4 \
                AND state='active' AND id<>?5 ORDER BY id",
        )
        .bind(user_id)
        .bind(&goodreads)
        .bind(&book_kind)
        .bind(&value)
        .bind(route_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|error| format!("read existing round-21 Goodreads Book route: {error}"))?;

        let edition_id = if let Some(existing) = active_book_routes.first() {
            for route in &active_book_routes {
                let existing_work: WorkId = route.try_get("resolved_work_id").map_err(|error| {
                    format!("decode existing round-21 Goodreads Book owner: {error}")
                })?;
                if existing_work != work_id {
                    return Err(format!(
                        "round-21 Goodreads Book id {value} is already active on work {existing_work}"
                    ));
                }
            }
            let edition_id: Option<EditionId> = existing
                .try_get("edition_id")
                .map_err(|error| format!("decode existing round-21 Edition owner: {error}"))?;
            let edition_id = edition_id.ok_or_else(|| {
                format!("round-21 Goodreads Book route for {value} has no Edition owner")
            })?;
            let edition_work: Option<WorkId> = sqlx::query_scalar(
                "SELECT work_id FROM editions \
                  WHERE user_id=?1 AND id=?2 AND state='active'",
            )
            .bind(user_id)
            .bind(edition_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|error| format!("verify existing round-21 Edition owner: {error}"))?;
            if edition_work != Some(work_id) {
                return Err(format!(
                    "round-21 Goodreads Book Edition {edition_id} is not active on work {work_id}"
                ));
            }
            // Preserve the proven-Book route row and its provenance in place;
            // any already-correct duplicate becomes retained history.
            sqlx::query(
                "UPDATE identity_routes SET state='retired' \
                  WHERE user_id=?1 AND provider=?2 AND kind=?3 AND provider_scoped_id=?4 \
                    AND state='active' AND id<>?5",
            )
            .bind(user_id)
            .bind(&goodreads)
            .bind(&book_kind)
            .bind(&value)
            .bind(route_id)
            .execute(&mut *tx)
            .await
            .map_err(|error| format!("retire duplicate round-21 Goodreads Book route: {error}"))?;
            edition_id
        } else {
            let existing_editions = sqlx::query(
                "SELECT id, work_id FROM editions \
                  WHERE user_id=?1 AND source_provider=?2 AND provider_edition_id=?3 \
                    AND state='active' ORDER BY id",
            )
            .bind(user_id)
            .bind(&goodreads)
            .bind(&value)
            .fetch_all(&mut *tx)
            .await
            .map_err(|error| format!("read existing round-21 Goodreads Edition: {error}"))?;
            if let Some(existing) = existing_editions.first() {
                for edition in &existing_editions {
                    let edition_work: WorkId = edition.try_get("work_id").map_err(|error| {
                        format!("decode existing round-21 Goodreads Edition work: {error}")
                    })?;
                    if edition_work != work_id {
                        return Err(format!(
                            "round-21 Goodreads Edition id {value} already belongs to work {edition_work}"
                        ));
                    }
                }
                existing
                    .try_get("id")
                    .map_err(|error| format!("decode existing round-21 Edition id: {error}"))?
            } else {
                report.editions_created += 1;
                sqlx::query(
                    "INSERT INTO editions \
                        (user_id, work_id, format, source_provider, provider_edition_id, state) \
                     VALUES (?1, ?2, ?3, ?4, ?5, 'active')",
                )
                .bind(user_id)
                .bind(work_id)
                .bind(&unknown_format)
                .bind(&goodreads)
                .bind(&value)
                .execute(&mut *tx)
                .await
                .map_err(|error| format!("create round-21 Goodreads Edition: {error}"))?
                .last_insert_rowid()
            }
        };

        let updated = sqlx::query(
            "UPDATE identity_routes \
                SET owner_type='edition', work_id=NULL, edition_id=?1, kind=?2 \
              WHERE id=?3 AND user_id=?4 AND resolved_work_id=?5 \
                AND provider=?6 AND kind=?7 AND state='active'",
        )
        .bind(edition_id)
        .bind(&book_kind)
        .bind(route_id)
        .bind(user_id)
        .bind(work_id)
        .bind(&goodreads)
        .bind(&work_kind)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("relabel round-21 Goodreads Book route: {error}"))?;
        if updated.rows_affected() != 1 {
            return Err(format!(
                "round-21 Goodreads route {route_id} changed during namespace repair"
            ));
        }
        let counts = work_counts.entry((user_id, work_id)).or_default();
        match source {
            ProvenBookSource::OwnedFile => {
                report.owned_file_routes_relabelled += 1;
                counts.0 += 1;
            }
            ProvenBookSource::MigratedGrKey => {
                report.migrated_gr_key_routes_relabelled += 1;
                counts.1 += 1;
            }
        }
    }

    let now = chrono::Utc::now().to_rfc3339();
    for ((user_id, work_id), before) in route_graphs {
        let (owned_count, migrated_count) = work_counts
            .get(&(user_id, work_id))
            .copied()
            .unwrap_or_default();
        if owned_count + migrated_count == 0 {
            continue;
        }
        let advanced = sqlx::query(
            "UPDATE works SET identity_generation=identity_generation+1 \
              WHERE user_id=?1 AND id=?2",
        )
        .bind(user_id)
        .bind(work_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("advance round-21 identity generation: {error}"))?;
        if advanced.rows_affected() != 1 {
            return Err(format!("round-21 namespace repair lost work {work_id}"));
        }
        report.works_advanced += 1;
        if invalidate_retry_state_if_route_graph_changed(&mut tx, user_id, work_id, &before)
            .await
            .map_err(|error| format!("reset round-21 provider retry standing: {error}"))?
        {
            report.retry_works_reset += 1;
        }
        let generation: i64 =
            sqlx::query_scalar("SELECT identity_generation FROM works WHERE user_id=?1 AND id=?2")
                .bind(user_id)
                .bind(work_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|error| format!("read round-21 repaired generation: {error}"))?;
        sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'round21-goodreads-book-namespace-heal', \
                     'startup-heal', ?3, ?4)",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(format!(
            "generation={generation};owned_file_routes={owned_count};migrated_gr_key_routes={migrated_count};from=GoodreadsWork;to=GoodreadsBookEdition"
        ))
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("audit round-21 Goodreads namespace heal: {error}"))?;
    }

    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) \
         VALUES ('identity_round21_goodreads_book_namespace_heal', ?1) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(IDENTITY_ROUND21_GOODREADS_NAMESPACE_HEAL_GENERATION.to_string())
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("stamp identity round-21 Goodreads namespace marker: {error}"))?;
    tx.commit()
        .await
        .map_err(|error| format!("commit identity round-21 Goodreads namespace heal: {error}"))?;
    Ok(report)
}

/// Snapshot the exact automatic Goodreads-sourced slots the startup repair
/// may clear. The durable queue is seeded before any slot is mutated, so a
/// crash after clear but before re-materialization resumes the same work on
/// the next startup. A completed marker makes the second run all-zero.
pub async fn plan_identity_round15_gr_cover_reselect(
    pool: &SqlitePool,
) -> Result<IdentityRound15GrCoverReselectPlan, String> {
    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|error| format!("begin identity round-15 GR cover plan: {error}"))?;
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta \
          WHERE key='identity_round15_gr_cover_reselect'",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| format!("read identity round-15 GR cover marker: {error}"))?;
    if marker
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|generation| generation >= IDENTITY_ROUND15_GR_COVER_RESELECT_GENERATION)
    {
        tx.rollback()
            .await
            .map_err(|error| format!("close completed round-15 GR cover plan: {error}"))?;
        return Ok(IdentityRound15GrCoverReselectPlan::default());
    }

    sqlx::query(
        "INSERT INTO identity_round15_gr_cover_reselect_queue \
             (user_id, work_id, ebook, audiobook) \
         SELECT user_id, id, \
                COALESCE(cover_source='goodreads' AND cover_manual=0, 0), \
                COALESCE(audiobook_cover_source='goodreads' \
                         AND audiobook_cover_manual=0, 0) \
           FROM works \
          WHERE (cover_source='goodreads' AND cover_manual=0) \
             OR (audiobook_cover_source='goodreads' AND audiobook_cover_manual=0) \
         ON CONFLICT(user_id, work_id) DO NOTHING",
    )
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("seed identity round-15 GR cover queue: {error}"))?;

    let rows = sqlx::query(
        "SELECT user_id, work_id, ebook, audiobook \
           FROM identity_round15_gr_cover_reselect_queue \
          ORDER BY user_id, work_id",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| format!("list identity round-15 GR cover queue: {error}"))?;
    let mut plan = IdentityRound15GrCoverReselectPlan::default();
    for row in rows {
        let ebook = row
            .try_get::<i64, _>("ebook")
            .map_err(|error| format!("decode round-15 GR ebook target: {error}"))?
            != 0;
        let audiobook = row
            .try_get::<i64, _>("audiobook")
            .map_err(|error| format!("decode round-15 GR audiobook target: {error}"))?
            != 0;
        plan.ebook_slots += usize::from(ebook);
        plan.audiobook_slots += usize::from(audiobook);
        plan.targets.push(IdentityRound15GrCoverReselectTarget {
            user_id: row
                .try_get("user_id")
                .map_err(|error| format!("decode round-15 GR cover user: {error}"))?,
            work_id: row
                .try_get("work_id")
                .map_err(|error| format!("decode round-15 GR cover work: {error}"))?,
            ebook,
            audiobook,
        });
    }

    let manual_counts = sqlx::query(
        "SELECT \
             COUNT(CASE WHEN cover_source='goodreads' AND cover_manual=1 THEN 1 END) AS ebook, \
             COUNT(CASE WHEN audiobook_cover_source='goodreads' \
                              AND audiobook_cover_manual=1 THEN 1 END) AS audiobook \
           FROM works",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| format!("count protected round-15 GR cover slots: {error}"))?;
    plan.manual_ebook_slots_preserved = manual_counts
        .try_get::<i64, _>("ebook")
        .map_err(|error| format!("decode protected round-15 GR ebook count: {error}"))?
        as usize;
    plan.manual_audiobook_slots_preserved = manual_counts
        .try_get::<i64, _>("audiobook")
        .map_err(|error| format!("decode protected round-15 GR audiobook count: {error}"))?
        as usize;
    tx.commit()
        .await
        .map_err(|error| format!("commit identity round-15 GR cover plan: {error}"))?;
    Ok(plan)
}

/// Clear one queued work's still-machine-owned Goodreads slots in one write
/// transaction. Each manual predicate is rechecked at mutation time. The
/// returned flags say which queued slots remain machine-owned and therefore
/// still need selection/materialization (including crash-resumed cleared slots).
pub async fn clear_identity_round15_gr_cover_slots(
    pool: &SqlitePool,
    target: IdentityRound15GrCoverReselectTarget,
) -> Result<IdentityRound15GrCoverSlotsCleared, String> {
    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|error| format!("begin identity round-15 GR cover clear: {error}"))?;
    if target.ebook {
        sqlx::query(
            "UPDATE works \
                SET cover_url=NULL, cover_source=NULL, cover_width=0, cover_height=0 \
              WHERE user_id=?1 AND id=?2 \
                AND cover_source='goodreads' AND cover_manual=0",
        )
        .bind(target.user_id)
        .bind(target.work_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("clear round-15 GR ebook slot: {error}"))?;
    }
    if target.audiobook {
        sqlx::query(
            "UPDATE works \
                SET audiobook_cover_url=NULL, audiobook_cover_source=NULL, \
                    audiobook_cover_width=0, audiobook_cover_height=0 \
              WHERE user_id=?1 AND id=?2 \
                AND audiobook_cover_source='goodreads' \
                AND audiobook_cover_manual=0",
        )
        .bind(target.user_id)
        .bind(target.work_id)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("clear round-15 GR audiobook slot: {error}"))?;
    }

    let state: Option<(bool, bool)> = sqlx::query_as(
        "SELECT cover_manual, audiobook_cover_manual \
           FROM works WHERE user_id=?1 AND id=?2",
    )
    .bind(target.user_id)
    .bind(target.work_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| format!("read round-15 GR cover ownership after clear: {error}"))?;
    tx.commit()
        .await
        .map_err(|error| format!("commit identity round-15 GR cover clear: {error}"))?;
    let Some((ebook_manual, audiobook_manual)) = state else {
        return Ok(IdentityRound15GrCoverSlotsCleared::default());
    };
    Ok(IdentityRound15GrCoverSlotsCleared {
        ebook: target.ebook && !ebook_manual,
        audiobook: target.audiobook && !audiobook_manual,
    })
}

/// Remove a repaired work from the durable startup worklist only after its
/// changed slots have passed through the materialization gate.
pub async fn finish_identity_round15_gr_cover_target(
    pool: &SqlitePool,
    target: IdentityRound15GrCoverReselectTarget,
) -> Result<(), String> {
    sqlx::query(
        "DELETE FROM identity_round15_gr_cover_reselect_queue \
          WHERE user_id=?1 AND work_id=?2",
    )
    .bind(target.user_id)
    .bind(target.work_id)
    .execute(pool)
    .await
    .map_err(|error| format!("finish identity round-15 GR cover target: {error}"))?;
    Ok(())
}

/// Stamp the one-shot only after the durable worklist and every automatic
/// Goodreads predicate are empty. An interrupted repair therefore remains
/// resumable and can never be mistaken for completion. A partial worklist is
/// a successful, unstamped pass whose remaining counts are returned to startup.
pub async fn complete_identity_round15_gr_cover_reselect(
    pool: &SqlitePool,
) -> Result<IdentityRound15GrCoverReselectCompletion, String> {
    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|error| format!("begin identity round-15 GR cover completion: {error}"))?;
    let queued: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM identity_round15_gr_cover_reselect_queue")
            .fetch_one(&mut *tx)
            .await
            .map_err(|error| format!("verify round-15 GR cover queue: {error}"))?;
    let remaining: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM works \
          WHERE (cover_source='goodreads' AND cover_manual=0) \
             OR (audiobook_cover_source='goodreads' AND audiobook_cover_manual=0)",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| format!("verify round-15 GR cover completion: {error}"))?;
    let completion = IdentityRound15GrCoverReselectCompletion {
        queued_works: queued as usize,
        automatic_target_works: remaining as usize,
    };
    if queued != 0 || remaining != 0 {
        tx.rollback()
            .await
            .map_err(|error| format!("close incomplete round-15 GR cover repair: {error}"))?;
        return Ok(completion);
    }
    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) \
         VALUES ('identity_round15_gr_cover_reselect', ?1) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(IDENTITY_ROUND15_GR_COVER_RESELECT_GENERATION.to_string())
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("stamp identity round-15 GR cover marker: {error}"))?;
    tx.commit()
        .await
        .map_err(|error| format!("commit identity round-15 GR cover completion: {error}"))?;
    Ok(completion)
}

/// Re-open only REQ-027 search ledgers whose Work still has no active
/// provider-work route. Edition-only and zero/non-edition route classes are
/// counted separately; the counts, scoped delete, and completion marker share
/// one transaction.
pub async fn heal_identity_round15_search_ledger(
    pool: &SqlitePool,
) -> Result<IdentityRound15SearchLedgerResetReport, String> {
    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|error| format!("begin identity round-15 search ledger reset: {error}"))?;
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta \
          WHERE key='identity_round15_search_ledger_reset'",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| format!("read identity round-15 search ledger marker: {error}"))?;
    if marker
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|generation| generation >= IDENTITY_ROUND15_SEARCH_LEDGER_RESET_GENERATION)
    {
        tx.rollback()
            .await
            .map_err(|error| format!("close completed round-15 ledger reset: {error}"))?;
        return Ok(IdentityRound15SearchLedgerResetReport::default());
    }

    let counts = sqlx::query(
        "SELECT \
           COUNT(DISTINCT CASE WHEN EXISTS ( \
               SELECT 1 FROM identity_routes e \
                WHERE e.user_id=a.user_id AND e.resolved_work_id=a.work_id \
                  AND e.state='active' \
                  AND e.kind IN ('\"Isbn13Edition\"','\"AsinEdition\"','\"GoodreadsBookEdition\"') \
           ) THEN a.work_id END) AS edition_works, \
           COALESCE(SUM(CASE WHEN EXISTS ( \
               SELECT 1 FROM identity_routes e \
                WHERE e.user_id=a.user_id AND e.resolved_work_id=a.work_id \
                  AND e.state='active' \
                  AND e.kind IN ('\"Isbn13Edition\"','\"AsinEdition\"','\"GoodreadsBookEdition\"') \
           ) THEN 1 ELSE 0 END), 0) AS edition_attempts, \
           COUNT(DISTINCT CASE WHEN NOT EXISTS ( \
               SELECT 1 FROM identity_routes e \
                WHERE e.user_id=a.user_id AND e.resolved_work_id=a.work_id \
                  AND e.state='active' \
                  AND e.kind IN ('\"Isbn13Edition\"','\"AsinEdition\"','\"GoodreadsBookEdition\"') \
           ) THEN a.work_id END) AS zero_works, \
           COALESCE(SUM(CASE WHEN NOT EXISTS ( \
               SELECT 1 FROM identity_routes e \
                WHERE e.user_id=a.user_id AND e.resolved_work_id=a.work_id \
                  AND e.state='active' \
                  AND e.kind IN ('\"Isbn13Edition\"','\"AsinEdition\"','\"GoodreadsBookEdition\"') \
           ) THEN 1 ELSE 0 END), 0) AS zero_attempts \
         FROM identity_provider_attempts a \
        WHERE a.provider='livrarr-convergence' AND a.route_kind='bridge-upgrade' \
          AND NOT EXISTS ( \
              SELECT 1 FROM identity_routes w \
               WHERE w.user_id=a.user_id AND w.resolved_work_id=a.work_id \
                 AND w.state='active' \
                 AND w.kind IN ('\"OpenLibraryWork\"','\"GoodreadsWork\"','\"HardcoverWork\"') \
          )",
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(|error| format!("count round-15 search ledger classes: {error}"))?;
    let report = IdentityRound15SearchLedgerResetReport {
        edition_only_works_reopened: counts
            .try_get::<i64, _>("edition_works")
            .map_err(|error| format!("decode round-15 edition work count: {error}"))?
            as usize,
        edition_only_attempts_cleared: counts
            .try_get::<i64, _>("edition_attempts")
            .map_err(|error| format!("decode round-15 edition attempt count: {error}"))?
            as usize,
        zero_route_works_reopened: counts
            .try_get::<i64, _>("zero_works")
            .map_err(|error| format!("decode round-15 zero-route work count: {error}"))?
            as usize,
        zero_route_attempts_cleared: counts
            .try_get::<i64, _>("zero_attempts")
            .map_err(|error| format!("decode round-15 zero-route attempt count: {error}"))?
            as usize,
    };

    let deleted = sqlx::query(
        "DELETE FROM identity_provider_attempts \
          WHERE provider='livrarr-convergence' AND route_kind='bridge-upgrade' \
            AND NOT EXISTS ( \
                SELECT 1 FROM identity_routes w \
                 WHERE w.user_id=identity_provider_attempts.user_id \
                   AND w.resolved_work_id=identity_provider_attempts.work_id \
                   AND w.state='active' \
                   AND w.kind IN ('\"OpenLibraryWork\"','\"GoodreadsWork\"','\"HardcoverWork\"') \
            )",
    )
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("delete round-15 parked search attempts: {error}"))?
    .rows_affected() as usize;
    if deleted != report.edition_only_attempts_cleared + report.zero_route_attempts_cleared {
        return Err(format!(
            "round-15 search ledger count/delete mismatch: counted {}, deleted {deleted}",
            report.edition_only_attempts_cleared + report.zero_route_attempts_cleared
        ));
    }

    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) \
         VALUES ('identity_round15_search_ledger_reset', ?1) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(IDENTITY_ROUND15_SEARCH_LEDGER_RESET_GENERATION.to_string())
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("stamp identity round-15 search ledger marker: {error}"))?;
    tx.commit()
        .await
        .map_err(|error| format!("commit identity round-15 search ledger reset: {error}"))?;
    Ok(report)
}

/// Generation 2 of the round-10 attempt heal. Round 10 briefly charged
/// cache-served convergence visits as real provider chases, so re-open only
/// that exact ledger class once after the real-fetch accounting fix lands.
/// The delete and completion marker share one write transaction.
pub async fn heal_identity_round11_attempt_residue(
    pool: &SqlitePool,
) -> Result<IdentityRound11AttemptRehealReport, String> {
    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|error| format!("begin identity round-11 attempt re-heal: {error}"))?;
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key='identity_round11_attempt_reheal'",
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(|error| format!("read identity round-11 attempt re-heal marker: {error}"))?;
    if marker
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|generation| generation >= IDENTITY_ROUND11_ATTEMPT_REHEAL_GENERATION)
    {
        tx.rollback()
            .await
            .map_err(|error| format!("close completed round-11 attempt re-heal: {error}"))?;
        return Ok(IdentityRound11AttemptRehealReport::default());
    }

    let bridge_attempts_cleared = sqlx::query(
        "DELETE FROM identity_provider_attempts \
          WHERE provider='livrarr-convergence' AND route_kind='bridge-upgrade'",
    )
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("clear cache-counted convergence attempts: {error}"))?
    .rows_affected() as usize;

    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) \
         VALUES ('identity_round11_attempt_reheal', ?1) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(IDENTITY_ROUND11_ATTEMPT_REHEAL_GENERATION.to_string())
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("stamp identity round-11 attempt re-heal marker: {error}"))?;
    tx.commit()
        .await
        .map_err(|error| format!("commit identity round-11 attempt re-heal: {error}"))?;
    Ok(IdentityRound11AttemptRehealReport {
        bridge_attempts_cleared,
    })
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IdentityDedupResidueHealReport {
    pub orphans_folded: usize,
    pub works_bumped: usize,
    pub invalid_cards_cancelled: usize,
    pub duplicate_cards_cancelled: usize,
}

/// Fold the route-less Work left by the former direct-add review arm and
/// collapse pre-fix duplicate pending GroupIdentity proposals. The orphan
/// signature is intentionally conservative: it requires the exact proposed
/// identity, creation/card/audit proximity, generation-one engine creation,
/// no enrichment, and no dependent or user-authored data. The legacy Work
/// schema defaults ebook monitoring on, so that derived flag is not treated
/// as proof of independent intent.
pub async fn heal_identity_dedup_residue(
    pool: &SqlitePool,
) -> Result<IdentityDedupResidueHealReport, String> {
    let marker: Option<String> = sqlx::query_scalar(
        "SELECT value FROM _livrarr_meta WHERE key='identity_dedup_residue_heal_generation'",
    )
    .fetch_optional(pool)
    .await
    .map_err(|error| format!("read identity dedup-residue heal generation: {error}"))?;
    if marker
        .and_then(|value| value.parse::<i64>().ok())
        .is_some_and(|generation| generation >= IDENTITY_DEDUP_RESIDUE_HEAL_GENERATION)
    {
        return Ok(IdentityDedupResidueHealReport::default());
    }

    type CardRow = (i64, i64, Option<i64>, String, String);
    let mut tx = crate::pool::begin_write(pool)
        .await
        .map_err(|error| format!("begin identity dedup-residue heal: {error}"))?;
    let cards: Vec<CardRow> = sqlx::query_as(
        "SELECT id, user_id, work_id, payload, created_at FROM identity_review_cards \
          WHERE kind=?1 AND status='pending' ORDER BY id",
    )
    .bind(ReviewKind::GroupIdentity.storage_code())
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| format!("select pending GroupIdentity residue: {error}"))?;
    let mut decoded = Vec::with_capacity(cards.len());
    for (card_id, user_id, work_id, payload, created_at) in cards {
        let card: SettlementReviewCard = serde_json::from_str(&payload)
            .map_err(|error| format!("decode GroupIdentity card {card_id}: {error}"))?;
        decoded.push((card_id, user_id, work_id, card, created_at));
    }

    let mut folds = Vec::new();
    let mut claimed_orphans = BTreeSet::new();
    for (card_id, user_id, _, card, card_created_at) in &decoded {
        let SettlementReviewCard::GroupIdentity {
            work_ids,
            proposed_identity: Some(proposed),
            ..
        } = card
        else {
            continue;
        };
        if work_ids.is_empty() || proposed.routes.is_empty() {
            continue;
        }
        let anchor = work_ids[0];
        let anchor_exists: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM works WHERE user_id=?1 AND id=?2)")
                .bind(user_id)
                .bind(anchor)
                .fetch_one(&mut *tx)
                .await
                .map_err(|error| format!("check dedup-residue anchor {anchor}: {error}"))?;
        if !anchor_exists {
            continue;
        }

        let candidates: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, added_at FROM works \
              WHERE user_id=?1 AND normalized_identity_main=?2 \
                AND normalized_identity_subtitle=?3 AND normalized_identity_volume=?4 \
                AND primary_author_id=?5 AND identity_generation=1 \
                AND identity_status_v2='not_connected' \
                AND enrichment_status='pending' AND enriched_at IS NULL \
                AND series_id IS NULL AND import_id IS NULL \
                AND next_convergence_at IS NULL \
                AND cover_url IS NULL AND audiobook_cover_url IS NULL \
              ORDER BY id",
        )
        .bind(user_id)
        .bind(&proposed.title.normalized_main)
        .bind(&proposed.title.normalized_subtitle)
        .bind(&proposed.title.normalized_volume)
        .bind(proposed.primary_author_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|error| format!("find orphan candidates for card {card_id}: {error}"))?;
        let card_time = chrono::DateTime::parse_from_rfc3339(card_created_at).ok();
        let mut matches = Vec::new();
        for (candidate_id, added_at) in candidates {
            if work_ids.contains(&candidate_id) || claimed_orphans.contains(&candidate_id) {
                continue;
            }
            let close_to_card = card_time.is_some_and(|card_time| {
                chrono::DateTime::parse_from_rfc3339(&added_at)
                    .ok()
                    .is_some_and(|added| {
                        (added.timestamp_millis() - card_time.timestamp_millis()).abs() <= 10_000
                    })
            });
            if !close_to_card
                || !dedup_orphan_has_only_engine_creation(&mut tx, *user_id, candidate_id).await?
                || !dedup_orphan_has_no_dependents(&mut tx, candidate_id).await?
            {
                continue;
            }
            matches.push(candidate_id);
        }
        if matches.len() == 1 {
            claimed_orphans.insert(matches[0]);
            folds.push((*user_id, anchor, matches[0]));
        }
    }

    let mut report = IdentityDedupResidueHealReport::default();
    for (user_id, anchor, orphan) in folds {
        for (card_id, card_user_id, card_work_id, card, _) in &decoded {
            let SettlementReviewCard::GroupIdentity { work_ids, .. } = card else {
                continue;
            };
            if *card_user_id == user_id && work_ids.contains(&orphan) {
                let audit_work_id = card_work_id
                    .filter(|work_id| *work_id != orphan)
                    .unwrap_or(anchor);
                if cancel_pending_group_card(
                    &mut tx,
                    user_id,
                    *card_id,
                    audit_work_id,
                    "orphan-work-folded",
                )
                .await?
                {
                    report.invalid_cards_cancelled += 1;
                }
            }
        }
        absorb_work_into(&mut tx, user_id, anchor, orphan)
            .await
            .map_err(|error| {
                format!("fold dedup-residue Work {orphan} into {anchor}: {error:?}")
            })?;
        let updated = sqlx::query(
            "UPDATE works SET identity_generation=identity_generation+1 \
              WHERE user_id=?1 AND id=?2",
        )
        .bind(user_id)
        .bind(anchor)
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("bump dedup-residue anchor {anchor}: {error}"))?;
        if updated.rows_affected() != 1 {
            return Err(format!("dedup-residue anchor Work {anchor} disappeared"));
        }
        let generation: i64 =
            sqlx::query_scalar("SELECT identity_generation FROM works WHERE user_id=?1 AND id=?2")
                .bind(user_id)
                .bind(anchor)
                .fetch_one(&mut *tx)
                .await
                .map_err(|error| format!("read healed generation for Work {anchor}: {error}"))?;
        sqlx::query(
            "INSERT INTO identity_audit_events \
                (user_id, work_id, event_kind, actor, payload, created_at) \
             VALUES (?1, ?2, 'settlement', 'identity-dedup-residue-heal', ?3, ?4)",
        )
        .bind(user_id)
        .bind(anchor)
        .bind(format!("generation={generation};folded_work_id={orphan}"))
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(|error| format!("audit dedup-residue fold {orphan}: {error}"))?;
        report.orphans_folded += 1;
        report.works_bumped += 1;
    }

    // Independently collapse any pre-fix equivalent pending proposals. This
    // also cleans the live duplicate pair if its orphan signature cannot be
    // proven safely enough to fold on a particular installation.
    let remaining: Vec<(i64, i64, Option<i64>, String)> = sqlx::query_as(
        "SELECT id, user_id, work_id, payload FROM identity_review_cards \
          WHERE kind=?1 AND status='pending' ORDER BY id",
    )
    .bind(ReviewKind::GroupIdentity.storage_code())
    .fetch_all(&mut *tx)
    .await
    .map_err(|error| format!("select remaining GroupIdentity residue: {error}"))?;
    let mut survivors: BTreeMap<(i64, String), i64> = BTreeMap::new();
    for (card_id, user_id, work_id, payload) in remaining {
        let card: SettlementReviewCard = serde_json::from_str(&payload)
            .map_err(|error| format!("decode remaining GroupIdentity card {card_id}: {error}"))?;
        let Some(key) = group_identity_proposal_key(&card)
            .map_err(|error| format!("key GroupIdentity card {card_id}: {error:?}"))?
        else {
            continue;
        };
        if let Some(oldest) = survivors.insert((user_id, key.clone()), card_id) {
            // ORDER BY id makes the prior mapping the survivor.
            survivors.insert((user_id, key), oldest);
            let audit_work_id = work_id.unwrap_or_default();
            if cancel_pending_group_card(
                &mut tx,
                user_id,
                card_id,
                audit_work_id,
                "equivalent-pending-card",
            )
            .await?
            {
                report.duplicate_cards_cancelled += 1;
            }
        }
    }

    sqlx::query(
        "INSERT INTO _livrarr_meta (key, value) \
         VALUES ('identity_dedup_residue_heal_generation', ?1) \
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
    )
    .bind(IDENTITY_DEDUP_RESIDUE_HEAL_GENERATION.to_string())
    .execute(&mut *tx)
    .await
    .map_err(|error| format!("stamp identity dedup-residue heal generation: {error}"))?;
    tx.commit()
        .await
        .map_err(|error| format!("commit identity dedup-residue heal: {error}"))?;
    Ok(report)
}

async fn dedup_orphan_has_only_engine_creation(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: UserId,
    work_id: WorkId,
) -> Result<bool, String> {
    let audits: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT event_kind, actor, payload FROM identity_audit_events \
          WHERE user_id=?1 AND work_id=?2 ORDER BY id",
    )
    .bind(user_id)
    .bind(work_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|error| format!("inspect orphan Work {work_id} audits: {error}"))?;
    Ok(audits.len() == 1
        && audits[0].0 == "settlement"
        && audits[0].1 == "identity-engine"
        && audits[0].2 == "generation=1")
}

async fn dedup_orphan_has_no_dependents(
    tx: &mut Transaction<'_, Sqlite>,
    work_id: WorkId,
) -> Result<bool, String> {
    orphan_has_no_dependents(tx, work_id, false).await
}

async fn orphan_has_no_dependents(
    tx: &mut Transaction<'_, Sqlite>,
    work_id: WorkId,
    allow_identity_routes: bool,
) -> Result<bool, String> {
    for (table, column) in [
        ("external_ids", "work_id"),
        ("library_items", "work_id"),
        ("grabs", "work_id"),
        ("history", "work_id"),
        ("bookmarks", "work_id"),
        ("editions", "work_id"),
        ("work_subjects", "work_id"),
        ("work_default_editions", "work_id"),
        ("work_cover_selections", "work_id"),
        ("machine_subtitle_projections", "work_id"),
        ("identity_conflicts_v2", "current_work_id"),
        ("identity_provider_attempts", "work_id"),
        ("work_identity_anchors", "work_id"),
        ("work_identity_conflicts", "existing_work_id"),
        ("work_field_dissents", "work_id"),
        ("provider_retry_state", "work_id"),
        ("work_metadata_provenance", "work_id"),
        ("work_identity_review_candidates", "work_id"),
        ("work_anchor_dead_ends", "work_id"),
        ("import_intents", "work_id"),
        ("author_link_key_attempts", "work_id"),
        ("author_provider_routes", "evidence_work_id"),
        ("work_contributor_roles", "work_id"),
    ] {
        let query = format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE {column}=?1)");
        let exists: bool = sqlx::query_scalar(&query)
            .bind(work_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|error| format!("inspect orphan Work {work_id} table {table}: {error}"))?;
        if exists {
            return Ok(false);
        }
    }
    for column in ["from_work_id", "target_work_id"] {
        let query = format!("SELECT EXISTS(SELECT 1 FROM work_relationships WHERE {column}=?1)");
        let exists: bool = sqlx::query_scalar(&query)
            .bind(work_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|error| format!("inspect orphan Work {work_id} relationships: {error}"))?;
        if exists {
            return Ok(false);
        }
    }
    if !allow_identity_routes {
        let routes: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM identity_routes WHERE resolved_work_id=?1 OR work_id=?1)",
        )
        .bind(work_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| format!("inspect orphan Work {work_id} routes: {error}"))?;
        if routes {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Conservative proof used by the leading-article startup heal. Provider
/// routes are deliberately allowed because the absorption transaction moves
/// them losslessly; all other user/dependent state remains disqualifying.
/// A contradictory active work key also forces review instead of auto-fold.
pub(crate) async fn article_duplicate_is_safe_to_fold(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: UserId,
    loser_work_id: WorkId,
) -> Result<bool, String> {
    if !dedup_orphan_has_only_engine_creation(tx, user_id, loser_work_id).await?
        || !orphan_has_no_dependents(tx, loser_work_id, true).await?
    {
        return Ok(false);
    }
    let work_kinds = [
        serde_json::to_string(&RouteKind::OpenLibraryWork)
            .map_err(|error| format!("encode OpenLibrary route kind: {error}"))?,
        serde_json::to_string(&RouteKind::GoodreadsWork)
            .map_err(|error| format!("encode Goodreads route kind: {error}"))?,
        serde_json::to_string(&RouteKind::HardcoverWork)
            .map_err(|error| format!("encode Hardcover route kind: {error}"))?,
    ];
    let contradiction: bool = sqlx::query_scalar(
        "SELECT EXISTS( \
            SELECT 1 FROM identity_routes loser \
            JOIN identity_routes other \
              ON other.user_id=loser.user_id \
             AND other.provider=loser.provider AND other.kind=loser.kind \
             AND other.provider_scoped_id<>loser.provider_scoped_id \
             AND other.resolved_work_id<>loser.resolved_work_id \
             AND other.state='active' \
           WHERE loser.user_id=?1 AND loser.resolved_work_id=?2 \
             AND loser.state='active' AND loser.kind IN (?3, ?4, ?5))",
    )
    .bind(user_id)
    .bind(loser_work_id)
    .bind(&work_kinds[0])
    .bind(&work_kinds[1])
    .bind(&work_kinds[2])
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| {
        format!("inspect article duplicate Work {loser_work_id} route contradictions: {error}")
    })?;
    Ok(!contradiction)
}

pub(crate) async fn absorb_article_duplicate(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: UserId,
    winner_work_id: WorkId,
    loser_work_id: WorkId,
) -> Result<(), String> {
    absorb_work_into(tx, user_id, winner_work_id, loser_work_id)
        .await
        .map(|_| ())
        .map_err(|error| {
            format!("fold article duplicate Work {loser_work_id} into {winner_work_id}: {error:?}")
        })
}

async fn cancel_pending_group_card(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: UserId,
    card_id: i64,
    work_id: WorkId,
    reason: &str,
) -> Result<bool, String> {
    let now = chrono::Utc::now().to_rfc3339();
    let updated = sqlx::query(
        "UPDATE identity_review_cards SET status='cancelled', resolved_at=?1 \
          WHERE id=?2 AND user_id=?3 AND kind=?4 AND status='pending'",
    )
    .bind(&now)
    .bind(card_id)
    .bind(user_id)
    .bind(ReviewKind::GroupIdentity.storage_code())
    .execute(&mut **tx)
    .await
    .map_err(|error| format!("cancel GroupIdentity card {card_id}: {error}"))?;
    if updated.rows_affected() == 0 {
        return Ok(false);
    }
    sqlx::query(
        "INSERT INTO identity_audit_events \
            (user_id, work_id, event_kind, actor, payload, created_at) \
         VALUES (?1, ?2, 'review-dismissal', 'identity-dedup-residue-heal', ?3, ?4)",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(format!("card_id={card_id};reason={reason}"))
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(|error| format!("audit GroupIdentity cancellation {card_id}: {error}"))?;
    Ok(true)
}

fn encode_identity_status(status: IdentityStatus) -> &'static str {
    match status {
        IdentityStatus::UserConfirmed => "user_confirmed",
        IdentityStatus::Connected => "connected",
        IdentityStatus::NotConnected => "not_connected",
    }
}

fn decode_identity_status(value: &str) -> Result<IdentityStatus, IdentityRepositoryError> {
    match value {
        "user_confirmed" => Ok(IdentityStatus::UserConfirmed),
        "connected" => Ok(IdentityStatus::Connected),
        "not_connected" => Ok(IdentityStatus::NotConnected),
        other => Err(IdentityRepositoryError::Database(format!(
            "invalid identity status {other}"
        ))),
    }
}

fn encode_inspection_outcome(outcome: EmbeddedCoverInspectionOutcome) -> &'static str {
    match outcome {
        EmbeddedCoverInspectionOutcome::Extracted => "extracted",
        EmbeddedCoverInspectionOutcome::VerifiedNoCover => "verified_no_cover",
        EmbeddedCoverInspectionOutcome::CouldNotInspect => "could_not_inspect",
        EmbeddedCoverInspectionOutcome::FileGone => "file_gone",
    }
}

fn decode_inspection_outcome(
    value: &str,
) -> Result<EmbeddedCoverInspectionOutcome, IdentityRepositoryError> {
    match value {
        "extracted" => Ok(EmbeddedCoverInspectionOutcome::Extracted),
        "verified_no_cover" => Ok(EmbeddedCoverInspectionOutcome::VerifiedNoCover),
        "could_not_inspect" => Ok(EmbeddedCoverInspectionOutcome::CouldNotInspect),
        "file_gone" => Ok(EmbeddedCoverInspectionOutcome::FileGone),
        other => Err(IdentityRepositoryError::Database(format!(
            "invalid inspection outcome {other}"
        ))),
    }
}

fn empty_cover_presentation() -> WorkCoverPresentation {
    WorkCoverPresentation {
        format_needed: None,
        ebook: CoverSlotPresentation {
            selected: None,
            placeholder: Some(CoverPlaceholderState::NowhereToLook),
        },
        audiobook: CoverSlotPresentation {
            selected: None,
            placeholder: Some(CoverPlaceholderState::NowhereToLook),
        },
    }
}

#[derive(Debug)]
struct LegacyGroupReviewCohort {
    user_id: UserId,
    work_ids: Vec<WorkId>,
    generation: i64,
}

type LegacyGroupKey = (UserId, String, String, String, AuthorId, String);
type LegacyGroupMember = (WorkId, i64);

async fn legacy_group_review_cohorts(
    db: &SqliteDb,
) -> Result<Vec<LegacyGroupReviewCohort>, IdentityMigrationError> {
    let rows = sqlx::query(
        "SELECT id, user_id, identity_generation, \
                CASE WHEN trim(normalized_identity_main) = '' \
                           OR normalized_identity_main = '__UNMIGRATED__' \
                     THEN normalized_title ELSE normalized_identity_main END AS identity_main, \
                COALESCE(NULLIF(trim(normalized_identity_subtitle), ''), \
                         lower(trim(COALESCE(subtitle, '')))) AS identity_subtitle, \
                COALESCE(NULLIF(trim(normalized_identity_volume), ''), \
                         NULLIF(trim(identity_volume), ''), \
                         CASE WHEN series_position IS NULL THEN '' \
                              ELSE CAST(series_position AS TEXT) END) AS identity_volume, \
                COALESCE(primary_author_id, author_id) AS primary_author_id, \
                COALESCE(NULLIF(trim(text_distinction), ''), 'common') \
                    AS text_distinction \
           FROM works ORDER BY user_id, id",
    )
    .fetch_all(db.pool())
    .await
    .map_err(migration_db)?;
    let mut groups: BTreeMap<LegacyGroupKey, Vec<LegacyGroupMember>> = BTreeMap::new();
    for row in rows {
        let identity_main: String = row.try_get("identity_main").map_err(migration_db)?;
        let primary_author_id: Option<AuthorId> =
            row.try_get("primary_author_id").map_err(migration_db)?;
        let Some(primary_author_id) = primary_author_id else {
            continue;
        };
        let user_id = row.try_get("user_id").map_err(migration_db)?;
        groups
            .entry((
                user_id,
                identity_main,
                row.try_get("identity_subtitle").map_err(migration_db)?,
                row.try_get("identity_volume").map_err(migration_db)?,
                primary_author_id,
                row.try_get("text_distinction").map_err(migration_db)?,
            ))
            .or_default()
            .push((
                row.try_get("id").map_err(migration_db)?,
                row.try_get("identity_generation").map_err(migration_db)?,
            ));
    }
    Ok(groups
        .into_iter()
        .filter_map(|((user_id, _, _, _, _, _), members)| {
            (members.len() > 1).then(|| LegacyGroupReviewCohort {
                user_id,
                generation: members[0].1,
                work_ids: members.into_iter().map(|(work_id, _)| work_id).collect(),
            })
        })
        .collect())
}

async fn build_migration_report(
    db: &SqliteDb,
) -> Result<IdentityMigrationReport, IdentityMigrationError> {
    let schema_version = sqlx::query_scalar::<_, String>(
        "SELECT value FROM _livrarr_meta WHERE key = 'schema_version'",
    )
    .fetch_optional(db.pool())
    .await
    .map_err(migration_db)?
    .ok_or(IdentityMigrationError::SchemaMismatch)?
    .parse::<u32>()
    .map_err(|_| IdentityMigrationError::SchemaMismatch)?;
    if schema_version != 83 {
        return Err(IdentityMigrationError::SchemaMismatch);
    }
    let rows = sqlx::query(
        "SELECT id, user_id, title, COALESCE(author_name, '') AS author_name, \
                COALESCE(ol_key, '') AS ol_key, COALESCE(hc_key, '') AS hc_key, \
                COALESCE(gr_key, '') AS gr_key, COALESCE(isbn_13, '') AS isbn_13, \
                COALESCE(asin, '') AS asin \
           FROM works ORDER BY user_id, id",
    )
    .fetch_all(db.pool())
    .await
    .map_err(migration_db)?;
    let legacy_work_count = rows.len() as u64;
    let mut source_hash = Sha256::new();
    let mut mapped_route_count = 0_u64;
    for row in &rows {
        let values = (
            row.try_get::<i64, _>("id").map_err(migration_db)?,
            row.try_get::<i64, _>("user_id").map_err(migration_db)?,
            row.try_get::<String, _>("title").map_err(migration_db)?,
            row.try_get::<String, _>("author_name")
                .map_err(migration_db)?,
            row.try_get::<String, _>("ol_key").map_err(migration_db)?,
            row.try_get::<String, _>("hc_key").map_err(migration_db)?,
            row.try_get::<String, _>("gr_key").map_err(migration_db)?,
            row.try_get::<String, _>("isbn_13").map_err(migration_db)?,
            row.try_get::<String, _>("asin").map_err(migration_db)?,
        );
        for route in [&values.4, &values.5, &values.6, &values.7, &values.8] {
            mapped_route_count += u64::from(!route.trim().is_empty());
        }
        source_hash.update(
            serde_json::to_vec(&values)
                .map_err(|error| IdentityMigrationError::Database(error.to_string()))?,
        );
    }
    let external_route_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM external_ids")
        .fetch_one(db.pool())
        .await
        .map_err(migration_db)?;
    mapped_route_count += external_route_count as u64;
    source_hash.update(external_route_count.to_le_bytes());
    let source_fingerprint: [u8; 32] = source_hash.finalize().into();
    let edition_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM editions")
        .fetch_one(db.pool())
        .await
        .map_err(migration_db)?;
    let group_cards = legacy_group_review_cohorts(db).await?.len() as u64;
    let repair_cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards \
              WHERE kind = ?1 AND status = 'pending'",
    )
    .bind(ReviewKind::MigrationRepair.storage_code())
    .fetch_one(db.pool())
    .await
    .map_err(migration_db)?;
    let field_cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards \
              WHERE kind = ?1 AND status = 'pending'",
    )
    .bind(ReviewKind::FieldResolution.storage_code())
    .fetch_one(db.pool())
    .await
    .map_err(migration_db)?;
    let contributor_cards: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_review_cards \
              WHERE kind = ?1 AND status = 'pending'",
    )
    .bind(ReviewKind::ContributorOrder.storage_code())
    .fetch_one(db.pool())
    .await
    .map_err(migration_db)?;
    let mut output_hash = Sha256::new();
    output_hash.update(source_fingerprint);
    output_hash.update(mapped_route_count.to_le_bytes());
    output_hash.update((edition_count as u64).to_le_bytes());
    output_hash.update(group_cards.to_le_bytes());
    let canonical_output_fingerprint = output_hash.finalize().into();
    Ok(IdentityMigrationReport {
        source_schema_version: schema_version,
        source_fingerprint,
        canonical_output_fingerprint,
        mapped_route_count,
        edition_count: edition_count as u64,
        repair_cards: repair_cards as u64,
        group_cards,
        field_cards: field_cards as u64,
        contributor_cards: contributor_cards as u64,
        index_ready: group_cards == 0,
        trivially_empty: legacy_work_count == 0 && mapped_route_count == 0,
        legacy_work_count,
    })
}

async fn persist_cutover_report(
    db: &SqliteDb,
    mode: IdentityCutoverMode,
    report: &IdentityMigrationReport,
) -> Result<(), IdentityMigrationError> {
    let mut tx = crate::pool::begin_write(db.pool())
        .await
        .map_err(migration_db)?;
    let mode_text = match mode {
        IdentityCutoverMode::Rehearsal => "rehearsal",
        IdentityCutoverMode::Apply => "apply",
    };
    let branch = if report.trivially_empty {
        "trivially_empty"
    } else {
        "snapshot"
    };
    let blocker_count =
        report.repair_cards + report.group_cards + report.field_cards + report.contributor_cards;
    let status = if report.index_ready && blocker_count == 0 {
        "ready"
    } else {
        "blocked"
    };
    let now = chrono::Utc::now().to_rfc3339();
    let run = sqlx::query(
        "INSERT INTO identity_cutover_runs \
            (mode, branch, source_schema_version, source_fingerprint, \
             canonical_output_fingerprint, status, report_json, created_at, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
    )
    .bind(mode_text)
    .bind(branch)
    .bind(i64::from(report.source_schema_version))
    .bind(report.source_fingerprint.to_vec())
    .bind(report.canonical_output_fingerprint.to_vec())
    .bind(status)
    .bind(
        serde_json::to_string(report)
            .map_err(|error| IdentityMigrationError::Database(error.to_string()))?,
    )
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(migration_db)?;
    sqlx::query(
        "INSERT INTO identity_cutover_reports \
            (run_id, source_schema_version, source_fingerprint, canonical_output_fingerprint, \
             mapped_route_count, edition_count, blocker_count, index_ready, trivially_empty) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
    )
    .bind(run.last_insert_rowid())
    .bind(i64::from(report.source_schema_version))
    .bind(report.source_fingerprint.to_vec())
    .bind(report.canonical_output_fingerprint.to_vec())
    .bind(report.mapped_route_count as i64)
    .bind(report.edition_count as i64)
    .bind(blocker_count as i64)
    .bind(i64::from(report.index_ready))
    .bind(i64::from(report.trivially_empty))
    .execute(&mut *tx)
    .await
    .map_err(migration_db)?;
    tx.commit().await.map_err(migration_db)
}

async fn stage_legacy_identity_rows(
    db: &SqliteDb,
    report: &IdentityMigrationReport,
) -> Result<(), IdentityMigrationError> {
    let group_review_cohorts = legacy_group_review_cohorts(db).await?;
    if group_review_cohorts.len() as u64 != report.group_cards {
        return Err(IdentityMigrationError::Database(
            "legacy group-card staging no longer matches the approved report".to_string(),
        ));
    }
    let mut tx = crate::pool::begin_write(db.pool())
        .await
        .map_err(migration_db)?;
    sqlx::query(
        "UPDATE works SET \
            normalized_identity_main = \
                CASE WHEN trim(normalized_identity_main) = '' \
                           OR normalized_identity_main = '__UNMIGRATED__' \
                     THEN normalized_title ELSE normalized_identity_main END, \
            normalized_identity_subtitle = \
                COALESCE(NULLIF(trim(normalized_identity_subtitle), ''), \
                         lower(trim(COALESCE(subtitle, '')))), \
            normalized_identity_volume = \
                COALESCE(NULLIF(trim(normalized_identity_volume), ''), \
                         NULLIF(trim(identity_volume), ''), \
                         CASE WHEN series_position IS NULL THEN '' \
                              ELSE CAST(series_position AS TEXT) END), \
            primary_author_id = COALESCE(primary_author_id, author_id), \
            text_distinction = \
                COALESCE(NULLIF(trim(text_distinction), ''), 'common')",
    )
    .execute(&mut *tx)
    .await
    .map_err(migration_db)?;
    let works = sqlx::query(
        "SELECT id, user_id, identity_generation, ol_key, hc_key, gr_key, isbn_13, asin \
           FROM works ORDER BY user_id, id",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(migration_db)?;
    for work in works {
        let work_id: i64 = work.try_get("id").map_err(migration_db)?;
        let user_id: i64 = work.try_get("user_id").map_err(migration_db)?;
        let generation: i64 = work.try_get("identity_generation").map_err(migration_db)?;
        for (column, provider, kind) in [
            (
                "ol_key",
                livrarr_domain::identity_layer::IdentityProvider::OpenLibrary,
                livrarr_domain::identity_layer::RouteKind::OpenLibraryWork,
            ),
            (
                "hc_key",
                livrarr_domain::identity_layer::IdentityProvider::Hardcover,
                livrarr_domain::identity_layer::RouteKind::HardcoverWork,
            ),
        ] {
            if let Some(value) = nonempty_column(&work, column)? {
                stage_route(
                    &mut tx,
                    LegacyRouteStage {
                        user_id,
                        work_id,
                        owner: RouteOwner::Work(work_id),
                        provider,
                        kind,
                        value,
                        legacy_field: column,
                        generation,
                    },
                )
                .await?;
            }
        }
        for (column, provider, kind) in [
            (
                "gr_key",
                livrarr_domain::identity_layer::IdentityProvider::Goodreads,
                livrarr_domain::identity_layer::RouteKind::GoodreadsBookEdition,
            ),
            (
                "isbn_13",
                livrarr_domain::identity_layer::IdentityProvider::IsbnRegistry,
                livrarr_domain::identity_layer::RouteKind::Isbn13Edition,
            ),
            (
                "asin",
                livrarr_domain::identity_layer::IdentityProvider::Amazon,
                livrarr_domain::identity_layer::RouteKind::AsinEdition,
            ),
        ] {
            if let Some(value) = nonempty_column(&work, column)? {
                let edition_id =
                    staged_edition(&mut tx, user_id, work_id, &provider, &kind, &value).await?;
                stage_route(
                    &mut tx,
                    LegacyRouteStage {
                        user_id,
                        work_id,
                        owner: RouteOwner::Edition(edition_id),
                        provider,
                        kind,
                        value,
                        legacy_field: column,
                        generation,
                    },
                )
                .await?;
            }
        }
    }
    for cohort in group_review_cohorts {
        let work_id = cohort.work_ids[0];
        let payload = serde_json::to_string(&SettlementReviewCard::GroupIdentity {
            work_ids: cohort.work_ids,
            proposed_identity: None,
            merge_choices: Vec::new(),
        })
        .map_err(|error| IdentityMigrationError::Database(error.to_string()))?;
        sqlx::query(
            "INSERT INTO identity_review_cards \
                (user_id, work_id, kind, generation, status, payload, created_at) \
             SELECT ?1, ?2, ?3, ?4, 'pending', ?5, ?6 \
              WHERE NOT EXISTS (SELECT 1 FROM identity_review_cards \
                                 WHERE user_id = ?1 AND work_id = ?2 \
                                   AND kind = ?3 AND payload = ?5)",
        )
        .bind(cohort.user_id)
        .bind(work_id)
        .bind(ReviewKind::GroupIdentity.storage_code())
        .bind(cohort.generation)
        .bind(payload)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(migration_db)?;
    }
    let staged_counts: (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
            COALESCE(SUM(CASE WHEN kind = ?1 THEN 1 ELSE 0 END), 0), \
            COALESCE(SUM(CASE WHEN kind = ?2 THEN 1 ELSE 0 END), 0), \
            COALESCE(SUM(CASE WHEN kind = ?3 THEN 1 ELSE 0 END), 0), \
            COALESCE(SUM(CASE WHEN kind = ?4 THEN 1 ELSE 0 END), 0) \
           FROM identity_review_cards WHERE status = 'pending'",
    )
    .bind(ReviewKind::GroupIdentity.storage_code())
    .bind(ReviewKind::FieldResolution.storage_code())
    .bind(ReviewKind::MigrationRepair.storage_code())
    .bind(ReviewKind::ContributorOrder.storage_code())
    .fetch_one(&mut *tx)
    .await
    .map_err(migration_db)?;
    let approved_counts = (
        report.group_cards as i64,
        report.field_cards as i64,
        report.repair_cards as i64,
        report.contributor_cards as i64,
    );
    if staged_counts != approved_counts {
        return Err(IdentityMigrationError::Database(format!(
            "staged review-card counts {staged_counts:?} do not match approved report {approved_counts:?}"
        )));
    }
    tx.commit().await.map_err(migration_db)
}

fn nonempty_column(
    row: &sqlx::sqlite::SqliteRow,
    column: &str,
) -> Result<Option<String>, IdentityMigrationError> {
    Ok(row
        .try_get::<Option<String>, _>(column)
        .map_err(migration_db)?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty()))
}

async fn staged_edition(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: UserId,
    work_id: WorkId,
    provider: &livrarr_domain::identity_layer::IdentityProvider,
    kind: &livrarr_domain::identity_layer::RouteKind,
    value: &str,
) -> Result<EditionId, IdentityMigrationError> {
    let provider_json = serde_json::to_string(provider)
        .map_err(|error| IdentityMigrationError::Database(error.to_string()))?;
    let kind_json = serde_json::to_string(kind)
        .map_err(|error| IdentityMigrationError::Database(error.to_string()))?;
    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT edition_id FROM identity_routes \
          WHERE user_id = ?1 AND provider = ?2 AND kind = ?3 \
            AND provider_scoped_id = ?4 AND state = 'active'",
    )
    .bind(user_id)
    .bind(&provider_json)
    .bind(&kind_json)
    .bind(value)
    .fetch_optional(&mut **tx)
    .await
    .map_err(migration_db)?;
    if let Some(edition_id) = existing {
        return Ok(edition_id);
    }
    let inserted = sqlx::query(
        "INSERT INTO editions \
            (user_id, work_id, format, source_provider, provider_edition_id, state) \
         VALUES (?1, ?2, ?3, ?4, ?5, 'active')",
    )
    .bind(user_id)
    .bind(work_id)
    .bind(
        serde_json::to_string(&EditionFormat::Unknown)
            .map_err(|error| IdentityMigrationError::Database(error.to_string()))?,
    )
    .bind(provider_json)
    .bind(value)
    .execute(&mut **tx)
    .await
    .map_err(migration_db)?;
    Ok(inserted.last_insert_rowid())
}

struct LegacyRouteStage<'a> {
    user_id: UserId,
    work_id: WorkId,
    owner: RouteOwner,
    provider: livrarr_domain::identity_layer::IdentityProvider,
    kind: livrarr_domain::identity_layer::RouteKind,
    value: String,
    legacy_field: &'a str,
    generation: i64,
}

async fn stage_route(
    tx: &mut Transaction<'_, Sqlite>,
    staged: LegacyRouteStage<'_>,
) -> Result<(), IdentityMigrationError> {
    let provider_json = serde_json::to_string(&staged.provider)
        .map_err(|error| IdentityMigrationError::Database(error.to_string()))?;
    let kind_json = serde_json::to_string(&staged.kind)
        .map_err(|error| IdentityMigrationError::Database(error.to_string()))?;
    let existing = sqlx::query(
        "SELECT resolved_work_id FROM identity_routes \
          WHERE user_id = ?1 AND provider = ?2 AND kind = ?3 \
            AND provider_scoped_id = ?4 AND state = 'active'",
    )
    .bind(staged.user_id)
    .bind(&provider_json)
    .bind(&kind_json)
    .bind(&staged.value)
    .fetch_optional(&mut **tx)
    .await
    .map_err(migration_db)?;
    if let Some(existing) = existing {
        let current_work: i64 = existing.try_get("resolved_work_id").map_err(migration_db)?;
        if current_work != staged.work_id {
            let owner_id = match staged.owner {
                RouteOwner::Work(owner_id) | RouteOwner::Edition(owner_id) => owner_id,
            };
            sqlx::query(
                "INSERT INTO identity_conflicts_v2 \
                    (user_id, current_work_id, class, candidate_provider, candidate_kind, \
                     candidate_value, proposed_owner_type, proposed_owner_id, status, \
                     expected_generation) \
                 SELECT ?1, ?2, 'class_c', ?3, ?4, ?5, ?6, ?7, 'pending', ?8 \
                  WHERE NOT EXISTS (SELECT 1 FROM identity_conflicts_v2 \
                                     WHERE user_id = ?1 AND candidate_provider = ?3 \
                                       AND candidate_kind = ?4 AND candidate_value = ?5 \
                                       AND status = 'pending')",
            )
            .bind(staged.user_id)
            .bind(current_work)
            .bind(provider_json)
            .bind(kind_json)
            .bind(staged.value)
            .bind(match staged.owner {
                RouteOwner::Work(_) => "work",
                RouteOwner::Edition(_) => "edition",
            })
            .bind(owner_id)
            .bind(staged.generation)
            .execute(&mut **tx)
            .await
            .map_err(migration_db)?;
        }
        return Ok(());
    }
    let (owner_type, owner_work_id, edition_id) = match staged.owner {
        RouteOwner::Work(owner_id) => ("work", Some(owner_id), None),
        RouteOwner::Edition(owner_id) => ("edition", None, Some(owner_id)),
    };
    sqlx::query(
        "INSERT INTO identity_routes \
            (user_id, owner_type, work_id, edition_id, resolved_work_id, provider, kind, \
             provider_scoped_id, state, provenance, user_confirmed, observed_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active', ?9, 0, ?10)",
    )
    .bind(staged.user_id)
    .bind(owner_type)
    .bind(owner_work_id)
    .bind(edition_id)
    .bind(staged.work_id)
    .bind(provider_json)
    .bind(kind_json)
    .bind(staged.value)
    .bind(
        serde_json::to_string(&RouteProvenance::Migrated {
            legacy_field: staged.legacy_field.to_string(),
        })
        .map_err(|error| IdentityMigrationError::Database(error.to_string()))?,
    )
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&mut **tx)
    .await
    .map_err(migration_db)?;
    Ok(())
}

async fn reuse_staged_rows(
    db: &SqliteDb,
    approved: &IdentityMigrationReport,
) -> Result<(), IdentityMigrationError> {
    let matching: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM identity_cutover_reports \
          WHERE source_schema_version = ?1 AND source_fingerprint = ?2",
    )
    .bind(i64::from(approved.source_schema_version))
    .bind(approved.source_fingerprint.to_vec())
    .fetch_one(db.pool())
    .await
    .map_err(migration_db)?;
    if matching == 0 {
        return Err(IdentityMigrationError::RehearsalMismatch);
    }
    Ok(())
}

#[cfg(any(test, feature = "test-helpers"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityDbFailpoint {
    None = 0,
    CommitAfterWork = 1,
    CommitAfterContributors = 2,
    CommitAfterRoutes = 3,
    CommitBeforeCommit = 4,
    TransferBeforeOwnerUpdate = 5,
    TransferBeforeCommit = 6,
    ReadinessIndex = 7,
    ActivationIndex = 8,
}

#[cfg(any(test, feature = "test-helpers"))]
thread_local! {
    // Behavioral tests each run on a current-thread Tokio runtime. Keeping the
    // armed fault local to that runtime thread prevents an unrelated parallel
    // database test from consuming another test's one-shot fault.
    static IDENTITY_DB_FAILPOINT: std::cell::Cell<u8> =
        const { std::cell::Cell::new(IdentityDbFailpoint::None as u8) };
}

#[cfg(any(test, feature = "test-helpers"))]
pub fn set_identity_db_failpoint_for_tests(failpoint: IdentityDbFailpoint) {
    IDENTITY_DB_FAILPOINT.with(|armed| armed.set(failpoint as u8));
}

#[cfg(any(test, feature = "test-helpers"))]
fn consume_failpoint(expected: IdentityDbFailpoint) -> bool {
    IDENTITY_DB_FAILPOINT.with(|armed| {
        if armed.get() == expected as u8 {
            armed.set(IdentityDbFailpoint::None as u8);
            true
        } else {
            false
        }
    })
}

fn commit_settlement_failpoint(stage: &str) -> Result<(), IdentityRepositoryError> {
    #[cfg(any(test, feature = "test-helpers"))]
    let hit = match stage {
        "work" => consume_failpoint(IdentityDbFailpoint::CommitAfterWork),
        "contributors" => consume_failpoint(IdentityDbFailpoint::CommitAfterContributors),
        "routes" => consume_failpoint(IdentityDbFailpoint::CommitAfterRoutes),
        "reviews" => consume_failpoint(IdentityDbFailpoint::CommitBeforeCommit),
        _ => false,
    };
    #[cfg(not(any(test, feature = "test-helpers")))]
    let hit = {
        let _ = stage;
        false
    };
    if hit {
        Err(IdentityRepositoryError::AtomicRollback)
    } else {
        Ok(())
    }
}

fn transfer_route_failpoint(stage: &str) -> Result<(), IdentityRepositoryError> {
    #[cfg(any(test, feature = "test-helpers"))]
    let hit = match stage {
        "before-owner-update" => consume_failpoint(IdentityDbFailpoint::TransferBeforeOwnerUpdate),
        "before-commit" => consume_failpoint(IdentityDbFailpoint::TransferBeforeCommit),
        _ => false,
    };
    #[cfg(not(any(test, feature = "test-helpers")))]
    let hit = {
        let _ = stage;
        false
    };
    if hit {
        Err(IdentityRepositoryError::AtomicRollback)
    } else {
        Ok(())
    }
}

fn readiness_index_failpoint() -> Result<(), IdentityMigrationError> {
    #[cfg(any(test, feature = "test-helpers"))]
    if consume_failpoint(IdentityDbFailpoint::ReadinessIndex) {
        return Err(IdentityMigrationError::Database(
            "readiness index failpoint".to_string(),
        ));
    }
    Ok(())
}

fn activation_index_failpoint() -> Result<(), IdentityMigrationError> {
    #[cfg(any(test, feature = "test-helpers"))]
    if consume_failpoint(IdentityDbFailpoint::ActivationIndex) {
        return Err(IdentityMigrationError::Database(
            "activation index failpoint".to_string(),
        ));
    }
    Ok(())
}

#[cfg(any(test, feature = "test-helpers"))]
impl SqliteDb {
    pub async fn seed_transfer_target_for_tests(
        &self,
        user_id: UserId,
        work_id: WorkId,
        format: EditionFormat,
    ) -> Result<EditionId, IdentityRepositoryError> {
        let row = sqlx::query(
            "INSERT INTO editions (user_id, work_id, format, state) \
             VALUES (?1, ?2, ?3, 'active')",
        )
        .bind(user_id)
        .bind(work_id)
        .bind(serde_json::to_string(&format).map_err(repo_json)?)
        .execute(self.pool())
        .await
        .map_err(repo_db)?;
        Ok(row.last_insert_rowid())
    }
}
