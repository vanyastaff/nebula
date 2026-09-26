# nebula-credential — design

| Field | Value |
|---|---|
| Status | Current implementation boundary; pre-1.0 |
| Reviewed | 2026-09-23 |
| Layer | Core/shared infrastructure |

## Bounded contexts

The crate contains three tightly coupled contexts that share the typed credential contract:

1. **Type system** — `Credential`, typed properties/state/scheme, capability traits, registry.
2. **Runtime** — resolve/project, refresh, lease, pending-state, cached typed handles.
3. **Management** — semantic service plus the authority-bound command controller.

The read-only worker runtime and shared slot projection live in `src/runtime/projection/`.
`src/service/slot.rs` owns service-specific adapters and binding validation. Shared tenant
identity lives in `src/scope.rs`, and material-source selection in `src/runtime/state_source.rs`.
Crate-root re-exports remain the canonical imports across these internal boundaries.

SQL backends and persistence decorators are not a fourth context here. The object-safe contract is
`nebula_storage_port::CredentialPersistence`; implementations are owned exclusively by
`nebula-storage`.

## Command authority

Durable provider operations retain their own operation kind. Refresh and revoke still compete
for the same aggregate-scoped claim, so they cannot run side effects concurrently. Revoke pins
the observed material epoch and uses an operation-specific reconciliation decision: confirmed
revocation commits a tombstone, evidence, and claim removal together. Clearing a claim alone
is not a successful revoke recovery. New material use consults the same persisted operation
state independently of refresh capability or local material expiry.

The public availability projection contains an operation category and in-flight/reconciliation
state; claim authority stays in the runtime/storage seam. The SQL operational-head read joins
aggregate and claim state in one snapshot. Historical untyped incidents remain explicitly
unclassified, and cannot be interpreted as refresh by a default value.

This boundary advances credential aggregate durability in canon §12.2 and §13.2. It does not
implement acquisition command receipts, provider idempotency, cross-replica resource-pool
invalidation, or the supported embedded composition root.

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
optional system actor. Service management methods (`create`, `update`, `delete`, `test`, `refresh`,
`revoke`, `resolve`, `continue_resolve`) are crate-private, so external service callers must submit
`CredentialCommand` through the controller. Public technical reads, binding validation, and slot
projection remain available. The `compile_fail_service_mutations_private` suite checks this
visibility boundary; controller tests separately exercise authorization and command dispatch.
Lower-level runtime and persistence contracts remain technical composition seams. Closing service
visibility does not add operation-ledger idempotency, transactional audit/outbox, or the global
sole-semantic-writer guarantee; those remain K3 work.

The first-party trust bridge lives in `apps/server`: it converts the API's private-field
`AuthenticatedPrincipal` into typed credential actor claims, re-reads one consistent role snapshot
from the same membership source used by HTTP RBAC, applies the operation's credential permission,
and asks the tenancy resolver to reproduce the exact org/workspace scope. An unwired or failed
membership source returns unavailable; a valid snapshot with no organization membership denies.
Workflow/system actors fail closed until durable provenance policy is implemented. The route's
Access Kernel guard remains responsible for the separate token-grant check.

### Reconciliation seam

`CredentialCommand::Reconcile` resolves one *poisoned* provider-operation claim (an expired `sentinel=1` row
that `try_claim` answers as outcome-unknown) with the provider outcome an operator has established
off-platform. It carries the credential, the incident it resolves (`CredentialIncidentRef`, as
published in the `ReconciliationRequired` lifecycle state), a typed `CredentialOperationDecision`,
and an operator note. The incident is the anchor for retries: a credential poisons again after its
first incident is resolved, and a decision resent after a lost acknowledgement must reach the
incident it was made for. The adjudicator answers a resolved incident from its own record first,
resolves the named incident only when it is the current poisoned claim, and refuses any other
poisoned claim as `StaleIncident`. The incident identity is the expired claim's UUID; its fencing
generation is never exposed.

The controller holds the seam as a constructor dependency, `Arc<dyn RefreshClaimAdjudicator>`
(`nebula-storage-port`), beside its optional `Arc<dyn AuditSink>`. Neither is a service method:
reconciliation owns the incident transaction. For `ProviderRevoked`, that transaction also
tombstones the aggregate after checking the material epoch captured by the revoke. The caller
cannot clear the claim separately from that terminal transition.

Authority is layered, and the layers have different standing:

- the **adjudicator's incident row** is the authoritative, transactional record — clearing the
  poison, writing the resolution, and any confirmed-revoke tombstone commit together;
