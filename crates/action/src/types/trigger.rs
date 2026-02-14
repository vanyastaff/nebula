use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::action::Action;
use crate::context::ActionContext;
use crate::error::ActionError;

/// Kind of trigger — determines how the engine invokes this trigger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TriggerKind {
    /// Engine polls at a fixed interval.
    Poll {
        /// How often to call `poll()`.
        interval: Duration,
    },
    /// Engine registers an HTTP endpoint and forwards requests.
    Webhook {
        /// URL path suffix (e.g. `"/github-events"`).
        path: String,
    },
    /// Engine uses a cron expression to schedule invocations.
    Cron {
        /// Standard cron expression (e.g. `"0 */5 * * *"`).
        expression: String,
    },
}

/// An event emitted by a trigger.
#[derive(Debug, Clone)]
pub struct TriggerEvent<T> {
    /// The event payload.
    pub data: T,
    /// When this event occurred.
    pub timestamp: DateTime<Utc>,
    /// Optional deduplication key — if two events share the same key,
    /// the engine may drop the duplicate.
    pub dedup_key: Option<String>,
}

impl<T> TriggerEvent<T> {
    /// Create a new event with the current timestamp.
    pub fn new(data: T) -> Self {
        Self {
            data,
            timestamp: Utc::now(),
            dedup_key: None,
        }
    }

    /// Create an event with a deduplication key.
    pub fn with_dedup(data: T, key: impl Into<String>) -> Self {
        Self {
            data,
            timestamp: Utc::now(),
            dedup_key: Some(key.into()),
        }
    }
}

/// Incoming webhook request forwarded by the engine to a trigger.
#[derive(Debug, Clone)]
pub struct WebhookRequest {
    /// HTTP method (e.g. `"POST"`).
    pub method: String,
    /// Request path.
    pub path: String,
    /// HTTP headers.
    pub headers: HashMap<String, String>,
    /// Parsed request body.
    pub body: serde_json::Value,
}

/// Event source that starts or feeds workflow executions.
///
/// Triggers are the entry points for workflows. They produce events
/// that the engine converts into workflow executions.
///
/// Each trigger declares its [`TriggerKind`], which tells the engine
/// whether to poll, listen for webhooks, or schedule via cron.
///
/// # Type Parameters
///
/// - `Config`: trigger-specific configuration (e.g. URL, credentials, filters).
/// - `Event`: the event payload type produced by this trigger.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::*;
/// use nebula_action::trigger::*;
/// use async_trait::async_trait;
///
/// struct IntervalTimer { meta: ActionMetadata }
///
/// #[async_trait]
/// impl TriggerAction for IntervalTimer {
///     type Config = serde_json::Value;
///     type Event = serde_json::Value;
///
///     fn kind(&self, _config: &Self::Config) -> TriggerKind {
///         TriggerKind::Poll {
///             interval: std::time::Duration::from_secs(60),
///         }
///     }
///
///     async fn poll(
///         &self, _config: &Self::Config, _last_state: Option<serde_json::Value>,
///         ctx: &ActionContext,
///     ) -> Result<Vec<TriggerEvent<Self::Event>>, ActionError> {
///         ctx.check_cancelled()?;
///         Ok(vec![TriggerEvent::new(serde_json::json!({
///             "tick": chrono::Utc::now().to_rfc3339(),
///         }))])
///     }
/// }
/// ```
#[async_trait]
pub trait TriggerAction: Action {
    /// Trigger-specific configuration type.
    type Config: Send + Sync + 'static;
    /// Event payload type produced by this trigger.
    type Event: Send + Sync + 'static;

    /// Declare the kind of trigger — determines how the engine drives it.
    fn kind(&self, config: &Self::Config) -> TriggerKind;

    /// Poll for new events (used when `kind()` returns `TriggerKind::Poll`).
    ///
    /// `last_state` contains opaque state from the previous poll (e.g. a cursor
    /// or timestamp) to avoid re-processing old events. Return an empty vec if
    /// no new events are available.
    async fn poll(
        &self,
        _config: &Self::Config,
        _last_state: Option<serde_json::Value>,
        _ctx: &ActionContext,
    ) -> Result<Vec<TriggerEvent<Self::Event>>, ActionError> {
        Ok(vec![])
    }

    /// Handle an incoming webhook request (used when `kind()` returns `TriggerKind::Webhook`).
    ///
    /// The engine forwards inbound HTTP requests matching this trigger's path.
    /// Default implementation returns a fatal error — override for webhook triggers.
    async fn handle_webhook(
        &self,
        _config: &Self::Config,
        _request: WebhookRequest,
        _ctx: &ActionContext,
    ) -> Result<TriggerEvent<Self::Event>, ActionError> {
        Err(ActionError::fatal("webhook not supported by this trigger"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_event_new() {
        let event = TriggerEvent::new(42);
        assert_eq!(event.data, 42);
        assert!(event.dedup_key.is_none());
    }

    #[test]
    fn trigger_event_with_dedup() {
        let event = TriggerEvent::with_dedup("payload", "unique-123");
        assert_eq!(event.data, "payload");
        assert_eq!(event.dedup_key.as_deref(), Some("unique-123"));
    }

    #[test]
    fn trigger_kind_poll() {
        let kind = TriggerKind::Poll {
            interval: Duration::from_secs(30),
        };
        match &kind {
            TriggerKind::Poll { interval } => {
                assert_eq!(*interval, Duration::from_secs(30));
            }
            _ => panic!("expected Poll"),
        }
    }

    #[test]
    fn trigger_kind_webhook() {
        let kind = TriggerKind::Webhook {
            path: "/github-events".into(),
        };
        match &kind {
            TriggerKind::Webhook { path } => {
                assert_eq!(path, "/github-events");
            }
            _ => panic!("expected Webhook"),
        }
    }

    #[test]
    fn trigger_kind_cron() {
        let kind = TriggerKind::Cron {
            expression: "0 */5 * * *".into(),
        };
        match &kind {
            TriggerKind::Cron { expression } => {
                assert_eq!(expression, "0 */5 * * *");
            }
            _ => panic!("expected Cron"),
        }
    }

    #[test]
    fn webhook_request_construction() {
        let req = WebhookRequest {
            method: "POST".into(),
            path: "/hooks/abc".into(),
            headers: HashMap::from([("content-type".into(), "application/json".into())]),
            body: serde_json::json!({"event": "push"}),
        };
        assert_eq!(req.method, "POST");
        assert_eq!(req.headers.len(), 1);
    }
}
