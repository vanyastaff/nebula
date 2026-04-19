---
id: 0017
title: control-queue-reclaim-policy
status: accepted
date: 2026-04-19
supersedes: []
superseded_by: []
tags: [engine, storage, control-queue, reclaim, canon-12.2]
related:
  - crates/engine/src/control_consumer.rs
  - crates/storage/src/repos/control_queue.rs
  - docs/adr/0008-execution-control-queue-consumer.md
  - docs/adr/0015-execution-lease-lifecycle.md
  - docs/adr/0016-engine-cancel-registry.md
  - docs/PRODUCT_CANON.md#122-execution-single-semantic-core-durable-control-plane
---

# 0017. Control-queue reclaim policy

## Context

[ADR-0008](./0008-execution-control-queue-consumer.md) §5 accepted that a
runner crashing after `claim_pending` but before `mark_completed` /
`mark_failed` leaves rows in `Processing` forever. The consumer's happy-path
docs explicitly name B1 as the follow-up that redelivers those rows. A2
(#481) and A3 (#482) landed real dispatch bodies whose idempotency (per
ADR-0016 for cancel, per CAS-on-transition for start/resume/restart) was
designed with B1-driven redelivery in mind.

Three semantic questions are L2 calls, not implementation details:

1. **How old is "stuck"?** A slow dispatch is not a dead runner. Too-eager
   reclaim redelivers work that the original runner is still doing, doubling
   observable side effects even with idempotent dispatch (the command
   executes twice, the ack races). Too-lazy reclaim leaves operators staring
   at stuck rows.
2. **How many retries before we give up?** A genuinely broken dispatch
   (e.g. a bug in `dispatch_restart` that panics the frontier task on every
   invocation) should not loop forever between `Pending` and `Processing`.
   At some point the row is a poison pill and must go to `Failed` so humans
   notice.
3. **Who drives the sweep?** Multi-runner is already the design intent
   (ADR-0015). If every runner sweeps, the sweep itself must be safe under
   contention.

## Decision

**Time-based reclaim with a bounded retry budget, safe under concurrent
runners via CAS on the status transition.** Specifics:

### Staleness window

`reclaim_after = 150s` (5 × lease TTL from ADR-0015). A lease heartbeat is
10s and the TTL is 30s; a runner that has not acked in 150s has missed 15
heartbeats — orders of magnitude past any plausible GC pause. Tunable via
`ControlConsumer::with_reclaim_after`.

Rows with `status = 'Processing'` and `processed_at < NOW() - reclaim_after`
are reclaim candidates.

### Retry budget

`max_reclaim_count = 3`. A new `reclaim_count` column (default 0, bumped on
each reclaim) tracks redeliveries. When a reclaim would push the count past
`max_reclaim_count`, the row is moved to `Failed` with the error message:

> `reclaim exhausted: processor <processor_id> presumed dead after <N> reclaims`

Tunable via `ControlConsumer::with_max_reclaim_count`. The default of 3 is
deliberately small — if three consecutive runners fail to ack the same
dispatch, the command itself is the suspect, not the runners.

### Reclaim sweep cadence

Every runner calls `reclaim_stuck` on a 30s interval from inside
`ControlConsumer::run` (separate `tokio::select!` arm, does not block the
hot claim loop). Multi-runner safety is achieved by the CAS on the
`Processing → Pending` status transition: at most one runner's update
succeeds for any given row; the others observe zero-rows-affected and move
on. **No leader election required.**

### Out of scope

- **Heartbeat-based reclaim.** `processed_at` age is the only signal.
  Heartbeats on control-queue rows would reproduce the lease heartbeat
  lifecycle inside the queue itself — we would be re-implementing
  ADR-0015. A future ADR can revisit if the 150s window proves too lazy
  for a specific deployment.
- **Cross-node `processor_id` liveness detection.** Asking "is the process
  named in `processor_id` still alive?" requires a node-liveness registry
  that does not exist. Time-based reclaim is intentionally dumb.
