# ADR-0042: Layered retry — `nebula-resilience` for action-internal, `NodeDefinition.retry_policy` for node-level

**Status:** Accepted
**Date:** 2026-04-29
**Supersedes:** —
**Superseded by:** —
**ROADMAP:** §M2.1
**Issues:** #266 (idempotency), #290 / #296 (retry surface), #311 (workflow_input persistence — already shipped under M0)

## Context

The codebase has carried two retry surfaces in tension:

1. **`nebula-resilience::retry_with`** ([crates/resilience/src/retry.rs](../../crates/resilience/src/retry.rs)) — composable inside an action around outbound calls. Carries `RetryConfig` (Fixed / Linear / Exponential / Fibonacci / Custom backoff) and `Classify`-driven retryability. Mature; widely used.

2. **`ActionResult::Retry { after, reason }`** + **`NodeDefinition.retry_policy: Option<RetryConfig>`** + **`WorkflowConfig.retry_policy`** ([crates/workflow/src/node.rs:32](../../crates/workflow/src/node.rs:32), [crates/workflow/src/definition.rs:123](../../crates/workflow/src/definition.rs:123)) — the engine retry surface. Until M2.1, neither was honored end-to-end:
   - `ActionResult::Retry` was feature-gated behind `unstable-retry-scheduler`; if a handler returned it, the engine threaded it through a synthetic-failure path with the message "Action retry is not supported by the engine".
   - `NodeDefinition.retry_policy` / `WorkflowConfig.retry_policy` were declared, serializable, claimed in public API; engine never read either field.

This created a **§4.5 operational-honesty gap**: the workflow API claimed an operator-level retry policy that did not exist, while the action-level `Retry` variant promised a scheduler that was perpetually "planned".

The 1.0 milestone forces a decision: which retry surface is real, and which is fiction?

## Decision

Two independent retry layers, with disjoint triggers:

### Layer 1 — Action-internal retry (`nebula-resilience::retry_with`)

For in-call failures the action understands and can recover from inside its own invocation:

- Transient HTTP errors (`5xx`, `Retry-After` header)
- Database connection drops, reconnect on retry
- Rate limit cooldowns where the action backs off and retries the call
- Other "the action knows what failed and can retry the same call" cases

This layer **stays in the action's source code**. The action wraps its outbound call with `nebula-resilience::retry_with(...)` (or composes with `bulkhead`, `circuit-breaker`, etc) and returns `ActionResult::Success` after the internal retry succeeds, or an error variant after the internal retry budget is exhausted.

### Layer 2 — Engine-level node retry (`NodeDefinition.retry_policy`)

For whole-action failures that cannot or should not be handled inside the action:

- **Sandbox crash / panic / OOM** — the action's process is dead; only the engine can re-spawn it.
- **Param-resolution failure** — the action never started (e.g. credential lookup failed because credential was rotating). Re-running on next attempt may succeed.
- **Third-party plugin actions** — the workflow author cannot modify the action source to add internal retry. They declare retry at the workflow level instead.
- **Operator-declared policy** — "this critical payment node should retry 3× with exponential backoff, regardless of what the action's author chose to do internally."
- **Long-running batch actions** — re-running the whole action with idempotency tracking is cleaner than wrapping each internal step in `retry_with`.

This layer is **declared by the workflow author** at activation time (`NodeDefinition.retry_policy` for per-node policy, `WorkflowConfig.retry_policy` as workflow-wide default). The engine reads the effective `RetryConfig` on node failure, schedules the next attempt with backoff, persists `next_attempt_at` on `NodeExecutionState`, and re-dispatches the action when the scheduled time arrives. `NodeAttempt` records are pushed to `NodeExecutionState.attempts` per attempt so `ExecutionState::idempotency_key_for_node` differentiates retries (canon §11.3).

### `ActionResult::Retry` is removed

Once the two layers above are wired, `ActionResult::Retry` adds no semantic value:

- If the action knows it should retry, the action can `retry_with` internally (Layer 1) or simply return an error and let the engine's `retry_policy` re-dispatch (Layer 2).
- A handler returning `Retry` would be either redundant with `retry_with` (action-internal) or it would conflate "retry me" with the workflow author's policy declaration (engine-level).

The variant + the `unstable-retry-scheduler` feature flag + the `is_retry()` predicate + the synthetic-failure handler in the engine are all removed in M2.1 T0.

## Trigger boundaries

The two layers are disjoint by **timing**, not by **purpose**, and that is what makes them composable:

- Layer 1 fires *during* the action's own execution. The engine sees only the final outcome.
- Layer 2 fires *after* the engine receives the action's final outcome (`Result::Err` from the spawn) and the workflow has a `retry_policy` that says "try again."

The composition is: `Layer 1` runs first inside the action; if it ultimately fails, the action returns an error; the engine then consults `retry_policy` and may apply `Layer 2`. Both layers can be active on the same node — an action can use `retry_with` inside, *and* the workflow can declare a `retry_policy` for the whole node — without conflict, because the trigger boundary is "did the action's last attempt succeed or fail?" The engine never observes Layer 1's intermediate retries.

## Consequences

### Positive

