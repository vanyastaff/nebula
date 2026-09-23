//! Finding #2 — `graceful_shutdown`-vs-acquire use-after-drain.
//!
//! `lookup()` runs `shutdown_guard()` (Defense A) *before* the
//! `InFlightCounter::new()` increment. An acquire that passes `lookup()`
//! while `shutting_down == false`, then has its increment land *after*
//! `wait_for_drain` already observed `0` and `registry.clear()` ran, is
//! a logical use-after-drain: the post-`InFlightCounter::new()` re-check
//! (`reject_if_tainted_post_count`) only observed taint, never
//! `shutting_down`, so the acquire completed and a `ResourceGuard` was
//! handed out for a resource the manager had already drained and cleared.
//!
//! This is structurally identical to the revoke path, which *is* closed
//! by a symmetric taint pre-check + post-count re-check. The shutdown
//! path had the pre-check (`lookup`'s `shutdown_guard`) but no symmetric
//! post-count re-check.
//!
//! The race window (`lookup` → `InFlightCounter::new`) has no `.await`,
//! so this test reproduces the interleave deterministically by splitting
//! it at exactly that seam: resolve the managed row via the same private
//! lookup `acquire_resident` uses (while `shutting_down == false`), then
//! run `graceful_shutdown`'s Phase 1–3 (signal + drain-sees-`0` because
//! the counter increment has not happened yet + `registry.clear()`),
//! then drive the private post-lookup tail (`run_acquire`) with that
//! resolved row. Pre-fix the tail succeeds and hands out a guard for a
//! cleared registry; post-fix it must reject with `Cancelled`.

use std::{sync::Arc, time::Duration};

use nebula_core::{ExecutionId, ResourceKey, WorkspaceId, resource_key, scope::Scope};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::{
    TopologyTag,
    context::ResourceContext,
    error::ErrorKind,
    options::AcquireOptions,
    resource::{ResourceConfig, ResourceMetadataDraft},
    topology::{Resident, resident::config::Config as ResidentConfig},
};

#[derive(Clone, Default, nebula_schema::Schema)]
struct RaceCfg;

impl ResourceConfig for RaceCfg {
    fn fingerprint(&self) -> u64 {
        // Unit struct: all instances identical — constant 0 is correct.
        0
    }
}

#[derive(Clone)]
struct ShutdownRaceResident;

#[async_trait::async_trait]
impl Provider for ShutdownRaceResident {
    type Config = RaceCfg;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test.shutdown_post_count_race.resident")
    }

    async fn create(&self, _config: &RaceCfg, _ctx: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            crate::metadata_name!("test.shutdown_post_count_race.resident"),
            "",
        )
    }
}

crate::no_credential_slots!(ShutdownRaceResident);

impl crate::topology::ResidentProvider for ShutdownRaceResident {
    fn is_alive_sync(&self, _runtime: &()) -> bool {
        true
    }
}

fn ctx() -> ResourceContext {
    let scope = Scope {
        execution_id: Some(ExecutionId::new()),
        ..Default::default()
    };
    ResourceContext::minimal(scope, CancellationToken::new())
}

fn register_race_resident(manager: &Manager, topology: Resident<ShutdownRaceResident>) {
    let spec = RegistrationSpec {
        resource: ShutdownRaceResident,
        config: RaceCfg,
        scope: ScopeLevel::Global,
        slot_identity: crate::dedup::SlotIdentity::Unbound,
        topology,
        recovery_gate: None,
        rate_limit: None,
    };
    assert!(manager.register(spec).is_ok(), "register succeeds");
}

#[tokio::test]
async fn registration_after_shutdown_cannot_repopulate_registry() {
    let manager = Manager::new();
    manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("empty shutdown");
    let result = manager.register(RegistrationSpec {
        resource: ShutdownRaceResident,
        config: RaceCfg,
        scope: ScopeLevel::Global,
        slot_identity: crate::dedup::SlotIdentity::Unbound,
        topology: Resident::new(ResidentConfig::default()),
        recovery_gate: None,
        rate_limit: None,
    });
    assert_eq!(
        result.expect_err("closed admission").kind(),
        &ErrorKind::Cancelled
    );
    assert!(!manager.contains(&ShutdownRaceResident::key()));
}

