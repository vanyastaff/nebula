# Spec 20 — Testing story for action and trigger authors

> **Status:** draft
> **Canon target:** §4.4 DX pillar (fulfilment), new §12.12 testing contract
> **Depends on:** 08 (cancel), 09 (retry), 11 (triggers), 14 (stateful actions), 19 (error taxonomy)
> **Depended on by:** every plugin development, CI, contract tests for community plugins

## Problem

Canon §4.4 promises:

> *«Integration authoring is the product surface for contributors: fast scaffolding, test harnesses (`nebula-testing` and friends), actionable errors at API boundaries, **integration tests as the reference for how to ship a node**.»*

Without a shipped `nebula-testing` crate:

- Every plugin author invents their own mocking strategy (result: unmaintained zoo)
- Testing triggers is hard — cron needs time control, webhooks need HTTP mocks, queues need message simulation
- Stateful action tests can't exercise `WaitUntil` without waiting real wall clock
- No contract enforcement — community plugins may ship without proving idempotency / cancellation / PII safety
- Canon §13 «knife scenario» is normative but has no executable fixture

## Decision

**Three-tier test harness in a new `nebula-testing` workspace crate.** Adapters over standard Rust tools (`mockall`, `wiremock`, `proptest`, `tokio::test`), not a parallel framework. Time control via `tokio::time::pause` in Tier 1/2 and `TestClock` trait in Tier 3. Dedicated trigger testing helpers covering cron, webhook, event, polling with scenario replay. Canonical knife fixture for canon §13 compliance. Contract tests for plugin authors.

## Three-tier architecture

| Tier | Scope | Speed | What's real | What's mocked |
|---|---|---|---|---|
| **1 — Unit** | one `action.execute()` call | < 10 ms | action code | everything else (ctx, storage, clock, http) |
| **2 — Component** | one action through runtime wrapper | < 500 ms | runtime, retry, checkpoint, ephemeral SQLite | engine, API, trigger |
| **3 — Integration** | full workflow end-to-end | 1–10 s | engine, runtime, storage, eventbus, journal, metrics | external APIs, triggers injected by helper |

Authors pick tier based on what they need to prove. Most tests are **Tier 1**. Tier 2 for retry/checkpoint behavior. Tier 3 for canon §13 / integration bar compliance.

## Crate layout

```
crates/testing/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── unit/
│   │   ├── mod.rs
│   │   ├── context.rs        // ActionContextBuilder — Tier 1 primary
│   │   ├── credentials.rs    // MockCredentialAccessor
│   │   ├── resources.rs      // MockResourceAccessor
│   │   ├── cancellation.rs   // manual cancel control
│   │   └── metrics.rs        // in-memory metric capture
│   ├── component/
│   │   ├── mod.rs
│   │   ├── action_test.rs    // Tier 2 — ActionTest builder
│   │   ├── stateful_test.rs  // StatefulActionTest (run_until_waiting, advance_time, resume)
│   │   └── fixtures.rs
│   ├── integration/
│   │   ├── mod.rs
│   │   ├── environment.rs    // TestEnvironment — Tier 3 full stack
│   │   ├── workflow.rs       // WorkflowBuilder test helper
│   │   ├── clock.rs          // TestClock trait + injection
│   │   └── knife.rs          // canon §13 fixture
│   ├── triggers/
│   │   ├── mod.rs
│   │   ├── base.rs           // TriggerTest base
│   │   ├── cron.rs           // CronTriggerTest
│   │   ├── webhook.rs        // WebhookTriggerTest
│   │   ├── queue.rs          // QueueTriggerTest + MockQueue
│   │   └── polling.rs        // PollingTriggerTest + MockSource
│   ├── external/
│   │   ├── mod.rs
│   │   ├── http.rs           // wiremock wrapper with Nebula helpers
│   │   └── sql.rs            // ephemeral DB fixtures
│   ├── property/
│   │   ├── mod.rs
│   │   └── strategies.rs     // proptest strategies for Nebula types
│   ├── assertions/
│   │   ├── mod.rs
│   │   ├── execution.rs
│   │   ├── retry.rs
│   │   ├── checkpoint.rs
│   │   ├── events.rs
│   │   └── metrics.rs
│   └── contract/
│       ├── mod.rs
│       └── verify.rs         // verify_plugin contract suite
├── tests/
│   └── self_tests.rs         // testing the testing crate
```

**Depends on:** `nebula-action`, `nebula-runtime`, `nebula-engine`, `nebula-workflow`, `nebula-execution`, `nebula-storage` (test_support feature), `nebula-error`, `nebula-log`, `nebula-metrics`, `nebula-eventbus`. Plus external: `mockall`, `wiremock`, `proptest`, `tokio` test features, `insta` (optional for snapshots), `tempfile`.

**Published to crates.io** so out-of-workspace plugin authors can `[dev-dependencies] nebula-testing = "0.1"`.

---

## Tier 1 — Unit test primitives

### `ActionContextBuilder`

Construct a real `ActionContext` with mocks attached. Everything configurable, sane defaults.

```rust
// nebula-testing/src/unit/context.rs
use nebula_action::{ActionContext, AttemptId, ExecutionId};
use std::time::Duration;

pub struct ActionContextBuilder {
    execution_id: ExecutionId,
    attempt_id: AttemptId,
    attempt_number: u32,
    credentials: MockCredentialAccessor,
    resources: MockResourceAccessor,
    cancellation: CancellationController,
    http_client: Option<reqwest::Client>,
    vars: serde_json::Value,
    trigger_payload: serde_json::Value,
    node_outputs: HashMap<String, serde_json::Value>,
}

impl ActionContextBuilder {
    pub fn new() -> Self { /* defaults */ }
    
    pub fn with_execution_id(mut self, id: ExecutionId) -> Self { self.execution_id = id; self }
    pub fn with_attempt(mut self, n: u32) -> Self { self.attempt_number = n; self }
    
    pub fn with_credential(mut self, key: &str, value: impl Into<CredentialValue>) -> Self {
        self.credentials.insert(key, value.into());
        self
    }
    
    pub fn with_resource(mut self, key: &str, value: impl Into<ResourceHandle>) -> Self {
        self.resources.insert(key, value.into());
        self
    }
    
    pub fn with_http_client(mut self, client: reqwest::Client) -> Self {
        self.http_client = Some(client);
        self
    }
    
    pub fn with_trigger_payload(mut self, v: serde_json::Value) -> Self {
        self.trigger_payload = v;
        self
    }
    
    pub fn with_node_output(mut self, node_id: &str, output: serde_json::Value) -> Self {
        self.node_outputs.insert(node_id.into(), output);
        self
    }
    
    /// Construct ActionContext. Mocks wrap real trait impls.
    pub fn build(self) -> (ActionContext, TestHandle) {
        let ctx = ActionContext {
            execution_id: self.execution_id,
            attempt_id: self.attempt_id,
            attempt_number: self.attempt_number,
            cancellation: self.cancellation.signal(),
            credentials: Arc::new(self.credentials.clone()),
            resources: Arc::new(self.resources.clone()),
            http_client: self.http_client.unwrap_or_else(reqwest::Client::new),
            // ... etc
        };
        let handle = TestHandle {
            cancellation: self.cancellation,
            captured_metrics: Arc::new(Mutex::new(Vec::new())),
            captured_logs: Arc::new(Mutex::new(Vec::new())),
        };
        (ctx, handle)
    }
}

pub struct TestHandle {
    pub cancellation: CancellationController,
    captured_metrics: Arc<Mutex<Vec<MetricEvent>>>,
    captured_logs: Arc<Mutex<Vec<LogEntry>>>,
}

impl TestHandle {
    pub fn send_cancel(&self) {
        self.cancellation.cancel();
    }
    
    pub fn metrics(&self) -> Vec<MetricEvent> {
        self.captured_metrics.lock().unwrap().clone()
    }
    
    pub fn logs(&self) -> Vec<LogEntry> {
        self.captured_logs.lock().unwrap().clone()
    }
}
```

### Example — first test an author writes

