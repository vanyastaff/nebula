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

use crate::dto::{ControlMsg, NewExecution, PlanFlavorRevisionIds};
use crate::error::StorageError;
use crate::ids::ExecutionContractBundleId;
use crate::scope::Scope;

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

/// Everything one keyed start acceptance must write, in one transaction.
///
/// `execution` and `command` are the aggregate row and the Start control row;
/// both are written only when this request wins the reservation.
#[derive(Debug)]
pub struct KeyedStart<'a> {
    /// Tenant that owns the reservation. Two tenants never share a start key.
    pub scope: &'a Scope,
    /// Caller-supplied key identifying this accepted command.
    pub start_key: &'a str,
    /// Fingerprint of the canonicalized request behind this key.
    pub fingerprint: StartFingerprint,
    /// Execution identity to materialize when this request wins.
    pub execution_id: &'a str,
    /// Workflow identity and initial state for the new aggregate.
    pub execution: NewExecution<'a>,
    /// The Start control row to enqueue in the same transaction.
    pub command: &'a ControlMsg,
}

/// The exact contract identity every materialized execution carries (NS01).
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

/// Everything one **materialized** keyed start must write, in one transaction.
///
/// A superset of [`KeyedStart`]: besides the reservation, the execution
/// aggregate, and the Start command, the execution's live plan/flavor
/// reference is admitted and persisted under the same commit. Until a caller
/// can supply a real [`StartContractIdentity`], the transitional
/// [`StartAcceptanceStore::accept_keyed_start`] remains the keyed path.
#[derive(Debug)]
pub struct MaterializedKeyedStart<'a> {
    /// The keyed acceptance identity: reservation, aggregate, Start command.
    pub keyed: KeyedStart<'a>,
    /// The exact contract identity to admit and persist as the execution's
    /// live reference.
    pub identity: StartContractIdentity,
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
    /// Carries no execution id for the same reason
    /// [`StartAcceptance::FingerprintMismatch`] does: the caller proved
    /// knowledge of a key, not of the execution behind it.
    FingerprintMismatch,
    /// The exact revisions were not admitted. Nothing was written — no
    /// reservation, execution, command, or reference row survived.
    RevisionRejected(StartRevisionRejection),
}

/// What a keyed start acceptance did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartAcceptance {
    /// This request reserved the key and created the execution.
    Accepted {
        /// Durable identity of the execution this request created.
        execution_id: String,
    },
    /// The key was already reserved for an identical request. This is the
    /// original acceptance receipt; no new rows were written.
    Replayed {
        /// Durable identity of the execution the original request created.
        execution_id: String,
    },
    /// The key is reserved for a *different* request. Nothing was written.
    ///
    /// Deliberately carries no execution id: the caller proved knowledge of a
    /// key, not of the execution behind it, and echoing the id would turn a
    /// guessed key into a cross-request identity oracle.
    FingerprintMismatch,
}

/// Durable owner of keyed start acceptance.
///
/// Implementations **own** every write below and must not delegate to
/// [`crate::store::ExecutionStore::create`] or
/// [`crate::store::ControlQueue::enqueue`] — those run on their own
/// connections and would break the single-commit contract.
#[async_trait::async_trait]
pub trait StartAcceptanceStore: Send + Sync + std::fmt::Debug {
    /// Reserve `(scope, start_key)`, materialize the execution, and enqueue
    /// the Start command — committing exactly once.
    ///
    /// **Ordering (all backends), inside one transaction:**
    ///
    /// 1. Insert the reservation with `ON CONFLICT DO NOTHING`.
    ///    - Inserted: continue to 2.
    ///    - Conflicted: read the stored fingerprint back *in the same
    ///      transaction* and return [`StartAcceptance::Replayed`] when it
    ///      matches, [`StartAcceptance::FingerprintMismatch`] when it does
    ///      not. Either way, no further writes.
    /// 2. Insert the execution aggregate row.
    /// 3. Insert the Start control row.
    /// 4. Commit, returning [`StartAcceptance::Accepted`].
    ///
    /// A failure at 2 or 3 rolls the reservation back with everything else, so
    /// a key is never left reserved for an execution that does not exist.
    async fn accept_keyed_start(
        &self,
        start: &KeyedStart<'_>,
    ) -> Result<StartAcceptance, StorageError>;

    /// Reserve the start key, admit the exact revisions, and materialize the
    /// execution, its Start command, and its live revision reference —
    /// committing exactly once (NS01).
    ///
    /// **Ordering (all backends), inside one transaction:**
    ///
    /// 1. If `(scope, start_key)` is already reserved, read the stored
    ///    fingerprint back *in the same transaction* and return
    ///    [`StartMaterialization::Replayed`] when it matches or
    ///    [`StartMaterialization::FingerprintMismatch`] when it does not.
    ///    Either way, no further writes — a replay converges on the original
    ///    receipt regardless of the retry's contract identity.
    /// 2. Admit the exact plan/flavor pair: both revisions must exist and be
    ///    Active, and the plan must be pinned to the requested flavor. A pair
    ///    that is missing, draining, deleted, or mismatched returns
    ///    [`StartMaterialization::RevisionRejected`] with zero durable delta —
    ///    on SQL backends the tentative reservation insert rolls back with the
    ///    transaction; in-memory backends validate before writing.
    /// 3. Insert the execution aggregate row.
    /// 4. Insert the Start control row.
    /// 5. Insert the execution's live revision reference under the admitted
    ///    pair and the supplied bundle identity.
    /// 6. Commit, returning [`StartMaterialization::Accepted`].
    ///
    /// The reservation, the reference admission, and the aggregate writes are
    /// one commit: a crash cannot leave a key reserved for a missing
    /// execution, an execution without a Start command, or a live reference
    /// whose execution does not exist.
    async fn materialize_keyed_start(
        &self,
        start: &MaterializedKeyedStart<'_>,
    ) -> Result<StartMaterialization, StorageError>;

    /// Drop reservations older than `retention`; returns the count deleted.
    ///
    /// A reservation only has to outlive the retries that could still race it.
    async fn evict_reservations_older_than(
        &self,
        retention: std::time::Duration,
    ) -> Result<u64, StorageError>;
}
