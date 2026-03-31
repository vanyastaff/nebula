# nebula-credential Beta Refactor Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Bring nebula-credential to beta quality by fixing bugs, improving security, integrating shared crates (nebula-error, nebula-resilience), eliminating duplication, and removing dead code.

**Architecture:** The refactoring preserves the existing layer/trait architecture while replacing hand-rolled retry/circuit-breaker with nebula-resilience, migrating error types to use `#[derive(Classify)]`, deduplicating storage backends, and fixing all identified bugs. No breaking changes to types consumed by nebula-action/nebula-sdk (`CredentialSnapshot`, `CredentialMetadata`, `AnyCredential`, `CredentialContext`, `CredentialId`).

**Tech Stack:** Rust 1.93, nebula-error (`Classify` derive, `NebulaError<E>`), nebula-resilience (`CircuitBreaker`, `retry_with`), nebula-storage (KV `Storage` trait)

---

## Phase 1: Bug Fixes (correctness issues that affect runtime behavior)

### Task 1: Fix OAuth2 client_secret double-send (RFC 6749 violation)

**Files:**
- Modify: `crates/credential/src/credentials/oauth2_flow.rs:107-144`
- Modify: `crates/credential/src/credentials/oauth2_flow.rs` (all token exchange functions: `exchange_authorization_code`, `exchange_client_credentials`, `refresh_token`)

**Step 1: Read all three token exchange functions to verify the pattern**

Read `oauth2_flow.rs` fully.

**Step 2: Write failing tests**

```rust
// In oauth2_flow.rs tests — verify that AuthStyle::Header does NOT include client_secret in form body
#[tokio::test]
async fn header_auth_style_excludes_secret_from_body() {
    // Use a mock HTTP server (or inspect form params) to verify
    // When auth_style = Header:
    //   - Authorization: Basic <base64(id:secret)> header IS present
    //   - client_id and client_secret are NOT in the form body
}
```

**Step 3: Fix all three functions**

Pattern for each: when `AuthStyle::Header`, exclude `client_id` and `client_secret` from the form body. When `AuthStyle::PostBody`, include them in the form body only.

```rust
let mut form = vec![("grant_type", "authorization_code"), ("code", code)];

match config.auth_style {
    AuthStyle::Header => {
        let credentials = BASE64.encode(format!("{client_id}:{client_secret}"));
        req = req.header("Authorization", format!("Basic {credentials}"));
    }
    AuthStyle::PostBody => {
        form.push(("client_id", client_id));
        form.push(("client_secret", client_secret));
    }
}
```

**Step 4: Run tests**

Run: `rtk cargo nextest run -p nebula-credential -- oauth2`

**Step 5: Commit**

`fix(credential): exclude client_secret from POST body when using Header auth style`

---

### Task 2: Fix OAuth2Credential::refresh hardcoding ClientCredentials grant type

**Files:**
- Modify: `crates/credential/src/credentials/oauth2.rs:381-388`

**Step 1: Fix — use a grant-type-agnostic config builder for refresh**

The refresh call only needs `token_url` and potentially `auth_style`. The grant type is irrelevant because refresh always sends `grant_type=refresh_token`. But `auth_style` MUST be preserved from the original state.

Add `auth_style: AuthStyle` to `OAuth2State` (defaulting `Header` for backward compat via `#[serde(default)]`), then use it in refresh:

```rust
async fn refresh(state: &mut OAuth2State, _ctx: &CredentialContext) -> Result<RefreshOutcome, CredentialError> {
    if state.refresh_token.is_none() {
        return Ok(RefreshOutcome::ReauthRequired);
    }
    let config = OAuth2Config::client_credentials()
        .token_url(&state.token_url)
        .auth_style(state.auth_style)
        .scopes(state.scopes.clone())
        .build();
    oauth2_flow::refresh_token(state, &config).await?;
    Ok(RefreshOutcome::Refreshed)
}
```

