---
id: 0008
title: execution-control-queue-consumer
status: accepted
date: 2026-04-18
supersedes: []
superseded_by: []
tags: [engine, control-queue, lifecycle, canon-12.2, outbox]
related: [crates/engine/src/control_consumer.rs, crates/storage/src/repos/control_queue.rs, crates/api/src/handlers/execution.rs, docs/PRODUCT_CANON.md]
---

# 0008. Execution control-queue consumer

## Context

Canon §12.2 mandates a durable control plane: every `Cancel` / `Terminate` /
`Resume` / `Restart` signal is written to `execution_control_queue` in the
same logical operation as its state transition, and "a dispatch worker
drains the queue and forwards commands to a consumer that the engine
actually listens to." Canon §12.2 also names this as a non-negotiable L2
invariant: "a demo handler that logs the command and discards it does not
satisfy this invariant."

Today (pre-0008):

- **Producer exists.** `crates/api/src/handlers/execution.rs:311-368`
  transitions state via CAS then enqueues the `Cancel` signal. Ordering and
  503-on-backend-down behaviour already comply with §13 step 6.
- **Consumer does not exist.** `grep -rn "ControlQueueRepo\|ControlCommand::"
  crates/engine/` returns zero hits. The engine never imports the repo, never
  instantiates it, never drains it.
- **Docs lie twice.** `crates/engine/src/lib.rs:11-13` asserts that the
  engine is "the single real consumer"; `crates/engine/README.md:20-22` says
  the same thing; `crates/storage/src/repos/mod.rs:7` calls the queue
  "Consumed by the API cancel handler" — the API is the producer, not the
  consumer. All three are §11.6 false-capability / documentation-truth bugs.
- **No binary wires both API and Engine.** `apps/cli/*` constructs
  `WorkflowEngine` for in-process one-shot runs (no external producer writes
  to the queue in those modes); `crates/api/examples/simple_server.rs`
  constructs the API without a `WorkflowEngine` and already carries a
  "DEMO ONLY" marker per §12.2.
- **Knife scenario §13 step 5 cannot pass** without a consumer: the API
  enqueues `Cancel`, nothing drains the queue, nothing calls the engine's
  cancel path, the execution never reaches `Cancelled`.

This ADR records the wiring decisions for the `ControlConsumer` landed in
A1 of the engine-lifecycle canon cluster. A1 is the skeleton; A2 and A3
layer `start` and `cancel` dispatch on top; A4 adds the knife integration
test that exercises all three end-to-end.

## Decision

### 1. Wiring shape — polling loop with backoff + claim/ack

The consumer lives in `crates/engine::control_consumer` and drains the queue
via the existing `ControlQueueRepo::claim_pending` / `mark_completed` /
`mark_failed` surface — the same shape the Postgres implementation will
require (`FOR UPDATE SKIP LOCKED`). The consumer:

1. Calls `claim_pending(processor_id, batch_size)` to atomically claim a
   batch.
2. For each claimed entry, calls the engine-owned dispatch trait (see
   decision 2). A1 dispatches nothing; A2 wires `Resume` / `Restart` →
   start-path; A3 wires `Cancel` / `Terminate` → cancel-path.
3. On success: `mark_completed(id)`.
4. On dispatch error: `mark_failed(id, error)` so the row is not reclaimed
   on the next poll (avoids a poison-pill reclaim loop).
5. Sleeps a bounded interval when `claim_pending` returns empty; wakes
   immediately when commands arrive in the next tick.

**Alternatives considered and rejected:**

- **In-process `tokio::sync::mpsc` channel from the API producer directly
  to the engine.** Violates §4.5 ("in-process channels are not a durable
  backbone") and §12.2 ("any second control channel... is forbidden unless
  the canon is updated with a reconciliation story"). Also does not survive
  API restarts.
- **Postgres `LISTEN / NOTIFY` push.** Correct for Postgres but cannot be
  the only wiring: the in-memory and SQLite paths (canon §12.3 local path)
  have no equivalent. `LISTEN / NOTIFY` is additive — a future optimisation
  that reduces poll latency by waking the loop early; the loop is still
  authoritative so the local path keeps working.
- **Per-command task spawn from the enqueue site.** Couples API and engine
  processes; breaks §12.1 layering (API must not own engine dispatch) and
  loses durability across crashes.

### 2. Surface boundary — engine-owned dispatch trait

The consumer depends on an **engine-owned** trait named `ControlDispatch`,
defined inside `crates/engine/src/control_consumer.rs`. It is the only
surface the consumer knows about for delivering commands to running work.
Public methods on `ControlConsumer` accept `Arc<dyn ControlDispatch>` — no
type from `nebula-api` or `nebula-storage`'s row layer appears on the
consumer's **public** signatures beyond `Arc<dyn ControlQueueRepo>` (which
is already the engine's legitimate storage-layer dependency per the layer
rules in `CLAUDE.md`).

