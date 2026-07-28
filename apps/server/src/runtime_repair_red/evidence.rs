//! Deterministic, authority-free evidence controls for the RED profile.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use nebula_core::{NodeKey, accessor::Clock, id::ExecutionId};
use nebula_engine::ExecutionEvent;
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;
use tokio::sync::watch;

const MAX_EVIDENCE_NAME_BYTES: usize = 96;
const MAX_ARTIFACT_ENTRIES: usize = 4_096;
const MAX_OBSERVATION_WAIT: Duration = Duration::from_mins(1);
const MAX_LIFECYCLE_OBSERVATIONS: usize = 16_384;

/// A validated, bounded name for a deterministic evidence point.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EvidencePoint(Arc<str>);

impl EvidencePoint {
    /// Parse a bounded ASCII evidence-point name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is empty, too long, or contains
    /// characters outside lowercase kebab-case plus digits.
    pub fn parse(name: impl AsRef<str>) -> Result<Self, EvidenceIntegrityError> {
        let name = name.as_ref();
        let is_valid = !name.is_empty()
            && name.len() <= MAX_EVIDENCE_NAME_BYTES
            && !name.starts_with('-')
            && !name.ends_with('-')
            && !name.contains("--")
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !is_valid {
            return Err(EvidenceIntegrityError::InvalidEvidencePoint);
        }
        Ok(Self(Arc::from(name)))
    }

    /// Return the validated evidence-point name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Monotonic manual clock used by the profile and its scenarios.
#[derive(Debug, Clone)]
pub struct ManualClock {
    inner: Arc<ManualClockInner>,
}

#[derive(Debug)]
struct ManualClockInner {
    state: Mutex<ManualClockState>,
    changes: watch::Sender<u64>,
}

#[derive(Debug, Clone, Copy)]
struct ManualClockState {
    now_millis: u64,
    wall_time: DateTime<Utc>,
    monotonic: Instant,
}

impl ManualClock {
    /// Create a clock at the supplied deterministic millisecond instant.
    ///
    /// Milliseconds are interpreted as a non-negative Unix timestamp for wall
    /// time. Monotonic time starts at the process-local `Instant` captured by
    /// this constructor and advances by the delta from `initial_millis`.
    ///
    /// # Errors
    ///
    /// Returns an error when the initial wall-clock value is outside Chrono's
    /// representable range.
    pub fn new(initial_millis: u64) -> Result<Self, EvidenceIntegrityError> {
        let initial_wall_time =
            wall_time_from_millis(initial_millis).ok_or(EvidenceIntegrityError::ClockOverflow)?;
        Ok(Self::from_valid_initial(initial_millis, initial_wall_time))
    }

    /// Start a manual clock anchored at the current wall time.
    ///
    /// The durable adapters composed into the same profile stamp rows with
    /// `Utc::now()` (`control_queue.rs` on both backends), so a clock left at
    /// the Unix epoch puts every engine-computed deadline decades behind every
    /// persisted `not_before`/`cutoff`: lease reclaim, visibility timeouts and
    /// retry gating then fire immediately or never, and the recorded failures
    /// are harness artifacts rather than product defects. Anchoring here keeps
    /// both sides in one era; advancement stays fully manual and deterministic.
    #[must_use]
    pub fn started_at_wall_clock() -> Self {
        let wall_time = Utc::now();
        u64::try_from(wall_time.timestamp_millis()).map_or_else(
            |_| Self::from_valid_initial(0, DateTime::<Utc>::UNIX_EPOCH),
            |millis| Self::from_valid_initial(millis, wall_time),
        )
    }

    fn from_valid_initial(initial_millis: u64, initial_wall_time: DateTime<Utc>) -> Self {
        let (changes, _) = watch::channel(initial_millis);
        Self {
            inner: Arc::new(ManualClockInner {
                state: Mutex::new(ManualClockState {
                    now_millis: initial_millis,
                    wall_time: initial_wall_time,
                    monotonic: Instant::now(),
                }),
                changes,
            }),
        }
    }

    /// Read the current deterministic instant in milliseconds.
    #[must_use]
    pub fn now_millis(&self) -> u64 {
        self.state().now_millis
    }

    /// Advance by an exact number of milliseconds.
    ///
    /// # Errors
    ///
    /// Returns an error if the new instant would overflow `u64`.
    pub fn advance_by_millis(&self, delta_millis: u64) -> Result<u64, EvidenceIntegrityError> {
        let mut state = self.state();
        let next_millis = state
            .now_millis
            .checked_add(delta_millis)
            .ok_or(EvidenceIntegrityError::ClockOverflow)?;
        let next_wall_time =
            wall_time_from_millis(next_millis).ok_or(EvidenceIntegrityError::ClockOverflow)?;
        let next_monotonic = state
            .monotonic
            .checked_add(Duration::from_millis(delta_millis))
            .ok_or(EvidenceIntegrityError::ClockOverflow)?;
        state.now_millis = next_millis;
        state.wall_time = next_wall_time;
        state.monotonic = next_monotonic;
        drop(state);
        self.inner.changes.send_replace(next_millis);
        Ok(next_millis)
    }

