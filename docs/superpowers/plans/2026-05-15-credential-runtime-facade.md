# Credential Runtime — Facade / Observability / StateSource (Plan 2 of 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use checkbox (`- [ ]`). Depends on Plan 1 (`docs/superpowers/plans/2026-05-15-credential-runtime-foundation.md`) being committed — its commits `801371a9..dc133084` provide `nebula-credential-runtime` (scaffold + `CredentialServiceError`) and `nebula-credential-builtin::register_builtins`.

**Goal:** Build the crate-private secure `CredentialService<B, PS>` facade — typestate builder, layered store composition, `CredentialDispatch`, non-optional `CredentialObserver`, `StateSource` — so the management bounded context exists end-to-end (still headless; API wiring is Plan 3).

**Architecture:** B-lite (spec `2026-05-15-credential-runtime-subsystem-design.md` §3–§9, refined §5/§5a). `CredentialStore`/`PendingStateStore` are RPITIT non-object-safe → facade is generic `CredentialService<B: CredentialStore, PS: PendingStateStore>` (params on struct only). `build()` constructs the layered store `ScopeLayer(AuditLayer(CacheLayer(EncryptionLayer(B))))`, the `CredentialResolver`, and spawns `LeaseLifecycle` internally — never injected. Dispatch mirrors engine's `StateProjectionRegistry` verbatim pattern.

**Tech Stack:** Rust 1.95/edition 2024; `nebula-credential`/`-storage`/`-engine`/`-eventbus`/`-resilience`/`-error`; `tokio_util::sync::CancellationToken`; `arc-swap`; `tracing`; per-crate verify (worktree env: `cargo fmt --all` is broken with os error 206 — see `reference_cargo_fmt_all_winpath` memory; use `cargo fmt -p` via PowerShell).

**Conventions (every commit):** git identity env (`GIT_AUTHOR_NAME=vanyastaff` etc.); Conventional Commits (`convco`); scope `credential-runtime`; end with `Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>`; stage root `Cargo.lock` on dep changes; no `unwrap/expect/panic` in lib code; no plan/phase IDs in code/comments; format with `cargo fmt -p nebula-credential-runtime` (PowerShell) before each commit.

---

## Verbatim reference patterns (read before coding; do not guess)

- **Dispatch exemplar (mirror exactly):** `crates/engine/src/credential/registry.rs` — `StateProjectionRegistry`:
  ```rust
  type ProjectFn = Arc<dyn Fn(&[u8]) -> Result<Box<dyn std::any::Any + Send + Sync>, StateProjectionError> + Send + Sync>;
  pub struct StateProjectionRegistry { handlers: HashMap<String, ProjectFn> }
  pub fn register<C>(&mut self) -> Result<(), StateProjectionError> where C: Credential, C::Scheme: 'static { /* dup-check; insert Arc::new(|bytes| { let s: C::State = from_slice(bytes)?; Ok(Box::new(C::project(&s))) }) */ }
  pub fn project(&self, state_kind: &str, data: &[u8]) -> Result<Box<dyn Any+Send+Sync>, StateProjectionError> { self.handlers.get(state_kind).ok_or(UnknownKind)?(data) }
  ```
