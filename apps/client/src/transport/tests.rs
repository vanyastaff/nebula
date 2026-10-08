use super::*;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, header, method, path},
};

/// Uses only an isolated, operator-enrolled test deployment; never a production account.
#[tokio::test]
#[ignore = "requires isolated SQLite server; see apps/client/README.md"]
async fn live_existing_server_edit_conflict_publish_run_and_persisted_output() {
    use crate::document::Draft;
    use nebula_api_contract::v1::{
        auth::SecretString,
        execution::{ExecutionNodeOutput, ExecutionStatus},
        workflow::CreateWorkflowRequest,
    };
    let endpoint =
        std::env::var("NEBULA_CLIENT_TEST_ENDPOINT").expect("isolated test endpoint required");
    let signed_in = Connection::new(&endpoint)
        .unwrap()
        .sign_in(SignIn::Password(LoginRequest {
            email: "client-acceptance@example.test".into(),
            password: SecretString::new("isolated-client-acceptance-password".into()),
            totp: None,
        }))
        .await
        .unwrap();
    let connection = signed_in.connection;
    let name = format!("Client acceptance {}", uuid::Uuid::new_v4());
    let created: WorkflowResponse = connection
        .write(
            "POST",
            connection
                .url(&["orgs", "personal", "workspaces", "default", "workflows"])
                .unwrap(),
            &CreateWorkflowRequest {
                name: name.clone(),
                description: None,
                definition: serde_json::json!({
                    "nodes":[{"id":"transform","name":"Transform","plugin_key":"core","action_key":"json_transform","parameters":{
                        "data":{"type":"literal","value":{"value":1}},
                        "operations":{"type":"literal","value":[]}
                    }}],"connections":[]
                }),
            },
            None,
            &[201],
        )
        .await
        .unwrap();
    let listed = connection.list("personal", "default", 1).await.unwrap();
    assert!(
        listed
            .workflows
            .iter()
            .any(|workflow| workflow.id == created.id)
    );
    let loaded = connection
        .load("personal", "default", &created.id)
        .await
        .unwrap();
    let mut draft = Draft::new(loaded).unwrap();
    draft.edit("transform", "data", r#"{"value":2}"#).unwrap();
    let mut competing = draft.save_request();
    competing.update.definition.as_mut().unwrap()["nodes"][0]["parameters"]["data"]["value"] =
        serde_json::json!({"value":3});
    connection
        .save("personal", "default", &created.id, &competing)
        .await
        .unwrap();
    assert_eq!(
        connection
            .save("personal", "default", &created.id, &draft.save_request())
            .await
            .unwrap_err(),
        Failure::Conflict
    );
    assert!(draft.dirty());
    draft.remote = Some(
        connection
            .load("personal", "default", &created.id)
            .await
            .unwrap(),
    );
    draft.reapply().unwrap();
    let saved = connection
        .save("personal", "default", &created.id, &draft.save_request())
        .await
        .unwrap();
    assert_eq!(saved.revision, 3);
    let published = connection
        .publish("personal", "default", &created.id, saved.revision)
        .await
        .unwrap();
    assert_eq!(
        published.definition["nodes"][0]["parameters"]["data"]["value"],
        serde_json::json!({"value":2})
    );
    let key = uuid::Uuid::new_v4().to_string();
    let receipt = connection
        .run("personal", "default", &created.id, &key)
        .await
        .unwrap();
    let replay = connection
        .run("personal", "default", &created.id, &key)
        .await
        .unwrap();
    assert_eq!(receipt.id, replay.id);
    let status = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let status = connection
                .status("personal", "default", &receipt.id)
                .await
                .unwrap();
            if matches!(
                status.execution.status,
                ExecutionStatus::Completed | ExecutionStatus::Failed
            ) {
                break status;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("worker must reach persisted terminal status");
    assert_eq!(status.execution.status, ExecutionStatus::Completed);
    assert!(
        matches!(&status.nodes["transform"].output, Some(ExecutionNodeOutput::Inline { value }) if value == &serde_json::json!({"value":2}))
    );
    let history = connection
        .history("personal", "default", &created.id)
        .await
        .unwrap();
    assert_eq!(history.items.len(), 1);
    assert_eq!(history.items[0].id, receipt.id);
}

#[test]
fn endpoints_cannot_smuggle_authority_or_use_cleartext_remote_transport() {
    for url in [
        "http://remote.test",
        "https://user:secret@example.test",
        "https://example.test/?token=secret",
        "https://example.test/#secret",
    ] {
        assert!(Connection::new(url).is_err());
    }
    let connection = Connection::new("http://127.0.0.1:8000/prefix").unwrap();
    assert_eq!(
        connection
            .url(&["orgs", "a/b", "workspaces", "ws"])
            .unwrap()
            .as_str(),
        "http://127.0.0.1:8000/prefix/api/v1/orgs/a%2Fb/workspaces/ws"
    );
}

#[tokio::test]
async fn token_sign_in_probes_root_version_then_authenticates_the_profile() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/version"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"name":"nebula","version":"0.33.0"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/me"))
        .and(header("authorization", "Bearer isolated-fixture-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "user_id":"user_fixture", "email":"fixture@example.test",
            "display_name":"Fixture", "email_verified":true,
            "mfa_enabled":false, "tokens_count":1
        })))
        .expect(1)
        .mount(&server)
        .await;
    let signed_in = Connection::new(&server.uri())
        .unwrap()
        .sign_in(SignIn::Token(Zeroizing::new(
            "isolated-fixture-token".into(),
        )))
        .await
        .unwrap();
    assert_eq!(signed_in.profile.user_id, "user_fixture");
}

