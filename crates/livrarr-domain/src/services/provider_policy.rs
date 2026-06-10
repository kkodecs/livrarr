//! Provider selection policy by language (REQ-003/REQ-014). DB is source of
//! truth; the server loads + atomically swaps an in-memory snapshot and injects
//! [`ProviderPolicySource`]. Lists are used as written — no fallback, no
//! concatenation, no provider twice within a list (AC-015).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::MetadataProvider;

/// Which priority list to consult (REQ-003). A work may be enriched for BOTH;
/// the pipeline never branches on a per-work format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListKind {
    Ebook,
    Audiobook,
}

/// A provider + its rank within one list (REQ-003). Used as written: no
/// fallback, no concatenation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRef {
    pub provider: MetadataProvider,
    pub rank: u8,
}

/// An ordered, de-duplicated priority list (REQ-003/AC-015): no provider twice
/// WITHIN a list (a provider MAY appear in both the ebook and audiobook lists,
/// per ST-001/ST-002).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderList {
    pub entries: Vec<ProviderRef>,
}

impl ProviderList {
    /// Build a validated list (AC-015): rejects a provider that appears more
    /// than once within the list.
    pub fn new(entries: Vec<ProviderRef>) -> Result<Self, ProviderPolicyError> {
        for (i, entry) in entries.iter().enumerate() {
            if entries[..i].iter().any(|e| e.provider == entry.provider) {
                return Err(ProviderPolicyError::DuplicateInList(entry.provider));
            }
        }
        Ok(Self { entries })
    }
}

/// Per-language selection policy (REQ-003): two self-contained priority lists.
/// Foreign-language lists exclude Hardcover + OpenLibrary (REQ-014/DD-003).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderPolicy {
    pub ebook: ProviderList,
    pub audiobook: ProviderList,
}

impl ProviderPolicy {
    /// The priority list for a kind (REQ-003 two self-contained lists).
    pub fn list_for(&self, kind: ListKind) -> &ProviderList {
        match kind {
            ListKind::Ebook => &self.ebook,
            ListKind::Audiobook => &self.audiobook,
        }
    }
}

/// The in-memory snapshot the server builds + atomically swaps (REQ-003). A
/// language absent from `by_language` resolves to `generic`, used standalone
/// (REQ-003/AC-014) — never appended to another list.
#[derive(Debug, Clone, Default)]
pub struct ProviderPolicySnapshot {
    pub by_language: HashMap<String, ProviderPolicy>,
    pub generic: ProviderPolicy,
}

impl ProviderPolicySnapshot {
    /// Resolve a language to its policy (REQ-003): the language's own row, or
    /// the generic row used STANDALONE when the language is unlisted (AC-014) —
    /// never appended to another list. An empty language resolves to English
    /// (REQ-002 interim default). The server's `ProviderPolicySource` impl
    /// delegates here; this is the pure resolution seam tests target.
    pub fn for_language(&self, language: &str) -> ProviderPolicy {
        let key = if language.is_empty() { "en" } else { language };
        self.by_language
            .get(key)
            .cloned()
            .unwrap_or_else(|| self.generic.clone())
    }
}

/// Error building a provider policy from rows (AC-015).
#[derive(Debug, thiserror::Error)]
pub enum ProviderPolicyError {
    #[error("provider {0:?} appears more than once within a single list")]
    DuplicateInList(MetadataProvider),
}

/// Reads the in-memory ProviderPolicy snapshot by language (REQ-003). The
/// server builds + swaps the snapshot; an unlisted language resolves to the
/// generic row. Synchronous — a snapshot read, not I/O.
pub trait ProviderPolicySource: Send + Sync {
    fn for_language(&self, language: &str) -> ProviderPolicy;
}
