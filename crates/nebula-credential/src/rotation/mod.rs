//! Credential Rotation Module
//!
//! # T101: Module Documentation
//!
//! Automatic credential rotation with zero downtime, supporting multiple rotation policies
//! and advanced safety features for production environments.
//!
//! # Rotation Policies
//!
//! The module supports four rotation strategies:
//!
//! - **Periodic**: Rotate at fixed intervals (e.g., every 90 days)
//!   - Use case: Compliance requirements (PCI-DSS, SOC 2)
//!   - Includes jitter to prevent thundering herd
//!   - Configurable grace period for gradual migration
//!
//! - **BeforeExpiry**: Rotate when credential approaches expiration (e.g., at 80% TTL)
//!   - Use case: OAuth tokens, TLS certificates, temporary credentials
//!   - Prevents service disruption from expired credentials
//!   - Automatic calculation of rotation time based on TTL
//!
//! - **Scheduled**: Rotate at specific date/time (e.g., maintenance window)
//!   - Use case: Planned maintenance, coordinated deployments
//!   - Pre-rotation notifications
//!   - Precise control over rotation timing
//!
//! - **Manual**: Rotate on-demand (e.g., security incident response)
//!   - Use case: Credential compromise, emergency revocation
//!   - Immediate revocation option (no grace period)
//!   - Audit trail with reason and triggered_by tracking
//!
//! # Safety Features
//!
//! ## Zero-Downtime Patterns
//!
//! - **Blue-Green Rotation**: Database credentials with standby validation
//! - **Grace Periods**: Dual-credential validity during migration
//! - **Usage Tracking**: Monitor old credential usage, auto-cleanup when safe
//!
//! ## Failure Handling
//!
//! - **Automatic Rollback**: On validation failure, restore previous credential
//! - **Retry Logic**: Exponential backoff with jitter for transient failures
//! - **Failure Classification**: Distinguish transient (network) vs permanent (auth) errors
//!
//! ## Concurrency Control
//!
//! - **Two-Phase Commit**: Atomic rotation with prepare/commit phases
//! - **Optimistic Locking**: Version-based CAS to prevent concurrent rotations
//! - **State Machine**: Enforced state transitions for correctness
//!
//! # Examples
//!
//! ## Periodic Rotation
//!
//! ```rust,no_run
//! use nebula_credential::rotation::{RotationPolicy, PeriodicConfig};
//! use std::time::Duration;
//!
//! // Configure 90-day rotation with 24-hour grace period
//! let policy = RotationPolicy::Periodic(
//!     PeriodicConfig::new(
//!         Duration::from_secs(90 * 24 * 60 * 60), // interval
//!         Duration::from_secs(24 * 60 * 60),      // grace_period
//!         true,                                    // enable_jitter
//!     ).expect("valid config")
//! );
//! ```
//!
//! ## Scheduled Rotation
//!
//! ```rust,no_run
//! use nebula_credential::rotation::{RotationPolicy, ScheduledConfig};
//! use chrono::{Utc, Duration};
//!
//! // Rotate during next maintenance window
//! let policy = RotationPolicy::Scheduled(
//!     ScheduledConfig::new(
//!         Utc::now() + Duration::days(7),
//!         std::time::Duration::from_secs(3600), // grace_period (1 hour)
//!         Some(std::time::Duration::from_secs(24 * 3600)), // notify_before
//!     ).expect("valid config")
//! );
//! ```
//!
//! ## Manual Emergency Rotation
//!
//! ```rust,no_run
//! use nebula_credential::rotation::{RotationPolicy, ManualConfig};
//!
//! // Emergency rotation with immediate revocation
//! let policy = RotationPolicy::Manual(ManualConfig::emergency());
//! ```
//!
//! ## Blue-Green Database Rotation
//!
//! ```rust,no_run
//! use nebula_credential::rotation::BlueGreenRotation;
//! use nebula_credential::core::CredentialId;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let blue = CredentialId::new("db-prod")?;
//! let green = CredentialId::new("db-prod-standby")?;
//!
//! let mut rotation = BlueGreenRotation::new(blue, green);
//!
//! // Create and validate standby credential
//! rotation.validate_standby_connectivity(|id| async {
//!     // Test database connection with standby credential
//!     Ok(())
//! }).await?;
//!
//! // Atomic swap to standby
//! rotation.swap_credentials(|active, standby| async {
//!     // Update application config to use new credential
//!     Ok(())
//! }).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Architecture
//!
//! The rotation system uses a layered architecture:
//!
//! ## Layer 1: Policy & Configuration
//! - `RotationPolicy`: Defines when to rotate
//! - `GracePeriodConfig`: Controls dual-credential validity
//! - `RotationRetryPolicy`: Configures retry behavior
//!
//! ## Layer 2: Transaction & State
//! - `RotationTransaction`: Tracks rotation lifecycle
//! - `RotationState`: State machine (Pending → Creating → Validating → Committed)
//! - `TransactionPhase`: Two-phase commit (Preparing → Prepared → Committing → Committed)
//!
//! ## Layer 3: Validation & Safety
//! - `FailureHandler`: Classify and handle failures
//! - `OptimisticLock`: Prevent concurrent rotations
//! - `RotationBackup`: Disaster recovery
//!
//! ## Layer 4: Events & Observability
//! - `NotificationEvent`: Rotation lifecycle events
//! - `RotationErrorLog`: Detailed failure tracking
//! - Structured logging via `tracing`
//!
//! # Integration Example
//!
//! Complete example showing how to implement rotation for a database credential:
//!
//! ```rust,no_run
//! use nebula_credential::rotation::{
//!     RotationPolicy, PeriodicConfig, RotatableCredential, TestableCredential,
//!     TestResult, RotationResult,
//! };
//! use async_trait::async_trait;
//! use std::time::Duration;
//!
//! // Your credential type
//! struct PostgresCredential {
//!     username: String,
//!     password: String,
//!     host: String,
//!     database: String,
//! }
//!
//! #[async_trait]
//! impl TestableCredential for PostgresCredential {
//!     async fn test(&self) -> RotationResult<TestResult> {
//!         // Test database connection
//!         // let conn = tokio_postgres::connect(&self.connection_string(), ...).await?;
//!         Ok(TestResult::success("Connection successful"))
//!     }
//! }
//!
//! #[async_trait]
//! impl RotatableCredential for PostgresCredential {
//!     async fn rotate(&self) -> RotationResult<Self> {
//!         // Generate new password
//!         let new_password = generate_secure_password();
//!
//!         // Create new database user with same privileges
//!         // let new_username = format!("{}_v{}", self.username, version);
//!         // CREATE USER new_username WITH PASSWORD new_password;
//!         // GRANT ALL PRIVILEGES ON DATABASE ... TO new_username;
//!
//!         Ok(PostgresCredential {
//!             username: format!("{}_rotated", self.username),
//!             password: new_password,
//!             host: self.host.clone(),
//!             database: self.database.clone(),
//!         })
//!     }
//!
//!     async fn cleanup_old(&self) -> RotationResult<()> {
//!         // Drop old database user after grace period
//!         // DROP USER IF EXISTS old_username;
//!         Ok(())
//!     }
//! }
//!
//! # fn generate_secure_password() -> String { "secret".to_string() }
//! ```
//!
//! # Configuration Storage
//!
//! Rotation policies are stored in `CredentialMetadata.rotation_policy`.
//! This allows per-credential configuration without code changes.
//!
//! ```rust,ignore
//! // Store policy in metadata when creating credential
//! let metadata = CredentialMetadata {
//!     rotation_policy: Some(RotationPolicy::Periodic(
//!         PeriodicConfig::new(
//!             Duration::from_secs(90 * 24 * 3600),
//!             Duration::from_secs(24 * 3600),
//!             true,
//!         ).expect("valid config")
//!     )),
//!     ..Default::default()
//! };
//!
//! manager.store(&id, encrypted_data, metadata, &context).await?;
//!
//! // Retrieve policy from metadata during rotation
//! let (data, metadata) = manager.retrieve(&id, &context).await?;
//! if let Some(policy) = metadata.rotation_policy {
//!     // Use policy to schedule rotation
//!     scheduler.schedule_rotation(&id, &policy).await?;
//! }
//! ```
//!
//! For external configuration (database, config file), retrieve the policy
//! and store it in metadata when creating the credential.
//!
//! # See Also
//!
//! - [`RotationPolicy`] - Rotation strategies
//! - [`RotationTransaction`] - Transaction lifecycle
//! - [`BlueGreenRotation`] - Zero-downtime database rotation
//! - [`FailureHandler`] - Failure handling
//! - [`RotatableCredential`] - Trait for rotatable credentials

