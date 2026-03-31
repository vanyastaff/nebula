# nebula-action v2 — HLD Addendum: Agent, Pipeline, Provide Ports

> **Source:** Architecture session, Claude (lead architect) + Vanya (lead architect).
> **Status:** Draft — pending adversarial review (ChatGPT, DeepSeek, Gemini).
> **Scope:** Post-freeze additions to nebula-action v2 HLD. Does not modify frozen decisions.
> **Builds on:** 8 HLD docs + DataTag registry (~4558 lines frozen).

---

## 1. Executive Summary

Six architectural additions to nebula-action, motivated by competitive analysis
(LangGraph, CrewAI, AutoGen, ComfyUI, Blender, Deckard audio graph) and production
use-case analysis (AI agents, real-time media processing, cached incremental execution).

| Addition | What | Why |
|----------|------|-----|
| **AgentAction** | 5th core type with AgentContext | Different execution model: internal loop, tools, budget, streaming |
| **OutputPort::Provide** | 3rd port variant (Support ↔ Provide) | Symmetric capability declaration: supplier-side was missing |
| **#[provide(tool(...))]** | Tool provision on any action type | No separate ToolAction trait; one mechanism for all tool supply |
| **Task\<T\>** | Structured concurrency on all contexts | ctx.spawn() / ctx.background() with cancel-on-drop |
| **StreamProcessor** | DX type for real-time pipelines | Audio/video/MIDI/IoT processing without async overhead |
| **CachePolicy** | Incremental re-execution (ComfyUI-mode) | Skip unchanged nodes, cache outputs, dirty propagation |

**What does NOT change:** 4 existing core types, ActionResult enum, ActionError model,
ActionContext/TriggerContext shape, existing port system, DataTag system, handler/registry
architecture, derive macros, testing infrastructure.

---

## 2. AgentAction — 5th Core Type

### 2.1 Motivation

The decisive argument is **context separation**, not execution model.

Existing principle (already accepted): different capabilities → different contexts.
TriggerAction has TriggerContext because triggers need emit/checkpoint/scheduler
that StatelessAction should never see. By the same principle, agents need
tools/budget/streaming that StatelessAction should never see.

If these capabilities land on ActionContext, TransformAction sees invoke_tool().
That's capability pollution.

```
ActionContext   — resource, credential, ports, call_action, heartbeat
                  for StatelessAction, StatefulAction, ResourceAction

TriggerContext  — resource, credential, emit, checkpoint, scheduler, parameters
                  for TriggerAction

AgentContext    — resource, credential, tools, invoke_tool, stream, budget, usage
                  for AgentAction                                          [NEW]
```

Secondary argument: StatefulAction + Continue forces engine-driven iteration
(persist → requeue → rebind → execute per LLM round). Agent loop needs internal
iteration — one engine step, many LLM rounds. This is the same distinction as
TriggerAction::run() vs StatefulAction::execute().

### 2.2 Core Trait

```rust
pub trait AgentAction: Action {
    type Input: DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + Send + Sync + 'static;

    /// Run agent loop. Single engine step. Agent controls iteration.
    ///
    /// - resume: if Some, agent was parked (approval, callback) and is resuming
    /// - Returns Complete(output) or Park(waiting for external event)
    fn run(
        &self,
        input: Self::Input,
        resume: Option<AgentCheckpoint>,
        ctx: &AgentContext,
    ) -> impl Future<Output = Result<AgentOutcome<Self::Output>, ActionError>> + Send;
}

pub enum AgentOutcome<T> {
    /// Agent finished, here's the output.
    Complete(T),
    /// Agent needs external input (approval, callback) to continue.
    /// Engine checkpoints state, creates wait condition, resumes later.
    Park(AgentPark),
}

pub enum AgentPark {
    WaitingApproval { reason: String, timeout: Duration },
    WaitingCallback { callback_id: String, timeout: Duration },
}

/// Serializable checkpoint for resuming a parked agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCheckpoint {
    pub messages: serde_json::Value,
    pub iteration: u32,
    pub pending_call: Option<serde_json::Value>,
    pub custom: Option<serde_json::Value>,
}
```

### 2.3 AgentContext

