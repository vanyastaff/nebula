---
# budget-justified: ADR prose document — one contiguous decision record (the nebula-resource teardown contract reset/destroy + author-foolproofing ladder + canon §11.4 revision), not decomposable code
id: 0093
title: resource-teardown-contract-and-foolproofing
status: accepted
date: 2026-06-12
supersedes: []
amends: []
superseded_by: []
tags: [resource, teardown, release, destroy, reset, lifecycle, foolproofing, cancellation, deadline, dx, canon]
related:
  - docs/superpowers/specs/2026-06-11-resource-case-fit-matrix.md  # the 22-case matrix that forced A4/A5/A6
  - docs/PRODUCT_CANON.md  # §11.4 async-release best-effort — REVISED by this ADR
  - crates/resource/README.md  # lifecycle + best-effort release contract
---

# 0093. Resource teardown is real async work — the reset/destroy contract and the author-foolproofing ladder

## Status

**Accepted** (2026-06-12, owner-directed, design dialogue).

This ADR sets the teardown contract for `nebula-resource`: how a leased
resource's session state is reset for reuse, how an instance is destroyed,
how those operations are bounded and isolated against a careless or hostile
author, and how the timeout/cancellation budget is owned. It extends the
resource runtime / SlotCell finalization (ADR-0067, landed on `main`) by
replacing the "release is cheap Drop-glue" assumption with "release/destroy are
fallible, async, ordered, deadline-bound work," and it **revises Product Canon
§11.4** (see Consequences).

It does **not** re-open the topology open-vs-closed decision (closed +
framework-owned stands) nor the bind-inversion (the framework owns the acquire
loop and the revoke fence — the structural foundation this contract builds on).

## Context

### The gap

The shipped model treated `release`/`destroy`/`check` as cheap, synchronous,
`Drop`-shaped. The owner's 22-case fit matrix
(`docs/superpowers/specs/2026-06-11-resource-case-fit-matrix.md`) established
that **16 of 22 enumerated resources need teardown to be fallible, async,
ordered, and deadline-bound**, and ranked three contract additions as
MUST-1.0:

- **A4** — per-acquire session-init **and a symmetric pre-release reset**
  (7 cases). Session state (`SET`/PRAGMA/cwd/txn/prepared statements) leaks
  across actions and tenants without a reset on the release path. A
  correctness/security bug, not a nicety.
- **A5** — fallible async release/reset with ordering (reset-before-re-issue;
  reset can fail → evict; panic-in-teardown → poison → evict) — not `Drop`
  glue (7 cases). Applies to **Pooled too** (a dirty connection must not be
  re-pooled), not just exclusive caps.
- **A6** — graceful async `destroy` with a deadline (flush/drain/close ordered
  before the instance is dropped) (6+ cases). `Drop` is insufficient: in-flight
  batches/streams are lost; cost/safety leaks for GPU/VM/object-store handles.

### Ecosystem evidence (primary-source, 2026-06-12)

Three research sweeps grounded the API shape:

1. **Connection pools** (deadpool, bb8, r2d2, sqlx, HikariCP). **Zero**
   libraries pass a `Duration`/deadline **into** the teardown trait method;
   all keep the budget in config and wrap with an external
   `tokio::time::timeout`. The documented rationale: trait method signatures
   must be stable, and a timeout is operational policy, not protocol. sqlx's
   `after_release(conn, meta) -> Result<bool>` is the canonical
   reset-for-reuse hook — a **tri-state** result (`Ok(true)` re-pool /
   `Ok(false)` close-clean / `Err` close-with-error) richer than deadpool's
   binary recycle. deadpool-postgres `RecyclingMethod::Clean` runs the
   `DISCARD ALL` family. HikariCP uses a hard close timeout, never interrupts
   an in-use connection, and treats leak detection as warn-only. Lease metadata
   (age, idle-for, use-count) passed into teardown is universally useful for a
   reset-vs-replace decision.

2. **Cancellation / graceful shutdown** (tokio `CancellationToken` +
   `TaskTracker`, hyper/axum graceful shutdown, Oxide RFD-0400, the
   deadline-composition argument). Preemptive cancellation (dropping a future,
   `abort()`) drops at the next await point with no cleanup; cooperative
   cancellation (a token the future watches) allows a clean flush. The blessed
   pattern is **cooperative signal + hard-timeout backstop**. `Duration`
   per-step timeouts **do not compose** (each step gets a fresh budget, so the
   total exceeds the intended limit); **`Instant` deadlines compose**
   (`timeout_at(deadline)` honored to one epoch across every step). There is no
   async `Drop` in stable Rust (edition 2024) — teardown must be an explicit
   awaited method or scheduled work, never `Drop`.

