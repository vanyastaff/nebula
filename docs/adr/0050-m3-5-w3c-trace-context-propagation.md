# ADR-0050: M3.5 W3C Trace Context propagation (HTTP → queue → engine)

**Status:** Accepted (2026-05-11)  
**Tags:** observability, tracing, opentelemetry, api, engine, m3, m9

## Context

Operators need a single trace across the HTTP edge, asynchronous handoff through
`execution_control_queue`, and engine dispatch. W3C Trace Context is the
cross-vendor standard; Nebula already uses `tracing` and optional OTLP via
`nebula-log` in shipping binaries.

## Decision

1. **Shared carrier** — `nebula_core::obs::W3cTraceContext` holds validated
   `traceparent` (+ optional `tracestate`). No HTTP types in core.
2. **HTTP edge** — `nebula-api` middleware extracts inbound headers, stores the
   carrier on request extensions, links `tower_http::trace::TraceLayer` spans via
   `tracing_opentelemetry`, and injects response `traceparent` / `tracestate`
   where policy allows (including CORS expose rules).
3. **Binary init** — `nebula_api::init_api_telemetry` installs **both** a global
   `TraceContextPropagator` **and** a `tracing` Subscriber that includes
   `tracing_opentelemetry::OpenTelemetryLayer` over an exporter-less
   `SdkTracerProvider`. The layer is what makes
   `OpenTelemetrySpanExt::set_parent` / `Span::current().context()` non-empty;
   without it, the propagator install alone is a silent no-op for inbound
   attach, response injection, and control-queue carrier capture.
4. **Durable handoff** — `ControlQueueEntry::w3c_trace_context` persists the
   carrier (Postgres JSONB / SQLite TEXT, migration `0026_*`). API `enqueue_start`
   and `cancel_execution` stamp the active span when possible; enqueue still
   succeeds if capture fails.
5. **Engine** — `ControlConsumer` builds `engine.control_queue.dispatch`, calls
   `attach_control_queue_w3c_parent` when the row carries a carrier, then
   dispatches commands. Cooperative cancel: when a node returns
   `ActionError::Cancelled` while the execution `CancellationToken` is already
   cancelled, the frontier tears down like an external cancel — **not** as a
   synthetic `failed_node` — so final status stays `Cancelled` (ADR-0008 A3).
6. **Actions** — `ActionRuntimeContext::instrument_resource_http_request` (and
   `resource_http_request_span`) names outbound resource HTTP under
   `nebula.action.resource_http.request` with host/scheme fields only (no path,
   no secrets).

## M9.2 gate (OTLP exporter)

Works today **without** OTLP:

- W3C parsing / validation (`nebula_core::obs`) and serde round-trip;
- inbound `traceparent` → per-request `tracing::Span` parent attach (via the
  Subscriber installed by `init_api_telemetry`);
- response `traceparent` / `tracestate` injection;
- control-queue row stamping (`w3c_trace_context_for_control_queue`) and engine
  consumer re-attach (`attach_control_queue_w3c_parent`).

Gated on M9.2:

- shipping the same trace to an external collector (OTLP exporter wiring lives
  in `nebula-log::telemetry::otel` behind the `telemetry` feature and only
  activates when an OTLP endpoint is configured);
- end-to-end backend verification against a real collector.

## Consequences

- New migration and column shape; older engines ignore unknown JSON fields until
  upgraded.
- Tests that touch `opentelemetry::global::set_text_map_propagator` must
  serialise (mutex in `control_trace` unit tests) to avoid cross-test pollution.

## References

- `crates/api/src/middleware/trace_w3c.rs`, `crates/api/src/trace_capture.rs`
- `crates/engine/src/control_trace.rs`, `crates/engine/src/control_consumer.rs`
- `crates/action/src/context.rs` (resource HTTP spans)
