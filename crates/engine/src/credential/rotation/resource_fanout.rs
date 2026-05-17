//! Engine-owned reverse index: `CredentialId` -> affected resource rows.
//!
//! Per ADR-0030, `nebula-engine` (exec layer) owns credential rotation
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
//! [`nebula_resource::dedup`] and [`nebula_resource::SLOT_IDENTITY_UNBOUND`].
//! Two registrations of the same resource type at the same scope whose
//! resolved credentials differ are *distinct rows* (the multi-tenant
//! anti-bleed barrier). A `Manager::refresh_slot` call against a multi-row
//! `(key, scope)` fails closed (`Ambiguous`) precisely because it cannot pick
//! a row without the resolved identity.
//!
//! The reverse-index entry therefore records the resolved `slot_identity`
//! alongside `(ResourceKey, ScopeLevel, slot_name)` so a rotation routes to
//! the *specific* resolved registry row rather than the whole `(key, scope)`
//! family. This is forward-correctness against the structural dedup model,
//! not extra precision for its own sake.
//!
//! Per ADR-0036 (event-driven cross-crate flow) the engine consumes the
//! credential rotation signal and translates it into typed `Manager` port
//! calls; per ADR-0044 the resource layer never reaches back across the
//! boundary. This index is an in-process, in-memory routing table only —
//! never persisted and never sent across a trust boundary.

use std::time::Duration;

use dashmap::DashMap;
use nebula_core::{ResourceKey, ScopeLevel};
use nebula_credential::CredentialId;

/// One resource registry row affected by a credential rotation.
///
/// - `resource_key` / `scope`: the structural address of the registry row.
/// - `slot_name`: the credential slot on that row that resolved the rotated
///   credential.
/// - `slot_identity`: the resolved structural identity from
///   [`nebula_resource::dedup::slot_identity`]; it disambiguates multi-tenant
///   rows that share `(resource_key, scope)` so a rotation routes to exactly
///   the row whose slot resolved to the rotated credential.
///
/// [`nebula_resource::SLOT_IDENTITY_UNBOUND`] is the `slot_identity` for a row
/// that resolved no credential slots (single-row-per-`(key, scope)` legacy
/// behaviour); such rows still appear here verbatim.
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
    /// Resolved structural slot identity disambiguating multi-tenant rows.
    pub slot_identity: u64,
}

