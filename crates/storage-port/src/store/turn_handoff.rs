//! Durable handoff from a dispatch claim to an execution turn.
//!
//! A dispatch claim protects *delivery*, not work. Holding one for the duration
//! of an action makes the queue's reclaim timeout an implicit limit on how long
//! an action may run: a slow action outlives its claim, a sweep redelivers the
//! row, and a second worker starts the same turn while the first is still
//! running it. Extending the claim while the action runs only moves the
//! problem — it turns the queue row into a second, weaker lease competing with
//! the execution aggregate's own.
//!
//! The handoff ends the claim at a definite point instead. Runtime control
//! durably accepts ownership of the execution turn under the aggregate's
//! lease/fence **and** acknowledges the queue row in one transaction, so the
//! two can never disagree:
//!
//! - Crash before the commit: the row is still `Processing`, the reclaim sweep
//!   redelivers it, and no turn was ever accepted.
//! - Crash after the commit: the row is acknowledged and the execution holds a
//!   durable lease, so recovery drives from aggregate truth rather than from
//!   the queue.
//!
//! After the handoff the action's duration is governed by the execution lease
//! and persisted recovery state. The dispatch claim is already finished, so it
//! cannot be extended by how long the action takes.

use core::fmt;
use std::time::Duration;

use crate::error::StorageError;
use crate::ids::{FencingToken, WorkerFlavorRevisionId};
use crate::scope::Scope;
use crate::store::job_dispatch::JobClaimToken;

/// Supported command semantics accepted by the existing execution owner.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ControlTurnCommand {
    /// Arm only the target recorded in the actual persisted Resume command.
    Resume {
        /// `None` preserves the existing untargeted Resume semantics.
        target: Option<crate::dto::ResumeTarget>,
    },
    /// Re-drive a supported nonterminal execution; this grants no rewind capability.
    Restart,
}

impl fmt::Debug for ControlTurnCommand {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl ControlTurnCommand {
    /// Exact command text persisted in the control queue.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Resume { .. } => "Resume",
            Self::Restart => "Restart",
        }
    }

    /// Expected target; storage compares it with the actual claimed row.
    #[must_use]
    pub fn target(&self) -> Option<&crate::dto::ResumeTarget> {
        match self {
            Self::Resume { target } => target.as_ref(),
            Self::Restart => None,
        }
    }
}

/// Single source of scope, execution CAS and actual owner fence.
#[derive(Clone, Copy)]
#[non_exhaustive]
pub enum ControlTurnTransition<'a> {
    /// Accept an existing-state re-drive without inventing a state transition.
    Unchanged {
        /// Execution tenant.
        scope: &'a Scope,
        /// Scoped execution identifier.
        execution_id: &'a str,
        /// Version validated by runtime preflight.
        expected_version: u64,
        /// Actual live owner token, never a historical generation.
        fence: FencingToken,
    },
    /// Existing owner checkpoint, including all journal and outbox effects.
    Checkpoint(&'a crate::TransitionBatch),
}

impl ControlTurnTransition<'_> {
    /// Tenant controlling this transition.
    #[must_use]
    pub fn scope(&self) -> &Scope {
        match self {
            Self::Unchanged { scope, .. } => scope,
            Self::Checkpoint(batch) => batch.scope(),
        }
    }
    /// Execution being accepted.
    #[must_use]
    pub fn execution_id(&self) -> &str {
        match self {
            Self::Unchanged { execution_id, .. } => execution_id,
            Self::Checkpoint(batch) => batch.execution_id(),
        }
    }
    /// Expected persisted CAS version.
    #[must_use]
    pub fn expected_version(&self) -> u64 {
        match self {
            Self::Unchanged {
                expected_version, ..
            } => *expected_version,
            Self::Checkpoint(batch) => batch.expected_version(),
        }
    }
    /// Actual owner fence to validate under the aggregate lock.
    #[must_use]
    pub fn fence(&self) -> FencingToken {
        match self {
            Self::Unchanged { fence, .. } => *fence,
            Self::Checkpoint(batch) => batch.fencing(),
        }
    }
}