```rust
pub struct AgentContext {
    pub execution_id: ExecutionId,
    pub node_id: NodeId,
    pub workflow_id: WorkflowId,
    pub cancellation: CancellationToken,
    guard: ExecutionGuard,
    // Shared with ActionContext
    resources: Arc<dyn ResourceAccessor>,
    credentials: Arc<dyn CredentialAccessor>,
    // Agent-specific
    tools: Vec<ResolvedTool>,
    tool_router: Arc<dyn ToolRouter>,
    budget: Arc<AgentBudget>,
    usage: Arc<Mutex<AgentUsage>>,
    stream: Arc<dyn AgentStreamSink>,
}

impl AgentContext {
    // ── Shared capabilities ──
    pub async fn resource_typed<R>(&self, key: &str) -> Result<R, ActionError> { ... }
    pub async fn credential_typed<S>(&self, key: &str) -> Result<S, ActionError> { ... }
    pub fn is_cancelled(&self) -> bool { ... }

    // ── Tools ──
    pub fn tools(&self) -> &[ResolvedTool] { ... }
    pub fn tool_specs(&self) -> Vec<&ToolSpec> { ... }
    pub async fn invoke_tool(&self, call: ToolCall) -> Result<ToolResult, ActionError> { ... }
    pub async fn invoke_tools_parallel(&self, calls: Vec<ToolCall>)
        -> Vec<Result<ToolResult, ActionError>> { ... }

    // ── Streaming ──
    pub async fn stream_event(&self, event: StreamEvent<'_>) -> Result<(), ActionError> { ... }

    // ── Usage tracking ──
    pub fn record_usage(&self, usage: LlmUsage) -> Result<(), ActionError> { ... }
    pub fn total_usage(&self) -> AgentUsage { ... }

    // ── Budget ──
    pub fn check_budget(&self) -> Result<(), ActionError> { ... }

    // ── Task system (see §5) ──
    pub fn spawn<F, T>(&self, future: F) -> Task<T> { ... }
    pub fn background<F, T>(&self, f: F) -> Task<T> { ... }
}

pub struct ResolvedTool {
    pub spec: ToolSpec,
    pub source_node: NodeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBudget {
    pub max_iterations: Option<u32>,
    pub max_tokens: Option<u64>,
    pub max_tool_calls: Option<u32>,
    pub max_duration: Option<Duration>,
    pub max_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct AgentUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_cost_usd: f64,
    pub llm_calls: u32,
    pub tool_calls: u32,
    pub iterations: u32,
}

#[derive(Debug, Clone, Serialize)]
pub enum StreamEvent<'a> {
    Text(&'a str),
    ToolStart(&'a ToolCall),
    ToolResult { call_id: &'a str, result: &'a ToolResult },
    Progress { current: u64, total: Option<u64> },
    Log { level: LogLevel, message: &'a str },
}
```

### 2.4 DX Types

All blanket-impl to AgentAction. Engine never sees DX types.

```
AgentAction (core)
├── ReActAgent        — tool-use loop (most common agent pattern)
├── PlanExecuteAgent  — plan steps → execute → optionally revise → synthesize
├── SupervisorAgent   — delegate to sub-agents, parallel or sequential
└── RouterAgent       — classify input → route to port (single LLM call, no loop)
```

#### ReActAgent

```rust
pub trait ReActAgent: Action {
    type Input: DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + Send + Sync + 'static;

    fn system_prompt(&self, input: &Self::Input, ctx: &AgentContext)
        -> Result<String, ActionError>;

    fn user_message(&self, input: &Self::Input)
        -> Result<String, ActionError>;

    fn process_answer(
        &self, input: Self::Input, answer: String, ctx: &AgentContext,
    ) -> impl Future<Output = Result<Self::Output, ActionError>> + Send;

    /// Guardrail: called before each tool invocation.
    fn before_tool_call(
        &self, call: &ToolCall, input: &Self::Input, ctx: &AgentContext,
    ) -> impl Future<Output = Result<ToolCallDecision, ActionError>> + Send {
        async { Ok(ToolCallDecision::Allow) }
    }

    /// Message history management between iterations.
    fn history_strategy(&self) -> HistoryStrategy {
        HistoryStrategy::Full
    }

    /// Post-process tool result before adding to history.
    fn process_tool_result(
        &self, call: &ToolCall, result: &ToolResult, ctx: &AgentContext,
    ) -> impl Future<Output = Result<String, ActionError>> + Send { ... }

    /// Chat options per iteration.
    fn chat_options(&self, iteration: u32) -> ChatOptions {
        ChatOptions { temperature: Some(0.0), ..Default::default() }
    }
}

pub enum ToolCallDecision {
    Allow,
    Deny { reason: String },
    RequireApproval { reason: String },
}

pub enum HistoryStrategy {
    Full,
    SlidingWindow(usize),
    Summarize,
}
```

#### PlanExecuteAgent

