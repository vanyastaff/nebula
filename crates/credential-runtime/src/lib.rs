//! # nebula-credential-runtime
//!
//! **Role:** Credential management runtime — the single owner of the
//! credential *management bounded context*. Sole public entry is
//! `CredentialService` (lands in a later increment); all
//! invariant-bearing composition is crate-private so the secure
//! construction path is the only path.
//!
//! Exec tier. Narrowly supersedes the facade-ownership slice of
//! engine credential orchestration (engine retains the low-level resolver / RefreshCoordinator
//! / lease mechanism); see .
//!
//! This increment ships only the crate scaffold and the
//! [`CredentialServiceError`] taxonomy.
#![forbid(unsafe_code)]

pub mod binding;
pub mod builder;
pub mod error;
pub mod head;
pub mod observer;
pub mod ops;
pub mod scope;
pub mod service;
pub mod state_source;

/// Test-only credential fixtures (a refreshable type the static builtins
/// cannot provide). Gated `cfg(any(test, feature = "test-util"))`;
/// test-util gating forbids `test-util` in a release build.
#[cfg(any(test, feature = "test-util"))]
pub mod test_fixtures;

pub use binding::{TenantFingerprint, ValidatedCredentialBinding, ValidatedCredentialBindingError};
pub use builder::CredentialServiceBuilder;
pub use error::CredentialServiceError;
pub use head::CredentialHead;
pub use observer::{CredentialObserver, EventMetricObserver, NoopObserver};
pub use ops::{
    DispatchError, DispatchOps, register_all_builtin_ops, register_interactive_ops,
    register_refreshable_ops, register_revocable_ops, register_runtime_ops, register_testable_ops,
};
pub use scope::{FixedScopeResolver, TenantScope};
pub use service::{
    Acquisition, CredentialService, CredentialTypeInfo, LayeredStore, TestReport, TypeCapabilities,
};
pub use state_source::StateSource;

/// In-memory `CredentialService` test-support seam. Gated
/// `cfg(any(test, feature = "test-util"))`; test-util gating forbids `test-util`
/// in a release build (see [`service::test_support`] for the rationale).
#[cfg(any(test, feature = "test-util"))]
pub use service::test_support;
