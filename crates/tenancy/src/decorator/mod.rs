//! Scope-enforcing decorators over the storage port (spec §6.2).
//!
//! Each decorator wraps an `Arc<dyn …Store>` from `nebula-storage-port`
//! together with a **bound** [`Scope`] resolved once at the composition
//! root by a [`ScopeResolver`]. Every call **substitutes** the bound
//! scope, ignoring whatever scope the caller passed. The engine/api
//! therefore cannot forge another tenant's scope — they hold only the
//! decorated handle (the raw adapter constructor is crate-private to
//! wiring), so the confused-deputy abuse case (§6.1) is closed *by
//! construction*, not by discipline.
//!
//! Why substitute rather than compare-and-reject? A reject path that
//! distinguishes "wrong scope" from "no such row" is itself an existence
//! oracle. Substituting the bound scope and letting the backend's
//! `WHERE workspace_id = ? AND org_id = ?` do the filtering yields a
//! uniform `NotFound`/`Ok(None)` for any id that is not in the bound
//! tenant — exactly the §6.1 contract ("never the row, never
//! `ScopeViolation` leaking existence").
//!
//! [`Scope`]: nebula_storage_port::Scope
//! [`ScopeResolver`]: crate::ScopeResolver

mod control_queue;
mod execution;
mod idempotency;
mod journal;
mod node_result;
mod webhook;
mod workflow;

pub use control_queue::ScopedControlQueue;
pub use execution::ScopedExecutionStore;
pub use idempotency::{ScopedIdempotencyGuard, ScopedIdempotencyStore};
pub use journal::ScopedExecutionJournalReader;
pub use node_result::ScopedNodeResultStore;
pub use webhook::ScopedWebhookActivationStore;
pub use workflow::{ScopedWorkflowStore, ScopedWorkflowVersionStore};
