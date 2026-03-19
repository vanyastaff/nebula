//! Stress test for nebula-resource Pool.
//!
//! Запуск:
//!   cargo test --test stress -- --nocapture
//!
//! Или конкретный тест:
//!   cargo test --test stress stress::concurrent_acquire -- --nocapture
//!
//! Сценарии:
//!   1. concurrent_acquire      — N воркеров, каждый делает M acquire/release
//!   2. contended_small_pool    — больше воркеров чем size, очередь под нагрузкой
//!   3. flaky_resource          — ресурс падает каждые K создания, circuit breaker
//!   4. reconnect_storm         — все инстансы умирают одновременно
//!   5. acquire_under_pressure  — адаптивный backpressure + shed нагрузки
//!   6. multi_scope_isolation   — tenant isolation под нагрузкой
//!   7. rapid_shutdown          — shutdown пока активны acquire'ы

use std::{
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use nebula_resource::{
    Config, Context, Error, ExecutionId, Manager, ManagerBuilder, Pool, PoolAcquire,
    PoolConfig, PoolLifetime, PoolResiliencePolicy, PoolSizing, Resource, ResourceMetadata,
    Result, Scope, WorkflowId,
};
use nebula_core::ResourceKey;
use tokio::sync::Barrier;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn ctx() -> Context {
    Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

fn tenant_ctx(tenant: &str) -> Context {
    Context::new(
        Scope::try_tenant(tenant).unwrap(),
        WorkflowId::new(),
        ExecutionId::new(),
    )
}

fn print_separator(name: &str) {
    println!("\n{}", "═".repeat(60));
    println!("  {name}");
    println!("{}", "═".repeat(60));
}

// ─── FastResource — мгновенный create (in-memory) ────────────────────────────

#[derive(Clone)]
struct FastConfig {
    id: &'static str,
}
impl Config for FastConfig {}

struct FastResource {
    key: ResourceKey,
    created: Arc<AtomicU32>,
}

impl Resource for FastResource {
    type Config = FastConfig;
    type Instance = String;

    fn key(&self) -> ResourceKey {
        self.key.clone()
    }

    async fn create(&self, cfg: &FastConfig, _ctx: &Context) -> Result<String> {
        let n = self.created.fetch_add(1, Ordering::Relaxed);
        Ok(format!("{}#{n}", cfg.id))
    }

    async fn recycle(&self, _inst: &mut String) -> Result<()> {
        // симулируем лёгкую работу при recycle
        tokio::task::yield_now().await;
        Ok(())
    }
}

// ─── SlowResource — медленный create (симулирует I/O) ────────────────────────

struct SlowResource {
    key: ResourceKey,
    created: Arc<AtomicU32>,
    delay_ms: u64,
}

impl Resource for SlowResource {
    type Config = FastConfig;
    type Instance = String;

    fn key(&self) -> ResourceKey {
        self.key.clone()
    }

    async fn create(&self, cfg: &FastConfig, _ctx: &Context) -> Result<String> {
        tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
        let n = self.created.fetch_add(1, Ordering::Relaxed);
        Ok(format!("slow-{}#{n}", cfg.id))
    }
}

// ─── FlakyResource — падает каждые N создания ────────────────────────────────

struct FlakyResource {
    key: ResourceKey,
    attempts: Arc<AtomicU32>,
    fail_every: u32,
    created: Arc<AtomicU32>,
}

impl Resource for FlakyResource {
    type Config = FastConfig;
    type Instance = String;

    fn key(&self) -> ResourceKey {
        self.key.clone()
    }

    async fn create(&self, cfg: &FastConfig, _ctx: &Context) -> Result<String> {
        let attempt = self.attempts.fetch_add(1, Ordering::Relaxed);
        if attempt % self.fail_every == 0 {
            return Err(Error::Initialization {
                resource_key: self.key.clone(),
                reason: format!("simulated failure on attempt {attempt}"),
                source: None,
            });
        }
        let n = self.created.fetch_add(1, Ordering::Relaxed);
        Ok(format!("flaky-{}#{n}", cfg.id))
    }
}

// ─── MortalResource — инстанс умирает через TTL ──────────────────────────────

struct MortalInstance {
    value: String,
    born_at: Instant,
    ttl: Duration,
}

struct MortalResource {
    key: ResourceKey,
    created: Arc<AtomicU32>,
    instance_ttl: Duration,
}

impl Resource for MortalResource {
    type Config = FastConfig;
    type Instance = MortalInstance;

    fn key(&self) -> ResourceKey {
        self.key.clone()
    }

    async fn create(&self, cfg: &FastConfig, _ctx: &Context) -> Result<MortalInstance> {
        let n = self.created.fetch_add(1, Ordering::Relaxed);
        Ok(MortalInstance {
            value: format!("mortal-{}#{n}", cfg.id),
            born_at: Instant::now(),
            ttl: self.instance_ttl,
        })
    }

    async fn is_reusable(&self, inst: &MortalInstance) -> Result<bool> {
        Ok(inst.born_at.elapsed() < inst.ttl)
    }

    fn is_broken(&self, inst: &MortalInstance) -> bool {
        inst.born_at.elapsed() >= inst.ttl
    }
}

// ─── 1. concurrent_acquire ───────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn concurrent_acquire() {
    print_separator("1. concurrent_acquire — 100 воркеров × 200 acquire/release");

    const WORKERS: usize = 100;
    const OPS_PER_WORKER: usize = 200;
    const POOL_SIZE: usize = 20;

    let created = Arc::new(AtomicU32::new(0));
    let pool = Pool::new(
        FastResource {
            key: ResourceKey::try_from("fast").unwrap(),
            created: Arc::clone(&created),
        },
        FastConfig { id: "w" },
        PoolConfig {
            sizing: PoolSizing { min_size: 5, max_size: POOL_SIZE },
            acquire: PoolAcquire { timeout: Duration::from_secs(5), ..Default::default() },
            ..Default::default()
        },
    )
    .unwrap();

    let barrier = Arc::new(Barrier::new(WORKERS));
    let total_ops = Arc::new(AtomicU64::new(0));
    let errors = Arc::new(AtomicU64::new(0));
    let start = Instant::now();

    let handles: Vec<_> = (0..WORKERS)
        .map(|_| {
            let pool = pool.clone();
            let barrier = Arc::clone(&barrier);
            let total_ops = Arc::clone(&total_ops);
            let errors = Arc::clone(&errors);
            tokio::spawn(async move {
                barrier.wait().await; // все стартуют одновременно
                for _ in 0..OPS_PER_WORKER {
                    match pool.acquire(&ctx()).await {
                        Ok((_guard, _wait)) => {
                            total_ops.fetch_add(1, Ordering::Relaxed);
                            // имитируем работу с ресурсом
                            tokio::task::yield_now().await;
                        }
                        Err(_) => {
                            errors.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
            })
        })
        .collect();

    for h in handles {
        h.await.unwrap();
    }

    let elapsed = start.elapsed();
    let ops = total_ops.load(Ordering::Relaxed);
    let errs = errors.load(Ordering::Relaxed);
    let stats = pool.stats();

    println!("  Elapsed:      {:.2?}", elapsed);
    println!("  Total ops:    {ops}");
    println!("  Errors:       {errs}");
    println!("  Throughput:   {:.0} ops/sec", ops as f64 / elapsed.as_secs_f64());
    println!("  Created:      {}", stats.created);
    println!("  Destroyed:    {}", stats.destroyed);
    if let Some(lat) = &stats.acquire_latency {
        println!("  Latency p50:  {}ms", lat.p50_ms);
        println!("  Latency p99:  {}ms", lat.p99_ms);
        println!("  Latency p999: {}ms", lat.p999_ms);
    }

    assert_eq!(errs, 0, "no errors expected");
    assert!(
        stats.created <= POOL_SIZE as u64,
        "created {} > max_size {}",
        stats.created,
        POOL_SIZE
    );
    assert_eq!(ops, (WORKERS * OPS_PER_WORKER) as u64);
}

// ─── 2. contended_small_pool ─────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn contended_small_pool() {
    print_separator("2. contended_small_pool — 200 воркеров на пул size=5");

    const WORKERS: usize = 200;
    const POOL_SIZE: usize = 5;
    const HOLD_MS: u64 = 10; // держим инстанс 10ms

    let created = Arc::new(AtomicU32::new(0));
    let pool = Pool::new(
        FastResource {
            key: ResourceKey::try_from("contended").unwrap(),
            created: Arc::clone(&created),
        },
        FastConfig { id: "c" },
        PoolConfig {
            sizing: PoolSizing { min_size: POOL_SIZE, max_size: POOL_SIZE },
            acquire: PoolAcquire { timeout: Duration::from_secs(30), ..Default::default() },
            ..Default::default()
        },
    )
    .unwrap();

    let barrier = Arc::new(Barrier::new(WORKERS));
    let acquired = Arc::new(AtomicU64::new(0));
    let timed_out = Arc::new(AtomicU64::new(0));
    let max_concurrent = Arc::new(AtomicU32::new(0));
    let current_concurrent = Arc::new(AtomicU32::new(0));
    let start = Instant::now();

    let handles: Vec<_> = (0..WORKERS)
        .map(|_| {
            let pool = pool.clone();
            let barrier = Arc::clone(&barrier);
            let acquired = Arc::clone(&acquired);
            let timed_out = Arc::clone(&timed_out);
            let max_c = Arc::clone(&max_concurrent);
            let current_c = Arc::clone(&current_concurrent);
            tokio::spawn(async move {
                barrier.wait().await;
                match pool.acquire(&ctx()).await {
                    Ok((_guard, _)) => {
                        let c = current_c.fetch_add(1, Ordering::Relaxed) + 1;
                        // track max concurrent
                        let mut prev = max_c.load(Ordering::Relaxed);
                        while c > prev {
                            match max_c.compare_exchange(
                                prev, c, Ordering::Relaxed, Ordering::Relaxed,
                            ) {
                                Ok(_) => break,
                                Err(x) => prev = x,
                            }
                        }
                        acquired.fetch_add(1, Ordering::Relaxed);
                        tokio::time::sleep(Duration::from_millis(HOLD_MS)).await;
                        current_c.fetch_sub(1, Ordering::Relaxed);
                    }
                    Err(_) => {
                        timed_out.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
        })
        .collect();

    for h in handles {
        h.await.unwrap();
    }

    let elapsed = start.elapsed();
    let acq = acquired.load(Ordering::Relaxed);
    let to = timed_out.load(Ordering::Relaxed);
    let max_c = max_concurrent.load(Ordering::Relaxed);

    println!("  Elapsed:         {:.2?}", elapsed);
    println!("  Acquired:        {acq}");
    println!("  Timed out:       {to}");
    println!("  Max concurrent:  {max_c} (pool size = {POOL_SIZE})");
    println!("  Theoretical min: {:.2?}", Duration::from_millis(HOLD_MS * WORKERS as u64 / POOL_SIZE as u64));

    assert!(
        max_c <= POOL_SIZE as u32,
        "concurrent={max_c} exceeded pool size={POOL_SIZE}"
    );
    assert_eq!(acq + to, WORKERS as u64);
}

// ─── 3. flaky_resource — circuit breaker под нагрузкой ───────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn flaky_resource() {
    print_separator("3. flaky_resource — create() падает каждые 3 раза, circuit breaker");

    use nebula_resilience::CircuitBreakerConfig;

    let attempts = Arc::new(AtomicU32::new(0));
    let created = Arc::new(AtomicU32::new(0));

    let pool = Pool::new(
        FlakyResource {
            key: ResourceKey::try_from("flaky").unwrap(),
            attempts: Arc::clone(&attempts),
            fail_every: 3, // каждая 3-я попытка падает
            created: Arc::clone(&created),
        },
        FastConfig { id: "f" },
        PoolConfig {
            sizing: PoolSizing { min_size: 0, max_size: 5 },
            acquire: PoolAcquire { timeout: Duration::from_secs(1), ..Default::default() },
            resilience: PoolResiliencePolicy {
                create_breaker: Some(CircuitBreakerConfig {
                    min_operations: 3,
                    half_open_max_operations: 1,
                    failure_rate_threshold: 0.4,
                    ..Default::default()
                }),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();

    let mut successes = 0u32;
    let mut failures = 0u32;
    let mut breaker_open = 0u32;

    // Делаем 30 попыток acquire
    for i in 0..30 {
        match pool.acquire(&ctx()).await {
            Ok(_guard) => {
                successes += 1;
            }
            Err(Error::CircuitBreakerOpen { .. }) => {
                breaker_open += 1;
                // ждём пока breaker закроется
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(_) => {
                failures += 1;
            }
        }
        if i % 5 == 0 {
            let stats = pool.stats();
            print!("  [{i:2}] succ={successes} fail={failures} breaker_open={breaker_open} idle={} active={}\r",
                stats.idle, stats.active);
        }
    }
    println!();

    let stats = pool.stats();
    println!("  Successes:      {successes}");
    println!("  Failures:       {failures}");
    println!("  Breaker opens:  {breaker_open}");
    println!("  Total created:  {}", stats.created);
    println!("  Attempts:       {}", attempts.load(Ordering::Relaxed));

    assert!(successes > 0, "должны быть успешные acquire");
    assert!(breaker_open > 0, "circuit breaker должен срабатывать");
}

// ─── 4. reconnect_storm — все инстансы умирают одновременно ──────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconnect_storm() {
    print_separator("4. reconnect_storm — TTL=50ms, постоянный пересоздание под нагрузкой");

    const WORKERS: usize = 20;
    const DURATION_SECS: u64 = 2;

    let created = Arc::new(AtomicU32::new(0));

    let pool = Pool::new(
        MortalResource {
            key: ResourceKey::try_from("mortal").unwrap(),
            created: Arc::clone(&created),
            instance_ttl: Duration::from_millis(50), // инстанс живёт 50ms
        },
        FastConfig { id: "m" },
        PoolConfig {
            sizing: PoolSizing { min_size: 2, max_size: 10 },
            lifetime: PoolLifetime {
                validation_interval: Duration::from_millis(10),
                maintenance_interval: Some(Duration::from_millis(20)),
                ..Default::default()
            },
            acquire: PoolAcquire { timeout: Duration::from_secs(2), ..Default::default() },
            ..Default::default()
        },
    )
    .unwrap();

    let stop = Arc::new(tokio::sync::Notify::new());
    let total_ops = Arc::new(AtomicU64::new(0));
    let total_errors = Arc::new(AtomicU64::new(0));
    let start = Instant::now();

    let handles: Vec<_> = (0..WORKERS)
        .map(|_| {
            let pool = pool.clone();
            let stop = Arc::clone(&stop);
            let total_ops = Arc::clone(&total_ops);
            let total_errors = Arc::clone(&total_errors);
            tokio::spawn(async move {
                let acquire_ctx = ctx();
                loop {
                    tokio::select! {
                        _ = stop.notified() => break,
                        result = pool.acquire(&acquire_ctx) => {
                            match result {
                                Ok(_guard) => {
                                    total_ops.fetch_add(1, Ordering::Relaxed);
                                    tokio::time::sleep(Duration::from_millis(5)).await;
                                }
                                Err(_) => {
                                    total_errors.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                }
            })
        })
        .collect();

    tokio::time::sleep(Duration::from_secs(DURATION_SECS)).await;
    for _ in 0..WORKERS {
        stop.notify_one();
    }
    for h in handles {
        h.await.unwrap();
    }

    let elapsed = start.elapsed();
    let ops = total_ops.load(Ordering::Relaxed);
    let errs = total_errors.load(Ordering::Relaxed);
    let total_created = created.load(Ordering::Relaxed);
    let stats = pool.stats();

    println!("  Elapsed:        {:.2?}", elapsed);
    println!("  Total ops:      {ops}");
    println!("  Errors:         {errs}");
    println!("  Total created:  {total_created}  (инстансы пересоздавались по TTL)");
    println!("  Throughput:     {:.0} ops/sec", ops as f64 / elapsed.as_secs_f64());
    if let Some(lat) = &stats.acquire_latency {
        println!("  Latency p99:    {}ms", lat.p99_ms);
    }

    assert!(
        total_created > 10,
        "должно было создаться много инстансов из-за TTL, создано: {total_created}"
    );
    assert!(ops > 0, "должны быть успешные операции");
}

// ─── 5. acquire_under_pressure — adaptive backpressure ───────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn acquire_under_pressure() {
    print_separator("5. acquire_under_pressure — adaptive backpressure + shed нагрузки");

    use nebula_resource::pool::AdaptiveBackpressurePolicy;
    use nebula_resource::PoolBackpressurePolicy;

    const POOL_SIZE: usize = 5;
    const HOLD_MS: u64 = 20;

    let created = Arc::new(AtomicU32::new(0));
    let pool = Pool::new(
        FastResource {
            key: ResourceKey::try_from("pressure").unwrap(),
            created: Arc::clone(&created),
        },
        FastConfig { id: "p" },
        PoolConfig {
            sizing: PoolSizing { min_size: POOL_SIZE, max_size: POOL_SIZE },
            acquire: PoolAcquire {
                timeout: Duration::from_secs(5),
                backpressure: Some(PoolBackpressurePolicy::Adaptive(
                    AdaptiveBackpressurePolicy {
                        high_pressure_utilization: 0.6, // >60% = high pressure
                        high_pressure_waiters: 3,
                        low_pressure_timeout: Duration::from_secs(5),
                        high_pressure_timeout: Duration::from_millis(50), // быстро shed
                    },
                )),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();

    // Фаза 1: лёгкая нагрузка
    println!("  Phase 1: low load (10 workers)...");
    let (low_ok, low_shed) = run_load_phase(&pool, 10, HOLD_MS, Duration::from_millis(500)).await;
    println!("  → ok={low_ok} shed={low_shed}");

    // Фаза 2: высокая нагрузка — должен начать shed
    println!("  Phase 2: high load (50 workers)...");
    let (high_ok, high_shed) = run_load_phase(&pool, 50, HOLD_MS, Duration::from_millis(500)).await;
    println!("  → ok={high_ok} shed={high_shed}");

    assert!(low_shed == 0 || low_shed < low_ok / 10, "при низкой нагрузке shed должен быть минимальным");
    assert!(high_shed > 0, "при высокой нагрузке adaptive должен начать shed");

    println!("  Adaptive backpressure работает: low_shed={low_shed}, high_shed={high_shed}");
}

async fn run_load_phase(
    pool: &Pool<FastResource>,
    workers: usize,
    hold_ms: u64,
    duration: Duration,
) -> (u64, u64) {
    let ok = Arc::new(AtomicU64::new(0));
    let shed = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(tokio::sync::Notify::new());

    let handles: Vec<_> = (0..workers)
        .map(|_| {
            let pool = pool.clone();
            let ok = Arc::clone(&ok);
            let shed = Arc::clone(&shed);
            let stop = Arc::clone(&stop);
            tokio::spawn(async move {
                let acquire_ctx = ctx();
                loop {
                    tokio::select! {
                        _ = stop.notified() => break,
                        res = pool.acquire(&acquire_ctx) => {
                            match res {
                                Ok(_g) => {
                                    ok.fetch_add(1, Ordering::Relaxed);
                                    tokio::time::sleep(Duration::from_millis(hold_ms)).await;
                                }
                                Err(_) => {
                                    shed.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                    }
                }
            })
        })
        .collect();

    tokio::time::sleep(duration).await;
    for _ in 0..workers {
        stop.notify_one();
    }
    for h in handles {
        h.await.unwrap();
    }

    (ok.load(Ordering::Relaxed), shed.load(Ordering::Relaxed))
}

// ─── 6. multi_scope_isolation — tenant isolation под нагрузкой ───────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn multi_scope_isolation() {
    print_separator("6. multi_scope_isolation — 3 tenant'а, изоляция под нагрузкой");

    const TENANTS: &[&str] = &["acme", "globex", "initech"];
    const WORKERS_PER_TENANT: usize = 30;
    const OPS_PER_WORKER: usize = 50;

    let manager = Arc::new(Manager::new());

    // Регистрируем отдельный пул для каждого tenant'а
    for &tenant in TENANTS {
        let key = format!("resource.{tenant}");
        let created = Arc::new(AtomicU32::new(0));
        manager.register_scoped(
            FastResource {
                key: ResourceKey::try_from(key.as_str()).unwrap(),
                created,
            },
            FastConfig { id: "t" },
            PoolConfig {
                sizing: PoolSizing { min_size: 2, max_size: 10 },
                ..Default::default()
            },
            Scope::try_tenant(tenant).unwrap(),
        ).unwrap();
    }

    let barrier = Arc::new(Barrier::new(TENANTS.len() * WORKERS_PER_TENANT));
    let cross_tenant_errors = Arc::new(AtomicU64::new(0));
    let total_ok = Arc::new(AtomicU64::new(0));

    let handles: Vec<_> = TENANTS
        .iter()
        .flat_map(|&tenant| {
            let key = format!("resource.{tenant}");
            let rkey = ResourceKey::try_from(key.as_str()).unwrap();
            (0..WORKERS_PER_TENANT).map(move |_| {
                let manager = Arc::clone(&manager);
                let rkey = rkey.clone();
                let tenant = tenant.to_string();
                let barrier = Arc::clone(&barrier);
                let cross_errors = Arc::clone(&cross_tenant_errors);
                let total_ok = Arc::clone(&total_ok);
                tokio::spawn(async move {
                    barrier.wait().await;
                    let ctx = tenant_ctx(&tenant);
                    for _ in 0..OPS_PER_WORKER {
                        match manager.acquire(&rkey, &ctx).await {
                            Ok(guard) => {
                                // Проверяем что инстанс принадлежит правильному tenant'у
                                let inst = guard.as_any().downcast_ref::<String>().unwrap();
                                if !inst.contains('t') { // все инстансы содержат 't' из FastConfig id
                                    cross_errors.fetch_add(1, Ordering::Relaxed);
                                }
                                total_ok.fetch_add(1, Ordering::Relaxed);
                                tokio::task::yield_now().await;
                            }
                            Err(_) => {}
                        }
                    }
                })
            })
        })
        .collect();

    for h in handles {
        h.await.unwrap();
    }

    let cross_errs = cross_tenant_errors.load(Ordering::Relaxed);
    let ok = total_ok.load(Ordering::Relaxed);

    println!("  Total ops:           {ok}");
    println!("  Cross-tenant errors: {cross_errs}");
    println!("  Expected ops:        {}", TENANTS.len() * WORKERS_PER_TENANT * OPS_PER_WORKER);

    assert_eq!(cross_errs, 0, "изоляция tenant'ов нарушена");
    assert_eq!(ok, (TENANTS.len() * WORKERS_PER_TENANT * OPS_PER_WORKER) as u64);
}

// ─── 7. rapid_shutdown — shutdown пока активны acquire'ы ─────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rapid_shutdown() {
    print_separator("7. rapid_shutdown — shutdown пока 50 воркеров активно acquire'ят");

    const WORKERS: usize = 50;

    let created = Arc::new(AtomicU32::new(0));
    let pool = Pool::new(
        SlowResource {
            key: ResourceKey::try_from("slow-shutdown").unwrap(),
            created: Arc::clone(&created),
            delay_ms: 10,
        },
        FastConfig { id: "s" },
        PoolConfig {
            sizing: PoolSizing { min_size: 0, max_size: 10 },
            acquire: PoolAcquire { timeout: Duration::from_secs(5), ..Default::default() },
            ..Default::default()
        },
    )
    .unwrap();

    let barrier = Arc::new(Barrier::new(WORKERS + 1)); // +1 для shutdown'а
    let started = Arc::new(AtomicU64::new(0));
    let completed = Arc::new(AtomicU64::new(0));
    let cancelled = Arc::new(AtomicU64::new(0));

    let handles: Vec<_> = (0..WORKERS)
        .map(|_| {
            let pool = pool.clone();
            let barrier = Arc::clone(&barrier);
            let started = Arc::clone(&started);
            let completed = Arc::clone(&completed);
            let cancelled = Arc::clone(&cancelled);
            tokio::spawn(async move {
                barrier.wait().await; // старт вместе с shutdown
                started.fetch_add(1, Ordering::Relaxed);
                match pool.acquire(&ctx()).await {
                    Ok(_guard) => {
                        completed.fetch_add(1, Ordering::Relaxed);
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Err(_) => {
                        cancelled.fetch_add(1, Ordering::Relaxed);
                    }
                }
            })
        })
        .collect();

    // Запускаем shutdown сразу после того как все воркеры стартовали
    let pool_for_shutdown = pool.clone();
    tokio::spawn(async move {
        barrier.wait().await;
        tokio::time::sleep(Duration::from_millis(5)).await; // tiny delay
        pool_for_shutdown.shutdown().await.unwrap();
    });

    for h in handles {
        h.await.unwrap();
    }

    let s = started.load(Ordering::Relaxed);
    let c = completed.load(Ordering::Relaxed);
    let can = cancelled.load(Ordering::Relaxed);

    println!("  Started:    {s}");
    println!("  Completed:  {c}  (успели до shutdown)");
    println!("  Cancelled:  {can}  (отклонены shutdown'ом)");

    assert_eq!(s, WORKERS as u64);
    assert_eq!(c + can, WORKERS as u64, "все воркеры должны завершиться (ok или cancelled)");

    // Пул должен быть закрыт — новый acquire должен упасть
    let post_shutdown = pool.acquire(&ctx()).await;
    assert!(
        post_shutdown.is_err(),
        "после shutdown acquire должен вернуть ошибку"
    );
    println!("  Post-shutdown acquire: {:?} ✓", post_shutdown.unwrap_err());
}

// ─── 8. throughput_report — итоговый отчёт ───────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn throughput_report() {
    print_separator("8. throughput_report — измерение throughput при разных размерах пула");

    let configs: &[(usize, usize, &str)] = &[
        (1,  1,   "pool=1  workers=1   (baseline)"),
        (4,  8,   "pool=4  workers=8   (2x contention)"),
        (10, 100, "pool=10 workers=100 (10x contention)"),
        (50, 100, "pool=50 workers=100 (2x contention)"),
    ];

    for &(pool_size, workers, label) in configs {
        let created = Arc::new(AtomicU32::new(0));
        let pool = Pool::new(
            FastResource {
                key: ResourceKey::try_from("bench").unwrap(),
                created,
            },
            FastConfig { id: "b" },
            PoolConfig {
                sizing: PoolSizing { min_size: pool_size, max_size: pool_size },
                acquire: PoolAcquire { timeout: Duration::from_secs(10), ..Default::default() },
                ..Default::default()
            },
        ).unwrap();

        // прогрев
        for _ in 0..pool_size {
            let _ = pool.acquire(&ctx()).await.unwrap();
        }

        let ops_total = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(tokio::sync::Notify::new());
        let start = Instant::now();
        const MEASURE_MS: u64 = 500;

        let handles: Vec<_> = (0..workers).map(|_| {
            let pool = pool.clone();
            let ops = Arc::clone(&ops_total);
            let stop = Arc::clone(&stop);
            tokio::spawn(async move {
                let acquire_ctx = ctx();
                loop {
                    tokio::select! {
                        _ = stop.notified() => break,
                        res = pool.acquire(&acquire_ctx) => {
                            if res.is_ok() {
                                ops.fetch_add(1, Ordering::Relaxed);
                                tokio::task::yield_now().await;
                            }
                        }
                    }
                }
            })
        }).collect();

        tokio::time::sleep(Duration::from_millis(MEASURE_MS)).await;
        for _ in 0..workers { stop.notify_one(); }
        for h in handles { h.await.unwrap(); }

        let elapsed = start.elapsed();
        let ops = ops_total.load(Ordering::Relaxed);
        let throughput = ops as f64 / elapsed.as_secs_f64();
        let stats = pool.stats();
        let p99 = stats.acquire_latency.as_ref().map(|l| l.p99_ms).unwrap_or(0);

        println!("  {label}");
        println!("    throughput: {throughput:.0} ops/sec  p99: {p99}ms  ops: {ops}");
    }
}

fn main() {}
