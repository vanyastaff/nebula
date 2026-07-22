//! First-party credential authority and API gateway composition.
//!
//! This module is the trust bridge between Plane-A authentication and the
//! credential bounded context. API handlers submit only API-owned intent; this
//! adapter maps middleware-authenticated claims into credential actor claims,
//! and the credential controller asks the injected tenant authority exactly
//! once before deriving an owner partition or touching persistence.

use std::{collections::BTreeMap, fmt, sync::Arc};

use async_trait::async_trait;
use nebula_api::{
    error::ApiError,
    middleware::auth::{AuthenticatedPrincipal, AuthenticatedPrincipalKind},
    ports::credential_command::{
        CredentialCommandGateway, CredentialGatewayAcquisition, CredentialGatewayCommand,
        CredentialGatewayError, CredentialGatewayRecord, CredentialGatewayResult,
        CredentialGatewayTestFailure, CredentialGatewayTestResult,
        CredentialGatewayValidationIssue, CredentialGatewayValidationReport,
    },
    ports::credential_schema::{CredentialValidationCode, CredentialValidationLocation},
    state::{MembershipStore, WorkspaceResolver},
};
use nebula_core::{
    OrgId, Permission, Principal as CorePrincipal, ServiceAccountId, TenantContext, UserId,
    WorkspaceGrant, WorkspaceId, effective_workspace_role,
};
use nebula_credential::{
    Acquisition, AuthorizationDecision, CredentialActor, CredentialActorKind,
    CredentialAuthenticationBinding, CredentialAuthorizationError, CredentialCommand,
    CredentialCommandResult, CredentialController, CredentialControllerError, CredentialDisplay,
    CredentialDisplayPatch, CredentialOperation, CredentialServiceError, CredentialTenantAuthority,
    InteractionRequest, TestFailureCode, UserInput,
};
use nebula_storage_port::Scope;
use nebula_tenancy::{BindingScopeResolver, Principal as TenantPrincipal, ScopeResolver as _};

use nebula_api::domain::credential::dto::{AcquisitionInteraction, FormPostField};

/// First-party tenant authority for credential management commands.
///
/// Plane-A middleware has already authenticated the actor. This authority
/// independently reads the same membership source as HTTP RBAC, applies the
/// operation's credential permission, validates every identifier as a typed
/// Nebula ID, and requires the tenancy resolver to reproduce the exact scope.
/// An unwired or failed policy source returns unavailable. A valid snapshot
/// without organization membership, plus system/workflow actors, is denied.
pub(crate) struct ServerCredentialAuthority {
    resolver: BindingScopeResolver,
    membership_store: Option<Arc<dyn MembershipStore>>,
    workspace_resolver: Option<Arc<dyn WorkspaceResolver>>,
}

impl ServerCredentialAuthority {
    /// Bind tenant authorization to the same membership source used by HTTP
    /// RBAC. An absent source is retained only so unprovisioned composition
    /// can fail with `Unavailable`; it never grants access.
    pub(crate) fn new(
        membership_store: Option<Arc<dyn MembershipStore>>,
        workspace_resolver: Option<Arc<dyn WorkspaceResolver>>,
    ) -> Self {
        Self {
            resolver: BindingScopeResolver,
            membership_store,
            workspace_resolver,
        }
    }

    const fn permission(operation: CredentialOperation) -> Option<Permission> {
        match operation {
            CredentialOperation::Get | CredentialOperation::List => {
                Some(Permission::CredentialRead)
            },
            CredentialOperation::Create
            | CredentialOperation::Update
            | CredentialOperation::Test
            | CredentialOperation::Refresh
            | CredentialOperation::Resolve
            | CredentialOperation::ContinueResolve => Some(Permission::CredentialWrite),
            CredentialOperation::Delete | CredentialOperation::Revoke => {
                Some(Permission::CredentialDelete)
            },
            _ => None,
        }
    }
}