Also update `state_from_token_response` to capture `auth_style` from the config.

**Step 2: Run tests**

Run: `rtk cargo nextest run -p nebula-credential -- oauth2`

**Step 3: Commit**

`fix(credential): preserve auth_style in OAuth2State for correct refresh behavior`

---

### Task 3: Fix RotationMetrics computing global avg instead of per-credential

**Files:**
- Modify: `crates/credential/src/rotation/metrics.rs:148-203`

**Step 1: Write failing test**

```rust
#[test]
fn per_credential_avg_duration_is_independent() {
    let metrics = RotationMetrics::new();
    let cred_a = CredentialId::new_v4();
    let cred_b = CredentialId::new_v4();

    metrics.record_rotation_duration(&cred_a, Duration::from_secs(10), true);
    metrics.record_rotation_duration(&cred_b, Duration::from_secs(100), true);

    let stats_a = metrics.credential_stats(&cred_a).unwrap();
    let stats_b = metrics.credential_stats(&cred_b).unwrap();

    // Each credential should have its own average, not the global one
    assert_eq!(stats_a.avg_duration, Some(Duration::from_secs(10)));
    assert_eq!(stats_b.avg_duration, Some(Duration::from_secs(100)));
}
```

**Step 2: Fix — compute per-credential average from per-credential data**

Either store per-credential durations (extra memory) or compute a running average. Simplest: running average.

```rust
// In CredentialMetricEntry, add:
pub(crate) total_duration: Duration,

// In record_rotation_duration:
cred_metrics.total_duration += duration;
cred_metrics.total += 1;
cred_metrics.avg_duration = Some(cred_metrics.total_duration / cred_metrics.total as u32);
```

**Step 3: Run tests, commit**

`fix(credential): compute per-credential avg_duration instead of global average`

---

### Task 4: Fix PostgresStore::list returning silent empty Vec

**Files:**
- Modify: `crates/credential/src/store_postgres.rs:221-226`

**Step 1: Change to return an explicit error**

```rust
async fn list(&self, _state_kind: Option<&str>) -> Result<Vec<String>, StoreError> {
    Err(StoreError::Backend(
        "list not yet supported: Storage trait lacks prefix-scan (see POSTGRES_STORAGE_SPEC.md)".into(),
    ))
}
```

**Step 2: Update the test that asserts empty to assert error**

**Step 3: Run tests, commit**

`fix(credential): PostgresStore::list returns explicit error instead of silent empty`

---

### Task 5: Fix CacheLayer::exists missing hit counter

