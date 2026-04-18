# 22 — Credential system v3

> **Status:** DRAFT
> **Authority:** subordinate to `docs/PRODUCT_CANON.md`. Canon wins on conflict.
> **Parent:** [`./README.md`](./README.md)
> **Replaces parts of:** `crates/credential/` (keeps core; relocates storage, rotation, retry, executor)
> **Depends on:** `nebula-core`, `nebula-error`, `nebula-schema` (spec 21), `nebula-resilience`, `nebula-validator`, `nebula-eventbus`, `nebula-telemetry`, `nebula-metrics`
> **Consumers:** `nebula-action` (credential injection), `nebula-resource` (Resource credential binding), `nebula-engine` (resolution runtime), `nebula-plugin-sdk` (plugin credentials), `apps/*` (UI, CLI)
> **Related specs:** 02 (tenancy), 04 (RBAC), 16 (storage schema), 17 (multi-process coord), 19 (error taxonomy), 21 (schema crate)
> **Research references:** HashiCorp Vault dynamic secrets, envelope encryption (AWS/GCP KMS), SPIFFE/SPIRE, OIDC workload identity federation, OAuth2 token rotation best practices, Rust ecosystem (secrecy, zeroize, vaultrs, kms-aead)

## 1. Problem

The existing `nebula-credential` crate (~20 800 LOC) is impressively thorough
but accumulated cross-cutting responsibilities that belong elsewhere and is
missing capabilities modern credential systems are expected to provide.

### 1.1 Responsibility bleed

The crate currently owns concerns that logically belong to other crates:

| Concern | Current location | Should live in |
|---|---|---|
| Credential **persistence** (CRUD, layers, migrations) | `credential/store.rs`, `store_memory.rs`, `layer/` | `nebula-storage` (already has `ExecutionRepo`, `WorkflowRepo`) |
| **Pending state** persistence (interactive flows) | `credential/pending_store*.rs` | `nebula-storage` |
| **Rotation** coordinator (ops process, feature-gated) | `credential/rotation/` | `nebula-storage` (operates on store) |
| **Retry** facade (366 lines wrapping `nebula-resilience`) | `credential/retry.rs` | **Deleted** — callers use `nebula-resilience` directly |
| Framework **executor** with timeouts | `credential/executor.rs` | `nebula-engine` (runtime concern) |
| Direct `reqwest` dependency for OAuth2 flows | `credential/Cargo.toml` + `credentials/oauth2*.rs` | Future: `CredentialHttp` trait + `HttpResource` impl |

Result: `nebula-credential` should shrink from ~20 800 to ~10-12K LOC,
focused entirely on credential semantics.

### 1.2 Missing capabilities

Modern credential systems provide features that `nebula-credential` v2 does not:

1. **Envelope encryption** — industry standard for encrypting large volumes
   via cloud KMS. Single master key works for small scale; beyond
   that, DEK-per-credential with KEK in KMS is the norm.

2. **External credential providers** — delegation to Vault, AWS Secrets
   Manager, GCP Secret Manager, Azure Key Vault, Infisical, Doppler,
   platform keyrings. Currently everything is "locally stored, locally
   encrypted".

3. **Dynamic secrets** — secrets generated on-demand with lease TTL
   (Vault-style database credentials). Current system only handles
   user-provided static secrets + OAuth2 refresh.

4. **Workload identity federation (OIDC)** — exchange workload OIDC tokens
   for short-lived cloud credentials (AWS STS, GCP STS, Azure AD). No
   long-lived secrets stored at all.

5. **SPIFFE/SPIRE integration** — CNCF workload identity standard. X.509
   or JWT SVIDs from local SPIRE agent.

6. **Distributed refresh coordination** — current `RefreshCoordinator` is
   in-process only. Multi-process deployment (spec 17) needs coordination
   via Postgres advisory locks to prevent cross-process thundering herd.

7. **Tamper-evident audit log** — current `AuditLayer` logs operations
   but not cryptographically chained. Compliance scenarios need hash-chain
   integrity.

8. **Tenancy scoping** — credentials currently not explicitly scoped to
   organization / workspace (spec 02). RBAC integration (spec 04) is
   ad-hoc via `ScopeResolver`.

9. **Postgres persistence** — only `InMemoryStore` + `InMemoryPendingStore`
   exist. Production needs durable storage (spec 16 has `credentials`
   table defined).

10. **Credential metrics & observability** — no built-in hooks for
    "which credentials used, when, how often". Need integration with
    `nebula-metrics` + `nebula-telemetry` spans.

### 1.3 Dependency on `nebula-parameter`

Credential crate currently depends on `nebula-parameter` for schema
definition in `Credential::parameters()`. With spec 21 migrating to
`nebula-schema`, all credential impls need to update to new API. This
is a beneficial migration — dedicated `SecretField` replaces ad-hoc
`secret: bool` flag, matching credential semantics exactly.

## 2. Decision

### 2.1 Three-crate split

```
nebula-credential/                ~10K LOC (core semantics only)
├── traits (Credential, CredentialState, PendingState, etc.)
├── 12 universal AuthScheme types (re-exported from nebula-core)
├── Built-in Credential implementations (ApiKey, BasicAuth, OAuth2, ...)
├── Runtime types (CredentialContext, CredentialHandle, Guard)
├── Resolution engine (CredentialResolver, Registry)
├── Refresh coordination (RefreshCoordinator — interface + in-process impl)
├── Crypto primitives (AES-GCM wrapper, PKCE helpers, zeroize utilities)
└── Interfaces for storage providers (PluggableProvider trait)

nebula-storage/src/credential/    ~3-4K LOC (NEW submodule)
├── CredentialRepo trait + StoredCredential + PutMode
├── PendingRepo trait
├── InMemoryCredentialRepo, InMemoryPendingRepo
├── PostgresCredentialRepo (new, per spec 16)
├── PostgresPendingRepo (new, for interactive flow resume)
├── Layers:
│   ├── EncryptionLayer (single-key legacy)
│   ├── EnvelopeLayer (NEW — DEK per credential + pluggable KMS)
│   ├── CacheLayer
│   ├── AuditLayer (with optional tamper-evident hash chain)
│   └── ScopeLayer
└── Rotation:
    ├── RotationCoordinator
    ├── GracePeriodConfig
    └── RotationEvent (via nebula-eventbus)

nebula-engine/src/credential/     (NEW submodule)
└── CredentialExecutor (framework executor with timeouts, moved from credential)
```

### 2.2 Delete `retry.rs` facade

`credential/retry.rs` is a 366-line facade over `nebula-resilience::retry`.
Delete entirely. Callers construct `RetryConfig` directly. Credential crate
exports a helper `credential::retry::classify_for_retry(err: &CredentialError) -> RetryAdvice`
that callers feed into `RetryConfig::retry_if`. Everything else uses
`nebula-resilience::retry_with(config, op)`.

### 2.3 New capabilities

1. **Envelope encryption** (EnvelopeLayer) — pluggable `Kms` trait, built-in
   providers for AWS KMS, GCP KMS, Azure Key Vault, and local (age / master-key).
2. **External credential providers** (ExternalProvider trait) — delegate
   credential resolution to Vault / AWS SM / GCP SM / Infisical / Doppler.
3. **Dynamic secrets** (`Credential::DYNAMIC: bool = false`) — credentials
   issued per-use with lease TTL and automatic revocation on lease expiry.
4. **Workload identity** (OIDC federation) — new built-in
   `OidcFederationCredential` exchanging workload OIDC tokens for cloud STS.
5. **Tamper-evident audit** (`AuditLayer::with_hash_chain()`) — each
   audit record includes HMAC of previous record.
6. **Distributed refresh** (`RefreshCoordinator::postgres_advisory()`) —
   cross-process single-flight via `pg_try_advisory_xact_lock`.
7. **SPIFFE/SPIRE** (optional feature `spiffe`) — `SpiffeCredential` backed
   by rust-spiffe workload API client.
8. **Credential metrics** — `nebula-metrics` counters: resolve_total,
   refresh_total, refresh_failed_total, test_total, rotations_total.
9. **Tenancy scoping** — `CredentialContext` carries `OrgId`, `WorkspaceId`,
   `ExecutionId` (aligned with spec 02, 17).
10. **Schema migration** — all built-in credentials use `nebula_schema::Schema`
    and `Field::secret("...")` instead of `Parameter::string("...").secret()`.

### 2.4 What stays the same

The core `Credential` trait (v2 unified design) is **kept as-is**. It is
already excellent:

- Three associated types (`Scheme`, `State`, `Pending`)
- Five capability consts (`INTERACTIVE`, `REFRESHABLE`, `REVOCABLE`, `TESTABLE`, `REFRESH_POLICY`)
- Single method `resolve()` required
- Clean lifecycle (resolve → continue_resolve → test → refresh → revoke)
- 12 universal `AuthScheme` types + open trait
- Framework handles pending state lifecycle (author returns raw state)
- `SecretString` + zeroize + forbid unsafe

Adding `DYNAMIC: bool` const and a couple of lifecycle methods (`lease_release`)
keeps backward compat for existing credentials while enabling dynamic secrets.

## 3. Data model

### 3.1 `Credential` trait (v3)

Minor extensions to v2, backward compatible:

