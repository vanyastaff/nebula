//! The per-unit checkout facade ([`ManagedRow`]): a row whose units check
//! out an instance per attempt, only after the attempt's quota and row-gate
//! waits.
//!
//! # Why a row facade
//!
//! A [`Managed`](super::Managed) facade owns one lease for all its units:
//! a pooled connection stays checked out while its units wait for quota
//! (QUOTA-DX.md:41-43). A [`ManagedRow`] holds no lease. Each attempt, in
//! order (never holding `Manager.admission` or any sync lock across an
//! await, and never waiting for quota or the row gate while holding a
//! checkout):
//!
//! 1. **Budget** — past [`Operation::max_attempts`] granted attempts it is
//!    refused permanently.
//! 2. **Row pre-check**, lock-free — a tainted row is `Revoked`; a
//!    shutting-down manager, a removed or replaced row `Cancelled`; the
//!    unit's admission generation closed maps as a lease's does; a
//!    suspended row is `CredentialUnavailable`; a row whose phase refuses
//!    acquires is `Backpressure`.
//! 3. **Quota**, nothing held — the attempt's [`Cost`] is booked on the
//!    row's limit (a [`Cost::FREE`] attempt only waits out a pause), raced
//!    against the unit's generation, the manager's shutdown and
//!    [`Unit::cancel`](super::Unit::cancel).
//! 4. **Row gate** — one permit per checkout, FIFO, sized to the topology's
//!    capacity: attempts queue here instead of failing on a full pool.
//!    Still full at the unit's deadline: `Backpressure`.
//! 5. **Credential read R1**, outside every lock — on a strict manager and
//!    a row with bound slots (zero reads otherwise).
//! 6. **Pin** — a unit's slots are pinned on its first attempt, after R1.
//! 7. **Lock #1 and dispatch** — the acquire pipeline's own admission
//!    ([`AcquireLink::acquire_admitted`]): the in-flight count, the taint
//!    and shutdown re-check, R1 applied, suspension, phase and the capture
//!    of the checkout's admission generation under `Manager.admission`, the
//!    recovery gate, then the checkout itself outside every lock, bounded by
//!    the unit's deadline, and the hand-out check.
//! 8. **Credential read R2** — only for a checkout that *created* its
//!    instance on a strict row: the instance was built after R1, so a
//!    join-next re-read decides; an idle hit is served by R1.
//! 9. **Lock #2 and grant** — on a strict row, under `Manager.admission`:
//!    taint, shutdown, R2 applied, suspension, the checkout's generation
//!    and the unit's pin (`Rebinding` when a rotation superseded it). A row
//!    that read nothing grants lock-free once the checkout's generation is
//!    open.
//!
//! Every refusal is `NotSent` and forfeits a booked cost; a refused
//! checkout goes back to the pool untainted. A later attempt of the unit
//! repeats every step and may land on another instance; its pin is not
//! retaken.

use std::{fmt, sync::Arc};

use nebula_core::ResourceKey;
use tokio::sync::Semaphore;

use super::{
    Operation,
    cost::Cost,
    error::OpError,
    managed::{
        Checkout, OpCx, Unit, UnitHost, UnitScope, cancelled_before_grant, generation_refusal,
        submit_unit,
    },
    pin::PinSlots,
    session::{SessionCx, SessionFuture, SessionProvider, SessionSpec, Sessioned},
    strict::{UnitPin, capture_pin, read_credentials, register_grant},
};
use crate::{
    context::ResourceContext,
    error::{Error, ErrorKind},
    guard::ResourceGuard,
    hook_guard::DEFAULT_AUTHOR_HOOK_CEILING,
    manager::AcquireLink,
    options::AcquireOptions,
    registry::ManagedHandle as _,
    resource::Provider,
    runtime::{admission::AdmissionGeneration, managed::ManagedResource},
    topology::{PoolProvider, Pooled},
};

