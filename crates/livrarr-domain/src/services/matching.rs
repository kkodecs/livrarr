use crate::MediaType;

#[derive(Debug, Clone)]
pub struct MatchCluster {
    pub author: Option<String>,
    pub title: Option<String>,
    pub series: Option<String>,
    pub series_position: Option<f64>,
    pub language: Option<String>,
}

#[derive(Debug)]
pub struct MatchInput {
    pub file_path: Option<std::path::PathBuf>,
    pub grouped_paths: Option<Vec<std::path::PathBuf>>,
    pub parse_string: Option<String>,
    pub media_type: Option<MediaType>,
    pub scan_root: Option<std::path::PathBuf>,
}

#[trait_variant::make(Send)]
pub trait MatchingService: Send + Sync {
    async fn extract_and_reconcile(&self, input: &MatchInput) -> Vec<MatchCluster>;
}
