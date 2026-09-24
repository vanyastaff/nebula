//! Generic cell rate algorithm (GCRA): a steady rate with a bounded burst,
//! and reservations of future slots.
//!
//! One integer per key — the *theoretical arrival time* (TAT) of the next
//! permit — is the whole state, so the same [`step`] functions run over an
//! in-process mutex ([`Gcra`], [`MemoryLimitStore`]) and inside one atomic
//! statement of a shared store (SQL `UPDATE … RETURNING`, a Redis script).
//!
//! What sets this limiter apart from the others in the crate:
//!
//! - **Reservation.** [`step::reserve`] books the next slot even when it lies
//!   in the future and reports how long to wait, so waiters are served in
//!   arrival order and a durable caller can park until its slot instead of
//!   polling for one.
//! - **Deadline-bounded.** A slot beyond `max_wait` is refused *without*
//!   consuming anything; allow, wait-until-deadline and reserve are the same
//!   call with `max_wait` of zero, the deadline, or [`Duration::MAX`].
//! - **Penalty.** [`step::penalize`] pushes the schedule past a provider's
//!   `Retry-After`, for every caller of the key at once, and is capped.
//! - **Exact integers.** The interval between permits is rounded *up* to
//!   whole nanoseconds, so the configured rate is never exceeded (rounding
//!   down, as some implementations do, admits an extra permit at window
//!   edges).
//!
//! Time is an opaque monotonic `u64` of nanoseconds chosen by the store: a
//! process-local store counts from its own start on `tokio::time`, a shared
//! store uses its own server clock. Callers only ever see relative
//! [`Duration`]s.

use std::{fmt, num::NonZeroU32, time::Duration};

use crate::ConfigError;

mod limiter;
pub mod step;
mod store;

pub use limiter::Gcra;
pub use store::{
    ErasedLimitStore, LimitKey, LimitStore, LimitStoreError, MAX_LIMIT_KEY_BYTES, MemoryLimitStore,
    ReservationId, ReserveRequest,
};

const NANOS_PER_SECOND: u64 = 1_000_000_000;

fn nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// A steady rate with a burst allowance.
///
/// Built from a count per period ([`Rate::new`], [`Rate::per_second`]) or from
/// a provider's window quota ([`Rate::per_window`]). The interval between
/// permits is rounded up to whole nanoseconds.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rate {
    emission_nanos: u64,
    tolerance_nanos: u64,
    burst: NonZeroU32,
}

impl fmt::Debug for Rate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Rate")
            .field("interval", &self.emission_interval())
            .field("burst", &self.burst)
            .finish_non_exhaustive()
    }
}

impl Rate {
    /// `requests` spread evenly over `period`, no burst.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] when `period` is zero or longer than ~584 years per
    /// permit.
    pub fn new(requests: NonZeroU32, period: Duration) -> Result<Self, ConfigError> {
        let period_nanos = period.as_nanos();
        if period_nanos == 0 {
            return Err(ConfigError::new("period", "must be greater than zero"));
        }
        let emission = period_nanos.div_ceil(u128::from(requests.get()));
        let emission_nanos = u64::try_from(emission)
            .map_err(|_| ConfigError::new("period", "interval between permits is too long"))?;
        Ok(Self {
            emission_nanos,
            tolerance_nanos: 0,
            burst: NonZeroU32::MIN,
        })
    }

    /// `requests` per second, no burst.
    #[must_use]
    pub const fn per_second(requests: NonZeroU32) -> Self {
        Self {
            emission_nanos: NANOS_PER_SECOND.div_ceil(requests.get() as u64),
            tolerance_nanos: 0,
            burst: NonZeroU32::MIN,
        }
    }

    /// Allows `burst` permits back to back after an idle spell.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] when the burst window does not fit in `u64`
    /// nanoseconds.
    pub fn with_burst(self, burst: NonZeroU32) -> Result<Self, ConfigError> {
        let tolerance_nanos = self
            .emission_nanos
            .checked_mul(u64::from(burst.get() - 1))
            .ok_or_else(|| ConfigError::new("burst", "burst window is too long"))?;
        Ok(Self {
            tolerance_nanos,
            burst,
            ..self
        })
    }

