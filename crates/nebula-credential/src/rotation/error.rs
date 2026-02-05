//! Rotation-specific error types
//!
//! This module defines all errors that can occur during credential rotation.

use thiserror::Error;

use crate::core::CredentialId;

/// Errors that can occur during credential rotation
#[derive(Debug, Error)]
pub enum RotationError {
    /// Policy validation failed
    #[error("Invalid rotation policy: {reason}")]
    InvalidPolicy { reason: String },

    /// State transition is not allowed
    #[error("Invalid state transition from {from} to {to}")]
    InvalidStateTransition { from: String, to: String },

    /// Credential validation failed during rotation
    #[error("Credential validation failed for {credential_id}: {reason}")]
    ValidationFailed {
        credential_id: CredentialId,
        reason: String,
    },

    /// Rotation transaction failed
    #[error("Rotation transaction failed: {reason}")]
    TransactionFailed { reason: String },

    /// Rollback failed
    #[error("Rollback failed for {credential_id}: {reason}")]
    RollbackFailed {
        credential_id: CredentialId,
        reason: String,
    },

    /// Backup creation failed
    #[error("Backup creation failed for {credential_id}: {reason}")]
    BackupFailed {
        credential_id: CredentialId,
        reason: String,
    },

    /// Backup restoration failed
    #[error("Backup restoration failed for backup {backup_id}: {reason}")]
    RestoreFailed { backup_id: String, reason: String },

    /// Scheduler error
    #[error("Scheduler error: {reason}")]
    SchedulerError { reason: String },

    /// Notification sending failed
    #[error("Notification failed for {credential_id}: {reason}")]
    NotificationFailed {
        credential_id: String,
        reason: String,
    },

    /// Grace period error
    #[error("Grace period error: {reason}")]
    GracePeriodError { reason: String },

    /// Timeout during operation
    #[error("Operation timed out after {timeout_secs}s: {operation}")]
    Timeout {
        operation: String,
        timeout_secs: u64,
    },

    /// Storage provider error
    #[error("Storage error: {0}")]
    Storage(#[from] crate::core::CredentialError),

    /// Concurrent rotation detected
    #[error("Rotation already in progress for credential {credential_id}")]
    ConcurrentRotation { credential_id: CredentialId },

    /// Credential not found
    #[error("Credential not found: {credential_id}")]
    CredentialNotFound { credential_id: CredentialId },

    /// Maximum retry attempts exceeded
    #[error("Maximum retry attempts ({max_attempts}) exceeded for {operation}")]
    MaxRetriesExceeded {
        operation: String,
        max_attempts: u32,
    },

    /// Internal error (should not normally occur)
    #[error("Internal rotation error: {0}")]
    Internal(String),
}

/// Result type for rotation operations
pub type RotationResult<T> = Result<T, RotationError>;

/// Rotation error log for detailed failure tracking
///
/// Records comprehensive details about rotation failures for debugging,
/// audit trails, and incident response.
///
/// # T076: Rotation Error Log
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::error::RotationErrorLog;
///
/// let error_log = RotationErrorLog::new(
///     transaction_id,
///     credential_id,
///     "Validation failed: connection timeout",
/// )
/// .with_retry_count(3)
/// .with_error_classification("transient");
///
/// println!("Error: {}", error_log);
/// ```
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RotationErrorLog {
    /// Transaction ID that failed
    pub transaction_id: String,

    /// Credential that was being rotated
    pub credential_id: CredentialId,

    /// Error message
    pub error_message: String,

    /// When the error occurred
    pub occurred_at: chrono::DateTime<chrono::Utc>,

    /// Number of retry attempts made
    pub retry_count: u32,

    /// Error classification (transient, permanent, unknown)
    pub error_classification: Option<String>,

    /// Stack trace or additional context
    pub additional_context: Option<String>,

    /// Whether rollback was triggered
    pub rollback_triggered: bool,

    /// Current rotation state when error occurred
    pub rotation_state: Option<String>,
}

impl RotationErrorLog {
    /// Create a new error log entry
    pub fn new(
        transaction_id: impl Into<String>,
        credential_id: CredentialId,
        error_message: impl Into<String>,
    ) -> Self {
        Self {
            transaction_id: transaction_id.into(),
            credential_id,
            error_message: error_message.into(),
            occurred_at: chrono::Utc::now(),
            retry_count: 0,
            error_classification: None,
            additional_context: None,
            rollback_triggered: false,
            rotation_state: None,
        }
    }

    /// Set retry count
    pub fn with_retry_count(mut self, count: u32) -> Self {
        self.retry_count = count;
        self
    }

    /// Set error classification
    pub fn with_error_classification(mut self, classification: impl Into<String>) -> Self {
        self.error_classification = Some(classification.into());
        self
    }

    /// Set additional context
    pub fn with_context(mut self, context: impl Into<String>) -> Self {
        self.additional_context = Some(context.into());
        self
    }

    /// Mark that rollback was triggered
    pub fn with_rollback_triggered(mut self) -> Self {
        self.rollback_triggered = true;
        self
    }

    /// Set rotation state when error occurred
    pub fn with_rotation_state(mut self, state: impl Into<String>) -> Self {
        self.rotation_state = Some(state.into());
        self
    }
}

impl std::fmt::Display for RotationErrorLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Rotation Error [{}] for credential {} at {}: {} (retries: {}, rollback: {})",
            self.transaction_id,
            self.credential_id,
            self.occurred_at,
            self.error_message,
            self.retry_count,
            self.rollback_triggered
        )
    }
}
