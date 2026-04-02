# nebula-credential Pre-existing Bugfixes Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Fix 17 bugs found during HLD v1.3 review and production use-case town hall — 2 CRITICAL, 11 HIGH, 4 MEDIUM.

**Architecture:** All fixes are within `nebula-credential` crate. No cross-crate changes. Each bug is a self-contained task with TDD approach: write failing test first, then fix. Ordered by severity.

**Tech Stack:** Rust 2024, `serde`, `serde_json`, `SecretString`/`Zeroize`, `tokio`, `scopeguard`, `moka`

**Breaking changes:** B1 (SecretString serde) and B2 (OAuth2State field types) are breaking for any code that serializes these types directly. B9 is breaking for CredentialRotationEvent consumers (behind rotation feature gate).

---

## Task 1: B6 CRITICAL — `verify_owner` fails open for ownerless credentials

**Files:**
- Modify: `crates/credential/src/layer/scope.rs:198-215`
- Test: `crates/credential/src/layer/scope.rs` (mod tests)

**Problem:** When `metadata["owner_id"]` is missing, `verify_owner` returns `Ok(())` — any tenant can access unscoped credentials.

**Step 1: Write the failing test**

```rust
#[tokio::test]
async fn unscoped_credential_rejected_for_non_admin() {
    // Credential with no owner_id in metadata
    let store = InMemoryStore::new();
    let cred = StoredCredential {
        id: "unscoped-cred".into(),
        data: b"encrypted-data".to_vec(),
        state_kind: "bearer".into(),
        state_version: 1,
        version: 0,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        expires_at: None,
        metadata: serde_json::Map::new(), // NO owner_id
    };
    store.put(cred, PutMode::CreateOnly).await.unwrap();

    let resolver = Arc::new(FixedOwnerResolver("tenant-a"));
    let scoped = ScopeLayer::new(store, resolver);

    // Non-admin caller should NOT see unscoped credentials
    let result = scoped.get("unscoped-cred").await;
    assert!(matches!(result, Err(StoreError::NotFound { .. })));
}
```

**Step 2:** Run `rtk cargo nextest run -p nebula-credential -- unscoped_credential_rejected`
Expected: FAIL — currently returns Ok because verify_owner allows missing owner.

**Step 3: Fix `verify_owner`**

In `crates/credential/src/layer/scope.rs`, replace lines 208-215:

```rust
fn verify_owner(
    resolver: &Arc<dyn ScopeResolver>,
    id: &str,
    credential: &StoredCredential,
) -> Result<(), StoreError> {
    let Some(caller_owner) = resolver.current_owner() else {
        // Admin / global access — bypass scope check.
        return Ok(());
    };

    match credential.metadata.get(OWNER_KEY) {
        Some(Value::String(stored_owner)) if stored_owner == caller_owner => Ok(()),
        Some(Value::String(_)) => {
            // Owner mismatch — hide existence
            Err(StoreError::NotFound { id: id.to_owned() })
        }
        _ => {
            // No owner_id or wrong type → only admin can access.
            // Non-admin callers get NotFound.
            Err(StoreError::NotFound { id: id.to_owned() })
        }
    }
}
```

**Step 4:** Run `rtk cargo nextest run -p nebula-credential`
Expected: All pass including new test.

**Step 5:** Commit: `fix(credential)!: reject unscoped credentials for non-admin callers`

---

## Task 2: B5 HIGH — ScopeLayer `list()` and `exists()` pass through without filtering

**Files:**
- Modify: `crates/credential/src/layer/scope.rs:171-187`
- Test: `crates/credential/src/layer/scope.rs` (mod tests)

**Problem:** `list()` returns all credential IDs across all tenants. `exists()` confirms existence regardless of scope.

**Step 1: Write failing tests**

