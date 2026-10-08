//! Workbench state and transitions. Nothing here touches egui or the network, so every
//! reducer is testable with fabricated replies.

use crate::{
    document::Draft,
    effects::{Reply, RequestKind},
    schema::Form,
    session::{DraftKey, RequestStamp, Session, SessionContext},
    transport::{Connection, Failure, PAGE_SIZE},
};
use nebula_api_contract::v1::{
    catalog::{ActionParametersResponse, ActionSummary},
    execution::{ExecutionDetailResponse, ExecutionResponse, ListExecutionsResponse},
    me::MeResponse,
    workflow::{WorkflowDocumentResponse, WorkflowResponse},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use zeroize::Zeroize;

/// How many workspaces the workspace page offers again.
const RECENT_WORKSPACES: usize = 5;

/// A workspace the user opened before.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkspaceRef {
    pub(crate) organization: String,
    pub(crate) workspace: String,
}

/// What the app keeps between launches. It never holds a secret: no password, code or token.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Remembered {
    pub(crate) endpoint: String,
    pub(crate) email: String,
    pub(crate) mode: SignInMode,
    /// Most recent first.
    pub(crate) recent: Vec<WorkspaceRef>,
}

/// How the sign-in form authenticates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SignInMode {
    #[default]
    Password,
    Token,
}

/// Sign-in and workspace selection. Secrets are wiped when the form is dropped or consumed.
pub(crate) struct ConnectionForm {
    pub(crate) endpoint: String,
    pub(crate) mode: SignInMode,
    /// Set when the server asked for a second factor. The code field stays until sign-in completes.
    pub(crate) mfa_required: bool,
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
            mode: SignInMode::default(),
            mfa_required: false,
            email: String::new(),
            password: String::new(),
            totp: String::new(),
            token: String::new(),
            organization: String::new(),
            workspace: String::new(),
        }
    }

    /// Sign-in is offered only once the server address and the chosen credentials are filled in.
    pub(crate) fn can_sign_in(&self) -> bool {
        if self.endpoint.trim().is_empty() {
            return false;
        }
        match self.mode {
            SignInMode::Token => !self.token.trim().is_empty(),
            SignInMode::Password => {
                !self.email.trim().is_empty()
                    && !self.password.is_empty()
                    && (!self.mfa_required || !self.totp.trim().is_empty())
            },
        }
    }

    /// Both slugs are needed; an empty one would fail every workspace request before it reaches the server.
    pub(crate) fn can_open_workspace(&self) -> bool {
        !self.organization.trim().is_empty() && !self.workspace.trim().is_empty()
    }

    /// Switching modes wipes the secrets of the mode being left, so neither mode inherits the other's.
    pub(crate) fn set_mode(&mut self, mode: SignInMode) {
        if self.mode == mode {
            return;
        }
        self.mode = mode;
        match mode {
            SignInMode::Password => self.token.zeroize(),
            SignInMode::Token => {
                self.password.zeroize();
                self.totp.zeroize();
                self.mfa_required = false;
            },
        }
    }

    /// The server wants a code. The password stays, so the retry does not ask for it again.
    pub(crate) fn require_second_factor(&mut self) {
        self.mfa_required = true;
        self.totp.zeroize();
    }

    pub(crate) fn clear_secrets(&mut self) {
        self.password.zeroize();
        self.totp.zeroize();
        self.token.zeroize();
        self.mfa_required = false;
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
    /// Narrows the listed page by name. It filters what was read; it does not search the server.
    pub(crate) filter: String,
    /// The inline form for a new workflow is open.
    pub(crate) creating: bool,
    /// The name field takes the cursor on the next frame, once.
    pub(crate) focus_name: bool,
    pub(crate) new_name: String,
    refresh_pending: bool,
}

impl Navigator {
    /// Opens the inline form with the cursor in its name field.
    pub(crate) fn start_creating(&mut self) {
        self.creating = true;
        self.focus_name = true;
    }

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

