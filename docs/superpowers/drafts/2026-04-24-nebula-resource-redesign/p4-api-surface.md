# П4 — Canonical API surface snapshot (post-П3, pre-П4)

**Source-of-truth ledger for the doc-rewrite subagents.** Every claim made in
`docs/api-reference.md` / `docs/adapters.md` / `docs/events.md` MUST trace
back to a line range in this file (or to the source files this file cites).

If a subagent finds drift between this file and `crates/resource/src/`, the
**source wins** — update this ledger and re-issue the contradiction back via
the review loop.

Captured at: branch `claude/resource-p4-doc-rewrite` HEAD = `bbcee56f` (П4 plan commit on top of action-П1 main `b21864fe`).

---

## Public re-export surface (`crates/resource/src/lib.rs`)

```text
// Top-level types
Cell                                        (cell.rs)
ResourceContext                             (context.rs)
Error, ErrorKind, ErrorScope                (error.rs)
RefreshOutcome, RevokeOutcome, RotationOutcome  (error.rs)
ResourceEvent                               (events.rs — 12 variants, see below)
HasResourcesExt                             (ext.rs)
ResourceGuard                               (guard.rs)
AcquireResilience, AcquireRetryConfig       (integration.rs)

// Manager surface
DrainTimeoutPolicy, Manager, ManagerConfig, RegisterOptions,
ResourceHealthSnapshot, ShutdownConfig, ShutdownError, ShutdownReport
                                            (manager/mod.rs + manager/options.rs + manager/shutdown.rs)

// Metrics
OutcomeCountersSnapshot, ResourceOpsMetrics, ResourceOpsSnapshot
                                            (metrics.rs)

// Re-exports from nebula-core
ExecutionId, ResourceKey, ScopeLevel, WorkflowId, resource_key!
                                            (nebula_core)

// Re-exports from nebula-credential (ADR-0036)
Credential, CredentialContext, CredentialId, NoCredential,
NoCredentialState, SchemeGuard              (nebula_credential)

// Macros
ClassifyError, Resource                     (nebula_resource_macros — derives)

// Acquire options
AcquireIntent, AcquireOptions               (options.rs)

// Recovery surface
GateState, RecoveryGate, RecoveryGateConfig, RecoveryGroupKey,
RecoveryGroupRegistry, RecoveryTicket, RecoveryWaiter,
WatchdogConfig, WatchdogHandle              (recovery/)

// Registry
AnyManagedResource, Registry                (registry.rs)
ReleaseQueue                                (release_queue.rs)
ReloadOutcome                               (reload.rs)

// Resource trait surface
AnyResource, MetadataCompatibilityError, Resource,
ResourceConfig, ResourceMetadata            (resource.rs)

// Runtime types (for direct register())
TopologyRuntime                             (runtime/mod.rs — 5 variants)
ExclusiveRuntime, ManagedResource, PoolRuntime, PoolStats,
ResidentRuntime, ServiceRuntime, TransportRuntime
                                            (runtime/{exclusive,managed,pool,resident,service,transport}.rs)

// State
ResourcePhase, ResourceStatus               (state.rs)

// Topology configs (for register_*)
Exclusive, ExclusiveConfig                  (topology/exclusive/)
BrokenCheck, InstanceMetrics, Pooled, RecycleDecision, PoolConfig
                                            (topology/pooled/)
Resident, ResidentConfig                    (topology/resident/)
Service, TokenMode, ServiceConfig           (topology/service/)
Transport, TransportConfig                  (topology/transport/)

// Topology tag (5 variants — POST-П3, no Daemon/EventSource)
TopologyTag                                 (topology_tag.rs)
```

---

## `Resource` trait — actual signature (`crates/resource/src/resource.rs:229-362`)

```rust
pub trait Resource: Send + Sync + 'static {
    type Config: ResourceConfig;
    type Runtime: Send + Sync + 'static;
    type Lease: Send + Sync + 'static;
    type Error: std::error::Error + Send + Sync + Into<crate::Error> + 'static;
    type Credential: Credential;        // ADR-0036 — NOT `type Auth`

    fn key() -> ResourceKey;

    fn create(
        &self,
        config: &Self::Config,
        scheme: &<Self::Credential as Credential>::Scheme,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;

    fn on_credential_refresh<'a>(
        &self,
        new_scheme: SchemeGuard<'a, Self::Credential>,
        ctx: &'a CredentialContext,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'a {
        let _ = (new_scheme, ctx);
        async { Ok(()) }
    }

    fn on_credential_revoke(
        &self,
        credential_id: &CredentialId,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = credential_id;
        async { Ok(()) }
    }

    fn check(...)    -> impl Future<Output = Result<(), Self::Error>> + Send { /* default Ok(()) */ }
    fn shutdown(...) -> impl Future<Output = Result<(), Self::Error>> + Send { /* default no-op */ }
    fn destroy(...)  -> impl Future<Output = Result<(), Self::Error>> + Send { /* default drop */ }

    fn schema()   -> nebula_schema::ValidSchema where Self: Sized;
    fn metadata() -> ResourceMetadata          where Self: Sized;
}
```

