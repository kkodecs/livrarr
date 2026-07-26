//! `HttpFetcher` test double shared by this crate's provider transport tests
//! (`hardcover.rs`, and any other provider module that dispatches through
//! `HttpFetcher`).

use std::sync::{Arc, Mutex};

use livrarr_domain::services::{FetchError, FetchRequest, FetchResponse, HttpFetcher, RateBucket};

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

// ---------------------------------------------------------------------------
// Shared provider-breaker serialization
// ---------------------------------------------------------------------------

/// `outbound_queue::shared()` is a process-global singleton, so every
/// per-provider circuit breaker is one piece of mutable state shared by every
/// test in this binary. Any test that EMITS a breaker signal (a non-2xx, an
/// anti-bot body, a parsed payload), READS breaker state, or DEPENDS on a
/// breaker being closed (anything reaching a bucket through a real fetcher)
/// must hold this lock.
///
/// ONE lock for all buckets, not one per bucket: the queue's pacing and
/// in-flight caps are shared too, and breaker tests are few and fast, so
/// serializing them all is simpler than reasoning about which buckets can
/// safely overlap.
///
/// Private on purpose. [`lock_breaker`] is the only way in, so a test cannot
/// take the lock without the reset that makes holding it meaningful, and a
/// second same-named lock in another module — which serializes nothing — cannot
/// quietly reappear.
///
/// `tokio::sync::Mutex` rather than `std`: the guard is held across awaits.
static BREAKER_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Exclusive access to one provider's shared breaker, reset to production
/// defaults on entry AND on drop.
///
/// Resetting on drop rather than at the end of the test body is what makes this
/// panic-safe: an assertion firing while the breaker is deliberately Open, or
/// while a one-strike config is installed, would otherwise leak that state into
/// every later test in the binary. Goodreads' production `open_duration_secs`
/// is 3600, so a leaked Open breaker does not recover within a test run.
///
/// The symptom of a leak is not a red test but a **hang**: admission is refused
/// before any socket is opened, so a test awaiting its own one-shot server waits
/// for a connection that is never attempted.
pub struct BreakerGuard {
    bucket: RateBucket,
    _lock: tokio::sync::MutexGuard<'static, ()>,
}

impl Drop for BreakerGuard {
    fn drop(&mut self) {
        reset_breaker(self.bucket.clone());
    }
}

/// Take the shared breaker lock and start from a clean `bucket`. See
/// [`BreakerGuard`].
pub async fn lock_breaker(bucket: RateBucket) -> BreakerGuard {
    let lock = BREAKER_LOCK.lock().await;
    reset_breaker(bucket.clone());
    BreakerGuard {
        bucket,
        _lock: lock,
    }
}

/// Take the shared breaker lock for Goodreads — the common case.
pub async fn lock_gr_breaker() -> BreakerGuard {
    lock_breaker(RateBucket::Goodreads).await
}

fn reset_breaker(bucket: RateBucket) {
    livrarr_http::outbound_queue::shared().reset_breaker_for_tests(bucket);
}

/// Serve one canned HTTP response per connection from a throwaway local
/// listener, and return its base URL. Lets a test drive a client that owns a
/// concrete `HttpFetcherImpl` (so no fetcher double can be injected) all the
/// way through the real transport.
pub async fn spawn_canned_http_server(status: u16, body: &'static str) -> String {
    spawn_canned_http_server_seq(vec![(status, body)]).await
}

/// Like [`spawn_canned_http_server`] but answers successive connections with
/// successive entries, repeating the last one once the list is exhausted.
///
/// Needed whenever one operation makes more than one request and the legs must
/// differ — e.g. an autocomplete hit followed by a detail page that genuinely
/// fails to parse. Serving one body to both cannot express that: the detail
/// parser may well find fields in the autocomplete JSON, so the test silently
/// exercises the parse-SUCCESS path instead.
///
/// Each canned response sets `Connection: close`, so one request is one
/// connection and the sequence lines up with the request order.
pub async fn spawn_canned_http_server_seq(responses: Vec<(u16, &'static str)>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    assert!(!responses.is_empty(), "need at least one canned response");
    let responses = Arc::new(Mutex::new(responses));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind canned server");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let (status, body) = {
                let mut queued = responses.lock().unwrap();
                if queued.len() > 1 {
                    queued.remove(0)
                } else {
                    queued[0]
                }
            };
            tokio::spawn(async move {
                // Drain what the client sent; without this the write can race
                // the request and the client sees a reset instead of a reply.
                let mut buf = [0u8; 4096];
                let _ = sock.read(&mut buf).await;
                let response = format!(
                    "HTTP/1.1 {status} STATUS\r\nContent-Type: text/html\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.flush().await;
            });
        }
    });
    format!("http://{addr}")
}