/// Atomically accept a claimed Resume/Restart under an already held execution lease.
/// Runtime owns signal matching and state-machine eligibility; these are technical
/// inputs whose persisted scope, command, target, CAS and fence storage rechecks.
pub struct ControlTurnCommit<'a> {
    /// Current control claim.
    claim: crate::store::ControlClaimToken,
    /// Exact live revision reference validated by runtime preflight.
    worker_flavor_revision_id: WorkerFlavorRevisionId,
    /// Expected command and immutable target.
    command: ControlTurnCommand,
    /// State checkpoint or unchanged re-drive, with its sole identity/fence source.
    transition: ControlTurnTransition<'a>,
}

impl<'a> ControlTurnCommit<'a> {
    /// Bind a claimed control command to its exact revision and owner transition.
    pub const fn new(
        claim: crate::store::ControlClaimToken,
        worker_flavor_revision_id: WorkerFlavorRevisionId,
        command: ControlTurnCommand,
        transition: ControlTurnTransition<'a>,
    ) -> Self {
        Self {
            claim,
            worker_flavor_revision_id,
            command,
            transition,
        }
    }

    /// Current control claim.
    pub const fn claim(&self) -> crate::store::ControlClaimToken {
        self.claim
    }

    /// Exact live revision validated by runtime preflight.
    pub const fn worker_flavor_revision_id(&self) -> WorkerFlavorRevisionId {
        self.worker_flavor_revision_id
    }

    /// Expected command and immutable target.
    pub const fn command(&self) -> &ControlTurnCommand {
        &self.command
    }

    /// Checkpoint or unchanged transition owned by the current execution fence.
    pub const fn transition(&self) -> &ControlTurnTransition<'a> {
        &self.transition
    }

    /// Replace the expected command while preserving the claim and transition.
    pub fn with_command(mut self, command: ControlTurnCommand) -> Self {
        self.command = command;
        self
    }

    /// Replace the exact worker revision while preserving the claim and transition.
    pub const fn with_worker_flavor_revision_id(
        mut self,
        worker_flavor_revision_id: WorkerFlavorRevisionId,
    ) -> Self {
        self.worker_flavor_revision_id = worker_flavor_revision_id;
        self
    }

    /// Replace the owner transition while preserving the claim and command.
    pub const fn with_transition(mut self, transition: ControlTurnTransition<'a>) -> Self {
        self.transition = transition;
        self
    }
}

impl fmt::Debug for ControlTurnCommit<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ControlTurnCommit")
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

/// Rejections leave checkpoint, marker and queue untouched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum ControlTurnCommitOutcome {
    /// The current owner may continue; this does not mint another lease.
    Accepted {
        /// Validated existing fence.
        fence: FencingToken,
        /// Persisted version after the optional checkpoint.
        new_version: u64,
    },
    /// Claim, command, scope, target or exact live reference no longer matches.
    ClaimSuperseded,
    /// Owner lease was absent, expired or superseded.
    FencedOut,
    /// Runtime must reload and preflight the current aggregate.
    VersionConflict {
        /// Current version within the matched tenant only.
        actual: u64,
    },
}

/// Everything one durable handoff needs, in one transaction.
#[derive(Debug, Clone)]
pub struct TurnHandoff<'a> {
    /// Tenant that owns the execution and the queue row.
    scope: &'a Scope,
    /// Execution whose turn is being accepted.
    execution_id: &'a str,
    /// Proof the caller currently owns the queue row it is acknowledging.
    ///
    /// A processor identity cannot serve here: a stable processor may claim a
    /// row, lose it to a reclaim sweep, and claim it again, so an
    /// acknowledgement issued against the first claim would terminalise a row
    /// the second claim is still working.
    claim: JobClaimToken,
    /// Exact worker-flavor revision proven during dispatch admission.
    ///
    /// Storage rechecks this value against both the claimed queue row and the
    /// execution's live revision reference before it leases the aggregate.
    worker_flavor_revision_id: WorkerFlavorRevisionId,
    /// Identity recorded as the execution's lease holder.
    holder: &'a str,
    /// How long the accepted turn's lease is valid for.
    ///
    /// This bounds recovery latency after a crash, not how long the action may
    /// run: a live owner renews it, and the queue claim is already finished.
    ///
    /// Every backend clamps it to `[1s, 24h]`, exactly as
    /// [`crate::store::ExecutionStore::acquire_lease`] does, so a handoff cannot
    /// mint a lease the execution store would have refused. `Duration::ZERO`
    /// therefore becomes one second rather than an already-expired lease — a
    /// caller cannot acknowledge a queue row and be left with no durable
    /// ownership.
    lease_ttl: Duration,
}

