//! Atomic, idempotent acceptance of a keyed workflow start.
//!
//! A start key identifies **one accepted command**, not one request. Two
//! requests carrying the same key and the same canonical request must converge
//! on one durable execution identity; the same key with a different request
//! must be refused without any durable change.
//!
//! Splitting that across `ExecutionStore::create` and `ControlQueue::enqueue`
//! cannot provide it. Those are separate statements on separate connections,
//! so a crash between them leaves an execution no consumer will ever drive,
//! and neither call sees a start key at all — a retried request simply mints a
//! second execution id and creates a second execution.
//!
//! An HTTP idempotency cache does not provide it either. It can replay a
//! response body, but it is not the execution aggregate's writer: it cannot
//! stop a second execution from being created when the cached response is
//! missing, expired, or served by a process that never saw the first request.
//! The reservation has to live in the same transaction as the aggregate write.

use crate::dto::PlanFlavorRevisionIds;
use crate::error::StorageError;
use crate::ids::ExecutionContractBundleId;
use crate::scope::Scope;

/// A complete start failed before commit, or its commit acknowledgement is unknown.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StartMaterializationError {
    /// The proposed fresh aggregate, command or bundle identities disagree.
    #[error("start materialization envelope is invalid")]
    InvalidEnvelope,
    /// An execution identity already names a different original materialization.
    #[error("execution identity is bound to another materialization")]
    MaterializationConflict,
    /// Backend operation failed before commit submission.
    #[error("start materialization storage operation failed")]
    Storage(#[from] StorageError),
    /// Commit was submitted but not authoritatively acknowledged.
    #[error("start materialization commit outcome is unknown")]
    OutcomeUnknown,
}

/// Version of the request-canonicalization rules a fingerprint was computed
/// under.
///
/// Persisted alongside the digest so a future change to what "the same
/// request" means is a visible mismatch rather than a silent collision
/// between two different canonical forms.
pub type FingerprintVersion = u16;

/// Digest of one canonicalized start request.
///
/// Carries its canonicalization version, because comparing digests produced by
/// different rules is meaningless: two requests are "the same" only if the
/// same rules produced the same bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StartFingerprint {
    version: FingerprintVersion,
    digest: [u8; 32],
}

impl StartFingerprint {
    /// Build a fingerprint from a digest and the rules that produced it.
    #[must_use]
    pub const fn new(version: FingerprintVersion, digest: [u8; 32]) -> Self {
        Self { version, digest }
    }

    /// Canonicalization version.
    #[must_use]
    pub const fn version(&self) -> FingerprintVersion {
        self.version
    }

    /// Raw digest bytes.
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}

/// The exact contract identity every materialized execution carries.
///
/// Bundle, executable-plan revision, and worker-flavor revision are pinned
/// together: the bundle carries the workflow revision inside it, so this
/// triple plus the bundle's own pins is the complete non-terminal identity.
/// Identities are opaque typed ids — no payload bytes cross this seam.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StartContractIdentity {
    bundle_id: ExecutionContractBundleId,
    revisions: PlanFlavorRevisionIds,
}

impl StartContractIdentity {
    /// Pair one bundle identity with the exact plan/flavor revisions it
    /// authorizes.
    #[must_use]
    pub const fn new(
        bundle_id: ExecutionContractBundleId,
        revisions: PlanFlavorRevisionIds,
    ) -> Self {
        Self {
            bundle_id,
            revisions,
        }
    }

    /// The execution contract bundle identity recorded on the live reference.
    #[must_use]
    pub const fn bundle_id(&self) -> ExecutionContractBundleId {
        self.bundle_id
    }

    /// The exact plan/flavor revisions the execution runs under.
    #[must_use]
    pub const fn revisions(&self) -> PlanFlavorRevisionIds {
        self.revisions
    }
}

/// Why an exact plan/flavor pair was refused at start materialization.
///
/// Every variant is fail-closed and payload-redacted: nothing was written,
/// and the reply names only which identity failed admission. A revision that
/// began draining before this start committed loses the race — the start
/// does not proceed against it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartRevisionRejection {
    /// The executable-plan revision is unknown to the catalog (never inserted
    /// or already deleted).
    PlanUnavailable,
    /// The worker-flavor revision is unknown to the catalog.
    WorkerFlavorUnavailable,
    /// The exact pair exists but is not admissible: it is draining or
    /// deleted, or the plan is pinned to a different worker flavor.
    PairNotAdmitted,
}

