# Recon — M3.5 / M9.2 OpenTelemetry tracing + metrics end-to-end

**Scope:** read-only inventory of OTLP / W3C / collector wiring for the M3.5
trace-propagation closure and the M9.2 OTLP exporter bridge.

**Anchors (canon):**
- ROADMAP §M3.5 — `docs/ROADMAP.md:248-269` (in-process pieces done, integration test + OTLP exporter open).
- ROADMAP §M9.2 — `docs/ROADMAP.md:552-565` (OTLP bridge verification, references #598).
- ADR-0050 — `docs/adr/0050-m3-5-w3c-trace-context-propagation.md:1-78` (binary init contract + M9.2 gate).
- ADR-0046 — `docs/adr/0046-metrics-telemetry-boundary.md:1-176` (merged crate, OTLP exporter explicitly out of scope).

---

## 1. W3C trace context middleware (M3.5)

### Middleware location
- `crates/api/src/middleware/trace_w3c.rs:1-159` — single module; module-level docstring says it runs **before** `tower_http::trace::TraceLayer` (`crates/api/src/middleware/trace_w3c.rs:3-5`).
- Public re-exports: `crates/api/src/middleware/mod.rs:26-28` (`InboundW3cTraceContext`, `inject_w3c_trace_response_headers`, `trace_context_middleware`).

### What it parses
- `traceparent` header — `crates/api/src/middleware/trace_w3c.rs:31-34` (read via `nebula_core::W3C_TRACEPARENT`).
- `tracestate` header — `crates/api/src/middleware/trace_w3c.rs:35-38` (`W3C_TRACESTATE`).
- Validation routed through `nebula_core::W3cTraceContext::from_optional_headers` — `crates/api/src/middleware/trace_w3c.rs:40` (definition: `crates/core/src/obs.rs:74-87`).
- Parser entry points in core: `parse_traceparent` `crates/core/src/obs.rs:172-175`; `from_traceparent_str` `crates/core/src/obs.rs:90-101`.
- Invalid headers do **not** fail the request — they log WARN with a typed static `reason` field (`crates/api/src/middleware/trace_w3c.rs:50-55`).

### Attach to per-request span
- `attach_inbound_trace_parent` — `crates/api/src/middleware/trace_w3c.rs:133-159` extracts via the global `TextMapPropagator` and calls `span.set_parent(...)` from `tracing_opentelemetry::OpenTelemetrySpanExt`.
- Wired into `TraceLayer::make_span_with` inside `build_app` — `crates/api/src/app.rs:156-165` (custom closure reads `InboundW3cTraceContext` from request extensions and forwards to `attach_inbound_trace_parent`).
- The `InboundW3cTraceContext` extension is stamped by `trace_context_middleware` — `crates/api/src/middleware/trace_w3c.rs:40-46`.
- Layer ordering (production): `trace_context_middleware` runs **before** `TraceLayer` so the extension is present when the span is built. See `crates/api/src/app.rs:180-186` (middleware applied bottom-up: `trace_context_middleware` is applied after `middleware_stack` which holds the `TraceLayer`, so it executes first per axum semantics).

### Response emission
- `inject_w3c_trace_response_headers` — `crates/api/src/middleware/trace_w3c.rs:96-103` injects via `global::get_text_map_propagator` over `HttpHeaderInjector` (`crates/api/src/middleware/trace_w3c.rs:105-117`).
- Mounted inside `TraceLayer`'s span scope at `crates/api/src/app.rs:167-170` (note the `inside TraceLayer` invariant in the docstring comment).
- CORS alignment — `traceparent`/`tracestate` are both in `allow_headers` (`crates/api/src/app.rs:338-341`) and `expose_headers` (`crates/api/src/app.rs:347-352`) inside `build_cors_layer`.

### Outbound capture for control queue
- `crates/api/src/trace_capture.rs:1-75` — `w3c_trace_context_for_control_queue()` injects the current span via the propagator and validates it through `W3cTraceContext::from_optional_headers` (`crates/api/src/trace_capture.rs:51-72`).
- Callers: `crates/api/src/domain/execution/handler.rs:403,510,723` (start / cancel / terminate).
- Storage stamp: `crates/api/src/state.rs:849,911` (`enqueue_control_scoped`, `cas_transition_with_control_scoped`) write `w3c_traceparent` into `ControlMsg`.

---

## 2. `OpenTelemetryLayer` install points

### API binary telemetry bootstrap
- `crates/api/src/telemetry_init.rs:1-60` — `init_api_telemetry()` is the single install entry point.
  - Sets the W3C propagator — `telemetry_init.rs:35` (`global::set_text_map_propagator(TraceContextPropagator::new())`).
  - Builds an **exporter-less** `SdkTracerProvider` — `telemetry_init.rs:37-38`.
  - Installs `tracing_opentelemetry::OpenTelemetryLayer::new(tracer)` into the subscriber — `telemetry_init.rs:42-50`.
  - Idempotent via `try_init` — `telemetry_init.rs:48-58`.
- Public re-export: `crates/api/src/lib.rs:89` (`pub use telemetry_init::init_api_telemetry;`).
- Called from `apps/server/src/main.rs:19` (the only shipping binary entry point).

### `TraceLayer` install
- `crates/api/src/app.rs:12` — import (`tower_http::trace::{DefaultMakeSpan, MakeSpan, TraceLayer}`).
- `crates/api/src/app.rs:156-165` — `TraceLayer::new_for_http().make_span_with(...)` with custom closure that pulls `InboundW3cTraceContext` and links the inbound parent. Span level forced to `INFO` (`app.rs:158`) so the default `RUST_LOG=info` filter still records it (comment at `app.rs:152-155` documents the reasoning).

### ADR-0050 wiring location
- The exporter-less `SdkTracerProvider` + `OpenTelemetryLayer` install is at `crates/api/src/telemetry_init.rs:37-50`. ADR-0050 §"Binary init" `docs/adr/0050-m3-5-w3c-trace-context-propagation.md:21-30` names this as the load-bearing wire.
- Smoke regression test: `crates/api/tests/trace_w3c_smoke.rs:28-66` (`init_api_telemetry_emits_traceparent_on_response`) — proves the layer is wired (without it, response carries no `traceparent`).
- A **second** OTLP-capable `OpenTelemetryLayer` exists in `nebula-log` at `crates/log/src/telemetry/otel.rs:91-138`, gated by `feature = "telemetry"` (`crates/log/src/builder/mod.rs:171-194`), but is **not** wired into the API binary today — `apps/server/src/main.rs` calls `nebula_api::init_api_telemetry()` only and never `nebula_log::init_with(...)`.

---

## 3. Engine-side trace propagation (M3.5 closed engine-side)

### Control queue propagation
- Persisted carrier: `ControlMsg.w3c_traceparent: Option<String>` — `crates/storage-port/src/dto/control.rs:42-56`.
- API enqueue path stamps the carrier — `crates/api/src/state.rs:849` and `crates/api/src/state.rs:911`.
- Engine consumer re-attach: `crates/engine/src/control_consumer.rs:597-607`:
  ```rust
  let span = tracing::info_span!("engine.control_queue.dispatch", ...);
  if let Some(ref w3c) = w3c_opt {
      crate::control_trace::attach_control_queue_w3c_parent(&span, w3c);
  }
  ```
- Carrier normalize / validate (per-row, non-fatal) — `crates/engine/src/control_consumer.rs:298-326` (`RawClaimed::normalize`).
- Helper: `crates/engine/src/control_trace.rs:44-78` — `attach_control_queue_w3c_parent`. Mirrors API inbound logic but **does not** import `nebula-api` (layer boundary noted in module docs `control_trace.rs:1-3`).
- Engine README mentions ADR-0050 wiring: `crates/engine/README.md:44`.

### Context type that carries the trace into action execution
- `nebula_action::ActionRuntimeContext` — `crates/action/src/context.rs:140-159` (struct), constructed inside the engine at `crates/engine/src/engine.rs:4145-4151`.
- The trace flow is **not** an explicit field on the context — it propagates through the surrounding `tracing::Span` that the `engine.control_queue.dispatch` span installs via `.instrument(span).await` (`crates/engine/src/control_consumer.rs:640`). Subsequent `tracing::Span::current()` reads inside action execution inherit that parent.
- Outbound resource HTTP span anchor: `ActionRuntimeContext::resource_http_request_span` `crates/action/src/context.rs:254-272` (creates `nebula.action.resource_http.request` debug span with host/scheme only — no path, no userinfo, per ADR-0050 §6).
- Instrumented wrapper: `ActionRuntimeContext::instrument_resource_http_request` `crates/action/src/context.rs:282-312`.

### Test #661 (full-stack one-root-span)
- **[NOT FOUND]** — no integration test asserts a single root span across API → control queue → engine → action.
- Subset coverage today:
  - `crates/engine/src/control_trace.rs:79-126` — unit test (`attach_wires_dispatch_span_to_carrier_trace_id`) proves the engine dispatch span inherits the carrier trace id.
  - `crates/api/tests/trace_w3c_smoke.rs:73-159` — `inbound_traceparent_round_trips_with_same_trace_id` proves API edge round-trip.
  - `crates/engine/tests/lease_takeover.rs:647` — exercises the consumer but always passes `w3c_traceparent: None`.
  - `crates/engine/tests/control_consumer_wiring.rs:61` — same (no W3C carrier in tests).
- ROADMAP §M3.5 explicitly tracks this as open: `docs/ROADMAP.md:264-266` ("Integration test: full stack API → engine → action → resource with one root span ... (engine `control_trace` + lease-takeover cancel path cover subsets today)").
- #661 referenced in git log only as the merged W3C work (`docs/ROADMAP.md:72,260` — "DONE via #661"), **not** as an open integration test number. The "test 661" framing in the task is the same PR number; the open test does not yet exist.

---

## 4. `nebula-metrics` post-ADR-0046 state

### Crate layout
- `crates/metrics/src/lib.rs:1-58` — flat module layout (ADR-0046 §"Flat module layout" `docs/adr/0046-metrics-telemetry-boundary.md:46-65`).

Files (single-line role):
1. `crates/metrics/src/counter.rs` — `Counter` primitive (atomic u64).
2. `crates/metrics/src/gauge.rs` — `Gauge` primitive.
3. `crates/metrics/src/histogram.rs` — `Histogram`, `HistogramSnapshot`, default bucket layout.
4. `crates/metrics/src/labels.rs` — `LabelInterner`, `LabelSet`, `MetricKey`, `LabelKey`, `LabelValue`.
5. `crates/metrics/src/registry.rs` — concurrent `MetricsRegistry` (DashMap + interner).
6. `crates/metrics/src/filter.rs` — `LabelAllowlist` (cardinality guard; `filter.rs:41`).
7. `crates/metrics/src/naming.rs` — `NEBULA_*` constants + per-counter outcome label modules (~50 metric names; `naming.rs:1-900+`).
8. `crates/metrics/src/prometheus.rs` — `PrometheusExporter`, `snapshot()` text-format renderer.
9. `crates/metrics/src/eventbus.rs` — `record_eventbus_stats(EventBusStats)` adapter.
10. `crates/metrics/src/error.rs` — `MetricsError`, `MetricsResult`, `MetricKind`.
11. `crates/metrics/src/prelude.rs` — convenience re-exports.

### OTLP exporter status
- **No OTLP code in `crates/metrics/`.** Verified by grep at `crates/metrics/*` — only doc references: `crates/metrics/src/registry.rs:285` ("Used by exporters (Prometheus, OTLP)") and `crates/metrics/src/histogram.rs:228` ("Intended for exposition (Prometheus, OTLP)"). No `opentelemetry-otlp` dep in `crates/metrics/Cargo.toml:14-21`.
- The crate README is explicit: `crates/metrics/README.md:76` ("Not an OTLP exporter — Prometheus text is the only current export format") and `crates/metrics/README.md:86` ("OTLP export is `planned`; Prometheus text is the implemented export format").
- The only OTLP exporter in the workspace is **for traces only**, in `nebula-log`: `crates/log/src/telemetry/otel.rs:193-198` (`SpanExporter`). No metrics exporter (`SdkMeterProvider` / `MeterProvider`) anywhere — grep across the workspace returns zero hits.
- ADR-0046 explicitly defers OTLP exporter wiring to M9.2 (`docs/adr/0046-metrics-telemetry-boundary.md:36`).
- Canon "bridge missing or only partially wired" claim is confirmed: traces side has a `nebula-log`-gated OTLP exporter that is **never installed in any shipping binary**; metrics side has nothing at all.

### `MetricsRegistry` snapshot path
- `MetricsRegistry::snapshot_counters` — `crates/metrics/src/registry.rs:286-295`.
- `MetricsRegistry::snapshot_gauges` — `crates/metrics/src/registry.rs:297-306`.
- `MetricsRegistry::snapshot_histograms` — `crates/metrics/src/registry.rs:308-317`.
- These three are the documented OTLP integration seam: `crates/metrics/src/registry.rs:285` ("Used by exporters (Prometheus, OTLP) to serialize the current state").
- Free-function `nebula_metrics::snapshot(registry)` — `crates/metrics/src/prometheus.rs:367-475`.
- HTTP `/metrics` mount — `crates/api/src/domain/metrics.rs:14-39` (`prometheus_handler` calls `nebula_metrics::snapshot(registry)` and `nebula_metrics::content_type()`).

### Cardinality regression test (M3.3 hand-off)
- **[NOT FOUND]** as a regression test under that name.
- ROADMAP §M3.3 lists it explicitly as deferred: `docs/ROADMAP.md:236-239` ("Per-outcome `REQUESTS_TOTAL` + latency histogram + **cardinality regression test land as a 1.0 follow-up**").
- The cardinality budget for `NEBULA_WEBHOOK_REQUESTS_TOTAL` lives only as a comment in the naming module: `crates/metrics/src/naming.rs:253-257`.
- Closest existing artifacts (none are a regression test):
  - `crates/metrics/examples/cardinality_guard.rs:1-115` — demo only.
  - `crates/metrics/src/filter.rs:155-210` — unit tests for `LabelAllowlist` (functional, not a webhook regression).
- The "M3.3 hand-off" line is a roadmap-tracked debt, not a shipped test.

---

## 5. `nebula-log` OTLP integration

- OTLP module: `crates/log/src/telemetry/otel.rs:1-310` (single file).
- `build_layer` (the trace-side OTLP entry point) — `crates/log/src/telemetry/otel.rs:91-138`.
  - Uses `opentelemetry_otlp::SpanExporter::builder().with_tonic().with_endpoint(...)` — `crates/log/src/telemetry/otel.rs:193-198`.
  - Returns an `OpenTelemetryLayer` boxed into a `tracing-subscriber` layer — `crates/log/src/telemetry/otel.rs:131-137`.
  - Builds via `Sampler::AlwaysOn|AlwaysOff|TraceIdRatioBased` based on `TelemetryConfig.sampling_rate` — `crates/log/src/telemetry/otel.rs:93-100`.
  - Batch exporter when a tokio runtime is present, simple exporter otherwise — `crates/log/src/telemetry/otel.rs:114-124`.
- Endpoint resolution: `resolve_endpoint_from` `crates/log/src/telemetry/otel.rs:42-63` (config wins → env fallback → off; literal `disabled` and empty strings opt out — issue #375 in comments).
- Globals install only after subscriber `try_init` succeeds: `install_globals` `crates/log/src/telemetry/otel.rs:163-167`; `shutdown_unused_provider` `crates/log/src/telemetry/otel.rs:177-183` (issue #380).
- Builder wiring inside the subscriber stack: `crates/log/src/builder/mod.rs:171-194` (gated by `feature = "telemetry"`).
- **Not wired into the API binary**: `apps/server/src/main.rs:19` calls `nebula_api::init_api_telemetry()` only. The API crate does not depend on `nebula-log`. Grep for `nebula_log::init` across the repo returns only examples + tests (no shipping binary).

### Env var handling
- `OTEL_EXPORTER_OTLP_ENDPOINT` read in `resolve_endpoint` — `crates/log/src/telemetry/otel.rs:69`.
- Doc-only references in `crates/api/src/telemetry_init.rs:15` ("OTLP shipping is the M9.2 gate ... layered on top by `nebula-log` when operators enable it") and `crates/log/docs/Integration.md:125`.

### File rolling + runtime reload state
- File rolling appender: `crates/log/src/writer.rs:355-377` (`tracing_appender::rolling::{hourly, daily, never}` + size-rolling via `RollingFileAppender::builder()`).
- Size guards: `crates/log/src/writer.rs:484-505` (overflow / zero-size rejected at config time).
- Runtime reload: `crates/log/src/builder/reload.rs:11-117` (`ReloadHandle::reload` parses + swaps `EnvFilter`).
- Reload + watcher: `crates/log/src/builder/watcher.rs:1-200` (auto-reload on config-file change via `notify`).
- Roadmap status: ROADMAP §M14.6 `docs/ROADMAP.md:1018-1020` — "File rolling + runtime reload audit under load" still open.

---

## 6. Local OTLP collector infrastructure

### `task obs:up`
- `Taskfile.yml:286-289`:
  ```yaml
  obs:up:
    desc: Start observability stack (Jaeger + OTEL collector)
    cmds:
      - docker compose -f {{.COMPOSE_OBSERVABILITY}} up -d
  ```
- `obs:down` companion — `Taskfile.yml:291-294`.
- `COMPOSE_OBSERVABILITY` resolved at `Taskfile.yml:14` to `'deploy/docker/docker-compose.observability.yml'`.

### Compose file
- **[NOT FOUND]** — `deploy/docker/docker-compose.observability.yml` does **not exist**. The `deploy/` directory itself does not exist (`ls C:/Users/vanya/RustroverProjects/nebula/deploy` returns "No such file or directory"). The `task obs:up` target references a file that has not been committed.
- The sibling compose files (`COMPOSE_LOCAL`, `COMPOSE_SELFHOSTED` at `Taskfile.yml:12-13`) also point at `deploy/docker/...` paths that are absent.
- (Historical: agent-template compose YAMLs used to live under `.claude/skills/aif-dockerize/templates/`; those AI Factory skill packs were retired entirely, so the only compose-template surface in the tree today is whatever lands under `deploy/` — currently nothing.)

### `OTEL_EXPORTER_OTLP_ENDPOINT` env handling
- Read at runtime only in `crates/log/src/telemetry/otel.rs:69` (`resolve_endpoint`).
- Treated as opt-in / opt-out per #375 — empty, `"disabled"`, missing all map to OTLP off (`otel.rs:42-63`).
- Documented for operators: `crates/log/docs/OPERATIONS.md:23`, `crates/log/docs/Integration.md:125,204`, `crates/log/README.md:86`.
- No `.env`/`.env.example` reference visible in repo top level for OTLP (Taskfile loads `deploy/.env` + `deploy/.env.example` — `Taskfile.yml:6-8`, neither of which exists).

---

## 7. Existing OTLP tests

No integration test exports to a live OTLP endpoint. The only tests touching the OTLP code path are:

1. `crates/log/src/telemetry/otel.rs:200-310` (in-file `#[cfg(test)] mod tests`):
   - `unset_config_and_env_is_opt_out` (line 213-217) — pure resolve test, no exporter.
   - `empty_env_is_opt_out` (220-225), `disabled_env_is_opt_out` (228-232), `empty_config_wins_over_env` (236-243), `disabled_config_wins_over_env` (245-253), `config_endpoint_wins_over_env` (256-262), `env_used_when_config_none` (265-271) — same.
   - `build_layer_then_shutdown_is_safe` (281-307) — builds an `OtelLayer` against unreachable `http://127.0.0.1:1`, immediately calls `shutdown_unused_provider`. **No network export attempted**; just exercises construct/teardown.
2. `crates/log/tests/init_hardening.rs:40-50` — references `otlp_endpoint: Some("http://127.0.0.1:1")` for the unreachable-endpoint init hardening; not an export test.
3. `crates/log/examples/otlp_setup.rs:1-90` — runnable example only (not a `#[test]`).

W3C / trace-context tests (not OTLP export, but related):
- `crates/api/tests/trace_w3c_smoke.rs:28-159` — two `#[tokio::test]` cases (response emits `traceparent`; inbound trace id round-trips).
- `crates/engine/src/control_trace.rs:79-126` — `attach_wires_dispatch_span_to_carrier_trace_id` unit test.

**Result:** zero integration tests exercise actual OTLP export to a collector. The "verified via Jaeger UI probe or collector debug output" goal at `docs/ROADMAP.md:558-562` is unmet.

---

## 8. Issue #598

- ROADMAP §M9.2 lists "Read #598 + comments to capture the open question" as the **first** unchecked task: `docs/ROADMAP.md:554`.
- ADR-0046 explicitly references it: `docs/adr/0046-metrics-telemetry-boundary.md:8` ("#598 (telemetry: verify OpenTelemetry setup against bridge-pattern guide). Referenced for context — this ADR does **not** claim to close them").
- **No pinned issue file** (no `docs/issues/598.md` — grep returns nothing other than the roadmap line and the ADR-0046 line).
- The issue exists only as a GitHub issue tracker reference; the open question is "verify OpenTelemetry setup against the bridge-pattern guide", i.e. the same question this recon is mapping.

---

## 9. Cross-dep readiness

### `nebula-eventbus` migration of `ExecutionEvent` (M14.2)
- ROADMAP §M14.2 `docs/ROADMAP.md:970-979` lists `ExecutionEvent` migration as **open** ("still on raw mpsc and multi-subscriber consumers reinvent the channel").
- **Code reality contradicts the roadmap**: `ExecutionEvent` is already on `nebula_eventbus::EventBus<ExecutionEvent>`:
  - Type alias: `crates/engine/src/engine.rs:65` (`type EventBus = nebula_eventbus::EventBus<ExecutionEvent>;`).
  - Engine field: `crates/engine/src/engine.rs:242` (`event_bus: Option<EventBus>`).
  - Builder: `crates/engine/src/engine.rs:941-944` (`with_event_bus`).
  - Publisher: `crates/engine/src/engine.rs:952-957` (`emit_event` — single `broadcast::send` via `bus.emit(event)`).
  - All emit sites use the bus, not mpsc: `engine.rs:1178, 1675, 2145, 2408, 2513, 2572, 2861, 3075, 3142, 3645`.
  - Bounded fan-out documented at `crates/engine/src/engine.rs:55-72`.
- Tests subscribe via `EventBus::subscribe()` — e.g. `crates/engine/src/engine.rs:7068-7073, 7233-7238, 7980-8016`; integration test usage at `crates/engine/tests/lease_takeover.rs:360, 572, 809`.
- The only remaining `event_sender` mention is a stale docstring at `crates/engine/src/event.rs:3` ("Subscribe via `WorkflowEngine::with_event_sender`") — the actual method is `with_event_bus`. Roadmap entry appears to lag the code.

### ADR-0046 closed-boundary work
- ADR-0046 — `docs/adr/0046-metrics-telemetry-boundary.md:1-176`. Merged `nebula-telemetry` into `nebula-metrics` as a single observability crate; flat module layout (`docs/adr/0046-metrics-telemetry-boundary.md:46-65`).
- Status: accepted, status section at `docs/adr/0046-metrics-telemetry-boundary.md:3`. ROADMAP §M9.4 marks the boundary work `[x] DONE 2026-05-06`: `docs/ROADMAP.md:577-581`.
- Exporter impact: ADR-0046 deletes `TelemetryAdapter` and folds primitives + Prometheus exporter into one crate (`adr/0046-metrics-telemetry-boundary.md:71-72, 95-100`). The `MetricsRegistry::snapshot_*` methods are stable and are the documented OTLP-bridge seam (`crates/metrics/src/registry.rs:285`).
- The next step the ADR explicitly defers ("comprehensive observability re-audit ... after the merge implementation lands" — `adr/0046-metrics-telemetry-boundary.md:10`) is the M9.2 gate this recon supports.

---

## 10. Gaps for PR plan

For each goal the plan needs to close, here are the concrete missing file:lines, anchored to the existing surface.

### Goal 1 — OTLP traces exporter installed in API + engine binaries

- `crates/api/src/telemetry_init.rs:37-50` — the `SdkTracerProvider` is built **exporter-less** (no `.with_batch_exporter(...)` call). To wire OTLP: add an env-driven exporter setup branch parallel to `nebula-log`'s `build_layer` (`crates/log/src/telemetry/otel.rs:91-138`). Either copy the resolver in-place (~50–80 LOC: resolver + exporter build + provider builder + shutdown guard) or take a `nebula-log` dep and call its `build_layer` here.
- `apps/server/src/main.rs:19` — only `nebula_api::init_api_telemetry()` is called. Either:
  - extend `init_api_telemetry` to install the OTLP exporter when `OTEL_EXPORTER_OTLP_ENDPOINT` is set (~30 LOC + the resolver above), **or**
  - add a `nebula_log::init_with(...)` call alongside it (~15 LOC) and make sure both paths share **one** `OpenTelemetryLayer` (otherwise the engine's `Span::current().set_parent()` reads the wrong provider).
- Engine binary side: there is no engine-only binary — engine work is hosted in `apps/server` and uses the API's subscriber (`crates/engine/src/control_consumer.rs:597-640` instruments spans, but installation is owned by the API binary). Confirm via grep: no `nebula_log::init` or `init_api_telemetry` call elsewhere in `apps/server`.
- Provider shutdown discipline: `crates/api/src/telemetry_init.rs` has **no** counterpart to `crates/log/src/telemetry/otel.rs:177-183` (`shutdown_unused_provider`) — a real exporter must add it (~20 LOC) plus a guard returned from `init_api_telemetry` so `main.rs` can hold it.
- ADR-0050 acceptance note: `docs/adr/0050-m3-5-w3c-trace-context-propagation.md:42-56` flags this as gated on M9.2.

**Estimate:** ~120–160 LOC concentrated in `crates/api/src/telemetry_init.rs` (resolver + exporter build + shutdown guard) + a small `apps/server/src/main.rs` change (~5 LOC) to hold the guard until shutdown.

### Goal 2 — OTLP metrics exporter installed

- `crates/metrics/src/` — **no metrics exporter** anywhere. The `MetricsRegistry::snapshot_*` triple (`crates/metrics/src/registry.rs:286-317`) is the documented seam, but no consumer pushes those snapshots to OTLP. ADR-0046 `docs/adr/0046-metrics-telemetry-boundary.md:46-65` keeps the module layout flat — a new `otlp.rs` parallel to `prometheus.rs` is the natural location (~250–350 LOC for the full `SdkMeterProvider` + periodic reader + `Counter`/`Gauge`/`Histogram` translation, plus a `LabelAllowlist`-aware label-name sanitiser).
- `crates/metrics/Cargo.toml:14-21` — must add `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp` workspace deps (the workspace already declares them at `Cargo.toml:152-155`, plus `tokio` feature gates).
- `crates/metrics/src/lib.rs:30-58` — must `mod otlp;` and re-export the new exporter (~5–10 LOC) without breaking ADR-0046 §"Flat module layout".
- `crates/api/src/domain/metrics.rs:14-39` — Prometheus handler is fine; the OTLP push runs as a background task on a periodic reader. Installation point candidate: `crates/api/src/telemetry_init.rs` (alongside the trace exporter) or a new dedicated init in the binary (~30–50 LOC).
- Registry lifecycle: `crates/api/src/state.rs:399-412` (`metrics_registry: Option<Arc<MetricsRegistry>>` — exact lines via grep `metrics_registry`); the push exporter must hold the same `Arc`.
- ADR-0046 §"Public contract is unaffected" `docs/adr/0046-metrics-telemetry-boundary.md:30` means the metric **names** stay; the OTLP exporter translates `nebula_*` snake_case to OTel instrument names + the `nebula_*` units convention.

**Estimate:** ~300–400 LOC in a new `crates/metrics/src/otlp.rs`, ~10 LOC in `lib.rs`, ~30–50 LOC of init wiring, ~10 LOC of `Cargo.toml` deps. Cardinality budget per-counter is already encoded in `naming.rs` (`crates/metrics/src/naming.rs:248-257` for webhook example).

### Goal 3 — Full-stack one-root-span integration test against `task obs:up`

- **[NOT FOUND]** — neither the test nor the compose stack exist.
- Compose file gap — `deploy/docker/docker-compose.observability.yml` referenced by `Taskfile.yml:14,287-289` does not exist. Sibling compose paths under `deploy/docker/` (`Taskfile.yml:12-13`) also missing. The whole `deploy/` directory is absent. A minimal compose with `otelcol-contrib` + Jaeger (~30–60 LOC YAML) plus a `deploy/.env.example` is needed.
- Test gap — no `crates/api/tests/otlp_*.rs` or `crates/engine/tests/otlp_*.rs`. The existing `crates/api/tests/trace_w3c_smoke.rs:1-159` (~160 LOC) is the closest template. A full-stack test mirroring `crates/api/tests/execution_terminate_e2e.rs:1-435` (control-queue + engine seam) plus an OTLP probe against the collector debug endpoint is ~200–300 LOC.
- Shared test scaffolding: `crates/api/tests/common/mod.rs:823-1079` already wires the real `WorkflowEngine` + `ControlConsumer` over the same in-memory storage — reusable for the new test (~0 LOC change, just import).
- The test must enable the same provider/exporter as the binary path; the in-process `SdkTracerProvider` from `telemetry_init.rs:37-38` must be replaced or augmented with the OTLP-enabled provider for the duration of the test.
- Compose-aware tests need a CI gate or `DATABASE_URL`-style env guard so they only run when the collector is up (template at `crates/api/tests/idempotency_e2e.rs` — DATABASE_URL gating, ~15 LOC of `if std::env::var(...).is_err() { return; }`).

**Estimate:** ~60 LOC compose YAML, ~200–300 LOC integration test (one tokio test that POSTs to a real `build_app`, awaits engine drainage, and verifies a trace id via the collector debug exporter or Jaeger API).

---

## Bonus: pre-existing OpenTelemetry deps (workspace)

- Workspace pins: `Cargo.toml:152-155` (`opentelemetry = "0.31.0"`, `opentelemetry-otlp = "0.31.1"` with `grpc-tonic,trace`, `opentelemetry_sdk = "0.31.0"` with `rt-tokio`, `tracing-opentelemetry = "0.32.1"`).
- API crate deps: `crates/api/Cargo.toml:48-50` (opentelemetry + sdk + tracing-opentelemetry; **no** `opentelemetry-otlp`).
- Metrics crate deps: `crates/metrics/Cargo.toml:14-21` (none of the OTel deps).
- Log crate carries the only OTLP dep via the `telemetry` feature (visible at `crates/log/src/telemetry/otel.rs:6` `use opentelemetry_otlp::WithExportConfig;`).

To enable Goal 1 with the smallest dep churn, adding `opentelemetry-otlp = { workspace = true }` to `crates/api/Cargo.toml` is enough (the `trace` feature on the workspace pin already covers span export).

For Goal 2 (metrics exporter), the workspace pin needs the `metrics` feature flag added to `opentelemetry-otlp` (`Cargo.toml:153` — currently only `grpc-tonic, trace`).

---

RECON COMPLETE — 160 citations, 4 not-found markers