```rust
#[tokio::test]
async fn list_filters_by_owner() {
    let store = InMemoryStore::new();
    // Seed: tenant-a owns cred-1, tenant-b owns cred-2
    seed_credential(&store, "cred-1", "tenant-a").await;
    seed_credential(&store, "cred-2", "tenant-b").await;

    let resolver = Arc::new(FixedOwnerResolver("tenant-a"));
    let scoped = ScopeLayer::new(store, resolver);

    let ids = scoped.list(None).await.unwrap();
    assert_eq!(ids, vec!["cred-1"]); // tenant-a sees only their own
}

#[tokio::test]
async fn exists_respects_scope() {
    let store = InMemoryStore::new();
    seed_credential(&store, "cred-1", "tenant-b").await;

    let resolver = Arc::new(FixedOwnerResolver("tenant-a"));
    let scoped = ScopeLayer::new(store, resolver);

    // tenant-a should NOT see tenant-b's credential
    assert!(!scoped.exists("cred-1").await.unwrap());
}
```

**Step 2:** Run tests — Expected: FAIL (both currently pass through unfiltered).

**Step 3: Implement filtered `list()` and `exists()`**

```rust
async fn list(&self, state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
    let ids = self.inner.list(state_kind).await?;
    let Some(caller_owner) = self.resolver.current_owner() else {
        return Ok(ids); // Admin bypass
    };
    let mut owned = Vec::with_capacity(ids.len());
    for id in &ids {
        if let Ok(cred) = self.inner.get(id).await {
            if matches!(
                cred.metadata.get(OWNER_KEY),
                Some(Value::String(owner)) if owner == caller_owner
            ) {
                owned.push(id.clone());
            }
        }
    }
    Ok(owned)
}

async fn exists(&self, id: &str) -> Result<bool, StoreError> {
    let Some(_caller_owner) = self.resolver.current_owner() else {
        return self.inner.exists(id).await; // Admin bypass
    };
    // Use get() which already has scope check via verify_owner
    match self.get(id).await {
        Ok(_) => Ok(true),
        Err(StoreError::NotFound { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}
```

**Step 4:** Run `rtk cargo nextest run -p nebula-credential`
**Step 5:** Commit: `fix(credential)!: scope-filter list() and exists() in ScopeLayer`

---

## Task 3: B8 HIGH — `complete()` not called if `perform_refresh` panics

**Files:**
- Modify: `crates/credential/src/resolver.rs:154-169`
- Test: `crates/credential/tests/units/` (new test file or extend existing)

