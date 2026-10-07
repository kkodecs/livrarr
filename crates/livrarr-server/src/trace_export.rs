//! Request traces for a local trace viewer, sent over OTLP/HTTP with protobuf.
//!
//! `[log] trace_endpoint` turns this on. `request_span` opens one span per web
//! request; `request_trace_layer` exports those spans and nothing else. The span
//! records only the method, the matched route template and the status code; its
//! start and end give the duration. It never records the path, query string,
//! headers, body or any other request value.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use opentelemetry::trace::TracerProvider as _;
use std::io::Read;

use opentelemetry_proto::tonic::collector::trace::v1::{
    ExportTraceServiceRequest, ExportTraceServiceResponse,
};
use opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema;
use opentelemetry_proto::transform::trace::tonic::group_spans_by_resource_and_scope;
use opentelemetry_sdk::error::{OTelSdkError, OTelSdkResult};
use opentelemetry_sdk::trace::{SpanData, SpanExporter};
use prost::Message;
use tracing::Instrument;
use tracing_subscriber::filter::{FilterFn, Targets};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

/// The tracing target of the request span. The trace layer accepts only this
/// target; `not_request_spans` keeps it out of the log layers.
pub const REQUEST_SPAN_TARGET: &str = "request_trace";

/// The service name the viewer shows.
const SERVICE_NAME: &str = "livrarr";

/// How often queued spans are sent to the viewer.
const EXPORT_INTERVAL: Duration = Duration::from_secs(1);

/// How long one export may wait for the viewer before it is abandoned.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(5);

/// How many finished spans may wait for export; later spans are dropped.
const MAX_QUEUED_SPANS: usize = 2048;

/// How many spans one export sends at most.
const MAX_SPANS_PER_EXPORT: usize = 512;

/// The largest viewer reply read; a longer one counts as a failed export.
const MAX_REPLY_BYTES: u64 = 64 * 1024;

/// Axum middleware: runs the request inside a request span named
/// `"{method} {route template}"`, then records the response status on it.
/// A request that matched no route records the method only.
pub async fn request_span(request: Request, next: Next) -> Response {
    let span = tracing::info_span!(
        target: REQUEST_SPAN_TARGET,
        "request",
        otel.name = tracing::field::Empty,
        otel.kind = "server",
        http.request.method = tracing::field::Empty,
        http.route = tracing::field::Empty,
        http.response.status_code = tracing::field::Empty,
    );
    if span.is_disabled() {
        return next.run(request).await;
    }

    let method = method_name(request.method());
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map(|path| path.as_str().to_owned());
    let name = match &route {
        Some(route) => format!("{method} {route}"),
        None => method.to_owned(),
    };
    span.record("otel.name", name.as_str());
    span.record("http.request.method", method);
    if let Some(route) = &route {
        span.record("http.route", route.as_str());
    }

    let response = next.run(request).instrument(span.clone()).await;
    span.record(
        "http.response.status_code",
        i64::from(response.status().as_u16()),
    );
    response
}

/// The standard method name, or `_OTHER` for an extension method.
fn method_name(method: &axum::http::Method) -> &'static str {
    use axum::http::Method;
    match *method {
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::DELETE => "DELETE",
        Method::CONNECT => "CONNECT",
        Method::OPTIONS => "OPTIONS",
        Method::TRACE => "TRACE",
        Method::PATCH => "PATCH",
        _ => "_OTHER",
    }
}

/// The tracing layer that exports request spans to `traces_url` (an OTLP/HTTP
/// traces URL, as `LogConfig::trace_endpoint` gives it). It accepts only
/// request spans, records none of tracing's own location, thread, level, target
/// or timing fields, and sends spans in batches from a background thread, so a
/// slow or unreachable viewer never delays a request. Every export setting is
/// fixed here; no environment variable changes what is sent, where, or whether.
pub fn request_trace_layer<S>(traces_url: &str) -> Result<impl Layer<S> + Send + Sync, String>
where
    S: tracing::Subscriber + for<'span> LookupSpan<'span> + Send + Sync,
{
    let exporter = ViewerExporter::new(traces_url)?;
    let processor = opentelemetry_sdk::trace::BatchSpanProcessor::builder(exporter)
        .with_batch_config(
            opentelemetry_sdk::trace::BatchConfigBuilder::default()
                .with_scheduled_delay(EXPORT_INTERVAL)
                .with_max_queue_size(MAX_QUEUED_SPANS)
                .with_max_export_batch_size(MAX_SPANS_PER_EXPORT)
                .build(),
        )
        .build();
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_span_processor(processor)
        .with_sampler(opentelemetry_sdk::trace::Sampler::AlwaysOn)
        .with_span_limits(opentelemetry_sdk::trace::SpanLimits::default())
        .with_resource(service_resource())
        .build();

    Ok(tracing_opentelemetry::layer()
        .with_tracer(provider.tracer(SERVICE_NAME))
        .with_location(false)
        .with_threads(false)
        .with_level(false)
        .with_target(false)
        .with_tracked_inactivity(false)
        .with_filter(Targets::new().with_target(REQUEST_SPAN_TARGET, tracing::Level::TRACE)))
}

