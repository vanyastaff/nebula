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
//! 1. **Budget** — past [`Operation::max_attempts`](super::Operation::max_attempts)
//!    granted attempts it is refused permanently.
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
        Checkout, OpCx, Unit, UnitHost, cancelled_before_grant, generation_refusal, submit_unit,
    },
    pin::PinSlots,
    strict::{UnitPin, capture_pin, read_credentials, register_grant},
};
use crate::{
    context::ResourceContext,
    error::{Error, ErrorKind},
    hook_guard::DEFAULT_AUTHOR_HOOK_CEILING,
    manager::AcquireLink,
    options::AcquireOptions,
    registry::ManagedHandle as _,
    resource::Provider,
    runtime::{admission::AdmissionGeneration, managed::ManagedResource},
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
pub struct ManagedRow<R: Provider> {
    shared: Arc<RowShared<R>>,
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
    /// per attempt.
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
        }
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
            operation,
            O::EFFECT,
            std::any::type_name::<O>(),
        )
    }
}

impl<R: Provider> RowShared<R> {
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
    pub(super) async fn admit_row(
        &mut self,
        row: &'u RowShared<R>,
        cost: &Cost,
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
        let mut slot = slot;
        let (options_ref, ctx) = (&options, &row.ctx);
        let dispatch = || {
            let slot = slot.take();
            let hook_timeout = deadline
                .saturating_duration_since(tokio::time::Instant::now())
                .min(DEFAULT_AUTHOR_HOOK_CEILING);
            async move {
                let guard = row
                    .link
                    .dispatch_checkout(managed, ctx, options_ref, hook_timeout)
                    .await?;
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
        Ok((Checkout::new(guard), pin))
    }
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
