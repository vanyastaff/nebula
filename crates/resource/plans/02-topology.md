# 02 — Topology: семь trait-ов

Каждая topology определяет как организовать Runtime instances и какова семантика acquire/release.

---

## Pooled

N interchangeable instances. Checkout из idle или create new. Самая используемая topology для stateful connections.

```rust
pub trait Pooled: Resource {
    /// Sync O(1) broken check. Вызывается в Drop path LeaseGuard.
    ///
    /// ОБЯЗАН быть дешёвым — нет async, нет IO.
    /// Проверяет: process alive? TCP closed? Error flags?
    ///
    /// BrokenCheck::NeedsAsyncCheck — "не знаю, проверь async".
    /// Pool тогда вызовет Resource::check() перед выдачей.
    fn is_broken(&self, runtime: &Self::Runtime) -> BrokenCheck {
        let _ = runtime;
        BrokenCheck::Healthy
    }

    /// Async recycle при возврате в pool.
    ///
    /// Получает:
    ///   &Self::Runtime — instance для recycle.
    ///   &InstanceMetrics — error_count, checkout_count, created_at.
    ///
    /// ТОЛЬКО instance cleanup: reset session state, verify alive.
    /// Policy decisions (stale config, max age, max lifetime) — framework handles
    /// BEFORE calling recycle(). Config не передаётся.
    ///
    /// Postgres: DISCARD ALL (Smart mode — только если была транзакция).
    /// Redis Dedicated: UNWATCH, SELECT default db, PING.
    /// Browser: clear cookies, clear storage, navigate about:blank (~500ms).
    fn recycle(
        &self,
        runtime: &Self::Runtime,
        metrics: &InstanceMetrics,
    ) -> impl Future<Output = Result<RecycleDecision, Self::Error>> + Send {
        let _ = (runtime, metrics);
        async { Ok(RecycleDecision::Keep) }
    }

    /// Подготовить instance для конкретного execution context.
    ///
    /// Вызывается фреймворком ПОСЛЕ checkout, ПЕРЕД выдачей caller-у.
    /// Caller получает уже подготовленный instance.
    ///
    /// Use cases:
    ///   Postgres: SET search_path TO tenant_schema; SET ROLE tenant_user;
    ///   Redis Dedicated: SELECT tenant_database; CLIENT SETNAME;
    ///   Browser: navigate to login page; set auth cookies;
    ///
    /// Tenant context приходит через ctx.ext::<TenantContext>().
    /// Всегда вызывается (и для fresh, и для recycled instances).
    fn prepare(
        &self,
        runtime: &Self::Runtime,
        ctx:     &dyn Ctx,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = (runtime, ctx);
        async { Ok(()) }
    }
}

pub enum BrokenCheck {
    /// Instance ок.
    Healthy,
    /// Instance сломан. Reason для diagnostics.
    Broken(Cow<'static, str>),
    /// Sync check недостаточен. Pool вызовет async Resource::check() перед выдачей.
    /// Для: Browser page (page crash не видна sync).
    NeedsAsyncCheck,
}

pub enum RecycleDecision {
    /// Вернуть в idle queue.
    Keep,
    /// Уничтожить. Pool создаст новый при необходимости.
    Drop,
}
```

**Пример: Postgres**

```rust
impl Pooled for Postgres {
    fn is_broken(&self, conn: &PgConnection) -> BrokenCheck {
        if conn.client.is_closed() { return BrokenCheck::Broken("TCP closed".into()); }
        if conn.conn_task.is_finished() { return BrokenCheck::Broken("conn task done".into()); }
        if conn.had_error.load(Ordering::Acquire) && conn.was_in_transaction.load(Ordering::Acquire) {
            return BrokenCheck::Broken("error in transaction".into());
        }
        BrokenCheck::Healthy
    }

    async fn recycle(&self, conn: &PgConnection, metrics: &InstanceMetrics)
        -> Result<RecycleDecision, PgError>
    {
        // Too many errors → untrusted, drop.
        if metrics.error_count >= 5 { return Ok(RecycleDecision::Drop); }
        // Connection alive?
        if conn.client.is_closed() { return Ok(RecycleDecision::Drop); }

        // Smart recycle: DISCARD ALL only if needed.
        if conn.was_in_transaction.load(Ordering::Acquire) || conn.had_error.load(Ordering::Acquire) {
            conn.client.simple_query("DISCARD ALL").await?;
        }

        conn.was_in_transaction.store(false, Ordering::Release);
        conn.had_error.store(false, Ordering::Release);
        Ok(RecycleDecision::Keep)
    }

    async fn prepare(&self, conn: &PgConnection, ctx: &dyn Ctx) -> Result<(), PgError> {
        if let Some(tenant) = ctx.ext::<TenantContext>() {
            let path = format!("{},public", tenant.schema);
            if conn.last_search_path.lock().unwrap().as_deref() != Some(&path) {
                conn.client.simple_query(&format!("SET search_path TO {}", path)).await?;
                *conn.last_search_path.lock().unwrap() = Some(path);
            }
            if let Some(ref role) = tenant.role {
                conn.client.simple_query(&format!("SET ROLE {}", role)).await?;
            }
        }
        Ok(())
    }
}
```

