//! How the workspace pages take their replies and failures, and how a watched execution's states
//! reach every place that shows that execution.

use super::{
    Workbench,
    pages::{CredentialDraft, RegisteredWebhook, Remote},
};
use crate::{
    effects::{Reply, Target, ended},
    transport::Failure,
};
use nebula_api_contract::v1::{
    credential::TestCredentialResponse,
    execution::{ExecutionDetailResponse, ExecutionSummary},
};
use zeroize::Zeroizing;

impl Workbench {
    /// Replies to the pages' requests. The editor's replies are taken in `receive_reply`.
    pub(super) fn receive_page_reply(&mut self, reply: Reply) {
        match reply {
            Reply::ActionDetail(detail) => {
                self.catalog_page.detail_request = None;
                self.catalog_page
                    .details
                    .insert(detail.key.clone(), Remote::Ready(detail));
            },
            Reply::Executions(page, appending) => {
                let executions = &mut self.executions;
                let mut items = page.items;
                if appending && let Some(shown) = executions.list.value_mut() {
                    shown.append(&mut items);
                    items = std::mem::take(shown);
                }
                executions.list = Remote::Ready(items);
                executions.next_cursor = page.next_cursor;
                executions.appending = false;
            },
            Reply::Execution(detail) => self.receive_execution(detail),
            Reply::Cancelled(execution) => {
                // A watched run shows the cancellation as its states stream in; any other row is
                // read again.
                if self.wanted_watch().as_deref() != Some(execution.id.as_str()) {
                    self.executions.list.invalidate();
                }
                self.feedback
                    .info("Cancellation requested. The runtime stops the run under its own lease.");
            },
            Reply::Rerun(receipt) => {
                self.executions.selected = Some(receipt.id);
                self.executions.detail = Remote::Idle;
                self.executions.node = None;
                self.executions.list.invalidate();
                self.feedback.info("Run started.");
            },
            Reply::CredentialTypes(list) => self.credentials.types = Remote::Ready(list.types),
            Reply::Credentials(list) => self.credentials.list = Remote::Ready(list.credentials),
            Reply::CredentialCreated(created) => {
                self.credentials.draft = None;
                self.credentials.list.invalidate();
                self.feedback.info(format!(
                    "Credential \u{201c}{}\u{201d} saved.",
                    created.name
                ));
            },
            Reply::CredentialDeleted(id) => {
                if let Some(list) = self.credentials.list.value_mut() {
                    list.retain(|credential| credential.id != id);
                }
                self.credentials.tests.remove(&id);
                self.credentials.confirm_delete = None;
                self.feedback.info("Credential deleted.");
            },
            Reply::CredentialTested(id, result) => {
                match &result {
                    TestCredentialResponse::Success { message, .. } => {
                        self.feedback.info(message.clone());
                    },
                    TestCredentialResponse::Failed { message, .. } => {
                        self.feedback.error(message.clone());
                    },
                }
                self.credentials.tests.insert(id, result);
            },
            Reply::Documents(documents) => self.triggers.documents = Remote::Ready(documents),
            Reply::TriggersSaved(document) => {
                // A draft without local edits follows the new revision; one with edits reviews it
                // like any other server change when it saves.
                if let Some(draft) = self
                    .session
                    .drafts
                    .values_mut()
                    .find(|draft| draft.base.workflow.id == document.workflow.id)
                    && !draft.dirty()
                {
                    draft.saved((*document).clone());
                }
                if let Some(documents) = self.triggers.documents.value_mut()
                    && let Some(stored) = documents
                        .iter_mut()
                        .find(|stored| stored.workflow.id == document.workflow.id)
                {
                    *stored = *document;
                }
                self.feedback.info("Triggers saved.");
            },
            Reply::WebhookRegistered {
                workflow,
                trigger,
                response,
            } => {
                self.triggers.registered = Some(RegisteredWebhook {
                    workflow,
                    trigger,
                    url: response.webhook_url.clone(),
                    secret: Zeroizing::new(response.signing_secret.clone()),
                });
                self.feedback
                    .info("Webhook registered. Copy its signing secret now; it is shown once.");
            },
            Reply::Profile(profile) => {
                profile
                    .display_name
                    .clone_into(&mut self.settings.display_name);
                self.profile = Some(profile.clone());
                self.settings.profile = Remote::Ready(profile);
            },
            Reply::Tokens(list) => self.settings.tokens = Remote::Ready(list.tokens),
            Reply::TokenCreated(created) => {
                if let Some(tokens) = self.settings.tokens.value_mut() {
                    tokens.push(created.summary.clone());
                }
                self.settings.revealed = Some((
                    created.summary.name.clone(),
                    Zeroizing::new(created.token.clone()),
                ));
                self.settings.new_token = super::pages::NewToken::default();
            },
            Reply::TokenRevoked(id) => {
                if let Some(tokens) = self.settings.tokens.value_mut() {
                    tokens.retain(|token| token.id != id);
                }
                self.settings.confirm_revoke = None;
                self.feedback.info("Token revoked.");
            },
            Reply::OrgMembers(list) => self.team.organization = Remote::Ready(list.members),
            Reply::OrgMemberAdded(member) => {
                // Adding someone already in the organization changes their role.
                if let Some(members) = self.team.organization.value_mut() {
                    match members
                        .iter_mut()
                        .find(|existing| existing.principal_id == member.principal_id)
                    {
                        Some(existing) => *existing = member,
                        None => members.push(member),
                    }
                }
                self.team.new_member.clear();
                self.feedback.info("Member added to the organization.");
            },
            Reply::OrgMemberRemoved(principal) => {
                if let Some(members) = self.team.organization.value_mut() {
                    members.retain(|member| member.principal_id != principal);
                }
                // The server takes their workspace access away with them.
                if let Some(members) = self.team.workspace.value_mut() {
                    members.retain(|member| member.principal_id != principal);
                }
                self.team.confirm_remove = None;
                self.feedback.info("Member removed from the organization.");
            },
            Reply::WorkspaceMembers(list) => self.team.workspace = Remote::Ready(list.members),
            Reply::WorkspaceMemberSet(member) => {
                if let Some(members) = self.team.workspace.value_mut() {
                    match members
                        .iter_mut()
                        .find(|existing| existing.principal_id == member.principal_id)
                    {
                        Some(existing) => *existing = member,
                        None => members.push(member),
                    }
                }
                self.feedback.info("Workspace access updated.");
            },
            Reply::WorkspaceMemberRemoved(principal) => {
                if let Some(members) = self.team.workspace.value_mut() {
                    members.retain(|member| member.principal_id != principal);
                }
                self.team.confirm_remove = None;
                self.feedback.info("Workspace access removed.");
            },
            // The editor's replies, taken in `receive_reply` before this is reached.
            Reply::Connected(_)
            | Reply::Listed(_)
            | Reply::Created(_)
            | Reply::Loaded(_)
            | Reply::Saved(_)
            | Reply::Published(_)
            | Reply::Started(_)
            | Reply::History(..)
            | Reply::Status(_)
            | Reply::Actions(_)
            | Reply::Action(..) => {},
        }
    }

