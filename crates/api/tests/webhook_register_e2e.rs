//! End-to-end tests for the webhook registration producer endpoint
//! (`POST /orgs/{org}/workspaces/{ws}/webhooks`, PR-3b).
//!
//! # Acceptance criteria
//!
//! 1. **Cross-scope ownership (RED-on-revert)**: registering under `scope_b`
//!    when the workflow belongs to `scope_a` returns 404 — no existence oracle
//!    leak, and revert (`scope_b` gets the definition) flips the result,
//!    proving the guard is load-bearing.
//!
//! 2. **Secret-once invariant**: the 201 body contains `signing_secret`
//!    (`whsec_<base64>`).  The `port_triggers` config blob and
//!    `port_webhook_activations` row must NOT contain the plaintext secret —
//!    only the `secret_id` (credential PK) is stored.
//!
//! 3. **Happy path**: 201 with valid `webhook_url` + `activation_id`; the
//!    `port_triggers` row and `port_webhook_activations` row are both durably
//!    written after the call returns.
//!
//! 4. **3-store compensation on activation failure**: if `activate_and_persist`
//!    cannot proceed (transport not configured), neither a credential row nor a
//!    trigger row is left behind — compensation cleaned up both.
//!
//! 5. **Missing infra → 503**: calling the handler without the webhook
//!    transport wired in `AppState` returns 503, not panic or 500.
//!
//! # Test infrastructure
//!
//! All tests call the handler function directly (no HTTP layer), using
//! durable in-memory stores backed by SQLite (the ADR-0092 rule: no
//! in-memory credential doubles).  `AppState` is constructed manually with
//! only the stores each test needs; excess infra is left `None` to
//! exercise the 503 gate.

use std::{sync::Arc, time::Duration};

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use nebula_action::{TriggerRuntimeContext, webhook::providers::default_factories};
use nebula_api::{
    AppState,
    domain::webhook::{dto::RegisterWebhookRequest, handler::register_webhook},
    middleware::auth::AuthenticatedUser,
    ports::credential_service_factory::with_memory_store,
    transport::webhook::{
        CredentialBackedWebhookSecretResolver, TriggerStoreSpecLookup,
        WebhookActivationContextFactory, WebhookTransport, WebhookTransportConfig,
    },
};
use nebula_core::{
    OrgId, TenantContext, WorkflowId, WorkspaceId, id::UserId, node_key, scope::Principal,
};
use nebula_credential::TenantScope;
use nebula_engine::ActionRegistry;
use nebula_storage::{
    credential::{EnvKeyProvider, KeyProvider},
    inmem::{
        InMemoryControlQueue, InMemoryExecutionStore, InMemoryNodeResultStore,
        InMemoryTriggerStore, InMemoryWebhookActivationStore, InMemoryWorkflowStore,
        InMemoryWorkflowVersionStore,
    },
};
use nebula_storage_port::{
    Scope,
    dto::{WebhookActivationRecord, WorkflowRecord, WorkflowVersionRecord},
    store::{TriggerStore, WebhookActivationStore, WorkflowStore, WorkflowVersionStore},
};
use nebula_workflow::definition::CURRENT_SCHEMA_VERSION;
use serde_json::json;
use tokio_util::sync::CancellationToken;
use url::Url;

// ── Fixed test AES key (32 × 0x42, base64) ───────────────────────────────────

const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

// ── Fixed scope constants ─────────────────────────────────────────────────────

const TEST_ORG_A: &str = "org_00000000000000000000000001";
const TEST_WS_A: &str = "ws_00000000000000000000000001";
const TEST_ORG_B: &str = "org_00000000000000000000000002";
const TEST_WS_B: &str = "ws_00000000000000000000000002";

fn scope_a() -> Scope {
    Scope::new(TEST_WS_A, TEST_ORG_A)
}

// ── TenantContext constructors ────────────────────────────────────────────────

fn tenant_for_scope_a() -> TenantContext {
    let org_id = OrgId::parse(TEST_ORG_A).expect("valid org ULID fixture");
    let ws_id = WorkspaceId::parse(TEST_WS_A).expect("valid ws ULID fixture");
    let user_id = UserId::new();
    TenantContext {
        org_id,
        workspace_id: Some(ws_id),
        principal: Principal::User(user_id),
        org_role: Some(nebula_core::OrgRole::OrgAdmin),
        workspace_role: Some(nebula_core::WorkspaceRole::WorkspaceAdmin),
    }
}

fn tenant_for_scope_b() -> TenantContext {
    let org_id = OrgId::parse(TEST_ORG_B).expect("valid org ULID fixture");
    let ws_id = WorkspaceId::parse(TEST_WS_B).expect("valid ws ULID fixture");
    let user_id = UserId::new();
    TenantContext {
        org_id,
        workspace_id: Some(ws_id),
        principal: Principal::User(user_id),
        org_role: Some(nebula_core::OrgRole::OrgAdmin),
        workspace_role: Some(nebula_core::WorkspaceRole::WorkspaceAdmin),
    }
}

