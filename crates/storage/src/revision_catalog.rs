//! Backend-independent exact plan/flavor catalog decisions.
//!
//! Every adapter — the in-memory reference model, SQLite, and PostgreSQL —
//! routes record identity, recorded-form validity, and lifecycle vocabulary
//! through this module, so the three backends cannot answer the same question
//! differently. Row plumbing stays in each adapter; the decisions do not.
//!
//! Durable lifecycle and recorded-form text is the same vocabulary ordered
//! migration 0041 constrains with `CHECK` clauses. The constants below are the
//! single Rust-side definition of that vocabulary; nothing re-spells it.

#[cfg(any(feature = "sqlite", feature = "postgres"))]
use nebula_core::{ExecutablePlanRevisionId, WorkerFlavorRevisionId};
#[cfg(any(feature = "sqlite", feature = "postgres"))]
use nebula_storage_port::RevisionRecordBytes;
use nebula_storage_port::{
    BeginDrainOutcome, ExecutablePlanRecordFormat, PlanFlavorRevisionIds, PlanFlavorRevisionRecord,
    PlanFlavorRevisionTarget, RevisionCatalogError, RevisionInsertOutcome,
    WorkerFlavorRevisionRecord,
};

/// Durable `record_format` text of a version-one JSON worker-flavor record.
///
/// Only a SQL backend spells a recorded form as durable text; the in-memory
/// reference model keeps the typed format on the record itself.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const WORKER_FLAVOR_V1_JSON: &str = "v1_json";

/// Durable `record_format` text of a Graph-v1 JSON executable-plan record.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) const EXECUTABLE_PLAN_GRAPH_V1_JSON: &str = "graph_v1_json";

/// Durable lifecycle of one immutable catalog identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArtifactLifecycle {
    /// Accepts exact loads and new references.
    Active,
    /// Accepts exact loads, rejects new references and new content.
    Draining,
    /// Identity tombstone; payload bytes are cleared.
    Deleted,
}

/// Durable lifecycle text is only spelled by a SQL backend; the in-memory
/// reference model holds the typed lifecycle directly.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
impl ArtifactLifecycle {
    /// Return the durable lifecycle text migration 0041 constrains.
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Draining => "draining",
            Self::Deleted => "deleted",
        }
    }

    /// Parse durable lifecycle text, rejecting any vocabulary the schema does
    /// not admit.
    pub(crate) fn from_text(text: &str) -> Option<Self> {
        match text {
            "active" => Some(Self::Active),
            "draining" => Some(Self::Draining),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }
}

/// Whether two JSON record bodies carry the same content.
///
/// Compares the *parsed* documents, never the raw bytes. Record bodies are
/// ordinary `serde_json` output, so their exact bytes depend on struct field
/// declaration order and on whether `serde_json` was built with
/// `preserve_order` — a workspace-unified feature any crate in the graph can
/// turn on. Byte equality therefore made two binaries that agree on a
/// revision's content address nonetheless disagree on its record, so
/// re-installing the same immutable revision failed permanently with
/// `ContentConflict` instead of reporting `AlreadyPresent`.
///
/// Parsing normalises exactly the incidental differences (key order,
/// whitespace) while still separating genuinely different documents, so
/// conflict detection keeps its meaning. Bodies that do not parse fall back to
/// byte equality rather than being treated as equal.
pub(crate) fn json_bodies_match(stored: &[u8], candidate: &[u8]) -> bool {
    match (
        serde_json::from_slice::<serde_json::Value>(stored),
        serde_json::from_slice::<serde_json::Value>(candidate),
    ) {
        (Ok(stored), Ok(candidate)) => stored == candidate,
        _ => stored == candidate,
    }
}

/// Whether two worker-flavor records are the same immutable content.
pub(crate) fn flavor_records_match(
    stored: &WorkerFlavorRevisionRecord,
    candidate: &WorkerFlavorRevisionRecord,
) -> bool {
    stored.id() == candidate.id()
        && stored.format() == candidate.format()
        && json_bodies_match(stored.bytes(), candidate.bytes())
}

