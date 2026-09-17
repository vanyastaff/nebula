use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use chrono::TimeDelta;
use nebula_core::{ExecutablePlanRevisionId, WorkerFlavorRevisionId};
use nebula_storage_port::RevisionRecordBytes;
use tokio::sync::Barrier;

use super::*;
use crate::inmem::InMemoryExecutionStore;

#[derive(Debug)]
struct FixedRevisionClock {
    epoch_millis: AtomicI64,
}

impl FixedRevisionClock {
    fn new(now: DateTime<Utc>) -> Self {
        Self {
            epoch_millis: AtomicI64::new(now.timestamp_millis()),
        }
    }

    fn set(&self, now: DateTime<Utc>) {
        self.epoch_millis
            .store(now.timestamp_millis(), Ordering::SeqCst);
    }
}

impl RevisionClock for FixedRevisionClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.epoch_millis.load(Ordering::SeqCst))
            .unwrap_or_default()
    }
}

#[derive(Clone)]
struct RevisionCatalogTestDriver {
    inner: SharedState,
}

impl RevisionCatalogTestDriver {
    fn retain(
        &self,
        reference: RevisionReference,
    ) -> Result<RetainDecision, InternalRevisionError> {
        retain_exact_locked(&mut self.inner.lock(), reference)
    }

    fn transition(
        &self,
        transition: OwningReferenceTransition,
    ) -> Result<ReferenceDecision, InternalRevisionError> {
        transition_reference_locked(&mut self.inner.lock(), transition)
    }

    fn reference_rows(&self) -> usize {
        self.inner.lock().revision_catalog.references.len()
    }

    fn remove_worker_flavor_record(&self, worker_flavor_id: WorkerFlavorRevisionId) {
        self.inner
            .lock()
            .revision_catalog
            .worker_flavors
            .remove(&worker_flavor_id);
    }

    fn replace_plan_record(
        &self,
        plan_id: ExecutablePlanRevisionId,
        record: PlanFlavorRevisionRecord,
    ) {
        self.inner
            .lock()
            .revision_catalog
            .executable_plans
            .get_mut(&plan_id)
            .expect("fixture inserts the plan before replacing its stored record")
            .record = Some(record);
    }
}

struct Fixture {
    catalog: InMemoryPlanFlavorCatalog,
    driver: RevisionCatalogTestDriver,
    clock: Arc<FixedRevisionClock>,
    record: PlanFlavorRevisionRecord,
    ids: PlanFlavorRevisionIds,
}

