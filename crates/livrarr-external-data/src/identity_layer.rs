//! Identity-layer-rewrite (F2) provider-evidence normalization boundary. IR
//! v1 `livrarr-external-data` module (ir-v1-identity-layer-rewrite.yaml:1146-1178).
//! Fails closed on every unsampled/drifting shape (ST-006, FP-006).

use livrarr_domain::identity_layer::{
    AliasEquivalenceProof, EditionFormat, IdentityProvider, ProbeId, RouteKind, SampledTextSignal,
    WorkRoute,
};
use livrarr_domain::{AnchorQuery, MetadataProvider, RequestPriority};

use crate::provider_client::ProviderClient;
use crate::ProviderOutcome;

/// IR v1 names `ProviderRouteEvidence` without a field list. Shaped as
/// `WorkRoute`'s identifying core (provider/kind/id) minus the
/// ownership/state/provenance fields the identity road — not the provider
/// boundary — decides. See STUBS-REPORT.md.
#[derive(Debug, Clone)]
pub struct ProviderRouteEvidence {
    pub provider: IdentityProvider,
    pub kind: RouteKind,
    pub provider_scoped_id: String,
}

/// IR v1 names `NormalizedEditionEvidence` without a field list. Shaped as
/// `Edition`'s content fields (format/language/subtitle/routes) minus the
/// identity fields (`id`/`work_id`/`state`) not yet assigned at fetch time.
#[derive(Debug, Clone)]
pub struct NormalizedEditionEvidence {
    pub format: Option<livrarr_domain::identity_layer::EditionFormat>,
    pub language: Option<String>,
    pub subtitle: Option<String>,
    pub routes: Vec<ProviderRouteEvidence>,
}

/// IR v1 names `ProviderContributorEvidence` without a field list. Reuses
/// the existing `livrarr_domain::ProviderAuthorRef` where present, plus the
/// raw name/role a provider payload carries before F1 identity resolution.
#[derive(Debug, Clone)]
pub struct ProviderContributorEvidence {
    pub author_ref: Option<livrarr_domain::ProviderAuthorRef>,
    pub name: String,
    pub role: Option<String>,
}

/// A previously-fetched Goodreads book page, replayed for zero-extra-request
/// work-id capture (`route_capture_handoff.goodreads_rule`).
#[derive(Debug, Clone)]
pub struct GoodreadsBookPage {
    pub book_id: String,
    pub raw_html: String,
}

