//! Rotation Notification Events
//!
//! Provides event types and notification abstraction for rotation lifecycle events.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::RotationResult;
use crate::core::CredentialId;

/// Rollback data (boxed to reduce enum size)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackData {
    /// Credential that had rotation rolled back
    pub credential_id: CredentialId,

    /// When rollback occurred
    pub rolled_back_at: DateTime<Utc>,

    /// Rotation transaction ID
    pub transaction_id: String,

    /// Reason for rollback
    pub reason: String,

    /// Error classification (transient, permanent, unknown)
    pub error_classification: Option<String>,

    /// Number of retries attempted before rollback
    pub retry_count: u32,

    /// Rotation state when rollback was triggered
    pub rotation_state: String,
}

/// Emergency rotation data (boxed to reduce enum size)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmergencyRotationData {
    /// Credential being rotated
    pub credential_id: CredentialId,

    /// When emergency was triggered
    pub triggered_at: DateTime<Utc>,

    /// Rotation transaction ID
    pub transaction_id: String,

    /// Reason for emergency rotation
    pub reason: String,

    /// Who triggered the emergency rotation
    pub triggered_by: String,

    /// Incident tracking ID (if linked)
    pub incident_id: Option<String>,

    /// Whether old credential was immediately revoked
    pub immediate_revoke: bool,
}

/// Notification event for rotation lifecycle
///
/// Events are emitted at key points during rotation:
/// - Scheduled: When rotation is scheduled (with advance notice)
/// - Starting: Just before rotation begins
/// - Complete: After successful rotation
/// - Failed: If rotation fails
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NotificationEvent {
    /// Rotation has been scheduled
    RotationScheduled {
        /// Credential being rotated
        credential_id: CredentialId,

        /// When rotation will occur
        scheduled_at: DateTime<Utc>,

        /// Time remaining until rotation
        time_until: std::time::Duration,
    },

    /// Rotation is about to start
    RotationStarting {
        /// Credential being rotated
        credential_id: CredentialId,

        /// Current timestamp
        starting_at: DateTime<Utc>,

        /// Rotation transaction ID
        transaction_id: String,
    },

    /// Rotation completed successfully
    RotationComplete {
        /// Credential that was rotated
        credential_id: CredentialId,

        /// When rotation completed
        completed_at: DateTime<Utc>,

        /// Rotation transaction ID
        transaction_id: String,

        /// Duration of rotation process
        duration: std::time::Duration,

        /// Old version number
        old_version: u32,

        /// New version number
        new_version: u32,
    },

    /// Rotation failed
    RotationFailed {
        /// Credential that failed rotation
        credential_id: CredentialId,

        /// When rotation failed
        failed_at: DateTime<Utc>,

        /// Rotation transaction ID
        transaction_id: String,

        /// Error message
        error: String,

        /// Retry attempt number (0 for first attempt)
        retry_attempt: u32,
    },

    /// Emergency manual rotation triggered (security incident)
    EmergencyRotation(Box<EmergencyRotationData>),

    /// Rotation was rolled back due to failure
    ///
    /// # T077: Rollback Event Logging
    RotationRolledBack(Box<RollbackData>),
}

impl NotificationEvent {
    /// Get the credential ID associated with this event
    pub fn credential_id(&self) -> &CredentialId {
        match self {
            NotificationEvent::RotationScheduled { credential_id, .. } => credential_id,
            NotificationEvent::RotationStarting { credential_id, .. } => credential_id,
            NotificationEvent::RotationComplete { credential_id, .. } => credential_id,
            NotificationEvent::RotationFailed { credential_id, .. } => credential_id,
            NotificationEvent::EmergencyRotation(data) => &data.credential_id,
            NotificationEvent::RotationRolledBack(data) => &data.credential_id,
        }
    }

