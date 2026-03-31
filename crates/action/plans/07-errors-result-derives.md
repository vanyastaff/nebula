# Error Model, ActionResult, Derive Macros, and DX Helpers

## ActionError

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ActionError {
    #[error("retryable: {message}")]
    Retryable {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        backoff_hint: Option<Duration>,
        code: Option<ErrorCode>,
    },

    #[error("fatal: {message}")]
    Fatal {
        message: String,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
        code: Option<ErrorCode>,
    },

    #[error("validation: {message}")]
    Validation { message: String },

    #[error("sandbox violation: {capability}: {message}")]
    SandboxViolation { message: String, capability: String },

    #[error("cancelled")]
    Cancelled,

    #[error("data limit exceeded: {message} (limit={limit}, actual={actual})")]
    DataLimitExceeded { message: String, limit: usize, actual: usize },
}

// ── String-based constructors (no std::error::Error needed) ──
impl ActionError {
    pub fn fatal(msg: impl Into<String>) -> Self {
        ActionError::Fatal { message: msg.into(), source: None, code: None }
    }
    pub fn retryable_msg(msg: impl Into<String>) -> Self {
        ActionError::Retryable { message: msg.into(), source: None, backoff_hint: None, code: None }
    }
    pub fn validation(msg: impl Into<String>) -> Self {
        ActionError::Validation { message: msg.into() }
    }
}

// ── Error-based constructors (preserves source) ──
impl ActionError {
    pub fn retryable<E: std::error::Error + Send + Sync + 'static>(err: E) -> Self {
        ActionError::Retryable {
            message: err.to_string(), source: Some(Box::new(err)),
            backoff_hint: None, code: None,
        }
    }
    pub fn fatal_err<E: std::error::Error + Send + Sync + 'static>(err: E) -> Self {
        ActionError::Fatal {
            message: err.to_string(), source: Some(Box::new(err)), code: None,
        }
    }
    pub fn retryable_code<E: std::error::Error + Send + Sync + 'static>(err: E, code: ErrorCode) -> Self {
        ActionError::Retryable {
            message: err.to_string(), source: Some(Box::new(err)),
            backoff_hint: None, code: Some(code),
        }
    }
}
```

## ErrorCode

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ErrorCode {
    RateLimited,
    Conflict,
    AuthExpired,
    UpstreamUnavailable,
    UpstreamTimeout,
    InvalidInput,
    QuotaExhausted,
    ActionPanicked,
    Custom(Cow<'static, str>),
}
```

## RetryPolicy / TimeoutPolicy

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub initial_interval: Duration,
    pub backoff_coefficient: f64,
    pub maximum_interval: Duration,
    pub non_retryable_codes: Vec<ErrorCode>,
}
impl Default for RetryPolicy { /* max 3, 1s, 2.0, 60s */ }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeoutPolicy {
    pub schedule_to_start: Option<Duration>,
    pub start_to_close: Duration,               // default 5min
    pub schedule_to_close: Option<Duration>,
    pub heartbeat: Option<Duration>,             // engine cancels if 2× missed
}
impl Default for TimeoutPolicy { /* start_to_close = 300s */ }
```

### ActionResult::Retry vs ActionError::Retryable — disambiguation

Both trigger re-execution, but with different semantics:

| Scenario | Use | Why |
|----------|-----|-----|
| HTTP 429 with Retry-After header | `ActionResult::Retry { after }` | Not an error — server says "come back later". Data expected on retry. |
| Network connection dropped mid-request | `ActionError::Retryable` | Real error — request may or may not have reached server. |
| Eventual consistency: data not ready yet | `ActionResult::Retry { after }` | Not an error — action will succeed on next attempt. |
| External API returned 500 | `ActionError::Retryable` | Server error — may be transient. |
| Auth token expired mid-workflow | `ActionError::Retryable { code: AuthExpired }` | Credential refresh needed before retry. |
| Upstream says "processing, check back in 30s" | `ActionResult::Retry { after: 30s }` | Polling for async result — not an error. |

**Rule of thumb:** If the action ran correctly but there's no result yet → `Retry`.
If something went wrong → `Retryable`.

`Retry` counts are NOT limited by RetryPolicy (it's a successful signal).
`Retryable` counts ARE limited by RetryPolicy (max_attempts, backoff).

**Note on `async_trait` vs `impl Future`:**
Action traits (`StatelessAction`, `TriggerAction`) use `-> impl Future<...> + Send`
because they have associated types and are NOT object-safe — each action is a
concrete type known at compile time. Handler traits (`StatelessHandler`, `TriggerHandler`)
use `#[async_trait]` because they MUST be object-safe for `Box<dyn StatelessHandler>`
in the type-erased registry layer. This asymmetry is intentional.

