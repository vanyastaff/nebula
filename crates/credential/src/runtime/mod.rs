//! Credential lifecycle orchestration (ADR-0092).
//!
//! These modules host the runtime resolution/dispatch primitives the execution
//! engine drives. They were relocated here from `nebula-engine::credential` so
//! the whole credential subsystem lives in one crate; they depend only on the
//! contract types in this crate (no `nebula-engine` / `nebula-storage` edge).

pub mod dispatchers;
pub mod executor;
pub mod lease;
pub mod refresh;
pub mod scoped_accessor;

pub use dispatchers::{dispatch_release, dispatch_revoke, dispatch_test};
pub use executor::{ExecutorError, ResolveResponse, execute_continue, execute_resolve};
pub use lease::{
    LeaseLifecycle, LeaseLifecycleConfig, LeaseLifecycleError, LeaseToken, RenewalPolicy,
};
pub use refresh::{
    ConfigError, ReclaimSweepHandle, RefreshAttempt, RefreshConfigError, RefreshCoordConfig,
    RefreshCoordMetrics, RefreshCoordinator, RefreshError, SentinelDecision,
    SentinelThresholdConfig, SentinelTrigger,
};
pub use scoped_accessor::ScopedCredentialAccessor;
