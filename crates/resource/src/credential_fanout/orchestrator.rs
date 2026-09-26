//! Fan-out dispatch methods for [`ResourceFanoutIndex`].
//!
//! Splits the dispatch orchestration — `dispatch_refresh`, `dispatch_revoke`,
//! and the shared `dispatch` skeleton — out of `index.rs` so that the
//! reverse-index data structure and its lookup/bind/unbind operations stay
//! in one file and the Manager-calling orchestration lives here.
//!
//! [`FanoutOp`] and [`RowOutcome`] are private dispatch details used only
//! within this module.

use std::time::Duration;

use nebula_core::CredentialKey;
use nebula_credential::{Capabilities, CredentialId, CredentialSlotResolver, TenantScope};
use tokio_util::sync::CancellationToken;

use super::index::{ResourceFanoutIndex, RevokeAdmissionClaim, RotationOutcome};

impl ResourceFanoutIndex {
    /// Applies the synchronous phase of a revoke before the driver returns to
    /// polling either event bus. Slow drain and hook work is left in the
    /// pending-admission ledger for a background task.
    pub(crate) fn prepare_revoke(
        &self,
        cid: CredentialId,
        mgr: &crate::Manager,
        retain_terminal_credential_revoke: bool,
    ) -> RotationOutcome {
        let has_staged_binding = self.has_staged_binding(&cid);
        let mut summary = RotationOutcome::default();
        let Some((tainted, failed)) = mgr.fence_published_credential_bindings(
            self,
            cid,
            retain_terminal_credential_revoke,
            retain_terminal_credential_revoke,
        ) else {
            return summary;
        };
        summary.failed += failed;
        for (binding, tainted) in tainted {
            self.remember_pending_revoke(
                cid,
                binding.resource_key,
                &binding.slot_name,
                tainted.managed_handle(),
            );
        }
        summary.failed += usize::from(has_staged_binding);
        summary
    }

    fn prepare_durable_tombstone(
        &self,
        cid: CredentialId,
        mgr: &crate::Manager,
    ) -> RotationOutcome {
        let has_staged_binding = self.has_staged_binding(&cid);
        let mut summary = RotationOutcome::default();
        let Some((tainted, failed)) =
            mgr.fence_published_credential_bindings(self, cid, true, true)
        else {
            return summary;
        };
        summary.failed += failed;
        for (binding, tainted) in tainted {
            self.remember_pending_revoke(
                cid,
                binding.resource_key,
                &binding.slot_name,
                tainted.managed_handle(),
            );
        }
        summary.failed += usize::from(has_staged_binding);
        summary
    }

    pub(crate) async fn finish_prepared_revoke(
        &self,
        cid: CredentialId,
        mgr: &crate::Manager,
    ) -> RotationOutcome {
        self.retry_pending_revoke_admissions_for(mgr, Some(cid))
            .await
    }

