//! Exhaustive named wire-schema decoding; unknown contracts fail the gate.
use nebula_api_contract::v1;
use serde::de::DeserializeOwned;
use serde_json::Value;

pub(super) fn decode(name: &str, value: Value) -> Option<bool> {
    fn decode<T: DeserializeOwned>(value: Value) -> Option<bool> {
        Some(serde_json::from_value::<T>(value).is_ok())
    }
    match name {
        "SignupRequest" => decode::<v1::auth::SignupRequest>(value),
        "LoginRequest" => decode::<v1::auth::LoginRequest>(value),
        "ForgotPasswordRequest" => decode::<v1::auth::ForgotPasswordRequest>(value),
        "ResetPasswordRequest" => decode::<v1::auth::ResetPasswordRequest>(value),
        "VerifyEmailRequest" => decode::<v1::auth::VerifyEmailRequest>(value),
        "MfaEnrollRequest" => decode::<v1::auth::MfaEnrollRequest>(value),
        "MfaConfirmEnrollRequest" => decode::<v1::auth::MfaConfirmEnrollRequest>(value),
        "MfaLoginCompleteRequest" => decode::<v1::auth::MfaLoginCompleteRequest>(value),
        "LoginResponse" => decode::<v1::auth::LoginResponse>(value),
        "MfaChallengeResponse" => decode::<v1::auth::MfaChallengeResponse>(value),
        "SignupResponse" => decode::<v1::auth::SignupResponse>(value),
        "MfaEnrollResponse" => decode::<v1::auth::MfaEnrollResponse>(value),
        "OAuthStartResponse" => decode::<v1::auth::OAuthStartResponse>(value),
        "UserProfile" => decode::<v1::auth::UserProfile>(value),
        "OAuthProvider" => decode::<v1::auth::OAuthProvider>(value),
        "OAuthCallbackParams" => decode::<v1::auth::OAuthCallbackParams>(value),
        "ActionSummary" => decode::<v1::catalog::ActionSummary>(value),
        "ListActionsResponse" => decode::<v1::catalog::ListActionsResponse>(value),
        "ActionDetailResponse" => decode::<v1::catalog::ActionDetailResponse>(value),
        "PluginSummary" => decode::<v1::catalog::PluginSummary>(value),
        "ListPluginsResponse" => decode::<v1::catalog::ListPluginsResponse>(value),
        "PluginDetailResponse" => decode::<v1::catalog::PluginDetailResponse>(value),
        "CredentialCapabilities" => decode::<v1::credential::CredentialCapabilities>(value),
        "CreateCredentialRequest" => decode::<v1::credential::CreateCredentialRequest>(value),
        "UpdateCredentialRequest" => decode::<v1::credential::UpdateCredentialRequest>(value),
        "CredentialResponse" => decode::<v1::credential::CredentialResponse>(value),
        "CredentialSummary" => decode::<v1::credential::CredentialSummary>(value),
        "CredentialLifecycleState" => decode::<v1::credential::CredentialLifecycleState>(value),
        "ListCredentialsResponse" => decode::<v1::credential::ListCredentialsResponse>(value),
        "ListCredentialsQuery" => decode::<v1::credential::ListCredentialsQuery>(value),
        "ReauthorizeCredentialRequest" => {
            decode::<v1::credential::ReauthorizeCredentialRequest>(value)
        },
        "ReauthorizeCredentialResponse" => {
            decode::<v1::credential::ReauthorizeCredentialResponse>(value)
        },
        "ResolveCredentialRequest" => decode::<v1::credential::ResolveCredentialRequest>(value),
        "FormPostField" => decode::<v1::credential::FormPostField>(value),
        "AcquisitionInteraction" => decode::<v1::credential::AcquisitionInteraction>(value),
        "ResolveCredentialResponse" => decode::<v1::credential::ResolveCredentialResponse>(value),
        "ContinueResolveRequest" => decode::<v1::credential::ContinueResolveRequest>(value),
        "ContinueResolveResponse" => decode::<v1::credential::ContinueResolveResponse>(value),
        "CredentialTestFailureCodeV1" => {
            decode::<v1::credential::CredentialTestFailureCodeV1>(value)
        },
        "TestCredentialResponse" => decode::<v1::credential::TestCredentialResponse>(value),
        "RefreshCredentialResponse" => decode::<v1::credential::RefreshCredentialResponse>(value),
        "RevokeCredentialResponse" => decode::<v1::credential::RevokeCredentialResponse>(value),
        "CredentialReconcileOperationV1" => {
            decode::<v1::credential::CredentialReconcileOperationV1>(value)
        },
        "CredentialReconcileDecisionV1" => {
            decode::<v1::credential::CredentialReconcileDecisionV1>(value)
        },
        "ReconcileCredentialRequest" => decode::<v1::credential::ReconcileCredentialRequest>(value),
        "ReconcileCredentialResponse" => {
            decode::<v1::credential::ReconcileCredentialResponse>(value)
        },
        "CredentialTypeInfo" => decode::<v1::credential::CredentialTypeInfo>(value),
        "ListCredentialTypesResponse" => {
            decode::<v1::credential::ListCredentialTypesResponse>(value)
        },
        "StartExecutionRequest" => decode::<v1::execution::StartExecutionRequest>(value),
        "ExecutionResponse" => decode::<v1::execution::ExecutionResponse>(value),
        "ExecutionStatus" => decode::<v1::execution::ExecutionStatus>(value),
        "ExecutionSummary" => decode::<v1::execution::ExecutionSummary>(value),
        "ListExecutionsResponse" => decode::<v1::execution::ListExecutionsResponse>(value),
        "ExecutionDetailResponse" => decode::<v1::execution::ExecutionDetailResponse>(value),
        "ExecutionLogEntry" => decode::<v1::execution::ExecutionLogEntry>(value),
        "ExecutionLogsResponse" => decode::<v1::execution::ExecutionLogsResponse>(value),
        "HealthResponse" => decode::<v1::health::HealthResponse>(value),
        "ReadinessResponse" => decode::<v1::health::ReadinessResponse>(value),
        "DependenciesStatus" => decode::<v1::health::DependenciesStatus>(value),
        "VersionInfo" => decode::<v1::health::VersionInfo>(value),
        "MeResponse" => decode::<v1::me::MeResponse>(value),
        "UpdateMeRequest" => decode::<v1::me::UpdateMeRequest>(value),
        "OrgSummary" => decode::<v1::me::OrgSummary>(value),
        "MyOrgsResponse" => decode::<v1::me::MyOrgsResponse>(value),
        "TokenSummary" => decode::<v1::me::TokenSummary>(value),
        "MyTokensResponse" => decode::<v1::me::MyTokensResponse>(value),
        "CreateTokenRequest" => decode::<v1::me::CreateTokenRequest>(value),
        "CreateTokenResponse" => decode::<v1::me::CreateTokenResponse>(value),
        "OrgResponse" => decode::<v1::org::OrgResponse>(value),
        "UpdateOrgRequest" => decode::<v1::org::UpdateOrgRequest>(value),
        "MemberSummary" => decode::<v1::org::MemberSummary>(value),
        "MembersResponse" => decode::<v1::org::MembersResponse>(value),
        "AddMemberRequest" => decode::<v1::org::AddMemberRequest>(value),
        "ServiceAccountSummary" => decode::<v1::org::ServiceAccountSummary>(value),
        "ServiceAccountsResponse" => decode::<v1::org::ServiceAccountsResponse>(value),
        "CreateServiceAccountRequest" => decode::<v1::org::CreateServiceAccountRequest>(value),
        "CreateServiceAccountResponse" => decode::<v1::org::CreateServiceAccountResponse>(value),
        "ProblemDetails" => decode::<v1::problem::ProblemDetails>(value),
        "ValidationFieldError" => decode::<v1::problem::ValidationFieldError>(value),
        "ResourceSummary" => decode::<v1::resource::ResourceSummary>(value),
        "ListResourcesResponse" => decode::<v1::resource::ListResourcesResponse>(value),
        "CreateResourceRequest" => decode::<v1::resource::CreateResourceRequest>(value),
        "CreateResourceResponse" => decode::<v1::resource::CreateResourceResponse>(value),
        "UpdateResourceRequest" => decode::<v1::resource::UpdateResourceRequest>(value),
        "UpdateResourceResponse" => decode::<v1::resource::UpdateResourceResponse>(value),
        "ResourcePhase" => decode::<v1::resource::ResourcePhase>(value),
        "ResourceStatusDto" => decode::<v1::resource::ResourceStatusDto>(value),
        "CursorParams" => decode::<v1::shared::CursorParams>(value),
        "PaginationParams" => decode::<v1::shared::PaginationParams>(value),
        "AckResponse" => decode::<v1::shared::AckResponse>(value),
        "OrgRoleDto" => decode::<v1::shared::OrgRoleDto>(value),
        "WorkspaceRoleDto" => decode::<v1::shared::WorkspaceRoleDto>(value),
        "RegisterWebhookRequest" => decode::<v1::webhook::RegisterWebhookRequest>(value),
        "RegisterWebhookResponse" => decode::<v1::webhook::RegisterWebhookResponse>(value),
        "CreateWorkflowRequest" => decode::<v1::workflow::CreateWorkflowRequest>(value),
        "UpdateWorkflowRequest" => decode::<v1::workflow::UpdateWorkflowRequest>(value),
        "UpdateWorkflowDocumentRequest" => {
            decode::<v1::workflow::UpdateWorkflowDocumentRequest>(value)
        },
        "WorkflowResponse" => decode::<v1::workflow::WorkflowResponse>(value),
        "WorkflowDocumentResponse" => decode::<v1::workflow::WorkflowDocumentResponse>(value),
        "ListWorkflowsResponse" => decode::<v1::workflow::ListWorkflowsResponse>(value),
        "WorkflowValidateResponse" => decode::<v1::workflow::WorkflowValidateResponse>(value),
        "WorkspaceMemberSummary" => {
            decode::<v1::workspace_membership::WorkspaceMemberSummary>(value)
        },
        "WorkspaceMembersResponse" => {
            decode::<v1::workspace_membership::WorkspaceMembersResponse>(value)
        },
        "UpsertWorkspaceMemberRequest" => {
            decode::<v1::workspace_membership::UpsertWorkspaceMemberRequest>(value)
        },
        _ => None,
    }
}
