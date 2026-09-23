//! Authority-bound management command controller.
//!
//! The controller is the only supported mutation entry for authenticated
//! management requests. It asks one injected tenant authority for exactly one
//! decision, privately mints an authorized command, and consumes that command
//! immediately. A caller can describe intent, but cannot construct an owner
//! selector, an authorization proof, or a privileged system actor.

use std::{collections::BTreeMap, fmt, sync::Arc};

use async_trait::async_trait;
use nebula_core::{CredentialId, CredentialKey, Permission, ServiceAccountId, UserId, WorkflowId};
use nebula_storage_port::{
    CredentialOwner, CredentialSelector, Scope,
    store::{
        RefreshAdjudication, RefreshClaimAdjudicationError, RefreshClaimAdjudicator,
        RefreshOutcomeDecision,
    },
};
use serde_json::Value;
use thiserror::Error;

use crate::audit::{AuditEvent, AuditOperation, AuditResult, AuditSink};
use crate::metrics::CredentialMetrics;
use crate::resolve::{TestResult, UserInput};
use crate::{
    CredentialAuthenticationBinding, CredentialDisplay, CredentialServiceError, TenantScope,
};

use super::{Acquisition, CredentialHead, CredentialService, ManagementRefreshReport};

/// Typed authenticated actor presenting a credential command.
///
/// There is deliberately no public `System` variant. System authority may only
/// be introduced from a verified durable provenance record; absence of an
/// ordinary actor is never interpreted as administrator access.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialActor {
    /// Human user authenticated by Plane A.
    User(UserId),
    /// Non-human service account authenticated by Plane A.
    ServiceAccount(ServiceAccountId),
    /// Durable workflow identity.
    Workflow(WorkflowId),
}

impl CredentialActor {
    /// Construct claims for a typed Plane-A user.
    #[must_use]
    pub const fn user(subject: UserId) -> Self {
        Self::User(subject)
    }

    /// Construct claims for a typed Plane-A service account.
    #[must_use]
    pub const fn service_account(subject: ServiceAccountId) -> Self {
        Self::ServiceAccount(subject)
    }

    /// Construct claims for a typed durable workflow.
    #[must_use]
    pub const fn workflow(subject: WorkflowId) -> Self {
        Self::Workflow(subject)
    }
}

impl fmt::Debug for CredentialActor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = match self {
            Self::User(_) => "User",
            Self::ServiceAccount(_) => "ServiceAccount",
            Self::Workflow(_) => "Workflow",
        };
        formatter
            .debug_struct("CredentialActor")
            .field("kind", &kind)
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
    /// Begin reauthorization of an existing credential.
    Reauthorize,
    /// Continue an acquisition flow.
    ContinueResolve,
    /// Resolve a poisoned refresh claim with an operator outcome decision.
    Reconcile,
}