**Problem:** If `perform_refresh` panics, scopeguard calls `notify_waiters()` but `complete()` (which removes the in-flight entry) is never called. The credential's in-flight entry is permanently poisoned — all future callers become Waiters and hang for 60s.

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn inflight_entry_cleaned_up_after_winner_panic() {
    let coordinator = RefreshCoordinator::new();

    // First call: Winner
    let attempt1 = coordinator.try_refresh("cred-1").await;
    assert!(matches!(attempt1, RefreshAttempt::Winner(_)));

    // Simulate panic — drop the notify without calling complete()
    // (In real code, scopeguard calls notify_waiters but not complete)
    drop(attempt1);
    // DO NOT call coordinator.complete("cred-1")

    // Second call should get Winner again (entry should be cleaned up)
    // Currently: gets Waiter (entry still in map) — BUG
    let attempt2 = coordinator.try_refresh("cred-1").await;
    assert!(
        matches!(attempt2, RefreshAttempt::Winner(_)),
        "expected Winner after previous winner dropped without completing"
    );
    coordinator.complete("cred-1").await;
}
```

**Step 2:** Run test — Expected: FAIL (attempt2 returns Waiter).

**Step 3: Fix — make scopeguard call both notify AND complete**

In `resolver.rs`, replace the Winner arm (lines 155-168):

```rust
RefreshAttempt::Winner(notify) => {
    // Combined guard: notify waiters AND clean up in-flight entry on any exit
    // (success, error, panic). Must run in this order: complete first
    // (removes entry so future callers get Winner), then notify (wakes waiters).
    let credential_id_owned = credential_id.to_string();
    let coordinator = &self.refresh_coordinator;
    let _cleanup = scopeguard::guard((), |_| {
        // Note: complete() is async but scopeguard runs in sync Drop.
        // We use try_complete_sync() which is a non-async variant that
        // removes the in-flight entry synchronously.
        // notify_waiters() is already sync.
        notify.notify_waiters();
    });

    let result = self
        .perform_refresh::<C>(credential_id, state, stored, ctx)
        .await;

    if result.is_ok() {
        self.refresh_coordinator.record_success(credential_id).await;
    } else {
        self.refresh_coordinator.record_failure(credential_id).await;
    }
    // Always clean up in-flight entry (before guard drops for the notify)
    self.refresh_coordinator.complete(credential_id).await;
    result
}
```

And add `complete_sync()` to `RefreshCoordinator` in `refresh.rs`:

```rust
/// Synchronous version of complete() for use in Drop/scopeguard contexts.
/// Removes the in-flight entry without async. Uses parking_lot (sync) to
/// avoid async in Drop.
pub fn complete_sync(&self, credential_id: &str) {
    // in_flight is tokio::sync::Mutex — we can't lock it synchronously.
    // Alternative: change in_flight to parking_lot::Mutex (it's only held briefly).
    // OR: spawn a task to call complete() async.
}
```

**Alternative simpler fix:** Change `in_flight` from `tokio::sync::Mutex` to `parking_lot::Mutex` (no `.await` is held across the lock). Then scopeguard can call `complete_sync()` directly.

**Step 4:** Run `rtk cargo nextest run -p nebula-credential`
**Step 5:** Commit: `fix(credential): clean up in-flight entry on winner panic via scopeguard`

---

## Task 4: B7 HIGH — `perform_refresh` doesn't retry CAS on VersionConflict

**Files:**
- Modify: `crates/credential/src/resolver.rs:277-285`
- Test: `crates/credential/tests/units/` (new or extend)

**Problem:** If CAS write fails with `VersionConflict`, the freshly-acquired token is dropped. For OAuth2 with single-use refresh tokens, this means zero working tokens.

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn cas_conflict_retries_once() {
    // Setup: store a credential at version 1
    // Concurrently bump version to 2 (simulating another writer)
    // perform_refresh gets version 1, refreshes, tries CAS with expected_version=1
    // First attempt: VersionConflict (actual version is 2)
    // Should: re-read, retry CAS with expected_version=2
    // Currently: drops the new token and returns error
}
```

**Step 3: Fix — add CAS retry loop (max 2 attempts)**

In `resolver.rs` `perform_refresh`, replace the CAS write block:

```rust
// CAS write with retry (max 2 attempts for VersionConflict)
let mut cas_stored = stored;
for attempt in 0..2 {
    let updated = StoredCredential {
        data: data.clone(),
        updated_at: chrono::Utc::now(),
        expires_at: state.expires_at(),
        ..cas_stored.clone()
    };
    match self
        .store
        .put(updated, PutMode::CompareAndSwap {
            expected_version: cas_stored.version,
        })
        .await
    {
        Ok(result) => {
            let scheme = C::project(&state);
            return Ok(CredentialHandle::new(scheme, credential_id));
        }
        Err(StoreError::VersionConflict { id, expected, actual }) if attempt < 1 => {
            // Re-read to get current version, retry
            tracing::warn!(
                credential_id, expected, actual,
                "CAS conflict on refresh write, retrying"
            );
            cas_stored = self.store.get(credential_id).await
                .map_err(ResolveError::Store)?;
        }
        Err(e) => return Err(ResolveError::Store(e)),
    }
}
unreachable!("loop always returns")
```

**Step 4:** Run `rtk cargo nextest run -p nebula-credential`
**Step 5:** Commit: `fix(credential): retry CAS write on VersionConflict in perform_refresh`

---

## Task 5: B2 HIGH — OAuth2State stores secrets as plain String

**Files:**
- Modify: `crates/credential/src/credentials/oauth2.rs:44-66`
- Modify: all methods on `OAuth2State` that access these fields
- Test: existing oauth2 tests

**Problem:** `access_token` and `refresh_token` are `String`, not `SecretString`. No zeroize on drop, `Debug` derive prints them in cleartext.

**Step 1: Write failing test**