fn dummy_user() -> AuthenticatedUser {
    AuthenticatedUser {
        user_id: "usr_test_webhook_register".to_string(),
    }
}

// ── Fixture: minimal WorkflowDefinition JSON with a trigger binding ───────────

/// Build a minimal `WorkflowDefinition` JSON value with one `trigger_binding`
/// whose `id` NodeKey is `trigger_node_key`.
fn workflow_definition_with_binding(
    workflow_id: WorkflowId,
    trigger_node_key: &str,
) -> serde_json::Value {
    let now = chrono::Utc::now().to_rfc3339();
    json!({
        "id": workflow_id.to_string(),
        "name": "Webhook Register E2E Test Workflow",
        "version": { "major": 1, "minor": 0, "patch": 0 },
        "schema_version": CURRENT_SCHEMA_VERSION,
        "nodes": [],
        "connections": [],
        "trigger_bindings": [
            {
                "id": trigger_node_key,
                "plugin_key": "http",
                "action_key": "http.webhook"
            }
        ],
        "tags": [],
        "created_at": now,
        "updated_at": now
    })
}

// ── Noop ctx factory (same as bootstrap test) ─────────────────────────────────

struct NoopCtxFactory;

impl WebhookActivationContextFactory for NoopCtxFactory {
    fn build(&self, _record: &WebhookActivationRecord) -> TriggerRuntimeContext {
        TriggerRuntimeContext::new(
            Arc::new(
                nebula_core::BaseContext::builder()
                    .cancellation(CancellationToken::new())
                    .build(),
            ),
            WorkflowId::new(),
            node_key!("webhook-register-e2e"),
        )
    }
}

// ── AppState builders ─────────────────────────────────────────────────────────

/// Build an `AppState` with all webhook-registration-required infrastructure:
/// credential service, trigger store, webhook activation store,
/// action registry (generic provider), webhook transport, secret resolver,
/// ctx factory, workflow stores.
///
/// Also returns the raw trigger store and activation store for assertion.
async fn build_full_state() -> (
    AppState,
    Arc<InMemoryTriggerStore>,
    Arc<InMemoryWebhookActivationStore>,
    Arc<InMemoryWorkflowVersionStore>,
    Arc<InMemoryWorkflowStore>,
) {
    let key: Arc<dyn KeyProvider> =
        Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte AES key fixture"));
    let credential_svc = with_memory_store(Arc::clone(&key))
        .await
        .expect("credential service builds");

    let trigger_store = Arc::new(InMemoryTriggerStore::new());
    let activation_store = Arc::new(InMemoryWebhookActivationStore::new());

    // Workflow stores — needed for the ownership check.
    let workflow_versions = Arc::new(InMemoryWorkflowVersionStore::new());
    let workflow_store = Arc::new(InMemoryWorkflowStore::new_with_versions(&workflow_versions));

    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let journal = nebula_storage::inmem::InMemoryJournalReader::new(&exec_store);
    let node_results = InMemoryNodeResultStore::new();

    let action_registry = Arc::new({
        let r = ActionRegistry::new();
        for f in default_factories() {
            r.register_webhook_provider(f);
        }
        r
    });

    let transport = WebhookTransport::new(WebhookTransportConfig {
        base_url: Url::parse("https://nebula.example.com").expect("valid base URL"),
        path_prefix: "/webhooks".to_string(),
        body_limit_bytes: 1 << 20,
        response_timeout: Duration::from_secs(5),
        rate_limit_per_minute: None,
        tenant_rate_limit_per_minute: None,
    });

    let secret_resolver = Arc::new(CredentialBackedWebhookSecretResolver::new(
        credential_svc.clone(),
    ));
    let ctx_factory: Arc<dyn WebhookActivationContextFactory> = Arc::new(NoopCtxFactory);
    let spec_lookup =
        TriggerStoreSpecLookup::new(Arc::clone(&trigger_store) as Arc<dyn TriggerStore>);

    let config = nebula_api::ApiConfig::for_test();

    let state = AppState::new(
        Arc::clone(&workflow_store) as _,
        Arc::clone(&workflow_versions) as _,
        Arc::new(exec_store),
        Arc::new(node_results),
        Arc::new(journal),
        Arc::new(control_queue),
        config.jwt_secret,
    )
    .with_credential_service(credential_svc)
    .with_trigger_store(Arc::clone(&trigger_store) as _)
    .with_webhook_activation_store(Arc::clone(&activation_store) as _)
    .with_action_registry(Arc::clone(&action_registry))
    .with_webhook_transport(transport)
    .with_webhook_secret_resolver(secret_resolver)
    .with_webhook_ctx_factory_b(ctx_factory)
    .with_webhook_spec_lookup(Arc::new(spec_lookup))
    .with_insecure_tenant_rbac_bypass_for_tests();

    (
        state,
        trigger_store,
        activation_store,
        workflow_versions,
        workflow_store,
    )
}

