//! Webhook action trait

use crate::{Result, TriggerCtx, WebhookPayload};
use async_trait::async_trait;
use std::time::Duration;

/// Result of a test operation
#[derive(Debug, Clone)]
pub struct TestResult {
    /// Whether the test was successful
    pub success: bool,

    /// Human-readable message about the test result
    pub message: String,

    /// Optional sample event for UI preview
    pub sample_event: Option<serde_json::Value>,

    /// Latency of the test operation
    pub latency: Option<Duration>,
}

impl TestResult {
    /// Create a successful test result
    pub fn success(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            sample_event: None,
            latency: None,
        }
    }

    /// Create a failed test result
    pub fn failure(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            sample_event: None,
            latency: None,
        }
    }

    /// Add a sample event to the test result
    pub fn with_sample(mut self, sample: serde_json::Value) -> Self {
        self.sample_event = Some(sample);
        self
    }

    /// Add latency information to the test result
    pub fn with_latency(mut self, latency: Duration) -> Self {
        self.latency = Some(latency);
        self
    }
}

/// Trait for webhook-triggered actions
///
/// Implementers define how to handle webhook lifecycle:
/// - `on_subscribe`: Register webhook with external provider
/// - `on_webhook`: Verify and parse incoming requests
/// - `on_unsubscribe`: Clean up webhook registration
/// - `test`: Verify configuration without side effects
///
/// # Example
///
/// ```no_run
/// use nebula_webhook::prelude::*;
/// use async_trait::async_trait;
///
/// struct TelegramTrigger;
///
/// #[async_trait]
/// impl WebhookAction for TelegramTrigger {
///     type Event = String;
///
///     async fn on_subscribe(&self, ctx: &TriggerCtx) -> Result<()> {
///         // Register webhook URL with Telegram
///         Ok(())
///     }
///
///     async fn on_webhook(
///         &self,
///         ctx: &TriggerCtx,
///         payload: WebhookPayload,
///     ) -> Result<Option<Self::Event>> {
///         // Verify signature, parse, and return event
///         Ok(Some("message".to_string()))
///     }
///
///     async fn on_unsubscribe(&self, ctx: &TriggerCtx) -> Result<()> {
///         // Delete webhook from Telegram
///         Ok(())
///     }
///
///     async fn test(&self, ctx: &TriggerCtx) -> Result<TestResult> {
///         Ok(TestResult::success("Connected to Telegram"))
///     }
/// }
/// ```
#[async_trait]
pub trait WebhookAction: Send + Sync {
    /// Event type emitted by this trigger
    type Event: Send + Sync + 'static;

    /// Called when the trigger is activated
    ///
    /// Use this to register the webhook URL with the external provider.
    /// The webhook URL is available via `ctx.webhook_url()`.
    ///
    /// # Errors
    ///
    /// Return an error if registration with the provider fails.
    async fn on_subscribe(&self, ctx: &TriggerCtx) -> Result<()>;

    /// Called when a webhook request is received
    ///
    /// Verify the request (signature, origin, etc.) and parse it into
    /// an event. Return `Ok(None)` to filter out the request without
    /// error (e.g., signature verification failed).
    ///
    /// # Errors
    ///
    /// Return an error if parsing fails or the request is invalid.
    async fn on_webhook(
        &self,
        ctx: &TriggerCtx,
        payload: WebhookPayload,
    ) -> Result<Option<Self::Event>>;

    /// Called when the trigger is deactivated
    ///
    /// Use this to unregister the webhook from the external provider.
    ///
    /// # Errors
    ///
    /// Return an error if cleanup fails. Non-critical errors should
    /// be logged but not propagated.
    async fn on_unsubscribe(&self, ctx: &TriggerCtx) -> Result<()>;

    /// Test the webhook configuration
    ///
    /// This is called in the Test environment before activation.
    /// Verify credentials, connectivity, and configuration without
    /// causing side effects.
    ///
    /// # Errors
    ///
    /// Return an error if the test cannot be completed.
    async fn test(&self, ctx: &TriggerCtx) -> Result<TestResult>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_result_success() {
        let result = TestResult::success("All checks passed");

        assert!(result.success);
        assert_eq!(result.message, "All checks passed");
        assert!(result.sample_event.is_none());
        assert!(result.latency.is_none());
    }

    #[test]
    fn test_result_failure() {
        let result = TestResult::failure("Connection failed");

        assert!(!result.success);
        assert_eq!(result.message, "Connection failed");
    }

    #[test]
    fn test_result_with_sample() {
        let sample = serde_json::json!({"key": "value"});
        let result = TestResult::success("OK").with_sample(sample.clone());

        assert_eq!(result.sample_event, Some(sample));
    }

    #[test]
    fn test_result_with_latency() {
        let latency = Duration::from_millis(150);
        let result = TestResult::success("OK").with_latency(latency);

        assert_eq!(result.latency, Some(latency));
    }

    #[test]
    fn test_result_chaining() {
        let sample = serde_json::json!({"event": "test"});
        let latency = Duration::from_millis(100);

        let result = TestResult::success("Connected")
            .with_sample(sample.clone())
            .with_latency(latency);

        assert!(result.success);
        assert_eq!(result.message, "Connected");
        assert_eq!(result.sample_event, Some(sample));
        assert_eq!(result.latency, Some(latency));
    }
}
