# nebula-metrics — Agent orientation
> Agent quick-map for `crates/metrics/`. Full design: `README.md`. Repo-wide rules: root `AGENTS.md`.

**Purpose:** In-memory metric primitives (counter/gauge/histogram), `nebula_*` naming policy, label-cardinality safety, and Prometheus + OTLP export for the engine.
**Layer:** Cross-cutting — importable at any layer; depends only on `nebula-error`, `nebula-eventbus`, and the OTel SDK.

## Commands
- `cargo check -p nebula-metrics`
- `cargo nextest run -p nebula-metrics`  ·  doctests: `cargo test -p nebula-metrics --doc`
- Snapshot tests use `insta`; review with `cargo insta review` after changing Prometheus output.

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
- `#![forbid(unsafe_code)]` and `#![warn(missing_docs)]` are crate-wide — every `pub` item needs a doc comment.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · ADR-0046 · `docs/OBSERVABILITY.md`, `docs/PRODUCT_CANON.md` §4.6
