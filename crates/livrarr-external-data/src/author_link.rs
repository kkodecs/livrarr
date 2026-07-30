//! Road-scoped keyed contributor fetches: every author credited on ONE
//! selected work, for OpenLibrary, Goodreads, and Hardcover.
//!
//! These adapters return contributors, never a chosen author. Selection is the
//! caller's guard, so a multi-contributor work reaches it whole. Three shapes
//! are kept apart on purpose: a readable record crediting nobody is an empty
//! success, an unreadable association shape is `LayoutDrift`, and a transport
//! failure keeps its retry timing.

use std::collections::HashSet;
use std::time::Duration;

use livrarr_domain::services::{
    FetchError, FetchRequest, HttpFetcher, HttpMethod, RateBucket, UserAgentProfile,
};
use livrarr_domain::{AuthorProvider, AuthorRouteKey, ProviderAuthorRef, RequestPriority};

use crate::goodreads;
use crate::hardcover::{hc_post, HardcoverError};
use crate::openlibrary::classify_ol_error;
use crate::provider_client::{GoodreadsClient, HardcoverClient, OpenLibraryClient};
use crate::types::ProviderFetchError;

/// How many Hardcover editions of one book are scanned for contributors.
/// Editions of the same book credit the same people; the cap bounds a
/// pathological catalogue without narrowing a real contributor set.
const HARDCOVER_EDITION_SCAN_LIMIT: u32 = 20;

/// Keep only the first ref per canonical route key, preserving the order the
/// provider credited them in.
fn canonical_distinct(refs: Vec<ProviderAuthorRef>) -> Vec<ProviderAuthorRef> {
    let mut seen = HashSet::new();
    refs.into_iter()
        .filter(|candidate| seen.insert(candidate.key.value()))
        .collect()
}

impl<F: HttpFetcher> OpenLibraryClient<F> {
    /// Every author credited on one OpenLibrary work.
    ///
    /// A work record names its contributors by key. The name is read from the
    /// record when it carries one and hydrated from the author record when it
    /// does not, so a contributor never reaches the name guard nameless.
    pub async fn fetch_work_authors(
        &self,
        work_key: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, ProviderFetchError> {
        let key = work_key
            .trim()
            .trim_start_matches("/works/")
            .trim_start_matches('/');
        if key.is_empty() {
            return Err(ProviderFetchError::Permanent(
                "OpenLibrary work route is empty".to_string(),
            ));
        }

        let record = self
            .fetch_ol_json(
                &format!("https://openlibrary.org/works/{key}.json"),
                priority,
            )
            .await?;

        let entries = record
            .get("authors")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();

        let mut refs = Vec::with_capacity(entries.len());
        for entry in &entries {
            let Some(raw_key) = entry
                .pointer("/author/key")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|raw| !raw.is_empty())
            else {
                continue;
            };
            let Ok(route) = AuthorRouteKey::parse(AuthorProvider::OpenLibrary, raw_key) else {
                tracing::warn!(
                    work_key = %key,
                    author_key = %raw_key,
                    "OpenLibrary work contributor key is not a canonical author key"
                );
                continue;
            };
            let AuthorRouteKey::OpenLibrary(author_key) = &route else {
                continue;
            };

            let name = match entry
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
            {
                Some(name) => name.to_string(),
                None => {
                    self.hydrate_author_name(author_key.as_str(), priority)
                        .await?
                }
            };

            refs.push(ProviderAuthorRef {
                key: route,
                name,
                role: entry
                    .pointer("/type/key")
                    .and_then(|value| value.as_str())
                    .map(|raw| raw.trim_start_matches("/type/").to_string()),
            });
        }

        Ok(canonical_distinct(refs))
    }

    /// The credited name on one OpenLibrary author record.
    ///
    /// A missing or blank name is a failure, never an empty name: an author
    /// ref with no name cannot be guarded, and pretending otherwise would put
    /// an unverifiable route in front of the guard.
    async fn hydrate_author_name(
        &self,
        author_key: &str,
        priority: RequestPriority,
    ) -> Result<String, ProviderFetchError> {
        let record = self
            .fetch_ol_json(
                &format!("https://openlibrary.org/authors/{author_key}.json"),
                priority,
            )
            .await?;
        record
            .get("name")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .ok_or_else(|| {
                ProviderFetchError::Permanent(format!(
                    "OpenLibrary author {author_key} record carries no name"
                ))
            })
    }

    /// One paced OpenLibrary GET, through the shared queue bucket and the
    /// established server identity.
    async fn fetch_ol_json(
        &self,
        url: &str,
        priority: RequestPriority,
    ) -> Result<serde_json::Value, ProviderFetchError> {
        let request = FetchRequest {
            url: url.to_string(),
            method: HttpMethod::Get,
            headers: vec![],
            body: None,
            timeout: Duration::from_secs(30),
            rate_bucket: RateBucket::OpenLibrary,
            max_body_bytes: 2 * 1024 * 1024,
            anti_bot_check: false,
            user_agent: UserAgentProfile::Server,
            priority,
        };
        let response = match self.fetcher().fetch(request).await {
            Ok(response) => response,
            Err(error) => return Err(map_ol_transport_error(error)),
        };
        if !(200..300).contains(&response.status) {
            return Err(map_ol_status(response.status));
        }
        serde_json::from_slice(&response.body).map_err(|error| {
            ProviderFetchError::LayoutDrift(format!("OpenLibrary response is not JSON: {error}"))
        })
    }
}