    /// A page's read failed: the reason replaces the data, where the page would have shown it.
    pub(super) fn receive_load_failure(&mut self, target: Target, error: &Failure) {
        let reason = error.to_string();
        match target {
            Target::Workflows => self.navigator.read = Remote::Failed(reason),
            Target::ActionDetail => {
                if let Some(key) = self.catalog_page.detail_request.take() {
                    self.catalog_page
                        .details
                        .insert(key, Remote::Failed(reason));
                }
            },
            Target::Executions => {
                self.executions.list = Remote::Failed(reason);
                self.executions.appending = false;
            },
            Target::Execution => self.executions.detail = Remote::Failed(reason),
            Target::CredentialTypes => self.credentials.types = Remote::Failed(reason),
            Target::Credentials => self.credentials.list = Remote::Failed(reason),
            Target::Triggers => self.triggers.documents = Remote::Failed(reason),
            Target::Profile => self.settings.profile = Remote::Failed(reason),
            Target::Tokens => self.settings.tokens = Remote::Failed(reason),
            Target::OrgMembers => self.team.organization = Remote::Failed(reason),
            Target::WorkspaceMembers => self.team.workspace = Remote::Failed(reason),
        }
    }

    /// A change whose answer was lost may have happened. Creations are not keyed, so a blind retry
    /// could make a second credential or token, or register a webhook again: the form closes, the
    /// list it changes is read again, and creating stays off until that read settles (see
    /// [`Remote::settled`]), so the person decides with the server's state in view.
    pub(super) fn receive_uncertain_change(&mut self, target: Target) {
        match target {
            Target::Executions => self.executions.list.invalidate(),
            Target::Execution => {
                self.executions.detail.invalidate();
                self.executions.list.invalidate();
            },
            Target::Credentials => {
                self.credentials.draft = None;
                self.credentials.list.invalidate();
            },
            Target::Tokens => {
                self.settings.new_token = super::pages::NewToken::default();
                self.settings.tokens.invalidate();
            },
            Target::Triggers => self.triggers.documents.invalidate(),
            Target::Profile => self.settings.profile.invalidate(),
            Target::OrgMembers => {
                self.team.organization.invalidate();
                self.team.workspace.invalidate();
            },
            Target::WorkspaceMembers => self.team.workspace.invalidate(),
            Target::Workflows | Target::ActionDetail | Target::CredentialTypes => {},
        }
        self.feedback.error(
            "The change may have been made. The list is being read again; check it before trying again.",
        );
    }