```rust
pub trait Credential: Send + Sync + 'static {
    type Scheme: AuthScheme;
    type State: CredentialState;
    type Pending: PendingState;

    const KEY: &'static str;
    const INTERACTIVE: bool = false;
    const REFRESHABLE: bool = false;
    const REVOCABLE: bool = false;
    const TESTABLE: bool = false;
    const DYNAMIC: bool = false;                       // NEW — dynamic/JIT secret
    const REFRESH_POLICY: RefreshPolicy = RefreshPolicy::DEFAULT;

    fn description() -> CredentialDescription where Self: Sized;
    fn input_schema() -> nebula_schema::Schema where Self: Sized;   // was: parameters() -> ParameterCollection

    fn project(state: &Self::State) -> Self::Scheme where Self: Sized;

    async fn resolve(
        values: &nebula_schema::FieldValues,           // was: &ParameterValues
        ctx: &CredentialContext,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, CredentialError>
    where Self: Sized;

    async fn continue_resolve(
        _pending: &Self::Pending,
        _input: &UserInput,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<Self::State, Self::Pending>, CredentialError>
    where Self: Sized {
        Err(CredentialError::NotInteractive)
    }

    async fn test(
        _scheme: &Self::Scheme,
        _ctx: &CredentialContext,
    ) -> Result<Option<TestResult>, CredentialError>
    where Self: Sized {
        Ok(None)
    }

    async fn refresh(
        _state: &mut Self::State,
        _ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError>
    where Self: Sized {
        Ok(RefreshOutcome::NotSupported)
    }

    async fn revoke(
        _state: &mut Self::State,
        _ctx: &CredentialContext,
    ) -> Result<(), CredentialError>
    where Self: Sized {
        Ok(())
    }

    /// NEW — release a dynamic secret lease. Called automatically by the
    /// framework when `DYNAMIC = true` and the lease TTL expires or the
    /// execution terminates.
    ///
    /// Default: no-op. Override when `DYNAMIC = true`.
    async fn release(
        _state: &mut Self::State,
        _ctx: &CredentialContext,
    ) -> Result<(), CredentialError>
    where Self: Sized {
        Ok(())
    }
}
```

**New const `DYNAMIC`** — marks credentials as dynamically provisioned
(Vault database credentials, AWS STS AssumeRole, GCP OIDC exchange, etc.).
Framework treats them specially:

- Never cached (always fresh per execution)
- Lease TTL tracked, `release()` called on execution end
- Telemetry marks as "ephemeral"

**New method `release()`** — invoked when dynamic credential lease expires.
Default no-op for static credentials.

**Schema migration**: `parameters() -> ParameterCollection` becomes
`input_schema() -> nebula_schema::Schema`. `ParameterValues` becomes
`FieldValues`.

### 3.2 `CredentialContext` — enriched

```rust
pub struct CredentialContext {
    // Identity
    pub principal: Principal,                // who's resolving (user / service account / workflow)

    // Tenancy (from spec 02)
    pub org_id: Option<OrgId>,
    pub workspace_id: Option<WorkspaceId>,

    // Execution context (from spec 17)
    pub execution_id: Option<ExecutionId>,
    pub node_id: Option<NodeId>,
    pub attempt_id: Option<AttemptId>,

    // Observability (from spec 18)
    pub trace_id: Option<TraceId>,
    pub request_id: Option<RequestId>,

    // Runtime services (injected by engine)
    refresh_coordinator: Arc<dyn RefreshCoordinator>,
    // Future: http: Arc<dyn CredentialHttp> (deferred — current design still uses reqwest)

    // Metrics emit target
    metrics: Arc<CredentialMetrics>,

    // Clock for deterministic testing (nebula-testing TestClock)
    clock: Arc<dyn Clock>,
}

impl CredentialContext {
    pub fn principal(&self) -> &Principal { &self.principal }
    pub fn workspace_id(&self) -> Option<&WorkspaceId> { self.workspace_id.as_ref() }
    pub fn trace_id(&self) -> Option<&TraceId> { self.trace_id.as_ref() }
    pub fn refresh_coordinator(&self) -> &dyn RefreshCoordinator {
        self.refresh_coordinator.as_ref()
    }
    pub fn metrics(&self) -> &CredentialMetrics { &self.metrics }
    pub fn now(&self) -> chrono::DateTime<chrono::Utc> { self.clock.now() }
}
```

**Rationale**: today `CredentialContext` only has `user_id`. v3 carries
everything credential logic might need:

- **Tenancy** — credential resolution must respect workspace boundaries
- **Execution context** — dynamic secrets are scoped per-execution
- **Observability** — automatic trace linking + metrics
- **Refresh coordinator** — injected so credentials don't hardcode in-process vs distributed
- **Clock** — testability via `nebula-testing::TestClock`

### 3.3 `Principal`

```rust
pub enum Principal {
    User(UserId),
    ServiceAccount(ServiceAccountId),
    Workflow { workflow_id: WorkflowId, trigger: TriggerRef },
    System,                                  // internal operations (migration, rotation)
}
```

Used by audit log and RBAC checks. From spec 04.

### 3.4 `RefreshCoordinator` — interface + implementations

```rust
/// Single-flight refresh coordination.
///
/// Prevents thundering herd when multiple concurrent callers want to
/// refresh the same credential.
#[async_trait]
pub trait RefreshCoordinator: Send + Sync {
    /// Acquire exclusive refresh right for the given credential ID.
    /// If another caller is currently refreshing, this blocks until
    /// that refresh completes (success or failure) and returns without
    /// taking the lock.
    async fn acquire_refresh(
        &self,
        credential_id: &CredentialId,
    ) -> Result<RefreshToken, RefreshCoordinatorError>;

    /// Release refresh right. Called on token drop.
    async fn release_refresh(&self, token: RefreshToken);
}

/// In-process implementation — single engine instance.
pub struct InProcessRefreshCoordinator {
    locks: Mutex<HashMap<CredentialId, Arc<Mutex<()>>>>,
}

/// Distributed implementation — uses Postgres advisory locks.
///
/// Multi-process safe. Per spec 17, all engine instances share one
/// Postgres connection pool; advisory locks serialize refresh across
/// processes.
pub struct PostgresRefreshCoordinator {
    pool: sqlx::PgPool,
}
```

**Selection at runtime**: engine constructs one based on deployment mode.
`apps/cli` uses `InProcessRefreshCoordinator`. Multi-process server uses
`PostgresRefreshCoordinator`.

### 3.5 `CredentialStore` → `CredentialRepo` (in `nebula-storage`)

Moves to `nebula-storage::credential::repo`:

```rust
// nebula-storage/src/credential/repo.rs

#[derive(Debug, Clone)]
pub struct StoredCredential {
    pub id: CredentialId,
    pub credential_key: String,               // Credential::KEY
    pub org_id: Option<OrgId>,
    pub workspace_id: Option<WorkspaceId>,
    pub data: Vec<u8>,                        // encrypted payload
    pub envelope: Option<EnvelopeMetadata>,   // NEW — per-credential DEK info
    pub state_kind: String,                   // CredentialState::KIND
    pub state_version: u32,
    pub version: u64,                         // CAS counter
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub lease_id: Option<String>,             // NEW — dynamic secret lease
    pub metadata: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvelopeMetadata {
    pub kek_id: String,                       // KEK identifier in KMS
    pub encrypted_dek: Vec<u8>,               // DEK encrypted under KEK
    pub algorithm: EncryptionAlgorithm,       // AES-256-GCM
    pub nonce: [u8; 12],
    pub aad_digest: [u8; 32],                 // SHA-256 of AAD for tamper check
}

#[derive(Debug, Clone, Copy)]
pub enum PutMode {
    CreateOnly,
    Overwrite,
    CompareAndSwap { expected_version: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum CredentialRepoError {
    #[error("credential not found: {id}")]
    NotFound { id: CredentialId },

    #[error("version conflict for {id}: expected {expected}, got {actual}")]
    VersionConflict { id: CredentialId, expected: u64, actual: u64 },

    #[error("credential already exists: {id}")]
    AlreadyExists { id: CredentialId },

    #[error("scope denied: {reason}")]
    ScopeDenied { reason: String },

    #[error("backend error: {0}")]
    Backend(Box<dyn std::error::Error + Send + Sync>),
}

#[async_trait]
pub trait CredentialRepo: Send + Sync {
    async fn get(&self, id: &CredentialId) -> Result<StoredCredential, CredentialRepoError>;

    async fn put(
        &self,
        credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, CredentialRepoError>;

    async fn delete(&self, id: &CredentialId) -> Result<(), CredentialRepoError>;

    async fn list(
        &self,
        scope: ListScope,
    ) -> Result<Vec<StoredCredentialSummary>, CredentialRepoError>;

    async fn exists(&self, id: &CredentialId) -> Result<bool, CredentialRepoError>;
}

pub struct ListScope {
    pub org_id: Option<OrgId>,
    pub workspace_id: Option<WorkspaceId>,
    pub state_kind: Option<String>,
    pub limit: Option<u32>,
}
```

Changes from v2 `CredentialStore`:

- `id: String` → `id: CredentialId` (typed newtype, prefixed ULID `cred_01J...`)
- Added `org_id` / `workspace_id` (tenancy)
- Added `envelope: Option<EnvelopeMetadata>` (envelope encryption)
- Added `lease_id` (dynamic secrets)
- Renamed error types
- `list()` now takes `ListScope` for tenancy filtering

### 3.6 Envelope encryption — `Kms` trait + layers

