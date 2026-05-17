//! Shared test helpers for `crates/api` integration tests.
//!
//! Each file under `tests/` is a separate Cargo compilation unit, so helpers
//! that are only used by one file look "dead" to the other. The `#[allow(dead_code)]`
//! attribute below suppresses those false-positive warnings.

#![allow(dead_code)]

use std::sync::Arc;

use nebula_api::{
    ApiConfig, AppState,
    errors::ApiError,
    state::{OrgResolver, WorkspaceResolver},
};
use nebula_core::{OrgId, WorkspaceId};
use nebula_storage::{
    InMemoryExecutionRepo, InMemoryWorkflowRepo, repos::InMemoryControlQueueRepo,
};

// ── Shared constants ─────────────────────────────────────────────────────────

pub(crate) const TEST_JWT_SECRET: &str = "test-secret-for-integration-tests-0123456789";

/// Fixed test org ID — use this in all test URLs.
pub const TEST_ORG: &str = "org_00000000000000000000000001";
/// Fixed test workspace ID — use this in all test URLs.
pub const TEST_WS: &str = "ws_00000000000000000000000001";

/// CSRF token value used by tests for state-changing requests.
pub const TEST_CSRF_TOKEN: &str = "test-csrf-token";
/// Pre-formatted cookie header value for CSRF.
pub const TEST_CSRF_COOKIE: &str = "nebula_csrf=test-csrf-token";

/// Helper to build a tenant-scoped workspace API path.
/// Example: `ws_path("/workflows")` → `/api/v1/orgs/org_.../workspaces/ws_.../workflows`
pub fn ws_path(suffix: &str) -> String {
    format!("/api/v1/orgs/{TEST_ORG}/workspaces/{TEST_WS}{suffix}")
}

/// Helper to build an org-scoped API path.
#[allow(dead_code)]
pub fn org_path(suffix: &str) -> String {
    format!("/api/v1/orgs/{TEST_ORG}{suffix}")
}

/// Stub OrgResolver that accepts any slug and returns a fixed OrgId.
pub struct TestOrgResolver;

#[async_trait::async_trait]
impl OrgResolver for TestOrgResolver {
    async fn resolve_by_slug(&self, _slug: &str) -> Result<OrgId, ApiError> {
        Ok(TEST_ORG.parse().expect("valid test org ID"))
    }
}

/// Stub WorkspaceResolver that accepts any slug and returns a fixed WorkspaceId.
pub struct TestWorkspaceResolver;

#[async_trait::async_trait]
impl WorkspaceResolver for TestWorkspaceResolver {
    async fn resolve_by_slug(&self, _org_id: OrgId, _slug: &str) -> Result<WorkspaceId, ApiError> {
        Ok(TEST_WS.parse().expect("valid test ws ID"))
    }
}

// ── Workflow definition builders ──────────────────────────────────────────────

/// Build a minimal, structurally valid `WorkflowDefinition` JSON that passes
/// `nebula_workflow::validate_workflow` (single node, no cycles, schema_version=1).
pub(crate) fn make_valid_workflow_definition(
    workflow_id: &nebula_core::WorkflowId,
) -> serde_json::Value {
    serde_json::json!({
        "id": workflow_id.to_string(),
        "name": "Valid Workflow",
        "version": { "major": 0, "minor": 1, "patch": 0 },
        "nodes": [
            { "id": "step_a", "name": "Step A", "action_key": "echo" }
        ],
        "connections": [],
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z",
        "schema_version": 1
    })
}

/// Build a `WorkflowDefinition` JSON that parses correctly but fails
/// `validate_workflow` due to a cycle (A → B, B → A) and therefore also
/// fails entry-node detection.
pub(crate) fn make_cyclic_workflow_definition(
    workflow_id: &nebula_core::WorkflowId,
) -> serde_json::Value {
    serde_json::json!({
        "id": workflow_id.to_string(),
        "name": "Cyclic Workflow",
        "version": { "major": 0, "minor": 1, "patch": 0 },
        "nodes": [
            { "id": "step_a", "name": "A", "action_key": "echo" },
            { "id": "step_b", "name": "B", "action_key": "echo" }
        ],
        "connections": [
            { "from_node": "step_a", "to_node": "step_b" },
            { "from_node": "step_b", "to_node": "step_a" }
        ],
        "created_at": "2024-01-01T00:00:00Z",
        "updated_at": "2024-01-01T00:00:00Z",
        "schema_version": 1
    })
}

// ── JWT helper ────────────────────────────────────────────────────────────────

/// Build a valid JWT token signed with [`TEST_JWT_SECRET`].
pub(crate) fn create_test_jwt() -> String {
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize)]
    struct Claims {
        sub: String,
        exp: u64,
        iat: u64,
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();

    encode(
        &Header::default(),
        &Claims {
            sub: "test-user".to_string(),
            exp: now + 3600,
            iat: now,
        },
        &EncodingKey::from_secret(TEST_JWT_SECRET.as_bytes()),
    )
    .unwrap()
}

