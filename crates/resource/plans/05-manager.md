# 05 — Manager: registration, acquire, lifecycle orchestration

Correctness amendments integrated: #4 (scope cross-tenant), #9 (config no rollback),
#10 (service drain no deadline), #12 (audit trail), #13 (MemoryMonitor Mutex),
#18 (Registry::get_typed async), #20 (ScopeResolver object-safe), #24 (pressure_snapshot init).

---

## Manager

Central orchestrator. Регистрирует ресурсы, выдаёт handles, управляет lifecycle.

```rust
pub struct Manager {
    /// Type-erased resource runtimes, indexed by (TypeId, ResourceId).
    registry: Registry,
    /// Shared recovery gates per backend.
    recovery_groups: RecoveryGroupRegistry,
    /// Global cancellation. Framework owns lifecycle.
    cancel: CancellationToken,
    /// Observability: metrics + execution recording.
    telemetry: Arc<dyn TelemetryService>,
    /// Lifecycle events (Registered, HealthChanged, Recovery, etc.).
    resource_bus: Arc<EventBus<ResourceEvent>>,
    /// Optional: adaptive pool sizing under memory pressure.
    /// Mutex locked ТОЛЬКО фоновым task-ом раз в check_interval.
    /// Maintenance loops читают pressure_snapshot (lock-free AtomicU8).
    memory_monitor:    Option<Arc<Mutex<MemoryMonitor>>>,
    pressure_snapshot: Arc<PressureSnapshot>,
    /// Scope containment mode. Default: Strict (multi-tenant safe).
    containment_mode: ContainmentMode,
    /// Resolver для strict containment. Default: RequiresScopeResolver (паникует).
    scope_resolver:   Arc<dyn ScopeResolver>,
}

impl Manager {
    pub fn new(telemetry: Arc<dyn TelemetryService>) -> Self {
        Self {
            registry:          Registry::new(),
            recovery_groups:   RecoveryGroupRegistry::new(),
            cancel:            CancellationToken::new(),
            telemetry,
            resource_bus:      Arc::new(EventBus::new(256)),
            memory_monitor:    None,
            // Исправлено (#15/Manager): все поля инициализированы в new().
            pressure_snapshot: Arc::new(PressureSnapshot::new(PressureLevel::Normal)),
            containment_mode:  ContainmentMode::Strict,
            scope_resolver:    Arc::new(RequiresScopeResolver),
        }
    }

    /// Adaptive pool sizing под memory pressure.
    ///
    /// `check_interval`: как часто проверять давление. Default: 10 seconds.
    /// Maintenance loops читают pressure_snapshot (AtomicU8, lock-free).
    /// Mutex на MemoryMonitor лочится ТОЛЬКО фоновым task-ом.
    pub fn with_memory_monitor(mut self, monitor: MemoryMonitor, check_interval: Duration) -> Self {
        let monitor  = Arc::new(Mutex::new(monitor));
        let snapshot = Arc::clone(&self.pressure_snapshot);
        let cancel   = self.cancel.child_token();

        let mon_clone = Arc::clone(&monitor);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(check_interval) => {
                        if let Ok(mut m) = mon_clone.try_lock() {
                            let level = match m.check_pressure() {
                                MemoryPressure::Normal   => PressureLevel::Normal,
                                MemoryPressure::Moderate => PressureLevel::Moderate,
                                MemoryPressure::High     => PressureLevel::High,
                                MemoryPressure::Critical => PressureLevel::Critical,
                            };
                            snapshot.store(level);
                        }
                    }
                }
            }
        });

        self.memory_monitor = Some(monitor);
        self
    }

    /// Multi-tenant production. Strict containment (default).
    pub fn with_scope_resolver(mut self, resolver: Arc<dyn ScopeResolver>) -> Self {
        self.scope_resolver = Arc::new(CachedScopeResolver::new(resolver, 10_000));
        self
    }

    /// Single-tenant or development deployments ONLY.
    /// Simplified containment: Organization contains ALL Projects (no cross-tenant check).
    ///
    /// Use when:
    /// - Single-tenant self-hosted deployment (one org, no isolation needed).
    /// - Local development / integration tests.
    ///
    /// DO NOT use in multi-tenant production — resources from Org A would be
    /// accessible to Org B. See amendment #4.
    pub fn with_simplified_scoping(mut self) -> Self {
        self.containment_mode = ContainmentMode::Simplified;
        self
    }

    pub fn resource_events(&self) -> &EventBus<ResourceEvent> {
        &self.resource_bus
    }

    /// Primary typed acquire. Topology dispatch внутри.
    /// Возвращает ResourceHandle<R> — unified, topology-agnostic.
    ///
    /// Hot path: typed fast path via downcast_ref (zero allocation).
    pub async fn acquire<R: Resource>(
        &self,
        resource_id: ResourceId,
        ctx: &dyn Ctx,
        options: AcquireOptions,
    ) -> Result<ResourceHandle<R>> {
        let managed = self.registry.get_typed::<R>(
            resource_id, ctx.scope(),
            self.scope_resolver.as_ref(),
            self.containment_mode,
        ).await?;
        managed.acquire(ctx, options).await
    }

    /// Remove resource. Graceful shutdown.
    pub async fn remove(&self, resource_id: ResourceId) -> Result<()> {
        self.registry.remove(resource_id).await
    }

    /// Создать ResourceMetrics для конкретного resource.
    fn metrics_for(&self, resource_key: &ResourceKey, resource_id: &str) -> ResourceMetrics {
        let adapter = TelemetryAdapter::new(self.telemetry.metrics_arc());
        ResourceMetrics::new(adapter, resource_key, resource_id)
    }
}
```