**Files:**
- Modify: `crates/credential/src/layer/cache.rs:176-181`

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn exists_increments_hit_counter() {
    // put a credential, then call exists — stats.hits should be 1
}
```

**Step 2: Fix**

```rust
async fn exists(&self, id: &str) -> Result<bool, StoreError> {
    if self.cache.get(id).await.is_some() {
        self.hits.fetch_add(1, Ordering::Relaxed);
        return Ok(true);
    }
    self.misses.fetch_add(1, Ordering::Relaxed);
    self.inner.exists(id).await
}
```

**Step 3: Run tests, commit**

`fix(credential): increment CacheLayer hit/miss counters in exists()`

---

### Task 6: Fix DatabaseCredential silent port fallback

**Files:**
- Modify: `crates/credential/src/credentials/database.rs:128-132`

**Step 1: Validate port before using**

```rust
let port: u16 = match values.get_number("port") {
    Some(n) if (1..=65535).contains(&(n as u64)) => n as u16,
    Some(n) => return Err(CredentialError::InvalidInput(
        format!("invalid port number: {n}")
    )),
    None => DEFAULT_PORT,
};
```

**Step 2: Update test that relies on silent fallback**

**Step 3: Run tests, commit**

`fix(credential): reject invalid port values in DatabaseCredential instead of silent fallback`

---

### Task 7: Fix device code polling string-matching

**Files:**
- Modify: `crates/credential/src/credentials/oauth2_flow.rs` (poll_device_code)
- Modify: `crates/credential/src/credentials/oauth2.rs:354-362`

**Step 1: Add a typed error for device code poll states**

```rust
/// Device code poll outcome (not an error — used for control flow).
pub(crate) enum DevicePollStatus {
    /// Token received successfully.
    Ready(OAuth2State),
    /// Authorization still pending — poll again.
    Pending,
    /// Slow down — increase interval.
    SlowDown,
    /// Device code expired.
    Expired,
}
```

**Step 2: Change `poll_device_code` to return `Result<DevicePollStatus, CredentialError>`**

Parse the `error` field from the JSON response and match on the string value directly in `oauth2_flow.rs`, returning typed status instead of stringified errors.

**Step 3: Update `OAuth2Credential::resolve` to match on `DevicePollStatus` instead of string-searching**

**Step 4: Run tests, commit**

`fix(credential): replace string-matching in device code polling with typed DevicePollStatus`

---

## Phase 2: Security Fixes

### Task 8: Use SecretString for OAuth2State.client_secret

**Files:**
- Modify: `crates/credential/src/credentials/oauth2.rs:43-60`
- Modify: `crates/credential/src/credentials/oauth2_flow.rs` (state_from_token_response)
- Modify: `crates/credential/src/credentials/oauth2.rs` (all usages of `state.client_secret`)

**Step 1: Change `client_secret: String` to `client_secret: SecretString`**

Add custom serde: deserialize from JSON string, serialize as the raw value (not `[REDACTED]`) because this is state that gets encrypted at rest. Use a `pub(crate)` serde module for "transparent" SecretString serialization.

```rust
pub struct OAuth2State {
    // ...
    #[serde(with = "crate::utils::serde_secret")]
    pub client_secret: SecretString,
    // ...
}
```

**Step 2: Update all call sites** that read `state.client_secret` to use `.expose_secret(|s| ...)`.

**Step 3: Do the same for `OAuth2Pending.client_secret`**

**Step 4: Run tests, commit**

`fix(credential): use SecretString for OAuth2 client_secret with zeroize-on-drop`

---

### Task 9: Fix ScopeLayer::exists to not leak cross-tenant existence

**Files:**
- Modify: `crates/credential/src/layer/scope.rs:183-188`

**Step 1: Write failing test**

```rust
#[tokio::test]
async fn exists_returns_not_found_for_wrong_scope() {
    // Store credential in scope "tenant-a"
    // Query exists from scope "tenant-b"
    // Should return Ok(false), not Ok(true)
}
```

**Step 2: Implement — delegate exists through get + catch NotFound**

```rust
async fn exists(&self, id: &str) -> Result<bool, StoreError> {
    match self.get(id).await {
        Ok(_) => Ok(true),
        Err(StoreError::NotFound { .. }) => Ok(false),
        Err(e) => Err(e),
    }
}
```

This works because `ScopeLayer::get` already validates scope ownership.

**Step 3: Run tests, commit**

`fix(credential): ScopeLayer::exists respects tenant scope isolation`

---

### Task 10: Remove utils/time.rs (uses expect, unused by v2)

**Files:**
- Delete: `crates/credential/src/utils/time.rs`
- Modify: `crates/credential/src/utils/mod.rs` (remove `pub mod time`)

**Step 1: Grep for any callers of `unix_now`, `unix_now_millis`**

If none — delete the module.

**Step 2: Run tests, commit**

`chore(credential): remove unused utils/time.rs (v2 uses chrono::Utc::now)`

---

## Phase 3: Error System Cleanup

### Task 11: Add `#[derive(Classify)]` to simple error types

**Files:**
- Modify: `crates/credential/src/core/error.rs`
- Modify: `crates/credential/Cargo.toml` (add `nebula-error-macros` dependency if not present)

