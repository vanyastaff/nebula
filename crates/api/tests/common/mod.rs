//! Shared test helpers for `crates/api` integration tests.
//!
//! Each file under `tests/` is a separate Cargo compilation unit, so helpers
//! that are only used by one file look "dead" to the other. The `#[allow(dead_code)]`
//! attribute below suppresses those false-positive warnings.

#![allow(dead_code)]

use std::sync::Arc;

use nebula_api::{
    ApiConfig, AppState,
    error::ApiError,
    ports::credential_schema::{
        CredentialFieldError, CredentialSchemaPort, CredentialTypeDescriptor,
    },
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

/// Permissive [`CredentialSchemaPort`] test double: accepts any `data`,
/// exposes no catalog types. Wired by default into [`create_state_with_queue`]
/// so credential happy-path tests keep their pre-ADR-0052-P4 behavior
/// (the old fail-open persisted unvalidated; the correct test default is
/// "a validator is present and permissive"). The reject / no-port cases
/// are exercised explicitly by `tests/seam_credential_write_path_validation.rs`.
pub(crate) struct PermissiveCredentialSchemaPort;

impl CredentialSchemaPort for PermissiveCredentialSchemaPort {
    fn validate_data(
        &self,
        _credential_key: &str,
        _data: &serde_json::Value,
    ) -> Result<(), Vec<CredentialFieldError>> {
        Ok(())
    }

    fn list_types(&self) -> Vec<CredentialTypeDescriptor> {
        Vec::new()
    }

    fn get_type(&self, _credential_key: &str) -> Option<CredentialTypeDescriptor> {
        None
    }
}

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

/// The single tenant scope this harness operates under — exactly the
/// scope the API derives from a request to `/orgs/{TEST_ORG}/
/// workspaces/{TEST_WS}/…`.
///
/// `AppState` stores raw, undecorated port handles and each accessor
/// applies the per-request `&Scope` that `request_scope(&TenantContext)`
/// projects, i.e. `Scope::new(workspace_id, org_id)`. Tests seed rows
/// directly under this scope (`seed_*`) and the engine seam binds its
/// store handles to it (see `engine_seam`), so harness writes, engine
/// writes, and API reads all key on the same `(TEST_WS, TEST_ORG)`
/// tuple. The engine's internal `engine_scope()` placeholder is
/// substituted by the request-scope-bound decorator the seam wraps the
/// stores in (engine per-execution scoping is a separate, tracked
/// follow-up — see ADR-0072 "Known follow-up").
pub(crate) fn port_scope() -> Scope {
    Scope::new(TEST_WS, TEST_ORG)
}

/// Widen a short test label into the fixed 16-byte `ControlConsumer`
/// processor id. Explicit padding at the test boundary — the production
/// type is `[u8; 16]` so distinct workers can no longer silently
/// fence-collapse.
pub(crate) fn proc16(label: &[u8]) -> [u8; 16] {
    let mut id = [0u8; 16];
    let n = label.len().min(16);
    id[..n].copy_from_slice(&label[..n]);
    id
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
///
/// When `with_credential_port` is `true` a [`PermissiveCredentialSchemaPort`]
/// is wired (the credential happy-path default); when `false` the
/// credential-schema port is left unset so the ADR-0052 P4 seam test can
/// assert the unconfigured write path returns 503 (never persists
/// unvalidated).
async fn build_port_state_with(with_credential_port: bool) -> (AppState, PortHandles) {
    use nebula_storage::inmem::{
        InMemoryJournalReader, InMemoryNodeResultStore, InMemoryWorkflowStore,
        InMemoryWorkflowVersionStore,
    };
    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let journal = InMemoryJournalReader::new(&exec_store);
    let node_results = InMemoryNodeResultStore::new();
    let workflow_versions = InMemoryWorkflowVersionStore::new();
    let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions);

    let api_config = ApiConfig::for_test();

    // Raw (undecorated) port handles: the `AppState` accessors apply the
    // per-request tenant scope at call time. `PortHandles` keeps clones of
    // the same raw stores so `seed_*` writes are visible through the
    // accessors (the engine seam + direct seeds all use `port_scope()`).
    let state = AppState::new(
        Arc::new(workflow_store.clone()),
        Arc::new(workflow_versions.clone()),
        Arc::new(exec_store.clone()),
        Arc::new(node_results),
        Arc::new(journal.clone()),
        Arc::new(control_queue.clone()),
        api_config.jwt_secret,
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver))
    .with_insecure_tenant_rbac_bypass_for_tests();

    let state = if with_credential_port {
        state.with_credential_schema(Arc::new(PermissiveCredentialSchemaPort))
    } else {
        state
    };

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

/// Build a port-wired `AppState` with the permissive credential-schema
/// port (the credential happy-path default).
async fn build_port_state() -> (AppState, PortHandles) {
    build_port_state_with(true).await
}

/// Build a port-wired `AppState` for harnesses whose focus is auth /
/// membership / control-plane wiring rather than storage rows: every
/// store is an in-memory port adapter wrapped in the `nebula-tenancy`
/// scoping decorator (bound to [`port_scope`]) and passed to
/// [`AppState::new`], plus the slug resolvers (`TestOrgResolver` /
/// `TestWorkspaceResolver`) every harness needs. **No** credential-schema
/// port and no auth/membership store are wired — the caller layers the
/// `.with_*` it needs (`me_support` adds the auth backend; `org_support`
/// adds the membership store). Synchronous: port-store construction has
/// no `.await`, so the sync `create_*_without_*` harnesses can call this
/// directly.
pub(crate) fn build_me_state() -> AppState {
    use nebula_storage::inmem::{
        InMemoryJournalReader, InMemoryNodeResultStore, InMemoryWorkflowStore,
        InMemoryWorkflowVersionStore,
    };
    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let journal = InMemoryJournalReader::new(&exec_store);
    let node_results = InMemoryNodeResultStore::new();
    let workflow_versions = InMemoryWorkflowVersionStore::new();
    let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions);

    let api_config = ApiConfig::for_test();

    // Raw (undecorated) port handles — the `AppState` accessors apply the
    // per-request tenant scope at call time.
    AppState::new(
        Arc::new(workflow_store),
        Arc::new(workflow_versions),
        Arc::new(exec_store),
        Arc::new(node_results),
        Arc::new(journal),
        Arc::new(control_queue),
        api_config.jwt_secret,
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver))
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