---

## Registration — typestate builder

Invalid states не компилируются. Забыл config → compile error. Забыл topology → compile error.

**Minimal registration (3 lines):**

```rust
// Simple case — config + topology + build. That's it.
manager.register(Postgres).config(pg_config).id(id)
    .pool(pool::Config::default()).build().await?;
```

**Full registration with all options:**

```rust
// Usage:
manager.register(Postgres)
    .config(pg_config)                                       // NeedsConfig → NeedsId
    .id(resource_id)                                         // NeedsId → NeedsTopology
    .scope(ScopeLevel::Organization(org_id))                 // optional, any state
    .recovery_group(RecoveryGroupKey::new("pg-primary"))     // optional
    .acquire_resilience(AcquireResilience {                  // optional
        timeout: Some(Duration::from_secs(5)),
        retry: Some(AcquireRetryConfig { max_attempts: 3, ..Default::default() }),
        circuit_breaker: Some(AcquireCircuitBreakerPreset::Standard),
    })
    .pool(pool::Config { max_size: 20, ..Default::default() })  // NeedsTopology → Ready
    .build().await?;                                         // Ready → registered

// Typestate:

pub struct RegistrationBuilder<R: Resource, State> {
    resource:              R,
    config:                Option<R::Config>,
    resource_id:           Option<ResourceId>,
    scope:                 ScopeLevel,
    recovery_group:        Option<RecoveryGroupKey>,
    acquire_resilience:    Option<AcquireResilience>,
    secondary_topologies:  Vec<SecondaryTopology>,  // for hybrid resources
    _state:                PhantomData<State>,
}

pub struct NeedsConfig;
pub struct NeedsId;
pub struct NeedsTopology;
pub struct Ready<T>;  // T = topology config type

impl Manager {
    pub fn register<R: Resource>(&self, resource: R) -> RegistrationBuilder<R, NeedsConfig> {
        RegistrationBuilder {
            resource, config: None, resource_id: None,
            scope: ScopeLevel::Global, recovery_group: None,
            acquire_resilience: None, secondary_topologies: Vec::new(),
            _state: PhantomData,
        }
    }
}

impl<R: Resource> RegistrationBuilder<R, NeedsConfig> {
    pub fn config(mut self, config: R::Config) -> RegistrationBuilder<R, NeedsId> {
        self.config = Some(config);
        RegistrationBuilder { config: self.config, _state: PhantomData, ..self }
    }
}

impl<R: Resource> RegistrationBuilder<R, NeedsId> {
    pub fn id(mut self, id: ResourceId) -> RegistrationBuilder<R, NeedsTopology> {
        self.resource_id = Some(id);
        RegistrationBuilder { resource_id: self.resource_id, _state: PhantomData, ..self }
    }
}

// Optional methods — available on NeedsTopology and Ready states.
// NOT available on NeedsConfig or NeedsId (no resource identity yet).
// Implemented via marker trait to avoid duplicating impls.
trait HasIdentity {}
impl HasIdentity for NeedsTopology {}
impl<T> HasIdentity for Ready<T> {}

impl<R: Resource, S: HasIdentity> RegistrationBuilder<R, S> {
    pub fn scope(mut self, scope: ScopeLevel) -> Self { self.scope = scope; self }
    pub fn recovery_group(mut self, key: RecoveryGroupKey) -> Self { self.recovery_group = Some(key); self }
    pub fn acquire_resilience(mut self, config: AcquireResilience) -> Self {
        self.acquire_resilience = Some(config); self
    }
}

// 7 topology finishers:
impl<R: Resource + Pooled> RegistrationBuilder<R, NeedsTopology> {
    pub fn pool(self, config: pool::Config) -> RegistrationBuilder<R, Ready<pool::Config>> { ... }
}

impl<R: Resource + Resident> RegistrationBuilder<R, NeedsTopology>
where R::Lease: Clone  // Change 9: Clone bound on Lease, not Runtime
{
    pub fn resident(self, config: resident::Config) -> RegistrationBuilder<R, Ready<resident::Config>> { ... }
}

impl<R: Resource + Service> RegistrationBuilder<R, NeedsTopology> {
    pub fn service(self, config: service::Config) -> RegistrationBuilder<R, Ready<service::Config>> { ... }
}

impl<R: Resource + Transport> RegistrationBuilder<R, NeedsTopology> {
    pub fn transport(self, config: transport::Config) -> RegistrationBuilder<R, Ready<transport::Config>> { ... }
}

impl<R: Resource + Exclusive> RegistrationBuilder<R, NeedsTopology> {
    pub fn exclusive(self, config: exclusive::Config) -> RegistrationBuilder<R, Ready<exclusive::Config>> { ... }
}

impl<R: Resource + EventSource> RegistrationBuilder<R, NeedsTopology> {
    pub fn event_source(self, config: event_source::Config) -> RegistrationBuilder<R, Ready<event_source::Config>> { ... }
}

impl<R: Resource + Daemon> RegistrationBuilder<R, NeedsTopology> {
    pub fn daemon(self, config: daemon::Config) -> RegistrationBuilder<R, Ready<daemon::Config>> { ... }
}

// Secondary topology additions — available on Ready state.
// Primary topology already selected. Secondary adds capabilities.
// Each also_* requires the resource to impl the corresponding trait.
impl<R: Resource + EventSource, T> RegistrationBuilder<R, Ready<T>> {
    pub fn also_event_source(mut self, config: event_source::Config) -> Self {
        self.secondary_topologies.push(SecondaryTopology::EventSource(config));
        self
    }
}

impl<R: Resource + Daemon, T> RegistrationBuilder<R, Ready<T>> {
    pub fn also_daemon(mut self, config: daemon::Config) -> Self {
        self.secondary_topologies.push(SecondaryTopology::Daemon(config));
        self
    }
}

/// Secondary topologies that can be added to any primary topology.
/// Only EventSource and Daemon make sense as secondaries:
///   - EventSource: adds incoming event stream (subscribe/recv).
///   - Daemon: adds background process (run loop, restart policy).
/// Other topologies are mutually exclusive (you can't be Pool AND Exclusive).
enum SecondaryTopology {
    EventSource(event_source::Config),
    Daemon(daemon::Config),
}

// Build — only from Ready state:
impl<R: Resource, T> RegistrationBuilder<R, Ready<T>> {
    pub async fn build(self) -> Result<()> {
        // 1. Validate config.
        // 2. Create ManagedResource<R> with:
        //    - ReleaseQueue (worker count by topology — see 03-infrastructure)
        //    - ResourceMetrics (from Manager.telemetry)
        //    - CancellationToken (child of Manager.cancel)
        //    - AcquireResilience → build ResilienceChain (if specified)
        // 3. Register in Registry (type-erased via AnyManagedResource).
        // 4. Setup recovery group (if specified).
        // 5. Setup secondary topologies (EventSource subscription, Daemon runner).
        // 6. Warmup (if pool + warmup strategy).
        // 7. Start daemon (if Daemon topology — primary or secondary).
        // 8. Emit ResourceEvent::Registered via resource_bus.
        Ok(())
    }
}
```

