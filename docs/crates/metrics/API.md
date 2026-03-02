# API

## Public Surface (Planned)

### Stable APIs (Current)

- `naming` — constants: `NEBULA_WORKFLOW_*`, `NEBULA_ACTION_*`
- `TelemetryAdapter` — adapter over `MetricsRegistry` with typed accessors
- `PrometheusExporter` — holds `Arc<MetricsRegistry>`, `.snapshot()` returns Prometheus text
- `snapshot(registry)` — render registry to Prometheus exposition format
- `content_type()` — `text/plain; version=0.0.4; charset=utf-8`

### Planned

- `OtlpExporter` — OTLP push (optional feature)

### Current APIs (No Crate)

Metrics APIs are implemented in other crates:

| Crate | API |
|-------|-----|
| `nebula-telemetry` | `MetricsRegistry`, `Counter`, `Gauge`, `Histogram`, `NoopMetricsRegistry` |
| `nebula-log` | `counter!`, `gauge!`, `histogram!`, `timed_block`, `timed_block_async` (feature `observability`) |
| `nebula-memory` | `MemoryMetricsExtension`, `Counter`, `Gauge`, `Histogram` (domain-specific) |

## Usage Patterns (Current)

### Engine/Runtime (Standard Names)

Engine and runtime use `nebula_metrics::naming` constants and record under `nebula_*` names:

```rust
use nebula_metrics::naming::{NEBULA_ACTION_EXECUTIONS_TOTAL, NEBULA_ACTION_DURATION_SECONDS};
let metrics = Arc::new(MetricsRegistry::new());
metrics.counter(NEBULA_ACTION_EXECUTIONS_TOTAL).inc();
metrics.histogram(NEBULA_ACTION_DURATION_SECONDS).observe(duration.as_secs_f64());
```

Or use `TelemetryAdapter` for typed accessors: `adapter.action_executions_total().inc()`.

### Log Observability

```rust
// With nebula-log observability feature
use nebula_log::metrics::{timed_block, timed_block_async};
let result = timed_block("operation_name", || expensive_work());
```

## Minimal Example (Target)

```rust
// Future: nebula-metrics
let registry = MetricsRegistry::new();
registry.counter("nebula_workflow_executions_total").inc();
let exporter = PrometheusExporter::new(registry);
// Serve /metrics endpoint
```

## Advanced Example (Target)

```rust
// Future: OTLP push, standard naming
let registry = MetricsRegistry::with_naming(MetricNaming::Nebula);
let otlp = OtlpExporter::builder()
    .endpoint("http://collector:4317")
    .build(registry)?;
otlp.start().await?;
```

## Error Semantics

- **Current:** Telemetry emit/record infallible; no Result.
- **Target:** Export may fail; retries/configurable; never block recording.

## Compatibility Rules

- **Major version bump:** Breaking changes to metric names, export format.
- **Deprecation policy:** Metric name changes deprecated 2 minor releases.
