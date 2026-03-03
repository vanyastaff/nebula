# Credential + Resource Integration Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Connect `nebula-credential` and `nebula-resource` with a type-safe, rotation-aware integration layer — replacing the broken `TypeId`-based `CredentialRef` and the string-based `credentials.rs` pull model with a typed push model (`CredentialRef<C>`, `ResourceRef<R>`, `HasResourceComponents`, `RotationStrategy`).

**Architecture:** Resources declare their dependencies (credential + sub-resources) via `HasResourceComponents`. The manager resolves them, injects credential state via `CredentialResource::authorize()` after creation, and re-authorizes on rotation using the declared `RotationStrategy` (HotSwap / DrainAndRecreate / Reconnect). The pool stores credential state as `serde_json::Value` and applies it through a type-erased closure captured at registration.

**Tech Stack:** Rust 2024 (MSRV 1.93), tokio, serde_json, nebula-core (`CredentialKey`, `ResourceKey`), thiserror.

---

## Context: What exists, what changes, what goes away

### What EXISTS and must be preserved
- `DependencyGraph` — topological sort, cycle detection (no changes)
- `Pool<R>` — acquire/release/health/quarantine core (extended, not replaced)
- `ResourceEvent` / `EventBus` — extended with one new variant
- `CredentialManager` — extended with rotation subscription
- All docs (PROTOCOLS.md, DECISIONS.md, CONSTITUTION.md, ARCHITECTURE.md) — already updated

### What CHANGES
| File | Change |
|------|--------|
| `crates/credential/src/core/reference.rs` | Replace `CredentialRef(TypeId)` with `CredentialRef<C: CredentialType>` |
| `crates/credential/src/traits/credential.rs` | Add `RotationStrategy` + `rotation_strategy()` to `CredentialResource` |
| `crates/credential/src/manager/mod.rs` | Add `CredentialRotationEvent`, `rotation_subscriber()` |
| `crates/credential/src/lib.rs` | Update re-exports |
| `crates/resource/src/pool.rs` | Add credential state storage + `handle_rotation()` |
| `crates/resource/src/events.rs` | Add `CredentialRotated` variant |
| `crates/resource/src/manager.rs` | Add `register_with_components()`, rotation subscription loop |
| `crates/resource/src/lib.rs` | Update re-exports |
| (future) `crates/action/src/components.rs` | `ActionComponents`, `HasActionComponents` (mirrors resource pattern) |
| (future) `crates/credential/src/components.rs` | `CredentialComponents`, `HasCredentialComponents` (if needed) |

### What is DELETED
| File | Reason |
|------|--------|
| `crates/resource/src/credentials.rs` | Replaced by typed push model |
| `ctx.credentials: Option<Arc<dyn CredentialProvider>>` | Removed from `Context` |

### What is CREATED
| File | Purpose |
|------|---------|
| `crates/resource/src/resource_ref.rs` | `ResourceRef<R: Resource>` typed reference |
| `crates/resource/src/components.rs` | `ResourceComponents`, `ErasedCredentialRef`, `ErasedResourceRef`, `HasResourceComponents` |

---

## Phase 1 — nebula-credential: Fix CredentialRef, add RotationStrategy

### Task 1: Replace `CredentialRef(TypeId)` with `CredentialRef<C: CredentialType>`

**Why:** Current `CredentialRef(TypeId)` is not stable across compilations, not serializable, cannot distinguish multiple instances of same type ("github-prod" vs "github-staging"), and is not linked to `CredentialType`. Violates D-015.

**Files:**
- Modify: `crates/credential/src/core/reference.rs`
- Modify: `crates/credential/src/lib.rs`

**Step 1: Write the failing test**

Add to `crates/credential/src/core/reference.rs` `#[cfg(test)]` block:

```rust
#[cfg(test)]
mod new_tests {
    use super::*;

    struct GithubOAuth2;
    impl CredentialType for GithubOAuth2 {
        fn credential_key() -> nebula_core::CredentialKey {
            nebula_core::CredentialKey::new("oauth2_github").unwrap()
        }
        type State = ();
    }

    #[test]
    fn credential_ref_captures_id_and_key() {
        let r = CredentialRef::<GithubOAuth2>::new("github-prod").unwrap();
        assert_eq!(r.id.as_str(), "github-prod");
        assert_eq!(r.credential_key().as_str(), "oauth2_github");
    }

    #[test]
    fn two_instances_same_type_are_different() {
        let prod = CredentialRef::<GithubOAuth2>::new("github-prod").unwrap();
        let staging = CredentialRef::<GithubOAuth2>::new("github-staging").unwrap();
        assert_ne!(prod.id, staging.id);
        assert_eq!(prod.credential_key(), staging.credential_key()); // same type
    }

    #[test]
    fn erase_preserves_id_and_key() {
        let r = CredentialRef::<GithubOAuth2>::new("github-prod").unwrap();
        let erased: ErasedCredentialRef = r.erase();
        assert_eq!(erased.id.as_str(), "github-prod");
        assert_eq!(erased.key.as_str(), "oauth2_github");
    }
}
```

**Step 2: Run test to confirm it fails**
```bash
cargo test -p nebula-credential credential_ref_captures_id_and_key 2>&1 | head -20
```
Expected: compile error — `CredentialRef<GithubOAuth2>` doesn't exist yet.

**Step 3: Replace the implementation**

Replace the ENTIRE content of `crates/credential/src/core/reference.rs` with:

