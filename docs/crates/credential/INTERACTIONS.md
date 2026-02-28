# Interactions

## Ecosystem Map (Current + Planned)

`nebula-credential` is the security boundary for secrets in the Nebula workflow platform. Dependency direction: action/engine/runtime/resource/api → credential; credential → core, log, parameter, storage.

## Existing Crates

- **core:** Shared IDs (`CredentialId`, `ScopeId`); credential context; domain types
- **log:** Credential lifecycle events; rotation audit; error traces
- **parameter:** `ParameterCollection` for protocol schemas; `ParameterValues` for credential input; `ParameterError` in credential errors
- **action:** Attaches credential references; uses `CredentialProvider` for credential access
- **resource:** May require credentials for DB/HTTP connections; uses credential context
- **storage:** Abstract storage provider (optional; credential has its own provider abstraction)
- **config:** May load credential manager config; hot-reload
- **validator:** Parameter validation used by credential schema validation

## Planned Crates

- **workflow / runtime / worker:** Will consume credential for node execution context
- **api / cli / ui:** Will use credential for auth flows, form rendering, secret input
- **engine:** Will inject `CredentialProvider` into action context

## Downstream Consumers

- **action:** Expects `CredentialProvider` trait; `CredentialRef` for type-safe acquisition; `CredentialContext` for scope
- **resource:** Expects credential resolution via context; `get_credential(id, &ctx)`
- **engine/runtime (future):** Expects `CredentialManager` or `CredentialProvider` in execution context
- **api (future):** Expects credential CRUD, validation, rotation APIs

## Upstream Dependencies

- **nebula-core:** `CredentialId`, `ScopeId`, domain primitives; hard contract on ID format and scope semantics
- **nebula-log:** Tracing macros; optional; fallback: no-op when disabled
- **nebula-parameter:** `ParameterCollection`, `ParameterValues`, `ParameterError`; hard contract for schema validation
- **tokio, async-trait:** Async runtime; required
- **aes-gcm, argon2, zeroize:** Crypto; required for encryption
- **aws-sdk-secretsmanager, vaultrs, kube (optional):** Storage backends; feature-gated

## Interaction Matrix

| This crate <-> Other | Direction | Contract | Sync/Async | Failure handling | Notes |
|---------------------|-----------|----------|------------|------------------|-------|
| credential -> core | out | CredentialId, ScopeId, context types | sync | N/A | core owns ID semantics |
| credential -> parameter | out | ParameterCollection, ParameterValues | sync | ParameterError in SchemaValidation | credential validates protocol schemas |
| credential -> log | out | tracing macros | sync | N/A | log never fails credential |
| action -> credential | in | CredentialProvider, CredentialRef | async | CredentialError, ManagerError | action acquires credentials |
| resource -> credential | in | get_credential(id, ctx) | async | CredentialError | resource resolves credentials |
| storage (optional) | in | StorageProvider trait | async | StorageError | credential owns provider impl |

## Runtime Sequence

1. Engine/runtime creates `CredentialManager` with `StorageProvider` (local/AWS/Vault/K8s)
2. Action/resource receives `CredentialProvider` in execution context
3. Action calls `provider.get("cred_id", &ctx)` or `provider.credential::<ApiKey>(&ctx)`
4. Manager checks cache; on miss, delegates to `StorageProvider::retrieve`
5. Decrypt, validate scope, return `SecretString` or protocol-specific state
6. On rotation: `RotationTransaction` coordinates backup → new credential → grace period → revoke old

## Cross-Crate Ownership

- **credential owns:** Credential lifecycle, encryption, scope enforcement, rotation orchestration, provider abstraction
- **core owns:** ID types, scope semantics, domain primitives
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
