//! Credential service layer — business logic for credential operations.
//!
//! Each function takes an `AppState` reference plus domain-specific parameters
//! and returns `ApiResult`.
//!
//! ## One persistence path (ADR-0088 D7)
//!
//! Every credential operation routes through the API-owned, object-safe
//! [`CredentialCommandGateway`]. The deployment adapter invokes the
//! credential-owned authority/controller and is the only code that can reach
//! the service. Handlers can submit authenticated intent, but cannot construct
//! an owner selector, authority proof, repository, or raw writer.
//!
//! When no gateway is wired (`credential_gateway: None`) every credential
//! operation returns an honest 503 (§4.5 operational honesty) — there is
//! no raw-store fallback path.
//!
//! ## Credential secrecy
//!
//! - Request `data` crosses the authenticated gateway and is validated once
//!   by the credential controller/service before it is resolved into typed
//!   state and encrypted at rest. The catalog port never sees mutation
//!   payloads; the API never stores or echoes them.
//! - The wire response types ([`CredentialResponse`] /
//!   [`CredentialSummary`]) are projected from the secret-free
//!   [`CredentialGatewayRecord`] — they structurally cannot carry material.
//! - Errors carry the credential id only; no secret reaches an
//!   `ApiError` / `ProblemDetails`. Tracing spans log `cred.id` /
//!   `cred.key` only.
//!
//! ## Workspace isolation
//!
//! Handlers submit an [`AuthenticatedPrincipal`] together with the resolved
//! [`Scope`]. The deployment gateway and credential controller authorize that
//! intent, then derive the persistence owner only through
//! [`Scope::credential_owner_id`] (ADR-0088 D7). Every command is owner-bound;
//! cross-workspace ids collapse to a flat 404 with no existence disclosure.
//!
use nebula_storage_port::Scope;

use crate::{
    domain::credential::dto::{
        ContinueResolveRequest, ContinueResolveResponse, CreateCredentialRequest,
        CredentialCapabilities, CredentialResponse, CredentialSummary, CredentialTestFailureCodeV1,
        CredentialTypeInfo, ListCredentialTypesResponse, ListCredentialsQuery,
        ListCredentialsResponse, RefreshCredentialResponse, ResolveCredentialRequest,
        ResolveCredentialResponse, RevokeCredentialResponse, TestCredentialResponse,
        UpdateCredentialRequest,
    },
    error::{ApiError, ApiResult},
    middleware::auth::AuthenticatedPrincipal,
    ports::credential_command::{
        CredentialCommandGateway, CredentialGatewayAcquisition, CredentialGatewayCommand,
        CredentialGatewayError, CredentialGatewayRecord, CredentialGatewayRefreshRetry,
        CredentialGatewayResult, CredentialGatewayTestFailure, CredentialGatewayTestResult,
    },
    state::AppState,
};

// ── Service access ───────────────────────────────────────────────────────────

const NO_CREDENTIAL_GATEWAY: &str = "credential command gateway not wired: the composition root did not provide an authenticated credential controller";

/// The wired [`CredentialCommandGateway`], or an honest 503 when the
/// composition root provided none.
fn gateway(state: &AppState) -> ApiResult<&dyn CredentialCommandGateway> {
    state
        .credential_gateway
        .as_ref()
        .map(AsRef::as_ref)
        .ok_or_else(|| ApiError::ServiceUnavailable(NO_CREDENTIAL_GATEWAY.to_owned()))
}

/// Execute one authenticated API-owned command for a composite handler.
///
/// Composite operations such as webhook registration use this seam for
/// platform-generated credential material: the credential controller still
/// authorizes and canonically validates the command, while the handler does not
/// pretend that generated data came through the public schema-precheck path.
pub(crate) async fn execute_gateway_command(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    command: CredentialGatewayCommand,
    credential_label: &str,
) -> ApiResult<CredentialGatewayResult> {
    gateway(state)?
        .execute(principal, scope, command)
        .await
        .map_err(|error| map_gateway_err(error, credential_label))
}

// ── Error mapping ────────────────────────────────────────────────────────────