/// Whether a stored executable plan is the same immutable content as
/// `candidate`'s plan half.
///
/// The paired worker flavor is compared through its own identity, never
/// through a second copy carried inside the plan. Each payload has exactly one
/// durable home, so a flavor-content difference must be reported against the
/// flavor identity that owns it — blaming the plan named an immutable identity
/// whose own bytes are unchanged, and the two backends disagreed about which
/// identity conflicted for the same insert.
pub(crate) fn plan_content_matches(
    stored_ids: PlanFlavorRevisionIds,
    stored_format: ExecutablePlanRecordFormat,
    stored_plan_bytes: &[u8],
    candidate: &PlanFlavorRevisionRecord,
) -> bool {
    stored_ids == candidate.ids()
        && stored_format == candidate.plan_format()
        && json_bodies_match(stored_plan_bytes, candidate.plan_bytes())
}

/// Whether a stored pair is the same immutable plan content as `candidate`.
pub(crate) fn plan_records_match(
    stored: &PlanFlavorRevisionRecord,
    candidate: &PlanFlavorRevisionRecord,
) -> bool {
    plan_content_matches(
        stored.ids(),
        stored.plan_format(),
        stored.plan_bytes(),
        candidate,
    )
}

/// Stable label naming one catalog rejection, so every adapter reports the
/// same outcome vocabulary on its spans.
pub(crate) const fn error_label(error: &RevisionCatalogError) -> &'static str {
    match *error {
        RevisionCatalogError::PlanUnavailable { .. } => "plan_unavailable",
        RevisionCatalogError::WorkerFlavorUnavailable { .. } => "worker_flavor_unavailable",
        RevisionCatalogError::PlanFlavorMismatch { .. } => "plan_flavor_mismatch",
        RevisionCatalogError::ContentConflict { .. } => "content_conflict",
        RevisionCatalogError::Draining { .. } => "draining",
        RevisionCatalogError::Deleted { .. } => "deleted",
        RevisionCatalogError::DrainRequired { .. } => "drain_required",
        RevisionCatalogError::Referenced { .. } => "referenced",
        RevisionCatalogError::DependentPlans { .. } => "dependent_plans",
        RevisionCatalogError::EmptyRecord => "empty_record",
        RevisionCatalogError::UnsupportedRecordFormat { .. } => "unsupported_record_format",
        RevisionCatalogError::CorruptRecord { .. } => "corrupt_record",
        RevisionCatalogError::Unavailable => "unavailable",
        RevisionCatalogError::OutcomeUnknown => "outcome_unknown",
        // The port marks this error `#[non_exhaustive]`, so a wildcard is
        // required. A label that names the gap is better than folding an
        // unrecognised rejection into a neighbouring bucket, where it would
        // read on a dashboard as a rejection this build understands.
        _ => "unclassified",
    }
}

/// Stable outcome label for one insert attempt.
pub(crate) const fn insert_label(
    result: &Result<RevisionInsertOutcome, RevisionCatalogError>,
) -> &'static str {
    match *result {
        Ok(RevisionInsertOutcome::Inserted) => "inserted",
        Ok(RevisionInsertOutcome::AlreadyPresent) => "already_present",
        Err(ref error) => error_label(error),
    }
}

/// Stable outcome label for one drain attempt.
pub(crate) const fn drain_label(
    result: &Result<BeginDrainOutcome, RevisionCatalogError>,
) -> &'static str {
    match *result {
        Ok(BeginDrainOutcome::Started(_)) => "started",
        Ok(BeginDrainOutcome::AlreadyDraining(_)) => "already_draining",
        Err(ref error) => error_label(error),
    }
}

/// Stable outcome label for one exact load.
pub(crate) const fn load_label(
    result: &Result<PlanFlavorRevisionRecord, RevisionCatalogError>,
) -> &'static str {
    match *result {
        Ok(_) => "loaded",
        Err(ref error) => error_label(error),
    }
}

/// Stable outcome label for one guarded delete.
pub(crate) const fn delete_label(result: &Result<(), RevisionCatalogError>) -> &'static str {
    match *result {
        Ok(()) => "deleted",
        Err(ref error) => error_label(error),
    }
}

/// Reject bytes that do not satisfy the contract of their recorded form.
///
/// Both recorded forms this catalog admits are JSON documents, so a body that
/// does not parse is durable corruption rather than an opaque payload storage
/// merely relays. Checking it on the way in keeps a caller from persisting a
/// record that only fails when a later exact load needs it; checking it on the
/// way out keeps a byte-level poke at the table from being handed to a plugin
/// layer as if it were a valid record.
///
/// # Errors
///
/// Returns [`RevisionCatalogError::CorruptRecord`] when `bytes` is not a
/// well-formed JSON document.
pub(crate) fn validate_recorded_form(
    bytes: &[u8],
    target: PlanFlavorRevisionTarget,
) -> Result<(), RevisionCatalogError> {
    serde_json::from_slice::<serde::de::IgnoredAny>(bytes)
        .map(|_ignored| ())
        .map_err(|_parse| RevisionCatalogError::CorruptRecord { target })
}

