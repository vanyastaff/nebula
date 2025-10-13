//! Resource-based observability configuration
//!
//! This module provides LoggerResource for per-node logging configuration.
//! Resources are scoped to individual nodes, not globally, ensuring security
//! and multi-tenancy isolation.

use serde::{Deserialize, Serialize};
use tracing::Level;

/// Notification preferences for error reporting
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationPrefs {
    /// Send email notifications on errors
    pub email_enabled: bool,
    /// Email addresses to notify (if email_enabled)
    pub email_addresses: Vec<String>,
    /// Send webhook notifications on errors
    pub webhook_enabled: bool,
    /// Minimum severity level for notifications
    pub min_severity: NotificationSeverity,
    /// Rate limit: max notifications per hour (0 = unlimited)
    pub rate_limit_per_hour: u32,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self {
            email_enabled: false,
            email_addresses: Vec::new(),
            webhook_enabled: false,
            min_severity: NotificationSeverity::Error,
            rate_limit_per_hour: 10,
        }
    }
}

/// Notification severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NotificationSeverity {
    /// Informational notifications
    Info,
    /// Warning notifications
    Warning,
    /// Error notifications (default minimum)
    Error,
    /// Critical error notifications
    Critical,
}

/// Logger resource for per-node observability configuration
///
/// This resource is attached to individual nodes, providing isolated,
/// secure configuration for logging, telemetry, and error notifications.
///
/// # Security
///
/// LoggerResource is scoped per-node and stored in NodeContext's ResourceMap.
/// This prevents:
/// - Cross-tenant information leakage
/// - Unauthorized access to other nodes' configurations
/// - Global credential exposure
///
/// # Example
///
/// ```rust
/// use nebula_log::observability::{LoggerResource, NotificationPrefs, NodeContext};
/// use tracing::Level;
///
/// let logger = LoggerResource::new()
///     .with_log_level(Level::DEBUG)
///     .with_sentry_dsn("https://key@sentry.io/project")
///     .with_webhook("https://hooks.slack.com/services/...");
///
/// // Attach to node context (isolated, secure)
/// let ctx = NodeContext::new("my-node", "http.request")
///     .with_resource("LoggerResource", logger);
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggerResource {
    /// Sentry DSN for error reporting (optional)
    pub sentry_dsn: Option<String>,

    /// Webhook URL for notifications (optional)
    pub webhook_url: Option<String>,

    /// Log level for this node
    pub log_level: LogLevel,

    /// Notification preferences
    pub notification_prefs: NotificationPrefs,

    /// Custom tags to attach to all logs from this node
    pub tags: Vec<(String, String)>,

    /// Enable sampling (reduce log volume)
    pub sampling_enabled: bool,

    /// Sampling rate (0.0 - 1.0, where 1.0 = 100% of events)
    pub sampling_rate: f64,
}

/// Serializable log level wrapper
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum LogLevel {
    /// Trace level
    Trace,
    /// Debug level
    Debug,
    /// Info level
    Info,
    /// Warn level
    Warn,
    /// Error level
    Error,
}

impl From<LogLevel> for Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => Level::TRACE,
            LogLevel::Debug => Level::DEBUG,
            LogLevel::Info => Level::INFO,
            LogLevel::Warn => Level::WARN,
            LogLevel::Error => Level::ERROR,
        }
    }
}

impl From<Level> for LogLevel {
    fn from(level: Level) -> Self {
        match level {
            Level::TRACE => LogLevel::Trace,
            Level::DEBUG => LogLevel::Debug,
            Level::INFO => LogLevel::Info,
            Level::WARN => LogLevel::Warn,
            Level::ERROR => LogLevel::Error,
        }
    }
}

impl Default for LoggerResource {
    fn default() -> Self {
        Self {
            sentry_dsn: None,
            webhook_url: None,
            log_level: LogLevel::Info,
            notification_prefs: NotificationPrefs::default(),
            tags: Vec::new(),
            sampling_enabled: false,
            sampling_rate: 1.0,
        }
    }
}

impl LoggerResource {
    /// Create a new LoggerResource with default settings
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the Sentry DSN for error tracking
    pub fn with_sentry_dsn(mut self, dsn: impl Into<String>) -> Self {
        self.sentry_dsn = Some(dsn.into());
        self
    }