```rust
// nebula-storage/src/credential/kms.rs

#[async_trait]
pub trait Kms: Send + Sync {
    /// Generate a new Data Encryption Key encrypted under the current KEK.
    /// Returns plaintext DEK (to use immediately) + ciphertext DEK (to store).
    async fn generate_dek(
        &self,
        aad: &[u8],
    ) -> Result<GeneratedDek, KmsError>;

    /// Decrypt an existing DEK.
    async fn decrypt_dek(
        &self,
        ciphertext: &[u8],
        aad: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>, KmsError>;

    /// Identifier of the key used by this KMS (for envelope metadata).
    fn kek_id(&self) -> &str;

    /// Whether this KMS supports key rotation (rewrap).
    fn supports_rotation(&self) -> bool;

    /// Rewrap an existing encrypted DEK under a newer KEK version.
    /// Default: decrypt then re-encrypt via `generate_dek`.
    async fn rewrap(&self, ciphertext: &[u8], aad: &[u8]) -> Result<Vec<u8>, KmsError>;
}

pub struct GeneratedDek {
    pub plaintext: Zeroizing<Vec<u8>>,        // 32 bytes for AES-256
    pub ciphertext: Vec<u8>,                  // encrypted under KEK
}

#[derive(Debug, thiserror::Error)]
pub enum KmsError {
    #[error("KMS API error: {0}")]
    Api(String),
    #[error("key not found: {0}")]
    KeyNotFound(String),
    #[error("access denied")]
    AccessDenied,
    #[error("wrapped DEK corrupt")]
    Corrupt,
    #[error("transport error: {0}")]
    Transport(Box<dyn std::error::Error + Send + Sync>),
}
```

**Built-in KMS implementations**:

```rust
// nebula-storage/src/credential/kms/local.rs
/// Local KMS — uses a single master key (from file / env var / keyring).
/// For development, self-host without cloud KMS. Not recommended for
/// multi-tenant production.
pub struct LocalKms {
    master_key: Arc<Zeroizing<[u8; 32]>>,
    kek_id: String,                           // e.g. "local-v1"
}

// nebula-storage/src/credential/kms/aws.rs  (feature = "kms-aws")
/// AWS KMS envelope encryption via aws-sdk-kms.
pub struct AwsKms {
    client: aws_sdk_kms::Client,
    key_arn: String,
    kek_id: String,                           // key ARN as identifier
}

// nebula-storage/src/credential/kms/gcp.rs  (feature = "kms-gcp")
pub struct GcpKms { /* ... */ }

// nebula-storage/src/credential/kms/azure.rs  (feature = "kms-azure")
pub struct AzureKms { /* ... */ }

// nebula-storage/src/credential/kms/vault.rs  (feature = "kms-vault")
/// HashiCorp Vault Transit engine.
pub struct VaultTransitKms { /* ... */ }
```

### 3.7 `EnvelopeLayer` — wraps any `CredentialRepo`

```rust
// nebula-storage/src/credential/layer/envelope.rs

/// Envelope encryption layer — fresh DEK per credential record,
/// DEK wrapped by KMS-managed KEK.
///
/// On write:
/// 1. Ask KMS for fresh DEK (plaintext + wrapped)
/// 2. Encrypt credential payload with plaintext DEK (AES-256-GCM) + AAD=credential_id
/// 3. Store wrapped DEK in EnvelopeMetadata
/// 4. Store encrypted payload in StoredCredential.data
///
/// On read:
/// 1. Fetch StoredCredential with EnvelopeMetadata
/// 2. Ask KMS to unwrap DEK (using stored ciphertext + AAD)
/// 3. Decrypt payload with plaintext DEK
/// 4. Return plaintext
///
/// KEK rotation:
/// - KMS returns new DEK wrapped under new KEK on next write
/// - On read, stale KEK version is detected via kek_id mismatch → rewrap
/// - Lazy rotation: old records rewrap on first read after KEK rotation
pub struct EnvelopeLayer<R: CredentialRepo> {
    inner: R,
    kms: Arc<dyn Kms>,
    rewrap_on_read: bool,
}

impl<R: CredentialRepo> EnvelopeLayer<R> {
    pub fn new(inner: R, kms: Arc<dyn Kms>) -> Self {
        Self { inner, kms, rewrap_on_read: true }
    }

    pub fn with_rewrap_disabled(mut self) -> Self {
        self.rewrap_on_read = false;
        self
    }
}

#[async_trait]
impl<R: CredentialRepo> CredentialRepo for EnvelopeLayer<R> {
    async fn get(&self, id: &CredentialId) -> Result<StoredCredential, CredentialRepoError> {
        let mut stored = self.inner.get(id).await?;
        let envelope = stored.envelope.as_ref()
            .ok_or(CredentialRepoError::Backend("missing envelope".into()))?;

        // Unwrap DEK
        let dek = self.kms.decrypt_dek(&envelope.encrypted_dek, id.as_bytes()).await?;

        // Decrypt payload with AEAD + AAD binding to credential ID
        let plaintext = aes_gcm_decrypt(&dek, &envelope.nonce, &stored.data, id.as_bytes())?;

        // Lazy KEK rotation
        if self.rewrap_on_read && envelope.kek_id != self.kms.kek_id() {
            let new_envelope = self.wrap_dek(&stored.id, &plaintext).await?;
            // Best-effort rewrap — don't fail read on rewrap error
            let _ = self.inner.put(
                StoredCredential { envelope: Some(new_envelope), ..stored.clone() },
                PutMode::CompareAndSwap { expected_version: stored.version },
            ).await;
        }

        stored.data = plaintext;
        Ok(stored)
    }

    async fn put(
        &self,
        mut credential: StoredCredential,
        mode: PutMode,
    ) -> Result<StoredCredential, CredentialRepoError> {
        let plaintext = credential.data.clone();
        let envelope = self.wrap_dek(&credential.id, &plaintext).await?;
        credential.data = aes_gcm_encrypt(
            &envelope.plaintext_dek,
            &envelope.metadata.nonce,
            &plaintext,
            credential.id.as_bytes(),
        )?;
        credential.envelope = Some(envelope.metadata);
        self.inner.put(credential, mode).await
    }

    // ... delete, list, exists delegate unchanged
}
```

### 3.8 External credential providers

```rust
// nebula-credential/src/provider.rs

/// Trait for external credential sources — Vault, AWS Secrets Manager,
/// GCP Secret Manager, Infisical, Doppler, Keyring, etc.
///
/// Delegates credential *resolution* to an external system instead of
/// storing credentials in our own store.
#[async_trait]
pub trait ExternalProvider: Send + Sync {
    /// Unique identifier of this provider (e.g. "vault", "aws-sm", "infisical").
    fn provider_id(&self) -> &str;

    /// Fetch a credential by external reference.
    ///
    /// `reference` is provider-specific (Vault path, AWS SM ARN, etc).
    async fn fetch(
        &self,
        reference: &ExternalReference,
        ctx: &CredentialContext,
    ) -> Result<FetchedCredential, ExternalProviderError>;

    /// Whether this provider supports push (writing secrets back).
    /// Most providers are read-only for Nebula.
    fn supports_write(&self) -> bool { false }

    /// Optional: write a credential back to the provider.
    async fn store(
        &self,
        _reference: &ExternalReference,
        _value: &FetchedCredential,
        _ctx: &CredentialContext,
    ) -> Result<(), ExternalProviderError> {
        Err(ExternalProviderError::NotSupported)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalReference {
    pub provider_id: String,
    pub path: String,                         // provider-specific path
    pub version: Option<String>,              // optional version (Vault v2 KV, AWS SM)
    pub params: serde_json::Value,            // arbitrary provider params
}

#[derive(Debug, Clone)]
pub struct FetchedCredential {
    pub data: serde_json::Value,              // credential payload as structured data
    pub lease_id: Option<String>,             // for dynamic secrets with lease
    pub lease_duration: Option<Duration>,
    pub renewable: bool,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExternalProviderError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("access denied")]
    AccessDenied,
    #[error("provider operation not supported")]
    NotSupported,
    #[error("provider transport error: {0}")]
    Transport(Box<dyn std::error::Error + Send + Sync>),
    #[error("provider returned invalid data: {0}")]
    InvalidData(String),
}
```

**Built-in providers** (feature-gated in sub-crate `nebula-credential-providers`):

- `VaultProvider` — HashiCorp Vault KV v1/v2 + Database secrets engine (dynamic DB creds)
- `AwsSecretsManagerProvider` — AWS Secrets Manager (static secrets + rotation via AWS)
- `GcpSecretManagerProvider` — GCP Secret Manager
- `AzureKeyVaultProvider` — Azure Key Vault
- `InfisicalProvider` — Infisical (via REST API)
- `DopplerProvider` — Doppler (via REST API)
- `KeyringProvider` — OS keyring via `keyring` crate (macOS Keychain, Windows DPAPI, Linux libsecret)
- `EnvProvider` — environment variables (dev only)

**Resolution flow with external provider**:

```
User setup:
  1. User configures Credential with provider_id = "vault" + path = "secret/data/github"
  2. `Credential::resolve()` stores ExternalReference instead of raw secret
  3. Our store holds only the reference (no actual secret)

Runtime:
  1. Action requests credential via ctx.credential::<GithubCred>()
  2. CredentialResolver loads ExternalReference from store
  3. Delegates to VaultProvider.fetch(reference, ctx)
  4. Vault returns current value (dynamic or static)
  5. Wrap as CredentialGuard, pass to action
  6. On drop: guard zeroizes plaintext
```

Benefit: secrets never live in Nebula's DB, they live in dedicated vaults.
Rotation handled by vault (we just fetch on every use).

### 3.9 `Credential::Reference` — new credential kind

