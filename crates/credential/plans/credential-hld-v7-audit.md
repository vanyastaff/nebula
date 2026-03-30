# nebula-credential v2 — HLD v7 Audit

> **Date:** 2026-03-30
> **Based on:** All 7 plan documents + actual code after Phases 1-4 partial implementation
> **Source documents** (all in `crates/credential/plans/`):
> - **Research phase** (informed the v6 design):
>   - `auth_taxonomy_for_library.md` — auth concept taxonomy (protocol vs credential vs token vs transport)
>   - `interactive_auth_flow_model.md` — multi-step flow state machine design
>   - `nebula-auth-flat.md` — early iteration: flat single-crate approach (predecessor to v6)
> - **Final design** (HLD v6 + companions):
>   - `credential-hld-v6-final.md` — core types, traits, resolver, refresh, security model
>   - `credential-storage-hld.md` — storage backends, layer composition, PendingStateStore, key management
>   - `credential-api-hld.md` — REST endpoints, callback handling, error normalization
>   - `credential-flows-hld.md` — end-to-end flow sequences (OAuth2, SAML, Device Code, etc.)
>
> **Purpose:** Gap analysis, mismatch inventory, and prioritized fix plan

---

## Executive Summary

HLD v6 is a thorough design document. Phases 1-4 were partially implemented — the core skeleton exists (Credential trait, CredentialStore, Resolver, RefreshCoordinator, 5 AuthScheme types, 3 built-in credentials). However there are **significant gaps** between the HLD spec and the implementation, plus **architectural mismatches** where the implementation diverged from the design in ways that will cause problems.

The crate also carries **~19K LOC of v1 code** that is not deleted. v1 types are still re-exported from `lib.rs`. This creates confusion, bloats the public API surface, and makes it unclear which types to use.

---

## 1. Gap Inventory: HLD v6 vs Implementation

### 1.1 AuthScheme trait (nebula-core)

| HLD v6 spec | Implementation | Status |
|---|---|---|
| `Serialize + DeserializeOwned + Send + Sync + Clone + 'static` | `Send + Sync + Clone + 'static` | **MISMATCH** |
| `const KIND: &'static str` | absent | **MISSING** |
| `fn expires_at(&self) -> Option<DateTime<Utc>>` | absent | **MISSING** |

**Impact:** Without `KIND`, the resolver can't do scheme compatibility checks at the trait level. Without `expires_at()`, the framework can't schedule auto-refresh generically — it falls back to checking `CredentialStateV2::expires_at()` instead of the scheme, which means the HLD's three-step resolution flow (State → project → Scheme → `expires_at()` on scheme) is broken.

Without `Serialize + DeserializeOwned`, the `identity_state!` path works only because `CredentialStateV2` requires those bounds separately. But the HLD's security contract ("serialization happens exclusively inside EncryptionLayer") is weakened because there's no trait-level marker that AuthScheme types *can* be serialized.

**Recommendation:** Add `Serialize + DeserializeOwned` bounds and `KIND` + `expires_at()` to `AuthScheme` in nebula-core. This is a **breaking change to core** — all AuthScheme impls must update. Currently only 5 impls exist, so blast radius is manageable.

### 1.2 AuthScheme types

| Type | HLD v6 | Implemented | Notes |
|---|---|---|---|
| BearerToken | ✅ | ✅ | |
| BasicAuth | ✅ | ✅ | |
| DatabaseAuth | ✅ | ✅ | |
| ApiKeyAuth | ✅ | ✅ | Missing `apply_to_request()` helper |
| OAuth2Token | ✅ | ✅ | Missing `expires_at()` (no trait method) |
| HeaderAuth | ✅ | ❌ | |
| CertificateAuth | ✅ | ❌ | |
| SshAuth | ✅ | ❌ | |
| AwsAuth | ✅ | ❌ | |
| LdapAuth | ✅ | ❌ | |
| SamlAuth | ✅ | ❌ | |
| KerberosAuth | ✅ | ❌ | |
| HmacSecret | ✅ | ❌ | |