```rust
// In the plugin crate
use nebula_testing::unit::ActionContextBuilder;
use my_plugin::actions::HttpGetAction;

#[tokio::test]
async fn http_get_returns_parsed_json() {
    // Mock HTTP server (wiremock)
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/users/42"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "id": 42,
            "name": "alice"
        })))
        .mount(&server)
        .await;
    
    // Build test context
    let (ctx, _handle) = ActionContextBuilder::new()
        .with_http_client(reqwest::Client::new())
        .build();
    
    // Run action
    let action = HttpGetAction;
    let input = HttpGetInput { url: format!("{}/users/42", server.uri()) };
    let output = action.execute(ctx, input).await.expect("action should succeed");
    
    // Assert
    assert_eq!(output.user_id, 42);
    assert_eq!(output.user_name, "alice");
}
```

**~30 lines for a simple action test.** Author doesn't need to think about storage, retry, cancellation — just the action's own logic.

### Testing cancellation (Tier 1)

```rust
#[tokio::test]
async fn slow_action_bails_on_cancel() {
    let (ctx, handle) = ActionContextBuilder::new().build();
    
    // Spawn action that sleeps, will see cancel via ctx.cancellation
    let action = SlowAction;
    let input = SlowInput { delay_ms: 10_000 };
    
    // Schedule cancel after short delay
    tokio::spawn({
        let handle = handle.clone();
        async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            handle.send_cancel();
        }
    });
    
    let result = action.execute(ctx, input).await;
    assert!(matches!(result, Err(ActionError::Cancelled)));
}
```

### Testing error classification

```rust
#[tokio::test]
async fn http_503_classified_as_transient() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(503))
        .mount(&server)
        .await;
    
    let (ctx, _) = ActionContextBuilder::new().build();
    let action = HttpGetAction;
    let input = HttpGetInput { url: server.uri() };
    
    let result = action.execute(ctx, input).await;
    match result {
        Err(ActionError::Transient(_)) => { /* ok */ }
        other => panic!("expected Transient, got {:?}", other),
    }
}
```

---

## Tier 2 — Component test harness

`ActionTest` runs **one action through the real runtime wrapper** — same code that runs in production, with ephemeral storage, real retry logic, real checkpoint handling.

### `ActionTest` builder

```rust
// nebula-testing/src/component/action_test.rs
use nebula_testing::integration::ephemeral_storage;

pub struct ActionTest<A: Action> {
    action: A,
    storage: Arc<dyn Storage>,
    metadata: ActionMetadata,
    retry_policy: RetryPolicy,
    injected_errors: Vec<ActionError>,  // for simulating failures
}

impl<A: Action> ActionTest<A> {
    pub async fn new(action: A) -> Self {
        let storage = ephemeral_storage().await;
        let metadata = action.metadata();
        Self {
            action,
            storage,
            metadata,
            retry_policy: RetryPolicy::default(),
            injected_errors: Vec::new(),
        }
    }
    
    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }
    
    /// Inject N failures before success (for retry testing)
    pub fn fail_n_times(mut self, n: u32, err: ActionError) -> Self {
        self.injected_errors = vec![err; n as usize];
        self
    }
    
    /// Run the action through runtime wrapper.
    /// Returns result + access to storage for assertions.
    pub async fn run(self, input: A::Input) -> ActionTestResult<A::Output> {
        // Construct ActionContext with ephemeral storage
        let execution_id = ExecutionId::new();
        let (ctx, _handle) = ActionContextBuilder::new()
            .with_execution_id(execution_id)
            .build();
        
        // Run via real runtime wrapper — includes retry logic
        let result = nebula_runtime::run_action_with_retry(
            &self.action,
            ctx,
            input,
            &self.retry_policy,
            &self.storage,
        ).await;
        
        ActionTestResult {
            result,
            execution_id,
            storage: self.storage,
        }
    }
}

pub struct ActionTestResult<O> {
    pub result: Result<O, NebulaError<RuntimeError>>,
    pub execution_id: ExecutionId,
    pub storage: Arc<dyn Storage>,
}

impl<O> ActionTestResult<O> {
    /// Count attempts for this execution in storage
    pub async fn attempt_count(&self) -> u32 {
        self.storage.count_attempts(self.execution_id).await.unwrap()
    }
    
    /// Get last attempt's final state
    pub async fn last_attempt(&self) -> Option<ExecutionNodeRow> {
        self.storage.latest_attempt(self.execution_id).await.unwrap()
    }
    
    /// Journal entries for this execution
    pub async fn journal(&self) -> Vec<JournalEntry> {
        self.storage.read_journal(self.execution_id).await.unwrap()
    }
}
```

### Example — retry test (Tier 2)

```rust
#[tokio::test]
async fn charge_customer_retries_on_transient() {
    let action = ChargeCustomerAction;
    let input = ChargeInput { customer_id: "cus_123", amount_cents: 1000 };
    
    let result = ActionTest::new(action).await
        .with_retry_policy(RetryPolicy {
            max_attempts: 3,
            backoff: BackoffStrategy::Fixed(Duration::from_millis(10)),
            retry_on: RetryClassifier::ClassifyBased,
            total_timeout: Some(Duration::from_secs(1)),
        })
        .fail_n_times(2, ActionError::Transient("network blip".into()))
        .run(input)
        .await;
    
    // Should succeed on 3rd attempt
    assert!(result.result.is_ok());
    assert_eq!(result.attempt_count().await, 3);
    
    // Verify journal captured all attempts
    let journal = result.journal().await;
    assert_eq!(journal.iter().filter(|e| matches!(e, JournalEntry::NodeFailed { .. })).count(), 2);
    assert_eq!(journal.iter().filter(|e| matches!(e, JournalEntry::NodeSucceeded { .. })).count(), 1);
}
```

### Example — retry budget exhausted

```rust
#[tokio::test]
async fn retry_exhausts_budget() {
    let result = ActionTest::new(FailingAction).await
        .with_retry_policy(RetryPolicy {
            max_attempts: 5,
            backoff: BackoffStrategy::Fixed(Duration::from_millis(10)),
            total_timeout: Some(Duration::from_millis(30)),  // will exhaust before 5 attempts
            retry_on: RetryClassifier::ClassifyBased,
        })
        .fail_n_times(10, ActionError::Transient("always fails".into()))
        .run(FailingInput)
        .await;
    
    assert!(result.result.is_err());
    match &result.result {
        Err(e) => {
            assert_eq!(e.code(), codes::RETRY_BUDGET_EXHAUSTED);
        }
        _ => panic!("expected error"),
    }
}
```

### `StatefulActionTest` — specialized for stateful

```rust
pub struct StatefulActionTest<A: StatefulAction> {
    inner: ActionTest<A>,
    // stateful-specific fields
}

impl<A: StatefulAction> StatefulActionTest<A> {
    pub async fn new(action: A) -> Self { /* ... */ }
    
    /// Run until first WaitUntil hit. Returns the condition to observe.
    pub async fn run_until_waiting(&mut self, input: A::Input) -> WaitCondition {
        // Runtime runs iterations until StepOutcome::WaitUntil
        // Returns the condition so test can inspect
    }
    
    /// Advance tokio test time to simulate wall clock pass.
    /// Only works when test uses `tokio::time::pause()`.
    pub async fn advance_time(&mut self, by: Duration) {
        tokio::time::advance(by).await;
    }
    
    /// Deliver a signal to wake the suspended action.
    pub async fn send_signal(&mut self, name: &str, payload: serde_json::Value) {
        self.inner.storage.insert_signal(
            self.inner.current_attempt_id(),
            name,
            payload,
        ).await.unwrap();
    }
    
    /// Resume iteration after wake condition met.
    pub async fn resume(&mut self) -> StatefulStepResult<A::Output> {
        // Runtime picks up the suspended attempt, resumes iteration loop
    }
    
    /// Read current state from checkpoint.
    pub async fn current_state(&self) -> A::State {
        self.inner.storage.load_state(self.inner.current_attempt_id()).await.unwrap()
    }
}
```

### Example — stateful action with WaitUntil