**Compile-time safety examples:**

```rust
// ✓ Compiles: Postgres impl Pooled.
manager.register(Postgres).config(cfg).id(id).pool(pool_cfg).build().await?;

// ✗ Compile error: Postgres does NOT impl Resident.
manager.register(Postgres).config(cfg).id(id).resident(res_cfg).build().await?;
//                                              ^^^^^^^^ ERROR: Resident not implemented

// ✗ Compile error: forgot topology.
manager.register(Postgres).config(cfg).id(id).build().await?;
//                                            ^^^^^ ERROR: expected .pool() or similar

// ✗ Compile error: forgot config.
manager.register(Postgres).id(id).pool(pool_cfg).build().await?;
//                         ^^ ERROR: expected .config() first

// ✓ Hybrid: Telegram Bot = Service (primary) + EventSource + Daemon (secondary).
manager.register(TelegramBot)
    .config(tg_config)
    .id(tg_id)
    .service(service::Config::default())               // primary topology
    .also_event_source(event_source::Config::default()) // secondary
    .also_daemon(daemon::Config::default())             // secondary
    .build().await?;

// ✗ Compile error: Postgres does NOT impl Daemon.
manager.register(Postgres).config(cfg).id(id).pool(pool_cfg)
    .also_daemon(daemon::Config::default())
//  ^^^^^^^^^^^ ERROR: Daemon not implemented for Postgres
    .build().await?;
```

