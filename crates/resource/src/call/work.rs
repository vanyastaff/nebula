//! What the unit runtime runs: the private [`UnitWork`] trait and its
//! adapter for a public [`Operation`] ([`Plain`]).
//!
//! Every public entry point — [`submit`](super::ResourceHandle::submit),
//! [`session`](super::ResourceHandle::session) and
//! [`submit_streaming`](super::ResourceHandle::submit_streaming) — hands the
//! runtime a [`UnitWork`]: its declaration ([`Declared`]), the inputs of a
//! journaled route (the developer key part, the canonical request and the
//! output codec) and the body. The runtime routes the unit from the
//! declaration and the caller's authority; none of the adapters is an
//! [`Operation`], so the public trait stays what authors implement.

use std::{future::Future, num::NonZeroU32, time::Duration};

use super::{
    Operation, cost::Effect, declaration::canonical_json, error::OperationError, journal::UnitKind,
    managed::OperationCx, owned::OutputCodec, pin::PinSlots,
};
use crate::resource::Provider;

/// A unit's declaration: what the runtime routes it by and what a journal
/// records it under.
#[derive(Debug, Clone, Copy)]
pub(super) struct Declared {
    pub(super) kind: UnitKind,
    /// The operation key or the session name.
    pub(super) name: &'static str,
    pub(super) version: u32,
    pub(super) effect: Effect,
    pub(super) key_window: Duration,
    pub(super) record_output: bool,
}

/// One unit of work as the runtime runs it.
pub(super) trait UnitWork<R: Provider + PinSlots>: Send + 'static {
    /// What a successful unit yields.
    type Output: Send + 'static;

    /// The unit's declaration.
    fn declared(&self) -> Declared;

    /// A defect a builder found and could not report (a session name that
    /// breaks the key rules, a request that does not canonicalize): the
    /// unit is refused at submit on every route.
    fn defect(&self) -> Option<&'static str> {
        None
    }

    /// The developer part of the provider idempotency key.
    fn key_part(&self) -> Option<String>;

    /// The canonical request a journal digests; asked for only on a
    /// journaled route.
    ///
    /// # Errors
    ///
    /// Why the request has no canonical form.
    fn canonical_request(&self) -> Result<Vec<u8>, OperationError>;

    /// How a journal records and replays the output; `None` for a unit that
    /// cannot be journaled (a stream).
    fn codec() -> Option<OutputCodec<Self::Output>>;

    /// How many attempts the unit may be granted.
    fn max_attempts(&self) -> NonZeroU32;

    /// Runs the unit.
    fn run(
        self,
        cx: &mut OperationCx<'_, R>,
    ) -> impl Future<Output = Result<Self::Output, OperationError>> + Send;
}

/// A submitted [`Operation`] as unit work: the operation's constants are its
/// declaration and the operation value its canonical request.
pub(super) struct Plain<O>(pub(super) O);

impl<R, O> UnitWork<R> for Plain<O>
where
    R: Provider + PinSlots,
    O: Operation<R>,
{
    type Output = O::Output;

    fn declared(&self) -> Declared {
        Declared {
            kind: UnitKind::Operation,
            name: O::KEY,
            version: O::VERSION,
            effect: O::EFFECT,
            key_window: O::KEY_WINDOW,
            record_output: O::RECORD_OUTPUT,
        }
    }

    fn key_part(&self) -> Option<String> {
        self.0.idempotency_key()
    }

    fn canonical_request(&self) -> Result<Vec<u8>, OperationError> {
        canonical_json(&self.0)
    }

    fn codec() -> Option<OutputCodec<O::Output>> {
        Some(OutputCodec::json())
    }

    fn max_attempts(&self) -> NonZeroU32 {
        self.0.max_attempts()
    }

    async fn run(self, cx: &mut OperationCx<'_, R>) -> Result<O::Output, OperationError> {
        self.0.run(cx).await
    }
}