```rust
#[tokio::test]
async fn approval_action_resumes_on_signal() {
    tokio::time::pause();  // enable test time control
    
    let mut test = StatefulActionTest::new(ApprovalAction).await;
    let input = ApprovalInput { request_id: "req_01", approver_role: "manager" };
    
    // Run until action hits WaitUntil { Signal("approval_received", ..) }
    let condition = test.run_until_waiting(input).await;
    assert!(matches!(condition, WaitCondition::Signal { name, .. } if name == "approval_received"));
    
    // Check intermediate state
    let state = test.current_state().await;
    assert_eq!(state.phase, ApprovalPhase::WaitingForApproval);
    assert_eq!(state.requested_at.is_some(), true);
    
    // Simulate manager approving
    test.send_signal("approval_received", json!({
        "approved": true,
        "by": "user_manager_01"
    })).await;
    
    // Resume, expect completion
    let result = test.resume().await;
    match result {
        StatefulStepResult::Done(output) => {
            assert_eq!(output.status, "approved");
        }
        other => panic!("expected Done, got {:?}", other),
    }
}
```

### Example — crash recovery

```rust
#[tokio::test]
async fn stateful_resumes_from_checkpoint_after_crash() {
    tokio::time::pause();
    
    let mut test = StatefulActionTest::new(BatchProcessAction).await;
    let input = BatchInput { items: (0..100).collect() };
    
    // Run 25 iterations then simulate crash
    test.run_iterations(25).await;
    let state_before_crash = test.current_state().await;
    assert_eq!(state_before_crash.processed_count, 25);
    
    // Simulate crash: drop in-memory runtime, restart with same storage
    test.simulate_crash().await;
    
    // Resume — should read last checkpoint and continue
    let result = test.run_until_done().await;
    assert!(result.is_ok());
    
    // Verify final state
    let final_state = test.current_state().await;
    assert_eq!(final_state.processed_count, 100);
    
    // Verify iteration count in storage matches (some re-work due to replay between checkpoints)
    assert!(test.inner.attempt_count().await == 1);  // same attempt, resumed
}
```

---

## Tier 3 — Full integration environment

`TestEnvironment` spins up ephemeral full stack: SQLite in-memory, engine, runtime, eventbus, metrics, **without** API HTTP layer (tests call engine directly, not via HTTP).

### Environment

```rust
// nebula-testing/src/integration/environment.rs
pub struct TestEnvironment {
    storage: Arc<dyn Storage>,
    engine: Arc<WorkflowEngine>,
    eventbus: Arc<EventBus<ExecutionEvent>>,
    journal: Arc<dyn JournalReader>,
    metrics: Arc<TestMetricsRegistry>,
    clock: Arc<TestClock>,
    org_id: OrgId,
    workspace_id: WorkspaceId,
    user_id: UserId,
}

impl TestEnvironment {
    /// Fresh env with default org/workspace/user seeded.
    pub async fn ephemeral() -> Self {
        let storage = sqlite_memory().await;
        let clock = Arc::new(TestClock::new(default_start_time()));
        let engine = Arc::new(WorkflowEngine::new_with_clock(
            storage.clone(),
            clock.clone(),
        ));
        
        // Seed default tenant
        let (org_id, workspace_id, user_id) = seed_default_tenant(&storage).await;
        
        Self { storage, engine, /* ... */ }
    }
    
    /// Register plugin's actions.
    pub async fn register_plugin(&mut self, plugin: impl Plugin) {
        self.engine.register_plugin(plugin).await;
    }
    
    /// Submit a workflow definition, returns workflow_id.
    pub async fn submit_workflow(&mut self, def: WorkflowDefinition) -> WorkflowId {
        self.engine.create_workflow(self.tenant_ctx(), def).await.unwrap()
    }
    
    /// Create and publish in one call.
    pub async fn publish_workflow(&mut self, def: WorkflowDefinition) -> WorkflowId {
        let id = self.submit_workflow(def).await;
        self.engine.publish_workflow_latest(self.tenant_ctx(), id).await.unwrap();
        id
    }
    
    /// Start an execution manually (bypasses triggers).
    pub async fn start_execution(&self, workflow_id: WorkflowId, input: serde_json::Value) -> ExecutionId {
        self.engine.start_execution(self.tenant_ctx(), workflow_id, input).await.unwrap()
    }
    
    /// Drive the engine until execution reaches a terminal state.
    pub async fn wait_for_execution(&self, id: ExecutionId) -> ExecutionRow {
        self.engine.drive_until_terminal(id).await.unwrap()
    }
    
    /// Advance test clock.
    pub async fn advance_clock(&mut self, by: Duration) {
        self.clock.advance(by);
        tokio::time::advance(by).await;  // also advance tokio
    }
    
    /// Full tenant context for handler calls.
    pub fn tenant_ctx(&self) -> TenantContext {
        TenantContext {
            org_id: self.org_id,
            workspace_id: self.workspace_id,
            principal: Principal::User(self.user_id),
            /* roles set to Admin for tests */
        }
    }
    
    /// Access to journal for assertions.
    pub fn journal(&self) -> &dyn JournalReader {
        self.journal.as_ref()
    }
    
    /// Access to metric snapshot for assertions.
    pub fn metrics(&self) -> &TestMetricsRegistry {
        &self.metrics
    }
}
```

### `TestClock` trait

```rust
// nebula-testing/src/integration/clock.rs
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> { Utc::now() }
}

pub struct TestClock {
    now: AtomicI64,  // unix millis
}

impl TestClock {
    pub fn new(initial: DateTime<Utc>) -> Self {
        Self { now: AtomicI64::new(initial.timestamp_millis()) }
    }
    
    pub fn advance(&self, by: Duration) {
        self.now.fetch_add(by.as_millis() as i64, Ordering::SeqCst);
    }
    
    pub fn set(&self, instant: DateTime<Utc>) {
        self.now.store(instant.timestamp_millis(), Ordering::SeqCst);
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        DateTime::from_timestamp_millis(self.now.load(Ordering::SeqCst)).unwrap()
    }
}
```

Engine / scheduler read `Clock` trait through constructor injection. Production uses `SystemClock`, tests use `TestClock`.

**Key property:** cron scheduler, retry `wake_at`, `stateful_max_duration` all read `Clock::now()`. Test can advance hours in milliseconds.

### Example — full workflow test

```rust
#[tokio::test]
async fn full_order_workflow_succeeds() {
    tokio::time::pause();
    let mut env = TestEnvironment::ephemeral().await;
    
    // Register plugin
    env.register_plugin(OrderProcessingPlugin).await;
    
    // Mock external HTTP (Stripe, email)
    let stripe = env.mock_http_server().await;
    stripe.mock_charge_success("cus_123", 5000).await;
    let email = env.mock_http_server().await;
    email.mock_send_ok().await;
    
    // Define workflow
    let workflow = WorkflowBuilder::new("order_processing")
        .add_node("validate", "order.validate", json!({"order": "${trigger.payload}"}))
        .add_node("charge", "stripe.create_charge", json!({
            "customer": "${nodes.validate.output.customer_id}",
            "amount": "${nodes.validate.output.total}"
        }))
        .add_node("send_receipt", "email.send", json!({
            "to": "${nodes.validate.output.email}",
            "subject": "Your order",
            "body": "Order ${trigger.payload.id} confirmed"
        }))
        .add_edge("validate", "charge", EdgeCondition::OnSuccess)
        .add_edge("charge", "send_receipt", EdgeCondition::OnSuccess)
        .build();
    
    let wf_id = env.publish_workflow(workflow).await;
    
    // Start execution with input
    let exec_id = env.start_execution(wf_id, json!({
        "id": "order_42",
        "customer_id": "cus_123",
        "total": 5000,
        "email": "alice@example.com"
    })).await;
    
    // Drive to completion
    let final_row = env.wait_for_execution(exec_id).await;
    
    // Assertions
    assert_execution_succeeded!(env, exec_id);
    assert_node_succeeded!(env, exec_id, "validate");
    assert_node_succeeded!(env, exec_id, "charge");
    assert_node_succeeded!(env, exec_id, "send_receipt");
    
    // Verify external calls happened
    stripe.verify_charge_called("cus_123", 5000).await;
    email.verify_send_called("alice@example.com").await;
    
    // Verify metrics
    assert_metric_incremented!(env, "nebula_executions_succeeded_total", 1);
    assert_metric_incremented!(env, "nebula_action_succeeded_total", 3);  // 3 nodes
}
```