### Pool lifecycle flow

```
acquire:
  1. Try checkout from idle queue (LIFO/FIFO).
     a. If idle available → is_broken()? → test_on_checkout? check() → prepare(ctx) → return.
     b. If broken → destroy, try next idle.
  2. No idle → create() (respecting max_size, max_concurrent_creates).
     a. create() success → prepare(ctx) → return.
     b. create() failure → error to caller.
  3. Pool full → wait (with timeout from AcquireOptions.deadline).
  4. Max acquire attempts (default 3): if prepare() fails with retryable error →
     is_broken()? broken → destroy + retry. healthy + retryable → return to pool + retry.
     permanent error → fail fast.

release (Drop LeaseGuard → HandleInner::Guarded on_release):
  1. is_broken()? → if broken → destroy (via ReleaseQueue).
  2. Submit to ReleaseQueue for async processing:
     a. Framework policy: stale config (fingerprint) → destroy.
     b. Framework policy: max_lifetime exceeded → destroy.
     c. Resource recycle() → Keep → push to idle.
     d. Resource recycle() → Drop → destroy.

maintenance (periodic, if configured):
  - reap_idle: destroy instances idle > idle_timeout.
  - recycle_idle: recycle idle instances for stale state cleanup.
  - probe_idle: check() on idle instances.
```

### Pool Config

```rust
pub mod pool {
    pub struct Config {
        /// Minimum instances kept alive (warmup creates these eagerly).
        pub min_size: usize,
        /// Maximum instances. Acquire blocks when pool full.
        pub max_size: usize,
        /// LIFO: better cache locality, recent connections used first.
        /// FIFO: even distribution, all connections equally aged.
        pub strategy: Strategy,
        /// How to fill min_size at startup.
        pub warmup: WarmupStrategy,
        /// Destroy idle instances after this duration.
        pub idle_timeout: Duration,
        /// Destroy instances older than this (prevent memory leaks).
        pub max_lifetime: Duration,
        /// Run check() on checkout from idle.
        pub test_on_checkout: bool,
        /// Background maintenance loop.
        pub maintenance: Option<MaintenanceConfig>,
        /// Max parallel create() calls. Prevents thundering herd on empty pool.
        pub max_concurrent_creates: Option<usize>,
        /// Timeout for individual create() call. None = resource manages timeout.
        pub create_timeout: Option<Duration>,
        /// Parallel recycle workers in ReleaseQueue. Default 1 (fine for Postgres ~1ms recycle).
        /// Set higher for heavy recycle (Browser ~500ms).
        pub recycle_workers: usize,
        /// When to run check().
        pub check_policy: CheckPolicy,
        /// Max attempts for checkout + prepare cycle. Default: 3.
        pub max_acquire_attempts: u32,
    }

    pub enum Strategy { Lifo, Fifo }

    pub enum WarmupStrategy {
        /// No warmup. First acquire triggers first create.
        None,
        /// Create min_size sequentially at startup.
        Sequential,
        /// Create min_size in parallel (up to concurrency).
        Parallel { concurrency: usize },
        /// Create one every `delay`. Avoids thundering herd on target server.
        Staggered { delay: Duration },
    }

    pub struct MaintenanceConfig {
        /// How often to run maintenance.
        pub interval: Duration,
        /// Run check() on idle instances.
        pub probe_idle: bool,
        /// Destroy instances idle > idle_timeout.
        pub reap_idle: bool,
        /// Run recycle() on idle instances (clear stale state).
        pub recycle_idle: bool,
    }

    pub enum CheckPolicy {
        /// Never check. Trust is_broken() only.
        Never,
        /// Check on every acquire from idle.
        OnAcquire,
        /// Check at interval. Between checks — cached result.
        Interval(Duration),
    }
}
```

