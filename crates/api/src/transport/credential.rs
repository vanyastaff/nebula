//! Credential service layer — business logic for credential operations.
//!
//! Each function takes an `AppState` reference plus domain-specific parameters
//! and returns `ApiResult`.
//!
//! ## One persistence path (ADR-0088 D7)
//!
//! Every credential operation routes through the **`CredentialService`
//! facade** (`AppState::credential_service`): CRUD (`create` / `get` /
//! `update` / `delete` / `list`), lifecycle (`test` / `refresh` /
//! `revoke`), and acquisition (`resolve` / `continue_resolve`). The facade
//! owns the layered store (`Audit(Cache(Encryption(raw)))`), the typed
//! validate→resolve pipeline, and the per-operation tenant check, so the
//! api layer never touches a raw store or re-implements validation.
//!
//! The OAuth2 two-phase flow (`domain::credential::oauth`) persists
//! through `scoped_store` — a `CredentialScopeLayer` over the **same**
//! facade store handle ([`CredentialService::credential_store_handle`]) —
//! so an OAuth-acquired credential is visible to `get`/`list` and there
//! is no second store to drift from.
//!
//! When no service is wired (`credential_service: None`) every credential
//! operation returns an honest 503 (§4.5 operational honesty) — there is
//! no raw-store fallback path.
//!
//! ## Credential secrecy
//!
//! - Request `data` is validated against the credential type's schema
//!   (api-side [`CredentialSchemaPort`] pre-check for structured field
//!   errors, then the facade's canonical pipeline) and resolved into
//!   typed state that the facade encrypts at rest. The api never stores
//!   or echoes the raw payload.
//! - The wire response types ([`CredentialResponse`] /
//!   [`CredentialSummary`]) are projected from the secret-free
//!   [`CredentialHead`] — they structurally cannot carry material.
//! - Errors carry the credential id only; no secret reaches an
//!   `ApiError` / `ProblemDetails`. Tracing spans log `cred.id` /
//!   `cred.key` only.
//!
//! ## Workspace isolation
//!
//! Handlers derive a [`TenantScope`] from the resolved request scope via
//! the single canonical derivation ([`TenantScope::from_scope`] →
//! `Scope::credential_owner_id`, ADR-0088 D7). The facade enforces the
//! owner check on every operation; cross-workspace ids collapse to a
//! flat 404 with no existence disclosure.
//!
//! [`CredentialSchemaPort`]: crate::ports::credential_schema::CredentialSchemaPort

use std::collections::HashMap;
use std::sync::Arc;

use nebula_credential::resolve::{InteractionRequest, UserInput};
use nebula_credential::{CredentialDisplay, ErasedCredentialStore, ScopeResolver};
use nebula_credential_runtime::{
    Acquisition, CredentialHead, CredentialService, CredentialServiceError, TenantScope,
};
use nebula_storage_port::Scope;
use nebula_tenancy::CredentialScopeLayer;

use crate::{
    domain::credential::dto::{
        AcquisitionInteraction, ContinueResolveRequest, ContinueResolveResponse,
        CreateCredentialRequest, CredentialCapabilities, CredentialResponse, CredentialSummary,
        CredentialTypeInfo, FormPostField, ListCredentialTypesResponse, ListCredentialsQuery,
        ListCredentialsResponse, RefreshCredentialResponse, ResolveCredentialRequest,
        ResolveCredentialResponse, RevokeCredentialResponse, TestCredentialResponse,
        UpdateCredentialRequest,
    },
    error::{ApiError, ApiResult},
    state::AppState,
};

// ── Service access ───────────────────────────────────────────────────────────

const NO_CREDENTIAL_SERVICE: &str = "credential service not wired: the composition root did not provide a CredentialService \
     (set NEBULA_CRED_MASTER_KEY and compose via try_default_credential_service)";