    /// Projects the owner-qualified durable replacement, installs the new
    /// guard, and only then dispatches the resource refresh hook for every
    /// affected row.
    pub async fn dispatch_material_replacement(
        &self,
        cid: CredentialId,
        scope: &TenantScope,
        credential_key: &CredentialKey,
        resolver: &dyn CredentialSlotResolver,
        mgr: &crate::Manager,
    ) -> RotationOutcome {
        let affected = self.affected(&cid);
        let has_published_binding = !affected.is_empty();
        let has_staged_binding = self.has_staged_binding(&cid);
        let dispatches = affected.into_iter().map(|binding| async move {
            let Ok(projection_permit) = self.projection_admission.acquire().await else {
                return RowOutcome::Failed {
                    drain_timed_out: false,
                };
            };
            // Pin the registration before credential I/O. A replacement row
            // with identical routing keys must never receive this result.
            let managed = match mgr.lookup_published_credential_binding(self, &cid, &binding) {
                Ok(managed) => managed,
                Err(_) => {
                    return RowOutcome::Failed {
                        drain_timed_out: false,
                    };
                },
            };
            if managed.is_tainted() {
                let pending = self.pending_revokes().into_iter().find(|entry| {
                    entry.credential_id == cid
                        && entry.slot == binding.slot_name
                        && std::sync::Arc::ptr_eq(&entry.managed, &managed)
                });
                let Some(claim) = pending
                    .as_ref()
                    .and_then(|entry| self.claim_pending_revoke(entry))
                else {
                    return RowOutcome::Success {
                        drain_timed_out: false,
                    };
                };
                drop(projection_permit);
                let retry_key = claim.entry.key.clone();
                let retry_slot = claim.entry.slot.clone();
                let retry_managed = std::sync::Arc::clone(&claim.entry.managed);
                let mut claim = Some(claim);
                let (tail, admission) = mgr
                    .retry_tainted_revoke_admission(
                        &retry_key,
                        &retry_slot,
                        retry_managed,
                        Duration::from_secs(30),
                        || {
                            if let Some(claim) = claim.take() {
                                claim.accepted();
                            }
                        },
                    )
                    .await;
                settle_revoke_claim(claim, admission);
                return revoke_tail_outcome(tail);
            }
            let Some((generation, metadata)) =
                managed.credential_slot_projection(&binding.slot_name)
            else {
                return RowOutcome::Failed {
                    drain_timed_out: false,
                };
            };
            let projection_is_current = metadata.as_ref().is_some_and(|metadata| {
                metadata.credential_id() == cid
                    && metadata.credential_key() == credential_key
                    && metadata.scope().map(TenantScope::owner_id) == Some(scope.owner_id())
            });
            if !(projection_is_current || generation == 0 && metadata.is_none()) {
                return RowOutcome::Failed {
                    drain_timed_out: false,
                };
            }
            project_and_refresh(
                self,
                mgr,
                managed,
                ProjectionTarget {
                    binding: &binding,
                    slot: &binding.slot_name,
                    generation,
                    scope,
                    credential_id: cid,
                    credential_key: credential_key.clone(),
                    install_live: true,
                },
                resolver,
                projection_permit,
            )
            .await
        });
        let outcomes = futures::future::join_all(dispatches).await;
        let mut summary = summarize_row_outcomes(outcomes);
        // A registration that has staged its reverse-index ownership is not
        // routable until the manager publishes it. Keep the authoritative
        // material context for reconciliation even when every already-live
        // row succeeded during this dispatch.
        summary.failed += usize::from(has_staged_binding || !has_published_binding);
        summary
    }

    /// Reconcile live projections against credential-owned durable state.
    /// Events only accelerate this scan; loss, lag and driver restart cannot
    /// permanently strand a guard at an older material epoch.
    pub(crate) async fn reconcile_material(
        &self,
        mgr: &crate::Manager,
        resolver: &dyn CredentialSlotResolver,
        credential_id: Option<CredentialId>,
    ) -> RotationOutcome {
        use futures::FutureExt;

        let mut summary = RotationOutcome::default();
        let mut projections = Vec::new();
        let targeted_refresh = credential_id.is_some();
        for (bound_credential_id, binding, context) in self.published_bindings(credential_id) {
            let Some((credential_scope, credential_key)) = context else {
                if targeted_refresh {
                    projections.push(
                        refresh_binding(
                            self,
                            bound_credential_id,
                            mgr,
                            binding,
                            Duration::from_secs(30),
                        )
                        .boxed(),
                    );
                }
                continue;
            };
            let Ok(managed) =
                mgr.lookup_published_credential_binding(self, &bound_credential_id, &binding)
            else {
                projections.push(
                    async {
                        RowOutcome::Failed {
                            drain_timed_out: false,
                        }
                    }
                    .boxed(),
                );
                continue;
            };
            if managed.is_tainted() {
                continue;
            }
            let Some((generation, metadata)) =
                managed.credential_slot_projection(&binding.slot_name)
            else {
                projections.push(
                    async {
                        RowOutcome::Failed {
                            drain_timed_out: false,
                        }
                    }
                    .boxed(),
                );
                continue;
            };
            let install_live = match metadata {
                Some(metadata)
                    if metadata.credential_id() == bound_credential_id
                        && metadata.credential_key() == &credential_key
                        && metadata.scope() == Some(&credential_scope) =>
                {
                    true
                },
                Some(_) => {
                    projections.push(
                        async {
                            RowOutcome::Failed {
                                drain_timed_out: false,
                            }
                        }
                        .boxed(),
                    );
                    continue;
                },
                None => generation == 0,
            };
            projections.push(
                async move {
                    let Ok(projection_permit) = self.projection_admission.acquire().await else {
                        return RowOutcome::Failed {
                            drain_timed_out: false,
                        };
                    };
                    project_and_refresh(
                        self,
                        mgr,
                        managed,
                        ProjectionTarget {
                            binding: &binding,
                            slot: &binding.slot_name,
                            generation,
                            scope: &credential_scope,
                            credential_id: bound_credential_id,
                            credential_key,
                            install_live,
                        },
                        resolver,
                        projection_permit,
                    )
                    .await
                }
                .boxed(),
            );
        }
        let outcomes = futures::future::join_all(projections).await;
        summary.add(summarize_row_outcomes(outcomes));
        if credential_id.is_none() {
            let retries = self.pending_material_contexts().into_iter().map(
                |(cid, scope, credential_key, context_sequence)| async move {
                    let outcome = self
                        .dispatch_material_replacement(cid, &scope, &credential_key, resolver, mgr)
                        .await;
                    if outcome.all_hooks_settled_successfully() {
                        self.complete_material_context(&cid, context_sequence, mgr);
                    }
                    outcome
                },
            );
            for outcome in futures::future::join_all(retries).await {
                summary.add(outcome);
            }
        }
        summary
    }