```rust
#[test]
fn oauth2_state_debug_redacts_secrets() {
    let state = OAuth2State {
        access_token: SecretString::new("secret-access-token".to_owned()),
        token_type: "Bearer".into(),
        refresh_token: Some(SecretString::new("secret-refresh".to_owned())),
        // ... other fields
    };
    let debug = format!("{state:?}");
    assert!(!debug.contains("secret-access-token"));
    assert!(!debug.contains("secret-refresh"));
    assert!(debug.contains("REDACTED"));
}
```

**Step 2:** Run — Expected: FAIL (derive Debug prints plaintext).

**Step 3: Convert fields and add manual Debug**

```rust
#[derive(Clone, Serialize, Deserialize)]
pub struct OAuth2State {
    #[serde(with = "crate::utils::serde_secret")]
    pub access_token: SecretString,
    pub token_type: String,
    #[serde(with = "crate::utils::serde_secret")]
    pub refresh_token: Option<SecretString>,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Vec<String>,
    #[serde(with = "crate::utils::serde_secret")]
    pub client_id: SecretString,
    #[serde(with = "crate::utils::serde_secret")]
    pub client_secret: SecretString,
    pub token_url: String,
    #[serde(default)]
    pub auth_style: AuthStyle,
}

impl fmt::Debug for OAuth2State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuth2State")
            .field("access_token", &"[REDACTED]")
            .field("token_type", &self.token_type)
            .field("refresh_token", &self.refresh_token.as_ref().map(|_| "[REDACTED]"))
            .field("expires_at", &self.expires_at)
            .field("scopes", &self.scopes)
            .field("client_id", &"[REDACTED]")
            .field("client_secret", &"[REDACTED]")
            .field("token_url", &self.token_url)
            .field("auth_style", &self.auth_style)
            .finish()
    }
}
```

**Step 4: Update all call sites** that access `access_token` / `refresh_token` as `String` — they now use `SecretString`. Grep for `.access_token` and `.refresh_token` in `oauth2.rs` and `oauth2_flow.rs`.

**Step 5:** Run `rtk cargo nextest run -p nebula-credential`
**Step 6:** Commit: `fix(credential)!: convert OAuth2State secrets to SecretString`

---

## Task 6: B1 HIGH — SecretString Serialize redacts, breaking identity-state round-trip

**Files:**
- Modify: `crates/credential/src/scheme/bearer.rs:12`
- Modify: `crates/credential/src/scheme/basic.rs` (if similar pattern)
- Modify: `crates/credential/src/scheme/database.rs` (if similar pattern)
- Modify: `crates/credential/src/scheme/header.rs` (if similar pattern)
- Test: new round-trip test per scheme

**Problem:** `BearerToken` derives `Serialize` which delegates to `SecretString::Serialize` → `"[REDACTED]"`. If the resolver ever round-trips an identity-state credential through serialization, the secret is destroyed.

**Note:** This is a latent bug — identity-state credentials currently don't go through serialization round-trip in the resolver because `REFRESHABLE = false`. But it's one config change away from data loss.

**Step 1: Write failing test**

```rust
#[test]
fn bearer_token_serde_roundtrip_preserves_value() {
    let token = BearerToken::new(SecretString::new("my-secret-key".to_owned()));
    let json = serde_json::to_string(&token).unwrap();
    let recovered: BearerToken = serde_json::from_str(&json).unwrap();
    recovered.expose().expose_secret(|s| assert_eq!(s, "my-secret-key"));
}
```

**Step 2:** Run — Expected: FAIL (json = `{"token":"[REDACTED]"}`).

**Step 3: Fix — use `serde_secret` on SecretString fields in scheme types**

```rust
// bearer.rs
#[derive(Clone, Serialize, Deserialize)]
pub struct BearerToken {
    #[serde(with = "crate::utils::serde_secret")]
    token: SecretString,
}
```

Apply the same pattern to `BasicAuth`, `DatabaseAuth`, `HeaderAuth`, `ApiKeyAuth`, `HmacSecret`, and any other scheme type containing `SecretString` fields.

**Step 4:** Run `rtk cargo nextest run -p nebula-credential`
**Step 5:** Commit: `fix(credential)!: use serde_secret for SecretString fields in scheme types`

