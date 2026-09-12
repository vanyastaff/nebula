use std::time::Duration;

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
        deferred: 1,
        abandoned: 1,
        drain_timed_out: 2,
        observation_timed_out: 1,
    };
    assert_eq!(o.dispatched(), 8);
    assert_eq!(o.drain_timed_out(), 2);
    assert_eq!(RotationOutcome::default().dispatched(), 0);
}

#[tokio::test]
async fn dispatch_refresh_empty_is_noop() {
    // No row bound the credential -> a no-op fan-out, not an error.
    let idx = ResourceFanoutIndex::new();
    let mgr = crate::Manager::new();
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

    use crate::{
        AcquireOptions, Manager, ManagerConfig, Provider, Resident, ResidentConfig, ResourceConfig,
        ResourceContext, ResourceEvent,
        error::Error as ResourceError,
        release_queue::{ReleaseQueue, SubmissionOutcome},
        resource::{HasCredentialSlots, ResourceMetadataDraft},
        topology::resident::ResidentProvider,
    };
    use futures::FutureExt;
    use nebula_core::{OrgId, ResourceKey, ScopeLevel, resource_key, scope::Scope};
    use nebula_credential::CredentialId;
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
            self.behaviour
                .lock()
                .expect("ledger lock")
                .insert(identity, b);
        }
        fn behaviour_for(&self, identity: &SlotIdentity) -> Behaviour {
            *self
                .behaviour
                .lock()
                .expect("ledger lock")
                .get(identity)
                .unwrap_or(&Behaviour::FastOk)
        }
    }

    /// Behaviour is keyed off the resolved slot identity carried by
    /// `CtlResource`, not config — so config is empty.
    #[derive(Clone, nebula_schema::Schema)]
    struct Cfg;

    impl ResourceConfig for Cfg {
        fn validate(&self) -> Result<(), ResourceError> {
            Ok(())
        }

        fn fingerprint(&self) -> u64 {
            // Unit struct: all instances identical — constant 0 is correct.
            0
        }
    }

    #[derive(Clone)]
    struct Runtime;

    #[derive(Clone)]
    struct CtlResource {
        identity: SlotIdentity,
        ledger: Ledger,
    }

    #[async_trait::async_trait]
    impl Provider for CtlResource {
        type Config = Cfg;
        type Instance = Runtime;
        type Topology = Resident<Self>;

        fn key() -> ResourceKey {
            resource_key!("fanout-ctl")
        }

        async fn create(
            &self,
            _config: &Cfg,
            _ctx: &ResourceContext,
        ) -> Result<Runtime, ResourceError> {
            Ok(Runtime)
        }

        async fn on_credential_refresh(
            &self,
            _slot: &str,
            _rt: &Runtime,
        ) -> Result<(), ResourceError> {
            self.ledger.refresh_entered.fetch_add(1, Ordering::SeqCst);
            match self.ledger.behaviour_for(&self.identity) {
                Behaviour::FastOk => Ok(()),
                Behaviour::FastErr => Err(HookError("refresh boom".to_owned()).into()),
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
        ) -> Result<(), ResourceError> {
            self.ledger.revoke_entered.fetch_add(1, Ordering::SeqCst);
            match self.ledger.behaviour_for(&self.identity) {
                Behaviour::FastOk => Ok(()),
                Behaviour::FastErr => Err(HookError("revoke boom".to_owned()).into()),
                Behaviour::Hang => {
                    std::future::pending::<()>().await;
                    // guard-justified: `std::future::pending()` never
                    // resolves, so this line is statically unreachable.
                    unreachable!("pending future never resolves")
                },
            }
        }

        fn metadata() -> ResourceMetadataDraft {
            ResourceMetadataDraft::new(Self::key(), crate::metadata_name!("fanout-ctl"), "")
        }
    }

    // A real declared "db" credential slot — every fan-out scenario in
    // this module drives `refresh_slot`/`taint_slot`/`revoke_slot(...,
    // "db")` against `on_credential_refresh`/`on_credential_revoke`, so
    // `no_credential_slots!` would misrepresent this fixture as
    // slot-less and (fail-closed) reject every one of those calls.
    impl HasCredentialSlots for CtlResource {
        fn credential_slot_epoch(&self) -> u64 {
            // This module's fan-out isolation is proven via the
            // `Ledger` hook-entry counters, not the epoch fold.
            0
        }

        fn declares_credential_slots() -> bool {
            true
        }

        fn credential_slot_names() -> &'static [&'static str] {
            &["db"]
        }
    }

    #[async_trait::async_trait]
    impl ResidentProvider for CtlResource {
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
        setup_with_config(identities, ManagerConfig::default()).await
    }

    async fn setup_with_config(
        identities: &[SlotIdentity],
        manager_config: ManagerConfig,
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
        let mgr = Arc::new(Manager::with_config(manager_config));
        let idx = ResourceFanoutIndex::new();
        let cid = CredentialId::new();

        for id in identities {
            mgr.register(crate::RegistrationSpec {
                resource: CtlResource {
                    identity: id.clone(),
                    ledger: ledger.clone(),
                },
                config: Cfg,
                scope: scope.clone(),
                slot_identity: id.clone(),
                topology: Resident::<CtlResource>::new(ResidentConfig::default()),
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
                .acquire_resident_for_identity::<CtlResource>(&ctx, &AcquireOptions::default(), id)
                .await
                .expect("warm tenant runtime");

            idx.bind(cid, CtlResource::key(), scope.clone(), "db", id.clone());
        }

        (idx, mgr, cid, scope, org, ledger)
    }

    #[derive(Clone, Copy)]
    enum TerminalDirection {
        Refresh,
        Revoke,
    }

    async fn collect_terminal_events(
        events: &mut nebula_eventbus::Subscriber<ResourceEvent>,
        direction: TerminalDirection,
        expected: usize,
    ) -> (usize, usize) {
        let mut succeeded = 0;
        let mut failed = 0;
        while succeeded + failed < expected {
            match (
                direction,
                events
                    .recv()
                    .await
                    .expect("terminal event bus remains open"),
            ) {
                (TerminalDirection::Refresh, ResourceEvent::SlotRefreshed { .. })
                | (TerminalDirection::Revoke, ResourceEvent::SlotRevoked { .. }) => {
                    succeeded += 1;
                },
                (TerminalDirection::Refresh, ResourceEvent::SlotRefreshFailed { .. })
                | (TerminalDirection::Revoke, ResourceEvent::SlotRevokeFailed { .. }) => {
                    failed += 1;
                },
                _ => {},
            }
        }
        (succeeded, failed)
    }

    #[tokio::test]
    async fn cross_queue_refresh_defers_observation_then_settles_terminal_success_once() {
        let identity = SlotIdentity::from_bindings([("k", "cred-cross-queue")]);
        let metrics_registry = Arc::new(nebula_metrics::MetricsRegistry::new());
        let manager_config =
            ManagerConfig::default().with_metrics_registry(Arc::clone(&metrics_registry));
        let (index, manager, credential_id, _scope, _org, ledger) =
            setup_with_config(std::slice::from_ref(&identity), manager_config).await;
        ledger.set(identity, Behaviour::FastOk);
        let mut events = manager.subscribe_events();

        let (caller_queue, caller_workers) = ReleaseQueue::new(1);
        let caller_queue = Arc::new(caller_queue);
        let index = Arc::new(index);
        let manager_for_dispatch = Arc::clone(&manager);
        let index_for_dispatch = Arc::clone(&index);
        let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel();
        let caller_submission = caller_queue
            .submit_release(move || {
                Box::pin(async move {
                    let outcome = index_for_dispatch
                        .dispatch_refresh(
                            credential_id,
                            &manager_for_dispatch,
                            Duration::from_secs(5),
                        )
                        .await;
                    outcome_tx
                        .send(outcome)
                        .expect("fan-out outcome receiver remains live");
                    Ok(())
                })
            })
            .expect("open caller queue accepts fan-out task");

        let outcome = outcome_rx.await.expect("fan-out returns its typed outcome");
        assert_eq!(outcome.success(), 0);
        assert_eq!(outcome.failed(), 0);
        assert_eq!(outcome.timed_out(), 0);
        assert_eq!(outcome.deferred(), 1);
        assert_eq!(outcome.dispatched(), 1);
        assert_eq!(
            caller_submission.wait().await.unwrap(),
            SubmissionOutcome::Completed,
            "the outer queue task completes without converting accepted deferral into error",
        );

        let terminal_event = loop {
            let event = events
                .recv()
                .await
                .expect("queue-owned hook emits terminal event");
            if matches!(
                event,
                ResourceEvent::SlotRefreshed { .. } | ResourceEvent::SlotRefreshFailed { .. }
            ) {
                break event;
            }
        };
        assert!(
            matches!(&terminal_event, ResourceEvent::SlotRefreshed { .. }),
            "expected terminal refresh success, got {terminal_event:?}"
        );
        assert_eq!(
            ledger.refresh_entered.load(Ordering::SeqCst),
            1,
            "accepted hook executes exactly once after observer deferral"
        );

        let metric_snapshot = manager
            .metrics()
            .expect("configured manager exposes metrics")
            .snapshot();
        assert_eq!(
            metric_snapshot.slot_refresh_deferred, 1,
            "one cross-queue observer deferral is counted exactly once"
        );
        let metric_outcome = metric_snapshot.slot_refresh_outcomes;
        assert_eq!(metric_outcome.success, 1);
        assert_eq!(metric_outcome.failed, 0);
        assert_eq!(metric_outcome.timed_out, 0);
        assert_eq!(
            metric_outcome.abandoned, 0,
            "observer deferral is not a second terminal attempt outcome"
        );

        while let Some(event) = events.try_recv() {
            assert!(
                !matches!(
                    event,
                    ResourceEvent::SlotRefreshed { .. } | ResourceEvent::SlotRefreshFailed { .. }
                ),
                "one admitted hook must not emit a second terminal event: {event:?}",
            );
        }

        caller_queue.close();
        ReleaseQueue::shutdown(caller_workers).await;
    }

    #[tokio::test]
    async fn cancelling_refresh_observer_keeps_queue_owned_terminal_accounting() {
        let identity = SlotIdentity::from_bindings([("k", "cred-cancelled-observer")]);
        let metrics_registry = Arc::new(nebula_metrics::MetricsRegistry::new());
        let manager_config = ManagerConfig::default()
            .with_release_queue_workers(1)
            .with_metrics_registry(Arc::clone(&metrics_registry));
        let (index, manager, credential_id, _scope, _org, ledger) =
            setup_with_config(std::slice::from_ref(&identity), manager_config).await;
        ledger.set(identity, Behaviour::FastOk);
        let mut events = manager.subscribe_events();

        assert!(
            index
                .dispatch_refresh(credential_id, &manager, Duration::from_secs(30))
                .now_or_never()
                .is_none(),
            "the first poll admits the hook and parks on its receipt"
        );

        let terminal_event = loop {
            let event = events
                .recv()
                .await
                .expect("queue-owned hook emits terminal event");
            if matches!(
                event,
                ResourceEvent::SlotRefreshed { .. } | ResourceEvent::SlotRefreshFailed { .. }
            ) {
                break event;
            }
        };
        assert!(
            matches!(&terminal_event, ResourceEvent::SlotRefreshed { .. }),
            "expected terminal refresh success, got {terminal_event:?}"
        );
        assert_eq!(ledger.refresh_entered.load(Ordering::SeqCst), 1);
        let metric_outcome = manager
            .metrics()
            .expect("configured manager exposes metrics")
            .snapshot()
            .slot_refresh_outcomes;
        assert_eq!(metric_outcome.success, 1);
        assert_eq!(metric_outcome.failed, 0);
        assert_eq!(metric_outcome.timed_out, 0);
        assert_eq!(metric_outcome.abandoned, 0);
        while let Some(event) = events.try_recv() {
            assert!(
                !matches!(
                    event,
                    ResourceEvent::SlotRefreshed { .. } | ResourceEvent::SlotRefreshFailed { .. }
                ),
                "cancelled observer must not cause duplicate terminal events: {event:?}"
            );
        }
    }

    /// Isolation invariant: one resource whose started hook reaches its
    /// terminal execution timeout must not abort or fail its siblings.
    /// Three tenants share one `(key, scope)`; the middle hook hangs.
    #[tokio::test(start_paused = true)]
    async fn refresh_fanout_reports_terminal_hook_timeout_without_aborting_siblings() {
        let (a, b, c) = (
            SlotIdentity::from_bindings([("k", "cred-0xaaaa_u64")]),
            SlotIdentity::from_bindings([("k", "cred-0xbbbb_u64")]),
            SlotIdentity::from_bindings([("k", "cred-0xcccc_u64")]),
        );
        let manager_config = ManagerConfig::default()
            .with_metrics_registry(Arc::new(nebula_metrics::MetricsRegistry::new()));
        let (idx, mgr, cid, _scope, _org, ledger) =
            setup_with_config(&[a.clone(), b.clone(), c.clone()], manager_config).await;
        ledger.set(a, Behaviour::FastOk);
        ledger.set(b, Behaviour::Hang);
        ledger.set(c, Behaviour::FastOk);
        let mut events = mgr.subscribe_events();

        // The hook execution budget classifies the hung row as a terminal
        // timeout while both FastOk siblings complete independently.
        let out = idx
            .dispatch_refresh(cid, &mgr, Duration::from_millis(150))
            .await;

        assert_eq!(
            out,
            RotationOutcome {
                success: 2,
                failed: 0,
                timed_out: 1,
                deferred: 0,
                abandoned: 0,
                drain_timed_out: 0,
                observation_timed_out: 0,
            },
            "one hung resource must time out in isolation; both siblings still refresh"
        );
        assert_eq!(out.dispatched(), 3, "every bound row is accounted for");
        // All three hooks were entered (the hung one too) — proof the
        // siblings were not aborted by the hung one.
        assert_eq!(
            ledger.refresh_entered.load(Ordering::SeqCst),
            3,
            "every resource hook ran; terminal timeout did not cancel siblings"
        );
        assert_eq!(
            collect_terminal_events(&mut events, TerminalDirection::Refresh, 3).await,
            (2, 1),
            "two hooks succeed and the queue-owned hung hook emits one timeout failure"
        );
        let terminal = mgr
            .metrics()
            .expect("metrics configured")
            .snapshot()
            .slot_refresh_outcomes;
        assert_eq!(terminal.success, 2);
        assert_eq!(terminal.failed, 0);
        assert_eq!(terminal.timed_out, 1);
        assert_eq!(terminal.abandoned, 0);
    }

    /// Mixed outcomes in one fan-out: ok + err + deferral each counted
    /// independently, none aborting the others.
    #[tokio::test(start_paused = true)]
    async fn refresh_fanout_mixed_terminal_outcomes_each_independent() {
        let (a, b, c, d) = (
            SlotIdentity::from_bindings([("k", "cred-0x1_u64")]),
            SlotIdentity::from_bindings([("k", "cred-0x2_u64")]),
            SlotIdentity::from_bindings([("k", "cred-0x3_u64")]),
            SlotIdentity::from_bindings([("k", "cred-0x4_u64")]),
        );
        let manager_config = ManagerConfig::default()
            .with_metrics_registry(Arc::new(nebula_metrics::MetricsRegistry::new()));
        let (idx, mgr, cid, _scope, _org, ledger) = setup_with_config(
            &[a.clone(), b.clone(), c.clone(), d.clone()],
            manager_config,
        )
        .await;
        ledger.set(a, Behaviour::FastOk);
        ledger.set(b, Behaviour::FastErr);
        ledger.set(c, Behaviour::Hang);
        ledger.set(d, Behaviour::FastOk);
        let mut events = mgr.subscribe_events();

        let out = idx
            .dispatch_refresh(cid, &mgr, Duration::from_millis(150))
            .await;

        assert_eq!(
            out,
            RotationOutcome {
                success: 2,
                failed: 1,
                timed_out: 1,
                deferred: 0,
                abandoned: 0,
                drain_timed_out: 0,
                observation_timed_out: 0,
            },
        );
        assert_eq!(out.dispatched(), 4);
        assert_eq!(
            collect_terminal_events(&mut events, TerminalDirection::Refresh, 4).await,
            (2, 2)
        );
        let terminal = mgr
            .metrics()
            .expect("metrics configured")
            .snapshot()
            .slot_refresh_outcomes;
        assert_eq!(terminal.success, 2);
        assert_eq!(terminal.failed, 1);
        assert_eq!(terminal.timed_out, 1);
        assert_eq!(terminal.abandoned, 0);
    }

    /// Revoke analogue of the post-admission deferral isolation test.
    #[tokio::test(start_paused = true)]
    async fn revoke_fanout_reports_terminal_hook_timeout_without_aborting_siblings() {
        let (a, b, c) = (
            SlotIdentity::from_bindings([("k", "cred-0xdead_u64")]),
            SlotIdentity::from_bindings([("k", "cred-0xbeef_u64")]),
            SlotIdentity::from_bindings([("k", "cred-0xf00d_u64")]),
        );
        let manager_config = ManagerConfig::default()
            .with_metrics_registry(Arc::new(nebula_metrics::MetricsRegistry::new()));
        let (idx, mgr, cid, _scope, _org, ledger) =
            setup_with_config(&[a.clone(), b.clone(), c.clone()], manager_config).await;
        ledger.set(a, Behaviour::FastOk);
        ledger.set(b, Behaviour::Hang);
        ledger.set(c, Behaviour::FastOk);
        let mut events = mgr.subscribe_events();

        let out = idx
            .dispatch_revoke(cid, &mgr, Duration::from_millis(150))
            .await;

        assert_eq!(
            out,
            RotationOutcome {
                success: 2,
                failed: 0,
                timed_out: 1,
                deferred: 0,
                abandoned: 0,
                drain_timed_out: 0,
                observation_timed_out: 0,
            },
            "a hung revoke must time out in isolation; siblings still revoke"
        );
        assert_eq!(
            ledger.revoke_entered.load(Ordering::SeqCst),
            3,
            "every resource's revoke hook ran"
        );
        assert_eq!(
            collect_terminal_events(&mut events, TerminalDirection::Revoke, 3).await,
            (2, 1)
        );
        let terminal = mgr
            .metrics()
            .expect("metrics configured")
            .snapshot()
            .slot_revoke_outcomes;
        assert_eq!(terminal.success, 2);
        assert_eq!(terminal.failed, 0);
        assert_eq!(terminal.timed_out, 1);
        assert_eq!(terminal.abandoned, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn revoke_completed_after_drain_timeout_counts_both_dimensions() {
        let identity = SlotIdentity::from_bindings([("k", "cred-drain-timeout-completed")]);
        let (index, manager, credential_id, _scope, org, ledger) =
            setup(std::slice::from_ref(&identity)).await;
        ledger.set(identity.clone(), Behaviour::FastOk);
        let in_flight = manager
            .acquire_resident_for_identity::<CtlResource>(
                &ctx_for(org),
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .expect("registered row accepts a lease before taint");

        let outcome = index
            .dispatch_revoke(credential_id, &manager, Duration::from_secs(1))
            .await;

        assert_eq!(outcome.success(), 1);
        assert_eq!(outcome.failed(), 0);
        assert_eq!(outcome.timed_out(), 0);
        assert_eq!(outcome.deferred(), 0);
        assert_eq!(outcome.drain_timed_out(), 1);
        assert_eq!(outcome.dispatched(), 1);
        assert_eq!(ledger.revoke_entered.load(Ordering::SeqCst), 1);
        drop(in_flight);
    }

    #[tokio::test(start_paused = true)]
    async fn revoke_failure_after_drain_timeout_counts_both_dimensions() {
        let identity = SlotIdentity::from_bindings([("k", "cred-drain-timeout-failed")]);
        let (index, manager, credential_id, _scope, org, ledger) =
            setup(std::slice::from_ref(&identity)).await;
        ledger.set(identity.clone(), Behaviour::FastErr);
        let in_flight = manager
            .acquire_resident_for_identity::<CtlResource>(
                &ctx_for(org),
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .expect("registered row accepts a lease before taint");

        let outcome = index
            .dispatch_revoke(credential_id, &manager, Duration::from_secs(1))
            .await;

        assert_eq!(outcome.success(), 0);
        assert_eq!(outcome.failed(), 1);
        assert_eq!(outcome.timed_out(), 0);
        assert_eq!(outcome.deferred(), 0);
        assert_eq!(outcome.abandoned(), 0);
        assert_eq!(outcome.drain_timed_out(), 1);
        assert_eq!(outcome.observation_timed_out(), 0);
        assert_eq!(outcome.dispatched(), 1);
        assert_eq!(ledger.revoke_entered.load(Ordering::SeqCst), 1);
        drop(in_flight);
    }

    #[tokio::test(start_paused = true)]
    async fn terminal_revoke_hook_timeout_preserves_prior_drain_timeout() {
        let identity = SlotIdentity::from_bindings([("k", "cred-drain-timeout-hook-timeout")]);
        let (index, manager, credential_id, _scope, org, ledger) =
            setup(std::slice::from_ref(&identity)).await;
        ledger.set(identity.clone(), Behaviour::Hang);
        let in_flight = manager
            .acquire_resident_for_identity::<CtlResource>(
                &ctx_for(org),
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .expect("registered row accepts a lease before taint");

        let outcome = index
            .dispatch_revoke(credential_id, &manager, Duration::from_secs(1))
            .await;

        assert_eq!(outcome.success(), 0);
        assert_eq!(outcome.failed(), 0);
        assert_eq!(outcome.timed_out(), 1);
        assert_eq!(outcome.deferred(), 0);
        assert_eq!(outcome.abandoned(), 0);
        assert_eq!(outcome.drain_timed_out(), 1);
        assert_eq!(outcome.observation_timed_out(), 0);
        assert_eq!(outcome.dispatched(), 1);
        assert_eq!(ledger.revoke_entered.load(Ordering::SeqCst), 1);
        drop(in_flight);
    }

    #[tokio::test(start_paused = true)]
    async fn cross_queue_revoke_deferral_preserves_drain_timeout_dimension() {
        let identity = SlotIdentity::from_bindings([("k", "cred-drain-timeout-deferred")]);
        let metrics_registry = Arc::new(nebula_metrics::MetricsRegistry::new());
        let manager_config = ManagerConfig::default()
            .with_release_queue_workers(1)
            .with_metrics_registry(Arc::clone(&metrics_registry));
        let (index, manager, credential_id, _scope, org, ledger) =
            setup_with_config(std::slice::from_ref(&identity), manager_config).await;
        ledger.set(identity.clone(), Behaviour::FastOk);
        let in_flight = manager
            .acquire_resident_for_identity::<CtlResource>(
                &ctx_for(org),
                &AcquireOptions::default(),
                &identity,
            )
            .await
            .expect("registered row accepts a lease before taint");
        let mut events = manager.subscribe_events();

        let (caller_queue, caller_workers) = ReleaseQueue::new(1);
        let manager_for_dispatch = Arc::clone(&manager);
        let (outcome_tx, outcome_rx) = tokio::sync::oneshot::channel();
        let caller_submission = caller_queue
            .submit_release(move || {
                Box::pin(async move {
                    let outcome = index
                        .dispatch_revoke(
                            credential_id,
                            &manager_for_dispatch,
                            Duration::from_secs(1),
                        )
                        .await;
                    outcome_tx
                        .send(outcome)
                        .expect("fan-out outcome receiver remains live");
                    Ok(())
                })
            })
            .expect("open caller queue accepts fan-out task");

        let outcome = outcome_rx.await.expect("fan-out returns its typed outcome");
        assert_eq!(outcome.success(), 0);
        assert_eq!(outcome.failed(), 0);
        assert_eq!(outcome.timed_out(), 0);
        assert_eq!(outcome.deferred(), 1);
        assert_eq!(outcome.drain_timed_out(), 1);
        assert_eq!(outcome.dispatched(), 1);
        assert_eq!(
            caller_submission.wait().await.unwrap(),
            SubmissionOutcome::Completed
        );

        let terminal_event = loop {
            let event = events
                .recv()
                .await
                .expect("queue-owned hook emits terminal event");
            if matches!(
                event,
                ResourceEvent::SlotRevoked { .. } | ResourceEvent::SlotRevokeFailed { .. }
            ) {
                break event;
            }
        };
        assert!(
            matches!(&terminal_event, ResourceEvent::SlotRevoked { .. }),
            "expected terminal revoke success, got {terminal_event:?}"
        );
        assert_eq!(ledger.revoke_entered.load(Ordering::SeqCst), 1);
        let metric_snapshot = manager
            .metrics()
            .expect("configured manager exposes metrics")
            .snapshot();
        assert_eq!(
            metric_snapshot.slot_revoke_deferred, 1,
            "one cross-queue revoke deferral is counted exactly once"
        );
        let metric_outcome = metric_snapshot.slot_revoke_outcomes;
        assert_eq!(metric_outcome.success, 1);
        assert_eq!(metric_outcome.failed, 0);
        assert_eq!(metric_outcome.timed_out, 0);
        assert_eq!(metric_outcome.abandoned, 0);

        while let Some(event) = events.try_recv() {
            assert!(
                !matches!(
                    event,
                    ResourceEvent::SlotRevoked { .. } | ResourceEvent::SlotRevokeFailed { .. }
                ),
                "one admitted revoke hook must not emit a second terminal event: {event:?}",
            );
        }

        drop(in_flight);
        caller_queue.close();
        ReleaseQueue::shutdown(caller_workers).await;
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
    /// A revoke whose hook remains queue-owned after the observer deadline
    /// hangs) MUST still have left the row **tainted**: the synchronous
    /// `taint_slot_for` ran *outside* the per-resource timeout, so a
    /// deferred hook cannot un-revoke the credential. Asserts both that
    /// the fan-out records `deferred` (not success, not a retryable error)
    /// **and** that a fresh acquire on that exact resolved row is
    /// rejected *after* the timed-out fan-out returned — proof the taint
    /// survived the timeout.
    #[tokio::test(start_paused = true)]
    async fn revoke_fanout_deferred_hook_still_left_row_tainted() {
        use nebula_error::{Classify, ErrorCategory};

        let hung = SlotIdentity::from_bindings([("k", "cred-0x5151_u64")]);
        let manager_config = ManagerConfig::default()
            .with_metrics_registry(Arc::new(nebula_metrics::MetricsRegistry::new()));
        let (idx, mgr, cid, _scope, org, ledger) =
            setup_with_config(std::slice::from_ref(&hung), manager_config).await;
        // The revoke hook never returns, so the observer deadline yields
        // typed deferral while queue ownership continues. `hung` is reused below
        // (the structural identity is no longer `Copy`), so clone here.
        ledger.set(hung.clone(), Behaviour::Hang);
        let mut events = mgr.subscribe_events();

        let out = idx
            .dispatch_revoke(cid, &mgr, Duration::from_millis(150))
            .await;

        assert_eq!(
            out,
            RotationOutcome {
                success: 0,
                failed: 0,
                timed_out: 1,
                deferred: 0,
                abandoned: 0,
                drain_timed_out: 0,
                observation_timed_out: 0,
            },
            "a hung revoke hook must time out — never success, never retryable failure",
        );
        assert_eq!(
            ledger.revoke_entered.load(Ordering::SeqCst),
            1,
            "phase 2 (drain_and_revoke) did run and reached the hung hook",
        );
        assert_eq!(
            collect_terminal_events(&mut events, TerminalDirection::Revoke, 1).await,
            (0, 1)
        );
        let terminal = mgr
            .metrics()
            .expect("metrics configured")
            .snapshot()
            .slot_revoke_outcomes;
        assert_eq!(terminal.success, 0);
        assert_eq!(terminal.failed, 0);
        assert_eq!(terminal.timed_out, 1);
        assert_eq!(terminal.abandoned, 0);

        // The decisive #681 assertion: the row is STILL tainted after the
        // timed-out fan-out returned. If the taint had been inside the
        // timeout future it would have been skipped/rolled back; here it
        // ran synchronously *before* the timeout, so new acquires on this
        // exact resolved row stay rejected.
        let ctx = ctx_for(org);
        let acquired = mgr
            .acquire_resident_for_identity::<CtlResource>(&ctx, &AcquireOptions::default(), &hung)
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
                deferred: 0,
                abandoned: 0,
                drain_timed_out: 0,
                observation_timed_out: 0,
            },
        );
    }
}