/// Map a [`CredentialGatewayError`] onto a typed [`ApiError`].
///
/// Cross-workspace / unknown ids collapse to a flat `404` with **no
/// existence disclosure**. Capability gaps are client errors (`400`),
/// optimistic-concurrency failures are `409`, expired interactive tokens are
/// `401`, provider/backend unavailability is `503`. Dynamic reason payloads
/// are deliberately discarded at this boundary: credential implementations
/// and storage/provider adapters are not trusted to produce client-safe text.
/// Validated identifiers, capability names, and version numbers remain where
/// they are actionable.
fn map_gateway_err(err: CredentialGatewayError, cred: &str) -> ApiError {
    match err {
        CredentialGatewayError::NotFound => ApiError::NotFound("credential not found".to_owned()),
        CredentialGatewayError::VersionConflict { expected, actual } => ApiError::VersionMismatch(
            format!("credential {cred}: expected version {expected}, found {actual}"),
        ),
        CredentialGatewayError::IdAlreadyExists => {
            ApiError::AlreadyExists("credential id is already reserved".to_owned())
        },
        CredentialGatewayError::NameAlreadyExists => {
            ApiError::AlreadyExists("credential display name is already in use".to_owned())
        },
        CredentialGatewayError::VersionExhausted => ApiError::VersionExhausted(
            "credential can no longer be changed; create a replacement credential".to_owned(),
        ),
        CredentialGatewayError::ValidationFailed { report } => ApiError::Validation {
            detail: "credential properties were rejected".to_owned(),
            errors: report
                .issues()
                .map(|issue| crate::error::ValidationFieldError {
                    detail: issue.message().to_owned(),
                    pointer: issue.path().to_owned(),
                    code: issue.code().to_owned(),
                })
                .collect(),
        },
        CredentialGatewayError::TypeUnknown { key } => ApiError::Validation {
            detail: format!("unknown credential type: {key}"),
            errors: vec![crate::error::ValidationFieldError {
                detail: "no such credential type".to_owned(),
                pointer: "/credential_key".to_owned(),
                code: "unknown_credential_type".to_owned(),
            }],
        },
        CredentialGatewayError::CapabilityUnsupported { capability, key } => ApiError::Validation {
            detail: format!("credential type '{key}' does not support capability '{capability}'"),
            errors: vec![],
        },
        CredentialGatewayError::PendingExpired => ApiError::Unauthorized(
            "pending acquisition token expired or already consumed".to_owned(),
        ),
        CredentialGatewayError::ReauthRequired => ApiError::CredentialReauthRequired,
        CredentialGatewayError::RefreshNotApplied {
            retry: CredentialGatewayRefreshRetry::Never,
        } => ApiError::CredentialRefreshNotAppliedNever,
        CredentialGatewayError::RefreshNotApplied {
            retry: CredentialGatewayRefreshRetry::After { seconds },
        } => ApiError::CredentialRefreshNotAppliedAfter {
            retry_after_secs: seconds,
        },
        CredentialGatewayError::RefreshReconciliationRequired => {
            ApiError::CredentialRefreshReconciliationRequired
        },
        CredentialGatewayError::RevokeReconciliationRequired => {
            ApiError::CredentialRevokeReconciliationRequired
        },
        CredentialGatewayError::Forbidden => {
            ApiError::Forbidden("credential command is not authorized".to_owned())
        },
        CredentialGatewayError::Unavailable => ApiError::ServiceUnavailable(
            "credential command service is temporarily unavailable".to_owned(),
        ),
        CredentialGatewayError::OutcomeUnknown => ApiError::OutcomeUnknown(
            "credential mutation may have committed; reconcile credential state before retrying"
                .to_owned(),
        ),
        CredentialGatewayError::Internal => {
            ApiError::Internal("credential runtime operation failed".to_owned())
        },
    }
}

// ── Response projection ──────────────────────────────────────────────────────

/// Type-level facts (auth pattern + capability flags) for a credential
/// key, sourced from the schema port (the same registry the facade
/// dispatches on, so the two cannot drift). Unknown keys fall back to
/// the honest "custom / no declared capabilities" classification.
fn type_facts(state: &AppState, credential_key: &str) -> (String, CredentialCapabilities) {
    state
        .credential_schema
        .as_ref()
        .and_then(|port| port.get_type(credential_key))
        .map(|d| {
            (
                d.auth_pattern,
                CredentialCapabilities {
                    interactive: d.capabilities.interactive,
                    refreshable: d.capabilities.refreshable,
                    testable: d.capabilities.testable,
                    revocable: d.capabilities.revocable,
                },
            )
        })
        .unwrap_or_else(|| {
            (
                "Custom".to_owned(),
                CredentialCapabilities {
                    interactive: false,
                    refreshable: false,
                    testable: false,
                    revocable: false,
                },
            )
        })
}

/// Project a secret-free gateway record into the full wire response.
fn to_response(state: &AppState, record: CredentialGatewayRecord) -> CredentialResponse {
    let (auth_pattern, capabilities) = type_facts(state, &record.credential_key);
    CredentialResponse {
        id: record.id,
        credential_key: record.credential_key,
        name: record.display_name.unwrap_or_default(),
        description: record.description,
        auth_pattern,
        capabilities,
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
        expires_at: record.expires_at.map(|t| t.to_rfc3339()),
        version: record.version,
        reauth_required: record.reauth_required,
        tags: record.tags.into_iter().collect(),
    }
}

/// Project a secret-free gateway record into the list summary.
fn to_summary(state: &AppState, record: CredentialGatewayRecord) -> CredentialSummary {
    let (auth_pattern, _) = type_facts(state, &record.credential_key);
    CredentialSummary {
        id: record.id,
        credential_key: record.credential_key,
        name: record.display_name.unwrap_or_default(),
        auth_pattern,
        expires_at: record.expires_at.map(|t| t.to_rfc3339()),
        version: record.version,
        reauth_required: record.reauth_required,
    }
}

// ── CRUD ────────────────────────────────────────────────────────────────────

