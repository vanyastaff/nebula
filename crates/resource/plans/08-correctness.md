# 08 — Correctness Amendments

25 архитектурных проблем, выявленных при ревью планов 01–07.
Часть I (#1–#13): первый раунд ревью. Часть II (#14–#25): второй раунд.
Каждый раздел: **проблема → решение → что меняется** (ссылки на план-файл).

Статусы: ✅ integrated into plan file | 🔧 pending integration

---

## #1 — ReleaseQueue: тихая потеря данных (03-infrastructure.md) ✅

**Проблема.** `submit()` вызывает `try_send()`, который молча дропает task если bounded-канал полон.
Для Pool это означает что instance никогда не вернётся в idle и никогда не будет destroyed —
утечка соединений. При нагрузке pool деградирует: `max_size` занят "потерянными" инстансами,
`acquire` блокируется, zombie-соединения на стороне сервера.

**Решение.** Двухуровневая стратегия: primary bounded + fallback unbounded + метрики.

```rust
pub struct ReleaseQueue {
    tx:          mpsc::Sender<ReleaseTask>,
    fallback_tx: mpsc::UnboundedSender<ReleaseTask>,
    metrics:     Arc<ReleaseQueueMetrics>,
    workers:     Vec<JoinHandle<()>>,
}

#[derive(Default)]
pub struct ReleaseQueueMetrics {
    pub submitted:     AtomicU64,
    pub fallback_used: AtomicU64,
    /// Must be 0 in healthy system. Алерт если растёт.
    pub dropped:       AtomicU64,
}

impl ReleaseQueue {
    pub fn submit(&self, task: ReleaseTask) {
        self.metrics.submitted.fetch_add(1, Ordering::Relaxed);
        match self.tx.try_send(task) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(task)) => {
                // Primary полон — burst нагрузка. Unbounded fallback.
                // Если часто — capacity слишком мал.
                self.metrics.fallback_used.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    "ReleaseQueue primary full, using fallback. \
                     Consider increasing capacity."
                );
                if self.fallback_tx.send(task).is_err() {
                    // Fallback закрыт = shutdown in progress.
                    self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                    tracing::error!(
                        "ReleaseQueue fallback closed during shutdown, task dropped"
                    );
                }
            }
            Err(mpsc::error::TrySendError::Closed(task)) => {
                self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                let _ = self.fallback_tx.send(task); // last-ditch
            }
        }
    }
}

/// Worker: primary имеет приоритет (bounded = нормальный flow).
/// Fallback = overflow, тоже должен обрабатываться.
async fn worker_loop(
    primary:  Arc<AsyncMutex<mpsc::Receiver<ReleaseTask>>>,
    fallback: Arc<AsyncMutex<mpsc::UnboundedReceiver<ReleaseTask>>>,
    cancel:   CancellationToken,
) {
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                // Drain remaining on shutdown.
                let mut pg = primary.lock().await;
                let mut fb = fallback.lock().await;
                while let Ok(t) = pg.try_recv() { (t.release_fn)().await; }
                while let Ok(t) = fb.try_recv() { (t.release_fn)().await; }
                break;
            }
            task = async { primary.lock().await.recv().await } => {
                match task { Some(t) => (t.release_fn)().await, None => break }
            }
            task = async { fallback.lock().await.recv().await } => {
                match task { Some(t) => (t.release_fn)().await, None => {} }
            }
        }
    }
}
```

`ReleaseQueueHandle::submit()` — та же двухуровневая логика (держит оба sender-а).

---

## #2 — HandleInner::Guarded: panic в Deref после detach (03-infrastructure.md) ✅

**Проблема.** `value.as_ref().expect("value consumed")` в `Deref` паникует если кто-то
вызвал `detach()` и потом сделал deref через ещё живую ссылку. В многопоточном коде с
`Arc<Mutex<ResourceHandle>>` это реальная race.

**Решение.** Два изменения:

1. `detach()` возвращает `Result<R::Lease, DetachError>` (не `Option`), ясно сигнализируя
   о невозможности detach.
2. `Deref` использует `unreachable!` вместо `expect` — по конструкции `value` всегда
   `Some` пока `ResourceHandle` существует (detach потребляет self + вызывает
   `std::mem::forget`).

```rust
impl<R: Resource> ResourceHandle<R> {
    /// Detach от pool. Потребляет self — после detach нет ResourceHandle для deref.
    /// Disarms on_release callback. Pool не ждёт возврата instance.
    pub fn detach(mut self) -> Result<R::Lease, DetachError> {
        match &mut self.inner {
            HandleInner::Guarded { value, on_release, .. } => {
                on_release.take(); // disarm
                let lease = value.take().ok_or(DetachError::AlreadyConsumed)?;
                // forget(self) → Drop не вызывается → on_release не fire-ится повторно.
                std::mem::forget(self);
                Ok(lease)
            }
            HandleInner::Owned(_)   => Err(DetachError::NotDetachable),
            HandleInner::Shared { .. } => Err(DetachError::NotDetachable),
        }
    }
}

impl<R: Resource> Deref for ResourceHandle<R> {
    type Target = R::Lease;
    fn deref(&self) -> &R::Lease {
        match &self.inner {
            HandleInner::Owned(v)              => v,
            HandleInner::Guarded { value, .. } => {
                // SAFETY: value всегда Some пока ResourceHandle жив.
                // detach() потребляет self + forget → невозможно иметь &ResourceHandle
                // после detach. None здесь = framework bug, не user error.
                value.as_ref().unwrap_or_else(|| {
                    unreachable!(
                        "ResourceHandle::deref on consumed value. Framework bug."
                    )
                })
            }
            HandleInner::Shared { value, .. } => value.as_ref(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DetachError {
    #[error("lease already consumed (framework bug)")]
    AlreadyConsumed,
    #[error("this handle type does not support detach (Owned or Shared)")]
    NotDetachable,
}
```

---

## #3 — Credential design gap: todo!() по всей кодовой базе (01-core.md) ✅

**Проблема.** `create()` и `prepare()` содержат `todo!("credential integration")`.
Архитектура уже ограничивает будущий дизайн: `Ctx` trait не имеет credential метода.
При реализации придётся либо ломать `Ctx` API, либо делать небезопасный downcast.

**Решение.** `CredentialCtx` — отдельный extension trait. Resource декларирует
`type Credential`. Framework резолвит перед `create()`. Backward compatible.

```rust
// ── Credential ──

/// Маркер: credential такого типа.
pub trait Credential: Send + Sync + Clone + 'static {
    /// Уникальный ключ типа. E.g. "database", "api_token", "ssh_key".
    const KIND: &'static str;
}

/// Нет credentials (HTTP client, статичный ресурс).
impl Credential for () { const KIND: &'static str = "none"; }

/// Credential store — резолвит credentials runtime.
/// Implemented: vault, env vars, k8s secrets, nebula-credential.
pub trait CredentialStore: Send + Sync {
    fn resolve<C: Credential>(
        &self,
        scope: &ScopeLevel,
    ) -> impl Future<Output = Result<C, CredentialError>> + Send;
}

/// Extension trait. Добавляет credential access к Ctx.
/// Отдельный trait = backward compatible (не ломает существующие Ctx impls).
pub trait CredentialCtx: Ctx {
    fn credential_store(&self) -> &dyn CredentialStore;
}

// ── Resource обновлённый trait ──

pub trait Resource: Send + Sync + 'static {
    // ... существующие ассоциированные типы ...

    /// Credential тип. `()` для ресурсов без secrets.
    /// Framework резолвит через CredentialStore перед create().
    /// NOTE: associated type defaults не стабильны — каждый impl указывает явно.
    type Credential: Credential;

    /// create() принимает уже резолвленный credential.
    /// НЕ вызывает credential store сам — это ответственность framework.
    fn create(
        &self,
        config:     &Self::Config,
        credential: &Self::Credential,
        ctx:        &dyn Ctx,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;
}

// ── Framework resolution в ManagedResource::create_instance() ──

async fn create_instance<R: Resource>(
    resource: &R,
    config:   &R::Config,
    ctx:      &dyn CredentialCtx,
) -> Result<R::Runtime, Error> {
    let credential = ctx.credential_store()
        .resolve::<R::Credential>(ctx.scope())
        .await
        .map_err(|e| Error::permanent(e))?;

    resource.create(config, &credential, ctx)
        .await
        .map_err(Into::into)
}

// ── Пример: Postgres ──

pub struct DatabaseCredential {
    pub host:     String,
    pub port:     u16,
    pub database: String,
    pub username: String,
    pub password: SecretString,
    pub ssl_mode: SslMode,
}
impl Credential for DatabaseCredential { const KIND: &'static str = "database"; }

impl Resource for Postgres {
    type Credential = DatabaseCredential;

    async fn create(
        &self,
        config: &PgResourceConfig,
        cred:   &DatabaseCredential,    // ← резолвлен framework-ом
        _ctx:   &dyn Ctx,
    ) -> Result<PgConnection, PgError> {
        let (client, connection) = tokio_postgres::Config::new()
            .host(&cred.host)
            .port(cred.port)
            .dbname(&cred.database)
            .user(&cred.username)
            .password(cred.password.expose())
            .connect_timeout(config.connect_timeout)
            .connect(NoTls).await.map_err(PgError::Connect)?;
        // ...
    }
}

// ── HTTP Client — без credentials ──
impl Resource for HttpClient {
    type Credential = ();  // нет secrets
    async fn create(&self, config: &HttpConfig, _cred: &(), _ctx: &dyn Ctx)
        -> Result<reqwest::Client, HttpError>
    {
        reqwest::Client::builder().timeout(config.timeout).build().map_err(HttpError::Build)
    }
}
```

**Credential rotation.** При `CredentialRotatedEvent` (из `nebula-eventbus`):
- Pool: sets stale fingerprint → instances evicted at next recycle + recreate with new cred.
- Resident/Service/Daemon: `destroy + create` с новым credential (через CredentialStore).
- Resource author ничего не делает — это framework responsibility.

---

## #4 — Scope lookup: simplified containment без strict validation (05-manager.md) ✅

**Проблема.** `scope_is_compatible` использует "simplified containment": Organization A
может обслуживать Project из Organization B. Cross-tenant утечка ресурсов. В multi-tenant
системе это критическая security-проблема.

**Решение.** Strict containment по умолчанию. `ScopeResolver` trait для проверки
parent-child. Simplified — только явный opt-in.

```rust
/// Режим проверки scope containment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainmentMode {
    /// Strict: Organization X содержит ТОЛЬКО свои Projects.
    /// Требует ScopeResolver для проверки parent-child.
    /// DEFAULT — безопасен для multi-tenant.
    Strict,

    /// Simplified: Organization содержит ВСЕ Projects.
    /// Только для single-tenant или development.
    /// Требует явного opt-in (Manager::with_simplified_scoping).
    Simplified,
}

/// Резолвит parent-child scope relationships.
/// Implemented by platform (DB lookup org→project mapping).
pub trait ScopeResolver: Send + Sync {
    fn is_child_of(
        &self,
        child:  &ScopeLevel,
        parent: &ScopeLevel,
    ) -> impl Future<Output = bool> + Send;
}

/// Cached resolver — избегает DB round-trip на каждый acquire.
/// moka cache с TTL=5min. Scope relationships меняются редко.
pub struct CachedScopeResolver {
    inner: Arc<dyn ScopeResolver>,
    cache: moka::future::Cache<(ScopeLevel, ScopeLevel), bool>,
}

/// Паникует если scope resolution без resolver в Strict mode.
/// Заставляет платформу явно выбрать стратегию.
struct RequiresScopeResolver;
impl ScopeResolver for RequiresScopeResolver {
    async fn is_child_of(&self, _: &ScopeLevel, _: &ScopeLevel) -> bool {
        panic!(
            "ScopeResolver not configured. \
             For multi-tenant: Manager::with_scope_resolver(). \
             For single-tenant: Manager::with_simplified_scoping()."
        );
    }
}

impl Manager {
    pub fn new(telemetry: Arc<dyn TelemetryService>) -> Self {
        Self {
            // Strict по умолчанию + PanicResolver → заставляет настроить.
            containment_mode: ContainmentMode::Strict,
            scope_resolver: Arc::new(RequiresScopeResolver),
            // ...
        }
    }

    /// Single-tenant или development.
    pub fn with_simplified_scoping(mut self) -> Self {
        self.containment_mode = ContainmentMode::Simplified;
        self
    }

    /// Multi-tenant production.
    pub fn with_scope_resolver(mut self, resolver: Arc<dyn ScopeResolver>) -> Self {
        self.scope_resolver = Arc::new(CachedScopeResolver::new(resolver, 10_000));
        self
    }
}

// Обновлённый scope_is_compatible в Registry::get_erased():
async fn scope_is_compatible(
    &self,
    registered: &ScopeLevel,
    request:    &ScopeLevel,
) -> bool {
    match self.containment_mode {
        ContainmentMode::Simplified => request.is_contained_in(registered),
        ContainmentMode::Strict => {
            match (registered, request) {
                // Global обслуживает всех.
                (ScopeLevel::Global, _) => true,
                // Тот же уровень → точное совпадение.
                _ if registered == request => true,
                // Request специфичнее registered → проверить parent-child.
                _ if scope_specificity(request) > scope_specificity(registered) => {
                    self.scope_resolver.is_child_of(request, registered).await
                }
                // Request менее специфичен → несовместим.
                _ => false,
            }
        }
    }
}
```

**NOTE:** `Registry::get_erased()` становится `async` из-за scope_resolver.
Typed fast path `get_typed()` через него — тоже async. Manager::acquire остаётся async.

---

## #5 — PoisonToken: non-atomic coordination (03-infrastructure.md) ✅

**Проблема.** `LeaseGuard::drop()`:
```rust
let tainted = self.tainted || self.poison.load(Ordering::Acquire);
if let (Some(lease), Some(release_fn)) = (self.lease.take(), self.on_release.take()) {
    release_fn(lease, tainted);
}
```
Между `poison.load()` и `release_fn()` — зазор. Poison в другом потоке после load
но до release_fn → instance возвращается в pool как здоровый хотя отравлен.

**Решение.** Передать `Arc<AtomicBool>` в release_fn. Финальная проверка — в
release pipeline (ReleaseQueue worker), непосредственно перед решением recycle vs destroy.

```rust
// Обновлённая сигнатура on_release:
type ReleaseFn<L> = Box<dyn FnOnce(L, bool, Arc<AtomicBool>) + Send>;
//                                          ^tainted_at_drop  ^poison_flag

impl<L: Send + 'static> Drop for LeaseGuard<L> {
    fn drop(&mut self) {
        let tainted = self.tainted; // snapshot at drop time
        if let (Some(lease), Some(release_fn)) = (self.lease.take(), self.on_release.take()) {
            // poison_flag передаётся BY REFERENCE в release pipeline.
            // Финальная проверка: tainted_at_drop || poison.load() — в worker.
            release_fn(lease, tainted, Arc::clone(&self.poison));
        }
    }
}

// В ReleaseQueue worker (pool release pipeline):
async fn process_release<R: Resource + Pooled>(
    resource:          &R,
    lease:             R::Runtime,
    tainted_at_drop:   bool,
    poison:            Arc<AtomicBool>,
    // ...
) {
    // Final taint check AT THE DECISION POINT. Закрывает race window.
    let is_tainted = tainted_at_drop || poison.load(Ordering::Acquire);
    if is_tainted {
        resource.destroy(lease).await.ok();
        return;
    }
    // ... is_broken check → recycle или destroy
}
```

Это НЕ полностью устраняет race (между последним `poison.load()` в worker и фактическим
recycle — всё ещё есть наносекундный зазор), но сужает window до минимума:
window = только atomic load + одна ветка, против прежнего window = весь путь через
ReleaseQueue.

---

## #6 — RecoveryGate: try_begin = todo!() (04-recovery-resilience.md) ✅

**Проблема.** `try_begin()` — самый критический concurrency primitive в системе —
не реализован.

**Решение.** CAS loop на `ArcSwap`. Нужна поддержка `compare_and_swap` в ArcSwap.

```rust
impl RecoveryGate {
    pub fn try_begin(&self) -> Result<RecoveryTicket, RecoveryWaiter> {
        loop {
            let current = self.state.load();

            match current.as_ref() {
                GateState::Idle => {
                    let next = Arc::new(GateState::InProgress {
                        attempt:  1,
                        started: Instant::now(),
                    });
                    // CAS: succeed only if state still == current.
                    let prev = self.state.compare_and_swap(&current, next);
                    if Arc::ptr_eq(&prev, &current) {
                        // Мы выиграли race.
                        return Ok(RecoveryTicket { attempt: 1, _private: () });
                    }
                    // Проиграли — retry.
                    continue;
                }

                GateState::InProgress { .. } => {
                    return Err(self.make_waiter());
                }

                GateState::Failed { until, attempt, .. } => {
                    if Instant::now() < *until {
                        // Backoff не истёк.
                        return Err(self.make_waiter());
                    }
                    // Backoff истёк — попробовать начать новый recovery.
                    let new_attempt = attempt + 1;
                    if new_attempt > self.max_recovery_attempts {
                        // Escalate: → PermanentlyFailed.
                        let error = match current.as_ref() {
                            GateState::Failed { error, .. } => Arc::clone(error),
                            _ => unreachable!(),
                        };
                        let next = Arc::new(GateState::PermanentlyFailed { error });
                        self.state.store(next);
                        self.notify.notify_waiters();
                        return Err(self.make_waiter());
                    }
                    let next = Arc::new(GateState::InProgress {
                        attempt: new_attempt,
                        started: Instant::now(),
                    });
                    let prev = self.state.compare_and_swap(&current, next);
                    if Arc::ptr_eq(&prev, &current) {
                        return Ok(RecoveryTicket { attempt: new_attempt, _private: () });
                    }
                    continue;
                }

                GateState::PermanentlyFailed { .. } => {
                    return Err(self.make_waiter());
                }
            }
        }
    }

    fn make_waiter(&self) -> RecoveryWaiter {
        RecoveryWaiter { state: &self.state, notify: &self.notify }
    }
}

/// Обновлённый GateState — добавляем attempt в Failed для escalation tracking.
enum GateState {
    Idle,
    InProgress { attempt: u32, started: Instant },
    Failed {
        error:   Arc<crate::Error>,
        until:   Instant,
        attempt: u32,  // ← для подсчёта max_recovery_attempts
    },
    PermanentlyFailed { error: Arc<crate::Error> },
}

/// max_recovery_attempts на RecoveryGate. Default: 10.
pub struct RecoveryGate {
    state:                  ArcSwap<GateState>,
    notify:                 Notify,
    max_recovery_attempts:  u32,
}

/// RecoveryWaiter — future, resolves когда состояние меняется.
pub struct RecoveryWaiter<'a> {
    state:  &'a ArcSwap<GateState>,
    notify: &'a Notify,
}

impl RecoveryWaiter<'_> {
    /// Ждать до recovery или permanent failure.
    /// Ok(()) = recovered, Err = permanently failed или backoff истёк.
    pub async fn wait(&self) -> Result<(), Arc<crate::Error>> {
        loop {
            match self.state.load().as_ref() {
                GateState::Idle => return Ok(()),
                GateState::PermanentlyFailed { error } => return Err(Arc::clone(error)),
                GateState::Failed { until, error, .. } => {
                    if Instant::now() >= *until {
                        return Err(Arc::clone(error)); // backoff expired, caller retries
                    }
                    tokio::select! {
                        _ = self.notify.notified() => continue,
                        _ = tokio::time::sleep_until((*until).into()) => continue,
                    }
                }
                GateState::InProgress { .. } => {
                    self.notify.notified().await;
                }
            }
        }
    }
}
```

---

## #7 — Pool acquire: prepare() retry с тем же instance (07-implementation.md / pool/acquire.rs) 🔧

**Проблема.** При retryable `prepare()` error тот же instance возвращается в pool и тут же
выдаётся обратно тому же caller-у — 3 итерации одного и того же connection с одной и той
же ошибкой. Нет механизма "пропустить этот instance".

**Решение.** Blacklist per-acquire-cycle. Instance остаётся в pool для других caller-ов,
но не выдаётся повторно в текущем цикле.

```rust
// В pool::acquire():
pub async fn acquire(...) -> Result<ResourceHandle<R>, Error> {
    let max_attempts = pool.config.max_acquire_attempts; // default: 3

    // Instances с retryable prepare() error в текущем цикле.
    // SmallVec: heap alloc только если > 4 blacklisted.
    let mut blacklisted: SmallVec<[InstanceId; 4]> = SmallVec::new();

    for attempt in 1..=max_attempts {
        // 1. Checkout — пропускать blacklisted instances.
        let instance = idle_queue
            .checkout_excluding(&blacklisted)  // ← новый метод
            .or_else(|| try_create(resource, config, ctx).await?);

        let instance = match instance {
            Some(inst) => inst,
            None => wait_with_deadline(options.deadline).await?,
        };

        // 2. is_broken + test_on_checkout (без изменений).

        // 3. prepare().
        match resource.prepare(&instance.runtime, ctx).await {
            Ok(()) => return Ok(wrap_as_handle(instance)),
            Err(e) => {
                let error: crate::Error = e.into();
                if error.is_retryable() {
                    match resource.is_broken(&instance.runtime) {
                        BrokenCheck::Broken(_) => {
                            // Сломан → destroy, следующий attempt возьмёт другой instance.
                            destroy_instance(instance).await;
                        }
                        _ => {
                            // Instance здоров, prepare не прошёл (timeout, transient).
                            // Blacklist → другой caller получит этот instance.
                            blacklisted.push(instance.id);
                            return_to_idle(instance);
                        }
                    }
                    if attempt < max_attempts { continue; }
                }
                return Err(error);
            }
        }
    }

    Err(Error::exhausted(AcquireExhausted { attempts: max_attempts }, Some(Duration::from_secs(1))))
}

// Новый метод IdleQueue:
impl<R: Resource> IdleQueue<R> {
    /// Checkout, пропуская instances с ID в exclude.
    /// Временно peeked entries возвращаются обратно в очередь.
    pub fn checkout_excluding(&self, exclude: &[InstanceId]) -> Option<PoolEntry<R>> {
        let mut skipped = SmallVec::<[PoolEntry<R>; 4]>::new();
        while let Some(entry) = self.pop() {
            if !exclude.contains(&entry.id) {
                // Вернуть skipped instances (в обратном порядке для LIFO consistency).
                for s in skipped.into_iter().rev() { self.push_front(s); }
                return Some(entry);
            }
            skipped.push(entry);
        }
        // Все available были blacklisted — вернуть всех обратно.
        for s in skipped.into_iter().rev() { self.push_front(s); }
        None
    }
}
```

---

## #8 — Daemon: бесконечный recreate цикл (02-topology.md) ✅

**Проблема.** После `max_restarts` → `destroy + create` + `consecutive_failures = 0`.
Если `create()` успешно но `run()` всегда падает — бесконечный recreate навсегда.

**Решение.** `RecreateBudget` в `daemon::Config`. Global limit + windowed rate limit.

```rust
pub struct daemon::Config {
    pub restart_policy: RestartPolicy,
    pub recreate_budget: RecreateBudget,
    // ...
}

/// Лимиты на полное пересоздание daemon runtime.
pub struct RecreateBudget {
    /// Максимум recreates за весь lifetime daemon.
    /// After this → permanent failure.
    /// Default: 3.
    pub max_total: u32,

    /// Максимум recreates в window. Предотвращает rapid create-crash-recreate.
    /// Default: 2 recreates за 10 minutes.
    pub max_per_window: u32,
    pub window:         Duration,

    /// Backoff между recreates (отдельно от run() restart backoff).
    /// Default: 30s initial, 5min max, ×2.
    pub recreate_backoff: BackoffConfig,
}

impl Default for RecreateBudget {
    fn default() -> Self {
        Self {
            max_total:       3,
            max_per_window:  2,
            window:          Duration::from_secs(600),
            recreate_backoff: BackoffConfig {
                initial:    Duration::from_secs(30),
                max:        Duration::from_secs(300),
                multiplier: 2.0,
            },
        }
    }
}

// Состояние в daemon::Runtime:
struct DaemonState {
    consecutive_run_failures: u32,
    total_recreates:          u32,
    window_recreates:         u32,
    window_started:           Option<Instant>,
}

// В restart loop — перед destroy+create:
async fn try_recreate(&mut self, runtime: &mut R::Runtime) -> Result<(), Error> {
    let budget = &self.config.recreate_budget;

    // Global limit.
    if self.state.total_recreates >= budget.max_total {
        return Err(Error::permanent(DaemonExhausted {
            reason: format!("exceeded max total recreates ({})", budget.max_total),
        }));
    }

    // Windowed limit.
    let now = Instant::now();
    match self.state.window_started {
        Some(ws) if now.duration_since(ws) < budget.window => {
            if self.state.window_recreates >= budget.max_per_window {
                return Err(Error::permanent(DaemonExhausted {
                    reason: format!(
                        "exceeded recreate rate ({}/{:?})",
                        budget.max_per_window, budget.window
                    ),
                }));
            }
        }
        _ => {
            // Window expired или первый раз — сброс.
            self.state.window_started = Some(now);
            self.state.window_recreates = 0;
        }
    }

    // Recreate backoff.
    let delay = budget.recreate_backoff.delay_for(self.state.total_recreates + 1);
    tracing::warn!(
        total_recreates = self.state.total_recreates + 1,
        max = budget.max_total,
        ?delay,
        "daemon recreating runtime"
    );
    tokio::time::sleep(delay).await;

    // Destroy old + create new.
    let old = std::mem::replace(runtime, /* placeholder */ unsafe { std::mem::zeroed() });
    self.resource.shutdown(&old).await.ok();
    self.resource.destroy(old).await.ok();
    *runtime = self.resource.create(&self.config.resource_config, &self.credential, &ctx)
        .await.map_err(|e| Error::from(e.into()))?;

    self.state.consecutive_run_failures = 0;
    self.state.total_recreates += 1;
    self.state.window_recreates += 1;
    Ok(())
}
```

---

## #9 — Config hot-reload: нет rollback (05-manager.md) ✅

**Проблема.** Если новый config частично применился (evict old instances, start creating new)
но упал при create → inconsistent state. Для Resident: partial apply = полное отсутствие runtime.

**Решение.** Two-phase reload — create и validate ПЕРЕД swap. Для Resident/Service/Exclusive:
создаём новый runtime → health check → atomic swap. Только потом destroy old.

```rust
pub enum ReloadResult {
    Applied,
    RolledBack { reason: String },
    Skipped    { reason: String },
}

// В TopologyRuntime::on_config_changed() (per-topology):

// Pool — безопасен (lazy eviction):
TopologyRuntime::Pool(rt) => {
    rt.update_config_fingerprint(&new_config); // lazy: stale evicted at recycle
    rt.set_config(new_config);
    Ok(ReloadResult::Applied)
}

// Resident — two-phase:
TopologyRuntime::Resident(rt) => {
    // Phase 1: create new (old untouched).
    let new_runtime = match resource.create(&new_config, &credential, ctx).await {
        Ok(r) => r,
        Err(e) => return Ok(ReloadResult::RolledBack {
            reason: format!("create failed: {}", e)
        }),
    };
    // Phase 2: health check new runtime.
    if let Err(e) = resource.check(&new_runtime).await {
        resource.destroy(new_runtime).await.ok();
        return Ok(ReloadResult::RolledBack {
            reason: format!("health check failed: {}", e)
        });
    }
    // Phase 3: atomic swap. Old Arc drops when refcount → 0.
    let old_arc = rt.cell.swap(new_runtime);
    rt.set_config(new_config);
    // Phase 4: destroy old.
    if let Some(old) = Arc::try_unwrap(old_arc).ok() {
        resource.shutdown(&old).await.ok();
        resource.destroy(old).await.ok();
    }
    Ok(ReloadResult::Applied)
}

// Service — аналогично, но natural drain через Arc refcount:
TopologyRuntime::Service(rt) => {
    let new_runtime = match resource.create(&new_config, &credential, ctx).await {
        Ok(r) => r,
        Err(e) => return Ok(ReloadResult::RolledBack { reason: e.to_string() }),
    };
    if let Err(e) = resource.check(&new_runtime).await {
        resource.destroy(new_runtime).await.ok();
        return Ok(ReloadResult::RolledBack { reason: e.to_string() });
    }
    rt.swap_runtime(new_runtime, new_config); // spawn drain watchdog (см. #10)
    Ok(ReloadResult::Applied)
}

// Daemon — cancel + restart:
TopologyRuntime::Daemon(rt) => {
    rt.cancel_and_restart(new_config).await;
    Ok(ReloadResult::Applied)
}
```

`ReloadResult::RolledBack` сигнализирует `ManagerConfigAdapter::configure()` о том что
config не применился — старый конфиг активен.

---

## #10 — Service natural drain: нет deadline (05-manager.md) ✅

**Проблема.** Если caller держит `Arc<OldRuntime>` бесконечно (leaked handle, long task)
— старый runtime никогда не уничтожится. Memory leak + zombie runtime (старый Telegram bot
с устаревшим token продолжает работать).

**Решение.** Drain watchdog с deadline. Не форсирует drop Arc (невозможно), но логирует
утечки и metrics.

```rust
// В service::Runtime::swap_runtime() при config reload:
pub fn swap_runtime(&self, new_runtime: R::Runtime, config: R::Config) {
    let old_arc = self.current.swap(Arc::new(ServiceInner {
        runtime: new_runtime,
        config,
    }));

    // Downgrade: natural drain через Arc refcount.
    let weak = Arc::downgrade(&old_arc);
    drop(old_arc); // release наша strong ref

    // Spawn watchdog.
    let drain_deadline = self.config.drain_deadline; // default: 5 minutes
    let resource_key   = R::KEY;

    tokio::spawn(async move {
        let start = Instant::now();
        loop {
            if weak.strong_count() == 0 {
                tracing::info!(%resource_key, "old runtime drained naturally");
                return;
            }
            if start.elapsed() > drain_deadline {
                let remaining = weak.strong_count();
                tracing::warn!(
                    %resource_key,
                    remaining_refs = remaining,
                    deadline = ?drain_deadline,
                    "old runtime drain deadline exceeded"
                );
                // Нельзя форсировать drop Arc holders.
                // Логируем + metrics. Old Arc дропнется когда holders завершатся.
                // TODO (v2): forced drain через CancellationToken в ServiceInner.
                return;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

// service::Config добавить:
pub struct service::Config {
    // ...
    /// Timeout до warn о leaked old runtime при config reload.
    /// Default: 5 minutes.
    pub drain_deadline: Duration,
}
impl Default for service::Config {
    fn default() -> Self {
        Self {
            drain_deadline: Duration::from_secs(300),
            // ...
        }
    }
}
```

**V2 улучшение (deferred):** `ServiceInner` хранит `CancellationToken`. Watchdog вызывает
`old_token.cancel()` при deadline. Service::acquire_token() проверяет token и возвращает
ошибку — graceful forced drain.

---

## #11 — Extensions: TypeId collision (01-core.md) ✅

**Проблема.** Разные версии одного crate дают разные `TypeId` для "одного" типа.
С duplicated dependencies в workspace: `ext::<TenantContext>()` возвращает `None`
когда `TenantContext` есть, но из другой версии крейта. Silent None → prepare() пропускает
`SET search_path` → cross-tenant data access.

**Решение.** Debug-mode validation + named insertion. В production TypeId lookup работает
как раньше (O(1)), без overhead. Debug mode добавляет строковый индекс для диагностики.

```rust
pub struct Extensions {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,

    /// Только в debug builds: type_name → TypeId.
    /// Используется для диагностики дублирования версий.
    #[cfg(debug_assertions)]
    name_index: HashMap<&'static str, TypeId>,
}

impl Extensions {
    pub fn insert<T: Send + Sync + 'static>(&mut self, value: T) {
        let type_id = TypeId::of::<T>();

        #[cfg(debug_assertions)]
        {
            let type_name = std::any::type_name::<T>();
            if let Some(&existing_id) = self.name_index.get(type_name) {
                if existing_id != type_id {
                    // Два разных TypeId для одного имени = duplicated crate version.
                    tracing::error!(
                        type_name,
                        "TypeId collision detected in Extensions. \
                         Two versions of the same crate are loaded. \
                         ext::<{}> will silently return None for some callers. \
                         Fix: unify dependency versions in Cargo.toml.",
                        type_name
                    );
                }
            }
            self.name_index.insert(type_name, type_id);
        }

        self.map.insert(type_id, Box::new(value));
    }

    pub fn get<T: Send + Sync + 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|v| v.downcast_ref::<T>())
    }

    /// Debug: найти по type_name строке.
    /// Полезно если TypeId lookup вернул None — проверить есть ли коллизия.
    #[cfg(debug_assertions)]
    pub fn debug_get_by_name(&self, type_name: &str) -> Option<&dyn Any> {
        let type_id = self.name_index.get(type_name)?;
        self.map.get(type_id).map(|v| v.as_ref() as &dyn Any)
    }
}
```

**Долгосрочное решение:** унифицировать версии `TenantContext` крейта в workspace через
`cargo deny` / `cargo tree`. Это infrastructure issue, не runtime issue. Extensions
debug logging помогает обнаружить проблему быстро.

---

## #12 — Audit trail: нет structured logging для resource access (05-manager.md) ✅

**Проблема.** `AcquireOptions` имеет tags для metrics, но нет structured audit logging:
кто (execution_id, scope, user), какой ресурс, когда, как долго держал. Для
compliance-sensitive ресурсов (DB с PII, payment API) — пробел.

**Решение.** Audit span в acquire path. Автоматически — не нужно action author-у делать ничего.

```rust
// В ManagedResource::acquire() — после успешного handle:
tracing::info!(
    target:       "nebula_resource::audit",
    resource_key  = %R::KEY,
    resource_id   = %self.resource_id,
    execution_id  = %ctx.execution_id(),
    scope         = ?ctx.scope(),
    topology      = self.topology_tag(),
    intent        = ?options.intent,
    "resource.acquire"
);

// В HandleInner Drop (через on_release callback в release pipeline):
tracing::info!(
    target:       "nebula_resource::audit",
    resource_key  = %resource_key,
    execution_id  = %execution_id,
    hold_ms       = hold_duration.as_millis(),
    tainted       = is_tainted,
    "resource.release"
);
```

`target: "nebula_resource::audit"` — позволяет включить/выключить audit logging отдельно:

```
RUST_LOG=nebula_resource::audit=info,nebula_resource=warn
```

**Structured fields для audit events (обязательные):**
- `resource_key` — тип ресурса
- `resource_id` — конкретный instance
- `execution_id` — execution context
- `scope` — tenant/org/project/workflow
- `intent` — AcquireIntent (Standard/LongRunning/Critical/...)
- `hold_ms` — при release: сколько держал

**Opinionated:** audit logging через `tracing` (не отдельный audit crate), потому что:
1. Уже есть в стеке (`nebula-log` wraps tracing).
2. Subscriber-based: production может sink в SIEM через tracing subscriber.
3. Structured fields нативно поддерживаются.

---

## #13 — MemoryMonitor: Mutex contention (05-manager.md) ✅

**Проблема.** `Option<Arc<Mutex<MemoryMonitor>>>` — `Mutex` на горячем пути maintenance loop.
При 50 pools — contention. Каждый pool периодически лочит Mutex для `check_pressure()`.

**Решение.** Периодический snapshot в `AtomicU8`. Maintenance loop читает атомик (lock-free),
отдельный task обновляет snapshot каждые N секунд.

```rust
/// Lock-free pressure snapshot. Обновляется фоновым task-ом.
/// Читается в maintenance loop каждого pool (lock-free).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureLevel {
    Normal   = 0,
    Moderate = 1,
    High     = 2,
    Critical = 3,
}

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

// Обновлённый Manager:
pub struct Manager {
    // ...
    /// MemoryMonitor (Mutex, но лочится только background task раз в N сек).
    memory_monitor:   Option<Arc<Mutex<MemoryMonitor>>>,
    /// Lock-free snapshot. Читается в maintenance loops без lock.
    pressure_snapshot: Arc<PressureSnapshot>,
}

impl Manager {
    pub fn with_memory_monitor(mut self, monitor: MemoryMonitor, check_interval: Duration) -> Self {
        let monitor = Arc::new(Mutex::new(monitor));
        let snapshot = Arc::new(PressureSnapshot { level: AtomicU8::new(0) });

        // Один background task обновляет snapshot с интервалом.
        let mon_clone  = Arc::clone(&monitor);
        let snap_clone = Arc::clone(&snapshot);
        let cancel     = self.cancel.child_token();
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
                            snap_clone.store(level);
                        }
                    }
                }
            }
        });

        self.memory_monitor   = Some(monitor);
        self.pressure_snapshot = snapshot;
        self
    }
}

// Maintenance loop каждого pool:
let pressure = manager.pressure_snapshot.load(); // AtomicU8::load — нет lock
match pressure {
    PressureLevel::High     => pool.shrink_idle_by(0.25),
    PressureLevel::Critical => pool.shrink_idle_by(0.50),
    _                       => {}
}
```

---

# Часть II — Дополнительные проблемы (code review раунд 2)

Найдены после ревью всех план-файлов. Нумерация продолжает предыдущий список (#14–#25).

---

## #14 — ReleaseQueue workers: параллелизм убит общим Mutex (03-infrastructure.md) ✅

**Проблема.** Multi-worker дизайн с `Arc<AsyncMutex<Receiver>>`:
```rust
task = async { rx.lock().await.recv().await } => { ... }
```
Все 4 workers соревнуются за один Mutex. В каждый момент — 3 idle, 1 работает.
Для Browser (~500ms recycle) это означает throughput = 1 release / 500ms вместо 4 / 500ms.
Задача с configurable `num_workers` теряет смысл.

**Решение.** Отдельный `mpsc::Receiver` на каждого worker-а. Primary channel → N workers
каждый со своим rx. Fallback unbounded — один, все workers читают через `Arc<AsyncMutex>`
(fallback редкий, поэтому contention не критично).

```rust
impl ReleaseQueue {
    pub fn new(capacity: usize, num_workers: usize, cancel: CancellationToken) -> Self {
        // Один fallback для overflow. Contention здесь допустима (редкий путь).
        let (fallback_tx, fallback_rx) = mpsc::unbounded_channel();
        let fallback_rx = Arc::new(AsyncMutex::new(fallback_rx));
        let metrics = Arc::new(ReleaseQueueMetrics::default());

        // N независимых primary receivers — настоящий параллелизм.
        let mut senders = Vec::with_capacity(num_workers);
        let workers = (0..num_workers).map(|_| {
            let (tx, rx) = mpsc::channel(capacity / num_workers.max(1) + 1);
            senders.push(tx);
            let cancel   = cancel.clone();
            let fb_rx    = Arc::clone(&fallback_rx);
            let metrics  = Arc::clone(&metrics);
            tokio::spawn(async move {
                loop {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => {
                            while let Ok(t) = rx.try_recv() { (t.release_fn)().await; }
                            // fallback drain — shared, один раз
                            let mut fb = fb_rx.lock().await;
                            while let Ok(t) = fb.try_recv() { (t.release_fn)().await; }
                            break;
                        }
                        task = rx.recv() => {          // ← свой rx, нет Mutex!
                            match task { Some(t) => (t.release_fn)().await, None => break }
                        }
                        task = async { fb_rx.lock().await.recv().await } => {
                            match task { Some(t) => (t.release_fn)().await, None => {} }
                        }
                    }
                }
            })
        }).collect();

        // Round-robin sender selection при submit().
        Self { senders, next_worker: AtomicUsize::new(0), fallback_tx, metrics, workers }
    }

    pub fn submit(&self, task: ReleaseTask) {
        self.metrics.submitted.fetch_add(1, Ordering::Relaxed);
        // Round-robin → равномерное распределение.
        let idx = self.next_worker.fetch_add(1, Ordering::Relaxed) % self.senders.len();
        let tx  = &self.senders[idx];
        match tx.try_send(task) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(task)) => {
                self.metrics.fallback_used.fetch_add(1, Ordering::Relaxed);
                tracing::warn!("ReleaseQueue worker {} full, using fallback", idx);
                if self.fallback_tx.send(task).is_err() {
                    self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                    tracing::error!("ReleaseQueue fallback closed, task dropped");
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}
```

---

## #15 — Daemon try_recreate: `mem::zeroed()` UB (02-topology.md) ✅

**Проблема.** В описании `try_recreate`:
```rust
let old = std::mem::replace(runtime, unsafe { std::mem::zeroed() });
```
`R::Runtime` не обязан иметь valid zero-bit pattern. `Arc` с нулевым указателем = UB при drop.
`JoinHandle` с нулевыми bytes = UB. Это UB на первом daemon recreate.

**Решение.** `Option<R::Runtime>` в daemon state. `None` = "slot temporarily empty".

```rust
// В daemon::Runtime внутреннее состояние:
struct DaemonState<R: Resource> {
    runtime:              Option<R::Runtime>,  // None только во время recreate
    consecutive_failures: u32,
    total_recreates:      u32,
    window_recreates:     u32,
    window_started:       Option<Instant>,
}

// В try_recreate():
async fn try_recreate(state: &mut DaemonState<R>, ...) -> Result<(), Error> {
    // ... budget checks ...

    // Взять runtime (оставить None в слоте).
    let old = state.runtime.take()
        .ok_or_else(|| Error::permanent("daemon runtime in inconsistent state"))?;

    resource.shutdown(&old).await.ok();
    resource.destroy(old).await.ok();

    // Создать новый.
    let new = resource.create(config, credential, ctx)
        .await
        .map_err(|e| Error::from(e.into()))?;

    state.runtime = Some(new);
    Ok(())
}

// Restart loop использует `state.runtime.as_ref().unwrap()` для run().
// None между shutdown и create — gap только внутри try_recreate, не виден снаружи.
```

---

## #16 — Cell<T>: структурно сломан (03-infrastructure.md) ✅

**Проблема.** `ArcSwap<Option<T>>` + `load_arc()` возвращает `Arc<Option<T>>`.
1. `guard.is_some()` всегда true (проверяет не-null Arc, а не inner Option).
2. Caller получает `Arc<Option<T>>`, а не `Arc<T>` — clone для Lease неудобен.
3. `store(value: T)` → `Arc::new(Some(value))` — лишняя обёртка.

**Решение.** Использовать `ArcSwapOption<T>` или `ArcSwap<T>` напрямую.

```rust
use arc_swap::ArcSwapOption;

/// Lock-free ячейка для одного значения.
/// Read = load_full() → Option<Arc<T>>. One atomic op.
/// Write = store(Arc<T>) → atomic swap. Old Arc dropped when refcount → 0.
pub struct Cell<T: Send + Sync + 'static> {
    inner: ArcSwapOption<T>,
}

impl<T: Send + Sync + 'static> Cell<T> {
    pub fn empty() -> Self {
        Self { inner: ArcSwapOption::empty() }
    }

    pub fn new(value: T) -> Self {
        Self { inner: ArcSwapOption::from_pointee(value) }
    }

    /// Read. One atomic op. Returns Arc<T> or None if not yet initialized.
    pub fn load(&self) -> Option<Arc<T>> {
        self.inner.load_full()
    }

    /// Atomic store.
    pub fn store(&self, value: T) {
        self.inner.store(Some(Arc::new(value)));
    }

    /// Swap and return old Arc<T>.
    pub fn swap(&self, value: T) -> Option<Arc<T>> {
        self.inner.swap(Some(Arc::new(value)))
    }
}
```

Resident acquire: `cell.load()? → Arc<T>` → clone T (если T: Clone) → `HandleInner::Owned`.

---

## #17 — RecoveryWaiter: borrowed lifetime в async context (04-recovery-resilience.md) ✅

**Проблема.** `RecoveryWaiter<'a>` хранит `&'a ArcSwap<GateState>` и `&'a Notify`.
`RecoveryGate` обычно в `Arc<RecoveryGate>`. Future с borrowed refs не `'static` →
не совместим с `tokio::spawn`. Lifetime errors при имплементации.

**Решение.** `RecoveryWaiter` хранит `Arc` клоны вместо borrowed refs.

```rust
/// Owned waiter — 'static, compatible with tokio::spawn.
pub struct RecoveryWaiter {
    state:  Arc<ArcSwap<GateState>>,
    notify: Arc<Notify>,
}

// RecoveryGate обновлённый:
pub struct RecoveryGate {
    state:                 Arc<ArcSwap<GateState>>,  // Arc вместо owned
    notify:                Arc<Notify>,               // Arc вместо owned
    max_recovery_attempts: u32,
}

impl RecoveryGate {
    fn make_waiter(&self) -> RecoveryWaiter {
        RecoveryWaiter {
            state:  Arc::clone(&self.state),
            notify: Arc::clone(&self.notify),
        }
    }
}

impl RecoveryWaiter {
    pub async fn wait(&self) -> Result<(), Arc<crate::Error>> {
        loop {
            match self.state.load().as_ref() {
                GateState::Idle => return Ok(()),
                // ... same logic as before
            }
        }
    }
}
```

`RecoveryWaiter` теперь `'static` и `Send` — можно передавать в tokio tasks.

---

## #18 — Registry::get_typed не может вернуть &ManagedResource<R> если lookup async (05-manager.md) ✅

**Проблема.** После amendment #4 `scope_is_compatible` стала async (ScopeResolver).
`get_typed() -> &ManagedResource<R>` возвращает borrowed ref из self. async fn не может
заимствовать self через await. "Typed hot path zero allocation" несовместима с async lookup.

**Решение.** Возвращать `Arc<ManagedResource<R>>` (одна аллокация, уже есть в Registry).

```rust
impl Registry {
    /// Typed fast path. downcast_ref + Arc clone.
    /// Scope check async только в Strict mode (CachedScopeResolver = ~10ns warm path).
    pub async fn get_typed<R: Resource>(
        &self,
        id:    ResourceId,
        scope: &ScopeLevel,
    ) -> Result<Arc<ManagedResource<R>>> {
        let erased = self.get_erased(TypeId::of::<R>(), id, scope).await?;
        erased.as_any()
            .downcast_ref::<ManagedResource<R>>()
            .map(|_| {
                // SAFETY: мы уже проверили тип. Clone Arc.
                // erased это Arc<dyn AnyManagedResource>, downcast к конкретному типу.
                Arc::clone(&erased)
                    .downcast_arc::<ManagedResource<R>>()
                    .expect("type mismatch after downcast_ref succeeded")
            })
            .ok_or_else(|| Error::not_found("resource type mismatch"))
    }

    pub async fn get_erased(
        &self,
        type_id: TypeId,
        id:      ResourceId,
        scope:   &ScopeLevel,
    ) -> Result<Arc<dyn AnyManagedResource>> {
        let entries = self.by_type.get(&(type_id, id))
            .ok_or_else(|| Error::not_found("resource"))?;

        let mut best: Option<Arc<dyn AnyManagedResource>> = None;
        let mut best_spec = -1i8;

        for entry in entries.iter() {
            if self.scope_compatible(&entry.scope, scope).await {
                let spec = scope_specificity(&entry.scope) as i8;
                if spec > best_spec {
                    best_spec = spec;
                    best = Some(Arc::clone(&entry.managed));
                }
            }
        }

        best.ok_or_else(|| Error::not_found("resource for scope"))
    }
}
```

**Performance note:** Warm path через `CachedScopeResolver` = одна atomic load (moka).
Cold path = DB round-trip. Документируем явно что Strict mode имеет latency implications.

---

## #19 — CredentialStore: не object-safe из-за generic метода (01-core.md) ✅

**Проблема.** `fn resolve<C: Credential>` — generic method, `dyn CredentialStore` не valid.
`CredentialCtx::credential_store() -> &dyn CredentialStore` не скомпилируется.

**Решение.** Type erasure через `BoxFuture` + downcast.

```rust
/// Type-erased credential store для dynamic dispatch.
pub trait CredentialStore: Send + Sync {
    /// Type-erased resolve. Возвращает `Box<dyn Any + Send + Sync>`.
    /// Framework downcast-ит к нужному типу.
    fn resolve_erased(
        &self,
        scope: &ScopeLevel,
        kind:  &'static str,
    ) -> BoxFuture<'_, Result<Box<dyn Any + Send + Sync>, CredentialError>>;
}

/// Extension: typed helper поверх CredentialStore.
/// Не dyn — вызывается только в typed context (ManagedResource::create_instance).
trait CredentialStoreExt: CredentialStore {
    fn resolve<C: Credential + 'static>(
        &self,
        scope: &ScopeLevel,
    ) -> impl Future<Output = Result<C, CredentialError>> + Send {
        async move {
            let boxed = self.resolve_erased(scope, C::KIND).await?;
            boxed.downcast::<C>()
                .map(|b| *b)
                .map_err(|_| CredentialError::TypeMismatch {
                    expected: C::KIND,
                    got: "unknown",
                })
        }
    }
}
impl<T: CredentialStore> CredentialStoreExt for T {}

// CredentialCtx возвращает &dyn CredentialStore (object-safe):
pub trait CredentialCtx: Ctx {
    fn credential_store(&self) -> &dyn CredentialStore;
}

// В ManagedResource::create_instance():
async fn create_instance<R: Resource>(
    resource: &R,
    config:   &R::Config,
    ctx:      &dyn CredentialCtx,
) -> Result<R::Runtime, Error> {
    let credential = ctx.credential_store()
        .resolve::<R::Credential>(ctx.scope())  // ← CredentialStoreExt::resolve (typed)
        .await
        .map_err(|e| Error::permanent(e))?;
    resource.create(config, &credential, ctx).await.map_err(Into::into)
}
```

---

## #20 — ScopeResolver: не object-safe из-за `impl Future` (05-manager.md) ✅

**Проблема.** `fn is_child_of -> impl Future` не object-safe. `Arc<dyn ScopeResolver>` не valid.

**Решение.** `BoxFuture` return type (как в `#[async_trait]` desugaring).

```rust
use futures::future::BoxFuture;

/// Resolver parent-child scope relationships. Object-safe.
pub trait ScopeResolver: Send + Sync {
    fn is_child_of<'a>(
        &'a self,
        child:  &'a ScopeLevel,
        parent: &'a ScopeLevel,
    ) -> BoxFuture<'a, bool>;
}

// Реализация с async:
impl ScopeResolver for DbScopeResolver {
    fn is_child_of<'a>(&'a self, child: &'a ScopeLevel, parent: &'a ScopeLevel)
        -> BoxFuture<'a, bool>
    {
        Box::pin(async move {
            // DB lookup: "is child's org_id == parent's org_id?"
            self.db.query_scope_relation(child, parent).await.unwrap_or(false)
        })
    }
}

impl ScopeResolver for RequiresScopeResolver {
    fn is_child_of<'a>(&'a self, _: &'a ScopeLevel, _: &'a ScopeLevel)
        -> BoxFuture<'a, bool>
    {
        Box::pin(async { panic!("ScopeResolver not configured.") })
    }
}
```

Аналогично `CredentialStore::resolve_erased` — используем `BoxFuture` везде где нужен `dyn`.

---

## #21 — Transport: нет лимита на concurrent sessions (02-topology.md) ✅

**Проблема.** Transport открывает сессии без лимита. SSH connection имеет ~10-100 channel
лимит. При leaked handle-ах или высокой нагрузке — `open_session()` падает, triggering
recovery gate. Gate не помогает (transport жив), acquire зависает.

**Решение.** `max_sessions` + `Arc<Semaphore>` в `transport::Config` и `transport::Runtime`.

```rust
// transport::Config:
pub struct transport::Config {
    /// Максимум одновременных открытых sessions. None = без лимита.
    /// SSH: typically 10-100 (ssh_config MaxSessions). Default: 32.
    pub max_sessions:         Option<usize>,
    /// Timeout ожидания session permit.
    pub session_acquire_timeout: Duration,
    // ...
}

// transport::Runtime<R> хранит:
struct TransportRuntime<R: Resource + Transport> {
    runtime:  Arc<R::Runtime>,
    // Semaphore ограничивает concurrent open sessions.
    // None если max_sessions = None.
    semaphore: Option<Arc<Semaphore>>,
    // ...
}

// В acquire():
async fn acquire_session(rt: &TransportRuntime<R>, ...) -> Result<ResourceHandle<R>> {
    let _permit = if let Some(sem) = &rt.semaphore {
        Some(
            tokio::time::timeout(
                rt.config.session_acquire_timeout,
                sem.acquire_owned(),
            )
            .await
            .map_err(|_| Error::exhausted(SessionLimitExceeded, None))??
        )
    } else {
        None
    };

    let session = resource.open_session(&rt.runtime, ctx).await
        .map_err(Into::into)?;

    // Handle: Guarded с on_release = close_session + drop permit.
    Ok(ResourceHandle::guarded(
        session,
        Box::new(move |session, tainted| {
            // permit dropped AFTER close_session.
            release_queue.submit(ReleaseTask::new(async move {
                resource.close_session(&runtime, session, !tainted).await.ok();
                drop(_permit);  // permit released → next caller unblocked
            }));
        }),
        R::KEY,
        "transport",
    ))
}
```

---

## #22 — WatchdogHandle: ticket получен но не resolved → gate навсегда stuck (04-recovery-resilience.md) ✅

**Проблема.** В `WatchdogHandle::spawn`:
```rust
if let Ok(ticket) = gate.try_begin() {
    // Recovery logic... (recreate runtime)
    // On success: gate.resolve(ticket)
    // On failure: gate.fail_transient(ticket, err, backoff)
}
```
Ticket получен, recovery logic = комментарий, ticket drop-нут.
`RecoveryTicket` дропается без вызова `resolve/fail_*` → gate навсегда в `InProgress`.
Все последующие acquire висят на `wait()` которое никогда не fires.

**Решение.** Реализовать recovery logic + добавить Drop guard на ticket.

```rust
// RecoveryTicket: Drop guard — если дропнут без resolve/fail → паникует в debug, fail_transient в release.
pub struct RecoveryTicket {
    pub(crate) attempt: u32,
    gate: Arc<RecoveryGate>,   // weak ref для auto-fail
    resolved: bool,
    _private: (),
}

impl Drop for RecoveryTicket {
    fn drop(&mut self) {
        if !self.resolved {
            // Ticket забыли завершить — auto fail с generic error.
            // Иначе gate навсегда stuck в InProgress.
            debug_assert!(false, "RecoveryTicket dropped without resolve/fail — framework bug");
            self.gate.fail_transient_internal(
                crate::Error::permanent("recovery ticket dropped without completion"),
                Duration::from_secs(5), // short backoff for framework bug
            );
        }
    }
}

// WatchdogHandle recovery logic:
if let Ok(ticket) = gate.try_begin() {
    let resource = Arc::clone(&resource);
    let runtime  = Arc::clone(&runtime);
    let gate_    = Arc::clone(&gate);
    let config_  = config.clone();

    tokio::spawn(async move {
        let result = tokio::time::timeout(
            watchdog_config.probe_timeout * 3,
            do_recovery(&resource, &runtime, &config_),
        ).await;

        match result {
            Ok(Ok(())) => gate_.resolve(ticket),
            Ok(Err(e)) => gate_.fail_transient(ticket, e, watchdog_config.backoff.initial),
            Err(_)     => gate_.fail_transient(
                ticket,
                crate::Error::transient("recovery probe timeout"),
                watchdog_config.backoff.initial,
            ),
        }
    });
}

async fn do_recovery<R: Resource>(
    resource: &R,
    runtime:  &R::Runtime,
    config:   &R::Config,
) -> Result<(), crate::Error> {
    // Liveness check.
    resource.check(runtime).await.map_err(Into::into)
    // Для более сложного recovery (recreate) — topology-specific logic.
}
```

---

## #23 — Resident::is_alive sync но пример делает blocking I/O (02-topology.md) ✅

**Проблема.** Trait определяет `fn is_alive(&self, runtime: &Self::Runtime) -> bool` (sync).
Пример Kafka:
```rust
fn is_alive(&self, producer: &FutureProducer) -> bool {
    producer.client().fetch_metadata(None, Duration::from_secs(2)).is_ok()
}
```
`fetch_metadata` — blocking I/O с 2-секундным timeout прямо в async task. Блокирует tokio thread.

**Решение.** Переименовать в `is_alive_sync` с явным контрактом + убрать пример с I/O.
Для I/O-based health detection — использовать `Resource::check()` + `stale_after`.

```rust
pub trait Resident: Resource {
    /// Sync liveness check. ОБЯЗАН быть O(1), без IO, без blocking.
    /// Проверяет только internal state flags.
    ///
    /// ПРАВИЛЬНО: проверить атомик, статус Arc, флаг closed.
    /// НЕПРАВИЛЬНО: делать network I/O, lock, sleep.
    ///
    /// Для I/O-based health detection: используйте Resource::check() + stale_after().
    /// Framework вызовет check() async перед acquire если stale.
    fn is_alive_sync(&self, runtime: &Self::Runtime) -> bool { true }

    /// Interval между async Resource::check() вызовами.
    /// None = без периодических проверок.
    fn stale_after(&self) -> Option<Duration> { None }
}

// Пример Kafka — правильно:
impl Resident for KafkaProducer {
    fn is_alive_sync(&self, producer: &FutureProducer) -> bool {
        // Только sync check — есть ли connection error flag?
        !producer.client().fatal_error().is_some()
        // Полная проверка (fetch_metadata) происходит через stale_after + Resource::check()
    }

    fn stale_after(&self) -> Option<Duration> {
        Some(Duration::from_secs(30)) // → вызовет Resource::check() раз в 30 сек
    }
}

// Resource::check() для Kafka (async, I/O ok):
impl Resource for KafkaProducer {
    async fn check(&self, producer: &FutureProducer) -> Result<(), KafkaError> {
        producer.client()
            .fetch_metadata(None, Duration::from_secs(2))
            .map_err(KafkaError::MetadataFetch)?;
        Ok(())
    }
}
```

---

## #24 — Manager::new() не инициализирует pressure_snapshot (05-manager.md) ✅

**Проблема.** `Manager` struct получил поле `pressure_snapshot: Arc<PressureSnapshot>` в amendment #13,
но `Manager::new()` не инициализирует его.

**Решение.** Добавить в `new()`:

```rust
impl Manager {
    pub fn new(telemetry: Arc<dyn TelemetryService>) -> Self {
        Self {
            registry:          Registry::new(),
            recovery_groups:   RecoveryGroupRegistry::new(),
            cancel:            CancellationToken::new(),
            telemetry,
            resource_bus:      Arc::new(EventBus::new(256)),
            memory_monitor:    None,
            pressure_snapshot: Arc::new(PressureSnapshot { level: AtomicU8::new(0) }),
            containment_mode:  ContainmentMode::Strict,
            scope_resolver:    Arc::new(RequiresScopeResolver),
        }
    }
}
```

---

## #25 — ResourceConfig::fingerprint() = 0 default: stale detection silently bypassed (01-core.md) ✅

**Проблема.** Default `fn fingerprint() -> u64 { 0 }` — если impl забыл override,
все config changes невидимы для pool eviction. Zero compile-time protection.

**Решение.** Предупреждение через `#[must_use]` + debug assertion + документация.

```rust
pub trait ResourceConfig: Send + Sync + Clone + 'static {
    fn validate(&self) -> Result<()> { Ok(()) }

    /// Стабильный fingerprint для stale instance detection.
    ///
    /// ВАЖНО: если config может изменяться (statement_timeout, search_path, etc.)
    /// — реализуйте этот метод. Default 0 означает что pool НИКОГДА не будет
    /// считать instances stale при смене конфига.
    ///
    /// Используйте стабильный hasher (не DefaultHasher — нестабилен между процессами):
    ///   use std::hash::{Hash, Hasher};
    ///   let mut h = rustc_hash::FxHasher::default(); // или xxhash / fnv
    ///   self.field.hash(&mut h);
    ///   h.finish()
    ///
    /// НЕ включайте credentials — они rotated отдельно.
    fn fingerprint(&self) -> u64 { 0 }
}

// В pool maintenance loop — предупреждение в debug builds:
#[cfg(debug_assertions)]
fn check_fingerprint_configured<R: Resource + Pooled>(config: &R::Config) {
    if config.fingerprint() == 0 {
        tracing::debug!(
            resource_key = %R::KEY,
            "ResourceConfig::fingerprint() returns 0. \
             Config changes will NOT trigger pool eviction. \
             Override fingerprint() if config affects connection behavior."
        );
    }
}
```

Также меняем пример `PgResourceConfig::fingerprint()` с `DefaultHasher` на `FxHasher`
(стабильный cross-process):

```rust
impl ResourceConfig for PgResourceConfig {
    fn fingerprint(&self) -> u64 {
        use rustc_hash::FxHasher;
        use std::hash::{Hash, Hasher};
        let mut h = FxHasher::default();
        self.statement_timeout.map(|d| d.as_millis() as u64).hash(&mut h);
        self.application_name.hash(&mut h);
        self.search_path.hash(&mut h);
        h.finish()
    }
}
```

---

## Сводная таблица изменений по план-файлам

| # | Проблема | Файл | Тип изменения | Статус |
|---|----------|------|---------------|--------|
| 1 | ReleaseQueue silent drop | 03-infrastructure.md | Replace submit() + add ReleaseQueueMetrics | ✅ |
| 2 | HandleInner Deref panic | 03-infrastructure.md | Replace detach() return type + Deref unreachable! | ✅ |
| 3 | Credential design gap | 01-core.md | Add Credential trait + CredentialCtx + update Resource::create() | ✅ |
| 4 | Scope cross-tenant | 05-manager.md | Add ContainmentMode + ScopeResolver + strict default | ✅ |
| 5 | PoisonToken race | 03-infrastructure.md | Pass Arc<AtomicBool> to release_fn | ✅ |
| 6 | RecoveryGate todo | 04-recovery-resilience.md | Implement try_begin() CAS loop + update GateState | ✅ |
| 7 | Pool retry same instance | 07-implementation.md | Add blacklist to pool/acquire.rs spec | 🔧 |
| 8 | Daemon infinite recreate | 02-topology.md | Add RecreateBudget to daemon::Config | ✅ |
| 9 | Config no rollback | 05-manager.md | Two-phase reload + ReloadResult | ✅ |
| 10 | Service drain no deadline | 05-manager.md | Drain watchdog in swap_runtime() | ✅ |
| 11 | Extensions TypeId collision | 01-core.md | Debug-mode name_index + warn on collision | ✅ |
| 12 | No audit trail | 05-manager.md | tracing audit span in acquire/release | ✅ |
| 13 | MemoryMonitor Mutex | 05-manager.md | AtomicU8 PressureSnapshot + background updater | ✅ |
| 14 | ReleaseQueue workers Mutex | 03-infrastructure.md | N independent receivers, round-robin submit | ✅ |
| 15 | Daemon mem::zeroed() UB | 02-topology.md | Option<R::Runtime> in daemon state | ✅ |
| 16 | Cell<T> structurally broken | 03-infrastructure.md | ArcSwapOption<T> instead of ArcSwap<Option<T>> | ✅ |
| 17 | RecoveryWaiter borrowed lifetime | 04-recovery-resilience.md | Arc clones instead of borrowed refs | ✅ |
| 18 | Registry::get_typed borrow vs async | 05-manager.md | Return Arc<ManagedResource<R>> | ✅ |
| 19 | CredentialStore not object-safe | 01-core.md | BoxFuture + resolve_erased type erasure | ✅ |
| 20 | ScopeResolver not object-safe | 05-manager.md | BoxFuture return type | ✅ |
| 21 | Transport no session limit | 02-topology.md | max_sessions + Semaphore | ✅ |
| 22 | WatchdogHandle ticket leak | 04-recovery-resilience.md | Drop guard on RecoveryTicket | ✅ |
| 23 | Resident::is_alive blocking I/O | 02-topology.md | Rename is_alive_sync + stale_after() | ✅ |
| 24 | Manager::new() missing init | 05-manager.md | Initialize pressure_snapshot in new() | ✅ |
| 25 | fingerprint() = 0 silent bypass | 01-core.md | Debug assertion + documentation | ✅ |

---

## Приоритеты реализации

### Критические (блокируют v1) — security и data integrity
- **#4** Scope isolation — cross-tenant security risk
- **#1** ReleaseQueue — connection leak под нагрузкой
- **#3** Credential design — иначе все примеры остаются с `todo!()`
- **#6** RecoveryGate — core concurrency primitive не реализован
- **#15** Daemon mem::zeroed() — UB на первом recreate
- **#19** CredentialStore object-safety — `dyn CredentialStore` не скомпилируется
- **#20** ScopeResolver object-safety — `Arc<dyn ScopeResolver>` не valid

### Серьёзные (нужны в v1) — correctness под нагрузкой
- **#8** Daemon recreate budget — infinite restart loop
- **#7** Pool retry blacklist — correctness issue под нагрузкой
- **#2** HandleInner Deref — framework bug potential
- **#5** PoisonToken race — connection leak под niche conditions
- **#14** ReleaseQueue workers — throughput 1/N от ожидаемого
- **#16** Cell<T> — guard.is_some() всегда true (логическая ошибка)
- **#17** RecoveryWaiter — не совместим с tokio::spawn (compile error)
- **#22** WatchdogHandle ticket — gate навсегда stuck

### Стоит включить (желательно в v1) — DX и operational safety
- **#9** Config rollback — consistency под reload
- **#10** Service drain deadline — memory leak detection
- **#12** Audit trail — compliance
- **#13** MemoryMonitor — performance under load
- **#11** Extensions debug — DX для debugging
- **#18** Registry::get_typed — async scope resolver ломает borrow
- **#21** Transport sessions — session exhaustion без лимита
- **#23** Resident is_alive — blocking I/O в async task
- **#24** Manager::new() — missing field init
- **#25** fingerprint() default — silent config stale bypass

---

## Примечания по интеграции

24 из 25 amendments интегрированы в соответствующие план-файлы.

**Не интегрирован:**
- **#7** (Pool retry blacklist) — не добавлен в 07-implementation.md. Blacklist per-acquire-cycle
  и `IdleQueue::checkout_excluding()` должны быть задокументированы в секции pool/acquire.rs.

**Нет противоречий между amendments:**
- #1 и #14 дополняют друг друга (двухуровневая очередь → N independent receivers).
- #6, #17, #22 последовательно улучшают RecoveryGate (implement → fix lifetime → add Drop guard).
- #3 и #19 последовательны (add CredentialStore → fix object-safety).