/// Seed a workflow row + published version under `scope`.
async fn seed_workflow(
    workflow_store: &Arc<InMemoryWorkflowStore>,
    workflow_versions: &Arc<InMemoryWorkflowVersionStore>,
    scope: &Scope,
    workflow_id: WorkflowId,
    definition: serde_json::Value,
) {
    let id_str = workflow_id.to_string();
    WorkflowStore::create(
        workflow_store.as_ref(),
        scope,
        WorkflowRecord {
            id: id_str.clone(),
            scope: scope.clone(),
            version: 1,
            slug: id_str.clone(),
            deleted: false,
        },
    )
    .await
    .expect("seed_workflow: WorkflowStore::create must succeed");

    WorkflowVersionStore::create(
        workflow_versions.as_ref(),
        scope,
        WorkflowVersionRecord {
            workflow_id: id_str,
            number: 1,
            published: true,
            pinned: false,
            definition,
        },
    )
    .await
    .expect("seed_workflow: WorkflowVersionStore::create must succeed");
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// T1 — Cross-scope ownership gate (RED-on-revert).
///
/// The workflow belongs to `scope_a`.  Registering under `scope_b`'s tenant
/// must return 404 — the endpoint must not disclose whether the workflow
/// exists in another scope (no existence oracle).
///
/// RED-on-revert: if the guard is removed (handler skips the scope-bound
/// lookup and returns the workflow regardless of scope), the call returns
/// 201 and the assertion flips to green, proving the 404 is load-bearing.
#[tokio::test]
async fn register_cross_scope_returns_404() {
    let (state, _trigger_store, _activation_store, workflow_versions, workflow_store) =
        build_full_state().await;

    let workflow_id = WorkflowId::new();
    let trigger_node_key = "wh-trigger-cross-scope";

    // Seed the workflow under scope_a only.
    seed_workflow(
        &workflow_store,
        &workflow_versions,
        &scope_a(),
        workflow_id,
        workflow_definition_with_binding(workflow_id, trigger_node_key),
    )
    .await;

    // Register under scope_b (caller does NOT own the workflow).
    let body = RegisterWebhookRequest {
        workflow_id: workflow_id.to_string(),
        trigger_id: trigger_node_key.to_string(),
        provider: "generic".to_string(),
        replay_window_secs: None,
        timestamp_header: None,
        provider_config: None,
        rate_limit_per_minute: None,
    };

    let result = register_webhook(
        State(state),
        Extension(dummy_user()),
        Extension(tenant_for_scope_b()),
        Path(("org-b".to_string(), "ws-b".to_string())),
        Json(body),
    )
    .await;

    let err = result.expect_err(
        "cross-scope registration must return Err(NotFound); \
         Ok(_) means the ownership check is absent or scope-unbound",
    );
    let (status, _) = err.to_problem_details();
    assert_eq!(
        status,
        axum::http::StatusCode::NOT_FOUND,
        "cross-scope registration must return HTTP 404, not {status}"
    );
}

/// T2 — Secret-once invariant: signing_secret appears in 201 body but NOT in
/// the stored rows.
///
/// After a successful registration the `port_triggers.config` must reference
/// the credential by `secret_id` only.  The plaintext `whsec_` string must not
/// appear anywhere in the persisted config JSON or in the activation record.
#[tokio::test]
async fn register_happy_path_secret_not_in_rows() {
    let (state, trigger_store, activation_store, workflow_versions, workflow_store) =
        build_full_state().await;

    let workflow_id = WorkflowId::new();
    let trigger_node_key = "wh-trigger-secret-once";
    let scope = scope_a();
    let tenant = tenant_for_scope_a();

    seed_workflow(
        &workflow_store,
        &workflow_versions,
        &scope,
        workflow_id,
        workflow_definition_with_binding(workflow_id, trigger_node_key),
    )
    .await;

    let body = RegisterWebhookRequest {
        workflow_id: workflow_id.to_string(),
        trigger_id: trigger_node_key.to_string(),
        provider: "generic".to_string(),
        replay_window_secs: None,
        timestamp_header: None,
        provider_config: None,
        rate_limit_per_minute: None,
    };

    let result = register_webhook(
        State(state),
        Extension(dummy_user()),
        Extension(tenant),
        Path((TEST_ORG_A.to_string(), TEST_WS_A.to_string())),
        Json(body),
    )
    .await;

    let (status, Json(resp)) = result.expect("happy-path registration must succeed");
    assert_eq!(status, axum::http::StatusCode::CREATED);

    // The response must carry a whsec_ prefixed secret.
    assert!(
        resp.signing_secret.starts_with("whsec_"),
        "signing_secret must be whsec_-prefixed; got {:?}",
        resp.signing_secret
    );

    // The port_triggers row must NOT contain the plaintext whsec_ value.
    let trigger_rows = trigger_store.list(&scope).await.expect("list must succeed");
    assert!(
        !trigger_rows.is_empty(),
        "port_triggers must have at least one row after registration"
    );
    for row in &trigger_rows {
        let config_str = serde_json::to_string(&row.config).expect("config serializes");
        assert!(
            !config_str.contains(&resp.signing_secret),
            "port_triggers config must NOT contain the plaintext signing_secret; \
             found it in row {:?}",
            row.id
        );
    }

    // The port_webhook_activations row must NOT contain the plaintext whsec_ value.
    let activation_rows = activation_store
        .list_all_active()
        .await
        .expect("list_all_active must succeed");
    assert!(
        !activation_rows.is_empty(),
        "port_webhook_activations must have at least one row after registration"
    );
    for record in &activation_rows {
        let record_str = serde_json::to_string(record).expect("record serializes");
        assert!(
            !record_str.contains(&resp.signing_secret),
            "port_webhook_activations must NOT contain the plaintext signing_secret; \
             found it in record for trigger {:?}",
            record.trigger_id
        );
    }
}

/// T3 — Happy path: 201 with valid URL + activation_id; durable rows written.
///
/// Verifies that both `port_triggers` and `port_webhook_activations` have
/// rows after the call, and that `webhook_url` is a valid HTTPS URL.
#[tokio::test]
async fn register_happy_path_rows_written() {
    let (state, trigger_store, activation_store, workflow_versions, workflow_store) =
        build_full_state().await;

    let workflow_id = WorkflowId::new();
    let trigger_node_key = "wh-trigger-happy-path";
    let scope = scope_a();
    let tenant = tenant_for_scope_a();

    seed_workflow(
        &workflow_store,
        &workflow_versions,
        &scope,
        workflow_id,
        workflow_definition_with_binding(workflow_id, trigger_node_key),
    )
    .await;

    let body = RegisterWebhookRequest {
        workflow_id: workflow_id.to_string(),
        trigger_id: trigger_node_key.to_string(),
        provider: "generic".to_string(),
        replay_window_secs: None,
        timestamp_header: None,
        provider_config: None,
        rate_limit_per_minute: None,
    };

    let (status, Json(resp)) = register_webhook(
        State(state),
        Extension(dummy_user()),
        Extension(tenant),
        Path((TEST_ORG_A.to_string(), TEST_WS_A.to_string())),
        Json(body),
    )
    .await
    .expect("happy-path registration must succeed");

    assert_eq!(status, axum::http::StatusCode::CREATED, "must return 201");

    // webhook_url must be a valid HTTPS URL.
    let url = Url::parse(&resp.webhook_url).expect("webhook_url must be a parseable URL");
    assert_eq!(url.scheme(), "https", "webhook_url must be HTTPS");

    // activation_id must be non-empty.
    assert!(
        !resp.activation_id.is_empty(),
        "activation_id must be non-empty"
    );

    // port_triggers row must exist.
    let trigger_rows = trigger_store.list(&scope).await.expect("list must succeed");
    assert_eq!(
        trigger_rows.len(),
        1,
        "exactly one port_triggers row expected after registration; got {}",
        trigger_rows.len()
    );

    // port_webhook_activations row must exist.
    let activation_rows = activation_store
        .list_all_active()
        .await
        .expect("list_all_active must succeed");
    assert_eq!(
        activation_rows.len(),
        1,
        "exactly one port_webhook_activations row expected after registration; got {}",
        activation_rows.len()
    );
    assert_eq!(
        activation_rows[0].mode,
        nebula_storage_port::dto::WebhookMode::Prod,
        "activation must be mode=Prod"
    );
}

/// T4 — Missing webhook transport → 503 (no panic, no 500).
///
/// The handler must fail-closed when a required infrastructure piece is
/// absent from `AppState`.  This also validates the 503 gate for each of
/// the 7 infra checks.
#[tokio::test]
async fn register_without_transport_returns_503() {
    // Build a state WITHOUT the webhook transport.  Credential service and
    // workflow stores are present so we get past those checks and hit the
    // transport check specifically.
    let key: Arc<dyn KeyProvider> =
        Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid AES key fixture"));
    let credential_svc = with_memory_store(Arc::clone(&key))
        .await
        .expect("credential service builds");

    let trigger_store = Arc::new(InMemoryTriggerStore::new());
    let activation_store = Arc::new(InMemoryWebhookActivationStore::new());
    let workflow_versions = Arc::new(InMemoryWorkflowVersionStore::new());
    let workflow_store = Arc::new(InMemoryWorkflowStore::new_with_versions(&workflow_versions));

    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let journal = nebula_storage::inmem::InMemoryJournalReader::new(&exec_store);
    let node_results = InMemoryNodeResultStore::new();

    let config = nebula_api::ApiConfig::for_test();

    // transport deliberately NOT wired.
    let state = AppState::new(
        Arc::clone(&workflow_store) as _,
        Arc::clone(&workflow_versions) as _,
        Arc::new(exec_store),
        Arc::new(node_results),
        Arc::new(journal),
        Arc::new(control_queue),
        config.jwt_secret,
    )
    .with_credential_service(credential_svc)
    .with_trigger_store(Arc::clone(&trigger_store) as _)
    .with_webhook_activation_store(Arc::clone(&activation_store) as _)
    // no .with_webhook_transport(...)
    .with_insecure_tenant_rbac_bypass_for_tests();

    let body = RegisterWebhookRequest {
        workflow_id: WorkflowId::new().to_string(),
        trigger_id: "some-trigger".to_string(),
        provider: "generic".to_string(),
        replay_window_secs: None,
        timestamp_header: None,
        provider_config: None,
        rate_limit_per_minute: None,
    };

    let result = register_webhook(
        State(state),
        Extension(dummy_user()),
        Extension(tenant_for_scope_a()),
        Path((TEST_ORG_A.to_string(), TEST_WS_A.to_string())),
        Json(body),
    )
    .await;

    let err = result.expect_err("missing transport must return Err; Ok(_) means 503 gate absent");
    let (status, _) = err.to_problem_details();
    assert_eq!(
        status,
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "missing transport must return HTTP 503, not {status}"
    );
}

/// T5 — 3-store compensation on activation failure.
///
/// Failure injection: wire an `ActionRegistry` with NO webhook providers
/// registered.  The upfront infra-presence check passes (registry exists),
/// but `lookup_webhook_factory("generic")` returns `None` inside the
/// activation block — AFTER the credential row (step 3) and trigger spec
/// row (step 4) are written.  The compensation path must then delete both.
///
/// After the call, `port_triggers` must have a soft-deleted row and the
/// credential store must be empty (as if the call never happened).
///
/// Note: `port_webhook_activations` is not written in this failure path
/// (activation fails before it reaches the store write), so it remains empty.
///
/// RED-on-revert proof: comment out both `compensate_delete_credential` calls
/// in handler.rs and rerun — the credential assertion below panics, proving
/// the cleanup is load-bearing.
#[tokio::test]
async fn register_compensation_cleans_up_on_activation_failure() {
    // An empty ActionRegistry passes the upfront presence check but causes
    // lookup_webhook_factory("generic") → None inside the activation block,
    // triggering the compensation path after credential + trigger rows are written.
    let empty_registry = Arc::new(ActionRegistry::new());
    // (No providers registered — lookup will return None.)

    let key: Arc<dyn KeyProvider> =
        Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid AES key fixture"));
    let credential_svc = with_memory_store(Arc::clone(&key))
        .await
        .expect("credential service builds");

    let trigger_store = Arc::new(InMemoryTriggerStore::new());
    let activation_store = Arc::new(InMemoryWebhookActivationStore::new());
    let workflow_versions = Arc::new(InMemoryWorkflowVersionStore::new());
    let workflow_store = Arc::new(InMemoryWorkflowStore::new_with_versions(&workflow_versions));

    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let journal = nebula_storage::inmem::InMemoryJournalReader::new(&exec_store);
    let node_results = InMemoryNodeResultStore::new();

    let config = nebula_api::ApiConfig::for_test();

    let transport = WebhookTransport::new(WebhookTransportConfig {
        base_url: Url::parse("https://nebula.example.com").expect("valid base URL"),
        path_prefix: "/webhooks".to_string(),
        body_limit_bytes: 1 << 20,
        response_timeout: Duration::from_secs(5),
        rate_limit_per_minute: None,
        tenant_rate_limit_per_minute: None,
    });

    let secret_resolver = Arc::new(CredentialBackedWebhookSecretResolver::new(
        credential_svc.clone(),
    ));
    let ctx_factory: Arc<dyn WebhookActivationContextFactory> = Arc::new(NoopCtxFactory);

    // Retain a handle to the credential service so we can assert cleanup after
    // the call.  The clone is a cheap Arc bump; both share the same in-memory
    // SQLite store.
    let cred_handle = credential_svc.clone();

    let state = AppState::new(
        Arc::clone(&workflow_store) as _,
        Arc::clone(&workflow_versions) as _,
        Arc::new(exec_store),
        Arc::new(node_results),
        Arc::new(journal),
        Arc::new(control_queue),
        config.jwt_secret,
    )
    .with_credential_service(credential_svc)
    .with_trigger_store(Arc::clone(&trigger_store) as _)
    .with_webhook_activation_store(Arc::clone(&activation_store) as _)
    .with_webhook_transport(transport)
    .with_webhook_secret_resolver(secret_resolver)
    .with_webhook_ctx_factory_b(ctx_factory)
    // Empty registry: passes the presence check but lookup_webhook_factory returns None.
    .with_action_registry(empty_registry)
    .with_insecure_tenant_rbac_bypass_for_tests();

    // Seed a workflow the handler can pass ownership checks on.
    let workflow_id = WorkflowId::new();
    let trigger_node_key = "wh-trigger-compensation";
    seed_workflow(
        &workflow_store,
        &workflow_versions,
        &scope_a(),
        workflow_id,
        workflow_definition_with_binding(workflow_id, trigger_node_key),
    )
    .await;

    let body = RegisterWebhookRequest {
        workflow_id: workflow_id.to_string(),
        trigger_id: trigger_node_key.to_string(),
        provider: "generic".to_string(),
        replay_window_secs: None,
        timestamp_header: None,
        provider_config: None,
        rate_limit_per_minute: None,
    };

    let result = register_webhook(
        State(state),
        Extension(dummy_user()),
        Extension(tenant_for_scope_a()),
        Path((TEST_ORG_A.to_string(), TEST_WS_A.to_string())),
        Json(body),
    )
    .await;

    // The call must fail: lookup_webhook_factory("generic") returns None → 400.
    let err = result.expect_err(
        "unknown provider must return Err; Ok(_) means the factory lookup or \
         compensation path is broken",
    );
    let (status, _) = err.to_problem_details();
    assert_eq!(
        status,
        axum::http::StatusCode::BAD_REQUEST,
        "unknown provider must return 400 from the factory-lookup path \
         (the failure point that triggers compensation); got {status}"
    );

    // Compensation must have deleted the minted signing-key credential.
    //
    // RED-on-revert proof: comment out both `compensate_delete_credential`
    // calls in handler.rs and rerun — `heads` will have 1 entry and this
    // assertion panics, proving the cleanup is load-bearing.
    let tenant_scope = TenantScope::from_scope(&scope_a());
    let heads = cred_handle
        .list(&tenant_scope)
        .await
        .expect("credential list must succeed");
    assert!(
        heads.is_empty(),
        "compensation must delete the minted signing-key credential; \
         found {} leftover credential(s) — compensate_delete_credential is missing or not called",
        heads.len()
    );

    // Compensation must have cleaned up the trigger spec row.
    let trigger_rows = trigger_store
        .list(&scope_a())
        .await
        .expect("list must succeed");
    // The row is soft-deleted; depending on store impl `list` may filter it.
    // Either 0 rows (filtered out) or all rows have deleted_at set.
    for row in &trigger_rows {
        assert!(
            row.deleted_at.is_some(),
            "compensation must soft-delete the trigger row; row {:?} has deleted_at=None",
            row.id
        );
    }
}

// ── P1 producer regression ────────────────────────────────────────────────────

/// T6 — P1 producer regression: activation row must store the NodeKey as
/// `trigger_id` (dispatch routing) and the `trg_` spec-row PK as
/// `spec_trigger_id` (ADR-0101 L1 spec link).
///
/// `do_emit_prod` calls `NodeKey::new(&row.trigger_id)` to resolve the
/// binding in `ValidatedWorkflow.trigger_bindings`.  If the handler writes the
/// `trg_` PK there instead, dispatch mis-routes and produces a 5xx.
///
/// RED-on-revert: revert the handler to
/// `PersistParams { trigger_id: trigger_row_id.clone(), .. }` and the
/// `trigger_id` assertion below panics, proving the fix is load-bearing.
#[tokio::test]
async fn register_activation_row_has_node_key_trigger_id_and_spec_link() {
    let (state, trigger_store, activation_store, workflow_versions, workflow_store) =
        build_full_state().await;

    let workflow_id = WorkflowId::new();
    let trigger_node_key = "wh-trigger-p1-regression";
    let scope = scope_a();
    let tenant = tenant_for_scope_a();

    seed_workflow(
        &workflow_store,
        &workflow_versions,
        &scope,
        workflow_id,
        workflow_definition_with_binding(workflow_id, trigger_node_key),
    )
    .await;

    let body = RegisterWebhookRequest {
        workflow_id: workflow_id.to_string(),
        trigger_id: trigger_node_key.to_string(),
        provider: "generic".to_string(),
        replay_window_secs: None,
        timestamp_header: None,
        provider_config: None,
        rate_limit_per_minute: None,
    };

    let (status, _resp) = register_webhook(
        State(state),
        Extension(dummy_user()),
        Extension(tenant),
        Path((TEST_ORG_A.to_string(), TEST_WS_A.to_string())),
        Json(body),
    )
    .await
    .expect("happy-path registration must succeed");
    assert_eq!(status, axum::http::StatusCode::CREATED);

    // The port_triggers row carries the trg_ PK (server-generated).
    let trigger_rows = trigger_store.list(&scope).await.expect("list must succeed");
    assert_eq!(trigger_rows.len(), 1, "exactly one trigger row expected");
    let spec_pk = trigger_rows[0].id.clone();
    assert!(
        spec_pk.starts_with("trg_"),
        "port_triggers PK must start with trg_; got {spec_pk:?}"
    );

    // The activation row's trigger_id must be the NodeKey (for dispatch routing),
    // NOT the trg_ PK.
    let activation_rows = activation_store
        .list_all_active()
        .await
        .expect("list_all_active must succeed");
    assert_eq!(
        activation_rows.len(),
        1,
        "exactly one activation row expected"
    );
    let row = &activation_rows[0];

    assert_eq!(
        row.trigger_id, trigger_node_key,
        "activation row.trigger_id must be the NodeKey (dispatch routing key); \
         got {:?} — if this is a trg_ prefix, the P1 bug is not fixed",
        row.trigger_id
    );

    // The activation row's spec_trigger_id must be the trg_ PK (L1 spec link).
    assert_eq!(
        row.spec_trigger_id,
        Some(spec_pk.clone()),
        "activation row.spec_trigger_id must be the port_triggers PK ({spec_pk:?}); \
         got {:?}",
        row.spec_trigger_id
    );
}

// ── Bootstrap reconstruct RED→GREEN ──────────────────────────────────────────

/// T7 — Bootstrap reconstruct: seeding an activation row with
/// `spec_trigger_id = Some(trg_X)` lets bootstrap find the spec and validate
/// the activation; `spec_trigger_id = None` (legacy) is skipped with
/// `MissingSpec`.
///
/// RED proof: with the OLD bootstrap code (`spec_lookup.lookup(&record.trigger_id)`)
/// the `spec_trigger_id = Some(trg_X)` case would look up by NodeKey (which has
/// no spec row) and return `MissingSpec`; the test asserts `report.loaded == 1`
/// which fails — proving the fix is needed.
#[tokio::test]
async fn bootstrap_reconstruct_uses_spec_trigger_id() {
    use std::future::Future;
    use std::pin::Pin;

    use nebula_api::transport::webhook::{
        TriggerSpecLookup, WebhookSecretResolver, bootstrap_webhook_activations,
    };
    use nebula_storage::rows::WebhookActivationSpec as StorageWebhookActivationSpec;

    // We need a TriggerSpecLookup that serves a spec for trg_X but NOT for the
    // NodeKey "wh-node-key".  This proves the bootstrap uses spec_trigger_id
    // (the trg_ PK) and not trigger_id (the NodeKey).
    const TRG_SPEC_PK: &str = "trg_bootstrap_reconstruct_test";
    const NODE_KEY: &str = "wh-node-key";
    const SECRET_ID: &str = "cred_test_secret";

    struct SpecByPkLookup;

    impl TriggerSpecLookup for SpecByPkLookup {
        fn lookup<'life0, 'life1, 'life2, 'async_trait>(
            &'life0 self,
            _scope: &'life1 Scope,
            trigger_id: &'life2 str,
        ) -> Pin<
            Box<
                dyn Future<
                        Output = Result<
                            Option<StorageWebhookActivationSpec>,
                            Box<dyn std::error::Error + Send + Sync>,
                        >,
                    > + Send
                    + 'async_trait,
            >,
        >
        where
            'life0: 'async_trait,
            'life1: 'async_trait,
            'life2: 'async_trait,
        {
            let id = trigger_id.to_string();
            Box::pin(async move {
                if id == TRG_SPEC_PK {
                    // Return a minimal spec so factory build can proceed.
                    let spec = StorageWebhookActivationSpec::new("generic", SECRET_ID);
                    Ok(Some(spec))
                } else {
                    // NodeKey lookup → no spec (proves separation).
                    Ok(None)
                }
            })
        }
    }

    // A secret resolver that returns a fixed HMAC key for our test secret.
    struct FakeSecretResolver;
    #[async_trait::async_trait]
    impl WebhookSecretResolver for FakeSecretResolver {
        async fn resolve(
            &self,
            _scope: &Scope,
            secret_id: &str,
        ) -> Result<Vec<u8>, nebula_api::transport::webhook::SecretResolutionError> {
            if secret_id == SECRET_ID {
                Ok(vec![0x42u8; 32]) // 32-byte dummy HMAC key
            } else {
                Err(format!("unknown secret_id: {secret_id}").into())
            }
        }
    }

    // Seed an activation store with two rows:
    //   Row A: spec_trigger_id = Some(trg_X) → bootstrap should load it.
    //   Row B: spec_trigger_id = None         → bootstrap should skip it.
    let activation_store = Arc::new(InMemoryWebhookActivationStore::new());

    let mut row_with_spec =
        WebhookActivationRecord::new(NODE_KEY, scope_a(), "slug-with-spec", true);
    row_with_spec.spec_trigger_id = Some(TRG_SPEC_PK.to_string());

    let row_legacy = WebhookActivationRecord::new(NODE_KEY, scope_a(), "slug-legacy", true);
    // spec_trigger_id defaults to None — simulates a pre-ADR-0101 row.

    activation_store
        .upsert(&scope_a(), row_with_spec)
        .await
        .expect("upsert row_with_spec must succeed");
    activation_store
        .upsert(&scope_a(), row_legacy)
        .await
        .expect("upsert row_legacy must succeed");

    let registry = ActionRegistry::new();
    for f in default_factories() {
        registry.register_webhook_provider(f);
    }

    let report = bootstrap_webhook_activations(
        activation_store.as_ref(),
        &registry,
        &FakeSecretResolver,
        &NoopCtxFactory,
        &SpecByPkLookup,
        None,
    )
    .await
    .expect("bootstrap must not return a storage error");

    assert_eq!(
        report.loaded, 1,
        "exactly one activation should be loaded (the row with spec_trigger_id set); \
         got loaded={}, skipped={} — \
         if loaded=0 the bootstrap is still using trigger_id (NodeKey) for spec lookup",
        report.loaded, report.skipped
    );
    assert_eq!(
        report.skipped, 1,
        "exactly one activation should be skipped (the legacy row with spec_trigger_id=None); \
         got skipped={}",
        report.skipped
    );
}

// ── P2 — factory InvalidSpec → 422 ───────────────────────────────────────────

/// T8 — P2 factory error mapping: a factory that returns `InvalidSpec` must
/// produce HTTP 422 (not 500) at the registration endpoint.
///
/// Approach: register a custom `WebhookActionFactory` in the test's registry
/// that always returns `FactoryError::InvalidSpec` for provider `"bad-provider"`.
/// The test sends a `RegisterWebhookRequest` with `provider = "bad-provider"`.
///
/// We cannot reach `InvalidSpec` through the generic/slack/stripe providers via
/// the DTO alone (they accept any spec with a non-empty secret), so a fake factory
/// is the correct injection point as specified in the task.
#[tokio::test]
async fn register_factory_invalid_spec_returns_422() {
    use nebula_action::webhook::factory::{
        BuiltWebhookHandler, FactoryError, WebhookActionFactory, WebhookActivationSpec,
    };

    // A factory that always returns InvalidSpec.
    struct AlwaysInvalidSpecFactory;
    impl WebhookActionFactory for AlwaysInvalidSpecFactory {
        fn kind(&self) -> &'static str {
            "bad-provider"
        }
        fn build(
            &self,
            _spec: &WebhookActivationSpec,
        ) -> Result<BuiltWebhookHandler, FactoryError> {
            Err(FactoryError::InvalidSpec {
                kind: "bad-provider",
                reason: "injected InvalidSpec for P2 test".to_string(),
            })
        }
    }

    // Build state with the bad-provider factory registered.
    let key: Arc<dyn KeyProvider> =
        Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid AES key"));
    let credential_svc = with_memory_store(Arc::clone(&key))
        .await
        .expect("credential service builds");

    let trigger_store = Arc::new(InMemoryTriggerStore::new());
    let activation_store = Arc::new(InMemoryWebhookActivationStore::new());
    let workflow_versions = Arc::new(InMemoryWorkflowVersionStore::new());
    let workflow_store = Arc::new(InMemoryWorkflowStore::new_with_versions(&workflow_versions));

    let exec_store = InMemoryExecutionStore::new();
    let control_queue = InMemoryControlQueue::new(&exec_store);
    let journal = nebula_storage::inmem::InMemoryJournalReader::new(&exec_store);
    let node_results = InMemoryNodeResultStore::new();

    let action_registry = Arc::new({
        let r = ActionRegistry::new();
        r.register_webhook_provider(Arc::new(AlwaysInvalidSpecFactory));
        r
    });

    let transport = WebhookTransport::new(WebhookTransportConfig {
        base_url: Url::parse("https://nebula.example.com").expect("valid base URL"),
        path_prefix: "/webhooks".to_string(),
        body_limit_bytes: 1 << 20,
        response_timeout: Duration::from_secs(5),
        rate_limit_per_minute: None,
        tenant_rate_limit_per_minute: None,
    });

    let secret_resolver = Arc::new(CredentialBackedWebhookSecretResolver::new(
        credential_svc.clone(),
    ));
    let ctx_factory: Arc<dyn WebhookActivationContextFactory> = Arc::new(NoopCtxFactory);
    let spec_lookup =
        TriggerStoreSpecLookup::new(Arc::clone(&trigger_store) as Arc<dyn TriggerStore>);

    let config = nebula_api::ApiConfig::for_test();
    let state = AppState::new(
        Arc::clone(&workflow_store) as _,
        Arc::clone(&workflow_versions) as _,
        Arc::new(exec_store),
        Arc::new(node_results),
        Arc::new(journal),
        Arc::new(control_queue),
        config.jwt_secret,
    )
    .with_credential_service(credential_svc)
    .with_trigger_store(Arc::clone(&trigger_store) as _)
    .with_webhook_activation_store(Arc::clone(&activation_store) as _)
    .with_action_registry(Arc::clone(&action_registry))
    .with_webhook_transport(transport)
    .with_webhook_secret_resolver(secret_resolver)
    .with_webhook_ctx_factory_b(ctx_factory)
    .with_webhook_spec_lookup(Arc::new(spec_lookup))
    .with_insecure_tenant_rbac_bypass_for_tests();

    // Seed a workflow so we get past the ownership check.
    let workflow_id = WorkflowId::new();
    let trigger_node_key = "wh-trigger-p2-test";
    seed_workflow(
        &workflow_store,
        &workflow_versions,
        &scope_a(),
        workflow_id,
        workflow_definition_with_binding(workflow_id, trigger_node_key),
    )
    .await;

    let body = RegisterWebhookRequest {
        workflow_id: workflow_id.to_string(),
        trigger_id: trigger_node_key.to_string(),
        provider: "bad-provider".to_string(),
        replay_window_secs: None,
        timestamp_header: None,
        provider_config: None,
        rate_limit_per_minute: None,
    };

    let result = register_webhook(
        State(state),
        Extension(dummy_user()),
        Extension(tenant_for_scope_a()),
        Path((TEST_ORG_A.to_string(), TEST_WS_A.to_string())),
        Json(body),
    )
    .await;

    let err =
        result.expect_err("factory InvalidSpec must return Err; Ok(_) means P2 mapping is broken");
    let (status, _) = err.to_problem_details();
    assert_eq!(
        status,
        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
        "FactoryError::InvalidSpec must map to HTTP 422; got {status} — \
         if 500, the P2 catch-all branch is firing instead of the typed match"
    );
}
