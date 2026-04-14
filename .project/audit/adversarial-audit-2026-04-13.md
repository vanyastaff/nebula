# Nebula Adversarial Audit — 2026-04-13

Staff+ / incident-reviewer / bug-hunter pass. Real fixes + regression tests.

## 1. Architecture summary (zones inspected)

- **Orchestration:** `crates/runtime/src/runtime.rs` — `ActionRuntime`,
  dispatch per `ActionHandler` kind, data-limit enforcement, metrics.
- **Credential lifecycle:** `crates/credential/src/refresh.rs` (thundering-
  herd coordinator) + `crates/credential/src/resolver.rs` (winner/waiter
  refresh protocol + scopeguard cleanup + circuit breaker).
- **Resource lifecycle:** `crates/resource/src/release_queue.rs` (bounded
  cleanup task pool), `crates/resource/src/recovery/gate.rs` (recovery
  state machine).
- **Sandbox lifecycle:** `crates/sandbox/src/process.rs` (subprocess
  supervisor + stderr drainer + handshake timeout).
- **Plugin transport:** `crates/plugin-sdk/src/transport.rs` (UDS / named
  pipe bootstrap).
- **Deny / CI:** `deny.toml`, `.github/workflows/ci.yml`.

Critical paths: the `ActionRuntime::enforce_data_limit` gate sits between
every action output and every downstream node, and the
`CredentialResolver::resolve_and_refresh` Waiter path fires on every cache
miss against a refreshing credential — both are hit on every single
workflow node, so bugs there compound workflow-wide.

## 2. Prioritised findings

### F-1  Data-limit bypass for fan-out and branch-alternative outputs
- **Severity:** MEDIUM (DoS / memory exhaustion surface)
- **Confidence:** HIGH — reproduced by two red-team tests, both previously
  passed on the broken code.
- **Files:** `crates/runtime/src/runtime.rs` — `primary_output_mut` +
  `enforce_data_limit`.
- **Failure scenario:** An action returns
  `ActionResult::MultiOutput { main_output: Some(tiny), outputs: {"big_port": ActionOutput::Value(huge)} }`.
  Downstream nodes subscribed to `big_port` receive `huge` — which can be
  arbitrarily large, far beyond `DataPassingPolicy::max_node_output_bytes`.
  Same hole for `ActionResult::Branch.alternatives` (a HashMap of preview
  outputs that also ship downstream when previews are wired).
- **Why it matters operationally:** runtime accepts untrusted plugin
  output. A single misbehaving or malicious plugin could OOM a worker, or
  cascade a 1 GB preview through the engine, by hiding the payload in a
  non-main port. The whole point of `DataPassingPolicy` is to cap
  per-node output; the limit was effectively per-slot-on-main.
- **Why tests missed it:** existing tests only exercised `Success`,
  which has exactly one output slot and correctly hit the limit.
  `MultiOutput` / `Branch.alternatives` had no size-limit coverage.
- **Fix:** replaced `primary_output_mut` with `collect_output_slots_mut`,
  which walks every downstream-visible slot in the `ActionResult` and
  hands `enforce_data_limit` a disjoint `Vec<&mut ActionOutput<Value>>`.
  The enforcement loop then applies Reject/SpillToBlob per-slot. Spill
  rewrites each oversized slot to `ActionOutput::Reference` in place.

### F-2  Refresh waiter lost-wakeup → 60 s stall per miss
- **Severity:** HIGH (latency spike, SLO breach under bursty credential
  refresh)
- **Confidence:** HIGH — primitive behaviour pinned by a unit test.
- **Files:** `crates/credential/src/resolver.rs` (`RefreshAttempt::Waiter`
  arm), pattern consumed from `crates/credential/src/refresh.rs`.
- **Failure scenario:** Classic `tokio::sync::Notify` lost-wakeup. T1 wins
  `try_refresh`, T2 loses and receives `Arc<Notify>`. T2 then evaluates
  `tokio::time::timeout(60s, notify.notified())` — constructing the
  `Notified` future, wrapping it in a `Timeout`, and finally polling.
  `Notified` only *registers on its parent* when polled. If T1's
  refresh returns between `try_refresh` and T2's first poll (e.g. cached
  provider response, or simply scheduler ordering), T1's
  `notify_waiters()` from the scopeguard wakes zero waiters. T2 then
  stalls for the full 60-second timeout before falling through to the
  recovery re-read. The recovery re-read is correct, but 60 s is
  user-visible latency that will trip SLOs under a synchronised refresh
  burst (e.g. many credentials expiring at the top of the hour).
- **Why it matters operationally:** the coordinator exists specifically
  to handle thundering herds — the worst-case is bursts. Under a burst,
  the race probability goes up, not down, and a 60-second stall per
  losing waiter at burst time is an availability failure.
- **Why tests missed it:** the existing `complete_and_notify_wakes_waiters`
  and `multiple_waiters_all_notified` tests use an explicit `oneshot`
  handshake (`ready_tx`/`ready_rx`) to force the waiter to register
  before the winner notifies. This makes the test deterministic — and
  silently papers over the real-world race it was meant to cover. The
  tests exercised the happy path of `Notify::notify_waiters`, not the
  lost-wakeup path.
