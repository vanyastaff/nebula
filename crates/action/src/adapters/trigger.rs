//! Adapter for [`TriggerAction`] to [`InternalHandler`].

use std::sync::Arc;

use async_trait::async_trait;
use nebula_parameter::collection::ParameterCollection;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::action::Action;
use crate::context::ActionContext;
use crate::error::ActionError;
use crate::handler::InternalHandler;
use crate::metadata::ActionMetadata;
use crate::result::ActionResult;
use crate::types::trigger::{TriggerAction, WebhookRequest};

/// Internal tagged enum for dispatching trigger operations via JSON input.
#[derive(serde::Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum TriggerOperation {
    Poll {
        config: serde_json::Value,
        last_state: Option<serde_json::Value>,
    },
    Webhook {
        config: serde_json::Value,
        request: WebhookRequest,
    },
    Kind {
        config: serde_json::Value,
    },
}

/// Adapter that wraps a typed [`TriggerAction`] as an [`InternalHandler`].
///
/// Triggers have 3 operations (poll, webhook, kind) dispatched via tagged JSON input:
/// - `{"op": "poll", "config": {...}, "last_state": ...}`
/// - `{"op": "webhook", "config": {...}, "request": {...}}`
/// - `{"op": "kind", "config": {...}}`
pub struct TriggerActionAdapter<A> {
    action: Arc<A>,
}

impl<A> TriggerActionAdapter<A> {
    /// Wrap a trigger action in an adapter.
    pub fn new(action: A) -> Self {
        Self {
            action: Arc::new(action),
        }
    }
}

#[async_trait]
impl<A> InternalHandler for TriggerActionAdapter<A>
where
    A: TriggerAction + Send + Sync + 'static,
    A::Config: DeserializeOwned + Send + Sync + 'static,
    A::Event: Serialize + Send + Sync + 'static,
{
    async fn execute(
        &self,
        input: serde_json::Value,
        ctx: ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        let operation: TriggerOperation = serde_json::from_value(input).map_err(|e| {
            ActionError::validation(format!(
                "invalid trigger operation (expected {{\"op\": \"poll|webhook|kind\", ...}}): {e}"
            ))
        })?;

        match operation {
            TriggerOperation::Poll { config, last_state } => {
                let typed_config: A::Config = serde_json::from_value(config).map_err(|e| {
                    ActionError::validation(format!("config deserialization failed: {e}"))
                })?;

                let events = self.action.poll(&typed_config, last_state, &ctx).await?;

                let json_events = serde_json::to_value(events)
                    .map_err(|e| ActionError::fatal(format!("events serialization failed: {e}")))?;

                Ok(ActionResult::success(json_events))
            }
            TriggerOperation::Webhook { config, request } => {
                let typed_config: A::Config = serde_json::from_value(config).map_err(|e| {
                    ActionError::validation(format!("config deserialization failed: {e}"))
                })?;

                let event = self
                    .action
                    .handle_webhook(&typed_config, request, &ctx)
                    .await?;

                let json_event = serde_json::to_value(event)
                    .map_err(|e| ActionError::fatal(format!("event serialization failed: {e}")))?;

                Ok(ActionResult::success(json_event))
            }
            TriggerOperation::Kind { config } => {
                let typed_config: A::Config = serde_json::from_value(config).map_err(|e| {
                    ActionError::validation(format!("config deserialization failed: {e}"))
                })?;

                let kind = self.action.kind(&typed_config);

                let json_kind = serde_json::to_value(kind).map_err(|e| {
                    ActionError::fatal(format!("trigger kind serialization failed: {e}"))
                })?;

                Ok(ActionResult::success(json_kind))
            }
        }
    }

    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    fn action_type(&self) -> crate::metadata::ActionType {
        self.action.action_type()
    }

    fn parameters(&self) -> Option<&ParameterCollection> {
        self.action.parameters()
    }
}