```rust
/// Reference credential — delegates to an external provider.
///
/// Stores an ExternalReference in its state. Resolution fetches via
/// the registered ExternalProvider. Supports dynamic secrets via lease_id.
pub struct ReferenceCredential;

#[derive(Clone, Serialize, Deserialize)]
pub struct ReferenceState {
    pub reference: ExternalReference,
    pub cached_value: Option<CachedSecret>,     // optional in-memory cache
}

impl Credential for ReferenceCredential {
    type Scheme = DynamicAuthMaterial;            // varies — provider returns what it returns
    type State = ReferenceState;
    type Pending = NoPendingState;

    const KEY: &'static str = "external_reference";
    const REFRESHABLE: bool = true;

    // Users fill out provider + path
    fn input_schema() -> Schema {
        Schema::new()
            .add(Field::select("provider_id")
                .label("Provider")
                .options(available_providers())
                .required())
            .add(Field::string("path")
                .label("Reference path")
                .placeholder("secret/data/my-app/github")
                .required())
            .add(Field::string("version")
                .label("Version")
                .placeholder("latest"))
    }

    async fn resolve(
        values: &FieldValues,
        _ctx: &CredentialContext,
    ) -> Result<ResolveResult<ReferenceState, NoPendingState>, CredentialError> {
        let reference = ExternalReference {
            provider_id: values.get_string("provider_id").unwrap().into(),
            path: values.get_string("path").unwrap().into(),
            version: values.get_string("version").map(String::from),
            params: serde_json::Value::Null,
        };
        Ok(ResolveResult::Complete(ReferenceState {
            reference,
            cached_value: None,
        }))
    }

    async fn refresh(
        state: &mut ReferenceState,
        ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        // Framework resolver hooks in here with ExternalProvider registry.
        let provider = ctx.provider_registry().get(&state.reference.provider_id)?;
        let fetched = provider.fetch(&state.reference, ctx).await?;
        state.cached_value = Some(CachedSecret::from(fetched));
        Ok(RefreshOutcome::Refreshed)
    }
}
```

### 3.10 Dynamic secrets — Postgres example

```rust
/// Dynamic Postgres credential — Vault database secrets engine style.
/// Issues a fresh Postgres user per resolution, with TTL-based revocation.
pub struct DynamicPostgresCredential;

#[derive(Clone, Serialize, Deserialize)]
pub struct DynamicPostgresState {
    pub vault_mount: String,
    pub vault_role: String,
    pub lease_id: String,
    pub username: String,
    pub password: SecretString,
    pub expires_at: DateTime<Utc>,
}

impl Credential for DynamicPostgresCredential {
    type Scheme = IdentityPassword;
    type State = DynamicPostgresState;
    type Pending = NoPendingState;

    const KEY: &'static str = "dynamic_postgres";
    const DYNAMIC: bool = true;                   // ← key flag
    const REFRESHABLE: bool = true;
    const REVOCABLE: bool = true;

    fn input_schema() -> Schema {
        Schema::new()
            .add(Field::string("vault_mount").required())
            .add(Field::string("vault_role").required())
    }

    async fn resolve(
        values: &FieldValues,
        ctx: &CredentialContext,
    ) -> Result<ResolveResult<DynamicPostgresState, NoPendingState>, CredentialError> {
        let vault = ctx.provider_registry().get("vault")?;
        let resp = vault.fetch(
            &ExternalReference {
                provider_id: "vault".into(),
                path: format!("{}/creds/{}",
                    values.get_string("vault_mount").unwrap(),
                    values.get_string("vault_role").unwrap()),
                version: None,
                params: serde_json::Value::Null,
            },
            ctx,
        ).await?;

        let username = resp.data.get("username").and_then(|v| v.as_str()).ok_or_else(
            || CredentialError::Provider("missing username".into())
        )?;
        let password = resp.data.get("password").and_then(|v| v.as_str()).ok_or_else(
            || CredentialError::Provider("missing password".into())
        )?;

        Ok(ResolveResult::Complete(DynamicPostgresState {
            vault_mount: values.get_string("vault_mount").unwrap().into(),
            vault_role: values.get_string("vault_role").unwrap().into(),
            lease_id: resp.lease_id.unwrap_or_default(),
            username: username.into(),
            password: SecretString::new(password.into()),
            expires_at: Utc::now() + resp.lease_duration.unwrap_or(Duration::from_secs(3600)),
        }))
    }

    async fn release(
        state: &mut DynamicPostgresState,
        ctx: &CredentialContext,
    ) -> Result<(), CredentialError> {
        let vault = ctx.provider_registry().get("vault")?;
        // Tell Vault to revoke the lease — user dropped per spec
        vault.revoke_lease(&state.lease_id, ctx).await?;
        Ok(())
    }
}
```

### 3.11 OIDC Workload Identity Federation

```rust
/// Workload Identity Federation credential — exchanges OIDC token from
/// the workload runtime (K8s service account, GitHub Actions, etc.) for
/// a short-lived cloud credential (AWS STS, GCP SA token, Azure AD).
pub struct OidcFederationCredential;

#[derive(Clone, Serialize, Deserialize)]
pub struct OidcFederationState {
    pub target_cloud: OidcTarget,
    pub role_arn_or_sa: String,
    pub session_name: String,
    pub subject_token_type: String,
    pub access_token: SecretString,
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub enum OidcTarget {
    Aws { region: String, role_arn: String },
    Gcp { workload_provider: String, service_account: String },
    Azure { tenant_id: String, client_id: String },
}

impl Credential for OidcFederationCredential {
    type Scheme = SecretToken;                    // bearer token (AWS / GCP / Azure)
    type State = OidcFederationState;
    type Pending = NoPendingState;

    const KEY: &'static str = "oidc_federation";
    const DYNAMIC: bool = true;
    const REFRESHABLE: bool = true;

    fn input_schema() -> Schema {
        Schema::new()
            .add(Field::select("target_cloud")
                .options(&[
                    ("aws", "AWS"),
                    ("gcp", "GCP"),
                    ("azure", "Azure"),
                ])
                .required())
            .add(Field::string("role_arn_or_sa").required())
            .add(Field::string("session_name").default(json!("nebula-workflow")))
    }

    async fn resolve(
        values: &FieldValues,
        ctx: &CredentialContext,
    ) -> Result<ResolveResult<OidcFederationState, NoPendingState>, CredentialError> {
        // 1. Get OIDC token from workload runtime (K8s projected token / GHA env var / etc.)
        let workload_token = fetch_workload_oidc_token().await?;

        // 2. Exchange via cloud STS
        let target = match values.get_string("target_cloud").unwrap() {
            "aws" => OidcTarget::Aws { /* ... */ },
            "gcp" => OidcTarget::Gcp { /* ... */ },
            "azure" => OidcTarget::Azure { /* ... */ },
            _ => return Err(CredentialError::Validation("unsupported cloud".into())),
        };

        let exchange = exchange_oidc_for_sts(&target, &workload_token, ctx).await?;

        Ok(ResolveResult::Complete(OidcFederationState {
            target_cloud: target,
            role_arn_or_sa: values.get_string("role_arn_or_sa").unwrap().into(),
            session_name: values.get_string("session_name").unwrap_or("nebula").into(),
            subject_token_type: "urn:ietf:params:oauth:token-type:jwt".into(),
            access_token: exchange.access_token,
            expires_at: exchange.expires_at,
        }))
    }

    async fn refresh(
        state: &mut OidcFederationState,
        ctx: &CredentialContext,
    ) -> Result<RefreshOutcome, CredentialError> {
        // Re-exchange: fetch fresh workload token, re-exchange via STS.
        let workload_token = fetch_workload_oidc_token().await?;
        let exchange = exchange_oidc_for_sts(&state.target_cloud, &workload_token, ctx).await?;
        state.access_token = exchange.access_token;
        state.expires_at = exchange.expires_at;
        Ok(RefreshOutcome::Refreshed)
    }
}
```

**Benefit**: zero long-lived secrets stored. Workload proves identity
cryptographically (K8s SA token signed by K8s OIDC issuer). Cloud STS
exchanges for short-lived credentials. Nothing to rotate, nothing to leak.

### 3.12 Tamper-evident audit log

```rust
// nebula-storage/src/credential/layer/audit.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub seq: u64,                             // monotonic sequence
    pub timestamp: DateTime<Utc>,
    pub principal: Principal,
    pub operation: AuditOperation,
    pub credential_id: CredentialId,
    pub org_id: Option<OrgId>,
    pub workspace_id: Option<WorkspaceId>,
    pub result: AuditResult,
    pub trace_id: Option<TraceId>,

    // Tamper-evident chain fields (computed, not user-set):
    pub prev_hmac: [u8; 32],                  // HMAC of previous record
    pub self_hmac: [u8; 32],                  // HMAC over this record + prev_hmac
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditOperation {
    Created,
    Resolved,                                  // successful resolution
    ResolveFailed { reason: String },
    Refreshed,
    RefreshFailed { reason: String },
    Tested { passed: bool },
    Revoked,
    Rotated { old_version: u64, new_version: u64 },
    Deleted,
    Accessed,                                  // cache hit (no actual resolve)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AuditResult {
    Success,
    Failure { error_code: String, error_class: ErrorClass },
}

#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn write(&self, event: AuditEvent) -> Result<(), AuditError>;

    /// Verify integrity of the entire chain (periodic audit task).
    async fn verify_chain(&self) -> Result<ChainVerification, AuditError>;
}

pub struct ChainVerification {
    pub total_events: u64,
    pub first_tampered_seq: Option<u64>,      // None = intact
    pub last_verified_at: DateTime<Utc>,
}
```

**AuditLayer modes**:

```rust
/// Plain audit — appends events, no chain.
pub struct AuditLayer<R: CredentialRepo> { /* ... */ }

/// Tamper-evident audit — each event references previous via HMAC chain.
/// Uses a dedicated audit HMAC key (separate from payload encryption key).
pub struct TamperEvidentAuditLayer<R: CredentialRepo> {
    inner: R,
    sink: Arc<dyn AuditSink>,
    hmac_key: Arc<Zeroizing<[u8; 32]>>,
    last_hmac: Mutex<[u8; 32]>,                // in-memory chain tail
}

impl<R: CredentialRepo> TamperEvidentAuditLayer<R> {
    async fn append_event(&self, mut event: AuditEvent) -> Result<(), AuditError> {
        let mut chain = self.last_hmac.lock().await;
        event.prev_hmac = *chain;
        event.self_hmac = compute_hmac(&self.hmac_key, &event);
        *chain = event.self_hmac;
        drop(chain);

        self.sink.write(event).await
    }
}
```

**Verification**: operator runs `nebula credential audit verify` which
walks the chain from seq=0, recomputes HMACs, reports first broken link.

### 3.13 `nebula-credential` — simplified module layout

```
crates/credential/src/
├── lib.rs
├── credential.rs             (Credential trait — v3)
├── credentials/              (built-in impls)
│   ├── mod.rs
│   ├── api_key.rs            (uses nebula_schema::Field::secret())
│   ├── basic_auth.rs
│   ├── oauth2.rs             (still uses reqwest — CredentialHttp deferred)
│   ├── oauth2_config.rs
│   ├── oauth2_flow.rs
│   ├── oidc_federation.rs    (NEW — workload identity)
│   ├── reference.rs          (NEW — external provider delegation)
│   └── dynamic_postgres.rs   (NEW — Vault DB dynamic example)
├── scheme/                   (12 universal schemes, unchanged)
├── state.rs
├── pending.rs                (types only, no store)
├── description.rs            (uses nebula_schema::Schema)
├── metadata.rs
├── snapshot.rs
├── resolve.rs                (ResolveResult, types)
├── context.rs                (CredentialContext with Principal + tenancy + clock)
├── accessor.rs               (Accessor trait)
├── access_error.rs
├── any.rs
├── handle.rs                 (CredentialHandle)
├── guard.rs                  (CredentialGuard with zeroize)
├── key.rs                    (CredentialId newtype — prefixed ULID "cred_01J...")
├── error.rs
├── crypto.rs                 (AES-GCM primitives, PKCE, zeroize helpers)
├── resolver.rs               (CredentialResolver runtime engine)
├── refresh.rs                (RefreshCoordinator trait + InProcess impl)
├── registry.rs               (CredentialRegistry — type-erased)
├── provider.rs               (NEW — ExternalProvider trait)
├── provider_registry.rs      (NEW — provider registry)
├── metrics.rs                (NEW — CredentialMetrics counters)
├── static_protocol.rs
└── macros/                   (#[derive(Credential)], #[derive(AuthScheme)])
```

**Removed from crate** (moved elsewhere):

- `store.rs`, `store_memory.rs` → `nebula-storage/src/credential/repo.rs`
- `pending_store.rs`, `pending_store_memory.rs` → `nebula-storage/src/credential/pending_repo.rs`
- `layer/` → `nebula-storage/src/credential/layer/`
- `rotation/` → `nebula-storage/src/credential/rotation.rs`
- `executor.rs` → `nebula-engine/src/credential_executor.rs`
- `retry.rs` → **deleted** (callers use `nebula-resilience` directly)

Projected size: ~10-12K LOC (was ~20.8K). Focus entirely on credential
semantics and resolution.

### 3.14 `CredentialId` newtype

```rust
/// Typed credential identifier. Prefixed ULID "cred_01J..."
///
/// Aligned with spec 06 — prefixed ULID via `nebula_core::domain_key`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CredentialId(DomainKey<CredentialIdMarker>);

pub struct CredentialIdMarker;
impl KeyMarker for CredentialIdMarker {
    const PREFIX: &'static str = "cred";
}

impl CredentialId {
    pub fn generate() -> Self { Self(DomainKey::generate()) }
    pub fn as_bytes(&self) -> &[u8] { self.0.as_bytes() }
    pub fn as_str(&self) -> &str { self.0.as_str() }
}
```

Consistent with other IDs from spec 06.

### 3.15 `CredentialMetrics`

```rust
pub struct CredentialMetrics {
    pub resolve_total: Counter,               // label: credential_key, result (ok/err/cached)
    pub resolve_duration: Histogram,          // seconds
    pub refresh_total: Counter,               // label: credential_key, outcome
    pub refresh_failed_total: Counter,        // label: credential_key, error_kind
    pub test_total: Counter,
    pub rotation_total: Counter,
    pub dynamic_lease_issued_total: Counter,
    pub dynamic_lease_expired_total: Counter,
    pub tamper_detection_total: Counter,      // label: chain_id
}

impl CredentialMetrics {
    pub fn new(registry: &nebula_metrics::Registry) -> Self {
        Self {
            resolve_total: registry.counter(
                "credential_resolve_total",
                "Total credential resolution attempts",
                &["credential_key", "result"],
            ),
            // ... etc
        }
    }
}
```

## 4. Flows

### 4.1 Static credential resolution (API key, Basic Auth)

```
1. Action code: let token = ctx.credential::<GithubToken>().await?;
2. CredentialResolver::resolve(credential_id, ctx):
   a. Check metrics: resolve_total += 1, start timer
   b. Acquire refresh lock (RefreshCoordinator) — prevents concurrent refresh
   c. Fetch from CredentialRepo (via EnvelopeLayer, decrypted)
   d. If expired: trigger refresh() path (see 4.2)
   e. Project state → scheme
   f. Wrap in CredentialGuard (zeroize on drop)
   g. Release refresh lock
   h. Emit AuditEvent::Resolved
   i. Return handle
3. Action uses token (Guard.expose() inside scope)
4. Action completes → Guard dropped → zeroize
```

### 4.2 OAuth2 refresh flow

```
1. CredentialResolver detects state expires within early_refresh window
2. Acquire refresh lock for credential_id via RefreshCoordinator
   - InProcess: mutex
   - Postgres: pg_try_advisory_xact_lock
3. Re-check state (another caller may have refreshed)
4. Call Credential::refresh(state, ctx)
   - OAuth2Credential makes POST to token_url with refresh_token grant
   - Success → state.access_token updated, expires_at bumped
   - ReauthRequired → state dirty, flow goes to re-authenticate
5. Persist updated state (put with CAS)
6. Emit AuditEvent::Refreshed
7. Release lock
8. Return updated scheme
```

### 4.3 Interactive OAuth2 flow (Authorization Code + PKCE)

```
SETUP (once, via UI):
1. User clicks "Add credential" → fills setup form (input_schema())
2. Framework calls Credential::resolve(values, ctx):
   - Generates PKCE verifier + challenge
   - Constructs authorization URL
   - Returns ResolveResult::Pending { state: OAuth2Pending, interaction: Redirect }
3. Framework (not credential code):
   - Encrypts OAuth2Pending via EnvelopeLayer
   - Generates CSPRNG PendingToken
   - Stores in PendingRepo (feature: InMemory OR Postgres)
   - Returns redirect URL + PendingToken to UI
4. UI redirects browser to provider
5. User authorizes, provider redirects back with code + state
6. Callback handler receives code + state:
   - Loads pending state via PendingToken
   - Calls Credential::continue_resolve(pending, UserInput::Callback { params })
7. Credential::continue_resolve:
   - Validates state parameter (constant-time compare)
   - Exchanges code for tokens (POST to token_url with code_verifier)
   - Returns ResolveResult::Complete(OAuth2State)
8. Framework persists OAuth2State via EnvelopeLayer → CredentialRepo
9. Emit AuditEvent::Created

RESUME ACROSS RESTARTS (new in v3):
- PendingRepo uses PostgresPendingRepo (persistent)
- If engine restarts between steps 3 and 6, pending state still available
- PendingToken TTL enforced (default 10 minutes)
```

### 4.4 Dynamic secret resolution (Vault DB creds)

```
1. Action requests credential: ctx.credential::<DynamicPostgresCred>().await?
2. Resolver sees DYNAMIC = true — skips cache, always fresh
3. Acquire refresh lock
4. Credential::resolve(values, ctx):
   - Delegates to VaultProvider.fetch(ref, ctx)
   - Vault generates new Postgres user with TTL
   - Returns lease_id + username + password
5. Framework:
   - Does NOT persist to CredentialRepo (dynamic secrets are ephemeral)
   - Registers lease with execution lifecycle (drops on execution end)
   - Emit AuditEvent::Resolved (with lease_id)
6. Action uses credentials
7. Execution ends → framework calls Credential::release(state, ctx):
   - VaultProvider.revoke_lease(lease_id)
   - Emit AuditEvent::Deleted
```

### 4.5 OIDC workload identity exchange

```
1. Engine starts (K8s pod, GitHub Actions, etc.)
2. Action requests OidcFederationCredential
3. Credential::resolve fetches OIDC token:
   - K8s: reads /var/run/secrets/kubernetes.io/serviceaccount/token
   - GHA: reads GITHUB_TOKEN env + OIDC claims URL
4. POST to AWS STS AssumeRoleWithWebIdentity / GCP STS
5. Receives short-lived cloud credentials (15-60 min typical)
6. Returns OidcFederationState
7. Framework persists (only the exchanged token, not the OIDC token itself)
8. Refresh at T-5min: repeats the exchange (fresh OIDC token each time)
```