Concrete rules, enforced by a compile-time test in A1:

- `ControlConsumer::new` takes `Arc<dyn ControlQueueRepo>` (storage port),
  `Arc<dyn ControlDispatch>` (engine port), and a processor identifier.
- `ControlDispatch` trait methods take typed engine arguments (e.g.
  `ExecutionId`), **not** `ControlQueueEntry` / raw byte slices.
- No `nebula_api::*` type appears anywhere in the consumer module.
- Translation from `ControlQueueEntry` (storage encoding, UTF-8 ULID bytes)
  to typed `ExecutionId` happens inside the consumer, so `ControlDispatch`
  implementors see only validated domain types.

This keeps the engine's bounded context clean: storage types stay in the
consumer's input boundary, engine types flow out to dispatch.

### 3. Atomicity contract — documented at-least-once + idempotent consumer

Canon §12.2 requires the producer side to write the control row "in the
same logical operation" as the state transition. Today the API cancel
handler (`crates/api/src/handlers/execution.rs:311-327`) achieves this by
ordering:

1. CAS transition via `ExecutionRepo::transition`.
2. Enqueue via `ControlQueueRepo::enqueue`.

If step 2 fails after step 1 succeeds, the execution row is already
`cancelled` but the engine never sees the signal. The handler returns 503
(per §13 step 6) so the caller retries; the retry sees the terminal status
and short-circuits (idempotent producer).

This ADR **accepts** the orphan window explicitly for the in-memory /
SQLite paths, because a real shared-transaction wrapper requires
`execution_repo` and `control_queue_repo` to live in the same backend — a
Postgres-only concern tracked as a follow-up. The comment at
`crates/api/src/handlers/execution.rs:311-315` already documents this; the
`ControlConsumer` does not attempt to reconcile it.

**Consumer-side semantics — at-least-once + idempotent:**

- `claim_pending` moves rows to `Processing` before dispatch. A crash
  between claim and dispatch leaves the row in `Processing`; the reclaim
  sweep (B1, ADR-0017) recovers it by moving the row back to `Pending`
  after `reclaim_after` (default 150s, 5× the ADR-0015 lease TTL) and
  bumping `reclaim_count`. Rows past `max_reclaim_count` (default 3) are
  moved to `Failed` with error `"reclaim exhausted: processor <id> presumed
  dead after <N> reclaims"` so an operator sees genuinely poisoned
  commands. The sweep is safe under concurrent runners — CAS on the
  `Processing → Pending` status transition fences duplicates.
- Idempotency contract on the `ControlDispatch` trait: implementors must
  treat a repeated command for a terminal execution as a no-op (e.g. a
  second `Cancel` on an already-`Cancelled` execution returns `Ok`, not an
  error). A2 / A3 define this explicitly when they land `start` / `cancel`.
- `mark_failed` records a human-readable error on the row; the operator
  sees it via `SELECT ... FROM execution_control_queue WHERE status =
  'Failed'`. Failed rows are not auto-retried — canon §12.2 "removing rows
  before the engine has acted is broken" applies symmetrically to
  auto-retry after failure, which could mask a bug.

### 4. `simple_server.rs` — keeps DEMO ONLY marker; does not run the consumer

The example already carries an explicit "DEMO ONLY — no real engine
consumer" comment (`crates/api/examples/simple_server.rs:21-24`). A1 does
**not** wire the consumer into that example for two reasons:

1. The example does not instantiate `WorkflowEngine` at all. Adding the
   full engine construction (plugin registry, action runtime, sandbox,
   metrics, credential / resource managers) grows A1 far beyond
   "skeleton + ADR" scope.
2. The consumer is only useful if the engine has a dispatch trait
   implementation available — A2 lands the start path, A3 lands the cancel
   path. Wiring the consumer into an example before A3 would produce a
   DEMO-level consumer that logs and drops commands, which is exactly the
   §12.2 antipattern this ADR is eradicating.

Decision: the example's existing marker is kept and the comment is
updated to reference this ADR so a future reader knows where the real
consumer lives. A4 (knife integration test) is the canonical "both wired
together" seam; A proper single-binary production composition root
(planned name `apps/server` or equivalent) is out of scope for Group A and
tracked separately.

### 5. At-least-once delivery and dispatch failure handling

Concrete rules the consumer honors from A1 forward:

- **Same command delivered twice** — the consumer's `ControlDispatch`
  contract requires implementors to be idempotent by execution id
  + command. The dispatch layer sees only typed arguments, so a repeated
  `dispatch_cancel(execution_id)` on a terminal execution returns `Ok`.
- **Dispatch returns an error** — the consumer calls `mark_failed(id, err)`
  and continues with the next entry. The row stays `Failed`; no implicit
  retry. This is deliberate: §12.2 explicitly treats "removing rows before
  the engine has acted" as broken, and silent retry after a genuine
  dispatch bug would mask it.