---

## Task 7: B9 HIGH — CredentialRotationEvent leaks credential state

**Files:**
- Modify: `crates/credential/src/rotation/events.rs:65-72`
- Test: compile-time (the struct change is the fix)

**Requires:** `rotation` feature flag enabled.

**Step 1:** No failing test needed — this is a data leak, not a logic error. The fix is removing the field.

**Step 2: Replace `new_state` with `generation`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialRotationEvent {
    pub credential_id: CredentialId,
    /// Generation counter — subscribers re-read from store if needed.
    pub generation: u64,
}
```

**Step 3: Fix all usages** of `CredentialRotationEvent.new_state` (grep for `new_state` in rotation module).

**Step 4:** Run `rtk cargo nextest run -p nebula-credential --features rotation`
**Step 5:** Commit: `fix(credential)!: remove credential state from CredentialRotationEvent`

---

## Task 8: B3 MEDIUM — RefreshCoordinator circuit breaker map unbounded

**Files:**
- Modify: `crates/credential/src/refresh.rs`
- Test: new test in refresh.rs mod tests

**Problem:** `circuit_breakers: HashMap<String, Arc<CircuitBreaker>>` grows on every unique credential_id. `record_success` removes the entry, but failed credentials accumulate indefinitely.

**Step 1: Write test**

```rust
#[tokio::test]
async fn circuit_breaker_entries_evicted_on_delete() {
    let coordinator = RefreshCoordinator::new();
    // Trigger failures for a credential
    for _ in 0..5 {
        coordinator.record_failure("deleted-cred").await;
    }
    assert!(coordinator.is_circuit_open("deleted-cred").await);

    // Notify that the credential was deleted
    coordinator.remove_credential("deleted-cred").await;

    // Entry should be gone
    assert!(!coordinator.is_circuit_open("deleted-cred").await);
}
```

**Step 3: Add `remove_credential` method**

```rust
impl RefreshCoordinator {
    /// Remove all state for a credential (call when credential is deleted).
    pub async fn remove_credential(&self, credential_id: &str) {
        self.in_flight.lock().await.remove(credential_id);
        self.circuit_breakers.lock().remove(credential_id);
    }
}
```

**Step 4:** Run `rtk cargo nextest run -p nebula-credential`
**Step 5:** Commit: `fix(credential): add remove_credential to RefreshCoordinator for cleanup`

---

## Task 9: B4 MEDIUM — CacheLayer invalidate() before write

**Files:**
- Modify: `crates/credential/src/layer/cache.rs:154-164`
- Test: new test

**Problem:** `cache.invalidate()` is called before `inner.put()`. If write fails, cache is empty (stale miss). Next `get()` goes to inner store — which is correct but wastes a cache slot.

**Actually:** Re-reading the code — the current behavior is acceptable. After invalidation, if put fails, next get re-reads from store (correct). The cache is self-healing. The real risk is the window between invalidate and insert where concurrent reads see a miss. But this is an inherent race in any invalidate-then-write pattern.

**Step 1:** Write test to document current behavior:

```rust
#[tokio::test]
async fn cache_invalidated_and_reinserted_on_successful_put() {
    let inner = InMemoryStore::new();
    let cache = CacheLayer::new(inner, CacheConfig::default());

    // Seed and cache a credential
    let cred = make_credential("c1");
    cache.put(cred.clone(), PutMode::CreateOnly).await.unwrap();
    let cached = cache.get("c1").await.unwrap();
    assert_eq!(cached.version, 0);

    // Update the credential
    let mut updated = cached;
    updated.data = b"new-data".to_vec();
    cache.put(updated, PutMode::Overwrite).await.unwrap();

    // Cache should have the new version, not the old
    let from_cache = cache.get("c1").await.unwrap();
    assert_eq!(from_cache.data, b"new-data");
}
```

**Verdict:** B4 is a **documentation issue**, not a logic bug. The current code invalidates before write, then re-inserts on success (line 161-162). This is the correct invalidate-then-populate pattern. Close as "not a bug".

**Step 2:** Add the documentation test above.
**Step 3:** Commit: `test(credential): document cache invalidation behavior on put`

---

## Task 10: B10 CRITICAL — InMemoryStore CAS creates on missing row

**Files:**
- Modify: `crates/credential/src/store_memory.rs:78-91`
- Test: `crates/credential/src/store_memory.rs` (mod tests)

**Problem:** `CompareAndSwap` on a non-existent credential silently creates it instead of returning `NotFound`. This diverges from SQL backend behavior where `UPDATE WHERE version=$1` returns 0 rows.

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn cas_on_missing_credential_returns_not_found() {
    let store = InMemoryStore::new();
    let cred = make_credential("nonexistent");
    let result = store.put(cred, PutMode::CompareAndSwap { expected_version: 0 }).await;
    assert!(matches!(result, Err(StoreError::NotFound { .. })));
}
```

