//! # nebula-credential-runtime
//!
//! **Role:** Credential management runtime — the single owner of the
//! credential *management bounded context*. Sole public entry is
//! `CredentialService` (lands in a later increment); all
//! invariant-bearing composition is crate-private so the secure
//! construction path is the only path.
//!
//! Exec tier. Narrowly supersedes the facade-ownership slice of
//! ADR-0030 (engine retains the low-level resolver / RefreshCoordinator
//! / lease mechanism); see `docs/adr/0066-credential-runtime-crate.md`.
//!
//! This increment ships only the crate scaffold and the
//! [`CredentialServiceError`](error::CredentialServiceError) taxonomy.
#![forbid(unsafe_code)]

pub mod builder;
pub mod dispatch;
pub mod error;
pub mod observer;
pub mod ops;
pub mod scope;
pub mod service;
pub mod state_source;

/// Test-only credential fixtures (a refreshable type the static builtins
/// cannot provide). Gated `cfg(any(test, feature = "test-util"))`;
/// ADR-0023 forbids `test-util` in a release build.
#[cfg(any(test, feature = "test-util"))]
pub mod test_fixtures;

pub use builder::CredentialServiceBuilder;
pub use dispatch::{CredentialDispatch, DispatchError};
pub use error::CredentialServiceError;
pub use observer::{CredentialObserver, EventMetricObserver, NoopObserver};
pub use ops::{
    DispatchOps, register_all_builtin_ops, register_interactive_ops, register_refreshable_ops,
    register_revocable_ops, register_runtime_ops, register_testable_ops,
};
pub use scope::{FixedScopeResolver, TenantScope};
pub use service::{
    Acquisition, CredentialService, CredentialTypeInfo, TestReport, TypeCapabilities,
};
pub use state_source::StateSource;

/// In-memory `CredentialService` test-support seam. Gated
/// `cfg(any(test, feature = "test-util"))`; ADR-0023 forbids `test-util`
/// in a release build (see [`service::test_support`] for the rationale).
#[cfg(any(test, feature = "test-util"))]
pub use service::test_support;