impl Fixture {
    fn new() -> Self {
        let execution_store = InMemoryExecutionStore::new();
        let inner = execution_store.shared();
        let now = DateTime::from_timestamp(1_900_000_000, 0).unwrap_or_default();
        let clock = Arc::new(FixedRevisionClock::new(now));
        let catalog = InMemoryPlanFlavorCatalog::with_clock(Arc::clone(&inner), clock.clone());
        let plan_id = ExecutablePlanRevisionId::from_bytes([0x31; 32]);
        let worker_flavor_id = WorkerFlavorRevisionId::from_bytes([0x41; 32]);
        let worker_flavor = WorkerFlavorRevisionRecord::v1_json(
            worker_flavor_id,
            RevisionRecordBytes::try_from_vec(br#"{"flavor":"v1"}"#.to_vec())
                .expect("fixture flavor bytes are non-empty"),
        );
        let record = PlanFlavorRevisionRecord::graph_v1_json(
            plan_id,
            RevisionRecordBytes::try_from_vec(br#"{"plan":"v1"}"#.to_vec())
                .expect("fixture plan bytes are non-empty"),
            worker_flavor,
        );
        let ids = record.ids();
        Self {
            catalog,
            driver: RevisionCatalogTestDriver { inner },
            clock,
            record,
            ids,
        }
    }

    fn reference(&self) -> RevisionReference {
        RevisionReference::new(
            RevisionReferenceOwner::for_execution(ExecutionId::new()),
            ExecutionContractBundleId::new(),
            self.ids,
        )
    }

    async fn insert(&self) {
        assert_eq!(
            self.catalog.insert(&self.record).await,
            Ok(RevisionInsertOutcome::Inserted)
        );
    }
}

/// Red→green: re-installing the same revision through a differently
/// configured encoder is idempotent, not a permanent conflict.
///
/// Record bodies are plain `serde_json` output, so field order depends on
/// struct declaration order and on whether `serde_json` was built with
/// `preserve_order`. Comparing raw bytes turned that incidental difference
/// into `ContentConflict` for a revision both binaries agree on by content
/// address — an immutable plan that could never be installed again.
#[tokio::test]
async fn semantically_equal_json_record_is_already_present_not_a_conflict() {
    let fixture = Fixture::new();
    fixture.insert().await;

    // Same document emitted with incidental whitespace differences.
    let reencoded = PlanFlavorRevisionRecord::graph_v1_json(
        fixture.ids.plan(),
        RevisionRecordBytes::try_from_vec(br#"{  "plan"  :  "v1"  }"#.to_vec())
            .expect("re-encoded bytes are non-empty"),
        fixture.record.worker_flavor().clone(),
    );

    assert_eq!(
        fixture.catalog.insert(&reencoded).await,
        Ok(RevisionInsertOutcome::AlreadyPresent),
        "an encoding difference must not make an immutable revision uninstallable"
    );
    assert_eq!(
        fixture.catalog.load_exact(fixture.ids).await,
        Ok(fixture.record.clone()),
        "the originally stored bytes stay authoritative"
    );
}

/// Red→green: a draining revision reports `Draining` to every installer,
/// not only to one that has never inserted this plan.
///
/// The draining check used to be gated on the new-plan path, so a retrying
/// installer that had already stored the identical plan got
/// `AlreadyPresent` — reading as "installed and healthy" for a revision
/// being retired — while a caller arriving with a new plan against the same
/// flavor correctly got `Draining`. Whether the truth surfaced depended on
/// the caller's own history rather than on the catalog.
#[tokio::test]
async fn idempotent_reinsert_reports_draining_rather_than_already_present() {
    let fixture = Fixture::new();
    fixture.insert().await;
    assert_eq!(
        fixture.catalog.insert(&fixture.record).await,
        Ok(RevisionInsertOutcome::AlreadyPresent),
        "an ordinary retry against a live revision is still idempotent"
    );

    assert!(matches!(
        fixture
            .catalog
            .begin_drain(PlanFlavorRevisionTarget::WorkerFlavor(
                fixture.ids.worker_flavor()
            ))
            .await,
        Ok(BeginDrainOutcome::Started(_))
    ));

    assert_eq!(
        fixture.catalog.insert(&fixture.record).await,
        Err(RevisionCatalogError::Draining {
            target: PlanFlavorRevisionTarget::WorkerFlavor(fixture.ids.worker_flavor()),
        }),
        "once the flavor is draining, the same retry must report Draining"
    );
}

/// A draining executable plan is reportable through `insert` at all.
///
/// The plan's own lifecycle was never consulted — only the flavor's — so
/// `Draining` on the plan could not surface from this entry point.
#[tokio::test]
async fn insert_reports_a_draining_executable_plan() {
    let fixture = Fixture::new();
    fixture.insert().await;
    assert!(matches!(
        fixture
            .catalog
            .begin_drain(PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan()))
            .await,
        Ok(BeginDrainOutcome::Started(_))
    ));

    assert_eq!(
        fixture.catalog.insert(&fixture.record).await,
        Err(RevisionCatalogError::Draining {
            target: PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan()),
        }),
        "a draining plan must be reported, not silently accepted"
    );
}

#[tokio::test]
async fn insert_and_exact_load_are_idempotent_without_latest_fallback() {
    let fixture = Fixture::new();
    fixture.insert().await;
    assert_eq!(
        fixture.catalog.insert(&fixture.record).await,
        Ok(RevisionInsertOutcome::AlreadyPresent)
    );
    assert_eq!(
        fixture.catalog.load_exact(fixture.ids).await,
        Ok(fixture.record.clone())
    );

    let wrong_ids = PlanFlavorRevisionIds::new(
        fixture.ids.plan(),
        WorkerFlavorRevisionId::from_bytes([0x42; 32]),
    );
    assert!(matches!(
        fixture.catalog.load_exact(wrong_ids).await,
        Err(RevisionCatalogError::PlanFlavorMismatch { .. })
    ));
}

#[tokio::test]
async fn exact_load_rejects_over_nested_persisted_plan_with_redacted_budget_error() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let hostile_json = format!(
        "{}\"inmemory-hostile-canary\"{}",
        "[".repeat(RevisionRecordBytes::MAX_JSON_NESTING_DEPTH + 1),
        "]".repeat(RevisionRecordBytes::MAX_JSON_NESTING_DEPTH + 1)
    );
    let hostile_bytes = RevisionRecordBytes::try_from_vec(hostile_json.into_bytes())
        .expect("over-nested fixture remains within the one-MiB record envelope");
    let hostile_record = PlanFlavorRevisionRecord::graph_v1_json(
        fixture.ids.plan(),
        hostile_bytes,
        fixture.record.worker_flavor().clone(),
    );
    fixture
        .driver
        .replace_plan_record(fixture.ids.plan(), hostile_record);
    let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());

    let error = fixture
        .catalog
        .load_exact(fixture.ids)
        .await
        .expect_err("over-nested persisted JSON must fail at the InMemory load boundary");
    assert!(!format!("{error} {error:?}").contains("inmemory-hostile-canary"));
    std::assert_matches!(
        error,
        RevisionCatalogError::RecordNestingTooDeep { target: actual } if actual == target
    );
}