    /// Set the webhook URL for notifications
    pub fn with_webhook(mut self, url: impl Into<String>) -> Self {
        self.webhook_url = Some(url.into());
        self
    }

    /// Set the log level
    pub fn with_log_level(mut self, level: impl Into<LogLevel>) -> Self {
        self.log_level = level.into();
        self
    }

    /// Set notification preferences
    pub fn with_notifications(mut self, prefs: NotificationPrefs) -> Self {
        self.notification_prefs = prefs;
        self
    }

    /// Add a custom tag
    pub fn with_tag(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.push((key.into(), value.into()));
        self
    }

    /// Enable sampling with a rate
    pub fn with_sampling(mut self, rate: f64) -> Self {
        self.sampling_enabled = true;
        self.sampling_rate = rate.clamp(0.0, 1.0);
        self
    }

    /// Check if this resource should send notifications for a given severity
    pub fn should_notify(&self, severity: NotificationSeverity) -> bool {
        severity >= self.notification_prefs.min_severity
            && (self.notification_prefs.email_enabled || self.notification_prefs.webhook_enabled)
    }

    /// Get the Sentry DSN if configured
    pub fn sentry_dsn(&self) -> Option<&str> {
        self.sentry_dsn.as_deref()
    }

    /// Get the webhook URL if configured
    pub fn webhook_url(&self) -> Option<&str> {
        self.webhook_url.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_logger_resource_builder() {
        let resource = LoggerResource::new()
            .with_sentry_dsn("https://key@sentry.io/project")
            .with_webhook("https://hooks.slack.com/services/test")
            .with_log_level(LogLevel::Debug)
            .with_tag("environment", "production")
            .with_sampling(0.5);

        assert_eq!(resource.sentry_dsn(), Some("https://key@sentry.io/project"));
        assert_eq!(
            resource.webhook_url(),
            Some("https://hooks.slack.com/services/test")
        );
        assert!(matches!(resource.log_level, LogLevel::Debug));
        assert_eq!(resource.tags.len(), 1);
        assert!(resource.sampling_enabled);
        assert_eq!(resource.sampling_rate, 0.5);
    }

    #[test]
    fn test_notification_severity() {
        let prefs = NotificationPrefs {
            min_severity: NotificationSeverity::Error,
            ..Default::default()
        };

        let resource = LoggerResource::new().with_notifications(prefs);

        assert!(!resource.should_notify(NotificationSeverity::Info));
        assert!(!resource.should_notify(NotificationSeverity::Warning));
        // Note: should_notify also requires email or webhook enabled
        // Since default has both disabled, this will be false
        assert!(!resource.should_notify(NotificationSeverity::Error));
    }

    #[test]
    fn test_notification_enabled() {
        let mut prefs = NotificationPrefs::default();
        prefs.webhook_enabled = true;
        prefs.min_severity = NotificationSeverity::Warning;

        let resource = LoggerResource::new().with_notifications(prefs);

        assert!(!resource.should_notify(NotificationSeverity::Info));
        assert!(resource.should_notify(NotificationSeverity::Warning));
        assert!(resource.should_notify(NotificationSeverity::Error));
        assert!(resource.should_notify(NotificationSeverity::Critical));
    }

    #[test]
    fn test_log_level_conversion() {
        let level: Level = LogLevel::Debug.into();
        assert_eq!(level, Level::DEBUG);

        let log_level: LogLevel = Level::WARN.into();
        assert!(matches!(log_level, LogLevel::Warn));
    }

    #[test]
    fn test_sampling_rate_clamp() {
        let resource1 = LoggerResource::new().with_sampling(1.5);
        assert_eq!(resource1.sampling_rate, 1.0);

        let resource2 = LoggerResource::new().with_sampling(-0.5);
        assert_eq!(resource2.sampling_rate, 0.0);

        let resource3 = LoggerResource::new().with_sampling(0.75);
        assert_eq!(resource3.sampling_rate, 0.75);
    }

    #[test]
    fn test_serialization() {
        let resource = LoggerResource::new()
            .with_sentry_dsn("test-dsn")
            .with_log_level(LogLevel::Debug);

        let json = serde_json::to_string(&resource).unwrap();
        let deserialized: LoggerResource = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.sentry_dsn(), Some("test-dsn"));
        assert!(matches!(deserialized.log_level, LogLevel::Debug));
    }
}