    /// Get the event timestamp
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            NotificationEvent::RotationScheduled { scheduled_at, .. } => *scheduled_at,
            NotificationEvent::RotationStarting { starting_at, .. } => *starting_at,
            NotificationEvent::RotationComplete { completed_at, .. } => *completed_at,
            NotificationEvent::RotationFailed { failed_at, .. } => *failed_at,
            NotificationEvent::EmergencyRotation(data) => data.triggered_at,
            NotificationEvent::RotationRolledBack(data) => data.rolled_back_at,
        }
    }

    /// Get a human-readable event description
    pub fn description(&self) -> String {
        match self {
            NotificationEvent::RotationScheduled {
                credential_id,
                scheduled_at,
                ..
            } => {
                format!(
                    "Rotation scheduled for credential {} at {}",
                    credential_id, scheduled_at
                )
            }
            NotificationEvent::RotationStarting {
                credential_id,
                transaction_id,
                ..
            } => {
                format!(
                    "Rotation starting for credential {} (transaction: {})",
                    credential_id, transaction_id
                )
            }
            NotificationEvent::RotationComplete {
                credential_id,
                old_version,
                new_version,
                ..
            } => {
                format!(
                    "Rotation complete for credential {} (v{} → v{})",
                    credential_id, old_version, new_version
                )
            }
            NotificationEvent::RotationFailed {
                credential_id,
                error,
                retry_attempt,
                ..
            } => {
                format!(
                    "Rotation failed for credential {} (attempt {}): {}",
                    credential_id,
                    retry_attempt + 1,
                    error
                )
            }
            NotificationEvent::EmergencyRotation(data) => {
                let revoke_status = if data.immediate_revoke {
                    "IMMEDIATE REVOCATION"
                } else {
                    "with grace period"
                };
                let incident_info = data
                    .incident_id
                    .as_ref()
                    .map(|id| format!(" [Incident: {}]", id))
                    .unwrap_or_default();
                format!(
                    "🚨 EMERGENCY: {} rotation for credential {} by {} - Reason: {}{}",
                    revoke_status,
                    data.credential_id,
                    data.triggered_by,
                    data.reason,
                    incident_info
                )
            }
            NotificationEvent::RotationRolledBack(data) => {
                let classification = data
                    .error_classification
                    .as_ref()
                    .map(|c| format!(" [{}]", c))
                    .unwrap_or_default();
                format!(
                    "🔄 ROLLBACK: Rotation rolled back for credential {} after {} retries in state '{}'{} - Reason: {}",
                    data.credential_id,
                    data.retry_count,
                    data.rotation_state,
                    classification,
                    data.reason
                )
            }
        }
    }
}

/// Trait for notification senders
///
/// Implement this trait to send notifications via different channels:
/// - Email (SMTP)
/// - Slack webhooks
/// - PagerDuty
/// - Custom logging
/// - Metrics/monitoring systems
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::events::{NotificationSender, NotificationEvent};
///
/// pub struct SlackNotifier {
///     webhook_url: String,
/// }
///
/// #[async_trait]
/// impl NotificationSender for SlackNotifier {
///     async fn send(&self, event: &NotificationEvent) -> RotationResult<()> {
///         let payload = json!({
///             "text": event.description(),
///             "username": "Credential Rotation Bot",
///         });
///
///         reqwest::Client::new()
///             .post(&self.webhook_url)
///             .json(&payload)
///             .send()
///             .await?;
///
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait NotificationSender: Send + Sync {
    /// Send a notification event
    ///
    /// # Arguments
    ///
    /// * `event` - The notification event to send
    ///
    /// # Returns
    ///
    /// * `RotationResult<()>` - Success or error
    async fn send(&self, event: &NotificationEvent) -> RotationResult<()>;
}

/// Send notification with retry logic
///
/// Attempts to send notification with exponential backoff retry on failure.
///
/// # Arguments
///
/// * `sender` - Notification sender implementation
/// * `event` - Event to send
/// * `policy` - Retry policy configuration
///
/// # Returns
///
/// * `RotationResult<()>` - Success or final error after retries
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::events::{send_notification, NotificationEvent};
/// use nebula_credential::rotation::RotationRetryPolicy;
///
/// let policy = RotationRetryPolicy::default();
/// let event = NotificationEvent::RotationComplete { /* ... */ };
/// send_notification(&slack_notifier, &event, &policy).await?;
/// ```
pub async fn send_notification<S: NotificationSender>(
    sender: &S,
    event: &NotificationEvent,
    policy: &super::retry::RotationRetryPolicy,
) -> RotationResult<()> {
    use super::retry::retry_with_backoff;

    retry_with_backoff(policy, "send_notification", || async {
        sender.send(event).await
    })
    .await
}

