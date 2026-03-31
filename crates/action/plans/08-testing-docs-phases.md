# Testing, Documentation, Module Layout, Phases, and Migration

## Testing Infrastructure

Behind `test-support` feature flag. Single-node scope only.

### TestContext

```rust
pub struct TestContext {
    ctx: ActionContext,
    resources: HashMap<String, Box<dyn Any + Send + Sync>>,
    credentials: HashMap<String, CredentialSnapshot>,
    logs: Arc<Mutex<Vec<LogEntry>>>,
    heartbeat_count: Arc<AtomicU64>,
}

impl TestContext {
    pub fn new() -> Self { ... }
    pub fn inject_resource<T: Send + Sync + 'static>(&mut self, key: &str, resource: T) { ... }
    pub fn inject_credential(&mut self, id: &str, snapshot: CredentialSnapshot) { ... }
    pub fn action_context(&self) -> &ActionContext { &self.ctx }
    pub fn logs(&self) -> Vec<LogEntry> { ... }
    pub fn heartbeat_count(&self) -> u64 { ... }
}
```

### StatefulTestHarness

```rust
pub struct StatefulTestHarness<A: StatefulAction> {
    action: A,
    ctx: TestContext,
    state: Option<A::State>,
}

impl<A: StatefulAction> StatefulTestHarness<A> {
    pub fn new(action: A) -> Self { ... }
    pub async fn step(&mut self, input: A::Input) -> Result<ActionResult<A::Output>, ActionError> { ... }
    pub fn seed_state(&mut self, state: A::State) { ... }
    pub fn current_state(&self) -> Option<&A::State> { ... }
}
```

### TriggerTestHarness

```rust
pub struct TriggerTestHarness<A: TriggerAction> {
    action: A,
    ctx: TriggerContext,
    state: Option<A::State>,
}

impl<A: TriggerAction> TriggerTestHarness<A> {
    pub fn new(action: A) -> Self { ... }
    pub fn seed_state(&mut self, state: A::State) { ... }
    pub async fn start(&mut self) -> Result<TriggerStartMode, ActionError> { ... }
    pub async fn run(&mut self) -> Result<TriggerCompletion, ActionError> { ... }
    pub async fn stop(&mut self) -> Result<(), ActionError> { ... }
    pub async fn health_check(&self) -> Result<TriggerHealth, ActionError> { ... }
    pub fn checkpointed_state(&self) -> Option<&serde_json::Value> { ... }
    pub fn emitted_executions(&self) -> &[serde_json::Value] { ... }
}
```

### WebhookTestRequest

```rust
pub struct WebhookTestRequest {
    path: String,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl WebhookTestRequest {
    pub fn new(path: &str) -> Self { ... }
    pub fn header(mut self, key: &str, value: &str) -> Self { ... }
    pub fn json_body(mut self, body: &serde_json::Value) -> Self { ... }
    pub fn raw_body(mut self, body: Vec<u8>) -> Self { ... }
    pub fn sign_hmac_sha256(mut self, secret: &[u8], header_name: &str) -> Self { ... }
}

pub async fn test_webhook<A: WebhookAction>(
    action: &A, req: WebhookTestRequest, ctx: &TriggerContext,
) -> Result<WebhookResponse, ActionError> { ... }
```

### Assertion macros

```rust
assert_success!(result);
assert_success!(result, output == expected);
assert_skip!(result);
assert_retryable!(error);
assert_fatal!(error);
assert_validation!(error);
assert_continue!(result);
assert_wait!(result);
```

### test_node! (extended with mock block)

```rust
test_node!(
    http_tests,
    HttpRequest::new(),
    mock {
        http: MockHttpClient::new()
            .when_get("https://example.com")
            .respond_json(200, json!({"ok": true})),
        api_token: BearerToken::new("test-token"),
    },
    happy_path: json!({"url": "https://example.com"}) => is_success,
    empty_url: json!({"url": ""}) => is_validation_error,
);
```

---

## Documentation and Onboarding

### "Choose your trait" decision tree

```
What does your node do?

Pure data transform (no I/O)?
└── TransformAction

One-shot operation with side effects?
├── Simple → SimpleAction
└── Needs flow control → StatelessAction

Iterative / multi-step?
├── Paginate API → PaginatedAction
├── Batch process → BatchAction
├── Human approval → InteractiveAction
├── Saga/transaction → TransactionalAction
└── Custom logic → StatefulAction

Incoming events?
├── From managed resource (WS, Kafka, Redis) → EventTrigger
├── HTTP webhook → WebhookAction / RawWebhookAction
│   (lifecycle: check_exists → on_activate → handle_request → on_deactivate)
├── Periodic poll → PollAction
├── Schedule (cron/interval/one-time) → ScheduledTrigger
└── Custom (composite, exotic) → TriggerAction

Scoped resource for downstream?
└── ResourceAction
```

### 5 canonical examples (must-ship)

1. HTTP JSON API (SimpleAction + ActionDeps + ResultActionExt)
2. PostgreSQL Query (StatelessAction + credential)
3. Slack Message (ActionInput with dynamic options_loader + visibility)
4. API Pagination (PaginatedAction)
5. GitHub Webhook (WebhookAction + HMAC-SHA256)