impl fmt::Debug for ServerCredentialAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerCredentialAuthority")
            .field("membership_store_wired", &self.membership_store.is_some())
            .field(
                "workspace_resolver_wired",
                &self.workspace_resolver.is_some(),
            )
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl CredentialTenantAuthority for ServerCredentialAuthority {
    async fn decide(
        &self,
        actor: &CredentialActor,
        scope: &Scope,
        operation: CredentialOperation,
    ) -> Result<AuthorizationDecision, CredentialAuthorizationError> {
        let org_id =
            OrgId::parse(&scope.org_id).map_err(|_| CredentialAuthorizationError::InvalidScope)?;
        let workspace_id = WorkspaceId::parse(&scope.workspace_id)
            .map_err(|_| CredentialAuthorizationError::InvalidScope)?;

        let workspace_resolver = self
            .workspace_resolver
            .as_ref()
            .ok_or(CredentialAuthorizationError::Unavailable)?;
        match workspace_resolver.resolve_by_id(org_id, workspace_id).await {
            Ok(resolved) if resolved == workspace_id => {},
            Ok(_) | Err(ApiError::NotFound(_)) => return Ok(AuthorizationDecision::Deny),
            Err(_) => return Err(CredentialAuthorizationError::Unavailable),
        }

        let core_actor = match actor.kind() {
            CredentialActorKind::User => UserId::parse(actor.subject())
                .map(CorePrincipal::User)
                .map_err(|_| CredentialAuthorizationError::Denied)?,
            CredentialActorKind::ServiceAccount => ServiceAccountId::parse(actor.subject())
                .map(CorePrincipal::ServiceAccount)
                .map_err(|_| CredentialAuthorizationError::Denied)?,
            CredentialActorKind::Workflow => return Ok(AuthorizationDecision::Deny),
            _ => return Ok(AuthorizationDecision::Deny),
        };

        let membership_store = self
            .membership_store
            .as_ref()
            .ok_or(CredentialAuthorizationError::Unavailable)?;
        let membership = membership_store
            .get_tenant_membership(org_id, Some(workspace_id), &core_actor)
            .await
            .map_err(|_| {
                tracing::warn!(
                    ?operation,
                    stage = "tenant_membership",
                    "credential authority lookup failed"
                );
                CredentialAuthorizationError::Unavailable
            })?;
        if membership.org_role.is_none() {
            return Ok(AuthorizationDecision::Deny);
        }
        let workspace_role =
            effective_workspace_role(membership.org_role, membership.workspace_role)
                .map(|role| WorkspaceGrant::new(workspace_id, role));
        let tenant = TenantContext {
            org_id,
            workspace_id: Some(workspace_id),
            principal: core_actor.clone(),
            org_role: membership.org_role,
            workspace_role,
        };
        let Some(permission) = Self::permission(operation) else {
            return Ok(AuthorizationDecision::Deny);
        };
        if tenant.require(permission).is_err() {
            return Ok(AuthorizationDecision::Deny);
        }

        let binding = TenantPrincipal::workspace(core_actor, org_id, workspace_id);
        let resolved = self
            .resolver
            .resolve(&binding)
            .map_err(|_| CredentialAuthorizationError::Denied)?;
        if resolved != *scope {
            tracing::warn!(?operation, "credential tenant binding mismatch");
            return Ok(AuthorizationDecision::Deny);
        }
        Ok(AuthorizationDecision::Allow)
    }
}

/// First-party adapter from the API-owned command port to the
/// credential-owned authority/controller.
pub(crate) struct ServerCredentialGateway {
    controller: Arc<CredentialController>,
}

impl ServerCredentialGateway {
    /// Bind a controller for the process lifetime.
    pub(crate) fn new(controller: Arc<CredentialController>) -> Self {
        Self { controller }
    }

    fn actor(
        principal: &AuthenticatedPrincipal,
    ) -> Result<CredentialActor, CredentialGatewayError> {
        let actor = match principal.kind() {
            AuthenticatedPrincipalKind::User => CredentialActor::user(principal.subject()),
            AuthenticatedPrincipalKind::ServiceAccount => {
                CredentialActor::service_account(principal.subject())
            },
            AuthenticatedPrincipalKind::Workflow => CredentialActor::workflow(principal.subject()),
            AuthenticatedPrincipalKind::System => return Err(CredentialGatewayError::Forbidden),
            _ => return Err(CredentialGatewayError::Forbidden),
        };
        actor.map_err(|_| CredentialGatewayError::Forbidden)
    }

    fn command(
        principal: &AuthenticatedPrincipal,
        command: CredentialGatewayCommand,
    ) -> Result<CredentialCommand, CredentialGatewayError> {
        let authentication_binding =
            CredentialAuthenticationBinding::parse(principal.authentication_binding())
                .map_err(|_| CredentialGatewayError::Internal)?;
        let command =
            match command {
                CredentialGatewayCommand::Create(request) => CredentialCommand::Create {
                    credential_key: request.credential_key,
                    properties: request.data,
                    display: CredentialDisplay {
                        display_name: Some(request.name),
                        description: request.description,
                        tags: request.tags.unwrap_or_default().into_iter().collect(),
                    },
                },
                CredentialGatewayCommand::Get { credential_id } => {
                    CredentialCommand::Get { credential_id }
                },
                CredentialGatewayCommand::List => CredentialCommand::List,
                CredentialGatewayCommand::Update {
                    credential_id,
                    request,
                } => CredentialCommand::Update {
                    credential_id,
                    properties: request.data,
                    expected_version: request.version,
                    display: CredentialDisplayPatch {
                        display_name: request.name,
                        description: request.description,
                        tags: request
                            .tags
                            .map(|tags| tags.into_iter().collect::<BTreeMap<_, _>>()),
                    },
                },
                CredentialGatewayCommand::Delete { credential_id } => {
                    CredentialCommand::Delete { credential_id }
                },
                CredentialGatewayCommand::Test { credential_id } => {
                    CredentialCommand::Test { credential_id }
                },
                CredentialGatewayCommand::Refresh { credential_id } => {
                    CredentialCommand::Refresh { credential_id }
                },
                CredentialGatewayCommand::Revoke { credential_id } => {
                    CredentialCommand::Revoke { credential_id }
                },
                CredentialGatewayCommand::Resolve(request) => CredentialCommand::Resolve {
                    credential_key: request.credential_key,
                    properties: request.data,
                    authentication_binding,
                },
                CredentialGatewayCommand::ContinueResolve(request) => {
                    let user_input: UserInput = serde_json::from_value(request.user_input)
                        .map_err(|_| CredentialGatewayError::ValidationFailed {
                            report: CredentialGatewayValidationReport::single(
                                CredentialValidationLocation::UserInput,
                                CredentialValidationCode::UserInputInvalid,
                            ),
                        })?;
                    CredentialCommand::ContinueResolve {
                        credential_key: request.credential_key,
                        pending_token: request.pending_token,
                        user_input,
                        authentication_binding,
                    }
                },
                _ => return Err(CredentialGatewayError::Internal),
            };
        Ok(command)
    }