impl From<StartRevisionRejection> for &'static str {
    fn from(rejection: StartRevisionRejection) -> Self {
        match rejection {
            StartRevisionRejection::PlanUnavailable => "plan-unavailable",
            StartRevisionRejection::WorkerFlavorUnavailable => "worker-flavor-unavailable",
            StartRevisionRejection::PairNotAdmitted => "pair-not-admitted",
        }
    }
}

/// What a materialized keyed start did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartMaterialization {
    /// This request reserved the key, the revisions were admitted, and the
    /// execution, its Start command, and its live reference were committed.
    Accepted {
        /// Durable identity of the execution this request created.
        execution_id: String,
    },
    /// The key was already reserved for an identical request. This is the
    /// original acceptance receipt; no new rows were written. The retry's
    /// contract identity is deliberately ignored: convergence is defined by
    /// the key and the request fingerprint, not by revision state.
    Replayed {
        /// Durable identity of the execution the original request created.
        execution_id: String,
    },
    /// The key is reserved for a *different* request. Nothing was written.
    ///
    /// Carries no execution id because the caller proved knowledge of a key,
    /// not of the execution behind it.
    FingerprintMismatch,
    /// The exact revisions were not admitted. Nothing was written — no
    /// reservation, execution, command, or reference row survived.
    RevisionRejected(StartRevisionRejection),
}

/// Durable owner of keyed start acceptance.
///
/// Implementations **own** every write below and must not delegate to
/// [`crate::store::ExecutionStore::create`] or
/// [`crate::store::ControlQueue::enqueue`] — those run on their own
/// connections and would break the single-commit contract.
#[async_trait::async_trait]
pub trait StartAcceptanceStore: Send + Sync + std::fmt::Debug {
    /// Read the original winner in the existing scoped trigger-event namespace.
    /// Legacy winners may have no contract bundle; this method never invents one.
    ///
    /// # Errors
    /// Returns a storage failure when the scoped reservation cannot be read.
    async fn lookup_trigger_start(
        &self,
        scope: &Scope,
        key: &crate::dto::TriggerStartKey<'_>,
    ) -> Result<Option<String>, StorageError>;
    /// Atomically materialize state, Start, immutable bundle, and live references.
    /// Existing keyed reservation replay precedes new-envelope validation/admission.
    /// An unkeyed request creates no synthetic reservation. Retrying the same
    /// execution ID compares the entire original materialization, not merely row existence.
    ///
    /// # Errors
    ///
    /// Returns [`StartMaterializationError`] when the envelope is invalid, an
    /// identity conflicts, storage fails, or commit acknowledgement is unknown.
    async fn materialize_start(
        &self,
        start: &crate::dto::MaterializedStart<'_>,
    ) -> Result<StartMaterialization, StartMaterializationError>;

    /// Read a tenant-qualified original keyed acceptance. This grants no authority.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when the scoped reservation cannot be read.
    async fn lookup_start(
        &self,
        scope: &Scope,
        key: &str,
    ) -> Result<Option<crate::dto::StartReservation>, StorageError>;

    /// Read the execution's immutable bundle within its owning tenant.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when the scoped bundle cannot be read.
    async fn read_contract_bundle(
        &self,
        scope: &Scope,
        execution_id: &str,
    ) -> Result<Option<crate::dto::StoredContractBundle>, StorageError>;
}

/// Process-wide maintenance authority for keyed-start reservations.
///
/// This port is deliberately separate from [`StartAcceptanceStore`]: eviction
/// spans every tenant and must never be reachable through a scope-bound store.
#[async_trait::async_trait]
pub trait StartReservationMaintenance: Send + Sync + std::fmt::Debug {
    /// Drop reservations older than `retention`; returns the count deleted.
    ///
    /// A reservation only has to outlive the retries that could still race it.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError`] when the global retention operation fails.
    async fn evict_reservations_older_than(
        &self,
        retention: std::time::Duration,
    ) -> Result<u64, StorageError>;
}
