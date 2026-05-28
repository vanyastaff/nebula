//! Backend repository traits for the persistence concerns that have not
//! moved onto the `nebula-storage-port` contract.
//!
//! Execution and workflow state are served by the spec-16 port adapters
//! (`crate::inmem` / `crate::sqlite` / `crate::postgres`). What lives
//! here is the remaining surface:
//!
//! - **Control-command outbox** — `ControlQueueRepo` with
//!   `InMemoryControlQueueRepo` and, behind the `postgres` feature,
//!   `pg::PgControlQueueRepo` (`FOR UPDATE SKIP LOCKED`).
//!   The five commands (`Start` / `Resume` / `Restart` / `Cancel` /
//!   `Terminate`) and the crashed-runner `reclaim_stuck` sweep
//!   are implemented on both backings.
//! - **Idempotency-cache store** — `IdempotencyStoreRepo` /
//!   `InMemoryIdempotencyStoreRepo`, consumed by the API idempotency
//!   middleware (`StorageBackedIdempotencyStore`).
//! - **Webhook-activation store** — `WebhookActivationRepo`.
//! - **Identity-row surface** — `AuditRepo`, `BlobRepo`, `CredentialRepo`,
//!   `OrgRepo`, `QuotaRepo`, `ResourceRepo`, `TriggerRepo`, `UserRepo`,
//!   `WorkspaceRepo`. The Postgres glue in `crate::pg` implements the
//!   subset the API consumes.
//!
//! ## Conventions
//!
//! - Traits accept **raw byte slices** for IDs; callers encode their
//!   domain newtypes. This keeps the storage layer independent of
//!   `nebula-core` ID types.
//! - Return types are row structs from [`crate::rows`] — multi-tenant by
//!   construction (`workspace_id` / `org_id` are mandatory columns).
//! - All errors funnel through [`crate::StorageError`].
//!
mod audit;
mod blob;
mod control_queue;
mod credential;
mod idempotency;
mod org;
mod quota;
mod resource;
mod trigger;
mod user;
pub(crate) mod webhook_activation;
mod workspace;

pub use audit::AuditRepo;
pub use blob::BlobRepo;
pub use control_queue::{
    ControlCommand, ControlQueueEntry, ControlQueueRepo, InMemoryControlQueueRepo, ReclaimOutcome,
};
pub use credential::CredentialRepo;
pub use idempotency::{CachedRecord, IdempotencyStoreRepo, InMemoryIdempotencyStoreRepo};
pub use org::OrgRepo;
pub use quota::QuotaRepo;
pub use resource::{ResourceEntry, ResourceRepo};
pub use trigger::TriggerRepo;
pub use user::{
    ExternalIdentityRepo, OAuthStateRepo, PatRepo, SessionRepo, UserRepo, VerificationTokenRepo,
};
pub use webhook_activation::{InMemoryWebhookActivationRepo, WebhookActivationRepo};
pub use workspace::WorkspaceRepo;