### Example — failure and recovery

```rust
#[tokio::test]
async fn workflow_retries_transient_then_succeeds() {
    tokio::time::pause();
    let mut env = TestEnvironment::ephemeral().await;
    env.register_plugin(OrderProcessingPlugin).await;
    
    // Mock: first 2 calls fail, 3rd succeeds
    let stripe = env.mock_http_server().await;
    stripe.mock_sequence(vec![
        Response::status(503),
        Response::status(503),
        Response::json(200, json!({"id": "ch_xyz"})),
    ]).await;
    
    let wf_id = env.publish_workflow(/* simple charge workflow */).await;
    let exec_id = env.start_execution(wf_id, /* input */).await;
    
    // Advance time to let retries fire (exponential backoff: 1s, 2s)
    env.advance_clock(Duration::from_secs(10)).await;
    env.wait_for_execution(exec_id).await;
    
    assert_execution_succeeded!(env, exec_id);
    
    // Verify 3 attempts captured
    let attempts = env.journal().node_attempts(exec_id, "charge").await;
    assert_eq!(attempts.len(), 3);
    assert_eq!(attempts[0].status, NodeStatus::Failed);
    assert_eq!(attempts[1].status, NodeStatus::Failed);
    assert_eq!(attempts[2].status, NodeStatus::Succeeded);
}
```

---

## Trigger testing

This is the specialized part. Each trigger type gets its own test harness layered over `TestEnvironment`.

### Base `TriggerTest`

```rust
// nebula-testing/src/triggers/base.rs
pub struct TriggerTest<T: TriggerAction> {
    env: TestEnvironment,
    trigger: T,
    trigger_id: TriggerId,
    workflow_id: WorkflowId,
}

impl<T: TriggerAction> TriggerTest<T> {
    /// Register trigger with a simple echo workflow that stores trigger payload.
    pub async fn new(trigger: T, config: T::Config) -> Self { /* ... */ }
    
    /// Register trigger with a specific workflow.
    pub async fn new_with_workflow(trigger: T, config: T::Config, workflow: WorkflowDefinition) -> Self { /* ... */ }
    
    /// Assert that an event with given event_id created exactly one execution.
    pub async fn assert_execution_created(&self, event_id: &str) -> ExecutionRow {
        let row = self.env.storage.find_execution_by_trigger_event(self.trigger_id, event_id)
            .await.unwrap().expect("no execution for event");
        row
    }
    
    /// Assert event was deduplicated (no new execution created).
    pub async fn assert_deduplicated(&self, event_id: &str) {
        let executions = self.env.storage.count_executions_by_trigger_event(self.trigger_id, event_id)
            .await.unwrap();
        assert_eq!(executions, 1, "expected exactly 1 execution for dedup'd event");
    }
    
    /// Check that no execution was created for an event.
    pub async fn assert_rejected(&self, event_id: &str) {
        let row = self.env.storage.find_execution_by_trigger_event(self.trigger_id, event_id)
            .await.unwrap();
        assert!(row.is_none(), "event should have been rejected");
    }
}
```

### Cron trigger testing

Cron needs **time control** — scheduler reads `Clock::now()`, tests advance the clock.

```rust
// nebula-testing/src/triggers/cron.rs
pub struct CronTriggerTest {
    base: TriggerTest<CronTrigger>,
    scheduler: CronScheduler,
}

impl CronTriggerTest {
    pub async fn new(config: CronTriggerConfig) -> Self {
        let mut base = TriggerTest::new_with_workflow(
            CronTrigger,
            config.clone(),
            echo_workflow(),
        ).await;
        
        // Inject scheduler that uses env's TestClock
        let scheduler = CronScheduler::new(
            base.env.engine.clone(),
            base.env.clock.clone(),
        );
        
        Self { base, scheduler }
    }
    
    /// Advance clock to specific time.
    pub async fn advance_to(&mut self, instant: DateTime<Utc>) {
        self.base.env.clock.set(instant);
        tokio::time::sleep(Duration::from_millis(1)).await;  // yield
    }
    
    /// Run scheduler tick — claims any due cron fire slots.
    pub async fn tick(&mut self) {
        self.scheduler.tick().await;
    }
    
    /// Assert that a fire at given time created an execution.
    pub async fn assert_fired_at(&self, scheduled_for: DateTime<Utc>) -> ExecutionRow {
        let claim = self.base.env.storage
            .find_cron_fire_slot(self.base.trigger_id, scheduled_for)
            .await.unwrap()
            .expect("no fire slot claimed");
        assert!(claim.execution_id.is_some(), "slot claimed but no execution");
        self.base.env.storage.load_execution(claim.execution_id.unwrap()).await.unwrap().unwrap()
    }
    
    /// Assert that a scheduled time was skipped (catch_up=Skip or overlap=Skip).
    pub async fn assert_skipped(&self, scheduled_for: DateTime<Utc>) {
        let claim = self.base.env.storage
            .find_cron_fire_slot(self.base.trigger_id, scheduled_for)
            .await.unwrap();
        // Either no claim, or claim exists but no execution (skipped due to overlap)
        if let Some(c) = claim {
            assert!(c.execution_id.is_none());
        }
    }
}
```

**Example — cron fires at scheduled time:**

```rust
#[tokio::test]
async fn cron_fires_daily_at_9am() {
    tokio::time::pause();
    
    let config = CronTriggerConfig {
        schedule: "0 9 * * *".into(),
        timezone: "UTC".into(),
        overlap_policy: OverlapPolicy::Skip,
        catch_up: CatchUpPolicy::Skip,
        jitter_seconds: 0,  // disable for deterministic test
        run_as: None,
    };
    
    let mut test = CronTriggerTest::new(config).await;
    
    // Start at 8:59:59
    test.advance_to(Utc.with_ymd_and_hms(2026, 4, 15, 8, 59, 59).unwrap()).await;
    test.tick().await;
    // Nothing fires yet
    test.assert_skipped(Utc.with_ymd_and_hms(2026, 4, 15, 9, 0, 0).unwrap()).await;
    
    // Advance to 9:00:00
    test.advance_to(Utc.with_ymd_and_hms(2026, 4, 15, 9, 0, 0).unwrap()).await;
    test.tick().await;
    
    // Fire slot claimed, execution created
    let row = test.assert_fired_at(Utc.with_ymd_and_hms(2026, 4, 15, 9, 0, 0).unwrap()).await;
    assert!(matches!(row.source, ExecutionSource::Cron { .. }));
}
```

**Example — catch_up=Skip ignores missed runs:**

```rust
#[tokio::test]
async fn cron_catch_up_skip_ignores_missed_runs() {
    tokio::time::pause();
    
    let config = CronTriggerConfig {
        schedule: "0 * * * *".into(),  // hourly
        catch_up: CatchUpPolicy::Skip,
        ..Default::default()
    };
    
    let mut test = CronTriggerTest::new(config).await;
    
    // Start at 10:00:00 — simulate process was down from 00:00 to 10:00
    test.advance_to(Utc.with_ymd_and_hms(2026, 4, 15, 10, 0, 0).unwrap()).await;
    test.tick().await;
    
    // Missed hours 00-09 should be SKIPPED, not fired
    for hour in 0..10 {
        let missed = Utc.with_ymd_and_hms(2026, 4, 15, hour, 0, 0).unwrap();
        test.assert_skipped(missed).await;
    }
    
    // Only 10:00 should fire
    test.assert_fired_at(Utc.with_ymd_and_hms(2026, 4, 15, 10, 0, 0).unwrap()).await;
}
```

**Example — catch_up=LatestOnly fires most recent missed:**

