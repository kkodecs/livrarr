//! Series reconcile: stub creation, work↔series FK linking, and unlink GC.
//!
//! Stubs are series rows created from a work's `series_name` metadata rather
//! than from Goodreads. A stub carries `gr_key = "stub:<normalized name>"` —
//! the existing `UNIQUE(user_id, author_id, gr_key)` key then makes stubs
//! per-name unique with no schema change — and `work_count = i32::MAX`, so a
//! stub never wins the "fewest books = most specific" assignment guard
//! against a GR-backed series (`SeriesDb::link_work_to_series`).
//!
//! Link arbitration:
//! - a user edit of `series_name` always relinks;
//! - a system write (enrichment/merge/back-fill) never moves or removes the
//!   FK of a work linked to a GR-backed (non-stub) series — string-only;
//! - an unmonitored stub left with zero linked works is deleted; monitored
//!   series and GR-backed rows are never auto-deleted here.

use livrarr_db::{CreateSeriesDbRequest, DbError, SeriesDb, UserId, WorkDb};
use livrarr_domain::{normalize_for_matching, split_series_suffix, Work};

// Single authority for stub-key semantics lives in livrarr-domain (handlers
// mask these at the API boundary).
pub use livrarr_domain::{
    is_series_stub_key as is_stub_key, SERIES_STUB_KEY_PREFIX as STUB_KEY_PREFIX,
    SERIES_STUB_WORK_COUNT as STUB_WORK_COUNT,
};

pub fn stub_key_for(clean_name: &str) -> String {
    format!("{STUB_KEY_PREFIX}{}", normalize_for_matching(clean_name))
}

/// Who initiated the series_name value being reconciled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesLinkOrigin {
    /// Explicit user edit — always wins.
    User,
    /// Enrichment, merge, import metadata, or the startup back-fill.
    System,
}

/// Reconcile one work's series link against its current `series_name`
/// (REQ-001). Ensures a series row exists for (user, author, normalized
/// name) — creating an unmonitored stub when absent — links the work, and
/// GCs a previously-linked stub the work just left. Returns the linked
/// series id, or `None` when reconciliation was skipped or was an unlink.
///
/// Skips (never an error): empty/blank `series_name` handled as unlink;
/// NULL `author_id` (author deleted) — the name stays display-only.
pub async fn reconcile_work_series<D>(
    db: &D,
    work: &Work,
    origin: SeriesLinkOrigin,
) -> Result<Option<i64>, DbError>
where
    D: SeriesDb + WorkDb + Send + Sync,
{
    let user_id = work.user_id;
    let raw_name = work.series_name.as_deref().unwrap_or("").trim().to_string();

    // Cleared name → unlink (User always; System only off stub links).
    if raw_name.is_empty() {
        if let Some(current_id) = work.series_id {
            let current = db.get_series(user_id, current_id).await?;
            let gr_backed = current.as_ref().is_some_and(|s| !is_stub_key(&s.gr_key));
            if gr_backed && origin == SeriesLinkOrigin::System {
                return Ok(Some(current_id));
            }
            db.set_work_series_id(user_id, work.id, None).await?;
            gc_stub_if_empty(db, user_id, current_id).await?;
        }
        return Ok(None);
    }

    // NULL author: stub rows require an author (series.author_id NOT NULL).
    // Display-only string until the recurring back-fill heals it.
    let Some(author_id) = work.author_id else {
        return Ok(None);
    };

    // Q-002 normalization: strip positional suffix, rewrite the work
    // coherently (position fills only when the work has none).
    let (clean_name, extracted_pos) = split_series_suffix(&raw_name);
    if clean_name.is_empty() {
        return Ok(None);
    }
    if clean_name != raw_name {
        db.normalize_work_series_fields(user_id, work.id, &clean_name, extracted_pos)
            .await?;
    }
    let normalized = normalize_for_matching(&clean_name);
    if normalized.is_empty() {
        return Ok(None);
    }

    // Find the target row by normalized name among the author's series.
    // Prefer a GR-backed row over a stub when both carry the same name.
    let author_series = db.list_series_for_author(user_id, author_id).await?;
    let mut target = None;
    for s in &author_series {
        if normalize_for_matching(&s.name) == normalized {
            if !is_stub_key(&s.gr_key) {
                target = Some(s.clone());
                break;
            }
            if target.is_none() {
                target = Some(s.clone());
            }
        }
    }

    // Arbitration against the work's current link.
    if let Some(current_id) = work.series_id {
        if target.as_ref().is_some_and(|t| t.id == current_id) {
            return Ok(Some(current_id)); // already linked right
        }
        let current = db.get_series(user_id, current_id).await?;
        let gr_backed = current.as_ref().is_some_and(|s| !is_stub_key(&s.gr_key));
        if gr_backed && origin == SeriesLinkOrigin::System {
            // Never displace a GR-grounded assignment on a system write.
            return Ok(Some(current_id));
        }
        let target_row = match target {
            Some(t) => t,
            None => create_stub(db, user_id, author_id, &clean_name).await?,
        };
        db.set_work_series_id(user_id, work.id, Some(target_row.id))
            .await?;
        gc_stub_if_empty(db, user_id, current_id).await?;
        return Ok(Some(target_row.id));
    }

    let target_row = match target {
        Some(t) => t,
        None => create_stub(db, user_id, author_id, &clean_name).await?,
    };
    db.set_work_series_id(user_id, work.id, Some(target_row.id))
        .await?;
    Ok(Some(target_row.id))
}

