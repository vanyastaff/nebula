# Metrics & Telemetry — Development Roadmap

> **Last updated:** 2026-03-15  
> Covers `nebula-telemetry` and `nebula-metrics` crates.

---

## Current State (Phase 1 — Complete)

| Component | Status | Notes |
|---|---|---|
| `Counter` / `Gauge` / `Histogram` | ✅ Done | Atomic, Prometheus-style buckets, constant memory |
| `MetricsRegistry` | ✅ Done | `lasso`-interned keys, `DashMap` sharded maps, labeled variants |
| `LabelInterner` / `LabelSet` | ✅ Done | `lasso::ThreadedRodeo` backend, order-invariant hash |
| `TelemetryError` | ✅ Done | Typed errors with `thiserror` |
| `TelemetryService` trait | ✅ Done | `NoopTelemetry`, `ProductionTelemetry` |
| `BufferedRecorder` | ✅ Done | Async MPSC + pluggable `RecordSink` |
| `PrometheusExporter` | ✅ Done | Dynamic snapshot with full label rendering |
| `TelemetryAdapter` | ✅ Done | Canonical `nebula_*` names + labeled variants |
| W3C `TraceContext` | ✅ Done | `traceparent` parse/format |

---

## Phase 2 — `metrics` Facade Bridge

> **Goal:** Metrics emitted by third-party crates (Tokio, Axum, Tower, sqlx) flow
> into `MetricsRegistry` automatically, visible in `/metrics`.

### What