/// A registered row turned into a per-unit checkout facade.
///
/// Built by [`Manager::managed_row`](crate::Manager::managed_row). Unlike
/// [`Managed`](super::Managed) it holds no lease: every attempt of a
/// submitted [`Operation`] checks out an instance of its own after its
/// quota and row-gate waits and releases it when the attempt ends, so a
/// unit waiting for its rate limit holds no connection (see the module
/// docs). There is no `Deref` to an instance:
///
/// ```compile_fail
/// use nebula_resource::{Provider, call::ManagedRow};
///
/// fn skip_the_facade<R: Provider>(row: &ManagedRow<R>) -> &R::Instance {
///     &**row
/// }
/// ```
///
/// Bound to one registration: once that row is removed or replaced, its
/// units fail `Cancelled`. Clones share the row.
///
/// Bound to the caller that built it too: its units inherit the caller
/// context's cancellation (a unit whose first attempt was not granted yet
/// is cancelled `NotSent`; a granted one runs on to its deadline) and, when
/// the caller has one, its deadline, which bounds every unit's deadline.
pub struct ManagedRow<R: Provider> {
    shared: Arc<RowShared<R>>,
    scope: UnitScope,
}

/// What every unit of a row facade shares.
pub(super) struct RowShared<R: Provider> {
    pub(super) managed: Arc<ManagedResource<R>>,
    pub(super) link: AcquireLink,
    pub(super) key: ResourceKey,
    /// The context each checkout's create and prepare hooks see.
    ctx: ResourceContext,
    /// The row gate; `None` for a topology with no capacity.
    gate: Option<Arc<Semaphore>>,
}

impl<R: Provider> Clone for ManagedRow<R> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            scope: self.scope.clone(),
        }
    }
}

impl<R: Provider> fmt::Debug for ManagedRow<R> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedRow")
            .field("resource_key", &self.shared.key)
            .finish_non_exhaustive()
    }
}

impl<R: Provider> ManagedRow<R> {
    /// A facade over `managed`, latching the row's rate-limit profile to
    /// per attempt. Its units inherit nothing until
    /// [`with_unit_scope`](Self::with_unit_scope).
    pub(crate) fn new(
        managed: Arc<ManagedResource<R>>,
        link: AcquireLink,
        ctx: &ResourceContext,
    ) -> Self {
        managed.rate_limiter.latch_per_attempt();
        let gate = managed.row_gate();
        Self {
            shared: Arc::new(RowShared {
                managed,
                link,
                key: R::key(),
                ctx: ctx.clone_for_acquire(),
                gate,
            }),
            scope: UnitScope::default(),
        }
    }

    /// The facade whose units inherit `scope`: the building caller's
    /// cancellation and deadline.
    pub(crate) fn with_unit_scope(mut self, scope: UnitScope) -> Self {
        self.scope = scope;
        self
    }

    /// The row's key.
    #[must_use]
    pub fn resource_key(&self) -> &ResourceKey {
        &self.shared.key
    }
}

impl<R: Provider + PinSlots> ManagedRow<R> {
    /// Submits `operation` as one unit of work; each of its attempts checks
    /// out an instance of its own (see the module docs).
    ///
    /// The unit is lazy: nothing happens until it is first polled. Its first
    /// poll hands the operation to the runtime, which runs it under the
    /// unit's deadline and settles it. Dropping the [`Unit`] before its first
    /// poll means the operation never ran; dropping it later only stops
    /// waiting.
    pub fn submit<O: Operation<R>>(&self, operation: O) -> Unit<O::Output> {
        submit_unit(
            UnitHost::Row(Arc::clone(&self.shared)),
            &self.scope,
            operation,
            O::EFFECT,
            std::any::type_name::<O>(),
        )
    }
}