/// The wired [`CredentialService`], or an honest 503 when the composition
/// root provided none (§4.5 operational honesty — no raw-store fallback).
fn service(state: &AppState) -> ApiResult<&Arc<CredentialService>> {
    state
        .credential_service
        .as_ref()
        .ok_or_else(|| ApiError::ServiceUnavailable(NO_CREDENTIAL_SERVICE.to_owned()))
}

/// Canonical owner key for a resolved request scope (ADR-0088 D7) —
/// shared with the OAuth pending-state binding so both planes key the
/// same tenant identically.
pub(crate) fn owner_id_from_scope(scope: &Scope) -> String {
    scope.credential_owner_id()
}

#[derive(Debug)]
struct RequestCredentialOwner(String);

impl ScopeResolver for RequestCredentialOwner {
    fn current_owner(&self) -> Option<&str> {
        Some(&self.0)
    }
}

/// Owner-scoped raw access to the **facade's** layered store for the
/// OAuth two-phase write path (`domain::credential::oauth`).
///
/// Wraps [`CredentialService::credential_store_handle`] — the same
/// `Audit(Cache(Encryption(raw)))` stack the facade CRUD uses — in a
/// `CredentialScopeLayer` that stamps/checks `metadata["owner_id"]` on
/// every call. One store, two access styles: typed facade operations and
/// the OAuth raw-state writes; both planes stamp the same canonical
/// owner key, so OAuth-acquired rows are visible to `get`/`list`.
pub(crate) fn scoped_store(
    state: &AppState,
    owner_id: &str,
) -> ApiResult<CredentialScopeLayer<ErasedCredentialStore>> {
    let svc = service(state)?;
    Ok(CredentialScopeLayer::new(
        ErasedCredentialStore::new(svc.credential_store_handle()),
        Arc::new(RequestCredentialOwner(owner_id.to_owned())),
    ))
}

// ── Error mapping ────────────────────────────────────────────────────────────