#[derive(Debug, Clone)]
pub struct NormalizedWorkIdentityEvidence {
    pub provider: IdentityProvider,
    pub work_routes: Vec<ProviderRouteEvidence>,
    pub editions: Vec<NormalizedEditionEvidence>,
    pub contributors: Vec<ProviderContributorEvidence>,
    pub text_signals: Vec<SampledTextSignal>,
    pub alias_proof: Option<AliasEquivalenceProof>,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum ProviderEvidenceError {
    #[error("provider not configured")]
    NotConfigured,
    #[error("retryable: {0}")]
    Retryable(String),
    #[error("permanent: {0}")]
    Permanent(String),
    #[error("layout drift: {0}")]
    LayoutDrift(String),
    #[error("blocked on probe {0:?}")]
    ProbeBlocked(ProbeId),
}

#[trait_variant::make(Send)]
pub trait IdentityProviderGateway: Send + Sync {
    async fn fetch_by_route(
        &self,
        route: WorkRoute,
        priority: RequestPriority,
    ) -> Result<NormalizedWorkIdentityEvidence, ProviderEvidenceError>;
}

impl IdentityProviderGateway for ProviderClient {
    async fn fetch_by_route(
        &self,
        route: WorkRoute,
        priority: RequestPriority,
    ) -> Result<NormalizedWorkIdentityEvidence, ProviderEvidenceError> {
        let query = route_query(&route)?;
        if !client_serves_route(self.provider(), &route) {
            return Err(ProviderEvidenceError::NotConfigured);
        }

        match self.fetch_by_anchor(query, None, priority).await {
            ProviderOutcome::Success(detail) => {
                Ok(normalize_provider_detail(route, self.provider(), *detail))
            }
            ProviderOutcome::NotFound => Err(ProviderEvidenceError::Permanent(
                "provider_route_not_found".to_string(),
            )),
            ProviderOutcome::NotConfigured => Err(ProviderEvidenceError::NotConfigured),
            ProviderOutcome::WillRetry { reason, .. } => Err(ProviderEvidenceError::Retryable(
                format!("provider_{reason:?}").to_ascii_lowercase(),
            )),
            ProviderOutcome::PermanentFailure { reason } => Err(ProviderEvidenceError::Permanent(
                format!("provider_{reason:?}").to_ascii_lowercase(),
            )),
            ProviderOutcome::Conflict { .. } => Err(ProviderEvidenceError::LayoutDrift(
                "provider_payload_conflict".to_string(),
            )),
        }
    }
}

/// Goodreads-specific zero-extra-request work-id capture. A new name — IR
/// v1 does not reuse the crate's existing Goodreads client type for this
/// surface. See STUBS-REPORT.md.
pub struct GoodreadsAdapter;

impl GoodreadsAdapter {
    pub async fn capture_work_route_from_fetched_book_page(
        &self,
        fetched_page: GoodreadsBookPage,
    ) -> Result<Option<ProviderRouteEvidence>, ProviderEvidenceError> {
        capture_goodreads_work_route(&fetched_page)
    }
}

fn route_query(route: &WorkRoute) -> Result<AnchorQuery, ProviderEvidenceError> {
    let value = route.provider_scoped_id.trim();
    if value.is_empty() {
        return Err(ProviderEvidenceError::Permanent(
            "empty_provider_route".to_string(),
        ));
    }
    match route.kind {
        RouteKind::OpenLibraryWork => Ok(AnchorQuery::OlKey(value.to_string())),
        RouteKind::GoodreadsBookEdition => Ok(AnchorQuery::GrKey(value.to_string())),
        RouteKind::GoodreadsWork => Err(ProviderEvidenceError::Permanent(
            "goodreads_work_route_not_fetchable".to_string(),
        )),
        RouteKind::HardcoverWork => Ok(AnchorQuery::HcKey(value.to_string())),
        RouteKind::Isbn13Edition => Ok(AnchorQuery::Isbn13(value.to_string())),
        RouteKind::AsinEdition => Ok(AnchorQuery::Asin(value.to_string())),
        RouteKind::Undeclared { .. } => Err(ProviderEvidenceError::Permanent(
            "manual_only_route_kind".to_string(),
        )),
    }
}

fn client_serves_route(provider: MetadataProvider, route: &WorkRoute) -> bool {
    match (&route.provider, &route.kind) {
        (IdentityProvider::OpenLibrary, RouteKind::OpenLibraryWork) => {
            provider == MetadataProvider::OpenLibrary
        }
        (IdentityProvider::Goodreads, RouteKind::GoodreadsBookEdition) => {
            provider == MetadataProvider::Goodreads
        }
        (IdentityProvider::Hardcover, RouteKind::HardcoverWork) => {
            provider == MetadataProvider::Hardcover
        }
        (IdentityProvider::IsbnRegistry, RouteKind::Isbn13Edition) => matches!(
            provider,
            MetadataProvider::OpenLibrary
                | MetadataProvider::Hardcover
                | MetadataProvider::GoogleBooks
        ),
        (IdentityProvider::Amazon, RouteKind::AsinEdition) => {
            matches!(
                provider,
                MetadataProvider::Audible | MetadataProvider::Audnexus
            )
        }
        _ => false,
    }
}

fn normalize_provider_detail(
    route: WorkRoute,
    provider: MetadataProvider,
    detail: crate::NormalizedWorkDetail,
) -> NormalizedWorkIdentityEvidence {
    let identity_provider = route.provider.clone();
    let route_evidence = ProviderRouteEvidence {
        provider: identity_provider.clone(),
        kind: route.kind.clone(),
        provider_scoped_id: route.provider_scoped_id,
    };
    let edition_scoped = matches!(
        route.kind,
        RouteKind::Isbn13Edition | RouteKind::AsinEdition | RouteKind::GoodreadsBookEdition
    );
    let work_routes = if edition_scoped {
        Vec::new()
    } else {
        vec![route_evidence.clone()]
    };
    let editions = if edition_scoped {
        vec![NormalizedEditionEvidence {
            format: Some(EditionFormat::Unknown),
            language: detail.language.clone(),
            subtitle: detail.subtitle.clone(),
            routes: vec![route_evidence],
        }]
    } else {
        Vec::new()
    };
    let contributors = detail.author_name.map_or_else(Vec::new, |name| {
        vec![ProviderContributorEvidence {
            author_ref: None,
            name,
            // ST-006-A found no work-level role labels in either accepted OL
            // type encoding; an absent role is evidence, not a guessed author rank.
            role: None,
        }]
    });
    debug_assert!(provider_matches_identity(
        provider,
        &identity_provider,
        &route.kind
    ));

    NormalizedWorkIdentityEvidence {
        provider: identity_provider,
        work_routes,
        editions,
        contributors,
        text_signals: Vec::new(),
        alias_proof: None,
    }
}

fn provider_matches_identity(
    provider: MetadataProvider,
    identity_provider: &IdentityProvider,
    kind: &RouteKind,
) -> bool {
    let synthetic_route = WorkRoute {
        id: 0,
        user_id: 0,
        owner: livrarr_domain::identity_layer::RouteOwner::Work(0),
        resolved_work_id: 0,
        provider: identity_provider.clone(),
        kind: kind.clone(),
        provider_scoped_id: String::new(),
        state: livrarr_domain::identity_layer::WorkRouteState::Active,
        provenance: livrarr_domain::identity_layer::RouteProvenance::Migrated {
            legacy_field: String::new(),
        },
        user_confirmed: false,
        observed_at: chrono::Utc::now(),
    };
    client_serves_route(provider, &synthetic_route)
}

fn capture_goodreads_work_route(
    fetched_page: &GoodreadsBookPage,
) -> Result<Option<ProviderRouteEvidence>, ProviderEvidenceError> {
    const PROBE_ID: &str = "CAPTURE-ST002-GR-WORK-REF";
    if fetched_page.book_id.trim().is_empty() {
        return Err(ProviderEvidenceError::ProbeBlocked(ProbeId(
            PROBE_ID.to_string(),
        )));
    }
    if crate::goodreads::parse_detail_html(&fetched_page.raw_html).is_none() {
        return Err(ProviderEvidenceError::ProbeBlocked(ProbeId(
            PROBE_ID.to_string(),
        )));
    }

    let next_data =
        regex::Regex::new(r#"(?si)<script\s+id=["']__NEXT_DATA__["'][^>]*>(.*?)</script>"#)
            .map_err(|_| ProviderEvidenceError::LayoutDrift("next_data_pattern".to_string()))?
            .captures(&fetched_page.raw_html)
            .and_then(|capture| capture.get(1))
            .map(|capture| capture.as_str())
            .ok_or_else(|| ProviderEvidenceError::ProbeBlocked(ProbeId(PROBE_ID.to_string())))?;
    let root: serde_json::Value = serde_json::from_str(next_data)
        .map_err(|_| ProviderEvidenceError::LayoutDrift("invalid_next_data".to_string()))?;
    let page_props = root
        .get("props")
        .and_then(|value| value.get("pageProps"))
        .ok_or_else(|| ProviderEvidenceError::LayoutDrift("missing_page_props".to_string()))?;
    let echoed_book_id = page_props
        .get("params")
        .and_then(|value| value.get("book_id"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ProviderEvidenceError::LayoutDrift("missing_book_id_echo".to_string()))?;
    if echoed_book_id != fetched_page.book_id {
        return Err(ProviderEvidenceError::LayoutDrift(
            "book_id_echo_mismatch".to_string(),
        ));
    }
    let apollo = page_props
        .get("apolloState")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| ProviderEvidenceError::LayoutDrift("missing_apollo_state".to_string()))?;
    let root_query = apollo
        .get("ROOT_QUERY")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| ProviderEvidenceError::LayoutDrift("missing_root_query".to_string()))?;
    let expected_pointer = format!(
        "getBookByLegacyId({{\"legacyId\":\"{}\"}})",
        fetched_page.book_id
    );
    let mut book_pointers = root_query
        .iter()
        .filter(|(key, _)| key.starts_with("getBookByLegacyId("));
    let (pointer, book_pointer) = book_pointers
        .next()
        .ok_or_else(|| ProviderEvidenceError::ProbeBlocked(ProbeId(PROBE_ID.to_string())))?;
    if book_pointers.next().is_some() || pointer != &expected_pointer {
        return Err(ProviderEvidenceError::LayoutDrift(
            "ambiguous_book_pointer".to_string(),
        ));
    }
    let book_ref = book_pointer
        .get("__ref")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ProviderEvidenceError::LayoutDrift("invalid_book_pointer".to_string()))?;
    let book = apollo
        .get(book_ref)
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| ProviderEvidenceError::LayoutDrift("missing_book_record".to_string()))?;
    let book_legacy_id = json_decimal_string(book.get("legacyId"))
        .ok_or_else(|| ProviderEvidenceError::LayoutDrift("missing_book_legacy_id".to_string()))?;
    if book_legacy_id != fetched_page.book_id {
        return Err(ProviderEvidenceError::LayoutDrift(
            "book_record_id_mismatch".to_string(),
        ));
    }
    let Some(work_value) = book.get("work") else {
        return Err(ProviderEvidenceError::LayoutDrift(
            "missing_work_field".to_string(),
        ));
    };
    if work_value.is_null() {
        return Ok(None);
    }
    let work_ref = work_value
        .get("__ref")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| ProviderEvidenceError::LayoutDrift("invalid_work_pointer".to_string()))?;
    let work = apollo
        .get(work_ref)
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| ProviderEvidenceError::LayoutDrift("missing_work_record".to_string()))?;
    let work_id = json_decimal_string(work.get("legacyId"))
        .filter(|value| value != &fetched_page.book_id)
        .ok_or_else(|| ProviderEvidenceError::LayoutDrift("invalid_work_legacy_id".to_string()))?;

    // The Apollo Book record's `work.__ref` resolves to a distinct Work
    // entity; this `legacyId` is therefore provably Work-namespace evidence,
    // unlike the fetched Book record's legacyId checked above.
    Ok(Some(ProviderRouteEvidence {
        provider: IdentityProvider::Goodreads,
        kind: RouteKind::GoodreadsWork,
        provider_scoped_id: work_id,
    }))
}

fn json_decimal_string(value: Option<&serde_json::Value>) -> Option<String> {
    let value = value?;
    let text = value
        .as_str()
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|number| number.to_string()))?;
    (!text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit())).then_some(text)
}
