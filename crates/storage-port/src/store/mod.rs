//! Repository traits — ISP-segregated, object-safe, `#[async_trait]`.
//!
//! One atomic aggregate ([`crate::store::ExecutionStore`]) owns the §12.2 unit (state
//! transition + outbox + journal). All read-only and non-atomic concerns are
//! separate role traits so no impl becomes a god-object and consumers depend
//! only on what they use. Every trait is `dyn`-compatible — the engine/api
//! consume them as `Arc<dyn …>`.

mod checkpoint;
mod control_queue;
mod execution;
mod idempotency;
mod identity;
mod job_dispatch;
mod journal;
mod node_result;
mod refresh_claim;
mod trigger_dedup;
mod webhook;
mod workflow;

pub use checkpoint::CheckpointStore;
pub use control_queue::{ControlQueue, ReclaimOutcome};
pub use execution::ExecutionStore;
pub use idempotency::{IdempotencyGuard, IdempotencyStore};
pub use identity::{
    AuditStore, BlobStore, MembershipStore, OrgStore, QuotaStore, ResourceStore, TriggerStore,
    UserStore, WorkspaceStore,
};
pub use job_dispatch::JobDispatchQueue;
pub use journal::ExecutionJournalReader;
pub use node_result::NodeResultStore;
pub use refresh_claim::{
    ClaimAttempt, ClaimToken, HeartbeatError, ReclaimedClaim, RefreshClaim, RefreshClaimError,
    RefreshClaimStore, ReplicaId, SentinelState,
};
pub use trigger_dedup::TriggerDedupInbox;
pub use webhook::WebhookActivationStore;
pub use workflow::{WorkflowStore, WorkflowVersionStore};