/// Create a new credential in the given workspace.
///
/// Routes through `CredentialService::create`: schema-validate, resolve
/// to typed state, encrypt, persist scoped to the tenant. Returns
/// metadata only (never the secret). Interactive types (e.g. `oauth2`)
/// are not creatable here — they go through the acquisition or OAuth
/// flow — and are refused with a 400.
#[tracing::instrument(skip_all, fields(cred.key = %req.credential_key))]
pub async fn create_credential(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    req: CreateCredentialRequest,
) -> ApiResult<CredentialResponse> {
    let result = gateway(state)?
        .execute(principal, scope, CredentialGatewayCommand::Create(req))
        .await
        .map_err(|e| map_gateway_err(e, "<create>"))?;
    let CredentialGatewayResult::Record(record) = result else {
        return Err(ApiError::Internal(
            "credential gateway returned an invalid create result".to_owned(),
        ));
    };

    tracing::info!(cred.id = %record.id, "credential created");
    Ok(to_response(state, record))
}

/// Retrieve a single credential by ID within a workspace.
///
/// Returns metadata only — the facade head never carries state bytes,
/// so the response structurally cannot echo the secret.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn get_credential(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    cred: &str,
) -> ApiResult<CredentialResponse> {
    let result = gateway(state)?
        .execute(
            principal,
            scope,
            CredentialGatewayCommand::Get {
                credential_id: cred.to_owned(),
            },
        )
        .await
        .map_err(|e| map_gateway_err(e, cred))?;
    let CredentialGatewayResult::Record(record) = result else {
        return Err(ApiError::Internal(
            "credential gateway returned an invalid get result".to_owned(),
        ));
    };
    Ok(to_response(state, record))
}

/// Update an existing credential in the workspace.
///
/// Partial update: only provided fields change. A provided `version`
/// engages compare-and-swap (409 on mismatch). A provided `data`
/// re-runs the typed validate→resolve pipeline for the (unchanged)
/// credential type; a metadata-only update never re-resolves provider
/// material, although the storage layer may re-encrypt the unchanged semantic
/// state into a fresh envelope/current key during the write.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn update_credential(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    cred: &str,
    req: UpdateCredentialRequest,
) -> ApiResult<CredentialResponse> {
    // The credential-owned controller performs the owner-bound read, display
    // merge, canonical type validation, and write under one authorization
    // decision. The API never fetches a head to manufacture write authority.
    let result = gateway(state)?
        .execute(
            principal,
            scope,
            CredentialGatewayCommand::Update {
                credential_id: cred.to_owned(),
                request: req,
            },
        )
        .await
        .map_err(|e| map_gateway_err(e, cred))?;
    let CredentialGatewayResult::Record(record) = result else {
        return Err(ApiError::Internal(
            "credential gateway returned an invalid update result".to_owned(),
        ));
    };

    tracing::info!(cred.id = %record.id, "credential updated");
    Ok(to_response(state, record))
}

/// Tombstone a credential in the workspace.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn delete_credential(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    cred: &str,
) -> ApiResult<()> {
    let result = gateway(state)?
        .execute(
            principal,
            scope,
            CredentialGatewayCommand::Delete {
                credential_id: cred.to_owned(),
            },
        )
        .await
        .map_err(|e| map_gateway_err(e, cred))?;
    if !matches!(result, CredentialGatewayResult::Deleted) {
        return Err(ApiError::Internal(
            "credential gateway returned an invalid delete result".to_owned(),
        ));
    }
    tracing::info!(cred.id = %cred, "credential tombstoned");
    Ok(())
}

/// List credentials in the workspace with optional filters.
///
/// Returns paginated metadata summaries (no secret material). Rows
/// acquired through the OAuth flow share the facade store, so they
/// appear here too — a row awaiting authorization is flagged
/// `reauth_required`.
#[tracing::instrument(skip_all)]
pub async fn list_credentials(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    query: ListCredentialsQuery,
) -> ApiResult<ListCredentialsResponse> {
    let result = gateway(state)?
        .execute(principal, scope, CredentialGatewayCommand::List)
        .await
        .map_err(|e| map_gateway_err(e, "<list>"))?;
    let CredentialGatewayResult::Records(records) = result else {
        return Err(ApiError::Internal(
            "credential gateway returned an invalid list result".to_owned(),
        ));
    };

    let mut summaries: Vec<CredentialSummary> = records
        .into_iter()
        .map(|head| to_summary(state, head))
        .filter(|s| {
            query
                .credential_key
                .as_ref()
                .is_none_or(|k| &s.credential_key == k)
                && query
                    .auth_pattern
                    .as_ref()
                    .is_none_or(|p| &s.auth_pattern == p)
        })
        .collect();

    summaries.sort_by(|a, b| a.id.cmp(&b.id));
    let total = summaries.len();
    let offset = query.offset();
    let limit = query.limit();
    let page: Vec<CredentialSummary> = summaries.into_iter().skip(offset).take(limit).collect();

    Ok(ListCredentialsResponse {
        credentials: page,
        total,
        page: query.page,
        page_size: query.page_size,
    })
}

// ── Lifecycle (test / refresh / revoke) ──────────────────────────────────────

