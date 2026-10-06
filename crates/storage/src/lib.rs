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
//! | [`auth`] | Plane-A accounts outside the port (users, sessions, PATs, OAuth, MFA, identity secrets); PostgreSQL implementations in `auth::postgres` | `auth::postgres`: `postgres` |
//! | [`http_idempotency`] | the API's idempotent-replay response cache | `PgHttpIdempotencyStore`: `postgres` |
//! | [`webhook_activation`] | the webhook activation spec persisted with a trigger | always |
//!
//! Private modules hold the backend-neutral decision cores every adapter
//! shares (execution fence, operation-ledger rules, checkpoint rules,
//! revision catalog, start materialization, resource status), so a rule is
//! written once and cannot drift between backends, and the SQL plumbing both
//! SQL backends share (`sql_error`, `execution_listing`). Every backend is
//! held to the same conformance suites in `tests/`.
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

// ── Port adapters ───────────────────────────────────────────────────────────

/// In-memory adapter implementing the `nebula-storage-port` contract.
pub mod inmem;
/// Postgres adapter implementing the `nebula-storage-port` contract
/// (production multi-process; real tx + `FOR UPDATE SKIP LOCKED`).
#[cfg(feature = "postgres")]
pub mod postgres;
/// SQLite adapter implementing the `nebula-storage-port` contract
/// (dev / edge single-writer; spec §5 SQLite parity boundary).
#[cfg(feature = "sqlite")]
pub mod sqlite;

// ── Persistence outside the port contract ──────────────────────────────────

pub mod auth;
/// Credential persistence (encryption, audit, refresh claims, pending state).
pub mod credential;
pub mod http_idempotency;
pub mod webhook_activation;

// ── Backend-neutral decision cores (one rule, every backend) ───────────────

mod control_turn;
/// Execution-lease fence, shared by every adapter that writes under a turn's
/// lease so the fence cannot drift between ports.
mod execution_fence;
mod execution_state;
/// Iteration-checkpoint decisions: monotone upsert, exact recommit, conflict
/// and regress.
mod iteration_checkpoint;
/// Operation-ledger decisions: state vocabulary, fence comparison, and
/// outcome write-once rules.
mod operation_ledger;
/// Resource-status decisions: TTL clipping, prune horizon, persisted-value
/// conversions.
mod resource_status;
/// Exact plan/flavor catalog decisions: record identity, recorded-form
/// validity, lifecycle vocabulary.
mod revision_catalog;
mod start_materialization;
mod workflow_activation;

// ── SQL plumbing shared by the SQL backends ─────────────────────────────────

/// Column codec of the execution listing projection.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
mod execution_listing;
#[cfg(any(test, feature = "sqlite", feature = "postgres"))]
mod migration;
/// The one `sqlx` error → `StorageError` mapping of the SQL backends.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
mod sql_error;

#[cfg(test)]
pub mod test_support;

pub use inmem::{
    InMemoryCheckpointStore, InMemoryControlQueue, InMemoryExecutionStore,
    InMemoryIdempotencyGuard, InMemoryIdempotencyStore, InMemoryJournalReader,
    InMemoryNodeResultStore, InMemoryPlanFlavorCatalog, InMemoryResourceRuntime,
    InMemoryResourceStatusStore, InMemoryResumeProducer, InMemoryResumeTokenStore,
    InMemoryWebhookActivationStore, InMemoryWorkflowStore, InMemoryWorkflowVersionStore,
};
/// The one storage error of the crate: the port's.
pub use nebula_storage_port::StorageError;
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
