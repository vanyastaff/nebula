//! Poison behavior tests for `Poison<T>` and pool integration.

use std::time::Duration;

use nebula_core::{resource_key, ResourceKey};
use nebula_resource::context::Context;
use nebula_resource::error::Error;
use nebula_resource::poison::{Poison, PoisonError};
use nebula_resource::pool::{Pool, PoolConfig};
use nebula_resource::resource::{Config, Resource};
use nebula_resource::scope::Scope;
use nebula_resource::{ExecutionId, PoolAcquire, PoolSizing, WorkflowId};

#[test]
fn clean_poison_can_arm_and_disarm() {
    let mut state = Poison::new("int", 1_i32);
    let mut guard = state.check_and_arm().expect("must arm on clean state");
    *guard.data_mut() = 2;
    guard.disarm();
    assert!(!state.is_poisoned());
}

#[test]
fn dropping_guard_without_disarm_poisons() {
    let mut state = Poison::new("int", 1_i32);
    {
        let _guard = state.check_and_arm().expect("must arm");
    }
    assert!(state.is_poisoned());
}

#[test]
fn second_arm_on_poisoned_state_returns_error_with_label() {
    let mut state = Poison::new("counter", 0_i32);
    {
        let _guard = state.check_and_arm().expect("must arm");
    }

    let err = match state.check_and_arm() {
        Ok(_) => panic!("poisoned state must reject arm"),
        Err(err) => err,
    };
    match err {
        PoisonError::Poisoned { what, .. } => assert_eq!(what, "counter"),
    }
    assert!(err.to_string().contains("counter"));
}

#[derive(Debug, Clone)]
struct TestConfig;

impl Config for TestConfig {}

struct TestResource;

impl Resource for TestResource {
    type Config = TestConfig;
    type Instance = String;

    fn key(&self) -> ResourceKey {
        resource_key!("poison-test")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _ctx: &Context,
    ) -> nebula_resource::Result<String> {
        Ok("ok".to_string())
    }
}

fn ctx() -> Context {
    Context::new(Scope::Global, WorkflowId::new(), ExecutionId::new())
}

#[test]
fn reentrant_arm_poisons_state() {
    // Leave the Poison in Armed state by forgetting the guard (neither disarm
    // nor drop runs). Then re-entering an Armed state must not succeed silently:
    // the implementation must poison the value and return Err.
    let mut state = Poison::new("worker", 0_i32);
    let guard = state.check_and_arm().expect("first arm must succeed");
    // Forget the guard: Drop won't fire, state remains Armed.
    std::mem::forget(guard);
    // Re-arm while Armed → must fail and transition to Poisoned.
    let err = match state.check_and_arm() {
        Ok(_) => panic!("re-entering an armed state must not succeed"),
        Err(e) => e,
    };
    assert!(matches!(err, PoisonError::Poisoned { .. }));
    assert!(state.is_poisoned(), "state must be poisoned after re-arm");
}

#[tokio::test]
async fn pool_acquire_returns_internal_when_state_poisoned() {
    let pool = Pool::new(
        TestResource,
        TestConfig,
        PoolConfig {
            sizing: PoolSizing { min_size: 0, max_size: 1 },
            acquire: PoolAcquire { timeout: Duration::from_secs(1), ..Default::default() },
            ..Default::default()
        },
    )
    .expect("pool must be created");

    pool.poison_for_test();

    let err = pool.acquire(&ctx()).await.expect_err("acquire should fail");
    assert!(matches!(err, Error::Internal { .. }));
    assert!(err.to_string().contains("pool state poisoned"));
}
