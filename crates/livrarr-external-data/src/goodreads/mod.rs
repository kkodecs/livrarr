//! Goodreads provider integration: HTTP transport, HTML/JSON page parsing,
//! and an LLM extraction fallback for pages where direct parsing fails.
//!
//! Replaces LLM-based scraping with direct HTML parsing for foreign language
//! works. LLM is kept as fallback only (see fallback chain in design doc).

mod client;
mod llm_repair;
mod parsers;

pub use client::*;
pub use llm_repair::*;
pub use parsers::*;