---

## Resident

Один Clone-able handle. Acquire = clone. Zero contention.

```rust
pub trait Resident: Resource
where
    Self::Lease: Clone,
{
    /// Sync O(1) liveness check для shared handle.
    ///
    /// КОНТРАКТ: НЕТ I/O, НЕТ блокирующих операций. O(1) atomic/flag check ТОЛЬКО.
    /// Вызывается из async context — blocking здесь засоряет tokio thread pool.
    ///
    /// reqwest::Client: всегда true (stateless, нет state to check).
    /// fred::Client: client.is_connected() (атомарный флаг, O(1)).
    ///
    /// Исправлено (#23): переименовано is_alive → is_alive_sync, явный контракт.
    /// Для I/O-based проверок (Kafka metadata fetch) используй Resource::check() async.
    ///
    /// Default: true.
    fn is_alive_sync(&self, _runtime: &Self::Runtime) -> bool { true }

    /// Интервал проверки. None = никогда (stateless clients).
    ///
    /// reqwest::Client: None (не проверять).
    /// Redis Shared: Some(15s) (atomic connectivity flag).
    ///
    /// Framework вызывает is_alive_sync() с этим интервалом.
    /// Если is_alive_sync() = false → recovery (destroy + recreate).
    fn stale_after(&self) -> Option<Duration> { None }
}
```

**Пример: Redis Shared (sync check)**

```rust
impl Resident for RedisShared {
    // O(1) — проверяет атомарный connected flag (нет I/O).
    fn is_alive_sync(&self, client: &fred::Client) -> bool {
        client.is_connected()
    }

    fn stale_after(&self) -> Option<Duration> {
        Some(Duration::from_secs(15))
    }
}
```

**Пример: Kafka Producer (I/O check → Resource::check)**

```rust
// Kafka fetch_metadata = network round-trip → нельзя в is_alive_sync.
// Используем async Resource::check() вместо:
impl Resource for KafkaProducer {
    async fn check(&self, runtime: &FutureProducer) -> Result<HealthStatus, Error> {
        match runtime.client().fetch_metadata(None, Duration::from_secs(2)) {
            Ok(_) => Ok(HealthStatus::Healthy),
            Err(e) => Ok(HealthStatus::Degraded(e.to_string())),
        }
    }
}
```

**Resident lifecycle:**

```
register:
  1. If eager_create → create() immediately.
  2. Store in Cell<T> (ArcSwap-based).

acquire:
  1. Cell::load() → Arc<Runtime>.
  2. Clone → R::Lease (since Lease: Clone).
  3. Return HandleInner::Owned(lease). Zero contention — ArcSwap load.

is_alive_sync check (if stale_after configured):
  Background task every stale_after:
    1. is_alive_sync(runtime)?  // O(1), no I/O — contract enforced by rename (#23)
    2. If false → shutdown(runtime) → destroy(runtime) → create(config, ctx) → Cell::store(new).
    3. Existing clones у callers: видят старый Arc, но продолжают работать.
       Новые acquire получают fresh runtime.
```

---

## Service

Long-running runtime + token-based access. Token = `Self::Lease` (единый source of truth).

```rust
pub trait Service: Resource {
    /// Cloned: Token cheap clone, release = noop. HandleInner::Owned.
    ///   Telegram Bot: TelegramBotHandle (Bot.clone() + broadcast.subscribe()).
    ///   WebSocket outbound: WsHandle (mpsc::Sender clone).
    ///
    /// Tracked: Token = tracked resource, release обязателен. HandleInner::Guarded.
    ///   Rate-limited API: permit from semaphore.
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;

    /// Создать token (Self::Lease) для caller-а.
    fn acquire_token(
        &self,
        runtime: &Self::Runtime,
        ctx: &dyn Ctx,
    ) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;

    /// Вернуть token. Для Cloned — noop (drop).
    fn release_token(
        &self,
        runtime: &Self::Runtime,
        token: Self::Lease,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = (runtime, token);
        async { Ok(()) }
    }
}

pub enum TokenMode {
    /// Token = cheap clone. HandleInner::Owned. release_token = noop.
    Cloned,
    /// Token = tracked resource. HandleInner::Guarded. release_token called via ReleaseQueue.
    Tracked,
}
```

