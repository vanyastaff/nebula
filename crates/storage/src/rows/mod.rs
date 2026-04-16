//! Database row types — 1:1 mappings to SQL table columns.
//!
//! These are raw storage shapes, not domain types. IDs are `Vec<u8>` (BYTEA/BLOB),
//! enums are `String`, timestamps are `chrono::DateTime<Utc>`, JSON fields are
//! `serde_json::Value`. The mapping layer converts between rows and domain types.

// Row structs are plain data containers where field names mirror SQL columns.
// Documenting every field individually adds noise without value.
#[allow(missing_docs)]
mod audit;
#[allow(missing_docs)]
mod credential;
#[allow(missing_docs)]
mod execution;
#[allow(missing_docs)]
mod org;
#[allow(missing_docs)]
mod quota;
#[allow(missing_docs)]
mod trigger;
#[allow(missing_docs)]
mod user;
#[allow(missing_docs)]
mod workflow;

pub use audit::{AuditLogRow, BlobRow, SlugHistoryRow};
pub use credential::{CredentialAuditRow, CredentialRow, PendingCredentialRow};
pub use execution::{ExecutionNodeRow, ExecutionRow};
pub use org::{OrgMemberRow, OrgRow, ServiceAccountRow, WorkspaceMemberRow, WorkspaceRow};
pub use quota::{OrgQuotaRow, OrgQuotaUsageRow, WorkspaceQuotaUsageRow};
pub use trigger::{CronFireSlotRow, PendingSignalRow, TriggerEventRow, TriggerRow};
pub use user::{OAuthLinkRow, PersonalAccessTokenRow, SessionRow, UserRow, VerificationTokenRow};
pub use workflow::{WorkflowRow, WorkflowVersionRow};
