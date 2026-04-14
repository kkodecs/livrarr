// Cross-user isolation tests for livrarr-db.
//
// These tests prove that AUTH-003 holds: every user-scoped query is fenced by
// user_id and cannot leak or mutate another user's data.
//
// Intentional cross-user methods (background jobs) are documented in
// `background_job_queries_are_cross_user` — they are verified to be
// cross-user BY DESIGN, not by accident.

#[cfg(test)]
mod tests {
    use crate::{
        test_helpers::create_test_db, AuthorDb, AuthorId, CreateAuthorDbRequest,
        CreateDownloadClientDbRequest, CreateGrabDbRequest, CreateImportDbRequest,
        CreateLibraryItemDbRequest, CreateSeriesDbRequest, CreateUserDbRequest,
        CreateWorkDbRequest, DbError, DownloadClientDb, DownloadClientImplementation, GrabDb,
        GrabStatus, ImportDb, LibraryItemDb, MediaType, RootFolderDb, SeriesDb, UserDb, UserId,
        UserRole, WorkDb, WorkId,
    };

    // -------------------------------------------------------------------------
    // Seed helper
    // -------------------------------------------------------------------------

    struct TwoUsers {
        user_a_id: UserId,
        user_b_id: UserId,
        author_a_id: AuthorId,
        author_b_id: AuthorId,
        work_a_id: WorkId,
        work_b_id: WorkId,
    }

    async fn seed_two_users(db: &(impl UserDb + AuthorDb + WorkDb)) -> TwoUsers {
        let user_a = db
            .create_user(CreateUserDbRequest {
                username: "alice".to_string(),
                password_hash: "hash_a".to_string(),
                role: UserRole::User,
                api_key_hash: "api_hash_a".to_string(),
            })
            .await
            .expect("create user_a");

        let user_b = db
            .create_user(CreateUserDbRequest {
                username: "bob".to_string(),
                password_hash: "hash_b".to_string(),
                role: UserRole::User,
                api_key_hash: "api_hash_b".to_string(),
            })
            .await
            .expect("create user_b");

        let author_a = db
            .create_author(CreateAuthorDbRequest {
                user_id: user_a.id,
                name: "Author A".to_string(),
                sort_name: None,
                ol_key: None,
                gr_key: None,
                hc_key: None,
                import_id: None,
            })
            .await
            .expect("create author_a");

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
            .expect("create author_b");

        let work_a = db
            .create_work(CreateWorkDbRequest {
                user_id: user_a.id,
                title: "Work A".to_string(),
                author_name: "Author A".to_string(),
                author_id: Some(author_a.id),
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                metadata_source: None,
                detail_url: None,
                language: None,
                import_id: None,
                series_id: None,
                series_name: None,
                series_position: None,
                monitor_ebook: true,
                monitor_audiobook: false,
            })
            .await
            .expect("create work_a");

        let work_b = db
            .create_work(CreateWorkDbRequest {
                user_id: user_b.id,
                title: "Work B".to_string(),
                author_name: "Author B".to_string(),
                author_id: Some(author_b.id),
                ol_key: None,
                gr_key: None,
                year: None,
                cover_url: None,
                metadata_source: None,
                detail_url: None,
                language: None,
                import_id: None,
                series_id: None,
                series_name: None,
                series_position: None,
                monitor_ebook: true,
                monitor_audiobook: false,
            })
            .await
            .expect("create work_b");

        TwoUsers {
            user_a_id: user_a.id,
            user_b_id: user_b.id,
            author_a_id: author_a.id,
            author_b_id: author_b.id,
            work_a_id: work_a.id,
            work_b_id: work_b.id,
        }
    }

