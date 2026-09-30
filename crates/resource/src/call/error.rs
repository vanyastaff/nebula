//! The error of one managed unit: what went wrong, whether the provider may
//! have seen it, and whether it is safe to retry.

use std::{fmt, time::Duration};

use nebula_core::ResourceKey;

use super::cost::{Effect, SentState};
use crate::{
    error::{Error, ErrorKind},
    rate_limit::DEFAULT_MAX_PENALTY,
};

/// The error of a managed unit ([`Submission`](super::Submission)).
///
/// Carries an [`ErrorKind`], a static detail, and — once the runtime settled
/// the unit — the unit's [`SentState`] and its operation's [`Effect`]. From
/// those it decides [`is_retryable`](Self::is_retryable): a unit whose
/// attempt may have been applied is only retried when its effect is replay
/// safe.
///
/// Secret-free by construction: the detail is a `&'static str`, and
/// converting a resource [`Error`] keeps its kind and resource key only,
/// never its message or source (a provider error may echo request data).
/// For the same reason it has no [`source`](std::error::Error::source).
///
/// | Kind retryable? | Sent state | Effect | Retryable? | As [`Error`] |
/// |---|---|---|---|---|
/// | no | any | any | no | its kind |
/// | yes | `NotSent` | any | yes | its kind |
/// | yes | `Sent` | any, kind `Exhausted` | yes (the provider refused; nothing applied) | its kind |
/// | yes | `Sent` / `MaybeSent` | `Read` / `Idempotent` | yes | its kind |
/// | yes | `Sent` / `MaybeSent` | `Write` | no | [`ErrorKind::OutcomeUnknown`] |
///
/// # Classifying a provider call
///
/// A provider call made through [`OperationCx::call`](super::OperationCx::call)
/// (or finished with [`Attempt::finish`](super::Attempt::finish)) returns one
/// of the classified errors below, and the runtime derives everything else
/// from it: the attempt's [`SentState`], what the rate limit is told, how an
/// execution journal records the call, and whether `call` takes another
/// attempt (within [`Operation::max_attempts`](super::Operation::max_attempts)
/// and the unit's deadline).
///
/// | Error | Sent | Rate limit | Journal | Retried inside `call` |
/// |---|---|---|---|---|
/// | (success) | `Sent` | pass | applied | — |
/// | [`throttled`](Self::throttled) / [`throttled_key`](Self::throttled_key) | `Sent` (kind `Exhausted`) | throttled: the quota / the cost's key pauses | not crossed | yes |
/// | [`unreachable`](Self::unreachable) / [`unreachable_as`](Self::unreachable_as) | `NotSent` | nothing | not crossed | yes (`unreachable_as`: a retryable kind only) |
/// | [`interrupted`](Self::interrupted) | `MaybeSent` | nothing | ambiguous | only a replay-safe effect |
/// | [`rejected`](Self::rejected) / [`rejected_as`](Self::rejected_as) | `Sent` | pass | rejected | no |
/// | unclassified ([`new`](Self::new), `?` on an [`Error`]) | `MaybeSent` | nothing | ambiguous | only a replay-safe effect with a retryable kind |
///
/// Neither `unreachable` nor `interrupted` tells the rate limit anything,
/// so a failure never resets a backoff in progress. The throttle's pause is
/// waited out by the next attempt's quota booking; nothing sleeps.
#[derive(Debug, Clone)]
pub struct OperationError {
    kind: ErrorKind,
    detail: &'static str,
    sent: SentState,
    effect: Effect,
    resource_key: Option<ResourceKey>,
    signal: Signal,
}

/// What a provider call's error says about the call, as its constructor
/// classified it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Signal {
    /// Not classified: [`OperationError::new`] or a converted [`Error`].
    Unclassified,
    /// The provider asked to slow down; `per_key` pauses only the
    /// attempt's keyed cost.
    Throttled {
        /// Only the attempt's [`Cost::keyed`](super::Cost::keyed) key pauses.
        per_key: bool,
    },
    /// Provably not sent.
    Unreachable,
    /// May have been sent.
    Interrupted,
    /// Sent and definitively refused.
    Rejected,
}

impl OperationError {
    /// An error of `kind` with a static, secret-free `detail`. Its sent
    /// state and effect are set by the runtime when the unit settles.
    ///
    /// Returned from a provider call it is unclassified: the call may have
    /// been sent (see the type docs). Prefer a classifying constructor
    /// there.
    #[must_use]
    pub fn new(kind: ErrorKind, detail: &'static str) -> Self {
        Self {
            kind,
            detail,
            sent: SentState::NotSent,
            effect: Effect::Write,
            resource_key: None,
            signal: Signal::Unclassified,
        }
    }

