# Decisions

## D-001: Provider Abstraction (Trait Object, Not Generic)

Status: accepted

Decision:
- `StorageProvider` is a trait object (`Arc<dyn StorageProvider>`), not a generic parameter `CredentialManager<S: StorageProvider>`.

Reason:
- A generic manager means `CredentialManager<LocalStorageProvider>` and `CredentialManager<AwsProvider>` are different types — you can't pass one where another is expected.
- Trait objects (`dyn StorageProvider`) unify the type at the cost of one vtable dispatch per operation — acceptable for storage I/O.

Rejected:
- Generic `<S: StorageProvider>` — type signature leaks across crate boundaries; export/import complexity.

## D-002: Context + Scope Isolation

Status: accepted

Decision:
- Credential access is context-driven (tenant/user/scope aware). Every operation requires `CredentialContext`.

Reason:
- Strict multi-tenant security boundary. A scope violation is a security incident, not an operational error.

Rules:
- Cache keyed by `(CredentialId, ScopeId)` — never serve cross-scope hits
- `ScopeViolation` logged with full context before returning `Err`

## D-003: Protocol-Agnostic Core

Status: accepted

Decision:
- Protocol-specific logic isolated in `protocols/`, with shared contracts in `traits/`.
- `CredentialManager` does NOT import anything from `protocols/`.

Reason:
- Adding SAML, Kerberos, or any new auth protocol should require only implementing a trait and registering a type — no changes to `core/` or `manager/`.

## D-004: Rotation as First-Class Subsystem

Status: accepted

Decision:
- Rotation is a typed `RotationTransaction` struct with explicit phases, not an implicit "replace credential" operation in the manager.

Reason:
- Operational safety and auditability. A rotation that fails halfway must never leave the system without a valid credential.

Rejected:
- Simple "atomic replace" — loses grace period semantics
- Background task with no explicit phases — loses observability and rollback

## D-005: Security-First Utility Layer

Status: accepted

Decision:
- Centralize crypto/secret/time/retry utilities under controlled APIs (`utils/`).

Reason:
- Avoid duplicated, inconsistent secret handling across modules. Single source of truth for `SecretString`, `EncryptionKey`, encrypt/decrypt.

## D-006: Fail-Secure for Crypto Errors

Status: accepted

Decision:
- `CryptoError::DecryptionFailed` is a terminal error with no retry or fallback path. There is no "return partial data" path.

Reason:
- A workflow that receives an encrypted blob as if it were plaintext is catastrophically wrong. When decryption fails, the correct response is a hard error.

Rejected:
- Returning `None` on decryption failure (confused with "not found")
- Returning the encrypted blob (caller might misuse it)
- Silent fallback to default

## D-007: FlowProtocol over Ad-Hoc HTTP Redirects

Status: accepted

Decision:
- Model interactive auth flows as a `FlowProtocol` state machine (InitializeResult → RequiresInteraction → UserInput → Complete) rather than hardcoding OAuth2 redirect logic in the API layer.

Reason:
- n8n hardcodes OAuth2 in the HTTP layer — fragile and not extensible to SAML, Kerberos, Device Flow.
- Nebula owns the state machine in the credential crate; the API layer is a thin transport.

Rejected:
- Hardcoded OAuth2 in API layer (n8n pattern)
- Separate "OAuth2 service" — unnecessary indirection

## D-008: PKCE Mandatory for OAuth2 Authorization Code (New)

Status: accepted

Decision:
- PKCE (Proof Key for Code Exchange) is mandatory for all OAuth2 authorization code flows. No opt-out.

Reason:
- OAuth 2.1 specification makes PKCE mandatory. Prevents authorization code interception attacks.
- SHA-256 challenge method (`S256`); 32-byte random verifier (256 bits entropy).

Source: Archive `Meta/SECURITY-SPECIFICATION.md` — OAuth2 Security section, RFC 7636.

## D-009: Nonce Handling Strategy (New)

Status: accepted

