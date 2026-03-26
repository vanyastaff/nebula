# nebula-credential Storage — High-Level Design

> **Companion to:** credential-hld-v6-final.md (core types and traits)
> **Scope:** Storage backends, layer composition, encryption, caching, scoping,
> audit, PendingStateStore, key management, migration.

---

## Overview

nebula-credential separates storage into two subsystems:

1. **CredentialStore** — persistent storage for encrypted credential state.
   Long-lived (days → years). CRUD with CAS. Layered with encryption, cache, scope, audit.

2. **PendingStateStore** — ephemeral storage for interactive flow state.
   Short-lived (minutes). TTL-enforced. 4-dimensional token binding.
   Single-use consume semantics.

Both share the same EncryptionLayer primitives (AES-256-GCM) but have
different lifecycle, consistency, and deployment requirements.

---

## CredentialStore — Persistent State

### Trait recap (from core HLD)

```rust
pub trait CredentialStore: Send + Sync {
    fn put(&self, id: &CredentialId, entry: &StoredCredential, mode: PutMode)
        -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;
    fn get(&self, id: &CredentialId)
        -> impl Future<Output = Result<StoredCredential, StoreError>> + Send;
    fn delete(&self, id: &CredentialId)
        -> impl Future<Output = Result<(), StoreError>> + Send;
    fn list(&self, filter: &ListFilter)
        -> impl Future<Output = Result<Vec<CredentialEntry>, StoreError>> + Send;
    fn exists(&self, id: &CredentialId)
        -> impl Future<Output = Result<bool, StoreError>> + Send;
}
```

### StoredCredential format

```rust
pub struct StoredCredential {
    pub state_kind: String,        // "oauth2", "bearer", "database"
    pub scheme_kind: String,       // "oauth2", "bearer", "database"
    pub state_version: u16,        // CredentialState::VERSION at serialize time (for migrations)
    pub data: Vec<u8>,             // ciphertext (AES-256-GCM encrypted JSON)
    pub metadata: CredentialMetadata,
    pub version: u64,              // CAS version, incremented on every write
}

pub struct CredentialMetadata {
    pub owner_id: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub lifecycle: CredentialLifecycle,  // Active | ReauthRequired | Terminal
}
```

**On-disk format:** JSON envelope with ciphertext payload:
```json
{
  "state_kind": "oauth2",
  "scheme_kind": "oauth2",
  "data": "base64(nonce + ciphertext + tag)",
  "metadata": {
    "owner_id": "user-123",
    "created_at": "2026-01-15T10:00:00Z",
    "updated_at": "2026-03-25T14:30:00Z",
    "lifecycle": "Active"
  },
  "version": 7
}
```

The `data` field is opaque ciphertext. Only EncryptionLayer can decrypt it.
Cache, scope, audit layers never see plaintext credential state.

---

## Layer Composition

### Ordering (outermost → innermost)

```
Request → ScopeLayer → AuditLayer → EncryptionLayer → CacheLayer → Backend
```

| Layer | Role | Sees plaintext? |
|-------|------|-----------------|
| ScopeLayer | Multi-tenant isolation. Rejects cross-tenant access. | No |
| AuditLayer | Logs access. Receives metadata only. | No |
| EncryptionLayer | Decrypts on read, encrypts on write. | Yes (briefly) |
| CacheLayer | Caches ciphertext (moka LRU + TTL). | No |
| Backend | Raw persistence (file, DB, Vault). | No |

```rust
let store = LocalFileStore::new(data_dir)
    .layer(CacheLayer::ciphertext(cache_config))
    .layer(EncryptionLayer::new(master_key))
    .layer(AuditLayer::redacted(audit_sink))
    .layer(ScopeLayer::new(scope_resolver));
```

**Read path:** ScopeLayer validates tenant → AuditLayer logs → EncryptionLayer calls inner.get() → CacheLayer checks cache → Backend reads → CacheLayer stores ciphertext → EncryptionLayer decrypts → returns plaintext to caller.

**Write path:** ScopeLayer validates → AuditLayer logs → EncryptionLayer encrypts → CacheLayer invalidates old entry → Backend writes → CacheLayer optionally caches new ciphertext → returns committed StoredCredential.

### Invariants

- Plaintext exists only inside EncryptionLayer boundary, transiently
- Cache stores ciphertext — no plaintext in cache heap
- AuditLayer never sees `data` field content — only metadata
- ScopeLayer checked before any data access — fail fast
- CacheLayer invalidates on put() and delete() — no stale credentials after rotation

---

## EncryptionLayer

### Algorithm: AES-256-GCM