- **Fix:**
  1. Pre-construct + `tokio::pin!` + `.as_mut().enable()` the
     `Notified` future *before* any await. `enable()` registers the
     waiter on `Notify`'s internal queue atomically with respect to
     `notify_waiters()`, so the race window shrinks from "await's first
     poll" to "a handful of instructions between `try_refresh` returning
     and `enable()` running".
  2. Shortened the Waiter-side timeout from 60 s to 5 s. The post-wait
     `resolve::<C>()` re-read is always correct and serves as race
     recovery — a 5 s bound on worst-case waiter latency is far more
     acceptable than 60 s, and is still long enough for a legitimate
     refresh to complete (refreshes hold a `refresh_semaphore` permit
     and are normally sub-second).
- **Residual risk:** the race is narrowed, not eliminated. An adversarial
  scheduler could still interleave `notify_waiters()` between the
  `try_refresh` mutex release and the `enable()` call. The 5 s short-
  timeout + store re-read remains the safety net for this case.

## 3. Patches applied

### `crates/runtime/src/runtime.rs`
- Deleted `primary_output_mut`; added `collect_output_slots_mut` that
  walks every `ActionResult` variant's downstream-visible slots.
- Rewrote `enforce_data_limit` to iterate over all collected slots,
  applying `Reject` / `SpillToBlob` per slot, rewriting spilled slots
  in place to `ActionOutput::Reference`.
- Trade-off: `SpillToBlob` now issues one blob write per oversized slot.
  For a `MultiOutput` with several large ports, this is N sequential
  writes. Could be parallelised with `join_all`, but sequential keeps
  the error path simple (short-circuit on first failure) and spill is
  a rare slow path anyway.

### `crates/credential/src/resolver.rs`
- Waiter arm pre-enables the `Notified` future before await.
- Timeout reduced from 60 s → 5 s.
- Timeout branch downgraded from `warn!` → `debug!` since it is now an
  expected race-recovery path, not an abnormal signal.

### `.project/context/crates/runtime.md`, `.../crates/credential.md`
- Documented the new invariants so the next reviewer does not undo
  them.

## 4. Tests added

### `crates/runtime/src/runtime.rs::tests`
- `multi_output_fanout_port_respects_reject_limit` — constructs a
  `MultiOutput` with tiny `main_output` and a large fan-out port; asserts
  `DataLimitExceeded`. Previously passed on the broken code.
- `branch_alternatives_respect_reject_limit` — constructs a `Branch`
  whose selected output fits the limit but whose `alternatives` entry
  does not; asserts `DataLimitExceeded`. Previously passed on the
  broken code.

### `crates/credential/src/refresh.rs::tests`
- `pre_enabled_notified_receives_notify_waiters` — pins the
  `Notified::enable()` invariant: create + pin + enable, **then** call
  `notify.notify_waiters()` before ever polling, then await. Without
  `enable()` this would hang; with it, the await completes immediately.
  This is a guard-rail against tokio semantic drift as much as a
  regression test for the Waiter path.

### Test pass
- `cargo nextest run -p nebula-runtime -p nebula-credential` — 325/325
  pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — exit 0.

## 5. Remaining risks not yet fixed

1. **Race residue on refresh waiters** (F-2 residual). Fully closing the
   race would require either a `watch` channel with a version counter,
   or returning a pre-registered wait handle from `try_refresh` itself.
   Both are bigger changes — the current fix is the maximum mitigation
   achievable with a local patch and the short-timeout safety net makes
   the residual survivable.

2. **`SpillToBlob` fan-out writes are sequential.** A `MultiOutput` with
   N oversized ports takes N sequential blob writes, blocking the
   dispatch path for N × write-latency. Acceptable today because spill
   is a rare slow path, but if production workloads start seeing
   multi-port spills regularly, this should become a `join_all`.

3. **`tokio::spawn(drain_plugin_stderr)` in `sandbox/process.rs:321` has
   no explicit lifecycle comment.** Handle is dropped; correctness
   depends on `child.kill_on_drop(true)` closing stderr which ends the
   task. Invariant holds today — left unfixed since it's not a bug,
   but documenting it on the spawn site would make it safer for future
   edits. Flagged here for a follow-up touch.

4. **`deny.toml` still doesn't enforce layers.** Called out in the
   earlier pass (`audit-2026-04-13.md`), and `CLAUDE.md` now correctly
   says "convention only — not enforced by tooling". Adding real
   `[[bans.deny]]` edges is a policy decision, not a bug fix.

5. **Context-budget violations:** 8 `.project/context/*.md` files remain
   over budget (`action.md` dominates at 5256/1500). Pre-existing from
   the earlier pass, not touched here.

## Verification

- `cargo check --workspace --all-targets` — exit 0
- `cargo clippy --workspace --all-targets -- -D warnings` — exit 0
- `cargo nextest run -p nebula-runtime -p nebula-credential` —
  325/325 passed, 0 failed
