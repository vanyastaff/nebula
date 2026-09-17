//! Acceptance gate for the typed durable-failure envelope (#1016): a node's
//! failure must reach no durable or operator-facing surface carrying the failed
//! action's own text.
//!
//! A node failure used to be persisted as a free-text `String` built by walking
//! the error's `.source()` chain. `ActionError` forwards `Display` to whatever
//! `dyn Error` the action supplied (`ActionErrorSource`), so that walk published
//! provider text — and any secret the provider chose to quote — into
//! `executions.state`, `execution_journal`, OnError payloads, and every log line
//! that re-read them. The fix records a typed
//! `nebula_execution::ErrorEnvelope` whose message comes from the **top-level**
//! `Display` only (see `durable_error_envelope` in
//! `crates/engine/src/engine/mod.rs`).
//!
//! This is the end-to-end form of the unit-level gate in
//! `crates/engine/src/engine/tests.rs`
//! (`durable_failure_record_keeps_the_actions_own_text_out_of_every_surface`).
//! One store-backed, start-acceptance-entered run drives the failing node and is
//! asserted against the four surfaces a transport, a post-mortem read, or an
//! operator actually observes.
//!
//! The capture harness is the `CaptureBuf` + `tracing-subscriber` `MakeWriter`
//! shape from the established redaction gates (`crates/credential/tests/redaction.rs`,
//! `resource_rotation_wiring_redaction.rs`), installed with
//! `tracing::subscriber::set_global_default` because the frontier `tokio::spawn`s
//! node tasks and a thread-local subscriber would miss their events.

use std::{
    io::{self, Write},
    sync::{Arc, Mutex, OnceLock},
};

use nebula_action::{
    ActionError, ActionMetadataDraft, action::Action, result::ActionResult,
    stateless::StatelessAction,
};
use nebula_core::{Dependencies, NodeKey, action_key, id::ExecutionId, node_key};
use nebula_engine::{
    ActionRegistry, ActionRuntime, DataPassingPolicy, EngineError, InProcessRunner, RuntimeError,
    WorkflowEngine,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_storage::{InMemoryExecutionStore, InMemoryJournalReader};
use nebula_storage_port::store::{ExecutionJournalReader, ExecutionStore};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};
use tracing_subscriber::fmt::MakeWriter;

mod exact_fixture;

/// Provider text the failing action quotes. Long and structured so a substring
/// match cannot false-positive on unrelated engine output.
const MARKER: &str = "MARKER-9f3a-secret";

/// The single node whose action fails with [`MARKER`] behind its source chain.
fn leaking_node_key() -> NodeKey {
    node_key!("leaking_node")
}

/// Escaping and bounding are the envelope's job, so nothing in this file needs
/// to anticipate how a recorded message is encoded.
fn leaky_provider_error() -> EngineError {
    // The exact shape an in-flight action failure takes by the time the frontier
    // records it: `execute_action_with_node` returns the `RuntimeError`, which
    // its caller wraps as `EngineError::Runtime`.
    EngineError::Runtime(RuntimeError::ActionError(ActionError::fatal(format!(
        "provider rejected token {MARKER}"
    ))))
}

/// A stateless action that always fails with a provider payload.
///
/// `ActionError::fatal`'s own `Display` is a constant (`"fatal action failure"`);
/// the payload it was handed lands behind `#[source] error: ActionErrorSource`.
/// That source chain — not the top-level `Display` — is the channel a durable
/// record must never walk.
struct LeakyProviderAction;

impl Action for LeakyProviderAction {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadataDraft {
        ActionMetadataDraft::new(
            action_key!("core.leaky_provider"),
            nebula_action::metadata_name!("LeakyProvider"),
            "always fails quoting a provider payload",
        )
        .with_effect_contract(nebula_action::effect::ActionEffectContract::NoExternalEffects)
    }

    fn dependencies() -> &'static Dependencies {
        static DEPENDENCIES: OnceLock<Dependencies> = OnceLock::new();
        DEPENDENCIES.get_or_init(Dependencies::new)
    }
}

