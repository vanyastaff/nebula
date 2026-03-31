# Resilience Migration Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace 3 hand-rolled retry implementations with nebula-resilience, eliminating ~150 lines of duplicate logic.

**Architecture:** Each migration replaces a manual retry loop with `nebula_resilience::retry_with` + `RetryConfig`. Config types (`AcquireResilience`, `RotationRetryPolicy`, `RetryPolicy`) become thin facades that construct `RetryConfig` internally. Public API shapes preserved where possible via `#[deprecated]` re-exports.

**Tech Stack:** nebula-resilience (retry_with, BackoffConfig, JitterConfig, total_budget), nebula-error (Classify)

---

### Task 1: nebula-resource — add nebula-resilience dependency

**Files:**
- Modify: `crates/resource/Cargo.toml`

Add `nebula-resilience = { path = "../resilience" }` to `[dependencies]`.

### Task 2: nebula-resource — replace execute_with_resilience

**Files:**
- Modify: `crates/resource/src/manager.rs:1390-1477`

Replace the 87-line `execute_with_resilience()` with ~20 lines using `nebula_resilience::retry_with` + `RetryConfig`. The `Error` type already implements `Classify` with correct `is_retryable()` and `retry_hint()`, so `retry_with` auto-skips non-retryable errors and respects retry-after hints natively.

Map `CallError` back to `Error` at the boundary.

### Task 3: nebula-resource — verify

Run: `cargo check -p nebula-resource && cargo nextest run -p nebula-resource`

### Task 4: nebula-credential rotation/retry.rs — replace retry_with_backoff

**Files:**
- Modify: `crates/credential/src/rotation/retry.rs`

Replace `retry_with_backoff()` with a thin wrapper around `nebula_resilience::retry_with`. `RotationError` already implements `Classify`. Keep `RotationRetryPolicy` as a config facade that builds `RetryConfig`.

Remove `rand` dependency from backoff_duration (resilience uses `fastrand` internally via `JitterConfig`).

### Task 5: nebula-credential utils/retry.rs — replace retry_with_policy

**Files:**
- Modify: `crates/credential/src/utils/retry.rs`

Replace `retry_with_policy()`. This one is trickier: `E: Display` (no Classify bound). Use `RetryConfig::retry_if(|_| true)` to retry all errors (matching current behavior). Keep `RetryPolicy` as config facade.

### Task 6: nebula-credential — verify

Run: `cargo check -p nebula-credential && cargo nextest run -p nebula-credential`

### Task 7: Full workspace validation

Run: `cargo fmt && cargo clippy --workspace -- -D warnings && cargo nextest run --workspace`

### Task 8: Commit

One commit per crate: resource migration, credential migration.