```rust
//! Credential references and provider traits.

use std::future::Future;
use std::marker::PhantomData;

use serde::{Deserialize, Serialize};

use crate::core::{CredentialContext, CredentialError, SecretString};
use crate::traits::CredentialType;

// ─── CredentialRef<C> ────────────────────────────────────────────────────────

/// Typed, compile-time reference to a specific credential instance.
///
/// Captures BOTH which instance (`CredentialId`) and which type (`C: CredentialType`).
/// Use `erase()` when you need to store it in a collection without generics.
///
/// # Example
/// ```rust,ignore
/// let prod   = CredentialRef::<GithubOAuth2>::new("github-prod").unwrap();
/// let staging = CredentialRef::<GithubOAuth2>::new("github-staging").unwrap();
/// // Both are GithubOAuth2, but different instances.
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CredentialRef<C: CredentialType> {
    pub id: crate::core::CredentialId,
    _phantom: PhantomData<fn() -> C>,
}

impl<C: CredentialType> CredentialRef<C> {
    /// Create a reference to a named credential instance.
    ///
    /// # Errors
    /// Returns `ValidationError` if the id is invalid (empty, path-traversal chars, etc).
    pub fn new(id: impl Into<String>) -> Result<Self, crate::core::ValidationError> {
        Ok(Self {
            id: crate::core::CredentialId::new(id)?,
            _phantom: PhantomData,
        })
    }

    /// The protocol-level key for this credential type (from nebula-core, D-015).
    /// Stable across compilations, serializable, human-readable.
    pub fn credential_key() -> nebula_core::CredentialKey {
        C::credential_key()
    }

    /// Erase the type parameter for storage in collections / manager internals.
    pub fn erase(self) -> ErasedCredentialRef {
        ErasedCredentialRef {
            id:  self.id,
            key: C::credential_key(),
        }
    }
}

// ─── ErasedCredentialRef ─────────────────────────────────────────────────────

/// Type-erased credential reference — used inside `ResourceComponents` and manager internals.
///
/// Preserves both the instance id and the protocol key (stable, serializable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErasedCredentialRef {
    /// Which credential instance ("github-prod", "stripe-live", …)
    pub id:  crate::core::CredentialId,
    /// Which protocol type ("oauth2_github", "api_key", …) — from nebula-core CredentialKey.
    pub key: nebula_core::CredentialKey,
}

// ─── CredentialProvider ──────────────────────────────────────────────────────

/// Provider trait for acquiring credentials — decouples acquisition from `CredentialManager`.
pub trait CredentialProvider: Send + Sync {
    /// Acquire typed credential state (returns raw `SecretString` for simple cases).
    fn credential<C: Send + 'static>(
        &self,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<SecretString, CredentialError>> + Send;

    /// Acquire by string id (type-erased fallback).
    fn get(
        &self,
        id: &str,
        ctx: &CredentialContext,
    ) -> impl Future<Output = Result<SecretString, CredentialError>> + Send;
}
```

**Step 4: Run tests**
```bash
cargo test -p nebula-credential 2>&1 | tail -20
```
Expected: all new tests pass, old `CredentialRef` tests may fail (they tested `TypeId` — remove them).

**Step 5: Fix broken usages of old `CredentialRef(TypeId)`**
```bash
cargo check -p nebula-credential 2>&1 | grep "error"
```
Fix any remaining call sites in `lib.rs` re-exports.

**Step 6: Commit**
```bash
git add crates/credential/src/core/reference.rs crates/credential/src/lib.rs
git commit -m "refactor(credential): replace CredentialRef(TypeId) with CredentialRef<C: CredentialType>"
```

---

### Task 2: Add `RotationStrategy` + `rotation_strategy()` to `CredentialResource`

**Files:**
- Modify: `crates/credential/src/traits/credential.rs`

**Step 1: Write failing test**

In the test module of `credential.rs`:
```rust
#[test]
fn default_rotation_strategy_is_hotswap() {
    struct MyHttpClient;
    impl CredentialResource for MyHttpClient {
        type Credential = TestApiKey;
        fn authorize(&mut self, _: &()) {}
        // no rotation_strategy override → default
    }
    assert!(matches!(MyHttpClient::rotation_strategy(), RotationStrategy::HotSwap));
}

