//! Exact executable-plan and worker-flavor catalog values.

use std::fmt;

use serde::de::{DeserializeOwned, DeserializeSeed, MapAccess, SeqAccess, Visitor};

use crate::ids::{ExecutablePlanRevisionId, WorkerFlavorRevisionId};

/// Exact executable-plan and worker-flavor revision pair.
///
/// The pair carries typed identifiers so a plan identifier cannot be
/// accidentally substituted for a worker-flavor identifier at a storage
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PlanFlavorRevisionIds {
    plan: ExecutablePlanRevisionId,
    worker_flavor: WorkerFlavorRevisionId,
}

impl PlanFlavorRevisionIds {
    /// Construct one exact plan/flavor revision pair.
    pub const fn new(
        plan: ExecutablePlanRevisionId,
        worker_flavor: WorkerFlavorRevisionId,
    ) -> Self {
        Self {
            plan,
            worker_flavor,
        }
    }

    /// Return the exact executable-plan revision identifier.
    pub const fn plan(self) -> ExecutablePlanRevisionId {
        self.plan
    }

    /// Return the exact worker-flavor revision identifier.
    pub const fn worker_flavor(self) -> WorkerFlavorRevisionId {
        self.worker_flavor
    }
}

/// Non-empty opaque serialized revision record.
///
/// Storage does not interpret these bytes. The owning plugin layer decodes
/// and verifies the versioned record after an exact load.
#[must_use]
#[derive(Clone, PartialEq, Eq)]
pub struct RevisionRecordBytes(Box<[u8]>);

impl RevisionRecordBytes {
    /// Maximum serialized size of one revision record.
    pub const MAX_BYTES: usize = 1024 * 1024;
    /// Maximum JSON object/array nesting accepted before typed decoding.
    pub const MAX_JSON_NESTING_DEPTH: usize = 64;
    /// Maximum decoded UTF-8 size of one JSON string.
    pub const MAX_JSON_STRING_BYTES: usize = 256 * 1024;
    /// Maximum decoded UTF-8 bytes across all JSON keys and string values.
    pub const MAX_JSON_TOTAL_STRING_BYTES: usize = 512 * 1024;
    /// Maximum aggregate entries across all JSON objects and arrays.
    pub const MAX_JSON_COLLECTION_ENTRIES: usize = 65_536;

    /// Convert a bounded, non-empty byte vector into an opaque revision record.
    ///
    /// # Errors
    ///
    /// Returns [`RevisionCatalogError::EmptyRecord`] when `bytes` is empty or
    /// [`RevisionCatalogError::RecordTooLarge`] when it exceeds [`Self::MAX_BYTES`].
    pub fn try_from_vec(bytes: Vec<u8>) -> Result<Self, RevisionCatalogError> {
        if bytes.is_empty() {
            return Err(RevisionCatalogError::EmptyRecord);
        }
        if bytes.len() > Self::MAX_BYTES {
            return Err(RevisionCatalogError::RecordTooLarge {
                max_bytes: Self::MAX_BYTES,
                actual_bytes: bytes.len(),
            });
        }

        Ok(Self(bytes.into_boxed_slice()))
    }

    /// Decode one JSON recorded form after enforcing generic resource limits.
    ///
    /// These limits protect the persistence boundary itself. Domain and schema
    /// validation remain owned by their typed decoders after this method
    /// returns.
    ///
    /// # Errors
    ///
    /// Returns a payload-redacted [`RevisionCatalogError`] when the record is
    /// malformed or exceeds a nesting, string, or collection budget.
    pub fn deserialize_json<T>(
        &self,
        target: PlanFlavorRevisionTarget,
    ) -> Result<T, RevisionCatalogError>
    where
        T: DeserializeOwned,
    {
        let mut budget = JsonDecodeBudget::default();
        let mut scanner = serde_json::Deserializer::from_slice(&self.0);
        let scan_result = JsonBudgetSeed {
            budget: &mut budget,
        }
        .deserialize(&mut scanner)
        .and_then(|()| scanner.end());

        if scan_result.is_err() {
            return Err(match budget.exceeded {
                Some(JsonLimit::NestingDepth) => {
                    RevisionCatalogError::RecordNestingTooDeep { target }
                },
                Some(JsonLimit::SingleStringBytes) => {
                    RevisionCatalogError::RecordStringTooLarge { target }
                },
                Some(JsonLimit::TotalStringBytes) => {
                    RevisionCatalogError::RecordStringBudgetExceeded { target }
                },
                Some(JsonLimit::CollectionEntries) => {
                    RevisionCatalogError::RecordCollectionBudgetExceeded { target }
                },
                None => RevisionCatalogError::CorruptRecord { target },
            });
        }

        serde_json::from_slice(&self.0)
            .map_err(|_decode| RevisionCatalogError::CorruptRecord { target })
    }