/// Same as [`create_state_with_queue`] but with **no** credential-schema
/// port wired — for the ADR-0052 P4 seam test that asserts the
/// unconfigured write path returns 503 (never persists unvalidated).
/// Port-wired exactly like [`build_port_state`], minus the
/// `with_credential_schema` call.
pub(crate) async fn create_state_with_queue_no_credential_port() -> (AppState, InMemoryControlQueue)
{
    let (state, handles) = build_port_state_with(false).await;
    (state, handles.control_queue)
}

// ── `me/*` end-to-end harness (Phase 2) ──────────────────────────────────────
//
// Builds an `AppState` wired with a real `InMemoryAuthBackend` (the
// §4.5-honest production-quality Plane-A backend — Argon2id / RFC 6238
// TOTP / SHA-256 PAT lookup), registers one user, and hands back a JWT
// whose `sub` is that user's real `UserId`. The auth middleware's JWT
// path parses `sub` via `UserId::from_str` → `Principal::User`, so the
// full middleware → handler → `AuthBackend` port path is exercised
// end-to-end against a real backend (not a mock).

pub(crate) mod me_support {
    use std::{str::FromStr, sync::Arc};

    use nebula_api::{
        AppState,
        domain::{
            auth::backend::{AuthBackend, InMemoryAuthBackend, SignupRequest, dto::SecretString},
            org::InMemoryMembershipStore,
        },
    };
    use nebula_core::{OrgRole, Principal, UserId};