- Layer ctors: `EncryptionLayer::new(inner, Arc<dyn KeyProvider>)`, `CacheLayer::new(inner, CacheConfig)`, `AuditLayer::new(inner, Arc<dyn AuditSink>)`, `ScopeLayer::new(inner, Arc<dyn ScopeResolver>)` (`nebula_storage::credential::*`).
- `CacheConfig { max_entries: u64, ttl: Duration, tti: Duration }` + `Default` (`storage/src/credential/layer/cache.rs:29`).
- `AuditSink: Send+Sync { fn record(&self, &AuditEvent) -> Result<(), StoreError> }`; `ScopeResolver: Send+Sync { fn current_owner(&self) -> Option<&str> }`; `ScopeLayer` keys on `metadata["owner_id"]`.
- `KeyProvider: Send+Sync+'static { fn current_key(&self) -> Result<Arc<EncryptionKey>, ProviderError>; fn version(&self) -> &str }`; `EnvKeyProvider::from_env()`, `StaticKeyProvider::new(Arc<EncryptionKey>)` (test-util gated).
- Resolver: `CredentialResolver::new(Arc<S>)`, `.with_event_bus(Arc<EventBus<CredentialEvent>>)`, `.with_refresh_coordinator(Arc<RefreshCoordinator>)`, `resolve::<C: Credential>(&self,&str)->Result<CredentialHandle<C::Scheme>,ResolveError>`, `resolve_with_refresh::<C: Refreshable>(&self,&str,&CredentialContext)->…`.
- Executor: `execute_resolve::<C: Credential, S: PendingStateStore>(&FieldValues,&CredentialContext,&S)->Result<ResolveResponse<C::State>,ExecutorError>`; `execute_continue::<C: Interactive, S>(&PendingToken,&UserInput,&CredentialContext,&S)->…`; `ResolveResponse<S>::{Complete(S),Pending{token,interaction},Retry{after,token}}`.
- Dispatchers: `dispatch_test::<C: Testable>(&C::Scheme,&CredentialContext)->Result<TestResult,CredentialError>`; `dispatch_revoke::<C: Revocable>(&mut C::State,&CredentialContext)->Result<(),CredentialError>`.
- Lease: `LeaseLifecycle::spawn(LeaseLifecycleConfig, Option<Arc<EventBus<LeaseEvent>>>, Option<Arc<dyn MetricsEmitter>>, CancellationToken)`, `.track(Arc<dyn LeasedProvider>, ProviderResolution, Option<CredentialId>)`, `.revoke_for_credential(CredentialId)->usize`.
- `CredentialRegistry::resolve_any(&str)->Option<&dyn AnyCredential>` / `resolve::<C>(&str)->Option<&C>`; `register::<C: Credential+5×Is*>(instance,&'static str)`.
- `CredentialContextBuilder::new(BaseContext, Arc<dyn CredentialAccessor>, Arc<dyn ResourceAccessor>).owner_id(String).session_id(String).build()`; `CredentialContext::for_test(owner)` (tests).
- `CredentialSnapshot::new::<S: AuthScheme+Clone>(kind, CredentialRecord, scheme)`; `.kind()`, `.scheme_pattern()`, `.record()`; `Debug` redacts; secret-free.
- `CredentialHandle<S: AuthScheme>::{new(S,id), snapshot()->Arc<S>, credential_id()->&str}` — secret-free.
- `EventBus::<E: Clone+Send>::new(buffer: usize)`, `.emit(E)->PublishOutcome`; `CredentialEvent::{Refreshed{credential_id},Revoked{credential_id},ReauthRequired{credential_id,reason}}`; `LeaseEvent` (5 variants).
- `MetricsEmitter: Send+Sync { fn counter(&self,&str,u64,&[(&str,&str)]); fn gauge(...); fn histogram(...) }` (`nebula_core::accessor`). `CredentialMetrics::{RESOLVE_TOTAL,REFRESH_TOTAL,REFRESH_FAILED_TOTAL,TEST_TOTAL,…}` (`nebula_credential::metrics`).
- `nebula_resilience::retry_with(RetryConfig<E>, F) -> Result<T, CallError<E>>` where `E: nebula_error::Classify`; `RetryConfig::new(u32)?.backoff(BackoffConfig::Exponential{base,multiplier,max}).jitter(JitterConfig…)`; usage exemplar `crates/credential/src/rotation/events.rs:340`.
- `StoredCredential { id, credential_key, data: Vec<u8>, state_kind, state_version, version, created_at, updated_at, expires_at, reauth_required, metadata: serde_json::Map }`; `PutMode::{CreateOnly,Overwrite,CompareAndSwap{expected_version}}`; `StoreError::{NotFound,VersionConflict,AlreadyExists,AuditFailure,Backend}`.
- `nebula_core::scope::{Scope, ScopeLevel::{Organization(OrgId),Workspace(WorkspaceId),…}}`.
- `properties_pipeline.rs:37-138` — validation pipeline: `let schema = C::properties_schema(); let values = FieldValues::from_json(raw)?; schema.validate(&values)?; let typed: C::Properties = serde_json::from_value(raw)?;` — `{"$expr":…}` fails the `from_value` step (canon §12.5).

---

## File Structure

| Path | Responsibility | Action |
|------|----------------|--------|
| `crates/credential-runtime/Cargo.toml` | add deps | Modify |
| `crates/credential-runtime/src/lib.rs` | module wiring + re-exports | Modify |
| `crates/credential-runtime/src/scope.rs` | `TenantScope` + `owner_id` derivation + per-call `ScopeResolver` | Create |
| `crates/credential-runtime/src/observer.rs` | object-safe `CredentialObserver` + `NoopObserver` + `EventMetricObserver` (default impl) | Create |
| `crates/credential-runtime/src/dispatch.rs` | `CredentialDispatch` (mirrors `StateProjectionRegistry`) + `register_dispatch*` | Create |
| `crates/credential-runtime/src/state_source.rs` | `StateSource` enum + external resolution | Create |
| `crates/credential-runtime/src/builder.rs` | typestate `CredentialServiceBuilder` | Create |
| `crates/credential-runtime/src/service.rs` | `CredentialService<B, PS>` + 12 operations | Create |
| `crates/credential-runtime/src/lib.rs` | re-export public surface | Modify |
| `crates/credential-runtime/tests/adversarial.rs` | 8 abuse-case integration tests | Create |
| `crates/credential-runtime/tests/compile_fail/raw_store_without_layers.rs` | compile-fail probe (abuse #7) | Create |
| `crates/credential-runtime/tests/compile_fail.rs` | trybuild harness | Create |

---

## Task 1: deps + `TenantScope` + scope resolver

**Files:** Modify `crates/credential-runtime/Cargo.toml`; Create `crates/credential-runtime/src/scope.rs`; Modify `src/lib.rs`.

- [ ] **Step 1: Add dependencies** to `crates/credential-runtime/Cargo.toml` `[dependencies]` (after `thiserror`):

```toml
nebula-credential = { path = "../credential" }
nebula-credential-builtin = { path = "../credential-builtin" }
nebula-storage = { path = "../storage", features = ["credential-in-memory", "test-util"] }
nebula-engine = { path = "../engine" }
nebula-core = { path = "../core" }
nebula-schema = { path = "../schema" }
nebula-eventbus = { path = "../eventbus" }
nebula-resilience = { path = "../resilience" }
tokio = { workspace = true, features = ["rt", "sync", "macros", "time"] }
tokio-util = { workspace = true }
tracing = { workspace = true }
serde_json = { workspace = true }
arc-swap = { workspace = true }
chrono = { workspace = true }
```

Add to `[dev-dependencies]`:

```toml
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "time"] }
trybuild = "1.0"
```

- [ ] **Step 2: deny.toml — widen wrappers.** In `deny.toml`: add `"nebula-credential-runtime"` to the `wrappers = [...]` arrays of the `nebula-engine`, `nebula-storage`, and `nebula-credential-builtin` ban entries. Then change the `nebula-credential-runtime` entry's wrappers from `["nebula-credential-runtime"]` to `["nebula-credential-runtime", "nebula-api", "nebula-cli"]`.

- [ ] **Step 3: Write failing test** — create `crates/credential-runtime/src/scope.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::TenantScope;

    #[test]
    fn owner_id_is_org_slash_workspace() {
        let s = TenantScope::new("org-1", "ws-2");
        assert_eq!(s.owner_id(), "org-1/ws-2");
    }

    #[test]
    fn scope_resolver_returns_owner() {
        use nebula_storage::credential::ScopeResolver;
        let s = TenantScope::new("o", "w");
        let r = s.resolver();
        assert_eq!(r.current_owner(), Some("o/w"));
    }
}
```

- [ ] **Step 4: Run — verify fails:** `cargo test -p nebula-credential-runtime --lib scope` → FAIL (`TenantScope` undefined).

- [ ] **Step 5: Implement** — prepend to `scope.rs`:

```rust
//! Tenant scoping: `TenantScope` is a mandatory operation argument; it
//! derives the `owner_id` string the storage `ScopeLayer` keys on, and
//! supplies a per-call `ScopeResolver`. Confused-deputy (spec §6 #1) is
//! closed by type: no operation is callable without a `&TenantScope`.

use nebula_storage::credential::ScopeResolver;

/// Tenant identity for a credential operation. `owner_id` =
/// `"{org}/{workspace}"` — the value persisted in `StoredCredential.
/// metadata["owner_id"]` and matched by `ScopeLayer`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantScope {
    owner_id: String,
}