impl<A: Action> std::fmt::Debug for TriggerActionAdapter<A> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerActionAdapter")
            .field("action_key", &self.action.metadata().key)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::{ActionMetadata, ActionType};
    use crate::types::trigger::{TriggerEvent, TriggerKind};

    use nebula_core::id::{ExecutionId, NodeId, WorkflowId};
    use nebula_core::scope::ScopeLevel;
    use serde::{Deserialize, Serialize};

    fn test_ctx() -> ActionContext {
        ActionContext::new(
            ExecutionId::v4(),
            NodeId::v4(),
            WorkflowId::v4(),
            ScopeLevel::Global,
        )
    }

    // ── Test trigger action ──

    #[derive(Debug, Serialize, Deserialize)]
    struct TestConfig {
        url: String,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct TestEvent {
        data: String,
    }

    #[derive(Debug)]
    struct TestTrigger {
        meta: ActionMetadata,
    }

    impl TestTrigger {
        fn new() -> Self {
            Self {
                meta: ActionMetadata::new("test.trigger", "Test Trigger", "A test trigger"),
            }
        }
    }

    impl Action for TestTrigger {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
        fn action_type(&self) -> ActionType {
            ActionType::Trigger
        }
    }

    #[async_trait]
    impl TriggerAction for TestTrigger {
        type Config = TestConfig;
        type Event = TestEvent;

        fn kind(&self, config: &Self::Config) -> TriggerKind {
            TriggerKind::Webhook {
                path: config.url.clone(),
            }
        }

        async fn poll(
            &self,
            config: &Self::Config,
            last_state: Option<serde_json::Value>,
            _ctx: &ActionContext,
        ) -> Result<Vec<TriggerEvent<Self::Event>>, ActionError> {
            let mut events = vec![TriggerEvent::new(TestEvent {
                data: format!("polled:{}", config.url),
            })];
            if let Some(state) = last_state {
                events.push(TriggerEvent::new(TestEvent {
                    data: format!("with_state:{state}"),
                }));
            }
            Ok(events)
        }

        async fn handle_webhook(
            &self,
            _config: &Self::Config,
            request: WebhookRequest,
            _ctx: &ActionContext,
        ) -> Result<TriggerEvent<Self::Event>, ActionError> {
            Ok(TriggerEvent::new(TestEvent {
                data: format!("webhook:{}", request.method),
            }))
        }
    }

    // ── Tests ──

    #[tokio::test]
    async fn poll_operation() {
        let adapter = TriggerActionAdapter::new(TestTrigger::new());
        let result = adapter
            .execute(
                serde_json::json!({
                    "op": "poll",
                    "config": {"url": "https://example.com"},
                    "last_state": null
                }),
                test_ctx(),
            )
            .await
            .unwrap();

        match result {
            ActionResult::Success { output } => {
                let val = output.as_value().unwrap();
                let arr = val.as_array().unwrap();
                assert_eq!(arr.len(), 1);
                assert_eq!(arr[0]["data"]["data"], "polled:https://example.com");
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn poll_with_last_state() {
        let adapter = TriggerActionAdapter::new(TestTrigger::new());
        let result = adapter
            .execute(
                serde_json::json!({
                    "op": "poll",
                    "config": {"url": "https://example.com"},
                    "last_state": "cursor-123"
                }),
                test_ctx(),
            )
            .await
            .unwrap();

        match result {
            ActionResult::Success { output } => {
                let val = output.as_value().unwrap();
                let arr = val.as_array().unwrap();
                assert_eq!(arr.len(), 2);
                assert_eq!(arr[1]["data"]["data"], "with_state:\"cursor-123\"");
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn webhook_operation() {
        let adapter = TriggerActionAdapter::new(TestTrigger::new());
        let result = adapter
            .execute(
                serde_json::json!({
                    "op": "webhook",
                    "config": {"url": "https://example.com"},
                    "request": {
                        "method": "POST",
                        "path": "/hooks/abc",
                        "headers": {"content-type": "application/json"},
                        "body": {"event": "push"}
                    }
                }),
                test_ctx(),
            )
            .await
            .unwrap();

        match result {
            ActionResult::Success { output } => {
                let val = output.as_value().unwrap();
                assert_eq!(val["data"]["data"], "webhook:POST");
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn kind_operation() {
        let adapter = TriggerActionAdapter::new(TestTrigger::new());
        let result = adapter
            .execute(
                serde_json::json!({
                    "op": "kind",
                    "config": {"url": "/github-events"}
                }),
                test_ctx(),
            )
            .await
            .unwrap();

        match result {
            ActionResult::Success { output } => {
                let val = output.as_value().unwrap();
                assert_eq!(val["Webhook"]["path"], "/github-events");
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn invalid_operation() {
        let adapter = TriggerActionAdapter::new(TestTrigger::new());
        let err = adapter
            .execute(
                serde_json::json!({
                    "op": "unknown",
                    "config": {}
                }),
                test_ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Validation(_)));
    }

    #[tokio::test]
    async fn malformed_input() {
        let adapter = TriggerActionAdapter::new(TestTrigger::new());
        let err = adapter
            .execute(serde_json::json!("not an object"), test_ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Validation(_)));
    }

    #[tokio::test]
    async fn config_deserialization_failure() {
        let adapter = TriggerActionAdapter::new(TestTrigger::new());
        let err = adapter
            .execute(
                serde_json::json!({
                    "op": "poll",
                    "config": {"wrong_field": 42},
                    "last_state": null
                }),
                test_ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Validation(_)));
    }

    #[tokio::test]
    async fn metadata_delegation() {
        let adapter = TriggerActionAdapter::new(TestTrigger::new());
        assert_eq!(adapter.metadata().key, "test.trigger");
        assert_eq!(adapter.action_type(), ActionType::Trigger);
        assert!(adapter.parameters().is_none());
    }
}