**Step 1: Replace manual Classify impls with derive for:**

- `CryptoError` — all variants are `category = "internal"`, differing only by code
- `ValidationError` — all variants are `category = "validation"`, differing only by code
- `StorageError` — each variant maps to a fixed category

Example:
```rust
#[derive(Debug, Error, nebula_error::Classify)]
pub enum CryptoError {
    #[classify(category = "internal", code = "CREDENTIAL:CRYPTO_DECRYPT")]
    #[error("Decryption failed - invalid key or corrupted data")]
    DecryptionFailed,
    // ... etc
}
```

Keep manual impl for `CredentialError` and `ManagerError` (they delegate to inner types).

**Step 2: Run tests to verify Classify behavior unchanged**

Run: `rtk cargo nextest run -p nebula-credential -- error`

**Step 3: Commit**

`refactor(credential): use derive(Classify) for CryptoError, ValidationError, StorageError`

---

### Task 12: Add Classify impls for v2 error types

**Files:**
- Modify: `crates/credential/src/credential_store.rs` (StoreError)
- Modify: `crates/credential/src/resolver.rs` (ResolveError)
- Modify: `crates/credential/src/credential_registry.rs` (RegistryError)
- Modify: `crates/credential/src/executor.rs` (ExecutorError)
- Modify: `crates/credential/src/pending_store.rs` (PendingStoreError)

**Step 1: Add `#[derive(Classify)]` to each**

```rust
#[derive(Debug, Error, nebula_error::Classify)]
pub enum StoreError {
    #[classify(category = "not_found", code = "CREDENTIAL:STORE_NOT_FOUND")]
    #[error("credential not found: {id}")]
    NotFound { id: String },

    #[classify(category = "conflict", code = "CREDENTIAL:STORE_VERSION_CONFLICT")]
    #[error("version conflict for {id}: expected {expected}, got {actual}")]
    VersionConflict { id: String, expected: u64, actual: u64 },

    #[classify(category = "conflict", code = "CREDENTIAL:STORE_ALREADY_EXISTS")]
    #[error("credential already exists: {id}")]
    AlreadyExists { id: String },

    #[classify(category = "internal", code = "CREDENTIAL:STORE_BACKEND")]
    #[error("store backend error: {0}")]
    Backend(Box<dyn std::error::Error + Send + Sync>),
}
```

Similarly for `ResolveError`, `RegistryError`, `ExecutorError`, `PendingStoreError`.

**Step 2: Run full test suite**

Run: `rtk cargo nextest run -p nebula-credential`

**Step 3: Commit**

`feat(credential): add Classify derive for StoreError, ResolveError, RegistryError, ExecutorError, PendingStoreError`

---

### Task 13: Fix From<StorageError> for CredentialError id extraction hack

**Files:**
- Modify: `crates/credential/src/core/error.rs:540-553`

**Step 1: Remove the `id` field from `CredentialError::Storage`**

The inner `StorageError` already carries the id for variants that have it. The outer `id` is redundant and wrong for `Timeout`/`NotSupported`.

```rust
/// Storage error for credential operation
#[error("storage error: {source}")]
Storage {
    #[source]
    source: StorageError,
},
```

**Step 2: Simplify From impl**

```rust
impl From<StorageError> for CredentialError {
    fn from(source: StorageError) -> Self {
        Self::Storage { source }
    }
}
```

**Step 3: Update all pattern matches on `CredentialError::Storage { id, source }` to `CredentialError::Storage { source }`**

**Step 4: Run tests, commit**

`refactor(credential): remove redundant id field from CredentialError::Storage`

---

### Task 14: Unify error variant for "missing field" across credentials

**Files:**
- Modify: `crates/credential/src/credentials/api_key.rs`
- Modify: `crates/credential/src/credentials/database.rs`
- Modify: `crates/credential/src/credentials/oauth2.rs`

**Step 1: Standardize on `CredentialError::InvalidInput` for user-provided parameter errors**

