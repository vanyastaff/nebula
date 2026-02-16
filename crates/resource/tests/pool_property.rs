//! Property tests for pool acquire/release invariants.
//!
//! T015: After N acquire/release cycles, `stats.active + stats.idle <= max_size`
//! always holds.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::{Pool, PoolConfig, PoolStrategy};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// Test resource
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
struct TestConfig;

impl Config for TestConfig {}

struct CountingResource {
    counter: AtomicU64,
}

impl CountingResource {
    fn new() -> Self {
        Self {
            counter: AtomicU64::new(0),
        }
    }
}

impl Resource for CountingResource {
    type Config = TestConfig;
    type Instance = u64;

    fn id(&self) -> &str {
        "counting"
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<u64> {
        Ok(self.counter.fetch_add(1, Ordering::SeqCst))
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, "wf", "ex")
}

// ---------------------------------------------------------------------------
// Property: active + idle <= max_size after arbitrary acquire/release cycles
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(30))]

    #[test]
    fn pool_invariant_active_plus_idle_le_max_size(
        max_size in 1usize..8,
        ops in proptest::collection::vec(prop_oneof![Just(true), Just(false)], 1..30),
        strategy in prop_oneof![Just(PoolStrategy::Fifo), Just(PoolStrategy::Lifo)],
    ) {
        // Run the async property test on the Tokio runtime.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async {
            let pool_config = PoolConfig {
                min_size: 0,
                max_size,
                acquire_timeout: Duration::from_millis(50),
                strategy,
                ..Default::default()
            };
            let pool = Pool::new(CountingResource::new(), TestConfig, pool_config).unwrap();
            let mut guards = Vec::new();

            for op_is_acquire in &ops {
                if *op_is_acquire {
                    // Acquire (may fail if pool is exhausted -- that is fine)
                    if let Ok(guard) = pool.acquire(&ctx()).await {
                        guards.push(guard);
                    }
                } else if !guards.is_empty() {
                    // Release one
                    guards.pop();
                    // Give the spawned return-to-pool task time to run
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }

                // INVARIANT: active + idle <= max_size
                let stats = pool.stats();
                prop_assert!(
                    stats.active + stats.idle <= max_size,
                    "invariant violated: active={} + idle={} = {} > max_size={}",
                    stats.active, stats.idle, stats.active + stats.idle, max_size,
                );
            }

            // Drop all remaining guards and verify
            drop(guards);
            tokio::time::sleep(Duration::from_millis(50)).await;

            let final_stats = pool.stats();
            prop_assert!(
                final_stats.active + final_stats.idle <= max_size,
                "final invariant violated: active={} + idle={} > max_size={}",
                final_stats.active, final_stats.idle, max_size,
            );
            prop_assert_eq!(
                final_stats.active, 0,
                "all guards dropped, active should be 0"
            );

            Ok(())
        })?;
    }
}

/// Deterministic test: rapid acquire-release cycles maintain pool invariants.
#[tokio::test]
async fn rapid_acquire_release_preserves_invariants() {
    let max_size = 4;
    let pool_config = PoolConfig {
        min_size: 0,
        max_size,
        acquire_timeout: Duration::from_millis(200),
        ..Default::default()
    };
    let pool = Pool::new(CountingResource::new(), TestConfig, pool_config).unwrap();

    for _ in 0..20 {
        let g = pool.acquire(&ctx()).await.unwrap();
        drop(g);
        tokio::time::sleep(Duration::from_millis(10)).await;

        let stats = pool.stats();
        assert!(
            stats.active + stats.idle <= max_size,
            "invariant violated during rapid cycling"
        );
    }
}

/// Verify that total_releases == total_acquisitions after all guards are dropped.
#[tokio::test]
async fn acquisitions_equal_releases_after_cleanup() {
    let pool_config = PoolConfig {
        min_size: 0,
        max_size: 3,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    };
    let pool = Pool::new(CountingResource::new(), TestConfig, pool_config).unwrap();

    let mut guards = Vec::new();
    for _ in 0..3 {
        guards.push(pool.acquire(&ctx()).await.unwrap());
    }

    let stats = pool.stats();
    assert_eq!(stats.total_acquisitions, 3);
    assert_eq!(stats.active, 3);

    drop(guards);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let stats = pool.stats();
    assert_eq!(stats.total_releases, 3);
    assert_eq!(stats.active, 0);
    assert_eq!(stats.total_acquisitions, stats.total_releases);
}
