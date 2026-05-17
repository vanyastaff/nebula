//! Application State
//!
//! Shared state for all handlers via Arc.
//! Contains only ports (traits) — independent of concrete implementations.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use nebula_core::{OrgId, OrgRole, WorkspaceId, WorkspaceRole, id::ExecutionId, scope::Principal};
use nebula_credential::PendingToken;
use nebula_engine::ActionRegistry;
use nebula_metrics::MetricsRegistry;
use nebula_plugin::PluginRegistry;
use nebula_storage::{
    credential::{InMemoryPendingStore, InMemoryStore},
    repos::WebhookActivationRepo,
};
use nebula_storage_port::store::{
    ControlQueue, ExecutionJournalReader, ExecutionStore, NodeResultStore, WorkflowStore,
    WorkflowVersionStore,
};
use tokio::sync::RwLock;

use crate::{
    config::JwtSecret, domain::auth::backend::AuthBackend, error::ApiError,
    middleware::IdempotencyStore, transport::webhook::WebhookTransport,
};

// ── Port traits ──────────────────────────────────────────────────────────────

/// Resolves org identifiers (slug or ULID) to [`OrgId`].
#[async_trait]
pub trait OrgResolver: Send + Sync {
    /// Look up an org by its human-readable slug.
    async fn resolve_by_slug(&self, slug: &str) -> Result<OrgId, ApiError>;
}

/// Resolves workspace identifiers (slug or ULID) within an org to [`WorkspaceId`].
#[async_trait]
pub trait WorkspaceResolver: Send + Sync {
    /// Look up a workspace by its slug within the given org.
    async fn resolve_by_slug(&self, org_id: OrgId, slug: &str) -> Result<WorkspaceId, ApiError>;
}

/// One organisation membership row, as seen by the org/* handlers.
///
/// **Port-level type — deliberately decoupled from the wire DTO**
/// ([`crate::domain::org::dto::MemberSummary`]) per ADR-0047 §3. It carries
/// only what the RBAC store actually knows: *who* (the resolved
/// [`Principal`]) and *what role*. There is intentionally **no** `email`
/// or `joined_at` — the membership store is the RBAC role index, not a
/// user-identity directory, so synthesizing those fields would be a
/// canon §4.5 false capability (the Phase-3 "Option 1" honest contract:
/// see the module docs of [`crate::domain::org::handler`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrgMember {
    /// The member's resolved principal identity.
    pub principal: Principal,
    /// The member's org-level role.
    pub role: OrgRole,
}

/// Outcome of [`MembershipStore::add_member_guarded`].
///
/// The membership store enforces the **org-lockout invariant** ("an org
/// always retains ≥ 1 `OrgOwner`/`OrgAdmin`") atomically under its own
/// write lock, so a privilege-*reducing* upsert that would zero the
/// privileged set is refused at the seam — there is no check-then-act
/// window the handler could lose a race on. The handler maps each variant
/// to an HTTP status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddMemberOutcome {
    /// The member was added or their role upserted.
    Added,
    /// Refused: the write would have dropped the org's privileged
    /// (`OrgOwner | OrgAdmin`) set below one — permanent-lockout
    /// prevention. Maps to HTTP 409.
    WouldLockOut,
}

/// Outcome of [`MembershipStore::remove_member_guarded`].
///
/// Same atomic-seam contract as [`AddMemberOutcome`]: membership check,
/// lockout check, and the delete all happen under one write lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveMemberOutcome {
    /// The member row was removed.
    Removed,
    /// The principal was not a member of the org (handler maps to 404 —
    /// member existence is never disclosed cross-tenant).
    NotFound,
    /// Refused: removing this member would have dropped the org's
    /// privileged set below one. Maps to HTTP 409.
    WouldLockOut,
}

/// Membership role index for RBAC middleware **and** the org member-management
/// handlers.
///
/// This is the single contract that [`crate::middleware::rbac`] consults to
/// authorize every org/workspace request *and* that the
/// `GET/POST/DELETE /orgs/{org}/members` handlers read/write. A production
/// composition wires exactly one shared `Arc<dyn MembershipStore>` so a
/// membership added via [`Self::add_member_guarded`] is immediately visible
/// to the next RBAC check on the same process (no eventual-consistency
/// window — proven by
/// `tests/org_e2e.rs::added_member_is_immediately_rbac_authorized`).
///
/// Point lookups (`get_org_role` / `get_workspace_role`) stay on the hot
/// auth path; the enumeration/mutation methods back the member endpoints.
#[async_trait]
pub trait MembershipStore: Send + Sync {
    /// Return the caller's org-level role, if they are an org member.
    async fn get_org_role(
        &self,
        org_id: OrgId,
        principal: &Principal,
    ) -> Result<Option<OrgRole>, ApiError>;

    /// Return the caller's workspace-level role, if they are a workspace member.
    async fn get_workspace_role(
        &self,
        workspace_id: WorkspaceId,
        principal: &Principal,
    ) -> Result<Option<WorkspaceRole>, ApiError>;

    /// List every member of an org (`GET /orgs/{org}/members`).
    ///
    /// Returns role-index rows only — no user-directory fields (see
    /// [`OrgMember`]). Order is unspecified; the handler does not paginate
    /// (membership sets are bounded per org).
    async fn list_members(&self, org_id: OrgId) -> Result<Vec<OrgMember>, ApiError>;

    /// Low-level upsert primitive — **not** for request paths.
    ///
    /// Idempotent on `(org_id, principal)`. This performs **no**
    /// org-lockout check, so a request handler MUST use
    /// [`Self::add_member_guarded`] instead (the unguarded path could
    /// demote the last `OrgOwner`/`OrgAdmin` and permanently lock the org
    /// out). Retained only as a building block for seeding/tests and as
    /// the primitive `add_member_guarded` is implemented on top of.
    async fn add_member(
        &self,
        org_id: OrgId,
        principal: &Principal,
        role: OrgRole,
    ) -> Result<(), ApiError>;

