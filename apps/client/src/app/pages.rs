//! Turns the workspace pages' intents into requests. Each page marks its data as being read only
//! after the request started, so an intent dropped while another request is in flight leaves the
//! page as it was and the page asks again on a later frame.

use super::ClientApp;
use crate::{
    effects::Operation,
    schema::{Form, credential_data},
    transport::{ExecutionQuery, status_key},
    views::{Intent, triggers::WEBHOOK_PROVIDER},
    workbench::pages::Remote,
};
use eframe::egui;
use nebula_api_contract::v1::{
    credential::CreateCredentialRequest,
    me::{CreateTokenRequest, UpdateMeRequest},
    org::AddMemberRequest,
    shared::{OrgRoleDto, WorkspaceRoleDto},
    webhook::RegisterWebhookRequest,
    workflow::{UpdateWorkflowDocumentRequest, UpdateWorkflowRequest},
    workspace_membership::UpsertWorkspaceMemberRequest,
};
use serde_json::{Value, json};

/// Executions read per page of the history.
const EXECUTIONS_PER_PAGE: u32 = 25;
const SECONDS_PER_DAY: u64 = 86_400;

impl ClientApp {
    /// The pages' intents; the editor's are run in `run_intent`.
    pub(super) fn run_page_intent(&mut self, context: &egui::Context, intent: Intent) {
        match intent {
            Intent::LoadActionDetail(key) => {
                if self.dispatch(context, Operation::ActionDetail(key.clone())) {
                    let catalog = &mut self.workbench.catalog_page;
                    catalog.detail_request = Some(key.clone());
                    catalog.details.insert(key, Remote::Loading);
                }
            },
            Intent::LoadExecutions { more } => self.load_executions(context, more),
            Intent::OpenExecution(id) => {
                if self.dispatch(context, Operation::Execution(id.clone())) {
                    let executions = &mut self.workbench.executions;
                    if executions.selected.as_deref() != Some(id.as_str()) {
                        executions.node = None;
                    }
                    executions.selected = Some(id.clone());
                    executions.detail = Remote::Loading;
                    // Opening a run again is how a stopped live status resumes.
                    if self.workbench.watch_failed.as_deref() == Some(id.as_str()) {
                        self.workbench.watch_failed = None;
                    }
                }
            },
            Intent::CancelExecution(id) => {
                self.dispatch(context, Operation::Cancel(id));
            },
            Intent::RerunWorkflow(workflow) => {
                let key = uuid::Uuid::new_v4().to_string();
                self.dispatch(context, Operation::Rerun(workflow, key));
            },
            Intent::LoadCredentials => {
                if self.dispatch(context, Operation::Credentials) {
                    self.workbench.credentials.list.begin();
                }
            },
            Intent::LoadCredentialTypes => {
                if self.dispatch(context, Operation::CredentialTypes) {
                    self.workbench.credentials.types.begin();
                }
            },
            Intent::CreateCredential => {
                if let Some(request) = self.credential_request() {
                    self.dispatch(context, Operation::CreateCredential(request));
                }
            },
            Intent::DeleteCredential(id) => {
                self.dispatch(context, Operation::DeleteCredential(id));
            },
            Intent::TestCredential(id) => {
                self.dispatch(context, Operation::TestCredential(id));
            },
            Intent::LoadTriggers => {
                if self.dispatch(context, Operation::Documents) {
                    self.workbench.triggers.documents.begin();
                }
            },
            Intent::SaveTriggers(workflow, bindings) => {
                if let Some(request) = self.triggers_request(&workflow, bindings) {
                    self.dispatch(context, Operation::SaveTriggers(workflow, request));
                }
            },
            Intent::RegisterWebhook(workflow_id, trigger_id) => {
                let request = RegisterWebhookRequest {
                    workflow_id,
                    trigger_id,
                    provider: WEBHOOK_PROVIDER.to_owned(),
                    replay_window_secs: None,
                    timestamp_header: None,
                    provider_config: None,
                    rate_limit_per_minute: None,
                };
                self.dispatch(context, Operation::RegisterWebhook(request));
            },
            Intent::LoadProfile => {
                if self.dispatch(context, Operation::Profile) {
                    self.workbench.settings.profile.begin();
                }
            },
            Intent::SaveProfile => {
                let request = UpdateMeRequest {
                    display_name: Some(self.workbench.settings.display_name.trim().to_owned()),
                    avatar_url: None,
                };
                self.dispatch(context, Operation::UpdateProfile(request));
            },
            Intent::LoadTokens => {
                if self.dispatch(context, Operation::Tokens) {
                    self.workbench.settings.tokens.begin();
                }
            },
            Intent::CreateToken => {
                let new = &self.workbench.settings.new_token;
                let request = CreateTokenRequest {
                    name: new.name.trim().to_owned(),
                    scopes: new.scopes.iter().cloned().collect(),
                    ttl_seconds: Some(u64::from(new.ttl_days) * SECONDS_PER_DAY),
                };
                self.dispatch(context, Operation::CreateToken(request));
            },
            Intent::RevokeToken(id) => {
                self.dispatch(context, Operation::RevokeToken(id));
            },
            Intent::LoadOrgMembers => {
                if self.dispatch(context, Operation::OrgMembers) {
                    self.workbench.team.organization.begin();
                }
            },
            Intent::AddOrgMember => {
                let team = &self.workbench.team;
                let request = AddMemberRequest {
                    principal_id: team.new_member.trim().to_owned(),
                    role: OrgRoleDto(team.new_role.clone()),
                };
                self.dispatch(context, Operation::AddOrgMember(request));
            },
            Intent::RemoveOrgMember(principal) => {
                self.dispatch(context, Operation::RemoveOrgMember(principal));
            },
            Intent::LoadWorkspaceMembers => {
                if self.dispatch(context, Operation::WorkspaceMembers) {
                    self.workbench.team.workspace.begin();
                }
            },
            Intent::SetWorkspaceMember(principal, role) => {
                let request = UpsertWorkspaceMemberRequest {
                    role: WorkspaceRoleDto(role),
                };
                self.dispatch(context, Operation::SetWorkspaceMember(principal, request));
            },
            Intent::RemoveWorkspaceMember(principal) => {
                self.dispatch(context, Operation::RemoveWorkspaceMember(principal));
            },
            // The editor's intents, run in `run_intent` before this is reached.
            Intent::SignIn
            | Intent::OpenDemo
            | Intent::OpenWorkspace
            | Intent::ListWorkflows(_)
            | Intent::CreateWorkflow
            | Intent::LoadWorkflow(_)
            | Intent::SaveDraft
            | Intent::PublishDraft
            | Intent::RunDraft
            | Intent::RefreshRuns
            | Intent::LoadExecution(_)
            | Intent::LoadCatalog
            | Intent::LoadSchema(_) => {},
        }
    }

