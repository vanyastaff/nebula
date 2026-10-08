//! Workbench state and transitions. Nothing here touches egui or the network, so every
//! reducer is testable with fabricated replies.

use crate::{
    document::Draft,
    effects::{Reply, RequestKind},
    session::{DraftKey, RequestStamp, Session, SessionContext},
    transport::{Connection, Failure, PAGE_SIZE},
};
use nebula_api_contract::v1::{
    execution::{ExecutionDetailResponse, ExecutionResponse, ListExecutionsResponse},
    me::MeResponse,
    workflow::{WorkflowDocumentResponse, WorkflowResponse},
};
use zeroize::Zeroize;

/// Sign-in and workspace selection. Secrets are wiped when the form is dropped or consumed.
pub(crate) struct ConnectionForm {
    pub(crate) endpoint: String,
    pub(crate) use_token: bool,
    pub(crate) email: String,
    pub(crate) password: String,
    pub(crate) totp: String,
    pub(crate) token: String,
    pub(crate) organization: String,
    pub(crate) workspace: String,
}

impl ConnectionForm {
    pub(crate) fn new(endpoint: String) -> Self {
        Self {
            endpoint,
            use_token: false,
            email: String::new(),
            password: String::new(),
            totp: String::new(),
            token: String::new(),
            organization: String::new(),
            workspace: String::new(),
        }
    }

    pub(crate) fn clear_secrets(&mut self) {
        self.password.zeroize();
        self.totp.zeroize();
        self.token.zeroize();
    }
}

impl Drop for ConnectionForm {
    fn drop(&mut self) {
        self.clear_secrets();
    }
}

#[derive(Default)]
pub(crate) struct Navigator {
    pub(crate) workflows: Vec<WorkflowResponse>,
    pub(crate) page: usize,
    pub(crate) total: usize,
    pub(crate) new_name: String,
    refresh_pending: bool,
}

impl Navigator {
    pub(crate) fn has_previous_page(&self) -> bool {
        self.page > 1
    }

    pub(crate) fn has_next_page(&self) -> bool {
        self.page * PAGE_SIZE < self.total
    }

    /// True once after a workflow was created, so the caller re-reads the first page.
    pub(crate) fn take_refresh(&mut self) -> bool {
        std::mem::take(&mut self.refresh_pending)
    }
}

/// The literal parameter currently open in the inspector.
#[derive(Default)]
pub(crate) struct ParameterSelection {
    pub(crate) node: String,
    pub(crate) parameter: String,
    pub(crate) text: String,
}

impl ParameterSelection {
    pub(crate) fn is_open(&self) -> bool {
        !self.parameter.is_empty()
    }

    pub(crate) fn open(&mut self, node: &str, parameter: &str, text: String) {
        self.node = node.into();
        self.parameter = parameter.into();
        self.text = text;
    }

    pub(crate) fn close(&mut self) {
        self.node.clear();
        self.parameter.clear();
        self.text.zeroize();
    }
}

impl Drop for ParameterSelection {
    fn drop(&mut self) {
        self.close();
    }
}

#[derive(Default)]
pub(crate) struct Feedback {
    pub(crate) message: String,
    pub(crate) failure: bool,
}

impl Feedback {
    pub(crate) fn info(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.failure = false;
    }

    pub(crate) fn error(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.failure = true;
    }
}

pub(crate) struct Workbench {
    pub(crate) session: Session,
    pub(crate) connection: Option<Connection>,
    pub(crate) profile: Option<MeResponse>,
    pub(crate) form: ConnectionForm,
    pub(crate) navigator: Navigator,
    pub(crate) parameter: ParameterSelection,
    pub(crate) status: Option<Box<ExecutionDetailResponse>>,
    pub(crate) history: Option<ListExecutionsResponse>,
    pub(crate) feedback: Feedback,
    /// Lets the user return to the workspace form while a workspace is open.
    pub(crate) workspace_form_open: bool,
}

/// Which draft commands are safe to offer. Views disable the rest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DraftGate {
    pub(crate) can_save: bool,
    pub(crate) can_publish: bool,
    pub(crate) can_run: bool,
}

