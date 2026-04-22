use livrarr_domain::services::{SortDirection, WorkSortField};
use serde::Serialize;

#[derive(Debug, serde::Deserialize)]
pub struct PaginationQuery {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub sort_by: Option<WorkSortField>,
    pub sort_dir: Option<SortDirection>,
}

impl PaginationQuery {
    pub fn page(&self) -> u32 {
        self.page.unwrap_or(1).max(1)
    }
    pub fn page_size(&self) -> u32 {
        self.page_size.unwrap_or(100).clamp(1, 1000)
    }
    pub fn sort_by(&self) -> WorkSortField {
        self.sort_by.unwrap_or(WorkSortField::DateAdded)
    }
    pub fn sort_dir(&self) -> SortDirection {
        self.sort_dir.unwrap_or(SortDirection::Desc)
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedResponse<T: Serialize> {
    pub items: Vec<T>,
    pub total: i64,
    pub page: u32,
    pub page_size: u32,
}
