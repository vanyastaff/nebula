//! In-memory adapter — the `nebula-storage-port` implementation for tests,
//! local single-process runs, and the loom probe.
//!
//! Each store is one `parking_lot::Mutex`-guarded map. `commit` performs the
//! whole §12.2 triple (CAS + lease fencing + state + outbox + journal) under
//! a single lock, so it behaviourally models the single-writer contract the
//! conformance suite asserts. The scope predicate is enforced exactly as the
//! SQL backends enforce `WHERE workspace_id = ? AND org_id = ?`, so
//! cross-tenant denial is proven uniformly across backends.

mod control_queue;
mod control_turn;
mod execution;
mod execution_fence;
mod identity;
mod iteration_checkpoint;
mod job_dispatch;
mod journal;
mod node_result;
mod operation_ledger;
mod plan_flavor_catalog;
mod resource_runtime;
mod resource_status;
mod resume_producer;
mod resume_token;
mod start_acceptance;
mod turn_handoff;
mod turn_recovery;
mod webhook_activation;
mod workflow;

pub use control_queue::InMemoryControlQueue;
pub use execution::{InMemoryExecutionStore, InMemoryIdempotencyGuard};
pub use identity::{
    InMemoryIdentityDirectory, InMemoryMembershipStore, InMemoryOrgStore, InMemoryResourceStore,
    InMemoryTriggerStore, InMemoryWorkspaceStore,
};
pub use iteration_checkpoint::InMemoryCheckpointStore;
pub use job_dispatch::InMemoryJobDispatchQueue;
pub use journal::InMemoryJournalReader;
pub use node_result::InMemoryNodeResultStore;
pub use operation_ledger::InMemoryOperationLedger;
pub use plan_flavor_catalog::InMemoryPlanFlavorCatalog;
pub use resource_runtime::InMemoryResourceRuntime;
pub use resource_status::InMemoryResourceStatusStore;
pub use resume_producer::InMemoryResumeProducer;
pub use resume_token::InMemoryResumeTokenStore;
pub use start_acceptance::InMemoryStartAcceptanceStore;
pub use turn_handoff::InMemoryTurnHandoff;
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) use turn_handoff::{acceptance_label, control_acceptance_label};
pub use webhook_activation::InMemoryWebhookActivationStore;
pub use workflow::{InMemoryWorkflowStore, InMemoryWorkflowVersionStore};
