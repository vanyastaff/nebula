//! The workspace pages' reducers and the execution watch, fed with what the demo answers.

use super::*;
use crate::{
    demo::Demo,
    effects::{Reply, RequestKind, Target},
    session::SessionContext,
    transport::{ExecutionQuery, Failure},
};
use nebula_api_contract::v1::execution::{
    ExecutionDetailResponse, ExecutionStatus, ListExecutionsResponse,
};

fn workspace() -> Workbench {
    let mut workbench = Workbench::new(String::new());
    workbench.session.switch(Some(SessionContext {
        endpoint: "https://one.test/".into(),
        principal: "usr_one".into(),
        organization: "org".into(),
        workspace_selector: "ws".into(),
    }));
    workbench
}

/// A demo with one run just started, which has not ended yet.
fn started_run() -> (Demo, String) {
    let demo = Demo::new().unwrap();
    let workflow = demo
        .list(1)
        .unwrap()
        .workflows
        .into_iter()
        .find(|workflow| workflow.name == "Order fulfillment")
        .unwrap();
    let started = demo.run(&workflow.id, "page-tests").unwrap();
    (demo, started.id)
}

fn detail(demo: &Demo, id: &str) -> Box<ExecutionDetailResponse> {
    Box::new(demo.status(id).unwrap())
}

#[test]
fn changed_data_stays_on_screen_while_it_is_read_again() {
    let mut list = Remote::Ready(vec![1]);
    list.invalidate();
    assert_eq!(list, Remote::Stale(vec![1]));
    assert!(list.wants_read());
    list.begin();
    assert_eq!(list, Remote::Reloading(vec![1]));
    assert!(!list.wants_read());
    // A read already on its way is left to finish.
    list.invalidate();
    assert_eq!(list, Remote::Reloading(vec![1]));

    let mut failed = Remote::<Vec<u8>>::Failed("down".into());
    failed.invalidate();
    assert_eq!(failed, Remote::Idle);
    failed.begin();
    assert_eq!(failed, Remote::Loading);
}

#[test]
fn the_executions_page_watches_the_chosen_run_until_it_ends() {
    let mut workbench = workspace();
    let (demo, id) = started_run();
    workbench.go(Page::Executions);
    assert_eq!(workbench.wanted_watch(), None);

    workbench.executions.selected = Some(id.clone());
    // Until its first state arrives, the run may still change.
    assert_eq!(workbench.wanted_watch().as_deref(), Some(id.as_str()));

    let generation = workbench.session.generation();
    let mut state = detail(&demo, &id);
    workbench.receive_watch(generation, Ok(state.clone()));
    assert!(matches!(
        &workbench.executions.detail,
        Remote::Ready(shown) if shown.execution.id == id
    ));
    assert_eq!(workbench.wanted_watch().as_deref(), Some(id.as_str()));

    state.execution.status = ExecutionStatus::Completed;
    workbench.receive_watch(generation, Ok(state));
    assert_eq!(workbench.wanted_watch(), None);
}

#[test]
fn only_the_page_that_shows_a_run_watches_it() {
    let mut workbench = workspace();
    let (_, id) = started_run();
    workbench.executions.selected = Some(id);
    workbench.go(Page::Catalog);

    assert_eq!(workbench.wanted_watch(), None);
}

#[test]
fn a_failed_watch_waits_until_the_run_is_opened_again() {
    let mut workbench = workspace();
    let (_, id) = started_run();
    workbench.go(Page::Executions);
    workbench.executions.selected = Some(id.clone());

    workbench.receive_watch(workbench.session.generation(), Err(Failure::ReadFailed));

    assert_eq!(workbench.watch_failed.as_deref(), Some(id.as_str()));
    assert_eq!(workbench.wanted_watch(), None);
    assert!(workbench.feedback.failure);
    assert!(
        workbench
            .feedback
            .message
            .starts_with("Live status stopped")
    );
}

