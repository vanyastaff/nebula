# 03 — Infrastructure: ResourceHandle, LeaseGuard, primitives

---

## ResourceHandle — unified access для Action

Action author видит ОДИН тип. Topology скрыт. Deref к `R::Lease`.

Three ownership patterns cover all 7 topologies:

```rust
pub struct ResourceHandle<R: Resource> {
    inner: HandleInner<R>,
    resource_key: ResourceKey,     // from R::KEY — for diagnostics/logs/metrics
    topology_tag: &'static str,   // "pool", "resident", "service", etc.
}

enum HandleInner<R: Resource> {
    /// Owned value, no cleanup. Drop = drop value.
    /// Used by: Resident (clone), Service Cloned (token).
    Owned(R::Lease),

    /// Owned value + async cleanup callback.
    /// Used by: Pool (recycle/destroy), Service Tracked (release_token),
    /// Transport (close_session via ReleaseQueue).
    Guarded {
        value: Option<R::Lease>,
        on_release: Option<Box<dyn FnOnce(R::Lease, bool) + Send>>,
        tainted: bool,
        acquired_at: Instant,
    },

    /// Shared ref + async cleanup callback.
    /// Used by: Exclusive (reset + permit release via ReleaseQueue).
    Shared {
        value: Arc<R::Lease>,
        on_release: Option<Box<dyn FnOnce(bool) + Send>>,
        tainted: bool,
        acquired_at: Instant,
    },
}

impl<R: Resource> Deref for ResourceHandle<R> {
    type Target = R::Lease;

    fn deref(&self) -> &R::Lease {
        match &self.inner {
            HandleInner::Owned(v) => v,
            HandleInner::Guarded { value, .. } => {
                // SAFETY: value всегда Some пока ResourceHandle существует.
                // detach() потребляет self + вызывает std::mem::forget →
                // после detach нет ResourceHandle для deref (compile-time guarantee).
                // None здесь = framework bug, не user error.
                value.as_ref().unwrap_or_else(|| {
                    unreachable!(
                        "ResourceHandle::deref on consumed value. \
                         This is a framework bug (should be unreachable by construction)."
                    )
                })
            }
            HandleInner::Shared { value, .. } => value.as_ref(),
        }
    }
}

impl<R: Resource> ResourceHandle<R> {
    /// Resource key for diagnostics/logging/UI.
    pub fn resource_key(&self) -> &ResourceKey { &self.resource_key }

    /// Topology tag for diagnostics/logging/UI (e.g., "pool", "resident", "service").
    pub fn topology_tag(&self) -> &'static str { self.topology_tag }

    /// Пометить как broken. Guarded/Shared only.
    /// Owned — noop (no cleanup path to affect).
    pub fn taint(&mut self) {
        match &mut self.inner {
            HandleInner::Guarded { tainted, .. } => *tainted = true,
            HandleInner::Shared { tainted, .. } => *tainted = true,
            HandleInner::Owned(_) => {}
        }
    }

    /// Отсоединить от pool. Caller становится owner. Pool не ждёт возврата.
    /// Guarded only (Pool, Transport). Disarms on_release callback.
    ///
    /// Потребляет self — после detach нет ResourceHandle для deref.
    /// std::mem::forget предотвращает повторный вызов Drop.
    pub fn detach(mut self) -> Result<R::Lease, DetachError> {
        match &mut self.inner {
            HandleInner::Guarded { value, on_release, .. } => {
                on_release.take(); // disarm callback
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

#[derive(Debug, thiserror::Error)]
pub enum DetachError {
    #[error("lease already consumed (framework bug)")]
    AlreadyConsumed,
    #[error("this handle type does not support detach (Owned = already yours; Shared = not detachable)")]
    NotDetachable,
}

    /// Как долго handle удерживается. Guarded/Shared only.
    pub fn hold_duration(&self) -> Option<Duration> {
        match &self.inner {
            HandleInner::Guarded { acquired_at, .. } => Some(acquired_at.elapsed()),
            HandleInner::Shared { acquired_at, .. } => Some(acquired_at.elapsed()),
            HandleInner::Owned(_) => None,
        }
    }
}

impl<R: Resource> Drop for ResourceHandle<R> {
    fn drop(&mut self) {
        match &mut self.inner {
            HandleInner::Owned(_) => {} // noop
            HandleInner::Guarded { value, on_release, tainted, .. } => {
                if let (Some(v), Some(f)) = (value.take(), on_release.take()) {
                    f(v, *tainted); // sync: submits to ReleaseQueue
                }
            }
            HandleInner::Shared { on_release, tainted, .. } => {
                if let Some(f) = on_release.take() {
                    f(*tainted); // sync: submits to ReleaseQueue
                }
            }
        }
    }
}
```