```rust
#[tokio::test]
async fn cron_catch_up_latest_only_fires_most_recent_missed() {
    tokio::time::pause();
    
    let config = CronTriggerConfig {
        schedule: "0 * * * *".into(),
        catch_up: CatchUpPolicy::LatestOnly,
        ..Default::default()
    };
    
    let mut test = CronTriggerTest::new(config).await;
    
    // Process came up at 10:30 after being down
    test.advance_to(Utc.with_ymd_and_hms(2026, 4, 15, 10, 30, 0).unwrap()).await;
    test.tick().await;
    
    // 00-08 skipped, 09 fired (most recent missed)
    for hour in 0..9 {
        test.assert_skipped(Utc.with_ymd_and_hms(2026, 4, 15, hour, 0, 0).unwrap()).await;
    }
    test.assert_fired_at(Utc.with_ymd_and_hms(2026, 4, 15, 9, 0, 0).unwrap()).await;
    // 10:00 hasn't passed yet from scheduler POV (it's 10:30 now, but 10:00 was before we started ticking)
}
```

**Example — leaderless claim (two schedulers, one wins):**

```rust
#[tokio::test]
async fn cron_leaderless_claim_prevents_double_fire() {
    tokio::time::pause();
    
    let config = CronTriggerConfig {
        schedule: "0 9 * * *".into(),
        ..Default::default()
    };
    
    let mut test = CronTriggerTest::new(config).await;
    
    // Simulate two scheduler instances
    let scheduler_a = test.spawn_second_scheduler().await;
    
    test.advance_to(Utc.with_ymd_and_hms(2026, 4, 15, 9, 0, 0).unwrap()).await;
    
    // Both tick concurrently
    let (_, _) = tokio::join!(
        test.tick(),
        scheduler_a.tick(),
    );
    
    // Only one execution should exist for this fire slot (unique constraint)
    let executions = test.base.env.storage
        .count_executions_for_trigger_at(test.base.trigger_id, Utc.with_ymd_and_hms(2026, 4, 15, 9, 0, 0).unwrap())
        .await.unwrap();
    assert_eq!(executions, 1);
}
```

### Webhook trigger testing

Webhooks need **HTTP request simulation** + **auth verification** testing.

```rust
// nebula-testing/src/triggers/webhook.rs
pub struct WebhookTriggerTest<T: WebhookAction> {
    base: TriggerTest<T>,
}

impl<T: WebhookAction> WebhookTriggerTest<T> {
    pub async fn new(trigger: T, config: T::Config) -> Self { /* ... */ }
    
    /// POST request to the trigger endpoint.
    pub async fn post(&mut self, body: serde_json::Value) -> WebhookResponse {
        self.post_with_headers(body, HeaderMap::new()).await
    }
    
    pub async fn post_with_headers(&mut self, body: serde_json::Value, headers: HeaderMap) -> WebhookResponse {
        let req = WebhookRequest {
            method: http::Method::POST,
            headers,
            body: serde_json::to_vec(&body).unwrap().into(),
            query_params: HashMap::new(),
            remote_addr: Some("127.0.0.1".parse().unwrap()),
        };
        self.base.env.dispatch_webhook(self.base.trigger_id, req).await
    }
    
    /// Helper: POST with HMAC-SHA256 signature header.
    pub async fn post_with_hmac(&mut self, body: serde_json::Value, secret: &str, header: &str) -> WebhookResponse {
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let sig = compute_hmac_sha256(&body_bytes, secret);
        let mut headers = HeaderMap::new();
        headers.insert(header, format!("sha256={}", hex::encode(sig)).parse().unwrap());
        self.post_with_headers(body, headers).await
    }
    
    /// Helper: POST with Stripe-Signature header.
    pub async fn post_with_stripe_signature(&mut self, body: serde_json::Value, secret: &str, at: DateTime<Utc>) -> WebhookResponse {
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let signed_payload = format!("{}.{}", at.timestamp(), String::from_utf8_lossy(&body_bytes));
        let sig = compute_hmac_sha256(signed_payload.as_bytes(), secret);
        let header_value = format!("t={},v1={}", at.timestamp(), hex::encode(sig));
        let mut headers = HeaderMap::new();
        headers.insert("stripe-signature", header_value.parse().unwrap());
        self.post_with_headers(body, headers).await
    }
}
```

**Example — webhook with HMAC auth:**

```rust
#[tokio::test]
async fn github_webhook_accepts_valid_signature() {
    let mut test = WebhookTriggerTest::new(
        GitHubWebhookAction,
        GitHubWebhookConfig {
            secret_ref: CredentialId::from_mock("github_secret"),
            // ...
        },
    ).await;
    
    test.env_mut().set_credential("github_secret", "my-webhook-secret").await;
    
    let body = json!({
        "action": "opened",
        "pull_request": { "number": 42, "title": "Fix bug" }
    });
    
    let response = test.post_with_hmac(body.clone(), "my-webhook-secret", "X-Hub-Signature-256").await;
    
    assert_eq!(response.status, http::StatusCode::ACCEPTED);  // 202
    
    // Verify event was accepted and execution created
    let event_id = extract_event_id_from_response(&response);
    test.base.assert_execution_created(&event_id).await;
}

#[tokio::test]
async fn github_webhook_rejects_bad_signature() {
    let mut test = WebhookTriggerTest::new(GitHubWebhookAction, /* ... */).await;
    test.env_mut().set_credential("github_secret", "my-webhook-secret").await;
    
    let body = json!({"action": "opened"});
    
    // Send with wrong secret
    let response = test.post_with_hmac(body, "wrong-secret", "X-Hub-Signature-256").await;
    
    assert_eq!(response.status, http::StatusCode::UNAUTHORIZED);
    
    // Verify NO event created
    assert_eq!(test.base.env.storage.count_trigger_events(test.base.trigger_id).await.unwrap(), 0);
}
```

**Example — dedup test:**

```rust
#[tokio::test]
async fn webhook_dedups_same_event_id() {
    let mut test = WebhookTriggerTest::new(/* */).await;
    
    let body = json!({"event": {"id": "evt_42", "data": "..."}});
    
    // First POST
    let resp1 = test.post(body.clone()).await;
    assert_eq!(resp1.status, http::StatusCode::ACCEPTED);
    
    // Same event_id, second POST
    let resp2 = test.post(body).await;
    assert_eq!(resp2.status, http::StatusCode::ACCEPTED);  // still accepts, dedups internally
    
    // Verify only ONE execution created
    test.base.assert_deduplicated("evt_42").await;
    
    // Verify dedup counter incremented
    assert_metric_incremented!(test.base.env, "nebula_trigger_events_deduplicated_total", 1);
}
```

**Example — replay attack rejected by timestamp tolerance:**

```rust
#[tokio::test]
async fn stripe_webhook_rejects_stale_timestamp() {
    let mut test = WebhookTriggerTest::new(StripeWebhookAction, /* */).await;
    test.env_mut().set_credential("stripe_secret", "whsec_test").await;
    
    let body = json!({"type": "charge.succeeded"});
    
    // Send with timestamp 10 minutes old (outside 5-minute tolerance)
    let old_time = Utc::now() - Duration::from_secs(600);
    let response = test.post_with_stripe_signature(body, "whsec_test", old_time).await;
    
    assert_eq!(response.status, http::StatusCode::BAD_REQUEST);
    
    // No event stored
    assert_eq!(test.base.env.storage.count_trigger_events(test.base.trigger_id).await.unwrap(), 0);
}
```

**Example — `AcknowledgeAndQueue` response mode:**

```rust
#[tokio::test]
async fn webhook_ack_mode_returns_202_immediately() {
    let mut test = WebhookTriggerTest::new(
        SlowProcessingWebhook,  // workflow takes 30s
        SlowConfig {
            response_mode: WebhookResponseMode::AcknowledgeAndQueue,
            ..Default::default()
        },
    ).await;
    
    let start = Instant::now();
    let response = test.post(json!({"event": {"id": "e1"}})).await;
    let elapsed = start.elapsed();
    
    // Should respond within 100ms even though workflow takes 30s
    assert!(elapsed < Duration::from_millis(100));
    assert_eq!(response.status, http::StatusCode::ACCEPTED);
    
    // Verify execution queued but not yet complete
    let row = test.base.assert_execution_created("e1").await;
    assert_eq!(row.status, ExecutionStatus::Pending);
}
```

### Queue trigger testing

Queue triggers need a **mock queue** implementation for controlled message delivery.