impl TenantScope {
    /// Construct from organization + workspace identifiers.
    #[must_use]
    pub fn new(org: impl AsRef<str>, workspace: impl AsRef<str>) -> Self {
        Self {
            owner_id: format!("{}/{}", org.as_ref(), workspace.as_ref()),
        }
    }

    /// The scope key persisted/matched by the storage `ScopeLayer`.
    #[must_use]
    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    /// A `ScopeResolver` pinned to this scope, for the per-call layered
    /// store stack.
    #[must_use]
    pub fn resolver(&self) -> FixedScopeResolver {
        FixedScopeResolver {
            owner: self.owner_id.clone(),
        }
    }
}

/// `ScopeResolver` that always reports one fixed owner — constructed
/// per operation from the caller's `TenantScope`.
#[derive(Debug)]
pub struct FixedScopeResolver {
    owner: String,
}

impl ScopeResolver for FixedScopeResolver {
    fn current_owner(&self) -> Option<&str> {
        Some(&self.owner)
    }
}
```

- [ ] **Step 6: Wire `lib.rs`** — add after `pub mod error;`:

```rust
pub mod scope;

pub use scope::{FixedScopeResolver, TenantScope};
```

- [ ] **Step 7: Format + verify:** `cargo fmt -p nebula-credential-runtime` (PowerShell); `cargo test -p nebula-credential-runtime --lib scope` → PASS (2 tests); `cargo clippy -p nebula-credential-runtime -- -D warnings` → clean.

- [ ] **Step 8: Commit:**

```
git add crates/credential-runtime/Cargo.toml Cargo.lock deny.toml crates/credential-runtime/src/scope.rs crates/credential-runtime/src/lib.rs
git commit -m "feat(credential-runtime): deps + TenantScope/ScopeResolver