#[derive(Clone)]
struct BlockingValidation {
    entered: Arc<std::sync::Barrier>,
    resume: Arc<std::sync::Barrier>,
}

// This typed-only validation race fixture carries synchronization state,
// not JSON configuration. It deliberately has no Deserialize implementation.
impl nebula_schema::HasSchema for BlockingValidation {
    fn schema() -> Result<nebula_schema::ValidSchema, nebula_schema::ValidationReport> {
        Ok(nebula_schema::ValidSchema::empty())
    }
}

impl ResourceConfig for BlockingValidation {
    fn fingerprint(&self) -> u64 {
        0
    }

    fn validate(&self) -> Result<(), Error> {
        self.entered.wait();
        self.resume.wait();
        Ok(())
    }
}

struct ValidatingResident;

#[async_trait::async_trait]
impl Provider for ValidatingResident {
    type Config = BlockingValidation;
    type Instance = ();
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        resource_key!("test.registration_validation_race")
    }
    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            crate::metadata_name!("test.registration_validation_race"),
            "",
        )
    }
    async fn create(&self, _: &Self::Config, _: &ResourceContext) -> Result<(), Error> {
        Ok(())
    }
}

crate::no_credential_slots!(ValidatingResident);

impl crate::topology::ResidentProvider for ValidatingResident {
    fn is_alive_sync(&self, (): &()) -> bool {
        true
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registration_validation_cannot_commit_after_shutdown_snapshot() {
    let manager = Arc::new(Manager::new());
    let entered = Arc::new(std::sync::Barrier::new(2));
    let resume = Arc::new(std::sync::Barrier::new(2));
    let registering = Arc::clone(&manager);
    let config = BlockingValidation {
        entered: Arc::clone(&entered),
        resume: Arc::clone(&resume),
    };
    let registration = std::thread::spawn(move || {
        registering.register(RegistrationSpec {
            resource: ValidatingResident,
            config,
            scope: ScopeLevel::Global,
            slot_identity: crate::dedup::SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: None,
        })
    });
    entered.wait();
    // This must finish while author validation is blocked: author code
    // cannot hold admission, and its later commit must recheck shutdown.
    let shutdown = manager.graceful_shutdown(ShutdownConfig::default()).await;
    resume.wait();
    let result = registration.join().expect("registration thread");
    assert!(shutdown.expect("empty snapshot shutdown").registry_cleared);
    assert_eq!(
        result.expect_err("final commit fenced").kind(),
        &ErrorKind::Cancelled
    );
    assert!(!manager.contains(&ValidatingResident::key()));
}

#[tokio::test]
async fn replacement_and_removal_fence_previously_resolved_rows() {
    let manager = Manager::new();
    register_race_resident(&manager, Resident::new(ResidentConfig::default()));
    let first = manager
        .lookup::<ShutdownRaceResident>(&ScopeLevel::Global)
        .expect("first row");
    register_race_resident(&manager, Resident::new(ResidentConfig::default()));
    let second = manager
        .lookup::<ShutdownRaceResident>(&ScopeLevel::Global)
        .expect("replacement");
    assert!(!Arc::ptr_eq(&first, &second));
    assert!(
        first.store.is_closed(),
        "replacement fences a captured old row"
    );
    assert!(!second.store.is_closed());
    manager
        .remove(&ShutdownRaceResident::key())
        .expect("remove replacement");
    assert!(second.store.is_closed(), "removal fences a captured row");
    assert!(!manager.contains(&ShutdownRaceResident::key()));
    manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("retirement cleanup");
}

#[tokio::test]
async fn saturated_retirement_queue_leaves_registry_ownership_unchanged() {
    let manager = Manager::with_config(ManagerConfig::default().with_retirement_queue_capacity(1));
    register_race_resident(&manager, Resident::new(ResidentConfig::default()));
    let original = manager
        .lookup::<ShutdownRaceResident>(&ScopeLevel::Global)
        .expect("original row");
    let held_capacity = manager
        .retirement_supervisor
        .try_reserve()
        .expect("sole retirement slot");

    let new_scope = ScopeLevel::Workspace(WorkspaceId::new());
    manager
        .register(RegistrationSpec {
            resource: ShutdownRaceResident,
            config: RaceCfg,
            scope: new_scope.clone(),
            slot_identity: crate::dedup::SlotIdentity::Unbound,
            topology: Resident::new(ResidentConfig::default()),
            recovery_gate: None,
            rate_limit: None,
        })
        .expect("brand-new row does not consume retirement capacity");
    assert_eq!(
        manager
            .remove(&resource_key!("test.retirement.missing"))
            .expect_err("missing whole-key removal must win over queue saturation")
            .kind(),
        &ErrorKind::NotFound
    );
    assert_eq!(
        manager
            .remove_for(
                &ShutdownRaceResident::key(),
                &ScopeLevel::Workflow(nebula_core::WorkflowId::new()),
                &crate::dedup::SlotIdentity::Unbound,
            )
            .expect_err("missing row removal must win over queue saturation")
            .kind(),
        &ErrorKind::NotFound
    );

    let replacement = manager.register(RegistrationSpec {
        resource: ShutdownRaceResident,
        config: RaceCfg,
        scope: ScopeLevel::Global,
        slot_identity: crate::dedup::SlotIdentity::Unbound,
        topology: Resident::new(ResidentConfig::default()),
        recovery_gate: None,
        rate_limit: None,
    });
    assert_eq!(
        replacement
            .expect_err("replacement must reject before mutation")
            .kind(),
        &ErrorKind::Backpressure
    );
    let after_replacement = manager
        .lookup::<ShutdownRaceResident>(&ScopeLevel::Global)
        .expect("original row remains registered");
    assert!(Arc::ptr_eq(&original, &after_replacement));
    assert!(!original.store.is_closed());

    let removal = manager.remove(&ShutdownRaceResident::key());
    assert_eq!(
        removal
            .expect_err("removal must reject before mutation")
            .kind(),
        &ErrorKind::Backpressure
    );
    assert!(manager.contains(&ShutdownRaceResident::key()));
    assert!(!original.store.is_closed());

    drop(held_capacity);
    let workspace_a = ScopeLevel::Workspace(WorkspaceId::new());
    let workspace_b = ScopeLevel::Workspace(WorkspaceId::new());
    for scope in [workspace_a.clone(), workspace_b.clone()] {
        manager
            .register(RegistrationSpec {
                resource: ShutdownRaceResident,
                config: RaceCfg,
                scope,
                slot_identity: crate::dedup::SlotIdentity::Unbound,
                topology: Resident::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("additional row exceeds command capacity, not row capacity");
    }
    let workspace_a_row = manager
        .lookup::<ShutdownRaceResident>(&workspace_a)
        .expect("workspace A row");
    let workspace_b_row = manager
        .lookup::<ShutdownRaceResident>(&workspace_b)
        .expect("workspace B row");
    let new_scope_row = manager
        .lookup::<ShutdownRaceResident>(&new_scope)
        .expect("new row remains registered");
    manager
        .remove(&ShutdownRaceResident::key())
        .expect("one batch command retires more rows than queue capacity");
    assert!(!manager.contains(&ShutdownRaceResident::key()));
    for row in [
        &original,
        &new_scope_row,
        &workspace_a_row,
        &workspace_b_row,
    ] {
        assert!(row.store.is_closed(), "every batch owner is fenced");
    }
    manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("cleanup after releasing retirement capacity");
}

#[tokio::test]
async fn cancelled_shutdown_resumes_manager_owned_snapshot_publication() {
    let manager = Arc::new(Manager::with_config(
        ManagerConfig::default().with_retirement_queue_capacity(1),
    ));
    register_race_resident(&manager, Resident::new(ResidentConfig::default()));
    let row = manager
        .lookup::<ShutdownRaceResident>(&ScopeLevel::Global)
        .expect("registered row");
    let held_capacity = manager
        .retirement_supervisor
        .try_reserve()
        .expect("sole retirement slot");

    let shutdown_manager = Arc::clone(&manager);
    let caller = tokio::spawn(async move {
        shutdown_manager
            .graceful_shutdown(ShutdownConfig::default())
            .await
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        while manager.contains(&ShutdownRaceResident::key()) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("shutdown reaches its fenced final snapshot");
    assert!(row.store.is_closed(), "snapshot fences the retained row");

    caller.abort();
    let _ = caller.await;
    drop(held_capacity);

    let report = manager
        .graceful_shutdown(ShutdownConfig::default())
        .await
        .expect("retry awaits the same manager-owned terminal task");
    assert!(report.registry_cleared);
    assert!(report.release_queue_drained);
    assert_eq!(report.dropped_release_tasks, 0);
}

#[tokio::test]
async fn publication_timeout_abandons_snapshot_and_closes_terminal_workers() {
    let manager = Manager::with_config(ManagerConfig::default().with_retirement_queue_capacity(1));
    register_race_resident(&manager, Resident::new(ResidentConfig::default()));
    let row = manager
        .lookup::<ShutdownRaceResident>(&ScopeLevel::Global)
        .expect("registered row");
    let held_capacity = manager
        .retirement_supervisor
        .try_reserve()
        .expect("sole retirement slot");

    let error = manager
        .graceful_shutdown(
            ShutdownConfig::default().with_release_queue_timeout(Duration::from_millis(20)),
        )
        .await
        .expect_err("bounded publication must surface its timeout");
    assert!(matches!(error, ShutdownError::ReleaseQueueTimeout { .. }));
    assert!(row.store.is_closed(), "timed-out snapshot remains fenced");
    assert_eq!(
        manager.retirement_tracker.0.load(AtomicOrdering::Acquire),
        0,
        "abandoned snapshot settles terminal ownership"
    );
    assert!(manager.release_queue_handle.lock().await.is_none());
    manager
        .release_queue
        .submit(|| panic!("closed queue must not invoke a late factory"));
    assert_eq!(manager.release_queue.dropped_count(), 1);
    drop(held_capacity);
}

#[tokio::test(start_paused = true)]
async fn rejected_retirement_aborts_maintenance_holding_its_own_row() {
    let manager = Manager::new();
    register_race_resident(&manager, Resident::new(ResidentConfig::default()));
    let row = manager
        .lookup::<ShutdownRaceResident>(&ScopeLevel::Global)
        .expect("registered row");
    let weak = Arc::downgrade(&row);
    let task_row = Arc::clone(&row);
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (alive_tx, alive_rx) = tokio::sync::oneshot::channel::<()>();
    row.maintenance.set_task(tokio::spawn(async move {
        // Real maintenance likewise upgrades its Weak for a whole sweep.
        let _row = task_row;
        let _alive = alive_tx;
        started_tx.send(()).expect("start receiver");
        std::future::pending::<()>().await;
    }));
    started_rx.await.expect("maintenance owns the row");
    manager.release_queue.close();
    drop(row);
    drop(manager);
    let stopped = tokio::time::timeout(Duration::from_secs(1), alive_rx).await;
    assert!(
        stopped.is_ok(),
        "an unpolled rejected retirement must abort its maintenance task"
    );
    assert!(
        weak.upgrade().is_none(),
        "maintenance must not retain its own row after abandonment"
    );
}

/// Runs the resident acquire through the framework loop — the same
/// monomorphic dispatch `run_acquire_dispatch` performs.
async fn race_resident_acquire(
    managed: &Arc<ManagedResource<ShutdownRaceResident>>,
    ctx: &ResourceContext,
) -> Result<crate::guard::ResourceGuard<ShutdownRaceResident>, Error> {
    managed
        .run_acquire_loop(ctx, &AcquireOptions::default(), None)
        .await
}

/// Deterministic reproduction of the use-after-drain. The acquire
/// resolves its row *before* shutdown (Defense A passes), shutdown then
/// drains (sees `0` because the acquire has not yet hit
/// `InFlightCounter::new()`) and clears the registry, and only *then*
/// does the post-lookup acquire tail run. The tail must reject — the
/// caller must NOT receive a `ResourceGuard` for a drained-and-cleared
/// resource.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_acquire_rejects_when_drain_completed_after_lookup_passed() {
    let manager = Manager::new();
    let resident_rt = Resident::<ShutdownRaceResident>::new(ResidentConfig::default());
    register_race_resident(&manager, resident_rt);

    let acquire_ctx = ctx();

    // Step 1: the acquire passes `lookup()` (Defense A) while
    // `shutting_down == false`. This is the same private resolution
    // `acquire_resident` performs before `run_acquire`.
    let managed = manager
        .lookup_for_acquire_scope::<ShutdownRaceResident>(&acquire_ctx)
        .expect("lookup must succeed before shutdown starts");

    // Step 2: `graceful_shutdown` Phase 1–3 run *now*, while the
    // resolved-but-not-yet-counted acquire is parked between `lookup()`
    // and `InFlightCounter::new()`. The drain observes `0` (the
    // increment has not happened) and the registry is cleared.
    manager.shutting_down.store(true, AtomicOrdering::Release);
    manager.cancel.cancel();
    manager
        .wait_for_drain(Duration::from_secs(5))
        .await
        .expect("drain sees 0 — the racing acquire has not incremented yet");
    manager.registry.clear();

    // Step 3: only now does the post-lookup acquire tail run. Its
    // `InFlightCounter::new()` increment lands *after* the drain saw
    // `0` and the registry was cleared. The post-count re-check is the
    // last line of defense; it must reject.
    let result = manager
        .run_acquire(
            Arc::clone(&managed),
            &acquire_ctx,
            &AcquireOptions::default(),
            || {
                let managed = Arc::clone(&managed);
                let ctx = &acquire_ctx;
                async move { race_resident_acquire(&managed, ctx).await }
            },
        )
        .await;

    match result {
        Err(e) if matches!(e.kind(), ErrorKind::Cancelled) => {
            // Correct: the acquire whose counter landed after the drain
            // completed is rejected — no guard for a cleared registry.
        },
        Ok(guard) => panic!(
            "use-after-drain: run_acquire handed out a {:?} guard for a \
             resource whose drain completed and registry was cleared \
             before the in-flight increment landed",
            guard.topology_tag()
        ),
        Err(other) => {
            panic!("expected Cancelled (post-count shutdown re-check), got {other:?}")
        },
    }

    // The drained guard must not leave a leaked in-flight count behind.
    assert_eq!(
        manager.drain_tracker.0.load(AtomicOrdering::Acquire),
        0,
        "rejected acquire must not leak a manager-wide in-flight count"
    );
}

/// Sanity twin: when shutdown has *not* started, the identical
/// post-lookup tail succeeds and hands out a real resident guard. This
/// pins that the fix rejects *only* the drained-after-lookup race, not
/// every acquire (no false-positive regression of the happy path).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_acquire_still_succeeds_when_not_shutting_down() {
    let manager = Manager::new();
    let resident_rt = Resident::<ShutdownRaceResident>::new(ResidentConfig::default());
    register_race_resident(&manager, resident_rt);

    let acquire_ctx = ctx();
    let managed = manager
        .lookup_for_acquire_scope::<ShutdownRaceResident>(&acquire_ctx)
        .expect("lookup succeeds");

    let result = manager
        .run_acquire(
            Arc::clone(&managed),
            &acquire_ctx,
            &AcquireOptions::default(),
            || {
                let managed = Arc::clone(&managed);
                let ctx = &acquire_ctx;
                async move { race_resident_acquire(&managed, ctx).await }
            },
        )
        .await;

    let guard = result.expect("acquire must succeed when not shutting down");
    assert_eq!(guard.topology_tag(), TopologyTag::Resident);
}
