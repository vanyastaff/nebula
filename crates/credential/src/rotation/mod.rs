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
//!
//! ## Module scope (audited 2026-05-20)
//!
//! Domain types only: rotation events, errors, state-machine enum, IDs,
//! policy. No orchestration: no `tokio::spawn`, no `tokio::select!`, no
//! `tokio::time::{sleep, interval, tick}`, no `JoinHandle` ownership.
//! Background tick loops, blue-green transactions, fan-out drivers,
//! schedulers, grace-period reapers — all live in
//! `nebula_engine::credential::rotation::*` instead.
//!
//! Note: `error.rs` imports `tokio::time::timeout` solely to enforce the
//! per-test deadline declared by `TestableCredential::test_timeout()`. This
//! is a one-shot future combinator, not background orchestration — no tasks
//! are spawned and no loops are started.
//!
//! If you find yourself wanting to `tokio::spawn` here, you are wrong:
//! move the work to engine and emit a typed event from this module.

// Contract type modules — these stay in nebula-credential
pub mod error;
pub mod events;
pub mod ids;
pub mod policy;
pub mod state;

// Re-exports — contract types only
pub use error::{
    FailureHandler, FailureKind, RotatableCredential, RotationError, RotationErrorLog,
    RotationResult, RotationTestResult, SuccessCriteria, TestContext, TestMethod,
    TestableCredential, ValidationTest,
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
