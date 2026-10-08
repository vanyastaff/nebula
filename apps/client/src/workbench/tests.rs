use super::*;
use crate::{document::tests::snapshot, transport::SignedIn};
use nebula_api_contract::v1::{catalog::ListActionsResponse, workflow::ListWorkflowsResponse};
use serde_json::json;

const SERVER: &str = "http://127.0.0.1:8080";

fn fixture_profile() -> MeResponse {
    serde_json::from_value(json!({
        "user_id": "user_fixture",
        "email": "fixture@example.test",
        "display_name": "Fixture",
        "email_verified": true,
        "mfa_enabled": false,
        "tokens_count": 1
    }))
    .unwrap()
}

fn open_workspace_session(workbench: &mut Workbench) {
    workbench.session.switch(Some(SessionContext {
        endpoint: "https://one.test/".into(),
        principal: "usr_one".into(),
        organization: "org".into(),
        workspace_selector: "ws".into(),
    }));
}

#[test]
fn created_workflow_opens_in_the_editor_and_refreshes_the_list() {
    let mut workbench = Workbench::new(String::new());
    open_workspace_session(&mut workbench);
    workbench.navigator.new_name = "Echo".into();
    let stamp = workbench.session.begin().unwrap();
    let created = snapshot(1, 7);

    workbench.receive(stamp, RequestKind::Create, Ok(Reply::Created(created)));

    assert!(workbench.session.draft().is_some());
    assert!(workbench.navigator.new_name.is_empty());
    assert!(workbench.navigator.take_refresh());
    assert!(!workbench.navigator.take_refresh());
}

#[test]
fn the_list_refresh_after_creation_keeps_the_creation_message() {
    let mut workbench = Workbench::new(String::new());
    open_workspace_session(&mut workbench);
    let stamp = workbench.session.begin().unwrap();
    workbench.receive(
        stamp,
        RequestKind::Create,
        Ok(Reply::Created(snapshot(1, 7))),
    );
    let listed: ListWorkflowsResponse =
        serde_json::from_value(json!({"workflows": [], "total": 1, "page": 1, "page_size": 25}))
            .unwrap();
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(stamp, RequestKind::Read, Ok(Reply::Listed(listed)));

    assert_eq!(
        workbench.feedback.message,
        "Workflow created with an empty graph."
    );
    assert!(!workbench.feedback.failure);
}

#[test]
fn reply_from_a_superseded_session_changes_nothing() {
    let mut workbench = Workbench::new(String::new());
    open_workspace_session(&mut workbench);
    let stamp = workbench.session.begin().unwrap();
    workbench.session.switch(None);
    let listed: ListWorkflowsResponse =
        serde_json::from_value(json!({"workflows": [], "total": 3, "page": 1, "page_size": 25}))
            .unwrap();

    workbench.receive(stamp, RequestKind::Read, Ok(Reply::Listed(listed)));

    assert_eq!(workbench.navigator.total, 0);
    assert!(!workbench.feedback.failure);
}

#[test]
fn unauthorized_reply_ends_the_session_and_keeps_the_reason() {
    let mut workbench = Workbench::new(String::new());
    open_workspace_session(&mut workbench);
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(stamp, RequestKind::Read, Err(Failure::Unauthorized));

    assert!(!workbench.workspace_open());
    assert!(workbench.feedback.failure);
    assert_eq!(
        workbench.feedback.message,
        Failure::Unauthorized.to_string()
    );
}

#[test]
fn server_version_waits_for_review_when_the_draft_has_unsaved_edits() {
    let mut workbench = Workbench::new(String::new());
    open_workspace_session(&mut workbench);
    let context = workbench.session.context.clone().unwrap();
    let key = DraftKey {
        context,
        workflow: "wf_test".into(),
    };
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft.edit("echo", "message", "9").unwrap();
    workbench.session.drafts.insert(key.clone(), draft);
    workbench.session.selected = Some(key.clone());
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(stamp, RequestKind::Read, Ok(Reply::Loaded(snapshot(2, 8))));

    let draft = &workbench.session.drafts[&key];
    assert_eq!(draft.base.revision, 1);
    assert_eq!(draft.remote.as_ref().unwrap().revision, 2);
    assert!(draft.dirty());
}

