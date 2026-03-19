# nebula-metrics
Standard `nebula_*` metric naming via typed `MetricName` structs, Prometheus text export, and label safety guards.

## Invariants
- In-memory metric primitives (`Counter`, `Gauge`, `Histogram`, `MetricsRegistry`) live in nebula-telemetry. This crate adds naming, export, and label filtering.
- All 27 well-known metrics are defined as `MetricName` constants in `naming.rs` with name, kind, and help text.
- `ALL_METRICS` array contains every well-known metric — used by the Prometheus exporter for HELP text lookup.

## Key Decisions
- Metric name constants use short names without `NEBULA_` prefix (e.g. `WORKFLOW_EXECUTIONS_STARTED`, not `NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL`). The string value retains the full `nebula_*` Prometheus name.
- `MetricName` is `Copy` — pass `.as_str()` when calling registry methods that take `&str`.
- `MetricKind` enum (`Counter`, `Gauge`, `Histogram`) enables type-aware processing.
- `LabelAllowlist` strips high-cardinality label keys before they reach the registry — **always configure this in production** to prevent cardinality explosion.
- `PrometheusExporter` outputs Prometheus text format with `# HELP` / `# TYPE` metadata and per-bucket histogram lines.

## Traps
- `LabelAllowlist::default()` is deny-all — strips all labels. Use `LabelAllowlist::only([...])` to allow specific keys, or `LabelAllowlist::all()` in tests.
- `snapshot()` returns Prometheus text format — call `content_type()` for the correct `Content-Type` header.
- `MetricsRegistry` methods take `&str`, so pass `METRIC.as_str()` not the `MetricName` directly.
- `TelemetryAdapter` was deleted — use `MetricsRegistry` directly with naming constants.

## Relations
- Re-exports `Counter`, `Gauge`, `Histogram`, `MetricsRegistry` from nebula-telemetry. Used by nebula-api for the `/metrics` endpoint.

<!-- reviewed: 2026-03-19 -->