/// GC hook for paths that remove a work outright (REQ-001 / AC-012): after a
/// work deletion, delete the series row it pointed at if that row is an
/// unmonitored stub with no remaining linked works.
pub async fn gc_stub_if_empty<D>(db: &D, user_id: UserId, series_id: i64) -> Result<(), DbError>
where
    D: SeriesDb + Send + Sync,
{
    let Some(series) = db.get_series(user_id, series_id).await? else {
        return Ok(());
    };
    if !is_stub_key(&series.gr_key) || series.monitor_ebook || series.monitor_audiobook {
        return Ok(());
    }
    if db.count_works_in_series(user_id, series_id).await? == 0 {
        db.delete_series(user_id, series_id).await?;
    }
    Ok(())
}

async fn create_stub<D>(
    db: &D,
    user_id: UserId,
    author_id: i64,
    clean_name: &str,
) -> Result<livrarr_db::Series, DbError>
where
    D: SeriesDb + Send + Sync,
{
    db.upsert_series(CreateSeriesDbRequest {
        user_id,
        author_id,
        name: clean_name.to_string(),
        gr_key: stub_key_for(clean_name),
        monitor_ebook: false,
        monitor_audiobook: false,
        monitor_language: None,
        work_count: STUB_WORK_COUNT,
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use livrarr_db::test_helpers::create_test_db;
    use livrarr_db::{
        sqlite::SqliteDb, AuthorDb, CreateAuthorDbRequest, CreateUserDbRequest,
        CreateWorkDbRequest, UserDb, UserRole, WorkDbCreate,
    };

    async fn seed_user_author(db: &SqliteDb) -> (i64, i64) {
        let user = db
            .create_user(CreateUserDbRequest {
                username: "u".to_string(),
                password_hash: "h".to_string(),
                role: UserRole::User,
                api_key_hash: "k".to_string(),
            })
            .await
            .unwrap();
        let author = db
            .create_author(CreateAuthorDbRequest {
                user_id: user.id,
                name: "Jim Butcher".to_string(),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                hc_key: None,
                import_id: None,
            })
            .await
            .unwrap();
        (user.id, author.id)
    }

    async fn seed_work(
        db: &SqliteDb,
        user_id: i64,
        author_id: Option<i64>,
        title: &str,
        series_name: Option<&str>,
        series_id: Option<i64>,
        series_position: Option<f64>,
    ) -> Work {
        let (work, _) = db
            .create_work(CreateWorkDbRequest {
                user_id,
                title: title.to_string(),
                author_name: "Jim Butcher".to_string(),
                normalized_title: normalize_for_matching(title),
                normalized_author: "jim butcher".to_string(),
                author_id,
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                language: None,
                import_id: None,
                series_id,
                series_name: series_name.map(str::to_string),
                series_position,
                monitor_ebook: true,
                monitor_audiobook: false,
                source_provider_json: None,
                isbn_13: None,
                asin: None,
                description: None,
                cover_manual: false,
            })
            .await
            .unwrap();
        work
    }

    #[tokio::test]
    async fn creates_unmonitored_stub_and_links_work() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed_user_author(&db).await;
        let work = seed_work(
            &db,
            user_id,
            Some(author_id),
            "Storm Front",
            Some("The Dresden Files"),
            None,
            None,
        )
        .await;

        let linked = reconcile_work_series(&db, &work, SeriesLinkOrigin::System)
            .await
            .unwrap();
        let series_id = linked.expect("work should link");

        let series = db.get_series(user_id, series_id).await.unwrap().unwrap();
        assert_eq!(series.name, "The Dresden Files");
        assert!(is_stub_key(&series.gr_key));
        assert!(!series.monitor_ebook && !series.monitor_audiobook);
        assert_eq!(series.work_count, STUB_WORK_COUNT);
        let reloaded = db.get_work(user_id, work.id).await.unwrap();
        assert_eq!(reloaded.series_id, Some(series_id));
    }

    #[tokio::test]
    async fn second_work_links_same_stub_no_duplicate() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed_user_author(&db).await;
        let w1 = seed_work(
            &db,
            user_id,
            Some(author_id),
            "Storm Front",
            Some("The Dresden Files"),
            None,
            None,
        )
        .await;
        let w2 = seed_work(
            &db,
            user_id,
            Some(author_id),
            "Fool Moon",
            Some("The Dresden Files"),
            None,
            None,
        )
        .await;

        let s1 = reconcile_work_series(&db, &w1, SeriesLinkOrigin::System)
            .await
            .unwrap()
            .unwrap();
        let s2 = reconcile_work_series(&db, &w2, SeriesLinkOrigin::System)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(s1, s2);
        assert_eq!(db.list_all_series(user_id).await.unwrap().len(), 1);
        assert_eq!(db.count_works_in_series(user_id, s1).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn distinct_series_names_coexist_as_distinct_stubs() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed_user_author(&db).await;
        let w1 = seed_work(
            &db,
            user_id,
            Some(author_id),
            "Storm Front",
            Some("The Dresden Files"),
            None,
            None,
        )
        .await;
        let w2 = seed_work(
            &db,
            user_id,
            Some(author_id),
            "Furies of Calderon",
            Some("Codex Alera"),
            None,
            None,
        )
        .await;

        let s1 = reconcile_work_series(&db, &w1, SeriesLinkOrigin::System)
            .await
            .unwrap()
            .unwrap();
        let s2 = reconcile_work_series(&db, &w2, SeriesLinkOrigin::System)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(s1, s2, "AC-011: distinct stubs must not merge");
        assert_eq!(db.list_all_series(user_id).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn clearing_series_name_unlinks_and_gcs_empty_stub() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed_user_author(&db).await;
        let work = seed_work(
            &db,
            user_id,
            Some(author_id),
            "Storm Front",
            Some("The Dresden Files"),
            None,
            None,
        )
        .await;
        let sid = reconcile_work_series(&db, &work, SeriesLinkOrigin::System)
            .await
            .unwrap()
            .unwrap();

        // Simulate the user clearing series_name: reconcile sees the cleared
        // string + still-set FK (the update path's post-write state).
        let mut cleared = db.get_work(user_id, work.id).await.unwrap();
        cleared.series_name = None;
        let out = reconcile_work_series(&db, &cleared, SeriesLinkOrigin::User)
            .await
            .unwrap();
        assert_eq!(out, None);
        assert_eq!(db.get_work(user_id, work.id).await.unwrap().series_id, None);
        assert!(
            db.get_series(user_id, sid).await.unwrap().is_none(),
            "AC-012: empty unmonitored stub is GC'd"
        );
    }

    #[tokio::test]
    async fn monitored_series_survives_at_zero_works() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed_user_author(&db).await;
        let work = seed_work(
            &db,
            user_id,
            Some(author_id),
            "Storm Front",
            Some("The Dresden Files"),
            None,
            None,
        )
        .await;
        let sid = reconcile_work_series(&db, &work, SeriesLinkOrigin::System)
            .await
            .unwrap()
            .unwrap();
        // Monitor it (DB-level; the service-level stub guard is a different road).
        db.update_series_flags(user_id, sid, true, false, None)
            .await
            .unwrap();

        let mut cleared = db.get_work(user_id, work.id).await.unwrap();
        cleared.series_name = None;
        reconcile_work_series(&db, &cleared, SeriesLinkOrigin::User)
            .await
            .unwrap();
        assert!(
            db.get_series(user_id, sid).await.unwrap().is_some(),
            "AC-012: monitored series is never auto-deleted"
        );
        assert_eq!(db.count_works_in_series(user_id, sid).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn positional_suffix_normalizes_stub_and_work() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed_user_author(&db).await;
        let work = seed_work(
            &db,
            user_id,
            Some(author_id),
            "The Dragon Reborn",
            Some("The Wheel of Time, Book 3"),
            None,
            None,
        )
        .await;

        let sid = reconcile_work_series(&db, &work, SeriesLinkOrigin::System)
            .await
            .unwrap()
            .unwrap();
        let series = db.get_series(user_id, sid).await.unwrap().unwrap();
        assert_eq!(series.name, "The Wheel of Time", "AC-017: clean stub name");
        let reloaded = db.get_work(user_id, work.id).await.unwrap();
        assert_eq!(reloaded.series_name.as_deref(), Some("The Wheel of Time"));
        assert_eq!(reloaded.series_position, Some(3.0));
    }

    #[tokio::test]
    async fn existing_series_position_never_clobbered() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed_user_author(&db).await;
        let work = seed_work(
            &db,
            user_id,
            Some(author_id),
            "The Dragon Reborn",
            Some("The Wheel of Time, Book 5"),
            None,
            Some(3.0),
        )
        .await;

        reconcile_work_series(&db, &work, SeriesLinkOrigin::System)
            .await
            .unwrap();
        let reloaded = db.get_work(user_id, work.id).await.unwrap();
        assert_eq!(
            reloaded.series_position,
            Some(3.0),
            "AC-017: existing position kept"
        );
        assert_eq!(reloaded.series_name.as_deref(), Some("The Wheel of Time"));
    }

    #[tokio::test]
    async fn null_author_work_is_skipped_without_error() {
        let db = create_test_db().await;
        let (user_id, _author_id) = seed_user_author(&db).await;
        let work = seed_work(
            &db,
            user_id,
            None,
            "Orphan Book",
            Some("Some Series"),
            None,
            None,
        )
        .await;

        let out = reconcile_work_series(&db, &work, SeriesLinkOrigin::System)
            .await
            .unwrap();
        assert_eq!(out, None, "AC-019: NULL-author work skipped");
        assert!(db.list_all_series(user_id).await.unwrap().is_empty());
        assert_eq!(
            db.get_work(user_id, work.id)
                .await
                .unwrap()
                .series_name
                .as_deref(),
            Some("Some Series"),
            "series_name stays display-only"
        );
    }

    #[tokio::test]
    async fn system_write_never_displaces_gr_backed_link() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed_user_author(&db).await;
        let real = db
            .upsert_series(CreateSeriesDbRequest {
                user_id,
                author_id,
                name: "The Wheel of Time".to_string(),
                gr_key: "45175".to_string(),
                monitor_ebook: true,
                monitor_audiobook: false,
                monitor_language: None,
                work_count: 14,
            })
            .await
            .unwrap();
        // Work linked to the GR-backed row, then enrichment changes the string.
        let work = seed_work(
            &db,
            user_id,
            Some(author_id),
            "The Eye of the World",
            Some("Different Series Name"),
            Some(real.id),
            None,
        )
        .await;

        let out = reconcile_work_series(&db, &work, SeriesLinkOrigin::System)
            .await
            .unwrap();
        assert_eq!(out, Some(real.id), "AC-021: FK stays on the GR-backed row");
        assert_eq!(
            db.get_work(user_id, work.id).await.unwrap().series_id,
            Some(real.id)
        );
        assert_eq!(
            db.list_all_series(user_id).await.unwrap().len(),
            1,
            "no stub created for the enrichment string"
        );
    }

    #[tokio::test]
    async fn user_edit_relinks_away_from_gr_backed_row() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed_user_author(&db).await;
        let real = db
            .upsert_series(CreateSeriesDbRequest {
                user_id,
                author_id,
                name: "The Wheel of Time".to_string(),
                gr_key: "45175".to_string(),
                monitor_ebook: true,
                monitor_audiobook: false,
                monitor_language: None,
                work_count: 14,
            })
            .await
            .unwrap();
        let work = seed_work(
            &db,
            user_id,
            Some(author_id),
            "The Eye of the World",
            Some("My Custom Shelf"),
            Some(real.id),
            None,
        )
        .await;

        let out = reconcile_work_series(&db, &work, SeriesLinkOrigin::User)
            .await
            .unwrap();
        let new_sid = out.unwrap();
        assert_ne!(new_sid, real.id, "user edit always relinks");
        let stub = db.get_series(user_id, new_sid).await.unwrap().unwrap();
        assert!(is_stub_key(&stub.gr_key));
        assert_eq!(stub.name, "My Custom Shelf");
    }

    #[tokio::test]
    async fn matching_name_links_to_gr_backed_row_not_a_new_stub() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed_user_author(&db).await;
        let real = db
            .upsert_series(CreateSeriesDbRequest {
                user_id,
                author_id,
                name: "The Wheel of Time".to_string(),
                gr_key: "45175".to_string(),
                monitor_ebook: true,
                monitor_audiobook: false,
                monitor_language: None,
                work_count: 14,
            })
            .await
            .unwrap();
        let work = seed_work(
            &db,
            user_id,
            Some(author_id),
            "New Spring",
            Some("The Wheel of Time"),
            None,
            None,
        )
        .await;

        let out = reconcile_work_series(&db, &work, SeriesLinkOrigin::System)
            .await
            .unwrap();
        assert_eq!(out, Some(real.id), "name match prefers the GR-backed row");
        assert_eq!(db.list_all_series(user_id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reconcile_is_idempotent() {
        let db = create_test_db().await;
        let (user_id, author_id) = seed_user_author(&db).await;
        let work = seed_work(
            &db,
            user_id,
            Some(author_id),
            "Storm Front",
            Some("The Dresden Files"),
            None,
            None,
        )
        .await;

        let s1 = reconcile_work_series(&db, &work, SeriesLinkOrigin::System)
            .await
            .unwrap()
            .unwrap();
        let reloaded = db.get_work(user_id, work.id).await.unwrap();
        let s2 = reconcile_work_series(&db, &reloaded, SeriesLinkOrigin::System)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(s1, s2);
        assert_eq!(
            db.list_all_series(user_id).await.unwrap().len(),
            1,
            "AC-003: re-running creates no duplicates"
        );
    }
}