/// Reject a whole pair whose bytes violate their recorded form.
///
/// # Errors
///
/// Returns [`RevisionCatalogError::CorruptRecord`] naming the first identity
/// whose body is not a well-formed JSON document.
pub(crate) fn validate_pair_recorded_form(
    record: &PlanFlavorRevisionRecord,
) -> Result<(), RevisionCatalogError> {
    let ids = record.ids();
    validate_recorded_form(
        record.plan_bytes(),
        PlanFlavorRevisionTarget::ExecutablePlan(ids.plan()),
    )?;
    validate_recorded_form(
        record.worker_flavor().bytes(),
        PlanFlavorRevisionTarget::WorkerFlavor(ids.worker_flavor()),
    )
}

/// Return the `…Unavailable` variant that matches `target`'s kind.
pub(crate) fn unavailable_for(target: PlanFlavorRevisionTarget) -> RevisionCatalogError {
    match target {
        PlanFlavorRevisionTarget::ExecutablePlan(plan_id) => {
            RevisionCatalogError::PlanUnavailable { plan_id }
        },
        PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id) => {
            RevisionCatalogError::WorkerFlavorUnavailable { worker_flavor_id }
        },
    }
}

/// Return the deletion tombstone rejection for `target`.
pub(crate) const fn deleted_for(target: PlanFlavorRevisionTarget) -> RevisionCatalogError {
    RevisionCatalogError::Deleted { target }
}

/// Return the drain rejection for `target`.
pub(crate) const fn draining_for(target: PlanFlavorRevisionTarget) -> RevisionCatalogError {
    RevisionCatalogError::Draining { target }
}

/// One decoded durable worker-flavor row.
///
/// Only the SQL backends decode durable rows; the in-memory reference model
/// holds records directly.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredWorkerFlavor {
    /// Durable lifecycle of the immutable identity.
    pub(crate) lifecycle: ArtifactLifecycle,
    /// Payload, absent exactly when the identity is a tombstone.
    pub(crate) record: Option<WorkerFlavorRevisionRecord>,
}

/// One decoded durable executable-plan row.
///
/// The durable plan row pins its worker flavor by identifier and stores only
/// its own bytes; the paired flavor record is composed from the flavor row so
/// the pair has exactly one durable copy of each payload.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredExecutablePlan {
    /// Durable lifecycle of the immutable identity.
    pub(crate) lifecycle: ArtifactLifecycle,
    /// Worker flavor this immutable plan is pinned to.
    pub(crate) worker_flavor_id: WorkerFlavorRevisionId,
    /// Payload, absent exactly when the identity is a tombstone.
    pub(crate) plan_bytes: Option<RevisionRecordBytes>,
}

/// Interpret a durable 32-byte revision identifier column.
///
/// # Errors
///
/// Returns [`RevisionCatalogError::CorruptRecord`] when the column is not
/// exactly 32 bytes wide.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
fn revision_id_bytes(
    column: Vec<u8>,
    target: PlanFlavorRevisionTarget,
) -> Result<[u8; 32], RevisionCatalogError> {
    <[u8; 32]>::try_from(column).map_err(|_width| RevisionCatalogError::CorruptRecord { target })
}

/// Interpret a durable worker-flavor row.
///
/// # Errors
///
/// Returns [`RevisionCatalogError::UnsupportedRecordFormat`] when the durable
/// format names a recorded form this build cannot read, and
/// [`RevisionCatalogError::CorruptRecord`] when lifecycle text is unknown or
/// payload presence contradicts the lifecycle.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn decode_worker_flavor_row(
    worker_flavor_id: WorkerFlavorRevisionId,
    lifecycle: &str,
    record_format: &str,
    record_bytes: Option<Vec<u8>>,
) -> Result<StoredWorkerFlavor, RevisionCatalogError> {
    let target = PlanFlavorRevisionTarget::WorkerFlavor(worker_flavor_id);
    let lifecycle = ArtifactLifecycle::from_text(lifecycle)
        .ok_or(RevisionCatalogError::CorruptRecord { target })?;
    if record_format != WORKER_FLAVOR_V1_JSON {
        return Err(RevisionCatalogError::UnsupportedRecordFormat { target });
    }

    let record = match (lifecycle, record_bytes) {
        (ArtifactLifecycle::Deleted, None) => None,
        (ArtifactLifecycle::Deleted, Some(_)) | (_, None) => {
            return Err(RevisionCatalogError::CorruptRecord { target });
        },
        (ArtifactLifecycle::Active | ArtifactLifecycle::Draining, Some(payload)) => {
            validate_recorded_form(&payload, target)?;
            let bytes = RevisionRecordBytes::try_from_vec(payload)
                .map_err(|_empty| RevisionCatalogError::CorruptRecord { target })?;
            Some(WorkerFlavorRevisionRecord::v1_json(worker_flavor_id, bytes))
        },
    };

    Ok(StoredWorkerFlavor { lifecycle, record })
}

