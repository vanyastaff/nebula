//! Repository traits — the public API of `nebula-storage`.
//!
//! Each domain has its own trait. All traits are `Send + Sync` and async.
//! Implementations live in this crate (in-memory, Postgres) or downstream.
//!
//! # Design
//!
//! - Traits accept **raw byte slices** for IDs; callers encode their domain newtypes. This keeps
//!   the storage layer independent of `nebula-core` ID types and avoids cross-crate compile-time
//!   coupling.
//! - Return types are row structs from `crate::rows::*`.
//! - All errors funnel through [`crate::StorageError`].
//!
//! # Example
//!
//! ```ignore
//! use nebula_storage::{OrgRepo, rows::OrgRow};
//!
//! async fn create_acme(repo: &dyn OrgRepo) -> Result<OrgRow, nebula_storage::StorageError> {
//!     let row = /* build OrgRow */ unimplemented!();
//!     repo.create(&row).await
//! }
//! ```

mod audit;
mod blob;
mod control_queue;
mod credential;
mod execution;
mod execution_node;
mod journal;
mod org;
mod quota;
mod resource;
mod trigger;
mod user;
mod workflow;
mod workspace;

pub use audit::AuditRepo;
pub use blob::BlobRepo;
pub use control_queue::{ControlCommand, ControlQueueRepo};
pub use credential::CredentialRepo;
pub use execution::ExecutionRepo;
pub use execution_node::ExecutionNodeRepo;
pub use journal::JournalRepo;
pub use org::OrgRepo;
pub use quota::QuotaRepo;
pub use resource::ResourceRepo;
pub use trigger::TriggerRepo;
pub use user::{PatRepo, SessionRepo, UserRepo};
pub use workflow::{WorkflowRepo, WorkflowVersionRepo};
pub use workspace::WorkspaceRepo;
