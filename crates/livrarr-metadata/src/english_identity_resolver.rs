use livrarr_domain::identity::*;
use livrarr_domain::services::WorkIdentityError;
use livrarr_domain::UserId;
use std::sync::Arc;
use std::time::Duration;

pub use livrarr_domain::identity::WorkSeed;

#[derive(Debug, Clone)]
pub struct ResolverConfig {
    pub confirm_title_jaccard: f64,
    pub confirm_runner_up_delta: f64,
    pub call_timeout: Duration,
}

impl Default for ResolverConfig {
    fn default() -> Self {
        Self {
            confirm_title_jaccard: 0.75,
            confirm_runner_up_delta: 0.10,
            call_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OlSearchHit {
    pub ol_key: String,
    pub title: String,
    pub author_combined: String,
    pub first_publish_year: Option<i32>,
    pub isbn: Option<String>,
}

#[derive(Debug, Clone)]
pub enum OlError {
    Transient(String),
    CircuitOpen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
}

#[trait_variant::make(Send)]
pub trait OpenLibraryClient: Send + Sync {
    fn circuit_state(&self) -> CircuitState;
    async fn isbn_to_work(&self, isbn: &str) -> Result<Option<String>, OlError>;
    async fn search_works(
        &self,
        title: &str,
        author: &str,
        limit: u32,
    ) -> Result<Vec<OlSearchHit>, OlError>;
}

pub use livrarr_domain::services::IdentityResolver as EnglishIdentityResolver;

pub struct LiveEnglishIdentityResolver<O> {
    pub ol: Arc<O>,
    pub config: ResolverConfig,
}

impl<O: OpenLibraryClient> EnglishIdentityResolver for LiveEnglishIdentityResolver<O> {
    async fn resolve(
        &self,
        user_id: UserId,
        seed: &WorkSeed,
        tier: LatencyTier,
    ) -> Result<Resolution, WorkIdentityError> {
        let _ = (user_id, seed, tier);
        todo!()
    }
}

impl<O> LiveEnglishIdentityResolver<O> {
    /// REQ-008 provider matrix scoped by seed + tier + language; excludes any
    /// provider lacking its prerequisite (GB key / LLM / Audnexus tier) and
    /// never narrows a multi-eligible seed to a single provider (the #97 guard).
    pub fn select_providers(
        &self,
        seed: &WorkSeed,
        tier: LatencyTier,
    ) -> Vec<livrarr_domain::MetadataProvider> {
        let _ = (seed, tier);
        todo!()
    }
}

/// Build an in-memory, never-persisted Work from the seed to drive
/// ProviderClient::fetch (which takes &Work) during pre-create discovery (cR-004).
pub fn build_transient_work_from_seed(seed: &WorkSeed, user_id: UserId) -> livrarr_domain::Work {
    let _ = (seed, user_id);
    todo!()
}

/// Trust a non-harvested Goodreads key only by inspecting the payload the fetch
/// already returned (no extra network, REQ-014): require a populated title that
/// matches the resolved identity beyond the similarity threshold (REQ-024);
/// otherwise the key is stripped.
pub fn verify_gr_payload(
    payload: &crate::NormalizedWorkDetail,
    captured: &CapturedIdentity,
) -> bool {
    let _ = (payload, captured);
    todo!()
}

/// Group responders by shared returned anchor / normalized title+author:
/// majority wins, a 1-vs-1 split is a QuorumTie, two providers agreeing on
/// title+author but returning different same-type anchors are a terminal
/// Conflict (REQ-018/020). Provisional signature — no standalone TDD directive;
/// refined during implementation.
pub fn run_quorum(
    responders: &std::collections::HashMap<
        livrarr_domain::MetadataProvider,
        crate::NormalizedWorkDetail,
    >,
    seed: &WorkSeed,
) -> Resolution {
    let _ = (responders, seed);
    todo!()
}