    /// Low-level removal primitive — **not** for request paths.
    ///
    /// `Ok(true)` when a row was removed, `Ok(false)` when absent. This
    /// performs **no** org-lockout check; a request handler MUST use
    /// [`Self::remove_member_guarded`]. Retained as a seeding/test
    /// building block and the primitive the guarded variant builds on.
    async fn remove_member(&self, org_id: OrgId, principal: &Principal) -> Result<bool, ApiError>;

    /// Upsert a member **with the org-lockout invariant enforced
    /// atomically** (`POST /orgs/{org}/members`).
    ///
    /// The implementation MUST, under a single exclusive critical section
    /// (the in-memory impl: one write-guard; a future storage impl: one
    /// transaction), compute the post-write privileged
    /// (`OrgOwner | OrgAdmin`) count and refuse with
    /// [`AddMemberOutcome::WouldLockOut`] if the upsert would drop it
    /// below one — covering a privilege-*reducing* upsert of the **last**
    /// privileged principal whether that is the caller themselves or a
    /// cross-target. Otherwise it upserts and returns
    /// [`AddMemberOutcome::Added`]. All *policy* checks (admin gate,
    /// role-clamp, role-precedence) remain the handler's job; only the
    /// **lockout invariant** lives here, at the lock, so no check-then-act
    /// race can bypass it.
    async fn add_member_guarded(
        &self,
        org_id: OrgId,
        principal: &Principal,
        role: OrgRole,
    ) -> Result<AddMemberOutcome, ApiError>;

    /// Remove a member **with the org-lockout invariant enforced
    /// atomically** (`DELETE /orgs/{org}/members/{principal}`).
    ///
    /// Under one exclusive critical section: if the principal is not a
    /// member → [`RemoveMemberOutcome::NotFound`]; if removing them would
    /// drop the org's privileged set below one →
    /// [`RemoveMemberOutcome::WouldLockOut`]; otherwise remove and return
    /// [`RemoveMemberOutcome::Removed`]. The membership re-check inside
    /// the critical section collapses an existence TOCTOU to a clean
    /// `NotFound`; the lockout count is consistent with the delete because
    /// both happen under the same lock.
    async fn remove_member_guarded(
        &self,
        org_id: OrgId,
        principal: &Principal,
    ) -> Result<RemoveMemberOutcome, ApiError>;

    /// Enumerate every `(org, role)` the principal is a member of
    /// (`GET /me/orgs`, and the `MeResponse.orgs_count` source). Backs the
    /// Phase-2 carry-over: this is the principal→orgs enumeration that was
    /// structurally absent when `me/list_my_orgs` was first stubbed.
    async fn list_orgs_for_principal(
        &self,
        principal: &Principal,
    ) -> Result<Vec<(OrgId, OrgRole)>, ApiError>;
}

/// Application state passed through `Router::with_state`.
#[derive(Clone)]
pub struct AppState {
    /// JWT secret used to validate Bearer tokens.
    ///
    /// Wrapped in [`JwtSecret`] so construction enforces a
    /// 32-byte minimum length and rejects the well-known development
    /// placeholder. The middleware calls `as_bytes()` — same call
    /// shape as the previous `Arc<str>`.
    pub jwt_secret: JwtSecret,

    /// Static API keys accepted via `X-API-Key` header.
    ///
    /// Each key must use the `nbl_sk_` prefix. Compared in constant time.
    /// An empty `Vec` means API key auth is disabled for this route group.
    pub api_keys: Arc<Vec<String>>,

    /// Optional metrics registry for Prometheus export.
    /// When `None`, the `GET /metrics` endpoint returns 503.
    pub metrics_registry: Option<Arc<MetricsRegistry>>,

    /// Optional action registry for the action catalog endpoints.
    /// When `None`, the `GET /actions` endpoints return 503.
    pub action_registry: Option<Arc<ActionRegistry>>,

    /// Optional plugin registry for the plugin catalog endpoints.
    /// When `None`, the `GET /plugins` endpoints return 503.
    pub plugin_registry: Option<Arc<RwLock<PluginRegistry>>>,

    /// Optional credential-schema port (ADR-0052 P4). When `None`, the
    /// credential write path and credential-type catalog return 503
    /// (honest §4.5 stub, mirroring `action_registry`).
    pub credential_schema: Option<Arc<dyn crate::ports::credential_schema::CredentialSchemaPort>>,

    /// Optional webhook HTTP transport. When `None`, no `/webhooks/*`
    /// routes are mounted on the app; webhook-style `WebhookAction`
    /// triggers registered via `ActionRegistry::register_webhook`
    /// will never fire until the transport is attached.
    pub webhook_transport: Option<WebhookTransport>,

    /// OAuth pending state store (ADR-0031 §4.2 — TTL ≤ 10 min, single-use).
    pub oauth_pending_store: Arc<InMemoryPendingStore>,

    /// Maps signed state -> pending token so callback can consume pending data.
    pub oauth_state_tokens: Arc<RwLock<HashMap<String, PendingToken>>>,

    /// Credential state store used by OAuth callback completion.
    pub oauth_credential_store: Arc<InMemoryStore>,

    /// Optional org-slug → [`OrgId`] resolver.
    pub org_resolver: Option<Arc<dyn OrgResolver>>,

    /// Optional workspace-slug → [`WorkspaceId`] resolver.
    pub workspace_resolver: Option<Arc<dyn WorkspaceResolver>>,

    /// Optional Plane-A authentication backend.
    ///
    /// When `Some`, the auth middleware resolves session cookies and PATs
    /// through this single contract. When `None`, only JWT and `X-API-Key`
    /// authentication paths are available.
    ///
    /// See [`crate::domain::auth::backend::AuthBackend`] for the trait
    /// surface and [`crate::domain::auth::backend::InMemoryAuthBackend`] for
    /// the default impl.
    pub auth_backend: Option<Arc<dyn AuthBackend>>,

    /// Optional membership store for RBAC role lookups.
    pub membership_store: Option<Arc<dyn MembershipStore>>,

