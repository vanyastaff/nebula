# nebula-resource П1 — Trait Shape Scaffolding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reshape `Resource` trait per ADR-0036 — replace `type Auth: AuthScheme` with `type Credential: Credential`, add `on_credential_refresh` + `on_credential_revoke` lifecycle hooks (default no-op), introduce `NoCredential` opt-out type, and migrate 29 in-tree call sites. Zero behavioral change — pure type-level scaffolding ahead of П2 (rotation L2).

**Architecture:** Two-crate change. `nebula-credential` gains `NoCredential` (a no-auth `Credential` impl with `type Scheme = ()`). `nebula-resource::Resource` swaps its credential associated-type, threads `&<Self::Credential as Credential>::Scheme` through `create()` and all `Manager::acquire_*` / runtime call paths, and gains two default-no-op rotation hooks matching the canonical `SchemeGuard<'a, _>` signature from credential Tech Spec §15.7. Reverse-index write path and dispatcher logic stay as `todo!()` (already are today) — П2's job. `OnCredentialRefresh<C>` parallel trait in `nebula-credential` is left in place, marked deprecated; П2 removes it once Manager-side dispatch lands on the new method.

**Tech Stack:** Rust 1.95 (workspace MSRV per ADR-0019), `nebula-credential` П1 primitives (`Credential`, `SchemeGuard`, `CredentialContext`, `CredentialId`), tokio, `nebula-error`, `nebula-schema`. Test harness: `cargo nextest` per `.github/workflows/test-matrix.yml`.

**Non-goals (explicitly deferred):**
- Reverse-index write at `Manager::register_*` — П2.
- `dispatch_rotation` parallel `join_all` + per-resource timeout — П2.
- `RotationConfig`, observability events `CredentialRotated` / `RotationPartialFailure` — П2.
- `manager.rs` 7-submodule split — П3.
- `set_phase_all_failed` for `DrainTimeoutPolicy::Abort` — П3.
- `Daemon` / `EventSource` extraction (ADR-0037) — П4.
- Doc rewrite (`api-reference.md`, `Architecture.md`, `events.md`) — П5.
- `warmup_pool` `Scheme::default()` ban (security amendment B-3) — П2 (must land with dispatcher to give pool a real warmup signature).

**Source documents:**
- ADR: [docs/adr/0036-resource-credential-adoption-auth-retirement.md](../../adr/0036-resource-credential-adoption-auth-retirement.md) — accepted, amended-in-place 2026-04-26 (cross-cascade R2 canonical CP5 form).
- Tech Spec §2.1, §2.2: [docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md](../specs/2026-04-24-nebula-resource-tech-spec.md).
- Strategy §4.1: [docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md](../specs/2026-04-24-nebula-resource-redesign-strategy.md).
- credential `OnCredentialRefresh` parallel trait (transitional): [crates/credential/src/secrets/scheme_guard.rs:194-300](../../../crates/credential/src/secrets/scheme_guard.rs).
- Concerns register: [docs/tracking/nebula-resource-concerns-register.md](../../tracking/nebula-resource-concerns-register.md) — P1 lands R-001 (Auth dead → Credential adoption), partial R-005 (warmup ban deferred to П2).

---

## File Structure

### New files

| File | Purpose |
|---|---|
| `crates/credential/src/no_credential.rs` | `NoCredential`, `NoCredentialState` impls |
| `crates/credential/tests/no_credential_smoke.rs` | Smoke tests: `Credential::project()` returns `()`, `resolve()` returns `Resolved(NoCredentialState)`, `KEY = "no_credential"` |

### Modified files

| File | Change |
|---|---|
| `crates/credential/src/lib.rs` | `pub use no_credential::{NoCredential, NoCredentialState};` |
| `crates/credential/src/secrets/scheme_guard.rs` | Mark `OnCredentialRefresh<C>` `#[deprecated(since = "0.1.0", note = "Use Resource::on_credential_refresh; removal in П2")]` |
| `crates/resource/src/resource.rs` | Trait reshape: drop `type Auth`, add `type Credential`, change `create` sig, add `on_credential_refresh` + `on_credential_revoke` |
| `crates/resource/src/lib.rs` | Re-export `NoCredential`, `SchemeGuard`, `CredentialContext`, `CredentialId` from `nebula_credential` |
| `crates/resource/src/manager.rs` | Update 5× `acquire_*<R>` signatures (`auth: &R::Auth` → `scheme: &<R::Credential as Credential>::Scheme`); update `warmup_pool` bound `R::Auth: Default` → `<R::Credential as Credential>::Scheme: Default` (interim — П2 removes) |
| `crates/resource/src/runtime/pool.rs` | 6× internal `auth: &R::Auth` parameters retyped; 1× `impl Resource for MockPool` migration |
| `crates/resource/src/runtime/resident.rs` | Internal sigs + 2× `impl Resource for {MockResident, HangingResident}` |
| `crates/resource/src/runtime/service.rs` | Internal sigs + 2× `impl Resource for {ClonedService, TrackedService}` |
| `crates/resource/src/runtime/daemon.rs` | Internal sigs + 2× `impl Resource for {FlakyDaemon, OneShotDaemon}` |
| `crates/resource/src/guard.rs` | 1× `impl Resource for DummyResource` |
| `crates/resource/tests/basic_integration.rs` | 14× test resource impls |
| `crates/resource/tests/dx_evaluation.rs` | 3× test resource impls |
| `crates/resource/tests/dx_audit.rs` | 3× test resource impls |
| `crates/resource/docs/README.md` | Line 90 example (`type Auth = ();` → `type Credential = NoCredential;`) |
| `crates/resource/docs/adapters.md` | Line 204 example |
| `crates/credential/README.md` | One-line entry under "Built-in types" pointing at `NoCredential` |
| `crates/resource/README.md` | One-line note: trait now uses `Credential` not `Auth`; `NoCredential` opt-out |

