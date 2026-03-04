# Interactions

## Ecosystem Map (Current + Planned)

`nebula-credential` is the security boundary for secrets in the Nebula workflow platform. Dependency direction: action/engine/runtime/resource/api → credential; credential → core, log, parameter, storage.

## Existing Crates

- **core:** Shared IDs (`CredentialId`, `CredentialKey`, `ScopeLevel`); credential context; domain types
- **log:** Credential lifecycle events; rotation audit; error traces
- **parameter:** `ParameterCollection` for protocol schemas; `ParameterValues` for credential input; `ParameterError` in credential errors
- **action:** Attaches credential references; uses `CredentialProvider` for credential access
- **resource:** May require credentials for DB/HTTP connections; uses credential context
- **storage:** Abstract storage provider (optional; credential has its own provider abstraction)
- **config:** May load credential manager config; hot-reload
- **validator:** Parameter validation used by credential schema validation

## Planned Crates

- **workflow / runtime / worker:** Will consume credential for node execution context
- **engine:** Will inject `CredentialProvider` into action context
- **nebula-api** *(Phase 4)*:
  - Credential CRUD surface: `GET /credentials`, `GET /credentials/:id`, `POST /credentials`, `POST /credentials/:id/callback`, `DELETE /credentials/:id`
  - Credential type catalog: `GET /credential-types` — returns registered type schemas from `CredentialManager`
  - Interactive flow bridge: receives `CreateResult::RequiresInteraction` → responds 202 with interaction descriptor → accepts callback params → calls `CredentialManager::continue_flow(id, UserInput::Callback { params })`
  - Expected contract: `CredentialManager::list()`, `get(id)`, `create(type_id, input)`, `continue_flow(id, user_input)`, `delete(id)`, `list_types()`
  - Current status: `create`, `continue_flow`, `list_types` are implemented for built-in protocols (`api_key`, `basic_auth`, `oauth2`); `CredentialProvider` impl supports id-based `get(id)` and type-based `credential<C>()` when type registry is configured
  - **Security boundary**: api layer never stores or logs raw secrets; only passes opaque params to manager

## Downstream Consumers

- **action:** Expects `CredentialProvider` trait; `CredentialRef` for type-safe acquisition; `CredentialContext` for scope
- **resource:** Expects credential resolution via context; `get_credential(id, &ctx)`
- **engine/runtime (future):** Expects `CredentialManager` or `CredentialProvider` in execution context
- **nebula-api** *(Phase 4)*:
  - `CredentialManager::list(filter)` → `Vec<CredentialMetadata>` for `GET /credentials`
  - `CredentialManager::get(id)` → `Option<CredentialMetadata>` + status for `GET /credentials/:id`
  - `CredentialManager::create(type_id, input)` → `InitializeResult` (may be `RequiresInteraction`) for `POST /credentials`
  - `CredentialManager::continue(id, UserInput)` → `InitializeResult<Complete>` for `POST /credentials/:id/callback`
  - `CredentialManager::delete(id)` → `Result<()>` for `DELETE /credentials/:id`
  - `CredentialManager::list_types()` → `Vec<CredentialTypeSchema>` for `GET /credential-types`
  - **read-only contract on secrets**: api reads metadata and orchestrates flows; never accesses raw secret material

## Upstream Dependencies

- **nebula-core:** `CredentialId` (UUID for instances), `CredentialKey` (normalized key for protocol types), `ScopeLevel` (hierarchical scope enum for access control), domain primitives; hard contract on ID format, key format, and scope semantics
- **nebula-log:** Tracing macros; optional; fallback: no-op when disabled
- **nebula-parameter:** `ParameterCollection`, `ParameterValues`, `ParameterError`; hard contract for schema validation
- **tokio, async-trait:** Async runtime; required
- **aes-gcm, argon2, zeroize:** Crypto; required for encryption
- **aws-sdk-secretsmanager, vaultrs, kube (optional):** Storage backends; feature-gated

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|---------------------|-----------|----------|------------|------------------|-------|
| credential -> core | out | CredentialId, CredentialKey, ScopeLevel, context types | sync | N/A | core owns ID/scope semantics |
| credential -> parameter | out | ParameterCollection, ParameterValues | sync | ParameterError in SchemaValidation | credential validates protocol schemas |
| credential -> log | out | tracing macros | sync | N/A | log never fails credential |
| action -> credential | in | CredentialProvider, CredentialRef | async | CredentialError, ManagerError | action acquires credentials |
| resource -> credential | in | get_credential(id, ctx) | async | CredentialError | resource resolves credentials |
| storage (optional) | in | StorageProvider trait | async | StorageError | credential owns provider impl |
| credential <-> nebula-api | in | CRUD + interactive flow continuation | async | CredentialError → HTTP 4xx/5xx | Phase 4; api never accesses secrets |
| credential <-> resource (cascade) | out | CredentialResource::authorize(new_state) | async | resource drain on failure | on rotation: linked resources refresh their instances |