    fn result(
        result: CredentialCommandResult,
    ) -> Result<CredentialGatewayResult, CredentialGatewayError> {
        let result = match result {
            CredentialCommandResult::Head(head) => CredentialGatewayResult::Record(map_head(head)),
            CredentialCommandResult::Heads(heads) => {
                CredentialGatewayResult::Records(heads.into_iter().map(map_head).collect())
            },
            CredentialCommandResult::Deleted => CredentialGatewayResult::Deleted,
            CredentialCommandResult::Tested(result) => {
                CredentialGatewayResult::Tested(map_test_result(result))
            },
            CredentialCommandResult::Refreshed(report) => CredentialGatewayResult::Refreshed {
                record: map_head(report.head),
                refreshed: report.refreshed,
            },
            CredentialCommandResult::Revoked => CredentialGatewayResult::Revoked,
            CredentialCommandResult::Acquisition(acquisition) => {
                CredentialGatewayResult::Acquisition(map_acquisition(acquisition)?)
            },
            _ => return Err(CredentialGatewayError::Internal),
        };
        Ok(result)
    }
}

impl fmt::Debug for ServerCredentialGateway {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServerCredentialGateway")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl CredentialCommandGateway for ServerCredentialGateway {
    async fn execute(
        &self,
        principal: &AuthenticatedPrincipal,
        scope: &Scope,
        command: CredentialGatewayCommand,
    ) -> Result<CredentialGatewayResult, CredentialGatewayError> {
        let actor = Self::actor(principal)?;
        let command = Self::command(principal, command)?;
        let result = self
            .controller
            .execute(&actor, scope, command)
            .await
            .map_err(map_controller_error)?;
        Self::result(result)
    }
}

fn map_head(head: nebula_credential::CredentialHead) -> CredentialGatewayRecord {
    CredentialGatewayRecord {
        id: head.id,
        credential_key: head.credential_key,
        version: head.version,
        created_at: head.created_at,
        updated_at: head.updated_at,
        expires_at: head.expires_at,
        reauth_required: head.reauth_required,
        display_name: head.display.display_name,
        description: head.display.description,
        tags: head.display.tags,
    }
}

fn map_test_result(result: nebula_credential::TestResult) -> CredentialGatewayTestResult {
    match result {
        nebula_credential::TestResult::Success => CredentialGatewayTestResult::Success,
        nebula_credential::TestResult::Failed { code } => {
            CredentialGatewayTestResult::Failed(match code {
                TestFailureCode::AuthenticationRejected => {
                    CredentialGatewayTestFailure::AuthenticationRejected
                },
                TestFailureCode::PermissionDenied => CredentialGatewayTestFailure::PermissionDenied,
                TestFailureCode::AccountRestricted => {
                    CredentialGatewayTestFailure::AccountRestricted
                },
                TestFailureCode::InvalidConfiguration => {
                    CredentialGatewayTestFailure::InvalidConfiguration
                },
                TestFailureCode::Other => CredentialGatewayTestFailure::Other,
                _ => CredentialGatewayTestFailure::Other,
            })
        },
        _ => CredentialGatewayTestResult::Failed(CredentialGatewayTestFailure::Other),
    }
}

fn map_acquisition(
    acquisition: Acquisition,
) -> Result<CredentialGatewayAcquisition, CredentialGatewayError> {
    match acquisition {
        Acquisition::Complete { head } => Ok(CredentialGatewayAcquisition::Complete {
            credential_id: head.id,
        }),
        Acquisition::Pending { token, interaction } => Ok(CredentialGatewayAcquisition::Pending {
            pending_token: token,
            interaction: map_interaction(interaction)?,
        }),
        Acquisition::Retry { after } => Ok(CredentialGatewayAcquisition::Retry {
            retry_after_secs: after.as_secs(),
        }),
        _ => Err(CredentialGatewayError::Internal),
    }
}

fn map_interaction(
    interaction: InteractionRequest,
) -> Result<AcquisitionInteraction, CredentialGatewayError> {
    match interaction {
        InteractionRequest::Redirect { url } => Ok(AcquisitionInteraction::Redirect { url }),
        InteractionRequest::FormPost { url, fields } => Ok(AcquisitionInteraction::FormPost {
            url,
            fields: fields
                .into_iter()
                .map(|(name, value)| FormPostField { name, value })
                .collect(),
        }),
        InteractionRequest::DisplayInfo {
            title,
            message,
            data,
            expires_in,
        } => Ok(AcquisitionInteraction::DisplayInfo {
            title,
            message,
            data: serde_json::to_value(data).map_err(|_| CredentialGatewayError::Internal)?,
            expires_in,
        }),
        _ => Err(CredentialGatewayError::Internal),
    }
}

