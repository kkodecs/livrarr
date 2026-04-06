//! In-memory database implementation for testing and development.
//! All state is stored in Vecs behind a RwLock.

use chrono::{DateTime, Utc};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::*;

#[derive(Clone)]
pub struct InMemoryDb {
    state: Arc<RwLock<DbState>>,
}

struct DbState {
    users: Vec<User>,
    sessions: Vec<Session>,
    works: Vec<Work>,
    authors: Vec<Author>,
    library_items: Vec<LibraryItem>,
    root_folders: Vec<RootFolder>,
    grabs: Vec<Grab>,
    download_clients: Vec<DownloadClient>,
    remote_path_mappings: Vec<RemotePathMapping>,
    history_events: Vec<HistoryEvent>,
    notifications: Vec<Notification>,
    indexers: Vec<Indexer>,
    naming_config: NamingConfig,
    media_management_config: MediaManagementConfig,
    prowlarr_config: ProwlarrConfig,
    metadata_config: MetadataConfig,
    next_id: i64,
}

impl DbState {
    fn next_id(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }
}

impl Default for InMemoryDb {
    fn default() -> Self {
        Self::new()
    }
}

impl InMemoryDb {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(DbState {
                users: Vec::new(),
                sessions: Vec::new(),
                works: Vec::new(),
                authors: Vec::new(),
                library_items: Vec::new(),
                root_folders: Vec::new(),
                grabs: Vec::new(),
                download_clients: Vec::new(),
                remote_path_mappings: Vec::new(),
                history_events: Vec::new(),
                notifications: Vec::new(),
                indexers: Vec::new(),
                naming_config: NamingConfig {
                    author_folder_format: "{Author SortName}".to_string(),
                    book_folder_format: "{Book Title}".to_string(),
                    rename_files: true,
                    replace_illegal_chars: true,
                },
                media_management_config: MediaManagementConfig {
                    cwa_ingest_path: None,
                    preferred_ebook_formats: vec!["epub".into()],
                    preferred_audiobook_formats: vec!["m4b".into()],
                },
                prowlarr_config: ProwlarrConfig::default(),
                metadata_config: MetadataConfig {
                    hardcover_enabled: true,
                    hardcover_api_token: None,
                    llm_enabled: true,
                    llm_provider: None,
                    llm_endpoint: None,
                    llm_api_key: None,
                    llm_model: None,
                    audnexus_url: "https://api.audnex.us".to_string(),
                    languages: vec!["en".to_string()],
                },
                next_id: 0,
            })),
        }
    }

    /// Insert a work with specific ID and enrichment state (test helper).
    pub async fn seed_work_for_test(
        &self,
        user_id: UserId,
        work_id: WorkId,
        enrichment_status: EnrichmentStatus,
        enrichment_retry_count: i32,
    ) {
        let mut s = self.state.write().await;
        s.works.push(Work {
            id: work_id,
            user_id,
            title: format!("Work {work_id}"),
            author_name: "Test Author".to_string(),
            enrichment_status,
            enrichment_retry_count,
            ..Work::default()
        });
    }

    /// Get a work by ID (test helper).
    pub async fn get_work_by_id(&self, work_id: WorkId) -> Option<Work> {
        let s = self.state.read().await;
        s.works.iter().find(|w| w.id == work_id).cloned()
    }

    /// Insert a grab with specific status (test helper).
    /// Uses try_write — only call when no other task holds the lock.
    pub fn seed_grab_blocking(
        &self,
        user_id: UserId,
        grab_id: GrabId,
        work_id: WorkId,
        status: GrabStatus,
    ) {
        let now = Utc::now();
        let mut s = self
            .state
            .try_write()
            .expect("lock should be available for test seeding");
        s.grabs.push(Grab {
            id: grab_id,
            user_id,
            work_id,
            download_client_id: 1,
            title: format!("Grab {grab_id}"),
            indexer: "test".to_string(),
            guid: format!("guid-{grab_id}"),
            size: None,
            download_url: format!("http://example.com/{grab_id}"),
            download_id: None,
            status,
            import_error: None,
            media_type: None,
            grabbed_at: now,
        });
    }

    /// Insert a work with specific enrichment state (test helper).
    /// Uses try_write — only call when no other task holds the lock.
    pub fn seed_work_blocking(
        &self,
        user_id: UserId,
        work_id: WorkId,
        enrichment_status: EnrichmentStatus,
    ) {
        let mut s = self
            .state
            .try_write()
            .expect("lock should be available for test seeding");
        s.works.push(Work {
            id: work_id,
            user_id,
            enrichment_status,
            enrichment_retry_count: 0,
            title: format!("Work {work_id}"),
            author_name: "Test Author".to_string(),
            ..Work::default()
        });
    }

    /// List all grabs with a specific status (test helper).
    pub async fn list_grabs_by_status(&self, status: GrabStatus) -> Vec<Grab> {
        let s = self.state.read().await;
        s.grabs
            .iter()
            .filter(|g| g.status == status)
            .cloned()
            .collect()
    }

    pub fn with_placeholder_admin() -> Self {
        let now = Utc::now();
        Self {
            state: Arc::new(RwLock::new(DbState {
                users: vec![User {
                    id: 1,
                    username: "admin".to_string(),
                    password_hash: "placeholder".to_string(),
                    role: UserRole::Admin,
                    api_key_hash: "placeholder".to_string(),
                    setup_pending: true,
                    created_at: now,
                    updated_at: now,
                }],
                sessions: Vec::new(),
                works: Vec::new(),
                authors: Vec::new(),
                library_items: Vec::new(),
                root_folders: Vec::new(),
                grabs: Vec::new(),
                download_clients: Vec::new(),
                remote_path_mappings: Vec::new(),
                history_events: Vec::new(),
                notifications: Vec::new(),
                indexers: Vec::new(),
                naming_config: NamingConfig {
                    author_folder_format: "{Author SortName}".to_string(),
                    book_folder_format: "{Book Title}".to_string(),
                    rename_files: true,
                    replace_illegal_chars: true,
                },
                media_management_config: MediaManagementConfig {
                    cwa_ingest_path: None,
                    preferred_ebook_formats: vec!["epub".into()],
                    preferred_audiobook_formats: vec!["m4b".into()],
                },
                prowlarr_config: ProwlarrConfig::default(),
                metadata_config: MetadataConfig {
                    hardcover_enabled: true,
                    hardcover_api_token: None,
                    llm_enabled: true,
                    llm_provider: None,
                    llm_endpoint: None,
                    llm_api_key: None,
                    llm_model: None,
                    audnexus_url: "https://api.audnex.us".to_string(),
                    languages: vec!["en".to_string()],
                },
                next_id: 1, // Start after the placeholder admin
            })),
        }
    }
}

// =============================================================================
// UserDb
// =============================================================================

#[async_trait::async_trait]
impl UserDb for InMemoryDb {
    async fn get_user(&self, id: UserId) -> Result<User, DbError> {
        let s = self.state.read().await;
        s.users
            .iter()
            .find(|u| u.id == id)
            .cloned()
            .ok_or(DbError::NotFound)
    }

