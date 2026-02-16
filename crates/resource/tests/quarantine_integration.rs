//! T052, T053, T055: Quarantine integration tests.
//!
//! Verifies:
//! T052: Quarantined resource causes acquire to fail with retryable error.
//! T053: Releasing quarantine allows acquire to succeed again.
//! T055: Recovery after quarantine release returns resource to pool.
//! Also tests QuarantineManager standalone: quarantine, is_quarantined, release.
//! Also tests RecoveryStrategy exponential backoff calculation.

use std::time::Duration;

use nebula_resource::Manager;
use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::pool::PoolConfig;
use nebula_resource::quarantine::{
    QuarantineConfig, QuarantineManager, QuarantineReason, RecoveryStrategy,
};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Deserialize)]
struct TestConfig;
impl Config for TestConfig {}

struct NamedResource {
    name: &'static str,
}

impl Resource for NamedResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        self.name
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Ok(format!("{}-instance", self.name))
    }
}

fn pool_cfg() -> PoolConfig {
    PoolConfig {
        min_size: 0,
        max_size: 4,
        acquire_timeout: Duration::from_secs(1),
        ..Default::default()
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, "wf", "ex")
}

// ---------------------------------------------------------------------------
// T052: Quarantined resource — acquire fails with retryable error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn quarantined_resource_acquire_fails_retryable() {
    let mgr = Manager::new();
    mgr.register(NamedResource { name: "db" }, TestConfig, pool_cfg())
        .unwrap();

    // Quarantine the resource through the manager's quarantine manager
    let quarantined = mgr.quarantine().quarantine(
        "db",
        QuarantineReason::HealthCheckFailed {
            consecutive_failures: 3,
        },
    );
    assert!(quarantined, "resource should be newly quarantined");

    // Acquire should fail
    let result = mgr.acquire("db", &ctx()).await;
    let err = result.expect_err("acquire should fail for quarantined resource");

    // Error should mention quarantine
    assert!(
        err.to_string().contains("quarantined"),
        "error should mention quarantine, got: {err}"
    );

    // Error should be retryable (quarantine is temporary)
    assert!(
        err.is_retryable(),
        "quarantine error should be retryable, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// T053: Release quarantine — acquire works again
// ---------------------------------------------------------------------------

#[tokio::test]
async fn quarantine_release_allows_acquire() {
    let mgr = Manager::new();
    mgr.register(NamedResource { name: "db" }, TestConfig, pool_cfg())
        .unwrap();

    // Quarantine
    mgr.quarantine().quarantine(
        "db",
        QuarantineReason::ManualQuarantine {
            reason: "maintenance".into(),
        },
    );
    assert!(mgr.acquire("db", &ctx()).await.is_err());

    // Release quarantine
    let entry = mgr.quarantine().release("db");
    assert!(entry.is_some(), "should have been quarantined");

    // Acquire should now succeed
    let guard = mgr.acquire("db", &ctx()).await;
    assert!(
        guard.is_ok(),
        "acquire should succeed after quarantine release, got: {:?}",
        guard.err()
    );
}

// ---------------------------------------------------------------------------
// T055: Recovery after quarantine release returns resource to pool
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn recovery_after_quarantine_release_returns_to_pool() {
    let mgr = Manager::new();
    mgr.register(NamedResource { name: "cache" }, TestConfig, pool_cfg())
        .unwrap();

    // Pre-quarantine: acquire and release to populate pool
    {
        let _g = mgr.acquire("cache", &ctx()).await.unwrap();
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Quarantine
    mgr.quarantine().quarantine(
        "cache",
        QuarantineReason::HealthCheckFailed {
            consecutive_failures: 5,
        },
    );

    // Verify blocked
    assert!(mgr.acquire("cache", &ctx()).await.is_err());

    // Release and verify pool works normally
    mgr.quarantine().release("cache");

    // Acquire twice to verify pool recycling still works
    let g1 = mgr.acquire("cache", &ctx()).await.unwrap();
    let val = g1.as_any().downcast_ref::<String>().unwrap();
    assert_eq!(val, "cache-instance");
    drop(g1);
    tokio::time::sleep(Duration::from_millis(50)).await;

    let g2 = mgr.acquire("cache", &ctx()).await.unwrap();
    let val2 = g2.as_any().downcast_ref::<String>().unwrap();
    assert_eq!(val2, "cache-instance");
}

// ---------------------------------------------------------------------------
// QuarantineManager standalone: quarantine, is_quarantined, release
// ---------------------------------------------------------------------------

#[test]
fn standalone_quarantine_lifecycle() {
    let qm = QuarantineManager::default();

    assert!(!qm.is_quarantined("db"));
    assert!(qm.is_empty());

    // Quarantine
    let added = qm.quarantine(
        "db",
        QuarantineReason::HealthCheckFailed {
            consecutive_failures: 3,
        },
    );
    assert!(added);
    assert!(qm.is_quarantined("db"));
    assert_eq!(qm.len(), 1);

    // Check entry details
    let entry = qm.get("db").unwrap();
    assert_eq!(entry.resource_id, "db");
    assert_eq!(entry.recovery_attempts, 0);
    assert!(!entry.is_exhausted());

    // Duplicate quarantine is no-op
    let added_again = qm.quarantine(
        "db",
        QuarantineReason::HealthCheckFailed {
            consecutive_failures: 10,
        },
    );
    assert!(!added_again);
    assert_eq!(qm.len(), 1);

    // Release
    let released = qm.release("db");
    assert!(released.is_some());
    assert!(!qm.is_quarantined("db"));
    assert!(qm.is_empty());

    // Release again is None
    assert!(qm.release("db").is_none());
}

#[test]
fn standalone_quarantine_multiple_resources() {
    let qm = QuarantineManager::default();

    qm.quarantine(
        "db",
        QuarantineReason::HealthCheckFailed {
            consecutive_failures: 3,
        },
    );
    qm.quarantine(
        "cache",
        QuarantineReason::ManualQuarantine {
            reason: "maintenance".into(),
        },
    );

    assert_eq!(qm.len(), 2);
    assert!(qm.is_quarantined("db"));
    assert!(qm.is_quarantined("cache"));
    assert!(!qm.is_quarantined("other"));

    let mut ids = qm.quarantined_ids();
    ids.sort();
    assert_eq!(ids, vec!["cache", "db"]);

    // Release one
    qm.release("db");
    assert_eq!(qm.len(), 1);
    assert!(!qm.is_quarantined("db"));
    assert!(qm.is_quarantined("cache"));
}

// ---------------------------------------------------------------------------
// RecoveryStrategy: exponential backoff calculation
// ---------------------------------------------------------------------------

#[test]
fn recovery_strategy_exponential_backoff() {
    let strategy = RecoveryStrategy::default();
    // base_delay=1s, multiplier=2.0, max_delay=60s

    // Attempt 1: 1 * 2^0 = 1s
    assert_eq!(strategy.delay_for(1), Duration::from_secs(1));
    // Attempt 2: 1 * 2^1 = 2s
    assert_eq!(strategy.delay_for(2), Duration::from_secs(2));
    // Attempt 3: 1 * 2^2 = 4s
    assert_eq!(strategy.delay_for(3), Duration::from_secs(4));
    // Attempt 4: 1 * 2^3 = 8s
    assert_eq!(strategy.delay_for(4), Duration::from_secs(8));
    // Attempt 5: 1 * 2^4 = 16s
    assert_eq!(strategy.delay_for(5), Duration::from_secs(16));
    // Attempt 6: 1 * 2^5 = 32s
    assert_eq!(strategy.delay_for(6), Duration::from_secs(32));
    // Attempt 7: 1 * 2^6 = 64s, capped to 60s
    assert_eq!(strategy.delay_for(7), Duration::from_secs(60));
}

#[test]
fn recovery_strategy_custom_params() {
    let strategy = RecoveryStrategy {
        base_delay: Duration::from_millis(500),
        max_delay: Duration::from_secs(5),
        multiplier: 3.0,
    };

    // Attempt 1: 0.5 * 3^0 = 0.5s
    assert_eq!(strategy.delay_for(1), Duration::from_millis(500));
    // Attempt 2: 0.5 * 3^1 = 1.5s
    assert_eq!(strategy.delay_for(2), Duration::from_millis(1500));
    // Attempt 3: 0.5 * 3^2 = 4.5s
    assert_eq!(strategy.delay_for(3), Duration::from_millis(4500));
    // Attempt 4: 0.5 * 3^3 = 13.5s, capped to 5s
    assert_eq!(strategy.delay_for(4), Duration::from_secs(5));
}

#[test]
fn recovery_strategy_caps_at_max() {
    let strategy = RecoveryStrategy {
        base_delay: Duration::from_secs(1),
        max_delay: Duration::from_secs(10),
        multiplier: 2.0,
    };

    // 1 * 2^5 = 32, capped to 10
    assert_eq!(strategy.delay_for(6), Duration::from_secs(10));
    // Very high attempt still caps
    assert_eq!(strategy.delay_for(100), Duration::from_secs(10));
}

// ---------------------------------------------------------------------------
// Recovery attempts tracking
// ---------------------------------------------------------------------------

#[test]
fn failed_recovery_increments_and_exhausts() {
    let config = QuarantineConfig {
        failure_threshold: 3,
        max_recovery_attempts: 3,
        recovery_strategy: RecoveryStrategy::default(),
    };
    let qm = QuarantineManager::new(config);

    qm.quarantine(
        "db",
        QuarantineReason::HealthCheckFailed {
            consecutive_failures: 3,
        },
    );

    // Attempt 1
    assert!(qm.record_failed_recovery("db"));
    let entry = qm.get("db").unwrap();
    assert_eq!(entry.recovery_attempts, 1);
    assert!(!entry.is_exhausted());
    assert!(entry.next_recovery_at.is_some());

    // Attempt 2
    assert!(qm.record_failed_recovery("db"));
    let entry = qm.get("db").unwrap();
    assert_eq!(entry.recovery_attempts, 2);
    assert!(!entry.is_exhausted());

    // Attempt 3 -- exhausted
    assert!(qm.record_failed_recovery("db"));
    let entry = qm.get("db").unwrap();
    assert_eq!(entry.recovery_attempts, 3);
    assert!(entry.is_exhausted());
    assert!(
        entry.next_recovery_at.is_none(),
        "exhausted entry should have no next recovery time"
    );
}

#[test]
fn record_failed_recovery_nonexistent_returns_false() {
    let qm = QuarantineManager::default();
    assert!(!qm.record_failed_recovery("nonexistent"));
}

// ---------------------------------------------------------------------------
// Manager + quarantine: scoped resource quarantine
// ---------------------------------------------------------------------------

#[tokio::test(start_paused = true)]
async fn scoped_resource_quarantine_blocks_acquire() {
    let mgr = Manager::new();

    mgr.register_scoped(
        NamedResource { name: "tenant-db" },
        TestConfig,
        pool_cfg(),
        Scope::tenant("A"),
    )
    .unwrap();

    let ctx_a = Context::new(Scope::tenant("A"), "wf1", "ex1");

    // Verify it works before quarantine
    let g = mgr.acquire("tenant-db", &ctx_a).await.unwrap();
    drop(g);
    tokio::time::sleep(Duration::from_millis(30)).await;

    // Quarantine
    mgr.quarantine().quarantine(
        "tenant-db",
        QuarantineReason::HealthCheckFailed {
            consecutive_failures: 5,
        },
    );

    // Acquire should fail
    let err = mgr.acquire("tenant-db", &ctx_a).await.unwrap_err();
    assert!(err.to_string().contains("quarantined"));
    assert!(err.is_retryable());

    // Release and verify it works again
    mgr.quarantine().release("tenant-db");
    let g2 = mgr.acquire("tenant-db", &ctx_a).await.unwrap();
    assert!(g2.as_any().downcast_ref::<String>().is_some());
}
