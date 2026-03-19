# 04 — Recovery, Extensions, Resilience

---

## RecoveryGate — thundering herd prevention

Один probe, остальные ждут. CAS-based state machine.

```rust
/// Исправлено (#17): state и notify обёрнуты в Arc для clone в make_waiter().
/// RecoveryWaiter теперь хранит Arc-клоны — 'static + Send, совместим с tokio::spawn.
pub struct RecoveryGate {
    state:                  Arc<ArcSwap<GateState>>,
    notify:                 Arc<Notify>,
    /// Максимум recovery attempts до PermanentlyFailed. Default: 10.
    max_recovery_attempts:  u32,
}

/// GateState — добавлено `attempt` поле в Failed для escalation tracking.
enum GateState {
    /// Ресурс ок. Нет recovery.
    Idle,
    /// Recovery in progress. Другие acquire ждут notify.
    InProgress { attempt: u32, started: Instant },
    /// Transient failure. Retry через backoff (с ±25% jitter).
    Failed { error: Arc<crate::Error>, until: Instant, attempt: u32 },
    /// Permanent failure. Не retry.
    PermanentlyFailed { error: Arc<crate::Error> },
}

impl RecoveryGate {
    pub fn new() -> Self {
        Self {
            state:                 Arc::new(ArcSwap::from_pointee(GateState::Idle)),
            notify:                Arc::new(Notify::new()),
            max_recovery_attempts: 10,
        }
    }

    /// Попробовать начать recovery. CAS loop: Idle/Failed(expired) → InProgress.
    ///
    /// Ok(ticket) — мы первые, можно начать recovery probe.
    /// Err(waiter) — кто-то уже recovers, или backend permanently failed.
    ///               Caller ждёт waiter.wait() или returns error.
    ///
    /// `self: &Arc<Self>` чтобы создать Arc-клон для RecoveryTicket Drop guard.
    /// ManagedResource хранит `gate: Arc<RecoveryGate>` и вызывает `gate.try_begin()`.
    pub fn try_begin(self: &Arc<Self>) -> Result<RecoveryTicket, RecoveryWaiter> {
        loop {
            let current = self.state.load();

            match current.as_ref() {
                GateState::Idle => {
                    let next = Arc::new(GateState::InProgress {
                        attempt: 1,
                        started: Instant::now(),
                    });
                    // CAS: succeed only if state still == current ptr.
                    let prev = self.state.compare_and_swap(&current, next);
                    if Arc::ptr_eq(&prev, &current) {
                        return Ok(RecoveryTicket {
                            attempt:  1,
                            gate:     Arc::clone(self),
                            resolved: false,
                            _private: (),
                        });
                    }
                    // Lost race — retry CAS loop.
                    continue;
                }

                GateState::InProgress { .. } => {
                    // Кто-то уже recovers.
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
                        // Слишком много попыток → escalate to PermanentlyFailed.
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
                        return Ok(RecoveryTicket {
                            attempt:  new_attempt,
                            gate:     Arc::clone(self),
                            resolved: false,
                            _private: (),
                        });
                    }
                    continue;
                }

                GateState::PermanentlyFailed { .. } => {
                    return Err(self.make_waiter());
                }
            }
        }
    }

    /// Recovery успешна.
    pub fn resolve(&self, mut ticket: RecoveryTicket) {
        ticket.resolved = true; // disarm Drop guard
        self.state.store(Arc::new(GateState::Idle));
        self.notify.notify_waiters();
    }

    /// Recovery failed. Transient — retry с backoff.
    pub fn fail_transient(&self, mut ticket: RecoveryTicket, error: crate::Error, backoff: Duration) {
        ticket.resolved = true; // mark before drop to prevent Drop guard from firing
        self.fail_transient_internal_with_attempt(error, backoff, ticket.attempt);
    }

    /// Internal: fail without consuming ticket (for Drop guard).
    pub(crate) fn fail_transient_internal(&self, error: crate::Error, backoff: Duration) {
        // Attempt unknown here (ticket is mid-drop). Use current state attempt.
        let attempt = match self.state.load().as_ref() {
            GateState::InProgress { attempt, .. } => *attempt,
            _ => 0,
        };
        self.fail_transient_internal_with_attempt(error, backoff, attempt);
    }

    fn fail_transient_internal_with_attempt(&self, error: crate::Error, backoff: Duration, attempt: u32) {
        let jitter = backoff.mul_f64(0.75 + rand::random::<f64>() * 0.5); // ±25%
        let until  = Instant::now() + jitter;
        self.state.store(Arc::new(GateState::Failed {
            error:   Arc::new(error),
            until,
            attempt,
        }));
        self.notify.notify_waiters();
    }

    /// Recovery failed. Permanent — не retry.
    pub fn fail_permanent(&self, mut ticket: RecoveryTicket, error: crate::Error) {
        ticket.resolved = true; // disarm Drop guard
        self.state.store(Arc::new(GateState::PermanentlyFailed { error: Arc::new(error) }));
        self.notify.notify_waiters();
    }

    /// Текущее состояние.
    pub fn status(&self) -> GateStatus {
        match self.state.load().as_ref() {
            GateState::Idle                       => GateStatus::Healthy,
            GateState::InProgress { attempt, .. } => GateStatus::Recovering { attempt: *attempt },
            GateState::Failed { until, .. }       => GateStatus::Failed {
                retry_in: until.saturating_duration_since(Instant::now()),
            },
            GateState::PermanentlyFailed { .. }   => GateStatus::PermanentlyFailed,
        }
    }

    /// Returns an owned waiter — 'static + Send, compatible with tokio::spawn.
    fn make_waiter(&self) -> RecoveryWaiter {
        RecoveryWaiter {
            state:  Arc::clone(&self.state),
            notify: Arc::clone(&self.notify),
        }
    }
}

/// Исправлено (#22): добавлен Drop guard. Если ticket дропнут без resolve/fail →
/// gate навсегда stuck в InProgress. Все acquire зависают на wait() вечно.
/// Drop guard: debug_assert + auto fail_transient с коротким backoff.
pub struct RecoveryTicket {
    pub(crate) attempt: u32,
    /// Arc к gate для auto-fail в Drop.
    gate:     Arc<RecoveryGate>,
    resolved: bool,
    _private: (),
}

impl Drop for RecoveryTicket {
    fn drop(&mut self) {
        if !self.resolved {
            debug_assert!(
                false,
                "RecoveryTicket dropped without resolve/fail — framework bug. \
                 Gate will be stuck in InProgress forever."
            );
            // Auto-fail с коротким backoff чтобы разблокировать waiters.
            self.gate.fail_transient_internal(
                crate::Error::permanent("recovery ticket dropped without completion"),
                Duration::from_secs(5),
            );
        }
    }
}

/// Owned waiter — 'static + Send, compatible with tokio::spawn.
///
/// Исправлено (#17): предыдущая RecoveryWaiter<'a> хранила borrowed refs.
/// Future с borrowed refs не 'static → несовместим с tokio::spawn.
/// Теперь хранит Arc-клоны из RecoveryGate.
pub struct RecoveryWaiter {
    state:  Arc<ArcSwap<GateState>>,
    notify: Arc<Notify>,
}

impl RecoveryWaiter {
    /// Ждать до recovery или permanent failure.
    /// Ok(()) = recovered (Idle state).
    /// Err = permanent failure или backoff истёк (caller retries acquire).
    pub async fn wait(&self) -> Result<(), Arc<crate::Error>> {
        loop {
            match self.state.load().as_ref() {
                GateState::Idle => return Ok(()),
                GateState::PermanentlyFailed { error } => return Err(Arc::clone(error)),
                GateState::Failed { until, error, .. } => {
                    if Instant::now() >= *until {
                        // Backoff expired — caller retries acquire (will trigger new probe).
                        return Err(Arc::clone(error));
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

pub enum GateStatus {
    Healthy,
    Recovering { attempt: u32 },
    Failed { retry_in: Duration },
    PermanentlyFailed,
}
```