- **Workflow authors get a real operator-level retry policy** that matches the public API claim. The §4.5 gap closes.
- **Action authors keep a battle-tested in-call retry surface** (`nebula-resilience`) with rich semantics (backoff strategies, `Classify`, hedging).
- **No conflicting retry path** to reason about: the trigger boundary makes the layers compose cleanly.
- **Standard workflow-engine feature** (Airflow `retries`, Temporal `RetryPolicy`, n8n "Retry on Fail") matches operator expectations.
- **Engine-internal retry persists across resume** via `next_attempt_at` on `NodeExecutionState` (M2.1 T2 + T3 schema bump). A resumed engine picks up scheduled retries at their declared time.

### Negative

- **Schema migration cost** — `NodeExecutionState` gains a new field. M0's pattern (forward-compatible JSON deserialization) keeps the migration cheap, but legacy state without the field deserializes as `None`, which the engine treats as "no scheduled retry pending."
- **Frontier-loop scheduling complexity** — engine must wait for `next_attempt_at` while remaining responsive to cancel/terminate signals. M2.1 T5 uses `tokio::select!` between sleep and cancel; cap at `max_concurrent_nodes` keeps this O(n) tractable. A min-heap timer wheel is a 1.1 optimization if profiling shows pressure.
- **`ActionResult::Retry` removal is a public API break** for any out-of-tree consumer that opted into `unstable-retry-scheduler`. Mitigation: the feature was `unstable-retry-scheduler` (default-off) and explicitly documented as `planned` per canon §11.2; consumers should not have been depending on it. Documented in M2.1 T0 commit message.

### Out of scope

- **Per-node retry budget interaction with `ExecutionBudget.max_total_retries`** is intentionally global (sum across all nodes). Workflow-level `retry_policy.max_attempts` remains per-node. The engine consults both; whichever caps first wins. Documented at M2.1 T4 acceptance criteria.
- **Cross-execution retry coordination** (e.g. circuit breakers across workflow runs) — engine retry is per-execution-per-node only. Cross-execution patterns belong in `nebula-resilience` or in a separate observability layer.

## Alternatives considered

### A. Ship `ActionResult::Retry` as the canonical retry surface (engine-driven only)

Rejected. The action-internal layer is genuinely useful for in-call retry semantics that the action knows how to handle. Forcing every retry through the engine round-trip would lose the rich `Classify`/`Retry-After`/`Backoff` strategies inside actions and add unnecessary checkpoint overhead for trivially-recoverable transient failures.

### B. Remove both surfaces; require all retry via `nebula-resilience`

Rejected. Loses the **operator-level retry policy** which is a standard workflow-engine expectation. Workflow authors using third-party plugin actions could not declare retry without editing the plugin source. Standard tools (Airflow, Temporal, n8n) all ship operator-level retry; downgrading from this would be a 1.0 regression vs the public API claim.

### C. Keep `ActionResult::Retry` as a niche signal alongside `retry_policy`

Rejected. Two retry surfaces with overlapping semantics force consumers to learn both and choose between them per case. The trigger boundary that makes Layer 1 + Layer 2 compose disappears once `Retry` is added — `Retry` would conflict with `retry_policy` ("did the action succeed or did it ask to retry?"). Eliminating `Retry` makes the two layers truly disjoint.

## Implementation notes (M2.1 task map)

- **T0** (this PR foundation) — remove `ActionResult::Retry` variant, `unstable-retry-scheduler` feature, `is_retry()` predicate, and the synthetic-failure handler in `engine.rs`.
- **T1** — this ADR.
- **T2 + T3** — extend `NodeExecutionState` with `next_attempt_at`; storage migration mirrors PG + SQLite.
- **T4** — engine reads effective `retry_policy`, computes backoff, decides retry-vs-finalize. Ordering: `[budget-check] → [retry-decision] → schedule | classify+route+checkpoint`.
- **T5** — frontier loop honors `next_attempt_at`, with cancel/terminate/budget guards before and after the sleep.
- **T6** — 9 integration tests covering the core path + edge cases (cancel/terminate/budget interaction, idempotency-key differentiation).
- **T7** — close ROADMAP §M2.1; cross-reference this ADR from action README, workflow README, engine README L4 debt.
- **T8** — `validate_workflow` rejects invalid `RetryConfig` (max_attempts == 0; backoff_multiplier ≤ 0 or non-finite; max_delay_ms < initial_delay_ms; initial_delay_ms == 0 with max_attempts > 1).

## References

- [crates/resilience/src/retry.rs](../../crates/resilience/src/retry.rs) — Layer 1 surface (`retry_with`).
- [crates/workflow/src/node.rs:32](../../crates/workflow/src/node.rs:32), [crates/workflow/src/definition.rs:123](../../crates/workflow/src/definition.rs:123), [crates/workflow/src/definition.rs:171-180](../../crates/workflow/src/definition.rs:171) — Layer 2 surface (`retry_policy`, `RetryConfig`).
- [crates/execution/src/state.rs:89-111](../../crates/execution/src/state.rs:89) — `start_attempt()` already wires `Failed → Retrying → Running` state transitions; T5 reuses, does not reinvent.
- [crates/execution/src/state.rs:386-396](../../crates/execution/src/state.rs:386) — `idempotency_key_for_node` already retry-aware via `attempt_count().max(1)`; T4 makes the helper actively differentiate by pushing `NodeAttempt` per attempt.
- [crates/execution/src/context.rs:61-63](../../crates/execution/src/context.rs:61) — `ExecutionBudget.max_total_retries` (global cap, sum across all nodes).
- canon §11.2 (retry surface), §11.3 (idempotency on retry), §11.5 (durability precedes visibility), §4.5 (operational honesty).
