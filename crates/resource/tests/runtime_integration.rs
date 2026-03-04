//! Integration contracts for `nebula-runtime` orchestration.
//!
//! These tests capture lifecycle and failure semantics runtime depends on.

use nebula_core::ResourceKey;
use nebula_resource::quarantine::QuarantineReason;
use nebula_resource::{
    Context, ErrorCategory, ExecutionId, HealthState, Manager, PoolConfig, Resource, Scope,
    WorkflowId,
};

#[derive(Debug, Clone)]
struct TestConfig;

impl nebula_resource::Config for TestConfig {}

struct RuntimeResourceA;
struct RuntimeResourceB;

impl Resource for RuntimeResourceA {
    type Config = TestConfig;
    type Instance = &'static str;

    fn metadata(&self) -> nebula_resource::ResourceMetadata {
        nebula_resource::ResourceMetadata::from_key(
            ResourceKey::try_from("runtime-a").expect("valid resource key"),
        )
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

    fn metadata(&self) -> nebula_resource::ResourceMetadata {
        nebula_resource::ResourceMetadata::from_key(
            ResourceKey::try_from("runtime-b").expect("valid resource key"),
        )
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
        Scope::workflow(workflow_id),
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
            Scope::workflow("wf-a"),
        )
        .expect("resource a registered");
    manager
        .register_scoped(
            RuntimeResourceB,
            TestConfig,
            PoolConfig::default(),
            Scope::workflow("wf-b"),
        )
        .expect("resource b registered");

    let key_a = ResourceKey::try_from("runtime-a").expect("valid resource key");
    let key_b = ResourceKey::try_from("runtime-b").expect("valid resource key");

    manager
        .shutdown_scope(&Scope::workflow("wf-a"))
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

    let key = ResourceKey::try_from("runtime-a").expect("valid resource key");
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

    let key = ResourceKey::try_from("runtime-a").expect("valid resource key");
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
