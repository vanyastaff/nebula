---
id: 0016
title: engine-cancel-registry
status: accepted
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [engine, execution, control-plane, cancellation]
related:
  - crates/engine/src/engine.rs
  - crates/engine/src/control_dispatch.rs
  - crates/engine/src/control_consumer.rs
  - docs/adr/0008-execution-control-queue-consumer.md
  - docs/PRODUCT_CANON.md#122-execution-single-semantic-core-durable-control-plane
linear:
  - NEB-330
---

# 0016. Engine cancel registry

## Context

[ADR-0008](./0008-execution-control-queue-consumer.md) landed the durable
control-queue consumer. Its follow-up A2 wired `Start` / `Resume` / `Restart`
into `EngineControlDispatch` (#481, #332 / #327). The A3 chip in the same
follow-up stack closed the symmetric cancel half (#330).

A3's core problem: `WorkflowEngine::execute_workflow` and `resume_execution`
create a per-run `tokio_util::sync::CancellationToken` at the top of the
frontier scope. The token is observed inside `run_frontier` (the select-arm
at `cancel_token.cancelled()` breaks the loop; the per-node `JoinSet` is
aborted). **The token is not exposed outside the function scope.** Until
A3, a durable `Cancel` delivered via the control queue could be read and
acked, but it had no way to reach the live frontier loop — the API's CAS
transition to `Cancelled` landed on the row, the Cancel signal was
enqueued, the consumer claimed it, and then… nothing. The slow handler kept
sleeping. The durable state said the run was over; the in-process
`JoinSet` said otherwise. That is exactly the two-truth gap canon §14 calls
out.

Three design pressures made this an L2 rather than a hidden
implementation detail:

1. **Canon §12.2 contract.** Every `Cancel` signal must be engine-consumable,
   not just stored. A discard-and-log consumer is broken by construction. The
   A1 skeleton landed with that exact pattern, gated behind explicit
   `planned` markers and a `ControlDispatchError::Rejected` so the row
   would end up `Failed` rather than silently acked. A3 has to remove the
   `planned` marker, which means the engine must expose a real cooperative-
   cancel seam.

2. **Cross-runner honesty.** A durable control-queue row can be drained by
   any runner that shares the `ControlQueueRepo`; the runner that drains
   the Cancel is not necessarily the runner holding the frontier loop. The
   dispatch must not fence a `Cancel` as a failure when the holding runner
   is a sibling, yet it also cannot silently no-op — ADR-0008 §5 requires
   the dispatch path to have acted on every non-orphan delivery.

3. **Terminate has no separate engine path.** ADR-0008 names `Cancel`
   (cooperative drain) and `Terminate` (forced termination) as two commands
   with distinct semantics. The engine has one cancel path today. Either
   A3 ships half-implemented forced-abort machinery (violates canon §4.5
   operational honesty) or it documents the gap and treats `Terminate` as
   a `Cancel` synonym for now.

## Decision

**`WorkflowEngine` owns a per-instance volatile index of in-flight
`CancellationToken`s, keyed by `ExecutionId`. `EngineControlDispatch`
signals that index on every non-orphan `Cancel` / `Terminate` delivery.
Durable truth remains in `executions` + `execution_control_queue`; the
registry is a runtime convenience, not a source of truth.**

### The cancel registry

```rust
pub struct WorkflowEngine {
    // … existing fields …
    running: Arc<DashMap<ExecutionId, CancellationToken>>,
}
```

- `execute_workflow` and `resume_execution` insert the execution's
  `CancellationToken` right after it is minted, before lease acquisition.
- An RAII `RunningRegistration` guard removes the entry on every exit path
  — normal completion, heartbeat-lost `EngineError::Leased` early-return,
  final-persist errors, panic unwind. This was load-bearing: without the
  guard, a panicking frontier loop would leave a stale token visible to
  future Cancel signals, leading to a double-cancel when the same
  `ExecutionId` is reused (post-restart recovery). `replay_execution`
  does **not** register — it manufactures a fresh `ExecutionId` each call
  and is a direct-Rust-API entry point the control queue cannot target.
- `WorkflowEngine::cancel_execution(id) -> bool` looks up the token and
  calls `.cancel()`. Returns `true` if this instance held the entry,
  `false` otherwise. A `false` return is not an error — it is the honest
  cross-runner answer.

### `EngineControlDispatch` dispatch bodies

```rust
dispatch_cancel:
  match read_status:
    None                    -> Rejected("orphaned")
    Some(status)            -> engine.cancel_execution(id); Ok(())

dispatch_terminate:
  self.dispatch_cancel(id).await
```

Two points deserve emphasis:

- **No status short-circuit on terminal.** Unlike
  `dispatch_start` / `dispatch_resume` / `dispatch_restart`, Cancel does
  not gate on `ExecutionStatus::is_terminal()`. The API's
  `cancel_execution` handler CAS-transitions the execution row to
  `Cancelled` in the same logical operation as the enqueue (canon §12.2 /
  §13 step 5 wire order), so by the time the consumer drains the Cancel,
  the read here typically reports a terminal status even for a live
  frontier loop. A short-circuit would leave the `JoinSet` orphaned. The
  signal is idempotent per token, so signalling always is safe under
  at-least-once redelivery.

- **`Terminate` is a cooperative-cancel synonym today.** ADR-0008 names
  `Terminate` "forced termination", but the engine has no distinct
  forced-shutdown path (process-level kill, `JoinSet::abort_all` without
  the frontier-loop wrapper, etc.). Cooperative cancel via the same token
  is the honest A3 minimum; a future chip may split the paths if product
  pressure demands it. The distinction is documented in the trait's
  per-method docstrings and this ADR — callers cannot rely on `Terminate`
  terminating faster than `Cancel` today.

### The `Running → Cancelling → Cancelled` bridge

A side effect of A3 surfacing the live cooperative-cancel path is that the
engine's final-state transition from `Running → Cancelled` becomes
reachable from outside the frontier loop. The state machine (see
`nebula-execution::transition`, #273) does not carve
`Running → Cancelled` as a one-step transition — it only offers
`Running → Cancelling → Cancelled`. Before A3, the
`let _ = exec_state.transition_status(Cancelled)` in each engine tail
silently failed on that invalid transition, leaving `exec_state.status =
Running`; the persisted row then carried `Running` while the
`ExecutionResult` returned `Cancelled`. That was a two-truth violation
hidden by lack of observability — no external caller could trip the
Running → Cancelled transition before A3.

A3 bridges this in the engine tails (`execute_workflow`, `resume_execution`,
`replay_execution`) with a minimal two-step:

```rust
if final_status == Cancelled && exec_state.status == Running {
    let _ = exec_state.transition_status(Cancelling);
}
let _ = exec_state.transition_status(final_status);
```

No canon state-machine change — the existing `Running → Cancelling →
Cancelled` edges already exist for the API's pending future contract.
Adding `Running → Cancelled` as a one-step would require an ADR
superseding #273's "no phantom bridges" rationale, and the bridge is
cheaper than that debate.

## Consequences

### Positive

- `execution_control_queue` honours its §12.2 contract for every command
  kind. No `planned` markers remain on the A2/A3 chip stack.
- `knife_step5_engine_cancels_running_execution_end_to_end` asserts the
  full producer → consumer → engine path: a 30-second slow handler exits
  within a few seconds of `POST /cancel`. Without A3 the test would time
  out.
- `dispatch_cancel` is idempotent under at-least-once redelivery because
  the underlying `CancellationToken::cancel` is idempotent per token and
  the registry lookup is a no-op on a missing entry. No short-circuit
  needed.
- Cross-runner delivery is safe: the runner that drains the Cancel may or
  may not hold the frontier loop; either way the dispatch returns `Ok(())`
  and the durable state carries the truth.

### Negative

- The registry is volatile. On process crash the entries vanish with the
  runner; the replacement runner has to reload from storage — which is
  the correct durability contract, but it means A3 does not solve "cancel
  reached a crashed runner." That is the B1 reclaim path's job
  (separate chip; stuck-Processing-row recovery).
- `Terminate` does not have distinct semantics from `Cancel` today. An
  operator who issues `Terminate` hoping for a process-level abort gets
  cooperative cancel. The docstrings spell this out, but a user who
  doesn't read them could be surprised. The cost of splitting paths is
  a new `JoinSet::abort_all()` tail-control hook plus a canon §4.5 review;
  neither is justified until the product actually needs the distinction.
- One additional `Arc<DashMap<_, _>>` field on `WorkflowEngine`. Memory
  cost is one `CancellationToken` per live execution per runner — the
  same order of magnitude as the existing lease heartbeat task. No
  measurable allocator pressure.

### Neutral

- The `Running → Cancelling → Cancelled` bridge is a two-line block in
  three tails. A helper method could factor it, but each tail already
  manages slightly different preconditions (lease state, persist
  reconciliation); duplicating the bridge is clearer than introducing
  a narrow helper with two call sites' worth of context.

## Alternatives considered

### Surface the token via a method on `WorkflowEngine` that returns it

```rust
pub fn cancel_token(&self, id: ExecutionId) -> Option<CancellationToken>
```

Gives the caller a clone of the token, not a cancel side effect. Rejected
because it forces `EngineControlDispatch` to duplicate the null-token and
registry-lookup logic and does not carry the "was this runner holding it"
signal that `bool` does. The signal-and-report shape is the right surface.

### Store the token inside `ExecutionState`

Would put the `CancellationToken` on the same structure that goes through
`ExecutionRepo`. Rejected: `ExecutionState` is serialized across the
storage boundary; `CancellationToken` is runtime-only. Conflating them
would violate canon §11.1 (storage is authoritative) and canon §14
(no phantom types that don't serialize).

### Treat `Terminate` as a hard abort via `JoinSet::abort_all`

Would call `handle.abort()` on the frontier-loop task. Rejected for A3:
the frontier loop owns lease release, final-state persistence, and the
journal append. An abort would skip all three, producing a stale lease
and an unpersisted terminal state — worse than not shipping the
distinction. A real forced-shutdown path has to cooperate with those
responsibilities, which is a larger design and a separate ADR.

### Extend the state machine to allow `Running → Cancelled` one-step

Would skip the bridge. Rejected because #273 explicitly carved the
"no phantom Running bridge" rationale for `Created → Cancelled`; adding
`Running → Cancelled` requires a canon-level discussion about whether
`Cancelling` is a legitimate observable state (it is, for the API's
pending Cancelling-first refactor). The bridge is local; the state machine
stays conservative.

## Follow-ups

- **Forced-shutdown `Terminate` path** (future chip) — if the product
  needs a distinct `Terminate` that bypasses cooperative cancel, split
  the path. Requires lease / persist / journal cooperation. Not currently
  on the roadmap.
- **API handler: write `Cancelling` instead of `Cancelled` first** — the
  current `cancel_execution` handler hand-edits the row JSON to set
  `status = cancelled` directly, bypassing the state machine (see
  `crates/api/src/handlers/execution.rs`). This predates A3 and is
  orthogonal, but a cleaner flow would be API-writes-`Cancelling`,
  engine-drives-`Cancelling → Cancelled` via the cancel dispatch. Then
  the bridge becomes unnecessary. Tracked separately.
- **Reclaim path for stuck `Processing` rows** (B1) — Cancels delivered to
  a crashed runner still need a recovery path. That is ADR-0008 §5's
  stated follow-up and lands with the B-chip stack.
