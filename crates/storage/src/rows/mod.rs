//! Database row types — 1:1 mappings to SQL table columns.
//!
//! These are raw storage shapes, not domain types. IDs are `Vec<u8>` (BYTEA/BLOB),
//! enums are `String`, timestamps are `chrono::DateTime<Utc>`, JSON fields are
//! `serde_json::Value`. The mapping layer converts between rows and domain types.

// Row structs are plain data containers where field names mirror SQL columns.
// Documenting every field individually adds noise without value.
#[expect(
    missing_docs,
    reason = "row structs mirror SQL columns; per-field docs add noise without value"
)]
mod audit;
#[expect(
    missing_docs,
    reason = "row structs mirror SQL columns; per-field docs add noise without value"
)]
mod credential;
#[expect(
    missing_docs,
    reason = "row structs mirror SQL columns; per-field docs add noise without value"
)]
mod execution;
#[expect(
    missing_docs,
    reason = "row structs mirror SQL columns; per-field docs add noise without value"
)]
mod org;
#[expect(
    missing_docs,
    reason = "row structs mirror SQL columns; per-field docs add noise without value"
)]
mod quota;
#[expect(
    missing_docs,
    reason = "row structs mirror SQL columns; per-field docs add noise without value"
)]
mod trigger;
#[expect(
    missing_docs,
    reason = "row structs mirror SQL columns; per-field docs add noise without value"
)]
mod user;
#[expect(
    missing_docs,
    reason = "row structs mirror SQL columns; per-field docs add noise without value"
)]
mod workflow;

pub use audit::{AuditLogRow, BlobRow, SlugHistoryRow};
pub use credential::{CredentialAuditRow, CredentialRow, PendingCredentialRow};
pub use execution::{ExecutionNodeRow, ExecutionRow};
pub use org::{OrgMemberRow, OrgRow, ServiceAccountRow, WorkspaceMemberRow, WorkspaceRow};
pub use quota::{OrgQuotaRow, OrgQuotaUsageRow, WorkspaceQuotaUsageRow};
pub use trigger::{CronFireSlotRow, PendingSignalRow, TriggerEventRow, TriggerRow};
pub use user::{OAuthLinkRow, PersonalAccessTokenRow, SessionRow, UserRow, VerificationTokenRow};
pub use workflow::{WorkflowRow, WorkflowVersionRow};