---

## RecoveryGroup — shared gate for same backend

Postgres primary → N pools. Если primary down — один recovery probe, все pools ждут.

```rust
pub struct RecoveryGroupKey(String);

impl RecoveryGroupKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }
}

pub struct RecoveryGroup {
    pub key:  RecoveryGroupKey,
    pub gate: Arc<RecoveryGate>,
}

/// Global registry of recovery groups.
pub struct RecoveryGroupRegistry {
    groups: DashMap<String, Arc<RecoveryGate>>,
}

impl RecoveryGroupRegistry {
    /// Получить или создать gate для группы.
    pub fn get_or_create(&self, key: &RecoveryGroupKey) -> Arc<RecoveryGate> {
        self.groups
            .entry(key.0.clone())
            .or_insert_with(|| Arc::new(RecoveryGate::new()))
            .clone()
    }
}
```

Регистрация:

```rust
manager.register(Postgres)
    .config(pg_config)
    .id(resource_id)
    .recovery_group(RecoveryGroupKey::new("pg-primary"))  // shared gate
    .pool(pool_config)
    .build().await?;
```

---

## WatchdogHandle — opt-in background probe

```rust
pub struct WatchdogHandle {
    task:   JoinHandle<()>,
    cancel: CancellationToken,
}

pub struct WatchdogConfig {
    /// Interval между probes.
    pub interval: Duration,
    /// Timeout на одну probe.
    pub probe_timeout: Duration,
    /// Сколько consecutive failures до recovery trigger.
    pub failure_threshold: u32,
    /// Сколько consecutive successes до "recovered".
    pub recovery_threshold: u32,
    /// Backoff при consecutive failures. Reuses BackoffConfig from daemon (02-topology).
    pub backoff: BackoffConfig,
    /// Auto-trigger recovery при failure_threshold breach.
    pub auto_recover: bool,
}

impl WatchdogHandle {
    pub fn spawn<R: Resource>(
        resource: Arc<R>,
        runtime: Arc<R::Runtime>,
        config: WatchdogConfig,
        gate: Arc<RecoveryGate>,
    ) -> Self {
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        let task = tokio::spawn(async move {
            let mut consecutive_failures = 0u32;
            let mut consecutive_successes = 0u32;

            loop {
                tokio::select! {
                    _ = cancel_clone.cancelled() => break,
                    _ = tokio::time::sleep(config.interval) => {
                        let result = tokio::time::timeout(
                            config.probe_timeout,
                            resource.check(&runtime),
                        ).await;

                        match result {
                            Ok(Ok(_)) => {
                                consecutive_failures = 0;
                                consecutive_successes += 1;
                            }
                            _ => {
                                consecutive_successes = 0;
                                consecutive_failures += 1;

                                // Исправлено (#22): реализован recovery logic.
                                // Ticket ВСЕГДА завершается через resolve() или fail_transient().
                                // Drop guard на RecoveryTicket auto-fails если забыли.
                                if config.auto_recover && consecutive_failures >= config.failure_threshold {
                                    if let Ok(ticket) = gate.try_begin() {
                                        let resource_  = Arc::clone(&resource);
                                        let runtime_   = Arc::clone(&runtime);
                                        let gate_      = Arc::clone(&gate);
                                        let probe_t    = config.probe_timeout;
                                        let backoff_t  = config.backoff.initial;

                                        tokio::spawn(async move {
                                            let result = tokio::time::timeout(
                                                probe_t * 3,
                                                do_recovery(&resource_, &runtime_),
                                            ).await;

                                            match result {
                                                Ok(Ok(())) => gate_.resolve(ticket),
                                                Ok(Err(e)) => gate_.fail_transient(
                                                    ticket, e, backoff_t,
                                                ),
                                                Err(_) => gate_.fail_transient(
                                                    ticket,
                                                    crate::Error::transient("recovery probe timeout"),
                                                    backoff_t,
                                                ),
                                            }
                                        });
                                        consecutive_failures = 0; // reset counter
                                    }
                                    // else: gate already InProgress (другой caller recovers)
                                }
                            }
                        }
                    }
                }
            }
        });

        Self { task, cancel }
    }

    pub async fn shutdown(self) {
        self.cancel.cancel();
        let _ = self.task.await;
    }
}

/// Recovery probe: liveness check через Resource::check().
/// Для более сложного recovery (recreate) — topology-specific logic вместо этого helper.
async fn do_recovery<R: Resource>(
    resource: &R,
    runtime:  &R::Runtime,
) -> Result<(), crate::Error> {
    resource.check(runtime).await.map_err(Into::into)
}
```