#[test]
fn db_resource_declares_drain_and_recreate() {
    struct MyDbPool;
    impl CredentialResource for MyDbPool {
        type Credential = TestDbCred;
        fn authorize(&mut self, _: &()) {}
        fn rotation_strategy() -> RotationStrategy { RotationStrategy::DrainAndRecreate }
    }
    assert!(matches!(MyDbPool::rotation_strategy(), RotationStrategy::DrainAndRecreate));
}
```

**Step 2: Run test to confirm it fails**
```bash
cargo test -p nebula-credential default_rotation_strategy_is_hotswap 2>&1 | head -10
```

**Step 3: Add `RotationStrategy` and update `CredentialResource`**

In `crates/credential/src/traits/credential.rs`, add before `CredentialResource`:

```rust
/// Declares how the resource pool reacts when this resource's credential rotates.
///
/// Choose based on where authentication state lives in the client:
/// - Token in a header/field you can swap → `HotSwap`
/// - Password baked into a connection at connect-time → `DrainAndRecreate`
/// - Session-level auth (SSH, LDAP bind) → `Reconnect`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RotationStrategy {
    /// Call `authorize()` on all live instances. In-flight requests complete
    /// with old credential; new requests get the new credential immediately.
    /// Good for: HTTP bearer tokens, API keys in headers, gRPC metadata.
    #[default]
    HotSwap,

    /// Gracefully drain the pool (in-flight complete), then recreate all
    /// instances with the new credential. New instances call `authorize()` after creation.
    /// Good for: database connections, Redis AUTH, any connection-level auth.
    DrainAndRecreate,

    /// Immediately close all instances. Next acquire triggers fresh creation.
    /// Good for: SSH sessions, LDAP binds, any session-level auth.
    Reconnect,
}
```

Then update `CredentialResource`:
```rust
pub trait CredentialResource: Send + Sync + 'static {
    type Credential: CredentialType;

    /// Apply credential state to this instance.
    /// Called: (1) after `Resource::create()`, (2) on every rotation.
    fn authorize(&mut self, state: &<Self::Credential as CredentialType>::State);

    /// How the resource pool handles credential rotation.
    /// Override only if `HotSwap` is not correct for this resource.
    fn rotation_strategy() -> RotationStrategy
    where
        Self: Sized,
    {
        RotationStrategy::HotSwap
    }
}
```

**Step 4: Run tests**
```bash
cargo test -p nebula-credential 2>&1 | tail -10
```

**Step 5: Commit**
```bash
git add crates/credential/src/traits/credential.rs
git commit -m "feat(credential): add RotationStrategy enum and rotation_strategy() to CredentialResource"
```

---

### Task 3: Add `CredentialRotationEvent` + `rotation_subscriber()` to `CredentialManager`

**Why:** The resource manager needs to subscribe to rotation events so it can trigger `handle_rotation()` on affected pools.

**Files:**
- Modify: `crates/credential/src/manager/mod.rs` (or wherever `CredentialManager` is defined)
- Modify: `crates/credential/src/lib.rs`

**Step 1: Find the manager file**
```bash
grep -r "pub struct CredentialManager" crates/credential/src/ --include="*.rs" -l
```

**Step 2: Write failing test**
```rust
#[tokio::test]
async fn rotation_subscriber_receives_event() {
    let manager = CredentialManager::new_for_test();
    let mut sub = manager.rotation_subscriber();

    manager.emit_rotation_event(CredentialRotationEvent {
        credential_id: CredentialId::new("test-cred").unwrap(),
        credential_key: CredentialKey::new("api_key").unwrap(),
        new_state: serde_json::json!({"token": "new-value"}),
    });

    let event = tokio::time::timeout(Duration::from_millis(100), sub.recv())
        .await
        .expect("timeout")
        .expect("channel closed");
    assert_eq!(event.credential_id.as_str(), "test-cred");
}
```

**Step 3: Add `CredentialRotationEvent` and subscriber**

Add to the manager module:
```rust
use nebula_eventbus::EventBus;

/// Emitted after every successful credential rotation.
/// `ResourceManager` subscribes to this to trigger pool re-authorization.
#[derive(Debug, Clone)]
pub struct CredentialRotationEvent {
    /// Which credential instance was rotated.
    pub credential_id:  CredentialId,
    /// Which protocol type (stable, serializable — D-015).
    pub credential_key: nebula_core::CredentialKey,
    /// New credential state, type-erased as JSON.
    /// Deserialized by the pool's credential handler into the concrete State type.
    pub new_state:      serde_json::Value,
}
```

Add to `CredentialManager`:
```rust
pub struct CredentialManager {
    // ... existing fields ...
    rotation_bus: EventBus<CredentialRotationEvent>,
}

impl CredentialManager {
    /// Subscribe to credential rotation events.
    /// Call this from `ResourceManager` at startup.
    pub fn rotation_subscriber(&self) -> nebula_eventbus::EventSubscriber<CredentialRotationEvent> {
        self.rotation_bus.subscribe()
    }

    /// Internal: emit after a successful rotation (called inside rotate logic).
    pub(crate) fn emit_rotation(&self, event: CredentialRotationEvent) {
        self.rotation_bus.send(event);
    }
}
```

**Step 4: Run tests**
```bash
cargo test -p nebula-credential rotation_subscriber 2>&1 | tail -10
```

**Step 5: Commit**
```bash
git add crates/credential/src/manager/
git commit -m "feat(credential): add CredentialRotationEvent and rotation_subscriber() to CredentialManager"
```

---

## Phase 2 — nebula-resource: Remove old credential system

### Task 4: Delete `credentials.rs`, remove `ctx.credentials` from `Context`

**Why:** The string-based pull model (`ctx.credentials()?.get("key")`) is replaced entirely by the typed push model (`authorize(&state)` called by the pool). Keeping both creates confusion.

**Files:**
- Delete: `crates/resource/src/credentials.rs`
- Modify: `crates/resource/src/context.rs`
- Modify: `crates/resource/src/lib.rs`
- Modify: `crates/resource/src/http.rs` (if it uses ctx.credentials)

**Step 1: Find all usages of the old API**
```bash
grep -r "ctx\.credentials\|CredentialProvider\|SecureString" crates/resource/src/ --include="*.rs"
```

**Step 2: Remove `credentials.rs` from `mod.rs` / `lib.rs`**

In `crates/resource/src/lib.rs`, remove:
```rust
// REMOVE these lines:
pub mod credentials;
pub use credentials::{CredentialProvider, SecureString};
```

**Step 3: Remove `credentials` field from `Context`**

In `crates/resource/src/context.rs`, remove:
```rust
// REMOVE:
use crate::credentials::CredentialProvider;
// ...
pub credentials: Option<Arc<dyn CredentialProvider>>,
// ...
pub fn with_credentials(mut self, provider: Arc<dyn CredentialProvider>) -> Self { ... }
pub fn credentials(&self) -> Option<&dyn CredentialProvider> { ... }
```

Also remove the test `test_context_with_credentials`.

**Step 4: Delete the file**
```bash
rm crates/resource/src/credentials.rs
```

**Step 5: Fix compile errors**
```bash
cargo check -p nebula-resource 2>&1 | grep "error"
```
Fix any remaining usages (likely in `http.rs` or tests).

**Step 6: Run tests**
```bash
cargo test -p nebula-resource 2>&1 | tail -20
```

**Step 7: Commit**
```bash
git add -u crates/resource/
git commit -m "refactor(resource): remove legacy string-based credentials.rs and ctx.credentials"
```

---

## Phase 3 — nebula-resource: New typed system

### Task 5: Create `ResourceRef<R: Resource>`

**Files:**
- Create: `crates/resource/src/resource_ref.rs`
- Modify: `crates/resource/src/lib.rs`

**Step 1: Write failing test**

Create `crates/resource/src/resource_ref.rs` with tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct HttpPool;
    impl Resource for HttpPool {
        type Config = ();
        type Instance = ();
        fn metadata(&self) -> crate::metadata::ResourceMetadata { todo!() }
        async fn create(&self, _: &(), _: &crate::Context) -> crate::error::Result<()> { Ok(()) }
    }

    #[test]
    fn resource_ref_captures_key() {
        let r = ResourceRef::<HttpPool>::new("http-global").unwrap();
        assert_eq!(r.key.as_str(), "http-global");
    }

    #[test]
    fn erase_preserves_key() {
        let r = ResourceRef::<HttpPool>::new("http-global").unwrap();
        let erased = r.erase();
        assert_eq!(erased.key.as_str(), "http-global");
    }
}
```