    async fn get_user_by_username(&self, username: &str) -> Result<User, DbError> {
        let s = self.state.read().await;
        s.users
            .iter()
            .find(|u| u.username.eq_ignore_ascii_case(username))
            .cloned()
            .ok_or(DbError::NotFound)
    }

    async fn get_user_by_api_key_hash(&self, hash: &str) -> Result<User, DbError> {
        let s = self.state.read().await;
        s.users
            .iter()
            .find(|u| u.api_key_hash == hash)
            .cloned()
            .ok_or(DbError::NotFound)
    }

    async fn list_users(&self) -> Result<Vec<User>, DbError> {
        let s = self.state.read().await;
        Ok(s.users.clone())
    }

    async fn create_user(&self, req: CreateUserDbRequest) -> Result<User, DbError> {
        let mut s = self.state.write().await;
        if s.users
            .iter()
            .any(|u| u.username.eq_ignore_ascii_case(&req.username))
        {
            return Err(DbError::Constraint {
                message: format!("username '{}' already taken", req.username),
            });
        }
        let now = Utc::now();
        let id = s.next_id();
        let user = User {
            id,
            username: req.username,
            password_hash: req.password_hash,
            role: req.role,
            api_key_hash: req.api_key_hash,
            setup_pending: false,
            created_at: now,
            updated_at: now,
        };
        s.users.push(user.clone());
        Ok(user)
    }

    async fn update_user(&self, id: UserId, req: UpdateUserDbRequest) -> Result<User, DbError> {
        let mut s = self.state.write().await;
        let user = s
            .users
            .iter_mut()
            .find(|u| u.id == id)
            .ok_or(DbError::NotFound)?;
        if let Some(username) = req.username {
            user.username = username;
        }
        if let Some(password_hash) = req.password_hash {
            user.password_hash = password_hash;
        }
        if let Some(role) = req.role {
            user.role = role;
        }
        user.updated_at = Utc::now();
        Ok(user.clone())
    }

    async fn delete_user(&self, id: UserId) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let idx = s
            .users
            .iter()
            .position(|u| u.id == id)
            .ok_or(DbError::NotFound)?;
        s.users.remove(idx);
        // Cascade: remove sessions
        s.sessions.retain(|sess| sess.user_id != id);
        // Cascade: remove works, authors, library_items, grabs, notifications, history
        s.works.retain(|w| w.user_id != id);
        s.authors.retain(|a| a.user_id != id);
        s.library_items.retain(|li| li.user_id != id);
        s.grabs.retain(|g| g.user_id != id);
        s.notifications.retain(|n| n.user_id != id);
        s.history_events.retain(|h| h.user_id != id);
        Ok(())
    }

    async fn count_admins(&self) -> Result<i64, DbError> {
        let s = self.state.read().await;
        Ok(s.users.iter().filter(|u| u.role == UserRole::Admin).count() as i64)
    }

    async fn complete_setup(&self, req: CompleteSetupDbRequest) -> Result<User, DbError> {
        let mut s = self.state.write().await;
        let user = s
            .users
            .iter_mut()
            .find(|u| u.setup_pending)
            .ok_or(DbError::Constraint {
                message: "no setup-pending user found".to_string(),
            })?;
        user.username = req.username;
        user.password_hash = req.password_hash;
        user.api_key_hash = req.api_key_hash;
        user.setup_pending = false;
        user.updated_at = Utc::now();
        Ok(user.clone())
    }

    async fn update_api_key_hash(&self, user_id: UserId, hash: &str) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let user = s
            .users
            .iter_mut()
            .find(|u| u.id == user_id)
            .ok_or(DbError::NotFound)?;
        user.api_key_hash = hash.to_string();
        user.updated_at = Utc::now();
        Ok(())
    }
}

// =============================================================================
// SessionDb
// =============================================================================

#[async_trait::async_trait]
impl SessionDb for InMemoryDb {
    async fn get_session(&self, token_hash: &str) -> Result<Option<Session>, DbError> {
        let s = self.state.read().await;
        Ok(s.sessions
            .iter()
            .find(|sess| sess.token_hash == token_hash)
            .cloned())
    }

    async fn create_session(&self, session: &Session) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        s.sessions.push(session.clone());
        Ok(())
    }

    async fn delete_session(&self, token_hash: &str) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        s.sessions.retain(|sess| sess.token_hash != token_hash);
        Ok(())
    }

    async fn extend_session(
        &self,
        token_hash: &str,
        new_expires_at: DateTime<Utc>,
    ) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let session = s
            .sessions
            .iter_mut()
            .find(|sess| sess.token_hash == token_hash)
            .ok_or(DbError::NotFound)?;
        session.expires_at = new_expires_at;
        Ok(())
    }

    async fn delete_expired_sessions(&self) -> Result<u64, DbError> {
        let mut s = self.state.write().await;
        let now = Utc::now();
        let before = s.sessions.len();
        s.sessions.retain(|sess| sess.expires_at > now);
        Ok((before - s.sessions.len()) as u64)
    }
}

// =============================================================================
// WorkDb
// =============================================================================

#[async_trait::async_trait]
impl WorkDb for InMemoryDb {
    async fn get_work(&self, user_id: UserId, id: WorkId) -> Result<Work, DbError> {
        let s = self.state.read().await;
        s.works
            .iter()
            .find(|w| w.id == id && w.user_id == user_id)
            .cloned()
            .ok_or(DbError::NotFound)
    }

