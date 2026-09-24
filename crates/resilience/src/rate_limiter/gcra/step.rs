//! Pure GCRA transitions over [`GcraState`].
//!
//! Every function takes the current state and the store's `now` and returns
//! the decision plus the state to write back (`None` = write nothing). A
//! store runs one of them atomically per key; nothing here reads a clock.
//! All arithmetic saturates, so no input can panic.

use std::time::Duration;

use super::{Denied, GcraState, Grant, Rate, nanos};

/// The rate a key enforces for one call: the caller's `requested` rate when
/// the key is idle (nothing booked past `now`), otherwise the stricter of
/// `requested` and the rate the key has been enforcing.
///
/// GCRA arithmetic on one TAT assumes one interval, and callers of a shared
/// key may disagree (two rows of one provider account with different
/// overrides, two plugin versions mid-rollout). Taking the stricter rate
/// while any booking is outstanding never admits more than either caller
/// allows; an idle key forgets, so a loosened limit takes effect as soon as
/// the key drains.
#[must_use]
pub fn effective_rate(state: GcraState, now: u64, stored: Option<&Rate>, requested: &Rate) -> Rate {
    match stored {
        Some(stored) if state.tat > now => stored.stricter(requested),
        _ => *requested,
    }
}

/// Books `permits` if their slot is at most `max_wait` away.
///
/// The slot is `max(tat, now) + (permits − 1)·T − τ`; on success the TAT
/// advances by `permits·T` from `max(tat, now)`. A refusal returns `None`
/// for the state: nothing is consumed. Zero permits is a no-op grant.
pub fn reserve(
    state: GcraState,
    now: u64,
    rate: &Rate,
    permits: u32,
    max_wait: Duration,
) -> (Result<Grant, Denied>, Option<GcraState>) {
    if permits == 0 {
        let grant = Grant {
            wait: Duration::ZERO,
            permits: 0,
            allow_at: now,
            end_tat: state.tat,
            seq: state.seq,
        };
        return (Ok(grant), None);
    }
    if permits > rate.burst().get() {
        return (
            Err(Denied::Never {
                burst: rate.burst().get(),
            }),
            None,
        );
    }
    let base = state.tat.max(now);
    let extra = rate.emission_nanos().saturating_mul(u64::from(permits - 1));
    let allow_at = base
        .saturating_add(extra)
        .saturating_sub(rate.tolerance_nanos())
        .max(now);
    let wait = allow_at - now;
    let max_wait = nanos(max_wait);
    if wait > max_wait {
        let retry_after = Duration::from_nanos(wait);
        return (Err(Denied::Later { retry_after }), None);
    }
    let next = GcraState {
        tat: base.saturating_add(rate.emission_nanos().saturating_mul(u64::from(permits))),
        seq: state.seq.wrapping_add(1),
    };
    let grant = Grant {
        wait: Duration::from_nanos(wait),
        permits,
        allow_at,
        end_tat: next.tat,
        seq: next.seq,
    };
    (Ok(grant), Some(next))
}

/// Blocks the key until `now + min(retry_after, max_penalty)`.
///
/// Monotonic: it only ever moves the schedule later, so it cannot release
/// slots already booked, and it also blocks an idle key (whose TAT is in the
/// past). A request of `n` permits waits `(n − 1)·T` beyond the penalty.
#[must_use]
pub fn penalize(
    state: GcraState,
    now: u64,
    rate: &Rate,
    retry_after: Duration,
    max_penalty: Duration,
) -> GcraState {
    let until = now.saturating_add(nanos(retry_after.min(max_penalty)));
    GcraState {
        tat: state.tat.max(until.saturating_add(rate.tolerance_nanos())),
        seq: state.seq.wrapping_add(1),
    }
}

/// Returns `grant`'s permits if it is still the most recent mutation of the
/// key and its slot has not arrived yet; `None` otherwise.
///
/// Only the tail can be returned: refunding a reservation with later ones
/// behind it would let those later slots run early and exceed the rate. The
/// `(end_tat, seq)` match makes a repeated cancel a no-op instead of
/// refunding someone else's reservation.
#[must_use]
pub fn cancel(state: GcraState, now: u64, rate: &Rate, grant: &Grant) -> Option<GcraState> {
    // The grant recorded the state it left behind as `(end_tat, seq)`.
    let left_behind = GcraState {
        tat: grant.end_tat,
        seq: grant.seq,
    };
    let is_tail = state == left_behind;
    if grant.permits == 0 || !is_tail || grant.allow_at <= now {
        return None;
    }
    let refund = rate
        .emission_nanos()
        .saturating_mul(u64::from(grant.permits));
    Some(GcraState {
        tat: state.tat.saturating_sub(refund),
        seq: state.seq.wrapping_add(1),
    })
}

/// Permits that could be reserved right now without waiting.
#[must_use]
pub fn available(state: GcraState, now: u64, rate: &Rate) -> u32 {
    let base = state.tat.max(now);
    let headroom = now.saturating_add(rate.tolerance_nanos());
    if headroom < base {
        return 0;
    }
    let fits = (headroom - base) / rate.emission_nanos() + 1;
    u32::try_from(fits)
        .unwrap_or(u32::MAX)
        .min(rate.burst().get())
}
