//! Scenarios drive real engine decisions; evidence is read independently afterwards.

mod capture;
mod evidence;
mod fixture;
mod metrics;
mod takeover;

use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};

use nebula_engine::{
    ActionRuntime, ControlDispatch, DataPassingPolicy, EngineControlDispatch, InProcessRunner,
    PlanFlavorRevisionLoader, WorkflowEngine,
};
use nebula_execution::{ExecutionControlOutcome, ExecutionControlSource, ExecutionState};
use nebula_metrics::MetricsRegistry;
use nebula_storage_port::store::ControlClaimToken;

use self::{
    capture::TraceCapture,
    evidence::CaseEvidence,
    fixture::{Admitted, Ports},
};

fn engine(
    ports: &Ports,
    calls: &Arc<AtomicU32>,
    registry: &MetricsRegistry,
    artifact: u8,
) -> Arc<WorkflowEngine> {
    let (actions, frozen) = fixture::frozen_registry_for_artifact(calls, artifact);
    let runtime = Arc::new(
        ActionRuntime::try_new(
            actions,
            Arc::new(InProcessRunner::new()),
            DataPassingPolicy::default(),
            registry.clone(),
        )
        .expect("scenario action runtime"),
    );
    Arc::new(
        WorkflowEngine::new(runtime, registry.clone())
            .expect("scenario engine")
            .with_execution_stores(ports.stores.clone())
            .with_plan_flavor_runtime(
                Arc::new(PlanFlavorRevisionLoader::new(ports.catalog.clone())),
                frozen,
                ports.bundles.clone(),
            ),
    )
}

fn dispatch(ports: &Ports, engine: Arc<WorkflowEngine>) -> EngineControlDispatch {
    EngineControlDispatch::new(
        engine,
        ports.stores.execution.clone(),
        ports.handoff.clone(),
        "operator-outcome-scenario".to_owned(),
        Duration::from_secs(30),
    )
}

async fn claim_start(ports: &Ports, calls: &Arc<AtomicU32>) -> ControlClaimToken {
    let (_, frozen) = fixture::frozen_registry(calls);
    let mut claims = ports
        .queue
        .claim_pending_for_flavor(&[0x72; 16], 1, frozen.revision().id())
        .await
        .expect("claim actual persisted Start");
    assert_eq!(claims.len(), 1);
    claims.remove(0).token
}

fn claim_source(claim: &ControlClaimToken) -> ExecutionControlSource {
    ExecutionControlSource::ControlQueue {
        row_id: *claim.row_id(),
        queue_claim_generation: claim.generation().get(),
    }
}

async fn claim_resume(
    ports: &Ports,
    admitted: &Admitted,
    calls: &Arc<AtomicU32>,
) -> ControlClaimToken {
    let message = nebula_storage_port::dto::ControlMsg {
        id: nebula_core::ExecutionId::new().as_bytes(),
        execution_id: admitted.id.to_string(),
        scope: admitted.scope.clone(),
        command: nebula_storage_port::dto::ControlCommand::Resume,
        resume_target: None,
        w3c_traceparent: None,
        reclaim_count: 0,
    };
    ports
        .queue
        .enqueue(&message)
        .await
        .expect("persist real Resume command");
    claim_start(ports, calls).await
}

async fn collect(
    ports: &Ports,
    admitted: &Admitted,
    registry: &MetricsRegistry,
    (before, journal_before): (Vec<metrics::CounterObservation>, usize),
    source: ExecutionControlSource,
    outcome: ExecutionControlOutcome,
    backend: &str,
) -> CaseEvidence {
    let row = ports
        .stores
        .execution
        .get(&admitted.scope, &admitted.id.to_string())
        .await
        .expect("read actual execution state")
        .expect("scenario durable execution");
    assert_eq!(row.id, admitted.id.to_string());
    assert_eq!(row.scope, admitted.scope);
    let state = <ExecutionState as serde::Deserialize>::deserialize(&row.state)
        .expect("typed execution state");
    assert_eq!(state.execution_id, admitted.id);
    let case = CaseEvidence {
        scenario: outcome.as_str().to_owned(),
        execution_id: admitted.id.to_string(),
        org_id: admitted.scope.org_id.clone(),
        workspace_id: admitted.scope.workspace_id.clone(),
        executable_plan_revision_id: state
            .executable_plan_revision_id
            .expect("real admitted plan"),
        worker_flavor_revision_id: state
            .worker_flavor_revision_id
            .expect("real retained flavor"),
        expected_source: source,
        journal: ports
            .stores
            .journal
            .get_journal(&admitted.scope, &admitted.id.to_string())
            .await
            .expect("read backend journal independently"),
        counters_before: before,
        journal_before,
        counters_after: metrics::snapshot(registry),
        trace: TraceCapture::global().for_execution(&admitted.id.to_string()),
    };
    case.verify(backend, outcome);
    case.verify_journal_metric_parity(backend);
    case
}