```rust
pub trait PlanExecuteAgent: Action {
    type Input: DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + Send + Sync + 'static;
    type Step: Serialize + DeserializeOwned + Send + Sync + 'static;

    fn plan(&self, input: &Self::Input, ctx: &AgentContext)
        -> impl Future<Output = Result<Vec<Self::Step>, ActionError>> + Send;

    fn execute_step(&self, step: &Self::Step, previous: &[StepResult], ctx: &AgentContext)
        -> impl Future<Output = Result<StepResult, ActionError>> + Send;

    /// Optional: revise remaining plan based on intermediate results.
    fn revise_plan(&self, remaining: Vec<Self::Step>, results: &[StepResult], ctx: &AgentContext)
        -> impl Future<Output = Result<Vec<Self::Step>, ActionError>> + Send {
        async { Ok(remaining) }
    }

    fn synthesize(&self, input: Self::Input, results: Vec<StepResult>, ctx: &AgentContext)
        -> impl Future<Output = Result<Self::Output, ActionError>> + Send;
}
```

#### SupervisorAgent

```rust
pub trait SupervisorAgent: Action {
    type Input: DeserializeOwned + Send + Sync + 'static;
    type Output: Serialize + Send + Sync + 'static;

    fn route(&self, input: &Self::Input, history: &[DelegationResult], ctx: &AgentContext)
        -> impl Future<Output = Result<Delegation, ActionError>> + Send;

    fn synthesize(&self, input: Self::Input, results: Vec<DelegationResult>, ctx: &AgentContext)
        -> impl Future<Output = Result<Self::Output, ActionError>> + Send;
}

pub enum Delegation {
    Delegate { agent: String, input: serde_json::Value },
    Parallel(Vec<DelegationTarget>),
    Complete,
}
```

#### RouterAgent

```rust
pub trait RouterAgent: Action {
    type Input: DeserializeOwned + Send + Sync + 'static;

    fn routes(&self) -> Vec<Route>;

    fn system_prompt(&self, input: &Self::Input) -> Result<String, ActionError> {
        Ok("Classify the input and select the appropriate route.".into())
    }
}

// Blanket impl → AgentAction → returns ActionResult::Route { port }
```

### 2.5 Handler and Registry

```rust
// ActionInstance — closed enum, adding variant = major bump
pub enum ActionInstance {
    Stateless(Box<dyn StatelessHandler>),
    Stateful(Box<dyn StatefulHandler>),
    Trigger(Box<dyn TriggerHandler>),
    Resource(Box<dyn ResourceHandler>),
    Agent(Box<dyn AgentHandler>),          // NEW
}

pub enum ActionKind {
    Stateless,
    Stateful,
    Trigger,
    Resource,
    Agent,                                  // NEW
}

#[async_trait]
pub trait AgentHandler: Send + Sync {
    async fn run(
        &self,
        input: serde_json::Value,
        resume: Option<AgentCheckpoint>,
        ctx: AgentContext,
    ) -> Result<AgentHandlerResult, ActionError>;
}

pub struct AgentHandlerResult {
    pub outcome: AgentOutcome<serde_json::Value>,
    pub usage: AgentUsage,
}
```

### 2.6 Engine Behavior

Engine при виде AgentAction node:
1. Resolves Provide→Support edges: collects tool handles from connected ToolProvider nodes
2. Builds AgentContext with tool router, stream sink, budget from metadata
3. Calls run(input, resume, ctx) in dedicated tokio task
4. On Complete: routes output through standard ActionResult→Port pipeline
5. On Park: checkpoints state, creates WaitCondition, resumes later
6. After completion: records AgentUsage in execution metadata

---

## 3. Port System: OutputPort::Provide

### 3.1 Problem

InputPort has Support variant (consumer declares "I accept capability").
OutputPort has no counterpart (supplier cannot declare "I provide capability").
Connection validity is guessed through ConnectionFilter with allowed_tags — fragile.

### 3.2 Solution: Provide ↔ Support Pair

```rust
pub enum InputPort {
    Flow { key: PortKey },
    Support(SupportPort),           // unchanged
}

pub enum OutputPort {
    Flow { key: PortKey, kind: FlowKind },
    Dynamic(DynamicPort),
    Provide(ProvidePort),           // NEW
}

pub struct ProvidePort {
    pub key: PortKey,
    pub name: String,
    pub kind: ProvideKind,
    pub data_tag: Option<DataTag>,
}

pub enum ProvideKind {
    /// Static data: model reference, config, embedding.
    Data { schema: Option<serde_json::Value> },
    /// Callable tool(s): agent can invoke this node.
    Tool { specs: Vec<ToolSpec> },
    /// Resource handle: DB pool, memory store.
    Resource,
}

impl OutputPort {
    pub fn provide_tool(key: impl Into<PortKey>, name: impl Into<String>,
        parameters_schema: serde_json::Value) -> Self { ... }
    pub fn provide_data(key: impl Into<PortKey>, name: impl Into<String>) -> Self { ... }
    pub fn provide_resource(key: impl Into<PortKey>, name: impl Into<String>) -> Self { ... }
    pub fn is_provide(&self) -> bool { ... }
}
```

