//! Comprehensive scope isolation tests
//!
//! Tests every meaningful combination of ResourceScope::contains()
//! to ensure tenant isolation and hierarchical access control.

use nebula_resource::ResourceScope;

// ---------------------------------------------------------------------------
// 1. Global scope
// ---------------------------------------------------------------------------

#[test]
fn global_contains_global() {
    assert!(ResourceScope::Global.contains(&ResourceScope::Global));
}

#[test]
fn global_contains_tenant() {
    assert!(ResourceScope::Global.contains(&ResourceScope::tenant("A")));
}

#[test]
fn global_contains_workflow() {
    assert!(ResourceScope::Global.contains(&ResourceScope::workflow("wf1")));
}

#[test]
fn global_contains_execution() {
    assert!(ResourceScope::Global.contains(&ResourceScope::execution("ex1")));
}

#[test]
fn global_contains_action() {
    assert!(ResourceScope::Global.contains(&ResourceScope::action("a1")));
}

#[test]
fn global_contains_custom() {
    assert!(ResourceScope::Global.contains(&ResourceScope::custom("env", "prod")));
}

// ---------------------------------------------------------------------------
// 2. Tenant isolation
// ---------------------------------------------------------------------------

#[test]
fn tenant_contains_same_tenant() {
    let t = ResourceScope::tenant("A");
    assert!(t.contains(&ResourceScope::tenant("A")));
}

#[test]
fn tenant_does_not_contain_different_tenant() {
    let t = ResourceScope::tenant("A");
    assert!(!t.contains(&ResourceScope::tenant("B")));
}

#[test]
fn tenant_does_not_contain_global() {
    assert!(!ResourceScope::tenant("A").contains(&ResourceScope::Global));
}

#[test]
fn tenant_contains_workflow_with_matching_parent() {
    let t = ResourceScope::tenant("A");
    let wf = ResourceScope::workflow_in_tenant("wf1", "A");
    assert!(t.contains(&wf));
}

#[test]
fn tenant_rejects_workflow_with_wrong_parent() {
    let t = ResourceScope::tenant("A");
    let wf = ResourceScope::workflow_in_tenant("wf1", "B");
    assert!(!t.contains(&wf));
}

#[test]
fn tenant_rejects_workflow_without_parent() {
    let t = ResourceScope::tenant("A");
    let wf = ResourceScope::workflow("wf1");
    assert!(!t.contains(&wf));
}

#[test]
fn tenant_contains_execution_with_matching_tenant() {
    let t = ResourceScope::tenant("A");
    let ex = ResourceScope::execution_in_workflow("ex1", "wf1", Some("A".to_string()));
    assert!(t.contains(&ex));
}

#[test]
fn tenant_rejects_execution_with_wrong_tenant() {
    let t = ResourceScope::tenant("A");
    let ex = ResourceScope::execution_in_workflow("ex1", "wf1", Some("B".to_string()));
    assert!(!t.contains(&ex));
}

#[test]
fn tenant_rejects_execution_without_tenant() {
    let t = ResourceScope::tenant("A");
    let ex = ResourceScope::execution("ex1");
    assert!(!t.contains(&ex));
}

#[test]
fn tenant_contains_action_with_matching_tenant() {
    let t = ResourceScope::tenant("A");
    let a = ResourceScope::action_in_execution(
        "a1",
        "ex1",
        Some("wf1".to_string()),
        Some("A".to_string()),
    );
    assert!(t.contains(&a));
}

#[test]
fn tenant_rejects_action_with_wrong_tenant() {
    let t = ResourceScope::tenant("A");
    let a = ResourceScope::action_in_execution(
        "a1",
        "ex1",
        Some("wf1".to_string()),
        Some("B".to_string()),
    );
    assert!(!t.contains(&a));
}

#[test]
fn tenant_rejects_action_without_tenant() {
    let t = ResourceScope::tenant("A");
    let a = ResourceScope::action("a1");
    assert!(!t.contains(&a));
}

// ---------------------------------------------------------------------------
// 3. Workflow containment
// ---------------------------------------------------------------------------

#[test]
fn workflow_contains_same_workflow() {
    let wf = ResourceScope::workflow("wf1");
    assert!(wf.contains(&ResourceScope::workflow("wf1")));
}

#[test]
fn workflow_does_not_contain_different_workflow() {
    let wf = ResourceScope::workflow("wf1");
    assert!(!wf.contains(&ResourceScope::workflow("wf2")));
}

#[test]
fn workflow_contains_execution_with_matching_parent() {
    let wf = ResourceScope::workflow("wf1");
    let ex = ResourceScope::execution_in_workflow("ex1", "wf1", None);
    assert!(wf.contains(&ex));
}

#[test]
fn workflow_rejects_execution_with_wrong_parent() {
    let wf = ResourceScope::workflow("wf1");
    let ex = ResourceScope::execution_in_workflow("ex1", "wf2", None);
    assert!(!wf.contains(&ex));
}

#[test]
fn workflow_rejects_execution_without_parent() {
    let wf = ResourceScope::workflow("wf1");
    let ex = ResourceScope::execution("ex1");
    assert!(!wf.contains(&ex));
}