## ParameterBindingError (with context)

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ParameterBindingError {
    #[error("expression failed: field '{field}' from {source:?}: {message}")]
    ExpressionError { field: String, source: BindingSource, message: String },
    #[error("validation failed: field '{field}' from {source:?}: {message}")]
    ValidationError { field: String, source: BindingSource, message: String },
    #[error("binding failed: field '{field}' from {source:?}: {message}")]
    BindingError { field: String, source: BindingSource, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingSource { Form, Upstream, Environment, TriggerPayload }

impl From<ParameterBindingError> for ActionError {
    fn from(e: ParameterBindingError) -> Self {
        ActionError::Validation { message: e.to_string() }
    }
}
```

---

## ResultActionExt (DX extension trait)

```rust
pub trait ResultActionExt<T, E> {
    fn retryable(self) -> Result<T, ActionError>;
    fn retryable_code(self, code: ErrorCode) -> Result<T, ActionError>;
    fn fatal(self) -> Result<T, ActionError>;
    fn validation(self) -> Result<T, ActionError>;
}

impl<T, E: std::error::Error + Send + Sync + 'static> ResultActionExt<T, E> for Result<T, E> {
    fn retryable(self) -> Result<T, ActionError> { self.map_err(ActionError::retryable) }
    fn retryable_code(self, code: ErrorCode) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::retryable_code(e, code))
    }
    fn fatal(self) -> Result<T, ActionError> { self.map_err(ActionError::fatal_err) }
    fn validation(self) -> Result<T, ActionError> {
        self.map_err(|e| ActionError::validation(e.to_string()))
    }
}

// Validation assertion macro
macro_rules! ensure {
    ($cond:expr, $err:expr) => {
        if !($cond) { return Err($err); }
    };
    ($cond:expr, $fmt:literal $(, $arg:expr)*) => {
        if !($cond) { return Err(ActionError::validation(format!($fmt $(, $arg)*))); }
    };
}
```

**Usage:**
```rust
let resp = client.get(&url).send().await.retryable()?;
let body: Data = resp.json().await.fatal()?;
ensure!(body.items.len() <= 1000, ActionError::validation("too many items"));
// Or with format string (creates ActionError::validation automatically):
ensure!(body.items.len() <= 1000, "too many items: {} > 1000", body.items.len());
```

---

## ActionResult and ActionOutput (aligned with existing codebase)

### ActionResult\<T\>

Existing codebase has richer variants than initial HLD: `Branch` includes
`alternatives` (for preview), `MultiOutput` includes `main_output`,
`Skip` carries optional output. All durations serialized as milliseconds.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum ActionResult<T> {
    /// Successful completion — pass output downstream.
    Success {
        output: ActionOutput<T>,
    },

    /// Skip — engine skips downstream dependents.
    Skip {
        reason: String,
        output: Option<ActionOutput<T>>,  // optional partial output
    },

    /// Stateful iteration: not done, need another call.
    Continue {
        output: ActionOutput<T>,
        progress: Option<f64>,            // 0.0..=1.0
        #[serde(default, with = "duration_opt_ms")]
        delay: Option<Duration>,          // rate limiting
    },

    /// Stateful iteration: complete.
    Break {
        output: ActionOutput<T>,
        reason: BreakReason,
    },

    /// Choose a workflow branch (if/else, switch).
    Branch {
        selected: BranchKey,              // key of chosen branch
        output: ActionOutput<T>,          // output for selected branch
        alternatives: HashMap<BranchKey, ActionOutput<T>>,  // non-selected (preview)
    },

    /// Route output to a specific output port.
    Route {
        port: PortKey,
        data: ActionOutput<T>,
    },

    /// Fan-out to multiple ports.
    MultiOutput {
        outputs: HashMap<PortKey, ActionOutput<T>>,
        main_output: Option<ActionOutput<T>>,  // optional default port
    },

    /// Pause until external event.
    Wait {
        condition: WaitCondition,
        #[serde(default, with = "duration_opt_ms")]
        timeout: Option<Duration>,
        partial_output: Option<ActionOutput<T>>,
    },

    /// Request re-execution after delay (successful signal, not error).
    Retry {
        #[serde(with = "duration_ms")]
        after: Duration,
        reason: String,
    },
}
```

**Convenience constructors (from codebase):**
```rust
impl<T> ActionResult<T> {
    pub fn success(output: T) -> Self { ... }
    pub fn success_binary(data: BinaryData) -> Self { ... }
    pub fn success_reference(reference: DataReference) -> Self { ... }
    pub fn success_deferred(deferred: DeferredOutput) -> Self { ... }
    pub fn success_empty() -> Self { ... }
    pub fn success_output(output: ActionOutput<T>) -> Self { ... }
    pub fn skip(reason: impl Into<String>) -> Self { ... }
    pub fn skip_with_output(reason: impl Into<String>, output: T) -> Self { ... }

    pub fn is_success(&self) -> bool { ... }
    pub fn is_continue(&self) -> bool { ... }
    pub fn is_waiting(&self) -> bool { ... }
    pub fn is_retry(&self) -> bool { ... }

    /// Extract primary output from any variant.
    pub fn into_primary_output(self) -> Option<ActionOutput<T>> { ... }
    /// Extract primary value T (only if Value variant).
    pub fn into_primary_value(self) -> Option<T> { ... }
    /// Transform output value in every variant.
    pub fn map_output<U>(self, f: impl FnMut(T) -> U) -> ActionResult<U> { ... }
    /// Fallible map (Binary/Streaming pass through unchanged).
    pub fn try_map_value<U, E>(self, f: impl FnOnce(T) -> Result<U, E>) -> Result<ActionResult<U>, E> { ... }
}
```

