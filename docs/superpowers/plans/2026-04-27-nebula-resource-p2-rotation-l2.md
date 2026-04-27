# nebula-resource П2 — Rotation L2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `Manager::on_credential_refreshed` / `on_credential_revoked` `todo!()` panics with reverse-index population + parallel `join_all` dispatcher + per-resource timeout isolation + failure semantics + observability events. Closes 🔴-1 (silent revocation drop), 🔴-3 (rotation thundering-herd), 🔴-4 (drain-abort phase corruption). Also lands Manager file-split (Tech Spec §5.4), drain-abort fix (R-023), `warmup_pool` security amendment B-3 fix, and `OnCredentialRefresh<C>` deprecated trait removal.

**Architecture:** Type-erased trampoline trait `ResourceDispatcher` + generic `TypedDispatcher<R>` wrapper. `Manager` populates `DashMap<CredentialId, Vec<Arc<dyn ResourceDispatcher>>>` at register time when `R::Credential != NoCredential` (TypeId check). `on_credential_refreshed`/`on_credential_revoked` read this map, fan out via `tokio::time::timeout` per-resource + `futures::future::join_all`. Blue-green pool swap stays in resource impls (`Arc<RwLock<Pool>>` pattern, not Manager-orchestrated). Per-resource isolation invariant: one slow/failing resource never poisons siblings (security amendment B-1).

**Tech Stack:** Rust 1.95, tokio, `futures::future::join_all`, dashmap, tracing, `nebula-credential` П1 primitives (`Credential`, `SchemeGuard`, `CredentialContext`, `CredentialId`).

**Source documents:**
- [docs/adr/0036-resource-credential-adoption-auth-retirement.md](../../adr/0036-resource-credential-adoption-auth-retirement.md) — §Decision bullets 5-8 (rotation dispatch, blue-green, warmup ban, no clone)
- [docs/superpowers/specs/2026-04-24-nebula-resource-tech-spec.md](../specs/2026-04-24-nebula-resource-tech-spec.md) — §3 (dispatcher), §5.1 (blue-green), §5.4 (file-split), §5.5 (drain-abort), §6 (observability)
- [docs/superpowers/specs/2026-04-24-nebula-resource-redesign-strategy.md](../specs/2026-04-24-nebula-resource-redesign-strategy.md) — §4.2, §4.3, §4.6, §4.9
- [docs/tracking/nebula-resource-concerns-register.md](../../tracking/nebula-resource-concerns-register.md) — closes R-002, R-003, R-004, R-005, R-023, R-060

**Closes (concerns register):**
- R-002 — `credential_resources` reverse-index never written → silent drop
- R-003 — `on_credential_revoked` semantics (invariant + mechanism)
- R-004 — Rotation dispatch parallel isolation
- R-005 — `warmup_pool` no `Scheme::default()`
- R-023 — Drain-abort phase corruption fix
- R-060 — Rotation observability (span/counter/event)

**Non-goals (explicitly deferred):**
- Daemon/EventSource extraction → П3 (ADR-0037)
- Doc rewrite (Architecture.md, events.md, remaining api-reference.md sections) → П4
- `RegisterOptions::tainting_policy` knob (SL-1) — Tech Spec §5.6 deferred
- `warmup_pool_by_id` ergonomic helper (SL-2) — Tech Spec §5.6 deferred
- `FuturesUnordered` concurrency cap — CP2 commits to unbounded `join_all` for N ≤ 32
- Histogram bucket tuning — CP3 §11

---

## File Structure

### New files

| File | Purpose |
|---|---|
| `crates/resource/src/manager/mod.rs` | New module root after file-split — `Manager` struct, `register_*`, `acquire_*`, `subscribe_events`, `is_accepting`, `lookup` |
| `crates/resource/src/manager/options.rs` | `ManagerConfig`, `RegisterOptions`, `ShutdownConfig`, `DrainTimeoutPolicy`, `RotationConfig` |
| `crates/resource/src/manager/registration.rs` | `register_inner` (private), reverse-index write logic, error constructors |
| `crates/resource/src/manager/gate.rs` | `GateAdmission`, `admit_through_gate` |
| `crates/resource/src/manager/execute.rs` | `execute_with_resilience`, `validate_pool_config` |
| `crates/resource/src/manager/rotation.rs` | `ResourceDispatcher` trait, `TypedDispatcher<R>`, `on_credential_refreshed`, `on_credential_revoked`, outcome enums |
| `crates/resource/src/manager/shutdown.rs` | `graceful_shutdown`, `set_phase_all`, `set_phase_all_failed` (drain-abort fix) |
| `crates/resource/tests/rotation.rs` | Integration tests for rotation dispatch (fixtures + 10+ test cases) |
| `crates/resource/tests/probes/resource_refresh_retention.rs` | Compile-fail probe — `Resource::on_credential_refresh` retention barrier |
| `crates/resource/tests/compile_fail_resource_refresh_retention.rs` | trybuild driver |

### Modified files

| File | Change |
|---|---|
| `crates/resource/src/manager.rs` → DELETED (split into `manager/` directory) | — |
| `crates/resource/src/lib.rs` | Update internal imports if any — `pub use` surface unchanged |
| `crates/resource/src/error.rs` | Add `Error::missing_credential_id`, `Error::scheme_type_mismatch` constructors |
| `crates/resource/src/events.rs` | Add `ResourceEvent::{CredentialRefreshed, CredentialRevoked}` variants |
| `crates/resource/src/metrics.rs` | Add counter + histogram fields for rotation |
| `crates/resource/src/runtime/managed.rs` | Remove `#[expect(dead_code)]` from `set_failed`, ensure event emission wired |
| `crates/resource/Cargo.toml` | Add `trybuild` to `[dev-dependencies]` |
| `crates/credential/src/secrets/scheme_guard.rs` | DELETE `OnCredentialRefresh<C>` trait def (~80 lines) |
| `crates/credential/src/secrets/mod.rs` | Remove `OnCredentialRefresh` re-export + `#[allow(deprecated)]` |
| `crates/credential/src/lib.rs` | Remove `OnCredentialRefresh` from `pub use` block |

### Verification commands

```
cargo check -p nebula-resource
cargo check -p nebula-credential
cargo check --workspace
cargo nextest run -p nebula-resource --profile ci --no-tests=pass
cargo nextest run -p nebula-credential --profile ci --no-tests=pass
cargo nextest run --workspace --profile ci --no-tests=pass
cargo +nightly fmt --all -- --check
cargo clippy --workspace -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-resource -p nebula-credential --no-deps
```

---

## Task 1: Foundation — outcome enums + Error constructors

**Files:**
- Modify: `crates/resource/src/error.rs`
- Modify: (later) `crates/resource/src/manager/rotation.rs` will import these enums; for Task 1 they live alongside Error

**Why:** Self-contained foundation. Task 2 dispatcher trait references `RefreshOutcome` / `RevokeOutcome`; Task 4-5 dispatch loops emit them; tests assert on them. Building first lets every later task reference the canonical types.

- [ ] **Step 1: Add outcome enums in `crates/resource/src/error.rs` (or a new sibling file)**

Recommend: keep close to `Error` since outcomes carry `crate::Error` for the failed case. Append to `error.rs` after the `Error` impl:

```rust
/// Outcome of a single resource's `on_credential_refresh` invocation.
#[derive(Debug, Clone)]
pub enum RefreshOutcome {
    /// Resource successfully applied the new scheme.
    Ok,
    /// Resource returned an error from `on_credential_refresh`.
    Failed(crate::Error),
    /// Per-resource timeout budget exceeded.
    TimedOut { budget: std::time::Duration },
}

/// Outcome of a single resource's `on_credential_revoke` invocation.
#[derive(Debug, Clone)]
pub enum RevokeOutcome {
    Ok,
    Failed(crate::Error),
    TimedOut { budget: std::time::Duration },
}

/// Aggregate counts derived from a rotation cycle's per-resource outcomes.
///
/// Constructed at event-emission time from `Vec<(ResourceKey, RefreshOutcome)>`
/// or its revoke counterpart. Used in `ResourceEvent::CredentialRefreshed` /
/// `CredentialRevoked` payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RotationOutcome {
    pub ok: usize,
    pub failed: usize,
    pub timed_out: usize,
}

impl RotationOutcome {
    /// Total resources affected by the rotation cycle.
    pub fn total(&self) -> usize {
        self.ok + self.failed + self.timed_out
    }

    /// True if any resource did not complete the hook successfully.
    pub fn has_partial_failure(&self) -> bool {
        self.failed + self.timed_out > 0
    }
}
```

Note: if `error.rs` becomes too large or starts mixing concerns, split into `outcomes.rs` and re-export. Decide based on file LOC after this task — if `error.rs` exceeds ~600 LOC after additions, split.

- [ ] **Step 2: Add `Error` constructors**

Append to the `impl Error` block (or wherever existing constructors live):