`Provider` should be for provider-side errors (HTTP failures, API errors). `InvalidInput` for user mistakes.

Replace all `CredentialError::Provider("missing required field ...")` with `CredentialError::InvalidInput("missing required field ...")`.

**Step 2: Run tests, commit**

`refactor(credential): use InvalidInput consistently for missing field errors`

---

### Task 15: Remove v1-remnant ManagerError wrapping from CredentialError

**Files:**
- Modify: `crates/credential/src/core/error.rs`
- Modify: `crates/credential/src/core/mod.rs` (re-exports)
- Modify: `crates/credential/src/lib.rs` (root re-exports)

**Step 1: Verify no external crate uses ManagerError, ManagerResult, RefreshErrorKind, RetryAdvice, ResolutionStage**

Already confirmed — only internal test files. Proceed.

**Step 2: Remove from re-exports in `lib.rs` and `core/mod.rs`:**

- `ManagerError`, `ManagerResult` — keep the type but remove from public API (make `pub(crate)`)
- `RefreshErrorKind`, `RetryAdvice` — keep for now (used by CredentialError::RefreshFailed), but plan to replace with `nebula_error::RetryHint` in future
- `ResolutionStage` — remove if unused internally (grep first)

**Step 3: Run full test suite, commit**

`refactor(credential): reduce public error surface by hiding v1-remnant types`

---

## Phase 4: DRY — Deduplicate Storage Backends

### Task 16: Add serde support to StoredCredential directly

**Files:**
- Modify: `crates/credential/src/credential_store.rs`
- Modify: `crates/credential/src/utils/mod.rs` (serde_base64 already exists)