    fn classified(kind: ErrorKind, detail: &'static str, signal: Signal) -> Self {
        Self {
            signal,
            ..Self::new(kind, detail)
        }
    }

    /// The provider asked to slow down (an HTTP `429`): it applied nothing.
    /// Kind [`ErrorKind::Exhausted`] with the provider's hint; the whole
    /// quota of the row pauses for `retry_after` (capped by the policy), or
    /// for a backoff when the provider named no time.
    #[must_use]
    pub fn throttled(retry_after: Option<Duration>) -> Self {
        Self::classified(
            ErrorKind::Exhausted { retry_after },
            "provider throttled the call",
            Signal::Throttled { per_key: false },
        )
    }

    /// As [`throttled`](Self::throttled), for a limit on the one key the
    /// attempt's [`Cost::keyed`](super::Cost::keyed) named — a chat's own
    /// flood limit: only that key pauses. An attempt whose cost named no key
    /// pauses the quota.
    #[must_use]
    pub fn throttled_key(retry_after: Option<Duration>) -> Self {
        Self::classified(
            ErrorKind::Exhausted { retry_after },
            "provider throttled the call's key",
            Signal::Throttled { per_key: true },
        )
    }

    /// The call provably never reached the provider (no connection, a
    /// refused request built before sending). Kind
    /// [`ErrorKind::Transient`], sent state `NotSent`: safe to retry for
    /// any effect.
    #[must_use]
    pub fn unreachable(detail: &'static str) -> Self {
        Self::classified(ErrorKind::Transient, detail, Signal::Unreachable)
    }

    /// As [`unreachable`](Self::unreachable), with a `kind` that says more
    /// about why nothing was sent: a local buffer full
    /// ([`ErrorKind::Backpressure`]), a client already shut down
    /// ([`ErrorKind::Cancelled`]). Sent state `NotSent`; retried inside a
    /// call only when the kind is retryable.
    #[must_use]
    pub fn unreachable_as(kind: ErrorKind, detail: &'static str) -> Self {
        Self::classified(kind, detail, Signal::Unreachable)
    }

    /// The call may have reached the provider, and no answer says whether
    /// it applied (a connection lost mid-request, a gateway timeout). Kind
    /// [`ErrorKind::Transient`], sent state `MaybeSent`: retried only for a
    /// replay-safe effect; a `Write` ends with an unknown outcome.
    #[must_use]
    pub fn interrupted(detail: &'static str) -> Self {
        Self::classified(ErrorKind::Transient, detail, Signal::Interrupted)
    }

    /// The provider answered and definitively refused the call (an HTTP
    /// `4xx`): kind [`ErrorKind::Permanent`], sent state `Sent`, never
    /// retried.
    #[must_use]
    pub fn rejected(detail: &'static str) -> Self {
        Self::classified(ErrorKind::Permanent, detail, Signal::Rejected)
    }

    /// As [`rejected`](Self::rejected), with a non-retryable `kind` that
    /// says more ([`ErrorKind::NotFound`], a refused credential). A kind
    /// that invites a retry is recorded [`ErrorKind::Permanent`]: a
    /// definitive refusal is never retried.
    #[must_use]
    pub fn rejected_as(kind: ErrorKind, detail: &'static str) -> Self {
        let kind = if kind.is_default_retryable() {
            ErrorKind::Permanent
        } else {
            kind
        };
        Self::classified(kind, detail, Signal::Rejected)
    }

    /// What the error says about the provider call.
    pub(crate) fn signal(&self) -> Signal {
        self.signal
    }

    /// The sent state of the attempt that returned this error.
    pub(crate) fn attempt_sent(&self) -> SentState {
        match self.signal {
            Signal::Throttled { .. } | Signal::Rejected => SentState::Sent,
            Signal::Unreachable => SentState::NotSent,
            Signal::Interrupted | Signal::Unclassified => SentState::MaybeSent,
        }
    }

    /// Whether [`OperationCx::call`](super::OperationCx::call) takes another
    /// attempt after an attempt of an `effect` operation returned this error
    /// (budget and deadline permitting).
    pub(crate) fn retried_in_call(&self, effect: Effect) -> bool {
        match self.signal {
            Signal::Throttled { .. } => true,
            Signal::Unreachable => self.kind.is_default_retryable(),
            Signal::Interrupted => effect.is_replay_safe(),
            Signal::Rejected => false,
            Signal::Unclassified => effect.is_replay_safe() && self.kind.is_default_retryable(),
        }
    }

