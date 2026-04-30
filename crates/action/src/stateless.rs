//! Core [`StatelessAction`] trait.
//!
//! Stateless actions are pure functions from input to result — no state is
//! kept between executions, and the engine may run multiple instances in
//! parallel. For iterative execution with persistent state, use
//! [`StatefulAction`](crate::stateful::StatefulAction).
//!
//! ## Cancellation
//!
//! Cancellation is handled by the runtime (e.g. `tokio::select!` between
//! `execute` and `ctx.cancellation().cancelled()`). Implementations do not
//! need to check cancellation unless they want cooperative checks at specific
//! points.

use std::{fmt, future::Future};

use serde_json::Value;

use crate::{
    action::Action,
    context::ActionContext,
    error::{ActionError, ValidationReason},
    metadata::ActionMetadata,
    result::ActionResult,
};

/// Stateless action: pure function from input to result.
///
/// No state is kept between executions. The engine may run multiple
/// instances in parallel. Use [`StatefulAction`](crate::stateful::StatefulAction)
/// for iterative or stateful behavior.
///
/// `Self::Input` and `Self::Output` are inherited from
/// [`Action`](crate::Action); concrete implementations declare them on the
/// base trait.
///
/// # Cancellation
///
/// Cancellation is handled by the runtime (e.g. `tokio::select!` between
/// `execute` and `ctx.cancellation().cancelled()`). Implementations do not
/// need to check cancellation unless they want cooperative checks at specific
/// points.
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement StatelessAction",
    note = "implement the `execute` method (Self::Input/Output declared on the base Action trait)"
)]
pub trait StatelessAction: Action {
    /// Execute the action with the given input and context.
    ///
    /// Returns [`ActionResult`] for flow control (Success, Skip, Branch, Wait, etc.)
    /// or [`ActionError`] for retryable/fatal failures.
    ///
    /// The returned future must be `Send` so the runtime can run it in
    /// `tokio::select!` with cancellation (no per-action cancellation boilerplate).
    fn execute(
        &self,
        input: <Self as Action>::Input,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ActionResult<<Self as Action>::Output>, ActionError>> + Send;
}

// ── StatelessHandler trait ──────────────────────────────────────────────────

/// Stateless handler — JSON-erased one-shot execution contract.
///
/// The engine dispatches every `StatelessAction` through this `dyn` trait
/// (wrapped by [`StatelessActionAdapter`]). For typed authoring, write
/// `impl StatelessAction` and let the adapter bridge to JSON.
///
/// # Errors
///
/// Returns [`ActionError`] on validation, retryable, or fatal failure.
#[async_trait::async_trait]
pub trait StatelessHandler: Send + Sync + 'static {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Execute one-shot with JSON input.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if execution fails (validation, retryable, or fatal).
    async fn execute(
        &self,
        input: Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError>;
}

// ── StatelessActionAdapter ──────────────────────────────────────────────────

/// Wraps a [`StatelessAction`] as a [`dyn StatelessHandler`].
///
/// Handles JSON deserialization of input and serialization of output so the
/// runtime can work with untyped JSON throughout, while action authors write
/// strongly-typed Rust.
pub struct StatelessActionAdapter<A> {
    action: A,
}

impl<A> StatelessActionAdapter<A> {
    /// Wrap a typed stateless action.
    #[must_use]
    pub fn new(action: A) -> Self {
        Self { action }
    }

    /// Consume the adapter, returning the inner action.
    #[must_use]
    pub fn into_inner(self) -> A {
        self.action
    }
}

#[async_trait::async_trait]
impl<A> StatelessHandler for StatelessActionAdapter<A>
where
    A: StatelessAction + Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        <A as Action>::metadata()
    }

    async fn execute(
        &self,
        input: Value,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let typed_input: <A as Action>::Input = serde_json::from_value(input).map_err(|e| {
            ActionError::validation(
                "input",
                ValidationReason::MalformedJson,
                Some(e.to_string()),
            )
        })?;

        let result = self.action.execute(typed_input, ctx).await?;

        result.try_map_output(|output| {
            serde_json::to_value(output)
                .map_err(|e| ActionError::fatal(format!("output serialization failed: {e}")))
        })
    }
}

