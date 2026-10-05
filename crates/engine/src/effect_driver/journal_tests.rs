use std::collections::VecDeque;

use nebula_core::ResourceKey;

use nebula_resource::{
    AcquireOptions, Manager, RegistrationSpec, Resident, ResidentConfig, ResourceContext,
    call::{Cost, Operation, OperationCx, OperationError, ResourceHandle, SentState},
    resource::{Provider, ResourceMetadataDraft},
    topology::resident::ResidentProvider,
};
use nebula_storage_port::{
    dto::{EffectOccurrenceKey, PrepareOutcome},
    store::ExecutionStore,
};
use tokio_util::sync::CancellationToken;

use super::*;

/// Iteration checkpoints: attestation, the barrier gate, resume integrity.
#[path = "journal_checkpoint_tests.rs"]
mod checkpoint;

// ── the journal shape of each action kind ────────────────────────────────

#[test]
fn stateless_stateful_and_agent_actions_are_journaled_and_the_rest_say_why_not() {
    use nebula_action::ActionKind;
    assert_eq!(JournalShape::of(ActionKind::Stateless), JournalShape::Flat);
    assert!(JournalShape::Flat.is_journaled());
    assert!(!JournalShape::Flat.is_gated());
    assert_eq!(JournalShape::read_only_detail(ActionKind::Stateless), None);
    assert_eq!(
        JournalShape::read_only_detail(ActionKind::Control),
        Some(
            "control actions decide flow and must not cause effects; move effects to a \
             stateless action"
        )
    );
    assert_eq!(
        JournalShape::of(ActionKind::Stateful),
        JournalShape::Iterated
    );
    assert!(JournalShape::Iterated.is_journaled());
    assert!(JournalShape::Iterated.is_gated());
    assert_eq!(JournalShape::read_only_detail(ActionKind::Stateful), None);
    // An agent's turns are journaled, each a positional run.
    assert_eq!(JournalShape::of(ActionKind::Agent), JournalShape::Turned);
    assert!(JournalShape::Turned.is_journaled());
    assert!(JournalShape::Turned.is_gated());
    assert_eq!(JournalShape::read_only_detail(ActionKind::Agent), None);
    for kind in [
        ActionKind::Control,
        ActionKind::Stream,
        ActionKind::Interactive,
        ActionKind::Trigger,
        ActionKind::Resource,
    ] {
        assert_eq!(JournalShape::of(kind), JournalShape::None, "{kind:?}");
        assert!(!JournalShape::of(kind).is_journaled(), "{kind:?}");
        assert!(JournalShape::read_only_detail(kind).is_some(), "{kind:?}");
    }
}

// ── the provider idempotency key ─────────────────────────────────────────

fn frame_of(bytes: &[u8]) -> Vec<u8> {
    let mut framed = (bytes.len() as u64).to_be_bytes().to_vec();
    framed.extend_from_slice(bytes);
    framed
}

/// The key, derived independently of the implementation from the documented
/// framing.
fn expected_key(version: u32, developer: &[u8]) -> String {
    let tenant = [frame_of(b"org-a"), frame_of(b"workspace-a")].concat();
    let mut digest = Sha256::new();
    digest.update(frame_of(b"nebula.idempotency-key.v1"));
    digest.update(frame_of(&tenant));
    digest.update(frame_of(b"billing.gateway"));
    digest.update(frame_of(b"billing.charge"));
    digest.update(version.to_be_bytes());
    digest.update(frame_of(developer));
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest.finalize())
}

fn key_parts<'a>(
    developer: Option<&'a str>,
    execution_id: &'a str,
    occurrence: &'a str,
) -> ProviderKeyParts<'a> {
    ProviderKeyParts {
        org_id: "org-a",
        workspace_id: "workspace-a",
        resource_key: "billing.gateway",
        operation: "billing.charge",
        version: 1,
        developer,
        execution_id,
        node_key: "charge",
        occurrence,
    }
}

fn derived_key(parts: &ProviderKeyParts<'_>) -> String {
    provider_idempotency_key(parts)
        .expect("a valid key")
        .as_str()
        .to_owned()
}

const OCCURRENCE_0: &str = "unit/v1/#000000";
const OCCURRENCE_1: &str = "unit/v1/#000001";

#[test]
fn provider_key_golden_vectors() {
    // Pinned values: a change here re-keys every journaled effect in
    // flight, so the provider would apply a retried effect twice.
    let developer = derived_key(&key_parts(Some("order-123"), "exec-1", OCCURRENCE_0));
    assert_eq!(developer, "rT-cFUezYGxWViT9yxc08nun9dXX-uC5FWXmLEN4PWI");
    assert_eq!(developer, expected_key(1, b"order-123"));

    // Re-pinned (unreleased) when occurrences became one node-wide
    // positional sequence (`unit/v1/#n`): the run part frames the
    // occurrence.
    let run = derived_key(&key_parts(None, "exec-1", OCCURRENCE_0));
    assert_eq!(run, "EMMjio7V0v7X92ICkDncX5KjpQg9MmQb_bCcDSt3kFg");
    let run_part = [
        frame_of(b"exec-1"),
        frame_of(b"charge"),
        frame_of(OCCURRENCE_0.as_bytes()),
    ]
    .concat();
    assert_eq!(run, expected_key(1, &run_part));
    assert_eq!(run.len(), 43);
}

#[test]
fn a_developer_part_carries_no_execution_node_or_occurrence() {
    let first = derived_key(&key_parts(Some("order-123"), "exec-1", OCCURRENCE_0));
    let mut elsewhere = key_parts(Some("order-123"), "exec-2", OCCURRENCE_1);
    elsewhere.node_key = "other-node";
    assert_eq!(
        first,
        derived_key(&elsewhere),
        "a developer key deduplicates across executions"
    );
    assert_ne!(
        first,
        derived_key(&key_parts(Some("order-124"), "exec-1", OCCURRENCE_0))
    );
}

#[test]
fn a_run_part_is_scoped_to_the_execution_node_and_occurrence() {
    let first = derived_key(&key_parts(None, "exec-1", OCCURRENCE_0));
    assert_ne!(first, derived_key(&key_parts(None, "exec-2", OCCURRENCE_0)));
    assert_ne!(first, derived_key(&key_parts(None, "exec-1", OCCURRENCE_1)));
    let mut other_node = key_parts(None, "exec-1", OCCURRENCE_0);
    other_node.node_key = "refund";
    assert_ne!(first, derived_key(&other_node));
}

#[test]
fn the_key_is_bound_to_tenant_resource_operation_and_version() {
    let base = derived_key(&key_parts(Some("order-1"), "e", "o"));
    let mut other_tenant = key_parts(Some("order-1"), "e", "o");
    other_tenant.workspace_id = "workspace-b";
    let mut other_resource = key_parts(Some("order-1"), "e", "o");
    other_resource.resource_key = "billing.other";
    let mut other_operation = key_parts(Some("order-1"), "e", "o");
    other_operation.operation = "billing.refund";
    let mut other_version = key_parts(Some("order-1"), "e", "o");
    other_version.version = 2;
    for other in [other_tenant, other_resource, other_operation, other_version] {
        assert_ne!(derived_key(&other), base);
    }
    let mut v2 = key_parts(Some("order-1"), "e", "o");
    v2.version = 2;
    assert_eq!(derived_key(&v2), expected_key(2, b"order-1"));
}

// ── policy, identity and evidence ────────────────────────────────────────

fn policy(
    effect: Effect,
    recovery: Recovery,
    max_invocations: u32,
) -> Result<PreparedEffectPolicy, EffectExecutionError> {
    slot_policy(
        effect,
        recovery,
        NonZeroU32::new(max_invocations).expect("non-zero"),
    )
}

fn cap_ms() -> u64 {
    u64::try_from(OPERATION_DEADLINE_CAP.as_millis()).expect("fits")
}

#[test]
fn slot_policies_project_the_declared_recovery() {
    let write = policy(Effect::Write, Recovery::Opaque, 3).expect("write");
    assert_eq!(write.capability(), DestinationCapability::Opaque);
    assert_eq!(write.max_invocations(), 3);
    assert_eq!(write.max_queries(), 0);
    assert_eq!(write.recovery_window_ms(), cap_ms());
    assert_eq!(write.stable_window_ms(), None);

    let stable = policy(
        Effect::Idempotent,
        Recovery::StableKey {
            window: Duration::from_hours(24),
        },
        20_000,
    )
    .expect("idempotent");
    assert_eq!(stable.capability(), DestinationCapability::StableKey);
    assert_eq!(stable.max_invocations(), 10_000, "clamped to the ledger");
    assert_eq!(stable.stable_window_ms(), Some(86_400_000));
    assert_eq!(stable.recovery_window_ms(), 86_400_000);

    let short = policy(
        Effect::Idempotent,
        Recovery::StableKey {
            window: Duration::from_secs(30),
        },
        1,
    )
    .expect("short window");
    assert_eq!(short.stable_window_ms(), Some(30_000));
    assert_eq!(
        short.recovery_window_ms(),
        cap_ms(),
        "recovery covers at least the unit deadline"
    );

    let long = policy(
        Effect::Idempotent,
        Recovery::StableKey {
            window: Duration::from_hours(24 * 800),
        },
        1,
    )
    .expect("long window");
    assert_eq!(long.stable_window_ms(), Some(31_536_000_000));
    assert_eq!(long.recovery_window_ms(), 31_536_000_000);

    for (effect, recovery) in [
        (
            Effect::Write,
            Recovery::StableKey {
                window: Duration::from_secs(1),
            },
        ),
        (Effect::Idempotent, Recovery::Opaque),
        (Effect::Read, Recovery::Opaque),
        (Effect::RecordedRead, Recovery::Opaque),
        (Effect::Write, Recovery::Observation),
    ] {
        assert_eq!(
            policy(effect, recovery, 1).err(),
            Some(EffectExecutionError::InvalidContract)
        );
    }

    // A recorded read: a stable key over the ledger's longest window, with
    // the ledger's ceiling of calls whatever the unit's attempts.
    let read = policy(Effect::RecordedRead, Recovery::Observation, 1).expect("recorded read");
    assert_eq!(read.capability(), DestinationCapability::StableKey);
    assert_eq!(read.max_invocations(), 10_000);
    assert_eq!(read.max_queries(), 0);
    assert_eq!(read.stable_window_ms(), Some(31_536_000_000));
    assert_eq!(read.recovery_window_ms(), 31_536_000_000);
    assert_eq!(
        policy(Effect::RecordedRead, Recovery::Observation, 7).expect("read"),
        read,
        "the unit's attempts do not change the slot's contract"
    );
    // The effect class is part of the contract identity: a write never
    // replays as a read, nor the reverse.
    assert_eq!(effect_class(Effect::Idempotent), Ok(1));
    assert_eq!(effect_class(Effect::Write), Ok(2));
    assert_eq!(effect_class(Effect::RecordedRead), Ok(3));
    assert_eq!(
        effect_class(Effect::Read),
        Err(EffectExecutionError::InvalidContract),
        "a plain read is never prepared"
    );
}

#[test]
fn credential_ids_not_material_identify_the_slot() {
    let a = SlotIdentity::from_bindings([("api", "cred-a")]);
    let rotated = SlotIdentity::from_bindings([("api", "cred-a")]);
    let repointed = SlotIdentity::from_bindings([("api", "cred-b")]);
    assert_eq!(
        slot_identity_bytes(&a).expect("bytes"),
        slot_identity_bytes(&rotated).expect("bytes")
    );
    assert_ne!(
        slot_identity_bytes(&a).expect("bytes"),
        slot_identity_bytes(&repointed).expect("bytes")
    );
    assert_eq!(
        slot_identity_bytes(&SlotIdentity::Unbound).expect("bytes"),
        vec![0]
    );
}

#[test]
fn recorded_outcomes_replay_exactly() {
    let operation = OperationId::from_bytes([4; 16]);
    let call = OperationCallId::from_bytes([5; 16]);
    let output = br#"{"receipt":"r-1","amount":7}"#;
    let applied =
        journal_evidence(operation, call, CallOutcome::Applied(output), false).expect("applied");
    assert_eq!(applied.outcome(), KnownOutcome::Succeeded);
    let Ok(RecordedOutcome::Succeeded(bytes)) = replay(operation, &applied) else {
        panic!("an applied output replays");
    };
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).expect("json"),
        serde_json::from_slice::<Value>(output).expect("json")
    );

    let digest_only = journal_evidence(operation, call, CallOutcome::AppliedWithoutOutput, false)
        .expect("digest");
    assert_eq!(
        replay(operation, &digest_only),
        Ok(RecordedOutcome::OutputUnavailable)
    );

    for code in ERROR_KIND_CODES {
        let rejected = journal_evidence(operation, call, CallOutcome::Rejected(code), false)
            .expect("rejected");
        assert_eq!(rejected.outcome(), KnownOutcome::Failed);
        assert_eq!(
            replay(operation, &rejected),
            Ok(RecordedOutcome::Failed(code))
        );
    }

    // Evidence of another operation is never replayed.
    assert_eq!(
        replay(OperationId::from_bytes([6; 16]), &applied).err(),
        Some(EffectExecutionError::InvalidEvidence)
    );

    // An output too large to keep is recorded as applied without output.
    let oversized = serde_json::to_vec(&"a".repeat(1_048_577)).expect("json");
    let kept =
        journal_evidence(operation, call, CallOutcome::Applied(&oversized), false).expect("kept");
    assert_eq!(kept.outcome(), KnownOutcome::Succeeded);
    assert_eq!(
        replay(operation, &kept),
        Ok(RecordedOutcome::OutputUnavailable)
    );

    // A recorded read's answer is never kept digest-only: refused instead.
    let answered = journal_evidence(operation, call, CallOutcome::Applied(output), true)
        .expect("an answer that fits");
    assert!(matches!(
        replay(operation, &answered),
        Ok(RecordedOutcome::Succeeded(_))
    ));
    for refused in [
        CallOutcome::Applied(&oversized),
        CallOutcome::Applied(b"not json"),
        CallOutcome::AppliedWithoutOutput,
    ] {
        assert_eq!(
            journal_evidence(operation, call, refused, true).err(),
            Some(EffectExecutionError::InvalidEvidence),
            "{refused:?}"
        );
    }
}

// ── refusals and verdicts ────────────────────────────────────────────────

#[test]
fn ledger_failures_map_to_one_refusal_and_one_verdict() {
    let slot_id = EffectSlotId::from_storage_bytes([7; 16]);
    let cases = [
        (
            EffectExecutionError::Ledger(OperationLedgerError::OperationMismatch { slot_id }),
            JournalRefusal::Mismatch,
            EffectExecutionError::OccurrenceMismatch,
            effect_journal_verdict::OCCURRENCE_MISMATCH,
        ),
        (
            EffectExecutionError::OccurrenceMismatch,
            JournalRefusal::Mismatch,
            EffectExecutionError::OccurrenceMismatch,
            effect_journal_verdict::OCCURRENCE_MISMATCH,
        ),
        (
            EffectExecutionError::InvalidContract,
            JournalRefusal::Mismatch,
            EffectExecutionError::InvalidContract,
            effect_journal_verdict::INVALID_CONTRACT,
        ),
        (
            EffectExecutionError::Ledger(OperationLedgerError::AcknowledgementUnknown),
            JournalRefusal::AcknowledgementUnknown,
            EffectExecutionError::Ledger(OperationLedgerError::AcknowledgementUnknown),
            effect_journal_verdict::DEFERRED,
        ),
        (
            EffectExecutionError::Ledger(OperationLedgerError::ExecutionLeaseRejected),
            JournalRefusal::LeaseLost,
            EffectExecutionError::Ledger(OperationLedgerError::ExecutionLeaseRejected),
            effect_journal_verdict::DEFERRED,
        ),
        (
            EffectExecutionError::Ledger(OperationLedgerError::Unavailable),
            JournalRefusal::Unavailable,
            EffectExecutionError::Ledger(OperationLedgerError::Unavailable),
            effect_journal_verdict::DEFERRED,
        ),
        (
            EffectExecutionError::InvalidEvidence,
            JournalRefusal::Unavailable,
            EffectExecutionError::InvalidEvidence,
            effect_journal_verdict::INVALID_EVIDENCE,
        ),
    ];
    for (error, refusal, verdict, label) in cases {
        assert_eq!(classify_failure(error), (refusal, verdict), "{error:?}");
        assert_eq!(verdict_label(Some(&verdict)), label, "{error:?}");
    }
    assert_eq!(verdict_label(None), effect_journal_verdict::OK);
    let unknown = EffectExecutionError::JournalOutcomeUnknown {
        slot_id,
        unresolved: 2,
    };
    assert_eq!(unknown.code(), "ENGINE:EFFECT_OUTCOME_UNKNOWN");
    assert_eq!(
        verdict_label(Some(&unknown)),
        effect_journal_verdict::OUTCOME_UNKNOWN
    );
    assert_eq!(
        EffectExecutionError::OccurrenceMismatch.code(),
        "ENGINE:EFFECT_OCCURRENCE_MISMATCH"
    );
}

// ── a journal over a ledger ──────────────────────────────────────────────

/// Counts every ledger call, forwarding to the in-memory ledger; once told
/// to, never answers an outcome write.
#[derive(Debug)]
struct CountingLedger {
    inner: nebula_storage::inmem::InMemoryOperationLedger,
    calls: AtomicUsize,
    hang_outcomes: AtomicBool,
    /// Every later occurrence listing never returns.
    hang_reads: AtomicBool,
    /// Every prepare yields to the scheduler first, so units joined in one
    /// task interleave.
    yield_prepares: AtomicBool,
    /// The next prepare commits and never answers.
    lose_next_prepare_answer: AtomicBool,
    /// The next prepare never reaches the store and never answers.
    stall_next_prepare: AtomicBool,
    /// Notified when a prepare stalls.
    prepare_stalled: tokio::sync::Notify,
    /// The next unsent-failure record is refused, unwritten.
    fail_next_unsent_failure: AtomicBool,
    /// The next call grant commits, then its answer waits for
    /// `release_grant`.
    hold_next_grant: AtomicBool,
    /// Notified when a grant is held.
    grant_held: tokio::sync::Notify,
    /// Releases a held grant's answer.
    release_grant: tokio::sync::Notify,
    /// Notified when an outcome write hangs.
    hung: tokio::sync::Notify,
    /// Every outcome write is refused, unwritten, while set.
    fail_outcomes: AtomicBool,
    /// The next outcome write commits and its acknowledgement is lost.
    lose_next_outcome_ack: AtomicBool,
}