/// Log a rollback event with structured logging
///
/// # T077: Log Rollback Event
///
/// Creates a structured log entry and notification event for rollback operations.
///
/// # Arguments
///
/// * `error_log` - Detailed error log from the rotation failure
/// * `sender` - Optional notification sender for alerting
/// * `policy` - Retry policy for notification delivery
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::events::log_rollback_event;
/// use nebula_credential::rotation::error::RotationErrorLog;
///
/// let error_log = RotationErrorLog::new(
///     transaction_id,
///     credential_id,
///     "Validation failed",
/// )
/// .with_rollback_triggered()
/// .with_retry_count(3);
///
/// log_rollback_event(&error_log, Some(&notifier), &retry_policy).await?;
/// ```
pub async fn log_rollback_event<S: NotificationSender>(
    error_log: &super::error::RotationErrorLog,
    sender: Option<&S>,
    policy: &super::retry::RotationRetryPolicy,
) -> RotationResult<()> {
    use tracing::warn;

    // Log to tracing system
    warn!(
        transaction_id = %error_log.transaction_id,
        credential_id = %error_log.credential_id,
        retry_count = error_log.retry_count,
        error_classification = ?error_log.error_classification,
        rotation_state = ?error_log.rotation_state,
        "Rotation rolled back due to failure: {}",
        error_log.error_message
    );

    // Send notification if sender provided
    if let Some(sender) = sender {
        let event = NotificationEvent::RotationRolledBack(Box::new(RollbackData {
            credential_id: error_log.credential_id.clone(),
            rolled_back_at: error_log.occurred_at,
            transaction_id: error_log.transaction_id.clone(),
            reason: error_log.error_message.clone(),
            error_classification: error_log.error_classification.clone(),
            retry_count: error_log.retry_count,
            rotation_state: error_log
                .rotation_state
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
        }));

        send_notification(sender, &event, policy).await?;
    }

    Ok(())
}

/// Transaction audit log entry
///
/// # T094: TransactionLog for Audit Trail
///
/// Records all state transitions and events during a rotation transaction
/// for audit, compliance, and debugging purposes.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_credential::rotation::events::TransactionLog;
/// use nebula_credential::rotation::state::RotationState;
///
/// let mut log = TransactionLog::new(
///     transaction_id.clone(),
///     credential_id.clone(),
/// );
///
/// // Log state transition
/// log.log_transition(RotationState::Creating, "Starting credential creation");
///
/// // Log validation
/// log.log_validation_result(true, "Connection test passed");
///
/// // Check if log contains errors
/// if log.has_errors() {
///     eprintln!("Transaction had errors: {:?}", log.get_error_entries());
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionLog {
    /// Transaction ID this log belongs to
    pub transaction_id: String,

    /// Credential ID being rotated
    pub credential_id: CredentialId,

    /// Log entries (chronological order)
    pub entries: Vec<TransactionLogEntry>,

    /// When transaction started
    pub started_at: DateTime<Utc>,

    /// When transaction completed (if finished)
    pub completed_at: Option<DateTime<Utc>>,

    /// Final transaction outcome
    pub outcome: Option<TransactionOutcome>,
}

/// Individual log entry in transaction audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransactionLogEntry {
    /// When this entry was created
    pub timestamp: DateTime<Utc>,

    /// Entry type
    pub entry_type: LogEntryType,

    /// Entry message
    pub message: String,

    /// Additional context (JSON-serializable)
    pub context: Option<serde_json::Value>,
}

/// Type of log entry
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LogEntryType {
    /// State transition occurred
    StateTransition,

    /// Validation was performed
    Validation,

    /// Error occurred
    Error,

    /// Warning issued
    Warning,

    /// Informational message
    Info,

    /// Rollback triggered
    Rollback,

    /// Commit succeeded
    Commit,
}

/// Final outcome of transaction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransactionOutcome {
    /// Transaction committed successfully
    Committed,

    /// Transaction was rolled back
    RolledBack,

    /// Transaction was aborted
    Aborted,

    /// Transaction is still in progress
    InProgress,
}

impl TransactionLog {
    /// Create a new transaction log
    pub fn new(transaction_id: String, credential_id: CredentialId) -> Self {
        Self {
            transaction_id,
            credential_id,
            entries: Vec::new(),
            started_at: Utc::now(),
            completed_at: None,
            outcome: None,
        }
    }

    /// Add a log entry
    fn add_entry(&mut self, entry_type: LogEntryType, message: impl Into<String>) {
        self.entries.push(TransactionLogEntry {
            timestamp: Utc::now(),
            entry_type,
            message: message.into(),
            context: None,
        });
    }

    /// Add a log entry with context
    fn add_entry_with_context(
        &mut self,
        entry_type: LogEntryType,
        message: impl Into<String>,
        context: serde_json::Value,
    ) {
        self.entries.push(TransactionLogEntry {
            timestamp: Utc::now(),
            entry_type,
            message: message.into(),
            context: Some(context),
        });
    }

