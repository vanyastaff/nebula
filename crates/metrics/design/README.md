# nebula-metrics

Unified metric naming and export adapters for the Nebula workflow engine.

## Scope

- **In scope:**
  - Standard metric naming convention (`nebula_*` prefix) — **implemented** in `nebula-metrics`
  - Adapter over `nebula-telemetry::MetricsRegistry` with canonical names — **implemented**
  - Prometheus/OTLP export backends — **planned** (Phase 3; `prometheus` feature stub present)
  - Integration with nebula-telemetry in-memory primitives — **implemented** via `TelemetryAdapter`

- **Out of scope:**
  - In-memory metric primitives (live in `nebula-telemetry`)
  - Structured logging and traces (see `nebula-log`)
  - Domain-specific metrics implementations (memory, credential, resource) — they stay in domain crates; export via adapters later

## Current State

- **Maturity:** Crate exists (`crates/metrics`); naming + adapter done; export planned (Phase 3)
- **Key strengths:** Metrics implemented in `nebula-telemetry` (Counter, Gauge, Histogram, MetricsRegistry); `nebula-log` has standard `metrics` crate (observability feature); domain crates (memory, credential, resource, resilience) have their own metrics
- **Key risks:** Fragmented metrics landscape; no unified export; telemetry Histogram unbounded
- **Detailed baseline:** [CURRENT_STATE.md](./CURRENT_STATE.md) — telemetry, log, engine/runtime, resource, credential, resilience, memory

## Target State

- **Production criteria:** Standalone `nebula-metrics` crate or telemetry Phase 3 export; Prometheus scrape endpoint; OTLP push; bounded histograms
- **Compatibility guarantees:** Metric names stable; export format Prometheus-compatible
- **Unified export and naming:** [TARGET.md](./TARGET.md) — `nebula_*` convention and export goals

## Phase 1 (Documentation and Alignment)

Phase 1 is **complete** when: (1) this doc set is complete, (2) current state and target are documented, (3) ROADMAP is aligned with [telemetry](../telemetry/ROADMAP.md) and [eventbus](../eventbus/ROADMAP.md) ROADMAPs. See [ROADMAP.md](./ROADMAP.md) for the checklist.

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [ROADMAP.md](./ROADMAP.md)
- [CURRENT_STATE.md](./CURRENT_STATE.md) — current state: telemetry, log, domain crates
- [TARGET.md](./TARGET.md) — target: unified export, naming convention
- [MIGRATION.md](./MIGRATION.md)


