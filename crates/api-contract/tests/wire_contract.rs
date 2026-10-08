use nebula_api_contract::v1::{
    auth::{LoginRequest, LoginResponse, OAuthCallbackParams, OAuthProvider},
    catalog::ActionSummary,
    credential::{
        CredentialLifecycleState, CredentialReconcileDecisionV1, CredentialReconcileOperationV1,
        CredentialResponse, ReconcileCredentialRequest, ReconcileCredentialResponse,
        TestCredentialResponse,
    },
    execution::{ExecutionDetailResponse, ExecutionResponse, StartExecutionRequest},
    problem::ProblemDetails,
    shared::{PaginatedResponse, PaginationParams},
    webhook::{RegisterWebhookRequest, RegisterWebhookResponse},
    workflow::UpdateWorkflowRequest,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

fn round_trip<T: Serialize + DeserializeOwned>(wire: Value) {
    let decoded: T = serde_json::from_value(wire.clone()).expect("wire body decodes");
    assert_eq!(
        serde_json::to_value(decoded).expect("wire body encodes"),
        wire
    );
}

#[test]
fn shared_transport_supports_both_endpoints_without_schema_traits() {
    round_trip::<StartExecutionRequest>(json!({"input":{"answer":42}}));
    round_trip::<ExecutionResponse>(json!({
        "id":"exe_01", "workflow_id":"wf_01", "status":"running", "started_at":1
    }));
    round_trip::<ProblemDetails>(json!({
        "type":"https://nebula.dev/problems/credential-operation-blocked",
        "title":"Credential operation blocked", "status":409, "operation":"refresh"
    }));
    round_trip::<RegisterWebhookRequest>(json!({
        "workflow_id":"wf_01", "trigger_id":"incoming", "provider":"generic",
        "replay_window_secs":null, "timestamp_header":null,
        "provider_config":null, "rate_limit_per_minute":null
    }));
    round_trip::<RegisterWebhookResponse>(json!({
        "webhook_url":"https://example.invalid/incoming", "signing_secret":"whsec_transit",
        "activation_id":"activation_01"
    }));
    let page = PaginatedResponse::last_page(vec![json!({"id":"exe_01"})]);
    assert_eq!(
        serde_json::to_value(page).unwrap(),
        json!({
            "items":[{"id":"exe_01"}], "has_more":false
        })
    );
    let pagination: PaginationParams = serde_json::from_value(json!({})).unwrap();
    assert_eq!(
        serde_json::to_value(pagination).unwrap(),
        json!({"page":1,"page_size":10})
    );
}

#[test]
fn execution_inspection_round_trips_optional_times_and_inline_null() {
    round_trip::<ExecutionDetailResponse>(json!({
        "id":"exe_01", "workflow_id":"wf_01", "status":"created",
        "created_at":"2026-10-07T00:00:00.000000Z",
        "updated_at":"2026-10-07T00:00:00.000000Z",
        "snapshot_version":0, "total_retries":0, "total_output_bytes":4,
        "nodes":{"step":{
            "status":"pending", "attempts":[],
            "output":{"type":"inline","value":null}
        }}
    }));
}

#[test]
fn responses_omitting_empty_collections_decode_from_their_own_output() {
    // The server skips empty `tags`; a client must still decode that body.
    round_trip::<CredentialResponse>(json!({
        "id":"cred_01", "credential_key":"api_key", "name":"Untagged",
        "auth_pattern":"SecretToken",
        "capabilities":{"interactive":false,"refreshable":false,"testable":true,"revocable":false},
        "created_at":"2026-10-05T00:00:00Z", "updated_at":"2026-10-05T00:00:00Z",
        "version":1, "lifecycle":{"status":"ready"}
    }));
    // A problem body without extension members must not decode as `Some({})`.
    let problem: ProblemDetails = serde_json::from_value(json!({
        "type":"about:blank", "title":"Not Found", "status":404
    }))
    .expect("plain problem decodes");
    assert!(problem.extensions.is_none());
}

#[test]
fn credential_v1_preserves_incident_identity_and_frozen_unions() {
    let incident = "00000000-0000-4000-8000-000000000001";
    round_trip::<CredentialLifecycleState>(json!({
        "status":"reconciliation_required", "operation":"revoke", "incident":incident
    }));
    round_trip::<ReconcileCredentialRequest>(json!({
        "operation":"revoke", "incident":incident,
        "decision":"provider_revoked", "evidence":"operator observed revocation"
    }));
    round_trip::<ReconcileCredentialResponse>(json!({
        "operation":"revoke", "incident":incident, "decision":"provider_revoked",
        "changed":true, "evidence_digest":"0123456789abcdef", "message":"recorded"
    }));
    let legacy: ReconcileCredentialRequest = serde_json::from_value(json!({
        "incident":incident, "decision":"provider_not_applied", "evidence":"observed"
    }))
    .unwrap();
    assert_eq!(legacy.operation, CredentialReconcileOperationV1::Refresh);
    assert_eq!(
        legacy.decision,
        CredentialReconcileDecisionV1::ProviderNotApplied
    );
    assert!(
        serde_json::from_value::<ReconcileCredentialRequest>(json!({
            "incident":"not-a-uuid", "decision":"provider_applied", "evidence":"observed"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<CredentialLifecycleState>(json!({"status":"unknown"})).is_err()
    );
    assert!(serde_json::from_value::<TestCredentialResponse>(json!({"status":"unknown"})).is_err());
}

#[test]
fn client_construction_and_decoding_keep_auth_authority_redacted() {
    let request: LoginRequest = serde_json::from_value(json!({
        "email":"person@example.invalid", "password":"secret_canary_password", "remember_me":false
    }))
    .unwrap();
    assert!(!format!("{request:?}").contains("secret_canary_password"));
    assert_eq!(
        serde_json::to_value(request).unwrap()["password"],
        "secret_canary_password"
    );
    let response: LoginResponse = serde_json::from_value(json!({
        "user": {"user_id":"user_01", "email":"person@example.invalid", "display_name":"User",
            "email_verified":true, "mfa_enabled":false},
        "csrf_token":"secret_canary_csrf"
    }))
    .unwrap();
    assert!(!format!("{response:?}").contains("secret_canary_csrf"));
    assert_eq!(
        serde_json::to_value(response).unwrap()["csrf_token"],
        "secret_canary_csrf"
    );
    round_trip::<OAuthCallbackParams>(json!({"state":"state", "code":"code", "error":null}));
    assert_eq!(
        serde_json::to_value(OAuthProvider::GitHub).unwrap(),
        "github"
    );
    assert!(
        "unknown_canary_provider"
            .parse::<OAuthProvider>()
            .unwrap_err()
            .to_string()
            .contains("unknown OAuth provider")
    );
}

#[test]
fn workflow_update_keeps_absence_distinct_from_explicit_null() {
    let absent: UpdateWorkflowRequest = serde_json::from_value(json!({})).unwrap();
    let explicit_null: UpdateWorkflowRequest =
        serde_json::from_value(json!({"definition":null})).unwrap();
    assert!(absent.definition.is_none());
    assert_eq!(explicit_null.definition, Some(Value::Null));
}

#[test]
fn an_action_summary_carries_its_kind_and_reads_one_without_it() {
    round_trip::<ActionSummary>(json!({
        "key": "core.if", "name": "If", "version": "2.0.0", "kind": "control"
    }));
    // A server that predates the field still decodes, and its actions are not refused.
    let older: ActionSummary =
        serde_json::from_value(json!({"key": "core.map", "name": "Map", "version": "2.0.0"}))
            .expect("summary without kind decodes");
    assert!(older.kind.is_none());
    assert!(older.is_graph_node());

    let kind = |kind: &str| ActionSummary {
        key: "plugin.action".into(),
        name: "Action".into(),
        version: "1.0.0".into(),
        kind: Some(kind.into()),
    };
    for graph in ["stateless", "stateful", "control", "agent"] {
        assert!(kind(graph).is_graph_node(), "{graph}");
    }
    for other in ["trigger", "stream", "resource", "interactive"] {
        assert!(!kind(other).is_graph_node(), "{other}");
    }
}