**Когда Watchdog нужен:**
- Service (Telegram Bot polling): мониторить что polling loop жив.
- Transport (SSH): мониторить что connection жива + keepalive.

**Когда Watchdog НЕ нужен:**
- Pool с test_on_checkout: check() при каждом checkout.
- Resident с stale_after: is_alive() периодически.
- HTTP client: stateless, всегда "жив".

---

## ConnectionAware — disconnect detection (v2, deferred)

> **Deferred to v2.** Not needed for core topologies. Requires runtime integration
> with each topology's reconnect flow. Will be designed after v1 is stable.

```rust
pub trait ConnectionAware: Resource {
    /// Как обнаружить disconnect. Framework вызывает один раз при create().
    fn disconnect_signal(&self, runtime: &Self::Runtime) -> DisconnectSignal<Self::Error>;
}

pub enum DisconnectSignal<E> {
    /// Native event. Framework awaits this future.
    /// WebSocket: background loop sends on oneshot when connection drops.
    /// fred.rs: error_rx stream bridges to future.
    Watch(Pin<Box<dyn Future<Output = E> + Send>>),

    /// watch::Receiver — multiple subscribers.
    /// Telegram: CancellationToken bridge.
    Channel(watch::Receiver<Option<E>>),

    /// Нет native signal. Framework falls back to periodic check().
    /// SSH (openssh has no disconnect callback), generic TCP.
    PollFallback,
}
```