    use super::{TEST_JWT_SECRET, TEST_ORG, build_me_state};

    /// A registered user plus a JWT that authenticates *as that user*.
    pub(crate) struct MeUser {
        /// `user_<ULID>` string form (the JWT `sub`).
        pub(crate) user_id: String,
        /// Lowercased email used at registration.
        pub(crate) email: String,
        /// Bearer JWT whose `sub` is [`Self::user_id`].
        pub(crate) jwt: String,
    }

    /// Mint a JWT signed with [`TEST_JWT_SECRET`] for an explicit subject.
    pub(crate) fn jwt_for(subject: &str) -> String {
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
                sub: subject.to_owned(),
                exp: now + 3600,
                iat: now,
            },
            &EncodingKey::from_secret(TEST_JWT_SECRET.as_bytes()),
        )
        .unwrap()
    }

    /// Build an `AppState` wired with a real `InMemoryAuthBackend` **and**
    /// a real shared `InMemoryMembershipStore` seeded so the registered
    /// user is an `OrgMember` of exactly **one** org (`TEST_ORG`). Returns
    /// the state, the typed backend handle (for white-box assertions), and
    /// the user.
    ///
    /// The membership store is wired so the Phase-3 `orgs_count` /
    /// `list_my_orgs` graduations are exercised against a *real* count
    /// (not the honest-absent degradation). `/me/*` routes are **not**
    /// behind `rbac_middleware` (only auth + csrf — see `domain/mod.rs`),
    /// so seeding a membership store does not change any other `me/*`
    /// behaviour; it only feeds the two membership-backed reads.
    pub(crate) async fn create_me_state() -> (AppState, Arc<InMemoryAuthBackend>, MeUser) {
        let backend = Arc::new(InMemoryAuthBackend::new());
        let email = "me-e2e@nebula.dev".to_owned();
        let profile = backend
            .register_user(SignupRequest {
                email: email.clone(),
                password: SecretString::new("hunter22".to_owned()),
                display_name: "Me E2E".to_owned(),
            })
            .await
            .expect("register seed user");

        // Seed the registered user as a member of one org so the
        // membership-backed `me` reads return a real value.
        let seed_uid = UserId::from_str(&profile.user_id).expect("registered user id parses");
        let membership = InMemoryMembershipStore::seeded(
            TEST_ORG.parse().expect("valid test org id"),
            Principal::User(seed_uid),
            OrgRole::OrgMember,
        )
        .into_arc();

        let backend_dyn: Arc<dyn AuthBackend> = Arc::clone(&backend) as _;
        let membership_dyn: Arc<dyn nebula_api::state::MembershipStore> =
            Arc::clone(&membership) as _;
        let state = build_me_state()
            .with_auth_backend(backend_dyn)
            .with_membership_store(membership_dyn);

        let jwt = jwt_for(&profile.user_id);
        let user = MeUser {
            user_id: profile.user_id,
            email,
            jwt,
        };
        (state, backend, user)
    }

    /// Build an `AppState` whose `auth_backend` port is **absent** (for the
    /// honest 503 port-absent path). The JWT still parses to a `User`
    /// principal so the request reaches the handler, where the missing
    /// port is detected.
    pub(crate) fn create_me_state_without_backend() -> (AppState, String) {
        // `auth_backend` deliberately left unset (the honest 503
        // port-absent path). `build_me_state` already wires the slug
        // resolvers.
        let state = build_me_state();
        // A syntactically valid UserId so the JWT path yields
        // `Principal::User` and the request reaches the handler body.
        let jwt = jwt_for(&UserId::new().to_string());
        (state, jwt)
    }
}

