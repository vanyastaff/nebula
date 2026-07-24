---
name: nebula-credential
role: Typed credential contract, runtime, and authority-bound management
status: partial
last-reviewed: 2026-07-22
canon-invariants: [L2-12.5, L2-13.2]
related: [nebula-core, nebula-schema, nebula-storage-port, nebula-storage, nebula-resource]
---

# nebula-credential

## Purpose

Nebula credentials are typed lifecycle objects, not JSON blobs handed to action code. A
`Credential` separates:

- `Properties` — typed setup input validated without workflow-expression resolution;
- `State` — zeroizing runtime material that may be persisted only through the encrypted storage
  composition; and
- `Scheme` — the projected authentication capability a consumer receives.

The crate also owns credential resolution, refresh/lease coordination, cached handles, and the
management service/controller. It links neither SQL drivers nor general provider HTTP clients.
Integration authors consume its curated contracts through `nebula-sdk`; this crate is an
implementation boundary, not a second branded Rust product.

## Architecture

```text
authenticated API intent
        |
        v
CredentialController -- exactly one --> CredentialTenantAuthority
        |                                  allow / deny
        v
private one-use authorized command
        |
        v
CredentialService --> dyn CredentialPersistence --> storage decorators/backend
                         (nebula-storage-port)        (nebula-storage)
```

The controller accepts public intent but never accepts an owner key, selector, or proof from the
caller. After one authority decision it derives a mandatory `TenantScope`, privately creates a
non-cloneable/non-serializable authorized command, and consumes it in the same call. Absence of an
actor is never administrator authority; system provenance is denied until backed by a verified
durable record.

`CredentialPersistence`, `CredentialOwner`, typed `CredentialSelector`, structural
`StoredCredential`, lifecycle commands, and closed persistence errors are port-local types in
`nebula-storage-port`. This crate depends downward on that object-safe contract.
SQLite/PostgreSQL and the internal in-memory reference adapter, encryption/audit/cache decorators,
schema admission, and key providers live only in `nebula-storage`.

Every read, create, version-fenced replace/tombstone, existence check, cache key, and list is
owner-bound. Generic overwrite and ordinary hard delete do not exist. Metadata may carry a
compatibility/audit owner stamp, but storage never reads it as authority; the owner-qualified
selector and physical owner column are authoritative. Wrong-owner and missing rows are
intentionally indistinguishable.

## Main public contracts

### Type system

- `Credential` with `Properties`, `State`, and `Scheme` associated types.
- `CredentialState`, `AuthScheme`, and the sensitive/public/external scheme classifications.
- Capability sub-traits: `Interactive`, `Refreshable`, `Revocable`, `Testable`, and `Dynamic`.
- `CredentialRegistry` and `DispatchOps`; duplicate keys fail in debug and release.
- Built-in typed schemes and credentials used by first-party compositions.

Capabilities originate in trait membership. Registry bitflags are a derived discovery projection,
not a caller-supplied assertion.

### Runtime

- `CredentialResolver` for owner-bound, typed resolution and refresh-aware cached handles.
- `CredentialGuard`, `SchemeGuard`, and `SchemeFactory` for redacted, zeroizing access.
- Pending-state, refresh, lease, revocation, and provider contracts.
- `ValidatedCredentialBinding` for slot binding without caller-created tenant authority.

Resolver cache identity includes the full `CredentialSelector` and scheme `TypeId`; equal
credential IDs in different owner partitions cannot share a handle.

### Management

- `CredentialService` for semantic CRUD, acquisition, lifecycle operations, and slot resolution.
- `CredentialController`, `CredentialCommand`, and `CredentialCommandResult` for authenticated
  management calls.
- `CredentialTenantAuthority`, `CredentialActor`, `CredentialOperation`, and
  `AuthorizationDecision` for injected policy.
- `CredentialServiceError` with a non-empty, secret-safe `CredentialValidationReport` carrying only
  RFC 6901 paths and stable codes.

The service does not expose a store handle or an unscoped resolver. Runtime construction remains a
composition concern rather than an integration-author API.

## Property validation and secrecy

The supported authenticated HTTP mutation path uses one command/validation pipeline:

1. authorize the public management command once in `CredentialController`;
2. convert the credential's declared wire shape into `FieldValues` inside the service operation;
3. validate with `schema_of::<C::Properties>()`;
4. deserialize the canonicalized output into `C::Properties`; and
5. resolve/project typed state.

The API schema port is a catalog/form read model, not a second mutation validator. Its absence does
not block an otherwise wired credential command path.

The pipeline deliberately never calls `ValidValues::resolve` against a workflow expression
context. Secrets cannot depend on per-execution variables. Typed deserialization rejects surviving
expression envelopes as defense in depth.

Only structural path/code pairs cross the validation and public HTTP management boundaries.
Validator messages, parameters, submitted values, provider text, and source errors are discarded
by the controller/gateway mapping because custom validators or providers may echo secret material.
Internal technical `CredentialServiceError` values may still carry diagnostic strings; they are not
an SDK/HTTP error contract and must never be rendered into public responses or secret-bearing logs.