    /// Reads the history as the page filters it; `more` reads the page after the shown ones.
    fn load_executions(&mut self, context: &egui::Context, more: bool) {
        let executions = &self.workbench.executions;
        let cursor = if more {
            let Some(cursor) = executions.next_cursor.clone() else {
                return;
            };
            Some(cursor)
        } else {
            None
        };
        let statuses = executions
            .statuses
            .iter()
            .filter_map(|filter| filter.status())
            .map(status_key)
            .collect::<Vec<_>>()
            .join(",");
        let query = ExecutionQuery {
            workflow: executions.workflow.clone(),
            statuses,
            cursor,
            limit: EXECUTIONS_PER_PAGE,
        };
        if self.dispatch(context, Operation::Executions(query)) {
            let executions = &mut self.workbench.executions;
            executions.appending = more;
            if !more {
                executions.list.begin();
            }
        }
    }

    /// The credential being created, its values shaped by its type's schema.
    fn credential_request(&self) -> Option<CreateCredentialRequest> {
        let credentials = &self.workbench.credentials;
        let draft = credentials.draft.as_ref()?;
        let kind = credentials
            .types
            .value()?
            .iter()
            .find(|kind| kind.key == draft.kind)?;
        let form = Form::from_json_schema(&kind.schema);
        Some(CreateCredentialRequest {
            credential_key: kind.key.clone(),
            name: draft.name.trim().to_owned(),
            description: None,
            data: Value::Object(credential_data(&form, &draft.entries)),
            tags: None,
        })
    }

    /// Replaces a workflow's trigger bindings, fenced by the revision the page shows.
    fn triggers_request(
        &self,
        workflow: &str,
        bindings: Value,
    ) -> Option<UpdateWorkflowDocumentRequest> {
        let document = self
            .workbench
            .triggers
            .documents
            .value()?
            .iter()
            .find(|document| document.workflow.id == workflow)?;
        Some(UpdateWorkflowDocumentRequest {
            update: UpdateWorkflowRequest {
                name: None,
                description: None,
                definition: Some(json!({ "trigger_bindings": bindings })),
            },
            expected_revision: Some(document.revision),
        })
    }
}