// ── `org/*` member-management end-to-end harness (Phase 3) ───────────────────
//
// Builds an `AppState` whose `membership_store` is the real shared
// `InMemoryMembershipStore` (the SAME `Arc` `rbac_middleware` consults).
// Wiring a membership store ACTIVATES RBAC enforcement, so this builder is
// used ONLY by `org_e2e.rs` — it MUST NOT be added to
// `create_state_with_queue` / `create_me_state` (those power knife / me /
// idempotency / etc. and would regress to RBAC-404 with no seeded role).
//
// The org routes' JWT auth path turns the JWT `sub` into
// `Principal::User(UserId::from_str(sub))` *without* consulting any auth
// backend, so this harness needs no `AuthBackend` — only the seeded
// membership store + the slug resolvers (`TestOrgResolver` maps every
// slug to `TEST_ORG`).

pub(crate) mod org_support {
    use std::sync::Arc;

    use nebula_api::{AppState, domain::org::InMemoryMembershipStore};
    use nebula_core::{OrgRole, Principal, UserId};

    use super::{TEST_ORG, build_me_state, me_support::jwt_for};

    /// A principal with a known org role plus a JWT authenticating as it.
    pub(crate) struct OrgActor {
        /// `usr_<ULID>` string form (the JWT `sub`).
        pub(crate) user_id: String,
        /// The resolved principal (what RBAC + handlers see).
        pub(crate) principal: Principal,
        /// Bearer JWT whose `sub` is [`Self::user_id`].
        pub(crate) jwt: String,
    }

    impl OrgActor {
        /// Mint a fresh user principal (not yet a member of any org).
        pub(crate) fn new_user() -> Self {
            let uid = UserId::new();
            let user_id = uid.to_string();
            let jwt = jwt_for(&user_id);
            Self {
                user_id,
                principal: Principal::User(uid),
                jwt,
            }
        }
    }

    /// Build an `AppState` whose shared `InMemoryMembershipStore` is seeded
    /// with one **org admin** on `TEST_ORG`, plus the typed store handle
    /// (for white-box assertions) and the seeded-admin actor.
    ///
    /// `OrgAdmin` (not `OrgOwner`) is the default seed so the abuse tests
    /// can exercise both "admin cannot grant owner" (role-clamp) and the
    /// admin-level happy paths; tests that need an owner seed explicitly
    /// via [`seed_member`].
    pub(crate) fn create_org_state() -> (AppState, Arc<InMemoryMembershipStore>, OrgActor) {
        create_org_state_with_role(OrgRole::OrgAdmin)
    }

    /// Like [`create_org_state`] but the seeded actor gets `role`.
    pub(crate) fn create_org_state_with_role(
        role: OrgRole,
    ) -> (AppState, Arc<InMemoryMembershipStore>, OrgActor) {
        let admin = OrgActor::new_user();
        let org_id = TEST_ORG.parse().expect("valid test org id");
        let store =
            InMemoryMembershipStore::seeded(org_id, admin.principal.clone(), role).into_arc();
        let store_dyn: Arc<dyn nebula_api::state::MembershipStore> = Arc::clone(&store) as _;

        // `build_me_state` already wires the slug resolvers; this harness
        // adds the seeded membership store (which activates RBAC).
        let state = build_me_state().with_membership_store(store_dyn);

        (state, store, admin)
    }

    /// Seed an additional member directly into the shared store (bypasses
    /// the handler authz gate — fixture setup, not a request path).
    pub(crate) async fn seed_member(
        store: &InMemoryMembershipStore,
        principal: Principal,
        role: OrgRole,
    ) {
        let org_id = TEST_ORG.parse().expect("valid test org id");
        store.seed(org_id, principal, role).await;
    }