    /// Wait until the manual clock reaches `deadline_millis`.
    ///
    /// This uses a state-carrying watch channel, so an advance that happens
    /// before the waiter subscribes cannot be lost.
    ///
    /// # Errors
    ///
    /// Returns an error only if every clock owner is dropped while waiting.
    pub async fn wait_until_millis(
        &self,
        deadline_millis: u64,
    ) -> Result<(), EvidenceIntegrityError> {
        let mut changes = self.inner.changes.subscribe();
        loop {
            if *changes.borrow_and_update() >= deadline_millis {
                return Ok(());
            }
            changes
                .changed()
                .await
                .map_err(|_| EvidenceIntegrityError::ControlClosed)?;
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, ManualClockState> {
        match self.inner.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

impl Default for ManualClock {
    fn default() -> Self {
        Self::from_valid_initial(0, DateTime::<Utc>::UNIX_EPOCH)
    }
}

impl Clock for ManualClock {
    fn now(&self) -> DateTime<Utc> {
        self.state().wall_time
    }

    fn monotonic(&self) -> Instant {
        self.state().monotonic
    }
}

fn wall_time_from_millis(millis: u64) -> Option<DateTime<Utc>> {
    let signed_millis = i64::try_from(millis).ok()?;
    DateTime::<Utc>::from_timestamp_millis(signed_millis)
}

/// A named one-shot failpoint.
#[derive(Debug, Clone)]
pub struct OneShotFailpoint {
    name: EvidencePoint,
    state: Arc<AtomicU8>,
    fired: watch::Sender<bool>,
}

impl OneShotFailpoint {
    fn new(name: EvidencePoint) -> Self {
        let (fired, _) = watch::channel(false);
        Self {
            name,
            state: Arc::new(AtomicU8::new(0)),
            fired,
        }
    }

    /// Return this failpoint's bounded name.
    #[must_use]
    pub fn name(&self) -> &EvidencePoint {
        &self.name
    }

    /// Arm this failpoint exactly once.
    ///
    /// # Errors
    ///
    /// Returns an error after it has already been armed or fired.
    pub fn arm(&self) -> Result<(), EvidenceIntegrityError> {
        self.state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| ())
            .map_err(|_| EvidenceIntegrityError::FailpointAlreadyUsed)
    }

    /// Trip an armed failpoint, consuming its one allowed firing.
    #[must_use]
    pub fn trip(&self) -> bool {
        if self
            .state
            .compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.fired.send_replace(true);
            true
        } else {
            false
        }
    }

    /// Wait until the failpoint has fired without a lost-wakeup window.
    ///
    /// # Errors
    ///
    /// Returns an error only if every failpoint owner is dropped while waiting.
    pub async fn wait_fired(&self) -> Result<(), EvidenceIntegrityError> {
        let mut fired = self.fired.subscribe();
        loop {
            if *fired.borrow_and_update() {
                return Ok(());
            }
            fired
                .changed()
                .await
                .map_err(|_| EvidenceIntegrityError::ControlClosed)?;
        }
    }
}

/// A named, monotonic multi-phase barrier.
#[derive(Debug, Clone)]
pub struct PhaseGate {
    name: EvidencePoint,
    phase: Arc<AtomicU64>,
    changes: watch::Sender<u64>,
}

impl PhaseGate {
    fn new(name: EvidencePoint) -> Self {
        let (changes, _) = watch::channel(0);
        Self {
            name,
            phase: Arc::new(AtomicU64::new(0)),
            changes,
        }
    }

    /// Return this barrier's bounded name.
    #[must_use]
    pub fn name(&self) -> &EvidencePoint {
        &self.name
    }

    /// Return the current phase.
    #[must_use]
    pub fn current_phase(&self) -> u64 {
        self.phase.load(Ordering::Acquire)
    }

    /// Advance monotonically to `phase`.
    ///
    /// # Errors
    ///
    /// Returns an error when asked to move backwards.
    pub fn advance_to(&self, phase: u64) -> Result<(), EvidenceIntegrityError> {
        match self
            .phase
            .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                (phase > current).then_some(phase)
            }) {
            Ok(_) => {
                self.publish_current_phase();
                Ok(())
            },
            Err(current) if phase == current => {
                self.publish_current_phase();
                Ok(())
            },
            Err(current) => Err(EvidenceIntegrityError::PhaseRegression {
                current,
                requested: phase,
            }),
        }
    }

    fn publish_current_phase(&self) {
        let current = self.current_phase();
        self.changes.send_if_modified(|published| {
            if current > *published {
                *published = current;
                true
            } else {
                false
            }
        });
    }