impl<'a> TurnHandoff<'a> {
    /// Bind a dispatch claim to the exact execution and worker revision.
    pub const fn for_claim(
        scope: &'a Scope,
        execution_id: &'a str,
        claim: JobClaimToken,
        worker_flavor_revision_id: WorkerFlavorRevisionId,
    ) -> TurnHandoffLease<'a> {
        TurnHandoffLease {
            scope,
            execution_id,
            claim,
            worker_flavor_revision_id,
        }
    }

    /// Tenant owning the execution and queue row.
    pub const fn scope(&self) -> &Scope {
        self.scope
    }
    /// Execution whose turn is being accepted.
    pub const fn execution_id(&self) -> &str {
        self.execution_id
    }
    /// Current generation-bound dispatch claim.
    pub const fn claim(&self) -> JobClaimToken {
        self.claim
    }
    /// Exact worker revision admitted for this execution.
    pub const fn worker_flavor_revision_id(&self) -> WorkerFlavorRevisionId {
        self.worker_flavor_revision_id
    }

    /// Execution lease holder identity.
    pub const fn holder(&self) -> &str {
        self.holder
    }
    /// Requested execution lease lifetime.
    pub const fn lease_ttl(&self) -> Duration {
        self.lease_ttl
    }
}

/// Dispatch claim awaiting its explicit execution lease request.
#[derive(Debug, Clone)]
#[must_use = "finish the handoff with `lease_to`"]
pub struct TurnHandoffLease<'a> {
    scope: &'a Scope,
    execution_id: &'a str,
    claim: JobClaimToken,
    worker_flavor_revision_id: WorkerFlavorRevisionId,
}

impl<'a> TurnHandoffLease<'a> {
    /// Request the execution lease that atomically completes this dispatch claim.
    pub const fn lease_to(self, holder: &'a str, lease_ttl: Duration) -> TurnHandoff<'a> {
        TurnHandoff {
            scope: self.scope,
            execution_id: self.execution_id,
            claim: self.claim,
            worker_flavor_revision_id: self.worker_flavor_revision_id,
            holder,
            lease_ttl,
        }
    }
}

/// Exact preflight and current control claim to hand to the execution owner.
///
/// These fields are technical data. The backend rechecks their relationship to
/// the actual scoped Start row, immutable revision reference and execution CAS.
#[derive(Debug, Clone)]
pub struct ControlStartHandoff<'a> {
    /// Tenant owning both rows.
    scope: &'a Scope,
    /// Execution validated by the runtime's exact preflight.
    execution_id: &'a str,
    /// Current claim on the persisted Start command.
    claim: crate::store::ControlClaimToken,
    /// Holder recorded on the accepted execution lease.
    holder: &'a str,
    /// Backend-clock lease duration, clamped to one second through one day.
    lease_ttl: Duration,
    /// Execution version observed during exact preflight.
    expected_execution_version: u64,
    /// Exact worker flavor validated during preflight.
    worker_flavor_revision_id: WorkerFlavorRevisionId,
}

impl<'a> ControlStartHandoff<'a> {
    /// Bind an exact Start claim to its execution and worker revision.
    pub const fn for_claim(
        scope: &'a Scope,
        execution_id: &'a str,
        claim: crate::store::ControlClaimToken,
        worker_flavor_revision_id: WorkerFlavorRevisionId,
    ) -> ControlStartPreflight<'a> {
        ControlStartPreflight {
            scope,
            execution_id,
            claim,
            worker_flavor_revision_id,
        }
    }

    /// Tenant owning the execution and control row.
    pub const fn scope(&self) -> &Scope {
        self.scope
    }
    /// Execution validated by exact preflight.
    pub const fn execution_id(&self) -> &str {
        self.execution_id
    }
    /// Current generation-bound control claim.
    pub const fn claim(&self) -> crate::store::ControlClaimToken {
        self.claim
    }
    /// Holder requested for the execution lease.
    pub const fn holder(&self) -> &str {
        self.holder
    }
    /// Requested execution lease lifetime.
    pub const fn lease_ttl(&self) -> Duration {
        self.lease_ttl
    }
    /// Execution version observed by exact preflight.
    pub const fn expected_execution_version(&self) -> u64 {
        self.expected_execution_version
    }
    /// Exact worker revision observed by exact preflight.
    pub const fn worker_flavor_revision_id(&self) -> WorkerFlavorRevisionId {
        self.worker_flavor_revision_id
    }
    /// Rebind the request to a different tenant for policy and conformance checks.
    pub const fn with_scope(mut self, scope: &'a Scope) -> Self {
        self.scope = scope;
        self
    }
    /// Rebind the request to a different execution for policy and conformance checks.
    pub const fn with_execution_id(mut self, execution_id: &'a str) -> Self {
        self.execution_id = execution_id;
        self
    }
    /// Replace the current claim proof.
    pub const fn with_claim(mut self, claim: crate::store::ControlClaimToken) -> Self {
        self.claim = claim;
        self
    }
    /// Replace the expected execution version.
    pub const fn with_expected_execution_version(mut self, version: u64) -> Self {
        self.expected_execution_version = version;
        self
    }
    /// Replace the exact worker revision.
    pub const fn with_worker_flavor_revision_id(
        mut self,
        worker_flavor_revision_id: WorkerFlavorRevisionId,
    ) -> Self {
        self.worker_flavor_revision_id = worker_flavor_revision_id;
        self
    }
    /// Replace the requested lease lifetime.
    pub const fn with_lease_ttl(mut self, lease_ttl: Duration) -> Self {
        self.lease_ttl = lease_ttl;
        self
    }
}