#[test]
fn watched_states_from_an_earlier_session_are_dropped() {
    let mut workbench = workspace();
    let (demo, id) = started_run();
    workbench.go(Page::Executions);
    workbench.executions.selected = Some(id.clone());
    let earlier = workbench.session.generation();
    workbench.session.switch(workbench.session.context.clone());

    workbench.receive_watch(earlier, Ok(detail(&demo, &id)));

    assert!(matches!(workbench.executions.detail, Remote::Idle));
}

#[test]
fn a_watched_state_reaches_every_place_that_shows_the_run() {
    let mut workbench = workspace();
    let (demo, id) = started_run();
    let page = demo
        .executions(&ExecutionQuery {
            workflow: None,
            statuses: String::new(),
            cursor: None,
            limit: 5,
        })
        .unwrap();
    workbench.executions.list = Remote::Ready(page.items);
    let mut state = detail(&demo, &id);
    state.execution.status = ExecutionStatus::Failed;

    workbench.receive_watch(workbench.session.generation(), Ok(state));

    let rows = workbench.executions.list.value().unwrap();
    let row = rows.iter().find(|row| row.id == id).unwrap();
    assert_eq!(row.status, ExecutionStatus::Failed);
}

#[test]
fn a_further_page_of_executions_extends_the_list() {
    let mut workbench = workspace();
    let demo = Demo::new().unwrap();
    let query = |cursor| ExecutionQuery {
        workflow: None,
        statuses: String::new(),
        cursor,
        limit: 3,
    };
    let first = demo.executions(&query(None)).unwrap();
    let cursor = first.next_cursor.clone();
    assert!(cursor.is_some());
    let stamp = workbench.session.begin().unwrap();
    workbench.receive(
        stamp,
        RequestKind::Load(Target::Executions),
        Ok(Reply::Executions(first, false)),
    );

    let second = demo.executions(&query(cursor)).unwrap();
    let stamp = workbench.session.begin().unwrap();
    workbench.receive(
        stamp,
        RequestKind::Load(Target::Executions),
        Ok(Reply::Executions(second, true)),
    );

    let rows = workbench.executions.list.value().unwrap();
    assert_eq!(rows.len(), 6);
    let mut ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    ids.dedup();
    assert_eq!(ids.len(), 6);
}

#[test]
fn a_creation_whose_answer_was_lost_closes_its_form_and_waits_for_the_list() {
    let mut workbench = workspace();
    let demo = Demo::new().unwrap();
    workbench.credentials.list = Remote::Ready(demo.credentials(1).unwrap().credentials);
    workbench.start_credential(Some("api_key".into()));
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(
        stamp,
        RequestKind::Change(Target::NewCredential),
        Err(Failure::OutcomeUnknown),
    );

    assert!(workbench.credentials.draft.is_none());
    // Shown while it is read again, but not settled, so creating stays off.
    assert!(workbench.credentials.list.value().is_some());
    assert!(!workbench.credentials.list.settled());
    assert!(workbench.credentials.list.wants_read());
    assert!(workbench.feedback.failure);
}

#[test]
fn an_uncertain_test_or_deletion_keeps_the_draft_being_typed() {
    let mut workbench = workspace();
    let demo = Demo::new().unwrap();
    workbench.credentials.list = Remote::Ready(demo.credentials(1).unwrap().credentials);
    workbench.start_credential(Some("api_key".into()));
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(
        stamp,
        RequestKind::Change(Target::Credentials),
        Err(Failure::OutcomeUnknown),
    );

    assert!(workbench.credentials.draft.is_some());
    assert!(workbench.credentials.list.wants_read());
}

fn history_page(demo: &Demo, limit: u32, cursor: Option<String>) -> ListExecutionsResponse {
    demo.executions(&ExecutionQuery {
        workflow: None,
        statuses: String::new(),
        cursor,
        limit,
    })
    .unwrap()
}

