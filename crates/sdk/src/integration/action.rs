//! Curated action contracts for integration authors.
//!
//! Remote-effect declarations describe an adapter's authoring contract. The
//! durable ledger projection and storage authority remain internal runtime
//! concerns and are intentionally absent from this module.
//!
//! Erased action inputs, prepared proofs, factories, and admitted metadata are
//! runtime concerns and are intentionally absent from this module.

pub use nebula_action::{
    Action, ActionEffectContract, EffectFailureCode, EffectInvocationContext,
    EffectInvocationOutcome, EffectPreparationContext, EffectPreparationError, EffectQueryContext,
    EffectReconciliationOutcome, JournalProtocol, Metadata, OperationCallId, OperationId,
    PreparedEffectAdapter, PreparedRemoteEffect, ReadOnlyEffectQuery, RemoteDestinationGuarantee,
    RemoteEffectAction, RemoteEffectDeclarationError, RemoteEffectDescriptor, RemoteEffectPolicy,
    RemoteEffectPolicyBuilder, RemoteEffectPolicyError, StableKeyGuarantee,
};
pub use nebula_core::{ExecutionId, NodeKey, OrgId, WorkflowId, WorkspaceId};
pub use tokio_util::sync::CancellationToken;
