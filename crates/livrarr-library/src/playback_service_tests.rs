// Service-layer behavioral tests for playback-enhancements.
//
// Written from IR v2 TDD directives. Tests verify:
// - FileService::update_progress duration guard (audiobook lifecycle suppression)
// - ChapterService ownership check
// - BookmarkService ownership check + CRUD through service

#[cfg(test)]
mod tests {
    use livrarr_db::{
        test_helpers::create_test_db, AuthorDb, ChapterDb, CreateAuthorDbRequest,
        CreateLibraryItemDbRequest, CreateUserDbRequest, CreateWorkDbRequest, LibraryItemDb,
        MediaType, PlaybackProgressDb, RootFolderDb, TagStatus, UserDb, UserRole, WorkDb,
    };
    use livrarr_domain::services::{
        BookmarkService, ChapterService, FileService, FileServiceError, ProgressKind,
    };
    use livrarr_domain::AudiobookChapter;

    use crate::bookmark_service::BookmarkServiceImpl;
    use crate::chapter_service::ChapterServiceImpl;
    use crate::file_service::FileServiceImpl;

    // -------------------------------------------------------------------------
    // Seed helper
    // -------------------------------------------------------------------------

    struct ServiceTestSeed {
        user_id: i64,
        user_b_id: i64,
        work_id: i64,
        audiobook_item_id: i64,
        ebook_item_id: i64,
    }

    async fn seed(
        db: &(impl UserDb + AuthorDb + WorkDb + livrarr_db::WorkDbCreate + RootFolderDb + LibraryItemDb),
    ) -> ServiceTestSeed {
        let user = db
            .create_user(CreateUserDbRequest {
                username: "alice".to_string(),
                password_hash: "hash".to_string(),
                role: UserRole::User,
                api_key_hash: "api_hash".to_string(),
            })
            .await
            .unwrap();

        let user_b = db
            .create_user(CreateUserDbRequest {
                username: "bob".to_string(),
                password_hash: "hash_b".to_string(),
                role: UserRole::User,
                api_key_hash: "api_hash_b".to_string(),
            })
            .await
            .unwrap();

        let (author, _) = db
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

        ServiceTestSeed {
            user_id: user.id,
            user_b_id: user_b.id,
            work_id: work.id,
            audiobook_item_id: audiobook_item.id,
            ebook_item_id: ebook_item.id,
        }
    }

    // =========================================================================
    // FileService::update_progress — duration guard (IR v2: works-list-progress)
    // =========================================================================

    #[tokio::test]
    async fn audiobook_with_finite_duration_sets_finished_at() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        // Set duration on the audiobook item
        db.update_chapter_scan_result(s.audiobook_item_id, "scanned", Some(3600.0))
            .await
            .unwrap();

        let svc = FileServiceImpl::new(db.clone());
        svc.update_progress(
            s.user_id,
            s.audiobook_item_id,
            "3564.0",
            0.99,
            ProgressKind::Progress,
            None,
        )
        .await
        .unwrap();

