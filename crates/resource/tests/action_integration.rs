//! Integration contracts for `nebula-action` consumers.
//!
//! These tests intentionally stay inside `nebula-resource` public API to avoid
//! cross-crate coupling while still locking behavior relied on by action code.

use std::time::Duration;

use nebula_core::{ResourceKey, resource_key};

mod scope_helpers;
use nebula_resource::{
    Context, ErrorCategory, ExecutionId, Manager, PoolAcquire, PoolConfig, PoolSizing, Resource,
    WorkflowId,
};
use scope_helpers::*;

#[derive(Debug, Clone)]
struct TestConfig;

impl nebula_resource::Config for TestConfig {}

struct EchoResource;

impl Resource for EchoResource {
    type Config = TestConfig;
    type Instance = String;

    fn key(&self) -> ResourceKey {
        resource_key!("action-echo")
    }

    async fn create(
        &self,
        _config: &Self::Config,
        _ctx: &Context,
    ) -> nebula_resource::Result<Self::Instance> {
        Ok("echo".to_string())
    }
}

fn action_ctx(tenant_id: &str) -> Context {
    Context::new(
        scope_action_in_execution(
            "act-1",
            "exec-1",
            Some("wf-1".to_string()),
            Some(tenant_id.to_string()),
        ),
        WorkflowId::new(),
        ExecutionId::new(),
    )
}

#[tokio::test]
async fn action_dynamic_acquire_supports_downcast_contract() {
    let manager = Manager::new();
    manager
        .register(EchoResource, TestConfig, PoolConfig::default())
        .expect("resource registered");

    let key = resource_key!("action-echo");
    let guard = manager
        .acquire(&key, &action_ctx("tenant-a"))
        .await
        .expect("acquire succeeds");

    let value = guard
        .as_any()
        .downcast_ref::<String>()
        .expect("action must be able to downcast to requested instance type");
    assert_eq!(value, "echo");
}

#[tokio::test]
async fn action_scope_mismatch_returns_fatal_error_contract() {
    let manager = Manager::new();
    manager
        .register_scoped(
            EchoResource,
            TestConfig,
            PoolConfig::default(),
            scope_tenant("tenant-a"),
        )
        .expect("resource registered");

    let key = resource_key!("action-echo");
    let err = manager
        .acquire(&key, &action_ctx("tenant-b"))
        .await
        .expect_err("cross-tenant action acquire must be denied");

    assert_eq!(err.category(), ErrorCategory::Fatal);
    assert!(!err.is_retryable());
}

#[tokio::test]
async fn action_pool_exhaustion_maps_to_retryable_category_contract() {
    let manager = Manager::new();
    manager
        .register(
            EchoResource,
            TestConfig,
            PoolConfig {
                sizing: PoolSizing {
                    min_size: 0,
                    max_size: 1,
                },
                acquire: PoolAcquire {
                    timeout: Duration::from_millis(30),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .expect("resource registered");

    let key = resource_key!("action-echo");
    let held = manager
        .acquire(&key, &action_ctx("tenant-a"))
        .await
        .expect("first acquire succeeds");

    let err = manager
        .acquire(&key, &action_ctx("tenant-a"))
        .await
        .expect_err("second acquire should fail while pool is exhausted");

    assert_eq!(err.category(), ErrorCategory::Retryable);
    assert!(err.is_retryable());
    drop(held);
}
