//! Bounded topology — a runtime concurrency cap over a non-pooled resource.
//!
//! `Bounded` fills the one capability gap the other two built-ins leave open:
//! a resource that must limit how many leases are live at once, but that does
//! **not** keep a warm idle pool of interchangeable instances. Three modes,
//! selected at construction by a *runtime* value (not a const generic):
//!
//! | Mode | Cap gate | Instance lifecycle | Use case |
//! |------|----------|--------------------|----------|
//! | [`BoundedMode::Unbounded`] | none | fresh per lease, destroyed on release | no cap, no reuse |
//! | [`BoundedMode::Capped`] | `Semaphore(n)` | fresh per lease, destroyed on release | license seats, connection cap |
//! | [`BoundedMode::Exclusive`] | `Semaphore(1)` | **one** instance reused, reset on release | serial device / single session |
//!
//! Bounded is **not** a pool: `Capped`/`Unbounded` build a fresh instance per
//! acquire and destroy it on release (no idle reuse — that is what
//! [`Pooled`](crate::topology::Pooled) is for). Only `Exclusive` reuses its one
//! instance, resetting it between leases; a failed reset destroys the instance
//! rather than hand a half-reset one to the next acquirer (the S4 invariant),
//! and the single permit is held until the reset resolves so no acquirer can
//! observe the in-between state.
//!
//! A resource that declares `type Topology = Bounded<Self>` implements the
//! [`BoundedProvider`] hook trait so the framework
//! [`Bounded`](crate::topology::Bounded) topology can drive its reset policy.

use std::num::NonZeroUsize;

use async_trait::async_trait;

use crate::{error::Error, resource::Provider};

/// The runtime concurrency-cap mode of a [`Bounded`](crate::topology::Bounded)
/// topology.
///
/// Chosen by value at construction (see [`Bounded::capped`],
/// [`Bounded::exclusive`], [`Bounded::unbounded`]); the cap is read from a
/// runtime value, never a const generic, so the same code path serves a
/// config-driven seat count.
///
/// [`Bounded::capped`]: crate::topology::Bounded::capped
/// [`Bounded::exclusive`]: crate::topology::Bounded::exclusive
/// [`Bounded::unbounded`]: crate::topology::Bounded::unbounded
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BoundedMode {
    /// At most `n` concurrent leases (`n >= 1`). Each lease is an independent
    /// instance, created on acquire and destroyed on release — there is no idle
    /// reuse. Backs the gate with a `tokio::Semaphore(n)`.
    Capped(NonZeroUsize),
    /// Exactly one lease at a time, over a single reused instance. Backs the
    /// gate with a `tokio::Semaphore(1)`; the instance is reset on release
    /// (see [`BoundedProvider::reset`]) and the permit is held until the reset
    /// resolves.
    Exclusive,
    /// No concurrency limit. Each lease is an independent instance, created on
    /// acquire and destroyed on release; `try_reserve` always succeeds.
    Unbounded,
}

/// Bounded provider hooks — reset policy for the reused exclusive instance.
///
/// A resource that declares `type Topology = Bounded<Self>` implements this
/// trait. Only [`BoundedMode::Exclusive`] reuses its instance and therefore
/// calls [`reset`](BoundedProvider::reset); `Capped` / `Unbounded` destroy
/// every instance on release and never reset, so the default no-op suffices for
/// them.
#[async_trait]
pub trait BoundedProvider: Provider {
    /// Reset the single exclusive instance before it is reissued to the next
    /// acquirer (roll back a transaction, clear per-session state, re-arm a
    /// device).
    ///
    /// Runs on the release path **only** for [`BoundedMode::Exclusive`], while
    /// the lease's permit is still held, so no other acquirer can observe the
    /// instance mid-reset.
    ///
    /// # Errors
    ///
    /// `Err` ⇒ the framework destroys the instance instead of reusing it (a
    /// half-reset instance is never handed onward — the S4 invariant) and
    /// surfaces the error to an awaited
    /// [`release()`](crate::guard::ResourceGuard::release); a fresh instance is
    /// built on the next acquire. The default implementation never fails.
    async fn reset(&self, _instance: &mut Self::Instance) -> Result<(), Error> {
        Ok(())
    }
}
