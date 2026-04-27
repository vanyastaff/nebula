---
name: nebula-action П1 — trait shape scaffolding
status: draft (writing-plans skill output 2026-04-27 — awaiting execution-mode choice)
date: 2026-04-27
authors: [vanyastaff, Claude]
phase: П1
scope: cross-cutting — nebula-action, nebula-engine (compile-fix), nebula-sandbox (compile-fix), nebula-sdk (re-export), nebula-api (compile-fix)
related:
  - docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md
  - docs/superpowers/specs/2026-04-24-action-redesign-strategy.md
  - docs/adr/0038-action-trait-shape.md
  - docs/adr/0039-action-macro-emission.md
  - docs/tracking/cascade-queue.md
---

# Action П1 — Trait Shape Scaffolding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the post-Q1+Q6+Q7+Q8 `nebula-action` trait shape per Tech Spec FROZEN CP4 — `TriggerSource` + `TriggerAction::{Source, Error, accepts_events, idempotency_key, handle}` (Q7 R3 + Q8 F2 + Q6); the four `*Handler` dyn traits migrated to `#[async_trait]` (Q1 §15.9 / ADR-0024 §1+§4); `ActionMetadata::max_concurrent: Option<NonZeroU32>` (Q8 F9); new `IdempotencyKey` type. Zero credential CP6 dependency. macro / sealed DX / security floor / engine wiring all deferred to П2-П6.

**Architecture:** П1 is one coherent breaking-change wave on `nebula-action`'s public surface. Touches one base crate (`nebula-action`) + mechanical reverse-deps fixups in 4 sibling crates (`nebula-engine` Handler call-sites, `nebula-sandbox` Handler bridges, `nebula-sdk` prelude alignment, `nebula-api` webhook consumer). Done in a dedicated worktree, one commit per task, no backward-compat shims (per `feedback_hard_breaking_changes.md` + `feedback_no_shims.md`). Tests: existing nextest suite updated against the new shape; one new compile-fail probe verifies `Source` is required.

**Tech Stack:** Rust 1.95.0 (pinned, ADR-0019), tokio 1.51, `async-trait` 0.1 (Q1 amendment authority via ADR-0024 §1+§4), `serde`/`serde_json`, `cargo-nextest` (CI test runner per `.github/workflows/test-matrix.yml`), `cargo-public-api` (ABI snapshot baseline + delta), `trybuild` 1.0 (compile-fail probe).

**Pre-execution requirement:** Plan execution agent runs in a dedicated worktree per `superpowers:using-git-worktrees`. (The current `romantic-swirles-d6df0e` worktree is already isolated; if executor wants a fresh one, see Stage 0 Task 0.1 alternative.)

**Reading order for the engineer:**
- Tech Spec §2.2.3 (TriggerAction shape) lines 230–391 + §15.10 (Q6 lifecycle gap fix) + §15.11 R3-R5 (Q7 lifecycle slips) + §15.12 F2 / F9 (Q8 idempotency / concurrency)
- Strategy §4.3.1 (HRTB modernization scope — `*Handler` is in-cascade)
- ADR-0024 §1 + §4 (`async_trait` authority for the four `*Handler` dyn-consumed traits)
- ADR-0038 (action trait shape — accepted)
- This plan.

**Non-goals (explicitly deferred):**
- `#[action]` attribute macro replacing `#[derive(Action)]` — П2.
- Sealed DX surface (`ControlAction`/`PaginatedAction`/`BatchAction`/`WebhookAction`/`PollAction` per ADR-0040) + canon §3.5 revision — П2 (gated on Q5 user ratification of ADR-0040).
- Security floor (CR3 hard-removal `CredentialContextExt::credential<S>()`, CR4 JSON depth cap 128, `redacted_display`, cancellation-zeroize test, NEW `nebula-redact` crate) — П3.
- Credential CP6 slot integration (`SlotBinding`/`SchemeGuard<'a, C>`) — П4 (gated on credential CP6 implementation cascade landing per cascade-queue slot 1).
- `ActionResult::Terminate` engine wiring + `unstable-terminate-scheduler` flag — П5.
- T1/T2/T5/T6 codemod transforms — distributed across П2-П6.
- Cluster-mode trait placeholders (`CursorPersistence`/`LeaderElection`/`ExternalSubscriptionLedger`/`ScheduleLedger` per Tech Spec §3.7) — engine cascade slot 2 (engine-side, not action-side).

**П1 surface delta (cargo-public-api):** breaking. Pre-1.0 alpha break is acceptable per `feedback_hard_breaking_changes.md` + Phase 0 audit T7 (semver-checks advisory-only). П1 commit captures pre/post snapshots so post-1.0 callers have a stable target later.

---

## File Map

### Created files

| Path | Purpose |
|------|---------|
| `crates/action/src/idempotency.rs` | `IdempotencyKey` type + smoke tests |
| `crates/action/src/trigger/source.rs` | `TriggerSource` trait + blanket bounds |
| `crates/action/src/webhook/source.rs` | `WebhookSource` zero-sized type implementing `TriggerSource<Event = WebhookRequest>` |
| `crates/action/src/poll/source.rs` | `PollSource` zero-sized type implementing `TriggerSource<Event = PollEvent>` (`PollEvent` is the existing poll cycle envelope; new `*Source` only ties it to the trait) |
| `crates/action/tests/trigger_source_required_compile_fail.rs` | trybuild probe: `impl TriggerAction` without `type Source` fails to compile (E0046) |
| `crates/action/tests/probes/missing_trigger_source.rs` + `.stderr` | trybuild fixture for the probe above |

### Modified files

| Path | Change |
|------|--------|
| `crates/action/Cargo.toml` | `[dependencies]` += `async-trait = "0.1"`; `[dev-dependencies]` += `trybuild = "1"` |
| `crates/action/src/lib.rs` | Re-export `IdempotencyKey`; `trigger::TriggerSource` (and `WebhookSource`, `PollSource` from their domain modules) |
| `crates/action/src/trigger.rs` | (1) Reshape `TriggerAction` per spec §2.2.3 — drop `: Action` super-bound, add `Source: TriggerSource`, `Error`, default `accepts_events()`/`idempotency_key()`, typed `handle()`. (2) `TriggerHandler` flipped to `#[async_trait]` per Q1 |
| `crates/action/src/stateless.rs` | `StatelessHandler` flipped to `#[async_trait]` per Q1 |
| `crates/action/src/stateful.rs` | `StatefulHandler` flipped to `#[async_trait]` per Q1 |
| `crates/action/src/resource.rs` | `ResourceHandler` flipped to `#[async_trait]` per Q1 |
| `crates/action/src/handler.rs` | Update test stub impls (4 of them, lines 143-269) to `#[async_trait]` shape; no public-surface change |
| `crates/action/src/webhook.rs` | Migrate `WebhookAction` to typed `Source = WebhookSource`; adapter downcast via `<Self::Source as TriggerSource>::Event = WebhookRequest` |
| `crates/action/src/poll.rs` | Migrate `PollAction` to typed `Source = PollSource`; adapter downcast |
| `crates/action/src/metadata.rs` | Add `pub max_concurrent: Option<core::num::NonZeroU32>` field with `#[serde(default, skip_serializing_if = "Option::is_none")]`; constructor stays `max_concurrent = None` |
| `crates/action/src/prelude.rs` | Add `IdempotencyKey`, `TriggerSource`, `WebhookSource`, `PollSource` to re-exports |
| `crates/action/tests/dx_webhook.rs` | Update fixture impls to new TriggerAction shape |
| `crates/action/tests/dx_control.rs` | Same (any TriggerAction-implementing fixture) |
| `crates/action/tests/dx_poll.rs` | Same |
| `crates/action/tests/execution_integration.rs` | Same |
| `crates/action/tests/resource_roundtrip.rs` | ResourceHandler async_trait migration touch-up |
| `crates/sdk/src/runtime.rs` | Compile-fix Handler call sites (no signature change at the trait level once `#[async_trait]` lands; review for explicit `Pin<Box<dyn Future>>` patterns we still hand-author) |
| `crates/sandbox/src/handler.rs` | Compile-fix; mirrors `*Handler` HRTB-Box pattern that needs to become `async fn` body or an `#[async_trait]`-emitted shape |
| `crates/sandbox/src/remote_action.rs` | Same |
| `crates/sandbox/src/discovery.rs` | Same |
| `crates/engine/src/runtime/runtime.rs` | Compile-fix `Handler` invocations |
| `crates/engine/src/engine.rs` | Same |
| `crates/api/src/services/webhook/transport.rs` | Compile-fix `WebhookAction` consumer call sites for the typed `Source = WebhookSource` shape |
| `crates/api/src/services/webhook/routing.rs` | Same |
| `crates/api/tests/webhook_transport_integration.rs` | Same — test fixture |

### Deleted files

| Path | Reason |
|------|--------|
| (none in П1) | All trait surface deletions (legacy `CredentialContextExt::credential<S>()`, `#[derive(Action)]`-only paths) are П2-П3. |

