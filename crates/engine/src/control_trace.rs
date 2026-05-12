//! Link control-queue dispatch spans to W3C Trace Context persisted on [`ControlQueueEntry`]
//! (M3.5). Mirrors the API inbound attach path (`nebula_api::middleware::trace_w3c`) without
//! importing the API crate (layer boundary — `deny.toml`).

use nebula_core::W3cTraceContext;
use opentelemetry::global;
use opentelemetry::propagation::Extractor;
use opentelemetry::trace::TraceContextExt;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

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

/// Attach the remote OpenTelemetry parent from `w3c` to `span` when extraction yields a valid
/// trace. If invalid, leaves `span` as root — dispatch still proceeds (same non-fatal policy as
/// the HTTP edge).
///
/// **Redelivery / reclaim:** each `handle_entry` invocation builds a **new** dispatch `Span` and
/// re-attaches the **same** row carrier; there is no stacking of synthetic roots — the remote
/// parent id is fixed by the queue payload for that delivery attempt.
pub(crate) fn attach_control_queue_w3c_parent(span: &Span, w3c: &W3cTraceContext) {
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
        let _ = span.set_parent(parent);
        tracing::debug!(
            trace_id = %trace_id,
            "engine.control_trace: linked dispatch span to W3C parent from control-queue row"
        );
    } else {
        tracing::debug!(
            "engine.control_trace: W3C carrier on row did not yield valid OTel parent; dispatch span stays root"
        );
    }
}

#[cfg(test)]
mod tests {
    //! `RUST_LOG=debug` is optional when debugging propagator wiring.
    use super::*;
    use opentelemetry::trace::{TraceContextExt, TracerProvider as _};
    use std::sync::Mutex;
    use tracing_opentelemetry::OpenTelemetrySpanExt;
    use tracing_subscriber::prelude::*;

    static GLOBAL_PROPAGATOR_LOCK: Mutex<()> = Mutex::new(());

    /// Serialises tests that touch the process-global W3C text-map propagator.
    #[test]
    fn attach_wires_dispatch_span_to_carrier_trace_id() {
        let _lock = GLOBAL_PROPAGATOR_LOCK
            .lock()
            .expect("poisoned global propagator test lock");

        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder().build();
        let tracer = provider.tracer("nebula-engine.control_trace.test");
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::OpenTelemetryLayer::new(tracer));

        tracing::subscriber::with_default(subscriber, || {
            global::set_text_map_propagator(
                opentelemetry_sdk::propagation::TraceContextPropagator::new(),
            );

            let w3c = W3cTraceContext::from_traceparent_str(
                "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            )
            .expect("valid synthetic traceparent");

            let dispatch = tracing::info_span!("engine.control_queue.dispatch.test");
            attach_control_queue_w3c_parent(&dispatch, &w3c);

            let otel_ctx = dispatch.context();
            assert!(
                otel_ctx.span().span_context().is_valid(),
                "expected dispatch span to carry a valid OTel context"
            );
            assert_eq!(
                otel_ctx.span().span_context().trace_id().to_string(),
                "4bf92f3577b34da6a3ce929d0e0e4736",
                "dispatch span should inherit trace id from persisted W3C carrier"
            );
        });
    }
}
