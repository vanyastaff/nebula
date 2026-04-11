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
- Cache metrics (`NEBULA_CACHE_HITS`, `NEBULA_CACHE_MISSES`, `NEBULA_CACHE_EVICTIONS`, `NEBULA_CACHE_SIZE`) are **gauges** (point-in-time snapshots), not counters — no `_total` suffix. The registry bridge uses `gauge().set()`.
- When adding naming constants, ensure the Prometheus type (`counter_help` / `gauge_help` / `histogram_help`) matches the semantic type and the test uses the matching registry method.

## Relations
- Re-exports `Counter`, `Gauge`, `Histogram`, `MetricsRegistry` from nebula-telemetry. Used by nebula-api for the `/metrics` endpoint.

<\!-- reviewed: 2026-03-30 -->

<!-- reviewed: 2026-04-04 -->

<!-- reviewed: 2026-04-11 — Workspace-wide nightly rustfmt pass applied (group_imports = "StdExternalCrate", imports_granularity = "Crate", wrap_comments, format_code_in_doc_comments). Touches every Rust file in the crate; purely formatting, zero behavior change. -->
