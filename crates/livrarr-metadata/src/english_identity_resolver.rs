use livrarr_domain::identity::*;
use livrarr_domain::text_norm;
use std::sync::Arc;
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

#[trait_variant::make(Send)]
pub trait EnglishIdentityResolver: Send + Sync {
    async fn resolve(&self, seed: &EnglishSeed) -> IdentityResolution;
}

pub struct LiveEnglishIdentityResolver<O> {
    pub ol: Arc<O>,
    pub config: ResolverConfig,
}

impl<O: OpenLibraryClient> EnglishIdentityResolver for LiveEnglishIdentityResolver<O> {
    async fn resolve(&self, seed: &EnglishSeed) -> IdentityResolution {
        if let Some(ref ol_key) = seed.user_confirmed_ol_key {
            return IdentityResolution::Confirmed {
                ol_key: ol_key.clone(),
                method: IdentityMethod::UserSelected,
                score: ResolutionScore {
                    title_jaccard: 1.0,
                    author_overlap: 0,
                    runner_up_delta: 1.0,
                },
            };
        }

        if self.ol.circuit_state() == CircuitState::Open {
            return IdentityResolution::Pending {
                reason: PendingReason::OlUnavailable,
                top_candidates: Vec::new(),
            };
        }

        if let Some(ref isbn) = seed.isbn {
            match tokio::time::timeout(self.config.call_timeout, self.ol.isbn_to_work(isbn)).await {
                Ok(Ok(Some(ol_key))) => {
                    return IdentityResolution::Confirmed {
                        ol_key,
                        method: IdentityMethod::IsbnDirect,
                        score: ResolutionScore {
                            title_jaccard: 1.0,
                            author_overlap: 0,
                            runner_up_delta: 1.0,
                        },
                    };
                }
                Ok(Ok(None)) => {}
                Ok(Err(OlError::CircuitOpen)) => {
                    return IdentityResolution::Pending {
                        reason: PendingReason::OlUnavailable,
                        top_candidates: Vec::new(),
                    };
                }
                Ok(Err(OlError::Transient(_))) => {}
                Err(_timeout) => {}
            }
        }

        let search_result = tokio::time::timeout(
            self.config.call_timeout,
            self.ol.search_works(&seed.title, &seed.author_name, 10),
        )
        .await;

        let hits = match search_result {
            Ok(Ok(hits)) => hits,
            Ok(Err(_)) | Err(_) => {
                return IdentityResolution::Pending {
                    reason: PendingReason::OlUnavailable,
                    top_candidates: Vec::new(),
                };
            }
        };

        if hits.is_empty() {
            return IdentityResolution::Pending {
                reason: PendingReason::NoCandidates,
                top_candidates: Vec::new(),
            };
        }

        let mut candidates: Vec<OlCandidate> =
            hits.iter().map(|hit| score_candidate(seed, hit)).collect();

        candidates.sort_by(|a, b| {
            b.title_jaccard
                .partial_cmp(&a.title_jaccard)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.author_overlap.cmp(&a.author_overlap))
        });
        candidates.truncate(3);

        let top = &candidates[0];

        if top.title_jaccard.is_nan() || top.title_jaccard < self.config.confirm_title_jaccard {
            return IdentityResolution::Pending {
                reason: PendingReason::LowConfidence,
                top_candidates: candidates,
            };
        }

        if top.author_overlap < 1 {
            return IdentityResolution::Pending {
                reason: PendingReason::LowConfidence,
                top_candidates: candidates,
            };
        }

        let runner_up_jaccard = candidates.get(1).map(|c| c.title_jaccard).unwrap_or(0.0);
        let runner_up_delta = top.title_jaccard - runner_up_jaccard;

        if runner_up_delta >= self.config.confirm_runner_up_delta {
            IdentityResolution::Confirmed {
                ol_key: top.ol_key.clone(),
                method: IdentityMethod::TitleAuthorSearch,
                score: ResolutionScore {
                    title_jaccard: top.title_jaccard,
                    author_overlap: top.author_overlap,
                    runner_up_delta,
                },
            }
        } else {
            IdentityResolution::Pending {
                reason: PendingReason::LowConfidence,
                top_candidates: candidates,
            }
        }
    }
}

pub fn score_candidate(seed: &EnglishSeed, raw: &OlSearchHit) -> OlCandidate {
    let seed_title_tokens = text_norm::title_tokens(&seed.title);
    let hit_title_tokens = text_norm::title_tokens(&raw.title);
    let title_jaccard = text_norm::jaccard(&seed_title_tokens, &hit_title_tokens);

    let seed_author_tokens = text_norm::author_tokens(&seed.author_name);
    let hit_author_tokens = text_norm::author_tokens(&raw.author_combined);
    let author_overlap = seed_author_tokens.intersection(&hit_author_tokens).count() as u32;

    OlCandidate {
        ol_key: raw.ol_key.clone(),
        title: raw.title.clone(),
        author: raw.author_combined.clone(),
        year: raw.first_publish_year,
        title_jaccard,
        author_overlap,
    }
}