async fn accepted(ports: Ports, backend: &str) -> CaseEvidence {
    TraceCapture::global();
    let calls = Arc::new(AtomicU32::new(0));
    let admitted = fixture::admit(ports.clone(), &calls, false).await;
    let claim = claim_start(&ports, &calls).await;
    let source = claim_source(&claim);
    let registry = MetricsRegistry::new();
    let before = metrics::snapshot(&registry);
    let journal_before = journal_len(&ports, &admitted).await;
    let owner = dispatch(&ports, engine(&ports, &calls, &registry, 0x74));
    assert!(matches!(
        owner
            .dispatch_claimed_start(&admitted.scope, admitted.id, claim)
            .await,
        nebula_engine::ClaimedControlDispatchOutcome::Accepted(Ok(()))
    ));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "accepted engine really ran predecessor"
    );
    collect(
        &ports,
        &admitted,
        &registry,
        (before, journal_before),
        source,
        ExecutionControlOutcome::Accepted,
        backend,
    )
    .await
}

async fn throttled(ports: Ports, backend: &str) -> CaseEvidence {
    TraceCapture::global();
    let calls = Arc::new(AtomicU32::new(0));
    let admitted = fixture::admit(ports.clone(), &calls, true).await;
    let claim = claim_start(&ports, &calls).await;
    let source_row_id = *claim.row_id();
    let registry = MetricsRegistry::new();
    let before = metrics::snapshot(&registry);
    let journal_before = journal_len(&ports, &admitted).await;
    let owner = dispatch(&ports, engine(&ports, &calls, &registry, 0x74));
    assert!(matches!(
        owner
            .dispatch_claimed_start(&admitted.scope, admitted.id, claim)
            .await,
        nebula_engine::ClaimedControlDispatchOutcome::Accepted(Ok(()))
    ));
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "shared bucket admits predecessor but refuses successor before its action"
    );
    let source = ExecutionControlSource::AcceptedTurn {
        source_kind: nebula_execution::ExecutionControlQueueKind::ControlQueue,
        source_row_id,
        // This fresh admitted execution has had no technical lease before
        // Start. Its first accepted turn must hold generation one.
        accepted_execution_lease_generation: 1,
    };
    let case = collect(
        &ports,
        &admitted,
        &registry,
        (before, journal_before),
        source,
        ExecutionControlOutcome::Throttled,
        backend,
    )
    .await;
    let state = ports
        .stores
        .execution
        .get(&admitted.scope, &admitted.id.to_string())
        .await
        .expect("read throttled state")
        .expect("throttled execution");
    assert_eq!(
        state
            .state
            .pointer("/node_states/successor/state")
            .and_then(serde_json::Value::as_str),
        Some("failed"),
        "actual rate-limit refusal reaches the successor state"
    );
    let observation = case
        .journal
        .iter()
        .find_map(
            |row| match <nebula_execution::JournalEntry as serde::Deserialize>::deserialize(
                &row.payload,
            )
            .unwrap()
            {
                nebula_execution::JournalEntry::ControlObserved { observation, .. }
                    if observation.outcome() == ExecutionControlOutcome::Throttled =>
                {
                    Some(observation)
                },
                _ => None,
            },
        )
        .expect("already verified typed throttle observation");
    let attempt = observation
        .attempt()
        .expect("throttle belongs to a real node attempt");
    assert_eq!(attempt.node_key, nebula_core::node_key!("successor"));
    assert_eq!(attempt.attempt, 0);
    case
}