```rust
// nebula-testing/src/triggers/queue.rs
pub struct MockQueue {
    inner: Arc<Mutex<MockQueueState>>,
}

struct MockQueueState {
    messages: VecDeque<QueueMessage>,
    committed: HashSet<String>,
    in_flight: HashMap<String, QueueMessage>,
}

#[derive(Clone, Debug)]
pub struct QueueMessage {
    pub id: String,
    pub topic: String,
    pub partition: u32,
    pub offset: u64,
    pub payload: Vec<u8>,
    pub headers: HashMap<String, String>,
}

impl MockQueue {
    pub fn new() -> Self { /* ... */ }
    
    /// Push a message to the queue (test-driver side).
    pub async fn push(&self, msg: QueueMessage) {
        self.inner.lock().await.messages.push_back(msg);
    }
    
    /// Consumer side (plugin-facing).
    pub async fn consume(&self) -> Option<QueueMessage> {
        let mut state = self.inner.lock().await;
        let msg = state.messages.pop_front()?;
        state.in_flight.insert(msg.id.clone(), msg.clone());
        Some(msg)
    }
    
    /// Consumer commits offset.
    pub async fn commit(&self, msg_id: &str) -> Result<()> {
        let mut state = self.inner.lock().await;
        state.in_flight.remove(msg_id);
        state.committed.insert(msg_id.into());
        Ok(())
    }
    
    /// Test assertion — was this message committed?
    pub async fn is_committed(&self, msg_id: &str) -> bool {
        self.inner.lock().await.committed.contains(msg_id)
    }
    
    /// Test assertion — is message still in-flight (consumed but not committed)?
    pub async fn is_in_flight(&self, msg_id: &str) -> bool {
        self.inner.lock().await.in_flight.contains_key(msg_id)
    }
    
    /// Simulate consumer restart — messages not committed go back to queue.
    pub async fn simulate_restart(&self) {
        let mut state = self.inner.lock().await;
        let uncommitted: Vec<_> = state.in_flight.drain().map(|(_, m)| m).collect();
        for msg in uncommitted {
            state.messages.push_front(msg);
        }
    }
}

pub struct QueueTriggerTest<T: EventAction> {
    base: TriggerTest<T>,
    queue: MockQueue,
}

impl<T: EventAction> QueueTriggerTest<T> {
    /// Push a message and run consumer until idle.
    pub async fn push_and_drain(&mut self, msg: QueueMessage) {
        self.queue.push(msg).await;
        self.run_consumer_until_idle().await;
    }
    
    pub async fn run_consumer_until_idle(&mut self) { /* ... */ }
    
    pub fn queue(&self) -> &MockQueue { &self.queue }
}
```

**Example — queue consumer commits after emit:**

```rust
#[tokio::test]
async fn kafka_consumer_commits_after_dedup_check() {
    let mut test = QueueTriggerTest::new(KafkaEventAction).await;
    
    let msg = QueueMessage {
        id: "kafka:orders:0:42".into(),
        topic: "orders".into(),
        partition: 0,
        offset: 42,
        payload: serde_json::to_vec(&json!({"order_id": "ord_42"})).unwrap(),
        headers: HashMap::new(),
    };
    
    test.push_and_drain(msg.clone()).await;
    
    // Message consumed and committed
    assert!(test.queue().is_committed("kafka:orders:0:42").await);
    
    // Execution created
    test.base.assert_execution_created("kafka:orders:0:42").await;
}
```

**Example — crash recovery doesn't double-process:**

```rust
#[tokio::test]
async fn queue_consumer_recovers_without_double_processing() {
    let mut test = QueueTriggerTest::new(KafkaEventAction).await;
    
    let msg = QueueMessage {
        id: "kafka:orders:0:42".into(),
        // ...
    };
    
    // Emit happens but crash before commit
    test.queue().push(msg.clone()).await;
    test.run_consumer_until_emit_but_not_commit().await;
    assert!(test.queue().is_in_flight("kafka:orders:0:42").await);
    
    // Execution was created already
    test.base.assert_execution_created("kafka:orders:0:42").await;
    
    // Simulate restart
    test.queue().simulate_restart().await;
    
    // Resume consumer — message reappears, dedup prevents second execution
    test.run_consumer_until_idle().await;
    
    // Committed after successful dedup recognition
    assert!(test.queue().is_committed("kafka:orders:0:42").await);
    
    // STILL only one execution
    test.base.assert_deduplicated("kafka:orders:0:42").await;
    assert_metric_incremented!(test.base.env, "nebula_trigger_events_deduplicated_total", 1);
}
```

**Example — backpressure when quota exceeded:**

```rust
#[tokio::test]
async fn queue_consumer_pauses_on_quota_exceeded() {
    let mut test = QueueTriggerTest::new(KafkaEventAction).await;
    
    // Set low concurrent quota
    test.base.env.set_workspace_concurrent_limit(2).await;
    
    // Start 2 long-running executions (fills quota)
    for i in 0..2 {
        test.base.env.start_execution(/* long-workflow */, /* */).await;
    }
    
    // Push more messages
    for i in 0..5 {
        let msg = QueueMessage { id: format!("msg_{}", i), /* */ };
        test.queue().push(msg).await;
    }
    
    test.run_consumer_until_idle().await;
    
    // First 2 should be consumed but the rest stay in queue (backpressure)
    // NOT committed because emit returned QuotaExceeded
    for i in 0..5 {
        assert!(!test.queue().is_committed(&format!("msg_{}", i)).await);
    }
    
    // Metric showing backpressure
    assert_metric_incremented!(test.base.env, "nebula_trigger_backpressure_total", 1);
}
```

### Polling trigger testing

```rust
// nebula-testing/src/triggers/polling.rs
pub struct MockPollingSource<Item: Clone + Send + Sync> {
    batches: Arc<Mutex<VecDeque<Vec<Item>>>>,
    call_count: Arc<AtomicU32>,
}

impl<Item: Clone + Send + Sync> MockPollingSource<Item> {
    pub fn new() -> Self { /* */ }
    
    /// Enqueue a batch to return on next poll.
    pub async fn enqueue_batch(&self, items: Vec<Item>) {
        self.batches.lock().await.push_back(items);
    }
    
    pub fn call_count(&self) -> u32 {
        self.call_count.load(Ordering::SeqCst)
    }
}

pub struct PollingTriggerTest<T: PollingAction> {
    base: TriggerTest<T>,
    source: Arc<MockPollingSource<T::Item>>,
}

impl<T: PollingAction> PollingTriggerTest<T> {
    pub fn source(&self) -> &MockPollingSource<T::Item> { &self.source }
    
    /// Advance time and let polling fire.
    pub async fn advance_and_poll(&mut self, by: Duration) {
        self.base.env.advance_clock(by).await;
    }
}
```

**Example — polling respects interval:**

```rust
#[tokio::test]
async fn polling_fires_at_configured_interval() {
    tokio::time::pause();
    
    let mut test = PollingTriggerTest::new(
        NewEmailsTrigger,
        ImapConfig { /* poll_interval: 5 min */ },
    ).await;
    
    // Enqueue items to be returned
    test.source().enqueue_batch(vec![
        Email { message_id: "abc".into(), /* */ },
        Email { message_id: "def".into(), /* */ },
    ]).await;
    
    // Initial state: no polls yet
    assert_eq!(test.source().call_count(), 0);
    
    // Advance 5 minutes
    test.advance_and_poll(Duration::from_secs(300)).await;
    
    // One poll happened, 2 events created
    assert_eq!(test.source().call_count(), 1);
    test.base.assert_execution_created("imap:abc").await;
    test.base.assert_execution_created("imap:def").await;
}
```

---

## Retry and cancel testing (Tier 2)

Already covered in `ActionTest` examples above — `fail_n_times`, retry policy configuration, journal assertions.

### Manual cancel delivery

```rust
#[tokio::test]
async fn long_running_action_respects_cancel() {
    tokio::time::pause();
    
    let (ctx, handle) = ActionContextBuilder::new().build();
    let action = LongPollingAction;
    
    // Spawn action
    let task = tokio::spawn(action.execute(ctx, LongPollingInput {}));
    
    // Simulate 100ms elapsed
    tokio::time::advance(Duration::from_millis(100)).await;
    
    // Send cancel
    handle.send_cancel();
    
    // Task should return within cooperative grace
    let result = tokio::time::timeout(Duration::from_secs(1), task).await
        .expect("task should complete within 1s of cancel");
    
    assert!(matches!(result.unwrap(), Err(ActionError::Cancelled)));
}
```