## Runtime Sequence

### Standard acquire sequence

1. Engine/runtime creates `CredentialManager` with `StorageProvider` (local/AWS/Vault/K8s)
2. Action/resource receives `CredentialProvider` in execution context
3. Action calls `provider.get("cred_id", &ctx)` or `provider.credential::<ApiKey>(&ctx)`
4. Manager checks cache; on miss, delegates to `StorageProvider::retrieve`
5. Decrypt, validate scope via `caller_scope.is_contained_in_strict(&entry.owner_scope, resolver)`, return `SecretString` or protocol-specific state as `serde_json::Value` (deserialized to typed `State` for `CredentialType` users)
6. On rotation: `RotationTransaction` coordinates backup → new credential → grace period → revoke old

### Interactive credential creation (OAuth2 Authorization Code)

This sequence applies when the desktop user adds a new OAuth2 credential (e.g. GitHub, Google) through the UI.

```
Desktop → POST /credentials { type_id: "oauth2_github", params: { client_id, scope } }
        ← 202 { id, status: "pending_interaction",
                 interaction: { type: "redirect", url: "https://github.com/login/oauth/authorize?..." } }

Desktop opens url in system browser
  (redirect_uri = "nebula://credential/callback")

User authenticates in browser
Browser → nebula://credential/callback?code=XXX&state=YYY

Tauri deep-link handler parses URL
Desktop → POST /credentials/:id/callback { params: { code: "XXX", state: "YYY" } }

API → CredentialManager::continue(id, UserInput::Callback { params })
    → OAuth2Protocol exchanges code for access_token + refresh_token
    → Manager encrypts and stores state via StorageProvider
        ← 200 { id, status: "active", metadata: { name, type, scopes } }

Desktop shows credential as active; resources linked to this credential
receive authorize(new_state) and recreate pool instances if needed
```

### Credential-Resource refresh cascade

When a credential is rotated (token refresh or manual rotation):

1. `RotationTransaction` completes → new encrypted state persisted
2. `CredentialManager` emits internal `CredentialRotated { credential_id }` event
3. Resources registered with `CredentialResource` (associated type `Credential`) bound to this credential_id receive `authorize(&new_state)`
4. Pool instances are drained and recreated with the new auth material
5. In-flight instances finish naturally; new acquires use updated instances

This ensures resource pools never use stale credentials without manual intervention.

## Cross-Crate Ownership

- **credential owns:** Credential lifecycle, encryption, scope enforcement, rotation orchestration, provider abstraction
- **core owns:** ID types (CredentialId UUID, CredentialKey domain key), ScopeLevel semantics; credential uses them for instance identity, type identity, and access control
- **parameter owns:** Schema types, validation rules; credential uses for protocol schemas
- **action/resource own:** When to fetch credentials, how to use them; credential owns how to store/retrieve
- **storage (if used):** Credential defines `StorageProvider` trait; concrete backends in credential or storage crate

## Failure Propagation

- Storage failures bubble as `StorageError` → `CredentialError::Storage`
- Crypto failures: `CryptoError` → `CredentialError::Crypto`; no retry (fail secure)
- Scope violations: `ManagerError::ScopeViolation`; fail-fast
- Batch operations: `ManagerError::BatchError` with partial results
- Rotation failures: `RotationError`; rollback to previous credential; alert operator

## Versioning and Compatibility

- **Compatibility promise:** Patch/minor preserve `StorageProvider`, `CredentialProvider`, `CredentialContext`, error variants
- **Breaking-change protocol:** Declare in MIGRATION.md; major version bump; migration path for provider/manager API
- **Deprecation window:** Minimum 6 months for public API changes

## Contract Tests Needed

- action/credential: `CredentialProvider` mock; scope enforcement; error propagation
- resource/credential: credential resolution in resource context
- parameter/credential: protocol schema validation; `ParameterError` in `SchemaValidation`
- api/credential: `POST /credentials` returns 202 with redirect interaction for OAuth2 AuthCode flow
- api/credential: `POST /credentials/:id/callback` completes flow and returns 200 with active status
- api/credential: `GET /credential-types` returns type schemas
- cascade: resource instances receive `authorize(new_state)` after credential rotation
