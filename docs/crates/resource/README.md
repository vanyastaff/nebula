# nebula-resource

Lifecycle and pooling runtime for external and internal resources used by workflow execution.

## Scope

- In scope:
  - resource registration and dependency ordering
  - scoped access control (tenant/workflow/execution/action)
  - pooling, acquire back-pressure, recycling, cleanup
  - health, quarantine, hooks, events, optional metrics/autoscaling
  - provider contracts for runtime/action crates
- Out of scope:
  - business logic of actions/triggers
  - persistence of workflow state
  - transport-specific drivers (should live in separate crates)

## Current State

- maturity: advanced implementation with broad integration and stress test coverage.
- key strengths:
  - `ManagerBuilder` and `Manager` expose practical runtime knobs
  - `Pool` handles cancellation, fairness strategy, and lifecycle transitions
  - strict scope containment prevents cross-tenant leakage by default
  - rich test suite covers races, shutdown, exhaustion, hooks, health, autoscaling
  - `Poison<T>` arm/disarm guard prevents corrupted pool state from being reused after panics or cancelled futures
  - `Gate`/`GateGuard` cooperative shutdown barrier ensures in-flight maintenance tasks drain cleanly before pool close
- key risks:
  - API surface is large and some paths are feature-gated, increasing integration complexity
  - `reload_config` performs full pool swap, which can be disruptive for some resources

## Target State

- production criteria:
  - stable cross-crate contract for runtime/action/resource usage
  - deterministic failure semantics under load and during shutdown
  - documented policy profiles for acquire behavior and scoping
  - hard guarantees for no secret leakage and no resource leaks
- compatibility guarantees:
  - additive traits/methods/events in minor releases
  - scope semantics, error taxonomy, and acquire contracts change only in major releases

## Document Map

- planning and migration docs (this directory):
  - [ROADMAP.md](./ROADMAP.md)
  - [MIGRATION.md](./MIGRATION.md)
  - [PLAN.md](./PLAN.md)
  - [TASKS.md](./TASKS.md)
  - [VISION.md](./VISION.md)
- implementation and API docs (in crate):
  - [README.md](../../../crates/resource/docs/README.md) — overview, concepts, feature matrix
  - [architecture.md](../../../crates/resource/docs/architecture.md) — module map, data flow, design invariants
  - [api-reference.md](../../../crates/resource/docs/api-reference.md) — complete typed API reference
  - [pooling.md](../../../crates/resource/docs/pooling.md) — pool config, strategies, backpressure, auto-scaling
  - [health-and-quarantine.md](../../../crates/resource/docs/health-and-quarantine.md) — health checks, quarantine, recovery
  - [events-and-hooks.md](../../../crates/resource/docs/events-and-hooks.md) — EventBus catalog, subscriptions, hook system
  - [adapters.md](../../../crates/resource/docs/adapters.md) — writing Resource adapter / driver crates


