//! A self-contained Nebula workspace for exploring the client without a server. It answers the same
//! calls as the HTTP API with the contract's own types and the server's rules: revisions fence
//! saves and publications, publication validates the graph, starts are idempotent per key, and a
//! started run is played by the simulated executor (`executor`), whose node states advance in time.
//! The action catalog and credential types are snapshots of the bundled server's release.

mod eval;
mod executor;
mod seed;

use crate::{
    clock,
    schema::{self, Form, Values},
    transport::{CREDENTIALS_PER_PAGE, ExecutionQuery, Failure, PAGE_SIZE, status_key},
};
use executor::Run;
use nebula_api_contract::v1::{
    catalog::{ActionDetailResponse, ActionParametersResponse, ActionSummary, ListActionsResponse},
    credential::{
        CreateCredentialRequest, CredentialLifecycleState, CredentialResponse, CredentialSummary,
        CredentialTestFailureCodeV1, CredentialTypeInfo, ListCredentialTypesResponse,
        ListCredentialsResponse, TestCredentialResponse,
    },
    execution::{ExecutionDetailResponse, ExecutionResponse, ListExecutionsResponse},
    me::{
        CreateTokenRequest, CreateTokenResponse, MeResponse, MyTokensResponse, TokenSummary,
        UpdateMeRequest,
    },
    org::{AddMemberRequest, MemberSummary, MembersResponse},
    shared::{AckResponse, OrgRoleDto},
    webhook::{RegisterWebhookRequest, RegisterWebhookResponse},
    workflow::{
        CreateWorkflowRequest, ListWorkflowsResponse, UpdateWorkflowDocumentRequest,
        WorkflowDocumentResponse, WorkflowResponse,
    },
    workspace_membership::{
        UpsertWorkspaceMemberRequest, WorkspaceMemberSummary, WorkspaceMembersResponse,
    },
};
use serde_json::{Map, Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

/// The demo's organization and workspace slugs.
pub(crate) const ORG: &str = "acme";
pub(crate) const WORKSPACE: &str = "production";

const ORG_ROLES: [&str; 4] = ["owner", "admin", "billing", "member"];
const WORKSPACE_ROLES: [&str; 4] = ["viewer", "runner", "editor", "admin"];

/// The demo workspace. Clones share one world, as calls to one server share its state.
#[derive(Clone)]
pub(crate) struct Demo {
    world: Arc<Mutex<World>>,
}

/// One action of the catalog snapshot.
#[derive(serde::Deserialize)]
struct CatalogEntry {
    detail: ActionDetailResponse,
    parameters: Value,
}

#[derive(serde::Deserialize)]
struct Catalog {
    actions: Vec<CatalogEntry>,
}

struct StoredWorkflow {
    document: WorkflowDocumentResponse,
    /// The definition the last publication activated; runs use it, not the draft.
    published: Option<Value>,
    /// The payload a run starts with, standing in for the trigger's event.
    sample_input: Value,
}

struct World {
    /// Monotonic source of identities.
    sequence: u64,
    me: MeResponse,
    catalog: Vec<CatalogEntry>,
    credential_types: Vec<CredentialTypeInfo>,
    workflows: Vec<StoredWorkflow>,
    runs: Vec<Run>,
    /// Idempotency keys of starts, so a retried start returns the same run.
    starts: BTreeMap<String, String>,
    credentials: Vec<CredentialResponse>,
    tokens: Vec<TokenSummary>,
    org_members: Vec<MemberSummary>,
    workspace_members: Vec<WorkspaceMemberSummary>,
    webhooks: Vec<(String, String)>,
}

impl Demo {
    /// A fresh demo workspace with its seeded workflows, history, credentials and team.
    pub(crate) fn new() -> Result<Self, Failure> {
        let catalog: Catalog =
            serde_json::from_str(include_str!("catalog.json")).map_err(|_| Failure::Unsupported)?;
        let types: ListCredentialTypesResponse =
            serde_json::from_str(include_str!("credential_types.json"))
                .map_err(|_| Failure::Unsupported)?;
        let mut world = World {
            sequence: 0,
            me: seed::me(),
            catalog: catalog.actions,
            credential_types: types.types,
            workflows: Vec::new(),
            runs: Vec::new(),
            starts: BTreeMap::new(),
            credentials: Vec::new(),
            tokens: Vec::new(),
            org_members: seed::org_members(),
            workspace_members: seed::workspace_members(),
            webhooks: Vec::new(),
        };
        seed::populate(&mut world, clock::now_millis());
        Ok(Self {
            world: Arc::new(Mutex::new(world)),
        })
    }

    fn with<R>(&self, call: impl FnOnce(&mut World) -> Result<R, Failure>) -> Result<R, Failure> {
        let mut world = self.world.lock().map_err(|_| Failure::InvalidResponse)?;
        call(&mut world)
    }

    pub(crate) fn me(&self) -> Result<MeResponse, Failure> {
        self.with(|world| {
            world.me.tokens_count = world.tokens.len() as u32;
            Ok(world.me.clone())
        })
    }

    pub(crate) fn update_me(&self, request: &UpdateMeRequest) -> Result<MeResponse, Failure> {
        self.with(|world| {
            if let Some(name) = &request.display_name {
                let name = name.trim();
                if name.is_empty() {
                    return Err(Failure::Invalid("display_name: Enter a name.".to_owned()));
                }
                name.clone_into(&mut world.me.display_name);
            }
            Ok(world.me.clone())
        })
    }

    pub(crate) fn list(&self, page: usize) -> Result<ListWorkflowsResponse, Failure> {
        self.with(|world| {
            let mut workflows: Vec<WorkflowResponse> = world
                .workflows
                .iter()
                .map(|stored| stored.document.workflow.clone())
                .collect();
            workflows.sort_by_key(|workflow| std::cmp::Reverse(workflow.updated_at));
            let total = workflows.len();
            let page = page.max(1);
            Ok(ListWorkflowsResponse {
                workflows: workflows
                    .into_iter()
                    .skip((page - 1) * PAGE_SIZE)
                    .take(PAGE_SIZE)
                    .collect(),
                total,
                page,
                page_size: PAGE_SIZE,
            })
        })
    }

    pub(crate) fn load(&self, id: &str) -> Result<WorkflowDocumentResponse, Failure> {
        self.with(|world| Ok(world.workflow(id)?.document.clone()))
    }

    pub(crate) fn create(
        &self,
        request: &CreateWorkflowRequest,
    ) -> Result<WorkflowDocumentResponse, Failure> {
        self.with(|world| {
            let name = request.name.trim();
            if name.is_empty() {
                return Err(Failure::Invalid("name: Enter a name.".to_owned()));
            }
            let now = clock::now_millis();
            let id = world.next_id("wf", now);
            let mut definition = request.definition.clone();
            if let Some(object) = definition.as_object_mut() {
                object.insert("id".into(), json!(id));
                object.insert("name".into(), json!(name));
            }
            let document = seed::document(&id, name, definition, now / 1000, 1);
            world.workflows.push(StoredWorkflow {
                document: document.clone(),
                published: None,
                sample_input: json!({}),
            });
            Ok(document)
        })
    }

    /// Merges the patch's top-level definition keys over the stored definition, as the server does.
    pub(crate) fn save(
        &self,
        id: &str,
        request: &UpdateWorkflowDocumentRequest,
    ) -> Result<WorkflowDocumentResponse, Failure> {
        self.with(|world| {
            let stored = world.workflow_mut(id)?;
            if request
                .expected_revision
                .is_some_and(|expected| expected != stored.document.revision)
            {
                return Err(Failure::Conflict);
            }
            let document = &mut stored.document;
            if let Some(name) = &request.update.name {
                name.clone_into(&mut document.workflow.name);
            }
            if let Some(description) = &request.update.description {
                document.workflow.description = Some(description.clone());
            }
            if let (Some(patch), Some(definition)) = (
                request
                    .update
                    .definition
                    .as_ref()
                    .and_then(Value::as_object),
                document.definition.as_object_mut(),
            ) {
                for (key, value) in patch {
                    definition.insert(key.clone(), value.clone());
                }
            }
            document.revision += 1;
            document.workflow.updated_at = clock::now_millis() / 1000;
            Ok(document.clone())
        })
    }

    pub(crate) fn publish(
        &self,
        id: &str,
        revision: u64,
    ) -> Result<WorkflowDocumentResponse, Failure> {
        self.with(|world| {
            let issues = world.validate(&world.workflow(id)?.document.definition);
            let stored = world.workflow_mut(id)?;
            if stored.document.revision != revision {
                return Err(Failure::Conflict);
            }
            if !issues.is_empty() {
                return Err(Failure::Invalid(format!(
                    "The server rejected this workflow. {}",
                    issues.join(" ")
                )));
            }
            stored.published = Some(stored.document.definition.clone());
            stored.document.revision += 1;
            stored.document.workflow.updated_at = clock::now_millis() / 1000;
            Ok(stored.document.clone())
        })
    }

    pub(crate) fn run(&self, workflow: &str, key: &str) -> Result<ExecutionResponse, Failure> {
        self.with(|world| {
            let now = clock::now_millis();
            if let Some(existing) = world.starts.get(key) {
                let run = world.run(existing)?;
                return Ok(receipt(run, now));
            }
            let stored = world.workflow(workflow)?;
            let definition = stored.published.clone().ok_or_else(|| {
                Failure::Invalid(
                    "The workflow is not activated. Publish it before running it.".to_owned(),
                )
            })?;
            let input = stored.sample_input.clone();
            let id = world.next_id("exe", now);
            let run = Run::plan(
                id.clone(),
                workflow.to_owned(),
                &definition,
                Some(input),
                now,
            );
            let answer = receipt(&run, now);
            world.runs.push(run);
            world.starts.insert(key.to_owned(), id);
            Ok(answer)
        })
    }

    pub(crate) fn history(&self, workflow: &str) -> Result<ListExecutionsResponse, Failure> {
        self.executions(&ExecutionQuery {
            workflow: Some(workflow.to_owned()),
            limit: 20,
            ..ExecutionQuery::default()
        })
    }

    /// Newest first, filtered and paged by an opaque cursor (the position of the next item).
    pub(crate) fn executions(
        &self,
        query: &ExecutionQuery,
    ) -> Result<ListExecutionsResponse, Failure> {
        self.with(|world| {
            let now = clock::now_millis();
            let wanted: Vec<&str> = query
                .statuses
                .split(',')
                .map(str::trim)
                .filter(|status| !status.is_empty())
                .collect();
            let mut matching: Vec<&Run> = world
                .runs
                .iter()
                .filter(|run| {
                    query
                        .workflow
                        .as_ref()
                        .is_none_or(|workflow| &run.workflow_id == workflow)
                })
                .filter(|run| wanted.is_empty() || wanted.contains(&status_key(run.status(now))))
                .collect();
            matching.sort_by_key(|run| std::cmp::Reverse(run.created));
            let offset = match &query.cursor {
                Some(cursor) => cursor
                    .strip_prefix("c")
                    .and_then(|position| position.parse::<usize>().ok())
                    .ok_or(Failure::Rejected(400))?,
                None => 0,
            };
            let limit = query.limit.clamp(1, 100) as usize;
            let items: Vec<_> = matching
                .iter()
                .skip(offset)
                .take(limit)
                .map(|run| run.summary(now))
                .collect();
            let has_more = matching.len() > offset + limit;
            Ok(ListExecutionsResponse {
                items,
                next_cursor: has_more.then(|| format!("c{}", offset + limit)),
                has_more,
            })
        })
    }

    pub(crate) fn status(&self, id: &str) -> Result<ExecutionDetailResponse, Failure> {
        self.with(|world| Ok(world.run(id)?.detail(clock::now_millis())))
    }

    pub(crate) fn cancel(&self, id: &str) -> Result<ExecutionResponse, Failure> {
        self.with(|world| {
            let now = clock::now_millis();
            let run = world
                .runs
                .iter_mut()
                .find(|run| run.id == id)
                .ok_or(Failure::Rejected(404))?;
            if !run.cancel(now) {
                return Err(Failure::Invalid(
                    "This run has already finished, so there is nothing to cancel.".to_owned(),
                ));
            }
            Ok(receipt(run, now))
        })
    }

    pub(crate) fn actions(&self) -> Result<ListActionsResponse, Failure> {
        self.with(|world| {
            Ok(ListActionsResponse {
                actions: world
                    .catalog
                    .iter()
                    .map(|entry| ActionSummary {
                        key: entry.detail.key.clone(),
                        name: entry.detail.name.clone(),
                        version: entry.detail.version.clone(),
                    })
                    .collect(),
            })
        })
    }

    pub(crate) fn action(&self, key: &str) -> Result<ActionDetailResponse, Failure> {
        self.with(|world| {
            Ok(world
                .action(key)
                .ok_or(Failure::Rejected(404))?
                .detail
                .clone())
        })
    }

    pub(crate) fn action_parameters(&self, key: &str) -> Result<ActionParametersResponse, Failure> {
        self.with(|world| {
            let entry = world.action(key).ok_or(Failure::Rejected(404))?;
            Ok(ActionParametersResponse {
                key: entry.detail.key.clone(),
                parameters: entry.parameters.clone(),
            })
        })
    }

    pub(crate) fn credential_types(&self) -> Result<ListCredentialTypesResponse, Failure> {
        self.with(|world| {
            Ok(ListCredentialTypesResponse {
                types: world.credential_types.clone(),
            })
        })
    }

    /// One page of credentials, paged as the server pages them.
    pub(crate) fn credentials(&self, page: usize) -> Result<ListCredentialsResponse, Failure> {
        self.with(|world| {
            let page = page.max(1);
            let credentials: Vec<CredentialSummary> = world
                .credentials
                .iter()
                .skip((page - 1) * CREDENTIALS_PER_PAGE)
                .take(CREDENTIALS_PER_PAGE)
                .map(|credential| CredentialSummary {
                    id: credential.id.clone(),
                    credential_key: credential.credential_key.clone(),
                    name: credential.name.clone(),
                    auth_pattern: credential.auth_pattern.clone(),
                    expires_at: credential.expires_at.clone(),
                    version: credential.version,
                    lifecycle: credential.lifecycle.clone(),
                })
                .collect();
            Ok(ListCredentialsResponse {
                total: world.credentials.len(),
                credentials,
                page,
                page_size: CREDENTIALS_PER_PAGE,
            })
        })
    }

    /// Checks the data against the type's schema the way the server admits it: every required field
    /// present and non-empty.
    pub(crate) fn create_credential(
        &self,
        request: &CreateCredentialRequest,
    ) -> Result<CredentialResponse, Failure> {
        self.with(|world| {
            let kind = world
                .credential_types
                .iter()
                .find(|kind| kind.key == request.credential_key)
                .ok_or_else(|| {
                    Failure::Invalid("credential_key: Choose a credential type.".into())
                })?
                .clone();
            if request.name.trim().is_empty() {
                return Err(Failure::Invalid("name: Enter a name.".to_owned()));
            }
            let form = Form::from_json_schema(&kind.schema);
            let entries: Map<String, Value> = request
                .data
                .as_object()
                .into_iter()
                .flatten()
                .map(|(key, value)| (key.clone(), json!({"type": "literal", "value": value})))
                .collect();
            let values = Values::of(&form, &entries);
            let missing: Vec<String> = if form.tagged_union {
                // A union holds exactly one of its tags.
                let tags = request.data.as_object().map_or(0, Map::len);
                if tags == 1 {
                    Vec::new()
                } else {
                    vec!["data: Choose one type and fill it in.".to_owned()]
                }
            } else {
                form.fields
                    .iter()
                    .filter(|field| field.is_required(&values))
                    .filter(|field| request.data.get(&field.key).is_none_or(schema::is_empty))
                    .map(|field| format!("data.{}: Provide a value.", field.key))
                    .collect()
            };
            if !missing.is_empty() {
                return Err(Failure::Invalid(missing.join(" ")));
            }
            let now = clock::rfc3339(clock::now_millis());
            let credential = CredentialResponse {
                id: world.next_id("cred", clock::now_millis()),
                credential_key: kind.key.clone(),
                name: request.name.trim().to_owned(),
                description: request.description.clone(),
                auth_pattern: kind.auth_pattern.clone(),
                capabilities: kind.capabilities,
                created_at: now.clone(),
                updated_at: now,
                expires_at: None,
                version: 1,
                lifecycle: CredentialLifecycleState::Ready,
                tags: request.tags.clone().unwrap_or_default(),
            };
            world.credentials.push(credential.clone());
            Ok(credential)
        })
    }

    pub(crate) fn delete_credential(&self, id: &str) -> Result<AckResponse, Failure> {
        self.with(|world| {
            let before = world.credentials.len();
            world.credentials.retain(|credential| credential.id != id);
            if world.credentials.len() == before {
                return Err(Failure::Rejected(404));
            }
            Ok(AckResponse::ok())
        })
    }

    /// A credential that needs reauthorization fails its test, as the provider would reject it.
    pub(crate) fn test_credential(&self, id: &str) -> Result<TestCredentialResponse, Failure> {
        self.with(|world| {
            let credential = world
                .credentials
                .iter()
                .find(|credential| credential.id == id)
                .ok_or(Failure::Rejected(404))?;
            let tested_at = clock::rfc3339(clock::now_millis());
            Ok(match credential.lifecycle {
                CredentialLifecycleState::ReauthRequired => TestCredentialResponse::Failed {
                    code: CredentialTestFailureCodeV1::AuthenticationRejected,
                    message: "The provider rejected the stored grant. Reauthorize this credential."
                        .to_owned(),
                    tested_at,
                },
                _ => TestCredentialResponse::Success {
                    message: "The provider accepted the credential.".to_owned(),
                    tested_at,
                },
            })
        })
    }

    pub(crate) fn register_webhook(
        &self,
        request: &RegisterWebhookRequest,
    ) -> Result<RegisterWebhookResponse, Failure> {
        self.with(|world| {
            let stored = world.workflow(&request.workflow_id)?;
            let bound = stored.document.definition["trigger_bindings"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|binding| binding["id"] == request.trigger_id.as_str());
            if !bound {
                return Err(Failure::Invalid(format!(
                    "trigger_id: The workflow has no trigger `{}`.",
                    request.trigger_id
                )));
            }
            if request.provider != "generic" {
                return Err(Failure::Invalid(format!(
                    "provider: Unknown webhook provider \"{}\".",
                    request.provider
                )));
            }
            // The server names an activation by the trigger's UUID.
            let activation = uuid::Uuid::new_v4().to_string();
            let secret = world.next_secret("whsec");
            world
                .webhooks
                .push((request.workflow_id.clone(), request.trigger_id.clone()));
            Ok(RegisterWebhookResponse {
                webhook_url: format!("https://nebula.example/hooks/{activation}"),
                signing_secret: secret,
                activation_id: activation,
            })
        })
    }

    pub(crate) fn tokens(&self) -> Result<MyTokensResponse, Failure> {
        self.with(|world| {
            Ok(MyTokensResponse {
                tokens: world.tokens.clone(),
            })
        })
    }

    pub(crate) fn create_token(
        &self,
        request: &CreateTokenRequest,
    ) -> Result<CreateTokenResponse, Failure> {
        self.with(|world| {
            if request.name.trim().is_empty() {
                return Err(Failure::Invalid("name: Enter a name.".to_owned()));
            }
            crate::api::check_scopes(&request.scopes).map_err(Failure::Invalid)?;
            let now = clock::now_millis();
            let summary = TokenSummary {
                id: world.next_id("pat", now),
                name: request.name.trim().to_owned(),
                scopes: request.scopes.clone(),
                created_at: clock::rfc3339(now),
                last_used_at: None,
                expires_at: request
                    .ttl_seconds
                    .map(|ttl| clock::rfc3339(now + i64::try_from(ttl).unwrap_or(0) * 1000)),
            };
            world.tokens.push(summary.clone());
            Ok(CreateTokenResponse {
                token: world.next_secret("nbl_pat"),
                summary,
            })
        })
    }

    pub(crate) fn revoke_token(&self, id: &str) -> Result<AckResponse, Failure> {
        self.with(|world| {
            let before = world.tokens.len();
            world.tokens.retain(|token| token.id != id);
            if world.tokens.len() == before {
                return Err(Failure::Rejected(404));
            }
            Ok(AckResponse::ok())
        })
    }

    pub(crate) fn org_members(&self, org: &str) -> Result<MembersResponse, Failure> {
        self.with(|world| {
            check_org(org)?;
            Ok(MembersResponse {
                members: world.org_members.clone(),
            })
        })
    }

    pub(crate) fn add_org_member(
        &self,
        org: &str,
        request: &AddMemberRequest,
    ) -> Result<MemberSummary, Failure> {
        self.with(|world| {
            check_org(org)?;
            check_principal(&request.principal_id)?;
            if !ORG_ROLES.contains(&request.role.0.as_str()) {
                return Err(Failure::Invalid(
                    "role: Choose owner, admin, billing or member.".to_owned(),
                ));
            }
            if world
                .org_members
                .iter()
                .any(|member| member.principal_id == request.principal_id)
            {
                return Err(Failure::Conflict);
            }
            let member = MemberSummary {
                principal_id: request.principal_id.clone(),
                role: OrgRoleDto(request.role.0.clone()),
            };
            world.org_members.push(member.clone());
            Ok(member)
        })
    }

    /// The last owner stays, so the organization never loses its owner.
    pub(crate) fn remove_org_member(
        &self,
        org: &str,
        principal: &str,
    ) -> Result<AckResponse, Failure> {
        self.with(|world| {
            check_org(org)?;
            let owners = world
                .org_members
                .iter()
                .filter(|member| member.role.0 == "owner")
                .count();
            let target = world
                .org_members
                .iter()
                .find(|member| member.principal_id == principal)
                .ok_or(Failure::Rejected(404))?;
            if target.role.0 == "owner" && owners == 1 {
                return Err(Failure::Invalid(
                    "The organization needs at least one owner.".to_owned(),
                ));
            }
            world
                .org_members
                .retain(|member| member.principal_id != principal);
            // As the server does: leaving the organization ends every workspace grant in it.
            world
                .workspace_members
                .retain(|member| member.principal_id != principal);
            Ok(AckResponse::ok())
        })
    }

    pub(crate) fn workspace_members(&self) -> Result<WorkspaceMembersResponse, Failure> {
        self.with(|world| {
            Ok(WorkspaceMembersResponse {
                members: world.workspace_members.clone(),
            })
        })
    }

    pub(crate) fn set_workspace_member(
        &self,
        principal: &str,
        request: &UpsertWorkspaceMemberRequest,
    ) -> Result<WorkspaceMemberSummary, Failure> {
        self.with(|world| {
            check_principal(principal)?;
            if !WORKSPACE_ROLES.contains(&request.role.0.as_str()) {
                return Err(Failure::Invalid(
                    "role: Choose viewer, runner, editor or admin.".to_owned(),
                ));
            }
            let member = WorkspaceMemberSummary {
                principal_id: principal.to_owned(),
                role: request.role.clone(),
            };
            match world
                .workspace_members
                .iter_mut()
                .find(|existing| existing.principal_id == principal)
            {
                Some(existing) => *existing = member.clone(),
                None => world.workspace_members.push(member.clone()),
            }
            Ok(member)
        })
    }

    pub(crate) fn remove_workspace_member(&self, principal: &str) -> Result<AckResponse, Failure> {
        self.with(|world| {
            let before = world.workspace_members.len();
            world
                .workspace_members
                .retain(|member| member.principal_id != principal);
            if world.workspace_members.len() == before {
                return Err(Failure::Rejected(404));
            }
            Ok(AckResponse::ok())
        })
    }
}

impl World {
    fn workflow(&self, id: &str) -> Result<&StoredWorkflow, Failure> {
        self.workflows
            .iter()
            .find(|stored| stored.document.workflow.id == id)
            .ok_or(Failure::Rejected(404))
    }

    fn workflow_mut(&mut self, id: &str) -> Result<&mut StoredWorkflow, Failure> {
        self.workflows
            .iter_mut()
            .find(|stored| stored.document.workflow.id == id)
            .ok_or(Failure::Rejected(404))
    }

    fn run(&self, id: &str) -> Result<&Run, Failure> {
        self.runs
            .iter()
            .find(|run| run.id == id)
            .ok_or(Failure::Rejected(404))
    }

    fn action(&self, key: &str) -> Option<&CatalogEntry> {
        self.catalog.iter().find(|entry| entry.detail.key == key)
    }

    /// A ULID-style identity (`wf_01M4D4B2HZ7VJDJBF15K354H5F`): the instant, then a counter-derived
    /// suffix, so identities sort by creation like the server's.
    fn next_id(&mut self, prefix: &str, at: i64) -> String {
        self.sequence += 1;
        let mut value =
            (u128::from(at.max(0).unsigned_abs()) << 80) | u128::from(mix(self.sequence));
        let mut text = [0_u8; 26];
        for slot in text.iter_mut().rev() {
            *slot = CROCKFORD[(value & 31) as usize];
            value >>= 5;
        }
        format!("{prefix}_{}", String::from_utf8_lossy(&text))
    }

    /// A secret shown once, such as a token or a webhook signing secret.
    fn next_secret(&mut self, prefix: &str) -> String {
        self.sequence += 1;
        let first = mix(self.sequence);
        let second = mix(self.sequence ^ 0x5eed);
        format!("{prefix}_{first:016x}{second:016x}")
    }

    /// The checks publication makes, as `path: remediation` lines.
    fn validate(&self, definition: &Value) -> Vec<String> {
        let nodes = definition["nodes"].as_array().cloned().unwrap_or_default();
        let mut issues = Vec::new();
        if nodes.is_empty() {
            issues.push("nodes: Add at least one node.".to_owned());
        }
        let ids: Vec<&str> = nodes
            .iter()
            .filter_map(|node| node["id"].as_str())
            .collect();
        for (index, node) in nodes.iter().enumerate() {
            let key = crate::document::catalog_key(node);
            let Some(entry) = self.action(&key) else {
                issues.push(format!(
                    "nodes[{index}].action_key: `{key}` is not in the catalog; choose an action it lists."
                ));
                continue;
            };
            let form = Form::parse(&entry.parameters);
            let entries = node["parameters"].as_object().cloned().unwrap_or_default();
            let values = Values::of(&form, &entries);
            for field in &form.fields {
                let set =
                    entries
                        .get(&field.key)
                        .is_some_and(|entry| match entry["type"].as_str() {
                            Some("literal") => !schema::is_empty(&entry["value"]),
                            Some("expression") => entry["expr"]
                                .as_str()
                                .is_some_and(|text| !text.trim().is_empty()),
                            _ => true,
                        });
                if field.is_required(&values) && !set && field.default.is_none() {
                    issues.push(format!(
                        "nodes[{index}].parameters.{}: Provide a value.",
                        field.key
                    ));
                }
            }
        }
        let connections = definition["connections"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for (index, connection) in connections.iter().enumerate() {
            for end in ["from_node", "to_node"] {
                if !connection[end]
                    .as_str()
                    .is_some_and(|node| ids.contains(&node))
                {
                    issues.push(format!(
                        "connections[{index}].{end}: Connect existing nodes only."
                    ));
                }
            }
        }
        if has_cycle(&ids, &connections) {
            issues.push("connections: Remove the cycle; a workflow runs in one direction.".into());
        }
        // The server lists the first issues only, so a broken graph does not drown its own message.
        issues.truncate(3);
        issues
    }
}

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// SplitMix64: spreads a counter into well-mixed bits.
const fn mix(seed: u64) -> u64 {
    let mut value = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}

fn has_cycle(ids: &[&str], connections: &[Value]) -> bool {
    let mut incoming: Vec<usize> = ids
        .iter()
        .map(|id| {
            connections
                .iter()
                .filter(|connection| connection["to_node"] == *id)
                .count()
        })
        .collect();
    let mut placed = 0;
    let mut done = vec![false; ids.len()];
    while let Some(index) = (0..ids.len()).find(|index| !done[*index] && incoming[*index] == 0) {
        done[index] = true;
        placed += 1;
        for connection in connections
            .iter()
            .filter(|connection| connection["from_node"] == ids[index])
        {
            if let Some(target) = ids.iter().position(|id| connection["to_node"] == *id) {
                incoming[target] = incoming[target].saturating_sub(1);
            }
        }
    }
    placed < ids.len()
}

fn check_org(org: &str) -> Result<(), Failure> {
    if org == ORG {
        Ok(())
    } else {
        Err(Failure::Forbidden)
    }
}

/// Principals are user identities, `usr_` followed by a ULID.
fn check_principal(principal: &str) -> Result<(), Failure> {
    let valid = principal.strip_prefix("usr_").is_some_and(|rest| {
        rest.len() == 26
            && rest
                .bytes()
                .all(|byte| CROCKFORD.contains(&byte.to_ascii_uppercase()))
    });
    if valid {
        Ok(())
    } else {
        Err(Failure::Invalid(
            "principal_id: Enter a user identity such as usr_01M4D48S6NAPHAHN345EHADBPK."
                .to_owned(),
        ))
    }
}

/// The acknowledgement of a start or a cancel: the run as it stands.
fn receipt(run: &Run, at: i64) -> ExecutionResponse {
    ExecutionResponse {
        id: run.id.clone(),
        workflow_id: run.workflow_id.clone(),
        status: status_key(run.status(at)).to_owned(),
        started_at: run.created / 1000,
        finished_at: None,
        input: None,
        output: None,
    }
}

#[cfg(test)]
mod tests;
