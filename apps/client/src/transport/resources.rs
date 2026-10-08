//! Workspace resources and the account, beyond the workflow document: execution history and
//! cancellation, credentials, webhook registration, the signed-in user's profile and tokens, and the
//! members of the organization and workspace. Paths and success statuses follow the server's routes.

use super::{Connection, Failure};
use nebula_api_contract::v1::{
    credential::{
        CreateCredentialRequest, CredentialResponse, ListCredentialTypesResponse,
        ListCredentialsResponse, TestCredentialResponse,
    },
    execution::{ExecutionResponse, ListExecutionsResponse},
    me::{CreateTokenRequest, CreateTokenResponse, MeResponse, MyTokensResponse, UpdateMeRequest},
    org::{AddMemberRequest, MemberSummary, MembersResponse},
    shared::AckResponse,
    webhook::{RegisterWebhookRequest, RegisterWebhookResponse},
    workspace_membership::{
        UpsertWorkspaceMemberRequest, WorkspaceMemberSummary, WorkspaceMembersResponse,
    },
};
use serde::de::DeserializeOwned;
use url::Url;

/// Filters of the workspace execution history, newest first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExecutionQuery {
    /// Only executions of this workflow.
    pub(crate) workflow: Option<String>,
    /// Comma-separated statuses, such as `failed,timed_out`; empty for every status.
    pub(crate) statuses: String,
    /// The `next_cursor` of the previous page.
    pub(crate) cursor: Option<String>,
    pub(crate) limit: u32,
}

impl Connection {
    fn workspace_url(&self, org: &str, workspace: &str, rest: &[&str]) -> Result<Url, Failure> {
        let mut segments = vec!["orgs", org, "workspaces", workspace];
        segments.extend_from_slice(rest);
        self.url(&segments)
    }

    /// A mutation without a body. Like every mutation, a lost reply leaves its outcome unknown.
    async fn bodiless<T: DeserializeOwned>(
        &self,
        method: &str,
        url: Url,
        statuses: &[u16],
    ) -> Result<T, Failure> {
        super::decode(
            &self.exchange(method, url, None, None).await?,
            true,
            statuses,
        )
    }

    async fn remove<T: DeserializeOwned>(&self, url: Url, statuses: &[u16]) -> Result<T, Failure> {
        self.bodiless("DELETE", url, statuses).await
    }