---

## InfraProvider — nested lifecycle (v2, deferred)

> **Deferred to v2.** Complex nested lifecycle (Browser process → Pages pool).
> V1 handles this via custom `create()` logic in the resource impl.
> Formal InfraProvider trait will be designed when we have real use cases in v1.

Для ресурсов с parent infrastructure. Browser (process) → Pages (pool instances).

```rust
pub trait InfraProvider: Resource {
    /// Parent infrastructure type.
    type Infra: Send + Sync + 'static;

    /// Get or create shared infrastructure.
    /// Browser: spawn chromium process (once, shared by all pages).
    fn get_or_create_infra(
        &self,
        config: &Self::Config,
    ) -> impl Future<Output = Result<Arc<Self::Infra>, Self::Error>> + Send;

    /// Check infrastructure alive.
    /// Browser: process alive? WebSocket to DevTools connected?
    fn check_infra(
        &self,
        infra: &Self::Infra,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Create Runtime instance WITHIN infrastructure.
    /// Browser: create new Page in existing browser process.
    fn create_in_infra(
        &self,
        infra:  &Self::Infra,
        config: &Self::Config,
        ctx:    &dyn Ctx,
    ) -> impl Future<Output = Result<Self::Runtime, Self::Error>> + Send;
}
```

Framework при InfraProvider:
1. `get_or_create_infra()` при registration → store Arc<Infra>.
2. `create_in_infra()` вместо `Resource::create()` для pool instances.
3. `check_infra()` periodically.
4. Если infra dead → mark ALL runtime instances broken → recreate infra → refill pool.

---

## Resilience — use `nebula-resilience`, don't reimplement

**Решение:** `nebula-resource` НЕ содержит собственных CircuitBreaker, RateLimiter, RetryPolicy, BulkheadSemaphore. Всё это уже есть в `nebula-resilience` (const-generic CircuitBreaker, RetryStrategy, RateLimiter trait impls, Bulkhead, LayerBuilder → ResilienceChain). Используем напрямую.

### RecoveryGate vs CircuitBreaker — разные задачи

| | RecoveryGate | CircuitBreaker |
|---|---|---|
| **Scope** | Весь backend (shared gate) | Одна операция / один resource |
| **Семантика** | "Backend down, один probe recovers" | "Слишком много failures, stop calling" |
| **State machine** | Idle → InProgress → Failed/Resolved | Closed → Open → HalfOpen → Closed |
| **Координация** | Один winner пробует, остальные ждут | Каждый caller решает сам |

RecoveryGate — infrastructure-level координация (plan 04, оставляем).
CircuitBreaker — operation-level protection (из `nebula-resilience`).
Они дополняют друг друга:

```
acquire() path — strict ordering:
  1. RecoveryGate: backend alive? (shared probe, thundering herd prevention)
     - If InProgress → wait for probe result (no retry, just wait).
     - If Failed → return error immediately (with retry_after hint).
     - If PermanentlyFailed → return error, no retry.
     - If Idle → proceed to step 2.
  2. AcquireResilience (on ManagedResource): timeout → retry → circuit breaker
     - Wraps the actual acquire operation (step 3).
     - Retry only retries step 3, NOT step 1.
     - Circuit breaker tracks step 3 failures independently from RecoveryGate.
  3. Topology acquire: pool checkout / create / prepare / open_session / etc.
```

**Why this ordering matters:**
- RecoveryGate is shared across resources on same backend. It answers "is the backend alive?"
- AcquireResilience is per-resource. It answers "can I get an instance right now?"
- If RecoveryGate says "failed", retrying acquire is pointless — the backend is down.
- If AcquireResilience retry exhausts → passive recovery trigger: notify RecoveryGate.

**Passive recovery trigger:**
When AcquireResilience exhausts all retries (step 2 fails), AND the resource has a
RecoveryGroup, the framework calls `gate.try_begin()`. If this is the first failure,
a recovery probe starts. Otherwise, the gate is already in recovery.

