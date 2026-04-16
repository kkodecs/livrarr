//! Per-provider client seam used by `DefaultProviderQueue`.
//!
//! Real network adapters (Audnexus, Hardcover, OpenLibrary, Goodreads, LLM)
//! are added as new variants in a follow-on session. This phase only ships the
//! `Stub` variant, which is enough to drive the queue's behavioral contract
//! through the test harness.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use livrarr_domain::{MetadataProvider, Work};

use crate::{EnrichmentContext, NormalizedWorkDetail, ProviderOutcome};

/// Heterogeneous provider client. Enum dispatch instead of `Box<dyn>` because
/// `trait_variant::make(Send)` traits are not dyn-compatible. New real-provider
/// adapters are added as new variants here.
#[derive(Clone)]
pub enum ProviderClient {
    Stub(StubProviderClient),
}

impl ProviderClient {
    pub async fn fetch(
        &self,
        work: &Work,
        ctx: &EnrichmentContext,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        match self {
            Self::Stub(s) => s.fetch(work, ctx).await,
        }
    }

    pub fn provider(&self) -> MetadataProvider {
        match self {
            Self::Stub(s) => s.provider,
        }
    }

    pub fn call_count(&self) -> usize {
        match self {
            Self::Stub(s) => s.call_count(),
        }
    }
}

/// Scriptable provider client for behavioral tests. The harness builds one of
/// these per scenario, configures the outcome it should return, and reads
/// `call_count` to assert dispatch behavior.
#[derive(Clone)]
pub struct StubProviderClient {
    pub provider: MetadataProvider,
    outcome: Arc<Mutex<ProviderOutcome<NormalizedWorkDetail>>>,
    panic_on_call: bool,
    call_count: Arc<AtomicUsize>,
}

impl StubProviderClient {
    pub fn new(provider: MetadataProvider, outcome: ProviderOutcome<NormalizedWorkDetail>) -> Self {
        Self {
            provider,
            outcome: Arc::new(Mutex::new(outcome)),
            panic_on_call: false,
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn with_panic(provider: MetadataProvider) -> Self {
        Self {
            provider,
            // Panic before the lock is touched, so the outcome value is irrelevant.
            outcome: Arc::new(Mutex::new(ProviderOutcome::NotFound)),
            panic_on_call: true,
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub fn call_count(&self) -> usize {
        self.call_count.load(Ordering::SeqCst)
    }

    async fn fetch(
        &self,
        _work: &Work,
        _ctx: &EnrichmentContext,
    ) -> ProviderOutcome<NormalizedWorkDetail> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        if self.panic_on_call {
            panic!(
                "StubProviderClient panic-on-call: provider={:?}",
                self.provider
            );
        }
        self.outcome.lock().unwrap().clone()
    }
}