#[tokio::test]
async fn immutable_plan_content_conflict_preserves_the_original_record() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let conflicting_record = PlanFlavorRevisionRecord::graph_v1_json(
        fixture.ids.plan(),
        RevisionRecordBytes::try_from_vec(br#"{"plan":"different"}"#.to_vec())
            .expect("fixture conflict bytes are non-empty"),
        fixture.record.worker_flavor().clone(),
    );

    assert_eq!(
        fixture.catalog.insert(&conflicting_record).await,
        Err(RevisionCatalogError::ContentConflict {
            target: PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan()),
        })
    );
    assert_eq!(
        fixture.catalog.load_exact(fixture.ids).await,
        Ok(fixture.record.clone())
    );
}

#[tokio::test]
async fn immutable_flavor_conflict_does_not_partially_insert_the_new_plan() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let new_plan_id = ExecutablePlanRevisionId::from_bytes([0x32; 32]);
    let conflicting_flavor = WorkerFlavorRevisionRecord::v1_json(
        fixture.ids.worker_flavor(),
        RevisionRecordBytes::try_from_vec(br#"{"flavor":"different"}"#.to_vec())
            .expect("fixture conflict bytes are non-empty"),
    );
    let conflicting_record = PlanFlavorRevisionRecord::graph_v1_json(
        new_plan_id,
        RevisionRecordBytes::try_from_vec(br#"{"plan":"new"}"#.to_vec())
            .expect("fixture plan bytes are non-empty"),
        conflicting_flavor,
    );

    assert_eq!(
        fixture.catalog.insert(&conflicting_record).await,
        Err(RevisionCatalogError::ContentConflict {
            target: PlanFlavorRevisionTarget::WorkerFlavor(fixture.ids.worker_flavor()),
        })
    );
    assert_eq!(
        fixture.catalog.load_exact(conflicting_record.ids()).await,
        Err(RevisionCatalogError::PlanUnavailable {
            plan_id: new_plan_id,
        })
    );
}

#[tokio::test]
async fn draining_blocks_new_references_but_retained_execution_still_loads() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let retained = fixture.reference();
    assert_eq!(
        fixture.driver.retain(retained),
        Ok(RetainDecision::Retained)
    );
    assert!(matches!(
        fixture
            .catalog
            .begin_drain(PlanFlavorRevisionTarget::ExecutablePlan(
                fixture.ids.plan()
            ))
            .await,
        Ok(BeginDrainOutcome::Started(counts))
            if counts.live_executions() == 1 && counts.rollback_windows() == 0
    ));
    assert!(matches!(
        fixture
            .catalog
            .begin_drain(PlanFlavorRevisionTarget::ExecutablePlan(
                fixture.ids.plan()
            ))
            .await,
        Ok(BeginDrainOutcome::AlreadyDraining(counts))
            if counts.live_executions() == 1 && counts.rollback_windows() == 0
    ));
    assert_eq!(
        fixture.catalog.load_exact(fixture.ids).await,
        Ok(fixture.record.clone())
    );

    let new_reference = fixture.reference();
    assert_eq!(
        fixture.driver.retain(new_reference),
        Err(InternalRevisionError::Draining)
    );
    assert_eq!(
        fixture.driver.retain(retained),
        Ok(RetainDecision::AlreadyRetained)
    );
    assert_eq!(fixture.driver.reference_rows(), 1);
}

