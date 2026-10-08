//! The engine's API as the client uses it. Every call takes and returns the contract's own types
//! (`nebula-api-contract`), answered either by a Nebula server over HTTP or by the built-in demo
//! workspace, so pages and reducers never know which one is behind them.

use crate::{
    demo::Demo,
    session::SessionContext,
    transport::{Connection, ExecutionQuery, Failure, SignIn},
};
use nebula_api_contract::v1::{
    catalog::{ActionDetailResponse, ActionParametersResponse, ListActionsResponse},
    credential::{
        CreateCredentialRequest, CredentialResponse, ListCredentialTypesResponse,
        ListCredentialsResponse, TestCredentialResponse,
    },
    execution::{ExecutionDetailResponse, ExecutionResponse, ListExecutionsResponse},
    me::{CreateTokenRequest, CreateTokenResponse, MeResponse, MyTokensResponse, UpdateMeRequest},
    org::{AddMemberRequest, MemberSummary, MembersResponse},
    shared::AckResponse,
    webhook::{RegisterWebhookRequest, RegisterWebhookResponse},
    workflow::{
        CreateWorkflowRequest, ListWorkflowsResponse, UpdateWorkflowDocumentRequest,
        WorkflowDocumentResponse,
    },
    workspace_membership::{
        UpsertWorkspaceMemberRequest, WorkspaceMemberSummary, WorkspaceMembersResponse,
    },
};

/// Where requests go. Cloning shares the same server session or the same demo world.
#[derive(Clone)]
pub(crate) enum Backend {
    Server(Connection),
    Demo(Demo),
}

/// A backend with the profile of whoever signed in.
pub(crate) struct SignedIn {
    pub(crate) backend: Backend,
    pub(crate) profile: MeResponse,
}

/// How a backend is told apart in the session key and the interface.
pub(crate) const DEMO_ENDPOINT: &str = "demo://workspace";

/// Calls the same method on whichever backend answers, passing the scope's organization and
/// workspace to the HTTP side.
macro_rules! scoped {
    ($backend:expr, $scope:expr, $method:ident ( $($argument:expr),* )) => {
        match $backend {
            Backend::Server(connection) => {
                connection
                    .$method(&$scope.organization, &$scope.workspace_selector $(, $argument)*)
                    .await
            },
            Backend::Demo(demo) => demo.$method($($argument),*),
        }
    };
}

/// Calls the same method on whichever backend answers, for calls outside a workspace.
macro_rules! unscoped {
    ($backend:expr, $method:ident ( $($argument:expr),* )) => {
        match $backend {
            Backend::Server(connection) => connection.$method($($argument),*).await,
            Backend::Demo(demo) => demo.$method($($argument),*),
        }
    };
}

impl Backend {
    /// The address that keys drafts and recent workspaces: the server URL, or the demo marker.
    pub(crate) fn endpoint(&self) -> &str {
        match self {
            Self::Server(connection) => connection.endpoint(),
            Self::Demo(_) => DEMO_ENDPOINT,
        }
    }

    pub(crate) const fn is_demo(&self) -> bool {
        matches!(self, Self::Demo(_))
    }

    /// Signs in to a server, or opens the demo world as its signed-in user.
    pub(crate) async fn sign_in(self, intent: Option<SignIn>) -> Result<SignedIn, Failure> {
        match (self, intent) {
            (Self::Server(connection), Some(intent)) => {
                let signed_in = connection.sign_in(intent).await?;
                Ok(SignedIn {
                    backend: Self::Server(signed_in.connection),
                    profile: signed_in.profile,
                })
            },
            (Self::Demo(demo), None) => Ok(SignedIn {
                profile: demo.me()?,
                backend: Self::Demo(demo),
            }),
            // A server needs credentials and the demo takes none.
            _ => Err(Failure::Configuration),
        }
    }

    pub(crate) async fn workflows(
        &self,
        scope: &SessionContext,
        page: usize,
    ) -> Result<ListWorkflowsResponse, Failure> {
        scoped!(self, scope, list(page))
    }

    pub(crate) async fn workflow(
        &self,
        scope: &SessionContext,
        id: &str,
    ) -> Result<WorkflowDocumentResponse, Failure> {
        scoped!(self, scope, load(id))
    }

    /// Every workflow of the first page with its document, as the triggers page lists them.
    pub(crate) async fn workflow_documents(
        &self,
        scope: &SessionContext,
    ) -> Result<Vec<WorkflowDocumentResponse>, Failure> {
        let page = self.workflows(scope, 1).await?;
        let mut documents = Vec::with_capacity(page.workflows.len());
        for workflow in &page.workflows {
            documents.push(self.workflow(scope, &workflow.id).await?);
        }
        Ok(documents)
    }

    pub(crate) async fn create_workflow(
        &self,
        scope: &SessionContext,
        request: &CreateWorkflowRequest,
    ) -> Result<WorkflowDocumentResponse, Failure> {
        scoped!(self, scope, create(request))
    }

    pub(crate) async fn save_workflow(
        &self,
        scope: &SessionContext,
        id: &str,
        request: &UpdateWorkflowDocumentRequest,
    ) -> Result<WorkflowDocumentResponse, Failure> {
        scoped!(self, scope, save(id, request))
    }

