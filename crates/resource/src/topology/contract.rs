//! Open [`Topology`] trait — the lease/concurrency/policy contract.
//!
//! A [`Topology`] describes how [`Instance`](crate::resource::Provider::Instance)s
//! are *leased* to callers: the concurrency gate ([`try_reserve`] → [`Ticket`]),
//! the acquire→guard→release mechanics ([`acquire`] / [`on_release`]), and a
//! read-only availability surface ([`phase`] / [`load`]).
//!
//! The structural rule that makes the open trait safe: a `Topology` is handed a
//! lifetime-bound `&InstanceStore<Self::Slot>` it cannot retain past a call.
//! It therefore cannot build a cross-scope instance cache that bypasses the
//! per-tenant [`SlotIdentity`] fence.
//!
//! [`try_reserve`]: Topology::try_reserve
//! [`acquire`]: Topology::acquire
//! [`on_release`]: Topology::on_release
//! [`phase`]: Topology::phase
//! [`load`]: Topology::load
//! [`SlotIdentity`]: crate::dedup::SlotIdentity
//! [`InstanceStore`]: crate::topology::store::InstanceStore

use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::OwnedSemaphorePermit;

use crate::{
    error::{Error, ErrorKind},
    topology::store::InstanceStore,
};

// ─── AdmissionPhase ──────────────────────────────────────────────────────────

/// Admission phase snapshot for a topology's resource instances.
///
/// Orthogonal to the lifecycle [`ResourcePhase`](crate::state::ResourcePhase):
/// a resource can be `Active` (lifecycle) while `Warming` (admission) until
/// its first connection completes. The authoritative admission gate is always
/// [`Topology::try_reserve`] — this value is advisory only (diagnostics,
/// load-balancer hints, backoff scheduling).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdmissionPhase {
    /// Instances are available for leasing.
    Ready,
    /// All capacity is in use. `try_reserve` returns
    /// [`Unavailable::Saturated`].
    ///
    /// Advisory only: do not gate admission on this value. The authoritative
    /// gate is `try_reserve`.
    Saturated,
    /// The topology is warming up (cold-start, index build, not yet ready to
    /// serve). `try_reserve` returns [`Unavailable::Warming`].
    Warming,
    /// The topology is mid-reconnect / mid-reset / rebalancing.
    /// `try_reserve` returns [`Unavailable::Recovering`].
    Recovering,
    /// Credentials revoked or instances poisoned.
    /// `try_reserve` returns [`Unavailable::Tainted`].
    Tainted,
}

// ─── Load ─────────────────────────────────────────────────────────────────────

/// Optional load snapshot from a topology.
///
/// Minimal placeholder; a future release replaces this with a richer
/// `Load { saturation: f32, est_wait: Option<Duration>, detail: LoadDetail }`
/// shaped after tower `Load`.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct Load {
    /// Saturation in `0.0..=1.0`. `1.0` = fully saturated.
    pub saturation: f32,
}

impl Load {
    /// Constructs a `Load` from a `(used, total)` permit count.
    pub fn permits(used: usize, total: usize) -> Self {
        let saturation = if total == 0 {
            0.0
        } else {
            (used as f32) / (total as f32)
        };
        Self { saturation }
    }
}

// ─── Unavailable ──────────────────────────────────────────────────────────────

/// Typed reason a [`Topology`]'s `try_reserve` could not grant a ticket.
///
/// The variant drives the engine's park/reschedule/fail decision:
/// - [`Saturated`]: capacity full now — park and reschedule after `retry_after`.
/// - [`Warming`]: not yet query-ready (cold-start, index build) — park until
///   next phase-change event.
/// - [`Recovering`]: mid-reconnect / mid-reset / rebalancing.
/// - [`Tainted`]: credentials revoked or instance poisoned — fail or route to
///   recovery-gate.
///
/// [`Saturated`]: Unavailable::Saturated
/// [`Warming`]: Unavailable::Warming
/// [`Recovering`]: Unavailable::Recovering
/// [`Tainted`]: Unavailable::Tainted
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Unavailable {
    /// All capacity is in use right now. Park and retry after `retry_after`
    /// if specified, or on the next `try_reserve` cycle if `None`.
    Saturated {
        /// Suggested delay before retrying, if known.
        retry_after: Option<Duration>,
    },
    /// The topology's instances are not yet ready to serve requests
    /// (cold start, warmup, index build). Retry when the resource moves
    /// to [`AdmissionPhase::Ready`].
    Warming,
    /// The topology is mid-reconnect / mid-reset / rebalancing. Retry when
    /// the resource moves to [`AdmissionPhase::Ready`].
    Recovering,
    /// Credentials revoked or instances poisoned. Route to recovery or fail.
    Tainted,
}

