//! Contract value types produced by the external-data provider clients and
//! consumed by the enrichment queue and merge layer.

use chrono::{DateTime, Utc};
use livrarr_domain::{NarrationType, PermanentFailureReason, WillRetryReason};

/// TEMP(pk-tdd): normalized provider output — common schema for all metadata providers.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct NormalizedWorkDetail {
    pub title: Option<String>,
    pub subtitle: Option<String>,
    pub original_title: Option<String>,
    pub author_name: Option<String>,
    pub description: Option<String>,
    pub year: Option<i32>,
    pub series_name: Option<String>,
    pub series_position: Option<f64>,
    pub genres: Option<Vec<String>>,
    pub language: Option<String>,
    pub page_count: Option<i32>,
    pub duration_seconds: Option<i32>,
    pub publisher: Option<String>,
    pub publish_date: Option<String>,
    pub hc_key: Option<String>,
    pub gr_key: Option<String>,
    pub ol_key: Option<String>,
    pub isbn_13: Option<String>,
    pub asin: Option<String>,
    pub narrator: Option<Vec<String>>,
    pub narration_type: Option<NarrationType>,
    pub abridged: Option<bool>,
    pub rating: Option<f64>,
    pub rating_count: Option<i32>,
    pub cover_url: Option<String>,
    pub additional_isbns: Vec<String>,
    pub additional_asins: Vec<String>,
}

impl From<livrarr_domain::services::SourceProviderData> for NormalizedWorkDetail {
    fn from(src: livrarr_domain::services::SourceProviderData) -> Self {
        Self {
            title: None,
            subtitle: None,
            original_title: None,
            author_name: None,
            description: src.description,
            year: None,
            series_name: src.series_name,
            series_position: src.series_position.and_then(|s| s.parse::<f64>().ok()),
            genres: src.genres,
            language: None,
            page_count: src.page_count,
            duration_seconds: None,
            publisher: src.publisher,
            publish_date: None,
            hc_key: None,
            gr_key: None,
            ol_key: None,
            isbn_13: src.isbn,
            asin: src.asin,
            narrator: None,
            narration_type: None,
            abridged: None,
            rating: src.rating,
            rating_count: src.rating_count,
            cover_url: src.cover_url,
            additional_isbns: vec![],
            additional_asins: vec![],
        }
    }
}

/// TEMP(pk-tdd): per-provider outcome with typed payload for Success.
#[derive(Debug, Clone)]
pub enum ProviderOutcome<T> {
    Success(Box<T>),
    NotFound,
    NotConfigured,
    WillRetry {
        reason: WillRetryReason,
        next_attempt_at: DateTime<Utc>,
    },
    PermanentFailure {
        reason: PermanentFailureReason,
    },
    Conflict {
        detail: String,
    },
}

impl<T> ProviderOutcome<T> {
    pub fn class(&self) -> livrarr_domain::OutcomeClass {
        match self {
            Self::Success(_) => livrarr_domain::OutcomeClass::Success,
            Self::NotFound => livrarr_domain::OutcomeClass::NotFound,
            Self::NotConfigured => livrarr_domain::OutcomeClass::NotConfigured,
            Self::WillRetry { .. } => livrarr_domain::OutcomeClass::WillRetry,
            Self::PermanentFailure { .. } => livrarr_domain::OutcomeClass::PermanentFailure,
            Self::Conflict { .. } => livrarr_domain::OutcomeClass::Conflict,
        }
    }

    /// TEMP(pk-tdd): returns true if this outcome is eligible for merge in background mode.
    pub fn can_merge(&self) -> bool {
        self.class().can_merge()
    }

    /// TEMP(pk-tdd): returns true if this outcome is eligible for merge in manual/hard-refresh mode.
    /// Manual mode coerces WillRetry; only Conflict still blocks.
    pub fn can_merge_manual(&self) -> bool {
        !matches!(self, Self::Conflict { .. })
    }
}

/// Transport/provider failure for query functions that don't already have a
/// typed error enum (OpenLibrary, Audnexus, Audible). Distinguishes a
/// breaker-open pause (R-11: the enrichment-surface caller must map this to
/// `WillRetryReason::CircuitOpen`, never burn retry budget on it), a genuine
/// not-found, the two retryable-and-budget-consuming classes (`RateLimited`,
/// `Transient` — Unit A, mirroring `google_books::map_http_error`), and any
/// other opaque failure.
#[derive(Debug, Clone)]
pub enum ProviderFetchError {
    CircuitOpen(std::time::Duration),
    /// The resource is genuinely absent upstream (HTTP 404/410) — a no-match,
    /// never a transient failure. Callers may fall through to weaker tiers.
    NotFound,
    /// HTTP 429 — a genuine rate-limit signal from the provider itself (not a
    /// local queue/circuit pause). This IS a real provider verdict, so unlike
    /// `CircuitOpen` it consumes one retry-budget attempt; callers map it to
    /// `WillRetry { RateLimit }` (6h + jitter, matching Google Books).
    RateLimited,
    /// HTTP 5xx, or a transport-level timeout/connection/DNS/TLS failure.
    /// Retryable and budget-consuming; callers map it to
    /// `WillRetry { ServerError }` (5 min).
    Transient,
    /// The outbound queue's local admission cap rejected the request — no
    /// HTTP was attempted (D3). A transport-level pause exactly like
    /// `CircuitOpen`, never a provider verdict; callers map it to
    /// `WillRetry { QueueFull }`, which — like `CircuitOpen` — never
    /// consumes a retry-budget attempt.
    QueueFull(std::time::Duration),
    Other(String),
}

impl std::fmt::Display for ProviderFetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CircuitOpen(d) => write!(f, "circuit open, retry after {d:?}"),
            Self::NotFound => write!(f, "not found"),
            Self::RateLimited => write!(f, "rate limited"),
            Self::Transient => write!(f, "transient failure"),
            Self::QueueFull(d) => write!(f, "queue full, retry after {d:?}"),
            Self::Other(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for ProviderFetchError {}