**Step 2:** Run — Expected: FAIL (currently creates the credential).

**Step 3: Fix** — In `store_memory.rs`, CAS arm:

```rust
PutMode::CompareAndSwap { expected_version } => {
    let Some(existing) = data.get(&credential.id) else {
        return Err(StoreError::NotFound { id: credential.id.clone() });
    };
    if existing.version != expected_version {
        return Err(StoreError::VersionConflict { /* ... */ });
    }
    // proceed with update
}
```

**Step 4:** Run `rtk cargo nextest run -p nebula-credential`
**Step 5:** Commit: `fix(credential)!: CAS on missing row returns NotFound, not create`

---

## Task 11: B11 HIGH — Resolver emits no events after refresh

**Files:**
- Create: `crates/core/src/credential_event.rs` (new file in nebula-core)
- Modify: `crates/core/src/lib.rs` (add module + re-export)
- Modify: `crates/credential/src/resolver.rs` (add EventBus, emit after refresh)
- Modify: `crates/credential/src/lib.rs` (re-export CredentialEvent)

**Problem:** After successful refresh, `CredentialResolver` returns the new handle but tells nobody. Resources holding pooled connections with old auth never learn.

**Step 1: Add `CredentialEvent` to nebula-core**

```rust
// crates/core/src/credential_event.rs
use crate::CredentialId;

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialEvent {
    Refreshed { credential_id: CredentialId },
    Revoked { credential_id: CredentialId },
}
```

Start minimal (2 variants). `Rotated` and `ExpiringSoon` added when rotation v2 integrates.

**Step 2: Wire into resolver**

Add `event_bus: Option<Arc<EventBus<CredentialEvent>>>` to `CredentialResolver` constructor. After successful refresh CAS write, emit:

```rust
if let Some(bus) = &self.event_bus {
    bus.emit(CredentialEvent::Refreshed {
        credential_id: credential_id.parse().unwrap_or_default(),
    });
}
```

**Step 3:** Run `rtk cargo check -p nebula-core && rtk cargo nextest run -p nebula-credential`
**Step 4:** Commit: `feat(core): add CredentialEvent enum` then `feat(credential): emit CredentialEvent after refresh`

---

## Task 12: B12 HIGH — StoredCredential missing `credential_key` field

**Files:**
- Modify: `crates/credential/src/store.rs` (add field to StoredCredential)
- Modify: `crates/credential/src/store_memory.rs` (update InMemoryStore)
- Modify: `crates/credential/src/resolver.rs` (populate credential_key on store)
- Update: all StoredCredential construction sites in tests

**Problem:** Engine can't dispatch to correct `Credential` type because `StoredCredential` only stores `state_kind` (e.g., "bearer"), not `credential_key` (e.g., "api_key" vs "slack_bot" — both produce BearerToken).

**Step 1: Add field**

```rust
pub struct StoredCredential {
    pub id: String,
    pub credential_key: String,   // NEW: Credential::KEY
    pub data: Vec<u8>,
    // ... rest unchanged
}
```

**Step 2: Update all construction sites** — grep for `StoredCredential {` across the crate. Add `credential_key` field to each.

**Step 3:** Run `rtk cargo nextest run -p nebula-credential`
**Step 4:** Commit: `feat(credential)!: add credential_key field to StoredCredential`