---

## Registry — type-erased, scope-aware lookup

**Scope resolution order** (most specific wins, with fallback):

```
Action(exec_id, node_id) → Execution(exec_id) → Workflow(wf_id) → Project(proj_id) → Organization(org_id) → Global
```

When `acquire()` is called with a request scope, the registry finds the most specific
registered scope that is compatible. Example: resource registered at `Organization("acme")`,
requested from `Execution("run-42")` within that org → matches if ScopeResolver confirms
the execution belongs to that organization. If no compatible scope found → `Error::NotFound`.

Two acquire paths: typed hot (Arc clone) и erased cold.

> **Amendment #18:** `get_typed()` returns `Arc<ManagedResource<R>>` (not `&ref`) because
> scope resolution is async (ScopeResolver). One Arc clone on hot path — acceptable.
>
> **Amendment #20:** `ScopeResolver::is_child_of` returns `BoxFuture` for object-safety
> (`Arc<dyn ScopeResolver>` required by `CachedScopeResolver`).

```rust
/// Type erasure trait. Every ManagedResource<R> implements this.
pub trait AnyManagedResource: Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
    fn resource_key(&self) -> &ResourceKey;
    fn topology_kind(&self) -> TopologyKind;
    fn health_status(&self) -> HealthStatus;
    fn config_fingerprint(&self) -> u64;
    fn shutdown(&self) -> BoxFuture<'_, Result<()>>;
}

struct Registry {
    /// Primary index. (TypeId of R, ResourceId) → scoped runtimes.
    by_type: DashMap<(TypeId, ResourceId), SmallVec<[ScopedRuntime; 4]>>,
    /// Secondary index. ResourceKey → TypeId.
    by_key: DashMap<ResourceKey, TypeId>,
}

struct ScopedRuntime {
    scope:   ScopeLevel,
    managed: Arc<dyn AnyManagedResource>,
}

impl Registry {
    /// Typed fast path. downcast_ref — zero allocation.
    ///
    /// Исправлено (#18): async потому что scope_is_compatible → ScopeResolver::is_child_of async.
    pub async fn get_typed<R: Resource>(
        &self,
        id:       ResourceId,
        scope:    &ScopeLevel,
        resolver: &dyn ScopeResolver,
        mode:     ContainmentMode,
    ) -> Result<Arc<ManagedResource<R>>> {
        let erased = self.get_erased(TypeId::of::<R>(), id, scope, resolver, mode).await?;
        erased.as_any().downcast_ref::<ManagedResource<R>>()
            .cloned()
            .ok_or_else(|| Error::internal("type mismatch in registry"))
    }

    /// Erased path. For dynamic/string-based lookup (admin API, health dashboard).
    pub async fn get_erased(
        &self,
        type_id:  TypeId,
        id:       ResourceId,
        scope:    &ScopeLevel,
        resolver: &dyn ScopeResolver,
        mode:     ContainmentMode,
    ) -> Result<Arc<dyn AnyManagedResource>> {
        let entries = self.by_type.get(&(type_id, id))
            .ok_or_else(|| Error::not_found("resource"))?;

        // Find most specific scope that matches.
        // ScopeLevel from nebula-core provides hierarchy:
        // Action > Execution > Workflow > Project > Organization > Global.
        // NOTE: async filter — await each compatibility check.
        let mut best: Option<(u8, Arc<dyn AnyManagedResource>)> = None;
        for entry in entries.iter() {
            if scope_is_compatible(resolver, mode, &entry.scope, scope).await {
                let specificity = scope_specificity(&entry.scope);
                if best.as_ref().map_or(true, |(s, _)| specificity > *s) {
                    best = Some((specificity, Arc::clone(&entry.managed)));
                }
            }
        }
        best.map(|(_, m)| m).ok_or_else(|| Error::not_found("resource"))
    }
}

/// Scope specificity for registry lookup ordering.
/// Uses ScopeLevel from nebula-core.
fn scope_specificity(scope: &ScopeLevel) -> u8 {
    match scope {
        ScopeLevel::Global             => 0,
        ScopeLevel::Organization(_)    => 1,
        ScopeLevel::Project(_)         => 2,
        ScopeLevel::Workflow(_)        => 3,
        ScopeLevel::Execution(_)       => 4,
        ScopeLevel::Action(_, _)       => 5,  // (ExecutionId, NodeId)
    }
}

// ── Scope containment ──────────────────────────────────────────────────

/// Режим проверки scope containment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainmentMode {
    /// Strict: Organization X содержит ТОЛЬКО свои Projects.
    /// Требует ScopeResolver для проверки parent-child связей.
    /// DEFAULT — безопасен для multi-tenant.
    Strict,
    /// Simplified: Organization содержит ВСЕ Projects.
    /// Только для single-tenant или development.
    /// Требует явного opt-in через Manager::with_simplified_scoping().
    Simplified,
}

/// Резолвит parent-child scope relationships.
/// Implemented by platform (DB lookup org→project mapping).
///
/// Исправлено (#20): `impl Future` → `BoxFuture<'a, bool>` для object-safety.
/// `impl Future` в трейте делает его non-object-safe (нельзя `dyn ScopeResolver`).
/// `CachedScopeResolver` хранит `Arc<dyn ScopeResolver>` → нужен BoxFuture.
pub trait ScopeResolver: Send + Sync {
    fn is_child_of<'a>(
        &'a self,
        child:  &'a ScopeLevel,
        parent: &'a ScopeLevel,
    ) -> BoxFuture<'a, bool>;
}