Decision:
- AES-256-GCM nonces use format: `[4-byte random prefix | 8-byte counter]`.
- Nonce reuse with the same key is treated as a **critical security incident** (key-recovery attack on AES-GCM).

Reason:
- Random prefix provides 2^32 unique sequences. Monotonic counter prevents reuse within sequence.
- Total nonce space: 2^96 (meets GCM requirements).

Source: Archive `Meta/SECURITY-SPECIFICATION.md` — Nonce Generation section.

## D-010: API Key Format and Storage (New)

Status: accepted

Decision:
- API key format: `sk_<43-char base64url random>` (256-bit entropy, `sk_` prefix for detection).
- Storage: BLAKE3 hash only — API keys never stored in plaintext.
- Validation: constant-time comparison via `subtle` crate.

Reason:
- Prefix enables automated detection (GitHub secret scanning, CI hooks).
- BLAKE3 faster than SHA-256, cryptographically secure, parallelizable.
- Constant-time comparison prevents timing attacks.

Source: Archive `Meta/SECURITY-SPECIFICATION.md` — API Key Security section.

## D-011: Immutable Credential Ownership (New)

Status: accepted

Decision:
- `OwnerId` is set at credential creation and cannot be changed. There is no `transfer_ownership` method.

Reason:
- Ownership transfer creates privilege escalation vectors. Immutable ownership is simpler to reason about and audit.
- If credential needs a new owner, revoke and re-create.

Source: Archive `Advanced/Access-Control.md` — Ownership Immutability section.

## D-012: Type-Safe Credential-Resource Binding (New)

Status: accepted

Decision:
- `CredentialResource` trait uses associated type `type Credential: CredentialType` for compile-time binding between resources and their required credentials.
- No string-based credential type selection (unlike n8n).

Reason:
- Compile-time safety: impossible to bind a GitHub HTTP client to a database credential.
- Refactoring safety: renaming a credential type is a compile error, not a runtime surprise.

Source: Archive `2026-02-18-protocol-system-design.md` — CredentialResource section.

## D-013: Dynamic ProtocolRegistry via ErasedProtocol Type Erasure

Status: accepted

Decision:
- `ProtocolRegistry` stores `Arc<dyn ErasedProtocol>` — object-safe, type-erased trait.
- Typed protocols (`FlowProtocol`, `StaticProtocol`) are bridged via `ProtocolDriver<P>` and `StaticProtocolDriver<P>` adapters that capture config at registration time and serialize/deserialize state through `serde_json::Value`.
- Community plugins implement `ErasedProtocol` directly for full flexibility.
- `ProtocolRegistry` is keyed by `CredentialKey` from `nebula-core`.

Reason:
- Dynamic registry requires runtime-composable types. Rust trait objects (`dyn Trait`) require object safety — no associated types or `where Self: Sized` on called methods in the erased layer.
- Typed traits (`FlowProtocol`) keep compile-time safety for protocol implementors; the erased layer provides runtime flexibility for plugin loading and the API layer.
- `serde_json::Value` for state is the correct boundary: state must be serialized to storage anyway.

Rejected:
- Compile-time only (inventory/linkme crate) — cannot support runtime plugin loading.
- Raw `Box<dyn Any>` for state — not serializable; loses schema information.
- `Box<dyn FlowProtocol>` directly — not object-safe due to associated types.

## D-014: ScopeLevel (Enum) Not ScopeId (UUID) for Access Control

Status: accepted

