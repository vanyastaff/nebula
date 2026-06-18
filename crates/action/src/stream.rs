//! Core [`StreamAction`] author trait.
//!
//! Stream actions open an async chunk stream and fold it into a single output
//! value in-process. They share the same one-shot dispatch shape as
//! [`StatelessAction`](crate::StatelessAction) but carry a distinct
//! [`ActionKind::Stream`](crate::metadata::ActionKind::Stream) and a richer
//! future seam — `open_stream` exposes the chunk boundary so later units
//! (S3 cursor-replay, S4 egress) can observe chunks without touching stateless
//! dispatch.
//!
//! ## Execution contract (S1)
//!
//! 1. The engine calls `StreamHandle::dispatch` once.
//! 2. The adapter deserializes input → `Self::Input`, calls `open_stream`.
//! 3. Every chunk is pulled in-process: `Ok(chunk)` is folded via `init`+`fold`;
//!    the first `Err(e)` short-circuits and returns the error — no partial output.
//! 4. The folded `Self::Output` is serialized → `Value` and wrapped in
//!    [`ActionResult::success`](crate::result::ActionResult::success).
//!
//! ## Cancellation
//!
//! The stream future is driven inside the adapter; callers that need
//! per-chunk cancellation should check `ctx.cancellation()` inside
//! `open_stream` at natural yield points.
//!
//! ## Extension seam
//!
//! `StreamHandle` is a **separate** trait from `StatelessHandle` so that S3
//! (cursor) and S4 (egress) can add chunk-observing methods to the handle
//! surface without touching stateless dispatch.

use futures::Stream;

use crate::{action::Action, context::ActionContext, error::ActionError};