**8 of 13 types missing.** The 5 implemented ones are the most common. The missing ones are needed before production resources (SSH, AWS, LDAP, etc.) can use the v2 system.

### 1.3 CredentialHandle — Arc vs ArcSwap

| HLD v6 | Implementation |
|---|---|
| `scheme: ArcSwap<S>` | `scheme: Arc<S>` |
| `snapshot() -> Arc<S>` | `snapshot() -> &S` |
| `replace()` for hot-swap on refresh | absent |

**Impact:** The current `CredentialHandle` is immutable after creation. Auto-refresh cannot update the handle in place — callers must re-resolve. This defeats the HLD's "transparent auto-refresh" guarantee. The `ArcSwap` design allows `RefreshCoordinator` to swap in refreshed material while callers continue using valid snapshots.

**Recommendation:** Switch to `ArcSwap` and change `snapshot()` to return `Arc<S>`. This is a **breaking change** to `CredentialHandle`, but currently only tests use it.

### 1.4 CredentialContext — thin vs full

| HLD v6 field | Implementation |
|---|---|
| `owner_id` | ✅ |
| `caller_scope` | ✅ |
| `trace_id` | ✅ |
| `timestamp` | ✅ |
| `callback_url` | ❌ |
| `app_url` | ❌ |
| `session_id` | ❌ |
| `resolver: Option<Arc<dyn CredentialResolverRef>>` | ❌ |
| `resolve_credential<S>()` | ❌ |

**Impact:** Without `callback_url`, OAuth2/SAML credentials can't build redirect URLs. Without `resolver`, credential composition (AWS Assume Role depending on base credential) is impossible. Without `session_id`, the PendingStateStore can't do 4-dimensional token binding.

**Recommendation:** Extend `CredentialContext` with the missing fields. This is additive and non-breaking.

### 1.5 PendingStateStore — entirely missing

The HLD specifies a `PendingStateStore` trait with:
- `put<P>()` → `PendingToken` (4-dimensional binding: credential_kind + owner + session + token_id)
- `get<P>()` (read without consuming, for polling)
- `consume<P>()` (atomic read + delete)
- `delete()` (cleanup)
- `InMemoryPendingStore` implementation

**None of this exists.** Without it, interactive credential flows (OAuth2, SAML, device code) cannot work in v2.

### 1.6 Framework resolve/continue executor — missing

The HLD describes `execute_resolve()` and `execute_continue()` functions that:
- Wrap credential methods in 30s timeouts
- Handle PendingState lifecycle automatically
- Generate PendingTokens

These framework orchestration functions don't exist. Credential authors would have to manage PendingState manually, which violates the HLD's core principle.

### 1.7 Error model mismatch

| HLD v6 type | Implementation |
|---|---|
| `CredentialError` (author-facing, no credential_id) | `CredentialError` (v1, has credential_id, wraps Storage/Crypto/Validation) |
| `ResolutionError` (framework-facing, with context) | `ResolveError` (in resolver.rs, different structure) |
| `RetryAdvice` enum | ❌ |
| `RefreshErrorKind` enum | ❌ |
| `ResolutionStage` enum | ❌ |
| `CredentialError::RefreshFailed { kind, retry, source }` | ❌ |
| `CredentialError::CompositionFailed` | ❌ |
| `CredentialError::CompositionNotAvailable` | ❌ |
| `CredentialError::InvalidInput` | ❌ |
| `CredentialError::SchemeMismatch` | ❌ |

**Impact:** The current `CredentialError` is a v1 type with v1-specific variants (Storage{id, source}, Crypto, Manager). The v2 `Credential` trait returns this same error type, but it lacks the v2-specific variants the HLD defines. Two error types coexist: `core::error::CredentialError` (v1) and `resolver::ResolveError` (v2). This will cause confusion.

**Recommendation:** Create the v2 `CredentialError` as specified in HLD. Either rename the v1 error or phase it out.

