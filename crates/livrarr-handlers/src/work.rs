use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;

use axum::response::{IntoResponse, Response};

use crate::context::{
    HasAppConfigService, HasAuthorMonitorWorkflow, HasAuthorService, HasDiscoveryService,
    HasEmailService, HasEnrichmentWorkflow, HasFileService, HasHistoryService, HasHmacKey,
    HasIdentityLayerRepository, HasIdentityResolver, HasIdentityRoadService, HasImportService,
    HasNotificationService, HasSeriesQueryService, HasTagService, HasWorkIdentityRepository,
    HasWorkService,
};

use crate::middleware::RequireAdmin;
use crate::types::work::{merge_preview_to_response, work_to_detail};
use crate::{
    AddWorkRequest, AddWorkResponse, ApiError, AuthContext, DeleteWorkResponse, LookupApiResponse,
    MergePreviewResponse, MergeWorksRequest, MergeWorksResponse, RefreshWorkResponse,
    UpdateWorkRequest, WorkDetailResponse, WorkSearchResult,
};
use livrarr_domain::history_events;
use livrarr_domain::identity::{AnchorConfidence, AnchorSetter, AnchorType};
use livrarr_domain::identity_layer::IdentityRoadService;
use livrarr_domain::identity_layer::WorkIdentityRepository as _;
use livrarr_domain::services::{
    AppConfigService, AuthorService, CreateNotificationRequest, DiscoveryService, EmailService,
    FileService, HistoryService, ImportService, MergeFieldChoiceEntry, NotificationService,
    RefreshSurface, SeriesQueryService, WorkIdentityRepository, WorkService, WorkServiceError,
};

/// A composition-level claim covering the whole live-add background chain.
/// Nested service calls keep their own counted claims; this outer Drop closes
/// the signal on every normal return and every panic unwind.
struct BackgroundEnrichingGuard<S: HasWorkService> {
    state: S,
    user_id: i64,
    work_id: i64,
}

impl<S: HasWorkService> Drop for BackgroundEnrichingGuard<S> {
    fn drop(&mut self) {
        self.state
            .work_service()
            .end_enriching(self.user_id, self.work_id);
    }
}

/// Submit one fresh route handoff produced by the direct-add background
/// chain. Resolver capture, add completion, and the delayed refresh are all
/// machine continuations of the add, so they share `EnrichmentPass` origin.
async fn apply_add_background_handoff<S: HasIdentityRoadService>(
    state: &S,
    user_id: i64,
    work_id: i64,
    phase: &'static str,
    handoff: Option<livrarr_domain::identity_layer::CapturedRouteHandoff>,
) -> bool {
    let Some(handoff) = handoff else {
        return true;
    };
    match state
        .identity_road_service()
        .apply_captured_route_handoff(
            user_id,
            work_id,
            livrarr_domain::identity_layer::IdentityRoadOrigin::EnrichmentPass,
            handoff,
        )
        .await
    {
        Ok(Some(livrarr_domain::identity_layer::IdentityRoadOutcome::Settled { .. }))
        | Ok(None) => true,
        Ok(Some(other)) => {
            tracing::warn!(
                work_id,
                phase,
                ?other,
                "live-add identity route settlement parked"
            );
            false
        }
        Err(error) => {
            tracing::warn!(
                work_id,
                phase,
                "live-add identity route settlement failed: {error}"
            );
            false
        }
    }
}

fn proxy_cover_url(url: String) -> String {
    if url.starts_with('/') {
        return url;
    }
    format!("/api/v1/coverproxy?url={}", urlencoding::encode(&url))
}

/// Validate that image data begins with a recognized magic byte signature.
pub fn validate_image_magic_bytes(data: &[u8]) -> Result<(), ApiError> {
    const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF];
    const PNG_MAGIC: &[u8] = &[0x89, 0x50, 0x4E, 0x47];
    const WEBP_RIFF: &[u8] = b"RIFF";
    const WEBP_WEBP: &[u8] = b"WEBP";

    if data.len() < 12 {
        return Err(ApiError::BadRequest(
            "image data too small to identify format".into(),
        ));
    }
    if data.starts_with(JPEG_MAGIC) || data.starts_with(PNG_MAGIC) {
        return Ok(());
    }
    if data.starts_with(WEBP_RIFF) && data[8..12] == *WEBP_WEBP {
        return Ok(());
    }
    Err(ApiError::BadRequest(
        "unsupported image format: expected JPEG, PNG, or WebP".into(),
    ))
}

fn mime_for_ext(ext: &str) -> &'static str {
    match ext {
        "epub" => "application/epub+zip",
        "pdf" => "application/pdf",
        "mobi" => "application/x-mobipocket-ebook",
        "azw3" => "application/x-mobi8-ebook",
        "m4b" | "m4a" => "audio/mp4",
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        _ => "application/octet-stream",
    }
}

#[derive(serde::Deserialize)]
pub struct LookupQuery {
    pub term: Option<String>,
    pub lang: Option<String>,
    pub raw: Option<bool>,
}

