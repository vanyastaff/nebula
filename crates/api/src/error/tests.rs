#[test]
fn stored_revision_record_limit_failures_are_internal_and_payload_free() {
    use nebula_error::Classify;
    use nebula_storage_port::dto::{PlanFlavorRevisionTarget, RevisionCatalogError};

    let target = PlanFlavorRevisionTarget::ExecutablePlan(
        nebula_core::ExecutablePlanRevisionId::from_bytes([0x5a; 32]),
    );
    let cases = [
        (
            "record byte limit",
            RevisionCatalogError::RecordTooLarge {
                max_bytes: 1_048_576,
                actual_bytes: 9_876_543,
            },
        ),
        (
            "record nesting limit",
            RevisionCatalogError::RecordNestingTooDeep { target },
        ),
        (
            "single string limit",
            RevisionCatalogError::RecordStringTooLarge { target },
        ),
        (
            "aggregate string budget",
            RevisionCatalogError::RecordStringBudgetExceeded { target },
        ),
        (
            "collection entry budget",
            RevisionCatalogError::RecordCollectionBudgetExceeded { target },
        ),
    ];

    for (case, source) in cases {
        let start_error = nebula_engine::WorkflowStartError::RevisionUnavailable(Box::new(
            nebula_engine::PlanFlavorRevisionBridgeError::Catalog { source },
        ));
        let api_error = ApiError::from(&start_error);
        std::assert_matches!(
            &api_error,
            ApiError::Internal(message)
                if message == "Stored workflow revisions are inconsistent",
            "unexpected API classification for {case}: {api_error:?}"
        );
        assert_eq!(
            api_error.category(),
            nebula_error::ErrorCategory::Internal,
            "unexpected error category for {case}"
        );
        assert_eq!(api_error.code().as_str(), "API:INTERNAL");

        let (status, problem) = api_error.to_problem_details();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "case: {case}");
        assert_eq!(
            serde_json::to_value(problem).expect("problem details serialize"),
            serde_json::json!({
                "type": "about:blank",
                "title": "Internal Server Error",
                "status": 500,
            }),
            "problem details exposed diagnostics for {case}"
        );
    }
}

#[test]
fn emitter_admission_source_maps_to_the_same_transport_error() {
    let error =
        nebula_action::ActionError::fatal_from(nebula_engine::WorkflowStartError::MissingWorkflow);
    let source = std::error::Error::source(&error).expect("typed emitter error retains source");
    let admission = source
        .downcast_ref::<nebula_engine::WorkflowStartError>()
        .expect("source is the original runtime admission error");
    assert_eq!(
        ApiError::from(admission).to_problem_details().0,
        StatusCode::NOT_FOUND
    );
}

#[test]
fn accepted_receipt_failure_retains_identity_without_a_retry_hint() {
    let execution_id = nebula_core::ExecutionId::new();
    let error =
        ApiError::from(&nebula_engine::WorkflowStartError::ReceiptUnavailable { execution_id });
    let (status, problem) = error.to_problem_details();
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    let encoded = serde_json::to_value(problem).unwrap();
    assert_eq!(encoded["execution_id"], execution_id.to_string());
    assert_eq!(encoded["accepted"], true);
    assert!(
        error
            .into_response()
            .headers()
            .get(header::RETRY_AFTER)
            .is_none()
    );
}

#[test]
fn binding_resolution_failures_are_secret_free_and_backend_outage_is_retryable() {
    use nebula_engine::{BindingResolutionError, WorkflowStartError};

    for source in [
        BindingResolutionError::NotFound,
        BindingResolutionError::Ambiguous,
        BindingResolutionError::Incompatible,
        BindingResolutionError::InvalidManifest,
    ] {
        let error = ApiError::from(&WorkflowStartError::BindingResolution(source));
        let (status, problem) = error.to_problem_details();
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let encoded = serde_json::to_string(&problem).expect("problem details serialize");
        assert!(!encoded.contains("credential"));
        assert!(!encoded.contains("resource"));
        assert!(!encoded.contains("selector"));
    }

    let unavailable = ApiError::from(&WorkflowStartError::BindingResolution(
        BindingResolutionError::Unavailable,
    ));
    assert_eq!(
        unavailable.to_problem_details().0,
        StatusCode::SERVICE_UNAVAILABLE
    );
}

