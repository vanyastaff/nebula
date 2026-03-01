# nebula-credential Constitution

> **Version**: 1.0.0 | **Created**: 2026-03-01

---

## Platform Role

Nebula is a workflow automation platform. Workflows connect to external services:
GitHub, Slack, Postgres, Google Sheets, Stripe, Salesforce. Every one of those
connections requires authentication — an API key, an OAuth2 token, a database
password. The workflow author should not manage raw secrets. The platform should.

**nebula-credential is the security boundary of the Nebula platform.**

It answers one question: *How does a workflow action get the secret it needs to
talk to an external service, without ever seeing the raw secret in workflow JSON,
in logs, or in error messages?*

```
workflow author adds GitHub OAuth2 credential in UI
    ↓
nebula-credential stores encrypted state via StorageProvider
    ↓
workflow engine runs "Create Issue" action
    ↓
action receives CredentialProvider from engine context
    ↓
action calls provider.credential::<OAuth2GitHub>(&ctx)
    ↓
credential decrypts, validates scope, returns SecretString
    ↓
action uses token to call GitHub API
    ↓
raw secret never appears in logs, errors, or workflow JSON
```

This is the security contract. It never bends.

---

## User Stories

### Story 1 — Workflow Author Adds a GitHub OAuth2 Credential (P1)

A workflow author wants to create a workflow that creates GitHub Issues.
They go to the credential page, click "Add GitHub OAuth2", get redirected
to GitHub, authorize Nebula, and return to see the credential is active.
The raw tokens never appear anywhere — not in the URL, not in the logs, not
in the workflow JSON.

**Acceptance**:
- `POST /credentials` with `{ type_id: "oauth2_github" }` → 202 with redirect URL
- User opens URL in browser, authorizes, GitHub redirects back
- `POST /credentials/:id/callback` with `{ code, state }` → 200 active credential
- Token stored AES-256-GCM encrypted, never returned in any API response

### Story 2 — Action Developer Accesses a Credential (P1)

A developer writing a "Send Slack Message" action needs to get the Slack
OAuth2 token for the current execution context. They should not know about
encryption, storage providers, or scope rules — they just want the token.

**Acceptance**:
```rust
let token: OAuth2State = ctx.credentials()
    .credential::<SlackOAuth2>(&ctx)
    .await?;
// token.access_token is SecretString — redacted in Debug/Display
use_slack_api(token.access_token.expose_secret()).await?;
```
No awareness of provider backends, encryption, or scope enforcement required.

### Story 3 — Platform Operator Rotates Credentials in Production (P2)

An operator discovers that a set of API keys may have been compromised.
They need to rotate them across all workflows that use them, with zero
downtime: old key must remain valid during rotation, new key must be
tested before old is revoked.

**Acceptance**:
- Rotation triggered via API or scheduled policy
- `RotationTransaction`: backup → generate new → grace period → revoke old
- Resource pools automatically receive `authorize(new_state)` and refresh
- No in-flight workflow request fails during grace period

### Story 4 — Security Auditor Reviews Credential Access (P3)

A security auditor needs to verify that credentials are being accessed
only by the workflows they belong to, and that every access and rotation
is logged with a structured audit trail.

**Acceptance**:
- Every `acquire`, `rotate`, `revoke` operation emits a structured audit event
- `CredentialContext` with `owner_id` and `scope_id` is recorded on every access
- `ScopeViolation` errors are logged and auditable
- No raw secret material appears in any audit log entry

---

## Core Principles

### I. Fail-Secure — Never Return Partial Data

**On any cryptographic failure, return `Err`. Never return partial, corrupted,
or decrypted-with-wrong-key data.**

**Rationale**: A workflow that receives an invalid token is confused. A workflow
that receives an encrypted blob as if it were plaintext is catastrophically wrong.
When decryption fails, the correct response is a hard error that the caller
handles — not a best-effort partial result.

**Rules**:
- `CryptoError::DecryptionFailed` MUST be a terminal error — no retry, no fallback
- MUST NOT return `Option<T>` from decrypt — only `Result<T, CryptoError>`
- MUST NOT log the plaintext of any credential payload, ever
- `SecretString` MUST redact itself in `Debug`, `Display`, and `Serialize`

### II. Scope Isolation is Non-Negotiable

**A credential scoped to Tenant A must NEVER be accessible to Tenant B, to
any other workflow, or to any action running in a different execution context.**

