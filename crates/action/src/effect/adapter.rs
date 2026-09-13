//! Separate preparation, invocation, and authenticated read-only interfaces.

use std::{fmt, future::Future, sync::Arc};

use nebula_core::{ExecutionId, NodeKey, OrgId, WorkflowId, WorkspaceId};
use nebula_core::{OperationCallId, OperationId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use super::RemoteEffectDescriptor;
use crate::{Action, ActionError, ActionInput, ActionMetadata, ActionResult, PreparedActionInput};

pub(crate) mod sealed {
    pub trait Sealed {}
}

/// Typed authoring contract for a remote-effect action.
pub trait RemoteEffectAction: Action {
    /// Exact effect protocol declaration implemented by this adapter.
    fn descriptor(&self) -> &RemoteEffectDescriptor;

    /// Freeze a logical provider request from typed, admitted action input.
    #[must_use = "remote preparation does nothing unless its future is awaited"]
    fn prepare(
        &self,
        input: Self::Input,
        context: &EffectPreparationContext,
    ) -> impl Future<Output = Result<PreparedRemoteEffect, EffectPreparationError>> + Send;
}

/// Identity-only context for non-egress request preparation.
///
/// No operation identity, credentials, resource clients, or ledger capabilities
/// are available here. The trusted adapter must not invoke a provider while
/// preparing the immutable logical request.
#[derive(Debug)]
pub struct EffectPreparationContext {
    execution_id: ExecutionId,
    workflow_id: WorkflowId,
    node_key: NodeKey,
    org_id: OrgId,
    workspace_id: WorkspaceId,
}

impl EffectPreparationContext {
    /// Project already admitted execution identity; this creates no effect authority.
    #[must_use]
    pub const fn new(
        execution_id: ExecutionId,
        workflow_id: WorkflowId,
        node_key: NodeKey,
        org_id: OrgId,
        workspace_id: WorkspaceId,
    ) -> Self {
        Self {
            execution_id,
            workflow_id,
            node_key,
            org_id,
            workspace_id,
        }
    }
    /// Owning execution.
    #[must_use]
    pub const fn execution_id(&self) -> ExecutionId {
        self.execution_id
    }
    /// Owning workflow.
    #[must_use]
    pub const fn workflow_id(&self) -> WorkflowId {
        self.workflow_id
    }
    /// Stable logical node identity, independent of retry attempt.
    #[must_use]
    pub const fn node_key(&self) -> &NodeKey {
        &self.node_key
    }
    /// Admitted organization.
    #[must_use]
    pub const fn org_id(&self) -> OrgId {
        self.org_id
    }
    /// Admitted workspace.
    #[must_use]
    pub const fn workspace_id(&self) -> WorkspaceId {
        self.workspace_id
    }
}

/// Payload-free failure before a request can reach the effect protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EffectPreparationError {
    /// Input cannot describe a valid logical request.
    #[error("remote effect request is invalid")]
    InvalidRequest,
    /// The destination/account binding is missing or exceeds its bound.
    #[error("remote effect destination binding is invalid")]
    InvalidDestination,
    /// Required preparation data is temporarily unavailable; no effect was called.
    #[error("remote effect preparation is unavailable")]
    Unavailable,
}

/// Capability supplied by a trusted factory declaring a remote effect.
#[async_trait::async_trait]
pub trait RemoteEffectFactory: sealed::Sealed + Send + Sync {
    /// Must equal the exact declaration when the plugin registry freezes it.
    /// Runtime later uses the checked plan declaration and never re-reads this value.
    fn descriptor(&self) -> &RemoteEffectDescriptor;

    /// Admitted metadata shared with the owning [`crate::ActionFactory`].
    fn metadata(&self) -> &Arc<ActionMetadata>;

    /// Validate and decode input with this factory's retained typed contract.
    ///
    /// # Errors
    ///
    /// Returns a redacted validation error when schema preparation or typed
    /// decoding fails.
    fn prepare_input(&self, input: ActionInput) -> Result<PreparedActionInput, ActionError>;