    /// Build an `AppState` whose `membership_store` is **absent** (`None`)
    /// — the exact shape `apps/server::compose::default_state` produces
    /// for org routes in the un-provisioned default binary (PR #671 P1
    /// fix: no auto-seed → no RBAC deadlock).
    ///
    /// With no store wired, `rbac_middleware`'s `is_some()` guard stays
    /// inert (the request is NOT 404'd — it reaches the handler), and the
    /// org member handler's port-absent path returns an honest 503
    /// (mirrors `me_support::create_me_state_without_backend` for the
    /// `auth_backend`-absent case). Returns the state plus a JWT whose
    /// `sub` resolves to a `Principal::User` so the request body is
    /// actually exercised.
    pub(crate) fn create_org_state_without_store() -> (AppState, String) {
        // No `.with_membership_store(...)` — exactly the default binary.
        // `build_me_state` already wires the slug resolvers.
        let state = build_me_state();
        let jwt = jwt_for(&UserId::new().to_string());
        (state, jwt)
    }
}

// ── Orchestration-absent control queue (canon §13 step 6) ─────────────────────

/// A scoped control-queue port whose `enqueue` always fails — simulates
/// the "orchestration backend unavailable" scenario (canon §13 step 6).
/// `StorageError::Internal` is the sentinel `cancel_execution` /
/// `terminate_execution` map to `ApiError::ServiceUnavailable` → HTTP 503.
#[derive(Debug)]
pub(crate) struct AlwaysFailControlQueue;

#[async_trait::async_trait]
impl nebula_storage_port::store::ControlQueue for AlwaysFailControlQueue {
    async fn enqueue(
        &self,
        _msg: &nebula_storage_port::dto::ControlMsg,
    ) -> Result<(), nebula_storage_port::StorageError> {
        Err(nebula_storage_port::StorageError::Internal(
            "control queue backend unavailable (simulated)".to_string(),
        ))
    }

    async fn claim_pending(
        &self,
        _processor: &[u8; 16],
        _batch_size: u32,
    ) -> Result<Vec<nebula_storage_port::dto::ControlMsg>, nebula_storage_port::StorageError> {
        Ok(vec![])
    }

    async fn mark_completed(
        &self,
        _id: &[u8; 16],
        _processor: &[u8; 16],
    ) -> Result<(), nebula_storage_port::StorageError> {
        Ok(())
    }

    async fn mark_failed(
        &self,
        _id: &[u8; 16],
        _processor: &[u8; 16],
        _error: &str,
    ) -> Result<(), nebula_storage_port::StorageError> {
        Ok(())
    }

    async fn reclaim_stuck(
        &self,
        _reclaim_after: std::time::Duration,
        _max_reclaim_count: u32,
    ) -> Result<nebula_storage_port::store::ReclaimOutcome, nebula_storage_port::StorageError> {
        Ok(nebula_storage_port::store::ReclaimOutcome::default())
    }

    async fn cleanup(
        &self,
        _retention: std::time::Duration,
    ) -> Result<u64, nebula_storage_port::StorageError> {
        Ok(0)
    }
}

/// Create an `AppState` wired through the storage port whose **control
/// queue** always fails on `enqueue` (canon §13 step 6). Every other
/// store is the normal in-memory port adapter behind its `nebula-tenancy`
/// scoping decorator — only the control queue is the always-fail double.
///
/// Returns the state plus the raw (undecorated) [`InMemoryExecutionStore`]
/// so the §13 step-6 test can seed a running execution row directly under
/// [`port_scope`] before asserting the enqueue-fails-503 path. The store is
/// `Clone` over an `Arc<Mutex<…>>`, so the returned handle shares state
/// with the scoping-decorated one inside `AppState`.
pub(crate) async fn create_state_with_failing_queue() -> (AppState, InMemoryExecutionStore) {
    use nebula_storage::inmem::{
        InMemoryJournalReader, InMemoryNodeResultStore, InMemoryWorkflowStore,
        InMemoryWorkflowVersionStore,
    };
    let exec_store = InMemoryExecutionStore::new();
    let journal = InMemoryJournalReader::new(&exec_store);
    let node_results = InMemoryNodeResultStore::new();
    let workflow_versions = InMemoryWorkflowVersionStore::new();
    let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions);

    let api_config = ApiConfig::for_test();

    // Raw (undecorated) port handles; the always-failing control queue is
    // wired directly so the enqueue-fails-503 path is asserted regardless
    // of the per-request scope the `AppState` accessors apply.
    let state = AppState::new(
        Arc::new(workflow_store),
        Arc::new(workflow_versions),
        Arc::new(exec_store.clone()),
        Arc::new(node_results),
        Arc::new(journal),
        Arc::new(AlwaysFailControlQueue),
        api_config.jwt_secret,
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver))
    .with_insecure_tenant_rbac_bypass_for_tests();

    (state, exec_store)
}