### 3.3 ConnectionFilter Enhancement

```rust
pub struct ConnectionFilter {
    pub allowed_node_types: Option<Vec<String>>,  // existing
    pub allowed_tags: Option<Vec<String>>,          // existing
    pub allowed_data_tags: Option<Vec<DataTag>>,   // NEW
    pub required_provide: Option<ProvideKindFilter>, // NEW
}

pub enum ProvideKindFilter { Data, Tool, Resource }

impl ConnectionFilter {
    /// Editor calls this to validate wire compatibility.
    pub fn accepts(&self, supply: &ProvidePort) -> bool { ... }
}
```

---

## 4. Tool Provision via #[provide(tool(...))]

### 4.1 Principle

Tool provision is a **port concern**, not a type concern. No separate ToolAction trait.
Any action type can declare #[provide(tool(...))] — the macro adds OutputPort::Provide(Tool).

### 4.2 How It Works

```rust
// SimpleAction node declares it can be used as a tool:
#[derive(Action)]
#[action(key = "tool.google_search", name = "Google Search")]
#[provide(tool(name = "google_search", description = "Search the web"))]
struct GoogleSearch;

impl SimpleAction for GoogleSearch {
    type Input = SearchInput;      // also used as tool parameters schema
    type Output = SearchOutput;
    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> Result<Self::Output, ActionError> { ... }
}
```

The same execute() is called whether the node is invoked via flow edge or as a tool.
Engine handles the adaptation:
- Flow edge: deserialize JSON → Input, call execute(), serialize Output → downstream
- Tool invocation: deserialize ToolCall.args → Input, call execute(), wrap Output → ToolResult

### 4.3 Agent-as-Tool

AgentAction nodes automatically provide themselves as tools when connected
to another agent's Support("tools") port:

```rust
#[derive(Action)]
#[action(key = "ai.research_agent", name = "Research Agent")]
#[provide(tool(name = "research", description = "Research a topic"))]
struct ResearchAgent;

impl ReActAgent for ResearchAgent { ... }

// When Analyst Agent calls ctx.invoke_tool("research", args):
//   Engine deserializes args → ResearchInput
//   Engine builds AgentContext for Research Agent (resolves its own tools, model)
//   Engine calls ResearchAgent.run(input, None, agent_ctx)
//   Engine wraps ResearchOutput → ToolResult::Value(...)
```

### 4.4 Tool Types

```rust
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,   // JSON Schema
    pub hints: ToolHints,
}

pub struct ToolHints {
    pub return_direct: bool,             // result goes straight to user
    pub has_side_effects: bool,          // writes, sends, deletes
    pub destructive: bool,              // stronger than side_effects
    pub idempotent: bool,               // safe to retry
    pub cost_estimate_usd: Option<f64>, // budget planning
    pub rate_limit_per_min: Option<u32>,
    pub accepts_binary: bool,
    pub returns_binary: bool,
}

pub struct ToolCall {
    pub tool_name: String,
    pub call_id: String,
    pub args: serde_json::Value,
    pub binary_inputs: Vec<BinaryData>,
}

pub enum ToolResult {
    Value(serde_json::Value),
    Binary { data: BinaryData, summary: Option<String> },
    Direct(serde_json::Value),           // return_direct: bypass LLM
    Error { message: String, retryable: bool },
}
```

### 4.5 Multi-Operation Tools

Nodes providing multiple tools (e.g., SQL node: query + schema + execute):

```rust
#[derive(Action)]
#[action(key = "tool.sql", name = "SQL Tools")]
#[provide(tool(name = "sql_query", description = "Read-only SQL", input = SqlQueryInput))]
#[provide(tool(name = "sql_schema", description = "Get table schema", input = SqlSchemaInput))]
#[provide(tool(name = "sql_execute", description = "Write SQL", input = SqlExecInput,
    hints(side_effects)))]
struct SqlTool;

// Input is dispatched by tool_name:
#[derive(Deserialize)]
#[serde(tag = "tool", content = "args")]
enum SqlOperation {
    #[serde(rename = "sql_query")]   Query(SqlQueryInput),
    #[serde(rename = "sql_schema")]  Schema(SqlSchemaInput),
    #[serde(rename = "sql_execute")] Execute(SqlExecInput),
}

impl SimpleAction for SqlTool {
    type Input = SqlOperation;
    type Output = serde_json::Value;
    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> Result<Self::Output, ActionError> {
        match input {
            SqlOperation::Query(q) => { ... }
            SqlOperation::Schema(s) => { ... }
            SqlOperation::Execute(e) => { ... }
        }
    }
}
```

### 4.6 Stateful Tools (Session)

Browser, REPL, and similar session-based tools use StatefulAction:

```rust
#[derive(Action)]
#[action(key = "tool.browser", name = "Web Browser")]
#[provide(tool(name = "browse_navigate", description = "Navigate to URL"))]
#[provide(tool(name = "browse_click", description = "Click element"))]
#[provide(tool(name = "browse_extract", description = "Extract text"))]
struct BrowserTool;

impl StatefulAction for BrowserTool {
    type Input = BrowserOperation;
    type Output = serde_json::Value;
    type State = BrowserSession;   // { session_id, current_url }
    // Engine tracks session state across tool calls within one agent run.
}
```

---

## 5. Task System — Structured Concurrency

### 5.1 Motivation

Action authors need parallel execution (parallel HTTP calls, parallel tool
invocations) without importing tokio directly. Spawned tasks must respect
execution lifecycle (cancellation, panic safety).

### 5.2 Task<T>

```rust
/// Spawned task handle. Cancels on drop (structured concurrency).
pub struct Task<T> {
    handle: tokio::task::JoinHandle<Result<T, ActionError>>,
}

impl<T: Send + 'static> Task<T> {
    pub async fn await_result(self) -> Result<T, ActionError> {
        match self.handle.await {
            Ok(result) => result,
            Err(e) if e.is_panic() =>
                Err(ActionError::fatal(format!("task panicked: {e}"))),
            Err(_) => Err(ActionError::Cancelled),
        }
    }

    /// Join all, fail-fast on first error.
    pub async fn join_all(tasks: Vec<Task<T>>) -> Result<Vec<T>, ActionError> {
        let mut results = Vec::with_capacity(tasks.len());
        for task in tasks {
            results.push(task.await_result().await?);
        }
        Ok(results)
    }

    /// First success wins, rest auto-cancelled (dropped).
    pub async fn race(tasks: Vec<Task<T>>) -> Result<T, ActionError> { ... }
}

impl<T> Drop for Task<T> {
    fn drop(&mut self) { self.handle.abort(); }
}
```

### 5.3 Context Methods

Same two methods on all three contexts:

```rust
impl ActionContext {
    /// Spawn async task tied to this execution's cancellation.
    pub fn spawn<F, T>(&self, future: F) -> Task<T>
    where
        F: Future<Output = Result<T, ActionError>> + Send + 'static,
        T: Send + 'static,
    {
        let cancel = self.cancellation.clone();
        Task {
            handle: tokio::spawn(async move {
                tokio::select! {
                    result = future => result,
                    _ = cancel.cancelled() => Err(ActionError::Cancelled),
                }
            }),
        }
    }

    /// CPU-bound work on blocking thread pool.
    pub fn background<F, T>(&self, f: F) -> Task<T>
    where
        F: FnOnce() -> Result<T, ActionError> + Send + 'static,
        T: Send + 'static,
    {
        Task {
            handle: tokio::spawn(async {
                tokio::task::spawn_blocking(f).await
                    .map_err(|e| ActionError::fatal(format!("panicked: {e}")))?
            }),
        }
    }
}

// Identical methods on TriggerContext and AgentContext.
```

### 5.4 Usage Patterns

```rust
// Parallel tool calls (agent):
let tasks: Vec<Task<ToolResult>> = tool_calls.into_iter()
    .map(|call| {
        let router = ctx.tool_router();
        ctx.spawn(async move { router.invoke(call).await })
    })
    .collect();
let results = Task::join_all(tasks).await?;

// Parallel HTTP (any action):
let tasks: Vec<Task<Response>> = urls.iter()
    .map(|url| {
        let http = http.clone();
        let url = url.clone();
        ctx.spawn(async move { http.get(&url).send().await.retryable() })
    })
    .collect();
let responses = Task::join_all(tasks).await?;

// Race — fastest provider:
let fastest = Task::race(vec![
    ctx.spawn(async move { provider_a.query(&q).await.retryable() }),
    ctx.spawn(async move { provider_b.query(&q).await.retryable() }),
]).await?;

// CPU-bound:
let parsed = ctx.background(move || {
    serde_json::from_str::<Heavy>(&payload).fatal()
}).await_result().await?;
```

### 5.5 Guarantees

| Property | Guarantee |
|----------|-----------|
| Cancellation | Task cancelled when ctx.cancellation fires |
| Drop safety | Task::drop() aborts the tokio task |
| Panic isolation | JoinError::is_panic() → ActionError::Fatal |
| Guard | ExecutionGuard shared — post-completion spawn returns error |

---

## 6. CachePolicy — ComfyUI-Style Incremental Re-Execution

### 6.1 Motivation

ComfyUI and Blender node editors achieve "real-time" feel through three
engine-level features, none of which require Action trait changes:

1. **Output cache** — node output saved, reused when input unchanged
2. **Dirty propagation** — parameter change marks node + all downstream dirty
3. **Incremental execution** — execute ONLY dirty nodes, skip cached

### 6.2 CachePolicy in ActionMetadata

```rust
pub struct ActionMetadata {
    // ... existing fields ...
    pub cache_policy: CachePolicy,    // NEW
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CachePolicy {
    /// Never cache (side effects, non-deterministic). Default.
    Disabled,
    /// Cache indefinitely (pure function, same input → same output).
    Enabled,
    /// Cache with time-to-live.
    Ttl(Duration),
    /// Cache with max entries (LRU eviction).
    Lru(usize),
}

impl Default for CachePolicy {
    fn default() -> Self { Self::Disabled }
}
```

### 6.3 Derive Macro

```rust
#[derive(Action)]
#[action(key = "ai.clip_encode", name = "CLIP Text Encode")]
#[cache(enabled)]           // pure function, safe to cache
struct ClipTextEncode;

#[derive(Action)]
#[action(key = "http.request", name = "HTTP Request")]
#[cache(disabled)]          // side effects, never cache
struct HttpRequest;

#[derive(Action)]
#[action(key = "ai.ksampler", name = "KSampler")]
#[cache(ttl = "5m")]        // seed-dependent, cache briefly
struct KSampler;
```

### 6.4 Cache Key (Content-Addressed)

Engine computes cache key from action identity + serialized input hash.
Same input → same output → skip execution. Implementation lives in
nebula-engine, not nebula-action.

```rust
// nebula-engine (pseudocode):
struct NodeCacheKey {
    action_key: ActionKey,
    action_version: InterfaceVersion,
    input_hash: u64,   // hash of serialized input
}
```

### 6.5 Which ActionResults Are Cacheable

```
ActionResult::Success    → cacheable ✓
ActionResult::Skip       → cacheable ✓  (deterministic)
ActionResult::Branch     → cacheable ✓  (deterministic routing)
ActionResult::Wait       → NOT cacheable (external dependency)
ActionResult::Retry      → NOT cacheable (transient state)
ActionResult::Continue   → NOT cacheable (mid-iteration)
```

### 6.6 Engine Behavior (nebula-engine, not nebula-action)

Engine on graph execution:
1. Topo-sort nodes
2. For each node: compute cache_key(action, version, input_hash)
3. If cached + not dirty → skip, use cached output
4. If dirty or not cached → execute → cache result if policy allows
5. On parameter change → mark node + all downstream dirty → invalidate cache

---

## 7. StreamProcessor — Real-Time Pipeline DX

### 7.1 Motivation

Real-time media processing (audio effects, video filters, MIDI, IoT sensors)
requires sync tight loop with microsecond latency. Async execute() with JSON
serialization is too slow for 44.1kHz audio processing.

This is a **DX type** (blanket-impls to SimpleAction), not a 6th core type.
Executed by nebula-pipeline (separate crate), not nebula-engine.

### 7.2 Trait

```rust
pub trait StreamProcessor: Action {
    /// Setup when pipeline starts.
    fn init(&mut self, config: &PipelineConfig) {}

    /// Process one tick. Engine calls at appropriate rate.
    /// MUST be fast (<1ms). No async. No I/O.
    /// &mut self: in-memory state between blocks (phase, buffers).
    fn tick(&mut self, io: &mut StreamIO, clock: u64);

    /// Cleanup when pipeline stops.
    fn reset(&mut self) {}
}

pub struct PipelineConfig {
    pub sample_rate: Option<u32>,    // audio: 44100
    pub block_size: usize,           // 256 frames per chunk
    pub max_latency: Duration,       // 10ms target
}
```

### 7.3 StreamIO — Typed Multi-Media I/O

```rust
pub struct StreamIO { /* engine-managed port buffers */ }

impl StreamIO {
    /// Audio: read/write f32 sample blocks.
    pub fn audio_in(&self, port: &str) -> &[f32] { ... }
    pub fn audio_out(&mut self, port: &str) -> &mut [f32] { ... }

    /// Video: read/write single frames.
    pub fn frame_in<F: FrameData>(&self, port: &str) -> Option<&F> { ... }
    pub fn frame_out<F: FrameData>(&mut self, port: &str, frame: F) { ... }

    /// Events (MIDI, messages, IoT): read/write event batches.
    pub fn events_in<E: EventData>(&self, port: &str) -> &[E] { ... }
    pub fn events_out<E: EventData>(&mut self, port: &str, events: &[E]) { ... }

    /// Block size for this tick.
    pub fn frames(&self) -> usize { ... }
}
```

### 7.4 Blanket Impl → SimpleAction

