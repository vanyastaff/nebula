//! Integration tests for ADR-0099 **W-S3a** — targeted, kind-aware Resume.
//!
//! W-S2b shipped the resume machinery but it is UNTARGETED: a Resume arms every
//! signal-`Waiting` node of an execution. W-S3a persists each parked node's
//! resume-IDENTITY (`WaitSignal`) and threads a `ResumeTarget` through
//! `dispatch_resume` so a Resume arms ONLY the matching wait — and a webhook
//! Resume can NEVER satisfy an approval gate (the kind-confusion safety rule).
//!
//! These tests drive a store-backed engine so the durable satisfy-CAS written
//! by `satisfy_signal_waits` is observable, and assert on EFFECTS (which
//! downstream gate ran, which node stayed `Waiting`) — not implementation
//! details. Each has a falsifiability clause naming the regression it catches.
//!
//! ## Timing discipline
//!
//! `dispatch_start` / `dispatch_resume` both run the frontier loop synchronously
//! to completion (park or terminal), so assertions follow immediately — no
//! wall-clock sleeps.

use std::{
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
};

use chrono::Utc;
use nebula_action::{
    ActionError,
    action::Action,
    metadata::ActionMetadata,
    result::{ActionResult, WaitCondition},
    stateless::StatelessAction,
};
use nebula_core::{Dependencies, action_key, id::ExecutionId, node_key};
use nebula_engine::{
    ActionExecutor, ActionRegistry, ActionRuntime, ControlDispatch, DataPassingPolicy,
    EngineControlDispatch, InProcessRunner, ResumeTarget, WorkflowEngine,
};
use nebula_execution::{ExecutionState, ExecutionStatus};
use nebula_metrics::MetricsRegistry;
use nebula_storage::{InMemoryExecutionStore, InMemoryWorkflowVersionStore};
use nebula_storage_port::{
    dto::WorkflowVersionRecord,
    store::{ExecutionStore, WorkflowVersionStore},
};
use nebula_workflow::{
    CURRENT_SCHEMA_VERSION, Connection, NodeDefinition, Version, WorkflowConfig, WorkflowDefinition,
};

// ── Action stubs ──────────────────────────────────────────────────────────────

macro_rules! static_action_impl {
    ($ty:ty, $key:expr, $name:expr) => {
        impl Action for $ty {
            type Input = serde_json::Value;
            type Output = serde_json::Value;

            fn metadata() -> ActionMetadata {
                ActionMetadata::new($key, $name, "resume_targeting integration test stub")
            }
            fn dependencies() -> &'static Dependencies {
                static D: OnceLock<Dependencies> = OnceLock::new();
                D.get_or_init(Dependencies::new)
            }
        }
    };
}

/// Parks on `WaitCondition::Webhook { callback_id: "A" }` (no timeout) →
/// persists `WaitSignal::Webhook { callback_id: "A" }`.
struct WebhookWaitA;
static_action_impl!(
    WebhookWaitA,
    action_key!("test.target.webhook_a"),
    "WebhookWaitA"
);
impl StatelessAction for WebhookWaitA {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Webhook {
                callback_id: "A".to_owned(),
            },
            timeout: None,
            partial_output: None,
        })
    }
}

/// Parks on `WaitCondition::Webhook { callback_id: "B" }` (no timeout) →
/// persists `WaitSignal::Webhook { callback_id: "B" }`.
struct WebhookWaitB;
static_action_impl!(
    WebhookWaitB,
    action_key!("test.target.webhook_b"),
    "WebhookWaitB"
);
impl StatelessAction for WebhookWaitB {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Webhook {
                callback_id: "B".to_owned(),
            },
            timeout: None,
            partial_output: None,
        })
    }
}

/// Parks on `WaitCondition::Approval { approver: "boss", .. }` (no timeout) →
/// persists `WaitSignal::Approval { approver: "boss" }`. The approval `message`
/// is deliberately NOT persisted (that is W-S4).
struct ApprovalWaitBoss;
static_action_impl!(
    ApprovalWaitBoss,
    action_key!("test.target.approval_boss"),
    "ApprovalWaitBoss"
);
impl StatelessAction for ApprovalWaitBoss {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Approval {
                approver: "boss".to_owned(),
                message: "please approve".to_owned(),
            },
            timeout: None,
            partial_output: None,
        })
    }
}