    pub(crate) async fn retry_pending_revoke_admissions(
        &self,
        mgr: &crate::Manager,
    ) -> RotationOutcome {
        self.retry_pending_revoke_admissions_for(mgr, None).await
    }

    async fn retry_pending_revoke_admissions_for(
        &self,
        mgr: &crate::Manager,
        credential_id: Option<CredentialId>,
    ) -> RotationOutcome {
        // Claim the whole eligible batch before awaiting any drain. Claims
        // exclude event delivery and other reconciliation passes, while
        // join_all gives every independently bounded row immediate progress.
        let claims = self
            .pending_revokes()
            .into_iter()
            .filter(|pending| {
                credential_id.is_none_or(|credential_id| pending.credential_id == credential_id)
            })
            .filter_map(|pending| self.claim_pending_revoke(&pending))
            .collect::<Vec<_>>();
        let tails = futures::future::join_all(claims.into_iter().map(|claim| async move {
            let retry_key = claim.entry.key.clone();
            let retry_slot = claim.entry.slot.clone();
            let retry_managed = std::sync::Arc::clone(&claim.entry.managed);
            let mut claim = Some(claim);
            let (tail, admission) = mgr
                .retry_tainted_revoke_admission(
                    &retry_key,
                    &retry_slot,
                    retry_managed,
                    Duration::from_secs(30),
                    || {
                        if let Some(claim) = claim.take() {
                            claim.accepted();
                        }
                    },
                )
                .await;
            settle_revoke_claim(claim, admission);
            revoke_tail_outcome(tail)
        }))
        .await;
        let mut summary = RotationOutcome::default();
        for tail in tails {
            summary.add(summarize_row_outcomes([tail]));
        }
        summary
    }

