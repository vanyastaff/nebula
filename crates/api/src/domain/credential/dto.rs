//! Server compatibility imports and transport-to-domain mappings.

pub use nebula_api_contract::v1::credential::*;

use nebula_storage_port::store::{
    CredentialOperationDecision, CredentialOperationKind, RefreshOutcomeDecision,
    RevokeOutcomeDecision,
};
#[cfg(test)]
use std::collections::HashMap;

/// The typed port decision named by this wire pair.
#[must_use]
pub(crate) const fn reconcile_decision_to_port(
    decision: CredentialReconcileDecisionV1,
    operation: CredentialReconcileOperationV1,
) -> Option<CredentialOperationDecision> {
    match (operation, decision) {
        (
            CredentialReconcileOperationV1::Refresh,
            CredentialReconcileDecisionV1::ProviderApplied,
        ) => Some(CredentialOperationDecision::Refresh(
            RefreshOutcomeDecision::ProviderApplied,
        )),
        (
            CredentialReconcileOperationV1::Refresh,
            CredentialReconcileDecisionV1::ProviderNotApplied,
        ) => Some(CredentialOperationDecision::Refresh(
            RefreshOutcomeDecision::ProviderNotApplied,
        )),
        (
            CredentialReconcileOperationV1::Revoke,
            CredentialReconcileDecisionV1::ProviderRevoked,
        ) => Some(CredentialOperationDecision::Revoke(
            RevokeOutcomeDecision::ProviderRevoked,
        )),
        (
            CredentialReconcileOperationV1::Revoke,
            CredentialReconcileDecisionV1::ProviderNotRevoked,
        ) => Some(CredentialOperationDecision::Revoke(
            RevokeOutcomeDecision::ProviderNotRevoked,
        )),
        _ => None,
    }
}

/// The wire value naming `decision`.
///
/// Total arms and no wildcard, which is the point: the port enum is not
/// `#[non_exhaustive]`, so a new provider outcome breaks this build
/// instead of silently classifying as an existing decision on a command that
/// decides whether a credential may be used again.
#[must_use]
pub(crate) const fn reconcile_decision_from_port(
    decision: CredentialOperationDecision,
) -> CredentialReconcileDecisionV1 {
    match decision {
        CredentialOperationDecision::Refresh(RefreshOutcomeDecision::ProviderApplied) => {
            CredentialReconcileDecisionV1::ProviderApplied
        },
        CredentialOperationDecision::Refresh(RefreshOutcomeDecision::ProviderNotApplied) => {
            CredentialReconcileDecisionV1::ProviderNotApplied
        },
        CredentialOperationDecision::Revoke(RevokeOutcomeDecision::ProviderRevoked) => {
            CredentialReconcileDecisionV1::ProviderRevoked
        },
        CredentialOperationDecision::Revoke(RevokeOutcomeDecision::ProviderNotRevoked) => {
            CredentialReconcileDecisionV1::ProviderNotRevoked
        },
    }
}

