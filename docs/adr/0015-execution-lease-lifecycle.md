---
id: 0015
title: execution-lease-lifecycle
status: accepted
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [engine, execution, storage, concurrency, multi-runner]
related: [crates/engine/src/engine.rs, crates/storage/src/execution_repo.rs, docs/PRODUCT_CANON.md]
linear:
  - NEB-325
---

# 0015. Execution lease lifecycle

## Context

`ExecutionRepo` has been carrying `acquire_lease` / `renew_lease` /
`release_lease` methods (with both in-memory and Postgres implementations)
since the earliest storage refactor. PR #386 (`6c12a127`, batch 5C) fixed
the in-memory TTL semantics so stale leases actually expire (closed #317).

The methods work. Nothing calls them.

Issue [#325](https://github.com/vanyastaff/nebula/issues/325) surfaced this
in the 2026-04-14 deep-review: `WorkflowEngine::execute_workflow` and
`resume_execution` both run without touching the lease primitives. Two
runners that pick up the same `ExecutionId` — whether by scheduler race,
operator-issued retry, or a restart of a worker that thought the first
instance was dead — will both run the frontier loop, dispatch the same
nodes, and invoke side effects twice. The CAS on `ExecutionState.version`
catches some writes but does not fence action dispatch: the HTTP call,
the database write, the credit capture, the email send all fire in both
runners before either loses the version race.

This is a real multi-runner correctness gap — not a latent one. It is
contained only because the current deployment story is single-runner;
the moment a second worker process comes up for redundancy or horizontal
scaling, side effects double. The canon §12.2 durability story and the
§13 knife scenario both implicitly depend on exactly-one-runner-per-execution;
the lease is the mechanism that makes that implicit contract explicit.

A separate pressure: **fencing stale writers**. The 2026-04-16 workspace
health audit (`docs/superpowers/specs/2026-04-16-workspace-health-audit.md`
§2.4) flagged that the credential allowlist and several cross-component
writes assumed single-runner semantics without enforcing them. Closing
that gap requires a coordination primitive the engine actually honors.

## Decision

**The engine acquires a lease at the start of every `execute_workflow` and
`resume_execution` call, renews it on a heartbeat while the frontier loop
runs, and releases it on every terminal path (success, failure, cancel,
panic catchall).** Lease ownership is the authoritative "who runs this
execution right now" signal. No other engine instance may dispatch nodes
for an execution it does not hold the lease on.

### Holder identity

The lease holder is a stable per-engine-instance string, format:

```
engine_<ulid>
```

constructed once at `WorkflowEngine::new` via a monotonic ULID generator
and logged at startup alongside the engine's `config_version`. A single
process runs exactly one holder string for its lifetime; restarts rotate
the string so a post-restart runner cannot accidentally "inherit" a lease
from its previous incarnation (which is the point — the previous incarnation
may still be finishing disk flushes).

### TTL and heartbeat

- **TTL:** 30 seconds. Long enough to survive a GC pause or a slow
  checkpoint write; short enough that a crashed runner's lease expires
  inside a minute and redelivery doesn't feel stuck.
- **Heartbeat:** every 10 seconds (TTL / 3). The frontier loop spawns a
  heartbeat task at the start of `execute_workflow` and `cancel_token`s
  it at the end. Heartbeat calls `renew_lease(id, &holder, ttl)`.
- **Heartbeat failure** (renew returns `Ok(false)` — stolen or expired):
  the engine **aborts the current dispatch and does NOT persist further
  state**. This is a §12.2 invariant: a stale writer producing checkpoint
  entries would corrupt the canonical state another runner is now driving.
  The cancel_token is tripped; in-flight `NodeTask`s observe cancellation
  and exit. The final `determine_final_status` is skipped and no
  `ExecutionFinished` event is emitted — the active lease holder emits
  that.

### Contention

- **`execute_workflow` on already-leased execution:** returns
  `EngineError::Leased { holder: String }`. The API handler routes this
  to `ApiError::Conflict (409)` so clients can back off. The scheduler
  (when it exists) treats it as "not mine" and moves on.
- **`resume_execution` on already-leased execution:** same — 409 at the
  HTTP edge, "not mine" at the scheduler. Resume is always explicit, so
  the cleanup of a stale runner takes precedence over a manual resume
  race.
- **Same-holder re-acquire:** if `acquire_lease` returns `Ok(false)` and
  the existing holder string matches the current engine's holder, treat
  it as idempotent success (pre-crash-restart of the same instance within
  the TTL window). Unlikely in practice but worth handling cleanly.

### Release

- **Normal completion (Completed/Failed/Cancelled):** `release_lease(id,
  &holder)` runs in the same tail block that records `ExecutionFinished`.
- **Panic escape from the frontier task:** the engine's top-level
  `catch_unwind` or task-tracker shim calls `release_lease` before the
  task exits. If that path is skipped (e.g., `std::process::abort()`),
  the TTL expires and the lease becomes acquirable after 30 s.
- **`ExecutionState::transition_to(terminal)` regressions:** releasing on
  terminal status rather than on scope exit is tempting but wrong — the
  checkpoint that persists the terminal state MUST be written under the
  lease. Release only after the final persist succeeds or is known failed.

## Consequences

Positive:

- **Exactly-one-runner-per-execution becomes enforceable.** The canon
  §12.2 and §13 story is no longer implicit.
- **Stale-writer fencing.** A partitioned-off runner's heartbeat fails
  within 30 s and it self-aborts instead of producing corrupt checkpoints.
- **Operator visibility.** The lease holder string surfaces in error
  responses and logs — "which box is running execution X right now" is
  answerable without a dashboard.
- **Unblocks horizontal engine scale-out.** Second and third engine
  instances become safe to add without duplicate side-effect risk.

Negative / accepted costs:

- **Heartbeat adds ~2 storage writes per minute per active execution.**
  On the current deployment scale this is negligible; at 10k concurrent
  executions it's 333 writes/s sustained, within Postgres budget.
- **30 s redelivery latency** after a hard crash. A stuck execution does
  not resume for up to TTL. Tuning TTL down to 10 s / heartbeat 3 s is
  possible if redelivery latency becomes load-bearing; the trade is more
  heartbeat writes.
- **New error path in API: 409 on leased.** Clients must handle it with
  exponential backoff — a retry loop is the natural response, and the
  `Retry-After` header should include a TTL-sized hint.
- **Test fixtures must mock the lease cleanly.** The in-memory repo
  handles acquire/renew/release via `tokio::time::Instant`, so
  `start_paused = true` tests already work deterministically per batch
  5C. No new fixture infrastructure required.

Follow-up work this enables:

- The "execution scheduler" concept (picking next work) can now be
  implemented as a simple pull loop: list running → try acquire → if
  acquired, dispatch, else skip. No queue primitive needed for MVP.
- Replaces the hypothetical "locking" story sometimes considered for
  the credential refresh coordinator — lease covers it.

## Alternatives considered

### A. Process-level advisory lock (pg_advisory_lock on execution_id hash)

**Rejected.** Works only for Postgres; breaks the in-memory backend
parity contract (see audit §2.3 on storage two-truths). Also couples
the correctness story to a specific backend primitive rather than an
application-level invariant.

### B. Distributed lock manager (etcd, Redis, ZooKeeper)

**Rejected.** Introduces a new infrastructure dependency at a layer
below the engine. Nebula's canon §11 commits to "no framework without
a product use for it". The existing storage-layer lease primitive
already provides atomic acquire + TTL — adding a separate coordinator
doubles the moving parts.

### C. CAS-only, no lease

**Rejected.** CAS catches lost updates on `ExecutionState.version` but
does not fence action dispatch. Two runners would both invoke `send
email` / `POST /stripe` before either loses the version race. The
side-effect doubling is the actual failure mode #325 describes, not
the write race.

### D. Lease only on resume, not on execute

**Rejected.** Initial-start races are rarer but not zero — a scheduler
that double-dispatches at startup, or an operator that issues
`start_execution` twice in quick succession, hits the same failure
mode. Consistency between execute and resume also keeps the engine's
public shape simple.

## Seam / verification

The lease invariant lives at these seams:

- [`crates/engine/src/engine.rs`](crates/engine/src/engine.rs) —
  `WorkflowEngine::execute_workflow` and `resume_execution` both enter
  and exit the lease scope. A regression test
  (`engine_fences_second_runner_via_lease`) spawns two tokio tasks that
  both invoke `execute_workflow` on the same `ExecutionId`; asserts
  exactly one dispatches, the other returns `EngineError::Leased`.
- [`crates/storage/src/execution_repo.rs`](crates/storage/src/execution_repo.rs)
  — the lease contract (TTL-respecting, holder-validating) is the
  storage-layer guarantee the engine depends on. The
  `transition_unknown_execution_returns_false_without_creating_row`
  precedent (from #334 / `c9db2df0`) is the pattern for locking down
  backend-parity contracts with a regression test.
- Metric: new counter `NEBULA_ENGINE_LEASE_CONTENTION_TOTAL` with a
  `reason` label (`already_held`, `heartbeat_lost`) so multi-runner
  races are observable in Grafana. `reason=heartbeat_lost` crossing
  zero is a genuine incident signal.

## Open questions

None load-bearing for initial implementation. Future refinements:

- **Adaptive TTL** — should TTL scale with estimated execution duration
  (pin longer for known long-running workflows)? Defer until we have a
  baseline distribution of execution length.
- **Scheduler integration** — the scheduler that picks next work from
  the pool isn't built yet; this ADR assumes it exists as a pull loop
  over `list_running`. When it lands, its own ADR can cross-reference
  this one.