#[derive(serde::Deserialize)]
pub struct StreamQuery {
    pub token: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct PreaddCoverQuery {
    pub title: String,
    pub author: Option<String>,
    pub lang: Option<String>,
    pub isbn_13: Option<String>,
}

pub async fn preadd_cover_alternatives<S>(
    State(state): State<S>,
    ctx: AuthContext,
    Query(q): Query<PreaddCoverQuery>,
) -> Result<Json<Vec<livrarr_domain::services::PreaddCoverCandidate>>, ApiError>
where
    S: crate::context::HasPreaddCoverService,
{
    if q.title.trim().is_empty() {
        return Err(ApiError::BadRequest("title is required".into()));
    }
    let author = q.author.as_deref().unwrap_or("");
    let lang = q.lang.as_deref().unwrap_or("en");
    let svc = state.preadd_cover_service();
    let candidates = livrarr_domain::services::PreaddCoverService::fetch_cover_alternatives(
        svc,
        ctx.user.id,
        q.title.trim(),
        author,
        lang,
        q.isbn_13.as_deref(),
    )
    .await
    .map_err(|e| ApiError::Internal(format!("cover alternatives: {e}")))?;
    Ok(Json(candidates))
}

pub async fn lookup<S: HasDiscoveryService>(
    State(state): State<S>,
    ctx: AuthContext,
    Query(q): Query<LookupQuery>,
) -> Result<Json<LookupApiResponse>, ApiError> {
    let req = livrarr_domain::services::LookupRequest {
        term: q.term.unwrap_or_default(),
        lang_override: q.lang,
    };
    let raw = q.raw.unwrap_or(false);

    let resp = state
        .discovery_service()
        .lookup_filtered(ctx.user.id, req, raw)
        .await?;

    let results = resp
        .results
        .into_iter()
        .map(|r| WorkSearchResult {
            ol_key: r.ol_key,
            title: r.title,
            author_name: r.author_name,
            author_ol_key: r.author_ol_key,
            year: r.year,
            cover_url: r.cover_url.map(proxy_cover_url),
            description: r.description,
            series_name: r.series_name,
            series_position: r.series_position,
            source: r.source,
            source_type: r.source_type,
            language: r.language,
            detail_url: r.detail_url,
            rating: r.rating,
            candidate_id: r.candidate_id,
            isbn_13: r.isbn_13,
            hc_key: r.hc_key,
            gr_key: r.gr_key,
            asin: r.asin,
        })
        .collect();

    Ok(Json(LookupApiResponse {
        results,
        filtered_count: resp.filtered_count,
        raw_count: resp.raw_count,
        raw_available: resp.raw_available,
    }))
}

pub async fn add<
    S: HasWorkService
        + HasAuthorService
        + HasSeriesQueryService
        + HasEnrichmentWorkflow
        + HasIdentityResolver
        + HasIdentityRoadService
        + HasIdentityLayerRepository
        + HasAppConfigService,
>(
    State(state): State<S>,
    ctx: AuthContext,
    Json(req): Json<AddWorkRequest>,
) -> Result<Json<AddWorkResponse>, ApiError> {
    use livrarr_domain::identity::RawHarvest;
    use livrarr_domain::seed::{seed_add_box, SeedInput, SeedLanguage};

    let default_language = state.app_config_service().get_default_language().await?;
    let language = SeedLanguage::resolve(req.language.as_deref(), &default_language);

    // The UI hands back the picked cover in its proxied display form
    // (`/api/v1/coverproxy?url=<encoded>`); persist the real provider URL so it
    // is a usable cover source (a proxied, leading-`/` value is dropped by the
    // DB cover-URL backstop and the manual pick never sticks).
    let cover_url = req
        .cover_url
        .as_deref()
        .map(livrarr_domain::unproxy_cover_url);
    let cover_is_manual = req.cover_manual && cover_url.is_some();

    // Local-only identity derivation (REQ-004): sanitize the harvest and take
    // the badge the seed itself supports — zero network before the response.
    // Same-book duplicates are caught by add_fast's local dedup (work-anchor,
    // verdict-gated bridge, normalized); provider-backed identity completion
    // and any resulting conflict surface through the background completion,
    // exactly like the batch doors.
    let resolved = state
        .work_service()
        .resolve_identity_local(RawHarvest {
            ol_key: req.ol_key.clone(),
            gr_key: req.gr_key.clone(),
            hc_key: req.hc_key.clone(),
            isbn: req.isbn_13.clone(),
            asin: req.asin.clone(),
            title: Some(req.title.clone()),
            author_name: Some(req.author_name.clone()),
            language: Some(language.as_str().to_string()),
            series_name: None,
            year: req.year,
            user_confirmed: true,
        })
        .map_err(|e| ApiError::Internal(format!("identity resolve: {e}")))?;
    let identity = resolved.identity;
    let candidate_id_for_completion = req.candidate_id.clone();

    let author_result = state
        .author_service()
        .add(
            ctx.user.id,
            livrarr_domain::services::AddAuthorRequest {
                name: req.author_name.clone(),
                sort_name: None,
                ol_key: req.author_ol_key.clone(),
                monitored: true,
            },
        )
        .await?;
    let author_id = author_result.author().id;
    // The canonical seed constructor is the normalization boundary for this
    // door. The road receives its parsed fields and sanitized anchors; the raw
    // HTTP payload is never reconstructed downstream.
    let candidate = seed_add_box(
        SeedInput {
            title: req.title,
            author_name: req.author_name,
            language,
            author_ol_key: req.author_ol_key,
            year: req.year,
            cover_url,
            detail_url: req.detail_url,
            description: None,
            series_name: None,
            series_position: None,
        },
        identity,
        req.candidate_id,
        cover_is_manual,
    );
    let provider_identity = candidate_provider_evidence(&candidate.identity);
    let minimum = livrarr_domain::identity_layer::MinimumWorkEvidence {
        title: candidate.fields.title.clone(),
        authors: vec![author_id],
    };
    let road_outcome = state
        .identity_road_service()
        .settle(livrarr_domain::identity_layer::IdentityRoadRequest {
            user_id: ctx.user.id,
            origin: livrarr_domain::identity_layer::IdentityRoadOrigin::CreationDoor(
                livrarr_domain::identity_layer::DoorKind::DirectAdd,
            ),
            evidence: livrarr_domain::identity_layer::IdentityEvidenceBundle {
                user_choice: Some(
                    livrarr_domain::identity_layer::UserIdentityChoice::ExplicitCreate(
                        minimum.clone(),
                    ),
                ),
                owned_files: Vec::new(),
                provider_identity,
                minimum: Some(minimum),
            },
            interaction: livrarr_domain::identity_layer::IdentityRoadInteraction::HumanWatching,
            existing_work_id: None,
        })
        .await
        .map_err(map_identity_road_error)?;
    let (road_work_id, road_created) = match road_outcome {
        livrarr_domain::identity_layer::IdentityRoadOutcome::Settled {
            work_id,
            created,
            status,
            ..
        } => {
            let _ = status;
            (work_id, created)
        }
        livrarr_domain::identity_layer::IdentityRoadOutcome::ReviewPending {
            review_id, ..
        } => {
            return Err(ApiError::Conflict {
                reason: format!("direct add requires review card {review_id}"),
            });
        }
        livrarr_domain::identity_layer::IdentityRoadOutcome::Deferred { reason } => {
            return Err(ApiError::Conflict { reason: reason.0 });
        }
        livrarr_domain::identity_layer::IdentityRoadOutcome::Rejected { reason } => {
            return Err(map_identity_road_error(reason));
        }
    };

    let work = state.work_service().get(ctx.user.id, road_work_id).await?;
    let result = livrarr_domain::services::AddWorkResult {
        enrichment_status: work.enrichment_status,
        work,
        created: road_created,
        author_created: author_result.is_created(),
        author_id: Some(author_id),
        messages: Vec::new(),
        cover_mtime: None,
        audiobook_cover_mtime: None,
    };

    // Background completion (REQ-004): identity fan-out + enrichment + covers
    // run off the response path; the +5s anchor top-up refresh chains AFTER
    // completion so two enrichment runs never race on one work.
    if result.created {
        let s = state.clone();
        let uid = ctx.user.id;
        let wid = result.work.id;
        // Claim synchronously before spawning so the first detail poll cannot
        // observe a false gap ahead of identity capture.
        state.work_service().begin_enriching(uid, wid);
        tokio::spawn(async move {
            let _background_enriching = BackgroundEnrichingGuard {
                state: s.clone(),
                user_id: uid,
                work_id: wid,
            };
            let route_handoff = match s
                .work_service()
                .capture_add_identity_route_handoff(
                    uid,
                    wid,
                    livrarr_domain::identity::IdentityMode::Interactive,
                )
                .await
            {
                Ok(handoff) => handoff,
                Err(error) => {
                    tracing::warn!(wid, "live-add identity capture failed: {error}");
                    return;
                }
            };
            if !apply_add_background_handoff(&s, uid, wid, "resolver-capture", route_handoff).await
            {
                return;
            }
            let completion_handoff = s
                .work_service()
                .complete_add(
                    uid,
                    wid,
                    None,
                    candidate_id_for_completion,
                    livrarr_domain::identity::IdentityMode::Interactive,
                    livrarr_domain::identity::ConflictSource::ManualAdd,
                )
                .await;
            if !apply_add_background_handoff(&s, uid, wid, "complete-add", completion_handoff).await
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            match s
                .work_service()
                .refresh(uid, wid, RefreshSurface::Interactive)
                .await
            {
                Ok(mut result) => {
                    let _ = apply_add_background_handoff(
                        &s,
                        uid,
                        wid,
                        "delayed-refresh",
                        result.route_handoff.take(),
                    )
                    .await;
                }
                Err(error) => {
                    tracing::warn!(wid, "live-add delayed refresh failed: {error}");
                }
            }
        });
    }

    if result.author_created {
        if let Some(author_id) = result.author_id {
            let s = state.clone();
            let user_id = ctx.user.id;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if let Err(e) = s
                    .author_service()
                    .refresh_bibliography(user_id, author_id)
                    .await
                {
                    tracing::debug!(author_id, "background bibliography fetch skipped: {e}");
                }
            });

            // Goodreads candidate discovery for the author picker. It links
            // nothing: a name-similarity score is not proof of identity, so the
            // author's Goodreads route stays the user's choice (REQ-004/AC-005).
            let s_gr = state.clone();
            let uid = ctx.user.id;
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                if let Err(e) = s_gr
                    .series_query_service()
                    .resolve_gr_candidates(uid, author_id)
                    .await
                {
                    tracing::debug!(author_id, "background GR resolve skipped: {e}");
                }
            });
        }
    }

    let mut detail = crate::types::work::work_to_detail_with_cover_mtime(
        &result.work,
        result.cover_mtime,
        result.audiobook_cover_mtime,
    );
    project_work_identity_presentations(&state, ctx.user.id, std::slice::from_mut(&mut detail))
        .await?;
    // A created work has its completion running right now (spawned above) —
    // report it directly rather than racing the registry's first insert.
    detail.enriching = result.created;
    Ok(Json(AddWorkResponse {
        work: detail,
        created: result.created,
        author_created: result.author_created,
        messages: result.messages,
    }))
}

