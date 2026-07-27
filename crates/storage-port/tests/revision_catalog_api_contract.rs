use std::{error::Error, sync::Arc};

use async_trait::async_trait;
use nebula_storage_port::{
    BeginDrainOutcome, ExecutablePlanRecordFormat, PlanFlavorCatalog, PlanFlavorCatalogAdmin,
    PlanFlavorCatalogWriter, PlanFlavorRevisionIds, PlanFlavorRevisionRecord,
    PlanFlavorRevisionTarget, RevisionCatalogError, RevisionInsertOutcome, RevisionRecordBytes,
    RevisionReferenceCounts, WorkerFlavorRecordFormat, WorkerFlavorRevisionRecord,
    ids::{ExecutablePlanRevisionId, WorkerFlavorRevisionId},
};

#[derive(Debug)]
struct CatalogProbe;

#[async_trait]
impl PlanFlavorCatalog for CatalogProbe {
    async fn load_exact(
        &self,
        ids: PlanFlavorRevisionIds,
    ) -> Result<PlanFlavorRevisionRecord, RevisionCatalogError> {
        Err(RevisionCatalogError::PlanUnavailable {
            plan_id: ids.plan(),
        })
    }
}

#[async_trait]
impl PlanFlavorCatalogWriter for CatalogProbe {
    async fn insert(
        &self,
        _record: &PlanFlavorRevisionRecord,
    ) -> Result<RevisionInsertOutcome, RevisionCatalogError> {
        Ok(RevisionInsertOutcome::AlreadyPresent)
    }
}

#[async_trait]
impl PlanFlavorCatalogAdmin for CatalogProbe {
    async fn begin_drain(
        &self,
        _target: PlanFlavorRevisionTarget,
    ) -> Result<BeginDrainOutcome, RevisionCatalogError> {
        Ok(BeginDrainOutcome::AlreadyDraining(
            RevisionReferenceCounts::new(2, 3),
        ))
    }

    async fn delete_drained(
        &self,
        _target: PlanFlavorRevisionTarget,
    ) -> Result<(), RevisionCatalogError> {
        Ok(())
    }
}

fn plan_id(byte: u8) -> ExecutablePlanRevisionId {
    ExecutablePlanRevisionId::from_bytes([byte; 32])
}

fn worker_flavor_id(byte: u8) -> WorkerFlavorRevisionId {
    WorkerFlavorRevisionId::from_bytes([byte; 32])
}

#[test]
fn exact_pair_records_preserve_typed_identity_and_format() {
    let plan_bytes = RevisionRecordBytes::try_from_vec(b"{\"secret\":\"plan-canary\"}".to_vec())
        .expect("non-empty plan record is valid");
    let flavor_bytes =
        RevisionRecordBytes::try_from_vec(b"{\"secret\":\"flavor-canary\"}".to_vec())
            .expect("non-empty flavor record is valid");
    let flavor = WorkerFlavorRevisionRecord::v1_json(worker_flavor_id(2), flavor_bytes);
    let record = PlanFlavorRevisionRecord::graph_v1_json(plan_id(1), plan_bytes, flavor);

    assert_eq!(
        record.ids(),
        PlanFlavorRevisionIds::new(plan_id(1), worker_flavor_id(2))
    );
    assert_eq!(
        record.plan_format(),
        ExecutablePlanRecordFormat::GraphV1Json
    );
    assert_eq!(
        record.worker_flavor().format(),
        WorkerFlavorRecordFormat::V1Json
    );
    assert_eq!(record.plan_bytes(), b"{\"secret\":\"plan-canary\"}");
    assert_eq!(
        record.worker_flavor().bytes(),
        b"{\"secret\":\"flavor-canary\"}"
    );
}

#[test]
fn revision_record_bytes_reject_empty_input_and_redact_debug() {
    assert_eq!(
        RevisionRecordBytes::try_from_vec(Vec::new()),
        Err(RevisionCatalogError::EmptyRecord)
    );

    let bytes = RevisionRecordBytes::try_from_vec(b"payload-canary".to_vec())
        .expect("non-empty record is valid");
    assert_eq!(bytes.as_bytes(), b"payload-canary");
    assert_eq!(format!("{bytes:?}"), "RevisionRecordBytes { len: 14 }");
    assert_eq!(bytes.into_vec(), b"payload-canary");
}

