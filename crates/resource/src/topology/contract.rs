//! Open [`Topology`] trait — the slot-centric, framework-driven lease contract.
//!
//! A [`Topology<R>`] describes how [`Instance`](crate::resource::Provider::Instance)s
//! are *leased* to callers, but it does **not** own the acquire loop. The
//! framework owns the loop: it runs `try_reserve` (the sync concurrency gate),
//! the fenced [`InstanceStore::checkout`](crate::topology::store::InstanceStore::checkout),
//! the stale-slot destroy, the create-or-accept decision, the cancel-safe
//! guard-wrap, and the on-release return-or-destroy. The topology supplies only
//! thin, R-aware policy hooks (`create_slot` / `slot_instance` / `into_instance`
//! / `accept` / `prepare` / `on_release` / …) that it **cannot** use to skip the
//! credential-revoke fence.
//!
//! This is the inversion the open trait exists for: a custom topology author
//! writes zero `store.checkout()` / `resource.destroy()` / stale-loop /
//! epoch-compare code. The fence is framework-owned for **every** topology —
//! built-in and custom alike — by construction, not by author discipline.
//!
//! # Storage safety
//!
//! Every store-bearing method receives a lifetime-bound
//! `&InstanceStore<Self::Slot>` it cannot retain past the call. It therefore
//! cannot build a cross-scope instance cache that bypasses the per-tenant
//! [`SlotIdentity`] fence. Cross-tenant runtime bleed is prevented by API shape,
//! not author discipline.
//!
//! [`SlotIdentity`]: crate::dedup::SlotIdentity
//! [`InstanceStore`]: crate::topology::store::InstanceStore

use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::OwnedSemaphorePermit;

