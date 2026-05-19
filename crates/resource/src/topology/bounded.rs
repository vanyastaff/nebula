//! Bounded topology — one parameterized trait folding the former
//! `Service`, `Transport`, and `Exclusive` topologies.
//!
//! A *bounded* resource holds one long-lived runtime and hands out
//! short-lived leases gated by a [`Cap`](CapMarker). The cap is a
//! **typestate associated marker**, not a runtime discriminant: the
//! release shape (whether a release hook must run, and how the concurrency
//! permit is sized) is wired through the type system so invalid
//! combinations — a tracked lease with no release hook, an exclusive
//! resource with no reset — are *compile* errors rather than silent
//! runtime no-ops.
//!
//! | Cap | Folds | Permits | `release_one` |
//! |-----|-------|---------|---------------|
//! | [`Unbounded`] | `Service` (`Cloned`) | none | not called (owned handle) |
//! | [`Capped<N>`] | `Service` (`Tracked`), `Transport` | `N` | required |
//! | [`Exclusive`] | `Exclusive` | `1` | required (the reset) |
//!
//! The three folded behaviors map onto one [`Bounded::acquire_one`] /
//! [`BoundedRelease::release_one`] pair:
//!
//! - **Service `Cloned`** → `Cap = Unbounded`: `acquire_one` mints a cheap
//!   token, the handle is owned, `release_one` is never invoked.
//! - **Service `Tracked`** → `Cap = Capped<N>`: `acquire_one` mints a
//!   tracked token, the handle is guarded, `release_one` returns it.
//! - **Transport** → `Cap = Capped<N>`: `acquire_one` opens a session
//!   (gated by `N = max_sessions`), `release_one` closes it; the optional
//!   [`BoundedRelease::keepalive`] is driven on an interval by the runtime.
//! - **Exclusive** → `Cap = Exclusive`: `acquire_one` clones the runtime
//!   into a lease behind a `Semaphore(1)`, `release_one` resets it; the
//!   permit is held until `release_one` resolves (#384), and a failed
//!   reset destroys the instance instead of handing it onward (S4).
//!
//! The `release_one` *error* is observed (tracing + a release-error
//! metric), never `let _ =`-swallowed (R17).

use std::future::Future;

use crate::{context::ResourceContext, resource::Resource};

/// Private module sealing [`CapMarker`] so the cap typestate set is closed
/// to this crate. External resources select a cap by name; they cannot
/// introduce a fourth shape that the runtime does not know how to drive.
mod sealed {
    /// Sealing super-trait — only the in-crate cap markers implement it.
    pub trait Sealed {}
}

/// Marker trait for the bounded concurrency caps.
///
/// Sealed: the closed set is [`Unbounded`], [`Capped<N>`], and
/// [`Exclusive`]. Each impl reports the runtime's semaphore sizing
/// ([`PERMITS`](CapMarker::PERMITS)) and whether the release hook is
/// part of the contract ([`RELEASE_REQUIRED`](CapMarker::RELEASE_REQUIRED)).
///
/// The release-shape is wired through the *type* rather than a runtime
/// `==`: a `Bounded` impl declares its cap as an associated type, so a
/// tracked-lease resource that forgets `release_one`, or an exclusive
/// resource that forgets `reset`, fails to type-check (see
/// [`CapMarker`] / the compile-fail probes) instead of silently
/// no-op-ing at run time.
pub trait CapMarker: sealed::Sealed + Send + Sync + 'static {
    /// Semaphore permit count for this cap.
    ///
    /// `None` means *no semaphore* — acquire is never gated (the former
    /// Service `Cloned` path). `Some(n)` sizes a `Semaphore(n)`:
    /// `Capped<N>` yields `Some(N)`, `Exclusive` yields `Some(1)`.
    const PERMITS: Option<usize>;

    /// Whether [`BoundedRelease::release_one`] participates in the lease
    /// lifecycle.
    ///
    /// `false` for [`Unbounded`] (owned handle, no callback). `true` for
    /// [`Capped<N>`] and [`Exclusive`] — the runtime hands out a guarded
    /// handle whose drop runs `release_one` and observes its error.
    const RELEASE_REQUIRED: bool;
}

/// Unbounded cap — no concurrency limit, no release hook.
///
/// Folds the former `Service` topology's *cloned* token mode: the runtime
/// hands out a cheap owned token and never runs a release callback.
#[derive(Debug, Clone, Copy)]
pub struct Unbounded;