    /// Wait until the gate reaches at least `phase`.
    ///
    /// # Errors
    ///
    /// Returns an error only if every gate owner is dropped while waiting.
    pub async fn wait_for(&self, phase: u64) -> Result<(), EvidenceIntegrityError> {
        let mut changes = self.changes.subscribe();
        loop {
            if *changes.borrow_and_update() >= phase {
                return Ok(());
            }
            changes
                .changed()
                .await
                .map_err(|_| EvidenceIntegrityError::ControlClosed)?;
        }
    }
}

/// Sanitized terminal outcome observed from the execution lifecycle bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionObservationOutcome {
    /// The engine reported a successful execution.
    Succeeded,
    /// The engine reported a failed execution.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct NodeObservationIdentity {
    execution_id: ExecutionId,
    node_key: NodeKey,
}

#[derive(Debug, Default)]
struct LifecycleObservationState {
    started: HashSet<NodeObservationIdentity>,
    parked: HashSet<NodeObservationIdentity>,
    wait_completed: HashSet<NodeObservationIdentity>,
    finished: HashMap<ExecutionId, ExecutionObservationOutcome>,
    distinct_fact_count: usize,
}

#[derive(Debug)]
struct LifecycleObservationInner {
    state: Mutex<LifecycleObservationState>,
    sequence: watch::Sender<u64>,
    maximum_distinct_facts: usize,
}

/// State-carrying, authority-free execution lifecycle observations.
///
/// Only sanitized lifecycle facts and typed identities are retained. Raw
/// events, action inputs, outputs, failure messages, stores, and the event bus
/// are never exposed through this registry.
#[derive(Debug, Clone)]
pub struct LifecycleObservationRegistry {
    inner: Arc<LifecycleObservationInner>,
}

/// Bounded count-only lifecycle snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifecycleObservationSnapshot {
    /// Distinct observed node-start facts.
    pub node_started_count: usize,
    /// Distinct observed durable node-park facts.
    pub node_parked_count: usize,
    /// Distinct observed node-wait-completed facts.
    pub node_wait_completed_count: usize,
    /// Distinct observed terminal executions.
    pub execution_finished_count: usize,
}

impl LifecycleObservationRegistry {
    pub(super) fn new() -> Self {
        Self::with_capacity(MAX_LIFECYCLE_OBSERVATIONS)
    }

    fn with_capacity(maximum_distinct_facts: usize) -> Self {
        let (sequence, _) = watch::channel(0);
        Self {
            inner: Arc::new(LifecycleObservationInner {
                state: Mutex::new(LifecycleObservationState::default()),
                sequence,
                maximum_distinct_facts,
            }),
        }
    }

    #[cfg(test)]
    pub(super) fn with_capacity_for_integrity(maximum_distinct_facts: usize) -> Self {
        Self::with_capacity(maximum_distinct_facts)
    }

