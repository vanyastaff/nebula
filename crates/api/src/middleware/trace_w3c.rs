//! W3C Trace Context extraction (`traceparent` / `tracestate`).
//!
//! Runs **before** `tower_http::trace::TraceLayer` so the per-request span can
//! attach a remote parent (see `build_app` layer ordering in `app.rs`).

use axum::{
    extract::Request,
    http::{HeaderMap, HeaderName, HeaderValue},
    middleware::Next,
    response::Response,
};
use nebula_core::{W3C_TRACEPARENT, W3C_TRACESTATE, W3cTraceContext, W3cTraceContextError};
use opentelemetry::global;
use opentelemetry::propagation::Extractor;
use opentelemetry::trace::TraceContextExt;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Request extension: validated inbound W3C trace context (if any).
#[derive(Debug, Clone)]
pub struct InboundW3cTraceContext(pub W3cTraceContext);

/// Extract `traceparent` / `tracestate`, validate via [`nebula_core::obs`], store on extensions.
///
/// Invalid headers do **not** fail the request; they are logged at `WARN` with a static reason
/// and omitted from extensions (new root trace in `TraceLayer`).
pub async fn trace_context_middleware(mut request: Request, next: Next) -> Response {
    let traceparent = request
        .headers()
        .get(W3C_TRACEPARENT)
        .and_then(|v| v.to_str().ok());
    let tracestate = request
        .headers()
        .get(W3C_TRACESTATE)
        .and_then(|v| v.to_str().ok());

    match W3cTraceContext::from_optional_headers(traceparent, tracestate) {
        Ok(Some(ctx)) => {
            tracing::debug!(
                traceparent_len = ctx.traceparent().len(),
                has_tracestate = ctx.tracestate().is_some(),
                "w3c_trace_context: accepted inbound trace context"
            );
            request.extensions_mut().insert(InboundW3cTraceContext(ctx));
        },
        Ok(None) => {
            tracing::debug!("w3c_trace_context: no traceparent header");
        },
        Err(err) => {
            let (reason, field) = w3c_error_fields(&err);
            tracing::warn!(
                w3c.reason = reason,
                w3c.field = field,
                "w3c_trace_context: rejected inbound trace context"
            );
        },
    }

    next.run(request).await
}

fn w3c_error_fields(err: &W3cTraceContextError) -> (&'static str, &'static str) {
    match err {
        W3cTraceContextError::InvalidTraceparent { reason } => (*reason, "traceparent"),
        W3cTraceContextError::InvalidTracestate { reason } => (*reason, "tracestate"),
    }
}

struct W3cTraceExtractor<'a> {
    traceparent: &'a str,
    tracestate: Option<&'a str>,
}

impl Extractor for W3cTraceExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        if key.eq_ignore_ascii_case("traceparent") {
            return Some(self.traceparent);
        }
        if key.eq_ignore_ascii_case("tracestate") {
            return self.tracestate;
        }
        None
    }

    fn keys(&self) -> Vec<&str> {
        if self.tracestate.is_some() {
            vec!["traceparent", "tracestate"]
        } else {
            vec!["traceparent"]
        }
    }
}

/// Inject `traceparent` / `tracestate` for the **current** tracing span onto the HTTP response.
///
/// Must run **inside** `tower_http::trace::TraceLayer`'s async scope (see `build_app` layer order)
/// so [`Span::current()`] resolves to the per-request HTTP span after `set_parent` wiring.
pub async fn inject_w3c_trace_response_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    inject_current_trace_into_response_headers(response.headers_mut());
    tracing::debug!("w3c_trace_context: injected W3C trace headers on HTTP response");
    response
}

struct HttpHeaderInjector<'a>(&'a mut HeaderMap);

impl opentelemetry::propagation::Injector for HttpHeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        let Ok(name) = HeaderName::from_bytes(key.as_bytes()) else {
            return;
        };
        let Ok(val) = HeaderValue::from_str(&value) else {
            return;
        };
        self.0.insert(name, val);
    }
}

fn inject_current_trace_into_response_headers(headers: &mut HeaderMap) {
    let span = Span::current();
    let cx = span.context();
    global::get_text_map_propagator(|prop| {
        prop.inject_context(&cx, &mut HttpHeaderInjector(headers));
    });
}

/// Attach the remote OpenTelemetry parent carried in `w3c` to `span` when extraction is valid.
pub(crate) fn attach_inbound_trace_parent(span: &Span, w3c: &W3cTraceContext) {
    let parent = global::get_text_map_propagator(|prop| {
        prop.extract(&W3cTraceExtractor {
            traceparent: w3c.traceparent(),
            tracestate: w3c.tracestate(),
        })
    });

    let (is_valid, trace_id) = {
        let otel_span = parent.span();
        let ctx = otel_span.span_context();
        (ctx.is_valid(), ctx.trace_id())
    };
    if is_valid {
        match span.set_parent(parent) {
            Ok(()) => tracing::debug!(
                trace_id = %trace_id,
                "w3c_trace_context: linked HTTP span to remote OpenTelemetry parent"
            ),
            Err(err) => tracing::warn!(
                trace_id = %trace_id,
                error = ?err,
                "w3c_trace_context: span.set_parent failed after carrier validation; span stays root"
            ),
        }
    } else {
        tracing::debug!(
            "w3c_trace_context: extracted OpenTelemetry context invalid; span stays root"
        );
    }
}