```rust
impl Error {
    /// A credential-bearing resource was registered without a `credential_id`.
    pub fn missing_credential_id(key: ResourceKey) -> Self {
        Self::permanent(format!(
            "{}: credential-bearing Resource (Credential != NoCredential) requires a credential_id at register time",
            key
        ))
    }

    /// A dispatcher failed to downcast `&(dyn Any)` to the expected `<R::Credential as Credential>::Scheme`.
    /// Indicates a dispatcher bug — the engine passed a scheme of the wrong type.
    pub fn scheme_type_mismatch<R: crate::resource::Resource>() -> Self {
        Self::permanent(format!(
            "{}: scheme type mismatch — dispatcher expected <{} as Credential>::Scheme",
            R::key(),
            std::any::type_name::<R::Credential>()
        ))
    }
}
```

- [ ] **Step 3: Add re-exports in `crates/resource/src/lib.rs`**

Add to the existing `error::*` re-export block:

```rust
pub use error::{
    // ... existing ...
    RefreshOutcome, RevokeOutcome, RotationOutcome,
};
```

- [ ] **Step 4: Smoke test**

Append to `crates/resource/tests/basic_integration.rs` (or new `crates/resource/tests/rotation_outcome.rs`):

```rust
#[test]
fn rotation_outcome_aggregates_correctly() {
    use nebula_resource::{RefreshOutcome, RotationOutcome};

    let outcomes = vec![
        RefreshOutcome::Ok,
        RefreshOutcome::Ok,
        RefreshOutcome::Failed(nebula_resource::Error::permanent("test")),
        RefreshOutcome::TimedOut { budget: std::time::Duration::from_secs(30) },
    ];

    let agg = RotationOutcome {
        ok: outcomes.iter().filter(|o| matches!(o, RefreshOutcome::Ok)).count(),
        failed: outcomes.iter().filter(|o| matches!(o, RefreshOutcome::Failed(_))).count(),
        timed_out: outcomes.iter().filter(|o| matches!(o, RefreshOutcome::TimedOut { .. })).count(),
    };

    assert_eq!(agg.total(), 4);
    assert_eq!(agg.ok, 2);
    assert!(agg.has_partial_failure());
}

#[test]
fn missing_credential_id_error_carries_key() {
    let err = nebula_resource::Error::missing_credential_id(
        nebula_core::resource_key!("test.resource")
    );
    assert!(format!("{err}").contains("test.resource"));
    assert!(format!("{err}").contains("credential_id"));
}
```

- [ ] **Step 5: Compile + run**

```
cargo nextest run -p nebula-resource --test basic_integration --profile ci --no-tests=pass
cargo +nightly fmt -p nebula-resource --
cargo clippy -p nebula-resource -- -D warnings
```

- [ ] **Step 6: Commit**

```
feat(resource): rotation outcome enums + error constructors (П2 foundation)

Lays the typed-failure substrate for the rotation dispatcher landing
in the next commits:
- RefreshOutcome / RevokeOutcome — per-resource hook result
- RotationOutcome — aggregate (ok/failed/timed_out counts)
- Error::missing_credential_id — register-time invariant
- Error::scheme_type_mismatch — dispatcher safety net

No behavior change — types only. Two unit tests cover aggregation
and error formatting.
```

---

## Task 2: ResourceDispatcher trait + TypedDispatcher trampoline

