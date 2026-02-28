# Decisions

## D001: Metrics in Telemetry (Current)

**Status:** Adopt

**Context:** Engine and runtime need lightweight metrics without external dependencies for MVP.

**Decision:** In-memory Counter, Gauge, Histogram, MetricsRegistry live in `nebula-telemetry`. No standalone metrics crate for MVP.

**Alternatives considered:** Separate nebula-metrics crate from start; Prometheus as direct dep.

**Trade-offs:** Telemetry crate owns both events and metrics; export deferred to Phase 3.

**Consequences:** Metrics crate is planned, not current; telemetry ROADMAP includes export.

**Migration impact:** None.

**Validation plan:** Engine/runtime integration tests; telemetry tests.

---

## D002: Standard Metrics Crate in Log

**Status:** Adopt

**Context:** Observability stack needs standard `metrics` crate for ecosystem compatibility.

**Decision:** `nebula-log` exposes `metrics` crate (feature `observability`); `timed_block`, `timed_block_async`; Prometheus exporter optional.

**Alternatives considered:** Custom metrics only; no log metrics.

**Trade-offs:** Two metric implementations (telemetry in-memory, log standard); acceptable until unification.

**Consequences:** Future metrics crate may bridge or replace.

**Migration impact:** None.

**Validation plan:** Log observability tests.

---

## D003: Domain-Specific Metrics in Each Crate

**Status:** Adopt

**Context:** Memory, credential, resource, resilience have domain-specific metric needs.

**Decision:** Each crate defines its own metric types and recording; no shared registry across domains.

**Alternatives considered:** Centralized registry; single metrics crate for all.

**Trade-offs:** Fragmentation; no unified export. Acceptable for current phase.

**Consequences:** Future metrics crate will need adapters for domain metrics.

**Migration impact:** Adapters in Phase 2/3.

**Validation plan:** Per-crate metrics tests.

---

## D004: Prometheus as Export Target

**Status:** Adopt (Planned)

**Context:** Industry standard; Grafana integration; pull model.

**Decision:** Prometheus-compatible export as primary target; `/metrics` endpoint.

**Alternatives considered:** Push-only; custom format.

**Trade-offs:** Pull model requires scrape endpoint; standard format.

**Consequences:** API or metrics crate must expose HTTP endpoint.

**Migration impact:** New feature.

**Validation plan:** Prometheus scrape test; format validation.

---

## D005: OTLP as Optional Export

**Status:** Defer

**Context:** Cloud-native; vendor-neutral; push model.

**Decision:** OTLP export as optional feature in Phase 3; not required for MVP.

**Alternatives considered:** OTLP only; no OTLP.

**Trade-offs:** Additional dependency; optional keeps binary small.

**Consequences:** Feature-gated; documented.

**Migration impact:** None until implemented.

**Validation plan:** OTLP push integration test.
