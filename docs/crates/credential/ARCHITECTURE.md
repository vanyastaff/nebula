# Architecture

## Positioning

`nebula-credential` is a security-critical infrastructure crate.

Dependency direction:
- runtime/action/api layers -> `nebula-credential`
- `nebula-credential` should not depend on workflow business logic

## Module Map

| Module | Key types exported | Role |
|--------|-------------------|------|
| `core` | `CredentialId`, `ScopeId`, `CredentialContext`, `CredentialMetadata`, `CredentialDescription`, `CredentialFilter`, `CredentialState`, `CredentialRef`, `CredentialProvider`, `CredentialError`, `StorageError`, `CryptoError`, `ValidationError`, `ManagerError`, `SecretString` | Identity, scope, errors, primitives |
| `traits` | `StorageProvider`, `StateStore`, `DistributedLock`, `CredentialType`, `FlowProtocol`, `StaticProtocol`, `InteractiveCredential`, `Refreshable`, `Revocable`, `RotatableCredential`, `TestableCredential` | Infrastructure contracts |
| `providers` | `MockStorageProvider`, `LocalStorageProvider`*, `AwsSecretsManagerProvider`*, `HashiCorpVaultProvider`*, `KubernetesSecretsProvider`*, `ProviderConfig`, `StorageMetrics` | Concrete backends (feature-gated) |
| `manager` | `CredentialManager`, `CredentialManagerBuilder`, `CacheLayer`, `CacheConfig`, `CacheStats`, `ValidationResult`, `ValidationDetails`, `ManagerConfig`, `EvictionStrategy` | High-level CRUD, caching, validation |
| `protocols` | `ApiKeyProtocol`, `BasicAuthProtocol`, `DatabaseProtocol`, `HeaderAuthProtocol`, `OAuth2Protocol`+config+state+flow, `LdapProtocol`, `SamlConfig`, `KerberosConfig`, `MtlsConfig` | Protocol-specific models |
| `rotation` | `RotationPolicy`, `RotationTransaction`, `RotationState`, `RotationError`, `GracePeriodConfig`, rotation scheduler, blue-green helpers | Policy-driven rotation orchestration |
| `utils` | `EncryptionKey`, `EncryptedData`, `encrypt`, `decrypt`, `SecretString`, `RetryPolicy` | Crypto, secret handling, retry |

\* Feature-gated: `storage-local`, `storage-aws`, `storage-vault`, `storage-k8s`

## Data and Control Flow

### Credential Acquire (happy path)

```
caller
  │
  ├─→ CredentialManager::retrieve(id, ctx)
  │         │
  │         ├─→ scope check (CredentialContext validates tenant/scope)
  │         │
  │         ├─→ CacheLayer::get(id)  ──hit──→ return EncryptedData
  │         │        │
  │         │       miss
  │         │        │
  │         ├─→ StorageProvider::retrieve(id)
  │         │        │
  │         │   StorageError::NotFound → None
  │         │   StorageError::* → CredentialError::Storage
  │         │        │
  │         ├─→ CacheLayer::insert(id, data, ttl)
  │         │
  │         └─→ return Some((EncryptedData, CredentialMetadata))
  │
  └── caller decrypts with EncryptionKey → SecretString
```

### Rotation Flow (RotationTransaction)

```
RotationScheduler detects policy trigger
  │
  ├─→ RotationTransaction::begin()
  │         │
  │         ├─→ backup current credential
  │         ├─→ generate/acquire new credential
  │         ├─→ store new encrypted state via StorageProvider
  │         ├─→ grace period: old credential still valid
  │         │
  │         ├── failure at any point → rollback to backup
  │         │
  │         └─→ revoke old credential (end of grace period)
  │
  └─→ CredentialManager emits CredentialRotated event
            │
            └─→ resource::Manager::notify_credential_rotated(id, &new_state)
                      → linked CredentialResource instances call authorize(&new_state)
                      → pool drained; new acquires use updated auth
```

## Key Internal Invariants

- `#![forbid(unsafe_code)]` enforced at lib root
- `CredentialManager` is `Clone`; clones share the same `Arc<StorageProvider>` and `Arc<CacheLayer>`
- Cache is keyed by `(CredentialId, ScopeId)` — scope isolation is enforced at the cache boundary
- `CryptoError::DecryptionFailed` is never retried; it is fatal (fail-secure)
- `SecretString` implements `Debug` with redaction; never exposes raw secret in error messages or logs
- `StorageProvider::retrieve` is idempotent and safe to retry; `store`/`delete` are not
- Rotation failure always triggers rollback to the backup credential; new state is never partially applied
- `core::adapter` module is disabled (TODO Phase 5 comment in source)

## Security Boundaries

- encrypted credential payloads are first-class values
- context + scope is used for tenant isolation
- secret value handling is centralized in dedicated utility types
- unsafe code is forbidden at crate root

## Operational Properties

- async-first API surface
- provider abstraction allows environment-specific backend choice
- cache layer is optional and configurable
- rotation subsystem supports periodic/scheduled/manual/before-expiry patterns

## Known Complexity Hotspots

- wide feature matrix for providers and protocols
- large rotation subsystem with many safety components
- extensive internal docs in `crates/credential/docs/` require synchronization discipline

## Comparative Analysis

Sources: n8n, Node-RED, Activepieces/Activeflow, Temporal/Prefect/Airflow (credential-relevant parts).

- **Adopt:** Encrypted credential storage; scope/tenant isolation; provider abstraction; OAuth2 flows; credential type schemas; rotation with grace period
- **Reject:** Plaintext credential storage; global credential namespace; credentials in workflow JSON
- **Defer:** Credential sharing between workflows; credential versioning UI; HSM integration
