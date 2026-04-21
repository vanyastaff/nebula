//! Engine credential rotation orchestration surface.
//!
//! During P8 migration, engine exposes scheduler/orchestration types through
//! this module while canonical contract/state types remain in
//! `nebula_credential::rotation`.

pub mod scheduler;
pub mod token_refresh;

pub use nebula_credential::rotation::{
    RotationError, RotationResult,
    blue_green::{BlueGreenRotation, BlueGreenState},
    grace_period::{
        GracePeriodConfig, GracePeriodState, GracePeriodTracker, UsageMetrics,
        cleanup_expired_credentials, track_credential_usage,
    },
    policy::{BeforeExpiryConfig, ManualConfig, PeriodicConfig, RotationPolicy, ScheduledConfig},
    transaction::{
        BackupId, ManualRotation, OptimisticLock, RollbackStrategy, RotationId,
        RotationTransaction, TransactionPhase, ValidationResult,
    },
};
pub use scheduler::{ExpiryMonitor, PeriodicScheduler, ScheduledRotation};
pub use token_refresh::{TokenRefreshError, refresh_oauth2_state};
