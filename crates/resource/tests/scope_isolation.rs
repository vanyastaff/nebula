//! Comprehensive scope isolation tests
//!
//! Tests every meaningful combination of Scope::contains()
//! to ensure tenant isolation and hierarchical access control.
//! Includes T018: Manager-level scope validation denies cross-scope acquire.

use nebula_resource::Scope;

// ---------------------------------------------------------------------------
// 1. Global scope
// ---------------------------------------------------------------------------

#[test]
fn global_contains_global() {
    assert!(Scope::Global.contains(&Scope::Global));
}

#[test]
fn global_contains_tenant() {
    assert!(Scope::Global.contains(&Scope::tenant("A")));
}

#[test]
fn global_contains_workflow() {
    assert!(Scope::Global.contains(&Scope::workflow("wf1")));
}

#[test]
fn global_contains_execution() {
    assert!(Scope::Global.contains(&Scope::execution("ex1")));
}

#[test]
fn global_contains_action() {
    assert!(Scope::Global.contains(&Scope::action("a1")));
}

#[test]
fn global_contains_custom() {
    assert!(Scope::Global.contains(&Scope::custom("env", "prod")));
}

// ---------------------------------------------------------------------------
// 2. Tenant isolation
// ---------------------------------------------------------------------------

#[test]
fn tenant_contains_same_tenant() {
    let t = Scope::tenant("A");
    assert!(t.contains(&Scope::tenant("A")));
}

#[test]
fn tenant_does_not_contain_different_tenant() {
    let t = Scope::tenant("A");
    assert!(!t.contains(&Scope::tenant("B")));
}

#[test]
fn tenant_does_not_contain_global() {
    assert!(!Scope::tenant("A").contains(&Scope::Global));
}

#[test]
fn tenant_contains_workflow_with_matching_parent() {
    let t = Scope::tenant("A");
    let wf = Scope::workflow_in_tenant("wf1", "A");
    assert!(t.contains(&wf));
}

#[test]
fn tenant_rejects_workflow_with_wrong_parent() {
    let t = Scope::tenant("A");
    let wf = Scope::workflow_in_tenant("wf1", "B");
    assert!(!t.contains(&wf));
}

#[test]
fn tenant_rejects_workflow_without_parent() {
    let t = Scope::tenant("A");
    let wf = Scope::workflow("wf1");
    assert!(!t.contains(&wf));
}

#[test]
fn tenant_contains_execution_with_matching_tenant() {
    let t = Scope::tenant("A");
    let ex = Scope::execution_in_workflow("ex1", "wf1", Some("A".to_string()));
    assert!(t.contains(&ex));
}

#[test]
fn tenant_rejects_execution_with_wrong_tenant() {
    let t = Scope::tenant("A");
    let ex = Scope::execution_in_workflow("ex1", "wf1", Some("B".to_string()));
    assert!(!t.contains(&ex));
}

#[test]
fn tenant_rejects_execution_without_tenant() {
    let t = Scope::tenant("A");
    let ex = Scope::execution("ex1");
    assert!(!t.contains(&ex));
}

#[test]
fn tenant_contains_action_with_matching_tenant() {
    let t = Scope::tenant("A");
    let a = Scope::action_in_execution("a1", "ex1", Some("wf1".to_string()), Some("A".to_string()));
    assert!(t.contains(&a));
}

#[test]
fn tenant_rejects_action_with_wrong_tenant() {
    let t = Scope::tenant("A");
    let a = Scope::action_in_execution("a1", "ex1", Some("wf1".to_string()), Some("B".to_string()));
    assert!(!t.contains(&a));
}

#[test]
fn tenant_rejects_action_without_tenant() {
    let t = Scope::tenant("A");
    let a = Scope::action("a1");
    assert!(!t.contains(&a));
}

// ---------------------------------------------------------------------------
// 3. Workflow containment
// ---------------------------------------------------------------------------

#[test]
fn workflow_contains_same_workflow() {
    let wf = Scope::workflow("wf1");
    assert!(wf.contains(&Scope::workflow("wf1")));
}

#[test]
fn workflow_does_not_contain_different_workflow() {
    let wf = Scope::workflow("wf1");
    assert!(!wf.contains(&Scope::workflow("wf2")));
}

#[test]
fn workflow_contains_execution_with_matching_parent() {
    let wf = Scope::workflow("wf1");
    let ex = Scope::execution_in_workflow("ex1", "wf1", None);
    assert!(wf.contains(&ex));
}

#[test]
fn workflow_rejects_execution_with_wrong_parent() {
    let wf = Scope::workflow("wf1");
    let ex = Scope::execution_in_workflow("ex1", "wf2", None);
    assert!(!wf.contains(&ex));
}

#[test]
fn workflow_rejects_execution_without_parent() {
    let wf = Scope::workflow("wf1");
    let ex = Scope::execution("ex1");
    assert!(!wf.contains(&ex));
}