3. **Specialized resources** (Kafka `flush(timeout)`, Docker
   `stop_container{t}` = SIGTERM→grace→SIGKILL, k8s `terminationGracePeriod`,
   S3 `AbortMultipartUpload`, wgpu `poll(Wait)`→`destroy`, etcd/Redis lease
   revoke). The **graceful-then-hard-kill-with-deadline** pattern recurs
   independently; the deadline is owned by the **framework/caller**, not the
   resource. Most teardowns need only the framework backstop; cooperative
   cancellation mid-flight is genuinely required only for long-running compute,
   server streams, and drain loops (GPU/gRPC/NATS) — which the matrix itself
   defers to 1.1 / the engine-daemon tier.

## Decision

### 1. Teardown is two operations, both fallible-async, both with safe defaults

```rust
// RESET — Pooled recycle path only. Wipe per-lease/session state and decide
// whether the instance is safe to re-pool. Runs BEFORE return-to-store.
// Analog of sqlx `after_release` (tri-state).
async fn reset(&self, instance: &mut Self::Instance, meta: LeaseMeta)
    -> Result<ResetDecision, Error>
{
    // Safe by default: recycle ONLY when the resource holds no session state.
    if Self::holds_session_state() { Ok(ResetDecision::Discard) }
    else { Ok(ResetDecision::Recycle) }
}

enum ResetDecision { Recycle, Discard }
// Ok(Recycle) -> return to pool; Ok(Discard) -> evict cleanly (rotation, no noise);
// Err -> evict + log.

// DESTROY — final teardown. Flush/drain/close ordered BEFORE the instance is
// dropped. Default Ok(()) (Drop suffices for fd/flock/in-process values).
async fn destroy(&self, instance: Self::Instance, cx: TeardownCx)
    -> Result<(), Error> { Ok(()) }

#[non_exhaustive]
struct TeardownCx {
    deadline: Instant,        // read-only; the framework budget
    reason: TeardownReason,   // Released | Evicted | Revoked | Shutdown
    // 1.1: cancel: CancellationToken — cooperative signal for streaming/compute
}
```

- `LeaseMeta` carries `age` / `idle_for` / `use_count` so the author can decide
  reset-vs-discard without the framework owning that policy.
- `reset` is dispatched only on the Pooled recycle path; Resident/Exclusive
  topologies never call it (nothing is "returned to a pool").

### 2. The budget is framework-owned; the author sees a deadline, never a Duration

The teardown deadline is **composed by the framework** from two axes and handed
to the author as a read-only `Instant`:

```
deadline = now + min(resource_declared_need, context_budget)
```

- **resource_declared_need** — a property of the resource type (its config /
  metadata): "my destroy may need up to N seconds to flush."
- **context_budget** — a property of the operation, from policy:
  `Shutdown` = long drain, `Revoke` = short, `Evict` = medium. Surfaced to the
  author as `cx.reason` so the author can adapt (flush on `Shutdown`, skip on
  `Revoke`).

The framework **always** wraps teardown in `timeout_at(cx.deadline, …)` as a
hard backstop. An author who wants graceful behavior reads `cx.deadline` and
bounds their own flush to the **same** epoch (composing); an author who ignores
it is still bounded by the backstop. We **reject** putting a `Duration` in the
method signature (ecosystem-unanimous against it; it does not compose; it is a
half-solution that still needs config for the second axis).

Cooperative `CancellationToken` is **deferred to 1.1**, scoped to the
streaming/compute family the matrix already defers. `TeardownCx` is
`#[non_exhaustive]`, so adding `cancel` later is non-breaking.

### 3. The foolproofing ladder — defense by descending strength

The governing principle: **the lazy or wrong default must be SAFE, not leaky.**
(The notable ecosystem footgun is sqlx defaulting to re-pool without reset.)

**Tier 1 — impossible by construction**
- `reset`/`destroy` receive only `&mut Self::Instance` / `Self::Instance` —
  never `&store`, the registry, or sibling slots. An author cannot reach
  another tenant's instance, re-implement the revoke fence, or cache below the
  `SlotIdentity` barrier. (Inherited from the bind-inversion.)
- **Safe-by-default `reset`:** a credentialed/stateful resource defaults to
  `Discard`, not `Recycle`. Forgetting to wipe a dirty connection makes the
  pool **recreate** the instance, never **leak** tenant A's session state to
  tenant B. Recycling a stateful instance requires the author to **explicitly**
  write `reset` (wipe → return `Recycle`). The unsafe path requires deliberate
  action; safety is the default. `holds_session_state()` defaults to
  `!credential_slots().is_empty()`.
- `TeardownCx` fields are read-only — the author cannot extend the deadline or
  disarm the backstop.

**Tier 2 — framework-contained (author writes it wrong; the framework catches)**
- Hang → `timeout_at(cx.deadline)` drops the future; the worker is freed.
- Panic → `catch_unwind` → typed error + **poison-evict**: a panicked or
  half-reset instance is **never** returned to the pool, only destroyed.
- Post-reset corruption (author returns `Recycle` on a broken instance) → the
  next checkout's `prepare`/health gate evicts it; a dirty instance is never
  served.
- A revoked-mid-lease credential → the revoke-epoch fence evicts the stale slot
  on return regardless of the `ResetDecision`.