### Verification commands (used throughout the plan)

| Purpose | Command |
|---|---|
| Compile credential | `cargo check -p nebula-credential` |
| Compile resource | `cargo check -p nebula-resource` |
| Compile workspace | `cargo check --workspace` |
| Tests credential | `cargo nextest run -p nebula-credential --profile ci --no-tests=pass` |
| Tests resource | `cargo nextest run -p nebula-resource --profile ci --no-tests=pass` |
| Tests workspace | `cargo nextest run --workspace --profile ci --no-tests=pass` |
| Clippy | `cargo clippy --workspace -- -D warnings` |
| Format check | `cargo +nightly fmt --all -- --check` |

---

## Task 1: Create `NoCredential` type in `nebula-credential`

**Files:**
- Create: `crates/credential/src/no_credential.rs`
- Create: `crates/credential/tests/no_credential_smoke.rs`
- Modify: `crates/credential/src/lib.rs` (add `mod` + re-exports)

**Why first:** `nebula-resource::Resource` will reference `NoCredential` as the canonical opt-out via re-export (Task 2). Building it in credential first lets us migrate resource sites against a real type, not a stub.

- [ ] **Step 1: Write the failing smoke test**

`crates/credential/tests/no_credential_smoke.rs`:

```rust
//! Smoke tests for the `NoCredential` opt-out type.
//!
//! Verifies the basic `Credential` contract works for the no-auth case
//! used by `Resource` impls that don't need credential binding.

use nebula_credential::{
    AuthPattern, AuthScheme, Credential, CredentialContext, CredentialState, NoCredential,
    NoCredentialState, ResolveResult,
};
use nebula_schema::FieldValues;

#[test]
fn key_matches_spec() {
    assert_eq!(NoCredential::KEY, "no_credential");
}

#[test]
fn scheme_is_unit_with_noauth_pattern() {
    assert_eq!(<<NoCredential as Credential>::Scheme>::pattern(), AuthPattern::NoAuth);
}

#[test]
fn state_kind_matches_spec() {
    assert_eq!(NoCredentialState::KIND, "no_credential");
    assert_eq!(NoCredentialState::VERSION, 1);
}

#[test]
fn project_returns_unit_scheme() {
    let state = NoCredentialState;
    let _scheme: <NoCredential as Credential>::Scheme = NoCredential::project(&state);
    // Compiles iff Scheme = (); explicit assertion would be redundant.
}

#[tokio::test]
async fn resolve_returns_resolved_state() {
    let values = FieldValues::default();
    let ctx = CredentialContext::for_test("test-owner");
    let outcome = NoCredential::resolve(&values, &ctx)
        .await
        .expect("NoCredential::resolve never fails");
    assert!(matches!(outcome, ResolveResult::Resolved(NoCredentialState)));
}
```

- [ ] **Step 2: Run test to verify it fails (compile error)**

Run: `cargo nextest run -p nebula-credential --test no_credential_smoke --profile ci --no-tests=pass`
Expected: **FAIL** with `error[E0432]: unresolved import 'nebula_credential::NoCredential'`.

- [ ] **Step 3: Write `NoCredential` impl**

Create `crates/credential/src/no_credential.rs`:

```rust
//! `NoCredential` — idiomatic opt-out for resources without an authenticated binding.
//!
//! Per ADR-0036, `Resource` impls that don't need credential material write
//! `type Credential = NoCredential;`. The associated `Scheme = ()` already
//! implements [`AuthScheme`] (with `pattern() = AuthPattern::NoAuth`) and
//! `PublicScheme` in `nebula_core::auth`, so no secret material flows.
//!
//! This is the credential-side mirror of the previous `type Auth = ();` pattern
//! retired in the П1 trait reshape.

use std::future::Future;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{
    Credential, CredentialContext, CredentialError, CredentialMetadata, CredentialState,
    ResolveResult,
};
use nebula_schema::FieldValues;

/// State for [`NoCredential`]. Carries no data — it is the type-level marker
/// the credential subsystem hands resources that don't bind any auth material.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NoCredentialState;

impl Zeroize for NoCredentialState {
    fn zeroize(&mut self) {
        // No sensitive data to zeroize — this is the no-auth marker.
    }
}

impl ZeroizeOnDrop for NoCredentialState {}

impl CredentialState for NoCredentialState {
    const KIND: &'static str = "no_credential";
    const VERSION: u32 = 1;
}

/// Opt-out [`Credential`] for resources without an authenticated binding.
///
/// Replaces the legacy `type Auth = ();` pattern from before the П1 trait
/// reshape. Use as `type Credential = NoCredential;` on any `Resource` impl
/// that does not need credential material in `create()`.
///
/// # Examples
///
/// ```ignore
/// use nebula_credential::NoCredential;
/// use nebula_resource::Resource;
///
/// struct PingResource;
///
/// impl Resource for PingResource {
///     type Config = ();
///     type Runtime = ();
///     type Lease = ();
///     type Error = std::io::Error;
///     type Credential = NoCredential;
///     // create() receives `&()` as `scheme` — no secrets.
/// }
/// ```
#[derive(Clone, Copy, Debug, Default)]
pub struct NoCredential;

