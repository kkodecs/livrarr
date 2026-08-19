//! Identity-layer-rewrite (F2) additive handler DTOs. IR v1 `livrarr-handlers`
//! module (ir-v1-identity-layer-rewrite.yaml:1332-1351). New file — no
//! existing `types/identity*.rs` to collide with.

use serde::{Deserialize, Serialize};

use livrarr_domain::identity_layer::{IdentityProvider, ReviewResolutionCommand, RouteKey};

/// IR v1 names `ReviewResolutionRequest` (`identity_review::resolve`'s sole
/// input) without a field list. `ReviewResolutionCommand` already carries
/// `card_id`/`expected_generation`/action, so the request is a thin wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewResolutionRequest {
    pub command: ReviewResolutionCommand,
}

/// IR v1 names `TitleAuthorQuery` (`work::manual_provider_search`'s query
/// input) without a field list; the two fields its own name specifies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleAuthorQuery {
    pub title: String,
    pub author: String,
}

/// IR v1 names `ProviderIdentityCandidate` (`work::manual_provider_search`'s
/// result element) without a field list. Reconstructed as a provider route
/// plus the title/author preview a manual-search UI needs to show it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderIdentityCandidate {
    pub provider: IdentityProvider,
    pub route: RouteKey,
    pub title: String,
    pub author: String,
}
