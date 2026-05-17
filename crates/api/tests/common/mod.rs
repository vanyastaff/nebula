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
use nebula_storage::inmem::{InMemoryControlQueue, InMemoryExecutionStore};
use nebula_storage_port::Scope;

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

/// Build an `AppState` whose execution / workflow / control-queue
/// surface is the scoped storage port, wired exactly as the composition
/// root does: the in-memory port adapters wrapped in the
/// `nebula-tenancy` scoping decorators (bound to the placeholder scope)
/// and passed straight to [`AppState::new`].
///
/// Returns the state plus the raw (undecorated) `InMemoryControlQueue`
/// and `InMemoryExecutionStore` handles. Both port stores are `Clone`
/// over an `Arc<Mutex<…>>`, so the returned handles share state with the
/// decorated ones inside `AppState`: a test can seed an execution row
/// directly through the returned `InMemoryExecutionStore` and observe
/// the durable outbox through the returned `InMemoryControlQueue`'s
/// non-consuming `snapshot()`. One shared execution-store core backs the
/// control queue and journal so a `commit`/`enqueue` is visible through
/// every reader; one workflow-version store instance is shared between
/// the workflow-CRUD path and the resume/definition path so a version
/// published via the workflow handlers is readable through the
/// execution accessor.
/// Raw (undecorated) in-memory port store handles that share state with
/// the scoping-decorated stores inside the returned [`AppState`].
///
/// Every port store is `Clone` over an `Arc<Mutex<…>>`, so these handles
/// observe (and can seed) exactly the rows the API sees through the
/// tenancy decorators. The seed/read helpers below wrap the port-store
/// API with the ergonomics the pre-port `state.execution_repo` /
/// `state.workflow_repo` calls had, so a test body is a one-line change.
pub(crate) struct PortHandles {
    /// Durable control-queue outbox (non-consuming `snapshot()` for
    /// asserting enqueued Start/Cancel rows).
    pub control_queue: InMemoryControlQueue,
    exec_store: InMemoryExecutionStore,
    journal: nebula_storage::inmem::InMemoryJournalReader,
    workflow_store: nebula_storage::inmem::InMemoryWorkflowStore,
    workflow_versions: nebula_storage::inmem::InMemoryWorkflowVersionStore,
}

/// The fixed placeholder scope every handle (and the `AppState` tenancy
/// decorators) bind to — mirrors `AppState::placeholder_scope`. A row
/// seeded under this scope is visible through the decorated stores.
fn port_scope() -> Scope {
    Scope::new("nebula", "nebula")
}

impl PortHandles {
    /// Seed an execution row directly (port equivalent of the old
    /// `state.execution_repo.create(id, workflow_id, state_json)`).
    pub(crate) async fn seed_execution(
        &self,
        execution_id: nebula_core::ExecutionId,
        workflow_id: nebula_core::WorkflowId,
        state_json: serde_json::Value,
    ) {
        use nebula_storage_port::store::ExecutionStore;
        ExecutionStore::create(
            &self.exec_store,
            &port_scope(),
            &execution_id.to_string(),
            &workflow_id.to_string(),
            state_json,
        )
        .await
        .expect("seed_execution: port create must succeed");
    }

    /// Seed a workflow definition directly (port equivalent of the old
    /// `state.workflow_repo.save(id, 0, definition)`): a workflow row at
    /// version 1 plus a published version record #1 — exactly what
    /// `AppState::workflow_save(version == 0)` performs, so the activate
    /// / get-by-id handlers resolve the definition identically.
    pub(crate) async fn seed_workflow(
        &self,
        workflow_id: nebula_core::WorkflowId,
        definition: serde_json::Value,
    ) {
        use nebula_storage_port::store::{WorkflowStore, WorkflowVersionStore};
        let scope = port_scope();
        let id_str = workflow_id.to_string();
        WorkflowStore::create(
            &self.workflow_store,
            &scope,
            nebula_storage_port::dto::WorkflowRecord {
                id: id_str.clone(),
                scope: scope.clone(),
                version: 1,
                slug: id_str.clone(),
                deleted: false,
            },
        )
        .await
        .expect("seed_workflow: port workflow create must succeed");
        WorkflowVersionStore::create(
            &self.workflow_versions,
            &scope,
            nebula_storage_port::dto::WorkflowVersionRecord {
                workflow_id: id_str,
                number: 1,
                published: true,
                pinned: false,
                definition,
            },
        )
        .await
        .expect("seed_workflow: port version create must succeed");
    }

