//! Dynamic handler trait and typed action adapters.
//!
//! The runtime stores all actions as `Arc<dyn InternalHandler>` — a JSON-erased
//! interface. Typed action authors write `impl StatelessAction<Input=T, Output=U>`
//! and register via [`StatelessActionAdapter`] (or the registry's helper methods),
//! which handles (de)serialization automatically.
//!
//! ## Handler traits
//!
//! Five handler traits model the JSON-level contract for each action kind:
//!
//! - [`StatelessHandler`] — one-shot JSON in, JSON out
//! - [`StatefulHandler`] — iterative with mutable JSON state
//! - [`TriggerHandler`] — start/stop lifecycle (uses [`TriggerContext`])
//! - [`ResourceHandler`] — configure/cleanup lifecycle
//! - [`AgentHandler`] — autonomous agent (stub for Phase 9)
//!
//! [`ActionHandler`] is the top-level enum the engine dispatches on.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::context::{ActionContext, TriggerContext};
use crate::error::ActionError;
use crate::execution::StatelessAction;
use crate::metadata::ActionMetadata;
use crate::result::ActionResult;

/// Handler trait for action execution; runtime looks up by key and calls
/// `execute` with JSON input and [`ActionContext`].
///
/// This is the *internal* contract between registry and runtime. Action authors
/// implement typed traits ([`StatelessAction`] etc.) and use adapters to
/// convert to `dyn InternalHandler`.
#[async_trait]
pub trait InternalHandler: Send + Sync {
    /// Get action metadata.
    fn metadata(&self) -> &ActionMetadata;
    /// Execute the action with the given input and execution context.
    async fn execute(
        &self,
        input: serde_json::Value,
        context: &ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError>;
}

/// Wraps a [`StatelessAction`] as a [`dyn InternalHandler`].
///
/// Handles JSON deserialization of input and serialization of output so the
/// runtime can work with untyped JSON throughout, while action authors write
/// strongly-typed Rust.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::{StatelessActionAdapter, StatelessAction, Action, ActionResult, ActionError};
/// use nebula_action::handler::InternalHandler;
///
/// struct EchoAction { meta: ActionMetadata }
/// impl Action for EchoAction { ... }
/// impl StatelessAction for EchoAction {
///     type Input = serde_json::Value;
///     type Output = serde_json::Value;
///     async fn execute(&self, input: Self::Input, _ctx: &impl Context)
///         -> Result<ActionResult<Self::Output>, ActionError>
///     {
///         Ok(ActionResult::success(input))
///     }
/// }
///
/// let handler: Arc<dyn InternalHandler> = Arc::new(StatelessActionAdapter::new(EchoAction { ... }));
/// ```
pub struct StatelessActionAdapter<A> {
    action: A,
}

impl<A> StatelessActionAdapter<A> {
    /// Wrap a typed stateless action.
    pub fn new(action: A) -> Self {
        Self { action }
    }

    /// Consume the adapter, returning the inner action.
    pub fn into_inner(self) -> A {
        self.action
    }
}

#[async_trait]
impl<A> InternalHandler for StatelessActionAdapter<A>
where
    A: StatelessAction + Send + Sync + 'static,
    A::Input: serde::de::DeserializeOwned + Send + Sync,
    A::Output: serde::Serialize + Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        context: &ActionContext,
    ) -> Result<ActionResult<serde_json::Value>, ActionError> {
        let typed_input: A::Input = serde_json::from_value(input)
            .map_err(|e| ActionError::validation(format!("input deserialization failed: {e}")))?;

        let result = self.action.execute(typed_input, context).await?;

        result.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
        })
    }
}

// ── Handler traits ─────────────────────────────────────────────────────────

/// Stateless action handler — JSON in, JSON out.
///
/// This is the JSON-level contract for one-shot actions. The engine sends
/// a `serde_json::Value` input and receives a `serde_json::Value` output
/// wrapped in [`ActionResult`].
///
/// # Errors
///
/// Returns [`ActionError`] on validation, retryable, or fatal failures.
#[async_trait]
pub trait StatelessHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Execute with JSON input and context.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if execution fails (validation, retryable, or fatal).
    async fn execute(
        &self,
        input: Value,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}

/// Stateful action handler — JSON in, mutable JSON state, JSON out.
///
/// The engine calls `execute` repeatedly. State is persisted as JSON between
/// iterations for checkpointing. Return [`ActionResult::Continue`] for another
/// iteration or [`ActionResult::Break`] when done.
///
/// # Errors
///
/// Returns [`ActionError`] on validation, retryable, or fatal failures.
#[async_trait]
pub trait StatefulHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Create initial state as JSON for the first iteration.
    fn init_state(&self) -> Value;

    /// Execute one iteration with mutable JSON state.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if execution fails (validation, retryable, or fatal).
    async fn execute(
        &self,
        input: Value,
        state: &mut Value,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}