async fn deferred(mut ports: Ports, backend: &str) -> CaseEvidence {
    TraceCapture::global();
    let calls = Arc::new(AtomicU32::new(0));
    let admitted = fixture::admit(ports.clone(), &calls, false).await;
    let registry = MetricsRegistry::new();
    let engine = engine(&ports, &calls, &registry, 0x74);
    let start = claim_start(&ports, &calls).await;
    assert!(matches!(
        dispatch(&ports, engine.clone())
            .dispatch_claimed_start(&admitted.scope, admitted.id, start)
            .await,
        nebula_engine::ClaimedControlDispatchOutcome::Accepted(Ok(()))
    ));
    let claim = claim_resume(&ports, &admitted, &calls).await;
    let source = claim_source(&claim);
    let interleaving = Arc::new(takeover::CheckpointBeforeCommit {
        inner: ports.handoff.clone(),
        execution: ports.stores.execution.clone(),
        triggered: std::sync::atomic::AtomicBool::new(false),
    });
    ports.handoff = interleaving.clone();
    let before = metrics::snapshot(&registry);
    let journal_before = journal_len(&ports, &admitted).await;
    let state_before = ports
        .stores
        .execution
        .get(&admitted.scope, &admitted.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        dispatch(&ports, engine)
            .dispatch_claimed_resume(&admitted.scope, admitted.id, None, claim)
            .await,
        nebula_engine::ClaimedControlDispatchOutcome::NotAccepted(Err(
            nebula_engine::ControlDispatchError::Deferred(_)
        ))
    ));
    assert!(
        interleaving.triggered.load(Ordering::SeqCst),
        "actual owner checkpoint raced the control commit"
    );
    let state_after = ports
        .stores
        .execution
        .get(&admitted.scope, &admitted.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state_after.version,
        state_before.version + 1,
        "only the real concurrent owner checkpoint committed"
    );
    assert_eq!(
        state_after.state["node_states"], state_before.state["node_states"],
        "deferred delivery never publishes its uncommitted wait arm"
    );
    assert_eq!(state_after.status, state_before.status);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "deferred Resume invokes no successor action"
    );
    collect(
        &ports,
        &admitted,
        &registry,
        (before, journal_before),
        source,
        ExecutionControlOutcome::Deferred,
        backend,
    )
    .await
}
async fn flavor_mismatch(ports: Ports, backend: &str) -> CaseEvidence {
    TraceCapture::global();
    let calls = Arc::new(AtomicU32::new(0));
    let admitted = fixture::admit(ports.clone(), &calls, false).await;
    let registry = MetricsRegistry::new();
    let original = dispatch(&ports, engine(&ports, &calls, &registry, 0x74));
    let start = claim_start(&ports, &calls).await;
    assert!(matches!(
        original
            .dispatch_claimed_start(&admitted.scope, admitted.id, start)
            .await,
        nebula_engine::ClaimedControlDispatchOutcome::Accepted(Ok(()))
    ));
    let claim = claim_resume(&ports, &admitted, &calls).await;
    let source = claim_source(&claim);
    let before = metrics::snapshot(&registry);
    let journal_before = journal_len(&ports, &admitted).await;
    let incompatible = dispatch(&ports, engine(&ports, &calls, &registry, 0x75));
    let decision = incompatible
        .dispatch_claimed_resume(&admitted.scope, admitted.id, None, claim)
        .await;
    assert!(
        matches!(
            decision,
            nebula_engine::ClaimedControlDispatchOutcome::NotAccepted(Err(_))
        ),
        "incompatible real runtime must refuse the retained exact flavor"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "wrong flavor never invokes the successor"
    );
    collect(
        &ports,
        &admitted,
        &registry,
        (before, journal_before),
        source,
        ExecutionControlOutcome::FlavorMismatch,
        backend,
    )
    .await
}