    /// Workflows on the current page whose name contains the filter, ignoring case.
    pub(crate) fn visible(&self) -> Vec<&WorkflowResponse> {
        let filter = self.filter.trim().to_lowercase();
        self.workflows
            .iter()
            .filter(|workflow| filter.is_empty() || workflow.name.to_lowercase().contains(&filter))
            .collect()
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

/// Outcome of the last action. The view shows it as a toast: information fades, a failure stays until
/// dismissed.
#[derive(Default)]
pub(crate) struct Feedback {
    pub(crate) message: String,
    pub(crate) failure: bool,
    /// Bumped by every message, so a repeated text still counts as new.
    pub(crate) serial: u64,
}

impl Feedback {
    pub(crate) fn info(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.failure = false;
        self.serial += 1;
    }

    pub(crate) fn error(&mut self, message: impl Into<String>) {
        self.message = message.into();
        self.failure = true;
        self.serial += 1;
    }

    pub(crate) fn dismiss(&mut self) {
        self.message.clear();
        self.failure = false;
    }

    /// Shows the outcome of a local command: the success text, or the error that stopped it.
    pub(crate) fn report<E: std::fmt::Display>(&mut self, result: Result<(), E>, success: &str) {
        match result {
            Ok(()) => self.info(success),
            Err(error) => self.error(error.to_string()),
        }
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
    /// Recent runs were asked for since the workflow opened or last started a run, so the runs panel
    /// does not ask again every frame, even when the answer was a failure.
    pub(crate) history_requested: bool,
    /// Why the last read of recent runs failed, shown in place of the list until a read succeeds.
    pub(crate) history_error: Option<String>,
    pub(crate) feedback: Feedback,
    /// Lets the user return to the workspace form while a workspace is open.
    pub(crate) workspace_form_open: bool,
    /// Narrow layouts show the workflow list as a page while this is set. Wide layouts always show it.
    pub(crate) sidebar_open: bool,
    /// Node shown in the inspector. Cleared with the rest of the selection.
    pub(crate) selected_node: Option<String>,
    /// Name being edited for the selected node.
    pub(crate) rename: String,
    /// Output port being dragged on the canvas; the drop target decides the connection.
    pub(crate) link_from: Option<String>,
    /// Card being dragged on the canvas; its placement is recorded when the drag ends.
    pub(crate) node_drag: Option<NodeDrag>,
    /// Canvas scale. Purely visual, so it is not part of the draft or its history.
    pub(crate) zoom: f32,
    pub(crate) add_node: AddNodeForm,
    pub(crate) catalog: Catalog,
    /// Workspaces that listed successfully, most recent first.
    pub(crate) recent: Vec<WorkspaceRef>,
    /// Parameter schemas by action key, for the node form. They belong to the server signed in to.
    pub(crate) schemas: HashMap<String, SchemaState>,
    /// The action whose schema is being read, so a failure knows which entry it settles.
    pub(crate) schema_request: Option<String>,
    pub(crate) inspector_tab: InspectorTab,
}

/// A card being dragged. The offset is in screen pixels, so the card follows the pointer.
#[derive(Clone)]
pub(crate) struct NodeDrag {
    pub(crate) node: String,
    pub(crate) offset: [f32; 2],
}

pub(crate) const CATALOG_UNAVAILABLE: &str =
    "This server publishes no action catalog. Type the action key instead.";

/// What the server's action catalog answered the last time it was asked.
#[derive(Default)]
pub(crate) enum Catalog {
    #[default]
    NotRequested,
    Ready(Vec<ActionSummary>),
    /// The server has no action registry (503) or the request failed.
    Unavailable,
}

/// Tab of the node sidebar.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum InspectorTab {
    #[default]
    Parameters,
    Settings,
    Output,
}

/// What the editor knows about one action's parameter schema.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SchemaState {
    Loading,
    Ready(Form),
    /// The server has no schema for this action; the reason says why, for the inspector.
    Unavailable(String),
}

/// The add-node palette: whether it is open, its catalog filter, and the hand-typed action for servers
/// without a catalog.
#[derive(Default)]
pub(crate) struct AddNodeForm {
    pub(crate) open: bool,
    pub(crate) filter: String,
    pub(crate) action_key: String,
    pub(crate) name: String,
    /// Node the new one connects after, set by the "+" on an output port.
    pub(crate) connect_from: Option<String>,
}

impl AddNodeForm {
    /// Opens the palette for a node that follows `after`, or for a free node when it is `None`.
    pub(crate) fn open_after(&mut self, after: Option<String>) {
        self.open = true;
        self.connect_from = after;
    }