**Пример: Telegram Bot**

```rust
impl Service for TelegramBot {
    const TOKEN_MODE: TokenMode = TokenMode::Cloned;

    async fn acquire_token(&self, runtime: &TelegramBotRuntime, _ctx: &dyn Ctx)
        -> Result<TelegramBotHandle, TelegramError>
    {
        Ok(TelegramBotHandle {
            bot:       runtime.inner.bot.clone(),
            update_rx: runtime.inner.update_tx.subscribe(),
            info:      Arc::clone(&runtime.inner.info),
        })
    }

    // release_token: default noop (Cloned mode).
}
```

---

## Transport

Один transport + N мультиплексированных sessions. Session = `Self::Lease`.

```rust
pub trait Transport: Resource {
    /// Открыть session (Self::Lease) на transport.
    /// SSH: spawn child process через multiplexed SSH connection.
    fn open_session(
        &self,
        transport: &Self::Runtime,
        ctx: &dyn Ctx,
    ) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;

    /// Закрыть session. healthy = не было ошибок.
    /// Called async via ReleaseQueue (not in Drop).
    fn close_session(
        &self,
        transport: &Self::Runtime,
        session: Self::Lease,
        healthy: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = (transport, session, healthy);
        async { Ok(()) }
    }

    /// Keepalive для transport connection.
    fn keepalive(
        &self,
        transport: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = transport;
        async { Ok(()) }
    }
}
```

**Transport config:**

```rust
/// Исправлено (#21): явный лимит на concurrent sessions через Semaphore.
/// Без лимита open_session вызывается неограниченно → OOM / SSH server отказывает.
pub struct transport::Config {
    /// Максимум одновременных sessions (Semaphore permits).
    /// SSH: ограничен MaxSessions в sshd_config (обычно 10). Default: 8.
    /// None = unlimited (осторожно: только для протоколов без server-side limit).
    pub max_sessions:              Option<u32>,
    /// Таймаут на получение session slot при достижении max_sessions.
    pub session_acquire_timeout:   Duration,
}

impl Default for transport::Config {
    fn default() -> Self {
        Self {
            max_sessions:            Some(8),
            session_acquire_timeout: Duration::from_secs(30),
        }
    }
}
```

**Transport acquire flow (с session limit):**

```
acquire:
  1. If max_sessions configured:
     a. semaphore.acquire_owned().timeout(session_acquire_timeout).await
        → Err(Timeout) if no slots available.
     b. Permit held until session released.
  2. resource.open_session(transport, ctx).await → Lease.
  3. Return HandleInner::Guarded { value: session, on_release: close_fn + drop(permit) }.

release (Drop HandleInner::Guarded → on_release):
  1. Submit to ReleaseQueue (sync, with permit moved into closure).
  2. ReleaseQueue worker: close_session(transport, session, healthy).await.
  3. drop(permit) → semaphore permit released → next acquire unblocked.
  Note: permit released only AFTER close_session completes.
```

**Пример: SSH**

```rust
impl Transport for Ssh {
    async fn open_session(&self, transport: &SshRuntime, _ctx: &dyn Ctx) -> Result<SshSession, SshError> {
        let child = transport.session.command("bash").spawn().await?;
        Ok(SshSession { child, opened_at: Instant::now() })
    }

    async fn close_session(&self, _transport: &SshRuntime, session: SshSession, _healthy: bool) -> Result<(), SshError> {
        drop(session.child);
        Ok(())
    }

    async fn keepalive(&self, transport: &SshRuntime) -> Result<(), SshError> {
        transport.session.check().await.map_err(SshError::KeepaliveFailed)
    }
}
```

---

## Exclusive

Один owner за раз. `Arc<Semaphore>(1)` + `OwnedSemaphorePermit`.

```rust
pub trait Exclusive: Resource {
    /// Reset state между lease-ами. Called by framework in release path.
    /// Caller не вызывает — Drop LeaseGuard → framework reset → permit release.
    ///
    /// Kafka Consumer: commit offsets.
    /// Serial port: flush buffers.
    ///
    /// If reset() fails → destroy + recreate.
    fn reset(
        &self,
        runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        let _ = runtime;
        async { Ok(()) }
    }
}
```