    /// Fans a completed credential refresh out to every resource registry
    /// row that resolved `cid`, calling
    /// an ownership-qualified [`Manager`](crate::Manager) refresh port per
    /// row.
    ///
    /// The engine (exec layer) owns rotation orchestration: it has already
    /// resolved and stored the fresh credential material before this is
    /// called; this method only translates the single rotation signal into
    /// the typed per-row resource port, and the resource layer never reaches
    /// back.
    ///
    /// **Per-resource timeout isolation.** All rows are driven concurrently
    /// via [`futures::future::join_all`]. Each manager port applies separate
    /// post-admission observation and queue-owned hook-execution budgets, so
    /// one slow, failed, timed-out, deferred, or abandoned row never aborts a
    /// sibling. Every row contributes exactly one dispatch count:
    /// `success + failed + timed_out + deferred + abandoned == affected_rows`.
    ///
    /// Identity routing: a multi-tenant `(key, scope)` has more than one
    /// resolved row, so `Manager::refresh_slot` (identity-agnostic) would
    /// fail closed with `Ambiguous`. This drives the slot-identity-pinned
    /// an exact published binding with the `slot_identity` recorded at
    /// [`bind`](ResourceFanoutIndex::bind) time so the rotation reaches exactly
    /// the resolved row and cannot cross into a replacement owner.
    ///
    /// Redaction: only the aggregate counts and per-row key / slot / scope /
    /// `slot_identity` / duration reach spans — never credential or secret
    /// material. The returned aggregate is a metrics/dashboard signal, **not**
    /// an audit record; the caller still owns any audit write.
    ///
    /// An empty `affected(cid)` returns
    /// [`RotationOutcome::default()`](RotationOutcome) (a no-op fan-out).
    ///
    /// # Cancel safety
    ///
    /// Dropping this future stops fan-out admission for rows that were not
    /// yet submitted. Hooks already admitted remain queue-owned and reach
    /// exactly one terminal metric/event; cancellation only loses this
    /// call's aggregate [`RotationOutcome`], not admitted execution.
    #[tracing::instrument(
        level = "debug",
        name = "nebula.credential.rotation.fanout_refresh",
        skip(self, mgr),
        fields(credential_id = %cid, affected, success, failed, timed_out, deferred, abandoned, drain_timed_out, observation_timed_out)
    )]
    pub async fn dispatch_refresh(
        &self,
        cid: CredentialId,
        mgr: &crate::Manager,
        per_resource_timeout: Duration,
    ) -> RotationOutcome {
        self.dispatch(cid, mgr, per_resource_timeout, FanoutOp::Refresh)
            .await
    }

    /// Fans a credential revoke (e.g. a lease revoke) out to every resource
    /// registry row that resolved `cid`, calling
    /// an ownership-qualified [`Manager`](crate::Manager) revoke port per row.
    ///
    /// Same per-resource timeout isolation, identity routing, redaction, and
    /// "aggregate is not an audit record" contract as
    /// [`dispatch_refresh`](Self::dispatch_refresh) — only the per-row port
    /// differs (exact-binding taint → drain → revoke hook).
    ///
    /// # Cancel safety
    ///
    /// A revoke row that already synchronously tainted its resource remains
    /// tainted. Hooks admitted before cancellation remain queue-owned and
    /// reach exactly one terminal metric/event; hooks not yet admitted do not
    /// run. Cancellation only prevents construction of the aggregate
    /// [`RotationOutcome`].
    #[tracing::instrument(
        level = "debug",
        name = "nebula.credential.rotation.fanout_revoke",
        skip(self, mgr),
        fields(credential_id = %cid, affected, success, failed, timed_out, deferred, abandoned, drain_timed_out, observation_timed_out)
    )]
    pub async fn dispatch_revoke(
        &self,
        cid: CredentialId,
        mgr: &crate::Manager,
        per_resource_timeout: Duration,
    ) -> RotationOutcome {
        let has_staged_binding = self.has_staged_binding(&cid);
        let mut outcome = self
            .dispatch(cid, mgr, per_resource_timeout, FanoutOp::Revoke)
            .await;
        outcome.failed += usize::from(has_staged_binding);
        outcome
    }

    /// Shared fan-out skeleton for [`dispatch_refresh`](Self::dispatch_refresh)
    /// and [`dispatch_revoke`](Self::dispatch_revoke).
    ///
    /// Snapshots `affected(cid)`, then drives every slot-identity-pinned
    /// `Manager` port concurrently via [`futures::future::join_all`]. Each
    /// port owns its per-resource budget: observation expiry returns typed
    /// deferral only for admitted work that has not started, without dropping
    /// queue ownership. Once started, a hook reaches its independently bounded
    /// terminal result. A slow, failed, deferred, or timed-out row therefore
    /// cannot abort a sibling.
    ///
    /// **Revoke is two-phase and cancellation-safe.**
    /// `Manager::revoke_slot_for_identity` is *not* called inside the
    /// deadline: a Rust `async fn` body is lazy, so a future dropped
    /// before its first poll would skip the synchronous taint and leave new
    /// acquires accepted on a credential whose revoke "timed out". Instead
    /// the synchronous `Manager::taint_slot_for_identity` runs **first,
    /// outside and before** the awaited drain/hook tail. A cancelled drain
    /// therefore leaves the row tainted. After hook admission, the queue owns
    /// execution and terminal observability even if this fan-out is dropped.
    /// A failed taint (resolution miss / shutting down) is the row's terminal
    /// outcome (`failed`); the drain tail is then not entered.
    ///
    /// Each row's `Bind` is moved into its own dispatch future (the snapshot
    /// from [`affected`](ResourceFanoutIndex::affected) is already an owned `Vec`, so no
    /// clone is added over the snapshot), keeping every future self-contained
    /// without `unsafe` lifetime juggling. Only key / slot / scope /
    /// `slot_identity` / duration / counts are logged — never credential
    /// material.
    async fn dispatch(
        &self,
        cid: CredentialId,
        mgr: &crate::Manager,
        per_resource_timeout: Duration,
        op: FanoutOp,
    ) -> RotationOutcome {
        let rows = self.affected(&cid);
        let affected = rows.len();
        tracing::Span::current().record("affected", affected);
        if rows.is_empty() {
            return RotationOutcome::default();
        }

        let op_name = op.as_str();
        let dispatches = rows.into_iter().map(|b| async move {
            match op {
                FanoutOp::Refresh => {
                    refresh_binding(self, cid, mgr, b, per_resource_timeout).await
                },
                FanoutOp::Revoke => {
                    // Phase 1 — SYNCHRONOUS taint, OUTSIDE the timeout. It is
                    // fully applied before `taint_slot_for` returns, so a
                    // subsequently-dropped timeout on the drain tail can
                    // never skip it. A taint failure
                    // (resolution miss / manager shutting down) is this
                    // row's terminal outcome — the drain tail is not entered.
                    let tainted = match mgr.taint_published_credential_binding(self, &cid, &b) {
                        Ok(t) => t,
                        Err(err) => {
                            tracing::warn!(
                                credential_id = %cid,
                                resource_key = %b.resource_key,
                                slot = %b.slot_name,
                                slot_identity = ?b.slot_identity,
                                error = %err,
                                "rotation fan-out: per-resource revoke taint failed; \
                                 siblings unaffected",
                            );
                            return RowOutcome::Failed {
                                drain_timed_out: false,
                            };
                        },
                    };
                    let managed = tainted.managed_handle();
                    let Some(claim) =
                        self.claim_revoke_admission(cid, &b.slot_name, &managed)
                    else {
                        return RowOutcome::Success {
                            drain_timed_out: false,
                        };
                    };
                    // Phase 2 — the cancellation-safe drain + revoke hook.
                    // `drain_and_revoke` is the SINGLE owner of the
                    // per-resource budget: it bounds the drain (best-effort
                    // — a timed-out drain still runs the hook) and the hook
                    // itself (a wedged hook is the only thing the budget
                    // cuts). It is therefore called WITHOUT an outer
                    // `tokio::time::timeout` wrapper — that wrapper used to
                    // be able to elapse on a slow drain and drop the whole
                    // future *before the hook ran*, silently skipping the
                    // documented "hook still runs after a timed-out drain"
                    // guarantee. The row is already tainted (phase 1); every
                    // tail outcome leaves it tainted.
                    let mut claim = Some(claim);
                    let (tail, admission) = mgr
                        .drain_and_revoke_with_admission(tainted, per_resource_timeout, || {
                            if let Some(claim) = claim.take() {
                                claim.accepted();
                            }
                        })
                        .await;
                    settle_revoke_claim(claim, admission);
                    match tail {
                        crate::RevokeTail::Done { drain } => RowOutcome::Success {
                            drain_timed_out: matches!(
                                drain,
                                crate::SlotDrainOutcome::TimedOut { .. }
                            ),
                        },
                        crate::RevokeTail::HookFailed { error, drain } => {
                            tracing::warn!(
                                credential_id = %cid,
                                resource_key = %b.resource_key,
                                slot = %b.slot_name,
                                slot_identity = ?b.slot_identity,
                                error = %error,
                                "rotation fan-out: per-resource revoke hook failed \
                                 (row stays tainted); siblings unaffected",
                            );
                            RowOutcome::Failed {
                                drain_timed_out: matches!(
                                    drain,
                                    crate::SlotDrainOutcome::TimedOut { .. }
                                ),
                            }
                        },
                        crate::RevokeTail::HookTimedOut { drain } => {
                            tracing::warn!(
                                credential_id = %cid,
                                resource_key = %b.resource_key,
                                slot = %b.slot_name,
                                slot_identity = ?b.slot_identity,
                                timeout_ms = per_resource_timeout.as_millis() as u64,
                                "rotation fan-out: per-resource revoke hook timed out \
                                 (drain already completed or also timed out; row stays \
                                 tainted, no new leases); siblings unaffected",
                            );
                            RowOutcome::TimedOut {
                                drain_timed_out: matches!(
                                    drain,
                                    crate::SlotDrainOutcome::TimedOut { .. }
                                ),
                            }
                        },
                        crate::RevokeTail::Deferred { drain, reason } => {
                            if let crate::SlotDrainOutcome::TimedOut {
                                outstanding_leases,
                            } = drain
                            {
                                tracing::warn!(
                                    credential_id = %cid,
                                    resource_key = %b.resource_key,
                                    slot = %b.slot_name,
                                    slot_identity = ?b.slot_identity,
                                    outstanding_leases,
                                    "rotation fan-out: revoke hook deferred after the per-resource drain timed out"
                                );
                            }
                            RowOutcome::Deferred {
                                drain_timed_out: matches!(
                                    drain,
                                    crate::SlotDrainOutcome::TimedOut { .. }
                                ),
                                observation_timed_out: matches!(
                                    reason,
                                    crate::SlotDeferralReason::ObservationTimedOut
                                ),
                            }
                        },
                        crate::RevokeTail::Abandoned { drain } => RowOutcome::Abandoned {
                            drain_timed_out: matches!(
                                drain,
                                crate::SlotDrainOutcome::TimedOut { .. }
                            ),
                        },
                    }
                },
            }
        });

        let results = futures::future::join_all(dispatches).await;

        let outcome = summarize_row_outcomes(results);

        tracing::Span::current().record("success", outcome.success);
        tracing::Span::current().record("failed", outcome.failed);
        tracing::Span::current().record("timed_out", outcome.timed_out);
        tracing::Span::current().record("deferred", outcome.deferred);
        tracing::Span::current().record("abandoned", outcome.abandoned);
        tracing::Span::current().record("drain_timed_out", outcome.drain_timed_out);
        tracing::Span::current().record("observation_timed_out", outcome.observation_timed_out);
        tracing::debug!(
            credential_id = %cid,
            affected,
            success = outcome.success,
            failed = outcome.failed,
            timed_out = outcome.timed_out,
            deferred = outcome.deferred,
            abandoned = outcome.abandoned,
            drain_timed_out = outcome.drain_timed_out,
            observation_timed_out = outcome.observation_timed_out,
            "rotation fan-out {op_name} complete",
        );
        outcome
    }
}