/// Map a [`CredentialServiceError`] onto a typed [`ApiError`].
///
/// Cross-workspace / unknown ids collapse to a flat `404` with **no
/// existence disclosure**. Capability gaps are client errors (`400`),
/// optimistic-concurrency failures are `409`, expired interactive tokens
/// are `401`, provider/backend unavailability is `503`. The mapped
/// strings never carry secret material — the facade's error reasons are
/// value-free by contract (schema code/path only).
fn map_service_err(err: CredentialServiceError, cred: &str) -> ApiError {
    match err {
        CredentialServiceError::NotFound { .. } => {
            ApiError::NotFound("credential not found".to_owned())
        },
        CredentialServiceError::VersionConflict {
            expected, actual, ..
        } => ApiError::VersionMismatch(format!(
            "credential {cred}: expected version {expected}, found {actual}"
        )),
        CredentialServiceError::ValidationFailed { reason } => ApiError::Validation {
            detail: format!("credential properties rejected: {reason}"),
            errors: vec![],
        },
        CredentialServiceError::TypeUnknown { key } => ApiError::Validation {
            detail: format!("unknown credential type: {key}"),
            errors: vec![],
        },
        CredentialServiceError::CapabilityUnsupported { capability, key } => ApiError::Validation {
            detail: format!("credential type '{key}' does not support capability '{capability}'"),
            errors: vec![],
        },
        CredentialServiceError::PendingExpired => ApiError::Unauthorized(
            "pending acquisition token expired or already consumed".to_owned(),
        ),
        CredentialServiceError::TransientProvider(reason) => ApiError::ServiceUnavailable(format!(
            "credential provider temporarily unavailable: {reason}"
        )),
        CredentialServiceError::Provider(reason) => {
            ApiError::ServiceUnavailable(format!("credential provider error: {reason}"))
        },
        CredentialServiceError::ExternalSourceNotWired { provider } => {
            ApiError::ServiceUnavailable(format!(
                "external credential source '{provider}' is configured but not wired"
            ))
        },
        CredentialServiceError::Store(reason) => {
            ApiError::Internal(format!("credential store error: {reason}"))
        },
        // SessionRequired / ScopeViolation / Cancelled / CapabilityWithoutOps
        // are composition or defence-in-depth faults the api wiring prevents
        // (a session is always attached on the acquisition paths); surfacing
        // one is an internal bug, never a client error. `#[non_exhaustive]`
        // future variants land here too — fail closed, no secret echo.
        other => ApiError::Internal(format!("credential runtime error: {other}")),
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

/// Project a secret-free [`CredentialHead`] into the full wire response.
fn to_response(state: &AppState, head: CredentialHead) -> CredentialResponse {
    let (auth_pattern, capabilities) = type_facts(state, &head.credential_key);
    CredentialResponse {
        id: head.id,
        credential_key: head.credential_key,
        name: head.display.display_name.unwrap_or_default(),
        description: head.display.description,
        auth_pattern,
        capabilities,
        created_at: head.created_at.to_rfc3339(),
        updated_at: head.updated_at.to_rfc3339(),
        expires_at: head.expires_at.map(|t| t.to_rfc3339()),
        version: head.version,
        reauth_required: head.reauth_required,
        tags: head.display.tags.into_iter().collect(),
    }
}

/// Project a secret-free [`CredentialHead`] into the list summary.
fn to_summary(state: &AppState, head: CredentialHead) -> CredentialSummary {
    let (auth_pattern, _) = type_facts(state, &head.credential_key);
    CredentialSummary {
        id: head.id,
        credential_key: head.credential_key,
        name: head.display.display_name.unwrap_or_default(),
        auth_pattern,
        expires_at: head.expires_at.map(|t| t.to_rfc3339()),
        version: head.version,
        reauth_required: head.reauth_required,
    }
}

/// Build the per-instance display metadata from request fields.
fn display_from_parts(
    name: Option<String>,
    description: Option<String>,
    tags: Option<HashMap<String, String>>,
) -> CredentialDisplay {
    CredentialDisplay {
        display_name: name,
        description,
        tags: tags.unwrap_or_default().into_iter().collect(),
    }
}

// ── Request `data` pre-validation (api-side structured 400s) ────────────────

/// Map a secret-safe [`CredentialFieldError`] list to the api-wide
/// validation status (400 — `ApiError::Validation`, consistent with every
/// other request-validation failure).
///
/// `CredentialFieldError` carries only an RFC-6901 path, a validator code,
/// and a static message — never the submitted value. The mapping
/// introduces no value either.
///
/// [`CredentialFieldError`]: crate::ports::credential_schema::CredentialFieldError
fn credential_validation_error(
    errs: Vec<crate::ports::credential_schema::CredentialFieldError>,
) -> ApiError {
    let errors = errs
        .into_iter()
        .map(|e| crate::error::ValidationFieldError {
            code: e.code,
            detail: e.message,
            pointer: e.path,
        })
        .collect();
    ApiError::Validation {
        detail: "credential data failed schema validation".to_owned(),
        errors,
    }
}

/// Validate credential `data` against the credential type's resolved
/// schema **before** it reaches the facade. The facade re-validates
/// through its canonical pipeline (authoritative); this api-side
/// pre-check exists to return structured field errors (RFC-6901
/// pointers) instead of the facade's flattened reason string. When no
/// port is configured the request is rejected with 503 — credential
/// `data` is **never** forwarded unvalidated (fail-closed).
fn validate_credential_data(
    state: &AppState,
    credential_key: &str,
    data: &serde_json::Value,
) -> ApiResult<()> {
    match state.credential_schema.as_ref() {
        Some(port) => port
            .validate_data(credential_key, data)
            .map_err(credential_validation_error),
        None => Err(ApiError::ServiceUnavailable(
            "credential data validation unavailable: no credential-schema port configured"
                .to_owned(),
        )),
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
    scope: &Scope,
    req: CreateCredentialRequest,
) -> ApiResult<CredentialResponse> {
    // api-side pre-check for structured field errors; no port ⇒ 503
    // (never forward unvalidated). The facade re-validates afterwards.
    validate_credential_data(state, &req.credential_key, &req.data)?;

    let svc = service(state)?;
    let tenant = TenantScope::from_scope(scope);
    let display = display_from_parts(Some(req.name), req.description, req.tags);
    let head = svc
        .create(&tenant, &req.credential_key, req.data, display)
        .await
        .map_err(|e| map_service_err(e, "<create>"))?;

    tracing::info!(cred.id = %head.id, "credential created");
    Ok(to_response(state, head))
}

/// Retrieve a single credential by ID within a workspace.
///
/// Returns metadata only — the facade head never carries state bytes,
/// so the response structurally cannot echo the secret.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn get_credential(
    state: &AppState,
    scope: &Scope,
    cred: &str,
) -> ApiResult<CredentialResponse> {
    let svc = service(state)?;
    let tenant = TenantScope::from_scope(scope);
    let head = svc
        .get(&tenant, cred)
        .await
        .map_err(|e| map_service_err(e, cred))?;
    Ok(to_response(state, head))
}

/// Update an existing credential in the workspace.
///
/// Partial update: only provided fields change. A provided `version`
/// engages compare-and-swap (409 on mismatch). A provided `data`
/// re-runs the typed validate→resolve pipeline for the (unchanged)
/// credential type; a metadata-only update never re-resolves or
/// re-encrypts state.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn update_credential(
    state: &AppState,
    scope: &Scope,
    cred: &str,
    req: UpdateCredentialRequest,
) -> ApiResult<CredentialResponse> {
    let svc = service(state)?;
    let tenant = TenantScope::from_scope(scope);

    // Merge semantics: only provided display fields change. Read the
    // current head (owner-checked) and overlay. The read-modify-write
    // window on display is closed by the CAS version when supplied.
    let existing = svc
        .get(&tenant, cred)
        .await
        .map_err(|e| map_service_err(e, cred))?;

    if let Some(ref data) = req.data {
        validate_credential_data(state, &existing.credential_key, data)?;
    }

    let mut display = existing.display;
    if let Some(name) = req.name {
        display.display_name = Some(name);
    }
    if req.description.is_some() {
        display.description = req.description;
    }
    if let Some(tags) = req.tags {
        display.tags = tags.into_iter().collect();
    }

    let head = svc
        .update(&tenant, cred, req.data, req.version, display)
        .await
        .map_err(|e| map_service_err(e, cred))?;

    tracing::info!(cred.id = %head.id, "credential updated");
    Ok(to_response(state, head))
}

/// Delete a credential from the workspace.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn delete_credential(state: &AppState, scope: &Scope, cred: &str) -> ApiResult<()> {
    let svc = service(state)?;
    let tenant = TenantScope::from_scope(scope);
    svc.delete(&tenant, cred)
        .await
        .map_err(|e| map_service_err(e, cred))?;
    tracing::info!(cred.id = %cred, "credential deleted");
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
    scope: &Scope,
    query: ListCredentialsQuery,
) -> ApiResult<ListCredentialsResponse> {
    let svc = service(state)?;
    let tenant = TenantScope::from_scope(scope);
    let heads = svc
        .list(&tenant)
        .await
        .map_err(|e| map_service_err(e, "<list>"))?;

    let mut summaries: Vec<CredentialSummary> = heads
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

/// Test credential connectivity against the external system.
///
/// Dispatches the registered type's `Testable::test` through the facade.
/// A type without the capability is refused with 400 before any decrypt.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn test_credential(
    state: &AppState,
    scope: &Scope,
    cred: &str,
) -> ApiResult<TestCredentialResponse> {
    let svc = service(state)?;
    let tenant = TenantScope::from_scope(scope);
    let report = svc
        .test(&tenant, cred)
        .await
        .map_err(|e| map_service_err(e, cred))?;
    Ok(TestCredentialResponse {
        success: report.ok,
        message: report.message.unwrap_or_else(|| {
            if report.ok {
                "credential accepted by provider".to_owned()
            } else {
                "credential rejected by provider".to_owned()
            }
        }),
        tested_at: chrono::Utc::now().to_rfc3339(),
    })
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
    scope: &Scope,
    cred: &str,
) -> ApiResult<RefreshCredentialResponse> {
    let svc = service(state)?;
    let tenant = TenantScope::from_scope(scope);
    let report = svc
        .refresh(&tenant, cred)
        .await
        .map_err(|e| map_service_err(e, cred))?;
    // The facade's fallback-on-interrupt serves the still-valid stored
    // material when the provider failed transiently — honest reporting:
    // that is NOT a refresh, and the old expiry is not a "new" one.
    Ok(if report.refreshed {
        RefreshCredentialResponse {
            refreshed: true,
            message: "credential refreshed".to_owned(),
            new_expires_at: report.head.expires_at.map(|t| t.to_rfc3339()),
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

/// Explicitly revoke the credential at the provider and delete the row.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn revoke_credential(
    state: &AppState,
    scope: &Scope,
    cred: &str,
) -> ApiResult<RevokeCredentialResponse> {
    let svc = service(state)?;
    let tenant = TenantScope::from_scope(scope);
    svc.revoke(&tenant, cred)
        .await
        .map_err(|e| map_service_err(e, cred))?;
    Ok(RevokeCredentialResponse {
        revoked: true,
        message: "credential revoked at the provider and removed".to_owned(),
    })
}

// ── Acquisition (resolve / continue) ─────────────────────────────────────────

/// Map the facade's [`InteractionRequest`] onto the wire DTO.
///
/// `InteractionRequest` is `#[non_exhaustive]`; an unrecognized future
/// arm is an internal composition gap (the api cannot instruct a UI it
/// does not understand), surfaced as 500 — never a fake interaction.
fn map_interaction(interaction: InteractionRequest) -> ApiResult<AcquisitionInteraction> {
    match interaction {
        InteractionRequest::Redirect { url } => Ok(AcquisitionInteraction::Redirect { url }),
        InteractionRequest::FormPost { url, fields } => Ok(AcquisitionInteraction::FormPost {
            url,
            fields: fields
                .into_iter()
                .map(|(name, value)| FormPostField { name, value })
                .collect(),
        }),
        InteractionRequest::DisplayInfo {
            title,
            message,
            data,
            expires_in,
        } => Ok(AcquisitionInteraction::DisplayInfo {
            title,
            message,
            data: serde_json::to_value(&data).map_err(|e| {
                ApiError::Internal(format!("failed to encode interaction display data: {e}"))
            })?,
            expires_in,
        }),
        other => Err(ApiError::Internal(format!(
            "unsupported credential interaction kind: {other:?}"
        ))),
    }
}

/// Map a facade [`Acquisition`] outcome onto the wire response.
fn map_acquisition(acq: Acquisition) -> ApiResult<ResolveCredentialResponse> {
    match acq {
        Acquisition::Complete { head } => Ok(ResolveCredentialResponse::Complete {
            credential_id: head.id,
        }),
        Acquisition::Pending { token, interaction } => Ok(ResolveCredentialResponse::Pending {
            pending_token: token,
            interaction: map_interaction(interaction)?,
        }),
        Acquisition::Retry { after } => Ok(ResolveCredentialResponse::Retry {
            retry_after_secs: after.as_secs(),
        }),
        other => Err(ApiError::Internal(format!(
            "unrecognized credential acquisition outcome: {other:?}"
        ))),
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
    scope: &Scope,
    user_id: &str,
    req: ResolveCredentialRequest,
) -> ApiResult<ResolveCredentialResponse> {
    validate_credential_data(state, &req.credential_key, &req.data)?;

    let svc = service(state)?;
    let tenant = TenantScope::from_scope(scope).with_session(user_id);
    let acq = svc
        .resolve(&tenant, &req.credential_key, req.data)
        .await
        .map_err(|e| map_service_err(e, "<resolve>"))?;
    map_acquisition(acq)
}

/// Continue a multi-step credential acquisition.
///
/// `user_input` is the typed continuation payload (the serialized
/// [`UserInput`] shape: `"Poll"`, `{"Code":{"code":".."}}`,
/// `{"Callback":{"params":{..}}}`, `{"FormData":{"params":{..}}}`).
#[tracing::instrument(skip_all, fields(cred.key = %req.credential_key))]
pub async fn continue_resolve(
    state: &AppState,
    scope: &Scope,
    user_id: &str,
    req: ContinueResolveRequest,
) -> ApiResult<ContinueResolveResponse> {
    let svc = service(state)?;
    let user_input: UserInput = serde_json::from_value(req.user_input).map_err(|_| {
        // The serde error text can echo the (potentially secret) payload —
        // deliberately omitted.
        ApiError::Validation {
            detail: "user_input is not a recognized continuation payload \
                     (expected Poll / Code / Callback / FormData)"
                .to_owned(),
            errors: vec![],
        }
    })?;
    let tenant = TenantScope::from_scope(scope).with_session(user_id);
    let acq = svc
        .continue_resolve(&tenant, &req.credential_key, &req.pending_token, user_input)
        .await
        .map_err(|e| map_service_err(e, "<continue>"))?;
    map_acquisition(acq)
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
    use nebula_storage::credential::EnvKeyProvider;
    use nebula_storage::inmem::{
        InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader,
        InMemoryNodeResultStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
    };

    /// 32 `0x42` bytes, base64 — a valid AES-256 key fixture (mirrors the
    /// factory's dev key). Not a secret: a fixed test constant.
    const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

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
    fn test_state() -> AppState {
        let port = crate::ports::credential_schema_registry::try_default_registry_port()
            .expect("first-party registry composes");
        let key =
            Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte AES key"));
        let svc = crate::ports::credential_service_factory::with_key_provider(key)
            .expect("service composes");
        base_state()
            .with_credential_schema(port)
            .with_credential_service(svc)
    }

    fn test_scope() -> Scope {
        Scope::new("w", "o")
    }

    /// §4.5 operational honesty: with no `CredentialService` wired, every
    /// credential operation refuses with a typed 503 — never a faked
    /// success, and no raw-store fallback path exists.
    #[tokio::test]
    async fn all_credential_fns_are_503_without_service() {
        let s = base_state();
        let scope = test_scope();
        assert!(matches!(
            get_credential(&s, &scope, "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            delete_credential(&s, &scope, "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            list_credentials(
                &s,
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
            test_credential(&s, &scope, "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            refresh_credential(&s, &scope, "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        assert!(matches!(
            revoke_credential(&s, &scope, "cred_x").await,
            Err(ApiError::ServiceUnavailable(_))
        ));
        // create/resolve hit the schema-port gate first (also absent here)
        // — still a 503, never a persist.
        assert!(matches!(
            create_credential(
                &s,
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
        assert!(matches!(
            scoped_store(&s, "o:w"),
            Err(ApiError::ServiceUnavailable(_))
        ));
    }

    /// CRUD through the facade: create → get → list → update (rename via
    /// CAS) → delete; the response projection never carries `data` and
    /// the secret never appears in any returned struct.
    #[tokio::test]
    async fn crud_round_trips_without_secret_in_projection() {
        let s = test_state();
        let scope = test_scope();
        let secret = "sk-unit-crud-NEVER-LEAK-7a7a";
        let created = create_credential(
            &s,
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

        let got = get_credential(&s, &scope, &created.id).await.expect("get");
        assert_eq!(got.id, created.id);
        assert!(!format!("{got:?}").contains(secret));

        let listed = list_credentials(
            &s,
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

        delete_credential(&s, &scope, &created.id)
            .await
            .expect("delete");
        assert!(matches!(
            get_credential(&s, &scope, &created.id).await,
            Err(ApiError::NotFound(_))
        ));
    }

    /// Lifecycle ops on a static type are a client error (400), sourced
    /// from the facade's capability gate — not a 503 and not a fake
    /// success.
    #[tokio::test]
    async fn lifecycle_on_static_type_is_validation_error() {
        let s = test_state();
        let scope = test_scope();
        let created = create_credential(
            &s,
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
            test_credential(&s, &scope, &created.id).await,
            Err(ApiError::Validation { .. })
        ));
        assert!(matches!(
            refresh_credential(&s, &scope, &created.id).await,
            Err(ApiError::Validation { .. })
        ));
        assert!(matches!(
            revoke_credential(&s, &scope, &created.id).await,
            Err(ApiError::Validation { .. })
        ));
    }

    /// Generic resolve completes synchronously for a static type and the
    /// persisted credential is visible to the CRUD plane (one store).
    #[tokio::test]
    async fn resolve_complete_persists_and_is_visible_to_crud() {
        let s = test_state();
        let scope = test_scope();
        let res = resolve_credential(
            &s,
            &scope,
            "user-1",
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
        let got = get_credential(&s, &scope, &credential_id)
            .await
            .expect("resolved credential is gettable");
        assert_eq!(got.credential_key, "api_key");
    }

    /// Cross-workspace ids collapse to a flat 404 (no existence
    /// disclosure) end-to-end through the transport mapping.
    #[tokio::test]
    async fn cross_workspace_get_is_flat_404() {
        let s = test_state();
        let scope_a = Scope::new("ws-a", "org");
        let scope_b = Scope::new("ws-b", "org");
        let created = create_credential(
            &s,
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

        let err = get_credential(&s, &scope_b, &created.id)
            .await
            .expect_err("cross-workspace get denied");
        assert!(matches!(err, ApiError::NotFound(_)));
    }

    /// The service-error mapping is total and secret-safe for the
    /// client-relevant arms.
    #[test]
    fn service_error_mapping_statuses() {
        assert!(matches!(
            map_service_err(
                CredentialServiceError::NotFound { id: "x".into() },
                "cred_x"
            ),
            ApiError::NotFound(_)
        ));
        assert!(matches!(
            map_service_err(
                CredentialServiceError::VersionConflict {
                    id: "x".into(),
                    expected: 1,
                    actual: 2
                },
                "cred_x"
            ),
            ApiError::VersionMismatch(_)
        ));
        assert!(matches!(
            map_service_err(
                CredentialServiceError::ValidationFailed {
                    reason: "[code] /path".into()
                },
                "cred_x"
            ),
            ApiError::Validation { .. }
        ));
        assert!(matches!(
            map_service_err(
                CredentialServiceError::TypeUnknown { key: "nope".into() },
                "cred_x"
            ),
            ApiError::Validation { .. }
        ));
        assert!(matches!(
            map_service_err(
                CredentialServiceError::CapabilityUnsupported {
                    capability: "refresh".into(),
                    key: "api_key".into()
                },
                "cred_x"
            ),
            ApiError::Validation { .. }
        ));
        assert!(matches!(
            map_service_err(CredentialServiceError::PendingExpired, "cred_x"),
            ApiError::Unauthorized(_)
        ));
        assert!(matches!(
            map_service_err(CredentialServiceError::Provider("down".into()), "cred_x"),
            ApiError::ServiceUnavailable(_)
        ));
        assert!(matches!(
            map_service_err(CredentialServiceError::Store("io".into()), "cred_x"),
            ApiError::Internal(_)
        ));
    }
}
