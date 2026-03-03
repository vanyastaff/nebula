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