// ── Engine seam harness (canon §13 step 5 / ADR-0008 A3 / ADR-0016) ──────────
//
// Shared real-engine-consumer wiring for the durable-control-plane seam
// tests. The producer half (API enqueues `Cancel` / `Terminate`) is
// asserted per-test; this harness owns the CONSUMER half: a long-running
// cooperatively-cancellable node + the real `WorkflowEngine` +
// `ControlConsumer` + `EngineControlDispatch` over the same in-memory
// repos the API writes to.
//
// `knife.rs::knife_step5_engine_cancels_running_execution_end_to_end`
// (the canon §13 seam) and
// `execution_terminate_e2e.rs::terminate_engine_drives_running_execution_to_terminal_end_to_end`
// both source the wiring here so they differ ONLY in the final HTTP call
// (DELETE-cancel vs POST-terminate) and the command/terminal assertion.
// The wiring (action key `"slow"`, the `ActionExecutor` closure,
// `InProcessSandbox`, `ActionRuntime`, `EngineControlDispatch`,
// `ControlConsumer` with a 10ms poll interval and the `b"knife-a3"`
// processor id) is byte-behaviorally identical to the original inline
// knife step-5 wiring — the move is mechanical, not a behavior change.

pub(crate) mod engine_seam {
    use std::{sync::Arc, time::Duration};

    use nebula_api::AppState;
    use nebula_core::action_key;
    use nebula_engine::{
        ActionExecutor, ActionRegistry, ActionRuntime, ControlConsumer, DataPassingPolicy,
        EngineControlDispatch, InProcessSandbox, WorkflowEngine,
    };
    use nebula_tenancy::{
        ScopedControlQueue, ScopedExecutionJournalReader, ScopedExecutionStore,
        ScopedNodeResultStore, ScopedWorkflowStore, ScopedWorkflowVersionStore,
    };
    use nebula_workflow::{
        Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
    };
    use tokio::task::JoinHandle;
    use tokio_util::sync::CancellationToken;

    /// A cooperatively-cancellable `slow` action: it would otherwise sleep
    /// 30s, but exits immediately when `ctx.cancellation()` is tripped.
    /// Asserting an execution reaches a terminal state well inside the 30s
    /// window proves a `Cancel` / `Terminate` signal reached the engine's
    /// *live* frontier loop — not merely that the API CAS-flipped the row.
    pub(crate) struct SlowAction {
        pub(crate) started: Arc<tokio::sync::Notify>,
    }

    impl nebula_action::action::Action for SlowAction {
        type Input = serde_json::Value;
        type Output = serde_json::Value;

