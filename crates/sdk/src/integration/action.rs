//! Curated action contracts for integration authors.
//!
//! Remote-effect declarations describe an adapter's authoring contract. The
//! durable ledger projection and storage authority remain internal runtime
//! concerns and are intentionally absent from this module.

pub use nebula_action::{
    ActionEffectContract, EffectFailureCode, EffectInvocationContext, EffectInvocationOutcome,
    EffectPreparationContext, EffectPreparationError, EffectQueryContext,
    EffectReconciliationOutcome, OperationCallId, OperationId, PreparedEffectAdapter,
    PreparedRemoteEffect, ReadOnlyEffectQuery, RemoteDestinationGuarantee, RemoteEffectDescriptor,
    RemoteEffectFactory, RemoteEffectPolicy, RemoteEffectPolicyBuilder, RemoteEffectPolicyError,
    StableKeyGuarantee,
};