// ── AppState builders ─────────────────────────────────────────────────────────

/// Create an `AppState` with fully functional in-memory repos; return both the
/// state and a typed reference to the control queue so tests can inspect it.
pub(crate) async fn create_state_with_queue() -> (AppState, Arc<InMemoryControlQueueRepo>) {
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo = Arc::new(InMemoryControlQueueRepo::new());
    let api_config = ApiConfig::for_test();

    let control_queue_dyn: Arc<dyn nebula_storage::repos::ControlQueueRepo> =
        Arc::clone(&control_queue_repo) as _;

    let state = AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_dyn,
        api_config.jwt_secret,
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver));

    (state, control_queue_repo)
}

/// Alias for [`create_state_with_queue`], preserving the name used by
/// `integration_tests.rs` callers so no test body needs to change.
pub(crate) async fn create_test_state_with_queue() -> (AppState, Arc<InMemoryControlQueueRepo>) {
    create_state_with_queue().await
}

/// Create an `AppState` whose execution / workflow / control-queue
/// surface is the **spec-16 scoped port**, wired exactly as the
/// composition root would: the InMemory port adapters wrapped in the
/// `nebula-tenancy` decorators (bound to the placeholder scope), then
/// attached via [`AppState::with_execution_store`] /
/// [`AppState::with_workflow_store`] / [`AppState::with_control_queue`].
///
/// The legacy repos passed to [`AppState::new`] stay present (the
/// not-yet-migrated fields still need them — expand-contract), but every
/// accessor the knife exercises now prefers the port. This lets the §13
/// knife run end-to-end through the port and proves it is
/// behaviourally equivalent to the legacy path. The returned
/// `InMemoryControlQueue` shares the execution store's core, so its
/// non-consuming `snapshot()` observes a `commit`'s / `enqueue`'s outbox
/// rows — the port analogue of `InMemoryControlQueueRepo::snapshot()`.
pub(crate) async fn create_state_with_port_queue()
-> (AppState, nebula_storage::inmem::InMemoryControlQueue) {
    use nebula_storage::inmem::{
        InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader,
        InMemoryNodeResultStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
    };
    use nebula_tenancy::{
        ScopedControlQueue, ScopedExecutionJournalReader, ScopedExecutionStore,
        ScopedNodeResultStore, ScopedWorkflowStore, ScopedWorkflowVersionStore,
    };

    // Fixed placeholder scope: the tenancy decorator substitutes its
    // bound scope on every call, so the concrete value is immaterial to
    // isolation — it only has to be a valid `Scope` (mirrors
    // `AppState`'s own `placeholder_scope`).
    let scope = nebula_storage_port::Scope::new("nebula", "nebula");

    // One shared execution-store core so the control queue + journal
    // observe the rows a `commit`/`enqueue` writes (same wiring contract
    // the conformance harness uses).
    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let journal = InMemoryJournalReader::new(&exec_store);
    let node_results = InMemoryNodeResultStore::new();
    let workflow_store = InMemoryWorkflowStore::new();
    // ONE workflow-version store instance shared between the
    // workflow-CRUD path (`with_workflow_store`) and the
    // resume/definition path (`with_execution_store`). Both InMemory
    // stores are `Clone` over an `Arc<Mutex<…>>`, so cloning shares
    // state — a version published via the workflow handlers must be
    // readable through the execution/resume accessor (otherwise a
    // workflow created in step 1 would be invisible to step 3's start).
    let workflow_versions = InMemoryWorkflowVersionStore::new();

    let api_config = ApiConfig::for_test();

    // Legacy repos remain for the not-yet-migrated `AppState` fields.
    let legacy_workflow = Arc::new(InMemoryWorkflowRepo::new());
    let legacy_execution = Arc::new(InMemoryExecutionRepo::new());
    let legacy_control: Arc<dyn nebula_storage::repos::ControlQueueRepo> =
        Arc::new(InMemoryControlQueueRepo::new());

    let state = AppState::new(
        legacy_workflow,
        legacy_execution,
        legacy_control,
        api_config.jwt_secret,
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver))
    .with_execution_store(
        Arc::new(ScopedExecutionStore::new(
            Arc::new(exec_store),
            scope.clone(),
        )),
        Arc::new(ScopedWorkflowVersionStore::new(
            Arc::new(workflow_versions.clone()),
            scope.clone(),
        )),
        Arc::new(ScopedNodeResultStore::new(
            Arc::new(node_results),
            scope.clone(),
        )),
        Arc::new(ScopedExecutionJournalReader::new(
            Arc::new(journal),
            scope.clone(),
        )),
    )
    .with_workflow_store(
        Arc::new(ScopedWorkflowStore::new(
            Arc::new(workflow_store),
            scope.clone(),
        )),
        Arc::new(ScopedWorkflowVersionStore::new(
            Arc::new(workflow_versions),
            scope.clone(),
        )),
    )
    .with_control_queue(Arc::new(ScopedControlQueue::new(
        Arc::new(control_queue.clone()),
        scope,
    )));

    (state, control_queue)
}