### 1.8 RefreshCoordinator — simplified vs hardened

| HLD v6 feature | Implementation |
|---|---|
| DashMap for lock-free coordination | HashMap + tokio::sync::Mutex |
| Circuit breaker (5 failures in 5 min) | ❌ |
| Failure tracking (`failure_counts`) | ❌ |
| `RefreshState` enum (Idle/Refreshing/Failed) | ❌ (only Notify in map) |
| Waiter timeout (60s max) | ❌ |
| scopeguard on Notify | ❌ |
| Framework 30s timeout on refresh calls | ❌ |
| RetryAdvice clamping (min 5s backoff) | ❌ |
| CAS write with lifecycle-aware loser handling | ❌ |

**Impact:** The current coordinator prevents thundering herd but lacks all defensive measures. A hung credential plugin can block workers forever. A flapping credential can cause hot-loop refresh attempts. A panic in the winner leaves waiters hanging.

**Priority: HIGH** — these are the adversarial-round hardening measures from HLD v6 Rev 6.

### 1.9 CredentialStore API mismatch

| HLD v6 | Implementation |
|---|---|
| `put(&self, id: &CredentialId, entry: &StoredCredential, mode: PutMode)` | `put(&self, credential: StoredCredential, mode: PutMode)` |
| `get(&self, id: &CredentialId)` | `get(&self, id: &str)` |
| `list(&self, filter: &ListFilter)` | `list(&self, state_kind: Option<&str>)` |
| `StoredCredential.scheme_kind` | ❌ |
| `StoredCredential.metadata: CredentialMetadata` | `metadata: serde_json::Map` |
| `CredentialLifecycle` enum | ❌ |

**Impact:** The simplified API works for current needs but diverges from HLD. `CredentialId` vs `&str` is a type safety issue. Missing `scheme_kind` means no scheme compatibility validation at storage level. Missing `CredentialLifecycle` means no lifecycle-aware CAS loser handling.

### 1.10 Storage layers

| Layer | HLD v6 | Implemented |
|---|---|---|
| EncryptionLayer | ✅ | ✅ |
| CacheLayer (ciphertext-only) | ✅ | ❌ |
| ScopeLayer (multi-tenant isolation) | ✅ | ❌ |
| AuditLayer (redacted metadata) | ✅ | ❌ |

Only EncryptionLayer exists. The others are Phase 5 per the migration plan.

### 1.11 Storage backends

| Backend | HLD v6 | Adapted to v2 |
|---|---|---|
| InMemoryStore | ✅ | ✅ |
| LocalFileStore | ✅ | ❌ |
| PostgresStore | ✅ | ❌ |
| VaultStore | ✅ | ❌ |
| AwsSecretsStore | ✅ | ❌ |
| K8sSecretsStore | ✅ | ❌ |

Only InMemoryStore adapted. All others still use v1 `StorageProvider` trait.

### 1.12 Other missing items

| Item | Status |
|---|---|
| `CredentialKey` newtype (vs `&'static str`) | ❌ |
| `credential_key!()` macro | ❌ |
| `#[derive(Credential)]` macro | ❌ |
| `StaticProtocol` trait (reusable protocols) | ❌ (v1 version exists but incompatible) |
| `FromParameters` derive macro | ❌ |
| Scheme coercion (TryFrom between schemes) | ❌ |
| `CredentialRotatedEvent` + EventBus | ❌ |
| `SecretGuard` with redacted Display/Debug | ❌ (uses closure-based `expose_secret()`) |
| `scrub_ephemeral()` on CredentialState | ❌ |
| `CredentialStateV2` → `CredentialState` rename | blocked by v1 |

---

## 2. Architectural Problems

### 2.1 V1/V2 coexistence is toxic