    /// Borrow the serialized record bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Consume this value and return its serialized record bytes.
    pub fn into_vec(self) -> Vec<u8> {
        self.0.into_vec()
    }
}

#[derive(Debug, Clone, Copy)]
enum JsonLimit {
    NestingDepth,
    SingleStringBytes,
    TotalStringBytes,
    CollectionEntries,
}

#[derive(Debug, Default)]
struct JsonDecodeBudget {
    nesting_depth: usize,
    total_string_bytes: usize,
    collection_entries: usize,
    exceeded: Option<JsonLimit>,
}

impl JsonDecodeBudget {
    fn enter_collection<E>(&mut self) -> Result<(), E>
    where
        E: serde::de::Error,
    {
        self.nesting_depth = self.nesting_depth.saturating_add(1);
        if self.nesting_depth > RevisionRecordBytes::MAX_JSON_NESTING_DEPTH {
            return self.reject(JsonLimit::NestingDepth);
        }
        Ok(())
    }

    fn leave_collection(&mut self) {
        self.nesting_depth = self.nesting_depth.saturating_sub(1);
    }

    fn account_collection_entry<E>(&mut self) -> Result<(), E>
    where
        E: serde::de::Error,
    {
        self.collection_entries = self.collection_entries.saturating_add(1);
        if self.collection_entries > RevisionRecordBytes::MAX_JSON_COLLECTION_ENTRIES {
            return self.reject(JsonLimit::CollectionEntries);
        }
        Ok(())
    }

    fn account_string<E>(&mut self, string_bytes: usize) -> Result<(), E>
    where
        E: serde::de::Error,
    {
        if string_bytes > RevisionRecordBytes::MAX_JSON_STRING_BYTES {
            return self.reject(JsonLimit::SingleStringBytes);
        }
        self.total_string_bytes = self.total_string_bytes.saturating_add(string_bytes);
        if self.total_string_bytes > RevisionRecordBytes::MAX_JSON_TOTAL_STRING_BYTES {
            return self.reject(JsonLimit::TotalStringBytes);
        }
        Ok(())
    }

    fn reject<E>(&mut self, limit: JsonLimit) -> Result<(), E>
    where
        E: serde::de::Error,
    {
        self.exceeded = Some(limit);
        Err(E::custom("revision record exceeds its JSON decode budget"))
    }
}

struct JsonBudgetSeed<'a> {
    budget: &'a mut JsonDecodeBudget,
}

impl<'de> DeserializeSeed<'de> for JsonBudgetSeed<'_> {
    type Value = ();

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(JsonBudgetVisitor {
            budget: self.budget,
        })
    }
}

struct JsonBudgetVisitor<'a> {
    budget: &'a mut JsonDecodeBudget,
}

impl<'de> Visitor<'de> for JsonBudgetVisitor<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("one bounded JSON value")
    }

    fn visit_bool<E>(self, _value: bool) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(())
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        JsonBudgetSeed {
            budget: self.budget,
        }
        .deserialize(deserializer)
    }

    fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.budget.account_string(value.len())
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.budget.account_string(value.len())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.budget.account_string(value.len())
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        self.budget.enter_collection()?;
        while sequence
            .next_element_seed(JsonBudgetSeed {
                budget: self.budget,
            })?
            .is_some()
        {
            self.budget.account_collection_entry()?;
        }
        self.budget.leave_collection();
        Ok(())
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        self.budget.enter_collection()?;
        while map
            .next_key_seed(JsonBudgetSeed {
                budget: self.budget,
            })?
            .is_some()
        {
            self.budget.account_collection_entry()?;
            map.next_value_seed(JsonBudgetSeed {
                budget: self.budget,
            })?;
        }
        self.budget.leave_collection();
        Ok(())
    }
}