impl CountingLedger {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn count(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
    }

    /// Every later outcome write never returns.
    fn hang_outcomes(&self) {
        self.hang_outcomes.store(true, Ordering::SeqCst);
    }
}

#[async_trait::async_trait]
impl OperationLedger for CountingLedger {
    async fn read_occurrence(
        &self,
        key: &EffectOccurrenceKey<'_>,
    ) -> Result<Option<OperationRecord>, OperationLedgerError> {
        self.count();
        self.inner.read_occurrence(key).await
    }

    async fn read_occurrences(
        &self,
        scope: &Scope,
        execution_id: &str,
        node_key: &str,
    ) -> Result<Vec<EffectOccurrenceRecord>, OperationLedgerError> {
        self.count();
        if self.hang_reads.load(Ordering::SeqCst) {
            return std::future::pending().await;
        }
        self.inner
            .read_occurrences(scope, execution_id, node_key)
            .await
    }

    async fn prepare(
        &self,
        binding: &EffectSlotBinding<'_>,
        fencing: FencingToken,
    ) -> Result<PrepareOutcome, OperationLedgerError> {
        self.count();
        if self.yield_prepares.load(Ordering::SeqCst) {
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
        }
        if self.stall_next_prepare.swap(false, Ordering::SeqCst) {
            // Nothing written; the answer never comes.
            self.prepare_stalled.notify_one();
            return std::future::pending().await;
        }
        let prepared = self.inner.prepare(binding, fencing).await;
        if self.lose_next_prepare_answer.swap(false, Ordering::SeqCst) {
            // Committed; the answer never comes back.
            return std::future::pending().await;
        }
        prepared
    }

    async fn read_exact(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
    ) -> Result<OperationRecord, OperationLedgerError> {
        self.count();
        self.inner.read_exact(scope, slot_id).await
    }

    async fn advance(
        &self,
        scope: &Scope,
        slot_id: EffectSlotId,
        fencing: FencingToken,
        command: &OperationCommand,
    ) -> Result<OperationAdvance, OperationLedgerError> {
        self.count();
        if self.hang_outcomes.load(Ordering::SeqCst)
            && matches!(command, OperationCommand::RecordOutcome(_))
        {
            self.hung.notify_one();
            return std::future::pending().await;
        }
        if matches!(command, OperationCommand::RecordUnsentFailure { .. })
            && self.fail_next_unsent_failure.swap(false, Ordering::SeqCst)
        {
            return Err(OperationLedgerError::Unavailable);
        }
        if matches!(command, OperationCommand::RecordOutcome(_)) {
            if self.fail_outcomes.load(Ordering::SeqCst) {
                return Err(OperationLedgerError::Unavailable);
            }
            if self.lose_next_outcome_ack.swap(false, Ordering::SeqCst) {
                // Committed; the acknowledgement is lost.
                self.inner.advance(scope, slot_id, fencing, command).await?;
                return Err(OperationLedgerError::AcknowledgementUnknown);
            }
        }
        if matches!(command, OperationCommand::GrantInvocation { .. })
            && self.hold_next_grant.swap(false, Ordering::SeqCst)
        {
            // Committed; the answer comes back only when released.
            let granted = self.inner.advance(scope, slot_id, fencing, command).await;
            let released = self.release_grant.notified();
            self.grant_held.notify_one();
            released.await;
            return granted;
        }
        self.inner.advance(scope, slot_id, fencing, command).await
    }
}

/// A clock whose every monotonic reading is `step` after the previous one:
/// each ledger round trip seems to take `step`.
struct SteppingClock {
    origin: Instant,
    step: Duration,
    readings: std::sync::atomic::AtomicU32,
}

impl SteppingClock {
    fn new(step: Duration) -> Arc<Self> {
        Arc::new(Self {
            origin: Instant::now(),
            step,
            readings: std::sync::atomic::AtomicU32::new(0),
        })
    }
}

impl Clock for SteppingClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        nebula_core::accessor::SystemClock.now()
    }

    fn monotonic(&self) -> Instant {
        let reading = self.readings.fetch_add(1, Ordering::SeqCst);
        self.origin + self.step * reading
    }
}

/// One leased execution over a counting in-memory ledger.
struct Harness {
    executions: nebula_storage::InMemoryExecutionStore,
    ledger: Arc<CountingLedger>,
    scope: Scope,
    execution_id: ExecutionId,
    fencing: FencingToken,
    metrics: MetricsRegistry,
    manager: Arc<Manager>,
    desk: Arc<Desk>,
}

impl Harness {
    async fn new() -> Self {
        let executions = nebula_storage::InMemoryExecutionStore::new();
        let ledger = Arc::new(CountingLedger {
            inner: nebula_storage::inmem::InMemoryOperationLedger::new(&executions),
            calls: AtomicUsize::new(0),
            hang_outcomes: AtomicBool::new(false),
            hang_reads: AtomicBool::new(false),
            yield_prepares: AtomicBool::new(false),
            lose_next_prepare_answer: AtomicBool::new(false),
            stall_next_prepare: AtomicBool::new(false),
            prepare_stalled: tokio::sync::Notify::new(),
            fail_next_unsent_failure: AtomicBool::new(false),
            hold_next_grant: AtomicBool::new(false),
            grant_held: tokio::sync::Notify::new(),
            release_grant: tokio::sync::Notify::new(),
            hung: tokio::sync::Notify::new(),
            fail_outcomes: AtomicBool::new(false),
            lose_next_outcome_ack: AtomicBool::new(false),
        });
        let scope = Scope::new("workspace-a", "org-a");
        let execution_id = ExecutionId::new();
        executions
            .create(
                &scope,
                &execution_id.to_string(),
                "workflow",
                serde_json::json!({"status": "Created"}),
            )
            .await
            .expect("execution row");
        let fencing = executions
            .acquire_lease(
                &scope,
                &execution_id.to_string(),
                "runner",
                Duration::from_secs(30),
            )
            .await
            .expect("lease")
            .expect("granted");
        let desk = Arc::new(Desk::default());
        let manager = Arc::new(Manager::new());
        manager
            .register(RegistrationSpec {
                resource: Gateway(Arc::clone(&desk)),
                config: GatewayConfig { endpoint: 1 },
                scope: nebula_core::ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::<Gateway>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register the gateway");
        Self {
            executions,
            ledger,
            scope,
            execution_id,
            fencing,
            metrics: MetricsRegistry::new(),
            manager,
            desk,
        }
    }

    /// The journal of attempt `attempt_generation` of node `charge`.
    fn journal(&self, attempt_generation: u64) -> NodeEffectJournal {
        self.journal_with_clock(
            attempt_generation,
            Arc::new(nebula_core::accessor::SystemClock),
        )
    }

    /// The journal of attempt `attempt_generation` of node `charge`, timing
    /// its grants by `clock`.
    fn journal_with_clock(
        &self,
        attempt_generation: u64,
        clock: Arc<dyn Clock>,
    ) -> NodeEffectJournal {
        NodeEffectJournal::new(self.authority(attempt_generation, clock, JournalShape::Flat))
    }

    /// The journal of attempt `attempt_generation` of a stateful node
    /// `charge`: it admits units only while an iteration is open.
    fn stateful_journal(&self, attempt_generation: u64) -> NodeEffectJournal {
        NodeEffectJournal::new(self.authority(
            attempt_generation,
            Arc::new(nebula_core::accessor::SystemClock),
            JournalShape::Iterated,
        ))
    }

    /// The journal of attempt `attempt_generation` of a stateless node
    /// `charge` that prepares at most `slot_cap` fresh slots.
    fn capped_journal(&self, attempt_generation: u64, slot_cap: u32) -> NodeEffectJournal {
        NodeEffectJournal::with_slot_cap(
            self.authority(
                attempt_generation,
                Arc::new(nebula_core::accessor::SystemClock),
                JournalShape::Flat,
            ),
            slot_cap,
        )
    }

    fn authority(
        &self,
        attempt_generation: u64,
        clock: Arc<dyn Clock>,
        shape: JournalShape,
    ) -> JournalAuthority {
        JournalAuthority {
            ledger: Arc::clone(&self.ledger) as Arc<dyn OperationLedger>,
            scope: self.scope.clone(),
            fencing: self.fencing,
            execution_id: self.execution_id,
            node_key: NodeKey::new("charge").expect("node key"),
            action_key: "billing.charge".to_owned(),
            action_version: semver::Version::new(1, 0, 0),
            attempt_generation,
            clock,
            metrics: self.metrics.clone(),
            shape,
            checkpoints: None,
        }
    }

    /// Points the gateway row at another endpoint.
    fn reload(&self, endpoint: u64) {
        assert_eq!(
            self.manager
                .reload_config::<Gateway>(
                    GatewayConfig { endpoint },
                    &nebula_core::ScopeLevel::Global
                )
                .expect("reload"),
            nebula_resource::ReloadOutcome::SwappedImmediately
        );
    }

    fn phase(slot: &EffectOccurrenceRecord) -> EffectPhase {
        slot.record().protocol().expect("protocol").phase()
    }

    /// A fresh manager — as a restarted process builds it — with the gateway
    /// row registered under a configuration rebuilt from scratch, calling
    /// the same provider desk.
    fn rebuilt_manager(&self) -> Arc<Manager> {
        let manager = Arc::new(Manager::new());
        manager
            .register(RegistrationSpec {
                resource: Gateway(Arc::clone(&self.desk)),
                config: GatewayConfig { endpoint: 1 },
                scope: nebula_core::ScopeLevel::Global,
                slot_identity: SlotIdentity::Unbound,
                topology: Resident::<Gateway>::new(ResidentConfig::default()),
                recovery_gate: None,
                rate_limit: None,
            })
            .expect("register the gateway again");
        manager
    }

    /// The gateway's handle under `journal`.
    fn handle(&self, journal: &NodeEffectJournal) -> ResourceHandle<Gateway> {
        Self::handle_on(&self.manager, journal)
    }

    /// The gateway's handle on `manager` under `journal`.
    fn handle_on(manager: &Manager, journal: &NodeEffectJournal) -> ResourceHandle<Gateway> {
        *manager
            .handle_any_journaled(
                &Gateway::key(),
                &ResourceContext::minimal(
                    nebula_core::scope::Scope::default(),
                    CancellationToken::new(),
                ),
                &AcquireOptions::default(),
                &SlotIdentity::Unbound,
                Arc::new(journal.clone()),
            )
            .expect("journaled handle")
            .downcast::<ResourceHandle<Gateway>>()
            .expect("typed handle")
    }

    async fn slots(&self) -> Vec<EffectOccurrenceRecord> {
        self.ledger
            .inner
            .read_occurrences(&self.scope, &self.execution_id.to_string(), "charge")
            .await
            .expect("occurrences")
    }

    fn counter(&self, name: &str, pairs: &[(&str, &str)]) -> u64 {
        let labels = self.metrics.interner().label_set(pairs);
        self.metrics
            .counter_labeled(name, &labels)
            .expect("counter")
            .get()
    }
}

/// How the gateway answers the next call.
#[derive(Debug, Clone, Copy)]
enum Reply {
    Applied,
    /// Applied, and the answer was lost.
    Lost,
    /// Applied, and the call never returns.
    Hang,
    /// Applied, and the call returns once the desk is released.
    Held,
    /// Throttled: the provider applied nothing.
    Throttled,
    /// Definitively rejected: the provider applied nothing.
    Rejected,
}

/// The fake gateway: every call it received, with the key it carried.
#[derive(Debug, Default)]
struct Desk {
    keys: Mutex<Vec<Option<String>>>,
    replies: Mutex<VecDeque<Reply>>,
    entered: tokio::sync::Notify,
    /// Releases a [`Reply::Held`] call.
    release: tokio::sync::Notify,
}

impl Desk {
    fn script(&self, replies: &[Reply]) {
        self.replies
            .lock()
            .expect("replies")
            .extend(replies.iter().copied());
    }

    fn keys(&self) -> Vec<Option<String>> {
        self.keys.lock().expect("keys").clone()
    }

    async fn charge(&self, key: Option<String>) -> Result<u64, OperationError> {
        let receipt = {
            let mut keys = self.keys.lock().expect("keys");
            keys.push(key);
            u64::try_from(keys.len()).expect("fits")
        };
        let reply = self
            .replies
            .lock()
            .expect("replies")
            .pop_front()
            .unwrap_or(Reply::Applied);
        self.entered.notify_one();
        match reply {
            Reply::Applied => Ok(receipt),
            Reply::Lost => Err(OperationError::interrupted("connection reset")),
            Reply::Hang => std::future::pending().await,
            Reply::Held => {
                self.release.notified().await;
                Ok(receipt)
            },
            Reply::Throttled => Err(OperationError::throttled(None)),
            Reply::Rejected => Err(OperationError::rejected("declined")),
        }
    }
}

/// The gateway row's configuration: which endpoint it calls.
#[derive(Clone, nebula_schema::Schema)]
struct GatewayConfig {
    endpoint: u64,
}

impl nebula_resource::ResourceConfig for GatewayConfig {
    fn fingerprint(&self) -> u64 {
        nebula_resource::ConfigFingerprint::new()
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

#[derive(Clone)]
struct Gateway(Arc<Desk>);

#[async_trait::async_trait]
impl Provider for Gateway {
    type Config = GatewayConfig;
    type Instance = Arc<Desk>;
    type Topology = Resident<Self>;

    fn key() -> ResourceKey {
        ResourceKey::new("billing.gateway").expect("resource key")
    }

    fn metadata() -> ResourceMetadataDraft {
        ResourceMetadataDraft::new(
            Self::key(),
            nebula_resource::metadata_name!("JournalGateway"),
            "",
        )
    }

    async fn create(
        &self,
        _: &GatewayConfig,
        _: &ResourceContext,
    ) -> Result<Arc<Desk>, nebula_resource::error::Error> {
        Ok(Arc::clone(&self.0))
    }
}

nebula_resource::no_credential_slots!(Gateway);

impl ResidentProvider for Gateway {}

/// A charge of `order`: an opaque `Write`, or — `IDEM` — an `Idempotent`
/// charge the provider deduplicates by key, allowed two attempts.
#[derive(Serialize, Deserialize)]
struct Charge<const IDEM: bool> {
    order: u64,
}

impl<const IDEM: bool> Operation<Gateway> for Charge<IDEM> {
    type Output = u64;
    const KEY: &'static str = "billing.charge";
    const EFFECT: Effect = if IDEM {
        Effect::Idempotent
    } else {
        Effect::Write
    };

    fn max_attempts(&self) -> NonZeroU32 {
        if IDEM {
            NonZeroU32::MIN.saturating_add(1)
        } else {
            NonZeroU32::MIN
        }
    }

    async fn run(self, cx: &mut OperationCx<'_, Gateway>) -> Result<u64, OperationError> {
        let key = cx.idempotency_key().map(ToString::to_string);
        cx.call(Cost::ONE, async move |desk, ()| {
            desk.charge(key.clone()).await
        })
        .await
    }
}

/// [`Charge`]'s opaque `Write` with the same key, version and request,
/// recorded as a digest only (`RECORD_OUTPUT = false`).
#[derive(Serialize, Deserialize)]
struct ChargeDigest {
    order: u64,
}

impl Operation<Gateway> for ChargeDigest {
    type Output = u64;
    const KEY: &'static str = "billing.charge";
    const RECORD_OUTPUT: bool = false;

    async fn run(self, cx: &mut OperationCx<'_, Gateway>) -> Result<u64, OperationError> {
        let key = cx.idempotency_key().map(ToString::to_string);
        cx.call(Cost::ONE, async move |desk, ()| {
            desk.charge(key.clone()).await
        })
        .await
    }
}

/// An `Idempotent` charge the provider deduplicates for two seconds only,
/// with one attempt.
#[derive(Serialize, Deserialize)]
struct Windowed {
    order: u64,
}

/// [`Windowed`]'s key window.
const SHORT_WINDOW: Duration = Duration::from_secs(2);

impl Operation<Gateway> for Windowed {
    type Output = u64;
    const KEY: &'static str = "billing.windowed";
    const EFFECT: Effect = Effect::Idempotent;
    const KEY_WINDOW: Duration = SHORT_WINDOW;

    async fn run(self, cx: &mut OperationCx<'_, Gateway>) -> Result<u64, OperationError> {
        let key = cx.idempotency_key().map(ToString::to_string);
        cx.call(Cost::ONE, async move |desk, ()| {
            desk.charge(key.clone()).await
        })
        .await
    }
}

/// Reads without an effect.
#[derive(Serialize, Deserialize)]
struct Balance;

impl Operation<Gateway> for Balance {
    type Output = usize;
    const KEY: &'static str = "billing.balance";
    const EFFECT: Effect = Effect::Read;

    async fn run(self, cx: &mut OperationCx<'_, Gateway>) -> Result<usize, OperationError> {
        cx.call(Cost::ONE, async |desk, ()| {
            Ok(desk.keys.lock().expect("keys").len())
        })
        .await
    }
}

/// A recorded read — a model call — whose answer (the desk's receipt)
/// changes on every real call.
#[derive(Serialize, Deserialize)]
struct Ask {
    prompt: String,
}

impl Ask {
    fn new() -> Self {
        Self {
            prompt: "what next?".to_owned(),
        }
    }
}

impl Operation<Gateway> for Ask {
    type Output = u64;
    const KEY: &'static str = "model.ask";
    const EFFECT: Effect = Effect::RecordedRead;

    async fn run(self, cx: &mut OperationCx<'_, Gateway>) -> Result<u64, OperationError> {
        cx.call(Cost::ONE, async move |desk, ()| desk.charge(None).await)
            .await
    }
}

const DRAIN: Duration = Duration::from_secs(5);

#[test]
fn ordinals_are_one_node_wide_sequence_from_zero() {
    let executions = nebula_storage::InMemoryExecutionStore::new();
    let journal = NodeEffectJournal::new(JournalAuthority {
        ledger: Arc::new(nebula_storage::inmem::InMemoryOperationLedger::new(
            &executions,
        )),
        scope: Scope::new("workspace-a", "org-a"),
        fencing: FencingToken::from_generation(1),
        execution_id: ExecutionId::new(),
        node_key: NodeKey::new("charge").expect("node key"),
        action_key: "billing.charge".to_owned(),
        action_version: semver::Version::new(1, 0, 0),
        attempt_generation: 1,
        clock: Arc::new(nebula_core::accessor::SystemClock),
        metrics: MetricsRegistry::new(),
        shape: JournalShape::Flat,
        checkpoints: None,
    });
    // Every resource and unit kind shares the node attempt's sequence.
    assert_eq!(journal.next_ordinal(), 0);
    assert_eq!(journal.next_ordinal(), 1);
    assert_eq!(journal.next_ordinal(), 2);
}

#[tokio::test]
async fn iteration_labels_restart_their_ordinal_per_iteration() {
    let harness = Harness::new().await;
    let journal = harness.journal(1);
    assert_eq!(
        journal.next_occurrence(),
        "unit/v1/#000000",
        "a stateless node's flat label"
    );
    journal.begin_iteration(0).expect("the first iteration");
    assert_eq!(journal.next_occurrence(), "it0/unit/v1/#000000");
    assert_eq!(journal.next_occurrence(), "it0/unit/v1/#000001");
    journal
        .end_iteration(DRAIN, true)
        .await
        .expect("nothing in flight");
    journal.begin_iteration(1).expect("the second iteration");
    assert_eq!(
        journal.next_occurrence(),
        "it1/unit/v1/#000000",
        "the ordinal restarts"
    );
    journal
        .end_iteration(DRAIN, true)
        .await
        .expect("nothing in flight");
    journal.begin_iteration(9_999).expect("the last iteration");
    assert_eq!(journal.next_occurrence(), "it9999/unit/v1/#000000");
    journal
        .end_iteration(DRAIN, true)
        .await
        .expect("nothing in flight");
    assert_eq!(
        harness.ledger.calls(),
        1,
        "labels write nothing; the barriers share the node's one read"
    );

    // An iteration past the last, or not after the open one, is refused.
    assert_eq!(
        harness.journal(1).begin_iteration(10_000),
        Err(EffectExecutionError::InvalidContract)
    );
    let backwards = harness.journal(1);
    backwards.begin_iteration(3).expect("iteration 3");
    assert_eq!(
        backwards.begin_iteration(3),
        Err(EffectExecutionError::InvalidContract)
    );
}

#[tokio::test(start_paused = true)]
async fn an_iteration_begins_only_with_no_unit_in_flight() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Held]);
    let journal = harness.stateful_journal(1);
    journal.begin_iteration(0).expect("the first iteration");
    // A unit of iteration 0 is mid-call as iteration 1 would begin.
    let unit = tokio::spawn(
        harness
            .handle(&journal)
            .submit(Charge::<false> { order: 40 }),
    );
    tokio::time::timeout(Duration::from_secs(5), harness.desk.entered.notified())
        .await
        .expect("the call reached the provider");
    assert_eq!(
        journal.begin_iteration(1),
        Err(EffectExecutionError::IterationUnitsOutstanding { iteration: 1 })
    );
    assert!(
        !journal.is_closed(),
        "nothing was waited for: the journal stays open"
    );
    assert_eq!(
        journal.begin_iteration(2),
        Err(EffectExecutionError::IterationUnitsOutstanding { iteration: 1 }),
        "no later iteration runs"
    );

    // The node's conclusion drains the unit within its full limit: its call
    // settles and is no unknown outcome.
    let desk = Arc::clone(&harness.desk);
    let release = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(10)).await;
        desk.release.notify_one();
    });
    let started = tokio::time::Instant::now();
    assert_eq!(
        journal.conclude(Duration::from_mins(1)).await,
        Err(EffectExecutionError::IterationUnitsOutstanding { iteration: 1 })
    );
    assert_eq!(started.elapsed(), Duration::from_secs(10), "it waited");
    assert_eq!(unit.await.expect("the unit ran").expect("it settled"), 1);
    assert_eq!(
        Harness::phase(&harness.slots().await[0]),
        EffectPhase::Resolved
    );
    release.await.expect("released");
}