impl Unavailable {
    /// Maps this [`Unavailable`] variant onto the resource [`Error`] the
    /// manager returns to callers when `try_reserve` rejects an acquire.
    ///
    /// Mapping:
    /// - [`Saturated`](Self::Saturated) → `Error::backpressure` (`Backpressure` — retryable with a
    ///   short delay; `retry_after` is passed through when present).
    /// - [`Warming`](Self::Warming) → `Error::transient` (Transient — retry on next
    ///   phase-change event).
    /// - [`Recovering`](Self::Recovering) → `Error::transient` (Transient — retry after
    ///   reconnect).
    /// - [`Tainted`](Self::Tainted) → `Error::revoked` (Revoked/Unavailable — route to
    ///   recovery or fail with a short backoff).
    pub fn into_error(self, context: impl std::fmt::Display) -> Error {
        match self {
            Self::Saturated { retry_after } => {
                if let Some(after) = retry_after {
                    // Topology supplied an explicit retry hint — use Exhausted
                    // so the hint is preserved verbatim rather than overridden
                    // by Backpressure's fixed 50 ms default.
                    Error::new(
                        ErrorKind::Exhausted {
                            retry_after: Some(after),
                        },
                        format!("{context}: topology saturated — retry after {after:?}"),
                    )
                } else {
                    Error::backpressure(format!(
                        "{context}: topology saturated — all capacity in use"
                    ))
                }
            },
            Self::Warming => Error::transient(format!(
                "{context}: topology warming up — not yet ready to serve"
            )),
            Self::Recovering => Error::transient(format!(
                "{context}: topology recovering — mid-reconnect or rebalancing"
            )),
            Self::Tainted => Error::revoked(format!(
                "{context}: topology tainted — credentials revoked or instances poisoned"
            )),
        }
    }
}

// ─── Ticket ───────────────────────────────────────────────────────────────────

/// A held reservation returned by [`Topology::try_reserve`].
///
/// A `Ticket` IS the reservation: holding it guarantees a slot will be
/// available when `acquire` is called. Dropping a `Ticket` without calling
/// `acquire` releases the reservation.
///
/// Variants:
/// - **Permit-bearing**: carries an [`OwnedSemaphorePermit`] from a
///   `tokio::Semaphore`. The permit IS the capacity gate — dropping it returns
///   one permit to the pool.
/// - **Infallible**: zero-cost (no capacity constraint, e.g. Resident).
///   Dropping it is a no-op.
/// - **Slot-bearing**: carries a pre-checked-out slot from [`InstanceStore`].
///   Dropping without calling `acquire` returns the slot.
pub struct Ticket<S> {
    kind: TicketKind<S>,
}

enum TicketKind<S> {
    /// Semaphore permit; dropping it returns capacity.
    Permit(OwnedSemaphorePermit),
    /// Semaphore permit + pre-checked-out slot; acquire consumes both.
    PermitWithSlot {
        permit: OwnedSemaphorePermit,
        slot: S,
        checkout_epoch: u64,
    },
    /// Zero-cost ticket (Resident / unbounded topologies).
    Infallible,
    /// Pre-checked-out slot from `InstanceStore` without a semaphore permit.
    Slot { slot: S, checkout_epoch: u64 },
}

impl<S> Ticket<S> {
    /// Creates a permit-bearing ticket (capacity-gated topologies).
    pub fn permit(permit: OwnedSemaphorePermit) -> Self {
        Self {
            kind: TicketKind::Permit(permit),
        }
    }

