//! Repository traits — ISP-segregated, object-safe, `#[async_trait]`.
//!
//! One atomic aggregate ([`crate::store::ExecutionStore`]) owns the §12.2 unit (state
//! transition + outbox + journal). All read-only and non-atomic concerns are
//! separate role traits so no impl becomes a god-object and consumers depend
//! only on what they use. Every trait is `dyn`-compatible — the engine/api
//! consume them as `Arc<dyn …>`.

mod checkpoint;
mod control_queue;
mod credential;
mod execution;
mod idempotency;
mod identity;
mod job_dispatch;
mod journal;
mod node_result;
mod operation_ledger;
mod refresh_claim;
mod resource_subscription;
mod resume_producer;
mod resume_token;
mod revision_catalog;
mod start_acceptance;
mod turn_handoff;
mod webhook;
mod workflow;

pub use crate::dto::RevisionCatalogError;
pub use checkpoint::CheckpointStore;
pub use control_queue::{ControlClaim, ControlClaimToken, ControlQueue, ReclaimOutcome};
pub use credential::{
    CredentialAlreadyExistsKey, CredentialPersistence, CredentialPersistenceError,
};
pub use execution::ExecutionStore;
pub use idempotency::{IdempotencyGuard, IdempotencyStore};
pub use identity::{
    AuditStore, BlobStore, MembershipStore, OrgStore, QuotaStore, ResourceStore, TriggerStore,
    UserStore, WorkspaceStore,
};
pub use job_dispatch::{ClaimGeneration, JobClaim, JobClaimToken, JobDispatchQueue};
pub use journal::ExecutionJournalReader;
pub use node_result::NodeResultStore;
pub use operation_ledger::{OperationLedger, OperationLedgerAdjudicator};
pub use refresh_claim::{
    ClaimAttempt, ClaimToken, ExpiredClaim, HeartbeatError, RefreshClaim, RefreshClaimError,
    RefreshClaimStore, ReplicaId, SentinelState,
};
pub use resource_subscription::{
    ResourceEventFanoutStore, ResourceExecutionHandoffStore, ResourceRuntimeRecovery,
    ResourceSourceLeaseStore, ResourceSubscriptionStore, SharedResourceStore,
};
pub use resume_producer::ResumeProducer;
pub use resume_token::ResumeTokenStore;
pub use revision_catalog::{PlanFlavorCatalog, PlanFlavorCatalogAdmin, PlanFlavorCatalogWriter};
pub use start_acceptance::{
    FingerprintVersion, StartAcceptanceStore, StartContractIdentity, StartFingerprint,
    StartMaterialization, StartMaterializationError, StartReservationMaintenance,
    StartRevisionRejection,
};
pub use turn_handoff::{
    ControlStartAcceptance, ControlStartHandoff, ControlTurnCommand, ControlTurnCommit,
    ControlTurnCommitOutcome, ControlTurnTransition, ExecutionTurnHandoff, RecoverableTurn,
    RecoverableTurnPage, RecoveryTurnAcceptance, RecoveryTurnHandoff, TurnAcceptance, TurnHandoff,
    TurnRecovery,
};
pub use webhook::WebhookActivationStore;
pub use workflow::{WorkflowPublicationError, WorkflowStore, WorkflowVersionStore};