impl fmt::Debug for RevisionRecordBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RevisionRecordBytes")
            .field("len", &self.0.len())
            .finish()
    }
}

/// Supported durable executable-plan record encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ExecutablePlanRecordFormat {
    /// Canonical Graph-v1 JSON recorded form.
    GraphV1Json,
}

/// Supported durable worker-flavor record encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WorkerFlavorRecordFormat {
    /// Version-one JSON recorded form.
    V1Json,
}

/// Opaque durable worker-flavor revision record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerFlavorRevisionRecord {
    id: WorkerFlavorRevisionId,
    format: WorkerFlavorRecordFormat,
    bytes: RevisionRecordBytes,
}

impl WorkerFlavorRevisionRecord {
    /// Construct a version-one JSON worker-flavor record.
    pub const fn v1_json(id: WorkerFlavorRevisionId, bytes: RevisionRecordBytes) -> Self {
        Self {
            id,
            format: WorkerFlavorRecordFormat::V1Json,
            bytes,
        }
    }

    /// Return the claimed worker-flavor revision identifier.
    pub const fn id(&self) -> WorkerFlavorRevisionId {
        self.id
    }

    /// Return the durable record encoding.
    pub const fn format(&self) -> WorkerFlavorRecordFormat {
        self.format
    }

    /// Borrow the opaque serialized record.
    pub fn bytes(&self) -> &[u8] {
        self.bytes.as_bytes()
    }

    /// Borrow the bounded record for checked decoding.
    pub const fn record_bytes(&self) -> &RevisionRecordBytes {
        &self.bytes
    }
}

/// One atomically persisted exact executable-plan and worker-flavor pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanFlavorRevisionRecord {
    ids: PlanFlavorRevisionIds,
    plan_format: ExecutablePlanRecordFormat,
    plan_bytes: RevisionRecordBytes,
    worker_flavor: WorkerFlavorRevisionRecord,
}

impl PlanFlavorRevisionRecord {
    /// Construct a Graph-v1 plan paired with its exact worker-flavor record.
    ///
    /// The worker-flavor identifier is derived from `worker_flavor`; callers
    /// cannot supply a second, conflicting flavor identifier.
    pub const fn graph_v1_json(
        plan_id: ExecutablePlanRevisionId,
        plan_bytes: RevisionRecordBytes,
        worker_flavor: WorkerFlavorRevisionRecord,
    ) -> Self {
        Self {
            ids: PlanFlavorRevisionIds::new(plan_id, worker_flavor.id()),
            plan_format: ExecutablePlanRecordFormat::GraphV1Json,
            plan_bytes,
            worker_flavor,
        }
    }

    /// Return the exact typed revision pair.
    pub const fn ids(&self) -> PlanFlavorRevisionIds {
        self.ids
    }

    /// Return the durable executable-plan record encoding.
    pub const fn plan_format(&self) -> ExecutablePlanRecordFormat {
        self.plan_format
    }

    /// Borrow the opaque serialized executable-plan record.
    pub fn plan_bytes(&self) -> &[u8] {
        self.plan_bytes.as_bytes()
    }

    /// Borrow the bounded executable-plan record for checked decoding.
    pub const fn plan_record_bytes(&self) -> &RevisionRecordBytes {
        &self.plan_bytes
    }

    /// Borrow the exact worker-flavor record paired with the plan.
    pub const fn worker_flavor(&self) -> &WorkerFlavorRevisionRecord {
        &self.worker_flavor
    }
}

/// Catalog lifecycle target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlanFlavorRevisionTarget {
    /// One immutable executable-plan revision.
    ExecutablePlan(ExecutablePlanRevisionId),
    /// One immutable worker-flavor revision.
    WorkerFlavor(WorkerFlavorRevisionId),
}

/// Authoritative blockers derived from revision-reference rows.
///
/// These values are projections, never independently persisted mutable
/// counters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RevisionReferenceCounts {
    live_executions: u64,
    rollback_windows: u64,
}