**Step 2: Run to confirm it fails**
```bash
cargo test -p nebula-resource resource_ref_captures_key 2>&1 | head -10
```

**Step 3: Implement**

```rust
//! Typed resource reference — links a ResourceKey to a concrete Resource type at compile time.

use std::marker::PhantomData;
use nebula_core::ResourceKey;
use crate::resource::Resource;

/// Typed reference to a specific resource instance in the registry.
///
/// Carries the `ResourceKey` (string id) plus compile-time type information.
/// Use `erase()` to store in collections or pass to `ResourceComponents`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRef<R: Resource> {
    pub key:  ResourceKey,
    _phantom: PhantomData<fn() -> R>,
}

impl<R: Resource> ResourceRef<R> {
    /// Create a typed reference. Returns an error if `key` is not a valid `ResourceKey`.
    pub fn new(key: impl Into<String>) -> Result<Self, nebula_core::KeyError> {
        Ok(Self { key: ResourceKey::new(key)?, _phantom: PhantomData })
    }

    pub fn erase(self) -> ErasedResourceRef {
        ErasedResourceRef { key: self.key }
    }
}

/// Type-erased resource reference — stored inside `ResourceComponents`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErasedResourceRef {
    pub key: ResourceKey,
}
```

**Step 4: Add to `lib.rs`**
```rust
pub mod resource_ref;
pub use resource_ref::{ResourceRef, ErasedResourceRef};
```

**Step 5: Run tests**
```bash
cargo test -p nebula-resource resource_ref 2>&1 | tail -10
```

**Step 6: Commit**
```bash
git add crates/resource/src/resource_ref.rs crates/resource/src/lib.rs
git commit -m "feat(resource): add ResourceRef<R> typed resource reference"
```

---

### Task 6: Create `ResourceComponents`, `HasResourceComponents`, erased refs

**Files:**
- Create: `crates/resource/src/components.rs`
- Modify: `crates/resource/src/lib.rs`

**Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components_with_no_deps() {
        let c = ResourceComponents::new();
        assert!(c.credential.is_none());
        assert!(c.resources.is_empty());
    }

    #[test]
    fn components_with_credential() {
        let c = ResourceComponents::new()
            .credential::<GithubOAuth2>("github-prod");
        let r = c.credential_ref().unwrap();
        assert_eq!(r.id.as_str(), "github-prod");
        assert_eq!(r.key.as_str(), "oauth2_github");
    }

    #[test]
    fn components_with_resource() {
        let c = ResourceComponents::new()
            .resource::<HttpPool>("http-global");
        assert_eq!(c.resource_refs().len(), 1);
        assert_eq!(c.resource_refs()[0].key.as_str(), "http-global");
    }

    #[test]
    #[should_panic(expected = "invalid credential id")]
    fn components_credential_panics_on_bad_id() {
        ResourceComponents::new().credential::<GithubOAuth2>("");  // empty id → panic
    }
}
```

**Step 2: Run to confirm it fails**
```bash
cargo test -p nebula-resource components_with_no_deps 2>&1 | head -10
```

**Step 3: Implement `components.rs`**

```rust
//! ResourceComponents and HasResourceComponents — typed dependency declaration for resources.

use nebula_credential::ErasedCredentialRef;
use crate::resource::Resource;
use crate::resource_ref::{ErasedResourceRef, ResourceRef};

/// Declares what a resource instance needs: an optional credential and zero or more sub-resources.
///
/// Used by `HasResourceComponents` and consumed by `ResourceManager` at registration time to:
/// 1. Register dependencies in `DependencyGraph` (init order).
/// 2. Set up credential rotation subscription.
/// 3. Populate `Context` with resolved sub-resource handles before `create()`.
///
/// Fields are private — use `credential()` / `resource()` builders and `pub(crate)` accessors.
#[derive(Debug, Clone, Default)]
pub struct ResourceComponents {
    credential: Option<ErasedCredentialRef>,
    resources:  Vec<ErasedResourceRef>,
}