Wires Exec deps, widens deny.toml wrappers (engine/storage/builtin +
api/cli consumers), adds TenantScope deriving the owner_id ScopeLayer
keys on (closes confused-deputy abuse #1 by type).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: `CredentialObserver` (object-safe) + `NoopObserver` + default impl

**Files:** Create `src/observer.rs`; Modify `src/lib.rs`. Closes spec §7 (observability is DoD) structurally — non-`Option`, on the single code path.

- [ ] **Step 1: Failing test** — create `src/observer.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::{CredentialObserver, EventMetricObserver, NoopObserver};
    use nebula_credential::CredentialId;
    use std::sync::Arc;

    #[test]
    fn noop_observer_is_object_safe_and_silent() {
        let obs: Arc<dyn CredentialObserver> = Arc::new(NoopObserver);
        obs.on_revoke(&CredentialId::from("c1"));
        assert!(obs.lease_bus().is_none());
        assert!(obs.metrics().is_none());
    }

    #[tokio::test]
    async fn event_metric_observer_emits_on_event_bus() {
        let obs = EventMetricObserver::new(8);
        let mut sub = obs.event_bus().subscribe();
        obs.on_refresh(&CredentialId::from("c2"));
        let ev = sub.try_recv().expect("event emitted");
        assert!(matches!(
            ev,
            nebula_credential::CredentialEvent::Refreshed { .. }
        ));
    }
}
```

- [ ] **Step 2: Verify fails:** `cargo test -p nebula-credential-runtime --lib observer` → FAIL.

- [ ] **Step 3: Implement** (prepend). The trait is deliberately object-safe: no RPITIT, no generic methods.

```rust
//! Non-optional observability seam. Closes canon §12.5/§3.5: emission
//! sits on the single facade code path, so "never wired" is
//! unrepresentable. `CredentialObserver` is object-safe by design
//! (`Arc<dyn CredentialObserver>`).

use std::sync::Arc;

use nebula_credential::metrics::CredentialMetrics;
use nebula_credential::{CredentialEvent, CredentialId};
use nebula_credential::provider::event::LeaseEvent;
use nebula_core::accessor::MetricsEmitter;
use nebula_eventbus::EventBus;

/// Observability hooks the facade calls on every lifecycle transition.
/// Object-safe (no RPITIT / generics) so it can be `Arc<dyn …>`.
pub trait CredentialObserver: Send + Sync {
    /// Event bus the internally-built `CredentialResolver` is wired to
    /// (`.with_event_bus`). Must be non-optional — the resolver always
    /// gets a real bus.
    fn event_bus(&self) -> Arc<EventBus<CredentialEvent>>;
    /// Optional lease event bus handed to `LeaseLifecycle::spawn`.
    fn lease_bus(&self) -> Option<Arc<EventBus<LeaseEvent>>>;
    /// Optional metrics emitter handed to `LeaseLifecycle::spawn` and
    /// used by the facade for resolve/refresh/test counters.
    fn metrics(&self) -> Option<Arc<dyn MetricsEmitter>>;
    /// Called after a successful resolve.
    fn on_resolve(&self, credential_id: &CredentialId);
    /// Called after a successful refresh.
    fn on_refresh(&self, credential_id: &CredentialId);
    /// Called after a successful revoke.
    fn on_revoke(&self, credential_id: &CredentialId);
}

/// Silent observer. Must be chosen *explicitly* at the composition root
/// (tests) — it is never a default that hides missing wiring.
#[derive(Debug)]
pub struct NoopObserver;

impl CredentialObserver for NoopObserver {
    fn event_bus(&self) -> Arc<EventBus<CredentialEvent>> {
        Arc::new(EventBus::new(1))
    }
    fn lease_bus(&self) -> Option<Arc<EventBus<LeaseEvent>>> {
        None
    }
    fn metrics(&self) -> Option<Arc<dyn MetricsEmitter>> {
        None
    }
    fn on_resolve(&self, _credential_id: &CredentialId) {}
    fn on_refresh(&self, _credential_id: &CredentialId) {}
    fn on_revoke(&self, _credential_id: &CredentialId) {}
}

/// Production observer: emits `CredentialEvent` to an `EventBus`,
/// increments `CredentialMetrics` counters via the supplied emitter.
pub struct EventMetricObserver {
    events: Arc<EventBus<CredentialEvent>>,
    leases: Arc<EventBus<LeaseEvent>>,
    metrics: Option<Arc<dyn MetricsEmitter>>,
}

impl EventMetricObserver {
    /// `buffer` is the per-bus capacity.
    #[must_use]
    pub fn new(buffer: usize) -> Self {
        Self {
            events: Arc::new(EventBus::new(buffer)),
            leases: Arc::new(EventBus::new(buffer)),
            metrics: None,
        }
    }

    /// Attach a metrics emitter (counters for resolve/refresh/revoke).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_metrics(mut self, emitter: Arc<dyn MetricsEmitter>) -> Self {
        self.metrics = Some(emitter);
        self
    }

    fn count(&self, name: &str, outcome: &str) {
        if let Some(m) = &self.metrics {
            m.counter(name, 1, &[("outcome", outcome)]);
        }
    }
}

impl CredentialObserver for EventMetricObserver {
    fn event_bus(&self) -> Arc<EventBus<CredentialEvent>> {
        Arc::clone(&self.events)
    }
    fn lease_bus(&self) -> Option<Arc<EventBus<LeaseEvent>>> {
        Some(Arc::clone(&self.leases))
    }
    fn metrics(&self) -> Option<Arc<dyn MetricsEmitter>> {
        self.metrics.clone()
    }
    fn on_resolve(&self, _credential_id: &CredentialId) {
        self.count(CredentialMetrics::RESOLVE_TOTAL, "ok");
    }
    fn on_refresh(&self, credential_id: &CredentialId) {
        let _ = self.events.emit(CredentialEvent::Refreshed {
            credential_id: credential_id.clone(),
        });
        self.count(CredentialMetrics::REFRESH_TOTAL, "ok");
    }
    fn on_revoke(&self, credential_id: &CredentialId) {
        let _ = self.events.emit(CredentialEvent::Revoked {
            credential_id: credential_id.clone(),
        });
    }
}
```

> If `CredentialId::from(&str)` / `EventBus::subscribe().try_recv()` / `LeaseEvent` path differ from the recon notes, apply the compiler's resolved path (types exist; only paths may differ — same deterministic-fix rule as Plan 1's `CredentialMetadata`).

- [ ] **Step 4: Wire lib.rs:** add `pub mod observer;` + `pub use observer::{CredentialObserver, EventMetricObserver, NoopObserver};`.

- [ ] **Step 5: Format + verify:** `cargo fmt -p nebula-credential-runtime`; `cargo test -p nebula-credential-runtime --lib observer` → PASS; clippy `-D warnings` clean.

- [ ] **Step 6: Commit** `feat(credential-runtime): non-optional CredentialObserver seam` (+ Co-Authored-By trailer).

---

## Task 3: `CredentialDispatch` (mirror `StateProjectionRegistry`)

**Files:** Create `src/dispatch.rs`; Modify `src/lib.rs`. This is spec §5a — the dyn→typed crux. **Read `crates/engine/src/credential/registry.rs` first and mirror its erasure shape verbatim.**