**Пример: Kafka Consumer**

```rust
impl Exclusive for KafkaConsumer {
    async fn reset(&self, consumer: &StreamConsumer) -> Result<(), KafkaError> {
        consumer.commit_consumer_state(CommitMode::Sync)
            .map_err(KafkaError::CommitFailed)?;
        Ok(())
    }
}
```

**Exclusive acquire flow:**

```
acquire:
  1. Arc::clone(&semaphore).acquire_owned() → OwnedSemaphorePermit.
  2. Return HandleInner::Shared { value: Arc<R::Lease>, on_release }.
  3. Caller sees &R::Lease through Deref on Arc.

release (Drop HandleInner::Shared → on_release):
  1. Submit to ReleaseQueue (sync).
  2. ReleaseQueue worker (async):
     a. If tainted → destroy + recreate.
     b. If healthy → reset(runtime).
     c. If reset fails → destroy + recreate.
     d. drop(permit) → semaphore permit released → next caller unblocked.
  Note: permit released AFTER reset. Next caller gets clean state.
```

---

## EventSource

Incoming event stream. Subscribe/recv split — persistent subscription.

```rust
pub trait EventSource: Resource {
    /// Тип event-а. Clone для broadcast к multiple subscribers.
    type Event: Send + Clone + 'static;

    /// Persistent subscription handle. Held by engine, not exposed to callers.
    type Subscription: Send + 'static;

    /// Create persistent subscription. Called once by engine.
    fn subscribe(
        &self,
        runtime: &Self::Runtime,
        ctx: &dyn Ctx,
    ) -> impl Future<Output = Result<Self::Subscription, Self::Error>> + Send;

    /// Receive next event from existing subscription. Blocks until event.
    fn recv(
        &self,
        subscription: &mut Self::Subscription,
    ) -> impl Future<Output = Result<Self::Event, Self::Error>> + Send;
}
```

**Key insight:** Action authors never call subscribe/recv directly. EventSource is internal engine concern for EventTrigger loop. Trigger listens, Action acts. These paths never cross.

**Пример: Redis Pub/Sub**

```rust
impl EventSource for RedisSubscriber {
    type Event = PubSubMessage;
    type Subscription = broadcast::Receiver<PubSubMessage>;

    async fn subscribe(
        &self,
        runtime: &RedisPubSubRuntime,
        _ctx: &dyn Ctx,
    ) -> Result<broadcast::Receiver<PubSubMessage>, RedisError> {
        Ok(runtime.message_tx.subscribe())
    }

    async fn recv(
        &self,
        sub: &mut broadcast::Receiver<PubSubMessage>,
    ) -> Result<PubSubMessage, RedisError> {
        sub.recv().await.map_err(|_| RedisError::SubscriptionClosed)
    }
}
```

---

## Daemon

Background process. Lifecycle only — no acquire/release. Framework manages start/stop/restart.

```rust
pub trait Daemon: Resource {
    /// Запустить daemon. Возвращает когда daemon завершится (или cancel).
    ///
    /// Framework: spawn в отдельный task.
    /// cancel = single source of truth for lifecycle (framework-owned).
    /// Runtime stores NO cancel — lifecycle is framework concern.
    ///
    /// Two stop scenarios:
    ///   1. Framework wants stop → cancel.cancel() → run() returns Ok(()).
    ///   2. Unrecoverable error → run() returns Err(...) → framework decides:
    ///      restart via RecoveryGate or permanent failure.
    fn run(
        &self,
        runtime: &Self::Runtime,
        ctx:     &dyn Ctx,
        cancel:  CancellationToken,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
```

**Пример: Telegram Bot polling loop**

```rust
impl Daemon for TelegramBot {
    async fn run(
        &self,
        runtime: &TelegramBotRuntime,
        _ctx: &dyn Ctx,
        cancel: CancellationToken,
    ) -> Result<(), TelegramError> {
        let mut offset = 0;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                result = runtime.inner.bot.get_updates().offset(offset).send() => {
                    match result {
                        Ok(updates) => {
                            for u in updates {
                                let _ = runtime.inner.update_tx.send(u.clone());
                                offset = u.id + 1;
                            }
                        }
                        Err(e) if e.is_permanent() => return Err(e.into()),
                        Err(_) => {
                            // Transient — backoff, continue.
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                }
            }
        }
    }
}
```

