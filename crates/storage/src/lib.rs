//! # nebula-storage — storage adapters
//!
//! The sole implementation of the `nebula-storage-port` contract, plus the
//! Plane-A account persistence that is not part of that contract.
//!
//! ## Modules
//!
//! | Module | Holds | Feature |
//! |---|---|---|
//! | [`inmem`] | port adapters over one mutex — the reference / conformance model, not a deployment backend | always |
//! | `sqlite` | port adapters for single-writer deployments | `sqlite` |
//! | `postgres` | port adapters for multi-process deployments (real transactions, `FOR UPDATE SKIP LOCKED`) | `postgres` |
//! | [`credential`] | owner-bound credential persistence and its decorators (encryption, audit, cache) | always; SQL stores with the backend features |
//! | [`repos`] + `pg` | Plane-A accounts (users, sessions, PATs, OAuth, MFA) and the API idempotency cache — traits in `repos`, PostgreSQL implementations in `pg` | `pg`: `postgres` |
//! | [`rows`] | row types of the Plane-A tables and the webhook activation spec | always |
//! | [`identity_secret`], [`session_token`] | Plane-A secret envelopes and session-token digests | always |
//!
//! Private modules hold the backend-neutral decision cores every adapter
//! shares (execution fence, operation-ledger rules, checkpoint rules,
//! revision catalog, start materialization, resource status), so a rule is
//! written once and cannot drift between backends. Every backend is held to
//! the same conformance suites in `tests/`.
//!
//! ## Durability
//!
//! - State, outbox, journal and resume tokens of one execution transition
//!   commit atomically through `ExecutionStore::commit(TransitionBatch)`,
//!   gated by the version CAS and the lease fencing token.
//! - Per-attempt idempotency through the port `IdempotencyGuard`.
//!
//! See `crates/storage/README.md` for the durability matrix and backend
//! status.

#![warn(missing_docs)]
#![warn(clippy::all)]
#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod control_turn;
/// Credential persistence (encryption, audit, refresh claims, pending state).
pub mod credential;
mod error;
/// Backend-independent execution-lease fence, shared by every adapter that
/// writes under a turn's lease so the fence cannot drift between ports.
mod execution_fence;
/// Column codec of the execution listing projection (SQL backends).
#[cfg(any(feature = "sqlite", feature = "postgres"))]
mod execution_listing;
mod execution_state;
/// Plane-A identity-secret envelopes and rotation-aware decryption.
pub mod identity_secret;
/// In-memory adapter implementing the `nebula-storage-port` contract.
pub mod inmem;
/// Backend-independent iteration-checkpoint decisions, shared by every
/// checkpoint adapter so monotone upsert, exact recommit, conflict and
/// regress cannot drift between backends.
mod iteration_checkpoint;
#[cfg(any(test, feature = "sqlite", feature = "postgres"))]
mod migration;
/// Backend-independent operation-ledger decisions, shared by every ledger
/// adapter so state vocabulary, fence comparison, and outcome write-once rules
/// cannot drift between backends.
mod operation_ledger;
/// PostgreSQL implementations of the Plane-A account [`repos`].
#[cfg(feature = "postgres")]
pub mod pg;
/// Postgres adapter implementing the `nebula-storage-port` contract
/// (production multi-process; real tx + `FOR UPDATE SKIP LOCKED`).
#[cfg(feature = "postgres")]
pub mod postgres;
/// Repository traits of the Plane-A account persistence and the API
/// idempotency cache — the surface outside the `nebula-storage-port`
/// contract.
pub mod repos;
/// Backend-independent resource-status decisions (TTL clipping, prune horizon,
/// persisted-value conversions), shared by every status adapter so liveness
/// cannot drift between backends.
mod resource_status;
/// Backend-independent exact plan/flavor catalog decisions, shared by every
/// catalog adapter so record identity, recorded-form validity, and lifecycle
/// vocabulary cannot drift between backends.
mod revision_catalog;
/// Database row types.
pub mod rows;
/// Domain-separated lookup digests for opaque browser-session tokens.
pub mod session_token;
/// SQLite adapter implementing the `nebula-storage-port` contract
/// (dev / edge single-writer; spec §5 SQLite parity boundary).
#[cfg(feature = "sqlite")]
pub mod sqlite;
mod start_materialization;
#[cfg(test)]
pub mod test_support;
mod workflow_activation;

pub use error::StorageError;
pub use inmem::{
    InMemoryCheckpointStore, InMemoryControlQueue, InMemoryExecutionStore,
    InMemoryIdempotencyGuard, InMemoryIdempotencyStore, InMemoryJournalReader,
    InMemoryNodeResultStore, InMemoryPlanFlavorCatalog, InMemoryResourceRuntime,
    InMemoryResourceStatusStore, InMemoryResumeProducer, InMemoryResumeTokenStore,
    InMemoryWebhookActivationStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
};
#[cfg(feature = "postgres")]
pub use postgres::{PgResourceRuntime, PgResourceStatusStore};
#[cfg(feature = "sqlite")]
pub use sqlite::{SqliteResourceRuntime, SqliteResourceStatusStore};
// Mirrors the gating on `migration::adopt`: with no backend feature there is no
// migration catalog to adopt a database into, and `sqlx` — which adoption is
// written entirely against — is not even a dependency. `mod migration` also
// builds under bare `test` for its catalog cases; adoption cannot, so this gate
// is narrower than that one on purpose.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub use migration::adopt::{LedgerAdoptionError, LedgerAdoptionOutcome};