    /// One execution's state, from a read or the watch: the executions page's detail and row, and
    /// the editor's run panel when it shows the same run.
    fn receive_execution(&mut self, detail: Box<ExecutionDetailResponse>) {
        let summary = &detail.execution;
        replace_row(self.executions.list.value_mut(), summary);
        if let Some(history) = &mut self.history {
            replace_row(Some(&mut history.items), summary);
        }
        let editor_shows = self
            .session
            .draft()
            .is_some_and(|draft| draft.execution_id.as_deref() == Some(summary.id.as_str()));
        if editor_shows {
            self.status = Some(detail.clone());
        }
        if self.executions.selected.as_deref() == Some(summary.id.as_str()) {
            self.executions.detail = Remote::Ready(detail);
        }
    }

    /// A watched execution's new state, or the failure that ended its watch. States read in an
    /// earlier session are dropped.
    pub(crate) fn receive_watch(
        &mut self,
        generation: u64,
        result: Result<Box<ExecutionDetailResponse>, Failure>,
    ) {
        if generation != self.session.generation() {
            return;
        }
        match result {
            Ok(detail) => self.receive_execution(detail),
            Err(Failure::Unauthorized) => {
                self.disconnect();
                self.feedback.error(Failure::Unauthorized.to_string());
            },
            Err(error) => {
                // Not watched again until the run is opened again, so a failing server is not
                // asked in a loop.
                self.watch_failed = self.wanted_watch();
                self.feedback.error(format!(
                    "Live status stopped: {error} Open the run again to resume."
                ));
            },
        }
    }

    /// The execution whose states should stream right now: the one the visible page shows, while it
    /// can still change.
    pub(crate) fn wanted_watch(&self) -> Option<String> {
        self.live_execution()
            .filter(|execution| self.watch_failed.as_ref() != Some(execution))
    }

    /// The execution the visible page shows while it can still change.
    fn live_execution(&self) -> Option<String> {
        let live = |detail: Option<&ExecutionDetailResponse>| {
            detail.is_none_or(|detail| !ended(detail.execution.status))
        };
        match self.page {
            super::pages::Page::Executions => {
                let selected = self.executions.selected.clone()?;
                let shown = self
                    .executions
                    .detail
                    .value()
                    .filter(|detail| detail.execution.id == selected);
                live(shown.map(AsRef::as_ref)).then_some(selected)
            },
            super::pages::Page::Editor => {
                let execution = self.session.draft()?.execution_id.clone()?;
                let shown = self
                    .status
                    .as_deref()
                    .filter(|status| status.execution.id == execution);
                live(shown).then_some(execution)
            },
            _ => None,
        }
    }

    /// Opens the credential form for a type, or closes it with `None`.
    pub(crate) fn start_credential(&mut self, kind: Option<String>) {
        self.credentials.draft = kind.map(|kind| CredentialDraft {
            kind,
            name: String::new(),
            entries: serde_json::Map::new(),
        });
    }
}

/// Puts an execution's latest summary in place of its row, if the list shows it.
fn replace_row(rows: Option<&mut Vec<ExecutionSummary>>, summary: &ExecutionSummary) {
    if let Some(row) = rows.into_iter().flatten().find(|row| row.id == summary.id) {
        *row = summary.clone();
    }
}
