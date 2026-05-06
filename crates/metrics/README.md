---
name: nebula-metrics
role: Observability — primitives, naming, cardinality safety, Prometheus export
status: stable
last-reviewed: 2026-05-06
canon-invariants: []
related: [nebula-eventbus, nebula-api]
---

# nebula-metrics

## Purpose

Single observability crate for the Nebula workflow engine. Provides the in-memory
primitives that record observations (counter, gauge, histogram), the standard
`nebula_*` naming constants that operators see in dashboards, the cardinality
guard that prevents label explosion, and the Prometheus text-format export used
by the `/metrics` HTTP endpoint.

Per ADR-0046 the formerly separate `nebula-telemetry` primitives layer was
absorbed into this crate. The cross-crate boundary that lived under canon
`[L1-§3.10]` was structurally unenforced (no `cargo deny` rule) and caused
daily friction (~500 LOC of bridge code, dual-import call-sites, the
`TelemetryAdapter` mediation type). Intra-crate module discipline (`mod`
boundaries + `pub`/`pub(crate)`) preserves the same separation of concerns
without the Cargo-level overhead.

## Role

**Observability primitives + policy + export** — single cross-cutting crate.
The `/metrics` HTTP endpoint that serves `snapshot()` output lives in
`nebula-api`. Consumers import this crate as the single boundary; types are
exposed flat (`nebula_metrics::Counter`, not `nebula_metrics::primitives::Counter`).

## Public API

- `MetricsRegistry` — concurrent registry for counters, gauges, and histograms.
- `Counter`, `Gauge`, `Histogram`, `HistogramSnapshot` — lock-free metric types
  backed by atomics.
- `LabelInterner`, `LabelSet`, `MetricKey` — `lasso`-backed string interning for
  label keys and values; enables zero-copy metric dimensions.
- `MetricsAdapter` — adapter that records using standard `nebula_*` name constants.
- `PrometheusExporter`, `snapshot() -> String` — Prometheus text-format export
  with `# HELP`, `# TYPE` metadata and per-bucket histogram output.
- `content_type() -> &'static str` — standard Prometheus `Content-Type` for HTTP
  exporters.
- `LabelAllowlist` — allowlist that strips high-cardinality label keys before
  they reach the registry.
- `naming` module — `nebula_*` metric name constants (e.g.
  `NEBULA_EXECUTIONS_STARTED_TOTAL`, `NEBULA_ACTION_DURATION_SECONDS`).
- `MetricsError`, `MetricsResult` — typed error and result alias.
- `prelude` — convenience re-exports.

## Module discipline (replaces the former cross-crate split)

The crate's `lib.rs` groups `mod` declarations by concern:

- **primitives** — `labels.rs`, `registry.rs` (counter/gauge/histogram + registry).
- **policy** — `naming.rs`, `filter.rs`, `adapter.rs`.
- **export** — `prometheus.rs`.
- **error** — `error.rs`.

A new `NEBULA_*` constant or a new label policy belongs in the policy section,
not in primitives. Adding `pub const NEBULA_FOO: &str = "..."` to `registry.rs`
is the moral equivalent of the canon `[L1-§3.10]` violation that the prior
cross-crate split tried to prevent — the constraint now lives at the file/`mod`
level inside one crate.

## Non-goals

- Not a log system — see `nebula-log`.
- Not an OTLP exporter — Prometheus text is the only current export format (OTLP
  is `planned`).
- Not the HTTP `/metrics` endpoint host — that lives in `nebula-api`.
- Not a tracing/spans system — use `tracing` directly per ARCHITECTURE.md.

## Maturity

- API stability: `stable` — `MetricsRegistry`, primitives, `MetricsAdapter`,
  `PrometheusExporter`, `snapshot()`, `LabelAllowlist`, and `naming` constants
  are in active use.
- OTLP export is `planned`; Prometheus text is the implemented export format.
- Naming enforcement is currently manual (no lint). Drift between call sites and
  constants is possible.

## Related

- ADR: `docs/adr/0046-metrics-telemetry-boundary.md` — the merge decision.
- Audit: `docs/audits/metrics-telemetry-merge.md` — evidence base for the merge.
- Canon: `docs/PRODUCT_CANON.md` §4.6 (Observability), `docs/OBSERVABILITY.md`.
- Siblings: `nebula-eventbus` (independent pub/sub, observed via the four
  `nebula_eventbus_*` gauges), `nebula-api` (hosts `/metrics` HTTP endpoint).