    pub(crate) async fn publish_workflow(
        &self,
        scope: &SessionContext,
        id: &str,
        revision: u64,
    ) -> Result<WorkflowDocumentResponse, Failure> {
        scoped!(self, scope, publish(id, revision))
    }

    pub(crate) async fn start(
        &self,
        scope: &SessionContext,
        workflow: &str,
        key: &str,
    ) -> Result<ExecutionResponse, Failure> {
        scoped!(self, scope, run(workflow, key))
    }

    pub(crate) async fn workflow_history(
        &self,
        scope: &SessionContext,
        workflow: &str,
    ) -> Result<ListExecutionsResponse, Failure> {
        scoped!(self, scope, history(workflow))
    }

    pub(crate) async fn executions(
        &self,
        scope: &SessionContext,
        query: &ExecutionQuery,
    ) -> Result<ListExecutionsResponse, Failure> {
        scoped!(self, scope, executions(query))
    }

    pub(crate) async fn execution(
        &self,
        scope: &SessionContext,
        id: &str,
    ) -> Result<ExecutionDetailResponse, Failure> {
        scoped!(self, scope, status(id))
    }

    pub(crate) async fn cancel(
        &self,
        scope: &SessionContext,
        id: &str,
    ) -> Result<ExecutionResponse, Failure> {
        scoped!(self, scope, cancel(id))
    }

    pub(crate) async fn actions(&self) -> Result<ListActionsResponse, Failure> {
        unscoped!(self, actions())
    }

    pub(crate) async fn action(&self, key: &str) -> Result<ActionDetailResponse, Failure> {
        unscoped!(self, action(key))
    }

    pub(crate) async fn action_parameters(
        &self,
        key: &str,
    ) -> Result<ActionParametersResponse, Failure> {
        unscoped!(self, action_parameters(key))
    }

    pub(crate) async fn credential_types(&self) -> Result<ListCredentialTypesResponse, Failure> {
        unscoped!(self, credential_types())
    }

    pub(crate) async fn credentials(
        &self,
        scope: &SessionContext,
    ) -> Result<ListCredentialsResponse, Failure> {
        scoped!(self, scope, credentials())
    }

    pub(crate) async fn create_credential(
        &self,
        scope: &SessionContext,
        request: &CreateCredentialRequest,
    ) -> Result<CredentialResponse, Failure> {
        scoped!(self, scope, create_credential(request))
    }

    pub(crate) async fn delete_credential(
        &self,
        scope: &SessionContext,
        id: &str,
    ) -> Result<AckResponse, Failure> {
        scoped!(self, scope, delete_credential(id))
    }

    pub(crate) async fn test_credential(
        &self,
        scope: &SessionContext,
        id: &str,
    ) -> Result<TestCredentialResponse, Failure> {
        scoped!(self, scope, test_credential(id))
    }

    pub(crate) async fn register_webhook(
        &self,
        scope: &SessionContext,
        request: &RegisterWebhookRequest,
    ) -> Result<RegisterWebhookResponse, Failure> {
        scoped!(self, scope, register_webhook(request))
    }

    pub(crate) async fn me(&self) -> Result<MeResponse, Failure> {
        unscoped!(self, me())
    }

    pub(crate) async fn update_me(&self, request: &UpdateMeRequest) -> Result<MeResponse, Failure> {
        unscoped!(self, update_me(request))
    }

    pub(crate) async fn tokens(&self) -> Result<MyTokensResponse, Failure> {
        unscoped!(self, tokens())
    }

    pub(crate) async fn create_token(
        &self,
        request: &CreateTokenRequest,
    ) -> Result<CreateTokenResponse, Failure> {
        unscoped!(self, create_token(request))
    }

    pub(crate) async fn revoke_token(&self, id: &str) -> Result<AckResponse, Failure> {
        unscoped!(self, revoke_token(id))
    }

    pub(crate) async fn org_members(
        &self,
        scope: &SessionContext,
    ) -> Result<MembersResponse, Failure> {
        unscoped!(self, org_members(&scope.organization))
    }

    pub(crate) async fn add_org_member(
        &self,
        scope: &SessionContext,
        request: &AddMemberRequest,
    ) -> Result<MemberSummary, Failure> {
        unscoped!(self, add_org_member(&scope.organization, request))
    }

    pub(crate) async fn remove_org_member(
        &self,
        scope: &SessionContext,
        principal: &str,
    ) -> Result<AckResponse, Failure> {
        unscoped!(self, remove_org_member(&scope.organization, principal))
    }

    pub(crate) async fn workspace_members(
        &self,
        scope: &SessionContext,
    ) -> Result<WorkspaceMembersResponse, Failure> {
        scoped!(self, scope, workspace_members())
    }

    pub(crate) async fn set_workspace_member(
        &self,
        scope: &SessionContext,
        principal: &str,
        request: &UpsertWorkspaceMemberRequest,
    ) -> Result<WorkspaceMemberSummary, Failure> {
        scoped!(self, scope, set_workspace_member(principal, request))
    }

    pub(crate) async fn remove_workspace_member(
        &self,
        scope: &SessionContext,
        principal: &str,
    ) -> Result<AckResponse, Failure> {
        scoped!(self, scope, remove_workspace_member(principal))
    }
}