- **Storage error on `mark_completed` / `mark_failed`** — the consumer logs
  at `error` level and continues. The row stays `Processing` and will be
  picked up by the reclaim path (tracked as B1 follow-up). Skipping ack is
  not the same as discarding the command — the next poll cycle or the
  reclaim path will retry.
- **Consumer panics inside a dispatch call** — `tokio::task` isolation
  bounds the blast radius to the single task; the row stays `Processing`.
  Graceful shutdown via `CancellationToken` flushes in-flight work and
  returns; forced shutdown leaves the row for reclaim.

## Consequences

Positive:

- §13 step 5 becomes implementable — A2 / A3 / A4 can land progressively
  on this skeleton without redoing the wiring story.
- Three doc-truth bugs fixed in the same PR as the skeleton lands
  (`crates/engine/src/lib.rs`, `crates/engine/README.md`,
  `crates/storage/src/repos/mod.rs`).
- `ControlDispatch` is the single engine-owned seam future dispatch paths
  (start, cancel, terminate, resume, restart) land behind. A2 and A3 extend
  this trait; the consumer does not change shape.
- Layer boundary preserved — no `nebula-api` or `storage`-private types
  appear on the consumer's public surface.

Negative / accepted costs:

- A1 introduces a spawned task that, on its own, performs no useful work
  (dispatches log and TODO per command). This is acceptable because:
  - the module's `//!` docs and the crate's lib.rs use canon §11.6
    `planned` vocabulary, so no surface advertises behaviour the code
    does not deliver;
  - A2 and A3 land in immediate follow-up chips, so the "log and TODO"
    window is bounded in time.
- The `simple_server.rs` example stays DEMO ONLY until a dedicated
  production composition root exists. The DEMO ONLY comment is canon-sanctioned
  for this transition.
- Per-deployment-mode wiring is still single: Postgres `LISTEN / NOTIFY`
  is an optimisation not lit up in A1. Acceptable because the polling path
  is authoritative; the notify is a wake-up hint only.

Follow-up:

- A2 implements `ControlDispatch::dispatch_resume` /
  `dispatch_restart` (chip A2, closes #332 / #327).
- A3 implements `ControlDispatch::dispatch_cancel` /
  `dispatch_terminate` (chip A3, closes #330).
- A4 adds the knife integration test across producer → consumer → engine
  (chip A4).
- Reclaim path for stuck `Processing` rows — **implemented** (B1 /
  ADR-0017, #482 follow-up). `ControlQueueRepo::reclaim_stuck` + periodic
  sweep in `ControlConsumer` moves abandoned rows back to `Pending` with a
  bounded retry budget before surfacing them as `Failed`.
- `apps/server` (or equivalent) single production composition root —
  tracked separately; this ADR only names the need.

## Alternatives considered

See decision 1 for the three wiring shapes considered (polling / mpsc
channel / per-command spawn) and decision 2 for the surface boundary
alternatives. The key framing choice — putting the consumer in
`nebula-engine` rather than a new `nebula-dispatch` crate — follows from
§12.1 (no new crates without a reason) and the fact that `nebula-engine`
is already canon-named (§12.2) as the consumer location.

## Seam / verification

Seams:

- `crates/engine/src/control_consumer.rs` — `ControlConsumer`,
  `ControlDispatch` trait, `spawn` helper with `CancellationToken`
  shutdown.
- `crates/engine/src/lib.rs` — re-exports; `//!` docs switched to
  §11.6 `planned` vocabulary for the behavioural surface that lands in
  A2 / A3.
- `crates/engine/README.md` — Public API section lists `ControlConsumer`
  / `ControlDispatch` with A1 status note.
- `crates/storage/src/repos/mod.rs` — status table switched from
  "Consumed by the API cancel handler" to "Produced by the API cancel
  handler; consumed by `nebula-engine::ControlConsumer` (skeleton — real
  dispatch lands with ADR-0008 follow-ups A2 / A3)".
- `crates/api/examples/simple_server.rs` — existing DEMO ONLY marker
  kept; comment references ADR-0008.

Tests: `crates/engine/tests/control_consumer_wiring.rs` — construction,
graceful shutdown via `CancellationToken`, and observed-via-trait
assertion (the consumer hands a claimed command to a test
`ControlDispatch` implementation; A1 asserts only that the command is
observed, not that the engine's state changes — that lands with A2 /
A3). A compile-test verifies the consumer's public signatures expose no
`nebula_api::*` or `nebula_storage::rows::*` types.

Related ADRs:

- 0007 (prefixed-ulid-identifiers) — `ExecutionId` shape the
  consumer decodes from the storage entry's UTF-8 bytes.
- A future B1 resume-schema ADR will extend `ControlDispatch` with
  `dispatch_resume`'s resume-cursor argument and land the reclaim path.