- the **`AuditOperation::Reconcile` event** is a non-authoritative observation emitted after that
  commit: its failure is logged and never converts a recorded decision into an error the caller
  could retry against different evidence. The `nebula.credential.reconcile_total` counter
  (labelled `outcome`) is never emitted, because no metrics emitter is wired at the composition
  root and the seam's only implementor discards every sample, so it has no failure to log;
- the **tenant gate** is the controller's own `CredentialTenantAuthority` decision, reached through
  `CredentialOperation::Reconcile` → `Permission::CredentialReconcile` → `credentials:reconcile`.
  The adjudication port takes no `Scope` operand, so it cannot be scope-keyed and no decorator can
  stand in for this gate.

Reconciliation is deliberately **not** guarded by `CredentialWrite`: that permission would hand the
privileged seam to every credential writer. It sits at the administrator tier with the other
privileged operations: `CredentialReconcile` maps to `WorkspaceRole::WorkspaceAdmin`, and
`TenantContext::require` checks the workspace role before the org branch, so this permission never
reaches an org gate. An org admin passes the workspace gate by implication, since `OrgAdmin` and
`OrgOwner` imply `WorkspaceAdmin` in every workspace.

Repeating an identical `(evidence digest, decision)` pair for the same incident is a success with
`changed: false`, not a conflict: it is the idempotent recommit of a lost acknowledgement. Different
evidence or a different decision for an already-resolved incident is `EvidenceConflict` — reconciliation resolves an
unknown outcome, it does not overrule a recorded one.

### Use admission

Every path that hands material to a consumer (the resolver and slot projection
for resources and actions) decides a new use through one credential-owned
classification (`runtime::availability::classify_use`), so the paths cannot
disagree on what a durable operation means:

| Operation status | New use |
|---|---|
| open | admitted |
| open, reauthorization required | denied (`ReauthRequired`) |
| refresh crossing the provider boundary | joined for up to five seconds, then busy (`RefreshInFlight { retry_after }` on the slot surface) |
| revoke, or a legacy unclassified operation, in flight | denied (`OperationBlocked`) |
| an operation awaiting reconciliation | denied (`OperationBlocked`) until adjudicated |

Uses already admitted keep the material they were handed. A refresh in flight
is transient: resource activation keeps the registration that serves the
credential instead of retiring it.

## Persistence boundary

`CredentialSelector` is `(CredentialOwner, CredentialId)` with private fields and accessors.
`CredentialOwner` is mandatory. All persistence methods are object-safe and owner-bound:

- physical `get`, live-only `get_head`/`get_operational_head`/`operation_status`/`exists`, and explicit `create`/`replace`/`tombstone` take
  a selector;
- `list`/`list_heads`/`list_operational_heads` take an owner and expose live rows only;
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

SQLite/PostgreSQL ready-store constructors hold backend-specific startup serialization across
read-only schema admission, the full ordered migration catalog (currently through paired `0057`),
and postflight. PostgreSQL lock acquisition/release and SQLite file-lock waiting are bounded;
migration duration follows the caller lifecycle and the operator's database timeout. Raw pools
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
atomically records evidence without deleting the row. The owner-qualified reclaimer evaluates the
sentinel window in that same transaction; crossing the threshold advances revision and material
epoch, clears stale retry evidence, and commits `reauth_required`. Metrics, audit, and EventBus
notifications consume the committed result and are never the source of truth. Provider
transport/read failure, a malformed
successful response, and an opaque integration error are likewise
`OutcomeUnknown`: dispatch began, so lack of a complete acknowledgement cannot prove the rotating
grant survived. Exact `invalid_grant` is instead persisted as `reauth_required`; missing local
refresh material is classified separately and performs no transport dispatch. Ambiguous and
post-provider outcomes are non-retryable to the originating caller. This is storm containment, not
provider-side exactly-once: explicit, authorized reconciliation of poisoned operations now ships as
the owner-qualified credential reconcile command, and elapsed time alone never grants replay
authority. A live critical task with no exact disposition deliberately keeps heartbeating
fail-closed; cancelling it cannot prove the provider did not consume the grant.

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

Runtime registration admits the schema-free `Credential::metadata()` draft with the
fallible schema derived from `C::Properties` before it installs the credential's
operation closures. `Properties` describes authored data; it is not a runtime proof.
Untyped mocks may use `serde_json::Value`, while concrete credentials declare the
actual schema-bearing properties type.

The authorized service operation owns property preparation and proof construction:
schema-directed `values_from_wire` decodes literal `AuthoredValue`, consuming
`validate` canonicalizes aliases, applies transforms once, and promotes secrets,
then `resolve_data` produces schema-bound values after full checks.
It never creates an expression engine. Template-looking property strings remain
literal data; executable expression nodes cannot pass the data-only transition.