/// The resource every exported span carries: the service name only, with no
/// attribute read from the environment.
fn service_resource() -> opentelemetry_sdk::Resource {
    opentelemetry_sdk::Resource::builder_empty()
        .with_service_name(SERVICE_NAME)
        .build()
}

/// A filter that passes everything except request spans, for the log layers.
pub fn not_request_spans() -> FilterFn<impl Fn(&tracing::Metadata<'_>) -> bool> {
    FilterFn::new(|metadata: &tracing::Metadata<'_>| metadata.target() != REQUEST_SPAN_TARGET)
}

/// Sends span batches to the viewer as uncompressed OTLP protobuf in one HTTP
/// POST each, through its own client: no proxy, the fixed timeout, and no
/// headers but the content type. An export succeeds only when the viewer
/// answers 2xx with a readable, bounded `ExportTraceServiceResponse` (an empty
/// body is one) rejecting no span; a batch is never sent twice, and the
/// viewer's error message is never read into the log. It also reports the viewer's state in the
/// log: one warning when an export fails after a success (or as the first
/// export), and one line when an export succeeds after a failure. Nothing is
/// logged per span or request; these events carry no endpoint value or error
/// text, and the trace layer's filter never exports them.
#[derive(Debug)]
struct ViewerExporter {
    client: reqwest::blocking::Client,
    traces_url: String,
    resource: ResourceAttributesWithSchema,
    failing: AtomicBool,
}

impl ViewerExporter {
    /// The exporter for `traces_url`. Its blocking client is built on a thread
    /// of its own, since building one inside the async runtime panics.
    fn new(traces_url: &str) -> Result<Self, String> {
        let client = std::thread::spawn(|| {
            reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(EXPORT_TIMEOUT)
                .connect_timeout(EXPORT_TIMEOUT)
                .build()
        })
        .join()
        .map_err(|_| "trace exporter client thread failed".to_owned())?
        .map_err(|e| format!("trace exporter client: {e}"))?;
        Ok(Self {
            client,
            traces_url: traces_url.to_owned(),
            resource: ResourceAttributesWithSchema::from(&service_resource()),
            failing: AtomicBool::new(false),
        })
    }

    fn send(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let body = ExportTraceServiceRequest {
            resource_spans: group_spans_by_resource_and_scope(batch, &self.resource),
        }
        .encode_to_vec();
        let response = self
            .client
            .post(&self.traces_url)
            .timeout(EXPORT_TIMEOUT)
            .header(reqwest::header::CONTENT_TYPE, "application/x-protobuf")
            .body(body)
            .send()
            .map_err(|e| OTelSdkError::InternalFailure(e.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(OTelSdkError::InternalFailure(format!(
                "viewer answered {status}"
            )));
        }
        // The request's timeout is one deadline from connecting to the end of
        // the body, so a stalled or trickling reply fails within it.
        let mut reply = Vec::new();
        response
            .take(MAX_REPLY_BYTES + 1)
            .read_to_end(&mut reply)
            .map_err(|_| OTelSdkError::InternalFailure("viewer reply unreadable".to_owned()))?;
        if reply.len() as u64 > MAX_REPLY_BYTES {
            return Err(OTelSdkError::InternalFailure(
                "viewer reply too large".to_owned(),
            ));
        }
        let reply = ExportTraceServiceResponse::decode(reply.as_slice())
            .map_err(|_| OTelSdkError::InternalFailure("viewer reply malformed".to_owned()))?;
        let rejected = reply
            .partial_success
            .map_or(0, |partial| partial.rejected_spans);
        if rejected > 0 {
            return Err(OTelSdkError::InternalFailure(format!(
                "viewer rejected {rejected} spans"
            )));
        }
        Ok(())
    }

    fn note(&self, exported: bool) {
        let was_failing = self.failing.swap(!exported, Ordering::Relaxed);
        if exported && was_failing {
            tracing::info!(
                "Request tracing: sending traces again to the viewer at [log] trace_endpoint"
            );
        } else if !exported && !was_failing {
            tracing::warn!(
                "Request tracing: cannot send traces to the viewer at [log] trace_endpoint; \
                 requests are unaffected and sending is retried every second"
            );
        }
    }
}

impl SpanExporter for ViewerExporter {
    async fn export(&self, batch: Vec<SpanData>) -> OTelSdkResult {
        let result = self.send(batch);
        self.note(result.is_ok());
        result
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.resource = ResourceAttributesWithSchema::from(resource);
    }
}
