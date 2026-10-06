//! # nebula-storage-port — the storage port
//!
//! Object-safe repository traits, port-local DTOs, the plain-data [`Scope`]
//! value type, and the [`TransitionBatch`] atomic unit-of-work. No backend
//! code lives here.
//!
//! The port defines the contract every storage backend (in-memory, SQLite,
//! Postgres) must satisfy. Consumers (engine, api, core) depend only on this
//! crate so they stay testable without a database driver. The plain-data
//! [`Scope`] value type lives here so tenant-scoped signatures can require it
//! without an upward dependency on the tenancy policy crate.
#![warn(missing_docs)]
#![warn(clippy::all)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod backend;
mod batch;
/// Port-local row/record DTOs.
pub mod dto;
mod error;
/// Id seam: re-exported `nebula-core` identifiers + the lease
/// [`ids::FencingToken`]. The port reuses core's typed ULIDs verbatim
/// rather than re-defining them.
pub mod ids;
mod scope;
/// Repository traits (ISP-segregated, object-safe).
pub mod store;

pub use backend::StorageBackendKind;
pub use batch::{ExecutionReferenceTransition, TransitionBatch, TransitionOutcome};
pub use dto::{
    AttemptGeneration, BeginDrainOutcome, CheckpointSaved, DestinationCapability,
    DestinationCapabilityParseError, EffectOccurrenceKey, EffectOccurrenceRecord,
    EffectSlotBinding, EffectSlotId, ExecutablePlanRecordFormat, ExecutionHistoryCursor,
    ExecutionHistoryPage, ExecutionHistoryPageSize, ExecutionHistoryPageSizeError,
    ExecutionHistoryQuery, ExecutionListing, ExecutionListingStatus, ExecutionStatusSet,
    ExecutionSummary, IterationCheckpoint, IterationCheckpointError, IterationCheckpointKey,
    KnownOutcome, MAX_CHECKPOINT_ITERATION, MAX_ITERATION_CHECKPOINT_KEY_PART_BYTES,
    MAX_ITERATION_CHECKPOINT_STATE_BYTES, MAX_OCCURRENCE_LABEL_BYTES,
    MAX_PROVIDER_IDEMPOTENCY_KEY_BYTES, MicrosInstant, OccurrenceLabelViolation,
    OperationLedgerError, OperationProtocolViolation, OperationRecord, OperationState,
    PlanFlavorRevisionIds, PlanFlavorRevisionRecord, PlanFlavorRevisionTarget, PrepareOutcome,
    PreparedEffectContract, PreparedEffectPolicy, PreparedEffectPolicyBuilder, PreparedOperation,
    ProviderIdempotencyKey, ProviderIdempotencyKeyError, RefreshRetryAdmission, RefreshRetryBlock,
    RefreshRetryDelay, RefreshRetryDelayError, RefreshRetryDiagnosticCode,
    RefreshRetryDiagnosticCodeError, RefreshRetryEvidence, RefreshRetryGate, RefreshRetryKind,
    RefreshRetryPhase, RefreshRetryProjection, RefreshRetrySnapshot, RefreshRetryTransition,
    RequestFingerprint, RevisionCatalogError, RevisionInsertOutcome, RevisionRecordBytes,
    RevisionReferenceCounts, UnknownExecutionStatus, WorkerFlavorRecordFormat,
    WorkerFlavorRevisionRecord,
};
pub use dto::{
    CredentialAdmissionEpoch, CredentialAdmissionEpochError, CredentialCommit, CredentialCreate,
    CredentialMaterial, CredentialMaterialEpoch, CredentialMaterialEpochError,
    CredentialMaterialTransition, CredentialOwner, CredentialRecordState, CredentialReplacement,
    CredentialReplacementFence, CredentialSelector, CredentialTombstone, CredentialVersion,
    CredentialVersionError, MaterialUpdate, SecretBytes, StoredCredential, StoredCredentialHead,
    StoredLiveCredential, StoredTombstonedCredential,
};
pub use dto::{ResumeTokenRow, ResumeTokenWaitKind, TokenHash, TokenHashLengthError};
pub use error::StorageError;
pub use ids::{CredentialId, FencingToken, OperationCallId, OperationId};
pub use scope::Scope;
pub use store::{
    ClaimAttempt, ClaimToken, CredentialAlreadyExistsKey, CredentialIncidentRef,
    CredentialOperationDecision, CredentialOperationIntent, CredentialOperationKind,
    CredentialOperationStatus, CredentialPersistence, CredentialPersistenceError,
    CredentialRefreshCursor, CredentialRefreshHorizon, CredentialRefreshHorizonError,
    CredentialRefreshPageSize, CredentialRefreshPageSizeError, CredentialRefreshSchedule,
    CredentialRefreshScheduleError, DueCredentialRefresh, ExecutionTurnHandoff,
    MAX_CREDENTIAL_REFRESH_HORIZON_SECS, OperationLedger, OperationLedgerAdjudicator,
    PlanFlavorCatalog, PlanFlavorCatalogAdmin, PlanFlavorCatalogWriter, RefreshAdjudication,
    RefreshClaim, RefreshClaimAdjudicationError, RefreshClaimAdjudicator, RefreshClaimError,
    RefreshClaimReclaimer, RefreshClaimStore, RefreshOutcomeDecision, ReplicaId,
    ResourceEventFanoutStore, ResourceExecutionHandoffStore, ResourceRuntimeRecovery,
    ResourceSourceLeaseStore, ResourceSubscriptionStore, RevokeOutcomeDecision,
    SentinelEscalationPolicy, SharedResourceStore, StoredCredentialOperationalHead, TurnAcceptance,
    TurnHandoff, TurnRecovery,
};