impl Credential for NoCredential {
    type Input = ();
    /// `()` — already implements `AuthScheme` with `AuthPattern::NoAuth` and
    /// `PublicScheme` in `nebula_core::auth`.
    type Scheme = ();
    type State = NoCredentialState;

    const KEY: &'static str = "no_credential";

    fn metadata() -> CredentialMetadata {
        CredentialMetadata::builder(Self::KEY)
            .name("No credential")
            .description("Opt-out marker for resources without an authenticated binding.")
            .build()
            .expect("NoCredential metadata is statically valid")
    }

    fn project(_state: &Self::State) -> Self::Scheme {}

    fn resolve(
        _values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> impl Future<Output = Result<ResolveResult<Self::State, ()>, CredentialError>> + Send {
        async { Ok(ResolveResult::Resolved(NoCredentialState)) }
    }
}
```

> **Note on `metadata()`:** the `CredentialMetadataBuilder` API is established by П1 credential; if `.build()` returns a `Result` with the exact name above, fine — if not (signature drift), Task 1 reads `crates/credential/src/metadata.rs` and adapts. The intent is "minimal valid metadata; never panics in practice."

- [ ] **Step 4: Wire into `crates/credential/src/lib.rs`**

Add immediately after the existing `pub mod context;` declaration block (sorted alphabetically with sibling modules):

```rust
mod no_credential;
```

And in the `// Built-in credential implementations` re-export block (currently exporting `ApiKeyCredential`, `BasicAuthCredential`, etc.), add:

```rust
pub use no_credential::{NoCredential, NoCredentialState};
```

- [ ] **Step 5: Run smoke test to verify pass**

Run: `cargo nextest run -p nebula-credential --test no_credential_smoke --profile ci --no-tests=pass`
Expected: **PASS** — 5 tests pass.

- [ ] **Step 6: Run full credential test suite (no regressions)**

Run: `cargo nextest run -p nebula-credential --profile ci --no-tests=pass`
Expected: PASS for the entire `nebula-credential` test set; smoke test now part of suite.

- [ ] **Step 7: Format + clippy gate**

Run:
```
cargo +nightly fmt -p nebula-credential --
cargo clippy -p nebula-credential -- -D warnings
```
Expected: clean.

- [ ] **Step 8: Commit**

```
git add crates/credential/src/no_credential.rs \
        crates/credential/tests/no_credential_smoke.rs \
        crates/credential/src/lib.rs
git commit -m "feat(credential): add NoCredential opt-out type for Resource trait

Per ADR-0036 §Decision: Resource impls without an authenticated binding
adopt 'type Credential = NoCredential' instead of legacy 'type Auth = ()'.
NoCredentialState is a zero-data marker (Zeroize no-op, KIND='no_credential',
VERSION=1). Scheme is unit type — already AuthScheme + PublicScheme via
nebula-core. Drop-in replacement for the no-auth case.

Resource-side adoption follows in nebula-resource П1 trait reshape (next PR)."
```

---

## Task 2: Mark `OnCredentialRefresh<C>` deprecated in `nebula-credential`

**Files:**
- Modify: `crates/credential/src/secrets/scheme_guard.rs:251` (the `pub trait OnCredentialRefresh<C: Credential>` declaration)

**Why:** `OnCredentialRefresh<C>` was a transitional parallel trait introduced in credential П1 because the (then-unmigrated) `Resource` trait still bound `Auth: AuthScheme`. The П1 reshape lands the canonical method on `Resource` itself. The trait must stay one PR longer (П2 removes it once Manager-side dispatch lands on `Resource::on_credential_refresh`), but downstream code must stop adopting the parallel form.

- [ ] **Step 1: Edit `OnCredentialRefresh` trait declaration**

In `crates/credential/src/secrets/scheme_guard.rs`, locate `pub trait OnCredentialRefresh<C: Credential>: Send + Sync {` (around line 251) and prepend the attribute:

```rust
#[deprecated(
    since = "0.1.0",
    note = "Resource::on_credential_refresh subsumes this trait per ADR-0036; \
            removal scheduled for nebula-resource П2 once Manager dispatch \
            lands on the new method."
)]
pub trait OnCredentialRefresh<C: Credential>: Send + Sync {
```

Also add `#[allow(deprecated)]` above the `pub use` line at `crates/credential/src/secrets/mod.rs:37` (the line `pub use scheme_guard::{OnCredentialRefresh, SchemeFactory, SchemeGuard};`) and at `crates/credential/src/lib.rs:167` (where `OnCredentialRefresh` is re-exported alongside `SchemeGuard`).

If the trait is referenced by any in-tree impl or doc test, locally `#[allow(deprecated)]` those sites only — do not delete impls in this PR.

- [ ] **Step 2: Verify workspace still compiles + check warning surface**

Run: `cargo check --workspace 2>&1 | grep -E '(deprecated|OnCredentialRefresh)' | head -20`
Expected: no `deprecated` warnings escape (we suppressed them at the re-export sites). The `#[deprecated]` attribute is metadata only — produces a warning at use sites we did not annotate; we expect ZERO such sites in trunk because nothing currently `impl OnCredentialRefresh for X`.

If a warning escapes: investigate, add `#[allow(deprecated)]` at that one site only.

- [ ] **Step 3: Commit (squash with Task 1 acceptable, or separate)**

```
git add crates/credential/src/secrets/scheme_guard.rs \
        crates/credential/src/secrets/mod.rs \
        crates/credential/src/lib.rs
git commit -m "chore(credential): deprecate OnCredentialRefresh parallel trait

Transitional parallel trait introduced in credential П1 because Resource
still bound Auth: AuthScheme. ADR-0036 lands on_credential_refresh as a
method on Resource itself; the parallel trait is supplanted.

Marked #[deprecated], #[allow(deprecated)] at re-export sites to avoid
escaping warnings during workspace builds. Trait stays one more PR —
nebula-resource П2 removes it after Manager-side dispatch migrates."
```

---

## Task 3: Reshape `Resource` trait declaration

**Files:**
- Modify: `crates/resource/src/resource.rs:1-12` (imports), `:220-299` (trait definition)

**Why:** This is the heart of П1 — replaces `type Auth: AuthScheme` with `type Credential: Credential`, retypes `create()`, adds two default-no-op rotation hooks per ADR-0036 §Decision (canonical CP5 form).

- [ ] **Step 1: Update imports at top of `crates/resource/src/resource.rs`**

Replace:
```rust
use std::future::Future;

use nebula_core::ResourceKey;
use nebula_credential::AuthScheme;

use crate::context::ResourceContext;
```

With:
```rust
use std::future::Future;

use nebula_core::ResourceKey;
use nebula_credential::{Credential, CredentialContext, CredentialId, SchemeGuard};

use crate::context::ResourceContext;
```

- [ ] **Step 2: Update doc comment for `Resource` trait**

Find the doc comment immediately above `pub trait Resource:` (lines around 200-219). Replace the line that says "Implementors supply five associated types and four lifecycle methods." with:

```
//! Implementors supply five associated types and six lifecycle methods.
//! The `Credential` associated type carries the credential-binding
//! contract per ADR-0036; resources without an authenticated binding
//! use `type Credential = NoCredential` (re-exported from
//! `nebula_credential`).
```

- [ ] **Step 3: Replace `type Auth: AuthScheme;` with `type Credential: Credential;`**

In the `pub trait Resource:` body, locate and replace:

```rust
    /// Authentication scheme resolved by the credential system.
    type Auth: AuthScheme;
```

With:

```rust
    /// The credential type bound to this resource per ADR-0036.
    ///
    /// Resources without an authenticated binding use
    /// [`NoCredential`](nebula_credential::NoCredential). The runtime
    /// projects `<Self::Credential as Credential>::Scheme` and threads it
    /// through [`create`](Self::create) and rotation hooks.
    type Credential: Credential;
```

- [ ] **Step 4: Update `create()` signature**

Replace:

```rust
    /// Creates a new runtime instance from config and auth material.
    fn create(
        &self,
        config: &Self::Config,
        auth: &Self::Auth,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;
```

With:

```rust
    /// Creates a new runtime instance from config and projected scheme material.
    ///
    /// `scheme` is borrowed from the credential subsystem; resources MUST NOT
    /// retain it past the returned future per `PRODUCT_CANON.md §12.5`
    /// (secret-handling discipline).
    fn create(
        &self,
        config: &Self::Config,
        scheme: &<Self::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;
```

- [ ] **Step 5: Add `on_credential_refresh` default-no-op method**

Insert immediately after `create()`, before `check()`:

```rust
    /// Called by the engine after a successful credential refresh.
    ///
    /// Default: no-op. Connection-bound resources (Pool, Service, Transport)
    /// override with the blue-green pool swap pattern per credential Tech
    /// Spec §15.7 — build a fresh pool from `new_scheme`, atomically swap
    /// into the resource's `Arc<RwLock<Pool>>`, let RAII drain old handles.
    ///
    /// `new_scheme` and `ctx` share the lifetime `'a`. The shared lifetime
    /// is the compile-time barrier preventing retention — see
    /// [`SchemeGuard`](nebula_credential::SchemeGuard) Probe 6.
    /// Implementations MUST NOT store either argument past this call.
    ///
    /// Cancellation safety: implementations MUST be cancel-safe — if the
    /// returned future is dropped mid-await, the resource MUST remain
    /// consistent (`SchemeGuard`'s `ZeroizeOnDrop` fires deterministically
    /// across the cancellation boundary).
    ///
    /// **П1 status:** Manager-side dispatch is not wired in this PR; this
    /// method exists for impl ergonomics and forward-compat. П2 lands the
    /// reverse-index write + parallel `join_all` dispatcher.
    fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
        ctx: &'a CredentialContext,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a {
        let _ = (new_scheme, ctx);
        async { Ok(()) }
    }
