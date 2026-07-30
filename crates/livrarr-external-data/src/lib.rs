//! External metadata acquisition: provider HTTP clients, response parsing,
//! normalization, transport caching, and language utilities. A
//! fetch-and-normalize pipe with no knowledge of enrichment policy, the work
//! store, or merge logic.

mod author_link;
pub mod types;

pub mod audible;
pub mod audnexus;
pub mod goodreads;
pub mod google_books;
pub mod hardcover;
pub mod language;
pub mod live_config;
pub mod llm_caller_service;
pub mod normalize;
pub mod openlibrary;
pub mod parsers;
pub mod provider_client;
pub mod provider_util;
pub mod transport_cache;

#[cfg(test)]
mod test_support;

pub use livrarr_domain::OpenLibraryAuthorCandidate;
pub use types::{NormalizedWorkDetail, ProviderOutcome};

pub use google_books::GoogleBooksClient;
pub use provider_client::{
    AudnexusClient, GoodreadsClient, HardcoverClient, OpenLibraryClient, ProviderClient,
    StubProviderClient,
};
