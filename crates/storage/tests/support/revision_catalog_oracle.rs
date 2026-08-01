//! One shared acceptance oracle for the exact plan/flavor catalog.
//!
//! The in-memory reference model, SQLite, and PostgreSQL implement the same
//! three port roles, so they must answer every catalog question identically.
//! This module owns those answers once; each backend's test file supplies a
//! catalog and runs the same cases against it. A behaviour that only one
//! backend gets right therefore fails somewhere, instead of being a per-file
//! assertion nobody wrote for the other two.
//!
//! Cases are keyed by a distinct seed so they can share one durable store
//! without colliding — spinning up an isolated PostgreSQL schema and replaying
//! the whole ordered migration catalog per case would cost minutes.
//!
//! ## What this oracle deliberately does not cover
//!
//! [`RevisionCatalogError::Referenced`] is unreachable from the port: revision
//! references can only be created inside an execution-owner transaction, which
//! is not implemented, so no production API can produce a blocker for the
//! guarded delete. Reference-blocked deletion is exercised by the in-memory
//! model's own unit tests through a backend-private driver, and moves here when
//! the owner transaction lands.

use nebula_core::{ExecutablePlanRevisionId, WorkerFlavorRevisionId};
use nebula_storage_port::{
    BeginDrainOutcome, PlanFlavorCatalog, PlanFlavorCatalogAdmin, PlanFlavorCatalogWriter,
    PlanFlavorRevisionIds, PlanFlavorRevisionRecord, PlanFlavorRevisionTarget,
    RevisionCatalogError, RevisionInsertOutcome, RevisionRecordBytes, WorkerFlavorRevisionRecord,
};

/// The three catalog roles one adapter offers together.
///
/// Production wiring hands each role out separately — an installer never
/// receives the admin capability — but a conformance run needs all three to
/// drive a revision through its whole lifecycle.
pub(crate) trait ExactRevisionCatalog:
    PlanFlavorCatalog + PlanFlavorCatalogWriter + PlanFlavorCatalogAdmin
{
}

impl<T> ExactRevisionCatalog for T where
    T: PlanFlavorCatalog + PlanFlavorCatalogWriter + PlanFlavorCatalogAdmin
{
}

/// Marker byte distinguishing an executable-plan identity from a flavor one.
const PLAN_IDENTITY_TAG: u8 = 0x50;

/// Marker byte distinguishing a worker-flavor identity from a plan one.
const FLAVOR_IDENTITY_TAG: u8 = 0x46;

/// Per-process namespace folded into every identity this oracle builds.
///
/// A backend whose durable store outlives the test run — a developer's or CI's
/// PostgreSQL — would otherwise meet the previous run's immutable identities on
/// the second run, so an `Inserted` case would report `AlreadyPresent` and a
/// tombstoned identity would refuse to install at all. A fresh namespace makes
/// each run independent of what earlier runs left behind.
static IDENTITY_NAMESPACE: std::sync::LazyLock<[u8; 16]> =
    std::sync::LazyLock::new(|| uuid::Uuid::new_v4().into_bytes());

fn identity_bytes(seed: u8, tag: u8, ordinal: u8) -> [u8; 32] {
    let mut bytes = [0x00_u8; 32];
    bytes[0] = seed;
    bytes[1] = tag;
    bytes[2] = ordinal;
    bytes[16..].copy_from_slice(IDENTITY_NAMESPACE.as_slice());
    bytes
}

/// An executable-plan identity unique to (`seed`, `ordinal`).
pub(crate) fn plan_id(seed: u8, ordinal: u8) -> ExecutablePlanRevisionId {
    ExecutablePlanRevisionId::from_bytes(identity_bytes(seed, PLAN_IDENTITY_TAG, ordinal))
}

/// A worker-flavor identity unique to (`seed`, `ordinal`).
pub(crate) fn worker_flavor_id(seed: u8, ordinal: u8) -> WorkerFlavorRevisionId {
    WorkerFlavorRevisionId::from_bytes(identity_bytes(seed, FLAVOR_IDENTITY_TAG, ordinal))
}