**Files:**
- Create: `crates/resource/src/manager/rotation.rs` (NEW — note: directory `manager/` doesn't exist yet; **see Task 11 file-split**. For Task 2, create `crates/resource/src/rotation.rs` at top level temporarily; Task 11 moves it under `manager/`.)

**Why:** The trampoline is the core type-erasure mechanism that lets `Manager::credential_resources` store heterogeneous `Resource` impls behind one trait object. Object-safe trait + generic wrapper is the canonical Rust pattern for this.

- [ ] **Step 1: Create `crates/resource/src/rotation.rs`**

```rust
//! Rotation dispatcher infrastructure for credential refresh / revoke.
//!
//! See ADR-0036 §Decision and Tech Spec §3.2-§3.5.
//!
//! `ResourceDispatcher` is an object-safe trampoline trait that stores
//! type-erased per-resource dispatch logic in `Manager::credential_resources`.
//! `TypedDispatcher<R>` is the generic implementor that downcasts
//! `&(dyn Any)` schemes to `<R::Credential as Credential>::Scheme` and
//! invokes the resource's `on_credential_refresh` / `on_credential_revoke`.

use std::any::{Any, TypeId};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use nebula_core::ResourceKey;
use nebula_credential::{Credential, CredentialId};

use crate::resource::Resource;
use crate::runtime::managed::ManagedResource;

/// Object-safe trampoline for type-erased rotation dispatch.
///
/// Stored as `Arc<dyn ResourceDispatcher>` in `Manager::credential_resources`.
/// Implementations (currently only `TypedDispatcher<R>`) must downcast schemes
/// to the resource's expected `<R::Credential as Credential>::Scheme` and
/// forward to `Resource::on_credential_refresh` / `on_credential_revoke`.
pub(crate) trait ResourceDispatcher: Send + Sync + 'static {
    /// Resource key for diagnostics + event emission.
    fn resource_key(&self) -> ResourceKey;

    /// `TypeId` of the resource's `<R::Credential as Credential>::Scheme`.
    /// Used by the engine to verify the scheme it's about to pass matches.
    fn scheme_type_id(&self) -> TypeId;

    /// Per-resource timeout override set at register time, or None to use Manager default.
    fn timeout_override(&self) -> Option<Duration>;

    /// Dispatch refresh: downcast `scheme` to expected type, forward to
    /// `Resource::on_credential_refresh`. Returns boxed future to keep the trait
    /// object-safe (RPITIT not allowed on dyn-safe traits).
    fn dispatch_refresh<'a>(
        &'a self,
        scheme: &'a (dyn Any + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send + 'a>>;

    /// Dispatch revoke: forward `credential_id` to `Resource::on_credential_revoke`.
    fn dispatch_revoke<'a>(
        &'a self,
        credential_id: &'a CredentialId,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send + 'a>>;
}

/// Typed wrapper that adapts `Resource::on_credential_refresh` / `on_credential_revoke`
/// to the object-safe `ResourceDispatcher` interface.
pub(crate) struct TypedDispatcher<R: Resource> {
    pub(crate) managed: Arc<ManagedResource<R>>,
    pub(crate) timeout_override: Option<Duration>,
}

impl<R: Resource> TypedDispatcher<R> {
    pub(crate) fn new(managed: Arc<ManagedResource<R>>, timeout_override: Option<Duration>) -> Self {
        Self { managed, timeout_override }
    }
}

impl<R: Resource> ResourceDispatcher for TypedDispatcher<R> {
    fn resource_key(&self) -> ResourceKey {
        R::key()
    }

    fn scheme_type_id(&self) -> TypeId {
        TypeId::of::<<R::Credential as Credential>::Scheme>()
    }

    fn timeout_override(&self) -> Option<Duration> {
        self.timeout_override
    }

    fn dispatch_refresh<'a>(
        &'a self,
        scheme: &'a (dyn Any + Send + Sync),
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send + 'a>> {
        Box::pin(async move {
            let scheme: &<R::Credential as Credential>::Scheme = scheme
                .downcast_ref::<<R::Credential as Credential>::Scheme>()
                .ok_or_else(crate::Error::scheme_type_mismatch::<R>)?;

            // The current Resource::on_credential_refresh signature accepts
            // SchemeGuard<'a, _> + &'a CredentialContext, NOT a bare &Scheme.
            // For П2 dispatcher integration we need to either:
            //   (a) construct a SchemeGuard from &Scheme, OR
            //   (b) extend the trait with a borrowed-Scheme-only variant.
            //
            // Option (a) requires SchemeFactory/SchemeGuard pub(crate) constructor
            // access from outside nebula-credential. Option (b) violates the
            // ADR-0036 canonical CP5 form.
            //
            // RESOLUTION: Engine side passes SchemeGuard, dispatcher's `scheme: &dyn Any`
            // erases the guard type. Manager dispatch loop (Task 4) reconstructs
            // a SchemeGuard from the engine-provided one and passes it through.
            //
            // For now, we pin a sentinel TODO and the Task 4 implementation
            // adapts. If the credential-side API needs a tweak, surface it
            // there as a NEEDS_CONTEXT escalation.
            todo!("TASK 4: wire SchemeGuard reconstruction from engine-provided scheme");
            #[allow(unreachable_code)]
            { let _ = scheme; Ok(()) }
        })
    }

    fn dispatch_revoke<'a>(
        &'a self,
        credential_id: &'a CredentialId,
    ) -> Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send + 'a>> {
        Box::pin(async move {
            self.managed
                .resource()
                .on_credential_revoke(credential_id)
                .await
                .map_err(Into::into)
        })
    }
}
```

> **CRITICAL note for the implementer:** The `dispatch_refresh` method body has an open design question — `Resource::on_credential_refresh` takes `SchemeGuard<'a, Self::Credential>` + `&'a CredentialContext`, not a bare `&Scheme`. Three options:
>
> 1. **Engine passes `SchemeGuard` directly to Manager dispatcher** — change Manager dispatcher signature to take `SchemeGuard<'a, ?>` (but it can't be generic — `Manager` is a single type). This means the dispatcher would need a custom typed entry point per credential type, defeating the type-erasure.
> 2. **Add a `SchemeFactory::borrow_with_lifetime<'a>(&'a Self::Scheme) -> SchemeGuard<'a, C>` constructor** in `nebula-credential` that takes a borrowed scheme and produces a guard with the same lifetime. This is a credential-side API addition.
> 3. **Extend `Resource` trait** with a `_dyn` variant of `on_credential_refresh` that takes `&Scheme` instead of `SchemeGuard` — anti-pattern, defeats the lifetime-pin invariant.
>
> **Recommended path: option 2.** Add `pub fn from_borrow<'a>(scheme: &'a C::Scheme) -> SchemeGuard<'a, C>` to `nebula-credential::SchemeGuard`. The borrow checker enforces non-retention identically to the existing constructor. This keeps the dispatcher trampoline clean.
>
> Task 2 lands the trampoline with `todo!()` in `dispatch_refresh`; Task 4 adds the credential-side helper if needed and wires it. If credential-side API changes feel out of П2 scope, escalate as NEEDS_CONTEXT before implementing.

- [ ] **Step 2: Wire the module in `crates/resource/src/lib.rs`**

Add `mod rotation;` (private — module is pub(crate)). Don't re-export `ResourceDispatcher` or `TypedDispatcher` publicly — they're crate-internal infrastructure.

- [ ] **Step 3: Compile-only smoke**

```
cargo check -p nebula-resource
```

Expected: clean (the `todo!()` body compiles).

Note: there are no tests yet for this module — Task 14 covers integration testing. Task 2 is a structural foundation.

- [ ] **Step 4: Commit**

```
feat(resource): ResourceDispatcher trampoline + TypedDispatcher (П2 dispatcher)

Object-safe trampoline trait that lets Manager store heterogeneous
Resource impls behind one trait object in credential_resources DashMap.

TypedDispatcher<R> generic wrapper downcasts &dyn Any schemes to
<R::Credential as Credential>::Scheme and forwards to Resource hooks.

dispatch_refresh body deferred — needs credential-side
SchemeGuard::from_borrow helper (or equivalent) to bridge dyn-erased
borrowed scheme to the typed SchemeGuard signature on Resource trait.
Task 4 of the П2 plan resolves this.

dispatch_revoke is fully wired since Resource::on_credential_revoke
takes a plain &CredentialId.
```

---

## Task 3: Retype `credential_resources` field + `register_inner` reverse-index write

**Files:**
- Modify: `crates/resource/src/manager.rs` (or `manager/registration.rs` if Task 11 file-split happens first — see decision below)

**Decision: do file-split FIRST (Task 11) or LAST?**

Per Tech Spec §5.4, file-split is structural-only and happens in CP2. Two approaches:

- **(A) Split first (Task 11 before Task 3):** clean target structure for all subsequent tasks. Larger initial diff but later tasks land cleanly.
- **(B) Split last:** all behavior changes land in `manager.rs`, then file-split is a pure mechanical move. Smaller intermediate diffs but file-split commit moves freshly-written code.

**Recommendation: (B) split last.** All Tasks 3-10 modify `manager.rs`. Splitting first means each task touches the freshly-split files; splitting last means one big mechanical move at the end. (B) is more reviewer-friendly because each behavior commit shows its changes in a familiar file.

**Plan adopts (B): Tasks 3-10 work in `manager.rs`; Task 11 splits.**

- [ ] **Step 1: Retype field at `crates/resource/src/manager.rs:262`**

Find:
```rust
credential_resources: dashmap::DashMap<CredentialId, Vec<ResourceKey>>,
```

Replace with:
```rust
credential_resources: dashmap::DashMap<CredentialId, Vec<Arc<dyn crate::rotation::ResourceDispatcher>>>,
```

Update the field doc comment to reflect the new shape.

- [ ] **Step 2: Add `register_inner` private helper**

Inside `impl Manager`, near the existing `register<R>` (around line 348), add:

```rust
fn register_inner<R: Resource>(
    &self,
    managed: Arc<ManagedResource<R>>,
    credential_id: Option<CredentialId>,
    timeout_override: Option<Duration>,
) -> Result<(), Error> {
    use std::any::TypeId;
    use nebula_credential::NoCredential;

    let opted_out = TypeId::of::<R::Credential>() == TypeId::of::<NoCredential>();

    match (opted_out, credential_id) {
        (true, Some(_)) => {
            tracing::warn!(
                resource = %R::key(),
                "register: NoCredential resource provided a credential_id; ignoring"
            );
        }
        (true, None) => {} // Normal: NoCredential, no reverse-index write.
        (false, None) => {
            return Err(Error::missing_credential_id(R::key()));
        }
        (false, Some(id)) => {
            let dispatcher: Arc<dyn crate::rotation::ResourceDispatcher> =
                Arc::new(crate::rotation::TypedDispatcher::new(
                    Arc::clone(&managed),
                    timeout_override,
                ));
            self.credential_resources
                .entry(id)
                .or_default()
                .push(dispatcher);
        }
    }
    Ok(())
}
```

- [ ] **Step 3: Wire `register_inner` into the existing `register<R>` and `register_*<R>` paths**

The existing `register<R>` (around `manager.rs:348`) currently builds `ManagedResource<R>` with `credential_id: None` and stores it in the registry but does NOT touch `credential_resources`. Insert a call to `register_inner` AFTER the registry write, BEFORE returning:

```rust
// existing: build managed, store in registry...
self.register_inner(Arc::clone(&managed), credential_id, options.credential_rotation_timeout)?;
```

Where `credential_id` and `options.credential_rotation_timeout` come from `RegisterOptions`. Wait — these don't exist yet. **Task 6 adds them.** For Task 3, accept `credential_id: None` and `timeout_override: None` from the existing API (Task 6 will retrofit).

But we also can't bind a credential without an `Option<CredentialId>` parameter. So Task 3 lands the field retype + `register_inner` skeleton; the call site in `register<R>` passes `None, None` — meaning ALL resources stay opted out (no behavior change). Task 6 adds the `RegisterOptions` field; **Task 6's commit is where the wire-up actually starts populating the index.**

- [ ] **Step 4: Existing `register_*<R>` shorthand variants**

All 10 `register_*<R>` (and `_with` variants) bind `R: Resource<Credential = NoCredential>` per П1. Their `register_inner(_, None, None)` calls always hit the `opted_out=true, credential_id=None` branch — no-op. Confirm by inspection that all 10 variants pass through `register<R>` (they should funnel through the generic version).

If any shorthand has its own registry-write path, add `self.register_inner(Arc::clone(&managed), None, None)?;` to that path too.

- [ ] **Step 5: Compile + tests**

```
cargo check -p nebula-resource
cargo nextest run -p nebula-resource --profile ci --no-tests=pass
```

Expected: 220+ tests pass (Task 1 added 2 new ones). Behavior unchanged because:
- All in-tree resources use `Credential = NoCredential` → all hit opted-out branch
- Task 6 hasn't added `credential_id` to `RegisterOptions` yet, so `register_inner` always sees `None`

- [ ] **Step 6: Commit**

```
feat(resource): retype credential_resources field + register_inner skeleton

Field: DashMap<CredentialId, Vec<ResourceKey>> →
       DashMap<CredentialId, Vec<Arc<dyn ResourceDispatcher>>>

New private register_inner helper performs the TypeId opt-out check
and reverse-index write when R::Credential != NoCredential.

This commit lands the plumbing; actual reverse-index writes start
when Task 6 adds credential_id to RegisterOptions and consumers
register credential-bearing resources. Today all in-tree resources
are NoCredential-bound, so the opted_out branch always fires.
```

---

## Task 4: `Manager::on_credential_refreshed` dispatch loop

**Files:**
- Modify: `crates/resource/src/manager.rs` (replace `todo!()` at lines ~1394-1417)
- Possibly modify: `crates/credential/src/secrets/scheme_guard.rs` (add `SchemeGuard::from_borrow` helper if needed — see Task 2 critical note)

**Why:** This is the central П2 change. The `todo!()` panic is replaced with the parallel `join_all` dispatcher per Tech Spec §3.2.

- [ ] **Step 1: Resolve the `dispatch_refresh` body design question (from Task 2)**

Read `crates/credential/src/secrets/scheme_guard.rs` and decide:

- If `SchemeGuard::from_borrow<'a>(scheme: &'a C::Scheme) -> SchemeGuard<'a, C>` (or equivalent) already exists → use it.
- If not → add it. Sketch:

```rust
// In crates/credential/src/secrets/scheme_guard.rs
impl<'a, C: Credential> SchemeGuard<'a, C> {
    /// Construct a borrowed SchemeGuard from a `&Scheme` reference.
    /// The borrow checker enforces non-retention identically to `new()`.
    pub fn from_borrow(scheme: &'a <C as Credential>::Scheme) -> Self
    where
        C::Scheme: 'a,  // implied but spelled out for clarity
    {
        // Implementation needs to wrap &Scheme in the same way SchemeGuard::new wraps owned Scheme.
        // If SchemeGuard's internals don't support borrowed schemes (only owned),
        // this is where the credential-side API needs structural change — escalate as NEEDS_CONTEXT.
        todo!("see SchemeGuard internals; if owned-only, escalate")
    }
}
```

If `SchemeGuard` stores `scheme: <C as Credential>::Scheme` (owned), it cannot wrap a borrow without internals change. Two paths:

- **(α) Structural change to `SchemeGuard`** — add a borrowed variant (`enum SchemeGuard<'a, C> { Owned(C::Scheme), Borrowed(&'a C::Scheme) }`). Significant credential-side change.
- **(β) Engine never erases the guard** — Manager dispatcher takes `Box<dyn Fn(SchemeGuard<...>) -> Future<...>>` instead of a bare `&dyn Any` scheme. Caller-side closure construction. Possible but uglier.

**Implementer must escalate this design choice before implementing.** Expected NEEDS_CONTEXT. If the controller decides on (α), Task 4 also lands the credential-side change and increases scope; if (β), the dispatcher trampoline shape changes.

For the rest of this Task 4 description, assume option (α) is chosen and `SchemeGuard::from_borrow` exists.

- [ ] **Step 2: Replace `Manager::on_credential_refreshed` body**

In `crates/resource/src/manager.rs` around lines 1394-1417, replace the existing body (which ends in `todo!()`) with:

```rust
pub async fn on_credential_refreshed(
    &self,
    credential_id: &CredentialId,
    scheme: &(dyn std::any::Any + Send + Sync),
    ctx: &CredentialContext,
) -> Result<Vec<(ResourceKey, RefreshOutcome)>, Error> {
    use futures::future::join_all;

    let dispatchers = self
        .credential_resources
        .get(credential_id)
        .map(|entry| entry.value().clone())
        .unwrap_or_default();

    if dispatchers.is_empty() {
        return Ok(Vec::new());
    }

    let span = tracing::info_span!(
        "resource.credential_refresh",
        credential_id = %credential_id,
        resources_affected = dispatchers.len(),
    );

    let _guard = span.enter();

    let default_timeout = self.config.credential_rotation_timeout;
    let futures = dispatchers.into_iter().map(|d| {
        let timeout = d.timeout_override().unwrap_or(default_timeout);
        let key = d.resource_key();
        let scheme: &(dyn std::any::Any + Send + Sync) = scheme;

        async move {
            let dispatch = d.dispatch_refresh(scheme);
            let outcome = match tokio::time::timeout(timeout, dispatch).await {
                Ok(Ok(())) => RefreshOutcome::Ok,
                Ok(Err(e)) => RefreshOutcome::Failed(e),
                Err(_) => RefreshOutcome::TimedOut { budget: timeout },
            };

            // Per-resource event/metric emission inline so cancellation
            // of the outer future doesn't lose individual results.
            // (See Task 7 + Task 8 for event/metric details.)

            (key, outcome)
        }
    });

    let results: Vec<(ResourceKey, RefreshOutcome)> = join_all(futures).await;

    // Emit aggregate event (Task 7 wires this).
    // Update counters (Task 8 wires this).

    Ok(results)
}
```

> **Note on `ctx: &CredentialContext`:** the engine passes the context. Manager forwards it into `dispatch_refresh` if needed (the `Resource::on_credential_refresh` signature wants `&'a CredentialContext`). The current `ResourceDispatcher::dispatch_refresh(&dyn Any)` skeleton from Task 2 doesn't accept a context — extend the trait method to take `ctx: &'a CredentialContext` and propagate.

- [ ] **Step 3: Update `ResourceDispatcher::dispatch_refresh` signature to accept `&CredentialContext`**

In `crates/resource/src/rotation.rs`:

```rust
fn dispatch_refresh<'a>(
    &'a self,
    scheme: &'a (dyn Any + Send + Sync),
    ctx: &'a CredentialContext,   // NEW
) -> Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send + 'a>>;
```

And in `TypedDispatcher::dispatch_refresh` body:

```rust
fn dispatch_refresh<'a>(
    &'a self,
    scheme: &'a (dyn Any + Send + Sync),
    ctx: &'a CredentialContext,
) -> Pin<Box<dyn Future<Output = Result<(), crate::Error>> + Send + 'a>> {
    Box::pin(async move {
        let scheme: &<R::Credential as Credential>::Scheme = scheme
            .downcast_ref::<<R::Credential as Credential>::Scheme>()
            .ok_or_else(crate::Error::scheme_type_mismatch::<R>)?;

        // Construct SchemeGuard from borrow (Step 1 helper)
        let guard = nebula_credential::SchemeGuard::<'a, R::Credential>::from_borrow(scheme);

        self.managed
            .resource()
            .on_credential_refresh(guard, ctx)
            .await
            .map_err(Into::into)
    })
}
```

- [ ] **Step 4: Add `RotationConfig` to `ManagerConfig`** *(this anticipates Task 6 — split if needed)*

In the existing `ManagerConfig` struct (around `manager.rs:140-180`):

```rust
pub struct ManagerConfig {
    // ... existing fields ...