    // -------------------------------------------------------------------------
    // Work isolation
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn get_work_wrong_user_returns_not_found() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        let result = db.get_work(seed.user_b_id, seed.work_a_id).await;
        assert!(
            matches!(result, Err(DbError::NotFound { .. })),
            "user_b must not be able to fetch user_a's work; got: {result:?}"
        );
    }

    #[tokio::test]
    async fn list_works_scoped_by_user() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        let works_a = db
            .list_works(seed.user_a_id)
            .await
            .expect("list_works for user_a");
        let ids_a: Vec<WorkId> = works_a.iter().map(|w| w.id).collect();

        assert!(
            ids_a.contains(&seed.work_a_id),
            "user_a list must contain work_a"
        );
        assert!(
            !ids_a.contains(&seed.work_b_id),
            "user_a list must NOT contain work_b"
        );
    }

    #[tokio::test]
    async fn delete_work_wrong_user_is_noop_or_not_found() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        let result = db.delete_work(seed.user_b_id, seed.work_a_id).await;
        // Either NotFound or (silently) affects 0 rows and returns an error.
        // The work must still exist after the attempted delete.
        match result {
            Err(DbError::NotFound { .. }) => {}
            Ok(_) => {
                // If somehow it returned Ok, the work must still be readable by its owner.
                let still_exists = db
                    .get_work(seed.user_a_id, seed.work_a_id)
                    .await
                    .expect("work_a must still exist after wrong-user delete attempt");
                assert_eq!(still_exists.id, seed.work_a_id, "work_a row was corrupted");
            }
            Err(other) => panic!("unexpected error on wrong-user delete: {other:?}"),
        }
    }

    #[tokio::test]
    async fn update_work_wrong_user_is_noop_or_not_found() {
        use crate::UpdateWorkUserFieldsDbRequest;

        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        let result = db
            .update_work_user_fields(
                seed.user_b_id,
                seed.work_a_id,
                UpdateWorkUserFieldsDbRequest {
                    title: Some("TAMPERED".to_string()),
                    author_name: None,
                    series_name: None,
                    series_position: None,
                    monitor_ebook: None,
                    monitor_audiobook: None,
                },
            )
            .await;

        match result {
            Err(DbError::NotFound { .. }) => {}
            Ok(_) => {
                // If it returned Ok the title must be unchanged for the real owner.
                let work = db
                    .get_work(seed.user_a_id, seed.work_a_id)
                    .await
                    .expect("work_a must still be readable after wrong-user update");
                assert_ne!(
                    work.title, "TAMPERED",
                    "user_b must not be able to update user_a's work title"
                );
            }
            Err(other) => panic!("unexpected error on wrong-user update: {other:?}"),
        }
    }

    // -------------------------------------------------------------------------
    // Author isolation
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn get_author_wrong_user_returns_not_found() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        let result = db.get_author(seed.user_b_id, seed.author_a_id).await;
        assert!(
            matches!(result, Err(DbError::NotFound { .. })),
            "user_b must not be able to fetch user_a's author; got: {result:?}"
        );
    }

    #[tokio::test]
    async fn list_authors_scoped_by_user() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        let authors_a = db
            .list_authors(seed.user_a_id)
            .await
            .expect("list_authors for user_a");
        let ids_a: Vec<AuthorId> = authors_a.iter().map(|a| a.id).collect();

        assert!(
            ids_a.contains(&seed.author_a_id),
            "user_a list must contain author_a"
        );
        assert!(
            !ids_a.contains(&seed.author_b_id),
            "user_a list must NOT contain author_b"
        );
    }

    // -------------------------------------------------------------------------
    // Grab isolation
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn list_grabs_scoped_by_user() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        // Grabs require a download_client_id FK — create one shared client.
        let dc = db
            .create_download_client(CreateDownloadClientDbRequest {
                name: "test-client".to_string(),
                implementation: DownloadClientImplementation::QBittorrent,
                host: "localhost".to_string(),
                port: 8080,
                use_ssl: false,
                skip_ssl_validation: false,
                url_base: None,
                username: None,
                password: None,
                category: "livrarr".to_string(),
                enabled: true,
                api_key: None,
            })
            .await
            .expect("create download client");

        let grab_a = db
            .upsert_grab(CreateGrabDbRequest {
                user_id: seed.user_a_id,
                work_id: seed.work_a_id,
                download_client_id: dc.id,
                title: "Grab A".to_string(),
                indexer: "idx1".to_string(),
                guid: "guid-a".to_string(),
                size: None,
                download_url: "http://example.com/a".to_string(),
                download_id: None,
                status: GrabStatus::Sent,
                media_type: Some(MediaType::Ebook),
            })
            .await
            .expect("upsert grab_a");

        let grab_b = db
            .upsert_grab(CreateGrabDbRequest {
                user_id: seed.user_b_id,
                work_id: seed.work_b_id,
                download_client_id: dc.id,
                title: "Grab B".to_string(),
                indexer: "idx1".to_string(),
                guid: "guid-b".to_string(),
                size: None,
                download_url: "http://example.com/b".to_string(),
                download_id: None,
                status: GrabStatus::Sent,
                media_type: Some(MediaType::Ebook),
            })
            .await
            .expect("upsert grab_b");

        let (grabs_a, _) = db
            .list_grabs_paginated(seed.user_a_id, 1, 100)
            .await
            .expect("list grabs for user_a");
        let ids_a: Vec<i64> = grabs_a.iter().map(|g| g.id).collect();

        assert!(
            ids_a.contains(&grab_a.id),
            "user_a grab list must contain grab_a"
        );
        assert!(
            !ids_a.contains(&grab_b.id),
            "user_a grab list must NOT contain grab_b"
        );

        let (grabs_b, _) = db
            .list_grabs_paginated(seed.user_b_id, 1, 100)
            .await
            .expect("list grabs for user_b");
        let ids_b: Vec<i64> = grabs_b.iter().map(|g| g.id).collect();

        assert!(
            ids_b.contains(&grab_b.id),
            "user_b grab list must contain grab_b"
        );
        assert!(
            !ids_b.contains(&grab_a.id),
            "user_b grab list must NOT contain grab_a"
        );
    }

    // -------------------------------------------------------------------------
    // Series isolation
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn series_isolation() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        let series_a = db
            .upsert_series(CreateSeriesDbRequest {
                user_id: seed.user_a_id,
                author_id: seed.author_a_id,
                name: "Series A".to_string(),
                gr_key: "gr-series-a".to_string(),
                monitor_ebook: false,
                monitor_audiobook: false,
                work_count: 3,
            })
            .await
            .expect("upsert series_a");

        let series_b = db
            .upsert_series(CreateSeriesDbRequest {
                user_id: seed.user_b_id,
                author_id: seed.author_b_id,
                name: "Series B".to_string(),
                gr_key: "gr-series-b".to_string(),
                monitor_ebook: false,
                monitor_audiobook: false,
                work_count: 5,
            })
            .await
            .expect("upsert series_b");

        let list_a = db
            .list_all_series(seed.user_a_id)
            .await
            .expect("list_all_series for user_a");
        let ids_a: Vec<i64> = list_a.iter().map(|s| s.id).collect();

        assert!(
            ids_a.contains(&series_a.id),
            "user_a series list must contain series_a"
        );
        assert!(
            !ids_a.contains(&series_b.id),
            "user_a series list must NOT contain series_b"
        );

        let list_b = db
            .list_all_series(seed.user_b_id)
            .await
            .expect("list_all_series for user_b");
        let ids_b: Vec<i64> = list_b.iter().map(|s| s.id).collect();

        assert!(
            ids_b.contains(&series_b.id),
            "user_b series list must contain series_b"
        );
        assert!(
            !ids_b.contains(&series_a.id),
            "user_b series list must NOT contain series_a"
        );
    }

    // -------------------------------------------------------------------------
    // Import isolation
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn list_imports_scoped_by_user() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        db.create_import(CreateImportDbRequest {
            id: "import-a".to_string(),
            user_id: seed.user_a_id,
            source: "readarr".to_string(),
            source_url: None,
            target_root_folder_id: None,
        })
        .await
        .expect("create import for user_a");

        db.create_import(CreateImportDbRequest {
            id: "import-b".to_string(),
            user_id: seed.user_b_id,
            source: "readarr".to_string(),
            source_url: None,
            target_root_folder_id: None,
        })
        .await
        .expect("create import for user_b");

        let imports_a = db
            .list_imports(seed.user_a_id)
            .await
            .expect("list_imports for user_a");
        let import_ids_a: Vec<String> = imports_a.iter().map(|i| i.id.clone()).collect();

        assert!(
            import_ids_a.contains(&"import-a".to_string()),
            "user_a import list must contain import-a"
        );
        assert!(
            !import_ids_a.contains(&"import-b".to_string()),
            "user_a import list must NOT contain import-b"
        );

        let imports_b = db
            .list_imports(seed.user_b_id)
            .await
            .expect("list_imports for user_b");
        let import_ids_b: Vec<String> = imports_b.iter().map(|i| i.id.clone()).collect();

        assert!(
            import_ids_b.contains(&"import-b".to_string()),
            "user_b import list must contain import-b"
        );
        assert!(
            !import_ids_b.contains(&"import-a".to_string()),
            "user_b import list must NOT contain import-a"
        );
    }

    // -------------------------------------------------------------------------
    // Library item isolation
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn library_item_isolation() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        // Library items require a root_folder FK.
        let root_folder = db
            .create_root_folder("/books", MediaType::Ebook)
            .await
            .expect("create root folder");

        let item_a = db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id: seed.user_a_id,
                work_id: seed.work_a_id,
                root_folder_id: root_folder.id,
                path: "author-a/work-a/work-a.epub".to_string(),
                media_type: MediaType::Ebook,
                file_size: 1024,
                import_id: None,
            })
            .await
            .expect("create library_item_a");

        let item_b = db
            .create_library_item(CreateLibraryItemDbRequest {
                user_id: seed.user_b_id,
                work_id: seed.work_b_id,
                root_folder_id: root_folder.id,
                path: "author-b/work-b/work-b.epub".to_string(),
                media_type: MediaType::Ebook,
                file_size: 2048,
                import_id: None,
            })
            .await
            .expect("create library_item_b");

        let items_a = db
            .list_library_items(seed.user_a_id)
            .await
            .expect("list_library_items for user_a");
        let item_ids_a: Vec<i64> = items_a.iter().map(|i| i.id).collect();

        assert!(
            item_ids_a.contains(&item_a.id),
            "user_a item list must contain item_a"
        );
        assert!(
            !item_ids_a.contains(&item_b.id),
            "user_a item list must NOT contain item_b"
        );

        // Confirm get_library_item is also scoped.
        let wrong_user_fetch = db.get_library_item(seed.user_b_id, item_a.id).await;
        assert!(
            matches!(wrong_user_fetch, Err(DbError::NotFound { .. })),
            "user_b must not be able to fetch user_a's library item; got: {wrong_user_fetch:?}"
        );
    }

    // -------------------------------------------------------------------------
    // Background job queries — intentionally cross-user (document + verify)
    // -------------------------------------------------------------------------

    /// `list_monitored_works_all_users` is a background-job query that intentionally
    /// spans all users. This test seeds both users with monitored works and asserts
    /// that BOTH appear in the result — proving cross-user access is deliberate and
    /// functional, not a bug.
    #[tokio::test]
    async fn background_job_queries_are_cross_user() {
        let db = create_test_db().await;
        let seed = seed_two_users(&db).await;

        // Both works were created with monitor_ebook=true in seed_two_users,
        // so both should appear in list_monitored_works_all_users.
        let all_monitored = db
            .list_monitored_works_all_users()
            .await
            .expect("list_monitored_works_all_users");

        let ids: Vec<WorkId> = all_monitored.iter().map(|w| w.id).collect();

        assert!(
            ids.contains(&seed.work_a_id),
            "cross-user monitored list must include work_a (user_a's monitored work)"
        );
        assert!(
            ids.contains(&seed.work_b_id),
            "cross-user monitored list must include work_b (user_b's monitored work)"
        );
    }
}
