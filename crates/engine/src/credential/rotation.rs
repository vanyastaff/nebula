//! Engine credential rotation orchestration surface.
//!
//! State-machine types (blue-green, grace-period, schedulers, transaction)
//! and token-refresh logic are relocated into `nebula_credential::runtime`
//! (ADR-0092). Re-exported here so existing
//! `nebula_engine::credential::rotation::*` paths continue to resolve.

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
// Token-refresh logic relocated to nebula-credential (ADR-0092 step 4B.2).
pub use nebula_credential::runtime::refresh::{TokenRefreshError, refresh_oauth2_state};