    /// Default per-resource timeout for credential rotation hooks.
    /// Overridable via `RegisterOptions::credential_rotation_timeout`.
    /// Default: 30 seconds.
    pub credential_rotation_timeout: Duration,
}

impl Default for ManagerConfig {
    fn default() -> Self {
        Self {
            // ... existing ...
            credential_rotation_timeout: Duration::from_secs(30),
        }
    }
}
```

If Task 6 lands this separately, do not duplicate — coordinate ordering.

- [ ] **Step 5: Compile + run existing tests (no regressions)**

```
cargo nextest run -p nebula-resource --profile ci --no-tests=pass
```

Expected: existing 220+ tests pass. Real rotation tests come in Task 14.

- [ ] **Step 6: Commit**

```
feat(resource): wire on_credential_refreshed dispatcher (П2 §3.2)

Replaces todo!() with parallel join_all dispatcher with per-resource
timeout isolation. Each resource future has its own timeout budget;
one slow/failing resource never poisons siblings (security amendment B-1).

Returns Vec<(ResourceKey, RefreshOutcome)>. Outer Err reserved for
setup-time failures only — per-resource errors aggregate into
RefreshOutcome::Failed / TimedOut variants.

ResourceDispatcher::dispatch_refresh extended to accept &CredentialContext
so the per-resource future can construct a SchemeGuard with the
correct lifetime per ADR-0036 canonical CP5 form.

Adds ManagerConfig::credential_rotation_timeout (default 30s).

If credential-side SchemeGuard::from_borrow was added in this
commit, also documents the cross-crate dep update.
```

---

## Task 5: `Manager::on_credential_revoked` dispatch loop

**Files:**
- Modify: `crates/resource/src/manager.rs` (replace `todo!()` at lines ~1424-1439)

**Why:** Symmetric to Task 4 but simpler — `on_credential_revoke` only takes `&CredentialId` (no scheme), so no downcasting concerns. The wrinkle is the `HealthChanged` emission per security amendment B-2.

- [ ] **Step 1: Replace `Manager::on_credential_revoked` body**

```rust
pub async fn on_credential_revoked(
    &self,
    credential_id: &CredentialId,
) -> Result<Vec<(ResourceKey, RevokeOutcome)>, Error> {
    use futures::future::join_all;

    let dispatchers = self
        .credential_resources
        .get(credential_id)
        .map(|entry| entry.value().clone())
        .unwrap_or_default();

    if dispatchers.is_empty() {
        return Ok(Vec::new());
    }

    let span = tracing::warn_span!(
        "resource.credential_revoke",
        credential_id = %credential_id,
        resources_affected = dispatchers.len(),
    );

    let _guard = span.enter();

    let default_timeout = self.config.credential_rotation_timeout;
    let event_tx = self.event_tx.clone();
    let futures = dispatchers.into_iter().map(|d| {
        let timeout = d.timeout_override().unwrap_or(default_timeout);
        let key = d.resource_key();
        let credential_id = credential_id.clone();
        let event_tx = event_tx.clone();

        async move {
            let dispatch = d.dispatch_revoke(&credential_id);
            let outcome = match tokio::time::timeout(timeout, dispatch).await {
                Ok(Ok(())) => RevokeOutcome::Ok,
                Ok(Err(e)) => RevokeOutcome::Failed(e),
                Err(_) => RevokeOutcome::TimedOut { budget: timeout },
            };

            // Per security amendment B-2: emit HealthChanged{healthy:false}
            // for any non-Ok revocation outcome. Successful revocations
            // emit only the aggregate CredentialRevoked event (Task 7).
            if !matches!(outcome, RevokeOutcome::Ok) {
                let _ = event_tx.send(crate::events::ResourceEvent::HealthChanged {
                    key: key.clone(),
                    healthy: false,
                });
            }

            (key, outcome)
        }
    });

    let results: Vec<(ResourceKey, RevokeOutcome)> = join_all(futures).await;

    // Emit aggregate event (Task 7) + counter (Task 8).

    Ok(results)
}
```

- [ ] **Step 2: Compile + smoke**

```
cargo check -p nebula-resource
cargo nextest run -p nebula-resource --profile ci --no-tests=pass
```

- [ ] **Step 3: Commit**

```
feat(resource): wire on_credential_revoked dispatcher (П2 §3.2 + B-2)

