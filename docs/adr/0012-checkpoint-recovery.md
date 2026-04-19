---
id: 0012
title: checkpoint-recovery
status: proposed
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [execution, durability, recovery, storage]
related:
  - docs/PRODUCT_CANON.md#122-execution-single-semantic-core-durable-control-plane
  - docs/PRODUCT_CANON.md#115-persistence--operators
  - crates/execution
  - crates/storage
  - crates/eventbus
linear:
  - NEB-150
---

# 0012. Checkpoint-based recovery (not event sourcing)

## Context

A durable workflow engine must recover from crashes mid-run. Two canonical
models:

1. **Event sourcing.** Log every state change, replay to rebuild. Temporal's
   model.
2. **Checkpoint snapshots.** Periodically write frontier + node outputs;
   recovery = load the latest checkpoint. Airflow's model.

Trade-offs:

- Event sourcing is replay-correct and deterministic, but forces workflows
  into a deterministic execution model (no wall-clock, no random, no
  arbitrary I/O without sandboxing the effects). Unbounded log growth also
  requires compaction engineering.
- Checkpointing is simpler, makes non-deterministic workflows feasible, but
  loses fine-grained audit unless supplemented. Recovery resumes from the
  last known good state — some re-execution is possible.

## Decision

Use **checkpoint-based recovery** as the authoritative durability model.

- The engine writes a frontier snapshot plus per-node outputs at checkpoint
  boundaries.
- Checkpoints are written via `nebula-storage` (SQLite or Postgres).
- Recovery loads the latest valid checkpoint and resumes from the frontier.
- For nodes that are **non-idempotent**, the runtime marks them with an
  explicit flag and the engine may re-ask the user / skip / require manual
  intervention on recovery.
- An audit log is still emitted through `nebula-eventbus` for observability
  — but the event log is **not** the authoritative state.

## Consequences

**Positive**

- Workflows can use wall-clock, randomness, and arbitrary I/O without a
  sandbox.
- Recovery is O(1) from the latest checkpoint.
- No log-compaction engineering.

**Negative**

- Non-idempotent side effects may execute twice on crash-recovery; mitigated
  by explicit idempotency markers (`PRODUCT_CANON` §11.3) and at-least-once
  semantics in the contract.
- Audit granularity is coarser than event-sourced — we rely on the eventbus
  stream for fine detail.

## Alternatives considered

- **Event sourcing (Temporal-style).** Rejected because forcing workflow
  authors into a deterministic execution model conflicts with Nebula's goal
  of letting authors write straightforward Rust, including wall-clock, I/O,
  and ecosystem crates. Also rejected on engineering cost: log growth,
  compaction, and sandboxing effects are a large surface area for a model
  whose benefits (fine-grained replay audit) can be approximated by the
  eventbus stream without owning authoritative state.

## Follow-ups

- `PRODUCT_CANON` §12.2 and §11.5 govern the boundary between checkpoint
  writes, the durable control queue, and best-effort semantics. This ADR is
  the decision record those sections refer back to.
- Non-idempotent node markers and the recovery UX (re-ask / skip / manual)
  are tracked as separate work under the checkpoint-recovery milestone.

## References

- `PRODUCT_CANON` §12.2 — Execution: single semantic core, durable control
  plane (`docs/PRODUCT_CANON.md`).
- `PRODUCT_CANON` §11.5 — Persistence & operators (checkpoint boundaries,
  best-effort framing).
- Airflow's checkpointing model (reference).
- Temporal's event-sourced workflow model (reference, rejected).