/// Claimed Start awaiting its exact execution preflight version.
#[derive(Debug, Clone)]
#[must_use = "bind the preflight version with `at_version`"]
pub struct ControlStartPreflight<'a> {
    scope: &'a Scope,
    execution_id: &'a str,
    claim: crate::store::ControlClaimToken,
    worker_flavor_revision_id: WorkerFlavorRevisionId,
}

impl<'a> ControlStartPreflight<'a> {
    /// Bind the execution version observed during exact preflight.
    pub const fn at_version(self, expected_execution_version: u64) -> ControlStartLease<'a> {
        ControlStartLease {
            scope: self.scope,
            execution_id: self.execution_id,
            claim: self.claim,
            worker_flavor_revision_id: self.worker_flavor_revision_id,
            expected_execution_version,
        }
    }
}

/// Preflighted Start awaiting its explicit execution lease request.
#[derive(Debug, Clone)]
#[must_use = "finish the handoff with `lease_to`"]
pub struct ControlStartLease<'a> {
    scope: &'a Scope,
    execution_id: &'a str,
    claim: crate::store::ControlClaimToken,
    worker_flavor_revision_id: WorkerFlavorRevisionId,
    expected_execution_version: u64,
}

impl<'a> ControlStartLease<'a> {
    /// Request the execution lease that atomically completes this Start claim.
    pub const fn lease_to(self, holder: &'a str, lease_ttl: Duration) -> ControlStartHandoff<'a> {
        ControlStartHandoff {
            scope: self.scope,
            execution_id: self.execution_id,
            claim: self.claim,
            holder,
            lease_ttl,
            expected_execution_version: self.expected_execution_version,
            worker_flavor_revision_id: self.worker_flavor_revision_id,
        }
    }
}

/// Atomic Control Start handoff decision; rejected decisions make no writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum ControlStartAcceptance {
    /// Start is completed and the execution lease is durably owned.
    Accepted {
        /// Fence for the accepted execution turn.
        fence: FencingToken,
    },
    /// The claim no longer identifies a current scoped Start for this exact flavor.
    ClaimSuperseded,
    /// An existing live execution lease prevents this handoff.
    TurnHeldByAnotherOwner,
    /// Execution changed after runtime preflight; the caller must preflight again.
    VersionConflict {
        /// Current persisted version in the already matched tenant.
        actual: u64,
    },
}

/// Advisory discovery record; its historical generation is never a lease grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverableTurn {
    /// Tenant of the originally accepted execution.
    scope: Scope,
    /// Execution requiring runtime eligibility and exact preflight.
    execution_id: String,
    /// Marker generation to recheck after preflight.
    accepted_fencing_generation: u64,
}

impl RecoverableTurn {
    /// Record a recovery candidate discovered from an accepted-turn marker.
    pub fn new(scope: Scope, execution_id: String, accepted_fencing_generation: u64) -> Self {
        Self {
            scope,
            execution_id,
            accepted_fencing_generation,
        }
    }
    /// Tenant of the accepted execution.
    pub const fn scope(&self) -> &Scope {
        &self.scope
    }
    /// Execution requiring exact runtime preflight.
    pub fn execution_id(&self) -> &str {
        &self.execution_id
    }
    /// Historical marker generation to recheck during acceptance.
    pub const fn accepted_fencing_generation(&self) -> u64 {
        self.accepted_fencing_generation
    }
}