**Constructors (crate-internal):**

```rust
impl<R: Resource> ResourceHandle<R> {
    /// Resident clone, Service Cloned token.
    pub(crate) fn owned(lease: R::Lease, key: ResourceKey, tag: &'static str) -> Self {
        Self { inner: HandleInner::Owned(lease), resource_key: key, topology_tag: tag }
    }

    /// Pool checkout, Service Tracked, Transport session.
    pub(crate) fn guarded(
        lease: R::Lease,
        on_release: Box<dyn FnOnce(R::Lease, bool) + Send>,
        key: ResourceKey,
        tag: &'static str,
    ) -> Self {
        Self {
            inner: HandleInner::Guarded {
                value: Some(lease),
                on_release: Some(on_release),
                tainted: false,
                acquired_at: Instant::now(),
            },
            resource_key: key,
            topology_tag: tag,
        }
    }

    /// Exclusive (Arc-shared + permit).
    pub(crate) fn shared(
        lease: Arc<R::Lease>,
        on_release: Box<dyn FnOnce(bool) + Send>,
        key: ResourceKey,
        tag: &'static str,
    ) -> Self {
        Self {
            inner: HandleInner::Shared {
                value: lease,
                on_release: Some(on_release),
                tainted: false,
                acquired_at: Instant::now(),
            },
            resource_key: key,
            topology_tag: tag,
        }
    }
}
```

**Использование в Action:**

```rust
// Action author — всегда одинаково:
let db  = ctx.resource::<Postgres>().await?;        // Pool → Deref to PgConnection
let bot = ctx.resource::<TelegramBot>().await?;     // Service → Deref to TelegramBotHandle
let ssh = ctx.resource::<Ssh>().await?;             // Transport → Deref to SshSession
let kc  = ctx.resource::<KafkaConsumer>().await?;   // Exclusive → Deref to StreamConsumer

// Deref → R::Lease → resource-specific API:
db.query("SELECT 1", &[]).await?;
bot.send_message(chat_id, "hi").await?;
ssh.exec("ls -la").await?;
kc.poll(Duration::from_secs(1));

// drop → automatic cleanup. Action author не думает о checkin/release.
```

---

## LeaseGuard — RAII internal primitive

**Note:** With the 3-variant HandleInner, LeaseGuard is only used internally by Pool's idle_queue to wrap `R::Runtime` entries. HandleInner::Guarded replaces the previous per-topology LeaseGuard usage at the ResourceHandle level.