    /// The error kind.
    #[must_use]
    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    /// Whether the unit's attempts reached the provider, as the runtime
    /// settled it. `NotSent` until the unit settles.
    #[must_use]
    pub fn sent(&self) -> SentState {
        self.sent
    }

    /// The effect declared by the unit's operation.
    #[must_use]
    pub fn effect(&self) -> Effect {
        self.effect
    }

    /// The row the unit ran against, once settled.
    #[must_use]
    pub fn resource_key(&self) -> Option<&ResourceKey> {
        self.resource_key.as_ref()
    }

    /// The static detail given when the error was raised.
    #[must_use]
    pub fn detail(&self) -> &'static str {
        self.detail
    }

    /// Whether retrying the unit is safe and useful; see the type docs for
    /// the table.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        if !self.kind_error().is_retryable() {
            return false;
        }
        match self.sent {
            SentState::NotSent => true,
            SentState::Sent => {
                matches!(self.kind, ErrorKind::Exhausted { .. }) || self.effect.is_replay_safe()
            },
            SentState::MaybeSent => self.effect.is_replay_safe(),
        }
    }

    /// How long to wait before retrying, for a retryable error: the kind's
    /// hint, capped at [`DEFAULT_MAX_PENALTY`] so a provider cannot park a
    /// caller for longer. `None` when the error is not retryable.
    #[must_use]
    pub fn retry_after(&self) -> Option<Duration> {
        if !self.is_retryable() {
            return None;
        }
        self.kind_error()
            .retry_after()
            .map(|after| after.min(DEFAULT_MAX_PENALTY))
    }

    /// Stamps the runtime's settlement onto the error.
    pub(crate) fn settled(mut self, sent: SentState, effect: Effect, key: &ResourceKey) -> Self {
        self.sent = sent;
        self.effect = effect;
        if self.resource_key.is_none() {
            self.resource_key = Some(key.clone());
        }
        self
    }

    /// Whether this error hides an effect that may have been applied: its
    /// kind is [`ErrorKind::OutcomeUnknown`] (an execution owner said so),
    /// or the kind invites a retry but the unit is not safe to retry.
    pub(crate) fn is_outcome_unknown(&self) -> bool {
        self.kind == ErrorKind::OutcomeUnknown
            || (self.kind_error().is_retryable() && !self.is_retryable())
    }

    /// A bare resource error of this kind, for the kind's classification.
    fn kind_error(&self) -> Error {
        Error::new(self.kind.clone(), String::new())
    }
}

impl From<Error> for OperationError {
    /// Keeps the kind and the resource key; drops the message and source,
    /// which may carry provider or request data.
    fn from(error: Error) -> Self {
        Self {
            kind: error.kind().clone(),
            detail: "resource error",
            sent: SentState::NotSent,
            effect: Effect::Write,
            resource_key: error.resource_key().cloned(),
            signal: Signal::Unclassified,
        }
    }
}

impl From<OperationError> for Error {
    /// A retry-unsafe unit becomes [`ErrorKind::OutcomeUnknown`], so no
    /// caller retries an effect that may have been applied; any other keeps
    /// its kind, with an `Exhausted` hint capped as
    /// [`OperationError::retry_after`] caps it.
    fn from(error: OperationError) -> Self {
        let converted = if error.is_outcome_unknown() {
            Error::outcome_unknown(format!(
                "{} ({}; {} effect {}; outcome unknown)",
                error.detail,
                error.kind,
                error.effect.as_str(),
                error.sent
            ))
        } else {
            let kind = match error.kind {
                ErrorKind::Exhausted { retry_after } => ErrorKind::Exhausted {
                    retry_after: retry_after.map(|after| after.min(DEFAULT_MAX_PENALTY)),
                },
                kind => kind,
            };
            let message = format!("{} ({kind}; {})", error.detail, error.sent);
            Error::new(kind, message)
        };
        match error.resource_key {
            Some(key) => converted.with_resource_key(key),
            None => converted,
        }
    }
}

impl fmt::Display for OperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(key) = &self.resource_key {
            write!(formatter, "[{key}] ")?;
        }
        write!(formatter, "{} ({}; {})", self.detail, self.kind, self.sent)
    }
}

impl std::error::Error for OperationError {}
