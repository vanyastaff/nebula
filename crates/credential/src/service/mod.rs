//! `CredentialService` facade — the sole public entry to the credential
//! management bounded context (ADR-0092 step 6b, relocated from
//! `nebula-credential-runtime`).
//!
//! All invariant-bearing composition is crate-private so the secure
//! construction path is the only path; the composition root
//! (`nebula-api`'s credential builder) is the only place that calls
//! `CredentialService::from_secure_parts`.
/// Acquisition (`resolve` / `continue_resolve`) methods of
/// `CredentialService` (split from `facade` for size; behaviour-preserving
/// `impl` block).
mod acquire;
pub mod binding;
/// Capability operations (`test` / `refresh` / `revoke`) of
/// `CredentialService` (split from `facade` for size; behaviour-preserving
/// `impl` block).
mod capabilities;
/// CRUD (create/read/list/update/delete) methods of `CredentialService`
/// (split from `facade` for size; behaviour-preserving `impl` block).
mod crud;
/// Type-discovery methods of `CredentialService` (split from `facade` for
/// size; behaviour-preserving `impl` block).
mod discovery;
pub mod error;
pub mod facade;
pub mod head;
pub mod observer;
pub mod ops;
pub mod scope;
/// Slot / binding resolution methods of `CredentialService` (split from
/// `facade` for size; behaviour-preserving `impl` block).
mod slot;
pub mod state_source;

pub use binding::{TenantFingerprint, ValidatedCredentialBinding, ValidatedCredentialBindingError};
pub use error::CredentialServiceError;
pub use facade::{
    Acquisition, CredentialService, CredentialTypeInfo, RefreshReport, TestReport, TypeCapabilities,
};
pub use head::CredentialHead;
pub use observer::{CredentialObserver, EventMetricObserver, NoopObserver};
pub use ops::{
    DispatchError, DispatchOps, register_all_builtin_ops, register_interactive_ops,
    register_refreshable_ops, register_revocable_ops, register_runtime_ops, register_testable_ops,
};
pub use scope::{FixedScopeResolver, TenantScope};
pub use state_source::StateSource;