fn map_test_failure_code(code: CredentialGatewayTestFailure) -> CredentialTestFailureCodeV1 {
    match code {
        CredentialGatewayTestFailure::AuthenticationRejected => {
            CredentialTestFailureCodeV1::AuthenticationRejected
        },
        CredentialGatewayTestFailure::PermissionDenied => {
            CredentialTestFailureCodeV1::PermissionDenied
        },
        CredentialGatewayTestFailure::AccountRestricted => {
            CredentialTestFailureCodeV1::AccountRestricted
        },
        CredentialGatewayTestFailure::InvalidConfiguration => {
            CredentialTestFailureCodeV1::InvalidConfiguration
        },
        CredentialGatewayTestFailure::Other => CredentialTestFailureCodeV1::Other,
    }
}

fn test_failure_message(code: CredentialTestFailureCodeV1) -> &'static str {
    match code {
        CredentialTestFailureCodeV1::AuthenticationRejected => "provider rejected the credential",
        CredentialTestFailureCodeV1::PermissionDenied => {
            "credential lacks required provider permissions"
        },
        CredentialTestFailureCodeV1::AccountRestricted => {
            "provider account is disabled, locked, or restricted"
        },
        CredentialTestFailureCodeV1::InvalidConfiguration => "credential configuration is invalid",
        CredentialTestFailureCodeV1::Other => "credential test failed",
    }
}

/// Pure projection from the payload-free gateway result to the v1 wire shape.
fn map_test_result(
    result: CredentialGatewayTestResult,
    tested_at: String,
) -> TestCredentialResponse {
    match result {
        CredentialGatewayTestResult::Success => TestCredentialResponse::Success {
            message: "credential accepted by provider".to_owned(),
            tested_at,
        },
        CredentialGatewayTestResult::Failed(failure) => {
            let code = map_test_failure_code(failure);
            TestCredentialResponse::Failed {
                code,
                message: test_failure_message(code).to_owned(),
                tested_at,
            }
        },
    }
}

/// Test credential connectivity against the external system.
///
/// Dispatches the registered type's `Testable::test` through the facade.
/// A type without the capability is refused with 400 before any decrypt.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn test_credential(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    cred: &str,
) -> ApiResult<TestCredentialResponse> {
    let result = gateway(state)?
        .execute(
            principal,
            scope,
            CredentialGatewayCommand::Test {
                credential_id: cred.to_owned(),
            },
        )
        .await
        .map_err(|e| map_gateway_err(e, cred))?;
    let CredentialGatewayResult::Tested(result) = result else {
        return Err(ApiError::Internal(
            "credential gateway returned an invalid test result".to_owned(),
        ));
    };
    Ok(map_test_result(result, chrono::Utc::now().to_rfc3339()))
}

/// Force a token refresh for the credential.
///
/// Dispatches the registered type's `Refreshable::refresh` through the
/// facade (retry + cross-replica coalescing + CAS re-persist). On a
/// transient provider failure with still-valid stored material the
/// facade returns the cached state instead of failing the call.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn refresh_credential(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    cred: &str,
) -> ApiResult<RefreshCredentialResponse> {
    let result = gateway(state)?
        .execute(
            principal,
            scope,
            CredentialGatewayCommand::Refresh {
                credential_id: cred.to_owned(),
            },
        )
        .await
        .map_err(|e| map_gateway_err(e, cred))?;
    let CredentialGatewayResult::Refreshed { record, refreshed } = result else {
        return Err(ApiError::Internal(
            "credential gateway returned an invalid refresh result".to_owned(),
        ));
    };
    // The facade's fallback-on-interrupt serves the still-valid stored
    // material when the provider failed transiently — honest reporting:
    // that is NOT a refresh, and the old expiry is not a "new" one.
    Ok(if refreshed {
        RefreshCredentialResponse {
            refreshed: true,
            message: "credential refreshed".to_owned(),
            new_expires_at: record.expires_at.map(|t| t.to_rfc3339()),
        }
    } else {
        RefreshCredentialResponse {
            refreshed: false,
            message: "provider temporarily unavailable; refresh did not run — stored \
                      credential material is still valid"
                .to_owned(),
            new_expires_at: None,
        }
    })
}

/// Explicitly revoke the credential at the provider and tombstone the row.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn revoke_credential(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    cred: &str,
) -> ApiResult<RevokeCredentialResponse> {
    let result = gateway(state)?
        .execute(
            principal,
            scope,
            CredentialGatewayCommand::Revoke {
                credential_id: cred.to_owned(),
            },
        )
        .await
        .map_err(|e| map_gateway_err(e, cred))?;
    if !matches!(result, CredentialGatewayResult::Revoked) {
        return Err(ApiError::Internal(
            "credential gateway returned an invalid revoke result".to_owned(),
        ));
    }
    Ok(RevokeCredentialResponse {
        revoked: true,
        message: "credential revoked at the provider and tombstoned".to_owned(),
    })
}

// ── Acquisition (resolve / continue) ─────────────────────────────────────────

/// Map an API-owned gateway acquisition onto the wire response.
fn map_acquisition(acq: CredentialGatewayAcquisition) -> ResolveCredentialResponse {
    match acq {
        CredentialGatewayAcquisition::Complete { credential_id } => {
            ResolveCredentialResponse::Complete { credential_id }
        },
        CredentialGatewayAcquisition::Pending {
            pending_token,
            interaction,
        } => ResolveCredentialResponse::Pending {
            pending_token,
            interaction,
        },
        CredentialGatewayAcquisition::Retry { retry_after_secs } => {
            ResolveCredentialResponse::Retry { retry_after_secs }
        },
    }
}

