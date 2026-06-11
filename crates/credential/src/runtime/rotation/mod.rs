//! Rotation state-machine orchestration (ADR-0092 step 4B.1).
//!
//! The four modules here are the reqwest-free rotation **orchestration** types
//! relocated from `nebula-engine::credential::rotation`. They depend only on
//! the contract/policy/state types in `crate::rotation` (the data-type module
//! at `crate::rotation::*`) — no engine, storage, or resource edges.
//!
//! # Module layout
//!
//! - `blue_green` — `BlueGreenRotation` / `BlueGreenState` / `DatabasePrivilege`
//! - `grace_period` — `GracePeriodConfig` / `GracePeriodState` / `GracePeriodTracker` / `UsageMetrics`
//! - `scheduler` — `PeriodicScheduler` / `ExpiryMonitor` / `ScheduledRotation`
//! - `transaction` — `RotationTransaction` / `TransactionPhase` / `RollbackStrategy` / `ManualRotation` / `OptimisticLock`
//!
//! # Feature gate
//!
//! This module is compiled only when the `rotation` Cargo feature is enabled
//! (same gate as `crate::rotation`).

pub mod blue_green;
pub mod grace_period;
pub mod scheduler;
pub mod transaction;

pub use blue_green::{
    BlueGreenRotation, BlueGreenState, DatabasePrivilege, enumerate_required_privileges,
    validate_privileges,
};
pub use grace_period::{
    GracePeriodConfig, GracePeriodState, GracePeriodTracker, UsageMetrics,
    cleanup_expired_credentials, track_credential_usage,
};
pub use scheduler::{ExpiryMonitor, PeriodicScheduler, ScheduledRotation};
pub use transaction::{
    BackupId, ManualRotation, OptimisticLock, RollbackStrategy, RotationId, RotationTransaction,
    TransactionPhase, ValidationResult,
};