**Lifecycle:** `create → check (periodic) → on_credential_refresh / on_credential_revoke (rotation) → shutdown → destroy`

**ResourceConfig bound:** `: nebula_schema::HasSchema + Send + Sync + Clone + 'static`
(api-reference.md currently MISSING the `HasSchema` super-bound — drift to fix.)

**ResourceMetadata shape:** `pub struct ResourceMetadata { pub base: nebula_metadata::BaseMetadata<ResourceKey> }`
(api-reference.md currently shows 4-field `{key, name, description, tags}` — fabrication to fix.)

`ResourceMetadataBuilder` exists with `with_schema(...)`, `with_version(major, minor)`, `build()`.

`ResourceMetadata::for_resource<R>(key, name, description)` derives schema from `R::Config` via `HasSchema`.

`ResourceMetadata::validate_compatibility(&self, previous: &Self) -> Result<(), MetadataCompatibilityError>` enforces catalog-citizen rules via `nebula_metadata::validate_base_compat`.

`MetadataCompatibilityError` is `#[non_exhaustive]` with one current variant `Base(nebula_metadata::BaseCompatError<ResourceKey>)`.

---

## `Manager` API — actual surface (`crates/resource/src/manager/mod.rs`)

| Section | Methods | Source lines |
|---|---|---|
| Construction | `new()`, `with_config(ManagerConfig)` | 129, 134 |
| Subscribe | `subscribe_events() -> broadcast::Receiver<ResourceEvent>` | 181 |
| Register (full) | `register<R: Resource>(resource, config, scope, topology, options: RegisterOptions)` | 225 |
| Register (no-credential convenience, default options) | `register_pooled`, `register_resident`, `register_service`, `register_exclusive`, `register_transport` (5 methods, all bound `Credential = NoCredential`) | 302, 339, 370, 403, 436 |
| Register (full options, supports credentials) | `register_pooled_with`, `register_resident_with`, `register_service_with`, `register_transport_with`, `register_exclusive_with` (5 methods, take `RegisterOptions`) | 470, 509, 542, 577, 612 |
| Lookup | `lookup<R>(scope) -> Arc<ManagedResource<R>>` (line 649), `contains` (1429), `keys` (1434), `get_any` (1485) | |
| Acquire (full, requires `scheme` arg) | `acquire_pooled`, `acquire_resident`, `acquire_service`, `acquire_transport`, `acquire_exclusive` (5 methods) | 682, 777, 851, 932, 1013 |
| Acquire (NoCredential default — no `scheme` arg) | `acquire_pooled_default`, `acquire_resident_default`, `acquire_service_default`, `acquire_transport_default`, `acquire_exclusive_default` (5 methods, bound `Credential = NoCredential`) | 751, 826, 906, 987, 1067 |
| Reload / remove | `reload_config<R>(new_config, scope)` (1322), `remove(key)` (1392) | |
| Shutdown | `shutdown()` immediate cancel (1423), `graceful_shutdown(ShutdownConfig) -> Result<ShutdownReport, ShutdownError>` in `manager/shutdown.rs:107` | |
| Observability accessors | `metrics()` (1445), `recovery_groups()` (1439), `cancel_token()` (1452), `is_shutdown()` (1457) | |
| Rotation (called by engine, not by users directly) | `on_credential_refreshed<C>(...)` (rotation.rs:239), `on_credential_revoked(...)` (rotation.rs:375) | |

**Total:** 11 register methods (1 full + 5 + 5), 10 acquire methods (5 + 5), plus the rest.

---

## `RegisterOptions` shape (verify in `crates/resource/src/manager/options.rs`)

```rust
#[non_exhaustive]
pub struct RegisterOptions {
    pub credential_id: Option<CredentialId>,        // populates rotation reverse-index
    pub resilience:    Option<AcquireResilience>,
    pub recovery_gate: Option<Arc<RecoveryGate>>,
    pub credential_rotation_timeout: Option<Duration>,
    pub scope: ScopeLevel,
    /* ... non_exhaustive — verify full field list against source */
}
```

Subagents: open `crates/resource/src/manager/options.rs` and copy the actual struct fields verbatim before writing the api-reference table.

---

## `ResourceEvent` enum — actual 12 variants (`crates/resource/src/events.rs:22-129`)