        fn metadata() -> &'static nebula_action::metadata::ActionMetadata {
            static M: std::sync::OnceLock<nebula_action::metadata::ActionMetadata> =
                std::sync::OnceLock::new();
            M.get_or_init(|| {
                nebula_action::metadata::ActionMetadata::new(
                    nebula_core::action_key!("seam.slow.static"),
                    "SlowAction",
                    "static",
                )
            })
        }
        fn dependencies() -> &'static nebula_core::Dependencies {
            static D: std::sync::OnceLock<nebula_core::Dependencies> = std::sync::OnceLock::new();
            D.get_or_init(nebula_core::Dependencies::new)
        }
    }
    impl nebula_action::stateless::StatelessAction for SlowAction {
        async fn execute(
            &self,
            input: <Self as nebula_action::action::Action>::Input,
            ctx: &(impl nebula_action::ActionContext + ?Sized),
        ) -> Result<
            nebula_action::result::ActionResult<<Self as nebula_action::action::Action>::Output>,
            nebula_action::ActionError,
        > {
            self.started.notify_one();
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(30)) => {
                    Ok(nebula_action::result::ActionResult::success(input))
                }
                () = ctx.cancellation().cancelled() => {
                    Err(nebula_action::ActionError::Cancelled)
                }
            }
        }
    }

    /// Persist a single-node workflow whose only node uses the `slow`
    /// action, returning its id. Mirrors the workflow the inline knife
    /// step-5 / terminate-e2e tests built by hand.
    pub(crate) async fn persist_slow_workflow(state: &AppState) -> nebula_core::WorkflowId {
        let workflow_id = nebula_core::WorkflowId::new();
        let now = chrono::Utc::now();
        let wf = WorkflowDefinition {
            id: workflow_id,
            name: "seam-slow".into(),
            description: None,
            version: Version::new(0, 1, 0),
            nodes: vec![
                NodeDefinition::new(nebula_core::node_key!("step"), "Step", "slow").unwrap(),
            ],
            connections: Vec::<Connection>::new(),
            variables: std::collections::HashMap::new(),
            config: WorkflowConfig::default(),
            trigger: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            owner_id: None,
            ui_metadata: None,
            schema_version: 1,
        };
        // Port equivalent of the old `state.workflow_repo.save(id, 0, def)`:
        // a workflow row at version 1 plus a published version record #1
        // through the scoped port handles on `AppState` (the tenancy
        // decorators substitute their bound scope, so the `port_scope()`
        // argument is immaterial — it only needs to be a valid `Scope`).
        let scope = super::port_scope();
        let id_str = workflow_id.to_string();
        state
            .workflow_store
            .create(
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
            .unwrap();
        state
            .workflow_version_store
            .create(
                &scope,
                nebula_storage_port::dto::WorkflowVersionRecord {
                    workflow_id: id_str,
                    number: 1,
                    published: true,
                    pinned: false,
                    definition: serde_json::to_value(&wf).unwrap(),
                },
            )
            .await
            .unwrap();
        workflow_id
    }

    /// Handle to the spawned engine-consumer harness. Drop-safe: call
    /// [`EngineSeam::shutdown`] at the end of the test so the spawned
    /// consumer task does not leak across tests.
    pub(crate) struct EngineSeam {
        /// Notified the moment the `slow` node enters its `select {}` —
        /// i.e. the consumer drained `Start` and the engine dispatched the
        /// node so the frontier loop is live.
        pub(crate) slow_started: Arc<tokio::sync::Notify>,
        shutdown: CancellationToken,
        consumer_handle: JoinHandle<()>,
    }

    impl EngineSeam {
        /// Graceful shutdown so the spawned consumer task doesn't leak.
        pub(crate) async fn shutdown(self) {
            self.shutdown.cancel();
            let _ = self.consumer_handle.await;
        }
    }

    /// Build the real `WorkflowEngine` + `ControlConsumer` +
    /// `EngineControlDispatch` over the shared in-memory repos and spawn
    /// the consumer so both `Start` and the later `Cancel` / `Terminate`
    /// are drained continuously.
    ///
    /// Byte-behaviorally identical to the original inline knife step-5
    /// wiring: same action key (`"slow"`), same `ActionExecutor` closure,
    /// `InProcessSandbox`, `ActionRuntime`, 10ms poll interval, and the
    /// `b"knife-a3"` processor id.
    pub(crate) fn spawn_engine_consumer(state: &AppState) -> EngineSeam {
        let slow_started = Arc::new(tokio::sync::Notify::new());
        let registry = Arc::new(ActionRegistry::new());
        registry.legacy_register_stateless_with_metadata(
            nebula_action::metadata::ActionMetadata::new(
                action_key!("slow"),
                "slow",
                "engine-seam cancellable handler",
            ),
            SlowAction {
                started: Arc::clone(&slow_started),
            },
        );

        let executor: ActionExecutor = Arc::new(|_ctx, _meta, input| {
            Box::pin(async move { Ok(nebula_action::result::ActionResult::success(input)) })
        });
        let sandbox = Arc::new(InProcessSandbox::new(executor));
        let metrics = nebula_metrics::MetricsRegistry::new();
        let runtime = Arc::new(
            ActionRuntime::try_new(
                registry,
                sandbox,
                DataPassingPolicy::default(),
                metrics.clone(),
            )
            .unwrap(),
        );

        // `AppState` now stores **raw** port handles and applies the
        // per-request tenant scope in its accessors; the engine, by
        // contrast, still calls its store handles with the internal
        // `engine_scope()` placeholder (a separate, tracked follow-up —
        // see ADR-0072 "Known follow-up: engine per-execution tenant
        // scoping"). To keep the seam coherent the engine-side handles
        // are wrapped here in `nebula-tenancy` decorators bound to
        // `port_scope()` — the request scope the API derives and the
        // tests seed under. The decorator substitutes its bound scope
        // for whatever the engine passes, so engine writes, harness
        // `seed_*` writes, and API reads all key on the same tenant.
        // This is the decorator's intended composition-seam use (the
        // security primitive, bound correctly), not a shim. The
        // slow-node seam never checkpoints or replays, so a fresh
        // in-memory checkpoint/idempotency pair suffices for the two
        // `ExecutionStores` fields `AppState` does not expose.
        let s = super::port_scope();
        let scoped_exec: Arc<dyn nebula_storage_port::store::ExecutionStore> = Arc::new(
            ScopedExecutionStore::new(Arc::clone(&state.execution_store), s.clone()),
        );
        let engine = Arc::new(
            WorkflowEngine::new(runtime, metrics)
                .unwrap()
                .with_execution_stores(nebula_engine::ExecutionStores {
                    execution: Arc::clone(&scoped_exec),
                    journal: Arc::new(ScopedExecutionJournalReader::new(
                        Arc::clone(&state.journal_reader),
                        s.clone(),
                    )),
                    node_results: Arc::new(ScopedNodeResultStore::new(
                        Arc::clone(&state.node_result_store),
                        s.clone(),
                    )),
                    checkpoints: Arc::new(nebula_storage::inmem::InMemoryCheckpointStore::new()),
                    idempotency: Arc::new(nebula_storage::inmem::InMemoryIdempotencyGuard::new()),
                })
                .with_workflow_stores(nebula_engine::WorkflowStores {
                    workflow: Arc::new(ScopedWorkflowStore::new(
                        Arc::clone(&state.workflow_store),
                        s.clone(),
                    )),
                    versions: Arc::new(ScopedWorkflowVersionStore::new(
                        Arc::clone(&state.workflow_version_store),
                        s.clone(),
                    )),
                }),
        );

        let dispatch = Arc::new(EngineControlDispatch::new(
            Arc::clone(&engine),
            Arc::clone(&scoped_exec),
        ));
        let consumer = ControlConsumer::new(
            Arc::new(ScopedControlQueue::new(Arc::clone(&state.control_queue), s)),
            dispatch,
            super::proc16(b"knife-a3"),
        )
        .with_poll_interval(Duration::from_millis(10));
        let shutdown = CancellationToken::new();
        let consumer_handle = consumer.spawn(shutdown.clone());

        EngineSeam {
            slow_started,
            shutdown,
            consumer_handle,
        }
    }
}
