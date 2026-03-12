# nebula-telemetry

Event bus, metrics, and telemetry for the Nebula workflow engine.

## Scope

- **In scope:**
  - Execution lifecycle events (EventBus, ExecutionEvent)
  - In-memory metrics primitives (Counter, Gauge, Histogram)
  - TelemetryService trait and NoopTelemetry implementation
  - Fire-and-forget event projection (events are projections, not source of truth)

- **Out of scope:**
  - Structured logging (see `nebula-log`)
  - OpenTelemetry traces / Sentry (see `nebula-log` telemetry features)
  - Prometheus/OTLP export (planned; see ROADMAP.md)
  - Distributed tracing span creation (handled by consumers via tracing crate)

## Current State

- **Maturity:** MVP — in-memory metrics, broadcast event bus, no external exporters
- **Key strengths:** Zero external deps for core path; pluggable via `TelemetryService`; engine/runtime integration complete
- **Key risks:** Histogram stores all observations in memory (unsuitable for high cardinality); no export path for production dashboards

## Target State

- **Production criteria:** Prometheus/OTLP exporter; bounded histogram; event schema versioning
- **Compatibility guarantees:** Event schema additive-only; metrics names stable; TelemetryService trait backward compatible

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [ROADMAP.md](./ROADMAP.md)
- [MIGRATION.md](./MIGRATION.md)