// Module exports
pub mod backup;
pub mod blue_green;
pub mod error;
pub mod events;
pub mod grace_period;
pub mod metrics;
pub mod policy;
pub mod retry;
pub mod scheduler;
pub mod state;
pub mod transaction;
pub mod validation;

// Re-exports
pub use backup::RotationBackup;
pub use blue_green::{
    BlueGreenRotation, BlueGreenState, DatabasePrivilege, enumerate_required_privileges,
    validate_privileges,
};
pub use error::{RotationError, RotationErrorLog, RotationResult};
pub use events::{
    LogEntryType, NotificationEvent, NotificationSender, TransactionLog, TransactionLogEntry,
    TransactionOutcome, log_rollback_event, send_notification,
};
pub use grace_period::{
    GracePeriodConfig, GracePeriodState, GracePeriodTracker, UsageMetrics,
    cleanup_expired_credentials, track_credential_usage,
};
pub use metrics::{CredentialMetrics, RotationMetrics};
pub use policy::{
    BeforeExpiryConfig, ManualConfig, PeriodicConfig, RotationPolicy, ScheduledConfig,
};
pub use retry::{RotationRetryPolicy, retry_with_backoff};
pub use state::RotationState;
pub use transaction::{
    BackupId, ManualRotation, OptimisticLock, RollbackStrategy, RotationId, RotationTransaction,
    TransactionPhase, ValidationResult,
};
pub use validation::{
    FailureHandler, FailureKind, RotatableCredential, SuccessCriteria, TestContext, TestMethod,
    TestResult, TestableCredential, ValidationTest,
};
