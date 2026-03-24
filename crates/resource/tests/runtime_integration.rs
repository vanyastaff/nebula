//! Integration contracts for `nebula-runtime` orchestration.
//!
//! These tests capture lifecycle and failure semantics runtime depends on.

use nebula_core::{ResourceKey, resource_key};
use nebula_resource::quarantine::QuarantineReason;

mod scope_helpers;
use nebula_resource::{
    Context, ErrorCategory, ExecutionId, HealthState, Manager, PoolConfig, Resource, WorkflowId,
};
use scope_helpers::*;

#[derive(Debug, Clone)]
struct TestConfig;

impl nebula_resource::Config for TestConfig {}

struct RuntimeResourceA;
struct RuntimeResourceB;

impl Resource for RuntimeResourceA {
    type Config = TestConfig;
    type Instance = &'static str;

    fn key(&self) -> ResourceKey {
        resource_key!("runtime-a")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _ctx: &Context,
    ) -> nebula_resource::Result<Self::Instance> {
        Ok("a")
    }
}

impl Resource for RuntimeResourceB {
    type Config = TestConfig;
    type Instance = &'static str;

    fn key(&self) -> ResourceKey {
        resource_key!("runtime-b")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _ctx: &Context,
    ) -> nebula_resource::Result<Self::Instance> {
        Ok("b")
    }
}

fn workflow_ctx(workflow_id: &str) -> Context {
    Context::new(
        scope_workflow(workflow_id),
        WorkflowId::new(),
        ExecutionId::new(),
    )
}

#[tokio::test]
async fn runtime_shutdown_scope_only_drains_target_scope_contract() {
    let manager = Manager::new();
    manager
        .register_scoped(
            RuntimeResourceA,
            TestConfig,
            PoolConfig::default(),
            scope_workflow("wf-a"),
        )
        .expect("resource a registered");
    manager
        .register_scoped(
            RuntimeResourceB,
            TestConfig,
            PoolConfig::default(),
            scope_workflow("wf-b"),
        )
        .expect("resource b registered");

    let key_a = resource_key!("runtime-a");
    let key_b = resource_key!("runtime-b");

    manager
        .shutdown_scope(&scope_workflow("wf-a"))
        .await
        .expect("shutdown scope succeeds");

    let err = manager
        .acquire(&key_a, &workflow_ctx("wf-a"))
        .await
        .expect_err("resource in shut down scope must be unavailable");
    assert_eq!(err.category(), ErrorCategory::Fatal);

    let guard = manager
        .acquire(&key_b, &workflow_ctx("wf-b"))
        .await
        .expect("resource outside shutdown scope remains available");
    assert_eq!(guard.as_any().downcast_ref::<&'static str>(), Some(&"b"));
}

#[tokio::test]
async fn runtime_manual_quarantine_is_retryable_contract() {
    let manager = Manager::new();
    manager
        .register(RuntimeResourceA, TestConfig, PoolConfig::default())
        .expect("resource registered");

    let key = resource_key!("runtime-a");
    let quarantined = manager.quarantine().quarantine(
        key.as_ref(),
        QuarantineReason::ManualQuarantine {
            reason: "maintenance".to_string(),
        },
    );
    assert!(quarantined);

    let err = manager
        .acquire(&key, &workflow_ctx("wf-a"))
        .await
        .expect_err("quarantined resource must not be acquired");
    assert_eq!(err.category(), ErrorCategory::Retryable);
    assert!(err.is_retryable());
}

#[tokio::test]
async fn runtime_unhealthy_nonrecoverable_is_fatal_contract() {
    let manager = Manager::new();
    manager
        .register(RuntimeResourceA, TestConfig, PoolConfig::default())
        .expect("resource registered");

    let key = resource_key!("runtime-a");
    manager.set_health_state(
        &key,
        HealthState::Unhealthy {
            reason: "hard failure".to_string(),
            recoverable: false,
        },
    );

    let err = manager
        .acquire(&key, &workflow_ctx("wf-a"))
        .await
        .expect_err("non-recoverable unhealthy state must block runtime acquire");
    assert_eq!(err.category(), ErrorCategory::Fatal);
    assert!(err.is_fatal());
}