/// Cached resolver — moka cache TTL=5min. Scope relationships меняются редко.
/// NOTE: moka dependency should be behind a feature gate:
///   [features]
///   scope-cache = ["dep:moka"]  # enabled by default
/// Without this feature, Manager::with_scope_resolver() wraps the resolver directly.
pub struct CachedScopeResolver {
    inner: Arc<dyn ScopeResolver>,
    cache: moka::future::Cache<(ScopeLevel, ScopeLevel), bool>,
}
impl CachedScopeResolver {
    pub fn new(inner: Arc<dyn ScopeResolver>, max_capacity: u64) -> Self {
        Self {
            inner,
            cache: moka::future::Cache::builder()
                .max_capacity(max_capacity)
                .time_to_live(Duration::from_secs(300))
                .build(),
        }
    }
}
impl ScopeResolver for CachedScopeResolver {
    fn is_child_of<'a>(&'a self, child: &'a ScopeLevel, parent: &'a ScopeLevel) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            let key = (child.clone(), parent.clone());
            self.cache.try_get_with(key, async move {
                Ok::<bool, Infallible>(self.inner.is_child_of(child, parent).await)
            }).await.unwrap_or(false)
        })
    }
}

/// Паникует если scope resolution без resolver в Strict mode.
/// Заставляет платформу явно выбрать стратегию при инициализации Manager.
struct RequiresScopeResolver;
impl ScopeResolver for RequiresScopeResolver {
    fn is_child_of<'a>(&'a self, _: &'a ScopeLevel, _: &'a ScopeLevel) -> BoxFuture<'a, bool> {
        Box::pin(async {
            panic!(
                "ScopeResolver not configured. \
                 For multi-tenant: Manager::with_scope_resolver(resolver). \
                 For single-tenant/dev: Manager::with_simplified_scoping()."
            )
        })
    }
}

/// Check if a registered scope is compatible with a request scope.
/// A more general scope serves more specific requests.
///
/// NOTE: эта функция теперь async из-за ScopeResolver (Strict mode).
/// Registry::get_erased() становится async.
async fn scope_is_compatible(
    resolver:  &dyn ScopeResolver,
    mode:      ContainmentMode,
    registered: &ScopeLevel,
    request:    &ScopeLevel,
) -> bool {
    match mode {
        ContainmentMode::Simplified => {
            // Как раньше: Organization содержит все Projects.
            request.is_contained_in(registered)
        }
        ContainmentMode::Strict => {
            match (registered, request) {
                // Global обслуживает всех.
                (ScopeLevel::Global, _) => true,
                // Тот же уровень → точное совпадение.
                _ if registered == request => true,
                // Request специфичнее registered → проверить parent-child.
                _ if scope_specificity(request) > scope_specificity(registered) => {
                    resolver.is_child_of(request, registered).await
                }
                // Request менее специфичен → несовместим.
                _ => false,
            }
        }
    }
}

// ── PressureSnapshot — lock-free memory pressure ──────────────────────

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureLevel { Normal = 0, Moderate = 1, High = 2, Critical = 3 }

pub struct PressureSnapshot {
    level: AtomicU8,
}
impl PressureSnapshot {
    pub fn load(&self) -> PressureLevel {
        match self.level.load(Ordering::Relaxed) {
            0 => PressureLevel::Normal,
            1 => PressureLevel::Moderate,
            2 => PressureLevel::High,
            _ => PressureLevel::Critical,
        }
    }
    fn store(&self, level: PressureLevel) {
        self.level.store(level as u8, Ordering::Relaxed);
    }
}
```

---

## Topology-aware default layers

Uses `nebula-resilience` LayerBuilder, not custom LayerStack.

```rust
use nebula_resilience::compose::LayerBuilder;