/// A transport failure keeps its class: a pause, a rate limit with its wait, or
/// a permanent refusal.
fn map_ol_transport_error(error: FetchError) -> ProviderFetchError {
    match error {
        FetchError::RateLimited => ProviderFetchError::Retryable {
            error: "OpenLibrary rate limited".to_string(),
            retry_not_before: None,
        },
        FetchError::CircuitOpen { retry_after } => ProviderFetchError::CircuitOpen(retry_after),
        FetchError::QueueFull { retry_after } => ProviderFetchError::QueueFull(retry_after),
        FetchError::HttpError { status, .. } => map_ol_status(status),
        other => ProviderFetchError::Retryable {
            error: format!("OpenLibrary transport failure: {other}"),
            retry_not_before: None,
        },
    }
}

/// A non-2xx OpenLibrary status, classified through the shared authority so
/// this surface cannot disagree with the enrichment surface about a status.
fn map_ol_status(status: u16) -> ProviderFetchError {
    if status == 404 || status == 410 {
        return ProviderFetchError::Permanent(format!("OpenLibrary HTTP {status}"));
    }
    match classify_ol_error(status) {
        ProviderFetchError::RateLimited | ProviderFetchError::Transient => {
            ProviderFetchError::Retryable {
                error: format!("OpenLibrary HTTP {status}"),
                retry_not_before: None,
            }
        }
        _ => ProviderFetchError::Permanent(format!("OpenLibrary HTTP {status}")),
    }
}

impl<F: HttpFetcher> GoodreadsClient<F> {
    /// Every contributor credited on one Goodreads book.
    ///
    /// Only the selected book's own contributor edges (or its JSON-LD author
    /// entries) are followed. An unreadable association shape is `LayoutDrift`,
    /// so a Goodreads layout change can never look like an uncredited book.
    pub async fn fetch_work_authors(
        &self,
        book_key: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, ProviderFetchError> {
        let key = book_key.trim();
        if key.is_empty() {
            return Err(ProviderFetchError::Permanent(
                "Goodreads book route is empty".to_string(),
            ));
        }
        let url = goodreads::detail_url_for_gr_key(self.base_url(), key);
        let html = goodreads::fetch_goodreads_html(self.fetcher(), &url, priority)
            .await
            .map_err(map_goodreads_error)?;

        let contributors = goodreads::parse_book_contributors(&html).ok_or_else(|| {
            ProviderFetchError::LayoutDrift(format!(
                "Goodreads book {key} has no readable contributor association"
            ))
        })?;

        let mut refs = Vec::with_capacity(contributors.len());
        for contributor in contributors {
            match AuthorRouteKey::parse(AuthorProvider::Goodreads, &contributor.raw_id) {
                Ok(route) => refs.push(ProviderAuthorRef {
                    key: route,
                    name: contributor.name,
                    role: contributor.role,
                }),
                Err(_) => tracing::warn!(
                    book_key = %key,
                    author_id = %contributor.raw_id,
                    "Goodreads contributor id is not a canonical author id"
                ),
            }
        }

        Ok(canonical_distinct(refs))
    }
}

/// Goodreads transport failures, keeping a pause distinct from a refusal and an
/// anti-bot block distinct from a missing page.
fn map_goodreads_error(error: goodreads::GoodreadsFetchError) -> ProviderFetchError {
    match error {
        goodreads::GoodreadsFetchError::CircuitOpen(retry_after) => {
            ProviderFetchError::CircuitOpen(retry_after)
        }
        goodreads::GoodreadsFetchError::QueueFull(retry_after) => {
            ProviderFetchError::QueueFull(retry_after)
        }
        goodreads::GoodreadsFetchError::HttpStatus(status) if status == 404 || status == 410 => {
            ProviderFetchError::Permanent(format!("Goodreads HTTP {status}"))
        }
        goodreads::GoodreadsFetchError::HttpStatus(status) => ProviderFetchError::Retryable {
            error: format!("Goodreads HTTP {status}"),
            retry_not_before: None,
        },
        goodreads::GoodreadsFetchError::SearchRouteFailure(status) => {
            ProviderFetchError::Retryable {
                error: format!("Goodreads route failure HTTP {status}"),
                retry_not_before: None,
            }
        }
        goodreads::GoodreadsFetchError::AntiBot => ProviderFetchError::Retryable {
            error: "Goodreads anti-bot challenge".to_string(),
            retry_not_before: None,
        },
        goodreads::GoodreadsFetchError::Network(detail)
        | goodreads::GoodreadsFetchError::SsrfRejected(detail) => ProviderFetchError::Retryable {
            error: detail,
            retry_not_before: None,
        },
        // The page fetch never parses, so this cannot originate here; a future
        // shared helper that does is drift, not an uncredited book.
        goodreads::GoodreadsFetchError::Parse => ProviderFetchError::LayoutDrift(
            "Goodreads page carried no readable payload".to_string(),
        ),
    }
}

impl<F: HttpFetcher> HardcoverClient<F> {
    /// Every contributor credited on one Hardcover book.
    ///
    /// Hardcover credits contributors per edition of a book, so the book's
    /// editions are the association this reads. A readable response with no
    /// contributions is an empty success; a response whose contribution or
    /// author shape has moved is `LayoutDrift`, never a silent empty answer.
    pub async fn fetch_work_authors(
        &self,
        book_key: String,
        priority: RequestPriority,
    ) -> Result<Vec<ProviderAuthorRef>, ProviderFetchError> {
        let config = self.live_config().snapshot();
        if !config.hardcover_enabled {
            return Err(ProviderFetchError::NotConfigured);
        }
        let Some(token) = config
            .hardcover_api_token
            .as_deref()
            .map(|token| {
                token
                    .trim()
                    .trim_start_matches("Bearer ")
                    .trim_start_matches("bearer ")
            })
            .filter(|token| !token.is_empty())
        else {
            return Err(ProviderFetchError::NotConfigured);
        };

        let book_id: i64 = book_key.trim().parse().map_err(|_| {
            ProviderFetchError::Permanent(format!(
                "Hardcover book route {book_key:?} is not an integer id"
            ))
        })?;

        let query = format!(
            r#"query GetBookContributors($bookId: Int!) {{
            editions(where: {{book_id: {{_eq: $bookId}}}}, order_by: [{{users_read_count: desc}}], limit: {HARDCOVER_EDITION_SCAN_LIMIT}) {{
                contributions {{
                    author {{
                        id
                        name
                    }}
                }}
            }}
        }}"#
        );
        let body = serde_json::json!({
            "query": query,
            "variables": {"bookId": book_id}
        });
        let response = hc_post(self.fetcher(), body, token, priority)
            .await
            .map_err(map_hardcover_error)?;