**Step 1: Add `#[serde(with = "crate::utils::serde_base64")]` to `StoredCredential::data`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredCredential {
    pub id: String,
    pub state_kind: String,
    #[serde(with = "crate::utils::serde_base64")]
    pub data: Vec<u8>,
    pub version: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub metadata: HashMap<String, serde_json::Value>,
}
```

**Step 2: Run existing tests to verify serialization round-trips**

**Step 3: Commit**

`refactor(credential): add serde support with base64 data encoding to StoredCredential`

---

### Task 17: Remove duplicate serde wrapper structs from backends

**Files:**
- Modify: `crates/credential/src/store_local.rs` (remove `StoredFile`, use `StoredCredential` directly)
- Modify: `crates/credential/src/store_aws.rs` (remove `SecretPayload`, use `StoredCredential`)
- Modify: `crates/credential/src/store_k8s.rs` (remove `SecretPayload`, use `StoredCredential`)
- Modify: `crates/credential/src/store_postgres.rs` (remove `StoredEntry`, use `StoredCredential`)
- Modify: `crates/credential/src/store_vault.rs` (remove `VaultPayload`, use `StoredCredential`)

For each backend:
1. Remove the private serde struct and its `From` impls
2. Replace serialization calls with `serde_json::to_vec(&credential)` / `serde_json::from_slice`
3. Run backend-specific tests

**Special case: VaultPayload** — Vault KV v2 stores `HashMap<String, String>`. Convert `StoredCredential` to/from JSON `Value`, then to/from `HashMap` at the Vault API boundary. This eliminates the manual base64/RFC3339 encoding.

**Step 4: Run all tests, commit**

`refactor(credential): eliminate 5 duplicate serde wrappers — use StoredCredential directly`

---

### Task 18: Extract shared PutMode logic into a helper

**Files:**
- Create: helper function in `crates/credential/src/credential_store.rs` (in `test_helpers` or as a pub(crate) fn)
- Modify: all 6 store backends

**Step 1: Extract `apply_put_mode` function**

```rust
/// Apply PutMode semantics to a credential before storage.
/// Returns the credential to store, or an error if the mode rejects the operation.
pub(crate) fn apply_put_mode(
    credential: &mut StoredCredential,
    mode: PutMode,
    existing: Option<&StoredCredential>,
) -> Result<(), StoreError> {
    let now = Utc::now();
    match mode {
        PutMode::CreateOnly => {
            if existing.is_some() {
                return Err(StoreError::AlreadyExists { id: credential.id.clone() });
            }
            credential.version = 1;
            credential.created_at = now;
            credential.updated_at = now;
        }
        PutMode::Overwrite => {
            if let Some(prev) = existing {
                credential.version = prev.version + 1;
                credential.created_at = prev.created_at;
            } else {
                credential.version = 1;
                credential.created_at = now;
            }
            credential.updated_at = now;
        }
        PutMode::CompareAndSwap(expected) => {
            let actual = existing.map(|e| e.version).unwrap_or(0);
            if actual != expected {
                return Err(StoreError::VersionConflict {
                    id: credential.id.clone(),
                    expected,
                    actual,
                });
            }
            credential.version = expected + 1;
            credential.created_at = existing.map(|e| e.created_at).unwrap_or(now);
            credential.updated_at = now;
        }
    }
    Ok(())
}
```

**Step 2: Replace inline PutMode logic in each backend with `apply_put_mode` call**

**Step 3: Run all tests, commit**

`refactor(credential): extract shared PutMode logic into apply_put_mode helper`

---

### Task 19: Deduplicate make_credential test helper

**Files:**
- Modify: `crates/credential/src/layer/audit.rs` (remove local `make_credential`)
- Modify: `crates/credential/src/layer/scope.rs` (remove local `make_credential`)

**Step 1: Replace local `make_credential` calls with `credential_store::test_helpers::make_credential`**

**Step 2: Run tests, commit**

`refactor(credential): use shared make_credential test helper in audit and scope tests`

---

## Phase 5: Integrate nebula-resilience

### Task 20: Replace hand-rolled RefreshCoordinator circuit breaker with nebula-resilience

**Files:**
- Modify: `crates/credential/src/refresh.rs`
- Modify: `crates/credential/src/resolver.rs`
- Modify: `crates/credential/Cargo.toml` (add `nebula-resilience`)

**Step 1: Add dependency**

```toml
nebula-resilience = { path = "../resilience" }
```

**Step 2: Replace circuit breaker in RefreshCoordinator**

Current: hand-rolled `failure_counts: HashMap<String, FailureRecord>` with manual threshold check.

Replace with: `HashMap<String, Arc<CircuitBreaker>>` — one CB per credential ID, lazily created.

```rust
use nebula_resilience::{CircuitBreaker, CircuitBreakerConfig};

impl RefreshCoordinator {
    fn get_circuit_breaker(&self, credential_id: &str) -> Arc<CircuitBreaker> {
        let mut cbs = self.circuit_breakers.lock();
        cbs.entry(credential_id.to_string())
            .or_insert_with(|| {
                let config = CircuitBreakerConfig::new()
                    .failure_threshold(5)
                    .reset_timeout(Duration::from_secs(300))
                    .build()
                    .expect("valid CB config");
                Arc::new(CircuitBreaker::new(config).expect("valid CB"))
            })
            .clone()
    }
}
```

**Step 3: Update resolver.rs to use `cb.can_execute()` and `cb.record_outcome()`**

Replace `is_circuit_open`, `record_success`, `record_failure` calls.

**Step 4: Run tests, commit**

`refactor(credential): replace hand-rolled circuit breaker with nebula-resilience CircuitBreaker`

---

### Task 21: Wire RefreshPolicy::jitter using resilience JitterConfig

**Files:**
- Modify: `crates/credential/src/resolver.rs:117-123`
- Modify: `crates/credential/src/resolve.rs` (RefreshPolicy)

**Step 1: Apply jitter to early_refresh calculation**

```rust
let jitter = if C::REFRESH_POLICY.jitter > Duration::ZERO {
    let range = C::REFRESH_POLICY.jitter.as_millis() as u64;
    Duration::from_millis(rand::random::<u64>() % range)
} else {
    Duration::ZERO
};
let early = chrono::Duration::from_std(C::REFRESH_POLICY.early_refresh + jitter)
    .unwrap_or(chrono::Duration::zero());
