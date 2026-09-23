use std::{assert_matches, sync::Arc};

use super::*;
use crate::ports::credential_command::{
    CredentialGatewayValidationIssue, CredentialGatewayValidationReport,
};
use crate::ports::credential_schema::{CredentialValidationCode, CredentialValidationLocation};
use nebula_storage::credential::EnvKeyProvider;
use nebula_storage::inmem::{
    InMemoryControlQueue, InMemoryExecutionStore, InMemoryJournalReader, InMemoryNodeResultStore,
    InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
};
use nebula_storage_port::store::RefreshOutcomeDecision;

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
    let workflow_store = InMemoryWorkflowStore::new_with_versions(&workflow_versions, &exec_store);
    AppState::new(
        Arc::new(workflow_store),
        Arc::new(workflow_versions),
        Arc::new(exec_store.clone()),
        Arc::new(InMemoryNodeResultStore::new()),
        Arc::new(journal),
        Arc::new(control_queue),
        Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
            &exec_store,
        )),
        Arc::new(nebula_storage::inmem::InMemoryTurnHandoff::new(&exec_store)),
        Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
            &exec_store,
        )),
        jwt,
    )
}

/// State with the real registry-backed schema port AND a composed
/// `CredentialService` — the production shape.
async fn test_state() -> AppState {
    let port = crate::ports::credential_schema_registry::try_default_registry_port()
        .expect("first-party registry composes");
    let key = Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid 32-byte AES key"));
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
    // Reconciliation is an operator remedy, not a credential write, but it
    // still needs the service behind it: with no gateway there is nothing
    // that could record a provider outcome, and answering 200 would tell an
    // operator the conflict is settled when no claim was ever touched.
    assert!(matches!(
        reconcile_credential(
            &s,
            &principal,
            &scope,
            "cred_x",
            &ReconcileCredentialRequest {
                decision: CredentialReconcileDecisionV1::ProviderNotApplied,
                evidence: "e".into(),
            }
        )
        .await,
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
    assert_eq!(
        created.lifecycle,
        crate::domain::credential::dto::CredentialLifecycleState::Ready
    );
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
    assert_matches!(
        map_gateway_err(CredentialGatewayError::ReconciliationNotRequired, "cred_x"),
        ApiError::CredentialReconciliationNotRequired
    );
    let ApiError::CredentialReconciliationConflict {
        recorded_digest,
        recorded_decision,
    } = map_gateway_err(
        CredentialGatewayError::ReconciliationConflict {
            recorded_digest: [0xabu8; 32],
            recorded_decision: RefreshOutcomeDecision::ProviderApplied,
        },
        "cred_x",
    )
    else {
        panic!("a reconciliation conflict must map to the conflict error");
    };
    assert_eq!(
        recorded_digest,
        "ab".repeat(32),
        "the conflict must carry the recorded digest as lowercase hex"
    );
    assert_eq!(
        recorded_decision, "provider_applied",
        "the conflict must carry the recorded decision in its wire spelling"
    );
    assert_matches!(
        map_gateway_err(
            CredentialGatewayError::ReconciliationEvidenceInvalid,
            "cred_x"
        ),
        ApiError::Validation { .. }
    );
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
        CredentialGatewayError::ReconciliationNotRequired,
        CredentialGatewayError::ReconciliationConflict {
            recorded_digest: [0xabu8; 32],
            recorded_decision: RefreshOutcomeDecision::ProviderApplied,
        },
        CredentialGatewayError::ReconciliationEvidenceInvalid,
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
