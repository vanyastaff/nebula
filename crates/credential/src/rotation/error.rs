//! Rotation-specific error types and validation framework.
//!
//! This module defines all errors that can occur during credential rotation,
//! plus the validation traits and types for testing credentials during rotation.

use std::{future::Future, time::Duration};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::time::timeout;

use crate::{CredentialId, CredentialRecord};

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
    Storage(#[from] crate::error::CredentialError),

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

impl nebula_error::Classify for RotationError {
    fn category(&self) -> nebula_error::ErrorCategory {
        match self {
            Self::InvalidPolicy { .. }
            | Self::InvalidStateTransition { .. }
            | Self::ValidationFailed { .. } => nebula_error::ErrorCategory::Validation,
            Self::TransactionFailed { .. }
            | Self::RollbackFailed { .. }
            | Self::BackupFailed { .. }
            | Self::RestoreFailed { .. }
            | Self::SchedulerError { .. }
            | Self::Internal(_) => nebula_error::ErrorCategory::Internal,
            Self::NotificationFailed { .. } | Self::GracePeriodError { .. } => {
                nebula_error::ErrorCategory::External
            },
            Self::Timeout { .. } => nebula_error::ErrorCategory::Timeout,
            Self::Storage(e) => nebula_error::Classify::category(e),
            Self::ConcurrentRotation { .. } => nebula_error::ErrorCategory::Conflict,
            Self::CredentialNotFound { .. } => nebula_error::ErrorCategory::NotFound,
            Self::MaxRetriesExceeded { .. } => nebula_error::ErrorCategory::Exhausted,
        }
    }

    fn code(&self) -> nebula_error::ErrorCode {
        nebula_error::ErrorCode::new(match self {
            Self::InvalidPolicy { .. } => "ROTATION:INVALID_POLICY",
            Self::InvalidStateTransition { .. } => "ROTATION:INVALID_TRANSITION",
            Self::ValidationFailed { .. } => "ROTATION:VALIDATION",
            Self::TransactionFailed { .. } => "ROTATION:TRANSACTION",
            Self::RollbackFailed { .. } => "ROTATION:ROLLBACK",
            Self::BackupFailed { .. } => "ROTATION:BACKUP",
            Self::RestoreFailed { .. } => "ROTATION:RESTORE",
            Self::SchedulerError { .. } => "ROTATION:SCHEDULER",
            Self::NotificationFailed { .. } => "ROTATION:NOTIFICATION",
            Self::GracePeriodError { .. } => "ROTATION:GRACE_PERIOD",
            Self::Timeout { .. } => "ROTATION:TIMEOUT",
            Self::Storage(_) => "ROTATION:STORAGE",
            Self::ConcurrentRotation { .. } => "ROTATION:CONCURRENT",
            Self::CredentialNotFound { .. } => "ROTATION:NOT_FOUND",
            Self::MaxRetriesExceeded { .. } => "ROTATION:MAX_RETRIES",
            Self::Internal(_) => "ROTATION:INTERNAL",
        })
    }

    fn is_retryable(&self) -> bool {
        matches!(self, Self::Timeout { .. } | Self::NotificationFailed { .. })
    }
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

// ── Validation traits and types ───────────────────────────────────────────

/// Trait for credentials that can test themselves during rotation.
///
/// Each credential type implements this using their client library:
/// - **MySQL/PostgreSQL**: `SELECT 1` query
/// - **OAuth2**: Call userinfo endpoint with token
/// - **API Key**: Call account/status endpoint
/// - **Certificate**: Perform TLS handshake
pub trait TestableCredential: Send + Sync {
    /// Test the credential by performing an actual operation.
    ///
    /// Returns `TestResult` with success/failure details.
    fn test(&self) -> impl Future<Output = RotationResult<TestResult>> + Send;

    /// Get test timeout (default: 30 seconds).
    fn test_timeout(&self) -> Duration {
        Duration::from_secs(30)
    }
}

/// Trait for credentials that support rotation.
///
/// Credentials implementing this trait can generate new versions and
/// cleanup old ones.
pub trait RotatableCredential: TestableCredential {
    /// Generate a new version of this credential.
    ///
    /// The new credential should have:
    /// - Different secrets (password, token, key)
    /// - Same permissions and access levels
    /// - Same connection details (host, port, database)
    fn rotate(&self) -> impl Future<Output = RotationResult<Self>> + Send
    where
        Self: Sized;

    /// Clean up old credential after rotation completes.
    ///
    /// Optional: implement if cleanup is needed (e.g., delete old database user).
    /// Called after grace period expires.
    fn cleanup_old(&self) -> impl Future<Output = RotationResult<()>> + Send {
        async { Ok(()) }
    }
}

// ── TestContext ────────────────────────────────────────────────────────────

/// Context for credential testing
#[derive(Debug, Clone)]
pub struct TestContext {
    /// Credential being validated
    pub credential_id: CredentialId,

    /// Credential record (runtime state)
    pub record: CredentialRecord,

    /// Timeout for validation
    pub timeout: Duration,

    /// Whether this is a retry attempt
    pub is_retry: bool,

    /// Retry attempt number (if retry)
    pub retry_attempt: u32,
}

impl TestContext {
    /// Create a new test context
    pub fn new(credential_id: CredentialId, record: CredentialRecord) -> Self {
        Self {
            credential_id,
            record,
            timeout: Duration::from_secs(30), // Default 30s timeout
            is_retry: false,
            retry_attempt: 0,
        }
    }

    /// Set custom timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Mark as retry attempt
    pub fn with_retry(mut self, attempt: u32) -> Self {
        self.is_retry = true;
        self.retry_attempt = attempt;
        self
    }

    /// Test a credential with timeout enforcement
    ///
    /// Wraps the credential's `test()` method with a timeout to prevent
    /// testing from hanging indefinitely.
    ///
    /// # Arguments
    ///
    /// * `credential` - The credential to test
    ///
    /// # Returns
    ///
    /// * `Ok(TestResult)` - Test completed within timeout
    /// * `Err(RotationError::Timeout)` - Test exceeded timeout
    pub async fn test<T: TestableCredential>(&self, credential: &T) -> RotationResult<TestResult> {
        let timeout_duration = self.timeout;

        tracing::debug!(
            credential_id = %self.credential_id,
            timeout_secs = timeout_duration.as_secs(),
            is_retry = self.is_retry,
            retry_attempt = self.retry_attempt,
            "Starting credential validation with timeout"
        );

        if let Ok(result) = timeout(timeout_duration, credential.test()).await {
            result
        } else {
            tracing::error!(
                credential_id = %self.credential_id,
                timeout_secs = timeout_duration.as_secs(),
                "Credential validation timed out"
            );
            Err(RotationError::Timeout {
                operation: "credential_validation".to_string(),
                timeout_secs: timeout_duration.as_secs(),
            })
        }
    }
}

/// Result of credential testing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    /// Whether validation passed
    pub passed: bool,

    /// Validation message
    pub message: String,

    /// Validation method used (e.g., "SELECT 1", "userinfo", "TLS handshake")
    pub method: String,

    /// Duration of validation
    pub duration: Duration,
}

impl TestResult {
    /// Create successful test result
    pub fn success(
        message: impl Into<String>,
        method: impl Into<String>,
        duration: Duration,
    ) -> Self {
        Self {
            passed: true,
            message: message.into(),
            method: method.into(),
            duration,
        }
    }

    /// Create failed test result
    pub fn failure(
        message: impl Into<String>,
        method: impl Into<String>,
        duration: Duration,
    ) -> Self {
        Self {
            passed: false,
            message: message.into(),
            method: method.into(),
            duration,
        }
    }
}

/// Validation test definition (for future use)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationTest {
    /// Test method
    pub test_method: TestMethod,

    /// Test endpoint or query
    pub endpoint: String,

    /// Expected success criteria
    pub expected_criteria: SuccessCriteria,

    /// Validation timeout
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
}