    /// Optional idempotency store backing [`crate::middleware::IdempotencyLayer`].
    ///
    /// When `Some`, `build_app` mounts the layer on `api_routes` (NOT on the
    /// merged webhook transport) so every state-changing API endpoint is
    /// replay-protected. When `None`, the layer is not mounted and POST
    /// endpoints have no replay protection — acceptable for tests that build
    /// minimal routers but a misconfiguration in production.
    ///
    /// See ADR-0048 for the backend selection contract; the composition root
    /// chooses between [`crate::middleware::InMemoryIdempotencyStore`] and a
    /// PG-backed bridge (`StorageBackedIdempotencyStore<PgIdempotencyStore>`)
    /// based on `ApiConfig.idempotency.backend`.
    pub idempotency_store: Option<Arc<dyn IdempotencyStore>>,

    /// Optional webhook-activation repository (M3.3 / ADR-0049).
    ///
    /// When `Some`, the composition root invokes
    /// [`crate::transport::webhook::bootstrap_webhook_activations`] before
    /// `build_app` to populate the transport's slug map. The same repo
    /// is consulted by the admin reload endpoint
    /// (`POST /internal/v1/webhooks/reload`).
    pub webhook_activation_repo: Option<Arc<dyn WebhookActivationRepo>>,

    /// Optional lifecycle event bus (M3.3 / ADR-0049 — E2).
    ///
    /// Producers (storage CRUD callsites) emit
    /// [`crate::transport::webhook::TriggerLifecycleEvent`] on this
    /// bus; the transport-side subscriber reapplies the change
    /// without a full reload. M3.3 ships the consumer; producer
    /// wiring is deferred to a follow-up.
    pub trigger_lifecycle_bus: Option<crate::transport::webhook::TriggerLifecycleBus>,

    /// Webhook credential resolver (M3.3 / ADR-0049 — E1+E3).
    ///
    /// Required for storage-driven slug bootstrap and admin reload.
    pub webhook_secret_resolver: Option<Arc<dyn crate::transport::webhook::WebhookSecretResolver>>,

    /// Webhook ctx-template factory (M3.3 / ADR-0049 — E1+E3).
    pub webhook_ctx_factory: Option<Arc<dyn crate::transport::webhook::WebhookContextFactory>>,

    /// Internal-routes shared token (M3.3 / ADR-0049 — E3).
    ///
    /// Required for `POST /internal/v1/webhooks/reload`. When `None`,
    /// every request to `/internal/v1/...` returns 503.
    pub internal_shared_token: Option<Arc<str>>,

    /// Spec-16 scoped execution-store port handle.
    ///
    /// Handlers read / transition execution state through this
    /// already-scoped port. The composition root wraps the raw adapter
    /// in the `nebula-tenancy` decorator so the handle is tenant-bound
    /// before it reaches `AppState`.
    pub execution_store: Arc<dyn ExecutionStore>,

    /// Spec-16 scoped workflow-version port handle (resume / definition
    /// lookup — the split model stores the definition here).
    pub workflow_version_store: Arc<dyn WorkflowVersionStore>,

    /// Spec-16 scoped workflow-row port handle. Workflow CRUD reads /
    /// mutates the workflow row + its versions through this scoped port
    /// pair; the spec-16 split stores the definition on version records,
    /// so this is always wired with [`Self::workflow_version_store`].
    pub workflow_store: Arc<dyn WorkflowStore>,

    /// Spec-16 scoped control-queue port handle (the cancel / start
    /// enqueue durable outbox — canon §12.2). Every control signal is
    /// enqueued here; the engine dispatcher drains it.
    pub control_queue: Arc<dyn ControlQueue>,

    /// Spec-16 scoped node-result port handle (per-node output reads on
    /// the outputs endpoint).
    pub node_result_store: Arc<dyn NodeResultStore>,

    /// Spec-16 scoped journal-reader port handle (execution log reads).
    pub journal_reader: Arc<dyn ExecutionJournalReader>,

    /// Optional resource repository for the resource catalog endpoints.
    ///
    /// When `None`, the resource catalog endpoints report `503 Service
    /// Unavailable`. Set via [`AppState::with_resource_repo`].
    pub resource_repo: Option<Arc<dyn nebula_storage::repos::ResourceRepo>>,

    /// Optional closed `kind → registrar` allowlist used to **validate**
    /// a resource config before it is persisted (`POST .../resources`).
    ///
    /// This is the config-CRUD validation seam, not engine activation:
    /// [`ResourceRegistrarRegistry::validate`](nebula_engine::ResourceRegistrarRegistry::validate)
    /// runs the kind's `R::Config` schema + closed-set guard with **no**
    /// `nebula_resource::Manager` mutation — live registration stays an
    /// engine-activation concern (INTEGRATION_MODEL §13.1). When `None`,
    /// `create_resource` fails closed (422) rather than persist an
    /// unvalidated config. Set via
    /// [`AppState::with_resource_registrars`].
    pub resource_registrars: Option<Arc<nebula_engine::ResourceRegistrarRegistry>>,

    /// Optional **read-only** resource runtime-status port.
    ///
    /// Projects a live resource's lifecycle phase in api-safe types
    /// ([`EngineResourceStatus`](nebula_engine::EngineResourceStatus) →
    /// [`ResourceRuntimeStatus`](nebula_engine::ResourceRuntimeStatus)) so
    /// the resource-status endpoint never imports `nebula-resource`
    /// (deny.toml `[[wrappers]]`: no upward deps from API). This is the
    /// status counterpart of the engine's resource accessor: it observes
    /// a phase and **cannot** mutate a resource — there is no
    /// acquire/release/drain seam here (resource lifecycle is
    /// engine-owned, INTEGRATION_MODEL §13.1). When `None`, the
    /// `GET .../resources/{res}/status` endpoint reports `503 Service
    /// Unavailable` (the catalog None-convention — never a fabricated
    /// status). Set via [`AppState::with_resource_status`]; compose in
    /// production from the same `nebula_resource::Manager` the engine is
    /// built with.
    pub resource_status: Option<Arc<dyn nebula_engine::EngineResourceStatus>>,
}

