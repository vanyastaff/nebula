# Architecture

## Problem Statement

- **Business problem:** Workflow engine and domain crates need metrics for dashboards, alerting, and SLO tracking. Current state is fragmented across telemetry, log, memory, credential, resource, resilience.
- **Technical problem:** No unified metrics export; no Prometheus/OTLP integration; telemetry Histogram unbounded; multiple implementations (telemetry in-memory, log standard metrics, memory domain metrics).

## Current Architecture

### Module Map (Current — No Metrics Crate)

| Location | Responsibility |
|----------|----------------|
| `nebula-telemetry` | Counter, Gauge, Histogram, MetricsRegistry (in-memory); used by engine, runtime |
| `nebula-log` | Standard `metrics` crate (feature `observability`); `timed_block`, `timed_block_async`; Prometheus exporter (optional) |
| `nebula-memory` | Domain metrics (MemoryMetric, Counter, Gauge, Histogram); extension traits |
| `nebula-credential` | StorageMetrics; provider operation metrics |
| `nebula-resource` | Resource lifecycle metrics |
| `nebula-resilience` | Circuit breaker, retry metrics |

### Data/Control Flow

1. **Engine/runtime:** Use `nebula-telemetry::MetricsRegistry`; record `actions_executed_total`, `actions_failed_total`, `action_duration_seconds`.
2. **Log observability:** `metrics::counter!`, `metrics::histogram!` via `nebula-log` when `observability` feature enabled.
3. **Domain crates:** Each defines its own metric types; no shared registry or export.

### Known Bottlenecks

- **No export:** Telemetry metrics never leave process; no Prometheus scrape.
- **Fragmentation:** Multiple metric APIs; no unified naming.
- **Histogram unbounded:** Telemetry Histogram stores all observations in memory.

## Target Architecture

### Target Module Map (Planned)

```
nebula-metrics/ (future crate)
├── registry.rs   — Unified registry or adapter to telemetry
├── export/       — Prometheus, OTLP exporters
├── naming.rs     — Standard metric names (nebula_*)
└── integration/  — Adapters for telemetry, log, domain crates
```

### Public Contract Boundaries

- `MetricsRegistry` trait or adapter over telemetry.
- Export endpoints: `/metrics` (Prometheus), OTLP push.
- Naming convention: `nebula_<domain>_<metric>_<unit>`.

### Internal Invariants

- Export never blocks hot path; metrics recorded asynchronously or via pull.
- Cardinality bounded; no unbounded label sets.

## Design Reasoning

### Key Trade-off 1: Standalone crate vs telemetry extension

- **Chosen:** TBD; either extract `nebula-metrics` or extend telemetry (Phase 3).
- **Rationale:** Telemetry ROADMAP already plans Prometheus/OTLP export; metrics crate would centralize export and naming.
- **Consequence:** Decision in PROPOSALS.

### Key Trade-off 2: Standard metrics crate vs custom

- **Current:** Telemetry uses custom in-memory; log uses standard `metrics` crate.
- **Target:** Unified on standard `metrics` crate for export compatibility, or keep telemetry primitives with adapter layer.

### Rejected Alternatives

- **Prometheus as direct dep in telemetry:** Would force Prometheus on all consumers; prefer optional export.
- **Per-crate Prometheus:** Duplication; no unified scrape.

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces, Temporal/Prefect/Airflow.

| Pattern | Verdict | Rationale |
|---------|---------|-----------|
| Prometheus metrics | **Adopt** | Industry standard; Grafana integration |
| OTLP export | **Adopt** | Cloud-native; vendor-neutral |
| Unified registry | **Adopt** | Single scrape endpoint; consistent naming |
| Per-crate metrics | **Defer** | Domain crates may keep internal metrics; export via adapter |

## Breaking Changes (if any)

- None until metrics crate or telemetry Phase 3 export is implemented.

## Open Questions

- Q1: Extract nebula-metrics crate or extend telemetry?
- Q2: Standard metric names for workflow, node, action, credential, resource?