impl sealed::Sealed for Unbounded {}
impl CapMarker for Unbounded {
    const PERMITS: Option<usize> = None;
    const RELEASE_REQUIRED: bool = false;
}

/// Capped cap — at most `N` concurrent leases, release hook required.
///
/// Folds the former `Service` topology's *tracked* token mode and the
/// former `Transport` topology: the runtime gates concurrency with a
/// `Semaphore(N)` and runs `release_one` (token return / session close)
/// on lease drop. For the transport shape, `N` is `max_sessions`.
#[derive(Debug, Clone, Copy)]
pub struct Capped<const N: usize>;

impl<const N: usize> sealed::Sealed for Capped<N> {}
impl<const N: usize> CapMarker for Capped<N> {
    const PERMITS: Option<usize> = Some(N);
    const RELEASE_REQUIRED: bool = true;
}

/// Exclusive cap — exactly one lease at a time, reset-on-release.
///
/// Folds the former `Exclusive` topology: a `Semaphore(1)` serializes
/// callers and `release_one` is the reset. The runtime holds the permit
/// until `release_one` resolves so the next caller cannot enter mid-reset
/// (#384); a failed reset destroys the instance rather than recycling it
/// (S4).
#[derive(Debug, Clone, Copy)]
pub struct Exclusive;

impl sealed::Sealed for Exclusive {}
impl CapMarker for Exclusive {
    const PERMITS: Option<usize> = Some(1);
    const RELEASE_REQUIRED: bool = true;
}

/// A bounded resource: one long-lived runtime, capped short-lived leases.
///
/// Folds `Service` / `Transport` / `Exclusive` into a single trait whose
/// release shape is type-enforced through [`Cap`](Bounded::Cap).
///
/// # Acquire bounds
///
/// The `BoundedRuntime` acquire path requires (the widest of the three
/// folded runtimes' bounds — the former exclusive runtime's):
/// - `R: Bounded + Clone + Send + Sync + 'static`
/// - `R::Runtime: Clone + Send + Sync + 'static`
/// - `R::Lease: Send + 'static`
///
/// `Resident` is intentionally **not** folded here — its `Lease: Clone`
/// super-bound and create-vs-rotate epoch reconcile are incompatible with
/// this fold (see the topology-collapse plan, Key Technical Decisions).
///
/// # Type-enforced release shape
///
/// `Cap` selects the concurrency permit count and whether a release hook
/// runs. The release hook lives on the [`BoundedRelease`] sub-trait, which
/// has **no default `release_one`**. A blanket impl supplies the (never
/// called) no-op *only* for [`Unbounded`]-cap resources, so a Cloned-mode
/// service writes no release boilerplate — but a [`Capped<N>`] or
/// [`Exclusive`] resource is **not** covered by that blanket and must
/// implement `BoundedRelease` itself. Forgetting it is therefore a
/// *compile* error (the runtime requires `R: BoundedRelease`), not the
/// silent run-time no-op the old `TOKEN_MODE ==` discriminant produced.
/// This is the sealed-typestate idiom: invalid cap/release combinations
/// are unrepresentable.
pub trait Bounded: Resource {
    /// The concurrency cap typestate.
    ///
    /// One of [`Unbounded`], [`Capped<N>`], or [`Exclusive`]. This
    /// associated type *is* the topology choice — there is no runtime
    /// discriminant.
    type Cap: CapMarker;

    /// Acquires one lease from the running bounded runtime.
    ///
    /// Maps to the folded acquire op:
    /// - Service `Cloned`/`Tracked` → mint a token,
    /// - Transport → open a session,
    /// - Exclusive → hand out the (semaphore-guarded) runtime lease.
    ///
    /// The runtime acquires the [`Cap`](Self::Cap) permit (if any) *before*
    /// calling this, so an implementation never has to size its own
    /// concurrency.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the lease cannot be produced.
    fn acquire_one(
        &self,
        runtime: &Self::Runtime,
        ctx: &ResourceContext,
    ) -> impl Future<Output = Result<Self::Lease, Self::Error>> + Send;
}