Symmetric to on_credential_refreshed. Per security amendment B-2,
emits HealthChanged{healthy:false} per resource where revocation
outcome is not Ok (failed or timed_out). Successful revocations
emit only the aggregate CredentialRevoked event (Task 7).
```

---

## Task 6: `RotationConfig` + `RegisterOptions` extension

**Files:**
- Modify: `crates/resource/src/options.rs` (extend `RegisterOptions`)
- Modify: `crates/resource/src/manager.rs` (wire `register_inner` to read from `options.credential_rotation_timeout` + `options.credential_id`)

**Why:** Tasks 4 and 5 reference `options.credential_rotation_timeout` and `register_inner` needs `credential_id` from `RegisterOptions`. This task formalizes those API additions.

- [ ] **Step 1: Extend `RegisterOptions` in `crates/resource/src/options.rs`**

```rust
pub struct RegisterOptions {
    // ... existing fields (scope, resilience, recovery_gate) ...

    /// Credential ID this resource binds to. Required for resources where
    /// `R::Credential != NoCredential`; ignored otherwise (Manager logs a
    /// warning if provided alongside `Credential = NoCredential`).
    pub credential_id: Option<CredentialId>,

    /// Per-resource override for the default credential rotation timeout.
    /// `None` falls back to `ManagerConfig::credential_rotation_timeout`.
    pub credential_rotation_timeout: Option<Duration>,
}

impl RegisterOptions {
    pub fn with_credential_id(mut self, id: CredentialId) -> Self {
        self.credential_id = Some(id);
        self
    }

    pub fn with_rotation_timeout(mut self, timeout: Duration) -> Self {
        self.credential_rotation_timeout = Some(timeout);
        self
    }
}
```

- [ ] **Step 2: Wire `register_inner` to read from options**

In `crates/resource/src/manager.rs::register<R>`:

```rust
self.register_inner(
    Arc::clone(&managed),
    options.credential_id.clone(),
    options.credential_rotation_timeout,
)?;
```

- [ ] **Step 3: Update `register_*<R>` shorthand variants to keep `credential_id: None` unchanged**

The 10 shorthand variants bind `Credential = NoCredential` so they always pass `None`. Verify by inspection.

- [ ] **Step 4: Compile + tests**

```
cargo nextest run -p nebula-resource --profile ci --no-tests=pass
```

Expected: 220+ tests pass; behavior unchanged because no test currently provides a `credential_id`.

- [ ] **Step 5: Commit**

```
feat(resource): RegisterOptions credential_id + rotation_timeout fields

Per Tech Spec §3.3-§3.4. Required for credential-bearing resources
to bind to a specific CredentialId; per-resource timeout override
supersedes the Manager-wide default.

NoCredential-bound resources never need either field — they remain
opted out of the reverse-index per Task 3's TypeId check.
```

---

## Task 7: `ResourceEvent::CredentialRefreshed` / `CredentialRevoked` variants

**Files:**
- Modify: `crates/resource/src/events.rs`
- Modify: `crates/resource/src/manager.rs` (wire emit at end of dispatch loops in Tasks 4/5)

- [ ] **Step 1: Add variants to `ResourceEvent`**

```rust
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ResourceEvent {
    // ... existing ...

    /// Aggregate event for one credential refresh cycle.
    CredentialRefreshed {
        credential_id: CredentialId,
        resources_affected: usize,
        outcome: RotationOutcome,  // Counts of ok/failed/timed_out
    },

    /// Aggregate event for one credential revocation cycle.
    CredentialRevoked {
        credential_id: CredentialId,
        resources_affected: usize,
        outcome: RotationOutcome,
    },
}
```

- [ ] **Step 2: Wire emit in Tasks 4/5 dispatch loops**

After `let results = join_all(futures).await;`:

```rust
// In on_credential_refreshed:
let outcome = RotationOutcome {
    ok: results.iter().filter(|(_, o)| matches!(o, RefreshOutcome::Ok)).count(),
    failed: results.iter().filter(|(_, o)| matches!(o, RefreshOutcome::Failed(_))).count(),
    timed_out: results.iter().filter(|(_, o)| matches!(o, RefreshOutcome::TimedOut { .. })).count(),
};
let _ = self.event_tx.send(ResourceEvent::CredentialRefreshed {
    credential_id: credential_id.clone(),
    resources_affected: results.len(),
    outcome,
});
```

Symmetric for revoke.

- [ ] **Step 3: Compile + tests**

```
cargo nextest run -p nebula-resource --profile ci --no-tests=pass
```

- [ ] **Step 4: Commit**

```
feat(resource): CredentialRefreshed/CredentialRevoked events (П2 §6.3)

Aggregate events emitted at the end of each rotation cycle. Per-resource
HealthChanged{healthy:false} on revocation failure (security amendment B-2)
already emitted inline in Task 5 — this is the aggregate cycle event.
```

---

## Task 8: Observability metrics + tracing spans

**Files:**
- Modify: `crates/resource/src/metrics.rs`
- Modify: `crates/resource/src/manager.rs` (wire counter increments + histogram observations)

- [ ] **Step 1: Extend `ResourceOpsMetrics` (or sibling registry struct)**

Find the existing metrics registry definition. Add:

```rust
pub struct ResourceOpsMetrics {
    // ... existing fields ...

    pub credential_rotation_attempts: Counter<u64>,
    pub credential_revoke_attempts: Counter<u64>,
    pub credential_rotation_dispatch_latency: Histogram<f64>,
}
```

Counter labels: `outcome ∈ {success, failed, timed_out}`.

Histogram label: `outcome` (matches counter); buckets: default `[0.001, 0.01, 0.1, 1.0, 10.0, 60.0]` seconds (deferred per Tech Spec §6.2 for tuning).

- [ ] **Step 2: Wire counter + histogram in dispatch loops**

In Task 4's `on_credential_refreshed` per-resource future, replace the inline-event-emit comment with:

```rust
let dispatch_start = std::time::Instant::now();
let dispatch = d.dispatch_refresh(scheme, ctx);
let outcome = match tokio::time::timeout(timeout, dispatch).await {
    Ok(Ok(())) => RefreshOutcome::Ok,
    Ok(Err(e)) => RefreshOutcome::Failed(e),
    Err(_) => RefreshOutcome::TimedOut { budget: timeout },
};
let elapsed = dispatch_start.elapsed().as_secs_f64();

let outcome_label = match &outcome {
    RefreshOutcome::Ok => "success",
    RefreshOutcome::Failed(_) => "failed",
    RefreshOutcome::TimedOut { .. } => "timed_out",
};

if let Some(metrics) = &self.metrics {
    metrics.credential_rotation_attempts
        .with_label_values(&[outcome_label])
        .inc();
    metrics.credential_rotation_dispatch_latency
        .with_label_values(&[outcome_label])
        .observe(elapsed);
}
```

Symmetric for `on_credential_revoked` (using `credential_revoke_attempts`).

- [ ] **Step 3: Verify trace spans from Task 4/5 fire correctly**

Run a manual smoke (or wait for Task 14 integration tests).

- [ ] **Step 4: Commit**

```
feat(resource): rotation observability metrics + spans (П2 §6.2)

Counters:
- nebula_resource.credential_rotation_attempts{outcome}
- nebula_resource.credential_revoke_attempts{outcome}

Histogram (default buckets, tuning deferred to CP3 §11):
- nebula_resource.credential_rotation_dispatch_latency_seconds{outcome}

Spans (already emitted in Tasks 4/5):
- resource.credential_refresh (INFO)
- resource.credential_revoke (WARN)

Cardinality bounded — outcome label has 3 values, no high-cardinality
labels (no resource key in label, no credential_id in label).
```

---

## Task 9: Drain-abort fix (R-023)

**Files:**
- Modify: `crates/resource/src/manager.rs` (around lines 1493-1510 — `DrainTimeoutPolicy::Abort` branch)
- Modify: `crates/resource/src/runtime/managed.rs` (remove `#[expect(dead_code)]` from `set_failed`)

**Why:** Phase 1 🔴-4 — current `DrainTimeoutPolicy::Abort` calls `set_phase_all(ResourcePhase::Ready)` while returning `ShutdownError::DrainTimeout`. Phase corruption: registry says Ready but caller saw timeout. Fix wires `set_failed`.

- [ ] **Step 1: Add `Manager::set_phase_all_failed`**