```rust
/// Internal RAII wrapper for pool entries.
/// Tracks per-instance metrics and provides on_release callback.
pub(crate) struct LeaseGuard<L> {
    lease: Option<L>,
    tainted: bool,
    poison: Arc<AtomicBool>,
    on_release: Option<Box<dyn FnOnce(L, bool) + Send>>,
    resource_key: ResourceKey,
    acquired_at: Instant,
}

impl<L: Send + 'static> Deref for LeaseGuard<L> {
    type Target = L;
    fn deref(&self) -> &L {
        self.lease.as_ref().expect("lease already consumed")
    }
}

impl<L: Send + 'static> LeaseGuard<L> {
    pub fn taint(&mut self) { self.tainted = true; }
    pub fn is_tainted(&self) -> bool { self.tainted || self.poison.load(Ordering::Acquire) }
    pub fn hold_duration(&self) -> Duration { self.acquired_at.elapsed() }

    pub fn detach(mut self) -> L {
        self.on_release.take();
        self.lease.take().unwrap()
    }

    pub fn poison_token(&self) -> PoisonToken {
        PoisonToken { flag: Arc::clone(&self.poison) }
    }
}

impl<L: Send + 'static> Drop for LeaseGuard<L> {
    fn drop(&mut self) {
        let tainted = self.tainted; // snapshot at drop time
        if let (Some(lease), Some(release_fn)) = (self.lease.take(), self.on_release.take()) {
            // poison_flag передаётся BY REFERENCE в release pipeline.
            // Финальная проверка tainted_at_drop || poison.load() происходит
            // в ReleaseQueue worker непосредственно перед recycle vs destroy решением.
            // Это сужает race window до минимума (atomic load + branch).
            release_fn(lease, tainted, Arc::clone(&self.poison));
        }
    }
}

// NOTE: on_release сигнатура обновлена:
//   Box<dyn FnOnce(L, bool) + Send>
//   → Box<dyn FnOnce(L, bool, Arc<AtomicBool>) + Send>
//                      ^tainted_at_drop  ^poison_flag

/// Shared poison flag for scope coordination.
pub struct PoisonToken {
    flag: Arc<AtomicBool>,
}

impl PoisonToken {
    pub fn poison(&self) { self.flag.store(true, Ordering::Release); }
    pub fn is_poisoned(&self) -> bool { self.flag.load(Ordering::Acquire) }
}
```

---

## AcquireOptions

```rust
#[derive(Debug, Clone)]
pub struct AcquireOptions {
    /// Намерение использования. Влияет на timeout и metrics.
    pub intent:   AcquireIntent,
    /// Дедлайн. Если pool full — ждать не дольше этого.
    /// NOTE: If deadline is in the past, acquire should return error immediately.
    /// Implementation should check `deadline.filter(|d| *d > Instant::now())`.
    pub deadline: Option<Instant>,
    /// Произвольные tags для tracing/metrics.
    pub tags:     SmallVec<[(Cow<'static, str>, Cow<'static, str>); 2]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AcquireIntent {
    /// Нормальное использование. Default timeout.
    #[default]
    Standard,
    /// Часы. SSH tunnel, port forward, long consumer loop.
    /// Pool не считает lease "stuck".
    LongRunning,
    /// 5-60 секунд. LLM stream, video transcode.
    /// Pool не timeout-ит, но metrics отслеживают duration.
    Streaming { expected: Duration },
    /// Низкий приоритет. Pool может delay в пользу Standard.
    Prefetch,
    /// Высший приоритет. Нет backpressure timeout.
    /// Для: health check, credential rotation, config reload.
    Critical,
}

impl AcquireOptions {
    pub fn standard() -> Self {
        Self { intent: AcquireIntent::Standard, deadline: None, tags: SmallVec::new() }
    }
    pub fn with_intent(mut self, intent: AcquireIntent) -> Self { self.intent = intent; self }
    pub fn with_deadline(mut self, deadline: Instant) -> Self { self.deadline = Some(deadline); self }
    pub fn with_tag(mut self, key: impl Into<Cow<'static, str>>, value: impl Into<Cow<'static, str>>) -> Self {
        self.tags.push((key.into(), value.into())); self
    }
}
```

---

## InstanceMetrics

Per-instance metrics. Pool хранит рядом с каждым idle instance.

