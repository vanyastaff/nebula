//! Webhook registration handler — `POST /orgs/{org}/workspaces/{ws}/webhooks`.
//!
//! This is the first live `mode=Prod` webhook producer.  It owns the full
//! 3-store write sequence and its compensation on partial failure:
//!
//! 1. Scope derivation (server-side; never from the request body).
//! 2. Ownership validation — `trigger_id` must exist in the workflow's
//!    `trigger_bindings`, scoped to the caller's tenant.  Cross-scope or absent
//!    ⇒ 404 (no existence oracle leak).
//! 3. Credential mint — `CredentialService::create` writes the `whsec_` secret.
//! 4. Spec write — `TriggerStore::create` writes the `port_triggers` row.
//! 5. Handler build + `activate_and_persist` — in-memory routing entry +
//!    `port_webhook_activations` row.
//! 6. HTTP 201 with `{ webhook_url, signing_secret (once), activation_id }`.
//!
//! Compensation on step-4 failure: best-effort credential delete.
//! Compensation on step-5 failure: best-effort spec-row soft-delete +
//! best-effort credential delete.
//!
//! # Security
//!
//! - `scope` is NEVER read from the request body.
//! - `mode=Prod` is set server-side only.
//! - `signing_secret` is returned exactly once; never logged, never in metrics.
//! - Handler build is refused when the built policy is `OptionalAcceptUnsigned`.
//! - Ownership is validated under `scope` BEFORE any credential is minted.

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use nebula_action::SignaturePolicy;
use nebula_core::{TenantContext, TriggerId};
use nebula_credential::{CredentialDisplay, TenantScope};
use nebula_storage::rows::WebhookActivationSpec;
use nebula_storage_port::dto::{TriggerRow, WebhookMode};
use nebula_tenancy::ScopedTriggerStore;
use nebula_workflow::definition::WorkflowDefinition;
use serde_json::json;
use tracing::Instrument as _;

use super::dto::{RegisterWebhookRequest, RegisterWebhookResponse};
use nebula_storage_port::store::TriggerStore as _;

use crate::{
    error::{ApiError, ProblemDetails},
    middleware::auth::AuthenticatedUser,
    state::AppState,
    transport::webhook::{PersistParams, activate_and_persist, mint_whsec},
};