/// Stream action: opens a chunk stream and folds it into a single output.
///
/// Authors implement `open_stream`, `init`, and `fold`. The engine adapter
/// drives the stream fully in-process and delivers one folded `ActionResult`
/// to the downstream node — identical to stateless from the engine's
/// perspective, except the kind is `ActionKind::Stream`.
///
/// `Self::Chunk` is the per-step emission type. `Self::Output` (from
/// [`Action`]) is the final accumulated value.
///
/// # Cancellation
///
/// Cancellation is handled by the runtime. To support cooperative cancellation,
/// implementations can check `ctx.cancellation()` inside `open_stream` at
/// natural suspension points.
///
/// # Example
///
/// ```rust,ignore
/// use futures::stream;
/// use nebula_action::prelude::*;
/// use nebula_action::stream::StreamAction;
///
/// struct SumStream;
///
/// impl Action for SumStream {
///     type Input  = serde_json::Value;
///     type Output = u64;
/// # fn metadata() -> nebula_action::ActionMetadata { todo!() }
/// # fn dependencies() -> &'static nebula_core::Dependencies { todo!() }
/// }
///
/// impl StreamAction for SumStream {
///     type Chunk = u64;
///
///     fn open_stream(
///         &self,
///         _input: serde_json::Value,
///         _ctx: &(impl ActionContext + ?Sized),
///     ) -> impl futures::Stream<Item = Result<u64, ActionError>> + Send {
///         stream::iter([Ok(1u64), Ok(2), Ok(3)])
///     }
///
///     fn init(&self) -> u64 { 0 }
///     fn fold(&self, acc: u64, chunk: u64) -> u64 { acc + chunk }
/// }
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` does not implement StreamAction",
    note = "implement `open_stream`, `init`, and `fold` (Chunk and Input/Output are associated types)"
)]
pub trait StreamAction: Action {
    /// The per-step chunk type emitted by the stream.
    type Chunk: Send;

    /// Open the async chunk stream for the given input.
    ///
    /// The returned stream must be `Send` so the engine can drive it in a
    /// Tokio task. Each `Ok(chunk)` is forwarded to [`Self::fold`]; the
    /// first `Err(e)` short-circuits the fold and propagates the error with
    /// no partial output emitted.
    #[must_use = "the stream does nothing until it is driven to completion"]
    fn open_stream(
        &self,
        input: <Self as Action>::Input,
        ctx: &(impl ActionContext + ?Sized),
    ) -> impl Stream<Item = Result<Self::Chunk, ActionError>> + Send;

    /// Build the initial accumulator before any chunk arrives.
    fn init(&self) -> <Self as Action>::Output;

    /// Fold one chunk into the running accumulator.
    fn fold(&self, acc: <Self as Action>::Output, chunk: Self::Chunk) -> <Self as Action>::Output;
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use futures::stream;
    use nebula_core::Dependencies;
    use nebula_schema::{HasSchema, ValidSchema};
    use nebula_workflow::NodeDefinition;
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    use super::*;
    use crate::{
        action::Action,
        context::ActionContext,
        error::ActionError,
        factory::{ActionFactory, GenericStreamFactory},
        from_workflow_node::FromWorkflowNode,
        handle::ActionHandle,
        metadata::{ActionKind, ActionMetadata},
        result::ActionResult,
        testing::TestContextBuilder,
    };

    fn make_ctx() -> crate::testing::TestActionContext {
        TestContextBuilder::new().build()
    }

    fn make_node(action_key: &str) -> NodeDefinition {
        NodeDefinition::new(
            nebula_core::NodeKey::new("test_node").unwrap(),
            action_key.to_owned(),
            "test_plugin",
            action_key,
        )
        .unwrap()
    }

    // ── SumStream fixture ────────────────────────────────────────────────────

    /// Yields Ok(1), Ok(2), Ok(3) and folds by sum → total must be 6.
    struct SumStream;

    #[derive(Debug, Deserialize)]
    struct SumInput {
        start: u8,
    }

    impl HasSchema for SumInput {
        fn schema() -> ValidSchema {
            ValidSchema::empty()
        }
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct SumOutput {
        total: u64,
    }

    impl HasSchema for SumOutput {
        fn schema() -> ValidSchema {
            ValidSchema::empty()
        }
    }

    impl Action for SumStream {
        type Input = SumInput;
        type Output = SumOutput;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(
                nebula_core::action_key!("test.stream.sum"),
                "SumStream",
                "Yields three chunks and sums them",
            )
        }

        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StreamAction for SumStream {
        type Chunk = u8;

        fn open_stream(
            &self,
            input: SumInput,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> impl Stream<Item = Result<Self::Chunk, ActionError>> + Send {
            // Emit three chunks starting from input.start; test passes start=0.
            stream::iter([
                Ok(input.start.saturating_add(1)),
                Ok(input.start.saturating_add(2)),
                Ok(input.start.saturating_add(3)),
            ])
        }

        fn init(&self) -> SumOutput {
            SumOutput { total: 0 }
        }

        fn fold(&self, acc: SumOutput, chunk: u8) -> SumOutput {
            SumOutput {
                total: acc.total + u64::from(chunk),
            }
        }
    }

    impl FromWorkflowNode for SumStream {
        type Error = ActionError;

        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(SumStream)
        }
    }

    // ── ErrorStream fixture ──────────────────────────────────────────────────

    /// Emits one chunk then an error — proves D-4 short-circuit with no partial output.
    struct ErrorStream;

    impl Action for ErrorStream {
        type Input = Value;
        type Output = Value;

        fn metadata() -> ActionMetadata {
            ActionMetadata::new(
                nebula_core::action_key!("test.stream.error"),
                "ErrorStream",
                "Emits one chunk then fails",
            )
        }

        fn dependencies() -> &'static Dependencies {
            static D: OnceLock<Dependencies> = OnceLock::new();
            D.get_or_init(Dependencies::new)
        }
    }

    impl StreamAction for ErrorStream {
        type Chunk = Value;

        fn open_stream(
            &self,
            _input: Value,
            _ctx: &(impl ActionContext + ?Sized),
        ) -> impl Stream<Item = Result<Self::Chunk, ActionError>> + Send {
            stream::iter([
                Ok(serde_json::json!(1)),
                Err(ActionError::retryable("stream chunk failed")),
            ])
        }

        fn init(&self) -> Value {
            serde_json::json!(0)
        }

        fn fold(&self, _acc: Value, chunk: Value) -> Value {
            chunk
        }
    }

    impl FromWorkflowNode for ErrorStream {
        type Error = ActionError;

        async fn from_workflow_node(
            _node: &NodeDefinition,
            _ctx: &dyn ActionContext,
        ) -> Result<Self, Self::Error> {
            Ok(ErrorStream)
        }
    }

    // ── Tests ────────────────────────────────────────────────────────────────

    /// Proves: factory produces `ActionHandle::Stream`, kind is `Stream`,
    /// dispatch folds Ok(1)+Ok(2)+Ok(3) → `Success { Value({"total":6}) }`.
    #[tokio::test]
    async fn sum_stream_folds_to_six() {
        let factory = GenericStreamFactory::<SumStream>::new();
        let node = make_node("test.stream.sum");
        let ctx = make_ctx();

        let handle = factory
            .instantiate(&node, &ctx)
            .await
            .expect("factory must produce a handle");

        let ActionHandle::Stream(stream_handle) = handle else {
            panic!("expected ActionHandle::Stream, got a different variant");
        };

        assert_eq!(
            stream_handle.metadata().kind,
            ActionKind::Stream,
            "factory must stamp ActionKind::Stream"
        );

        let result = stream_handle
            .dispatch(serde_json::json!({ "start": 0 }), &ctx)
            .await
            .expect("dispatch must succeed");

        match result {
            ActionResult::Success { output } => {
                let value = output.into_value().expect("output must be an inline Value");
                let out: SumOutput =
                    serde_json::from_value(value).expect("output must deserialize to SumOutput");
                assert_eq!(out.total, 6, "1+2+3 must fold to 6");
            },
            other => panic!("expected ActionResult::Success, got {other:?}"),
        }
    }

    /// Proves: `is_stream()` and `kind == Stream` set via factory.
    #[tokio::test]
    async fn stream_metadata_kind_and_predicate() {
        let factory = GenericStreamFactory::<SumStream>::new();
        let node = make_node("test.stream.sum");
        let ctx = make_ctx();

        let handle = factory.instantiate(&node, &ctx).await.unwrap();

        assert_eq!(
            handle.metadata().kind,
            ActionKind::Stream,
            "GenericStreamFactory must stamp ActionKind::Stream on the stored metadata"
        );
        assert!(
            handle.is_stream(),
            "is_stream() predicate must return true for ActionHandle::Stream"
        );
    }

    /// Proves D-4: a chunk `Err` short-circuits dispatch — no partial output, typed error returned.
    #[tokio::test]
    async fn error_chunk_short_circuits_with_no_partial_output() {
        let factory = GenericStreamFactory::<ErrorStream>::new();
        let node = make_node("test.stream.error");
        let ctx = make_ctx();

        let handle = factory.instantiate(&node, &ctx).await.unwrap();
        let ActionHandle::Stream(stream_handle) = handle else {
            panic!("expected ActionHandle::Stream");
        };

        let err = stream_handle
            .dispatch(serde_json::json!(null), &ctx)
            .await
            .expect_err("dispatch must propagate the chunk error without partial output");

        assert!(
            matches!(err, ActionError::Retryable { .. }),
            "the chunk error kind must be preserved; got {err:?}"
        );
    }
}