impl<A: Action> fmt::Debug for StatelessActionAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StatelessActionAdapter")
            .field("action", &<A as Action>::metadata().base.key)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, OnceLock};

    use nebula_core::Dependencies;
    use nebula_schema::{HasSchema, ValidSchema};
    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::testing::{TestActionContext, TestContextBuilder};

    fn make_ctx() -> TestActionContext {
        TestContextBuilder::new().build()
    }

    // ── StatelessActionAdapter tests ──────────────────────────────────────

    #[derive(Debug, Deserialize)]
    struct AddInput {
        a: i64,
        b: i64,
    }

    impl HasSchema for AddInput {
        fn schema() -> ValidSchema {
            use nebula_schema::{FieldCollector, Schema, field_key};
            Schema::builder()
                .integer(field_key!("a"), |n| n)
                .integer(field_key!("b"), |n| n)
                .build()
                .expect("AddInput schema is valid")
        }
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct AddOutput {
        sum: i64,
    }

    impl HasSchema for AddOutput {
        fn schema() -> ValidSchema {
            use nebula_schema::{FieldCollector, Schema, field_key};
            Schema::builder()
                .integer(field_key!("sum"), |n| n)
                .build()
                .expect("AddOutput schema is valid")
        }
    }

    struct AddAction;

    impl Action for AddAction {
        type Input = AddInput;
        type Output = AddOutput;

        fn metadata() -> &'static ActionMetadata {
            static M: OnceLock<ActionMetadata> = OnceLock::new();
            M.get_or_init(|| {
                ActionMetadata::new(
                    nebula_core::action_key!("math.add"),
                    "Add",
                    "Adds two numbers",
                )
            })
        }
        fn input_schema() -> &'static ValidSchema {
            static S: OnceLock<ValidSchema> = OnceLock::new();
            S.get_or_init(<AddInput as HasSchema>::schema)
        }
        fn output_schema() -> &'static ValidSchema {
            static S: OnceLock<ValidSchema> = OnceLock::new();
            S.get_or_init(<AddOutput as HasSchema>::schema)
        }
        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StatelessAction for AddAction {
        async fn execute(
            &self,
            input: <Self as Action>::Input,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
            Ok(ActionResult::success(AddOutput {
                sum: input.a + input.b,
            }))
        }
    }

    #[tokio::test]
    async fn adapter_executes_typed_action() {
        let adapter = StatelessActionAdapter::new(AddAction);
        let ctx = make_ctx();

        let input = serde_json::json!({ "a": 3, "b": 7 });
        let result = StatelessHandler::execute(&adapter, input, &ctx)
            .await
            .unwrap();

        match result {
            ActionResult::Success { output } => {
                let v = output.into_value().unwrap();
                let out: AddOutput = serde_json::from_value(v).unwrap();
                assert_eq!(out.sum, 10);
            },
            _ => panic!("expected Success"),
        }
    }

    #[tokio::test]
    async fn adapter_returns_validation_error_on_bad_input() {
        let adapter = StatelessActionAdapter::new(AddAction);
        let ctx = make_ctx();

        let bad_input = serde_json::json!({ "x": "not a number" });
        let err = StatelessHandler::execute(&adapter, bad_input, &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Validation { .. }));
    }

    #[tokio::test]
    async fn adapter_exposes_metadata() {
        let adapter = StatelessActionAdapter::new(AddAction);
        assert_eq!(
            StatelessHandler::metadata(&adapter).base.key,
            nebula_core::action_key!("math.add")
        );
    }

    #[test]
    fn adapter_is_dyn_compatible() {
        let adapter = StatelessActionAdapter::new(AddAction);
        let _: Arc<dyn StatelessHandler> = Arc::new(adapter);
    }

    #[tokio::test]
    async fn stateless_adapter_implements_stateless_handler() {
        let adapter = StatelessActionAdapter::new(AddAction);
        let handler: Arc<dyn StatelessHandler> = Arc::new(adapter);
        let ctx = make_ctx();

        let input = serde_json::json!({ "a": 5, "b": 3 });
        let result = handler.execute(input, &ctx).await.unwrap();

        match result {
            ActionResult::Success { output } => {
                let v = output.into_value().unwrap();
                let out: AddOutput = serde_json::from_value(v).unwrap();
                assert_eq!(out.sum, 8);
            },
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn stateless_adapter_into_inner_returns_action() {
        let adapter = StatelessActionAdapter::new(AddAction);
        let _action = adapter.into_inner();
        assert_eq!(
            <AddAction as Action>::metadata().base.key,
            nebula_core::action_key!("math.add")
        );
    }
}
