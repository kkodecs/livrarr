use crate::{AuthorId, DbError, LibraryItem, Series, UserId, Work, WorkId};

#[derive(Debug, thiserror::Error)]
pub enum SeriesServiceError {
    #[error("series not found")]
    NotFound,
    #[error("validation: {field}: {message}")]
    Validation { field: String, message: String },
    #[error("Goodreads unavailable")]
    GoodreadsUnavailable,
    #[error("database error: {0}")]
    Db(#[from] DbError),
}

#[trait_variant::make(Send)]
pub trait SeriesService: Send + Sync {
    async fn list(&self, user_id: UserId) -> Result<Vec<Series>, SeriesServiceError>;
    async fn get(&self, user_id: UserId, series_id: i64) -> Result<Series, SeriesServiceError>;
    async fn refresh(&self, user_id: UserId, series_id: i64) -> Result<Series, SeriesServiceError>;
    async fn monitor(
        &self,
        user_id: UserId,
        series_id: i64,
        monitored: bool,
    ) -> Result<Series, SeriesServiceError>;
    async fn update(
        &self,
        user_id: UserId,
        series_id: i64,
        title: Option<String>,
    ) -> Result<Series, SeriesServiceError>;
}

#[derive(Debug)]
pub struct SeriesListView {
    pub id: i64,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub works_in_library: i64,
    pub author_id: i64,
    pub author_name: String,
    pub first_work_id: Option<WorkId>,
}

#[derive(Debug)]
pub struct SeriesDetailView {
    pub id: i64,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub author_id: i64,
    pub author_name: String,
    pub works: Vec<SeriesWorkView>,
}

#[derive(Debug)]
pub struct SeriesWorkView {
    pub work: Work,
    pub library_items: Vec<LibraryItem>,
}

/// One row of a series' full-roster expansion (REQ-010).
#[derive(Debug)]
pub enum SeriesBookRow {
    /// Roster entry (or linked work) present in the library.
    InLibrary {
        position: Option<f64>,
        entry: Box<SeriesWorkView>,
    },
    /// Roster entry the library has no work for — display-only.
    Missing {
        position: Option<f64>,
        title: String,
        year: Option<i32>,
    },
}

/// Full-roster expansion of one series (REQ-010). `roster_available` is false
/// for stubs (no gr_key — no roster source); their rows are linked works only.
#[derive(Debug)]
pub struct SeriesBooksView {
    pub roster_available: bool,
    pub rows: Vec<SeriesBookRow>,
}

#[derive(Debug)]
pub struct UpdateSeriesView {
    pub id: i64,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub works_in_library: i64,
}

#[derive(Debug, Clone)]
pub struct GrAuthorCandidateView {
    pub gr_key: String,
    pub name: String,
    pub profile_url: String,
}

#[derive(Debug)]
pub struct AuthorSeriesListView {
    pub series: Vec<AuthorSeriesItemView>,
    pub fetched_at: Option<String>,
    pub raw_available: bool,
    pub filtered_count: usize,
    pub raw_count: usize,
}

#[derive(Debug)]
pub struct AuthorSeriesItemView {
    pub id: Option<i64>,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub works_in_library: i64,
}

#[derive(Debug)]
pub struct MonitorSeriesServiceRequest {
    pub gr_key: String,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
}

#[derive(Debug)]
pub struct MonitorSeriesView {
    pub id: i64,
    pub name: String,
    pub gr_key: String,
    pub book_count: i32,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
    pub works_in_library: i64,
}

#[derive(Debug)]
pub struct SeriesMonitorWorkerParams {
    pub user_id: UserId,
    pub author_id: AuthorId,
    pub series_id: i64,
    pub series_name: String,
    pub series_gr_key: String,
    pub monitor_ebook: bool,
    pub monitor_audiobook: bool,
}

/// Outcome of a stub-promotion attempt (REQ-009). `Resolved` means the row
/// now carries a real gr_key (adopted in place, or merged into an existing
/// GR-backed row) and the caller can run the normal monitor flow on it.
#[derive(Debug)]
pub enum PromoteStubOutcome {
    /// The stub's author has no gr_key — resolve the author first
    /// (existing author-candidate flow), then retry.
    NeedsAuthorResolution { author_id: AuthorId },
    /// No exact name match (or several) among the author's GR series — the
    /// caller surfaces the picker; retry with an explicit gr_key.
    NeedsPicker {
        author_id: AuthorId,
        candidates: Vec<AuthorSeriesItemView>,
    },
    /// gr_key resolved. `series_id` is the surviving row (the stub itself,
    /// or the pre-existing row the stub merged into).
    Resolved {
        author_id: AuthorId,
        series_id: i64,
        gr_key: String,
        name: String,
    },
}

#[trait_variant::make(Send)]
pub trait SeriesQueryService: Send + Sync {
    async fn list_enriched(
        &self,
        user_id: UserId,
    ) -> Result<Vec<SeriesListView>, SeriesServiceError>;
    async fn get_detail(
        &self,
        user_id: UserId,
        series_id: i64,
    ) -> Result<SeriesDetailView, SeriesServiceError>;
    async fn update_flags(
        &self,
        user_id: UserId,
        series_id: i64,
        monitor_ebook: bool,
        monitor_audiobook: bool,
    ) -> Result<UpdateSeriesView, SeriesServiceError>;
    async fn resolve_gr_candidates(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<Vec<GrAuthorCandidateView>, SeriesServiceError>;
    async fn list_author_series(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        raw: bool,
    ) -> Result<AuthorSeriesListView, SeriesServiceError>;
    async fn refresh_author_series(
        &self,
        user_id: UserId,
        author_id: AuthorId,
    ) -> Result<AuthorSeriesListView, SeriesServiceError>;
    async fn monitor_series(
        &self,
        user_id: UserId,
        author_id: AuthorId,
        req: MonitorSeriesServiceRequest,
    ) -> Result<MonitorSeriesView, SeriesServiceError>;
    async fn run_series_monitor_worker(
        &self,
        params: SeriesMonitorWorkerParams,
    ) -> Result<(), SeriesServiceError>;
    /// Resolve a stub series' gr_key (REQ-009): author key required (else
    /// NeedsAuthorResolution), exact normalized-name match adopts silently,
    /// ambiguity surfaces NeedsPicker, an explicit `gr_key` (picker choice)
    /// short-circuits matching. A gr_key collision with another row for the
    /// same author merges the stub into that row (works relinked, stub
    /// deleted). Never enables monitoring itself — the caller runs the
    /// monitor flow on the Resolved row.
    async fn promote_stub(
        &self,
        user_id: UserId,
        series_id: i64,
        explicit_gr_key: Option<String>,
    ) -> Result<PromoteStubOutcome, SeriesServiceError>;
    /// Full-roster expansion (REQ-010): merge the persisted roster with the
    /// series' FK-linked works. Fetches + persists the roster once when a
    /// GR-backed series has none stored; stubs never fetch.
    async fn series_books(
        &self,
        user_id: UserId,
        series_id: i64,
    ) -> Result<SeriesBooksView, SeriesServiceError>;
}
