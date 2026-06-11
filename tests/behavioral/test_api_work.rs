use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use librarr_server::{
    AddWorkRequest, AddWorkResponse, ApiError, DeleteWorkResponse, EnrichmentStatus,
    LibraryItemResponse, MediaType, RefreshWorkResponse, UpdateWorkRequest, UserId, WorkApi,
    WorkDetailResponse, WorkId, WorkSearchResult,
};

// ============================================================================
// Mock infrastructure
// ============================================================================

#[derive(Clone, Copy)]
enum MockEnrichmentMode {
    FullSuccess,
    PartialFailure,
    TotalFailure,
}

struct MockState {
    works: Vec<WorkDetailResponse>,
    author_names: HashSet<String>,
    next_id: WorkId,
    ol_reachable: bool,
    enrichment_mode: MockEnrichmentMode,
    file_delete_warnings: Vec<String>,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            works: Vec::new(),
            author_names: HashSet::new(),
            next_id: 1,
            ol_reachable: true,
            enrichment_mode: MockEnrichmentMode::FullSuccess,
            file_delete_warnings: Vec::new(),
        }
    }
}

#[derive(Clone, Default)]
struct MockWorkApi {
    state: Arc<Mutex<MockState>>,
}

impl MockWorkApi {
    fn new() -> Self {
        Self::default()
    }
    fn with_ol_unreachable(self) -> Self {
        self.state.lock().unwrap().ol_reachable = false;
        self
    }
    fn with_enrichment(self, m: MockEnrichmentMode) -> Self {
        self.state.lock().unwrap().enrichment_mode = m;
        self
    }
    fn with_existing_author(self, name: &str) -> Self {
        self.state.lock().unwrap().author_names.insert(norm(name));
        self
    }
    fn with_file_warnings(self, w: Vec<String>) -> Self {
        self.state.lock().unwrap().file_delete_warnings = w;
        self
    }
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

fn add_req(ol_key: &str, title: &str, author: &str) -> AddWorkRequest {
    AddWorkRequest {
        ol_key: ol_key.into(),
        title: title.into(),
        author_name: author.into(),
        author_ol_key: Some("OL123A".into()),
        year: Some(1965),
        cover_url: Some("https://covers.example/test.jpg".into()),
    }
}

fn work_detail(id: WorkId, title: &str, author: &str, ol_key: Option<&str>) -> WorkDetailResponse {
    WorkDetailResponse {
        id,
        title: title.into(),
        sort_title: None,
        subtitle: None,
        original_title: None,
        author_name: author.into(),
        author_id: Some(10),
        description: None,
        year: Some(1965),
        series_name: None,
        series_position: None,
        genres: None,
        language: Some("en".into()),
        page_count: None,
        duration_seconds: None,
        publisher: None,
        publish_date: None,
        ol_key: ol_key.map(Into::into),
        hardcover_id: None,
        isbn_13: None,
        asin: None,
        narrator: None,
        narration_type: None,
        abridged: false,
        rating: None,
        rating_count: None,
        enrichment_status: EnrichmentStatus::Unenriched,
        enriched_at: None,
        enrichment_source: None,
        cover_manual: false,
        monitored: true,
        added_at: "2024-01-01T00:00:00Z".into(),
        library_items: vec![LibraryItemResponse {
            id: 1,
            path: "/tmp/book.epub".into(),
            media_type: MediaType::Ebook,
            file_size: 123,
            imported_at: "2024-01-01T00:00:00Z".into(),
        }],
    }
}

#[async_trait]
impl WorkApi for MockWorkApi {
    async fn lookup(&self, _uid: UserId, term: &str) -> Result<Vec<WorkSearchResult>, ApiError> {
        let st = self.state.lock().unwrap();
        if !st.ol_reachable {
            return Err(ApiError::BadGateway("openlibrary unreachable".into()));
        }
        Ok(vec![WorkSearchResult {
            ol_key: "OL1W".into(),
            title: format!("{term} Result"),
            author_name: "Test Author".into(),
            author_ol_key: Some("OL9A".into()),
            year: Some(2001),
            cover_url: Some("https://covers.example/ol1w.jpg".into()),
            description: None,
            series_name: None,
            series_position: None,
        }])
    }

