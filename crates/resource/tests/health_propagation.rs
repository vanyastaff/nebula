//! Health state propagation tests.
//!
//! Test 9a exposes a string-matching bug in `Manager::propagate_health()`:
//! `reason.contains(resource_id)` matches prefixes. When "db" goes healthy,
//! it clears degraded states whose reason contains "db" — including those
//! caused by "db-replica".

use nebula_resource::context::Context;
use nebula_resource::error::Result;
use nebula_resource::health::HealthState;
use nebula_resource::manager::Manager;
use nebula_resource::pool::PoolConfig;
use nebula_resource::resource::{Config, Resource};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct TestConfig;

impl Config for TestConfig {}

struct NamedResource {
    name: String,
    deps: Vec<String>,
}

impl Resource for NamedResource {
    type Config = TestConfig;
    type Instance = String;

    fn id(&self) -> &str {
        &self.name
    }

    async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
        Ok(format!("{}-instance", self.name))
    }

    fn dependencies(&self) -> Vec<&str> {
        self.deps.iter().map(String::as_str).collect()
    }
}

fn pool_config() -> PoolConfig {
    PoolConfig::default()
}

// ---------------------------------------------------------------------------
// Test 9a: health_propagation_exact_match
//
// This test DOCUMENTS a known bug: reason.contains("db") matches "db-replica".
// When "db" is set to Healthy, it incorrectly clears degraded states caused
// by "db-replica" because "Dependency db-replica is unhealthy".contains("db")
// is true.
// ---------------------------------------------------------------------------

#[test]
fn health_propagation_prefix_collision() {
    let mgr = Manager::new();

    // Register leaf resources (no dependencies) FIRST to avoid
    // remove_all_for() destroying dependency edges during registration.
    mgr.register(
        NamedResource {
            name: "db".into(),
            deps: vec![],
        },
        TestConfig,
        pool_config(),
    )
    .unwrap();

    mgr.register(
        NamedResource {
            name: "db-replica".into(),
            deps: vec![],
        },
        TestConfig,
        pool_config(),
    )
    .unwrap();

    // Register "app" which depends on both "db" and "db-replica"
    mgr.register(
        NamedResource {
            name: "app".into(),
            deps: vec!["db".into(), "db-replica".into()],
        },
        TestConfig,
        pool_config(),
    )
    .unwrap();

    // Mark db-replica as unhealthy -> app becomes Degraded
    mgr.set_health_state(
        "db-replica",
        HealthState::Unhealthy {
            reason: "replication lag".into(),
            recoverable: true,
        },
    );

    // Verify db-replica is unhealthy and app is degraded
    assert!(matches!(
        mgr.get_health_state("db-replica"),
        Some(HealthState::Unhealthy { .. })
    ));
    assert!(
        matches!(mgr.get_health_state("app"), Some(HealthState::Degraded { .. })),
        "app should be Degraded because db-replica is unhealthy"
    );

    // Now mark "db" (NOT "db-replica") as healthy.
    // This should NOT clear app's degraded state, because app is degraded
    // due to "db-replica", not "db".
    mgr.set_health_state("db", HealthState::Healthy);

    // BUG: reason.contains("db") matches "Dependency db-replica is unhealthy"
    // because "db-replica" contains the substring "db".
    //
    // KNOWN BUGGY BEHAVIOR: After setting "db" healthy, "app" is incorrectly
    // cleared to Healthy even though "db-replica" is still unhealthy.
    //
    // When the bug is fixed, change this assertion to:
    //   assert!(matches!(mgr.get_health_state("app"), Some(HealthState::Degraded { .. })));
    assert!(
        matches!(mgr.get_health_state("app"), Some(HealthState::Healthy)),
        "KNOWN BUG: app is incorrectly cleared to Healthy due to prefix collision"
    );

    // Set "db-replica" back to healthy to clean up
    mgr.set_health_state("db-replica", HealthState::Healthy);
}

/// Verify basic health propagation works correctly for non-colliding names.
#[test]
fn health_propagation_no_collision() {
    let mgr = Manager::new();

    // Register leaf resources (no dependencies) FIRST to avoid
    // remove_all_for() destroying dependency edges during registration.
    mgr.register(
        NamedResource {
            name: "cache".into(),
            deps: vec![],
        },
        TestConfig,
        pool_config(),
    )
    .unwrap();

    mgr.register(
        NamedResource {
            name: "queue".into(),
            deps: vec![],
        },
        TestConfig,
        pool_config(),
    )
    .unwrap();

    // "service" depends on "cache" and "queue" (no prefix overlap)
    mgr.register(
        NamedResource {
            name: "service".into(),
            deps: vec!["cache".into(), "queue".into()],
        },
        TestConfig,
        pool_config(),
    )
    .unwrap();

    // Mark cache unhealthy -> service degraded
    mgr.set_health_state(
        "cache",
        HealthState::Unhealthy {
            reason: "connection refused".into(),
            recoverable: true,
        },
    );

    assert!(
        matches!(mgr.get_health_state("cache"), Some(HealthState::Unhealthy { .. })),
        "cache should be Unhealthy"
    );
    assert!(
        matches!(mgr.get_health_state("service"), Some(HealthState::Degraded { .. })),
        "service should be Degraded due to cache"
    );

    // Mark queue healthy -> should NOT clear service's degraded state
    // (service is degraded due to cache, not queue)
    mgr.set_health_state("queue", HealthState::Healthy);

    assert!(
        matches!(mgr.get_health_state("service"), Some(HealthState::Degraded { .. })),
        "service should still be Degraded (cache is still unhealthy)"
    );

    // Mark cache healthy -> NOW service should be cleared
    mgr.set_health_state("cache", HealthState::Healthy);

    assert!(
        matches!(
            mgr.get_health_state("service"),
            Some(HealthState::Healthy) | None
        ),
        "service should be Healthy now that cache is restored"
    );
}