impl ResourceComponents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare the credential type and instance this resource requires.
    ///
    /// # Panics
    /// If `id` is not a valid `CredentialId`. In `HasResourceComponents::components()` this is
    /// always a string literal — panic is intentional (misconfiguration, not runtime error).
    ///
    /// # Example
    /// ```rust,ignore
    /// ResourceComponents::new().credential::<GithubOAuth2>("github-prod")
    /// ```
    pub fn credential<C>(mut self, id: impl Into<String>) -> Self
    where
        C: nebula_credential::CredentialType,
    {
        let id = nebula_credential::CredentialId::new(id)
            .expect("invalid credential id in HasResourceComponents::components()");
        self.credential = Some(ErasedCredentialRef { id, key: C::credential_key() });
        self
    }

    /// Declare a sub-resource dependency.
    ///
    /// # Panics
    /// If `key` is not a valid `ResourceKey`.
    ///
    /// # Example
    /// ```rust,ignore
    /// .resource::<HttpPool>("http-global")
    /// ```
    pub fn resource<R: Resource>(mut self, key: impl Into<String>) -> Self {
        let key = nebula_core::ResourceKey::new(key)
            .expect("invalid resource key in HasResourceComponents::components()");
        self.resources.push(ErasedResourceRef { key });
        self
    }

    // ─── Manager-internal accessors ─────────────────────────────────────────

    pub(crate) fn credential_ref(&self) -> Option<&ErasedCredentialRef> {
        self.credential.as_ref()
    }

    pub(crate) fn resource_refs(&self) -> &[ErasedResourceRef] {
        &self.resources
    }
}

/// Implement on a `Resource` factory to declare its credential and sub-resource dependencies.
///
/// # Example
/// ```rust,ignore
/// impl HasResourceComponents for GithubApiClient {
///     fn components() -> ResourceComponents {
///         ResourceComponents::new()
///             .credential::<GithubOAuth2>("github-prod")
///             .resource::<HttpPool>("http-global")
///     }
/// }
/// ```
pub trait HasResourceComponents: Resource {
    fn components() -> ResourceComponents
    where
        Self: Sized;
}
```

**Step 4: Add to `lib.rs`**
```rust
pub mod components;
pub use components::{HasResourceComponents, ResourceComponents};
```

**Step 5: Run tests**
```bash
cargo test -p nebula-resource components 2>&1 | tail -10
```

**Step 6: Compile check**
```bash
cargo check -p nebula-resource 2>&1 | grep "error"
```

**Step 7: Commit**
```bash
git add crates/resource/src/components.rs crates/resource/src/lib.rs
git commit -m "feat(resource): add ResourceComponents and HasComponents trait"
```

---

## Phase 4 — Context enrichment + Pool

### Task 7: Add `ctx.resource<R>()` — sub-resource injection into `Context`

**Why:** `HasResourceComponents` declares which sub-resources a factory needs. Without this task, those declarations sit unused — `create()` has no way to access the resolved sub-resource handles.

**Files:**
- Modify: `crates/resource/src/context.rs`

**Step 1: Write failing test**

```rust
#[test]
fn context_resolves_sub_resource_by_type() {
    use std::any::Any;
    use std::sync::Arc;

    struct HttpPool;
    impl Resource for HttpPool { /* ... */ }

    let http_pool_arc: Arc<dyn Any + Send + Sync> = Arc::new("http-pool-handle");

    let mut ctx = Context::new(Scope::global());
    ctx.inject_resource(ResourceKey::new("http-global").unwrap(), http_pool_arc);

    // Typed access
    let retrieved = ctx.resource::<HttpPool>("http-global");
    assert!(retrieved.is_some());
}
```

**Step 2: Run to confirm it fails**
```bash
cargo test -p nebula-resource context_resolves_sub_resource 2>&1 | head -10
```

**Step 3: Update `Context`**

In `crates/resource/src/context.rs`, add the resolved map and accessors:

```rust
use std::any::Any;
use std::collections::HashMap;
use nebula_core::ResourceKey;

pub struct Context {
    // ... existing fields ...

    /// Sub-resource handles injected by the manager before `create()`.
    /// Keyed by ResourceKey. Typed via `Any` downcast.
    resolved_resources: HashMap<ResourceKey, Arc<dyn Any + Send + Sync>>,
}

impl Context {
    // ... existing methods ...

    /// Inject a resolved sub-resource handle (called by manager, not by resource impls).
    pub(crate) fn inject_resource(
        &mut self,
        key: ResourceKey,
        handle: Arc<dyn Any + Send + Sync>,
    ) {
        self.resolved_resources.insert(key, handle);
    }