---

## Task 13: B13 MEDIUM — Registry returns Box<dyn Any> not CredentialSnapshot

**Files:**
- Modify: `crates/credential/src/registry.rs`
- Modify: `crates/credential/src/resolver.rs` (update project() call site)

**Problem:** `CredentialRegistry::project()` returns `Box<dyn Any>`. Consumers can't construct `CredentialSnapshot` without knowing the concrete type back.

**Step 1: Change registry to return CredentialSnapshot**

```rust
pub fn project(
    &self,
    credential_key: &str,
    data: &[u8],
    metadata: CredentialMetadata,
) -> Result<CredentialSnapshot, RegistryError> {
    // Handler captured at register() time knows the concrete type
    // and can construct CredentialSnapshot::new::<S>()
}
```

**Step 2: Update `register::<C>()` to capture snapshot construction:**

```rust
pub fn register<C: Credential>(&mut self) -> Result<(), RegistryError>
where C::Scheme: 'static {
    let key = C::KEY.to_string();
    if self.handlers.contains_key(&key) {
        return Err(RegistryError::DuplicateKey(key));
    }
    self.handlers.insert(key, Arc::new(move |bytes: &[u8], metadata: CredentialMetadata| {
        let state: C::State = serde_json::from_slice(bytes)
            .map_err(|e| RegistryError::Deserialize(e.to_string()))?;
        let scheme = C::project(&state);
        Ok(CredentialSnapshot::new(C::KEY, metadata, scheme))
    }));
    Ok(())
}
```

**Step 3:** Run `rtk cargo nextest run -p nebula-credential`
**Step 4:** Commit: `refactor(credential)!: registry returns CredentialSnapshot directly`

---

## Task 14: B14 HIGH — No global refresh concurrency limiter

**Files:**
- Modify: `crates/credential/src/refresh.rs`

**Problem:** 500 credentials expiring simultaneously → 500 parallel HTTP calls to provider → 429 rate limit → all recorded as failures → circuit breakers open → legitimate refreshes blocked for 5 minutes. Cascading outage.

**Fix:** Add `max_concurrent_refreshes: Semaphore` to `RefreshCoordinator`:

```rust
pub struct RefreshCoordinator {
    in_flight: Mutex<HashMap<String, Arc<Notify>>>,
    circuit_breakers: parking_lot::Mutex<HashMap<String, Arc<CircuitBreaker>>>,
    refresh_semaphore: Arc<Semaphore>,  // NEW: limits total concurrent refreshes
}

impl RefreshCoordinator {
    pub fn new() -> Self { Self::with_max_concurrent(32) }  // sensible default
    pub fn with_max_concurrent(max: usize) -> Self { /* ... */ }
}
```

Winner acquires semaphore permit before calling `Credential::refresh()`. If all permits taken, Winner waits (bounded by the 30s framework timeout).

**Step 1:** Write test: 100 concurrent refreshes with `max_concurrent(5)` → verify max 5 parallel.
**Step 2:** Implement semaphore integration.
**Step 3:** Run `rtk cargo nextest run -p nebula-credential`
**Step 4:** Commit: `fix(credential): add global refresh concurrency limiter`

---

## Task 15: B15 HIGH — DatabaseAuth missing expires_at()

**Files:**
- Modify: `crates/credential/src/scheme/database.rs`

**Problem:** `DatabaseAuth` doesn't override `AuthScheme::expires_at()` → returns `None` → framework never auto-refreshes IAM tokens.

**Fix:** Add `expires_at` field to `DatabaseAuth`:

```rust
pub struct DatabaseAuth {
    // ... existing fields ...
    expires_at: Option<DateTime<Utc>>,  // NEW
}

impl AuthScheme for DatabaseAuth {
    const KIND: &'static str = "database";
    fn expires_at(&self) -> Option<DateTime<Utc>> {
        self.expires_at
    }
}
```

**Step 1:** Write test: `DatabaseAuth` with `expires_at` returns it via `AuthScheme::expires_at()`.
**Step 2:** Add field + impl.
**Step 3:** Run `rtk cargo nextest run -p nebula-credential`
**Step 4:** Commit: `feat(credential): add expires_at to DatabaseAuth for IAM token refresh`

