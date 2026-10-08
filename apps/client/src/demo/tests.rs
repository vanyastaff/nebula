use super::*;
use serde::{Serialize, de::DeserializeOwned};

fn demo() -> Demo {
    Demo::new().unwrap()
}

/// A value survives the wire unchanged: what the demo answers is exactly what the contract decodes.
fn round_trips<T: Serialize + DeserializeOwned>(value: &T) {
    let wire = serde_json::to_value(value).unwrap();
    let decoded: T = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&decoded).unwrap(), wire);
}

fn workflow_named(demo: &Demo, name: &str) -> WorkflowResponse {
    demo.list(1)
        .unwrap()
        .workflows
        .into_iter()
        .find(|workflow| workflow.name == name)
        .unwrap()
}

#[test]
fn the_seeded_workspace_answers_in_the_contract_s_own_shapes() {
    let demo = demo();
    let workflows = demo.list(1).unwrap();
    assert_eq!(workflows.total, 6);
    round_trips(&workflows);
    for workflow in &workflows.workflows {
        let document = demo.load(&workflow.id).unwrap();
        round_trips(&document);
        assert_eq!(document.definition["id"], workflow.id.as_str());
    }
    let history = demo
        .executions(&ExecutionQuery {
            limit: 100,
            ..ExecutionQuery::default()
        })
        .unwrap();
    assert!(history.items.len() >= 10);
    round_trips(&history);
    for run in &history.items {
        round_trips(&demo.status(&run.id).unwrap());
    }
    round_trips(&demo.actions().unwrap());
    round_trips(&demo.credential_types().unwrap());
    round_trips(&demo.credentials().unwrap());
    round_trips(&demo.tokens().unwrap());
    round_trips(&demo.org_members(ORG).unwrap());
    round_trips(&demo.workspace_members().unwrap());
    round_trips(&demo.me().unwrap());
}

#[test]
fn every_action_in_the_catalog_has_a_form() {
    let demo = demo();
    for action in demo.actions().unwrap().actions {
        let parameters = demo.action_parameters(&action.key).unwrap();
        let form = Form::parse(&parameters.parameters);
        assert!(!form.fields.is_empty(), "{} has fields", action.key);
    }
    assert_eq!(
        demo.action_parameters("core.missing").unwrap_err(),
        Failure::Rejected(404)
    );
}

#[test]
fn the_seeded_history_holds_completed_failed_and_cancelled_runs() {
    let demo = demo();
    let count = |statuses: &str| {
        demo.executions(&ExecutionQuery {
            statuses: statuses.to_owned(),
            limit: 100,
            ..ExecutionQuery::default()
        })
        .unwrap()
        .items
        .len()
    };
    assert!(count("completed") >= 8);
    assert_eq!(count("failed"), 1);
    assert_eq!(count("cancelled"), 1);
    assert_eq!(count("failed,cancelled"), 2);
}

#[test]
fn history_pages_follow_the_cursor_newest_first() {
    let demo = demo();
    let query = |cursor: Option<String>| ExecutionQuery {
        cursor,
        limit: 4,
        ..ExecutionQuery::default()
    };
    let first = demo.executions(&query(None)).unwrap();
    assert_eq!(first.items.len(), 4);
    assert!(first.has_more);
    let second = demo.executions(&query(first.next_cursor.clone())).unwrap();
    assert!(first.items[3].created_at >= second.items[0].created_at);
    assert_ne!(first.items[0].id, second.items[0].id);
    assert_eq!(
        demo.executions(&query(Some("bogus".into()))).unwrap_err(),
        Failure::Rejected(400)
    );
}