    /// Retrieve a resolved sub-resource pool handle.
    ///
    /// Returns `None` if not injected (resource not declared in `HasResourceComponents`, or not yet init).
    ///
    /// # Example
    /// ```rust,ignore
    /// let http = ctx.resource::<HttpPool>("http-global")
    ///     .expect("http-global declared in components");
    /// let instance = http.acquire(&ctx).await?;
    /// ```
    pub fn resource<R: Resource>(&self, key: &str) -> Option<Arc<Pool<R>>> {
        let rkey = ResourceKey::new(key).ok()?;
        self.resolved_resources
            .get(&rkey)?
            .clone()
            .downcast::<Pool<R>>()
            .ok()
            .map(Arc::from)
    }
}
```

**Step 4: Wire up in manager — populate `Context` before `create()`**

In `register_with_components()` (Task 10), before calling `Pool::new()`, the manager stores pool `Arc`s in a lookup table. Before each `create()`, it injects sub-resource handles into `Context`:

```rust
// Inside Pool::create_instance() or called by manager before create():
for dep_key in &components.resources {
    if let Some(dep_pool) = self.pools.get(&dep_key.key) {
        ctx.inject_resource(dep_key.key.clone(), Arc::clone(dep_pool) as _);
    }
}
```

Document this in Task 10 (manager implementation).

**Step 5: Run tests**
```bash
cargo test -p nebula-resource context 2>&1 | tail -10
```

**Step 6: Commit**
```bash
git add crates/resource/src/context.rs
git commit -m "feat(resource): add ctx.resource<R>() for sub-resource injection into Context"
```

---

### Task 8: Add credential state + `handle_rotation()` to `Pool<R>`

**Why:** The pool needs to (1) store current credential state as `serde_json::Value`, (2) call a type-erased authorizer after `create()`, and (3) react to rotation events with the declared strategy.

**Files:**
- Modify: `crates/resource/src/pool.rs`
- Modify: `crates/resource/src/events.rs`

**Step 1: Add `CredentialRotated` to `ResourceEvent`**

In `crates/resource/src/events.rs`, add to `ResourceEvent`:
```rust
/// A resource pool's credential was rotated and re-authorization was applied.
CredentialRotated {
    /// Resource key of the affected pool.
    resource_key: ResourceKey,
    /// The protocol type that was rotated.
    credential_key: nebula_core::CredentialKey,
    /// Strategy that was applied.
    strategy: String,  // "HotSwap" | "DrainAndRecreate" | "Reconnect"
},
```

**Step 2: Write failing tests for pool rotation**

In `crates/resource/src/pool.rs` test module:
```rust
#[tokio::test]
async fn pool_hot_swap_calls_authorize_on_all_idle() {
    // Create a pool with a test resource that tracks authorize() calls
    let counter = Arc::new(AtomicUsize::new(0));
    let pool = Pool::new_with_credential_handler(
        TestResource::new(counter.clone()),
        TestConfig,
        PoolConfig::default(),
        Some(Arc::new(CountingAuthorizer::new(counter.clone()))),
    );

    // Acquire and release to put instances in idle
    let guard = pool.acquire(test_ctx()).await.unwrap();
    drop(guard);

    let initial_count = counter.load(Ordering::SeqCst);
    pool.handle_rotation(
        &serde_json::json!({"token": "new"}),
        RotationStrategy::HotSwap,
    ).await.unwrap();

    assert!(counter.load(Ordering::SeqCst) > initial_count, "authorize should be called");
}
```

**Step 3: Add credential handler infrastructure to `Pool`**

Add to `crates/resource/src/pool.rs`:

```rust
use nebula_credential::RotationStrategy;

/// Type-erased credential handler stored in the pool.
/// Created at registration time by the manager; captures the concrete State type.
pub(crate) trait CredentialHandler<I>: Send + Sync {
    /// Apply serialized credential state to an instance.
    fn authorize(&self, instance: &mut I, state: &serde_json::Value) -> Result<()>;
    fn rotation_strategy(&self) -> RotationStrategy;
}

/// Pool extension: credential-aware fields (None for resources without credentials).
// Add to Pool<R> struct:
// credential_state:   Arc<RwLock<Option<serde_json::Value>>>,
// credential_handler: Option<Arc<dyn CredentialHandler<R::Instance>>>,
```

Add `handle_rotation()` to `Pool<R>`:
```rust
impl<R: Resource> Pool<R> {
    /// Called by `ResourceManager` when the bound credential rotates.
    pub async fn handle_rotation(
        &self,
        new_state: &serde_json::Value,
        strategy: RotationStrategy,
    ) -> Result<()> {
        // 1. Update stored state (new instances will use this)
        *self.credential_state.write() = Some(new_state.clone());

        match strategy {
            RotationStrategy::HotSwap => {
                // Call authorize() on every idle instance
                let handler = self.credential_handler.as_ref().ok_or(Error::NotConfigured)?;
                let mut idle = self.idle.lock();
                for slot in idle.iter_mut() {
                    handler.authorize(&mut slot.instance, new_state)?;
                }
            }
            RotationStrategy::DrainAndRecreate => {
                // Drain: let in-flight finish, evict all idle now
                self.drain_idle().await;
                // New instances will get authorize() called in create_instance()
            }
            RotationStrategy::Reconnect => {
                // Close all instances immediately (idle + in-flight are invalidated)
                self.drain_all().await;
            }
        }
        Ok(())
    }

    /// After create(), call authorize() if a credential handler is set.
    async fn create_instance(&self, config: &R::Config, ctx: &Context) -> Result<R::Instance> {
        let mut instance = self.resource.create(config, ctx).await?;
        if let (Some(handler), Some(state)) = (
            &self.credential_handler,
            &*self.credential_state.read(),
        ) {
            handler.authorize(&mut instance, state)?;
        }
        Ok(instance)
    }
}
```

**Step 4: Run tests**
```bash
cargo test -p nebula-resource pool 2>&1 | tail -20
```

**Step 5: Commit**
```bash
git add crates/resource/src/pool.rs crates/resource/src/events.rs
git commit -m "feat(resource): add credential state storage and handle_rotation() to Pool"
```

---

## Phase 5 — ResourceManager: component-based registration + rotation loop

### Task 9: `TypedCredentialHandler<I>` — the concrete bridge

**Why:** The pool stores `Arc<dyn CredentialHandler<R::Instance>>` (type-erased). This task creates the concrete impl that bridges the erased pool layer back to typed `CredentialResource::authorize()`. Must be done before Task 10 (manager uses it in `register_with_components()`).

**Files:**
- Modify: `crates/resource/src/components.rs`

**Step 1: Write test**
```rust
#[test]
fn typed_handler_deserializes_and_calls_authorize() {
    let mut instance = MockHttpClient::new();
    let handler = TypedCredentialHandler::<MockHttpClient>::new();
    handler.authorize(
        &mut instance,
        &serde_json::json!({"access_token": "tok123", "token_type": "Bearer"}),
    ).unwrap();
    assert_eq!(instance.current_token(), "tok123");
}