        let prog = db
            .get_progress(s.user_id, s.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            prog.finished_at.is_some(),
            "audiobook with finite duration at 0.99 must set finished_at"
        );
    }

    #[tokio::test]
    async fn audiobook_with_null_duration_does_not_set_finished_at() {
        let db = create_test_db().await;
        let s = seed(&db).await;
        // audiobook_item has NULL duration_seconds by default

        let svc = FileServiceImpl::new(db.clone());
        svc.update_progress(
            s.user_id,
            s.audiobook_item_id,
            "3564.0",
            0.99,
            ProgressKind::Progress,
            None,
        )
        .await
        .unwrap();

        let prog = db
            .get_progress(s.user_id, s.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            prog.finished_at.is_none(),
            "audiobook with NULL duration must NOT set finished_at"
        );
    }

    #[tokio::test]
    async fn ebook_always_sets_finished_at_no_duration_guard() {
        let db = create_test_db().await;
        let s = seed(&db).await;
        // ebook has no duration_seconds — no guard should apply

        let svc = FileServiceImpl::new(db.clone());
        svc.update_progress(
            s.user_id,
            s.ebook_item_id,
            "epubcfi(/6/98)",
            0.99,
            ProgressKind::Progress,
            None,
        )
        .await
        .unwrap();

        let prog = db
            .get_progress(s.user_id, s.ebook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            prog.finished_at.is_some(),
            "ebook at 0.99 must always set finished_at (no duration guard)"
        );
    }

    #[tokio::test]
    async fn audiobook_backward_seek_clears_finished_at() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        db.update_chapter_scan_result(s.audiobook_item_id, "scanned", Some(3600.0))
            .await
            .unwrap();

        let svc = FileServiceImpl::new(db.clone());
        svc.update_progress(
            s.user_id,
            s.audiobook_item_id,
            "3564.0",
            0.99,
            ProgressKind::Progress,
            None,
        )
        .await
        .unwrap();
        svc.update_progress(
            s.user_id,
            s.audiobook_item_id,
            "1800.0",
            0.50,
            ProgressKind::Seek,
            None,
        )
        .await
        .unwrap();

        let prog = db
            .get_progress(s.user_id, s.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert!(
            prog.finished_at.is_none(),
            "backward seek must clear finished_at"
        );
    }

    #[tokio::test]
    async fn resume_position_always_saved_regardless_of_duration() {
        let db = create_test_db().await;
        let s = seed(&db).await;
        // NULL duration — lifecycle suppressed, but position must still save

        let svc = FileServiceImpl::new(db.clone());
        svc.update_progress(
            s.user_id,
            s.audiobook_item_id,
            "1234.5",
            0.35,
            ProgressKind::Progress,
            None,
        )
        .await
        .unwrap();

        let prog = db
            .get_progress(s.user_id, s.audiobook_item_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(prog.position, "1234.5");
        assert!((prog.progress_pct - 0.35).abs() < 0.001);
    }

    // =========================================================================
    // ChapterService — ownership check (IR v2: service-chapters)
    // =========================================================================

    #[tokio::test]
    async fn get_chapters_for_owned_item_returns_chapters() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        let chapters: Vec<AudiobookChapter> = (0..3)
            .map(|i| AudiobookChapter {
                id: 0,
                library_item_id: s.audiobook_item_id,
                chapter_index: i,
                title: format!("Ch {}", i + 1),
                start_time_secs: i as f64 * 300.0,
                end_time_secs: (i + 1) as f64 * 300.0,
            })
            .collect();
        db.replace_chapters(s.audiobook_item_id, &chapters)
            .await
            .unwrap();

        let svc = ChapterServiceImpl::new(db);
        let result = svc
            .get_chapters(s.user_id, s.audiobook_item_id)
            .await
            .unwrap();
        assert_eq!(result.len(), 3);
    }

    #[tokio::test]
    async fn get_chapters_for_unowned_item_returns_error() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        let svc = ChapterServiceImpl::new(db);
        let result = svc.get_chapters(s.user_b_id, s.audiobook_item_id).await;
        assert!(
            matches!(result, Err(FileServiceError::Db(_))),
            "must not return chapters for unowned item"
        );
    }

    // =========================================================================
    // BookmarkService — ownership + CRUD (IR v2: service-bookmarks)
    // =========================================================================

    #[tokio::test]
    async fn create_bookmark_returns_with_id() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        let svc = BookmarkServiceImpl::new(db);
        let bm = svc
            .create_bookmark(s.user_id, s.audiobook_item_id, "60.0", 60.0, "My BM", None)
            .await
            .unwrap();
        assert!(bm.id > 0);
        assert_eq!(bm.work_id, s.work_id, "work_id must be derived from item");
    }

    #[tokio::test]
    async fn create_bookmark_for_unowned_item_fails() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        let svc = BookmarkServiceImpl::new(db);
        let result = svc
            .create_bookmark(s.user_b_id, s.audiobook_item_id, "60.0", 60.0, "Hack", None)
            .await;
        assert!(result.is_err(), "must not create bookmark for unowned item");
    }

    #[tokio::test]
    async fn list_bookmarks_for_unowned_item_fails() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        let svc = BookmarkServiceImpl::new(db);
        let result = svc.list_bookmarks(s.user_b_id, s.audiobook_item_id).await;
        assert!(result.is_err(), "must not list bookmarks for unowned item");
    }

    #[tokio::test]
    async fn rename_other_users_bookmark_fails() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        let svc = BookmarkServiceImpl::new(db);
        let bm = svc
            .create_bookmark(s.user_id, s.audiobook_item_id, "60.0", 60.0, "Mine", None)
            .await
            .unwrap();

        let result = svc.rename_bookmark(s.user_b_id, bm.id, "Stolen").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn delete_other_users_bookmark_fails() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        let svc = BookmarkServiceImpl::new(db);
        let bm = svc
            .create_bookmark(s.user_id, s.audiobook_item_id, "60.0", 60.0, "Mine", None)
            .await
            .unwrap();

        let result = svc.delete_bookmark(s.user_b_id, bm.id).await;
        assert!(result.is_err());
    }

    // =========================================================================
    // FileService::get_progress_for_items (IR v2: works-list-progress)
    // =========================================================================

    #[tokio::test]
    async fn batch_progress_returns_correct_items() {
        let db = create_test_db().await;
        let s = seed(&db).await;

        let svc = FileServiceImpl::new(db.clone());
        svc.update_progress(
            s.user_id,
            s.audiobook_item_id,
            "100.0",
            0.30,
            ProgressKind::Progress,
            None,
        )
        .await
        .unwrap();
        svc.update_progress(
            s.user_id,
            s.ebook_item_id,
            "epubcfi(/6/20)",
            0.25,
            ProgressKind::Progress,
            None,
        )
        .await
        .unwrap();

        let results = svc
            .get_progress_for_items(s.user_id, &[s.audiobook_item_id, s.ebook_item_id])
            .await
            .unwrap();
        assert_eq!(results.len(), 2);
    }
}