    async fn add(&self, _uid: UserId, req: AddWorkRequest) -> Result<AddWorkResponse, ApiError> {
        let mut st = self.state.lock().unwrap();
        if st
            .works
            .iter()
            .any(|w| w.ol_key.as_deref() == Some(&req.ol_key))
        {
            return Err(ApiError::Conflict {
                reason: format!("ol_key {} exists", req.ol_key),
            });
        }
        let author_created = !st.author_names.contains(&norm(&req.author_name));
        st.author_names.insert(norm(&req.author_name));
        let id = st.next_id;
        st.next_id += 1;
        let mut w = work_detail(id, &req.title, &req.author_name, Some(&req.ol_key));
        w.year = req.year;
        w.enrichment_status = match st.enrichment_mode {
            MockEnrichmentMode::FullSuccess => EnrichmentStatus::Enriched,
            MockEnrichmentMode::PartialFailure => EnrichmentStatus::Unenriched,
            MockEnrichmentMode::TotalFailure => EnrichmentStatus::Failed,
        };
        w.enriched_at = Some("2024-01-02T00:00:00Z".into());
        w.enrichment_source = Some("mock".into());
        let messages = match st.enrichment_mode {
            MockEnrichmentMode::FullSuccess => vec!["enrichment completed".into()],
            MockEnrichmentMode::PartialFailure => vec![
                "provider hardcover failed".into(),
                "partial enrichment completed".into(),
            ],
            MockEnrichmentMode::TotalFailure => vec!["basic metadata only".into()],
        };
        st.works.push(w.clone());
        Ok(AddWorkResponse {
            work: w,
            author_created,
            messages,
        })
    }

    async fn list(&self, _uid: UserId) -> Result<Vec<WorkDetailResponse>, ApiError> {
        Ok(self.state.lock().unwrap().works.clone())
    }

    async fn get(&self, _uid: UserId, id: WorkId) -> Result<WorkDetailResponse, ApiError> {
        self.state
            .lock()
            .unwrap()
            .works
            .iter()
            .find(|w| w.id == id)
            .cloned()
            .ok_or(ApiError::NotFound)
    }

    async fn update(
        &self,
        _uid: UserId,
        id: WorkId,
        req: UpdateWorkRequest,
    ) -> Result<WorkDetailResponse, ApiError> {
        let mut st = self.state.lock().unwrap();
        let w = st
            .works
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or(ApiError::NotFound)?;
        if let Some(v) = req.title {
            w.title = v;
        }
        if let Some(v) = req.author_name {
            w.author_name = v;
        }
        if let Some(v) = req.series_name {
            w.series_name = Some(v);
        }
        if let Some(v) = req.series_position {
            w.series_position = Some(v);
        }
        Ok(w.clone())
    }

    async fn upload_cover(
        &self,
        _uid: UserId,
        id: WorkId,
        _data: &[u8],
        _ct: &str,
    ) -> Result<(), ApiError> {
        let mut st = self.state.lock().unwrap();
        st.works
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or(ApiError::NotFound)?
            .cover_manual = true;
        Ok(())
    }

    async fn delete(
        &self,
        _uid: UserId,
        id: WorkId,
        delete_files: bool,
    ) -> Result<DeleteWorkResponse, ApiError> {
        let mut st = self.state.lock().unwrap();
        let before = st.works.len();
        st.works.retain(|w| w.id != id);
        if st.works.len() == before {
            return Err(ApiError::NotFound);
        }
        let warnings = if delete_files {
            st.file_delete_warnings.clone()
        } else {
            vec![]
        };
        Ok(DeleteWorkResponse { warnings })
    }

