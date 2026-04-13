# Poll trigger — quick hardening bundle

**Status:** ready to implement
**Scope:** `crates/action/src/poll.rs` + `crates/action/src/trigger.rs` (docs only) + tests + context file
**Context file:** `.project/context/crates/action.md`
**Follow-on:** `docs/plans/2026-04-13-poll-api-v2.md` (deferred breaking changes)

## Motivation

After landing the B1–B6 critical correctness fixes (see
`2026-04-13-poll-critical-fixes.md`), an audit surfaced a cluster of
cheap, non-breaking improvements. Three agents (architect, tech-lead,
sdk-user) converged on the same shipping list. This plan bundles them
into one pass so they land together and `nebula-action` gets as close
to v1-stable as we can without breaking the trait shape.

## Principle established by this plan

**`PollConfig` is the action's *downward declaration of integration
constraints*.** It is not a scheduling directive. Who decides *when*
the trigger runs (first poll, fleet-wide staggering, warmup,
activation delay) is a runtime / engine concern, not an action
concern. This principle informs every decision in this plan and is
written down in the crate context file so future contributors don't
re-add `FirstPoll`-shaped knobs.

## Items in scope

### H1 — Flip the loop to `poll → sleep`

Today: `loop { sleep → poll → dispatch }`. First poll happens after
`base_interval`. Bad UX for alerting / Gmail / Stripe / CRM.

Fix: `loop { poll → dispatch → sleep }` with a cheap pre-poll
cancellation check so a `start()` → immediate `cancel()` sequence
exits before running any poll.

```rust
loop {
    if ctx.cancellation.is_cancelled() {
        return Ok(());
    }
    // poll + dispatch + resolve_cycle (unchanged)
    // ...
    let interval = /* compute with override_next clamping */;
    tokio::select! {
        _ = ctx.cancellation.cancelled() => return Ok(()),
        _ = tokio::time::sleep(interval) => {}
    }
}
```

**No new API.** No `FirstPoll` enum, no config field. Runtime controls
pre-activation delay by delaying its call to `adapter.start(ctx)` once
it grows an `ActivationScheduler` (not this plan). For now, all
actions first-poll immediately — this matches user expectations
(`PollConfig::fixed(60s)` should mean "every 60s starting now", not
"wait 60s then every 60s").

Breaking-change note: `poll_habr` example will now first-poll
immediately on process start. Intentional. Update its README if the
timing is referenced there.

### H2 — `stop()` cancels `ctx.cancellation`

Today: `PollTriggerAdapter::stop` is `Ok(())`. But `TriggerHandler`
contract (trigger.rs:325-327) says *"Clears any state set by start so
a subsequent start call is accepted"*. For shape-2 adapters that's
only achievable via the cancellation token — and `stop()` was
relying on the caller to cancel it. Wrong owner.

Fix: `stop()` calls `ctx.cancellation.cancel()` itself. The background
task sees the cancel on its next `select!`, exits the loop, RAII
`StartedGuard` clears the `started` flag. Callers who need
synchronous restart still do `stop(); handle.await; start()` — that
stays documented in the `stop()` docstring.

**Keep the `started: AtomicBool` + `StartedGuard`.** Architect
argued for deletion (runtime is sole caller, guard is footgun). I
disagree for now: runtime is alpha, guard is defense-in-depth, and
the stop→start deadlock it causes is exactly what H2 fixes. If we
ever move task ownership into runtime we can revisit deletion; until
then the guard stays.

### H3 — Jitter seed uses trigger identity

Today: `action_key_seed(&self.action.metadata().key)` — same for all
workflow instances of the same action type. 100 fleet instances get
identical jitter streams.

Fix: seed = FNV-1a hash over `action_key ∥ workflow_id ∥ trigger_id`
read from `TriggerContext`. Each trigger instance gets a unique seed;
same action type across different workflows/nodes desynchronizes.

No API change — `action_key_seed` gets renamed to `trigger_seed` and
takes `&TriggerContext` (or the three IDs separately). Called once at
`start()`.

### H4 — `#[non_exhaustive]` on `PollResult<E>`

Missing today. `PollConfig`, `PollOutcome`, `EmitFailurePolicy`
already have it. Add the attribute so adding fields later is not a
source break.

Also audit: verify `FirstPoll`-style struct additions to `PollConfig`
are source-break-safe (they are — `PollConfig` is already
`#[non_exhaustive]`).

### H5 — Validate `PollConfig` invariants

Add constructor-time validation:
- `max_interval >= base_interval` — today a rogue `max < base`
  silently caps the effective interval to `max`, confusing.
- `backoff_factor` clamped to `[1.0, 60.0]`. Today only `>= 1.0` via
  `.max(1.0)` in `with_backoff`. Upper bound prevents `f64::INFINITY`
  or absurd `1e308` from producing NaN-land behavior.
- `poll_timeout > 0` — a zero timeout makes every poll retryable-timeout.
- `jitter` clamp already exists (0.0–0.5), keep.

Validation runs in `with_backoff`, `fixed`, and a new `PollConfig::validate`
method called by the adapter at `start()` before entering the loop.
On failure, log warn + clamp to safe defaults (don't fail startup —
this is configuration, not credentials).

### H6 — Loud persistence warning in `PollAction` trait doc