fn candidate_provider_evidence(
    identity: &livrarr_domain::identity::IdentityState,
) -> Vec<livrarr_domain::identity_layer::ProviderIdentityEvidence> {
    use livrarr_domain::identity_layer::{
        IdentityProvider, ProviderIdentityEvidence, RouteKey, RouteKind,
    };

    let Some(anchors) = identity.seed_or_confirmed_anchors() else {
        return Vec::new();
    };
    [
        anchors.ol_key.as_ref().map(|value| {
            (
                IdentityProvider::OpenLibrary,
                RouteKind::OpenLibraryWork,
                value,
            )
        }),
        anchors
            .gr_key
            .as_ref()
            .map(|value| (IdentityProvider::Goodreads, RouteKind::GoodreadsWork, value)),
        anchors
            .hc_key
            .as_ref()
            .map(|value| (IdentityProvider::Hardcover, RouteKind::HardcoverWork, value)),
        anchors.isbn_13.as_ref().map(|value| {
            (
                IdentityProvider::IsbnRegistry,
                RouteKind::Isbn13Edition,
                value,
            )
        }),
        anchors
            .asin
            .as_ref()
            .map(|value| (IdentityProvider::Amazon, RouteKind::AsinEdition, value)),
    ]
    .into_iter()
    .flatten()
    .map(|(provider, kind, value)| ProviderIdentityEvidence {
        provider: provider.clone(),
        route: RouteKey {
            provider,
            kind,
            value: value.clone(),
        },
        work_core: None,
        provenance: Default::default(),
    })
    .collect()
}

fn map_identity_road_error(error: livrarr_domain::identity_layer::IdentityRoadError) -> ApiError {
    use livrarr_domain::identity_layer::IdentityRoadError;
    match error {
        IdentityRoadError::NotFound => ApiError::NotFound,
        IdentityRoadError::StaleGeneration
        | IdentityRoadError::ReviewProposalInvalidated(_)
        | IdentityRoadError::ReviewRequired
        | IdentityRoadError::ProbeBlocked(_) => ApiError::Conflict {
            reason: error.to_string(),
        },
        IdentityRoadError::UnauthorizedScope => ApiError::Forbidden,
        IdentityRoadError::InvalidDoorEvidence
        | IdentityRoadError::ReviewKindMismatch
        | IdentityRoadError::InvalidResolution => ApiError::BadRequest(error.to_string()),
        IdentityRoadError::ProviderBoundary => ApiError::BadGateway(error.to_string()),
        IdentityRoadError::Cancelled => ApiError::ServiceUnavailable,
        IdentityRoadError::Database(message) => ApiError::Internal(message),
    }
}

pub async fn list<S: HasWorkService + HasFileService + HasIdentityLayerRepository>(
    State(state): State<S>,
    ctx: AuthContext,
    Query(pq): Query<crate::PaginationQuery>,
) -> Result<Json<crate::PaginatedResponse<WorkDetailResponse>>, ApiError> {
    let view = state
        .work_service()
        .list_paginated(
            ctx.user.id,
            pq.page(),
            pq.page_size(),
            pq.sort_by(),
            pq.sort_dir(),
            pq.media_type,
            pq.language.as_deref(),
        )
        .await?;

    let work_ids: Vec<i64> = view.works.iter().map(|work| work.work.id).collect();
    let identity_presentations: std::collections::HashMap<_, _> = state
        .identity_layer_repository()
        .read_identity_presentations(ctx.user.id, &work_ids)
        .await
        .map_err(map_identity_repository_error)?
        .into_iter()
        .map(|presentation| (presentation.work_id, presentation))
        .collect();

    let all_item_ids: Vec<i64> = view
        .works
        .iter()
        .flat_map(|w| w.library_items.iter().map(|li| li.id))
        .collect();

    let progress_list = state
        .file_service()
        .get_progress_for_items(ctx.user.id, &all_item_ids)
        .await
        .unwrap_or_default();

    let progress_map: std::collections::HashMap<i64, &livrarr_domain::services::ItemProgress> =
        progress_list
            .iter()
            .map(|p| (p.library_item_id, p))
            .collect();

    let items = view
        .works
        .into_iter()
        .map(|wv| {
            let work_duration = wv.work.duration_seconds.map(|d| d as f64);
            let mut detail = crate::types::work::work_to_detail_with_cover_mtime(
                &wv.work,
                wv.cover_mtime,
                wv.audiobook_cover_mtime,
            );
            if let Some(presentation) = identity_presentations.get(&wv.work.id) {
                crate::types::work::apply_work_identity_presentation(&mut detail, presentation);
            }
            detail.library_items = wv
                .library_items
                .iter()
                .map(|li| {
                    let prog = progress_map.get(&li.id);
                    crate::LibraryItemResponse {
                        id: li.id,
                        path: li.path.clone(),
                        media_type: li.media_type,
                        file_size: li.file_size,
                        imported_at: li.imported_at.to_rfc3339(),
                        progress_pct: prog.map(|p| p.progress_pct),
                        duration_seconds: li.duration_seconds.or(work_duration),
                        finished_at: prog.and_then(|p| p.finished_at.map(|d| d.to_rfc3339())),
                    }
                })
                .collect();
            detail
        })
        .collect();

    Ok(Json(crate::PaginatedResponse {
        items,
        total: view.total,
        page: view.page,
        page_size: view.page_size,
    }))
}

pub async fn get<S: HasWorkService + HasFileService + HasIdentityLayerRepository>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<WorkDetailResponse>, ApiError> {
    let view = state.work_service().get_detail(ctx.user.id, id).await?;

    let item_ids: Vec<i64> = view.library_items.iter().map(|li| li.id).collect();
    let progress_list = state
        .file_service()
        .get_progress_for_items(ctx.user.id, &item_ids)
        .await
        .unwrap_or_default();
    let progress_map: std::collections::HashMap<i64, &livrarr_domain::services::ItemProgress> =
        progress_list
            .iter()
            .map(|p| (p.library_item_id, p))
            .collect();

    let work_duration = view.work.duration_seconds.map(|d| d as f64);
    let mut detail = crate::types::work::work_to_detail_with_cover_mtime(
        &view.work,
        view.cover_mtime,
        view.audiobook_cover_mtime,
    );
    match state
        .identity_layer_repository()
        .read_captured_identity(ctx.user.id, id)
        .await
    {
        Ok(captured) => {
            let siblings = state
                .identity_layer_repository()
                .list_captured_identities_in_group(
                    ctx.user.id,
                    captured.identity_title.normalized_main.clone(),
                    captured.primary_author_id,
                )
                .await
                .map_err(map_identity_repository_error)?;
            let author_name = state
                .identity_layer_repository()
                .read_primary_author_names(ctx.user.id, captured.primary_author_id)
                .await
                .map_err(map_identity_repository_error)?
                .into_iter()
                .next()
                .unwrap_or_else(|| view.work.author_name.clone());
            crate::types::work::apply_identity_presentation(
                &mut detail,
                &view.work,
                &captured,
                siblings,
                author_name,
            );
        }
        Err(livrarr_domain::identity_layer::IdentityRepositoryError::NotFound) => {}
        Err(error) => return Err(map_identity_repository_error(error)),
    }
    detail.enriching = state.work_service().is_enriching(ctx.user.id, id);
    detail.library_items = view
        .library_items
        .iter()
        .map(|li| {
            let prog = progress_map.get(&li.id);
            crate::LibraryItemResponse {
                id: li.id,
                path: li.path.clone(),
                media_type: li.media_type,
                file_size: li.file_size,
                imported_at: li.imported_at.to_rfc3339(),
                progress_pct: prog.map(|p| p.progress_pct),
                duration_seconds: li.duration_seconds.or(work_duration),
                finished_at: prog.and_then(|p| p.finished_at.map(|d| d.to_rfc3339())),
            }
        })
        .collect();

    Ok(Json(detail))
}

pub async fn update<
    S: HasWorkService + HasAuthorService + HasIdentityRoadService + HasIdentityLayerRepository,