```rust
// In workflow mode, StreamProcessor works as standard SimpleAction:
impl<T: StreamProcessor + Clone> SimpleAction for T {
    type Input = StreamInput;     // { data, config }
    type Output = StreamOutput;

    async fn execute(&self, input: Self::Input, _ctx: &ActionContext)
        -> Result<Self::Output, ActionError>
    {
        let mut proc = self.clone();
        proc.init(&input.config);
        let result = proc.process_all(&input.data);
        Ok(result)
    }
}
```

### 7.5 Cross-Media Nodes

Nodes that convert between media types (MIDI → audio, text → audio, video → events)
implement StreamProcessor with mixed port types:

```rust
// Synth: MIDI events in → audio samples out
impl StreamProcessor for PolySynth {
    fn tick(&mut self, io: &mut StreamIO, _clock: u64) {
        for event in io.events_in::<MidiEvent>("midi") {
            // allocate/release voices
        }
        let output = io.audio_out("audio");
        for voice in &mut self.voices {
            voice.render(output, io.frames());
        }
    }
}
```

---

## 8. Two Engines, One Protocol

### 8.1 Architecture

```
nebula-action (protocol crate — shared)
├── Action trait, ActionMetadata, Ports, DataTags
├── Registry, Plugin, ActionDescriptor, ActionFactory
├── StatelessAction, StatefulAction, TriggerAction, ResourceAction, AgentAction
├── StreamProcessor (DX type)
├── Task<T> system
└── CachePolicy

nebula-engine (workflow execution)
├── Async, durable, JSON serialization
├── ActionResult routing, retry, state persistence
├── Error recovery, human-in-the-loop
├── Cache + dirty propagation (ComfyUI mode)
└── Uses: StatelessAction, StatefulAction, TriggerAction, ResourceAction, AgentAction

nebula-pipeline (streaming execution)  [FUTURE]
├── Sync tight loop, zero-copy buffers
├── Real-time constraint (<10ms latency)
├── No error recovery (show must go on)
├── Tick-based: audio 172Hz, video 30Hz, MIDI per-tick
└── Uses: StreamProcessor::tick() directly

nebula-editor (UI — shared)
├── Same drag-drop for both
├── Same port wiring, same registry
├── Workflow canvas → nebula-engine
└── Pipeline canvas → nebula-pipeline
```

### 8.2 Bridge: Pipeline as Sub-Graph in Workflow

```rust
// Engine wraps pipeline execution as one "node":
impl SimpleAction for PipelineRunner {
    type Input = PipelineInput;    // { audio: BinaryData, graph: PipelineGraph }
    type Output = BinaryData;

    async fn execute(&self, input: Self::Input, ctx: &ActionContext)
        -> Result<Self::Output, ActionError>
    {
        let pipeline = CompiledPipeline::from_graph(input.graph)?;
        ctx.background(move || pipeline.process_file(&input.audio))
            .await_result().await
    }
}
```

---

## 9. nebula-ai Crate (Dependency for Agent DX)

### 9.1 Motivation

ReActAgent blanket impl needs to call LLM and parse tool_calls.
Without a normalized LLM interface, every agent author writes their own
provider-specific parsing. LlmClient trait + normalized types solve this.

### 9.2 Core Types

```rust
// nebula-ai crate

#[async_trait]
pub trait LlmClient: Send + Sync + 'static {
    async fn chat(&self, messages: &[Message], tools: &[&ToolSpec],
        options: &ChatOptions) -> Result<ChatResponse, LlmError>;
    async fn chat_stream(&self, messages: &[Message], tools: &[&ToolSpec],
        options: &ChatOptions) -> Result<ChatStream, LlmError>;
}

pub struct Message {
    pub role: Role,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub attachments: Vec<Attachment>,
}

pub enum Role { System, User, Assistant, Tool }

pub struct ChatResponse {
    pub message: Message,
    pub tool_calls: Option<Vec<ToolCallRequest>>,
    pub usage: LlmUsage,
    pub finish_reason: FinishReason,
    pub model: String,
}

pub struct ChatOptions {
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stop_sequences: Vec<String>,
    pub response_format: Option<ResponseFormat>,
}

pub struct LlmUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

pub enum FinishReason { Stop, ToolUse, MaxTokens, ContentFilter }
pub enum ResponseFormat { Text, Json, JsonSchema(serde_json::Value) }
```

### 9.3 Crate Dependency

```
nebula-core → nebula-parameter → nebula-action → nebula-ai (optional)
                                                       ↑
                                              ReActAgent blanket impl
                                              depends on LlmClient trait
```

nebula-ai is an optional dependency of nebula-action, gated behind `ai` feature flag.
Agent DX types (ReActAgent, PlanExecuteAgent) require `nebula-action/ai`.
Core types (AgentAction, AgentContext) do NOT depend on nebula-ai.

---

