//! Test-only in-process adapter for API integration suites.
//!
//! Production compositions use an adapter under `apps/`. This module exists
//! only behind the explicitly unsupported `test-util` feature so API tests can
//! exercise the same credential controller without creating a dependency cycle
//! on the server binary.

use std::{collections::BTreeMap, fmt, num::NonZeroU64, sync::Arc};

use async_trait::async_trait;
use nebula_credential::{
    Acquisition, AuthorizationDecision, CredentialActor, CredentialAuthenticationBinding,
    CredentialAuthorizationError, CredentialCommand, CredentialCommandResult, CredentialController,
    CredentialControllerError, CredentialDisplay, CredentialDisplayPatch, CredentialOperation,
    CredentialService, CredentialServiceError, CredentialTenantAuthority, InteractionRequest,
    TestFailureCode, UserInput,
};
use nebula_storage_port::Scope;

use super::{
    CredentialCommandGateway, CredentialGatewayAcquisition, CredentialGatewayCommand,
    CredentialGatewayError, CredentialGatewayRecord, CredentialGatewayRefreshRetry,
    CredentialGatewayResult, CredentialGatewayTestFailure, CredentialGatewayTestResult,
    CredentialGatewayValidationIssue, CredentialGatewayValidationReport,
};
use crate::{
    domain::credential::dto::{AcquisitionInteraction, FormPostField},
    middleware::auth::{AuthenticatedPrincipal, AuthenticatedPrincipalKind},
    ports::credential_schema::{CredentialValidationCode, CredentialValidationLocation},
};

/// Build the test-only authenticated gateway around a composed service.
///
/// The adapter still routes every command through [`CredentialController`];
/// only tenant policy is simplified to `Allow` because API tests already
/// isolate policy behavior in the auth/RBAC middleware suites.
pub fn test_gateway_from_service(
    service: Arc<CredentialService>,
) -> Arc<dyn CredentialCommandGateway> {
    let authority: Arc<dyn CredentialTenantAuthority> = Arc::new(TestAuthority);
    let controller = Arc::new(CredentialController::new(service, authority));
    Arc::new(TestGateway { controller })
}

#[derive(Debug)]
struct TestAuthority;

#[async_trait]
impl CredentialTenantAuthority for TestAuthority {
    async fn decide(
        &self,
        _actor: &CredentialActor,
        _scope: &Scope,
        _operation: CredentialOperation,
    ) -> Result<AuthorizationDecision, CredentialAuthorizationError> {
        Ok(AuthorizationDecision::Allow)
    }
}

struct TestGateway {
    controller: Arc<CredentialController>,
}

impl TestGateway {
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
        Ok(match command {
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
                let user_input: UserInput =
                    serde_json::from_value(request.user_input).map_err(|_| {
                        CredentialGatewayError::ValidationFailed {
                            report: CredentialGatewayValidationReport::single(
                                CredentialValidationLocation::UserInput,
                                CredentialValidationCode::UserInputInvalid,
                            ),
                        }
                    })?;
                CredentialCommand::ContinueResolve {
                    credential_key: request.credential_key,
                    pending_token: request.pending_token,
                    user_input,
                    authentication_binding,
                }
            },
        })
    }

    fn result(
        result: CredentialCommandResult,
    ) -> Result<CredentialGatewayResult, CredentialGatewayError> {
        match result {
            CredentialCommandResult::Head(head) => {
                Ok(CredentialGatewayResult::Record(map_head(head)))
            },
            CredentialCommandResult::Heads(heads) => Ok(CredentialGatewayResult::Records(
                heads.into_iter().map(map_head).collect(),
            )),
            CredentialCommandResult::Deleted => Ok(CredentialGatewayResult::Deleted),
            CredentialCommandResult::Tested(result) => {
                Ok(CredentialGatewayResult::Tested(map_test_result(result)))
            },
            CredentialCommandResult::Refreshed(report) => Ok(CredentialGatewayResult::Refreshed {
                record: map_head(report.head),
                refreshed: report.refreshed,
            }),
            CredentialCommandResult::Revoked => Ok(CredentialGatewayResult::Revoked),
            CredentialCommandResult::Acquisition(acquisition) => Ok(
                CredentialGatewayResult::Acquisition(map_acquisition(acquisition)?),
            ),
            _ => Err(CredentialGatewayError::Internal),
        }
    }
}