/// POST /orgs/{org}/workspaces/{ws}/webhooks — Register a webhook trigger.
///
/// Validates ownership, mints a `whsec_` HMAC secret, persists the trigger
/// spec and activation, and returns the endpoint URL + signing secret exactly
/// once.
#[utoipa::path(
    post,
    path = "/orgs/{org}/workspaces/{ws}/webhooks",
    tag = "workspaces.webhooks",
    security(("bearer" = []), ("api_key" = [])),
    params(
        ("org" = String, Path, description = "Organisation slug or `org_<ULID>`."),
        ("ws" = String, Path, description = "Workspace slug or `ws_<ULID>`."),
    ),
    request_body = RegisterWebhookRequest,
    responses(
        (status = 201, description = "Webhook registered. `signing_secret` returned ONCE — store it immediately.", body = RegisterWebhookResponse),
        (status = 400, description = "Missing or invalid request field.", body = ProblemDetails),
        (status = 401, description = "Authentication required.", body = ProblemDetails),
        (status = 403, description = "Caller does not have access to this workspace.", body = ProblemDetails),
        (status = 404, description = "Workflow or trigger_id not found in this scope.", body = ProblemDetails),
        (status = 422, description = "The factory refused to build a compliant handler (policy not `Required`).", body = ProblemDetails),
        (status = 503, description = "Webhook transport, trigger store, credential service, or secret resolver not configured.", body = ProblemDetails),
    ),
)]
pub async fn register_webhook(
    State(state): State<AppState>,
    Extension(user): Extension<AuthenticatedUser>,
    Extension(tenant): Extension<TenantContext>,
    Path((_org, _ws)): Path<(String, String)>,
    Json(body): Json<RegisterWebhookRequest>,
) -> Result<(StatusCode, Json<RegisterWebhookResponse>), ApiError> {
    // ── Input validation ─────────────────────────────────────────────────────
    if body.workflow_id.is_empty() {
        return Err(ApiError::Validation {
            detail: "workflow_id must not be empty".to_string(),
            errors: vec![],
        });
    }
    if body.trigger_id.is_empty() {
        return Err(ApiError::Validation {
            detail: "trigger_id must not be empty".to_string(),
            errors: vec![],
        });
    }
    if body.provider.is_empty() {
        return Err(ApiError::Validation {
            detail: "provider must not be empty".to_string(),
            errors: vec![],
        });
    }

    // ── Step 1: scope — server-derived, NEVER from request ──────────────────
    let scope = crate::middleware::tenancy::request_scope(&tenant)?;

    // ── Infrastructure availability checks ───────────────────────────────────
    let transport = state.webhook_transport.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("webhook transport not configured".to_string())
    })?;
    let trigger_store = state
        .trigger_store
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable("trigger store not configured".to_string()))?;
    let credential_service = state.credential_service.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("credential service not configured".to_string())
    })?;
    let webhook_activation_store = state.webhook_activation_store.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("webhook activation store not configured".to_string())
    })?;
    let action_registry = state.action_registry.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("action registry not configured".to_string())
    })?;
    let secret_resolver = state.webhook_secret_resolver.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("webhook secret resolver not configured".to_string())
    })?;
    let ctx_factory = state.webhook_ctx_factory_b.as_ref().ok_or_else(|| {
        ApiError::ServiceUnavailable("webhook ctx factory not configured".to_string())
    })?;

    // ── Step 2: ownership — validate trigger_id under scope ─────────────────
    //
    // The workflow must exist in this scope; the trigger_id must appear in its
    // trigger_bindings.  A cross-scope or absent result is 404 — we never
    // disclose whether the workflow exists in another scope.
    let workflow_id_parsed = nebula_core::WorkflowId::parse(&body.workflow_id).map_err(|_| {
        ApiError::NotFound(format!(
            "workflow_id {:?} not found in this scope",
            body.workflow_id
        ))
    })?;

    let definition_value = state
        .workflow_definition_scoped(&scope, workflow_id_parsed)
        .await?
        .ok_or_else(|| {
            ApiError::NotFound(format!(
                "workflow_id {:?} not found in this scope",
                body.workflow_id
            ))
        })?;

    // Parse the definition to extract trigger_bindings.  A parse failure is
    // not a 500 — the stored definition might be an older schema version.  Treat
    // it as "trigger not found" (the binding couldn't be verified, so we refuse).
    //
    // NOTE: We round-trip through `to_string` + `from_str` rather than
    // `from_value` because `serde_json::Value`'s deserializer cannot lend
    // a `&str` borrow to `Key::deserialize` (which calls `<&str>::deserialize`
    // for human-readable formats).  `from_str` drives the streaming JSON
    // parser which CAN serve `&str` borrows.
    let definition_str = serde_json::to_string(&definition_value).map_err(|e| {
        tracing::warn!(
            target: "nebula::api::webhook::register",
            error = %e,
            workflow_id = %body.workflow_id,
            "failed to serialize workflow definition for trigger ownership check"
        );
        ApiError::NotFound(format!(
            "trigger_id {:?} not found in workflow {:?}",
            body.trigger_id, body.workflow_id
        ))
    })?;
    let definition: WorkflowDefinition = serde_json::from_str(&definition_str).map_err(|e| {
        tracing::warn!(
            target: "nebula::api::webhook::register",
            error = %e,
            workflow_id = %body.workflow_id,
            "failed to parse workflow definition for trigger ownership check"
        );
        ApiError::NotFound(format!(
            "trigger_id {:?} not found in workflow {:?}",
            body.trigger_id, body.workflow_id
        ))
    })?;

    let trigger_found = definition
        .trigger_bindings
        .iter()
        .any(|b| b.id.as_str() == body.trigger_id);
    if !trigger_found {
        return Err(ApiError::NotFound(format!(
            "trigger_id {:?} not found in workflow {:?}",
            body.trigger_id, body.workflow_id
        )));
    }

    // ── Step 3: mint credential ──────────────────────────────────────────────
    //
    // Generate a fresh CSPRNG-backed `whsec_` secret and store it as a
    // `signing_key` credential scoped to this tenant.  `whsec_` is the ONLY
    // copy that is ever returned to the caller.  The persistence layer stores
    // the id, not the plaintext bytes.
    let whsec = mint_whsec();
    let tenant_scope = TenantScope::from_scope(&scope);
    let credential_head = credential_service
        .create(
            &tenant_scope,
            "signing_key",
            json!({
                "key": whsec,
                "algorithm": "hmac-sha256"
            }),
            CredentialDisplay {
                display_name: Some(format!("webhook-signing-key-{}", &body.trigger_id)),
                ..CredentialDisplay::default()
            },
        )
        .await
        .map_err(|e| {
            tracing::error!(
                target: "nebula::api::webhook::register",
                error = %e,
                "credential mint failed during webhook registration"
            );
            ApiError::Internal(format!("failed to create signing credential: {e}"))
        })?;
    let secret_id = credential_head.id.clone();

    // ── Step 4: build the spec and write the trigger row ────────────────────
    //
    // The storage row's PK is a server-generated `TriggerId` (prefix `trg_`).
    // The user-supplied `body.trigger_id` is a `NodeKey` (the binding id from
    // the workflow definition's `trigger_bindings`) and goes in `slug`.
    //
    // On failure: best-effort credential delete before returning the error.
    let trigger_uuid = TriggerId::new();
    let trigger_row_id = trigger_uuid.to_string();

    let mut spec = WebhookActivationSpec::new(&body.provider, &secret_id);
    if let Some(secs) = body.replay_window_secs {
        spec = spec.with_replay_window_secs(secs);
    }
    if let Some(header) = body.timestamp_header.as_ref() {
        spec = spec.with_timestamp_header(header.clone());
    }
    if let Some(config) = body.provider_config.clone() {
        spec = spec.with_provider_config(config);
    }
    if let Some(rpm) = body.rate_limit_per_minute {
        spec = spec.with_rate_limit_per_minute(rpm);
    }

    let trigger_config = spec
        .write_into_trigger_config(serde_json::Value::Null)
        .map_err(|e| {
            tracing::error!(
                target: "nebula::api::webhook::register",
                error = %e,
                "failed to serialize webhook activation spec"
            );
            ApiError::Internal(format!("failed to build trigger config: {e}"))
        })?;

    let trigger_row = TriggerRow {
        id: trigger_row_id.clone(),
        workspace_id: scope.workspace_id.clone(),
        workflow_id: body.workflow_id.clone(),
        slug: body.trigger_id.clone(),
        display_name: format!("webhook-{}", &body.trigger_id),
        kind: "webhook".to_string(),
        config: trigger_config,
        state: "active".to_string(),
        run_as: None,
        webhook_path: None,
        created_at: chrono::Utc::now().to_rfc3339(),
        created_by: user.user_id.clone(),
        version: 1,
        deleted_at: None,
    };

    let scoped_trigger = ScopedTriggerStore::new(Arc::clone(trigger_store), scope.clone());
    if let Err(e) = scoped_trigger.create(&scope, trigger_row).await {
        // Compensation: best-effort credential delete.
        compensate_delete_credential(credential_service, &tenant_scope, &secret_id).await;
        tracing::error!(
            target: "nebula::api::webhook::register",
            error = %e,
            trigger_id = %body.trigger_id,
            "trigger spec row creation failed"
        );
        return Err(ApiError::Internal(format!(
            "failed to persist trigger spec: {e}"
        )));
    }

    // ── Step 5: resolve secret bytes → build handler → activate_and_persist ─
    //
    // On failure: best-effort spec-row soft-delete + credential delete.
    let activation_result = async {
        // Resolve the credential to raw HMAC bytes.
        let raw_bytes = secret_resolver
            .resolve(&scope, &secret_id)
            .await
            .map_err(|e| {
                tracing::error!(
                    target: "nebula::api::webhook::register",
                    "secret resolution failed after credential mint"
                );
                ApiError::Internal(format!("secret resolver failed: {e}"))
            })?;

        // Look up the factory and build the handler.
        let factory = action_registry
            .lookup_webhook_factory(&body.provider)
            .ok_or_else(|| ApiError::Validation {
                detail: format!("unknown webhook provider {:?}", body.provider),
                errors: vec![],
            })?;

        use nebula_action::webhook::factory::WebhookActivationSpec as ActionSpec;
        let mut action_spec = ActionSpec::new(body.provider.clone(), raw_bytes);
        if let Some(secs) = body.replay_window_secs {
            action_spec = action_spec.with_replay_window_secs(secs);
        }
        if let Some(header) = body.timestamp_header.as_ref() {
            action_spec = action_spec.with_timestamp_header(header.clone());
        }
        if let Some(config) = body.provider_config.clone() {
            action_spec = action_spec.with_provider_config(config);
        }
        if let Some(rpm) = body.rate_limit_per_minute {
            action_spec = action_spec.with_rate_limit_per_minute(rpm);
        }

        // P2: map FactoryError variants to the correct HTTP status.
        // - InvalidSpec → 422 (semantically-invalid caller input, same tier as
        //   the OptionalAcceptUnsigned gate just below).
        // - UnknownKind → 400 (the provider string is not registered).
        // - SecretResolution + catch-all → 500 (genuine server fault).
        let built = factory.build(&action_spec).map_err(|e| {
            use nebula_action::webhook::factory::FactoryError;
            match e {
                FactoryError::InvalidSpec { kind, ref reason } => {
                    tracing::warn!(
                        target: "nebula::api::webhook::register",
                        provider = %body.provider,
                        kind = %kind,
                        reason = %reason,
                        "factory rejected spec (invalid input)"
                    );
                    ApiError::Unprocessable(format!(
                        "invalid webhook spec for provider {kind:?}: {reason}"
                    ))
                },
                FactoryError::UnknownKind(ref kind) => {
                    tracing::warn!(
                        target: "nebula::api::webhook::register",
                        provider = %body.provider,
                        kind = %kind,
                        "factory build failed — unknown provider kind"
                    );
                    ApiError::Validation {
                        detail: format!("unknown webhook provider {kind:?}"),
                        errors: vec![],
                    }
                },
                // SecretResolution and any future variants are server faults.
                _ => {
                    tracing::error!(
                        target: "nebula::api::webhook::register",
                        error = %e,
                        provider = %body.provider,
                        "factory build failed (server fault)"
                    );
                    ApiError::Internal(format!("factory build failed for {:?}: {e}", body.provider))
                },
            }
        })?;

        // Security gate: refuse `OptionalAcceptUnsigned` — the Prod producer
        // MUST result in a Required policy.  Any factory that produces
        // OptionalAcceptUnsigned is a misconfiguration.
        if matches!(
            built.config.signature_policy(),
            SignaturePolicy::OptionalAcceptUnsigned
        ) {
            return Err(ApiError::Unprocessable(
                "webhook factory produced OptionalAcceptUnsigned policy; \
                 Prod activations require a Required or Custom signature policy"
                    .to_string(),
            ));
        }

        // Build the ctx template from the (yet-to-be-persisted) activation
        // record.  `trigger_id` is the NodeKey (dispatch routing key); the ctx
        // factory builds the runtime context template keyed on it.
        let mut activation_record = nebula_storage_port::dto::WebhookActivationRecord::new(
            &body.trigger_id, // NodeKey — the dispatch routing key
            scope.clone(),
            &body.trigger_id, // slug = NodeKey from the workflow definition binding
            true,
        );
        activation_record.workflow_id = Some(body.workflow_id.clone());
        activation_record.mode = WebhookMode::Prod;

        let ctx_template = ctx_factory.build(&activation_record);

        let handle = activate_and_persist(
            transport,
            webhook_activation_store.as_ref(),
            PersistParams {
                handler: built.handler,
                action_config: built.config,
                ctx_template,
                // P1 FIX: trigger_id must be the NodeKey (dispatch routing key),
                // NOT the trg_ spec-row PK.  `do_emit_prod` calls
                // `NodeKey::new(&row.trigger_id)` to resolve the binding in
                // `ValidatedWorkflow.trigger_bindings`.
                trigger_id: body.trigger_id.clone(),
                // ADR-0101 L1 spec link: the port_triggers PK so bootstrap
                // reconstruct can re-resolve the webhook spec via
                // TriggerSpecLookup::lookup.
                spec_trigger_id: trigger_row_id.clone(),
                scope: scope.clone(),
                workflow_id: Some(body.workflow_id.clone()),
                mode: WebhookMode::Prod,
            },
        )
        .await
        .map_err(|e| {
            tracing::error!(
                target: "nebula::api::webhook::register",
                error = %e,
                trigger_id = %body.trigger_id,
                "activate_and_persist failed"
            );
            ApiError::Internal(format!("activation failed: {e}"))
        })?;

        Ok(handle)
    }
    .instrument(tracing::info_span!(
        "webhook.register.activate",
        trigger_id = %body.trigger_id,
        workflow_id = %body.workflow_id,
        provider = %body.provider,
    ))
    .await;

    let handle = match activation_result {
        Ok(h) => h,
        Err(api_err) => {
            // Compensation: best-effort spec-row soft-delete + credential delete.
            compensate_delete_trigger_spec(&scoped_trigger, &scope, &trigger_row_id).await;
            compensate_delete_credential(credential_service, &tenant_scope, &secret_id).await;
            return Err(api_err);
        },
    };

    // ── Step 6: respond 201 — signing_secret returned exactly once ──────────
    let activation_id = handle.activation_id();

    tracing::info!(
        target: "nebula::api::webhook::register",
        trigger_id = %body.trigger_id,
        workflow_id = %body.workflow_id,
        provider = %body.provider,
        activation_id = %activation_id,
        // webhook_url deliberately not logged (capability URL)
        // signing_secret NEVER logged
        "webhook activation registered"
    );

    Ok((
        StatusCode::CREATED,
        Json(RegisterWebhookResponse {
            webhook_url: handle.endpoint_url.to_string(),
            signing_secret: whsec,
            activation_id,
        }),
    ))
}

