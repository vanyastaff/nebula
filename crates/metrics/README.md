# nebula-metrics

Unified metric naming (`nebula_*`), a telemetry adapter, Prometheus text export, and label-safety guards — sitting on top of `nebula-telemetry` primitives.

**Layer:** Cross-cutting
**Canon:** §3.10 (cross-cutting; **sits on top of `nebula-telemetry`** — not a parallel primitive stack)

## Status

**Overall:** `implemented` — the `nebula_*` naming convention and Prometheus export are the default metrics surface for the workspace.

**Works today:**

- `naming` module — standard `nebula_*` metric name constants (e.g. `nebula_executions_total`, `nebula_action_duration_seconds`)
- `TelemetryAdapter` — adapter over `nebula-telemetry::MetricsRegistry` that records using those names
- `PrometheusExporter` + `snapshot()` — Prometheus text-format export with `# HELP`, `# TYPE`, per-bucket histogram output
- `content_type()` — standard Prometheus `Content-Type` for HTTP exporters
- `LabelAllowlist` — allowlist that strips high-cardinality label keys before they reach the registry (prevents cardinality explosion)
- `prelude` — convenience re-exports
- Re-exports `Counter` / `Gauge` / `Histogram` / `MetricsRegistry` from `nebula-telemetry` so consumers only import one crate
- 4 unit test markers, 1 integration test

**Known gaps / deferred:**

- **OTLP export** — not implemented. Prometheus text is the only export format today.
- **Scrape endpoint wiring** — Prometheus `snapshot()` produces text; the actual `/metrics` HTTP endpoint lives in `nebula-api`. If absent or partial there, it's an API-layer gap, not a metrics-layer one.
- **Naming enforcement** — the constants in `naming` exist; **lint/check** that every metric-recording site uses them is not automated. Drift is possible.

## Architecture notes

- **Minimal deps.** `nebula-eventbus` + `nebula-telemetry`. No upward dependencies.
- **Five modules + `export/` subdirectory** for 1620 lines — clean split between `adapter`, `export`, `filter`, `naming`, `prelude`.
- **No dead code or compat shims.**
- **Layering with `nebula-telemetry` is correct.** This crate re-exports telemetry types so consumers import `nebula-metrics` only — but telemetry is the single source of the primitives. If primitive types start being redefined here, that's DRY violation — push them down.
- **`LabelAllowlist` is the right place for cardinality safety.** Not in telemetry (primitives shouldn't know about cardinality policy), not in consumer crates (each would re-implement). Correct SRP.

## What this crate provides

| Type / module | Role |
| --- | --- |
| `TelemetryAdapter` | Adapter over `nebula-telemetry::MetricsRegistry` using `nebula_*` names. |
| `PrometheusExporter`, `snapshot()`, `content_type()` | Prometheus text export. |
| `LabelAllowlist` | Cardinality-safety filter. |
| `naming` module | `nebula_*` constant name table. |
| `prelude` | Convenience re-exports. |
| `Counter`, `Gauge`, `Histogram`, `MetricsRegistry` | Re-exports from `nebula-telemetry`. |

## Where the contract lives

- Source: `src/lib.rs`, `src/naming.rs`, `src/adapter.rs`, `src/filter.rs`, `src/export/`
- Canon: `docs/PRODUCT_CANON.md` §3.10

## See also

- `nebula-telemetry` — primitive layer below this crate
- `nebula-eventbus` — independent pub/sub layer
- `nebula-api` — hosts the `/metrics` HTTP endpoint (if implemented)