// Bound projection independently of queue-owned hook execution. Cancelling
// this future also cancels cooperative resolver work through the drop guard.
struct ProjectionTarget<'a> {
    binding: &'a crate::Bind,
    slot: &'a str,
    generation: u64,
    scope: &'a TenantScope,
    credential_id: CredentialId,
    credential_key: CredentialKey,
    install_live: bool,
}

pub(super) struct DeferredHookWake {
    state: std::sync::atomic::AtomicU8,
    notify: std::sync::Arc<tokio::sync::Notify>,
}

impl DeferredHookWake {
    const COMPLETED: u8 = 1;
    const DEFERRED: u8 = 2;

    pub(super) fn new(notify: std::sync::Arc<tokio::sync::Notify>) -> Self {
        Self {
            state: std::sync::atomic::AtomicU8::new(0),
            notify,
        }
    }

    pub(super) fn completed(&self) {
        let previous = self
            .state
            .fetch_or(Self::COMPLETED, std::sync::atomic::Ordering::AcqRel);
        if previous & Self::DEFERRED != 0 {
            self.notify.notify_one();
        }
    }

    pub(super) fn arm_deferred(&self) {
        let previous = self
            .state
            .fetch_or(Self::DEFERRED, std::sync::atomic::Ordering::AcqRel);
        if previous & Self::COMPLETED != 0 {
            self.notify.notify_one();
        }
    }
}