fn map_controller_error(error: CredentialControllerError) -> CredentialGatewayError {
    match error {
        CredentialControllerError::Authorization(error) => match error {
            CredentialAuthorizationError::Denied | CredentialAuthorizationError::InvalidScope => {
                CredentialGatewayError::Forbidden
            },
            CredentialAuthorizationError::Unavailable => CredentialGatewayError::Unavailable,
            _ => CredentialGatewayError::Internal,
        },
        CredentialControllerError::Service(error) => map_service_error(error),
        _ => CredentialGatewayError::Internal,
    }
}

fn map_service_error(error: CredentialServiceError) -> CredentialGatewayError {
    match error {
        CredentialServiceError::NotFound { .. } => CredentialGatewayError::NotFound,
        CredentialServiceError::VersionConflict {
            expected, actual, ..
        } => CredentialGatewayError::VersionConflict { expected, actual },
        CredentialServiceError::ValidationFailed { report } => {
            CredentialGatewayError::ValidationFailed {
                report: CredentialGatewayValidationReport::new(
                    CredentialGatewayValidationIssue::new(
                        CredentialValidationLocation::Data,
                        CredentialValidationCode::from_untrusted(report.primary().code()),
                    ),
                    report
                        .issues()
                        .skip(1)
                        .map(|issue| {
                            CredentialGatewayValidationIssue::new(
                                CredentialValidationLocation::Data,
                                CredentialValidationCode::from_untrusted(issue.code()),
                            )
                        })
                        .collect(),
                ),
            }
        },
        CredentialServiceError::TypeUnknown { key } => CredentialGatewayError::TypeUnknown { key },
        CredentialServiceError::CapabilityUnsupported { capability, key } => {
            CredentialGatewayError::CapabilityUnsupported { capability, key }
        },
        CredentialServiceError::PendingExpired => CredentialGatewayError::PendingExpired,
        CredentialServiceError::ReauthRequired { .. } => CredentialGatewayError::ReauthRequired,
        CredentialServiceError::TransientProvider(_)
        | CredentialServiceError::Provider(_)
        | CredentialServiceError::ExternalSourceNotWired { .. } => {
            CredentialGatewayError::Unavailable
        },
        CredentialServiceError::Store(_)
        | CredentialServiceError::SessionRequired { .. }
        | CredentialServiceError::CapabilityWithoutOps { .. }
        | CredentialServiceError::Internal(_)
        | CredentialServiceError::Cancelled
        | CredentialServiceError::ScopeViolation { .. } => CredentialGatewayError::Internal,
        _ => CredentialGatewayError::Internal,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use nebula_api::{
        domain::org::InMemoryMembershipStore,
        error::ApiError,
        state::{AddMemberOutcome, OrgMember, RemoveMemberOutcome, TenantMembershipSnapshot},
    };
    use nebula_core::{OrgRole, WorkspaceRole};
    use nebula_storage::credential::EnvKeyProvider;

    use super::*;

    const TEST_KEY_B64: &str = "QkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkI=";

    #[derive(Debug)]
    struct CountingAuthority {
        calls: Arc<AtomicUsize>,
        decision: AuthorizationDecision,
    }

    #[derive(Debug, Default)]
    struct RecordingDenyAuthority {
        operations: Mutex<Vec<CredentialOperation>>,
    }

    #[async_trait]
    impl CredentialTenantAuthority for RecordingDenyAuthority {
        async fn decide(
            &self,
            _actor: &CredentialActor,
            _scope: &Scope,
            operation: CredentialOperation,
        ) -> Result<AuthorizationDecision, CredentialAuthorizationError> {
            self.operations
                .lock()
                .expect("test operation lock")
                .push(operation);
            Ok(AuthorizationDecision::Deny)
        }
    }

    #[derive(Debug)]
    struct RecordingMembershipStore {
        snapshot: TenantMembershipSnapshot,
        fail_snapshot: bool,
        snapshot_calls: AtomicUsize,
        point_calls: AtomicUsize,
        principal: Mutex<Option<CorePrincipal>>,
    }

    #[derive(Debug)]
    struct ExactWorkspaceResolver {
        org_id: OrgId,
        workspace_id: WorkspaceId,
        calls: AtomicUsize,
    }

    impl ExactWorkspaceResolver {
        fn new(org_id: OrgId, workspace_id: WorkspaceId) -> Arc<Self> {
            Arc::new(Self {
                org_id,
                workspace_id,
                calls: AtomicUsize::new(0),
            })
        }
    }

    fn workspace_resolver(
        org_id: OrgId,
        workspace_id: WorkspaceId,
    ) -> Option<Arc<dyn WorkspaceResolver>> {
        Some(ExactWorkspaceResolver::new(org_id, workspace_id))
    }

    #[async_trait]
    impl WorkspaceResolver for ExactWorkspaceResolver {
        async fn resolve_by_slug(
            &self,
            _org_id: OrgId,
            _slug: &str,
        ) -> Result<WorkspaceId, ApiError> {
            Err(ApiError::NotFound("workspace not found".to_owned()))
        }

        async fn resolve_by_id(
            &self,
            org_id: OrgId,
            workspace_id: WorkspaceId,
        ) -> Result<WorkspaceId, ApiError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if org_id == self.org_id && workspace_id == self.workspace_id {
                Ok(workspace_id)
            } else {
                Err(ApiError::NotFound("workspace not found".to_owned()))
            }
        }
    }

    impl RecordingMembershipStore {
        fn new(org_role: Option<OrgRole>, workspace_role: Option<WorkspaceRole>) -> Self {
            Self {
                snapshot: TenantMembershipSnapshot {
                    org_role,
                    workspace_role,
                },
                fail_snapshot: false,
                snapshot_calls: AtomicUsize::new(0),
                point_calls: AtomicUsize::new(0),
                principal: Mutex::new(None),
            }
        }

        fn failing() -> Self {
            Self {
                fail_snapshot: true,
                ..Self::new(
                    Some(OrgRole::OrgMember),
                    Some(WorkspaceRole::WorkspaceEditor),
                )
            }
        }
    }

    #[async_trait]
    impl MembershipStore for RecordingMembershipStore {
        async fn get_tenant_membership(
            &self,
            _org_id: OrgId,
            _workspace_id: Option<WorkspaceId>,
            principal: &CorePrincipal,
        ) -> Result<TenantMembershipSnapshot, ApiError> {
            self.snapshot_calls.fetch_add(1, Ordering::SeqCst);
            *self.principal.lock().expect("test principal lock") = Some(principal.clone());
            if self.fail_snapshot {
                Err(ApiError::ServiceUnavailable(
                    "test membership unavailable".to_owned(),
                ))
            } else {
                Ok(self.snapshot)
            }
        }

        async fn get_org_role(
            &self,
            _org_id: OrgId,
            _principal: &CorePrincipal,
        ) -> Result<Option<OrgRole>, ApiError> {
            self.point_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.snapshot.org_role)
        }

        async fn get_workspace_role(
            &self,
            _workspace_id: WorkspaceId,
            _principal: &CorePrincipal,
        ) -> Result<Option<WorkspaceRole>, ApiError> {
            self.point_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.snapshot.workspace_role)
        }

        async fn list_members(&self, _org_id: OrgId) -> Result<Vec<OrgMember>, ApiError> {
            Ok(Vec::new())
        }

        async fn add_member(
            &self,
            _org_id: OrgId,
            _principal: &CorePrincipal,
            _role: OrgRole,
        ) -> Result<(), ApiError> {
            Ok(())
        }

        async fn remove_member(
            &self,
            _org_id: OrgId,
            _principal: &CorePrincipal,
        ) -> Result<bool, ApiError> {
            Ok(false)
        }

        async fn add_member_guarded(
            &self,
            _org_id: OrgId,
            _principal: &CorePrincipal,
            _role: OrgRole,
        ) -> Result<AddMemberOutcome, ApiError> {
            Ok(AddMemberOutcome::Added)
        }

        async fn remove_member_guarded(
            &self,
            _org_id: OrgId,
            _principal: &CorePrincipal,
        ) -> Result<RemoveMemberOutcome, ApiError> {
            Ok(RemoveMemberOutcome::NotFound)
        }

        async fn list_orgs_for_principal(
            &self,
            _principal: &CorePrincipal,
        ) -> Result<Vec<(OrgId, OrgRole)>, ApiError> {
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl CredentialTenantAuthority for CountingAuthority {
        async fn decide(
            &self,
            _actor: &CredentialActor,
            _scope: &Scope,
            _operation: CredentialOperation,
        ) -> Result<AuthorizationDecision, CredentialAuthorizationError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.decision)
        }
    }

    async fn service() -> Arc<nebula_credential::CredentialService> {
        let key =
            Arc::new(EnvKeyProvider::from_base64(TEST_KEY_B64).expect("valid fixed test key"));
        crate::credential_composition::compose_memory_service(key)
            .await
            .expect("test credential service composes")
    }

    fn actor() -> CredentialActor {
        CredentialActor::user(UserId::new().to_string()).expect("typed user subject")
    }

    #[tokio::test]
    async fn controller_obtains_exactly_one_decision_for_an_allowed_command() {
        let calls = Arc::new(AtomicUsize::new(0));
        let authority: Arc<dyn CredentialTenantAuthority> = Arc::new(CountingAuthority {
            calls: Arc::clone(&calls),
            decision: AuthorizationDecision::Allow,
        });
        let controller = CredentialController::new(service().await, authority);

        let result = controller
            .execute(
                &actor(),
                &Scope::new("workspace", "org"),
                CredentialCommand::List,
            )
            .await
            .expect("allowed list executes");

        assert!(matches!(result, CredentialCommandResult::Heads(rows) if rows.is_empty()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn production_gateway_propagates_authenticated_list_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let authority: Arc<dyn CredentialTenantAuthority> = Arc::new(CountingAuthority {
            calls: Arc::clone(&calls),
            decision: AuthorizationDecision::Allow,
        });
        let gateway = ServerCredentialGateway::new(Arc::new(CredentialController::new(
            service().await,
            authority,
        )));
        let subject = UserId::new();
        let principal = AuthenticatedPrincipal::for_test_user(subject.to_string());

        let result = gateway
            .execute(
                &principal,
                &Scope::new(WorkspaceId::new().to_string(), OrgId::new().to_string()),
                CredentialGatewayCommand::List,
            )
            .await
            .expect("gateway list executes");

        assert!(matches!(result, CredentialGatewayResult::Records(rows) if rows.is_empty()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn production_gateway_preserves_actor_and_opaque_interactive_binding() {
        let subject = UserId::new();
        let principal = AuthenticatedPrincipal::for_test_user(subject.to_string());
        let expected_binding =
            CredentialAuthenticationBinding::parse(principal.authentication_binding())
                .expect("middleware test principal emits a valid binding");

        let actor = ServerCredentialGateway::actor(&principal).expect("user actor maps");
        assert_eq!(actor.kind(), CredentialActorKind::User);
        assert_eq!(actor.subject(), subject.to_string());

        let resolve = ServerCredentialGateway::command(
            &principal,
            CredentialGatewayCommand::Resolve(
                nebula_api::domain::credential::dto::ResolveCredentialRequest {
                    credential_key: "oauth2".to_owned(),
                    data: serde_json::json!({}),
                },
            ),
        )
        .expect("resolve maps");
        assert!(matches!(
            resolve,
            CredentialCommand::Resolve {
                authentication_binding,
                ..
            } if authentication_binding == expected_binding
        ));

        let continue_resolve = ServerCredentialGateway::command(
            &principal,
            CredentialGatewayCommand::ContinueResolve(
                nebula_api::domain::credential::dto::ContinueResolveRequest {
                    credential_key: "oauth2".to_owned(),
                    pending_token: "opaque-token".to_owned(),
                    user_input: serde_json::json!("Poll"),
                },
            ),
        )
        .expect("continue maps");
        assert!(matches!(
            continue_resolve,
            CredentialCommand::ContinueResolve {
                authentication_binding,
                ..
            } if authentication_binding == expected_binding
        ));
        assert_ne!(principal.authentication_binding(), principal.subject());
    }

    #[tokio::test]
    async fn denied_command_stops_before_service_dispatch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let authority: Arc<dyn CredentialTenantAuthority> = Arc::new(CountingAuthority {
            calls: Arc::clone(&calls),
            decision: AuthorizationDecision::Deny,
        });
        let controller = CredentialController::new(service().await, authority);

        let error = controller
            .execute(
                &actor(),
                &Scope::new("workspace", "org"),
                CredentialCommand::Get {
                    credential_id: "cred_missing".to_owned(),
                },
            )
            .await
            .expect_err("denied command never reaches the not-found service path");

        assert!(matches!(
            error,
            CredentialControllerError::Authorization(CredentialAuthorizationError::Denied)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn every_command_gets_one_exact_decision_before_any_effect() {
        let binding = CredentialAuthenticationBinding::parse("A".repeat(43))
            .expect("fixed test authentication binding");
        let commands = vec![
            CredentialCommand::Create {
                credential_key: "api_key".to_owned(),
                properties: serde_json::json!({ "api_key": "secret" }),
                display: CredentialDisplay::default(),
            },
            CredentialCommand::Get {
                credential_id: "cred-1".to_owned(),
            },
            CredentialCommand::List,
            CredentialCommand::Update {
                credential_id: "cred-1".to_owned(),
                properties: None,
                expected_version: None,
                display: CredentialDisplayPatch::default(),
            },
            CredentialCommand::Delete {
                credential_id: "cred-1".to_owned(),
            },
            CredentialCommand::Test {
                credential_id: "cred-1".to_owned(),
            },
            CredentialCommand::Refresh {
                credential_id: "cred-1".to_owned(),
            },
            CredentialCommand::Revoke {
                credential_id: "cred-1".to_owned(),
            },
            CredentialCommand::Resolve {
                credential_key: "oauth2".to_owned(),
                properties: serde_json::json!({}),
                authentication_binding: binding.clone(),
            },
            CredentialCommand::ContinueResolve {
                credential_key: "oauth2".to_owned(),
                pending_token: "opaque".to_owned(),
                user_input: UserInput::Poll,
                authentication_binding: binding,
            },
        ];
        let expected = [
            CredentialOperation::Create,
            CredentialOperation::Get,
            CredentialOperation::List,
            CredentialOperation::Update,
            CredentialOperation::Delete,
            CredentialOperation::Test,
            CredentialOperation::Refresh,
            CredentialOperation::Revoke,
            CredentialOperation::Resolve,
            CredentialOperation::ContinueResolve,
        ];
        let credential_service = service().await;
        let authority = Arc::new(RecordingDenyAuthority::default());
        let controller =
            CredentialController::new(Arc::clone(&credential_service), authority.clone());
        let scope = Scope::new("workspace", "org");
        for command in commands {
            let error = controller
                .execute(&actor(), &scope, command)
                .await
                .expect_err("denying authority stops every command");
            assert!(matches!(
                error,
                CredentialControllerError::Authorization(CredentialAuthorizationError::Denied)
            ));
        }
        assert_eq!(
            authority
                .operations
                .lock()
                .expect("test operation lock")
                .as_slice(),
            expected
        );

        let allow_calls = Arc::new(AtomicUsize::new(0));
        let allow: Arc<dyn CredentialTenantAuthority> = Arc::new(CountingAuthority {
            calls: Arc::clone(&allow_calls),
            decision: AuthorizationDecision::Allow,
        });
        let verifier = CredentialController::new(credential_service, allow);
        let result = verifier
            .execute(&actor(), &scope, CredentialCommand::List)
            .await
            .expect("allowed verification list");
        assert!(matches!(result, CredentialCommandResult::Heads(rows) if rows.is_empty()));
        assert_eq!(allow_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn first_party_authority_validates_typed_binding_and_denies_workflows() {
        let org_id = OrgId::new();
        let workspace_id = WorkspaceId::new();
        let user_id = UserId::new();
        let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::seeded(
            org_id,
            CorePrincipal::User(user_id),
            OrgRole::OrgOwner,
        ));
        let authority = ServerCredentialAuthority::new(
            Some(membership),
            workspace_resolver(org_id, workspace_id),
        );
        let scope = Scope::new(workspace_id.to_string(), org_id.to_string());
        let user = CredentialActor::user(user_id.to_string()).expect("typed user");
        assert_eq!(
            authority
                .decide(&user, &scope, CredentialOperation::Get)
                .await
                .expect("typed binding decision"),
            AuthorizationDecision::Allow
        );

        let workflow = CredentialActor::workflow(nebula_core::WorkflowId::new().to_string())
            .expect("typed workflow");
        assert_eq!(
            authority
                .decide(&workflow, &scope, CredentialOperation::Get)
                .await
                .expect("workflow decision"),
            AuthorizationDecision::Deny
        );
    }

    #[tokio::test]
    async fn first_party_authority_fails_closed_without_membership_state() {
        let org_id = OrgId::new();
        let workspace_id = WorkspaceId::new();
        let authority =
            ServerCredentialAuthority::new(None, workspace_resolver(org_id, workspace_id));
        let scope = Scope::new(workspace_id.to_string(), org_id.to_string());
        let user = CredentialActor::user(UserId::new().to_string()).expect("typed user");

        assert_eq!(
            authority
                .decide(&user, &scope, CredentialOperation::Get)
                .await
                .expect_err("missing membership authority must not grant access"),
            CredentialAuthorizationError::Unavailable
        );
    }

    #[tokio::test]
    async fn first_party_authority_denies_non_members() {
        let org_id = OrgId::new();
        let workspace_id = WorkspaceId::new();
        let owner_id = UserId::new();
        let membership: Arc<dyn MembershipStore> = Arc::new(InMemoryMembershipStore::seeded(
            org_id,
            CorePrincipal::User(owner_id),
            OrgRole::OrgOwner,
        ));
        let authority = ServerCredentialAuthority::new(
            Some(membership),
            workspace_resolver(org_id, workspace_id),
        );
        let outsider = CredentialActor::user(UserId::new().to_string()).expect("typed outsider");

        assert_eq!(
            authority
                .decide(
                    &outsider,
                    &Scope::new(workspace_id.to_string(), org_id.to_string()),
                    CredentialOperation::Get,
                )
                .await
                .expect("membership denial is a decision"),
            AuthorizationDecision::Deny
        );
    }

    #[tokio::test]
    async fn first_party_authority_denies_phantom_workspace_before_membership_read() {
        let org_id = OrgId::new();
        let real_workspace = WorkspaceId::new();
        let phantom_workspace = WorkspaceId::new();
        let membership = Arc::new(RecordingMembershipStore::new(Some(OrgRole::OrgOwner), None));
        let workspace = ExactWorkspaceResolver::new(org_id, real_workspace);
        let authority =
            ServerCredentialAuthority::new(Some(membership.clone()), Some(workspace.clone()));
        let user = CredentialActor::user(UserId::new().to_string()).expect("typed user");

        assert_eq!(
            authority
                .decide(
                    &user,
                    &Scope::new(phantom_workspace.to_string(), org_id.to_string()),
                    CredentialOperation::Create,
                )
                .await
                .expect("unknown workspace is a denial"),
            AuthorizationDecision::Deny
        );
        assert_eq!(workspace.calls.load(Ordering::SeqCst), 1);
        assert_eq!(membership.snapshot_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn first_party_authority_enforces_role_operation_matrix_from_one_snapshot() {
        let org_id = OrgId::new();
        let workspace_id = WorkspaceId::new();
        let scope = Scope::new(workspace_id.to_string(), org_id.to_string());
        let user = CredentialActor::user(UserId::new().to_string()).expect("typed user");
        let operations = [
            CredentialOperation::Create,
            CredentialOperation::Get,
            CredentialOperation::List,
            CredentialOperation::Update,
            CredentialOperation::Delete,
            CredentialOperation::Test,
            CredentialOperation::Refresh,
            CredentialOperation::Revoke,
            CredentialOperation::Resolve,
            CredentialOperation::ContinueResolve,
        ];

        let viewer = Arc::new(RecordingMembershipStore::new(
            Some(OrgRole::OrgMember),
            Some(WorkspaceRole::WorkspaceViewer),
        ));
        let viewer_authority = ServerCredentialAuthority::new(
            Some(viewer.clone()),
            workspace_resolver(org_id, workspace_id),
        );
        for operation in operations {
            let expected = if matches!(
                operation,
                CredentialOperation::Get | CredentialOperation::List
            ) {
                AuthorizationDecision::Allow
            } else {
                AuthorizationDecision::Deny
            };
            assert_eq!(
                viewer_authority
                    .decide(&user, &scope, operation)
                    .await
                    .expect("viewer decision"),
                expected,
                "unexpected viewer decision for {operation:?}"
            );
        }
        assert_eq!(
            viewer.snapshot_calls.load(Ordering::SeqCst),
            operations.len()
        );
        assert_eq!(viewer.point_calls.load(Ordering::SeqCst), 0);

        let editor = Arc::new(RecordingMembershipStore::new(
            Some(OrgRole::OrgMember),
            Some(WorkspaceRole::WorkspaceEditor),
        ));
        let editor_authority = ServerCredentialAuthority::new(
            Some(editor.clone()),
            workspace_resolver(org_id, workspace_id),
        );
        for operation in operations {
            assert_eq!(
                editor_authority
                    .decide(&user, &scope, operation)
                    .await
                    .expect("editor decision"),
                AuthorizationDecision::Allow,
                "editor must be allowed for {operation:?}"
            );
        }
        assert_eq!(
            editor.snapshot_calls.load(Ordering::SeqCst),
            operations.len()
        );
        assert_eq!(editor.point_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn first_party_authority_distinguishes_denial_unavailability_and_invalid_claims() {
        let org_id = OrgId::new();
        let workspace_id = WorkspaceId::new();
        let scope = Scope::new(workspace_id.to_string(), org_id.to_string());
        let user = CredentialActor::user(UserId::new().to_string()).expect("typed user");

        let non_member = Arc::new(RecordingMembershipStore::new(
            None,
            Some(WorkspaceRole::WorkspaceEditor),
        ));
        let authority = ServerCredentialAuthority::new(
            Some(non_member.clone()),
            workspace_resolver(org_id, workspace_id),
        );
        assert_eq!(
            authority
                .decide(&user, &scope, CredentialOperation::Get)
                .await
                .expect("valid no-membership snapshot is a denial"),
            AuthorizationDecision::Deny
        );
        assert_eq!(non_member.snapshot_calls.load(Ordering::SeqCst), 1);

        let failing = Arc::new(RecordingMembershipStore::failing());
        let authority = ServerCredentialAuthority::new(
            Some(failing.clone()),
            workspace_resolver(org_id, workspace_id),
        );
        assert_eq!(
            authority
                .decide(&user, &scope, CredentialOperation::Get)
                .await
                .expect_err("membership read failure is unavailable"),
            CredentialAuthorizationError::Unavailable
        );
        assert_eq!(failing.snapshot_calls.load(Ordering::SeqCst), 1);

        let unread = Arc::new(RecordingMembershipStore::new(
            Some(OrgRole::OrgMember),
            Some(WorkspaceRole::WorkspaceEditor),
        ));
        let authority = ServerCredentialAuthority::new(
            Some(unread.clone()),
            workspace_resolver(org_id, workspace_id),
        );
        let malformed_actor = CredentialActor::user("not-a-typed-user").expect("non-empty actor");
        assert_eq!(
            authority
                .decide(&malformed_actor, &scope, CredentialOperation::Get)
                .await
                .expect_err("malformed actor is rejected"),
            CredentialAuthorizationError::Denied
        );
        assert_eq!(unread.snapshot_calls.load(Ordering::SeqCst), 0);

        assert_eq!(
            authority
                .decide(
                    &user,
                    &Scope::new("invalid-workspace", org_id.to_string()),
                    CredentialOperation::Get,
                )
                .await
                .expect_err("malformed scope is rejected"),
            CredentialAuthorizationError::InvalidScope
        );
        assert_eq!(unread.snapshot_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn first_party_authority_uses_typed_service_account_membership() {
        let service_account_id = ServiceAccountId::new();
        let store = Arc::new(RecordingMembershipStore::new(
            Some(OrgRole::OrgMember),
            Some(WorkspaceRole::WorkspaceViewer),
        ));
        let org_id = OrgId::new();
        let workspace_id = WorkspaceId::new();
        let authority = ServerCredentialAuthority::new(
            Some(store.clone()),
            workspace_resolver(org_id, workspace_id),
        );
        let actor = CredentialActor::service_account(service_account_id.to_string())
            .expect("typed service account");
        let scope = Scope::new(workspace_id.to_string(), org_id.to_string());

        assert_eq!(
            authority
                .decide(&actor, &scope, CredentialOperation::Get)
                .await
                .expect("service-account decision"),
            AuthorizationDecision::Allow
        );
        assert_eq!(store.snapshot_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.point_calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            store
                .principal
                .lock()
                .expect("test principal lock")
                .as_ref(),
            Some(&CorePrincipal::ServiceAccount(service_account_id))
        );
    }
}