Sdk-user's biggest gotcha: cursor is in-memory, resets on process
restart, silent data loss or flood on Stripe/Gmail. Docstring today
mentions it once at module level.

Fix: add a `# Persistence — read before shipping` section to the
`PollAction` trait docstring. Copy:

> Cursor state is in-memory only. On process restart `initial_cursor`
> is called again and all progress is lost. This is acceptable for
> best-effort integrations (RSS, news feeds). **It is NOT acceptable
> for payment, audit, or high-value integrations** — a restart will
> either re-flood upstream (if `initial_cursor` is `Default`) or
> silently skip the gap between last poll and restart (if
> `initial_cursor = now`). Durable cursor storage across restarts
> is a runtime concern tracked as F8 / future work. If your
> integration needs strong delivery guarantees, do NOT ship against
> the current `PollAction` without external idempotency at the
> workflow layer.

Same paragraph as a `<!-- NOTE -->` block in the crate context file.

### H7 — `PollConfig` docstring records the principle

One-paragraph addition at the top of `PollConfig` doc:

> `PollConfig` declares the *integration-intrinsic timing constraints*
> of a poll action — upper/lower interval bounds, per-cycle timeout,
> failure policy. It does NOT decide *when* polls are scheduled,
> staggered, or activated. Those are runtime concerns. In particular,
> there is no "first poll delay" knob: the adapter always first-polls
> immediately after `start()`, and the runtime is expected to delay
> its call to `start()` if it needs per-integration warmup or
> fleet-wide stagger.

This is the fence against future `FirstPoll`-shaped additions.

## Items explicitly deferred (moved to `poll-api-v2.md`)

- F4 `PollBudget { deadline, max_pages, max_events }` replacing
  `max_pages_hint`. Breaking: changes `PollAction::poll` signature.
- F7 drop `Default` bound on `PollAction::Cursor` — supplanted by
  `initial_cursor()`.
- Move `jitter` and `backoff_factor` into adapter policy (sdk-user's
  point — action doesn't own thundering-herd defense). Requires a
  runtime-side policy object and is breaking.
- `emit_failure` re-homing onto workflow binding (architect). Bigger
  design question.
- Cursor persistence (F8). Blocked on runtime storage.
- QueueTrigger / StreamTrigger families (F9). Not a plan doc yet.
  One-universal-TriggerHandler-with-two-shapes remains the bet until
  a real Kafka user files an issue.

## Test plan

New tests in `dx_poll.rs`:

1. `first_poll_runs_immediately_after_start`
   - `start()` + yield, no `tokio::time::advance`
   - assert poll_count >= 1 and emitter.count() >= 1
2. `stop_cancels_cancellation_token`
   - spawn `start()`, call `adapter.stop()`, assert the task exits
     without the test having to call `cancel()` itself
3. `jitter_differs_for_different_trigger_ids`
   - build two `TriggerContext`s with different `trigger_id`
   - assert the computed seed differs (deterministic hash, call
     `trigger_seed` directly via a `#[cfg(test)]` helper)
4. `poll_config_max_interval_below_base_clamped_with_warn`
   - construct `PollConfig { base: 60s, max: 10s }` via direct
     struct update
   - run one start/stop cycle, assert spy_logger saw the warn
5. `poll_config_backoff_factor_clamped_to_sixty`
   - `PollConfig::with_backoff(1s, 1h, 1e9)` → factor == 60.0

Existing tests to re-verify:
- `poll_adapter_emits_events` — should get faster (no initial sleep)
- `poll_adapter_clamps_zero_interval_to_floor` — still valid, loop
  structure change doesn't affect the floor logic
- `poll_adapter_start_after_cancellation_succeeds` — should still
  pass because H2 just makes it more reliable

## Step sequence

1. H1 — flip loop structure, add pre-poll cancel check.
2. H2 — `stop()` calls `ctx.cancellation.cancel()`.
3. H3 — new `trigger_seed(action_key, workflow_id, trigger_id)`,
   wire through `start()`.
4. H4 — `#[non_exhaustive]` on `PollResult`.
5. H5 — `PollConfig::validate` + call from `start()` + clamp.
6. H6 — `PollAction` trait doc: add `# Persistence` section.
7. H7 — `PollConfig` doc: add principle paragraph.
8. Tests: write 5 new, re-run full suite.
9. Update `.project/context/crates/action.md`: principle line,
   eviction candidates list, persistence warning.
10. Run: `cargo +nightly fmt -p nebula-action`,
    `cargo clippy --workspace -- -D warnings`,
    `cargo nextest run -p nebula-action`,
    `cargo test -p nebula-action --doc`.
11. Write `poll-api-v2.md` skeleton so F4/F7 don't rot.
12. Add F8 one-line to `.project/context/active-work.md` under Blocked.

## Acceptance

- All 347+ tests green (342 existing + 5 new).
- `cargo clippy --workspace -- -D warnings` clean.
- No API break to `PollAction` trait shape. `PollConfig` gets new
  behavior (validation + clamping) but no new fields.
- `poll_habr` example still compiles + runs; first poll now
  immediate. No README change required unless README claims "waits
  one interval before first poll."
- Principle (`PollConfig` is constraint, not policy) recorded in
  crate context file.
- `poll-api-v2.md` exists as a skeleton with F4, F7, and the two
  policy-re-homing items listed.
