//! Per-resource rate limiting.
//!
//! A [`RateLimiter`] spreads work over time with the generic cell rate
//! algorithm (GCRA): `requests` per `period`, with up to `burst` of them back
//! to back. One limiter belongs to one registry row and is consumed both when
//! a lease is acquired and, through the guard, per outbound call — a single
//! long-lived lease (a `Resident` bot client) can otherwise make any number
//! of calls.
//!
//! When the limit is exhausted the caller waits for its slot, but never past
//! its deadline: if the slot lands after the deadline the caller gets
//! [`ErrorKind::Exhausted`](crate::ErrorKind::Exhausted) with a `retry_after`
//! immediately, and no slot is consumed. The limiter is local process state,
//! so its denial is not a backend-health signal and never trips a
//! [`RecoveryGate`](crate::RecoveryGate).
//!
//! State is one atomic: the theoretical arrival time of the next request, in
//! nanoseconds since the limiter was built. Time comes from `tokio::time`, so
//! paused-clock tests are deterministic. A reservation committed by a caller
//! that is then cancelled is not refunded — the limiter errs on the side of
//! sending less, never more.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use nebula_schema::Schema;
use serde::{Deserialize, Serialize};
use tokio::time::Instant;

use crate::error::Error;

/// Operator settings for a [`RateLimiter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Schema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct RateLimitSettings {
    /// Requests permitted per `period_ms`; at least 1.
    #[field(label = "Requests", description = "Permitted per period; at least 1")]
    pub requests: u32,
    /// Window the requests are spread over, in milliseconds; positive.
    #[field(
        label = "Period (ms)",
        description = "Window the requests are spread over"
    )]
    pub period_ms: u64,
    /// Requests allowed back to back after an idle spell; defaults to 1.
    #[field(
        label = "Burst",
        description = "Back-to-back requests after idling; default 1"
    )]
    #[serde(default)]
    pub burst: Option<u32>,
}

impl RateLimitSettings {
    /// `requests` per `period_ms`, no burst.
    #[must_use]
    pub const fn new(requests: u32, period_ms: u64) -> Self {
        Self {
            requests,
            period_ms,
            burst: None,
        }
    }

    /// Allows `burst` requests back to back.
    #[must_use]
    pub const fn with_burst(mut self, burst: u32) -> Self {
        self.burst = Some(burst);
        self
    }
}

/// Why a [`RateLimiter::reserve`] was denied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum RateLimitDenial {
    /// The slot is further away than the allowed wait; retry after this long.
    Later(Duration),
    /// More permits were requested than the burst can ever grant at once.
    ExceedsBurst,
}

/// A lock-free GCRA rate limiter. See the [module docs](self).
#[derive(Debug)]
pub struct RateLimiter {
    base: Instant,
    /// Time between two requests at the steady rate, in nanoseconds.
    emission_nanos: u64,
    /// How far ahead of now the schedule may run (`emission * burst`).
    tolerance_nanos: u64,
    burst: u32,
    settings: RateLimitSettings,
    /// Theoretical arrival time of the next request, nanoseconds since `base`.
    tat: AtomicU64,
}

impl RateLimiter {
    /// Builds a limiter from validated settings.
    ///
    /// # Errors
    ///
    /// Returns a permanent [`Error`] when `requests`, `period_ms` or `burst`
    /// is zero, or when the rate is too high to represent (less than one
    /// nanosecond between requests).
    pub fn new(settings: RateLimitSettings) -> Result<Self, Error> {
        if settings.requests == 0 {
            return Err(Error::permanent("rate limit: requests must be at least 1"));
        }
        if settings.period_ms == 0 {
            return Err(Error::permanent("rate limit: period_ms must be positive"));
        }
        let burst = settings.burst.unwrap_or(1);
        if burst == 0 {
            return Err(Error::permanent("rate limit: burst must be at least 1"));
        }
        let period_nanos = u128::from(settings.period_ms) * 1_000_000;
        let emission_nanos = u64::try_from(period_nanos / u128::from(settings.requests))
            .ok()
            .filter(|nanos| *nanos > 0)
            .ok_or_else(|| Error::permanent("rate limit: rate is too high to represent"))?;
        let tolerance_nanos = emission_nanos
            .checked_mul(u64::from(burst))
            .ok_or_else(|| Error::permanent("rate limit: burst window is too large"))?;
        Ok(Self {
            base: Instant::now(),
            emission_nanos,
            tolerance_nanos,
            burst,
            settings,
            tat: AtomicU64::new(0),
        })
    }

    /// The settings this limiter was built from.
    #[must_use]
    pub const fn settings(&self) -> RateLimitSettings {
        self.settings
    }

    fn now_nanos(&self) -> u64 {
        u64::try_from(self.base.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }

    /// Reserves `permits` and returns how long the caller must wait before
    /// using them, provided that wait is at most `max_wait` (`None` = no
    /// bound). A denied reservation consumes nothing.
    ///
    /// # Errors
    ///
    /// [`RateLimitDenial::Later`] when the slot is further than `max_wait`;
    /// [`RateLimitDenial::ExceedsBurst`] when `permits` exceeds the burst.
    pub fn reserve(
        &self,
        permits: u32,
        max_wait: Option<Duration>,
    ) -> Result<Duration, RateLimitDenial> {
        if permits == 0 {
            return Ok(Duration::ZERO);
        }
        if permits > self.burst {
            return Err(RateLimitDenial::ExceedsBurst);
        }
        let cost = self.emission_nanos.saturating_mul(u64::from(permits));
        let mut tat = self.tat.load(Ordering::Acquire);
        loop {
            let now = self.now_nanos();
            let next_tat = tat.max(now).saturating_add(cost);
            let wait = Duration::from_nanos((next_tat - now).saturating_sub(self.tolerance_nanos));
            if max_wait.is_some_and(|max| wait > max) {
                return Err(RateLimitDenial::Later(wait));
            }
            match self
                .tat
                .compare_exchange_weak(tat, next_tat, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Ok(wait),
                Err(current) => tat = current,
            }
        }
    }

    /// Waits until `permits` may be used, but never past `deadline`.
    ///
    /// # Errors
    ///
    /// - [`ErrorKind::Exhausted`](crate::ErrorKind::Exhausted) with a
    ///   `retry_after` when the slot lands after `deadline`; nothing is
    ///   consumed.
    /// - [`ErrorKind::Permanent`](crate::ErrorKind::Permanent) when `permits`
    ///   exceeds the configured burst and can never be granted.
    ///
    /// # Cancel safety
    ///
    /// Cancelling during the wait forfeits the reserved slot; it is not
    /// refunded.
    pub async fn until_ready(&self, permits: u32, deadline: Option<Instant>) -> Result<(), Error> {
        let max_wait = deadline.map(|deadline| deadline.saturating_duration_since(Instant::now()));
        match self.reserve(permits, max_wait) {
            Ok(wait) => {
                if !wait.is_zero() {
                    tokio::time::sleep(wait).await;
                }
                Ok(())
            },
            Err(RateLimitDenial::Later(retry_after)) => Err(Error::exhausted(
                "rate limit exhausted before the deadline",
                Some(retry_after),
            )),
            Err(RateLimitDenial::ExceedsBurst) => Err(Error::permanent(format!(
                "rate limit: {permits} permits exceed the burst of {}",
                self.burst
            ))),
        }
    }
}

#[cfg(test)]
#[path = "rate_limit_tests.rs"]
mod tests;
