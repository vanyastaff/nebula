---
id: 0012
title: checkpoint-recovery
status: accepted
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [engine, execution, storage, checkpoints, resume, persistence]
related:
  - docs/PRODUCT_CANON.md#115-persistence--operators
  - docs/PRODUCT_CANON.md#122-execution-single-semantic-core-durable-control-plane
  - docs/adr/0009-resume-persistence-schema.md
  - crates/storage/src/execution_repo.rs
  - crates/engine/src/engine.rs
linear:
  - NEB-150
---

# 0012. Checkpoint recovery model

## Context

Nebula runs long-horizon workflows — minutes to days (PRODUCT_CANON §1).
Process restarts happen; networks flap; third-party APIs time out mid-step.
A workflow engine that re-executes every finished step on resume would be
useless at the scale we target. A workflow engine that promises
exactly-once without owning the primitives would be dishonest.

This ADR records the **recovery model** already in force in
[`§11.5`](../PRODUCT_CANON.md#115-persistence--operators), so future
design work has a single pointer rather than re-deriving the policy from
invariants.

[`ADR-0009 — Resume persistence schema`](./0009-resume-persistence-schema.md)
is the *mechanism* ADR (what rows and columns store what). This one is
the *policy* ADR (what contract resume carries).

## Decision

1. **Checkpointing is policy-driven, not "fsync every step."** The engine
   writes a checkpoint at declared boundaries (workflow- or action-level
   policy) and on workflow completion. Between checkpoints, progress is
   in-memory and may be lost on crash. Authors place boundaries before
   side effects that are **irreversible or expensive to re-run**.
2. **Recovery falls back to the last successful checkpoint.** On resume,
   the engine reconstructs state from the durable surface (`executions` +
   `execution_journal` + `stateful_checkpoints`). Work performed *after*
   the last successful checkpoint may be replayed. Authors treat such
   replay as the expected case for any step that lacks an idempotency
   key.
3. **Checkpoint-write failure is best-effort.** A failed checkpoint write
   is **logged** but does **not** abort the live execution. The engine
   keeps running; the failed write surfaces as stale durable state and
   an alert, not as a crash. Resume picks up from whatever checkpoint
   last made it to durable storage.
4. **Idempotency keys, not "exactly once."** The
   checkpoint-vs-side-effect race (side effect commits externally, then
   the checkpoint write fails) is a real failure mode and is handled by
   design through idempotency keys (PRODUCT_CANON §11.3 / §11.6). Docs
   and APIs **do not advertise exactly-once**; they advertise
   **at-least-once with idempotency**.
5. **The durable surface is authoritative.** In-process `mpsc` channels
   and ephemeral state are **never** treated as recovery inputs. Any
   control signal — cancel, restart, resume — lives in
   `execution_control_queue` (§12.2) and is driven by the engine's
   single consumer wiring per deployment mode.

## Consequences

**Positive**

- Operators have one checkpoint story to audit; no hidden second
  durability layer.
- Recovery cost is bounded by "time since last checkpoint" — authors
  tune that by placing boundaries where replay hurts.
- The engine stays live across transient storage hiccups; one failed
  checkpoint write does not take down a long-running run.

**Negative**

- Work between checkpoints is at risk. A run that places *no* checkpoints
  and crashes loses everything it did that session. The engine trades
  per-step strictness for long-run survivability; authors must
  understand the trade.
- Replay requires either idempotency or an author-side compensation
  story. Nebula does not mask this with false promises.

**Neutral**

- Checkpoint frequency is a knob, not a constant. Tuning happens per
  workflow / action, not globally.

## Alternatives considered

- **Checkpoint every step (synchronous fsync).** Reject. Dominates run
  wall-time for any non-trivial workflow; degrades operator experience
  more than it protects work.
- **Pretend exactly-once.** Reject. Not achievable without owning the
  two-phase-commit path with every external system we call; lying about
  it in docs violates §11.6 (documentation truth).
- **Discard the journal, keep only latest checkpoint.** Reject. Loses
  the replayable history that drives observability, debugging, and
  forensic analysis of failed runs.

## Follow-ups

- Keep `crates/storage/src/execution_repo.rs` and
  `crates/engine/src/engine.rs` in sync with this policy; any change to
  checkpoint placement, write semantics, or replay contract lands as a
  new ADR that supersedes this one.
- `docs/OBSERVABILITY.md` should keep a narrative of "what operators see
  when checkpoint writes fail" — this ADR pins the policy it describes.
- `nebula-resilience` wiring around retries (M2 exit criterion) must
  interact with checkpoint boundaries explicitly — its ADR will cite
  this one.