/// Interpret a durable executable-plan row.
///
/// # Errors
///
/// Returns [`RevisionCatalogError::UnsupportedRecordFormat`] when the durable
/// format names a recorded form this build cannot read, and
/// [`RevisionCatalogError::CorruptRecord`] when lifecycle text is unknown, the
/// pinned flavor identifier is malformed, or payload presence contradicts the
/// lifecycle.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn decode_executable_plan_row(
    plan_id: ExecutablePlanRevisionId,
    worker_flavor_id: Vec<u8>,
    lifecycle: &str,
    record_format: &str,
    record_bytes: Option<Vec<u8>>,
) -> Result<StoredExecutablePlan, RevisionCatalogError> {
    let target = PlanFlavorRevisionTarget::ExecutablePlan(plan_id);
    let lifecycle = ArtifactLifecycle::from_text(lifecycle)
        .ok_or(RevisionCatalogError::CorruptRecord { target })?;
    if record_format != EXECUTABLE_PLAN_GRAPH_V1_JSON {
        return Err(RevisionCatalogError::UnsupportedRecordFormat { target });
    }
    let worker_flavor_id =
        WorkerFlavorRevisionId::from_bytes(revision_id_bytes(worker_flavor_id, target)?);

    let plan_bytes = match (lifecycle, record_bytes) {
        (ArtifactLifecycle::Deleted, None) => None,
        (ArtifactLifecycle::Deleted, Some(_)) | (_, None) => {
            return Err(RevisionCatalogError::CorruptRecord { target });
        },
        (ArtifactLifecycle::Active | ArtifactLifecycle::Draining, Some(payload)) => {
            validate_recorded_form(&payload, target)?;
            Some(
                RevisionRecordBytes::try_from_vec(payload)
                    .map_err(|_empty| RevisionCatalogError::CorruptRecord { target })?,
            )
        },
    };

    Ok(StoredExecutablePlan {
        lifecycle,
        worker_flavor_id,
        plan_bytes,
    })
}

/// Compose the durable pair from a plan row and the flavor row it pins.
///
/// # Errors
///
/// Returns [`RevisionCatalogError::CorruptRecord`] when either identity is a
/// tombstone-shaped row at this point, or when `flavor` is not the row the plan
/// pins.
#[cfg(any(feature = "sqlite", feature = "postgres"))]
pub(crate) fn compose_pair(
    plan_id: ExecutablePlanRevisionId,
    plan: &StoredExecutablePlan,
    flavor: &StoredWorkerFlavor,
) -> Result<PlanFlavorRevisionRecord, RevisionCatalogError> {
    let plan_target = PlanFlavorRevisionTarget::ExecutablePlan(plan_id);
    let flavor_target = PlanFlavorRevisionTarget::WorkerFlavor(plan.worker_flavor_id);

    let plan_bytes = plan
        .plan_bytes
        .clone()
        .ok_or(RevisionCatalogError::CorruptRecord {
            target: plan_target,
        })?;
    let flavor_record = flavor
        .record
        .clone()
        .ok_or(RevisionCatalogError::CorruptRecord {
            target: flavor_target,
        })?;
    if flavor_record.id() != plan.worker_flavor_id {
        return Err(RevisionCatalogError::CorruptRecord {
            target: flavor_target,
        });
    }

    Ok(PlanFlavorRevisionRecord::graph_v1_json(
        plan_id,
        plan_bytes,
        flavor_record,
    ))
}

#[cfg(test)]
mod tests {
    use nebula_core::WorkerFlavorRevisionId as TestWorkerFlavorRevisionId;

    use super::*;

