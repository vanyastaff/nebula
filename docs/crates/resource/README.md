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

- [CONSTITUTION.md](./CONSTITUTION.md) — platform role, principles, production vision
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
