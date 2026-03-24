# 04 — Recovery, Extensions, Resilience

---

## Health check levels

Three distinct levels of health checking, from most local to most global:

| Level | Mechanism | Scope | Question |
|-------|-----------|-------|----------|
| **Instance** | `is_broken()`, `Resource::check()` | Single runtime instance | "Is this specific connection alive?" |
| **Acquire** | `AcquireResilience` (timeout/retry/CB) | Per-resource acquire path | "Can I get an instance right now?" |
| **Backend** | `RecoveryGate` + `RecoveryGroup` | Shared infrastructure | "Is the backend reachable at all?" |

Ordering in acquire path: Backend (step 1) → Acquire (step 2) → Instance (step 3).
See "RecoveryGate vs CircuitBreaker" section below for details.

---

## RecoveryGate — thundering herd prevention

Infrastructure-level coordination: one probe attempt, all other callers wait.
Prevents thundering herd when a shared backend goes down (e.g., Postgres primary).

**CAS-based state machine** (via `ArcSwap::compare_and_swap`):

> **NOTE:** `compare_and_swap` may be deprecated in future `arc-swap` versions.
> Current usage is correct: we need early return from CAS loop (`Ok(ticket)` / `Err(waiter)`),
> which `rcu()` does not support. Pin `arc-swap` version in `Cargo.toml`.

```
                  ┌──────────────────────────────────────────────┐
                  │                                              │
                  ▼                                              │
            ┌──────────┐   try_begin() wins CAS    ┌────────────┴───┐
     ──────►│   Idle   │──────────────────────────►│  InProgress    │
            └──────────┘                            │ {attempt, t0}  │
                  ▲                                 └───┬───────┬───┘
                  │                                     │       │
           resolve(ticket)                    fail_transient  fail_permanent
                  │                                     │       │
                  │           ┌──────────────┐          │       │
                  │           │   Failed     │◄─────────┘       │
                  │           │ {err, until, │                  │
                  │           │  attempt}    │                  ▼
                  │           └──────┬───────┘       ┌──────────────────┐
                  │                  │               │ PermanentlyFailed│
                  │     backoff expires +            │ {error}          │
                  │     attempt < max                └──────────────────┘
                  │          │                         ▲
                  │          │  try_begin()             │ attempt >= max
                  │          │  wins CAS               │
                  │          └──► InProgress ───────────┘
                  │                    │                (escalation)
                  └────────────────────┘
                       resolve(ticket)
```

**Key invariant:** Only ONE caller holds a `RecoveryTicket` at a time.
All other callers receive `RecoveryWaiter` and block on `Notify`.

**Correctness amendments integrated:**
- **#6**: `try_begin()` fully implemented with CAS loop (was `todo!()`).
- **#17**: `RecoveryWaiter` is `'static + Send` — holds `Arc` clones, not borrowed refs. Compatible with `tokio::spawn`.
- **#22**: `RecoveryTicket` has a Drop guard — auto-fails with transient error if dropped without `resolve()`/`fail_*()`. Prevents gate from being permanently stuck in `InProgress`.

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

**Решение:** `nebula-resource` НЕ содержит собственных CircuitBreaker, RateLimiter, RetryPolicy, BulkheadSemaphore. Всё это уже есть в `nebula-resilience` (`CircuitBreaker`, `RetryConfig`, `RateLimiter` trait impls, `Bulkhead`, `PipelineBuilder` → `ResiliencePipeline`). Используем напрямую.

### RecoveryGate vs CircuitBreaker — different concerns, complementary

| | RecoveryGate | CircuitBreaker |
|---|---|---|
| **Level** | Infrastructure (shared backend) | Operation (per-resource acquire) |
| **Question** | "Is the backend alive at all?" | "Are too many operations failing?" |
| **State machine** | Idle → InProgress → Failed/Idle | Closed → Open → HalfOpen → Closed |
| **Coordination** | Single winner probes; all others wait | Each caller decides independently |
| **Lives in** | `nebula-resource` (plan 04) | `nebula-resilience` (reused, not reimplemented) |
| **Shared across** | Multiple resources on same backend (via `RecoveryGroup`) | Single `ManagedResource` |
| **Trigger** | Watchdog or passive (acquire exhaustion) | Consecutive call failures |