## Plane law

This crate models Plane-B integration credentials. Plane-A user login OAuth belongs to
`nebula-api` authentication. The public Plane-B HTTP surface is the universal credential command
and `resolve` / `resolve/continue` model; provider-specific browser ceremony routes remain parked.
The first-party registry does not advertise `OAuth2Credential` until its universal pending flow is
wired to a hardened injected transport.

## Invariants

- In the first-party secure composition, stored secret state is zeroized in process and encrypted at
  rest; no supported API/SDK debug bypass exists.
- Consumers receive projected schemes, not stored state or persistence rows.
- Supported authenticated HTTP management reaches persistence only after one authority decision for
  the exact command. Technical runtime/service seams remain below that supported boundary until K3.
- Owner identity is mandatory and selector-bound; there is no optional/global owner shortcut.
- Revocation and management deletion are the same terminal, version-fenced tombstone transition;
  a tombstone cannot carry secret bytes or other live-only fields and its id cannot be resurrected.
- Ready SQLite/PostgreSQL stores pass fail-closed schema admission and canonical migration before
  they can be constructed; unchecked raw pools are not a composition surface.
- Each refresh implementation receives one move-only `RefreshAttempt`. Provider dispatch consumes
  that witness; transport failure or an ambiguous/malformed response can produce only an
  outcome-unknown report and retains the durable claim. Only a pre-dispatch witness or a complete
  response proving no effect may release it. Coalescing and persistence outcomes remain
  framework-owned.
- Refresh authority is compared through the durable material epoch, never serialized-byte
  equality or the general row version. Display-only writes preserve the epoch and are merged before
  provider dispatch; explicit material/reconnect, durable reauthentication decisions, and
  successful refresh transitions advance it even when bytes are identical, and clear any retry
  gate.
- A known refresh outcome whose required retry-gate or reauthentication transition definitely
  failed is `RefreshFinalization`, distinct from an ambiguous `OutcomeUnknown`. Both retain the
  claim and are non-retryable. Payload-free L1 completion preserves that distinction for concurrent
  waiters: retry-unsafe completion becomes `RefreshReconciliationRequired`, while only an unknown
  completion becomes `OutcomeUnknown`.
- Credential properties never resolve workflow expressions.
- Durable business state never relies on `nebula-eventbus`; events are observations/wake hints.
- Provider-controlled strings never become public validation or HTTP error text.
- No raw writer, admin repository, runtime constructor, or unscoped resolver is exposed through the
  supported API/SDK. `CredentialPersistence` and construction seams remain unsupported technical
  contracts for trusted workspace composition.

## Features and checks

- `rotation` gates evolving rotation support.
- `cargo nextest run -p nebula-credential`
- `cargo test -p nebula-credential --doc`
- Compile-fail suites under `tests/compile_fail_*` lock down capability, sensitivity, guard, and
  slot invariants.

## Known limits

- Universal first-party interactive OAuth acquisition remains parked pending the universal
  acquisition and authority flow.
- Proactive pre-expiry refresh and some rotation behavior remain evolving.
- Production composition (key policy, persistence, catalog, refresh transport, and authority)
  lives in `apps/server`. API fixtures inject a deterministic no-network refresh adapter; they do
  not carry a second HTTP implementation. The first-party refresh transport enforces
  rustls/HTTPS-only, no redirects/retries/implicit proxies, and connect-time all-answer
  global-unicast DNS validation. A downstream implementation of the technical `RefreshTransport`
  seam must enforce the same connect policy; only the first-party adapter is mechanically covered
  by this workspace's DNS/redirect/proxy behavior tests. The first-party server binds claims to
  the admitted credential SQLite pool, uses a process-unique replica ID, and retains the sole
  periodic poison-accounting sweep for its complete serving lifecycle.
- A lost database acknowledgement after commit is `OutcomeUnknown`. K2 never replays it
  automatically. An expired provider-side-effect claim remains durable fail-closed poison, and
  elapsed TTL never grants replay authority. Explicit reconciliation and safe operation
  replay/idempotency belong to K3.
- Interim audit is non-authoritative observation. It cannot make a confirmed mutation fail;
  atomic audit/outbox evidence belongs to K3.
- Tombstoning clears current live material but does not claim historical erasure from database
  WAL, snapshots, or backups; retention/key-destruction policy is a separate operator concern.
- K3 must make the controller plus semantic idempotency/operation ledger the sole management writer;
  K1 intentionally proves only the authenticated HTTP command path.
- K4 must provide supported workspace-directory and membership/deployment composition. The default
  server leaves both policy ports unwired, so tenant routes return 503.

## Related

- Root `AGENTS.md` for layer and supported-surface invariants.
- `docs/PRODUCT_CANON.md` §12.5 and §13.2.
- `docs/INTEGRATION_MODEL.md` for the Credential/Resource/Action connection model.
- `crates/storage-port/README.md` and `crates/storage/README.md` for persistence contracts and
  adapters.