#[test]
fn conflict_marks_the_draft_for_review_without_discarding_it() {
    let mut workbench = Workbench::new(String::new());
    open_workspace_session(&mut workbench);
    let context = workbench.session.context.clone().unwrap();
    let key = DraftKey {
        context,
        workflow: "wf_test".into(),
    };
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    draft.edit("echo", "message", "9").unwrap();
    draft.uncertain_save = true;
    workbench.session.drafts.insert(key.clone(), draft);
    workbench.session.selected = Some(key.clone());
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(stamp, RequestKind::Save, Err(Failure::Conflict));

    let draft = &workbench.session.drafts[&key];
    assert!(draft.save_conflict);
    assert!(!draft.uncertain_save);
    assert!(draft.dirty());
    assert_eq!(
        workbench.session.draft().map(draft_gate),
        Some(DraftGate {
            can_save: false,
            can_publish: false,
            can_run: false,
        })
    );
}

#[test]
fn a_publish_remembers_the_revision_it_made_live() {
    let mut workbench = Workbench::new(String::new());
    open_workspace_session(&mut workbench);
    let context = workbench.session.context.clone().unwrap();
    let key = DraftKey {
        context,
        workflow: "wf_test".into(),
    };
    workbench
        .session
        .drafts
        .insert(key.clone(), Draft::new(snapshot(1, 7)).unwrap());
    workbench.session.selected = Some(key.clone());
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(
        stamp,
        RequestKind::Publish,
        Ok(Reply::Published(snapshot(2, 7))),
    );

    let draft = &workbench.session.drafts[&key];
    assert_eq!(draft.published_revision, Some(2));
    assert_eq!(draft.base.revision, 2);
}

#[test]
fn a_catalog_without_a_registry_is_a_state_and_says_so() {
    let mut workbench = Workbench::new(String::new());
    open_workspace_session(&mut workbench);
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(stamp, RequestKind::Catalog, Err(Failure::Rejected(503)));

    assert!(matches!(workbench.catalog, Catalog::Unavailable));
    assert_eq!(workbench.feedback.message, CATALOG_UNAVAILABLE);
    assert!(!workbench.feedback.failure);
}

#[test]
fn a_published_catalog_is_kept_for_the_add_node_form() {
    let mut workbench = Workbench::new(String::new());
    open_workspace_session(&mut workbench);
    let stamp = workbench.session.begin().unwrap();
    let catalog: ListActionsResponse = serde_json::from_value(json!({
        "actions": [{"key": "json_transform", "name": "JSON transform", "version": "1.0"}]
    }))
    .unwrap();

    workbench.receive(stamp, RequestKind::Catalog, Ok(Reply::Actions(catalog)));

    match &workbench.catalog {
        Catalog::Ready(actions) => assert_eq!(actions[0].key, "json_transform"),
        _ => panic!("the catalog should be ready"),
    }
}

#[test]
fn a_blank_slug_never_opens_a_workspace() {
    let mut workbench = Workbench::new(String::new());
    workbench.begin_sign_in(Connection::new(SERVER).unwrap());
    workbench.profile = Some(fixture_profile());
    workbench.form.organization = "personal".into();
    workbench.form.workspace = "   ".into();

    assert!(!workbench.open_workspace());
    assert!(!workbench.workspace_open());
}

#[test]
fn sign_in_waits_for_the_server_address_and_the_chosen_credentials() {
    let mut form = ConnectionForm::new(SERVER.into());
    assert!(!form.can_sign_in());
    form.email = "fixture@example.test".into();
    assert!(!form.can_sign_in());
    form.password = "fixture-secret".into();
    assert!(form.can_sign_in());
    form.endpoint = "   ".into();
    assert!(!form.can_sign_in());

    form.endpoint = SERVER.into();
    form.set_mode(SignInMode::Token);
    assert!(!form.can_sign_in());
    form.token = "nbt_fixture".into();
    assert!(form.can_sign_in());
}