**Daemon semantics:**
- `create()` = setup infrastructure (bot client, channels). NO polling loop.
- `Daemon::run()` = behaviour (polling loop). Framework spawns this.
- On crash → framework restart loop (see below).
- On max retries → `destroy()` + `create()` → new runtime → `run()` again.

### RestartPolicy — framework-owned restart loop

Framework owns the restart loop in `daemon::Runtime`. Resource authors never implement restart logic.

```rust
/// Configured via daemon::Config at registration time.
pub enum RestartPolicy {
    /// Never restart. If run() returns Err → permanent failure.
    Never,
    /// Restart on failure only. If run() returns Ok → stopped.
    OnFailure {
        max_restarts: u32,          // 0 = unlimited
        backoff: BackoffConfig,     // exponential with jitter
    },
    /// Always restart (unless cancelled). Even Ok(()) triggers restart.
    /// Use case: daemon that does periodic work and returns Ok between cycles.
    Always {
        max_restarts: u32,
        backoff: BackoffConfig,
    },
}

pub struct BackoffConfig {
    pub initial: Duration,          // e.g. 1s
    pub max: Duration,              // e.g. 60s
    pub multiplier: f64,            // e.g. 2.0
}

/// Лимиты на полное пересоздание daemon runtime (destroy + create).
///
/// Нужен потому что стандартный цикл restarts сбрасывает `consecutive_failures = 0`
/// после recreate. Без бюджета: если create() успешен но run() всегда падает →
/// бесконечный recreate цикл навсегда.
pub struct RecreateBudget {
    /// Максимум recreates за весь lifetime daemon. Default: 3.
    /// После исчерпания → permanent failure.
    pub max_total: u32,
    /// Максимум recreates в скользящем окне. Default: 2 за 10 минут.
    /// Предотвращает rapid create-crash-recreate loop.
    pub max_per_window: u32,
    pub window:         Duration,
    /// Backoff между recreates (отдельно от run() restart backoff). Default: 30s→5min×2.
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

impl Default for RestartPolicy {
    fn default() -> Self {
        RestartPolicy::OnFailure {
            max_restarts: 5,
            backoff: BackoffConfig {
                initial: Duration::from_secs(1),
                max: Duration::from_secs(60),
                multiplier: 2.0,
            },
        }
    }
}
```

**Framework restart loop (in `daemon::Runtime`):**

Runtime state использует `Option<R::Runtime>`. `None` означает "слот временно пуст во время recreate".

```rust
/// Исправлено (#15): Option<R::Runtime> вместо mem::zeroed() UB.
/// mem::zeroed() для типов с указателями (Arc, JoinHandle) = UB при drop.
struct DaemonState<R: Resource> {
    runtime:              Option<R::Runtime>, // None только внутри try_recreate
    consecutive_failures: u32,
    total_recreates:      u32,
    window_recreates:     u32,
    window_started:       Option<Instant>,
}
```

```
daemon::Runtime::start(resource, config, credential, ctx, cancel, restart_policy, recreate_budget):
  state = DaemonState {
    runtime:              Some(initial_runtime),  // создан до start()
    consecutive_failures: 0,
    total_recreates:      0,
    window_recreates:     0,
    window_started:       None,
  }
  loop:
    runtime_ref = state.runtime.as_ref()
      .expect("daemon runtime slot empty — framework bug")
    result = resource.run(runtime_ref, &ctx, cancel.child_token()).await

    if cancel.is_cancelled() → break (framework shutdown)

    match (result, restart_policy):
      (Ok(()), Never)       → break (clean stop)
      (Ok(()), OnFailure)   → break (clean stop)
      (Ok(()), Always)      → state.consecutive_failures = 0; continue
      (Err(e), Never)       → emit DaemonFailed event; break
      (Err(e), OnFailure { max_restarts, backoff } | Always { max_restarts, backoff }):
        state.consecutive_failures += 1
        if max_restarts > 0 && state.consecutive_failures > max_restarts:
          // Исчерпали restarts с ОДНИМ runtime → пробуем recreate.
          match try_recreate(&mut state, resource, config, credential, ctx, &recreate_budget).await {
            Ok(()) →
              state.consecutive_failures = 0
              state.total_recreates += 1; state.window_recreates += 1
              continue
            Err(budget_exceeded) →
              emit DaemonFailed(budget_exceeded); break
          }
        else:
          delay = backoff.delay_for(state.consecutive_failures) // ±25% jitter
          sleep(delay); continue

async try_recreate(state, resource, config, credential, ctx, budget):
  // 1. Global budget.
  if state.total_recreates >= budget.max_total → Err("exceeded max_total recreates")
  // 2. Windowed rate limit.
  now = Instant::now()
  if state.window_started within budget.window:
    if state.window_recreates >= budget.max_per_window → Err("recreate rate exceeded")
  else:
    state.window_started = now; state.window_recreates = 0  // reset window
  // 3. Recreate backoff (отдельный от run() backoff).
  delay = budget.recreate_backoff.delay_for(state.total_recreates + 1)
  sleep(delay)
  // 4. Take old runtime from slot (slot → None во время recreate).
  //    Исправлено (#15): take() вместо mem::zeroed() — no UB.
  old = state.runtime.take()
    .ok_or_else(|| Error::permanent("daemon runtime slot empty — inconsistent state"))?
  resource.shutdown(&old).await.ok()
  resource.destroy(old).await.ok()
  // 5. Create new runtime. If create fails → permanent failure (caller breaks loop).
  new = resource.create(config, credential, ctx).await?
  state.runtime = Some(new)  // restore slot
  Ok(())
```