**Rationale**: Multi-tenant workflow automation is the primary use case.
A scope violation is a security incident, not an operational error.
The system must fail loudly and completely when scope is violated.

**Rules**:
- MUST validate `CredentialContext.scope` on every `retrieve`, `list`, and `validate`
- `ScopeViolation` MUST log the violation with full context before returning `Err`
- Cache MUST be keyed by `(credential_id, scope_id)` — never serve cross-scope hits
- MUST forbid `unsafe` code in any scope enforcement path

### III. Protocol-Agnostic Core

**The core types know nothing about OAuth2, API keys, or LDAP.
Protocol-specific logic lives in `protocols/` and is composable.**

**Rationale**: Today the platform needs OAuth2 and API keys. In six months
it will need SAML and mTLS. If the core bakes in OAuth2 concepts, adding
SAML requires modifying the core. With a protocol-agnostic core, adding
SAML is implementing a trait and registering a type.

**Rules**:
- `CredentialManager` MUST NOT import anything from `protocols/`
- `StorageProvider` stores `EncryptedData` — not protocol-specific state
- New auth protocols MUST be addable without touching `core/` or `manager/`
- `FlowProtocol` trait MUST handle multi-step interactive flows generically

### IV. Interactive Flows are First-Class

**OAuth2, SAML, Device Flow, and other multi-step authentication flows
are first-class concerns — not afterthoughts handled by the API layer.**

**Rationale**: n8n handles OAuth2 by hardcoding redirect URL logic in the
HTTP layer. That makes it fragile and hard to extend. Nebula models interactive
flows with `FlowProtocol` + `InitializeResult` + `UserInput` — a state machine
that the credential crate owns and the API layer merely transports.

**Rules**:
- `FlowProtocol::initialize` MUST return `InitializeResult` (Complete / Pending / RequiresInteraction)
- `InteractiveCredential::continue_flow` MUST accept `UserInput` and return `InitializeResult`
- The API layer MUST be a thin transport — it passes `UserInput` to `CredentialManager`, not vice versa
- `CredentialManager` MUST store partial state for in-progress flows

### V. Rotation is a Safety Subsystem, Not a Cron Job

**Credential rotation is a critical operational safety feature. It must be
transactional, observable, and resilient to partial failures.**

**Rationale**: A rotation that fails halfway leaves the system in an unknown
state. If the old credential was revoked but the new one wasn't stored,
the workflow breaks. Rotation must be atomic in the "succeed or roll back" sense.

**Rules**:
- `RotationTransaction` MUST implement backup → new → grace period → revoke atomically
- Failure at any phase MUST roll back to the previous credential
- MUST emit audit events for every rotation phase outcome
- Grace period MUST allow old credential to serve until explicitly revoked or expired

---

## Production Vision

### The credential page (n8n-inspired but better)

A production Nebula deployment has a credential management page in the
desktop UI where workflow authors:
1. Browse available credential types (OAuth2 GitHub, API Key Slack, DB Postgres)
2. Add credentials with interactive flows (OAuth2 redirects, device flows)
3. See credential status: active / pending_interaction / rotating / error
4. See which workflows use which credentials
5. Trigger manual rotation

Behind the scenes:
- Credentials encrypted with AES-256-GCM, keys in KMS (AWS/Vault)
- Multi-level cache: L1 in-memory (hot credentials), L2 optional Redis (shared fleet)
- Distributed lock via Redis/etcd for rotation transactions (prevents double-rotation)
- Audit log streaming to S3 or Kafka for compliance teams

### From the archives: full production architecture

The archive `_archive/archive-nebula-credential-architecture-2.md` contains a rich
design with a Rust concept called the "Token Cache" — L1 (in-memory) and L2 (Redis):

```
CredentialManager
    │
    ├── L1 Cache: in-memory LRU, per-node, ~5 min TTL
    ├── L2 Cache: Redis, shared across fleet, ~30 min TTL  ← production add
    ├── StateStore: PostgreSQL (primary) / DynamoDB (cloud) ← production add
    ├── DistributedLock: Redis/etcd for rotation           ← production add
    └── AuditLog: S3/Kafka for compliance                  ← production add
```

Current implementation has L1 cache only. The path to production requires
integrating nebula-storage for durable state and a distributed lock backend.

### Key gaps from current state to prod