```

- [ ] **Step 6: Add `on_credential_revoke` default-no-op method**

Insert immediately after `on_credential_refresh`:

```rust
    /// Called by the engine after a credential is revoked.
    ///
    /// Default: no-op. Override invariant per ADR-0036 §Decision:
    /// post-invocation, the resource MUST emit no further authenticated
    /// traffic on the revoked credential. The mechanism (destroy pool /
    /// mark-tainted / wait-for-drain / reject-new-acquires) is the
    /// implementor's choice; П2 Tech Spec §5 specifies typical patterns.
    ///
    /// **П1 status:** Manager-side dispatch is not wired in this PR; this
    /// method exists for impl ergonomics and forward-compat. П2 lands the
    /// reverse-index write + revocation dispatcher.
    fn on_credential_revoke(
        &self,
        credential_id: &CredentialId,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = credential_id;
        async { Ok(()) }
    }
```

- [ ] **Step 7: Compile-check resource crate (will fail — call sites still use `auth`)**

Run: `cargo check -p nebula-resource 2>&1 | head -50`
Expected: many errors of the shape `error[E0220]: associated type 'Auth' not found for 'R'` and `error[E0046]: not all trait items implemented, missing: 'Credential'`. This is correct — Tasks 4–8 fix them.

Do NOT commit yet — trait change is mechanically incomplete until call sites + impls migrate.

---

## Task 4: Update `Manager::acquire_*<R>` signatures

**Files:**
- Modify: `crates/resource/src/manager.rs:754, 810, 835, 1140, 1259-1280` (5 acquire_* methods + warmup_pool)

**Why:** `Manager` is the public façade — every consumer crate's `ctx.manager.acquire_pooled::<R>(&auth, ...)` call site flows through here. The signature change must keep the same shape (`&Scheme` borrowed, not owned) — the only difference is the type expression.

- [ ] **Step 1: Update `acquire_pooled` signature**

Locate `pub async fn acquire_pooled<R>(` (around line 754). Replace the `auth: &R::Auth` parameter and the body's `auth` references:

```rust
    pub async fn acquire_pooled<R>(
        &self,
        scheme: &<R::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
        options: &AcquireOptions,
    ) -> Result<crate::guard::ResourceGuard<R>, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
    {
        // body: rename internal `auth` → `scheme` references; pass `scheme` through
        // to runtime layer (Task 5 retypes those signatures).
    }
```

Add `use nebula_credential::Credential;` at the top of `manager.rs` if not already present.

- [ ] **Step 2: Update `acquire_resident`, `acquire_service`, `acquire_transport`, `acquire_exclusive`, `acquire_event_source`, `acquire_daemon`**

Same mechanical change for each. Per the inventory: 6 sibling methods around lines 810, 835, 1140, plus the EventSource and Daemon ones (line numbers shift after the first edit — search by name).

For each: `auth: &R::Auth` → `scheme: &<R::Credential as Credential>::Scheme`, body `auth` → `scheme`.

- [ ] **Step 3: Update `warmup_pool` bound and body**

Locate `pub async fn warmup_pool<R>` (around line 1259). Replace:

```rust
    pub async fn warmup_pool<R>(&self, ctx: &ResourceContext) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
        R::Auth: Default,
    {
        let managed = self.lookup::<R>(&ctx.scope_level())?;
        let config = managed.config();
        let auth = R::Auth::default();
        match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                let count = rt.warmup(&managed.resource, &config, &auth, ctx).await;
                Ok(count)
            },
            ...
        }
    }