/// Project a typed port operation into the public wire vocabulary.
#[must_use]
pub(crate) const fn reconcile_operation_from_port(
    operation: CredentialOperationKind,
) -> Option<CredentialReconcileOperationV1> {
    match operation {
        CredentialOperationKind::Refresh => Some(CredentialReconcileOperationV1::Refresh),
        CredentialOperationKind::Revoke => Some(CredentialReconcileOperationV1::Revoke),
        CredentialOperationKind::LegacyUnclassified => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET_CANARY: &str = "credential-dto-secret-NEVER-DEBUG-c41f";

    #[test]
    fn create_request_debug_redacts_free_form_fields() {
        let request = CreateCredentialRequest {
            credential_key: "api_key".to_owned(),
            name: SECRET_CANARY.to_owned(),
            description: Some(SECRET_CANARY.to_owned()),
            data: serde_json::json!({ "api_key": SECRET_CANARY }),
            tags: Some(HashMap::from([(
                SECRET_CANARY.to_owned(),
                SECRET_CANARY.to_owned(),
            )])),
        };

        let debug = format!("{request:?}");
        assert!(
            !debug.contains(SECRET_CANARY),
            "create request Debug must not expose free-form input: {debug}"
        );
    }

    #[test]
    fn update_request_debug_redacts_free_form_fields() {
        let request = UpdateCredentialRequest {
            name: Some(SECRET_CANARY.to_owned()),
            description: Some(SECRET_CANARY.to_owned()),
            data: Some(serde_json::json!({ "api_key": SECRET_CANARY })),
            tags: Some(HashMap::from([(
                SECRET_CANARY.to_owned(),
                SECRET_CANARY.to_owned(),
            )])),
            version: Some(42),
        };

        let debug = format!("{request:?}");
        assert!(
            !debug.contains(SECRET_CANARY),
            "update request Debug must not expose free-form input: {debug}"
        );
        assert!(
            debug.contains("42"),
            "safe CAS version should remain visible"
        );
    }

    #[test]
    fn credential_projection_debug_redacts_user_metadata() {
        let response = CredentialResponse {
            id: "cred-123".to_owned(),
            credential_key: "api_key".to_owned(),
            name: SECRET_CANARY.to_owned(),
            description: Some(SECRET_CANARY.to_owned()),
            auth_pattern: "SecretToken".to_owned(),
            capabilities: CredentialCapabilities {
                interactive: false,
                refreshable: false,
                testable: true,
                revocable: true,
            },
            created_at: "2026-07-21T12:34:56Z".to_owned(),
            updated_at: "2026-07-21T12:34:56Z".to_owned(),
            expires_at: None,
            version: 42,
            lifecycle: CredentialLifecycleState::Ready,
            tags: HashMap::from([(SECRET_CANARY.to_owned(), SECRET_CANARY.to_owned())]),
        };
        let summary = CredentialSummary {
            id: "cred-123".to_owned(),
            credential_key: "api_key".to_owned(),
            name: SECRET_CANARY.to_owned(),
            auth_pattern: "SecretToken".to_owned(),
            expires_at: None,
            version: 42,
            lifecycle: CredentialLifecycleState::Ready,
        };

        for debug in [format!("{response:?}"), format!("{summary:?}")] {
            assert!(
                !debug.contains(SECRET_CANARY),
                "credential projection Debug must not expose user metadata: {debug}"
            );
            assert!(
                debug.contains("cred-123") && debug.contains("42"),
                "safe identity and concurrency fields should remain visible: {debug}"
            );
        }
    }

    #[test]
    fn resolve_request_debug_redacts_input_data() {
        let request = ResolveCredentialRequest {
            credential_key: "probe".to_owned(),
            data: serde_json::json!({ "client_secret": SECRET_CANARY }),
        };

        let debug = format!("{request:?}");
        assert!(
            !debug.contains(SECRET_CANARY),
            "resolve request Debug must not expose input data: {debug}"
        );
    }

    #[test]
    fn continue_request_debug_redacts_token_and_user_input() {
        let request = ContinueResolveRequest {
            credential_key: "probe".to_owned(),
            pending_token: SECRET_CANARY.to_owned(),
            user_input: serde_json::json!({ "code": SECRET_CANARY }),
        };

        let debug = format!("{request:?}");
        assert!(
            !debug.contains(SECRET_CANARY),
            "continue request Debug must not expose token or user input: {debug}"
        );
    }

    #[test]
    fn reconcile_request_debug_redacts_evidence() {
        let request = ReconcileCredentialRequest {
            operation: CredentialReconcileOperationV1::Refresh,
            incident: uuid::Uuid::nil(),
            decision: CredentialReconcileDecisionV1::ProviderApplied,
            evidence: SECRET_CANARY.to_owned(),
        };

        let debug = format!("{request:?}");
        assert!(
            !debug.contains(SECRET_CANARY),
            "reconcile request Debug must not expose the operator evidence: {debug}"
        );
        assert!(
            debug.contains("ProviderApplied"),
            "the decision is not sensitive and should remain visible: {debug}"
        );
    }

    #[test]
    fn pending_response_debug_redacts_token_and_interaction_payload() {
        let response = ResolveCredentialResponse::Pending {
            pending_token: SECRET_CANARY.to_owned(),
            interaction: AcquisitionInteraction::FormPost {
                url: format!("https://provider.example/submit?state={SECRET_CANARY}"),
                fields: vec![FormPostField {
                    name: SECRET_CANARY.to_owned(),
                    value: SECRET_CANARY.to_owned(),
                }],
            },
        };

        let debug = format!("{response:?}");
        assert!(
            !debug.contains(SECRET_CANARY),
            "pending response Debug must not expose its token or interaction payload: {debug}"
        );
    }

    #[test]
    fn test_response_debug_redacts_message_payload() {
        let responses = [
            TestCredentialResponse::Success {
                message: SECRET_CANARY.to_owned(),
                tested_at: "2026-07-21T12:34:56Z".to_owned(),
            },
            TestCredentialResponse::Failed {
                code: CredentialTestFailureCodeV1::Other,
                message: SECRET_CANARY.to_owned(),
                tested_at: "2026-07-21T12:34:56Z".to_owned(),
            },
        ];

        for response in responses {
            let debug = format!("{response:?}");
            assert!(
                !debug.contains(SECRET_CANARY),
                "test response Debug must not expose its platform message: {debug}"
            );
        }
    }

    #[test]
    fn reconcile_decision_wire_spelling_matches_the_durable_port_spelling() {
        // Iterating the port's variants is the point: the wire spelling is not
        // an independent choice, it is the durable spelling. What catches a
        // *third* variant is the exhaustiveness of `from_port`, not this array.
        let port_variants = [
            CredentialOperationDecision::Refresh(RefreshOutcomeDecision::ProviderApplied),
            CredentialOperationDecision::Refresh(RefreshOutcomeDecision::ProviderNotApplied),
            CredentialOperationDecision::Revoke(RevokeOutcomeDecision::ProviderRevoked),
            CredentialOperationDecision::Revoke(RevokeOutcomeDecision::ProviderNotRevoked),
        ];

        for port in port_variants {
            let wire = serde_json::to_value(reconcile_decision_from_port(port))
                .expect("wire decision must serialize");
            assert_eq!(
                wire,
                serde_json::Value::String(port.as_str().to_owned()),
                "the wire spelling of {port:?} must be byte-identical to the durable \
                 adjudication spelling the port's as_str() writes to the adjudication_decision \
                 column: a drift either records a decision storage cannot decode or renames an \
                 already-persisted row"
            );
            assert_eq!(
                serde_json::from_value::<CredentialReconcileDecisionV1>(wire.clone())
                    .expect("every spelling this route emits must also decode"),
                reconcile_decision_from_port(port),
                "the wire vocabulary must round-trip, so a client can send back what it was told"
            );
        }
    }

    #[test]
    fn reconcile_request_without_operation_keeps_legacy_refresh_meaning() {
        let request: ReconcileCredentialRequest = serde_json::from_value(serde_json::json!({
            "incident": "7b0f3c1e-1f2a-4c55-9a51-0f1b6c0d2e41",
            "decision": "provider_applied",
            "evidence": "provider ticket"
        }))
        .expect("legacy refresh reconciliation request must remain accepted");

        assert_eq!(request.operation, CredentialReconcileOperationV1::Refresh);
        assert_eq!(
            reconcile_decision_to_port(request.decision, request.operation)
                .expect("legacy decision matches refresh"),
            CredentialOperationDecision::Refresh(RefreshOutcomeDecision::ProviderApplied)
        );
    }

    /// A request that names no incident cannot be attributed to one, so it is
    /// refused at the wire rather than resolving whatever is poisoned now.
    #[test]
    fn reconcile_request_without_an_incident_is_refused() {
        let refused = serde_json::from_value::<ReconcileCredentialRequest>(serde_json::json!({
            "operation": "refresh",
            "decision": "provider_applied",
            "evidence": "provider ticket"
        }));
        assert!(refused.is_err());
    }
}
