use async_trait::async_trait;
use librarr_db::*;

#[async_trait]
pub trait DbTestHarness: Send + Sync {
    type Db: WorkDb + AuthorDb + LibraryItemDb + RootFolderDb;
    async fn setup() -> Self;
    fn db(&self) -> &Self::Db;
    fn user_ids(&self) -> (UserId, UserId);
}

#[macro_export]
macro_rules! db_core_tests {
    ($harness:ty) => {
        fn make_work_req(user_id: UserId, title: &str, author: &str) -> CreateWorkDbRequest {
            CreateWorkDbRequest {
                user_id,
                title: title.to_string(),
                author_name: author.to_string(),
                author_id: None,
                ol_key: None,
                year: Some(2024),
                cover_url: Some("https://example.test/cover.jpg".to_string()),
            }
        }

        fn make_author_req(user_id: UserId, name: &str) -> CreateAuthorDbRequest {
            CreateAuthorDbRequest {
                user_id,
                name: name.to_string(),
                sort_name: Some(name.to_string()),
                ol_key: None,
            }
        }

        async fn make_root_folder<DB: RootFolderDb>(
            db: &DB,
            path: &str,
            media_type: MediaType,
        ) -> RootFolder {
            db.create_root_folder(path, media_type).await.unwrap()
        }

        fn make_library_item_req(
            user_id: UserId,
            work_id: WorkId,
            root_folder_id: RootFolderId,
            path: &str,
        ) -> CreateLibraryItemDbRequest {
            CreateLibraryItemDbRequest {
                user_id,
                work_id,
                root_folder_id,
                path: path.to_string(),
                media_type: MediaType::Ebook,
                file_size: 1234,
            }
        }

        #[tokio::test]
        async fn nominal_work_create_get_list_update_delete() {
            // Satisfies: AUTH-003, SEARCH-013
            // IR: WorkDb::{create_work,get_work,list_works,update_work_user_fields,delete_work}
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let created = db
                .create_work(make_work_req(u1, "Title One", "Author One"))
                .await
                .unwrap();
            let got = db.get_work(u1, created.id).await.unwrap();
            assert_eq!(got.id, created.id);
            assert_eq!(got.user_id, u1);
            assert_eq!(got.title, "Title One");
            assert_eq!(got.author_name, "Author One");

            let listed = db.list_works(u1).await.unwrap();
            assert!(listed.iter().any(|w| w.id == created.id));

            let updated = db
                .update_work_user_fields(
                    u1,
                    created.id,
                    UpdateWorkUserFieldsDbRequest {
                        title: Some("Retitled".to_string()),
                        author_name: Some("Renamed Author".to_string()),
                        series_name: Some("Series A".to_string()),
                        series_position: Some(2.5),
                    },
                )
                .await
                .unwrap();
            assert_eq!(updated.title, "Retitled");
            assert_eq!(updated.author_name, "Renamed Author");
            assert_eq!(updated.series_name.as_deref(), Some("Series A"));
            assert_eq!(updated.series_position, Some(2.5));

            let got2 = db.get_work(u1, created.id).await.unwrap();
            assert_eq!(got2.title, "Retitled");

            let deleted = db.delete_work(u1, created.id).await.unwrap();
            assert_eq!(deleted.id, created.id);
            assert!(matches!(
                db.get_work(u1, created.id).await,
                Err(DbError::NotFound)
            ));
        }

        #[tokio::test]
        async fn nominal_work_specific_queries() {
            // Satisfies: SEARCH-004, SEARCH-011, AUTHOR-002, IMPORT-017
            // IR: WorkDb::{work_exists_by_ol_key,list_works_for_enrichment,list_works_by_author_ol_keys,find_by_normalized_match}
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let mut req = make_work_req(u1, "The Stand", "Stephen King");
            req.ol_key = Some("OL123W".to_string());
            let created = db.create_work(req).await.unwrap();

            assert!(db.work_exists_by_ol_key(u1, "OL123W").await.unwrap());
            let enrich = db.list_works_for_enrichment(u1).await.unwrap();
            assert!(enrich.iter().any(|w| w.id == created.id));

            let by_author = db
                .list_works_by_author_ol_keys(u1, "Stephen King")
                .await
                .unwrap();
            assert!(by_author.iter().any(|k| k == "OL123W"));

            let matched = db
                .find_by_normalized_match(u1, "The Stand", "Stephen King")
                .await
                .unwrap();
            assert!(matched.iter().any(|w| w.id == created.id));
        }

        #[tokio::test]
        async fn nominal_author_create_get_list_update_delete_and_find() {
            // Satisfies: SEARCH-005, AUTHOR-001
            // IR: AuthorDb::{create_author,get_author,list_authors,update_author,delete_author,find_author_by_name}
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let created = db
                .create_author(make_author_req(u1, "Jane Austen"))
                .await
                .unwrap();
            let got = db.get_author(u1, created.id).await.unwrap();
            assert_eq!(got.id, created.id);
            assert_eq!(got.name, "Jane Austen");

            let listed = db.list_authors(u1).await.unwrap();
            assert!(listed.iter().any(|a| a.id == created.id));

            let updated = db
                .update_author(
                    u1,
                    created.id,
                    UpdateAuthorDbRequest {
                        name: Some("Jane A.".to_string()),
                        sort_name: Some("Austen, Jane".to_string()),
                        ol_key: Some("OLAUTH1A".to_string()),
                        monitored: Some(true),
                        monitor_new_items: Some(true),
                        monitor_since: None,
                    },
                )
                .await
                .unwrap();
            assert_eq!(updated.name, "Jane A.");
            assert_eq!(updated.ol_key.as_deref(), Some("OLAUTH1A"));
            assert!(updated.monitored);

            let found = db.find_author_by_name(u1, "Jane A.").await.unwrap();
            assert!(found.is_some());
            assert_eq!(found.unwrap().id, created.id);

            db.delete_author(u1, created.id).await.unwrap();
            assert!(matches!(
                db.get_author(u1, created.id).await,
                Err(DbError::NotFound)
            ));
        }

        #[tokio::test]
        async fn nominal_root_folder_create_get_list_by_media_type_path_roundtrip() {
            // Satisfies: IMPORT-001, IMPORT-002
            // IR: RootFolderDb::{create_root_folder,get_root_folder,list_root_folders,get_root_folder_by_media_type}
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();

            let rf = db
                .create_root_folder("/books/ebooks", MediaType::Ebook)
                .await
                .unwrap();
            assert_eq!(rf.path, "/books/ebooks");
            assert_eq!(rf.media_type, MediaType::Ebook);

            let got = db.get_root_folder(rf.id).await.unwrap();
            assert_eq!(got.id, rf.id);
            assert_eq!(got.path, "/books/ebooks");

            let listed = db.list_root_folders().await.unwrap();
            assert!(listed.iter().any(|r| r.id == rf.id));

            let by_type = db
                .get_root_folder_by_media_type(MediaType::Ebook)
                .await
                .unwrap();
            assert_eq!(by_type.unwrap().id, rf.id);
        }

        #[tokio::test]
        async fn nominal_library_item_create_get_list_by_work_taggable_delete() {
            // Satisfies: IMPORT-004, TAG-007
            // IR: LibraryItemDb::{create_library_item,get_library_item,list_library_items,list_library_items_by_work,list_taggable_items_by_work,delete_library_item,library_items_exist_for_root}
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work = db
                .create_work(make_work_req(u1, "Item Work", "Item Author"))
                .await
                .unwrap();
            let root = make_root_folder(db, "/library", MediaType::Ebook).await;

            assert!(!db.library_items_exist_for_root(root.id).await.unwrap());

            let item = db
                .create_library_item(make_library_item_req(u1, work.id, root.id, "a/book.epub"))
                .await
                .unwrap();
            let got = db.get_library_item(u1, item.id).await.unwrap();
            assert_eq!(got.id, item.id);
            assert_eq!(got.work_id, work.id);

            let listed = db.list_library_items(u1).await.unwrap();
            assert!(listed.iter().any(|i| i.id == item.id));

            let by_work = db.list_library_items_by_work(u1, work.id).await.unwrap();
            assert!(by_work.iter().any(|i| i.id == item.id));

            let taggable = db.list_taggable_items_by_work(u1, work.id).await.unwrap();
            assert!(taggable.iter().any(|i| i.id == item.id));

            assert!(db.library_items_exist_for_root(root.id).await.unwrap());

            let deleted = db.delete_library_item(u1, item.id).await.unwrap();
            assert_eq!(deleted.id, item.id);
            assert!(matches!(
                db.get_library_item(u1, item.id).await,
                Err(DbError::NotFound)
            ));
        }

        #[tokio::test]
        async fn failure_nonexistent_ids_return_not_found() {
            // Satisfies: AUTH-003
            // IR: WorkDb::{get_work,update_work_user_fields,delete_work}; AuthorDb::{get_author,update_author,delete_author}; LibraryItemDb::{get_library_item,delete_library_item}; RootFolderDb::{get_root_folder,delete_root_folder}
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert!(matches!(
                db.get_work(u1, 999999).await,
                Err(DbError::NotFound)
            ));
            assert!(matches!(
                db.update_work_user_fields(
                    u1,
                    999999,
                    UpdateWorkUserFieldsDbRequest {
                        title: Some("x".into()),
                        author_name: None,
                        series_name: None,
                        series_position: None
                    }
                )
                .await,
                Err(DbError::NotFound)
            ));
            assert!(matches!(
                db.delete_work(u1, 999999).await,
                Err(DbError::NotFound)
            ));

            assert!(matches!(
                db.get_author(u1, 999999).await,
                Err(DbError::NotFound)
            ));
            assert!(matches!(
                db.update_author(
                    u1,
                    999999,
                    UpdateAuthorDbRequest {
                        name: Some("x".into()),
                        sort_name: None,
                        ol_key: None,
                        monitored: None,
                        monitor_new_items: None,
                        monitor_since: None
                    }
                )
                .await,
                Err(DbError::NotFound)
            ));
            assert!(matches!(
                db.delete_author(u1, 999999).await,
                Err(DbError::NotFound)
            ));

            assert!(matches!(
                db.get_library_item(u1, 999999).await,
                Err(DbError::NotFound)
            ));
            assert!(matches!(
                db.delete_library_item(u1, 999999).await,
                Err(DbError::NotFound)
            ));

            assert!(matches!(
                db.get_root_folder(999999).await,
                Err(DbError::NotFound)
            ));
            assert!(matches!(
                db.delete_root_folder(999999).await,
                Err(DbError::NotFound)
            ));
        }

        #[tokio::test]
        async fn failure_duplicate_root_folder_media_type_is_constraint() {
            // Satisfies: IMPORT-001
            // IR: RootFolderDb::create_root_folder
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();

            db.create_root_folder("/one", MediaType::Ebook)
                .await
                .unwrap();
            let err = db
                .create_root_folder("/two", MediaType::Ebook)
                .await
                .unwrap_err();
            assert!(matches!(err, DbError::Constraint { .. }));
        }

        #[tokio::test]
        async fn failure_library_item_same_path_different_work_is_constraint() {
            // Satisfies: IMPORT-015
            // IR: LibraryItemDb::create_library_item
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let w1 = db.create_work(make_work_req(u1, "W1", "A")).await.unwrap();
            let w2 = db.create_work(make_work_req(u1, "W2", "A")).await.unwrap();
            let root = make_root_folder(db, "/items", MediaType::Ebook).await;

            db.create_library_item(make_library_item_req(u1, w1.id, root.id, "same.epub"))
                .await
                .unwrap();
            let err = db
                .create_library_item(make_library_item_req(u1, w2.id, root.id, "same.epub"))
                .await
                .unwrap_err();
            assert!(matches!(err, DbError::Constraint { .. }));
        }

        #[tokio::test]
        async fn boundary_user_isolation_for_work_author_and_library_item_queries() {
            // Satisfies: AUTH-003
            // IR: WorkDb::{get_work,list_works,find_by_normalized_match}; AuthorDb::{get_author,list_authors,find_author_by_name}; LibraryItemDb::{get_library_item,list_library_items,list_library_items_by_work,list_taggable_items_by_work}
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, u2) = h.user_ids();

            let w = db
                .create_work(make_work_req(u1, "Secret Title", "Secret Author"))
                .await
                .unwrap();
            let a = db
                .create_author(make_author_req(u1, "Secret Person"))
                .await
                .unwrap();
            let root = make_root_folder(db, "/iso", MediaType::Ebook).await;
            let li = db
                .create_library_item(make_library_item_req(u1, w.id, root.id, "secret.epub"))
                .await
                .unwrap();

            assert!(matches!(
                db.get_work(u2, w.id).await,
                Err(DbError::NotFound)
            ));
            assert!(!db
                .list_works(u2)
                .await
                .unwrap()
                .iter()
                .any(|x| x.id == w.id));
            assert!(db
                .find_by_normalized_match(u2, "Secret Title", "Secret Author")
                .await
                .unwrap()
                .is_empty());

            assert!(matches!(
                db.get_author(u2, a.id).await,
                Err(DbError::NotFound)
            ));
            assert!(!db
                .list_authors(u2)
                .await
                .unwrap()
                .iter()
                .any(|x| x.id == a.id));
            assert!(db
                .find_author_by_name(u2, "Secret Person")
                .await
                .unwrap()
                .is_none());

            assert!(matches!(
                db.get_library_item(u2, li.id).await,
                Err(DbError::NotFound)
            ));
            assert!(!db
                .list_library_items(u2)
                .await
                .unwrap()
                .iter()
                .any(|x| x.id == li.id));
            assert!(db
                .list_library_items_by_work(u2, w.id)
                .await
                .unwrap()
                .is_empty());
            assert!(db
                .list_taggable_items_by_work(u2, w.id)
                .await
                .unwrap()
                .is_empty());
        }

        #[tokio::test]
        async fn boundary_library_item_create_is_idempotent_for_same_work_same_path() {
            // Satisfies: IMPORT-015
            // IR: LibraryItemDb::create_library_item
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work = db
                .create_work(make_work_req(u1, "Idempotent", "Author"))
                .await
                .unwrap();
            let root = make_root_folder(db, "/idem", MediaType::Ebook).await;

            let first = db
                .create_library_item(make_library_item_req(u1, work.id, root.id, "same.epub"))
                .await
                .unwrap();
            let second = db
                .create_library_item(make_library_item_req(u1, work.id, root.id, "same.epub"))
                .await
                .unwrap();

            assert_eq!(first.id, second.id);
            let listed = db.list_library_items_by_work(u1, work.id).await.unwrap();
            let count = listed.iter().filter(|i| i.path == "same.epub").count();
            assert_eq!(count, 1);
        }

        #[tokio::test]
        async fn boundary_empty_lists_and_unused_root_folder_guard() {
            // Satisfies: IMPORT-004
            // IR: WorkDb::{list_works,list_works_for_enrichment}; AuthorDb::list_authors; LibraryItemDb::{list_library_items,library_items_exist_for_root}; RootFolderDb::list_root_folders
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            assert!(db.list_works(u1).await.unwrap().is_empty());
            assert!(db.list_works_for_enrichment(u1).await.unwrap().is_empty());
            assert!(db.list_authors(u1).await.unwrap().is_empty());
            assert!(db.list_library_items(u1).await.unwrap().is_empty());
            assert!(db.list_root_folders().await.unwrap().is_empty());

            let root = make_root_folder(db, "/unused", MediaType::Ebook).await;
            assert!(!db.library_items_exist_for_root(root.id).await.unwrap());
        }

        #[tokio::test]
        async fn boundary_list_monitored_authors_scoped_to_user() {
            // Satisfies: AUTHOR-002
            // IR: AuthorDb::list_monitored_authors — user-scoped, filters monitored + ol_key
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, u2) = h.user_ids();

            // a1: u1, monitored=true, ol_key set — should appear for u1
            let a1 = db
                .create_author(CreateAuthorDbRequest {
                    user_id: u1,
                    name: "Author One".into(),
                    sort_name: Some("Author One".into()),
                    ol_key: Some("OL1A".into()),
                })
                .await
                .unwrap();
            // a2: u2, monitored=true, ol_key set — should appear for u2, not u1
            let a2 = db
                .create_author(CreateAuthorDbRequest {
                    user_id: u2,
                    name: "Author Two".into(),
                    sort_name: Some("Author Two".into()),
                    ol_key: Some("OL2A".into()),
                })
                .await
                .unwrap();
            // a3: u1, monitored=false — should NOT appear
            let a3 = db
                .create_author(make_author_req(u1, "Author Three"))
                .await
                .unwrap();

            db.update_author(
                u1,
                a1.id,
                UpdateAuthorDbRequest {
                    name: None,
                    sort_name: None,
                    ol_key: None,
                    monitored: Some(true),
                    monitor_new_items: None,
                    monitor_since: None,
                },
            )
            .await
            .unwrap();
            db.update_author(
                u2,
                a2.id,
                UpdateAuthorDbRequest {
                    name: None,
                    sort_name: None,
                    ol_key: None,
                    monitored: Some(true),
                    monitor_new_items: None,
                    monitor_since: None,
                },
            )
            .await
            .unwrap();

            // u1 sees only their own monitored author with ol_key
            let monitored_u1 = db.list_monitored_authors(u1).await.unwrap();
            assert!(monitored_u1.iter().any(|a| a.id == a1.id));
            assert!(!monitored_u1.iter().any(|a| a.id == a2.id), "u1 should not see u2's authors");
            assert!(!monitored_u1.iter().any(|a| a.id == a3.id), "a3 not monitored");

            // u2 sees only their own
            let monitored_u2 = db.list_monitored_authors(u2).await.unwrap();
            assert!(monitored_u2.iter().any(|a| a.id == a2.id));
            assert!(!monitored_u2.iter().any(|a| a.id == a1.id), "u2 should not see u1's authors");
        }

        #[tokio::test]
        async fn failure_cannot_delete_root_folder_with_referencing_library_items() {
            // Satisfies: IMPORT-004
            // IR: RootFolderDb::delete_root_folder; LibraryItemDb::library_items_exist_for_root
            let h = <$harness as DbTestHarness>::setup().await;
            let db = h.db();
            let (u1, _) = h.user_ids();

            let work = db
                .create_work(make_work_req(u1, "Delete Guard", "Author"))
                .await
                .unwrap();
            let root = make_root_folder(db, "/guarded", MediaType::Ebook).await;
            db.create_library_item(make_library_item_req(u1, work.id, root.id, "guarded.epub"))
                .await
                .unwrap();

            assert!(db.library_items_exist_for_root(root.id).await.unwrap());
            let err = db.delete_root_folder(root.id).await.unwrap_err();
            assert!(matches!(err, DbError::Constraint { .. }));
        }
    };
}