>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    Json(req): Json<UpdateWorkRequest>,
) -> Result<Response, ApiError> {
    use crate::types::api_error::FieldError;
    use livrarr_domain::services::UpdateWorkRequest as DomainUpdateWorkRequest;

    let mut errors = Vec::new();
    if matches!(req.title, Some(None)) {
        errors.push(FieldError {
            field: "title".into(),
            message: "cannot be null".into(),
        });
    }
    if matches!(req.author_name, Some(None)) {
        errors.push(FieldError {
            field: "authorName".into(),
            message: "cannot be null".into(),
        });
    }
    if matches!(req.monitor_ebook, Some(None)) {
        errors.push(FieldError {
            field: "monitorEbook".into(),
            message: "cannot be null".into(),
        });
    }
    if matches!(req.monitor_audiobook, Some(None)) {
        errors.push(FieldError {
            field: "monitorAudiobook".into(),
            message: "cannot be null".into(),
        });
    }
    if let Some(Some(ref t)) = req.title {
        if t.trim().is_empty() {
            errors.push(FieldError {
                field: "title".into(),
                message: "cannot be empty".into(),
            });
        }
    }
    if let Some(Some(ref a)) = req.author_name {
        if a.trim().is_empty() {
            errors.push(FieldError {
                field: "authorName".into(),
                message: "cannot be empty".into(),
            });
        }
    }
    if !errors.is_empty() {
        return Err(ApiError::Validation { errors });
    }

    if req.title.is_some() || req.author_name.is_some() {
        let current = state.work_service().get(ctx.user.id, id).await?;
        let requested_title = req.title.flatten().unwrap_or_else(|| current.title.clone());
        let requested_author = req
            .author_name
            .flatten()
            .unwrap_or_else(|| current.author_name.clone());
        let author = state
            .author_service()
            .add(
                ctx.user.id,
                livrarr_domain::services::AddAuthorRequest {
                    name: requested_author,
                    sort_name: None,
                    ol_key: None,
                    monitored: true,
                },
            )
            .await?
            .into_author();
        let outcome = state
            .identity_road_service()
            .settle(livrarr_domain::identity_layer::IdentityRoadRequest {
                user_id: ctx.user.id,
                origin: livrarr_domain::identity_layer::IdentityRoadOrigin::WorkUpdateRekey,
                evidence: livrarr_domain::identity_layer::IdentityEvidenceBundle {
                    user_choice: None,
                    owned_files: Vec::new(),
                    provider_identity: Vec::new(),
                    minimum: Some(livrarr_domain::identity_layer::MinimumWorkEvidence {
                        title: requested_title,
                        authors: vec![author.id],
                    }),
                },
                interaction: livrarr_domain::identity_layer::IdentityRoadInteraction::HumanWatching,
                existing_work_id: Some(id),
            })
            .await
            .map_err(map_identity_road_error)?;
        let (card_id, expected_generation) = pending_review_claim(outcome)?;
        state
            .identity_road_service()
            .resolve_review(
                livrarr_domain::identity_layer::ReviewActor::AuthenticatedUser {
                    user_id: ctx.user.id,
                },
                livrarr_domain::identity_layer::ReviewResolutionCommand::GroupIdentity {
                    card_id,
                    expected_generation,
                    action: livrarr_domain::identity_layer::GroupIdentityAction::DifferentFromAll,
                },
            )
            .await
            .map_err(map_identity_road_error)?;
        let work = state.work_service().get(ctx.user.id, id).await?;
        let mut detail = work_to_detail(&work);
        project_work_identity_presentations(&state, ctx.user.id, std::slice::from_mut(&mut detail))
            .await?;
        return Ok(Json(detail).into_response());
    }

    let work = state
        .work_service()
        .update(
            ctx.user.id,
            id,
            DomainUpdateWorkRequest {
                title: req.title.flatten(),
                author_name: req.author_name.flatten(),
                series_name: req.series_name,
                series_position: req.series_position,
                monitor_ebook: req.monitor_ebook.flatten(),
                monitor_audiobook: req.monitor_audiobook.flatten(),
            },
        )
        .await?;

    let mut detail = work_to_detail(&work);
    project_work_identity_presentations(&state, ctx.user.id, std::slice::from_mut(&mut detail))
        .await?;
    Ok(Json(detail).into_response())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PendingIdentityReviewResponse {
    card_id: i64,
    kind: livrarr_domain::identity_layer::ReviewKind,
    expected_generation: i64,
    provenance: &'static str,
}

fn pending_review_claim(
    outcome: livrarr_domain::identity_layer::IdentityRoadOutcome,
) -> Result<(i64, i64), ApiError> {
    match outcome {
        livrarr_domain::identity_layer::IdentityRoadOutcome::ReviewPending {
            review_id,
            expected_generation,
            ..
        } => Ok((review_id, expected_generation)),
        other => Err(ApiError::Internal(format!(
            "identity continuation did not originate a review: {other:?}"
        ))),
    }
}

fn pending_review_response(
    outcome: livrarr_domain::identity_layer::IdentityRoadOutcome,
) -> Result<PendingIdentityReviewResponse, ApiError> {
    match outcome {
        livrarr_domain::identity_layer::IdentityRoadOutcome::ReviewPending {
            review_id,
            kind,
            expected_generation,
            provenance,
            ..
        } => Ok(PendingIdentityReviewResponse {
            card_id: review_id,
            kind,
            expected_generation,
            provenance: match provenance {
                livrarr_domain::identity_layer::EvidenceProvenance::User => "User",
                livrarr_domain::identity_layer::EvidenceProvenance::OwnedFile => "OwnedFile",
                livrarr_domain::identity_layer::EvidenceProvenance::Provider(_) => "Provider",
                livrarr_domain::identity_layer::EvidenceProvenance::Migrated => "Migrated",
            },
        }),
        other => Err(ApiError::Internal(format!(
            "identity continuation did not originate a review: {other:?}"
        ))),
    }
}

fn map_identity_repository_error(
    error: livrarr_domain::identity_layer::IdentityRepositoryError,
) -> ApiError {
    use livrarr_domain::identity_layer::IdentityRepositoryError;
    match error {
        IdentityRepositoryError::NotFound => ApiError::NotFound,
        IdentityRepositoryError::StaleGeneration
        | IdentityRepositoryError::ReviewProposalInvalidated(_)
        | IdentityRepositoryError::RouteOwnershipCollision
        | IdentityRepositoryError::KeyCollision
        | IdentityRepositoryError::StillAmbiguous
        | IdentityRepositoryError::ReviewKindMismatch => ApiError::Conflict {
            reason: error.to_string(),
        },
        IdentityRepositoryError::UnauthorizedScope => ApiError::Forbidden,
        IdentityRepositoryError::InvalidResolution => ApiError::BadRequest(error.to_string()),
        IdentityRepositoryError::Cancelled => ApiError::ServiceUnavailable,
        IdentityRepositoryError::AtomicRollback | IdentityRepositoryError::Database(_) => {
            ApiError::Internal(error.to_string())
        }
    }
}

pub(crate) async fn project_work_identity_presentations<S: HasIdentityLayerRepository>(
    state: &S,
    user_id: livrarr_domain::UserId,
    details: &mut [WorkDetailResponse],
) -> Result<(), ApiError> {
    let work_ids: Vec<_> = details.iter().map(|detail| detail.id).collect();
    let presentations = read_work_identity_presentations(state, user_id, &work_ids).await?;
    for detail in details {
        if let Some(presentation) = presentations.get(&detail.id) {
            crate::types::work::apply_work_identity_presentation(detail, presentation);
        }
    }
    Ok(())
}

pub(crate) async fn read_work_identity_presentations<S: HasIdentityLayerRepository>(
    state: &S,
    user_id: livrarr_domain::UserId,
    work_ids: &[livrarr_domain::WorkId],
) -> Result<
    std::collections::HashMap<
        livrarr_domain::WorkId,
        livrarr_domain::identity_layer::WorkIdentityPresentation,
    >,
    ApiError,
> {
    Ok(state
        .identity_layer_repository()
        .read_identity_presentations(user_id, work_ids)
        .await
        .map_err(map_identity_repository_error)?
        .into_iter()
        .map(|presentation| (presentation.work_id, presentation))
        .collect())
}

pub async fn upload_cover<S: HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    mut multipart: axum::extract::Multipart,
) -> Result<(), ApiError> {
    let mut image_data: Option<Bytes> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::BadRequest(format!("multipart error: {e}")))?
    {
        if field.name() == Some("image_data") {
            image_data = Some(
                field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::BadRequest(format!("read field: {e}")))?,
            );
            break;
        }
    }
    let data = image_data.ok_or_else(|| ApiError::BadRequest("missing image_data field".into()))?;
    validate_image_magic_bytes(&data)?;
    state
        .work_service()
        .upload_cover(ctx.user.id, id, &data)
        .await?;
    Ok(())
}

pub async fn delete<S: HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<DeleteWorkResponse>, ApiError> {
    state.work_service().delete(ctx.user.id, id).await?;
    Ok(Json(DeleteWorkResponse { warnings: vec![] }))
}