    pub(crate) fn close(&mut self) {
        *self = Self::default();
    }
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
            history_requested: false,
            history_error: None,
            feedback: Feedback::default(),
            workspace_form_open: false,
            sidebar_open: true,
            selected_node: None,
            rename: String::new(),
            link_from: None,
            node_drag: None,
            zoom: 1.0,
            add_node: AddNodeForm::default(),
            catalog: Catalog::NotRequested,
            recent: Vec::new(),
            schemas: HashMap::new(),
            schema_request: None,
            inspector_tab: InspectorTab::default(),
        }
    }

    /// Marks the schema of `action` as being read. Returns false when it is already known or pending,
    /// so each action is asked for once per sign-in.
    pub(crate) fn begin_schema(&mut self, action: &str) -> bool {
        if self.schemas.contains_key(action) {
            return false;
        }
        self.schemas.insert(action.to_owned(), SchemaState::Loading);
        self.schema_request = Some(action.to_owned());
        true
    }

    /// Marks the recent runs of the open workflow as being read and returns its id. Called when the
    /// request starts, not when the runs panel asks, so an intent dropped behind another request
    /// leaves the panel free to ask again.
    pub(crate) fn begin_recent_runs(&mut self) -> Option<String> {
        let workflow = self.session.draft()?.base.workflow.id.clone();
        self.history_requested = true;
        Some(workflow)
    }

    fn receive_schema(&mut self, action: String, schema: &ActionParametersResponse) {
        self.schemas
            .insert(action, SchemaState::Ready(Form::parse(&schema.parameters)));
        self.schema_request = None;
    }

    /// A failed read settles the pending entry with a reason the inspector shows instead of a toast.
    fn receive_schema_failure(&mut self, error: &Failure) {
        let Some(action) = self.schema_request.take() else {
            return;
        };
        let reason = match error {
            Failure::Rejected(503) => {
                "This server publishes no action catalog, so the form for this node is not available."
                    .to_owned()
            },
            Failure::Rejected(404) => format!("The server does not know the action {action}."),
            other => other.to_string(),
        };
        self.schemas
            .insert(action, SchemaState::Unavailable(reason));
    }

    /// Fills the sign-in form and the recent workspaces from the previous launch. An empty remembered
    /// address keeps the default one.
    pub(crate) fn restore(&mut self, remembered: Remembered) {
        if !remembered.endpoint.trim().is_empty() {
            self.form.endpoint = remembered.endpoint;
        }
        self.form.email = remembered.email;
        self.form.mode = remembered.mode;
        self.recent = remembered.recent;
        self.recent.truncate(RECENT_WORKSPACES);
    }

    pub(crate) fn remembered(&self) -> Remembered {
        Remembered {
            endpoint: self.form.endpoint.clone(),
            email: self.form.email.clone(),
            mode: self.form.mode,
            recent: self.recent.clone(),
        }
    }

    /// Moves the open workspace to the front of the recent list once the server has answered for it.
    fn remember_workspace(&mut self) {
        let Some(context) = &self.session.context else {
            return;
        };
        let opened = WorkspaceRef {
            organization: context.organization.clone(),
            workspace: context.workspace_selector.clone(),
        };
        self.recent.retain(|known| *known != opened);
        self.recent.insert(0, opened);
        self.recent.truncate(RECENT_WORKSPACES);
    }

    pub(crate) fn is_signed_in(&self) -> bool {
        self.profile.is_some()
    }

    pub(crate) fn workspace_open(&self) -> bool {
        self.session.context.is_some()
    }

    pub(crate) fn begin_sign_in(&mut self, connection: Connection) {
        self.session.switch(None);
        self.schemas.clear();
        self.schema_request = None;
        self.connection = Some(connection);
    }

    /// Scopes the session to the form's workspace. Returns false until a sign-in has completed and both
    /// slugs are filled in.
    pub(crate) fn open_workspace(&mut self) -> bool {
        let (Some(connection), Some(profile)) = (&self.connection, &self.profile) else {
            return false;
        };
        if !self.form.can_open_workspace() {
            return false;
        }
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
        self.sidebar_open = true;
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
        // On narrow layouts the list is a page, so opening a workflow hands the page back to the editor.
        self.sidebar_open = false;
        true
    }

    /// The workflow has a local draft in this workspace with edits the server has not seen.
    pub(crate) fn has_unsaved(&self, workflow: &str) -> bool {
        self.session.context.as_ref().is_some_and(|context| {
            let key = DraftKey {
                context: context.clone(),
                workflow: workflow.into(),
            };
            self.session.drafts.get(&key).is_some_and(Draft::dirty)
        })
    }

    pub(crate) fn clear_selection(&mut self) {
        self.parameter.close();
        self.selected_node = None;
        self.link_from = None;
        self.node_drag = None;
        // The palette and its "connect after" target belong to the workflow that was open.
        self.add_node.close();
        self.status = None;
        self.history = None;
        self.history_requested = false;
        self.history_error = None;
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
        // The next sign-in may reach another server, whose actions differ.
        self.catalog = Catalog::NotRequested;
        self.schemas.clear();
        self.schema_request = None;
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

    /// A second-factor request is a question for the user, so it is shown as information, not a failure.
    fn receive_sign_in_failure(&mut self, error: Failure) {
        if error == Failure::MfaRequired {
            self.form.require_second_factor();
            self.feedback.info(error.to_string());
        } else {
            self.form.clear_secrets();
            self.feedback.error(error.to_string());
        }
    }

    fn receive_failure(&mut self, kind: RequestKind, error: Failure) {
        if kind == RequestKind::Connect {
            self.receive_sign_in_failure(error);
            return;
        }
        if kind == RequestKind::Schema && error != Failure::Unauthorized {
            self.receive_schema_failure(&error);
            return;
        }
        if kind == RequestKind::History && error != Failure::Unauthorized {
            // The runs panel says it where the list would be, so no toast repeats it.
            self.history_error = Some(error.to_string());
            return;
        }
        if kind == RequestKind::Create && error == Failure::OutcomeUnknown {
            // The server may already hold the workflow. Creation is not keyed, so a retry could make a
            // second one: the form closes and the list is read, so the person decides with it in view.
            self.navigator.refresh_pending = true;
            self.navigator.creating = false;
            self.navigator.new_name.clear();
            self.feedback.error(
                "The workflow may have been created. Check the list before creating it again.",
            );
            return;
        }
        if kind == RequestKind::Catalog {
            self.catalog = Catalog::Unavailable;
        }
        // A server without a catalog is a state the palette shows itself, so it needs no toast.
        if kind != RequestKind::Catalog || error != Failure::Rejected(503) {
            self.feedback.error(error.to_string());
        }
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
                // The workspace page that follows is the confirmation, so no toast repeats it.
                self.form.clear_secrets();
            },
            Reply::Listed(page) => {
                self.navigator.workflows = page.workflows;
                self.navigator.total = page.total;
                self.navigator.page = page.page;
                // Only a workspace the server answered for is worth offering again.
                self.remember_workspace();
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
            // Both land in the runs panel, which is their confirmation; a toast would only repeat it.
            Reply::History(history, status) => {
                self.history = Some(history);
                self.history_error = None;
                match status {
                    Some(Ok(status)) => self.status = Some(status),
                    Some(Err(Failure::Unauthorized)) => {
                        self.disconnect();
                        self.feedback.error(Failure::Unauthorized.to_string());
                    },
                    Some(Err(error)) => self.feedback.error(error.to_string()),
                    None => {},
                }
            },
            Reply::Status(status) => self.status = Some(status),
            Reply::Action(action, detail) => self.receive_schema(action, &detail),
            Reply::Actions(list) => {
                let count = list.actions.len();
                self.catalog = Catalog::Ready(list.actions);
                self.feedback
                    .info(format!("{count} actions in the server catalog."));
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
                self.navigator.creating = false;
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
                // A reload of an unchanged draft looks the same, so it says that it happened.
                self.feedback.info("Up to date with the server.");
            }
        } else {
            match Draft::new(document) {
                // Opening a workflow shows it, which is its own confirmation.
                Ok(draft) => {
                    self.session.drafts.insert(key, draft);
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
        // Activation writes exactly the next revision of the one it was given. A later revision means
        // another write landed before the read, so the loaded version is not the published one.
        if draft.base.revision.checked_add(1) == Some(document.revision) {
            draft.published_revision = Some(document.revision);
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
        // The new run belongs in the list, so the runs panel reads it again.
        self.history_requested = false;
        self.feedback
            .info("Run accepted. Read persisted status to see whether it has started.");
    }
}

#[cfg(test)]
mod tests;
