# nebula-telemetry

In-memory metric primitives — counters, gauges, histograms, and a lock-free registry with label interning.

**Layer:** Cross-cutting
**Canon:** §3.10 (cross-cutting; primitives only — naming/export live in `nebula-metrics`)

## Status

**Overall:** `implemented` — the primitive layer used by every other crate that records metrics.

**Works today:**

- `MetricsRegistry` — concurrent registry for counters, gauges, histograms
- `Counter`, `Gauge`, `Histogram` — lock-free atomics-backed metric types
- `LabelInterner` / `LabelSet` — `lasso`-backed string interning for label keys/values, enabling zero-copy metric dimensions
- `MetricKey` — typed metric identity
- `TelemetryError` / `TelemetryResult` — typed error for the telemetry subsystem
- 2 unit test markers, **0 integration tests**

**Known gaps / deferred:**

- **Naming convention (`nebula_*` prefix)** is defined in doc comments but **enforcement lives in `nebula-metrics`**, not here. This crate is the primitive layer; anything opinionated about names, adapters, or export format belongs one layer up.
- **Export formats** (Prometheus text, OTLP, …) — not in scope. See `nebula-metrics`.
- **No integration tests** — coverage relies on `nebula-metrics` integ tests + unit tests here.
- **Histogram bucketing configuration** — review whether current bucketing is configurable per-metric or registry-wide.

## Architecture notes

- **Minimal deps.** Only `nebula-error`. Correct for a cross-cutting primitive crate.
- **Four modules for 1411 lines** — `error`, `labels`, `metrics`, `lib`. Clean.
- **No dead code or compat shims.**
- **Boundary with `nebula-metrics` is worth watching.** Canon §3.10 says: *"`nebula-metrics` sits on top of `nebula-telemetry`"*. If naming helpers or adapters start appearing here, that's a layering violation — push them up.
- **No SRP/DRY violations observed.**

## What this crate provides

| Type | Role |
| --- | --- |
| `MetricsRegistry` | Concurrent registry. |
| `Counter`, `Gauge`, `Histogram` | Atomics-backed primitives. |
| `LabelInterner`, `LabelSet`, `MetricKey` | String interning for labels. |
| `TelemetryError`, `TelemetryResult` | Typed error. |

## Where the contract lives

- Source: `src/lib.rs`, `src/metrics.rs`, `src/labels.rs`
- Canon: `docs/PRODUCT_CANON.md` §3.10

## See also

- `nebula-metrics` — naming + adapters + export on top of these primitives
- `nebula-eventbus` — independent pub/sub layer; orthogonal to metrics