async fn project_and_refresh(
    index: &ResourceFanoutIndex,
    mgr: &crate::Manager,
    managed: std::sync::Arc<dyn crate::registry::ManagedHandle>,
    target: ProjectionTarget<'_>,
    resolver: &dyn CredentialSlotResolver,
    projection_permit: tokio::sync::SemaphorePermit<'_>,
) -> RowOutcome {
    let ProjectionTarget {
        binding,
        slot,
        generation,
        scope,
        credential_id: cid,
        credential_key,
        install_live,
    } = target;
    // Captured before the credential is read: a successful resolve proves it
    // usable and may reopen a suspension, unless one landed after this point.
    let reopen = crate::manager::CredentialGateTicket::new(managed.credential_gate_epoch());
    let cancel = CancellationToken::new();
    let _cancel_on_drop = cancel.clone().drop_guard();
    let guard = match tokio::time::timeout(
        Duration::from_secs(30),
        resolver.resolve_slot(scope, cid, credential_key, Capabilities::empty(), cancel),
    )
    .await
    {
        Ok(Ok(guard)) => guard,
        Ok(Err(nebula_credential::CredentialSlotResolveError::Revoked)) => {
            // This resolver result is an authoritative durable tombstone,
            // unlike an independently revoked lease observation. Fence any
            // concurrent or future publication before handling this row.
            let key = managed.resource_key();
            tracing::warn!(
                credential_id = %cid,
                resource_key = %key,
                slot,
                "durable credential tombstone discovered during material reconciliation; revoking resource slot"
            );
            let pending_managed = std::sync::Arc::clone(&managed);
            let preparation = index.prepare_durable_tombstone(cid, mgr);
            if preparation.failed != 0 {
                return RowOutcome::Failed {
                    drain_timed_out: false,
                };
            }
            let Some(claim) = index.claim_revoke_admission(cid, slot, &pending_managed) else {
                drop(projection_permit);
                return RowOutcome::Success {
                    drain_timed_out: false,
                };
            };
            drop(projection_permit);
            let mut claim = Some(claim);
            let retry_key = key;
            let retry_managed = pending_managed;
            let (tail, admission) = mgr
                .retry_tainted_revoke_admission(
                    &retry_key,
                    slot,
                    retry_managed,
                    Duration::from_secs(30),
                    || {
                        if let Some(claim) = claim.take() {
                            claim.accepted();
                        }
                    },
                )
                .await;
            settle_revoke_claim(claim, admission);
            return revoke_tail_outcome(tail);
        },
        Ok(Err(error)) => {
            tracing::warn!(credential_id = %cid, error = %error,
                "material replacement projection failed");
            return RowOutcome::Failed {
                drain_timed_out: false,
            };
        },
        Err(_) => {
            tracing::warn!(credential_id = %cid, "material replacement projection timed out");
            return RowOutcome::TimedOut {
                drain_timed_out: false,
            };
        },
    };
    if !install_live {
        drop(projection_permit);
        return RowOutcome::Success {
            drain_timed_out: false,
        };
    }
    let key = managed.resource_key();
    let publication_binding = binding.clone();
    let hook_completion_wake =
        std::sync::Arc::new(DeferredHookWake::new(index.material_hook_completion_wake()));
    let terminal_wake = std::sync::Arc::clone(&hook_completion_wake);
    let mut arm_deferred = false;
    let outcome = match mgr
        .install_and_refresh_resolved(
            &key,
            slot,
            managed,
            guard,
            crate::manager::ResolvedAt {
                slot_generation: Some(generation),
                gate_ticket: Some(reopen),
            },
            (
                || {
                    if !index.contains_published_binding(&cid, &publication_binding)
                        || index.terminal_publication_rejected(&cid)
                    {
                        return Err(crate::Error::not_found(&publication_binding.resource_key));
                    }
                    Ok(())
                },
                move || {
                    drop(projection_permit);
                },
                move || terminal_wake.completed(),
            ),
        )
        .await
    {
        Ok(crate::manager::EpochRefreshOutcome::Applied(outcome)) => match outcome {
            crate::SlotDispatchOutcome::Completed { .. } => RowOutcome::Success {
                drain_timed_out: false,
            },
            crate::SlotDispatchOutcome::TimedOut { .. } => RowOutcome::TimedOut {
                drain_timed_out: false,
            },
            crate::SlotDispatchOutcome::Deferred { reason, .. } => {
                arm_deferred = true;
                RowOutcome::Deferred {
                    drain_timed_out: false,
                    observation_timed_out: matches!(
                        reason,
                        crate::SlotDeferralReason::ObservationTimedOut
                    ),
                }
            },
            crate::SlotDispatchOutcome::Abandoned { .. } => RowOutcome::Abandoned {
                drain_timed_out: false,
            },
        },
        Ok(crate::manager::EpochRefreshOutcome::Stale { .. }) => RowOutcome::Success {
            drain_timed_out: false,
        },
        Ok(crate::manager::EpochRefreshOutcome::Pending { .. }) => RowOutcome::Deferred {
            drain_timed_out: false,
            observation_timed_out: false,
        },
        Err(error) => {
            tracing::warn!(credential_id = %cid, error = %error,
                "material replacement installation or hook failed");
            RowOutcome::Failed {
                drain_timed_out: false,
            }
        },
    };
    if arm_deferred {
        hook_completion_wake.arm_deferred();
    }
    outcome
}

