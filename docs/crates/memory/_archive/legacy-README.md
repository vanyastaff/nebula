# nebula-memory

Memory subsystem for Nebula runtime components.

This crate provides:
- specialized allocators (`bump`, `pool`, `stack`)
- object pooling primitives (`ObjectPool`, `ThreadSafePool`, TTL/priority/hierarchical variants)
- cache implementations (compute, concurrent, multi-level, partitioned, scheduled)
- memory budgeting and pressure control
- optional statistics/monitoring, logging, async helpers

## Goals

- predictable latency under high allocation churn
- safe reuse patterns without leaking resources
- modular feature flags so binaries only include needed pieces

## Docs Map

| Document | Description |
|----------|-------------|
| [ARCHITECTURE.md](./ARCHITECTURE.md) | Module layout, runtime model, feature-gated design |
| [API.md](./API.md) | Core imports, usage examples, error model |
| [INTERACTIONS.md](./INTERACTIONS.md) | Ecosystem map, upstream/downstream contracts |
| [DECISIONS.md](./DECISIONS.md) | Key architectural decisions and rationale |
| [ROADMAP.md](./ROADMAP.md) | Planned improvements and phases |
| [PROPOSALS.md](./PROPOSALS.md) | Future ideas under consideration |
| [SECURITY.md](./SECURITY.md) | Threat model, security controls, abuse cases |
| [RELIABILITY.md](./RELIABILITY.md) | SLO targets, failure modes, resilience strategies |
| [TEST_STRATEGY.md](./TEST_STRATEGY.md) | Test pyramid, invariants, tooling |
| [MIGRATION.md](./MIGRATION.md) | Versioning policy, breaking changes, rollout |

## Archive

Legacy and imported materials were moved to:
- [`_archive/`](./_archive/)