/// Fixed placeholder scope passed to scoped port handles.
///
/// `AppState`'s port handles are always wrapped in the
/// `nebula-tenancy` decorator, which **substitutes** its bound
/// (request-derived) tenant scope on every call and ignores the
/// argument. The concrete value here is therefore immaterial to
/// isolation — it only needs to be a valid [`Scope`].
fn placeholder_scope() -> nebula_storage_port::Scope {
    nebula_storage_port::Scope::new("nebula", "nebula")
}

impl AppState {
    /// Create new AppState with provided dependencies.
    ///
    /// `jwt_secret` is a validated [`JwtSecret`]. Obtain one from
    /// [`crate::config::ApiConfig::from_env`] (production) or
    /// `ApiConfig::for_test` (tests with the `test-util` feature).
    /// All six handles MUST already be wrapped in the `nebula-tenancy`
    /// scope-enforcing decorator (tenant-bound) by the composition root —
    /// `AppState` never sees a raw adapter. The spec-16 split stores a
    /// workflow's definition on its version records, so `workflow_store`
    /// and `workflow_version_store` are always wired together.
    pub fn new(
        workflow_store: Arc<dyn WorkflowStore>,
        workflow_version_store: Arc<dyn WorkflowVersionStore>,
        execution_store: Arc<dyn ExecutionStore>,
        node_result_store: Arc<dyn NodeResultStore>,
        journal_reader: Arc<dyn ExecutionJournalReader>,
        control_queue: Arc<dyn ControlQueue>,
        jwt_secret: JwtSecret,
    ) -> Self {
        Self {
            jwt_secret,
            api_keys: Arc::new(Vec::new()),
            metrics_registry: None,
            action_registry: None,
            plugin_registry: None,
            credential_schema: None,
            webhook_transport: None,
            oauth_pending_store: Arc::new(InMemoryPendingStore::new()),
            oauth_state_tokens: Arc::new(RwLock::new(HashMap::new())),
            oauth_credential_store: Arc::new(InMemoryStore::new()),
            org_resolver: None,
            workspace_resolver: None,
            auth_backend: None,
            membership_store: None,
            idempotency_store: None,
            webhook_activation_repo: None,
            trigger_lifecycle_bus: None,
            webhook_secret_resolver: None,
            webhook_ctx_factory: None,
            internal_shared_token: None,
            execution_store,
            workflow_version_store,
            workflow_store,
            control_queue,
            node_result_store,
            journal_reader,
            resource_repo: None,
            resource_registrars: None,
            resource_status: None,
        }
    }

    /// Build an `AppState` whose execution / workflow / control-queue
    /// surface is the **in-memory storage port**: the
    /// [`nebula_storage::inmem`] adapters wrapped in the `nebula-tenancy`
    /// scoping decorators, bound to the local-first placeholder scope.
    ///
    /// This is the single source of truth for the local-first port
    /// wiring (the composition root's `default_state` and the runnable
    /// `api_simple_server` example both build on it, instead of each
    /// re-deriving the six-handle decorator stack). One shared
    /// execution-store core backs the control queue and journal so a
    /// `commit`/`enqueue` is observable through every reader; one
    /// workflow-version store instance is shared between the
    /// workflow-CRUD path and the resume/definition path so a version
    /// published via the workflow handlers is readable through the
    /// execution accessor.
    ///
    /// `jwt_secret` is a validated [`JwtSecret`] — this constructor adds
    /// **no** auth bypass (it is not behind `test-util`); it only owns
    /// the in-memory persistence wiring. Identity/credential ports
    /// (`auth_backend`, `credential_schema`, …) are left unset and are
    /// wired by the caller via the `with_*` builders, exactly as with
    /// [`AppState::new`].
    #[must_use]
    pub fn in_memory(jwt_secret: JwtSecret) -> Self {
        use nebula_storage::inmem::{
            InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader,
            InMemoryNodeResultStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
        };
        use nebula_tenancy::{
            ScopedControlQueue, ScopedExecutionJournalReader, ScopedExecutionStore,
            ScopedNodeResultStore, ScopedWorkflowStore, ScopedWorkflowVersionStore,
        };

        let scope = placeholder_scope();

        let exec_store = InMemoryExecutionStore::new();
        let control_queue = InMemoryControlQueue::new(&exec_store);
        let journal = InMemoryJournalReader::new(&exec_store);
        let node_results = InMemoryNodeResultStore::new();
        // The workflow-row store shares the version store's map so
        // `workflow_save`'s atomic `save_with_published_version` commits
        // the row + version as one unit and the version-read path
        // observes the same data (mirrors the shared execution core
        // backing the control queue / journal above).
        let workflow_versions = InMemoryWorkflowVersionStore::new();
        let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions);

