# Nebula Action — Hardening & Refactor Plan

**Date:** 2026-04-10
**Crate:** `nebula-action`
**Source:** parallel audit passes (audit.md / safety.md / architect.md / tech-lead / security-lead)
**Status:** approved, Phase 1 starting

---

## Motivation

After a structural cleanup of `nebula-action` (module reorganisation by action family — commit `ff62029c`), a set of parallel audit passes revealed correctness, security, and architectural issues in the crate that were not addressed by the reorganisation:

- One **CRITICAL** data corruption bug in `StatefulActionAdapter` (state mutations lost on error → duplicated side effects on retry)
- Three **HIGH** adapter bugs (webhook double-start leak, poll busy loop, resource Config/Instance type mismatch)
- One **HIGH** security hole: webhook signature verification has no constant-time helper, leading authors into timing side-channels
- Seven **MEDIUM** findings including the fake `TransactionalAction` state machine, `ErrorCode`/`Classify::code()` name collision, unbounded `IncomingEvent` body and header count, silent error swallowing in `PollTriggerAdapter`, `TestResourceAccessor` consume-on-acquire, and `ActionContext`/`TriggerContext` credential-method duplication
- Seven **LOW** findings, mostly validation and log-injection hardening

This plan executes all of them in a phased sequence.

## Principles

- **No shims, no backward-compatibility tax.** Per `architect.md`: break the wrong thing, update callers, move on.
- **Type-level guarantees over runtime checks.** Make invalid states unrepresentable where possible.
- **Verification before claims.** Each phase ends with `cargo fmt && cargo clippy -p nebula-action -- -D warnings && cargo nextest run -p nebula-action` and, where the fix crosses crates, the same on `nebula-sdk -p nebula-runtime -p nebula-engine`.
- **Scope discipline.** Each phase stays within its declared file set. No drive-by refactors.

---

## Decisions (resolved)

| Topic | Decision |
|---|---|
| M1 `TransactionalAction` | **Delete entirely**. Trait adds nothing over `StatefulAction` with tuple output; future saga orchestrator will need a completely different shape. |
| M2 `error_code` naming | **Rename** `ErrorCode` → `RetryHintCode`, `error_code()` → `retry_hint_code()` (by value, enum is `Copy`). Concepts differ: retry hint vs classifier tag. |
| M6 logging mechanism | **`ctx.logger` via `ActionLogger`**, not `tracing` direct. Matches capability-injection pattern of the crate. |
| L7 `Validation` | **Structured refactor**: `Validation { field: &'static str, reason: ValidationReason, detail: Option<String> }` with `detail` sanitized at construction (control-char escape, 256 B cap). |
| `subtle` dependency | Accepted. Added to workspace deps. License BSD-3, already on allow-list. |
| `DEFAULT_MAX_BODY_BYTES` | 1 MiB (covers 99% of real webhook payloads; overrideable via `try_new_with_limits`). |
| `MAX_HEADER_COUNT` | 128. |
| `POLL_INTERVAL_FLOOR` | 100 ms. |
| `WarnThrottle::COOLDOWN` | 30 s. |

---

## Phase sequence

Nine independent PRs, any order within a phase. Phases are ordered by impact (critical bugs → security → cleanup).

### Phase 1 — Critical adapter bugs `fix(action): adapter correctness`

**PR 1.** Three commits in one PR, shared reviewer context.

- **Commit 1.1 — A1 StatefulActionAdapter state checkpoint**
  - File: `crates/action/src/handler.rs` `StatefulActionAdapter::execute` (lines 175-204)
  - Write `*state = to_value(&typed_state)` on **both** `Ok` and `Err` paths from `action.execute()`
  - On serialization failure during error path: log via `tracing::error!`, propagate the original action error (serde failure must not mask the actionable signal)
  - Keep the `Validation` deserialization errors unchanged — typed_state never existed
  - Update `StatefulHandler::execute` doc with a "State checkpointing" section describing the invariant
  - Tests (in `handler.rs` unit tests): `stateful_adapter_checkpoints_state_on_retryable_error`, `stateful_adapter_checkpoints_state_on_fatal_error`, `stateful_adapter_preserves_state_on_validation_error`
  - Update `.claude/crates/action.md` Traps

- **Commit 1.2 — A2 Double-start rejection**
  - `WebhookTriggerAdapter::start` (handler.rs:303-307): read-lock pre-check + write-lock re-verify, reject with `Fatal("already started")`; on write-lock race, deactivate the just-created state
  - `PollTriggerAdapter`: add `started: AtomicBool`, use `compare_exchange(false, true, AcqRel, Acquire)`, RAII `StartedGuard` with defused pattern to clear flag on exit (not `mem::forget`)
  - Update `TriggerHandler::start` doc to declare double-start rejection as the contract
  - Tests: `webhook_adapter_rejects_double_start`, `webhook_adapter_start_stop_start_succeeds`, `poll_adapter_rejects_concurrent_start` (uses `tokio::time::pause()`)
  - Update `.claude/crates/action.md` Traps