**Problem:** `lib.rs` re-exports both v1 and v2 types. This means:
- `CredentialError` refers to the v1 error (via `core::error`)
- `Credential` trait (v2) uses the v1 `CredentialError`
- v1 `CredentialProvider`, `CredentialType`, `FlowProtocol`, `InteractiveCredential`, `Refreshable`, `Revocable`, `StaticProtocol`, `StorageProvider` are all still public
- v1 `CredentialManager` with its own `CacheLayer`, `CacheConfig`, etc. coexists with v2 `CredentialStore`
- v1 `protocols/` module has protocol types that shadow v2 credential types
- prelude re-exports v1 types

**Impact:** External consumers cannot tell which API to use. The v1 public surface is ~50+ types that should not be there.

**Recommendation:** Delete v1 code. It's in git history. The migration plan said "big-bang rewrite" — time to actually rewrite. At minimum, gate v1 behind a `deprecated-v1` feature flag and remove from default exports.

### 2.2 Error type shared across v1/v2

The `CredentialError` in `core::error` serves both the v1 manager and the v2 `Credential` trait. Its variants (`Storage{id, source}`, `Crypto`, `Manager`, `Validation`) are v1 concepts. The v2 trait only uses `NotInteractive` and `Provider(String)`, which were tacked on.

The HLD v6 specifies a clean v2 `CredentialError` with `InvalidInput`, `RefreshFailed{kind, retry, source}`, `RevokeFailed`, `CompositionFailed`, etc. None of these exist.

**Recommendation:** Create a separate v2 `CredentialError` per HLD. The v1 error stays with v1 code.

### 2.3 CredentialStateV2 naming

The trait is called `CredentialStateV2` because v1 has a `CredentialState` type in `core::state`. This V2 suffix propagates through the entire v2 API. Once v1 is deleted, rename to `CredentialState`.

### 2.4 SecretString API differs from HLD

HLD specifies `SecretString::expose() -> SecretGuard<'_>` where `SecretGuard` has:
- `Deref<Target=str>` for `.len()`, `.contains()`, etc.
- `Display` prints `<REDACTED>` (safe for logging)
- `Debug` prints `SecretGuard(<REDACTED>)`
- `as_str()` for explicit secret access

Current implementation uses a closure-based `expose_secret(|s| ...)` pattern. This is functional but ergonomically worse — requires a closure for every access, can't easily pass the reference around.

**Recommendation:** Adopt the HLD's `SecretGuard` pattern. More ergonomic, equally safe.

---

## 3. Quality Issues

### 3.1 CredentialHandle::snapshot() returns &S, not Arc<S>

This prevents sharing across tasks. HLD specifies `Arc<S>` to allow multiple concurrent users of the same credential snapshot. The current `&S` requires the handle to outlive all uses.

### 3.2 RefreshCoordinator::complete() is not guaranteed

HLD specifies scopeguard to ensure `notify_waiters()` is always called, even on panic. Current implementation relies on the caller remembering to call `complete()`. A panic in `perform_refresh()` in the resolver will leave all waiters hanging forever.

### 3.3 No timeout on credential methods

HLD specifies 30s hard limits on `resolve()`, `continue_resolve()`, `refresh()`, `test()`, `revoke()`. Without these, a hung credential plugin blocks the tokio worker indefinitely.

### 3.4 resolver.rs uses `stored.version` after moving `stored`

In `perform_refresh()`:
```rust
let updated = StoredCredential {
    data,
    updated_at: chrono::Utc::now(),
    expires_at: state.expires_at(),
    ..stored  // moves stored
};
self.store.put(updated, PutMode::CompareAndSwap {
    expected_version: stored.version,  // ← uses moved value
}).await
```
Wait — actually Rust struct update syntax `..stored` moves remaining fields, so `stored.version` should have been moved too. Let me re-check... actually `stored.version` is `u64` which is `Copy`, so this works. But the `..stored` effectively consumes `stored`, and `stored.version` is used after. Since `version` is `Copy`, this compiles. Tricky but correct.

### 3.5 No early refresh window

The resolver checks `expires_at <= Utc::now()` — only refreshes when already expired. HLD specifies `early_refresh` (default 5 min before expiry) with jitter. Current behavior means callers may get expired tokens.

---

## 4. Findings from Companion Documents

