#![allow(dead_code)]

use nebula_resource::Scope;

pub fn scope_tenant(tenant_id: impl Into<String>) -> Scope {
    Scope::try_tenant(tenant_id).expect("valid tenant scope in test")
}

pub fn scope_workflow(workflow_id: impl Into<String>) -> Scope {
    Scope::try_workflow(workflow_id).expect("valid workflow scope in test")
}

pub fn scope_workflow_in_tenant(
    workflow_id: impl Into<String>,
    tenant_id: impl Into<String>,
) -> Scope {
    Scope::try_workflow_in_tenant(workflow_id, tenant_id)
        .expect("valid workflow-in-tenant scope in test")
}

pub fn scope_execution(execution_id: impl Into<String>) -> Scope {
    Scope::try_execution(execution_id).expect("valid execution scope in test")
}

pub fn scope_execution_in_workflow(
    execution_id: impl Into<String>,
    workflow_id: impl Into<String>,
    tenant_id: Option<String>,
) -> Scope {
    Scope::try_execution_in_workflow(execution_id, workflow_id, tenant_id)
        .expect("valid execution-in-workflow scope in test")
}

pub fn scope_action(action_id: impl Into<String>) -> Scope {
    Scope::try_action(action_id).expect("valid action scope in test")
}

pub fn scope_action_in_execution(
    action_id: impl Into<String>,
    execution_id: impl Into<String>,
    workflow_id: Option<String>,
    tenant_id: Option<String>,
) -> Scope {
    Scope::try_action_in_execution(action_id, execution_id, workflow_id, tenant_id)
        .expect("valid action-in-execution scope in test")
}

pub fn scope_custom(key: impl Into<String>, value: impl Into<String>) -> Scope {
    Scope::try_custom(key, value).expect("valid custom scope in test")
}