- [ ] **Step 1: Failing test** — create `src/dispatch.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::CredentialDispatch;
    use nebula_credential_builtin::BearerTokenCredential;

    #[test]
    fn register_and_lookup_resolve_fn() {
        let mut d = CredentialDispatch::new();
        d.register::<BearerTokenCredential>().expect("register ok");
        assert!(d.contains("bearer_token"));
        assert_eq!(d.len(), 1);
        // bearer_token is static -> no refresh/test/revoke closures.
        assert!(!d.is_refreshable("bearer_token"));
        assert!(!d.is_testable("bearer_token"));
        assert!(!d.is_revocable("bearer_token"));
    }

    #[test]
    fn duplicate_key_is_rejected() {
        let mut d = CredentialDispatch::new();
        d.register::<BearerTokenCredential>().unwrap();
        let err = d.register::<BearerTokenCredential>().unwrap_err();
        assert!(matches!(
            err,
            super::DispatchError::DuplicateKey { .. }
        ));
        assert_eq!(d.len(), 1);
    }
}
```

- [ ] **Step 2: Verify fails:** `cargo test -p nebula-credential-runtime --lib dispatch` → FAIL.

- [ ] **Step 3: Implement** (prepend). Mirrors `StateProjectionRegistry`: `Arc<dyn Fn… + Send + Sync>` erased closures keyed by `Credential::KEY`; capability closures populated only via capability-bounded `register_*` methods (closure presence = capability, structural — no reflection).

```rust
//! Type-erased credential operation dispatch keyed by `Credential::KEY`.
//! Mirrors `nebula_engine::credential::StateProjectionRegistry`: boxed
//! closures monomorphize a generic `C` so a runtime string key can drive
//! `Credential::resolve` / `Refreshable::refresh` / `Testable::test` /
//! `Revocable::revoke` without reflection. Capability is encoded by
//! closure *presence* (a `*_fn: Option`), populated only by the
//! capability-bounded `register_*` methods — structurally impossible to
//! advertise a capability the type lacks.

use std::collections::HashMap;
use std::sync::Arc;

use nebula_credential::Credential;

/// Registration-time failure (fail-closed on duplicate KEY, Tech Spec §15.6).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DispatchError {
    /// Two registrations shared a `Credential::KEY`. First wins; second
    /// rejected; table unchanged.
    #[error("duplicate credential dispatch key '{key}'")]
    DuplicateKey {
        /// The colliding key.
        key: &'static str,
    },
}

/// One credential type's erased operations. `None` ⇒ the type does not
/// implement that capability sub-trait.
struct DispatchEntry {
    refreshable: bool,
    testable: bool,
    revocable: bool,
}

/// Key → erased operations. Built alongside `register_builtins`.
#[derive(Default)]
pub struct CredentialDispatch {
    entries: HashMap<&'static str, DispatchEntry>,
}

impl CredentialDispatch {
    /// Empty table.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Register a credential type's base operations. Fail-closed on
    /// duplicate KEY.
    ///
    /// # Errors
    /// [`DispatchError::DuplicateKey`] if `C::KEY` already registered.
    pub fn register<C: Credential>(&mut self) -> Result<(), DispatchError> {
        let key: &'static str = C::KEY;
        if self.entries.contains_key(key) {
            return Err(DispatchError::DuplicateKey { key });
        }
        self.entries.insert(
            key,
            DispatchEntry {
                refreshable: false,
                testable: false,
                revocable: false,
            },
        );
        tracing::info!(credential.key = key, "credential dispatch registered");
        Ok(())
    }

    /// Number of registered types.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when no types are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// True when `key` is registered.
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Whether the type at `key` has a refresh closure.
    #[must_use]
    pub fn is_refreshable(&self, key: &str) -> bool {
        self.entries.get(key).is_some_and(|e| e.refreshable)
    }

    /// Whether the type at `key` has a test closure.
    #[must_use]
    pub fn is_testable(&self, key: &str) -> bool {
        self.entries.get(key).is_some_and(|e| e.testable)
    }

    /// Whether the type at `key` has a revoke closure.
    #[must_use]
    pub fn is_revocable(&self, key: &str) -> bool {
        self.entries.get(key).is_some_and(|e| e.revocable)
    }
}
```

> **Scope note for this task:** the boxed *operation* closures (`resolve_fn`, `refresh_fn`, …) that actually invoke `execute_resolve::<C,_>` / `resolve_with_refresh::<C>` / `dispatch_test::<C>` are added in Task 5 (the service) where the store/pending generics `<B, PS>` are in scope — they cannot be fully typed until the service's generic context exists. Task 3 lands the table + capability bookkeeping + the `register*` skeleton mirroring `StateProjectionRegistry`; the closures are filled when the service composes them. This split keeps Task 3 a self-contained, testable unit (mirrors how `StateProjectionRegistry` is a standalone registry consumed by the resolver).

- [ ] **Step 4: Wire lib.rs:** `pub mod dispatch;` + `pub use dispatch::{CredentialDispatch, DispatchError};`.

- [ ] **Step 5: Format + verify:** `cargo fmt -p nebula-credential-runtime`; `cargo test -p nebula-credential-runtime --lib dispatch` → PASS (2 tests); clippy clean.

- [ ] **Step 6: Commit** `feat(credential-runtime): CredentialDispatch table (mirrors StateProjectionRegistry)`.

---

## Task 4: `StateSource`

**Files:** Create `src/state_source.rs`; Modify `src/lib.rs`. Spec §8 — replace the resolver's store-only assumption with a polymorphic source (no bridge).