```rust
pub struct EncryptionLayer<S: CredentialStore> {
    inner: S,
    key: Arc<EncryptionKey>,
}

pub struct EncryptionKey {
    key_id: String,
    key_bytes: Zeroizing<[u8; 32]>,  // 256-bit key, zeroized on drop
    created_at: DateTime<Utc>,
}
```

**Encrypt (write path):**
1. Serialize state to JSON → `Zeroizing<Vec<u8>>` (zeroized buffer)
2. Generate 12-byte random nonce (CSPRNG)
3. AES-256-GCM encrypt(key, nonce, plaintext, aad=credential_id)
4. Output: `nonce(12) || ciphertext || tag(16)`
5. Drop `Zeroizing<Vec<u8>>` — plaintext buffer zeroized
6. Store ciphertext blob in `StoredCredential.data`

**Decrypt (read path):**
1. Load ciphertext from `StoredCredential.data`
2. Split: nonce(12) || ciphertext || tag(16)
3. AES-256-GCM decrypt(key, nonce, ciphertext, aad=credential_id)
4. Deserialize JSON → State type
5. Return State to caller (CredentialResolver projects to Scheme)

**AAD (Additional Authenticated Data):** credential_id is used as AAD. This
binds the ciphertext to the credential — moving encrypted data to a different
credential ID causes authentication failure on decrypt. Prevents record swapping.

### Key management

```rust
pub struct KeyManager {
    current: Arc<EncryptionKey>,
    previous: Option<Arc<EncryptionKey>>,  // for rotation window
}
```

**Key rotation:**
1. Generate new key → becomes `current`
2. Old key → becomes `previous`
3. Decrypt: try `current` first, fallback to `previous`
4. Encrypt: always use `current`
5. Background re-encryption job: read all credentials, decrypt with old, encrypt with new
6. After re-encryption complete: drop `previous`

**Key storage:** Master key stored outside the credential store:
- **Dev/local:** environment variable or file (plaintext, `chmod 600`)
- **Production:** KMS (AWS KMS, Azure Key Vault, GCP KMS, HashiCorp Vault Transit)
- **Key derivation:** HKDF-SHA256 from master key → per-credential-type subkeys (optional, Phase N)

---

## CacheLayer

### Design: ciphertext-only moka cache

```rust
pub struct CacheLayer<S: CredentialStore> {
    inner: S,
    cache: moka::future::Cache<CredentialId, CachedEntry>,
}

struct CachedEntry {
    credential: StoredCredential,  // contains ciphertext, never plaintext
    cached_at: Instant,
}
```

**Configuration:**
```rust
pub struct CacheConfig {
    pub max_entries: u64,         // default: 10_000
    pub ttl: Duration,            // default: 5 minutes
    pub tti: Duration,            // time-to-idle, default: 2 minutes
}
```

**Cache key:** `CredentialId` (no version — invalidation-based).

**Invalidation strategy:**
- `put()`: invalidate cache entry for the credential ID before writing
- `delete()`: invalidate cache entry
- TTL: moka auto-evicts after `ttl`
- TTI: moka auto-evicts after `tti` of inactivity

**Security:** Cache eviction does NOT zeroize entries (standard allocator
reclaims memory). Acceptable because cache stores ciphertext, not plaintext.
No secret material in cache heap.

---

## ScopeLayer

### Multi-tenant isolation

```rust
pub struct ScopeLayer<S: CredentialStore> {
    inner: S,
    resolver: Arc<dyn ScopeResolver>,
}

pub trait ScopeResolver: Send + Sync {
    /// Returns the scope (tenant) for the current request context.
    fn current_scope(&self) -> Option<ScopeLevel>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeLevel {
    /// Global scope — admin access to all credentials.
    Global,
    /// Tenant scope — access limited to tenant's credentials.
    Tenant(String),
    /// User scope — access limited to user's own credentials.
    User(String),
}
```

**Enforcement:**
- `get()`: load credential, check `metadata.owner_id` against current scope
- `put()`: set `metadata.owner_id` to current scope on create
- `list()`: filter results by current scope
- `delete()`: verify ownership before deleting

**Cross-tenant composition:** blocked by default. AWS Assume Role referencing
another tenant's credential will fail at ScopeLayer during `resolve_credential()`.
Explicit scope configuration required for shared credentials.

---

## AuditLayer

### Access logging (redacted)

