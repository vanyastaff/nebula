# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html)
once a stable release ships. While the workspace is `frontier`, breaking
changes are expected between minor releases — call them out here.

## [Unreleased]

### Breaking

- **`nebula-resource` removes the `Lease` managed-call facade.**
  `call::ResourceHandle<R>`
  (`Manager::handle`, `handle_for_identity` and the erased `handle_any*`
  family) is the only managed call facade: every attempt checks out an
  instance of its own after its quota and row-gate waits, so no unit holds a
  connection while it waits. Holding one instance across several units
  belongs to a future, qualified explicit-session profile, not to an
  unbounded escape hatch.
  - Removed: `call::Lease<R>` (and its root re-export `nebula_resource::Lease`),
    `ResourceGuard::into_lease`, `impl From<ResourceGuard<R>> for Lease<R>`,
    `Lease::submit` / `submit_streaming` / `closing` / `is_closing` /
    `resource_key`, and the per-lease unit caps (one unit at a time on a
    `Pooled` / `Bounded` lease, 64 on a shared one) — a handle's units queue
    at the row gate, sized to the topology's capacity, instead.
  - `ResourceGuard` and `Manager::acquire*` stay as host-only capabilities of
    the manager, the engine and tests; a guard no longer becomes a facade.
    `OperationCx::closing` is the closing notice of the row generation the
    unit started under.
  - `nebula-sdk` is unaffected: it stopped exporting `Lease` in 0.27.0.
  - Migration:

    | Before | After |
    |---|---|
    | `manager.acquire::<R>(&ctx, &opts).await?.into_lease()` (or `Lease::from(guard)`) | `manager.handle::<R>(&ctx)?` (`handle_for_identity` for a pinned slot identity) |
    | `lease.submit(op)` / `lease.submit_streaming(op, n)` | `handle.submit(op)` / `handle.submit_streaming(op, n)` |
    | `lease.closing()` / `lease.is_closing()` | `OperationCx::closing()` inside the operation; the row's suspension or removal shows as the unit's refusal |
    | several units sharing one held instance | one unit per attempt, or a `ResourceHandle::session` on a pooled `SessionProvider` for several native calls on one connection |

- **A stable resource configuration fingerprint advances development packages
  to 0.28.0 in lockstep.** `ResourceConfig::fingerprint` is durable: the effect
  journal binds every recorded effect's destination to it, yet the derive and
  the SDK `HttpConfig` computed it with `std::hash::Hash` folded into
  `DefaultHasher`, whose output is stable neither between compiler versions
  nor across platforms — after a toolchain update every in-flight journaled
  slot would have failed as a contract mismatch. The trait signature is
  unchanged (`-> u64`); its contract and the derive change:
  - `nebula-resource`: new `ConfigFingerprint` builder and
    `ConfigFingerprintError` (also exported from
    `nebula_sdk::integration::resource`). The fingerprint is the first eight
    bytes, big-endian, of `SHA-256("nebula-resource/config-fingerprint/v1" ||
    0x00 || canonical JSON)`, where the canonical JSON is an object of the
    fingerprinted fields keyed by name, each written by its `Serialize` impl
    with every object's keys sorted (a duplicate key refused, 1 MiB per
    field). Declaration order and map iteration order do not change it; a
    field rename does. The trait docs now require a pure function of the
    configuration's content and forbid `Hash`/`DefaultHasher`.
  - `#[derive(ResourceConfig)]`: every fingerprinted field must implement
    `serde::Serialize` instead of `std::hash::Hash` (skip a field with
    `#[config(skip_fingerprint)]`). The derive now always emits `validate`
    for a config with fingerprinted fields, refusing a config whose fields
    have no stable fingerprint (`Error::permanent`, traced as a `warn` event
    with the field name and the failed invariant, never the value, under a
    `resource.config.validate` span carrying the resource key) before
    delegating to `#[config(validate = path)]`. Fieldless configs still
    return `0`. A fingerprinted field whose type names a hash-ordered set
    (`HashSet`, `FxHashSet`, …) is a compile error — its array order follows
    the per-process hash seed; use `BTreeSet`, a sorted `Vec` or
    `#[config(skip_fingerprint)]` (`HashMap` is fine: object keys are
    sorted). A NaN or infinite float is refused
    (`ConfigFingerprintError::NonFiniteFloat`) instead of aliasing `null`;
    operation-request canonicalization is unchanged.
  - The manager computes a row's fingerprint once, when its configuration
    is registered or reloaded, and stores it with the configuration behind
    the same atomic swap; unit binding, grants and topology acquires read
    the stored value instead of re-encoding the configuration.
  - Canonical JSON (fingerprints and operation requests alike) keeps a raw
    JSON number's exact decimal value — a `RawValue`'s numbers, and a
    `serde_json::Number` should a dependency enable `arbitrary_precision`
    — instead of collapsing it through an `f64`. Canonical bytes change
    only for raw numbers an `f64` cannot hold (integers outside
    `i64`/`u64`, decimals with more digits than an `f64` keeps): such a
    request recorded by an earlier build resolves as a mismatch once,
    nothing sent.
  - Every fingerprint value changes once: a hot reload compares values within
    one process and is unaffected; journaled slots recorded by an earlier
    build under a fingerprint of the old scheme resolve as a contract
    mismatch once (nothing is sent).
  - Migration: replace a hand-written `DefaultHasher` fingerprint with
    `ConfigFingerprint::new().field("name", &self.name)….finish()` and call
    `try_finish()?` from `validate`; give derived configs' field types a
    `Serialize` impl.

- **A single action route to resources advances development packages to
  0.27.0 in lockstep.** `ResourceHandle<R>` is now the only resource
  capability an action context can name; a raw lease (`ResourceGuard<R>`,
  `call::Lease<R>`, `Manager::acquire*`) stays a host-only capability of
  `nebula-resource` and the engine, because a lease bypasses the effect
  journal:
  - `nebula-core`: `ResourceAccessor::acquire_any` and `try_acquire_any` are
    removed; the accessor serves `resource_handle_any` /
    `try_resource_handle_any` (and `has`) only.
  - `nebula-action`: `ActionContextExt::acquire_resource_by_id`,
    `ActionRuntimeContext::resource` (keep `has_resource`), the
    `pub use nebula_resource::ResourceRef` re-export and the raw
    `TestContextBuilder::with_resource(key, value)` are removed.
    `TestContextBuilder::with_resource_manager(Arc<Manager>)` replaces the
    test path: it serves `ResourceHandle<R>`s for the rows registered on a
    real manager (unbound slot identity, the context's cancellation; library
    effect semantics, no journal).
  - `#[derive(Action)]`: a `#[resource]` field must be `ResourceHandle<R>` or
    `Option<ResourceHandle<R>>`. `ResourceGuard<R>` in any wrapper (`Option`,
    `Lazy`, `Option<Lazy<..>>`) fails with "`ResourceGuard<T>` slots were
    removed in 0.27.0; hold `ResourceHandle<T>` — a lease bypasses the
    effect journal". `Lazy` stays credential-only.
  - `nebula-resource`: `HasResourcesExt` (`ctx.resource::<R>()`,
    `try_resource`) and `ResourceRef<R>` are deleted. `ResourceGuard` and
    `call::Lease` are documented host-only (`Lease` is removed in a later
    release).
  - `nebula-engine`: the raw `acquire_any` routes of `EngineResourceAccessor`
    and `LayeredResourceAccessor` and the `JournaledResourceAccessor`
    refusal wrapper are gone. A key a branch scope holds still fails closed
    for a handle (`CoreError::ScopeViolation`), so a branch-scoped payload is
    no longer reachable from an action context at all.
  - `nebula-sdk`: `ResourceGuard`, `ReleaseOutcome` and `Lease` are no longer
    exported from the prelude or `integration::resource`;
    `http::open_stream` / `open_stream_until` take `&ResourceHandle<R>`.
  - **Behaviour change:** `ReadOnly` and `Remote` actions used to be able to
    check out a raw lease; they now reach a resource only through a
    read-only handle — a `Read` unit runs, an `Idempotent` / `Write` unit is
    refused `Permanent` / `NotSent` before any provider call.
  - Migration:

    | Before (≤ 0.26) | 0.27.0 |
    |---|---|
    | `#[resource] db: ResourceGuard<Db>` (or `Option` / `Lazy`) | `#[resource] db: ResourceHandle<Db>` (or `Option<..>`); move provider calls into an `Operation` and `self.db.submit(op)` |
    | `ctx.acquire_resource_by_id::<R>(id).await` | `ctx.resource_handle_by_id::<R>(id)` (`try_resource_handle_by_id` for an optional one) |
    | `ctx.resource(key).await` / `ctx.resources().acquire_any(&key)` | `ActionContextExt::resource_handle_by_id::<R>`, or `resource_handle_any` on the accessor |
    | `ctx.resource::<R>()` / `try_resource::<R>()` (`HasResourcesExt`), `ResourceRef<R>::resolve` | a `#[resource]` `ResourceHandle<R>` field |
    | `impl ResourceAccessor { fn acquire_any / try_acquire_any }` | delete both; implement `resource_handle_any` / `try_resource_handle_any` if the accessor serves rows |
    | `TestContextBuilder::with_resource(key, value)` | register the row on a `Manager`, `TestContextBuilder::with_resource_manager(manager)` |
    | `nebula_sdk::…::{ResourceGuard, ReleaseOutcome, Lease}` | `ResourceHandle` (host code that truly needs a lease depends on `nebula-resource` directly) |
    | `open_stream(&lease, request)` | `open_stream(&handle, request)` |

- **The removal of the `Limited` closure family advances development
  packages to 0.26.0 in lockstep** (MIGRATION P10):
  - Removed from `nebula_resource::rate_limit` and the SDK's
    `integration::resource`: `Limited` (`run`, `run_until`, `run_for`,
    `run_for_until`, `unlimited`, `limits`), `LimitedError`,
    `ResourceLimiter::wrap`, the `Throttle` trait, `NoThrottle`, `OnError`
    and `on_error`. `Verdict` is crate-private: the managed call facade
    derives it from a call's `OperationError`, so it is no longer exported.
  - `RateLimitProfile::InterimPerClosure` and `RateLimitProfile::is_interim`
    are removed; a row reports `PausesOnly`, `PerAcquire` or `PerAttempt`
    (`as_str`: `pauses_only`, `per_acquire`, `per_attempt`).
  - `ResourceLimiter` keeps its pacing and pause API (`rate`, `profile`,
    `ready`, `ready_for`, `penalize`, `penalize_for`), and
    `ResourceContext::limits` / `ResourceGuard::limits` stay for a pause
    signalled outside a call.
  - Migration: return the client itself as the provider's instance and make
    each provider call through the managed call facade — a
    `Manager::handle` (or a derived `#[resource] ResourceHandle<R>` action
    field) and `ResourceHandle::submit(op)`, with one
    `OperationCx::call(cost, ..)` per provider call. `run` becomes
    `cx.call(Cost::ONE, ..)`, `run_for` becomes `Cost::keyed(dimension,
    value)`, `run_until` becomes `Submission::with_deadline`, a `Throttle`
    becomes a call returning `OperationError::throttled` /
    `throttled_key`, `LimitedError` becomes `OperationError`, and
    `unlimited` has no replacement by design. A unit's quota wait ends unsent
    when its row is revoked, suspended or shut down, as a `Limited` wait did,
    and a reload still does not end it.

- **The Journaled action effect default advances development packages to
  0.25.0 in lockstep.** An action that declares no effect contract is no
  longer refused; it may perform effects, but only through resource handles:
  - `ActionEffectContract::Undeclared` is removed. The default is now
    `ActionEffectContract::Journaled(JournalProtocol::V1)`, serialized as
    `{"Journaled":"V1"}`; the old `"Undeclared"` tag no longer decodes.
    `JournalProtocol` (non-exhaustive, `V1`) is re-exported from
    `nebula_action` and its prelude. `ReadOnly` keeps its frozen
    `"NoExternalEffects"` wire tag and `#[action(read_only)]` remains the
    only effect flag; `Remote(..)` is unchanged and stays stateless-only.
  - The Graph-v1 compiler no longer raises
    `PLUGIN_PLAN_GRAPH_V1:UNDECLARED_EFFECTS` (and its activation diagnostic
    and remediation are gone). New plans record the default as
    `{"Journaled":{"protocol_version":1}}`; an unknown protocol version fails
    the plan's integrity check. Because the closed effect grammar grew a
    variant, new plans use compiler epoch 6 (canonical hash version 3), so
    their plan revision ids differ from epoch-5 ids for the same workflow.
    Epoch-5 records stay readable exactly as before and reject a `Journaled`
    effect as non-canonical. `nebula_plugin::RecordedPlanEpochV1` decodes a
    record's version header so a reader refuses an unknown epoch
    (`UnsupportedFormat`) before decoding its body; the engine's plan loader
    does. A reader from before this release decodes the body first, so it
    refuses an epoch-6 record either as an unsupported format or, when an
    action records `Journaled`, as a record decode error — never as a
    readable plan. A plan recorded without an effect field stays
    `PlanActionEffectContract::LegacyUndeclared` and is still refused — it is
    never reinterpreted as `Journaled`.
  - Only handle-routed effects are journaled; a side channel an action opens
    itself is invisible to the engine. Until the engine effect journal lands
    (it now has, for stateless actions — see "Changed"),
    a `Journaled` action of any kind runs with read-only handle authority:
    reads run, and a write through a handle is refused as `NotSent` before
    any provider call. A `Journaled` action cannot take a raw lease: a
    `ResourceGuard<R>` slot, `acquire_resource_by_id` or a raw
    `acquire_any` / `try_acquire_any` fails with a non-retryable
    `CoreError::ResourceUnavailable` pointing at `ResourceHandle<R>`
    (superseded in 0.27.0: those routes no longer exist for any action).
  - The public `ActionRuntime` entry points (`execute_action*`,
    `execute_action_with_node`) still run only explicitly `ReadOnly`
    actions and refuse a `Journaled` one with `EffectRequiresOwner`, as they
    refused `Undeclared`: a caller-supplied context may carry any resource
    accessor. `Journaled` actions run through the engine's node dispatch.