/// Bounded scan page, including progress past currently leased executions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverableTurnPage {
    /// Accepted executions whose leases were absent or expired when scanned.
    turns: Vec<RecoverableTurn>,
    /// Exclusive last scanned execution, or `None` when this pass is exhausted.
    /// An empty `turns` vector can still carry a cursor and must not reset progress.
    next_cursor: Option<String>,
}

impl RecoverableTurnPage {
    /// Build a bounded discovery page and its exclusive continuation cursor.
    pub fn new(turns: Vec<RecoverableTurn>, next_cursor: Option<String>) -> Self {
        Self { turns, next_cursor }
    }
    /// Candidates discovered in this page.
    pub fn turns(&self) -> &[RecoverableTurn] {
        &self.turns
    }
    /// Exclusive continuation cursor, when more rows may remain.
    pub fn next_cursor(&self) -> Option<&str> {
        self.next_cursor.as_deref()
    }
    /// Consume the page into candidates and its continuation cursor.
    pub fn into_parts(self) -> (Vec<RecoverableTurn>, Option<String>) {
        (self.turns, self.next_cursor)
    }
}

/// Runtime's exact preflight applied to an advisory recovery candidate.
#[derive(Debug, Clone)]
pub struct RecoveryTurnHandoff<'a> {
    /// Execution tenant.
    scope: &'a Scope,
    /// Execution to recover.
    execution_id: &'a str,
    /// New holder recorded only if the atomic recheck succeeds.
    holder: &'a str,
    /// Backend-authored lease lifetime, clamped to one second through one day.
    lease_ttl: Duration,
    /// Actual execution version validated during runtime preflight.
    expected_execution_version: u64,
    /// Exact retained flavor validated by that preflight.
    worker_flavor_revision_id: WorkerFlavorRevisionId,
    /// Historical marker generation returned by discovery, not write authority.
    expected_accepted_fencing_generation: u64,
}

impl<'a> RecoveryTurnHandoff<'a> {
    /// Bind a discovered marker to its exact execution and worker revision.
    pub const fn for_candidate(
        scope: &'a Scope,
        execution_id: &'a str,
        worker_flavor_revision_id: WorkerFlavorRevisionId,
        expected_accepted_fencing_generation: u64,
    ) -> RecoveryTurnPreflight<'a> {
        RecoveryTurnPreflight {
            scope,
            execution_id,
            worker_flavor_revision_id,
            expected_accepted_fencing_generation,
        }
    }
    /// Tenant of the recovery candidate.
    pub const fn scope(&self) -> &Scope {
        self.scope
    }
    /// Execution requiring recovery.
    pub const fn execution_id(&self) -> &str {
        self.execution_id
    }
    /// New execution lease holder.
    pub const fn holder(&self) -> &str {
        self.holder
    }
    /// Requested execution lease lifetime.
    pub const fn lease_ttl(&self) -> Duration {
        self.lease_ttl
    }
    /// Execution version validated during preflight.
    pub const fn expected_execution_version(&self) -> u64 {
        self.expected_execution_version
    }
    /// Exact retained worker revision validated during preflight.
    pub const fn worker_flavor_revision_id(&self) -> WorkerFlavorRevisionId {
        self.worker_flavor_revision_id
    }
    /// Historical marker generation that acceptance must recheck.
    pub const fn expected_accepted_fencing_generation(&self) -> u64 {
        self.expected_accepted_fencing_generation
    }

    /// Rebind the candidate to a different tenant for policy and conformance checks.
    pub const fn with_scope(mut self, scope: &'a Scope) -> Self {
        self.scope = scope;
        self
    }
    /// Replace the exact worker revision validated during preflight.
    pub const fn with_worker_flavor_revision_id(
        mut self,
        worker_flavor_revision_id: WorkerFlavorRevisionId,
    ) -> Self {
        self.worker_flavor_revision_id = worker_flavor_revision_id;
        self
    }
    /// Replace the expected marker generation.
    pub const fn with_expected_accepted_fencing_generation(mut self, generation: u64) -> Self {
        self.expected_accepted_fencing_generation = generation;
        self
    }
    /// Replace the execution version observed during preflight.
    pub const fn with_expected_execution_version(mut self, version: u64) -> Self {
        self.expected_execution_version = version;
        self
    }
    /// Replace the requested lease lifetime.
    pub const fn with_lease_ttl(mut self, lease_ttl: Duration) -> Self {
        self.lease_ttl = lease_ttl;
        self
    }
}