/// Counts invocations and succeeds — a downstream gate probe. The test asserts
/// the exact invocation count to verify which wait's edge activated.
struct CountingEcho {
    invocation_count: Arc<AtomicU32>,
}
static_action_impl!(
    CountingEcho,
    action_key!("test.target.echo"),
    "CountingEcho"
);
impl StatelessAction for CountingEcho {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        self.invocation_count.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

// A second echo key so two independent waits can each gate their own probe.
struct CountingEchoB {
    invocation_count: Arc<AtomicU32>,
}
static_action_impl!(
    CountingEchoB,
    action_key!("test.target.echo_b"),
    "CountingEchoB"
);
impl StatelessAction for CountingEchoB {
    async fn execute(
        &self,
        input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        self.invocation_count.fetch_add(1, Ordering::SeqCst);
        Ok(ActionResult::success(input))
    }
}

// ── Stores ──────────────────────────────────────────────────────────────────

struct Stores {
    execution: Arc<InMemoryExecutionStore>,
    journal: Arc<nebula_storage::InMemoryJournalReader>,
    node_results: Arc<nebula_storage::InMemoryNodeResultStore>,
    checkpoints: Arc<nebula_storage::InMemoryCheckpointStore>,
    idempotency: Arc<nebula_storage::InMemoryIdempotencyGuard>,
    workflow: Arc<nebula_storage::InMemoryWorkflowStore>,
    versions: Arc<InMemoryWorkflowVersionStore>,
}

impl Stores {
    fn new() -> Self {
        let execution = Arc::new(InMemoryExecutionStore::new());
        let journal = Arc::new(nebula_storage::InMemoryJournalReader::new(&execution));
        let versions = InMemoryWorkflowVersionStore::new();
        let workflow = nebula_storage::InMemoryWorkflowStore::new_with_versions(&versions);
        Self {
            execution,
            journal,
            node_results: Arc::new(nebula_storage::InMemoryNodeResultStore::new()),
            checkpoints: Arc::new(nebula_storage::InMemoryCheckpointStore::new()),
            idempotency: Arc::new(nebula_storage::InMemoryIdempotencyGuard::new()),
            workflow: Arc::new(workflow),
            versions: Arc::new(versions),
        }
    }

    fn execution_stores(&self) -> nebula_engine::ExecutionStores {
        nebula_engine::ExecutionStores {
            execution: self.execution.clone(),
            journal: self.journal.clone(),
            node_results: self.node_results.clone(),
            checkpoints: self.checkpoints.clone(),
            idempotency: self.idempotency.clone(),
        }
    }

    fn workflow_stores(&self) -> nebula_engine::WorkflowStores {
        nebula_engine::WorkflowStores {
            workflow: self.workflow.clone(),
            versions: self.versions.clone(),
        }
    }

    fn attach(&self, engine: WorkflowEngine) -> WorkflowEngine {
        engine
            .with_execution_stores(self.execution_stores())
            .with_workflow_stores(self.workflow_stores())
    }

    async fn save_workflow(&self, wf: &WorkflowDefinition) {
        self.versions
            .create(
                &nebula_engine::store_seam::single_tenant_scope(),
                WorkflowVersionRecord {
                    workflow_id: wf.id.to_string(),
                    number: 0,
                    published: true,
                    pinned: false,
                    definition: serde_json::to_value(wf).unwrap(),
                },
            )
            .await
            .unwrap();
    }

    async fn create_execution(&self, workflow_id: nebula_core::WorkflowId) -> ExecutionId {
        let execution_id = ExecutionId::new();
        let mut exec_state = ExecutionState::new(execution_id, workflow_id, &[]);
        exec_state.set_workflow_input(serde_json::json!(null));
        let state_json = serde_json::to_value(&exec_state).unwrap();
        self.execution
            .create(
                &nebula_engine::store_seam::single_tenant_scope(),
                &execution_id.to_string(),
                &workflow_id.to_string(),
                state_json,
            )
            .await
            .unwrap();
        execution_id
    }