impl RevisionReferenceCounts {
    /// Construct reference counts derived from authoritative rows.
    pub const fn new(live_executions: u64, rollback_windows: u64) -> Self {
        Self {
            live_executions,
            rollback_windows,
        }
    }

    /// Return the number of live execution references.
    pub const fn live_executions(self) -> u64 {
        self.live_executions
    }

    /// Return the number of unexpired rollback-window references.
    pub const fn rollback_windows(self) -> u64 {
        self.rollback_windows
    }

    /// Return whether no authoritative reference blocks deletion.
    pub const fn is_empty(self) -> bool {
        self.live_executions == 0 && self.rollback_windows == 0
    }
}

/// Result of atomically inserting one exact plan/flavor pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RevisionInsertOutcome {
    /// The pair was absent and is now stored as Active.
    Inserted,
    /// Records with the same identifiers and semantically equal parsed JSON
    /// documents already existed. Incidental JSON whitespace and object-key
    /// order do not make immutable content conflict.
    AlreadyPresent,
}

/// Result of beginning one revision's drain lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BeginDrainOutcome {
    /// Active changed to Draining with the observed reference projection.
    Started(RevisionReferenceCounts),
    /// The revision was already Draining.
    AlreadyDraining(RevisionReferenceCounts),
}

/// Closed, payload-redacted exact revision catalog failure.
///
/// Variants contain only typed revision identifiers and bounded counts.
/// Serialized records, driver messages, SQL, tenant data, and authority
/// proofs never cross this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RevisionCatalogError {
    /// The exact executable-plan revision does not exist.
    #[error("executable-plan revision is unavailable")]
    PlanUnavailable {
        /// Requested executable-plan revision.
        plan_id: ExecutablePlanRevisionId,
    },

    /// The exact worker-flavor revision does not exist.
    #[error("worker-flavor revision is unavailable")]
    WorkerFlavorUnavailable {
        /// Requested worker-flavor revision.
        worker_flavor_id: WorkerFlavorRevisionId,
    },

    /// The stored plan is pinned to a different worker flavor.
    #[error("executable-plan and worker-flavor revisions do not match")]
    PlanFlavorMismatch {
        /// Exact pair requested by the caller.
        requested: PlanFlavorRevisionIds,
        /// Worker flavor pinned by the stored plan.
        stored_worker_flavor_id: WorkerFlavorRevisionId,
    },

    /// An immutable identifier already contains a semantically different parsed
    /// JSON document.
    #[error("revision identity already contains different content")]
    ContentConflict {
        /// Immutable identity whose content differs.
        target: PlanFlavorRevisionTarget,
    },

    /// A new reference lost a race with revision drain.
    #[error("revision is draining and rejects new references")]
    Draining {
        /// Draining revision.
        target: PlanFlavorRevisionTarget,
    },

    /// The immutable identity has a deletion tombstone.
    #[error("revision has been deleted")]
    Deleted {
        /// Deleted revision.
        target: PlanFlavorRevisionTarget,
    },

    /// Deletion was requested before the revision began draining.
    #[error("revision must begin draining before deletion")]
    DrainRequired {
        /// Active revision.
        target: PlanFlavorRevisionTarget,
    },

    /// Live executions or rollback windows still retain the revision.
    ///
    /// Not yet reachable in a release build: reference rows can only be
    /// created by the execution-owner transaction, which is unimplemented, so
    /// the shipped backend's reference constructors are `#[cfg(test)]`. Do not
    /// rely on its absence as evidence that a revision is unreferenced — see
    /// [`PlanFlavorCatalogAdmin::delete_drained`](crate::PlanFlavorCatalogAdmin::delete_drained).
    #[error("revision remains referenced")]
    Referenced {
        /// Revision whose deletion was rejected.
        target: PlanFlavorRevisionTarget,
        /// Authoritative reference projection observed by the operation.
        references: RevisionReferenceCounts,
    },

    /// Stored plans still depend on the worker flavor.
    #[error("worker-flavor revision still has dependent executable plans")]
    DependentPlans {
        /// Worker flavor whose deletion was rejected.
        worker_flavor_id: WorkerFlavorRevisionId,
        /// Number of non-deleted stored plans pinned to the worker flavor.
        dependent_plans: u64,
    },

    /// An opaque durable record was empty.
    #[error("revision record is empty")]
    EmptyRecord,

    /// An opaque durable record exceeded the persistence ceiling.
    #[error("revision record exceeds the byte limit")]
    RecordTooLarge {
        /// Maximum accepted serialized record size.
        max_bytes: usize,
        /// Serialized size supplied to the checked constructor.
        actual_bytes: usize,
    },

    /// JSON nesting exceeded the generic decode safety ceiling.
    #[error("revision record exceeds the JSON nesting limit")]
    RecordNestingTooDeep {
        /// Revision whose record exceeded the limit.
        target: PlanFlavorRevisionTarget,
    },

    /// One JSON string exceeded the generic decode safety ceiling.
    #[error("revision record contains an oversized JSON string")]
    RecordStringTooLarge {
        /// Revision whose record exceeded the limit.
        target: PlanFlavorRevisionTarget,
    },

    /// Aggregate JSON key and string bytes exceeded the decode budget.
    #[error("revision record exceeds the aggregate JSON string budget")]
    RecordStringBudgetExceeded {
        /// Revision whose record exceeded the budget.
        target: PlanFlavorRevisionTarget,
    },

    /// Aggregate JSON object and array entries exceeded the decode budget.
    #[error("revision record exceeds the aggregate JSON collection budget")]
    RecordCollectionBudgetExceeded {
        /// Revision whose record exceeded the budget.
        target: PlanFlavorRevisionTarget,
    },

    /// Persisted format metadata names an unsupported recorded form.
    #[error("revision record format is unsupported")]
    UnsupportedRecordFormat {
        /// Revision whose format metadata is unsupported.
        target: PlanFlavorRevisionTarget,
    },

    /// Persisted record bytes violate their recorded-form contract.
    #[error("revision record is corrupt")]
    CorruptRecord {
        /// Revision whose record is corrupt.
        target: PlanFlavorRevisionTarget,
    },

    /// The operation definitely did not commit.
    #[error("revision catalog is unavailable")]
    Unavailable,

    /// Commit was dispatched but authoritative acknowledgement was lost.
    #[error("revision catalog outcome is unknown; do not retry blindly")]
    OutcomeUnknown,
}