/// Start credential acquisition / resolution.
///
/// Static types complete synchronously (`complete` + persisted id);
/// interactive types return `pending` with the next UI interaction. The
/// pending token is bound to `(kind, owner, session, token)` — the
/// caller's user id is the session, so only the same user can continue.
#[tracing::instrument(skip_all, fields(cred.key = %req.credential_key))]
pub async fn resolve_credential(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    req: ResolveCredentialRequest,
) -> ApiResult<ResolveCredentialResponse> {
    let result = gateway(state)?
        .execute(principal, scope, CredentialGatewayCommand::Resolve(req))
        .await
        .map_err(|e| map_gateway_err(e, "<resolve>"))?;
    let CredentialGatewayResult::Acquisition(acquisition) = result else {
        return Err(ApiError::Internal(
            "credential gateway returned an invalid resolve result".to_owned(),
        ));
    };
    Ok(map_acquisition(acquisition))
}

/// Continue a multi-step credential acquisition.
///
/// `user_input` is the typed continuation payload (the serialized
/// [`ContinueResolveRequest::user_input`] shape: `"Poll"`, `{"Code":{"code":".."}}`,
/// `{"Callback":{"params":{..}}}`, `{"FormData":{"params":{..}}}`).
#[tracing::instrument(skip_all, fields(cred.key = %req.credential_key))]
pub async fn continue_resolve(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    req: ContinueResolveRequest,
) -> ApiResult<ContinueResolveResponse> {
    let result = gateway(state)?
        .execute(
            principal,
            scope,
            CredentialGatewayCommand::ContinueResolve(req),
        )
        .await
        .map_err(|e| map_gateway_err(e, "<continue>"))?;
    let CredentialGatewayResult::Acquisition(acquisition) = result else {
        return Err(ApiError::Internal(
            "credential gateway returned an invalid continuation result".to_owned(),
        ));
    };
    Ok(map_acquisition(acquisition))
}

// ── Type discovery (schema port) ─────────────────────────────────────────────

/// Map a port [`CredentialTypeDescriptor`] to the wire DTO, applying the
/// api-owned public projection to the schema (the raw `json_schema()`
/// export's `x-nebula-root-rules` / predicate operands are stripped
/// before the unauthenticated wire).
///
/// [`CredentialTypeDescriptor`]: crate::ports::credential_schema::CredentialTypeDescriptor
fn credential_type_info_from_descriptor(
    d: crate::ports::credential_schema::CredentialTypeDescriptor,
) -> CredentialTypeInfo {
    CredentialTypeInfo {
        key: d.key,
        name: d.name,
        description: d.description,
        auth_pattern: d.auth_pattern,
        capabilities: CredentialCapabilities {
            interactive: d.capabilities.interactive,
            refreshable: d.capabilities.refreshable,
            testable: d.capabilities.testable,
            revocable: d.capabilities.revocable,
        },
        schema: crate::domain::credential::schema_projection::project_public_schema(d.schema_json),
        icon: d.icon,
        documentation_url: d.documentation_url,
    }
}

const NO_CRED_SCHEMA_PORT: &str =
    "credential type discovery unavailable: no credential-schema port configured";

/// List registered credential types with their public-projected input
/// schema. No port ⇒ honest 503.
pub async fn list_credential_types(state: &AppState) -> ApiResult<ListCredentialTypesResponse> {
    let port = state
        .credential_schema
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable(NO_CRED_SCHEMA_PORT.to_owned()))?;
    let types = port
        .list_types()
        .into_iter()
        .map(credential_type_info_from_descriptor)
        .collect();
    Ok(ListCredentialTypesResponse { types })
}