Decision:
- Credential access control uses `ScopeLevel` from `nebula-core` — a hierarchical enum (`Global`, `Organization`, `Project`, `Workflow`, `Execution`, `Action`) — not a flat `ScopeId` UUID.
- `CredentialContext` carries `caller_scope: ScopeLevel` (the requester's runtime scope).
- `CredentialEntry` stores `owner_scope: ScopeLevel` (typically `Project` or `Organization`).
- Access check: `caller_scope.is_contained_in_strict(&owner_scope, resolver)` using `ScopeResolver` to verify ownership chains.

Reason:
- `ScopeId` as a UUID does not exist in `nebula-core`. The actual scope system is `ScopeLevel` with hierarchical containment semantics and a `ScopeResolver` trait for ownership verification.
- Hierarchical containment: an `Action` running in `Execution(E)` within `Project(P)` automatically has access to credentials owned by `Project(P)` — no special-casing required.
- `is_contained_in_strict` uses a `ScopeResolver` to verify the full ownership chain (execution→workflow→project→organization), preventing scope spoofing.

Rejected:
- Flat `ScopeId` UUID — cannot express hierarchical containment; requires explicit join for every access check.
- `Option<ScopeId>` with `None` meaning global — ambiguous and error-prone in multi-tenant systems.

## D-015: CredentialKey (from nebula-core) as Protocol Type Identifier

Status: accepted

Decision:
- Protocol type identity uses `CredentialKey` from `nebula-core` — a normalized domain key (`[a-z][a-z0-9_]*`) — not a raw `&str` or a new `ProtocolId` type.
- `ErasedProtocol::credential_key() -> &CredentialKey` is the protocol's type identity in the registry.
- `CredentialType::credential_key() -> CredentialKey` links typed Rust structs to their registry entry.
- `ProtocolRegistry` is `HashMap<CredentialKey, Arc<dyn ErasedProtocol>>`.

Reason:
- `CredentialKey` already exists in `nebula-core` alongside `PluginKey`, `ActionKey`, `ParameterKey` — consistent naming and validation across the platform.
- Domain validation (normalized format) prevents typos and malformed type identifiers.
- Clear separation: `CredentialId` (UUID) identifies a credential *instance*; `CredentialKey` identifies a credential *type* — consistent with `PluginKey` vs entity UUIDs elsewhere in the system.

Rejected:
- Raw `&str` — no validation, easy typos, not domain-tagged.
- New `ProtocolId` type — reinvents `CredentialKey` which already exists in core.
- `std::any::TypeId` — not stable across compilations, not serializable, not human-readable.

## D-016: CredentialLifecycle (Internal) vs CredentialStatus (Public API)

Status: accepted

Decision:
- `CredentialLifecycle` is a rich 11-state internal enum used by the state machine, rotation engine, and cache invalidation logic.
- `CredentialStatus` is a lean 6-state public enum returned in API responses and UI — it hides internal implementation states from callers.

```rust
// Internal — only inside nebula-credential
pub(crate) enum CredentialLifecycle {
    Uninitialized, PendingInteraction, Authenticating,
    Active, Expired, Refreshing,
    RotationScheduled, Rotating, GracePeriod,
    Revoked, Failed,
}

// Public — API responses, CredentialEntry, list_credentials()
pub enum CredentialStatus {
    PendingInteraction,   // waiting for user action (OAuth2, Device Flow)
    Active,               // ready for use; also covers GracePeriod (both credentials valid)
    Rotating,             // rotation in progress (covers RotationScheduled + Rotating)
    Expired,              // refresh needed
    Revoked,              // terminal — manually revoked
    Failed,               // terminal — unrecoverable error
}
```

Mapping:
- `Uninitialized | Authenticating` → not yet visible in public API (credential not committed to storage)
- `Refreshing` → `Active` (transparent to callers; old token still valid)
- `RotationScheduled | Rotating` → `Rotating`
- `GracePeriod` → `Active` (both credentials valid; caller unaffected)

Reason:
- Callers (UI, API clients, action developers) need to know: "can I use this credential?" — not "is it in RotationScheduled or Rotating?". Exposing internal states leaks implementation details and breaks API stability when internal states change.
- `GracePeriod → Active` hides rotation mechanics from callers transparently.
- `Refreshing → Active` avoids confusing UI states for a background operation.

Rejected:
- Single flat enum for both — internal state changes break the public API contract.
- `Option<CredentialLifecycle>` in public API — overly complex; callers don't need 11 states.
