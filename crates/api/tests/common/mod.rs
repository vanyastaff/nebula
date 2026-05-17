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
        ApiConfig, AppState,
        domain::{
            auth::backend::{AuthBackend, InMemoryAuthBackend, SignupRequest, dto::SecretString},
            org::InMemoryMembershipStore,
        },
    };
    use nebula_core::{OrgRole, Principal, UserId};
    use nebula_storage::{
        InMemoryExecutionRepo, InMemoryWorkflowRepo, repos::InMemoryControlQueueRepo,
    };

    use super::{TEST_JWT_SECRET, TEST_ORG, TestOrgResolver, TestWorkspaceResolver};

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
        let api_config = ApiConfig::for_test();
        let state = AppState::new(
            Arc::new(InMemoryWorkflowRepo::new()),
            Arc::new(InMemoryExecutionRepo::new()),
            Arc::new(InMemoryControlQueueRepo::new()),
            api_config.jwt_secret,
        )
        .with_org_resolver(Arc::new(TestOrgResolver))
        .with_workspace_resolver(Arc::new(TestWorkspaceResolver))
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
        let api_config = ApiConfig::for_test();
        let state = AppState::new(
            Arc::new(InMemoryWorkflowRepo::new()),
            Arc::new(InMemoryExecutionRepo::new()),
            Arc::new(InMemoryControlQueueRepo::new()),
            api_config.jwt_secret,
        )
        .with_org_resolver(Arc::new(TestOrgResolver))
        .with_workspace_resolver(Arc::new(TestWorkspaceResolver));
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

    use nebula_api::{ApiConfig, AppState, domain::org::InMemoryMembershipStore};
    use nebula_core::{OrgRole, Principal, UserId};
    use nebula_storage::{
        InMemoryExecutionRepo, InMemoryWorkflowRepo, repos::InMemoryControlQueueRepo,
    };

    use super::{TEST_ORG, TestOrgResolver, TestWorkspaceResolver, me_support::jwt_for};

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

        let api_config = ApiConfig::for_test();
        let state = AppState::new(
            Arc::new(InMemoryWorkflowRepo::new()),
            Arc::new(InMemoryExecutionRepo::new()),
            Arc::new(InMemoryControlQueueRepo::new()),
            api_config.jwt_secret,
        )
        .with_org_resolver(Arc::new(TestOrgResolver))
        .with_workspace_resolver(Arc::new(TestWorkspaceResolver))
        .with_membership_store(store_dyn);

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
        let api_config = ApiConfig::for_test();
        let state = AppState::new(
            Arc::new(InMemoryWorkflowRepo::new()),
            Arc::new(InMemoryExecutionRepo::new()),
            Arc::new(InMemoryControlQueueRepo::new()),
            api_config.jwt_secret,
        )
        .with_org_resolver(Arc::new(TestOrgResolver))
        .with_workspace_resolver(Arc::new(TestWorkspaceResolver));
        // No `.with_membership_store(...)` — exactly the default binary.
        let jwt = jwt_for(&UserId::new().to_string());
        (state, jwt)
    }
}

// ── Orchestration-absent control queue (canon §13 step 6) ─────────────────────

/// A control-queue repo that always fails on `enqueue` — used to simulate
/// the "orchestration backend unavailable" scenario (canon §13 step 6).
/// `StorageError::Internal` is the sentinel `cancel_execution` /
/// `terminate_execution` map to `ApiError::ServiceUnavailable` → HTTP 503.
pub(crate) struct AlwaysFailControlQueueRepo;

#[async_trait::async_trait]
impl nebula_storage::repos::ControlQueueRepo for AlwaysFailControlQueueRepo {
    async fn enqueue(
        &self,
        _entry: &nebula_storage::repos::ControlQueueEntry,
    ) -> Result<(), nebula_storage::StorageError> {
        Err(nebula_storage::StorageError::Internal(
            "control queue backend unavailable (simulated)".to_string(),
        ))
    }

    async fn claim_pending(
        &self,
        _processor: &[u8],
        _batch_size: u32,
    ) -> Result<Vec<nebula_storage::repos::ControlQueueEntry>, nebula_storage::StorageError> {
        Ok(vec![])
    }

    async fn mark_completed(
        &self,
        _id: &[u8],
        _processor: &[u8],
    ) -> Result<(), nebula_storage::StorageError> {
        Ok(())
    }

    async fn mark_failed(
        &self,
        _id: &[u8],
        _processor: &[u8],
        _error: &str,
    ) -> Result<(), nebula_storage::StorageError> {
        Ok(())
    }

    async fn reclaim_stuck(
        &self,
        _reclaim_after: std::time::Duration,
        _max_reclaim_count: u32,
    ) -> Result<nebula_storage::repos::ReclaimOutcome, nebula_storage::StorageError> {
        Ok(nebula_storage::repos::ReclaimOutcome::default())
    }

    async fn cleanup(
        &self,
        _retention: std::time::Duration,
    ) -> Result<u64, nebula_storage::StorageError> {
        Ok(0)
    }
}

/// Create an `AppState` wired with the always-failing control queue repo.
/// All other repos are fully functional in-memory implementations.
pub(crate) async fn create_state_with_failing_queue() -> AppState {
    let workflow_repo = Arc::new(InMemoryWorkflowRepo::new());
    let execution_repo = Arc::new(InMemoryExecutionRepo::new());
    let control_queue_repo: Arc<dyn nebula_storage::repos::ControlQueueRepo> =
        Arc::new(AlwaysFailControlQueueRepo);
    let api_config = ApiConfig::for_test();

    AppState::new(
        workflow_repo,
        execution_repo,
        control_queue_repo,
        api_config.jwt_secret,
    )
    .with_org_resolver(Arc::new(TestOrgResolver))
    .with_workspace_resolver(Arc::new(TestWorkspaceResolver))
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
        state
            .workflow_repo
            .save(workflow_id, 0, serde_json::to_value(&wf).unwrap())
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

        let engine = Arc::new(
            WorkflowEngine::new(runtime, metrics)
                .unwrap()
                .with_execution_repo(Arc::clone(&state.execution_repo))
                .with_workflow_repo(Arc::clone(&state.workflow_repo)),
        );

        let dispatch = Arc::new(EngineControlDispatch::new(
            Arc::clone(&engine),
            Arc::clone(&state.execution_repo),
        ));
        let consumer = ControlConsumer::new(
            Arc::clone(&state.control_queue_repo),
            dispatch,
            b"knife-a3".to_vec(),
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
