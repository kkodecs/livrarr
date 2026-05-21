use crate::identity::{EnglishSeed, IdentityResolution};

#[trait_variant::make(Send)]
pub trait IdentityResolver: Send + Sync {
    async fn resolve(&self, seed: &EnglishSeed) -> IdentityResolution;
}
