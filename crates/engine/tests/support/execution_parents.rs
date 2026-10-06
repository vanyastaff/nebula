//! The rows an execution needs before it can exist on a SQL backend.
//!
//! Executions reference their workflow, and the workflow its org and
//! workspace (`crates/storage/docs/database-standard.md`). The in-memory
//! store does not check references between aggregates, but SQLite and
//! PostgreSQL do, so a SQL test that creates executions first provisions the
//! tenant behind its scope and the workflow its executions name. Every call is
//! idempotent. Engine-side twin of `crates/storage/tests/support/execution_parents.rs`.

use nebula_storage_port::Scope;
use nebula_storage_port::dto::{
    PrincipalKind, TenantDefaultWorkspaceCreate, TenantOrgCreate, TenantProvisioningOutcome,
    TenantProvisioningRequest, WorkflowRecord,
};
use nebula_storage_port::store::{TenantProvisioningStore, WorkflowStore};

/// Provision `scope`'s org and workspace, then the live workflow
/// `workflow_id` in it.
pub(crate) async fn seed_execution_parents(
    tenants: &dyn TenantProvisioningStore,
    workflows: &dyn WorkflowStore,
    scope: &Scope,
    workflow_id: &str,
) {
    let org = TenantOrgCreate::new(
        scope.org_id.clone(),
        scope.org_id.clone(),
        "Fixture".into(),
        "fixture".into(),
        "free".into(),
        None,
        serde_json::json!({}),
    )
    .expect("org values");
    let workspace = TenantDefaultWorkspaceCreate::new(
        scope.workspace_id.clone(),
        "default".into(),
        "Default".into(),
        None,
        "fixture".into(),
        serde_json::json!({}),
    )
    .expect("workspace values");
    let request = TenantProvisioningRequest::new(
        org,
        workspace,
        PrincipalKind::User,
        "fixture-owner".into(),
        None,
    )
    .expect("provisioning request");
    let outcome = tenants
        .provision_tenant(request)
        .await
        .expect("provision the fixture scope");
    assert!(
        matches!(
            outcome,
            TenantProvisioningOutcome::Created | TenantProvisioningOutcome::Replayed
        ),
        "the fixture scope must provision or replay, got {outcome:?}"
    );
    if workflows
        .get(scope, workflow_id)
        .await
        .expect("read the fixture workflow")
        .is_some()
    {
        return;
    }
    workflows
        .create(
            scope,
            WorkflowRecord {
                id: workflow_id.into(),
                scope: scope.clone(),
                version: 1,
                slug: workflow_id.into(),
            },
        )
        .await
        .expect("create the fixture workflow");
}

/// [`seed_execution_parents`] directly on a backend's pool.
#[async_trait::async_trait]
pub(crate) trait SeedExecutionParents {
    /// Provision `scope` and the live workflow `workflow_id` in it.
    async fn seed_execution_parents(&self, scope: &Scope, workflow_id: &str);
}

#[async_trait::async_trait]
impl SeedExecutionParents for sqlx::SqlitePool {
    async fn seed_execution_parents(&self, scope: &Scope, workflow_id: &str) {
        seed_execution_parents(
            &nebula_storage::sqlite::SqliteTenantProvisioningStore::new(self.clone()),
            &nebula_storage::sqlite::SqliteWorkflowStore::new(self.clone()),
            scope,
            workflow_id,
        )
        .await;
    }
}

#[async_trait::async_trait]
impl SeedExecutionParents for sqlx::PgPool {
    async fn seed_execution_parents(&self, scope: &Scope, workflow_id: &str) {
        seed_execution_parents(
            &nebula_storage::postgres::PgTenantProvisioningStore::new(self.clone()),
            &nebula_storage::postgres::PgWorkflowStore::new(self.clone()),
            scope,
            workflow_id,
        )
        .await;
    }
}
