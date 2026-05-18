//! Credential rotation — **contract types** (feature `rotation`).
//!
//! Policy shapes, state machines, error types, events, and validation traits
//! for credential rotation. Backup row types consumed by storage live in
//! `nebula_storage::credential::backup` (backup store / storage credential layers).
//!
//! **Orchestration** (schedulers, blue-green deployment, grace-period management,
//! transaction state machines) lives in `nebula-engine` (`credential::rotation`,
//! feature `rotation` on the engine crate) — see engine credential orchestration. This crate keeps the
//! portable contract and error types the engine re-exports.
//!
//! # Feature gate
//!
//! The `rotation` Cargo feature must be enabled for this module to compile in
//! consumers that need rotation policy types.
//!
//! # See also
//!
//! -
//! -
//! - `crates/credential/README.md` — crate role vs `nebula-engine::credential`

// Contract type modules — these stay in nebula-credential
pub mod error;
pub mod events;
pub mod ids;
pub mod policy;
pub mod state;

// Re-exports — contract types only
pub use error::{
    FailureHandler, FailureKind, RotatableCredential, RotationError, RotationErrorLog,
    RotationResult, SuccessCriteria, TestContext, TestMethod, TestResult, TestableCredential,
    ValidationTest,
};
pub use events::{
    CredentialRotationEvent, LogEntryType, NotificationEvent, NotificationSender, TransactionLog,
    TransactionLogEntry, TransactionOutcome, log_rollback_event, send_notification,
};
pub use ids::{BackupId, RotationId};
pub use policy::{
    BeforeExpiryConfig, ManualConfig, PeriodicConfig, RotationPolicy, ScheduledConfig,
};
pub use state::RotationState;