### Verification commands (used throughout the plan)

| Purpose | Command |
|---------|---------|
| Compile action only | `cargo check -p nebula-action` |
| Compile workspace | `cargo check --workspace` |
| Tests action only | `cargo nextest run -p nebula-action --profile ci --no-tests=pass` |
| Tests workspace | `cargo nextest run --workspace --profile ci --no-tests=pass` |
| Clippy | `cargo clippy --workspace -- -D warnings` |
| Format check | `cargo +nightly fmt --all -- --check` |
| Public API snapshot | `cargo public-api --manifest-path crates/action/Cargo.toml > /tmp/action-pre-p1.txt` (Stage 0); `> /tmp/action-post-p1.txt` (Stage 8) |

---

## Stage 0 — Worktree + baseline

### Task 0.1 — Worktree confirm + baseline gate

**Files:** none (verification only)

- [ ] **Step 1: Confirm worktree isolation**

```bash
git rev-parse --abbrev-ref HEAD     # expect: claude/romantic-swirles-d6df0e (or fresh worktree branch)
git status --short                   # expect: clean (no uncommitted edits)
```

If working in a fresh worktree instead, create one:

```bash
git worktree add -b nebula-action-p1-trait-shape ../nebula-action-p1
cd ../nebula-action-p1
```

- [ ] **Step 2: Baseline gate — full local CI mirror**

Run:

```bash
cargo +nightly fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo nextest run -p nebula-action -p nebula-engine -p nebula-sandbox -p nebula-sdk -p nebula-api --profile ci --no-tests=pass
```

Expected: PASS. If anything fails, **halt** — fix the pre-existing breakage in a separate commit before П1 work begins (per `feedback_active_dev_mode.md`: do not pile П1 changes on top of red main).

- [ ] **Step 3: Capture pre-П1 cargo-public-api snapshot**

```bash
cargo public-api --manifest-path crates/action/Cargo.toml > /tmp/action-pre-p1.txt
```

Hold the file aside — Stage 8 Task 8.3 diffs against it to validate the breaking-change boundary is exactly what П1 promises.

- [ ] **Step 4: Empty marker commit**

```bash
git commit --allow-empty -m "chore(action): П1 worktree baseline marker

Pre-П1 cargo-public-api snapshot captured at /tmp/action-pre-p1.txt
(local-only; not committed). All workspace gates green at this commit.

Refs: docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md"
```

---

## Stage 1 — `IdempotencyKey` type

Foundational for Stage 3 (TriggerAction reshape uses it). Standalone module — zero coupling to other П1 work.

### Task 1.1 — Add `idempotency` module + smoke test

**Files:**
- Create: `crates/action/src/idempotency.rs`
- Create: `crates/action/tests/idempotency_smoke.rs`
- Modify: `crates/action/src/lib.rs`

- [ ] **Step 1: Write the failing smoke test**

`crates/action/tests/idempotency_smoke.rs`:

```rust
//! Smoke tests for [`IdempotencyKey`].
//!
//! Per Tech Spec §15.12 F2 — `TriggerAction::idempotency_key()` returns
//! `Option<IdempotencyKey>`; this type is the concrete return.

use nebula_action::IdempotencyKey;

#[test]
fn new_round_trips_string() {
    let k = IdempotencyKey::new("delivery-abc-123");
    assert_eq!(k.as_str(), "delivery-abc-123");
}

#[test]
fn equality_is_string_equality() {
    let a = IdempotencyKey::new("x");
    let b = IdempotencyKey::new("x");
    let c = IdempotencyKey::new("y");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn hash_matches_string() {
    use std::collections::HashSet;
    let mut s: HashSet<IdempotencyKey> = HashSet::new();
    s.insert(IdempotencyKey::new("x"));
    assert!(s.contains(&IdempotencyKey::new("x")));
    assert!(!s.contains(&IdempotencyKey::new("y")));
}

#[test]
fn debug_redacts_nothing() {
    // IdempotencyKey is NOT a secret — engine logs / metrics may include it.
    // (Per Tech Spec §15.12 F2, key is a stable transport-level dedup id,
    // not a credential.) This test pins the contract.
    let k = IdempotencyKey::new("delivery-abc-123");
    let debug = format!("{k:?}");
    assert!(debug.contains("delivery-abc-123"));
}
```

- [ ] **Step 2: Run test to verify it fails (compile error — type missing)**

```bash
cargo nextest run -p nebula-action --test idempotency_smoke --profile ci --no-tests=pass
```

Expected: **FAIL** with `error[E0432]: unresolved import 'nebula_action::IdempotencyKey'`.

- [ ] **Step 3: Write `IdempotencyKey`**

Create `crates/action/src/idempotency.rs`:

```rust
//! [`IdempotencyKey`] — transport-level dedup identifier.
//!
//! Returned by [`TriggerAction::idempotency_key`](crate::TriggerAction::idempotency_key)
//! per Tech Spec §15.12 F2. Engine uses the key to suppress duplicate workflow
//! executions when a trigger transport delivers the same event more than once
//! (webhook retry, queue redelivery, schedule replay).
//!
//! Not a secret — engine logs and metrics MAY include the key. For dedup
//! windows and storage, see PRODUCT_CANON §11.3 idempotency.

use std::fmt;

/// Stable per-event dedup identifier returned by a trigger.
///
/// `None` from [`TriggerAction::idempotency_key`] means the trigger does not
/// supply a dedup id — engine falls back to the transport's own dedup or
/// re-delivers events.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Build a key from any string-convertible source.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// View the key as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for IdempotencyKey {
    fn as_ref(&self) -> &str {
        &self.0
    }
}
```

- [ ] **Step 4: Wire module + re-export**

Modify `crates/action/src/lib.rs`:

```rust
// Add near the other domain modules (alphabetical position):
/// [`IdempotencyKey`] — transport-level dedup identifier returned by triggers.
pub mod idempotency;

// In the public re-exports block:
pub use idempotency::IdempotencyKey;
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo nextest run -p nebula-action --test idempotency_smoke --profile ci --no-tests=pass
```

Expected: PASS — 4 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/action/src/idempotency.rs crates/action/src/lib.rs crates/action/tests/idempotency_smoke.rs
git commit -m "feat(action): introduce IdempotencyKey type (П1 / Q8 F2)

Per Tech Spec §15.12 F2 — TriggerAction::idempotency_key() returns
Option<IdempotencyKey>. This commit lands the type; the trait method
follows in Stage 3.

Refs: docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md"
```

---

## Stage 2 — `TriggerSource` trait + blessed sources

Foundational for Stage 3 (TriggerAction.Source associated type) and Stage 6 (Webhook/Poll concrete sources).

### Task 2.1 — Define `TriggerSource` trait

**Files:**
- Create: `crates/action/src/trigger/source.rs` (new submodule of `trigger`)
- Modify: `crates/action/src/trigger.rs` (declare submodule + re-export `TriggerSource`)

> **Note on file layout:** the existing `trigger.rs` is a flat file. Stage 2 promotes it to a `mod trigger` directory: move existing content to `crates/action/src/trigger/mod.rs`, and add `crates/action/src/trigger/source.rs`. Step 1 below does this rename atomically.

- [ ] **Step 1: Promote `trigger.rs` → `trigger/mod.rs`**

```bash
mkdir crates/action/src/trigger
git mv crates/action/src/trigger.rs crates/action/src/trigger/mod.rs
```

Verify: `cargo check -p nebula-action` passes (no content change yet).

- [ ] **Step 2: Write the `TriggerSource` smoke test**

Create `crates/action/tests/trigger_source_smoke.rs`:

```rust
//! Smoke tests for [`TriggerSource`] — verifies the trait exists,
//! is `Send + Sync + 'static`, and exposes an `Event` associated type.

use nebula_action::TriggerSource;

#[derive(Debug)]
struct DummySource;

impl TriggerSource for DummySource {
    type Event = String;
}

#[test]
fn trigger_source_compiles_with_send_sync_static_event() {
    fn assert_send_sync_static<T: Send + Sync + 'static>() {}
    assert_send_sync_static::<DummySource>();
    assert_send_sync_static::<<DummySource as TriggerSource>::Event>();
}
```

- [ ] **Step 3: Run test to verify it fails**

```bash
cargo nextest run -p nebula-action --test trigger_source_smoke --profile ci --no-tests=pass
```

Expected: **FAIL** with `error[E0432]: unresolved import 'nebula_action::TriggerSource'`.

- [ ] **Step 4: Implement `TriggerSource`**

Create `crates/action/src/trigger/source.rs`:

```rust
//! [`TriggerSource`] — typed envelope marker for trigger event families.
//!
//! Per Tech Spec §2.2.3 line 230 — `TriggerAction` has an associated type
//! `Source: TriggerSource`, and the typed event a trigger handler receives
//! is `<Self::Source as TriggerSource>::Event`. This replaces the
//! transport-erased `TriggerEvent` envelope at the user-facing trait
//! level (the dyn-layer envelope still exists for engine routing —
//! see [`crate::trigger::TriggerEvent`]).
//!
//! ## Why the shape is `Source: TriggerSource` and not `type Event` directly
//!
//! Per spike Iter-2 §2.2 (`docs/superpowers/drafts/2026-04-24-nebula-action-redesign/07-spike-NOTES.md`)
//! the indirection lets a trigger family carry transport-specific
//! invariants on the `Source` type (e.g., `WebhookSource` documents
//! `WebhookRequest` body-size caps, `PollSource` documents cursor
//! invariants) without leaking into the base trait.