All author-hook execution funnels through the single
`hook_guard::guard_author_hook` chokepoint so bound+isolate is structural, not
per-site discipline.

**Tier 3 — loud in dev**
- Registration footgun-guard (mirrors the shared-topology revoke guard): a
  credentialed Pooled resource still on the default `reset` emits one
  `tracing::warn!` + `debug_assert!` — "this resource discards on every
  release; override `reset()` to pool safely." The performance cost of the safe
  default is **loud, not silent**.

**Tier 4 — observable in prod**
- `dropped_count` counts worker-path teardown timeout/panic (the leak metric is
  honest — the B12 fix on this branch).
- Structured fault log: `hook.fault = panicked | timed_out`,
  `hook.site = reset | destroy`.
- Recycle/Discard ratio metric: 100% discard means pooling is silently disabled
  by a missing `reset` → warn.
- `ResourceEvent` evict/poison variants.

### 4. The honest gap — sync-blocking authors

A `std::thread::sleep` / blocking syscall inside `reset`/`destroy` is **not**
caught by `timeout_at`: the future never yields, so the executor never polls the
timeout, and the worker thread is pinned. This gap exists in the current
foolproofing too. The contract's stance:

- The `ReleaseQueue` is multi-worker, so the blast radius is one worker, not the
  whole queue.
- A dev-mode block detector (`debug_assert` via tokio block-in-place detection).
- Hard documentation that teardown must not block the runtime.
- A **`spawn_blocking` isolation mode** offered as opt-in config for untrusted
  community plugins (not the default — `spawn_blocking` per teardown is too
  costly to impose universally).

## Consequences

### Product Canon §11.4 is revised

§11.4 currently reads "async release is best-effort on crash." This stays true
**for the crash case** (the process dies → `ReleaseQueue` cannot run → the next
process drains via the durable path). But for **normal** release it is no longer
the whole story: normal teardown is now **awaited, deadline-bounded, fallible,
and ordered**. The canon must distinguish two contracts:

- **Normal teardown** — `reset`/`destroy` run to completion or a typed
  deadline error, with the drain accounted and the slot evicted-or-recycled
  deterministically.
- **Crash teardown** — best-effort; relies on the next process / server-side
  TTL. Authors still must not assume "release ran" without a durable
  checkpoint.

The canon edit is a separate change tracked against this ADR.

### Phasing

- **Non-breaking first** (lands incrementally): the framework deadline backstop
  sourced from config/policy on the *existing* `destroy`; the safe-by-default
  `reset` hook (new, defaulted, so no existing impl breaks); the Tier-3
  registration guard; the Tier-4 observability. The B12 `dropped_count` honesty
  fix already landed.
- **Breaking sweep** (own `feat(resource)!` commit, mechanical): `destroy`
  gains `cx: TeardownCx`; `reset` gains the tri-state veto wired into the
  Pooled recycle path. Every impl in engine/examples/tests updates.

### What this does NOT cover

- Streaming handles (acquire-yields-a-stream) — deferred to 1.1 / engine-daemon
  (matrix A9). This contract does not force them into `reset`/`destroy`.
- Cooperative cancellation mid-flight — deferred to 1.1 (matrix-aligned).
- Renew-while-held / fencing for the lock-lease family (matrix A7) — separate.

## Alternatives considered

- **`destroy(timeout: Duration)` in the trait signature** — rejected.
  Ecosystem-unanimous against it (zero pool libs do it); `Duration` does not
  compose across multi-step teardown; it still needs config for the
  framework-budget axis, so it is a half-solution that also breaks every impl.
- **One unified `release()` operation** — rejected. reset (instance survives,
  state wiped, re-pool decision) and destroy (instance gone, flush before drop)
  are semantically distinct; sqlx/deadpool/pgbouncer all separate them. Merging
  them forces a stateful resource to choose between "can't reset" and
  "can't express final flush."
- **Default `reset` = `Discard` for all resources** — rejected. Safe but
  disables pooling for the common stateless case (a Postgres pool wants
  recycle). The `holds_session_state()`-gated default keeps stateless recycle
  fast while making stateful recycle opt-in.
- **Cooperative `CancellationToken` in 1.0** — deferred. Genuinely needed only
  by the streaming/compute family, which the matrix defers; shipping it now
  would guess the `TeardownCx` shape before a real consumer exists.
- **`spawn_blocking` every teardown to neutralize blocking authors** — rejected
  as the default (cost); offered as opt-in for untrusted plugins.

## Implementation notes

- Author-hook execution stays funneled through `hook_guard::guard_author_hook`
  (bound + isolate); this ADR's deadline replaces the global
  `TASK_EXECUTION_TIMEOUT` constant as the budget source.
- `reset` dispatch lives on the Pooled recycle path (the release-entry in
  `runtime/managed.rs`); Resident/Exclusive do not call it.
- The reset attestation for the Tier-3 `debug_assert` is a "hook was invoked"
  flag, not a content check — the framework cannot verify what "reset" means
  for a given resource.