impl Manager {
    fn default_resilience_for(&self, topology: TopologyKind) -> Option<AcquireResilience> {
        match topology {
            // Acquire = real work (checkout, connect, open session).
            TopologyKind::Pool | TopologyKind::Exclusive | TopologyKind::Transport => {
                Some(AcquireResilience {
                    timeout: Some(Duration::from_secs(30)),
                    retry: Some(AcquireRetryConfig {
                        max_attempts: 3,
                        base_delay: Duration::from_millis(100),
                        max_delay: Duration::from_secs(5),
                        backoff: BackoffKind::Exponential,
                    }),
                    circuit_breaker: None, // opt-in per resource (use AcquireCircuitBreakerPreset)
                })
            }
            // Acquire = instant (clone, token). No resilience needed.
            TopologyKind::Resident | TopologyKind::Service | TopologyKind::EventSource => None,
            // No acquire path.
            TopologyKind::Daemon => None,
        }
    }
}
```

---

## Config hot-reload — per-topology strategy, two-phase with rollback

> **Amendment #9:** All Eager topologies use two-phase reload: create new → health check →
> atomic swap. If create or health check fails → `ReloadResult::RolledBack`, old runtime
> continues serving. No partial apply, no inconsistent state.

Per-topology reload strategies:

| Topology | Strategy | Hot-reload? | Safety |
|----------|----------|-------------|--------|
| Pool | **Lazy eviction** — update fingerprint, stale instances evicted at next recycle | ✅ Yes | Safe — no state change during create |
| Resident | **Two-phase** — create new → health check → ArcSwap → destroy old | ✅ Yes | Rollback if create/check fails |
| Service | **Two-phase + natural drain** — create new → health check → swap → old drains via Arc | ✅ Yes | Rollback + drain deadline watchdog (amendment #10) |
| Transport | **Two-phase** — create new → health check → swap → destroy old | ✅ Yes | Rollback |
| Exclusive | **Two-phase** — create new → health check → swap (wait permit) | ✅ Yes | Rollback |
| EventSource | **Two-phase** — resubscribe → verify → swap | ✅ Yes | Rollback |
| Daemon | **Restart** — cancel → restart with new config | ⚠️ Restart | Brief unavailability during restart |

**Key invariant**: для всех Eager topologies — создать новый runtime ПЕРЕД уничтожением
старого. Если create или health check fails → `ReloadResult::RolledBack`, старый runtime
продолжает работать.

```rust
/// Результат hot-reload.
pub enum ReloadResult {
    Applied,
    /// Применить не удалось. Старый конфиг активен. reason — для логов.
    RolledBack { reason: String },
    Skipped    { reason: String },
}
```

Two-phase reload в `TopologyRuntime::on_config_changed()`:

```rust
// Resident — two-phase:
TopologyRuntime::Resident(rt) => {
    // Phase 1: create new (старый runtime untouched).
    let new_runtime = match resource.create(&new_config, &credential, ctx).await {
        Ok(r) => r,
        Err(e) => return Ok(ReloadResult::RolledBack {
            reason: format!("create failed: {}", e)
        }),
    };
    // Phase 2: health check нового runtime перед swap.
    if let Err(e) = resource.check(&new_runtime).await {
        resource.destroy(new_runtime).await.ok();
        return Ok(ReloadResult::RolledBack {
            reason: format!("health check failed: {}", e)
        });
    }
    // Phase 3: atomic swap. Старый Arc дропнется когда refcount → 0.
    let old_arc = rt.cell.swap(new_runtime);
    rt.set_config(new_config);
    // Phase 4: destroy old.
    if let Some(old) = Arc::try_unwrap(old_arc).ok() {
        resource.shutdown(&old).await.ok();
        resource.destroy(old).await.ok();
    }
    Ok(ReloadResult::Applied)
}

// Service — two-phase + natural drain + drain watchdog:
TopologyRuntime::Service(rt) => {
    let new_runtime = match resource.create(&new_config, &credential, ctx).await {
        Ok(r) => r,
        Err(e) => return Ok(ReloadResult::RolledBack { reason: e.to_string() }),
    };
    if let Err(e) = resource.check(&new_runtime).await {
        resource.destroy(new_runtime).await.ok();
        return Ok(ReloadResult::RolledBack { reason: e.to_string() });
    }
    // Swap + spawn drain watchdog (см. секцию ниже).
    rt.swap_runtime_with_watchdog(new_runtime, new_config);
    Ok(ReloadResult::Applied)
}

