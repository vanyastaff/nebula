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

use nebula_credential::CredentialId;

use super::index::{ResourceFanoutIndex, RotationOutcome};

impl ResourceFanoutIndex {
    /// Fans a completed credential refresh out to every resource registry
    /// row that resolved `cid`, calling
    /// [`Manager::refresh_slot_for_identity`](crate::Manager::refresh_slot_for_identity)
    /// per row.
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
    /// `refresh_slot_for_identity` with the `slot_identity` recorded at
    /// [`bind`](ResourceFanoutIndex::bind) time so the rotation reaches exactly the
    /// resolved row.
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
    /// [`Manager::revoke_slot_for_identity`](crate::Manager::revoke_slot_for_identity)
    /// per row.
    ///
    /// Same per-resource timeout isolation, identity routing, redaction, and
    /// "aggregate is not an audit record" contract as
    /// [`dispatch_refresh`](Self::dispatch_refresh) — only the per-row port
    /// differs (`revoke_slot_for_identity` taints → drains → runs the revoke
    /// hook).
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
        self.dispatch(cid, mgr, per_resource_timeout, FanoutOp::Revoke)
            .await
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
                    // Admission is synchronous. The manager starts receipt
                    // observation only after admission. Expiry returns
                    // Deferred only while the hook remains queued; once it
                    // starts, its bounded terminal result wins. Neither path
                    // drops queue ownership or invents a retryable error.
                    let refresh = mgr.refresh_slot_for_identity_with_timeout(
                        &b.resource_key,
                        b.scope.clone(),
                        &b.slot_name,
                        &b.slot_identity,
                        per_resource_timeout,
                        per_resource_timeout,
                    );
                    match refresh.await {
                        Ok(crate::SlotDispatchOutcome::Completed { .. }) => {
                            RowOutcome::Success {
                                drain_timed_out: false,
                            }
                        },
                        Ok(crate::SlotDispatchOutcome::TimedOut { .. }) => {
                            RowOutcome::TimedOut {
                                drain_timed_out: false,
                            }
                        },
                        Ok(crate::SlotDispatchOutcome::Deferred { reason, .. }) => {
                            RowOutcome::Deferred {
                                drain_timed_out: false,
                                observation_timed_out: matches!(
                                    reason,
                                    crate::SlotDeferralReason::ObservationTimedOut
                                ),
                            }
                        },
                        Ok(crate::SlotDispatchOutcome::Abandoned { .. }) => {
                            RowOutcome::Abandoned {
                                drain_timed_out: false,
                            }
                        },
                        Err(err) => {
                            // Resource-crate errors are already
                            // credential-free (key/slot/scope only).
                            tracing::warn!(
                                credential_id = %cid,
                                resource_key = %b.resource_key,
                                slot = %b.slot_name,
                                slot_identity = ?b.slot_identity,
                                error = %err,
                                "rotation fan-out: per-resource refresh failed; \
                                 siblings unaffected",
                            );
                            RowOutcome::Failed {
                                drain_timed_out: false,
                            }
                        },
                    }
                },
                FanoutOp::Revoke => {
                    // Phase 1 — SYNCHRONOUS taint, OUTSIDE the timeout. It is
                    // fully applied before `taint_slot_for` returns, so a
                    // subsequently-dropped timeout on the drain tail can
                    // never skip it. A taint failure
                    // (resolution miss / manager shutting down) is this
                    // row's terminal outcome — the drain tail is not entered.
                    let tainted = match mgr.taint_slot_for_identity(
                        &b.resource_key,
                        b.scope.clone(),
                        &b.slot_name,
                        &b.slot_identity,
                    ) {
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
                    match mgr.drain_and_revoke(tainted, per_resource_timeout).await {
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
