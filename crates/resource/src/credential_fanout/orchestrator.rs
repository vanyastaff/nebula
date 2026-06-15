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
    /// **Per-resource timeout isolation.** Each row's
    /// `refresh_slot_for_identity` is independently wrapped in
    /// `tokio::time::timeout(per_resource_timeout, …)` and all are driven
    /// concurrently via [`futures::future::join_all`]. One slow, failed, or
    /// timed-out row therefore **never aborts or fails a sibling** — every
    /// row's outcome is recorded independently and folded into the returned
    /// [`RotationOutcome`] (`success + failed + timed_out == affected_rows`).
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
    #[tracing::instrument(
        level = "debug",
        name = "nebula.credential.rotation.fanout_refresh",
        skip(self, mgr),
        fields(credential_id = %cid, affected, success, failed, timed_out)
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
    #[tracing::instrument(
        level = "debug",
        name = "nebula.credential.rotation.fanout_revoke",
        skip(self, mgr),
        fields(credential_id = %cid, affected, success, failed, timed_out)
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
    /// Snapshots `affected(cid)`, then for **each** row independently wraps
    /// the matching slot-identity-pinned `Manager` port call in
    /// `tokio::time::timeout(per_resource_timeout, …)` and drives them all
    /// concurrently via [`futures::future::join_all`]. This
    /// independent per-future timeout + `join_all` is exactly what
    /// guarantees the timeout-isolation invariant: a slow, failed,
    /// or timed-out row's future resolves on its own and cannot abort or
    /// fail a sibling — every row's outcome is recorded independently.
    ///
    /// **Revoke is two-phase and cancellation-safe.**
    /// `Manager::revoke_slot_for_identity` is *not* called inside the
    /// timeout: a Rust `async fn` body is lazy, so a timeout future dropped
    /// before its first poll would skip the synchronous taint and leave new
    /// acquires accepted on a credential whose revoke "timed out". Instead
    /// the synchronous `Manager::taint_slot_for_identity` runs **first,
    /// outside and before** the `tokio::time::timeout` (the taint is fully
    /// applied the instant it returns), and **only** the cancellation-safe
    /// `Manager::drain_and_revoke` tail is wrapped in the per-resource
    /// timeout. A timed-out (or otherwise dropped) drain tail therefore
    /// leaves the row tainted — recorded `timed_out`, never silently
    /// un-revoked. A failed *taint* (resolution miss / shutting down) is the
    /// row's terminal outcome (`failed`); the drain tail is then not entered.
    /// Refresh has no pre-`await` state mutation (the engine already stored
    /// the fresh material before this is called), so it stays a single
    /// timeout-wrapped `refresh_slot_for_identity` call.
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
                    // Refresh has no pre-`await` state mutation, so the whole
                    // call is safe to wrap in the per-resource timeout.
                    let refresh = mgr.refresh_slot_for_identity(
                        &b.resource_key,
                        b.scope.clone(),
                        &b.slot_name,
                        &b.slot_identity,
                    );
                    match tokio::time::timeout(per_resource_timeout, refresh).await {
                        Ok(Ok(())) => RowOutcome::Success,
                        Ok(Err(err)) => {
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
                            RowOutcome::Failed
                        },
                        Err(_elapsed) => {
                            tracing::warn!(
                                credential_id = %cid,
                                resource_key = %b.resource_key,
                                slot = %b.slot_name,
                                slot_identity = ?b.slot_identity,
                                timeout_ms = per_resource_timeout.as_millis() as u64,
                                "rotation fan-out: per-resource refresh timed out; \
                                 siblings unaffected",
                            );
                            RowOutcome::TimedOut
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
                            return RowOutcome::Failed;
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
                        crate::RevokeTail::Done => RowOutcome::Success,
                        crate::RevokeTail::HookFailed(err) => {
                            tracing::warn!(
                                credential_id = %cid,
                                resource_key = %b.resource_key,
                                slot = %b.slot_name,
                                slot_identity = ?b.slot_identity,
                                error = %err,
                                "rotation fan-out: per-resource revoke hook failed \
                                 (row stays tainted); siblings unaffected",
                            );
                            RowOutcome::Failed
                        },
                        crate::RevokeTail::HookTimedOut => {
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
                            RowOutcome::TimedOut
                        },
                    }
                },
            }
        });

        let results = futures::future::join_all(dispatches).await;

        let mut outcome = RotationOutcome::default();
        for r in results {
            match r {
                RowOutcome::Success => outcome.success += 1,
                RowOutcome::Failed => outcome.failed += 1,
                RowOutcome::TimedOut => outcome.timed_out += 1,
            }
        }

        tracing::Span::current().record("success", outcome.success);
        tracing::Span::current().record("failed", outcome.failed);
        tracing::Span::current().record("timed_out", outcome.timed_out);
        tracing::debug!(
            credential_id = %cid,
            affected,
            success = outcome.success,
            failed = outcome.failed,
            timed_out = outcome.timed_out,
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
    /// synchronous `Manager::taint_slot_for_identity` outside the timeout,
    /// then the timeout-wrapped cancellation-safe `Manager::drain_and_revoke`
    /// tail.
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
    Success,
    Failed,
    TimedOut,
}