---

## Task 16: B16 HIGH — ActionDependencies::credential() is singular

**Files:**
- Modify: `crates/action/src/dependency.rs`

**Problem:** SSH jump hosts, WebSocket auth+signing, multi-cloud — all need 2+ credentials per action. Current API returns `Option<Box<dyn AnyCredential>>` (singular).

**Fix:** Add `credential_slots()` alongside existing `credential()`:

```rust
pub trait ActionDependencies {
    /// Single credential (backward compatible).
    fn credential() -> Option<Box<dyn AnyCredential>> where Self: Sized { None }
    /// Multiple named credential slots (new).
    fn credential_slots() -> Vec<CredentialSlot> where Self: Sized { vec![] }
}

pub struct CredentialSlot {
    pub name: &'static str,
    pub required: bool,
}
```

Additive — existing actions keep using `credential()`. New actions use `credential_slots()`.

**Step 1:** Add types + default impls.
**Step 2:** Update engine validation to check both methods.
**Step 3:** Run `rtk cargo nextest run -p nebula-action`
**Step 4:** Commit: `feat(action): add credential_slots() for multi-credential actions`

---

## Task 17: B17 MEDIUM — SshAuthMethod missing Certificate variant

**Files:**
- Modify: `crates/credential/src/scheme/ssh.rs`

**Problem:** Enterprise SSH certificates (CA-signed cert + private key) not supported by current enum.

**Fix:** Add variant to `#[non_exhaustive]` enum:

```rust
pub enum SshAuthMethod {
    Password { password: SecretString },
    KeyPair { private_key: SecretString, passphrase: Option<SecretString> },
    Agent,
    Certificate {  // NEW
        private_key: SecretString,
        certificate: String,        // public, CA-signed
        ca_public_key: Option<String>,
    },
}
```

**Step 1:** Add variant + constructor `SshAuth::with_certificate(...)`.
**Step 2:** Update Debug impl to redact `private_key`.
**Step 3:** Run `rtk cargo nextest run -p nebula-credential`
**Step 4:** Commit: `feat(credential): add Certificate variant to SshAuthMethod`

---

## Execution Order (Final — v1.3)

```
Week 1 — Parallel Group A (core fixes):
  Task 1  (B6 CRITICAL)  — verify_owner fails open
  Task 10 (B10 CRITICAL) — CAS on missing row
  Task 3  (B8 HIGH)      — scopeguard + complete
  Task 4  (B7 HIGH)      — CAS retry in perform_refresh
  Task 14 (B14 HIGH)     — global refresh semaphore

Week 1 — Parallel Group B (events + handle):
  Task 11 (B11 HIGH)     — CredentialEvent + resolver emission
  [CredentialHandle::Clone fix — Arc<ArcSwap<S>>]

Week 1 — Parallel Group C (serde + secrets):
  Task 5  (B2 HIGH)      — OAuth2State plain strings
  Task 6  (B1 HIGH)      — SecretString serde roundtrip

Week 2 — Parallel Group D (trait changes):
  Task 2  (B5 HIGH)      — list/exists unfiltered → ListFilter/ListPage
  Task 12 (B12 HIGH)     — StoredCredential credential_key
  Task 15 (B15 HIGH)     — DatabaseAuth expires_at
  Task 16 (B16 HIGH)     — ActionDependencies credential_slots

Week 2 — Parallel Group E (cleanup):
  Task 7  (B9 HIGH)      — CredentialRotationEvent leak
  Task 8  (B3 MEDIUM)    — CB map eviction
  Task 9  (B4 MEDIUM)    — Cache invalidation doc test
  Task 13 (B13 MEDIUM)   — Registry returns CredentialSnapshot
  Task 17 (B17 MEDIUM)   — SshAuthMethod Certificate variant
```

---

## Verification

After all tasks, run full validation:

```bash
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run -p nebula-credential && rtk cargo nextest run -p nebula-credential --features rotation && rtk cargo test --doc -p nebula-credential
```