#[tokio::test]
async fn same_owner_with_different_pins_is_rejected_without_reference_delta() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let retained = fixture.reference();
    assert_eq!(
        fixture.driver.retain(retained),
        Ok(RetainDecision::Retained)
    );
    let mismatched = RevisionReference::new(
        retained.owner,
        ExecutionContractBundleId::new(),
        retained.ids,
    );
    assert_eq!(
        fixture.driver.retain(mismatched),
        Err(InternalRevisionError::ReferenceMismatch)
    );
    assert_eq!(fixture.driver.reference_rows(), 1);
}

#[tokio::test]
async fn stale_transition_with_different_pins_cannot_close_the_owner_reference() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let retained = fixture.reference();
    fixture
        .driver
        .retain(retained)
        .expect("active fixture pair accepts a live reference");
    let stale_reference = RevisionReference::new(
        retained.owner,
        ExecutionContractBundleId::new(),
        retained.ids,
    );

    assert_eq!(
        fixture
            .driver
            .transition(OwningReferenceTransition::ReleaseLive {
                reference: stale_reference,
            }),
        Err(InternalRevisionError::ReferenceMismatch)
    );
    let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
    assert!(matches!(
        fixture.catalog.begin_drain(target).await,
        Ok(BeginDrainOutcome::Started(references))
            if references.live_executions() == 1
    ));
}

#[tokio::test]
async fn insert_does_not_heal_a_plan_whose_pinned_flavor_row_is_missing() {
    let fixture = Fixture::new();
    fixture.insert().await;
    fixture
        .driver
        .remove_worker_flavor_record(fixture.ids.worker_flavor());

    assert_eq!(
        fixture.catalog.insert(&fixture.record).await,
        Err(RevisionCatalogError::CorruptRecord {
            target: PlanFlavorRevisionTarget::WorkerFlavor(fixture.ids.worker_flavor()),
        })
    );
    assert!(matches!(
        fixture.catalog.load_exact(fixture.ids).await,
        Err(RevisionCatalogError::WorkerFlavorUnavailable { .. })
    ));
}

#[tokio::test]
async fn rollback_deadline_equality_is_expired_and_retry_cannot_extend_it() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let retained = fixture.reference();
    fixture
        .driver
        .retain(retained)
        .expect("active fixture pair accepts a live reference");
    let now = fixture.clock.now();
    let rollback_window_id = RollbackWindowId([0x55; 16]);
    let transition = OwningReferenceTransition::RetainForRollback {
        reference: retained,
        window_id: rollback_window_id,
        retain_until: now,
    };
    assert_eq!(
        fixture.driver.transition(transition),
        Ok(ReferenceDecision::Applied)
    );
    assert_eq!(
        fixture.driver.transition(transition),
        Ok(ReferenceDecision::AlreadyApplied)
    );
    assert_eq!(
        fixture
            .driver
            .transition(OwningReferenceTransition::RetainForRollback {
                reference: retained,
                window_id: rollback_window_id,
                retain_until: now + TimeDelta::minutes(5),
            }),
        Err(InternalRevisionError::ReferenceMismatch)
    );

    let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
    assert!(matches!(
        fixture.catalog.begin_drain(target).await,
        Ok(BeginDrainOutcome::Started(counts)) if counts.is_empty()
    ));
    assert_eq!(fixture.catalog.delete_drained(target).await, Ok(()));
}