impl StatelessAction for LeakyProviderAction {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Err(ActionError::fatal(format!(
            "provider rejected token {MARKER}"
        )))
    }
}

// ── CaptureBuf (established shape, reused not re-invented) ────────────────

#[derive(Clone, Default)]
struct CaptureBuf(Arc<Mutex<Vec<u8>>>);

impl CaptureBuf {
    fn as_string(&self) -> String {
        let captured = self.0.lock().expect("capture buffer poisoned");
        String::from_utf8_lossy(&captured).into_owned()
    }
}

impl Write for CaptureBuf {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .expect("capture buffer poisoned")
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for CaptureBuf {
    type Writer = CaptureBuf;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The process-wide capture sink, with its subscriber installed on first use.
///
/// The frontier `tokio::spawn`s node tasks, so a thread-local `set_default`
/// subscriber would miss their events; `set_global_default` is the shape the
/// established redaction gates use for a spawned-task path.
///
/// `OnceLock` — rather than a bare install in the test body — keeps installation
/// idempotent, so the outcome does not depend on which test in this binary runs
/// first (`set_global_default` refuses a second subscriber) and a retried test
/// observes the same sink.
///
/// A refused subscriber fails here, at the refusal, rather than being reported
/// and carried on with: the sink is behind the `OnceLock`, so a refusal would
/// leave it empty for the whole process, and the run would then die on the
/// capture-is-real assertion with a message naming the wrong cause.
fn capture_sink() -> &'static CaptureBuf {
    static SINK: OnceLock<CaptureBuf> = OnceLock::new();

    SINK.get_or_init(|| {
        let sink = CaptureBuf::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(sink.clone())
            .with_ansi(false)
            .with_target(true)
            .with_level(true)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        if let Err(error) = tracing::subscriber::set_global_default(subscriber) {
            panic!(
                "failure_redaction: global capture subscriber refused ({error}); \
                 without it the sink captures nothing and the captured-surface \
                 assertions cannot hold — check for another test in this binary \
                 installing a global default"
            );
        }
        sink
    })
}

/// The absence gate every surface is checked with.
///
/// A shared helper, not a per-surface `assert!`, so the negative test below can
/// prove it fires on text that does carry the marker — an absence assertion that
/// cannot fail is not a gate.
fn assert_marker_absent(haystack: &str, surface: &str) {
    assert!(
        !haystack.contains(MARKER),
        "failure-envelope leak: marker {MARKER:?} reached {surface} \
         (case-sensitive):\n---- {surface} ----\n{haystack}\n----"
    );
}

// ── The durable fixture ───────────────────────────────────────────────────

/// A store-backed engine plus the handles the surfaces are read through.
struct RedactionFixture {
    engine: WorkflowEngine,
    /// Raw store handle: `ExecutionStore::get` is how the durable record is read.
    execution: Arc<InMemoryExecutionStore>,
    journal: Arc<InMemoryJournalReader>,
    /// The scope every store call is made under — `single_tenant_scope()` in
    /// production, and the one the engine drove the execution under.
    scope: nebula_storage_port::Scope,
    execution_id: ExecutionId,
    node_key: NodeKey,
}

/// Build the durable fixture and admit its start through real start acceptance.
///
/// `WorkflowEngine::execute_workflow` refuses to run when stores are attached, so
/// a store-backed run has to be entered the way production enters it: the exact
/// plan is compiled and installed into the flavor catalog, the state is
/// materialized through the `StartAcceptanceStore` (which acknowledges the Start
/// command), and the engine is then driven with `resume_execution`.
async fn leaky_provider_fixture() -> RedactionFixture {
    let registry = Arc::new(ActionRegistry::new());
    registry
        .register_stateless_instance(LeakyProviderAction::metadata(), LeakyProviderAction)
        .expect("valid test catalog definition");
    let frozen = exact_fixture::freeze_registry(&registry, &[("core", "core.leaky_provider")]);

    let execution = Arc::new(InMemoryExecutionStore::new());
    let journal = Arc::new(InMemoryJournalReader::new(&execution));
    let scope = nebula_engine::store_seam::single_tenant_scope();

    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            Arc::new(InProcessRunner::new()),
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .expect("test runtime builds"),
    );
    let engine = WorkflowEngine::new(runtime, metrics)
        .expect("test engine builds")
        .with_execution_stores(nebula_engine::ExecutionStores {
            execution: execution.clone(),
            journal: journal.clone(),
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
            resume_tokens: Arc::new(execution.resume_token_store()),
            operation_ledger: Arc::new(nebula_storage::inmem::InMemoryOperationLedger::new(
                &execution,
            )),
        })
        .with_plan_flavor_runtime(
            Arc::new(nebula_engine::PlanFlavorRevisionLoader::new(Arc::new(
                execution.plan_flavor_catalog(),
            ))),
            Arc::clone(&frozen),
            Arc::new(nebula_storage::inmem::InMemoryStartAcceptanceStore::new(
                &execution,
            )),
        );

