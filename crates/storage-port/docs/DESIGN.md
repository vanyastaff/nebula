# nebula-storage-port — current design

| Field | Value |
|---|---|
| Status | Current K1 contract; pre-1.0 |
| Reviewed | 2026-07-22 |
| Layer | Core contract; no backend code |
| Related | ADR-0072, ADR-0041, ADR-0088, ADR-0092, product canon §11–§12 |

## Purpose

`nebula-storage-port` owns the object-safe persistence contracts and port-local data that product
crates use without importing SQL drivers, migrations, pools, or backend implementations. It is a
technical workspace boundary, not a supported integration product; `nebula-sdk` remains the sole
supported and branded Rust surface.

The crate owns:

- object-safe `#[async_trait]` repository traits segregated by role;
- port-local DTO rows with no dependency on higher-tier domain types;
- plain-data `Scope { workspace_id, org_id }` with no policy;
- `StorageError`, `FencingToken`, and the builder-only `TransitionBatch` atomic unit of work; and
- the K1 `CredentialPersistence` contract plus owner/selector/row/error DTOs.

It deliberately owns no SQL, migrations, connection pools, encryption implementation, cache,
audit sink, tenant-policy resolution, API authentication, or deployment configuration.

## Dependency direction

```text
engine / credential / api / apps
                 │
                 ▼
       nebula-storage-port       (contracts and DTOs)
                 ▲
                 │ implements
          nebula-storage         (SQLite/PostgreSQL; internal reference adapters)

nebula-tenancy wraps the Scope-taking non-credential ports.
Credential persistence is owner-bound directly and is not wrapped by tenancy.
```

Direct downward imports of these technical contracts are expected inside the workspace. They do
not promote storage-port into a supported downstream API. `apps/server` depends on the port
directly because first-party composition roots select concrete adapters and object-safe handles.

## Public contract map

### Shared persistence primitives

- `Scope` is data only. `BindingScopeResolver` and request tenant policy live in
  `nebula-tenancy`.
- `TransitionBatch` structurally requires execution scope, CAS precondition, and fencing before
  `ExecutionStore::commit` can atomically apply state, outbox, and journal changes.
- `FencingToken` prevents a stale lease holder from committing after ownership changes.
- `StorageError` is the general storage-port error. Credential persistence has a deliberately
  separate, secret-safe `CredentialPersistenceError` contract.

### Repository families

`src/store/` contains the execution/workflow/control-queue/journal/checkpoint/node-result,
idempotency, identity, webhook, trigger-dedup, dispatch/resume, refresh-claim, and credential role
traits. `src/dto/` contains their port-local rows. Exact exports are defined by `src/lib.rs` and
`src/store/mod.rs`; this document describes ownership rather than duplicating a symbol inventory.

All repository traits remain directly dyn-compatible and are consumed as `Arc<dyn …>`. Adding a
generic/RPITIT-only method without an object-safe alternative is an architectural break.

## K1 credential persistence

`CredentialPersistence` is the one object-safe credential persistence contract. The retired
credential-local store/RPITIT bridge and tenancy metadata decorator have no compatibility aliases.
The port exposes:

- `CredentialOwner` — one mandatory canonical owner partition;
- `CredentialSelector` — `(owner, credential_id)` for every row operation;
- `CredentialWriteMode` — create-only, overwrite, or compare-and-swap;
- `StoredCredential` and `StoredCredentialHead` — opaque row/full and secret-free projection DTOs;
- `CredentialPersistenceError` — redacted port diagnostics; and
- `CredentialPersistence` — `get`, `get_head`, `put`, `delete`, `list`, `list_heads`, and `exists`.

Owner and selector values are data, not authorization proofs. Their constructors are public because
trusted technical runtime/storage code must carry them across crate boundaries. They are absent
from `nebula-sdk` and must never be accepted from an HTTP request or treated as evidence that an
actor may access a tenant.