/// Preview combining `loser_id` into `id` (the survivor) without applying
/// anything (REQ-015 b). Both works must belong to the caller — enforced by
/// `WorkService::preview_merge_works` itself, not just here (REQ-015 a).
pub async fn preview_merge<S: HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path((id, loser_id)): Path<(i64, i64)>,
) -> Result<Json<MergePreviewResponse>, ApiError> {
    let preview = state
        .work_service()
        .preview_merge_works(ctx.user.id, id, loser_id)
        .await?;
    Ok(Json(merge_preview_to_response(preview)))
}

/// Combine `loser_id` into `id` (the survivor): reassigns library items and
/// grabs, resolves user-sovereign fields, then removes the loser row — all
/// in one transaction (REQ-015 c/e). Physical file reorganization under the
/// survivor's canonical path is a separate, best-effort follow-up: it never
/// blocks or reverses the transaction above, which is the guarantee the
/// user actually needs (items/grabs moved, loser gone, zero deletions).
pub async fn merge<
    S: HasWorkService + HasImportService + HasIdentityLayerRepository + HasIdentityRoadService,
>(
    State(state): State<S>,
    ctx: AuthContext,
    Path((id, loser_id)): Path<(i64, i64)>,
    Json(req): Json<MergeWorksRequest>,
) -> Result<Response, ApiError> {
    let choices = req
        .choices
        .into_iter()
        .map(|c| MergeFieldChoiceEntry {
            field: c.field,
            choice: c.choice,
        })
        .collect::<Vec<_>>();
    let current = state.work_service().get(ctx.user.id, id).await?;
    // User-scope both sides before the road creates a card.
    state.work_service().get(ctx.user.id, loser_id).await?;
    let captured = state
        .identity_layer_repository()
        .read_captured_identity(ctx.user.id, id)
        .await
        .map_err(map_identity_repository_error)?;
    let outcome = state
        .identity_road_service()
        .settle(livrarr_domain::identity_layer::IdentityRoadRequest {
            user_id: ctx.user.id,
            origin: livrarr_domain::identity_layer::IdentityRoadOrigin::ManualWorkMerge {
                loser_work_id: loser_id,
                choices: choices.clone(),
            },
            evidence: livrarr_domain::identity_layer::IdentityEvidenceBundle {
                user_choice: None,
                owned_files: Vec::new(),
                provider_identity: Vec::new(),
                minimum: Some(livrarr_domain::identity_layer::MinimumWorkEvidence {
                    title: current.title,
                    authors: vec![captured.primary_author_id],
                }),
            },
            interaction: livrarr_domain::identity_layer::IdentityRoadInteraction::HumanWatching,
            existing_work_id: Some(id),
        })
        .await
        .map_err(map_identity_road_error)?;
    if choices.is_empty() {
        let card = pending_review_response(outcome)?;
        return Ok((StatusCode::ACCEPTED, Json(card)).into_response());
    }
    let (card_id, expected_generation) = pending_review_claim(outcome)?;
    let resolved = state
        .identity_road_service()
        .resolve_review(
            livrarr_domain::identity_layer::ReviewActor::AuthenticatedUser {
                user_id: ctx.user.id,
            },
            livrarr_domain::identity_layer::ReviewResolutionCommand::GroupIdentity {
                card_id,
                expected_generation,
                action: livrarr_domain::identity_layer::GroupIdentityAction::AttachOrMerge {
                    anchor: id,
                },
            },
        )
        .await
        .map_err(map_identity_road_error)?;
    let (library_items_moved, grabs_moved) = match resolved {
        livrarr_domain::identity_layer::IdentityRoadOutcome::Settled {
            library_items_moved,
            grabs_moved,
            ..
        } => (library_items_moved, grabs_moved),
        livrarr_domain::identity_layer::IdentityRoadOutcome::ReviewPending {
            review_id, ..
        } => {
            return Err(ApiError::Conflict {
                reason: format!("manual merge still requires review card {review_id}"),
            });
        }
        livrarr_domain::identity_layer::IdentityRoadOutcome::Deferred { reason } => {
            return Err(ApiError::Conflict { reason: reason.0 });
        }
        livrarr_domain::identity_layer::IdentityRoadOutcome::Rejected { reason } => {
            return Err(map_identity_road_error(reason));
        }
    };

    let reorg_warnings = state
        .import_service()
        .reorganize_work_files(ctx.user.id, id)
        .await
        .unwrap_or_else(|e| vec![format!("file reorganization skipped: {e}")]);

    let survivor = state.work_service().get(ctx.user.id, id).await?;
    let mut survivor = work_to_detail(&survivor);
    project_work_identity_presentations(&state, ctx.user.id, std::slice::from_mut(&mut survivor))
        .await?;

    Ok(Json(MergeWorksResponse {
        survivor,
        library_items_moved,
        grabs_moved,
        warnings: reorg_warnings,
    })
    .into_response())
}

pub async fn refresh<S: HasWorkService + HasIdentityRoadService + HasIdentityLayerRepository>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<RefreshWorkResponse>, ApiError> {
    // WorkService::refresh() runs the unified enrichment pipeline:
    // provider dispatch, merge, cover download, and tag sync are all handled inside.
    let mut result = state
        .work_service()
        .refresh(ctx.user.id, id, RefreshSurface::Interactive)
        .await?;

    if let Some(handoff) = result.route_handoff.take() {
        state
            .identity_road_service()
            .apply_captured_route_handoff(
                ctx.user.id,
                id,
                livrarr_domain::identity_layer::IdentityRoadOrigin::ManualRefresh,
                handoff,
            )
            .await
            .map_err(map_identity_road_error)?;
    }

    let failure_reason = result
        .provider_unavailable
        .then_some(crate::types::work::RefreshFailureReason::ProviderUnavailable);
    let mut detail = work_to_detail(&result.work);
    project_work_identity_presentations(&state, ctx.user.id, std::slice::from_mut(&mut detail))
        .await?;
    Ok(Json(RefreshWorkResponse {
        work: detail,
        messages: result.messages,
        reason: failure_reason,
    }))
}

/// The Refresh-All sweep body (REQ-013): every work refreshes through the
/// Bulk surface; a per-work failure is counted and never aborts the sweep.
/// Concurrency is bounded at 3 in-flight refreshes (AC-019) — the win is
/// overlapping different providers across works; per-provider pacing stays
/// governed by the outbound queue (ST-012). refresh() funnels through
/// run_unified, which materializes covers and tags itself — the sweep does
/// not re-download or re-tag (REQ-001).
pub async fn bulk_refresh_sweep<W: livrarr_domain::services::WorkService>(
    work_service: &W,
    user_id: livrarr_domain::UserId,
    works: Vec<livrarr_domain::Work>,
) -> (usize, usize) {
    use futures::StreamExt;

    let results: Vec<bool> = futures::stream::iter(works)
        .map(|work| async move {
            match work_service
                .refresh(user_id, work.id, RefreshSurface::Bulk)
                .await
            {
                Ok(_) => true,
                Err(e) => {
                    tracing::warn!(work_id = work.id, "refresh_all: refresh failed: {e}");
                    false
                }
            }
        })
        .buffer_unordered(3)
        .collect()
        .await;

    let enriched = results.iter().filter(|ok| **ok).count();
    (enriched, results.len() - enriched)
}

/// Active library filters carried into Refresh All (REQ-015): the sweep
/// refreshes what the user sees. No params = all works (AC-017).
#[derive(serde::Deserialize)]
pub struct RefreshAllParams {
    pub language: Option<String>,
    pub monitored: Option<bool>,
    pub enrichment_status: Option<livrarr_domain::EnrichmentStatus>,
    pub media_type: Option<livrarr_domain::MediaType>,
}