    pub(super) fn record_event(&self, event: ExecutionEvent) -> Result<(), EvidenceIntegrityError> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| EvidenceIntegrityError::RecorderPoisoned)?;
        let mut added_distinct_fact = false;
        let changed = match event {
            ExecutionEvent::NodeStarted {
                execution_id,
                node_key,
                ..
            } => {
                let identity = NodeObservationIdentity {
                    execution_id,
                    node_key,
                };
                if state.started.contains(&identity) {
                    false
                } else {
                    ensure_observation_capacity(&state, self.inner.maximum_distinct_facts)?;
                    added_distinct_fact = state.started.insert(identity);
                    added_distinct_fact
                }
            },
            ExecutionEvent::NodeParked {
                execution_id,
                node_key,
                ..
            } => {
                let identity = NodeObservationIdentity {
                    execution_id,
                    node_key,
                };
                if state.parked.contains(&identity) {
                    false
                } else {
                    ensure_observation_capacity(&state, self.inner.maximum_distinct_facts)?;
                    added_distinct_fact = state.parked.insert(identity);
                    added_distinct_fact
                }
            },
            ExecutionEvent::NodeWaitCompleted {
                execution_id,
                node_key,
            } => {
                let identity = NodeObservationIdentity {
                    execution_id,
                    node_key,
                };
                if state.wait_completed.contains(&identity) {
                    false
                } else {
                    ensure_observation_capacity(&state, self.inner.maximum_distinct_facts)?;
                    added_distinct_fact = state.wait_completed.insert(identity);
                    added_distinct_fact
                }
            },
            ExecutionEvent::ExecutionFinished {
                execution_id,
                success,
                ..
            } => {
                let outcome = if success {
                    ExecutionObservationOutcome::Succeeded
                } else {
                    ExecutionObservationOutcome::Failed
                };
                match state.finished.get(&execution_id) {
                    Some(previous) if *previous == outcome => false,
                    Some(_) => {
                        return Err(EvidenceIntegrityError::ConflictingTerminalObservation);
                    },
                    None => {
                        ensure_observation_capacity(&state, self.inner.maximum_distinct_facts)?;
                        state.finished.insert(execution_id, outcome);
                        added_distinct_fact = true;
                        true
                    },
                }
            },
            _ => false,
        };
        if added_distinct_fact {
            state.distinct_fact_count = state
                .distinct_fact_count
                .checked_add(1)
                .ok_or(EvidenceIntegrityError::CounterOverflow)?;
        }
        drop(state);
        if changed {
            let next_sequence = self
                .inner
                .sequence
                .borrow()
                .checked_add(1)
                .ok_or(EvidenceIntegrityError::CounterOverflow)?;
            self.inner.sequence.send_replace(next_sequence);
        }
        Ok(())
    }

    /// Await an observed node start without polling or a lost-wakeup window.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid timeout, timeout expiry, closed
    /// observation control, or unavailable registry state.
    pub async fn await_node_started(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        maximum_wait: Duration,
    ) -> Result<(), EvidenceIntegrityError> {
        let identity = NodeObservationIdentity {
            execution_id,
            node_key,
        };
        self.await_state(maximum_wait, |state| {
            state.started.contains(&identity).then_some(())
        })
        .await
    }

    /// Await a durable node-park observation without polling or sleeping.
    ///
    /// `NodeParked` is emitted by the engine only after the waiting checkpoint
    /// has been committed.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid timeout, timeout expiry, closed
    /// observation control, or unavailable registry state.
    pub async fn await_node_parked(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        maximum_wait: Duration,
    ) -> Result<(), EvidenceIntegrityError> {
        let identity = NodeObservationIdentity {
            execution_id,
            node_key,
        };
        self.await_state(maximum_wait, |state| {
            state.parked.contains(&identity).then_some(())
        })
        .await
    }

    /// Await a node-wait-completed observation without polling or sleeping.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid timeout, timeout expiry, closed
    /// observation control, or unavailable registry state.
    pub async fn await_node_wait_completed(
        &self,
        execution_id: ExecutionId,
        node_key: NodeKey,
        maximum_wait: Duration,
    ) -> Result<(), EvidenceIntegrityError> {
        let identity = NodeObservationIdentity {
            execution_id,
            node_key,
        };
        self.await_state(maximum_wait, |state| {
            state.wait_completed.contains(&identity).then_some(())
        })
        .await
    }

    /// Await an execution's terminal success/failure observation.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid timeout, timeout expiry, closed
    /// observation control, or unavailable registry state.
    pub async fn await_execution_finished(
        &self,
        execution_id: ExecutionId,
        maximum_wait: Duration,
    ) -> Result<ExecutionObservationOutcome, EvidenceIntegrityError> {
        self.await_state(maximum_wait, |state| {
            state.finished.get(&execution_id).copied()
        })
        .await
    }

    /// Return bounded counts without exposing the underlying event stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the registry state is unavailable.
    pub fn snapshot(&self) -> Result<LifecycleObservationSnapshot, EvidenceIntegrityError> {
        let state = self
            .inner
            .state
            .lock()
            .map_err(|_| EvidenceIntegrityError::RecorderPoisoned)?;
        Ok(LifecycleObservationSnapshot {
            node_started_count: state.started.len(),
            node_parked_count: state.parked.len(),
            node_wait_completed_count: state.wait_completed.len(),
            execution_finished_count: state.finished.len(),
        })
    }

    async fn await_state<T>(
        &self,
        maximum_wait: Duration,
        inspect: impl Fn(&LifecycleObservationState) -> Option<T>,
    ) -> Result<T, EvidenceIntegrityError> {
        if maximum_wait.is_zero() || maximum_wait > MAX_OBSERVATION_WAIT {
            return Err(EvidenceIntegrityError::InvalidObservationTimeout);
        }
        let mut sequence = self.inner.sequence.subscribe();
        let wait = async {
            loop {
                let observed = {
                    let state = self
                        .inner
                        .state
                        .lock()
                        .map_err(|_| EvidenceIntegrityError::RecorderPoisoned)?;
                    inspect(&state)
                };
                if let Some(observed) = observed {
                    return Ok(observed);
                }
                sequence
                    .changed()
                    .await
                    .map_err(|_| EvidenceIntegrityError::ControlClosed)?;
            }
        };
        tokio::time::timeout(maximum_wait, wait)
            .await
            .map_err(|_| EvidenceIntegrityError::ObservationTimedOut)?
    }
}

fn ensure_observation_capacity(
    state: &LifecycleObservationState,
    maximum_distinct_facts: usize,
) -> Result<(), EvidenceIntegrityError> {
    if state.distinct_fact_count >= maximum_distinct_facts {
        Err(EvidenceIntegrityError::ObservationCapacityExceeded)
    } else {
        Ok(())
    }
}

/// Independent provider-effect probe that never reads Nebula durable state.
#[derive(Debug, Clone, Default)]
pub struct EffectProbe {
    state: Arc<Mutex<EffectProbeState>>,
}

#[derive(Debug, Default)]
struct EffectProbeState {
    call_count: u64,
    committed_operations: BTreeSet<EvidencePoint>,
}

/// Evidence capability for one recorded provider call.
#[derive(Debug, Clone)]
pub struct EffectCall {
    operation: EvidencePoint,
}