// ── Compensation helpers ──────────────────────────────────────────────────────

/// Best-effort credential deletion on registration failure.
///
/// Logs warn on failure; does NOT return an error (the caller's original error
/// is surfaced instead).  The credential id is never logged.
async fn compensate_delete_credential(
    service: &nebula_credential::CredentialService,
    scope: &TenantScope,
    secret_id: &str,
) {
    match service.delete(scope, secret_id).await {
        Ok(()) => {
            tracing::warn!(
                target: "nebula::api::webhook::register",
                "compensation: signing credential deleted after registration failure"
            );
        },
        Err(e) => {
            tracing::warn!(
                target: "nebula::api::webhook::register",
                error = %e,
                "compensation: signing credential delete failed — orphan credential may exist"
            );
        },
    }
}

/// Best-effort trigger-spec row soft-delete on registration failure.
///
/// Logs warn on failure; does NOT return an error.
async fn compensate_delete_trigger_spec(
    scoped_store: &ScopedTriggerStore,
    scope: &nebula_storage_port::Scope,
    trigger_id: &str,
) {
    match scoped_store.soft_delete(scope, trigger_id).await {
        Ok(()) => {
            tracing::warn!(
                target: "nebula::api::webhook::register",
                trigger_id = %trigger_id,
                "compensation: trigger spec row soft-deleted after activation failure"
            );
        },
        Err(e) => {
            tracing::warn!(
                target: "nebula::api::webhook::register",
                error = %e,
                trigger_id = %trigger_id,
                "compensation: trigger spec row soft-delete failed — orphan trigger row may exist"
            );
        },
    }
}
