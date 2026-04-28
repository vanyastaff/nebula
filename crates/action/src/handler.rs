//! Top-level [`ActionHandler`] enum dispatcher.
//!
//! The engine dispatches actions via [`ActionHandler`], a top-level enum whose
//! variants wrap `Arc<dyn XxxHandler>` trait objects. Typed action authors write
//! `impl StatelessAction<Input=T, Output=U>` and register via the registry's
//! helper methods (e.g., `nebula_runtime::ActionRegistry::register_stateless`),
//! which wraps the typed action in the corresponding adapter automatically.
//!
//! ## Handler traits
//!
//! Five handler traits model the JSON-level contract for each action kind.
//! Each trait lives in its domain file:
//!
//! - [`StatelessHandler`](crate::stateless::StatelessHandler) — one-shot JSON in, JSON out
//! - [`StatefulHandler`](crate::stateful::StatefulHandler) — iterative with mutable JSON state
//! - [`TriggerHandler`](crate::trigger::TriggerHandler) — start/stop lifecycle with
//!   [`TriggerEvent`](crate::trigger::TriggerEvent) and
//!   [`TriggerEventOutcome`](crate::trigger::TriggerEventOutcome); webhook and poll specializations
//!   live in [`crate::webhook`] and [`crate::poll`]
//! - [`ResourceHandler`](crate::resource::ResourceHandler) — configure/cleanup lifecycle
//!
//! [`ActionHandler`] itself is the sum type the engine switches on. All handler
//! types are also re-exported at the crate root, so the canonical import is
//! `use nebula_action::{StatelessHandler, TriggerEvent, ...}`.

use std::{fmt, sync::Arc};

use crate::{
    metadata::ActionMetadata, resource::ResourceHandler, stateful::StatefulHandler,
    stateless::StatelessHandler, trigger::TriggerHandler,
};

// ── ActionHandler enum ─────────────────────────────────────────────────────

/// Top-level handler enum — the engine dispatches based on variant.
///
/// Each variant wraps an `Arc<dyn XxxHandler>` so handlers can be shared
/// across nodes in the workflow graph.
#[derive(Clone)]
#[non_exhaustive]
pub enum ActionHandler {
    /// One-shot stateless execution.
    Stateless(Arc<dyn StatelessHandler>),
    /// Iterative execution with persistent JSON state.
    Stateful(Arc<dyn StatefulHandler>),
    /// Workflow trigger (start/stop lifecycle).
    Trigger(Arc<dyn TriggerHandler>),
    /// Graph-scoped resource (configure/cleanup).
    Resource(Arc<dyn ResourceHandler>),
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
}

impl fmt::Debug for ActionHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stateless(h) => f
                .debug_tuple("Stateless")
                .field(&h.metadata().base.key)
                .finish(),
            Self::Stateful(h) => f
                .debug_tuple("Stateful")
                .field(&h.metadata().base.key)
                .finish(),
            Self::Trigger(h) => f
                .debug_tuple("Trigger")
                .field(&h.metadata().base.key)
                .finish(),
            Self::Resource(h) => f
                .debug_tuple("Resource")
                .field(&h.metadata().base.key)
                .finish(),
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::Value;

    use super::*;
    use crate::{
        ActionError, ActionResult,
        context::{ActionContext, TriggerContext},
    };

    // Shared test stubs — these exist solely to construct `ActionHandler`
    // variants in cross-variant tests (metadata delegation, is_* predicates,
    // Debug output). Adapter-internal tests live in their own domain files
    // (stateless.rs, stateful.rs, trigger.rs, webhook.rs, poll.rs, resource.rs).

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

    #[async_trait::async_trait]
    impl StatelessHandler for TestStatelessHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        async fn execute(
            &self,
            input: Value,
            _ctx: &dyn ActionContext,
        ) -> Result<ActionResult<Value>, ActionError> {
            Ok(ActionResult::success(input))
        }
    }

    struct TestStatefulHandler {
        meta: ActionMetadata,
    }

    #[async_trait::async_trait]
    impl StatefulHandler for TestStatefulHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        fn init_state(&self) -> Result<Value, ActionError> {
            Ok(serde_json::json!(0))
        }

        async fn execute(
            &self,
            input: &Value,
            state: &mut Value,
            _ctx: &dyn ActionContext,
        ) -> Result<ActionResult<Value>, ActionError> {
            let count = state.as_u64().unwrap_or(0);
            *state = serde_json::json!(count + 1);
            Ok(ActionResult::success(input.clone()))
        }
    }

    struct TestTriggerHandler {
        meta: ActionMetadata,
    }

    #[async_trait::async_trait]
    impl TriggerHandler for TestTriggerHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        async fn start(&self, _ctx: &dyn TriggerContext) -> Result<(), ActionError> {
            Ok(())
        }

        async fn stop(&self, _ctx: &dyn TriggerContext) -> Result<(), ActionError> {
            Ok(())
        }
    }

    struct TestResourceHandler {
        meta: ActionMetadata,
    }

    #[async_trait::async_trait]
    impl ResourceHandler for TestResourceHandler {
        fn metadata(&self) -> &ActionMetadata {
            &self.meta
        }

        async fn configure(
            &self,
            _config: Value,
            _ctx: &dyn ActionContext,
        ) -> Result<Box<dyn std::any::Any + Send + Sync>, ActionError> {
            Ok(Box::new(42u32) as Box<dyn std::any::Any + Send + Sync>)
        }

        async fn cleanup(
            &self,
            _instance: Box<dyn std::any::Any + Send + Sync>,
            _ctx: &dyn ActionContext,
        ) -> Result<(), ActionError> {
            Ok(())
        }
    }

    // ── Dyn-compatibility smoke tests ──────────────────────────────────────

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

    // ── ActionHandler metadata delegation ──────────────────────────────────

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
        ];

        for (expected_key, handler) in &cases {
            assert_eq!(
                handler.metadata().base.key,
                nebula_core::ActionKey::new(expected_key).expect("valid test key")
            );
        }
    }

    // ── ActionHandler variant checks ───────────────────────────────────────

    #[test]
    fn action_handler_variant_checks() {
        let stateless = ActionHandler::Stateless(Arc::new(TestStatelessHandler {
            meta: test_meta("test.stateless"),
        }));
        assert!(stateless.is_stateless());
        assert!(!stateless.is_stateful());
        assert!(!stateless.is_trigger());
        assert!(!stateless.is_resource());

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
    }

    // ── ActionHandler Debug ────────────────────────────────────────────────

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
