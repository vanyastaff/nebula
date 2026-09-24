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
        CredentialCapabilities, CredentialReconcileDecisionV1, CredentialResponse,
        CredentialSummary, CredentialTestFailureCodeV1, CredentialTypeInfo,
        ListCredentialTypesResponse, ListCredentialsQuery, ListCredentialsResponse,
        ReauthorizeCredentialRequest, ReauthorizeCredentialResponse, ReconcileCredentialRequest,
        ReconcileCredentialResponse, RefreshCredentialResponse, ResolveCredentialRequest,
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
                .map(|issue| {
                    crate::error::ValidationFieldError::field(
                        issue.code().to_owned(),
                        issue.message().to_owned(),
                        issue.path().to_owned(),
                    )
                })
                .collect(),
        },
        CredentialGatewayError::StateEnvelopeRefused => ApiError::CredentialStateRefused,
        CredentialGatewayError::TypeUnknown { key } => ApiError::Validation {
            detail: format!("unknown credential type: {key}"),
            errors: vec![crate::error::ValidationFieldError::field(
                "unknown_credential_type",
                "no such credential type",
                "/credential_key",
            )],
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
        CredentialGatewayError::AcquisitionReconciliationRequired => {
            ApiError::CredentialAcquisitionReconciliationRequired
        },
        CredentialGatewayError::RevokeReconciliationRequired => {
            ApiError::CredentialRevokeReconciliationRequired
        },
        // 409 for both: the credential exists and the caller is authorized, so
        // neither is a 404 and neither is a 403 — the request contradicts the
        // claim's durable state. Each gets its own variant and problem type,
        // following the two reconciliation-required siblings above: the two
        // refusals are acted on differently, so a client that could only read
        // them apart from a free-text `detail` had no contractual way to tell
        // them apart at all.
        CredentialGatewayError::ReconciliationNotRequired => {
            ApiError::CredentialReconciliationNotRequired
        },
        CredentialGatewayError::ReconciliationConflict {
            recorded_digest,
            recorded_decision,
        } => ApiError::CredentialReconciliationConflict {
            recorded_digest: digest_hex(&recorded_digest),
            recorded_decision: recorded_decision.as_str().to_owned(),
        },
        CredentialGatewayError::ReconciliationEvidenceInvalid => ApiError::Validation {
            detail: "reconciliation evidence was rejected".to_owned(),
            errors: vec![],
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
        lifecycle: lifecycle_response(record.lifecycle),
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
        lifecycle: lifecycle_response(record.lifecycle),
    }
}

fn lifecycle_response(
    lifecycle: crate::ports::credential_command::CredentialGatewayLifecycleState,
) -> crate::domain::credential::dto::CredentialLifecycleState {
    use crate::domain::credential::dto::CredentialLifecycleState;
    use crate::ports::credential_command::CredentialGatewayLifecycleState;

    match lifecycle {
        CredentialGatewayLifecycleState::Ready => CredentialLifecycleState::Ready,
        CredentialGatewayLifecycleState::RefreshDeferred { retry_at } => {
            CredentialLifecycleState::RefreshDeferred {
                retry_at: retry_at.to_rfc3339(),
            }
        },
        CredentialGatewayLifecycleState::RefreshBlocked => CredentialLifecycleState::RefreshBlocked,
        CredentialGatewayLifecycleState::ReauthRequired => CredentialLifecycleState::ReauthRequired,
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
/// `lifecycle`.
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

/// Record the provider outcome for a refresh whose local result is unknown.
///
/// This is the only shipped caller that can clear a **retained poison**: when a
/// refresh's claim expires in flight, `RefreshClaimStore::release` refuses to
/// delete the row while an unresolved incident exists
/// (`crates/storage-port/src/store/refresh_claim.rs:211-227`) and the refresh
/// coordinator swallows that refusal
/// (`crates/credential/src/runtime/refresh/coordinator.rs:520-539`), so
/// `try_claim` answers `OutcomeUnknown` for that credential from then on and
/// the refresh route answers 409 forever. Recording the decision is what
/// retires the incident and lets the next refresh acquire a claim again.
///
/// Authority is not duplicated here: the caller's `scope` is passed through
/// unchanged and no ownership check is added, so the credential controller's
/// read at `crates/credential/src/service/controller.rs:597-620` stays the one
/// place a caller's tenant meets the credential being reconciled. The port
/// takes no scope operand, which is exactly why that read has to remain the
/// single authority.
#[tracing::instrument(skip_all, fields(cred.id = %cred))]
pub async fn reconcile_credential(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    cred: &str,
    request: &ReconcileCredentialRequest,
) -> ApiResult<ReconcileCredentialResponse> {
    let result = gateway(state)?
        .execute(
            principal,
            scope,
            CredentialGatewayCommand::Reconcile {
                credential_id: cred.to_owned(),
                decision: request.decision.to_port(),
                evidence: request.evidence.clone(),
            },
        )
        .await
        .map_err(|e| map_gateway_err(e, cred))?;
    let CredentialGatewayResult::Reconciled {
        decision: port_decision,
        changed,
        evidence_digest,
    } = result
    else {
        return Err(ApiError::Internal(
            "credential gateway returned an invalid reconcile result".to_owned(),
        ));
    };
    // `changed: false` is the idempotent recommit of an identical
    // `(decision, evidence)` pair already on record for the same incident. It
    // is reported as success with an honest message, not folded into a
    // conflict: the caller's intent is satisfied either way, and the only thing
    // they can act on differently is whether this call wrote.
    Ok(ReconcileCredentialResponse {
        decision: CredentialReconcileDecisionV1::from_port(port_decision),
        changed,
        evidence_digest: digest_hex(&evidence_digest),
        message: if changed {
            "provider outcome recorded; the credential can be refreshed again".to_owned()
        } else {
            "identical decision was already on record; nothing changed".to_owned()
        },
    })
}

/// Lowercase hex of a 32-byte evidence digest — the wire spelling of the
/// reconciliation retry identity on the success response and the conflict
/// problem's `evidence_digest` extension.
///
/// Inline rather than a `hex` dependency: one `String` result and no hex-crate
/// edge on the api library surface (the same shape as the idempotency cache
/// key).
fn digest_hex(digest: &[u8; 32]) -> String {
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push_str(&format!("{byte:02x}"));
    }
    encoded
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

/// Begin a replacement authorization bound to an existing credential.
#[tracing::instrument(skip_all)]
pub async fn reauthorize_credential(
    state: &AppState,
    principal: &AuthenticatedPrincipal,
    scope: &Scope,
    credential_id: &str,
    request: ReauthorizeCredentialRequest,
) -> ApiResult<ReauthorizeCredentialResponse> {
    let result = gateway(state)?
        .execute(
            principal,
            scope,
            CredentialGatewayCommand::Reauthorize {
                credential_id: credential_id.to_owned(),
                request,
            },
        )
        .await
        .map_err(|error| map_gateway_err(error, credential_id))?;
    let CredentialGatewayResult::Acquisition(acquisition) = result else {
        return Err(ApiError::Internal(
            "credential gateway returned an invalid reauthorization result".to_owned(),
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
#[path = "credential_tests.rs"]
mod tests;
