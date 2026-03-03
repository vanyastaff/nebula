# Proposals (Senior Review)

## P-001: Mandatory Capability Negotiation for Providers (Potential Breaking)

Problem:
- Provider behavior can differ subtly by backend (CAS support, list filtering, metrics).

Proposal:
- Introduce explicit `ProviderCapabilities` struct returned by `StorageProvider::capabilities()`.
- Manager validates at startup that required capabilities are present.

Impact:
- Stricter startup validation, potential configuration breakage for ambiguous setups.
- New trait method (breaking for existing implementations).

## P-002: Strict Scope Enforcement Mode (Potential Breaking)

Problem:
- Scope handling mistakes are catastrophic in multi-tenant systems. Currently scope can be `None`.

Proposal:
- Add strict mode that rejects any operation with `None` or ambiguous scope context.
- Configurable: `ScopeEnforcement::Strict` (reject None) vs `ScopeEnforcement::Permissive` (allow None, current default).

Impact:
- Some existing calls may fail until context propagation is fixed.
- Significantly reduces risk of cross-tenant data leakage.

## P-003: Rotation Policy Versioning

Problem:
- Policy evolution risks schema drift for persisted rotation metadata.

Proposal:
- Add explicit versioned policy envelope and migration helpers.
- Schema: `{ version: u32, policy: RotationPolicy }`.

Impact:
- Non-breaking initially with compatibility parser; long-term safer migrations.

## P-004: Secret Access Budget and Rate Controls

Problem:
- High-load systems can over-pull secrets and stress provider backends (AWS rate limits: ~10K/sec, Vault: configurable).

Proposal:
- Configurable fetch budgets per scope/credential with token-bucket rate limiting.
- Backpressure via `Err(RateLimited { retry_after })`.

Impact:
- Behavior changes under load; improves resilience and cost control.
- Prevents runaway credential fetch storms.

## P-005: Unified Error Taxonomy Across Modules

Problem:
- Manager/provider/rotation errors are rich but can fragment observability pipelines.

Proposal:
- Define shared machine-readable error taxonomy with categories (auth, storage, crypto, scope, rotation).
- Add `error_code()` method returning a stable string identifier (e.g. `"CRED-001"`).

Impact:
- Additive documentation + helper APIs; large observability payoff.
- Enables structured alerting without pattern-matching on error messages.

## P-006: AuditLogger Trait (New)

Problem:
- Audit logging is currently done via `tracing` macros inline. No structured contract for audit consumers.

Proposal:
- Introduce `AuditLogger` trait with structured methods:
  ```rust
  pub trait AuditLogger: Send + Sync {
      async fn log_access(&self, event: CredentialAccessEvent) -> Result<(), AuditError>;
      async fn log_rotation(&self, event: RotationEvent) -> Result<(), AuditError>;
      async fn log_violation(&self, event: SecurityViolation) -> Result<(), AuditError>;
  }
  ```
- Default implementation: tracing-based (no-op if tracing disabled).
- Production implementations: S3, Kafka, database.

Impact:
- Additive. Enables compliance pipelines (SOC2, HIPAA, GDPR) without coupling to tracing.
- Makes audit events first-class types, not log strings.

Source: Archive `Meta/ARCHITECTURE-DESIGN.md` — AuditLogger trait design.

## P-007: EncryptionProvider Trait (New)

Problem:
- Encryption is currently a utility function (`utils::crypto::encrypt/decrypt`). Cannot swap encryption backends (local AES vs HSM vs KMS).

Proposal:
- Introduce `EncryptionProvider` trait:
  ```rust
  pub trait EncryptionProvider: Send + Sync {
      fn encrypt(&self, plaintext: &[u8], context: &EncryptionContext) -> Result<Vec<u8>, EncryptionError>;
      fn decrypt(&self, ciphertext: &[u8], context: &EncryptionContext) -> Result<Vec<u8>, EncryptionError>;
      fn generate_key(&self) -> Result<EncryptionKey, EncryptionError>;
  }
  ```
- Default: `LocalEncryptionProvider` (AES-256-GCM, Argon2id).
- Production: `KmsEncryptionProvider` (AWS KMS), `VaultTransitProvider` (Vault Transit).

Impact:
- Breaking: `CredentialManagerBuilder` gains `encryption_provider` setter.
- Enables HSM/KMS integration without changing caller code.
- Key hierarchy becomes configurable.

Source: Archive `Meta/ARCHITECTURE-DESIGN.md` — EncryptionProvider trait.

## P-008: L2 Redis Cache (New)

Problem:
- Current L1 in-memory cache is per-node. In a multi-node fleet, cache misses on one node hit storage even if another node has the credential cached.

Proposal:
- Add L2 cache layer backed by Redis (shared across fleet).
- Cache flow: L1 hit → return. L1 miss → L2 hit → populate L1, return. L2 miss → storage → populate L2 + L1.
- Feature-gated: `cache-redis`.

Impact:
- Additive (feature-gated). Significantly reduces storage load in fleet deployments.
- TTL: L1 ~5 min, L2 ~30 min.
- Requires Redis TLS, authentication, and cache poisoning prevention (HMAC on entries).

Source: Archive `archive-nebula-credential-architecture-2.md` — L1/L2 cache design.

## P-009: Credential Lifecycle State Machine (New)

Problem:
- Credential lifecycle states (Active, Expired, Rotating, etc.) are implicit. No explicit state machine validates transitions.

Proposal:
- Introduce `CredentialStateMachine` with 11 states and validated transitions.
- Illegal transitions return `Err(StateError::IllegalTransition { from, to })`.
- Track transition history for audit trail.
- See ARCHITECTURE.md for state graph.

Impact:
- Potentially breaking if credential status is currently set without validation.
- Improves safety: impossible to accidentally transition `Active → Authenticating`.
- Enables rich status reporting in UI (`pending_interaction`, `rotating`, `grace_period`).

Source: Archive `Meta/ARCHITECTURE-DESIGN.md` — State Machine Architecture.

## P-010: Proactive OAuth2 Token Refresh (New)

Problem:
- OAuth2 access tokens have short TTLs (~1 hour). Actions that run near token expiry may receive stale tokens.

Proposal:
- Background refresh task spawned by `CredentialManager` when a cached OAuth2 token is within 5 minutes of expiry.
- Token refreshed transparently via `FlowProtocol::refresh()`.
- Uses `RotationPolicy::BeforeExpiry` with 80% threshold.

Impact:
- Additive. Requires `FlowProtocol::refresh` to be implemented for OAuth2.
- Prevents expiration-related workflow failures.

Source: Archive `Advanced/Rotation-Policies.md` — Before-Expiry policy for OAuth2 tokens.