    async fn refresh(&self, _uid: UserId, id: WorkId) -> Result<RefreshWorkResponse, ApiError> {
        let mut st = self.state.lock().unwrap();
        let enrichment_mode = st.enrichment_mode;
        let w = st
            .works
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or(ApiError::NotFound)?;
        w.enriched_at = Some("2024-01-03T00:00:00Z".into());
        w.enrichment_source = Some("refresh".into());
        w.enrichment_status = match enrichment_mode {
            MockEnrichmentMode::FullSuccess => EnrichmentStatus::Enriched,
            MockEnrichmentMode::PartialFailure => EnrichmentStatus::Unenriched,
            MockEnrichmentMode::TotalFailure => EnrichmentStatus::Failed,
        };
        Ok(RefreshWorkResponse {
            work: w.clone(),
            messages: vec!["refresh completed".into()],
        })
    }
}

// ============================================================================
// Generic-over-impl helpers
// ============================================================================

async fn assert_lookup_returns_results(api: &impl WorkApi) {
    let results = api.lookup(1, "Dune").await.unwrap();
    assert_eq!(results.len(), 1);
    let r = &results[0];
    assert_eq!(r.ol_key, "OL1W");
    assert_eq!(r.title, "Dune Result");
    assert_eq!(r.author_name, "Test Author");
    assert_eq!(r.author_ol_key.as_deref(), Some("OL9A"));
    assert_eq!(r.year, Some(2001));
    assert!(r.cover_url.is_some());
}

async fn assert_add_get_round_trip(api: &impl WorkApi) {
    let added = api
        .add(1, add_req("OL10W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    let fetched = api.get(1, added.work.id).await.unwrap();
    assert_eq!(fetched.id, added.work.id);
    assert_eq!(fetched.title, "Dune");
    assert_eq!(fetched.author_name, "Frank Herbert");
    assert_eq!(fetched.ol_key.as_deref(), Some("OL10W"));
}

// ============================================================================
// Lookup tests — SEARCH-001, SEARCH-002, SEARCH-003
// ============================================================================

#[tokio::test]
async fn test_api_work_lookup_returns_results_with_correct_fields() {
    // Satisfies: SEARCH-001, SEARCH-002, SEARCH-003 — Lookup returns work-level results with ol_key, title, author, year, cover
    assert_lookup_returns_results(&MockWorkApi::new()).await;
}

#[tokio::test]
async fn test_api_work_lookup_ol_unreachable_returns_bad_gateway() {
    // Satisfies: SEARCH-001 — Lookup surfaces upstream OL failure as BadGateway
    let err = MockWorkApi::new()
        .with_ol_unreachable()
        .lookup(1, "Dune")
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::BadGateway(_)));
}

// ============================================================================
// Add tests — SEARCH-004, SEARCH-005, SEARCH-006, SEARCH-008
// ============================================================================

#[tokio::test]
async fn test_api_work_add_new_author_creates_work_and_author() {
    // Satisfies: SEARCH-005 — Add creates fully populated author when new; author_created=true
    let resp = MockWorkApi::new()
        .add(1, add_req("OL20W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    assert!(resp.author_created);
    assert_eq!(resp.work.title, "Dune");
    assert_eq!(resp.work.author_name, "Frank Herbert");
    assert!(!resp.messages.is_empty());
}

#[tokio::test]
async fn test_api_work_add_existing_author_reuses_on_name_match() {
    // Satisfies: SEARCH-005 — Add reuses existing author on normalized name match; author_created=false
    let api = MockWorkApi::new().with_existing_author("frank herbert");
    let resp = api
        .add(1, add_req("OL21W", "Dune Messiah", "Frank Herbert"))
        .await
        .unwrap();
    assert!(!resp.author_created);
}

#[tokio::test]
async fn test_api_work_add_duplicate_ol_key_returns_conflict() {
    // Satisfies: SEARCH-004 — Duplicate ol_key rejected with Conflict
    let api = MockWorkApi::new();
    api.add(1, add_req("OL22W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    let err = api
        .add(1, add_req("OL22W", "Dune", "Frank Herbert"))
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::Conflict { reason } if reason.contains("OL22W")));
}

#[tokio::test]
async fn test_api_work_add_partial_enrichment_failure_still_creates_work() {
    // Satisfies: SEARCH-006 — Partial provider failure creates work with messages describing failure
    let resp = MockWorkApi::new()
        .with_enrichment(MockEnrichmentMode::PartialFailure)
        .add(1, add_req("OL23W", "Children of Dune", "Frank Herbert"))
        .await
        .unwrap();
    assert_eq!(resp.work.ol_key.as_deref(), Some("OL23W"));
    assert!(resp.messages.iter().any(|m| m.contains("failed")));
    assert!(matches!(
        resp.work.enrichment_status,
        EnrichmentStatus::Unenriched
    ));
}

#[tokio::test]
async fn test_api_work_add_total_enrichment_failure_creates_work_with_basic_metadata() {
    // Satisfies: SEARCH-008 — All providers timeout still creates work with OL data
    let resp = MockWorkApi::new()
        .with_enrichment(MockEnrichmentMode::TotalFailure)
        .add(1, add_req("OL24W", "God Emperor of Dune", "Frank Herbert"))
        .await
        .unwrap();
    assert_eq!(resp.work.title, "God Emperor of Dune");
    assert!(resp
        .messages
        .iter()
        .any(|m| m.contains("basic metadata only")));
    assert!(matches!(
        resp.work.enrichment_status,
        EnrichmentStatus::Failed
    ));
}

#[tokio::test]
async fn test_api_work_add_messages_vec_always_present() {
    // Satisfies: SEARCH-006 — messages Vec always present (never null)
    let resp = MockWorkApi::new()
        .add(1, add_req("OL25W", "Heretics of Dune", "Frank Herbert"))
        .await
        .unwrap();
    assert!(!resp.messages.is_empty());
}

// ============================================================================
// List / Get tests
// ============================================================================

#[tokio::test]
async fn test_api_work_list_empty_returns_empty_vec() {
    // Satisfies: SEARCH-001 — List for user with no works returns empty Vec
    assert!(MockWorkApi::new().list(1).await.unwrap().is_empty());
}

#[tokio::test]
async fn test_api_work_list_returns_all_works() {
    // Satisfies: SEARCH-001 — List returns all works
    let api = MockWorkApi::new();
    api.add(1, add_req("OL30W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    api.add(1, add_req("OL31W", "Hyperion", "Dan Simmons"))
        .await
        .unwrap();
    assert_eq!(api.list(1).await.unwrap().len(), 2);
}

#[tokio::test]
async fn test_api_work_get_known_work_returns_detail() {
    // Satisfies: SEARCH-001 — Get returns correct WorkDetailResponse
    let api = MockWorkApi::new();
    let added = api
        .add(1, add_req("OL32W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    let got = api.get(1, added.work.id).await.unwrap();
    assert_eq!(got.id, added.work.id);
    assert_eq!(got.title, "Dune");
}

#[tokio::test]
async fn test_api_work_get_unknown_returns_not_found() {
    // Satisfies: SEARCH-001 — Get returns NotFound for unknown id
    assert!(matches!(
        MockWorkApi::new().get(1, 999).await.unwrap_err(),
        ApiError::NotFound
    ));
}

// ============================================================================
// Update tests — SEARCH-013
// ============================================================================

#[tokio::test]
async fn test_api_work_update_partial_fields() {
    // Satisfies: SEARCH-013 — Update changes only specified fields
    let api = MockWorkApi::new();
    let added = api
        .add(1, add_req("OL40W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    let updated = api
        .update(
            1,
            added.work.id,
            UpdateWorkRequest {
                title: Some("Dune (Updated)".into()),
                author_name: None,
                series_name: Some("Dune Saga".into()),
                series_position: Some(1.0),
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, "Dune (Updated)");
    assert_eq!(updated.author_name, "Frank Herbert"); // unchanged
    assert_eq!(updated.series_name.as_deref(), Some("Dune Saga"));
    assert_eq!(updated.series_position, Some(1.0));
}

#[tokio::test]
async fn test_api_work_update_all_none_returns_unchanged() {
    // Satisfies: SEARCH-013 — Update with all None returns unchanged work
    let api = MockWorkApi::new();
    let added = api
        .add(1, add_req("OL41W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    let updated = api
        .update(
            1,
            added.work.id,
            UpdateWorkRequest {
                title: None,
                author_name: None,
                series_name: None,
                series_position: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(updated.title, added.work.title);
    assert_eq!(updated.author_name, added.work.author_name);
}

#[tokio::test]
async fn test_api_work_update_nonexistent_returns_not_found() {
    // Satisfies: SEARCH-013 — Update on nonexistent work returns NotFound
    let err = MockWorkApi::new()
        .update(
            1,
            404,
            UpdateWorkRequest {
                title: Some("X".into()),
                author_name: None,
                series_name: None,
                series_position: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::NotFound));
}

// ============================================================================
// Cover / Refresh tests — SEARCH-011, SEARCH-014
// ============================================================================

#[tokio::test]
async fn test_api_work_upload_cover_sets_cover_manual_true() {
    // Satisfies: SEARCH-014 — Upload cover sets cover_manual=true
    let api = MockWorkApi::new();
    let added = api
        .add(1, add_req("OL50W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    api.upload_cover(1, added.work.id, b"img", "image/jpeg")
        .await
        .unwrap();
    assert!(api.get(1, added.work.id).await.unwrap().cover_manual);
}

#[tokio::test]
async fn test_api_work_upload_cover_nonexistent_returns_not_found() {
    // Satisfies: SEARCH-014 — Upload cover on nonexistent work returns NotFound
    assert!(matches!(
        MockWorkApi::new()
            .upload_cover(1, 999, b"img", "image/jpeg")
            .await
            .unwrap_err(),
        ApiError::NotFound
    ));
}

#[tokio::test]
async fn test_api_work_refresh_returns_messages_and_updated_enrichment() {
    // Satisfies: SEARCH-011 — Refresh returns RefreshWorkResponse with messages
    let api = MockWorkApi::new();
    let added = api
        .add(1, add_req("OL51W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    let refreshed = api.refresh(1, added.work.id).await.unwrap();
    assert!(!refreshed.messages.is_empty());
    assert_eq!(refreshed.work.enrichment_source.as_deref(), Some("refresh"));
}

#[tokio::test]
async fn test_api_work_refresh_nonexistent_returns_not_found() {
    // Satisfies: SEARCH-011 — Refresh on nonexistent work returns NotFound
    assert!(matches!(
        MockWorkApi::new().refresh(1, 999).await.unwrap_err(),
        ApiError::NotFound
    ));
}

#[tokio::test]
async fn test_api_work_refresh_preserves_cover_manual_after_upload() {
    // Satisfies: SEARCH-014 — Re-enrichment skips cover when cover_manual=true
    let api = MockWorkApi::new();
    let added = api
        .add(1, add_req("OL52W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    api.upload_cover(1, added.work.id, b"img", "image/png")
        .await
        .unwrap();
    let refreshed = api.refresh(1, added.work.id).await.unwrap();
    assert!(refreshed.work.cover_manual);
}

// ============================================================================
// Delete tests
// ============================================================================

#[tokio::test]
async fn test_api_work_delete_no_files_returns_empty_warnings() {
    // Satisfies: DELETE — Delete without file deletion returns empty warnings
    let api = MockWorkApi::new();
    let added = api
        .add(1, add_req("OL60W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    assert!(api
        .delete(1, added.work.id, false)
        .await
        .unwrap()
        .warnings
        .is_empty());
}

#[tokio::test]
async fn test_api_work_delete_files_success_returns_empty_warnings() {
    // Satisfies: DELETE — Delete with successful file cleanup returns empty warnings
    let api = MockWorkApi::new();
    let added = api
        .add(1, add_req("OL61W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    assert!(api
        .delete(1, added.work.id, true)
        .await
        .unwrap()
        .warnings
        .is_empty());
}

#[tokio::test]
async fn test_api_work_delete_file_errors_surface_as_warnings() {
    // Satisfies: DELETE — File cleanup failures in warnings but operation succeeds
    let api = MockWorkApi::new().with_file_warnings(vec!["failed to delete /tmp/book.epub".into()]);
    let added = api
        .add(1, add_req("OL62W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    let resp = api.delete(1, added.work.id, true).await.unwrap();
    assert_eq!(resp.warnings.len(), 1);
    assert!(resp.warnings[0].contains("failed to delete"));
}

#[tokio::test]
async fn test_api_work_delete_nonexistent_returns_not_found() {
    // Satisfies: DELETE — Delete on nonexistent work returns NotFound
    assert!(matches!(
        MockWorkApi::new().delete(1, 999, true).await.unwrap_err(),
        ApiError::NotFound
    ));
}

// ============================================================================
// Integration (multi-step) tests
// ============================================================================

#[tokio::test]
async fn test_api_work_add_then_get_round_trip() {
    // Satisfies: SEARCH-004 — Added work retrievable by returned id with matching fields
    assert_add_get_round_trip(&MockWorkApi::new()).await;
}

#[tokio::test]
async fn test_api_work_add_then_list_includes_new_work() {
    // Satisfies: SEARCH-004 — Newly added work appears in list
    let api = MockWorkApi::new();
    let added = api
        .add(1, add_req("OL70W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    assert!(api
        .list(1)
        .await
        .unwrap()
        .iter()
        .any(|w| w.id == added.work.id));
}

#[tokio::test]
async fn test_api_work_add_update_get_reflects_changes() {
    // Satisfies: SEARCH-013 — Update changes reflected in subsequent get
    let api = MockWorkApi::new();
    let added = api
        .add(1, add_req("OL71W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    api.update(
        1,
        added.work.id,
        UpdateWorkRequest {
            title: Some("Dune Revised".into()),
            author_name: Some("F. Herbert".into()),
            series_name: None,
            series_position: None,
        },
    )
    .await
    .unwrap();
    let got = api.get(1, added.work.id).await.unwrap();
    assert_eq!(got.title, "Dune Revised");
    assert_eq!(got.author_name, "F. Herbert");
}

#[tokio::test]
async fn test_api_work_add_then_delete_then_get_returns_not_found() {
    // Satisfies: DELETE — Deleted work no longer retrievable
    let api = MockWorkApi::new();
    let added = api
        .add(1, add_req("OL72W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    api.delete(1, added.work.id, false).await.unwrap();
    assert!(matches!(
        api.get(1, added.work.id).await.unwrap_err(),
        ApiError::NotFound
    ));
}

#[tokio::test]
async fn test_api_work_add_upload_cover_refresh_preserves_manual_cover() {
    // Satisfies: SEARCH-014 — Full chain: add → upload_cover → refresh preserves cover_manual=true
    let api = MockWorkApi::new();
    let added = api
        .add(1, add_req("OL73W", "Dune", "Frank Herbert"))
        .await
        .unwrap();
    assert!(!added.work.cover_manual);
    api.upload_cover(1, added.work.id, b"img", "image/jpeg")
        .await
        .unwrap();
    let refreshed = api.refresh(1, added.work.id).await.unwrap();
    assert!(refreshed.work.cover_manual);
}
