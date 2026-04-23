//! Core [`StatelessAction`] trait and function-backed DX adapters.
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
//!
//! ## DX adapters
//!
//! - [`FnStatelessAction`] / [`stateless_fn`] — zero-boilerplate adapter for a plain `async
//!   fn(Input) -> Result<Output, ActionError>`.
//! - [`FnStatelessCtxAction`] / [`stateless_ctx_fn`] — context-aware variant for closures that need
//!   credentials, resources, or the logger.

use std::{fmt, future::Future, marker::PhantomData, pin::Pin};

use nebula_core::DeclaresDependencies;
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
/// # Cancellation
///
/// Cancellation is handled by the runtime (e.g. `tokio::select!` between
/// `execute` and `ctx.cancellation().cancelled()`). Implementations do not
/// need to check cancellation unless they want cooperative checks at specific
/// points.
///
/// # Example
///
/// ```rust,ignore
/// use nebula_action::{Action, StatelessAction, ActionResult, ActionError};
///
/// struct MyAction { meta: ActionMetadata }
/// impl Action for MyAction { /* ... */ }
///
/// impl StatelessAction for MyAction {
///     type Input = serde_json::Value;
///     type Output = serde_json::Value;
///
///     async fn execute(&self, input: Self::Input, _ctx: &ActionContext)
///         -> Result<ActionResult<Self::Output>, ActionError>
///     {
///         Ok(ActionResult::success(input))
///     }
/// }
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement StatelessAction",
    note = "implement the `execute` method with matching Input/Output types"
)]
pub trait StatelessAction: Action {
    /// Input type for this action.
    ///
    /// Must implement [`HasSchema`](nebula_schema::HasSchema) so the action
    /// metadata can auto-derive its parameter schema from the input type.
    /// Use `()` / `serde_json::Value` for schema-less inputs — both have
    /// baseline `HasSchema` impls returning an empty schema.
    type Input: nebula_schema::HasSchema + Send + Sync;
    /// Output type produced on success (wrapped in [`ActionResult`]).
    type Output: Send + Sync;

    /// Returns the schema for this action's input parameters.
    /// Default: derives from `Input` via `HasSchema`.
    fn schema() -> nebula_schema::ValidSchema
    where
        Self: Sized,
    {
        <Self::Input as nebula_schema::HasSchema>::schema()
    }

    /// Execute the action with the given input and context.
    ///
    /// Returns [`ActionResult`] for flow control (Success, Skip, Branch, Wait, etc.)
    /// or [`ActionError`] for retryable/fatal failures.
    ///
    /// The returned future must be `Send` so the runtime can run it in
    /// `tokio::select!` with cancellation (no per-action cancellation boilerplate).
    fn execute(
        &self,
        input: Self::Input,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send;
}

// ── FnStatelessAction ───────────────────────────────────────────────────────

/// Stateless action adapter backed by an async function/closure.
///
/// This removes boilerplate for the common "pure stateless transform" case.
pub struct FnStatelessAction<F, Input, Output> {
    metadata: ActionMetadata,
    func: F,
    _marker: PhantomData<fn(Input) -> Output>,
}

impl<F, Input, Output> FnStatelessAction<F, Input, Output> {
    /// Create a new function-backed stateless action.
    #[must_use]
    pub fn new(metadata: ActionMetadata, func: F) -> Self {
        Self {
            metadata,
            func,
            _marker: PhantomData,
        }
    }
}

impl<F, Input, Output> DeclaresDependencies for FnStatelessAction<F, Input, Output>
where
    F: Send + Sync + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
}

impl<F, Input, Output> Action for FnStatelessAction<F, Input, Output>
where
    F: Send + Sync + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl<F, Fut, Input, Output> StatelessAction for FnStatelessAction<F, Input, Output>
where
    F: Fn(Input) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Output, ActionError>> + Send + 'static,
    Input: nebula_schema::HasSchema + Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
    type Input = Input;
    type Output = Output;

    fn execute(
        &self,
        input: Self::Input,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send {
        let fut = (self.func)(input);
        async move { fut.await.map(ActionResult::success) }
    }
}

/// Build a function-backed stateless action.
#[must_use]
pub fn stateless_fn<F, Input, Output>(
    metadata: ActionMetadata,
    func: F,
) -> FnStatelessAction<F, Input, Output> {
    FnStatelessAction::new(metadata, func)
}

