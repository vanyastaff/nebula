use super::*;
use crate::document::tests::snapshot;
use nebula_api_contract::v1::workflow::ListWorkflowsResponse;
use serde_json::json;

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
