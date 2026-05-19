//! Engine-owned reverse index: `CredentialId` -> affected resource rows.
//!
//! `nebula-engine` (exec layer) owns credential rotation
//! orchestration; `nebula-resource` exposes only the typed
//! `Manager::{refresh_slot_for, revoke_slot_for}` port. When a credential
//! rotates, the engine must fan that single event out to every resource
//! registry row whose resolved slot binding consumed it.
//!
//! This module is the index half of that fan-out. It maps a rotated
//! `CredentialId` to the set of resource rows that bound it, so the
//! orchestrator can drive `Manager::{refresh_slot_for, revoke_slot_for}` per
//! row.
//!
//! # Why the bind struct carries `slot_identity`
//!
//! The resource registry is keyed structurally by
//! `(ResourceKey, ScopeLevel, slot_identity)` — see
//! [`nebula_resource::dedup`] and [`nebula_resource::SlotIdentity`]. Two
//! registrations of the same resource type at the same scope whose
//! resolved credentials differ are *distinct rows* (the multi-tenant
//! anti-bleed barrier). A `Manager::refresh_slot` call against a multi-row
//! `(key, scope)` fails closed (`Ambiguous`) precisely because it cannot pick
//! a row without the resolved identity.
//!
//! The reverse-index entry therefore records the resolved
//! [`SlotIdentity`] alongside
//! `(ResourceKey, ScopeLevel, slot_name)` so a rotation routes to the
//! *specific* resolved registry row rather than the whole `(key, scope)`
//! family. The identity is the **collision-free structural** value
//! (`SlotIdentity`, exact string equality over the canonical-sorted
//! resolved `(slot, credential)` pairs — *not* a collidable digest), so the
//! reverse-index key cannot alias two tenants' rows. This is
//! forward-correctness against the structural dedup model, not extra
//! precision for its own sake.
//!
//! The engine consumes each credential rotation signal and translates it into typed `Manager` port
//! calls; the resource layer never reaches back across the boundary. This index is an in-process,
//! in-memory routing table only — never persisted and never sent across a trust boundary.

use std::time::Duration;

use dashmap::DashMap;
use nebula_core::{ResourceKey, ScopeLevel};
use nebula_credential::CredentialId;
use nebula_resource::SlotIdentity;

/// One resource registry row affected by a credential rotation.
///
/// - `resource_key` / `scope`: the structural address of the registry row.
/// - `slot_name`: the credential slot on that row that resolved the rotated
///   credential.
/// - `slot_identity`: the resolved **collision-free structural** identity
///   ([`SlotIdentity`]); it disambiguates
///   multi-tenant rows that share `(resource_key, scope)` so a rotation
///   routes to exactly the row whose slot resolved to the rotated
///   credential. Equality is exact string equality (no digest), so two
///   distinct resolved binding sets can never alias this reverse-index key.
///
/// [`SlotIdentity::Unbound`](nebula_resource::SlotIdentity) is the
/// `slot_identity` for a row that resolved no credential slots
/// (single-row-per-`(key, scope)` behaviour); such rows still appear here
/// verbatim.
///
/// Fields are named (rather than a positional tuple) so call sites that
/// destructure a bind cannot transpose `resource_key`/`scope`/`slot_name`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bind {
    /// Structural key of the affected resource registry row.
    pub resource_key: ResourceKey,
    /// Lifecycle scope of the affected resource registry row.
    pub scope: ScopeLevel,
    /// Credential slot on the row that resolved the rotated credential.
    pub slot_name: String,
    /// Resolved **collision-free structural** slot identity disambiguating
    /// multi-tenant rows (exact string equality, not a collidable digest).
    pub slot_identity: SlotIdentity,
}

/// Aggregate of a per-slot rotation fan-out across every affected resource
/// registry row.
///
/// One [`Bind`] contributes exactly one of the three counts, so
/// `success + failed + timed_out == affected_rows`./// per-resource timeout-isolation invariant a slow, failed, or timed-out row
/// never aborts or fails its siblings — each row's outcome is independent. The
/// struct carries only counts (no key/slot/credential material) so it is safe
/// to log or emit as a metrics/dashboard signal; it is **not** a substitute
/// for an audit write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RotationOutcome {
    /// Rows whose `Manager::{refresh,revoke}_slot_for` hook returned `Ok`.
    pub success: usize,
    /// Rows whose hook returned `Err` (resolution miss, hook failure, …).
    pub failed: usize,
    /// Rows whose hook did not complete within the per-resource timeout.
    pub timed_out: usize,
}

impl RotationOutcome {
    /// Total rows the fan-out dispatched to
    /// (`success + failed + timed_out`).
    #[must_use]
    pub fn dispatched(&self) -> usize {
        self.success + self.failed + self.timed_out
    }
}

/// One reverse-index row plus the number of live registrations that
/// resolved it.
///
/// Identical resolved rows dedupe to a single fan-out target (one
/// [`Bind`]); `refs` counts how many `register_and_bind` stagings
/// currently depend on it. A failed registration releases exactly one
/// reference; the row is removed only when the last referent is gone, so
/// a failing staging can never delete a row a concurrent successful
/// registration still holds. `refs` is `>= 1` for any present entry (the
/// entry is removed at zero), so a plain `usize` with that invariant is
/// sufficient — no `NonZero` ceremony.
#[derive(Debug, Clone)]
struct BindRef {
    bind: Bind,
    refs: usize,
}

