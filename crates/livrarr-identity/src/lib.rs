//! Identity resolution — "what work is this?". The tier-scoped multi-provider
//! resolver plus its background-convergence helpers. Depends only on the domain
//! contract and the external-data substrate; holds no enrichment edge (the
//! one-way identity/enrichment sibling boundary).

pub mod async_resolver;
pub mod english_identity_resolver;
pub mod title_cleanup;