#[test]
fn saves_and_publications_are_fenced_by_revision() {
    let demo = demo();
    let workflow = workflow_named(&demo, "Lead dedupe");
    let document = demo.load(&workflow.id).unwrap();
    let request = |revision: u64| UpdateWorkflowDocumentRequest {
        update: nebula_api_contract::v1::workflow::UpdateWorkflowRequest {
            name: None,
            description: None,
            definition: Some(json!({"variables": {"region": "EU"}})),
        },
        expected_revision: Some(revision),
    };
    let saved = demo
        .save(&workflow.id, &request(document.revision))
        .unwrap();
    assert_eq!(saved.revision, document.revision + 1);
    // The patch merges its top-level keys; the graph stays.
    assert_eq!(saved.definition["variables"]["region"], "EU");
    assert_eq!(saved.definition["nodes"], document.definition["nodes"]);
    assert_eq!(
        demo.save(&workflow.id, &request(document.revision))
            .unwrap_err(),
        Failure::Conflict
    );
    assert_eq!(
        demo.publish(&workflow.id, document.revision).unwrap_err(),
        Failure::Conflict
    );
    let published = demo.publish(&workflow.id, saved.revision).unwrap();
    assert_eq!(published.revision, saved.revision + 1);
}

#[test]
fn publication_reports_what_the_graph_is_missing() {
    let demo = demo();
    let draft = workflow_named(&demo, "Customer sync (draft)");
    let revision = demo.load(&draft.id).unwrap().revision;
    let Failure::Invalid(message) = demo.publish(&draft.id, revision).unwrap_err() else {
        panic!("a validation failure");
    };
    assert!(
        message.contains("nodes[0].parameters.keys: Provide a value."),
        "{message}"
    );
    // A draft was never published, so it cannot run either.
    assert!(matches!(
        demo.run(&draft.id, "k1"),
        Err(Failure::Invalid(_))
    ));
}

#[test]
fn a_start_is_idempotent_per_key_and_its_run_advances() {
    let demo = demo();
    let workflow = workflow_named(&demo, "Order fulfillment");
    let first = demo.run(&workflow.id, "start-1").unwrap();
    let again = demo.run(&workflow.id, "start-1").unwrap();
    assert_eq!(first.id, again.id);
    let other = demo.run(&workflow.id, "start-2").unwrap();
    assert_ne!(first.id, other.id);
    let detail = demo.status(&first.id).unwrap();
    assert_eq!(detail.execution.workflow_id, workflow.id);
    assert_ne!(
        detail.execution.status,
        nebula_api_contract::v1::execution::ExecutionStatus::Completed
    );
    let cancelled = demo.cancel(&first.id).unwrap();
    assert_eq!(cancelled.status, "cancelling");
    assert!(matches!(demo.cancel(&first.id), Err(Failure::Invalid(_))));
}

#[test]
fn credentials_are_checked_against_their_type_schema() {
    let demo = demo();
    let create = |key: &str, data: Value| {
        demo.create_credential(&CreateCredentialRequest {
            credential_key: key.to_owned(),
            name: "Test".to_owned(),
            description: None,
            data,
            tags: None,
        })
    };
    let Failure::Invalid(message) = create("basic_auth", json!({"username": "ops"})).unwrap_err()
    else {
        panic!("a validation failure");
    };
    assert_eq!(message, "data.password: Provide a value.");
    let created = create(
        "basic_auth",
        json!({"username": "ops", "password": "s3cret"}),
    )
    .unwrap();
    assert_eq!(created.auth_pattern, "IdentityPassword");
    assert!(
        demo.credentials()
            .unwrap()
            .credentials
            .iter()
            .any(|c| c.id == created.id)
    );
    // A union holds one of its tags.
    assert!(matches!(
        create("oauth2", json!({})),
        Err(Failure::Invalid(_))
    ));
    assert!(
        create(
            "oauth2",
            json!({"client_credentials": {"token_url": "https://id.example/token"}})
        )
        .is_ok()
    );
    assert!(matches!(
        create("nope", json!({})),
        Err(Failure::Invalid(_))
    ));

    assert!(matches!(
        demo.test_credential(&created.id).unwrap(),
        TestCredentialResponse::Success { .. }
    ));
    demo.delete_credential(&created.id).unwrap();
    assert_eq!(
        demo.delete_credential(&created.id).unwrap_err(),
        Failure::Rejected(404)
    );
}