#[test]
fn record_debug_never_exposes_opaque_bytes() {
    let plan_bytes = RevisionRecordBytes::try_from_vec(b"plan-payload-canary".to_vec())
        .expect("non-empty plan record is valid");
    let flavor_bytes = RevisionRecordBytes::try_from_vec(b"flavor-payload-canary".to_vec())
        .expect("non-empty flavor record is valid");
    let flavor = WorkerFlavorRevisionRecord::v1_json(worker_flavor_id(2), flavor_bytes);
    let record = PlanFlavorRevisionRecord::graph_v1_json(plan_id(1), plan_bytes, flavor);

    let rendered = format!("{record:?}");
    assert!(!rendered.contains("plan-payload-canary"));
    assert!(!rendered.contains("flavor-payload-canary"));
    assert!(rendered.contains("len"));
}

#[test]
fn catalog_errors_have_closed_payload_free_diagnostics() {
    let plan = plan_id(3);
    let flavor = worker_flavor_id(4);
    let ids = PlanFlavorRevisionIds::new(plan, flavor);
    let plan_target = PlanFlavorRevisionTarget::ExecutablePlan(plan);
    let flavor_target = PlanFlavorRevisionTarget::WorkerFlavor(flavor);
    let references = RevisionReferenceCounts::new(5, 8);
    let failures = [
        RevisionCatalogError::PlanUnavailable { plan_id: plan },
        RevisionCatalogError::WorkerFlavorUnavailable {
            worker_flavor_id: flavor,
        },
        RevisionCatalogError::PlanFlavorMismatch {
            requested: ids,
            stored_worker_flavor_id: worker_flavor_id(6),
        },
        RevisionCatalogError::ContentConflict {
            target: plan_target,
        },
        RevisionCatalogError::Draining {
            target: flavor_target,
        },
        RevisionCatalogError::Deleted {
            target: plan_target,
        },
        RevisionCatalogError::DrainRequired {
            target: flavor_target,
        },
        RevisionCatalogError::Referenced {
            target: plan_target,
            references,
        },
        RevisionCatalogError::DependentPlans {
            worker_flavor_id: flavor,
            dependent_plans: 13,
        },
        RevisionCatalogError::EmptyRecord,
        RevisionCatalogError::UnsupportedRecordFormat {
            target: flavor_target,
        },
        RevisionCatalogError::CorruptRecord {
            target: plan_target,
        },
        RevisionCatalogError::Unavailable,
        RevisionCatalogError::OutcomeUnknown,
    ];

    for failure in failures {
        let rendered = format!("{failure} {failure:?}");
        for canary in [
            "revision-payload-canary",
            "tenant-canary",
            "postgres://",
            "SELECT ",
        ] {
            assert!(!rendered.contains(canary));
        }
        assert!(failure.source().is_none());
    }
}

#[test]
fn reference_counts_expose_bounded_projections() {
    let references = RevisionReferenceCounts::new(5, 8);

    assert_eq!(references.live_executions(), 5);
    assert_eq!(references.rollback_windows(), 8);
    assert!(!references.is_empty());
    assert!(RevisionReferenceCounts::default().is_empty());
}

#[test]
fn catalog_roles_are_object_safe_and_arc_forwarding_preserves_behavior() {
    fn accepts_reader(_: Option<Arc<dyn PlanFlavorCatalog>>) {}
    fn accepts_writer(_: Option<Arc<dyn PlanFlavorCatalogWriter>>) {}
    fn accepts_admin(_: Option<Arc<dyn PlanFlavorCatalogAdmin>>) {}
    fn assert_error_contract<E: Error + Send + Sync + 'static>() {}

    accepts_reader(None);
    accepts_writer(None);
    accepts_admin(None);
    assert_error_contract::<RevisionCatalogError>();

    let probe = Arc::new(CatalogProbe);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("test runtime can be built");
    let unavailable = runtime
        .block_on(PlanFlavorCatalog::load_exact(
            &probe,
            PlanFlavorRevisionIds::new(plan_id(7), worker_flavor_id(9)),
        ))
        .expect_err("probe always reports an unavailable plan");
    assert_eq!(
        unavailable,
        RevisionCatalogError::PlanUnavailable {
            plan_id: plan_id(7)
        }
    );
}
