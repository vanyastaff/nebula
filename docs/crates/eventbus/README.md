# nebula-eventbus (Planned)

Generic pub/sub event bus for asynchronous communication between Nebula components.

## Scope

- **In scope (target):**
  - Broadcast-based event distribution (fan-out to multiple subscribers)
  - Scoped subscriptions (workflow, execution, resource)
  - Event filtering (by type, scope, payload)
  - Back-pressure policies (DropOldest, DropNewest, Block)
  - Automatic cleanup on scope drop
  - Typed event channels per domain (ExecutionEvent, ResourceEvent, etc.)

- **Out of scope:**
  - Metrics and telemetry (see `nebula-telemetry`)
  - Structured logging (see `nebula-log`)
  - Distributed event transport (Phase 2+; single-node first)

## Current State

- **Maturity:** Planned — no standalone crate; EventBus implementations exist in `nebula-telemetry` (ExecutionEvent) and `nebula-resource` (ResourceEvent)
- **Key strengths:** Proven broadcast pattern in telemetry and resource; tokio::sync::broadcast; fire-and-forget semantics; zero blocking in hot path
- **Key risks:** Duplicated EventBus logic across crates; no unified scoped subscriptions or filtering; no shared abstraction

## Target State

- **Production criteria:** Single generic EventBus crate; scoped subscriptions; event filtering; BackPressurePolicy; consumers: telemetry, resource, log, metrics
- **Compatibility guarantees:** Event schema additive-only; emit never blocks; backward-compatible with current telemetry/resource EventBus APIs

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
