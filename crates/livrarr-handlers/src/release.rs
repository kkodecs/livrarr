use axum::extract::{Query, State};
use axum::Json;

use crate::context::HasReleaseService;
use crate::types::api_error::ApiError;
use crate::types::auth::AuthContext;
use crate::types::release::{
    GrabApiRequest, ReleaseResponse, ReleaseSearchResponse, SearchWarning,
};
use livrarr_domain::services::{DownloadProtocol, GrabRequest, GrabSource, ReleaseService};

#[derive(serde::Deserialize)]
pub struct SearchQuery {
    #[serde(rename = "workId")]
    pub work_id: Option<i64>,
    #[serde(default)]
    pub refresh: bool,
    #[serde(default, rename = "cacheOnly")]
    pub cache_only: bool,
}

pub async fn search<S: HasReleaseService>(
    State(state): State<S>,
    ctx: AuthContext,
    Query(q): Query<SearchQuery>,
) -> Result<Json<ReleaseSearchResponse>, ApiError> {
    use livrarr_domain::services::SearchReleasesRequest;

    let work_id = match q.work_id {
        Some(id) => id,
        None => {
            return Ok(Json(ReleaseSearchResponse {
                results: vec![],
                warnings: vec![],
                cache_age_seconds: None,
            }))
        }
    };

    let svc_response = match state
        .release_service()
        .search(
            ctx.user.id,
            SearchReleasesRequest {
                work_id,
                refresh: q.refresh,
                cache_only: q.cache_only,
            },
        )
        .await
    {
        Ok(resp) => resp,
        Err(livrarr_domain::services::ReleaseServiceError::AllIndexersFailed) => {
            return Ok(Json(ReleaseSearchResponse {
                results: vec![],
                warnings: vec![SearchWarning {
                    indexer: String::new(),
                    error: "All indexers failed".to_string(),
                }],
                cache_age_seconds: None,
            }));
        }
        Err(e) => return Err(e.into()),
    };

    let results = svc_response
        .results
        .into_iter()
        .map(|r| {
            let format = livrarr_matching::parse_release_title(&r.title)
                .format
                .map(|f| f.to_lowercase());
            ReleaseResponse {
                title: r.title,
                indexer: r.indexer,
                size: r.size,
                guid: r.guid,
                download_url: r.download_url,
                seeders: r.seeders,
                leechers: r.leechers,
                publish_date: r.publish_date,
                protocol: r.protocol.to_string(),
                categories: r.categories,
                format,
            }
        })
        .collect();

    let warnings = svc_response
        .warnings
        .into_iter()
        .map(|w| {
            let (indexer, error) = w
                .strip_prefix("indexer ")
                .and_then(|s| s.split_once(": "))
                .map(|(i, e)| (i.to_string(), e.to_string()))
                .unwrap_or_else(|| (String::new(), w));
            SearchWarning { indexer, error }
        })
        .collect();

    Ok(Json(ReleaseSearchResponse {
        results,
        warnings,
        cache_age_seconds: svc_response.cache_age_seconds,
    }))
}

pub async fn grab<S: HasReleaseService>(
    State(state): State<S>,
    ctx: AuthContext,
    Json(req): Json<GrabApiRequest>,
) -> Result<(), ApiError> {
    let protocol = match req.protocol.as_deref() {
        Some("usenet") => DownloadProtocol::Usenet,
        _ => DownloadProtocol::Torrent,
    };

    state
        .release_service()
        .grab(
            ctx.user.id,
            GrabRequest {
                work_id: req.work_id,
                download_url: req.download_url,
                title: req.title,
                indexer: req.indexer,
                guid: req.guid,
                size: req.size,
                protocol,
                categories: req.categories,
                download_client_id: req.download_client_id,
                source: GrabSource::Manual,
            },
        )
        .await?;

    Ok(())
}
