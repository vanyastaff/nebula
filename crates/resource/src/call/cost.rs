//! The vocabulary of one attempt: what it costs, whether it may be
//! replayed, and whether it reached the provider.

use std::{fmt, num::NonZeroU32};

/// What one provider attempt costs against the row's rate limit.
///
/// The facade books the cost when it grants the attempt
/// ([`OperationCx::attempt`](super::OperationCx::attempt)), never at acquire: a lease that
/// makes three calls books three costs (Design QUOTA-DX.md:32).
///
/// ```
/// use std::num::NonZeroU32;
///
/// use nebula_resource::call::Cost;
///
/// assert_eq!(Cost::ONE.permits(), 1);
/// assert_eq!(Cost::FREE.permits(), 0);
/// assert_eq!(Cost::units(NonZeroU32::new(3).unwrap()).permits(), 3);
/// // The key's value never reaches `Debug`.
/// let keyed = Cost::keyed("chat_id", 42);
/// assert!(!format!("{keyed:?}").contains("42"));
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct Cost {
    permits: u32,
    key: Option<(&'static str, String)>,
}

impl Cost {
    /// One permit of the account limit: one ordinary provider request.
    pub const ONE: Self = Self {
        permits: 1,
        key: None,
    };

    /// No permit: a call the provider does not count (a local buffer, a
    /// cached read). The attempt still passes admission — the lease must be
    /// open and the row admitting — but books nothing and never waits out a
    /// pause.
    pub const FREE: Self = Self {
        permits: 0,
        key: None,
    };

    /// `units` permits booked as one slot: a request the provider counts as
    /// several (a batch, a weighted endpoint). A cost above the limit's
    /// burst can never be granted and fails the attempt permanently.
    #[must_use]
    pub const fn units(units: NonZeroU32) -> Self {
        Self {
            permits: units.get(),
            key: None,
        }
    }

    /// One permit of the per-key limit `dimension` for `value` — one chat,
    /// one recipient — and one of the account limit. `dimension` must be
    /// declared with [`ResiliencePolicy::keyed`](crate::rate_limit::ResiliencePolicy::keyed).
    /// The value is hashed before it reaches a limit store and is never
    /// printed.
    #[must_use]
    pub fn keyed(dimension: &'static str, value: impl fmt::Display) -> Self {
        Self {
            permits: 1,
            key: Some((dimension, value.to_string())),
        }
    }

    /// Permits this cost books: `0` for [`FREE`](Self::FREE).
    #[must_use]
    pub const fn permits(&self) -> u32 {
        self.permits
    }

    /// Whether this cost books nothing.
    pub(crate) const fn is_free(&self) -> bool {
        self.permits == 0
    }

    /// The per-key limit this cost books, if any: `(dimension, value)`.
    pub(crate) fn key(&self) -> Option<(&'static str, &str)> {
        self.key
            .as_ref()
            .map(|(dimension, value)| (*dimension, value.as_str()))
    }
}

impl fmt::Debug for Cost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = formatter.debug_struct("Cost");
        debug.field("permits", &self.permits);
        if let Some((dimension, _)) = &self.key {
            debug.field("dimension", dimension);
            debug.field("value", &"<redacted>");
        }
        debug.finish()
    }
}

/// What repeating an operation does to the provider, declared once per
/// operation type ([`Operation::EFFECT`](super::Operation::EFFECT)).
///
/// It decides whether a unit whose attempt may have reached the provider is
/// safe to retry (Design DX-API.md:110): a replay-safe effect may be sent
/// again, a [`Write`](Self::Write) may not without reconciling first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum Effect {
    /// Reads only; sending it twice changes nothing. Never recorded: under
    /// an execution journal a replay asks the provider again, and an answer
    /// that changed since — and steers the program's later effects — makes
    /// the replay diverge, which halts it as an occurrence mismatch.
    Read,
    /// A read whose answer steers the program — a model's completion, a
    /// retrieval or search whose result decides what runs next — recorded
    /// under an execution journal so every replay observes the answer the
    /// program observed.
    ///
    /// Only for a call with **no provider-side effect**: the provider
    /// changes nothing it would not change for a plain read. A call that
    /// runs hosted tools (web search, a code interpreter), stores a
    /// response, appends to a server-side conversation or uploads a file is
    /// not a read: declare it [`Idempotent`](Self::Idempotent) or
    /// [`Write`](Self::Write), whose unknown outcomes are recovered as
    /// effects. Nothing can detect the difference at runtime: the
    /// declaration is the author's attestation.
    ///
    /// Under a journal the unit is prepared, granted and settled like an
    /// effect, with its output recorded (at most 1 MiB; never digest-only:
    /// [`Operation::RECORD_OUTPUT`](super::Operation::RECORD_OUTPUT) must
    /// stay `true`) before the caller sees it, and a replay yields the
    /// recorded answer without a provider call. Its outcome is never
    /// unknown: an unanswered call may be asked again. Without a journal
    /// (a library or read-only row) it runs as a plain [`Read`](Self::Read).
    RecordedRead,
    /// Changes provider state, but a repeat is absorbed (an idempotency key,
    /// a PUT of the same value, a flush).
    Idempotent,
    /// Changes provider state and a repeat applies again. The default: an
    /// operation has to declare that it is safe to replay.
    #[default]
    Write,
}

impl Effect {
    /// Whether sending the operation again after an unknown outcome cannot
    /// apply its effect twice.
    #[must_use]
    pub const fn is_replay_safe(self) -> bool {
        matches!(self, Self::Read | Self::RecordedRead | Self::Idempotent)
    }

    /// Stable lowercase name: `read`, `recorded_read`, `idempotent` or
    /// `write`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::RecordedRead => "recorded_read",
            Self::Idempotent => "idempotent",
            Self::Write => "write",
        }
    }
}

/// Whether a unit's provider attempts reached the provider.
///
/// Each attempt is finished from its call's classified result
/// ([`Attempt::finish`](super::Attempt::finish), which
/// [`OperationCx::call`](super::OperationCx::call) runs);
/// the runtime folds them into the unit's state. Ordered by how much it
/// commits the caller: `NotSent < Sent < MaybeSent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SentState {
    /// Nothing reached the provider. Proven by the runtime when no attempt
    /// was granted, whatever the author reported.
    NotSent,
    /// The provider received the request and answered.
    Sent,
    /// The request may have reached the provider, but no answer says whether
    /// it was applied: the attempt was left unsettled, the unit hit its
    /// deadline or panicked after a grant.
    MaybeSent,
}

impl SentState {
    /// Stable lowercase name: `not_sent`, `sent` or `maybe_sent`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotSent => "not_sent",
            Self::Sent => "sent",
            Self::MaybeSent => "maybe_sent",
        }
    }

    /// Rank in the fold: the worse of two states wins.
    pub(crate) const fn rank(self) -> u8 {
        match self {
            Self::NotSent => 0,
            Self::Sent => 1,
            Self::MaybeSent => 2,
        }
    }

    pub(crate) const fn from_rank(rank: u8) -> Self {
        match rank {
            0 => Self::NotSent,
            1 => Self::Sent,
            _ => Self::MaybeSent,
        }
    }
}

impl fmt::Display for SentState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