### 4.1 credential-storage-hld.md — Storage gaps

This document specifies significantly more detail than what's implemented:

**KeyManager (key rotation):**
- HLD specifies `KeyManager { current, previous }` for key rotation
- `EncryptionLayer` should try current key first, fallback to previous
- Background re-encryption job reads all creds, decrypts old, encrypts new
- **Not implemented.** Current `EncryptionLayer` has a single `Arc<EncryptionKey>`. No rotation support.

**AAD (Additional Authenticated Data):**
- HLD specifies credential_id as AAD in AES-256-GCM
- Prevents record-swapping (moving ciphertext between credential IDs)
- **Not implemented.** Current `encrypt_data()` doesn't pass AAD.

**CacheLayer details:**
- Moka LRU + TTL, ciphertext-only, invalidation on put/delete
- `CacheConfig { max_entries: 10_000, ttl: 5min, tti: 2min }`
- **Not implemented.** Only `EncryptionLayer` exists.

**Storage migration:**
- `CredentialState::VERSION` + `StoredCredential.state_version`
- Migration on read: check version, run migration function, re-encrypt
- `StateMigration` trait + migration registry
- **Not implemented.** `state_version` is stored but never checked.

**PendingStateStore backends:**
- HLD specifies InMemory, Redis, Postgres backends
- Redis: GET+DEL in transaction/Lua for atomic consume
- Postgres: RETURNING clause for atomic consume
- **None exist.** PendingStateStore trait itself doesn't exist.

**Configuration:**
- `StorageConfig { backend, cache, encryption, pending }`
- `BackendConfig` enum (InMemory/LocalFile/Postgres/Vault/AWS/K8s)
- `KeySource` enum (Direct/AwsKms/VaultTransit/File)
- **Not implemented.**

### 4.2 credential-api-hld.md — HTTP API gaps

This is entirely future work (nebula-api crate), but worth noting:

- REST API design for credential CRUD, OAuth2/SAML callbacks, device code polling
- PendingToken stored in HttpOnly SameSite=Lax cookie (NOT in URL)
- Error normalization: ResolutionError → generic public error codes
- Error sanitization: provider errors stripped (may contain secrets)
- Rate limiting per endpoint
- `If-Match` header for CAS on credential updates
- Credential picker API (`?scheme_kind=bearer&lifecycle=Active`)

**Key insight:** The API layer depends on `PendingStateStore`, `CredentialLifecycle`, `ResolutionError`, `ResolutionStage` — all of which are missing from the current implementation. The API cannot be built until these exist.

### 4.3 credential-flows-hld.md — Integration flow gaps

Detailed end-to-end sequences for:
- OAuth2 Authorization Code + PKCE (Google, GitHub, Microsoft, Slack, etc.)
- OAuth2 Client Credentials (server-to-server, no redirect)
- Google Service Account (JWT grant)
- SAML 2.0 (POST binding)
- OAuth2 Device Code (CLI/headless)

**Provider-specific quirks documented:**
| Provider | Quirk |
|----------|-------|
| GitHub | auth_style = PostBody (not BasicAuth for token exchange) |
| Google | `access_type=offline` + `prompt=consent` for offline access |
| Microsoft | Tenant-specific endpoints |
| Slack | Comma-separated scopes (not space-separated) |
| Salesforce | Custom domains |

These quirks need to be handled by the `OAuth2Flow` builder when implemented.

### 4.4 auth_taxonomy_for_library.md — Design validation

This document establishes that auth concepts (protocol, credential, token format, transport, session, provider, interaction) should be modeled as **orthogonal dimensions**, never mixed into one enum.

**nebula-credential v2 passes this test:** `AuthScheme` (what you produce) is orthogonal from `Credential` (how you produce it), `PendingState` (flow state), and `CredentialStore` (persistence). The taxonomy is clean.

However, the v1 code violates it: `ApiKeyProtocol`, `BasicAuthProtocol`, `OAuth2Protocol` mix protocol-level concerns (flow mechanics) with credential-level concerns (what gets stored). This is another reason to delete v1.

