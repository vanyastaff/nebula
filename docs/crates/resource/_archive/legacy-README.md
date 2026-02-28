# nebula-resource

Runtime resource lifecycle for Nebula workflows.

This crate owns:
- resource registration and dependency ordering (`Manager`, `DependencyGraph`)
- bounded pooling with back-pressure (`Pool`, `PoolConfig`, `PoolStrategy`)
- scope-safe access control (`Scope`, `Strategy`)
- health monitoring and threshold callbacks (`HealthChecker`, `HealthPipeline`)
- lifecycle observability (events, hooks, optional metrics/tracing)
- quarantine and auto-scaling extensions

## Why it exists

Workflow nodes should not construct expensive clients/connections directly.  
`nebula-resource` centralizes creation, reuse, cleanup, and failure handling so actions/triggers can request resources safely and consistently.

## Public entry points

- `Manager` / `ManagerBuilder` for registration, acquire/release, shutdown
- `Resource` and `Config` traits for custom resource types
- `ResourceProvider` and `ResourceRef` for decoupled access from runtime/action layers
- `Context` and `Scope` for isolation and policy checks

## Docs map

- [ARCHITECTURE.md](./ARCHITECTURE.md) - current design and runtime flow
- [API.md](./API.md) - practical API reference with usage patterns
- [DECISIONS.md](./DECISIONS.md) - key architectural decisions and tradeoffs
- [ROADMAP.md](./ROADMAP.md) - staged implementation plan
- [PROPOSALS.md](./PROPOSALS.md) - candidate improvements and breaking changes

## Archive

Previous docs and imported legacy notes were moved to:
- [`_archive/`](./_archive/)