#[cfg(test)]
mod tests {
    use serde::de::IgnoredAny;

    use super::*;

    fn flavor_target() -> PlanFlavorRevisionTarget {
        PlanFlavorRevisionTarget::WorkerFlavor(WorkerFlavorRevisionId::from_bytes([0x51; 32]))
    }

    #[test]
    fn record_bytes_accept_exact_limit_and_reject_limit_plus_one() {
        let exact = vec![b'x'; RevisionRecordBytes::MAX_BYTES];
        let bounded = RevisionRecordBytes::try_from_vec(exact)
            .expect("a record at the byte ceiling remains admissible");
        assert_eq!(bounded.as_bytes().len(), RevisionRecordBytes::MAX_BYTES);

        let mut oversized = vec![b'y'; RevisionRecordBytes::MAX_BYTES + 1];
        oversized[.."oversized-payload-canary".len()].copy_from_slice(b"oversized-payload-canary");
        let error = RevisionRecordBytes::try_from_vec(oversized)
            .expect_err("a record one byte over the ceiling must be rejected");
        assert_eq!(
            error,
            RevisionCatalogError::RecordTooLarge {
                max_bytes: RevisionRecordBytes::MAX_BYTES,
                actual_bytes: RevisionRecordBytes::MAX_BYTES + 1,
            }
        );
        assert!(!format!("{error} {error:?}").contains("oversized-payload-canary"));
    }