pub async fn refresh_all<S: HasWorkService + HasTagService + HasNotificationService>(
    State(state): State<S>,
    ctx: AuthContext,
    Query(params): Query<RefreshAllParams>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user_id = ctx.user.id;

    let Some(bulk_guard) = state.work_service().try_start_bulk_refresh(user_id) else {
        return Err(ApiError::Conflict {
            reason: "Refresh already in progress".to_string(),
        });
    };

    let works = state
        .work_service()
        .list(
            user_id,
            livrarr_domain::services::WorkFilter {
                author_id: None,
                monitored: params.monitored,
                enrichment_status: params.enrichment_status,
                media_type: params.media_type,
                language: params.language,
                sort_by: None,
                sort_dir: None,
            },
        )
        .await
        .map_err(ApiError::from)?;

    if works.is_empty() {
        return Ok(axum::http::StatusCode::ACCEPTED);
    }

    let total = works.len();
    let s = state.clone();
    tokio::spawn(async move {
        // Owns the slot: completion, error, panic unwind, and abort all
        // release via Drop (REQ-016).
        let _bulk_guard = bulk_guard;
        let (enriched, failed) = bulk_refresh_sweep(s.work_service(), user_id, works).await;

        if let Err(e) = s
            .notification_service()
            .create(CreateNotificationRequest {
                user_id,
                notification_type: livrarr_domain::NotificationType::BulkEnrichmentComplete,
                ref_key: None,
                message: format!(
                    "Bulk refresh complete: {enriched}/{total} enriched, {failed} failed"
                ),
                data: serde_json::json!({
                    "total": total,
                    "enriched": enriched,
                    "failed": failed,
                }),
            })
            .await
        {
            tracing::warn!("create_notification failed: {e}");
        }
    });

    Ok(axum::http::StatusCode::ACCEPTED)
}

/// User-triggered bulk recovery (REQ-011): sweep every incomplete work — Failed,
/// Unenriched, or identity-Pending — and re-run each through the one road in a
/// single pass. Replaces the deleted background `enrichment_retry` job (REQ-001):
/// recovery is now an explicit user action, not a recurring loop. Shares the
/// `try_start_bulk_refresh` guard with `refresh_all` so only one bulk sweep runs
/// per user at a time; the work happens in a spawned one-shot and the response is
/// an immediate 202.
pub async fn retry_all_incomplete<
    S: HasWorkService + HasNotificationService + HasIdentityRoadService + HasIdentityLayerRepository,
>(
    State(state): State<S>,
    ctx: AuthContext,
) -> Result<axum::http::StatusCode, ApiError> {
    let user_id = ctx.user.id;

    let Some(bulk_guard) = state.work_service().try_start_bulk_refresh(user_id) else {
        return Err(ApiError::Conflict {
            reason: "Bulk operation already in progress".to_string(),
        });
    };

    let s = state.clone();
    tokio::spawn(async move {
        // Owns the slot: every exit path releases via Drop (REQ-016).
        let _bulk_guard = bulk_guard;
        match s.work_service().retry_all_incomplete(user_id).await {
            Ok(mut summary) => {
                for (work_id, handoff) in std::mem::take(&mut summary.route_handoffs) {
                    if let Err(error) = s
                        .identity_road_service()
                        .apply_captured_route_handoff(
                            user_id,
                            work_id,
                            livrarr_domain::identity_layer::IdentityRoadOrigin::ConvergenceVisit,
                            handoff,
                        )
                        .await
                    {
                        tracing::warn!(work_id, "retry route handoff failed: {error}");
                    }
                }
                if let Err(e) = s
                    .notification_service()
                    .create(CreateNotificationRequest {
                        user_id,
                        notification_type: livrarr_domain::NotificationType::BulkEnrichmentComplete,
                        ref_key: None,
                        message: format!(
                            "Retry complete: {}/{} recovered, {} still incomplete",
                            summary.recovered, summary.total, summary.still_incomplete
                        ),
                        data: serde_json::json!({
                            "total": summary.total,
                            "recovered": summary.recovered,
                            "still_incomplete": summary.still_incomplete,
                        }),
                    })
                    .await
                {
                    tracing::warn!("create_notification failed: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("retry_all_incomplete failed: {e}");
            }
        }
    });

    Ok(axum::http::StatusCode::ACCEPTED)
}

pub async fn send_email<S: HasFileService + HasEmailService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let payload = state.file_service().prepare_email(ctx.user.id, id).await?;

    state
        .email_service()
        .send_file(payload.file_bytes, &payload.filename, &payload.extension)
        .await
        .map_err(|e| {
            tracing::error!("Email send failed: {e}");
            ApiError::Internal(e.to_string())
        })?;

    tracing::info!(file = %payload.filename, "Email sent");
    Ok(Json(serde_json::json!({ "success": true })))
}

async fn serve_library_file(
    path: std::path::PathBuf,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Response, ApiError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let content_type = mime_for_ext(&ext);

    use tower::Service;
    use tower_http::services::ServeFile;
    let mut svc = ServeFile::new(&path);
    let resp = svc
        .call(req)
        .await
        .map_err(|e| ApiError::Internal(format!("File serve error: {e}")))?;

    let (mut parts, body) = resp.into_response().into_parts();
    parts.headers.insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(content_type),
    );
    Ok(Response::from_parts(parts, body))
}

pub async fn download<S: HasFileService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(id): Path<i64>,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Response, ApiError> {
    let path = state.file_service().resolve_path(ctx.user.id, id).await?;
    serve_library_file(path, req).await
}

pub async fn stream<S: HasFileService + HasHmacKey>(
    State(state): State<S>,
    Path(id): Path<i64>,
    Query(params): Query<StreamQuery>,
    req: axum::http::Request<axum::body::Body>,
) -> Result<Response, ApiError> {
    // Unit C: the raw session token is gone from this query param — it's
    // now a short-lived, scoped stream token (see `crate::stream_token`).
    // `verify_stream_token` recovers `user_id` from the token itself (never
    // trusted from a query/user param) and confirms the token's own
    // `item_id` claim matches the item being requested here.
    let token = params.token.as_deref().ok_or(ApiError::Unauthorized)?;
    let user_id =
        crate::stream_token::verify_stream_token(state.hmac_key(), token, id, chrono::Utc::now())
            .map_err(|_| ApiError::Unauthorized)?;

    let path = state.file_service().resolve_path(user_id, id).await?;
    serve_library_file(path, req).await
}

pub async fn author_search<S: HasAuthorMonitorWorkflow>(
    State(state): State<S>,
    ctx: AuthContext,
    _admin: RequireAdmin,
) -> axum::http::StatusCode {
    let s = state.clone();
    let user_id = ctx.user.id;
    tokio::spawn(async move {
        use livrarr_domain::services::{AuthorMonitorWorkflow, MonitorError};
        let cancel = tokio_util::sync::CancellationToken::new();
        match s
            .author_monitor_workflow()
            .run_monitor(user_id, cancel)
            .await
        {
            Ok(_) => {}
            Err(MonitorError::AlreadyRunning) => {}
            Err(e) => tracing::error!("manual author search failed for user {user_id}: {e}"),
        }
    });
    axum::http::StatusCode::ACCEPTED
}

/// Anchor types already settled for this work: a confirmed legacy-ledger row
/// or an active identity-v2 route projected into the compatibility slots. A
/// pending guess for a settled slot is never offered and never affirmable.
fn settled_anchor_types(
    identifiers: &livrarr_domain::identity_layer::WorkIdentifierProjection,
    anchors: &[livrarr_domain::identity::WorkIdentityAnchor],
) -> std::collections::HashSet<String> {
    let mut settled: std::collections::HashSet<String> = anchors
        .iter()
        .filter(|a| a.confidence == AnchorConfidence::Confirmed)
        .map(|a| a.anchor_type.as_str().to_string())
        .collect();
    for (anchor_type, value) in [
        (AnchorType::OL_WORK, identifiers.ol_key.as_deref()),
        (AnchorType::GR_WORK, identifiers.gr_key.as_deref()),
        (AnchorType::HC_WORK, identifiers.hc_key.as_deref()),
        (AnchorType::ISBN_13, identifiers.isbn_13.as_deref()),
        (AnchorType::ASIN, identifiers.asin.as_deref()),
    ] {
        if value.is_some_and(|v| !v.is_empty()) {
            settled.insert(anchor_type.to_string());
        }
    }
    settled
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAnchorDto {
    pub anchor_type: String,
    pub value: String,
    pub setter: String,
}

/// Canonical string form of an [`AnchorSetter`] for the DTO (matches the
/// snake_case ledger values, e.g. `auto_search`).
fn anchor_setter_str(setter: AnchorSetter) -> &'static str {
    match setter {
        AnchorSetter::User => "user",
        AnchorSetter::AutoIsbn => "auto_isbn",
        AnchorSetter::AutoSearch => "auto_search",
        AnchorSetter::Import => "import",
        AnchorSetter::Redirect => "redirect",
    }
}