#[tokio::test(start_paused = true)]
async fn a_unit_outliving_its_iteration_fails_the_barrier_and_the_node_unknown() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Hang]);
    let journal = harness.stateful_journal(1);
    journal.begin_iteration(0).expect("the first iteration");
    let unit = tokio::spawn(
        harness
            .handle(&journal)
            .submit(Charge::<false> { order: 41 }),
    );
    tokio::time::timeout(Duration::from_secs(5), harness.desk.entered.notified())
        .await
        .expect("the call reached the provider");
    let started = tokio::time::Instant::now();
    assert_eq!(
        journal.end_iteration(Duration::from_secs(5), true).await,
        Err(EffectExecutionError::IterationUnitsOutstanding { iteration: 0 })
    );
    assert_eq!(started.elapsed(), Duration::from_secs(5), "bounded drain");
    assert!(journal.is_closed());
    assert!(journal.begin_iteration(1).is_err(), "no next iteration");

    // The node's verdict records the leaked call as ambiguous: an opaque
    // write's outcome is unknown. It does not wait out the drain again.
    let started = tokio::time::Instant::now();
    let verdict = journal.conclude(Duration::from_mins(1)).await;
    assert!(
        matches!(
            verdict,
            Err(EffectExecutionError::JournalOutcomeUnknown { unresolved: 1, .. })
        ),
        "{verdict:?}"
    );
    assert!(started.elapsed() < Duration::from_mins(1));
    assert_eq!(
        Harness::phase(&harness.slots().await[0]),
        EffectPhase::OutcomeUnknown
    );
    assert_eq!(harness.desk.keys().len(), 1);
    unit.abort();
}

#[tokio::test]
async fn an_unknown_outcome_the_action_swallowed_stops_the_next_iteration() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Lost]);
    let journal = harness.stateful_journal(1);
    journal.begin_iteration(0).expect("the first iteration");
    let lost = harness
        .handle(&journal)
        .submit(Charge::<false> { order: 42 })
        .await
        .expect_err("the answer was lost");
    assert_eq!(lost.sent(), SentState::MaybeSent, "{lost}");
    // The action swallows the error; the barrier does not.
    let stopped = journal.end_iteration(DRAIN, true).await;
    assert!(
        matches!(
            stopped,
            Err(EffectExecutionError::JournalOutcomeUnknown { unresolved: 1, .. })
        ),
        "{stopped:?}"
    );
    assert_eq!(journal.begin_iteration(1), stopped, "no next iteration");
    assert!(matches!(
        journal.conclude(DRAIN).await,
        Err(EffectExecutionError::JournalOutcomeUnknown { .. })
    ));
    assert_eq!(harness.desk.keys().len(), 1);
}

#[tokio::test]
async fn a_unit_submitted_between_iterations_is_refused_and_fails_the_node() {
    let harness = Harness::new().await;
    // Before the first iteration opens, nothing is admitted.
    assert_eq!(
        harness.stateful_journal(1).admit().map(|_| ()),
        Err(JournalRefusal::BetweenRuns)
    );

    let journal = harness.stateful_journal(1);
    journal.begin_iteration(0).expect("it0");
    journal.end_iteration(DRAIN, true).await.expect("it0 ends");
    // A detached task submits after iteration 0 ended, before iteration 1
    // begins: the unit belongs to neither and is refused unsent.
    let refused = harness
        .handle(&journal)
        .submit(Charge::<false> { order: 30 })
        .await
        .expect_err("between iterations");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert_eq!(
        *refused.kind(),
        nebula_resource::error::ErrorKind::Permanent
    );
    assert_eq!(
        refused.detail(),
        "effect submitted while its owner has no open run (between stateful iterations); unit \
         refused"
    );
    assert_eq!(
        journal.begin_iteration(1),
        Err(EffectExecutionError::IterationUnitsOutstanding { iteration: 0 }),
        "the node's verdict records the violation"
    );
    assert_eq!(
        journal.conclude(DRAIN).await,
        Err(EffectExecutionError::IterationUnitsOutstanding { iteration: 0 })
    );
    assert_eq!(harness.desk.keys().len(), 0, "nothing sent");
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_REFUSALS_TOTAL,
            &[("step", "submit"), ("refusal", "between_runs")]
        ),
        2
    );
    // A stateless journal has no runs: it admits without an iteration.
    assert!(harness.journal(1).admit().is_ok());
}

#[test]
fn admission_waits_for_the_rollover_it_races() {
    let executions = nebula_storage::InMemoryExecutionStore::new();
    let journal = NodeEffectJournal::new(JournalAuthority {
        ledger: Arc::new(nebula_storage::inmem::InMemoryOperationLedger::new(
            &executions,
        )),
        scope: Scope::new("workspace-a", "org-a"),
        fencing: FencingToken::from_generation(1),
        execution_id: ExecutionId::new(),
        node_key: NodeKey::new("charge").expect("node key"),
        action_key: "billing.charge".to_owned(),
        action_version: semver::Version::new(1, 0, 0),
        attempt_generation: 1,
        clock: Arc::new(nebula_core::accessor::SystemClock),
        metrics: MetricsRegistry::new(),
        shape: JournalShape::Iterated,
        checkpoints: None,
    });
    journal.begin_iteration(0).expect("it0");
    // The rollover holds the journal's state: an admission racing it waits
    // for the transition instead of slipping a ticket past its check.
    let rollover = journal.state();
    let admitting = std::thread::spawn({
        let journal = journal.clone();
        move || journal.admit().map(|ticket| (ticket, journal))
    });
    std::thread::sleep(Duration::from_millis(100));
    assert!(!admitting.is_finished(), "admission waits for the rollover");
    assert_eq!(journal.inner.in_flight.load(Ordering::SeqCst), 0);
    drop(rollover);
    let (ticket, _) = admitting
        .join()
        .expect("admitting thread")
        .expect("admitted while iteration 0 is open");
    // Once admitted, the ticket is seen by the next rollover.
    assert_eq!(
        journal.begin_iteration(1),
        Err(EffectExecutionError::IterationUnitsOutstanding { iteration: 1 })
    );
    drop(ticket);
}

#[tokio::test]
async fn the_slot_cap_counts_fresh_slots_and_replays_any_number_recorded() {
    let harness = Harness::new().await;
    let first = harness.capped_journal(1, 10);
    for order in 1..=3 {
        harness
            .handle(&first)
            .submit(Charge::<false> { order })
            .await
            .expect("applied");
    }
    assert_eq!(first.conclude(DRAIN).await, Ok(()));

    // A later attempt under a cap of one fresh slot replays all three
    // recorded effects, prepares one fresh slot, and refuses the next.
    let retry = harness.capped_journal(2, 1);
    let handle = harness.handle(&retry);
    for order in 1..=3 {
        assert_eq!(
            handle
                .submit(Charge::<false> { order })
                .await
                .expect("replayed"),
            order,
            "the recorded receipt"
        );
    }
    assert_eq!(
        handle
            .submit(Charge::<false> { order: 4 })
            .await
            .expect("the one fresh slot"),
        4
    );
    let refused = handle
        .submit(Charge::<false> { order: 5 })
        .await
        .expect_err("a second fresh slot is over the cap");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert_eq!(
        refused.detail(),
        "effect journal slot cap reached; unit refused"
    );
    // The action catches the refusal and submits again: the refused
    // position is met, not abandoned, so the next one is refused by the cap
    // too instead of deferring in place of the terminal cap verdict.
    let again = handle
        .submit(Charge::<false> { order: 6 })
        .await
        .expect_err("still over the cap");
    assert_eq!(
        again.detail(),
        "effect journal slot cap reached; unit refused"
    );
    assert_eq!(harness.desk.keys().len(), 4, "the replays called nothing");
    assert_eq!(
        retry.conclude(DRAIN).await,
        Err(EffectExecutionError::JournalSlotCapExceeded { cap: 1 })
    );
}

#[tokio::test(start_paused = true)]
async fn a_barrier_read_that_never_answers_defers_within_the_budget() {
    let harness = Harness::new().await;
    harness.ledger.hang_reads.store(true, Ordering::SeqCst);
    // An iteration with no effect: its barrier is the first to read what
    // earlier attempts recorded.
    let journal = harness.stateful_journal(1);
    journal.begin_iteration(0).expect("it0");
    let started = tokio::time::Instant::now();
    let stopped = journal.end_iteration(Duration::from_mins(1), true).await;
    assert_eq!(
        stopped,
        Err(EffectExecutionError::Ledger(
            OperationLedgerError::Unavailable
        ))
    );
    assert!(stopped.is_err_and(EffectExecutionError::is_deferred));
    assert_eq!(
        started.elapsed(),
        Duration::from_mins(1),
        "bounded by the barrier budget"
    );
    // The node defers instead of hanging: the verdict still tries its own
    // read for an unknown outcome a deferral must not mask, within the
    // drain limit.
    let started = tokio::time::Instant::now();
    let verdict = journal.conclude(Duration::from_mins(1)).await;
    assert!(verdict.is_err_and(EffectExecutionError::is_deferred));
    assert!(started.elapsed() <= Duration::from_mins(1));

    // A spent budget still gives the read its floor.
    let spent = harness.stateful_journal(1);
    spent.begin_iteration(0).expect("it0");
    let started = tokio::time::Instant::now();
    assert!(spent.end_iteration(Duration::ZERO, true).await.is_err());
    assert_eq!(started.elapsed(), FINAL_READ_FLOOR);
}

#[tokio::test(start_paused = true)]
async fn a_submission_after_a_cancelled_iteration_is_refused_and_not_waited_for() {
    let harness = Harness::new().await;
    let journal = harness.stateful_journal(1);
    journal.begin_iteration(0).expect("it0");
    // The node is cancelled mid-iteration; a detached task submits later.
    journal.abandon_iteration();
    let refused = harness
        .handle(&journal)
        .submit(Charge::<false> { order: 19 })
        .await
        .expect_err("the iteration was cancelled");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert_eq!(refused.detail(), "effect owner closed; unit refused");
    // Nothing is in flight: the conclusion does not wait, and the
    // cancellation stays the node's own outcome.
    let started = tokio::time::Instant::now();
    assert_eq!(journal.conclude(Duration::from_mins(1)).await, Ok(()));
    assert!(started.elapsed() < Duration::from_mins(1));
    assert_eq!(harness.desk.keys().len(), 0);

    // Cancelled between iterations (during a `Continue` delay): the same,
    // not a barrier violation that would turn the cancellation into a
    // failure.
    let between = harness.stateful_journal(2);
    between.begin_iteration(0).expect("it0");
    between.end_iteration(DRAIN, true).await.expect("it0 ends");
    between.abandon_iteration();
    let refused = harness
        .handle(&between)
        .submit(Charge::<false> { order: 18 })
        .await
        .expect_err("the node was cancelled between iterations");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert_eq!(refused.detail(), "effect owner closed; unit refused");
    assert_eq!(between.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 0);
}

#[tokio::test(start_paused = true)]
async fn a_prepare_that_never_answered_blocks_every_fresh_slot_above_it() {
    let harness = Harness::new().await;
    // The lower effect's prepare commits; its answer is lost and the unit
    // runs past its deadline.
    harness
        .ledger
        .lose_next_prepare_answer
        .store(true, Ordering::SeqCst);
    let first = harness.journal(1);
    let lower = harness
        .handle(&first)
        .submit(Charge::<false> { order: 20 })
        .await
        .expect_err("no answer before the deadline");
    assert_eq!(lower.sent(), SentState::NotSent, "{lower}");
    assert_eq!(harness.slots().await.len(), 1, "yet the row was written");
    // The higher fresh effect is not sent in this attempt: a recovery would
    // otherwise run the lower one after it.
    let higher = harness
        .handle(&first)
        .submit(Charge::<false> { order: 21 })
        .await
        .expect_err("above an uncertain position");
    assert_eq!(higher.sent(), SentState::NotSent, "{higher}");
    assert_eq!(harness.desk.keys().len(), 0, "nothing sent");
    let verdict = first.conclude(DRAIN).await;
    assert_eq!(
        verdict,
        Err(EffectExecutionError::Ledger(
            OperationLedgerError::AcknowledgementUnknown
        ))
    );
    assert!(verdict.is_err_and(EffectExecutionError::is_deferred));

    // The next attempt replays in order: the lower effect first.
    let recovery = harness.journal(2);
    let handle = harness.handle(&recovery);
    assert_eq!(
        handle
            .submit(Charge::<false> { order: 20 })
            .await
            .expect("lower"),
        1
    );
    assert_eq!(
        handle
            .submit(Charge::<false> { order: 21 })
            .await
            .expect("higher"),
        2
    );
    assert_eq!(recovery.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 2, "each sent once, in order");
}

#[tokio::test]
async fn a_recorded_effect_below_an_applied_one_is_never_run_after_it() {
    let harness = Harness::new().await;
    // The lower charge is throttled (nothing applied, its slot changed
    // nothing); the action goes on and the higher one applies.
    harness.desk.script(&[Reply::Throttled, Reply::Applied]);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    handle
        .submit(Charge::<false> { order: 22 })
        .await
        .expect_err("throttled");
    handle
        .submit(Charge::<false> { order: 23 })
        .await
        .expect("applied");
    assert_eq!(
        first.conclude_node(DRAIN, false).await,
        Ok(Concluded::Clean)
    );

    // A recovery reaching the lower charge again would apply it after the
    // higher one: not sent — it fails unsent again, as the program saw it
    // fail and moved past it — and the run goes on: the higher charge
    // replays, and the node is not stranded.
    let recovery = harness.journal(2);
    let handle = harness.handle(&recovery);
    let superseded = handle
        .submit(Charge::<false> { order: 22 })
        .await
        .expect_err("not sent again");
    assert_eq!(superseded.sent(), SentState::NotSent, "{superseded}");
    assert_eq!(
        superseded.detail(),
        "effect failed unsent in an earlier run that moved past it; not sent again"
    );
    assert_eq!(
        handle
            .submit(Charge::<false> { order: 23 })
            .await
            .expect("replayed"),
        2
    );
    assert_eq!(harness.desk.keys().len(), 2, "no further call");
    assert_eq!(recovery.conclude(DRAIN).await, Ok(()));
}