RecoveryGate is infrastructure-level coordination — "is the backend reachable?"
CircuitBreaker is operation-level protection — "should I even try this call?"
They complement each other in the acquire path:

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
use nebula_resilience::{PipelineBuilder, ResiliencePipeline};

/// Конфигурация resilience для acquire path.
pub struct AcquireResilience {
    /// Timeout на весь acquire (checkout + create + prepare).
    pub timeout: Option<Duration>,
    /// Retry transient create() failures.
    pub retry: Option<AcquireRetryConfig>,
    /// Circuit breaker: если N acquire подряд fail → stop trying.
    /// Uses preset (CircuitBreakerConfig, not const-generic).
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
/// Uses `CircuitBreakerConfig` from `nebula-resilience` (runtime config, not const generics).
///
/// Three presets cover typical real-world resource patterns:
///
///   **Standard** (5 failures, 30s reset) — databases, message brokers.
///     Postgres, MySQL, Kafka, RabbitMQ. Moderate tolerance for transient failures.
///
///   **Fast** (3 failures, 10s reset) — low-latency caches and APIs.
///     Redis, Memcached, HTTP APIs. Fail fast to avoid cascading latency.
///
///   **Slow** (10 failures, 60s reset) — high-latency or batch resources.
///     SSH tunnels, SMTP, browser automation, LLM APIs. Tolerant because
///     individual operations are inherently slow / flaky.
///
/// For custom values: use `CircuitBreakerConfig` directly.
pub enum AcquireCircuitBreakerPreset {
    /// 5 failures, 30s reset. Postgres, Kafka, RabbitMQ.
    Standard,
    /// 3 failures, 10s reset. Redis, Memcached, HTTP APIs.
    Fast,
    /// 10 failures, 60s reset. SSH, SMTP, Browser, LLM.
    Slow,
}

impl AcquireResilience {
    /// Build a `ResiliencePipeline` from config.
    /// Called once at registration time, stored in `ManagedResource`.
    ///
    /// Uses `PipelineBuilder` from `nebula-resilience` — NOT custom retry/CB logic.
    /// Layer order follows the recommended pipeline ordering:
    /// timeout (outermost) → retry → circuit_breaker (innermost).
    pub fn build_pipeline<E: Send + 'static>(&self) -> Option<ResiliencePipeline<E>> {
        let mut builder = PipelineBuilder::<E>::new();

        if let Some(timeout) = self.timeout {
            builder = builder.timeout(timeout);
        }
        if let Some(ref retry) = self.retry {
            let config = RetryConfig::<E>::new(retry.max_attempts)
                .expect("max_attempts > 0")
                .backoff(match retry.backoff {
                    BackoffKind::Exponential => BackoffConfig::Exponential {
                        initial: retry.base_delay,
                        max: retry.max_delay,
                        multiplier: 2.0,
                    },
                    BackoffKind::Fixed  => BackoffConfig::Fixed(retry.base_delay),
                    BackoffKind::Linear => BackoffConfig::Linear {
                        initial: retry.base_delay,
                        increment: retry.base_delay,
                        max: retry.max_delay,
                    },
                });
            builder = builder.retry(config);
        }
        if let Some(ref preset) = self.circuit_breaker {
            let config = match preset {
                AcquireCircuitBreakerPreset::Standard => CircuitBreakerConfig {
                    failure_threshold: 5,
                    reset_timeout: Duration::from_secs(30),
                    ..Default::default()
                },
                AcquireCircuitBreakerPreset::Fast => CircuitBreakerConfig {
                    failure_threshold: 3,
                    reset_timeout: Duration::from_secs(10),
                    ..Default::default()
                },
                AcquireCircuitBreakerPreset::Slow => CircuitBreakerConfig {
                    failure_threshold: 10,
                    reset_timeout: Duration::from_secs(60),
                    ..Default::default()
                },
            };
            let breaker = CircuitBreaker::new(config).expect("valid CB config");
            builder = builder.circuit_breaker(Arc::new(breaker));
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
    breaker: Arc<CircuitBreaker>,          // из nebula-resilience
    rate_limiter: Arc<GovernorRateLimiter>, // из nebula-resilience
}

impl LlmRuntime {
    pub async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, LlmError> {
        self.rate_limiter.acquire().await.map_err(LlmError::RateLimited)?;
        self.breaker.call(|| Box::pin(self.do_chat(req))).await
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