// =============================================================================
// Phase 2: Real in-memory DB harness
// =============================================================================

struct RealHarness {
    db: librarr_db::mem::InMemoryDb,
    u1: UserId,
    u2: UserId,
}

#[async_trait]
impl DbTestHarness for RealHarness {
    type Db = librarr_db::mem::InMemoryDb;

    async fn setup() -> Self {
        let db = librarr_db::mem::InMemoryDb::new();
        let u1 = db
            .create_user(librarr_db::CreateUserDbRequest {
                username: "testuser1".to_string(),
                password_hash: "hash1".to_string(),
                role: librarr_domain::UserRole::Admin,
                api_key_hash: "apikey1".to_string(),
            })
            .await
            .unwrap();
        let u2 = db
            .create_user(librarr_db::CreateUserDbRequest {
                username: "testuser2".to_string(),
                password_hash: "hash2".to_string(),
                role: librarr_domain::UserRole::User,
                api_key_hash: "apikey2".to_string(),
            })
            .await
            .unwrap();
        RealHarness {
            db,
            u1: u1.id,
            u2: u2.id,
        }
    }

    fn db(&self) -> &Self::Db {
        &self.db
    }

    fn user_ids(&self) -> (UserId, UserId) {
        (self.u1, self.u2)
    }
}

db_core_tests!(RealHarness);