    /// Log a state transition
    pub fn log_transition(
        &mut self,
        new_state: super::state::RotationState,
        message: impl Into<String>,
    ) {
        let context = serde_json::json!({
            "new_state": format!("{:?}", new_state),
        });
        self.add_entry_with_context(LogEntryType::StateTransition, message, context);
    }

    /// Log a validation result
    pub fn log_validation_result(&mut self, passed: bool, message: impl Into<String>) {
        let context = serde_json::json!({
            "passed": passed,
        });
        self.add_entry_with_context(LogEntryType::Validation, message, context);
    }

    /// Log an error
    pub fn log_error(&mut self, message: impl Into<String>) {
        self.add_entry(LogEntryType::Error, message);
    }

    /// Log a warning
    pub fn log_warning(&mut self, message: impl Into<String>) {
        self.add_entry(LogEntryType::Warning, message);
    }

    /// Log info message
    pub fn log_info(&mut self, message: impl Into<String>) {
        self.add_entry(LogEntryType::Info, message);
    }

    /// Log rollback event
    pub fn log_rollback(&mut self, reason: impl Into<String>) {
        self.add_entry(LogEntryType::Rollback, reason);
        self.completed_at = Some(Utc::now());
        self.outcome = Some(TransactionOutcome::RolledBack);
    }

    /// Log commit event
    pub fn log_commit(&mut self) {
        self.add_entry(LogEntryType::Commit, "Transaction committed successfully");
        self.completed_at = Some(Utc::now());
        self.outcome = Some(TransactionOutcome::Committed);
    }

    /// Check if log contains any errors
    pub fn has_errors(&self) -> bool {
        self.entries
            .iter()
            .any(|e| e.entry_type == LogEntryType::Error)
    }

    /// Get all error entries
    pub fn get_error_entries(&self) -> Vec<&TransactionLogEntry> {
        self.entries
            .iter()
            .filter(|e| e.entry_type == LogEntryType::Error)
            .collect()
    }

    /// Get transaction duration (if completed)
    pub fn duration(&self) -> Option<chrono::Duration> {
        self.completed_at.map(|end| end - self.started_at)
    }

    /// Get total entry count
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_notification_event_credential_id() {
        let cred_id = CredentialId::new("test-cred").unwrap();

        let event = NotificationEvent::RotationScheduled {
            credential_id: cred_id.clone(),
            scheduled_at: Utc::now(),
            time_until: std::time::Duration::from_secs(3600),
        };

        assert_eq!(event.credential_id(), &cred_id);
    }

    #[test]
    fn test_notification_event_description() {
        let cred_id = CredentialId::new("test-cred").unwrap();

        let event = NotificationEvent::RotationComplete {
            credential_id: cred_id.clone(),
            completed_at: Utc::now(),
            transaction_id: "tx-123".to_string(),
            duration: std::time::Duration::from_secs(30),
            old_version: 1,
            new_version: 2,
        };

        let desc = event.description();
        assert!(desc.contains("test-cred"));
        assert!(desc.contains("v1"));
        assert!(desc.contains("v2"));
    }

    // Mock notification sender for testing
    struct MockSender {
        should_fail: bool,
    }

    #[async_trait]
    impl NotificationSender for MockSender {
        async fn send(&self, _event: &NotificationEvent) -> RotationResult<()> {
            if self.should_fail {
                Err(super::super::error::RotationError::NotificationFailed {
                    credential_id: "test".to_string(),
                    reason: "Mock failure".to_string(),
                })
            } else {
                Ok(())
            }
        }
    }

    #[tokio::test]
    async fn test_send_notification_success() {
        use super::super::retry::RotationRetryPolicy;

        let sender = MockSender { should_fail: false };
        let event = NotificationEvent::RotationStarting {
            credential_id: CredentialId::new("test").unwrap(),
            starting_at: Utc::now(),
            transaction_id: "tx-123".to_string(),
        };

        let policy = RotationRetryPolicy::default();
        let result = send_notification(&sender, &event, &policy).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_notification_failure() {
        use super::super::retry::RotationRetryPolicy;

        let sender = MockSender { should_fail: true };
        let event = NotificationEvent::RotationStarting {
            credential_id: CredentialId::new("test").unwrap(),
            starting_at: Utc::now(),
            transaction_id: "tx-123".to_string(),
        };

        let policy = RotationRetryPolicy::default();
        let result = send_notification(&sender, &event, &policy).await;
        assert!(result.is_err());
    }
}