#[tokio::test]
async fn mutation_redirect_never_reaches_a_second_destination_and_is_uncertain() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(
            "/api/v1/orgs/org/workspaces/ws/workflows/wf/executions",
        ))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("location", format!("{}/redirected", server.uri())),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/redirected"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let result = Connection::new(&server.uri())
        .unwrap()
        .run("org", "ws", "wf", "one-intent")
        .await;
    assert_eq!(result.unwrap_err(), Failure::OutcomeUnknown);
}

#[tokio::test]
async fn uncertain_start_reuses_the_same_key_only_on_explicit_reconciliation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(header("idempotency-key", "same-intent"))
        .respond_with(ResponseTemplate::new(202).set_body_string("truncated receipt"))
        .expect(1)
        .mount(&server)
        .await;
    let connection = Connection::new(&server.uri()).unwrap();
    assert_eq!(
        connection
            .run("org", "ws", "wf", "same-intent")
            .await
            .unwrap_err(),
        Failure::OutcomeUnknown
    );
    server.verify().await;
    server.reset().await;
    Mock::given(method("POST"))
        .and(header("idempotency-key", "same-intent"))
        .respond_with(ResponseTemplate::new(202).set_body_json(
            serde_json::json!({"id":"exe_one","workflow_id":"wf","status":"Created","started_at":0}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    assert_eq!(
        connection
            .run("org", "ws", "wf", "same-intent")
            .await
            .unwrap()
            .id,
        "exe_one"
    );
}

#[tokio::test]
async fn create_posts_a_blank_graph_then_reads_back_the_editable_document() {
    use crate::document::{Draft, new_workflow_request};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/orgs/org/workspaces/ws/workflows"))
        .and(body_partial_json(serde_json::json!({
            "name": "Blank",
            "definition": {"nodes": [], "connections": []}
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(
            serde_json::json!({"id":"wf_blank","name":"Blank","created_at":0,"updated_at":0}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/orgs/org/workspaces/ws/workflows/wf_blank"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id":"wf_blank","name":"Blank","created_at":0,"updated_at":0,"revision":1,
            "definition":{"id":"wf_blank","nodes":[],"connections":[]}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let document = Connection::new(&server.uri())
        .unwrap()
        .create("org", "ws", &new_workflow_request("Blank"))
        .await
        .unwrap();
    assert_eq!(document.workflow.id, "wf_blank");
    assert_eq!(document.revision, 1);
    assert!(Draft::new(document).is_ok());
    server.verify().await;
}

#[tokio::test]
async fn created_workflow_that_cannot_be_read_back_is_an_uncertain_write() {
    use crate::document::new_workflow_request;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/orgs/org/workspaces/ws/workflows"))
        .respond_with(ResponseTemplate::new(201).set_body_json(
            serde_json::json!({"id":"wf_lost","name":"Lost","created_at":0,"updated_at":0}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/orgs/org/workspaces/ws/workflows/wf_lost"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    assert_eq!(
        Connection::new(&server.uri())
            .unwrap()
            .create("org", "ws", &new_workflow_request("Lost"))
            .await
            .unwrap_err(),
        Failure::OutcomeUnknown
    );
    server.verify().await;
}

#[test]
fn invalid_workflow_problem_names_the_path_and_remediation_but_not_free_form_detail() {
    let body = serde_json::json!({"errors": [{
        "path": "/nodes/http_request/action_key",
        "remediation": "register the exact namespaced action",
        "detail": "provider text that must stay out"
    }]})
    .to_string();
    let message = invalid_workflow_message(body.as_bytes());
    assert!(
        message.contains("/nodes/http_request/action_key: register the exact namespaced action")
    );
    assert!(!message.contains("provider text"));
    assert_eq!(
        invalid_workflow_message(b"not json"),
        "The server rejected this workflow."
    );
}

#[tokio::test]
async fn old_servers_without_document_revision_are_explicitly_unsupported() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"id":"wf","name":"Old server","created_at":0,"updated_at":0}),
        ))
        .mount(&server)
        .await;
    assert_eq!(
        Connection::new(&server.uri())
            .unwrap()
            .load("org", "ws", "wf")
            .await
            .unwrap_err(),
        Failure::Unsupported
    );
}