#[tokio::test]
async fn a_noted_failure_never_lets_a_skipped_recorded_effect_be_routed_past() {
    let harness = Harness::new().await;
    let first = harness.stateful_journal(1);
    first.begin_iteration(0).expect("it0");
    harness
        .handle(&first)
        .submit(Charge::<false> { order: 26 })
        .await
        .expect("applied at it0/#0");

    // A retry whose iteration 0 fails before its effect, while a detached
    // task submits between iterations: a routable barrier failure is noted.
    let between_runs = |journal: &NodeEffectJournal| {
        assert_eq!(
            journal.admit().map(|_| ()),
            Err(JournalRefusal::BetweenRuns)
        );
    };
    let failing = harness.stateful_journal(2);
    failing.begin_iteration(0).expect("it0");
    failing
        .end_iteration(DRAIN, false)
        .await
        .expect("a failing iteration keeps its own failure");
    between_runs(&failing);
    // The node skipped the applied effect: retried, never routed past.
    assert_eq!(
        failing.conclude_node(DRAIN, false).await,
        Ok(Concluded::SkippedRecordedEffect)
    );

    // A detached unit admitted and never prepared outlives its iteration.
    let detached = harness.stateful_journal(3);
    detached.begin_iteration(0).expect("it0");
    let ticket = detached.admit().expect("admitted in it0");
    assert_eq!(
        detached.begin_iteration(1),
        Err(EffectExecutionError::IterationUnitsOutstanding { iteration: 1 })
    );
    drop(ticket);
    assert_eq!(
        detached.conclude_node(DRAIN, false).await,
        Ok(Concluded::SkippedRecordedEffect)
    );

    // About to succeed past the skipped effect: a mismatch, whatever was
    // noted.
    let succeeding = harness.stateful_journal(4);
    succeeding.begin_iteration(0).expect("it0");
    succeeding
        .end_iteration(DRAIN, false)
        .await
        .expect("it0 ends");
    between_runs(&succeeding);
    assert_eq!(
        succeeding.conclude_node(DRAIN, true).await,
        Err(EffectExecutionError::OccurrenceMismatch)
    );
    assert_eq!(harness.desk.keys().len(), 1);
}

#[tokio::test]
async fn a_crossed_stable_key_effect_below_a_later_applied_one_is_not_granted_again() {
    let harness = Harness::new().await;
    // The lower charge (stable key, two attempts) is mid-call when the
    // process dies; the higher one, awaited after it settled... here: the
    // higher one is prepared after the lower unit was gone (the action
    // dropped it), and applies.
    harness.desk.script(&[Reply::Hang, Reply::Applied]);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    let lower = tokio::spawn(handle.submit(Charge::<true> { order: 24 }));
    tokio::time::timeout(Duration::from_secs(5), harness.desk.entered.notified())
        .await
        .expect("the lower call reached the provider");
    lower.abort();
    let _ = lower.await;
    // The lower unit's runtime task still holds its position until the
    // call ends; the test retires it as the dead process would.
    first.finish_occurrence("unit/v1/#000000");
    handle
        .submit(Charge::<false> { order: 25 })
        .await
        .expect("applied after the lower one");
    let recorded = harness.slots().await;
    assert_eq!(
        recorded[1]
            .record()
            .protocol()
            .expect("protocol")
            .concurrent_with(),
        Some(&[][..]),
        "the lower unit was gone when the higher one began"
    );

    // The recovery would grant the lower charge again under its key — after
    // the higher one applied. Its outcome is unknown instead: no call.
    let recovery = harness.journal(2);
    let unknown = harness
        .handle(&recovery)
        .submit(Charge::<true> { order: 24 })
        .await
        .expect_err("not granted again");
    assert_eq!(
        *unknown.kind(),
        nebula_resource::error::ErrorKind::OutcomeUnknown
    );
    assert_eq!(harness.desk.keys().len(), 2, "no further call");
    assert!(matches!(
        recovery.conclude(DRAIN).await,
        Err(EffectExecutionError::JournalOutcomeUnknown { .. })
    ));
}

/// The single position `ordinal`, as a run.
const fn at(ordinal: u32) -> PositionRange {
    match PositionRange::new(ordinal, ordinal) {
        Some(run) => run,
        None => panic!("a single position is a run"),
    }
}

/// The prior occurrences of recorded effects (no recorded read), each with
/// its weight and concurrency list.
fn effects<'a>(
    records: impl IntoIterator<Item = (&'a str, SlotWeight, Option<&'a [PositionRange]>)>,
) -> PriorOccurrences {
    PriorOccurrences::from_records(
        records
            .into_iter()
            .map(|(label, weight, concurrent)| (label, weight, concurrent, false)),
    )
}

#[test]
fn an_unsettled_slot_reorders_only_past_a_later_effect_the_program_ordered() {
    use SlotWeight::{Applied, Inert, Unsettled};
    const NONE: Option<&[PositionRange]> = Some(&[]);
    const AT_0: &[PositionRange] = &[at(0)];
    const AT_1: &[PositionRange] = &[at(1)];
    let lower = "unit/v1/#000000";
    let with_higher = |concurrent: Option<&'static [PositionRange]>| {
        effects([
            (lower, Unsettled, NONE),
            ("unit/v1/#000001", Applied, concurrent),
        ])
    };
    // The higher effect was prepared while the lower unit was open: they
    // ran concurrently, and the lower one replays.
    assert!(!with_higher(Some(AT_0)).reorders_at(lower));
    // The lower unit settled first (an explicit empty list): replaying it
    // now would reverse them.
    assert!(with_higher(NONE).reorders_at(lower));
    // A higher slot an older journal recorded without the list has unknown
    // concurrency: it orders nothing, as that journal had no order rule.
    assert!(!with_higher(None).reorders_at(lower));
    // A settled lower slot replays its outcome: no reordering.
    assert!(
        !effects([(lower, Applied, NONE), ("unit/v1/#000001", Applied, NONE)]).reorders_at(lower)
    );
    // A higher definitive rejection applied nothing: no reordering.
    assert!(
        !effects([(lower, Unsettled, NONE), ("unit/v1/#000001", Inert, NONE)]).reorders_at(lower)
    );

    // A hole: 0 open, 1 settled, 2 applied — 2 ran concurrently with 0
    // only, so 0 replays and 1 is refused.
    let hole = effects([
        ("unit/v1/#000000", Unsettled, NONE),
        ("unit/v1/#000001", Unsettled, NONE),
        ("unit/v1/#000002", Applied, Some(AT_0)),
    ]);
    assert!(!hole.reorders_at("unit/v1/#000000"));
    assert!(hole.reorders_at("unit/v1/#000001"));

    // Iterations are always ordered: the barrier drains one before the next.
    let across = effects([
        ("it0/unit/v1/#000000", Unsettled, NONE),
        ("it1/unit/v1/#000000", Applied, Some(AT_0)),
    ]);
    assert!(across.reorders_at("it0/unit/v1/#000000"));
    // Within an iteration, positions count from the iteration's ordinals.
    let within = |concurrent: &'static [PositionRange]| {
        effects([
            ("it1/unit/v1/#000001", Unsettled, NONE),
            ("it1/unit/v1/#000002", Applied, Some(concurrent)),
        ])
    };
    assert!(!within(AT_1).reorders_at("it1/unit/v1/#000001"));
    assert!(within(AT_0).reorders_at("it1/unit/v1/#000001"));
}

#[tokio::test]
async fn a_higher_fresh_prepare_waits_for_a_lower_one_that_is_then_dropped() {
    let harness = Harness::new().await;
    harness
        .ledger
        .stall_next_prepare
        .store(true, Ordering::SeqCst);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    // The lower unit reaches the ledger's prepare, which stalls before
    // writing; the higher unit is submitted meanwhile.
    let lower = tokio::spawn(handle.submit(Charge::<false> { order: 100 }));
    tokio::time::timeout(
        Duration::from_secs(5),
        harness.ledger.prepare_stalled.notified(),
    )
    .await
    .expect("the lower prepare stalled");
    let higher = tokio::spawn(handle.submit(Charge::<false> { order: 101 }));
    for _ in 0..32 {
        tokio::task::yield_now().await;
    }
    assert!(
        !higher.is_finished(),
        "the higher prepare waits for the lower one"
    );
    assert!(harness.slots().await.is_empty(), "nothing written yet");
    // The lower unit is dropped mid-prepare: its row may or may not exist.
    lower.abort();
    let _ = lower.await;
    let refused = tokio::time::timeout(Duration::from_secs(5), higher)
        .await
        .expect("the higher one decides")
        .expect("task")
        .expect_err("above an uncertain position");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert!(
        harness.slots().await.is_empty(),
        "the higher was not written"
    );
    assert_eq!(harness.desk.keys().len(), 0, "nothing sent");
    assert!(
        first
            .conclude(DRAIN)
            .await
            .is_err_and(EffectExecutionError::is_deferred),
        "the node defers"
    );

    // The next attempt runs both in order: no permanent gap.
    let recovery = harness.journal(2);
    let handle = harness.handle(&recovery);
    let (lower, higher) = tokio::join!(
        handle.submit(Charge::<false> { order: 100 }),
        handle.submit(Charge::<false> { order: 101 }),
    );
    lower.expect("lower sent");
    higher.expect("higher sent");
    assert_eq!(recovery.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 2);
    assert_eq!(harness.slots().await.len(), 2);
}

#[tokio::test(start_paused = true)]
async fn the_conclusion_admits_no_unit_and_drains_a_fixed_set() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Held]);
    let journal = harness.journal(1);
    let handle = harness.handle(&journal);
    // A unit the action did not await is mid-call when the action returns.
    let admitted = tokio::spawn(handle.submit(Charge::<false> { order: 90 }));
    tokio::time::timeout(Duration::from_secs(5), harness.desk.entered.notified())
        .await
        .expect("the call reached the provider");
    let concluding = tokio::spawn({
        let journal = journal.clone();
        async move { journal.conclude(Duration::from_mins(1)).await }
    });
    for _ in 0..16 {
        tokio::task::yield_now().await;
    }
    // A detached task submits after the result: refused unsent.
    let refused = handle
        .submit(Charge::<false> { order: 91 })
        .await
        .expect_err("after the node's result");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert_eq!(refused.detail(), "effect owner closed; unit refused");
    // The admitted unit still settles, and the verdict covers it alone.
    harness.desk.release.notify_one();
    admitted.await.expect("task").expect("applied");
    assert_eq!(concluding.await.expect("task"), Ok(()));
    assert_eq!(harness.desk.keys().len(), 1, "only the admitted call");
    assert_eq!(harness.slots().await.len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_retained_completed_handle_does_not_hold_the_barrier() {
    let harness = Harness::new().await;
    let journal = harness.stateful_journal(1);
    journal.begin_iteration(0).expect("it0");
    let handle = harness.handle(&journal);
    let mut kept = handle.submit(Charge::<false> { order: 70 });
    (&mut kept).await.expect("applied");
    // The action keeps the completed handle across the barrier.
    let started = tokio::time::Instant::now();
    journal
        .end_iteration(Duration::from_mins(1), true)
        .await
        .expect("nothing in flight");
    assert_eq!(started.elapsed(), Duration::ZERO, "no wait");
    journal
        .begin_iteration(1)
        .expect("the next iteration begins");
    drop(kept);
    assert_eq!(journal.conclude(DRAIN).await, Ok(()));
}

#[tokio::test]
async fn the_barrier_reports_whether_the_replay_reached_its_frontier() {
    let harness = Harness::new().await;
    let first = harness.stateful_journal(1);
    for iteration in 0..3 {
        first.begin_iteration(iteration).expect("begin");
        harness
            .handle(&first)
            .submit(Charge::<false> {
                order: 60 + u64::from(iteration),
            })
            .await
            .expect("applied");
        first.end_iteration(DRAIN, true).await.expect("end");
    }
    assert!(
        !first.recorded_after_open_iteration(),
        "the first attempt is always at its frontier"
    );

    let replay = harness.stateful_journal(2);
    let mut past = Vec::new();
    for iteration in 0..4 {
        replay.begin_iteration(iteration).expect("begin");
        if iteration < 3 {
            harness
                .handle(&replay)
                .submit(Charge::<false> {
                    order: 60 + u64::from(iteration),
                })
                .await
                .expect("replayed");
        }
        replay.end_iteration(DRAIN, true).await.expect("end");
        past.push(replay.recorded_after_open_iteration());
    }
    assert_eq!(
        past,
        [true, true, false, false],
        "iterations 0..=2 recorded"
    );
    assert_eq!(harness.desk.keys().len(), 3, "the replay sent nothing");
}

#[tokio::test]
async fn a_rejected_later_effect_does_not_order_an_unsent_earlier_one() {
    let harness = Harness::new().await;
    // 0 throttled (nothing applied), then 1 definitively rejected (nothing
    // applied either); the process dies.
    harness
        .desk
        .script(&[Reply::Throttled, Reply::Rejected, Reply::Applied]);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    handle
        .submit(Charge::<false> { order: 80 })
        .await
        .expect_err("throttled");
    handle
        .submit(Charge::<false> { order: 81 })
        .await
        .expect_err("rejected");

    // The recovery sends 0: the rejection after it changed nothing, so the
    // order cannot be reversed. 1 replays its recorded rejection.
    let recovery = harness.journal(2);
    let handle = harness.handle(&recovery);
    handle
        .submit(Charge::<false> { order: 80 })
        .await
        .expect("0 is sent");
    let replayed = handle
        .submit(Charge::<false> { order: 81 })
        .await
        .expect_err("the recorded rejection");
    assert_eq!(replayed.sent(), SentState::Sent, "{replayed}");
    assert_eq!(harness.desk.keys().len(), 3, "0, 1, and 0 again");
    assert_eq!(
        recovery.conclude_node(DRAIN, false).await,
        Ok(Concluded::Clean)
    );
}

#[tokio::test]
async fn a_fresh_slot_records_exactly_the_lower_units_still_open() {
    let harness = Harness::new().await;
    // 0: stable key, mid-call; 1: throttled (settled, applied nothing);
    // 2: applied while only 0 is open.
    harness.desk.script(&[
        Reply::Held,
        Reply::Throttled,
        Reply::Applied,
        Reply::Applied,
    ]);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    let open = tokio::spawn(handle.submit(Charge::<true> { order: 50 }));
    tokio::time::timeout(Duration::from_secs(5), harness.desk.entered.notified())
        .await
        .expect("the first call reached the provider");
    handle
        .submit(Charge::<false> { order: 51 })
        .await
        .expect_err("throttled");
    handle
        .submit(Charge::<false> { order: 52 })
        .await
        .expect("applied while 0 is open and 1 settled");
    let recorded: Vec<Vec<PositionRange>> = harness
        .slots()
        .await
        .iter()
        .map(|slot| {
            slot.record()
                .protocol()
                .expect("protocol")
                .concurrent_with()
                .expect("always recorded, even empty")
                .to_vec()
        })
        .collect();
    assert_eq!(
        recorded,
        [vec![], vec![at(0)], vec![at(0)]],
        "1 settled: not listed"
    );
    // 1 recorded how it failed: throttled, sending nothing.
    assert_eq!(
        harness.slots().await[1]
            .record()
            .protocol()
            .expect("protocol")
            .unsent_failure()
            .map(UnsentFailureCode::as_str),
        Some("exhausted")
    );
    // The process dies with 0 mid-call.
    open.abort();
    let _ = open.await;

    // The recovery re-sends 0 under its key (it ran alongside 2), and does
    // not send 1 (it settled unsent before 2 began: sending it now would
    // reverse them) — it fails unsent again, as the program saw.
    let recovery = harness.journal(2);
    let handle = harness.handle(&recovery);
    handle
        .submit(Charge::<true> { order: 50 })
        .await
        .expect("0 replays under its recorded key");
    let refused = handle
        .submit(Charge::<false> { order: 51 })
        .await
        .expect_err("1 is ordered before the applied 2");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert_eq!(
        refused.detail(),
        "effect failed unsent in an earlier run that moved past it; not sent again"
    );
    // The same failure the program saw: throttled, not a new `Permanent`.
    assert_eq!(
        *refused.kind(),
        nebula_resource::error::ErrorKind::Exhausted { retry_after: None }
    );
    let keys = harness.desk.keys();
    assert_eq!(keys.len(), 4, "0, 1 throttled, 2, 0 again");
    assert_eq!(keys[0], keys[3], "0 again under its recorded key");
}

/// Waits until the gateway received `calls` calls.
async fn calls_reach(desk: &Desk, calls: usize) {
    tokio::time::timeout(Duration::from_secs(30), async {
        while desk.keys().len() < calls {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the calls reached the gateway");
}

/// A hundred units awaited together, all mid-call when a later effect
/// applies, and the process dies: the later slot records all hundred as
/// concurrent — one run — so the recovery grants every one of them again
/// under its recorded key instead of reading any as settled before.
#[tokio::test]
async fn a_hundred_concurrent_units_are_all_recorded_and_all_recovered() {
    const UNITS: u64 = 100;
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Held; 100]);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    let joined: Vec<_> = (0..UNITS)
        .map(|order| tokio::spawn(handle.submit(Charge::<true> { order })))
        .collect();
    calls_reach(&harness.desk, 100).await;
    handle
        .submit(Charge::<false> { order: UNITS })
        .await
        .expect("applied while all hundred are open");
    let slots = harness.slots().await;
    assert_eq!(slots.len(), 101);
    assert_eq!(
        slots[100]
            .record()
            .protocol()
            .expect("protocol")
            .concurrent_with(),
        Some(&[PositionRange::new(0, 99).expect("run")][..]),
        "every open unit, as one run"
    );
    // The process dies with all hundred mid-call.
    for unit in joined {
        unit.abort();
        let _ = unit.await;
    }
    for ordinal in 0..100 {
        first.finish_occurrence(&format!("unit/v1/#{ordinal:06}"));
    }

    // Every lower unit ran alongside the applied one: each is granted again
    // under its recorded key, none refused or unknown.
    let recovery = harness.journal(2);
    let handle = harness.handle(&recovery);
    let keys = harness.desk.keys();
    for order in 0..UNITS {
        handle
            .submit(Charge::<true> { order })
            .await
            .expect("granted again: it ran concurrently");
    }
    handle
        .submit(Charge::<false> { order: UNITS })
        .await
        .expect("replayed");
    assert_eq!(recovery.conclude(DRAIN).await, Ok(()));
    let resent = harness.desk.keys();
    assert_eq!(
        resent.len(),
        201,
        "each lower one once more, the higher none"
    );
    let mut first_keys: Vec<_> = keys[..100].to_vec();
    let mut again: Vec<_> = resent[101..].to_vec();
    first_keys.sort();
    again.sort();
    assert_eq!(first_keys, again, "under their recorded keys");
}

/// Open units interleaved with settled ones beyond what a slot records: the
/// fresh effect that would need more runs is refused unsent — never
/// recorded with a truncated list — and the node fails with the limit.
#[tokio::test]
async fn too_interleaved_concurrent_units_refuse_the_fresh_effect_unsent() {
    let limit = u64::try_from(OperationProtocolRecord::MAX_CONCURRENT_RANGES).expect("fits");
    let harness = Harness::new().await;
    let journal = harness.journal(1);
    let handle = harness.handle(&journal);
    let mut held = Vec::new();
    let mut calls = 0;
    for run in 0..=limit {
        // An open unit, then a settled one: each settled one sees one more
        // run of open positions below it.
        harness.desk.script(&[Reply::Held]);
        held.push(tokio::spawn(
            handle.submit(Charge::<true> { order: run * 2 }),
        ));
        calls += 1;
        calls_reach(&harness.desk, calls).await;
        let settled = handle.submit(Charge::<false> { order: run * 2 + 1 });
        if run < limit {
            settled.await.expect("its open units fit");
            calls += 1;
        } else {
            let refused = settled.await.expect_err("one run too many");
            assert_eq!(refused.sent(), SentState::NotSent);
            assert_eq!(
                *refused.kind(),
                nebula_resource::error::ErrorKind::Permanent
            );
            assert!(
                refused
                    .detail()
                    .contains("too many interleaved concurrent effects"),
                "{}",
                refused.detail()
            );
        }
    }
    assert_eq!(
        harness.desk.keys().len(),
        calls,
        "the refused one sent nothing"
    );
    let refused_label = format!("unit/v1/#{:06}", limit * 2 + 1);
    assert!(
        harness
            .slots()
            .await
            .iter()
            .all(|slot| slot.occurrence() != refused_label),
        "nothing written"
    );
    harness.desk.release.notify_waiters();
    for unit in held {
        unit.await.expect("task").expect("released");
    }
    assert_eq!(
        journal.conclude(DRAIN).await,
        Err(EffectExecutionError::JournalConcurrencyLimit { limit: 64 })
    );
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_REFUSALS_TOTAL,
            &[
                ("step", effect_journal_step::PREPARE),
                ("refusal", JournalRefusal::ConcurrencyLimit.as_str()),
            ]
        ),
        1
    );
}