The operation consumes that value once with
`into_typed_exposing_secrets::<C::Properties>()` at a trusted disclosure boundary
and dispatches `&C::Properties`; it does not reconstruct proof from raw JSON or a
different schema. Built-in secret fields are zeroizing `SecretString` values by
the time provider code runs.
Ordinary `into_typed` rejects secret-bearing input, and typed-decoding diagnostics
hide input material throughout the public error source chain.

Required-field, type, and typed-property failures occur before provider dispatch.
Protocol-specific constraints not expressed by the schema remain provider checks,
such as OAuth's grant-dependent redirect URI requirement. Tests must exercise the
actual credential properties schema and preserve rejections at the boundary that
owns them, rather than manufacture a proof or weaken a required declaration.

The API schema port is catalog/form-read-only and its absence never blocks a
mutation. A rejected report is converted to a non-empty
`CredentialValidationReport` whose issues contain only:

- a canonical RFC 6901 pointer; and
- a stable machine-readable code.

Messages, params, input values, provider strings, and source errors never cross the validation
report or public HTTP gateway. The API maps codes to API-owned static text. Internal technical
service errors may retain diagnostics and must be collapsed before that boundary. This preserves
actionable field UX without performing an owner lookup or second authorization decision in the
handler.

## Integration boundary

Execution-time slot resolution crosses one object-safe `CredentialSlotResolver` boundary. The
resolver first performs an owner-qualified `get_head`, so missing and cross-tenant identifiers are
indistinguishable and key/capability rejection occurs before the encryption layer decrypts state.
Only after those checks pass does it load material, re-check the id, key, revision, material epoch,
state kind/version, and reauthentication bit against the validated head, then dispatch the
monomorphized projection registered in `DispatchOps`. Callers receive an opaque
`ErasedCredentialGuard`, never `CredentialSnapshot` or raw `Any`; checked typed extraction is the
only way to recover `CredentialGuard<S>`. Its secret-free metadata carries both material epoch and
aggregate revision so resource slots can reject stale replacement attempts.

The metadata also carries the admission epoch — the contract's use revision — read in the same
snapshot as the material, never from the earlier head. The backend advances that epoch in the same
transaction as every write that closes use: every advancing replacement (new material, and the
durable reauthentication decision, which advances authority over unchanged bytes), any change of
`reauth_required`, a won revoke claim, the provider-egress sentinel, and threshold escalation.
Display edits and retry-gate writes preserve it, and none of these writes moves the aggregate
revision. Two `Open` observations at equal admission epochs therefore saw the same material epoch
and reauthentication state with no denying observation between them (invariant I-A), which is what
lets a consumer that bound a projection at one epoch treat any other epoch as "old use revision
does not admit". Only the SQL backends provide the claim-side bumps; the in-memory claim repository
cannot. Consumer wiring (observer, activation, fan-out) is a follow-up.

Replacement carries material only on `Advance { material: MaterialUpdate::Replace(..) }`. A
display edit or retry-gate write sends `Preserve` and the reauthentication decision sends
`Advance { Unchanged }`; neither carries bytes, so neither can restore stale material captured at
load time, and the encryption layer re-seals only newly installed material.

`CredentialProjectionRuntime::from_secure_parts` is the worker composition surface for that
boundary. It accepts only an already-secured `CredentialPersistence`, `CredentialRegistry`,
`DispatchOps`, and `StateSource`. Construction proves every registered key has a base projector and
that every advertised capability has a matching operation closure. It deliberately cannot own or
construct refresh coordination, lease lifecycle, reclaim tasks, pending acquisition state, or
management command authority; those remain in the server management runtime.

`nebula-sdk` is the sole supported Rust surface. Its credential authoring contract exposes
`Credential`, `CredentialMetadataDraft`, typed `Properties`, the `#[credential]` impl
macro and `AuthScheme` derive, the shared `Icon`/maturity/deprecation vocabulary, built-in credential types, typed
snapshots/context, and universal OAuth types through the SDK prelude. `integration::credential`
exposes resolve and credential-test outcomes. SDK-only external fixtures compile and execute
representative credential derives, while perimeter fixtures prove that owner authority, raw
persistence, runtime constructors, credential records, and unscoped resolvers remain unavailable.