/// Marker trait identifying a trigger event family.
///
/// Implementors are zero-sized types per spec §2.2.3 — they exist only
/// to tie the family's typed event to [`TriggerAction`](crate::TriggerAction)
/// via the associated type.
pub trait TriggerSource: Send + Sync + 'static {
    /// Concrete event type this source delivers to the trigger handler.
    ///
    /// Examples: `WebhookSource::Event = WebhookRequest`,
    /// `PollSource::Event = PollEvent`.
    type Event: Send + 'static;
}
```

- [ ] **Step 5: Wire submodule and re-export**

Modify `crates/action/src/trigger/mod.rs` — add near the top:

```rust
mod source;
pub use source::TriggerSource;
```

Modify `crates/action/src/lib.rs` — extend the existing `pub use trigger::{...}` block:

```rust
pub use trigger::{
    TriggerAction, TriggerActionAdapter, TriggerEvent, TriggerEventOutcome, TriggerHandler,
    TriggerSource,
};
```

- [ ] **Step 6: Run test to verify it passes**

```bash
cargo nextest run -p nebula-action --test trigger_source_smoke --profile ci --no-tests=pass
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/action/src/trigger/ crates/action/src/lib.rs crates/action/tests/trigger_source_smoke.rs
git commit -m "feat(action): introduce TriggerSource trait (П1 / Q7 R3 step 1)

Per Tech Spec §2.2.3 line 230 — TriggerAction's typed event family
indirects through TriggerSource. This commit lands the marker trait
and promotes trigger.rs → trigger/mod.rs to host source.rs alongside.

The TriggerAction associated type lands in Stage 3.

Refs: docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md"
```

---

## Stage 3 — Reshape `TriggerAction`

Substantial trait reshape per Tech Spec §2.2.3 + Q6 + Q7 R3 + Q8 F2.

### Task 3.1 — Compile-fail probe: `Source` is required

**Files:**
- Create: `crates/action/tests/trigger_source_required_compile_fail.rs` + fixtures

- [ ] **Step 1: Write the trybuild driver**

Create `crates/action/tests/trigger_source_required_compile_fail.rs`:

```rust
//! Probe: `impl TriggerAction` without `type Source` must fail to compile.
//! Per Tech Spec §2.2.3 line 393 — "without it, `impl TriggerAction for X`
//! produces `error[E0046]: not all trait items implemented, missing: Source`".

#[test]
fn missing_source_fails() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/missing_trigger_source.rs");
}
```

Create `crates/action/tests/probes/missing_trigger_source.rs`:

```rust
//! Compile-fail fixture: TriggerAction without Source associated type.
//!
//! Expected error: E0046 "not all trait items implemented, missing: Source".

use nebula_action::{ActionError, ActionMetadata, IdempotencyKey, TriggerAction};
use nebula_action::context::TriggerContext;

struct BadTrigger;

impl TriggerAction for BadTrigger {
    type Error = ActionError;

    fn metadata(&self) -> &ActionMetadata {
        unimplemented!()
    }

    async fn start(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
        Ok(())
    }

    async fn stop(&self, _ctx: &(impl TriggerContext + ?Sized)) -> Result<(), ActionError> {
        Ok(())
    }
}

fn main() {}
```

Create `crates/action/tests/probes/missing_trigger_source.stderr` (placeholder — populated after Stage 3 Task 3.2 lands):

```text
TBD — populated after Task 3.2 by running `TRYBUILD=overwrite cargo test --test trigger_source_required_compile_fail`. Expected to contain "not all trait items implemented" + "missing: Source".
```

- [ ] **Step 2: Defer running the probe**

The probe **cannot** run until Task 3.2 lands the new `TriggerAction` shape (the trait does not yet have `Source`, so the fixture compiles fine today). Marker note added; the probe runs in Task 3.4.

- [ ] **Step 3: Modify `Cargo.toml` for trybuild**

`crates/action/Cargo.toml` — extend `[dev-dependencies]`:

```toml
[dev-dependencies]
# ... existing dev-deps ...
trybuild = "1"
```

Verify:

```bash
cargo check -p nebula-action --tests
```

Expected: PASS (probe driver compiles even though the fixture cannot yet enforce its expected error).

- [ ] **Step 4: Commit (probe scaffold)**

```bash
git add crates/action/Cargo.toml crates/action/tests/trigger_source_required_compile_fail.rs crates/action/tests/probes/missing_trigger_source.rs crates/action/tests/probes/missing_trigger_source.stderr
git commit -m "test(action): add compile-fail probe scaffold for missing Source (П1 / Q7 R3 step 2)

Probe enforces Tech Spec §2.2.3 line 393 invariant: TriggerAction without
type Source fails E0046. Driver lands now; fixture is currently
'compiles fine' because Source isn't yet on the trait — Task 3.2 makes
the probe meaningful and Task 3.4 records the expected stderr.

