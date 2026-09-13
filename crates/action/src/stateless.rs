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

use std::{fmt, future::Future, sync::Arc};

use serde_json::Value;

use crate::{
    ActionInput,
    action::Action,
    context::ActionContext,
    error::ActionError,
    handle::StatelessHandle,
    input::{ActionInputContract, PreparedActionInput},
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
/// [`Action`]; concrete implementations declare them on the
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
    #[must_use = "an action does nothing unless its returned future is awaited"]
    fn execute(
        &self,
        input: <Self as Action>::Input,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ActionResult<<Self as Action>::Output>, ActionError>> + Send;
}

// ── StatelessActionAdapter ──────────────────────────────────────────────────

/// Wraps a [`StatelessAction`] as a [`dyn StatelessHandle`].
///
/// Prepares input through its admitted schema before typed decoding, and
/// handles serialization of output so the
/// runtime can work with untyped JSON throughout, while action authors write
/// strongly-typed Rust.
pub struct StatelessActionAdapter<A> {
    action: A,
    meta: Arc<ActionMetadata>,
    input_contract: ActionInputContract,
}

impl<A> crate::handle::sealed::Stateless for StatelessActionAdapter<A> {}

impl<A> StatelessActionAdapter<A> {
    /// Wrap a typed stateless action.
    ///
    /// # Errors
    /// Returns a typed catalog error if metadata or an associated schema is invalid.
    #[tracing::instrument(name = "action.metadata.admit", skip_all, err)]
    pub fn new(action: A) -> Result<Self, crate::ActionMetadataAdmissionError>
    where
        A: Action,
    {
        let meta =
            Arc::new(<A as Action>::metadata().admit_for::<A>(crate::ActionKind::Stateless)?);
        let input_contract = ActionInputContract::new(meta.base().schema());
        Ok(Self {
            action,
            meta,
            input_contract,
        })
    }

    /// Consume the adapter, returning the inner action.
    #[must_use]
    pub fn into_inner(self) -> A {
        self.action
    }
}

#[async_trait::async_trait]
impl<A> StatelessHandle for StatelessActionAdapter<A>
where
    A: StatelessAction + Send + Sync + 'static,
{
    fn metadata(&self) -> &Arc<ActionMetadata> {
        &self.meta
    }

    fn prepare_input(&self, input: ActionInput) -> Result<PreparedActionInput, ActionError> {
        self.input_contract.prepare::<A::Input>(input)
    }

    async fn dispatch(
        &self,
        input: PreparedActionInput,
        ctx: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let typed_input = input.into_typed::<A::Input>(&self.input_contract)?;

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
            .field("action", self.meta.base().key())
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

    async fn execute(
        handler: &(impl StatelessHandle + ?Sized),
        input: Value,
        context: &dyn ActionContext,
    ) -> Result<ActionResult<Value>, ActionError> {
        let input = handler.prepare_input(ActionInput::Raw(input))?;
        handler.dispatch(input, context).await
    }

    // ── StatelessActionAdapter tests ──────────────────────────────────────

    #[derive(Debug, Deserialize)]
    struct AddInput {
        a: i64,
        b: i64,
    }

    impl HasSchema for AddInput {
        fn schema() -> Result<ValidSchema, nebula_schema::ValidationReport> {
            use nebula_schema::{FieldCollector, Schema, field_key};
            Schema::builder()
                .integer(field_key!("a"), |n| n)
                .integer(field_key!("b"), |n| n)
                .build()
        }
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct AddOutput {
        sum: i64,
    }

    impl HasSchema for AddOutput {
        fn schema() -> Result<ValidSchema, nebula_schema::ValidationReport> {
            use nebula_schema::{FieldCollector, Schema, field_key};
            Schema::builder().integer(field_key!("sum"), |n| n).build()
        }
    }

    struct AddAction;

    impl Action for AddAction {
        type Input = AddInput;
        type Output = AddOutput;

        fn metadata() -> crate::ActionMetadataDraft {
            crate::ActionMetadataDraft::new(
                nebula_core::action_key!("math.add"),
                crate::metadata_name!("Add"),
                "Adds two numbers",
            )
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
        let adapter =
            StatelessActionAdapter::new(AddAction).expect("valid test catalog definition");
        let ctx = make_ctx();

        let input = serde_json::json!({ "a": 3, "b": 7 });
        let result = execute(&adapter, input, &ctx).await.unwrap();

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
        let adapter =
            StatelessActionAdapter::new(AddAction).expect("valid test catalog definition");
        let ctx = make_ctx();

        let bad_input = serde_json::json!({ "x": "not a number" });
        let err = execute(&adapter, bad_input, &ctx).await.unwrap_err();
        assert!(matches!(err, ActionError::Validation { .. }));
    }

    #[tokio::test]
    async fn adapter_exposes_metadata() {
        let adapter =
            StatelessActionAdapter::new(AddAction).expect("valid test catalog definition");
        assert_eq!(
            adapter.metadata().base().key().clone(),
            nebula_core::action_key!("math.add")
        );
    }

    #[test]
    fn adapter_is_dyn_compatible() {
        let adapter =
            StatelessActionAdapter::new(AddAction).expect("valid test catalog definition");
        let _: Arc<dyn StatelessHandle> = Arc::new(adapter);
    }

    #[tokio::test]
    async fn stateless_adapter_implements_stateless_handle() {
        let adapter =
            StatelessActionAdapter::new(AddAction).expect("valid test catalog definition");
        let handler: Arc<dyn StatelessHandle> = Arc::new(adapter);
        let ctx = make_ctx();

        let input = serde_json::json!({ "a": 5, "b": 3 });
        let result = execute(handler.as_ref(), input, &ctx).await.unwrap();

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
        let adapter =
            StatelessActionAdapter::new(AddAction).expect("valid test catalog definition");
        let key = adapter.metadata().base().key().clone();
        let _action = adapter.into_inner();
        assert_eq!(key, nebula_core::action_key!("math.add"));
    }
}
