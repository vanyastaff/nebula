//! Authority-bound management command controller.
//!
//! The controller is the only supported mutation entry for authenticated
//! management requests. It asks one injected tenant authority for exactly one
//! decision, privately mints an authorized command, and consumes that command
//! immediately. A caller can describe intent, but cannot construct an owner
//! selector, an authorization proof, or a privileged system actor.

use std::{collections::BTreeMap, fmt, sync::Arc};

use async_trait::async_trait;
use nebula_storage_port::Scope;
use serde_json::Value;
use thiserror::Error;

use crate::resolve::{TestResult, UserInput};
use crate::{CredentialDisplay, CredentialServiceError};

use super::{
    Acquisition, CredentialAuthenticationBinding, CredentialHead, CredentialService, RefreshReport,
    TenantScope,
};

/// Kind of authenticated actor presenting a credential command.
///
/// There is deliberately no public `System` variant. System authority may only
/// be introduced from a verified durable provenance record; absence of an
/// ordinary actor is never interpreted as administrator access.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialActorKind {
    /// Human user authenticated by Plane A.
    User,
    /// Non-human service account authenticated by Plane A.
    ServiceAccount,
    /// Durable workflow identity.
    Workflow,
}

/// Invalid authenticated-actor claims.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialActorBuildError {
    /// The canonical subject was empty.
    #[error("credential actor subject must not be empty")]
    EmptySubject,
}

/// Authenticated actor claims presented to [`CredentialTenantAuthority`].
///
/// Claims grant no authority on their own. Only the injected authority can
/// bind this actor to the requested tenant scope.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialActor {
    kind: CredentialActorKind,
    subject: String,
}

impl CredentialActor {
    /// Construct user claims from a canonical Plane-A subject.
    pub fn user(subject: impl Into<String>) -> Result<Self, CredentialActorBuildError> {
        Self::new(CredentialActorKind::User, subject)
    }

    /// Construct service-account claims from a canonical Plane-A subject.
    pub fn service_account(subject: impl Into<String>) -> Result<Self, CredentialActorBuildError> {
        Self::new(CredentialActorKind::ServiceAccount, subject)
    }

    /// Construct workflow claims from a canonical durable workflow subject.
    pub fn workflow(subject: impl Into<String>) -> Result<Self, CredentialActorBuildError> {
        Self::new(CredentialActorKind::Workflow, subject)
    }

    fn new(
        kind: CredentialActorKind,
        subject: impl Into<String>,
    ) -> Result<Self, CredentialActorBuildError> {
        let subject = subject.into();
        if subject.trim().is_empty() {
            return Err(CredentialActorBuildError::EmptySubject);
        }
        Ok(Self { kind, subject })
    }

    /// Actor kind used by tenant policy.
    #[must_use]
    pub const fn kind(&self) -> CredentialActorKind {
        self.kind
    }

    /// Canonical authenticated subject.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }
}

impl fmt::Debug for CredentialActor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialActor")
            .field("kind", &self.kind)
            .field("subject", &"[REDACTED]")
            .finish()
    }
}

/// Credential operation evaluated by tenant policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialOperation {
    /// Create a credential.
    Create,
    /// Read one credential.
    Get,
    /// Enumerate credentials.
    List,
    /// Update credential material or display metadata.
    Update,
    /// Terminally tombstone a credential while reserving its id.
    Delete,
    /// Probe provider connectivity.
    Test,
    /// Refresh provider material.
    Refresh,
    /// Revoke provider material.
    Revoke,
    /// Begin an acquisition flow.
    Resolve,
    /// Continue an acquisition flow.
    ContinueResolve,
}

/// One-call tenant-authorization outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AuthorizationDecision {
    /// The actor may perform this operation in the requested scope.
    Allow,
    /// The actor may not perform this operation in the requested scope.
    Deny,
}

/// Failure to obtain an authorization decision.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialAuthorizationError {
    /// Policy denied the request. The message is platform-owned and carries no
    /// backend or credential details.
    #[error("credential command is not authorized for this tenant")]
    Denied,
    /// The authority could not establish a trustworthy decision.
    #[error("credential tenant authority is unavailable")]
    Unavailable,
    /// The presented scope was malformed or inconsistent.
    #[error("credential tenant scope is invalid")]
    InvalidScope,
}

/// Authority that binds authenticated claims to one concrete tenant scope.
#[async_trait]
pub trait CredentialTenantAuthority: fmt::Debug + Send + Sync {
    /// Decide whether `actor` may execute `operation` in `scope`.
    ///
    /// Implementations return exactly one decision for one command. They must
    /// fail closed when upstream membership or durable provenance cannot be
    /// verified.
    async fn decide(
        &self,
        actor: &CredentialActor,
        scope: &Scope,
        operation: CredentialOperation,
    ) -> Result<AuthorizationDecision, CredentialAuthorizationError>;
}

/// Partial, non-secret display update.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct CredentialDisplayPatch {
    /// Replacement display name when present.
    pub display_name: Option<String>,
    /// Replacement description when present.
    pub description: Option<String>,
    /// Replacement tag set when present.
    pub tags: Option<BTreeMap<String, String>>,
}

