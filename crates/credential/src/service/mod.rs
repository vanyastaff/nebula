//! `CredentialService` facade plus the authority-bound `CredentialController`
//! (ADR-0092 step 6b, relocated from `nebula-credential-runtime`). Supported
//! authenticated HTTP management enters through the controller; selected
//! technical runtime/service paths remain direct until K3 closes the sole
//! semantic writer and operation-ledger boundary.
//!
//! All invariant-bearing composition is crate-private so the secure
//! construction path is the only path; the composition root
//! (`nebula-api`'s credential builder) is the only place that calls
//! `CredentialService::from_secure_parts`.
/// Acquisition (`resolve` / `continue_resolve`) methods of
/// `CredentialService` (split from `facade` for size; behaviour-preserving
/// `impl` block).
mod acquire;
pub(crate) mod binding;
/// Capability operations (`test` / `refresh` / `revoke`) of
/// `CredentialService` (split from `facade` for size; behaviour-preserving
/// `impl` block).
mod capabilities;
/// Authority-bound command controller for management-plane credential writes.
mod controller;
/// CRUD (create/read/list/update/delete) methods of `CredentialService`
/// (split from `facade` for size; behaviour-preserving `impl` block).
mod crud;
/// Type-discovery methods of `CredentialService` (split from `facade` for
/// size; behaviour-preserving `impl` block).
mod discovery;
pub(crate) mod error;
pub(crate) mod facade;
pub(crate) mod head;
pub(crate) mod observer;
pub(crate) mod ops;
pub(crate) mod scope;
/// Slot / binding resolution methods of `CredentialService` (split from
/// `facade` for size; behaviour-preserving `impl` block).
mod slot;
pub(crate) mod state_source;

pub use binding::{TenantFingerprint, ValidatedCredentialBinding, ValidatedCredentialBindingError};
pub use controller::{
    AuthorizationDecision, CredentialActor, CredentialAuthorizationError, CredentialCommand,
    CredentialCommandResult, CredentialController, CredentialControllerError,
    CredentialDisplayPatch, CredentialOperation, CredentialTenantAuthority,
};
pub use error::{CredentialServiceError, CredentialValidationIssue, CredentialValidationReport};
pub use facade::{
    Acquisition, CredentialService, CredentialTypeInfo, ManagementRefreshReport, TypeCapabilities,
};
pub use head::CredentialHead;
pub use observer::{CredentialObserver, EventMetricObserver, NoopObserver};
pub use ops::{
    DispatchError, DispatchOps, register_all_builtin_ops, register_interactive_ops,
    register_refreshable_ops, register_revocable_ops, register_runtime_ops, register_testable_ops,
};
pub use scope::{
    CredentialAuthenticationBinding, CredentialAuthenticationBindingError, TenantScope,
};
pub use state_source::StateSource;