    let node_key = leaking_node_key();
    let now = chrono::Utc::now();
    let workflow = WorkflowDefinition {
        id: nebula_core::id::WorkflowId::new(),
        name: "failure-redaction".to_owned(),
        description: None,
        version: Version::new(0, 1, 0),
        // No retry policy anywhere: the first failure is final, so the drive
        // takes the single-attempt failure path this gate asserts on.
        nodes: vec![
            NodeDefinition::new(
                node_key.clone(),
                "LeakyProvider",
                "core",
                "core.leaky_provider",
            )
            .expect("valid node definition"),
        ],
        connections: Vec::<Connection>::new(),
        variables: std::collections::HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: Vec::new(),
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };

    let execution_id = ExecutionId::new();
    let mut state = ExecutionState::new(execution_id, workflow.id, std::slice::from_ref(&node_key));
    state.set_workflow_input(serde_json::json!({"request": "redact-me"}));
    exact_fixture::materialize_state(&execution, &scope, &frozen, &workflow, &mut state).await;

    RedactionFixture {
        engine,
        execution,
        journal,
        scope,
        execution_id,
        node_key,
    }
}

// ── The gate ──────────────────────────────────────────────────────────────

/// One durable, acceptance-entered run whose failing action quotes [`MARKER`]
/// in its source chain, asserted against all four surfaces the failure text
/// travels to: durable state, the journal, the result projection a transport
/// serializes, and the captured tracing buffer.
#[tokio::test]
async fn durable_failure_redaction_keeps_the_marker_out_of_every_surface() {
    // The fixture has to actually expose the marker through the source chain, or
    // every assertion below would pass for the wrong reason.
    use std::error::Error as _;

    let fixture_source = leaky_provider_error();
    let mut sources = String::new();
    let mut current = fixture_source.source();
    while let Some(source) = current {
        sources.push_str(&source.to_string());
        current = source.source();
    }
    assert!(
        sources.contains(MARKER),
        "fixture must reach the marker through the source chain: {sources}"
    );

    let fixture = leaky_provider_fixture().await;
    let capturing = capture_sink();

    let result = fixture
        .engine
        .resume_execution(&fixture.scope, fixture.execution_id)
        .await
        .expect("the durable turn is admitted and driven");
    assert_eq!(
        result.status,
        ExecutionStatus::Failed,
        "the failing node must finalize the execution as Failed"
    );

    // 1. Durable state — `port_executions.state`, the row a post-mortem reads.
    let record = fixture
        .execution
        .get(&fixture.scope, &fixture.execution_id.to_string())
        .await
        .expect("execution row read succeeds")
        .expect("the driven execution has a durable row");
    assert_marker_absent(&record.state.to_string(), "durable execution state");

    // Non-vacuity: the record must carry the node's failure as a typed object,
    // not as a bare string. A row that recorded no failure would satisfy the
    // absence assertion above without the envelope ever being exercised.
    let node_state = record
        .state
        .pointer(&format!("/node_states/{}/state", fixture.node_key))
        .and_then(|state| state.as_str());
    assert_eq!(
        node_state,
        Some("failed"),
        "the failing node's durable state must be failed: {}",
        record.state
    );
    let failure_record = record
        .state
        .pointer(&format!("/node_states/{}/error_message", fixture.node_key))
        .unwrap_or_else(|| panic!("no durable failure record: {}", record.state));
    assert!(
        failure_record.is_object(),
        "the durable failure carrier is a typed record, not a bare string, got: {failure_record}"
    );
    assert_eq!(
        failure_record.get("code").and_then(|code| code.as_str()),
        // The fixture fails with `ActionError::fatal(...)`, wrapped in
        // `RuntimeError::ActionError`. The durable record must carry the
        // action's own code (`ACTION:FATAL`), not the constant
        // `RUNTIME:ACTION_ERROR` the wrapper's derive would otherwise shadow
        // it with.
        Some("ACTION:FATAL"),
        "the persisted record must carry the machine-readable code: {failure_record}"
    );

    // 2. Journal — `port_execution_journal.payload`.
    //
    // Forward guard, deliberately not a non-empty assertion: the engine appends
    // no journal rows yet (every `TransitionBatch::builder()` in
    // `crates/engine/src` omits `.journal(...)`, so `TransitionBatch::journal()`
    // hands back an empty slice), which makes this leg vacuously true today. It
    // becomes load-bearing when the journal gains its writer (#1013 adds entries
    // on this same shape). Asserting emptiness instead would fail for the wrong
    // reason the moment that writer lands, and asserting nothing at all would
    // leave the surface unguarded — so drain it, and hold every payload to the
    // same absence rule.
    let journal = fixture
        .journal
        .get_journal(&fixture.scope, &fixture.execution_id.to_string())
        .await
        .expect("journal read succeeds");
    let journal_payloads = journal
        .iter()
        .map(|entry| entry.payload.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert_marker_absent(&journal_payloads, "execution journal payloads");

    // 3. API body — `ExecutionResult.node_errors` is the projection a transport
    //    serializes into a response.
    let reported = result
        .node_errors
        .get(&fixture.node_key)
        .unwrap_or_else(|| {
            panic!(
                "the failed node must reach the result projection, got {:?}",
                result.node_errors
            )
        });
    assert!(
        !reported.is_empty(),
        "the reported failure must not be hollowed out"
    );
    assert!(
        reported.starts_with("ACTION:FATAL: "),
        "the reported failure must carry the envelope's `code: message` shape: {reported}"
    );
    assert_marker_absent(reported, "ExecutionResult.node_errors entry");

    // 4. Spans and events — the whole durable drive ran under the capture
    //    subscriber at TRACE, including the frontier's spawned node tasks.
    let captured = capturing.as_string();

    // Capture-is-real: a buffer that recorded nothing cannot pass. The failing
    // node's own checkpoint span and the drive's terminal record both have to be
    // present, so an uninstalled or empty subscriber fails loudly here.
    assert!(
        captured.contains("checkpoint_node_port"),
        "expected the failing node's checkpoint span in the capture — \
         capture-is-real guard, got:\n{captured}"
    );
    assert!(
        captured.contains("execution_finished"),
        "expected the drive's terminal record in the capture — \
         capture-is-real guard, got:\n{captured}"
    );
    assert_marker_absent(&captured, "captured tracing spans and events");
}

/// Load-bearing self-check: the absence assertion must fire on a string that
/// obviously carries the marker. Subscriber-free, so it holds whatever the gate
/// above does with the global capture sink.
#[test]
#[should_panic(expected = "failure-envelope leak")]
fn failure_redaction_assertion_is_load_bearing() {
    assert_marker_absent(&format!("provider rejected token {MARKER}"), "self-check");
}