fn body(bytes: Vec<u8>) -> RevisionRecordBytes {
    RevisionRecordBytes::try_from_vec(bytes).expect("oracle record bodies are never empty")
}

/// Build a worker-flavor record whose body carries `content`.
pub(crate) fn flavor_record(seed: u8, ordinal: u8, content: &str) -> WorkerFlavorRevisionRecord {
    WorkerFlavorRevisionRecord::v1_json(
        worker_flavor_id(seed, ordinal),
        body(format!(r#"{{"flavor":"{content}"}}"#).into_bytes()),
    )
}

/// Build a plan/flavor pair whose two bodies carry `content`.
pub(crate) fn pair(seed: u8, ordinal: u8, content: &str) -> PlanFlavorRevisionRecord {
    PlanFlavorRevisionRecord::graph_v1_json(
        plan_id(seed, ordinal),
        body(format!(r#"{{"plan":"{content}"}}"#).into_bytes()),
        flavor_record(seed, ordinal, content),
    )
}

fn plan_target(record: &PlanFlavorRevisionRecord) -> PlanFlavorRevisionTarget {
    PlanFlavorRevisionTarget::ExecutablePlan(record.ids().plan())
}

fn flavor_target(record: &PlanFlavorRevisionRecord) -> PlanFlavorRevisionTarget {
    PlanFlavorRevisionTarget::WorkerFlavor(record.ids().worker_flavor())
}

async fn install(catalog: &impl ExactRevisionCatalog, record: &PlanFlavorRevisionRecord) {
    assert_eq!(
        catalog.insert(record).await,
        Ok(RevisionInsertOutcome::Inserted),
        "a fresh immutable pair installs exactly once"
    );
}

/// An installed pair loads back byte-for-byte through its exact identity.
pub(crate) async fn insert_then_exact_load_round_trips(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;

    assert_eq!(
        catalog.load_exact(record.ids()).await,
        Ok(record.clone()),
        "exact load returns the stored pair unchanged"
    );
}

/// Re-installing byte-identical content is idempotent, never a conflict.
pub(crate) async fn byte_identical_reinsert_is_already_present(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;

    assert_eq!(
        catalog.insert(&record).await,
        Ok(RevisionInsertOutcome::AlreadyPresent),
        "a retry of the same immutable pair is idempotent"
    );
    assert_eq!(catalog.load_exact(record.ids()).await, Ok(record));
}

/// Idempotency survives an encoder that emits the same document differently.
///
/// Record bodies are ordinary `serde_json` output, so key order and whitespace
/// depend on how the producing binary was built. Comparing raw bytes made two
/// binaries that agree on a revision's content address disagree on its record,
/// leaving an immutable revision permanently uninstallable.
pub(crate) async fn reencoded_identical_record_is_already_present(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;

    let reencoded = PlanFlavorRevisionRecord::graph_v1_json(
        record.ids().plan(),
        body(br#"{  "plan"  :  "v1"  }"#.to_vec()),
        WorkerFlavorRevisionRecord::v1_json(
            record.ids().worker_flavor(),
            body(br#"{  "flavor"  :  "v1"  }"#.to_vec()),
        ),
    );

    assert_eq!(
        catalog.insert(&reencoded).await,
        Ok(RevisionInsertOutcome::AlreadyPresent),
        "an encoding difference must not make an immutable revision uninstallable"
    );
    assert_eq!(
        catalog.load_exact(record.ids()).await,
        Ok(record),
        "the originally stored bytes stay authoritative"
    );
}

/// Reusing a plan identity with different content fails closed.
pub(crate) async fn plan_content_conflict_preserves_the_stored_record(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;

    let conflicting = PlanFlavorRevisionRecord::graph_v1_json(
        record.ids().plan(),
        body(br#"{"plan":"rewritten"}"#.to_vec()),
        record.worker_flavor().clone(),
    );
    assert_eq!(
        catalog.insert(&conflicting).await,
        Err(RevisionCatalogError::ContentConflict {
            target: plan_target(&record),
        })
    );
    assert_eq!(
        catalog.load_exact(record.ids()).await,
        Ok(record),
        "a rejected insert leaves the immutable record untouched"
    );
}

/// A flavor-content conflict names the flavor identity and writes no plan.
pub(crate) async fn worker_flavor_content_conflict_does_not_partially_insert_the_plan(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;

    // A new plan identity pinned to the installed flavor identity, but
    // carrying different flavor content.
    let conflicting = PlanFlavorRevisionRecord::graph_v1_json(
        plan_id(seed, 1),
        body(br#"{"plan":"second"}"#.to_vec()),
        WorkerFlavorRevisionRecord::v1_json(
            record.ids().worker_flavor(),
            body(br#"{"flavor":"rewritten"}"#.to_vec()),
        ),
    );
    assert_eq!(
        catalog.insert(&conflicting).await,
        Err(RevisionCatalogError::ContentConflict {
            target: flavor_target(&record),
        }),
        "the conflict is reported against the identity whose bytes differ"
    );
    assert_eq!(
        catalog.load_exact(conflicting.ids()).await,
        Err(RevisionCatalogError::PlanUnavailable {
            plan_id: plan_id(seed, 1),
        }),
        "the pair is atomic: a rejected flavor leaves no plan behind"
    );
}

/// Exact load never substitutes the flavor a plan is actually pinned to.
pub(crate) async fn exact_load_rejects_a_mismatched_worker_flavor(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;
    let other = pair(seed, 1, "other");
    install(catalog, &other).await;

    let crossed = PlanFlavorRevisionIds::new(record.ids().plan(), other.ids().worker_flavor());
    assert_eq!(
        catalog.load_exact(crossed).await,
        Err(RevisionCatalogError::PlanFlavorMismatch {
            requested: crossed,
            stored_worker_flavor_id: record.ids().worker_flavor(),
        }),
        "a plan is served only under the exact flavor it pins"
    );
}

/// An absent plan identity is unavailable, never resolved to a near match.
pub(crate) async fn exact_load_of_an_unknown_plan_is_unavailable(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;

    let unknown = PlanFlavorRevisionIds::new(plan_id(seed, 9), record.ids().worker_flavor());
    assert_eq!(
        catalog.load_exact(unknown).await,
        Err(RevisionCatalogError::PlanUnavailable {
            plan_id: plan_id(seed, 9),
        }),
        "exact load never falls back to a latest or closest revision"
    );
}

/// A body that is not a well-formed document of its recorded form is rejected,
/// and nothing about it is persisted.
pub(crate) async fn a_body_violating_its_recorded_form_persists_nothing(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let corrupt = PlanFlavorRevisionRecord::graph_v1_json(
        plan_id(seed, 0),
        body(b"this is not a graph-v1 json document".to_vec()),
        flavor_record(seed, 0, "v1"),
    );

    assert_eq!(
        catalog.insert(&corrupt).await,
        Err(RevisionCatalogError::CorruptRecord {
            target: plan_target(&corrupt),
        }),
        "a body that violates its recorded form never reaches durable storage"
    );
    assert_eq!(
        catalog.load_exact(corrupt.ids()).await,
        Err(RevisionCatalogError::PlanUnavailable {
            plan_id: plan_id(seed, 0),
        })
    );
    assert_eq!(
        catalog
            .begin_drain(PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id(
                seed, 0
            )))
            .await,
        Err(RevisionCatalogError::WorkerFlavorUnavailable {
            worker_flavor_id: worker_flavor_id(seed, 0),
        }),
        "the flavor half of a rejected pair is not written either"
    );
}

/// Draining is idempotent and does not stop a retained execution from loading.
pub(crate) async fn drain_is_idempotent_and_still_serves_exact_loads(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;
    let target = plan_target(&record);

    assert!(
        matches!(
            catalog.begin_drain(target).await,
            Ok(BeginDrainOutcome::Started(counts)) if counts.is_empty()
        ),
        "the first drain transitions Active to Draining"
    );
    assert!(
        matches!(
            catalog.begin_drain(target).await,
            Ok(BeginDrainOutcome::AlreadyDraining(counts)) if counts.is_empty()
        ),
        "repeating the drain is idempotent and re-projects the blockers"
    );
    assert_eq!(
        catalog.load_exact(record.ids()).await,
        Ok(record),
        "a draining revision still serves the executions that pinned it"
    );
}

/// A retrying installer learns that a revision is being retired.
pub(crate) async fn insert_against_a_draining_revision_reports_draining(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;
    assert_eq!(
        catalog.insert(&record).await,
        Ok(RevisionInsertOutcome::AlreadyPresent),
        "an ordinary retry against a live revision is idempotent"
    );

    let target = flavor_target(&record);
    assert!(matches!(
        catalog.begin_drain(target).await,
        Ok(BeginDrainOutcome::Started(_))
    ));
    assert_eq!(
        catalog.insert(&record).await,
        Err(RevisionCatalogError::Draining { target }),
        "drain is a property of the revision, not of the caller's history"
    );
}

/// Deletion of a still-active revision is refused.
pub(crate) async fn delete_requires_drain_first(catalog: &impl ExactRevisionCatalog, seed: u8) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;
    let target = plan_target(&record);

    assert_eq!(
        catalog.delete_drained(target).await,
        Err(RevisionCatalogError::DrainRequired { target })
    );
    assert_eq!(
        catalog.load_exact(record.ids()).await,
        Ok(record),
        "a refused delete changes nothing"
    );
}

/// A drained plan deletes to a tombstone, and repeating the delete reconciles a
/// commit whose acknowledgement was lost.
pub(crate) async fn delete_tombstones_and_a_repeat_delete_reconciles_a_lost_ack(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;
    let target = plan_target(&record);

    catalog
        .begin_drain(target)
        .await
        .expect("an installed plan starts draining");
    assert_eq!(catalog.delete_drained(target).await, Ok(()));
    assert_eq!(
        catalog.load_exact(record.ids()).await,
        Err(RevisionCatalogError::Deleted { target }),
        "a deleted identity keeps its tombstone rather than becoming unknown"
    );
    assert_eq!(
        catalog.delete_drained(target).await,
        Ok(()),
        "a lost delete acknowledgement is reconciled by the tombstone"
    );
}

/// A deleted identity can never be resurrected by re-installing its content.
pub(crate) async fn a_deleted_identity_cannot_be_resurrected(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;
    let target = plan_target(&record);

    catalog
        .begin_drain(target)
        .await
        .expect("an installed plan starts draining");
    catalog
        .delete_drained(target)
        .await
        .expect("an unreferenced drained plan deletes");

    assert_eq!(
        catalog.insert(&record).await,
        Err(RevisionCatalogError::Deleted { target }),
        "an immutable identity is spent once deleted"
    );
}

/// A worker flavor cannot be deleted while any non-deleted plan pins it.
pub(crate) async fn flavor_delete_waits_for_every_non_deleted_dependent_plan(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let record = pair(seed, 0, "v1");
    install(catalog, &record).await;
    let flavor = flavor_target(&record);
    let plan = plan_target(&record);

    catalog
        .begin_drain(flavor)
        .await
        .expect("an installed flavor starts draining");
    assert_eq!(
        catalog.delete_drained(flavor).await,
        Err(RevisionCatalogError::DependentPlans {
            worker_flavor_id: record.ids().worker_flavor(),
            dependent_plans: 1,
        })
    );

    catalog
        .begin_drain(plan)
        .await
        .expect("an installed plan starts draining");
    catalog
        .delete_drained(plan)
        .await
        .expect("an unreferenced drained plan deletes");
    assert_eq!(
        catalog.delete_drained(flavor).await,
        Ok(()),
        "the flavor deletes once its last dependent plan is gone"
    );
}

/// Draining an identity the catalog has never seen is unavailable.
pub(crate) async fn drain_of_an_unknown_revision_is_unavailable(
    catalog: &impl ExactRevisionCatalog,
    seed: u8,
) {
    let target = PlanFlavorRevisionTarget::ExecutablePlan(plan_id(seed, 0));
    assert_eq!(
        catalog.begin_drain(target).await,
        Err(RevisionCatalogError::PlanUnavailable {
            plan_id: plan_id(seed, 0),
        })
    );
    assert_eq!(
        catalog.delete_drained(target).await,
        Err(RevisionCatalogError::PlanUnavailable {
            plan_id: plan_id(seed, 0),
        })
    );
}

/// Generate one `#[tokio::test]` per shared case against `$catalog`.
///
/// `$catalog` is an async expression yielding `Option<impl
/// ExactRevisionCatalog>`; `None` means this backend is not reachable in the
/// current environment and the case reports that rather than asserting against
/// a substitute. Each case receives a distinct seed so the cases stay
/// independent while sharing one durable store.
///
/// The including file must declare this module as `oracle`.
#[macro_export]
macro_rules! revision_catalog_conformance_suite {
    ($catalog:expr) => {
        $crate::revision_catalog_case!(insert_then_exact_load_round_trips, 0x11, $catalog);
        $crate::revision_catalog_case!(byte_identical_reinsert_is_already_present, 0x12, $catalog);
        $crate::revision_catalog_case!(
            reencoded_identical_record_is_already_present,
            0x13,
            $catalog
        );
        $crate::revision_catalog_case!(
            plan_content_conflict_preserves_the_stored_record,
            0x14,
            $catalog
        );
        $crate::revision_catalog_case!(
            worker_flavor_content_conflict_does_not_partially_insert_the_plan,
            0x15,
            $catalog
        );
        $crate::revision_catalog_case!(
            exact_load_rejects_a_mismatched_worker_flavor,
            0x16,
            $catalog
        );
        $crate::revision_catalog_case!(
            exact_load_of_an_unknown_plan_is_unavailable,
            0x17,
            $catalog
        );
        $crate::revision_catalog_case!(
            a_body_violating_its_recorded_form_persists_nothing,
            0x18,
            $catalog
        );
        $crate::revision_catalog_case!(
            drain_is_idempotent_and_still_serves_exact_loads,
            0x19,
            $catalog
        );
        $crate::revision_catalog_case!(
            insert_against_a_draining_revision_reports_draining,
            0x1a,
            $catalog
        );
        $crate::revision_catalog_case!(delete_requires_drain_first, 0x1b, $catalog);
        $crate::revision_catalog_case!(
            delete_tombstones_and_a_repeat_delete_reconciles_a_lost_ack,
            0x1c,
            $catalog
        );
        $crate::revision_catalog_case!(a_deleted_identity_cannot_be_resurrected, 0x1d, $catalog);
        $crate::revision_catalog_case!(
            flavor_delete_waits_for_every_non_deleted_dependent_plan,
            0x1e,
            $catalog
        );
        $crate::revision_catalog_case!(drain_of_an_unknown_revision_is_unavailable, 0x1f, $catalog);
    };
}

/// Bind one shared case to a `#[tokio::test]` in the including backend file.
#[macro_export]
macro_rules! revision_catalog_case {
    ($case:ident, $seed:expr, $catalog:expr) => {
        #[tokio::test]
        async fn $case() {
            let Some(catalog) = $catalog.await else {
                eprintln!(concat!(
                    stringify!($case),
                    ": backend unreachable in this environment"
                ));
                return;
            };
            oracle::$case(&catalog, $seed).await;
        }
    };
}