/// Recovery candidate awaiting its exact execution preflight version.
#[derive(Debug, Clone)]
#[must_use = "bind the preflight version with `at_version`"]
pub struct RecoveryTurnPreflight<'a> {
    scope: &'a Scope,
    execution_id: &'a str,
    worker_flavor_revision_id: WorkerFlavorRevisionId,
    expected_accepted_fencing_generation: u64,
}

impl<'a> RecoveryTurnPreflight<'a> {
    /// Bind the execution version observed during exact preflight.
    pub const fn at_version(self, expected_execution_version: u64) -> RecoveryTurnLease<'a> {
        RecoveryTurnLease {
            scope: self.scope,
            execution_id: self.execution_id,
            worker_flavor_revision_id: self.worker_flavor_revision_id,
            expected_accepted_fencing_generation: self.expected_accepted_fencing_generation,
            expected_execution_version,
        }
    }
}

/// Preflighted recovery candidate awaiting its explicit lease request.
#[derive(Debug, Clone)]
#[must_use = "finish the handoff with `lease_to`"]
pub struct RecoveryTurnLease<'a> {
    scope: &'a Scope,
    execution_id: &'a str,
    worker_flavor_revision_id: WorkerFlavorRevisionId,
    expected_accepted_fencing_generation: u64,
    expected_execution_version: u64,
}

impl<'a> RecoveryTurnLease<'a> {
    /// Request a fresh execution lease after exact recovery preflight.
    pub const fn lease_to(self, holder: &'a str, lease_ttl: Duration) -> RecoveryTurnHandoff<'a> {
        RecoveryTurnHandoff {
            scope: self.scope,
            execution_id: self.execution_id,
            holder,
            lease_ttl,
            expected_execution_version: self.expected_execution_version,
            worker_flavor_revision_id: self.worker_flavor_revision_id,
            expected_accepted_fencing_generation: self.expected_accepted_fencing_generation,
        }
    }
}

/// Atomic recovery acceptance; rejected candidates change neither lease nor marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum RecoveryTurnAcceptance {
    /// A fresh, durably acknowledged recovery lease.
    Accepted {
        /// Actual execution fence; historical candidate values cannot replace it.
        fence: FencingToken,
    },
    /// Marker, tenant or live exact reference no longer matches this candidate.
    CandidateSuperseded,
    /// A live execution lease currently prevents recovery.
    TurnHeldByAnotherOwner,
    /// Execution changed after preflight.
    VersionConflict {
        /// Current version within the already matched tenant.
        actual: u64,
    },
}

/// What a durable handoff did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum TurnAcceptance {
    /// The turn is durably owned and the queue row is acknowledged.
    ///
    /// The fence is the execution aggregate's, so every subsequent write this
    /// owner makes is rejected once a reclaim supersedes it.
    Accepted {
        /// Fence proving ownership of the accepted turn.
        fence: FencingToken,
    },
    /// The claim was superseded before the handoff committed.
    ///
    /// Nothing was written: neither the lease nor the queue row moved. The
    /// caller must not begin the turn — another worker already holds the row.
    ClaimSuperseded,
    /// Another holder owns a live lease on this execution.
    ///
    /// Nothing was written, and the queue row is deliberately left
    /// unacknowledged so the sweep can redeliver it once that lease expires.
    /// Acknowledging here would drop the turn on the floor: the row would be
    /// terminal while no owner ever ran it.
    TurnHeldByAnotherOwner,
}