        Self::new(
            Arc::new(ScopedWorkflowStore::new(
                Arc::new(workflow_store),
                scope.clone(),
            )),
            Arc::new(ScopedWorkflowVersionStore::new(
                Arc::new(workflow_versions),
                scope.clone(),
            )),
            Arc::new(ScopedExecutionStore::new(
                Arc::new(exec_store),
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
            Arc::new(ScopedControlQueue::new(Arc::new(control_queue), scope)),
            jwt_secret,
        )
    }

    /// Read a workflow's stored definition, or `None` if absent.
    /// Dual-dispatch: the scoped spec-16 workflow stores (row +
    /// highest-numbered published version's `definition`) when wired,
    /// else the legacy `WorkflowRepo::get`. The definition lives on the
    /// version record in the split model.
    pub(crate) async fn workflow_definition(
        &self,
        id: nebula_core::id::WorkflowId,
    ) -> Result<Option<serde_json::Value>, ApiError> {
        Ok(self
            .workflow_with_version(id)
            .await?
            .map(|(_, definition)| definition))
    }

    /// Read a workflow's `(version, definition)`, or `None` if absent.
    /// Dual-dispatch: the spec-16 workflow-row `version` paired with its
    /// published version's `definition` when the port is wired, else the
    /// legacy `WorkflowRepo::get_with_version`. The workflow row carries
    /// no definition (spec-16 split), so a row with no published version
    /// is treated as absent — the legacy single-store always had a
    /// definition alongside its counter, so this preserves the
    /// caller-visible "exists ⇒ has a definition" invariant.
    pub(crate) async fn workflow_with_version(
        &self,
        id: nebula_core::id::WorkflowId,
    ) -> Result<Option<(u64, serde_json::Value)>, ApiError> {
        let scope = placeholder_scope();
        let id_str = id.to_string();
        let Some(row) = self
            .workflow_store
            .get(&scope, &id_str)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {e}")))?
        else {
            return Ok(None);
        };
        let published = self
            .workflow_version_store
            .get_published(&scope, &id_str)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {e}")))?;
        Ok(published.map(|v| (row.version, v.definition)))
    }

    /// Persist a workflow definition with optimistic concurrency, as a
    /// **single atomic unit of work** via
    /// [`WorkflowStore::save_with_published_version`].
    ///
    /// Spec-16 splits the workflow row from its version records and a
    /// row's definition lives on its published version — so a row without
    /// a published version is invisible to every reader ("the workflow
    /// vanished"). The previous two-await sequence (row write, then
    /// version write) left exactly that orphan window on any partial
    /// failure. This now commits the row and its published version
    /// together (one DB transaction per SQL backend, one mutex-guarded
    /// section in-memory) — both land or neither does.
    ///
    /// `version == 0` creates the workflow row at version 1 plus version
    /// record #1; otherwise it CAS-bumps the row counter to `version + 1`
    /// and appends the new published version record. A CAS miss maps to
    /// [`ApiError::Conflict`] with the exact message the legacy handler
    /// produced, so callers stay byte-identical.
    pub(crate) async fn workflow_save(
        &self,
        id: nebula_core::id::WorkflowId,
        version: u64,
        definition: serde_json::Value,
    ) -> Result<(), ApiError> {
        let scope = placeholder_scope();
        let id_str = id.to_string();
        let conflict =
            || ApiError::Conflict("Workflow was modified by another request".to_string());

        // `version == 0` → create (row v1 + version #1); else CAS the row
        // to `version + 1` and append version `version + 1`. The slug is
        // the workflow id string — unique per tenant among active rows,
        // all the partial-unique index requires (this REST surface has no
        // author-facing slug concept).
        let (row_version, ver_number, expected) = if version == 0 {
            (1u64, 1u32, None)
        } else {
            let next = version + 1;
            (next, u32::try_from(next).unwrap_or(u32::MAX), Some(version))
        };

        self.workflow_store
            .save_with_published_version(
                &scope,
                nebula_storage_port::dto::WorkflowRecord {
                    id: id_str.clone(),
                    scope: scope.clone(),
                    version: row_version,
                    slug: id_str.clone(),
                    deleted: false,
                },
                nebula_storage_port::dto::WorkflowVersionRecord {
                    workflow_id: id_str,
                    number: ver_number,
                    published: true,
                    pinned: false,
                    definition,
                },
                expected,
            )
            .await
            .map_err(|e| match e {
                // A row/version conflict, a missing row on CAS, or a
                // duplicate (create raced, or the version slot is taken)
                // all mean "modified by another request" — the exact
                // legacy message, byte-identical for callers.
                nebula_storage_port::StorageError::Conflict { .. }
                | nebula_storage_port::StorageError::NotFound { .. }
                | nebula_storage_port::StorageError::Duplicate { .. } => conflict(),
                other => ApiError::Internal(format!("Failed to save workflow: {other}")),
            })
    }

    /// Soft-delete a workflow via `WorkflowStore::soft_delete` (a missing
    /// row ⇒ `false`). Returns `true` iff a row existed and was removed.
    pub(crate) async fn workflow_delete(
        &self,
        id: nebula_core::id::WorkflowId,
    ) -> Result<bool, ApiError> {
        match self
            .workflow_store
            .soft_delete(&placeholder_scope(), &id.to_string())
            .await
        {
            Ok(()) => Ok(true),
            Err(nebula_storage_port::StorageError::NotFound { .. }) => Ok(false),
            Err(e) => Err(ApiError::Internal(format!(
                "Failed to delete workflow: {e}"
            ))),
        }
    }

    /// List workflows with pagination, ordered by `(created_at, id)`.
    /// The spec-16 split has no `created_at` column, so the ordering is
    /// reconstructed from the definition JSON's `created_at` (the
    /// handler writes it there), falling back to id order.
    pub(crate) async fn workflow_list(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(nebula_core::id::WorkflowId, serde_json::Value)>, ApiError> {
        let scope = placeholder_scope();
        let listed = self
            .workflow_store
            .list(&scope)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to list workflows: {e}")))?;
        let mut out: Vec<(nebula_core::id::WorkflowId, i64, serde_json::Value)> =
            Vec::with_capacity(listed.len());
        for row in listed {
            let Some(published) = self
                .workflow_version_store
                .get_published(&scope, &row.id)
                .await
                .map_err(|e| ApiError::Internal(format!("Failed to list workflows: {e}")))?
            else {
                // A row with no published version has no definition to
                // surface (mirrors `workflow_with_version`).
                continue;
            };
            let wid = nebula_core::id::WorkflowId::parse(&row.id).map_err(|e| {
                ApiError::Internal(format!("stored workflow id {:?} invalid: {e}", row.id))
            })?;
            let created = published
                .definition
                .get("created_at")
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(0);
            out.push((wid, created, published.definition));
        }
        // Contract: ORDER BY created_at, id.
        out.sort_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| a.0.to_string().cmp(&b.0.to_string()))
        });
        Ok(out
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|(id, _, def)| (id, def))
            .collect())
    }

    /// Total workflow count (matches [`Self::workflow_list`]'s filter
    /// scope) — the `WorkflowStore::list` length.
    pub(crate) async fn workflow_count(&self) -> Result<usize, ApiError> {
        self.workflow_store
            .list(&placeholder_scope())
            .await
            .map(|v| v.len())
            .map_err(|e| ApiError::Internal(format!("Failed to count workflows: {e}")))
    }

    /// List running execution ids through the scoped [`ExecutionStore`]
    /// port. The fixed placeholder scope is substituted by the
    /// `nebula-tenancy` decorator, so its value is immaterial to
    /// isolation.
    pub(crate) async fn list_running_executions(&self) -> Result<Vec<ExecutionId>, ApiError> {
        let ids = self
            .execution_store
            .list_running(&placeholder_scope())
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to list executions: {e}")))?;
        ids.iter()
            .map(|s| {
                ExecutionId::parse(s).map_err(|e| {
                    ApiError::Internal(format!("stored execution id {s:?} invalid: {e}"))
                })
            })
            .collect()
    }

    /// List running execution ids for one workflow (same scoped port as
    /// [`Self::list_running_executions`]).
    pub(crate) async fn list_running_executions_for_workflow(
        &self,
        workflow_id: nebula_core::id::WorkflowId,
    ) -> Result<Vec<ExecutionId>, ApiError> {
        let ids = self
            .execution_store
            .list_running_for_workflow(&placeholder_scope(), &workflow_id.to_string())
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to list executions: {e}")))?;
        ids.iter()
            .map(|s| {
                ExecutionId::parse(s).map_err(|e| {
                    ApiError::Internal(format!("stored execution id {s:?} invalid: {e}"))
                })
            })
            .collect()
    }

    /// Read an execution's persisted `(version, state-json)`, or `None`
    /// if absent, via the scoped [`ExecutionStore`] port (`get` →
    /// `(record.version, record.state)`). `context` labels the error
    /// (callers used distinct wording: "check" / "get" / …).
    pub(crate) async fn execution_state(
        &self,
        execution_id: ExecutionId,
        context: &str,
    ) -> Result<Option<(u64, serde_json::Value)>, ApiError> {
        self.execution_store
            .get(&placeholder_scope(), &execution_id.to_string())
            .await
            .map(|opt| opt.map(|r| (r.version, r.state)))
            .map_err(|e| ApiError::Internal(format!("Failed to {context} execution: {e}")))
    }

    /// Enqueue a control command onto the durable outbox via the scoped
    /// [`ControlQueue`] port (typed 16-byte id, opaque `execution_id`
    /// string, `traceparent` string — no UTF-8-of-ULID encoding). The
    /// §13-step-6 503-vs-500 error policy is centralized here so both
    /// enqueue sites stay identical.
    pub(crate) async fn enqueue_control(
        &self,
        command: nebula_storage_port::dto::ControlCommand,
        execution_id: ExecutionId,
        w3c: Option<nebula_core::W3cTraceContext>,
    ) -> Result<(), ApiError> {
        // §13 step 6: a backend that is intentionally absent or
        // unreachable (`Internal`/`Connection`) is a 503 (infra down,
        // not a logic bug); any other write failure is a 500.
        let to_api_err = |is_unavailable: bool, detail: String| {
            if is_unavailable {
                ApiError::ServiceUnavailable(format!(
                    "Execution {execution_id} persisted but control-queue backend is \
                     unavailable — orchestration absent (canon §13 step 6, §12.2 \
                     orphan): {detail}"
                ))
            } else {
                ApiError::Internal(format!(
                    "Execution {execution_id} persisted but failed to enqueue control \
                     signal (canon §12.2 orphan — caller should retry): {detail}"
                ))
            }
        };

        let msg = nebula_storage_port::dto::ControlMsg {
            id: *uuid::Uuid::new_v4().as_bytes(),
            execution_id: execution_id.to_string(),
            command,
            scope: placeholder_scope(),
            w3c_traceparent: w3c.as_ref().map(|c| c.traceparent().to_owned()),
            reclaim_count: 0,
        };
        self.control_queue.enqueue(&msg).await.map_err(|e| {
            use nebula_storage_port::StorageError;
            let unavailable = matches!(e, StorageError::Internal(_) | StorageError::Connection(_));
            to_api_err(unavailable, e.to_string())
        })
    }

    /// Create a fresh execution row via the scoped [`ExecutionStore`]
    /// port `create`.
    pub(crate) async fn create_execution(
        &self,
        execution_id: ExecutionId,
        workflow_id: nebula_core::id::WorkflowId,
        state_json: serde_json::Value,
    ) -> Result<(), ApiError> {
        self.execution_store
            .create(
                &placeholder_scope(),
                &execution_id.to_string(),
                &workflow_id.to_string(),
                state_json,
            )
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to create execution: {e}")))
    }

    /// CAS-update an execution's state, returning `false` on a
    /// version/fencing conflict (caller maps that to 409), via the
    /// scoped [`ExecutionStore`] port `commit`.
    ///
    /// The API is an *external* mutator (no held lease), so it reads the
    /// row's current fencing generation and commits at it. If a runner
    /// concurrently takes over (bumping the generation) the commit
    /// returns `FencedOut`, which maps to the same `Ok(false)` (retry) a
    /// version miss produces — the engine's reconciliation honors a
    /// concurrent terminal write (§11.5, #333).
    pub(crate) async fn cas_transition(
        &self,
        execution_id: ExecutionId,
        expected_version: u64,
        new_state: serde_json::Value,
    ) -> Result<bool, ApiError> {
        let scope = placeholder_scope();
        let id = execution_id.to_string();
        let current = self
            .execution_store
            .get(&scope, &id)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to cancel execution: {e}")))?;
        let Some(record) = current else {
            // No row: a CAS that can never match — caller treats
            // `false` as a 409 / refetch.
            return Ok(false);
        };
        let fencing =
            nebula_storage_port::FencingToken::from_generation(record.fencing.unwrap_or(0));
        let batch = nebula_storage_port::TransitionBatch::builder()
            .scope(scope)
            .execution_id(&id)
            .expected_version(expected_version)
            .fencing(fencing)
            .new_state(new_state)
            .build()
            .map_err(|e| ApiError::Internal(format!("Failed to build cancel transition: {e}")))?;
        match self.execution_store.commit(batch).await {
            Ok(nebula_storage_port::TransitionOutcome::Applied { .. }) => Ok(true),
            Ok(
                nebula_storage_port::TransitionOutcome::VersionConflict { .. }
                | nebula_storage_port::TransitionOutcome::FencedOut,
            ) => Ok(false),
            Err(e) => Err(ApiError::Internal(format!(
                "Failed to cancel execution: {e}"
            ))),
        }
    }

    /// Load all persisted per-node *outputs* for an execution via the
    /// scoped [`NodeResultStore`] port `load_all_node_outputs` (mapping
    /// `record.json`). Returns `Vec<(NodeKey, Value)>` (order-independent
    /// — callers re-key).
    pub(crate) async fn execution_node_outputs(
        &self,
        execution_id: ExecutionId,
    ) -> Result<Vec<(nebula_core::NodeKey, serde_json::Value)>, ApiError> {
        let rows = self
            .node_result_store
            .load_all_node_outputs(&placeholder_scope(), &execution_id.to_string())
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to load outputs: {e}")))?;
        rows.into_iter()
            .map(|(node_id, rec)| {
                nebula_core::NodeKey::new(&node_id)
                    .map(|k| (k, rec.json))
                    .map_err(|e| {
                        ApiError::Internal(format!("stored node id {node_id:?} invalid: {e}"))
                    })
            })
            .collect()
    }

    /// Load an execution's journal entries (opaque payloads) via the
    /// scoped [`ExecutionJournalReader`] port `get_journal` (mapping
    /// `entry.payload`).
    pub(crate) async fn execution_journal(
        &self,
        execution_id: ExecutionId,
    ) -> Result<Vec<serde_json::Value>, ApiError> {
        self.journal_reader
            .get_journal(&placeholder_scope(), &execution_id.to_string())
            .await
            .map(|entries| entries.into_iter().map(|e| e.payload).collect())
            .map_err(|e| ApiError::Internal(format!("Failed to load logs: {e}")))
    }

    /// Set the static API keys accepted via `X-API-Key` header.
    ///
    /// Each key should use the `nbl_sk_` prefix. Keys are compared in constant
    /// time inside the auth middleware.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_api_keys(mut self, keys: Vec<String>) -> Self {
        self.api_keys = Arc::new(keys);
        self
    }

    /// Attach a metrics registry for Prometheus export via `GET /metrics`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_metrics_registry(mut self, registry: Arc<MetricsRegistry>) -> Self {
        self.metrics_registry = Some(registry);
        self
    }

    /// Attach an action registry for the action catalog endpoints.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_action_registry(mut self, registry: Arc<ActionRegistry>) -> Self {
        self.action_registry = Some(registry);
        self
    }

    /// Attach a plugin registry for the plugin catalog endpoints.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_plugin_registry(mut self, registry: Arc<RwLock<PluginRegistry>>) -> Self {
        self.plugin_registry = Some(registry);
        self
    }

    /// Attach the credential-schema port (ADR-0052 P4) used to validate
    /// credential `data` before persist and to populate the credential-type
    /// catalog. When absent, those endpoints return an honest 503.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_credential_schema(
        mut self,
        port: Arc<dyn crate::ports::credential_schema::CredentialSchemaPort>,
    ) -> Self {
        self.credential_schema = Some(port);
        self
    }

    /// Attach a webhook HTTP transport. The router the transport
    /// exposes gets merged into the main app router in `build_app`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_transport(mut self, transport: WebhookTransport) -> Self {
        self.webhook_transport = Some(transport);
        self
    }

    /// Attach an org resolver for slug-to-ID lookups.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_org_resolver(mut self, resolver: Arc<dyn OrgResolver>) -> Self {
        self.org_resolver = Some(resolver);
        self
    }

    /// Attach a workspace resolver for slug-to-ID lookups.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_workspace_resolver(mut self, resolver: Arc<dyn WorkspaceResolver>) -> Self {
        self.workspace_resolver = Some(resolver);
        self
    }

    /// Attach a Plane-A authentication backend.
    ///
    /// Replaces the older `with_session_store` builder; the same slot now
    /// drives session resolution, password login, MFA, PATs, and Plane-A
    /// OAuth via [`crate::domain::auth::backend::AuthBackend`].
    #[must_use = "builder methods must be chained or built"]
    pub fn with_auth_backend(mut self, backend: Arc<dyn AuthBackend>) -> Self {
        self.auth_backend = Some(backend);
        self
    }

    /// Attach a membership store for RBAC role lookups.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_membership_store(mut self, store: Arc<dyn MembershipStore>) -> Self {
        self.membership_store = Some(store);
        self
    }

    /// Attach an idempotency store; `build_app` mounts
    /// [`crate::middleware::IdempotencyLayer`] on the API router when this is
    /// `Some`.
    ///
    /// See ADR-0048 for the backend selection contract.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_idempotency_store(mut self, store: Arc<dyn IdempotencyStore>) -> Self {
        self.idempotency_store = Some(store);
        self
    }

    /// Attach a webhook-activation repository (M3.3 / ADR-0049).
    ///
    /// Required for storage-driven slug bootstrap and for the admin
    /// reload endpoint. Composition roots that do not enable
    /// `WebhookApiConfig::bootstrap_from_storage` may leave this
    /// `None`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_activation_repo(mut self, repo: Arc<dyn WebhookActivationRepo>) -> Self {
        self.webhook_activation_repo = Some(repo);
        self
    }

    /// Attach a [`crate::transport::webhook::TriggerLifecycleBus`]
    /// for slug-routed activation lifecycle events (M3.3 / ADR-0049).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_trigger_lifecycle_bus(
        mut self,
        bus: crate::transport::webhook::TriggerLifecycleBus,
    ) -> Self {
        self.trigger_lifecycle_bus = Some(bus);
        self
    }

    /// Attach a webhook secret resolver (M3.3 / ADR-0049 — E1+E3).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_secret_resolver(
        mut self,
        resolver: Arc<dyn crate::transport::webhook::WebhookSecretResolver>,
    ) -> Self {
        self.webhook_secret_resolver = Some(resolver);
        self
    }

    /// Attach a webhook ctx-template factory (M3.3 / ADR-0049 — E1+E3).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_ctx_factory(
        mut self,
        factory: Arc<dyn crate::transport::webhook::WebhookContextFactory>,
    ) -> Self {
        self.webhook_ctx_factory = Some(factory);
        self
    }

    /// Attach the internal-routes shared token. Required for
    /// `POST /internal/v1/webhooks/reload`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_internal_shared_token(mut self, token: impl Into<Arc<str>>) -> Self {
        self.internal_shared_token = Some(token.into());
        self
    }

    /// Attach a resource repository for the resource catalog endpoints.
    ///
    /// When `None`, the resource catalog endpoints report `503 Service
    /// Unavailable`. Compose this in production with the Postgres-backed
    /// implementation; leave `None` in tests that exercise unrelated
    /// routes.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_resource_repo(
        mut self,
        repo: Arc<dyn nebula_storage::repos::ResourceRepo>,
    ) -> Self {
        self.resource_repo = Some(repo);
        self
    }

    /// Attach the closed `kind → registrar` allowlist used to validate a
    /// resource config before persistence.
    ///
    /// This is the config-CRUD validation seam (schema + closed-kind),
    /// **not** engine activation: it never live-registers into a
    /// `nebula_resource::Manager` (INTEGRATION_MODEL §13.1). Compose this
    /// in production from the same registry the engine is built with
    /// ([`WorkflowEngine::resource_registrars`](nebula_engine::WorkflowEngine::resource_registrars));
    /// when left `None`, `create_resource` fails closed (422) rather than
    /// persist an unvalidated config.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_resource_registrars(
        mut self,
        registrars: Arc<nebula_engine::ResourceRegistrarRegistry>,
    ) -> Self {
        self.resource_registrars = Some(registrars);
        self
    }

    /// Attach the **read-only** resource runtime-status port backing
    /// `GET .../resources/{res}/status`.
    ///
    /// Projects a live resource's lifecycle phase in api-safe types — it
    /// observes a phase and cannot mutate a resource (no
    /// acquire/release/drain; resource lifecycle is engine-owned,
    /// INTEGRATION_MODEL §13.1). Compose this in production from the same
    /// `nebula_resource::Manager` the engine is built with (via
    /// `nebula_engine::EngineManagerResourceStatus`). When left `None`
    /// the status endpoint reports `503` rather than fabricate a status.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_resource_status(
        mut self,
        status: Arc<dyn nebula_engine::EngineResourceStatus>,
    ) -> Self {
        self.resource_status = Some(status);
        self
    }
}