#[test]
fn an_answer_for_filters_changed_since_is_shown_but_read_again() {
    let mut workbench = workspace();
    let demo = Demo::new().unwrap();
    workbench.executions.read_for = Some((
        workbench.executions.statuses.clone(),
        workbench.executions.workflow.clone(),
    ));
    // The person picks a status while the unfiltered read is on its way.
    workbench.executions.statuses.insert(pages::StatusFilter(1));
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(
        stamp,
        RequestKind::Load(Target::Executions),
        Ok(Reply::Executions(history_page(&demo, 3, None), false)),
    );

    assert!(workbench.executions.list.value().is_some());
    assert!(workbench.executions.list.wants_read());
    assert!(workbench.executions.next_cursor.is_none());
}

#[test]
fn a_failed_older_page_keeps_the_rows_and_their_cursor() {
    let mut workbench = workspace();
    let demo = Demo::new().unwrap();
    let first = history_page(&demo, 3, None);
    let cursor = first.next_cursor.clone();
    let stamp = workbench.session.begin().unwrap();
    workbench.receive(
        stamp,
        RequestKind::Load(Target::Executions),
        Ok(Reply::Executions(first, false)),
    );
    workbench.executions.appending = true;
    let stamp = workbench.session.begin().unwrap();

    workbench.receive(
        stamp,
        RequestKind::Load(Target::Executions),
        Err(Failure::ReadFailed),
    );

    assert_eq!(workbench.executions.list.value().unwrap().len(), 3);
    assert_eq!(workbench.executions.next_cursor, cursor);
    assert!(workbench.executions.more_error.is_some());
}

#[test]
fn members_follow_the_server_s_upsert_and_cascade() {
    let mut workbench = workspace();
    let demo = Demo::new().unwrap();
    let organization = demo.org_members(demo::ORG).unwrap().members;
    let workspace_members = demo.workspace_members().unwrap().members;
    let grace = workspace_members
        .iter()
        .find(|member| {
            organization
                .iter()
                .any(|org| org.principal_id == member.principal_id)
        })
        .unwrap()
        .principal_id
        .clone();
    let count = organization.len();
    workbench.team.organization = Remote::Ready(organization.clone());
    workbench.team.workspace = Remote::Ready(workspace_members);

    // Adding someone already in the organization changes their role in place.
    let mut promoted = organization
        .iter()
        .find(|member| member.principal_id == grace)
        .unwrap()
        .clone();
    promoted.role.0 = "admin".into();
    let stamp = workbench.session.begin().unwrap();
    workbench.receive(
        stamp,
        RequestKind::Change(Target::OrgMembers),
        Ok(Reply::OrgMemberAdded(promoted)),
    );
    let members = workbench.team.organization.value().unwrap();
    assert_eq!(members.len(), count);
    assert!(
        members
            .iter()
            .any(|member| member.principal_id == grace && member.role.0 == "admin")
    );

    // Leaving the organization ends their workspace access too.
    let stamp = workbench.session.begin().unwrap();
    workbench.receive(
        stamp,
        RequestKind::Change(Target::OrgMembers),
        Ok(Reply::OrgMemberRemoved(grace.clone())),
    );
    assert!(
        !workbench
            .team
            .workspace
            .value()
            .unwrap()
            .iter()
            .any(|member| member.principal_id == grace)
    );
}

#[test]
fn a_failed_read_replaces_the_page_and_a_failed_change_keeps_it() {
    let mut workbench = workspace();
    let stamp = workbench.session.begin().unwrap();
    workbench.receive(
        stamp,
        RequestKind::Load(Target::Credentials),
        Err(Failure::ReadFailed),
    );
    assert!(matches!(
        &workbench.credentials.list,
        Remote::Failed(reason) if *reason == Failure::ReadFailed.to_string()
    ));

    workbench.credentials.list = Remote::Ready(Vec::new());
    let stamp = workbench.session.begin().unwrap();
    workbench.receive(
        stamp,
        RequestKind::Change(Target::Credentials),
        Err(Failure::Forbidden),
    );
    assert!(matches!(&workbench.credentials.list, Remote::Ready(list) if list.is_empty()));
    assert!(workbench.feedback.failure);
}
