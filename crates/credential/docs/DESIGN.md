# nebula-credential — design

| Field | Value |
|---|---|
| Status | Current implementation boundary; pre-1.0 |
| Reviewed | 2026-07-23 |
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

`CredentialSelector` is `(CredentialOwner, CredentialId)` with private fields and accessors.
`CredentialOwner` is mandatory. All persistence methods are object-safe and owner-bound:

- physical `get`, live-only `get_head`/`exists`, and explicit `create`/`replace`/`tombstone` take
  a selector;
- `list`/`list_heads` take an owner and expose live rows only;
- replace and tombstone compare owner, typed credential ID, live state, and expected
  `CredentialVersion`;
- create cannot smuggle identity/owner/version/timestamps, replacement cannot change immutable
  identity/type/creation fields, and tombstone carries only its expected version;
- owner is never updated; wrong-owner access has the same observable result as absence; and
- generic overwrite and physical delete are unnameable.

`StoredCredential` is structural `Live | Tombstoned`, redacted, and has no serde contract. The
tombstone payload cannot represent data, name, expiry, reauthentication, or metadata. Its id
remains permanently reserved while its owner-local name is released. Owner metadata is ordinary
compatibility/audit data only; the selector plus physical owner column are the sole persistence
authority.

SQLite/PostgreSQL ready-store constructors hold a bounded backend-specific startup lock across
read-only schema admission, canonical migration through paired `0040`, and postflight. Raw pools
cannot construct a ready credential store. Confirmed mutations return a secret-free
`CredentialCommit` from statement `RETURNING` only after commit; a lost commit acknowledgement is
the non-retryable `OutcomeUnknown`.

Coordinated refresh has an explicit irreversible boundary: the L2 claim is marked
`RefreshInFlight` before provider dispatch, then the provider call, state encoding, and credential
replacement run in an owned task. Caller cancellation, caller-wait timeout, and heartbeat loss do
not cancel that task or release L1/L2 early. A lost commit acknowledgement, or a definite
post-provider encoding/persistence failure, stops heartbeat but retains the sentinel claim.
The credential implementation receives one move-only `RefreshAttempt`: consuming it through
`dispatch` destroys the pre-dispatch witness, a failed dispatch yields only outcome-unknown
evidence, and only a complete-response proof can classify a provider rejection or prove no effect.
Providerless local completion is explicit. Coalescing, persistence, and claim disposition are
framework-owned and cannot be synthesized by an integration.
After TTL, storage keeps an expired `RefreshInFlight` row as durable fail-closed poison:
`try_claim` returns `OutcomeUnknown`, provider dispatch remains forbidden, and the reclaim sweep
atomically records evidence without deleting the row. Provider transport/read failure, a malformed
successful response, and an opaque integration error are likewise
`OutcomeUnknown`: dispatch began, so lack of a complete acknowledgement cannot prove the rotating
grant survived. Exact `invalid_grant` is instead persisted as `reauth_required`; missing local
refresh material is classified separately and performs no transport dispatch. Ambiguous and
post-provider outcomes are non-retryable to the originating caller. This is storm containment, not
provider-side exactly-once: explicit, authorized reconciliation of poisoned operations remains K3
work, and elapsed time alone never grants replay authority. A live critical task with no exact
disposition deliberately keeps heartbeating fail-closed;
cancelling it cannot prove the provider did not consume the grant.

The authenticated management `refresh` and `revoke` commands use the same owned L1/L2 boundary.
Their erased integration closures are invoked at most once: an opaque error after entry is
`OutcomeUnknown`, while a definite local encode/CAS/tombstone failure after provider success is
operation-specific `RefreshPostProviderPersistence` or
`RevokePostProviderPersistence` at the service boundary (the integration-facing
`CredentialError::PostProviderPersistence` remains operation-generic).
Concurrent callers coalesce, re-check the observed refresh-authority epoch or
revoke CAS version as appropriate, and never repeat provider work merely
because the first caller disconnected. The payload-free L1 signal preserves
exactness: a definite retry-unsafe winner yields an operation-specific
reconciliation error to waiters, while only a genuinely unknown or abnormal
completion yields `OutcomeUnknown`. Durable reauthentication advances the
material epoch and clears old retry evidence, so a stale gate finalizer cannot
reattach to a rejected grant. Refresh fallback to still-valid material is
permitted only for coordination failures proven to occur before provider
dispatch.

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

- **K3:** make the controller plus semantic idempotency/operation ledger the sole management writer;
  add an explicit reconciliation command that can resolve durable `OutcomeUnknown` poison and
  authorize safe replay when evidence permits, plus transactional audit/outbox evidence, versioned
  state envelopes, and durable cross-aggregate convergence.
- **K4:** provide supported membership/deployment wiring and finish curated SDK
  `client`/`embedded` façades without exposing internal authority. Production credential adapters
  already live in `apps/server`; the API-side factory is an unsupported test fixture only.