**Key insights:**
- Restart loop retries `run()` с ОДНИМ runtime (cheap). Только после `max_restarts`
  exhausted — `destroy() + create()` нового runtime.
- `Option<R::Runtime>` в DaemonState: `None` только в узком окне внутри `try_recreate`.
  Снаружи — всегда `Some`. Паника при `None` в основном цикле = framework bug.
- `RecreateBudget` предотвращает infinite recreate loop если `create()` успешен но
  `run()` стабильно падает.
- `daemon::Config` включает оба: `restart_policy` + `recreate_budget`.
- Если `create()` при recreate падает → немедленный permanent failure (нет смысла retry).

---

## Trait compatibility map

Resource может impl несколько topology traits (гибриды). Primary topology определяет acquire semantics. Secondary добавляют capabilities через `.also_event_source()`, `.also_daemon()` на builder.

```
                     PG   Redis-S  Redis-D  Redis-PS  Kafka-P  Kafka-C  SSH   WS    TG-Bot  Browser  LLM   HTTP
Resource             ✓    ✓       ✓       ✓         ✓       ✓        ✓    ✓    ✓       ✓       ✓     ✓
Pooled               ✓    ·       ✓       ·         ·       ·        ·    ·    ·       ✓       ·     ·
  prepare()          ✓    ·       ✓       ·         ·       ·        ·    ·    ·       ✓       ·     ·
  recycle()          ✓    ·       ✓       ·         ·       ·        ·    ·    ·       ✓       ·     ·
Resident             ·    ✓       ·       ·         ✓       ·        ·    ·    ·       ·       ✓     ✓
Service              ·    ·       ·       ·         ·       ·        ·    ✓    ✓       ·       ·     ·
Transport            ·    ·       ·       ·         ·       ·        ✓    ·    ·       ·       ·     ·
Exclusive            ·    ·       ·       ·         ·       ✓        ·    ·    ·       ·       ·     ·
EventSource          ·    ·       ·       ✓         ·       ·        ·    ✓*   ✓*      ·       ·     ·
Daemon               ·    ·       ·       ·         ·       ·        ·    ·    ✓       ·       ·     ·

* WS и Telegram — гибриды: Service (primary) + EventSource + Daemon (secondary).
  Registration: .service(cfg).also_event_source(es_cfg).also_daemon(d_cfg).build()
```

### Lease mapping per topology

```
Topology     | Lease relationship   | HandleInner variant
-------------|----------------------|--------------------
Pool         | Lease = Runtime      | Guarded (owned + on_release callback)
Resident     | Lease = Runtime      | Owned (clone, no callback)
Service(C)   | Lease = Token        | Owned (clone, no callback)
Service(T)   | Lease = Token        | Guarded (owned + on_release callback)
Transport    | Lease = Session      | Guarded (owned + on_close callback via ReleaseQueue)
Exclusive    | Lease = Runtime      | Shared (Arc + on_release callback via ReleaseQueue)
EventSource  | (not via acquire)    | —
Daemon       | (no acquire)         | —
```
