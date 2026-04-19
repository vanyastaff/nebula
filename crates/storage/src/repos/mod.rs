//! Repository traits — **planned spec-16 architecture**, per canon §11.6.
//!
//! ## Status — read before depending on these traits
//!
//! | Trait | Status | Notes |
//! |---|---|---|
//! | `ControlQueueRepo` + `InMemoryControlQueueRepo` | **implemented** | Produced by the API start / cancel handlers; consumed by `nebula_engine::ControlConsumer`. `Start` / `Resume` / `Restart` are dispatched via `nebula_engine::EngineControlDispatch` (ADR-0008 A2); `Cancel` / `Terminate` dispatch lands with A3. Safe to depend on as a storage port. |
//! | `ExecutionRepo`, `WorkflowRepo`, `ExecutionNodeRepo`, `JournalRepo` | **planned** | Trait definitions only — zero in-memory / Postgres implementations exist in this crate. Engine and API cannot compile against these signatures today. |
//! | `AuditRepo`, `BlobRepo`, `CredentialRepo`, `QuotaRepo`, `ResourceRepo`, `TriggerRepo`, `UserRepo`, `OrgRepo`, `WorkspaceRepo` | **planned** (some with partial Postgres glue) | Same caveat. |
//!
//! For execution / workflow persistence **use the layer-1 traits**
//! re-exported at the crate root (`nebula_storage::ExecutionRepo`,
//! `nebula_storage::WorkflowRepo`). Those are the production contract
//! the knife scenario exercises end-to-end.
//!
//! Adopting this module's design as the production contract requires
//! engine + API + runtime refactor tracked as "Sprint E — adopt spec-16
//! row model" in the workspace health audit spec
//! (`docs/superpowers/specs/2026-04-16-workspace-health-audit.md`).
//!
//! ## Design (when / if adopted)
//!
//! - Traits accept **raw byte slices** for IDs; callers encode their domain newtypes. This keeps
//!   the storage layer independent of `nebula-core` ID types and avoids cross-crate compile-time
//!   coupling.
//! - Return types are row structs from `crate::rows::*` — multi-tenant by construction
//!   (`workspace_id` / `org_id` are mandatory columns).
//! - All errors funnel through [`crate::StorageError`].
//!
//! # Example (layer-1 production path)
//!
//! ```ignore
//! use nebula_storage::{ExecutionRepo, InMemoryExecutionRepo};
//!
//! let repo: std::sync::Arc<dyn ExecutionRepo> =
//!     std::sync::Arc::new(InMemoryExecutionRepo::new());
//! // Use repo with the engine / API today — see canon §13 knife scenario.
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
pub use control_queue::{
    ControlCommand, ControlQueueEntry, ControlQueueRepo, InMemoryControlQueueRepo,
};
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