#[cfg(test)]
mod tests {
    use nebula_storage::inmem::{
        InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader,
        InMemoryNodeResultStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
    };
    use nebula_tenancy::{
        ScopedControlQueue, ScopedExecutionJournalReader, ScopedExecutionStore,
        ScopedNodeResultStore, ScopedWorkflowStore, ScopedWorkflowVersionStore,
    };

    use super::*;

    /// Minimal fake that satisfies `Arc<dyn ResourceRepo>` inside the test module.
    /// Production code never touches this; it only proves the builder slot is wired.
    struct FakeResourceRepo;

    #[async_trait::async_trait]
    impl nebula_storage::repos::ResourceRepo for FakeResourceRepo {
        async fn create(
            &self,
            _resource: &nebula_storage::repos::ResourceEntry,
        ) -> Result<(), nebula_storage::StorageError> {
            Ok(())
        }

        async fn get(
            &self,
            _id: &[u8],
        ) -> Result<Option<nebula_storage::repos::ResourceEntry>, nebula_storage::StorageError>
        {
            Ok(None)
        }

        async fn get_by_slug(
            &self,
            _workspace_id: &[u8],
            _slug: &str,
        ) -> Result<Option<nebula_storage::repos::ResourceEntry>, nebula_storage::StorageError>
        {
            Ok(None)
        }

