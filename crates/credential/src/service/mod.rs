//! Credential management: command authority, semantic mutations, and discovery.
//!
//! Authenticated HTTP management enters through `CredentialController`.
//! External callers submit semantic writes through that controller; service
//! mutations are crate-private. A durable operation ledger is not yet enforced.
//!
//! Management-runtime composition remains trusted application wiring through
//! `CredentialService::from_secure_parts`. Execution workers use the narrower
//! [`crate::CredentialProjectionRuntime::from_secure_parts`], which accepts only
//! read/project collaborators and cannot construct lifecycle or authority.
/// Initial acquisition and interactive continuation.
mod acquire;
pub(crate) mod binding;
/// Managed testing, refresh, and revocation.
mod capabilities;
/// Authority-bound command controller for management-plane credential writes.
mod controller;
/// Owner-scoped creation, reads, replacement, and tombstoning.
mod crud;
/// Secret-free catalog discovery.
mod discovery;
pub(crate) mod error;
pub(crate) mod facade;
pub(crate) mod head;
pub(crate) mod observer;
pub(crate) mod ops;
mod scheduled_refresh;
/// Binding validation and service adapters to the shared projection runtime.
mod slot;

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