use axum::{
    http::{StatusCode, header},
    response::IntoResponse,
};

use super::*;
use nebula_validator::foundation::ValidationError;

#[test]
fn validation_error_conversion_preserves_code_and_pointer() {
    let err = ValidationError::new("min_length", "Must be at least 3 characters")
        .with_field("profile.name");

    let api_error = ApiError::from(err);
    let (status, problem) = api_error.to_problem_details();

    assert_eq!(status, StatusCode::BAD_REQUEST);
    let errors = problem.errors.expect("validation errors must be present");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].code, "min_length");
    assert_eq!(errors[0].pointer.as_deref(), Some("/profile/name"));
}

#[test]
fn nested_validation_error_conversion_keeps_nested_entries() {
    let err = ValidationError::new("object_invalid", "Object validation failed").with_nested(vec![
        ValidationError::new("required", "Field is required").with_pointer("/email"),
    ]);

    let api_error = ApiError::from(err);
    let (status, problem) = api_error.to_problem_details();

    assert_eq!(status, StatusCode::BAD_REQUEST);
    let errors = problem.errors.expect("validation errors must be present");
    assert!(errors.iter().any(|e| e.code == "object_invalid"));
    assert!(
        errors
            .iter()
            .any(|e| e.code == "required" && e.pointer.as_deref() == Some("/email"))
    );
}

#[test]
fn invalid_workflow_definition_node_error_produces_node_pointer() {
    use nebula_core::node_key;
    use nebula_workflow::WorkflowError;

    let node = node_key!("step_a");
    let api_error = ApiError::InvalidWorkflowDefinition {
        detail: "1 error(s)".to_string(),
        errors: vec![WorkflowError::DuplicateNodeKey(node)],
    };
    let (status, problem) = api_error.to_problem_details();

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let errors = problem.errors.expect("errors must be present");
    assert_eq!(errors.len(), 1);
    assert!(
        errors[0]
            .path
            .as_deref()
            .is_some_and(|path| path.starts_with("/nodes/")),
        "DuplicateNodeKey must produce a /nodes/<key> pointer, got: {:?}",
        errors[0].pointer
    );
    assert_eq!(errors[0].path.as_deref(), Some("/nodes/step_a"));
}

#[test]
fn invalid_workflow_definition_structural_error_points_at_the_offending_section() {
    use nebula_workflow::WorkflowError;

    let api_error = ApiError::InvalidWorkflowDefinition {
        detail: "1 error(s)".to_string(),
        errors: vec![WorkflowError::CycleDetected],
    };
    let (status, problem) = api_error.to_problem_details();

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let errors = problem.errors.expect("errors must be present");
    assert_eq!(errors.len(), 1);

    // Structural rejections used to collapse to the RFC 6901 root pointer
    // (the empty string). Activation diagnostics require a non-empty path on every
    // diagnostic, and a cycle is always a property of the connections, so
    // pointing there is both required and strictly more useful than
    // pointing at the whole document.
    assert_eq!(errors[0].path.as_deref(), Some("/connections"));
    assert_eq!(errors[0].code, "WORKFLOW:CYCLE_DETECTED");
    assert_eq!(
        errors[0].expected.as_deref(),
        Some("an acyclic graph"),
        "the contract that was required travels as its own field"
    );
    assert_eq!(
        errors[0].actual.as_deref(),
        Some("a graph containing a cycle")
    );
    assert!(
        errors[0]
            .remediation
            .as_deref()
            .is_some_and(|text| text.contains("cycle")),
        "an author is told what to change, not just what is wrong"
    );
}

#[test]
fn invalid_workflow_definition_connection_error_produces_connection_pointer() {
    use nebula_core::node_key;
    use nebula_workflow::WorkflowError;

    let from = node_key!("a");
    let to = node_key!("b");
    let api_error = ApiError::InvalidWorkflowDefinition {
        detail: "1 error(s)".to_string(),
        errors: vec![WorkflowError::DuplicateConnection { from, to }],
    };
    let (status, problem) = api_error.to_problem_details();

    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let errors = problem.errors.expect("errors must be present");
    assert_eq!(errors.len(), 1);
    assert_eq!(
        errors[0].path.as_deref(),
        Some("/connections/a/b"),
        "a connection rejection names both endpoints it wires"
    );
    assert_eq!(
        errors[0].pointer, None,
        "an activation diagnostic reports a logical path, never a JSON Pointer it \
         could not resolve against an array of connections"
    );
}