/// Durable owner of the dispatch-claim → execution-turn handoff.
///
/// Implementations **own** both writes and must not delegate to
/// [`crate::store::ExecutionStore::acquire_lease`] plus
/// [`crate::store::JobDispatchQueue::mark_dispatched`] — those run as separate
/// operations, and a crash between them is exactly the state this capability
/// exists to make unreachable.
#[async_trait::async_trait]
pub trait ExecutionTurnHandoff: Send + Sync + fmt::Debug {
    /// Commit an existing owner's checkpoint, recovery marker and queue completion.
    ///
    /// All three writes share one atomic boundary. No lease is acquired here.
    /// An unknown commit acknowledgement permits neither new frontier work nor
    /// caller-side queue cleanup; durable recovery resolves the accepted marker.
    ///
    /// # Errors
    /// Invalid envelopes and backend failures return bounded errors. Commit
    /// acknowledgement loss returns [`StorageError::AcknowledgementUnknown`].
    async fn commit_control_turn(
        &self,
        commit: &ControlTurnCommit<'_>,
    ) -> Result<ControlTurnCommitOutcome, StorageError>;

    /// Atomically complete a current Control Start and acquire its execution lease.
    ///
    /// Validate persisted command kind, scope, execution, claim generation and
    /// immutable worker flavor, then execution CAS and lease availability, before
    /// changing either row. Retained references remain routable during catalog
    /// drain; this is ownership transfer for an already admitted execution.
    /// A replayed or foreign claim grants no turn. Only a successful fresh
    /// acknowledgement returns an execution fence; an uncertain commit cannot
    /// be converted to authority by reading the rows afterward.
    ///
    /// # Errors
    /// Backend failures or invalid numeric representations return a bounded
    /// storage error. Rejections before commit leave both rows unchanged.
    /// A lost commit acknowledgement may have completed both writes.
    async fn accept_control_start(
        &self,
        handoff: &ControlStartHandoff<'_>,
    ) -> Result<ControlStartAcceptance, StorageError>;

    /// Accept the execution turn and acknowledge the queue row, committing once.
    ///
    /// **Ordering (all backends), inside one transaction:**
    ///
    /// 1. Re-check the queue row is still `Processing` at the claim's
    ///    generation and its typed worker flavor matches the execution's live
    ///    revision reference. A superseded claim or revision returns
    ///    [`TurnAcceptance::ClaimSuperseded`] with nothing written.
    /// 2. Acquire the execution lease for `holder`. A live lease held by
    ///    someone else returns [`TurnAcceptance::TurnHeldByAnotherOwner`], again
    ///    with nothing written — including no queue acknowledgement.
    /// 3. Acknowledge the queue row.
    /// 4. Commit, returning [`TurnAcceptance::Accepted`] with the fence.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::NotFound`] when the queue row or the execution
    /// does not exist, and a connection error when the backend is unreachable.
    /// A failure at any step rolls back the whole transaction, so a turn is
    /// never accepted without its acknowledgement and a row is never
    /// acknowledged without an owner.
    async fn accept_turn(&self, handoff: &TurnHandoff<'_>) -> Result<TurnAcceptance, StorageError>;
}

/// Worker-only discovery and acceptance of abandoned durable turns.
///
/// This capability deliberately spans tenants. It is separate from
/// [`ExecutionTurnHandoff`] so a tenant-bound caller can receive the handoff
/// capability without also receiving a global enumeration surface.
#[async_trait::async_trait]
pub trait TurnRecovery: Send + Sync + fmt::Debug {
    /// Scan at most `limit` accepted markers with live references for this exact flavor.
    ///
    /// `limit` must be in `1..=256`. Apply flavor and exclusive cursor before the
    /// page limit; evaluate lease expiry using backend time afterward. The cursor
    /// advances past every scanned row, including live leases. Runtime still owns
    /// execution-state eligibility and exact preflight.
    ///
    /// # Errors
    /// Invalid limits and backend failures return bounded storage errors.
    async fn list_recoverable_turns(
        &self,
        flavor: WorkerFlavorRevisionId,
        after: Option<&str>,
        limit: u32,
    ) -> Result<RecoverableTurnPage, StorageError>;

    /// Recheck marker, exact live reference, CAS and expired lease, then grant once.
    ///
    /// Lease acquisition and marker generation advance commit atomically. A lost
    /// acknowledgement grants no authority; recovery must discover and preflight
    /// again after the uncertain lease expires. Historical executions without an
    /// accepted marker are never inferred from leases or completed queue rows.
    ///
    /// # Errors
    /// Backend and malformed numeric values fail closed. Commit uncertainty returns
    /// [`StorageError::AcknowledgementUnknown`] without a fence.
    async fn accept_recovery_turn(
        &self,
        handoff: &RecoveryTurnHandoff<'_>,
    ) -> Result<RecoveryTurnAcceptance, StorageError>;
}