```rust
pub struct AuditLayer<S: CredentialStore> {
    inner: S,
    sink: Arc<dyn AuditSink>,
}

pub trait AuditSink: Send + Sync {
    fn log(&self, event: AuditEvent);
}

pub struct AuditEvent {
    pub timestamp: DateTime<Utc>,
    pub credential_id: CredentialId,
    pub operation: AuditOperation,
    pub owner_id: String,
    pub caller_scope: Option<ScopeLevel>,
    pub trace_id: Option<Uuid>,
    pub result: AuditResult,
    // NO credential data — never plaintext
}

pub enum AuditOperation {
    Get,
    Put { mode: PutMode },
    Delete,
    List,
    Exists,
}

pub enum AuditResult {
    Success,
    NotFound,
    ScopeViolation,
    Conflict { expected: u64, actual: u64 },
    Error(String),  // sanitized error message, no secrets
}
```

**AuditLayer NEVER sees or logs:**
- `StoredCredential.data` (ciphertext)
- Plaintext credential state
- Secret fields from any type

**AuditLayer logs:**
- Who (owner_id, scope)
- What (credential_id, operation, PutMode)
- When (timestamp)
- Result (success/failure, sanitized)
- Correlation (trace_id)

---

## Storage Backends

### InMemoryStore (tests only)

```rust
pub struct InMemoryStore {
    data: DashMap<CredentialId, StoredCredential>,
}
```

Used for unit tests and ephemeral environments. Supports all PutMode variants
(CreateOnly, Overwrite, CompareAndSwap). No persistence across restarts.

### LocalFileStore

```rust
pub struct LocalFileStore {
    base_dir: PathBuf,
}
```

**Storage layout:**
```
{base_dir}/
  credentials/
    {credential_id}.json    // StoredCredential JSON (data field = ciphertext)
  pending/                  // PendingStateStore (if shared backend)
    {token_id}.json
```

**Atomicity:** Write to temp file → `rename()` (atomic on POSIX).
**CAS:** Read version from file, compare, write if match. File-lock for
concurrent access within same process.

**Use case:** Desktop app, single-node development, CLI tools.

### PostgresStore

```rust
pub struct PostgresStore {
    pool: sqlx::PgPool,
}
```

**Schema:**
```sql
CREATE TABLE credentials (
    id VARCHAR(255) PRIMARY KEY,
    state_kind VARCHAR(100) NOT NULL,
    scheme_kind VARCHAR(100) NOT NULL,
    data BYTEA NOT NULL,                  -- ciphertext
    owner_id VARCHAR(255) NOT NULL,
    lifecycle VARCHAR(20) NOT NULL DEFAULT 'Active',
    version BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_credentials_owner ON credentials(owner_id);
CREATE INDEX idx_credentials_scheme ON credentials(scheme_kind);
```

**CAS implementation:**
```sql
-- CompareAndSwap
UPDATE credentials
SET data = $1, version = version + 1, updated_at = now(), lifecycle = $2
WHERE id = $3 AND version = $4
RETURNING *;
-- If 0 rows updated → StoreError::Conflict
```

**Use case:** Production multi-node deployment.

### VaultStore (HashiCorp Vault)

```rust
pub struct VaultStore {
    client: vault::Client,
    mount: String,     // "secret" (KV v2)
}
```

**Mapping:** credential_id → Vault KV path `{mount}/data/{credential_id}`.
Version → Vault KV version. CAS via `cas` parameter on write.

**Note:** Double encryption — EncryptionLayer encrypts, then Vault encrypts
at rest. Redundant but defense-in-depth. Optional: skip EncryptionLayer
for Vault backend (Vault handles encryption).

### AwsSecretsStore / K8sSecretsStore

Similar pattern — map credential_id to AWS Secrets Manager secret or
Kubernetes Secret resource. CAS via version metadata.

---

## PendingStateStore — Ephemeral Interactive State

### Trait recap (from core HLD)

```rust
pub trait PendingStateStore: Send + Sync {
    fn put<P: PendingState>(
        &self, credential_kind: &str, owner_id: &str, session_id: &str, pending: P,
    ) -> impl Future<Output = Result<PendingToken, CredentialError>> + Send;

    fn get<P: PendingState>(
        &self, token: &PendingToken,
    ) -> impl Future<Output = Result<P, CredentialError>> + Send;

    fn consume<P: PendingState>(
        &self, credential_kind: &str, token: &PendingToken,
        owner_id: &str, session_id: &str,
    ) -> impl Future<Output = Result<P, CredentialError>> + Send;

    fn delete(
        &self, token: &PendingToken,
    ) -> impl Future<Output = Result<(), CredentialError>> + Send;
}
```

### Storage key: 4-dimensional binding

```
Key = (credential_kind, owner_id, session_id, token_id)
```

