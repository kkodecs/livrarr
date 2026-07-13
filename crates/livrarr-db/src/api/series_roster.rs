//! Series roster (GR series-page scrape) data access: `SeriesRosterDb` trait.

use crate::DbError;

/// One parsed roster entry of a GR series page (REQ-010): a primary work of
/// the series, as scraped by the monitor worker's fetch road.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesRosterEntry {
    pub title: String,
    pub gr_key: String,
    pub position: Option<f64>,
    pub year: Option<i32>,
}

/// Persisted roster of a series (one row per series; FK CASCADE).
#[derive(Debug, Clone)]
pub struct SeriesRoster {
    pub series_id: i64,
    pub entries: Vec<SeriesRosterEntry>,
    pub fetched_at: String,
}

#[trait_variant::make(Send)]
pub trait SeriesRosterDb: Send + Sync {
    async fn get_series_roster(&self, series_id: i64) -> Result<Option<SeriesRoster>, DbError>;

    /// Upsert the roster for a series (worker write-through or first-expand
    /// fetch). An empty entry list is stored too — "fetched, found none" must
    /// not refetch on every expansion.
    async fn save_series_roster(
        &self,
        series_id: i64,
        entries: &[SeriesRosterEntry],
    ) -> Result<SeriesRoster, DbError>;
}