#[tokio::test]
async fn the_node_slot_cap_refuses_further_prepares_and_sends_nothing() {
    let harness = Harness::new().await;
    let journal = harness.journal(1);
    // As if the node attempt had already prepared all but one.
    journal.state().reserved = MAX_NODE_SLOTS - 1;
    harness
        .handle(&journal)
        .submit(Charge::<false> { order: 43 })
        .await
        .expect("the last slot under the cap");
    let refused = harness
        .handle(&journal)
        .submit(Charge::<false> { order: 44 })
        .await
        .expect_err("over the cap");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert_eq!(
        refused.detail(),
        "effect journal slot cap reached; unit refused"
    );
    assert_eq!(harness.desk.keys().len(), 1, "nothing sent past the cap");
    assert_eq!(harness.slots().await.len(), 1, "nothing prepared past it");
    assert_eq!(
        journal.conclude(DRAIN).await,
        Err(EffectExecutionError::JournalSlotCapExceeded {
            cap: MAX_NODE_SLOTS
        })
    );
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_REFUSALS_TOTAL,
            &[("step", "prepare"), ("refusal", "slot_cap_exceeded")]
        ),
        1
    );
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_VERDICTS_TOTAL,
            &[("code", "slot_cap_exceeded")]
        ),
        1
    );
}

#[test]
fn a_gap_is_an_unrecorded_position_below_a_recorded_one() {
    let prior = PriorOccurrences::new(["unit/v1/#000001", "unit/v1/#000002"]);
    // Position 0 was left empty while 1 and 2 were recorded.
    assert!(prior.leaves_gap_at("unit/v1/#000000"));
    assert!(prior.refuses("unit/v1/#000000"));
    // Recorded positions are revisited; later ones extend the program.
    assert!(!prior.refuses("unit/v1/#000001"));
    assert!(!prior.refuses("unit/v1/#000003"));
    // A stateful iteration's label is no gap of the flat sequence, but a
    // node whose earlier attempt recorded flat labels changed kind.
    assert!(!prior.leaves_gap_at("it1/unit/v1/#000000"));
    assert!(prior.mixes_family_at("it1/unit/v1/#000000"));
    assert!(prior.refuses("it1/unit/v1/#000000"));
    assert!(!PriorOccurrences::default().refuses("unit/v1/#000000"));
    assert!(!PriorOccurrences::default().refuses("it0/unit/v1/#000000"));
}

#[test]
fn iteration_positions_order_by_iteration_then_ordinal() {
    let prior = PriorOccurrences::new([
        "it0/unit/v1/#000000",
        "it0/unit/v1/#000001",
        "it1/unit/v1/#000000",
    ]);
    // Recorded positions replay; a later ordinal of the last iteration and
    // any later iteration extend the program.
    for fresh in [
        "it0/unit/v1/#000001",
        "it1/unit/v1/#000001",
        "it2/unit/v1/#000000",
        "it9999/unit/v1/#000000",
    ] {
        assert!(!prior.refuses(fresh), "{fresh}");
    }
    // A new effect in an iteration the earlier attempt finished — at its
    // end too — sits below the recorded iteration 1: a gap.
    assert!(prior.leaves_gap_at("it0/unit/v1/#000002"));
    // Flat labels after recorded iterations: the action changed kind.
    assert!(prior.mixes_family_at("unit/v1/#000000"));
    assert!(prior.refuses("unit/v1/#000005"));

    // An iteration the earlier attempt reached without recording anything
    // in it is a gap below a later recorded iteration.
    let skipped = PriorOccurrences::new(["it0/unit/v1/#000000", "it2/unit/v1/#000000"]);
    assert!(skipped.leaves_gap_at("it1/unit/v1/#000000"));
    assert!(!skipped.refuses("it2/unit/v1/#000001"));
}

#[test]
fn positions_parse_only_labels_the_journal_builds() {
    assert_eq!(
        Position::parse("unit/v1/#000007"),
        Some(Position {
            family: Family::Flat,
            iteration: 0,
            ordinal: 7
        })
    );
    assert_eq!(
        Position::parse("it12/unit/v1/#1234567"),
        Some(Position {
            family: Family::Iterated,
            iteration: 12,
            ordinal: 1_234_567
        })
    );
    assert_eq!(
        Position::parse("it0/unit/v1/#000000").map(Position::order),
        Some((0, 0))
    );
    for label in [
        // Leading zeros in the iteration, or an empty or signed one.
        "it01/unit/v1/#000000",
        "it00/unit/v1/#000000",
        "it/unit/v1/#000000",
        "it+1/unit/v1/#000000",
        // Past the last iteration.
        "it10000/unit/v1/#000000",
        // A short, unpadded or over-padded ordinal.
        "unit/v1/#7",
        "unit/v1/#0000007",
        "unit/v1/#",
        "unit/v1/#-00001",
        // Another namespace or version.
        "unit/v2/#000000",
        "it1/unit/v1/x/#000000",
        "op/v1/#000000",
    ] {
        assert_eq!(Position::parse(label), None, "{label}");
    }
    // A label with no position is never a gap, whatever is recorded.
    let prior = PriorOccurrences::new(["it01/unit/v1/#000005"]);
    assert!(!prior.refuses("it1/unit/v1/#000000"));
}

#[test]
fn turn_labels_parse_strictly_as_their_own_family() {
    assert_eq!(
        Position::parse("turn12/unit/v1/#000003"),
        Some(Position {
            family: Family::Turned,
            iteration: 12,
            ordinal: 3
        })
    );
    assert_eq!(
        Position::parse("turn9999/unit/v1/#000000").map(Position::order),
        Some((9_999, 0))
    );
    for label in [
        "turn01/unit/v1/#000000",
        "turn00/unit/v1/#000000",
        "turn/unit/v1/#000000",
        "turn+1/unit/v1/#000000",
        "turn10000/unit/v1/#000000",
        "turn1/unit/v1/#7",
        "turn1/unit/v2/#000000",
        "tur1/unit/v1/#000000",
        "itturn1/unit/v1/#000000",
    ] {
        assert_eq!(Position::parse(label), None, "{label}");
    }
    assert_eq!(
        occurrence_label(Family::Turned, Some(7), 2),
        "turn7/unit/v1/#000002"
    );
    assert_eq!(
        occurrence_label(Family::Iterated, Some(7), 2),
        "it7/unit/v1/#000002"
    );
    assert_eq!(occurrence_label(Family::Turned, None, 2), "unit/v1/#000002");

    // Flat, iterated and turned labels are mutually exclusive for a node:
    // any of them after another family is a changed action kind.
    for (recorded, fresh) in [
        ("turn0/unit/v1/#000000", "it0/unit/v1/#000000"),
        ("turn0/unit/v1/#000000", "unit/v1/#000000"),
        ("it0/unit/v1/#000000", "turn0/unit/v1/#000000"),
        ("unit/v1/#000000", "turn0/unit/v1/#000000"),
    ] {
        let prior = PriorOccurrences::new([recorded]);
        assert!(prior.mixes_family_at(fresh), "{recorded} then {fresh}");
        assert!(prior.refuses(fresh), "{recorded} then {fresh}");
    }
    // Within the turned family, positions order by turn then ordinal.
    let turns = PriorOccurrences::new(["turn0/unit/v1/#000000", "turn2/unit/v1/#000000"]);
    assert!(turns.leaves_gap_at("turn1/unit/v1/#000000"));
    assert!(!turns.refuses("turn2/unit/v1/#000001"));
    assert!(!turns.refuses("turn3/unit/v1/#000000"));
}

#[tokio::test]
async fn an_agent_journal_labels_its_turns_and_admits_units_only_in_one() {
    let harness = Harness::new().await;
    let journal = NodeEffectJournal::new(harness.authority(
        1,
        Arc::new(nebula_core::accessor::SystemClock),
        JournalShape::Turned,
    ));
    // No turn open: a submission belongs to none.
    assert_eq!(
        journal.admit().map(|_| ()),
        Err(JournalRefusal::BetweenRuns)
    );
    let journal = NodeEffectJournal::new(harness.authority(
        2,
        Arc::new(nebula_core::accessor::SystemClock),
        JournalShape::Turned,
    ));
    journal.begin_iteration(0).expect("turn 0");
    assert_eq!(journal.next_occurrence(), "turn0/unit/v1/#000000");
    assert_eq!(journal.next_occurrence(), "turn0/unit/v1/#000001");
    journal
        .end_iteration(DRAIN, true)
        .await
        .expect("nothing in flight");
    journal.begin_iteration(1).expect("turn 1");
    assert_eq!(
        journal.next_occurrence(),
        "turn1/unit/v1/#000000",
        "the ordinal restarts per turn"
    );
    journal
        .end_iteration(DRAIN, true)
        .await
        .expect("nothing in flight");
    // Abandoned (a timeout or a cancellation): nothing is admitted after.
    journal.abandon_iteration();
    assert_eq!(journal.admit().map(|_| ()), Err(JournalRefusal::Closed));
    assert_eq!(journal.conclude(DRAIN).await, Ok(()));
}

#[tokio::test]
async fn an_effect_prepared_into_an_earlier_attempts_gap_is_a_mismatch() {
    let harness = Harness::new().await;
    // The first attempt reached the first position without recording it (a
    // unit whose prepare never became durable) and settled the charge at
    // the second.
    let first = harness.journal(1);
    assert_eq!(first.next_ordinal(), 0);
    harness
        .handle(&first)
        .submit(Charge::<false> { order: 3 })
        .await
        .expect("applied");
    assert_eq!(first.conclude(DRAIN).await, Ok(()));
    let before = harness.slots().await;
    assert_eq!(before.len(), 1);
    assert_eq!(before[0].occurrence(), OCCURRENCE_1);

    // The retry reaches the charge first: it would land on the empty first
    // position under a new provider key. Refused, nothing sent.
    let retry = harness.journal(2);
    let refused = harness
        .handle(&retry)
        .submit(Charge::<false> { order: 3 })
        .await
        .expect_err("the gap is refused");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert_eq!(refused.detail(), "effect occurrence mismatch");
    assert_eq!(
        retry.conclude(DRAIN).await,
        Err(EffectExecutionError::OccurrenceMismatch)
    );
    assert_eq!(harness.desk.keys().len(), 1, "one provider call");
    assert_eq!(harness.slots().await, before, "nothing prepared");
}

/// A unit's deadline passes before its first poll: it takes a position and
/// gives up before reaching the journal.
const PAST_THE_UNIT_DEADLINE: Duration = Duration::from_mins(6);

#[tokio::test(start_paused = true)]
async fn a_position_a_unit_gave_up_defers_the_next_fresh_effect() {
    let harness = Harness::new().await;
    let first = harness.journal(1);
    harness
        .handle(&first)
        .submit(Charge::<false> { order: 5 })
        .await
        .expect("applied at the first position");

    // The retry's first unit is polled only past its deadline: it takes the
    // first position and gives it up before reaching the journal.
    let retry = harness.journal(2);
    let late = harness.handle(&retry).submit(Charge::<false> { order: 6 });
    tokio::time::advance(PAST_THE_UNIT_DEADLINE).await;
    let gave_up = late.await.expect_err("past its deadline");
    assert_eq!(gave_up.sent(), SentState::NotSent, "{gave_up}");
    // The recorded charge, now at the second position, would be a fresh
    // slot under another provider key: refused, nothing sent. A position
    // given up is no proof the program took another path, so the node
    // defers rather than halting: the next attempt can meet it again.
    let refused = harness
        .handle(&retry)
        .submit(Charge::<false> { order: 5 })
        .await
        .expect_err("above a position this attempt gave up");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert_eq!(refused.detail(), "effect owner unavailable; unit refused");
    assert_eq!(harness.desk.keys().len(), 1, "sent once");
    assert_eq!(harness.slots().await.len(), 1, "nothing prepared");
    assert!(
        retry
            .conclude(DRAIN)
            .await
            .is_err_and(EffectExecutionError::is_deferred)
    );

    // The next attempt meets the recorded charge first and replays it.
    let next = harness.journal(2);
    assert_eq!(
        harness
            .handle(&next)
            .submit(Charge::<false> { order: 5 })
            .await
            .expect("replayed"),
        1
    );
    assert_eq!(next.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 1, "still sent once");
}

#[tokio::test(start_paused = true)]
async fn an_iteration_that_skips_a_recorded_effect_refuses_the_next_fresh_one() {
    let harness = Harness::new().await;
    let first = harness.stateful_journal(1);
    first.begin_iteration(0).expect("it0");
    harness
        .handle(&first)
        .submit(Charge::<false> { order: 7 })
        .await
        .expect("applied at it0/#0");
    first.end_iteration(DRAIN, true).await.expect("it0 ends");

    // Within an iteration: the retry's first unit of iteration 0 gives up
    // before reaching the journal; the charge lands one position higher.
    let retry = harness.stateful_journal(2);
    retry.begin_iteration(0).expect("it0");
    let late = harness.handle(&retry).submit(Charge::<false> { order: 8 });
    tokio::time::advance(PAST_THE_UNIT_DEADLINE).await;
    late.await.expect_err("past its deadline");
    let refused = harness
        .handle(&retry)
        .submit(Charge::<false> { order: 7 })
        .await
        .expect_err("above it0/#0, which the retry gave up");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert!(
        retry
            .end_iteration(DRAIN, true)
            .await
            .is_err_and(EffectExecutionError::is_deferred),
        "a position given up defers the node"
    );
    assert_eq!(harness.desk.keys().len(), 1, "sent once");
    assert_eq!(harness.slots().await.len(), 1);
}

#[tokio::test]
async fn an_iteration_that_passes_a_recorded_effect_by_stops_at_its_barrier() {
    let harness = Harness::new().await;
    // The first attempt: iteration 0 sends nothing, iteration 1 a charge;
    // then the process dies.
    let first = harness.stateful_journal(1);
    first.begin_iteration(0).expect("it0");
    first.end_iteration(DRAIN, true).await.expect("it0 ends");
    first.begin_iteration(1).expect("it1");
    harness
        .handle(&first)
        .submit(Charge::<false> { order: 9 })
        .await
        .expect("applied at it1/#0");

    // The replay's iteration 1 sends nothing (an unjournaled read changed):
    // its barrier stops the loop before iteration 2 could send the charge
    // again at a fresh position.
    let replay = harness.stateful_journal(2);
    replay.begin_iteration(0).expect("it0");
    replay.end_iteration(DRAIN, true).await.expect("it0 ends");
    replay.begin_iteration(1).expect("it1");
    assert_eq!(
        replay.end_iteration(DRAIN, true).await,
        Err(EffectExecutionError::OccurrenceMismatch)
    );
    assert_eq!(
        replay.begin_iteration(2),
        Err(EffectExecutionError::OccurrenceMismatch),
        "no next iteration"
    );

    // Even an owner that went on would refuse the fresh slot above it.
    let stubborn = harness.stateful_journal(3);
    stubborn.begin_iteration(2).expect("it2");
    let refused = harness
        .handle(&stubborn)
        .submit(Charge::<false> { order: 9 })
        .await
        .expect_err("above it1/#0, which this attempt never met");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert_eq!(harness.desk.keys().len(), 1, "sent once");

    // A failing iteration keeps its own failure: its conclusion decides.
    let failing = harness.stateful_journal(4);
    failing.begin_iteration(0).expect("it0");
    failing.end_iteration(DRAIN, true).await.expect("it0 ends");
    failing.begin_iteration(1).expect("it1");
    assert_eq!(failing.end_iteration(DRAIN, false).await, Ok(()));
    assert_eq!(
        failing.conclude_node(DRAIN, false).await,
        Ok(Concluded::SkippedRecordedEffect)
    );
}