| Gap | Priority | Notes |
|-----|---------|-------|
| Durable state store (Postgres/DynamoDB) | Critical | LocalStorage is single-node |
| Distributed lock for rotation (Redis/etcd) | High | Prevents double-rotation across fleet |
| L2 Redis cache | Medium | Reduces storage load on fleet |
| Audit log pipeline | High | Compliance requirement |
| KMS integration for encryption keys | High | Local key derivation is not prod |
| Token refresh background task | Medium | OAuth2 tokens expire; refresh proactively |

---

## Key Decisions

### D-001: Provider Abstraction (not extension trait)

**Decision**: `StorageProvider` is a trait object (`Arc<dyn StorageProvider>`),
not a generic parameter `CredentialManager<S: StorageProvider>`.

**Rationale**: A generic manager means the type signature leaks everywhere.
`CredentialManager<LocalStorageProvider>` and `CredentialManager<AwsProvider>`
are different types — you can't pass one where another is expected.
Trait objects (`dyn StorageProvider`) unify the type at the cost of one
vtable dispatch per operation — acceptable for storage I/O.

**Rejected**: Generic `<S: StorageProvider>` — export/import complexity across crates.

### D-002: Fail-Secure for Crypto Errors

**Decision**: `CryptoError::DecryptionFailed` is a terminal error with no
retry or fallback path. There is no "return partial data" path.

**Rejected**: Returning `None` on decryption failure (could be confused with "not found").
Returning the encrypted blob (caller might misuse it). Silent fallback to default.

### D-003: FlowProtocol over Ad-Hoc HTTP Redirects

**Decision**: Model interactive auth flows as a `FlowProtocol` state machine
rather than hardcoding OAuth2 redirect logic in the API layer.

**Rejected**: Hardcoded OAuth2 in the API layer (n8n pattern) — not extensible.
Separate "OAuth2 service" — unnecessary indirection for a pure auth flow.

### D-004: Rotation as RotationTransaction

**Decision**: Rotation is a typed `RotationTransaction` struct with explicit phases,
not an implicit "replace credential" operation in the manager.

**Rejected**: Simple "atomic replace" — loses grace period semantics.
Background task with no explicit phases — loses observability and rollback.

---

## Open Proposals

### P-001: Durable State Store via nebula-storage

**Problem**: `LocalStorageProvider` stores credentials as files on a single node.
Production needs a multi-node, durable, queryable store.

**Proposal**: Implement `StorageProvider` for a SQL-backed store using `nebula-storage`.
This requires `nebula-storage` Phase 2 (SQL backends) to be complete first.

**Impact**: No API changes. New `ProviderConfig` variant. Feature-gated.

### P-002: Proactive OAuth2 Token Refresh

**Problem**: OAuth2 access tokens have short TTLs (~1 hour). Actions that
run near token expiry may get stale tokens.

**Proposal**: Background refresh task spawned by `CredentialManager` when
a cached token is within 5 minutes of expiry. Token refreshed transparently.

**Dependency**: Requires `FlowProtocol::refresh` method and Tokio runtime.

### P-003: Composable Credential Type Registry

**Problem**: Each deployment needs different credential types. Today they're
registered statically. A plugin/registry system would let third-party drivers
register credential types dynamically.

**Proposal**: `CredentialTypeRegistry` that maps `type_id: String` to a
`Box<dyn FlowProtocol>`. `CredentialManager::list_types()` returns all registered.

---

## Non-Negotiables

1. **`SecretString` NEVER exposes raw secret** in `Debug`, `Display`, or any log
2. **Scope enforcement on every operation** — no bypass for "admin" or "internal" callers
3. **`CryptoError::DecryptionFailed` is terminal** — never retry, never return partial
4. **No cross-scope cache hits** — cache key MUST include scope
5. **`#![forbid(unsafe_code)]`** — enforced at lib root, always
6. **Rotation MUST support rollback** — never leave the system without a valid credential

---

## Governance

Amendments require:
- PATCH: wording — PR with explanation
- MINOR: new principle or non-negotiable — review with note in DECISIONS.md
- MAJOR: removing a non-negotiable, relaxing scope enforcement, or changing
  crypto primitives — full security review required

All PRs must verify:
- No raw secrets in error messages, logs, or panic messages
- Scope enforcement not bypassed
- Crypto primitives not weakened
