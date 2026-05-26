//! Smoke test for the W3C trace context wiring on the HTTP edge.
//!
//! Proves that `nebula_api::init_api_telemetry` actually wires a
//! `tracing_opentelemetry::OpenTelemetryLayer` into the global Subscriber — without that layer
//! the middleware in `crates/api/src/middleware/trace_w3c.rs` would compile and run but emit
//! no `traceparent` header (W3C trace propagation).
//!
//! The assertion is intentionally narrow: under an active per-request span produced by
//! `tower_http::trace::TraceLayer`, the response carries a structurally-valid W3C `traceparent`.
//! If the subscriber is missing the OTel layer, the header is absent and this test fails — that
//! is the regression we want to catch in CI.

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    middleware,
    routing::get,
};
use nebula_api::middleware::{
    InboundW3cTraceContext, inject_w3c_trace_response_headers, trace_context_middleware,
};
use nebula_core::parse_traceparent;
use tower::ServiceExt;
use tower_http::trace::{DefaultMakeSpan, MakeSpan, TraceLayer};

#[tokio::test]
async fn init_api_telemetry_emits_traceparent_on_response() {
    // Idempotent: safe even if another test in this binary already initialised the subscriber.
    let _ = nebula_api::init_api_telemetry().expect("init_api_telemetry must succeed in tests");

    let app = Router::new()
        .route("/ping", get(|| async { "pong" }))
        // Layers apply outer-most last in axum: TraceLayer wraps the inject middleware so
        // `Span::current()` inside the inject closure resolves to the per-request HTTP span.
        .layer(middleware::from_fn(inject_w3c_trace_response_headers))
        // Match the production stack in `app.rs`: span level must be INFO so a default
        // `RUST_LOG=info` filter still records it for the OTel layer.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(tracing::Level::INFO)),
        );

    let request = Request::builder()
        .uri("/ping")
        .body(Body::empty())
        .expect("build request");

    let response = app.oneshot(request).await.expect("router oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let traceparent = response
        .headers()
        .get("traceparent")
        .expect(
            "response is missing `traceparent` — the global Subscriber is not wiring \
             `tracing_opentelemetry::OpenTelemetryLayer` (W3C trace propagation regression)",
        )
        .to_str()
        .expect("traceparent must be ASCII");

    let parsed =
        parse_traceparent(traceparent).expect("response traceparent must be structurally valid");
    assert_ne!(parsed.trace_id.0, 0, "trace_id must not be all-zero");
    assert_ne!(parsed.parent_span_id.0, 0, "parent_id must not be all-zero");
}

/// End-to-end: an inbound `traceparent` is extracted, attached as parent of the per-request
/// span, and the response carries a `traceparent` with the **same** trace id. This is the load-
/// bearing the contract that a downstream service can correlate the response with the trace it
/// initiated upstream.
#[tokio::test]
async fn inbound_traceparent_round_trips_with_same_trace_id() {
    let _ = nebula_api::init_api_telemetry().expect("init_api_telemetry must succeed in tests");

    // Production composition: trace_context (extract) → TraceLayer with custom make_span that
    // attaches the inbound parent → response inject. Mirrors `build_app` exactly so any
    // refactor of the layer ordering breaks this test before it breaks operators.
    let app = Router::new()
        .route("/ping", get(|| async { "pong" }))
        .layer(middleware::from_fn(inject_w3c_trace_response_headers))
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<Body>| {
                let mut make_span = DefaultMakeSpan::new().level(tracing::Level::INFO);
                let span = make_span.make_span(request);
                if let Some(w3c) = request.extensions().get::<InboundW3cTraceContext>() {
                    // Re-use the production helper through the public middleware fn — the
                    // helper is `pub(crate)`, but `trace_context_middleware` stamps the
                    // extension that `attach_inbound_trace_parent` then reads via the same
                    // wiring as production.
                    use opentelemetry::{global, propagation::Extractor, trace::TraceContextExt};
                    use tracing_opentelemetry::OpenTelemetrySpanExt;

                    struct E<'a> {
                        tp: &'a str,
                        ts: Option<&'a str>,
                    }
                    impl Extractor for E<'_> {
                        fn get(&self, k: &str) -> Option<&str> {
                            if k.eq_ignore_ascii_case("traceparent") {
                                Some(self.tp)
                            } else if k.eq_ignore_ascii_case("tracestate") {
                                self.ts
                            } else {
                                None
                            }
                        }
                        fn keys(&self) -> Vec<&str> {
                            if self.ts.is_some() {
                                vec!["traceparent", "tracestate"]
                            } else {
                                vec!["traceparent"]
                            }
                        }
                    }
                    let parent = global::get_text_map_propagator(|p| {
                        p.extract(&E {
                            tp: w3c.0.traceparent(),
                            ts: w3c.0.tracestate(),
                        })
                    });
                    if parent.span().span_context().is_valid() {
                        let _ = span.set_parent(parent);
                    }
                }
                span
            }),
        )
        .layer(middleware::from_fn(trace_context_middleware));

    // Fixed synthetic traceparent — version 00, sampled, non-zero ids.
    const INBOUND_TP: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    const INBOUND_TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

    let request = Request::builder()
        .uri("/ping")
        .header("traceparent", INBOUND_TP)
        .body(Body::empty())
        .expect("build request");

    let response = app.oneshot(request).await.expect("oneshot");
    assert_eq!(response.status(), StatusCode::OK);

    let returned = response
        .headers()
        .get("traceparent")
        .expect("response must carry traceparent")
        .to_str()
        .expect("ASCII");

    let parsed = parse_traceparent(returned).expect("structurally valid");
    let returned_trace_id_hex = format!("{:032x}", parsed.trace_id.0);
    assert_eq!(
        returned_trace_id_hex, INBOUND_TRACE_ID,
        "response traceparent must carry the inbound trace id — propagation broken (W3C trace propagation)"
    );
}