#[test]
fn workflow_contains_action_with_matching_workflow() {
    let wf = Scope::workflow("wf1");
    let a = Scope::action_in_execution("a1", "ex1", Some("wf1".to_string()), None);
    assert!(wf.contains(&a));
}

#[test]
fn workflow_rejects_action_with_wrong_workflow() {
    let wf = Scope::workflow("wf1");
    let a = Scope::action_in_execution("a1", "ex1", Some("wf2".to_string()), None);
    assert!(!wf.contains(&a));
}

#[test]
fn workflow_rejects_action_without_workflow() {
    let wf = Scope::workflow("wf1");
    let a = Scope::action("a1");
    assert!(!wf.contains(&a));
}

#[test]
fn workflow_does_not_contain_global() {
    assert!(!Scope::workflow("wf1").contains(&Scope::Global));
}

#[test]
fn workflow_does_not_contain_tenant() {
    assert!(!Scope::workflow("wf1").contains(&Scope::tenant("A")));
}

// ---------------------------------------------------------------------------
// 4. Execution containment
// ---------------------------------------------------------------------------

#[test]
fn execution_contains_same_execution() {
    let ex = Scope::execution("ex1");
    assert!(ex.contains(&Scope::execution("ex1")));
}

#[test]
fn execution_does_not_contain_different_execution() {
    let ex = Scope::execution("ex1");
    assert!(!ex.contains(&Scope::execution("ex2")));
}

#[test]
fn execution_contains_action_with_matching_parent() {
    let ex = Scope::execution("ex1");
    let a = Scope::action_in_execution("a1", "ex1", None, None);
    assert!(ex.contains(&a));
}

#[test]
fn execution_rejects_action_with_wrong_parent() {
    let ex = Scope::execution("ex1");
    let a = Scope::action_in_execution("a1", "ex2", None, None);
    assert!(!ex.contains(&a));
}

#[test]
fn execution_rejects_action_without_parent() {
    let ex = Scope::execution("ex1");
    let a = Scope::action("a1");
    assert!(!ex.contains(&a));
}

#[test]
fn execution_does_not_contain_global() {
    assert!(!Scope::execution("ex1").contains(&Scope::Global));
}

#[test]
fn execution_does_not_contain_tenant() {
    assert!(!Scope::execution("ex1").contains(&Scope::tenant("A")));
}

#[test]
fn execution_does_not_contain_workflow() {
    assert!(!Scope::execution("ex1").contains(&Scope::workflow("wf1")));
}

// ---------------------------------------------------------------------------
// 5. Action containment
// ---------------------------------------------------------------------------

#[test]
fn action_contains_same_action() {
    let a = Scope::action("a1");
    assert!(a.contains(&Scope::action("a1")));
}

#[test]
fn action_does_not_contain_different_action() {
    let a = Scope::action("a1");
    assert!(!a.contains(&Scope::action("a2")));
}

#[test]
fn action_does_not_contain_global() {
    assert!(!Scope::action("a1").contains(&Scope::Global));
}

#[test]
fn action_does_not_contain_tenant() {
    assert!(!Scope::action("a1").contains(&Scope::tenant("A")));
}

// ---------------------------------------------------------------------------
// 6. Custom scope
// ---------------------------------------------------------------------------

#[test]
fn custom_contains_same_custom() {
    let c = Scope::custom("env", "prod");
    assert!(c.contains(&Scope::custom("env", "prod")));
}

#[test]
fn custom_does_not_contain_different_key() {
    let c = Scope::custom("env", "prod");
    assert!(!c.contains(&Scope::custom("region", "prod")));
}

#[test]
fn custom_does_not_contain_different_value() {
    let c = Scope::custom("env", "prod");
    assert!(!c.contains(&Scope::custom("env", "staging")));
}

#[test]
fn custom_does_not_contain_global() {
    assert!(!Scope::custom("env", "prod").contains(&Scope::Global));
}

// ---------------------------------------------------------------------------
// 7. Cross-level deny-by-default
// ---------------------------------------------------------------------------

#[test]
fn narrower_scope_cannot_contain_broader() {
    // Action cannot contain Execution
    let action =
        Scope::action_in_execution("a1", "ex1", Some("wf1".to_string()), Some("A".to_string()));
    let exec = Scope::execution("ex1");
    assert!(!action.contains(&exec));

    // Execution cannot contain Workflow
    let wf = Scope::workflow("wf1");
    assert!(!exec.contains(&wf));
}

#[test]
fn cross_type_scopes_are_incompatible() {
    // Custom does not contain Tenant
    assert!(!Scope::custom("env", "prod").contains(&Scope::tenant("A")));
    // Tenant does not contain Custom
    assert!(!Scope::tenant("A").contains(&Scope::custom("env", "prod")));
}