```rust
// In ManagedResource::acquire() — after AcquireResilience exhausted:
if let Some(gate) = &self.recovery_gate {
    if let Ok(ticket) = gate.try_begin() {
        // We're the first to notice. Start recovery probe.
        self.spawn_recovery_probe(ticket);
    }
    // Either way, return the error to caller.
}
```

### AcquireResilience — конфиг для acquire path

Lives on `ManagedResource<R>`, NOT on topology config. Set via `RegistrationBuilder::acquire_resilience()`.
Applied per-resource, wraps the topology-specific acquire operation.

```rust
use nebula_resilience::compose::{LayerBuilder, ResilienceChain};

/// Конфигурация resilience для acquire path.
pub struct AcquireResilience {
    /// Timeout на весь acquire (checkout + create + prepare).
    pub timeout: Option<Duration>,
    /// Retry transient create() failures.
    pub retry: Option<AcquireRetryConfig>,
    /// Circuit breaker: если N acquire подряд fail → stop trying.
    /// Uses preset (const-generic CircuitBreaker requires compile-time values).
    pub circuit_breaker: Option<AcquireCircuitBreakerPreset>,
}

pub struct AcquireRetryConfig {
    pub max_attempts: usize,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub backoff: BackoffKind, // Exponential | Fixed | Linear
}

pub enum BackoffKind {
    Exponential,
    Fixed,
    Linear,
}

/// Circuit breaker preset for acquire path.
///
/// nebula-resilience CircuitBreaker uses const generics:
///   CircuitBreaker<const FAILURE_THRESHOLD: usize, const RESET_TIMEOUT_MS: u64>
/// Values must be known at compile time.
///
/// Three presets cover typical use cases:
///   Standard (5 failures, 30s reset) — default for most resources.
///   Fast (3 failures, 10s reset) — latency-sensitive (API calls).
///   Slow (10 failures, 60s reset) — tolerant (batch, background).
///
/// For custom values: add a new type alias in nebula-resilience, or use
/// dynamic builder (CircuitBreakerConfigBuilder) if added in future.
pub enum AcquireCircuitBreakerPreset {
    /// 5 failures, 30s reset. Good default.
    Standard,
    /// 3 failures, 10s reset. Fail fast.
    Fast,
    /// 10 failures, 60s reset. Tolerant.
    Slow,
}

impl AcquireResilience {
    /// Собрать ResilienceChain из конфига.
    /// Вызывается один раз при registration, хранится в ManagedResource.
    pub fn build_chain<T: Send + 'static>(&self) -> Option<ResilienceChain<T>> {
        let mut builder = LayerBuilder::<T>::new();

        if let Some(timeout) = self.timeout {
            builder = builder.with_timeout(timeout);
        }
        if let Some(ref retry) = self.retry {
            builder = builder.with_retry_exponential(retry.max_attempts, retry.base_delay);
        }
        if let Some(ref preset) = self.circuit_breaker {
            let breaker: Arc<CircuitBreaker> = match preset {
                AcquireCircuitBreakerPreset::Standard => Arc::new(StandardCircuitBreaker::default()),
                AcquireCircuitBreakerPreset::Fast     => Arc::new(FastCircuitBreaker::default()),
                AcquireCircuitBreakerPreset::Slow     => Arc::new(SlowCircuitBreaker::default()),
            };
            builder = builder.with_circuit_breaker(breaker);
        }

        Some(builder.build())
    }
}
```

### Per-resource operation protection

Ресурсы (LLM, Payment API) с operation-level resilience используют `nebula-resilience` напрямую.
Не через resource framework — это ответственность resource impl.

```rust
/// LLM resource хранит breaker внутри Runtime.
pub struct LlmRuntime {
    client: reqwest::Client,
    breaker: Arc<CircuitBreaker<5, 30_000>>,  // из nebula-resilience
    rate_limiter: Arc<GovernorRateLimiter>,     // из nebula-resilience
}

impl LlmRuntime {
    pub async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        self.rate_limiter.acquire().await.map_err(LlmError::RateLimited)?;
        self.breaker.execute(|| self.do_chat(req)).await
            .map_err(LlmError::from_resilience)
    }
}
```

### Error mapping: ResilienceError → resource::Error

```rust
impl From<ResilienceError> for crate::Error {
    fn from(e: ResilienceError) -> Self {
        match e.classify() {
            ErrorClass::Transient => Error::transient(e),
            ErrorClass::ResourceExhaustion => Error::backpressure(e),
            ErrorClass::Configuration => Error::permanent(e),
            ErrorClass::Permanent => Error::permanent(e),
            ErrorClass::Unknown => Error::transient(e),
        }
    }
}
```