```rust
pub struct InstanceMetrics {
    // ── Resource author reads in recycle() ──

    /// Сколько ошибок за lifetime этого instance.
    /// Resource author: "5+ errors → Drop, instance unreliable".
    pub error_count: u64,

    /// Сколько раз instance выдавался callers.
    /// Resource author: "1000+ checkouts → Drop, force-rotate".
    pub checkout_count: u64,

    /// Когда instance создан.
    pub created_at: Instant,

    // ── Framework uses (policy decisions BEFORE recycle) ──

    /// Config fingerprint при создании. Stale detection.
    pub(crate) config_fingerprint: u64,

    /// Когда последний checkin (возврат в idle). Idle timeout reaping.
    pub(crate) last_checkin: Instant,

    /// Суммарное время hold callers. Metrics emission.
    pub(crate) total_hold_duration: Duration,
}

impl InstanceMetrics {
    pub fn age(&self) -> Duration { self.created_at.elapsed() }
    pub(crate) fn idle_duration(&self) -> Duration { self.last_checkin.elapsed() }
    pub(crate) fn is_stale(&self, current_fingerprint: u64) -> bool {
        self.config_fingerprint != current_fingerprint
    }
    pub(crate) fn record_error(&mut self) { self.error_count += 1; }
    pub(crate) fn record_checkout(&mut self) { self.checkout_count += 1; }
    pub(crate) fn record_checkin(&mut self, hold: Duration) {
        self.last_checkin = Instant::now();
        self.total_hold_duration += hold;
    }
}
```

---

## Cell — lock-free ячейка для Resident

```rust
use arc_swap::ArcSwapOption;

/// Lock-free ячейка для одного значения. ArcSwapOption-based.
/// Read = load_full() → Option<Arc<T>>. One atomic op.
/// Write = store(Arc<T>) → atomic swap. Old Arc dropped when refcount → 0.
///
/// Исправлено (#16): предыдущий ArcSwap<Option<T>> + load_arc() возвращал
/// Arc<Option<T>> — is_some() всегда true (проверяет не-null Arc, не inner Option).
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
    /// Resident acquire: cell.load()? → Arc<T> → clone T → HandleInner::Owned.
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

    /// Clear the cell, returning the old value if any.
    pub fn take(&self) -> Option<Arc<T>> {
        self.inner.swap(None)
    }
}
```

---

## ReleaseQueue — shared async cleanup workers

One ReleaseQueue per ManagedResource. Used by Pool (recycle/destroy), Transport (close_session), and Exclusive (reset + permit release).