### Cancel escalation

```rust
#[tokio::test]
async fn uncooperative_action_hard_killed() {
    tokio::time::pause();
    
    let result = ActionTest::new(BusyLoopAction).await
        .with_cancel_grace(Duration::from_millis(500))
        .run_and_cancel_after(Duration::from_millis(100))
        .await;
    
    // Action ignored cancel, runtime hard-killed it
    match &result.result {
        Err(e) => assert_eq!(e.code(), codes::ACTION_CANCELLED_ESCALATED),
        _ => panic!("expected escalation"),
    }
    
    // Journal has escalation marker
    let journal = result.journal().await;
    assert!(journal.iter().any(|e| matches!(e, JournalEntry::NodeCancelledEscalated { .. })));
}
```

---

## External API mocking

Wrap `wiremock` with Nebula-friendly helpers:

```rust
// nebula-testing/src/external/http.rs
pub struct MockHttpServer {
    inner: wiremock::MockServer,
}

impl MockHttpServer {
    pub async fn start() -> Self {
        Self { inner: wiremock::MockServer::start().await }
    }
    
    pub fn uri(&self) -> String { self.inner.uri() }
    
    /// Mock GET returning JSON.
    pub async fn mock_get_json(&self, path: &str, body: serde_json::Value) {
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(path))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&self.inner)
            .await;
    }
    
    /// Mock POST that returns a sequence of responses (for retry testing).
    pub async fn mock_sequence(&self, responses: Vec<wiremock::ResponseTemplate>) { /* ... */ }
    
    /// Verify a specific request was made.
    pub async fn verify_called(&self, method: &str, path: &str, times: u64) { /* ... */ }
}
```

### Stripe-specific helper

```rust
pub struct MockStripe {
    server: MockHttpServer,
}

impl MockStripe {
    pub async fn new() -> Self { /* */ }
    
    pub async fn mock_charge_success(&self, customer_id: &str, amount: u64) {
        self.server.mock_post_json(
            "/v1/charges",
            json!({
                "id": format!("ch_{}", random_id()),
                "customer": customer_id,
                "amount": amount,
                "status": "succeeded"
            })
        ).await;
    }
    
    pub async fn mock_charge_card_declined(&self) {
        self.server.mock_post_json_status(
            "/v1/charges",
            402,
            json!({
                "error": { "type": "card_error", "code": "card_declined" }
            })
        ).await;
    }
    
    pub async fn verify_charge_called(&self, customer_id: &str, amount: u64) { /* */ }
}
```

---

## Property testing patterns

Idempotency is the classic property to test:

```rust
// Property: running an action N times with the same idempotency key
// produces the same external side effect (via mock).
proptest! {
    #[test]
    fn idempotency_holds(attempts in 1..10u32) {
        tokio_test::block_on(async {
            let stripe = MockStripe::new().await;
            stripe.mock_charge_idempotent("cus_123", 1000).await;
            
            let action = ChargeAction;
            let input = ChargeInput { customer: "cus_123".into(), amount: 1000 };
            
            for _ in 0..attempts {
                let (ctx, _) = ActionContextBuilder::new()
                    .with_execution_id(ExecutionId::from_static("exec_fixed"))
                    .build();
                let _ = action.execute(ctx, input.clone()).await;
            }
            
            // External system should have seen N requests but only charged once
            // (because idempotency key was the same)
            stripe.verify_charge_count("cus_123", 1).await;
        });
    }
}
```

---

## Assertion library

Macros for common assertions, providing better error messages:

```rust
// nebula-testing/src/assertions/execution.rs
#[macro_export]
macro_rules! assert_execution_succeeded {
    ($env:expr, $exec_id:expr) => {{
        let row = $env.storage.load_execution($exec_id).await.unwrap().unwrap();
        assert_eq!(
            row.status,
            $crate::ExecutionStatus::Succeeded,
            "expected execution {} to be Succeeded, got {:?}; journal:\n{}",
            $exec_id, row.status, $env.journal().dump($exec_id).await
        );
    }};
}

#[macro_export]
macro_rules! assert_execution_failed {
    ($env:expr, $exec_id:expr $(, with_code = $code:expr)?) => {{
        let row = $env.storage.load_execution($exec_id).await.unwrap().unwrap();
        assert_eq!(row.status, $crate::ExecutionStatus::Failed);
        $(
            let final_error = $env.journal().final_error($exec_id).await;
            assert_eq!(final_error.code(), $code);
        )?
    }};
}

#[macro_export]
macro_rules! assert_node_succeeded {
    ($env:expr, $exec_id:expr, $logical_node:expr) => {{
        let attempts = $env.journal().node_attempts($exec_id, $logical_node).await;
        let last = attempts.last().expect("no attempts for node");
        assert_eq!(last.status, $crate::NodeStatus::Succeeded);
    }};
}

#[macro_export]
macro_rules! assert_node_retried {
    ($env:expr, $exec_id:expr, $logical_node:expr, times = $n:expr) => {{
        let attempts = $env.journal().node_attempts($exec_id, $logical_node).await;
        assert_eq!(
            attempts.len(), $n,
            "expected {} attempts for node {}, got {}",
            $n, $logical_node, attempts.len()
        );
    }};
}

#[macro_export]
macro_rules! assert_metric_incremented {
    ($env:expr, $name:expr, $delta:expr) => {{
        let current = $env.metrics().get_counter($name);
        assert!(
            current >= $delta,
            "expected metric {} to be incremented by at least {}, got {}",
            $name, $delta, current
        );
    }};
}

// Also available:
// assert_journal_contains!
// assert_audit_contains!
// assert_event_emitted!
// assert_checkpoint_at_iteration!
```

---

## Contract tests for plugin authors

Every plugin MUST prove these behaviors before merge into the registry:

```rust
// nebula-testing/src/contract/verify.rs
pub async fn verify_plugin_contract<P: Plugin>(plugin: P) -> ContractReport {
    let mut report = ContractReport::default();
    
    for action in plugin.actions() {
        report.merge(verify_action_idempotency(action.clone()).await);
        report.merge(verify_action_cancellation(action.clone()).await);
        report.merge(verify_action_no_panic_escape(action.clone()).await);
        report.merge(verify_action_no_credential_leak(action.clone()).await);
        report.merge(verify_action_error_classification(action.clone()).await);
    }
    
    for trigger in plugin.triggers() {
        report.merge(verify_trigger_dedup(trigger.clone()).await);
        report.merge(verify_trigger_auth(trigger.clone()).await);
    }
    
    report
}
```

Each verification function is a small test:

```rust
async fn verify_action_idempotency<A: Action>(action: A) -> ContractCheck {
    let (ctx, _) = ActionContextBuilder::new().build();
    let key_1 = ctx.idempotency_key();
    // Run twice with same context
    let _ = action.execute(ctx.clone(), /* canonical input */).await;
    let _ = action.execute(ctx.clone(), /* canonical input */).await;
    // Verify idempotency_key was referenced in external calls (via mock)
    /* ... */
}

async fn verify_action_cancellation<A: Action>(action: A) -> ContractCheck {
    // Send cancel immediately, verify action returns within 5 seconds
    /* ... */
}

async fn verify_action_no_credential_leak<A: Action>(action: A) -> ContractCheck {
    let (ctx, handle) = ActionContextBuilder::new()
        .with_credential("api_key", "SECRET_TOKEN_XYZ")
        .build();
    
    // Run action with a credential that has a distinctive marker
    let _ = action.execute(ctx, /* failing input */).await;
    
    // Check logs, metrics, returned errors — none should contain "SECRET_TOKEN_XYZ"
    let logs = handle.logs();
    for log in logs {
        assert!(!log.message.contains("SECRET_TOKEN_XYZ"),
            "credential leaked in log: {}", log.message);
    }
    /* ... */
}
```

Plugin authors run this as part of their test suite:

```rust
#[tokio::test]
async fn my_plugin_meets_contract() {
    let report = verify_plugin_contract(MyPlugin::new()).await;
    assert!(report.all_passed(), "contract violations: {}", report);
}
```