// ---------------------------------------------------------------------------
// 8. T018: Manager-level scope validation denies cross-scope acquire
// ---------------------------------------------------------------------------

#[cfg(test)]
mod manager_scope_tests {
    use std::time::Duration;

    use nebula_resource::Manager;
    use nebula_resource::context::Context;
    use nebula_resource::error::Result;
    use nebula_resource::pool::PoolConfig;
    use nebula_resource::resource::{Config, Resource};
    use nebula_resource::scope::Scope;

    #[derive(Debug, Clone, serde::Deserialize)]
    struct TestConfig;
    impl Config for TestConfig {}

    struct TestResource {
        name: &'static str,
    }

    impl Resource for TestResource {
        type Config = TestConfig;
        type Instance = String;

        fn id(&self) -> &str {
            self.name
        }

        async fn create(&self, _config: &TestConfig, _ctx: &Context) -> Result<String> {
            Ok(format!("{}-instance", self.name))
        }
    }

    fn pool_config() -> PoolConfig {
        PoolConfig {
            min_size: 0,
            max_size: 2,
            acquire_timeout: Duration::from_secs(1),
            ..Default::default()
        }
    }

    /// Register resource with Scope::Tenant("A"), try to acquire with
    /// Scope::Tenant("B"), expect error.
    #[tokio::test]
    async fn cross_tenant_acquire_denied() {
        let mgr = Manager::new();
        mgr.register_scoped(
            TestResource { name: "db" },
            TestConfig,
            pool_config(),
            Scope::tenant("A"),
        )
        .unwrap();

        let ctx_b = Context::new(Scope::tenant("B"), "wf1", "ex1");
        let result = mgr.acquire("db", &ctx_b).await;
        let err = result.expect_err("cross-tenant acquire should be denied");
        assert!(
            err.to_string().contains("Scope mismatch"),
            "error should mention scope mismatch, got: {err}"
        );
    }

    /// Same tenant scope should succeed.
    #[tokio::test]
    async fn same_tenant_acquire_allowed() {
        let mgr = Manager::new();
        mgr.register_scoped(
            TestResource { name: "db" },
            TestConfig,
            pool_config(),
            Scope::tenant("A"),
        )
        .unwrap();

        let ctx_a = Context::new(Scope::tenant("A"), "wf1", "ex1");
        let _guard = mgr
            .acquire("db", &ctx_a)
            .await
            .expect("same-tenant acquire should succeed");
    }

    /// Workflow-scoped resource denies access from a different workflow.
    #[tokio::test]
    async fn cross_workflow_acquire_denied() {
        let mgr = Manager::new();
        mgr.register_scoped(
            TestResource { name: "cache" },
            TestConfig,
            pool_config(),
            Scope::workflow("wf1"),
        )
        .unwrap();

        let ctx_wf2 = Context::new(Scope::workflow("wf2"), "wf2", "ex1");
        let err = mgr
            .acquire("cache", &ctx_wf2)
            .await
            .expect_err("cross-workflow acquire should be denied");
        assert!(
            err.to_string().contains("Scope mismatch"),
            "error should mention scope mismatch, got: {err}"
        );
    }

    /// Global-scoped resource allows access from any scope.
    #[tokio::test(start_paused = true)]
    async fn global_resource_accessible_from_any_scope() {
        let mgr = Manager::new();
        mgr.register(
            TestResource { name: "global-db" },
            TestConfig,
            pool_config(),
        )
        .unwrap();

        // From tenant scope
        let ctx_tenant = Context::new(Scope::tenant("A"), "wf1", "ex1");
        let _g1 = mgr
            .acquire("global-db", &ctx_tenant)
            .await
            .expect("global resource should be accessible from tenant scope");

        // Wait for guard to return
        drop(_g1);
        tokio::time::sleep(Duration::from_millis(30)).await;

        // From workflow scope
        let ctx_wf = Context::new(Scope::workflow("wf1"), "wf1", "ex1");
        let _g2 = mgr
            .acquire("global-db", &ctx_wf)
            .await
            .expect("global resource should be accessible from workflow scope");
    }

    /// Narrower resource scope does not contain a broader context scope.
    #[tokio::test]
    async fn narrow_resource_denies_broader_context() {
        let mgr = Manager::new();
        mgr.register_scoped(
            TestResource { name: "exec-db" },
            TestConfig,
            pool_config(),
            Scope::execution("ex1"),
        )
        .unwrap();

        // Global context is broader than execution scope
        let ctx_global = Context::new(Scope::Global, "wf1", "ex1");
        let err = mgr
            .acquire("exec-db", &ctx_global)
            .await
            .expect_err("execution-scoped resource should deny global context");
        assert!(
            err.to_string().contains("Scope mismatch"),
            "error should mention scope mismatch, got: {err}"
        );
    }
}