impl<R> ManagedRow<R>
where
    R: SessionProvider + PoolProvider + Provider<Topology = Pooled<R>> + Clone,
{
    /// Runs `body` as one session: one unit, one attempt, on one checked-out
    /// connection.
    ///
    /// The attempt books `spec`'s cost once, before the checkout, and is
    /// admitted like any row attempt (see the module docs); calls the body
    /// makes on the session are native and not counted. For a
    /// [`SessionBinding::Connection`](super::SessionBinding::Connection)
    /// provider the checkout must be built at the credential slot epoch the
    /// unit pinned: an older idle instance is destroyed and replaced. Then
    /// [`SessionProvider::open`] opens the session, `body` runs with it
    /// borrowed, and [`SessionProvider::close`] commits when the body
    /// succeeded and rolls back otherwise. A session is never retried by
    /// the runtime.
    ///
    /// | Body | Close | Unit | Sent | Instance |
    /// |---|---|---|---|---|
    /// | `Ok(t)` | `Committed` | `Ok(t)` | `Sent` | recycled |
    /// | `Ok` | `RolledBack { refused: Some(e) }` | `Err(e)` | `NotSent` | recycled |
    /// | `Err(e)` | `RolledBack` | `Err(e)` | `NotSent` | recycled |
    /// | any | `Unknown(e)` | `Err(e)` | `MaybeSent` | destroyed |
    ///
    /// Open failing is `NotSent`, destroys the instance and never calls the
    /// body. A deadline or a panic mid-session is `MaybeSent` and destroys
    /// the instance; for a [`Write`](super::Effect::Write) that is an
    /// unknown outcome. [`SessionCx::closing`] is cooperative: a granted
    /// session is never aborted. A session (or any unit) of the same row
    /// awaited inside a session body is refused `Permanent` — it would wait
    /// for the row gate while this session holds a checkout.
    pub fn session<T, F>(&self, spec: SessionSpec, body: F) -> Unit<T>
    where
        T: Send + 'static,
        F: for<'c, 's> FnOnce(&'s mut R::Session<'c>, &'s SessionCx) -> SessionFuture<'s, T>
            + Send
            + 'static,
    {
        let effect = spec.effect();
        submit_unit(
            UnitHost::Row(Arc::clone(&self.shared)),
            &self.scope,
            Sessioned::new(spec, body),
            effect,
            "session",
        )
    }
}

impl<R: Provider> RowShared<R> {
    /// The row's identity in the nested-session marker.
    pub(super) fn marker(&self) -> usize {
        Arc::as_ptr(&self.managed).addr()
    }

    /// The admission generation a new unit of the row runs under: the row's
    /// current one, or the refusal of a row that has none.
    pub(super) fn unit_generation(&self) -> Result<Arc<AdmissionGeneration>, OpError> {
        if let Some(generation) = self.managed.admission.current() {
            return Ok(generation);
        }
        self.precheck_row()?;
        Err(OpError::new(
            ErrorKind::Cancelled,
            "resource removed; new attempts refused",
        ))
    }

    /// Step 2 of a row attempt: the lock-free pre-check (see the module
    /// docs), against the unit's `generation`.
    pub(super) fn precheck(&self, generation: &AdmissionGeneration) -> Result<(), OpError> {
        generation_refusal(&self.managed, generation)?;
        self.precheck_row()
    }

    /// The pre-check's row-wide part: shutdown, removal, suspension, phase.
    fn precheck_row(&self) -> Result<(), OpError> {
        if self.managed.is_tainted() {
            return Err(OpError::new(
                ErrorKind::Revoked,
                "resource tainted by a credential revoke; new attempts refused",
            ));
        }
        if self.link.admission().shutdown_guard().is_err() {
            return Err(OpError::new(
                ErrorKind::Cancelled,
                "manager shutting down; attempt refused",
            ));
        }
        if self.managed.store.is_closed() {
            return Err(OpError::new(
                ErrorKind::Cancelled,
                "resource removed or replaced; new attempts refused",
            ));
        }
        if let Some(suspension) = self.managed.admission.suspension() {
            return Err(OpError::new(
                ErrorKind::CredentialUnavailable {
                    reason: suspension.reason(),
                },
                "bound credential unavailable; new attempts refused",
            ));
        }
        if !self.managed.phase().is_accepting() {
            return Err(OpError::new(
                ErrorKind::Backpressure,
                "resource cannot accept work in its current phase",
            ));
        }
        Ok(())
    }
}

/// Completes when `cancel` fires; never without one.
async fn cancelled(cancel: Option<&tokio_util::sync::CancellationToken>) {
    match cancel {
        Some(cancel) => cancel.cancelled().await,
        None => std::future::pending().await,
    }
}

impl<'u, R: Provider + PinSlots> OpCx<'u, R> {
    /// Steps 1–9 of a row attempt (see the module docs); on a grant, the
    /// attempt's checkout and the unit's pin.
    ///
    /// `fit` (a connection-bound session's) rejects a checkout built at
    /// another credential slot epoch than the unit pinned: an idle one is
    /// destroyed and the checkout retried until the deadline, a created one
    /// refuses the attempt `Rebinding`.
    pub(super) async fn admit_row(
        &mut self,
        row: &'u RowShared<R>,
        cost: &Cost,
        fit: Option<CheckoutFit<R>>,
    ) -> Result<(Checkout<R>, &UnitPin<R::Pinned>), OpError> {
        // 1–3: budget, the row pre-check and the quota, nothing held.
        self.admit_local(cost, Some(row)).await?;
        let generation = self.generation;
        let managed = &row.managed;
        let cancel = self.shared.cancel_before_grant();
        let shutdown = row.link.admission().cancel();

        // 4. The row gate, still holding nothing.
        let slot = match &row.gate {
            None => None,
            Some(gate) => Some(tokio::select! {
                biased;
                () = cancelled(cancel) => return Err(cancelled_before_grant()),
                () = generation.token().cancelled() => return Err(closed_refusal(row, generation)),
                () = shutdown.cancelled() => return Err(closed_refusal(row, generation)),
                permit = Arc::clone(gate).acquire_owned() => permit.map_err(|_closed| {
                    OpError::new(ErrorKind::Cancelled, "row gate closed; attempt refused")
                })?,
                () = tokio::time::sleep_until(self.deadline) => {
                    return Err(OpError::new(
                        ErrorKind::Backpressure,
                        "the row gate stayed full until the unit deadline",
                    ));
                },
            }),
        };

        // 5. R1, outside every lock (zero reads for a row that reads nothing).
        let first = read_credentials(managed, generation, self.deadline, cancel).await?;
        let strict = first.is_some();

        // 6. The unit's pin: taken afresh until an attempt is granted on it,
        //    kept after — a dropped attempt future loses nothing.
        if self.shared.attempts() == 0 || self.pin.is_none() {
            self.pin = Some(capture_pin(managed));
        }

        // 7. Lock #1, the recovery gate, the checkout and the hand-out check,
        //    raced against the unit's cancel only: a generation that closes
        //    meanwhile is the hand-out check's to refuse, so a created
        //    instance still reaches the pool. The gate permit rides on the
        //    checkout from the moment it exists.
        let options = AcquireOptions::default().with_deadline(self.deadline.into_std());
        let deadline = self.deadline;
        let pinned_epoch = self.pin.as_ref().map_or_else(
            || managed.resource.credential_slot_epoch(),
            UnitPin::slot_epoch,
        );
        let mut slot = slot;
        let (options_ref, ctx) = (&options, &row.ctx);
        let dispatch = || {
            let slot = slot.take();
            async move {
                let guard = loop {
                    let hook_timeout = deadline
                        .saturating_duration_since(tokio::time::Instant::now())
                        .min(DEFAULT_AUTHOR_HOOK_CEILING);
                    let mut guard = row
                        .link
                        .dispatch_checkout(managed, ctx, options_ref, hook_timeout)
                        .await?;
                    match fit {
                        Some(fits) if !fits(&guard, pinned_epoch) => {
                            evict_unfit(&mut guard, deadline)?;
                            // Wait for the eviction, so its topology permit
                            // is back before the next checkout.
                            let _outcome = guard.release().await;
                        },
                        _ => break guard,
                    }
                };
                Ok(match slot {
                    Some(slot) => guard.with_row_slot(slot),
                    None => guard,
                })
            }
        };
        let admitted = tokio::select! {
            biased;
            () = cancelled(cancel) => Err(cancelled_before_grant()),
            admitted = row.link.acquire_admitted(
                Arc::clone(managed),
                ctx,
                options_ref,
                first.as_ref(),
                std::time::Instant::now(),
                dispatch,
            ) => admitted.map_err(checkout_refusal),
        };
        let guard = admitted?;

        // 8. R2: a created instance was built after R1.
        let second = if strict && guard.created() {
            read_credentials(managed, generation, self.deadline, cancel).await?
        } else {
            None
        };

        // 9. Lock #2 (strict rows) and the grant. A refusal drops the
        //    checkout untainted: the instance goes back to the pool.
        let pin = self.pin.as_ref().ok_or_else(|| {
            OpError::new(ErrorKind::Permanent, "unit pin missing at registration")
        })?;
        register_grant(
            managed,
            strict,
            second.as_ref(),
            guard.admission(),
            pin,
            self.shared,
        )?;
        if let Some(metrics) = row.link.metrics() {
            metrics.record_row_checkout(guard.created());
        }
        tracing::debug!(
            resource.key = %row.key,
            created = guard.created(),
            "managed row attempt granted a checkout"
        );
        Ok((Checkout::new(guard), pin))
    }
}

/// Whether a checkout may host the attempt, given the credential slot epoch
/// its unit pinned (a connection-bound session's check).
pub(super) type CheckoutFit<R> = fn(&ResourceGuard<R>, u64) -> bool;

/// Taints a checkout that does not fit the unit's pin so its release
/// destroys it. A created instance that does not fit means the pin itself
/// was superseded: `Rebinding`. Past the unit's deadline: `Backpressure`.
fn evict_unfit<R: Provider>(
    guard: &mut ResourceGuard<R>,
    deadline: tokio::time::Instant,
) -> Result<(), Error> {
    guard.taint();
    if guard.created() {
        tracing::debug!(
            resource.key = %R::key(),
            "a fresh instance is newer than the unit's pinned credentials; attempt refused"
        );
        return Err(crate::Manager::credential_unavailable_error(
            &R::key(),
            crate::error::CredentialUnavailableReason::Rebinding,
        ));
    }
    tracing::debug!(
        resource.key = %R::key(),
        "idle instance built on superseded credentials evicted"
    );
    if tokio::time::Instant::now() >= deadline {
        return Err(Error::backpressure(
            "no instance at the unit's pinned credentials before the deadline",
        ));
    }
    Ok(())
}

/// The refusal of a row attempt whose checkout failed: the acquire
/// pipeline's kind, unsent.
fn checkout_refusal(error: Error) -> OpError {
    let detail = match error.kind() {
        ErrorKind::Backpressure => "row checkout refused: no capacity or not accepting",
        ErrorKind::Cancelled => "row checkout refused: row closing",
        ErrorKind::Revoked => "row checkout refused: credential revoked",
        ErrorKind::CredentialUnavailable { .. } => "row checkout refused: credential unavailable",
        _ => "row checkout failed",
    };
    OpError::new(error.kind().clone(), detail)
}

/// The refusal of a row attempt whose wait ended because the unit's
/// generation closed or the manager began shutting down.
fn closed_refusal<R: Provider>(row: &RowShared<R>, generation: &AdmissionGeneration) -> OpError {
    row.precheck(generation)
        .err()
        .unwrap_or_else(|| OpError::new(ErrorKind::Cancelled, "row closing; attempt refused"))
}

#[cfg(test)]
#[path = "../call_row_tests.rs"]
mod tests;
