//! `nebula-tenancy` — the multi-tenancy **security boundary**.
//!
//! This crate owns the *policy*, never the data:
//!
//! - [`ScopeResolver`] turns an authenticated [`Principal`] into the
//!   port's plain-data [`Scope`] (`nebula-storage-port` owns the type;
//!   this crate owns the resolution — spec §3 tension resolution).
//! - the scoping **decorators** (`decorator/`) wrap an
//!   `Arc<dyn …Store>` from the port and inject the resolved [`Scope`]
//!   into every call, so the engine/api are *structurally* unable to
//!   pass an arbitrary scope or reach the raw adapter (spec §6.2).
//!
//! Threat model (spec §6.1) is normative: id↔scope mismatch ⇒
//! `NotFound` (existence never leaks), tenant-namespaced idempotency
//! keys (no replay oracle), scoped control-queue enqueue (no
//! cross-tenant Cancel). Credential persistence receives mandatory
//! owner-bound selectors directly from its controller-owned port.
//!
//! [`Scope`]: nebula_storage_port::Scope

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod decorator;
mod error;
mod resolver;

pub use decorator::{
    ScopedControlQueue, ScopedExecutionJournalReader, ScopedExecutionStore, ScopedIdempotencyGuard,
    ScopedIdempotencyStore, ScopedNodeResultStore, ScopedResourceStore, ScopedResumeTokenStore,
    ScopedTriggerStore, ScopedWebhookActivationStore, ScopedWorkflowStore,
    ScopedWorkflowVersionStore,
};
pub use error::TenancyError;
pub use resolver::{BindingScopeResolver, Principal, ScopeResolver, request_scope};