    #[test]
    fn checked_json_decode_bounds_nesting() {
        let at_limit = format!(
            "{}0{}",
            "[".repeat(RevisionRecordBytes::MAX_JSON_NESTING_DEPTH),
            "]".repeat(RevisionRecordBytes::MAX_JSON_NESTING_DEPTH)
        );
        RevisionRecordBytes::try_from_vec(at_limit.into_bytes())
            .expect("depth fixture fits the byte ceiling")
            .deserialize_json::<IgnoredAny>(flavor_target())
            .expect("nesting at the ceiling remains admissible");

        let over_limit = format!(
            "{}0{}",
            "[".repeat(RevisionRecordBytes::MAX_JSON_NESTING_DEPTH + 1),
            "]".repeat(RevisionRecordBytes::MAX_JSON_NESTING_DEPTH + 1)
        );
        let error = RevisionRecordBytes::try_from_vec(over_limit.into_bytes())
            .expect("depth fixture fits the byte ceiling")
            .deserialize_json::<IgnoredAny>(flavor_target())
            .expect_err("nesting above the ceiling must be rejected before typed decoding");
        assert!(
            matches!(error, RevisionCatalogError::RecordNestingTooDeep { target } if target == flavor_target()),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn checked_json_decode_bounds_single_and_aggregate_string_bytes() {
        let exact_string = format!(
            "\"{}\"",
            "s".repeat(RevisionRecordBytes::MAX_JSON_STRING_BYTES)
        );
        RevisionRecordBytes::try_from_vec(exact_string.into_bytes())
            .expect("single-string boundary fixture fits the record")
            .deserialize_json::<IgnoredAny>(flavor_target())
            .expect("a string at the ceiling remains admissible");

        let oversized_string = format!(
            "\"{}\"",
            "s".repeat(RevisionRecordBytes::MAX_JSON_STRING_BYTES + 1)
        );
        let error = RevisionRecordBytes::try_from_vec(oversized_string.into_bytes())
            .expect("single-string overflow fixture fits the record")
            .deserialize_json::<IgnoredAny>(flavor_target())
            .expect_err("a string one byte over the ceiling must be rejected");
        assert!(
            matches!(error, RevisionCatalogError::RecordStringTooLarge { target } if target == flavor_target()),
            "unexpected error: {error:?}"
        );

        let string_bytes_per_item = RevisionRecordBytes::MAX_JSON_TOTAL_STRING_BYTES / 4;
        let exact_aggregate = format!(
            "[\"{0}\",\"{0}\",\"{0}\",\"{0}\"]",
            "a".repeat(string_bytes_per_item)
        );
        RevisionRecordBytes::try_from_vec(exact_aggregate.into_bytes())
            .expect("aggregate-string boundary fixture fits the record")
            .deserialize_json::<IgnoredAny>(flavor_target())
            .expect("aggregate string bytes at the ceiling remain admissible");

        let over_aggregate = format!(
            "[\"{0}\",\"{0}\",\"{0}\",\"{1}\"]",
            "a".repeat(string_bytes_per_item),
            "a".repeat(string_bytes_per_item + 1)
        );
        let error = RevisionRecordBytes::try_from_vec(over_aggregate.into_bytes())
            .expect("aggregate-string overflow fixture fits the record")
            .deserialize_json::<IgnoredAny>(flavor_target())
            .expect_err("aggregate string bytes above the ceiling must be rejected");
        assert!(
            matches!(error, RevisionCatalogError::RecordStringBudgetExceeded { target } if target == flavor_target()),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn checked_json_decode_bounds_aggregate_collection_entries() {
        let at_limit = format!(
            "[{}]",
            std::iter::repeat_n("0", RevisionRecordBytes::MAX_JSON_COLLECTION_ENTRIES)
                .collect::<Vec<_>>()
                .join(",")
        );
        RevisionRecordBytes::try_from_vec(at_limit.into_bytes())
            .expect("collection boundary fixture fits the record")
            .deserialize_json::<IgnoredAny>(flavor_target())
            .expect("aggregate collection count at the ceiling remains admissible");

        let over_limit = format!(
            "[{}]",
            std::iter::repeat_n("0", RevisionRecordBytes::MAX_JSON_COLLECTION_ENTRIES + 1)
                .collect::<Vec<_>>()
                .join(",")
        );
        let error = RevisionRecordBytes::try_from_vec(over_limit.into_bytes())
            .expect("collection overflow fixture fits the record")
            .deserialize_json::<IgnoredAny>(flavor_target())
            .expect_err("aggregate collection count above the ceiling must be rejected");
        assert!(
            matches!(error, RevisionCatalogError::RecordCollectionBudgetExceeded { target } if target == flavor_target()),
            "unexpected error: {error:?}"
        );
    }
}