use crate::{
    context::ResourceContext,
    error::{Error, ErrorKind},
    resource::Provider,
    topology::store::InstanceStore,
    topology_tag::TopologyTag,
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

// ─── AdmissionStatus ────────────────────────────────────────────────────────

/// Diagnostic admission snapshot for a registered resource: its advisory
/// [`AdmissionPhase`] plus an optional [`Load`]. Returned by
/// [`Manager::admission_status`](crate::Manager::admission_status).
///
/// Advisory only — the authoritative admission gate is always
/// [`Topology::try_reserve`]. Use this for admin APIs, dashboards, and
/// load-balancer hints, never to gate admission.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct AdmissionStatus {
    /// Advisory admission phase.
    pub phase: AdmissionPhase,
    /// Optional load snapshot — `None` for topologies that do not report load
    /// (e.g. resident / permit-less).
    pub load: Option<Load>,
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

/// A held concurrency reservation returned by [`Topology::try_reserve`].
///
/// A `Ticket` IS the reservation: holding it guarantees a concurrency slot is
/// available for the acquire the framework is about to run. Dropping a `Ticket`
/// without leasing releases the reservation (returns the permit). The framework
/// holds the ticket's permit (if any) in the resulting
/// [`ResourceGuard`](crate::guard::ResourceGuard) for the whole lease, so the
/// permit is returned to the topology's semaphore when the guard drops.
///
/// Variants:
/// - **Permit-bearing**: carries an [`OwnedSemaphorePermit`] from a
///   `tokio::Semaphore`. The permit IS the capacity gate — dropping it returns
///   one permit to the pool.
/// - **Infallible**: zero-cost (no capacity constraint, e.g. Resident).
///   Dropping it is a no-op.
pub struct Ticket {
    permit: Option<OwnedSemaphorePermit>,
}

impl Ticket {
    /// Creates a permit-bearing ticket (capacity-gated topologies).
    pub fn permit(permit: OwnedSemaphorePermit) -> Self {
        Self {
            permit: Some(permit),
        }
    }

    /// Creates a zero-cost ticket (Resident / unbounded).
    pub fn infallible() -> Self {
        Self { permit: None }
    }

    /// Consumes the ticket, returning the held capacity permit (if any).
    ///
    /// The framework calls this to move the permit into the
    /// [`ResourceGuard`](crate::guard::ResourceGuard) so it is held for the
    /// whole lease and returned on guard drop.
    pub fn into_permit(self) -> Option<OwnedSemaphorePermit> {
        self.permit
    }
}

// ─── MaintenanceSchedule ──────────────────────────────────────────────────────

/// Background-maintenance cadence + TTLs for a topology that runs a reaper.
///
/// Returned by [`Topology::maintenance_schedule`]: the framework spawns a
/// maintenance reaper task only when a topology returns `Some` here and at
/// least one TTL is configured. The reaper runs the framework store's
/// [`evict_stale`](crate::topology::store::InstanceStore::evict_stale) (revoke
/// fence) plus the per-slot [`Topology::idle_evictable`] predicate
/// (fingerprint / max-lifetime / idle-timeout), destroying evicted slots via
/// [`Topology::into_instance`] → [`Provider::destroy`].
#[derive(Debug, Clone, Copy)]
pub struct MaintenanceSchedule {
    /// Idle-timeout TTL, if configured.
    pub idle_timeout: Option<Duration>,
    /// Max-lifetime TTL, if configured.
    pub max_lifetime: Option<Duration>,
    /// Interval between maintenance sweeps.
    pub maintenance_interval: Duration,
}

// ─── Topology trait ───────────────────────────────────────────────────────────

/// Author-facing, framework-driven lease policy for a resource's instances.
///
/// A `Topology<R>` describes *how* already-built, already-authorized instances
/// are leased to callers under concurrency — but the **framework owns the
/// acquire loop and the credential-revoke fence**. The topology supplies thin
/// R-aware hooks the framework calls *inside* its own loop; it cannot reach the
/// store's fence, retain the store, or skip stale-slot destruction.
///
/// # Slot-centric
///
/// [`Slot`](Topology::Slot) is the leasable unit the framework stores in its
/// [`InstanceStore<Self::Slot>`]. The guard holds the same `Slot` for the whole
/// lease, so per-slot metadata (`created_at` for max-lifetime, `fingerprint`,
/// `checkout_count`) survives the checkout → lease → return round-trip:
///
/// - **Pooled**: `Slot = PoolSlot<R>` (instance + metrics + fingerprint).
/// - **Resident**: `Slot = R::Instance` (the cloned shared handle); `pools() ==
///   false`, so a released clone is dropped, never pooled.
/// - **Permit-only**: a thin id/handle slot, or `()`-shaped.
///
/// # The framework acquire loop (what the topology does NOT write)
///
/// ```text
/// let ticket = topology.try_reserve(&store)?;           // sync gate
/// loop {
///     let checkout = store.checkout().await;            // FRAMEWORK fences on pop
///     for stale in checkout.stale {                     // FRAMEWORK destroys stale
///         resource.destroy(topology.into_instance(stale)).await;
///     }
///     match checkout.fresh {
///         Some(co) => { let (mut slot, epoch) = co.into_parts();
///                       if topology.accept(&mut slot, …).await { break (slot, epoch); }
///                       resource.destroy(topology.into_instance(slot)).await; }
///         None => break (topology.create_slot(…).await?, store.stamp_epoch()),
///     }
/// }
/// // CreateGuard-wrap (cancel-safety) → topology.prepare(&mut slot, …).await? → build guard
/// // Guard Deref = topology.slot_instance(&slot).
/// // On drop: topology.on_release(&mut slot)?; if pools() && kept
/// //          store.return_slot(slot, epoch) else destroy(into_instance(slot)).
/// ```
///
/// A topology that finds itself writing `store.checkout()`, `resource.destroy`,
/// a stale-slot loop, or a revoke epoch-compare is doing the framework's job —
/// that logic belongs in the framework loop, not in a `Topology` hook.
///
/// # Storage safety
///
/// Every store-bearing method receives a `&InstanceStore<Self::Slot>` whose
/// borrow does not exceed the call, so the topology cannot retain the store or
/// build a cross-scope cache that bypasses the per-tenant `SlotIdentity` fence.
///
/// # Async dispatch
///
/// `#[async_trait]` is used because topologies are reached **monomorphically**
/// inside `ManagedResource<R>` — the one `Box<dyn Future>` per call is
/// negligible next to the I/O the hooks do. Sync hooks (`try_reserve`,
/// `slot_instance`, `into_instance`, `phase`, `load`, `pools`, …) stay plain
/// sync.
#[async_trait]
pub trait Topology<R: Provider>: Send + Sync + 'static {
    /// The leasable unit the framework stores and the guard holds.
    ///
    /// - Pooled: `PoolSlot<R>` (one connection handle + metrics).
    /// - Resident: `R::Instance` (the cloned shared handle).
    /// - Permit-only: a thin id/handle, or `()`-shaped.
    type Slot: Send + Sync + 'static;

    // ── concurrency gate ────────────────────────────────────────────────────

    /// Non-blocking concurrency gate. Returns a permit-bearing [`Ticket`] or a
    /// typed [`Unavailable`].
    ///
    /// The `Ticket` IS the reservation — holding it guarantees a concurrency
    /// slot is available for the acquire the framework runs next; the framework
    /// moves the ticket's permit into the resulting guard so it is held for the
    /// whole lease. This resolves the check-then-acquire TOCTOU: success hands
    /// back a held capacity token, not a readiness boolean.
    fn try_reserve(&self, store: &InstanceStore<Self::Slot>) -> Result<Ticket, Unavailable>;

    // ── slot lifecycle (framework-driven) ───────────────────────────────────

    /// Make one fresh, credential-resolved leasable slot.
    ///
    /// Pooled builds `PoolSlot { instance: <R::create>, metrics: now,
    /// fingerprint }`; Resident clones the shared master handle into
    /// `Slot = R::Instance`; a permit pool stores an id/handle. Credentials are
    /// resolved into the resource's slot cells before this runs. The framework
    /// drives it on an idle-miss (during acquire) and during warmup.
    ///
    /// # Errors
    ///
    /// Returns the create/clone error; the framework fails the acquire and
    /// drops the held permit, releasing capacity.
    async fn create_slot(
        &self,
        resource: &R,
        config: &R::Config,
        ctx: &ResourceContext,
    ) -> Result<Self::Slot, Error>;

    /// Project a held slot to its leasable instance — the guard's `Deref`
    /// target. Pooled: `&slot.instance`; Resident: the slot itself.
    fn slot_instance<'s>(&self, slot: &'s Self::Slot) -> &'s R::Instance;

    /// Consume a slot back into its instance for [`Provider::destroy`]
    /// (stale-fenced / accept-rejected / maintenance-evicted / non-pooled
    /// slots). Pooled: `slot.instance`; Resident: identity.
    // guard-justified: `&self` borrows the framework-owned topology while the
    // `slot` argument is consumed — `into_*` names the slot→instance
    // conversion, not a `self`-consuming builder, so wrong_self_convention is a
    // false match here.
    #[allow(
        clippy::wrong_self_convention,
        reason = "topology is borrowed, the slot argument is consumed; the conversion is slot→instance"
    )]
    fn into_instance(&self, slot: Self::Slot) -> R::Instance;

    /// Validate a checked-out idle slot **in place** before it is leased
    /// (Pooled: stale-fingerprint / max-lifetime / `is_broken` /
    /// `test_on_checkout`).
    ///
    /// `false` ⇒ the framework destroys the slot
    /// ([`into_instance`](Topology::into_instance) → [`Provider::destroy`]) and
    /// loops to the next idle slot, then `create_slot`. Default `true` (no
    /// post-checkout policy).
    async fn accept(&self, _slot: &mut Self::Slot, _resource: &R, _ctx: &ResourceContext) -> bool {
        true
    }

    /// Per-acquire session-init on the slot about to be leased (Pooled
    /// `PoolProvider::prepare`, `SET search_path`, …).
    ///
    /// # Errors
    ///
    /// `Err` ⇒ the framework destroys the slot and fails the acquire.
    async fn prepare(
        &self,
        _slot: &mut Self::Slot,
        _resource: &R,
        _ctx: &ResourceContext,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Reset / recycle a released slot before the framework returns it (rollback
    /// txn, reset PRAGMAs, UNSUBSCRIBE, `PoolProvider::recycle`). Runs on the
    /// release path, **before** the framework's revoke-epoch fence.
    ///
    /// - `Ok(true)` → the framework keeps the slot: if [`pools`](Topology::pools)
    ///   it runs the store's revoke-epoch fence and recycles, else it destroys.
    /// - `Ok(false)` → the framework destroys the slot (do not recycle).
    /// - `Err(_)` → the framework destroys the slot.
    ///
    /// The `resource` handle is supplied so a pooling topology can run the
    /// `PoolProvider::recycle` / `is_broken` policy here. The framework already
    /// destroys a *tainted* lease before calling this, so taint is not a case
    /// the hook handles.
    ///
    /// Default `Ok(true)` (no reset; recycle if the topology pools).
    async fn on_release(&self, _slot: &mut Self::Slot, _resource: &R) -> Result<bool, Error> {
        Ok(true)
    }

    /// Whether a released, kept slot returns to the framework idle store.
    ///
    /// Pooled: `true`; Resident / pure-permit: `false` (a released slot is
    /// dropped via `into_instance` → destroy, never pooled). Default `false`.
    fn pools(&self) -> bool {
        false
    }

    /// Whether this **non-pooling** topology tears down its own
    /// credential-bound instances on revoke (instead of relying on the
    /// framework store's revoke-epoch fence, which only reaches *pooled* idle
    /// slots).
    ///
    /// A shared/multiplexed instance (a gRPC channel, a WebSocket, the
    /// `Resident` cell) is held continuously and never enters the idle store,
    /// so the store fence cannot evict it; its revoke teardown must run through
    /// [`dispatch_credential_hook`](Topology::dispatch_credential_hook). A
    /// topology that holds credential-bound state on `pools() == false` MUST
    /// override both this (to `true`) and the hook, or a revoked credential
    /// keeps serving. The framework asserts this at registration.
    ///
    /// Ignored when [`pools`](Topology::pools) is `true` (the store fence
    /// covers pooled slots). Default `false` — a non-pooling topology opts in
    /// only when it genuinely handles its own revoke.
    fn handles_own_revoke(&self) -> bool {
        false
    }

    /// The idle capacity cap the framework applies to this topology's store.
    ///
    /// `Some(n)` caps the framework idle queue at `n` slots (Pooled: `max_size`
    /// — an idle slot beyond the concurrency cap can never be leased, so it is
    /// pure waste); `None` is unbounded (Resident / permit-only topologies whose
    /// store stays empty). Read once at registration to build
    /// `ManagedResource::store`. Default `None`.
    fn store_capacity(&self) -> Option<usize> {
        None
    }

    // ── warmup / maintenance (framework-driven over the store) ──────────────

    /// Idle count the framework pre-warms by calling
    /// [`create_slot`](Topology::create_slot) + depositing into the store
    /// (fenced) at registration. `0` = no warmup. Default `0`.
    fn warmup_target(&self, _config: &R::Config) -> usize {
        0
    }

    /// Predicate for the framework maintenance reaper: should this idle slot be
    /// evicted now (Pooled: stale-fingerprint / max-lifetime / idle-timeout)?
    ///
    /// The framework already evicts revoke-stale slots via the store's
    /// [`evict_stale`](crate::topology::store::InstanceStore::evict_stale); this
    /// predicate covers only the non-revoke arms. Default `false`.
    fn idle_evictable(&self, _slot: &Self::Slot) -> bool {
        false
    }

    /// `Some(schedule)` if the framework should spawn a maintenance reaper for
    /// this topology, `None` = none. Default `None`.
    fn maintenance_schedule(&self) -> Option<MaintenanceSchedule> {
        None
    }

    // ── credential rotation / revoke fence ──────────────────────────────────

    /// Per-slot credential rotation hook, framework-driven over the live store.
    ///
    /// The framework passes the borrowed `&InstanceStore<Self::Slot>` so a
    /// pooling topology can walk its idle slots under the store lock (the same
    /// lock `checkout` / `return_slot` take, so no checkout can interleave
    /// mid-rotation). The borrow does not exceed the call; the topology cannot
    /// retain it.
    ///
    /// `refresh = true` selects `Provider::on_credential_refresh`, `false`
    /// `Provider::on_credential_revoke`. Default no-op: a topology with no
    /// pooled instances has nothing to rotate over the store.
    ///
    /// A multiplexed / shared topology (`pools() == false` holding a
    /// credential-bearing singleton — a gRPC channel, a WebSocket) that is
    /// **not** in the framework store cannot be reached by the store's
    /// revoke-epoch fence, so its revoke teardown MUST run here. The default
    /// no-op leaks streams on revoke for such a topology; the framework emits a
    /// register-time warning when `pools() == false` and the resource declares
    /// ≥1 credential slot.
    ///
    /// # Errors
    ///
    /// Returns the first hook error; the framework surfaces it to the rotation
    /// dispatch caller.
    async fn dispatch_credential_hook(
        &self,
        _resource: &R,
        _store: &InstanceStore<Self::Slot>,
        _slot: &str,
        _refresh: bool,
    ) -> Result<(), Error> {
        Ok(())
    }

    /// Update the config fingerprint so stale idle slots evict on the next
    /// sweep / acquire. Default no-op (topologies that track no fingerprint).
    fn set_fingerprint(&self, _fingerprint: u64) {}

    // ── availability surface ────────────────────────────────────────────────

    /// Returns the current admission phase.
    ///
    /// Advisory only — do not gate admission on this value. The authoritative
    /// gate is [`try_reserve`](Topology::try_reserve).
    fn phase(&self, _store: &InstanceStore<Self::Slot>) -> AdmissionPhase {
        AdmissionPhase::Ready
    }

    /// Returns an optional load snapshot.
    fn load(&self, _store: &InstanceStore<Self::Slot>) -> Option<Load> {
        None
    }

    /// Topology identifier tag for rotation / diagnostic spans.
    ///
    /// The built-in [`Pooled`](crate::topology::Pooled) /
    /// [`Resident`](crate::topology::Resident) topologies override this with
    /// [`TopologyTag::Pool`] / [`TopologyTag::Resident`]; the default
    /// [`TopologyTag::Custom`] labels an author-supplied open topology.
    fn tag(&self) -> TopologyTag {
        TopologyTag::Custom
    }
}

// ─── NoTopology ─────────────────────────────────────────────────────────────

/// A zero-cost no-op topology for resources whose lifecycle is **not** managed
/// by the resource [`Manager`](crate::Manager).
///
/// Some `Provider` types (e.g. engine daemon sources) implement the
/// [`Provider`] trait for its metadata / lifecycle
/// hooks but are never acquired through the resource Manager's lease pipeline —
/// they are owned by a different runtime. Such a resource still has to name a
/// [`Provider::Topology`]; `NoTopology`
/// is the marker that says "this resource is not leased here". Its
/// [`try_reserve`](Topology::try_reserve) always grants an infallible ticket and
/// its `create_slot` always errors, so a `NoTopology` resource is never actually
/// acquired through the lease pipeline — by construction, not by convention.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoTopology;

#[async_trait]
impl<R: Provider> Topology<R> for NoTopology {
    type Slot = R::Instance;

    fn try_reserve(&self, _store: &InstanceStore<R::Instance>) -> Result<Ticket, Unavailable> {
        Ok(Ticket::infallible())
    }

    async fn create_slot(
        &self,
        _resource: &R,
        _config: &R::Config,
        _ctx: &ResourceContext,
    ) -> Result<R::Instance, Error> {
        Err(Error::permanent(
            "NoTopology: this resource is not leased through the resource Manager",
        ))
    }

    fn slot_instance<'s>(&self, slot: &'s R::Instance) -> &'s R::Instance {
        slot
    }

    fn into_instance(&self, slot: R::Instance) -> R::Instance {
        slot
    }
}