    /// Freeze the actual logical request without provider I/O.
    ///
    /// Canonical bytes exclude credentials/signatures but include every logical
    /// parameter that can alter the business effect. Destination bytes identify
    /// the target, account, and non-secret auth binding. Invocation must use that
    /// same request and target; credential rotation cannot change their meaning.
    ///
    /// # Errors
    /// Rejects invalid requests or unavailable preparation data before invocation.
    async fn prepare(
        &self,
        input: PreparedActionInput,
        context: &EffectPreparationContext,
    ) -> Result<PreparedRemoteEffect, EffectPreparationError>;
}

/// Immutable prepared request and its trusted provider adapter.
///
/// Runtime hashes the canonical request and binding before calling the ledger.
/// This object cannot be cloned; recovery prepares and checks the same logical
/// request against the original persisted binding before it can proceed.
pub struct PreparedRemoteEffect {
    canonical_request: Box<[u8]>,
    destination_binding: Box<[u8]>,
    adapter: Box<dyn PreparedEffectAdapter>,
}

impl PreparedRemoteEffect {
    /// Freeze bounded logical bytes and the adapter that invokes exactly them.
    ///
    /// # Errors
    /// Rejects empty requests, requests over 1 MiB, or destination bindings over 64 KiB.
    pub fn new(
        canonical_request: Box<[u8]>,
        destination_binding: Box<[u8]>,
        adapter: Box<dyn PreparedEffectAdapter>,
    ) -> Result<Self, EffectPreparationError> {
        if canonical_request.is_empty() || canonical_request.len() > 1_048_576 {
            return Err(EffectPreparationError::InvalidRequest);
        }
        if destination_binding.is_empty() || destination_binding.len() > 65_536 {
            return Err(EffectPreparationError::InvalidDestination);
        }
        Ok(Self {
            canonical_request,
            destination_binding,
            adapter,
        })
    }
    /// Canonical logical request bytes; never include these in diagnostics.
    #[must_use]
    pub fn canonical_request(&self) -> &[u8] {
        &self.canonical_request
    }
    /// Exact non-secret target/account/auth binding; it may still be sensitive.
    #[must_use]
    pub fn destination_binding(&self) -> &[u8] {
        &self.destination_binding
    }
    /// Provider interface. Runtime calls it only after acknowledging a durable grant.
    #[must_use]
    pub fn adapter(&self) -> &dyn PreparedEffectAdapter {
        self.adapter.as_ref()
    }
}

impl fmt::Debug for PreparedRemoteEffect {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedRemoteEffect")
            .field("request_bytes", &self.canonical_request.len())
            .field("binding_bytes", &self.destination_binding.len())
            .finish_non_exhaustive()
    }
}

/// Provider-call metadata supplied by runtime.
///
/// This public interface is metadata rather than an authorization boundary:
/// operation identities can be reconstructed from durable bytes, while the
/// effect driver owns the actual call authority in its private control flow and
/// invokes an adapter only after a fresh ledger grant. Implementing this trait
/// cannot make runtime call a provider.
pub trait EffectInvocationContext: Send + Sync {
    /// Durable operation identity reused for the same effect slot.
    fn operation_id(&self) -> OperationId;
    /// This particular durably authorized invocation.
    fn call_id(&self) -> OperationCallId;
    /// Backend-authored latest invocation time; adapters must honor this bound.
    fn deadline_unix_ms(&self) -> i64;
    /// Cooperative cancellation for the granted call.
    fn cancellation(&self) -> &CancellationToken;
}

/// Metadata for an authenticated read-only reconciliation query.
///
/// As with [`EffectInvocationContext`], the effect driver owns query authority;
/// this interface only describes the call once runtime has granted it.
pub trait EffectQueryContext: Send + Sync {
    /// Original effect identity to query.
    fn operation_id(&self) -> OperationId;
    /// Durably budgeted query identity.
    fn call_id(&self) -> OperationCallId;
    /// Backend-authored query deadline.
    fn deadline_unix_ms(&self) -> i64;
    /// Cooperative cancellation for this query.
    fn cancellation(&self) -> &CancellationToken;
}