- [ ] **Step 1: Failing test** — `src/state_source.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::StateSource;

    #[test]
    fn default_is_local_encrypted() {
        assert!(matches!(StateSource::default(), StateSource::LocalEncrypted));
    }

    #[test]
    fn external_carries_provider() {
        let chain = nebula_credential::provider::ExternalProviderChain::new();
        let s = StateSource::External(std::sync::Arc::new(chain));
        assert!(matches!(s, StateSource::External(_)));
    }
}
```

- [ ] **Step 2: Verify fails:** `cargo test -p nebula-credential-runtime --lib state_source` → FAIL.

- [ ] **Step 3: Implement** (prepend):

```rust
//! Polymorphic credential state source. Replaces the resolver's
//! hardcoded "state always from `CredentialStore`" (spec §8 — no
//! adapter/bridge). `External` fulfils ADR-0051's deferred Phase-D
//! non-goal: resolved secrets with a lease are tracked via
//! `LeaseLifecycle`.

use std::sync::Arc;

use nebula_credential::provider::ExternalProvider;

/// Where a credential's resolved material comes from.
pub enum StateSource {
    /// The crate-private layered encrypted store (default).
    LocalEncrypted,
    /// An external secret provider chain (Vault, etc.). A
    /// `ProviderResolution` carrying a lease is handed to
    /// `LeaseLifecycle::track`.
    External(Arc<dyn ExternalProvider>),
}

impl Default for StateSource {
    fn default() -> Self {
        Self::LocalEncrypted
    }
}

impl std::fmt::Debug for StateSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalEncrypted => f.write_str("StateSource::LocalEncrypted"),
            Self::External(p) => {
                f.debug_tuple("StateSource::External").field(&p.provider_name()).finish()
            },
        }
    }
}
```

> If `ExternalProviderChain::new()` differs, read `crates/credential/src/provider/chain.rs` for the exact constructor and adjust the test only.

- [ ] **Step 4: Wire lib.rs:** `pub mod state_source;` + `pub use state_source::StateSource;`.

- [ ] **Step 5: Format + verify + commit** `feat(credential-runtime): StateSource (store-or-external, no bridge)`.

---

## Task 5: typestate builder + `CredentialService<B, PS>` + crate-private composition + operations

**Files:** Create `src/builder.rs`, `src/service.rs`; Modify `src/lib.rs`. The largest task — split into sub-commits per checkbox group.

- [ ] **Step 1: Failing integration test** — create `crates/credential-runtime/tests/adversarial.rs` with the first abuse case (plaintext-at-rest impossible by construction + happy-path create→get round-trip) using `StaticKeyProvider` + `InMemoryStore` + `InMemoryPendingStore` + `NoopObserver`:

```rust
use nebula_credential_runtime::{CredentialService, NoopObserver, TenantScope};
use std::sync::Arc;

#[tokio::test]
async fn create_then_get_roundtrip_is_tenant_scoped_and_encrypted() {
    let svc = nebula_credential_runtime::test_support::in_memory_service();
    let scope = TenantScope::new("org1", "ws1");
    let snap = svc
        .create(&scope, "bearer_token", serde_json::json!({ "token": "sk-1" }))
        .await
        .expect("create ok");
    let got = svc.get(&scope, snap.id()).await.expect("get ok");
    assert_eq!(got.kind(), "bearer_token");
    // Cross-tenant get is denied (abuse #1).
    let other = TenantScope::new("org1", "ws2");
    assert!(svc.get(&other, snap.id()).await.is_err());
}
```

- [ ] **Step 2: Verify fails** (`test_support`/`CredentialService` undefined).

- [ ] **Step 3: Implement the typestate builder** in `src/builder.rs` — phantom-typestate so `.build()` exists only when all mandatory setters were called. Pattern:

```rust
//! Typestate builder: a missing mandatory collaborator is a compile
//! error (no runtime panic). Each mandatory setter flips a phantom
//! `Missing → Set`; `build()` is implemented only for the all-`Set`
//! state. `build()` performs the crate-private secure composition.

use std::marker::PhantomData;
use std::sync::Arc;

use nebula_credential::CredentialRegistry;
use nebula_credential::pending_store::PendingStateStore;
use nebula_credential::store::CredentialStore;
use nebula_storage::credential::{
    AuditSink, CacheConfig, CacheLayer, AuditLayer, EncryptionLayer, KeyProvider, ScopeLayer,
};
use tokio_util::sync::CancellationToken;

use crate::dispatch::CredentialDispatch;
use crate::observer::CredentialObserver;
use crate::service::CredentialService;
use crate::state_source::StateSource;

/// Marker: setter not yet called.
pub struct Missing;
/// Marker: setter called.
pub struct Set;

#[doc(hidden)]
pub struct CredentialServiceBuilder<B, PS, SKey, SKp, SAudit, SScope, SCache, SPend, SReg, SDisp, SObs, SLease, SShut> {
    raw_store: Option<B>,
    key_provider: Option<Arc<dyn KeyProvider>>,
    audit_sink: Option<Arc<dyn AuditSink>>,
    scope_resolver: Option<Arc<dyn nebula_storage::credential::ScopeResolver>>,
    cache_config: Option<CacheConfig>,
    pending_store: Option<PS>,
    registry: Option<Arc<CredentialRegistry>>,
    dispatch: Option<Arc<CredentialDispatch>>,
    observer: Option<Arc<dyn CredentialObserver>>,
    lease_config: Option<nebula_engine::credential::LeaseLifecycleConfig>,
    shutdown: Option<CancellationToken>,
    refresh_coordinator: Option<Arc<nebula_engine::credential::RefreshCoordinator>>,
    external: StateSource,
    _pd: PhantomData<(B, PS, SKey, SKp, SAudit, SScope, SCache, SPend, SReg, SDisp, SObs, SLease, SShut)>,
}
```