- **Commit 1.3 — A3 Poll interval floor + contract doc**
  - `PollTriggerAdapter::start`: `const POLL_INTERVAL_FLOOR: Duration = Duration::from_millis(100)`; `let interval = raw_interval.max(POLL_INTERVAL_FLOOR)`; warn-log on clamp
  - Rewrite `TriggerHandler::start` trait doc with explicit "setup-and-return vs run-until-cancelled" dual contract; document "callers MUST spawn"
  - Silent error drops → `tracing::warn!`/`tracing::debug!` (temporary — Phase 5 will replace with `ActionLogger`-based throttled logging in M6)
  - Tests: `poll_adapter_clamps_zero_interval_to_floor`, `poll_adapter_continues_on_retryable_poll_error`, `poll_adapter_stops_on_fatal_poll_error`
  - Update `.claude/crates/action.md` Traps

**Do NOT touch in Phase 1:** `TriggerContext` pub fields, `StatefulAction::execute` signature, `StatelessActionAdapter`, `ResourceActionAdapter`, `PollAction::Cursor` persistence, any other adapter.

**Verification:** `cargo fmt && cargo clippy -p nebula-action --all-targets -- -D warnings && cargo nextest run -p nebula-action && cargo nextest run -p nebula-sdk -p nebula-runtime -p nebula-engine`.

---

### Phase 2 — Architecture corrections `refactor(action): eliminate false flexibility`

Two independent PRs.

- **PR 2 — A4 ResourceAction single-type collapse**
  - `resource.rs`: delete `type Instance`, rename `type Config` → `type Resource`, update `configure`/`cleanup` signatures
  - `handler.rs` `ResourceActionAdapter` (lines 470-521): simplify to single-type bound, delete Config-vs-Instance doc caveat; `downcast<A::Resource>` becomes a true invariant (unreachable fatal)
  - Test `MockResourceAction` at `handler.rs:1546` — update to `type Resource = String`
  - Add integration test `crates/action/tests/resource_roundtrip.rs` with a `Pool` type verifying configure→cleanup works
  - Update `docs/plans/2026-04-08-action-v2-examples.md:601-603` (doc example)
  - Update `.claude/crates/action.md`

- **PR 3 — M2 RetryHintCode rename**
  - `error.rs`: rename `ErrorCode` → `RetryHintCode`, `error_code()` → `retry_hint_code()` returning `Option<RetryHintCode>` by value, `retryable_with_code`/`fatal_with_code` → `*_with_hint`, `ActionErrorExt::retryable_with_code`/`fatal_with_code` → `*_with_hint`
  - `lib.rs`, `prelude.rs`: update re-exports
  - Update tests in `error.rs` (5 call sites)
  - Update `crates/action/docs/Error Model.md:564`
  - Update `.claude/crates/action.md`

**Verification:** same as Phase 1.

---

### Phase 3 — Security hardening `feat(action): webhook security primitives`

Two independent PRs.

- **PR 4 — A5 Webhook signature verification primitive**
  - New module `crates/action/src/webhook.rs`:
    - `enum SignatureOutcome { Valid, Missing, Invalid }` with `is_valid()` method, `#[non_exhaustive]`
    - `verify_hmac_sha256(event, secret, header) -> Result<SignatureOutcome, ActionError>` supporting bare hex and `sha256=` prefix; rejects empty secret with `Validation`; delegates comparison to `hmac::Mac::verify_slice`
    - `hmac_sha256_compute(secret, payload) -> [u8; 32]` escape hatch for Stripe/Slack-style signature schemes
    - `verify_tag_constant_time(a, b) -> bool` via `subtle::ConstantTimeEq`
  - `lib.rs`: `pub mod webhook;`
  - `trigger.rs` doc example: fix to use `verify_hmac_sha256` instead of the fictional `verify()`
  - `Cargo.toml`: add `hmac`, `sha2`, `hex`, `subtle` as deps (first three already in workspace, `subtle` added to `[workspace.dependencies]`)
  - New test file `crates/action/tests/webhook_signature.rs` with 11 tests (valid, prefixed, wrong secret, tampered body, missing header, case-insensitive, invalid hex, wrong length, empty secret, verify_tag_constant_time, Stripe-style custom scheme)
  - Update `.claude/crates/action.md`