#[tokio::test]
async fn a_fresh_effect_waits_for_a_lower_recorded_position_still_preparing() {
    let harness = Harness::new().await;
    let first = harness.journal(1);
    harness
        .handle(&first)
        .submit(Charge::<false> { order: 10 })
        .await
        .expect("applied at the first position");

    // The retry submits the recorded charge and a new one together; the
    // new one's fresh slot is decided only once the recorded one met its
    // position.
    harness.ledger.yield_prepares.store(true, Ordering::SeqCst);
    let retry = harness.journal(2);
    let handle = harness.handle(&retry);
    let (replayed, fresh) = tokio::join!(
        handle.submit(Charge::<false> { order: 10 }),
        handle.submit(Charge::<false> { order: 11 }),
    );
    assert_eq!(replayed.expect("replayed"), 1, "the recorded receipt");
    assert_eq!(fresh.expect("a new effect"), 2);
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 2, "each charge sent once");
}

#[tokio::test(start_paused = true)]
async fn a_final_read_that_never_answers_defers_the_verdict() {
    let harness = Harness::new().await;
    harness.ledger.hang_reads.store(true, Ordering::SeqCst);
    let journal = harness.journal(1);
    let started = tokio::time::Instant::now();
    let verdict = journal.conclude(Duration::from_mins(1)).await;
    assert_eq!(
        verdict,
        Err(EffectExecutionError::Ledger(
            OperationLedgerError::Unavailable
        ))
    );
    assert!(verdict.is_err_and(EffectExecutionError::is_deferred));
    assert_eq!(
        started.elapsed(),
        Duration::from_mins(1),
        "bounded by the verdict budget"
    );

    // A spent budget still gives the read its floor.
    let spent = harness.journal(1);
    let started = tokio::time::Instant::now();
    assert!(spent.conclude(Duration::ZERO).await.is_err());
    assert_eq!(started.elapsed(), FINAL_READ_FLOOR);
}

#[tokio::test]
async fn a_succeeding_node_that_skips_a_recorded_effect_is_a_mismatch() {
    let harness = Harness::new().await;
    let first = harness.journal(1);
    harness
        .handle(&first)
        .submit(Charge::<false> { order: 5 })
        .await
        .expect("applied");
    assert_eq!(first.conclude(DRAIN).await, Ok(()));

    // The recovered dispatch takes another path and submits nothing: its
    // success would stand for an attempt that never applied the charge.
    let skipping = harness.journal(2);
    assert_eq!(
        skipping.conclude(DRAIN).await,
        Err(EffectExecutionError::OccurrenceMismatch)
    );
    // A failing dispatch keeps its own failure, flagged: a retry may meet
    // the effect again, no error strategy may route past it.
    let failing = harness.journal(2);
    assert_eq!(
        failing.conclude_node(DRAIN, false).await,
        Ok(Concluded::SkippedRecordedEffect)
    );
    // A dispatch that meets the effect again replays it and succeeds, and
    // a failing one that met it is clean.
    let replaying = harness.journal(2);
    harness
        .handle(&replaying)
        .submit(Charge::<false> { order: 5 })
        .await
        .expect("replayed");
    assert_eq!(replaying.conclude(DRAIN).await, Ok(()));
    let failing_after_replay = harness.journal(2);
    harness
        .handle(&failing_after_replay)
        .submit(Charge::<false> { order: 5 })
        .await
        .expect("replayed");
    assert_eq!(
        failing_after_replay.conclude_node(DRAIN, false).await,
        Ok(Concluded::Clean)
    );
    assert_eq!(harness.desk.keys().len(), 1, "one provider call");
}

#[tokio::test]
async fn a_read_only_in_practice_node_writes_nothing_and_reads_once() {
    let harness = Harness::new().await;
    let journal = harness.journal(1);
    let handle = harness.handle(&journal);
    assert_eq!(handle.submit(Balance).await.expect("a read runs"), 0);
    assert_eq!(handle.submit(Balance).await.expect("a read runs"), 0);
    assert_eq!(harness.ledger.calls(), 0, "reads are never prepared");
    // Concluding reads the node's occurrences once, and nothing else: an
    // earlier dispatch may have left a slot even at generation 1.
    assert_eq!(journal.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.ledger.calls(), 1);
    assert!(harness.slots().await.is_empty());
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_VERDICTS_TOTAL,
            &[("code", effect_journal_verdict::OK)]
        ),
        1
    );

    let retry = harness.journal(2);
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.ledger.calls(), 2);
}

#[tokio::test]
async fn a_crash_before_the_attempt_was_recorded_still_fails_the_node() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Hang]);
    let crashed = harness.journal(1);
    let in_flight = tokio::spawn(
        harness
            .handle(&crashed)
            .submit(Charge::<false> { order: 14 }),
    );
    tokio::time::timeout(Duration::from_secs(30), harness.desk.entered.notified())
        .await
        .expect("the call reached the gateway");

    // The process died before the dispatch was recorded: the recovered node
    // runs again as attempt 1, and this time submits no effect.
    let recovered = harness.journal(1);
    assert_eq!(
        harness
            .handle(&recovered)
            .submit(Balance)
            .await
            .expect("a read runs"),
        1
    );
    let verdict = recovered.conclude(DRAIN).await;
    assert!(
        matches!(
            verdict,
            Err(EffectExecutionError::JournalOutcomeUnknown { unresolved: 1, .. })
        ),
        "{verdict:?}"
    );
    in_flight.abort();
}

#[tokio::test]
async fn a_settled_write_replays_on_the_next_attempt_without_a_provider_call() {
    let harness = Harness::new().await;
    let first = harness.journal(1);
    let receipt = harness
        .handle(&first)
        .submit(Charge::<false> { order: 7 })
        .await
        .expect("applied");
    assert_eq!(first.conclude(DRAIN).await, Ok(()));
    let keys = harness.desk.keys();
    assert_eq!(keys.len(), 1);
    let slots = harness.slots().await;
    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].occurrence(), OCCURRENCE_0);
    assert_eq!(
        keys[0].as_deref(),
        slots[0]
            .record()
            .operation()
            .provider_key()
            .as_ref()
            .map(ProviderIdempotencyKey::as_str),
        "the provider received the recorded key"
    );

    // A node retry reuses the occurrence: the recorded output replays.
    let retry = harness.journal(2);
    let replayed = harness
        .handle(&retry)
        .submit(Charge::<false> { order: 7 })
        .await
        .expect("replayed");
    assert_eq!(replayed, receipt);
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 1, "no second provider call");
    assert_eq!(harness.slots().await, slots, "a replay writes nothing");
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_PREPARES_TOTAL,
            &[("phase", effect_journal_prepare_phase::REPLAY)]
        ),
        1
    );
}

#[tokio::test]
async fn a_settled_write_replays_after_a_restart_under_a_rebuilt_identical_config() {
    use nebula_resource::ResourceConfig as _;

    // The contract binds the configuration fingerprint; recorded contracts
    // are durable, so the fingerprint is a pure function of the content,
    // pinned here (first eight bytes of
    // `SHA-256("nebula-resource/config-fingerprint/v1\0{\"endpoint\":1}")`):
    // a toolchain or platform change cannot move it.
    assert_eq!(
        GatewayConfig { endpoint: 1 }.fingerprint(),
        0xcfe8_eb13_93ce_3cd3
    );

    let harness = Harness::new().await;
    let first = harness.journal(1);
    let receipt = harness
        .handle(&first)
        .submit(Charge::<false> { order: 9 })
        .await
        .expect("applied");
    assert_eq!(first.conclude(DRAIN).await, Ok(()));
    let slots = harness.slots().await;
    assert_eq!(slots.len(), 1);

    // A restarted process rebuilds the manager and the configuration from
    // scratch: the recorded slot still resolves, and replays.
    let rebuilt = harness.rebuilt_manager();
    let retry = harness.journal(2);
    let replayed = Harness::handle_on(&rebuilt, &retry)
        .submit(Charge::<false> { order: 9 })
        .await
        .expect("replayed under the rebuilt configuration");
    assert_eq!(replayed, receipt);
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 1, "no second provider call");
    assert_eq!(harness.slots().await, slots, "a replay writes nothing");
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_REFUSALS_TOTAL,
            &[
                ("step", effect_journal_step::PREPARE),
                ("refusal", JournalRefusal::Mismatch.as_str()),
            ]
        ),
        0,
        "no contract mismatch"
    );
}

#[tokio::test]
async fn an_interrupted_write_is_unknown_even_when_the_action_swallows_it() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Lost]);
    let first = harness.journal(1);
    let error = harness
        .handle(&first)
        .submit(Charge::<false> { order: 8 })
        .await
        .expect_err("the answer was lost");
    assert_eq!(error.sent(), SentState::MaybeSent);
    // The action swallowed the error; the journal still fails the node.
    let verdict = first.conclude(DRAIN).await;
    assert!(
        matches!(
            verdict,
            Err(EffectExecutionError::JournalOutcomeUnknown { unresolved: 1, .. })
        ),
        "{verdict:?}"
    );

    // A later attempt is refused without a provider call.
    let retry = harness.journal(2);
    let refused = harness
        .handle(&retry)
        .submit(Charge::<false> { order: 8 })
        .await
        .expect_err("unknown");
    assert_eq!(*refused.kind(), nebula_resource::ErrorKind::OutcomeUnknown);
    assert!(retry.conclude(DRAIN).await.is_err());
    assert_eq!(harness.desk.keys().len(), 1);
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_PREPARES_TOTAL,
            &[("phase", effect_journal_prepare_phase::UNKNOWN)]
        ),
        1
    );
}

#[tokio::test]
async fn idempotent_crash_residue_is_granted_again_under_the_same_key() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Hang]);
    let crashed = harness.journal(1);
    let handle = harness.handle(&crashed);
    let in_flight = tokio::spawn(handle.submit(Charge::<true> { order: 9 }));
    tokio::time::timeout(Duration::from_secs(30), harness.desk.entered.notified())
        .await
        .expect("the call reached the gateway");
    let slots = harness.slots().await;
    assert_eq!(
        slots[0].record().protocol().expect("protocol").phase(),
        EffectPhase::InvocationOutstanding
    );

    // The next attempt finds the outstanding call and records it as an
    // ambiguous crossing: the stable-key effect is sent again, same key.
    let retry = harness.journal(2);
    let receipt = harness
        .handle(&retry)
        .submit(Charge::<true> { order: 9 })
        .await
        .expect("granted again within the window");
    assert_eq!(receipt, 2);
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    let keys = harness.desk.keys();
    assert_eq!(keys.len(), 2);
    assert!(keys[0].is_some());
    assert_eq!(keys[0], keys[1], "one provider key across the crash");
    in_flight.abort();
}

#[tokio::test]
async fn a_caught_mismatch_and_a_new_submission_still_conclude_the_mismatch() {
    let harness = Harness::new().await;
    let first = harness.journal(1);
    harness
        .handle(&first)
        .submit(Charge::<false> { order: 40 })
        .await
        .expect("applied");
    assert_eq!(first.conclude(DRAIN).await, Ok(()));

    // The retry diverges at #0, catches the mismatch and submits again.
    let retry = harness.journal(2);
    let handle = harness.handle(&retry);
    let mismatch = handle
        .submit(Charge::<false> { order: 41 })
        .await
        .expect_err("another request under the recorded occurrence");
    assert_eq!(mismatch.detail(), "effect occurrence mismatch");
    // #0 was refused, not abandoned: the next one is not deferred as if a
    // unit had given up below it — it is a mismatch too (#0 stays unmet).
    let again = handle
        .submit(Charge::<false> { order: 42 })
        .await
        .expect_err("above a recorded effect this attempt never met");
    assert_eq!(again.detail(), "effect occurrence mismatch");
    assert_eq!(again.sent(), SentState::NotSent);
    let verdict = retry.conclude(DRAIN).await;
    assert_eq!(verdict, Err(EffectExecutionError::OccurrenceMismatch));
    assert!(verdict.is_err_and(EffectExecutionError::halts_execution));
    assert_eq!(harness.desk.keys().len(), 1, "nothing sent again");
}

/// A recorded effect after a replay mismatch is never granted: the first
/// attempt applied #0 and died with #1 only prepared; a retry diverges at
/// #0, catches the mismatch and submits #1 exactly as recorded — the
/// grantable prepared slot is refused, so the divergent run sends nothing.
#[tokio::test]
async fn no_recorded_effect_is_granted_after_a_replay_mismatch() {
    let harness = Harness::new().await;
    let first = harness.journal(1);
    harness
        .handle(&first)
        .submit(Charge::<false> { order: 46 })
        .await
        .expect("applied");
    harness
        .ledger
        .lose_next_prepare_answer
        .store(true, Ordering::SeqCst);
    let crashed = tokio::spawn(harness.handle(&first).submit(Charge::<false> { order: 47 }));
    tokio::time::timeout(Duration::from_secs(30), async {
        while harness.slots().await.len() < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the second prepare committed");
    crashed.abort();
    let _ = crashed.await;
    assert_eq!(harness.desk.keys().len(), 1);

    let retry = harness.journal(2);
    let handle = harness.handle(&retry);
    let mismatch = handle
        .submit(Charge::<false> { order: 48 })
        .await
        .expect_err("another request under #0");
    assert_eq!(mismatch.detail(), "effect occurrence mismatch");
    let refused = handle
        .submit(Charge::<false> { order: 47 })
        .await
        .expect_err("#1 as recorded, after the halting mismatch");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    let verdict = retry.conclude(DRAIN).await;
    assert_eq!(verdict, Err(EffectExecutionError::OccurrenceMismatch));
    assert_eq!(
        harness.desk.keys().len(),
        1,
        "the prepared #1 was never sent"
    );
}

/// A halting verdict another unit records while a grant is in flight stops
/// that grant too: the committed call is recorded not crossed and refused,
/// so nothing reaches the provider after the node is known divergent.
#[tokio::test]
async fn a_halting_verdict_during_a_grant_withholds_its_call() {
    let harness = Harness::new().await;
    harness.ledger.hold_next_grant.store(true, Ordering::SeqCst);
    let journal = harness.journal(1);
    let held = harness.ledger.grant_held.notified();
    let unit = tokio::spawn(
        harness
            .handle(&journal)
            .submit(Charge::<false> { order: 49 }),
    );
    held.await;
    // Another unit's divergence lands while this grant's answer is held.
    journal.note_failure(EffectExecutionError::OccurrenceMismatch);
    harness.ledger.release_grant.notify_one();
    let refused = unit
        .await
        .expect("unit task")
        .expect_err("granted after the halting verdict");
    assert_eq!(refused.sent(), SentState::NotSent, "{refused}");
    assert!(
        harness.desk.keys().is_empty(),
        "nothing reached the provider"
    );
    let slots = harness.slots().await;
    assert_eq!(slots.len(), 1);
    assert_eq!(Harness::phase(&slots[0]), EffectPhase::BeforeBoundary);
    assert_eq!(
        journal.conclude(DRAIN).await,
        Err(EffectExecutionError::OccurrenceMismatch)
    );
}

/// A terminal failure noted first is never displaced by a later deferral;
/// a halting one displaces any other.
#[tokio::test]
async fn a_later_deferral_never_displaces_a_terminal_verdict() {
    let harness = Harness::new().await;
    let journal = harness.capped_journal(1, 1);
    let handle = harness.handle(&journal);
    handle
        .submit(Charge::<false> { order: 43 })
        .await
        .expect("under the cap");
    handle
        .submit(Charge::<false> { order: 44 })
        .await
        .expect_err("over the cap");
    // A later step that only defers (a lost lease, an unanswered ledger).
    journal.note_failure(EffectExecutionError::Ledger(
        OperationLedgerError::ExecutionLeaseRejected,
    ));
    assert_eq!(
        journal.conclude(DRAIN).await,
        Err(EffectExecutionError::JournalSlotCapExceeded { cap: 1 })
    );

    // A halting failure displaces a deferral and a terminal one alike.
    let other = harness.journal(2);
    other.note_failure(EffectExecutionError::Ledger(
        OperationLedgerError::Unavailable,
    ));
    other.note_failure(EffectExecutionError::JournalSlotCapExceeded { cap: 1 });
    other.note_failure(EffectExecutionError::OccurrenceMismatch);
    other.note_failure(EffectExecutionError::Ledger(
        OperationLedgerError::AcknowledgementUnknown,
    ));
    assert_eq!(
        other.state().failure,
        Some(EffectExecutionError::OccurrenceMismatch)
    );
}

/// An earlier attempt durably prepared an effect and died before any call.
/// A later attempt about to succeed without meeting it dropped an effect
/// the program intended: a mismatch. One failing before it stopped short
/// of it and keeps its own failure.
#[tokio::test]
async fn a_succeeding_node_that_skips_a_prepared_only_effect_is_a_mismatch() {
    let harness = Harness::new().await;
    harness
        .ledger
        .lose_next_prepare_answer
        .store(true, Ordering::SeqCst);
    let first = harness.journal(1);
    let crashed = tokio::spawn(harness.handle(&first).submit(Charge::<false> { order: 45 }));
    tokio::time::timeout(Duration::from_secs(30), async {
        while harness.slots().await.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the prepare committed");
    crashed.abort();
    let _ = crashed.await;
    assert_eq!(
        Harness::phase(&harness.slots().await[0]),
        EffectPhase::Prepared
    );

    let failing = harness.journal(2);
    assert_eq!(
        failing.conclude_node(DRAIN, false).await,
        Ok(Concluded::Clean),
        "stopped short of it: its own failure stands"
    );
    let succeeding = harness.journal(3);
    assert_eq!(
        succeeding.conclude_node(DRAIN, true).await,
        Err(EffectExecutionError::OccurrenceMismatch)
    );
    assert!(harness.desk.keys().is_empty(), "nothing sent");
}

/// A throttled unit whose classification the ledger does not take fails
/// closed: the action catches the throttle and submits a later effect,
/// which is not prepared or sent, and the node defers. The retry meets the
/// slot again, records the classification and completes the same branch; a
/// recovery past the later applied effect then replays the throttle.
#[tokio::test]
async fn an_unrecorded_unsent_failure_holds_later_effects_and_defers() {
    let harness = Harness::new().await;
    // The program: charge 60; throttled → charge 61.
    let program = async |journal: &NodeEffectJournal| {
        let handle = harness.handle(journal);
        let first = handle.submit(Charge::<false> { order: 60 }).await;
        let throttled = first.as_ref().err().map(|error| error.kind().clone());
        let then = handle.submit(Charge::<false> { order: 61 }).await;
        (throttled, then)
    };

    harness.desk.script(&[Reply::Throttled]);
    harness
        .ledger
        .fail_next_unsent_failure
        .store(true, Ordering::SeqCst);
    let first = harness.journal(1);
    let (throttled, then) = program(&first).await;
    assert_eq!(
        throttled,
        Some(nebula_resource::error::ErrorKind::Exhausted { retry_after: None })
    );
    let held = then.expect_err("held above the unrecorded position");
    assert_eq!(held.sent(), SentState::NotSent);
    assert_eq!(harness.desk.keys().len(), 1, "only the throttled call");
    let verdict = first.conclude(DRAIN).await;
    assert!(
        verdict.is_err_and(EffectExecutionError::is_deferred),
        "{verdict:?}"
    );
    assert_eq!(harness.slots().await.len(), 1, "nothing prepared above it");
    assert_eq!(
        harness.slots().await[0]
            .record()
            .protocol()
            .expect("protocol")
            .unsent_failure(),
        None
    );

    // The retry: throttled again, recorded this time; the branch runs.
    harness.desk.script(&[Reply::Throttled]);
    let retry = harness.journal(2);
    let (throttled, then) = program(&retry).await;
    assert!(throttled.is_some());
    then.expect("the later effect applies");
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    assert_eq!(
        harness.slots().await[0]
            .record()
            .protocol()
            .expect("protocol")
            .unsent_failure()
            .map(UnsentFailureCode::as_str),
        Some("exhausted")
    );

    // A recovery: the lower one is superseded with the recorded throttle,
    // the same branch replays the later one, nothing is sent.
    let sent = harness.desk.keys().len();
    let recovery = harness.journal(3);
    let (throttled, then) = program(&recovery).await;
    assert_eq!(
        throttled,
        Some(nebula_resource::error::ErrorKind::Exhausted { retry_after: None })
    );
    then.expect("replayed");
    assert_eq!(recovery.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), sent, "nothing sent on recovery");
}

/// Two recorded slots that sent nothing, the higher one prepared after the
/// lower one settled (`concurrent_with = []`). A replay polling both
/// together grants the higher one only once the lower one settled: the
/// lower applies first, as the program ordered them.
#[tokio::test]
async fn a_replay_keeps_the_recorded_order_of_two_unsent_slots() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Throttled, Reply::Throttled]);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    for order in [70, 71] {
        handle
            .submit(Charge::<false> { order })
            .await
            .expect_err("throttled");
    }
    assert_eq!(first.conclude(DRAIN).await, Ok(()));
    let slots = harness.slots().await;
    assert_eq!(
        slots[1]
            .record()
            .protocol()
            .expect("protocol")
            .concurrent_with(),
        Some(&[][..]),
        "the lower one settled before the higher one began"
    );

    // The replay polls both; the lower call is held at the provider.
    harness.desk.script(&[Reply::Held]);
    let retry = harness.journal(2);
    let handle = harness.handle(&retry);
    let lower = tokio::spawn(handle.submit(Charge::<false> { order: 70 }));
    calls_reach(&harness.desk, 3).await;
    let higher = tokio::spawn(handle.submit(Charge::<false> { order: 71 }));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        harness.desk.keys().len(),
        3,
        "the higher one waits for the lower one to settle"
    );
    harness.desk.release.notify_waiters();
    lower.await.expect("task").expect("the lower one applies");
    higher.await.expect("task").expect("then the higher one");
    let keys = harness.desk.keys();
    assert_eq!(keys.len(), 4);
    assert_eq!(keys[2], keys[0], "the lower one, under its recorded key");
    assert_eq!(
        keys[3], keys[1],
        "then the higher one, under its recorded key"
    );
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
}