### Error classification guide

| Situation | Error | ErrorCode |
|-----------|-------|-----------|
| Network timeout | `Retryable` | `UpstreamTimeout` |
| HTTP 429 | `Retryable` | `RateLimited` |
| HTTP 500-503 | `Retryable` | `UpstreamUnavailable` |
| Invalid user input | `Validation` | — |
| Bad response schema | `Fatal` | — |
| Expired auth token | `Retryable` | `AuthExpired` |
| Undeclared resource | `SandboxViolation` | — |
| Action panicked | `Fatal` | `ActionPanicked` |

### Credential rotation guidance

> `credential_typed()` returns a valid snapshot for the current access window.
> Authors should NOT implement refresh loops. For long-running actions with
> Continue iterations, reacquire credential at iteration boundary.
> Credential layer handles refresh transparently.

---

## Module Layout

```
nebula-action/src/
├── lib.rs
├── action.rs                   // Action base trait
├── metadata.rs                 // ActionMetadata, RetryPolicy, TimeoutPolicy
├── components.rs               // ActionComponents, CredentialRef, ResourceRef
├── descriptor.rs               // ActionDescriptor, ActionKind
├── factory.rs                  // ActionFactory, ActionBuildError
├── instance.rs                 // ActionInstance enum
│
├── execution/                  // Core traits
│   ├── stateless.rs
│   ├── stateful.rs             // PersistedState, NextPersistedState, StateMigrationError
│   ├── trigger.rs              // TriggerStartMode, TriggerCompletion
│   └── resource.rs             // ReleaseOutcome
│
├── dx/                         // DX types (blanket impls to core)
│   ├── simple.rs               // → StatelessAction
│   ├── transform.rs            // → SimpleAction → StatelessAction
│   ├── paginated.rs            // → StatefulAction
│   ├── batch.rs                // → StatefulAction
│   ├── interactive.rs          // → StatefulAction (InteractionHandle, epoch)
│   ├── transactional.rs        // → StatefulAction
│   ├── event_trigger.rs        // → TriggerAction (EventSource bridge)
│   ├── scheduled.rs            // → TriggerAction (cron, interval, one-time)
│   ├── webhook.rs              // → TriggerAction (JSON, State, lifecycle hooks)
│   ├── raw_webhook.rs          // → TriggerAction (non-JSON)
│   └── poll.rs                 // → TriggerAction (NextPoll)
│
├── result.rs                   // ActionResult, WaitCondition, try_map_value
├── output.rs                   // ActionOutput, BinaryData
├── error.rs                    // ActionError, ErrorCode, ParameterBindingError
│
├── context/
│   ├── action_ctx.rs           // ActionContext (slim: resource, credential, port_data, support_data, call_action)
│   ├── trigger_ctx.rs          // TriggerContext (resources, params, checkpoint)
│   └── guard.rs                // ExecutionGuard
│
├── core_resources/             // Core resource trait definitions (impl in nebula-runtime)
│   ├── mod.rs                  // re-exports
│   ├── binary_storage.rs       // BinaryStorage trait, BinaryData
│   └── stream_output.rs        // StreamOutput trait, StreamSender, StreamChunk
│
├── capability/
│   ├── manifest.rs             // CapabilityManifest
│   ├── resource.rs             // ResourceAccessor
│   ├── credential.rs           // CredentialAccessor
│   ├── scheduler.rs            // TriggerScheduler (schedule_at, unschedule)
│   ├── emitter.rs              // ExecutionEmitter (emit_and_checkpoint, batch, status)
│   ├── parameter.rs            // ParameterProvider
│   └── action_executor.rs      // ActionExecutor
│
├── handler/
│   ├── stateless.rs            // StatelessHandler, StatelessAdapter
│   ├── stateful.rs             // StatefulHandler, StatefulHandlerResult, StatefulAdapter
│   ├── trigger.rs              // TriggerHandler, TriggerStartResult
│   └── resource.rs             // ResourceHandler
│
├── registry.rs                 // ActionRegistry, RegistrationError
├── port.rs                     // InputPort, OutputPort, SupportPort, DynamicPort, FlowKind, ConnectionFilter
├── data_tag.rs                 // DataTag, DataTagRegistry, DataTagInfo, TagRegistrationError, namespace validation
│
├── authoring/
│   ├── errors.rs               // ResultActionExt, ensure!
│   ├── patterns.rs             // Pagination/webhook helpers
│   └── helpers.rs
│
├── testing/
│   ├── test_context.rs
│   ├── stateful_harness.rs
│   ├── trigger_harness.rs
│   ├── webhook_test.rs
│   ├── mock_resource.rs
│   ├── mock_credential.rs
│   └── macros.rs               // test_node!, assert_*
│
└── prelude.rs                  // Common imports
```

---

## Implementation Phases