## 10. Updated Type Hierarchy

```
Action (base trait — metadata only, object-safe)
│
├── StatelessAction       ctx: &ActionContext
│   ├── SimpleAction      (Result<O> → auto Success)
│   ├── TransformAction   (sync, no ctx)
│   └── StreamProcessor   (real-time, blanket → SimpleAction)     [NEW]
│
├── StatefulAction        ctx: &ActionContext + &mut State
│   ├── PaginatedAction   (fetch_page + cursor)
│   ├── BatchAction       (chunked processing)
│   ├── InteractiveAction (human-in-the-loop, epoch)
│   └── TransactionalAction (saga compensate)
│
├── TriggerAction         ctx: &TriggerContext
│   ├── EventTrigger      (from Resource EventSource)
│   ├── WebhookAction     (HTTP + signature + lifecycle)
│   ├── RawWebhookAction  (non-JSON webhooks)
│   ├── PollAction        (periodic + cursor + error policy)
│   └── ScheduledTrigger  (cron/interval/one-time)
│
├── AgentAction           ctx: &AgentContext                      [NEW]
│   ├── ReActAgent        (tool-use loop)
│   ├── PlanExecuteAgent  (plan → execute → synthesize)
│   ├── SupervisorAgent   (delegate to sub-agents)
│   └── RouterAgent       (classify → route)
│
└── ResourceAction        ctx: &ActionContext (acquire/release)

Port system:
  InputPort:   Flow | Support
  OutputPort:  Flow | Dynamic | Provide                            [NEW]

Tool provision:   #[provide(tool(...))] on any action type         [NEW]
Task system:      ctx.spawn() / ctx.background() → Task<T>        [NEW]
Cache policy:     #[cache(enabled|disabled|ttl)]                   [NEW]
Execution modes:  nebula-engine (workflow) + nebula-pipeline (streaming) [NEW]
```

---

## 11. Implementation Impact

### 11.1 Changes to nebula-action

| What | Effort | Phase |
|------|--------|-------|
| OutputPort::Provide + ProvidePort + ProvideKind | 2 days | Phase 18 (ports) |
| ConnectionFilter: required_provide + allowed_data_tags | 1 day | Phase 18 |
| #[provide(tool(...))] in derive macro | 2 days | Phase 16 (derives) |
| ToolSpec, ToolHints, ToolCall, ToolResult types | 1 day | Phase 12 |
| CachePolicy enum + #[cache] derive | 1 day | Phase 13 (policies) |
| Task\<T\> + spawn/background on contexts | 2 days | Phase 8 (context) |
| AgentAction trait + AgentOutcome + AgentCheckpoint | 1 day | Phase 2 (core) |
| AgentContext struct | 2 days | Phase 8 (context) |
| AgentHandler + AgentAdapter | 2 days | Phase 2-3 (handlers) |
| ActionInstance::Agent + ActionKind::Agent | 0.5 day | Phase 1 |
| ReActAgent DX type + blanket impl | 3 days | Phase 10-11 |
| PlanExecuteAgent, SupervisorAgent, RouterAgent | 3 days | Phase 10-11 |
| StreamProcessor trait + StreamIO + blanket impl | 3 days | Phase 10-11 |
| AgentTestHarness | 2 days | Phase 17 |
| **Total** | **~3.5 weeks** | |

### 11.2 New Crates

| Crate | Purpose | Priority |
|-------|---------|----------|
| nebula-ai | LlmClient trait, Message, ChatResponse, LlmUsage | P1 (needed for ReActAgent blanket impl) |
| nebula-pipeline | Real-time streaming engine | P3 (future, after nebula-engine stable) |

### 11.3 Updated Phase Plan

Original: ~13-14 weeks.
With additions: ~16-17 weeks (+3.5 weeks for agent, provide, task, cache, stream).

---

## 12. Open Questions for Review

1. **AgentAction as core type vs DX helper** — ChatGPT argues for helper-over-ActionContext.
   Our position: context separation principle (same as TriggerAction) justifies core type.
   Needs adversarial review.

2. **ReActAgent dependency on LlmClient** — should nebula-ai be optional or required
   dependency of nebula-action? Currently proposed: optional behind feature flag.

3. **StreamProcessor::Frame type** — current design uses StreamIO with typed accessors.
   Alternative: generic `type Frame` on trait. Tradeoff: type safety vs cross-media nodes.

4. **ToolResult::Direct** — "return directly to user, skip LLM" — how does engine
   handle this in ReActAgent blanket impl? Proposed: break loop, return Direct value
   as final output. Needs validation.

5. **AgentPark integration with existing WaitCondition** — should AgentPark reuse
   WaitCondition enum or have its own? Currently proposed: own enum (agent-specific
   semantics). May unify later.
