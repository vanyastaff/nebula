# Poll trigger critical fixes

**Status:** in progress
**Scope:** `crates/action/src/poll.rs`, `crates/action/tests/dx_poll.rs`
**Context file:** `.project/context/crates/action.md`

## Motivation

Audit of `PollTriggerAdapter` surfaced six correctness bugs and several
design gaps. See the audit notes in conversation. This plan addresses the
bugs that silently lose data or misreport health — everything that must
land before `nebula-action` is declared v1-stable.

Out of scope for this plan (tracked separately): jitter PRNG quality,
dispatch cancellation, persistence, cluster leader election, Retry-After
helpers, and the broader feature-set inspired by n8n.

## Bugs in scope

### B1 — RetryBatch dispatch failure does not back off

`handle_dispatch_failure` and the `Partial + RetryBatch` branch both
return `CycleOutcome { backoff: false, emitted: 0 }`. In the main loop
this falls into `!backoff && emitted == 0` → `consecutive_empty = 0`
and `record_idle()`. Effect: if the emitter is down under
`RetryBatch`, the trigger hot-loops every `base_interval` with zero
backoff and reports idle on the health dashboard.

**Fix:** failed-dispatch cycles must set `backoff: true` AND call
`record_error()` instead of `record_idle()`. A new `errored` flag on
`CycleOutcome` disambiguates "idle (no data)" from "errored (data lost
or retried)".

### B2 — Partial with empty events + retryable error is swallowed

`PollOutcome::Partial { events: empty, error }` with a retryable error
returns `Ok(idle)` without logging and without `record_error()`. The
`debug_assert` in `PollResult::partial` catches direct constructor use
but not the enum variant.

**Fix:** route empty-events Partial through the same path as top-level
`Err`: log via `poll_warn`, `record_error()`, rollback cursor, backoff.

### B3 — DropAndContinue total-loss reports "idle"

When all events in a Ready batch get dropped (serialization or emit
failure under DropAndContinue), `is_total_loss()` sets `backoff=true,
emitted=0`. The main loop then calls `record_idle()`. Metrics show a
calm trigger while 100 % of events are being silently lost.

**Fix:** total-loss path must `record_error()`.

### B4 — `DeduplicatingCursor` deserialize with `max_seen = 0`

The `Deserialize` impl reads `max_seen` without clamping. `cap = 0`
leaves every `try_insert` in a state where the key is added and
immediately evicted → dedup effectively disabled but silent.
`with_max_seen` clamps, deserialize does not.

**Fix:** `let cap = wire.max_seen.max(1);`. Add a regression test that
deserializing `{"max_seen": 0, ...}` yields `max_seen == 1` and
dedup still works.

### B5 — `override_next` not clamped by `max_interval`

`override_next` (e.g. from a `Retry-After` header) is clamped only by
`POLL_INTERVAL_FLOOR`. A rogue upstream returning 86400 s makes the
trigger sleep a day even if `max_interval = 1 h`. Either cap it or
document explicitly.

**Fix:** clamp `override_next` by `max(POLL_INTERVAL_FLOOR)
.min(max_interval)`. This matches user intent: `max_interval` means
"never sleep longer than this, period."

### B6 — `DeduplicatingCursor` undocumented batch-size trap

If a single poll batch is larger than `max_seen`, FIFO eviction
inside `filter_new` evicts items from the start of the same batch
before the batch is finished. Next cycle re-sees those at the
boundary → duplicates. The type-level doc does not warn about this.

**Fix:** documentation only. Add a "Sizing" section to
`DeduplicatingCursor` docs explaining `max_seen ≥ 2 × expected batch`.
Add a `debug_assert!` in `filter_new` when `items.len() > max_seen`.

## Non-goals