/// Test method for validation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TestMethod {
    /// HTTP request test
    HttpRequest {
        method: String,
        headers: Vec<(String, String)>,
    },

    /// Database query test
    DatabaseQuery { query: String },

    /// TLS handshake test
    TlsHandshake { hostname: String, port: u16 },

    /// Custom test
    Custom { command: String },
}

/// Success criteria for validation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SuccessCriteria {
    /// HTTP 2xx response
    HttpSuccess,

    /// Query returns result
    QuerySuccess,

    /// Valid TLS handshake
    HandshakeSuccess,

    /// Custom criteria
    Custom { description: String },
}

/// Validation failure classification
///
/// Categorizes validation failures to determine appropriate response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// Network connectivity issue (transient - may succeed on retry)
    NetworkError,

    /// Authentication failure (likely permanent - credential invalid)
    AuthenticationError,

    /// Authorization failure (permission issue - likely permanent)
    AuthorizationError,

    /// Timeout during validation (transient - may succeed on retry)
    Timeout,

    /// Invalid credential format (permanent - need new credential)
    InvalidFormat,

    /// Service unavailable (transient - may succeed on retry)
    ServiceUnavailable,

    /// Unknown error (default - treat as permanent)
    Unknown,
}

impl FailureKind {
    /// Check if failure is transient (worth retrying)
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            FailureKind::NetworkError | FailureKind::Timeout | FailureKind::ServiceUnavailable
        )
    }

    /// Check if failure is permanent (should trigger rollback)
    pub fn is_permanent(&self) -> bool {
        !self.is_transient()
    }
}