impl fmt::Debug for CredentialDisplayPatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialDisplayPatch")
            .field("display_name_present", &self.display_name.is_some())
            .field("description_present", &self.description.is_some())
            .field("tags_present", &self.tags.is_some())
            .finish()
    }
}

/// Public management intent accepted by [`CredentialController`].
#[non_exhaustive]
pub enum CredentialCommand {
    /// Create a credential from type-specific properties.
    Create {
        /// Registered credential type key.
        credential_key: String,
        /// Type-specific properties. This value may contain secrets and is
        /// never rendered by `Debug`.
        properties: Value,
        /// Non-secret display metadata.
        display: CredentialDisplay,
    },
    /// Read one credential.
    Get {
        /// Credential identifier.
        credential_id: String,
    },
    /// Enumerate credentials in the authorized owner partition.
    List,
    /// Update material and/or display metadata.
    Update {
        /// Credential identifier.
        credential_id: String,
        /// Replacement type-specific properties when supplied.
        properties: Option<Value>,
        /// Compare-and-swap version when supplied.
        expected_version: Option<u64>,
        /// Display fields to overlay on the stored head.
        display: CredentialDisplayPatch,
    },
    /// Terminally tombstone one credential while reserving its id.
    Delete {
        /// Credential identifier.
        credential_id: String,
    },
    /// Test provider connectivity.
    Test {
        /// Credential identifier.
        credential_id: String,
    },
    /// Refresh provider material.
    Refresh {
        /// Credential identifier.
        credential_id: String,
    },
    /// Revoke provider material.
    Revoke {
        /// Credential identifier.
        credential_id: String,
    },
    /// Begin credential acquisition.
    Resolve {
        /// Registered credential type key.
        credential_key: String,
        /// Type-specific properties. This value may contain secrets.
        properties: Value,
        /// Opaque Plane-A authentication binding for pending state.
        authentication_binding: CredentialAuthenticationBinding,
    },
    /// Continue credential acquisition.
    ContinueResolve {
        /// Registered credential type key.
        credential_key: String,
        /// Opaque pending token.
        pending_token: String,
        /// Typed user input.
        user_input: UserInput,
        /// Opaque Plane-A authentication binding for pending state.
        authentication_binding: CredentialAuthenticationBinding,
    },
}

impl CredentialCommand {
    const fn operation(&self) -> CredentialOperation {
        match self {
            Self::Create { .. } => CredentialOperation::Create,
            Self::Get { .. } => CredentialOperation::Get,
            Self::List => CredentialOperation::List,
            Self::Update { .. } => CredentialOperation::Update,
            Self::Delete { .. } => CredentialOperation::Delete,
            Self::Test { .. } => CredentialOperation::Test,
            Self::Refresh { .. } => CredentialOperation::Refresh,
            Self::Revoke { .. } => CredentialOperation::Revoke,
            Self::Resolve { .. } => CredentialOperation::Resolve,
            Self::ContinueResolve { .. } => CredentialOperation::ContinueResolve,
        }
    }
}

impl fmt::Debug for CredentialCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialCommand")
            .field("operation", &self.operation())
            .finish_non_exhaustive()
    }
}

/// Result of one authorized credential command.
#[non_exhaustive]
pub enum CredentialCommandResult {
    /// One secret-free credential head.
    Head(CredentialHead),
    /// Secret-free heads in the authorized owner partition.
    Heads(Vec<CredentialHead>),
    /// A credential was deleted.
    Deleted,
    /// Provider connectivity test result.
    Tested(TestResult),
    /// Provider refresh result.
    Refreshed(RefreshReport),
    /// Provider material was revoked.
    Revoked,
    /// Acquisition result. Pending bearer material remains redacted by its
    /// own `Debug` implementation.
    Acquisition(Acquisition),
}

impl fmt::Debug for CredentialCommandResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Head(head) => formatter.debug_tuple("Head").field(head).finish(),
            Self::Heads(heads) => formatter
                .debug_struct("Heads")
                .field("count", &heads.len())
                .finish(),
            Self::Deleted => formatter.write_str("Deleted"),
            Self::Tested(result) => formatter.debug_tuple("Tested").field(result).finish(),
            Self::Refreshed(report) => formatter.debug_tuple("Refreshed").field(report).finish(),
            Self::Revoked => formatter.write_str("Revoked"),
            Self::Acquisition(acquisition) => formatter
                .debug_tuple("Acquisition")
                .field(acquisition)
                .finish(),
        }
    }
}