### 4.6 Credential rotation with grace period

```
1. Operator triggers rotation (manual or scheduled):
   nebula credential rotate <credential_id>
2. RotationCoordinator:
   a. Load current state (Secret A)
   b. Generate new Secret B (Credential-specific logic)
   c. Store both in CredentialRepo — new column `previous_version` holds A
   d. Emit AuditEvent::Rotated + CredentialRotationEvent via nebula-eventbus
3. Grace period (GracePeriodConfig.duration, default 1h):
   - CredentialResolver returns Secret B by default
   - But if Secret B fails (caller gets 401), falls back to Secret A
   - Both are valid during window
4. After grace expires:
   - Delete Secret A from CredentialRepo
   - Emit AuditEvent::Deleted
```

### 4.7 External provider fetch (Vault KV)

```
1. User creates ReferenceCredential pointing to Vault path
2. At action runtime:
   - Load ReferenceState from store (contains only the reference)
   - Lookup ExternalProvider::Vault via provider_registry
   - Call VaultProvider.fetch(reference, ctx):
     - Authenticates to Vault (via stored Vault token or OIDC federation)
     - GET /v1/secret/data/{path}
     - Returns FetchedCredential { data: {...}, lease_id: None }
   - Wrap as CredentialGuard
3. If provider returns lease_id → framework registers lease for cleanup
```

## 5. Edge cases

### 5.1 Refresh failure on shared credential

If `RefreshCoordinator` is `PostgresRefreshCoordinator` and refresh fails
after acquiring the advisory lock, the lock is released via `Drop`
(advisory locks are auto-released at txn end). Other waiters unblock
and either re-read a freshly-failed state (and propagate the error) or
retry refresh if their policy permits.

### 5.2 KMS unavailable during read

`EnvelopeLayer::get()` calls `kms.decrypt_dek()`. If KMS is down:

- Return `CredentialRepoError::Backend` wrapping `KmsError::Transport`
- Classified as `Transient` — caller retries via `nebula-resilience`
- Cache layer may still serve stale decrypted data if still in cache

Cache layer is configured with TTL; stale reads during KMS outage are
a trade-off operators configure.

### 5.3 Envelope metadata tampering

`EnvelopeMetadata.aad_digest` stores SHA-256 of AAD used at encryption
time. On read, `EnvelopeLayer` recomputes and compares. Mismatch →
`CredentialRepoError::Backend("envelope AAD tamper detected")`. Hard
error, not transient.

### 5.4 Hash chain replay / tampering

`TamperEvidentAuditLayer::append_event` uses `Mutex<last_hmac>` as chain
tail. If process restarts, chain tail is reloaded from storage via
`AuditSink::tail()`. Verification walks the entire log periodically
(operator-scheduled).

If an attacker inserts or modifies records in the audit store, the HMAC
chain breaks at that point and `verify_chain()` returns the first bad
seq. Alerting fires.

### 5.5 Dynamic secret revoke fails on execution end

`Credential::release()` called in execution teardown can fail (Vault
unreachable, lease already expired). Framework:

- Logs warning
- Emits `AuditEvent::DynamicReleaseFailed`
- Retries asynchronously via `nebula-resilience` policy
- If persistent failure: lease expires naturally in Vault, no manual action

Never blocks execution completion on release failure.

### 5.6 Pending state expired before callback

`PendingRepo` enforces TTL (default 10 min). If user takes too long:

- `continue_resolve` fails with `CredentialError::PendingExpired`
- UI shows "Your session expired, please retry"
- Stale pending record is garbage-collected by periodic job

### 5.7 External provider returns stale data

Dynamic secrets from Vault are always fresh (new credentials per fetch).
But static KV v2 secrets may have multiple versions. `ExternalReference`
carries optional `version` — if `None`, provider returns latest.

If operator updates secret in Vault and wants immediate pickup in
Nebula, they trigger `refresh` on the ReferenceCredential manually or
wait for scheduled refresh.

### 5.8 Cross-workspace credential access denied

`ScopeLayer` enforces workspace isolation at the repo layer:

```rust
impl<R: CredentialRepo> CredentialRepo for ScopeLayer<R> {
    async fn get(&self, id: &CredentialId) -> Result<StoredCredential, CredentialRepoError> {
        let stored = self.inner.get(id).await?;
        if let Some(caller_ws) = self.current_workspace() {
            if stored.workspace_id.as_ref() != Some(&caller_ws) {
                return Err(CredentialRepoError::ScopeDenied {
                    reason: format!("credential {} not accessible from workspace {}",
                        id, caller_ws),
                });
            }
        }
        Ok(stored)
    }
    // ... similar for put, delete, list
}
```

RBAC checks layered on top via caller (engine) before hitting repo.

### 5.9 Zeroization guarantees across async boundaries

All plaintext secrets wrapped in `Zeroizing<Vec<u8>>` or `SecretString`.
`Zeroize` derived on `CredentialState` impls. `Drop` zeroizes memory
on scope exit, including panic paths.

**Async concern**: `tokio::select!` cancelling a future mid-decrypt
might leave partial plaintext on stack. Mitigation: decrypt into
`Zeroizing<Vec<u8>>` allocated on heap, no intermediate stack copies.

### 5.10 Migration of legacy data

Existing nebula-credential v2 data:

- `StoredCredential.id: String` → `CredentialId::parse(&str)`
- `StoredCredential.data` encrypted with v2 single-key EncryptionLayer
- v3 `EnvelopeLayer` detects missing `envelope` field → treats as legacy,
  decrypts with legacy key, re-encrypts with envelope on first read

One-time migration tool: `nebula credential migrate-v2-to-v3 --dry-run`
surveys records, reports count of legacy + envelope, does read+write
pass to convert all in place.

## 6. Configuration surface

### 6.1 `Cargo.toml` features

```toml
[package]
name = "nebula-credential"

[features]
default = []

# Built-in credential types — always available
# (ApiKey, BasicAuth, OAuth2, Reference, OidcFederation)

# Optional integrations (minimal deps by default)
spiffe = ["dep:spiffe"]                         # rust-spiffe workload API
keyring = ["dep:keyring"]                       # OS keyring provider


# ─── nebula-storage credential submodule ─────────────────────────────
# (in nebula-storage/Cargo.toml)
[features]
default = []
credential = []                                 # enable credential repo + layers
postgres-credential = ["postgres", "credential"] # Postgres impl
rotation = ["credential"]                       # rotation coordinator

kms-local = ["credential"]                      # local master key KMS (dev)
kms-aws = ["credential", "dep:aws-sdk-kms"]     # AWS KMS
kms-gcp = ["credential", "dep:google-cloud-kms"]# GCP KMS
kms-azure = ["credential", "dep:azure-security-keyvault"] # Azure Key Vault
kms-vault = ["credential"]                      # HashiCorp Vault Transit


# ─── nebula-credential-providers crate ──────────────────────────────
# (new separate crate for pluggable external providers)
[features]
default = []
vault = ["dep:vaultrs"]
aws-sm = ["dep:aws-sdk-secretsmanager"]
gcp-sm = ["dep:google-cloud-secretmanager"]
azure-kv = ["dep:azure-security-keyvault-secrets"]
infisical = []                                  # REST API only
doppler = []                                    # REST API only
keyring = ["dep:keyring"]
env = []                                        # environment variables (dev)
```

### 6.2 Configuration examples

**`nebula.toml`** — self-host minimal:

```toml
[credential]
store = "postgres"                              # or "memory" for dev
kms = "local"                                   # reads master key from $NEBULA_MASTER_KEY

[credential.rotation]
enabled = true
grace_period = "1h"
schedule = "0 4 * * 0"                          # weekly Sunday 04:00 UTC

[credential.audit]
sink = "postgres"                               # or "eventbus" or "file"
tamper_evident = true
```

**`nebula.toml`** — production with AWS:

```toml
[credential]
store = "postgres"
kms = "aws"

[credential.kms.aws]
key_arn = "arn:aws:kms:us-east-1:123456789012:key/abcd1234-..."
region = "us-east-1"

[credential.providers.vault]
enabled = true
address = "https://vault.example.com:8200"
auth_method = "oidc"                            # workload identity federation
role = "nebula"

[credential.providers.aws-sm]
enabled = true
region = "us-east-1"
# No credentials — uses IAM role via IMDS / IRSA
```

## 7. Testing criteria

### 7.1 Unit tests

- `Credential` trait impl compiles with all 18 built-ins
- `CredentialId` validates prefix, format, error messages
- `EnvelopeLayer`:
  - encrypt → decrypt round-trip
  - AAD mismatch detected
  - KEK rotation lazily rewraps
  - Missing envelope treated as legacy
- `TamperEvidentAuditLayer`:
  - Normal append produces valid chain
  - Modified event detected by `verify_chain()`
  - Deleted event detected (seq gap)
  - Chain tail persisted across restarts
- `RefreshCoordinator::{InProcess, Postgres}`:
  - Single-flight under concurrent load (10 tokio tasks)
  - Failed refresh releases lock (no deadlock)
- `ExternalProvider` mocks:
  - Vault KV v2 success
  - Vault dynamic DB creds with lease
  - Access denied surfaces correctly

### 7.2 Integration tests

**16 credential prototypes** — written against real-world patterns:

1. **GitHub API Key** — ApiKey credential, static, secret field masked
2. **Slack Bot Token** — same pattern, token format validation
3. **OpenAI API Key** — ApiKey with regex pattern check
4. **Postgres (static)** — BasicAuth with connection string
5. **Postgres (Vault dynamic)** — DynamicPostgresCredential via VaultProvider
6. **Google OAuth2** — Authorization Code + PKCE + refresh
7. **GitHub OAuth2** — same flow, device code variant
8. **Microsoft OAuth2** — client credentials grant
9. **AWS (long-lived access key)** — ApiKey variant with key_id + secret
10. **AWS (OIDC federation from K8s)** — OidcFederationCredential → STS
11. **GCP (service account JSON)** — uploaded JSON blob → SecretField multiline
12. **GCP (workload identity federation)** — OIDC exchange
13. **SSH Key (PEM)** — SecretField multiline, PEM format validation
14. **mTLS Certificate** — Certificate scheme with cert + key + CA bundle
15. **HashiCorp Vault (KV v2 reference)** — ReferenceCredential
16. **Infisical (reference)** — ReferenceCredential via InfisicalProvider

Each test:
- Builds credential via derive macro
- Tests input_schema() produces expected nebula-schema Schema
- Runs resolve() with valid values → expected state
- Runs refresh() if applicable
- Tests test() endpoint if applicable
- Verifies audit events emitted

### 7.3 Property tests

- **Envelope encryption** is commutative with any `CredentialRepo`
  backend (encrypt+put+get+decrypt preserves plaintext)
- **Refresh policy** jitter stays within bounds
- **Grace period** math: `now < rotation_time + grace` correctly detects window
- **Hash chain** HMAC computation is deterministic across platforms

### 7.4 Security tests

- `Debug` impls redact all secret fields (panic on leak)
- `SecretString` zeroizes on drop (use `zeroize-test` pattern)
- AAD mismatch test: cross-credential blob swap fails hard
- PKCE verifier validation: altered state parameter detected
- Constant-time comparison used for state tokens
- `#![forbid(unsafe_code)]` enforced

### 7.5 Multi-process tests

- Start two engine instances against same Postgres
- Trigger concurrent refresh of same credential
- Verify exactly one actually calls provider (advisory lock works)
- Verify both receive the same refreshed state

## 8. Performance targets

| Operation | Target | Rationale |
|---|---|---|
| Resolve static credential (cache hit) | < 100 µs | Hot path per action |
| Resolve static credential (cache miss, local KMS) | < 5 ms | Decrypt via local master key |
| Resolve static credential (cache miss, AWS KMS) | < 50 ms | KMS API latency |
| Refresh OAuth2 token | < 500 ms | HTTP round-trip to provider |
| Vault dynamic Postgres creds | < 200 ms | Vault API + DB user creation |
| OIDC STS exchange | < 300 ms | STS API call |
| Envelope encrypt (1 KB plaintext) | < 1 ms | AES-GCM is fast |
| Hash chain append | < 50 µs | HMAC-SHA256 is trivial |
| Verify chain (10 K events) | < 1 s | Linear HMAC walk |

Measured via `criterion` benchmarks in `crates/credential/benches/`.

## 9. Module boundaries

`nebula-credential` is in the **Business layer** per `CLAUDE.md`:

```
Cross-cutting ── nebula-resilience ──┐
                 nebula-eventbus ────┤
                 nebula-telemetry ───┤
                 nebula-metrics ─────┤
                                     ├── nebula-credential (Business)
Core ─── nebula-core ────────────────┤
         nebula-error ───────────────┤
         nebula-schema ──────────────┤
         nebula-validator ───────────┘
```

**Depends on:**

- `nebula-core` — AuthScheme trait, SecretString, CredentialEvent
- `nebula-error` — Classify, NebulaError
- `nebula-schema` — Schema, Field, FieldValues (replaces nebula-parameter)
- `nebula-validator` — Rule for validation inside schemas
- `nebula-resilience` — retry, circuit breaker (replaces local facade)
- `nebula-eventbus` — emit CredentialRotationEvent
- `nebula-telemetry` — span correlation via TraceId
- `nebula-metrics` — CredentialMetrics counters
- `nebula-credential-macros` — derive macros
- Crypto: `aes-gcm`, `argon2`, `zeroize`, `subtle`
- (reqwest still present for OAuth2 — CredentialHttp refactor deferred)

**Does NOT depend on:**

- `nebula-resource` — avoided to prevent business-layer cycles
- `nebula-storage` — storage impls consume credential, not vice versa
- `nebula-engine`, `nebula-runtime`, `nebula-action` — upward deps forbidden

**Consumers** (reverse):

- `nebula-storage::credential::*` — implements CredentialRepo, KMS, etc.
- `nebula-engine::credential_executor` — runtime integration
- `nebula-action` — actions receive credentials via CredentialGuard
- `nebula-resource` — resources receive credentials via Resource::create(cred, ...)
- `nebula-credential-providers` — external providers (new sub-crate)
- `apps/*` — CLI, desktop UI, web dashboard

## 10. Migration path

### 10.1 PR sequence

**PR 0 — this spec** (now)
Add `docs/plans/2026-04-15-arch-specs/22-credential-system.md`. Link from
COMPACT.md and README.md.

**PR 1 — Schema migration (depends on spec 21 PR 1)**
Credential crate switches from `nebula-parameter` to `nebula-schema`:
- `use nebula_parameter::{Parameter, ParameterCollection, ParameterValues}` → `use nebula_schema::{Field, Schema, FieldValues}`
- `Credential::parameters() -> ParameterCollection` → `Credential::input_schema() -> Schema`
- All built-in credentials rewritten with new API
- `secret()` → `Field::secret()`
- CI green

**PR 2 — Extract storage to `nebula-storage`**
Move `store.rs`, `store_memory.rs`, `pending_store*.rs`, `layer/*`, `rotation/` to `nebula-storage::credential::*`:
- Rename `CredentialStore` → `CredentialRepo`
- Rename `CredentialStoreError` → `CredentialRepoError`
- `PendingStateStore` → `PendingRepo`
- `nebula-credential` depends on `nebula-storage` via interface trait (trait stays in credential for consumer use)
- In-memory impls move with traits
- CI green

**PR 3 — Delete `retry.rs` facade**
Callers in `refresh.rs`, `resolver.rs`, `executor.rs` use `nebula-resilience::retry_with` directly. Add `credential::classify_for_retry(err)` helper. Remove `retry.rs`. Remove `credential-internal` retry config types. CI green.

**PR 4 — Extract `executor.rs` to `nebula-engine`**
Move `credential/executor.rs` to `nebula-engine/src/credential_executor.rs`. Update consumers. CI green.

**PR 5 — `CredentialId` newtype**
Replace `String` credential IDs with typed `CredentialId` prefixed ULID. Migrate database schema (spec 16 `credentials` table adds typed ID column). CI green + migration test.

**PR 6 — `CredentialContext` enrichment**
Add `Principal`, `OrgId`, `WorkspaceId`, `ExecutionId`, `trace_id`, `refresh_coordinator`, `metrics`, `clock`. Engine constructs full context. Callers updated. CI green.

**PR 7 — `RefreshCoordinator` distributed variant**
Add `PostgresRefreshCoordinator` using `pg_try_advisory_xact_lock`. Benchmark against in-process variant. Integration test with two engine instances. CI green.

**PR 8 — `EnvelopeLayer` + `Kms` trait + `LocalKms`**
Add envelope encryption layer. `LocalKms` reads master key from env/keyring. Migration of legacy single-key encrypted records via lazy rewrap on read. CI green.

**PR 9 — Cloud KMS providers (feature-gated)**
- `AwsKms` (feature `kms-aws`)
- `GcpKms` (feature `kms-gcp`)
- `AzureKms` (feature `kms-azure`)
- `VaultTransitKms` (feature `kms-vault`)
Each is behind a Cargo feature. CI matrix builds all features.

**PR 10 — `TamperEvidentAuditLayer`**
Hash-chain audit log. `nebula credential audit verify` CLI command. CI green.

**PR 11 — External providers crate scaffold**
New crate `nebula-credential-providers` with trait + `VaultProvider` + `AwsSecretsManagerProvider` + `KeyringProvider` + `EnvProvider`. Feature-gated integrations. CI green.

**PR 12 — `ReferenceCredential` + provider registry**
Built-in `ReferenceCredential` delegates to external providers. Provider registry in `nebula-credential::provider_registry`. Integration test with mock Vault provider. CI green.

**PR 13 — `OidcFederationCredential` + STS exchange**
Built-in OIDC workload identity federation. AWS STS + GCP STS + Azure AD adapters. Integration test with mock OIDC issuer. CI green.

**PR 14 — `DynamicPostgresCredential` + dynamic lifecycle**
`DYNAMIC = true` const on Credential. Framework invokes `release()` on execution end. Integration with Vault DB secrets engine. CI green.

**PR 15 — Canon fold-in**
Update `docs/PRODUCT_CANON.md` §11.9 or new section for credential contract.

### 10.2 Breaking changes

- `CredentialStore` (trait) → `CredentialRepo`
- `CredentialContext::new(user_id)` → richer constructor
- `Credential::parameters()` → `Credential::input_schema()`
- `ParameterCollection` → `Schema` in all credential impls
- `secret()` method on Parameter builders → `Field::secret()`

External API (for credential authors via derive macro) mostly
unchanged; only the schema types differ.

## 11. Open questions

### 11.1 Sub-crate layout for providers

Option A: Single `nebula-credential-providers` crate with features per provider.
Option B: One crate per provider (`nebula-credential-vault`, `nebula-credential-aws-sm`, etc.).