### ActionOutput\<T\>

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[non_exhaustive]
pub enum ActionOutput<T> {
    /// Structured data (primary case).
    Value(T),
    /// Binary data (files, images) with inline/stored strategy.
    Binary(BinaryData),
    /// Reference to external data (S3, datasets).
    Reference(DataReference),
    /// Output resolved asynchronously (AI generation kicked off).
    /// Engine resolves before passing to downstream.
    Deferred(Box<DeferredOutput>),
    /// Streaming output reference. Engine collects or forwards.
    Streaming(StreamOutput),
    /// Multiple outputs (batch results, fan-out).
    Collection(Vec<ActionOutput<T>>),
    /// No output.
    Empty,
}

impl<T> ActionOutput<T> {
    pub fn is_empty(&self) -> bool { ... }
    pub fn is_binary(&self) -> bool { ... }
    pub fn as_value(&self) -> Option<&T> { ... }
    pub fn into_value(self) -> Option<T> { ... }
    pub fn needs_resolution(&self) -> bool { matches!(self, Self::Deferred(_)) }
    pub fn map<U>(self, f: &mut impl FnMut(T) -> U) -> ActionOutput<U> { ... }
    pub fn try_map<U, E>(self, f: &mut impl FnMut(T) -> Result<U, E>) -> Result<ActionOutput<U>, E> { ... }
}
```

### OutputEnvelope (from codebase — production metadata)

Wraps ActionOutput with production metadata. Used at engine/runtime boundaries.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputEnvelope<T = serde_json::Value> {
    /// The actual output data.
    pub output: ActionOutput<T>,
    /// Production metadata (origin, cost, timing, cache).
    pub meta: OutputMeta,
}

impl<T> OutputEnvelope<T> {
    pub fn new(output: ActionOutput<T>) -> Self { ... }
    pub fn with_meta(output: ActionOutput<T>, meta: OutputMeta) -> Self { ... }
}
```

OutputMeta captures: node origin, execution cost/tokens, timing, cache status.
Definition lives in output.rs. Action authors don't create it — engine/runtime adds
metadata after execution.