    async fn persisted_status(&self, execution_id: ExecutionId) -> ExecutionStatus {
        let record = self
            .execution
            .get(
                &nebula_engine::store_seam::single_tenant_scope(),
                &execution_id.to_string(),
            )
            .await
            .unwrap()
            .expect("execution row must exist");
        serde_json::from_value(record.state.get("status").cloned().unwrap()).unwrap()
    }
}

/// Wire an engine + dispatch over a fresh registry. The caller registers the
/// action keys it needs first.
fn build(registry: Arc<ActionRegistry>, stores: &Stores) -> EngineControlDispatch {
    let executor: ActionExecutor =
        Arc::new(|_ctx, _meta, input| Box::pin(async move { Ok(ActionResult::success(input)) }));
    let runner = Arc::new(InProcessRunner::new(executor));
    let metrics = MetricsRegistry::new();
    let runtime = Arc::new(
        ActionRuntime::try_new(
            registry,
            runner,
            DataPassingPolicy::default(),
            metrics.clone(),
        )
        .unwrap(),
    );
    let engine = Arc::new(stores.attach(WorkflowEngine::new(runtime, metrics).unwrap()));
    EngineControlDispatch::new(Arc::clone(&engine), stores.execution.clone())
}

/// Register a stateless action instance under the metadata its `Action` impl
/// declares — mirrors the `SignalHarness` registration pattern (the metadata's
/// key must match the `NodeDefinition` action ref string).
fn register_stateless<A>(registry: &ActionRegistry, action: A)
where
    A: StatelessAction + 'static,
{
    registry.register_stateless_instance(A::metadata(), action);
}

/// Two independent root waits, each gating its own downstream echo:
/// `wait_a → echo_a` and `wait_b → echo_b`. Returns `(workflow_id)`.
async fn two_webhook_subgraphs(
    stores: &Stores,
    echo_a: Arc<AtomicU32>,
    echo_b: Arc<AtomicU32>,
) -> (Arc<ActionRegistry>, nebula_core::WorkflowId) {
    let registry = Arc::new(ActionRegistry::new());
    register_stateless(&registry, WebhookWaitA);
    register_stateless(&registry, WebhookWaitB);
    register_stateless(
        &registry,
        CountingEcho {
            invocation_count: echo_a,
        },
    );
    register_stateless(
        &registry,
        CountingEchoB {
            invocation_count: echo_b,
        },
    );

    let wait_a = node_key!("wait_a");
    let wait_b = node_key!("wait_b");
    let downstream_a = node_key!("echo_a");
    let downstream_b = node_key!("echo_b");
    let workflow_id = nebula_core::WorkflowId::new();
    let now = Utc::now();
    let wf = WorkflowDefinition {
        id: workflow_id,
        name: "two-webhook-subgraphs".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(wait_a.clone(), "WaitA", "core", "test.target.webhook_a").unwrap(),
            NodeDefinition::new(wait_b.clone(), "WaitB", "core", "test.target.webhook_b").unwrap(),
            NodeDefinition::new(downstream_a.clone(), "EchoA", "core", "test.target.echo").unwrap(),
            NodeDefinition::new(downstream_b.clone(), "EchoB", "core", "test.target.echo_b")
                .unwrap(),
        ],
        connections: vec![
            Connection::new(wait_a, downstream_a),
            Connection::new(wait_b, downstream_b),
        ],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: Vec::new(),
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };
    stores.save_workflow(&wf).await;
    (registry, workflow_id)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

/// **W-S3a — a targeted Resume arms ONLY the matching callback_id.**
///
/// Two independent webhook waits in one execution (`wait_a` cb="A" → `echo_a`,
/// `wait_b` cb="B" → `echo_b`). A Resume targeting `Webhook { callback_id: "A" }`
/// must arm ONLY `wait_a`: `echo_a` runs exactly once, `echo_b` stays 0, and the
/// execution stays `Paused` because `wait_b` is still parked.
///
/// **Falsifiability**: revert the identity match (make the arm predicate ignore
/// the target and arm every signal wait, as W-S2b did) → BOTH waits arm →
/// `echo_b == 1` and the execution Completes → the `echo_b == 0` / `Paused`
/// assertions fail → RED.
#[tokio::test]
async fn targeted_resume_arms_only_matching_callback() {
    let echo_a = Arc::new(AtomicU32::new(0));
    let echo_b = Arc::new(AtomicU32::new(0));
    let stores = Stores::new();
    let (registry, workflow_id) =
        two_webhook_subgraphs(&stores, Arc::clone(&echo_a), Arc::clone(&echo_b)).await;
    let dispatch = build(registry, &stores);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let execution_id = stores.create_execution(workflow_id).await;

    // Park both waits → execution Paused.
    dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park both webhook waits");
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must be Paused after both signal nodes park"
    );

