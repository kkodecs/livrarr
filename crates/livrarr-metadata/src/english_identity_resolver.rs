use livrarr_domain::identity::*;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct EnglishSeed {
    pub title: String,
    pub author_name: String,
    pub isbn: Option<String>,
    pub user_confirmed_ol_key: Option<String>,
}

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

#[trait_variant::make(Send)]
pub trait EnglishIdentityResolver: Send + Sync {
    async fn resolve(&self, seed: &EnglishSeed) -> IdentityResolution;
}

pub struct LiveEnglishIdentityResolver<O> {
    pub ol: std::sync::Arc<O>,
    pub config: ResolverConfig,
}

pub fn score_candidate(_seed: &EnglishSeed, _raw: &OlSearchHit) -> OlCandidate {
    todo!()
}

#[derive(Debug, Clone)]
pub struct OlSearchHit {
    pub ol_key: String,
    pub title: String,
    pub author_combined: String,
    pub first_publish_year: Option<i32>,
    pub isbn: Option<String>,
}