async fn recovered(ports: Ports, backend: &str) -> CaseEvidence {
    TraceCapture::global();
    let calls = Arc::new(AtomicU32::new(0));
    let admitted = fixture::admit(ports.clone(), &calls, false).await;
    let claim = claim_start(&ports, &calls).await;
    let source_row_id = *claim.row_id();
    let (_, frozen) = fixture::frozen_registry(&calls);
    let id = admitted.id.to_string();
    let row = ports
        .stores
        .execution
        .get(&admitted.scope, &id)
        .await
        .unwrap()
        .unwrap();
    let handoff = nebula_storage_port::store::ControlStartHandoff::for_claim(
        &admitted.scope,
        &id,
        claim,
        frozen.revision().id(),
    )
    .at_version(row.version)
    .lease_to("crashed-before-first-checkpoint", Duration::from_secs(30));
    let nebula_storage_port::store::ControlStartAcceptance::Accepted { fence } =
        ports.handoff.accept_control_start(&handoff).await.unwrap()
    else {
        panic!("real owner must durably accept the crash-window Start");
    };
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "accepted crash window ran no action"
    );
    assert!(
        ports
            .stores
            .execution
            .release_lease(&admitted.scope, &id, fence)
            .await
            .unwrap()
    );
    let candidates = ports
        .recovery
        .list_recoverable_turns(frozen.revision().id(), None, 16)
        .await
        .unwrap();
    let candidate = candidates
        .turns()
        .iter()
        .find(|candidate| candidate.execution_id() == admitted.id)
        .expect("actual owner acceptance retained a recoverable marker");
    assert_eq!(candidate.scope(), &admitted.scope);
    assert_eq!(candidate.accepted_fencing_generation(), fence.generation());
    let source = ExecutionControlSource::AcceptedTurn {
        source_kind: nebula_execution::ExecutionControlQueueKind::ControlQueue,
        source_row_id,
        accepted_execution_lease_generation: fence.generation(),
    };
    let registry = MetricsRegistry::new();
    let before = metrics::snapshot(&registry);
    let journal_before = journal_len(&ports, &admitted).await;
    let owner = engine(&ports, &calls, &registry, 0x74);
    let decision = owner
        .resume_recoverable_turn(
            &admitted.scope,
            admitted.id,
            nebula_engine::RecoveryTurnRequest {
                handoff: ports.recovery.as_ref(),
                holder: "operator-recovery-owner",
                lease_ttl: Duration::from_secs(30),
                accepted_fencing_generation: candidate.accepted_fencing_generation(),
            },
        )
        .await;
    assert!(
        matches!(
            decision,
            nebula_engine::RecoveryTurnOutcome::Accepted(Ok(_))
        ),
        "real engine accepts the retained owner marker"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "recovered engine really ran predecessor"
    );
    let case = collect(
        &ports,
        &admitted,
        &registry,
        (before, journal_before),
        source,
        ExecutionControlOutcome::Recovered,
        backend,
    )
    .await;
    for row in &case.journal {
        if let nebula_execution::JournalEntry::ControlObserved { observation, .. } =
            <nebula_execution::JournalEntry as serde::Deserialize>::deserialize(&row.payload)
                .unwrap()
            && observation.outcome() == ExecutionControlOutcome::Recovered
        {
            assert!(
                observation.execution_lease_generation() > fence.generation(),
                "recovery replaces the historical accepted generation"
            );
        }
    }
    case
}

async fn fenced(mut ports: Ports, backend: &str) -> CaseEvidence {
    TraceCapture::global();
    let calls = Arc::new(AtomicU32::new(0));
    let admitted = fixture::admit(ports.clone(), &calls, false).await;
    let registry = MetricsRegistry::new();
    let owner = engine(&ports, &calls, &registry, 0x74);
    let initial = dispatch(&ports, owner.clone());
    let start = claim_start(&ports, &calls).await;
    assert!(matches!(
        initial
            .dispatch_claimed_start(&admitted.scope, admitted.id, start)
            .await,
        nebula_engine::ClaimedControlDispatchOutcome::Accepted(Ok(()))
    ));
    let claim = claim_resume(&ports, &admitted, &calls).await;
    let source = claim_source(&claim);
    let interleaving = Arc::new(takeover::TakeoverBeforeCommit {
        inner: ports.handoff.clone(),
        execution: ports.stores.execution.clone(),
        successor: std::sync::Mutex::new(None),
        triggered: std::sync::atomic::AtomicBool::new(false),
    });
    ports.handoff = interleaving.clone();
    let before = metrics::snapshot(&registry);
    let journal_before = journal_len(&ports, &admitted).await;
    let state_before = ports
        .stores
        .execution
        .get(&admitted.scope, &admitted.id.to_string())
        .await
        .unwrap()
        .unwrap();
    let decision = dispatch(&ports, owner)
        .dispatch_claimed_resume(&admitted.scope, admitted.id, None, claim)
        .await;
    assert!(matches!(
        decision,
        nebula_engine::ClaimedControlDispatchOutcome::NotAccepted(Err(_))
    ));
    assert!(
        interleaving.triggered.load(Ordering::SeqCst),
        "real engine reached the commit boundary"
    );
    let state_after = ports
        .stores
        .execution
        .get(&admitted.scope, &admitted.id.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (state_before.version, state_before.state),
        (state_after.version, state_after.state),
        "stale engine checkpoint never mutates the successor's execution aggregate"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "fenced actor runs no successor action"
    );
    let case = collect(
        &ports,
        &admitted,
        &registry,
        (before, journal_before),
        source,
        ExecutionControlOutcome::Fenced,
        backend,
    )
    .await;
    let successor = interleaving
        .successor
        .lock()
        .unwrap()
        .take()
        .expect("actual successor token");
    assert!(
        ports
            .stores
            .execution
            .release_lease(&admitted.scope, &admitted.id.to_string(), successor)
            .await
            .unwrap()
    );
    case
}