#[test]
fn a_workspace_needs_both_slugs() {
    let mut form = ConnectionForm::new(String::new());
    form.organization = "personal".into();
    assert!(!form.can_open_workspace());
    form.workspace = "  ".into();
    assert!(!form.can_open_workspace());
    form.workspace = "main".into();
    assert!(form.can_open_workspace());
}

#[test]
fn a_second_factor_request_keeps_the_password_and_asks_for_a_code() {
    let mut workbench = Workbench::new(SERVER.into());
    workbench.form.email = "fixture@example.test".into();
    workbench.form.password = "fixture-secret".into();
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(stamp, RequestKind::Connect, Err(Failure::MfaRequired));

    assert!(workbench.form.mfa_required);
    assert_eq!(workbench.form.password, "fixture-secret");
    assert!(!workbench.form.can_sign_in());
    assert!(!workbench.feedback.failure);
    workbench.form.totp = "123456".into();
    assert!(workbench.form.can_sign_in());
}

#[test]
fn a_rejected_sign_in_wipes_the_password_and_says_why() {
    let mut workbench = Workbench::new(SERVER.into());
    workbench.form.email = "fixture@example.test".into();
    workbench.form.password = "fixture-secret".into();
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(stamp, RequestKind::Connect, Err(Failure::Unauthorized));

    assert!(workbench.form.password.is_empty());
    assert!(!workbench.form.mfa_required);
    assert!(workbench.feedback.failure);
    assert_eq!(
        workbench.feedback.message,
        Failure::Unauthorized.to_string()
    );
}

#[test]
fn a_completed_sign_in_clears_every_secret() {
    let mut workbench = Workbench::new(SERVER.into());
    workbench.form.email = "fixture@example.test".into();
    workbench.form.password = "fixture-secret".into();
    let stamp = workbench.session.begin().unwrap();
    workbench.receive(stamp, RequestKind::Connect, Err(Failure::MfaRequired));
    workbench.form.totp = "123456".into();
    let connection = Connection::new(SERVER).unwrap();
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(
        stamp,
        RequestKind::Connect,
        Ok(Reply::Connected(SignedIn {
            connection,
            profile: fixture_profile(),
        })),
    );

    assert!(workbench.is_signed_in());
    assert!(workbench.form.password.is_empty());
    assert!(workbench.form.totp.is_empty());
    assert!(!workbench.form.mfa_required);
}

#[test]
fn a_repeated_message_is_a_new_toast_and_dismissing_clears_it() {
    let mut feedback = Feedback::default();
    feedback.info("Workflow loaded.");
    let first = feedback.serial;
    feedback.info("Workflow loaded.");
    assert!(feedback.serial > first);

    feedback.error("Server unreachable.");
    feedback.dismiss();
    assert!(feedback.message.is_empty());
    assert!(!feedback.failure);
}

#[test]
fn choosing_a_workflow_hands_the_narrow_page_back_to_the_editor() {
    let mut workbench = Workbench::new(SERVER.into());
    open_workspace_session(&mut workbench);
    workbench.sidebar_open = true;

    assert!(workbench.select_workflow("wf_test"));

    assert!(!workbench.sidebar_open);
}

#[test]
fn switching_modes_wipes_the_secrets_of_the_mode_being_left() {
    let mut form = ConnectionForm::new(SERVER.into());
    form.password = "fixture-secret".into();
    form.require_second_factor();
    form.totp = "123456".into();

    form.set_mode(SignInMode::Token);

    assert!(form.password.is_empty());
    assert!(form.totp.is_empty());
    assert!(!form.mfa_required);
    form.token = "nbt_fixture".into();
    form.set_mode(SignInMode::Password);
    assert!(form.token.is_empty());
}

#[test]
fn unsaved_edits_allow_saving_but_not_running_the_published_version() {
    let mut draft = Draft::new(snapshot(1, 7)).unwrap();
    assert_eq!(
        draft_gate(&draft),
        DraftGate {
            can_save: false,
            can_publish: true,
            can_run: true,
        }
    );
    draft.edit("echo", "message", "9").unwrap();
    assert_eq!(
        draft_gate(&draft),
        DraftGate {
            can_save: true,
            can_publish: false,
            can_run: false,
        }
    );
}