#[test]
fn refresh_not_applied_never_is_a_fixed_409_without_retry_after() {
    use nebula_error::Classify;

    let error = ApiError::CredentialRefreshNotAppliedNever;
    let (status, problem) = error.to_problem_details();

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error.category(), nebula_error::ErrorCategory::Conflict);
    assert_eq!(
        error.code().as_str(),
        "API:CREDENTIAL_REFRESH_NOT_APPLIED_NEVER"
    );
    assert!(!error.is_retryable());
    assert_eq!(error.retry_hint(), None);
    assert_eq!(
        problem.type_uri,
        "https://nebula.dev/problems/credential-refresh-not-applied"
    );
    assert_eq!(problem.title, "Credential Refresh Not Applied");
    assert_eq!(
        problem.detail.as_deref(),
        Some("The credential refresh was not applied for the current credential state.")
    );
    assert!(
        !problem
            .detail
            .as_deref()
            .is_some_and(|detail| detail.to_ascii_lowercase().contains("retry")),
        "Never must not advise the client to retry"
    );

    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(!response.headers().contains_key(header::RETRY_AFTER));
}

#[test]
fn refresh_not_applied_after_is_a_fixed_409_with_retry_after() {
    use nebula_error::Classify;

    let error = ApiError::CredentialRefreshNotAppliedAfter {
        retry_after_secs: NonZeroU64::new(17).expect("test delay is non-zero"),
    };
    let (status, problem) = error.to_problem_details();

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error.category(), nebula_error::ErrorCategory::Conflict);
    assert_eq!(
        error.code().as_str(),
        "API:CREDENTIAL_REFRESH_NOT_APPLIED_AFTER"
    );
    assert!(error.is_retryable());
    assert_eq!(error.retry_hint(), None);
    assert_eq!(
        problem.type_uri,
        "https://nebula.dev/problems/credential-refresh-not-applied"
    );
    assert_eq!(problem.title, "Credential Refresh Not Applied");
    assert_eq!(
        problem.detail.as_deref(),
        Some("The credential refresh was not applied. Retry only after the Retry-After delay.")
    );

    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        response.headers().get(header::RETRY_AFTER),
        Some(&axum::http::HeaderValue::from_static("17"))
    );
}

#[test]
fn refresh_reconciliation_required_is_a_fixed_non_retryable_409() {
    use nebula_error::Classify;

    let error = ApiError::CredentialRefreshReconciliationRequired;
    let (status, problem) = error.to_problem_details();

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error.category(), nebula_error::ErrorCategory::Conflict);
    assert_eq!(
        error.code().as_str(),
        "API:CREDENTIAL_REFRESH_RECONCILIATION_REQUIRED"
    );
    assert!(!error.is_retryable());
    assert_eq!(error.retry_hint(), None);
    assert_eq!(
        problem.type_uri,
        "https://nebula.dev/problems/credential-refresh-reconciliation-required"
    );
    assert_eq!(problem.title, "Credential Refresh Reconciliation Required");
    assert_eq!(
        problem.detail.as_deref(),
        Some(
            "The refresh outcome is known, but durable local finalization definitely failed. Do not retry automatically; reconcile or reconnect the integration credential."
        )
    );

    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(!response.headers().contains_key(header::RETRY_AFTER));
}

#[test]
fn revoke_reconciliation_required_is_a_fixed_non_retryable_409() {
    use nebula_error::Classify;

    let error = ApiError::CredentialRevokeReconciliationRequired;
    let (status, problem) = error.to_problem_details();

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error.category(), nebula_error::ErrorCategory::Conflict);
    assert_eq!(
        error.code().as_str(),
        "API:CREDENTIAL_REVOKE_RECONCILIATION_REQUIRED"
    );
    assert!(!error.is_retryable());
    assert_eq!(error.retry_hint(), None);
    assert_eq!(
        problem.type_uri,
        "https://nebula.dev/problems/credential-revoke-reconciliation-required"
    );
    assert_eq!(problem.title, "Credential Revoke Reconciliation Required");
    assert_eq!(
        problem.detail.as_deref(),
        Some(
            "The revoke outcome is known, but durable local finalization definitely failed. Do not retry automatically; reconcile credential state."
        )
    );

    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(!response.headers().contains_key(header::RETRY_AFTER));
}

