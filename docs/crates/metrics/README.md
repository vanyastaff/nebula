# nebula-metrics (Planned)

Unified metrics collection and export for the Nebula workflow engine.

## Scope

- **In scope (target):**
  - Prometheus/OTLP export backends
  - Standard metric naming convention (`nebula_*` prefix)
  - Integration with nebula-telemetry in-memory primitives
  - System, business, and custom metrics aggregation

- **Out of scope:**
  - In-memory metric primitives (live in `nebula-telemetry`)
  - Structured logging and traces (see `nebula-log`)
  - Domain-specific metrics implementations (memory, credential, resource)

## Current State

- **Maturity:** Planned — no standalone metrics crate exists
- **Key strengths:** Metrics implemented in `nebula-telemetry` (Counter, Gauge, Histogram, MetricsRegistry); `nebula-log` has standard `metrics` crate (observability feature); domain crates (memory, credential, resource, resilience) have their own metrics
- **Key risks:** Fragmented metrics landscape; no unified export; telemetry Histogram unbounded

## Target State

- **Production criteria:** Standalone `nebula-metrics` crate or telemetry Phase 3 export; Prometheus scrape endpoint; OTLP push; bounded histograms
- **Compatibility guarantees:** Metric names stable; export format Prometheus-compatible

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [INTERACTIONS.md](./INTERACTIONS.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)
- [SECURITY.md](./SECURITY.md)
- [RELIABILITY.md](./RELIABILITY.md)
- [TEST_STRATEGY.md](./TEST_STRATEGY.md)
- [MIGRATION.md](./MIGRATION.md)

## Archive

Legacy material:
- [`_archive/`](./_archive/)