/// One credential type by key. No port ⇒ honest 503; unknown key ⇒ 404
/// (credential *types* are public catalog info, so non-existence
/// disclosure is non-sensitive — unlike credential *instances*, which
/// are flat-404 per IDOR rules).
pub async fn get_credential_type(state: &AppState, key: &str) -> ApiResult<CredentialTypeInfo> {
    let port = state
        .credential_schema
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable(NO_CRED_SCHEMA_PORT.to_owned()))?;
    port.get_type(key)
        .map(credential_type_info_from_descriptor)
        .ok_or_else(|| ApiError::NotFound(format!("unknown credential type: {key}")))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::ports::credential_command::{
        CredentialGatewayValidationIssue, CredentialGatewayValidationReport,
    };
    use crate::ports::credential_schema::{CredentialValidationCode, CredentialValidationLocation};
    use nebula_storage::credential::EnvKeyProvider;
    use nebula_storage::inmem::{
        InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader,
        InMemoryNodeResultStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
    };

    /// 32 `0x42` bytes, base64 — a valid AES-256 key fixture (mirrors the
    /// factory's dev key). Not a secret: a fixed test constant.
    const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";
    const PROVIDER_SECRET_CANARY: &str = "provider-echoed-secret-NEVER-WIRE-a7d3";

    #[test]
    fn test_result_mapping_serializes_success_and_every_v1_failure() {
        const TESTED_AT: &str = "2026-07-21T12:34:56Z";

        let success = map_test_result(CredentialGatewayTestResult::Success, TESTED_AT.to_owned());
        assert_eq!(
            serde_json::to_value(&success).expect("serialize success response"),
            serde_json::json!({
                "status": "success",
                "message": "credential accepted by provider",
                "tested_at": TESTED_AT,
            })
        );

        for (gateway_code, wire_code, wire_name, message) in [
            (
                CredentialGatewayTestFailure::AuthenticationRejected,
                CredentialTestFailureCodeV1::AuthenticationRejected,
                "authentication_rejected",
                "provider rejected the credential",
            ),
            (
                CredentialGatewayTestFailure::PermissionDenied,
                CredentialTestFailureCodeV1::PermissionDenied,
                "permission_denied",
                "credential lacks required provider permissions",
            ),
            (
                CredentialGatewayTestFailure::AccountRestricted,
                CredentialTestFailureCodeV1::AccountRestricted,
                "account_restricted",
                "provider account is disabled, locked, or restricted",
            ),
            (
                CredentialGatewayTestFailure::InvalidConfiguration,
                CredentialTestFailureCodeV1::InvalidConfiguration,
                "invalid_configuration",
                "credential configuration is invalid",
            ),
            (
                CredentialGatewayTestFailure::Other,
                CredentialTestFailureCodeV1::Other,
                "other",
                "credential test failed",
            ),
        ] {
            assert_eq!(map_test_failure_code(gateway_code), wire_code);
            let response = map_test_result(
                CredentialGatewayTestResult::Failed(gateway_code),
                TESTED_AT.to_owned(),
            );
            let json = serde_json::to_value(&response).expect("serialize failed response");
            assert_eq!(
                json,
                serde_json::json!({
                    "status": "failed",
                    "code": wire_name,
                    "message": message,
                    "tested_at": TESTED_AT,
                })
            );
            assert!(!json.to_string().contains(PROVIDER_SECRET_CANARY));
            let debug = format!("{response:?}");
            assert!(debug.contains(&format!("{wire_code:?}")));
            assert!(!debug.contains(PROVIDER_SECRET_CANARY));
        }
    }

    fn base_state() -> AppState {
        let exec_store = InMemoryExecutionStore::new();
        let control_queue = InMemoryControlQueue::new(&exec_store);
        let journal = InMemoryJournalReader::new(&exec_store);
        let jwt = crate::config::ApiConfig::for_test().jwt_secret;
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

    /// State with the real registry-backed schema port AND a composed
    /// `CredentialService` — the production shape.
    async fn test_state() -> AppState {
        let port = crate::ports::credential_schema_registry::try_default_registry_port()
            .expect("first-party registry composes");
        let key =
            Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte AES key"));
        let svc = match crate::ports::credential_service_factory::with_memory_store(key).await {
            Ok(svc) => svc,
            // guard-justified: the fixed AES key fixture + ephemeral in-memory
            // store always compose; a failure means the host cannot open one.
            Err(err) => unreachable!("test credential service composes: {err}"),
        };
        base_state()
            .with_credential_schema(port)
            .with_credential_gateway(crate::ports::credential_command::test_gateway_from_service(
                svc,
            ))
    }

    fn test_scope() -> Scope {
        Scope::new("w", "o")
    }

    fn test_principal() -> AuthenticatedPrincipal {
        AuthenticatedPrincipal::for_test_user("usr_01ARZ3NDEKTSV4RRFFQ69G5FAV")
    }

    /// §4.5 operational honesty: with no `CredentialService` wired, every
    /// credential operation refuses with a typed 503 — never a faked
    /// success, and no raw-store fallback path exists.
    #[tokio::test]
    async fn all_credential_fns_are_503_without_service() {
        let s = base_state();
        let scope = test_scope();
        let principal = test_principal();
        assert!(matches!(
            get_credential(&s, &principal, &scope, "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            delete_credential(&s, &principal, &scope, "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            list_credentials(
                &s,
                &principal,
                &scope,
                ListCredentialsQuery {
                    page: 1,
                    page_size: 20,
                    credential_key: None,
                    auth_pattern: None,
                }
            )
            .await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            test_credential(&s, &principal, &scope, "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            refresh_credential(&s, &principal, &scope, "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            revoke_credential(&s, &principal, &scope, "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        // create/resolve hit the schema-port gate first (also absent here)
        // — still a 503, never a persist.
        assert!(matches!(
            create_credential(
                &s,
                &principal,
                &scope,
                CreateCredentialRequest {
                    credential_key: "api_key".into(),
                    name: "n".into(),
                    description: None,
                    data: serde_json::json!({ "api_key": "k" }),
                    tags: None,
                }
            )
            .await,
            Err(ApiError::ServiceUnavailable(_))
        ));
    }

    /// CRUD through the facade: create → get → list → update (rename via
    /// CAS) → delete; the response projection never carries `data` and
    /// the secret never appears in any returned struct.
    #[tokio::test]
    async fn crud_round_trips_without_secret_in_projection() {
        let s = test_state().await;
        let scope = test_scope();
        let principal = test_principal();
        let secret = "sk-unit-crud-NEVER-LEAK-7a7a";
        let created = create_credential(
            &s,
            &principal,
            &scope,
            CreateCredentialRequest {
                credential_key: "api_key".into(),
                name: "Unit Key".into(),
                description: Some("d".into()),
                data: serde_json::json!({ "api_key": secret }),
                tags: None,
            },
        )
        .await
        .expect("create");
        assert!(created.id.starts_with("cred_"));
        assert_eq!(created.version, 1);
        assert_eq!(created.name, "Unit Key");
        assert_eq!(created.auth_pattern, "SecretToken");
        assert!(!created.reauth_required);
        let dbg = format!("{created:?}");
        assert!(
            !dbg.contains(secret),
            "CredentialResponse Debug must not carry the secret: {dbg}"
        );

        let got = get_credential(&s, &principal, &scope, &created.id)
            .await
            .expect("get");
        assert_eq!(got.id, created.id);
        assert!(!format!("{got:?}").contains(secret));

        let listed = list_credentials(
            &s,
            &principal,
            &scope,
            ListCredentialsQuery {
                page: 1,
                page_size: 20,
                credential_key: None,
                auth_pattern: None,
            },
        )
        .await
        .expect("list");
        assert_eq!(listed.total, 1);
        assert!(!format!("{listed:?}").contains(secret));

        // Metadata-only rename via CAS on the returned version; the
        // secret state is untouched and the description survives.
        let renamed = update_credential(
            &s,
            &principal,
            &scope,
            &created.id,
            UpdateCredentialRequest {
                name: Some("Renamed Key".into()),
                description: None,
                data: None,
                tags: None,
                version: Some(created.version),
            },
        )
        .await
        .expect("rename");
        assert_eq!(renamed.name, "Renamed Key");
        assert_eq!(renamed.description.as_deref(), Some("d"));
        assert!(renamed.version > created.version);

        delete_credential(&s, &principal, &scope, &created.id)
            .await
            .expect("delete");
        assert!(matches!(
            get_credential(&s, &principal, &scope, &created.id).await,
            Err(ApiError::NotFound(_))
        ));
    }

    /// Lifecycle ops on a static type are a client error (400), sourced
    /// from the facade's capability gate — not a 503 and not a fake
    /// success.
    #[tokio::test]
    async fn lifecycle_on_static_type_is_validation_error() {
        let s = test_state().await;
        let scope = test_scope();
        let principal = test_principal();
        let created = create_credential(
            &s,
            &principal,
            &scope,
            CreateCredentialRequest {
                credential_key: "api_key".into(),
                name: "k".into(),
                description: None,
                data: serde_json::json!({ "api_key": "v" }),
                tags: None,
            },
        )
        .await
        .expect("create");

        assert!(matches!(
            test_credential(&s, &principal, &scope, &created.id).await,
            Err(ApiError::Validation { .. })
        ));
        assert!(matches!(
            refresh_credential(&s, &principal, &scope, &created.id).await,
            Err(ApiError::Validation { .. })
        ));
        assert!(matches!(
            revoke_credential(&s, &principal, &scope, &created.id).await,
            Err(ApiError::Validation { .. })
        ));
    }

    /// Generic resolve completes synchronously for a static type and the
    /// persisted credential is visible to the CRUD plane (one store).
    #[tokio::test]
    async fn resolve_complete_persists_and_is_visible_to_crud() {
        let s = test_state().await;
        let scope = test_scope();
        let principal = test_principal();
        let res = resolve_credential(
            &s,
            &principal,
            &scope,
            ResolveCredentialRequest {
                credential_key: "api_key".into(),
                data: serde_json::json!({ "api_key": "k-resolved" }),
            },
        )
        .await
        .expect("resolve");
        let ResolveCredentialResponse::Complete { credential_id } = res else {
            panic!("expected Complete for a static type, got {res:?}");
        };
        let got = get_credential(&s, &principal, &scope, &credential_id)
            .await
            .expect("resolved credential is gettable");
        assert_eq!(got.credential_key, "api_key");
    }

    /// Cross-workspace ids collapse to a flat 404 (no existence
    /// disclosure) end-to-end through the transport mapping.
    #[tokio::test]
    async fn cross_workspace_get_is_flat_404() {
        let s = test_state().await;
        let scope_a = Scope::new("ws-a", "org");
        let scope_b = Scope::new("ws-b", "org");
        let principal = test_principal();
        let created = create_credential(
            &s,
            &principal,
            &scope_a,
            CreateCredentialRequest {
                credential_key: "api_key".into(),
                name: "a".into(),
                description: None,
                data: serde_json::json!({ "api_key": "k" }),
                tags: None,
            },
        )
        .await
        .expect("create");

        let err = get_credential(&s, &principal, &scope_b, &created.id)
            .await
            .expect_err("cross-workspace get denied");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    /// The gateway-error mapping is total and secret-safe for the
    /// client-relevant arms.
    #[test]
    fn gateway_error_mapping_statuses() {
        assert!(matches!(
            map_gateway_err(CredentialGatewayError::NotFound, "cred_x"),
            ApiError::NotFound(_)
        ));
        assert!(matches!(
            map_gateway_err(
                CredentialGatewayError::VersionConflict {
                    expected: 1,
                    actual: 2,
                },
                "cred_x"
            ),
            ApiError::VersionMismatch(_)
        ));
        assert!(matches!(
            map_gateway_err(CredentialGatewayError::IdAlreadyExists, "cred_x"),
            ApiError::AlreadyExists(_)
        ));
        assert!(matches!(
            map_gateway_err(CredentialGatewayError::NameAlreadyExists, "cred_x"),
            ApiError::AlreadyExists(_)
        ));
        assert!(matches!(
            map_gateway_err(CredentialGatewayError::VersionExhausted, "cred_x"),
            ApiError::VersionExhausted(_)
        ));
        assert!(matches!(
            map_gateway_err(
                CredentialGatewayError::ValidationFailed {
                    report: CredentialGatewayValidationReport::single(
                        CredentialValidationLocation::Data,
                        CredentialValidationCode::Required,
                    ),
                },
                "cred_x"
            ),
            ApiError::Validation { .. }
        ));
        assert!(matches!(
            map_gateway_err(
                CredentialGatewayError::TypeUnknown { key: "nope".into() },
                "cred_x"
            ),
            ApiError::Validation { .. }
        ));
        assert!(matches!(
            map_gateway_err(
                CredentialGatewayError::CapabilityUnsupported {
                    capability: "refresh".into(),
                    key: "api_key".into(),
                },
                "cred_x"
            ),
            ApiError::Validation { .. }
        ));
        assert!(matches!(
            map_gateway_err(CredentialGatewayError::PendingExpired, "cred_x"),
            ApiError::Unauthorized(_)
        ));
        assert!(matches!(
            map_gateway_err(CredentialGatewayError::ReauthRequired, "cred_x"),
            ApiError::CredentialReauthRequired
        ));
        assert!(matches!(
            map_gateway_err(
                CredentialGatewayError::RefreshNotApplied {
                    retry: CredentialGatewayRefreshRetry::Never,
                },
                "cred_x"
            ),
            ApiError::CredentialRefreshNotAppliedNever
        ));
        assert!(matches!(
            map_gateway_err(
                CredentialGatewayError::RefreshNotApplied {
                    retry: CredentialGatewayRefreshRetry::After {
                        seconds: std::num::NonZeroU64::new(17)
                            .expect("test delay is non-zero"),
                    },
                },
                "cred_x"
            ),
            ApiError::CredentialRefreshNotAppliedAfter { retry_after_secs }
                if retry_after_secs.get() == 17
        ));
        assert!(matches!(
            map_gateway_err(CredentialGatewayError::Unavailable, "cred_x"),
            ApiError::ServiceUnavailable(_)
        ));
        assert!(matches!(
            map_gateway_err(CredentialGatewayError::OutcomeUnknown, "cred_x"),
            ApiError::OutcomeUnknown(_)
        ));
        assert!(matches!(
            map_gateway_err(
                CredentialGatewayError::RefreshReconciliationRequired,
                "cred_x"
            ),
            ApiError::CredentialRefreshReconciliationRequired
        ));
        assert!(matches!(
            map_gateway_err(
                CredentialGatewayError::RevokeReconciliationRequired,
                "cred_x"
            ),
            ApiError::CredentialRevokeReconciliationRequired
        ));
        assert!(matches!(
            map_gateway_err(CredentialGatewayError::Internal, "cred_x"),
            ApiError::Internal(_)
        ));
    }

    #[test]
    fn gateway_error_contract_cannot_carry_dynamic_reason_payloads() {
        const ERROR_SECRET_CANARY: &str = "provider-error-secret-NEVER-WIRE-3b9e";

        for gateway_error in [
            CredentialGatewayError::ValidationFailed {
                report: CredentialGatewayValidationReport::new(
                    CredentialGatewayValidationIssue::new(
                        CredentialValidationLocation::Data,
                        CredentialValidationCode::Required,
                    ),
                    Vec::new(),
                ),
            },
            CredentialGatewayError::RefreshNotApplied {
                retry: CredentialGatewayRefreshRetry::Never,
            },
            CredentialGatewayError::RefreshNotApplied {
                retry: CredentialGatewayRefreshRetry::After {
                    seconds: std::num::NonZeroU64::new(17).expect("test delay is non-zero"),
                },
            },
            CredentialGatewayError::Unavailable,
            CredentialGatewayError::OutcomeUnknown,
            CredentialGatewayError::RefreshReconciliationRequired,
            CredentialGatewayError::RevokeReconciliationRequired,
            CredentialGatewayError::Internal,
        ] {
            let api_error = map_gateway_err(gateway_error, "cred_safe");
            let debug = format!("{api_error:?}");
            assert!(
                !debug.contains(ERROR_SECRET_CANARY),
                "mapped API error must be safe for structured tracing: {debug}"
            );

            let (_status, problem) = api_error.to_problem_details();
            let wire = serde_json::to_string(&problem).expect("serialize problem details");
            assert!(
                !wire.contains(ERROR_SECRET_CANARY),
                "RFC 9457 response must discard dynamic service reasons: {wire}"
            );
        }
    }
}
