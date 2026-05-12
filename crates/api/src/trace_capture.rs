//! Capture W3C Trace Context from the active HTTP span for durable control-queue rows (M3.5).
//!
//! Uses the same global `TraceContextPropagator` as inbound extraction / response injection
//! (`telemetry_init`, `middleware::trace_w3c`). If injection or [`nebula_core::W3cTraceContext`]
//! validation fails, callers **must still enqueue** — see `w3c_trace_context_for_control_queue`.

use nebula_core::W3cTraceContext;
use opentelemetry::global;
use opentelemetry::propagation::Injector;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

struct W3cHeaderCapture {
    traceparent: Option<String>,
    tracestate: Option<String>,
}

impl Injector for W3cHeaderCapture {
    fn set(&mut self, key: &str, value: String) {
        if key.eq_ignore_ascii_case("traceparent") {
            self.traceparent = Some(value);
        } else if key.eq_ignore_ascii_case("tracestate") {
            self.tracestate = Some(value);
        }
    }
}

/// Build a validated [`W3cTraceContext`] from the current tracing span's OpenTelemetry context.
///
/// Returns `None` when the propagator does not emit a `traceparent`, or when
/// [`W3cTraceContext::from_optional_headers`] rejects the injected strings. The latter is logged at
/// **`WARN`** with the typed [`nebula_core::W3cTraceContextError`] display (static reasons only on
/// that type — never raw hostile header blobs at INFO+).
///
/// **Policy (M3.5):** treat all failures as non-fatal — enqueue control commands **without** a
/// carrier rather than failing the HTTP request.
#[must_use]
pub(crate) fn w3c_trace_context_for_control_queue() -> Option<W3cTraceContext> {
    let span = Span::current();
    let cx = span.context();
    let mut capture = W3cHeaderCapture {
        traceparent: None,
        tracestate: None,
    };
    global::get_text_map_propagator(|prop| {
        prop.inject_context(&cx, &mut capture);
    });

    match W3cTraceContext::from_optional_headers(
        capture.traceparent.as_deref(),
        capture.tracestate.as_deref(),
    ) {
        Ok(opt) => {
            if let Some(ref ctx) = opt {
                tracing::debug!(
                    traceparent_len = ctx.traceparent().len(),
                    has_tracestate = ctx.tracestate().is_some(),
                    "w3c_capture: control queue stamp from active span"
                );
            } else {
                tracing::debug!(
                    "w3c_capture: no valid traceparent from active span; control queue row omits W3C carrier"
                );
            }
            opt
        },
        Err(err) => {
            tracing::warn!(
                error = %err,
                "w3c_capture: invalid W3C values after OTel inject; omitting carrier (non-fatal)"
            );
            None
        },
    }
}
