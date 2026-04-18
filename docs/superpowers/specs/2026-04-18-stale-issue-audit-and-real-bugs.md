# Stale-Issue Audit + Real-Bug Backlog — 2026-04-18

**Date:** 2026-04-18
**Author:** Claude (Opus 4.7 1M)
**Authority:** Subordinate to `docs/PRODUCT_CANON.md`. This is an audit + backlog, not a plan.
**Status:** PENDING — awaiting tech-lead priority call on fix ordering.

---

## TL;DR

Two-part audit:

1. **Stale-issue cleanup (done).** 14 stale issues closed this session, 3 already-closed rediscovered. Pattern: squash-merge subjects use `fix(scope): subject (#PR)` instead of `closes #N`, so GitHub auto-close never fires. This is the third batch in ~a week; the root problem is unchanged.
2. **Confirmed-real bug backlog (pending).** 11 HIGH/MEDIUM issues were spot-checked against current code and **remain real**. Grouped below by cost/risk. Awaiting tech-lead ordering.

**Single biggest risk surfaced:** PR [#346](https://github.com/vanyastaff/nebula/pull/346) ("batch 2 execution-state correctness") was **CLOSED without merging**, yet batch 5C's PR body references it as if landed ("added in batch 2 PR #346 for #299"). 5 HIGH/MEDIUM issues (#299, #300, #301, #311, #321) are silently still real behind the belief that they were fixed.

---

## 1. Stale issues closed this session (14)

| # | Fix SHA | Subject |
|---|---------|---------|
| 256 | `7b811372` | fix(engine): credential access denies by default without declaration |
| 297 | `2b551b72` | fix(engine,credential): PR #326 f/u — checkpoint before emit_event |
| 298 | `2b551b72` | fix(engine,credential): PR #326 f/u — rate-limiter returns typed error |
| 307 | `2b551b72` | fix(engine,credential): PR #326 f/u — wall_clock_remaining deadline race |
| 305 | `2df8563c` | fix(runtime): batch 5B — dispatch-rejected counter |
| 308 | `2df8563c` | fix(runtime): batch 5B — StatefulCheckpointSink |
| 317 | `6c12a127` | fix(storage, execution): batch 5C — lease TTL |
| 319 | `abab4f15` | fix(api): batch 4 PR-A — JwtSecret newtype |
| 320 | `abab4f15` | fix(api): batch 4 PR-A — CORS x-api-key |
| 330 | `ef44c076` | fix(api): cancel_execution enqueues durable control signal |
| 334 | `c9db2df0` | fix(storage): transition does not create missing executions |
| 339 | `ec18b1c3` | fix(api/workflow): duplicate-connection test (PR #406) |
| 341 | `4cf44c23` | fix(engine): determine_final_status gates on all_nodes_terminal |
| 342 | `0c137758` | fix(api): list_workflows.count() |
| 343 | `ec18b1c3` | fix(api/workflow): extract_timestamp RFC3339 |

Already closed by a prior pass (rediscovered): #310, #313, #315.

**Pattern observed:** ~15/18 fix commits since 2026-04-14 use `(#N, #N, #N)` in the squash subject without the `closes` keyword. GitHub auto-close never fires. Every ~5 days we do another manual sweep. Worth fixing the root cause (§5 below).

---

## 2. Confirmed-real bugs — grouped by cost

Each bug was spot-checked against current code this session. File:line refs below are verified present.

### Group A — API handler edges (cheap, ~1 PR, ~50 lines) — **FIXED IN THIS PR**

Scope: [`/crates/api/src/handlers/execution.rs`](/crates/api/src/handlers/execution.rs).
The file:line refs below point at `main @ 2b205abf` (pre-fix); the fixes in this PR no longer match those line numbers. Evidence trail kept for future audits.

- **#329 — `get_execution` / `cancel_execution` misparse canonical timestamps.** At `main @ 2b205abf`, `crates/api/src/handlers/execution.rs:181-186` and `:392-397` used `.as_i64()` on RFC3339 strings, silently returning `0`. Fix (this PR): both sites route through `extract_timestamp` (promoted to `pub(crate)` from [`/crates/api/src/handlers/workflow.rs`](/crates/api/src/handlers/workflow.rs), where it already landed via PR #406 for #343). `get_execution` prefers the canonical `completed_at` field (see [`/crates/execution/src/state.rs`](/crates/execution/src/state.rs)) and falls back to legacy `finished_at`; `cancel_execution` prefers `finished_at` (just written by the handler) with the reverse fallback.
- **#331 — `cancel_execution` allows rewriting terminal `timed_out`.** At `main @ 2b205abf`, `execution.rs:290` checked `completed|failed|cancelled` but not `timed_out`. Fix (this PR): added `timed_out` to the guard set.
- **#335 — `cancel_execution` maps CAS conflict to 500.** At `main @ 2b205abf`, `execution.rs:324-326` returned `ApiError::Internal` on `transition_result == false`. Fix (this PR): maps to `ApiError::Conflict` (409).

**Cost:** ~1 hour + tests. Zero architectural risk. Good "warm-up" PR.

### Group B — Tenant-boundary bug (duplicates, 1 repo method + handler swap)

- **#286 / #288 / #328 — `list_executions` ignores `workflow_id` filter.** Three duplicate issues. [`/crates/api/src/handlers/execution.rs`](/crates/api/src/handlers/execution.rs) has a TODO around line 76 (at `main @ 2b205abf`) and still calls `list_running()` globally. Fix: add `ExecutionRepo::list_running_for_workflow(WorkflowId)` with in-memory + Postgres impls, switch handler, backfill integration test.

**Cost:** ~2 hours. Mechanical. Close the two duplicates as `duplicate` when the canonical one is fixed.

**Security note:** issue body flags this as a tenant-crossing info leak the moment real multi-tenant auth lands. Currently contained by the shared-trust-boundary JWT, but it's a latent escalation to HIGH.

### Group C — Resurrect PR #346 (5 bugs, work already done)

**Situation:** PR [#346](https://github.com/vanyastaff/nebula/pull/346) was a "batch 2 execution-state correctness" PR with code + tests for **#299, #300, #301, #311, #321**. It was **closed without merging** (`state: CLOSED, mergedAt: null`). Post-#346, at least one other PR (#386 / `6c12a127`) was authored as if #346 had landed — specifically, batch 5C's body says "the engine path already routes through the repo (added in batch 2 PR #346 for #299)", but at `main @ 2b205abf`, `crates/engine/src/engine.rs:1546` still shows the exact `ActionResult::success(output_value)` reconstruction that #299 describes. Direct link (pinned): [engine.rs#L1546 @ 2b205abf](https://github.com/vanyastaff/nebula/blob/2b205abf/crates/engine/src/engine.rs#L1546).

**What #346 covered:**
- **#321** — setup-failure now calls `checkpoint_node` + emits `NodeFailed` (ordering parity with runtime-failure branch).
- **#300** — `start_node_attempt` typed state-machine helper rejects invalid transitions instead of swallowing with `let _`.
- **#301** — `join_next_with_id` + `HashMap<task::Id, NodeKey>` so panicked nodes report real NodeId.
- **#311** — `ExecutionState.workflow_input` persisted + re-injected on resume.
- **#299** — `ExecutionRepo::save_node_result` / `load_node_result` hooks; preserves Branch/Route/MultiOutput routing across idempotency replay.

**Recommendation:** cherry-pick the PR #346 branch, rebase onto current main, re-run tests. Do NOT re-derive from scratch — this is ~6 weeks of recent context, several of the fixes interlock.

**Risk of doing nothing:** the "phantom fix" belief will keep propagating through other PR bodies. The next deep-review pass will find these again.

**Cost:** ~half a day to resurrect + verify (mostly: rebase conflicts from #412 NodeId→NodeKey rename, which happened after #346 was closed).

### Group D — Architectural, larger scope

- **#279 — `MemoryQueue::dequeue` holds receiver `Mutex` across `tokio::time::timeout`.** [`/crates/runtime/src/queue.rs`](/crates/runtime/src/queue.rs) around lines 195-196 (at `main @ 2b205abf`). Issue suggests swap to `flume` or `async-channel` (multi-consumer, drop-in-ish). Throughput ceiling is `1/timeout` per second — not correctness, but the "N workers" story in runtime design is silently false.
- **#325 — Execution leases exist but are never acquired/renewed/released in engine.** Verified: `acquire_lease` / `renew_lease` are not called anywhere in [`/crates/engine/src/`](/crates/engine/src/). Concurrent runners for the same execution can both execute nodes. HIGH per issue body; relevant for any multi-runner deployment.

**Cost:** #279 is a focused swap + benchmark delta. #325 is genuine lifecycle design (acquire → heartbeat loop → release on shutdown/cancel/error) and needs an ADR-level decision first.

---

## 3. What I recommend

Ship in this order, one PR per group:

1. **Group A** (today) — warm-up, mechanical, catches easy review feedback.
2. **Group C** (tomorrow) — highest value for lowest new effort; stops the "phantom fix" propagation immediately.
3. **Group B** (next) — tenant-boundary correctness.
4. **Group D/#279** — after above land; needs a benchmark before + after to justify the swap.
5. **Group D/#325** — ADR first (lease lifecycle + failure modes + multi-runner semantics), THEN code. Pair with observability so we can see leases in action.

**Cross-cutting — root-cause the stale-issue pattern.** Either:
- Squash-merge template changes to require `Closes #N` when `(#N)` appears in subject, or
- A `scripts/close-linked-issues.sh` hook wired into post-merge CI that scans commit messages for bare `(#N)` refs and closes them with a standard comment.

Either would eliminate the 5-day manual sweep that keeps bringing me back.

---

## 4. Open questions for tech-lead

1. **Scope for this iteration** — all four groups? First two only? One PR per group or bundle A+B into a single "API edges" PR?
2. **Group C (#346) — resurrect or re-derive?** Resurrect is ~4 hours, re-derive is ~2 days. Resurrecting inherits the rebase conflict against #412 (NodeId → NodeKey) plus whatever else shifted since 2026-04-14.
3. **Group D/#325 — who owns the ADR?** This touches engine + storage + observability; not a single-crate call.
4. **Root-cause on stale-issue pattern** — is this worth a dedicated PR now, or park until someone else also burns a sweep on it?

---

## 5. Evidence trail

All SHAs and file:line refs above are from `main` as of 2026-04-18 (HEAD `2b205abf`). Stale-issue closures logged in the `gh issue close` comments on each closed issue — each cites the fix SHA + subject + verification step. Anyone can reproduce by running `git log -S <distinctive_symbol>` on the cited file:line.