| Dimension | Prevents |
|-----------|----------|
| credential_kind | Type confusion (credential A reading credential B's pending state) |
| owner_id | Cross-user token replay |
| session_id | Session fixation / confused deputy |
| token_id | Token guessing (32-byte CSPRNG, 2^256 search space) |

All four validated on `consume()`. Mismatch → error, no data returned.

### PendingStateStore format

```rust
struct PendingEntry {
    credential_kind: String,
    owner_id: String,
    session_id: String,
    data: Vec<u8>,                  // ciphertext (encrypted PendingState JSON)
    expires_at: DateTime<Utc>,      // TTL enforcement
    consumed: bool,                 // single-use flag
}
```

### InMemoryPendingStore (dev/single-node)

```rust
pub struct InMemoryPendingStore {
    entries: DashMap<String, PendingEntry>,  // key = token_id
    encryption_key: Arc<EncryptionKey>,
}
```

**TTL enforcement:** Background task runs every 30s, removes expired entries.
**Single-use:** `consume()` sets `consumed = true` and removes entry atomically.
**Encryption:** Same AES-256-GCM as CredentialStore. Serialization buffer
wrapped in `Zeroizing<Vec<u8>>`.

**Limitation:** In-memory store WILL NOT WORK for multi-node. OAuth2/SAML
callbacks may route to a different node than the one that initiated resolve().

### RedisPendingStore (multi-node/HA)

```rust
pub struct RedisPendingStore {
    client: redis::Client,
    encryption_key: Arc<EncryptionKey>,
    key_prefix: String,
}
```

**Redis key:** `{prefix}:pending:{token_id}`
**Redis value:** JSON `PendingEntry` (data field = ciphertext)
**TTL:** Redis `EXPIRE` command — automatic cleanup
**Single-use consume:** `GET + DEL` in Redis transaction (MULTI/EXEC) or Lua script:

```lua
local entry = redis.call('GET', KEYS[1])
if entry then
    redis.call('DEL', KEYS[1])
    return entry
else
    return nil
end
```

**4-dimensional validation:** After GET, parse PendingEntry and validate
credential_kind, owner_id, session_id before returning data. If mismatch →
re-insert (not consumed) and return error.

### PostgresPendingStore (multi-node, shared with CredentialStore DB)

```sql
CREATE TABLE pending_states (
    token_id VARCHAR(64) PRIMARY KEY,
    credential_kind VARCHAR(100) NOT NULL,
    owner_id VARCHAR(255) NOT NULL,
    session_id VARCHAR(255) NOT NULL,
    data BYTEA NOT NULL,                  -- ciphertext
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_pending_expires ON pending_states(expires_at);
```

**Consume:** `DELETE FROM pending_states WHERE token_id = $1 RETURNING *`
**TTL cleanup:** Background job: `DELETE FROM pending_states WHERE expires_at < now()`

---

## Migration Strategy

### CredentialState versioning

Every `CredentialState` has `const VERSION: u16`. `StoredCredential.state_version`
(defined above) carries the version that was used to serialize the state.

**Migration on read:**
1. Load StoredCredential from backend
2. Decrypt data → JSON
3. Check `state_version` vs current `CredentialState::VERSION`
4. If mismatch → run migration function
5. Re-serialize + re-encrypt + write back

```rust
pub trait StateMigration: Send + Sync {
    fn migrate(&self, from_version: u16, data: serde_json::Value) -> Result<serde_json::Value, StoreError>;
}
```

**Migration registry:** maps `(state_kind, from_version, to_version)` → migration function.
Migrations are sequential: v1 → v2 → v3 (no skip).

### Encryption key rotation

See EncryptionLayer section. Background re-encryption job reads all
credentials, decrypts with old key, encrypts with new key, writes back via CAS.

---

## Configuration

```rust
pub struct StorageConfig {
    pub backend: BackendConfig,
    pub cache: CacheConfig,
    pub encryption: EncryptionConfig,
    pub pending: PendingStoreConfig,
}

pub enum BackendConfig {
    InMemory,
    LocalFile { path: PathBuf },
    Postgres { url: String, pool_size: u32 },
    Vault { address: String, token: SecretString, mount: String },
    AwsSecrets { region: String },
    K8sSecrets { namespace: String },
}

pub struct EncryptionConfig {
    pub master_key_source: KeySource,
}

pub enum KeySource {
    /// Direct key (dev only — key in env var or config file)
    Direct(SecretString),
    /// AWS KMS key ARN
    AwsKms(String),
    /// Vault Transit engine
    VaultTransit { address: String, key_name: String },
    /// File on disk (chmod 600)
    File(PathBuf),
}

pub enum PendingStoreConfig {
    InMemory,
    Redis { url: String },
    Postgres { url: String },
}
```