```rust
/// Async queue для обработки returned instances.
/// N parallel primary workers + 1 dedicated fallback worker.
/// Worker count determined by topology:
///   Pool: configurable (Postgres=1 ~1ms, Browser=4 ~500ms).
///   Transport: 1 worker.
///   Exclusive: 1 worker.
///   Others: 0 (no ReleaseQueue needed).
///
/// Architecture (amended from #14, reviewed per Gemini audit):
///   - N primary workers: each owns its own rx (no Mutex on hot path).
///   - 1 dedicated fallback worker: sole owner of fallback_rx (no Mutex).
///   - Fallback is BOUNDED (10_000) to prevent OOM under sustained backend failure.
///   - Every release_fn execution has a 30s timeout to prevent worker paralysis.
///   - If both primary and fallback are full: task is dropped (intentional fail-open
///     for process survival). This leaks a connection — recovery/maintenance paths
///     will detect stale instances and clean them up.
pub struct ReleaseQueue {
    /// N senders — one per primary worker. Submit round-robin.
    senders:     Vec<mpsc::Sender<ReleaseTask>>,
    /// Round-robin counter.
    next_worker: AtomicUsize,
    /// Bounded fallback — overflow buffer for burst load.
    /// BOUNDED (not unbounded) to prevent OOM if backend hangs and workers stall.
    fallback_tx: mpsc::Sender<ReleaseTask>,
    metrics:     Arc<ReleaseQueueMetrics>,
    workers:     Vec<JoinHandle<()>>,
}

/// Release timeout. If a release_fn (destroy/recycle) takes longer than this,
/// it is abandoned. Prevents worker paralysis when backend hangs.
const RELEASE_TASK_TIMEOUT: Duration = Duration::from_secs(30);

/// Fallback queue capacity. Large enough for burst, bounded to prevent OOM.
const FALLBACK_CAPACITY: usize = 10_000;

struct ReleaseTask {
    release_fn: Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>,
}

/// Per-queue metrics. `dropped` should be 0 in a healthy system — alert if growing.
#[derive(Default)]
pub struct ReleaseQueueMetrics {
    pub submitted:     AtomicU64,
    /// Primary worker was full — task routed to fallback (burst load).
    pub fallback_used: AtomicU64,
    /// Task dropped (both primary and fallback full, or shutdown).
    /// Intentional fail-open: leaks connection to prevent OOM.
    /// Recovery: pool maintenance will detect stale/missing instances.
    pub dropped:       AtomicU64,
    /// release_fn exceeded RELEASE_TASK_TIMEOUT. Backend may be hanging.
    pub timed_out:     AtomicU64,
}

/// Execute a release task with timeout protection.
async fn execute_with_timeout(task: ReleaseTask, metrics: &ReleaseQueueMetrics) {
    match tokio::time::timeout(RELEASE_TASK_TIMEOUT, (task.release_fn)()).await {
        Ok(()) => {}
        Err(_) => {
            metrics.timed_out.fetch_add(1, Ordering::Relaxed);
            tracing::error!(
                "ReleaseQueue: release task timed out after {:?}, dropping. \
                 Backend may be hanging — investigate.",
                RELEASE_TASK_TIMEOUT
            );
        }
    }
}

impl ReleaseQueue {
    pub fn new(capacity: usize, num_workers: usize, cancel: CancellationToken) -> Self {
        let metrics = Arc::new(ReleaseQueueMetrics::default());

        // Bounded fallback with dedicated worker (no Mutex, no contention).
        let (fallback_tx, fallback_rx) = mpsc::channel::<ReleaseTask>(FALLBACK_CAPACITY);
        let fb_cancel  = cancel.clone();
        let fb_metrics = Arc::clone(&metrics);
        let fallback_worker = tokio::spawn(async move {
            let mut fallback_rx = fallback_rx;
            loop {
                tokio::select! {
                    biased;
                    _ = fb_cancel.cancelled() => {
                        // Drain remaining on shutdown.
                        while let Ok(t) = fallback_rx.try_recv() {
                            execute_with_timeout(t, &fb_metrics).await;
                        }
                        break;
                    }
                    task = fallback_rx.recv() => {
                        match task {
                            Some(t) => execute_with_timeout(t, &fb_metrics).await,
                            None => break,
                        }
                    }
                }
            }
        });

        // N independent primary workers — true parallelism.
        // Each worker owns its own rx without Mutex.
        let per_worker_cap = (capacity / num_workers.max(1)).max(1);
        let mut senders = Vec::with_capacity(num_workers);
        let mut workers = Vec::with_capacity(num_workers + 1);
        workers.push(fallback_worker);

        for _ in 0..num_workers {
            let (tx, rx) = mpsc::channel::<ReleaseTask>(per_worker_cap);
            senders.push(tx);
            let cancel  = cancel.clone();
            let metrics = Arc::clone(&metrics);
            workers.push(tokio::spawn(async move {
                let mut rx = rx;
                loop {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => {
                            while let Ok(t) = rx.try_recv() {
                                execute_with_timeout(t, &metrics).await;
                            }
                            break;
                        }
                        task = rx.recv() => {
                            match task {
                                Some(t) => execute_with_timeout(t, &metrics).await,
                                None => break,
                            }
                        }
                    }
                }
            }));
        }

        Self {
            senders,
            next_worker: AtomicUsize::new(0),
            fallback_tx,
            metrics,
            workers,
        }
    }

    /// Submit release task. Non-blocking (sync — safe to call from Drop).
    ///
    /// Round-robin across primary workers. If selected worker full → bounded fallback.
    /// If fallback also full → drop task (intentional fail-open to prevent OOM).
    pub fn submit(&self, task: ReleaseTask) {
        self.metrics.submitted.fetch_add(1, Ordering::Relaxed);
        let idx = self.next_worker.fetch_add(1, Ordering::Relaxed) % self.senders.len();
        match self.senders[idx].try_send(task) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(task)) => {
                self.metrics.fallback_used.fetch_add(1, Ordering::Relaxed);
                tracing::warn!(
                    "ReleaseQueue worker {} full, routing to fallback. \
                     Consider increasing per-worker capacity.",
                    idx
                );
                match self.fallback_tx.try_send(task) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                        tracing::error!(
                            "ReleaseQueue: both primary and fallback full. \
                             Dropping release task to prevent OOM. \
                             This leaks a connection — investigate backend health."
                        );
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                        tracing::error!("ReleaseQueue fallback closed during shutdown");
                    }
                }
            }
            Err(mpsc::error::TrySendError::Closed(_task)) => {
                self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                tracing::error!("ReleaseQueue worker {} channel closed", idx);
            }
        }
    }

    pub fn metrics(&self) -> &ReleaseQueueMetrics { &self.metrics }

    /// Lightweight handle for submit. Clone-able, shares senders + metrics.
    pub fn handle(&self) -> ReleaseQueueHandle {
        ReleaseQueueHandle {
            senders:     self.senders.clone(),
            next_worker: Arc::new(AtomicUsize::new(0)),
            fallback_tx: self.fallback_tx.clone(),
            metrics:     Arc::clone(&self.metrics),
        }
    }

    /// Graceful shutdown. Drop all senders → workers drain then exit.
    pub async fn shutdown(self) {
        drop(self.senders);
        drop(self.fallback_tx);
        for w in self.workers {
            let _ = w.await;
        }
    }
}

/// Lightweight handle for submit only. Clone-able.
#[derive(Clone)]
pub struct ReleaseQueueHandle {
    senders:     Vec<mpsc::Sender<ReleaseTask>>,
    next_worker: Arc<AtomicUsize>,
    fallback_tx: mpsc::Sender<ReleaseTask>,
    metrics:     Arc<ReleaseQueueMetrics>,
}

impl ReleaseQueueHandle {
    pub fn submit(&self, task: ReleaseTask) {
        self.metrics.submitted.fetch_add(1, Ordering::Relaxed);
        let idx = self.next_worker.fetch_add(1, Ordering::Relaxed) % self.senders.len();
        match self.senders[idx].try_send(task) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(task)) => {
                self.metrics.fallback_used.fetch_add(1, Ordering::Relaxed);
                match self.fallback_tx.try_send(task) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(_)) => {
                        self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                        tracing::error!(
                            "ReleaseQueue: both primary and fallback full, task dropped"
                        );
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => {
                        self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.metrics.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}
```

### Release flow per topology

```
Pool:
  HandleInner::Guarded drop → on_release(lease, tainted)
    → is_broken(lease)? broken → submit destroy to ReleaseQueue
    → healthy → submit recycle to ReleaseQueue
    → ReleaseQueue worker: framework policy (fingerprint, max_lifetime)
      → resource.recycle() → Keep → push idle / Drop → destroy

Transport:
  HandleInner::Guarded drop → on_release(session, tainted)
    → submit close_session to ReleaseQueue
    → ReleaseQueue worker: resource.close_session(transport, session, !tainted)

Exclusive:
  HandleInner::Shared drop → on_release(tainted)
    → submit reset + permit release to ReleaseQueue
    → ReleaseQueue worker: if !tainted → resource.reset(runtime)
      → if reset fails → destroy + recreate
      → drop(permit) → semaphore permit released → next caller unblocked
```
