use livrarr_domain::MediaType;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub matched: i64,
    pub unmatched: Vec<ScanUnmatchedFile>,
    pub errors: Vec<ScanErrorEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanUnmatchedFile {
    pub path: String,
    pub media_type: MediaType,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanErrorEntry {
    pub path: String,
    pub message: String,
}