fn settle_revoke_claim(claim: Option<RevokeAdmissionClaim<'_>>, admission: Option<bool>) {
    match (claim, admission) {
        (None, Some(true)) => {},
        (Some(claim), Some(true)) => claim.accepted(),
        (Some(claim), Some(false)) => claim.retry(),
        (Some(claim), None) => claim.discard(),
        (None, Some(false) | None) => {},
    }
}

fn revoke_tail_outcome(tail: crate::RevokeTail) -> RowOutcome {
    match tail {
        crate::RevokeTail::Done { drain } => RowOutcome::Success {
            drain_timed_out: matches!(drain, crate::SlotDrainOutcome::TimedOut { .. }),
        },
        crate::RevokeTail::HookFailed { drain, .. } => RowOutcome::Failed {
            drain_timed_out: matches!(drain, crate::SlotDrainOutcome::TimedOut { .. }),
        },
        crate::RevokeTail::HookTimedOut { drain } => RowOutcome::TimedOut {
            drain_timed_out: matches!(drain, crate::SlotDrainOutcome::TimedOut { .. }),
        },
        crate::RevokeTail::Deferred { drain, reason } => RowOutcome::Deferred {
            drain_timed_out: matches!(drain, crate::SlotDrainOutcome::TimedOut { .. }),
            observation_timed_out: matches!(reason, crate::SlotDeferralReason::ObservationTimedOut),
        },
        crate::RevokeTail::Abandoned { drain } => RowOutcome::Abandoned {
            drain_timed_out: matches!(drain, crate::SlotDrainOutcome::TimedOut { .. }),
        },
    }
}