/// Engine-owned reverse index from a rotated `CredentialId` to the resource
/// registry rows that resolved it.
///
/// Concurrency-safe and lock-free for readers via [`DashMap`]; the
/// orchestrator binds rows as resources register and looks them up on a
/// rotation signal. Insert order within a single credential is preserved so
/// fan-out is deterministic for a given registration sequence.
///
/// This is a pure in-process routing table — see the module docs for why it
/// is never persisted or sent across a trust boundary.
#[derive(Debug, Default)]
pub struct ResourceFanoutIndex {
    /// `CredentialId` -> refcounted rows whose resolved slot bound that
    /// credential.
    ///
    /// `nebula-engine` has no direct `smallvec` dependency, so the
    /// per-credential row list is a plain `Vec`. Promoting this to a small
    /// inline buffer is a deferred, dependency-gated optimisation.
    by_credential: DashMap<CredentialId, Vec<BindRef>>,
}

impl ResourceFanoutIndex {
    /// Creates an empty index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that the resource row
    /// `(resource_key, scope, slot_name, slot_identity)` resolved `cid` for
    /// one of its credential slots.
    ///
    /// Re-binding an identical row under the same credential is idempotent
    /// *at the fan-out level* — [`affected`](Self::affected) still returns
    /// one entry and a rotation fans out once — but each call takes a
    /// reference (see [`BindRef`]). The presence check and the
    /// increment/insert both run under the `DashMap` shard lock held by
    /// `entry(cid)`, so a concurrent `bind` / `unbind_staged_entry` for the
    /// same `cid` cannot interleave between them. This closes the
    /// stage-then-roll-back TOCTOU in
    /// [`register_and_bind`](crate::resource::ResourceRegistrarRegistry::register_and_bind):
    /// a failing registration releases only its own reference and can
    /// never delete a row a concurrent successful registration still holds.
    pub fn bind(
        &self,
        cid: CredentialId,
        resource_key: ResourceKey,
        scope: ScopeLevel,
        slot_name: impl Into<String>,
        slot_identity: SlotIdentity,
    ) {
        let entry = Bind {
            resource_key,
            scope,
            slot_name: slot_name.into(),
            slot_identity,
        };
        let mut rows = self.by_credential.entry(cid).or_default();
        match rows.iter_mut().find(|r| r.bind == entry) {
            Some(existing) => existing.refs += 1,
            None => rows.push(BindRef {
                bind: entry,
                refs: 1,
            }),
        }
    }

    /// Returns every resource row that resolved `cid`, in registration order.
    ///
    /// Empty when no row bound the credential — the orchestrator treats that
    /// as a no-op rotation fan-out.
    #[must_use]
    pub fn affected(&self, cid: &CredentialId) -> Vec<Bind> {
        self.by_credential
            .get(cid)
            .map(|rows| rows.iter().map(|r| r.bind.clone()).collect())
            .unwrap_or_default()
    }

    /// Removes every binding under `(resource_key, scope)` across all
    /// `slot_identity` values and all credentials.
    ///
    /// This is unconditional: it does not preserve multi-tenant siblings that
    /// share `(resource_key, scope)` but differ in `slot_identity`. For
    /// row-granular removal that keeps such siblings intact, use
    /// `unbind_resource_identity`.
    pub fn unbind_resource(&self, resource_key: &ResourceKey, scope: &ScopeLevel) {
        self.by_credential.retain(|_, rows| {
            rows.retain(|r| r.bind.resource_key != *resource_key || r.bind.scope != *scope);
            !rows.is_empty()
        });
    }

    /// Drops bindings for the single resolved registry row
    /// `(resource_key, scope, slot_identity)`, leaving multi-tenant siblings
    /// that share `(resource_key, scope)` but differ in `slot_identity`
    /// intact.
    ///
    /// This is the precise inverse of [`bind`](Self::bind) at row
    /// granularity: when one resolved row is removed from a multi-row
    /// `(key, scope)` family, only that row's fan-out entries must go. Kept
    /// alongside [`unbind_resource`](Self::unbind_resource) because the
    /// orchestrator removes a *specific* resolved row on resource removal —
    /// matching the structural dedup model where `(key, scope)` alone is not
    /// a unique row.
    pub fn unbind_resource_identity(
        &self,
        resource_key: &ResourceKey,
        scope: &ScopeLevel,
        slot_identity: &SlotIdentity,
    ) {
        self.by_credential.retain(|_, rows| {
            rows.retain(|r| {
                r.bind.resource_key != *resource_key
                    || r.bind.scope != *scope
                    || r.bind.slot_identity != *slot_identity
            });
            !rows.is_empty()
        });
    }

    /// Removes exactly one `(cid, bind)` tuple — the precise per-entry
    /// inverse of a single [`bind`](Self::bind) call.
    ///
    /// Unlike [`unbind_resource_identity`](Self::unbind_resource_identity)
    /// (which drops *every* credential's binding for a
    /// `(resource_key, scope, slot_identity)` row), this removes only the
    /// one entry under `cid` that structurally equals `bind`, leaving any
    /// other credential's binding for the same resolved row — and any
    /// pre-existing identical binding under a different cid — untouched.
    ///
    /// It is the compensation primitive for the *stage-bind-before-
    /// register-then-roll-back-on-failure* ordering in
    /// [`ResourceRegistrarRegistry::register_and_bind`]: it **releases one
    /// reference** taken by [`bind`](Self::bind) and removes the row only
    /// when the last referent is gone. A registration that fails after
    /// staging therefore drops just its own reference; a concurrent (or
    /// prior) successful registration of the identical resolved row keeps
    /// its reference, so its live fan-out row survives. This makes the
    /// rollback correct without the registrar having to decide "did I
    /// insert this entry?" — a decision that could not be made atomically
    /// with the insert and was the source of the cross-registration
    /// corruption.
    pub(crate) fn unbind_staged_entry(&self, cid: &CredentialId, bind: &Bind) {
        // `remove_if_mut` holds the shard lock across the whole closure:
        // the matching entry is decremented (or removed at the last
        // reference) and the credential bucket is dropped iff it became
        // empty — atomically, with no TOCTOU between the decrement, the
        // emptiness check, and the bucket removal (mirrors the
        // `retain(!is_empty())` discipline of the bulk unbinds). `bind`
        // de-dups into one refcounted entry, so at most one structurally-
        // equal entry exists; an absent `(cid, bind)` is a no-op.
        self.by_credential.remove_if_mut(cid, |_, rows| {
            if let Some(pos) = rows.iter().position(|r| &r.bind == bind) {
                if rows[pos].refs > 1 {
                    rows[pos].refs -= 1;
                } else {
                    rows.remove(pos);
                }
            }
            rows.is_empty()
        });
    }

