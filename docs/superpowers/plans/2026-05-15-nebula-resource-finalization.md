# nebula-resource Finalization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close §M11.5 (engine-owned per-slot credential rotation fan-out) + §M12.4 (frontier→stable) for `nebula-resource`, including the engine-side typed-registration bridge and the HTTP API surface.

**Architecture:** Four dependency-ordered phases. **A** builds the runtime credential-slot substrate (absent today), reshapes the refresh hook to `&self` + `&Runtime` (supersedes ADR-0044 hook signature), adds the `Manager::{refresh_slot,revoke_slot}` port, fixes the structural dedup key, and wires the engine-side reverse-index + `join_all` fan-out with per-resource timeout isolation. **B** adds the engine-side erased `kind→registrar` indirection (resource-side `register_from_value` is already implemented). **C** wires the API (config CRUD + read-only status, ADR-0047). **D** is closure (ADR-0052, MATURITY flip, concerns-register reconciliation, docstring sweep). Each phase produces independently testable software.

**Tech Stack:** Rust 1.95 (edition 2024, AFIT/RPITIT stable), Tokio, `arc-swap`, `dashmap`, `thiserror` + `nebula_error::Classify`, `nebula-eventbus`, `nebula-metrics`, `utoipa`/`axum` (api), `cargo nextest` + `trybuild` + `insta`.

**Spec:** `docs/superpowers/specs/2026-05-15-nebula-resource-finalization-design.md` (D1/D2/D3 + 4 abuse invariants are binding context — read it first).

**Global commit rule:** every `git commit` message is Conventional-Commits (`<type>(<scope>): <summary>`, validated by `convco`) and **ends with** a trailer line:
`Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`
Scopes: `resource`, `engine`, `api`, `docs`. Run `task fmt` before any commit that touches `.rs`.

**Phase testability boundary:**
- After Phase A: rotation fan-out works end-to-end (engine rotate → affected resource hooks fire, isolated timeouts, revoke drains). `cargo nextest run -p nebula-resource -p nebula-engine` green.
- After Phase B: a stored `ResourceEntry.kind` string registers a typed resource; unknown kind ⇒ typed error.
- After Phase C: `GET/POST/PUT/DELETE …/resources` + `…/resources/{id}/status` work; OpenAPI honest (no 501/deprecated).
- After Phase D: ADR-0052 filed; MATURITY Engine-integration row `partial→stable`; concerns register retired; `task dev:check` green workspace-wide.

---

## Phase A — §M11.5 per-slot rotation (engine-owned fan-out)

### Task A1: Slot-storage substrate — `SlotCell` newtype

**Files:**
- Create: `crates/resource/src/slot.rs`
- Modify: `crates/resource/src/lib.rs` (add `mod slot;` + `pub use slot::SlotCell;`)
- Test: in `crates/resource/src/slot.rs` `#[cfg(test)]`

The runtime has no place to hold a resolved `CredentialGuard<C>` today (verified: macro emits only `DeclaresDependencies`). Build a per-slot lock-free cell reusing the existing `ArcSwapOption` pattern from `cell.rs:10-46`. `CredentialGuard<S>` is `!Clone` + `Drop`-zeroizing (`credential/src/secrets/guard.rs:36-64`); store `Arc<CredentialGuard<S>>` so swap never clones secret bytes.

- [ ] **Step 1: Write the failing test**

```rust
// crates/resource/src/slot.rs  (#[cfg(test)] module at bottom)
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Default)]
    struct FakeGuard(u32);
    impl zeroize::Zeroize for FakeGuard { fn zeroize(&mut self) { self.0 = 0; } }

    #[test]
    fn slot_cell_swaps_without_clone_and_reads_latest() {
        let cell: SlotCell<FakeGuard> = SlotCell::empty();
        assert!(cell.load().is_none());
        cell.store(Arc::new(FakeGuard(1)));
        assert_eq!(cell.load().expect("v1").0, 1);
        cell.store(Arc::new(FakeGuard(2)));
        assert_eq!(cell.load().expect("v2").0, 2);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-resource slot::tests -- --nocapture`
Expected: FAIL — `cannot find type 'SlotCell' in this scope` (module not created yet).

- [ ] **Step 3: Write minimal implementation**

```rust
// crates/resource/src/slot.rs
//! Per-slot runtime storage for a resolved credential.
//!
//! A resource declares `#[credential]` slots; the engine resolves each into a
//! `CredentialGuard<C>` and stores it here before `Resource::create`. On
//! rotation the engine swaps a fresh guard in without `&mut` on the resource
//! (D2 of the finalization spec). Lock-free via `arc-swap`.

use arc_swap::ArcSwapOption;
use std::sync::Arc;

/// Lock-free interior-mutable holder for one resolved credential slot.
///
/// Holds `Arc<CredentialGuard<S>>`: `CredentialGuard` is `!Clone` and
/// zeroizes on `Drop`, so the `Arc` indirection lets the engine swap a
/// rotated guard in with no secret-byte clone.
#[derive(Debug)]
pub struct SlotCell<S> {
    inner: ArcSwapOption<S>,
}

impl<S> SlotCell<S> {
    /// An unresolved slot.
    pub fn empty() -> Self {
        Self { inner: ArcSwapOption::empty() }
    }

    /// Install (or replace) the resolved value. Old value is dropped
    /// (zeroized if it is a `CredentialGuard`) once no reader holds it.
    pub fn store(&self, value: Arc<S>) {
        self.inner.store(Some(value));
    }

    /// Snapshot the current value, if resolved.
    pub fn load(&self) -> Option<Arc<S>> {
        self.inner.load_full()
    }
}

impl<S> Default for SlotCell<S> {
    fn default() -> Self {
        Self::empty()
    }
}
```

Add to `crates/resource/src/lib.rs` (next to the existing `mod cell;` / `pub use`):

```rust
mod slot;
pub use slot::SlotCell;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-resource slot::tests`
Expected: PASS (1 test).

- [ ] **Step 5: Commit**

```bash
task fmt
git add crates/resource/src/slot.rs crates/resource/src/lib.rs
git commit -m "feat(resource): add SlotCell lock-free credential-slot substrate

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task A2: Reshape the refresh hook — `&self` + `&Self::Runtime` + add `on_credential_revoke`

**Files:**
- Modify: `crates/resource/src/resource.rs:289-295` (the `on_credential_refresh` default)
- Test: `crates/resource/tests/hook_shape.rs` (new — compile-level shape assertion)

Supersedes ADR-0044's `&mut self` hook (recorded in ADR-0052, Task D1). The hook is a notification + reaction on the live runtime; the descriptor `self` is immutable.

- [ ] **Step 1: Write the failing test**

```rust
// crates/resource/tests/hook_shape.rs
//! Asserts the D2 hook shape: &self + &Runtime, plus on_credential_revoke.
use nebula_resource::Resource;

fn assert_refresh_takes_shared_ref<R: Resource>(r: &R, rt: &R::Runtime) {
    // Must compile with &self (not &mut self) and a &Runtime argument.
    let _f = r.on_credential_refresh("slot", rt);
    let _g = r.on_credential_revoke("slot", rt);
}

#[test]
fn hook_shape_is_shared_ref() {
    // Compilation is the assertion; this body is intentionally empty.
    let _ = assert_refresh_takes_shared_ref::<DummyNeverConstructed>;
}

struct DummyNeverConstructed;
// no impl needed — fn item reference above only type-checks the bound shape
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-resource --test hook_shape`
Expected: FAIL — current trait has `on_credential_refresh(&mut self, slot_name)` and no `on_credential_revoke`; `&r` call + revoke method do not resolve.