```

**Step 2: Run tests, commit**

`feat(credential): apply jitter from RefreshPolicy to early refresh window`

---

## Phase 6: Dead Code & Inconsistency Cleanup

### Task 22: Remove stale code

**Files:**
- Modify: `crates/credential/src/pending_token.rs` (remove `#[allow(dead_code)]` on `generate()`)
- Modify: `crates/credential/src/utils/mod.rs` (remove `pub mod time` if unused)
- Delete: `crates/credential/src/utils/time.rs` (if no callers)
- Modify: `crates/credential/src/utils/validation.rs` (remove if dead, or wire into v2)
- Modify: `crates/credential/src/rotation/events.rs` (remove emoji from format strings)
- Modify: `crates/credential/src/rotation/transaction.rs:515` (`BackupId::as_str` → `to_string` or `Display`)
- Modify: `crates/credential/src/scheme/kerberos.rs` (rename `expires_at_time` → `expires_at`)
- Modify: `crates/credential/src/rotation/transaction.rs:666` (remove duplicate `rollback_transaction`, keep `mark_rolled_back`)

**Step 1: Grep for callers of each item to verify unused**

**Step 2: Make changes, run full test suite**

Run: `rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run -p nebula-credential`

**Step 3: Commit**

`chore(credential): remove stale code, fix naming inconsistencies, remove emoji from lib code`

---

### Task 23: Fix CredentialContext inconsistent field visibility

**Files:**
- Modify: `crates/credential/src/core/context.rs`

**Step 1: Make all fields private, add accessor methods for the remaining public ones**

Currently `owner_id`, `caller_scope`, `trace_id`, `timestamp` are pub. Make them private with accessors.

`timestamp` is unused — remove it entirely.

**Step 2: Update callers (grep for direct field access)**

**Step 3: Run tests, commit**

`refactor(credential): make CredentialContext fields private with accessors, remove unused timestamp`

---

### Task 24: Fix CredentialDescription.key divergence risk

**Files:**
- Modify: `crates/credential/src/core/description.rs`

**Step 1: Remove `key` from `CredentialDescriptionBuilder`**

The key should always come from `Credential::KEY`. Add a `from_credential<C: Credential>()` constructor or make `key` set internally from `Credential::KEY` in the trait's `description()` default.

**Step 2: Run tests, commit**

`refactor(credential): CredentialDescription.key derived from Credential::KEY, not user-settable`

---

## Phase 7: Verify & Update Context

### Task 25: Full validation

**Step 1:** Run full validation suite

```bash
rtk cargo fmt && rtk cargo clippy --workspace -- -D warnings && rtk cargo nextest run --workspace && rtk cargo test --workspace --doc
```

**Step 2:** Fix any issues

**Step 3:** Update `.claude/crates/credential.md` with changes made

**Step 4:** Update `.claude/active-work.md`

**Step 5:** Final commit

`docs: update context files for credential beta refactor`

---

## Deferred (separate tasks, not this plan)

These items were identified but are too large / breaking for this refactor:

1. **Rotation ↔ v2 trait integration** — rotation module has its own disconnected trait hierarchy
2. **CredentialRegistry returns Box<dyn Any>** — needs DI redesign
3. **ResolveError loses structured CredentialError info** — cascades to nebula-engine
4. **StoredCredential::data as EncryptedBlob newtype** — needs type-level encryption boundary
5. **String-matching error detection in AWS/Vault/K8s** — needs SDK typed error matching, complex per-backend
6. **Add ListableStorage to nebula-storage** — unblocks PostgresStore::list
7. **PendingStateStore::get() owner_id validation** — trait API change
