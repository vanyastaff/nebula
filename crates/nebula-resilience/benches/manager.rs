//! Benchmarks for ResilienceManager
//!
//! Measures the impact of our optimizations:
//! - Arc<ResiliencePolicy> vs owned policy (Sprint 2 optimization)
//! - DashMap vs Arc<RwLock<HashMap>> (Sprint 2 optimization)
//! - execute() overhead with different policy combinations
//! - Concurrent access patterns

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use nebula_resilience::{ResilienceError, ResilienceManager, ResiliencePolicy};
use std::sync::Arc;
use std::time::Duration;

fn manager_policy_lookup(c: &mut Criterion) {
    let mut group = c.benchmark_group("manager/policy_lookup");

    // Benchmark: Get policy (DashMap lock-free read)
    group.bench_function("get_policy_registered", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ResilienceManager::with_defaults();

        rt.block_on(async {
            manager.register_service("test-api", ResiliencePolicy::default());
        });

        b.to_async(&rt).iter(|| async {
            // This is now lock-free with DashMap!
            let result = manager
                .execute("test-api", "operation", || async {
                    Ok::<_, ResilienceError>(black_box(42))
                })
                .await;
            black_box(result)
        });
    });

    // Benchmark: Get default policy (Arc clone)
    group.bench_function("get_policy_default", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ResilienceManager::with_defaults();

        b.to_async(&rt).iter(|| async {
            let result = manager
                .execute("unregistered-service", "operation", || async {
                    Ok::<_, ResilienceError>(black_box(42))
                })
                .await;
            black_box(result)
        });
    });

    group.finish();
}

fn manager_service_registration(c: &mut Criterion) {
    let mut group = c.benchmark_group("manager/registration");

    // Benchmark: Register service (DashMap write)
    group.bench_function("register_service", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = Arc::new(ResilienceManager::with_defaults());
        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));

        b.to_async(&rt).iter(|| {
            let manager = Arc::clone(&manager);
            let counter = Arc::clone(&counter);
            async move {
                let count = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let service_name = format!("service-{}", count);
                manager.register_service(service_name, ResiliencePolicy::default());
            }
        });
    });

    // Benchmark: Unregister service (DashMap remove)
    group.bench_function("unregister_service", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ResilienceManager::with_defaults();

        b.iter_batched(
            || {
                let service_name = format!("service-{}", rand::random::<u32>());
                manager.register_service(&service_name, ResiliencePolicy::default());
                service_name
            },
            |service_name| {
                manager.unregister_service(&service_name);
            },
            criterion::BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn manager_execute_overhead(c: &mut Criterion) {
    let mut group = c.benchmark_group("manager/execute_overhead");

    // Benchmark: Execute with minimal policy (no patterns)
    group.bench_function("no_patterns", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ResilienceManager::with_defaults();

        manager.register_service("api", ResiliencePolicy::default());

        b.to_async(&rt).iter(|| async {
            let result = manager
                .execute("api", "operation", || async {
                    Ok::<_, ResilienceError>(black_box(42))
                })
                .await;
            black_box(result)
        });
    });

    // Benchmark: Execute with timeout only
    group.bench_function("with_timeout", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ResilienceManager::with_defaults();

        let policy = ResiliencePolicy::default().with_timeout(Duration::from_secs(5));
        manager.register_service("api", policy);

        b.to_async(&rt).iter(|| async {
            let result = manager
                .execute("api", "operation", || async {
                    Ok::<_, ResilienceError>(black_box(42))
                })
                .await;
            black_box(result)
        });
    });

    // Benchmark: Execute with full policy (timeout + retry + circuit breaker)
    group.bench_function("full_policy", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ResilienceManager::with_defaults();

        let retry_config = nebula_resilience::patterns::retry::RetryConfig::new(
            nebula_resilience::patterns::retry::FixedDelay::<100>::default(),
            nebula_resilience::patterns::retry::ConservativeCondition::<3>::new(),
        );
        let retry_strategy =
            nebula_resilience::patterns::retry::RetryStrategy::new(retry_config).unwrap();
        let policy = ResiliencePolicy::default()
            .with_timeout(Duration::from_secs(5))
            .with_retry(retry_strategy);
        manager.register_service("api", policy);

        b.to_async(&rt).iter(|| async {
            let result = manager
                .execute("api", "operation", || async {
                    Ok::<_, ResilienceError>(black_box(42))
                })
                .await;
            black_box(result)
        });
    });

    group.finish();
}

fn manager_concurrent_access(c: &mut Criterion) {
    let mut group = c.benchmark_group("manager/concurrent_access");
    group.sample_size(50); // Reduce sample size for concurrency tests

    // Benchmark: Concurrent reads from multiple tasks (DashMap shines here!)
    for &num_tasks in &[10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("concurrent_execute", num_tasks),
            &num_tasks,
            |b, &num_tasks| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let manager = Arc::new(ResilienceManager::with_defaults());

                manager.register_service("api", ResiliencePolicy::default());

                b.to_async(&rt).iter(|| {
                    let manager = Arc::clone(&manager);
                    async move {
                        let mut handles = vec![];
                        for _ in 0..num_tasks {
                            let manager = Arc::clone(&manager);
                            let handle = tokio::spawn(async move {
                                manager
                                    .execute("api", "op", || async { Ok::<_, ResilienceError>(42) })
                                    .await
                            });
                            handles.push(handle);
                        }

                        for handle in handles {
                            let _ = handle.await;
                        }
                    }
                });
            },
        );
    }

    group.finish();
}

fn manager_metrics_collection(c: &mut Criterion) {
    let mut group = c.benchmark_group("manager/metrics");

    // Benchmark: get_metrics (DashMap lock-free reads)
    group.bench_function("get_metrics_single", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ResilienceManager::with_defaults();

        manager.register_service("api", ResiliencePolicy::default());

        b.to_async(&rt)
            .iter(|| async { black_box(manager.get_metrics("api").await) });
    });

    // Benchmark: get_all_metrics
    for &num_services in &[10, 50, 100] {
        group.bench_with_input(
            BenchmarkId::new("get_all_metrics", num_services),
            &num_services,
            |b, &num_services| {
                let rt = tokio::runtime::Runtime::new().unwrap();
                let manager = ResilienceManager::with_defaults();

                for i in 0..num_services {
                    manager.register_service(format!("service-{}", i), ResiliencePolicy::default());
                }

                b.to_async(&rt)
                    .iter(|| async { black_box(manager.get_all_metrics().await) });
            },
        );
    }

    // Benchmark: list_services (now synchronous!)
    group.bench_function("list_services_100", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let manager = ResilienceManager::with_defaults();

        for i in 0..100 {
            manager.register_service(format!("service-{}", i), ResiliencePolicy::default());
        }

        b.iter(|| {
            // No async needed with DashMap!
            black_box(manager.list_services())
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    manager_policy_lookup,
    manager_service_registration,
    manager_execute_overhead,
    manager_concurrent_access,
    manager_metrics_collection,
);

criterion_main!(benches);