    // Targeted Resume for callback_id "A" — arms ONLY wait_a.
    dispatch
        .dispatch_resume(
            &scope,
            execution_id,
            Some(ResumeTarget::Webhook {
                callback_id: "A".to_owned(),
            }),
        )
        .await
        .expect("targeted dispatch_resume must satisfy only the matching wait");

    assert_eq!(
        echo_a.load(Ordering::SeqCst),
        1,
        "echo_a must run exactly once — its wait was the target"
    );
    assert_eq!(
        echo_b.load(Ordering::SeqCst),
        0,
        "echo_b must NOT run — its wait (cb=B) was not targeted"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must stay Paused — wait_b is still parked"
    );

    // Now target "B": arms wait_b, runs echo_b, and the execution completes.
    dispatch
        .dispatch_resume(
            &scope,
            execution_id,
            Some(ResumeTarget::Webhook {
                callback_id: "B".to_owned(),
            }),
        )
        .await
        .expect("second targeted dispatch_resume must satisfy the remaining wait");

    assert_eq!(
        echo_b.load(Ordering::SeqCst),
        1,
        "echo_b must run exactly once after its callback is targeted"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must Complete once both waits have been targeted"
    );
}

/// **W-S3a — a webhook Resume must NOT satisfy an approval gate (KIND-CONFUSION).**
///
/// One subgraph parks a `Webhook { callback_id: "boss" }` wait (→ `echo_a`); the
/// other parks an `Approval { approver: "boss" }` gate (→ `echo_b`). The
/// identities deliberately COLLIDE ("boss" == "boss"), so the only thing that
/// can keep a webhook Resume from satisfying the approval gate is the KIND
/// discriminator. A Resume targeting `Webhook { callback_id: "boss" }` must arm
/// ONLY the webhook wait and must NEVER touch the approval gate.
///
/// This is the load-bearing security invariant: a webhook callback must not be
/// able to forge a human approval — even when the labels coincide.
///
/// **Falsifiability**: revert the kind-awareness (match on the bare identity
/// string, ignoring the variant kind) → the colliding "boss" identity makes the
/// webhook Resume also arm the approval gate → `echo_b == 1` → the
/// `echo_b == 0` assertion fails → RED.
#[tokio::test]
async fn webhook_resume_does_not_satisfy_approval_gate() {
    let echo_a = Arc::new(AtomicU32::new(0));
    let echo_b = Arc::new(AtomicU32::new(0));
    let stores = Stores::new();

    let registry = Arc::new(ActionRegistry::new());
    register_stateless(&registry, WebhookWaitX);
    register_stateless(&registry, ApprovalWaitBoss);
    register_stateless(
        &registry,
        CountingEcho {
            invocation_count: Arc::clone(&echo_a),
        },
    );
    register_stateless(
        &registry,
        CountingEchoB {
            invocation_count: Arc::clone(&echo_b),
        },
    );

    let wait_webhook = node_key!("wait_webhook");
    let wait_approval = node_key!("wait_approval");
    let downstream_a = node_key!("echo_a");
    let downstream_b = node_key!("echo_b");
    let workflow_id = nebula_core::WorkflowId::new();
    let now = Utc::now();
    let wf = WorkflowDefinition {
        id: workflow_id,
        name: "webhook-vs-approval".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(
                wait_webhook.clone(),
                "WaitWebhook",
                "core",
                "test.target.webhook_a",
            )
            .unwrap(),
            NodeDefinition::new(
                wait_approval.clone(),
                "WaitApproval",
                "core",
                "test.target.approval_boss",
            )
            .unwrap(),
            NodeDefinition::new(downstream_a.clone(), "EchoA", "core", "test.target.echo").unwrap(),
            NodeDefinition::new(downstream_b.clone(), "EchoB", "core", "test.target.echo_b")
                .unwrap(),
        ],
        connections: vec![
            Connection::new(wait_webhook, downstream_a),
            Connection::new(wait_approval, downstream_b),
        ],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: Vec::new(),
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };
    stores.save_workflow(&wf).await;

    let dispatch = build(registry, &stores);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let execution_id = stores.create_execution(workflow_id).await;

    dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the webhook wait and the approval gate");
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must be Paused with both signal nodes parked"
    );

    // A webhook Resume for "boss" must arm ONLY the webhook wait, never the
    // approval gate — despite the colliding "boss" identity.
    dispatch
        .dispatch_resume(
            &scope,
            execution_id,
            Some(ResumeTarget::Webhook {
                callback_id: "boss".to_owned(),
            }),
        )
        .await
        .expect("webhook-targeted dispatch_resume must satisfy only the webhook wait");

    assert_eq!(
        echo_a.load(Ordering::SeqCst),
        1,
        "the webhook wait's downstream must run exactly once"
    );
    assert_eq!(
        echo_b.load(Ordering::SeqCst),
        0,
        "the APPROVAL gate's downstream must NOT run — a webhook Resume must never \
         satisfy an approval gate (kind-confusion safety rule)"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must stay Paused — the approval gate is still awaiting approval"
    );
}