/// A draft is settled when no server conflict or uncertain write is waiting for review.
pub(crate) fn draft_gate(draft: &Draft) -> DraftGate {
    let settled = draft.remote.is_none() && !draft.uncertain_save && !draft.save_conflict;
    DraftGate {
        can_save: settled && draft.dirty(),
        can_publish: settled && !draft.dirty(),
        can_run: settled && !draft.dirty(),
    }
}

impl Workbench {
    pub(crate) fn new(endpoint: String) -> Self {
        Self {
            session: Session::default(),
            connection: None,
            profile: None,
            form: ConnectionForm::new(endpoint),
            navigator: Navigator::default(),
            parameter: ParameterSelection::default(),
            status: None,
            history: None,
            feedback: Feedback {
                message: "Connect to an existing Nebula server to open a workspace.".into(),
                failure: false,
            },
            workspace_form_open: false,
        }
    }

    pub(crate) fn is_signed_in(&self) -> bool {
        self.profile.is_some()
    }

    pub(crate) fn workspace_open(&self) -> bool {
        self.session.context.is_some()
    }

    pub(crate) fn begin_sign_in(&mut self, connection: Connection) {
        self.session.switch(None);
        self.connection = Some(connection);
    }

    /// Scopes the session to the form's workspace. Returns false until a sign-in has completed.
    pub(crate) fn open_workspace(&mut self) -> bool {
        let (Some(connection), Some(profile)) = (&self.connection, &self.profile) else {
            return false;
        };
        self.session.switch(Some(SessionContext {
            endpoint: connection.endpoint().into(),
            principal: profile.user_id.clone(),
            organization: self.form.organization.trim().into(),
            workspace_selector: self.form.workspace.trim().into(),
        }));
        self.navigator.workflows.clear();
        self.navigator.total = 0;
        self.navigator.page = 1;
        self.clear_selection();
        self.workspace_form_open = false;
        true
    }

    /// Makes `workflow` the selected draft. The caller then loads it from the server.
    pub(crate) fn select_workflow(&mut self, workflow: &str) -> bool {
        let Some(context) = self.session.context.clone() else {
            return false;
        };
        self.session.selected = Some(DraftKey {
            context,
            workflow: workflow.into(),
        });
        self.clear_selection();
        true
    }

    pub(crate) fn clear_selection(&mut self) {
        self.parameter.close();
        self.status = None;
        self.history = None;
    }

    pub(crate) fn disconnect(&mut self) {
        self.session.switch(None);
        self.connection = None;
        self.profile = None;
        self.workspace_form_open = false;
        self.navigator.workflows.clear();
        self.navigator.total = 0;
        self.form.clear_secrets();
        self.clear_selection();
        self.feedback
            .info("Disconnected. Your drafts remain available in this app session.");
    }

    /// Applies one completed request. Replies from a superseded session are dropped.
    pub(crate) fn receive(
        &mut self,
        stamp: RequestStamp,
        kind: RequestKind,
        result: Result<Reply, Failure>,
    ) {
        if !self.session.accept(stamp) {
            return;
        }
        match result {
            Ok(reply) => self.receive_reply(reply),
            Err(error) => self.receive_failure(kind, error),
        }
    }

    fn receive_failure(&mut self, kind: RequestKind, error: Failure) {
        self.feedback.error(error.to_string());
        if let Some(draft) = self.session.draft_mut() {
            // A definite conflict or rejection leaves the draft for review; an uncertain write
            // keeps its flag until the server is read.
            if matches!(kind, RequestKind::Save | RequestKind::Publish)
                && error != Failure::OutcomeUnknown
            {
                draft.uncertain_save = false;
                draft.save_conflict = error == Failure::Conflict;
            }
            if kind == RequestKind::Run && error != Failure::OutcomeUnknown {
                draft.start_key = None;
            }
        }
        if error == Failure::Unauthorized {
            self.disconnect();
            self.feedback.error(error.to_string());
        }
    }