/// Closed provider failure vocabulary; request and response text are not errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EffectFailureCode {
    /// Provider definitively rejected the logical request without applying it.
    Rejected,
    /// The adapter proved it could not cross the provider boundary.
    UnavailableBeforeBoundary,
    /// The prepared input is invalid for the exact adapter contract.
    InvalidRequest,
}

/// Trusted report of one invocation's provider boundary.
#[non_exhaustive]
#[derive(Debug)]
pub enum EffectInvocationOutcome {
    /// The effect is known applied. Runtime durably records output availability separately.
    Applied(Box<ActionResult<Value>>),
    /// The effect is known applied, but its output cannot be recovered.
    /// Runtime must block dependents without repeating the effect.
    AppliedWithoutOutput,
    /// A definitive provider rejection, with no applied effect.
    Rejected(EffectFailureCode),
    /// Positive evidence that no effect boundary was crossed; not a generic retry hint.
    BeforeBoundary(EffectFailureCode),
    /// The provider may have accepted the effect.
    Ambiguous,
}

/// Authoritative answer from an authenticated read-only query.
#[non_exhaustive]
#[derive(Debug)]
pub enum EffectReconciliationOutcome {
    /// The original effect is known applied, with its recoverable output if available.
    Applied(Box<ActionResult<Value>>),
    /// The effect is authoritatively known applied, but its output is unavailable.
    AppliedWithoutOutput,
    /// The original request is authoritatively known rejected without application.
    Rejected(EffectFailureCode),
    /// No authoritative answer. Ordinary not-found and transport failures belong here.
    Inconclusive,
}

/// Effecting call surface for the already frozen request.
#[async_trait::async_trait]
pub trait PreparedEffectAdapter: Send + Sync {
    /// Invoke once using runtime's newly acknowledged grant and original operation key.
    async fn invoke(&self, context: &dyn EffectInvocationContext) -> EffectInvocationOutcome;

    /// Separate authenticated read-only capability, when the descriptor permits queries.
    fn read_only_query(&self) -> Option<&dyn ReadOnlyEffectQuery> {
        None
    }
}

/// Provider-specific authenticated query that can never repeat the effecting call.
#[async_trait::async_trait]
pub trait ReadOnlyEffectQuery: Send + Sync {
    /// Query the original target/account/operation identity without changing provider state.
    async fn reconcile(&self, context: &dyn EffectQueryContext) -> EffectReconciliationOutcome;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Adapter;

    #[async_trait::async_trait]
    impl PreparedEffectAdapter for Adapter {
        async fn invoke(&self, _: &dyn EffectInvocationContext) -> EffectInvocationOutcome {
            EffectInvocationOutcome::Ambiguous
        }
    }

    #[test]
    fn prepared_request_is_bounded_and_redacted() {
        let request = PreparedRemoteEffect::new(
            b"sensitive-request".to_vec().into_boxed_slice(),
            b"sensitive-account".to_vec().into_boxed_slice(),
            Box::new(Adapter),
        )
        .unwrap();
        assert_eq!(request.canonical_request(), b"sensitive-request");
        assert_eq!(request.destination_binding(), b"sensitive-account");
        let debug = format!("{request:?}");
        assert!(!debug.contains("sensitive-request"));
        assert!(!debug.contains("sensitive-account"));
        assert!(request.adapter().read_only_query().is_none());
        for length in [0, 1_048_577] {
            assert!(matches!(
                PreparedRemoteEffect::new(
                    vec![0; length].into_boxed_slice(),
                    Box::from([1]),
                    Box::new(Adapter),
                ),
                Err(EffectPreparationError::InvalidRequest)
            ));
        }
        for length in [0, 65_537] {
            assert!(matches!(
                PreparedRemoteEffect::new(
                    Box::from([1]),
                    vec![0; length].into_boxed_slice(),
                    Box::new(Adapter),
                ),
                Err(EffectPreparationError::InvalidDestination)
            ));
        }
    }
}