Provide: `CredentialService::<B, PS>::builder()` returning the all-`Missing` builder; one setter per mandatory field flipping its phantom to `Set` (each consumes `self`, returns the type with that param `= Set`); `refresh_coordinator(..)`/`external_providers(..)` available in any state (optional). Implement `build(self) -> CredentialService<B, PS>` ONLY for the fully-`Set` state. `build()` body:

```rust
let store = ScopeLayer::new(
    AuditLayer::new(
        CacheLayer::new(
            EncryptionLayer::new(self.raw_store.unwrap(), self.key_provider.unwrap()),
            self.cache_config.unwrap(),
        ),
        self.audit_sink.unwrap(),
    ),
    self.scope_resolver.unwrap(),
);
let store = Arc::new(store);
let resolver = nebula_engine::credential::CredentialResolver::new(Arc::clone(&store))
    .with_refresh_coordinator(
        self.refresh_coordinator
            .unwrap_or_else(|| Arc::new(nebula_engine::credential::RefreshCoordinator::new())),
    )
    .with_event_bus(self.observer.as_ref().unwrap().event_bus());
let observer = self.observer.unwrap();
let lease = nebula_engine::credential::LeaseLifecycle::spawn(
    self.lease_config.unwrap(),
    observer.lease_bus(),
    observer.metrics(),
    self.shutdown.unwrap(),
);
CredentialService::__from_parts(store, resolver, lease, self.pending_store.unwrap(),
    self.registry.unwrap(), self.dispatch.unwrap(), observer, self.external)
```

> `.unwrap()` here is in a builder whose typestate *guarantees* `Some` — but lib code forbids `unwrap`. Use `expect("typestate guarantees set")` is also forbidden. Instead the setters store into a parallel all-required struct via the typestate transition so `build()` receives a `BuiltParts { … }` with non-`Option` fields (the typestate carries the values, not `Option`s). Implement that way: each `Set`-transition setter moves the value into a growing tuple/struct; `build()` destructures non-optionally. (This is the standard zero-unwrap typestate; see the Rust 1.95 typestate idiom — values travel in the type state, not behind `Option`.)

