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

## Docs map

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [DECISIONS.md](./DECISIONS.md)
- [ROADMAP.md](./ROADMAP.md)
- [PROPOSALS.md](./PROPOSALS.md)

## Archive

Legacy and imported materials were moved to:
- [`_archive/`](./_archive/)