/// **W-S3a — an untargeted Resume preserves the W-S2b all-arm behavior.**
///
/// With `resume_target == None`, a Resume arms EVERY signal-`Waiting` node — so
/// both independent webhook waits complete in one pass and the execution runs to
/// `Completed` with both downstream probes fired exactly once.
///
/// **Falsifiability**: make `None` arm nothing (or only the first node) → one
/// wait stays `Waiting` → the execution stays `Paused` and at least one
/// `echo == 1` assertion fails → RED.
#[tokio::test]
async fn untargeted_resume_keeps_legacy_all_arm() {
    let echo_a = Arc::new(AtomicU32::new(0));
    let echo_b = Arc::new(AtomicU32::new(0));
    let stores = Stores::new();
    let (registry, workflow_id) =
        two_webhook_subgraphs(&stores, Arc::clone(&echo_a), Arc::clone(&echo_b)).await;
    let dispatch = build(registry, &stores);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let execution_id = stores.create_execution(workflow_id).await;

    dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park both webhook waits");
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Paused
    );

    // Untargeted Resume — arms ALL signal waits (W-S2b behavior).
    dispatch
        .dispatch_resume(&scope, execution_id, None)
        .await
        .expect("untargeted dispatch_resume must satisfy every signal wait");

    assert_eq!(
        echo_a.load(Ordering::SeqCst),
        1,
        "echo_a must run — an untargeted Resume arms every signal wait"
    );
    assert_eq!(
        echo_b.load(Ordering::SeqCst),
        1,
        "echo_b must run — an untargeted Resume arms every signal wait"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must Complete — both waits armed in one untargeted pass"
    );
}

/// A `Webhook { callback_id: "boss" }` wait whose callback_id deliberately
/// COLLIDES with the approval gate's `approver` ("boss"). This makes the
/// kind-confusion test's safety guard depend on the KIND discriminator alone:
/// a kind-blind matcher comparing only the identity string would wrongly arm
/// the approval gate. Reuses the `webhook_a` action key so the registry stays
/// small (only one webhook action is registered per test).
struct WebhookWaitX;
static_action_impl!(
    WebhookWaitX,
    action_key!("test.target.webhook_a"),
    "WebhookWaitX"
);
impl StatelessAction for WebhookWaitX {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Webhook {
                callback_id: "boss".to_owned(),
            },
            timeout: None,
            partial_output: None,
        })
    }
}

/// Parks on `WaitCondition::Execution { execution_id }` (no timeout), where
/// `execution_id` is supplied at construction time. Used by the Execution-kind
/// targeting tests so two sibling waits can park on DIFFERENT `ExecutionId`s.
struct ExecutionWait {
    wait_for: ExecutionId,
}
impl Action for ExecutionWait {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.target.execution_wait"),
            "ExecutionWait",
            "resume_targeting Execution-kind stub",
        )
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}
impl StatelessAction for ExecutionWait {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Execution {
                execution_id: self.wait_for,
            },
            timeout: None,
            partial_output: None,
        })
    }
}

/// A second independent Execution-kind wait action key so the workflow DAG
/// can have two `ExecutionWait` nodes without key collision.
struct ExecutionWaitB {
    wait_for: ExecutionId,
}
impl Action for ExecutionWaitB {
    type Input = serde_json::Value;
    type Output = serde_json::Value;

    fn metadata() -> ActionMetadata {
        ActionMetadata::new(
            action_key!("test.target.execution_wait_b"),
            "ExecutionWaitB",
            "resume_targeting Execution-kind stub B",
        )
    }
    fn dependencies() -> &'static Dependencies {
        static D: OnceLock<Dependencies> = OnceLock::new();
        D.get_or_init(Dependencies::new)
    }
}
impl StatelessAction for ExecutionWaitB {
    async fn execute(
        &self,
        _input: <Self as Action>::Input,
        _ctx: &(impl nebula_action::ActionContext + ?Sized),
    ) -> Result<ActionResult<<Self as Action>::Output>, ActionError> {
        Ok(ActionResult::Wait {
            condition: WaitCondition::Execution {
                execution_id: self.wait_for,
            },
            timeout: None,
            partial_output: None,
        })
    }
}