#[tokio::test]
async fn rollback_release_requires_the_exact_window_and_rejects_stale_live_release() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let retained = fixture.reference();
    fixture
        .driver
        .retain(retained)
        .expect("active fixture pair accepts a live reference");
    let rollback_window_id = RollbackWindowId([0x57; 16]);
    let retain_until = fixture.clock.now() + TimeDelta::minutes(5);
    fixture
        .driver
        .transition(OwningReferenceTransition::RetainForRollback {
            reference: retained,
            window_id: rollback_window_id,
            retain_until,
        })
        .expect("live owner may enter its rollback window");

    assert_eq!(
        fixture
            .driver
            .transition(OwningReferenceTransition::ReleaseLive {
                reference: retained,
            }),
        Err(InternalRevisionError::ReferenceMismatch)
    );
    assert_eq!(
        fixture
            .driver
            .transition(OwningReferenceTransition::ReleaseRollback {
                reference: retained,
                window_id: rollback_window_id,
                retain_until: retain_until + TimeDelta::seconds(1),
            }),
        Err(InternalRevisionError::ReferenceMismatch)
    );

    let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
    assert!(matches!(
        fixture.catalog.begin_drain(target).await,
        Ok(BeginDrainOutcome::Started(references))
            if references.rollback_windows() == 1
    ));
    let release = OwningReferenceTransition::ReleaseRollback {
        reference: retained,
        window_id: rollback_window_id,
        retain_until,
    };
    assert_eq!(
        fixture.driver.transition(release),
        Ok(ReferenceDecision::Applied)
    );
    assert_eq!(
        fixture.driver.transition(release),
        Ok(ReferenceDecision::AlreadyApplied)
    );
    assert_eq!(fixture.catalog.delete_drained(target).await, Ok(()));
}

#[tokio::test]
async fn live_and_unexpired_rollback_rows_block_delete_until_owner_releases() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let retained = fixture.reference();
    fixture
        .driver
        .retain(retained)
        .expect("active fixture pair accepts a live reference");
    let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
    fixture
        .catalog
        .begin_drain(target)
        .await
        .expect("inserted plan starts draining");
    assert!(matches!(
        fixture.catalog.delete_drained(target).await,
        Err(RevisionCatalogError::Referenced { references, .. })
            if references.live_executions() == 1
    ));

    let retain_until = fixture.clock.now() + TimeDelta::minutes(5);
    fixture
        .driver
        .transition(OwningReferenceTransition::RetainForRollback {
            reference: retained,
            window_id: RollbackWindowId([0x56; 16]),
            retain_until,
        })
        .expect("live owner may enter its rollback window");
    assert!(matches!(
        fixture.catalog.delete_drained(target).await,
        Err(RevisionCatalogError::Referenced { references, .. })
            if references.rollback_windows() == 1
    ));

    fixture.clock.set(retain_until);
    assert_eq!(fixture.catalog.delete_drained(target).await, Ok(()));
}

#[tokio::test]
async fn flavor_delete_waits_for_every_non_deleted_dependent_plan() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let flavor_target = PlanFlavorRevisionTarget::WorkerFlavor(fixture.ids.worker_flavor());
    fixture
        .catalog
        .begin_drain(flavor_target)
        .await
        .expect("inserted flavor starts draining");
    assert!(matches!(
        fixture.catalog.delete_drained(flavor_target).await,
        Err(RevisionCatalogError::DependentPlans {
            dependent_plans: 1,
            ..
        })
    ));

    let plan_target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
    fixture
        .catalog
        .begin_drain(plan_target)
        .await
        .expect("inserted plan starts draining");
    fixture
        .catalog
        .delete_drained(plan_target)
        .await
        .expect("unreferenced plan may be deleted");
    assert_eq!(fixture.catalog.delete_drained(flavor_target).await, Ok(()));
    assert!(matches!(
        fixture.catalog.insert(&fixture.record).await,
        Err(RevisionCatalogError::Deleted { .. })
    ));
}