#[test]
fn typed_handler_returns_correct_rotation_strategy() {
    struct MyDb;
    impl CredentialResource for MyDb {
        type Credential = TestDbCred;
        fn authorize(&mut self, _: &()) {}
        fn rotation_strategy() -> RotationStrategy { RotationStrategy::DrainAndRecreate }
    }
    let handler = TypedCredentialHandler::<MyDb>::new();
    assert!(matches!(handler.rotation_strategy(), RotationStrategy::DrainAndRecreate));
}
```

**Step 2: Run to confirm they fail**
```bash
cargo test -p nebula-resource typed_handler 2>&1 | head -10
```

**Step 3: Implement in `components.rs`**

```rust
use nebula_credential::{CredentialResource, CredentialType, RotationStrategy};
use serde::de::DeserializeOwned;

/// Concrete credential handler for any instance implementing `CredentialResource`.
///
/// Stored in the pool at registration time. Deserializes JSON state
/// (from `CredentialRotationEvent::new_state`) and calls `authorize()`.
pub struct TypedCredentialHandler<I>(std::marker::PhantomData<fn() -> I>);

impl<I> TypedCredentialHandler<I> {
    pub fn new() -> Self { Self(std::marker::PhantomData) }
}

impl<I> Default for TypedCredentialHandler<I> {
    fn default() -> Self { Self::new() }
}

impl<I> CredentialHandler<I> for TypedCredentialHandler<I>
where
    I: CredentialResource,
    <I::Credential as CredentialType>::State: DeserializeOwned,
{
    fn authorize(&self, instance: &mut I, state: &serde_json::Value) -> crate::error::Result<()> {
        let typed = serde_json::from_value::<<I::Credential as CredentialType>::State>(
            state.clone(),
        )
        .map_err(|e| crate::error::Error::Configuration(e.to_string()))?;
        instance.authorize(&typed);
        Ok(())
    }

    fn rotation_strategy(&self) -> RotationStrategy {
        I::rotation_strategy()
    }
}
```

**Step 4: Run tests**
```bash
cargo test -p nebula-resource typed_handler 2>&1 | tail -10
```

**Step 5: Commit**
```bash
git add crates/resource/src/components.rs
git commit -m "feat(resource): add TypedCredentialHandler<I> — erased pool to typed CredentialResource bridge"
```

---

### Task 10: Add `register_with_components()` + rotation subscription to `ResourceManager`

**Files:**
- Modify: `crates/resource/src/manager.rs`

**Step 1: Write failing integration test**

```rust
#[tokio::test]
async fn manager_rotates_credential_for_hotswap_resource() {
    let cred_manager = Arc::new(MockCredentialManager::new());
    let mut manager = ResourceManager::new()
        .with_rotation_source(cred_manager.rotation_subscriber());

    manager.register_with_components::<GithubHttpClientFactory>(
        GithubHttpClientFactory,
        GithubConfig::default(),
        PoolConfig::default(),
        TypedCredentialHandler::<GithubHttpClient>::new(),
    ).await.unwrap();

    // Simulate rotation event
    cred_manager.emit_rotation(CredentialRotationEvent {
        credential_id:  CredentialId::new("github-prod").unwrap(),
        credential_key: CredentialKey::new("oauth2_github").unwrap(),
        new_state:      serde_json::json!({"access_token": "new-token"}),
    });

    // Give rotation loop time to process
    tokio::time::sleep(Duration::from_millis(10)).await;

    // Verify the pool was updated (check via pool stats or mock counter)
    let pool = manager.get_pool::<GithubHttpClientFactory>().unwrap();
    assert_eq!(pool.current_credential_key(), "new-token"); // mock accessor
}
```

**Step 2: Add `register_with_components()` to `ResourceManager`**

```rust
impl ResourceManager {
    /// Register a resource that has declared component dependencies.
    ///
    /// Automatically:
    /// 1. Registers sub-resource dependencies in `DependencyGraph`.
    /// 2. Creates the pool with an attached credential handler.
    /// 3. Maps credential id → pool (for rotation dispatch).
    pub async fn register_with_components<R>(
        &mut self,
        resource: R,
        config: R::Config,
        pool_config: PoolConfig,
        handler: impl CredentialHandler<R::Instance> + 'static,
    ) -> Result<()>
    where
        R: Resource + HasResourceComponents,
    {
        let components = R::components();

        // Register sub-resource deps in the dependency graph
        for dep in &components.resources {
            self.dependency_graph.add_dependency(
                resource.metadata().key.as_str(),
                dep.key.as_str(),
            )?;
        }

        // Create pool with credential handler attached
        let pool = Pool::new_with_credential_handler(
            resource,
            config,
            pool_config,
            Some(Arc::new(handler)),
        );

        let pool_arc = Arc::new(pool);

        // Track credential → pool mapping for rotation dispatch
        if let Some(cred_ref) = &components.credential {
            self.credential_pool_map
                .entry(cred_ref.id.clone())
                .or_default()
                .push(Arc::downgrade(&pool_arc) as _);
        }

        self.pools.insert(resource.metadata().key.clone(), pool_arc);
        Ok(())
    }