    /// Read the persisted `(version, state_json)` for an execution (port
    /// equivalent of the old `state.execution_repo.get_state(id)`).
    pub(crate) async fn execution_state(
        &self,
        execution_id: nebula_core::ExecutionId,
    ) -> Option<(u64, serde_json::Value)> {
        use nebula_storage_port::store::ExecutionStore;
        ExecutionStore::get(&self.exec_store, &port_scope(), &execution_id.to_string())
            .await
            .expect("execution_state: port get must not error")
            .map(|r| (r.version, r.state))
    }

    /// List running execution ids as opaque strings (port equivalent of
    /// the old `state.execution_repo.list_running()`; the port id form is
    /// the canonical string, not a typed `ExecutionId`).
    pub(crate) async fn running_executions(&self) -> Vec<String> {
        use nebula_storage_port::store::ExecutionStore;
        ExecutionStore::list_running(&self.exec_store, &port_scope())
            .await
            .expect("running_executions: port list_running must not error")
    }
}

/// Build an `AppState` whose execution / workflow / control-queue
/// surface is the scoped storage port, wired exactly as the composition
/// root does: the in-memory port adapters wrapped in the
/// `nebula-tenancy` scoping decorators (bound to the placeholder scope)
/// and passed straight to [`AppState::new`].
///
/// Returns the state plus a [`PortHandles`] bundle of raw (undecorated)
/// store handles that share state with the decorated ones inside
/// `AppState`. One shared execution-store core backs the control queue
/// and journal so a `commit`/`enqueue` is visible through every reader;
/// one workflow-version store instance is shared between the
/// workflow-CRUD path and the resume/definition path so a version
/// published via the workflow handlers is readable through the
/// execution accessor.
async fn build_port_state() -> (AppState, PortHandles) {
    use nebula_storage::inmem::{
        InMemoryJournalReader, InMemoryNodeResultStore, InMemoryWorkflowStore,
        InMemoryWorkflowVersionStore,
    };
    use nebula_tenancy::{
        ScopedControlQueue, ScopedExecutionJournalReader, ScopedExecutionStore,
        ScopedNodeResultStore, ScopedWorkflowStore, ScopedWorkflowVersionStore,
    };

    let scope = port_scope();

    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let journal = InMemoryJournalReader::new(&exec_store);
    let node_results = InMemoryNodeResultStore::new();
    let workflow_store = InMemoryWorkflowStore::new();
    let workflow_versions = InMemoryWorkflowVersionStore::new();

    let api_config = ApiConfig::for_test();

    let state = AppState::new(
        Arc::new(ScopedWorkflowStore::new(
            Arc::new(workflow_store.clone()),
            scope.clone(),
        )),
        Arc::new(ScopedWorkflowVersionStore::new(
            Arc::new(workflow_versions.clone()),
            scope.clone(),
        )),
        Arc::new(ScopedExecutionStore::new(
            Arc::new(exec_store.clone()),
            scope.clone(),
        )),
        Arc::new(ScopedNodeResultStore::new(
            Arc::new(node_results),
            scope.clone(),
        )),
        Arc::new(ScopedExecutionJournalReader::new(
            Arc::new(journal.clone()),
            scope.clone(),
        )),
        Arc::new(ScopedControlQueue::new(
            Arc::new(control_queue.clone()),
            scope,
        )),
        api_config.jwt_secret,
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver));

    (
        state,
        PortHandles {
            control_queue,
            exec_store,
            journal,
            workflow_store,
            workflow_versions,
        },
    )
}

/// Create an `AppState` plus a handle to the durable control queue so
/// tests can inspect the outbox via its non-consuming `snapshot()`.
pub(crate) async fn create_state_with_queue() -> (AppState, InMemoryControlQueue) {
    let (state, handles) = build_port_state().await;
    (state, handles.control_queue)
}

/// Alias for [`create_state_with_queue`], preserving the name used by
/// `integration_tests.rs` callers so no test body needs to change.
pub(crate) async fn create_test_state_with_queue() -> (AppState, InMemoryControlQueue) {
    create_state_with_queue().await
}

/// Like [`create_state_with_queue`] but also returns the [`PortHandles`]
/// bundle so a test can seed execution / workflow rows directly (the
/// port equivalent of the old `state.execution_repo` /
/// `state.workflow_repo` direct access).
pub(crate) async fn create_state_with_port_handles() -> (AppState, PortHandles) {
    build_port_state().await
}

/// Create an `AppState` wired through the scoped storage port, returning
/// the state and the raw `InMemoryControlQueue` handle. This is the
/// canonical §13-knife wiring; the asserted invariants are unchanged
/// from the pre-port path (see this file's header).
pub(crate) async fn create_state_with_port_queue() -> (AppState, InMemoryControlQueue) {
    create_state_with_queue().await
}