Option A is simpler to discover but forces some dependencies. Option B
is more modular but bloats workspace. **Recommendation**: Option A for v1.

### 11.2 `CredentialHttp` interface (deferred)

Current OAuth2 flows use `reqwest` directly. Spec proposes keeping this
for v3 and deferring `CredentialHttp` trait refactor to a later release.
Rationale: significant refactor, not blocking the v3 improvements listed
here.

### 11.3 Provider authentication itself

How does a provider (e.g. `VaultProvider`) authenticate to Vault? Chicken-
and-egg: Vault token is itself a credential. Options:

- Bootstrap token in `nebula.toml` (file, env var) — simple, insecure
- OIDC federation (workload identity) — preferred when running on K8s / GHA
- Separate Vault-auth-only bootstrap credential in `nebula-credential` — self-reference

**Recommendation**: document bootstrap token pattern first, add OIDC
workload identity for Vault as first-class in PR 13 (same PR as
OidcFederationCredential).

### 11.4 SPIFFE/SPIRE integration

Optional feature in v3 via `rust-spiffe`. New credential type
`SpiffeCredential` produces `X509Svid` or `JwtSvid` scheme. Use cases:
service mesh, zero-trust microservices. **Decision**: defer to future
spec, explicit reference in `nebula-credential` feature list so it
remains a planned feature.

### 11.5 Credential expiration vs cache TTL

Credentials have `expires_at` from provider. Cache layer has its own TTL.
Which wins? **Rule**: `min(credential_expires_at - early_refresh, cache_ttl)`.
Document explicitly; add property test.

### 11.6 Lease renewal vs re-resolution

Vault leases can be renewed (extend TTL without new credentials). Our
`refresh()` does **re-resolution** (new credentials). Some credentials
support both. Add optional `Credential::renew_lease()` method? Or
fold into `refresh()` with optional "extend only" hint? **Recommendation**:
single `refresh()` for now; providers handle "renew vs re-issue" internally.

### 11.7 UI flow for dynamic secrets

Dynamic secrets are issued per-execution. What does the UI show for
"credential setup"? Only the Vault path + role, no actual secret.
UI conveys "This credential is issued on-demand per run". Edge case:
test button must do a real fetch (costs a Vault lease). **Recommendation**:
document clearly in UI; test endpoint issues a lease and immediately
revokes it.

### 11.8 Offline operation

When KMS is unreachable, can credentials be resolved? Currently no —
hard dependency on KMS for envelope decryption. Mitigation: local cache
with TTL allows reads during short KMS outages. Long outages break all
credential access. **Recommendation**: document explicitly; operators
size cache TTL per their SLO.

### 11.9 Multi-region key replication

If Nebula runs in multi-region, credential encryption keys must be
accessible in all regions. Options:

- Replicate master key to all regions (simple, increases attack surface)
- Multi-region KMS (AWS KMS multi-region keys, GCP KMS regional replicas)
- Per-region envelope keys with cross-region sync at credential level

**Decision**: document multi-region KMS as the recommended pattern;
local KMS not supported for multi-region.

## Appendix A — Comparison with other systems

### HashiCorp Vault

- **Strengths**: industry standard, dynamic secrets engine library,
  transit engine, PKI engine, extensive ecosystem
- **Differences**: Vault is a standalone secret store, Nebula embeds
  credential semantics into workflow engine. Nebula's `ReferenceCredential`
  delegates to Vault where it excels.
- **Integration**: Nebula uses Vault as external provider, matching the
  "delegate secrets to Vault" pattern described in Vault's own docs.

Source: [Dynamic secrets for database credential management](https://developer.hashicorp.com/vault/tutorials/db-credentials/database-secrets)

### AWS Secrets Manager

- **Strengths**: native AWS integration, lambda-based rotation, IAM
  access control, automatic rotation schedules
- **Differences**: AWS-specific. Nebula uses it as external provider
  for AWS-hosted deployments.
- **Integration**: `AwsSecretsManagerProvider` in `nebula-credential-providers`.

### Infisical / Doppler

- **Strengths**: modern UX, secret versioning, ephemeral access controls,
  dynamic secrets (beta), free open source
- **Differences**: standalone secret managers focused on dev team
  workflows
- **Integration**: `InfisicalProvider`, `DopplerProvider` in providers crate

Source: [Infisical features](https://infisical.com/docs/documentation/platform/secret-rotation/overview),
[Doppler dynamic secrets](https://www.doppler.com/blog/secrets-management-best-practices-for-ephemeral-environments)

### n8n

- **Strengths**: credential sharing across workflows, OAuth2 built-in
- **Differences**: n8n stores everything in Postgres with single master
  key. No envelope encryption, no external providers, no dynamic secrets.
- **Lesson**: n8n's UX is good — credential setup wizards, OAuth2 flows,
  testing button. Nebula matches via `CredentialDescription` +
  interactive flows.

### SPIFFE/SPIRE

- **Strengths**: workload identity standard (CNCF), service mesh
  integration, X.509 and JWT SVIDs, zero-trust ready
- **Integration**: optional `spiffe` feature via `rust-spiffe` crate.
  `SpiffeCredential` implements `Credential` trait, returns `X509Svid`
  scheme.

Source: [rust-spiffe](https://github.com/maxlambrecht/rust-spiffe)

## Appendix B — References

### Industry patterns

- [Envelope encryption with AWS KMS](https://docs.aws.amazon.com/kms/latest/developerguide/kms-cryptography.html)
- [Envelope encryption with GCP KMS](https://cloud.google.com/kms/docs/envelope-encryption)
- [The security pattern every cloud developer should know](https://n.demir.io/articles/envelope-encryption-the-security-pattern-every-cloud-developer-should-know/)
- [Dynamic secrets for database credential management (Vault)](https://developer.hashicorp.com/vault/tutorials/db-credentials/database-secrets)
- [Automated secrets rotation](https://developer.hashicorp.com/hcp/docs/vault-secrets/auto-rotation)
- [Database Credential Rotation (HashiCorp)](https://www.hashicorp.com/en/solutions/credential-rotation)
- [Workload identity federation for cloud-native environments](https://aembit.io/blog/what-identity-federation-means-for-workloads-in-cloud-native-environments/)
- [Why we need short-lived credentials (HashiCorp)](https://www.hashicorp.com/en/blog/why-we-need-short-lived-credentials-and-how-to-adopt-them)
- [Just-in-time ephemeral credentials (Akeyless)](https://www.akeyless.io/blog/why-you-should-only-use-just-in-time-ephemeral-credentials/)
- [Ephemeral workload secrets (NHI)](https://nhimg.org/nhi-101/ephemeral-workload-secrets-non-human-identity)
- [Zero-downtime secrets rotation](https://oneuptime.com/blog/post/2026-01-30-security-secret-rotation-strategies/view)
- [Advanced credential rotation with grace period (tecRacer)](https://www.tecracer.com/blog/2023/06/advanced-credential-rotation-for-iam-users-with-a-grace-period.html)
- [OAuth2 token refresh best practices](https://oneuptime.com/blog/post/2026-01-24-oauth2-token-refresh/view)
- [Architecting scalable OAuth token management (Truto)](https://truto.one/blog/how-to-architect-a-scalable-oauth-token-management-system-for-saas-integrations/)
- [Tamper-evident audit log with SHA-256 hash chains](https://dev.to/veritaschain/building-a-tamper-evident-audit-log-with-sha-256-hash-chains-zero-dependencies-h0b)
- [Audit logs security: cryptographically signed logs (Cossack Labs)](https://www.cossacklabs.com/blog/audit-logs-security/)
- [SPIFFE/SPIRE workload identity](https://spiffe.io/)

### Rust ecosystem

- [secrecy crate](https://docs.rs/secrecy) — secret wrapper types
- [zeroize crate](https://docs.rs/zeroize) — memory wiping
- [keyring crate](https://docs.rs/keyring) — OS keyring abstraction
- [vaultrs crate](https://docs.rs/vaultrs) — HashiCorp Vault client
- [kms-aead crate](https://github.com/abdolence/kms-aead-rs) — KMS/AEAD envelope encryption for GCP/AWS
- [secret-vault crate](https://github.com/abdolence/secret-vault-rs) — secure in-memory vault
- [age crate](https://github.com/str4d/rage) — modern file encryption
- [rust-spiffe crate](https://github.com/maxlambrecht/rust-spiffe) — SPIFFE workload identity

### Alternatives review

- [Best secrets management tools 2026 (Infisical blog)](https://infisical.com/blog/best-secret-management-tools)
- [Infisical vs Doppler (Doppler blog)](https://www.doppler.com/blog/infisical-doppler-secrets-management-comparison-2025)
- [Top 5 secrets management tools 2026](https://guptadeepak.com/top-5-secrets-management-tools-hashicorp-vault-aws-doppler-infisical-and-azure-key-vault-compared/)

## Changelog

- **2026-04-15** — initial draft. Three-crate split (credential core /
  storage submodule / engine integration), envelope encryption with
  pluggable KMS, external provider delegation, dynamic secrets with
  `DYNAMIC` const, OIDC workload identity federation, tamper-evident
  audit log, distributed refresh coordination, tenancy-aware context,
  `CredentialId` newtype, schema migration to `nebula-schema`, retry
  facade deletion, executor relocation to engine. Integrates research
  from HashiCorp Vault, AWS KMS/Secrets Manager, GCP/Azure Key Vault,
  SPIFFE/SPIRE, Infisical, Doppler, and Rust ecosystem
  (secrecy/zeroize/keyring/vaultrs/kms-aead).