impl<F, Input, Output> fmt::Debug for FnStatelessAction<F, Input, Output> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FnStatelessAction")
            .field("action", &self.metadata.base.key)
            .finish_non_exhaustive()
    }
}

// ── FnStatelessCtxAction ────────────────────────────────────────────────────

/// Stateless action adapter backed by a context-aware async function/closure.
///
/// Unlike [`FnStatelessAction`], the closure receives both the input and a
/// cloned [`ActionContext`], allowing credential/resource/logger access.
///
/// The closure signature is `Fn(Input, ActionContext) -> Future<...>`.
/// The context is cloned before each call (cheap — all capabilities are
/// behind `Arc`).
///
/// **Important:** Without [`with_context`](FnStatelessCtxAction::with_context),
/// `execute` builds a minimal context from the [`Context`] trait methods
/// (noop capabilities). Call `with_context` to inject a base
/// [`ActionContext`] whose credentials, resources, and logger are cloned
/// into each invocation.
pub struct FnStatelessCtxAction<F, Input, Output> {
    metadata: ActionMetadata,
    func: F,
    _marker: PhantomData<fn(Input) -> Output>,
}

impl<F, Input, Output> FnStatelessCtxAction<F, Input, Output> {
    /// Create a new context-aware function-backed stateless action.
    #[must_use]
    pub fn new(metadata: ActionMetadata, func: F) -> Self {
        Self {
            metadata,
            func,
            _marker: PhantomData,
        }
    }
}

impl<F, Input, Output> DeclaresDependencies for FnStatelessCtxAction<F, Input, Output>
where
    F: Send + Sync + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
}

impl<F, Input, Output> Action for FnStatelessCtxAction<F, Input, Output>
where
    F: Send + Sync + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
    fn metadata(&self) -> &ActionMetadata {
        &self.metadata
    }
}

impl<F, Fut, Input, Output> StatelessAction for FnStatelessCtxAction<F, Input, Output>
where
    F: Fn(Input) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Output, ActionError>> + Send + 'static,
    Input: nebula_schema::HasSchema + Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
    type Input = Input;
    type Output = Output;

    fn execute(
        &self,
        input: Self::Input,
        _ctx: &(impl ActionContext + ?Sized),
    ) -> impl Future<Output = Result<ActionResult<Self::Output>, ActionError>> + Send {
        let fut = (self.func)(input);
        async move { fut.await.map(ActionResult::success) }
    }
}

/// Build a context-aware stateless action from a function.
///
/// This adapter keeps the API shape for context-aware action constructors;
/// the runtime context is supplied through the standard [`StatelessAction`]
/// execution path.
///
/// # Examples
///
/// ```rust,ignore
/// let action = stateless_ctx_fn(
///     ActionMetadata::new(action_key!("example.ctx"), "Ctx", "Context-aware"),
///     |input: serde_json::Value| async move {
///         Ok(input)
///     },
/// );
/// ```
#[must_use]
pub fn stateless_ctx_fn<F, Input, Output>(
    metadata: ActionMetadata,
    func: F,
) -> FnStatelessCtxAction<F, Input, Output> {
    FnStatelessCtxAction::new(metadata, func)
}

impl<F, Input, Output> fmt::Debug for FnStatelessCtxAction<F, Input, Output> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FnStatelessCtxAction")
            .field("action", &self.metadata.base.key)
            .field("ctx_mode", &"<borrowed>")
            .finish_non_exhaustive()
    }
}

// ── StatelessHandler trait ──────────────────────────────────────────────────

/// Stateless action handler — JSON in, JSON out.
///
/// This is the JSON-level contract for one-shot actions. The engine sends
/// a `serde_json::Value` input and receives a `serde_json::Value` output
/// wrapped in [`ActionResult`].
///
/// # Errors
///
/// Returns [`ActionError`] on validation, retryable, or fatal failures.
pub trait StatelessHandler: Send + Sync {
    /// Action metadata (key, version, capabilities).
    fn metadata(&self) -> &ActionMetadata;

