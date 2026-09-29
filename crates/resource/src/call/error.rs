//! The error of one managed unit: what went wrong, whether the provider may
//! have seen it, and whether it is safe to retry.

use std::{fmt, time::Duration};

use nebula_core::ResourceKey;

use super::cost::{Effect, SentState};
use crate::{
    error::{Error, ErrorKind},
    rate_limit::DEFAULT_MAX_PENALTY,
};

/// The error of a managed unit ([`Unit`](super::Unit)).
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
#[derive(Debug, Clone)]
pub struct OpError {
    kind: ErrorKind,
    detail: &'static str,
    sent: SentState,
    effect: Effect,
    resource_key: Option<ResourceKey>,
}

impl OpError {
    /// An error of `kind` with a static, secret-free `detail`. Its sent
    /// state and effect are set by the runtime when the unit settles.
    #[must_use]
    pub fn new(kind: ErrorKind, detail: &'static str) -> Self {
        Self {
            kind,
            detail,
            sent: SentState::NotSent,
            effect: Effect::Write,
            resource_key: None,
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

impl From<Error> for OpError {
    /// Keeps the kind and the resource key; drops the message and source,
    /// which may carry provider or request data.
    fn from(error: Error) -> Self {
        Self {
            kind: error.kind().clone(),
            detail: "resource error",
            sent: SentState::NotSent,
            effect: Effect::Write,
            resource_key: error.resource_key().cloned(),
        }
    }
}

impl From<OpError> for Error {
    /// A retry-unsafe unit becomes [`ErrorKind::OutcomeUnknown`], so no
    /// caller retries an effect that may have been applied; any other keeps
    /// its kind, with an `Exhausted` hint capped as
    /// [`OpError::retry_after`] caps it.
    fn from(error: OpError) -> Self {
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

impl fmt::Display for OpError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(key) = &self.resource_key {
            write!(formatter, "[{key}] ")?;
        }
        write!(formatter, "{} ({}; {})", self.detail, self.kind, self.sent)
    }
}

impl std::error::Error for OpError {}