Refs: docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md"
```

### Task 3.2 — Apply the new `TriggerAction` shape

**Files:**
- Modify: `crates/action/src/trigger/mod.rs` (the trait block)

- [ ] **Step 1: Replace the `TriggerAction` trait block**

Locate the existing trait at `crates/action/src/trigger/mod.rs` lines 61-72 (post-rename). Replace with:

```rust
/// Trigger action: workflow starter, lives outside the execution graph.
///
/// The runtime calls [`start`](Self::start) to begin listening (e.g.
/// webhook subscription, poll timer); [`stop`](Self::stop) to tear down.
/// Triggers emit new workflow executions; they do not run inside one.
///
/// Engine pushes external events via [`handle`](Self::handle); the
/// trigger returns a [`TriggerEventOutcome`] describing how many
/// workflow executions to start (skip / one / many).
///
/// ## Source associated type
///
/// `Source: TriggerSource` ties the trigger to its event family
/// (webhook / poll / queue / schedule). The typed event reaching
/// [`handle`](Self::handle) is `<Self::Source as TriggerSource>::Event`.
/// Per Tech Spec §2.2.3 spike Probe 2 — without `Source`, the impl
/// fails compile with E0046; this is intentional.
///
/// ## Idempotency
///
/// [`idempotency_key`](Self::idempotency_key) returns `None` by default;
/// triggers whose transport supplies a stable per-event id (webhook
/// delivery id, queue message id) override to return `Some(...)`.
/// Engine uses the key to suppress duplicate workflow executions per
/// PRODUCT_CANON §11.3 idempotency.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement TriggerAction",
    note = "implement `Source`, `Error`, and the `start`/`stop`/`handle` methods"
)]
pub trait TriggerAction: Send + Sync + 'static {
    /// Trigger event family — see [`TriggerSource`] (e.g.
    /// [`WebhookSource`](crate::WebhookSource), [`PollSource`](crate::PollSource)).
    type Source: TriggerSource;

    /// Error type returned by lifecycle methods.
    ///
    /// Most implementations use [`ActionError`](crate::ActionError) directly.
    /// Specialized triggers MAY use a richer typed error and let the
    /// adapter wrap it on the way to the dyn-layer.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Static metadata for this trigger.
    fn metadata(&self) -> &crate::ActionMetadata;

    /// Start the trigger (register listener, schedule poll, etc.).
    ///
    /// Per [`TriggerHandler::start`](crate::TriggerHandler::start) for the
    /// two valid lifecycle shapes (setup-and-return vs run-until-cancelled)
    /// and cancel-safety contract.
    fn start(
        &self,
        ctx: &(impl crate::context::TriggerContext + ?Sized),
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    /// Stop the trigger (unregister, cancel schedule).
    fn stop(
        &self,
        ctx: &(impl crate::context::TriggerContext + ?Sized),
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send;

    /// Whether this trigger accepts externally pushed events.
    /// Default: `false`. Override to return `true` from webhook / queue triggers.
    fn accepts_events(&self) -> bool {
        false
    }

    /// Stable transport-level dedup id for this event, if available.
    ///
    /// Default: `None`. Override when the transport supplies a stable
    /// per-event id (webhook delivery id, queue message id).
    /// Per Tech Spec §15.12 F2 + PRODUCT_CANON §11.3 idempotency.
    fn idempotency_key(
        &self,
        _event: &<Self::Source as TriggerSource>::Event,
    ) -> Option<crate::IdempotencyKey> {
        None
    }

    /// Handle an external event pushed to this trigger.
    ///
    /// Only called when [`accepts_events`](Self::accepts_events) returns
    /// `true`. The event is the typed `<Self::Source as TriggerSource>::Event`
    /// (e.g. `WebhookRequest` for `WebhookSource`).
    ///
    /// Returns a [`TriggerEventOutcome`] — `Skip` (filter out), `Emit(payload)`
    /// (start one workflow), or `EmitMany(payloads)` (fan-out).
    ///
    /// Default: returns [`Self::Error`] from the action's transport contract;
    /// triggers that do accept events MUST override.
    fn handle(
        &self,
        _ctx: &(impl crate::context::TriggerContext + ?Sized),
        _event: <Self::Source as TriggerSource>::Event,
    ) -> impl std::future::Future<Output = Result<TriggerEventOutcome, Self::Error>> + Send {
        async {
            // Default body: triggers that don't accept events should never
            // have this called. Engine checks `accepts_events()` first; this
            // path is a defensive guard.
            //
            // We can't construct a Self::Error here without knowing its
            // shape — convention is that pushed-event triggers override
            // accepts_events()=true AND override handle(). Implementations
            // that opt into events but forget to override handle() will
            // hit `unimplemented!()`; this matches Tech Spec §2.2.3
            // expected-author-discipline contract.
            unimplemented!(
                "TriggerAction::handle: trigger reports accepts_events=true \
                 but did not override handle(); see Tech Spec §2.2.3"
            )
        }
    }
}
```

> **Why `unimplemented!()` rather than a typed default error.** Per Tech Spec §2.2.3 the default-body contract is "triggers that opt into events override `handle()`; the default body is a defensive guard not a happy path." Returning a typed error from the default body would require `Self::Error: From<&'static str>` or similar, breaking the trait's freedom to use any `std::error::Error`. The TriggerHandler dyn layer (which IS bound to `ActionError`) provides the production-safe default — see Stage 4 Task 4.4.

- [ ] **Step 2: Verify `cargo check -p nebula-action` fails on consumers**

```bash
cargo check -p nebula-action 2>&1 | head -40
```

Expected: a wave of compile errors from `webhook.rs`, `poll.rs`, `handler.rs` test stubs, and downstream tests, because they all `impl TriggerAction for X` without the new `Source`/`Error`/`metadata` items. **This is the desired state** — Stage 3 Task 3.3, Stage 6, and Stage 7 fix the consumers.

- [ ] **Step 3: Commit (intentionally broken intermediate state — flagged in message)**

```bash
git add crates/action/src/trigger/mod.rs
git commit -m "feat(action)!: reshape TriggerAction trait per Tech Spec §2.2.3 (П1 / Q6 + Q7 R3 + Q8 F2) [BROKEN INTERMEDIATE]

Adds Source/Error associated types, accepts_events/idempotency_key default
methods, typed handle method. Drops `: Action` super-bound in favor of
inline Send/Sync/'static + metadata fn (mirrors spec §2.2.3 line 234).

THIS COMMIT INTENTIONALLY LEAVES THE WORKSPACE BROKEN — webhook.rs,
poll.rs, dyn handler stubs, and downstream tests do not yet match.
Stage 3 Tasks 3.3-3.4 + Stage 6 + Stage 7 close the wave.

Refs: docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md"
```

### Task 3.3 — Adjust `crates/action/src/trigger/mod.rs` self-tests

The flat `trigger.rs` file (now `trigger/mod.rs`) had a `MockTriggerAction` test impl at line ~518. After Task 3.2 it no longer compiles. Update it.

**Files:**
- Modify: `crates/action/src/trigger/mod.rs` (the existing `mod tests` block)

- [ ] **Step 1: Add a minimal `TestSource`**

Inside the existing `#[cfg(test)] mod tests { ... }` block, add near the top:

```rust
use crate::TriggerSource;

struct TestSource;
impl TriggerSource for TestSource {
    type Event = serde_json::Value;
}
```

- [ ] **Step 2: Update `MockTriggerAction` impl**

Replace the existing `impl TriggerAction for MockTriggerAction { ... }` block with:

```rust
impl TriggerAction for MockTriggerAction {
    type Source = TestSource;
    type Error = ActionError;

    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }

    async fn start(
        &self,
        _ctx: &(impl crate::context::TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        Ok(())
    }

    async fn stop(
        &self,
        _ctx: &(impl crate::context::TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        Ok(())
    }
}
```

(Note — `MockTriggerAction` already had a `metadata: ActionMetadata` field per the pre-existing structure; if not, add it.)

- [ ] **Step 3: Verify trigger.rs self-tests compile**

```bash
cargo check -p nebula-action --tests --lib --no-deps 2>&1 | grep "src/trigger" | head
```

Expected: no errors from `src/trigger/mod.rs` (the broader workspace still has errors from other files — fixed in Stage 6/7).

- [ ] **Step 4: Commit**

```bash
git add crates/action/src/trigger/mod.rs
git commit -m "test(action): update MockTriggerAction to new shape (П1)

Inline TestSource: TriggerSource. Adds metadata() impl. Drops the
`: Action` super-bound expectations.

Refs: docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md"
```

### Task 3.4 — Activate the compile-fail probe + record stderr

**Files:**
- Modify: `crates/action/tests/probes/missing_trigger_source.stderr` (placeholder → real)

- [ ] **Step 1: Run the probe in capture mode**

```bash
TRYBUILD=overwrite cargo nextest run -p nebula-action --test trigger_source_required_compile_fail --profile ci --no-tests=pass
```

Expected: trybuild writes the actual rustc stderr to `tests/probes/missing_trigger_source.stderr`. Inspect — the expected line contains "not all trait items implemented" and lists `Source`.

- [ ] **Step 2: Re-run without overwrite**

```bash
cargo nextest run -p nebula-action --test trigger_source_required_compile_fail --profile ci --no-tests=pass
```

Expected: PASS.

- [ ] **Step 3: Commit captured stderr**

```bash
git add crates/action/tests/probes/missing_trigger_source.stderr
git commit -m "test(action): record TriggerAction-without-Source compile-fail stderr (П1)

Probe now actively enforces Tech Spec §2.2.3 line 393 — adding
TriggerAction impls without 'type Source' fails E0046 'not all trait
items implemented'.

Refs: docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md"
```

---

## Stage 4 — `*Handler` traits → `#[async_trait]` (Q1 / ADR-0024)

Per Tech Spec §15.9 + ADR-0024 §1+§4 — the four dyn-consumed handler traits flip from hand-authored HRTB `Pin<Box<dyn Future + Send + 'a>>` to `#[async_trait]`.

### Task 4.1 — Add `async-trait` dependency

**Files:** `crates/action/Cargo.toml`

- [ ] **Step 1: Extend `[dependencies]`**

```toml
[dependencies]
# ... existing deps ...
async-trait = "0.1"
```

Verify:

```bash
cargo check -p nebula-action 2>&1 | head -5
```

(Errors from Stage 3 still present — that's expected. Just confirm `async-trait` itself resolves.)

- [ ] **Step 2: Commit**

```bash
git add crates/action/Cargo.toml
git commit -m "build(action): add async-trait dep for *Handler migration (П1 / Q1)

Per ADR-0024 §1+§4 — the four dyn-consumed *Handler traits flip to
#[async_trait]. Stages 4.2-4.5 apply per-trait.

Refs: docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md"
```

### Task 4.2 — `StatelessHandler` → `#[async_trait]`

**Files:** `crates/action/src/stateless.rs` (the `StatelessHandler` trait + adapter)

- [ ] **Step 1: Replace the trait block**

Locate the existing `pub trait StatelessHandler { ... }`. Replace with:

```rust
/// Stateless handler — JSON-erased one-shot execution contract.
///
/// The engine dispatches every `StatelessAction` through this `dyn` trait
/// (wrapped by [`StatelessActionAdapter`]). For typed authoring, write
/// `impl StatelessAction` and let the adapter bridge to JSON.
///
/// # Errors
///
/// Returns [`ActionError`] on validation, retryable, or fatal failure.
#[async_trait::async_trait]
pub trait StatelessHandler: Send + Sync + 'static {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Execute one-shot with JSON input.
    async fn execute(
        &self,
        input: Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}
```

- [ ] **Step 2: Update the existing adapter impl**

The `StatelessActionAdapter` currently implements `StatelessHandler::execute` with the HRTB Pin<Box> shape. Replace with the `#[async_trait]` form:

```rust
#[async_trait::async_trait]
impl<A> StatelessHandler for StatelessActionAdapter<A>
where
    A: StatelessAction + Send + Sync + 'static,
    A::Input: serde::de::DeserializeOwned + Send + Sync,
    A::Output: serde::Serialize + Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }

    async fn execute(
        &self,
        input: Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let typed_input: A::Input = serde_json::from_value(input).map_err(|e| {
            ActionError::validation(ValidationReason::Schema, format!("invalid input: {e}"))
        })?;
        let result = self.action.execute(typed_input, ctx).await?;
        let mapped = result.try_map_output(|output| serde_json::to_value(&output))?;
        Ok(mapped)
    }
}
```

> **Note:** the adapter body logic above is the same as the existing one — the change is purely the wrapper-attribute + signature simplification (no HRTB lifetimes).

- [ ] **Step 3: Verify**

```bash
cargo check -p nebula-action 2>&1 | grep "stateless.rs" | head
```

Expected: no errors specific to `stateless.rs` (other files still broken — closed in later tasks).

- [ ] **Step 4: Commit**

```bash
git add crates/action/src/stateless.rs
git commit -m "feat(action)!: StatelessHandler -> #[async_trait] (П1 / Q1)

Per ADR-0024 §1+§4 — dyn-consumed handler traits use #[async_trait]
to drop the HRTB boilerplate. Public adapter impl auto-translated.

Refs: docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md"
```

### Task 4.3 — `StatefulHandler` → `#[async_trait]`

**Files:** `crates/action/src/stateful.rs`

- [ ] **Step 1: Replace the trait block**

```rust
#[async_trait::async_trait]
pub trait StatefulHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;

    fn init_state(&self) -> Result<Value, ActionError>;

    fn migrate_state(&self, _old: Value) -> Option<Value> {
        None
    }

    async fn execute(
        &self,
        input: &Value,
        state: &mut Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}
```

- [ ] **Step 2: Update `StatefulActionAdapter` impl**

Replace the HRTB Pin<Box> bodies on the adapter impl with the `#[async_trait]` form (same logic, simpler signature). Pattern as in Task 4.2 Step 2.

- [ ] **Step 3: Verify + commit**

```bash
cargo check -p nebula-action 2>&1 | grep "stateful.rs" | head
git add crates/action/src/stateful.rs
git commit -m "feat(action)!: StatefulHandler -> #[async_trait] (П1 / Q1)"
```

### Task 4.4 — `TriggerHandler` → `#[async_trait]`

**Files:** `crates/action/src/trigger/mod.rs`

The trigger handler ALSO carries the `accepts_events` + `handle_event` pair (per current code lines 355-389). Migration:

- [ ] **Step 1: Replace the trait block**

```rust
#[async_trait::async_trait]
pub trait TriggerHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;

    async fn start(&self, ctx: &dyn TriggerContext) -> Result<(), ActionError>;
    async fn stop(&self, ctx: &dyn TriggerContext) -> Result<(), ActionError>;

    /// Whether this trigger accepts externally pushed events.
    fn accepts_events(&self) -> bool {
        false
    }

    /// Handle an external event pushed to this trigger.
    /// Default body returns Fatal — triggers that opt into events override.
    async fn handle_event(
        &self,
        event: TriggerEvent,
        ctx: &dyn TriggerContext,
    ) -> Result<TriggerEventOutcome, ActionError> {
        let _ = (event, ctx);
        Err(ActionError::fatal(
            "trigger does not accept external events",
        ))
    }
}
```

- [ ] **Step 2: Update `TriggerActionAdapter` impl**

The existing adapter delegates `start`/`stop` to the typed `TriggerAction::start`/`TriggerAction::stop`. Now also wires `accepts_events` and `handle_event` (the latter downcasts the erased payload to `<A::Source as TriggerSource>::Event`):

```rust
#[async_trait::async_trait]
impl<A> TriggerHandler for TriggerActionAdapter<A>
where
    A: TriggerAction + Send + Sync + 'static,
    <A::Source as TriggerSource>::Event: Send + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    async fn start(&self, ctx: &dyn TriggerContext) -> Result<(), ActionError> {
        self.action.start(ctx).await.map_err(|e| {
            ActionError::fatal(format!("trigger start failed: {e}"))
        })
    }

    async fn stop(&self, ctx: &dyn TriggerContext) -> Result<(), ActionError> {
        self.action.stop(ctx).await.map_err(|e| {
            ActionError::fatal(format!("trigger stop failed: {e}"))
        })
    }

    fn accepts_events(&self) -> bool {
        self.action.accepts_events()
    }

    async fn handle_event(
        &self,
        event: TriggerEvent,
        ctx: &dyn TriggerContext,
    ) -> Result<TriggerEventOutcome, ActionError> {
        let typed_event: <A::Source as TriggerSource>::Event =
            event.downcast().map_err(|err_payload| {
                ActionError::fatal(format!(
                    "trigger event type mismatch: expected {}, got payload of {}",
                    std::any::type_name::<<A::Source as TriggerSource>::Event>(),
                    err_payload.payload_type_name(),
                ))
            })?;

        self.action
            .handle(ctx, typed_event)
            .await
            .map_err(|e| ActionError::fatal(format!("trigger handle failed: {e}")))
    }
}
```

> **Note:** `TriggerEvent::downcast` already exists in `trigger/mod.rs` (the pre-rename `trigger.rs`) — verify its return signature (likely `Result<T, TriggerEvent>` or similar). Adjust the `err_payload.payload_type_name()` call to whatever accessor the existing code provides; if the accessor is internal-only, expose it as `pub(crate)` here in the same commit.

- [ ] **Step 3: Verify + commit**

```bash
cargo check -p nebula-action 2>&1 | grep "src/trigger" | head
git add crates/action/src/trigger/mod.rs
git commit -m "feat(action)!: TriggerHandler -> #[async_trait] + wire typed handle (П1 / Q1 + Q7 R3)"
```

### Task 4.5 — `ResourceHandler` → `#[async_trait]`

**Files:** `crates/action/src/resource.rs`

- [ ] **Step 1: Replace the trait block**

```rust
#[async_trait::async_trait]
pub trait ResourceHandler: Send + Sync + 'static {
    fn metadata(&self) -> &ActionMetadata;

    async fn configure(
        &self,
        config: Value,
        ctx: &dyn ActionContext,
    ) -> Result<Box<dyn std::any::Any + Send + Sync>, ActionError>;

    async fn cleanup(
        &self,
        instance: Box<dyn std::any::Any + Send + Sync>,
        ctx: &dyn ActionContext,
    ) -> Result<(), ActionError>;
}
```

- [ ] **Step 2: Update `ResourceActionAdapter` impl**

```rust
#[async_trait::async_trait]
impl<A> ResourceHandler for ResourceActionAdapter<A>
where
    A: ResourceAction + Send + Sync + 'static,
    A::Resource: Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }

    async fn configure(
        &self,
        _config: Value,
        ctx: &dyn ActionContext,
    ) -> Result<Box<dyn std::any::Any + Send + Sync>, ActionError> {
        let resource = self.action.configure(ctx).await?;
        Ok(Box::new(resource))
    }

    async fn cleanup(
        &self,
        instance: Box<dyn std::any::Any + Send + Sync>,
        ctx: &dyn ActionContext,
    ) -> Result<(), ActionError> {
        let typed: Box<A::Resource> = instance.downcast().map_err(|_| {
            ActionError::fatal(format!(
                "resource handler cleanup downcast failed: expected {}",
                std::any::type_name::<A::Resource>(),
            ))
        })?;
        self.action.cleanup(*typed, ctx).await
    }
}
```

- [ ] **Step 3: Verify + commit**

```bash
cargo check -p nebula-action 2>&1 | grep "src/resource" | head
git add crates/action/src/resource.rs
git commit -m "feat(action)!: ResourceHandler -> #[async_trait] (П1 / Q1)"
```

### Task 4.6 — Update test stubs in `handler.rs`

The 4 test stubs at `crates/action/src/handler.rs:139-269` (`TestStatelessHandler`, `TestStatefulHandler`, `TestTriggerHandler`, `TestResourceHandler`) all use the HRTB Pin<Box> shape. Migrate.

**Files:** `crates/action/src/handler.rs` (test module only)

- [ ] **Step 1: Rewrite each test stub**

Replace each stub's HRTB-style impl with the `#[async_trait]` form. Pattern (e.g. for `TestStatelessHandler`):

```rust
#[async_trait::async_trait]
impl StatelessHandler for TestStatelessHandler {
    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }

    async fn execute(
        &self,
        input: Value,
        _ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        Ok(ActionResult::success(input))
    }
}
```

Apply the same shape to `TestStatefulHandler`, `TestTriggerHandler`, `TestResourceHandler`.

For `TestTriggerHandler` — also remove the `'life0/'life1/'a` HRTB; do NOT add `accepts_events` / `handle_event` overrides (defaults are enough for the dyn-compat smoke test).

- [ ] **Step 2: Verify**

```bash
cargo nextest run -p nebula-action --lib --profile ci --no-tests=pass 2>&1 | tail -20
```

Expected: PASS for `handler::tests::*` (4 dyn-compat smokes + metadata-delegation + variant-checks + Debug = 8 tests).

- [ ] **Step 3: Commit**

```bash
git add crates/action/src/handler.rs
git commit -m "test(action): migrate handler dyn-compat stubs to #[async_trait] (П1 / Q1)"
```

---

## Stage 5 — `ActionMetadata::max_concurrent`

Per Tech Spec §15.12 F9 — engine-side dispatch throttle hint per action.

### Task 5.1 — Add the field + smoke test

**Files:**
- Modify: `crates/action/src/metadata.rs`
- Create: `crates/action/tests/metadata_max_concurrent_smoke.rs`

- [ ] **Step 1: Write the failing smoke test**

`crates/action/tests/metadata_max_concurrent_smoke.rs`:

```rust
//! Smoke tests for ActionMetadata::max_concurrent (Q8 F9).

use std::num::NonZeroU32;

use nebula_action::ActionMetadata;
use nebula_core::ActionKey;

fn meta() -> ActionMetadata {
    ActionMetadata::new(
        ActionKey::new("test.maxc").expect("valid key"),
        "test",
        "max_concurrent smoke",
    )
}

#[test]
fn default_is_none() {
    let m = meta();
    assert_eq!(m.max_concurrent, None);
}

#[test]
fn round_trips_through_json() {
    let mut m = meta();
    m.max_concurrent = Some(NonZeroU32::new(4).unwrap());
    let s = serde_json::to_string(&m).expect("serialize");
    let back: ActionMetadata = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(back.max_concurrent, Some(NonZeroU32::new(4).unwrap()));
}

#[test]
fn omits_when_none() {
    // None should not appear in serialized form (per #[serde(skip_serializing_if = "Option::is_none")]).
    let m = meta();
    let s = serde_json::to_string(&m).expect("serialize");
    assert!(!s.contains("max_concurrent"), "serialized form should omit None field, got: {s}");
}

#[test]
fn deserializes_when_field_absent() {
    // Older metadata (saved before F9 landed) MUST still deserialize.
    let json = r#"{"key":"test.maxc","name":"test","description":"x","schema":null,"version":"0.1.0","inputs":[],"outputs":[],"isolation_level":"none","category":"data"}"#;
    let _: ActionMetadata = serde_json::from_str(json).expect("backwards-compat deserialize");
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p nebula-action --test metadata_max_concurrent_smoke --profile ci --no-tests=pass
```

Expected: **FAIL** with "no field `max_concurrent`".

- [ ] **Step 3: Add the field**

`crates/action/src/metadata.rs` — modify the `ActionMetadata` struct (around line 96-118):

```rust
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionMetadata {
    /// Shared catalog prefix — see [`BaseMetadata`].
    #[serde(flatten)]
    pub base: BaseMetadata<ActionKey>,
    /// Input ports this action accepts.
    pub inputs: Vec<InputPort>,
    /// Output ports this action produces.
    pub outputs: Vec<OutputPort>,
    /// Isolation level for this action's execution.
    pub isolation_level: IsolationLevel,
    /// Broad category of this action.
    #[serde(default)]
    pub category: ActionCategory,
    /// Per-action concurrency throttle hint.
    ///
    /// `None` (default) — engine-global throttle still applies, but no
    /// per-action limit. `Some(n)` — at most `n` in-flight executions of
    /// this action across the engine.
    ///
    /// Per Tech Spec §15.12 F9 + PRODUCT_CANON §11 backpressure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrent: Option<core::num::NonZeroU32>,
}
```

Modify `ActionMetadata::new` (around line 127-138) to initialize `max_concurrent: None`:

```rust
impl ActionMetadata {
    pub fn new(key: ActionKey, name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            base: BaseMetadata::new(key, name, description, ValidSchema::empty()),
            inputs: port::default_input_ports(),
            outputs: port::default_output_ports(),
            isolation_level: IsolationLevel::None,
            category: ActionCategory::Data,
            max_concurrent: None,
        }
    }

    // ... rest of impl unchanged ...
}
```

- [ ] **Step 4: Add a setter (idiomatic API)**

Inside `impl ActionMetadata`:

```rust
/// Set the per-action concurrency throttle.
///
/// Per Tech Spec §15.12 F9. Builder-style; chainable.
#[must_use]
pub fn with_max_concurrent(mut self, n: core::num::NonZeroU32) -> Self {
    self.max_concurrent = Some(n);
    self
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo nextest run -p nebula-action --test metadata_max_concurrent_smoke --profile ci --no-tests=pass
```

Expected: PASS — 4 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/action/src/metadata.rs crates/action/tests/metadata_max_concurrent_smoke.rs
git commit -m "feat(action): ActionMetadata::max_concurrent field (П1 / Q8 F9)

Per-action concurrency throttle hint; engine-side dispatch consumes.
None default + skip_serializing_if preserves backwards-compat
deserialization of pre-F9 metadata blobs.

Refs: docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md"
```

---

## Stage 6 — Webhook + Poll trigger impls migrate to typed `Source`

`WebhookAction` and `PollAction` are the two existing implementors of `TriggerAction` in tree (community plugins are out of П1 scope; their migration lands as П6 codemod work).

### Task 6.1 — `WebhookSource` definition

**Files:**
- Create: `crates/action/src/webhook/source.rs`
- Modify: `crates/action/src/webhook.rs` → `crates/action/src/webhook/mod.rs` (rename to host source)

> Same `git mv` directory-promotion pattern as Stage 2 Task 2.1 Step 1.

- [ ] **Step 1: Promote `webhook.rs` → `webhook/mod.rs`**

```bash
mkdir crates/action/src/webhook
git mv crates/action/src/webhook.rs crates/action/src/webhook/mod.rs
```

- [ ] **Step 2: Write `WebhookSource`**

Create `crates/action/src/webhook/source.rs`:

```rust
//! [`WebhookSource`] — `TriggerSource` for HTTP webhook trigger family.

use crate::trigger::TriggerSource;
use crate::webhook::WebhookRequest;

/// Trigger event source for HTTP webhooks.
///
/// Implementations of [`WebhookAction`](crate::WebhookAction) must use
/// `type Source = WebhookSource;` — the `<Self::Source as TriggerSource>::Event`
/// projection then resolves to [`WebhookRequest`], which carries the
/// transport-specific body, headers, signature outcome, and method.
#[derive(Debug, Clone, Copy)]
pub struct WebhookSource;

impl TriggerSource for WebhookSource {
    type Event = WebhookRequest;
}
```

- [ ] **Step 3: Wire submodule + re-export**

`crates/action/src/webhook/mod.rs` — near the top (post-rename):

```rust
mod source;
pub use source::WebhookSource;
```

`crates/action/src/lib.rs` — extend the existing `pub use webhook::{...}` block to add `WebhookSource`.

- [ ] **Step 4: Smoke verify**

```bash
cargo check -p nebula-action 2>&1 | grep "webhook" | head
```

(Errors expected from `WebhookAction` trait body which still does not have `type Source`; the import itself should resolve.)

- [ ] **Step 5: Commit**

```bash
git add crates/action/src/webhook/ crates/action/src/lib.rs
git commit -m "feat(action): introduce WebhookSource: TriggerSource (П1 / Stage 6)"
```

### Task 6.2 — Migrate `WebhookAction` impl

The existing `WebhookAction` trait (in `webhook/mod.rs`) currently implements `TriggerAction` indirectly via `WebhookTriggerAdapter`. With the new typed `TriggerAction::handle(event: <Self::Source as TriggerSource>::Event)`, the adapter no longer downcasts `TriggerEvent` — the engine routes the typed `WebhookRequest` directly to `WebhookAction::on_request` (or whatever the existing entry method is named).

**Files:** `crates/action/src/webhook/mod.rs`

- [ ] **Step 1: Update `WebhookTriggerAdapter` impl of `TriggerAction`**

Locate `impl<A: WebhookAction> TriggerAction for WebhookTriggerAdapter<A>`. Replace with:

```rust
impl<A: WebhookAction + Send + Sync + 'static> TriggerAction for WebhookTriggerAdapter<A> {
    type Source = WebhookSource;
    type Error = ActionError;

    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    async fn start(
        &self,
        ctx: &(impl crate::context::TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        // Existing setup-and-return body; if the existing adapter has
        // bespoke registration logic, keep it.
        Ok(())
    }

    async fn stop(
        &self,
        ctx: &(impl crate::context::TriggerContext + ?Sized),
    ) -> Result<(), ActionError> {
        // Existing teardown body.
        Ok(())
    }

    fn accepts_events(&self) -> bool {
        true
    }

    fn idempotency_key(&self, event: &WebhookRequest) -> Option<crate::IdempotencyKey> {
        // Webhook delivery id from the X-Delivery-ID header (or transport-supplied id).
        // If the adapter has a delivery_id accessor, use it; otherwise None.
        event.delivery_id().map(crate::IdempotencyKey::new)
    }

    async fn handle(
        &self,
        ctx: &(impl crate::context::TriggerContext + ?Sized),
        event: WebhookRequest,
    ) -> Result<TriggerEventOutcome, ActionError> {
        match self.action.on_request(ctx, event).await? {
            WebhookResponse::Accept(outcome) => Ok(outcome),
            WebhookResponse::Reject { outcome, .. } => Ok(outcome),
            // Existing variant arms — preserve.
        }
    }
}
```

> **Editor note:** the existing `WebhookAction::on_request` signature may already match `(&self, ctx, request) -> Result<WebhookResponse, ActionError>` per `webhook.rs` lines 458-517. If so, the body above is a literal port of the old `TriggerHandler::handle_event` downcast logic, with the downcast eliminated (engine routes typed `WebhookRequest` directly). If `on_request` returns a different shape, adapt the match arms.

> **`WebhookRequest::delivery_id`** may not exist as an accessor today. If absent, add a `pub fn delivery_id(&self) -> Option<&str>` returning the X-Delivery-ID header value (or whatever the existing dedup id mechanism uses). Sub-step 1.5 below.

- [ ] **Step 1.5: If `WebhookRequest::delivery_id` missing — add accessor**

In `webhook/mod.rs` near the existing `impl WebhookRequest`:

```rust
impl WebhookRequest {
    /// Stable per-delivery id for idempotency.
    /// Reads `X-Delivery-ID` (or transport-equivalent) header.
    pub fn delivery_id(&self) -> Option<&str> {
        self.headers
            .get("x-delivery-id")
            .or_else(|| self.headers.get("X-Delivery-ID"))
            .map(String::as_str)
    }
}
```

(Adjust to whatever header-storage shape `WebhookRequest` already exposes.)

- [ ] **Step 2: Verify**

```bash
cargo check -p nebula-action 2>&1 | grep "webhook" | head
```

Expected: webhook errors shrink to zero.

- [ ] **Step 3: Commit**

```bash
git add crates/action/src/webhook/mod.rs
git commit -m "feat(action)!: WebhookAction adapter -> typed TriggerAction (П1 / Stage 6)

Adapter now uses Source = WebhookSource directly; engine routes typed
WebhookRequest to handle() without downcast. idempotency_key()
overrides default to return X-Delivery-ID.

Refs: docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md"
```

### Task 6.3 — `PollSource` + `PollAction` migration

Same shape as Tasks 6.1 + 6.2. `PollSource: TriggerSource<Event = PollEvent>` (or whatever the existing internal event envelope is named — likely `PollOutcome` or a dedicated `PollEvent`).

**Files:**
- Create: `crates/action/src/poll/source.rs`
- Modify: rename `crates/action/src/poll.rs` → `crates/action/src/poll/mod.rs`

- [ ] **Step 1: Promote `poll.rs` → `poll/mod.rs`** (same `git mv` pattern)

- [ ] **Step 2: Identify the typed event envelope**

Inspect `poll/mod.rs` (post-rename). The existing `PollAction::execute` likely returns `PollResult<...>` after consuming an internal cycle; the cycle envelope reaching the trigger is the natural `PollSource::Event`. If no dedicated type exists, the typed event is a `PollCycle` struct carrying `cursor: PollCursor` + a tick timestamp — define it inline if needed:

```rust
#[derive(Debug, Clone)]
pub struct PollCycle {
    pub cursor: PollCursor,
    pub tick: std::time::SystemTime,
}
```

- [ ] **Step 3: Define `PollSource`**

```rust
#[derive(Debug, Clone, Copy)]
pub struct PollSource;

impl crate::trigger::TriggerSource for PollSource {
    type Event = PollCycle;
}
```

- [ ] **Step 4: Migrate `PollTriggerAdapter` impl of `TriggerAction`**

Same pattern as Task 6.2. `accepts_events() = false` for poll triggers (engine drives them via `start` loop, not push-events) — therefore `handle()` falls back to default `unimplemented!` and is never called. Adapter still needs `Source = PollSource` to satisfy the trait.

- [ ] **Step 5: Verify + commit**

```bash
cargo check -p nebula-action 2>&1 | grep "poll" | head
git add crates/action/src/poll/ crates/action/src/lib.rs
git commit -m "feat(action)!: PollAction adapter -> typed TriggerAction (П1 / Stage 6)"
```

### Task 6.4 — Update `dx_*.rs` test fixtures

**Files:**
- `crates/action/tests/dx_webhook.rs`
- `crates/action/tests/dx_poll.rs`
- `crates/action/tests/dx_control.rs`
- `crates/action/tests/execution_integration.rs`
- `crates/action/tests/resource_roundtrip.rs`

Each test that has `impl TriggerAction for X` or implements one of the four `*Handler` traits needs the new shape.

- [ ] **Step 1: Per-file pattern — add `type Source = TestSource;` + `type Error = ActionError;` + `metadata` fn for each TriggerAction impl**

Pattern:

```rust
struct TestSource;
impl nebula_action::TriggerSource for TestSource {
    type Event = serde_json::Value;
}

impl TriggerAction for MyTestTrigger {
    type Source = TestSource;
    type Error = ActionError;

    fn metadata(&self) -> &ActionMetadata {
        &self.meta
    }

    async fn start(...) -> Result<(), ActionError> { ... }
    async fn stop(...) -> Result<(), ActionError> { ... }
}
```

- [ ] **Step 2: Per `*Handler` impl in tests — flip to `#[async_trait]`** (same pattern as Stage 4 Task 4.6).

- [ ] **Step 3: Verify**

```bash
cargo nextest run -p nebula-action --profile ci --no-tests=pass
```

Expected: all action-package tests PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/action/tests/
git commit -m "test(action): update dx_* + integration fixtures to new TriggerAction shape (П1)"
```

---

## Stage 7 — Reverse-deps T4 codemod (compile-fix)

Each downstream crate consumes `*Handler` traits — after the `#[async_trait]` migration their hand-authored HRTB impls (where present) need flattening, and `WebhookAction`/`TriggerAction` consumers need `WebhookSource`/typed-event awareness.

### Task 7.1 — `nebula-sdk` runtime

**Files:** `crates/sdk/src/runtime.rs`

- [ ] **Step 1: Audit Handler call sites**

```bash
grep -n "StatelessHandler\|StatefulHandler\|TriggerHandler\|ResourceHandler" crates/sdk/src/runtime.rs
```

- [ ] **Step 2: Update each impl / call site**

For each `impl XxxHandler for Y` that uses HRTB Pin<Box> shape — flip to `#[async_trait]`. For each call site that constructs `Box::pin(async { ... })` to satisfy a Handler signature — replace with `async fn` body or `Box::new(...)` per the new trait shape.

- [ ] **Step 3: Verify**

```bash
cargo check -p nebula-sdk
cargo nextest run -p nebula-sdk --profile ci --no-tests=pass
```

- [ ] **Step 4: Commit**

```bash
git add crates/sdk/src/runtime.rs
git commit -m "refactor(sdk): align Handler call sites to #[async_trait] shape (П1 / T4)"
```

### Task 7.2 — `nebula-sandbox`

**Files:**
- `crates/sandbox/src/handler.rs`
- `crates/sandbox/src/remote_action.rs`
- `crates/sandbox/src/discovery.rs`

- [ ] **Step 1: Per file, audit + migrate the same way as Task 7.1.**

- [ ] **Step 2: Verify + commit**

```bash
cargo check -p nebula-sandbox
cargo nextest run -p nebula-sandbox --profile ci --no-tests=pass
git add crates/sandbox/
git commit -m "refactor(sandbox): align Handler bridges to #[async_trait] shape (П1 / T4)"
```

### Task 7.3 — `nebula-engine`

**Files:**
- `crates/engine/src/runtime/runtime.rs`
- `crates/engine/src/engine.rs`

Same pattern. Engine is the heaviest consumer per Phase 0 audit (27+ import sites of action) — most are read-only consumers where the `#[async_trait]` migration is transparent (call sites only `.await` the future). The non-trivial ones are `WebhookAction` / `TriggerAction` invocations that must now thread typed events.

- [ ] **Step 1: Audit + migrate**

```bash
grep -rn "TriggerEvent\|StatelessHandler\|TriggerHandler\|ResourceHandler" crates/engine/src/runtime crates/engine/src/engine.rs
```

For each `TriggerHandler::handle_event` call site — the dyn signature still takes `TriggerEvent` (since the dyn layer cannot carry type-level erasure differently). No change at engine call sites; the typed downcast lives inside `TriggerActionAdapter::handle_event` (Task 4.4 Step 2).

- [ ] **Step 2: Verify + commit**

```bash
cargo check -p nebula-engine
cargo nextest run -p nebula-engine --profile ci --no-tests=pass
git add crates/engine/
git commit -m "refactor(engine): align Handler invocations to #[async_trait] shape (П1 / T4)"
```

### Task 7.4 — `nebula-api` webhook consumers

**Files:**
- `crates/api/src/services/webhook/transport.rs`
- `crates/api/src/services/webhook/routing.rs`
- `crates/api/tests/webhook_transport_integration.rs`

API is the only crate that constructs `TriggerEvent` envelopes from incoming HTTP requests. With `WebhookAction::Source = WebhookSource`, the dyn-layer `TriggerEvent` payload MUST be a `Box<WebhookRequest>` (or compatible) — verify the transport layer does this correctly.

- [ ] **Step 1: Audit**

```bash
grep -n "TriggerEvent::new\|TriggerEvent {" crates/api/src/services/webhook/
```

- [ ] **Step 2: For each construction site — confirm payload type is `WebhookRequest`**

If construction sites pass `WebhookRequest` as payload (likely already the case): no logic change. If they pass `serde_json::Value` or a wrapper: refactor to pass typed `WebhookRequest` so `<WebhookSource as TriggerSource>::Event` downcast in the adapter (Task 4.4) succeeds.

- [ ] **Step 3: Verify + commit**

```bash
cargo check -p nebula-api
cargo nextest run -p nebula-api --profile ci --no-tests=pass
git add crates/api/
git commit -m "refactor(api): webhook transport passes typed WebhookRequest payload (П1 / T4)"
```

---

## Stage 8 — Final verification + concerns register update

### Task 8.1 — Full workspace gate

- [ ] **Step 1: Run the full local CI mirror**

```bash
cargo +nightly fmt --all -- --check
cargo clippy --workspace -- -D warnings
cargo nextest run --workspace --profile ci --no-tests=pass
```

Expected: PASS. If anything fails, halt and fix in-place — do NOT push П1 with red.

### Task 8.2 — Public-API surface delta

- [ ] **Step 1: Capture post-П1 snapshot**

```bash
cargo public-api --manifest-path crates/action/Cargo.toml > /tmp/action-post-p1.txt
```

- [ ] **Step 2: Diff against pre-П1**

```bash
diff -u /tmp/action-pre-p1.txt /tmp/action-post-p1.txt | head -120
```

Expected delta:
- ADDED: `IdempotencyKey`, `TriggerSource`, `WebhookSource`, `PollSource`, `ActionMetadata::max_concurrent`, `ActionMetadata::with_max_concurrent`
- CHANGED: `TriggerAction` (associated types Source / Error; new methods accepts_events / idempotency_key / handle; metadata moved from `: Action` super to inherent method); `*Handler` 4 traits (HRTB Pin<Box> dropped, `async_trait` shape).
- ADDED: `WebhookRequest::delivery_id` (if introduced in Task 6.2 Step 1.5)

If unexpected items appear, audit them — they may be unintended public exposure or accidental removal.

### Task 8.3 — Concerns register update

**Files:** (likely) `docs/tracking/nebula-action-concerns-register.md`

If a Phase-7 register exists per Strategy §6.4, mark the concerns closed by П1:
- Q6 lifecycle gap fix — RESOLVED (TriggerAction `start`/`stop` already had it; Q6 just confirmed; no register row to flip)
- Q7 R3 lifecycle slips on TriggerAction — RESOLVED via Stage 3
- Q7 R4-R5 dyn handler shape slips — RESOLVED via Stage 4
- Q8 F2 idempotency hook — RESOLVED via Stage 1 + Stage 3
- Q8 F9 max_concurrent — RESOLVED via Stage 5

Add a row referencing this plan and the П1 commit range.

If the register file does not exist (Phase 7 was skipped per the cascade summary), open `docs/tracking/cascade-queue.md` and update slot 1 / 2 trigger conditions to reflect "action П1 landed" — engine cascade slot 2 cluster-mode-placeholder bodies are now valid to plan.

- [ ] **Step 1: Edit the appropriate file + commit**

```bash
git add docs/tracking/
git commit -m "docs(tracking): action П1 landed — update concerns register / cascade-queue (П1)"
```

### Task 8.4 — Status flip + summary commit

- [ ] **Step 1: Update plan frontmatter status**

In `docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md` — flip the frontmatter:

```yaml
status: LANDED 2026-MM-DD (commit range <first-sha>..<last-sha>)
```

- [ ] **Step 2: Update Tech Spec status (per `feedback_active_dev_mode.md`)**

In `docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md` — add a footer line near the §0 status block:

```markdown
**Implementation status:** П1 LANDED 2026-MM-DD (trait shape scaffolding) per [`2026-04-27-nebula-action-p1-trait-shape.md`](../plans/2026-04-27-nebula-action-p1-trait-shape.md).
```

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md docs/superpowers/specs/2026-04-24-nebula-action-tech-spec.md
git commit -m "docs(action): П1 trait shape scaffolding LANDED (status flip)

- TriggerAction reshape (Source / Error / accepts_events / idempotency_key / handle)
  per Tech Spec §2.2.3 + Q6 + Q7 R3 + Q8 F2.
- *Handler #[async_trait] migration (Q1 / ADR-0024 §1+§4).
- ActionMetadata::max_concurrent (Q8 F9).
- IdempotencyKey + TriggerSource + WebhookSource + PollSource types.

П2 next: #[action] attribute macro + sealed DX + canon §3.5 revision.

Refs: docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md"
```

### Task 8.5 — Open the П1 PR

- [ ] **Step 1: Push the worktree branch + open PR**

```bash
git push -u origin <branch-name>
gh pr create --title "feat(action)!: П1 — trait shape scaffolding" --body "$(cat <<'EOF'
## Summary

- TriggerAction reshape: Source / Error / accepts_events / idempotency_key / handle (Tech Spec §2.2.3 + Q6 + Q7 R3 + Q8 F2).
- *Handler #[async_trait] migration (Q1 / ADR-0024 §1+§4).
- ActionMetadata::max_concurrent (Q8 F9).
- New types: IdempotencyKey, TriggerSource, WebhookSource, PollSource.

Mirrors resource-P1 / credential-P1 scaffolding pattern. No credential CP6 dependency. macro / sealed DX / security floor / engine wiring all deferred to П2-П5.

## Test plan

- [x] cargo +nightly fmt --all -- --check
- [x] cargo clippy --workspace -- -D warnings
- [x] cargo nextest run --workspace --profile ci --no-tests=pass
- [x] cargo public-api delta reviewed at /tmp/action-{pre,post}-p1.txt
- [x] trybuild compile-fail probe enforces TriggerAction Source-required invariant

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review (writing-plans skill checklist)

### 1. Spec coverage

| Spec item | П1 task |
|---|---|
| Q1 `*Handler` `#[async_trait]` (§15.9) | Stage 4 Tasks 4.1-4.6 |
| Q6 TriggerAction lifecycle (start/stop preserved) | Stage 3 Task 3.2 (verbatim retained) |
| Q7 R3 TriggerAction handle / accepts_events / TriggerEventOutcome | Stage 3 Task 3.2 + Stage 4 Task 4.4 |
| Q7 R4 ResourceHandler Box<dyn Any> envelope | Stage 4 Task 4.5 |
| Q7 R5 TriggerHandler envelope | Stage 4 Task 4.4 |
| Q8 F2 idempotency_key + IdempotencyKey type | Stage 1 + Stage 3 Task 3.2 |
| Q8 F9 ActionMetadata::max_concurrent | Stage 5 |
| `TriggerSource` typed event family (§2.2.3 line 230) | Stage 2 + Stage 6 |

| Spec item NOT in П1 | Phase |
|---|---|
| §3.7 cluster-mode placeholders | engine cascade slot 2 |
| §4 #[action] macro | П2 |
| §6 security floor | П3 |
| §10 codemod T1/T2/T5/T6 | П2-П6 |
| §11 sealed DX | П2 |
| §12 NodeDefinition::action_version (Q8 F12) | engine cascade |

### 2. Placeholder scan

No "TBD" / "implement later" / "fill in details" remain. Two soft notes:
- Stage 3 Task 3.4 records expected stderr only after Task 3.2 lands — unavoidable trybuild discipline.
- Task 6.3 mentions inferring `PollCycle` from existing `poll.rs` content; if no envelope exists, the inline definition is provided.

### 3. Type / method consistency

- `IdempotencyKey::new(impl Into<String>)` — used identically in Stage 1 + Stage 3 Task 3.2 default body + Stage 6 Task 6.2.
- `TriggerSource::Event` — used identically in Stage 2, Stage 3 Task 3.2 (`<Self::Source as TriggerSource>::Event`), Stage 6 Tasks 6.1/6.3.
- `*Handler` traits — `#[async_trait]` shape applied uniformly (Stages 4.2-4.5 + 4.6 + 7.1-7.4).
- `ActionMetadata::max_concurrent: Option<core::num::NonZeroU32>` — same path everywhere.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-27-nebula-action-p1-trait-shape.md`. Two execution options:

**1. Subagent-Driven (recommended)** — orchestrator dispatches a fresh subagent per Task; review between tasks; fast iteration. Use `superpowers:subagent-driven-development`.

**2. Inline Execution** — execute Stages in this session via `superpowers:executing-plans`; batch with checkpoints at Stage boundaries (after Stage 3, Stage 5, Stage 7).

Which approach?