```rust
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ResourceEvent {
    Registered { key: ResourceKey },
    Removed    { key: ResourceKey },
    AcquireSuccess { key: ResourceKey, duration: Duration },
    AcquireFailed  { key: ResourceKey, error: String },
    Released   { key: ResourceKey, held: Duration, tainted: bool },
    HealthChanged   { key: ResourceKey, healthy: bool },
    ConfigReloaded  { key: ResourceKey },
    RetryAttempt    { key: ResourceKey, attempt: u32, backoff: Duration, error: String },
    BackpressureDetected { key: ResourceKey },
    RecoveryGateChanged  { key: ResourceKey, state: String },
    CredentialRefreshed  { credential_id: CredentialId, resources_affected: usize, outcome: RotationOutcome },
    CredentialRevoked    { credential_id: CredentialId, resources_affected: usize, outcome: RotationOutcome },
}

impl ResourceEvent {
    pub fn key(&self) -> Option<&ResourceKey>;
    // Returns Some for the 10 per-resource variants;
    // returns None for CredentialRefreshed / CredentialRevoked (use credential_id field).
}
```

`RotationOutcome` lives at `crate::error::RotationOutcome` (re-exported as `nebula_resource::RotationOutcome`).

---

## `TopologyTag` enum — actual 5 variants (`crates/resource/src/topology_tag.rs`)

```rust
#[non_exhaustive]
pub enum TopologyTag { Pool, Resident, Service, Transport, Exclusive }

impl TopologyTag {
    pub fn as_str(self) -> &'static str;  // "pool", "resident", "service", "transport", "exclusive"
}

impl fmt::Display for TopologyTag { /* delegates to as_str */ }
```

Daemon and EventSource were extracted to `nebula_engine::daemon` per ADR-0037 / П3 — they are NOT in `nebula-resource` anymore.

---

## `TopologyRuntime` enum — actual 5 variants (`crates/resource/src/runtime/mod.rs:28-52`)

```rust
pub enum TopologyRuntime<R: Resource> {
    Pool(PoolRuntime<R>),
    Resident(ResidentRuntime<R>),
    Service(ServiceRuntime<R>),
    Transport(TransportRuntime<R>),
    Exclusive(ExclusiveRuntime<R>),
}

impl<R: Resource> TopologyRuntime<R> {
    pub fn tag(&self) -> TopologyTag;
}
```

---

## `Manager` submodule split (Tech Spec §5.4 — `crates/resource/src/manager/`)

Per `crates/resource/src/manager/mod.rs:58-63`:

```rust
mod execute;            // resilience pipeline + register-time pool config validation
mod gate;               // GateAdmission + admit_through_gate + settle_gate_admission
pub(crate) mod options; // ManagerConfig, RegisterOptions, ShutdownConfig, DrainTimeoutPolicy
mod registration;       // register_inner + reverse-index write
pub(crate) mod rotation; // ResourceDispatcher trampoline + on_credential_* fan-out
mod shutdown;           // graceful_shutdown + drain helpers + set_phase_all*
```

The README "Crate Layout" must reflect `manager/` as a directory with these submodules, not the v1 single `manager.rs` file.

---

## Drift to FIX in api-reference.md (R-030)

- `TopologyTag` listed with 7 variants → FIX to 5
- `TopologyRuntime` listed with 7 variants → FIX to 5
- Whole "EventSource" + "Daemon" trait sections → REMOVE (link to `nebula_engine::daemon` instead)
- `EventSourceConfig` + `DaemonConfig` rows in topology configs table → REMOVE
- `ResourceEvent` enum listed with 7 variants → REPLACE with full 12
- `ResourceEvent::key()` shown as returning `&ResourceKey` → FIX to `Option<&ResourceKey>`
- `ResourceMetadata` shown as 4-field `{key, name, description, tags}` → REPLACE with `{ base: BaseMetadata<ResourceKey> }`
- `ResourceConfig` super-bound `Send + Sync + Clone + 'static` → ADD `nebula_schema::HasSchema +` prefix
- `Resource` trait section missing `on_credential_refresh` + `on_credential_revoke` → ADD per ADR-0036 §Decision
- `Resource::create` signature uses `auth: &R::Auth` (or `()`) → REPLACE with `scheme: &<Self::Credential as Credential>::Scheme`
- `Manager::register_*` listed at 4 methods → EXPAND to 11 (1 full + 5 + 5)
- `Manager::acquire_*` listed at 5 methods → EXPAND to 10 (5 + 5)
- `ResourceContext::with_scope` and `::with_cancel_token` → these DO NOT EXIST. Document the actual surface (`ResourceContext::new(execution_id) -> Self` + capability traits `HasResources`, `HasCredentials` etc.).
- `AcquireCircuitBreakerPreset` → does NOT exist. `AcquireResilience` does NOT include a `circuit_breaker` field. REMOVE the preset enum and the `circuit_breaker` field; document only `timeout` + `retry`.
- `Manager::register` full signature lists `resilience: Option<...>` and `recovery_gate: Option<Arc<RecoveryGate>>` as positional args → ACTUAL passes both via `RegisterOptions { resilience, recovery_gate, credential_id, .. }`. REWRITE.

