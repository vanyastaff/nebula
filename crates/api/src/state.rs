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
    auth::AuthBackend, config::JwtSecret, errors::ApiError, middleware::IdempotencyStore,
    services::webhook::WebhookTransport,
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

/// Loads membership roles for RBAC middleware.
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
    /// See [`crate::auth::AuthBackend`] for the trait surface and
    /// [`crate::auth::InMemoryAuthBackend`] for the default impl.
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
    /// [`crate::services::webhook::bootstrap_webhook_activations`] before
    /// `build_app` to populate the transport's slug map. The same repo
    /// is consulted by the admin reload endpoint
    /// (`POST /internal/v1/webhooks/reload`).
    pub webhook_activation_repo: Option<Arc<dyn WebhookActivationRepo>>,

    /// Optional lifecycle event bus (M3.3 / ADR-0049 — E2).
    ///
    /// Producers (storage CRUD callsites) emit
    /// [`crate::services::webhook::TriggerLifecycleEvent`] on this
    /// bus; the transport-side subscriber reapplies the change
    /// without a full reload. M3.3 ships the consumer; producer
    /// wiring is deferred to a follow-up.
    pub trigger_lifecycle_bus: Option<crate::services::webhook::TriggerLifecycleBus>,

    /// Webhook credential resolver (M3.3 / ADR-0049 — E1+E3).
    ///
    /// Required for storage-driven slug bootstrap and admin reload.
    pub webhook_secret_resolver: Option<Arc<dyn crate::services::webhook::WebhookSecretResolver>>,

    /// Webhook ctx-template factory (M3.3 / ADR-0049 — E1+E3).
    pub webhook_ctx_factory: Option<Arc<dyn crate::services::webhook::WebhookContextFactory>>,

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
        }
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

    /// Persist a workflow definition with optimistic concurrency.
    /// Dual-dispatch: the spec-16 split (`version == 0` creates the
    /// workflow row at version 1 plus version record #1, else a CAS bump
    /// of the row to `version + 1` plus a new published version record)
    /// when the port is wired, else the legacy `WorkflowRepo::save`. A
    /// CAS miss maps to [`ApiError::Conflict`] with the exact message the
    /// legacy handler produced, so callers stay byte-identical.
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

        if version == 0 {
            // New workflow: row at version 1 + first version record
            // (number 1, published). The row slug is the workflow id
            // string — unique per tenant among active rows, which is all
            // the partial-unique index requires (this REST surface has
            // no author-facing slug concept).
            self.workflow_store
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
                .map_err(|e| match e {
                    nebula_storage_port::StorageError::Duplicate { .. } => conflict(),
                    other => ApiError::Internal(format!("Failed to create workflow: {other}")),
                })?;
            self.workflow_version_store
                .create(
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
                .map_err(|e| {
                    ApiError::Internal(format!("Failed to create workflow version: {e}"))
                })?;
            return Ok(());
        }

        // Existing workflow: CAS the row counter forward and append the
        // next published version record. `version` is the caller's
        // expected current counter; the new counter is `version + 1`.
        let next = version + 1;
        self.workflow_store
            .update(
                &scope,
                nebula_storage_port::dto::WorkflowRecord {
                    id: id_str.clone(),
                    scope: scope.clone(),
                    version: next,
                    slug: id_str.clone(),
                    deleted: false,
                },
                version,
            )
            .await
            .map_err(|e| match e {
                nebula_storage_port::StorageError::Conflict { .. }
                | nebula_storage_port::StorageError::NotFound { .. } => conflict(),
                other => ApiError::Internal(format!("Failed to update workflow: {other}")),
            })?;
        self.workflow_version_store
            .create(
                &scope,
                nebula_storage_port::dto::WorkflowVersionRecord {
                    workflow_id: id_str,
                    number: u32::try_from(next).unwrap_or(u32::MAX),
                    published: true,
                    pinned: false,
                    definition,
                },
            )
            .await
            .map_err(|e| ApiError::Internal(format!("Failed to create workflow version: {e}")))?;
        Ok(())
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
    /// OAuth via [`crate::auth::AuthBackend`].
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

    /// Attach a [`crate::services::webhook::TriggerLifecycleBus`]
    /// for slug-routed activation lifecycle events (M3.3 / ADR-0049).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_trigger_lifecycle_bus(
        mut self,
        bus: crate::services::webhook::TriggerLifecycleBus,
    ) -> Self {
        self.trigger_lifecycle_bus = Some(bus);
        self
    }

    /// Attach a webhook secret resolver (M3.3 / ADR-0049 — E1+E3).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_secret_resolver(
        mut self,
        resolver: Arc<dyn crate::services::webhook::WebhookSecretResolver>,
    ) -> Self {
        self.webhook_secret_resolver = Some(resolver);
        self
    }

    /// Attach a webhook ctx-template factory (M3.3 / ADR-0049 — E1+E3).
    #[must_use = "builder methods must be chained or built"]
    pub fn with_webhook_ctx_factory(
        mut self,
        factory: Arc<dyn crate::services::webhook::WebhookContextFactory>,
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
}