impl CredentialOperation {
    /// The core permission that authorizes this operation.
    ///
    /// Total on purpose, and deliberately not an `Option`. The first-party
    /// authority used to map this enum with a `_ => None` wildcard consumed as
    /// a denial, so a variant added without a decision was denied in
    /// production by the same value a real policy denial produces — silent, and
    /// indistinguishable in a test that only asserts the denial. Returning a
    /// permission makes the mapping a compile-time decision at the site that
    /// owns the operation.
    #[must_use]
    pub const fn rbac_permission(self) -> Permission {
        match self {
            Self::Get | Self::List => Permission::CredentialRead,
            Self::Create
            | Self::Update
            | Self::Test
            | Self::Refresh
            | Self::Resolve
            | Self::Reauthorize
            | Self::ContinueResolve => Permission::CredentialWrite,
            Self::Delete | Self::Revoke => Permission::CredentialDelete,
            Self::Reconcile => Permission::CredentialReconcile,
        }
    }
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
        credential_key: CredentialKey,
        /// Type-specific properties. This value may contain secrets and is
        /// never rendered by `Debug`.
        properties: Value,
        /// Non-secret display metadata.
        display: CredentialDisplay,
    },
    /// Read one credential.
    Get {
        /// Credential identifier.
        credential_id: CredentialId,
    },
    /// Enumerate credentials in the authorized owner partition.
    List,
    /// Update material and/or display metadata.
    Update {
        /// Credential identifier.
        credential_id: CredentialId,
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
        credential_id: CredentialId,
    },
    /// Test provider connectivity.
    Test {
        /// Credential identifier.
        credential_id: CredentialId,
    },
    /// Refresh provider material.
    Refresh {
        /// Credential identifier.
        credential_id: CredentialId,
    },
    /// Revoke provider material.
    Revoke {
        /// Credential identifier.
        credential_id: CredentialId,
    },
    /// Begin credential acquisition.
    Resolve {
        /// Registered credential type key.
        credential_key: CredentialKey,
        /// Type-specific properties. This value may contain secrets.
        properties: Value,
        /// Opaque Plane-A authentication binding for pending state.
        authentication_binding: CredentialAuthenticationBinding,
    },
    /// Authorize an existing credential again without replacing its identity.
    Reauthorize {
        /// Credential whose owner-qualified material is being replaced.
        credential_id: CredentialId,
        /// Type-specific properties, potentially containing secrets.
        properties: Value,
        /// Opaque Plane-A authentication binding for pending state.
        authentication_binding: CredentialAuthenticationBinding,
    },
    /// Continue credential acquisition.
    ContinueResolve {
        /// Registered credential type key.
        credential_key: CredentialKey,
        /// Opaque pending token.
        pending_token: String,
        /// Typed user input.
        user_input: UserInput,
        /// Opaque Plane-A authentication binding for pending state.
        authentication_binding: CredentialAuthenticationBinding,
    },
    /// Resolve a poisoned refresh claim with the provider outcome an operator
    /// established.
    ///
    /// The command carries the evidence the operator observed, never an
    /// incident identity or a reconciliation token: the resolved set of a
    /// credential is a many-incident set, so no single incident can be named on
    /// the path that must accept a replay, and identity is therefore the wrong
    /// anchor. `(decision, evidence digest)` is what the durable adjudication
    /// compares.
    Reconcile {
        /// Credential whose refresh claim is poisoned.
        credential_id: CredentialId,
        /// What the provider side did for the unobserved refresh.
        decision: RefreshOutcomeDecision,
        /// Operator-supplied, secret-free note recording why the outcome is now
        /// known. Digested and persisted on the incident row; never rendered by
        /// `Debug`.
        evidence: String,
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
            Self::Reauthorize { .. } => CredentialOperation::Reauthorize,
            Self::ContinueResolve { .. } => CredentialOperation::ContinueResolve,
            Self::Reconcile { .. } => CredentialOperation::Reconcile,
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
    Refreshed(ManagementRefreshReport),
    /// Provider material was revoked.
    Revoked,
    /// Acquisition result. Pending bearer material remains redacted by its
    /// own `Debug` implementation.
    Acquisition(Acquisition),
    /// Reconciliation result: the decision now on record for the credential,
    /// and whether this call recorded it.
    Reconciled(RefreshAdjudication),
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
            Self::Reconciled(adjudication) => formatter
                .debug_tuple("Reconciled")
                .field(adjudication)
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
    /// Privileged reconciliation was refused or could not be recorded.
    ///
    /// A separate arm from [`CredentialControllerError::Service`]: adjudication
    /// resolves an operator decision about an unobservable provider outcome and
    /// has its own closed taxonomy, which the credential service knows nothing
    /// about.
    #[error(transparent)]
    Adjudication(#[from] RefreshClaimAdjudicationError),
}

/// Authority-bound credential command controller.
pub struct CredentialController {
    service: Arc<CredentialService>,
    authority: Arc<dyn CredentialTenantAuthority>,
    adjudicator: Arc<dyn RefreshClaimAdjudicator>,
    audit_sink: Option<Arc<dyn AuditSink>>,
}

impl CredentialController {
    /// Bind one credential service, one tenant authority, and the privileged
    /// reconciliation seam for the lifetime of the controller.
    ///
    /// The adjudicator is a mandatory constructor argument rather than an
    /// optional field: an unwired adjudicator must not turn into a runtime
    /// surprise on the one path that clears a fail-closed poison state.
    /// `audit_sink` is optional in the shape the refresh coordinator already
    /// uses (`with_audit_sink`): without a sink, audit emission is a no-op and
    /// the tracing and metric surfaces still observe.
    ///
    /// Adding the adjudicator took this signature from two parameters to four,
    /// and that break is deliberate: a builder or a defaulted field would have
    /// left the four in-repo construction sites compiling with no adjudicator —
    /// each failing closed at runtime on the one command that must not. The
    /// sites are `apps/server`'s composition root, its credential runtime
    /// (two), and the API command port's testkit. No SDK consumer is affected:
    /// the SDK facade re-exports derive-related items only, not this type.
    #[must_use]
    pub fn new(
        service: Arc<CredentialService>,
        authority: Arc<dyn CredentialTenantAuthority>,
        adjudicator: Arc<dyn RefreshClaimAdjudicator>,
        audit_sink: Option<Arc<dyn AuditSink>>,
    ) -> Self {
        Self {
            service,
            authority,
            adjudicator,
            audit_sink,
        }
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
            tracing::warn!(?operation, ?actor, "credential command denied");
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
                    .create(&scope, credential_key.as_str(), properties, display)
                    .await?,
            ),
            CredentialCommand::Get { credential_id } => CredentialCommandResult::Head(
                self.service.get(&scope, &credential_id.to_string()).await?,
            ),
            CredentialCommand::List => {
                CredentialCommandResult::Heads(self.service.list(&scope).await?)
            },
            CredentialCommand::Update {
                credential_id,
                properties,
                expected_version,
                display,
            } => {
                let credential_id = credential_id.to_string();
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
                self.service
                    .delete(&scope, &credential_id.to_string())
                    .await?;
                CredentialCommandResult::Deleted
            },
            CredentialCommand::Test { credential_id } => CredentialCommandResult::Tested(
                self.service
                    .test(&scope, &credential_id.to_string())
                    .await?,
            ),
            CredentialCommand::Refresh { credential_id } => CredentialCommandResult::Refreshed(
                self.service
                    .refresh(&scope, &credential_id.to_string())
                    .await?,
            ),
            CredentialCommand::Revoke { credential_id } => {
                self.service
                    .revoke(&scope, &credential_id.to_string())
                    .await?;
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
                        .resolve(&scope, credential_key.as_str(), properties)
                        .await?,
                )
            },
            CredentialCommand::Reauthorize {
                credential_id,
                properties,
                authentication_binding,
            } => {
                let scope = scope.with_authentication_binding(authentication_binding);
                CredentialCommandResult::Acquisition(
                    self.service
                        .reauthorize(&scope, credential_id, properties)
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
                        .continue_resolve(
                            &scope,
                            credential_key.as_str(),
                            &pending_token,
                            user_input,
                        )
                        .await?,
                )
            },
            CredentialCommand::Reconcile {
                credential_id,
                decision,
                evidence,
            } => CredentialCommandResult::Reconciled(
                self.reconcile(&scope, &credential_id, decision, &evidence)
                    .await?,
            ),
        };
        Ok(result)
    }

    /// Adjudicate one poisoned refresh claim, with the span, the outcome
    /// counter, and the audit observation the privileged path owes its
    /// operators.
    ///
    /// Everything after the adjudication is non-authoritative by contract: the
    /// incident row the adjudicator writes is the durable record, so a sink or
    /// emitter failure is logged and never converts a recorded decision into an
    /// error the caller could retry against different evidence.
    #[tracing::instrument(
        level = "debug",
        name = "credential.reconcile",
        // `evidence` is operator prose. It is persisted, digested, on the
        // incident row; broadcasting it to every trace consumer would leak an
        // operator note far beyond the review it exists for.
        skip_all,
        fields(
            credential_id = %credential_id,
            decision = decision.as_str(),
            outcome = tracing::field::Empty,
        )
    )]
    async fn reconcile(
        &self,
        scope: &TenantScope,
        credential_id: &CredentialId,
        decision: RefreshOutcomeDecision,
        evidence: &str,
    ) -> Result<RefreshAdjudication, CredentialControllerError> {
        // Preserve the live-record gate used by sibling management commands,
        // then carry the same owner through the adjudication predicate. The
        // port never accepts a bare globally unique id as tenant authority.
        self.service.get(scope, &credential_id.to_string()).await?;
        let selector = CredentialSelector::new(
            CredentialOwner::from_canonical(scope.owner_id()),
            *credential_id,
        );

        let adjudication = self
            .adjudicator
            .adjudicate(&selector, decision, evidence)
            .await;
        let outcome = if adjudication.is_ok() {
            CredentialMetrics::OUTCOME_SUCCESS
        } else {
            CredentialMetrics::OUTCOME_FAILURE
        };
        tracing::Span::current().record("outcome", outcome);
        self.count_reconciliation(outcome);
        let adjudication = adjudication?;
        self.record_reconciliation_audit(credential_id);
        Ok(adjudication)
    }

    /// Increment the reconciliation counter, when a metrics emitter is wired.
    ///
    /// Bound to the service's observer rather than a process-global registry:
    /// the credential crate emits every other operation counter through the
    /// same seam, so an unwired emitter is one configuration to diagnose rather
    /// than two.
    fn count_reconciliation(&self, outcome: &'static str) {
        let Some(metrics) = self.service.observer.metrics() else {
            return;
        };
        metrics.counter(
            CredentialMetrics::RECONCILE_TOTAL,
            1,
            &[(CredentialMetrics::LABEL_OUTCOME, outcome)],
        );
    }

    /// Emit the audit observation for a recorded reconciliation.
    ///
    /// The `AuditSink` is non-transactional by its own contract, so this runs
    /// after the adjudication committed and its failure stays an observation.
    fn record_reconciliation_audit(&self, credential_id: &CredentialId) {
        let Some(sink) = self.audit_sink.as_deref() else {
            return;
        };
        let event = AuditEvent {
            timestamp: chrono::Utc::now(),
            credential_id: credential_id.to_string(),
            operation: AuditOperation::Reconcile,
            result: AuditResult::Success,
        };
        if let Err(error) = sink.record(&event) {
            tracing::warn!(
                ?error,
                cred = %credential_id,
                "credential audit sink failed for Reconcile"
            );
        }
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
    fn actor_debug_redacts_typed_subject() {
        let subject = UserId::new();
        let debug = format!("{:?}", CredentialActor::user(subject));
        assert!(!debug.contains(&subject.to_string()));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn command_debug_never_renders_sensitive_payloads() {
        const CANARY: &str = "credential-controller-secret-never-debug";
        let command = CredentialCommand::Create {
            credential_key: CredentialKey::new("api_key").expect("valid test credential key"),
            properties: serde_json::json!({ "api_key": CANARY }),
            display: CredentialDisplay {
                display_name: Some(CANARY.to_owned()),
                ..CredentialDisplay::default()
            },
        };
        assert!(!format!("{command:?}").contains(CANARY));
    }
}
