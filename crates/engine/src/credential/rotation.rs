//! Engine credential rotation orchestration surface.
//!
//! State-machine types (blue-green, grace-period, schedulers, transaction)
//! are relocated into `nebula_credential::runtime::rotation` (ADR-0092 step 4B.1)
//! and re-exported here so existing `nebula_engine::credential::rotation::*`
//! paths continue to resolve.
//!
//! `token_http` and `token_refresh` (reqwest — next step) remain engine-local.

pub mod token_http;
pub mod token_refresh;

// Re-export relocated state-machine types from nebula-credential.
pub use nebula_credential::runtime::rotation::{
    BlueGreenRotation, BlueGreenState, DatabasePrivilege, ExpiryMonitor, GracePeriodConfig,
    GracePeriodState, GracePeriodTracker, ManualRotation, OptimisticLock, PeriodicScheduler,
    RollbackStrategy, RotationTransaction, ScheduledRotation, TransactionPhase, UsageMetrics,
    ValidationResult, cleanup_expired_credentials, enumerate_required_privileges,
    track_credential_usage, validate_privileges,
};
// Re-export domain IDs (sourced from credential contract via the rotation module).
pub use nebula_credential::runtime::rotation::{BackupId, RotationId};
// Re-export contract types from nebula-credential.
pub use nebula_credential::rotation::{
    CredentialRotationEvent, FailureHandler, FailureKind, RotatableCredential, RotationError,
    RotationResult, SuccessCriteria, TestContext, TestMethod, TestResult, TestableCredential,
    ValidationTest,
    error::RotationErrorLog,
    events::{
        LogEntryType, NotificationEvent, NotificationSender, TransactionLog, TransactionLogEntry,
        TransactionOutcome, log_rollback_event, send_notification,
    },
    policy::{BeforeExpiryConfig, ManualConfig, PeriodicConfig, RotationPolicy, ScheduledConfig},
    state::RotationState,
};
// Fan-out types relocated to nebula-resource (ADR-0092 step 5).
pub use nebula_resource::{Bind, ResourceFanoutDriver, ResourceFanoutIndex, RotationOutcome};
pub use token_refresh::{TokenRefreshError, refresh_oauth2_state};
