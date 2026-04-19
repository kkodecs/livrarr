use livrarr_domain::services::{MatchCluster, MatchInput, MatchingService};

#[derive(Clone)]
pub struct LiveMatchingService;

impl MatchingService for LiveMatchingService {
    async fn extract_and_reconcile(&self, input: &MatchInput) -> Vec<MatchCluster> {
        let server_input = crate::matching::types::MatchInput {
            file_path: input.file_path.clone(),
            grouped_paths: input.grouped_paths.clone(),
            parse_string: input.parse_string.clone(),
            media_type: input.media_type,
            scan_root: input.scan_root.clone(),
        };
        let clusters = crate::matching::extract_and_reconcile(&server_input).await;
        clusters
            .into_iter()
            .map(|c| MatchCluster {
                author: c.primary.author,
                title: c.primary.title,
                series: c.primary.series,
                series_position: c.primary.series_position,
                language: c.primary.language,
            })
            .collect()
    }
}