/// List a work's pending (unaffirmed) identity guesses (REQ-005).
pub async fn list_pending_anchors<S>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(work_id): Path<i64>,
) -> Result<Json<Vec<PendingAnchorDto>>, ApiError>
where
    S: HasWorkIdentityRepository + HasWorkService,
    S::WorkIdentityRepo: livrarr_domain::identity_layer::WorkIdentityRepository + Send + Sync,
{
    // R-3: the repo methods are work-id-only (no user scope), so verify ownership
    // via the user-scoped service first — another user's work must read as 404,
    // and a real service error must surface as 500, not be masked as not-found.
    let work = state
        .work_service()
        .get(ctx.user.id, work_id)
        .await
        .map_err(|e| match e {
            WorkServiceError::NotFound => ApiError::NotFound,
            other => ApiError::Internal(other.to_string()),
        })?;

    let anchors = state
        .work_identity_repo()
        .list_anchors(work_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let identifiers =
        livrarr_domain::identity_layer::WorkIdentityRepository::read_identity_presentations(
            state.work_identity_repo(),
            ctx.user.id,
            &[work.id],
        )
        .await
        .map_err(map_identity_repository_error)?
        .into_iter()
        .next()
        .map(|presentation| presentation.identifiers)
        .unwrap_or_default();
    let settled = settled_anchor_types(&identifiers, &anchors);
    let dtos = anchors
        .into_iter()
        .filter(|a| {
            a.confidence == AnchorConfidence::Pending
                && !a.anchor_value.is_empty()
                && !settled.contains(a.anchor_type.as_str())
        })
        .map(|a| PendingAnchorDto {
            anchor_type: a.anchor_type.as_str().to_string(),
            value: a.anchor_value,
            setter: anchor_setter_str(a.setter).to_string(),
        })
        .collect();

    Ok(Json(dtos))
}

pub async fn affirm_pending_anchor<S>(
    State(state): State<S>,
    ctx: AuthContext,
    Path((work_id, anchor_type)): Path<(i64, String)>,
) -> Result<Response, ApiError>
where
    S: HasWorkIdentityRepository
        + HasWorkService
        + HasIdentityRoadService
        + HasHistoryService
        + Clone
        + Send
        + Sync
        + 'static,
    S::WorkIdentityRepo: livrarr_domain::identity_layer::WorkIdentityRepository + Send + Sync,
    S::WorkSvc: 'static,
{
    let user_id = ctx.user.id;

    // R-3: confirm_anchor mutates works.* with no user scope — verify ownership
    // before any mutation so a cross-user affirm cannot touch another's work.
    let work = state
        .work_service()
        .get(user_id, work_id)
        .await
        .map_err(|e| match e {
            WorkServiceError::NotFound => ApiError::NotFound,
            other => ApiError::Internal(other.to_string()),
        })?;
    let anchor_type = AnchorType::new(anchor_type);

    // Pending value + identity generation read together (one transaction):
    // the coherent basis for the first-statement claim below (identity-edit
    // r4 §Writer coverage — pending affirm).
    let (_expected_generation, anchors) = state
        .work_identity_repo()
        .read_anchors_with_generation(work_id)
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    // Backstop to the settled-slot filter in `list_pending_anchors`: a stale or
    // hand-crafted affirm for a settled slot must not replace the identifier in
    // force (confirm_anchor overwrites works.* unconditionally).
    let identifiers =
        livrarr_domain::identity_layer::WorkIdentityRepository::read_identity_presentations(
            state.work_identity_repo(),
            user_id,
            &[work.id],
        )
        .await
        .map_err(map_identity_repository_error)?
        .into_iter()
        .next()
        .map(|presentation| presentation.identifiers)
        .unwrap_or_default();
    if settled_anchor_types(&identifiers, &anchors).contains(anchor_type.as_str()) {
        return Err(ApiError::Conflict {
            reason: "an identifier of this type is already confirmed for this work".into(),
        });
    }

    let value = anchors
        .into_iter()
        .find(|a| a.confidence == AnchorConfidence::Pending && a.anchor_type == anchor_type)
        .map(|a| a.anchor_value)
        .ok_or(ApiError::NotFound)?;

    let route_kind = match anchor_type.as_str() {
        AnchorType::OL_WORK => livrarr_domain::identity_layer::RouteKind::OpenLibraryWork,
        AnchorType::GR_WORK => livrarr_domain::identity_layer::RouteKind::GoodreadsWork,
        AnchorType::HC_WORK => livrarr_domain::identity_layer::RouteKind::HardcoverWork,
        AnchorType::ISBN_13 => livrarr_domain::identity_layer::RouteKind::Isbn13Edition,
        AnchorType::ASIN => livrarr_domain::identity_layer::RouteKind::AsinEdition,
        _ => {
            return Err(ApiError::BadRequest(
                "unsupported pending route kind".to_string(),
            ))
        }
    };
    let provider = match anchor_type.as_str() {
        AnchorType::OL_WORK => livrarr_domain::identity_layer::IdentityProvider::OpenLibrary,
        AnchorType::GR_WORK => livrarr_domain::identity_layer::IdentityProvider::Goodreads,
        AnchorType::HC_WORK => livrarr_domain::identity_layer::IdentityProvider::Hardcover,
        AnchorType::ISBN_13 => livrarr_domain::identity_layer::IdentityProvider::IsbnRegistry,
        AnchorType::ASIN => livrarr_domain::identity_layer::IdentityProvider::Amazon,
        _ => {
            return Err(ApiError::BadRequest(
                "unsupported pending route provider".to_string(),
            ))
        }
    };
    let route = livrarr_domain::identity_layer::RouteKey {
        provider: provider.clone(),
        kind: route_kind,
        value,
    };

    // PM sweep F3: an already-owned pending value must be a stable 409 before
    // the identity road can bump a generation, write a settlement audit, or
    // mint a card. The repository lookup validates the route-ledger ∪ legacy
    // projection and is same-user scoped.
    if let Some(owner) = state
        .work_identity_repo()
        .find_anchor_owner(user_id, &anchor_type, &route.value, work_id)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?
    {
        return Err(ApiError::ConflictDetailed {
            message: format!(
                "this identifier already belongs to \"{}\" — merge the works instead",
                owner.owning_work_title
            ),
            details: crate::types::api_error::ErrorDetails {
                code: "anchor_collision",
                owning_work_id: Some(owner.owning_work_id),
                owning_work_title: Some(owner.owning_work_title),
            },
        });
    }
    let outcome = state
        .identity_road_service()
        .settle(livrarr_domain::identity_layer::IdentityRoadRequest {
            user_id,
            origin: livrarr_domain::identity_layer::IdentityRoadOrigin::AffirmPendingRoute,
            evidence: livrarr_domain::identity_layer::IdentityEvidenceBundle {
                user_choice: None,
                owned_files: Vec::new(),
                provider_identity: vec![livrarr_domain::identity_layer::ProviderIdentityEvidence {
                    provider,
                    route: route.clone(),
                    work_core: None,
                    provenance: Default::default(),
                }],
                minimum: None,
            },
            interaction: livrarr_domain::identity_layer::IdentityRoadInteraction::HumanWatching,
            existing_work_id: Some(work_id),
        })
        .await
        .map_err(map_identity_road_error)?;
    let (card_id, expected_generation) = pending_review_claim(outcome)?;
    state
        .identity_road_service()
        .resolve_review(
            livrarr_domain::identity_layer::ReviewActor::AuthenticatedUser { user_id },
            livrarr_domain::identity_layer::ReviewResolutionCommand::PendingRoute {
                card_id,
                expected_generation,
                action: livrarr_domain::identity_layer::PendingRouteAction::Affirm {
                    surviving_routes: vec![route.clone()],
                },
            },
        )
        .await
        .map_err(map_identity_road_error)?;
    state
        .history_service()
        .record(
            user_id,
            history_events::identity_resolved(
                work_id,
                &work.title,
                "affirm",
                format!("{:?}: {}", route.kind, route.value),
            ),
        )
        .await;
    let refresh_state = state.clone();
    tokio::spawn(async move {
        let _ = refresh_state
            .work_service()
            .refresh(user_id, work_id, RefreshSurface::Interactive)
            .await;
    });
    Ok(StatusCode::NO_CONTENT.into_response())
}

// =============================================================================
// Identity edit: preview-confirm + clear (design identity-edit r4)
// =============================================================================

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityPreviewRequest {
    pub input: String,
    #[serde(default)]
    pub slot: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct IdentityCommitRequest {
    pub preview_id: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPreviewDto {
    pub title: Option<String>,
    pub author: Option<String>,
    pub year: Option<i32>,
    pub language: Option<String>,
    pub cover_url: Option<String>,
    pub slot: String,
    pub canonical_value: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SiblingAssessmentDto {
    pub slot: String,
    pub action: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeWarningDto {
    pub slot: String,
    pub message: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollisionDto {
    pub owning_work_id: i64,
    pub owning_work_title: String,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityPreviewResponse {
    pub resolved: Option<ResolvedPreviewDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_id: Option<String>,
    pub siblings: Vec<SiblingAssessmentDto>,
    pub bridge_warnings: Vec<BridgeWarningDto>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collision: Option<CollisionDto>,
    pub conflict_warning: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The five identity slots; editable excludes `hc_work` (internal numeric id
/// with no public page a user could obtain or verify — §Slot roster).
fn parse_identity_slot(slot: &str, allow_hc: bool) -> Result<AnchorType, ApiError> {
    match slot {
        AnchorType::GR_WORK | AnchorType::OL_WORK | AnchorType::ISBN_13 | AnchorType::ASIN => {
            Ok(AnchorType::new(slot))
        }
        AnchorType::HC_WORK if allow_hc => Ok(AnchorType::new(slot)),
        AnchorType::HC_WORK => Err(ApiError::BadRequest(
            "hc_work is not editable — it has no public identifier a user could paste".into(),
        )),
        _ => Err(ApiError::BadRequest(format!(
            "unknown identity slot: {slot}"
        ))),
    }
}

/// Phase 1 of 2 (§Preview): classify + fetch the certified record + assess
/// siblings/bridges + collision check; a certifiable outcome carries an
/// opaque single-use `previewId`.
pub async fn preview_identity_edit<S: HasWorkService>(
    State(state): State<S>,
    ctx: AuthContext,
    Path(work_id): Path<i64>,
    Json(body): Json<IdentityPreviewRequest>,
) -> Result<Json<IdentityPreviewResponse>, ApiError> {
    let slot_hint = match body.slot.as_deref() {
        None => None,
        Some(slot) => Some(parse_identity_slot(slot, false)?),
    };

    let preview = state
        .work_service()
        .preview_identity_edit(ctx.user.id, work_id, &body.input, slot_hint)
        .await
        .map_err(ApiError::from)?;

    let resolved = preview.resolved.map(|record| ResolvedPreviewDto {
        title: record.title,
        author: record.author,
        year: record.year,
        language: record.language,
        cover_url: record.cover_url,
        slot: preview
            .slot
            .as_ref()
            .map(|s| s.as_str().to_string())
            .unwrap_or_default(),
        canonical_value: preview.canonical_value.clone().unwrap_or_default(),
    });
    Ok(Json(IdentityPreviewResponse {
        resolved,
        preview_id: preview.preview_id,
        siblings: preview
            .siblings
            .into_iter()
            .map(|s| SiblingAssessmentDto {
                slot: s.slot.as_str().to_string(),
                action: match s.action {
                    livrarr_domain::services::SiblingAction::Keep => "keep",
                    livrarr_domain::services::SiblingAction::Drop => "drop",
                },
                cause: s.cause,
            })
            .collect(),
        bridge_warnings: preview
            .bridge_warnings
            .into_iter()
            .map(|w| BridgeWarningDto {
                slot: w.slot.as_str().to_string(),
                message: w.message,
            })
            .collect(),
        collision: preview.collision.map(|c| CollisionDto {
            owning_work_id: c.owning_work_id,
            owning_work_title: c.owning_work_title,
        }),
        conflict_warning: preview.conflict_warning,
        reason: preview.failure_reason,
    }))
}

/// Phase 2 (§Commit): consume the snapshot atomically and commit (or detect
/// the true no-op). Success returns the standard `WorkDetailResponse`.
pub async fn commit_identity_edit<
    S: HasWorkService + HasHistoryService + HasIdentityLayerRepository,
>(
    State(state): State<S>,
    ctx: AuthContext,
    Path((work_id, slot)): Path<(i64, String)>,
    Json(body): Json<IdentityCommitRequest>,
) -> Result<Json<WorkDetailResponse>, ApiError> {
    let user_id = ctx.user.id;
    let slot = parse_identity_slot(&slot, false)?;

    let commit = state
        .work_service()
        .commit_identity_edit(user_id, work_id, slot.clone(), &body.preview_id)
        .await
        .map_err(ApiError::from)?;

    if !commit.no_op {
        state
            .history_service()
            .record(
                user_id,
                history_events::identity_resolved(
                    work_id,
                    &commit.work.title,
                    "edit",
                    format!(
                        "{}: {} → {}",
                        slot.as_str(),
                        commit.old_value.as_deref().unwrap_or("(empty)"),
                        commit.new_value
                    ),
                ),
            )
            .await;

        // Fire-and-forget the refresh the certified identity unlocks
        // (door→road, insight 46).
        let s = state.clone();
        tokio::spawn(async move {
            if let Err(e) = s
                .work_service()
                .refresh(user_id, work_id, RefreshSurface::Interactive)
                .await
            {
                tracing::debug!(work_id, "post-edit background refresh skipped: {e}");
            }
        });
    }

    let mut detail = work_to_detail(&commit.work);
    project_work_identity_presentations(&state, user_id, std::slice::from_mut(&mut detail)).await?;
    detail.enriching = state.work_service().is_enriching(user_id, work_id);
    Ok(Json(detail))
}

/// Clear one identity slot (§Clear; all five slots). 404 when the slot is
/// truly empty (no confirmed row, no nonempty column, no pending row).
pub async fn clear_identity_slot<
    S: HasWorkService + HasHistoryService + HasIdentityLayerRepository,
>(
    State(state): State<S>,
    ctx: AuthContext,
    Path((work_id, slot)): Path<(i64, String)>,
) -> Result<Json<WorkDetailResponse>, ApiError> {
    let user_id = ctx.user.id;
    let slot = parse_identity_slot(&slot, true)?;

    let cleared = state
        .work_service()
        .clear_identity_slot(user_id, work_id, slot.clone())
        .await
        .map_err(ApiError::from)?;

    state
        .history_service()
        .record(
            user_id,
            history_events::identity_resolved(
                work_id,
                &cleared.work.title,
                "clear",
                format!("{}: {} → (cleared)", slot.as_str(), cleared.old_value),
            ),
        )
        .await;

    // No-conflict clears become chaseable — spawn the re-chase. A parked
    // work stays paused until the open conflict is reviewed (§Clear).
    if !cleared.parked_by_conflicts {
        let s = state.clone();
        tokio::spawn(async move {
            if let Err(e) = s
                .work_service()
                .refresh(user_id, work_id, RefreshSurface::Interactive)
                .await
            {
                tracing::debug!(work_id, "post-clear background refresh skipped: {e}");
            }
        });
    }

    let mut detail = work_to_detail(&cleared.work);
    project_work_identity_presentations(&state, user_id, std::slice::from_mut(&mut detail)).await?;
    detail.enriching = state.work_service().is_enriching(user_id, work_id);
    Ok(Json(detail))
}

// ---------------------------------------------------------------------------
// Identity-layer-rewrite (F2) additive handler. IR v1 `livrarr-handlers`
// module (ir-v1-identity-layer-rewrite.yaml:1348-1351). Verbatim module path
// (`work::manual_provider_search`) — no existing name collision here. No
// behavior, no wiring (stub scope): body is `todo!()`; not router-registered.
// ---------------------------------------------------------------------------

pub async fn manual_provider_search<S: crate::context::HasIdentityRoadService>(
    State(_state): State<S>,
    _ctx: AuthContext,
    Path(_work_id): Path<i64>,
    Query(query): Query<crate::types::identity_layer::TitleAuthorQuery>,
) -> Result<Json<Vec<crate::types::identity_layer::ProviderIdentityCandidate>>, ApiError> {
    if query.title.trim().is_empty() || query.author.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "title and author are required".to_string(),
        ));
    }
    // Candidate lookup is deliberately read-only. Provider-specific search is
    // composed behind the identity road; until a provider yields a typed route,
    // an empty candidate set is the complete, truthful response.
    Ok(Json(Vec::new()))
}