- **Exponential backoff between reclaim attempts.** The 150s window is
  already long; doubling it per retry would push the third redelivery to
  ~20 minutes of clock time, which hides genuinely stuck rows from
  operators. Fixed window is honest.

## Consequences

Positive:

- The control-queue liveness gap named in ADR-0008 §5 is closed. Operators
  no longer see indefinitely stuck `Processing` rows after a runner crash.
- The 150s staleness window + 3-retry budget gives a concrete, finite
  worst-case delivery latency for any `Cancel` / `Resume` / `Restart` /
  `Start` / `Terminate` command: `3 × 150s = 7.5min` before a genuinely
  broken dispatch surfaces as `Failed`.
- Multi-runner deployments become safe to reason about — every runner's
  sweep is idempotent, so the `apps/server` composition root can start N
  replicas without coordination.

Negative / accepted costs:

- **A successful-but-slow dispatch that exceeds 150s will be redelivered.**
  Existing idempotency contracts (ADR-0008 §5 for terminal-state guarding,
  ADR-0016 for cancel-token idempotence) make this safe but not
  zero-cost — the command body will execute twice. This is acceptable
  because no dispatch body in A2 / A3 takes 150s, and slower dispatches
  should be re-architected rather than tolerated.
- **Moving a row to `Failed` via reclaim-exhaustion is indistinguishable,
  at the `status` column level, from `mark_failed` after a genuine dispatch
  reject.** The `error_message` column disambiguates (prefix
  `"reclaim exhausted:"`) and a counter metric labels the reason, but
  grepping the status column alone does not.
- **Column add on `execution_control_queue` is a non-null column with a
  default.** Safe for Postgres (constant default, fast rewrite-free fill).
  For SQLite it is always backwards-compatible (no parallel writers at
  migration time).

## Alternatives considered

### A. Lease column on control-queue rows (mirror ADR-0015)

**Rejected.** Would require a heartbeat writer per claimed row. The
`execution_control_queue` is a queue, not an execution context — it should
not spawn its own heartbeat tasks. The simpler `processed_at` age is
honest and sufficient.

### B. Infinite retry (no `max_reclaim_count` cap)

**Rejected.** A genuinely broken dispatch (panic, OOM on specific
payload, data-corruption row) would loop between `Pending` and
`Processing` forever, consuming throughput and hiding the bug. The budget
forces a human to notice.

### C. Move straight to `Failed` on first reclaim (no retry)

**Rejected.** A single crashed runner is the common case; operators
restarting a pod should not see the pod's in-flight work marked `Failed`
for them to re-enqueue. At least one retry is essential operational
courtesy.

## Seam / verification

- `crates/storage/src/repos/control_queue.rs` — the `reclaim_stuck` method
  and the `reclaim_count` column-backed field are the storage-layer seam.
  Unit tests cover: (a) expired rows move back to `Pending` with bumped
  `reclaim_count`; (b) fresh rows are untouched; (c) rows past
  `max_reclaim_count` move to `Failed` with the canonical message.
- `crates/engine/src/control_consumer.rs` — the `reclaim_interval` arm in
  `run` is the engine-layer seam. The end-to-end test in
  `crates/engine/tests/control_consumer_wiring.rs` simulates a consumer
  crash mid-dispatch, runs the reclaim sweep, and verifies a fresh
  consumer picks up the redelivered row and drives it to `Completed`.
- Metric: `nebula_engine_control_reclaim_total{outcome="reclaimed|exhausted"}`
  — a non-zero `exhausted` counter crossing zero is a genuine incident
  signal; `reclaimed` climbing steadily is a crashed-runner signal (cross-
  reference with ADR-0015 heartbeat metrics).

## Open questions

None load-bearing for initial implementation. Future refinements:

- **Adaptive staleness window** — scale with observed dispatch latency
  p99? Defer until real production traffic surfaces variance.
- **Postgres `LISTEN / NOTIFY` on reclaim** — could wake sibling runners
  earlier than the 30s tick. Additive optimisation, not a correctness
  issue.