/// Bounded aggregate view of independent provider observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectProbeSnapshot {
    /// Total provider invocations, including repeat calls.
    pub call_count: u64,
    /// Distinct effects the independent provider accepted.
    pub committed_effect_count: u64,
}

impl EffectProbe {
    /// Record one provider invocation independently of Nebula state.
    ///
    /// # Errors
    ///
    /// Returns an error if the local evidence lock was poisoned.
    pub fn record_call(
        &self,
        operation: EvidencePoint,
    ) -> Result<EffectCall, EvidenceIntegrityError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| EvidenceIntegrityError::RecorderPoisoned)?;
        state.call_count = state
            .call_count
            .checked_add(1)
            .ok_or(EvidenceIntegrityError::CounterOverflow)?;
        Ok(EffectCall { operation })
    }

    /// Record provider acceptance for a previously observed call.
    ///
    /// Returns `true` only for the first committed effect with that operation
    /// key; repeat transport calls cannot inflate committed-effect evidence.
    ///
    /// # Errors
    ///
    /// Returns an error if the local evidence lock was poisoned.
    pub fn commit_effect(&self, call: &EffectCall) -> Result<bool, EvidenceIntegrityError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| EvidenceIntegrityError::RecorderPoisoned)?;
        Ok(state.committed_operations.insert(call.operation.clone()))
    }

    /// Snapshot the bounded independent counts.
    ///
    /// # Errors
    ///
    /// Returns an error if the local evidence lock was poisoned.
    pub fn snapshot(&self) -> Result<EffectProbeSnapshot, EvidenceIntegrityError> {
        let state = self
            .state
            .lock()
            .map_err(|_| EvidenceIntegrityError::RecorderPoisoned)?;
        let committed_effect_count = u64::try_from(state.committed_operations.len())
            .map_err(|_| EvidenceIntegrityError::CounterOverflow)?;
        Ok(EffectProbeSnapshot {
            call_count: state.call_count,
            committed_effect_count,
        })
    }
}

/// Required slot in a runtime-repair evidence artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArtifactSlot {
    /// Exact software, hardware, runtime, and database environment.
    Environment,
    /// Versioned scenario input and configuration revisions.
    Input,
    /// Eligible, included, excluded, and skipped denominator.
    Denominator,
    /// Sanitized raw observations.
    RawObservation,
    /// Query plan or an explicit non-applicability observation.
    QueryPlan,
}

/// Content classification applied before an artifact entry can exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactContentClassification {
    /// Bounded structured metadata with no secret or business payload.
    SanitizedReference,
    /// Secret-bearing content, which the profile always rejects.
    ContainsSecret,
    /// Raw business payload content, which the profile always rejects.
    RawBusinessPayload,
}

/// Closed, payload-free identity for an artifact entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArtifactReference {
    /// Reproducible runtime and database environment metadata.
    RuntimeEnvironment,
    /// Versioned scenario input revision metadata.
    ScenarioInputRevision,
    /// Included/excluded/skipped denominator counts.
    ScenarioDenominator,
    /// Count-only sanitized execution lifecycle observations.
    SanitizedLifecycleObservations,
    /// Database query-plan metadata.
    DatabaseQueryPlan,
    /// Explicit statement that a query plan does not apply.
    QueryPlanNotApplicable,
}

impl ArtifactReference {
    const fn slot(self) -> ArtifactSlot {
        match self {
            Self::RuntimeEnvironment => ArtifactSlot::Environment,
            Self::ScenarioInputRevision => ArtifactSlot::Input,
            Self::ScenarioDenominator => ArtifactSlot::Denominator,
            Self::SanitizedLifecycleObservations => ArtifactSlot::RawObservation,
            Self::DatabaseQueryPlan | Self::QueryPlanNotApplicable => ArtifactSlot::QueryPlan,
        }
    }
}

/// Fixed-width digest of already-sanitized evidence metadata.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ArtifactDigest([u8; 32]);

impl ArtifactDigest {
    /// Construct a digest from an already-computed fixed-width value.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn is_zero(&self) -> bool {
        self.0.iter().all(|byte| *byte == 0)
    }
}

impl std::fmt::Debug for ArtifactDigest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ArtifactDigest([redacted])")
    }
}

/// One bounded, structured, sanitized artifact entry.
pub struct ArtifactEntry {
    reference: ArtifactReference,
    observation_count: u64,
    digest: Option<ArtifactDigest>,
}

impl std::fmt::Debug for ArtifactEntry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ArtifactEntry")
            .field("slot", &self.reference.slot())
            .field("observation_count", &self.observation_count)
            .field("digest_present", &self.digest.is_some())
            .finish()
    }
}