/// Trigger handler — start/stop lifecycle for workflow triggers.
///
/// Uses [`TriggerContext`] (workflow_id, trigger_id, cancellation) instead
/// of [`ActionContext`]. Triggers live outside the execution graph and emit
/// new workflow executions.
///
/// # Errors
///
/// Returns [`ActionError`] if start or stop fails.
#[async_trait]
pub trait TriggerHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Start the trigger (register listener, schedule poll, etc.).
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the trigger cannot be started.
    async fn start(&self, ctx: &TriggerContext) -> Result<(), ActionError>;

    /// Stop the trigger (unregister, cancel schedule).
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the trigger cannot be stopped cleanly.
    async fn stop(&self, ctx: &TriggerContext) -> Result<(), ActionError>;
}

/// Resource handler — configure/cleanup lifecycle for graph-scoped resources.
///
/// The engine runs `configure` before downstream nodes; the resulting instance
/// is scoped to the branch. When the scope ends, `cleanup` is called.
///
/// # Errors
///
/// Returns [`ActionError`] on configuration or cleanup failure.
#[async_trait]
pub trait ResourceHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Build the resource for this scope.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if the resource cannot be configured.
    async fn configure(
        &self,
        config: Value,
        ctx: &ActionContext,
    ) -> Result<Box<dyn std::any::Any + Send + Sync>, ActionError>;

    /// Clean up the resource instance when the scope ends.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if cleanup fails.
    async fn cleanup(
        &self,
        instance: Box<dyn std::any::Any + Send + Sync>,
        ctx: &ActionContext,
    ) -> Result<(), ActionError>;
}

/// Agent handler — autonomous agent execution (stub for Phase 9).
///
/// Agents combine tool use, planning, and iterative execution. This trait
/// is a placeholder with the same signature as [`StatelessHandler`]; the
/// full agent protocol will be defined in Phase 9.
///
/// # Errors
///
/// Returns [`ActionError`] on execution failure.
#[async_trait]
pub trait AgentHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Execute the agent with JSON input and context.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if agent execution fails.
    async fn execute(
        &self,
        input: Value,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}

// ── ActionHandler enum ─────────────────────────────────────────────────────

/// Top-level handler enum — the engine dispatches based on variant.
///
/// Each variant wraps an `Arc<dyn XxxHandler>` so handlers can be shared
/// across nodes in the workflow graph.
pub enum ActionHandler {
    /// One-shot stateless execution.
    Stateless(Arc<dyn StatelessHandler>),
    /// Iterative execution with persistent JSON state.
    Stateful(Arc<dyn StatefulHandler>),
    /// Workflow trigger (start/stop lifecycle).
    Trigger(Arc<dyn TriggerHandler>),
    /// Graph-scoped resource (configure/cleanup).
    Resource(Arc<dyn ResourceHandler>),
    /// Autonomous agent (stub for Phase 9).
    Agent(Arc<dyn AgentHandler>),
}

impl ActionHandler {
    /// Get metadata regardless of variant.
    #[must_use]
    pub fn metadata(&self) -> &ActionMetadata {
        match self {
            Self::Stateless(h) => h.metadata(),
            Self::Stateful(h) => h.metadata(),
            Self::Trigger(h) => h.metadata(),
            Self::Resource(h) => h.metadata(),
            Self::Agent(h) => h.metadata(),
        }
    }

    /// Check if this is a stateless handler.
    #[must_use]
    pub fn is_stateless(&self) -> bool {
        matches!(self, Self::Stateless(_))
    }

    /// Check if this is a stateful handler.
    #[must_use]
    pub fn is_stateful(&self) -> bool {
        matches!(self, Self::Stateful(_))
    }

    /// Check if this is a trigger handler.
    #[must_use]
    pub fn is_trigger(&self) -> bool {
        matches!(self, Self::Trigger(_))
    }

    /// Check if this is a resource handler.
    #[must_use]
    pub fn is_resource(&self) -> bool {
        matches!(self, Self::Resource(_))
    }

    /// Check if this is an agent handler.
    #[must_use]
    pub fn is_agent(&self) -> bool {
        matches!(self, Self::Agent(_))
    }
}