/// A replayed answer keeps the recorded order too: a higher slot that
/// recorded a rejection after the lower one settled (`concurrent_with =
/// []`) hands its rejection back only once the lower one — sent again on
/// replay — settles, so the program cannot run its next effect ahead of it.
#[tokio::test]
async fn a_replayed_outcome_waits_for_the_lower_units_recorded_before_it() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Throttled, Reply::Rejected]);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    for order in [74, 75] {
        handle
            .submit(Charge::<false> { order })
            .await
            .expect_err("throttled, then rejected");
    }
    assert_eq!(first.conclude(DRAIN).await, Ok(()));

    // The replay: the lower one is sent again and held at the provider.
    harness.desk.script(&[Reply::Held]);
    let retry = harness.journal(2);
    let handle = harness.handle(&retry);
    let lower = tokio::spawn(handle.submit(Charge::<false> { order: 74 }));
    calls_reach(&harness.desk, 3).await;
    let higher = tokio::spawn(handle.submit(Charge::<false> { order: 75 }));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !higher.is_finished(),
        "the recorded rejection waits for the lower unit to settle"
    );
    harness.desk.release.notify_waiters();
    lower.await.expect("task").expect("the lower one applies");
    higher
        .await
        .expect("task")
        .expect_err("then the recorded rejection replays");
    assert_eq!(harness.desk.keys().len(), 3, "the rejection replays unsent");
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
}

/// A replay whose own deadline passes while it waits for the lower units
/// recorded before it never received the recorded answer: its position is
/// let go unmet (abandoned), so the node does not conclude as if the
/// recorded outcome had been replayed.
#[tokio::test]
async fn a_replay_dropped_while_it_waits_its_turn_is_not_counted_replayed() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Throttled, Reply::Rejected]);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    for order in [76, 77] {
        handle
            .submit(Charge::<false> { order })
            .await
            .expect_err("throttled, then rejected");
    }
    assert_eq!(first.conclude(DRAIN).await, Ok(()));

    harness.desk.script(&[Reply::Held]);
    let retry = harness.journal(2);
    let handle = harness.handle(&retry);
    let lower = tokio::spawn(handle.submit(Charge::<false> { order: 76 }));
    calls_reach(&harness.desk, 3).await;
    let gave_up = handle
        .submit(Charge::<false> { order: 77 })
        .with_deadline(Instant::now() + Duration::from_millis(100))
        .await
        .expect_err("its deadline passes while the lower one is held");
    assert_eq!(gave_up.sent(), SentState::NotSent, "{gave_up}");
    harness.desk.release.notify_waiters();
    lower.await.expect("task").expect("the lower one applies");
    let verdict = retry.conclude(DRAIN).await;
    assert!(
        verdict.is_err_and(EffectExecutionError::is_deferred),
        "the recorded answer was never delivered: not a clean replay ({verdict:?})"
    );
    assert_eq!(harness.desk.keys().len(), 3, "the rejection was never sent");
}

/// A recorded pair the program ran together (the higher one lists the lower
/// as concurrent) is not serialized on replay.
#[tokio::test]
async fn a_replay_does_not_serialize_a_recorded_concurrent_pair() {
    let harness = Harness::new().await;
    // The lower stable-key call hangs (the process dies with it open); the
    // higher one is throttled meanwhile.
    harness.desk.script(&[Reply::Hang, Reply::Throttled]);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    let crashed = tokio::spawn(handle.submit(Charge::<true> { order: 72 }));
    calls_reach(&harness.desk, 1).await;
    handle
        .submit(Charge::<false> { order: 73 })
        .await
        .expect_err("throttled");
    crashed.abort();
    let _ = crashed.await;
    first.finish_occurrence("unit/v1/#000000");
    assert_eq!(
        harness.slots().await[1]
            .record()
            .protocol()
            .expect("protocol")
            .concurrent_with(),
        Some(&[at(0)][..])
    );

    // The replay: the lower one is granted again and held; the higher one
    // is granted alongside it.
    harness.desk.script(&[Reply::Held]);
    let retry = harness.journal(2);
    let handle = harness.handle(&retry);
    let lower = tokio::spawn(handle.submit(Charge::<true> { order: 72 }));
    calls_reach(&harness.desk, 3).await;
    handle
        .submit(Charge::<false> { order: 73 })
        .await
        .expect("granted while the lower one is open");
    assert_eq!(harness.desk.keys().len(), 4);
    harness.desk.release.notify_waiters();
    lower.await.expect("task").expect("the lower one applies");
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
}

#[tokio::test]
async fn a_changed_request_under_the_same_occurrence_is_a_mismatch_and_sends_nothing() {
    let harness = Harness::new().await;
    let first = harness.journal(1);
    harness
        .handle(&first)
        .submit(Charge::<false> { order: 10 })
        .await
        .expect("applied");
    assert_eq!(first.conclude(DRAIN).await, Ok(()));

    let retry = harness.journal(2);
    let refused = harness
        .handle(&retry)
        .submit(Charge::<false> { order: 11 })
        .await
        .expect_err("another request under the same occurrence");
    assert_eq!(refused.sent(), SentState::NotSent);
    assert_eq!(
        retry.conclude(DRAIN).await,
        Err(EffectExecutionError::OccurrenceMismatch)
    );
    assert_eq!(harness.desk.keys().len(), 1, "nothing sent");
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_REFUSALS_TOTAL,
            &[
                ("step", effect_journal_step::PREPARE),
                ("refusal", JournalRefusal::Mismatch.as_str()),
            ]
        ),
        1
    );
}

#[tokio::test]
async fn a_lost_lease_defers_the_node_without_a_provider_call() {
    let harness = Harness::new().await;
    assert!(
        harness
            .executions
            .release_lease(
                &harness.scope,
                &harness.execution_id.to_string(),
                harness.fencing
            )
            .await
            .expect("released")
    );
    let journal = harness.journal(1);
    let refused = harness
        .handle(&journal)
        .submit(Charge::<false> { order: 12 })
        .await
        .expect_err("no lease, no prepare");
    assert_eq!(refused.sent(), SentState::NotSent);
    let verdict = journal.conclude(DRAIN).await;
    assert_eq!(
        verdict,
        Err(EffectExecutionError::Ledger(
            OperationLedgerError::ExecutionLeaseRejected
        ))
    );
    assert!(verdict.is_err_and(EffectExecutionError::is_deferred));
    assert!(harness.desk.keys().is_empty());
}

/// A deferring failure of one unit never masks the unknown outcome another
/// unit recorded: the verdict reads the occurrences first and halts.
#[tokio::test]
async fn a_deferral_never_masks_an_unknown_outcome_another_unit_recorded() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Lost]);
    let journal = harness.journal(1);
    let lost = harness
        .handle(&journal)
        .submit(Charge::<false> { order: 30 })
        .await
        .expect_err("the answer was lost");
    assert_eq!(lost.sent(), SentState::MaybeSent);
    assert!(
        harness
            .executions
            .release_lease(
                &harness.scope,
                &harness.execution_id.to_string(),
                harness.fencing
            )
            .await
            .expect("released")
    );
    let refused = harness
        .handle(&journal)
        .submit(Charge::<false> { order: 31 })
        .await
        .expect_err("no lease, no prepare");
    assert_eq!(refused.sent(), SentState::NotSent);
    let verdict = journal.conclude(DRAIN).await;
    assert!(
        matches!(
            verdict,
            Err(EffectExecutionError::JournalOutcomeUnknown { unresolved: 1, .. })
        ),
        "{verdict:?}"
    );
    assert!(verdict.is_err_and(EffectExecutionError::halts_execution));
    assert_eq!(harness.desk.keys().len(), 1);
}

/// A deferral whose verdict read cannot run stands.
#[tokio::test(start_paused = true)]
async fn a_deferral_stands_when_the_verdict_read_cannot_run() {
    let harness = Harness::new().await;
    assert!(
        harness
            .executions
            .release_lease(
                &harness.scope,
                &harness.execution_id.to_string(),
                harness.fencing
            )
            .await
            .expect("released")
    );
    let journal = harness.journal(1);
    harness
        .handle(&journal)
        .submit(Charge::<false> { order: 32 })
        .await
        .expect_err("no lease, no prepare");
    harness.ledger.hang_reads.store(true, Ordering::SeqCst);
    assert_eq!(
        journal.conclude(DRAIN).await,
        Err(EffectExecutionError::Ledger(
            OperationLedgerError::ExecutionLeaseRejected
        ))
    );
}

#[tokio::test(start_paused = true)]
async fn a_leaked_unit_bounds_the_drain_and_the_closed_journal_refuses_it() {
    let harness = Harness::new().await;
    let journal = harness.journal(1);
    let settled = journal.track();
    let leaked = journal.track();
    drop(settled);
    assert!(
        !journal.drain(Duration::from_secs(5)).await,
        "one unit leaked"
    );
    journal.close();
    assert!(journal.is_closed());
    let slot = JournalSlot::new(
        [1; 16],
        IdempotencyKey::new("k").expect("key"),
        0,
        SlotPhase::Runnable,
    );
    assert_eq!(journal.grant(&slot).await, Err(JournalRefusal::Closed));
    assert_eq!(
        journal
            .settle(
                &slot,
                CallGrant::from_bytes([2; 16]),
                CallOutcome::AppliedWithoutOutput
            )
            .await,
        Err(JournalRefusal::Closed)
    );
    drop(leaked);
    assert!(journal.drain(Duration::from_secs(5)).await);
    assert_eq!(harness.ledger.calls(), 0);
}

#[tokio::test]
async fn an_ambiguous_stable_key_call_the_action_swallowed_fails_the_node() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Lost, Reply::Lost]);
    let journal = harness.journal(1);
    let error = harness
        .handle(&journal)
        .submit(Charge::<true> { order: 15 })
        .await
        .expect_err("both answers were lost");
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert_eq!(harness.desk.keys().len(), 2);
    let slots = harness.slots().await;
    assert_eq!(Harness::phase(&slots[0]), EffectPhase::Ambiguous);

    // The action swallowed the error; a call that may have applied still
    // fails the node.
    let verdict = journal.conclude(DRAIN).await;
    assert!(
        matches!(
            verdict,
            Err(EffectExecutionError::JournalOutcomeUnknown { unresolved: 1, .. })
        ),
        "{verdict:?}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_grant_bounds_the_call_by_what_is_left_of_the_key_window() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Hang]);
    let journal = harness.journal(1);
    let started = tokio::time::Instant::now();
    let error = harness
        .handle(&journal)
        .submit(Windowed { order: 16 })
        .await
        .expect_err("cut at the end of the key window");
    let elapsed = started.elapsed();
    assert!(
        elapsed <= SHORT_WINDOW,
        "the call ran past the key window: {elapsed:?}"
    );
    assert_eq!(error.sent(), SentState::MaybeSent);
    assert_eq!(harness.desk.keys().len(), 1);
    assert!(matches!(
        journal.conclude(DRAIN).await,
        Err(EffectExecutionError::JournalOutcomeUnknown { unresolved: 1, .. })
    ));
}

#[tokio::test]
async fn a_first_grant_with_no_window_left_is_withheld_and_not_crossed() {
    let harness = Harness::new().await;
    // Every ledger round trip seems to outlast the two-second key window.
    let journal = harness.journal_with_clock(1, SteppingClock::new(Duration::from_secs(3)));
    let refused = harness
        .handle(&journal)
        .submit(Windowed { order: 17 })
        .await
        .expect_err("no window left for the call");
    assert_eq!(refused.sent(), SentState::NotSent);
    assert!(harness.desk.keys().is_empty(), "nothing sent");
    let slots = harness.slots().await;
    assert_eq!(Harness::phase(&slots[0]), EffectPhase::BeforeBoundary);
    assert_eq!(journal.conclude(DRAIN).await, Ok(()), "nothing crossed");
}

#[tokio::test]
async fn a_regrant_with_no_window_left_makes_the_outcome_unknown() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Lost]);
    let first = harness.journal(1);
    harness
        .handle(&first)
        .submit(Windowed { order: 18 })
        .await
        .expect_err("the answer was lost");
    assert!(first.conclude(DRAIN).await.is_err());

    // The retry may resend under the same key only while the provider
    // still deduplicates it; the grant arrives after that.
    let retry = harness.journal_with_clock(2, SteppingClock::new(Duration::from_secs(3)));
    let refused = harness
        .handle(&retry)
        .submit(Windowed { order: 18 })
        .await
        .expect_err("past the key window");
    assert_eq!(*refused.kind(), nebula_resource::ErrorKind::OutcomeUnknown);
    assert_eq!(harness.desk.keys().len(), 1, "never resent");
    let slots = harness.slots().await;
    assert_eq!(Harness::phase(&slots[0]), EffectPhase::OutcomeUnknown);
    assert!(matches!(
        retry.conclude(DRAIN).await,
        Err(EffectExecutionError::JournalOutcomeUnknown { unresolved: 1, .. })
    ));
}

#[tokio::test(start_paused = true)]
async fn a_unit_stuck_in_the_ledger_cannot_hold_the_verdict_past_the_drain() {
    let harness = Harness::new().await;
    let journal = harness.journal(1);
    harness.ledger.hang_outcomes();
    let stuck = tokio::spawn(
        harness
            .handle(&journal)
            .submit(Charge::<false> { order: 19 }),
    );
    tokio::time::timeout(Duration::from_secs(30), harness.ledger.hung.notified())
        .await
        .expect("the settle reached the ledger");
    assert_eq!(harness.desk.keys().len(), 1);

    // The unit holds its slot in a ledger write that never returns: the
    // verdict still comes within the drain limit, and the slot it could not
    // inspect counts as unresolved.
    let verdict = tokio::time::timeout(
        Duration::from_mins(1),
        journal.conclude(Duration::from_secs(1)),
    )
    .await
    .expect("the verdict is bounded by the drain limit");
    assert!(
        matches!(
            verdict,
            Err(EffectExecutionError::JournalOutcomeUnknown { unresolved: 1, .. })
        ),
        "{verdict:?}"
    );
    stuck.abort();
}