    fn receive_reply(&mut self, reply: Reply) {
        match reply {
            Reply::Connected(signed_in) => {
                self.connection = Some(signed_in.connection);
                self.profile = Some(signed_in.profile);
                self.feedback
                    .info("Signed in. Enter your organization and workspace slug or ID.");
            },
            Reply::Listed(page) => {
                self.navigator.workflows = page.workflows;
                self.navigator.total = page.total;
                self.navigator.page = page.page;
                self.feedback
                    .info(format!("{} workflows in this workspace.", page.total));
            },
            Reply::Created(document) => self.receive_created(document),
            Reply::Loaded(document) => self.receive_loaded(document),
            Reply::Saved(document) => {
                if let Some(draft) = self.session.draft_mut() {
                    draft.saved(document);
                }
                self.feedback
                    .info("Changes saved. Publish this version before running it.");
            },
            Reply::Published(document) => self.receive_published(document),
            Reply::Started(receipt) => self.receive_started(receipt),
            Reply::History(history) => {
                self.history = Some(history);
                self.feedback.info("Recent runs read from the server.");
            },
            Reply::Status(status) => {
                self.status = Some(status);
                self.feedback
                    .info("Execution snapshot read from persisted server state.");
            },
        }
    }

    fn receive_created(&mut self, document: WorkflowDocumentResponse) {
        let Some(context) = self.session.context.clone() else {
            return;
        };
        // The server stored the workflow even if the editor cannot open it, so the list refreshes either way.
        self.navigator.refresh_pending = true;
        let key = DraftKey {
            context,
            workflow: document.workflow.id.clone(),
        };
        match Draft::new(document) {
            Ok(draft) => {
                self.session.drafts.insert(key.clone(), draft);
                self.session.selected = Some(key);
                self.clear_selection();
                self.navigator.new_name.clear();
                self.feedback.info("Workflow created with an empty graph.");
            },
            Err(error) => self.feedback.error(error.to_string()),
        }
    }

    fn receive_loaded(&mut self, document: WorkflowDocumentResponse) {
        let Some(key) = self.session.selected.clone() else {
            return;
        };
        if document.workflow.id != key.workflow {
            self.feedback.error(Failure::InvalidResponse.to_string());
            return;
        }
        if let Some(draft) = self.session.drafts.get_mut(&key) {
            if draft.dirty() || draft.uncertain_save {
                draft.remote = Some(document);
                self.feedback.info(
                    "Server version read. Review it before replacing or reapplying your draft.",
                );
            } else {
                draft.saved(document);
                self.feedback.info("Workflow loaded.");
            }
        } else {
            match Draft::new(document) {
                Ok(draft) => {
                    self.session.drafts.insert(key, draft);
                    self.feedback.info("Workflow loaded.");
                },
                Err(error) => self.feedback.error(error.to_string()),
            }
        }
        self.parameter.close();
    }

    fn receive_published(&mut self, document: WorkflowDocumentResponse) {
        let Some(draft) = self.session.draft_mut() else {
            return;
        };
        if draft.definition["nodes"] == document.definition["nodes"] {
            draft.saved(document);
            self.feedback
                .info("Workflow published. Run uses the server's current publication.");
        } else {
            draft.remote = Some(document);
            self.feedback
                .error("The workflow changed during publication. Review the server version.");
        }
    }

    fn receive_started(&mut self, receipt: ExecutionResponse) {
        if let Some(draft) = self.session.draft_mut() {
            if receipt.workflow_id != draft.base.workflow.id {
                self.feedback.error(Failure::OutcomeUnknown.to_string());
                return;
            }
            draft.execution_id = Some(receipt.id);
            draft.start_key = None;
        }
        self.status = None;
        self.feedback
            .info("Run accepted. Read persisted status to see whether it has started.");
    }
}

#[cfg(test)]
mod tests {
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
    fn reply_from_a_superseded_session_changes_nothing() {
        let mut workbench = Workbench::new(String::new());
        open_workspace_session(&mut workbench);
        let stamp = workbench.session.begin().unwrap();
        workbench.session.switch(None);
        let listed: ListWorkflowsResponse = serde_json::from_value(
            json!({"workflows": [], "total": 3, "page": 1, "page_size": 25}),
        )
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
}