- **PR 5 — M4+M5 IncomingEvent bounds (bundled)**
  - `handler.rs` `IncomingEvent`:
    - Add `pub const DEFAULT_MAX_BODY_BYTES: usize = 1024 * 1024;`
    - Add `pub const MAX_HEADER_COUNT: usize = 128;`
    - **Delete** `fn new(body, headers) -> Self`
    - Add `fn try_new(body, headers) -> Result<Self, ActionError>` using defaults
    - Add `fn try_new_with_limits(body, headers, max_body, max_headers) -> Result<Self, ActionError>`
    - Normalize header keys to lowercase at construction (store as `HashMap<String, String>`)
    - Rewrite `header()` with fast path (lowercase key → direct lookup) and slow path (fold to lowercase, one allocation)
    - Document "last write wins" for duplicate keys
  - Update `crates/action/tests/dx_webhook.rs` (3 call sites) to use `try_new`
  - Tests: `try_new_rejects_oversized_body`, `try_new_accepts_exact_limit_body`, `try_new_with_limits_custom_cap`, `try_new_empty_body_accepted`, `try_new_rejects_too_many_headers`, `try_new_accepts_max_header_count`, `header_lookup_case_insensitive_fast_path`, `header_lookup_last_duplicate_wins`
  - Update `.claude/crates/action.md`

**Verification:** same as Phase 1.

---

### Phase 4 — Delete the fake saga `refactor(action): delete TransactionalAction`

One PR.

- **PR 6 — M1 Delete TransactionalAction + state machine**
  - `stateful.rs:344-480`: delete `TransactionPhase`, `TransactionState`, `TransactionalAction` trait, `impl_transactional_action!` macro
  - `lib.rs`, `prelude.rs`, `crates/sdk/src/prelude.rs`: drop re-exports
  - Delete `examples/transactional.rs`
  - Delete `crates/action/tests/dx_transactional.rs`
  - Strip transactional sections from `crates/action/docs/Action Types.md`, `Custom Actions.md`, `Architecture Review.md`
  - Mark transactional as deferred in `docs/plans/2026-04-09-phase-6-dx-stateful.md`
  - Add one-paragraph note in `.claude/decisions.md` explaining the deletion and the future saga replacement
  - Update `.claude/crates/action.md` and `sdk.md`

**Verification:** same as Phase 1.

---

### Phase 5 — Validation + logging hardening `fix(action): structured errors and poll logging`

Two independent PRs.

- **PR 7 — L7 Structured ValidationError**
  - `error.rs`:
    - Add `pub enum ValidationReason { MissingField, WrongType, OutOfRange, MalformedJson, StateDeserialization, Other }` with `as_str()` method, `#[non_exhaustive]`
    - Change `ActionError::Validation(String)` → `Validation { field: &'static str, reason: ValidationReason, detail: Option<String> }`
    - Update `#[error(...)]` attribute to format with sanitized detail
    - Add `sanitize_detail(raw)` helper: escape control chars as `\uXXXX`, truncate to 256 bytes
    - Update `ActionError::validation(field, reason, detail)` constructor signature
    - Add `pub const MAX_VALIDATION_DETAIL: usize = 256;`
    - Update `Classify` impl and `is_fatal()` match arms
  - Update callers: `handler.rs:100, 183, 187` (3 sites — `input`, `state` field names, `MalformedJson`/`StateDeserialization` reasons)
  - Update `crates/action/tests/dx_webhook.rs:59`
  - Update `error.rs` tests (5 sites)
  - Update `crates/action/docs/Action Types.md` examples (3 sites)
  - New tests: `validation_sanitizes_control_chars`, `validation_truncates_long_detail`, `validation_null_byte_escaped`, `validation_no_detail_still_useful`, `validation_is_fatal`, `validation_reason_as_str_stable`, `validation_serializes_structured`
  - Update `.claude/crates/action.md`

- **PR 8 — M6 PollTriggerAdapter logging + M3 TestResourceAccessor Arc + LOW misc**
  - M6: Add `WarnThrottle { last_logged: parking_lot::Mutex<Option<Instant>> }` with `COOLDOWN = 30s`, `should_log()` method
  - Add three throttles to `PollTriggerAdapter` (poll_warn, serialize_warn, emit_warn)
  - Replace silent error drops with `ctx.logger.log(ActionLogLevel::Warn, ...)` gated by throttle
  - Update `Debug` impl to maintain `finish_non_exhaustive()`
  - M3: `TestResourceAccessor` — `HashMap<String, Box<dyn Any>>` → `HashMap<String, Arc<dyn Any + Send + Sync>>`, clone Arc on acquire instead of remove. Update tests that depended on consume-on-acquire
  - L1: `result::duration_ms::serialize` — `u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)`
  - L2: `Progress::new`/`ActionResult::continue_with` — `fraction.clamp(0.0, 1.0)` with `if fraction.is_nan() { 0.0 }`
  - L3: `BinaryData::Inline` variant — remove separate `size` field, compute from `bytes.len()`
  - L4: `DeferredRetryConfig` — validate `backoff_coefficient` on construction (newtype `BackoffCoeff(f64)` with `TryFrom` that rejects NaN/≤0/infinity)
  - L5: `WebhookTriggerAdapter::handle_event` — document the stop/handle_event race in doc comment
  - L6: `impl_paginated_action!` — replace `.max(1)` with `debug_assert!(max >= 1)` + document contract
  - Tests: capturing logger, throttle tests, TestResourceAccessor dual-acquire, each LOW fix gets a minimal test
  - Update `.claude/crates/action.md`

