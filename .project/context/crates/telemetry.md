# nebula-telemetry
In-memory metrics primitives for the Nebula workflow engine.

## Module Structure
- `metrics` — Counter, Gauge, Histogram, MetricsRegistry (in-memory, lock-free via atomics)
- `labels` — LabelInterner, LabelSet, MetricKey (lasso-backed string interning)
- `error` — TelemetryError (Io)

## Invariants
- Pure metrics crate — no events, no tracing, no service traits.
- `MetricsRegistry` is cheaply cloneable (Arc-backed internally).
- Metric names must use `nebula_` prefix — enforced by convention, not by code.

## Key Decisions
- `LabelInterner` / `LabelSet` = `lasso`-backed string interning for metric label keys/values (zero-copy dimensions).
- In-memory primitives (`Counter`, `Gauge`, `Histogram`, `MetricsRegistry`) live here; nebula-metrics adds naming + export.
- Stripped to pure metrics (2026-04-04): removed ExecutionEvent, EventBus wrapper, TelemetryService trait, TraceContext, prelude. These were premature — engine/runtime are blocked and will redefine events when stabilized.
- Dependencies reduced from 11 to 5: nebula-error, thiserror, tracing, dashmap, lasso.

## Traps
- `tracing` dep is kept because `Histogram::with_buckets` logs a debug message on creation.
- Engine and runtime no longer have EventBus fields — they only record metrics. Execution events will be redesigned when engine stabilizes.

## Relations
- Used by nebula-metrics (naming conventions + adapter). Used by nebula-runtime and nebula-engine for recording metrics.

<!-- reviewed: 2026-04-04 — stripped to pure metrics primitives crate -->

<!-- reviewed: 2026-04-11 — Workspace-wide nightly rustfmt pass applied (group_imports = "StdExternalCrate", imports_granularity = "Crate", wrap_comments, format_code_in_doc_comments). Touches every Rust file in the crate; purely formatting, zero behavior change. -->