async fn refresh_binding(
    index: &ResourceFanoutIndex,
    credential_id: CredentialId,
    mgr: &crate::Manager,
    binding: crate::Bind,
    timeout: Duration,
) -> RowOutcome {
    match mgr
        .refresh_published_credential_binding(index, &credential_id, &binding, timeout, timeout)
        .await
    {
        Ok(crate::SlotDispatchOutcome::Completed { .. }) => RowOutcome::Success {
            drain_timed_out: false,
        },
        Ok(crate::SlotDispatchOutcome::TimedOut { .. }) => RowOutcome::TimedOut {
            drain_timed_out: false,
        },
        Ok(crate::SlotDispatchOutcome::Deferred { reason, .. }) => RowOutcome::Deferred {
            drain_timed_out: false,
            observation_timed_out: matches!(reason, crate::SlotDeferralReason::ObservationTimedOut),
        },
        Ok(crate::SlotDispatchOutcome::Abandoned { .. }) => RowOutcome::Abandoned {
            drain_timed_out: false,
        },
        Err(error) => {
            tracing::warn!(
                credential_id = %credential_id,
                resource_key = %binding.resource_key,
                slot = %binding.slot_name,
                slot_identity = ?binding.slot_identity,
                error = %error,
                "rotation fan-out: per-resource refresh failed; siblings unaffected",
            );
            RowOutcome::Failed {
                drain_timed_out: false,
            }
        },
    }
}

/// Which typed `Manager` slot port the fan-out drives per row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FanoutOp {
    /// `Manager::refresh_slot_for_identity` — credential rotated, fresh
    /// material already resolved and stored by the engine.
    Refresh,
    /// Credential revoked (e.g. lease revoke). Driven as the two-phase port:
    /// synchronous `Manager::taint_slot_for_identity`, then the
    /// cancellation-safe `Manager::drain_and_revoke` tail with queue-owned
    /// hook settlement after admission.
    Revoke,
}

impl FanoutOp {
    /// Stable label for spans/logs (no credential material).
    fn as_str(self) -> &'static str {
        match self {
            FanoutOp::Refresh => "refresh",
            FanoutOp::Revoke => "revoke",
        }
    }
}

/// Per-row fan-out result (one [`Bind`](super::index::Bind) → exactly one of these).
#[derive(Debug, Clone, Copy)]
enum RowOutcome {
    Success {
        drain_timed_out: bool,
    },
    Failed {
        drain_timed_out: bool,
    },
    TimedOut {
        drain_timed_out: bool,
    },
    Deferred {
        drain_timed_out: bool,
        observation_timed_out: bool,
    },
    Abandoned {
        drain_timed_out: bool,
    },
}

fn summarize_row_outcomes(outcomes: impl IntoIterator<Item = RowOutcome>) -> RotationOutcome {
    let mut summary = RotationOutcome::default();
    for outcome in outcomes {
        match outcome {
            RowOutcome::Success { drain_timed_out } => {
                summary.success += 1;
                summary.drain_timed_out += usize::from(drain_timed_out);
            },
            RowOutcome::Failed { drain_timed_out } => {
                summary.failed += 1;
                summary.drain_timed_out += usize::from(drain_timed_out);
            },
            RowOutcome::TimedOut { drain_timed_out } => {
                summary.timed_out += 1;
                summary.drain_timed_out += usize::from(drain_timed_out);
            },
            RowOutcome::Deferred {
                drain_timed_out,
                observation_timed_out,
            } => {
                summary.deferred += 1;
                summary.drain_timed_out += usize::from(drain_timed_out);
                summary.observation_timed_out += usize::from(observation_timed_out);
            },
            RowOutcome::Abandoned { drain_timed_out } => {
                summary.abandoned += 1;
                summary.drain_timed_out += usize::from(drain_timed_out);
            },
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::{RowOutcome, summarize_row_outcomes};

    #[test]
    fn mixed_completed_and_deferred_rows_are_counted_separately() {
        let outcome = summarize_row_outcomes([
            RowOutcome::Success {
                drain_timed_out: true,
            },
            RowOutcome::Deferred {
                drain_timed_out: true,
                observation_timed_out: true,
            },
            RowOutcome::Abandoned {
                drain_timed_out: true,
            },
        ]);

        assert_eq!(outcome.success, 1);
        assert_eq!(outcome.deferred, 1);
        assert_eq!(outcome.abandoned, 1);
        assert_eq!(outcome.drain_timed_out, 3);
        assert_eq!(outcome.observation_timed_out, 1);
        assert_eq!(outcome.failed, 0);
        assert_eq!(outcome.timed_out, 0);
        assert_eq!(outcome.dispatched(), 3);
    }
}