```rust
fn set_phase_all_failed(&self, error: ShutdownError) {
    for managed in self.registry.all_managed() {
        managed.set_failed(error.clone());
    }
}
```

- [ ] **Step 2: Replace `set_phase_all(Ready)` in `DrainTimeoutPolicy::Abort` branch**

Find the existing block (around `manager.rs:1493-1510`):

```rust
if let DrainTimeoutPolicy::Abort = self.config.drain_policy {
    let outstanding = self.drain_tracker.0.load(Ordering::SeqCst);
    let err = ShutdownError::DrainTimeout { outstanding };
    self.set_phase_all(ResourcePhase::Ready); // BUG: phase corruption
    return Err(err);
}
```

Replace with:

```rust
if let DrainTimeoutPolicy::Abort = self.config.drain_policy {
    let outstanding = self.drain_tracker.0.load(Ordering::SeqCst);
    let err = ShutdownError::DrainTimeout { outstanding };
    self.set_phase_all_failed(err.clone());
    return Err(err);
}
```

- [ ] **Step 3: Wire `ManagedResource::set_failed` event emission**

In `crates/resource/src/runtime/managed.rs`, find `set_failed` (currently `#[expect(dead_code)]`):

```rust
#[expect(dead_code)] // ← REMOVE
pub fn set_failed(&self, error: impl Into<crate::Error>) {
    // existing body
}
```

Remove the `#[expect(dead_code)]` attribute. Verify the body emits `ResourceEvent::HealthChanged { healthy: false }` — if not, add it.

- [ ] **Step 4: Add regression test**

In `crates/resource/tests/basic_integration.rs` (or new `tests/shutdown_drain_abort.rs`):

```rust
#[tokio::test]
async fn drain_abort_sets_phase_failed_not_ready() {
    let manager = Manager::new(Default::default());
    manager.register_pooled::<MyHangingResource>(
        // resource that holds outstanding leases past drain_timeout
    ).await.unwrap();

    let _lease = manager.acquire_pooled::<MyHangingResource>(&ctx, &Default::default())
        .await.unwrap();

    let result = manager.graceful_shutdown(
        ShutdownConfig::default()
            .with_drain_timeout(Duration::from_millis(50))
            .with_drain_policy(DrainTimeoutPolicy::Abort)
    ).await;

    assert!(matches!(result, Err(ShutdownError::DrainTimeout { .. })));

    // The fix: phase should be Failed, not Ready.
    let phase = manager.health_check::<MyHangingResource>(&ScopeLevel::Default)
        .unwrap().phase;
    assert_eq!(phase, ResourcePhase::Failed);
}
```

- [ ] **Step 5: Compile + test**

```
cargo nextest run -p nebula-resource --profile ci --no-tests=pass
```

- [ ] **Step 6: Commit**

```
fix(resource): drain-abort no longer corrupts phase to Ready (R-023)

DrainTimeoutPolicy::Abort previously called set_phase_all(Ready) before
returning ShutdownError::DrainTimeout — registry said Ready, caller saw
timeout. Phase corruption broke the invariant that Ready resources are
acquirable.

Wires the previously-dead-coded ManagedResource::set_failed via a new
Manager::set_phase_all_failed helper. Resources transition to Failed
on abort with HealthChanged{healthy:false} event emission.

Regression test: drain_abort_sets_phase_failed_not_ready.
```

---

## Task 10: `warmup_pool` security amendment B-3 fix

**Files:**
- Modify: `crates/resource/src/manager.rs` (around line 1288 — `warmup_pool<R>`)

**Why:** Closes R-005. Tech Spec §5.2 specifies two methods (`warmup_pool` for credential-bearing, `warmup_pool_no_credential` for `NoCredential`) with compile-time type-level enforcement.

- [ ] **Step 1: Rename existing `warmup_pool` → `warmup_pool_no_credential`**

The current INTERIM `warmup_pool` with `<R::Credential as Credential>::Scheme: Default` bound is structurally restricted to `NoCredential` (only `()` has `Default`). Rename and tighten:

