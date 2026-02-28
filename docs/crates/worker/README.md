# nebula-worker

Distributed worker runtime for task acquisition, isolated node execution, and execution result reporting.

## Scope

- In scope:
  - worker lifecycle and heartbeats
  - task pull/ack protocol from queue broker
  - execution concurrency and backpressure
  - integration with sandbox/resource/resilience/logging crates
  - operational metrics and worker-level health
- Out of scope:
  - workflow planning and DAG orchestration (engine/runtime)
  - node/action business logic (action/plugin)
  - user-facing API contracts (api/webhook)

## Current State

- maturity: planned docs-level design; `crates/worker` is not implemented yet.
- key strengths:
  - clear legacy intent: pool, scaling, isolation, progress/health, graceful shutdown.
  - strong alignment with production ops requirements (autoscaling, metrics, deployment).
- key risks:
  - no executable contract yet; risk of interface drift with `runtime`, `queue`, `sandbox`.
  - reliability and idempotency rules must be fixed before implementation.

## Target State

- production criteria:
  - deterministic task state machine (`queued -> claimed -> running -> succeeded/failed`)
  - bounded concurrency and queue backpressure under overload
  - graceful draining and lease handoff on shutdown/restart
  - hard resource isolation and policy enforcement per task
  - first-class observability (metrics, traces, structured logs)
- compatibility guarantees:
  - wire and trait contracts versioned, additive-first in minor releases
  - breaking execution semantics only in major release with migration path

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
