//! Port-local row/record DTOs.
//!
//! Every type here depends only on `serde` + `serde_json::Value` (plus the
//! port's own [`crate::Scope`]). None of them reference `ActionResult` or any
//! higher-tier type — that would invert the Core-tier dependency direction.
//! Adapters map their backend rows to/from these DTOs at the port edge.

mod control;
mod execution;
mod idempotency;
mod identity;
mod job_dispatch;
mod journal;
mod node_result;
mod trigger_dedup;
mod webhook;
mod workflow;

pub use control::{ControlCommand, ControlMsg};
pub use execution::{ExecutionRecord, NewExecution};
pub use idempotency::CachedRecord;
pub use identity::{
    AuditLogRow, BlobRow, MembershipRow, OrgRow, PrincipalKind, QuotaRow, ResourceRow, ScopeKind,
    TriggerRow, UserRow, WorkspaceRow,
};
pub use job_dispatch::{CapabilityTag, DispatchKind, DispatchOutcome, JobDispatchMsg};
pub use journal::JournalEntry;
pub use node_result::{MAX_SUPPORTED_RESULT_SCHEMA_VERSION, NodeResultRecord};
pub use trigger_dedup::TriggerDedupRow;
pub use webhook::WebhookActivationRecord;
pub use workflow::{WorkflowRecord, WorkflowVersionRecord};