```rust
pub async fn warmup_pool_no_credential<R>(&self, ctx: &ResourceContext) -> Result<usize, Error>
where
    R: crate::topology::pooled::Pooled<Credential = nebula_credential::NoCredential> + Clone + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Into<R::Runtime> + Send + 'static,
{
    let managed = self.lookup::<R>(&ctx.scope_level())?;
    let config = managed.config();
    let scheme = (); // NoCredential::Scheme = ()
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

The `R: Pooled<Credential = NoCredential>` bound is type-equality — the compiler refuses any non-NoCredential resource at the call site.

- [ ] **Step 2: Add credential-bearing `warmup_pool<R>` accepting borrowed scheme**

```rust
pub async fn warmup_pool<R>(
    &self,
    scheme: &<R::Credential as Credential>::Scheme,
    ctx: &ResourceContext,
) -> Result<usize, Error>
where
    R: crate::topology::pooled::Pooled + Clone + Send + Sync + 'static,
    R::Runtime: Clone + Into<R::Lease> + Send + Sync + 'static,
    R::Lease: Into<R::Runtime> + Send + 'static,
{
    let managed = self.lookup::<R>(&ctx.scope_level())?;
    let config = managed.config();
    match &managed.topology {
        TopologyRuntime::Pool(rt) => {
            let count = rt.warmup(&managed.resource, &config, scheme, ctx).await;
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

- [ ] **Step 3: Update existing call sites in tests**

Search test fixtures for `warmup_pool` calls. All in-tree test resources are `NoCredential`-bound, so they should call `warmup_pool_no_credential`. Update via find-replace.

- [ ] **Step 4: Compile + tests**

```
cargo nextest run -p nebula-resource --profile ci --no-tests=pass
```

Expected: tests still pass, just with renamed call.

- [ ] **Step 5: Commit**

```
fix(resource): warmup_pool security amendment B-3 (R-005)

Previously: <R::Credential as Credential>::Scheme: Default bound +
<...>::default() call. Worked structurally for NoCredential (Scheme = ())
but tantalized any future `impl Default for RealScheme` to silently warm
pools with empty credentials.

Now (per Tech Spec §5.2):
- warmup_pool_no_credential<R> where R::Credential = NoCredential
  (compile-time gate; passes &() internally)
- warmup_pool<R>(scheme: &<R::Credential as Credential>::Scheme, ctx)
  for credential-bearing resources

No call to Scheme::default() anywhere on the warmup hot path.
```

---

## Task 11: Manager file-split (Tech Spec §5.4)

**Files:**
- Delete: `crates/resource/src/manager.rs`
- Create: `crates/resource/src/manager/mod.rs`, `options.rs`, `registration.rs`, `gate.rs`, `execute.rs`, `rotation.rs` (move from top-level), `shutdown.rs`
- Modify: `crates/resource/src/lib.rs` (`mod manager;` declaration)

**Why:** Tech Spec §5.4 — structural-only, NO public API change. This is a pure mechanical refactor; all the behavior changes from Tasks 1-10 land first, then this task moves them into a coherent module structure.

- [ ] **Step 1: Confirm `manager.rs` LOC**

```bash
wc -l crates/resource/src/manager.rs
```

Should be ~2150-2300 lines after Tasks 1-10. Will become 7 smaller files.

- [ ] **Step 2: Create directory structure and move code**

Split per the file structure table at the top of this plan:

| Module | Move from `manager.rs` |
|---|---|
| `manager/mod.rs` | `Manager` struct definition, `new`, `subscribe_events`, `is_accepting`, `lookup`, `register*`, `acquire*`, public re-exports |
| `manager/options.rs` | `ManagerConfig`, `RegisterOptions`, `ShutdownConfig`, `DrainTimeoutPolicy` (and any other config types) |
| `manager/registration.rs` | `register_inner` + helpers |
| `manager/gate.rs` | `GateAdmission` and related gate code |
| `manager/execute.rs` | `execute_with_resilience`, `validate_pool_config` |
| `manager/rotation.rs` | Move `crates/resource/src/rotation.rs` here; keep `ResourceDispatcher`, `TypedDispatcher`, `on_credential_refreshed`, `on_credential_revoked`, `RefreshOutcome`, etc. |
| `manager/shutdown.rs` | `graceful_shutdown`, `set_phase_all`, `set_phase_all_failed` (+ drain-abort logic) |

Note: `RefreshOutcome`/`RevokeOutcome`/`RotationOutcome` from Task 1 move to `manager/rotation.rs` (or stay in `error.rs` — pick whichever is more cohesive after the move).

- [ ] **Step 3: Update `crates/resource/src/lib.rs` imports**

Change `mod manager;` if the old `manager.rs` declaration was different. Verify all `pub use manager::{...}` re-exports still resolve.

- [ ] **Step 4: Run gate**

```
cargo check -p nebula-resource
cargo nextest run -p nebula-resource --profile ci --no-tests=pass
cargo clippy -p nebula-resource -- -D warnings
cargo +nightly fmt -p nebula-resource --
```

Expected: zero behavior change, all tests pass. The diff is mostly mechanical moves with some `pub(crate)` adjustments for cross-module access.

- [ ] **Step 5: Commit**

```
refactor(resource): split manager.rs into 7 submodules (Tech Spec §5.4)

Pure structural move. No public API change — all imports continue
to resolve at nebula_resource::*.

Breakdown:
- manager/mod.rs       — Manager struct + register*/acquire* surface
- manager/options.rs   — Config types
- manager/registration.rs — register_inner + helpers
- manager/gate.rs      — admission control
- manager/execute.rs   — resilience pipeline
- manager/rotation.rs  — dispatcher + on_credential_* methods
- manager/shutdown.rs  — graceful_shutdown + drain-abort

Closes Tech Spec §5.4 file-split commitment.
```

---

## Task 12: Hard-remove deprecated `OnCredentialRefresh<C>` trait

**Files:**
- Modify: `crates/credential/src/secrets/scheme_guard.rs` — DELETE the trait definition (lines ~194-300)
- Modify: `crates/credential/src/secrets/mod.rs` — remove `OnCredentialRefresh` re-export and its `#[allow(deprecated)]`
- Modify: `crates/credential/src/lib.rs` — remove `OnCredentialRefresh` from the `pub use secrets::{...}` block

**Why:** П1 marked the parallel trait `#[deprecated]`. П2's Manager dispatch lands the canonical `Resource::on_credential_refresh` path — the parallel trait is now subsumed and its retention adds confused-deputy risk per the Pass 5 security review.

- [ ] **Step 1: Verify zero in-tree implementors**

```
grep -rn "impl OnCredentialRefresh" crates/
grep -rn "OnCredentialRefresh<" crates/
```

Expected: zero `impl` sites, only the trait definition + re-exports. If any `impl` exists, escalate.

- [ ] **Step 2: Delete trait definition in `scheme_guard.rs`**

Remove the entire `#[deprecated(...)] pub trait OnCredentialRefresh<C: Credential>: Send + Sync { ... }` block (around lines 194-300) including its docs.

- [ ] **Step 3: Remove re-exports**

In `secrets/mod.rs`:
```rust
// REMOVE:
#[allow(deprecated)]
pub use scheme_guard::OnCredentialRefresh;
```

In `lib.rs`, remove `OnCredentialRefresh` from the `pub use secrets::{...}` block. Remove the `#[allow(deprecated)]` if it's no longer needed (only there because of `OnCredentialRefresh`).

- [ ] **Step 4: Update doc comments**

Search `crates/credential/src/` for any `///` mentioning `OnCredentialRefresh` and remove/update.

- [ ] **Step 5: Compile + tests**

```
cargo check --workspace
cargo nextest run --workspace --profile ci --no-tests=pass
cargo clippy --workspace -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-credential --no-deps
```

Expected: clean. The trait is gone; nothing imports it.

- [ ] **Step 6: Commit**

```
refactor(credential)!: remove deprecated OnCredentialRefresh<C> trait

П1 marked this parallel trait #[deprecated] because Resource::on_credential_refresh
subsumed it. П2 wires Manager dispatch on the new method, so the parallel
trait is fully redundant. Hard-removed per ADR-0036 schedule.

BREAKING: nebula_credential::OnCredentialRefresh no longer exists.
No in-tree consumers (verified via grep). External plugins, when they
appear, must implement Resource::on_credential_refresh directly.
```

---

## Task 13: Probe 6 analogue compile-fail probe (Resource refresh retention)

**Files:**
- Modify: `crates/resource/Cargo.toml` (add `trybuild = "1"` to `[dev-dependencies]`)
- Create: `crates/resource/tests/probes/resource_refresh_retention.rs`
- Create: `crates/resource/tests/compile_fail_resource_refresh_retention.rs`

**Why:** П1 SHOULD-fix item from multi-pass review. credential-side has Probe 6 for `SchemeGuard` retention; resource-side should have an analogue for `Resource::on_credential_refresh`. Without it, future trait reshape regressions could weaken the lifetime-pin.

- [ ] **Step 1: Add `trybuild` dev-dependency**

```toml
[dev-dependencies]
trybuild = "1"
```

- [ ] **Step 2: Create `crates/resource/tests/probes/resource_refresh_retention.rs`**

```rust
//! Compile-fail probe: a Resource impl cannot retain SchemeGuard past
//! the on_credential_refresh call.
//!
//! Mirrors the credential-side Probe 6 (`scheme_guard_retention.rs`)
//! at the Resource trait layer.

use nebula_credential::{ApiKeyCredential, CredentialContext, SchemeGuard};
use nebula_resource::{Credential, Resource};
use nebula_core::ResourceKey;

struct LeakyResource {
    stash: Option<SchemeGuard<'static, ApiKeyCredential>>,
}

#[derive(Debug)]
struct LeakyError;
impl std::fmt::Display for LeakyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "leaky")
    }
}
impl std::error::Error for LeakyError {}
impl From<LeakyError> for nebula_resource::Error {
    fn from(_: LeakyError) -> Self {
        nebula_resource::Error::permanent("leaky")
    }
}

impl Resource for LeakyResource {
    type Config = ();
    type Runtime = ();
    type Lease = ();
    type Error = LeakyError;
    type Credential = ApiKeyCredential;

    fn key() -> ResourceKey { nebula_core::resource_key!("leaky") }

    fn create(
        &self,
        _config: &Self::Config,
        _scheme: &<Self::Credential as Credential>::Scheme,
        _ctx: &nebula_resource::ResourceContext,
    ) -> impl std::future::Future<Output = Result<(), LeakyError>> + Send {
        async { Ok(()) }
    }

    fn on_credential_refresh<'a>(
        &mut self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
        _ctx: &'a CredentialContext,
    ) -> impl std::future::Future<Output = Result<(), LeakyError>> + Send + 'a {
        async move {
            // Should fail: 'a cannot widen to 'static
            self.stash = Some(new_scheme);
            Ok(())
        }
    }
}

fn main() {}
```

- [ ] **Step 3: Create `crates/resource/tests/compile_fail_resource_refresh_retention.rs`**

```rust
#[test]
fn compile_fail_resource_refresh_retention() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/probes/resource_refresh_retention.rs");
}
```

- [ ] **Step 4: Run probe**

```
cargo nextest run -p nebula-resource --test compile_fail_resource_refresh_retention --profile ci --no-tests=pass
```

Expected: passes (the probe compiles iff the leak is *prevented*, i.e., the inner test fails compilation).

- [ ] **Step 5: Commit**

```
test(resource): Probe 6 analogue for on_credential_refresh retention

Mirrors credential-side Probe 6 (scheme_guard_retention.rs). Asserts
that a Resource impl cannot stash SchemeGuard<'a, _> in a 'static
field, enforced by the borrow checker via the shared 'a lifetime
on (SchemeGuard<'a, _>, &'a CredentialContext).

Adds trybuild as a dev-dependency.
```

---

## Task 14: Integration tests for rotation dispatch

**Files:**
- Create: `crates/resource/tests/rotation.rs`

**Why:** Currently zero tests exercise the rotation path (Manager dispatchers were `todo!()`). П2 lands the dispatchers; tests must lock down the contract.

- [ ] **Step 1: Build test fixtures**

Create a mock `Refreshable` credential with a typed scheme:

```rust
// At top of crates/resource/tests/rotation.rs

use std::sync::{Arc, Mutex};
use std::time::Duration;
use nebula_credential::{
    Credential, CredentialContext, CredentialId, CredentialMetadata, CredentialState,
    AuthScheme, AuthPattern, PublicScheme, ResolveResult,
};
use nebula_resource::{
    Resource, Manager, ManagerConfig, RegisterOptions, ResourceContext,
    ResourceEvent, RefreshOutcome, RevokeOutcome, RotationOutcome,
};
use nebula_schema::FieldValues;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
struct TestScheme { token: String }

impl AuthScheme for TestScheme {
    fn pattern() -> AuthPattern { AuthPattern::SecretToken }
}

impl PublicScheme for TestScheme {} // Test-only; real schemes might be SensitiveScheme.

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, zeroize::ZeroizeOnDrop)]
struct TestState { token: String }

impl zeroize::Zeroize for TestState {
    fn zeroize(&mut self) { self.token.zeroize(); }
}

impl CredentialState for TestState {
    const KIND: &'static str = "test_credential";
    const VERSION: u32 = 1;
}

#[derive(Clone, Copy, Debug, Default)]
struct TestCredential;

impl Credential for TestCredential {
    type Input = ();
    type Scheme = TestScheme;
    type State = TestState;
    const KEY: &'static str = "test_credential";

    fn metadata() -> CredentialMetadata { /* minimal — copy from NoCredential */ }
    fn project(state: &Self::State) -> Self::Scheme {
        TestScheme { token: state.token.clone() }
    }
    async fn resolve(
        _values: &FieldValues, _ctx: &CredentialContext
    ) -> Result<ResolveResult<Self::State, ()>, nebula_credential::CredentialError> {
        Ok(ResolveResult::Complete(TestState { token: "initial".into() }))
    }
}
```

Then a Resource impl that records refresh invocations:

```rust
struct TestResource {
    refresh_count: Arc<Mutex<usize>>,
    last_token: Arc<Mutex<Option<String>>>,
    refresh_delay: Duration,
    refresh_should_fail: bool,
}

impl Resource for TestResource {
    type Config = ();
    type Runtime = ();
    type Lease = ();
    type Error = Box<dyn std::error::Error + Send + Sync>;
    type Credential = TestCredential;

    fn key() -> nebula_core::ResourceKey { /* ... */ }

    fn create(/* ... */) -> impl Future<Output = Result<(), _>> + Send { async { Ok(()) } }

    fn on_credential_refresh<'a>(
        &self,
        new_scheme: nebula_credential::SchemeGuard<'a, TestCredential>,
        _ctx: &'a CredentialContext,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a {
        let count = Arc::clone(&self.refresh_count);
        let token_slot = Arc::clone(&self.last_token);
        let delay = self.refresh_delay;
        let should_fail = self.refresh_should_fail;
        async move {
            tokio::time::sleep(delay).await;
            if should_fail {
                return Err("test failure".into());
            }
            *count.lock().unwrap() += 1;
            *token_slot.lock().unwrap() = Some(new_scheme.token.clone());
            Ok(())
        }
    }
}
```

- [ ] **Step 2: Add test cases**

Six core scenarios:

```rust
#[tokio::test]
async fn refresh_dispatches_to_single_resource() {
    // Register one resource bound to a CredentialId.
    // Call manager.on_credential_refreshed(&id, &scheme, &ctx).
    // Assert refresh_count == 1, last_token matches.
    // Assert RefreshOutcome::Ok in result vec.
    // Assert ResourceEvent::CredentialRefreshed emitted with outcome.ok=1.
}

#[tokio::test]
async fn refresh_dispatches_parallel_to_multiple_resources() {
    // Register 5 resources bound to same CredentialId, each with refresh_delay=200ms.
    // Call dispatch.
    // Assert wall-clock < 1s (would be 1s sequential, ~200ms parallel).
    // Assert all 5 refresh_count == 1.
}

#[tokio::test]
async fn refresh_per_resource_timeout_isolates_slow_one() {
    // 3 resources: A (fast 50ms), B (slow 5s), C (fast 50ms).
    // Manager default timeout 100ms.
    // Assert A and C return Ok; B returns TimedOut.
    // Assert wall-clock < 200ms (B's timeout doesn't extend siblings).
}

#[tokio::test]
async fn refresh_failure_isolates_one_resource() {
    // 3 resources, middle one refresh_should_fail.
    // Assert outer Result is Ok; inner outcomes are [Ok, Failed, Ok].
}

#[tokio::test]
async fn revoke_emits_health_changed_for_failures() {
    // Register 2 resources; one's on_credential_revoke fails.
    // Subscribe to events; call dispatch.
    // Assert HealthChanged{healthy:false} emitted for failed one only.
    // Assert aggregate CredentialRevoked event has outcome.failed=1.
}

#[tokio::test]
async fn no_credential_resource_skips_reverse_index() {
    // Register a NoCredential-bound resource (use any existing test fixture).
    // Provide a fake credential_id (should be ignored with a warning).
    // Call dispatch with that id.
    // Assert empty result vec (no resources affected).
}
```

- [ ] **Step 3: Run tests**

```
cargo nextest run -p nebula-resource --test rotation --profile ci --no-tests=pass
```

Expected: 6/6 pass.

- [ ] **Step 4: Commit**

```
test(resource): rotation dispatch integration tests (П2 coverage)

Six scenarios covering Tech Spec §3.2-§3.5 invariants:
- single-resource dispatch
- parallel dispatch (multiple resources)
- per-resource timeout isolation (security amendment B-1)
- per-resource failure isolation
- revoke HealthChanged emission (security amendment B-2)
- NoCredential opt-out skip

Test fixtures: TestCredential (Refreshable mock), TestResource
(records refresh count + last token).
```

---

## Task 15: Final workspace gate + commit

**Files:**
- All previously committed; this task verifies + opens PR.

- [ ] **Step 1: Run full local gate**

```bash
cargo +nightly fmt --all -- --check
cargo clippy --workspace -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc -p nebula-resource -p nebula-credential --no-deps
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo nextest run --workspace --profile ci --no-tests=pass
```

All five MUST pass.

- [ ] **Step 2: Spot-check 5 consumer crates**

```bash
for c in nebula-action nebula-sdk nebula-engine nebula-plugin nebula-sandbox; do
  cargo check -p $c
done
```

Expected: all clean. If any fails — investigate (consumer crate may have started using `Manager::on_credential_refreshed` after П1 landed).

- [ ] **Step 3: Update concerns register**

Edit `docs/tracking/nebula-resource-concerns-register.md` — mark R-002, R-003, R-004, R-005, R-023, R-060 status as `landed`.

- [ ] **Step 4: Update `docs/MATURITY.md` if appropriate**

Tech Spec §6.4 trigger: "transition `frontier` → `core` per §6.4 trigger (no 🔴 in counter, register resolved)". After П2 lands, the 🔴-1, 🔴-3, 🔴-4 are closed. Soak window (1-2 weeks per Strategy §6.3) before MATURITY transition. **Do NOT change MATURITY in П2 PR** — flag as follow-up after soak.

- [ ] **Step 5: Final commit (if any docs changes)**

```
docs(resource): close R-002/R-003/R-004/R-005/R-023/R-060 in concerns register

П2 landed; rotation dispatch + drain-abort + warmup security fix all in.
MATURITY transition deferred to post-soak follow-up per Strategy §6.3.
```

- [ ] **Step 6: Push + open PR**

```
git push -u origin claude/resource-p2-rotation-l2
gh pr create --title "feat(resource)!: П2 — rotation L2 dispatch (ADR-0036)" --body "..."
```

PR description should mirror П1 structure: Summary, what changed (15 components), what's NOT included (П3-П5), test plan, multi-pass review summary (run after this gate), commits list, references.

---

## Self-review checklist

After all 15 tasks complete:

**1. Spec coverage:**
- [ ] ADR-0036 §Decision dispatcher signature → Tasks 4, 5
- [ ] Tech Spec §3.1 reverse-index write → Task 3
- [ ] Tech Spec §3.2 parallel join_all → Task 4, 5
- [ ] Tech Spec §3.3-§3.4 timeout + concurrency → Task 6
- [ ] Tech Spec §3.5 failure semantics → Tasks 1, 4, 5
- [ ] Tech Spec §5.1 blue-green pattern → documented in trait rustdoc; resource impls follow
- [ ] Tech Spec §5.2 warmup_pool security B-3 → Task 10
- [ ] Tech Spec §5.4 file-split → Task 11
- [ ] Tech Spec §5.5 drain-abort fix → Task 9
- [ ] Tech Spec §6.2 metrics → Task 8
- [ ] Tech Spec §6.3 events → Task 7
- [ ] Concerns register R-002/R-003/R-004/R-005/R-023/R-060 → all closed

**2. Placeholder scan:**
- [ ] No `todo!()` remains in `Manager::on_credential_refreshed` / `_revoked`
- [ ] No `TBD` / `FIXME` introduced
- [ ] All test fixtures concrete (no `unimplemented!()`)

**3. Type consistency:**
- [ ] `RefreshOutcome` / `RevokeOutcome` / `RotationOutcome` imports consistent across files
- [ ] `Manager::on_credential_refreshed` / `_revoked` return types match Tech Spec §3.2 specification
- [ ] `ResourceEvent` variant fields match Tech Spec §6.3

**4. Behavior preservation (where П2 doesn't change behavior):**
- [ ] All 220+ existing nebula-resource tests still pass
- [ ] All workspace tests still pass (3626+)
- [ ] 5 consumer crates compile clean

---

## Execution Handoff

**Plan saved to** `docs/superpowers/plans/2026-04-27-nebula-resource-p2-rotation-l2.md`.

Two execution options:

**1. Subagent-Driven (recommended)** — dispatch fresh subagent per task. 15 tasks → ~45 subagent invocations (implementer + spec review + code review per task). Complex tasks (4, 5, 11, 14) benefit most from fresh-context execution.

**2. Inline Execution** — run tasks in this session via `superpowers:executing-plans`. Faster handoff but heavier context usage.

**Critical-path callouts before starting:**

- **Task 2 + Task 4 design question** — `SchemeGuard::from_borrow` vs alternatives. Implementer MUST escalate as NEEDS_CONTEXT before implementing the dispatcher. This may require a credential-side commit not originally scoped here.
- **Task 11 ordering** — file-split goes LAST per the plan's "(B) split last" decision. Don't run Task 11 before Tasks 3-10.
- **Task 14 fixtures** — building a `Refreshable` test credential with `TestScheme: PublicScheme` is non-trivial. The fixtures sketch is a starting point, not a complete impl.

Pick option when ready to start.