// Pool — lazy (безопасен по конструкции):
TopologyRuntime::Pool(rt) => {
    rt.update_config_fingerprint(&new_config);
    rt.set_config(new_config);
    Ok(ReloadResult::Applied)
}

// Daemon — cancel + restart:
TopologyRuntime::Daemon(rt) => {
    rt.cancel_and_restart(new_config).await;
    Ok(ReloadResult::Applied)
}
```

`ManagerConfigAdapter::configure()` обрабатывает `ReloadResult::RolledBack`:
```rust
match result {
    ReloadResult::Applied => { /* emit ConfigReloaded event */ }
    ReloadResult::RolledBack { reason } => {
        tracing::warn!(%resource_id, %reason, "config reload rolled back, old config active");
        // emit ConfigReloadRejected event — НЕ возвращает Err.
        // Caller (nebula-config) не должен паниковать из-за rollback.
    }
    ReloadResult::Skipped { .. } => {}
}
```

### Integration with `nebula-config`

`nebula-config` provides `AsyncConfigurable` trait:
```rust
#[async_trait]
pub trait AsyncConfigurable: Send + Sync {
    type Config: Validatable;
    async fn configure(&mut self, config: Self::Config) -> Result<(), ConfigError>;
    fn configuration(&self) -> &Self::Config;
}
```

**Problem:** `AsyncConfigurable::configure()` takes `&mut self`, but `Manager` is shared
(`Arc<Manager>`). Two options:

1. **Wrapper struct** — `ManagerConfigAdapter` holds `Arc<Manager>`, uses internal
   mutability (Manager already uses DashMap, atomics, etc. — no external &mut needed).
2. **Direct method** — Manager exposes `reload_resource_config()` directly, called by
   the config system through a thin adapter.

**Chosen: option 1** — thin adapter implements AsyncConfigurable:

```rust
use nebula_config::{AsyncConfigurable, ConfigError, Validatable};

/// Wrapper for config hot-reload. Manager itself uses interior mutability.
pub struct ManagerConfigAdapter {
    manager: Arc<Manager>,
}

/// Config type for resource section. Implements Validatable.
#[derive(Clone)]
pub struct ResourceConfigSection {
    resources: HashMap<String, serde_json::Value>,
}

impl Validatable for ResourceConfigSection {
    fn validate(&self) -> Result<(), ConfigError> { Ok(()) }
    fn default_config() -> Self { Self { resources: HashMap::new() } }
}

#[async_trait]
impl AsyncConfigurable for ManagerConfigAdapter {
    type Config = ResourceConfigSection;

    async fn configure(&mut self, config: Self::Config) -> Result<(), ConfigError> {
        for (key, value) in &config.resources {
            if let Some(resource_id) = self.manager.registry.id_by_config_key(key) {
                let old_fp = self.manager.registry.config_fingerprint(resource_id);
                let new_fp = compute_fingerprint(value);

                if old_fp != new_fp {
                    tracing::info!(%resource_id, "config changed, reloading");
                    self.manager.reload_resource_config(resource_id, value.clone()).await
                        .map_err(|e| ConfigError::ApplyFailed(e.to_string()))?;

                    let _ = self.manager.resource_bus.send(ResourceEvent::ConfigReloaded {
                        resource_id,
                        resource_key: self.manager.registry.key_for(resource_id),
                        old_fingerprint: old_fp,
                        new_fingerprint: new_fp,
                    });
                }
            }
        }
        Ok(())
    }