- Changing the shape of `EmitFailurePolicy` or `PollOutcome` — public API.
- Fixing dispatch cancellation (#9 in audit) — separate plan, touches
  trait contract.
- Jitter PRNG replacement — separate plan, introduces dep decision.
- Removing `Default` bound on `PollAction::Cursor` — touches every
  existing implementor, separate plan.

## Design notes

### `CycleOutcome::errored`

Current struct has `backoff: bool` plus `emitted: usize`. The main
loop infers the health call from these two, which is the source of
B1/B3 drift. Adding an explicit `errored: bool` makes every branch
state its intent:

```rust
struct CycleOutcome<C> {
    cursor: C,
    backoff: bool,
    errored: bool,      // NEW — record_error vs record_idle
    override_next: Option<Duration>,
    emitted: usize,
}
```

Decision table inside `start()`:

| backoff | emitted>0 | errored | health call       | consecutive_empty |
|---------|-----------|---------|-------------------|-------------------|
| false   | yes       | false   | record_success    | reset to 0        |
| false   | no        | false   | record_idle       | reset to 0        |
| true    | yes       | false   | record_success    | += 1              |
| true    | no        | false   | record_idle       | += 1              |
| true    | any       | true    | record_error      | += 1              |

The `errored` column trumps the others for health reporting only —
it does not affect cursor handling, which is already decided in
`resolve_cycle`. That keeps cursor logic where it lives today.

### Override clamp

`override_next` is consumed at the top of the loop. The clamp goes
there:

```rust
let interval = override_next
    .take()
    .map(|d| d.clamp(POLL_INTERVAL_FLOOR, config.max_interval))
    .unwrap_or_else(|| compute_interval(&config, consecutive_empty, seed));
```

`clamp(lo, hi)` panics if `lo > hi`. `POLL_INTERVAL_FLOOR` is 100 ms;
`max_interval` default is 1 h, and `PollConfig::fixed(x)` sets
`max_interval = x` — the smallest legal `x` is 100 ms (enforced
elsewhere via the start-time warn + floor in `compute_interval`).
So `max_interval >= POLL_INTERVAL_FLOOR` for any config that does
not already log the warn; for configs that DO log the warn, clamp it
to `POLL_INTERVAL_FLOOR` upfront at loop start.

Actually simpler: compute `effective_max = max(max_interval, floor)`
locally once before the loop and use that.

## Test plan

New tests in `dx_poll.rs` (each uses a tiny action + mock emitter):

1. `retry_batch_dispatch_failure_backs_off`
   - emitter always fails, policy = RetryBatch
   - run 3 cycles, assert `consecutive_empty` grows (visible via
     interval observation), assert `error_streak` grows in health
     snapshot, assert `idle_streak == 0`.
2. `partial_with_empty_events_retryable_logs_and_errors`
   - action returns `PollOutcome::Partial { events: vec![], error: retryable }`
     constructed directly (bypass `debug_assert`)
   - assert health `error_streak > 0`, cursor rolled back
3. `drop_and_continue_total_loss_records_error`
   - serialization fails for every event, policy = DropAndContinue
   - assert health `error_streak > 0`, NOT `idle_streak`
4. `dedup_cursor_deserialize_clamps_max_seen_zero`
   - round-trip JSON with `max_seen: 0`
   - assert restored `max_seen >= 1` and `mark_seen`/`is_new` work
5. `override_next_clamped_by_max_interval`
   - action returns `override_next = Some(1h)` with config
     `max_interval = 60s`
   - assert next cycle sleeps at most 60s (observe via short interval)
6. Existing tests must still pass — no API shape changes.

## Step sequence

1. **Add `errored` field to `CycleOutcome`** and route every branch in
   `resolve_cycle` / `handle_dispatch_failure` through it. Switch the
   main loop's health logic to use the decision table above.
2. **Fix B1:** `handle_dispatch_failure` under RetryBatch and the
   Partial+RetryBatch branch set `backoff: true, errored: true`.
3. **Fix B2:** Partial with empty events + retryable — log via
   `poll_warn`, set `errored: true`, backoff, return Ok.
4. **Fix B3:** Ready + total-loss sets `errored: true`.
5. **Fix B4:** clamp `max_seen` in `Deserialize` impl, regression test.
6. **Fix B5:** clamp `override_next` against `max_interval` at loop
   top.
7. **Fix B6:** docs + `debug_assert` in `filter_new`.
8. **Tests:** write all six tests, run `cargo nextest run -p nebula-action`.
9. **Clippy + fmt:** `cargo clippy -p nebula-action -- -D warnings`,
   `cargo +nightly fmt`.
10. **Update context file:** `.project/context/crates/action.md` — note
    the poll trigger health fix and the cursor deserialize clamp.

## Acceptance

- `cargo nextest run -p nebula-action` green.
- `cargo clippy --workspace -- -D warnings` green.
- All six new tests present and green.
- Audit bugs B1–B6 demonstrably fixed by new tests.
- No changes to public trait shapes (PollAction / PollConfig /
  EmitFailurePolicy / PollOutcome / PollResult).