#[test]
fn workflow_contains_action_with_matching_workflow() {
    let wf = ResourceScope::workflow("wf1");
    let a = ResourceScope::action_in_execution("a1", "ex1", Some("wf1".to_string()), None);
    assert!(wf.contains(&a));
}

#[test]
fn workflow_rejects_action_with_wrong_workflow() {
    let wf = ResourceScope::workflow("wf1");
    let a = ResourceScope::action_in_execution("a1", "ex1", Some("wf2".to_string()), None);
    assert!(!wf.contains(&a));
}

#[test]
fn workflow_rejects_action_without_workflow() {
    let wf = ResourceScope::workflow("wf1");
    let a = ResourceScope::action("a1");
    assert!(!wf.contains(&a));
}

#[test]
fn workflow_does_not_contain_global() {
    assert!(!ResourceScope::workflow("wf1").contains(&ResourceScope::Global));
}

#[test]
fn workflow_does_not_contain_tenant() {
    assert!(!ResourceScope::workflow("wf1").contains(&ResourceScope::tenant("A")));
}

// ---------------------------------------------------------------------------
// 4. Execution containment
// ---------------------------------------------------------------------------

#[test]
fn execution_contains_same_execution() {
    let ex = ResourceScope::execution("ex1");
    assert!(ex.contains(&ResourceScope::execution("ex1")));
}

#[test]
fn execution_does_not_contain_different_execution() {
    let ex = ResourceScope::execution("ex1");
    assert!(!ex.contains(&ResourceScope::execution("ex2")));
}

#[test]
fn execution_contains_action_with_matching_parent() {
    let ex = ResourceScope::execution("ex1");
    let a = ResourceScope::action_in_execution("a1", "ex1", None, None);
    assert!(ex.contains(&a));
}

#[test]
fn execution_rejects_action_with_wrong_parent() {
    let ex = ResourceScope::execution("ex1");
    let a = ResourceScope::action_in_execution("a1", "ex2", None, None);
    assert!(!ex.contains(&a));
}

#[test]
fn execution_rejects_action_without_parent() {
    let ex = ResourceScope::execution("ex1");
    let a = ResourceScope::action("a1");
    assert!(!ex.contains(&a));
}

#[test]
fn execution_does_not_contain_global() {
    assert!(!ResourceScope::execution("ex1").contains(&ResourceScope::Global));
}

#[test]
fn execution_does_not_contain_tenant() {
    assert!(!ResourceScope::execution("ex1").contains(&ResourceScope::tenant("A")));
}

#[test]
fn execution_does_not_contain_workflow() {
    assert!(!ResourceScope::execution("ex1").contains(&ResourceScope::workflow("wf1")));
}

// ---------------------------------------------------------------------------
// 5. Action containment
// ---------------------------------------------------------------------------

#[test]
fn action_contains_same_action() {
    let a = ResourceScope::action("a1");
    assert!(a.contains(&ResourceScope::action("a1")));
}

#[test]
fn action_does_not_contain_different_action() {
    let a = ResourceScope::action("a1");
    assert!(!a.contains(&ResourceScope::action("a2")));
}

#[test]
fn action_does_not_contain_global() {
    assert!(!ResourceScope::action("a1").contains(&ResourceScope::Global));
}

#[test]
fn action_does_not_contain_tenant() {
    assert!(!ResourceScope::action("a1").contains(&ResourceScope::tenant("A")));
}

// ---------------------------------------------------------------------------
// 6. Custom scope
// ---------------------------------------------------------------------------

#[test]
fn custom_contains_same_custom() {
    let c = ResourceScope::custom("env", "prod");
    assert!(c.contains(&ResourceScope::custom("env", "prod")));
}

#[test]
fn custom_does_not_contain_different_key() {
    let c = ResourceScope::custom("env", "prod");
    assert!(!c.contains(&ResourceScope::custom("region", "prod")));
}

#[test]
fn custom_does_not_contain_different_value() {
    let c = ResourceScope::custom("env", "prod");
    assert!(!c.contains(&ResourceScope::custom("env", "staging")));
}

#[test]
fn custom_does_not_contain_global() {
    assert!(!ResourceScope::custom("env", "prod").contains(&ResourceScope::Global));
}

// ---------------------------------------------------------------------------
// 7. Cross-level deny-by-default
// ---------------------------------------------------------------------------

#[test]
fn narrower_scope_cannot_contain_broader() {
    // Action cannot contain Execution
    let action = ResourceScope::action_in_execution(
        "a1",
        "ex1",
        Some("wf1".to_string()),
        Some("A".to_string()),
    );
    let exec = ResourceScope::execution("ex1");
    assert!(!action.contains(&exec));

    // Execution cannot contain Workflow
    let wf = ResourceScope::workflow("wf1");
    assert!(!exec.contains(&wf));
}

#[test]
fn cross_type_scopes_are_incompatible() {
    // Custom does not contain Tenant
    assert!(!ResourceScope::custom("env", "prod").contains(&ResourceScope::tenant("A")));
    // Tenant does not contain Custom
    assert!(!ResourceScope::tenant("A").contains(&ResourceScope::custom("env", "prod")));
}