#[test]
fn a_credential_that_needs_reauthorization_fails_its_test() {
    let demo = demo();
    let sheets = demo
        .credentials()
        .unwrap()
        .credentials
        .into_iter()
        .find(|credential| credential.lifecycle == CredentialLifecycleState::ReauthRequired)
        .unwrap();
    assert!(matches!(
        demo.test_credential(&sheets.id).unwrap(),
        TestCredentialResponse::Failed {
            code: CredentialTestFailureCodeV1::AuthenticationRejected,
            ..
        }
    ));
}

#[test]
fn a_webhook_registers_only_for_a_bound_trigger() {
    let demo = demo();
    let invoices = workflow_named(&demo, "Invoice events");
    let request = |trigger: &str| RegisterWebhookRequest {
        workflow_id: invoices.id.clone(),
        trigger_id: trigger.to_owned(),
        provider: "generic".to_owned(),
        replay_window_secs: None,
        timestamp_header: None,
        provider_config: None,
        rate_limit_per_minute: None,
    };
    let registered = demo.register_webhook(&request("billing_webhook")).unwrap();
    assert!(registered.webhook_url.starts_with("https://"));
    assert!(registered.signing_secret.starts_with("whsec_"));
    assert!(matches!(
        demo.register_webhook(&request("nope")),
        Err(Failure::Invalid(_))
    ));
}

#[test]
fn tokens_and_members_follow_the_server_s_rules() {
    let demo = demo();
    let created = demo
        .create_token(&CreateTokenRequest {
            name: "Laptop".into(),
            scopes: vec!["workflows:read".into()],
            ttl_seconds: Some(3600),
        })
        .unwrap();
    assert!(created.token.starts_with("nbl_pat_"));
    assert_eq!(demo.me().unwrap().tokens_count, 3);
    demo.revoke_token(&created.summary.id).unwrap();
    assert_eq!(
        demo.revoke_token(&created.summary.id).unwrap_err(),
        Failure::Rejected(404)
    );

    let add = |principal: &str, role: &str| {
        demo.add_org_member(
            ORG,
            &AddMemberRequest {
                principal_id: principal.to_owned(),
                role: OrgRoleDto(role.to_owned()),
            },
        )
    };
    assert!(matches!(add("ada", "member"), Err(Failure::Invalid(_))));
    assert!(matches!(
        add("usr_01HZ8K7E5F7G9H1J3K5M7N9P1Q", "emperor"),
        Err(Failure::Invalid(_))
    ));
    add("usr_01HZ8K7E5F7G9H1J3K5M7N9P1Q", "member").unwrap();
    assert_eq!(
        add("usr_01HZ8K7E5F7G9H1J3K5M7N9P1Q", "member").unwrap_err(),
        Failure::Conflict
    );
    assert!(matches!(
        demo.remove_org_member(ORG, seed::ME),
        Err(Failure::Invalid(_))
    ));
    assert_eq!(
        demo.org_members("elsewhere").unwrap_err(),
        Failure::Forbidden
    );

    let member = demo
        .set_workspace_member(
            "usr_01HZ8K7E5F7G9H1J3K5M7N9P1Q",
            &UpsertWorkspaceMemberRequest {
                role: nebula_api_contract::v1::shared::WorkspaceRoleDto("runner".into()),
            },
        )
        .unwrap();
    assert_eq!(member.role.0, "runner");
    demo.remove_workspace_member("usr_01HZ8K7E5F7G9H1J3K5M7N9P1Q")
        .unwrap();
}

#[test]
fn identities_look_and_sort_like_the_server_s() {
    let demo = demo();
    let workflow = demo
        .create(&CreateWorkflowRequest {
            name: "Fresh".into(),
            description: None,
            definition: json!({"nodes": [], "connections": []}),
        })
        .unwrap();
    let id = &workflow.workflow.id;
    assert!(id.starts_with("wf_") && id.len() == 29, "{id}");
    assert!(id[3..].bytes().all(|byte| CROCKFORD.contains(&byte)));
    assert_eq!(workflow.revision, 1);
}
