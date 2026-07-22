# nebula-credential — design

| Field | Value |
|---|---|
| Status | Current implementation boundary; pre-1.0 |
| Reviewed | 2026-07-22 |
| Layer | Core/shared infrastructure |

## Bounded contexts

The crate contains three tightly coupled contexts that share the typed credential contract:

1. **Type system** — `Credential`, typed properties/state/scheme, capability traits, registry.
2. **Runtime** — resolve/project, refresh, lease, pending-state, cached typed handles.
3. **Management** — semantic service plus the authority-bound command controller.

SQL backends and persistence decorators are not a fourth context here. The object-safe contract is
`nebula_storage_port::CredentialPersistence`; implementations are owned exclusively by
`nebula-storage`.

## Command authority

```mermaid
sequenceDiagram
    participant H as API handler
    participant G as API command gateway
    participant C as CredentialController
    participant A as CredentialTenantAuthority
    participant S as CredentialService
    participant P as CredentialPersistence

    H->>G: authenticated principal + resolved Scope + public intent
    G->>C: CredentialActor + Scope + CredentialCommand
    C->>A: decide(actor, scope, operation)
    A-->>C: one Allow/Deny decision
    alt Allow
        C->>C: derive TenantScope and mint private one-use command
        C->>S: consume authorized command
        S->>P: owner-bound selector/list owner
    else Deny/error
        C-->>G: closed authorization error; no service call
    end
```

`AuthorizedCredentialCommand` is private, non-cloneable, and non-serializable. Public controller
commands contain intent only and accept no storage selector, owner key, tenant proof, raw writer, or
optional system actor. This is the supported authenticated HTTP management path, not yet a claim
that every technical `CredentialService`/runtime call is forced through the controller; that
sole-semantic-writer closure is K3 work.

The first-party trust bridge lives in `apps/server`: it converts the API's private-field
`AuthenticatedPrincipal` into typed credential actor claims, re-reads one consistent role snapshot
from the same membership source used by HTTP RBAC, applies the operation's credential permission,
and asks the tenancy resolver to reproduce the exact org/workspace scope. An unwired or failed
membership source returns unavailable; a valid snapshot with no organization membership denies.
Workflow/system actors fail closed until durable provenance policy is implemented. The route's
Access Kernel guard remains responsible for the separate token-grant check.

## Persistence boundary

`CredentialSelector` is `(CredentialOwner, credential_id)` with private fields and accessors.
`CredentialOwner` is mandatory. All persistence methods are object-safe and owner-bound:

- `get`, `put`, `delete`, and `exists` take a selector;
- `list` takes an owner;
- CAS compares owner, credential ID, and expected version;
- owner is never updated; and
- wrong-owner access has the same observable result as absence.

`StoredCredential` is a port DTO with redacted `Debug` and no serde contract. Its owner metadata
stamp is compatibility/audit information only; adapters overwrite it from the selector and never
use it as authorization input.

The resolver cache key includes `CredentialSelector` and output `TypeId`. Encryption retains the
credential-ID AAD format for existing ciphertext compatibility; owner isolation is enforced by the
database predicate and cache identity. Any future AAD migration must be ledgered and cannot be
silently mixed with ordinary reads.

## Validation boundary

Schema validation occurs exactly once inside the authorized service operation. The API schema port
is catalog/form-read-only and its absence never blocks a mutation. A rejected report is converted
to a non-empty `CredentialValidationReport` whose issues contain only:

- a canonical RFC 6901 pointer; and
- a stable machine-readable code.

Messages, params, input values, provider strings, and source errors never cross the validation
report or public HTTP gateway. The API maps codes to API-owned static text. Internal technical
service errors may retain diagnostics and must be collapsed before that boundary. This preserves
actionable field UX without performing an owner lookup or second authorization decision in the
handler.

## Integration boundary

`nebula-sdk` is the sole supported Rust surface. The external perimeter fixture compiles the
currently verified manual/builder subset (`ActionBuilder`, `WorkflowBuilder`, and credential
`TestResult`) with only `nebula-sdk` and separately proves forbidden authority, owner, writer,
repository, constructor, and unscoped-resolver paths are absent. Procedural derives still emit
leaf-crate paths and remain an explicit SDK gap rather than an implied direct-dependency escape.

## Non-goals

- No SQL/backend implementation, general HTTP client, or deployment configuration.
- No `None == admin`, string-made owner authority, metadata-only tenant enforcement, or post-read
  tenant check.
- No raw service/store handle for API handlers or integration authors. Public technical port and
  construction seams are unsupported workspace contracts, not SDK products.
- No provider-specific public OAuth ceremony while the universal pending transport is parked.
- No durable command/fact delivery over the lossy event bus.

## Remaining design work

- **K2:** add deployment upgrade migrations for the still-nullable SQLite and PostgreSQL owner
  columns and live-verify PostgreSQL owner/concurrency conformance.
- **K3:** make the controller plus semantic idempotency/operation ledger the sole management writer;
  add versioned state envelopes and durable cross-aggregate convergence.
- **K4:** provide supported membership/deployment wiring and finish curated SDK
  `client`/`embedded` façades without exposing internal authority. Production credential adapters
  already live in `apps/server`; the API-side factory is an unsupported test fixture only.