        async fn update(
            &self,
            _resource: &nebula_storage::repos::ResourceEntry,
            expected_version: i64,
        ) -> Result<i64, nebula_storage::StorageError> {
            // The store owns the post-CAS increment; the fake mirrors the
            // contract by returning `expected_version + 1`.
            Ok(expected_version + 1)
        }

        async fn soft_delete(&self, _id: &[u8]) -> Result<(), nebula_storage::StorageError> {
            Ok(())
        }

        async fn list(
            &self,
            _workspace_id: &[u8],
            _offset: u64,
            _limit: u64,
        ) -> Result<Vec<nebula_storage::repos::ResourceEntry>, nebula_storage::StorageError>
        {
            Ok(vec![])
        }
    }

    fn base_state() -> AppState {
        let jwt = JwtSecret::new("test-secret-for-state-module-tests-0123456789")
            .expect("static test secret is valid");
        // These tests only assert builder-slot wiring (no storage rows), so
        // fresh in-memory port adapters behind the tenancy scoping
        // decorators — exactly the production composition shape — suffice.
        let scope = placeholder_scope();
        let exec_store = InMemoryExecutionStore::new();
        let control_queue = InMemoryControlQueue::new(&exec_store);
        let journal = InMemoryJournalReader::new(&exec_store);
        AppState::new(
            Arc::new(ScopedWorkflowStore::new(
                Arc::new(InMemoryWorkflowStore::new()),
                scope.clone(),
            )),
            Arc::new(ScopedWorkflowVersionStore::new(
                Arc::new(InMemoryWorkflowVersionStore::new()),
                scope.clone(),
            )),
            Arc::new(ScopedExecutionStore::new(
                Arc::new(exec_store),
                scope.clone(),
            )),
            Arc::new(ScopedNodeResultStore::new(
                Arc::new(InMemoryNodeResultStore::new()),
                scope.clone(),
            )),
            Arc::new(ScopedExecutionJournalReader::new(
                Arc::new(journal),
                scope.clone(),
            )),
            Arc::new(ScopedControlQueue::new(Arc::new(control_queue), scope)),
            jwt,
        )
    }

    #[test]
    fn with_resource_repo_sets_field() {
        let repo: Arc<dyn nebula_storage::repos::ResourceRepo> = Arc::new(FakeResourceRepo);
        let st = base_state().with_resource_repo(Arc::clone(&repo));
        assert!(
            st.resource_repo.is_some(),
            "resource_repo must be Some after with_resource_repo"
        );
    }

    #[test]
    fn resource_repo_defaults_to_none() {
        let st = base_state();
        assert!(
            st.resource_repo.is_none(),
            "resource_repo must default to None"
        );
    }
}