```

With:

```rust
    pub async fn warmup_pool<R>(&self, ctx: &ResourceContext) -> Result<usize, Error>
    where
        R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
        R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
        R::Lease: Into<R::Runtime> + Send + 'static,
        // INTERIM (П1): retains a Default bound on the projected Scheme so
        // `NoCredential` (Scheme = ()) keeps working. П2 replaces this with a
        // credential-bearing warmup signature per ADR-0036 §Decision +
        // security-lead amendment B-3 (no Scheme::default() in production
        // hot paths). TODO(П2): remove Default bound, accept Scheme borrow.
        <R::Credential as Credential>::Scheme: Default,
    {
        let managed = self.lookup::<R>(&ctx.scope_level())?;
        let config = managed.config();
        let scheme = <<R::Credential as Credential>::Scheme as Default>::default();
        match &managed.topology {
            TopologyRuntime::Pool(rt) => {
                let count = rt.warmup(&managed.resource, &config, &scheme, ctx).await;
                Ok(count)
            },
            _ => Err(Error::permanent(format!(
                "{}: warmup_pool requires Pool topology, registered as {}",
                R::key(),
                managed.topology.tag()
            ))),
        }
    }
```

Also update the doc comment paragraph "Uses [`Default::default()`] for auth, which works for `R::Auth = ()` and any auth type that has a meaningful default." to "Uses [`Default::default()`] for the projected scheme, which works for `NoCredential` (`Scheme = ()`). Other credentials need П2's credential-bearing warmup signature."

- [ ] **Step 4: Don't compile-check yet — runtime signatures still untouched**

Continue to Task 5.

---

## Task 5: Retype runtime layer call paths

**Files:**
- Modify: `crates/resource/src/runtime/pool.rs:266, 459, 617, 710, 741, 785` (six `auth: &R::Auth` parameters)
- Modify: `crates/resource/src/runtime/resident.rs:80` (one site)
- Modify: `crates/resource/src/runtime/service.rs` (search for `auth: &R::Auth`)
- Modify: `crates/resource/src/runtime/transport.rs` (search)
- Modify: `crates/resource/src/runtime/exclusive.rs` (search)
- Modify: `crates/resource/src/runtime/event_source.rs` (search)
- Modify: `crates/resource/src/runtime/daemon.rs` (search)

**Why:** `Manager` delegates the actual `R::create(config, scheme, ctx)` invocation to the topology-runtime layer. Signatures must match.

- [ ] **Step 1: Mechanical replace in pool.rs**

In `crates/resource/src/runtime/pool.rs`, find every:
```rust
auth: &R::Auth,
```
and replace with:
```rust
scheme: &<R::Credential as Credential>::Scheme,
```

Also: every internal call site that passes `auth` onward (e.g., `R::create(config, auth, ctx).await` or `inner.create(..., auth, ...)`) — rename `auth` to `scheme`.

Add `use nebula_credential::Credential;` to file imports if not already present.

- [ ] **Step 2: Same mechanical pass for resident.rs / service.rs / transport.rs / exclusive.rs / event_source.rs / daemon.rs**

For each file: grep `auth: &R::Auth` and `R::Auth` and rename per Step 1 pattern. Also handle `&Self::Auth` if any — replace with `&<Self::Credential as Credential>::Scheme`.

- [ ] **Step 3: Compile-check resource crate**

Run: `cargo check -p nebula-resource 2>&1 | tee /tmp/check.log | head -80`
Expected: errors now reduce to the **impl Resource for X** sites — `error[E0046]: not all trait items implemented, missing: 'Credential'` for each of the 8 production impls + 19 test impls + 2 doc examples. Tasks 6–8 close them.

If you see signature mismatches in the runtime layer (errors mentioning function-arg types), fix in this task before proceeding.

---

## Task 6: Migrate 8 production `impl Resource` sites

**Files:**
- Modify: `crates/resource/src/guard.rs:370-...` (DummyResource)
- Modify: `crates/resource/src/runtime/daemon.rs:305-..., :345-...` (FlakyDaemon, OneShotDaemon)
- Modify: `crates/resource/src/runtime/pool.rs:1006-...` (MockPool)
- Modify: `crates/resource/src/runtime/resident.rs:200-..., :360-...` (MockResident, HangingResident)
- Modify: `crates/resource/src/runtime/service.rs:153-..., :198-...` (ClonedService, TrackedService)

**Why:** Each in-source `impl Resource for X` block currently has `type Auth = ();` plus `fn create(&self, config: &Self::Config, _auth: &Self::Auth, ctx: &ResourceContext)`. The П1 reshape replaces the associated type and the parameter type — body unchanged because `_auth` was already unused.

**Common transform pattern:**

Before:
```rust
impl Resource for MockPool {
    type Config = MockPoolConfig;
    type Runtime = MockPoolRuntime;
    type Lease = MockPoolRuntime;
    type Error = MockError;
    type Auth = ();