/// Validation failure handler
///
/// Analyzes validation failures and determines appropriate response.
#[derive(Debug, Clone)]
pub struct FailureHandler {
    /// Maximum retry attempts for transient failures
    pub max_retries: u32,

    /// Whether to auto-rollback on permanent failures
    pub auto_rollback: bool,
}

impl Default for FailureHandler {
    fn default() -> Self {
        Self {
            max_retries: 3,
            auto_rollback: true,
        }
    }
}

impl FailureHandler {
    /// Create a new validation failure handler
    pub fn new() -> Self {
        Self::default()
    }

    /// Classify an error message into failure type
    ///
    /// Uses heuristics based on error message content.
    pub fn classify_error(&self, error_message: &str) -> FailureKind {
        let error_lower = error_message.to_lowercase();

        if error_lower.contains("timeout")
            || error_lower.contains("timed out")
            || error_lower.contains("deadline exceeded")
        {
            return FailureKind::Timeout;
        }

        if error_lower.contains("network")
            || error_lower.contains("connection refused")
            || error_lower.contains("connection reset")
            || error_lower.contains("dns")
        {
            return FailureKind::NetworkError;
        }

        if error_lower.contains("authentication")
            || error_lower.contains("auth failed")
            || error_lower.contains("invalid credentials")
            || error_lower.contains("unauthorized")
            || error_lower.contains("401")
        {
            return FailureKind::AuthenticationError;
        }

        if error_lower.contains("authorization")
            || error_lower.contains("permission denied")
            || error_lower.contains("access denied")
            || error_lower.contains("forbidden")
            || error_lower.contains("403")
        {
            return FailureKind::AuthorizationError;
        }

        if error_lower.contains("service unavailable")
            || error_lower.contains("503")
            || error_lower.contains("temporarily unavailable")
        {
            return FailureKind::ServiceUnavailable;
        }

        if error_lower.contains("invalid format")
            || error_lower.contains("malformed")
            || error_lower.contains("parse error")
        {
            return FailureKind::InvalidFormat;
        }

        FailureKind::Unknown
    }

    /// Determine if rollback should be triggered
    ///
    /// # Arguments
    ///
    /// * `failure_type` - Type of validation failure
    /// * `retry_count` - Number of retries already attempted
    ///
    /// # Returns
    ///
    /// * `bool` - True if rollback should be triggered
    pub fn should_trigger_rollback(&self, failure_type: &FailureKind, retry_count: u32) -> bool {
        // Always rollback if auto-rollback is disabled
        if !self.auto_rollback {
            return false;
        }

        // Permanent failures trigger immediate rollback
        if failure_type.is_permanent() {
            return true;
        }

        // Transient failures trigger rollback after max retries
        if failure_type.is_transient() && retry_count >= self.max_retries {
            return true;
        }

        false
    }

