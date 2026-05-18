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
use nebula_storage_port::Scope;
use nebula_storage_port::store::{
    ControlQueue, ExecutionJournalReader, ExecutionStore, NodeResultStore, WorkflowStore,
    WorkflowVersionStore,
};
use nebula_tenancy::{
    ScopedControlQueue, ScopedExecutionJournalReader, ScopedExecutionStore, ScopedNodeResultStore,
    ScopedWorkflowStore, ScopedWorkflowVersionStore,
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

    /// Test-only escape hatch for harnesses that exercise tenant routes
    /// without modeling memberships. Production composition leaves this
    /// false so missing RBAC state disables tenant routes fail-closed.
    pub allow_insecure_tenant_rbac_bypass: bool,

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

impl AppState {
    /// Create new AppState with provided dependencies.
    ///
    /// `jwt_secret` is a validated [`JwtSecret`]. Obtain one from
    /// [`crate::config::ApiConfig::from_env`] (production) or
    /// `ApiConfig::for_test` (tests with the `test-util` feature).
    /// The six handles are the **raw, undecorated** port adapters: the
    /// per-request tenant `Scope` is applied by the `AppState` accessors
    /// at call time (a fresh request-scoped `nebula-tenancy` decorator
    /// per call), not baked in once here — binding a fixed scope at
    /// construction is exactly what collapsed every tenant into one
    /// shared bucket. The spec-16 split stores a workflow's definition on
    /// its version records, so `workflow_store` and
    /// `workflow_version_store` are always wired together.
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
            allow_insecure_tenant_rbac_bypass: false,
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
    /// surface is the **in-memory storage port**: the raw
    /// [`nebula_storage::inmem`] adapters, stored undecorated.
    ///
    /// This is the single source of truth for the local-first port
    /// wiring (the composition root's `default_state` and the runnable
    /// `api_simple_server` example both build on it, instead of each
    /// re-deriving the six-handle stack). One shared execution-store core
    /// backs the control queue and journal so a `commit`/`enqueue` is
    /// observable through every reader; one workflow-version store
    /// instance is shared between the workflow-CRUD path and the
    /// resume/definition path so a version published via the workflow
    /// handlers is readable through the execution accessor.
    ///
    /// The handles are stored **without** a `nebula-tenancy` scope
    /// decorator: the per-request tenant `Scope` is applied by the
    /// `AppState` accessors at call time (a fresh request-scoped
    /// decorator per call), not baked in once at construction — the
    /// previous "bind a fixed placeholder scope here" wiring collapsed
    /// every tenant into one shared bucket.
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
            Arc::new(workflow_store),
            Arc::new(workflow_versions),
            Arc::new(exec_store),
            Arc::new(node_results),
            Arc::new(journal),
            Arc::new(control_queue),
            jwt_secret,
        )
    }

    /// Read a workflow's stored definition for the caller's tenant, or
    /// `None` if absent. The spec-16 split stores the definition on the
    /// highest-numbered published version record, read through a freshly
    /// bound decorator pair keyed by `scope`.
    pub(crate) async fn workflow_definition_scoped(
        &self,
        scope: &Scope,
        id: nebula_core::id::WorkflowId,
    ) -> Result<Option<serde_json::Value>, ApiError> {
        Ok(self
            .workflow_with_version_scoped(scope, id)
            .await?
            .map(|(_, definition)| definition))
    }

    /// Read a workflow's `(version, definition)`, or `None` if absent,
    /// scoped to the caller's tenant. The workflow row + its published
    /// version's definition are read through a freshly bound
    /// `nebula-tenancy` decorator pair keyed by `scope` (the
    /// confused-deputy boundary is the decorator, bound per request). The
    /// row carries no definition (spec-16 split), so a row with no
    /// published version is treated as absent — preserving the
    /// caller-visible "exists ⇒ has a definition" invariant.
    pub(crate) async fn workflow_with_version_scoped(
        &self,
        scope: &Scope,
        id: nebula_core::id::WorkflowId,
    ) -> Result<Option<(u64, serde_json::Value)>, ApiError> {
        let rows = ScopedWorkflowStore::new(Arc::clone(&self.workflow_store), scope.clone());
        let versions = ScopedWorkflowVersionStore::new(
            Arc::clone(&self.workflow_version_store),
            scope.clone(),
        );
        let id_str = id.to_string();
        let Some(row) = rows
            .get(scope, &id_str)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {e}")))?
        else {
            return Ok(None);
        };
        let published = versions
            .get_published(scope, &id_str)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to get workflow: {e}")))?;
        Ok(published.map(|v| (row.version, v.definition)))
    }

    /// Persist a workflow definition with optimistic concurrency, as a
    /// **single atomic unit of work** via
    /// [`WorkflowStore::save_with_published_version`], scoped to the
    /// caller's tenant.
    ///
    /// Spec-16 splits the workflow row from its version records and a
    /// row's definition lives on its published version — so a row without
    /// a published version is invisible to every reader ("the workflow
    /// vanished"). This commits the row and its published version
    /// together (one DB transaction per SQL backend, one mutex-guarded
    /// section in-memory) — both land or neither does. The atomic unit
    /// runs through a freshly bound `ScopedWorkflowStore`, so it commits
    /// into the caller's real tenant `scope` and the decorator rebinds
    /// the embedded record scope to it (a forged cross-tenant record
    /// cannot escape the bound tenant).
    ///
    /// `version == 0` creates the workflow row at version 1 plus version
    /// record #1; otherwise it CAS-bumps the row counter to `version + 1`
    /// and appends the new published version record. A CAS miss maps to
    /// [`ApiError::Conflict`] with the exact message the legacy handler
    /// produced, so callers stay byte-identical.
    pub(crate) async fn workflow_save_scoped(
        &self,
        scope: &Scope,
        id: nebula_core::id::WorkflowId,
        version: u64,
        definition: serde_json::Value,
    ) -> Result<(), ApiError> {
        let rows = ScopedWorkflowStore::new(Arc::clone(&self.workflow_store), scope.clone());
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

        rows.save_with_published_version(
            scope,
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

    /// Soft-delete a workflow scoped to the caller's tenant via
    /// `WorkflowStore::soft_delete` (a missing row ⇒ `false`). Returns
    /// `true` iff a row existed in the tenant and was removed.
    pub(crate) async fn workflow_delete_scoped(
        &self,
        scope: &Scope,
        id: nebula_core::id::WorkflowId,
    ) -> Result<bool, ApiError> {
        let rows = ScopedWorkflowStore::new(Arc::clone(&self.workflow_store), scope.clone());
        match rows.soft_delete(scope, &id.to_string()).await {
            Ok(()) => Ok(true),
            Err(nebula_storage_port::StorageError::NotFound { .. }) => Ok(false),
            Err(e) => Err(ApiError::Internal(format!(
                "Failed to delete workflow: {e}"
            ))),
        }
    }

    /// List workflows for the caller's tenant with pagination, ordered
    /// by `(created_at, id)`. Rows + each row's published definition are
    /// read through a freshly bound decorator pair, so the listing is the
    /// caller's tenant only. The spec-16 split has no `created_at` column,
    /// so the ordering is reconstructed from the definition JSON's
    /// `created_at` via the dual-format [`extract_timestamp`] helper
    /// (RFC3339 string or legacy i64), falling back to id order when
    /// absent or unparseable.
    ///
    /// [`extract_timestamp`]: crate::domain::workflow::handler::extract_timestamp
    pub(crate) async fn workflow_list_scoped(
        &self,
        scope: &Scope,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(nebula_core::id::WorkflowId, serde_json::Value)>, ApiError> {
        let rows_store = ScopedWorkflowStore::new(Arc::clone(&self.workflow_store), scope.clone());
        let versions = ScopedWorkflowVersionStore::new(
            Arc::clone(&self.workflow_version_store),
            scope.clone(),
        );
        let listed = rows_store
            .list(scope)
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to list workflows: {e}")))?;
        let mut out: Vec<(nebula_core::id::WorkflowId, i64, serde_json::Value)> =
            Vec::with_capacity(listed.len());
        for row in listed {
            let Some(published) = versions
                .get_published(scope, &row.id)
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
            // The definition JSON stores `created_at` as an RFC3339 string
            // (canonical `DateTime<Utc>` serialization), while the legacy
            // write path used a raw i64. `extract_timestamp` accepts both,
            // so the `(created_at, id)` ordering stays stable across either
            // shape — a plain `as_i64` would coerce every RFC3339 row to 0
            // and collapse the sort to id-only.
            let created = crate::domain::workflow::handler::extract_timestamp(
                &published.definition,
                "created_at",
            )
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

    /// Total workflow count for the caller's tenant (matches
    /// [`Self::workflow_list_scoped`]'s filter scope) via the
    /// `WorkflowStore::count` port — a `SELECT COUNT(*)` through a
    /// freshly bound decorator on the SQL backends, not an `O(n)`
    /// `list().len()` (pagination totals are on the hot path).
    pub(crate) async fn workflow_count_scoped(&self, scope: &Scope) -> Result<usize, ApiError> {
        let rows = ScopedWorkflowStore::new(Arc::clone(&self.workflow_store), scope.clone());
        rows.count(scope)
            .await
            .map(|n| usize::try_from(n).unwrap_or(usize::MAX))
            .map_err(|e| ApiError::Internal(format!("Failed to count workflows: {e}")))
    }

    /// List running execution ids for the caller's tenant — read
    /// through a freshly bound `ScopedExecutionStore`, so the listing is
    /// that tenant only.
    pub(crate) async fn list_running_executions_scoped(
        &self,
        scope: &Scope,
    ) -> Result<Vec<ExecutionId>, ApiError> {
        let store = ScopedExecutionStore::new(Arc::clone(&self.execution_store), scope.clone());
        let ids = store
            .list_running(scope)
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

    /// List running execution ids for one workflow within the caller's
    /// tenant (same per-request-scoped `ExecutionStore` as
    /// [`Self::list_running_executions_scoped`]).
    pub(crate) async fn list_running_executions_for_workflow_scoped(
        &self,
        scope: &Scope,
        workflow_id: nebula_core::id::WorkflowId,
    ) -> Result<Vec<ExecutionId>, ApiError> {
        let store = ScopedExecutionStore::new(Arc::clone(&self.execution_store), scope.clone());
        let ids = store
            .list_running_for_workflow(scope, &workflow_id.to_string())
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

    /// Read an execution's persisted `(version, state-json)` for the
    /// caller's tenant, or `None` if absent. Read through a freshly bound
    /// `ScopedExecutionStore`, so a cross-tenant id resolves to `None`
    /// (never another tenant's state). `context` labels the error.
    pub(crate) async fn execution_state_scoped(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        context: &str,
    ) -> Result<Option<(u64, serde_json::Value)>, ApiError> {
        let store = ScopedExecutionStore::new(Arc::clone(&self.execution_store), scope.clone());
        store
            .get(scope, &execution_id.to_string())
            .await
            .map(|opt| opt.map(|r| (r.version, r.state)))
            .map_err(|e| ApiError::Internal(format!("Failed to {context} execution: {e}")))
    }

    /// Enqueue a control command onto the durable outbox for the
    /// caller's tenant. The control message is enqueued through a freshly
    /// bound `ScopedControlQueue`, so the row is stamped with that tenant
    /// scope (a forged `scope` field on the message is rebound to the
    /// bound tenant). The §13-step-6 503-vs-500 error policy is
    /// centralized here.
    pub(crate) async fn enqueue_control_scoped(
        &self,
        scope: &Scope,
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

        let queue = ScopedControlQueue::new(Arc::clone(&self.control_queue), scope.clone());
        let msg = nebula_storage_port::dto::ControlMsg {
            id: *uuid::Uuid::new_v4().as_bytes(),
            execution_id: execution_id.to_string(),
            command,
            scope: scope.clone(),
            w3c_traceparent: w3c.as_ref().map(|c| c.traceparent().to_owned()),
            reclaim_count: 0,
        };
        queue.enqueue(&msg).await.map_err(|e| {
            use nebula_storage_port::StorageError;
            let unavailable = matches!(e, StorageError::Internal(_) | StorageError::Connection(_));
            to_api_err(unavailable, e.to_string())
        })
    }

    /// Create a fresh execution row for the caller's tenant — created
    /// through a freshly bound `ScopedExecutionStore`, so it lands in
    /// that tenant.
    pub(crate) async fn create_execution_scoped(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        workflow_id: nebula_core::id::WorkflowId,
        state_json: serde_json::Value,
    ) -> Result<(), ApiError> {
        let store = ScopedExecutionStore::new(Arc::clone(&self.execution_store), scope.clone());
        store
            .create(
                scope,
                &execution_id.to_string(),
                &workflow_id.to_string(),
                state_json,
            )
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to create execution: {e}")))
    }

    /// CAS-update an execution state and append one control message in the
    /// same storage-port commit. This is the API-side path for control
    /// commands that also pre-set execution state (`Cancel` / `Terminate`):
    /// either the state transition and outbox row both land, or neither does.
    pub(crate) async fn cas_transition_with_control_scoped(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
        expected_version: u64,
        new_state: serde_json::Value,
        command: nebula_storage_port::dto::ControlCommand,
        w3c: Option<nebula_core::W3cTraceContext>,
    ) -> Result<bool, ApiError> {
        let store = ScopedExecutionStore::new(Arc::clone(&self.execution_store), scope.clone());
        let id = execution_id.to_string();
        let current = store.get(scope, &id).await.map_err(|e| {
            ApiError::Internal(format!(
                "Failed to read execution for control transition: {e}"
            ))
        })?;
        let Some(record) = current else {
            return Ok(false);
        };
        let fencing =
            nebula_storage_port::FencingToken::from_generation(record.fencing.unwrap_or(0));
        let msg = nebula_storage_port::dto::ControlMsg {
            id: *uuid::Uuid::new_v4().as_bytes(),
            execution_id: id.clone(),
            command,
            scope: scope.clone(),
            w3c_traceparent: w3c.as_ref().map(|c| c.traceparent().to_owned()),
            reclaim_count: 0,
        };
        let batch = nebula_storage_port::TransitionBatch::builder()
            .scope(scope.clone())
            .execution_id(&id)
            .expected_version(expected_version)
            .fencing(fencing)
            .new_state(new_state)
            .outbox(vec![msg])
            .build()
            .map_err(|e| ApiError::Internal(format!("Failed to build control transition: {e}")))?;
        match store.commit(batch).await {
            Ok(nebula_storage_port::TransitionOutcome::Applied { .. }) => Ok(true),
            Ok(
                nebula_storage_port::TransitionOutcome::VersionConflict { .. }
                | nebula_storage_port::TransitionOutcome::FencedOut,
            ) => Ok(false),
            Err(e) => Err(ApiError::Internal(format!(
                "Failed to apply control transition: {e}"
            ))),
        }
    }

    /// Load all persisted per-node *outputs* for an execution within the
    /// caller's tenant — read through a freshly bound
    /// `ScopedNodeResultStore`, so a cross-tenant id yields nothing.
    pub(crate) async fn execution_node_outputs_scoped(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<Vec<(nebula_core::NodeKey, serde_json::Value)>, ApiError> {
        let store = ScopedNodeResultStore::new(Arc::clone(&self.node_result_store), scope.clone());
        let rows = store
            .load_all_node_outputs(scope, &execution_id.to_string())
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

    /// Load an execution's journal entries for the caller's tenant —
    /// read through a freshly bound `ScopedExecutionJournalReader`,
    /// confined to that tenant.
    pub(crate) async fn execution_journal_scoped(
        &self,
        scope: &Scope,
        execution_id: ExecutionId,
    ) -> Result<Vec<serde_json::Value>, ApiError> {
        let reader =
            ScopedExecutionJournalReader::new(Arc::clone(&self.journal_reader), scope.clone());
        reader
            .get_journal(scope, &execution_id.to_string())
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

    /// Allow tenant routes to pass RBAC without a membership store.
    ///
    /// This builder exists only for integration tests compiled with
    /// `feature = "test-util"`; production callers cannot opt into it.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use = "builder methods must be chained or built"]
    pub fn with_insecure_tenant_rbac_bypass_for_tests(mut self) -> Self {
        self.allow_insecure_tenant_rbac_bypass = true;
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
        // These tests only assert builder-slot wiring (no storage rows),
        // so fresh raw in-memory port adapters — exactly the production
        // composition shape post-decorator-removal — suffice.
        let exec_store = InMemoryExecutionStore::new();
        let control_queue = InMemoryControlQueue::new(&exec_store);
        let journal = InMemoryJournalReader::new(&exec_store);
        let workflow_versions = InMemoryWorkflowVersionStore::new();
        let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions);
        AppState::new(
            Arc::new(workflow_store),
            Arc::new(workflow_versions),
            Arc::new(exec_store),
            Arc::new(InMemoryNodeResultStore::new()),
            Arc::new(journal),
            Arc::new(control_queue),
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
