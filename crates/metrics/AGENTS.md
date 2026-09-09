# nebula-metrics — Agent orientation
> Local guide for `crates/metrics/`. Read [root AGENTS.md](../../AGENTS.md) first;
> this guide adds crate-specific rules. Design and status: [README.md](README.md).

**Purpose:** In-memory metric primitives (counter/gauge/histogram), `nebula_*` naming policy, label-cardinality safety, and Prometheus + OTLP export for the engine.
**Layer:** Cross-cutting — importable at any layer; depends only on `nebula-error`, `nebula-eventbus`, and the OTel SDK.

## Commands

- Snapshot tests use `insta`; inspect Prometheus wire-format changes before accepting snapshots. Run `cargo insta review` from `crates/metrics/` to avoid accepting unrelated crate snapshots.

## Key files

- `src/lib.rs` — flat re-export surface + `mod` grouping (primitives / policy / export / instrumentation / error)
- `src/registry.rs` — `MetricsRegistry`; `snapshot_*` methods are the public seam both exporters read
- `src/{counter,gauge,histogram}.rs` — lock-free atomic-backed metric types
- `src/labels.rs` — `lasso`-backed `LabelInterner` / `LabelSet` / `MetricKey` (zero-copy dimensions)
- `src/naming.rs` — `NEBULA_*` name constants (the policy section; new names go here)
- `src/filter.rs` — `LabelAllowlist` cardinality guard (strips high-cardinality keys)
- `src/prometheus.rs` — text-format export (`snapshot()`, `content_type()`); `src/otlp.rs` — OTLP push exporter (ADR-0046 single OTel-SDK seam)

## Conventions & never-do

- Single observability crate (ADR-0046 absorbed `nebula-telemetry`). Keep the `mod` boundary discipline: a new `NEBULA_*` const or label policy belongs in the **policy** section (`naming.rs`/`filter.rs`), never in a primitive file.
- `src/otlp.rs` is the ONLY place OTel SDK types appear; do not import `opentelemetry*` from primitives/export. README still calls OTLP "planned" — it is now implemented.
- Not a log system (`nebula-log`), not the `/metrics` HTTP host (`nebula-api` serves `snapshot()`), not a tracing/spans system (use `tracing` directly).

## Change checks

| Change | Relevant evidence |
|--------|-------------------|
| Registry, naming, or labels | [integration](tests/integration.rs) and unit tests in [src/registry.rs](src/registry.rs), [src/filter.rs](src/filter.rs). |
| Export format | Unit tests in [src/prometheus.rs](src/prometheus.rs) and [src/otlp.rs](src/otlp.rs); wire snapshots do not establish delivery to a live collector. |

## See also

- `README.md` — full design · ADR-0046 · [docs/OBSERVABILITY.md](../../docs/OBSERVABILITY.md), [docs/PRODUCT_CANON.md](../../docs/PRODUCT_CANON.md) §4.6
