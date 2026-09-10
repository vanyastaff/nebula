//! Port-local row/record DTOs.
//!
//! Every type here depends only on `serde` + `serde_json::Value` (plus the
//! port's own [`crate::Scope`]). None of them reference `ActionResult` or any
//! higher-tier type — that would invert the Core-tier dependency direction.
//! Adapters map their backend rows to/from these DTOs at the port edge.

mod control;
pub mod credential;
mod credential_refresh_retry;
mod execution;
mod idempotency;
mod identity;
mod job_dispatch;
mod journal;
mod node_result;
mod operation_ledger;
mod operation_protocol;
mod resource_event;
mod resource_handoff;
mod resource_lease;
mod resource_subscription;
pub mod resume_token;
mod revision_catalog;
mod shared_resource;
mod start_materialization;
mod webhook;
mod workflow;

pub use control::{ControlCommand, ControlMsg, ResumeTarget};
pub use credential::{
    CredentialCommit, CredentialCreate, CredentialMaterialEpoch, CredentialMaterialEpochError,
    CredentialMaterialTransition, CredentialOwner, CredentialRecordState, CredentialReplacement,
    CredentialSelector, CredentialTombstone, CredentialVersion, CredentialVersionError,
    SecretBytes, StoredCredential, StoredCredentialHead, StoredLiveCredential,
    StoredTombstonedCredential,
};
pub use credential_refresh_retry::{
    RefreshRetryAdmission, RefreshRetryBlock, RefreshRetryDelay, RefreshRetryDelayError,
    RefreshRetryDiagnosticCode, RefreshRetryDiagnosticCodeError, RefreshRetryEvidence,
    RefreshRetryGate, RefreshRetryKind, RefreshRetryPhase, RefreshRetrySnapshot,
    RefreshRetryTransition,
};
pub use execution::{ExecutionRecord, NewExecution};
pub use idempotency::CachedRecord;
pub use identity::{
    AuditLogRow, BlobRow, MembershipRow, OrgRow, PrincipalKind, QuotaRow, ResourceRow, ScopeKind,
    TriggerRow, UserRow, WorkspaceRow,
};
pub use job_dispatch::JobDispatchMsg;
pub use journal::JournalEntry;
pub use node_result::{MAX_SUPPORTED_RESULT_SCHEMA_VERSION, NodeResultRecord};
pub use operation_ledger::{
    AttemptGeneration, DestinationCapability, DestinationCapabilityParseError, EffectOccurrenceKey,
    EffectSlotBinding, EffectSlotId, KnownOutcome, OperationLedgerError,
    OperationProtocolViolation, OperationRecord, OperationState, PrepareOutcome, PreparedOperation,
    RequestFingerprint,
};
pub use operation_protocol::{
    EffectPhase, FrozenOutcomeEvidence, InvocationDisposition, OperationAdvance, OperationCommand,
    OperationProtocolRecord, OutcomeEvidenceSource, PreparedEffectContract, PreparedEffectPolicy,
    PreparedEffectPolicyBuilder,
};
pub use resource_event::{
    AcceptResourceEventOutcome, AcceptResourceEventRequest, ClaimedResourceDelivery,
    CompleteResourceDeliveryOutcome, CompleteResourceDeliveryRequest, EventEnvelope,
    EventOccurrenceKey, EventOccurrenceNamespace, ResourceDeliveryCompletion, ResourceDeliveryId,
    ResourceEventAcceptance, ResourceEventId, ResourceEventRecord, ResourceEventState,
    ResourceEventStateParseError, ResourceEventValueError, TerminalDeliveryIneligibility,
    TerminalDeliveryIneligibilityParseError,
};
pub use resource_handoff::{
    AcknowledgeResourceHandoffOutcome, ClaimResourceHandoffsRequest, ClaimedResourceHandoff,
    HeartbeatResourceHandoffRequest, ResourceHandoffClaimRequest, ResourceHandoffClaimToken,
};
pub use resource_lease::{
    AcquireResourceSourceLeaseOutcome, AcquireResourceSourceLeaseRequest,
    ClaimResourceDeliveriesRequest, HeartbeatResourceDeliveryRequest,
    HeartbeatResourceSourceLeaseRequest, ReleaseResourceDeliveryRequest,
    ReleaseResourceSourceLeaseRequest, ResourceDeliveryClaimToken, ResourceLeaseGeneration,
    ResourceLeaseGenerationOverflow, ResourceLeaseHolder, ResourceLeaseTtl,
    ResourceLeaseValueError, ResourceSourceLease, ResourceSourceLeaseToken,
};
pub use resource_subscription::{
    PutResourceSubscriptionOutcome, PutResourceSubscriptionRequest, ResourceConsumerIdentity,
    ResourceConsumerKind, ResourceSubscriptionId, ResourceSubscriptionPage,
    ResourceSubscriptionRecord, ResourceSubscriptionState, ResourceSubscriptionStateParseError,
    ResourceSubscriptionValueError, ResourceSubscriptionVersion,
    TransitionResourceSubscriptionRequest,
};
pub use resume_token::{ResumeTokenRow, ResumeTokenWaitKind, TokenHash, TokenHashLengthError};
pub use revision_catalog::{
    BeginDrainOutcome, ExecutablePlanRecordFormat, PlanFlavorRevisionIds, PlanFlavorRevisionRecord,
    PlanFlavorRevisionTarget, RevisionCatalogError, RevisionInsertOutcome, RevisionRecordBytes,
    RevisionReferenceCounts, WorkerFlavorRecordFormat, WorkerFlavorRevisionRecord,
};
pub use shared_resource::{
    ReconciliationCursor, ResolveSharedResourceOutcome, ResolveSharedResourceRequest,
    ResourceCompatibilityVersion, ResourceConfigurationIdentity, ResourceKind, ResourcePageSize,
    ResourceSlotIdentity, SharedResourceId, SharedResourceIdentity, SharedResourcePage,
    SharedResourceRecord, SharedResourceValueError,
};
pub use start_materialization::{
    ContractBundleFormat, ContractBundleRecord, MAX_CONTRACT_BUNDLE_BYTES, MaterializedStart,
    StartKey, StartReservation, StoredContractBundle, TriggerStartKey,
};
pub use webhook::{WebhookActivationRecord, WebhookMode};
pub use workflow::{WorkflowActivation, WorkflowRecord, WorkflowVersionRecord};