#[test]
fn acquisition_reconciliation_required_is_a_fixed_non_retryable_409() {
    use nebula_error::Classify;

    let error = ApiError::CredentialAcquisitionReconciliationRequired;
    let (status, problem) = error.to_problem_details();

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error.category(), nebula_error::ErrorCategory::Conflict);
    assert_eq!(
        error.code().as_str(),
        "API:CREDENTIAL_ACQUISITION_RECONCILIATION_REQUIRED"
    );
    assert!(!error.is_retryable());
    assert_eq!(error.retry_hint(), None);
    assert_eq!(
        problem.type_uri,
        "https://nebula.dev/problems/credential-acquisition-reconciliation-required"
    );
    assert_eq!(
        problem.title,
        "Credential Acquisition Reconciliation Required"
    );

    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(!response.headers().contains_key(header::RETRY_AFTER));
}

#[test]
fn credential_state_refused_is_a_fixed_non_retryable_409() {
    use nebula_error::Classify;

    let error = ApiError::CredentialStateRefused;
    let (status, problem) = error.to_problem_details();

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(error.category(), nebula_error::ErrorCategory::Conflict);
    assert_eq!(error.code().as_str(), "API:CREDENTIAL_STATE_REFUSED");
    assert!(!error.is_retryable());
    assert_eq!(error.retry_hint(), None);
    assert_eq!(
        problem.type_uri,
        "https://nebula.dev/problems/credential-state-refused"
    );
    assert_eq!(problem.title, "Credential State Refused");
    assert_eq!(
        problem.detail.as_deref(),
        Some(
            "The stored credential state is not compatible with this runtime. Update the runtime or repair the credential state before retrying."
        )
    );

    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(!response.headers().contains_key(header::RETRY_AFTER));
}

#[test]
fn reconcile_refusals_are_distinguishable_fixed_non_retryable_409s() {
    use nebula_error::Classify;

    // The two refusals call for different next steps — "nothing to
    // adjudicate" is terminal for the request, while a conflicting decision
    // means the operator has to read what is already on record — so each
    // carries its own code and problem type instead of sharing the generic
    // conflict response and being told apart only by free-text detail.
    for (error, code, problem_type, title) in [
        (
            ApiError::CredentialReconciliationNotRequired,
            "API:CREDENTIAL_RECONCILIATION_NOT_REQUIRED",
            "https://nebula.dev/problems/credential-reconciliation-not-required",
            "Credential Reconciliation Not Required",
        ),
        (
            ApiError::CredentialReconciliationConflict {
                recorded_digest: "ab".repeat(32),
                recorded_decision: "provider_applied".to_owned(),
            },
            "API:CREDENTIAL_RECONCILIATION_CONFLICT",
            "https://nebula.dev/problems/credential-reconciliation-conflict",
            "Credential Reconciliation Conflict",
        ),
    ] {
        let (status, problem) = error.to_problem_details();
        assert_eq!(status, StatusCode::CONFLICT, "wrong status for {code}");
        assert_eq!(error.category(), nebula_error::ErrorCategory::Conflict);
        assert_eq!(error.code().as_str(), code);
        assert!(!error.is_retryable(), "wrong retryability for {code}");
        assert_eq!(error.retry_hint(), None);
        assert_eq!(problem.type_uri, problem_type);
        assert_eq!(problem.title, title);

        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(!response.headers().contains_key(header::RETRY_AFTER));
    }

    // The conflict problem names the recorded pair as extensions, so a
    // client holding its original evidence can confirm what is on record
    // rather than guess which of two observations disagreed.
    let (_, problem) = ApiError::CredentialReconciliationConflict {
        recorded_digest: "ab".repeat(32),
        recorded_decision: "provider_applied".to_owned(),
    }
    .to_problem_details();
    assert_eq!(
        problem.extensions,
        Some(serde_json::json!({
            "evidence_digest": "ab".repeat(32),
            "recorded_decision": "provider_applied",
        })),
        "the conflict problem must carry the recorded pair as extensions"
    );
}