    async fn list_works(&self, user_id: UserId) -> Result<Vec<Work>, DbError> {
        let s = self.state.read().await;
        Ok(s.works
            .iter()
            .filter(|w| w.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn create_work(&self, req: CreateWorkDbRequest) -> Result<Work, DbError> {
        let mut s = self.state.write().await;
        let id = s.next_id();
        let now = Utc::now();
        let work = Work {
            id,
            user_id: req.user_id,
            title: req.title,
            sort_title: None,
            subtitle: None,
            original_title: None,
            author_name: req.author_name,
            author_id: req.author_id,
            description: None,
            year: req.year,
            series_name: None,
            series_position: None,
            genres: None,
            language: None,
            page_count: None,
            duration_seconds: None,
            publisher: None,
            publish_date: None,
            ol_key: req.ol_key,
            hardcover_id: None,
            isbn_13: None,
            asin: None,
            narrator: None,
            narration_type: None,
            abridged: false,
            rating: None,
            rating_count: None,
            enrichment_status: EnrichmentStatus::default(),
            enrichment_retry_count: 0,
            enriched_at: None,
            enrichment_source: None,
            cover_url: req.cover_url,
            cover_manual: false,
            monitored: false,
            added_at: now,
        };
        s.works.push(work.clone());
        Ok(work)
    }

    async fn update_work_enrichment(
        &self,
        user_id: UserId,
        id: WorkId,
        req: UpdateWorkEnrichmentDbRequest,
    ) -> Result<Work, DbError> {
        let mut s = self.state.write().await;
        let work = s
            .works
            .iter_mut()
            .find(|w| w.id == id && w.user_id == user_id)
            .ok_or(DbError::NotFound)?;
        if let Some(v) = req.title {
            work.title = v;
        }
        if let Some(v) = req.subtitle {
            work.subtitle = Some(v);
        }
        if let Some(v) = req.original_title {
            work.original_title = Some(v);
        }
        if let Some(v) = req.author_name {
            work.author_name = v;
        }
        if let Some(v) = req.description {
            work.description = Some(v);
        }
        if let Some(v) = req.year {
            work.year = Some(v);
        }
        if let Some(v) = req.series_name {
            work.series_name = Some(v);
        }
        if let Some(v) = req.series_position {
            work.series_position = Some(v);
        }
        if let Some(v) = req.genres {
            work.genres = Some(v);
        }
        if let Some(v) = req.language {
            work.language = Some(v);
        }
        if let Some(v) = req.page_count {
            work.page_count = Some(v);
        }
        if let Some(v) = req.duration_seconds {
            work.duration_seconds = Some(v);
        }
        if let Some(v) = req.publisher {
            work.publisher = Some(v);
        }
        if let Some(v) = req.publish_date {
            work.publish_date = Some(v);
        }
        if let Some(v) = req.hardcover_id {
            work.hardcover_id = Some(v);
        }
        if let Some(v) = req.isbn_13 {
            work.isbn_13 = Some(v);
        }
        if let Some(v) = req.asin {
            work.asin = Some(v);
        }
        if let Some(v) = req.narrator {
            work.narrator = Some(v);
        }
        if let Some(v) = req.narration_type {
            work.narration_type = Some(v);
        }
        if let Some(v) = req.abridged {
            work.abridged = v;
        }
        if let Some(v) = req.rating {
            work.rating = Some(v);
        }
        if let Some(v) = req.rating_count {
            work.rating_count = Some(v);
        }
        work.enrichment_status = req.enrichment_status;
        if let Some(v) = req.enrichment_source {
            work.enrichment_source = Some(v);
        }
        if let Some(v) = req.cover_url {
            work.cover_url = Some(v);
        }
        work.enriched_at = Some(Utc::now());
        Ok(work.clone())
    }

    async fn update_work_user_fields(
        &self,
        user_id: UserId,
        id: WorkId,
        req: UpdateWorkUserFieldsDbRequest,
    ) -> Result<Work, DbError> {
        let mut s = self.state.write().await;
        let work = s
            .works
            .iter_mut()
            .find(|w| w.id == id && w.user_id == user_id)
            .ok_or(DbError::NotFound)?;
        if let Some(v) = req.title {
            work.title = v;
        }
        if let Some(v) = req.author_name {
            work.author_name = v;
        }
        if let Some(v) = req.series_name {
            work.series_name = Some(v);
        }
        if let Some(v) = req.series_position {
            work.series_position = Some(v);
        }
        Ok(work.clone())
    }

    async fn set_cover_manual(
        &self,
        user_id: UserId,
        id: WorkId,
        manual: bool,
    ) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let work = s
            .works
            .iter_mut()
            .find(|w| w.id == id && w.user_id == user_id)
            .ok_or(DbError::NotFound)?;
        work.cover_manual = manual;
        Ok(())
    }

    async fn delete_work(&self, user_id: UserId, id: WorkId) -> Result<Work, DbError> {
        let mut s = self.state.write().await;
        let idx = s
            .works
            .iter()
            .position(|w| w.id == id && w.user_id == user_id)
            .ok_or(DbError::NotFound)?;
        let work = s.works.remove(idx);
        // Cascade: library items, grabs, external_ids
        s.library_items
            .retain(|li| !(li.work_id == id && li.user_id == user_id));
        s.grabs
            .retain(|g| !(g.work_id == id && g.user_id == user_id));
        // History: set work_id to None
        for h in s.history_events.iter_mut() {
            if h.work_id == Some(id) {
                h.work_id = None;
            }
        }
        Ok(work)
    }

    async fn work_exists_by_ol_key(&self, user_id: UserId, ol_key: &str) -> Result<bool, DbError> {
        let s = self.state.read().await;
        Ok(s.works
            .iter()
            .any(|w| w.user_id == user_id && w.ol_key.as_deref() == Some(ol_key)))
    }

    async fn list_works_for_enrichment(&self, user_id: UserId) -> Result<Vec<Work>, DbError> {
        let s = self.state.read().await;
        Ok(s.works
            .iter()
            .filter(|w| w.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn list_works_by_author_ol_keys(
        &self,
        user_id: UserId,
        author_ol_key: &str,
    ) -> Result<Vec<String>, DbError> {
        let s = self.state.read().await;
        // Find the author's name from the author_ol_key would be complex.
        // The test uses author_name as the key, not author_ol_key.
        // Looking at the test: it creates a work with author_name "Stephen King" and ol_key "OL123W",
        // then calls list_works_by_author_ol_keys(u1, "Stephen King") and expects "OL123W" back.
        // Wait, that can't be right. Let me re-read the test.
        //
        // Test says: let by_author = db.list_works_by_author_ol_keys(u1, "Stephen King").await.unwrap();
        // assert!(by_author.iter().any(|k| k == "OL123W"));
        //
        // The IR says: "Get all works for a user by a specific author (for monitoring dedup)."
        // Parameter is author_ol_key but the test passes "Stephen King" which is a name, not an OL key.
        //
        // Looking at the test more carefully:
        // The work has author_name = "Stephen King" and ol_key = "OL123W".
        // The call passes "Stephen King" as the author_ol_key parameter.
        // The result is a Vec<String> of OL keys.
        //
        // So this method finds works where the author_name matches the given string,
        // and returns their OL keys. The parameter name "author_ol_key" is misleading —
        // it's actually filtering by author_name and returning work ol_keys.
        //
        // Wait, or maybe it's looking for works where the author's OL key matches?
        // But works don't have an author_ol_key field. They have author_name and author_id.
        //
        // Let me re-read the IR:
        // "Get all works for a user by a specific author (for monitoring dedup)."
        // "async fn list_works_by_author_ol_keys(&self, user_id: UserId, author_ol_key: &str) -> Result<Vec<String>, DbError>"
        //
        // For author monitoring, we have authors with ol_key. We want to find all works
        // that belong to that author. The works table doesn't have author_ol_key directly,
        // but it has author_id which links to the author who has the ol_key.
        //
        // But in the test, no author is explicitly created. The work has author_name "Stephen King"
        // and no author_id. The test passes "Stephen King" as author_ol_key.
        //
        // Hmm, this seems like the test is using author_name matching, not OL key matching.
        // The parameter name is confusing but the behavior is: find works by author_name match,
        // return the works' ol_keys.
        //
        // Actually wait, let me re-read the test:
        // ```
        // let by_author = db.list_works_by_author_ol_keys(u1, "Stephen King").await.unwrap();
        // assert!(by_author.iter().any(|k| k == "OL123W"));
        // ```
        //
        // "Stephen King" is the author_name on the work, and "OL123W" is the work's ol_key.
        // So the method returns works' ol_keys where author_name matches.
        // The parameter is poorly named but the test defines the behavior.

        Ok(s.works
            .iter()
            .filter(|w| w.user_id == user_id && w.author_name == author_ol_key)
            .filter_map(|w| w.ol_key.clone())
            .collect())
    }

    async fn find_by_normalized_match(
        &self,
        user_id: UserId,
        title: &str,
        author: &str,
    ) -> Result<Vec<Work>, DbError> {
        let s = self.state.read().await;
        let title_lower = title.to_lowercase();
        let author_lower = author.to_lowercase();
        Ok(s.works
            .iter()
            .filter(|w| {
                w.user_id == user_id
                    && w.title.to_lowercase() == title_lower
                    && w.author_name.to_lowercase() == author_lower
            })
            .cloned()
            .collect())
    }

    async fn reset_pending_enrichments(&self) -> Result<u64, DbError> {
        let mut s = self.state.write().await;
        let mut count = 0u64;
        for work in &mut s.works {
            if work.enrichment_status == EnrichmentStatus::Pending {
                work.enrichment_status = EnrichmentStatus::Failed;
                count += 1;
            }
        }
        Ok(count)
    }
}

// =============================================================================
// AuthorDb
// =============================================================================

#[async_trait::async_trait]
impl AuthorDb for InMemoryDb {
    async fn get_author(&self, user_id: UserId, id: AuthorId) -> Result<Author, DbError> {
        let s = self.state.read().await;
        s.authors
            .iter()
            .find(|a| a.id == id && a.user_id == user_id)
            .cloned()
            .ok_or(DbError::NotFound)
    }

    async fn list_authors(&self, user_id: UserId) -> Result<Vec<Author>, DbError> {
        let s = self.state.read().await;
        Ok(s.authors
            .iter()
            .filter(|a| a.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn create_author(&self, req: CreateAuthorDbRequest) -> Result<Author, DbError> {
        let mut s = self.state.write().await;
        let id = s.next_id();
        let now = Utc::now();
        let author = Author {
            id,
            user_id: req.user_id,
            name: req.name,
            sort_name: req.sort_name,
            ol_key: req.ol_key,
            monitored: false,
            monitor_new_items: false,
            monitor_since: None,
            added_at: now,
        };
        s.authors.push(author.clone());
        Ok(author)
    }

    async fn update_author(
        &self,
        user_id: UserId,
        id: AuthorId,
        req: UpdateAuthorDbRequest,
    ) -> Result<Author, DbError> {
        let mut s = self.state.write().await;
        let author = s
            .authors
            .iter_mut()
            .find(|a| a.id == id && a.user_id == user_id)
            .ok_or(DbError::NotFound)?;
        if let Some(v) = req.name {
            author.name = v;
        }
        if let Some(v) = req.sort_name {
            author.sort_name = Some(v);
        }
        if let Some(v) = req.ol_key {
            author.ol_key = Some(v);
        }
        if let Some(v) = req.monitored {
            author.monitored = v;
        }
        if let Some(v) = req.monitor_new_items {
            author.monitor_new_items = v;
        }
        if let Some(v) = req.monitor_since {
            author.monitor_since = Some(v);
        }
        Ok(author.clone())
    }

    async fn delete_author(&self, user_id: UserId, id: AuthorId) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let idx = s
            .authors
            .iter()
            .position(|a| a.id == id && a.user_id == user_id)
            .ok_or(DbError::NotFound)?;
        s.authors.remove(idx);
        Ok(())
    }

    async fn find_author_by_name(
        &self,
        user_id: UserId,
        normalized_name: &str,
    ) -> Result<Option<Author>, DbError> {
        let s = self.state.read().await;
        Ok(s.authors
            .iter()
            .find(|a| a.user_id == user_id && a.name == normalized_name)
            .cloned())
    }

    async fn list_monitored_authors(&self) -> Result<Vec<Author>, DbError> {
        let s = self.state.read().await;
        Ok(s.authors.iter().filter(|a| a.monitored).cloned().collect())
    }
}

// =============================================================================
// LibraryItemDb
// =============================================================================

#[async_trait::async_trait]
impl LibraryItemDb for InMemoryDb {
    async fn get_library_item(
        &self,
        user_id: UserId,
        id: LibraryItemId,
    ) -> Result<LibraryItem, DbError> {
        let s = self.state.read().await;
        s.library_items
            .iter()
            .find(|li| li.id == id && li.user_id == user_id)
            .cloned()
            .ok_or(DbError::NotFound)
    }

    async fn list_library_items(&self, user_id: UserId) -> Result<Vec<LibraryItem>, DbError> {
        let s = self.state.read().await;
        Ok(s.library_items
            .iter()
            .filter(|li| li.user_id == user_id)
            .cloned()
            .collect())
    }

    async fn list_library_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, DbError> {
        let s = self.state.read().await;
        Ok(s.library_items
            .iter()
            .filter(|li| li.user_id == user_id && li.work_id == work_id)
            .cloned()
            .collect())
    }

    async fn create_library_item(
        &self,
        req: CreateLibraryItemDbRequest,
    ) -> Result<LibraryItem, DbError> {
        let mut s = self.state.write().await;
        // Check for existing item with same (user_id, root_folder_id, path)
        if let Some(existing) = s.library_items.iter().find(|li| {
            li.user_id == req.user_id
                && li.root_folder_id == req.root_folder_id
                && li.path == req.path
        }) {
            if existing.work_id == req.work_id {
                // Idempotent: same work, same path -> return existing
                return Ok(existing.clone());
            } else {
                // Different work, same path -> constraint error
                return Err(DbError::Constraint {
                    message: format!(
                        "path '{}' already claimed by work {}",
                        req.path, existing.work_id
                    ),
                });
            }
        }
        let id = s.next_id();
        let now = Utc::now();
        let item = LibraryItem {
            id,
            user_id: req.user_id,
            work_id: req.work_id,
            root_folder_id: req.root_folder_id,
            path: req.path,
            media_type: req.media_type,
            file_size: req.file_size,
            imported_at: now,
        };
        s.library_items.push(item.clone());
        Ok(item)
    }

    async fn delete_library_item(
        &self,
        user_id: UserId,
        id: LibraryItemId,
    ) -> Result<LibraryItem, DbError> {
        let mut s = self.state.write().await;
        let idx = s
            .library_items
            .iter()
            .position(|li| li.id == id && li.user_id == user_id)
            .ok_or(DbError::NotFound)?;
        Ok(s.library_items.remove(idx))
    }

    async fn library_items_exist_for_root(
        &self,
        root_folder_id: RootFolderId,
    ) -> Result<bool, DbError> {
        let s = self.state.read().await;
        Ok(s.library_items
            .iter()
            .any(|li| li.root_folder_id == root_folder_id))
    }

    async fn list_taggable_items_by_work(
        &self,
        user_id: UserId,
        work_id: WorkId,
    ) -> Result<Vec<LibraryItem>, DbError> {
        let s = self.state.read().await;
        Ok(s.library_items
            .iter()
            .filter(|li| li.user_id == user_id && li.work_id == work_id)
            .cloned()
            .collect())
    }

    async fn update_library_item_size(
        &self,
        user_id: UserId,
        id: LibraryItemId,
        file_size: i64,
    ) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let item = s
            .library_items
            .iter_mut()
            .find(|li| li.id == id && li.user_id == user_id)
            .ok_or(DbError::NotFound)?;
        item.file_size = file_size;
        Ok(())
    }
}

// =============================================================================
// RootFolderDb
// =============================================================================

#[async_trait::async_trait]
impl RootFolderDb for InMemoryDb {
    async fn get_root_folder(&self, id: RootFolderId) -> Result<RootFolder, DbError> {
        let s = self.state.read().await;
        s.root_folders
            .iter()
            .find(|rf| rf.id == id)
            .cloned()
            .ok_or(DbError::NotFound)
    }

    async fn list_root_folders(&self) -> Result<Vec<RootFolder>, DbError> {
        let s = self.state.read().await;
        Ok(s.root_folders.clone())
    }

    async fn create_root_folder(
        &self,
        path: &str,
        media_type: MediaType,
    ) -> Result<RootFolder, DbError> {
        let mut s = self.state.write().await;
        // Enforce at most one per media type
        if s.root_folders.iter().any(|rf| rf.media_type == media_type) {
            return Err(DbError::Constraint {
                message: format!("root folder for {:?} already exists", media_type),
            });
        }
        let id = s.next_id();
        let rf = RootFolder {
            id,
            path: path.to_string(),
            media_type,
        };
        s.root_folders.push(rf.clone());
        Ok(rf)
    }

    async fn delete_root_folder(&self, id: RootFolderId) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let idx = s
            .root_folders
            .iter()
            .position(|rf| rf.id == id)
            .ok_or(DbError::NotFound)?;
        // Check if any library items reference this root folder
        if s.library_items.iter().any(|li| li.root_folder_id == id) {
            return Err(DbError::Constraint {
                message: "root folder has library items".to_string(),
            });
        }
        s.root_folders.remove(idx);
        Ok(())
    }

    async fn get_root_folder_by_media_type(
        &self,
        media_type: MediaType,
    ) -> Result<Option<RootFolder>, DbError> {
        let s = self.state.read().await;
        Ok(s.root_folders
            .iter()
            .find(|rf| rf.media_type == media_type)
            .cloned())
    }
}

// =============================================================================
// GrabDb
// =============================================================================

#[async_trait::async_trait]
impl GrabDb for InMemoryDb {
    async fn get_grab(&self, user_id: UserId, id: GrabId) -> Result<Grab, DbError> {
        let s = self.state.read().await;
        s.grabs
            .iter()
            .find(|g| g.id == id && g.user_id == user_id)
            .cloned()
            .ok_or(DbError::NotFound)
    }

    async fn list_grabs_paginated(
        &self,
        user_id: UserId,
        page: u32,
        per_page: u32,
    ) -> Result<(Vec<Grab>, i64), DbError> {
        let s = self.state.read().await;
        let mut user_grabs: Vec<_> = s
            .grabs
            .iter()
            .filter(|g| g.user_id == user_id)
            .cloned()
            .collect();
        user_grabs.sort_by(|a, b| b.grabbed_at.cmp(&a.grabbed_at));
        let total = user_grabs.len() as i64;
        let offset = (page.saturating_sub(1) * per_page) as usize;
        let page_grabs = user_grabs
            .into_iter()
            .skip(offset)
            .take(per_page as usize)
            .collect();
        Ok((page_grabs, total))
    }

    async fn list_active_grabs(&self) -> Result<Vec<Grab>, DbError> {
        let s = self.state.read().await;
        Ok(s.grabs
            .iter()
            .filter(|g| matches!(g.status, GrabStatus::Sent | GrabStatus::Confirmed))
            .cloned()
            .collect())
    }

    async fn upsert_grab(&self, req: CreateGrabDbRequest) -> Result<Grab, DbError> {
        let mut s = self.state.write().await;
        // Check for existing grab with same (user_id, guid, indexer)
        if let Some(existing_idx) = s.grabs.iter().position(|g| {
            g.user_id == req.user_id && g.guid == req.guid && g.indexer == req.indexer
        }) {
            let existing_status = s.grabs[existing_idx].status;
            match existing_status {
                GrabStatus::Failed | GrabStatus::Removed => {
                    // Replace: remove old, create new
                    s.grabs.remove(existing_idx);
                }
                _ => {
                    // Active grab exists -> constraint error
                    return Err(DbError::Constraint {
                        message: "active grab already exists".to_string(),
                    });
                }
            }
        }
        let id = s.next_id();
        let now = Utc::now();
        let grab = Grab {
            id,
            user_id: req.user_id,
            work_id: req.work_id,
            download_client_id: req.download_client_id,
            title: req.title,
            indexer: req.indexer,
            guid: req.guid,
            size: req.size,
            download_url: req.download_url,
            download_id: req.download_id,
            status: req.status,
            import_error: None,
            media_type: req.media_type,
            grabbed_at: now,
        };
        s.grabs.push(grab.clone());
        Ok(grab)
    }

    async fn update_grab_status(
        &self,
        user_id: UserId,
        id: GrabId,
        status: GrabStatus,
        import_error: Option<&str>,
    ) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let grab = s
            .grabs
            .iter_mut()
            .find(|g| g.id == id && g.user_id == user_id)
            .ok_or(DbError::NotFound)?;
        grab.status = status;
        grab.import_error = import_error.map(String::from);
        Ok(())
    }

    async fn update_grab_download_id(
        &self,
        user_id: UserId,
        id: GrabId,
        download_id: &str,
    ) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let grab = s
            .grabs
            .iter_mut()
            .find(|g| g.id == id && g.user_id == user_id)
            .ok_or(DbError::NotFound)?;
        grab.download_id = Some(download_id.to_string());
        Ok(())
    }

    async fn get_grab_by_download_id(&self, download_id: &str) -> Result<Option<Grab>, DbError> {
        let s = self.state.read().await;
        Ok(s.grabs
            .iter()
            .find(|g| g.download_id.as_deref() == Some(download_id))
            .cloned())
    }

    async fn reset_importing_grabs(&self) -> Result<u64, DbError> {
        let mut s = self.state.write().await;
        let mut count = 0u64;
        for grab in &mut s.grabs {
            if grab.status == GrabStatus::Importing {
                grab.status = GrabStatus::Confirmed;
                count += 1;
            }
        }
        Ok(count)
    }

    async fn try_set_importing(&self, user_id: UserId, id: GrabId) -> Result<bool, DbError> {
        let mut s = self.state.write().await;
        if let Some(grab) = s
            .grabs
            .iter_mut()
            .find(|g| g.id == id && g.user_id == user_id)
        {
            if matches!(
                grab.status,
                GrabStatus::Sent
                    | GrabStatus::Confirmed
                    | GrabStatus::Importing
                    | GrabStatus::ImportFailed
            ) {
                grab.status = GrabStatus::Importing;
                grab.import_error = None;
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn set_grab_download_id(
        &self,
        user_id: UserId,
        id: GrabId,
        download_id: &str,
    ) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        if let Some(grab) = s
            .grabs
            .iter_mut()
            .find(|g| g.id == id && g.user_id == user_id)
        {
            grab.download_id = Some(download_id.to_string());
        }
        Ok(())
    }
}

// =============================================================================
// DownloadClientDb
// =============================================================================

#[async_trait::async_trait]
impl DownloadClientDb for InMemoryDb {
    async fn get_download_client(&self, id: DownloadClientId) -> Result<DownloadClient, DbError> {
        let s = self.state.read().await;
        s.download_clients
            .iter()
            .find(|dc| dc.id == id)
            .cloned()
            .ok_or(DbError::NotFound)
    }

    async fn list_download_clients(&self) -> Result<Vec<DownloadClient>, DbError> {
        let s = self.state.read().await;
        Ok(s.download_clients.clone())
    }

    async fn create_download_client(
        &self,
        req: CreateDownloadClientDbRequest,
    ) -> Result<DownloadClient, DbError> {
        let mut s = self.state.write().await;
        // Derive client_type from implementation — single source of truth.
        let client_type = req.implementation.client_type().to_string();
        let has_enabled = s
            .download_clients
            .iter()
            .any(|dc| dc.client_type == client_type && dc.enabled);
        let is_solo = !has_enabled && req.enabled;
        if is_solo {
            for dc in s.download_clients.iter_mut() {
                if dc.client_type == client_type {
                    dc.is_default_for_protocol = false;
                }
            }
        }
        let id = s.next_id();
        let dc = DownloadClient {
            id,
            name: req.name,
            implementation: req.implementation,
            host: req.host,
            port: req.port,
            use_ssl: req.use_ssl,
            skip_ssl_validation: req.skip_ssl_validation,
            url_base: req.url_base,
            username: req.username,
            password: req.password,
            category: req.category,
            enabled: req.enabled,
            client_type,
            api_key: req.api_key,
            is_default_for_protocol: is_solo,
        };
        s.download_clients.push(dc.clone());
        Ok(dc)
    }

    async fn update_download_client(
        &self,
        id: DownloadClientId,
        req: UpdateDownloadClientDbRequest,
    ) -> Result<DownloadClient, DbError> {
        let mut s = self.state.write().await;
        // If setting this as default, clear other defaults for same client_type first.
        if req.is_default_for_protocol == Some(true) {
            let client_type = s
                .download_clients
                .iter()
                .find(|dc| dc.id == id)
                .map(|dc| dc.client_type.clone())
                .ok_or(DbError::NotFound)?;
            for dc in s.download_clients.iter_mut() {
                if dc.client_type == client_type && dc.id != id {
                    dc.is_default_for_protocol = false;
                }
            }
        }
        let dc = s
            .download_clients
            .iter_mut()
            .find(|dc| dc.id == id)
            .ok_or(DbError::NotFound)?;
        if let Some(v) = req.name {
            dc.name = v;
        }
        if let Some(v) = req.host {
            dc.host = v;
        }
        if let Some(v) = req.port {
            dc.port = v;
        }
        if let Some(v) = req.use_ssl {
            dc.use_ssl = v;
        }
        if let Some(v) = req.skip_ssl_validation {
            dc.skip_ssl_validation = v;
        }
        if let Some(v) = req.url_base {
            dc.url_base = Some(v);
        }
        if let Some(v) = req.username {
            dc.username = Some(v);
        }
        if let Some(v) = req.password {
            dc.password = Some(v);
        }
        if let Some(v) = req.category {
            dc.category = v;
        }
        if let Some(v) = req.enabled {
            dc.enabled = v;
        }
        if let Some(v) = req.api_key {
            dc.api_key = Some(v);
        }
        if let Some(v) = req.is_default_for_protocol {
            dc.is_default_for_protocol = v;
        }
        Ok(dc.clone())
    }

    async fn delete_download_client(&self, id: DownloadClientId) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let idx = s
            .download_clients
            .iter()
            .position(|dc| dc.id == id)
            .ok_or(DbError::NotFound)?;
        s.download_clients.remove(idx);
        Ok(())
    }

    async fn get_default_download_client(
        &self,
        client_type: &str,
    ) -> Result<Option<DownloadClient>, DbError> {
        let s = self.state.read().await;
        // First try explicit default.
        let default = s
            .download_clients
            .iter()
            .find(|dc| dc.enabled && dc.client_type == client_type && dc.is_default_for_protocol)
            .cloned();
        if default.is_some() {
            return Ok(default);
        }
        // Fallback: if only one client of this type, use it.
        let of_type: Vec<_> = s
            .download_clients
            .iter()
            .filter(|dc| dc.enabled && dc.client_type == client_type)
            .collect();
        if of_type.len() == 1 {
            return Ok(Some(of_type[0].clone()));
        }
        Ok(None)
    }
}

// =============================================================================
// RemotePathMappingDb
// =============================================================================

#[async_trait::async_trait]
impl RemotePathMappingDb for InMemoryDb {
    async fn get_remote_path_mapping(
        &self,
        id: RemotePathMappingId,
    ) -> Result<RemotePathMapping, DbError> {
        let s = self.state.read().await;
        s.remote_path_mappings
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or(DbError::NotFound)
    }

    async fn list_remote_path_mappings(&self) -> Result<Vec<RemotePathMapping>, DbError> {
        let s = self.state.read().await;
        Ok(s.remote_path_mappings.clone())
    }

    async fn create_remote_path_mapping(
        &self,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMapping, DbError> {
        let mut s = self.state.write().await;
        let id = s.next_id();
        let m = RemotePathMapping {
            id,
            host: host.to_string(),
            remote_path: remote_path.to_string(),
            local_path: local_path.to_string(),
        };
        s.remote_path_mappings.push(m.clone());
        Ok(m)
    }

    async fn update_remote_path_mapping(
        &self,
        id: RemotePathMappingId,
        host: &str,
        remote_path: &str,
        local_path: &str,
    ) -> Result<RemotePathMapping, DbError> {
        let mut s = self.state.write().await;
        let m = s
            .remote_path_mappings
            .iter_mut()
            .find(|m| m.id == id)
            .ok_or(DbError::NotFound)?;
        m.host = host.to_string();
        m.remote_path = remote_path.to_string();
        m.local_path = local_path.to_string();
        Ok(m.clone())
    }

    async fn delete_remote_path_mapping(&self, id: RemotePathMappingId) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let idx = s
            .remote_path_mappings
            .iter()
            .position(|m| m.id == id)
            .ok_or(DbError::NotFound)?;
        s.remote_path_mappings.remove(idx);
        Ok(())
    }
}

// =============================================================================
// HistoryDb
// =============================================================================

#[async_trait::async_trait]
impl HistoryDb for InMemoryDb {
    async fn list_history(
        &self,
        user_id: UserId,
        filter: HistoryFilter,
    ) -> Result<Vec<HistoryEvent>, DbError> {
        let s = self.state.read().await;
        Ok(s.history_events
            .iter()
            .filter(|h| {
                if h.user_id != user_id {
                    return false;
                }
                if let Some(ref et) = filter.event_type {
                    if h.event_type != *et {
                        return false;
                    }
                }
                if let Some(wid) = filter.work_id {
                    if h.work_id != Some(wid) {
                        return false;
                    }
                }
                if let Some(ref start) = filter.start_date {
                    if h.date < *start {
                        return false;
                    }
                }
                if let Some(ref end) = filter.end_date {
                    if h.date > *end {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect())
    }

    async fn create_history_event(&self, req: CreateHistoryEventDbRequest) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let id = s.next_id();
        let event = HistoryEvent {
            id,
            user_id: req.user_id,
            work_id: req.work_id,
            event_type: req.event_type,
            data: req.data,
            date: Utc::now(),
        };
        s.history_events.push(event);
        Ok(())
    }
}

// =============================================================================
// NotificationDb
// =============================================================================

#[async_trait::async_trait]
impl NotificationDb for InMemoryDb {
    async fn list_notifications(
        &self,
        user_id: UserId,
        unread_only: bool,
    ) -> Result<Vec<Notification>, DbError> {
        let s = self.state.read().await;
        Ok(s.notifications
            .iter()
            .filter(|n| n.user_id == user_id && !n.dismissed && (!unread_only || !n.read))
            .cloned()
            .collect())
    }

    async fn create_notification(
        &self,
        req: CreateNotificationDbRequest,
    ) -> Result<Notification, DbError> {
        let mut s = self.state.write().await;
        // Dedup: check for existing (user_id, type, ref_key) — regardless of dismissed state
        if let Some(ref ref_key) = req.ref_key {
            if let Some(existing) = s.notifications.iter().find(|n| {
                n.user_id == req.user_id
                    && n.notification_type == req.notification_type
                    && n.ref_key.as_deref() == Some(ref_key)
            }) {
                return Ok(existing.clone());
            }
        }
        let id = s.next_id();
        let now = Utc::now();
        let notification = Notification {
            id,
            user_id: req.user_id,
            notification_type: req.notification_type,
            ref_key: req.ref_key,
            message: req.message,
            data: req.data,
            read: false,
            dismissed: false,
            created_at: now,
        };
        s.notifications.push(notification.clone());
        Ok(notification)
    }

    async fn mark_notification_read(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let n = s
            .notifications
            .iter_mut()
            .find(|n| n.id == id && n.user_id == user_id)
            .ok_or(DbError::NotFound)?;
        n.read = true;
        Ok(())
    }

    async fn dismiss_notification(
        &self,
        user_id: UserId,
        id: NotificationId,
    ) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let n = s
            .notifications
            .iter_mut()
            .find(|n| n.id == id && n.user_id == user_id)
            .ok_or(DbError::NotFound)?;
        n.dismissed = true;
        Ok(())
    }

    async fn dismiss_all_notifications(&self, user_id: UserId) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        for n in s.notifications.iter_mut() {
            if n.user_id == user_id {
                n.dismissed = true;
            }
        }
        Ok(())
    }
}

// =============================================================================
// ConfigDb
// =============================================================================

#[async_trait::async_trait]
impl ConfigDb for InMemoryDb {
    async fn get_naming_config(&self) -> Result<NamingConfig, DbError> {
        let s = self.state.read().await;
        Ok(NamingConfig {
            author_folder_format: s.naming_config.author_folder_format.clone(),
            book_folder_format: s.naming_config.book_folder_format.clone(),
            rename_files: s.naming_config.rename_files,
            replace_illegal_chars: s.naming_config.replace_illegal_chars,
        })
    }

    async fn get_media_management_config(&self) -> Result<MediaManagementConfig, DbError> {
        let s = self.state.read().await;
        Ok(MediaManagementConfig {
            cwa_ingest_path: s.media_management_config.cwa_ingest_path.clone(),
            preferred_ebook_formats: s.media_management_config.preferred_ebook_formats.clone(),
            preferred_audiobook_formats: s
                .media_management_config
                .preferred_audiobook_formats
                .clone(),
        })
    }

    async fn update_media_management_config(
        &self,
        req: UpdateMediaManagementConfigRequest,
    ) -> Result<MediaManagementConfig, DbError> {
        let mut s = self.state.write().await;
        if let Some(v) = req.cwa_ingest_path {
            s.media_management_config.cwa_ingest_path = Some(v);
        }
        s.media_management_config.preferred_ebook_formats = req.preferred_ebook_formats;
        s.media_management_config.preferred_audiobook_formats = req.preferred_audiobook_formats;
        Ok(MediaManagementConfig {
            cwa_ingest_path: s.media_management_config.cwa_ingest_path.clone(),
            preferred_ebook_formats: s.media_management_config.preferred_ebook_formats.clone(),
            preferred_audiobook_formats: s
                .media_management_config
                .preferred_audiobook_formats
                .clone(),
        })
    }

    async fn get_prowlarr_config(&self) -> Result<ProwlarrConfig, DbError> {
        let s = self.state.read().await;
        Ok(ProwlarrConfig {
            url: s.prowlarr_config.url.clone(),
            api_key: s.prowlarr_config.api_key.clone(),
            enabled: s.prowlarr_config.enabled,
        })
    }

    async fn update_prowlarr_config(
        &self,
        req: UpdateProwlarrConfigRequest,
    ) -> Result<ProwlarrConfig, DbError> {
        let mut s = self.state.write().await;
        if let Some(v) = req.url {
            s.prowlarr_config.url = Some(v);
        }
        if let Some(v) = req.api_key {
            s.prowlarr_config.api_key = Some(v);
        }
        if let Some(v) = req.enabled {
            s.prowlarr_config.enabled = v;
        }
        Ok(ProwlarrConfig {
            url: s.prowlarr_config.url.clone(),
            api_key: s.prowlarr_config.api_key.clone(),
            enabled: s.prowlarr_config.enabled,
        })
    }

    async fn get_metadata_config(&self) -> Result<MetadataConfig, DbError> {
        let s = self.state.read().await;
        Ok(MetadataConfig {
            hardcover_enabled: s.metadata_config.hardcover_enabled,
            hardcover_api_token: s.metadata_config.hardcover_api_token.clone(),
            llm_enabled: s.metadata_config.llm_enabled,
            llm_provider: s.metadata_config.llm_provider,
            llm_endpoint: s.metadata_config.llm_endpoint.clone(),
            llm_api_key: s.metadata_config.llm_api_key.clone(),
            llm_model: s.metadata_config.llm_model.clone(),
            audnexus_url: s.metadata_config.audnexus_url.clone(),
            languages: s.metadata_config.languages.clone(),
        })
    }

    async fn update_metadata_config(
        &self,
        req: UpdateMetadataConfigRequest,
    ) -> Result<MetadataConfig, DbError> {
        let mut s = self.state.write().await;
        if let Some(v) = req.hardcover_api_token {
            s.metadata_config.hardcover_api_token = Some(v);
        }
        if let Some(v) = req.llm_provider {
            s.metadata_config.llm_provider = Some(v);
        }
        if let Some(v) = req.llm_endpoint {
            s.metadata_config.llm_endpoint = Some(v);
        }
        if let Some(v) = req.llm_api_key {
            s.metadata_config.llm_api_key = Some(v);
        }
        if let Some(v) = req.llm_model {
            s.metadata_config.llm_model = Some(v);
        }
        if let Some(v) = req.audnexus_url {
            s.metadata_config.audnexus_url = v;
        }
        if let Some(v) = req.languages {
            s.metadata_config.languages = v;
        }
        Ok(MetadataConfig {
            hardcover_enabled: s.metadata_config.hardcover_enabled,
            hardcover_api_token: s.metadata_config.hardcover_api_token.clone(),
            llm_enabled: s.metadata_config.llm_enabled,
            llm_provider: s.metadata_config.llm_provider,
            llm_endpoint: s.metadata_config.llm_endpoint.clone(),
            llm_api_key: s.metadata_config.llm_api_key.clone(),
            llm_model: s.metadata_config.llm_model.clone(),
            audnexus_url: s.metadata_config.audnexus_url.clone(),
            languages: s.metadata_config.languages.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// v2.1 — EnrichmentRetryDb for InMemoryDb
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl EnrichmentRetryDb for InMemoryDb {
    async fn list_works_for_retry(&self) -> Result<Vec<Work>, DbError> {
        let s = self.state.read().await;
        Ok(s.works
            .iter()
            .filter(|w| {
                (w.enrichment_status == EnrichmentStatus::Failed
                    || w.enrichment_status == EnrichmentStatus::Partial)
                    && w.enrichment_retry_count < 3
            })
            .cloned()
            .collect())
    }

    async fn reset_enrichment_for_refresh(
        &self,
        _user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let work = s
            .works
            .iter_mut()
            .find(|w| w.id == work_id)
            .ok_or(DbError::NotFound)?;
        work.enrichment_status = EnrichmentStatus::Pending;
        work.enrichment_retry_count = 0;
        Ok(())
    }

    async fn increment_retry_count(
        &self,
        _user_id: UserId,
        work_id: WorkId,
    ) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let work = s
            .works
            .iter_mut()
            .find(|w| w.id == work_id)
            .ok_or(DbError::NotFound)?;
        work.enrichment_retry_count += 1;
        // Transition to exhausted only when status is Failed and count >= 3
        if work.enrichment_status == EnrichmentStatus::Failed && work.enrichment_retry_count >= 3 {
            work.enrichment_status = EnrichmentStatus::Exhausted;
        }
        Ok(())
    }
}

// =============================================================================
// IndexerDb
// =============================================================================

#[async_trait::async_trait]
impl IndexerDb for InMemoryDb {
    async fn get_indexer(&self, id: IndexerId) -> Result<Indexer, DbError> {
        let s = self.state.read().await;
        s.indexers
            .iter()
            .find(|ix| ix.id == id)
            .cloned()
            .ok_or(DbError::NotFound)
    }

    async fn list_indexers(&self) -> Result<Vec<Indexer>, DbError> {
        let s = self.state.read().await;
        Ok(s.indexers.clone())
    }

    async fn list_enabled_interactive_indexers(&self) -> Result<Vec<Indexer>, DbError> {
        let s = self.state.read().await;
        Ok(s.indexers
            .iter()
            .filter(|ix| ix.enabled && ix.enable_interactive_search)
            .cloned()
            .collect())
    }

    async fn create_indexer(&self, req: CreateIndexerDbRequest) -> Result<Indexer, DbError> {
        let mut s = self.state.write().await;
        let id = s.next_id();
        let indexer = Indexer {
            id,
            name: req.name,
            protocol: req.protocol,
            url: req.url,
            api_path: req.api_path,
            api_key: req.api_key,
            categories: req.categories,
            priority: req.priority,
            enable_automatic_search: req.enable_automatic_search,
            enable_interactive_search: req.enable_interactive_search,
            supports_book_search: false,
            enabled: req.enabled,
            added_at: Utc::now(),
        };
        s.indexers.push(indexer.clone());
        Ok(indexer)
    }

    async fn update_indexer(
        &self,
        id: IndexerId,
        req: UpdateIndexerDbRequest,
    ) -> Result<Indexer, DbError> {
        let mut s = self.state.write().await;
        let ix = s
            .indexers
            .iter_mut()
            .find(|ix| ix.id == id)
            .ok_or(DbError::NotFound)?;

        // Track whether connection-affecting fields change — if so, reset
        // supports_book_search since the new endpoint hasn't been probed yet.
        let mut reset_supports = false;
        if let Some(v) = req.url {
            if v != ix.url {
                reset_supports = true;
            }
            ix.url = v;
        }
        if let Some(v) = req.api_path {
            if v != ix.api_path {
                reset_supports = true;
            }
            ix.api_path = v;
        }
        if let Some(v) = req.api_key {
            if Some(&v) != ix.api_key.as_ref() {
                reset_supports = true;
            }
            ix.api_key = Some(v);
        }
        if let Some(v) = req.name {
            ix.name = v;
        }
        if let Some(v) = req.categories {
            ix.categories = v;
        }
        if let Some(v) = req.priority {
            ix.priority = v;
        }
        if let Some(v) = req.enable_automatic_search {
            ix.enable_automatic_search = v;
        }
        if let Some(v) = req.enable_interactive_search {
            ix.enable_interactive_search = v;
        }
        if let Some(v) = req.enabled {
            ix.enabled = v;
        }
        if reset_supports {
            ix.supports_book_search = false;
        }
        Ok(ix.clone())
    }

    async fn delete_indexer(&self, id: IndexerId) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let idx = s
            .indexers
            .iter()
            .position(|ix| ix.id == id)
            .ok_or(DbError::NotFound)?;
        s.indexers.remove(idx);
        Ok(())
    }

    async fn set_supports_book_search(&self, id: IndexerId, supports: bool) -> Result<(), DbError> {
        let mut s = self.state.write().await;
        let ix = s
            .indexers
            .iter_mut()
            .find(|ix| ix.id == id)
            .ok_or(DbError::NotFound)?;
        ix.supports_book_search = supports;
        Ok(())
    }
}