impl ArtifactEntry {
    /// Construct a bounded structured entry after classifying its content.
    ///
    /// No arbitrary text or payload bytes can be represented by this type.
    ///
    /// # Errors
    ///
    /// Rejects secret-bearing and raw-business-payload classifications before
    /// an [`ArtifactEntry`] is constructed.
    pub fn new(
        classification: ArtifactContentClassification,
        reference: ArtifactReference,
        observation_count: u64,
        digest: Option<ArtifactDigest>,
    ) -> Result<Self, EvidenceIntegrityError> {
        match classification {
            ArtifactContentClassification::SanitizedReference => {},
            ArtifactContentClassification::ContainsSecret => {
                return Err(EvidenceIntegrityError::SecretArtifactRejected);
            },
            ArtifactContentClassification::RawBusinessPayload => {
                return Err(EvidenceIntegrityError::RawPayloadArtifactRejected);
            },
        }
        if digest.as_ref().is_some_and(ArtifactDigest::is_zero) {
            return Err(EvidenceIntegrityError::InvalidArtifactDigest);
        }
        Ok(Self {
            reference,
            observation_count,
            digest,
        })
    }
}

/// Bounded recorder for reproducible runtime-repair evidence.
#[derive(Debug, Clone)]
pub struct ArtifactRecorder {
    maximum_entries: usize,
    entries: Arc<Mutex<Vec<ArtifactEntry>>>,
}

/// Bounded recorder counts by required artifact slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRecorderSnapshot {
    /// Total retained entries.
    pub entry_count: usize,
    /// Counts for each classified evidence slot.
    pub counts_by_slot: BTreeMap<ArtifactSlot, usize>,
}

impl ArtifactRecorder {
    /// Create a recorder with an explicit non-zero bound.
    ///
    /// # Errors
    ///
    /// Returns an error if the bound is zero or above the profile ceiling.
    pub fn new(maximum_entries: usize) -> Result<Self, EvidenceIntegrityError> {
        if maximum_entries == 0 || maximum_entries > MAX_ARTIFACT_ENTRIES {
            return Err(EvidenceIntegrityError::InvalidArtifactCapacity);
        }
        Ok(Self {
            maximum_entries,
            entries: Arc::new(Mutex::new(Vec::with_capacity(maximum_entries))),
        })
    }

    /// Retain one structurally sanitized, bounded entry.
    ///
    /// # Errors
    ///
    /// Rejects capacity exhaustion and poisoned recorder state. Unsafe content
    /// classifications have already been rejected by [`ArtifactEntry::new`].
    pub fn record(&self, entry: ArtifactEntry) -> Result<(), EvidenceIntegrityError> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|_| EvidenceIntegrityError::RecorderPoisoned)?;
        if entries.len() == self.maximum_entries {
            return Err(EvidenceIntegrityError::ArtifactCapacityExceeded);
        }
        entries.push(entry);
        Ok(())
    }

    /// Snapshot counts without exposing recorded observation text.
    ///
    /// # Errors
    ///
    /// Returns an error if the recorder lock was poisoned.
    pub fn snapshot(&self) -> Result<ArtifactRecorderSnapshot, EvidenceIntegrityError> {
        let entries = self
            .entries
            .lock()
            .map_err(|_| EvidenceIntegrityError::RecorderPoisoned)?;
        let mut counts_by_slot = BTreeMap::new();
        for entry in entries.iter() {
            *counts_by_slot.entry(entry.reference.slot()).or_insert(0) += 1;
        }
        Ok(ArtifactRecorderSnapshot {
            entry_count: entries.len(),
            counts_by_slot,
        })
    }
}

/// File-SQLite lifecycle control whose database path remains opaque.
#[derive(Clone)]
pub struct FileSqliteReopenController {
    lease: Arc<SqliteFileLease>,
    completed_reopens: Arc<AtomicU64>,
}

struct SqliteFileLease {
    path: PathBuf,
}

impl std::fmt::Debug for FileSqliteReopenController {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FileSqliteReopenController")
            .field("database_path", &"[opaque]")
            .field("completed_reopens", &self.completed_reopens())
            .finish()
    }
}

impl FileSqliteReopenController {
    pub(super) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nebula-runtime-repair-{}.sqlite3",
            uuid::Uuid::new_v4()
        ));
        Self {
            lease: Arc::new(SqliteFileLease { path }),
            completed_reopens: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(super) fn database_path(&self) -> &Path {
        &self.lease.path
    }

    /// Return the number of completed close/reopen integrity cycles.
    #[must_use]
    pub fn completed_reopens(&self) -> u64 {
        self.completed_reopens.load(Ordering::Acquire)
    }

    /// Prove that a marker survives a real pool close and file reopen.
    ///
    /// # Errors
    ///
    /// Returns a closed, path-free error when open, write, close, reopen, or
    /// readback fails.
    pub async fn verify_close_reopen(&self) -> Result<(), EvidenceIntegrityError> {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

        let options = SqliteConnectOptions::new()
            .filename(self.database_path())
            .create_if_missing(true);
        let first_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options.clone())
            .await
            .map_err(|_| EvidenceIntegrityError::SqliteReopenFailed)?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS runtime_repair_reopen_probe \
             (marker INTEGER NOT NULL)",
        )
        .execute(&first_pool)
        .await
        .map_err(|_| EvidenceIntegrityError::SqliteReopenFailed)?;
        sqlx::query("DELETE FROM runtime_repair_reopen_probe")
            .execute(&first_pool)
            .await
            .map_err(|_| EvidenceIntegrityError::SqliteReopenFailed)?;
        sqlx::query("INSERT INTO runtime_repair_reopen_probe (marker) VALUES (1)")
            .execute(&first_pool)
            .await
            .map_err(|_| EvidenceIntegrityError::SqliteReopenFailed)?;
        first_pool.close().await;

        let reopened_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|_| EvidenceIntegrityError::SqliteReopenFailed)?;
        let marker_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM runtime_repair_reopen_probe WHERE marker = 1")
                .fetch_one(&reopened_pool)
                .await
                .map_err(|_| EvidenceIntegrityError::SqliteReopenFailed)?;
        reopened_pool.close().await;
        if marker_count != 1 {
            return Err(EvidenceIntegrityError::SqliteReopenFailed);
        }
        self.completed_reopens.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