### 4.5 interactive_auth_flow_model.md — Flow architecture validation

This document defines the core principle: **"Core owns flow semantics, integrator owns orchestration."**

nebula-credential v2's `Credential::resolve()` / `continue_resolve()` model matches this:
- Credential returns `ResolveResult::Pending { state, interaction }` (flow semantics)
- Framework stores pending state, manages cookies, handles timeouts (orchestration)

The `FlowStatus<Action, State, Output>` pattern from this doc maps directly to `ResolveResult<State, Pending>`. The design is sound.

### 4.6 nebula-auth-flat.md — Early design iteration

This is an **earlier iteration** of the nebula-credential v2 design, exploring a flat single-crate approach. Key ideas that made it into the final v6 design:

- `Authenticator` trait (non-interactive) → became the `Credential` trait with `resolve() → Complete`
- `InteractiveAuthenticator` (begin/advance state machine) → became `resolve()` / `continue_resolve()` with `ResolveResult::Pending`
- `FlowStatus<Action, State, Output>` → became `ResolveResult<State, Pending>` + `InteractionRequest`
- `AuthAction` (Redirect, ShowUserCode, Prompt) → became `InteractionRequest` enum
- `FlowEvent` (CallbackReceived, Poll, OtpSubmitted) → became `UserInput` enum
- `DynInteractiveAuthenticator` (type-erased for registry) → became `CredentialRegistry` with `ProjectFn`
- Taxonomy enums (`ProtocolKind`, `CredentialKind`, etc.) → informed the clean separation of `AuthScheme` from `Credential` from `PendingState`

**What was dropped or changed:**
- The two-trait split (`Authenticator` vs `InteractiveAuthenticator`) was unified into a single `Credential` trait with capability flags — simpler, less boilerplate
- `FlowState::is_expired()` convenience method → not in v2 (framework checks `expires_at()` directly)
- `DynInteractiveAuthenticator` blanket impl via `serde_json::Value` → replaced by `CredentialRegistry` with concrete `ProjectFn` closures
- axum/reqwest adapter modules → deferred to nebula-api layer

This doc is historical context, not an active design. The ideas were refined into HLD v6.

---

## 5. Prioritized Action Plan

### P0 — Must fix before any more development

1. **Delete v1 code** or gate behind feature flag
   - Remove v1 re-exports from `lib.rs`
   - Remove v1 types from prelude
   - Rename `CredentialStateV2` → `CredentialState`
   - Clean up `CredentialError` — separate v2 error

2. **Add scopeguard to RefreshCoordinator**
   - One-line fix: `let _guard = scopeguard::guard(notify, |n| n.notify_waiters());`
   - Without this, panics cause permanent waiter hangs

3. **Add early refresh window to resolver**
   - Use `RefreshPolicy::early_refresh` instead of `<= now`

### P1 — Required for production use

4. **Extend AuthScheme** in nebula-core
   - Add `const KIND: &'static str`
   - Add `fn expires_at(&self) -> Option<DateTime<Utc>> { None }`
   - Add `Serialize + DeserializeOwned` bounds
   - Update all 5 impls

5. **Switch CredentialHandle to ArcSwap**
   - `snapshot() -> Arc<S>`
   - Add `replace()` method

6. **Extend CredentialContext**
   - Add `callback_url`, `app_url`, `session_id`
   - Add `resolver` for credential composition

7. **Create v2 CredentialError** per HLD
   - `InvalidInput`, `RefreshFailed{kind, retry, source}`, `RevokeFailed`, `CompositionFailed`
   - `RetryAdvice`, `RefreshErrorKind`, `ResolutionStage`

8. **Harden RefreshCoordinator**
   - DashMap or keep Mutex but add circuit breaker
   - Waiter timeout (60s)
   - Framework timeout on refresh (30s)
   - RetryAdvice clamping

### P2 — Required for interactive flows

