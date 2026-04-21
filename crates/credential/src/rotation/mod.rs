//! Credential rotation — **contract types** (feature `rotation`).
//!
//! Policy shapes, grace-period / blue-green helpers, transaction state machines,
//! and validation traits for credential rotation. Backup row types consumed by
//! storage live in `nebula_storage::credential::backup` (ADR-0029 / ADR-0032).
//!
//! **Orchestration** (schedulers, expiry monitors, token refresh transport) lives
//! in `nebula-engine` (`credential::rotation`, feature `rotation` on the engine
//! crate) — see ADR-0030. This crate keeps the portable data and error types the
//! engine re-exports.
//!
//! # Feature gate
//!
//! The `rotation` Cargo feature must be enabled for this module to compile in
//! consumers that need rotation policy types.
//!
//! # See also
//!
//! - `docs/adr/0028-cross-crate-credential-invariants.md`
//! - `docs/adr/0030-engine-owns-credential-orchestration.md`
//! - `crates/credential/README.md` — crate role vs `nebula-engine::credential`

// Module exports
//
// Note: `RotationBackup` lives in `nebula_storage::credential::backup` per
// ADR-0029 §4 / ADR-0032 — it is a storage-side data struct. Credential only
// exposes the contract types (errors, IDs, policies, transactions) that
// `RotationBackup` references.
pub mod blue_green;
pub mod error;
pub mod events;
pub mod grace_period;
pub mod policy;
pub mod state;
pub mod transaction;
pub mod validation;

// Re-exports
pub use blue_green::{
    BlueGreenRotation, BlueGreenState, DatabasePrivilege, enumerate_required_privileges,
    validate_privileges,
};
pub use error::{RotationError, RotationErrorLog, RotationResult};
pub use events::{
    CredentialRotationEvent, LogEntryType, NotificationEvent, NotificationSender, TransactionLog,
    TransactionLogEntry, TransactionOutcome, log_rollback_event, send_notification,
};
pub use grace_period::{
    GracePeriodConfig, GracePeriodState, GracePeriodTracker, UsageMetrics,
    cleanup_expired_credentials, track_credential_usage,
};
pub use policy::{
    BeforeExpiryConfig, ManualConfig, PeriodicConfig, RotationPolicy, ScheduledConfig,
};
pub use state::RotationState;
pub use transaction::{
    BackupId, ManualRotation, OptimisticLock, RollbackStrategy, RotationId, RotationTransaction,
    TransactionPhase, ValidationResult,
};
pub use validation::{
    FailureHandler, FailureKind, RotatableCredential, SuccessCriteria, TestContext, TestMethod,
    TestResult, TestableCredential, ValidationTest,
};