    fn key() -> ResourceKey { resource_key!("mock.pool") }

    fn create(
        &self,
        config: &Self::Config,
        _auth: &Self::Auth,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send {
        async move { /* ... */ }
    }
}
```

After:
```rust
impl Resource for MockPool {
    type Config = MockPoolConfig;
    type Runtime = MockPoolRuntime;
    type Lease = MockPoolRuntime;
    type Error = MockError;
    type Credential = nebula_credential::NoCredential;

    fn key() -> ResourceKey { resource_key!("mock.pool") }

    fn create(
        &self,
        config: &Self::Config,
        _scheme: &<Self::Credential as nebula_credential::Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send {
        async move { /* unchanged body */ }
    }
}
```

> **Path-style note:** prefer the qualified `<Self::Credential as nebula_credential::Credential>::Scheme` only if needed for disambiguation; if the file already has `use nebula_credential::Credential;`, write `<Self::Credential as Credential>::Scheme`. Either is correct — pick the shorter of the two for the file's existing imports.

- [ ] **Step 1: Migrate `guard.rs` DummyResource**

Edit `crates/resource/src/guard.rs:370-...` per the transform pattern. The block is small — entire `impl Resource for DummyResource` is ~25 lines.

- [ ] **Step 2: Migrate `runtime/pool.rs` MockPool**

Edit `crates/resource/src/runtime/pool.rs:1006-...`. If `nebula_credential::Credential` is not imported at top of file, add `use nebula_credential::{Credential, NoCredential};` (Credential trait was likely already added in Task 5; just add NoCredential).

- [ ] **Step 3: Migrate `runtime/resident.rs` MockResident + HangingResident**

Two impls. Same transform pattern.

- [ ] **Step 4: Migrate `runtime/service.rs` ClonedService + TrackedService**

Two impls.

- [ ] **Step 5: Migrate `runtime/daemon.rs` FlakyDaemon + OneShotDaemon**

Two impls. (Daemon topology stays in resource crate for П1; ADR-0037 extraction is П4.)

- [ ] **Step 6: Compile-check resource crate (production-only path)**

Run: `cargo check -p nebula-resource --lib 2>&1 | head -40`
Expected: lib compiles clean (test impls migrate in Task 7).

If there's a residual error, it's likely an internal call site we missed in Task 5. Fix it before proceeding.

---

## Task 7: Migrate 19 test `impl Resource` sites

**Files:**
- Modify: `crates/resource/tests/basic_integration.rs:109, 183, 619, 1536, 1620, 1698, 2149, 2506, 2581, 2966, 3173, 3265, 3355, 3652` (14 sites)
- Modify: `crates/resource/tests/dx_evaluation.rs:93, 225, 352` (3 sites — line numbers from grep on `impl Resource for`)
- Modify: `crates/resource/tests/dx_audit.rs:130, 296, 439` (3 sites)

**Why:** Test resources mirror the production migration. Identical transform pattern.

**Approach:** Because the transform is mechanically identical across all 19 sites, the recommended workflow is:
1. Find-and-replace `type Auth = ();` → `type Credential = NoCredential;`
2. Find-and-replace `_auth: &Self::Auth` → `_scheme: &<Self::Credential as Credential>::Scheme`
3. Add `use nebula_credential::{Credential, NoCredential};` to the test file imports if needed (or `use nebula_resource::{Credential, NoCredential};` once Task 9 wires the re-export).

But run the find-and-replace **per file**, not workspace-wide — there may be unrelated `_auth:` parameters in helper functions.

- [ ] **Step 1: Migrate `tests/basic_integration.rs` (14 sites)**

Edit each impl block. After all 14 sites are updated, also check for any callers in test bodies that pass `&()` as the auth argument to `manager.acquire_*` — those still work because `<NoCredential as Credential>::Scheme = ()`, but make sure no test fixture builds a non-`()` auth.

- [ ] **Step 2: Migrate `tests/dx_evaluation.rs` (3 sites)**

- [ ] **Step 3: Migrate `tests/dx_audit.rs` (3 sites)**

- [ ] **Step 4: Compile-check resource tests**

Run: `cargo check -p nebula-resource --tests 2>&1 | head -60`
Expected: all tests compile.

If a test passes a non-`()` value where a scheme is expected, that's a real test fixture that was abusing `type Auth = ();` semantics — file as a follow-up note in the PR description, do NOT silently change test behavior.

- [ ] **Step 5: Run resource test suite**

Run: `cargo nextest run -p nebula-resource --profile ci --no-tests=pass`
Expected: all tests **PASS** with same counts as before П1. Behavior unchanged because:
- `NoCredential::Scheme = ()` — same effective type as `Auth = ()`
- `on_credential_refresh` / `on_credential_revoke` are default no-ops — never invoked in П1
- Manager dispatchers `on_credential_refreshed` / `on_credential_revoked` remain `todo!()` — same as today (no test exercises the path)

If a test fails, diagnose root cause — П1 must be behavior-preserving.

---

## Task 8: Update doc examples

**Files:**
- Modify: `crates/resource/docs/README.md:90`
- Modify: `crates/resource/docs/adapters.md:204`

**Why:** Two doc sites carry literal `type Auth = ();` in fenced code blocks. They are not compiled (no `doctest` harness), but they teach the trait shape — out-of-date examples mislead.

- [ ] **Step 1: Update `crates/resource/docs/README.md` line 90 area**

Change the fenced code example so the `impl Resource for …` block uses:
```
    type Credential = NoCredential;          // No credential needed
```
And the `create` signature uses `_scheme: &<Self::Credential as Credential>::Scheme`.

If the surrounding paragraph mentions "auth", rephrase to "credential scheme" with one sentence pointing at `NoCredential` for opt-out.

- [ ] **Step 2: Update `crates/resource/docs/adapters.md` line 204 area**

Same transform.

- [ ] **Step 3: Add a one-paragraph mention to `crates/resource/README.md`**

Append to the "Public API" or "Maturity" section (one sentence): "The trait binds to credentials via `type Credential: Credential` per ADR-0036; resources without an authenticated binding write `type Credential = NoCredential;` (re-exported from `nebula_credential`)."

- [ ] **Step 4: Add a one-line mention to `crates/credential/README.md`**

Under the "Built-in credential implementations" / "Built-in types" list, append: "- `NoCredential` — opt-out for resources without an authenticated binding (ADR-0036)."

---

## Task 9: Re-export from `nebula-resource::lib` and finalize

**Files:**
- Modify: `crates/resource/src/lib.rs` (re-exports)

**Why:** Tech Spec §2.2 + ADR-0036 §Decision both expect resource consumers to be able to write `use nebula_resource::NoCredential;` without reaching into `nebula_credential`. Mirror the credential П1 re-export pattern.

- [ ] **Step 1: Edit `crates/resource/src/lib.rs`**

Locate the existing `pub use` block (lib.rs has 100+ lines of re-exports). Add a new sub-block, sorted to live alongside other `nebula_credential` re-exports (or as a new credential-related cluster):

```rust
// Credential adoption surface per ADR-0036 — re-exported so resource
// consumers don't need a direct nebula-credential dep for trait shape.
pub use nebula_credential::{
    Credential, CredentialContext, CredentialId, NoCredential, NoCredentialState,
    SchemeGuard,
};
```

If `nebula_credential` already exposes `AuthScheme` and any resource impl uses it, do NOT re-export — keep import paths explicit. The re-export list above is the minimum surface needed for `impl Resource` sites.

- [ ] **Step 2: Workspace compile + tests**

Run, in order:
```
cargo check --workspace
cargo nextest run --workspace --profile ci --no-tests=pass
```
Expected: clean compile, all tests PASS.

- [ ] **Step 3: Format + clippy gate (workspace)**

```
cargo +nightly fmt --all -- --check
cargo clippy --workspace -- -D warnings
```
Expected: clean. If `fmt --check` fails, run without `--check` to fix, then re-check.

- [ ] **Step 4: Spot-check 5 consumer crates compile clean**

```
cargo check -p nebula-action
cargo check -p nebula-sdk
cargo check -p nebula-engine
cargo check -p nebula-plugin
cargo check -p nebula-sandbox
```
Expected: each clean. If a crate fails, it is calling `manager.acquire_*::<R>(&auth, ...)` with a non-`()` value — investigate; in П1 the only valid Scheme is `()` since all impls are `NoCredential`-bound. Fix the call site to pass `&()`.

- [ ] **Step 5: Verify deny.toml + MATURITY untouched**

Run: `git diff --stat deny.toml docs/MATURITY.md`
Expected: empty (П1 doesn't change crate boundaries or maturity tier — `frontier` stays `frontier` until П2 lands the dispatcher).

---

## Task 10: Final commit + PR creation

- [ ] **Step 1: Stage all changes**

```
git status --short
git diff --stat
```
Review the diff. Expected file count: ~20–22 files modified.

- [ ] **Step 2: Run the full local gate one more time**

```
cargo +nightly fmt --all -- --check && \
cargo clippy --workspace -- -D warnings && \
cargo nextest run --workspace --profile ci --no-tests=pass
```
All three must pass. If clippy gripes about `_scheme` unused arguments after migration: that's expected — the parameter is intentionally bound for trait conformance, even when unused. Use `_scheme` (leading underscore) to silence; it's already in the migration template.

- [ ] **Step 3: Commit with the conventional message**

```
git add -A   # acceptable here — diff has been reviewed
git commit -m "feat(resource)!: П1 — Resource::Credential adoption (ADR-0036)

Trait shape scaffolding per ADR-0036 §Decision (amended-in-place
2026-04-26 cross-cascade R2 canonical CP5 form):

- type Auth: AuthScheme  →  type Credential: Credential
- create(): auth: &R::Auth  →  scheme: &<R::Credential as Credential>::Scheme
- on_credential_refresh<'a>(SchemeGuard<'a, _>, &'a CredentialContext) — default no-op
- on_credential_revoke(&CredentialId) — default no-op
- NoCredential opt-out introduced in nebula-credential, re-exported here
- Manager::acquire_* + warmup_pool retyped (warmup retains Default bound on
  Scheme as INTERIM until П2 lands credential-bearing warmup)
- 8 production + 19 test impl sites migrated (NoCredential)
- nebula-credential::OnCredentialRefresh<C> marked #[deprecated]
  (transitional parallel trait — П2 removes once Manager dispatch lands
  on Resource::on_credential_refresh)

Behavior preserved: dispatchers Manager::on_credential_refreshed /
on_credential_revoked remain todo!() — same as today, П2 lands the
reverse-index write + parallel join_all dispatcher.

Closes register R-001 (Auth dead → Credential adoption).
Partial: R-005 (warmup ban) — interim Default-on-Scheme bound, П2 removes."
```

- [ ] **Step 4: Push + open PR**

```
git push -u origin claude/objective-albattani-4dcc21
gh pr create --title "feat(resource)!: П1 — Resource::Credential adoption (ADR-0036)" \
  --body "$(cat <<'EOF'
## Summary

- Adopts ADR-0036 §Decision verbatim (amended-in-place 2026-04-26 cross-cascade R2 canonical CP5 form).
- Reshapes Resource trait: type Auth → type Credential; create() retyped; two default-no-op rotation hooks added.
- Introduces NoCredential opt-out in nebula-credential; re-exports from nebula-resource.
- Migrates 8 production + 19 test impl sites to NoCredential (zero behavioral change).
- Marks transitional OnCredentialRefresh<C> parallel trait #[deprecated] — П2 removes.

## Out of scope (next PRs)

- П2: reverse-index write + dispatch_rotation + observability events (closes 🔴-1 silent revocation drop).
- П3: Manager file-split (manager.rs 2101L → 7 submodules) + drain-abort fix.
- П4: Daemon/EventSource extraction per ADR-0037.
- П5: doc rewrite (api-reference.md, Architecture.md, events.md).

## Test plan

- [x] cargo check --workspace
- [x] cargo nextest run --workspace --profile ci --no-tests=pass
- [x] cargo clippy --workspace -- -D warnings
- [x] cargo +nightly fmt --all -- --check
- [x] All resource integration tests pass (zero behavior change)
- [x] 5 consumer crates (action, sdk, engine, plugin, sandbox) compile clean

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

Expected: PR opens against `main`. CI runs the same gate locally just verified.

---

## Self-review checklist

After all 10 tasks complete, before marking the plan done:

**1. Spec coverage:**
- [x] ADR-0036 §Decision conceptual signature → Task 3 (trait reshape) + Task 5 (runtime layer)
- [x] ADR-0036 NoCredential opt-out → Task 1 (creation) + Task 9 (re-export)
- [x] ADR-0036 SchemeGuard<'a, _> shared-lifetime form → Task 3 Step 5 (signature uses canonical CP5 form)
- [x] ADR-0036 dispatcher / reverse-index → DEFERRED to П2 (documented in plan goal + commit message)
- [x] ADR-0036 warmup_pool Scheme::default() ban → INTERIM Default bound documented as TODO(П2)
- [x] Tech Spec §2.1 trait signature — covered by Task 3
- [x] Tech Spec §2.2 NoCredential definition — covered by Task 1
- [x] Concerns register R-001 (Auth dead) — closed by Task 6+7
- [x] Concerns register R-005 (warmup ban) — partial (interim only); explicit deferral

**2. Placeholder scan:**
- [x] No "TBD" / "TODO: implement later" outside the explicitly-marked TODO(П2) on warmup_pool.
- [x] No "Add appropriate error handling" — there is no new error path; all hooks return `Ok(())` by default.
- [x] No "Write tests for the above" — Tasks 1, 7, 9 list specific test commands.
- [x] No "Similar to Task N" — every task carries its own code blocks.

**3. Type consistency:**
- [x] `<Self::Credential as Credential>::Scheme` used uniformly in trait and call sites.
- [x] `SchemeGuard<'a, Self::Credential>` matches credential `crates/credential/src/secrets/scheme_guard.rs:64-84` definition.
- [x] `NoCredential` is consistently `nebula_credential::NoCredential` (or re-exported `nebula_resource::NoCredential` after Task 9).
- [x] `CredentialId`, `CredentialContext` import paths match credential lib.rs re-exports.

---

## Execution Handoff

**Plan complete and saved to** `docs/superpowers/plans/2026-04-27-nebula-resource-p1-trait-shape.md`.

Two execution options:

**1. Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration. Best for this plan because Tasks 6–7 are mechanically repetitive (27 sites) and benefit from fresh-context execution.

**2. Inline Execution** — run tasks in this session via `superpowers:executing-plans`. Faster handoff but burns more context on the repetitive migration work.

Pick one when ready to start implementation.
