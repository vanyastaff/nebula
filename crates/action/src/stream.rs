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
/// ```rust
/// use std::sync::OnceLock;
///
/// use futures::stream;
/// use nebula_action::prelude::*;
/// use nebula_action::StreamAction;
/// use nebula_core::action_key;
///
/// struct SumStream;
///
/// impl Action for SumStream {
///     type Input  = serde_json::Value;
///     type Output = u64;
///
///     fn metadata() -> nebula_action::ActionMetadataDraft {
///         nebula_action::ActionMetadataDraft::new(action_key!("demo.sum_stream"), nebula_action::metadata_name!("SumStream"), "Sums a chunk stream")
///     }
///     fn dependencies() -> &'static Dependencies {
///         static D: OnceLock<Dependencies> = OnceLock::new();
///         D.get_or_init(Dependencies::new)
///     }
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
///
/// // The adapter folds the stream; here we exercise the fold seam directly:
/// // init() + 1 + 2 + 3 == 6.
/// let action = SumStream;
/// let total = [1u64, 2, 3]
///     .into_iter()
///     .fold(action.init(), |acc, chunk| action.fold(acc, chunk));
/// assert_eq!(total, 6);
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
#[path = "stream_tests.rs"]
mod tests;
