//! Single-key, in-process GCRA limiter.

use std::time::Duration;

use parking_lot::Mutex;
use tokio::time::Instant;

use super::{Denied, GcraState, Grant, Rate, nanos, step};
use crate::{
    CallError,
    rate_limiter::{RateLimiter, RateLimiterStatus},
};

/// An in-process GCRA limiter for one key.
///
/// Time comes from `tokio::time`, so paused-clock tests are deterministic.
/// [`RateLimiter::acquire`] keeps the trait's fail-fast contract (it never
/// waits); waiting until a deadline is [`Gcra::until_ready`], and booking a
/// future slot is [`Gcra::reserve`].
///
/// # Examples
///
/// ```rust
/// use std::{num::NonZeroU32, time::Duration};
///
/// use nebula_resilience::rate_limiter::gcra::{Gcra, Rate};
///
/// # #[tokio::main(flavor = "current_thread", start_paused = true)]
/// # async fn main() {
/// let rate = Rate::per_second(NonZeroU32::new(2).unwrap());
/// let limiter = Gcra::new(rate);
/// limiter.until_ready(1, None).await.expect("first permit is free");
/// // The second waits half a second, then goes.
/// limiter
///     .until_ready(1, Some(tokio::time::Instant::now() + Duration::from_secs(1)))
///     .await
///     .expect("second permit within the deadline");
/// # }
/// ```
#[derive(Debug)]
pub struct Gcra {
    rate: Rate,
    base: Instant,
    state: Mutex<GcraState>,
}

impl Gcra {
    /// A limiter enforcing `rate`, with its full burst available.
    #[must_use]
    pub fn new(rate: Rate) -> Self {
        Self {
            rate,
            base: Instant::now(),
            state: Mutex::new(GcraState::default()),
        }
    }

    /// The enforced rate.
    #[must_use]
    pub const fn rate(&self) -> Rate {
        self.rate
    }

    fn now(&self) -> u64 {
        nanos(self.base.elapsed())
    }

    /// Books `permits` if their slot is at most `max_wait` away; the grant
    /// says how long to wait. A refusal consumes nothing.
    ///
    /// # Errors
    ///
    /// [`Denied::Later`] when the slot is further than `max_wait`;
    /// [`Denied::Never`] when `permits` exceeds the burst.
    pub fn reserve(&self, permits: u32, max_wait: Duration) -> Result<Grant, Denied> {
        let mut state = self.state.lock();
        let (decision, next) = step::reserve(*state, self.now(), &self.rate, permits, max_wait);
        if let Some(next) = next {
            *state = next;
        }
        decision
    }

    /// Waits until `permits` may be used, never past `deadline`
    /// (`None` = no deadline).
    ///
    /// # Errors
    ///
    /// The [`Denied`] from [`reserve`](Self::reserve); nothing is consumed.
    ///
    /// # Cancel safety
    ///
    /// Cancelling during the wait forfeits the booked slot unless the caller
    /// [`cancel`](Self::cancel)s it; the limiter errs on sending less.
    pub async fn until_ready(
        &self,
        permits: u32,
        deadline: Option<Instant>,
    ) -> Result<Grant, Denied> {
        let max_wait = deadline.map_or(Duration::MAX, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        let grant = self.reserve(permits, max_wait)?;
        if !grant.wait.is_zero() {
            tokio::time::sleep(grant.wait).await;
        }
        Ok(grant)
    }

    /// Blocks every caller for `retry_after` (capped at `max_penalty`), for
    /// a provider's `Retry-After`.
    pub fn penalize(&self, retry_after: Duration, max_penalty: Duration) {
        let mut state = self.state.lock();
        *state = step::penalize(*state, self.now(), &self.rate, retry_after, max_penalty);
    }

    /// Returns `grant`'s permits if it is still the latest reservation and
    /// its slot has not arrived; `false` when nothing was returned.
    pub fn cancel(&self, grant: &Grant) -> bool {
        let mut state = self.state.lock();
        step::cancel(*state, self.now(), &self.rate, grant).is_some_and(|next| {
            *state = next;
            true
        })
    }

    /// Permits available right now without waiting.
    #[must_use]
    pub fn available(&self) -> u32 {
        step::available(*self.state.lock(), self.now(), &self.rate)
    }
}

impl RateLimiter for Gcra {
    async fn acquire(&self) -> Result<(), CallError<()>> {
        match self.reserve(1, Duration::ZERO) {
            Ok(_) => Ok(()),
            Err(Denied::Later { retry_after }) => Err(CallError::rate_limited_after(retry_after)),
            // Unreachable for one permit (the burst is at least one), but a
            // refusal is still a refusal.
            Err(Denied::Never { .. }) => Err(CallError::rate_limited()),
        }
    }

    async fn status(&self) -> RateLimiterStatus {
        RateLimiterStatus::new(
            f64::from(self.available()),
            Some(self.rate.per_second_f64()),
        )
    }

    async fn reset(&self) {
        let mut state = self.state.lock();
        // Keep counting `seq` so a grant issued before the reset can never
        // match (and cancel against) the fresh state.
        *state = GcraState {
            tat: 0,
            seq: state.seq.wrapping_add(1),
        };
    }
}