### BreakReason

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BreakReason {
    Completed,
    MaxIterations,
    ConditionMet,
    Custom(String),
}
```

---

## Port System

Ports define the **graph topology contract** — how nodes wire together in a workflow.
Every action declares its ports in ActionMetadata. UI editor validates wiring,
engine routes data between nodes based on ports.

### Port types

```
InputPort                          OutputPort
├── Flow { key }                   ├── Flow { key, kind: Main|Error }
└── Support(SupportPort)           └── Dynamic(DynamicPort)
```

Three semantics:
- **Flow** — main data pipe. Every action has at least one input and output.
- **Support** — side-channel inputs (AI tools, model, memory). Don't affect main flow.
- **Dynamic** — config-driven outputs generated at resolve time (Switch node → N branches).

### Port declarations (from existing codebase)

```rust
pub type PortKey = String;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputPort {
    /// Main data flow input.
    Flow { key: PortKey },
    /// Sub-node / supply input (e.g. AI tool, memory, model).
    Support(SupportPort),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OutputPort {
    /// Data or error flow output.
    Flow { key: PortKey, kind: FlowKind },
    /// Config-driven dynamic outputs (e.g. Switch, Router).
    Dynamic(DynamicPort),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowKind {
    /// Primary data output.
    Main,
    /// Error output (appears when on-error handling is enabled).
    Error,
}
```

### SupportPort (AI composition)

```rust
pub struct SupportPort {
    pub key: PortKey,
    pub name: String,
    pub description: String,
    /// Whether a connection is required for the action to run.
    pub required: bool,
    /// Whether multiple sub-nodes may connect simultaneously.
    pub multi: bool,
    /// Restricts which node types may connect.
    pub filter: ConnectionFilter,
}

pub struct ConnectionFilter {
    /// Only nodes with these type keys may connect.
    pub allowed_node_types: Option<Vec<String>>,
    /// Only nodes carrying at least one of these tags may connect.
    pub allowed_tags: Option<Vec<String>>,
}
```

**Example — AI Agent with tools:**
```rust
let meta = ActionMetadata::new("ai.agent", "AI Agent", "Run agent")
    .with_inputs(vec![
        InputPort::flow("in"),
        InputPort::Support(SupportPort {
            key: "model".into(),
            name: "AI Model".into(),
            description: "Language model to use".into(),
            required: true,             // agent NEEDS a model
            multi: false,               // exactly one model
            filter: ConnectionFilter::new()
                .with_allowed_tags(vec!["llm".into()]),
        }),
        InputPort::Support(SupportPort {
            key: "tools".into(),
            name: "Tools".into(),
            description: "Available tools for agent".into(),
            required: false,            // tools are optional
            multi: true,                // multiple tools allowed
            filter: ConnectionFilter::new()
                .with_allowed_tags(vec!["langchain_tool".into()]),
        }),
        InputPort::Support(SupportPort {
            key: "memory".into(),
            name: "Memory".into(),
            description: "Conversation memory".into(),
            required: false,
            multi: false,
            filter: ConnectionFilter::new()
                .with_allowed_tags(vec!["memory".into()]),
        }),
    ]);
```

### DynamicPort (config-driven outputs)

```rust
pub struct DynamicPort {
    /// Base key prefix for generated ports (e.g. "rule" → "rule_0", "rule_1").
    pub key: PortKey,
    /// Config path to the array that drives port generation (e.g. "rules").
    pub source_field: String,
    /// Optional field name within each array element for port label.
    pub label_field: Option<String>,
    /// Whether to append a fallback port after generated ports.
    pub include_fallback: bool,
}
```

**Example — Switch node:**
```rust
let meta = ActionMetadata::new("flow.switch", "Switch", "Route by conditions")
    .with_inputs(vec![InputPort::flow("in")])
    .with_outputs(vec![
        OutputPort::dynamic("rule", "rules"),  // generates rule_0, rule_1, ...
    ]);

// At resolve time, if node config has:
//   { "rules": [{ "label": "VIP", "condition": "..." }, { "label": "Normal", "condition": "..." }] }
// Engine generates concrete ports:
//   rule_0 ("VIP"), rule_1 ("Normal"), __fallback (if include_fallback)
```

### Default ports (convention)

```rust
pub fn default_input_ports() -> Vec<InputPort> {
    vec![InputPort::flow("in")]      // single flow input
}

pub fn default_output_ports() -> Vec<OutputPort> {
    vec![OutputPort::flow("out")]    // single main flow output
}
```

Most actions use defaults. Explicit declaration only for: multi-input, branching,
error handling, AI composition.

---

### ActionResult → Port Routing Contract (normative)

Engine routes output data to ports based on ActionResult variant:

```
ActionResult variant              → Target port(s)              → Engine behavior
──────────────────────────────────────────────────────────────────────────────────
Success { output }                → "out" (Main flow)           → Pass to all downstream on "out"
Skip { reason, output }          → (none)                      → Skip downstream. If output present,
                                                                  log but don't route.
Continue { output, progress }    → (internal)                  → Engine re-enqueues node. Output
                                                                  available for progress UI only.
Break { output, reason }         → "out" (Main flow)           → Final iteration output → downstream.
Branch { selected, output, alt } → selected key port            → Activate ONLY the matched branch
                                                                  port. Alternatives for preview only.
Route { port, data }             → port key                    → Route to specific declared port.
MultiOutput { outputs, main }    → per-port map                → Route each entry to its port key.
                                                                  main_output → "out" if present.
Wait { condition, partial }      → (internal)                  → Park execution. No port routing
                                                                  until resume.
Retry { after, reason }          → (internal)                  → Re-enqueue after delay. No port
                                                                  routing.

ActionError (any variant)        → "error" port (if wired)     → Route error to error handler.
                                 → (propagate if unwired)      → Fail workflow if no error port.
```

**Rules:**
1. Action MUST only reference ports declared in its ActionMetadata.outputs.
2. `Branch.selected` key MUST match a declared output port key.
3. `Route.port` key MUST match a declared output port key.
4. `MultiOutput.outputs` keys MUST be subset of declared output ports.
5. Engine validates at wiring time (editor) AND at execution time (runtime).
6. Referencing undeclared port = runtime error (`ActionError::Validation`).

### Error port routing

```
Node declares: outputs: [Flow("out", Main), Flow("error", Error)]

Case 1: Error port WIRED to error handler node
  → ActionError caught by engine
  → Error details sent to "error" port
  → Error handler node executes
  → Workflow continues (error recovered)

Case 2: Error port NOT WIRED
  → ActionError propagates
  → Workflow execution fails with error
  → No recovery possible at graph level
```

**Error port is opt-in per node.** Action always declares `Flow("error", Error)`
in outputs. Whether it's wired = user decision in editor. Engine checks wiring.

### Multi-input flow semantics

Nodes with multiple flow input ports (Merge, Join, CompareDatasets):

```rust
let meta = ActionMetadata::new("flow.merge", "Merge", "Merge two inputs")
    .with_inputs(vec![
        InputPort::flow("input_a"),
        InputPort::flow("input_b"),
    ])
    .with_outputs(vec![OutputPort::flow("out")]);
```

**How data arrives:**

```
                 ┌─────────────────────┐
  upstream_A ──→ │ input_a             │
                 │         Merge  ──→ out ──→ downstream
  upstream_B ──→ │ input_b             │
                 └─────────────────────┘
```

**Runtime binding contract:**
1. Engine waits for **all** wired flow input ports to have data before executing node.
   (This is "join" semantics — all inputs ready = node executes.)
2. Data from each port available via `ctx.port_data("input_a")`, `ctx.port_data("input_b")`.
3. The `Self::Input` type (from `#[derive(ActionInput)]`) binds from the **primary** input port
   (first Flow port = "input_a"). Additional ports accessed via `ctx.port_data()`.
4. If a port is **not wired**, `port_data()` returns `None`.

**Alternative: array-input for fan-in:**
```
When engine does fan-out (SplitOut → N parallel branches → Merge),
the "input_a" port receives an ARRAY of results from all parallel branches.
Single port, multiple items. No multi-port needed for fan-in.
```

### Support port data flow

Support ports are **side-channel** — they don't participate in main flow scheduling.

```rust
// AI Agent execution:
async fn execute(&self, input: Self::Input, ctx: &ActionContext) -> Result<...> {
    // Main flow input (from "in" port)
    let query = input.query;
    
    // Support port data — available via special accessor
    // Runtime injects support connections as sub-node outputs
    let model_config = ctx.support_data("model")?;     // from model sub-node
    let tools: Vec<Value> = ctx.support_data_multi("tools")?; // from N tool sub-nodes
    let memory = ctx.support_data("memory").ok();       // optional, may be None
    
    // ... use model, tools, memory for agent execution
}
```

**Support data accessors** — `ctx.support_data(key)` and `ctx.support_data_multi(key)` —
are defined on ActionContext (see 05-context-capabilities.md, ActionContext section).
Values are immutable during execution — safe to hold references within execute().

**Support port validation contract (normative):**
```
Editor (design time):
  - Validates topology: allowed_node_types, allowed_tags from ConnectionFilter
  - Enforces cardinality: multi=false → max 1 connection
  - Shows warning for required=true ports without connection

Runtime (before execute):
  - MUST validate required=true ports have data. If missing → ActionError::Validation
    ("required support port 'model' not connected")
  - MUST validate multi=false cardinality. If >1 connection → ActionError::Validation
  - MUST NOT rely on editor validation alone (editor/runtime desync possible)
  - Validation runs BEFORE execute(), not inside — fail fast

Violation at any layer → ActionError::Validation, never silent skip.
```

### Dynamic port resolution (engine contract)

```
At workflow save time (editor):
  1. Editor reads DynamicPort declaration from ActionDescriptor
  2. Editor reads source_field from node config (e.g. "rules" array)
  3. Editor generates concrete ports: rule_0, rule_1, ..., __fallback
  4. Editor shows ports in UI for wiring

At execution time (engine):
  1. Engine re-resolves dynamic ports from current node config
  2. Engine validates that all wired ports still exist
  3. Action returns Route { port: "rule_1", data } or Branch { selected: "rule_0" }
  4. Engine routes data to concrete port
  5. If include_fallback and no rule matched → route to __fallback
```

### Port Data Tags (optional typed wiring)

Ports are **untyped by default** (n8n mode). DataTag adds optional type hints
for editor-level wiring validation. **Runtime NEVER enforces tags** — data is
always JSON + binary references, expressions always work regardless of tags.

**Two data paths between nodes:**
```
Path 1: Direct wire (drag-and-drop in editor)
  Node A [IMAGE] ──wire──→ Node B [IMAGE input]
  Editor: checks DataTag compatibility. IMAGE→IMAGE ✅, TEXT→IMAGE ❌
  Runtime: passes output A as input B. No type check.

Path 2: Expression reference (in parameter field)
  Node B parameter "file": {{ $node["S3"].output.binary_data }}
  Editor: does NOT check DataTag. User is in control.
  Runtime: expression engine resolves → binds to parameter. No tag check.
```

**Rule: DataTag = editor UX only, never runtime enforcement.**

#### DataTag type

```rust
/// Port data type tag. NOT arbitrary string — must be registered in DataTagRegistry.
/// Core types built-in, domain types registered by plugins.
///
/// **Empty tags = untyped (accept/produce anything).** This is the default —
/// preserves n8n behavior where any node connects to any node.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DataTag(Arc<str>);

impl DataTag {
    // ── Core types (always available) ──
    pub const JSON: DataTag     = DataTag::new_static("json");
    pub const TEXT: DataTag     = DataTag::new_static("text");
    pub const NUMBER: DataTag   = DataTag::new_static("number");
    pub const BOOLEAN: DataTag  = DataTag::new_static("boolean");
    pub const BINARY: DataTag   = DataTag::new_static("binary");
    pub const ARRAY: DataTag    = DataTag::new_static("array");
    pub const STREAM: DataTag   = DataTag::new_static("stream");
}
```

#### DataTagRegistry (validated at registration time)

```rust
pub struct DataTagRegistry {
    tags: HashMap<String, DataTagInfo>,
}

pub struct DataTagInfo {
    /// Tag identifier.
    pub tag: DataTag,
    /// Human-readable name (e.g. "Image").
    pub name: String,
    /// Description (e.g. "Raster image: PNG, JPEG, WebP").
    pub description: String,
    /// Editor color for wires/ports (e.g. "#4CAF50").
    pub color: String,
    /// Editor icon name (e.g. "image").
    pub icon: Option<String>,
    /// Compatibility: this tag can connect to ports accepting these tags.
    /// E.g. Image is compatible_with Binary (image IS binary).
    pub compatible_with: Vec<DataTag>,
    /// Who registered: "nebula-core" or "nebula-plugin-comfyui".
    pub registered_by: String,
}

impl DataTagRegistry {
    /// Register plugin-defined data tags. Unknown tags = registration error.
    pub fn register(&mut self, info: DataTagInfo) -> Result<(), RegistrationError> { ... }

    /// Check if source tag can connect to target port tags.
    /// Empty target = accepts anything (Json implicit).
    pub fn is_compatible(&self, source: &DataTag, target: &[DataTag]) -> bool {
        if target.is_empty() {
            return true; // untyped port accepts everything
        }
        target.iter().any(|t| t == source || self.is_subtype(source, t))
    }

    /// Transitive subtype check via BFS.
    /// ai.mask → image → binary: is_subtype(ai.mask, binary) = true.
    fn is_subtype(&self, source: &DataTag, target: &DataTag) -> bool {
        let mut visited = std::collections::HashSet::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(source.clone());

        while let Some(current) = queue.pop_front() {
            if !visited.insert(current.clone()) { continue; }
            if let Some(info) = self.tags.get(current.as_str()) {
                for compat in &info.compatible_with {
                    if compat == target { return true; }
                    queue.push_back(compat.clone());
                }
            }
        }
        false
    }
}
```

#### Domain tags (registered by plugins)

```rust
// nebula-plugin-ai registers:
registry.register(DataTagInfo {
    tag: DataTag::new("model"),
    name: "AI Model".into(),
    compatible_with: vec![],     // model only connects to model
    color: "#9C27B0".into(),     // purple
    ..
});
registry.register(DataTagInfo {
    tag: DataTag::new("embedding"),
    name: "Vector Embedding".into(),
    compatible_with: vec![DataTag::ARRAY], // embedding is also an array
    color: "#FF9800".into(),     // orange
    ..
});

// nebula-plugin-image registers:
registry.register(DataTagInfo {
    tag: DataTag::new("image"),
    name: "Image".into(),
    compatible_with: vec![DataTag::BINARY], // image IS binary
    color: "#4CAF50".into(),     // green
    ..
});
registry.register(DataTagInfo {
    tag: DataTag::new("mask"),
    name: "Image Mask".into(),
    compatible_with: vec![DataTag::new("image"), DataTag::BINARY],
    color: "#607D8B".into(),     // gray
    ..
});
```

#### Compatibility matrix

```
Source tag    → Target accepts     → Compatible?  → Why
─────────────────────────────────────────────────────────
Image        → [Image]            → ✅            → exact match
Image        → [Binary]           → ✅            → Image compatible_with Binary
Image        → []                 → ✅            → empty = untyped, accepts all
Text         → [Image]            → ❌            → incompatible
Number       → [Text, Number]     → ✅            → exact match on Number
Any tag      → []                 → ✅            → n8n mode: anything goes
(expression) → (any port)         → ✅ always     → expressions bypass tag check
```

#### Port declarations with tags

```rust
pub enum InputPort {
    Flow {
        key: PortKey,
        /// Accepted data types. Empty = accept anything (n8n default).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        accepts: Vec<DataTag>,
    },
    Support(SupportPort),
}

pub enum OutputPort {
    Flow {
        key: PortKey,
        kind: FlowKind,
        /// Data type produced. Empty = untyped JSON (n8n default).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        produces: Vec<DataTag>,
    },
    Dynamic(DynamicPort),
}
```

#### Usage examples

```rust
// ── n8n-style: untyped (default, no tags) ──
ActionMetadata::new("http.request", "HTTP Request", "...")
    .with_inputs(vec![InputPort::flow("in")])          // accepts: [] = any
    .with_outputs(vec![OutputPort::flow("out")])        // produces: [] = json

// ── ComfyUI-style: typed image pipeline ──
ActionMetadata::new("image.resize", "Resize Image", "...")
    .with_inputs(vec![
        InputPort::Flow { key: "image".into(), accepts: vec![DataTag::new("image")] },
        InputPort::Flow { key: "mask".into(),  accepts: vec![DataTag::new("mask")] },
    ])
    .with_outputs(vec![
        OutputPort::Flow {
            key: "image".into(),
            kind: FlowKind::Main,
            produces: vec![DataTag::new("image")],
        },
    ])

// ── Mixed: untyped flow + typed support (AI Agent) ──
ActionMetadata::new("ai.agent", "AI Agent", "...")
    .with_inputs(vec![
        InputPort::Flow { key: "in".into(), accepts: vec![] },  // any JSON
        InputPort::Support(SupportPort {
            key: "model".into(),
            required: true,
            multi: false,
            filter: ConnectionFilter::new()
                .with_allowed_tags(vec!["llm".into()]),
            ..
        }),
    ])
```

#### Editor behavior

Two editor modes determine how users connect data between nodes.
Action code is identical for both — this is purely a UI presentation choice.

```
Input-based mode (ComfyUI-style):
  → Parameters with DataTag become typed input PORTS
  → Wire = data binding (drag IMAGE output → IMAGE input)
  → Colored wires, type-checked connections
  → Best for: image pipelines, AI model chains, typed data flows

Parameter-based mode (n8n-style):
  → Parameters are expression fields with autocomplete
  → Wire = topology only (execution order)
  → Expressions pull data: {{ $node["S3"].output.file }}
  → Best for: API integrations, data transformations, general automation

Mixed mode (per-node or per-workspace setting):
  → AI pipeline nodes: input-based (typed ports)
  → Utility nodes (Set, If, HTTP): parameter-based (expressions)
  → Both coexist in same workflow
```

This is an **editor/UI concern**, not an action contract concern.
Action authors declare `#[param(tag = "image")]` — editor decides
how to render it based on user's preferred mode.

#### Plugin tag registration

Plugins register custom DataTags through the Plugin trait. Tags are validated
at registration time — not arbitrary strings.

**Via Plugin trait:**
```rust
impl Plugin for ShopifyPlugin {
    fn data_tags(&self) -> Vec<DataTagInfo> {
        vec![
            DataTagInfo {
                tag: DataTag::new("shopify.product"),
                name: "Shopify Product".into(),
                description: "Product object from Shopify API".into(),
                color: "#96BF48".into(),
                icon: Some("shopping-bag".into()),
                compatible_with: vec![DataTag::OBJECT, DataTag::JSON],
                registered_by: "nebula-plugin-shopify".into(),
            },
            DataTagInfo {
                tag: DataTag::new("shopify.order"),
                name: "Shopify Order".into(),
                description: "Order object from Shopify API".into(),
                color: "#96BF48".into(),
                icon: Some("receipt".into()),
                compatible_with: vec![DataTag::OBJECT, DataTag::JSON],
                registered_by: "nebula-plugin-shopify".into(),
            },
        ]
    }

    fn actions(&self) -> Vec<Arc<dyn ActionDescriptor>> {
        vec![
            Arc::new(GetProductDescriptor),  // output produces: ["shopify.product"]
            Arc::new(CreateOrderDescriptor), // input accepts: ["shopify.product"]
        ]
    }
}
```

**Via derive macro (inline with action):**
```rust
#[derive(Action)]
#[action(key = "shopify.get_product", name = "Get Product")]
#[data_tag(
    tag = "shopify.product",
    name = "Shopify Product",
    color = "#96BF48",
    compatible_with = ["object", "json"],
)]
struct GetShopifyProduct;
// Derive macro auto-adds tag to plugin's data_tags() return.
// Define once — available to all actions in the plugin.
```

**Runtime loads plugins in order:**
```rust
for plugin in plugins {
    // 1. Register tags FIRST (actions reference them)
    for tag_info in plugin.data_tags() {
        tag_registry.register(tag_info)?;
    }
    // 2. Register actions (can now reference plugin tags)
    for (desc, factory) in plugin.actions_with_factories() {
        action_registry.register(desc, factory)?;
    }
}
```

#### Tag registration validation

| Check | Error | Example |
|-------|-------|---------|
| Name format | `InvalidTagName` | `"SHOPIFY"` — must be lowercase dot-separated |
| Duplicate | `DuplicateTag` | `shopify.product` already registered |
| Unknown compatible_with | `UnknownTag` | References tag that doesn't exist yet |
| Circular compatibility | `CircularCompatibility` | A compat B compat A |
| Reserved namespace | `ReservedNamespace` | Plugin tries `ai.custom` — `ai.*` owned by nebula-ai |

```rust
impl DataTagRegistry {
    pub fn register(&mut self, info: DataTagInfo) -> Result<(), TagRegistrationError> {
        // 1. Validate name format: lowercase, dot-separated, no special chars
        Self::validate_name(&info.tag)?;

        // 2. Check namespace ownership
        Self::validate_namespace(&info.tag, &info.registered_by)?;

        // 3. Check for duplicates
        if self.tags.contains_key(info.tag.as_str()) {
            return Err(TagRegistrationError::DuplicateTag(info.tag.clone()));
        }

        // 4. Validate compatible_with references
        for compat in &info.compatible_with {
            if !self.tags.contains_key(compat.as_str()) {
                return Err(TagRegistrationError::UnknownTag(compat.clone()));
            }
        }

        // 5. Check for cycles: if any compat_with tag already has this tag
        //    in its transitive closure, adding this link creates a cycle.
        for compat in &info.compatible_with {
            if self.is_subtype(compat, &info.tag) {
                return Err(TagRegistrationError::CircularCompatibility(
                    info.tag.clone(), compat.clone()
                ));
            }
        }

        self.tags.insert(info.tag.as_str().to_string(), info);
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TagRegistrationError {
    #[error("invalid tag name: {0}")]
    InvalidTagName(String),
    #[error("duplicate tag: {0}")]
    DuplicateTag(DataTag),
    #[error("unknown compatible_with tag: {0}")]
    UnknownTag(DataTag),
    #[error("reserved namespace: {namespace} owned by {owner}")]
    ReservedNamespace { namespace: String, owner: String },
    #[error("circular compatibility: {0} ↔ {1}")]
    CircularCompatibility(DataTag, DataTag),
}
```

#### Namespace ownership

```
nebula-core:       json, text, number, boolean, array, object, binary, file, stream
nebula-media:      image, image.*, audio, video, pdf, document, spreadsheet, archive, font
nebula-ai:         ai.*
nebula-data:       data.*
nebula-comm:       html, markdown, xml, url, email, email.*, datetime, cron
{plugin-name}:     {plugin-namespace}.*
```

Plugin can only register tags in its own namespace.
`nebula-plugin-shopify` → `shopify.*` ✅
`nebula-plugin-shopify` → `ai.shopify` ❌ (`ai.*` owned by nebula-ai)

### Port patterns by node type

| Node type | Input ports | Output ports | Example |
|-----------|------------|-------------|---------|
| Simple action | `[Flow("in")]` | `[Flow("out", Main)]` | HTTP Request, Telegram Send |
| With error handling | `[Flow("in")]` | `[Flow("out", Main), Flow("error", Error)]` | HTTP Request, DB Query |
| If/Condition | `[Flow("in")]` | `[Flow("true", Main), Flow("false", Main)]` | If, Filter |
| Switch | `[Flow("in")]` | `[Dynamic("rule", "rules")]` | Switch, Router |
| Merge | `[Flow("a"), Flow("b")]` | `[Flow("out", Main)]` | Merge, Compare |
| SplitOut | `[Flow("in")]` | `[Flow("out", Main)]` | Split items to parallel |
| AI Agent | `[Flow("in"), Support("model"), Support("tools")]` | `[Flow("out", Main)]` | AI Agent, Chain |
| Trigger | `[]` (no input) | `[Flow("out", Main)]` | Webhook, Poll, Cron |
| ResourceAction | `[Flow("in")]` | `[Flow("out", Main)]` | Scoped DB Pool |

### Ordering

Port order in `Vec<InputPort>` / `Vec<OutputPort>` determines **UI layout order**.
First port = top position. Convention: main flow ports first, error port last,
support ports after flow.

---

## Execution Safety Policies

### Panic normalization (normative)

```
Panic in action code (execute, init_state, migrate_state):

⚠️  DO NOT use std::panic::catch_unwind around .await points.
    AssertUnwindSafe in async context can poison internal resources
    and cause deadlocks in the tokio executor.

✅  Use tokio::spawn + JoinError::is_panic() for isolation:

    let handle = tokio::spawn(async move {
        handler.execute(input, &ctx).await
    });
    match handle.await {
        Ok(Ok(result)) => result,                    // success
        Ok(Err(action_error)) => action_error,       // normal error
        Err(join_error) if join_error.is_panic() => { // panic caught!
            ActionError::Fatal {
                code: ErrorCode::ActionPanicked,
                message: format!("action panicked: {:?}", join_error),
            }
        }
        Err(join_error) => {                         // cancelled
            ActionError::Cancelled
        }
    }

Post-panic recovery:
1. State = pre-execute snapshot (last durably committed)
2. Mark node as panicked in execution log
3. No routing side effects
4. Optional: quarantine action after N panics (runtime policy)
```

### Pre-execute state snapshot (adapter contract)

```
1. Serialize current typed_state → pre_execute_snapshot
2. Call handler.execute(...)
3. If panic/error → engine uses pre_execute_snapshot
4. If Ok → use returned next_state
```

---

## Derive Macros

### #[derive(Action)]

```rust
#[derive(Action)]
#[action(
    key = "telegram.send",
    version = 2,                    // ← interface version (default: 1)
    name = "Send Message",
    category = "messaging",
)]
#[credential(BearerToken, key = "bot_token")]
#[resource(HttpClient, key = "http")]
struct SendTelegramMessage;

// Generates: Action impl (with InterfaceVersion), ActionDependencies,
// ActionDescriptor, ActionFactory, CapabilityManifest, and Deps struct:
pub struct SendTelegramMessageDeps {
    pub bot_token: BearerToken,
    pub http: HttpClient,
}
impl SendTelegramMessageDeps {
    pub async fn resolve(ctx: &ActionContext) -> Result<Self, ActionError> { ... }
}
```

### #[derive(ActionInput)]

Single-source: Deserialize + ParameterCollection + binding.

```rust
#[derive(ActionInput, Deserialize)]
pub struct HttpRequestInput {
    #[param(label = "Method", one_of("GET", "POST", "PUT", "DELETE"), default = "GET")]
    pub method: String,
    #[param(label = "URL")]
    pub url: String,                              // field name = key, non-Option = required
    #[param(label = "Headers")]
    pub headers: Option<HashMap<String, String>>, // Option = optional
    #[param(label = "Body", visible_if = "method in ['POST', 'PUT']")]
    pub body_json: Option<serde_json::Value>,
}
// Generates: fn parameters() -> ParameterCollection
```

### #[derive(ActionDeps)]

```rust
#[derive(ActionDeps)]
pub struct SlackDeps {
    #[dep(resource = "http")]
    pub http: HttpClient,
    #[dep(credential = "bot_token")]
    pub token: BearerToken,
}
// Generates: async fn resolve(ctx: &impl ResourceProvider) -> Result<Self, ActionError>
// Usage: let SlackDeps { http, token } = SlackDeps::resolve(ctx).await?;
```

**Works with BOTH ActionContext and TriggerContext** via shared trait:

```rust
/// Shared capability for resource/credential access.
/// Implemented by ActionContext and TriggerContext.
/// ActionDeps::resolve() is generic over this trait.
pub trait ResourceProvider: Send + Sync {
    async fn resource_typed<R: Send + Sync + 'static>(&self, key: &str) -> Result<R, ActionError>;
    async fn credential_typed<S: Send + Sync + 'static>(&self, key: &str) -> Result<S, ActionError>;
}
```

This means the same `#[derive(ActionDeps)]` struct works in actions AND triggers:
```rust
// In SimpleAction:
let deps = SlackDeps::resolve(ctx).await?;  // ctx: &ActionContext

// In WebhookAction:
let deps = SlackDeps::resolve(ctx).await?;  // ctx: &TriggerContext

// Same struct, same derive, both contexts. Zero boilerplate.
```

### #[derive(ParameterEnum)]

```rust
#[derive(Serialize, Deserialize, ParameterEnum)]
pub enum HttpMethod { GET, POST, PUT, DELETE }
// Generates: Select parameter with options from enum variants
```
