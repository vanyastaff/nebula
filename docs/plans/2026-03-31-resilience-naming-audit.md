# Resilience Naming Audit — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix all 8 naming issues (N1-N8) found during Rust API Guidelines audit of `nebula-resilience`.

**Architecture:** Pure rename refactoring — no logic changes. All changes are breaking API changes within the crate; no external consumers exist yet (alpha). Changes are ordered by dependency: trait renames first, then struct/method renames, then cleanup.

**Tech Stack:** Rust, cargo, `replace_all` edits.

---

### Task 1: N1 — Unify `execute` → `call` on `RateLimiter` trait

**Files:**
- Modify: `crates/resilience/src/rate_limiter.rs` — rename trait method + all 5 impls
- Modify: `crates/resilience/benches/rate_limiter.rs` — update bench calls
- Modify: `crates/resilience/tests/integration_rate_limiter.rs` — if uses `.execute()`

**What:** Rename `RateLimiter::execute` → `RateLimiter::call` in trait definition and all implementations (TokenBucket, LeakyBucket, SlidingWindow, AdaptiveRateLimiter, GovernorRateLimiter).

**Verify:** `cargo check -p nebula-resilience && cargo nextest run -p nebula-resilience`

---

### Task 2: N1 — Unify `execute` → `call` on HedgeExecutor + AdaptiveHedgeExecutor

**Files:**
- Modify: `crates/resilience/src/hedge.rs` — rename pub method + internal usage + tests

**What:** `HedgeExecutor::execute` → `HedgeExecutor::call`, `AdaptiveHedgeExecutor::execute` → `AdaptiveHedgeExecutor::call`. Update internal call in `AdaptiveHedgeExecutor::call` that delegates to `HedgeExecutor`.

**Verify:** `cargo check -p nebula-resilience`

---

### Task 3: N1 — Unify `execute` → `call` on CancellationContext

**Files:**
- Modify: `crates/resilience/src/cancellation.rs` — rename `execute` → `call`, `execute_with_timeout` → `call_with_timeout` + tests

**Verify:** `cargo check -p nebula-resilience`

---

### Task 4: N1 — Unify `execute` → `call` on FallbackOperation

**Files:**
- Modify: `crates/resilience/src/fallback.rs` — rename `execute` → `call` + tests
- Modify: `crates/resilience/benches/fallback.rs` — update bench calls
- Modify: `crates/resilience/tests/integration_fallback_fault_injection.rs` — update `.execute()` calls

**Verify:** `cargo check -p nebula-resilience && cargo nextest run -p nebula-resilience`

---

### Task 5: N2+N3 — Remove `timeout_fn` alias and `resilience` convenience module

**Files:**
- Modify: `crates/resilience/src/lib.rs` — remove `timeout as timeout_fn` re-export, delete `pub mod resilience { ... }` block

**Verify:** `cargo check --workspace` (ensure no external usage)

---

### Task 6: N4 — Rename `half_open_max_ops` → `max_half_open_operations`

**Files:**
- Modify: `crates/resilience/src/circuit_breaker.rs` — field rename + validate() + Default + all internal usage + tests
- Modify: `crates/resilience/src/pipeline.rs` — test that sets field
- Modify: `crates/resilience/benches/compose.rs` — if it sets the field

**Verify:** `cargo check -p nebula-resilience && cargo bench --no-run -p nebula-resilience`

---

### Task 7: N5 — Rename `can_execute` → `try_acquire`

**Files:**
- Modify: `crates/resilience/src/circuit_breaker.rs` — rename pub method + `call()` internal usage + tests
- Modify: `crates/resilience/src/pipeline.rs` — internal call + comment
- Modify: `crates/resilience/benches/README.md` — update text references

**Verify:** `cargo check -p nebula-resilience && cargo nextest run -p nebula-resilience`

---

### Task 8: N6 — Stop re-exporting `Outcome` at crate root

**Files:**
- Modify: `crates/resilience/src/lib.rs` — remove `Outcome` from re-exports line
- Verify no external usage of `nebula_resilience::Outcome` (only `circuit_breaker::Outcome`)

**Verify:** `cargo check --workspace`

---

### Task 9: N7 — Make `acquire_permit` private, keep only `acquire`

**Files:**
- Modify: `crates/resilience/src/bulkhead.rs` — `acquire_permit` is already not `pub` (it's `async fn`, no `pub`). Verify this is correct; if it IS pub, make it `pub(crate)` or inline.

**Verify:** `cargo check -p nebula-resilience`

---

### Task 10: N8 — Move `event_kind()` → `ResilienceEvent::kind()`

**Files:**
- Modify: `crates/resilience/src/sink.rs` — convert freestanding `event_kind` to method on `ResilienceEvent`, update `RecordingSink::count()` internal usage

**Verify:** `cargo check -p nebula-resilience && cargo nextest run -p nebula-resilience`

---

### Task 11: Final validation + context update

**Run:** `cargo fmt && cargo clippy -p nebula-resilience -- -D warnings && cargo nextest run -p nebula-resilience && cargo bench --no-run -p nebula-resilience && cargo check --workspace`

**Update:** `.claude/crates/resilience.md` — reflect naming changes.