        let editions = response
            .pointer("/data/editions")
            .and_then(|value| value.as_array())
            .ok_or_else(|| {
                ProviderFetchError::LayoutDrift(
                    "Hardcover response carries no editions array".to_string(),
                )
            })?;

        let mut refs = Vec::new();
        for edition in editions {
            let contributions = edition
                .get("contributions")
                .and_then(|value| value.as_array())
                .ok_or_else(|| {
                    ProviderFetchError::LayoutDrift(
                        "Hardcover edition carries no contributions array".to_string(),
                    )
                })?;
            for contribution in contributions {
                let author = contribution
                    .get("author")
                    .filter(|value| value.is_object())
                    .ok_or_else(|| {
                        ProviderFetchError::LayoutDrift(
                            "Hardcover contribution carries no author object".to_string(),
                        )
                    })?;
                let id = author
                    .get("id")
                    .and_then(|value| value.as_i64())
                    .ok_or_else(|| {
                        ProviderFetchError::LayoutDrift(
                            "Hardcover contribution author carries no integer id".to_string(),
                        )
                    })?;
                let name = author
                    .get("name")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| {
                        ProviderFetchError::LayoutDrift(
                            "Hardcover contribution author carries no name".to_string(),
                        )
                    })?;
                let route = AuthorRouteKey::parse(AuthorProvider::Hardcover, &id.to_string())
                    .map_err(|_| {
                        ProviderFetchError::LayoutDrift(format!(
                            "Hardcover author id {id} is not a canonical author id"
                        ))
                    })?;
                refs.push(ProviderAuthorRef {
                    key: route,
                    name: name.to_string(),
                    role: None,
                });
            }
        }

        Ok(canonical_distinct(refs))
    }
}

/// Hardcover failures: a pause stays a pause, and a readable-but-refused
/// GraphQL answer is permanent rather than drift.
fn map_hardcover_error(error: HardcoverError) -> ProviderFetchError {
    match error {
        HardcoverError::CircuitOpen(retry_after) => ProviderFetchError::CircuitOpen(retry_after),
        HardcoverError::Http(detail) if is_retryable_hardcover_failure(&detail) => {
            ProviderFetchError::Retryable {
                error: detail,
                retry_not_before: None,
            }
        }
        HardcoverError::Http(detail) => ProviderFetchError::Permanent(detail),
        HardcoverError::NoResults => {
            ProviderFetchError::Permanent("Hardcover returned no book".to_string())
        }
        HardcoverError::NoMatch(detail) => ProviderFetchError::Permanent(detail),
    }
}

/// A rate limit or a 5xx is Hardcover's own temporary state, so it retries;
/// every other refusal is an answer about this request and does not.
fn is_retryable_hardcover_failure(detail: &str) -> bool {
    detail
        .strip_prefix("HTTP ")
        .and_then(|status| status.trim().parse::<u16>().ok())
        .is_some_and(|status| status == 429 || (500..600).contains(&status))
}
