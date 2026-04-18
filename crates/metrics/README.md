---
name: nebula-metrics
role: Metric Export and Label-Safety (Prometheus-style naming, adapter, cardinality guard)
status: stable
last-reviewed: 2026-04-17
canon-invariants: []
related: [nebula-telemetry, nebula-eventbus, nebula-api]
---

# nebula-metrics

## Purpose

Raw metric primitives from `nebula-telemetry` need three additional concerns before reaching an
operator: a consistent naming convention (`nebula_*` prefix to avoid collisions), a cardinality
guard to prevent label explosion, and a serialization format for scraping (Prometheus text). Without
a shared layer for these, each crate would apply its own naming and cardinality policy
independently, making the resulting metric catalog inconsistent and hard to operate.
`nebula-metrics` provides all three in one place: a `TelemetryAdapter` that records using standard
`nebula_*` name constants, a `LabelAllowlist` that strips high-cardinality keys before they reach
the registry, and a `PrometheusExporter` for Prometheus text-format scrape output.

## Role

**Metric Export and Label-Safety** — sits on top of `nebula-telemetry` primitives and provides
the naming layer, cardinality guard, and Prometheus text export. Cross-cutting infrastructure.
The `/metrics` HTTP endpoint that serves `snapshot()` output lives in `nebula-api`. Consumers
should import this crate, which re-exports `Counter`, `Gauge`, `Histogram`, and `MetricsRegistry`
from `nebula-telemetry` so only one import is needed.

## Public API

- `TelemetryAdapter` — adapter over `nebula-telemetry::MetricsRegistry` that records using `nebula_*` name constants.
- `PrometheusExporter`, `snapshot() -> String` — Prometheus text-format export with `# HELP`, `# TYPE` metadata and per-bucket histogram output.
- `content_type() -> &'static str` — standard Prometheus `Content-Type` for HTTP exporters.
- `LabelAllowlist` — allowlist that strips high-cardinality label keys before they reach the registry.
- `naming` module — `nebula_*` metric name constants (e.g. `NEBULA_EXECUTIONS_STARTED_TOTAL`, `NEBULA_ACTION_DURATION_SECONDS`).
- `prelude` — convenience re-exports.
- `Counter`, `Gauge`, `Histogram`, `MetricsRegistry` — re-exported from `nebula-telemetry`.

## Contract

- **[L1-§4.6]** Observability is a first-class contract. The `nebula_*` naming constants in the `naming` module are the normative metric names for the workspace; ad-hoc metric names that bypass these constants risk collision and inconsistent scrape output.
- Cardinality safety: `LabelAllowlist` is the designated guard — callers must route labels through it before recording. Not automated by a lint yet (known gap — naming enforcement is manual).

## Non-goals

- Not an in-memory primitive — `Counter`, `Gauge`, `Histogram` primitives live in `nebula-telemetry`.
- Not a log system — see `nebula-log`.
- Not an OTLP exporter — Prometheus text is the only current export format (OTLP is `planned`).
- Not the HTTP `/metrics` endpoint host — that lives in `nebula-api`.

## Maturity

See `docs/MATURITY.md` row for `nebula-metrics`.

- API stability: `stable` — `TelemetryAdapter`, `PrometheusExporter`, `snapshot()`, `LabelAllowlist`, and `naming` constants are in active use.
- OTLP export is `planned`; Prometheus text is the implemented export format.
- Naming enforcement is currently manual (no lint). Drift between call sites and constants is possible.

## Related

- Canon: `docs/PRODUCT_CANON.md` §4.6 (Observability), §3.10 (cross-cutting), `docs/OBSERVABILITY.md`.
- Siblings: `nebula-telemetry` (primitive layer below), `nebula-eventbus` (independent pub/sub, used by this crate), `nebula-api` (hosts `/metrics` HTTP endpoint).