#[tokio::test]
async fn retain_racing_drain_has_exactly_one_linearized_winner() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let reference = fixture.reference();
    let barrier = Arc::new(Barrier::new(2));
    let retain_driver = fixture.driver.clone();
    let retain_barrier = Arc::clone(&barrier);
    let retain_task = tokio::spawn(async move {
        retain_barrier.wait().await;
        retain_driver.retain(reference)
    });
    let drain_catalog = fixture.catalog.clone();
    let drain_barrier = Arc::clone(&barrier);
    let plan_target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
    let drain_task = tokio::spawn(async move {
        drain_barrier.wait().await;
        drain_catalog.begin_drain(plan_target).await
    });

    let retain_outcome = retain_task.await.expect("retain task must not panic");
    let drain_outcome = drain_task.await.expect("drain task must not panic");
    match (retain_outcome, drain_outcome) {
        (Ok(RetainDecision::Retained), Ok(BeginDrainOutcome::Started(reference_counts))) => {
            assert_eq!(reference_counts.live_executions(), 1);
        },
        (
            Err(InternalRevisionError::Draining),
            Ok(BeginDrainOutcome::Started(reference_counts)),
        ) => assert!(reference_counts.is_empty()),
        outcomes => panic!("unexpected retain/drain outcomes: {outcomes:?}"),
    }
}

#[tokio::test]
async fn exact_load_racing_delete_returns_copied_record_or_deleted_tombstone() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
    fixture
        .catalog
        .begin_drain(target)
        .await
        .expect("inserted plan starts draining");

    let barrier = Arc::new(Barrier::new(2));
    let load_catalog = fixture.catalog.clone();
    let load_barrier = Arc::clone(&barrier);
    let ids = fixture.ids;
    let load_task = tokio::spawn(async move {
        load_barrier.wait().await;
        load_catalog.load_exact(ids).await
    });
    let delete_catalog = fixture.catalog.clone();
    let delete_barrier = Arc::clone(&barrier);
    let delete_task = tokio::spawn(async move {
        delete_barrier.wait().await;
        delete_catalog.delete_drained(target).await
    });

    let load_outcome = load_task.await.expect("load task must not panic");
    let delete_outcome = delete_task.await.expect("delete task must not panic");
    assert_eq!(delete_outcome, Ok(()));
    match load_outcome {
        Ok(record) => assert_eq!(record, fixture.record),
        Err(RevisionCatalogError::Deleted {
            target: deleted_target,
        }) => assert_eq!(deleted_target, target),
        outcome => panic!("unexpected load/delete outcome: {outcome:?}"),
    }
}

#[tokio::test]
async fn release_is_idempotent_and_unblocks_guarded_delete() {
    let fixture = Fixture::new();
    fixture.insert().await;
    let retained = fixture.reference();
    fixture
        .driver
        .retain(retained)
        .expect("active fixture pair accepts a live reference");
    let transition = OwningReferenceTransition::ReleaseLive {
        reference: retained,
    };
    assert_eq!(
        fixture.driver.transition(transition),
        Ok(ReferenceDecision::Applied)
    );
    assert_eq!(
        fixture.driver.transition(transition),
        Ok(ReferenceDecision::AlreadyApplied)
    );

    let target = PlanFlavorRevisionTarget::ExecutablePlan(fixture.ids.plan());
    fixture
        .catalog
        .begin_drain(target)
        .await
        .expect("inserted plan starts draining");
    assert_eq!(fixture.catalog.delete_drained(target).await, Ok(()));
    assert_eq!(
        fixture.catalog.delete_drained(target).await,
        Ok(()),
        "lost delete acknowledgement is reconciled by the tombstone"
    );
}