impl fmt::Debug for TestGateway {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TestCredentialGateway")
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl CredentialCommandGateway for TestGateway {
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

fn map_refresh_not_applied(advice: nebula_credential::RetryAdvice) -> CredentialGatewayError {
    // A future retry mode remains fail-closed until the HTTP contract gives it
    // an explicit representation.
    let retry = if let nebula_credential::RetryAdvice::After(delay) = advice {
        NonZeroU64::new(delay.get().as_secs())
            .map_or(CredentialGatewayRefreshRetry::Never, |seconds| {
                CredentialGatewayRefreshRetry::After { seconds }
            })
    } else {
        CredentialGatewayRefreshRetry::Never
    };
    CredentialGatewayError::RefreshNotApplied { retry }
}

fn map_service_error(error: CredentialServiceError) -> CredentialGatewayError {
    match error {
        CredentialServiceError::NotFound { .. } => CredentialGatewayError::NotFound,
        CredentialServiceError::VersionConflict {
            expected, actual, ..
        } => CredentialGatewayError::VersionConflict { expected, actual },
        CredentialServiceError::IdAlreadyExists => CredentialGatewayError::IdAlreadyExists,
        CredentialServiceError::NameAlreadyExists => CredentialGatewayError::NameAlreadyExists,
        CredentialServiceError::VersionExhausted => CredentialGatewayError::VersionExhausted,
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
        CredentialServiceError::RefreshNotApplied(context) => {
            map_refresh_not_applied(context.retry())
        },
        CredentialServiceError::TransientProvider(_)
        | CredentialServiceError::Provider(_)
        | CredentialServiceError::ExternalSourceNotWired { .. }
        | CredentialServiceError::PersistenceUnavailable => CredentialGatewayError::Unavailable,
        CredentialServiceError::OutcomeUnknown => CredentialGatewayError::OutcomeUnknown,
        CredentialServiceError::RefreshPostProviderPersistence
        | CredentialServiceError::RefreshRetryGateFinalization
        | CredentialServiceError::ReauthDecisionFinalization
        | CredentialServiceError::RefreshReconciliationRequired => {
            CredentialGatewayError::RefreshReconciliationRequired
        },
        CredentialServiceError::RevokePostProviderPersistence => {
            CredentialGatewayError::RevokeReconciliationRequired
        },
        CredentialServiceError::Store
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
    use super::*;

    #[test]
    fn test_gateway_preserves_refresh_retry_advice() {
        let delay = nebula_credential::RetryDelay::new(std::time::Duration::from_secs(17))
            .expect("non-zero test retry delay");

        assert_eq!(
            map_refresh_not_applied(nebula_credential::RetryAdvice::Never),
            CredentialGatewayError::RefreshNotApplied {
                retry: CredentialGatewayRefreshRetry::Never,
            }
        );
        assert_eq!(
            map_refresh_not_applied(nebula_credential::RetryAdvice::After(delay)),
            CredentialGatewayError::RefreshNotApplied {
                retry: CredentialGatewayRefreshRetry::After {
                    seconds: NonZeroU64::new(17).expect("test delay is non-zero"),
                },
            }
        );
    }

    #[test]
    fn test_gateway_distinguishes_unknown_refresh_and_revoke_outcomes() {
        assert_eq!(
            map_service_error(CredentialServiceError::OutcomeUnknown),
            CredentialGatewayError::OutcomeUnknown,
        );
        for error in [
            CredentialServiceError::RefreshPostProviderPersistence,
            CredentialServiceError::RefreshRetryGateFinalization,
            CredentialServiceError::ReauthDecisionFinalization,
            CredentialServiceError::RefreshReconciliationRequired,
        ] {
            assert_eq!(
                map_service_error(error),
                CredentialGatewayError::RefreshReconciliationRequired,
            );
        }
        assert_eq!(
            map_service_error(CredentialServiceError::RevokePostProviderPersistence),
            CredentialGatewayError::RevokeReconciliationRequired,
        );
    }
}