9. **Implement PendingStateStore** + InMemoryPendingStore
10. **Implement framework resolve/continue executors**
11. **Add PendingToken** with 4-dimensional binding

### P3 — Required for full AuthScheme coverage

12. **Add missing 8 AuthScheme types**
13. **Add scheme coercion** (TryFrom impls)
14. **Add CredentialRotatedEvent** + EventBus integration

### P4 — DX improvements

15. **`CredentialKey` newtype** + `credential_key!()` macro
16. **`#[derive(Credential)]` macro**
17. **`StaticProtocol` trait** for reusable protocols
18. **SecretGuard** pattern for ergonomic secret access

### P5 — Storage backends (Phase 5 of migration plan)

19. **CacheLayer, ScopeLayer, AuditLayer**
20. **Adapt LocalFileStore, PostgresStore, VaultStore, AwsSecretsStore, K8sSecretsStore**

---

## 6. What the HLD Gets Wrong (or should change)

### 5.1 CredentialStore::put() signature

HLD: `put(&self, id: &CredentialId, entry: &StoredCredential, mode: PutMode)`
Implementation: `put(&self, credential: StoredCredential, mode: PutMode)`

The implementation is simpler — `StoredCredential` already contains its `id`. Having `id` as a separate parameter is redundant and can cause bugs (id mismatch between parameter and struct). **Keep the implementation's version.**

### 5.2 CredentialStore uses &str not CredentialId

HLD uses `CredentialId` newtype everywhere. The newtype adds type safety but also adds conversion boilerplate. For a store trait, `&str` is pragmatic. **Decision needed:** add `CredentialId` newtype if it will carry validation (max length, charset), otherwise keep `&str`.

### 5.3 StoredCredential.metadata type

HLD: `metadata: CredentialMetadata` (structured type with owner_id, lifecycle, timestamps)
Implementation: `metadata: serde_json::Map<String, Value>` (untyped)

The untyped version is more flexible but loses type safety. The HLD's `CredentialLifecycle` needs a home. **Recommendation:** Add `lifecycle` as a top-level field on `StoredCredential`, keep `metadata` as the untyped extension bag.

### 5.4 SecretString design

HLD proposes `SecretGuard` with `Deref<Target=str>`. This is ergonomic but arguably less safe than the closure-based approach — a reference can be accidentally stored or logged. The closure forces explicit scoping.

**Recommendation:** Keep both. `expose_secret(|s| ...)` for most uses, `expose() -> SecretGuard` as a convenience for cases where the closure is awkward. Document that `SecretGuard` should not be stored.

### 5.5 DashMap vs Mutex in RefreshCoordinator

HLD suggests DashMap for lock-free coordination. The current Mutex is fine for expected cardinality (< 1000 concurrent credentials). DashMap adds a dependency and complexity. **Keep Mutex**, but add the defensive features (circuit breaker, timeout, scopeguard).

### 5.6 `CredentialState::scrub_ephemeral()`

HLD adds `fn scrub_ephemeral(&mut self) {}` for zeroizing ephemeral secrets before persistence. This is a nice-to-have but questionable — `EncryptionLayer` handles security, and `Zeroize on drop` handles memory. The serialization buffer can use `Zeroizing<Vec<u8>>`. **Skip unless a concrete use case appears.**

---

## 7. Recommended Execution Order

```
Step 1: P0 — v1 cleanup + scopeguard + early refresh       (1-2 days)
Step 2: P1 items 4-5 — AuthScheme + CredentialHandle       (1 day)
Step 3: P1 items 6-8 — Context + Error + RefreshCoordinator (2-3 days)
Step 4: P2 — PendingStateStore + executors                  (2-3 days)
Step 5: P3 — AuthScheme types + coercion + EventBus         (1-2 days)
Step 6: P4 — DX macros + SecretGuard                        (1-2 days)
Step 7: P5 — Storage layers + backends                      (3-5 days)
```

Total: ~12-18 days of focused work. Step 1 is critical — everything else builds on a clean codebase without v1 noise.