- [ ] **Step 4: Implement `CredentialService<B, PS>`** in `src/service.rs`: struct holding `store: Arc<ScopeLayer<AuditLayer<CacheLayer<EncryptionLayer<B>>>>>`, `resolver`, `lease`, `pending: PS`, `registry`, `dispatch`, `observer`, `source: StateSource`. Add `pub(crate) fn __from_parts(...)`. Implement operations; each takes `&self, scope: &TenantScope, …`. For each op:
  - `create`: validate via `registry` (`C::properties_schema().validate(&FieldValues::from_json(raw)?)` then `serde_json::from_value::<C::Properties>` — reject `$expr` by the from_value failure, spec §6 #2; dispatch the typed resolve via the `CredentialDispatch` closure populated here), build `StoredCredential` with `metadata["owner_id"] = scope.owner_id()`, `store.put(_, PutMode::CreateOnly)`. Return secret-free `CredentialSnapshot`.
  - `get`/`list`/`delete`: `store.get/list/delete`; `get` builds `CredentialSnapshot` from the projected handle (`resolver.resolve::<C>` via dispatch). Cross-tenant denied because `ScopeLayer` (built with `scope.resolver()`) filters by `owner_id` — construct the layered store **per call** bound to `scope.resolver()`, OR pass scope through `CredentialContext::owner_id`. (Implement per-call scoping: the stored stack's `ScopeLayer` resolver returns `scope.owner_id()`.)
  - `update`: `PutMode::CompareAndSwap { expected_version }` → map `StoreError::VersionConflict` to `CredentialServiceError::VersionConflict`.
  - `test`/`refresh`/`revoke`: consult `dispatch.is_testable/is_refreshable/is_revocable(key)`; if false → `CredentialServiceError::CapabilityUnsupported`; else invoke the capability closure (wrapping `refresh` in `nebula_resilience::retry_with` with `RetryConfig::new(3)?.backoff(BackoffConfig::Exponential{ base: 200ms, multiplier: 2.0, max: 5s })`), then `observer.on_refresh/on_revoke`.
  - `resolve`/`continue_resolve`: `execute_resolve`/`execute_continue` via dispatch + `pending` store; map `ResolveResponse` → `Acquisition`.
  - `list_types`/`get_type`: from `registry` (`CredentialMetadata`).
  Map every `StoreError`/`ResolveError`/`ExecutorError`/`ProviderError` into `CredentialServiceError` (no stringly leakage of secrets).

- [ ] **Step 5: `test_support` module** (feature/`cfg(test)` exposed) — `in_memory_service()` composing `StaticKeyProvider` + `InMemoryStore` + `InMemoryPendingStore` + `NoopObserver` + `register_builtins`-populated registry/dispatch, so tests/Plan-3 get a one-call constructor.

- [ ] **Step 6: Wire lib.rs** (`pub mod builder; pub mod service;` + re-exports `CredentialService`, builder markers, `Acquisition`).

- [ ] **Step 7: Format + verify** `cargo fmt -p`; `cargo test -p nebula-credential-runtime` (lib + the adversarial test) → PASS; clippy `-D warnings` clean; **zero `unwrap/expect/panic` in non-test code** (`grep -n "unwrap()\|expect(\|panic!" src/` — only test mods may match).

- [ ] **Step 8: Commit** in logical sub-commits: `feat(credential-runtime): typestate builder + secure layered composition`, then `feat(credential-runtime): CredentialService operations + dispatch closures`, then `feat(credential-runtime): in-memory test_support constructor`.

---

## Task 6: 8 abuse-case adversarial tests + compile-fail probe

**Files:** Extend `tests/adversarial.rs`; Create `tests/compile_fail.rs` + `tests/compile_fail/raw_store_without_layers.rs`.

- [ ] **Step 1:** Add one `#[tokio::test]` per remaining spec §6 abuse case (write each test, run, see it pass — they assert the invariants already built in Task 5):
  - #2 `$expr` in properties → `create` returns `CredentialServiceError::ValidationFailed` (json `{"token": {"$expr":"{{x}}"}}`).
  - #3 response carries no secret: serialize the returned `CredentialSnapshot`/response DTO to JSON; assert it does not contain the secret substring.
  - #4 capability-gated: `refresh`/`test`/`revoke` on `bearer_token` (static) → `CapabilityUnsupported`.
  - #5 cross-tenant `revoke`/`get` denied (extends Task 5 Step 1).
  - #6 pending hijack: `continue_resolve` with a wrong-`owner_id`/`session_id` token → error (drives `PendingStateStore::consume` 4D binding).
  - #8 audit fail-closed: inject an `AuditSink` whose `record` returns `Err` → `create` fails with a `Store`/audit error, and a follow-up `get` shows the row absent (not log-and-continue).
- [ ] **Step 2:** `tests/compile_fail/raw_store_without_layers.rs` — attempt to construct `CredentialService` from a raw `InMemoryStore` bypassing the builder (e.g., call `__from_parts` or instantiate the struct directly). Expected: compile error (`__from_parts` is `pub(crate)`; struct fields private). `tests/compile_fail.rs`: `#[test] fn t(){ trybuild::TestCases::new().compile_fail("tests/compile_fail/raw_store_without_layers.rs"); }`.
- [ ] **Step 3:** Run `cargo test -p nebula-credential-runtime` (all) + `cargo test -p nebula-credential-runtime --test compile_fail` → all green.
- [ ] **Step 4: Commit** `test(credential-runtime): 8 abuse-case invariants + compile-fail probe`.

---

## Task 7: pre-PR gate (per-crate; `cargo fmt --all` is env-broken here)

- [ ] **Step 1:** `cargo fmt -p nebula-credential-runtime` (PowerShell) — format clean.
- [ ] **Step 2:** Per-crate gate (workspace `task dev:check` cannot run `cargo fmt --all` in this worktree — os error 206; CI runs it):
  - `cargo fmt -p nebula-credential-runtime -- --check` → exit 0
  - `cargo clippy -p nebula-credential-runtime --all-targets -- -D warnings` → clean
  - `cargo nextest run -p nebula-credential-runtime` → all pass
  - `cargo nextest run -p nebula-credential -p nebula-credential-builtin -p nebula-engine -p nebula-storage` → unaffected, green (facade adds a consumer; no upstream change except deny.toml + the additive blanket-free generic facade)
  - `cargo deny check bans` → ok (runtime→engine/storage/builtin edges now allowlisted; still acyclic — engine does NOT depend on runtime)
- [ ] **Step 3:** Triage any failure by stage (most likely: a recon'd path differs → apply rustc's resolved path; a typestate `unwrap` slipped in → convert to value-in-typestate). Re-run until green.
- [ ] **Step 4: Commit** any triage fixes `chore(credential-runtime): green per-crate gate for facade`.

---

## Self-Review

**1. Spec coverage:** §5 typestate builder → Task 5; §5a dispatch (mirror StateProjectionRegistry) → Task 3 (+ closures Task 5); §6 8 abuse invariants → Task 5 (built-in) + Task 6 (asserted) + compile-fail probe (#7); §7 observability non-optional → Task 2; §8 StateSource → Task 4; §4 deny.toml widening → Task 1 Step 2. Tenant scoping (§6 #1) → Task 1. Not in Plan 2 (Plan 3): API services/AppState/OpenAPI, e2e wiremock-Vault, ADR-0028 final audit sign-off.

**2. Placeholder scan:** Operation bodies in Task 5 Step 4 are specified by behavior + exact APIs (every external signature is in the verbatim reference block) rather than full literal code, because they are generic over `<B, PS>` and the dispatch closures — writing 400 lines of literal generic glue here would be less reliable than the precise per-op contract + verbatim call signatures. This is the one calibrated deviation from "full literal code"; every type/method it references is defined in the reference block or Plan 1. The novel standalone units (scope, observer, dispatch table, state_source, typestate markers) have complete literal code.

**3. Type consistency:** `CredentialService<B, PS>`, `CredentialObserver`/`NoopObserver`/`EventMetricObserver`, `CredentialDispatch`/`DispatchError`, `StateSource`, `TenantScope`/`FixedScopeResolver` names are consistent across tasks, lib.rs re-exports, and the adversarial tests. `__from_parts` is the single crate-private constructor (compile-fail probe target).

**Plan 3 (API wiring + e2e + ADR-0028 audit) is written after Plan 2 merges**, against the concrete merged `CredentialService` surface.
