# nebula-action: Implementation Plan

> Generated: 2026-02-14
> Sources: `docs/` (root architecture), `crates/action/docs/`, team review (architect, PM, TD, security)

## Overview

The `nebula-action` crate is the execution abstraction layer for Nebula workflow nodes.
It defines **what** actions are and **how they communicate** with the engine, but NOT how
the engine orchestrates them (that's the future `engine` crate).

Current state: ~260KB documentation, ~14 lines of stub code.

---

## Architecture Alignment

This plan follows the **Ports & Drivers** architecture from `docs/`:

```
Core (action) → Ports (traits) ← Drivers (inprocess, wasm)
```

The action crate:
- Defines `Action` traits, `ActionResult<T>`, `ActionError`, `ActionContext`
- Does NOT depend on concrete drivers, resilience, or storage
- Provides port traits (`SandboxRunner`) that drivers implement

---

## Phase 1: Core Types & ProcessAction

**Goal:** Compilable crate with core types, ProcessAction trait, and tests.

### Module Structure

```
crates/action/src/
├── lib.rs                  # #![forbid(unsafe_code)], public re-exports
├── error.rs                # ActionError enum
├── result.rs               # ActionResult<T>, BreakReason, BranchKey, PortKey
├── context.rs              # ActionContext struct
├── metadata.rs             # ActionMetadata, ActionType, InterfaceVersion, ExecutionMode
├── capability.rs           # Capability enum, IsolationLevel enum
├── action.rs               # Base Action trait
├── types/
│   ├── mod.rs              # Re-exports
│   └── process.rs          # ProcessAction trait
└── output.rs               # NodeOutputData enum (Inline/BlobRef)
```

### Dependencies (Cargo.toml)

```toml
[dependencies]
nebula-core = { path = "../core" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tokio-util = { workspace = true }    # CancellationToken
async-trait = { workspace = true }
chrono = { workspace = true }

[dev-dependencies]
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "time"] }
pretty_assertions = { workspace = true }
serde_json = { workspace = true }
```

### Type Contracts

#### ActionError (`error.rs`)

```rust
#[derive(Debug, Clone, thiserror::Error)]
pub enum ActionError {
    /// Retryable — engine decides retry policy using backoff_hint
    #[error("retryable: {error}")]
    Retryable {
        error: String,
        backoff_hint: Option<Duration>,
        partial_output: Option<serde_json::Value>,
    },

    /// Fatal — never retry
    #[error("fatal: {error}")]
    Fatal {
        error: String,
        details: Option<serde_json::Value>,
    },

    /// Input validation failed (before execution)
    #[error("validation: {0}")]
    Validation(String),

    /// Action requested capability it doesn't have
    #[error("sandbox violation: {capability} by {action_id}")]
    SandboxViolation {
        capability: String,
        action_id: String,
    },

    /// Execution cancelled via CancellationToken
    #[error("cancelled")]
    Cancelled,

    /// Output exceeds data limit
    #[error("data limit exceeded: {actual_bytes} > {limit_bytes}")]
    DataLimitExceeded {
        limit_bytes: u64,
        actual_bytes: u64,
    },
}
```

Convenience constructors:
```rust
impl ActionError {
    pub fn retryable(msg: impl Into<String>) -> Self { ... }
    pub fn retryable_with_backoff(msg: impl Into<String>, backoff: Duration) -> Self { ... }
    pub fn fatal(msg: impl Into<String>) -> Self { ... }
    pub fn validation(msg: impl Into<String>) -> Self { ... }

    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Retryable { .. })
    }
}
```

#### ActionResult<T> (`result.rs`)

Phase 1 includes only MVP variants. Phase 2+ variants behind feature flags.

```rust
pub type BranchKey = String;
pub type PortKey = String;

#[derive(Debug, Clone)]
pub enum ActionResult<T> {
    /// Successful completion → engine passes output to dependent nodes
    Success { output: T },

    /// Skip this node → engine skips downstream
    Skip { reason: String, output: Option<T> },

    /// StatefulAction: need another iteration
    Continue {
        output: T,
        progress: Option<f64>,
        delay: Option<Duration>,
    },

    /// StatefulAction: iteration complete
    Break {
        output: T,
        reason: BreakReason,
    },

    /// Choose a branch (if/else, switch)
    Branch {
        selected: BranchKey,
        output: T,
        alternatives: HashMap<BranchKey, T>,
    },

    /// Route to specific output port
    Route { port: PortKey, data: T },

    /// Fan-out to multiple output ports
    MultiOutput {
        outputs: HashMap<PortKey, T>,
        main_output: Option<T>,
    },

    /// Wait for external event/timer/human
    Wait {
        condition: WaitCondition,
        timeout: Option<Duration>,
        partial_output: Option<T>,
    },
}

#[derive(Debug, Clone)]
pub enum BreakReason {
    Completed,
    MaxIterations,
    ConditionMet,
    Custom(String),
}

#[derive(Debug, Clone)]
pub enum WaitCondition {
    Webhook { callback_id: String },
    Until { datetime: DateTime<Utc> },
    Duration { duration: Duration },
    Approval { approver: String, message: String },
    Execution { execution_id: ExecutionId },
}
```

#### ActionContext (`context.rs`)

```rust
/// Runtime context provided to every action during execution.
/// The engine constructs this before calling Action::execute().
pub struct ActionContext {
    /// Unique execution run ID
    pub execution_id: ExecutionId,
    /// Node in the workflow graph
    pub node_id: NodeId,
    /// Workflow this execution belongs to
    pub workflow_id: WorkflowId,
    /// Scope for resource access control
    pub scope: ScopeLevel,
    /// Cancellation — actions MUST check in long-running loops
    pub cancellation: CancellationToken,
    /// Workflow-scoped variables (read/write by actions)
    variables: Arc<parking_lot::RwLock<serde_json::Map<String, serde_json::Value>>>,
}
```

Methods:
```rust
impl ActionContext {
    pub fn new(execution_id, node_id, workflow_id, scope) -> Self { ... }
    pub fn get_variable(&self, key: &str) -> Option<serde_json::Value> { ... }
    pub fn set_variable(&self, key: &str, value: serde_json::Value) { ... }
    pub fn check_cancelled(&self) -> Result<(), ActionError> { ... }
}
```

> Note: Phase 2 adds `credentials: Arc<dyn SecretsStore>`, `resources: Arc<ResourceManager>`,
> `telemetry: Arc<Telemetry>` fields. These are NOT in Phase 1 to avoid depending on
> crates that don't exist yet. The struct is non-exhaustive (#[non_exhaustive]) to allow
> adding fields without breaking changes.

#### ActionMetadata (`metadata.rs`)

```rust
#[derive(Debug, Clone)]
pub struct ActionMetadata {
    pub key: String,                     // e.g. "http.request"
    pub name: String,                    // e.g. "HTTP Request"
    pub description: String,
    pub category: String,                // e.g. "network", "transform"
    pub version: InterfaceVersion,
    pub capabilities: Vec<Capability>,
    pub isolation_level: IsolationLevel,
    pub execution_mode: ExecutionMode,
    pub input_schema: Option<serde_json::Value>,   // JSON Schema
    pub output_schema: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceVersion {
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionType {
    Process,
    Stateful,
    Trigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// Typed: Input/Output implement Serialize + Deserialize + Validate
    Typed,
    /// Dynamic: serde_json::Value + JSON Schema validation
    Dynamic,
}
```

#### Capability & IsolationLevel (`capability.rs`)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    Network { allowed_hosts: Vec<String> },
    FileSystem { paths: Vec<String>, read_only: bool },
    Resource(String),
    Credential(String),
    MaxMemory(usize),
    MaxCpuTime(Duration),
    Environment { keys: Vec<String> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IsolationLevel {
    /// Trusted builtin — no sandbox
    None,
    /// In-process with capability checks
    #[default]
    CapabilityGated,
    /// Full WASM/process isolation (mandatory for community actions)
    Isolated,
}
```

#### Base Action Trait (`action.rs`)

```rust
/// Base trait all actions implement. Provides identity and metadata.
/// The engine uses this to inspect capabilities, isolation level, etc.
pub trait Action: Send + Sync + 'static {
    /// Static metadata describing this action type
    fn metadata(&self) -> &ActionMetadata;

    /// Action type discriminant
    fn action_type(&self) -> ActionType;
}
```

#### ProcessAction (`types/process.rs`)

```rust
/// Stateless, single-execution action. The most common type.
/// Covers ~80% of workflow nodes: HTTP requests, transforms, filters, etc.
#[async_trait]
pub trait ProcessAction: Action {
    type Input: Send + Sync + 'static;
    type Output: Send + Sync + 'static;

    /// Execute the action with given input and context
    async fn execute(
        &self,
        input: Self::Input,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError>;

    /// Validate input before execution (optional, default: pass)
    async fn validate_input(&self, _input: &Self::Input) -> Result<(), ActionError> {
        Ok(())
    }
}
```

#### NodeOutputData (`output.rs`)

```rust
/// How action output is passed between nodes.
/// Small data is inline; large data spills to blob storage.
#[derive(Debug, Clone)]
pub enum NodeOutputData {
    Inline(serde_json::Value),
    BlobRef {
        key: String,
        size: u64,
        mime: String,
    },
}
```

### Phase 1 Deliverables

- [ ] All types above compile: `cargo check -p nebula-action`
- [ ] `#![forbid(unsafe_code)]` in lib.rs
- [ ] Unit tests for ActionError (constructors, is_retryable)
- [ ] Unit tests for ActionResult (pattern matching, all variants)
- [ ] Unit tests for ActionContext (get/set variable, cancellation)
- [ ] Example: JSON transform ProcessAction
- [ ] `cargo clippy -p nebula-action -- -D warnings`
- [ ] `cargo fmt --all -- --check`
- [ ] `cargo doc -p nebula-action --no-deps`

---

## Phase 2: StatefulAction + TriggerAction

**Goal:** Complete the 3 MVP action types from docs.

### New Files

```
crates/action/src/types/
├── stateful.rs             # StatefulAction trait
└── trigger.rs              # TriggerAction trait + TriggerKind + TriggerEvent
```

### StatefulAction (`types/stateful.rs`)

```rust
#[async_trait]
pub trait StatefulAction: Action {
    type State: Serialize + DeserializeOwned + Send + Sync + 'static;
    type Input: Send + Sync + 'static;
    type Output: Send + Sync + 'static;

    async fn execute_with_state(
        &self,
        input: Self::Input,
        state: &mut Self::State,
        ctx: &ActionContext,
    ) -> Result<ActionResult<Self::Output>, ActionError>;

    async fn initialize_state(
        &self,
        input: &Self::Input,
        ctx: &ActionContext,
    ) -> Result<Self::State, ActionError>;

    fn state_version(&self) -> u32 { 1 }

    async fn migrate_state(
        &self,
        old_state: serde_json::Value,
        old_version: u32,
    ) -> Result<Self::State, ActionError> {
        serde_json::from_value(old_state)
            .map_err(|e| ActionError::fatal(format!("state migration failed: {e}")))
    }
}
```

### TriggerAction (`types/trigger.rs`)

```rust
#[derive(Debug, Clone)]
pub enum TriggerKind {
    Poll { interval: Duration },
    Webhook { path: String },
    Cron { expression: String },
}

#[derive(Debug, Clone)]
pub struct TriggerEvent<T> {
    pub data: T,
    pub timestamp: DateTime<Utc>,
    pub dedup_key: Option<String>,
}

#[async_trait]
pub trait TriggerAction: Action {
    type Config: Send + Sync + 'static;
    type Event: Send + Sync + 'static;

    fn kind(&self, config: &Self::Config) -> TriggerKind;

    async fn poll(
        &self,
        config: &Self::Config,
        last_state: Option<serde_json::Value>,
        ctx: &ActionContext,
    ) -> Result<Vec<TriggerEvent<Self::Event>>, ActionError> {
        Ok(vec![])
    }

    async fn handle_webhook(
        &self,
        config: &Self::Config,
        request: WebhookRequest,
        ctx: &ActionContext,
    ) -> Result<TriggerEvent<Self::Event>, ActionError> {
        Err(ActionError::fatal("webhook not supported"))
    }
}

pub struct WebhookRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: serde_json::Value,
}
```

### Context Enrichment

Add optional credential + expression access to ActionContext:
```rust
// Behind feature flags or Option<Arc<...>>
pub struct ActionContext {
    // ... existing fields ...
    credentials: Option<Arc<dyn SecretsStore>>,
    // expression_engine: Option<Arc<ExpressionEngine>>,  // when available
}
```

### Phase 2 Deliverables

- [ ] StatefulAction compiles with serde bounds
- [ ] TriggerAction compiles with poll + webhook
- [ ] Example: paginated API scraper (StatefulAction)
- [ ] Example: interval timer trigger (TriggerAction)
- [ ] Tests for state initialization + migration
- [ ] Tests for trigger event deduplication

---

## Phase 3: Sandbox + Registry

**Goal:** Action registration, discovery, and sandboxed execution port.

### New Files

```
crates/action/src/
├── sandbox.rs              # SandboxedContext, SandboxRunner port trait
├── registry.rs             # ActionRegistry (type-erased)
└── budget.rs               # ExecutionBudget, DataPassingPolicy
```

### SandboxedContext (`sandbox.rs`)

```rust
pub struct SandboxedContext {
    inner: ActionContext,
    granted_capabilities: Vec<Capability>,
}

impl SandboxedContext {
    pub fn check_capability(&self, cap: &Capability) -> Result<(), ActionError> { ... }
    pub fn check_cancelled(&self) -> Result<(), ActionError> { ... }
    // Delegates to inner context with capability checks
}

/// Port trait — implemented by drivers (inprocess, wasm)
#[async_trait]
pub trait SandboxRunner: Send + Sync {
    async fn execute(
        &self,
        action: &dyn Action,
        context: SandboxedContext,
        metadata: &ActionMetadata,
    ) -> Result<serde_json::Value, ActionError>;
}
```

### ActionRegistry (`registry.rs`)

```rust
pub struct ActionRegistry {
    actions: HashMap<String, Arc<dyn Action>>,
}

impl ActionRegistry {
    pub fn new() -> Self { ... }
    pub fn register(&mut self, action: Arc<dyn Action>) { ... }
    pub fn get(&self, key: &str) -> Option<&Arc<dyn Action>> { ... }
    pub fn list(&self) -> Vec<&ActionMetadata> { ... }
}
```

### ExecutionBudget (`budget.rs`)

```rust
pub struct ExecutionBudget {
    pub max_concurrent_nodes: usize,
    pub max_total_retries: u32,
    pub max_wall_time: Duration,
    pub max_payload_bytes: u64,
    pub data_policy: DataPassingPolicy,
}

pub struct DataPassingPolicy {
    pub max_node_output_bytes: u64,
    pub max_total_execution_bytes: u64,
    pub large_data_strategy: LargeDataStrategy,
}

pub enum LargeDataStrategy {
    Reject,
    SpillToBlob,
}
```

### Phase 3 Deliverables

- [ ] SandboxedContext enforces capability checks
- [ ] SandboxRunner port trait defined
- [ ] ActionRegistry stores and retrieves actions
- [ ] ExecutionBudget + DataPassingPolicy types
- [ ] Tests: unauthorized capability access → SandboxViolation
- [ ] Tests: registry register + get + list

---

## Phase 4: Advanced Types & Integration

**Goal:** Remaining ActionResult variants, advanced action types.

- `AsyncOperation` variant (long-running external ops)
- `StreamItem` variant (streaming actions)
- `InteractionRequired` variant (human-in-the-loop)
- `TransactionPrepared` variant (saga pattern)
- Schema migration system (`SchemaMigration` enum)
- Integration with `nebula-resilience` via decorator pattern in engine layer

---

## Security Checklist (from audit)

- [x] `#![forbid(unsafe_code)]` — Phase 1
- [ ] ScopedCredentialAccessor via SecretsStore port — Phase 2
- [ ] Capability-based context (SandboxedContext) — Phase 3
- [ ] Execution timeouts enforced by engine — Phase 3
- [ ] Per-tenant rate limiting — Phase 3
- [ ] Audit events for action execution — Phase 3
- [ ] Fix `scope.rs:142` is_contained_in bug — separate PR

## Dependency Graph

```
Phase 1: nebula-action → nebula-core, serde_json, thiserror, tokio-util, async-trait, chrono
Phase 2: + serde (Serialize/Deserialize bounds for State)
Phase 3: no new deps (SandboxRunner is a port trait, no concrete driver)
```

nebula-action does NOT depend on:
- nebula-resilience (composed by engine/executor)
- nebula-expression (injected via context, optional)
- nebula-credential (accessed via SecretsStore port)
- nebula-config (actions don't hot-reload)
- nebula-memory (no arena allocation needed)

## Key Decisions Log

| # | Decision | Rationale |
|---|----------|-----------|
| 1 | Follow docs/ architecture (Ports & Drivers) | Root docs are the source of truth, more mature than crates/action/docs/ |
| 2 | ActionError from docs/ (6 variants) over TD's 5 | backoff_hint + partial_output in Retryable, SandboxViolation, DataLimitExceeded are justified |
| 3 | ActionResult<T> — full enum, not split Output+FlowDirective | Docs architecture expects engine to `match` on result; splitting loses information |
| 4 | ActionContext as struct (TD), not trait (Architect) | Only 1 consumer initially; test via construction, not mocking |
| 5 | 3 action types for MVP (Process, Stateful, Trigger) | PM: Process + Trigger; Docs: all 3; compromise: Process in Phase 1, Stateful+Trigger in Phase 2 |
| 6 | Resilience is NOT in action crate | TD + docs agree: Action returns Retryable, engine decides retry policy |
| 7 | #[non_exhaustive] on ActionContext | Allows adding fields (credentials, telemetry) in Phase 2 without breaking |
| 8 | Capability enum in action crate from Phase 1 | Needed for ActionMetadata; enforcement via SandboxedContext in Phase 3 |
