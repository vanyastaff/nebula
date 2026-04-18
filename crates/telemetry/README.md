---
name: nebula-telemetry
role: Metric Primitives (lock-free counters, gauges, histograms, label interning)
status: stable
last-reviewed: 2026-04-17
canon-invariants: []
related: [nebula-metrics, nebula-error]
---

# nebula-telemetry

## Purpose

Every crate that records metrics needs the same primitive building blocks: a thread-safe counter,
a gauge, a histogram, and a way to attach label dimensions without heap allocation on the hot path.
`nebula-telemetry` provides these primitives — and only these primitives. Naming conventions,
export adapters, and Prometheus text generation are deliberately out of scope; they live in
`nebula-metrics` one layer above. This boundary ensures that the low-level metric types stay
minimal, with no accidental coupling to naming policy or export format.

## Role

**Metric Primitives** — the in-memory atomics-backed metric layer below `nebula-metrics`.
Cross-cutting infrastructure (no upward dependencies; only `nebula-error` as an intra-workspace
dependency). `nebula-metrics` sits on top of this crate per canon §3.10; consumers should
generally import `nebula-metrics`, which re-exports these types.

## Public API

- `MetricsRegistry` — concurrent registry for counters, gauges, and histograms.
- `Counter`, `Gauge`, `Histogram` — lock-free metric types backed by atomics.
- `LabelInterner` — `lasso`-backed string interner for label keys and values; enables zero-copy metric dimensions.
- `LabelSet` — a resolved set of interned label key-value pairs.
- `MetricKey` — typed metric identity (name + label set).
- `TelemetryError`, `TelemetryResult` — typed error and result alias for this subsystem.

## Contract

- **[L1-§3.10]** This crate is the primitive layer. Naming conventions (`nebula_*` prefix), adapters, and export formats belong in `nebula-metrics` — not here. If naming helpers appear in this crate, that is a layering violation.
- No additional canon L2 invariants are directly assigned to this crate's seams. Consumers depend on `nebula-metrics` for enforcement of naming and export contracts.

## Non-goals

- Not a naming convention enforcer — see `nebula-metrics::naming` for `nebula_*` constants.
- Not an export layer — Prometheus text and OTLP export live in `nebula-metrics`.
- Not a log system — see `nebula-log`.
- Not an event bus — see `nebula-eventbus`.

## Maturity

See `docs/MATURITY.md` row for `nebula-telemetry`.

- API stability: `stable` — `MetricsRegistry`, `Counter`, `Gauge`, `Histogram`, `LabelInterner` are stable primitives in active use.
- No integration tests in this crate; coverage relies on `nebula-metrics` integration tests and unit tests here.
- Histogram bucketing configuration: review whether current bucketing is configurable per-metric or registry-wide (open question for future pass).

## Related

- Canon: `docs/PRODUCT_CANON.md` §3.10 (cross-cutting primitives).
- Siblings: `nebula-metrics` (naming, adapters, Prometheus export — sits on top of this crate), `nebula-error` (sole intra-workspace dependency).