---

## Drift to FIX in adapters.md (R-031)

- Step 5 example uses `acquire_pooled::<R>(&(), &ctx, &opts)` — verify against current signature; `()` is the `<NoCredential as Credential>::Scheme` for opt-out. **Cleaner alternative:** switch to `acquire_pooled_default::<R>(&ctx, &opts)` (matches ADR-0036 idiom).
- `Pooled::prepare` — VERIFY against `crates/resource/src/topology/pooled/mod.rs`. If absent today, REMOVE the bullet and the example.
- `Resource::Credential = NoCredential` line in checklist — verify wording still applies.
- `ClassifyError` macro syntax block uses `#[classify(transient)]`, `#[classify(permanent)]`, `#[classify(exhausted(retry_after_secs))]` — verify against `crates/resource-macros/src/`. If syntax has shifted (e.g., field-name reference vs literal), update.
- Step 1 `validate` rule "Reject every bad field, not just the first" — actual `Error` doesn't have a multi-field aggregator built-in. Soften to "Validate format and bounds before connectivity is attempted; connectivity belongs in `create`."

---

## Drift to FIX in events.md (R-035)

- Variant table currently lists 9 (Registered, Removed, AcquireSuccess, AcquireFailed, Released, HealthChanged, ConfigReloaded, CredentialRefreshed, CredentialRevoked) → ADD 3 (RetryAttempt, BackpressureDetected, RecoveryGateChanged).
- "Per-resource variants carry a key: ResourceKey accessible via event.key() (returns Some)" — drift: 10 of 12 carry a key; the 2 aggregate rotation variants don't. Adjust phrasing.
- Aggregate variant doc claims `outcome: RotationOutcome` — true; link to `crate::error::RotationOutcome` for navigation (it's not on `ResourceEvent` but on `Error`).

---

## Drift to FIX in README.md (R-033)

- Topology Decision Guide (lines 30-43) lists Daemon and EventSource as "secondary topology" — REMOVE both rows; they live in `nebula_engine::daemon` now.
- Crate Layout block (lines 248-279) lists v1 module names: `manager.rs` (now `manager/` directory with 7 submodules), `registry.rs` (still flat), `integration.rs` (still flat). UPDATE the tree.
- Documentation table (lines 285-292) lists 6 doc files; 4 references are wrong:
  - `architecture.md` — file is `Architecture.md`; deleted in Task 2 → REMOVE the row entirely.
  - `pooling.md` — file is `Pooling.md` (case mismatch on case-sensitive filesystems); fix in Task 6 by renaming `Pooling.md → pooling.md`.
  - `events-and-hooks.md` — file is `events.md`; FIX reference.
  - `health-and-quarantine.md` — file is `recovery.md`; FIX reference.
- Line 89 `type Credential = NoCredential;` — already correct from П1 (verify).
- Line 113 `recycle()` example — verify still matches actual `Pooled::recycle` signature.

---

## Drift to FIX in pooling.md (verify pass)

- Module-level intro (lines 1-7) names `Pool<R>` — actual public type is `PoolRuntime<R>`. Either rename or clarify.
- "How the Pool Works" diagram (lines 26-35): `idle_queue: VecDeque<IdleEntry<R::Instance>>` — `R::Instance` is not a `Resource` associated type; should be `R::Runtime`.
- `PoolConfig` field list (lines 60-74) — verify against `crates/resource/src/topology/pooled/config.rs`.
- "AutoScaler" / "auto-scaling" references — AutoScaler is a v1 concept; if mentioned, remove unless an actual current feature.
- Backpressure types (`PoolBackpressurePolicy`, `AdaptiveBackpressurePolicy`) — verify still public and the variants/fields match.

---

## Drift to FIX in recovery.md (verify pass)

- `RecoveryGate::try_begin` return type — verify against `crates/resource/src/recovery/gate.rs`.
- `RecoveryTicket` API — verify `resolve` / `fail_transient` / `fail_permanent` / `attempt`.
- `RecoveryGateConfig` fields — verify.
- `WatchdogHandle::start` signature — verify.
- "Manager registers a RecoveryGate" example — verify against `register_*_with(... RegisterOptions { recovery_gate, .. })`.