#[tokio::test]
async fn a_changed_output_recording_is_a_mismatch_and_sends_nothing() {
    let harness = Harness::new().await;
    let first = harness.journal(1);
    harness
        .handle(&first)
        .submit(Charge::<false> { order: 20 })
        .await
        .expect("applied");
    assert_eq!(first.conclude(DRAIN).await, Ok(()));

    // A redeploy turned `RECORD_OUTPUT` off under the same key and version.
    let retry = harness.journal(2);
    let refused = harness
        .handle(&retry)
        .submit(ChargeDigest { order: 20 })
        .await
        .expect_err("recorded under other semantics");
    assert_eq!(refused.sent(), SentState::NotSent);
    assert_eq!(
        retry.conclude(DRAIN).await,
        Err(EffectExecutionError::OccurrenceMismatch)
    );
    assert_eq!(harness.desk.keys().len(), 1, "nothing sent");
}

#[tokio::test]
async fn a_reload_to_another_endpoint_is_a_mismatch_and_sends_nothing() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Lost, Reply::Lost]);
    let first = harness.journal(1);
    harness
        .handle(&first)
        .submit(Charge::<true> { order: 21 })
        .await
        .expect_err("both answers were lost");
    assert!(first.conclude(DRAIN).await.is_err());

    // The row now points at an endpoint with no deduplication history of
    // the key: the ambiguous charge is never resent there.
    harness.reload(2);
    let retry = harness.journal(2);
    let refused = harness
        .handle(&retry)
        .submit(Charge::<true> { order: 21 })
        .await
        .expect_err("another destination under the same occurrence");
    assert_eq!(refused.sent(), SentState::NotSent);
    assert_eq!(*refused.kind(), nebula_resource::ErrorKind::Permanent);
    assert_eq!(harness.desk.keys().len(), 2, "nothing resent");
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_REFUSALS_TOTAL,
            &[
                ("step", effect_journal_step::PREPARE),
                ("refusal", JournalRefusal::Mismatch.as_str()),
            ]
        ),
        1
    );
    assert!(retry.conclude(DRAIN).await.is_err());
}

// ── recorded reads (S10, S11) ────────────────────────────────────────────

/// The protocol of the slot recorded at `occurrence`.
async fn protocol_at(harness: &Harness, occurrence: &str) -> OperationProtocolRecord {
    harness
        .slots()
        .await
        .into_iter()
        .find(|slot| slot.occurrence() == occurrence)
        .and_then(|slot| slot.record().protocol().cloned())
        .expect("recorded")
}

#[tokio::test]
async fn a_recorded_read_replays_its_answer_without_asking_again() {
    let harness = Harness::new().await;
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    assert_eq!(handle.submit(Ask::new()).await.expect("answered"), 1);
    assert_eq!(
        handle
            .submit(Charge::<false> { order: 30 })
            .await
            .expect("applied"),
        2
    );
    assert_eq!(first.conclude(DRAIN).await, Ok(()));
    let read = protocol_at(&harness, OCCURRENCE_0).await;
    assert!(read.is_observation(), "recorded as an observation");
    assert_eq!(
        read.contract().policy().capability(),
        DestinationCapability::StableKey
    );
    assert!(!protocol_at(&harness, OCCURRENCE_1).await.is_observation());
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_PREPARES_TOTAL,
            &[("phase", effect_journal_prepare_phase::OBSERVATION)]
        ),
        1
    );
    assert_eq!(
        harness
            .metrics
            .counter(NEBULA_EFFECT_JOURNAL_RECORDED_READ_BYTES_TOTAL)
            .expect("counter")
            .get(),
        1,
        "the answer `1` is one byte of JSON"
    );

    // The provider would answer differently now: the replay observes the
    // recorded answer, and the effect it steered replays under its key.
    let retry = harness.journal(2);
    let handle = harness.handle(&retry);
    assert_eq!(handle.submit(Ask::new()).await.expect("replayed"), 1);
    assert_eq!(
        handle
            .submit(Charge::<false> { order: 30 })
            .await
            .expect("replayed"),
        2
    );
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 2, "nothing asked or sent again");
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_PREPARES_TOTAL,
            &[("phase", effect_journal_prepare_phase::REPLAY)]
        ),
        2
    );
}

#[tokio::test]
async fn a_read_never_replays_as_a_write_nor_a_write_as_a_read() {
    let harness = Harness::new().await;
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    handle.submit(Ask::new()).await.expect("answered");
    handle
        .submit(Charge::<false> { order: 31 })
        .await
        .expect("applied");
    assert_eq!(first.conclude(DRAIN).await, Ok(()));

    // The program now writes where it read, and reads where it wrote:
    // another effect class at each recorded position, a mismatch, nothing
    // sent.
    let swapped = harness.journal(2);
    let handle = harness.handle(&swapped);
    let write = handle
        .submit(Charge::<false> { order: 31 })
        .await
        .expect_err("a write at a recorded read");
    assert_eq!(write.sent(), SentState::NotSent);
    assert_eq!(*write.kind(), nebula_resource::ErrorKind::Permanent);
    assert_eq!(
        swapped.conclude(DRAIN).await,
        Err(EffectExecutionError::OccurrenceMismatch)
    );
    assert_eq!(harness.desk.keys().len(), 2, "nothing sent");
}

#[tokio::test]
async fn an_unanswered_read_is_never_unknown_and_is_asked_again() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Lost]);
    let first = harness.journal(1);
    let lost = harness
        .handle(&first)
        .submit(Ask::new())
        .await
        .expect_err("the answer was lost");
    assert_eq!(*lost.kind(), nebula_resource::ErrorKind::Transient);
    assert_eq!(lost.sent(), SentState::MaybeSent);
    assert!(lost.is_retryable(), "asked again, never unknown");
    // The node fails with its own error: no unknown outcome.
    assert_eq!(
        first.conclude_node(DRAIN, false).await,
        Ok(Concluded::Clean)
    );
    let read = protocol_at(&harness, OCCURRENCE_0).await;
    assert_eq!(read.phase(), EffectPhase::Ambiguous);
    assert_eq!(
        read.unsent_failure().map(UnsentFailureCode::as_str),
        Some("transient"),
        "its failure is recorded whatever was sent"
    );

    // Nothing recorded above it: the retry asks the same position again.
    let retry = harness.journal(2);
    assert_eq!(
        harness
            .handle(&retry)
            .submit(Ask::new())
            .await
            .expect("asked again"),
        2
    );
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.slots().await.len(), 1, "the same position");
    assert_eq!(harness.desk.keys().len(), 2);
}

#[tokio::test]
async fn a_read_cut_off_mid_call_is_asked_again_after_the_crash() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Hang]);
    let first = harness.journal(1);
    let asking = tokio::spawn(harness.handle(&first).submit(Ask::new()));
    tokio::time::timeout(Duration::from_secs(5), harness.desk.entered.notified())
        .await
        .expect("the call reached the provider");
    // The process dies with the call granted and never explained.
    asking.abort();
    let _ = asking.await;
    assert_eq!(
        protocol_at(&harness, OCCURRENCE_0).await.phase(),
        EffectPhase::InvocationOutstanding
    );

    // The next attempt records the residue ambiguous and asks again — an
    // opaque write would be unknown here; a read is not.
    let retry = harness.journal(2);
    assert_eq!(
        harness
            .handle(&retry)
            .submit(Ask::new())
            .await
            .expect("asked again"),
        2
    );
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 2);
}

#[tokio::test]
async fn an_unanswered_read_below_any_later_position_replays_its_failure() {
    let harness = Harness::new().await;
    // The read's answer is lost; the program goes on (it swallowed the
    // failure) and charges, which the provider only throttles: a slot that
    // changed nothing, above the read.
    harness.desk.script(&[Reply::Lost, Reply::Throttled]);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    handle.submit(Ask::new()).await.expect_err("lost");
    handle
        .submit(Charge::<false> { order: 32 })
        .await
        .expect_err("throttled");
    assert_eq!(
        first.conclude_node(DRAIN, false).await,
        Ok(Concluded::Clean)
    );

    // Asking again could steer the replay elsewhere than the run that
    // recorded the charge: the read fails as the program saw it fail, with
    // nothing sent, and the charge — an effect below nothing — is sent.
    let retry = harness.journal(2);
    let handle = harness.handle(&retry);
    let superseded = handle
        .submit(Ask::new())
        .await
        .expect_err("not asked again");
    assert_eq!(*superseded.kind(), nebula_resource::ErrorKind::Transient);
    assert_eq!(superseded.sent(), SentState::NotSent);
    assert_eq!(
        superseded.detail(),
        "effect failed unsent in an earlier run that moved past it; not sent again"
    );
    assert_eq!(
        handle
            .submit(Charge::<false> { order: 32 })
            .await
            .expect("granted again"),
        3
    );
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 3, "the read was not asked again");
}

#[tokio::test]
async fn an_answered_read_orders_an_unsettled_write_below_it() {
    let harness = Harness::new().await;
    // The charge is throttled (changed nothing); the program goes on and
    // asks the model, whose answer is recorded.
    harness.desk.script(&[Reply::Throttled, Reply::Applied]);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    handle
        .submit(Charge::<false> { order: 33 })
        .await
        .expect_err("throttled");
    assert_eq!(handle.submit(Ask::new()).await.expect("answered"), 2);
    assert_eq!(
        first.conclude_node(DRAIN, false).await,
        Ok(Concluded::Clean)
    );

    // Sending the charge now would apply it after the answer the program
    // observed (S11): it fails as it did, and the answer replays.
    let retry = harness.journal(2);
    let handle = harness.handle(&retry);
    let superseded = handle
        .submit(Charge::<false> { order: 33 })
        .await
        .expect_err("not sent again");
    assert_eq!(superseded.sent(), SentState::NotSent);
    assert_eq!(handle.submit(Ask::new()).await.expect("replayed"), 2);
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 2, "no further call");
}

#[tokio::test]
async fn a_crossed_stable_key_write_below_an_answered_read_is_unknown() {
    let harness = Harness::new().await;
    // The idempotent charge's first answer is lost (it may have applied)
    // and its second attempt is throttled: one of its two calls crossed,
    // so its budget allows another. The program goes on and asks the
    // model.
    harness
        .desk
        .script(&[Reply::Lost, Reply::Throttled, Reply::Applied]);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    handle
        .submit(Charge::<true> { order: 34 })
        .await
        .expect_err("lost, then throttled");
    handle.submit(Ask::new()).await.expect("answered");
    assert!(first.conclude_node(DRAIN, false).await.is_err());

    // Granting the charge again could apply it after the observed answer:
    // its outcome is unknown, no call, and the node halts.
    let retry = harness.journal(2);
    let unknown = harness
        .handle(&retry)
        .submit(Charge::<true> { order: 34 })
        .await
        .expect_err("not granted again");
    assert_eq!(*unknown.kind(), nebula_resource::ErrorKind::OutcomeUnknown);
    assert!(matches!(
        retry.conclude(DRAIN).await,
        Err(EffectExecutionError::JournalOutcomeUnknown { .. })
    ));
    assert_eq!(harness.desk.keys().len(), 3, "no further call");
}

#[tokio::test]
async fn a_read_whose_ceiling_is_spent_fails_exhausted_and_the_node_is_not_unknown() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Lost]);
    let first = harness.journal(1);
    harness
        .handle(&first)
        .submit(Ask::new())
        .await
        .expect_err("lost");
    assert_eq!(
        first.conclude_node(DRAIN, false).await,
        Ok(Concluded::Clean)
    );
    // The ledger closes the slot (its window or ceiling ran out).
    let slot = harness.slots().await[0].record().operation().slot_id();
    let revision = protocol_at(&harness, OCCURRENCE_0).await.revision();
    harness
        .ledger
        .inner
        .advance(
            &harness.scope,
            slot,
            harness.fencing,
            &OperationCommand::MarkUnknown {
                expected_revision: revision,
            },
        )
        .await
        .expect("spent");

    let retry = harness.journal(2);
    let spent = harness
        .handle(&retry)
        .submit(Ask::new())
        .await
        .expect_err("not asked again");
    assert_eq!(
        *spent.kind(),
        nebula_resource::ErrorKind::Exhausted { retry_after: None }
    );
    assert_eq!(spent.sent(), SentState::NotSent);
    assert_eq!(
        retry.conclude_node(DRAIN, false).await,
        Ok(Concluded::Clean),
        "never an unknown outcome"
    );
    assert_eq!(harness.desk.keys().len(), 1, "never asked again");
}

#[tokio::test]
async fn a_read_spent_before_its_failure_was_recorded_replays_exhausted_once_superseded() {
    let harness = Harness::new().await;
    harness.desk.script(&[Reply::Hang, Reply::Applied]);
    let first = harness.journal(1);
    let asking = tokio::spawn(harness.handle(&first).submit(Ask::new()));
    tokio::time::timeout(Duration::from_secs(5), harness.desk.entered.notified())
        .await
        .expect("the call reached the provider");
    // The process dies mid-call: no failure is recorded for the read.
    asking.abort();
    let _ = asking.await;
    // The ledger closes the slot (its window or ceiling ran out) before any
    // run recorded how the read failed.
    let slot = harness.slots().await[0].record().operation().slot_id();
    let revision = protocol_at(&harness, OCCURRENCE_0).await.revision();
    harness
        .ledger
        .inner
        .advance(
            &harness.scope,
            slot,
            harness.fencing,
            &OperationCommand::MarkUnknown {
                expected_revision: revision,
            },
        )
        .await
        .expect("spent");
    assert_eq!(
        protocol_at(&harness, OCCURRENCE_0).await.unsent_failure(),
        None
    );

    // A run whose owner cannot record the read's failure fails closed:
    // nothing fresh is prepared above the read, and the node defers.
    harness
        .ledger
        .fail_next_unsent_failure
        .store(true, Ordering::SeqCst);
    let held_run = harness.journal(2);
    let handle = harness.handle(&held_run);
    let spent = handle.submit(Ask::new()).await.expect_err("spent");
    assert_eq!(
        *spent.kind(),
        nebula_resource::ErrorKind::Exhausted { retry_after: None }
    );
    let held = handle
        .submit(Charge::<false> { order: 35 })
        .await
        .expect_err("held above the unrecorded read");
    assert_eq!(held.sent(), SentState::NotSent);
    let verdict = held_run.conclude(DRAIN).await;
    assert!(
        verdict.is_err_and(EffectExecutionError::is_deferred),
        "{verdict:?}"
    );
    assert_eq!(harness.slots().await.len(), 1, "nothing prepared above it");

    // The next run's read fails `Exhausted`; the program goes on and
    // charges above it.
    let retry = harness.journal(3);
    let handle = harness.handle(&retry);
    let spent = handle.submit(Ask::new()).await.expect_err("spent");
    assert_eq!(
        *spent.kind(),
        nebula_resource::ErrorKind::Exhausted { retry_after: None }
    );
    assert_eq!(
        protocol_at(&harness, OCCURRENCE_0)
            .await
            .unsent_failure()
            .map(UnsentFailureCode::as_str),
        Some("exhausted"),
        "recorded before the program saw it"
    );
    assert_eq!(
        handle
            .submit(Charge::<false> { order: 35 })
            .await
            .expect("applied"),
        2
    );
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));

    // A later run meets the read superseded by the charge: it fails as the
    // program saw it fail, `Exhausted` — never `Permanent`.
    let replay = harness.journal(4);
    let handle = harness.handle(&replay);
    let superseded = handle.submit(Ask::new()).await.expect_err("superseded");
    assert_eq!(
        *superseded.kind(),
        nebula_resource::ErrorKind::Exhausted { retry_after: None }
    );
    assert_eq!(superseded.sent(), SentState::NotSent);
    assert_eq!(
        handle
            .submit(Charge::<false> { order: 35 })
            .await
            .expect("replayed"),
        2
    );
    assert_eq!(replay.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 2, "never asked again");
}

#[tokio::test]
async fn an_answer_the_ledger_did_not_record_is_withheld_and_holds_later_effects() {
    let harness = Harness::new().await;
    // Record before return: the settle of the answer fails, so the caller
    // never sees the answer.
    harness.ledger.fail_outcomes.store(true, Ordering::SeqCst);
    let first = harness.journal(1);
    let handle = harness.handle(&first);
    let withheld = handle
        .submit(Ask::new())
        .await
        .expect_err("the answer was not recorded");
    assert_eq!(*withheld.kind(), nebula_resource::ErrorKind::Transient);
    assert_eq!(withheld.sent(), SentState::MaybeSent);
    assert!(withheld.is_retryable());
    harness.ledger.fail_outcomes.store(false, Ordering::SeqCst);
    // The program went on from the failure: nothing fresh is written above
    // the read in this attempt.
    let held = handle
        .submit(Charge::<false> { order: 35 })
        .await
        .expect_err("held above the unrecorded read");
    assert_eq!(held.sent(), SentState::NotSent);
    assert_eq!(harness.desk.keys().len(), 1, "only the read was asked");
    let verdict = first.conclude_node(DRAIN, false).await;
    assert!(
        verdict.as_ref().is_err_and(|error| error.is_deferred()),
        "{verdict:?}"
    );

    // The retry asks the same read again and only then charges.
    let retry = harness.journal(2);
    let handle = harness.handle(&retry);
    assert_eq!(handle.submit(Ask::new()).await.expect("asked again"), 2);
    assert_eq!(
        handle
            .submit(Charge::<false> { order: 35 })
            .await
            .expect("applied"),
        3
    );
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
}

#[tokio::test]
async fn a_lost_settle_acknowledgement_recommits_the_exact_answer() {
    let harness = Harness::new().await;
    harness
        .ledger
        .lose_next_outcome_ack
        .store(true, Ordering::SeqCst);
    let first = harness.journal(1);
    assert_eq!(
        harness
            .handle(&first)
            .submit(Ask::new())
            .await
            .expect("recorded once the exact answer reads back"),
        1
    );
    assert_eq!(first.conclude(DRAIN).await, Ok(()));
    let retry = harness.journal(2);
    assert_eq!(
        harness
            .handle(&retry)
            .submit(Ask::new())
            .await
            .expect("replayed"),
        1
    );
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.desk.keys().len(), 1);
}
