// Behavioral tests for playback-enhancements feature.
//
// Written from IR v2 TDD directives (spec-driven). These tests verify
// ChapterDb, BookmarkDb, and PlaybackProgressDb extensions against the
// spec without referencing the implementation.

#[cfg(test)]
mod tests {
    use crate::{
        test_helpers::create_test_db, AuthorDb, BookmarkDb, ChapterDb, CreateAuthorDbRequest,
        CreateLibraryItemDbRequest, CreateUserDbRequest, CreateWorkDbRequest, DbError,
        LibraryItemDb, MediaType, PlaybackProgressDb, RootFolderDb, TagStatus, UserDb, UserRole,
        WorkDb,
    };
    use livrarr_domain::{AudiobookChapter, Bookmark};

    // -------------------------------------------------------------------------
    // Seed helpers
    // -------------------------------------------------------------------------

    struct TestSeed {
        user_id: i64,
        work_id: i64,
        audiobook_item_id: i64,
        ebook_item_id: i64,
    }

    struct TwoUserSeed {
        user_a: TestSeed,
        user_b_id: i64,
        user_b_work_id: i64,
        user_b_item_id: i64,
    }

    async fn seed_one_user(
        db: &(impl UserDb + AuthorDb + WorkDb + crate::WorkDbCreate + RootFolderDb + LibraryItemDb),
    ) -> TestSeed {
        let user = db
            .create_user(CreateUserDbRequest {
                username: "alice".to_string(),
                password_hash: "hash".to_string(),
                role: UserRole::User,
                api_key_hash: "api_hash".to_string(),
            })
            .await
            .unwrap();

        let author = db
            .create_author(CreateAuthorDbRequest {
                user_id: user.id,
                name: "Test Author".to_string(),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                hc_key: None,
                import_id: None,
            })
            .await
            .unwrap();

        let (work, _) = db
            .create_work(CreateWorkDbRequest {
                user_id: user.id,
                title: "Test Work".to_string(),
                author_name: "Test Author".to_string(),
                normalized_title: "test work".to_string(),
                normalized_author: "test author".to_string(),
                author_id: Some(author.id),
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                language: None,
                import_id: None,
                series_id: None,
                series_name: None,
                series_position: None,
                monitor_ebook: true,
                monitor_audiobook: true,
                source_provider_json: None,
                isbn_13: None,
                asin: None,
                description: None,
                cover_manual: false,
            })
            .await
            .unwrap();

        let rf_audio = db
            .create_root_folder("/audiobooks", MediaType::Audiobook)
            .await
            .unwrap();
        let rf_ebook = db
            .create_root_folder("/ebooks", MediaType::Ebook)
            .await
            .unwrap();

        let audiobook_item = db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id: user.id,
                work_id: work.id,
                root_folder_id: rf_audio.id,
                path: "author/work/work.m4b".to_string(),
                media_type: MediaType::Audiobook,
                file_size: 100_000,
                import_id: None,
                tag_status: TagStatus::Pending,
                tagged_at_generation: 0,
            })
            .await
            .unwrap();

        let ebook_item = db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id: user.id,
                work_id: work.id,
                root_folder_id: rf_ebook.id,
                path: "author/work/work.epub".to_string(),
                media_type: MediaType::Ebook,
                file_size: 50_000,
                import_id: None,
                tag_status: TagStatus::Pending,
                tagged_at_generation: 0,
            })
            .await
            .unwrap();

        TestSeed {
            user_id: user.id,
            work_id: work.id,
            audiobook_item_id: audiobook_item.id,
            ebook_item_id: ebook_item.id,
        }
    }

    async fn seed_two_users(
        db: &(impl UserDb + AuthorDb + WorkDb + crate::WorkDbCreate + RootFolderDb + LibraryItemDb),
    ) -> TwoUserSeed {
        let seed_a = seed_one_user(db).await;

        let user_b = db
            .create_user(CreateUserDbRequest {
                username: "bob".to_string(),
                password_hash: "hash_b".to_string(),
                role: UserRole::User,
                api_key_hash: "api_hash_b".to_string(),
            })
            .await
            .unwrap();

        let author_b = db
            .create_author(CreateAuthorDbRequest {
                user_id: user_b.id,
                name: "Author B".to_string(),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                hc_key: None,
                import_id: None,
            })
            .await
            .unwrap();

        let (work_b, _) = db
            .create_work(CreateWorkDbRequest {
                user_id: user_b.id,
                title: "Work B".to_string(),
                author_name: "Author B".to_string(),
                normalized_title: "work b".to_string(),
                normalized_author: "author b".to_string(),
                author_id: Some(author_b.id),
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                language: None,
                import_id: None,
                series_id: None,
                series_name: None,
                series_position: None,
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

        let rf = db
            .get_root_folder_by_media_type(MediaType::Audiobook)
            .await
            .unwrap()
            .unwrap();

        let item_b = db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id: user_b.id,
                work_id: work_b.id,
                root_folder_id: rf.id,
                path: "author-b/work-b/work-b.m4b".to_string(),
                media_type: MediaType::Audiobook,
                file_size: 200_000,
                import_id: None,
                tag_status: TagStatus::Pending,
                tagged_at_generation: 0,
            })
            .await
            .unwrap();

        TwoUserSeed {
            user_a: seed_a,
            user_b_id: user_b.id,
            user_b_work_id: work_b.id,
            user_b_item_id: item_b.id,
        }
    }

    fn make_chapters(item_id: i64, count: usize) -> Vec<AudiobookChapter> {
        (0..count)
            .map(|i| AudiobookChapter {
                id: 0,
                library_item_id: item_id,
                chapter_index: i as i32,
                title: format!("Chapter {}", i + 1),
                start_time_secs: i as f64 * 300.0,
                end_time_secs: (i + 1) as f64 * 300.0,
            })
            .collect()
    }

    fn make_bookmark(
        user_id: i64,
        work_id: i64,
        item_id: i64,
        position: &str,
        sort_key: f64,
        name: &str,
    ) -> Bookmark {
        Bookmark {
            id: 0,
            user_id,
            work_id,
            library_item_id: item_id,
            media_type: MediaType::Audiobook,
            position: position.to_string(),
            sort_key,
            name: name.to_string(),
            chapter_title: None,
            paired_bookmark_id: None,
            created_at: chrono::Utc::now(),
        }
    }

    // =========================================================================
    // ChapterDb tests (from IR v2 module: db-chapters)
    // =========================================================================

    #[tokio::test]
    async fn get_chapters_returns_sorted_by_start_time() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;
        let chapters = make_chapters(seed.audiobook_item_id, 3);
        db.replace_chapters(seed.audiobook_item_id, &chapters)
            .await
            .unwrap();

        let result = db.get_chapters(seed.audiobook_item_id).await.unwrap();
        assert_eq!(result.len(), 3);
        assert!(result[0].start_time_secs < result[1].start_time_secs);
        assert!(result[1].start_time_secs < result[2].start_time_secs);
    }

    #[tokio::test]
    async fn get_chapters_only_returns_chapters_for_requested_item() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;
        let chapters_a = make_chapters(seed.user_a.audiobook_item_id, 3);
        let chapters_b = make_chapters(seed.user_b_item_id, 2);

        db.replace_chapters(seed.user_a.audiobook_item_id, &chapters_a)
            .await
            .unwrap();
        db.replace_chapters(seed.user_b_item_id, &chapters_b)
            .await
            .unwrap();

        let result = db
            .get_chapters(seed.user_a.audiobook_item_id)
            .await
            .unwrap();
        assert_eq!(result.len(), 3, "must return only item A's chapters");
    }

    #[tokio::test]
    async fn get_chapters_nonexistent_item_returns_empty() {
        let db = create_test_db().await;
        let result = db.get_chapters(99999).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn replace_chapters_replaces_existing() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.replace_chapters(
            seed.audiobook_item_id,
            &make_chapters(seed.audiobook_item_id, 3),
        )
        .await
        .unwrap();
        let new_chapters = make_chapters(seed.audiobook_item_id, 2);
        db.replace_chapters(seed.audiobook_item_id, &new_chapters)
            .await
            .unwrap();

        let result = db.get_chapters(seed.audiobook_item_id).await.unwrap();
        assert_eq!(result.len(), 2, "old chapters must be replaced");
    }

    #[tokio::test]
    async fn replace_chapters_with_empty_clears_all() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.replace_chapters(
            seed.audiobook_item_id,
            &make_chapters(seed.audiobook_item_id, 3),
        )
        .await
        .unwrap();
        db.replace_chapters(seed.audiobook_item_id, &[])
            .await
            .unwrap();

        let result = db.get_chapters(seed.audiobook_item_id).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn replace_chapters_does_not_affect_bookmarks() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        let bm = make_bookmark(
            seed.user_id,
            seed.work_id,
            seed.audiobook_item_id,
            "120.5",
            120.5,
            "My Bookmark",
        );
        db.create_bookmark(&bm).await.unwrap();

        db.replace_chapters(
            seed.audiobook_item_id,
            &make_chapters(seed.audiobook_item_id, 5),
        )
        .await
        .unwrap();

        let bookmarks = db
            .list_bookmarks(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap();
        assert_eq!(bookmarks.len(), 1, "bookmark must survive chapter replace");
    }

    #[tokio::test]
    async fn has_chapters_false_when_empty() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;
        assert!(!db.has_chapters(seed.audiobook_item_id).await.unwrap());
    }

    #[tokio::test]
    async fn has_chapters_true_after_insert() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;
        db.replace_chapters(
            seed.audiobook_item_id,
            &make_chapters(seed.audiobook_item_id, 2),
        )
        .await
        .unwrap();
        assert!(db.has_chapters(seed.audiobook_item_id).await.unwrap());
    }

    #[tokio::test]
    async fn list_unscanned_returns_only_null_status_audiobooks() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        // audiobook_item has NULL scan_status → should appear
        // ebook_item has NULL scan_status but is ebook → should NOT appear
        let unscanned = db.list_unscanned_audiobook_items().await.unwrap();
        let ids: Vec<i64> = unscanned.iter().map(|(id, _)| *id).collect();
        assert!(
            ids.contains(&seed.audiobook_item_id),
            "audiobook with NULL scan_status must appear"
        );
        assert!(
            !ids.contains(&seed.ebook_item_id),
            "ebook must NOT appear in unscanned audiobook list"
        );
    }

    #[tokio::test]
    async fn list_unscanned_excludes_already_scanned() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.update_chapter_scan_result(seed.audiobook_item_id, "scanned", Some(3600.0))
            .await
            .unwrap();

        let unscanned = db.list_unscanned_audiobook_items().await.unwrap();
        let ids: Vec<i64> = unscanned.iter().map(|(id, _)| *id).collect();
        assert!(
            !ids.contains(&seed.audiobook_item_id),
            "scanned item must be excluded"
        );
    }

    #[tokio::test]
    async fn update_chapter_scan_result_sets_status_and_duration() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.update_chapter_scan_result(seed.audiobook_item_id, "scanned", Some(7200.0))
            .await
            .unwrap();

        let item = db
            .get_library_item(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap();
        assert_eq!(item.chapter_scan_status.as_deref(), Some("scanned"));
        assert_eq!(item.duration_seconds, Some(7200.0));
    }

    #[tokio::test]
    async fn update_chapter_scan_result_preserves_existing_duration_when_none() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.update_chapter_scan_result(seed.audiobook_item_id, "scanned", Some(3600.0))
            .await
            .unwrap();
        db.update_chapter_scan_result(seed.audiobook_item_id, "scanned", None)
            .await
            .unwrap();

        let item = db
            .get_library_item(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap();
        assert_eq!(
            item.duration_seconds,
            Some(3600.0),
            "existing duration must be preserved when None passed"
        );
    }

    // =========================================================================
    // BookmarkDb tests (from IR v2 module: db-bookmarks)
    // =========================================================================

    #[tokio::test]
    async fn list_bookmarks_user_scoped() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        let bm_a = make_bookmark(
            seed.user_a.user_id,
            seed.user_a.work_id,
            seed.user_a.audiobook_item_id,
            "60.0",
            60.0,
            "Bookmark A",
        );
        db.create_bookmark(&bm_a).await.unwrap();

        let bm_b = make_bookmark(
            seed.user_b_id,
            seed.user_b_work_id,
            seed.user_b_item_id,
            "120.0",
            120.0,
            "Bookmark B",
        );
        db.create_bookmark(&bm_b).await.unwrap();

        let list_a = db
            .list_bookmarks(seed.user_a.user_id, seed.user_a.audiobook_item_id)
            .await
            .unwrap();
        assert_eq!(list_a.len(), 1);
        assert_eq!(list_a[0].name, "Bookmark A");
    }

    #[tokio::test]
    async fn bookmarks_sorted_by_sort_key() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        let bm1 = make_bookmark(
            seed.user_id,
            seed.work_id,
            seed.audiobook_item_id,
            "300.0",
            300.0,
            "Later",
        );
        let bm2 = make_bookmark(
            seed.user_id,
            seed.work_id,
            seed.audiobook_item_id,
            "60.0",
            60.0,
            "Earlier",
        );
        db.create_bookmark(&bm1).await.unwrap();
        db.create_bookmark(&bm2).await.unwrap();

        let list = db
            .list_bookmarks(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap();
        assert_eq!(list[0].name, "Earlier");
        assert_eq!(list[1].name, "Later");
    }

    #[tokio::test]
    async fn create_bookmark_returns_with_id() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        let bm = make_bookmark(
            seed.user_id,
            seed.work_id,
            seed.audiobook_item_id,
            "45.0",
            45.0,
            "Test BM",
        );
        let created = db.create_bookmark(&bm).await.unwrap();
        assert!(created.id > 0, "created bookmark must have a non-zero ID");
        assert_eq!(created.name, "Test BM");
    }

    #[tokio::test]
    async fn create_bookmark_appears_in_list() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        let bm = make_bookmark(
            seed.user_id,
            seed.work_id,
            seed.audiobook_item_id,
            "90.0",
            90.0,
            "Listed BM",
        );
        db.create_bookmark(&bm).await.unwrap();

        let list = db
            .list_bookmarks(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Listed BM");
    }

    #[tokio::test]
    async fn rename_bookmark_changes_name() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        let bm = make_bookmark(
            seed.user_id,
            seed.work_id,
            seed.audiobook_item_id,
            "100.0",
            100.0,
            "Old Name",
        );
        let created = db.create_bookmark(&bm).await.unwrap();

        db.rename_bookmark(seed.user_id, created.id, "New Name")
            .await
            .unwrap();

        let list = db
            .list_bookmarks(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap();
        assert_eq!(list[0].name, "New Name");
    }

    #[tokio::test]
    async fn rename_other_users_bookmark_returns_not_found() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        let bm = make_bookmark(
            seed.user_a.user_id,
            seed.user_a.work_id,
            seed.user_a.audiobook_item_id,
            "100.0",
            100.0,
            "A's Bookmark",
        );
        let created = db.create_bookmark(&bm).await.unwrap();

        let result = db
            .rename_bookmark(seed.user_b_id, created.id, "Hijacked")
            .await;
        assert!(
            matches!(result, Err(DbError::NotFound { .. })),
            "IDOR: must not rename another user's bookmark"
        );
    }

    #[tokio::test]
    async fn delete_bookmark_removes_it() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        let bm = make_bookmark(
            seed.user_id,
            seed.work_id,
            seed.audiobook_item_id,
            "200.0",
            200.0,
            "Doomed",
        );
        let created = db.create_bookmark(&bm).await.unwrap();

        db.delete_bookmark(seed.user_id, created.id).await.unwrap();

        let list = db
            .list_bookmarks(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn delete_other_users_bookmark_returns_not_found() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        let bm = make_bookmark(
            seed.user_a.user_id,
            seed.user_a.work_id,
            seed.user_a.audiobook_item_id,
            "200.0",
            200.0,
            "A's BM",
        );
        let created = db.create_bookmark(&bm).await.unwrap();

        let result = db.delete_bookmark(seed.user_b_id, created.id).await;
        assert!(
            matches!(result, Err(DbError::NotFound { .. })),
            "IDOR: must not delete another user's bookmark"
        );
    }

    #[tokio::test]
    async fn delete_paired_bookmark_unpairs_partner() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        let bm_a = make_bookmark(
            seed.user_id,
            seed.work_id,
            seed.audiobook_item_id,
            "100.0",
            100.0,
            "BM A",
        );
        let bm_b = make_bookmark(
            seed.user_id,
            seed.work_id,
            seed.ebook_item_id,
            "epubcfi(/6/10)",
            0.25,
            "BM B",
        );
        let created_a = db.create_bookmark(&bm_a).await.unwrap();
        let created_b = db.create_bookmark(&bm_b).await.unwrap();

        // Manually set paired_bookmark_id bidirectionally via raw SQL
        sqlx::query("UPDATE bookmarks SET paired_bookmark_id = ? WHERE id = ?")
            .bind(created_b.id)
            .bind(created_a.id)
            .execute(db.pool())
            .await
            .unwrap();
        sqlx::query("UPDATE bookmarks SET paired_bookmark_id = ? WHERE id = ?")
            .bind(created_a.id)
            .bind(created_b.id)
            .execute(db.pool())
            .await
            .unwrap();

        // Delete A
        db.delete_bookmark(seed.user_id, created_a.id)
            .await
            .unwrap();

        // B should still exist with paired_bookmark_id = NULL (ON DELETE SET NULL)
        let list = db
            .list_bookmarks(seed.user_id, seed.ebook_item_id)
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert!(
            list[0].paired_bookmark_id.is_none(),
            "partner's paired_bookmark_id must be NULL after deletion"
        );
    }

    // =========================================================================
    // PlaybackProgressDb extended tests (from IR v2 module: db-progress-extended)
    // =========================================================================

    #[tokio::test]
    async fn upsert_progress_sets_finished_at_at_98_pct() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "3528.0", 0.99)
            .await
            .unwrap();

        let prog = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            prog.finished_at.is_some(),
            "finished_at must be set when pct >= 0.98"
        );
    }

    #[tokio::test]
    async fn upsert_progress_clears_finished_at_below_95_pct() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        // First set to finished
        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "3528.0", 0.99)
            .await
            .unwrap();
        // Then seek backward
        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "1800.0", 0.50)
            .await
            .unwrap();

        let prog = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            prog.finished_at.is_none(),
            "finished_at must be cleared when pct < 0.95"
        );
    }

    #[tokio::test]
    async fn upsert_progress_preserves_finished_at_in_dead_zone() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "3528.0", 0.99)
            .await
            .unwrap();
        let original = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();

        // 0.96 is in [0.95, 0.98) — dead zone
        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "3420.0", 0.96)
            .await
            .unwrap();

        let prog = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            prog.finished_at, original.finished_at,
            "finished_at must be preserved in dead zone [0.95, 0.98)"
        );
    }

    #[tokio::test]
    async fn upsert_progress_does_not_overwrite_existing_finished_at() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "3528.0", 0.99)
            .await
            .unwrap();
        let first = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();

        // Another update at 0.99 should NOT overwrite the timestamp
        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "3530.0", 0.99)
            .await
            .unwrap();
        let second = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            first.finished_at, second.finished_at,
            "existing finished_at must not be overwritten with a new timestamp"
        );
    }

    #[tokio::test]
    async fn fresh_insert_at_99_pct_sets_finished_at() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.upsert_progress(seed.user_id, seed.ebook_item_id, "epubcfi(/6/98)", 0.99)
            .await
            .unwrap();

        let prog = db
            .get_progress(seed.user_id, seed.ebook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            prog.finished_at.is_some(),
            "fresh insert at 0.99 must set finished_at"
        );
    }

    #[tokio::test]
    async fn fresh_insert_at_50_pct_has_null_finished_at() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.upsert_progress(seed.user_id, seed.ebook_item_id, "epubcfi(/6/50)", 0.50)
            .await
            .unwrap();

        let prog = db
            .get_progress(seed.user_id, seed.ebook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            prog.finished_at.is_none(),
            "fresh insert at 0.50 must have NULL finished_at"
        );
    }

    // ---- upsert_progress_no_lifecycle ----

    #[tokio::test]
    async fn no_lifecycle_at_99_pct_does_not_set_finished_at() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.upsert_progress_no_lifecycle(seed.user_id, seed.audiobook_item_id, "3528.0", 0.99)
            .await
            .unwrap();

        let prog = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            prog.finished_at.is_none(),
            "no_lifecycle must NOT set finished_at even at 0.99"
        );
    }

    #[tokio::test]
    async fn no_lifecycle_preserves_existing_finished_at() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        // Set finished_at via normal upsert
        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "3528.0", 0.99)
            .await
            .unwrap();
        let original = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(original.finished_at.is_some());

        // no_lifecycle at 0.99 — must preserve
        db.upsert_progress_no_lifecycle(seed.user_id, seed.audiobook_item_id, "3530.0", 0.99)
            .await
            .unwrap();
        let after = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.finished_at, original.finished_at);
    }

    #[tokio::test]
    async fn no_lifecycle_does_not_clear_finished_at_on_backward_seek() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        // Set finished_at
        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "3528.0", 0.99)
            .await
            .unwrap();

        // Backward seek via no_lifecycle — must NOT clear finished_at
        db.upsert_progress_no_lifecycle(seed.user_id, seed.audiobook_item_id, "1800.0", 0.50)
            .await
            .unwrap();

        let prog = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            prog.finished_at.is_some(),
            "no_lifecycle must not clear finished_at even on backward seek"
        );
    }

    // ---- get_progress_for_items (batch) ----

    #[tokio::test]
    async fn batch_progress_returns_correct_items() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "100.0", 0.30)
            .await
            .unwrap();
        db.upsert_progress(seed.user_id, seed.ebook_item_id, "epubcfi(/6/20)", 0.25)
            .await
            .unwrap();

        let results = db
            .get_progress_for_items(seed.user_id, &[seed.audiobook_item_id, seed.ebook_item_id])
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn batch_progress_empty_slice_returns_empty() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        let results = db.get_progress_for_items(seed.user_id, &[]).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn batch_progress_items_with_no_progress_returns_empty() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        let results = db
            .get_progress_for_items(seed.user_id, &[seed.audiobook_item_id])
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn batch_progress_user_isolation() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        db.upsert_progress(
            seed.user_a.user_id,
            seed.user_a.audiobook_item_id,
            "100.0",
            0.30,
        )
        .await
        .unwrap();

        // User B tries to batch-fetch user A's items
        let results = db
            .get_progress_for_items(seed.user_b_id, &[seed.user_a.audiobook_item_id])
            .await
            .unwrap();
        assert!(
            results.is_empty(),
            "user B must not see user A's progress via batch fetch"
        );
    }

    // ---- get_progress includes finished_at ----

    #[tokio::test]
    async fn get_progress_returns_finished_at_when_set() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "3528.0", 0.99)
            .await
            .unwrap();

        let prog = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(prog.finished_at.is_some());
    }

    #[tokio::test]
    async fn get_progress_returns_none_finished_at_when_not_set() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "100.0", 0.30)
            .await
            .unwrap();

        let prog = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(prog.finished_at.is_none());
    }

    // =========================================================================
    // Cascade delete tests (REQ-007)
    // =========================================================================

    #[tokio::test]
    async fn deleting_library_item_cascades_progress() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.upsert_progress(seed.user_id, seed.audiobook_item_id, "100.0", 0.30)
            .await
            .unwrap();
        db.delete_library_item(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap();

        let prog = db
            .get_progress(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap();
        assert!(
            prog.is_none(),
            "progress must be cascade-deleted with library item"
        );
    }

    #[tokio::test]
    async fn deleting_library_item_cascades_bookmarks() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        let bm = make_bookmark(
            seed.user_id,
            seed.work_id,
            seed.audiobook_item_id,
            "100.0",
            100.0,
            "Will Die",
        );
        db.create_bookmark(&bm).await.unwrap();

        db.delete_library_item(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap();

        let list = db
            .list_bookmarks(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap();
        assert!(
            list.is_empty(),
            "bookmarks must be cascade-deleted with library item"
        );
    }

    #[tokio::test]
    async fn deleting_library_item_cascades_chapters() {
        let db = create_test_db().await;
        let seed = seed_one_user(&db).await;

        db.replace_chapters(
            seed.audiobook_item_id,
            &make_chapters(seed.audiobook_item_id, 3),
        )
        .await
        .unwrap();

        db.delete_library_item(seed.user_id, seed.audiobook_item_id)
            .await
            .unwrap();

        let chapters = db.get_chapters(seed.audiobook_item_id).await.unwrap();
        assert!(
            chapters.is_empty(),
            "chapters must be cascade-deleted with library item"
        );
    }
}
