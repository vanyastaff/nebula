# nebula-cluster

Planned distributed execution and coordination layer for Nebula.

## Scope

- In scope:
  - cluster membership and node coordination
  - distributed workflow scheduling and failover
  - consensus-backed control-plane state
  - autoscaling and rebalancing policy contracts
- Out of scope:
  - local single-node execution internals
  - workflow definition semantics
  - tenant policy ownership (separate crate boundary)

## Current State

- maturity: planned; `crates/cluster` is not implemented yet.
- key strengths:
  - archived architectural direction already defines core concerns (consensus, scheduling, failover, autoscaling).
  - supporting crates (`runtime`, `execution`, `storage`, `telemetry`) exist and can anchor integration.
- key risks:
  - distributed concerns are currently not centralized, risking contract drift across runtime/worker/storage.

## Target State

- production criteria:
  - deterministic cluster control plane for scheduling/failover decisions
  - safe membership transitions and fault handling
  - observable and operable distributed execution behavior
- compatibility guarantees:
  - additive scheduling/metrics APIs in minor releases
  - consensus/state-machine and scheduling semantics change only in major releases

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