    #[test]
    fn a_body_that_is_not_json_violates_its_recorded_form() {
        let target = PlanFlavorRevisionTarget::WorkerFlavor(
            TestWorkerFlavorRevisionId::from_bytes([0x41; 32]),
        );
        assert_eq!(
            validate_recorded_form(b"not json", target),
            Err(RevisionCatalogError::CorruptRecord { target })
        );
        assert_eq!(
            validate_recorded_form(br#"{"flavor":"v1"}"#, target),
            Ok(())
        );
    }
}

/// Durable-row decoding is exercised only where a SQL backend exists to
/// produce those rows.
#[cfg(all(test, any(feature = "sqlite", feature = "postgres")))]
mod durable_row_tests {
    use nebula_storage_port::WorkerFlavorRecordFormat;

    use super::*;

    fn flavor_id() -> WorkerFlavorRevisionId {
        WorkerFlavorRevisionId::from_bytes([0x41; 32])
    }

    fn plan_id() -> ExecutablePlanRevisionId {
        ExecutablePlanRevisionId::from_bytes([0x31; 32])
    }

    #[test]
    fn lifecycle_text_round_trips_through_the_durable_vocabulary() {
        for lifecycle in [
            ArtifactLifecycle::Active,
            ArtifactLifecycle::Draining,
            ArtifactLifecycle::Deleted,
        ] {
            assert_eq!(
                ArtifactLifecycle::from_text(lifecycle.as_str()),
                Some(lifecycle)
            );
        }
        assert_eq!(ArtifactLifecycle::from_text("retired"), None);
    }

    #[test]
    fn decoding_reports_an_unreadable_recorded_form_before_reading_payload() {
        assert_eq!(
            decode_worker_flavor_row(flavor_id(), "active", "v2_cbor", Some(b"\x00".to_vec())),
            Err(RevisionCatalogError::UnsupportedRecordFormat {
                target: PlanFlavorRevisionTarget::WorkerFlavor(flavor_id()),
            })
        );
    }

    #[test]
    fn a_tombstone_row_that_still_carries_payload_is_corrupt() {
        assert_eq!(
            decode_worker_flavor_row(
                flavor_id(),
                "deleted",
                WORKER_FLAVOR_V1_JSON,
                Some(br#"{"flavor":"v1"}"#.to_vec()),
            ),
            Err(RevisionCatalogError::CorruptRecord {
                target: PlanFlavorRevisionTarget::WorkerFlavor(flavor_id()),
            })
        );
    }

    #[test]
    fn a_live_row_decodes_into_its_recorded_form() {
        let decoded = decode_worker_flavor_row(
            flavor_id(),
            "draining",
            WORKER_FLAVOR_V1_JSON,
            Some(br#"{"flavor":"v1"}"#.to_vec()),
        )
        .expect("a well-formed draining row decodes");
        assert_eq!(decoded.lifecycle, ArtifactLifecycle::Draining);
        let record = decoded.record.expect("a draining row retains its payload");
        assert_eq!(record.format(), WorkerFlavorRecordFormat::V1Json);
        assert_eq!(record.bytes(), br#"{"flavor":"v1"}"#);
    }

    #[test]
    fn a_plan_row_whose_pinned_flavor_column_is_narrow_is_corrupt() {
        assert_eq!(
            decode_executable_plan_row(
                plan_id(),
                vec![0x41; 16],
                "active",
                EXECUTABLE_PLAN_GRAPH_V1_JSON,
                Some(br#"{"plan":"v1"}"#.to_vec()),
            ),
            Err(RevisionCatalogError::CorruptRecord {
                target: PlanFlavorRevisionTarget::ExecutablePlan(plan_id()),
            })
        );
    }

    #[test]
    fn composing_a_pair_rejects_a_flavor_row_the_plan_does_not_pin() {
        let plan = decode_executable_plan_row(
            plan_id(),
            vec![0x41; 32],
            "active",
            EXECUTABLE_PLAN_GRAPH_V1_JSON,
            Some(br#"{"plan":"v1"}"#.to_vec()),
        )
        .expect("a well-formed plan row decodes");
        let other_flavor_id = WorkerFlavorRevisionId::from_bytes([0x42; 32]);
        let flavor = decode_worker_flavor_row(
            other_flavor_id,
            "active",
            WORKER_FLAVOR_V1_JSON,
            Some(br#"{"flavor":"v1"}"#.to_vec()),
        )
        .expect("a well-formed flavor row decodes");

        assert_eq!(
            compose_pair(plan_id(), &plan, &flavor),
            Err(RevisionCatalogError::CorruptRecord {
                target: PlanFlavorRevisionTarget::WorkerFlavor(flavor_id()),
            })
        );
    }
}
