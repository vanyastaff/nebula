# nebula-metrics
Standard `nebula_*` metric naming constants, TelemetryAdapter, Prometheus text export, and label safety guards.

## Invariants
- In-memory metric primitives (`Counter`, `Gauge`, `Histogram`, `MetricsRegistry`) live in nebula-telemetry. This crate adds naming, a thin adapter, and export.

## Key Decisions
- All metric name constants follow the `NEBULA_*` naming convention (e.g. `NEBULA_WORKFLOW_EXECUTIONS_STARTED_TOTAL`).
- `TelemetryAdapter` wraps `MetricsRegistry` and records using the standard names — use this instead of raw registry calls.
- `LabelAllowlist` strips high-cardinality label keys before they reach the registry — **always configure this in production** to prevent cardinality explosion.
- `PrometheusExporter` outputs Prometheus text format with `# HELP` / `# TYPE` metadata and per-bucket histogram lines.

## Traps
- Forgetting `LabelAllowlist` means unbounded label cardinality (e.g., per-user metric dimensions) → OOM in production.
- `snapshot()` function returns Prometheus text format — call `content_type()` for the correct `Content-Type` header.

## Relations
- Re-exports `Counter`, `Gauge`, `Histogram`, `MetricsRegistry` from nebula-telemetry. Used by nebula-api for the `/metrics` endpoint.