    /// Creates a permit + pre-checked-out slot ticket.
    pub fn permit_with_slot(permit: OwnedSemaphorePermit, slot: S, checkout_epoch: u64) -> Self {
        Self {
            kind: TicketKind::PermitWithSlot {
                permit,
                slot,
                checkout_epoch,
            },
        }
    }

    /// Creates a zero-cost ticket (Resident / unbounded).
    pub fn infallible() -> Self {
        Self {
            kind: TicketKind::Infallible,
        }
    }

    /// Creates a slot-bearing ticket without a semaphore permit.
    pub fn slot(slot: S, checkout_epoch: u64) -> Self {
        Self {
            kind: TicketKind::Slot {
                slot,
                checkout_epoch,
            },
        }
    }

    /// Consumes the ticket, returning any pre-checked-out slot and permit.
    ///
    /// Authors call this inside [`Topology::acquire`] to extract the held
    /// capacity token (if any) and any pre-checked-out slot the
    /// [`try_reserve`](Topology::try_reserve) path embedded in the ticket.
    ///
    /// Returns `(None, None)` for infallible (zero-cost) tickets.
    pub fn take_slot(self) -> (Option<(S, u64)>, Option<OwnedSemaphorePermit>) {
        match self.kind {
            TicketKind::Permit(p) => (None, Some(p)),
            TicketKind::PermitWithSlot {
                permit,
                slot,
                checkout_epoch,
            } => (Some((slot, checkout_epoch)), Some(permit)),
            TicketKind::Infallible => (None, None),
            TicketKind::Slot {
                slot,
                checkout_epoch,
            } => (Some((slot, checkout_epoch)), None),
        }
    }
}

// ─── Lease ────────────────────────────────────────────────────────────────────

/// A leased slot produced by [`Topology::acquire`].
///
/// The `Manager` wraps a `Lease` in a `ResourceGuard` that `Deref`s to
/// `R::Instance`. On guard drop the framework runs `on_release(slot)`, then
/// either returns the slot to `InstanceStore` (under the revoke-epoch fence)
/// or evicts and destroys it.
pub struct Lease<S> {
    /// The leased slot.
    pub slot: S,
    /// The epoch at which this slot was checked out (used during release).
    pub checkout_epoch: u64,
    /// Permit held for the duration of the lease; dropped on release.
    pub permit: Option<OwnedSemaphorePermit>,
}

impl<S> Lease<S> {
    /// Creates a lease directly.
    pub fn new(slot: S, checkout_epoch: u64, permit: Option<OwnedSemaphorePermit>) -> Self {
        Self {
            slot,
            checkout_epoch,
            permit,
        }
    }

    /// Creates a lease from a [`Ticket`] that already carried the slot,
    /// or from `default_slot`/`default_epoch` for a freshly-created instance.
    pub fn from_ticket(ticket: Ticket<S>, default_slot: S, default_epoch: u64) -> Self {
        let (slot_and_epoch, permit) = ticket.take_slot();
        let (slot, checkout_epoch) = slot_and_epoch.unwrap_or((default_slot, default_epoch));
        Self {
            slot,
            checkout_epoch,
            permit,
        }
    }
}

// ─── Topology trait ───────────────────────────────────────────────────────────

