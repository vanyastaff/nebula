use std::collections::VecDeque;

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

const OCCURRENCE_0: &str = "unit/v1/billing.gateway/op/billing.charge/v1/#000000";
const OCCURRENCE_1: &str = "unit/v1/billing.gateway/op/billing.charge/v1/#000001";

#[test]
fn provider_key_golden_vectors() {
    // Pinned values: a change here re-keys every journaled effect in
    // flight, so the provider would apply a retried effect twice.
    let developer = derived_key(&key_parts(Some("order-123"), "exec-1", OCCURRENCE_0));
    assert_eq!(developer, "rT-cFUezYGxWViT9yxc08nun9dXX-uC5FWXmLEN4PWI");
    assert_eq!(developer, expected_key(1, b"order-123"));

    let run = derived_key(&key_parts(None, "exec-1", OCCURRENCE_0));
    assert_eq!(run, "DgUZL5fC2s0-8JzBlM9tyBtrisosRGHk_CSmtT23tDE");
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
    ] {
        assert_eq!(
            policy(effect, recovery, 1).err(),
            Some(EffectExecutionError::InvalidContract)
        );
    }
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
    let applied = journal_evidence(operation, call, CallOutcome::Applied(output)).expect("applied");
    assert_eq!(applied.outcome(), KnownOutcome::Succeeded);
    let Ok(RecordedOutcome::Succeeded(bytes)) = replay(operation, &applied) else {
        panic!("an applied output replays");
    };
    assert_eq!(
        serde_json::from_slice::<Value>(&bytes).expect("json"),
        serde_json::from_slice::<Value>(output).expect("json")
    );

    let digest_only =
        journal_evidence(operation, call, CallOutcome::AppliedWithoutOutput).expect("digest");
    assert_eq!(
        replay(operation, &digest_only),
        Ok(RecordedOutcome::OutputUnavailable)
    );

    for code in ERROR_KIND_CODES {
        let rejected =
            journal_evidence(operation, call, CallOutcome::Rejected(code)).expect("rejected");
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
    let kept = journal_evidence(operation, call, CallOutcome::Applied(&oversized)).expect("kept");
    assert_eq!(kept.outcome(), KnownOutcome::Succeeded);
    assert_eq!(
        replay(operation, &kept),
        Ok(RecordedOutcome::OutputUnavailable)
    );
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

/// Counts every ledger call, forwarding to the in-memory ledger.
#[derive(Debug)]
struct CountingLedger {
    inner: nebula_storage::inmem::InMemoryOperationLedger,
    calls: AtomicUsize,
}

impl CountingLedger {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn count(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
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
        self.inner.prepare(binding, fencing).await
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
        self.inner.advance(scope, slot_id, fencing, command).await
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
                config: (),
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
        NodeEffectJournal::new(JournalAuthority {
            ledger: Arc::clone(&self.ledger) as Arc<dyn OperationLedger>,
            scope: self.scope.clone(),
            fencing: self.fencing,
            execution_id: self.execution_id,
            node_key: NodeKey::new("charge").expect("node key"),
            action_key: "billing.charge".to_owned(),
            action_version: semver::Version::new(1, 0, 0),
            attempt_generation,
            clock: Arc::new(nebula_core::accessor::SystemClock),
            metrics: self.metrics.clone(),
        })
    }

    /// The gateway's handle under `journal`.
    fn handle(&self, journal: &NodeEffectJournal) -> ResourceHandle<Gateway> {
        *self
            .manager
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
}

/// The fake gateway: every call it received, with the key it carried.
#[derive(Debug, Default)]
struct Desk {
    keys: Mutex<Vec<Option<String>>>,
    replies: Mutex<VecDeque<Reply>>,
    entered: tokio::sync::Notify,
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
        }
    }
}

#[derive(Clone)]
struct Gateway(Arc<Desk>);

#[async_trait::async_trait]
impl Provider for Gateway {
    type Config = ();
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
        (): &(),
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

const DRAIN: Duration = Duration::from_secs(5);

#[test]
fn ordinals_count_per_resource_kind_and_name_from_zero() {
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
    });
    let key = Gateway::key();
    let other = ResourceKey::new("billing.other").expect("resource key");
    assert_eq!(
        journal.next_ordinal(&key, UnitKind::Operation, "billing.charge"),
        0
    );
    assert_eq!(
        journal.next_ordinal(&key, UnitKind::Operation, "billing.charge"),
        1
    );
    assert_eq!(
        journal.next_ordinal(&other, UnitKind::Operation, "billing.charge"),
        0,
        "per resource"
    );
    assert_eq!(
        journal.next_ordinal(&key, UnitKind::Session, "billing.charge"),
        0,
        "sessions and operations are separate namespaces"
    );
    assert_eq!(
        journal.next_ordinal(&key, UnitKind::Operation, "billing.refund"),
        0,
        "per name"
    );
    assert_eq!(
        journal.next_ordinal(&key, UnitKind::Operation, "billing.charge"),
        2
    );
}

#[tokio::test]
async fn a_read_only_in_practice_node_does_no_ledger_io() {
    let harness = Harness::new().await;
    let journal = harness.journal(1);
    let handle = harness.handle(&journal);
    assert_eq!(handle.submit(Balance).await.expect("a read runs"), 0);
    assert_eq!(handle.submit(Balance).await.expect("a read runs"), 0);
    assert_eq!(journal.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.ledger.calls(), 0, "reads are never prepared");
    assert!(harness.slots().await.is_empty());
    assert_eq!(
        harness.counter(
            NEBULA_EFFECT_JOURNAL_VERDICTS_TOTAL,
            &[("code", effect_journal_verdict::OK)]
        ),
        1
    );

    // A later attempt of the node may find slots an earlier one left: it
    // reads the node's occurrences once, and nothing else.
    let retry = harness.journal(2);
    assert_eq!(retry.conclude(DRAIN).await, Ok(()));
    assert_eq!(harness.ledger.calls(), 1);
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