async fn all_outcomes(ports: Ports, backend: &str) -> serde_json::Value {
    let decisions = vec![
        accepted(ports.clone(), backend).await,
        fenced(ports.clone(), backend).await,
        deferred(ports.clone(), backend).await,
        throttled(ports.clone(), backend).await,
        recovered(ports.clone(), backend).await,
        flavor_mismatch(ports, backend).await,
    ];
    serde_json::json!({
        "producer_version": 1,
        "contract": "operator-control-outcomes",
        "scenario_inventory_version": 1,
        "backend": match backend {
            "in_memory" => "in-memory",
            "postgres" => "postgresql",
            other => other,
        },
        "decisions": decisions,
    })
}

pub(super) async fn in_memory() -> serde_json::Value {
    let core = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    all_outcomes(fixture::in_memory(&core), "in_memory").await
}

pub(super) async fn sqlite(pool: sqlx::SqlitePool) -> serde_json::Value {
    all_outcomes(fixture::sqlite(pool), "sqlite").await
}

pub(super) async fn postgres(pool: sqlx::PgPool) -> serde_json::Value {
    all_outcomes(fixture::postgres(pool), "postgres").await
}

pub(super) fn write_report(environment: &str, report: &serde_json::Value) {
    let Ok(path) = std::env::var(environment) else {
        return;
    };
    let path = std::path::Path::new(&path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let file = std::fs::File::create(path).expect("write actual operator observations");
    serde_json::to_writer_pretty(std::io::BufWriter::new(file), report).unwrap();
}

#[tokio::test]
async fn omitting_the_real_accepted_journal_witness_fails_the_gate() {
    let core = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let mut case = accepted(fixture::in_memory(&core), "in_memory").await;
    case.journal.clear();
    assert_omission_rejected(&case, "missing durable owner observation");
}

#[tokio::test]
async fn omitting_the_real_accepted_counter_witness_fails_the_gate() {
    let core = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let mut case = accepted(fixture::in_memory(&core), "in_memory").await;
    case.counters_after = case.counters_before.clone();
    assert_omission_rejected(&case, "missing independent operator metric");
}

#[tokio::test]
async fn omitting_the_real_accepted_span_witness_fails_the_gate() {
    let core = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let mut case = accepted(fixture::in_memory(&core), "in_memory").await;
    case.trace.clear();
    assert_omission_rejected(
        &case,
        "missing independently captured attributed operator span",
    );
}

#[tokio::test]
async fn unrelated_trace_with_matching_outcome_fields_cannot_replace_the_operator_span() {
    let core = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let mut case = accepted(fixture::in_memory(&core), "in_memory").await;
    for trace in &mut case.trace {
        trace.target = "unrelated::telemetry".to_owned();
        trace.name = "unrelated_action_event".to_owned();
    }
    assert_omission_rejected(
        &case,
        "missing independently captured attributed operator span",
    );
}

fn assert_omission_rejected(case: &CaseEvidence, expected: &str) {
    // The unmodified actual scenario was already verified before entering
    // this catch, so a broken producer cannot satisfy its own negative control.
    let failure =
        std::panic::catch_unwind(|| case.verify("in_memory", ExecutionControlOutcome::Accepted))
            .expect_err("removing a real required witness must fail the joint gate");
    let message = failure
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| failure.downcast_ref::<&str>().copied())
        .expect("gate assertion carries a readable failure");
    assert!(
        message.contains(expected),
        "gate failed for the wrong omitted surface: {message}"
    );
}