/// Author-facing lease policy for a resource's instances.
///
/// A `Topology` describes *how* already-built, already-authorized instances
/// are leased to callers under concurrency. The framework controls everything
/// else: credential resolution, revoke-epoch fence, rotation fan-out, drain,
/// metrics, spans.
///
/// # Storage safety
///
/// Every method receives a `&InstanceStore<Self::Slot>` reference. The
/// lifetime of that borrow does not exceed the call. A `Topology`
/// implementation therefore **cannot** retain the store, copy an `Arc` of it,
/// or build a static/host-keyed cache that bypasses the per-tenant
/// `SlotIdentity` fence. Cross-tenant runtime bleed is prevented by API shape,
/// not by author discipline.
///
/// # Async dispatch
///
/// `#[async_trait]` is used for `acquire` and `on_release` because topologies
/// are reached **monomorphically** inside `ManagedResource<R>` — the one
/// `Box<dyn Future>` per call is negligible next to the I/O the methods do.
/// Sync methods (`try_reserve`, `phase`, `load`) stay plain sync.
///
/// # Example — minimal permit-only topology
///
/// ```rust,no_run
/// use std::sync::Arc;
/// use tokio::sync::Semaphore;
/// use nebula_resource::topology::{
///     Topology, Ticket, Lease, Unavailable, AdmissionPhase,
///     store::InstanceStore,
/// };
/// use nebula_resource::error::Error;
///
/// pub struct FfmpegPool { sem: Arc<Semaphore>, cap: usize }
///
/// #[async_trait::async_trait]
/// impl Topology for FfmpegPool {
///     type Slot = ();
///
///     fn try_reserve(&self, _s: &InstanceStore<()>) -> Result<Ticket<()>, Unavailable> {
///         self.sem.clone().try_acquire_owned()
///             .map(Ticket::permit)
///             .map_err(|_| Unavailable::Saturated { retry_after: None })
///     }
///     async fn acquire(&self, ticket: Ticket<()>, _s: &InstanceStore<()>) -> Result<Lease<()>, Error> {
///         Ok(Lease::from_ticket(ticket, (), 0))
///     }
///     fn phase(&self, _s: &InstanceStore<()>) -> AdmissionPhase {
///         if self.sem.available_permits() == 0 { AdmissionPhase::Saturated } else { AdmissionPhase::Ready }
///     }
/// }
/// ```
#[async_trait]
pub trait Topology: Send + Sync + 'static {
    /// The leasable unit the framework stores.
    ///
    /// - Pooled: one connection handle.
    /// - Resident: the shared handle.
    /// - Permit-only: `()` (no stored instance; a semaphore permit IS the lease).
    type Slot: Send + Sync + 'static;

    // ── concurrency gate ────────────────────────────────────────────────────

    /// Non-blocking. Returns a permit-bearing [`Ticket`] or a typed
    /// [`Unavailable`].
    ///
    /// The `Ticket` IS the reservation — holding it guarantees a slot is
    /// available when `acquire` is called; dropping it releases the
    /// reservation. This resolves the check-then-acquire TOCTOU: success
    /// hands back a held capacity token, not a readiness boolean.
    ///
    /// Implementations may call [`InstanceStore::checkout`] to pop an idle
    /// slot and embed it in the ticket via [`Ticket::permit_with_slot`].
    fn try_reserve(
        &self,
        store: &InstanceStore<Self::Slot>,
    ) -> Result<Ticket<Self::Slot>, Unavailable>;

    /// Async, fallible. Consumes a [`Ticket`] and produces the leased slot.
    ///
    /// May do I/O (channel-open, post-checkout `SET search_path`, etc.).
    /// On error the framework propagates the error to the caller; the
    /// ticket's permit (if any) is dropped, releasing capacity.
    ///
    /// The framework wraps the returned [`Lease`] in a `ResourceGuard` whose
    /// release schedules `on_release`.
    async fn acquire(
        &self,
        ticket: Ticket<Self::Slot>,
        store: &InstanceStore<Self::Slot>,
    ) -> Result<Lease<Self::Slot>, Error>;

    /// Async, fallible, ordered-before-reissue.
    ///
    /// Reset the slot to a clean baseline (rollback txn, reset PRAGMAs,
    /// UNSUBSCRIBE, cancel transients).
    ///
    /// - `Ok(())` → the framework runs the revoke-epoch fence on the store,
    ///   then recycles the slot.
    /// - `Err(_)` → the framework evicts the slot (destroy, do not return).
    ///
    /// The default implementation is a no-op (suitable for permit-only or
    /// stateless topologies).
    async fn on_release(&self, _slot: &mut Self::Slot) -> Result<(), Error> {
        Ok(())
    }

    // ── availability surface ────────────────────────────────────────────────

    /// Returns the current admission phase.
    ///
    /// Advisory only — do not gate admission on this value. The authoritative
    /// gate is `try_reserve`.
    fn phase(&self, _store: &InstanceStore<Self::Slot>) -> AdmissionPhase {
        AdmissionPhase::Ready
    }

    /// Returns an optional load snapshot.
    fn load(&self, _store: &InstanceStore<Self::Slot>) -> Option<Load> {
        None
    }
}
