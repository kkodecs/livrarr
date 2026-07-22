//! `HttpFetcher` test double shared by this crate's provider transport tests
//! (`hardcover.rs`, and any other provider module that dispatches through
//! `HttpFetcher`).

use std::sync::{Arc, Mutex};

use livrarr_domain::services::{FetchError, FetchRequest, FetchResponse, HttpFetcher};

/// Fake `HttpFetcher` that records every request it receives (for
/// door-routing assertions) and replays a queue of canned responses. Queue
/// semantics: empty queue returns a bare 200, a single entry repeats forever,
/// multiple entries are consumed in order.
pub struct RecordingHttpFetcher {
    requests: Arc<Mutex<Vec<FetchRequest>>>,
    responses: Arc<Mutex<Vec<Result<FetchResponse, FetchError>>>>,
    /// When true, `fetch_ssrf_safe` always rejects (ignoring the response
    /// queue) while `fetch` behaves normally — lets a test prove a call site
    /// routes through the SSRF-safe method specifically, rather than the
    /// unrestricted one (Unit B4).
    reject_ssrf_safe: bool,
}

impl Default for RecordingHttpFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingHttpFetcher {
    pub fn new() -> Self {
        Self {
            requests: Arc::new(Mutex::new(vec![])),
            responses: Arc::new(Mutex::new(vec![])),
            reject_ssrf_safe: false,
        }
    }

    pub fn with_response(response: Result<FetchResponse, FetchError>) -> Self {
        let f = Self::new();
        f.responses.lock().unwrap().push(response);
        f
    }

    pub fn with_ok(status: u16, body: Vec<u8>) -> Self {
        Self::with_response(Ok(FetchResponse {
            status,
            headers: vec![],
            body,
        }))
    }

    /// Like [`Self::with_ok`] but with response headers — e.g. Audnexus's
    /// `Last-Modified` 304-cache revalidation.
    pub fn with_ok_headers(status: u16, headers: Vec<(String, String)>, body: Vec<u8>) -> Self {
        Self::with_response(Ok(FetchResponse {
            status,
            headers,
            body,
        }))
    }

    pub fn with_error(err: FetchError) -> Self {
        Self::with_response(Err(err))
    }

    /// A double that succeeds on `fetch` but always rejects `fetch_ssrf_safe`
    /// with a simulated SSRF error. Proves a call site routes through the
    /// SSRF-safe method: if the code under test ever called `fetch` instead,
    /// this would incorrectly observe a success (Unit B4).
    pub fn with_ok_but_ssrf_safe_rejects(status: u16, body: Vec<u8>) -> Self {
        let f = Self::with_ok(status, body);
        Self {
            reject_ssrf_safe: true,
            ..f
        }
    }

    /// Queue an additional response, consumed in FIFO order after any
    /// already queued — for scenarios where successive calls must see
    /// different responses (e.g. a 200 that populates a cache, then a 304
    /// that must be served from it).
    pub fn push_response(&self, response: Result<FetchResponse, FetchError>) {
        self.responses.lock().unwrap().push(response);
    }

    /// Captured requests, in call order.
    pub fn requests(&self) -> std::sync::MutexGuard<'_, Vec<FetchRequest>> {
        self.requests.lock().unwrap()
    }

    pub fn call_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    fn next_response(&self) -> Result<FetchResponse, FetchError> {
        let mut responses = self.responses.lock().unwrap();
        if responses.is_empty() {
            return Ok(FetchResponse {
                status: 200,
                headers: vec![],
                body: vec![],
            });
        }
        if responses.len() == 1 {
            return clone_result(&responses[0]);
        }
        responses.remove(0)
    }
}

fn clone_result(r: &Result<FetchResponse, FetchError>) -> Result<FetchResponse, FetchError> {
    match r {
        Ok(resp) => Ok(FetchResponse {
            status: resp.status,
            headers: resp.headers.clone(),
            body: resp.body.clone(),
        }),
        Err(e) => Err(clone_fetch_error(e)),
    }
}

fn clone_fetch_error(e: &FetchError) -> FetchError {
    match e {
        FetchError::Connection(s) => FetchError::Connection(s.clone()),
        FetchError::Timeout(d) => FetchError::Timeout(*d),
        FetchError::BodyTooLarge { max_bytes } => FetchError::BodyTooLarge {
            max_bytes: *max_bytes,
        },
        FetchError::AntiBotDetected => FetchError::AntiBotDetected,
        FetchError::Ssrf(s) => FetchError::Ssrf(s.clone()),
        FetchError::HttpError {
            status,
            classification,
        } => FetchError::HttpError {
            status: *status,
            classification: classification.clone(),
        },
        FetchError::RateLimited => FetchError::RateLimited,
        FetchError::CircuitOpen { retry_after } => FetchError::CircuitOpen {
            retry_after: *retry_after,
        },
        FetchError::QueueFull { retry_after } => FetchError::QueueFull {
            retry_after: *retry_after,
        },
    }
}

impl HttpFetcher for RecordingHttpFetcher {
    async fn fetch(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.requests.lock().unwrap().push(req);
        self.next_response()
    }

    async fn fetch_ssrf_safe(&self, req: FetchRequest) -> Result<FetchResponse, FetchError> {
        self.requests.lock().unwrap().push(req);
        if self.reject_ssrf_safe {
            return Err(FetchError::Ssrf(
                "rejected by test double (Unit B4)".to_string(),
            ));
        }
        self.next_response()
    }
}