| Phase | Content | Estimate |
|-------|---------|----------|
| 1 | Core types: Descriptor (with InterfaceVersion), Factory, Instance, Manifest, Guard | 4 days |
| 2 | Handler traits (4), StatefulHandlerResult, TriggerStartResult | 1 week |
| 3 | Adapters (4): try_map_value, pre-execute snapshot | 5 days |
| 4 | StatefulAction: init_state, PersistedState, state migration | 4 days |
| 5 | **TriggerAction: start/run split, TriggerStartMode, TriggerCompletion** | 4 days |
| 6 | **TriggerContext: resources, params, schedule_at, emit_and_checkpoint, batch, status** | 3 days |
| 7 | ResourceAction: acquire/release, ScopedResourceMap | 3 days |
| 8 | ActionContext: slim (resource_typed, credential_typed, port_data, support_data, call_action, heartbeat) | 3 days |
| 9 | **WaitCondition enum, Core resources: BinaryStorage + BinaryData, StreamOutput + StreamSender** | 4 days |
| 10 | DX: Simple, Transform, Paginated, Batch, Interactive | 1 week |
| 11 | **Trigger DX: EventTrigger, WebhookAction+State+Lifecycle, RawWebhook, PollAction+NextPoll, ScheduledTrigger** | 1 week |
| 12 | Error model: ErrorCode, ResultActionExt, ensure!, string constructors | 3 days |
| 13 | RetryPolicy, TimeoutPolicy, WebhookResponse, OutputEnvelope | 2 days |
| 14 | **ActionRegistry: VersionedActionKey, version-aware lookup, batch, catalog, DeprecationInfo** | 4 days |
| 15 | **Versioning: ActionMigration trait, migrate_parameters, derive macro support for #[action(version = N)]** | 3 days |
| 16 | Derive macros: Action+Deps, ActionInput, ActionDeps, ParameterEnum | 1.5 weeks |
| 17 | Testing: harnesses, WebhookTestRequest, assertion macros, test_node!+mock | 5 days |
| 18 | **Port system: DataTag, DataTagRegistry, TagRegistrationError, namespace ownership, accepts/produces on ports, Plugin::data_tags()** | 3 days |
| 19 | Documentation: decision tree, 5 examples, error guide, versioning guide, port patterns | 3 days |
| 20 | TransactionalAction, sandbox enforcement | 4 days |

**Total: ~13-14 weeks.**

---

## v6 Architectural Directions

Not for v5. Design exploration for future.

- **Hot-reload with generation pinning:** RegisterMode enum, stable TypeKey.
- **Binary state codec:** StateCodec trait for non-JSON state serialization.
- **WaitCondition::ChildWorkflow output channel:** oneshot::Receiver for result.
- **ExecutionTracker with callback:** Alternative to polling execution_status.
- **Composite triggers (AND/OR):** Workflow-level composition.
- **Multi-step InteractiveAction DX:** MultiStepAction with step orchestration.
- **Streaming action core type:** 5th ActionKind with engine backpressure protocol.

---

## Migration from v1

| v1 (current) | v2 (new) |
|--------------|----------|
| Single InternalHandler | Per-type Handlers + ActionInstance |
| StatelessActionAdapter only | All 4 adapters with try_map_value |
| ActionContext: Clone | NOT Clone, ExecutionGuard |
| No ActionDescriptor | Descriptor (catalog) + Factory (instantiation) |
| AnyAction + as_any() | ActionInstance closed enum |
| CapabilitySet (5 bools) | CapabilityManifest (structured) |
| StatefulAction::State: Default | init_state + PersistedState envelope |
| No state commit contract | 6-step durable commit |
| No panic handling | Normative panic policy |
| TriggerAction: start/stop only | start/run split + EventTrigger DX |
| No webhook lifecycle hooks | check_exists / on_activate / on_deactivate (n8n parity) |
| No action versioning | InterfaceVersion + VersionedActionKey + ActionMigration trait |
| No version coexistence | Registry holds multiple versions, workflows pin to specific version |
| No deprecation | DeprecationInfo + UI upgrade flow + parameter migration |
| No port routing contract | ActionResult→Port routing rules documented + validated |
| No support port data access | support_data() + support_data_multi() on ActionContext |
| No trigger resources | TriggerContext with ResourceAccessor |
| No trigger state output | TriggerStartResult + checkpoint |
| No trigger parameters | TriggerContext::parameter() |
| ResourceAction: configure/cleanup | acquire/release + ReleaseOutcome |
| No binary input | BinaryData (inline/stored) + BinaryStorage core resource |
| No streaming output | StreamSender |
| No WaitCondition shapes | 5-variant enum |
| No action invocation | call_action() capability |
| MockExecution (mini-engine) | Single-node + harnesses |

### Migration path

- Phase 1-3: Coexist with InternalHandler. New types additive.
- Phase 4-6: New executors handle all types. InternalHandler deprecated.
- Phase 7+: DX types, derives — purely additive.
- Old InternalHandler removed after all consumers migrated.

---

## Validation Summary

**Assessment at HLD level. Remaining risk = implementation fidelity.**

| Category | Count | Notes |
|----------|-------|-------|
| 🟢 Architecturally covered | 46 | All standard integrations, stateful, triggers, resources, AI |
| 🟡 Implementable with some friction | 6 | Composite triggers, some edge cases |
| 🔴 Architectural blockers | 0 | All previous blockers resolved |
