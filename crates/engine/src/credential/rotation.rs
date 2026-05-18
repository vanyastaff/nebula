//! Engine credential rotation orchestration surface.
//!
//! `nebula-engine` owns credential orchestration: scheduling,
//! blue-green deployment, grace-period management, and transaction state
//! machines. Contract/state types (policy, state, events, error, validation)
//! remain in `nebula_credential::rotation`.

pub mod blue_green;
pub mod fanout_driver;
pub mod grace_period;
pub mod resource_fanout;
pub mod scheduler;
pub mod token_http;
pub mod token_refresh;
pub mod transaction;

// Re-export contract types from nebula-credential (these stay in credential)
// Re-export orchestration types from engine-local modules
pub use blue_green::{
    BlueGreenRotation, BlueGreenState, DatabasePrivilege, enumerate_required_privileges,
    validate_privileges,
};
pub use fanout_driver::ResourceFanoutDriver;
pub use grace_period::{
    GracePeriodConfig, GracePeriodState, GracePeriodTracker, UsageMetrics,
    cleanup_expired_credentials, track_credential_usage,
};
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
pub use resource_fanout::{Bind, ResourceFanoutIndex, RotationOutcome};
pub use scheduler::{ExpiryMonitor, PeriodicScheduler, ScheduledRotation};
pub use token_refresh::{TokenRefreshError, refresh_oauth2_state};
pub use transaction::{
    BackupId, ManualRotation, OptimisticLock, RollbackStrategy, RotationId, RotationTransaction,
    TransactionPhase, ValidationResult,
};