impl Drop for SqliteFileLease {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(sidecar_path(&self.path, "-wal"));
        let _ = std::fs::remove_file(sidecar_path(&self.path, "-shm"));
    }
}

fn sidecar_path(path: &Path, suffix: &str) -> PathBuf {
    let mut sidecar: OsString = path.as_os_str().to_owned();
    sidecar.push(suffix);
    PathBuf::from(sidecar)
}

/// Required live-PostgreSQL connectivity and version probe.
#[derive(Clone)]
pub struct LivePostgresIntegrityProbe {
    database_dsn: Arc<SecretString>,
    succeeded: Arc<AtomicBool>,
}

impl std::fmt::Debug for LivePostgresIntegrityProbe {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LivePostgresIntegrityProbe")
            .field("database_dsn", &"[redacted]")
            .field("succeeded", &self.has_succeeded())
            .finish()
    }
}

/// Sanitized PostgreSQL environment evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostgresIntegritySnapshot {
    /// Server version reported by `SHOW server_version`.
    pub server_version: String,
}

impl LivePostgresIntegrityProbe {
    pub(super) fn new(database_dsn: Arc<SecretString>) -> Self {
        Self {
            database_dsn,
            succeeded: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(super) fn database_dsn(&self) -> &str {
        self.database_dsn.expose_secret()
    }

    /// Whether this required live probe has succeeded.
    #[must_use]
    pub fn has_succeeded(&self) -> bool {
        self.succeeded.load(Ordering::Acquire)
    }

    /// Require a live server and capture its sanitized version.
    ///
    /// PostgreSQL absence is an error; there is no skip or substitution path.
    ///
    /// # Errors
    ///
    /// Returns an error when the feature is absent or the live connection,
    /// query, or result validation fails.
    pub async fn verify_required(
        &self,
    ) -> Result<PostgresIntegritySnapshot, EvidenceIntegrityError> {
        #[cfg(feature = "postgres")]
        {
            use sqlx::postgres::PgPoolOptions;

            let pool = PgPoolOptions::new()
                .max_connections(1)
                .connect(self.database_dsn())
                .await
                .map_err(|_| EvidenceIntegrityError::PostgresRequiredUnavailable)?;
            let server_version: String = sqlx::query_scalar("SHOW server_version")
                .fetch_one(&pool)
                .await
                .map_err(|_| EvidenceIntegrityError::PostgresRequiredUnavailable)?;
            pool.close().await;
            if server_version.trim().is_empty() {
                return Err(EvidenceIntegrityError::PostgresRequiredUnavailable);
            }
            self.succeeded.store(true, Ordering::Release);
            Ok(PostgresIntegritySnapshot { server_version })
        }
        #[cfg(not(feature = "postgres"))]
        {
            Err(EvidenceIntegrityError::PostgresFeatureNotEnabled)
        }
    }
}

/// Authority-free evidence controls retained by a profile harness.
#[derive(Debug, Clone)]
pub struct EvidenceControls {
    clock: Arc<ManualClock>,
    observations: LifecycleObservationRegistry,
    effect_probe: EffectProbe,
    artifact_recorder: ArtifactRecorder,
    sqlite_reopen: Option<FileSqliteReopenController>,
    postgres_probe: Option<LivePostgresIntegrityProbe>,
}

impl EvidenceControls {
    pub(super) fn in_memory() -> Self {
        Self::new(None, None)
    }

    pub(super) fn file_sqlite(controller: FileSqliteReopenController) -> Self {
        Self::new(Some(controller), None)
    }

    pub(super) fn live_postgres(probe: LivePostgresIntegrityProbe) -> Self {
        Self::new(None, Some(probe))
    }

    fn new(
        sqlite_reopen: Option<FileSqliteReopenController>,
        postgres_probe: Option<LivePostgresIntegrityProbe>,
    ) -> Self {
        Self {
            clock: Arc::new(ManualClock::started_at_wall_clock()),
            observations: LifecycleObservationRegistry::new(),
            effect_probe: EffectProbe::default(),
            artifact_recorder: ArtifactRecorder {
                maximum_entries: 1_024,
                entries: Arc::new(Mutex::new(Vec::with_capacity(1_024))),
            },
            sqlite_reopen,
            postgres_probe,
        }
    }

    /// Clone the exact manual clock injected into the workflow engine.
    #[must_use]
    pub fn clock(&self) -> Arc<ManualClock> {
        Arc::clone(&self.clock)
    }

    /// Clone the authority-free execution lifecycle observation registry.
    #[must_use]
    pub fn observations(&self) -> LifecycleObservationRegistry {
        self.observations.clone()
    }

    /// Create a named one-shot failpoint.
    #[must_use]
    pub fn one_shot_failpoint(&self, name: EvidencePoint) -> OneShotFailpoint {
        OneShotFailpoint::new(name)
    }

    /// Create a named, monotonic phase barrier.
    #[must_use]
    pub fn phase_gate(&self, name: EvidencePoint) -> PhaseGate {
        PhaseGate::new(name)
    }

    /// Clone the independent effect probe.
    #[must_use]
    pub fn effect_probe(&self) -> EffectProbe {
        self.effect_probe.clone()
    }

    /// Clone the bounded artifact recorder.
    #[must_use]
    pub fn artifact_recorder(&self) -> ArtifactRecorder {
        self.artifact_recorder.clone()
    }

    /// Access the opaque file-SQLite close/reopen controller when selected.
    #[must_use]
    pub fn sqlite_reopen_controller(&self) -> Option<&FileSqliteReopenController> {
        self.sqlite_reopen.as_ref()
    }

    /// Access the required live-PostgreSQL integrity probe when selected.
    #[must_use]
    pub fn live_postgres_probe(&self) -> Option<&LivePostgresIntegrityProbe> {
        self.postgres_probe.as_ref()
    }

    pub(super) fn observation_registry(&self) -> LifecycleObservationRegistry {
        self.observations.clone()
    }
}

/// Integrity failures from deterministic evidence controls.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EvidenceIntegrityError {
    /// Evidence-point names must be bounded lowercase kebab-case.
    #[error("evidence point name is invalid")]
    InvalidEvidencePoint,
    /// Advancing the manual clock overflowed its finite representation.
    #[error("manual clock overflow")]
    ClockOverflow,
    /// An evidence counter overflowed.
    #[error("evidence counter overflow")]
    CounterOverflow,
    /// A one-shot failpoint cannot be armed or fired twice.
    #[error("one-shot failpoint has already been used")]
    FailpointAlreadyUsed,
    /// A monotonic phase gate cannot move backwards.
    #[error("phase gate cannot regress from {current} to {requested}")]
    PhaseRegression {
        /// Current phase.
        current: u64,
        /// Rejected earlier phase.
        requested: u64,
    },
    /// An evidence control closed before its awaited state arrived.
    #[error("evidence control closed before reaching the requested state")]
    ControlClosed,
    /// Observation waits must use a non-zero timeout no greater than the
    /// profile's fixed ceiling.
    #[error("lifecycle observation timeout is outside the allowed bound")]
    InvalidObservationTimeout,
    /// The requested lifecycle fact was not observed before the bounded wait
    /// expired.
    #[error("lifecycle observation did not arrive before the timeout")]
    ObservationTimedOut,
    /// The supervised lifecycle observer fell behind its bounded event stream.
    #[error("lifecycle observation stream lagged")]
    ObservationLagged,
    /// The bounded lifecycle registry cannot retain another distinct fact.
    #[error("lifecycle observation capacity exceeded")]
    ObservationCapacityExceeded,
    /// One execution was observed with two different terminal outcomes.
    #[error("conflicting terminal lifecycle observations")]
    ConflictingTerminalObservation,
    /// Artifact capacity must be within the closed profile bound.
    #[error("artifact recorder capacity is invalid")]
    InvalidArtifactCapacity,
    /// The bounded artifact recorder is full.
    #[error("artifact recorder capacity exceeded")]
    ArtifactCapacityExceeded,
    /// Secret-bearing evidence is forbidden.
    #[error("secret-bearing artifact rejected")]
    SecretArtifactRejected,
    /// Raw business payload evidence is forbidden.
    #[error("raw-payload artifact rejected")]
    RawPayloadArtifactRejected,
    /// Artifact digests must be non-zero fixed-width values.
    #[error("artifact digest is invalid")]
    InvalidArtifactDigest,
    /// A local evidence lock was poisoned.
    #[error("evidence recorder state is unavailable")]
    RecorderPoisoned,
    /// File SQLite did not survive a real close/reopen cycle.
    #[error("file-SQLite close/reopen integrity check failed")]
    SqliteReopenFailed,
    /// Live PostgreSQL is required but unavailable or failed its integrity query.
    #[error("required live PostgreSQL integrity check failed")]
    PostgresRequiredUnavailable,
    /// Live PostgreSQL was selected without compiling PostgreSQL support.
    #[error("required live PostgreSQL needs the `postgres` feature")]
    PostgresFeatureNotEnabled,
}
