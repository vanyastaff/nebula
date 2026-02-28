# API

## Public Surface (Planned)

### Stable APIs (Target)

- `MetricsRegistry` — unified registry or adapter
- `PrometheusExporter` — `/metrics` HTTP endpoint
- `OtlpExporter` — OTLP push (optional)
- `MetricNaming` — standard metric names

### Current APIs (No Crate)

Metrics APIs are implemented in other crates:

| Crate | API |
|-------|-----|
| `nebula-telemetry` | `MetricsRegistry`, `Counter`, `Gauge`, `Histogram`, `NoopMetricsRegistry` |
| `nebula-log` | `counter!`, `gauge!`, `histogram!`, `timed_block`, `timed_block_async` (feature `observability`) |
| `nebula-memory` | `MemoryMetricsExtension`, `Counter`, `Gauge`, `Histogram` (domain-specific) |

## Usage Patterns (Current)

### Engine/Runtime (Telemetry)

```rust
let metrics = Arc::new(MetricsRegistry::new());
metrics.counter("actions_executed_total").inc();
metrics.histogram("action_duration_seconds").observe(duration.as_secs_f64());
```

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