    /// Start the background rotation subscription loop.
    /// Call once at application startup after all resources are registered.
    pub fn spawn_rotation_listener(
        &self,
        mut sub: nebula_eventbus::EventSubscriber<nebula_credential::CredentialRotationEvent>,
    ) {
        let map = Arc::clone(&self.credential_pool_map);
        tokio::spawn(async move {
            while let Ok(event) = sub.recv().await {
                let pools = {
                    let guard = map.read();
                    guard.get(&event.credential_id)
                        .map(|v| v.iter().filter_map(|w| w.upgrade()).collect::<Vec<_>>())
                        .unwrap_or_default()
                };
                for pool in pools {
                    let strategy = pool.rotation_strategy();
                    if let Err(e) = pool.handle_rotation(&event.new_state, strategy).await {
                        tracing::error!(?e, "rotation failed for pool");
                    }
                }
            }
        });
    }
}
```

**Step 3: Run tests**
```bash
cargo test -p nebula-resource manager 2>&1 | tail -20
```

**Step 4: Full workspace check**
```bash
cargo check --workspace 2>&1 | grep "^error"
```

**Step 5: Commit**
```bash
git add crates/resource/src/manager.rs
git commit -m "feat(resource): add register_with_components() and rotation subscription loop to ResourceManager"
```

---

## Phase 6 — Integration + validation

### Task 9: End-to-end integration test

**Files:**
- Create: `crates/resource/tests/credential_integration.rs`

Write a test that exercises the full chain:
1. `CredentialManager` emits a `CredentialRotationEvent`
2. `ResourceManager` receives it and calls `pool.handle_rotation()`
3. For `HotSwap`: verify `authorize()` was called on idle instances
4. For `DrainAndRecreate`: verify idle instances were evicted

```rust
// crates/resource/tests/credential_integration.rs

use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use nebula_credential::{CredentialRotationEvent, CredentialId, RotationStrategy};
use nebula_resource::{ResourceManager, PoolConfig, HasResourceComponents, ResourceComponents};

// ... full test here ...

#[tokio::test]
async fn hotswap_rotation_calls_authorize_on_idle_instances() {
    // setup, emit rotation, assert authorize called
}

#[tokio::test]
async fn drain_recreate_rotation_evicts_idle_instances() {
    // setup, emit rotation, assert pool drained
}
```

**Step 1: Write tests**
**Step 2: Run — they fail**
**Step 3: Fix integration issues**
**Step 4: Run all workspace tests**
```bash
cargo test --workspace 2>&1 | tail -30
```
**Step 5: Run clippy**
```bash
cargo clippy --workspace -- -D warnings 2>&1 | grep "^error"
```
**Step 6: Commit**
```bash
git add crates/resource/tests/
git commit -m "test(resource): integration tests for credential rotation (HotSwap, DrainAndRecreate)"
```

---

### Task 10: `TypedCredentialHandler<I>` — the concrete bridge

**Why:** The manager needs a concrete `CredentialHandler` implementation that deserializes `serde_json::Value` into `I::Credential::State` and calls `I::authorize()`. This is the glue between the type-erased pool and the typed `CredentialResource` impl.

**Files:**
- Create or add to: `crates/resource/src/components.rs`

```rust
use nebula_credential::{CredentialResource, CredentialType, RotationStrategy};
use serde::de::DeserializeOwned;

/// Concrete credential handler for any instance implementing `CredentialResource`.
///
/// Stored in the pool. Deserializes JSON state and calls `authorize()`.
pub struct TypedCredentialHandler<I>(std::marker::PhantomData<fn() -> I>);

impl<I> TypedCredentialHandler<I> {
    pub fn new() -> Self { Self(std::marker::PhantomData) }
}

impl<I> CredentialHandler<I> for TypedCredentialHandler<I>
where
    I: CredentialResource,
    <I::Credential as CredentialType>::State: DeserializeOwned,
{
    fn authorize(&self, instance: &mut I, state: &serde_json::Value) -> crate::error::Result<()> {
        let typed = serde_json::from_value::<<I::Credential as CredentialType>::State>(
            state.clone()
        ).map_err(|e| crate::error::Error::Configuration(e.to_string()))?;
        instance.authorize(&typed);
        Ok(())
    }

    fn rotation_strategy(&self) -> RotationStrategy {
        I::rotation_strategy()
    }
}
```

**Step 1: Write test**
```rust
#[test]
fn typed_handler_deserializes_and_calls_authorize() {
    let mut instance = MockHttpClient::new();
    let handler = TypedCredentialHandler::<MockHttpClient>::new();
    handler.authorize(
        &mut instance,
        &serde_json::json!({"access_token": "tok123", "token_type": "Bearer"}),
    ).unwrap();
    assert_eq!(instance.current_token(), "tok123");
}
```

**Step 2: Run — fail, implement, run — pass**

**Step 3: Commit**
```bash
git add crates/resource/src/components.rs
git commit -m "feat(resource): add TypedCredentialHandler<I> — the bridge between pool and CredentialResource"
```

---

## Final check

```bash
# All tests pass
cargo test --workspace

# No warnings treated as errors
cargo clippy --workspace -- -D warnings

# Docs build
cargo doc --no-deps --workspace

# Format
cargo fmt --all -- --check
```

---

## Decision log (what we settled during design)

| Decision | Choice | Reason |
|----------|--------|--------|
| `CredentialRef` backing | `CredentialId` + `PhantomData<C>` | `TypeId` is unstable across compilations; `CredentialKey` is stable and serializable (D-015) |
| Credential injection timing | After `create()` via pool | Keeps `create()` credential-free; pool owns lifecycle |
| Credential state in pool | `serde_json::Value` | Must survive type erasure for storage; matches `ErasedProtocol` pattern (D-013) |
| Rotation dispatch | Manager subscribes to `CredentialRotationEvent` | Decoupled; manager knows which pools care about which credential |
| `credentials.rs` | Deleted | Entirely replaced by typed push model; keeping it creates two conflicting APIs |
| `RotationStrategy` location | `nebula-credential` | It describes how a `CredentialResource` behaves — belongs with that trait |
| `ResourceComponents` location | `nebula-resource` | It's about resource wiring, not credentials |
| `TypedCredentialHandler` | `nebula-resource` | Pool-side bridge; has no business in credential crate |