/// A handoff whose flavor-refusal observation always fails like a storage blip.
#[derive(Debug)]
struct FailingFlavorObservation(Arc<dyn nebula_storage_port::ExecutionTurnHandoff>);

#[async_trait::async_trait]
impl nebula_storage_port::ExecutionTurnHandoff for FailingFlavorObservation {
    async fn record_control_flavor_refusal(
        &self,
        _: &nebula_storage_port::store::ControlFlavorRefusal<'_>,
    ) -> Result<
        nebula_storage_port::store::ControlFlavorRefusalOutcome,
        nebula_storage_port::StorageError,
    > {
        Err(nebula_storage_port::StorageError::Connection(
            "injected transient observation failure".into(),
        ))
    }

    fn backend_kind(&self) -> nebula_storage_port::StorageBackendKind {
        self.0.backend_kind()
    }

    async fn commit_control_turn(
        &self,
        request: &nebula_storage_port::store::ControlTurnCommit<'_>,
    ) -> Result<
        nebula_storage_port::store::ControlTurnCommitOutcome,
        nebula_storage_port::StorageError,
    > {
        self.0.commit_control_turn(request).await
    }

    async fn accept_control_start(
        &self,
        request: &nebula_storage_port::store::ControlStartHandoff<'_>,
    ) -> Result<nebula_storage_port::store::ControlStartAcceptance, nebula_storage_port::StorageError>
    {
        self.0.accept_control_start(request).await
    }

    async fn accept_turn(
        &self,
        request: &nebula_storage_port::store::TurnHandoff<'_>,
    ) -> Result<nebula_storage_port::store::TurnAcceptance, nebula_storage_port::StorageError> {
        self.0.accept_turn(request).await
    }
}

/// Recording a flavor mismatch is an observation. A transient failure to
/// record it must not turn the exact-load rejection into a retriable
/// deferral that redelivers forever; it must be counted as unrecorded.
#[tokio::test]
async fn a_failed_flavor_observation_keeps_the_typed_rejection() {
    let core = Arc::new(nebula_storage::InMemoryExecutionStore::new());
    let ports = fixture::in_memory(&core);
    let calls = Arc::new(AtomicU32::new(0));
    let admitted = fixture::admit(ports.clone(), &calls, false).await;
    let registry = MetricsRegistry::new();
    let original = dispatch(&ports, engine(&ports, &calls, &registry, 0x74));
    let start = claim_start(&ports, &calls).await;
    assert!(matches!(
        original
            .dispatch_claimed_start(&admitted.scope, admitted.id, start)
            .await,
        nebula_engine::ClaimedControlDispatchOutcome::Accepted(Ok(()))
    ));
    let claim = claim_resume(&ports, &admitted, &calls).await;
    let mut faulty = ports.clone();
    faulty.handoff = Arc::new(FailingFlavorObservation(ports.handoff.clone()));
    let incompatible = dispatch(&faulty, engine(&faulty, &calls, &registry, 0x75));
    let decision = incompatible
        .dispatch_claimed_resume(&admitted.scope, admitted.id, None, claim)
        .await;
    // The wrong-flavor runtime defers the delivery to a runtime of the right
    // flavor with the exact-load reason. A failed observation must leave that
    // reason intact rather than replace it with a storage handoff error.
    assert!(
        matches!(
            &decision,
            nebula_engine::ClaimedControlDispatchOutcome::NotAccepted(Err(
                nebula_engine::ControlDispatchError::Deferred(reason)
            )) if reason.contains("not the requested exact worker flavor")
        ),
        "the exact-load rejection must win over the failed observation: {decision:?}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "wrong flavor never runs");
    let unrecorded = metrics::snapshot(&registry)
        .into_iter()
        .filter(|counter| {
            counter.name == "nebula_execution_control_observations_unrecorded_total"
                && counter.labels.get("outcome").map(String::as_str) == Some("flavor-mismatch")
        })
        .map(|counter| counter.value)
        .sum::<u64>();
    assert_eq!(unrecorded, 1, "the missing observation is counted");
}

/// Journal rows already present when a scenario starts observing.
async fn journal_len(ports: &Ports, admitted: &Admitted) -> usize {
    ports
        .stores
        .journal
        .get_journal(&admitted.scope, &admitted.id.to_string())
        .await
        .expect("read backend journal")
        .len()
}