/// Aggregate of a per-slot rotation fan-out across every affected resource
/// registry row.
///
/// One [`Bind`] contributes exactly one of the three counts, so
/// `success + failed + timed_out == affected_rows`. Per ADR-0036's
/// per-resource timeout-isolation invariant a slow, failed, or timed-out row
/// never aborts or fails its siblings — each row's outcome is independent. The
/// struct carries only counts (no key/slot/credential material) so it is safe
/// to log or emit as a metrics/dashboard signal; it is **not** a substitute
/// for an audit write (ADR-0028 §4).
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
    /// `CredentialId` -> rows whose resolved slot bound that credential.
    ///
    /// `nebula-engine` has no direct `smallvec` dependency, so the
    /// per-credential row list is a plain `Vec`. Promoting this to a small
    /// inline buffer is a deferred, dependency-gated optimisation.
    by_credential: DashMap<CredentialId, Vec<Bind>>,
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
    /// (no duplicate entry) so a resource that re-registers without changing
    /// its resolved binding does not fan out twice.
    pub fn bind(
        &self,
        cid: CredentialId,
        resource_key: ResourceKey,
        scope: ScopeLevel,
        slot_name: impl Into<String>,
        slot_identity: u64,
    ) {
        let entry = Bind {
            resource_key,
            scope,
            slot_name: slot_name.into(),
            slot_identity,
        };
        let mut rows = self.by_credential.entry(cid).or_default();
        if !rows.contains(&entry) {
            rows.push(entry);
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
            .map(|rows| rows.clone())
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
            rows.retain(|b| b.resource_key != *resource_key || b.scope != *scope);
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
        slot_identity: u64,
    ) {
        self.by_credential.retain(|_, rows| {
            rows.retain(|b| {
                b.resource_key != *resource_key
                    || b.scope != *scope
                    || b.slot_identity != slot_identity
            });
            !rows.is_empty()
        });
    }

    /// Fans a completed credential refresh out to every resource registry
    /// row that resolved `cid`, calling
    /// [`Manager::refresh_slot_for`](nebula_resource::Manager::refresh_slot_for)
    /// per row.
    ///
    /// The engine (exec layer, ADR-0030) owns rotation orchestration: it has
    /// already resolved and stored the fresh credential material before this
    /// is called; this method only translates the single rotation signal
    /// into the typed per-row resource port (ADR-0036), and the resource
    /// layer never reaches back (ADR-0044).
    ///
    /// **Per-resource timeout isolation (ADR-0036 invariant).** Each row's
    /// `refresh_slot_for` is independently wrapped in
    /// `tokio::time::timeout(per_resource_timeout, …)` and all are driven
    /// concurrently via [`futures::future::join_all`]. One slow, failed, or
    /// timed-out row therefore **never aborts or fails a sibling** — every
    /// row's outcome is recorded independently and folded into the returned
    /// [`RotationOutcome`] (`success + failed + timed_out == affected_rows`).
    ///
    /// Identity routing: a multi-tenant `(key, scope)` has more than one
    /// resolved row, so `Manager::refresh_slot` (identity-agnostic) would
    /// fail closed with `Ambiguous`. This drives the slot-identity-pinned
    /// `refresh_slot_for` with the `slot_identity` recorded at
    /// [`bind`](Self::bind) time so the rotation reaches exactly the
    /// resolved row.
    ///
    /// Redaction: only the aggregate counts and per-row key / slot / scope /
    /// `slot_identity` (a `u64`) / duration reach spans — never credential
    /// or secret material. The returned aggregate is a metrics/dashboard
    /// signal, **not** an audit record (ADR-0028 §4); the caller still owns
    /// any audit write.
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

    /// Fans a credential revoke (e.g. an ADR-0051 lease revoke) out to every
    /// resource registry row that resolved `cid`, calling
    /// [`Manager::revoke_slot_for`](nebula_resource::Manager::revoke_slot_for)
    /// per row.
    ///
    /// Same per-resource timeout isolation, identity routing, redaction, and
    /// "aggregate is not an audit record" contract as
    /// [`dispatch_refresh`](Self::dispatch_refresh) — only the per-row port
    /// differs (`revoke_slot_for` taints → drains → runs the revoke hook).
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
    /// guarantees the ADR-0036 timeout-isolation invariant: a slow, failed,
    /// or timed-out row's future resolves on its own and cannot abort or
    /// fail a sibling — every row's outcome is recorded independently.
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
            // The port future borrows `b`; it is constructed and awaited
            // entirely inside this per-row future, which owns `b`, so no
            // cross-future borrow and no clone beyond the owned snapshot.
            let port = async {
                match op {
                    FanoutOp::Refresh => {
                        mgr.refresh_slot_for(
                            &b.resource_key,
                            b.scope.clone(),
                            &b.slot_name,
                            b.slot_identity,
                        )
                        .await
                    },
                    FanoutOp::Revoke => {
                        mgr.revoke_slot_for(
                            &b.resource_key,
                            b.scope.clone(),
                            &b.slot_name,
                            b.slot_identity,
                        )
                        .await
                    },
                }
            };
            match tokio::time::timeout(per_resource_timeout, port).await {
                Ok(Ok(())) => RowOutcome::Success,
                Ok(Err(err)) => {
                    // Resource-crate errors are already credential-free
                    // (key/slot/scope only); safe to log verbatim.
                    tracing::warn!(
                        credential_id = %cid,
                        resource_key = %b.resource_key,
                        slot = %b.slot_name,
                        slot_identity = b.slot_identity,
                        error = %err,
                        "rotation fan-out: per-resource {op_name} failed; siblings unaffected",
                    );
                    RowOutcome::Failed
                },
                Err(_elapsed) => {
                    tracing::warn!(
                        credential_id = %cid,
                        resource_key = %b.resource_key,
                        slot = %b.slot_name,
                        slot_identity = b.slot_identity,
                        timeout_ms = per_resource_timeout.as_millis() as u64,
                        "rotation fan-out: per-resource {op_name} timed out; siblings unaffected",
                    );
                    RowOutcome::TimedOut
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
    /// `Manager::refresh_slot_for` — credential rotated, fresh material
    /// already resolved and stored by the engine.
    Refresh,
    /// `Manager::revoke_slot_for` — credential revoked (e.g. ADR-0051 lease
    /// revoke); taint → drain → revoke hook.
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

    fn bound(key: &ResourceKey, scope: &ScopeLevel, slot: &str, identity: u64) -> Bind {
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
        idx.bind(cid, key.clone(), scope.clone(), "db", 0x1234);
        assert_eq!(idx.affected(&cid), vec![bound(&key, &scope, "db", 0x1234)]);
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
        idx.bind(c1, key.clone(), scope.clone(), "db", 0xAAAA);
        idx.bind(c2, key.clone(), scope.clone(), "db", 0xBBBB);
        assert_eq!(idx.affected(&c1), vec![bound(&key, &scope, "db", 0xAAAA)]);
        assert_eq!(idx.affected(&c2), vec![bound(&key, &scope, "db", 0xBBBB)]);
    }

    #[test]
    fn rebinding_identical_tuple_is_idempotent() {
        let idx = ResourceFanoutIndex::new();
        let cid = cred();
        let key = rk("pg");
        let scope = wf_scope();
        idx.bind(cid, key.clone(), scope.clone(), "db", 0x1234);
        idx.bind(cid, key, scope, "db", 0x1234);
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
        idx.bind(c1, key.clone(), scope.clone(), "db", 0xAAAA);
        idx.bind(c2, key.clone(), scope.clone(), "db", 0xBBBB);

        idx.unbind_resource_identity(&key, &scope, 0xAAAA);

        assert!(
            idx.affected(&c1).is_empty(),
            "removed resolved row must be gone"
        );
        assert_eq!(
            idx.affected(&c2),
            vec![bound(&key, &scope, "db", 0xBBBB)],
            "sibling sharing (key, scope) but a different identity must survive"
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
    // Per-resource timeout-isolation fan-out tests (ADR-0036 invariant).
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
            Manager, RegisterOptions, ResidentConfig, Resource, ResourceConfig, ResourceContext,
            error::Error as ResourceError, resource::ResourceMetadata,
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
            /// slot_identity -> behaviour.
            behaviour: Arc<Mutex<HashMap<u64, Behaviour>>>,
            /// Total refresh-hook entries (proves siblings still ran).
            refresh_entered: Arc<AtomicUsize>,
            /// Total revoke-hook entries.
            revoke_entered: Arc<AtomicUsize>,
        }

        impl Ledger {
            fn set(&self, identity: u64, b: Behaviour) {
                self.behaviour.lock().expect("ledger").insert(identity, b);
            }
            fn behaviour_for(&self, identity: u64) -> Behaviour {
                *self
                    .behaviour
                    .lock()
                    .expect("ledger")
                    .get(&identity)
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
            identity: u64,
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
                match self.ledger.behaviour_for(self.identity) {
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
                match self.ledger.behaviour_for(self.identity) {
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
        /// return `(index, manager, cid, scope, ledger)`.
        async fn setup(
            identities: &[u64],
        ) -> (
            ResourceFanoutIndex,
            Arc<Manager>,
            CredentialId,
            ScopeLevel,
            Ledger,
        ) {
            let ledger = Ledger::default();
            let org = OrgId::new();
            let scope = ScopeLevel::Organization(org);
            let mgr = Arc::new(Manager::new());
            let idx = ResourceFanoutIndex::new();
            let cid = CredentialId::new();

            for &id in identities {
                mgr.register_resident_with(
                    CtlResource {
                        identity: id,
                        ledger: ledger.clone(),
                    },
                    Cfg,
                    ResidentConfig::default(),
                    RegisterOptions::default()
                        .with_scope(scope.clone())
                        .with_slot_identity(id),
                )
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
                    .acquire_resident_for::<CtlResource>(
                        &ctx,
                        &nebula_resource::AcquireOptions::default(),
                        id,
                    )
                    .await
                    .expect("warm tenant runtime");

                idx.bind(cid, CtlResource::key(), scope.clone(), "db", id);
            }

            (idx, mgr, cid, scope, ledger)
        }

        /// Isolation invariant: one resource that times out must NOT abort
        /// or fail its siblings — they still refresh. Three tenants under
        /// one `(key, scope)`; the middle one hangs.
        #[tokio::test]
        async fn refresh_fanout_isolates_a_timed_out_resource() {
            let (a, b, c) = (0xAAAA_u64, 0xBBBB_u64, 0xCCCC_u64);
            let (idx, mgr, cid, _scope, ledger) = setup(&[a, b, c]).await;
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
            let (a, b, c, d) = (0x1_u64, 0x2_u64, 0x3_u64, 0x4_u64);
            let (idx, mgr, cid, _scope, ledger) = setup(&[a, b, c, d]).await;
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
            let (a, b, c) = (0xDEAD_u64, 0xBEEF_u64, 0xF00D_u64);
            let (idx, mgr, cid, _scope, ledger) = setup(&[a, b, c]).await;
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

        /// All-OK fast path: every bound row refreshes, no failures/timeouts.
        #[tokio::test]
        async fn refresh_fanout_all_ok() {
            let ids = [10_u64, 20, 30];
            let (idx, mgr, cid, _scope, ledger) = setup(&ids).await;
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