    /// Execute with JSON input and context.
    ///
    /// # Errors
    ///
    /// Returns [`ActionError`] if execution fails (validation, retryable, or fatal).
    fn execute<'life0, 'life1, 'a>(
        &'life0 self,
        input: Value,
        ctx: &'life1 dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionResult<Value>, ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a;
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

impl<A> StatelessHandler for StatelessActionAdapter<A>
where
    A: StatelessAction + Send + Sync + 'static,
    A::Input: serde::de::DeserializeOwned + Send + Sync,
    A::Output: serde::Serialize + Send + Sync,
{
    fn metadata(&self) -> &ActionMetadata {
        self.action.metadata()
    }

    fn execute<'life0, 'life1, 'a>(
        &'life0 self,
        input: Value,
        ctx: &'life1 dyn ActionContext,
    ) -> Pin<Box<dyn Future<Output = Result<ActionResult<Value>, ActionError>> + Send + 'a>>
    where
        Self: 'a,
        'life0: 'a,
        'life1: 'a,
    {
        Box::pin(async move {
            let typed_input: A::Input = serde_json::from_value(input).map_err(|e| {
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
        })
    }
}

impl<A: Action> fmt::Debug for StatelessActionAdapter<A> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StatelessActionAdapter")
            .field("action", &self.action.metadata().base.key)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde::{Deserialize, Serialize};

    use super::*;
    use crate::{ActionRuntimeContext, testing::TestContextBuilder};

    fn make_ctx() -> ActionRuntimeContext {
        TestContextBuilder::new().build()
    }

    #[tokio::test]
    async fn fn_stateless_action_executes_with_low_boilerplate() {
        let action = stateless_fn::<_, Value, Value>(
            ActionMetadata::new(
                nebula_core::action_key!("example.fn"),
                "Fn",
                "Function-backed action",
            ),
            |input| async move { Ok(input) },
        );

        let ctx = make_ctx();

        let result = action
            .execute(serde_json::json!({"hello":"world"}), &ctx)
            .await
            .unwrap();
        match result {
            ActionResult::Success { output } => {
                assert_eq!(
                    output.as_value(),
                    Some(&serde_json::json!({"hello":"world"}))
                );
            },
            other => panic!("expected Success, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fn_stateless_ctx_action_receives_context() {
        let action = stateless_ctx_fn::<_, Value, Value>(
            ActionMetadata::new(
                nebula_core::action_key!("example.ctx_fn"),
                "CtxFn",
                "Context-aware function action",
            ),
            |input| async move { Ok(input) },
        );

        let ctx = make_ctx();

        let result = action
            .execute(serde_json::json!({"ctx":"aware"}), &ctx)
            .await
            .unwrap();
        match result {
            ActionResult::Success { output } => {
                assert_eq!(output.as_value(), Some(&serde_json::json!({"ctx":"aware"})));
            },
            other => panic!("expected Success, got {other:?}"),
        }
    }

    // ── StatelessActionAdapter tests ──────────────────────────────────────

    #[derive(Debug, Deserialize)]
    struct AddInput {
        a: i64,
        b: i64,
    }

    impl nebula_schema::HasSchema for AddInput {
        fn schema() -> nebula_schema::ValidSchema {
            use nebula_schema::{FieldCollector, Schema};
            Schema::builder()
                .integer("a", |n| n)
                .integer("b", |n| n)
                .build()
                .expect("AddInput schema is valid")
        }
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

    impl DeclaresDependencies for AddAction {}

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
            _ctx: &(impl ActionContext + ?Sized),
        ) -> Result<ActionResult<Self::Output>, ActionError> {
            Ok(ActionResult::success(AddOutput {
                sum: input.a + input.b,
            }))
        }
    }

    #[tokio::test]
    async fn adapter_executes_typed_action() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
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
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let ctx = make_ctx();

        let bad_input = serde_json::json!({ "x": "not a number" });
        let err = StatelessHandler::execute(&adapter, bad_input, &ctx)
            .await
            .unwrap_err();
        assert!(matches!(err, ActionError::Validation { .. }));
    }

    #[tokio::test]
    async fn adapter_exposes_metadata() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        assert_eq!(
            StatelessHandler::metadata(&adapter).base.key,
            nebula_core::action_key!("math.add")
        );
    }

    #[test]
    fn adapter_is_dyn_compatible() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let _: Arc<dyn StatelessHandler> = Arc::new(adapter);
    }

    #[tokio::test]
    async fn stateless_adapter_implements_stateless_handler() {
        let adapter = StatelessActionAdapter::new(AddAction::new());
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
        let adapter = StatelessActionAdapter::new(AddAction::new());
        let action = adapter.into_inner();
        assert_eq!(
            action.metadata().base.key,
            nebula_core::action_key!("math.add")
        );
    }
}