    fn configuration(&self) -> &Self::Config {
        // Returns cached last-applied config.
        // Implementation detail — omitted for brevity.
        todo!()
    }
}
```

---

## Service drain watchdog — deadline для старого runtime

При config reload Service topology: создаётся новый runtime, старый Arc дропается когда
все caller-ы отпустили хэндлы (natural drain через Arc refcount). Без watchdog leaked
handle = zombie runtime работает вечно.

```rust
// В service::Runtime:
pub fn swap_runtime_with_watchdog(&self, new_runtime: R::Runtime, config: R::Config) {
    let old_arc = self.current.swap(Arc::new(ServiceInner { runtime: new_runtime, config }));

    // Downgrade: natural drain через Arc refcount.
    let weak         = Arc::downgrade(&old_arc);
    let resource_key = R::KEY;
    let deadline     = self.config.drain_deadline; // default: 5 minutes
    drop(old_arc); // release наша strong ref

    tokio::spawn(async move {
        let start = Instant::now();
        loop {
            if weak.strong_count() == 0 {
                tracing::info!(%resource_key, "old runtime drained naturally");
                return;
            }
            if start.elapsed() > deadline {
                let remaining = weak.strong_count();
                tracing::warn!(
                    %resource_key,
                    remaining_refs = remaining,
                    ?deadline,
                    "old runtime drain deadline exceeded — possible handle leak"
                );
                // Arc holders не могут быть принудительно dropped.
                // Логируем + metrics. Old runtime дропнется когда holders завершатся.
                // V2: ServiceInner хранит CancellationToken →
                //     watchdog вызывает cancel → acquire_token() возвращает Err.
                return;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

// service::Config:
pub struct service::Config {
    // ...
    /// Timeout до warn о leaked old runtime при config reload. Default: 5 minutes.
    pub drain_deadline: Duration,
}
```

---

## Audit trail — structured logging для resource access

> **Amendment #12:** Added structured audit logging for compliance-sensitive resources.

Автоматический audit log каждого acquire/release. Action author не делает ничего.

```rust
// В ManagedResource::acquire() — после успешного ResourceHandle:
tracing::info!(
    target       = "nebula_resource::audit",
    resource_key = %R::KEY,
    resource_id  = %self.resource_id,
    execution_id = %ctx.execution_id(),
    scope        = ?ctx.scope(),
    topology     = self.topology_tag(),
    intent       = ?options.intent,
    "resource.acquire"
);

// В ReleaseQueue worker — перед recycle/destroy:
tracing::info!(
    target       = "nebula_resource::audit",
    resource_key = %resource_key,
    resource_id  = %resource_id,
    execution_id = %execution_id,
    hold_ms      = hold_duration.as_millis(),
    tainted      = is_tainted,
    "resource.release"
);
```

Включение/выключение audit отдельно от основных логов:

```
# Только audit:
RUST_LOG=nebula_resource::audit=info,nebula_resource=warn

# Без audit:
RUST_LOG=nebula_resource=debug
```

**Обязательные structured fields:**

| Field | Тип | Где |
|-------|-----|-----|
| `resource_key` | &'static str | acquire + release |
| `resource_id` | ResourceId | acquire + release |
| `execution_id` | ExecutionId | acquire + release |
| `scope` | ScopeLevel | acquire |
| `intent` | AcquireIntent | acquire |
| `topology` | &'static str | acquire |
| `hold_ms` | u128 | release |
| `tainted` | bool | release |

---

## ShutdownOrchestrator

Graceful shutdown в обратном topological порядке. Three phases with configurable timeouts.

**Dependency ordering:** if resource A depends on B, A shuts down BEFORE B.
This ensures dependents finish before their dependencies are destroyed.

```rust
pub struct ShutdownOrchestrator;

impl ShutdownOrchestrator {
    /// Phased graceful shutdown respecting dependency order.
    ///
    /// Phase 1 — DRAIN: Cancel global CancellationToken → stops Daemon tasks,
    ///   maintenance loops, new acquire() calls rejected. Wait for in-flight
    ///   guards to be released (up to drain_timeout).
    ///
    /// Phase 2 — CLEANUP: Build dependency graph → reverse topological sort.
    ///   Shut down each resource in dependency-safe order:
    ///   resource.shutdown() → drain ReleaseQueue → destroy all instances.
    ///   Per-resource timeout = cleanup_timeout.
    ///
    /// Phase 3 — TERMINATE: Force-close any resources that didn't complete
    ///   cleanup within terminate_timeout. Log leaked handles.
    ///
    /// NOTE (amendment #10): Service topology drain uses Arc refcount.
    /// Leaked handles prevent old runtime destruction — watchdog logs warnings
    /// but cannot force-drop Arc holders.
    pub async fn shutdown(manager: &Manager, config: ShutdownConfig) -> Result<()> {
        // Phase 1: Signal all resources to stop accepting new work.
        manager.cancel.cancel();

        // Wait for in-flight to drain.
        tokio::time::sleep(config.drain_timeout.min(Duration::from_secs(5))).await;

        // Phase 2: Dependency-ordered cleanup.
        // v1: reverse registration order (no dependency graph yet).
        // v2: proper topological sort from registered resource dependencies.
        let order = manager.registry.reverse_topological_order();

        for resource_id in order {
            let result = tokio::time::timeout(
                config.cleanup_timeout,
                manager.remove(resource_id),
            ).await;

            match result {
                Ok(Ok(())) => tracing::info!(%resource_id, "resource shut down"),
                Ok(Err(e)) => tracing::warn!(%resource_id, "shutdown error: {}", e),
                Err(_)     => tracing::warn!(%resource_id, "shutdown timeout, forcing"),
            }
        }

        Ok(())
    }
}
```