- **One outcome classification for managed calls advances development
  packages to 0.24.0 in lockstep.** A provider call's result is classified
  once and the runtime derives the attempt's sent state, the rate limit's
  verdict, the journal crossing and any re-attempt from it:
  - `OperationError` gains `throttled(retry_after)` (`Exhausted`, `Sent`;
    pauses the quota), `throttled_key(retry_after)` (pauses only the
    attempt's `Cost::keyed` key), `unreachable(detail)` /
    `unreachable_as(kind, detail)` (`NotSent`), `interrupted(detail)`
    (`Transient`, `MaybeSent`), `rejected(detail)` and
    `rejected_as(kind, detail)` (`Sent`, definitive; a retryable kind is
    recorded `Permanent`). `OperationError::new` and a converted `Error`
    are unclassified: `MaybeSent`, and nothing reaches the rate limit.
  - `OperationCx::call(cost, async move |instance, credentials| ..)` makes a
    provider call per attempt and re-attempts only what the classification
    allows — a throttle or an unreachable provider always, an interrupted
    call only for a replay-safe `EFFECT`, a rejection never — within
    `max_attempts` (one by default: no hidden retry) and the unit deadline;
    a throttle's pause is waited out by the next quota booking, never a
    sleep. The closure owns its captures (`'static`).
  - `Attempt::finish(&result)` is the low-level path (a stream finished at
    its head, several steps on one attempt). Removed from the public API:
    `Attempt::settle(SentState)` and `Attempt::report(Verdict)`; `Verdict`,
    `Throttle` and the deprecated `Limited` family stayed in `rate_limit`,
    their migration notes pointing at `OperationError::throttled`, until
    their removal in 0.26.0.
  - `unreachable` and `interrupted` never reset a backoff in progress; a
    unit's folded sent state ignores a throttled attempt that was not its
    last (the provider applied nothing).
  - The SDK HTTP adapter's hand-written retry loop is gone: each exchange
    classifies its answer (connect failure `unreachable`; a lost connection,
    `408` / `425` / `5xx` or a failed body read `interrupted`; `429`, or
    `503` with `Retry-After`, `throttled`; other `4xx` or a body over budget
    `rejected`) and `Request::run` uses `cx.call`. A `5xx` and a body read
    that failed after the head now settle `MaybeSent` instead of `Sent`; a
    `Write` is still `OutcomeUnknown` and a `Read` / `Idempotent` request
    still retryable. `http::send` finishes its attempt itself.

- **The unified resource `Operation` advances development packages to 0.23.0
  in lockstep.** An operation declares only what an execution journal needs,
  and the runtime derives the rest:
  - `Operation` is now `Serialize + DeserializeOwned` with a serializable
    `Output`, and gains `KEY` (required: 1–64 bytes of `[A-Za-z0-9_.-]`,
    alphanumeric at both ends, unique within the resource), `VERSION`
    (default 1), `KEY_WINDOW` (default 24 h, non-zero for `Idempotent`),
    `RECORD_OUTPUT` (default `true`) and `idempotency_key()` (the developer
    part, default `None`). A malformed declaration fails the build at
    `submit` and is refused `Permanent` / `NotSent` at runtime.
    `StreamOperation` gains `KEY`, `VERSION` and `idempotency_key()`, without
    serde bounds.
  - Removed: `EffectOperation`, `EffectContract`, `EffectRecovery`,
    `Recorded`, `IdempotencyKeyPart` (now a `String`), `OccurrenceLabel`,
    `ResourceHandle::submit_effect` / `session_effect`, and
    `SessionSpec::new` / `with_effect` / the `cost()` getter. One `submit`
    and one `session` route each unit by effect and caller authority: a
    journaled row drives `Idempotent` / `Write` units through its
    `EffectJournal` (streamed effects are refused), a read-only row refuses
    them, a library row or `Lease` runs them.
  - Sessions are declared with `SessionSpec::read(name)`,
    `::idempotent(name, &request)` or `::write(name, &request)` plus
    `.cost(..)`, `.idempotency_key(..)`, `.key_window(..)`, `.version(..)`;
    `ResourceHandle::session`'s output is `Serialize + DeserializeOwned`.
  - The journal seam: `EffectRecovery` → `call::journal::Recovery`;
    `JournalIntent` carries `kind` (`UnitKind`), `operation`, `version`,
    `record_output` and a `&str` key part instead of the contract, recovery
    declaration and `Recorded`; `EffectJournal::next_ordinal()`;
    occurrences are one node-wide positional sequence, `unit/v1/#{n:06}`
    (see "Changed"), with the key-sorted JSON of the operation as canonical request. An output
    over 1 MiB is recorded digest-only.
  - `OperationCx::idempotency_key()` (and the new
    `Attempt::idempotency_key()`) also returns a local key — base64url
    SHA-256 of resource, operation key, version and developer part — for an
    unjournaled unit that declares a part. The unit's span names its
    operation by `KEY` (or the session name), not its Rust type.
  - SDK HTTP adapter: `Method::OPERATION_KEY` (`http.get`, `http.post.keyed`,
    …); `Request` and `Response` serialize; the `Idempotency-Key` header of a
    `Keyed` request now carries the key derived from the developer part, not
    the raw part, so its bytes on the wire change.

  Every `Operation` implementation must add `KEY` and serde derives
  (non-intent fields `#[serde(skip)]`; with the SDK alone,
  `#[serde(crate = "nebula_sdk::serde")]`).
- **The managed call facade renames advance development packages to 0.22.0 in
  lockstep.** The resource facade is renamed to its approved names:
  - `ManagedRow` → `ResourceHandle<R>` (`Manager::handle*`) and
    `Managed`/`into_managed` → `Lease`/`into_lease`;
  - `OpCx` → `OperationCx`, `Unit` → `Submission`, `OpError` →
    `OperationError`, `UNIT_DEADLINE_CAP` → `OPERATION_DEADLINE_CAP`,
    `OperationKey` → `IdempotencyKey`, `Attempt::slots` → `credentials`;
  - the effect owner seam → `call::journal` (`EffectJournal`).

  On the action side:
  - the sealed dispatch trait `nebula_action::ResourceHandle` →
    `ResourceActionHandle`;
  - `managed_row_by_id` → `resource_handle_by_id`;
  - `#[action(no_external_effects)]` → `#[action(read_only)]` and
    `ActionEffectContract::NoExternalEffects` → `ReadOnly`, keeping the
    `"NoExternalEffects"` serde tag so stored plan records still decode.

  Behaviour is unchanged. The derive refuses the old field type and flag
  with a hint naming the new spelling.
- **The credential admission epoch advances development packages to 0.21.0 in
  lockstep.** `CredentialOperationStatus::Open` carries `admission_epoch`, the
  contract's use revision: a use admitted at one epoch must not continue at
  another. The backend advances it in the same transaction as every write that
  closes use — every advancing replacement, any change of `reauth_required`, a
  won revoke claim, a provider-egress sentinel (now transactional), and
  threshold escalation — and never moves the row version or `updated_at` for
  it. Display and retry-gate writes leave it. Exhaustion fails closed as
  `CredentialPersistenceError::AdmissionEpochExhausted` or
  `RefreshClaimError::AdmissionEpochExhausted`. Slot guard metadata reports the
  epoch read with the material (`CredentialGuardMetadata::admission_epoch`).

  `CredentialReplacement` no longer carries material bytes, state kind, state
  version, or expiry: `CredentialReplacement::new(expected_version, name,
  reauth_required, metadata, material_transition)`, and new material travels
  only in `CredentialMaterialTransition::Advance { material:
  MaterialUpdate::Replace(CredentialMaterial) }`. `Preserve` and
  `Advance { Unchanged }` leave the stored material byte-identical, so the
  encryption layer re-seals (and lazily rotates keys) only on real material
  writes. Update external `CredentialPersistence` implementations and
  exact-version pins together.

  Migration 0061 adds `credentials.admission_epoch` with 1 on every existing
  row; history is not guessed, so no binding made before the cutover matches
  a later observation. Stop old credential writers before applying migration
  0061 and restart with the new runtime: old writers do not advance the
  epoch, and on PostgreSQL their inserts fail because the backfill default is
  dropped. SQLite skips the PostgreSQL-only 0060. The in-memory claim
  repository cannot advance the epoch on claim transitions; only the SQL
  backends provide the full invariant.
- **Typed credential operation recovery advances development packages to 0.19.0
  in lockstep.** Claims persist refresh or revoke intent before provider dispatch;
  revoke pins the material epoch and has its own reconciliation decisions.
  Credential persistence adds authoritative aggregate/operation snapshots, and
  technical claim/adjudication ports require the operation-specific inputs.
  Update exact-version pins and implementations together. HTTP reconciliation
  requests without `operation` retain their refresh meaning; revoke decisions
  require `operation: "revoke"`. SDK lifecycle responses expose in-flight and
  reconciliation-required states without internal claim authority.

  Reconciliation names the incident it resolves. `reconciliation_required`
  lifecycle states carry an `incident` id, and the reconcile request requires
  it (`ReconcileCredentialRequest::new(incident, decision, evidence)` in the
  SDK, now `#[non_exhaustive]`; `RefreshClaimAdjudicator::adjudicate` takes a
  `CredentialIncidentRef`). A decision resent after a lost acknowledgement is
  answered from its own incident and can no longer resolve a newer incident on
  the same credential; one naming another incident is refused with 409
  `credential-reconciliation-stale-incident`. The resolved-set replay rule is
  removed.

  A refresh write-back re-bases onto a concurrent display-only edit instead of
  failing after the provider rotated the token, in both the resolver and the
  management refresh path. A projection that finds a refresh crossing the
  provider boundary waits up to five seconds for it and serves the refreshed
  material, instead of failing immediately with `operation_blocked`.

  Stop old credential writers before applying migration 0057 and restart with
  the new runtime. Historical claims cannot be reliably classified as refresh
  or revoke: unresolved legacy incidents remain blocked and reject typed
  adjudication. Explicitly delete the affected credential and acquire a new
  credential id after verifying the provider state. This change does not yet
  provide durable acquisition reservations or command receipts.
- **Resource rate limiting and stored-resource activation advance development
  packages to 0.20.0 in lockstep.** `ResourceRow` gains `topology` and
  `resilience_override` (migrations 0058–0060: operator settings, cross-process
  resource status, PostgreSQL rate limits). `RegistrationSpec` gains
  `rate_limit`; `RegisterRequest` gains `topology`, `resilience_override`,
  `limit_key` and `row_id`; `ResourceFactory` gains `validate_topology`,
  `resilience_policy`, `validate_resilience_override` and
  `validate_credential_bindings`. The resource status
  seam is async and reads worker-published status from storage. The pool
  `WarmupStrategy` default is now `Sequential` and every strategy is honoured,
  including by a background warmup when a stored row activates. Exact-version
  SDK consumers and external implementations of `ResourceStore` or
  `ResourceFactory` must update together.
- **Durable credential reauthentication advances development packages to 0.18.0
  in lockstep.** Refresh claims and sentinel incidents are owner-qualified, and
  threshold escalation now records the incident and advances the credential to
  `ReauthRequired` atomically. `RefreshClaimStore::try_claim` and
  `RefreshClaimAdjudicator::adjudicate` take `CredentialSelector`; reclaim
  authority moves to the separate `RefreshClaimReclaimer` port. Exact-version
  SDK consumers and external implementations of these technical ports must
  update together.
- **Workspace membership management advances development packages to 0.17.0 in
  lockstep.** The API membership authority now requires parent-qualified list,
  upsert, and removal operations for explicit workspace grants. The storage
  port enforces current organization membership, live parent resources, and
  grant cleanup during organization-member removal as one atomic authority.
  The server exposes the new workspace-member routes through the same durable
  tenant directory used by RBAC. Exact-version SDK consumers and external
  implementations of either technical membership trait must update together.
- **Tenant provisioning advances development packages to 0.16.0 in lockstep.**
  `WorkspaceStore` gains a parent-qualified active-slug lookup, and first-party
  storage gains the object-safe `TenantProvisioningStore` atomic boundary.
  InMemory, SQLite, and PostgreSQL now create an organization, its default
  workspace, and initial owner as one replay-safe operation. The server wires
  one backend-consistent tenant directory and offers an explicit operator
  bootstrap for an existing verified user. Workspace writers also enforce one
  active default per organization across every backend. Exact-version SDK consumers and
  external implementations of the technical storage traits must update every
  Nebula pin and implement the new lookup together.
- **Tenant membership advances development packages to 0.15.0 in lockstep.**
  The storage port replaces raw membership writes with typed, backend-guarded
  organization and parent-qualified workspace mutations. Organization writes
  cannot remove the last owner or administrator, role decoding fails closed,
  and authorization reads observe organization and workspace evidence in one
  logical snapshot. The API membership port removes its workspace-only lookup
  and unguarded mutation primitives. Exact-version SDK consumers and renamed
  leaf fixtures must update every Nebula pin together.
- **Credential management advances development packages to 0.14.0 in
  lockstep.** External callers can no longer invoke semantic mutations on
  `CredentialService` or construct its scheme factory. Submit `CredentialCommand`
  through an authority-bound `CredentialController`; public reads, binding
  validation, and slot projection remain technical surfaces. Worker projection
  now lives under `runtime::projection` and cannot construct management,
  refresh, or lease authority. Exact-version SDK consumers and renamed leaf
  fixtures must update every Nebula pin together.
- **Expression evaluation advances development packages to 0.13.0 in
  lockstep.** Evaluation now works on `RuntimeValue` instead of
  `serde_json::Value`, the `BuiltinFunction` contract takes `Argument`s and
  returns bounded output through `BuiltinOutputBuilder`, the `ExpressionError`
  taxonomy drops four variants and splits `EvalError`, and `Evaluator`/`eval`
  leave the supported surface (`BuiltinRegistry` becomes crate-private).
  Templates gain `{% if %}` / `{% for %}` blocks, methods, optional chaining,
  and namespaces. Exact-version SDK consumers and renamed leaf fixtures must
  continue to update all Nebula pins together; the crate changelog carries the
  full detail.
- **Credential reconciliation advances development packages to 0.12.0 in
  lockstep.** `CredentialController::new` takes the adjudicator and audit-sink
  dependencies it needs for the reconciliation command (two additional
  parameters); `RefreshClaimError` is now `#[non_exhaustive]`; and the
  engine's `credential` module with its `default_in_memory_coordinator`
  constructor is removed — `CredentialController` is the sole management
  writer of refresh-claim state, and test/desktop composition builds its
  in-memory coordinator where it is used. Exact-version SDK consumers and
  renamed leaf fixtures must continue to update all Nebula pins together.
- **Free-text error strings leave durable execution state and the journal,
  advancing the workspace to 0.11.0 in lockstep.** `ErrorEnvelope` replaces the
  `String` in `NodeAttempt::error`, `AttemptOutcome::Failure::error`,
  `JournalEntry::{NodeFailed, ExecutionFailed}::error`, and
  `NodeExecutionState::error_message`; `NodeAttempt::complete_failure` and
  `ExecutionState::mark_setup_failed` take one. A record carries a typed
  `ErrorCode`, its category, retryability, and a bounded control-escaped
  `redacted_message`. It deliberately does not walk the error's `.source()`
  chain, which is how provider text and PII reached storage, the journal,
  OnError payloads, and spans. Two consequences to plan for. When the failure
  is a typed `ActionError` (direct or wrapped by the runtime dispatcher), the
  record still carries that action's own code, category, and retryability —
  what is gone is the free-text detail (which credential, which field). A row
  persisted by an older build is refused rather than read, so such an
  execution fails closed on resume with a typed decode error.
  Drain or re-run pre-upgrade executions before upgrading. Note also that
  `ExecutionStatus::ExplicitFail::message` is unchanged: it holds the author's
  termination reason, not captured provider text.

  Six producers that built an `ActionError::Validation` detail from a
  `serde_json::Error` (the state and turn-state decode failures, three
  webhook provider bodies for generic, Slack, and Stripe, plus Slack's own
  `url_verification.response` reply) now publish a value-free summary
  instead.
  `serde_json::Error`'s `Display` embeds the offending value, quoted and
  uncapped, so a secret inside a stored state or a webhook body reached logs,
  spans, and the durable error record through `Validation`'s `Display`.
  `Validation`'s rendered shape is unchanged; only the detail text differs, and
  it keeps the failure kind and the parser position. Read a raw decode message
  from the error you caught, not from `detail`.

  Seven more decode paths stop publishing the text they choked on.
  `nebula_storage_port::StorageError`'s `From<serde_json::Error>` no longer
  forwards the decoder's `Display`, so a decode failure reports the failure kind
  and the parser position instead of the offending value. `control_dispatch`'s
  persisted-`status` decode and the daemon execution sink's identical read
  both name the shape mismatch in a framework-authored phrase, because each
  `Internal` variant is ack-failed and its text becomes the durable
  `error_message` or the dispatch-failure log line; the same phrase now
  carries that value-free summary in parentheses. The two `resume`-path state
  loads (`satisfy_signal_waits`, `cancel_dangling_nodes`) do the same for the
  `Deferred` reason a stuck control command carries. `timer_scan`'s
  overdue-row sweep and `deserialize_stored_result`'s persisted-result decode
  log the summary instead of interpolating the raw decode error. Direct
  `Serialization(…to_string())` constructors in `crates/storage/src` bypass
  the `StorageError` conversion and are not fixed here.

  A wait timeout's OnError input payload now carries the failure envelope's full
  `Display` (`CODE: message`) instead of its message alone, matching the
  action-failure path, so one handler parses one shape either way. A handler
  that matched on the payload text sees the code prefix from this release on.

  The envelope's guarantee is the type's, not the writer's. Decoding refuses a
  `redacted_message`, `code`, or `source_codes` entry that exceeds the 512-byte
  bound or carries a character the encode side always escapes, so a durable row
  that did not come from this build's encoder fails closed instead of rendering
  unbounded or line-forging text into logs and API bodies. The escaped set now
  covers bidirectional overrides, zero-width and joiner characters, and the
  line and paragraph separators, in addition to control characters; they are
  stored as `\u{…}` escapes. A `code` or `source_codes` entry outside that
  invariant is normalised on encode the same way, so every record `new` builds
  reads back. `nebula_error::ErrorCategory` gains `ALL`, a slice of every
  variant kept honest by an exhaustive match, and now decodes from an
  owned string as well as a borrowed one, so `serde_json::from_value` works on
  an `ErrorEnvelope`; `ExecutionState` still needs `from_str` because its node
  keys borrow. `nebula_execution::ExecutionError::Serialization` renders the
  constant `serialization failed` and keeps the decoder error as its `source`.

  The value-free `serde_json::Error` summary is one function,
  `nebula_error::decode::value_free_decode_summary`, behind a new optional
  `serde_json` feature on `nebula-error`; `nebula-action`, `nebula-storage-port`,
  and `nebula-engine` enable it. No new crate enters the dependency graph.

- **Checkpoint API removal advances development packages to 0.9.0 in lockstep.**
  Remove `CheckpointPolicy` imports and `with_checkpoint_policy` calls; the
  admitted metadata getter is also removed. `OnePass`, `Stepwise`, and
  `ForcedHandoff` had no end-to-end runtime implementation. No replacement
  authoring selector is provided. Historical metadata and plan tags retain their
  original encoding and identities, but non-default records cannot readmit or
  project into an executable graph. Existing runs with such contracts cannot
  resume under this runtime. The supported durable graph boundaries and the two
  retry layers remain unchanged; `ActionResult::Retry` is intentionally absent.
  See [migration guidance](crates/action/README.md#checkpoint-and-retry-migration).
- **The prior foundation migration moved the workspace from 0.5 to 0.6 in
  lockstep.** Exact-version SDK consumers and renamed leaf fixtures must
  continue to update all Nebula pins together.
- **Shared metadata admission and wire v2 advance the workspace to 0.7.0.**
  The upcoming, unreleased 0.7 release carries these breaking changes; the
  workspace package version and exact SDK/renamed-leaf fixture pins are bumped
  together in this change. `MetadataDraft::bind_schema`
  now returns `Result<BaseMetadata<K>, MetadataBuildError>` and requires
  `K: Serialize`; recorded decoding requires `K: FromStr + Serialize`.
  Categories, tags, links, deprecation chronology, canonical bytes, and complete
  record depth are checked at admission. Deprecation fields become private,
  with typed removal/replacement setters; independent `documentation_url` and
  free-form `sunset` storage are removed. `PluginManifestBuilder` no longer
  implements `Deserialize`, and `ManifestError::InvalidKey` loses its parser
  payload/source. New metadata errors carry closed field locations.
  Shared records and manifests require `metadata_wire_version: 2`; leaf records
  nest `base` and reject legacy flat, unversioned, positional, and unknown-field
  evidence. Use bounded slice/reader ingress and fresh-definition readmission.
  Historical plugin compiler/plan records and their hashes remain unchanged.
  Credential service `TypeCapabilities` adds public `interactive` and `dynamic`
  booleans, which Rust struct literals must now supply; discovery projects all
  five registry capabilities, while this Serialize-only type adds no first-party
  decode incompatibility.
  `CredentialMetadataAdmissionError::PropertiesSchema` changes its numeric
  discriminant from `0` to `1`, breaking numeric casts such as `as isize`;
  match the named variant instead of using its ordinal as an error code.
  See the [metadata breaking migration](crates/metadata/README.md#breaking) and
  [SDK migration guidance](crates/sdk/README.md).
- **Catalog-leaf metadata now has one-way admission.** Action, Credential, and
  Resource authors return schema-free `*MetadataDraft` values. The owning
  action factory, credential registry, or resource factory derives the schema
  from `Action::{Input, Output}`, `Credential::Properties`, or
  `Provider::Config`, checks identity and package invariants, and produces the
  immutable admitted `*Metadata`. The old terminal metadata builders,
  schema-taking constructors, schema setters, and public admitted-field
  mutation are removed. `BaseMetadata` and admitted leaf metadata are
  Serialize-only; persistence decodes into `Recorded*Metadata`, whose fields
  are evidence and must match a freshly admitted definition before use.
- **Schema values have authored, valid, resolved, and typed phases.**
  `FieldValue` / `FieldValues` are replaced by `ValueTree<E>` and the
  `AuthoredValue`, `ValidValues`, `ResolvedValues`, and final Rust-value
  transitions. `validate(AuthoredValue)` consumes input and prepares aliases,
  transforms, declared secrets, and programs exactly once. `resolve(context)`
  or data-only `resolve_data()` consumes `ValidValues`, completes every pending
  invariant, and returns schema-bound, expression-free `ResolvedValues` before
  trusted typed decoding.
- **Literal data and template authoring are separate inputs.**
  `AuthoredValue::from_data` never interprets template-looking strings or
  expression-shaped objects as code. `from_template_json` is the explicit
  authoring path and still requires the exact schema node to allow a program.
  Expression results re-enter as data, not recursively executable syntax.
  Authored serde and tree canonical identities use version 2; the separate
  durable `canonical_json_v1` plan contract is unchanged.
- **Typed secret extraction is explicit and cannot stage plaintext in an
  ordinary `serde_json::Value`.** A `#[field(secret)]` property must use an
  owned, zeroizing type that implements `SecretInput` (for example the
  credential `SecretString`). Normal `into_typed` rejects protected leaves;
  `into_typed_exposing_secrets` is the audited trusted boundary and feeds the
  target deserializer directly. Debug, wire, diagnostics, and error source
  chains remain protected even when a declared-secret value is malformed.
- **Credentials receive typed properties, not schema proof objects.**
  `Credential::Properties` is now `HasSchema + DeserializeOwned`; the runtime
  performs literal-only preparation and one trusted typed decode before calling
  `Credential::resolve(&Properties, ...)`. Initial OAuth token acquisition and
  refresh use separate injected authorities, so acquisition cannot enter the
  refresh coordinator's provider-to-persistence critical section. The built-in
  OAuth credential supports only Authorization Code with mandatory PKCE S256
  and Client Credentials. Device flow is removed rather than advertised as a
  dead capability.
- **Action and Resource preparation is owned by the exact selected factory.**
  Internal `ActionInput` may carry raw wire data or already-resolved values, but
  it is not an SDK authoring type. An action handle prepares and typed-decodes
  input into an opaque `PreparedActionInput` bound to that exact handle, so an
  equal schema from another factory grants no dispatch authority. Resource
  validation and registration reuse the selected `ResourceFactory`'s cached
  `R::Config` schema and pass only the internally decoded typed config onward;
  direct public JSON manager admission is removed.
- **Expression syntax is part of program identity.** `ProgramSyntax` preserves
  automatic, expression, and template parsing explicitly. A template remains
  string-valued even when it contains one expression. Authored wire-v2 expression
  entries require `syntax`; tree-v2 identities retain it. `CompiledProgram`
  retains source and syntax while sharing parsed program state through `Arc`.
  Evaluation is bounded by source/token/AST/result/depth ceilings plus a
  per-call step budget. Context
  policy may tighten, never raise, the engine ceiling. Durable JSON-v1 and
  recorded workflow parameter encodings remain unchanged.
- **Schema discovery and catalog construction are checked.**
  `HasSchema::schema` / `schema_of` return `Result`; construction failures
  propagate through action, credential, resource, and plugin discovery.
  `BaseMetadata` fields are private, names are checked through `MetadataName`,
  and lifecycle invariants are enforced while drafting and while decoding
  recorded evidence. There are no admitted-state setters or direct
  `BaseMetadata` deserialization escape hatches.
- **Core action input contracts advance to catalog version 2.0.0.** The core
  plugin bundle and its eleven newly declared input schemas advance together;
  `core.delay` keeps its unchanged 1.0.0 interface. These entity versions are
  independent of the Rust package version. Recorded plans retain their exact
  revisions rather than silently adopting different schemas.
- **Root shapes and uncertainty are explicit.** Unit types describe `null`,
  empty braced records describe objects, and primitives retain exact domains.
  `explain_assignable` / `explain_successor_of` replace binary compatibility
  shortcuts with `Yes`, `No`, and `Unknown`. Graph-v4 plans explicitly admit
  scalar schema descriptors; historical records retain their original shape
  and identity rather than becoming unit inputs implicitly.
- **Rule and expression evaluation cannot certify deferred work as success.**
  `EvaluationOutcome` preserves pending checks; full evaluation rejects missing
  context/evaluators. `CompiledProgram` retains bounded parsed syntax, evaluation
  intersects runtime policies, and JSON numeric comparisons avoid rounding
  integer operands through `f64`. Regex rules and transformers are checked at
  construction. See [schema migration details](crates/schema/CHANGELOG.md) and
  [validator migration guidance](crates/validator/docs/migration.md).
- **Validation paths are complete RFC 6901 pointers and validator invariants are
  fallible.** Root is `""`, `"/"` is an empty-key property, array indices are
  pointer segments, and `~` / `/` are escaped as `~0` / `~1`. Strict wire decode
  uses `FieldPath::from_pointer`; authored dot/bracket shorthand must opt into
  `FieldPath::parse`. Invalid regexes and inverted/incomparable numeric,
  string-length, or collection-size ranges now return typed construction errors
  instead of creating impossible validators.
- **Revision-catalog insertion is semantic-JSON idempotent and first-writer
  byte preserving.** Reinserting the same immutable plan/flavor identity with
  JSON that differs only in whitespace or object-key order returns
  `AlreadyPresent`. Different parsed content returns `ContentConflict`. The
  first accepted bytes remain authoritative and exact loads return them
  unchanged; idempotence never canonicalizes or rewrites durable records.
- **`nebula-sdk` is the only supported Rust dependency surface.** Integration
  authors migrate to its persona modules and draft metadata types. Internal
  admitted metadata, validated/resolved value proofs, erased `ActionInput`,
  prepared input tokens, registries, factories, persistence ports, and
  authority-bearing runtime types are removed from the curated SDK perimeter.
  Technical workspace crates may still expose them for first-party composition,
  without a separate compatibility promise.

- **Resource teardown has one consuming hook.** `Provider::shutdown(&Instance)`
  is removed; move flush, drain, close, and task joins into
  `Provider::destroy(Instance, TeardownCx)`. The instance stays consumed on
  error and is not retried. `TeardownCx::deadline` remains a
  `std::time::Instant`; the corrected Tokio timeout example uses
  `tokio::time::timeout_at(cx.deadline.into(), work)`.
- **Guards can only be acquired through the manager.** The public
  `ResourceGuard::{owned,guarded,guarded_with_permit,detach}` methods are
  removed. Acquire through `Manager` and release or drop the guard; extracting
  an instance would bypass lifecycle ownership. `generation()` now returns
  `u64` instead of `Option<u64>` because every admitted guard has a generation.
- **Custom topologies participate in retained ownership.**
  `Topology::create_entry` now receives `&RetainedStore<Self::Entry>` and returns
  `Result<CreatedEntry<Self::Entry>, Error>` instead of a raw entry.
  `into_instance(Entry) -> Instance` becomes
  `into_owned_instance(Entry) -> Option<Instance>`: return `Some` only for the
  final owner. Store retained roots in the supplied store; the credential hook
  also receives that store. The new `quiesce()` hook stops topology policy work;
  physical instance teardown remains in `Provider::destroy`.
- **Registry lookups no longer grant lifecycle authority.**
  `Registry::{register,remove,remove_for,clear}` and `ManagedHandle` become
  internal. Use manager registration/removal methods. Public
  `LookupOutcome::Found` and `PinnedLookup::Found` now contain
  `ManagedResourceView` instead of `Arc<dyn ManagedHandle>`; its diagnostic
  accessors replace lifecycle-handle access. `Manager::get_any` likewise
  returns a diagnostic view instead of a lifecycle handle.
  `ShutdownError` no longer implements `UnwindSafe` or `RefUnwindSafe`;
  callers must not rely on those auto-trait bounds around retained failures.
- **Rotation metrics are framework-owned terminal accounting.** The former
  `metrics::SlotDispatchOutcome` is removed and
  `ResourceOpsMetrics::{record_slot_refresh_outcome,record_slot_revoke_outcome}`
  become internal. Inspect snapshots and the public dispatch outcome instead
  of recording attempts manually. Terminal outcome labels grow from three to
  four: `success|failed|timed_out|abandoned`. Accepted observer deferral has
  separate counters and is never a terminal `abandoned` label. Terminal
  recording follows queue-owned settlement even after the observer returns;
  dashboards must not treat an immediate deferred observation as a terminal
  failure or add it to the eventual terminal attempt count. The dispatch
  latency histogram measures admission through the caller's terminal or
  deferred observation, including queue wait; a deferred sample is not hook
  completion latency. Its outcome labels include `deferred` alongside the
  four terminal labels, so its series budget differs from attempt counters.
- **Slot failure events carry typed, secret-free diagnostics.**
  `SlotRefreshFailed` and `SlotRevokeFailed` replace `error: String` with
  `kind: ErrorKind` and `message: SecretFreeMessage`. Match the kind for
  classification and use `message.as_str()` for display.
- **Credential-bearing custom topologies must define revoke ownership.**
  `Manager::register` now fails closed with `ErrorKind::Permanent` whenever a
  resource declares credential slots and its non-pooling topology does not
  report `handles_own_revoke()`. This also applies while all declared slots are
  unbound: slot declaration is a lifecycle capability, not proof of a current
  binding. Custom topology authors must either implement their revoke policy
  and return `true`, or use a pooling topology whose revoke fence is owned by
  the framework.
- **Resource release now reports accepted deferral as a value.**
  Before: `ResourceGuard::release() -> Result<(), Error>` represented accepted
  deferral as `ErrorKind::DeferredCleanup`. After:
  `ResourceGuard::release() -> Result<ReleaseOutcome, Error>` returns
  `ReleaseOutcome::{Completed, Deferred}`.
  Match `ReleaseOutcome::Completed`, `ReleaseOutcome::Deferred`, and a wildcard
  arm because the enum is non-exhaustive. `Deferred` means the queue owns the
  bounded, best-effort cleanup but completion is not guaranteed; the consumed
  guard is gone and callers must never retry the release.
- **Credential-slot dispatch preserves accepted deferral.** Before:
  `Manager::{refresh_slot,revoke_slot}* -> Result<(), Error>` erased whether
  accepted work completed. After these methods return
  `Result<SlotDispatchOutcome, Error>`, where `Completed { drain }`,
  `TimedOut { drain }`, `Deferred { drain, reason }`, and
  `Abandoned { drain }` preserve terminal execution, observer deferral, queue
  abandonment, and the exact `SlotDrainOutcome`. The lower-level `RevokeTail`
  variants likewise carry the drain result. Credential fan-out reports these
  states through `RotationOutcome` accessors, with `drain_timed_out()` and
  `observation_timed_out()` as orthogonal counts;
  external `RotationOutcome` struct literals are no longer supported.
- **Custom topology credential hooks return typed faults.** Before:
  `Topology::dispatch_credential_hook(...) -> Result<(), Error>`. After:
  `Topology::dispatch_credential_hook(...) -> Result<(), HookFault>`, where
  `HookFault::Failed(Error)` preserves author failures and
  `HookFault::TimedOut` preserves topology-owned execution timeout. Existing
  `Error` returns migrate with `?` through `From<Error> for HookFault`.
- **Retained cleanup failures are separately observable.** The non-exhaustive
  `ResourceEvent` adds `RetiredCleanupFailed { key, slot, kind, .. }` for
  framework cleanup that fails, times out, or is abandoned after a credential
  hook has already settled. Downstream event matches must retain a wildcard
  arm and match this data-bearing variant with `..`.
- **Row-retirement failures are observed where they settle.** `ResourceEvent`
  adds `ResourceTeardownFailed { key, origin, stage, kind, message, .. }`.
  Replacement, removal, shutdown, and manager-drop cleanup no longer rely on
  the eventual shutdown return to surface a failure; every failing row/stage
  emits independently while `ShutdownError` retains the first typed failure.
  Messages are fixed `SecretFreeMessage` values. Lease-accounting poison is a
  distinct restart-required, fail-closed stage; healthy siblings still tear
  down. `Removed` now explicitly means registry-unpublished, not physically
  destroyed. Dropping an open manager no longer queues teardown into the
  supervisor it is about to abort; it emits one `ManagerDrop` abandonment per
  remaining row. Use `graceful_shutdown` when physical teardown is required.

#### 0.5 to 0.6 migration examples

Metadata authors now return intent; the owning factory admits it:

```rust
// 0.5
fn metadata() -> ActionMetadata {
    ActionMetadata::builder(action_key!("acme.send"), "Send", "Send a message")
        .with_schema(schema_of::<Input>())
        .with_output_schema(schema_of::<Output>())
        .build()
}

// 0.6
fn metadata() -> ActionMetadataDraft {
    ActionMetadataDraft::new(
        action_key!("acme.send"),
        metadata_name!("Send"),
        "Send a message",
    )
}
```

Schema preparation now makes data/template intent and every consuming phase
explicit:

```rust
// 0.5
let resolved = schema.validate(values)?.resolve(context).await?;

// 0.6: literal data
let authored = AuthoredValue::from_data(json)?;
let valid = schema.validate(authored)?;
let resolved = valid.resolve_data()?;
let input: Input = resolved.into_typed()?;

// 0.6: explicitly authored templates
let authored = AuthoredValue::from_template_json(template_json)?;
let resolved = schema.validate(authored)?.resolve(context).await?;
```

Credentials no longer inspect generic values or receive `ResolvedValues`:

```rust
// 0.5
async fn resolve(values: &FieldValues, ctx: &CredentialContext) -> Result<_, _>;

// 0.6
type Properties = OAuth2Properties;
async fn resolve(properties: &OAuth2Properties, ctx: &CredentialContext) -> Result<_, _>;
```

Persisted metadata is recorded evidence, never reconstructed authority:

```rust
let recorded: RecordedActionMetadata = serde_json::from_slice(bytes)?;
let fresh = selected_factory.metadata();
let admitted = recorded.readmit_against(fresh)?;
```

### Removed

- `ErrorKind::DeferredCleanup` and `Error::deferred_cleanup`; accepted cleanup
  is represented by `ReleaseOutcome::Deferred` or
  `SlotDispatchOutcome::Deferred` instead of an error.

### Fixed

- **Journaled requests canonicalize without losing or hiding members.** The
  canonical request of an `Operation` was built from `serde_json::to_value`,
  which keeps only the last member of an object that writes one key twice (a
  `#[serde(flatten)]` collision, a hand-written impl), so two different
  requests could share canonical bytes and a changed effect would replay
  instead of reporting a mismatch; the 1 MiB cap was also checked only after
  the whole request had been materialized twice. A streaming serializer now
  writes the canonical form directly, refuses a duplicate key (`Permanent`,
  nothing echoed) and stops as soon as the output crosses the cap. The bytes
  of every valid request are unchanged. A `serde_json::value::RawValue` in a
  request is canonicalized as its text is parsed, under the same duplicate-key
  refusal and cap, instead of being parsed into a JSON value first. The HTTP
  adapter's `Request` also
  sorts its headers by name in its journaled intent (a repeated name keeps its
  values in order), so the same request built in another header order no
  longer resumes as a mismatch.

- **`OperationCx::call` no longer turns definite outcomes into ambiguous
  ones.** A retry refused by the execution owner (an unknown outcome once a
  stable-key window expired, a closed owner, a mismatch) ended the call with
  the retryable error of the previous attempt, letting a caller resubmit; it
  now ends with the owner's refusal, and a throttle before it no longer folds
  the unit `Sent` (which turned a `Write`'s retryable owner refusal into an
  unknown outcome). And a throttle whose rate-limit report
  a stalling shared limit store held until the unit deadline settled the unit
  as ended abnormally (`MaybeSent`, an unknown outcome for a `Write`, an
  ambiguous journal crossing); it now settles as the throttle it was.

- **An effect that was provably never sent no longer spends its invocation
  budget or expires into `OutcomeUnknown`.** The operation ledger counted every
  grant against `max_invocations` and checked the recovery and stable-key
  windows on every re-grant, so a budget-one `Write` became `OutcomeUnknown`
  after a single local refusal recorded as `BeforeBoundary`, and a slot that
  never crossed expired although nothing had reached the provider. Now only
  calls that may have crossed spend the budget, and the windows bind only once
  a call may have crossed; a never-crossed slot stays grantable at any age,
  bounded by a total of `OperationProtocolRecord::GRANT_CEILING` (10 000)
  grants whose refusal is `RecoveryExhausted` with no state change. Applies to
  every ledger backend and to the remote-effect driver, whose first possibly
  crossing call now takes its deadline from its own grant. Remaining
  limitation: after a late first crossing the windows are still measured from
  preparation, so such a slot fails closed early.

- **A resource lease is no longer handed out after a revoke that straddled
  its create.** A resident or bounded acquire whose create was in flight when
  `taint_slot`/`revoke_slot` returned now fails with `Revoked` (or `Cancelled`
  after a removal or shutdown) instead of serving the revoked credential; the
  built entry is released normally and the recovery gate is not tripped. A
  `Limited::run*` wait on a revoked, removed or shutting-down row now ends with
  a `Cancelled` limit error instead of holding the revoke or shutdown drain for
  the length of a provider pause; `reload_config` now serializes with acquire
  admission.

- **Resource and action slots join a refresh in flight.** Slot projection
  refused every new use while a refresh crossed the provider boundary, so each
  refresh was an outage for the resources and actions using the credential,
  and resource activation treated it as a lost credential and retired the
  serving registration. Projection now decides through the same
  credential-owned classification as the resolver: it joins the refresh for up
  to five seconds and serves the refreshed material, or answers the new
  transient `CredentialSlotResolveError::RefreshInFlight { retry_after }`.
  Revoke and reconciliation are still refused at once.
- **A credential admission reads its material and operation status in one
  statement.** `CredentialPersistence` gains a defaulted
  `get_with_operation_status`, which the SQLite and PostgreSQL adapters answer
  with one joined statement and the encryption, audit and cache layers
  forward. The resolver admits on it (one statement instead of two) and slot
  projection drops from three statements to two, keeping the secret-free
  checks ahead of decryption; the status now describes exactly the material
  returned. Measured on one host, this halves admission latency and doubles
  its throughput on both backends.
- **The PostgreSQL credential pool size is configurable.**
  `NEBULA_CRED_DB_MAX_CONNECTIONS` (default `10`, the previous fixed value)
  bounds the pool every admission reads through, in the server and the worker;
  `PgCredentialPersistence::connect_sized` / `connect_with_sized` take it.
- **Node retries honour the attempt's own retry hint.** A retryable
  `ActionError` carrying a `backoff_hint` (a provider's `Retry-After`, or a
  resource rate-limit pause surfaced through `ResourceUnavailable::retry_after`)
  was retried on the policy's backoff alone, burning attempts against a quota
  that was still closed and failing the node early. The retry delay is now the
  larger of the policy backoff and the hint, capped at the policy's
  `max_delay_ms`.
- **A pool's `max_size` now bounds its row identity, not one registration.**
  A same-identity replacement (credential refresh, new stored version, reload)
  takes over the displaced registration's checkout budget before it is
  published, so leases the displaced registration still holds keep counting and
  repeated replacements no longer stack a fresh `max_size` each; idle refill
  and warmup count those leases too. A smaller `max_size` converges as they
  return. `Topology` gains defaulted `inherit_from`, `leases_out` and
  `live_instances` hooks.
- **Resident live generations are counted and bounded.** Guards hold `Arc`
  aliases of the master, so masters displaced by a reload or recreate lived
  without bound while leased. `ResourceHealthSnapshot::live_instances` reports
  them, and no successor is built past four live masters: a live master keeps
  serving its old config, a dead one answers `Backpressure`. Docs that claimed
  one shared instance, or that a reload waits for liveness to fail, are
  corrected, and the credential-rotation guide names the window in which
  built resources keep serving a credential that turned `ReauthRequired` or
  blocked new use without a material change.
- **Retained lease fencing is generation-local.** A live lease or poisoned
  counter now fences only its own retained generation. Ready siblings are
  transferred by incremental cleanup independently and exactly once; terminal
  close likewise tears down healthy siblings before reporting poisoned
  accounting. Every transition that publishes a ready retirement wakes a
  parked cleanup observer. `RetiredDrain` is internally `#[must_use]` so a
  removed owner cannot be discarded silently.
- **Credential-hook admission is cancellation-safe.** A refresh or revoke
  observer timing out or being dropped after queue admission no longer
  returns a retryable hook error or loses terminal observability. The queue
  owns admitted execution and records exactly one eventual terminal
  metric/event; the immediate fan-out may report `deferred` without
  double-counting it as a terminal attempt. `RotationOutcome` exposes
  `drain_timed_out()` and `observation_timed_out()` as orthogonal counts,
  excluded from `dispatched()`, and reports terminal queue loss separately
  through `abandoned()`.

- **`task db:up` / `db:down` / `db:up:cache` were broken.** `Taskfile.yml`
  points `COMPOSE_LOCAL` at `deploy/docker/docker-compose.yml`, which commit
  `b1138dff` ("chore: remove stale docs") deleted as collateral alongside the
  genuinely stale blueprints. The local Postgres (+ optional Redis `cache`
  profile) stack is restored, on `postgres:17-alpine` to match the major that
  `test-matrix.yml` runs against instead of the 16 it was deleted on.
- **SQLite migration README documented a destructive, wrong procedure.** It
  claimed `0027_port_adapter_schema.sql` is byte-identical to
  `crates/storage/src/sqlite/schema.sql` and told the reader to regenerate it
  with `cp` on every port-schema change. The two have legitimately diverged —
  later port changes landed as migrations 0032–0035 — so following the
  instruction would rewrite an already-applied migration and duplicate them.
  The README now states the real invariant (replaying the migration chain must
  end in the same schema `init_schema` builds in one shot, verified by
  `pragma table_info`) and its "schema parity" section documents the four
  deliberately PostgreSQL-only migrations instead of claiming total parity.

- **Plugin load order was not deterministic across registries describing the
  same graph.** `dependency::resolve` sorted the *nodes* by key but left each
  node's edge list in manifest declaration order, so the DFS emitted
  dependencies in the order they happened to be declared. Two registries with
  the same plugins and the same dependency relationships could freeze to
  different `load_order`s — and therefore different `PluginSet` identities.
  Edge lists are now sorted (indices are assigned in ascending key order, so
  this sorts by dependency key) and deduplicated; a key declared more than once
  is still version-checked per occurrence but walked once.
- **A cargo feature changed the wire shape of a versioned config.**
  `nebula_log::Config::telemetry` is `#[cfg(feature = "telemetry")]`, so a
  `schema_version: 1` config serialized to `{"telemetry": null, …}` from a
  telemetry-enabled build and omitted the key otherwise. The field is now
  `skip_serializing_if = "Option::is_none"`, so v1 output is identical in both
  builds. The schema snapshot tests now hold as a cross-feature contract.
- **Stale source-text assertion in the Postgres refresh-retry test.** The
  snapshot statement gained a `material_epoch` column, which broke a verbatim
  `SELECT version, reauth_required, record_state` match. The test now asserts
  each required column is read in that one statement, which is the actual
  guarantee (the clock sample must come from the statement that observes
  version / reauth / record state); column order and extra projections are not
  part of it.

- **Library-first: doc-output collision** — the `nebula-worker` binary target
  and the `nebula-worker` library both emitted `doc/nebula_worker/index.html`,
  so one silently clobbered the other's published documentation. The binary now
  sets `doc = false` (no public API surface; the library carries the docs).
- **Library-first: 72 broken intra-doc links** repaired across the workspace
  (private-item docs), so `cargo doc --document-private-items` is warning-clean.

- **Plane-A OAuth `redirect_uri` missing `/api/v1` prefix (P2,
  surfaced by PR-5 wave-1 Codex review)** — the
  `derive_oauth_redirect_uri` helper in PRs #758 / #759 / #761
  produced `{public_url}/auth/oauth/{provider}/callback` while the
  actual Plane-A router is nested under `/api/v1/` in
  `crates/api/src/domain/mod.rs:170`. In production the IdP would
  have redirected to a 404 (no handler mounted at the un-prefixed
  path). Fixed in PR-5 by adding the `/api/v1` prefix to the
  derivation formula AND updating the spec.md / ADR-0085 / README
  documented examples so the operator-visible redirect_uri matches
  what they need to register with the IdP. No state-row data
  migration needed because no real Plane-A OAuth flow ran on
  affected commits (PR-1..4 implementation gated by
  `ProviderNotConfigured` for any non-configured provider; the
  fix lands before any operator wires real env vars).

### Added

- **Execution-owned managed-row effects (resource side; engine wiring
  pending).** `nebula_resource::call` gains the author surface
  `EffectOperation` (an `Operation` declaring an `EffectContract`, an
  `EffectRecovery` that must agree with its effect — `Idempotent` with
  `StableKey { window }`, `Write` with `Opaque` — what is `Recorded` of a
  success, its canonical request, and optionally an `IdempotencyKeyPart` and
  an `OccurrenceLabel`) and the owner-derived `IdempotencyKey`, exposed by
  `OperationCx::idempotency_key` and `SessionCx::idempotency_key`. The public
  seam `call::journal::EffectJournal` (with `JournalIntent`, `JournalSlot`,
  `SlotPhase`, `RecordedOutcome`, `CallGrant`, `Crossing`, `CallOutcome`,
  `JournalRefusal`, `InFlight`, `ErrorKindCode`) is what the engine will
  implement over the operation ledger. `Manager::handle_any_journaled` builds
  a handle carrying a journal: `ResourceHandle::submit_effect` and
  `ResourceHandle::session_effect` prepare
  the effect under `unit/v1/{resource_key}/{contract_id}/{label}` before any
  quota, checkout or credential read (a recorded success replays with no
  provider call; a recorded rejection, a digest-only success or an unknown
  outcome fail without one), have every attempt's call granted by the owner
  after the checkout and reads, and record the unit's last call before it
  settles; a plain `submit`/`session` of an effect on such a row is refused
  `Permanent` / `NotSent`. On a library row `submit_effect` runs as `submit`.
  An `OperationError` of kind `OutcomeUnknown` now counts as an unknown
  outcome (span field, `OperationOutcomeUnknown` event). Not SDK-exported yet.

- **The operation ledger is ready for many effects per node.**
  `OperationLedger::read_occurrences(scope, execution_id, node_key)` lists
  every slot one node prepared as `EffectOccurrenceRecord` (occurrence label
  and record), ordered by backend preparation time then label bytes, in all
  three adapters and behind the tenancy decorator. Occurrence labels are
  validated at the port (`EffectOccurrenceKey::validate_label`: 1..=512 bytes
  of visible ASCII) and rejected with `OperationLedgerError::InvalidOccurrence`
  before any durable access. A prepared slot may durably record an opaque,
  secret-free `ProviderIdempotencyKey` (1..=64 base64url bytes) in the same
  transaction, inside the existing protocol record (no migration); it is part
  of the prepare identity, never of the natural key, and is returned on
  `PreparedOperation::provider_key` so a resumed owner reads it back. A retry
  must reuse the key.

- **Actions reach resource handles.** A `#[derive(Action)]` `#[resource]`
  field may hold `ResourceHandle<R>` or `Option<ResourceHandle<R>>`
  (`Lazy<ResourceHandle<R>>` is rejected: resolution checks nothing out; the
  former `ManagedRow` spelling is refused with a hint); the factory resolves
  it synchronously through the new `ActionContextExt::resource_handle_by_id`,
  over the provided
  `nebula_core::accessor::ResourceAccessor::resource_handle_any` seam (the
  default refuses: an accessor serves no rows unless it opts in).
  The engine's `EngineResourceAccessor` serves it with the new read-only
  `Manager::handle_any_read_only` under the node's recorded slot identity; the
  layered accessor fails closed for a key a branch scope holds. The facade's
  units inherit the node's cancellation token — a unit not granted yet
  settles `Cancelled` / `NotSent`, a granted one runs on to its deadline —
  and the execution's remaining `max_duration`
  (`EngineResourceAccessor::with_deadline`) bounds every unit's deadline.
  Accepted through the engine end to end, over encrypted SQLite (F7) and on
  real PostgreSQL (PG9); the SDK perimeter proves the derived field
  (`action_resource_handle`). `#[action(read_only)]` explicitly emits the
  no-effect contract, `ActionEffectContract::ReadOnly` (its serde tag stays
  `"NoExternalEffects"`, so frozen plan records still decode; the former
  `no_external_effects` flag is refused with a hint); omitting it remains
  fail-closed `Undeclared`. The engine-side sealed dispatch trait for
  graph-scoped resource actions is `ResourceActionHandle`.
  Action-scoped rows admit `Effect::Read` only: idempotent/write operations
  and write sessions are refused `NotSent` before provider code because their
  business effects require execution-owner authority. No public ad-hoc
  accessor or SDK testing hook builds a row.
- **Resource handle and sessions.** `Manager::handle` /
  `handle_for_identity` return `nebula_resource::call::ResourceHandle<R>`:
  the managed call facade without a lease. Each attempt of a submitted
  `Operation` books its quota and waits on a FIFO row gate (sized to the
  topology's capacity) with nothing checked out, reads its bound credentials
  outside every lock, then checks out an instance of its own through the
  acquire pipeline's admission and releases it when the attempt ends; a
  checkout that created its instance on a strict row is read again and
  granted under `Manager.admission`. Refusals are unsent and a refused
  checkout returns to the pool. On a pooled provider implementing
  `call::SessionProvider` (`open` / `close` over a `Session<'c>` borrowing
  the instance), `ResourceHandle::session(SessionSpec, body)` runs one
  transaction per unit: the cost is booked once, the unannotated
  higher-ranked body borrows the session (`SessionFuture`, `SessionCx`), and
  `close` commits or rolls back (`SessionEnd`); `SessionClosed::Committed` is
  `Sent`, `RolledBack` `NotSent`, `Unknown` `MaybeSent` with the instance
  destroyed. A `SessionBinding::Connection` session only runs on an instance
  built at the credential slot epoch its unit pinned. New metrics
  `nebula_resource_row_checkouts_total{created}` and
  `nebula_resource_sessions_total{outcome}`, reported in
  `ResourceOpsSnapshot::{row_checkouts, sessions}`. The SDK curates
  `ResourceHandle` and the session vocabulary in
  `nebula_sdk::integration::resource` (not the prelude). The engine accepts
  sessions on real PostgreSQL connections (PG1–PG8, run by the PostgreSQL
  CI job). Interim: `Pooled`-only sessions, the 5-minute unit deadline (no
  `LISTEN` / `NOTIFY` or IMAP `IDLE`); actions reach a row through a
  derived field (see "Actions reach resource handles").
- **HTTP resource adapter in the SDK (feature `resource-http`).**
  `nebula_sdk::integration::resource::http` sends HTTP calls as managed
  units: `HttpConfig` (https, or http for a loopback host; timeouts, byte
  budgets, extra PEM roots; permanent validation errors that never echo the
  URL) and an auth-neutral `HttpTransport` built once in `Provider::create`
  and reused across rotations — no redirects followed, no client retries,
  no proxy, referer or cookies, platform TLS verification. A resource
  implements `HttpApi::authorize`; `Authorize::{bearer, basic,
  api_key_header}` apply the unit's pinned slots last as zeroized, sensitive
  headers (an unbound slot is `CredentialUnavailable { Absent }` and sends
  nothing; an expired OAuth2 token is `Transient`). `Request<M>` is an
  `Operation` whose method marker fixes the `Effect` (`Get`/`Head`/`Options`
  read, `Put`/`Delete` idempotent, `Post`/`Patch` write, `Keyed<_>` with an
  `Idempotency-Key`, `AsWrite<_>`); `send` classifies one attempt (connect
  failure `NotSent`, later failure `MaybeSent`, `429` / `503`+`Retry-After`
  reported and `Exhausted`, `408`/`425`/`5xx` `Transient`, other `4xx`
  `Permanent`), and a unit re-attempts only what cannot apply twice.
  `open_stream` / `open_stream_until` return a `ResponseStream` for chunked
  bodies with backpressure. No `reqwest` or `url` type is public; no URL,
  header value or transport error reaches `Debug`, errors or logs.
- **Streaming units in the managed call facade.** `nebula-resource`
  `call::{StreamOperation, StreamSink, Streaming, ConsumerGone}` and
  `Lease::submit_streaming` run an operation that yields items as one
  ordinary unit, through a bounded buffer; the unit's error follows the
  items once, and a dropped or cancelled consumer ends the operation. The
  SDK re-exports the family in `integration::resource`.
  `ResourceHandle::submit_streaming` runs one on a resource handle: each attempt
  waits for quota and the row gate with nothing checked out, and a consumer
  gone mid-stream releases the attempt's checkout and gate permit.
- **SDK-only credentialed resources.** `integration::resource` re-exports
  `CredentialSlot` and `CredentialGuard`, and `integration::credential` and
  the prelude `BearerTokenCredential`, so a derived credentialed resource
  compiles against `nebula-sdk` alone.
- **Strict per-acquire credential admission.** A `Manager` configured with
  `ManagerConfig::with_credential_observer` makes every credential-bound row
  `CredentialAdmissionProfile::StrictPerAcquire`: each acquire (and each
  explicit or background create) reads its bound credentials' availability
  first, outside every lock, through join-next coalescing (a caller only
  takes a read issued after it arrived; one read per credential lane in
  flight), bounded by the caller's deadline and 2 s; a refresh in flight is
  joined for the credential crate's bounded wait, with re-read pauses
  jittered below their bounds. Under `Manager.admission`
  the acquire then reopens or readmits a slot read usable, suspends a slot
  read blocked, and refuses on any denial. `CredentialUnavailableReason`
  gains `RefreshInFlight`, `Rebinding`, `CheckUnavailable` and `Absent`
  (refusals that do not suspend the row); a credential store outage refuses
  new credentialed work (`CheckUnavailable`) and never trips the recovery
  gate. The profile (`Unbound`, `StrictPerAcquire`, `InterimRowGate`) is
  reported by `ResourceHealthSnapshot::credential_admission` and
  `ManagedResourceView::credential_admission_profile()`, and is not
  re-exported through the SDK. A strict manager refuses to register a row
  whose declared slot lacks the projection port. New metrics:
  `nebula_resource_credential_admission_reads_total{outcome}`,
  `…_joined_total`, `…_read_duration_seconds`, `…_denied_total{reason}`.
  `nebula-credential` adds `CredentialSlotResolver::into_availability_observer`
  (an owned observer) and makes the refresh join bounds (`REFRESH_JOIN_WAIT`,
  `REFRESH_JOIN_FIRST_PAUSE`, `REFRESH_JOIN_MAX_PAUSE`,
  `REFRESH_BUSY_RETRY_AFTER`) public. Without an observer, credential-bound
  rows stay on the interim row gate with a one-time warning; the default
  becomes strict before the API freeze.

- **Managed call facade.** `ResourceGuard::into_lease` turns a lease into
  `nebula_resource::call::Lease<R>` (no `Deref`): each provider call is an
  `Operation` submitted as a lazy, runtime-owned `Submission`, and
  `OperationCx::attempt(Cost)` admits, books and grants one provider `Attempt`
  against the lease (budget, lease admission, quota at the attempt's cost
  raced against the lease closing and `Submission::cancel`, final admission).
  Each attempt settles a `SentState`; a failed unit's `OperationError` decides retry
  safety from the unit's sent state and the operation's `Effect`
  (`Read` / `Idempotent` / `Write`). New `ErrorKind::OutcomeUnknown`
  (`RESOURCE:OUTCOME_UNKNOWN`, never retried) is what a retry-unsafe unit
  becomes as a resource `Error`, with `ResourceEvent::OperationOutcomeUnknown`.
  `PinSlots` (hidden from the rendered docs) pins credential slots once per
  unit, read through `Attempt::credentials`; `#[derive(Resource)]` emits it
  (with a generated `<Name>PinnedSlots` for credentialed structs), as does
  `no_credential_slots!`. A row used this way reports
  `RateLimitProfile::PerAttempt`, and its acquires only honour pauses.
  Metrics: `nebula_resource_call_attempts_total{outcome}` and
  `nebula_resource_call_units_settled_total{sent}`; a `nebula.resource.unit`
  span per unit. `nebula-action` converts `OperationError` into `ActionError`
  (backoff hint kept, unknown outcome fatal). The SDK re-exports the facade
  from `integration::resource`, not the prelude: it is not frozen. Interim
  defaults (5-minute unit deadline cap, one unit per exclusive lease and 64
  per shared one, no refund on cancel) are listed in the resource README.

- **Strict per-attempt credential admission for managed calls.** On a
  manager with a credential availability observer, every `OperationCx::attempt` on
  a credential-bound row reads the bound credentials' availability after
  the attempt's quota wait — outside every lock, join-next shared with
  acquires, raced against the lease closing and `Submission::cancel` — and is
  registered under `Manager.admission`: taint and shutdown re-checked, the
  reading applied to the row (a block suspends it and closes its leases, an
  outage refuses `CheckUnavailable` without changing it, uninstalled
  material refuses `Rebinding`), and the unit's credential pin verified
  current before the grant. A later attempt whose pin a rotation superseded
  is refused `CredentialUnavailable { Rebinding }`, unsent; the unit's
  settled outcome decides the retry. Interim managers and slot-less rows read
  nothing. A strict row serving a facade reports the new
  `CredentialAdmissionProfile::StrictPerAttempt` (`strict_per_attempt`).

- **Resource rows report how their rate limit is enforced.**
  `nebula_resource::RateLimitProfile` names it: `PausesOnly` (no rate),
  `PerAcquire` (one permit per lease) or `InterimPerClosure` (a client wrapped
  with `ResourceLimiter::wrap`: one permit per `Limited::run*` closure).
  `ResourceLimiter::profile`, `ResourceHealthSnapshot::rate_limit_profile` and
  `ManagedResourceView::rate_limit_profile()` report it in process; the profile
  is observed, so a lazily created row reports its pre-wrap profile until the
  first create. Only `InterimPerClosure` is interim: the `Limited` closure
  family and `unlimited` are replaced by the managed call facade, and are
  documented as interim surface. The resource README carries the profile
  support matrix. `RateLimitProfile` is not re-exported through the SDK.

- **SDK public-API snapshots.** `crates/sdk/tests/public_api_snapshot.rs`
  records every public `nebula_sdk` path and the signatures of every resource
  item the SDK re-exports, tagging interim surface, so surface changes show up
  as a reviewed `.snap` diff (`task sdk:api:check`, `task sdk:api:bless`). It
  is review visibility, not a SemVer freeze.

- **Resource leases observe a closing notice.** Every acquire is admitted under
  its row's admission generation; `ResourceGuard::closing()` returns a
  `LeaseClosing` (`is_closing`, `closed`, `into_closed`; re-exported from
  `nebula_sdk::integration::resource`) that fires on credential taint/revoke,
  row removal, and manager shutdown (a graceful one when its drain starts), and
  never on a config reload, a credential refresh, or a same-identity
  replacement. The notice is cooperative: it stops no work and revokes no
  borrow, and the guard is released normally.

- **Credential-bound resource rows suspend while their credential denies use.**
  A credential that needs reauthentication, or that a revoke in flight or an
  unreconciled operation blocks, without changing its material now suspends
  every bound row: acquires, `until_accepting` and `Limited` waits fail with
  the new retryable `ErrorKind::CredentialUnavailable { reason }`
  (`CredentialUnavailableReason::{ReauthRequired, OperationBlocked}`; it never
  trips the recovery gate), every lease admitted since the previous suspension
  observes `closing`, nothing is built or health-probed, and idle and retained
  owners are kept. The row reopens without a rebuild when the same material is
  usable again at a newer use revision; a newer material installs and reopens
  through the ordinary refresh path; a taint always wins. `Manager` gains
  `suspend_credential_row(.., observed: Option<CredentialObservedAt>)`,
  `reopen_credential_row(.., ticket, observed: CredentialObservedAt)` and
  `credential_gate_ticket` (`CredentialGateTicket`, `CredentialObservedAt`,
  `CredentialSuspendOutcome`, `CredentialReopenOutcome`), `ResourceEvent`
  gains `CredentialSuspended` / `CredentialReopened`, and the health snapshot
  and `ManagedResourceView` report `CredentialSuspension`. Engine activation
  suspends (instead of retiring) a stored row whose credential is blocked and
  reopens it on the next activation that finds it usable; the worker status
  reports a suspended row as not accepting. The rotation fan-out re-observes
  bound rows on its scan and on `CredentialEvent::ReauthRequired`.
  `nebula-credential` adds `CredentialAvailabilityObserver`: one secret-free
  operational-head read, no decryption, implemented by
  `CredentialProjectionRuntime` and `CredentialService` and reachable through
  the defaulted `CredentialSlotResolver::as_availability_observer`; its
  `CredentialAvailabilityObservation` carries the use revision
  (`admission_epoch()`) of an `Open` status. It also re-exports
  `CredentialOperationKind`. On an interim manager suspension is cooperative
  and lands at the next activation or fan-out scan (30 s); a strict manager
  reads availability before every acquire (see "Strict per-acquire credential
  admission" below).

  Every consumer honours the credential **use revision** (admission epoch):
  observations are ordered by `(material_epoch, admission_epoch)`. A
  reauthentication read at one revision is only cleared by a newer one (a
  lagging `Available` read at the same revision is `StaleObservation`), an
  operation block read without a revision reopens at the revision the row
  admitted, and a late read older than what the row admitted is ignored. A
  higher revision at the installed material on an admitting row — a denial
  interval nobody observed, such as an abandoned revoke claim — answers
  `CredentialReopenOutcome::Readmitted`: new work is admitted under a fresh
  admission generation, leases admitted before stay open and nothing is
  rebuilt (logged, no new `ResourceEvent`). Engine activation tracks bound
  credentials at `(material_epoch, admission_epoch)`, so a credential display
  rename no longer registers the stored row again.

- **Per-resource rate limits, shared across workers.** `nebula-resilience`
  gains a GCRA limiter over a `LimitStore` contract (`reserve` with
  `ReserveRequest::not_before`, `penalize`, `cancel`, `penalty`), with the
  in-process `MemoryLimitStore` and a `conformance` test kit;
  `nebula-storage` implements it on PostgreSQL (`PgLimitStore`, migration
  0060). In `nebula-resource`, providers declare a `ResiliencePolicy` (rate,
  per-key limits with `keyed`, `account_credential` slots, the `LimitScope`,
  what stored rows may `overrides`, `max_penalty`); rows override within it.
  Every row gets a `ResourceLimiter` (`ResourceGuard::limits()`,
  `ResourceContext::limits()`): `wrap` turns a client into a `Limited` one
  whose calls book one permit each and pause on a provider's "slow down"
  recognised by a `Throttle` (`Verdict`, `on_error`, `NoThrottle`,
  `retry_after_from_header`). `ManagerConfig::with_shared_limit_store` makes
  cluster-scoped limits shared by every worker. The SDK re-exports these.
  Stored resources activate per execution with their operator settings,
  follow credential changes, retire when deleted, and publish their runtime
  status per worker. With the `rotation` feature and a live fan-out driver,
  a credential-bound row registers rotation-bound and its activation returns
  once the fan-out has reread the credentials and the row accepts acquires
  (`Manager::until_accepting`); without a live driver the row opts out of
  rotation and activation's own credential re-check keeps it current. A row
  that does not accept acquires yet builds no instances: warmup waits for
  the maintenance refill. The API refuses a resource row whose credential
  bindings name an undeclared slot, leave a required slot unbound or are not
  credential ids (`ResourceActivatorRegistry::validate_credential_bindings`),
  and pool settings refuse a `maintenance_interval_ms` above
  `MAX_MAINTENANCE_INTERVAL` (one day), the longest period the maintenance
  timer can be armed with.
- **Bounded shutdown drains, and rate limiters that actually share.**
  `nebula-api` gains `ShutdownGate`, which wraps a router in
  `nebula_resilience::Gate`: new requests get 503 once closing, `/health` and
  `/ready` stay admitted, and `apps/server` drains within a 10s budget instead
  of waiting on in-flight requests without a bound. `apps/worker` gets the
  same treatment (20s, `WorkerRunError::ShutdownTimedOut`). `WorkflowEngine`
  now keeps one `TokenBucket` per action key for its whole lifetime —
  previously a fresh bucket was built per node dispatch, so retries and
  sibling nodes each drew a full quota — and an unrepresentable
  `rate_limit` policy is a typed `ENGINE:RATE_LIMIT_POLICY` setup refusal
  instead of a silently dropped limit.
- **Credential reconciliation command.** A poisoned refresh claim (an expired
  in-flight row the claim store answers as outcome-unknown) is now resolvable
  through `CredentialController::reconcile` over the HTTP route, gated by the
  new `credentials:reconcile` permission. The operator decision and evidence
  note are recorded on the sentinel incident; the `(evidence digest, decision)`
  pair is the recommit identity, so an identical recommit is an idempotent
  no-op and a conflicting observation is refused. The response and the 409
  problem-details expose the evidence digest, so a client that lost its
  acknowledgement can confirm what is on record. The claim store's `release`
  now refuses (`ReleaseRefused`) while an unresolved incident is on record.
- **Durable runtime authority.** Exact bundle, plan, and flavor identity now
  governs the whole execution lifecycle rather than being merely recorded. A
  workflow activation binds a compiled executable plan to a compatible frozen
  worker flavor; start materialization creates the execution aggregate, its
  contract bundle, its live revision references and the durable `Start` command
  in one execution-owner transaction, so a lost-ack retry converges on the
  original receipt and a differing fingerprint is refused with no durable
  delta. Start, Resume, Restart, recovery and job dispatch load only the pinned
  pair — there is no reachable latest-revision or implicit-recompile fallback.
  `ExecutionTurnHandoff` ends the dispatch claim and accepts the turn under the
  aggregate fence in the same transaction, so an action's duration can no
  longer extend a queue claim, and durable checkpoints restore state and output
  across a reconnect. Effecting actions run under a durably minted
  `OperationId`, re-validated by the outcome ledger on every use: prepare is
  acknowledged before an adapter can observe the identity, stable-key recovery
  is bounded and capability-gated, reconciliation is read-only, and exhausted
  guarantees converge to a durable `OutcomeUnknown`. Five new paired migrations
  (0046–0050) carry the schema on SQLite and PostgreSQL.

- **Provenance-bound runtime-authority evidence.** The required CI jobs emit raw
  behaviour observations that `cargo xtask north-star-gates
  build-runtime-authority-bundle` assembles into an immutable artifact tree
  bound to the runner's own identity, and `verify-runtime-authority`
  recomputes the semantic policy over it. The verifier refuses a bundle naming
  a revision, repository or run other than the one it is itself running, and
  accepts only the job that actually produces the bundle as an artifact's
  source — membership in a gate's `required_ci` set is not a producer claim.
  Checked-in gate state remains a conservative baseline. Complete provenance
  and semantic verification derives `partial` for each verified gate; runtime
  evidence alone cannot emit release-level `passed`. A failed verification
  emits no effective-state result. The obsolete `TriggerDedupInbox` materialization port
  was removed, leaving `StartAcceptanceStore` as the only API that may create an
  execution, reserve a trigger key, retain exact revision references, persist
  the contract bundle, and enqueue `Start` in one transaction.

- **Authenticated credential command boundary.** API handlers now submit a
  middleware-created `AuthenticatedPrincipal`, resolved tenant `Scope`, and
  public intent through the object-safe `CredentialCommandGateway`. The
  apps-owned trust bridge invokes a credential-owned `CredentialController`,
  which obtains exactly one `CredentialTenantAuthority` decision before
  deriving an owner partition and consuming a private one-use command. The
  first-party authority revalidates the command permission from one consistent
  membership-role snapshot after verifying workspace existence and parentage
  through the canonical `WorkspaceResolver`. An unwired or unreachable
  directory/membership source fails unavailable; a valid snapshot without
  organization membership denies the command. The default server deliberately
  leaves both policy ports unwired, so tenant routes remain 503 until the K4
  supported composition path lands.
- **Object-safe owner-bound credential persistence.** `nebula-storage-port`
  now owns `CredentialPersistence` and its port-local owner/selector/row/error
  DTOs. SQLite, PostgreSQL, the internal in-memory reference adapter, and
  audit/encryption/cache decorators implement that one contract. Conformance
  covers wrong-owner indistinguishability and metadata-owner spoof rejection;
  live PostgreSQL execution remains a release gate.
- **SDK-only external perimeter proof.** Downstream fixtures with exactly one
  Nebula dependency (`nebula-sdk`, including a renamed package import) compile
  `ActionMetadataDraft`, the typed `Action::Input`/`Output` contract,
  `simple_action!`, `WorkflowBuilder`, credential `TestResult`, manual resource
  providers/custom topologies, and representative Action, Credential, Plugin,
  Resource, Schema, and Validator derives. Forty-six missing-name probes assert
  precise diagnostics for internal metadata/value proofs, authority,
  owner-selector, persistence, runtime-constructor, and unscoped-resolver
  access, including resource manager/factory/register/slot-identity paths under
  `__private`; a separate private-field probe proves the SDK-owned resource
  contribution bridge remains opaque. Another compile-pass fixture proves the
  derives also resolve explicitly renamed leaf-crate dependencies.
- **Structured secret-safe credential validation.** The credential service
  preserves a non-empty report of RFC 6901 path + stable-code issues through the
  controller/gateway, while discarding validator/provider messages, params,
  values, and sources. `FieldPath::as_str` exposes the canonical pointer;
  the API owns static value-free copy.
- **Plane-A OAuth composition seam** — `OAuthIdentityRuntime` and the opaque,
  secret-free `OAuthRuntimeBuildError` are re-exported from `nebula-api` for
  composition roots. `OAuthIdentityRuntime::from_config` returns
  `Result<Option<_>, _>`: an empty provider set creates no HTTP client, while a
  declared set creates one runtime for the selected Memory/Postgres backend.
  These are technical server-wiring exports; `nebula-sdk` remains the sole
  supported, branded Rust surface.
- **Library-first hardening pass** — `[package.metadata.docs.rs]
  all-features = true` on 15 feature-gated crates so docs.rs renders the
  complete API; a CI `feature-hygiene` job (`cargo hack --each-feature`, wired
  into the required-jobs gate) plus a `task features` target enforcing that
  each optional feature builds in isolation — per-feature + `--no-default-features`
  + all-default across every workspace member (the standalone-crate / modularity
  promise); and runnable crate-level Quick Start examples on `nebula-credential`
  (zeroizing-secret invariant) and `nebula-resource` (typed retry-classified
  errors).
- **Plane-A OAuth identity providers from operator secrets (ROADMAP
  §M3.1)** — the 1.0 surface contains exactly two reviewed profiles:
  canonical Google OIDC and GitHub.com. Operators supply only
  `API_AUTH_OAUTH_{GOOGLE,GITHUB}_{CLIENT_ID,CLIENT_SECRET}`; endpoints,
  scopes, token-auth policy, and JWKS are runtime-owned and cannot be
  overridden by environment. Microsoft, generic OIDC, GitHub Enterprise
  Server, and operator-supplied JWKS remain parked and fail boot through a
  secret-free configuration error. PostgreSQL and Memory both implement the
  staged callback: atomically consume state, perform provider egress without
  database locks, then atomically finalize local identity state. Migration
  `0029_external_identities.sql` adds the authoritative
  `(provider, subject) -> user_id` link with `ON DELETE CASCADE`.
- **OAuth MFA completion is challenge-based.** An existing linked user with
  MFA enabled receives `202 Accepted` plus an opaque, single-use challenge;
  the callback creates neither a session nor session/CSRF cookies. The
  finalizer records the MFA-required outcome and challenge atomically with its
  identity decision, and `POST /api/v1/auth/login/mfa` consumes the challenge
  to complete login and mint the session.

### Deprecated

- **The `Limited` closure family is deprecated since 0.21.0** in favour of
  the managed call facade: `ResourceLimiter::wrap`, `Limited` (`run`,
  `run_until`, `run_for`, `run_for_until`, `unlimited`) and `LimitedError`.
  Migrate each call to an `Operation` on `ResourceGuard::into_lease`: `run`
  becomes `OperationCx::attempt(Cost::ONE)`, `run_for` becomes `Cost::keyed`,
  `run_until` becomes `Submission::with_deadline`, a `Throttle` becomes
  `Attempt::report(Verdict)`, and `unlimited` has no replacement by design.
  They still work and stay re-exported by the SDK until their removal before
  the API freeze; the resource README carries the migration table.

### Breaking

- The workspace advances from `0.1` to `0.2`. Durable runtime ports now require
  exact revision and fencing data: implementers of `JobDispatchQueue` and
  `ControlQueue` must accept the worker-flavor selector, and execution owners
  must use `ExecutionTurnHandoff` for atomic claim-to-turn transfer.
- `StartAcceptanceStore` now accepts a complete `MaterializedStart` and returns
  `StartMaterialization`; process-wide reservation eviction moved to the
  separate `StartReservationMaintenance` capability. Composition roots must
  wire both capabilities explicitly.
- The operation ledger replaces direct outcome writes with the finite
  `prepare` / `read_occurrence` / `advance` protocol and separates privileged
  `OperationLedgerAdjudicator`. Effect policy, operation identities, records,
  and outcomes use validated constructors and accessors instead of public
  fields.
- `Orchestrator::new` now takes `WorkerFlavorContext` in place of
  `Vec<PluginKey>`, binding job claims and execution handoff to one exact
  worker-flavor revision.

### Security

- Update the locked TLS stack to `rustls 0.23.45`, including its required
  `aws-lc-rs`, `aws-lc-sys`, and `rustls-webpki` updates, to resolve
  [RUSTSEC-2026-0285](https://github.com/rustls/rustls/security/advisories/GHSA-2mjx-qc3c-rqvc).
- **(breaking) Credential owner authority is selector-bound.** At the
  port/application boundary, persistence
  operations require a mandatory `(owner, credential_id)` selector (or owner
  for list); CAS includes both plus expected version, owner is never updated,
  and wrong-owner access is indistinguishable from absence. The former
  metadata-keyed scope decorator, optional/global owner convention, and
  caller-created scope resolver are removed rather than aliased. The historical
  SQLite/PostgreSQL columns remain nullable until the K2 upgrade migration;
  `NULL` never grants administrator or global authority.
- **(breaking) Credential connectivity tests require write authority.**
  `POST .../credentials/{cred}/test` now consistently requires
  `credentials:write` in the HTTP access kernel, OpenAPI contract, and the
  credential command authority. Testing sends stored authority to an external
  provider and is therefore not treated as a metadata read.
- **Credential persistence diagnostics are secret-safe.** Secret-bearing port
  rows redact state, display names, and metadata contents; dynamic backend and
  audit failure details no longer render through `Display`/`Debug` or get
  forwarded into credential-service errors.
- **(breaking) Re-authentication reasons are payload-free.**
  `ReauthReason::{ProviderRejected,MissingRefreshMaterial}` no longer accept a
  provider/local `detail: String`; lifecycle events, errors, metrics, and logs
  carry only closed reason codes. Provider response text can therefore never
  enter the event bus or `Debug` through this type.
- **(breaking) Plane-A session and TOTP authorities are hardened at rest.**
  PostgreSQL sessions now store only a domain-separated SHA-256 digest of the
  256-bit cookie token; migration `0038` intentionally invalidates existing
  sessions. Active and pending TOTP seeds use versioned AES-256-GCM envelopes
  with distinct user/purpose-bound AAD, and promotion decrypts/re-seals rather
  than copying ciphertext. Credential and identity encryption consume one
  atomic `KeyProvider` snapshot, preventing key-id/key-generation races.
  Startup performs bounded, advisory-lock-serialized, crash-resumable live-row
  conversion and fails closed on tamper, unknown keys, or malformed legacy
  seeds. This conversion is not historical erasure: operators must quarantine
  or expire pre-migration backups/WAL/snapshots/replicas, retain old keys until
  every dependent backup expires, or invalidate and re-enroll MFA in strict
  deployments.

- **MFA re-enrollment no longer weakens an active factor.** Starting enrollment
  now writes a separate, ten-minute candidate and leaves the active secret
  envelope / `mfa_enabled` untouched. Confirmation verifies that candidate and promotes it
  through a storage-owned atomic consume-and-install operation; expiry, replay,
  replacement, and concurrent confirmation fail closed. Both enrollment routes
  require a CSRF-protected host-bound session created by primary authentication
  within the previous ten minutes; PAT, JWT, and API-key authority is denied.
- **Secret-bearing HTTP responses have a route-level no-store boundary.** Every
  response, including errors, from the auth and MFA routers, PAT and service-
  account creation, webhook registration, and interactive credential
  resolution now overwrites weaker inner cache policy with
  `Cache-Control: no-store`, `Pragma: no-cache`, and
  `Referrer-Policy: no-referrer`. This defense is independent of the
  idempotency replay allow-list, so adding a handler branch cannot silently
  make one-time authority cacheable.
- **(breaking, security) Webhook provider configuration is no longer an
  authority side channel.** The selected trusted factory now validates its
  complete provider configuration before registration mints a credential or
  writes a trigger/activation row; the default is fail-closed. The built-in
  Generic, Slack, and Stripe factories reject every unsupported non-empty
  `provider_config`, so arbitrary JSON is neither silently ignored nor retained
  in a soft-deleted failure tombstone, and legacy Generic `challenge_token`
  JSON fails closed. Trusted Rust composition can still set a Generic challenge
  through `GenericWebhookAction::with_challenge_token`; its authority now uses
  one shared zeroizing allocation with redacted diagnostics rather than
  cloneable plaintext strings.
- **Plane-A OAuth egress is fixed and connect-time guarded.** One opaque runtime
  now owns the fixed provider profiles, a rustls HTTPS-only client, DNS
  admission, redirects/retries/proxy prohibition, outbound concurrency, and a
  30-second per-operation network deadline; every callback egress stage reuses
  its one original deadline. Google discovery uses a singleflight/cache.
  Literal IPs and all
  DNS answers must be globally routable, and reqwest receives only the exact
  validated addresses. Provider bodies are capped at 256 KiB in zeroizing
  buffers; access tokens remain inside a one-shot opaque capability. Raw
  provider errors cannot cross the fixed RFC 9457 boundary.
- **Plane-A token-endpoint authentication is explicit and singular.** GitHub.com
  uses its fixed `client_secret_post` profile. Google prefers discovered
  `client_secret_basic`, falls back to `client_secret_post`, applies the OIDC
  Basic default when metadata omits the field, and rejects unsupported-only
  metadata. Basic authentication form-encodes each credential component before
  joining with `:` and Base64 encoding. A token request never carries client
  credentials in both the Authorization header and form body.
- **Google ID-token claims are validated on the direct-TLS path.** Google
  requires an ID token and validates its compact shape, RS256 header, pinned
  issuer, exact audience/`azp`, bounded `exp`/`iat`, nonce, `at_hash`, and subject
  equality with userinfo. Local cryptographic signature verification against
  provider JWKS remains deferred: the discovered JWKS URL is policy-validated
  but not fetched, and signature bytes receive syntax/size validation only.
- **OAuth callback traces are query-free.** HTTP request spans record method and
  the matched route template (or fixed `<unmatched>` marker), preserve inbound
  W3C parent context, and never record the raw URI containing one-time `code`
  and `state` values.
- **(breaking, security) Plane-A state is browser-bound.** OAuth start now sets
  a per-flow `Secure; HttpOnly; SameSite=Lax; Path=/` `__Host-` transaction
  cookie, and callback requires its exact version/provider/state binding before
  backend state consumption or provider egress. Accepted bindings are cleared
  on every terminal backend outcome; missing, duplicate, or swapped cookies
  return a fixed 401 without consuming the flow. A request carrying eight
  Nebula OAuth transaction-cookie names is rejected with 429 before state
  creation; this is a request-local cookie bound, not a globally atomic browser
  quota. Independently, each process or PostgreSQL deployment admits at most
  10,000 live OAuth state rows globally. A full or contended admission gate
  fails closed with 429 and does not mint state. Start and
  callback must use the `API_PUBLIC_URL` authority, so reverse proxies must
  preserve the public `Host`. Non-browser clients must migrate to a cookie jar
  that carries the matching start `Set-Cookie` into callback.
- **Provider-error callbacks are terminal without egress.** A bounded callback
  with exactly one `error` (and no `code`) must still pass authority, state, and
  browser-cookie binding. The backend consumes the matching state atomically,
  clears the accepted transaction cookie, performs no token/userinfo request,
  and returns a fixed 401 without surfacing provider text.
- **Verified-email absence is distinct from upstream failure.** After a valid
  provider identity is established, a first-link flow with no policy-acceptable
  verified email returns `EmailNotVerified` (403) and writes no link/session.
  Network failures, non-success provider responses, and malformed identity
  payloads remain the fixed upstream-failure lane (502).
- **OAuth email possession never auto-links accounts.** An existing
  `(provider, subject)` link is authoritative. A first login may create a new
  account only for an unused verified email; collision with an existing local
  account rolls back with `AccountLinkRequired` (409), creates no session, and
  requires a separate authenticated linking flow.
- **(breaking, security) Credential test contracts are payload-free.** Provider
  adapters must replace `TestResult::Failed { reason: String }` with
  `TestResult::Failed { code: TestFailureCode }`; raw provider text must be
  discarded locally. SDK consumers import both types from
  `nebula_sdk::integration::credential::{TestFailureCode, TestResult}`.
  `CredentialService::test` now returns `TestResult` directly and `TestReport`
  is removed. HTTP v1 clients must migrate from the former boolean response to
  the tagged `status` response: `success` carries `message`/`tested_at`, while
  `failed` additionally requires the frozen `CredentialTestFailureCodeV1`.
  Platform-owned messages never interpolate adapter errors; future core
  classifications map to wire code `other`.

### Changed

- **A refused write through a non-journaled action's resource handle says
  why.** Only stateless `Journaled` actions run under a node effect journal.
  A `Journaled` action of another kind keeps read-only handles (reads run,
  writes are refused `Permanent` / `NotSent` before any provider call), and
  the refusal now names the reason: "control actions decide flow and must
  not cause effects; move effects to a stateless action", "stateful effects
  are journaled per iteration in a later release", "agent effects are not
  journaled; the agent profile is planned", or "effects of this action kind
  are not journaled" (stream and others). The kind's reason takes
  precedence over "journaled effects need execution stores", which a
  stateless action without execution stores still reports. `ReadOnly`
  control actions (the built-in If, Switch, Filter) are unchanged. A
  crate-private `JournalShape` (`Flat` / `Iterated` / `None`) maps each kind
  to how it is journaled. Additive: no version bump.
- **Stateless `Journaled` actions get journaled resource effects.** On a
  durable turn (operation ledger and execution fence present), the engine
  runs a frozen, stateless `Journaled` action under one `NodeEffectJournal`
  per node attempt, and its resource handles drive every `Idempotent` /
  `Write` unit through it: each effect is one operation-ledger slot under the
  natural key `(scope, execution, node, occurrence)`, prepared lazily (no
  ledger write until the first effect; reads are never prepared; concluding
  always reads the node's occurrences once, since a crash before an attempt
  was recorded leaves the next one at the same generation), granted per
  provider call and settled or explained. A grant carries what is left of the
  ledger's window for the call (`CallGrant::with_budget` / `budget`, new):
  the resource runtime shrinks the unit's deadline to it, so no call starts
  after a stable key's deduplication window, and a grant with nothing left is
  withheld. A slot's contract identity binds the destination — resource key,
  credential slot identity and the row's configuration fingerprint
  (`JournalIntent::config_fingerprint`, new) — and `RECORD_OUTPUT`; a reload
  between a unit's submit and its grant refuses the attempt unsent.
  Occurrences are positional, `unit/v1/#{n:06}`, one sequence for all of
  a node attempt's effect units — every resource, operations and sessions
  (`EffectJournal::next_ordinal()` takes no arguments; the resource key and
  unit kind join the operation in the contract identity, so effects of
  different resources or kinds reordered also fail as a mismatch) — in
  the order units start preparing:
  the ordinal is taken by a unit's first poll, not at submit, so a
  submission dropped unpolled takes no position and cannot shift later
  effects onto unrecorded ones. Units prepared in another order, or an
  effect added or removed before recorded ones, meet other intents' slots
  and fail as a mismatch (identical intents are interchangeable); before
  its first prepare the journal reads the node's earlier occurrences once
  and refuses a fresh slot at a position an earlier attempt left empty
  below one it recorded. The operation key and version are
  bound by the contract identity, not the occurrence, so a redeploy that
  changes the operation at a recorded position without an action version
  bump fails `ENGINE:EFFECT_OCCURRENCE_MISMATCH` with nothing sent instead of
  preparing a fresh slot that would send the effect again under another
  provider key (the run-part provider keys, which frame the occurrence,
  differ from the earlier unreleased format). Occurrences restart per node
  attempt, so a retry or resume replays a settled effect's recorded output
  with no provider call, refuses an unknown one and re-grants a retryable
  failure within `Operation::max_attempts`; the `it{n}/` occurrence prefix is
  reserved for stateful iterations. The provider receives the key recorded at
  prepare, `base64url(SHA-256(...))` over the tenant, resource, operation,
  version and the developer key part (or execution, node and occurrence) —
  never an attempt number. The journal's verdict overrides the node's
  result on every exit — after the action returns and on each exit before
  it runs (cancellation, input resolution, credential refresh, rate limit),
  so an earlier dispatch's unknown call is never reported as a retryable
  failure: any slot whose call may have crossed without a recorded outcome
  (unknown, outstanding, ambiguous, or held past the drain limit by a stuck
  unit) fails the node `ENGINE:EFFECT_OUTCOME_UNKNOWN` (even when the action
  swallowed the unit's error), a changed request, binding, configuration or
  recording policy under a recorded occurrence fails it with the new
  `ENGINE:EFFECT_OCCURRENCE_MISMATCH` (`EffectExecutionError::OccurrenceMismatch`,
  plus `JournalOutcomeUnknown`) — as does a node about to succeed although
  an earlier attempt recorded an effect (settled, or a call that crossed)
  this attempt never met again —; these verdicts, a remote effect's
  unknown outcome and unreadable effect evidence
  (`EffectExecutionError::halts_execution`, new) take no error strategy:
  the node fails and the execution stops even under `IgnoreErrors` or
  `ContinueOnError`, and no OnError edge is routed. A node that fails before
  meeting such an earlier effect again keeps its own error, wrapped in the
  new `EngineError::SkippedJournaledEffect`: its retry policy may
  re-dispatch it (the retry replays the effect), but a final failure halts
  the execution the same way; and a lost lease (or a final occurrence
  read that does not answer within the verdict budget, at least 5 s)
  releases the turn without
  finalizing. Raw leases stay refused; only handle-routed effects are
  journaled (lease-facade units and raw egress are outside the journal).
  A journaled node's accessor keeps the node's branch-scoped layer in front
  of its journaled rows, so a key a scope shadows is refused as a scope
  violation rather than served by the global row.
  Without execution stores a journaled node keeps read-only handles whose
  refused writes say "journaled effects need execution stores"
  (`Manager::handle_any_read_only_because` is new); stateful, control and
  agent `Journaled` actions stay read-only until their iterations are
  journaled. A unit that fails locally (non-retryable) after its call
  succeeded — or a session the provider committed although its body
  failed — records the effect applied without output, never a provider
  rejection; only a call classified `rejected` records one. A grant whose
  budget the attempt's registration (the strict admission lock and
  reading) spent is explained not crossed and the attempt refused unsent;
  an expired unit deadline wins before the operation is polled again. New counters: `nebula_effect_journal_prepares_total{phase}`,
  `nebula_effect_journal_refusals_total{step,refusal}`,
  `nebula_effect_journal_verdicts_total{code}`.

- **`OperationProtocolRecord` counts not-crossed calls.** The record gains
  `not_crossed` (with `not_crossed()`, `crossed_invocations()`, the builder
  setter `not_crossed`, and the constant `GRANT_CEILING`). It lives in the
  existing protocol JSON payload, so no migration is needed: it is omitted when
  zero, keeping such records byte-identical, and a record written before it
  existed decodes with one not-crossed call when its latest disposition was
  `BeforeBoundary`, zero otherwise. `validate()` now requires
  `not_crossed <= invocations <= GRANT_CEILING`, bounds only the
  possibly-crossed calls by `max_invocations`, and rejects a `BeforeBoundary`
  disposition without a not-crossed call or an outstanding/ambiguous call
  counted as not crossed.

- **Breaking (workspace-internal): `EffectSlotBinding` gains `provider_key`.**
  The public-field struct now carries `provider_key:
  Option<ProviderIdempotencyKey>`; every struct literal must set it (the
  remote-effect driver passes `None`). `OperationLedger` gains the required
  method `read_occurrences`, so external implementations must add it, and
  `OperationLedgerError` gains `InvalidOccurrence`. `OperationMismatch` now
  also covers a differing provider key.

- **A resource handle is bound to the caller that built it.**
  `Manager::handle` / `handle_for_identity` link the context's
  cancellation token: once it fires, a unit whose first attempt was not
  granted yet (queued for quota, the row gate, a strict read or its
  checkout, or not started) settles `Cancelled` / `NotSent`, and the grant
  re-checks it; after the first grant it is ignored.
  `Submission::with_deadline` still only shortens a unit's deadline. The
  derive's error for a `#[resource]` field of another type now lists
  `ResourceHandle<T>`.
- **A managed unit pins its credential slots at its first grant**, not when
  it starts: the first attempt runs on the binding its final admission (and,
  on a strict manager, its credential read) validated. The pin is bracketed
  by the slots' generations and retaken when a rotation races it; a refused
  first attempt keeps no pin. `Attempt::credentials()` is unchanged for every
  attempt of a unit. The SDK's `integration::resource` docs now lead with
  the managed call facade.

- **The worker's resource manager is strict per acquire.** `apps/worker`
  composes its resource manager with the credential resolver's availability
  observer, so every credential-bound resource it activates reads the bound
  credential's availability before each acquire and create. Credentialed
  egress is therefore no more available than the credential store: while the
  store is unreachable, new credentialed work is refused
  (`CredentialUnavailable { reason: CheckUnavailable }`, retry after 1 s)
  instead of running on unverified credentials; work already admitted
  continues. Each new credentialed unit costs one secret-free head read
  (coalesced across concurrent acquires of the same credential).

- **Breaking: the storage seam and the runtime crates that ride it.** This milestone is a
  `feat!` across six crates. `nebula-storage-port` replaces
  `OperationLedger::commit_outcome` with a command-driven `advance`, retypes
  `OperationLedgerAdjudicator::adjudicate` to take frozen outcome evidence,
  adds a required worker-flavor argument to `JobDispatchQueue::claim_pending`
  and a required `ControlQueue::claim_pending_for_flavor`, and turns
  `JobDispatchMsg::required_worker_flavor_id` from an `Option` into a required
  field. `nebula-engine` gains required plan/flavor pins on `ExecutionState`
  and a non-optional operation ledger in `ExecutionStores`; `nebula-execution`
  gains public checkpoint state; `nebula-plugin`, `nebula-api` and
  `nebula-orchestrator` shift with them. The workspace release train advances
  from `0.1.0` to `0.2.0`, including the constructor retype that automated
  semver analysis does not detect when arity stays unchanged.

- **tower-http 0.7 and the OpenTelemetry family 0.32 / 0.33.** These are
  semver-major bumps, so the previous `cargo update` pass could not take them
  and — importantly — `cargo outdated` does not report them either: every
  dependency here is declared through `[workspace.dependencies]` and inherited
  with `workspace = true`, which that tool does not resolve, so it answers "all
  dependencies are up to date" while majors are outstanding. Treat dependabot,
  not `cargo outdated`, as the authority for this workspace.
  The four OpenTelemetry crates (`opentelemetry` 0.32, `opentelemetry-otlp`
  0.32, `opentelemetry_sdk` 0.32.1, `tracing-opentelemetry` 0.33) move together
  on purpose: bumping only `tracing-opentelemetry`, as the standalone dependabot
  PR proposed, would link two incompatible `opentelemetry` versions at once.
  Moving the set also collapsed a `reqwest` duplicate — `opentelemetry-http`
  0.31 pinned its own reqwest 0.12 alongside the workspace's 0.13. No source
  changes were needed; the API → control-queue → engine → action trace chain,
  W3C propagation and OTLP metric export are verified by the existing
  `OTEL_E2E_TEST` suite on the new versions.
- **(breaking) Rust 1.97.1 is the pinned toolchain and the MSRV.** `rust-version`
  moves `1.96` → `1.97` across the workspace, together with
  `rust-toolchain.toml`, `clippy.toml` `msrv`, every CI/nightly workflow pin, and
  the documented requirement in `README.md` / `AGENTS.md` / `CONTRIBUTING.md`.
  Consumers building on 1.96 must upgrade. Note that 1.97 enables the v0 symbol
  mangling scheme by default, so profiler and debugger symbol output changes
  shape (`rustc -C symbol-mangling-version=legacy` restores the old form).
- **Dependencies refreshed to latest, and `syn` upgraded to 3.0.** The
  proc-macro toolchain (`syn` / `quote` / `proc-macro2`) moved into
  `[workspace.dependencies]`; the eight derive crates now take it with
  `workspace = true` instead of eight independent pins that had already drifted
  between `"2.0"` and `"2"`. syn 3 migration was two call sites: `TypePath`
  gained an `attrs` field (all `Type` variants now carry attributes), and
  `ItemImpl::trait_` dropped its `Option<Bang>` element — negative-impl
  detection moved to `ImplModifiers`, which this workspace does not inspect
  (any trait impl is rejected by `#[credential]` regardless of polarity).
  `cargo update` picked up the remaining compatible bumps.
- **Feature-hygiene and MSRV CI jobs now deny rustc warnings** via Cargo 1.97's
  `build.warnings` (`CARGO_BUILD_WARNINGS: deny`). The clippy job only sees the
  workspace-level all / default / no-default configurations; a warning that
  fires solely under one isolated feature previously passed unnoticed through
  `cargo hack check --each-feature`, which is exactly the blind spot that job
  exists to cover.
- **(breaking) `nebula-sdk` is curated by persona, not workspace topology.**
  Broad `nebula_sdk::nebula_{action,core,credential,plugin,resource,schema,
  validator,workflow}` re-exports are gone. The currently verified one-dependency
  path is the manual/builder subset through `prelude`, `integration`, builders,
  and testing/runtime façades, plus the Action, Credential, Plugin, Resource,
  Schema, and Validator procedural-derive families. Generated derive paths
  support a renamed SDK dependency and explicitly renamed leaf dependencies;
  direct implementation-crate use remains unsupported.
- **(breaking) Credential persistence contracts moved down.** Consumers of
  the old credential-local RPITIT/dyn store bridge migrate to the directly
  object-safe `nebula_storage_port::CredentialPersistence` and port DTOs.
  This is an unsupported technical workspace contract, not an integration SDK
  surface, and there are no compatibility aliases.
- **(breaking) Membership authorization reads are snapshot-based.**
  `MembershipStore` implementors must add `get_tenant_membership` and return the
  organization plus optional workspace roles from one logical snapshot. RBAC
  and bounded-context authorities no longer reconstruct one decision from two
  independent point reads.
- **(breaking) Typed workspace paths are verified against the canonical
  directory.** `WorkspaceResolver` implementors must add
  `resolve_by_id(org_id, workspace_id)`. Middleware and credential authority no
  longer treat well-formed organization/workspace path IDs as proof that the
  workspace exists; they resolve the pair before making a membership decision.
- **(breaking) Production credential adapters moved to the application.** Key
  policy, SQLite selection, registry/catalog projection, refresh HTTP transport,
  encryption/audit wiring, and the authenticated gateway now live in
  `apps/server`. The similarly shaped API factory, registry adapter, and HTTP
  transport are available only behind unsupported `test-util`; default
  `nebula-api` has no direct credential implementation dependency.
- **(breaking) Webhook secret resolution is an API-owned port.** The public
  `CredentialBackedWebhookSecretResolver` and `mint_whsec` implementation
  helpers were removed from `nebula-api`. Composition roots implement
  `WebhookSecretResolver` using the closed, secret-free
  `SecretResolutionError`; the first-party credential-backed adapter now lives
  in `apps/server`.
- **(breaking) Audit sink failure is explicitly non-atomic.** `AuditLayer`
  propagates a sink error but never issues a compensating delete after an inner
  mutation has committed. The old CreateOnly rollback could delete a newer
  concurrent CAS write. Callers must reconcile a reported audit failure because
  the mutation may already be durable; K3 owns transactional outbox/ledger
  closure.
- **Credential mutations have one validator.** Create, update, and acquisition
  commands no longer run a competing API schema precheck. After the one
  credential-authority decision, `CredentialService` performs the canonical
  validate→resolve pipeline and returns structural path/code issues through the
  gateway. `CredentialSchemaPort` is catalog/form-schema read-model only; its
  absence does not make mutation routes return 503.
- **Breaking Plane-A OAuth Rust migration.** `AuthBackend` implementors must
  add `cancel_oauth(provider, state, redirect_uri)`. `OAuthCompletion` is now a
  non-exhaustive enum (`SessionCreated` or `MfaRequired`) rather than a cloneable
  struct, and callback query construction must account for the provider-error
  lane; `OAuthCallbackParams` is now non-exhaustive so future standard callback
  fields can be added without repeating this break. Raw Axum handlers remain a
  technical boundary; supported integrations should consume the HTTP contract
  or `nebula-sdk`, not construct handler DTOs directly.
- **Breaking auth diagnostic hardening.** `Debug` for login/reset/verify/MFA
  DTOs, session records, freshly minted PATs, token-creation responses, and
  service-account key responses and email envelopes/messages now preserves
  type/shape diagnostics while
  redacting passwords, TOTP values, reset/verification/challenge tokens,
  session/CSRF authority, PAT plaintext/hash material, MFA seeds, recipients,
  and message bodies. Secret-bearing authority values are no longer `Clone`:
  the password wrapper, live session, password/MFA outcome, MFA enrollment,
  OAuth start, freshly minted PAT, and one-time token/key responses must be
  moved through their single-owner path.
- **(breaking, security) Configuration and generated-client secret safety.**
  `ApiConfig`, its OAuth credential containers, and `SmtpEmailConfig` are now
  move-only so JWT, API-key, OAuth, and SMTP authority cannot be multiplied by
  a broad configuration clone. Serializing `ApiConfig` also omits static
  `api_keys` entirely. OpenAPI marks freshly generated PATs and service-account
  keys as response-only (`readOnly`) rather than request-only (`writeOnly`), so
  generated clients retain the one-time credential in creation responses
  without offering it as request input.
- **(breaking) Plane-A OAuth transport internals are private runtime state.**
  The former public `transport::oauth::{discovery,flow,http,userinfo}` modules,
  endpoint/config override types, raw HTTP helpers, PKCE internals, and custom-
  cfg bypass surface are removed. Composition roots receive only the opaque
  `OAuthIdentityRuntime` plus its secret-free build error; HTTP integrations
  use the versioned API and supported Rust integrations use `nebula-sdk`.
- **(breaking) Identity persistence contracts carry no reusable plaintext
  authority.** `UserRow::mfa_secret` becomes `mfa_secret_envelope`; storage-port
  user reads return `Arc<UserRow>` and the identity row is move-only with
  redacted diagnostics. Storage `SessionRow::id` becomes `token_digest`, new
  writes use `SessionDraft` plus a separately presented token, and
  `SessionRepo::{create,get,touch,revoke}` accept the presented token at the
  repository boundary. `OAuthStateRepo::create` becomes atomic `admit` with
  closed `Created | AtCapacity | Contended` outcomes; secret-bearing
  `UserRow`, `SessionRow`, `SessionDraft`, and `OAuthStateRow` values are
  move-only.
- **(breaking) Encryption-key providers return atomic generations.**
  `KeyProvider::{current_key,version}` is replaced by one `current() ->
  KeySnapshot`, whose validated key id and `Arc<EncryptionKey>` come from the
  same observation. External providers must synchronize rotation and return a
  new key id whenever key bytes change.
- **(breaking, security) Session bearers are cookie-only.** Successful
  password, MFA, and OAuth login responses no longer serialize `session_id`.
  The bearer exists only in the `Secure; HttpOnly` session cookie, preserving
  its XSS-containment boundary; JSON retains the non-bearer CSRF token needed
  by clients for the double-submit contract.
- **(breaking) Fixed browser-session protocol and Rust auth contract.** The
  former `nebula_session` / `nebula_csrf` cookies become
  `__Host-nebula-session` / `__Host-nebula-csrf`; `CookieConfig` and
  `ApiConfig::cookies` are removed because the runtime now fixes
  `Secure; Path=/; SameSite=Lax`, no `Domain`, a 14-day TTL, and `HttpOnly`
  only on the session cookie. `AuthMethod::Session` becomes
  `Session { authenticated_at }`, and `AuthBackend::get_principal_by_session`
  returns the session-authentication metadata required by fresh-session
  policy. Migration `0038` intentionally invalidates existing sessions while
  replacing persisted raw bearers with lookup digests. Operators must perform
  a coordinated cutover (mixed old/new nodes are unsupported) and users must
  authenticate again; browser clients must discard both legacy cookie names.
- **Webhook signing secrets are one-owner diagnostics.** The one-time
  registration response now redacts its `signing_secret` in `Debug`, is
  non-`Clone`, and describes the generated value as response-only in OpenAPI.
  `HmacSecret`, `WebhookActivationSpec`, and `GenericWebhookAction` are likewise
  move-only; webhook activation handles and endpoint providers redact the
  nonce-bearing capability URL, and the concrete endpoint provider is
  non-`Clone`. Integrations must move these authority-bearing values into the
  trusted factory/runtime rather than retaining broad clones.
- **(breaking) Webhook factory failures are a closed, secret-free contract.**
  `FactoryError::InvalidSpec.reason` and `FactoryError::UnknownKind` now accept
  only `&'static str`, preventing implementations from forwarding operator
  JSON or secret material into logs and problem responses. The unused
  `SecretResolution` variant is removed: authority resolution belongs before
  factory admission, and integrations must map failures to a static provider-
  owned classification.
- **(breaking, security) Idempotency replay is explicit and secret-free.**
  `IdempotencyLayer` now defaults to no replay-safe routes and requires an
  explicit matched-route allow-list. First-party composition opts in only
  authenticated POST contracts without one-time authority; auth/session, PAT,
  service-account, webhook activation, and interactive credential responses
  bypass cache lookup and storage. `Set-Cookie` and `Cache-Control: no-store`
  provide a second response-side veto, and cached-record `Debug` output redacts
  headers, bodies, and fingerprints across API, storage, and storage-port.
- **Breaking metrics vocabulary.** The parked Microsoft Plane-A profile and
  public `auth_oauth_provider::MICROSOFT` label were removed; the closed metric
  provider vocabulary is exactly `google|github` until another authority-bound
  profile is reviewed and implemented.
- **Closed provider wire tokens.** `OAuthProvider` now pins both serde and
  OpenAPI spellings explicitly to `google|github`; generated clients no longer
  receive the mechanical but invalid `git_hub` spelling for GitHub.
- **Release train:** these changes follow the released `0.1.0` frontier and
  contain intentional semver-major findings for pre-1.0 crates. The Unreleased
  train must be versioned as at least `0.2.0` by the release workflow; it must
  not be published again as `0.1.x`.
- **(breaking, security) Plane-A backend injection is runtime-based.**
  `InMemoryAuthBackend::with_oauth_providers` and
  `PgAuthBackend::with_oauth_providers` are replaced by
  `with_oauth_runtime(Arc<OAuthIdentityRuntime>)`. `AuthError` no longer carries
  attacker/provider-controlled OAuth payloads:
  `ProviderNotConfigured { provider }` and `OAuthFailed(String)` are now
  payload-free unit variants with fixed public messages. `AuthError` and
  `OAuthProvider` are now `#[non_exhaustive]`; downstream exhaustive matches
  must add a wildcard arm.
- **(breaking) `Topology<R>` async hooks are RPITIT, not `#[async_trait]`** —
  the five async hooks (`create_entry`, `accept`, `prepare`, `on_release`,
  `dispatch_credential_hook`) now return `impl Future<Output = …> + Send`
  instead of going through `async_trait`'s `Box<dyn Future>` shim. Custom
  topology authors: drop `#[async_trait]` from your `impl Topology<R>` block;
  keep plain `async fn` bodies. The same applies to the provider-hook traits
  the built-in topologies drive — `PoolProvider::recycle` /
  `PoolProvider::prepare` and `BoundedProvider::reset` are also RPITIT; drop
  `#[async_trait]` from those overrides too if you had one.
  `ResidentProvider` has no async hooks (nothing to migrate there).
  `Provider` is unchanged and still uses `#[async_trait]`. Drops three heap
  allocations per cache-hit acquire+release round trip on the default hook path.
- **(breaking) Public error and open taxonomy enums are now `#[non_exhaustive]`**
  for additive semver evolution — 10 error enums (credential, core, schema,
  resource, plugin, metadata) and 22 open outcome/event/state enums (credential
  rotation, resource, action). Closed sets stay exhaustive on purpose:
  protocol-bounded OAuth `GrantType`/`PkceMethod`/`AuthStyle`, security-critical
  `SignaturePolicy`, codegen-driving `SlotKind`, and metric-label-pinned
  `SlotDispatchOutcome`/`RecycleOutcome`.
- **Library-first: docs are now compile-checked** — converted 176 `ignore`d
  doctests to runnable / `no_run` examples across 16 crates (fixing API drift
  the `ignore` had masked); the workspace now has **zero `ignore` Rust doctest
  fences** (proc-macro derive examples are honest `text` pointing at the parent
  crate's runnable example). Hardened the CI doc gate to
  `--document-private-items`.
- **Library-first: tighter public surface** — 31 accidental `pub` items lowered
  to `pub(crate)` with `#![warn(unreachable_pub)]` guards on six crates; README
  crate map synced (orchestrator/worker/plugin-core; SQLite adapter status).
- **`nebula-resource`:** crate documentation scrubbed of plan-IDs, ADR
  numbers, internal issue/PR references, and stale temp-file links.
  Rewrote `docs/README.md` and `docs/topology-reference.md` to the v4
  three-topology surface (`Pooled`, `Resident`, `Bounded` with sealed
  `Cap` typestate). Added `# Errors` / `# Cancellation` / `# Drop` /
  `# Panics` sections to the `Resource` trait lifecycle methods, the
  `ResourceGuard` type, and the `Manager::register` / `acquire_*`
  entry points.
- Renamed runnable examples from `m6_*` to `resource_*`
  (`m6_postgres_pool` → `resource_postgres_pool`,
  `m6_resident_http` → `resource_resident_http`,
  `m6_telegram_multi_workflow` → `resource_telegram_multi_workflow`).
  Workspace `cargo run -p nebula-examples --example …` invocations
  updated accordingly.

### Removed

- **(breaking) Migration fossils retired ahead of the first release.** The
  workspace kept several `#[deprecated]` shims whose only job was to keep a
  pre-carve-out import path alive; each one is now gone rather than shipping
  into 0.1.0:
  - `nebula-credential`: the deprecated `AuthStyle` re-exports on
    `credentials`, `credentials::oauth2`, and `credentials::oauth2_config`.
    `AuthStyle` is owned by the scheme contract layer — import
    `nebula_credential::AuthStyle` or `scheme::oauth2::AuthStyle`.
  - `nebula-log`: the `field::NODE_ID` alias (use `field::NODE_KEY`) and the
    dead private `emit_to_hooks` dispatcher (`emit_to_hooks_inline` /
    `emit_to_hooks_bounded` are the real entry points).
  - `nebula-validator`: the `compose!` and `any_of!` macros. Composition is
    `ValidateExt` method chaining — `v1.and(v2)` / `v1.or(v2)` — which is what
    the macros expanded to anyway. This closes the removal already planned in
    the crate's `docs/DESIGN.md`.
  - `nebula-api`: the `EmailEnvelope` snapshot shim.
    `InMemoryAuthBackend::emails()` now returns the port's own
    `Vec<EmailMessage>`, so tests read the token from `body` and compare
    `kind` against the typed `EmailKind` instead of a stringly label.
- **(breaking, security) Legacy credential authority/store surfaces.** Removed
  the credential-local `CredentialStore`/erased dyn bridge, tenancy's
  metadata-keyed credential scope layer/resolver, public optional-owner
  authority, and broad SDK crate re-exports. No compatibility alias or supported
  API/SDK raw handle replaces them; trusted technical code uses the new
  storage-port contract directly.
- **(breaking, security) Raw Plane-A OAuth internals are no longer public.**
  `transport::oauth` and its former `discovery`, `flow`, `http`, and `userinfo`
  modules are crate-private. This removes the raw singleton client, standalone
  URL validators, discovery/userinfo wire values and errors, and the
  test-only discovery bypass from downstream reach. OAuth state/PKCE helpers
  are also private. The in-memory backend encodes replay protection through
  atomic remove-on-consume, while Postgres retains its durable,
  provider-aware atomic consume contract.
- **(breaking) Raw credential persistence access is no longer exposed by the
  supported credential/API surface.**
  Integrations using `CredentialService::credential_store_handle()` must use
  scoped facade methods instead; `CredentialHead::last_validated_at` exposes
  lifecycle metadata needed by supported callers without granting raw-store
  or write-authority access. `CredentialPersistence` remains a public but
  unsupported technical port for trusted workspace composition.
- **(breaking, security) Raw Plane-B OAuth ceremony routes were removed.**
  Clients must start and continue credential acquisition through the universal
  workspace-scoped `/credentials/resolve` and `/credentials/resolve/continue`
  endpoints. Plane-A identity OAuth routes are unchanged. The default
  credential catalog no longer advertises the unfinished `oauth2` adapter.

- `nebula-resource::docs/recovery.md` `WatchdogHandle` /
  `WatchdogConfig` section — these types are not in the public surface.
  Drive `Resource::check()` directly or compose `nebula-resilience`'s
  health-probe layer.

## How to read this file

- **Added** — new public API or capability.
- **Changed** — non-breaking behavior changes, refactors, or documentation
  improvements that may change reader expectations.
- **Deprecated** — public API still present but slated for removal.
- **Removed** — public API gone in this release.
- **Fixed** — bug fixes.
- **Security** — security-relevant fixes.

Per-crate changelogs may appear under `crates/<name>/CHANGELOG.md` once a
crate stabilises. Until then, this workspace-level changelog is the single
source of truth.