The supported authenticated HTTP management path obtains one
`CredentialTenantAuthority` decision in `CredentialController` before deriving its owner-bound
command. That policy belongs to `nebula-credential` plus the apps-owned trust bridge, not this port.
Technical service/runtime paths still exist below that boundary; K3 will make the controller and
operation ledger the sole semantic management writer.

Every credential adapter must obey these laws:

1. per-row reads, writes, deletes, existence checks, and CAS include owner plus credential ID;
2. list operations require exactly one owner;
3. wrong-owner access is indistinguishable from absence;
4. owner never changes during update;
5. metadata owner stamps are compatibility/audit data overwritten from the selector, never an
   authorization source; and
6. backend/audit-controlled diagnostic text is redacted at the port and service mapping boundaries.

`nebula-storage` is the sole implementation owner: SQLite, PostgreSQL, internal in-memory
reference/conformance adapters, and audit/encryption/cache decorators implement this contract.
`nebula-tenancy` intentionally does not implement a credential decorator.

## General tenant isolation

For ordinary `Scope`-taking stores, `nebula-tenancy` resolves an authenticated binding and exposes
scope-substituting decorators. Callers receive a handle already bound to a tenant and cannot swap a
different request scope. Backends still include workspace and organization columns in predicates so
wrong-scope and missing rows share one observable result.

This rule must not be generalized to credential persistence: its mandatory `CredentialOwner` and
`CredentialSelector` contract is a separate owner-isolation mechanism, with authorization decided
above the port.

## Backend and composition ownership

- `nebula-storage` owns all backend code, schema migrations, credential encryption/cache/audit
  decorators, and deployment-backed SQLite/PostgreSQL implementations.
- In-memory adapters are internal test/reference/conformance implementations, not supported
  deployment backends.
- `nebula-tenancy` owns principal-to-`Scope` policy and decorators for the enumerated general
  Scope-taking stores.
- `apps/*` are the first-party deployment composition roots. `nebula-api` is a technical HTTP
  boundary, not a supported downstream composition product.

## Verification

- Object-safety probes ensure the role traits can be held behind `Arc<dyn …>`.
- `TransitionBatch` tests protect builder-only scope/CAS/fencing construction.
- Credential persistence conformance covers owner-bound reads/lists/deletes, metadata spoof
  rejection, secret-redacted diagnostics, and adapter/decorator equivalence.
- SQLite and internal reference tests are local evidence only. Live PostgreSQL execution remains a
  release gate and skip-clean tests are not that proof.

## Explicit remaining work

- **K2 — schema ownership migration.** The historical SQLite and PostgreSQL `owner_id` columns are
  still nullable. Add deployment upgrade migrations for both backends, then run live PostgreSQL
  owner/concurrency conformance. Until then mandatory owner is a port/application invariant; `NULL`
  grants no global or administrator access. Applied migration `0030_credentials_store.sql` is
  SQLx-checksummed immutable history, including its legacy comment; K2 adds a new migration rather
  than editing `0030`. K2 must also make SQL write outcomes linearizable: the current adapters
  read after commit and calculate overwrite versions outside a fenced mutation, so the returned
  row is not yet a commit receipt under concurrent writers.
- **K3 — semantic writer closure.** Make the authority-bound controller plus semantic
  idempotency/operation ledger the sole credential management writer and add durable convergence
  contracts without moving authority into this port.
- **K4 — supported composition.** Provide the apps-owned durable membership bridge/operator
  configuration, and expose only curated client and embedded SDK façades. Production credential
  adapters already live in `apps/server`; API-side construction helpers are test-only.
- Independently, typed IDs in several older store signatures and the refresh-claim ownership/error
  shape remain pre-1.0 design debt; changes require a coordinated breaking wave across consumers and
  adapters.

## Invariants

- No backend or policy implementation enters this Core-tier crate.
- No port DTO depends on credential/action/API domain types.
- Object safety is preserved for every repository role.
- Durable execution transitions retain atomic state/outbox/journal plus CAS/fencing.
- Credential selectors are mandatory but never treated as actor authority.
- Lossy event-bus observations never replace persisted commands or business facts.