    /// Check if retry should be attempted
    ///
    /// # Arguments
    ///
    /// * `failure_type` - Type of validation failure
    /// * `retry_count` - Number of retries already attempted
    ///
    /// # Returns
    ///
    /// * `bool` - True if retry should be attempted
    pub fn should_retry(&self, failure_type: &FailureKind, retry_count: u32) -> bool {
        failure_type.is_transient() && retry_count < self.max_retries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mock credential for testing
    struct MockCredential {
        should_pass: bool,
    }

    impl TestableCredential for MockCredential {
        async fn test(&self) -> RotationResult<TestResult> {
            let start = std::time::Instant::now();
            let duration = start.elapsed();

            if self.should_pass {
                Ok(TestResult::success(
                    "Mock test passed",
                    "mock_test",
                    duration,
                ))
            } else {
                Ok(TestResult::failure(
                    "Mock test failed",
                    "mock_test",
                    duration,
                ))
            }
        }
    }

    impl RotatableCredential for MockCredential {
        async fn rotate(&self) -> RotationResult<Self> {
            Ok(MockCredential {
                should_pass: self.should_pass,
            })
        }
    }

    #[tokio::test]
    async fn test_testable_credential_success() {
        let cred = MockCredential { should_pass: true };
        let outcome = cred.test().await.unwrap();
        assert!(outcome.passed);
        assert_eq!(outcome.method, "mock_test");
    }

    #[tokio::test]
    async fn test_testable_credential_failure() {
        let cred = MockCredential { should_pass: false };
        let outcome = cred.test().await.unwrap();
        assert!(!outcome.passed);
    }

    #[tokio::test]
    async fn test_rotatable_credential() {
        let cred = MockCredential { should_pass: true };
        let new_cred = cred.rotate().await.unwrap();
        assert!(new_cred.should_pass);

        // Cleanup should succeed
        assert!(cred.cleanup_old().await.is_ok());
    }

    #[tokio::test]
    async fn test_validation_context() {
        let cred_id = CredentialId::new();
        let record = CredentialRecord {
            created_at: chrono::Utc::now(),
            last_accessed: None,
            last_modified: chrono::Utc::now(),
            owner_scope: None,
            rotation_policy: None,
            version: 1,
            expires_at: None,
            ttl_seconds: None,
            tags: std::collections::HashMap::new(),
        };

        let context = TestContext::new(cred_id, record)
            .with_timeout(Duration::from_secs(10))
            .with_retry(2);

        assert_eq!(context.credential_id, cred_id);
        assert_eq!(context.timeout, Duration::from_secs(10));
        assert!(context.is_retry);
        assert_eq!(context.retry_attempt, 2);
    }

    #[test]
    fn test_validation_failure_type_classification() {
        assert!(FailureKind::NetworkError.is_transient());
        assert!(FailureKind::Timeout.is_transient());
        assert!(FailureKind::ServiceUnavailable.is_transient());

        assert!(FailureKind::AuthenticationError.is_permanent());
        assert!(FailureKind::AuthorizationError.is_permanent());
        assert!(FailureKind::InvalidFormat.is_permanent());
        assert!(FailureKind::Unknown.is_permanent());
    }

    #[test]
    fn test_validation_failure_handler_classify_timeout() {
        let handler = FailureHandler::new();

        let result = handler.classify_error("Connection timeout");
        assert_eq!(result, FailureKind::Timeout);

        let result = handler.classify_error("Operation timed out");
        assert_eq!(result, FailureKind::Timeout);
    }

    #[test]
    fn test_validation_failure_handler_classify_network() {
        let handler = FailureHandler::new();

        let result = handler.classify_error("Network error occurred");
        assert_eq!(result, FailureKind::NetworkError);

        let result = handler.classify_error("Connection refused");
        assert_eq!(result, FailureKind::NetworkError);
    }

    #[test]
    fn test_validation_failure_handler_classify_auth() {
        let handler = FailureHandler::new();

        let result = handler.classify_error("Authentication failed");
        assert_eq!(result, FailureKind::AuthenticationError);

        let result = handler.classify_error("Invalid credentials");
        assert_eq!(result, FailureKind::AuthenticationError);
    }

    #[test]
    fn test_validation_failure_handler_should_trigger_rollback() {
        let handler = FailureHandler::new();

        // Permanent failures trigger immediate rollback
        assert!(handler.should_trigger_rollback(&FailureKind::AuthenticationError, 0));
        assert!(handler.should_trigger_rollback(&FailureKind::InvalidFormat, 0));

        // Transient failures don't trigger rollback until max retries
        assert!(!handler.should_trigger_rollback(&FailureKind::Timeout, 0));
        assert!(!handler.should_trigger_rollback(&FailureKind::Timeout, 2));
        assert!(handler.should_trigger_rollback(&FailureKind::Timeout, 3));
    }

    #[test]
    fn test_validation_failure_handler_should_retry() {
        let handler = FailureHandler::new();

        // Transient failures should retry
        assert!(handler.should_retry(&FailureKind::Timeout, 0));
        assert!(handler.should_retry(&FailureKind::NetworkError, 2));
        assert!(!handler.should_retry(&FailureKind::Timeout, 3));

        // Permanent failures should not retry
        assert!(!handler.should_retry(&FailureKind::AuthenticationError, 0));
        assert!(!handler.should_retry(&FailureKind::InvalidFormat, 0));
    }
}
