//! Application State
//!
//! Shared state for all handlers via Arc.
//! Contains only ports (traits) — independent of concrete implementations.

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use nebula_core::{OrgId, OrgRole, WorkspaceId, WorkspaceRole, id::ExecutionId, scope::Principal};
use nebula_credential::CredentialService;
use nebula_credential::PendingToken;
use nebula_engine::ActionRegistry;
use nebula_metrics::MetricsRegistry;
use nebula_plugin::PluginRegistry;
use nebula_storage::credential::InMemoryPendingStore;
use nebula_storage_port::Scope;
use nebula_storage_port::store::{
    ControlQueue, ExecutionJournalReader, ExecutionStore, NodeResultStore, TriggerDedupInbox,
    TriggerStore, WebhookActivationStore, WorkflowStore, WorkflowVersionStore,
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
/// ([`crate::domain::org::dto::MemberSummary`]) per 3. It carries
/// only what the RBAC store actually knows: *who* (the resolved
/// [`Principal`]) and *what role*. There is intentionally **no** `email`
/// or `joined_at` — the membership store is the RBAC role index, not a
/// user-identity directory, so synthesizing those fields would be a
/// honest capability contract false capability (the Phase-3 "Option 1" honest contract:
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

    /// Optional credential-schema port (credential-schema validation). When `None`, the
    /// credential write path and credential-type catalog return 503
    /// (honest capability stub, mirroring `action_registry`).
    pub credential_schema: Option<Arc<dyn crate::ports::credential_schema::CredentialSchemaPort>>,

    /// Optional `CredentialService` facade — the **single** credential
    /// persistence path (ADR-0088 D7). All credential CRUD, lifecycle, and
    /// acquisition operations route through it; the OAuth two-phase flow
    /// writes through a `CredentialScopeLayer` over the service's
    /// encryption+audit+cache store handle, so both planes share one store.
    ///
    /// When `None`, every credential endpoint returns an honest 503 —
    /// there is no raw-store fallback path.
    ///
    /// `CredentialService` is non-generic — its backend is erased behind
    /// `DynCredentialStore` / `ErasedPendingStore` at construction (ADR-0088 D4),
    /// so the api names it without a backend type parameter. The concrete
    /// backend is chosen by the composition root (the server binary), not here.
    pub credential_service: Option<Arc<CredentialService>>,

    /// Optional webhook HTTP transport. When `None`, no `/webhooks/*`
    /// routes are mounted on the app; webhook-style `WebhookAction`
    /// triggers registered via `ActionRegistry::register_webhook`
    /// will never fire until the transport is attached.
    pub webhook_transport: Option<WebhookTransport>,

    /// OAuth pending state store (API-owned OAuth flow §4.2 — TTL ≤ 10 min, single-use).
    pub oauth_pending_store: Arc<InMemoryPendingStore>,

    /// Maps signed state -> pending token so callback can consume pending data.
    pub oauth_state_tokens: Arc<RwLock<HashMap<String, PendingToken>>>,

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

    /// Optional outbound-email port for verification / password-reset
    /// flows. Composition roots wire a real transport here; the in-memory
    /// `InMemoryAuthBackend` keeps its own default
    /// [`crate::ports::email::EchoSink`] inbox when no port is provided,
    /// so leaving this `None` preserves legacy behaviour for callers that
    /// only build the dev backend.
    pub email_port: Option<Arc<dyn crate::ports::email::EmailPort>>,

    /// Publicly-reachable base URL for this Nebula instance, sourced
    /// from `ApiConfig::public_url` (`API_PUBLIC_URL` env). Required by
    /// the Plane-A OAuth handler to derive the canonical
    /// `redirect_uri` per ADR-0085 D-3 (recon-4) —
    /// `format!("{}/api/v1/auth/oauth/{}/callback", public_url, provider)`.
    ///
    /// Defaults to an empty string when constructed via
    /// [`Self::in_memory`]; the composition root (`build_state`) sets
    /// it from the parsed `ApiConfig`. Empty / relative values are
    /// rejected at boot when `auth.oauth.providers` is non-empty (T2.8
    /// REQ-compose-001 Invariant 1).
    pub public_url: Arc<str>,

    /// Optional membership store for RBAC role lookups.
    pub membership_store: Option<Arc<dyn MembershipStore>>,

    /// Test-only escape hatch for harnesses that exercise tenant routes
    /// without modeling memberships. Production composition leaves this
    /// false so missing RBAC state disables tenant routes fail-closed.
    allow_insecure_tenant_rbac_bypass: bool,

    /// Optional idempotency store backing [`crate::middleware::IdempotencyLayer`].
    ///
    /// When `Some`, `build_app` mounts the layer on `api_routes` (NOT on the
    /// merged webhook transport) so every state-changing API endpoint is
    /// replay-protected. When `None`, the layer is not mounted and POST
    /// endpoints have no replay protection — acceptable for tests that build
    /// minimal routers but a misconfiguration in production.
    ///
    /// See idempotency backend for the backend selection contract; the composition root
    /// chooses between [`crate::middleware::InMemoryIdempotencyStore`] and a
    /// PG-backed bridge (`StorageBackedIdempotencyStore<PgIdempotencyStore>`)
    /// based on `ApiConfig.idempotency.backend`.
    pub idempotency_store: Option<Arc<dyn IdempotencyStore>>,

    /// Optional trigger config store (ADR-0096 — spec-16 `port_triggers`).
    ///
    /// The **undecorated** base store. Per-request / per-tenant code wraps it
    /// in `nebula_tenancy::ScopedTriggerStore::new(store, scope)` at the call
    /// site; the bootstrap pathway calls through it via `TriggerStoreSpecLookup`
    /// which always supplies the activation row's own `scope`.
    ///
    /// Required for the webhook-bootstrap READ path: each `port_webhook_activations`
    /// row carries only routing/token/scope/workflow/mode data; the handler-build
    /// inputs (`provider`, `secret_id`, replay knobs) live in
    /// `port_triggers.config.webhook_activation`. Wire this alongside
    /// `webhook_activation_store` so `bootstrap_webhook_activations` can
    /// reconstruct a handler after a restart.
    ///
    /// When `None`, `bootstrap_webhook_activations` skips every row (spec lookup
    /// returns `None` for the absent store path) and `/internal/v1/webhooks/reload`
    /// returns 503.
    pub trigger_store: Option<Arc<dyn TriggerStore>>,

    /// Optional webhook-activation port store (ADR-0096 — B-world, spec-16 aligned).
    ///
    /// The undecorated base store; per-request code builds
    /// `ScopedWebhookActivationStore::new(store, scope)` where a tenant-scoped
    /// operation is needed. System-surface calls (`resolve_by_token`,
    /// `list_all_active`) go directly through the undecorated store.
    ///
    /// When `Some`, the mint-persist wrapper persists capability tokens at
    /// activation time and `resolve_by_token` is real (not conformance-only).
    /// When `None`, the fallback is the in-memory routing map only (tokens lost
    /// on restart — pre-ADR-0096 behaviour).
    pub webhook_activation_store: Option<Arc<dyn WebhookActivationStore>>,

    /// Trigger dedup inbox for durable webhook dispatch (ADR-0095 D1, U-D1.4b).
    ///
    /// Shared with the orchestrator's `JobDispatchQueue` — both must wrap the
    /// **same** underlying store so `claim_and_materialize_start` is atomic
    /// across the dedup-guard, Created-execution-row, and Start-job writes.
    ///
    /// When `Some` and the activation row is `mode=Prod`, incoming webhook
    /// events spawn durable executions via `DurableExecutionEmitter`.
    /// When `None`, Prod-mode webhooks are rejected fail-closed (5xx) rather
    /// than silently spawning dedup-blind.
    pub trigger_dedup_inbox: Option<Arc<dyn TriggerDedupInbox>>,

    /// Optional lifecycle event bus (webhook activation — E2).
    ///
    /// Producers (storage CRUD callsites) emit
    /// `TriggerLifecycleEvent`s on this bus; future subscribers reapply
    /// the change without a full reload. Producer wiring is deferred.
    pub trigger_lifecycle_bus: Option<crate::transport::webhook::TriggerLifecycleBus>,

    /// Webhook credential resolver (webhook activation — E1+E3).
    ///
    /// Required for storage-driven slug bootstrap and admin reload.
    pub webhook_secret_resolver: Option<Arc<dyn crate::transport::webhook::WebhookSecretResolver>>,

    /// B-world webhook ctx-template factory (ADR-0096 — E1+E3).
    ///
    /// Builds [`nebula_action::TriggerRuntimeContext`] from a B-world
    /// `WebhookActivationRecord` (carries `scope`, `trigger_id`, `workflow_id`).
    pub webhook_ctx_factory_b:
        Option<Arc<dyn crate::transport::webhook::WebhookActivationContextFactory>>,

    /// B-world trigger-config spec lookup (ADR-0096 — E1+E3).
    ///
    /// Resolves the `WebhookActivationSpec` stored in `triggers.config` for a
    /// given `trigger_id`. Required for bootstrap and admin reload — the B-world
    /// row is lean (routing + token only); handler-build inputs live in the trigger config.
    pub webhook_spec_lookup: Option<Arc<dyn crate::transport::webhook::TriggerSpecLookup>>,

    /// Internal-routes shared token (webhook activation — E3).
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
    /// enqueue durable outbox — durable control queue). Every control signal is
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
    /// [`ResourceActivatorRegistry::validate`](nebula_engine::ResourceActivatorRegistry::validate)
    /// runs the kind's `R::Config` schema + closed-set guard with **no**
    /// `nebula_resource::Manager` mutation — live registration stays an
    /// engine-activation concern (INTEGRATION_MODEL integration seam.1). When `None`,
    /// `create_resource` fails closed (422) rather than persist an
    /// unvalidated config. Set via
    /// [`AppState::with_resource_registrars`].
    pub resource_registrars: Option<Arc<nebula_engine::ResourceActivatorRegistry>>,

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
    /// engine-owned, INTEGRATION_MODEL integration seam.1). When `None`, the
    /// `GET .../resources/{res}/status` endpoint reports `503 Service
    /// Unavailable` (the catalog None-convention — never a fabricated
    /// status). Set via [`AppState::with_resource_status`]; compose in
    /// production from the same `nebula_resource::Manager` the engine is
    /// built with.
    pub resource_status: Option<Arc<dyn nebula_engine::EngineResourceStatus>>,

    /// Resume-token store for the W-S3d webhook→Resume producer
    /// (`POST /resume`).
    ///
    /// The **undecorated** (global) store — `consume` is a hash-keyed
    /// atomic delete with no tenant parameter; scope is read FROM the
    /// returned row (confused-deputy boundary is the absence of any
    /// tenant extractor on the resume handler).  Setting a
    /// `ScopedResumeTokenStore` here would be incorrect: the decorator
    /// adds per-scope filtering that the consume primitive intentionally
    /// lacks (`ResumeTokenStore` doc: "no `scope` parameter by design").
    ///
    /// When `None`, `POST /resume` returns `503 Service Unavailable`.
    /// Set via [`AppState::with_resume_token_store`].
    pub resume_token_store: Option<Arc<dyn nebula_storage_port::store::ResumeTokenStore>>,

    /// Resume producer for the W-S3d webhook→Resume path (`POST /resume`).
    ///
    /// `peek` is a read-only lookup (reject wrong-kind / expired tokens, surface
    /// storage faults as `503`); `consume_and_enqueue_resume` burns the token
    /// and enqueues the `Resume` in ONE transaction, closing the consume-then-
    /// enqueue durability gap (a failed enqueue rolls back, leaving the token
    /// live for retry).
    ///
    /// Like `resume_token_store`, this is the **undecorated** (global) store —
    /// the lookup is hash-keyed with no tenant parameter; scope is read FROM the
    /// row (confused-deputy boundary = the absence of any tenant extractor on
    /// the resume handler).
    ///
    /// When `None`, `POST /resume` returns `503 Service Unavailable`.
    /// Set via [`AppState::with_resume_producer`].
    pub resume_producer: Option<Arc<dyn nebula_storage_port::store::ResumeProducer>>,

    /// Rate limiters + clock for the W-S3d `POST /resume` handler.
    ///
    /// Bundled as [`crate::transport::webhook::resume::ResumeHandlerComponents`]
    /// so the three `WebhookRateLimiter` instances (IP / global / tenant)
    /// and the injectable clock are wired at the composition root rather
    /// than constructed per-request.  When `None`, `POST /resume` returns
    /// `503 Service Unavailable` (same fail-closed convention as
    /// `resume_token_store`).  Set via
    /// [`AppState::with_resume_handler_components`].
    pub resume_handler_components:
        Option<crate::transport::webhook::resume::ResumeHandlerComponents>,
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
            credential_service: None,
            webhook_transport: None,
            oauth_pending_store: Arc::new(InMemoryPendingStore::new()),
            oauth_state_tokens: Arc::new(RwLock::new(HashMap::new())),
            org_resolver: None,
            workspace_resolver: None,
            auth_backend: None,
            email_port: None,
            public_url: Arc::from(""),
            membership_store: None,
            allow_insecure_tenant_rbac_bypass: false,
            idempotency_store: None,
            trigger_store: None,
            webhook_activation_store: None,
            trigger_dedup_inbox: None,
            trigger_lifecycle_bus: None,
            webhook_secret_resolver: None,
            webhook_ctx_factory_b: None,
            webhook_spec_lookup: None,
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
            resume_token_store: None,
            resume_producer: None,
            resume_handler_components: None,
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
            InMemoryNodeResultStore, InMemoryTriggerDedupInbox, InMemoryWorkflowStore,
            InMemoryWorkflowVersionStore,
        };

        let exec_store = InMemoryExecutionStore::new();
        let control_queue = InMemoryControlQueue::new(&exec_store);
        let journal = InMemoryJournalReader::new(&exec_store);
        // TriggerDedupInbox must wrap the same shared core as the control
        // queue and journal: `claim_and_materialize_start` writes the dedup
        // guard, the Created execution row, and the Start job atomically in
        // one critical section — only possible when all three share the same
        // `Arc<Mutex<SharedState>>` (atomicity contract, durable_emitter.rs:106-108).
        // `new(&exec_store)` must be called BEFORE `Arc::new(exec_store)` moves
        // ownership — same ordering as `InMemoryControlQueue::new` and
        // `InMemoryJournalReader::new` above.
        let trigger_dedup_inbox = InMemoryTriggerDedupInbox::new(&exec_store);
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
        .with_trigger_dedup_inbox(Arc::new(trigger_dedup_inbox))
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
    /// bound tenant). The integration seam-step-6 503-vs-500 error policy is
    /// centralized here.
    pub(crate) async fn enqueue_control_scoped(
        &self,
        scope: &Scope,
        command: nebula_storage_port::dto::ControlCommand,
        execution_id: ExecutionId,
        w3c: Option<nebula_core::W3cTraceContext>,
    ) -> Result<(), ApiError> {
        // integration seam step 6: a backend that is intentionally absent or
        // unreachable (`Internal`/`Connection`) is a 503 (infra down,
        // not a logic bug); any other write failure is a 500.
        let to_api_err = |is_unavailable: bool, detail: String| {
            if is_unavailable {
                ApiError::ServiceUnavailable(format!(
                    "Execution {execution_id} persisted but control-queue backend is \
                     unavailable — orchestration absent (integration seam step 6, \
                     durable control queue orphan): {detail}"
                ))
            } else {
                ApiError::Internal(format!(
                    "Execution {execution_id} persisted but failed to enqueue control \
                     signal (durable control queue orphan — caller should retry): {detail}"
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
            // Untargeted Resume — the W-S3d targeted `/resume` producer builds
            // its own targeted `ControlMsg` via
            // `ResumeProducer::consume_and_enqueue_resume` instead.
            resume_target: None,
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
            // No targeted-Resume producer yet (W-S3d); enqueue untargeted.
            resume_target: None,
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

    /// Attach the credential-schema port (credential-schema validation) used to validate
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

    /// Attach the `CredentialService` facade — the single credential
    /// persistence path (CRUD, lifecycle, acquisition, and the OAuth
    /// two-phase writes all route through it). Without it every
    /// credential endpoint returns an honest 503.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_credential_service(mut self, service: Arc<CredentialService>) -> Self {
        self.credential_service = Some(service);
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

    /// Attach an outbound-email port for verification / password-reset
    /// flows.
    ///
    /// The default in-memory `AuthBackend` owns its own dev
    /// [`crate::ports::email::EchoSink`] inbox when this slot is `None`;
    /// production composition roots wire a real transport here so a
    /// storage-backed `AuthBackend` impl can delegate delivery to it.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_email_port(mut self, port: Arc<dyn crate::ports::email::EmailPort>) -> Self {
        self.email_port = Some(port);
        self
    }

    /// Set the publicly-reachable base URL for this Nebula instance.
    /// Required for Plane-A OAuth `redirect_uri` derivation per
    /// ADR-0085 D-3 (recon-4). Composition roots call this with
    /// `ApiConfig::public_url`. Tests can pass any absolute URL.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_public_url(mut self, public_url: impl Into<Arc<str>>) -> Self {
        self.public_url = public_url.into();
        self
    }

    /// Attach a membership store for RBAC role lookups.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_membership_store(mut self, store: Arc<dyn MembershipStore>) -> Self {
        self.membership_store = Some(store);
        self
    }

    /// Whether tenant RBAC is explicitly bypassed for test harnesses.
    pub(crate) const fn allow_insecure_tenant_rbac_bypass(&self) -> bool {
        self.allow_insecure_tenant_rbac_bypass
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
    /// See idempotency backend for the backend selection contract.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_idempotency_store(mut self, store: Arc<dyn IdempotencyStore>) -> Self {
        self.idempotency_store = Some(store);
        self
    }

    /// Attach the trigger config store (ADR-0096 — `port_triggers`).
    ///
    /// The **undecorated** base store. The bootstrap pathway wraps it in a
    /// `TriggerStoreSpecLookup` (see
    /// [`crate::transport::webhook::TriggerStoreSpecLookup`]) that enforces
    /// per-call scope binding via `nebula_tenancy::ScopedTriggerStore`, so
    /// the raw store passed here is never queried cross-tenant.
    ///
    /// In production: wire the same `TriggerStore` adapter as the trigger-
    /// CRUD endpoints. In tests: `nebula_storage::inmem::InMemoryTriggerStore`
    /// is the correct backing implementation (durable-enough for test fixtures,
    /// no bespoke double needed).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_trigger_store(mut self, store: Arc<dyn TriggerStore>) -> Self {
        self.trigger_store = Some(store);
        self
    }

    /// The store is the **undecorated** base implementation.  Per-request
    /// tenant-scoped operations build `ScopedWebhookActivationStore::new(store,
    /// scope)` at the call site; system-surface calls (`resolve_by_token`,
    /// `list_all_active`) call through the store directly.
    ///
    /// When wired, the mint-persist wrapper in
    /// [`crate::transport::webhook::activate_and_persist`] persists
    /// capability-token hashes at activation time and dispatch can resolve
    /// incoming tokens durably via `resolve_by_token`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_activation_store(mut self, store: Arc<dyn WebhookActivationStore>) -> Self {
        self.webhook_activation_store = Some(store);
        self
    }

    /// Attach the trigger-dedup inbox for durable webhook dispatch (ADR-0095 D1,
    /// U-D1.4b).
    ///
    /// **Atomicity requirement:** the inbox MUST share its underlying store with
    /// the orchestrator's `JobDispatchQueue` — both must wrap the same
    /// `Arc<Mutex<SharedState>>` (or equivalent transaction boundary) so
    /// `claim_and_materialize_start` can write the dedup guard, the Created
    /// execution row, and the Start job in one atomic critical section.
    ///
    /// [`AppState::in_memory`] wires this automatically via
    /// `InMemoryTriggerDedupInbox::new(&exec_store)` before the exec-store is
    /// moved into `Arc<dyn ExecutionStore>`.  Production composition roots must
    /// supply the same atomic wiring over their chosen backend (SQLite / PG).
    ///
    /// When `None`, Prod-mode webhook dispatches are rejected fail-closed (5xx)
    /// so dedup-blind spawns can never occur.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_trigger_dedup_inbox(mut self, inbox: Arc<dyn TriggerDedupInbox>) -> Self {
        self.trigger_dedup_inbox = Some(inbox);
        self
    }

    /// Attach a [`crate::transport::webhook::TriggerLifecycleBus`]
    /// for slug-routed activation lifecycle events (webhook activation).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_trigger_lifecycle_bus(
        mut self,
        bus: crate::transport::webhook::TriggerLifecycleBus,
    ) -> Self {
        self.trigger_lifecycle_bus = Some(bus);
        self
    }

    /// Attach a webhook secret resolver (webhook activation — E1+E3).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_secret_resolver(
        mut self,
        resolver: Arc<dyn crate::transport::webhook::WebhookSecretResolver>,
    ) -> Self {
        self.webhook_secret_resolver = Some(resolver);
        self
    }

    /// Attach the B-world webhook ctx-template factory (ADR-0096 — E1+E3).
    ///
    /// Builds a [`nebula_action::TriggerRuntimeContext`] from a
    /// `WebhookActivationRecord` (carries `scope`, `trigger_id`, `workflow_id`).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_ctx_factory_b(
        mut self,
        factory: Arc<dyn crate::transport::webhook::WebhookActivationContextFactory>,
    ) -> Self {
        self.webhook_ctx_factory_b = Some(factory);
        self
    }

    /// Attach the trigger-config spec lookup (ADR-0096 — E1+E3).
    ///
    /// Resolves the `WebhookActivationSpec` from `triggers.config` for a given
    /// `trigger_id`. Required for bootstrap and admin reload.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_spec_lookup(
        mut self,
        lookup: Arc<dyn crate::transport::webhook::TriggerSpecLookup>,
    ) -> Self {
        self.webhook_spec_lookup = Some(lookup);
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
    /// `nebula_resource::Manager` (INTEGRATION_MODEL integration seam.1). Compose this
    /// in production from the same registry the engine is built with
    /// ([`WorkflowEngine::resource_registrars`](nebula_engine::WorkflowEngine::resource_registrars));
    /// when left `None`, `create_resource` fails closed (422) rather than
    /// persist an unvalidated config.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_resource_registrars(
        mut self,
        registrars: Arc<nebula_engine::ResourceActivatorRegistry>,
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
    /// INTEGRATION_MODEL integration seam.1). Compose this in production from the same
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

    /// Attach the resume-token store for the W-S3d webhook→Resume
    /// producer (`POST /resume`).
    ///
    /// Must be the **undecorated** (global) store — consume is
    /// hash-keyed with no tenant parameter; scope comes FROM the row.
    /// When `None`, `POST /resume` returns `503 Service Unavailable`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_resume_token_store(
        mut self,
        store: Arc<dyn nebula_storage_port::store::ResumeTokenStore>,
    ) -> Self {
        self.resume_token_store = Some(store);
        self
    }

    /// Attach the resume producer for the W-S3d webhook→Resume path
    /// (`POST /resume`) — the atomic consume+enqueue seam (ADR-0099 Option B1).
    ///
    /// Must be the **undecorated** (global) producer backed by the SAME pool /
    /// shared state as the execution store and control queue, so the token
    /// DELETE and the `Resume` INSERT commit in one transaction. When `None`,
    /// `POST /resume` returns `503 Service Unavailable`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_resume_producer(
        mut self,
        producer: Arc<dyn nebula_storage_port::store::ResumeProducer>,
    ) -> Self {
        self.resume_producer = Some(producer);
        self
    }

    /// Attach rate-limiters + clock for the W-S3d `POST /resume` handler.
    ///
    /// When `None`, `POST /resume` returns `503 Service Unavailable`.
    /// Wire at the composition root alongside `with_resume_token_store`.
    #[must_use = "builder methods must be chained or built"]
    pub fn with_resume_handler_components(
        mut self,
        components: crate::transport::webhook::resume::ResumeHandlerComponents,
    ) -> Self {
        self.resume_handler_components = Some(components);
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
