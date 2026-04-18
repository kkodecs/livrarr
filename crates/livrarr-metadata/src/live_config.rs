//! Live, in-memory snapshot of `MetadataConfig` shared across enrichment
//! components.
//!
//! Why this exists: components that depend on user-configurable credentials
//! (LLM API key, HC token, etc.) used to bake the values into their structs at
//! startup, requiring a server restart to pick up UI config changes. That was
//! a bad design. Now those components hold an `Arc<LiveMetadataConfig>` and
//! read fresh on each call — zero DB hits per enrichment, immediate
//! propagation when the user updates settings.
//!
//! Pattern: `Arc<RwLock<Arc<MetadataConfig>>>`. Reads take a brief read-lock
//! and clone the inner `Arc` (cheap atomic refcount bump). Writes take the
//! write-lock and swap in a new `Arc<MetadataConfig>` — old readers complete
//! against the previous snapshot, new readers see the new one.
//!
//! Update path: the `update_metadata_config` HTTP handler writes the new
//! config to the DB AND calls `LiveMetadataConfig::replace(new)` on the
//! shared instance. Next enrichment call sees the new credentials.

use livrarr_db::MetadataConfig;
use std::sync::{Arc, RwLock};

/// Shared, mutable snapshot of `MetadataConfig`. Cheap to clone (Arc bump).
#[derive(Clone)]
pub struct LiveMetadataConfig {
    inner: Arc<RwLock<Arc<MetadataConfig>>>,
}

impl LiveMetadataConfig {
    pub fn new(initial: MetadataConfig) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Arc::new(initial))),
        }
    }

    /// Read the current snapshot. Cheap — brief read-lock + Arc clone.
    pub fn snapshot(&self) -> Arc<MetadataConfig> {
        self.inner.read().unwrap().clone()
    }

    /// Swap in a new snapshot. Called by config update handlers AFTER the
    /// DB write succeeds.
    pub fn replace(&self, new: MetadataConfig) {
        *self.inner.write().unwrap() = Arc::new(new);
    }
}