/// The release surface of a [`Bounded`] resource.
///
/// Split out of [`Bounded`] so the release hook can be **type-required**
/// for the release-bearing caps. There is deliberately *no* default
/// `release_one`: the only no-op implementation is the blanket below,
/// gated on `Cap = Unbounded`. A [`Capped<N>`] / [`Exclusive`] resource
/// is not covered by that blanket, so it must supply `release_one`
/// explicitly — omitting it fails to satisfy the `R: BoundedRelease`
/// bound the runtime requires (a compile error), instead of silently
/// no-op-ing a token return / reset at run time.
pub trait BoundedRelease: Bounded {
    /// Releases one lease back to the runtime.
    ///
    /// Maps to the folded release op: Service `release_token`, Transport
    /// `close_session`, Exclusive `reset`. `healthy` is `false` when the
    /// lease was tainted (mirrors the old `Transport::close_session`
    /// healthy flag; ignored by token/reset semantics).
    ///
    /// Invoked by the runtime only when
    /// [`Cap::RELEASE_REQUIRED`](CapMarker::RELEASE_REQUIRED) is `true`;
    /// for [`Unbounded`] the blanket no-op below stands in and is never
    /// called (the handle is owned).
    ///
    /// The runtime **observes the error** (tracing + a release-error
    /// metric) — it is never silently discarded (R17). For
    /// [`Exclusive`] specifically, an `Err` here additionally destroys
    /// the instance instead of handing the half-reset runtime to the
    /// next caller (S4); the permit is still returned so the semaphore
    /// cannot deadlock.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if release-time cleanup fails. The error is
    /// observed by the runtime, not propagated to a caller (the lease
    /// holder is already gone).
    fn release_one(
        &self,
        runtime: &Self::Runtime,
        lease: Self::Lease,
        healthy: bool,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Sends a keepalive on the long-lived runtime to prevent idle
    /// disconnection.
    ///
    /// Folds `Transport::keepalive`. Driven by the runtime on the
    /// configured interval (`Config::keepalive_interval`); the default
    /// is a no-op for caps that do not need it.
    ///
    /// # Errors
    ///
    /// Returns `Self::Error` if the keepalive fails (observed, not fatal).
    fn keepalive(
        &self,
        _runtime: &Self::Runtime,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        async { Ok(()) }
    }
}

/// Blanket no-op release for the [`Unbounded`] cap **only**.
///
/// A Cloned-mode service hands out an owned handle and never runs a
/// release callback, so requiring it to write `release_one` would be
/// pure boilerplate. This impl supplies the (never-invoked) no-op for
/// exactly `Cap = Unbounded`. It does **not** cover [`Capped<N>`] /
/// [`Exclusive`] — those caps differ in the `Bounded::Cap` associated
/// type, so this blanket and an author's explicit `impl BoundedRelease`
/// for a capped/exclusive resource never overlap, and a capped/exclusive
/// resource that omits `BoundedRelease` simply does not implement it
/// (compile error at the runtime's `R: BoundedRelease` bound).
impl<R> BoundedRelease for R
where
    R: Bounded<Cap = Unbounded>,
{
    async fn release_one(
        &self,
        _runtime: &Self::Runtime,
        _lease: Self::Lease,
        _healthy: bool,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Configuration types for the bounded topology.
///
/// One `Config` folds the three former per-topology configs:
/// - `Service` `drain_timeout` → [`Config::drain_timeout`](config::Config::drain_timeout),
/// - `Transport` `max_sessions` is the cap const generic `N`;
///   `keepalive_interval` → [`Config::keepalive_interval`](config::Config::keepalive_interval);
///   `acquire_timeout` → [`Config::acquire_timeout`](config::Config::acquire_timeout),
/// - `Exclusive` `acquire_timeout` → [`Config::acquire_timeout`](config::Config::acquire_timeout).
pub mod config {
    use std::time::Duration;

    /// Bounded runtime configuration.
    ///
    /// The concurrency *bound* itself is the cap typestate (`Capped<N>` /
    /// `Exclusive` / `Unbounded`), not a config field — it cannot be
    /// changed without changing the type, by design.
    #[derive(Debug, Clone)]
    pub struct Config {
        /// Timeout for acquiring the concurrency permit when the cap is
        /// saturated. Ignored by [`Unbounded`](super::Unbounded) (no
        /// permit). Defaults to 30s.
        pub acquire_timeout: Duration,
        /// Interval for sending [`keepalive`](super::BoundedRelease::keepalive)
        /// on the long-lived runtime. `None` disables keepalives.
        pub keepalive_interval: Option<Duration>,
        /// Timeout for draining active leases during shutdown. `None`
        /// means wait indefinitely. (Folds `Service::Config::drain_timeout`.)
        pub drain_timeout: Option<Duration>,
    }

    impl Default for Config {
        fn default() -> Self {
            Self {
                acquire_timeout: Duration::from_secs(30),
                keepalive_interval: None,
                drain_timeout: None,
            }
        }
    }
}