    pub(crate) async fn executions(
        &self,
        org: &str,
        workspace: &str,
        query: &ExecutionQuery,
    ) -> Result<ListExecutionsResponse, Failure> {
        let mut url = self.workspace_url(org, workspace, &["executions"])?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("limit", &query.limit.to_string());
            if let Some(workflow) = &query.workflow {
                pairs.append_pair("workflow_id", workflow);
            }
            if !query.statuses.is_empty() {
                pairs.append_pair("status", &query.statuses);
            }
            if let Some(cursor) = &query.cursor {
                pairs.append_pair("cursor", cursor);
            }
        }
        self.read(url).await
    }

    /// Asks the runtime to cancel. The reply is the execution as it stands; the cancellation itself
    /// lands later, under the runtime's own lease.
    pub(crate) async fn cancel(
        &self,
        org: &str,
        workspace: &str,
        execution: &str,
    ) -> Result<ExecutionResponse, Failure> {
        self.remove(
            self.workspace_url(org, workspace, &["executions", execution])?,
            &[202],
        )
        .await
    }

    pub(crate) async fn credential_types(&self) -> Result<ListCredentialTypesResponse, Failure> {
        self.read(self.url(&["credentials", "types"])?).await
    }

    pub(crate) async fn credentials(
        &self,
        org: &str,
        workspace: &str,
    ) -> Result<ListCredentialsResponse, Failure> {
        self.read(self.workspace_url(org, workspace, &["credentials"])?)
            .await
    }

    pub(crate) async fn create_credential(
        &self,
        org: &str,
        workspace: &str,
        request: &CreateCredentialRequest,
    ) -> Result<CredentialResponse, Failure> {
        self.write(
            "POST",
            self.workspace_url(org, workspace, &["credentials"])?,
            request,
            None,
            &[200, 201],
        )
        .await
    }

    pub(crate) async fn delete_credential(
        &self,
        org: &str,
        workspace: &str,
        credential: &str,
    ) -> Result<AckResponse, Failure> {
        self.remove(
            self.workspace_url(org, workspace, &["credentials", credential])?,
            &[200],
        )
        .await
    }

    pub(crate) async fn test_credential(
        &self,
        org: &str,
        workspace: &str,
        credential: &str,
    ) -> Result<TestCredentialResponse, Failure> {
        self.bodiless(
            "POST",
            self.workspace_url(org, workspace, &["credentials", credential, "test"])?,
            &[200],
        )
        .await
    }

    pub(crate) async fn register_webhook(
        &self,
        org: &str,
        workspace: &str,
        request: &RegisterWebhookRequest,
    ) -> Result<RegisterWebhookResponse, Failure> {
        self.write(
            "POST",
            self.workspace_url(org, workspace, &["webhooks"])?,
            request,
            None,
            &[201],
        )
        .await
    }

    pub(crate) async fn me(&self) -> Result<MeResponse, Failure> {
        self.read(self.url(&["me"])?).await
    }

    pub(crate) async fn update_me(&self, request: &UpdateMeRequest) -> Result<MeResponse, Failure> {
        self.write("PATCH", self.url(&["me"])?, request, None, &[200])
            .await
    }

    pub(crate) async fn tokens(&self) -> Result<MyTokensResponse, Failure> {
        self.read(self.url(&["me", "tokens"])?).await
    }

    pub(crate) async fn create_token(
        &self,
        request: &CreateTokenRequest,
    ) -> Result<CreateTokenResponse, Failure> {
        self.write("POST", self.url(&["me", "tokens"])?, request, None, &[201])
            .await
    }

    pub(crate) async fn revoke_token(&self, token: &str) -> Result<AckResponse, Failure> {
        self.remove(self.url(&["me", "tokens", token])?, &[200])
            .await
    }

    pub(crate) async fn org_members(&self, org: &str) -> Result<MembersResponse, Failure> {
        self.read(self.url(&["orgs", org, "members"])?).await
    }

    pub(crate) async fn add_org_member(
        &self,
        org: &str,
        request: &AddMemberRequest,
    ) -> Result<MemberSummary, Failure> {
        self.write(
            "POST",
            self.url(&["orgs", org, "members"])?,
            request,
            None,
            &[201],
        )
        .await
    }

    pub(crate) async fn remove_org_member(
        &self,
        org: &str,
        principal: &str,
    ) -> Result<AckResponse, Failure> {
        self.remove(self.url(&["orgs", org, "members", principal])?, &[200])
            .await
    }

    pub(crate) async fn workspace_members(
        &self,
        org: &str,
        workspace: &str,
    ) -> Result<WorkspaceMembersResponse, Failure> {
        self.read(self.workspace_url(org, workspace, &["members"])?)
            .await
    }

    pub(crate) async fn set_workspace_member(
        &self,
        org: &str,
        workspace: &str,
        principal: &str,
        request: &UpsertWorkspaceMemberRequest,
    ) -> Result<WorkspaceMemberSummary, Failure> {
        self.write(
            "PUT",
            self.workspace_url(org, workspace, &["members", principal])?,
            request,
            None,
            &[200],
        )
        .await
    }

    pub(crate) async fn remove_workspace_member(
        &self,
        org: &str,
        workspace: &str,
        principal: &str,
    ) -> Result<AckResponse, Failure> {
        self.remove(
            self.workspace_url(org, workspace, &["members", principal])?,
            &[200],
        )
        .await
    }
}