// ── Execution-kind targeting tests ────────────────────────────────────────────

/// **W-S3a — a targeted Resume arms ONLY the matching `ExecutionId`.**
///
/// Two independent `Execution`-kind waits in one execution: `wait_x` parks on
/// `execution_id = X` (→ `echo_a`) and `wait_y` parks on `execution_id = Y`
/// (→ `echo_b`). A Resume targeting `Execution { execution_id: X.to_string() }`
/// must arm ONLY `wait_x`: `echo_a` runs exactly once, `echo_b` stays 0, and
/// the execution stays `Paused` because `wait_y` is still parked.
///
/// **Falsifiability**: revert the identity check in the `Execution` arm of
/// `matches_resume_target` to always return `true` (kind-only, ignoring the
/// id string) → both waits arm → `echo_b == 1` and the execution Completes →
/// the `echo_b == 0` / `Paused` assertions fail → RED.
#[tokio::test]
async fn targeted_resume_arms_only_matching_execution() {
    let exec_id_x = ExecutionId::new();
    let exec_id_y = ExecutionId::new();
    let echo_a = Arc::new(AtomicU32::new(0));
    let echo_b = Arc::new(AtomicU32::new(0));
    let stores = Stores::new();

    let registry = Arc::new(ActionRegistry::new());
    // Two sibling waits on DIFFERENT ExecutionIds — DAG: wait_x→echo_a, wait_y→echo_b.
    registry.register_stateless_instance(
        ExecutionWait::metadata(),
        ExecutionWait {
            wait_for: exec_id_x,
        },
    );
    registry.register_stateless_instance(
        ExecutionWaitB::metadata(),
        ExecutionWaitB {
            wait_for: exec_id_y,
        },
    );
    register_stateless(
        &registry,
        CountingEcho {
            invocation_count: Arc::clone(&echo_a),
        },
    );
    register_stateless(
        &registry,
        CountingEchoB {
            invocation_count: Arc::clone(&echo_b),
        },
    );

    let wait_x = node_key!("wait_x");
    let wait_y = node_key!("wait_y");
    let downstream_a = node_key!("echo_a");
    let downstream_b = node_key!("echo_b");
    let workflow_id = nebula_core::WorkflowId::new();
    let now = Utc::now();
    let wf = WorkflowDefinition {
        id: workflow_id,
        name: "two-execution-waits".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(
                wait_x.clone(),
                "WaitX",
                "core",
                "test.target.execution_wait",
            )
            .unwrap(),
            NodeDefinition::new(
                wait_y.clone(),
                "WaitY",
                "core",
                "test.target.execution_wait_b",
            )
            .unwrap(),
            NodeDefinition::new(downstream_a.clone(), "EchoA", "core", "test.target.echo").unwrap(),
            NodeDefinition::new(downstream_b.clone(), "EchoB", "core", "test.target.echo_b")
                .unwrap(),
        ],
        connections: vec![
            Connection::new(wait_x, downstream_a),
            Connection::new(wait_y, downstream_b),
        ],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: Vec::new(),
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };
    stores.save_workflow(&wf).await;

    let dispatch = build(registry, &stores);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let execution_id = stores.create_execution(workflow_id).await;

    // Park both Execution-kind waits.
    dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park both execution-kind waits");
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must be Paused after both execution-kind waits park"
    );

    // Resume targeting ONLY execution_id X — must arm wait_x, never wait_y.
    dispatch
        .dispatch_resume(
            &scope,
            execution_id,
            Some(ResumeTarget::Execution {
                execution_id: exec_id_x.to_string(),
            }),
        )
        .await
        .expect("targeted dispatch_resume must satisfy only the X execution wait");

    assert_eq!(
        echo_a.load(Ordering::SeqCst),
        1,
        "echo_a must run exactly once — wait_x (id=X) was the target"
    );
    assert_eq!(
        echo_b.load(Ordering::SeqCst),
        0,
        "echo_b must NOT run — wait_y (id=Y) was not targeted by the X Resume"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must stay Paused — wait_y is still parked"
    );

    // Resume targeting Y — arms wait_y, runs echo_b, execution Completes.
    dispatch
        .dispatch_resume(
            &scope,
            execution_id,
            Some(ResumeTarget::Execution {
                execution_id: exec_id_y.to_string(),
            }),
        )
        .await
        .expect("second targeted dispatch_resume must satisfy the remaining wait");

    assert_eq!(
        echo_b.load(Ordering::SeqCst),
        1,
        "echo_b must run exactly once after wait_y is targeted"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Completed,
        "execution must Complete once both Execution-kind waits have been targeted"
    );
}