---

## Canon §13 knife fixture

Ship the canon knife scenario as a reusable test helper:

```rust
// nebula-testing/src/integration/knife.rs
pub async fn run_knife_scenario(env: &mut TestEnvironment) -> KnifeReport {
    let mut report = KnifeReport::default();
    
    // Step 1: define workflow
    let wf = WorkflowBuilder::knife_canonical();
    let wf_id = env.publish_workflow(wf).await;
    report.record("workflow_published");
    
    // Step 2: activate validated
    report.record("activation_validated");
    
    // Step 3: start execution
    let exec_id = env.start_execution(wf_id, json!({})).await;
    report.record("execution_started");
    
    // Step 4: GET execution, verify status + timing fields
    let row = env.storage.load_execution(exec_id).await.unwrap().unwrap();
    assert!(row.finished_at.is_none());
    assert!(row.started_at.is_some());
    report.record("initial_state_valid");
    
    // Step 5: cancel via durable queue
    env.engine.cancel_execution(exec_id).await.unwrap();
    report.record("cancel_enqueued");
    
    // Drive cancel cascade
    env.wait_for_execution(exec_id).await;
    
    // Verify terminal Cancelled
    let final_row = env.storage.load_execution(exec_id).await.unwrap().unwrap();
    assert_eq!(final_row.status, ExecutionStatus::Cancelled);
    report.record("cancel_reached_terminal");
    
    report
}

#[tokio::test]
async fn canon_knife_scenario_passes() {
    let mut env = TestEnvironment::ephemeral().await;
    let report = run_knife_scenario(&mut env).await;
    assert!(report.all_steps_passed());
}
```

**Every canon-touching PR runs this test.** If knife breaks, PR blocked.

---

## CI integration

Recommended CI layout (GitHub Actions example):

```yaml
jobs:
  test-unit:
    # Tier 1 only — < 30s
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo nextest run --workspace --lib
  
  test-component:
    # Tier 2 — < 2 min
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo nextest run --workspace --test '*_component'
  
  test-integration:
    # Tier 3 — 5-10 min, targeted suite
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo nextest run --workspace --test '*_integration'
  
  test-knife:
    # canon §13 fixture — must always pass
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - run: cargo nextest run -p nebula-testing --test knife
  
  test-nightly-full:
    # Full Tier 3 + property tests — nightly schedule
    runs-on: ubuntu-latest
    schedule: '0 3 * * *'
    steps:
      - uses: actions/checkout@v4
      - run: cargo nextest run --workspace --features property-tests
```

All layers run on every commit except nightly. Knife fixture is a merge gate.

---

## Coverage expectations

- **Plugin authors**: minimum **70% line coverage** on `execute()` body, 100% on `Classify` impl
- **Core crates** (`nebula-engine`, `nebula-runtime`, `nebula-storage`): minimum **80%** on public API
- **`nebula-testing` itself**: minimum **90%** (testing the testing crate)
- **`nebula-error`**: minimum **95%** (security-critical)
- **`nebula-credential`**: minimum **95%** (security-critical)

**Not gospel** — coverage is a signal, not a target. Tests that cover 100% of a weakly-typed function can still miss bugs. PR reviewers consider coverage + quality of assertions + contract tests.

---

## Configuration surface

```toml
[dependencies]
nebula-testing = "0.1"  # plugin authors use this

# Features
[features]
default = ["unit", "component"]
unit = []                   # Tier 1 helpers (no storage)
component = ["unit", "ephemeral-storage"]  # Tier 2 (SQLite memory)
integration = ["component", "full-engine"]  # Tier 3 (full stack)
property = ["dep:proptest"]  # Property testing strategies
contract = ["integration"]   # Contract verification suite
```

Authors opt into tiers they need. Tier 1 is lightweight (no storage deps), Tier 3 is heavier.

---

## Testing criteria (for this spec itself)

- `ActionContextBuilder` constructs valid `ActionContext` for every combination of mocks
- `ActionTest` correctly invokes retry logic matching spec 09
- `StatefulActionTest` correctly simulates WaitUntil / resume
- `TestEnvironment` correctly injects `TestClock` into engine/scheduler
- `CronTriggerTest` correctly simulates scheduling with time advance
- `WebhookTriggerTest` correctly verifies HMAC / Stripe signatures
- `QueueTriggerTest` correctly simulates consumer crash + recovery
- `MockQueue` never loses messages
- All assertion macros produce clear error messages when failing
- Contract verification catches plugins that don't meet requirements
- Knife fixture passes on clean environment

---

## Performance targets

- Tier 1 test: **< 10 ms** per test
- Tier 2 test: **< 500 ms** per test
- Tier 3 test: **< 5 seconds** per test (typical), **< 30 seconds** (complex workflow)
- `TestEnvironment::ephemeral()` startup: **< 200 ms**
- Knife scenario total: **< 3 seconds**
- CI suite (unit + component): **< 3 minutes** on a single runner
- CI suite (integration): **< 10 minutes** on a single runner

---

## Module boundaries

| Component | Crate |
|---|---|
| All test harnesses, mocks, assertions | `nebula-testing` (new) |
| `Clock` trait + `SystemClock` | `nebula-core` (production) |
| `TestClock` | `nebula-testing::integration::clock` |
| `ephemeral_storage()` helper | `nebula-storage::test_support` (existing) |
| Contract verification suite | `nebula-testing::contract` |
| Knife fixture | `nebula-testing::integration::knife` |
| Assertion macros | `nebula-testing::assertions` (re-exported at crate root) |

---

## Canon §12.12 new section — testing contract

Proposed wording:

```markdown
### 12.12 Testing contract

Plugin and integration authors MUST use `nebula-testing` harness for
verification. The three tiers are:

- **Unit** — `ActionContextBuilder` + mocks, no storage, < 10 ms per test
- **Component** — `ActionTest` or `StatefulActionTest`, ephemeral SQLite, real
  runtime wrappers, < 500 ms per test
- **Integration** — `TestEnvironment`, full stack, ephemeral engine + storage +
  eventbus, 1–10 s per test

Trigger testing uses specialized harnesses (`CronTriggerTest`, 
`WebhookTriggerTest`, `QueueTriggerTest`, `PollingTriggerTest`) with mock 
sources and `TestClock` for time control.

**Contract tests** (`verify_plugin_contract`) verify every action:
- Idempotency key used correctly
- Cancellation respected within 5 seconds
- No panic escapes action boundary
- No credentials leak into logs/errors
- Error classification correct per `Classify` trait

**Canon §13 knife scenario** is shipped as `run_knife_scenario(env)` and runs
on every canon-touching PR as a merge gate.

**Forbidden:**
- Testing with real external credentials (use mocks)
- Sleep-based time control (use `tokio::time::pause` or `TestClock`)
- Manual assertion logic without using `nebula-testing` macros for common cases
- Skipping contract verification for plugin crates
```

---

## Open questions

- **Real external integration tests** — some plugins need to test against real API (not mocks) in staging. Do we ship `testcontainers` integration for Postgres/Redis/Kafka? Deferred — for plugins that need it, they use `testcontainers` directly.
- **Snapshot testing with `insta`** — useful for workflow validation output, error messages. Optional dependency, recommended pattern, not required.
- **Golden file test for journal output** — compare `journal.dump()` against expected text. Valuable for regression testing, deferred as convention not enforcement.
- **Parallel test execution** — all tests should be `Send + Sync`, use ephemeral storage per test, parallel-safe. Document patterns, trust `cargo nextest` defaults.
- **Flake detection** — CI reruns flaky tests, surfaces flakiness metric. DevOps concern, deferred.
- **Test data generators** — library of factory functions (`test_user()`, `test_workflow()`). Useful convention, build up organically in `nebula-testing::fixtures`.
- **Performance regression tests** — use CodSpeed (already set up per memory). Separate from correctness tests.
- **Cross-platform testing** — SQLite on Windows has edge cases. CI matrix covers linux/macos/windows for core crates.
- **Integration with `nebula-telemetry` for test metric capture** — `TestMetricsRegistry` replaces production registry, captures all emissions. Design deferred to implementation.