impl fmt::Debug for ActionHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stateless(h) => f.debug_tuple("Stateless").field(&h.metadata().key).finish(),
            Self::Stateful(h) => f.debug_tuple("Stateful").field(&h.metadata().key).finish(),
            Self::Trigger(h) => f.debug_tuple("Trigger").field(&h.metadata().key).finish(),
            Self::Resource(h) => f.debug_tuple("Resource").field(&h.metadata().key).finish(),
            Self::Agent(h) => f.debug_tuple("Agent").field(&h.metadata().key).finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde::{Deserialize, Serialize};
    use tokio_util::sync::CancellationToken;

    use crate::action::Action;
    use crate::context::Context;
    use crate::dependency::ActionDependencies;
    use crate::execution::StatelessAction;
    use crate::metadata::ActionMetadata;
    use nebula_core::id::{ExecutionId, NodeId, WorkflowId};

    use super::*;

    // ── Test action ────────────────────────────────────────────────────────────

    #[derive(Debug, Deserialize)]
    struct AddInput {
        a: i64,
        b: i64,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct AddOutput {
        sum: i64,
    }

    struct AddAction {
        meta: ActionMetadata,
    }

    impl AddAction {
        fn new() -> Self {
            Self {
                meta: ActionMetadata::new(
                    nebula_core::action_key!("math.add"),
                    "Add",
                    "Adds two numbers",
                ),
            }
        }
    }

    impl ActionDependencies for AddAction {}

    impl Action for AddAction {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }
    }

    impl StatelessAction for AddAction {
        type Input = AddInput;
        type Output = AddOutput;

        async fn execute(
            &self,
            input: Self::Input,
            _ctx: &impl Context,
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::success(AddOutput {
                sum: input.a + input.b,
            }))
        }
    }

    fn make_ctx() -> ActionContext {
        ActionContext::new(
            ExecutionId::nil(),
            NodeId::nil(),
            WorkflowId::nil(),
            CancellationToken::new(),
        )
    }

    // ── Tests ──────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn adapter_executes_typed_action() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let ctx = make_ctx();

        let input = serde_json::json!({ "a": 3, "b": 7 });
        let result = adapter.execute(input, &ctx).await.unwrap();

        match result {
            ActionResult::Success { output } => {
                let v = output.into_value().unwrap();
                let out: AddOutput = serde_json::from_value(v).unwrap();
                assert_eq!(out.sum, 10);
            }
            _ => panic!("expected Success"),
        }
    }

    #[tokio::test]
    async fn adapter_returns_validation_error_on_bad_input() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let ctx = make_ctx();

        let bad_input = serde_json::json!({ "x": "not a number" });
        let err = adapter.execute(bad_input, &ctx).await.unwrap_err();
        assert!(matches!(err, ActionError::Validation(_)));
    }

    #[tokio::test]
    async fn adapter_exposes_metadata() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        assert_eq!(adapter.metadata().key, nebula_core::action_key!("math.add"));
    }

    #[test]
    fn adapter_is_dyn_compatible() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let _: Arc<dyn InternalHandler> = Arc::new(adapter);
    }

    // ── Handler trait test helpers ─────────────────────────────────────────────

    fn test_meta(key: &str) -> ActionMetadata {
        ActionMetadata::new(
            nebula_core::ActionKey::new(key).expect("valid test key"),
            key,
            "test handler",
        )
    }

    struct TestStatelessHandler {
        meta: ActionMetadata,
    }

    #[async_trait]
    impl StatelessHandler for TestStatelessHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        async fn execute(
            &self,
            input: Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<Value>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    struct TestStatefulHandler {
        meta: ActionMetadata,
    }

    #[async_trait]
    impl StatefulHandler for TestStatefulHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        fn init_state(&self) -> Value {
            serde_json::json!(0)
        }

        async fn execute(
            &self,
            input: Value,
            state: &mut Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<Value>, ActionError> {
            let count = state.as_u64().unwrap_or(0);
            *state = serde_json::json!(count + 1);
            Ok(ActionResult::success(input))
        }
    }

    struct TestTriggerHandler {
        meta: ActionMetadata,
    }

    #[async_trait]
    impl TriggerHandler for TestTriggerHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        async fn start(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
            Ok(())
        }

        async fn stop(&self, _ctx: &TriggerContext) -> Result<(), ActionError> {
            Ok(())
        }
    }

    struct TestResourceHandler {
        meta: ActionMetadata,
    }

    #[async_trait]
    impl ResourceHandler for TestResourceHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        async fn configure(
            &self,
            _config: Value,
            _ctx: &ActionContext,
        ) -> Result<Box<dyn std::any::Any + Send + Sync>, ActionError> {
            Ok(Box::new(42u32))
        }

        async fn cleanup(
            &self,
            _instance: Box<dyn std::any::Any + Send + Sync>,
            _ctx: &ActionContext,
        ) -> Result<(), ActionError> {
            Ok(())
        }
    }

    struct TestAgentHandler {
        meta: ActionMetadata,
    }

    #[async_trait]
    impl AgentHandler for TestAgentHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        async fn execute(
            &self,
            input: Value,
            _ctx: &ActionContext,
        ) -> Result<ActionResult<Value>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    // ── Handler trait dyn-compatibility tests ──────────────────────────────────

    #[test]
    fn stateless_handler_is_dyn_compatible() {
        let h = TestStatelessHandler {
            meta: test_meta("test.stateless"),
        };
        let _: Arc<dyn StatelessHandler> = Arc::new(h);
    }

    #[test]
    fn stateful_handler_is_dyn_compatible() {
        let h = TestStatefulHandler {
            meta: test_meta("test.stateful"),
        };
        let _: Arc<dyn StatefulHandler> = Arc::new(h);
    }

    #[test]
    fn trigger_handler_is_dyn_compatible() {
        let h = TestTriggerHandler {
            meta: test_meta("test.trigger"),
        };
        let _: Arc<dyn TriggerHandler> = Arc::new(h);
    }

    #[test]
    fn resource_handler_is_dyn_compatible() {
        let h = TestResourceHandler {
            meta: test_meta("test.resource"),
        };
        let _: Arc<dyn ResourceHandler> = Arc::new(h);
    }

    #[test]
    fn agent_handler_is_dyn_compatible() {
        let h = TestAgentHandler {
            meta: test_meta("test.agent"),
        };
        let _: Arc<dyn AgentHandler> = Arc::new(h);
    }

    // ── ActionHandler metadata delegation ──────────────────────────────────────

    #[test]
    fn action_handler_metadata_delegates_to_inner() {
        let cases: Vec<(&str, ActionHandler)> = vec![
            (
                "test.stateless",
                ActionHandler::Stateless(Arc::new(TestStatelessHandler {
                    meta: test_meta("test.stateless"),
                })),
            ),
            (
                "test.stateful",
                ActionHandler::Stateful(Arc::new(TestStatefulHandler {
                    meta: test_meta("test.stateful"),
                })),
            ),
            (
                "test.trigger",
                ActionHandler::Trigger(Arc::new(TestTriggerHandler {
                    meta: test_meta("test.trigger"),
                })),
            ),
            (
                "test.resource",
                ActionHandler::Resource(Arc::new(TestResourceHandler {
                    meta: test_meta("test.resource"),
                })),
            ),
            (
                "test.agent",
                ActionHandler::Agent(Arc::new(TestAgentHandler {
                    meta: test_meta("test.agent"),
                })),
            ),
        ];

        for (expected_key, handler) in &cases {
            assert_eq!(
                handler.metadata().key,
                nebula_core::ActionKey::new(expected_key).expect("valid test key")
            );
        }
    }

    // ── ActionHandler variant checks ───────────────────────────────────────────

    #[test]
    fn action_handler_variant_checks() {
        let stateless = ActionHandler::Stateless(Arc::new(TestStatelessHandler {
            meta: test_meta("test.stateless"),
        }));
        assert!(stateless.is_stateless());
        assert!(!stateless.is_stateful());
        assert!(!stateless.is_trigger());
        assert!(!stateless.is_resource());
        assert!(!stateless.is_agent());

        let stateful = ActionHandler::Stateful(Arc::new(TestStatefulHandler {
            meta: test_meta("test.stateful"),
        }));
        assert!(!stateful.is_stateless());
        assert!(stateful.is_stateful());

        let trigger = ActionHandler::Trigger(Arc::new(TestTriggerHandler {
            meta: test_meta("test.trigger"),
        }));
        assert!(!trigger.is_stateless());
        assert!(trigger.is_trigger());

        let resource = ActionHandler::Resource(Arc::new(TestResourceHandler {
            meta: test_meta("test.resource"),
        }));
        assert!(!resource.is_stateless());
        assert!(resource.is_resource());

        let agent = ActionHandler::Agent(Arc::new(TestAgentHandler {
            meta: test_meta("test.agent"),
        }));
        assert!(!agent.is_stateless());
        assert!(agent.is_agent());
    }

    // ── ActionHandler Debug ────────────────────────────────────────────────────

    #[test]
    fn action_handler_debug_shows_variant_and_key() {
        let handler = ActionHandler::Stateless(Arc::new(TestStatelessHandler {
            meta: test_meta("test.stateless"),
        }));
        let debug = format!("{handler:?}");
        assert!(debug.contains("Stateless"));
        assert!(debug.contains("test.stateless"));
    }
}