**Verification:** same as Phase 1.

---

### Phase 6 — Duplication cleanup `refactor(action): extract CredentialContextExt`

One PR.

- **PR 9 — M7 Context credential-method duplication**
  - Extract trait (e.g. `CredentialContextExt` or similar) with default method implementations for `credential<S>`, `credential_by_id`, `has_credential_id`, `credential_by_type`
  - Implement on both `ActionContext` and `TriggerContext`
  - Remove ~100 LOC of duplicated method bodies
  - Keep existing tests green (they exercise `ctx.credential(...)` through the public API, unchanged)
  - Update `.claude/crates/action.md`

**Verification:** same as Phase 1.

---

## Out of scope (explicit)

These came up during investigation but are deferred:

- Real saga / compensation orchestration (post-v1 engine feature)
- Persistent `PollAction::Cursor` across restarts (needs runtime storage integration)
- Real trigger runtime in `nebula-runtime` (currently `TriggerNotExecutable`)
- Stripe/Slack-specific signature helpers (require time source + tolerance window; ship raw primitive instead)
- Metrics capability on `TriggerContext` (counters for poll failures etc.) — follow-up after `nebula-metrics` capability lands
- `TriggerContext` pub-field access (`ctx.cancellation`, `ctx.emitter`) → method refactor — existing tech debt, tracked separately
- Any changes to `StatelessActionAdapter` — no bugs reported
- `nebula-core` trait changes — no cascade needed
- `handler.rs` file split into `handler/{stateless,stateful,trigger,resource}.rs` — the file is 1678 lines but thematically cohesive; separate effort

---

## Verification strategy

Every phase ends with:

```bash
cargo fmt
cargo clippy -p nebula-action --all-targets -- -D warnings
cargo nextest run -p nebula-action
cargo test -p nebula-action --doc
```

Phases that touch adapters (1, 2, 5) additionally run:

```bash
cargo nextest run -p nebula-sdk -p nebula-runtime -p nebula-engine
```

The full workspace check (`cargo check --workspace`) runs at the end of each phase to catch cross-crate breakage.

Pre-existing clippy warnings in unrelated crates (`nebula-parameter`, `nebula-credential`, `testing.rs:1043 manual_async_fn`, `derive_action.rs` dead fields) remain deferred — they predate this work and are tracked separately.

---

## Tracking

- Task list tracked via `TaskCreate` in the active session
- This plan file is the single source of truth between sessions
- Each PR updates `.claude/crates/action.md` with a fresh `<!-- reviewed: YYYY-MM-DD -->` marker
- Each PR lists its covered findings in the commit body (e.g., `Addresses: A1, A2, A3 from docs/plans/2026-04-10-action-hardening-and-refactor.md`)

---

## Finding cross-reference

| ID | Severity | Source | Phase | PR |
|---|---|---|---|---|
| A1 | CRITICAL | audit | 1 | 1 |
| A2 | HIGH | audit | 1 | 1 |
| A3 | HIGH | audit | 1 | 1 |
| A4 | HIGH | audit | 2 | 2 |
| A5 | HIGH | safety | 3 | 4 |
| M1 | MEDIUM | audit | 4 | 6 |
| M2 | MEDIUM | audit | 2 | 3 |
| M3 | MEDIUM | audit | 5 | 8 |
| M4 | MEDIUM | safety | 3 | 5 |
| M5 | MEDIUM | safety | 3 | 5 |
| M6 | MEDIUM | safety | 5 | 8 |
| M7 | MEDIUM | audit | 6 | 9 |
| L1 | LOW | audit | 5 | 8 |
| L2 | LOW | audit | 5 | 8 |
| L3 | LOW | audit | 5 | 8 |
| L4 | LOW | audit | 5 | 8 |
| L5 | LOW | audit | 5 | 8 |
| L6 | LOW | audit | 5 | 8 |
| L7 | MEDIUM (upgraded) | safety | 5 | 7 |