- [ ] **Step 3: Write minimal implementation**

Replace `crates/resource/src/resource.rs:289-295` (the existing `on_credential_refresh`) with:

```rust
    /// Called by the engine rotation fan-out after it has swapped the
    /// rotated credential into this resource's slot (`SlotCell`). `&self`:
    /// the resource impl is an immutable descriptor; blue-green / re-auth
    /// acts on `runtime`'s own interior mutability. `slot_name` identifies
    /// which `#[credential]` slot rotated. Default: no-op.
    fn on_credential_refresh(
        &self,
        slot_name: &str,
        runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = (slot_name, runtime);
        async { Ok(()) }
    }

    /// Called by the engine fan-out when a slot's credential is revoked.
    /// Post-invocation invariant (ADR-0036): the resource emits no further
    /// authenticated traffic on the revoked credential. Default: no-op
    /// (the engine still taints + drains the runtime around this call).
    fn on_credential_revoke(
        &self,
        slot_name: &str,
        runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = (slot_name, runtime);
        async { Ok(()) }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-resource --test hook_shape`
Expected: PASS. Then `cargo check -p nebula-resource` — expect compile errors in any in-crate impl overriding the old signature; none are expected in `nebula-resource` itself (overrides live in adapters/examples handled in A4).

- [ ] **Step 5: Commit**

```bash
task fmt
git add crates/resource/src/resource.rs crates/resource/tests/hook_shape.rs
git commit -m "refactor(resource)!: on_credential_refresh -> &self + &Runtime; add on_credential_revoke

Supersedes ADR-0044 hook signature per finalization spec D2.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task A3: `#[derive(Resource)]` emits slot cells + typed accessor

**Design (resolved 2026-05-15 — Alternative (a), supersedes the original
`__slot_lookup`/ManagedResource-side framing):** a pure `#[proc_macro_derive]`
cannot add/rewrite struct fields, and `ManagedResource` hands out `Arc<R>`
(no `&mut R`). Therefore the `#[credential]` field type **is**
`SlotCell<CredentialGuard<C>>` declared directly on the author's struct (was
bare `CredentialGuard<C>` per ADR-0044's now-superseded migration note). The
derive emits only an inherent **read accessor** `fn <field>_slot(&self) ->
Option<Arc<CredentialGuard<C>>>` = `self.<field>.load()`. The framework
populates/rotates via `SlotCell::store` through `&self` (A6). No
`__slot_lookup`, no framework side-table, fully per-instance, secret-safe
(`Arc<CredentialGuard>`, never cloned). Recorded in ADR-0052 (Task D1) as a
supersession of ADR-0044's slot-field/migration shape.

**Files:**
- Modify: `crates/resource/macros/src/field_slots.rs` — extend
  `decode_field_type` (`:131-158`) to accept `SlotCell<CredentialGuard<C>>`
  (with optional `Option<…>` / `Lazy<…>` wrappers, same tail-strip pattern);
  add an accessor emitter next to `emit_slot_field_registrations` (`:180-208`).
- Modify: `crates/resource/macros/src/resource.rs` — emit the accessor inherent
  impl alongside the existing `resource_impl` / `deps_impl` (`:60-94`).
- Test: `crates/resource/tests/trybuild/derive_slot_accessor.rs` + a harness
  entry (reuse the crate's existing trybuild harness if one exists — inspect
  `crates/resource/tests/` first; only create `trybuild.rs` if absent, matching
  any existing trybuild pattern in the crate).

- [ ] **Step 1: Write the failing test (trybuild pass-case)**

```rust
// crates/resource/tests/trybuild/derive_slot_accessor.rs
use nebula_resource::{Resource, SlotCell};
use nebula_credential::CredentialGuard;

#[derive(Resource)]
#[resource(key = "demo", topology = "resident", config = DemoCfg)]
struct Demo {
    #[credential(key = "db")]
    db: SlotCell<CredentialGuard<FakeCred>>,
}

#[derive(Clone, Default)]
struct DemoCfg;
impl nebula_schema::HasSchema for DemoCfg {
    fn schema() -> nebula_schema::ValidSchema { nebula_schema::ValidSchema::empty() }
}
impl nebula_resource::ResourceConfig for DemoCfg {}

// Minimal Credential fixture — inspect the real `nebula_credential::Credential`
// trait and satisfy its required items (or reuse an existing crate/test
// credential fixture if one exists; prefer reuse).
struct FakeCred;
/* impl nebula_credential::Credential for FakeCred { ... } */

fn main() {
    let d = Demo { db: SlotCell::empty() };
    // generated accessor exists, type-checks, returns None when unresolved:
    let _maybe: Option<std::sync::Arc<CredentialGuard<FakeCred>>> = d.db_slot();
}
```

Add a harness test (in the crate's existing trybuild harness, else a new
`crates/resource/tests/trybuild.rs`):

```rust
#[test]
fn derive_emits_slot_accessor() {
    let t = trybuild::TestCases::new();
    t.pass("tests/trybuild/derive_slot_accessor.rs");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-resource --test trybuild derive_emits_slot_accessor`
Expected: FAIL — no `db_slot()` accessor emitted (and/or `decode_field_type`
rejects the `SlotCell<…>` field shape).

- [ ] **Step 3: Write minimal implementation**

In `crates/resource/macros/src/field_slots.rs`, extend `decode_field_type` so
a `#[credential]` field of shape `SlotCell<CredentialGuard<C>>` (optionally
`Option<…>` / `Lazy<…>` wrapped, reusing the existing `strip_path_tail`
pattern) parses, yielding inner `C`. Then add an accessor emitter:

```rust
/// Per slot, emit a read accessor over the author-declared
/// `SlotCell<CredentialGuard<C>>` field. The field already exists on the
/// struct (the author declares it); the derive adds no fields.
pub(crate) fn emit_slot_accessors(
    slots: &[ParsedCredentialSlot],
) -> proc_macro2::TokenStream {
    let accessors = slots.iter().map(|slot| {
        let field = &slot.field_ident;
        let acc_ident = quote::format_ident!("{}_slot", field);
        let inner = &slot.inner_type;
        quote::quote! {
            #[doc = "Resolved credential for this slot, or `None` until the framework binds it."]
            pub fn #acc_ident(&self) -> ::std::option::Option<
                ::std::sync::Arc<::nebula_credential::CredentialGuard<#inner>>
            > {
                self.#field.load()
            }
        }
    });
    quote::quote!(#(#accessors)*)
}
```

In `crates/resource/macros/src/resource.rs`, after the existing `deps_impl`
block, emit an inherent impl with the accessors and add it to the final
expansion:

```rust
let slot_accessors = crate::field_slots::emit_slot_accessors(&slots);
let slot_accessor_impl = quote! {
    impl #impl_generics #struct_name #ty_generics #where_clause {
        #slot_accessors
    }
};
// final: quote!{ #resource_impl #deps_impl #slot_accessor_impl }
```

(If `nebula-resource-macros` lacks a path dep that lets it name `SlotCell`,
note it: the macro only emits `::nebula_resource::SlotCell` *tokens* — no
build-dep needed; the generated code resolves it in the user crate.)

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-resource --test trybuild derive_emits_slot_accessor`
Expected: PASS. Then `cargo check -p nebula-resource` and
`cargo check -p nebula-resource-macros` clean; run the full
`cargo test -p nebula-resource --test trybuild` to confirm no regression of
other derive trybuild cases.

- [ ] **Step 5: Commit**

```bash
cargo fmt -p nebula-resource -p nebula-resource-macros
git add crates/resource/macros/src/ crates/resource/tests/
git commit -m "feat(resource): derive(Resource) emits per-slot SlotCell accessor

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task A4: Migrate in-repo `Resource` impls + examples to the new hook shape

**Files:**
- Modify: every in-repo `impl Resource` overriding `on_credential_refresh` (find via grep below) — `crates/**`, `examples/**`
- Test: `cargo check --workspace --all-targets`

- [ ] **Step 1: Enumerate the impls + `#[credential]` fields to migrate**

Run: `rg -n "on_credential_refresh|on_credential_revoke" --type rust crates examples`, `rg -n "impl .*Resource for" --type rust crates examples`, and `rg -n "#\[credential" --type rust crates examples`.
Record the file:line list. Expected ≈ a handful of adapters/examples (`m6_postgres_pool`, `m6_resident_http`, `m6_telegram_multi_workflow` per README) + any test fixtures + every `#[derive(Resource)]` struct with `#[credential]` fields.

- [ ] **Step 2: Run the workspace check to see the failures**

Run: `cargo check --workspace --all-targets`
Expected: FAIL — overrides using `&mut self` / old arity no longer match the trait.

- [ ] **Step 3: Apply the mechanical migration to each site**

Two mechanical changes per site:
1. **`#[credential]` field type:** `#[credential(...)] f: CredentialGuard<C>` → `#[credential(...)] f: SlotCell<CredentialGuard<C>>` (likewise `Option<…>` / `Lazy<…>` wrappers stay outside). Construct with `SlotCell::empty()`. Read the resolved credential via the derive-generated accessor `self.f_slot()` (returns `Option<Arc<CredentialGuard<C>>>`) instead of `&self.f`. Update `create`/hook bodies accordingly.
2. **Hook signature:** `fn on_credential_refresh(&mut self, slot_name: &str)` → `fn on_credential_refresh(&self, slot_name: &str, runtime: &Self::Runtime)`; add `on_credential_revoke` override only where the old code had revoke logic. Move blue-green mutation onto `runtime`'s interior mutability (adapters own an `Arc<…>`/`ArcSwap` inside their `Runtime`). No deprecated alias (`feedback_no_shims`).

Also update the `crates/resource/README.md` migration-note/contract prose that shows `&self.auth` to the `self.auth_slot()` + `SlotCell` field shape (same-crate doc, in scope here).

- [ ] **Step 4: Re-run the workspace check**

Run: `cargo check --workspace --all-targets`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all  # if `task fmt` fails on this Windows path (OS 206), fall back to per-crate `cargo fmt -p <crate>` for each touched crate; lefthook fmt-check still gates
git add crates examples
git commit -m "refactor(resource)!: migrate Resource impls to SlotCell slots + &self hook

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task A5: `ResourceEvent` rotation variants (credential-data-free)

**Files:**
- Modify: `crates/resource/src/events.rs` (enum is `#[non_exhaustive]` — additive)
- Test: `crates/resource/src/events.rs` `#[cfg(test)]`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn slot_events_carry_no_credential_data() {
    let e = ResourceEvent::SlotRefreshed {
        key: nebula_core::ResourceKey::new("k").unwrap(),
        slot: "db".into(),
    };
    match e.key() { Some(k) => assert_eq!(k.as_str(), "k"), None => panic!("key") }
    // SlotRefreshFailed carries an error STRING (already redacted), never a token.
    let _ = ResourceEvent::SlotRefreshFailed {
        key: nebula_core::ResourceKey::new("k").unwrap(),
        slot: "db".into(),
        error: "transient: upstream 503".into(),
    };
    let _ = ResourceEvent::SlotRevoked {
        key: nebula_core::ResourceKey::new("k").unwrap(),
        slot: "db".into(),
    };
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-resource events:: -- slot_events`
Expected: FAIL — variants `SlotRefreshed/SlotRevoked/SlotRefreshFailed` do not exist.

- [ ] **Step 3: Write minimal implementation**

Add to the `ResourceEvent` enum in `crates/resource/src/events.rs` (and extend the existing `key()` match arms to return `Some(key)` for the three new variants):

```rust
    /// A `#[credential]` slot was refreshed on this resource (engine fan-out).
    SlotRefreshed { key: ResourceKey, slot: String },
    /// A `#[credential]` slot's credential was revoked; runtime tainted+drained.
    SlotRevoked { key: ResourceKey, slot: String },
    /// The per-resource refresh hook failed or timed out. `error` is an
    /// already-redacted string (NEVER credential material — PRODUCT_CANON §12.5).
    SlotRefreshFailed { key: ResourceKey, slot: String, error: String },
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-resource events:: -- slot_events`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
task fmt
git add crates/resource/src/events.rs
git commit -m "feat(resource): add SlotRefreshed/SlotRevoked/SlotRefreshFailed events

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task A6: `Manager::{refresh_slot,revoke_slot}` port + slot-cell install on create

**Slot model (per A3 Alternative (a)):** the `SlotCell<CredentialGuard<C>>`
lives on the **author's resource struct** (declared field), NOT in a
`ManagedResource`-side map. The framework "installs" a resolved credential by
calling `resource.<field>.store(Arc::new(guard))` via `&self` (lock-free); the
generated `<field>_slot()` accessor reads it. There is no `__slot_lookup` and
no `slots: DashMap` on `ManagedResource` — drop that from the original plan.

**Files:**
- Modify: `crates/resource/src/manager/mod.rs` (add `refresh_slot`/`revoke_slot`
  near the `register*` block ≈`:196-681`)
- Modify: `crates/resource/src/runtime/managed.rs` (add `taint()`,
  `drain_in_flight()`, `dispatch_on_refresh(slot)`, `dispatch_on_revoke(slot)`;
  these borrow the live `Self::Runtime` and call the `&self` hook — no slot map)
- Test: `crates/resource/tests/manager_refresh_slot.rs`

- [ ] **Step 1: Write the failing test**

```rust
// crates/resource/tests/manager_refresh_slot.rs
use nebula_resource::{Manager, ResourceContext};
// Build a Resident test resource whose Runtime counts on_credential_refresh calls.
#[tokio::test]
async fn refresh_slot_invokes_hook_with_runtime() {
    let mgr = Manager::new();
    // register a CountingResident at Workflow scope (helper in test module)
    let key = counting::register(&mgr).await;
    mgr.refresh_slot(&key, nebula_core::ScopeLevel::Workflow, "db")
        .await
        .expect("refresh_slot ok");
    assert_eq!(counting::refresh_calls(), 1, "hook fired exactly once with &Runtime");
}

#[tokio::test]
async fn revoke_slot_taints_then_drains_then_hooks() {
    let mgr = Manager::new();
    let key = counting::register(&mgr).await;
    mgr.revoke_slot(&key, nebula_core::ScopeLevel::Workflow, "db")
        .await
        .expect("revoke_slot ok");
    assert!(counting::was_tainted_before_hook(), "taint precedes on_credential_revoke");
    assert_eq!(counting::revoke_calls(), 1);
}
```

(Add a `mod counting` test helper in the same file: a `#[derive(Resource)]` Resident with an `AtomicUsize` Runtime; `register` uses `Manager::register_resident`.)

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-resource --test manager_refresh_slot`
Expected: FAIL — `Manager` has no `refresh_slot`/`revoke_slot`.

- [ ] **Step 3: Write minimal implementation**

In `crates/resource/src/manager/mod.rs` add (uses the existing registry lookup the `acquire_*` family already uses):

```rust
impl Manager {
    /// Engine-driven (D1 port). Apply a rotated slot to the live runtime.
    /// Idempotent; per-resource isolated (caller wraps in a timeout).
    pub async fn refresh_slot(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot_name: &str,
    ) -> Result<(), Error> {
        let managed = self.lookup_managed(key, scope)
            .ok_or_else(|| Error::not_found(key))?;
        let span = tracing::debug_span!(
            "nebula.resource.slot_refresh",
            key = %key.as_str(), slot = slot_name,
            topology = %managed.topology_tag()
        );
        let _e = span.enter();
        let res = managed.dispatch_on_refresh(slot_name).await; // calls Resource::on_credential_refresh(&self, slot, &Runtime)
        match &res {
            Ok(()) => { self.emit(ResourceEvent::SlotRefreshed { key: key.clone(), slot: slot_name.into() });
                        if let Some(m) = &self.metrics { m.record_slot_refresh(); } }
            Err(e) => { self.emit(ResourceEvent::SlotRefreshFailed { key: key.clone(), slot: slot_name.into(), error: e.to_string() });
                        if let Some(m) = &self.metrics { m.record_slot_refresh_error(); } }
        }
        res
    }

    /// Engine-driven. Taint (reject new acquires) → drain in-flight →
    /// invoke `on_credential_revoke`. Post-condition: no new acquire sees
    /// the revoked credential (ADR-0036).
    pub async fn revoke_slot(
        &self,
        key: &ResourceKey,
        scope: ScopeLevel,
        slot_name: &str,
    ) -> Result<(), Error> {
        let managed = self.lookup_managed(key, scope)
            .ok_or_else(|| Error::not_found(key))?;
        managed.taint();                       // reject new acquires NOW
        managed.drain_in_flight().await;       // ReleaseQueue + drain_tracker
        let res = managed.dispatch_on_revoke(slot_name).await;
        self.emit(ResourceEvent::SlotRevoked { key: key.clone(), slot: slot_name.into() });
        res
    }
}
```

In `crates/resource/src/runtime/managed.rs` add `taint()`, `drain_in_flight()`, `dispatch_on_refresh(slot)`, `dispatch_on_revoke(slot)`; `dispatch_on_refresh` borrows the live `Self::Runtime` and calls `resource.on_credential_refresh(slot, &runtime).await` per topology (Resident/Service/Transport/Exclusive: single runtime; Pooled: iterate pool instances). No slot map / no `__slot_lookup` — the resolved credential is read by the author via the generated `<field>_slot()` accessor over the struct-owned `SlotCell` (A3). A `register`/create-time credential-resolution step that calls `resource.<field>.store(...)` is wired by the engine (Task A10) — for THIS task's unit tests, construct the resource with its `SlotCell`s pre-populated via `SlotCell::store` in the test helper.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-resource --test manager_refresh_slot`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
task fmt
git add crates/resource/src/manager/mod.rs crates/resource/src/runtime/managed.rs crates/resource/src/slot.rs crates/resource/tests/manager_refresh_slot.rs
git commit -m "feat(resource): Manager::{refresh_slot,revoke_slot} port + slot install

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task A7: Metrics — `slot_refresh_total` / `slot_refresh_error_total`

**Files:**
- Modify: `crates/resource/src/metrics.rs` (`ResourceOpsMetrics` + `ResourceOpsSnapshot`)
- Test: `crates/resource/src/metrics.rs` `#[cfg(test)]`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn slot_refresh_counters_increment() {
    let reg = nebula_metrics::MetricsRegistry::new();
    let m = ResourceOpsMetrics::new(&reg).expect("metrics");
    m.record_slot_refresh();
    m.record_slot_refresh_error();
    let s = m.snapshot();
    assert_eq!(s.slot_refresh_total, 1);
    assert_eq!(s.slot_refresh_error_total, 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-resource metrics:: -- slot_refresh`
Expected: FAIL — no such fields/methods.

- [ ] **Step 3: Write minimal implementation**

Add two `Counter`s to `ResourceOpsMetrics::new` (names `nebula_metrics::naming::NEBULA_RESOURCE_SLOT_REFRESH_TOTAL` / `_ERROR_TOTAL` — add the constants in `nebula-metrics/src/naming.rs`), `record_slot_refresh`/`record_slot_refresh_error`, and the two snapshot fields.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-resource metrics:: -- slot_refresh`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
task fmt
git add crates/resource/src/metrics.rs crates/metrics/src/naming.rs
git commit -m "feat(resource): slot-refresh metrics counters + snapshot

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task A8: Structural dedup-key fix (abuse #1 — confirmed bug)

**Files:**
- Modify: `crates/resource/src/manager/mod.rs:265,423` (`config.fingerprint()` call sites) + `crates/resource/src/runtime/pool.rs:138` (`current_fingerprint`)
- Test: `crates/resource/tests/dedup_slot_identity.rs`

`ResourceConfig::fingerprint()` defaults to `0` (`resource.rs:64-66`) ⇒ same-type configs collapse to one runtime regardless of credential. Fix: the Manager dedup key = `(R::key(), ScopeLevel, slot_identity_hash)` where `slot_identity_hash` is a stable hash over the resolved `CredentialKey` of each declared slot — independent of the author `fingerprint()`.

- [ ] **Step 1: Write the failing test**

```rust
// crates/resource/tests/dedup_slot_identity.rs
#[tokio::test]
async fn same_type_different_credential_does_not_dedup() {
    let mgr = nebula_resource::Manager::new();
    // Register the SAME resident type at the SAME scope twice with default
    // fingerprint() but DIFFERENT resolved credential keys per slot.
    let h1 = dedup_fix::register_with_cred(&mgr, "cred-A").await;
    let h2 = dedup_fix::register_with_cred(&mgr, "cred-B").await;
    assert_ne!(dedup_fix::runtime_id(&mgr, h1).await,
               dedup_fix::runtime_id(&mgr, h2).await,
               "different credential per slot MUST yield distinct runtimes");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-resource --test dedup_slot_identity`
Expected: FAIL — with default `fingerprint()==0` both register calls dedupe to one runtime (assert_ne fails).

- [ ] **Step 3: Write minimal implementation**

Introduce a `DedupKey { resource_key, scope, slot_identity: u64 }` computed in the `register*`/dedup path. `slot_identity` = `fxhash`/`std::hash` over the sorted list of `(slot_key, resolved CredentialKey.as_str())` taken from the slot bindings the engine supplies (Task A6 slot install). Use it as the dedup map key instead of bare `config.fingerprint()`. Keep `fingerprint()` as an *additional* invalidation input for hot-reload (`:1162`), not the identity.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-resource --test dedup_slot_identity`
Expected: PASS. Also run `cargo test -p nebula-engine resource_integration -- cross_workflow` — the existing shared-resource dedup test must stay green (same key+scope+credential still dedupes).

- [ ] **Step 5: Commit**

```bash
task fmt
git add crates/resource/src/manager/mod.rs crates/resource/src/runtime/pool.rs crates/resource/tests/dedup_slot_identity.rs
git commit -m "fix(resource): dedup key includes resolved slot identity (abuse #1)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task A9: Engine reverse-index — `(CredentialId → [(ResourceKey,ScopeLevel,slot)])`

**Files:**
- Create: `crates/engine/src/credential/rotation/resource_fanout.rs`
- Modify: `crates/engine/src/credential/rotation.rs` (or `rotation/mod.rs`) to `mod resource_fanout; pub use`
- Test: in `resource_fanout.rs` `#[cfg(test)]`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn index_register_lookup_drain() {
    let idx = ResourceFanoutIndex::new();
    let cid = CredentialId::new_v4_for_test();
    let rk = nebula_core::ResourceKey::new("pg").unwrap();
    idx.bind(cid.clone(), rk.clone(), ScopeLevel::Workflow, "db");
    let hits = idx.affected(&cid);
    assert_eq!(hits, vec![(rk.clone(), ScopeLevel::Workflow, "db".to_string())]);
    idx.unbind_resource(&rk, ScopeLevel::Workflow);
    assert!(idx.affected(&cid).is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-engine resource_fanout -- index_register_lookup_drain`
Expected: FAIL — module/type absent.

- [ ] **Step 3: Write minimal implementation**

```rust
// crates/engine/src/credential/rotation/resource_fanout.rs
//! Engine-owned reverse index + fan-out for per-slot credential rotation
//! (finalization spec D1). nebula-resource exposes only the typed
//! Manager::{refresh_slot,revoke_slot} port; ownership of "which resources
//! does CredentialId X feed" is engine's, consistent with ADR-0030.
use dashmap::DashMap;
use smallvec::SmallVec;
use nebula_core::{ResourceKey, ScopeLevel};
use nebula_credential::CredentialId;

type Bind = (ResourceKey, ScopeLevel, String);

#[derive(Default)]
pub struct ResourceFanoutIndex {
    fwd: DashMap<CredentialId, SmallVec<[Bind; 2]>>,
}

impl ResourceFanoutIndex {
    pub fn new() -> Self { Self::default() }

    pub fn bind(&self, cid: CredentialId, key: ResourceKey, scope: ScopeLevel, slot: &str) {
        self.fwd.entry(cid).or_default().push((key, scope, slot.to_string()));
    }

    pub fn affected(&self, cid: &CredentialId) -> Vec<Bind> {
        self.fwd.get(cid).map(|v| v.clone().into_vec()).unwrap_or_default()
    }

    pub fn unbind_resource(&self, key: &ResourceKey, scope: ScopeLevel) {
        for mut e in self.fwd.iter_mut() {
            e.retain(|(k, s, _)| !(k == key && *s == scope));
        }
        self.fwd.retain(|_, v| !v.is_empty());
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-engine resource_fanout -- index_register_lookup_drain`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
task fmt
git add crates/engine/src/credential/rotation/resource_fanout.rs crates/engine/src/credential/rotation.rs
git commit -m "feat(engine): resource-fanout reverse index for per-slot rotation

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task A10: Engine fan-out — `join_all` with per-resource timeout isolation + bind/unbind wiring

**Files:**
- Modify: `crates/engine/src/credential/rotation/resource_fanout.rs` (add `dispatch_refresh`/`dispatch_revoke`)
- Modify: `crates/engine/src/credential/dispatchers.rs` (call fan-out after a refresh completes; subscribe to lease-revoke per ADR-0051)
- Modify: the engine register path that resolves a credential into a slot + calls `Manager::register_*` — call `index.bind(...)` there and `index.unbind_resource` on remove/shutdown
- Test: `crates/engine/tests/resource_rotation_fanout.rs`

- [ ] **Step 1: Write the failing test**

```rust
// crates/engine/tests/resource_rotation_fanout.rs
#[tokio::test]
async fn one_slow_resource_does_not_fail_siblings() {
    // 3 resources bound to the same CredentialId: #2 sleeps > per-resource budget.
    let h = fanout_fix::engine_with_three(/* #2 slow */).await;
    let outcome = h.rotate_credential(h.cid()).await;
    assert_eq!(outcome.ok, 2);
    assert_eq!(outcome.timed_out, 1);          // isolated, not cascaded
    assert!(h.sibling_runtimes_healthy());     // #1 and #3 refreshed
}

#[tokio::test]
async fn revoke_yields_zero_post_revoke_authenticated_acquire() {
    let h = fanout_fix::engine_with_one().await;
    h.revoke_credential(h.cid()).await;
    assert!(h.acquire_after_revoke().await.is_err(),
            "tainted runtime rejects new acquire (ADR-0036)");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-engine --test resource_rotation_fanout`
Expected: FAIL — dispatch path not wired.

- [ ] **Step 3: Write minimal implementation**

```rust
// resource_fanout.rs (add)
use std::time::Duration;
use futures::future::join_all;

#[derive(Debug, Default, PartialEq)]
pub struct RotationOutcome { pub ok: usize, pub failed: usize, pub timed_out: usize }

impl ResourceFanoutIndex {
    pub async fn dispatch_refresh(
        &self,
        cid: &CredentialId,
        mgr: &nebula_resource::Manager,
        per_resource: Duration,           // PER-RESOURCE budget — NOT global
    ) -> RotationOutcome {
        let targets = self.affected(cid);
        let futs = targets.into_iter().map(|(k, s, slot)| async move {
            match tokio::time::timeout(per_resource, mgr.refresh_slot(&k, s, &slot)).await {
                Ok(Ok(())) => Slot::Ok,
                Ok(Err(_)) => Slot::Failed,
                Err(_)     => Slot::TimedOut,
            }
        });
        let mut o = RotationOutcome::default();
        for r in join_all(futs).await { match r { Slot::Ok=>o.ok+=1, Slot::Failed=>o.failed+=1, Slot::TimedOut=>o.timed_out+=1 } }
        o
    }
    // dispatch_revoke: same shape, calls mgr.revoke_slot; revoke uses its own budget.
}
enum Slot { Ok, Failed, TimedOut }
```

In `dispatchers.rs`: after `RefreshCoordinator::refresh_coalesced` succeeds and the engine has stored the new material into the resource slot cell, call `index.dispatch_refresh(cid, manager, cfg.per_resource_rotation_timeout)`; emit `RotationOutcome` on `nebula-eventbus` (metrics fanout only — ADR-0028 §4, not audit). Subscribe lease-revoke (`LeaseEvent`, ADR-0051) → `dispatch_revoke`. Call `index.bind/unbind_resource` from the engine register/remove path.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-engine --test resource_rotation_fanout`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
task fmt
git add crates/engine/src/credential/
git commit -m "feat(engine): per-slot rotation fan-out (join_all, per-resource timeout)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task A11: Redaction test — no credential material in rotation observability

**Files:**
- Create: `crates/engine/tests/resource_rotation_redaction.rs` (mirrors ADR-0030 §4 `credential_refresh_redaction` pattern)

- [ ] **Step 1: Write the failing test**

```rust
// Inject a secret-bearing credential into the rotation path; capture all
// tracing spans + emitted ResourceEvents + metric labels; assert the secret
// substring never appears.
#[tokio::test]
async fn rotation_emits_no_secret_substring() {
    let secret = "SUPER-SECRET-TOKEN-9d3f";
    let cap = redaction::capture_all(); // tracing layer + event sink + metric sink
    redaction::rotate_with_secret(secret).await;
    assert!(!cap.contains(secret), "secret leaked into spans/events/metrics");
}
```

- [ ] **Step 2: Run test to verify it fails or passes**

Run: `cargo test -p nebula-engine --test resource_rotation_redaction`
Expected: PASS if A6/A10 spans carry only `key`/`slot`/`topology`/duration (designed so). If FAIL, fix the offending span/event field — never widen the test.

- [ ] **Step 3..5: (only if Step 2 failed) remove the leaking field, re-run, commit**

```bash
task fmt
git add crates/engine/tests/resource_rotation_redaction.rs crates/resource/src crates/engine/src
git commit -m "test(engine): rotation observability redaction gate (ADR-0030 §4)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task A12: Phase A verification gate

- [ ] **Step 1: Run the per-crate gates**

Run: `cargo nextest run -p nebula-resource -p nebula-engine`
Then: `cargo test -p nebula-resource --doc`
Then: `cargo clippy -p nebula-resource -p nebula-engine --all-targets -- -D warnings`
Expected: all green.

- [ ] **Step 2: Commit (no-op if clean) / fix-forward**

If clippy/doctests surface issues, fix inline, `task fmt`, commit `fix(resource): phase A cleanup`. Phase A done when green.

---

## Phase B — Engine-side typed-registration bridge

> Resource-side `Manager::register_from_value<R>(config_json, expr_engine, slot_bindings, …)` is **already implemented** (`manager/mod.rs:611-681`, verified). Phase B adds only the engine erased `kind→registrar` indirection.

### Task B1: `ErasedResourceRegistrar` trait + per-kind registry

**Files:**
- Create: `crates/engine/src/resource/registrar.rs`
- Modify: `crates/engine/src/resource/mod.rs` (or `engine.rs`) to expose it
- Test: in `registrar.rs` `#[cfg(test)]`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn known_kind_registers_unknown_kind_errors() {
    let mut reg = ResourceRegistrarRegistry::new();
    reg.insert("demo", demo_registrar());          // closed allowlist entry
    let mgr = nebula_resource::Manager::new();
    reg.register(&mgr, "demo", demo_entry_json(), demo_slot_bindings()).await
        .expect("known kind registers");
    let err = reg.register(&mgr, "ghost", serde_json::json!({}), Default::default()).await
        .expect_err("unknown kind must be a typed error");
    assert!(matches!(err, RegistrarError::UnknownKind(k) if k == "ghost"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p nebula-engine registrar -- known_kind`
Expected: FAIL — module absent.

- [ ] **Step 3: Write minimal implementation**

```rust
// crates/engine/src/resource/registrar.rs
//! Closed-allowlist kind -> typed register_from_value indirection
//! (finalization spec Track B + abuse #4). Built from PluginRegistry; no
//! reflection. Unknown kind => typed error, never a silent runtime grab.
use std::collections::HashMap;
use std::sync::Arc;
use nebula_core::CredentialKey;

#[derive(Debug, thiserror::Error, nebula_error::Classify)]
#[non_exhaustive]
pub enum RegistrarError {
    #[classify(category = "not_found", code = "ENGINE:RESOURCE_KIND_UNKNOWN")]
    #[error("unknown resource kind `{0}` (not in plugin-declared allowlist)")]
    UnknownKind(String),
    #[classify(category = "validation", code = "ENGINE:RESOURCE_REGISTER")]
    #[error("register_from_value failed for kind `{kind}`: {source}")]
    Register { kind: String, #[source] source: nebula_resource::Error },
}

#[async_trait::async_trait]
pub trait ErasedResourceRegistrar: Send + Sync {
    async fn register(
        &self,
        mgr: &nebula_resource::Manager,
        config_json: serde_json::Value,
        slot_bindings: HashMap<String, CredentialKey>,
    ) -> Result<(), nebula_resource::Error>;
}

#[derive(Default)]
pub struct ResourceRegistrarRegistry {
    by_kind: HashMap<String, Arc<dyn ErasedResourceRegistrar>>,
}

impl ResourceRegistrarRegistry {
    pub fn new() -> Self { Self::default() }
    pub fn insert(&mut self, kind: impl Into<String>, r: Arc<dyn ErasedResourceRegistrar>) {
        self.by_kind.insert(kind.into(), r);
    }
    pub async fn register(
        &self,
        mgr: &nebula_resource::Manager,
        kind: &str,
        config_json: serde_json::Value,
        slot_bindings: HashMap<String, CredentialKey>,
    ) -> Result<(), RegistrarError> {
        let r = self.by_kind.get(kind)
            .ok_or_else(|| RegistrarError::UnknownKind(kind.to_string()))?;
        r.register(mgr, config_json, slot_bindings).await
            .map_err(|source| RegistrarError::Register { kind: kind.to_string(), source })
    }
}
```

> Per-type `ErasedResourceRegistrar` impls are generated by `#[derive(Resource)]` (a thin blanket calling `mgr.register_from_value::<Self>(...)`); add that emission alongside Task A3 if not already covered, or hand-write per builtin resource. Registry is populated from `PluginRegistry` at engine startup (closed dependency graph, INTEGRATION_MODEL §114-120).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p nebula-engine registrar -- known_kind`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
task fmt
git add crates/engine/src/resource/
git commit -m "feat(engine): closed-allowlist kind->registrar bridge (Track B, abuse #4)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task B2: Populate the registry from `PluginRegistry` at startup

**Files:**
- Modify: `crates/engine/src/engine.rs` (where `PluginRegistry` is consumed at build) — build `ResourceRegistrarRegistry` from declared plugin resources; hold it in engine state
- Test: `crates/engine/tests/resource_registrar_from_plugins.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn plugin_declared_resources_become_registrable_kinds() {
    let engine = test_engine_with_plugin_declaring("demo").await;
    assert!(engine.resource_registrars().contains_kind("demo"));
    assert!(!engine.resource_registrars().contains_kind("not-declared"));
}
```

- [ ] **Step 2: Run to verify fail**

Run: `cargo test -p nebula-engine --test resource_registrar_from_plugins`
Expected: FAIL — no `resource_registrars()` accessor / not populated.

- [ ] **Step 3: Implement** — iterate the plugin registry's declared resources at engine build, `insert(kind, registrar)`; expose `fn resource_registrars(&self) -> &ResourceRegistrarRegistry`.

- [ ] **Step 4: Run to verify pass** — same command, Expected: PASS.

- [ ] **Step 5: Commit**

```bash
task fmt
git add crates/engine/src
git commit -m "feat(engine): populate resource registrars from PluginRegistry

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task B3: Secret-shaped config rejection regression (abuse #3)

**Files:**
- Test: `crates/resource/tests/register_from_value_rejects_secret.rs`

- [ ] **Step 1: Write the test**

```rust
#[tokio::test]
async fn config_with_inline_secret_field_is_rejected_by_schema() {
    // Config schema declares NO secret field; JSON carrying a `password`
    // key must fail schema validation in register_from_value.
    let mgr = nebula_resource::Manager::new();
    let bad = serde_json::json!({ "password": "p@ss", "host": "h" });
    let err = secretcfg::register(&mgr, bad).await.expect_err("must reject");
    assert!(err.to_string().contains("schema"), "rejected at schema validation");
}
```

- [ ] **Step 2: Run** — `cargo test -p nebula-resource --test register_from_value_rejects_secret` — Expected: PASS (schema validation already enforces typed Config). If it does NOT reject, that is a real gap — tighten `register_from_value` schema validation, do not weaken the test.

- [ ] **Step 3: Commit**

```bash
git add crates/resource/tests/register_from_value_rejects_secret.rs
git commit -m "test(resource): register_from_value rejects secret-shaped config (abuse #3)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Phase C — HTTP API surface (config CRUD + read-only status)

### Task C1: `AppState.resource_repo` + builder

**Files:**
- Modify: `crates/api/src/state.rs` (add field + `with_resource_repo`, mirroring `with_metrics_registry`)
- Test: `crates/api/src/state.rs` `#[cfg(test)]`

- [ ] **Step 1: Failing test**

```rust
#[test]
fn app_state_carries_resource_repo() {
    let st = test_state().with_resource_repo(std::sync::Arc::new(FakeResourceRepo::default()));
    assert!(st.resource_repo.is_some());
}
```

- [ ] **Step 2: Run** — `cargo test -p nebula-api state:: -- resource_repo` — Expected: FAIL (no field).

- [ ] **Step 3: Implement** — add `pub resource_repo: Option<Arc<dyn nebula_storage::ResourceRepo>>` to `AppState`, default `None`, builder `#[must_use] pub fn with_resource_repo(mut self, r: Arc<dyn ResourceRepo>) -> Self`.

- [ ] **Step 4: Run** — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
task fmt
git add crates/api/src/state.rs
git commit -m "feat(api): AppState.resource_repo + builder

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task C2: `list_resources` — replace the 501 stub honestly (ADR-0047)

**Files:**
- Modify: `crates/api/src/handlers/resource.rs` (remove `#[deprecated]` + 501 response)
- Modify: `crates/api/src/models/resource.rs` (pagination if needed)
- Test: `crates/api/tests/resource_handlers.rs`

- [ ] **Step 1: Failing test**

```rust
#[tokio::test]
async fn list_resources_returns_page_not_501() {
    let app = test_app_with_resources(&["pg", "redis"]).await;
    let r = app.get("/orgs/o/workspaces/w/resources").await;
    assert_eq!(r.status(), 200);
    let body: ListResourcesResponse = r.json().await;
    assert_eq!(body.resources.len(), 2);
}
```

- [ ] **Step 2: Run** — `cargo test -p nebula-api --test resource_handlers -- list_resources` — Expected: FAIL (handler returns 501).

- [ ] **Step 3: Implement** — body: `let repo = state.resource_repo.as_ref().ok_or(ApiError::Internal("resource repo not configured".into()))?; let rows = repo.list(ws_id_bytes).await.map_err(map_storage)?;` map `Vec<ResourceEntry>` → `Vec<ResourceSummary>`; `#[utoipa::path]` 200/401/403/500, **drop** `deprecated` + the 501 entry, drop ` (planned)` tag suffix.

- [ ] **Step 4: Run** — Expected: PASS.

- [ ] **Step 5: Commit**

```bash
task fmt
git add crates/api/src/handlers/resource.rs crates/api/src/models/resource.rs crates/api/tests/resource_handlers.rs
git commit -m "feat(api)!: implement list_resources, drop ADR-0047 501 stub

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task C3: `get_resource`

**Files:** Modify `crates/api/src/handlers/resource.rs`; Test `crates/api/tests/resource_handlers.rs`

- [ ] **Step 1: Failing test** — `get` known id ⇒ 200 + `ResourceSummary`; unknown ⇒ 404 `ProblemDetails`.
- [ ] **Step 2: Run** — Expected: FAIL (no handler).
- [ ] **Step 3: Implement** — `repo.get(id).await?` → `Option`; `None` ⇒ `ApiError::NotFound`; `#[utoipa::path] get` 200/401/403/404/500.
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(api): get_resource handler` (+ trailer).

---

### Task C4: `create_resource` (validates via Phase B bridge)

**Files:** Modify `crates/api/src/handlers/resource.rs`; Test `crates/api/tests/resource_handlers.rs`

- [ ] **Step 1: Failing test** — POST valid body ⇒ 201 + id; POST with schema-invalid config ⇒ 422 `ProblemDetails`; POST unknown `kind` ⇒ 422/404 (engine `RegistrarError::UnknownKind`).
- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement** — persist `ResourceEntry` via `repo.create`; validation goes through the engine `ResourceRegistrarRegistry::register` (Task B1) so schema + closed-kind checks run before persistence; map `RegistrarError::UnknownKind` ⇒ 422, schema error ⇒ 422.
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(api): create_resource with schema+kind validation` (+ trailer).

---

### Task C5: `update_resource` (CAS) + `delete_resource` (soft)

**Files:** Modify `crates/api/src/handlers/resource.rs`; Test `crates/api/tests/resource_handlers.rs`

- [ ] **Step 1: Failing test** — PUT with stale `expected_version` ⇒ 409; correct ⇒ 200. DELETE ⇒ 204 then `get` ⇒ 404.
- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement** — `repo.update(&entry, expected_version)` mapping CAS mismatch ⇒ `ApiError::Conflict`; `repo.soft_delete(id)`.
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(api): update (CAS) + delete (soft) resource` (+ trailer).

---

### Task C6: `get_resource_status` (read-only projection — NO lifecycle ops, D3)

**Files:** Modify `crates/api/src/handlers/resource.rs`, `crates/api/src/models/resource.rs`; Test `crates/api/tests/resource_handlers.rs`

- [ ] **Step 1: Failing test**

```rust
#[tokio::test]
async fn status_is_readonly_projection_without_secrets() {
    let app = test_app_with_running_resource("pg").await;
    let r = app.get("/orgs/o/workspaces/w/resources/res_x/status").await;
    assert_eq!(r.status(), 200);
    let s: ResourceStatusDto = r.json().await;
    assert!(matches!(s.phase.as_str(), "Ready"|"Degraded"|"Failed"|"Unregistered"));
    // No acquire/release endpoint exists:
    assert_eq!(app.post("/orgs/o/workspaces/w/resources/res_x/acquire").await.status(), 404);
}
```

- [ ] **Step 2: Run** — Expected: FAIL (no status handler).
- [ ] **Step 3: Implement** — read `ResourcePhase` + `ResourceOpsSnapshot` from the engine-held `Manager` (via `AppState`; add an optional `resource_manager: Option<Arc<nebula_resource::Manager>>` to `AppState` if not present, read-only use), project to `ResourceStatusDto { phase: String, healthy: bool, ops: OpsSnapshotDto }` — ADR-0047 wrappers, **no** secret/credential fields. Do **not** add acquire/release routes (D3).
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit** — `feat(api): read-only resource status projection` (+ trailer).

---

### Task C7: Route registration + OpenAPI drift check

**Files:** Modify `crates/api/src/routes/workspace.rs`; Test: build + `cargo test -p nebula-api`

- [ ] **Step 1: Failing check** — `rg "handlers::resource" crates/api/src/routes/workspace.rs` shows only `list_resources`.
- [ ] **Step 2: Implement** — register all handlers:

```rust
.routes(routes!(handlers::resource::list_resources))
.routes(routes!(handlers::resource::get_resource))
.routes(routes!(handlers::resource::create_resource))
.routes(routes!(handlers::resource::update_resource))
.routes(routes!(handlers::resource::delete_resource))
.routes(routes!(handlers::resource::get_resource_status))
```

- [ ] **Step 3: Run** — `cargo test -p nebula-api` (utoipa-axum compile gate fails build if any handler lacks `#[utoipa::path]`); also assert the generated spec has no `501` for `…/resources*` and no `deprecated: true`.
- [ ] **Step 4: Commit**

```bash
task fmt
git add crates/api/src/routes/workspace.rs
git commit -m "feat(api): register resource CRUD + status routes (OpenAPI honest, ADR-0047)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Phase D — Closure (§M12.4)

### Task D1: ADR-0052 — engine-owned fan-out + `&self` hook (supersession record)

**Files:**
- Create: `docs/adr/0052-engine-owned-rotation-fanout-self-refresh-hook.md`
- Modify: `docs/adr/README.md` (index row + Supersession table row: 0044 hook signature → 0052)

- [ ] **Step 1: Verify next number**

Run: `ls docs/adr | grep -oE '^00[0-9]{2}' | sort -u | tail -1`
Expected: `0051` ⇒ use `0052`.

- [ ] **Step 2: Write the ADR**

Sections: Context (PHASE4_BLOCKED §1 left reentrancy + ownership open; ADR-0030 says engine owns orchestration; a pure `#[proc_macro_derive]` cannot add fields so ADR-0044's bare-`CredentialGuard`-field migration shape is unimplementable); Decision D1 (engine `resource_fanout` reverse-index + `join_all` per-resource timeout; `Manager::{refresh_slot,revoke_slot}` narrow port), D2 (`&self` + `&Runtime` hook **and** `#[credential]` field type = `SlotCell<CredentialGuard<C>>` on the author struct + generated `<field>_slot()` accessor + framework `SlotCell::store` population; **supersedes ADR-0044's hook signature AND its slot-field/migration-note shape** — the slot-binding *declaration* model of 0044 — `#[credential(key=…)]` per-field — is untouched; only the field *type* and read pattern change), D3 (API config-CRUD + read-only status; no lifecycle over HTTP); Abuse invariants 1–4 (esp. structural dedup-key fix); **Deferred** section: R-006/R-041/R-042/R-050/R-052 with trigger conditions (verbatim from spec Track D); Consequences (call out the ~33 impl-site + README contract churn from the field-type change); Supersession (overrides `PHASE4_BLOCKED.md §1` candidate; supersedes ADR-0044 hook signature + migration shape).

- [ ] **Step 3: Update `docs/adr/README.md`** — add the `| 0052 | … | accepted (2026-05-15) | resource, engine, credential, rotation, api, m11 |` index row and a Supersession row `0044 (hook signature + slot-field/migration shape) | 0052 | &mut self→&self+&Runtime; #[credential] field type CredentialGuard<C>→SlotCell<CredentialGuard<C>> + <field>_slot() accessor`.

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0052-engine-owned-rotation-fanout-self-refresh-hook.md docs/adr/README.md
git commit -m "docs(resource): ADR-0052 engine-owned rotation fan-out + &self refresh hook

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

### Task D2: Topology docstring sweep (PHASE4_BLOCKED §4)

**Files:** Modify `crates/resource/src/topology/{pooled,resident,service,transport,exclusive}.rs`

- [ ] **Step 1: Find stale refs** — `rg -n "scheme|Scheme|R::Credential|type Credential|&mut self.*refresh" crates/resource/src/topology`
- [ ] **Step 2: Edit** — remove scheme-threading references; align any `on_credential_refresh` mentions with the D2 `&self, slot_name, &Runtime` shape. No behavioural change.
- [ ] **Step 3: Verify** — `RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc -p nebula-resource --no-deps` green.
- [ ] **Step 4: Commit** — `docs(resource): topology docstring sweep (drop scheme threading)` (+ trailer).

---

### Task D3: Concerns-register reconciliation note

**Files:** Modify `C:/Users/vanya/RustroverProjects/docs/tracking/nebula-resource-concerns-register.md` (parent tree — append-only "Register updates")

- [ ] **Step 1:** Append a `2026-05-15` update entry: R-002/R-003/R-004/R-060 superseded by ADR-0052 engine-side fan-out (П2 machinery was Phase-4-deleted; rebuilt engine-owned); R-040 confirmed resolved (`deny.toml:108`); R-006/R-041/R-042/R-050/R-052 carried into ADR-0052 "Deferred" with triggers; register **retires on the MATURITY flip** (Task D4).
- [ ] **Step 2:** This parent tree is **not a git repo** — do not `git commit` it (per doc-authority memory). Save only.
- [ ] **Step 3:** No commit (parent tree). Proceed.

---

### Task D4: Honest MATURITY flip (parent canon — gated on A+B+C green)

**Files:** Modify `C:/Users/vanya/RustroverProjects/docs/MATURITY.md:37` (parent tree, non-git)

- [ ] **Step 1: Gate check** — confirm Phase A+B+C verification gates passed (`task dev:check` in Step D5 must be green first; if not, STOP — do not flip).
- [ ] **Step 2: Edit row 37** — `nebula-resource` Engine-integration column `partial (lifecycle visible; CAS guards partial)` → `stable` (mirror the credential `partial → stable` honest-upgrade phrasing at `MATURITY.md:66`; add a short parenthetical: "per-slot rotation fan-out engine-owned, ADR-0052"). Taxonomy stays `frontier`/`stable` (NOT `core`).
- [ ] **Step 3:** Parent tree, non-git — save only, no commit.

---

### Task D5: Final workspace verification gate

- [ ] **Step 1: Run the full pre-PR gate**

Run: `task dev:check`
Expected: fmt + clippy `-D warnings` + nextest + doctests + deny — all green workspace-wide.

- [ ] **Step 2: Examples build**

Run: `task build` (or `cargo build -p nebula-examples`)
Expected: green (migrated examples from Task A4 compile + run shape intact).

- [ ] **Step 3: deny layer check**

Run: `task deny`
Expected: green — only `nebula-engine → nebula-resource` edge used (no `resource → engine`); `deny.toml:108,121-133` rules satisfied.

- [ ] **Step 4: Final commit (if any gate fix-forward needed)**

```bash
task fmt
git add -A
git commit -m "chore(resource): finalization verification gate green (M11.5 + M12.4 closed)

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage:**
- D1 engine-owned fan-out → A9, A10 (+ ADR D1). ✓
- D2 `&self`+`&Runtime` hook + ArcSwap slots → A1, A2, A3, A4. ✓
- D3 API config-CRUD + read-only status, no lifecycle HTTP → C2–C7 (C6 asserts no acquire route). ✓
- Abuse #1 structural dedup key → A8. ✓
- Abuse #2 revoke taint→drain ordering → A6 (`revoke_slot`), A10 (revoke test). ✓
- Abuse #3 secret-config rejection → B3. ✓
- Abuse #4 closed kind allowlist → B1. ✓
- DoD (typed error + span + event + metrics) → A2/A5/A6/A7/A11. ✓
- Track B engine bridge (resource side verified done) → B1, B2. ✓
- Track D ADR-0052 + docstrings + concerns reconcile + MATURITY + gate → D1–D5. ✓
- §M11.5 = Phase A; §M12.4 = Phase D. ✓ No spec requirement left unmapped.

**2. Placeholder scan:** No "TBD/TODO/handle appropriately". Test bodies are concrete; helper modules (`counting`, `fanout_fix`, `redaction`, `dedup_fix`, `secretcfg`) are named and their contract stated in-task (skilled engineer writes the fixture from the stated shape — acceptable, not a code placeholder). The one design note in A3 (slot storage via framework registry) is an explicit instruction, not a deferral.

**3. Type consistency:** `SlotCell::{empty,store,load}` consistent A1↔A3↔A6. Hook signature `on_credential_refresh(&self, slot_name:&str, runtime:&Self::Runtime)` identical A2↔A3↔A4↔D2. `Manager::refresh_slot(&ResourceKey, ScopeLevel, &str)`/`revoke_slot` identical A6↔A9↔A10. `ResourceEvent::{SlotRefreshed,SlotRevoked,SlotRefreshFailed}` identical A5↔A6. `RotationOutcome{ok,failed,timed_out}` A10. `ResourceRegistrarRegistry::register(... ) -> Result<(),RegistrarError>` / `RegistrarError::UnknownKind` consistent B1↔B2↔C4. `AppState.resource_repo: Option<Arc<dyn ResourceRepo>>` C1↔C2. No drift.
