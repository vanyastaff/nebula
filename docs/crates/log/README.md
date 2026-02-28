# nebula-log

Logging and observability foundation for the Nebula workflow platform.

## Scope

- **In scope:**
  - tracing initialization and configuration
  - structured logging (compact, pretty, JSON, logfmt)
  - async-safe context propagation (request/user/session)
  - operation timing primitives and macros
  - pluggable observability event hooks
  - optional metrics/telemetry integrations (OpenTelemetry, Sentry)
- **Out of scope:**
  - workflow domain logic
  - persistence of logs (delegated to writers/backends)
  - alerting rules (consumers use emitted events)

## Current State

- **Maturity:** Production-ready core; baseline writer fanout and size rolling are implemented
- **Key strengths:**
  - tracing-first design with structured spans/events
  - feature-gated integrations (minimal footprint)
  - panic-isolated hook dispatch
  - config presets (dev/prod/env)
  - multi-destination writer fanout with explicit failure policy
  - size-based rolling file writer
- **Key risks:**
  - non-blocking file mode backpressure/drop semantics need clearer operator guidance
  - hook execution remains inline by default (tail-latency sensitivity)
  - global observability registry requires test serialization

## Target State

- **Production criteria:**
  - formal env var and config precedence contract
  - benchmarked hot paths with CI thresholds
  - documented hook/runtime failure behavior under load
- **Compatibility guarantees:**
  - config schema stability via versioning and snapshot tests
  - deprecation window for breaking API changes

## Feature Flags

- `default`: `ansi`, `async`
- `file`: file writer + rolling support
- `log-compat`: bridge `log` crate into tracing
- `observability`: metrics + hooks
- `telemetry`: OpenTelemetry pipeline
- `sentry`: Sentry integration
- `full`: enables all major capabilities

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
