# nebula-metrics Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Nebula needs production-grade metrics: Prometheus scrape or OTLP push, standard naming (`nebula_*`), and aggregation from system, business, and domain crates (memory, credential, resource, resilience). A dedicated metrics crate (or telemetry Phase 3) provides export backends and naming convention so that operators get consistent dashboards.

**nebula-metrics is the planned unified metrics collection and export layer.**

It answers: *How do metrics from telemetry and domain crates get exported to Prometheus/OTLP, and what naming and cardinality rules apply?*

```
Domain crates (memory, resource, credential, resilience) record metrics
    ↓
nebula-telemetry in-memory primitives or metrics crate registry
    ↓
nebula-metrics: Prometheus scrape endpoint / OTLP push
    ↓
Standard naming (nebula_*); bounded cardinality
```

Contract: stable metric names; Prometheus-compatible export; bounded histograms. Crate is planned or folded into telemetry Phase 3.

---

## User Stories

### Story 1 — Operator Scrapes Prometheus (P1)

Operator configures Prometheus to scrape Nebula's metrics endpoint. Standard metrics (execution count, node duration, resource pool size) appear with nebula_* prefix. No high-cardinality explosion.

**Acceptance**: GET /metrics (or configured path) returns Prometheus text format; names stable; cardinality documented.

### Story 2 — OTLP Push for Cloud Observability (P2)

Nebula pushes metrics to OTLP collector (e.g. for cloud backends). Same metric names and labels; export is configurable.

**Acceptance**: OTLP exporter optional; same naming as Prometheus; no duplicate or conflicting names.

### Story 3 — Domain Crates Use Unified Registry (P2)

Memory, resource, credential, resilience register metrics with shared naming and label rules. Metrics crate (or telemetry) provides registry and export.

**Acceptance**: Naming convention documented; domain crates do not invent ad-hoc names; integration with telemetry in-memory primitives.

---

## Core Principles

### I. Standard Naming and Cardinality

**Metric names use nebula_* prefix; labels are bounded so that cardinality does not explode.**

**Rationale**: Dashboards and alerts depend on names; unbounded labels cause memory and query issues.

### II. Export Is Pluggable

**Prometheus and OTLP are backends; core or telemetry can work without export (in-memory only).**

**Rationale**: Tests and minimal deployments do not need export.

### III. No Domain Logic in Metrics Crate

**Metrics crate owns export and naming. It does not implement workflow, credential, or resource logic.**

**Rationale**: Single responsibility; domain crates record, metrics crate exports.

---

## Production Vision

Prometheus scrape and OTLP push; nebula_* naming; bounded histograms; integration with telemetry and domain crates. From archives: unified export; compatibility with telemetry. Gaps: standalone crate or telemetry Phase 3; bounded histogram in telemetry; formal naming registry.

### Key gaps

| Gap | Priority |
|-----|----------|
| Standalone nebula-metrics or telemetry Phase 3 export | High |
| Bounded histogram (telemetry) | High |
| Formal metric naming registry | Medium |
| Domain crate integration (memory, resource, credential) | Medium |

---

## Non-Negotiables

1. **Stable metric names** — nebula_* prefix; no breaking name in minor.
2. **Bounded cardinality** — labels and histograms documented and limited.
3. **Export pluggable** — Prometheus/OTLP optional; no mandatory external deps for core.
4. **Breaking metric contract = major + MIGRATION.md**.

---

## Governance

- **MINOR**: Additive metrics and labels.
- **MAJOR**: Naming or export format break; MIGRATION.md required.