The Phase-5 property design keeps explicit `Credential::Properties` canonical, with
value-only `#[property(...)]` fields. Consumer `#[slot(credential, ...)]` dependencies stay
outside properties. Static resolve/project and existing capability traits remain unchanged; the
shared metadata authoring foundation is tracked in the
[integration model](../../../docs/INTEGRATION_MODEL.md#shared-metadata-authoring-current-foundation).

## Non-goals

- No SQL/backend implementation, general HTTP client, or deployment configuration.
- No `None == admin`, string-made owner authority, metadata-only tenant enforcement, or post-read
  tenant check.
- No raw service/store handle for API handlers or integration authors. Public technical port and
  construction seams are unsupported workspace contracts, not SDK products.
- No provider-specific public OAuth ceremony outside the universal typed resolve/begin/continue
  pending protocol.
- No durable command/fact delivery over the lossy event bus.

## Remaining design work

- **K3 closure:** owner-qualified reconciliation, the persisted-state version envelope, and
  crate-private service management methods are implemented. External service management callers
  now enter through the controller, but a global sole semantic writer plus operation-ledger
  idempotency is not yet structurally enforced. Transactional audit/outbox evidence and durable
  cross-aggregate convergence also remain open. Sentinel threshold escalation now commits within
  the credential aggregate's owner-qualified storage transaction.
- **K4:** first-party membership and workspace-directory wiring is implemented in `apps/server`.
  SQLite and PostgreSQL provide durable tenant authority after explicit operator bootstrap; memory
  is a process-local development/test profile. Finish the curated SDK `client`/`embedded` façades
  without exposing internal authority. Production credential adapters live in `apps/server`; the
  API-side factory is an unsupported test fixture only.

### ADR-0088 status, updated 2026-09-22

Each item carries the ADR's own state as of 2026-06-12, then what the tree shows now.

- **D1 — ADR: "partial"; `#[credential]` attr + `CredentialPolicy` shipped, OAuth2 still a
  monolithic type, no shared `OAuth2Protocol` module yet.** Now: the derive half is complete in a
  second sense the ADR did not record, because the `#[credential]` macro synthesizes
  `CredentialLifecycle::policy` (`crates/credential/macros/src/credential_attr.rs`). The residue the
  ADR names is unchanged: no `OAuth2Protocol` module, and `OAuth2Credential` remains one `impl` block
  that hand-writes `policy` because its strategy is state-dependent.
- **D2 — ADR: "partial"; "runtime does not yet route policy-first in all paths".** Now: it routes
  policy-first in **no** production path, so the ADR's hedge understates the gap. `C::policy` has one
  production-code call site (`src/runtime/resolver/mod.rs`), and no in-repo path reaches it: the sole
  caller of `CredentialResolver::resolve_with_refresh` is `CredentialResolver::scheme_factory`.
  The unused `CredentialService::scheme_factory` wrapper has been removed; the low-level resolver
  APIs remain technical APIs with tests, without production slot-refresh wiring. The capability
  sub-traits plus the durable `reauth_required` bit are the gate (`src/runtime/projection/slot.rs`).
  **Recorded as a deliberate cut, not pending wiring:** the production seam is type-erased
  (`CredentialSlotResolver::resolve_slot` returns an `ErasedCredentialGuard`), so routing a policy
  through it needs a new erased port and red-to-green evidence, which is a design change rather than
  a documentation one. Capability traits remain the governing production model; the future
  supported role of the low-level policy-driven resolver requires a separate design decision.
  The authoring
  obligation survives the cut: an author must still hand-write `fn policy` where the synthesized
  value would be wrong.
- **D6 — ADR: "seam exists, producer is a frontier gap (M12.4)".** Now: split. The credential
  slot-resolution path landed 2026-09-13 with `CredentialSlotResolver` (now in `src/runtime/projection/slot.rs`).
  The crate carries two impls of it, and the one the engine's execution path reaches is
  `CredentialProjectionRuntime` (`src/runtime/projection/mod.rs`), wired through
  `with_credential_resolver`. Production plan-binding resolution landed in the same commit in
  `apps/server` (`ServerExecutionBindingResolver`). The resource **reverse-index** producer is still
  absent: `register_and_bind` has one caller, `WorkflowEngine::register_resource_and_bind`
  (`crates/engine/src/engine/mod.rs:1017`), which is itself uncalled and compiled only under the
  non-default `rotation` feature, so no live path reaches it. That half stays a real gap.

ADR-0088 lives in the maintainers' private design vault rather than this repository, so its own
2026-06-12 status table cannot be amended here. What this section does instead is put the residue
where the ADR's own text sends the reader: its pointer names this file for the forward design that
completes the unfinished D1/D2/D6 work, and the entries above are what the reader finds there. The
ADR's table is therefore the older of the two.
