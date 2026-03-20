# Type Cross-Reference Catalog

Comprehensive catalog of ALL type names, trait names, struct names, and enum names defined or referenced across plans 01-09.

**Last updated:** 2026-03-20
**Source files:** `01-core.md` through `09-topology-guide.md`

---

## Table of Contents

1. [Traits](#traits)
2. [Structs](#structs)
3. [Enums](#enums)
4. [Type Aliases & Associated Types](#type-aliases--associated-types)
5. [Naming Inconsistencies](#naming-inconsistencies)
6. [Type Signature Mismatches](#type-signature-mismatches)
7. [Driver Terminology Issues](#driver-terminology-issues)

---

## Traits

### Resource (core resource trait)

**Defined in:** `01-core.md` (lines 10-125)

**Associated types:**
- `Config: ResourceConfig`
- `Runtime: Send + Sync + 'static`
- `Lease: Send + Sync + 'static` (stable Rust: explicit in each impl, future: `= Self::Runtime` default)
- `Error: std::error::Error + Send + Sync + Into<crate::Error> + 'static`
- `Credential: Credential`

**Constants:**
- `KEY: ResourceKey`

**Methods:**
- `create(&self, config: &Self::Config, credential: &Self::Credential, ctx: &dyn Ctx) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send`
- `shutdown(&self, _runtime: &Self::Runtime) -> impl Future<Output = Result<(), Self::Error>> + Send` (default: noop)
- `destroy(&self, runtime: Self::Runtime) -> impl Future<Output = Result<(), Self::Error>> + Send` (default: noop)
- `check(&self, _runtime: &Self::Runtime) -> impl Future<Output = Result<(), Self::Error>> + Send` (default: Ok)
- `metadata() -> ResourceMetadata` (default)

**Referenced in:**
- `02-topology.md` (all topology traits extend this)
- `03-infrastructure.md` (ResourceHandle, ManagedResource)
- `04-recovery-resilience.md` (RecoveryGate, WatchdogHandle)
- `05-manager.md` (Manager, Registry, RegistrationBuilder)
- `06-action-integration.md` (ResourceContext, EventTrigger, ResourceAction)
- `07-implementation.md` (module layout, TopologyRuntime)
- `08-correctness.md` (amendments #3, #19)
- `09-topology-guide.md` (all examples)

**Examples:**
- `Postgres` (01-core.md, 09-topology-guide.md)
- `HttpClient` (01-core.md, 09-topology-guide.md)
- `TelegramBot` (01-core.md, 09-topology-guide.md)

---

### ResourceConfig

**Defined in:** `01-core.md` (lines 275-289)

**Methods:**
- `validate(&self) -> Result<()>` (default: Ok)
- `fingerprint(&self) -> u64` (default: 0)

**Referenced in:**
- `01-core.md` (Resource::Config bound)
- `05-manager.md` (config reload, fingerprint tracking)
- `08-correctness.md` (#25: fingerprint default 0 issue)

**Examples:**
- `PgResourceConfig` (01-core.md lines 294-336)
- `HttpConfig` (referenced)

**Issue:** Amendment #25 warns about default `fingerprint() = 0` silently bypassing stale detection.

---

### Credential

**Defined in:** `01-core.md` (lines 365-372), updated in `08-correctness.md` (#3, lines 172-180)

**Constants:**
- `KIND: &'static str`

**Trait bound:** `Send + Sync + Clone + 'static`

**Referenced in:**
- `01-core.md` (Resource::Credential associated type, CredentialStore)
- `08-correctness.md` (#3 full design, #19 object-safety fix)

**Implementations:**
- `()` for `Credential` (no credentials, KIND = "none")
- `DatabaseCredential` (KIND = "database")
- `TelegramCredential` (KIND = "telegram_bot")
- `SshKeyCredential` (KIND = "ssh_key")

---

### CredentialStore

**Defined in:** `01-core.md` (lines 374-389), updated in `08-correctness.md` (#19, lines 1433-1443)

**Original (non-object-safe):**
```rust
fn resolve<C: Credential>(&self, scope: &ScopeLevel) -> impl Future<...>
```

**Fixed (object-safe):**
```rust
fn resolve_erased(&self, scope: &ScopeLevel, kind: &'static str)
  -> BoxFuture<'_, Result<Box<dyn Any + Send + Sync>, CredentialError>>
```

**Extension trait:** `CredentialStoreExt` (blanket impl, typed helper over erased store)

**Referenced in:**
- `01-core.md` (CredentialCtx extension trait)
- `08-correctness.md` (#3 credential design, #19 object-safety fix)

---

### CredentialCtx (extension trait)

**Defined in:** `01-core.md` (lines 416-418), `08-correctness.md` (#3, lines 192-194)

**Methods:**
- `credential_store(&self) -> &dyn CredentialStore`

**Trait bound:** `Ctx`

**Referenced in:**
- `01-core.md` (credential integration)
- `05-manager.md` (ManagedResource::create_instance)

---

### Ctx

**Defined in:** `01-core.md` (lines 486-508)

**Methods:**
- `scope(&self) -> &ScopeLevel`
- `execution_id(&self) -> ExecutionId`
- `cancellation(&self) -> Option<&CancellationToken>` (default: None)
- `ext<T: Send + Sync + 'static>(&self) -> Option<&T>` (default: None)

**Referenced in:**
- All topology traits (create, prepare, open_session, run, etc.)
- `03-infrastructure.md` (BasicCtx)
- `08-correctness.md` (#11 Extensions TypeId collision)

**Implementations:**
- `BasicCtx` (01-core.md lines 577-582)

---

### Topology Traits (7 total)

#### Pooled

**Defined in:** `02-topology.md` (lines 12-67)

**Trait bound:** `Resource`

**Methods:**
- `is_broken(&self, runtime: &Self::Runtime) -> BrokenCheck` (default: Healthy)
- `recycle(&self, runtime: &Self::Runtime, metrics: &InstanceMetrics) -> impl Future<Output = Result<RecycleDecision, Self::Error>> + Send` (default: Keep)
- `prepare(&self, runtime: &Self::Runtime, ctx: &dyn Ctx) -> impl Future<Output = Result<(), Self::Error>> + Send` (default: noop)

**Referenced in:**
- `02-topology.md` (Pool lifecycle, compatibility map)
- `03-infrastructure.md` (pool entry, release flow)
- `05-manager.md` (RegistrationBuilder)
- `07-implementation.md` (pool/ module)
- `08-correctness.md` (#7 prepare retry)
- `09-topology-guide.md` (Pool section)

**Examples:**
- `Postgres` (02-topology.md lines 90-131, 09-topology-guide.md)

---

#### Resident

**Defined in:** `02-topology.md` (lines 240-266), updated in `08-correctness.md` (#23, lines 1684-1699)

**Trait bound:** `Resource where Self::Lease: Clone`

**Methods:**
- `is_alive_sync(&self, _runtime: &Self::Runtime) -> bool` (default: true) — renamed from `is_alive` in #23
- `stale_after(&self) -> Option<Duration>` (default: None)

**Referenced in:**
- `02-topology.md` (Resident lifecycle, compatibility map)
- `03-infrastructure.md` (Cell, resident acquire)
- `05-manager.md` (RegistrationBuilder where clause)
- `07-implementation.md` (resident/ module)
- `08-correctness.md` (#16 Cell fix, #23 is_alive_sync rename)
- `09-topology-guide.md` (Resident section)

**Examples:**
- `HttpClient` (01-core.md, 09-topology-guide.md)
- `RedisShared` (02-topology.md lines 272-282, 09-topology-guide.md)

**Issue:** Amendment #23 renamed `is_alive()` to `is_alive_sync()` with explicit O(1) no-I/O contract.

---

#### Service

**Defined in:** `02-topology.md` (lines 326-359)

**Trait bound:** `Resource`

**Constants:**
- `TOKEN_MODE: TokenMode` (default: Cloned)

**Methods:**
- `acquire_token(&self, runtime: &Self::Runtime, ctx: &dyn Ctx) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send`
- `release_token(&self, runtime: &Self::Runtime, token: Self::Lease) -> impl Future<Output = Result<(), Self::Error>> + Send` (default: noop)

**Note:** Service uses `Self::Lease` for tokens (no separate associated type).

**Referenced in:**
- `02-topology.md` (Service lifecycle, compatibility map)
- `05-manager.md` (RegistrationBuilder, service config reload, drain watchdog)
- `08-correctness.md` (#10 drain watchdog)
- `09-topology-guide.md` (Service section)

**Examples:**
- `TelegramBot` (02-topology.md lines 364-378, 09-topology-guide.md)

---

#### Transport

**Defined in:** `02-topology.md` (lines 387-417), updated in `08-correctness.md` (#21, lines 1536-1588)

**Trait bound:** `Resource`

**Methods:**
- `open_session(&self, transport: &Self::Runtime, ctx: &dyn Ctx) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send`
- `close_session(&self, transport: &Self::Runtime, session: Self::Lease, healthy: bool) -> impl Future<Output = Result<(), Self::Error>> + Send` (default: noop)
- `keepalive(&self, transport: &Self::Runtime) -> impl Future<Output = Result<(), Self::Error>> + Send` (default: noop)

**Note:** Transport uses `Self::Lease` for sessions (no separate associated type).

**Referenced in:**
- `02-topology.md` (Transport config, acquire flow, compatibility map)
- `05-manager.md` (RegistrationBuilder)
- `08-correctness.md` (#21 max_sessions semaphore)
- `09-topology-guide.md` (Transport section)

**Examples:**
- `Ssh` (02-topology.md lines 465-479, 09-topology-guide.md)

**Issue:** Amendment #21 adds `max_sessions` limit via `Arc<Semaphore>` to prevent unbounded session creation.

---

#### Exclusive

**Defined in:** `02-topology.md` (lines 489-504)

**Trait bound:** `Resource`

**Methods:**
- `reset(&self, runtime: &Self::Runtime) -> impl Future<Output = Result<(), Self::Error>> + Send` (default: noop)

**Referenced in:**
- `02-topology.md` (Exclusive acquire flow, compatibility map)
- `03-infrastructure.md` (HandleInner::Shared, ReleaseQueue)
- `05-manager.md` (RegistrationBuilder)
- `09-topology-guide.md` (Exclusive section)

**Examples:**
- `KafkaConsumer` (02-topology.md lines 510-516, 09-topology-guide.md)

---

#### EventSource

**Defined in:** `02-topology.md` (lines 544-563)

**Trait bound:** `Resource`

**Associated types:**
- `Event: Send + Clone + 'static`
- `Subscription: Send + 'static`

**Methods:**
- `subscribe(&self, runtime: &Self::Runtime, ctx: &dyn Ctx) -> impl Future<Output = Result<Self::Subscription, Self::Error>> + Send`
- `recv(&self, subscription: &mut Self::Subscription) -> impl Future<Output = Result<Self::Event, Self::Error>> + Send`

**Referenced in:**
- `02-topology.md` (compatibility map, EventSource examples)
- `05-manager.md` (RegistrationBuilder, secondary topologies)
- `06-action-integration.md` (EventTrigger DX)
- `09-topology-guide.md` (EventSource section)

**Examples:**
- `RedisSubscriber` (02-topology.md lines 571-589, 09-topology-guide.md)

---

#### Daemon

**Defined in:** `02-topology.md` (lines 599-616), updated in `08-correctness.md` (#8, lines 664-765, #15, lines 1221-1264)

**Trait bound:** `Resource`

**Methods:**
- `run(&self, runtime: &Self::Runtime, ctx: &dyn Ctx, cancel: CancellationToken) -> impl Future<Output = Result<(), Self::Error>> + Send`

**Referenced in:**
- `02-topology.md` (Daemon lifecycle, RestartPolicy, RecreateBudget, compatibility map)
- `05-manager.md` (RegistrationBuilder, secondary topologies)
- `08-correctness.md` (#8 infinite recreate loop fix, #15 mem::zeroed UB fix)
- `09-topology-guide.md` (Daemon section)

**Examples:**
- `TelegramBot` (02-topology.md lines 622-651, 09-topology-guide.md)

**Issues:**
- Amendment #8 adds `RecreateBudget` to prevent infinite recreate loop.
- Amendment #15 replaces `mem::zeroed()` with `Option<R::Runtime>` to avoid UB.

---

### AnyManagedResource (type erasure)

**Defined in:** `05-manager.md` (lines 328-335), `07-implementation.md` (lines 284)

**Methods:**
- `as_any(&self) -> &dyn Any`
- `resource_key(&self) -> &ResourceKey`
- `topology_kind(&self) -> TopologyKind`
- `health_status(&self) -> HealthStatus`
- `config_fingerprint(&self) -> u64`
- `shutdown(&self) -> BoxFuture<'_, Result<()>>`

**Referenced in:**
- `05-manager.md` (Registry)
- `07-implementation.md` (runtime/managed.rs)

---

### ScopeResolver

**Defined in:** `05-manager.md` (lines 429-435), updated in `08-correctness.md` (#4, #20)

**Original (non-object-safe):**
```rust
fn is_child_of(&self, child: &ScopeLevel, parent: &ScopeLevel) -> impl Future<Output = bool>
```

**Fixed (object-safe, #20):**
```rust
fn is_child_of<'a>(&'a self, child: &'a ScopeLevel, parent: &'a ScopeLevel) -> BoxFuture<'a, bool>
```

**Referenced in:**
- `05-manager.md` (scope containment, CachedScopeResolver, RequiresScopeResolver)
- `08-correctness.md` (#4 strict containment, #20 object-safety fix)

**Implementations:**
- `CachedScopeResolver` (moka cache wrapper)
- `RequiresScopeResolver` (panics if not configured)

---

### ResourceContext (action integration)

**Defined in:** `06-action-integration.md` (lines 12-24)

**Methods:**
- `resource<R: Resource>(&self) -> impl Future<Output = Result<ResourceHandle<R>, ActionError>> + Send`
- `credential<C: CredentialType>(&self) -> impl Future<Output = Result<C, ActionError>> + Send`

**Referenced in:**
- `06-action-integration.md` (ActionContext, TriggerContext implementations)

---

### EventTrigger (DX trait)

**Defined in:** `06-action-integration.md` (lines 156-182)

**Trait bound:** `Action`

**Associated types:**
- `Source: Resource`
- `Event: Serialize + DeserializeOwned`

**Methods:**
- `on_event(&self, source: &<Self::Source as Resource>::Lease, ctx: &TriggerContext) -> Result<Option<Self::Event>>`
- `on_error(&self, error: crate::Error, ctx: &TriggerContext) -> ErrorAction` (default: Reconnect)

**Referenced in:**
- `06-action-integration.md` (EventTrigger examples, engine-generated loop)

**Examples:**
- `IncomingMessageTrigger` (Telegram, lines 245-262)
- `OrderEventTrigger` (Redis Pub/Sub, lines 265-281)

---

### ResourceAction (scoped resource)

**Defined in:** `06-action-integration.md` (lines 288-313)

**Trait bound:** `Action`

**Associated types:**
- `Resource: Resource`

**Methods:**
- `configure(&self, ctx: &ActionContext) -> Result<<Self::Resource as Resource>::Config>`
- `topology(&self) -> ScopedTopology` (default: Resident)
- `cleanup(&self, ctx: &ActionContext) -> Result<()>` (default: noop)

**Referenced in:**
- `06-action-integration.md` (scoped resource in graph)

---

### Plugin System Traits

**Defined in:** `06-action-integration.md` (lines 347-393)

**Traits:**
- `Plugin` (key, manifest, resources, credentials, actions)
- `ResourceDescriptor` (key, manifest, config_schema, register, validate, unregister)
- `CredentialDescriptor` (key, manifest, schema, create, validate)
- `ActionDescriptor` (key, manifest, input/output/event schemas, create_action)

**Referenced in:**
- `06-action-integration.md` (PluginRegistry)

---

## Structs

### ResourceHandle<R: Resource>

**Defined in:** `03-infrastructure.md` (lines 12-113)

**Fields:**
- `inner: HandleInner<R>`
- `resource_key: ResourceKey`
- `topology_tag: &'static str`

**Methods:**
- `taint(&mut self)`
- `detach(mut self) -> Result<R::Lease, DetachError>` (updated in #2)
- `hold_duration(&self) -> Option<Duration>`

**Deref to:** `R::Lease`

**Referenced in:**
- All topology runtime acquire paths
- `06-action-integration.md` (ctx.resource() return type)
- `08-correctness.md` (#2 detach fix)

**Constructors (crate-internal):**
- `owned(lease, key, tag) -> Self`
- `guarded(lease, on_release, key, tag) -> Self`
- `shared(lease, on_release, key, tag) -> Self`

---

### HandleInner<R: Resource> (enum, 3 variants)

**Defined in:** `03-infrastructure.md` (lines 18-41)

**Variants:**
- `Owned(R::Lease)` — no cleanup, Drop = drop value
- `Guarded { value: Option<R::Lease>, on_release: Option<Box<dyn FnOnce(R::Lease, bool) + Send>>, tainted: bool, acquired_at: Instant }`
- `Shared { value: Arc<R::Lease>, on_release: Option<Box<dyn FnOnce(bool) + Send>>, tainted: bool, acquired_at: Instant }`

**Referenced in:**
- `03-infrastructure.md` (ResourceHandle Drop impl)
- `08-correctness.md` (#2 Deref unreachable fix)

**Used by topologies:**
- Pool → Guarded
- Resident → Owned
- Service (Cloned) → Owned
- Service (Tracked) → Guarded
- Transport → Guarded
- Exclusive → Shared

---

### LeaseGuard<L> (internal RAII)

**Defined in:** `03-infrastructure.md` (lines 208-252)

**Fields:**
- `lease: Option<L>`
- `tainted: bool`
- `poison: Arc<AtomicBool>`
- `on_release: Option<Box<dyn FnOnce(L, bool, Arc<AtomicBool>) + Send>>` (updated in #5)
- `resource_key: ResourceKey`
- `acquired_at: Instant`

**Methods:**
- `taint(&mut self)`
- `is_tainted(&self) -> bool`
- `hold_duration(&self) -> Duration`
- `detach(mut self) -> L`
- `poison_token(&self) -> PoisonToken`

**Deref to:** `L`

**Referenced in:**
- `03-infrastructure.md` (pool idle queue entries)
- `08-correctness.md` (#5 poison race fix)

**Note:** Post HandleInner redesign, LeaseGuard is only used internally by Pool's idle_queue, not at ResourceHandle level.

---

### PoisonToken

**Defined in:** `03-infrastructure.md` (lines 260-267), updated in `08-correctness.md` (#5)

**Fields:**
- `flag: Arc<AtomicBool>`

**Methods:**
- `poison(&self)`
- `is_poisoned(&self) -> bool`

**Referenced in:**
- `03-infrastructure.md` (LeaseGuard)
- `08-correctness.md` (#5 pass Arc to release_fn)

---

### AcquireOptions

**Defined in:** `03-infrastructure.md` (lines 274-313)

**Fields:**
- `intent: AcquireIntent`
- `deadline: Option<Instant>`
- `tags: SmallVec<[(Cow<'static, str>, Cow<'static, str>); 2]>`

**Methods:**
- `standard() -> Self`
- `with_intent(intent) -> Self`
- `with_deadline(deadline) -> Self`
- `with_tag(key, value) -> Self`

**Referenced in:**
- `05-manager.md` (Manager::acquire)
- `08-correctness.md` (#12 audit trail)

---

### InstanceMetrics

**Defined in:** `03-infrastructure.md` (lines 322-360)

**Fields (public):**
- `error_count: u64`
- `checkout_count: u64`
- `created_at: Instant`

**Fields (crate-internal):**
- `config_fingerprint: u64`
- `last_checkin: Instant`
- `total_hold_duration: Duration`

**Methods:**
- `age(&self) -> Duration`
- `idle_duration(&self) -> Duration` (crate-internal)
- `is_stale(&self, current_fingerprint: u64) -> bool` (crate-internal)
- `record_error(&mut self)` (crate-internal)
- `record_checkout(&mut self)` (crate-internal)
- `record_checkin(&mut self, hold: Duration)` (crate-internal)

**Referenced in:**
- `02-topology.md` (Pooled::recycle argument)
- `07-implementation.md` (pool/entry.rs)

---

### Cell<T: Send + Sync + 'static>

**Defined in:** `03-infrastructure.md` (lines 367-410), updated in `08-correctness.md` (#16, lines 1274-1313)

**Original (broken):** `ArcSwap<Option<T>>`

**Fixed:** `ArcSwapOption<T>`

**Methods:**
- `empty() -> Self`
- `new(value: T) -> Self`
- `load(&self) -> Option<Arc<T>>`
- `store(&self, value: T)`
- `swap(&self, value: T) -> Option<Arc<T>>`
- `take(&self) -> Option<Arc<T>>`

**Referenced in:**
- `03-infrastructure.md` (Resident runtime)
- `07-implementation.md` (resident/mod.rs)
- `08-correctness.md` (#16 ArcSwapOption fix)

**Issue:** Amendment #16 fixed `load()` returning `Arc<Option<T>>` (always Some) instead of `Option<Arc<T>>`.

---

### ReleaseQueue

**Defined in:** `03-infrastructure.md` (lines 418-554), updated in `08-correctness.md` (#1, #14)

**Original (broken):** Single `mpsc::Sender` + `Arc<AsyncMutex<Receiver>>`

**Fixed (#14):** N independent primary receivers per worker + fallback unbounded

**Fields:**
- `senders: Vec<mpsc::Sender<ReleaseTask>>`
- `next_worker: AtomicUsize`
- `fallback_tx: mpsc::UnboundedSender<ReleaseTask>`
- `metrics: Arc<ReleaseQueueMetrics>`
- `workers: Vec<JoinHandle<()>>`

**Methods:**
- `new(capacity: usize, num_workers: usize, cancel: CancellationToken) -> Self`
- `submit(&self, task: ReleaseTask)`
- `metrics(&self) -> &ReleaseQueueMetrics`
- `handle(&self) -> ReleaseQueueHandle`
- `shutdown(self) -> async`

**Referenced in:**
- `03-infrastructure.md` (release flow per topology)
- `05-manager.md` (ManagedResource)
- `07-implementation.md` (release_queue.rs)
- `08-correctness.md` (#1 silent drop fix, #14 mutex contention fix)

---

### ReleaseQueueMetrics

**Defined in:** `03-infrastructure.md` (lines 447-454), `08-correctness.md` (#1)

**Fields:**
- `submitted: AtomicU64`
- `fallback_used: AtomicU64`
- `dropped: AtomicU64` (MUST be 0 in healthy system)

**Referenced in:**
- `03-infrastructure.md` (ReleaseQueue)
- `08-correctness.md` (#1)

---

### ReleaseQueueHandle

**Defined in:** `03-infrastructure.md` (lines 557-584)

**Fields:**
- `senders: Vec<mpsc::Sender<ReleaseTask>>`
- `next_worker: Arc<AtomicUsize>`
- `fallback_tx: mpsc::UnboundedSender<ReleaseTask>`
- `metrics: Arc<ReleaseQueueMetrics>`

**Methods:**
- `submit(&self, task: ReleaseTask)`

**Referenced in:**
- `03-infrastructure.md` (ReleaseQueue::handle)

---

### Extensions

**Defined in:** `01-core.md` (lines 515-574), updated in `08-correctness.md` (#11, lines 929-979)

**Fields:**
- `map: HashMap<TypeId, Box<dyn Any + Send + Sync>>`
- `name_index: HashMap<&'static str, TypeId>` (debug only, #11)

**Methods:**
- `new() -> Self`
- `insert<T: Send + Sync + 'static>(&mut self, value: T)`
- `get<T: Send + Sync + 'static>(&self) -> Option<&T>`
- `debug_get_by_name(&self, type_name: &str) -> Option<&dyn Any>` (debug only, #11)

**Referenced in:**
- `01-core.md` (BasicCtx)
- `02-topology.md` (Pooled::prepare tenant context)
- `08-correctness.md` (#11 TypeId collision detection)

**Issue:** Amendment #11 adds debug-mode TypeId collision detection for duplicate crate versions.

---

### BasicCtx

**Defined in:** `01-core.md` (lines 577-582)

**Fields:**
- `scope: ScopeLevel`
- `execution_id: ExecutionId`
- `cancel: Option<CancellationToken>`
- `extensions: Extensions`

**Referenced in:**
- `01-core.md` (minimal Ctx impl for tests)

---

### RecoveryGate

**Defined in:** `04-recovery-resilience.md` (lines 10-178), updated in `08-correctness.md` (#6, #17, #22)

**Fields:**
- `state: Arc<ArcSwap<GateState>>` (Arc wrapper added in #17)
- `notify: Arc<Notify>` (Arc wrapper added in #17)
- `max_recovery_attempts: u32` (added in #6)

**Methods:**
- `new() -> Self`
- `try_begin(self: &Arc<Self>) -> Result<RecoveryTicket, RecoveryWaiter>` (implemented in #6)
- `resolve(&self, ticket: RecoveryTicket)`
- `fail_transient(&self, ticket: RecoveryTicket, error: Error, backoff: Duration)`
- `fail_permanent(&self, ticket: RecoveryTicket, error: Error)`
- `status(&self) -> GateStatus`

**Referenced in:**
- `04-recovery-resilience.md` (RecoveryGroup, WatchdogHandle)
- `05-manager.md` (Manager, RecoveryGroupRegistry)
- `08-correctness.md` (#6 CAS loop, #17 Arc for 'static, #22 Drop guard)

**Issues:**
- Amendment #6 implements `try_begin()` CAS loop (was `todo!()`).
- Amendment #17 wraps `state` and `notify` in `Arc` for RecoveryWaiter 'static.
- Amendment #22 adds Drop guard on RecoveryTicket.

---

### RecoveryTicket

**Defined in:** `04-recovery-resilience.md` (lines 183-189), updated in `08-correctness.md` (#22, lines 1609-1629)

**Fields:**
- `attempt: u32`
- `gate: Arc<RecoveryGate>` (added in #22)
- `resolved: bool` (added in #22)
- `_private: ()`

**Drop guard (added in #22):**
- Auto-fails with `fail_transient_internal` if dropped without `resolve/fail_*`
- `debug_assert!(false)` in debug mode

**Referenced in:**
- `04-recovery-resilience.md` (RecoveryGate::try_begin)
- `08-correctness.md` (#22 Drop guard implementation)

---

### RecoveryWaiter

**Defined in:** `04-recovery-resilience.md` (lines 213-243), updated in `08-correctness.md` (#17, lines 1325-1360)

**Original (borrowed):**
```rust
struct RecoveryWaiter<'a> {
    state: &'a ArcSwap<GateState>,
    notify: &'a Notify,
}
```

**Fixed (owned, 'static):**
```rust
struct RecoveryWaiter {
    state: Arc<ArcSwap<GateState>>,
    notify: Arc<Notify>,
}
```

**Methods:**
- `wait(&self) -> async Result<(), Arc<Error>>`

**Referenced in:**
- `04-recovery-resilience.md` (RecoveryGate::try_begin)
- `08-correctness.md` (#17 'static fix for tokio::spawn)

**Issue:** Amendment #17 changed from borrowed refs to Arc for tokio::spawn compatibility.

---

### RecoveryGroup

**Defined in:** `04-recovery-resilience.md` (lines 268-271)

**Fields:**
- `key: RecoveryGroupKey`
- `gate: Arc<RecoveryGate>`

**Referenced in:**
- `04-recovery-resilience.md` (RecoveryGroupRegistry)
- `05-manager.md` (registration builder)

---

### RecoveryGroupRegistry

**Defined in:** `04-recovery-resilience.md` (lines 274-287)

**Fields:**
- `groups: DashMap<String, Arc<RecoveryGate>>`

**Methods:**
- `get_or_create(&self, key: &RecoveryGroupKey) -> Arc<RecoveryGate>`

**Referenced in:**
- `05-manager.md` (Manager)

---

### WatchdogHandle

**Defined in:** `04-recovery-resilience.md` (lines 304-404), updated in `08-correctness.md` (#22)

**Fields:**
- `task: JoinHandle<()>`
- `cancel: CancellationToken`

**Methods:**
- `spawn<R: Resource>(resource, runtime, config, gate) -> Self` (recovery logic implemented in #22)
- `shutdown(self) -> async`

**Referenced in:**
- `04-recovery-resilience.md` (opt-in background probe)
- `08-correctness.md` (#22 recovery logic + ticket completion)

---

### AcquireResilience

**Defined in:** `04-recovery-resilience.md` (lines 563-633)

**Fields:**
- `timeout: Option<Duration>`
- `retry: Option<AcquireRetryConfig>`
- `circuit_breaker: Option<AcquireCircuitBreakerPreset>`

**Methods:**
- `build_chain<T: Send + 'static>(&self) -> Option<ResilienceChain<T>>`

**Referenced in:**
- `04-recovery-resilience.md` (acquire path resilience)
- `05-manager.md` (RegistrationBuilder, ManagedResource)

---

### Manager

**Defined in:** `05-manager.md` (lines 10-126), updated in `08-correctness.md` (#13, #24)

**Fields:**
- `registry: Registry`
- `recovery_groups: RecoveryGroupRegistry`
- `cancel: CancellationToken`
- `telemetry: Arc<dyn TelemetryService>`
- `resource_bus: Arc<EventBus<ResourceEvent>>`
- `memory_monitor: Option<Arc<Mutex<MemoryMonitor>>>`
- `pressure_snapshot: Arc<PressureSnapshot>` (added in #13, initialized in #24)
- `containment_mode: ContainmentMode` (added in #4)
- `scope_resolver: Arc<dyn ScopeResolver>` (added in #4)

**Methods:**
- `new(telemetry) -> Self`
- `with_memory_monitor(monitor, check_interval) -> Self` (updated in #13)
- `with_scope_resolver(resolver) -> Self` (added in #4)
- `with_simplified_scoping(self) -> Self` (added in #4)
- `resource_events(&self) -> &EventBus<ResourceEvent>`
- `acquire<R: Resource>(resource_id, ctx, options) -> async ResourceHandle<R>`
- `remove(resource_id) -> async Result<()>`
- `register<R: Resource>(resource) -> RegistrationBuilder<R, NeedsConfig>`

**Referenced in:**
- `05-manager.md` (RegistrationBuilder, Registry)
- `06-action-integration.md` (ResourceContext impl)
- `07-implementation.md` (manager/ module)
- `08-correctness.md` (#4 scope containment, #13 memory monitor, #24 initialization)

---

### RegistrationBuilder<R: Resource, State>

**Defined in:** `05-manager.md` (lines 152-283)

**States (typestate):**
- `NeedsConfig`
- `NeedsId`
- `NeedsTopology`
- `Ready<T>` (T = topology config type)

**Fields:**
- `resource: R`
- `config: Option<R::Config>`
- `resource_id: Option<ResourceId>`
- `scope: ScopeLevel`
- `recovery_group: Option<RecoveryGroupKey>`
- `acquire_resilience: Option<AcquireResilience>`
- `secondary_topologies: Vec<SecondaryTopology>`
- `_state: PhantomData<State>`

**Methods (by state):**
- `NeedsConfig::config(config) -> NeedsId`
- `NeedsId::id(id) -> NeedsTopology`
- `NeedsTopology::pool(config) -> Ready<pool::Config>` (if R: Pooled)
- `NeedsTopology::resident(config) -> Ready<resident::Config>` (if R: Resident where R::Lease: Clone)
- `NeedsTopology::service(config) -> Ready<service::Config>` (if R: Service)
- `NeedsTopology::transport(config) -> Ready<transport::Config>` (if R: Transport)
- `NeedsTopology::exclusive(config) -> Ready<exclusive::Config>` (if R: Exclusive)
- `NeedsTopology::event_source(config) -> Ready<event_source::Config>` (if R: EventSource)
- `NeedsTopology::daemon(config) -> Ready<daemon::Config>` (if R: Daemon)
- `Ready::also_event_source(config) -> Self` (if R: EventSource)
- `Ready::also_daemon(config) -> Self` (if R: Daemon)
- `Ready::build() -> async Result<()>`

**Optional methods (on NeedsTopology + Ready):**
- `scope(scope) -> Self`
- `recovery_group(key) -> Self`
- `acquire_resilience(config) -> Self`

**Referenced in:**
- `05-manager.md` (Manager::register)
- `08-correctness.md` (compile-time safety examples)

---

### Registry

**Defined in:** `05-manager.md` (lines 327-393), updated in `08-correctness.md` (#4, #18)

**Fields:**
- `by_type: DashMap<(TypeId, ResourceId), SmallVec<[ScopedRuntime; 4]>>`
- `by_key: DashMap<ResourceKey, TypeId>`

**Methods:**
- `get_typed<R: Resource>(id, scope, resolver, mode) -> async Arc<ManagedResource<R>>` (updated in #18)
- `get_erased(type_id, id, scope, resolver, mode) -> async Arc<dyn AnyManagedResource>` (async in #4)

**Referenced in:**
- `05-manager.md` (Manager)
- `08-correctness.md` (#4 async scope check, #18 Arc return instead of &)

---

### PressureSnapshot

**Defined in:** `08-correctness.md` (#13, lines 1049-1078)

**Fields:**
- `level: AtomicU8`

**Methods:**
- `load(&self) -> PressureLevel`
- `store(&self, level: PressureLevel)` (internal)

**Referenced in:**
- `05-manager.md` (Manager, pool maintenance loops)
- `08-correctness.md` (#13 lock-free memory pressure)

---

### PluginRegistry

**Defined in:** `06-action-integration.md` (lines 398-440)

**Fields:**
- `plugins: HashMap<String, Arc<dyn Plugin>>`
- `resources: HashMap<String, Arc<dyn ResourceDescriptor>>`
- `credentials: HashMap<String, Arc<dyn CredentialDescriptor>>`
- `actions: HashMap<String, Arc<dyn ActionDescriptor>>`

**Methods:**
- `install(&mut self, plugin: impl Plugin + 'static)`
- `load_resources_from_db(manager, db, platform) -> async Result<()>`
- `action_dependency_tree(action_key) -> DependencyTree`
- `plugin_catalog() -> Vec<PluginManifest>`
- `resource_catalog() -> Vec<ResourceManifest>`
- `action_catalog() -> Vec<ActionManifest>`

**Referenced in:**
- `06-action-integration.md` (plugin system)

---

### Topology-specific Runtime structs

Referenced in `07-implementation.md` (module layout):

- `pool::Runtime<R>` (pool/mod.rs)
- `resident::Runtime<R>` (resident/mod.rs)
- `service::Runtime<R>` (service/mod.rs)
- `transport::Runtime<R>` (transport/mod.rs)
- `exclusive::Runtime<R>` (exclusive/mod.rs)
- `event_source::Runtime<R>` (event_source/mod.rs)
- `daemon::Runtime<R>` (daemon/mod.rs)

All wrapped in:
- `TopologyRuntime<R>` (enum, 7 variants)
- `ManagedResource<R>` (contains TopologyRuntime)

---

### DaemonState<R: Resource>

**Defined in:** `08-correctness.md` (#15, lines 1232-1240)

**Fields:**
- `runtime: Option<R::Runtime>` (None only during recreate, replaced `mem::zeroed()` UB)
- `consecutive_failures: u32`
- `total_recreates: u32`
- `window_recreates: u32`
- `window_started: Option<Instant>`

**Referenced in:**
- `02-topology.md` (daemon restart loop)
- `08-correctness.md` (#8 recreate budget, #15 Option instead of zeroed)

---

## Enums

### ErrorKind

**Defined in:** `01-core.md` (lines 601-624)

**Variants:**
- `Transient` — connection refused, timeout, temporary network issue
- `Permanent` — auth failed, invalid config, database not found
- `Exhausted { retry_after: Option<Duration> }` — budget/quota/rate limit exceeded
- `Backpressure` — pool full, semaphore exhausted
- `NotFound` — resource not found in registry
- `Cancelled` — operation cancelled (CancellationToken)

**Referenced in:**
- `01-core.md` (Error struct)
- `04-recovery-resilience.md` (ResilienceError mapping)

---

### ErrorScope

**Defined in:** `01-core.md` (lines 627-637)

**Variants:**
- `Resource` (default) — entire resource broken, taint appropriate
- `Target { id: String }` — error scoped to specific target (e.g. bot blocked in one chat)

**Referenced in:**
- `01-core.md` (Error struct, ClassifyError macro)

---

### BrokenCheck

**Defined in:** `02-topology.md` (lines 69-77)

**Variants:**
- `Healthy` — instance OK
- `Broken(Cow<'static, str>)` — instance broken, reason for diagnostics
- `NeedsAsyncCheck` — sync check insufficient, pool will call async Resource::check()

**Referenced in:**
- `02-topology.md` (Pooled::is_broken)
- `07-implementation.md` (pool/acquire.rs)
- `08-correctness.md` (#7 acquire retry)

---

### RecycleDecision

**Defined in:** `02-topology.md` (lines 79-84)

**Variants:**
- `Keep` — return to idle queue
- `Drop` — destroy, pool creates new when needed

**Referenced in:**
- `02-topology.md` (Pooled::recycle)
- `07-implementation.md` (pool/release.rs)

---

### TokenMode

**Defined in:** `02-topology.md` (lines 353-358)

**Variants:**
- `Cloned` — token is cheap clone, HandleInner::Owned, release_token = noop
- `Tracked` — token is tracked resource, HandleInner::Guarded, release_token called via ReleaseQueue

**Referenced in:**
- `02-topology.md` (Service::TOKEN_MODE)
- `09-topology-guide.md` (Service section)

---

### GateState

**Defined in:** `04-recovery-resilience.md` (lines 20-29), updated in `08-correctness.md` (#6, lines 523-534)

**Variants:**
- `Idle` — resource OK, no recovery
- `InProgress { attempt: u32, started: Instant }` — recovery in progress
- `Failed { error: Arc<Error>, until: Instant, attempt: u32 }` — transient failure, retry after backoff (attempt added in #6)
- `PermanentlyFailed { error: Arc<Error> }` — permanent failure, no retry

**Referenced in:**
- `04-recovery-resilience.md` (RecoveryGate, RecoveryWaiter)
- `08-correctness.md` (#6 attempt tracking for escalation)

---

### GateStatus

**Defined in:** `04-recovery-resilience.md` (lines 245-250)

**Variants:**
- `Healthy`
- `Recovering { attempt: u32 }`
- `Failed { retry_in: Duration }`
- `PermanentlyFailed`

**Referenced in:**
- `04-recovery-resilience.md` (RecoveryGate::status)

---

### RestartPolicy

**Defined in:** `02-topology.md` (lines 665-731), updated in `08-correctness.md` (#8)

**Variants:**
- `Never` — never restart, run() returns Err → permanent failure
- `OnFailure { max_restarts: u32, backoff: BackoffConfig }` — restart on failure only
- `Always { max_restarts: u32, backoff: BackoffConfig }` — always restart (even Ok)

**Referenced in:**
- `02-topology.md` (daemon::Config, restart loop)
- `08-correctness.md` (#8 with RecreateBudget)

---

### ContainmentMode

**Defined in:** `05-manager.md` (lines 411-421), `08-correctness.md` (#4, lines 296-307)

**Variants:**
- `Strict` (default) — Organization X contains ONLY its Projects, requires ScopeResolver
- `Simplified` — Organization contains ALL Projects, single-tenant/dev only

**Referenced in:**
- `05-manager.md` (Manager, Registry scope checks)
- `08-correctness.md` (#4 strict containment by default)

---

### TopologyKind

**Defined in:** `07-implementation.md` (lines 23), referenced throughout

**Variants:**
- `Pool`
- `Resident`
- `Service`
- `Transport`
- `Exclusive`
- `EventSource`
- `Daemon`

**Referenced in:**
- `05-manager.md` (AnyManagedResource::topology_kind, default resilience)
- `07-implementation.md` (TopologyRuntime)

---

### ScopedTopology

**Defined in:** `06-action-integration.md` (lines 315-323)

**Variants:**
- `Resident` (default)
- `Pool(pool::Config)`
- `Exclusive`

**Referenced in:**
- `06-action-integration.md` (ResourceAction::topology)

---

### ErrorAction

**Defined in:** `06-action-integration.md` (lines 184-190)

**Variants:**
- `Reconnect` — re-acquire resource, continue loop
- `Stop` — stop trigger
- `Ignore` — ignore error, continue loop

**Referenced in:**
- `06-action-integration.md` (EventTrigger::on_error)

---

### SecondaryTopology

**Defined in:** `05-manager.md` (lines 256-264)

**Variants:**
- `EventSource(event_source::Config)`
- `Daemon(daemon::Config)`

**Referenced in:**
- `05-manager.md` (RegistrationBuilder, hybrid resources)

---

### ReloadResult

**Defined in:** `05-manager.md` (lines 591-595), `08-correctness.md` (#9, lines 777-783)

**Variants:**
- `Applied` — config applied successfully
- `RolledBack { reason: String }` — rollback, old config active
- `Skipped { reason: String }` — config unchanged

**Referenced in:**
- `05-manager.md` (TopologyRuntime::on_config_changed)
- `08-correctness.md` (#9 two-phase reload)

---

### DetachError

**Defined in:** `03-infrastructure.md` (lines 97-103), `08-correctness.md` (#2, lines 150-156)

**Variants:**
- `AlreadyConsumed` — framework bug
- `NotDetachable` — Owned or Shared handles don't support detach

**Referenced in:**
- `03-infrastructure.md` (ResourceHandle::detach)
- `08-correctness.md` (#2 Result return type)

---

### PressureLevel

**Defined in:** `08-correctness.md` (#13, lines 1051-1058)

**Variants:**
- `Normal = 0`
- `Moderate = 1`
- `High = 2`
- `Critical = 3`

**Repr:** `#[repr(u8)]`

**Referenced in:**
- `05-manager.md` (PressureSnapshot, pool maintenance)
- `08-correctness.md` (#13 AtomicU8 encoding)

---

### AcquireCircuitBreakerPreset

**Defined in:** `04-recovery-resilience.md` (lines 600-607)

**Variants:**
- `Standard` — 5 failures, 30s reset (default)
- `Fast` — 3 failures, 10s reset (latency-sensitive)
- `Slow` — 10 failures, 60s reset (tolerant)

**Referenced in:**
- `04-recovery-resilience.md` (AcquireResilience)

---

### BackoffKind

**Defined in:** `04-recovery-resilience.md` (lines 581-585)

**Variants:**
- `Exponential`
- `Fixed`
- `Linear`

**Referenced in:**
- `04-recovery-resilience.md` (AcquireRetryConfig)

---

### MemoryPressure

**Defined in:** Referenced in `08-correctness.md` (#13, lines 1104-1107)

**Variants:**
- `Normal`
- `Moderate`
- `High`
- `Critical`

**Referenced in:**
- `05-manager.md` (MemoryMonitor::check_pressure)
- `08-correctness.md` (#13 pressure snapshot background task)

---

## Type Aliases & Associated Types

### Associated Types (from Resource trait)

**Resource::Config** → `ResourceConfig`
- Defined in Resource trait (01-core.md line 13)
- Referenced in: all topology impls, RegistrationBuilder, ManagedResource

**Resource::Runtime** → `Send + Sync + 'static`
- Defined in Resource trait (01-core.md line 19)
- Internal managed instance
- Referenced in: all topology traits, TopologyRuntime

**Resource::Lease** → `Send + Sync + 'static`
- Defined in Resource trait (01-core.md line 28)
- Caller-facing handle, Deref target in ResourceHandle
- For most topologies: `= Runtime`
- Service: `= Token` (TelegramBotHandle)
- Transport: `= Session` (SshSession)
- **Change 9 (02-topology.md):** Resident requires `where Self::Lease: Clone` (not `Runtime: Clone`)

**Resource::Error** → `std::error::Error + Send + Sync + Into<crate::Error> + 'static`
- Defined in Resource trait (01-core.md line 32)
- Typed error, impl Into<nebula_resource::Error>

**Resource::Credential** → `Credential`
- Defined in Resource trait (01-core.md line 50), added in #3
- Framework resolves before create()

### Associated Types (from topology traits)

**EventSource::Event** → `Send + Clone + 'static`
- Defined in EventSource trait (02-topology.md line 546)

**EventSource::Subscription** → `Send + 'static`
- Defined in EventSource trait (02-topology.md line 549)

### Key Type Aliases

**ResourceKey**
- `= DomainKey<ResourceDomain>` (from nebula-core)
- Compile-time validated via `resource_key!()` macro
- Referenced in: Resource::KEY, ResourceHandle, Registry

**ResourceId**
- String-based identifier for resource instances
- Referenced in: Registry, Manager::acquire, RegistrationBuilder

**ExecutionId**
- Unique execution ID from Ctx
- Referenced in: Ctx trait, audit trail

**ScopeLevel** (from nebula-core)
- `Global | Organization(_) | Project(_) | Workflow(_) | Execution(_) | Action(_, _)`
- Referenced in: Ctx trait, scope containment, Registry lookup

**CancellationToken**
- From tokio_util
- Referenced in: Ctx::cancellation, Daemon::run, Manager::cancel

---

## Naming Inconsistencies

### 1. "Driver" vs "Resource" terminology

**Status:** Fully resolved in amendment #1 (subtask-1-1)

All "Driver" terminology has been replaced with "Resource" across all plan files. No remaining instances found.

### 2. Resident health check naming

**Status:** Fixed in amendment #23 (#23, 08-correctness.md)

**Issue:** `Resident::is_alive()` (sync) vs `Resource::check()` (async) naming ambiguity.

**Resolution:**
- Renamed `is_alive()` → `is_alive_sync()` with explicit O(1) no-I/O contract.
- Clarified: for I/O-based checks, use `Resource::check()` + `stale_after()`.

### 3. Token vs Lease terminology (Service topology)

**Status:** Consistent

Service trait documentation sometimes uses "token" colloquially, but the actual type is `Self::Lease` (consistent with Resource trait). No separate `type Token` associated type.

**Clarification:**
- `Service::acquire_token()` returns `Self::Lease`
- "Token" is used descriptively in docs (e.g., "TelegramBotHandle token")
- Type system uses `Lease` uniformly

### 4. Session vs Lease terminology (Transport topology)

**Status:** Consistent

Transport trait documentation uses "session" colloquially, but the actual type is `Self::Lease` (consistent with Resource trait). No separate `type Session` associated type.

**Clarification:**
- `Transport::open_session()` returns `Self::Lease`
- "Session" is used descriptively in docs (e.g., "SshSession")
- Type system uses `Lease` uniformly

---

## Type Signature Mismatches

### 1. CredentialStore object-safety

**File:** `01-core.md` (original), `08-correctness.md` (#19)

**Original (non-object-safe):**
```rust
fn resolve<C: Credential>(&self, scope: &ScopeLevel) -> impl Future<Output = Result<C, CredentialError>>
```

**Fixed:**
```rust
fn resolve_erased(&self, scope: &ScopeLevel, kind: &'static str)
  -> BoxFuture<'_, Result<Box<dyn Any + Send + Sync>, CredentialError>>
```

**Extension trait (typed):**
```rust
trait CredentialStoreExt {
    fn resolve<C: Credential + 'static>(&self, scope: &ScopeLevel) -> impl Future<...>
}
```

**Impact:** `dyn CredentialStore` now valid, CredentialCtx object-safe.

---

### 2. ScopeResolver object-safety

**File:** `05-manager.md` (original), `08-correctness.md` (#20)

**Original (non-object-safe):**
```rust
fn is_child_of(&self, child: &ScopeLevel, parent: &ScopeLevel) -> impl Future<Output = bool>
```

**Fixed:**
```rust
fn is_child_of<'a>(&'a self, child: &'a ScopeLevel, parent: &'a ScopeLevel) -> BoxFuture<'a, bool>
```

**Impact:** `Arc<dyn ScopeResolver>` now valid, CachedScopeResolver can wrap trait object.

---

### 3. RecoveryWaiter lifetime

**File:** `04-recovery-resilience.md` (original), `08-correctness.md` (#17)

**Original (non-'static):**
```rust
struct RecoveryWaiter<'a> {
    state: &'a ArcSwap<GateState>,
    notify: &'a Notify,
}
```

**Fixed:**
```rust
struct RecoveryWaiter {
    state: Arc<ArcSwap<GateState>>,
    notify: Arc<Notify>,
}
```

**Impact:** RecoveryWaiter now 'static + Send, compatible with tokio::spawn.

---

### 4. Registry::get_typed return type

**File:** `05-manager.md` (original), `08-correctness.md` (#18)

**Original:**
```rust
pub fn get_typed<R: Resource>(&self, ...) -> Result<&ManagedResource<R>>
```

**Issue:** Can't borrow self through await in async fn after scope_is_compatible became async (#4).

**Fixed:**
```rust
pub async fn get_typed<R: Resource>(&self, ...) -> Result<Arc<ManagedResource<R>>>
```

**Impact:** Returns Arc clone (one allocation), but compatible with async scope check.

---

### 5. LeaseGuard on_release signature

**File:** `03-infrastructure.md` (original), `08-correctness.md` (#5)

**Original:**
```rust
on_release: Box<dyn FnOnce(L, bool) + Send>
//                        ^lease, ^tainted_at_drop
```

**Issue:** Poison set after `tainted` snapshot but before `on_release` call → race.

**Fixed:**
```rust
on_release: Box<dyn FnOnce(L, bool, Arc<AtomicBool>) + Send>
//                        ^lease, ^tainted_at_drop, ^poison_flag
```

**Impact:** Final taint check in ReleaseQueue worker, narrower race window.

---

### 6. Cell<T> type wrapper

**File:** `03-infrastructure.md` (original), `08-correctness.md` (#16)

**Original (broken):**
```rust
struct Cell<T> {
    inner: ArcSwap<Option<T>>
}
// load_arc() returns Arc<Option<T>> — is_some() always true!
```

**Fixed:**
```rust
struct Cell<T> {
    inner: ArcSwapOption<T>
}
// load_full() returns Option<Arc<T>> — correct semantics
```

**Impact:** Resident acquire now correctly detects uninitialized cell.

---

### 7. Daemon runtime placeholder

**File:** `02-topology.md` (original), `08-correctness.md` (#15)

**Original (UB):**
```rust
let old = std::mem::replace(runtime, unsafe { std::mem::zeroed() });
// R::Runtime may have Arc/JoinHandle — zeroed = UB on drop!
```

**Fixed:**
```rust
struct DaemonState<R> {
    runtime: Option<R::Runtime>,  // None only during recreate
}
let old = state.runtime.take().ok_or(...)?;
```

**Impact:** No UB, sound memory safety.

---

### 8. Resident trait bound on Runtime vs Lease

**File:** `02-topology.md`, `05-manager.md`

**Original (implied):**
```rust
pub trait Resident: Resource where Self::Runtime: Clone
```

**Corrected (Change 9):**
```rust
pub trait Resident: Resource where Self::Lease: Clone
```

**Rationale:** Lease is the cloned type (acquire = clone Lease), not Runtime. For most Resident resources `Lease = Runtime`, but the bound should be on the actual cloned type.

**Impact:** RegistrationBuilder::resident() now requires `where R::Lease: Clone`.

---

### 9. ReleaseQueue worker contention

**File:** `03-infrastructure.md` (original), `08-correctness.md` (#14)

**Original (broken parallelism):**
```rust
// Single Arc<AsyncMutex<Receiver>> shared by N workers
task = async { rx.lock().await.recv().await } => { ... }
// All workers contend on one mutex — parallelism = 1!
```

**Fixed:**
```rust
// N independent receivers, each worker owns one (no mutex on hot path)
let (tx, rx) = mpsc::channel(per_worker_capacity);
senders.push(tx);
tokio::spawn(async move {
    loop {
        tokio::select! {
            task = rx.recv() => { ... }  // ← own rx, no lock!
            task = fallback_rx.lock().await.recv() => { ... }  // ← only for overflow
        }
    }
})
```

**Impact:** True parallel processing in ReleaseQueue (4 workers = 4× throughput for heavy recycle like Browser).

---

## Driver Terminology Issues

**Status:** All resolved in amendment #1 (subtask-1-1, e0cfe2e3)

**Original instances found and replaced:**
- ~~"Driver terminology and naming consistency"~~ → "Resource terminology..."
- All plan files audited, no remaining "Driver" references.

**Verification:**
```bash
grep -r "Driver" crates/resource/plans/*.md
# Result: 0 matches (excluding this cross-reference catalog)
```

**Conclusion:** No "Driver" terminology remains in plan files. All references use "Resource" consistently.

---

## Summary Statistics

**Total types cataloged:** 100+

**Breakdown:**
- **Traits:** 20 (Resource, 7 topology traits, 5 extension traits, 7 plugin traits)
- **Structs:** 40+ (ResourceHandle, Manager, Registry, primitives, topology runtimes)
- **Enums:** 15 (errors, states, policies, modes)
- **Associated Types:** 5 (Config, Runtime, Lease, Error, Credential)

**Critical amendments affecting types:**
- #3: Credential design (new Credential trait + CredentialCtx)
- #4: Scope containment (ContainmentMode, ScopeResolver)
- #5: PoisonToken race (on_release signature change)
- #6: RecoveryGate CAS (try_begin implementation)
- #9: Config reload rollback (ReloadResult enum)
- #15: Daemon mem::zeroed UB (DaemonState<Option<Runtime>>)
- #16: Cell type fix (ArcSwapOption)
- #17: RecoveryWaiter 'static (Arc wrappers)
- #18: Registry async (Arc return type)
- #19: CredentialStore object-safety (type erasure)
- #20: ScopeResolver object-safety (BoxFuture)
- #23: Resident is_alive rename (is_alive_sync)

**Files with most type definitions:**
1. `01-core.md` — 15 types (Resource, Ctx, Error, Credential, Extensions)
2. `02-topology.md` — 20 types (7 topology traits + config enums)
3. `03-infrastructure.md` — 18 types (ResourceHandle, primitives, ReleaseQueue)
4. `05-manager.md` — 12 types (Manager, Registry, RegistrationBuilder)
5. `04-recovery-resilience.md` — 10 types (RecoveryGate, WatchdogHandle)

**Cross-file dependencies (most referenced types):**
1. `Resource` trait — referenced in 8/9 files
2. `ResourceHandle<R>` — referenced in 6/9 files
3. `Ctx` trait — referenced in 7/9 files
4. `Error` enum — referenced in 8/9 files
5. `ScopeLevel` (from nebula-core) — referenced in 5/9 files

---

**End of Cross-Reference Catalog**
