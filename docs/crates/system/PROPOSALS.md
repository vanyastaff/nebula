# Proposals

## P001: Configurable Pressure Thresholds

**Type:** Non-breaking

**Motivation:** Different workloads have different tolerance; fixed 50/70/85% may not fit.

**Proposal:** Add `PressureConfig` with configurable thresholds; default to current values.

**Expected benefits:** Flexibility; better fit for edge/server vs desktop.

**Costs:** More API surface; validation of threshold ranges.

**Risks:** Breaking if defaults change; confusion over units.

**Compatibility impact:** Additive; defaults preserve current behavior.

**Status:** Draft

---

## P002: Async Wrappers

**Type:** Non-breaking

**Motivation:** Some consumers prefer async; blocking in async context is undesirable.

**Proposal:** Add `async` feature with `tokio::task::spawn_blocking` wrappers for heavy ops (e.g., `process::list()`).

**Expected benefits:** Better integration with async runtimes.

**Costs:** tokio dependency; additional API surface.

**Risks:** Spawn overhead; thread pool saturation.

**Compatibility impact:** Additive; sync API unchanged.

**Status:** Defer

---

## P003: OpenTelemetry Metrics

**Type:** Non-breaking

**Motivation:** Observability pipelines expect OTLP/Prometheus; manual instrumentation is tedious.

**Proposal:** `metrics` feature emits CPU, memory, disk gauges via `metrics` or `opentelemetry` crate.

**Expected benefits:** Drop-in observability; consistent with nebula-telemetry.

**Costs:** Optional dependency; refresh strategy for gauges.

**Risks:** Cardinality; performance of metric export.

**Compatibility impact:** Additive.

**Status:** Draft