/// **W-S3a — a malformed `execution_id` string in the target never matches.**
///
/// A node parks on a valid `Execution { execution_id: X }` wait. A Resume is
/// sent whose `ResumeTarget::Execution { execution_id }` carries a string that
/// does NOT parse as a valid `ExecutionId` (e.g. `"not-a-valid-uuid"`). The
/// `matches_resume_target` Execution arm calls `want.parse::<ExecutionId>()`
/// which returns `Err` → `is_ok_and(...)` returns `false` → the node stays
/// `Waiting` and the execution stays `Paused`. No panic, no spurious arm.
///
/// **Falsifiability**: remove the `parse::<ExecutionId>()` type gate and
/// compare raw strings (`execution_id.to_string() == *want`) → the malformed
/// string never equals the valid id, so this test would actually still pass —
/// BUT the clippy `cmp_owned` lint fires and (more importantly) a *different*
/// well-formed string that happens to share the same text but represents a
/// different semantic type would match. The real invariant tested here is that
/// the `Err` path of `parse` closes silently (no panic, no match).
#[tokio::test]
async fn execution_resume_parse_fail_never_matches() {
    let exec_id_real = ExecutionId::new();
    let echo_a = Arc::new(AtomicU32::new(0));
    let stores = Stores::new();

    let registry = Arc::new(ActionRegistry::new());
    registry.register_stateless_instance(
        ExecutionWait::metadata(),
        ExecutionWait {
            wait_for: exec_id_real,
        },
    );
    register_stateless(
        &registry,
        CountingEcho {
            invocation_count: Arc::clone(&echo_a),
        },
    );

    let wait_node = node_key!("wait_exec");
    let echo_node = node_key!("echo_a");
    let workflow_id = nebula_core::WorkflowId::new();
    let now = Utc::now();
    let wf = WorkflowDefinition {
        id: workflow_id,
        name: "execution-parse-fail".into(),
        description: None,
        version: Version::new(0, 1, 0),
        nodes: vec![
            NodeDefinition::new(
                wait_node.clone(),
                "WaitExec",
                "core",
                "test.target.execution_wait",
            )
            .unwrap(),
            NodeDefinition::new(echo_node.clone(), "Echo", "core", "test.target.echo").unwrap(),
        ],
        connections: vec![Connection::new(wait_node, echo_node)],
        variables: HashMap::new(),
        config: WorkflowConfig::default(),
        trigger_bindings: Vec::new(),
        tags: Vec::new(),
        created_at: now,
        updated_at: now,
        owner_id: None,
        ui_metadata: None,
        schema_version: CURRENT_SCHEMA_VERSION,
    };
    stores.save_workflow(&wf).await;

    let dispatch = build(registry, &stores);
    let scope = nebula_engine::store_seam::single_tenant_scope();
    let execution_id = stores.create_execution(workflow_id).await;

    dispatch
        .dispatch_start(&scope, execution_id)
        .await
        .expect("dispatch_start must park the execution-kind wait");
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must be Paused after the execution-kind wait parks"
    );

    // Resume with a string that does NOT parse as a valid ExecutionId.
    dispatch
        .dispatch_resume(
            &scope,
            execution_id,
            Some(ResumeTarget::Execution {
                execution_id: "not-a-valid-uuid".to_owned(),
            }),
        )
        .await
        .expect("dispatch_resume must not error — a parse-fail simply does not match");

    // The parse-fail must close silently: no arm, no panic.
    assert_eq!(
        echo_a.load(Ordering::SeqCst),
        0,
        "echo must NOT run — the malformed target id must never match the parked wait"
    );
    assert_eq!(
        stores.persisted_status(execution_id).await,
        ExecutionStatus::Paused,
        "execution must stay Paused — the malformed target left the wait intact"
    );
}
