# nebula-eventbus

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

- **Maturity:** Active in workspace (`crates/eventbus`) and integrated by telemetry/resource.
- **Implemented:** Generic `EventBus<E>`, `BackPressurePolicy`, `PublishOutcome`, `EventBusStats`, scoped/filter subscriptions, `EventBusRegistry`, benchmarks, and `nebula-metrics` snapshot integration.
- **Current risks:** Distributed transport backends (Redis/NATS) are not implemented yet; single-node in-process delivery only.

## Target State

- **Production criteria:** Eventbus remains transport-focused with predictable back-pressure semantics and explicit publish outcomes.
- **Compatibility guarantees:** Event schemas are additive-first; emit path remains non-blocking by default; wrappers in domain crates preserve stable ergonomics.

## Event Schema Versioning (T024)

Eventbus transports opaque domain events and does not own event payload schemas. Versioning policy applies to domain crates (`nebula-telemetry`, `nebula-resource`, etc.) that define events.

- Keep event enum variants additive in minor releases.
- Avoid reusing variant names with changed semantics.
- Add optional fields instead of replacing/removing existing fields.
- Reserve removals and semantic rewrites for major versions.
- Prefer explicit variant evolution (`NodeCompletedV2`) when semantics materially change.

### Recommended Pattern

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ExecutionEvent {
  Started {
    schema_version: u16,
    execution_id: String,
    workflow_id: String,
  },
  NodeCompleted {
    schema_version: u16,
    execution_id: String,
    node_id: String,
    duration_ms: u64,
    // New optional field for additive evolution.
    retries: Option<u32>,
  },
}
```

### Compatibility Rules

- Producers may emit newer additive payloads.
- Consumers must ignore unknown optional fields and unknown variants when possible.
- Schema version should be carried by domain events, not by `EventBus` transport types.

## Document Map

- [ARCHITECTURE.md](./ARCHITECTURE.md)
- [API.md](./API.md)
- [ROADMAP.md](./ROADMAP.md)
- [MIGRATION.md](./MIGRATION.md)