    /// Fans a completed credential refresh out to every resource registry
    /// row that resolved `cid`, calling
    /// [`Manager::refresh_slot_for_identity`](nebula_resource::Manager::refresh_slot_for_identity)
    /// per row.
    ///
    /// The engine (exec layer, ) owns rotation orchestration: it has
    /// already resolved and stored the fresh credential material before this
    /// is called; this method only translates the single rotation signal
    /// into the typed per-row resource port , and the resource
    /// layer never reaches back.
    ///
    /// **Per-resource timeout isolation .** Each row's
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
    /// [`bind`](Self::bind) time so the rotation reaches exactly the
    /// resolved row.
    ///
    /// Redaction: only the aggregate counts and per-row key / slot / scope /
    /// `slot_identity` (the resolved structural identity) / duration reach
    /// spans — never credential or secret material. The returned aggregate
    /// is a metrics/dashboard signal, **not** an audit record ; the caller
    /// still owns any audit write.
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
        mgr: &nebula_resource::Manager,
        per_resource_timeout: Duration,
    ) -> RotationOutcome {
        self.dispatch(cid, mgr, per_resource_timeout, FanoutOp::Refresh)
            .await
    }

    /// Fans a credential revoke (e.g. an lease revoke) out to every
    /// resource registry row that resolved `cid`, calling
    /// [`Manager::revoke_slot_for_identity`](nebula_resource::Manager::revoke_slot_for_identity)
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
        mgr: &nebula_resource::Manager,
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
    /// concurrently via [`join_all`](futures::future::join_all). This
    /// independent per-future timeout + `join_all` is exactly what
    /// guarantees the timeout-isolation invariant: a slow, failed,
    /// or timed-out row's future resolves on its own and cannot abort or
    /// fail a sibling — every row's outcome is recorded independently.
    ///
    /// **Revoke is two-phase and cancellation-safe .**
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
    /// from [`affected`](Self::affected) is already an owned `Vec`, so no
    /// clone is added over the snapshot), keeping every future self-contained
    /// without `unsafe` lifetime juggling. Only key / slot / scope /
    /// `slot_identity` / duration / counts are logged — never credential
    /// material.
    async fn dispatch(
        &self,
        cid: CredentialId,
        mgr: &nebula_resource::Manager,
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
                    // never skip it . A taint failure
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
                    // guarantee . The row
                    // is already tainted (phase 1); every tail outcome
                    // leaves it tainted.
                    match mgr.drain_and_revoke(tainted, per_resource_timeout).await {
                        nebula_resource::RevokeTail::Done => RowOutcome::Success,
                        nebula_resource::RevokeTail::HookFailed(err) => {
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
                        nebula_resource::RevokeTail::HookTimedOut => {
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
    /// Credential revoked (e.g. lease revoke). Driven as the
    /// two-phase port: synchronous `Manager::taint_slot_for_identity`
    /// outside the timeout, then the timeout-wrapped cancellation-safe
    /// `Manager::drain_and_revoke` tail.
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

/// Per-row fan-out result (one [`Bind`] → exactly one of these).
#[derive(Debug, Clone, Copy)]
enum RowOutcome {
    Success,
    Failed,
    TimedOut,
}

#[cfg(test)]
mod tests {
    use nebula_core::WorkflowId;

    use super::*;

    fn cred() -> CredentialId {
        CredentialId::new()
    }

    fn rk(name: &str) -> ResourceKey {
        ResourceKey::new(name).expect("valid resource key")
    }

    fn wf_scope() -> ScopeLevel {
        ScopeLevel::Workflow(WorkflowId::new())
    }

    fn bound(key: &ResourceKey, scope: &ScopeLevel, slot: &str, identity: SlotIdentity) -> Bind {
        Bind {
            resource_key: key.clone(),
            scope: scope.clone(),
            slot_name: slot.to_string(),
            slot_identity: identity,
        }
    }

    #[test]
    fn index_bind_lookup_unbind_with_identity() {
        let idx = ResourceFanoutIndex::new();
        let cid = cred();
        let key = rk("pg");
        let scope = wf_scope();
        idx.bind(
            cid,
            key.clone(),
            scope.clone(),
            "db",
            SlotIdentity::from_bindings([("k", "cred-0x1234")]),
        );
        assert_eq!(
            idx.affected(&cid),
            vec![bound(
                &key,
                &scope,
                "db",
                SlotIdentity::from_bindings([("k", "cred-0x1234")])
            )]
        );
        idx.unbind_resource(&key, &scope);
        assert!(idx.affected(&cid).is_empty());
    }

    #[test]
    fn distinct_slot_identity_same_resource_are_distinct_binds() {
        // Same ResourceKey + scope, different resolved slot_identity (e.g.
        // two tenants resolving the same resource type to different
        // credentials) MUST be separate entries so the orchestrator routes
        // each rotation to its own resolved registry row.
        let idx = ResourceFanoutIndex::new();
        let key = rk("pg");
        let scope = wf_scope();
        let c1 = cred();
        let c2 = cred();
        idx.bind(
            c1,
            key.clone(),
            scope.clone(),
            "db",
            SlotIdentity::from_bindings([("k", "cred-0xaaaa")]),
        );
        idx.bind(
            c2,
            key.clone(),
            scope.clone(),
            "db",
            SlotIdentity::from_bindings([("k", "cred-0xbbbb")]),
        );
        assert_eq!(
            idx.affected(&c1),
            vec![bound(
                &key,
                &scope,
                "db",
                SlotIdentity::from_bindings([("k", "cred-0xaaaa")])
            )]
        );
        assert_eq!(
            idx.affected(&c2),
            vec![bound(
                &key,
                &scope,
                "db",
                SlotIdentity::from_bindings([("k", "cred-0xbbbb")])
            )]
        );
    }

    #[test]
    fn rebinding_identical_tuple_is_idempotent() {
        let idx = ResourceFanoutIndex::new();
        let cid = cred();
        let key = rk("pg");
        let scope = wf_scope();
        idx.bind(
            cid,
            key.clone(),
            scope.clone(),
            "db",
            SlotIdentity::from_bindings([("k", "cred-0x1234")]),
        );
        idx.bind(
            cid,
            key,
            scope,
            "db",
            SlotIdentity::from_bindings([("k", "cred-0x1234")]),
        );
        assert_eq!(idx.affected(&cid).len(), 1);
    }

    #[test]
    fn unbind_resource_identity_keeps_multi_tenant_siblings() {
        // Two tenants resolve the same (ResourceKey, scope) to different
        // credentials -> two distinct slot identities. Removing one resolved
        // row must NOT collapse the sibling that shares (key, scope).
        let idx = ResourceFanoutIndex::new();
        let key = rk("pg");
        let scope = wf_scope();
        let c1 = cred();
        let c2 = cred();
        idx.bind(
            c1,
            key.clone(),
            scope.clone(),
            "db",
            SlotIdentity::from_bindings([("k", "cred-0xaaaa")]),
        );
        idx.bind(
            c2,
            key.clone(),
            scope.clone(),
            "db",
            SlotIdentity::from_bindings([("k", "cred-0xbbbb")]),
        );

        idx.unbind_resource_identity(
            &key,
            &scope,
            &SlotIdentity::from_bindings([("k", "cred-0xaaaa")]),
        );

        assert!(
            idx.affected(&c1).is_empty(),
            "removed resolved row must be gone"
        );
        assert_eq!(
            idx.affected(&c2),
            vec![bound(
                &key,
                &scope,
                "db",
                SlotIdentity::from_bindings([("k", "cred-0xbbbb")])
            )],
            "sibling sharing (key, scope) but a different identity must survive"
        );
    }

    #[test]
    fn unbind_staged_entry_removes_only_that_tuple() {
        // Precise per-entry inverse of one `bind`: it must drop exactly
        // the staged `(cid, bind)` tuple, keep another credential's
        // binding for the *same* resolved row, drop the bucket when it
        // empties, and be a no-op for an absent credential.
        let idx = ResourceFanoutIndex::new();
        let key = rk("pg");
        let scope = wf_scope();
        let c1 = cred();
        let c2 = cred();
        let id = SlotIdentity::from_bindings([("k", "cred-0xaaaa")]);
        idx.bind(c1, key.clone(), scope.clone(), "db", id.clone());
        idx.bind(c2, key.clone(), scope.clone(), "db", id.clone());

        // No-op for an absent credential.
        idx.unbind_staged_entry(&cred(), &bound(&key, &scope, "db", id.clone()));
        assert_eq!(idx.affected(&c1).len(), 1);
        assert_eq!(idx.affected(&c2).len(), 1);

        // Removes exactly c1's entry; c2's binding for the same resolved
        // row survives untouched.
        idx.unbind_staged_entry(&c1, &bound(&key, &scope, "db", id.clone()));
        assert!(
            idx.affected(&c1).is_empty(),
            "the staged tuple must be gone and its now-empty bucket dropped"
        );
        assert_eq!(
            idx.affected(&c2),
            vec![bound(&key, &scope, "db", id.clone())],
            "another credential's binding for the same row must be untouched"
        );

        // A non-matching bind under a present credential is left alone.
        idx.unbind_staged_entry(
            &c2,
            &bound(
                &key,
                &scope,
                "db",
                SlotIdentity::from_bindings([("k", "cred-0xbbbb")]),
            ),
        );
        assert_eq!(
            idx.affected(&c2),
            vec![bound(&key, &scope, "db", id)],
            "a structurally-different bind must not be removed"
        );
    }

    #[test]
    fn staged_bind_refcount_protects_a_concurrent_live_row() {
        // Two `register_and_bind` calls stage the IDENTICAL resolved row
        // (same cid + Bind). One fails and rolls back; the other
        // succeeds. The failing rollback must NOT delete the surviving
        // registration's live reverse-index row.
        //
        // Before the refcount fix the registrar read
        // `affected(cid).contains(&bind)` before `bind` to decide whether
        // to roll the entry back — a check-then-act race: both calls
        // could observe "absent", both stage it, and the failing call's
        // `unbind_staged_entry` would then delete the row the successful
        // call depends on, leaving a registered resource with no fan-out
        // (silent miss on the next rotation/revoke). This test pins the
        // refcounted ownership that makes the rollback correct without any
        // such read.
        let idx = ResourceFanoutIndex::new();
        let cid = cred();
        let key = rk("pg");
        let scope = wf_scope();
        let id = SlotIdentity::from_bindings([("k", "cred-0x1234")]);

        // Call A stages the row, call B stages the identical row: one
        // refcounted entry, two references.
        idx.bind(cid, key.clone(), scope.clone(), "db", id.clone());
        idx.bind(cid, key.clone(), scope.clone(), "db", id.clone());
        assert_eq!(
            idx.affected(&cid),
            vec![bound(&key, &scope, "db", id.clone())],
            "identical stagings dedupe to one fan-out target"
        );

        // Call A's `register` fails -> its scopeguard releases A's
        // reference. B is still live, so the row MUST survive.
        idx.unbind_staged_entry(&cid, &bound(&key, &scope, "db", id.clone()));
        assert_eq!(
            idx.affected(&cid),
            vec![bound(&key, &scope, "db", id.clone())],
            "a failed concurrent staging must not delete the surviving \
             registration's live fan-out row"
        );

        // B is later removed too -> last reference gone -> row dropped,
        // empty bucket reclaimed.
        idx.unbind_staged_entry(&cid, &bound(&key, &scope, "db", id));
        assert!(
            idx.affected(&cid).is_empty(),
            "the row is removed only when the last referent is gone"
        );
    }

    #[test]
    fn rotation_outcome_dispatched_is_sum() {
        let o = RotationOutcome {
            success: 3,
            failed: 1,
            timed_out: 2,
        };
        assert_eq!(o.dispatched(), 6);
        assert_eq!(RotationOutcome::default().dispatched(), 0);
    }

    #[tokio::test]
    async fn dispatch_refresh_empty_is_noop() {
        // No row bound the credential -> a no-op fan-out, not an error.
        let idx = ResourceFanoutIndex::new();
        let mgr = nebula_resource::Manager::new();
        let out = idx
            .dispatch_refresh(cred(), &mgr, Duration::from_secs(1))
            .await;
        assert_eq!(out, RotationOutcome::default());
        assert_eq!(out.dispatched(), 0);
    }

    // ────────────────────────────────────────────────────────────────────
    // Per-resource timeout-isolation fan-out tests.
    //
    // A controllable resident resource: its `on_credential_refresh` /
    // `on_credential_revoke` hooks either return immediately or block
    // forever, selected per registered slot identity through shared state.
    // Registered multi-tenant under one `(key, scope)` with distinct
    // `slot_identity` values, then driven via the real `Manager`
    // slot-identity-pinned ports the fan-out calls.
    // ────────────────────────────────────────────────────────────────────
    mod fanout_dispatch {
        use std::collections::HashMap;
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        use std::time::Duration;

        use nebula_core::{OrgId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
        use nebula_resource::{
            Manager, ResidentConfig, Resource, ResourceConfig, ResourceContext,
            error::Error as ResourceError,
            resource::ResourceMetadata,
            runtime::{TopologyRuntime, resident::ResidentRuntime},
            topology::resident::Resident,
        };
        use tokio_util::sync::CancellationToken;

        use super::super::*;

        #[derive(Debug)]
        struct HookError(String);
        impl std::fmt::Display for HookError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl std::error::Error for HookError {}
        impl From<HookError> for ResourceError {
            fn from(e: HookError) -> Self {
                ResourceError::transient(e.0)
            }
        }

        /// Per-tenant hook behaviour, keyed by the registered slot identity.
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Behaviour {
            /// Hook returns `Ok` immediately.
            FastOk,
            /// Hook returns `Err` immediately.
            FastErr,
            /// Hook never completes (models a wedged resource) — the
            /// per-resource timeout must fire and NOT abort siblings.
            Hang,
        }

        #[derive(Clone, Default)]
        struct Ledger {
            /// resolved structural slot_identity -> behaviour.
            behaviour: Arc<Mutex<HashMap<SlotIdentity, Behaviour>>>,
            /// Total refresh-hook entries (proves siblings still ran).
            refresh_entered: Arc<AtomicUsize>,
            /// Total revoke-hook entries.
            revoke_entered: Arc<AtomicUsize>,
        }

        impl Ledger {
            fn set(&self, identity: SlotIdentity, b: Behaviour) {
                self.behaviour.lock().expect("ledger").insert(identity, b);
            }
            fn behaviour_for(&self, identity: &SlotIdentity) -> Behaviour {
                *self
                    .behaviour
                    .lock()
                    .expect("ledger")
                    .get(identity)
                    .unwrap_or(&Behaviour::FastOk)
            }
        }

        /// Behaviour is keyed off the resolved slot identity carried by
        /// `CtlResource`, not config — so config is empty.
        #[derive(Clone)]
        struct Cfg;

        nebula_schema::impl_empty_has_schema!(Cfg);

        impl ResourceConfig for Cfg {
            fn validate(&self) -> Result<(), ResourceError> {
                Ok(())
            }
        }

        #[derive(Clone)]
        struct Runtime;

        #[derive(Clone)]
        struct CtlResource {
            identity: SlotIdentity,
            ledger: Ledger,
        }

        impl Resource for CtlResource {
            type Config = Cfg;
            type Runtime = Runtime;
            type Lease = Runtime;
            type Error = HookError;

            fn key() -> ResourceKey {
                resource_key!("fanout-ctl")
            }

            async fn create(
                &self,
                _config: &Cfg,
                _ctx: &ResourceContext,
            ) -> Result<Runtime, HookError> {
                Ok(Runtime)
            }

            async fn on_credential_refresh(
                &self,
                _slot: &str,
                _rt: &Runtime,
            ) -> Result<(), HookError> {
                self.ledger.refresh_entered.fetch_add(1, Ordering::SeqCst);
                match self.ledger.behaviour_for(&self.identity) {
                    Behaviour::FastOk => Ok(()),
                    Behaviour::FastErr => Err(HookError("refresh boom".to_owned())),
                    Behaviour::Hang => {
                        // Never completes; the fan-out's per-resource
                        // timeout must elapse and record TimedOut without
                        // touching siblings.
                        std::future::pending::<()>().await;
                        // guard-justified: `std::future::pending()` never
                        // resolves, so this line is statically unreachable.
                        unreachable!("pending future never resolves")
                    },
                }
            }

            async fn on_credential_revoke(
                &self,
                _slot: &str,
                _rt: &Runtime,
            ) -> Result<(), HookError> {
                self.ledger.revoke_entered.fetch_add(1, Ordering::SeqCst);
                match self.ledger.behaviour_for(&self.identity) {
                    Behaviour::FastOk => Ok(()),
                    Behaviour::FastErr => Err(HookError("revoke boom".to_owned())),
                    Behaviour::Hang => {
                        std::future::pending::<()>().await;
                        // guard-justified: `std::future::pending()` never
                        // resolves, so this line is statically unreachable.
                        unreachable!("pending future never resolves")
                    },
                }
            }

            fn metadata() -> ResourceMetadata {
                ResourceMetadata::from_key(&Self::key())
            }
        }

        impl Resident for CtlResource {
            fn is_alive_sync(&self, _rt: &Runtime) -> bool {
                true
            }
        }

        /// Register `identities.len()` distinct tenants under ONE
        /// `(key, scope)` (distinct `slot_identity`), warm each resident
        /// runtime, bind every row into a fresh index under `cid`, and
        /// return `(index, manager, cid, scope, org, ledger)`. `org` is the
        /// `OrgId` backing `scope` so a caller can build an acquire
        /// `ResourceContext` for the registered scope without re-deriving it
        /// from `scope` (no destructure-or-panic at the call site).
        async fn setup(
            identities: &[SlotIdentity],
        ) -> (
            ResourceFanoutIndex,
            Arc<Manager>,
            CredentialId,
            ScopeLevel,
            OrgId,
            Ledger,
        ) {
            let ledger = Ledger::default();
            let org = OrgId::new();
            let scope = ScopeLevel::Organization(org);
            let mgr = Arc::new(Manager::new());
            let idx = ResourceFanoutIndex::new();
            let cid = CredentialId::new();

            for id in identities {
                mgr.register(nebula_resource::RegistrationSpec {
                    resource: CtlResource {
                        identity: id.clone(),
                        ledger: ledger.clone(),
                    },
                    config: Cfg,
                    scope: scope.clone(),
                    slot_identity: id.clone(),
                    topology: TopologyRuntime::Resident(ResidentRuntime::<CtlResource>::new(
                        ResidentConfig::default(),
                    )),
                    acquire: Manager::erased_acquire_resident_for::<CtlResource>(),
                    resilience: None,
                    recovery_gate: None,
                })
                .expect("register tenant");

                // Resident materializes its shared runtime lazily on first
                // acquire — touch it so the rotation hook has a live
                // `&Runtime` to borrow.
                let ctx = ResourceContext::minimal(
                    Scope {
                        org_id: Some(org),
                        ..Default::default()
                    },
                    CancellationToken::new(),
                );
                let _g = mgr
                    .acquire_resident_for_identity::<CtlResource>(
                        &ctx,
                        &nebula_resource::AcquireOptions::default(),
                        id,
                    )
                    .await
                    .expect("warm tenant runtime");

                idx.bind(cid, CtlResource::key(), scope.clone(), "db", id.clone());
            }

            (idx, mgr, cid, scope, org, ledger)
        }

        /// Isolation invariant: one resource that times out must NOT abort
        /// or fail its siblings — they still refresh. Three tenants under
        /// one `(key, scope)`; the middle one hangs.
        #[tokio::test]
        async fn refresh_fanout_isolates_a_timed_out_resource() {
            let (a, b, c) = (
                SlotIdentity::from_bindings([("k", "cred-0xaaaa_u64")]),
                SlotIdentity::from_bindings([("k", "cred-0xbbbb_u64")]),
                SlotIdentity::from_bindings([("k", "cred-0xcccc_u64")]),
            );
            let (idx, mgr, cid, _scope, _org, ledger) =
                setup(&[a.clone(), b.clone(), c.clone()]).await;
            ledger.set(a, Behaviour::FastOk);
            ledger.set(b, Behaviour::Hang);
            ledger.set(c, Behaviour::FastOk);

            // Per-resource budget small enough that the hung row times out
            // quickly; the FastOk siblings complete well within it.
            let out = idx
                .dispatch_refresh(cid, &mgr, Duration::from_millis(150))
                .await;

            assert_eq!(
                out,
                RotationOutcome {
                    success: 2,
                    failed: 0,
                    timed_out: 1,
                },
                "one hung resource must time out in isolation; both siblings still refresh"
            );
            assert_eq!(out.dispatched(), 3, "every bound row is accounted for");
            // All three hooks were entered (the hung one too) — proof the
            // siblings were not aborted by the hung one.
            assert_eq!(
                ledger.refresh_entered.load(Ordering::SeqCst),
                3,
                "every resource's hook ran; the timeout did not cancel siblings"
            );
        }

        /// Mixed outcomes in one fan-out: ok + err + timeout each counted
        /// independently, none aborting the others.
        #[tokio::test]
        async fn refresh_fanout_mixed_outcomes_each_independent() {
            let (a, b, c, d) = (
                SlotIdentity::from_bindings([("k", "cred-0x1_u64")]),
                SlotIdentity::from_bindings([("k", "cred-0x2_u64")]),
                SlotIdentity::from_bindings([("k", "cred-0x3_u64")]),
                SlotIdentity::from_bindings([("k", "cred-0x4_u64")]),
            );
            let (idx, mgr, cid, _scope, _org, ledger) =
                setup(&[a.clone(), b.clone(), c.clone(), d.clone()]).await;
            ledger.set(a, Behaviour::FastOk);
            ledger.set(b, Behaviour::FastErr);
            ledger.set(c, Behaviour::Hang);
            ledger.set(d, Behaviour::FastOk);

            let out = idx
                .dispatch_refresh(cid, &mgr, Duration::from_millis(150))
                .await;

            assert_eq!(
                out,
                RotationOutcome {
                    success: 2,
                    failed: 1,
                    timed_out: 1,
                },
            );
            assert_eq!(out.dispatched(), 4);
        }

        /// Revoke analogue of the isolation test.
        #[tokio::test]
        async fn revoke_fanout_isolates_a_timed_out_resource() {
            let (a, b, c) = (
                SlotIdentity::from_bindings([("k", "cred-0xdead_u64")]),
                SlotIdentity::from_bindings([("k", "cred-0xbeef_u64")]),
                SlotIdentity::from_bindings([("k", "cred-0xf00d_u64")]),
            );
            let (idx, mgr, cid, _scope, _org, ledger) =
                setup(&[a.clone(), b.clone(), c.clone()]).await;
            ledger.set(a, Behaviour::FastOk);
            ledger.set(b, Behaviour::Hang);
            ledger.set(c, Behaviour::FastOk);

            let out = idx
                .dispatch_revoke(cid, &mgr, Duration::from_millis(150))
                .await;

            assert_eq!(
                out,
                RotationOutcome {
                    success: 2,
                    failed: 0,
                    timed_out: 1,
                },
                "a hung revoke must time out in isolation; siblings still revoke"
            );
            assert_eq!(
                ledger.revoke_entered.load(Ordering::SeqCst),
                3,
                "every resource's revoke hook ran"
            );
        }

        /// Builds an acquire context for the registered Organization scope
        /// without re-deriving it from a `ScopeLevel` (no destructure-or-
        /// panic): `setup` hands back the `OrgId` directly.
        fn ctx_for(org: OrgId) -> ResourceContext {
            ResourceContext::minimal(
                Scope {
                    org_id: Some(org),
                    ..Default::default()
                },
                CancellationToken::new(),
            )
        }

        /// #681 — the cancellation-safety invariant of the two-phase port.
        ///
        /// A revoke whose `drain_and_revoke` tail times out (the revoke hook
        /// hangs) MUST still have left the row **tainted**: the synchronous
        /// `taint_slot_for` ran *outside* the per-resource timeout, so a
        /// timed-out drain/hook cannot un-revoke the credential. Asserts both
        /// that the fan-out records `timed_out` (not success, not dropped)
        /// **and** that a fresh acquire on that exact resolved row is
        /// rejected *after* the timed-out fan-out returned — proof the taint
        /// survived the timeout.
        #[tokio::test]
        async fn revoke_fanout_timed_out_drain_still_left_row_tainted() {
            use nebula_error::{Classify, ErrorCategory};
            use nebula_resource::AcquireOptions;

            let hung = SlotIdentity::from_bindings([("k", "cred-0x5151_u64")]);
            let (idx, mgr, cid, _scope, org, ledger) = setup(std::slice::from_ref(&hung)).await;
            // The revoke hook never returns -> `drain_and_revoke` (the
            // timeout-wrapped phase 2) times out. `hung` is reused below
            // (the structural identity is no longer `Copy`), so clone here.
            ledger.set(hung.clone(), Behaviour::Hang);

            let out = idx
                .dispatch_revoke(cid, &mgr, Duration::from_millis(150))
                .await;

            assert_eq!(
                out,
                RotationOutcome {
                    success: 0,
                    failed: 0,
                    timed_out: 1,
                },
                "a hung revoke hook must record timed_out — never success, never dropped",
            );
            assert_eq!(
                ledger.revoke_entered.load(Ordering::SeqCst),
                1,
                "phase 2 (drain_and_revoke) did run and reached the hung hook",
            );

            // The decisive #681 assertion: the row is STILL tainted after the
            // timed-out fan-out returned. If the taint had been inside the
            // timeout future it would have been skipped/rolled back; here it
            // ran synchronously *before* the timeout, so new acquires on this
            // exact resolved row stay rejected.
            let ctx = ctx_for(org);
            let acquired = mgr
                .acquire_resident_for_identity::<CtlResource>(
                    &ctx,
                    &AcquireOptions::default(),
                    &hung,
                )
                .await;
            let err = match acquired {
                Err(e) => e,
                Ok(_) => {
                    // guard-justified: a live guard here is the exact #681
                    // regression (taint lost across the timeout); fail the
                    // test loudly with no salvage path.
                    unreachable!(
                        "acquire after a timed-out revoke must be rejected — \
                         the row must stay tainted (#681)"
                    )
                },
            };
            assert_eq!(
                err.category(),
                ErrorCategory::Unavailable,
                "post-timeout acquire must be the Revoked/Unavailable taint rejection, got: {err}",
            );
        }

        /// #681 — cancellation: dropping the `drain_and_revoke` future the
        /// instant after `taint_slot_for` returns must leave the row tainted.
        ///
        /// This drives the two-phase port directly (the exact split the
        /// fan-out uses): synchronous `taint_slot_for` first, then *drop* the
        /// still-pending `drain_and_revoke` future mid-flight (a
        /// `tokio::time::timeout` elapsing and dropping the wrapped future is
        /// the real-world trigger). A real in-flight guard is held so the
        /// per-resource drain genuinely parks the future (it cannot complete
        /// on its first poll). Because the taint already completed
        /// synchronously in phase 1, no acquire on the row may succeed
        /// afterward.
        #[tokio::test]
        async fn revoke_two_phase_dropping_drain_future_keeps_taint() {
            use nebula_error::{Classify, ErrorCategory};
            use nebula_resource::AcquireOptions;

            let id = SlotIdentity::from_bindings([("k", "cred-0x7a1d_u64")]);
            let (_idx, mgr, _cid, scope, org, ledger) = setup(std::slice::from_ref(&id)).await;
            // Even a hook that *would* succeed: we never let phase 2 run.
            // `id` is reused below (no longer `Copy`), so clone here.
            ledger.set(id.clone(), Behaviour::FastOk);

            // Hold a real in-flight guard so phase 2's per-resource drain
            // blocks (counter stays at 1) — `drain_and_revoke` parks instead
            // of completing on its first poll, making the subsequent drop a
            // true mid-flight cancellation.
            let in_flight = match mgr
                .acquire_resident_for_identity::<CtlResource>(
                    &ctx_for(org),
                    &AcquireOptions::default(),
                    &id,
                )
                .await
            {
                Ok(g) => g,
                Err(e) => {
                    // guard-justified: `setup` registered+warmed this row, so
                    // an acquire on the un-tainted resource cannot fail here;
                    // a failure is a broken-test invariant, not a real path.
                    unreachable!("acquire on the freshly warmed row must succeed: {e}")
                },
            };

            // Phase 1: synchronous taint, outside any timeout.
            let tainted = match mgr.taint_slot_for_identity(&CtlResource::key(), scope, "db", &id) {
                Ok(t) => t,
                Err(e) => {
                    // guard-justified: the row was just registered+warmed by
                    // `setup`, so phase-1 resolution cannot fail here; a
                    // failure is a broken-test invariant, not a runtime path.
                    unreachable!("taint_slot_for must resolve the freshly bound row: {e}")
                },
            };

            // Phase 2 constructed, polled until it parks in the per-resource
            // drain (the held guard keeps the counter > 0), then explicitly
            // DROPPED while still pending — models a generic task-abort /
            // runtime-shutdown cancellation of the awaiting task (post-#690
            // the fan-out no longer wraps this in an outer timeout; the
            // synchronous-phase-1 taint must still survive any such drop).
            {
                let mut fut = Box::pin(mgr.drain_and_revoke(tainted, Duration::from_secs(30)));
                let parked = tokio::time::timeout(Duration::from_millis(150), &mut fut).await;
                assert!(
                    parked.is_err(),
                    "drain_and_revoke must still be parked in the per-resource \
                     drain while the in-flight guard is held",
                );
                drop(fut);
            }
            assert_eq!(
                ledger.revoke_entered.load(Ordering::SeqCst),
                0,
                "the dropped (never-completed) drain future must not have run \
                 the revoke hook",
            );

            // Invariant: taint survived the dropped tail (it ran in phase 1).
            // Drop the in-flight guard first so this fresh acquire is gated
            // only by the taint, not by the still-held lease.
            drop(in_flight);
            let ctx = ctx_for(org);
            let acquired = mgr
                .acquire_resident_for_identity::<CtlResource>(&ctx, &AcquireOptions::default(), &id)
                .await;
            let err = match acquired {
                Err(e) => e,
                Ok(_) => {
                    // guard-justified: a guard after a dropped drain future
                    // means phase-1 taint did not stick — the exact #681
                    // cancellation hole; fail loudly, no salvage.
                    unreachable!(
                        "no acquire may succeed after a dropped drain future — \
                         the synchronous phase-1 taint already revoked the row (#681)"
                    )
                },
            };
            assert_eq!(
                err.category(),
                ErrorCategory::Unavailable,
                "dropped-tail acquire must still hit the Revoked/Unavailable taint, got: {err}",
            );
        }

        /// All-OK fast path: every bound row refreshes, no failures/timeouts.
        #[tokio::test]
        async fn refresh_fanout_all_ok() {
            let ids = [
                SlotIdentity::from_bindings([("k", "cred-10")]),
                SlotIdentity::from_bindings([("k", "cred-20")]),
                SlotIdentity::from_bindings([("k", "cred-30")]),
            ];
            let (idx, mgr, cid, _scope, _org, ledger) = setup(&ids).await;
            for id in ids {
                ledger.set(id, Behaviour::FastOk);
            }

            let out = idx
                .dispatch_refresh(cid, &mgr, Duration::from_secs(5))
                .await;

            assert_eq!(
                out,
                RotationOutcome {
                    success: 3,
                    failed: 0,
                    timed_out: 0,
                },
            );
        }
    }
}