The [`metrics`](https://crates.io/crates/metrics) crate is the de-facto standard
facade for Rust instrumentation — like `log` for logging.  Many dependencies
already call `metrics::counter!(...)` / `metrics::histogram!(...)`.  Without a
registered backend these calls are silently dropped.

Implement `metrics::Recorder` for `MetricsRegistry` and install it as the
global recorder on startup.

### Design

```rust
// crates/metrics/src/recorder.rs
use metrics::{Counter, Gauge, Histogram, Key, Recorder, Unit};
use nebula_telemetry::metrics::MetricsRegistry;

pub struct NebulaRecorder {
    registry: Arc<MetricsRegistry>,
}

impl Recorder for NebulaRecorder {
    fn register_counter(&self, key: &Key, _unit: Option<Unit>, _desc: Option<&str>) -> Counter {
        // Convert metrics::Key (name + labels) → counter_labeled(...)
    }
    // ... gauge, histogram
}
```

Registration at startup:
```rust
let recorder = NebulaRecorder::new(Arc::clone(&registry));
metrics::set_global_recorder(recorder)?;
```

### Key observations

- `metrics::Key` carries `labels: Vec<Label>` — map to `LabelSet` via `LabelInterner`.
- Histogram units differ (`metrics::Unit::Seconds`) — store unit metadata for better HELP text.
- Thread-safety: `NebulaRecorder` must be `Send + Sync`; `MetricsRegistry` already is.
- **One-way dependency**: `nebula-metrics` → `metrics` (facade crate).  `nebula-telemetry`
  should NOT depend on `metrics` directly.

### Dependencies to add

```toml
# crates/metrics/Cargo.toml
metrics = { workspace = true }
```

(Already declared in `[workspace.dependencies]`.)

---

## Phase 3 — OTLP Export

> **Goal:** Export traces and metrics to any OTLP-compatible backend (Jaeger,
> Tempo, Prometheus remote write, etc.) without changing application code.

### What

Wire the pre-declared `opentelemetry`, `opentelemetry-otlp`, `opentelemetry_sdk`,
and `tracing-opentelemetry` workspace dependencies into a concrete export path.

### Two concerns

#### 3a — Trace export (spans → OTLP)

Connect `tracing` spans to OpenTelemetry via `tracing-opentelemetry`:

```rust
// crates/telemetry/src/otlp.rs  (new file)
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::layer::SubscriberExt;

pub fn install_otlp_trace_layer(endpoint: &str) -> anyhow::Result<SdkTracerProvider> {
    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint)
        .build()?;
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .build();
    Ok(provider)
}
```

`TraceContext` (W3C `traceparent`) is already implemented — pass it via gRPC
metadata or HTTP headers on outbound calls.

#### 3b — Metrics export (MetricsRegistry → Prometheus remote write / OTLP)

Option A (preferred): Expose `PrometheusExporter` as an Axum route in
`nebula-api` — Prometheus scrapes `/metrics`.  Already partially implemented.

Option B: Use `opentelemetry-otlp` `MetricExporter` periodically flushing
`MetricsRegistry` snapshots.  More complex, needed only when scraping is
unavailable.

### Configuration

```toml
# deploy/docker/compose.yml example
NEBULA_OTLP_ENDPOINT=http://otel-collector:4317
NEBULA_METRICS_SCRAPE_INTERVAL=15  # seconds
```

### Dependencies to add

```toml
# crates/telemetry/Cargo.toml (only when otlp feature enabled)
opentelemetry = { workspace = true, optional = true }
opentelemetry-otlp = { workspace = true, optional = true }
opentelemetry_sdk = { workspace = true, optional = true }
tracing-opentelemetry = { workspace = true, optional = true }
```

---

## Phase 4 — Evaluate `measured`

> **Goal:** Assess whether `measured` by Conrad Ludgate is a better foundation
> than the current custom primitives for compile-time-typed, zero-cost metrics.

### What is `measured`?

[`measured`](https://crates.io/crates/measured) provides:
- **Compile-time metric descriptors** via `#[derive(MetricGroup)]`
- **Typed labels** via `#[derive(LabelGroup)]` — no string keys at runtime
- **Lock-free per-series storage** — no global mutex or lock per observation
- **Prometheus text export** built in

### Example

```rust
use measured::{CounterVec, HistogramVec, MetricGroup, LabelGroup};

#[derive(MetricGroup)]
struct ActionMetrics {
    executions_total: CounterVec<ActionLabels>,
    duration_seconds: HistogramVec<ActionLabels>,
}

#[derive(LabelGroup)]
struct ActionLabels {
    action_type: ActionType,
    status: Status,
}

#[derive(LabelValue, Clone, Copy)]
enum ActionType { HttpRequest, MathAdd }
```

### Pros

- Zero runtime string allocations for labels (enum-based)
- Compile-time exhaustiveness on label variants
- Prometheus export is part of the crate

### Cons

- Less mature ecosystem (0.1.x as of early 2026)
- Dynamic/unknown metric names (from plugins) still need the current `HashMap`-based approach
- Breaking change to replace existing `Counter/Gauge/Histogram` API

### Decision criteria

- If `measured` reaches 0.5+ and supports dynamic metric names, it can replace
  `nebula-telemetry`'s primitives behind a trait boundary.
- Until then: use current custom primitives for shared/dynamic paths, and evaluate
  a `nebula-metrics-typed` crate that wraps `measured` for Nebula-internal
  compile-time metrics (engine, runtime, resource).

### When to revisit

Track `measured` releases in `Cargo.lock`.  Re-evaluate when:
1. `measured` supports `dyn LabelGroup` or dynamic label sets, OR
2. Nebula-internal metrics fully enumerate all label variants.

---

## Phase 5 — Metrics Bridge to `metrics-exporter-prometheus` (Optional)

> Only needed if `PrometheusExporter` proves insufficient.

The workspace already declares `metrics-exporter-prometheus = "0.18"`.  If we
implement the Phase 2 `metrics::Recorder` bridge, the exporter can optionally
use `metrics-exporter-prometheus`'s built-in HTTP server instead of the custom
one — saving maintenance overhead.

---

## Naming Convention Reference

All internal metrics use the `nebula_` prefix. Full table:

| Metric | Type | Labels |
|---|---|---|
| `nebula_workflow_executions_started_total` | counter | — (Phase 2: `workflow_id`) |
| `nebula_workflow_executions_completed_total` | counter | — |
| `nebula_workflow_executions_failed_total` | counter | `error_kind` |
| `nebula_workflow_execution_duration_seconds` | histogram | — |
| `nebula_action_executions_total` | counter | `action_type` |
| `nebula_action_failures_total` | counter | `action_type`, `error_kind` |
| `nebula_action_duration_seconds` | histogram | `action_type` |
| `nebula_resource_create_total` | counter | `resource_key` |
| `nebula_resource_acquire_total` | counter | `resource_key` |
| `nebula_resource_release_total` | counter | `resource_key` |
| `nebula_resource_cleanup_total` | counter | `resource_key` |
| `nebula_resource_error_total` | counter | `resource_key`, `error_kind` |
| `nebula_resource_pool_exhausted_total` | counter | `resource_key` |
| `nebula_resource_pool_waiters` | gauge | `resource_key` |
| `nebula_resource_acquire_wait_duration_seconds` | histogram | `resource_key` |
| `nebula_resource_usage_duration_seconds` | histogram | `resource_key` |
| `nebula_resource_health_state` | gauge | `resource_key` |
| `nebula_resource_quarantine_total` | counter | `resource_key` |
| `nebula_resource_quarantine_released_total` | counter | `resource_key` |
| `nebula_resource_config_reloaded_total` | counter | `resource_key` |
| `nebula_resource_credential_rotated_total` | counter | `resource_key` |
| `nebula_eventbus_sent` | gauge | — |
| `nebula_eventbus_dropped` | gauge | — |
| `nebula_eventbus_subscribers` | gauge | — |
| `nebula_eventbus_drop_ratio_ppm` | gauge | — |

Labels in the "Labels" column marked with `(Phase 2: ...)` will be added when
the labeled accessors are wired into the respective crates.