    /// The fastest rate that never lets more than `limit` permits into any
    /// half-open window of length `window`, keeping `burst` back to back
    /// available, with `safety_margin_percent` of the quota held back for
    /// clock skew and other clients of the same account.
    ///
    /// GCRA with burst `B` and interval `T` admits at most
    /// `B + ceil(W / T) − 1` permits in a window `W`; this picks
    /// `T = ceil(W / (N − B + 1))` for the effective limit `N`, so that bound
    /// is at most `N`.
    ///
    /// # Errors
    ///
    /// [`ConfigError`] when the margin is 100 % or more, the effective limit
    /// is below the burst, or the window is zero or too long.
    pub fn per_window(
        limit: NonZeroU32,
        window: Duration,
        burst: NonZeroU32,
        safety_margin_percent: u8,
    ) -> Result<Self, ConfigError> {
        if safety_margin_percent >= 100 {
            return Err(ConfigError::new(
                "safety_margin_percent",
                "must be below 100",
            ));
        }
        let effective = u64::from(limit.get()) * u64::from(100 - safety_margin_percent) / 100;
        let slots = effective
            .checked_sub(u64::from(burst.get()))
            .map(|spare| spare + 1)
            .ok_or_else(|| {
                ConfigError::new("burst", "burst exceeds the limit left after the margin")
            })?;
        let slots = u32::try_from(slots)
            .ok()
            .and_then(NonZeroU32::new)
            .ok_or_else(|| ConfigError::new("limit", "limit is out of range"))?;
        Self::new(slots, window)?.with_burst(burst)
    }

    /// Interval between permits at the steady rate.
    #[must_use]
    pub const fn emission_interval(&self) -> Duration {
        Duration::from_nanos(self.emission_nanos)
    }

    /// Permits available back to back after an idle spell.
    #[must_use]
    pub const fn burst(&self) -> NonZeroU32 {
        self.burst
    }

    /// Steady-state permits per second.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "a nanosecond interval as f64 is exact below 2^53 ns (~104 days)"
    )]
    pub fn per_second_f64(&self) -> f64 {
        NANOS_PER_SECOND as f64 / self.emission_nanos as f64
    }

    /// `true` when this rate never admits more than `other`: a permit
    /// interval at least as long and a burst no larger. This is the check a
    /// "may only tighten" override uses — comparing normalized intervals,
    /// not raw count/period fields, which a changed period would bypass.
    #[must_use]
    pub const fn is_no_looser_than(&self, other: &Self) -> bool {
        self.emission_nanos >= other.emission_nanos && self.burst.get() <= other.burst.get()
    }

    pub(crate) const fn emission_nanos(&self) -> u64 {
        self.emission_nanos
    }

    pub(crate) const fn tolerance_nanos(&self) -> u64 {
        self.tolerance_nanos
    }
}

/// Serialisable form of a [`Rate`]: `requests` per `period_ms`, with an
/// optional `burst` (default 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(deny_unknown_fields))]
#[non_exhaustive]
pub struct RateConfig {
    /// Permits per period; at least 1.
    pub requests: u32,
    /// Period in milliseconds; positive.
    pub period_ms: u64,
    /// Back-to-back permits after an idle spell; defaults to 1.
    #[cfg_attr(feature = "serde", serde(default))]
    pub burst: Option<u32>,
}

impl RateConfig {
    /// `requests` per `period_ms`, no burst.
    #[must_use]
    pub const fn new(requests: u32, period_ms: u64) -> Self {
        Self {
            requests,
            period_ms,
            burst: None,
        }
    }

    /// Allows `burst` permits back to back.
    #[must_use]
    pub const fn with_burst(mut self, burst: u32) -> Self {
        self.burst = Some(burst);
        self
    }
}

impl TryFrom<RateConfig> for Rate {
    type Error = ConfigError;

    fn try_from(config: RateConfig) -> Result<Self, Self::Error> {
        let requests = NonZeroU32::new(config.requests)
            .ok_or_else(|| ConfigError::new("requests", "must be at least 1"))?;
        let burst = NonZeroU32::new(config.burst.unwrap_or(1))
            .ok_or_else(|| ConfigError::new("burst", "must be at least 1"))?;
        Self::new(requests, Duration::from_millis(config.period_ms))?.with_burst(burst)
    }
}

/// Why a reservation was refused. A refusal never consumes anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Denied {
    /// The slot is further away than the allowed wait.
    Later {
        /// Time until the slot arrives, when a request that waits for
        /// nothing would pass (the HTTP `Retry-After` meaning).
        retry_after: Duration,
    },
    /// More permits were requested than the burst can ever grant at once.
    Never {
        /// The configured burst.
        burst: u32,
    },
}

/// A booked reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grant {
    /// How long the caller waits before using the permits.
    pub wait: Duration,
    /// Permits booked.
    pub permits: u32,
    /// Store-clock time the permits may be used (for the store only).
    pub allow_at: u64,
    /// TAT this reservation left behind; identifies it for [`step::cancel`].
    pub end_tat: u64,
    /// Store sequence number after this reservation; with `end_tat` it
    /// makes cancellation exact (no ABA on a repeated cancel).
    pub seq: u64,
}

/// Persisted per-key state: the theoretical arrival time and a sequence
/// number bumped by every mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GcraState {
    /// Theoretical arrival time of the next permit (store clock, ns).
    pub tat: u64,
    /// Mutation counter; compared, never ordered.
    pub seq: u64,
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