/// Failure of an authority-bound command.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CredentialControllerError {
    /// Tenant authorization denied or could not be established.
    #[error(transparent)]
    Authorization(#[from] CredentialAuthorizationError),
    /// The credential bounded context rejected the authorized operation.
    #[error(transparent)]
    Service(#[from] CredentialServiceError),
}

/// Authority-bound credential command controller.
pub struct CredentialController {
    service: Arc<CredentialService>,
    authority: Arc<dyn CredentialTenantAuthority>,
}

impl CredentialController {
    /// Bind one credential service to one tenant authority for the lifetime of
    /// the controller.
    #[must_use]
    pub fn new(
        service: Arc<CredentialService>,
        authority: Arc<dyn CredentialTenantAuthority>,
    ) -> Self {
        Self { service, authority }
    }

    /// Authorize and execute one management command.
    ///
    /// The authority is invoked exactly once. On `Allow`, the controller
    /// privately creates an authorized command and consumes it in the same
    /// call; on `Deny` or authority failure, no service method is invoked.
    pub async fn execute(
        &self,
        actor: &CredentialActor,
        scope: &Scope,
        command: CredentialCommand,
    ) -> Result<CredentialCommandResult, CredentialControllerError> {
        let operation = command.operation();
        let decision = self.authority.decide(actor, scope, operation).await?;
        if decision == AuthorizationDecision::Deny {
            tracing::warn!(?operation, actor.kind = ?actor.kind(), "credential command denied");
            return Err(CredentialAuthorizationError::Denied.into());
        }

        let authorized = AuthorizedCredentialCommand {
            scope: TenantScope::from_scope(scope),
            command,
        };
        self.execute_authorized(authorized).await
    }

    async fn execute_authorized(
        &self,
        authorized: AuthorizedCredentialCommand,
    ) -> Result<CredentialCommandResult, CredentialControllerError> {
        let AuthorizedCredentialCommand { scope, command } = authorized;
        let result = match command {
            CredentialCommand::Create {
                credential_key,
                properties,
                display,
            } => CredentialCommandResult::Head(
                self.service
                    .create(&scope, &credential_key, properties, display)
                    .await?,
            ),
            CredentialCommand::Get { credential_id } => {
                CredentialCommandResult::Head(self.service.get(&scope, &credential_id).await?)
            },
            CredentialCommand::List => {
                CredentialCommandResult::Heads(self.service.list(&scope).await?)
            },
            CredentialCommand::Update {
                credential_id,
                properties,
                expected_version,
                display,
            } => {
                let existing = self.service.get(&scope, &credential_id).await?;
                let mut merged = existing.display;
                if let Some(display_name) = display.display_name {
                    merged.display_name = Some(display_name);
                }
                if let Some(description) = display.description {
                    merged.description = Some(description);
                }
                if let Some(tags) = display.tags {
                    merged.tags = tags;
                }
                // Freeze the version observed for the patch merge. Otherwise a
                // concurrent display write between this read and the service's
                // internal load could be silently overwritten when the caller
                // omitted an explicit CAS version.
                let expected_version = Some(expected_version.unwrap_or(existing.version));
                CredentialCommandResult::Head(
                    self.service
                        .update(&scope, &credential_id, properties, expected_version, merged)
                        .await?,
                )
            },
            CredentialCommand::Delete { credential_id } => {
                self.service.delete(&scope, &credential_id).await?;
                CredentialCommandResult::Deleted
            },
            CredentialCommand::Test { credential_id } => {
                CredentialCommandResult::Tested(self.service.test(&scope, &credential_id).await?)
            },
            CredentialCommand::Refresh { credential_id } => CredentialCommandResult::Refreshed(
                self.service.refresh(&scope, &credential_id).await?,
            ),
            CredentialCommand::Revoke { credential_id } => {
                self.service.revoke(&scope, &credential_id).await?;
                CredentialCommandResult::Revoked
            },
            CredentialCommand::Resolve {
                credential_key,
                properties,
                authentication_binding,
            } => {
                let scope = scope.with_authentication_binding(authentication_binding);
                CredentialCommandResult::Acquisition(
                    self.service
                        .resolve(&scope, &credential_key, properties)
                        .await?,
                )
            },
            CredentialCommand::ContinueResolve {
                credential_key,
                pending_token,
                user_input,
                authentication_binding,
            } => {
                let scope = scope.with_authentication_binding(authentication_binding);
                CredentialCommandResult::Acquisition(
                    self.service
                        .continue_resolve(&scope, &credential_key, &pending_token, user_input)
                        .await?,
                )
            },
        };
        Ok(result)
    }
}

impl fmt::Debug for CredentialController {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CredentialController")
            .field("authority", &self.authority)
            .finish_non_exhaustive()
    }
}

/// Private one-use proof that one exact command was authorized for one scope.
///
/// This type deliberately implements neither `Clone` nor serialization.
struct AuthorizedCredentialCommand {
    scope: TenantScope,
    command: CredentialCommand,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actor_subject_must_not_be_empty() {
        assert_eq!(
            CredentialActor::user("  ").expect_err("empty subject must fail"),
            CredentialActorBuildError::EmptySubject
        );
    }

    #[test]
    fn command_debug_never_renders_sensitive_payloads() {
        const CANARY: &str = "credential-controller-secret-never-debug";
        let command = CredentialCommand::Create {
            credential_key: "api_key".to_owned(),
            properties: serde_json::json!({ "api_key": CANARY }),
            display: CredentialDisplay {
                display_name: Some(CANARY.to_owned()),
                ..CredentialDisplay::default()
            },
        };
        assert!(!format!("{command:?}").contains(CANARY));
    }
}
