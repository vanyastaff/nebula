# Proposals

## P001: Extract nebula-metrics Crate

**Type:** Non-breaking (additive)

**Motivation:** Centralize export logic; clear ownership; separate from telemetry events.

**Proposal:** Create `nebula-metrics` crate with Prometheus/OTLP exporters. Adapter reads from `nebula-telemetry::MetricsRegistry`. Telemetry keeps in-memory primitives; metrics crate owns export.

**Expected benefits:** Clear separation; metrics crate can depend on telemetry; export optional.

**Costs:** New crate; adapter layer; maintenance.

**Risks:** Duplication if telemetry Phase 3 also adds export; need clear boundary.

**Compatibility impact:** Additive; telemetry unchanged.

**Status:** Draft

---

## P002: Extend Telemetry with Export (No New Crate)

**Type:** Non-breaking

**Motivation:** Simpler; fewer crates; telemetry already has metrics.

**Proposal:** Add `prometheus` and `otlp` features to `nebula-telemetry`. Implement `PrometheusExporter` and `OtlpExporter` in telemetry crate. API exposes `/metrics` builder.

**Expected benefits:** Single crate; no adapter; faster to implement.

**Costs:** Telemetry crate grows; mixed concerns (events + metrics + export).

**Risks:** Telemetry becomes heavy; optional features increase test matrix.

**Compatibility impact:** Additive features.

**Status:** Draft

---

## P003: Standard Metric Naming Convention

**Type:** Non-breaking

**Motivation:** Consistent naming across crates; Grafana dashboard compatibility; avoid collisions.

**Proposal:** Adopt `nebula_<domain>_<metric>_<unit>` pattern. Examples: `nebula_workflow_executions_total`, `nebula_action_duration_seconds`, `nebula_credential_operations_total`.

**Expected benefits:** Predictable; tooling-friendly; documentation.

**Costs:** May require renaming existing metrics (engine, runtime use `actions_executed_total` etc.).

**Risks:** Breaking if we rename; need migration path.

**Compatibility impact:** Additive for new metrics; existing may stay for compat.

**Status:** Draft

---

## P004: Unified Metrics Registry Trait

**Type:** Breaking (if replaces)

**Motivation:** Abstract over telemetry, log, domain crates; single export point.

**Proposal:** Define `MetricsRegistry` trait with `counter`, `gauge`, `histogram`. Telemetry, log, domain crates implement or adapt. Export layer iterates over trait.

**Expected benefits:** Single scrape; unified view.

**Costs:** Trait design; adapter implementations; possible performance overhead.

**Risks:** Complex; may not fit all domain metrics.

**Compatibility impact:** New trait; adapters for existing.

**Status:** Draft
