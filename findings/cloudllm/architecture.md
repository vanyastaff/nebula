# cloudllm — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/CloudLLM-ai/cloudllm
- **Stars:** 26 | **Forks:** 3
- **License:** MIT
- **Language:** Rust (Edition 2018)
- **Version:** 0.15.1 (latest tag: `0.14.0`)
- **Created:** 2023-10-01 | **Last push:** 2026-04-23
- **Maintainer:** Angel Leon (`gubatron@gmail.com`) — sole author based on git log
- **Governance:** Solo open source, MIT license, no stated commercial model
- **crates.io:** Published as `cloudllm`

---

## 1. Concept positioning [A1, A13, A20]

**Author's own README (first paragraph):**

> "CloudLLM is a batteries-included Rust toolkit for building intelligent agents with LLM integration, multi-protocol tool support, and multi-agent orchestration."
> — `README.md:5-8`

**Mine (after reading code):**

cloudllm is an LLM-client-and-agent SDK, not a workflow orchestration engine. Its core primitive is the `Agent` struct (an LLM-session wrapper with tool access and identity) plus `Orchestration` (multi-agent conversation coordination via 7 named modes). It has no DAG, no persistent workflow state, no credential subsystem, no resource lifecycle, and no trigger/event model.

**Comparison with Nebula:**

Nebula is a *workflow engine* that happens to support AI actions via generic mechanisms. cloudllm is an *LLM agent framework* that does not orchestrate arbitrary deterministic work. The overlap is narrow: Nebula's planned LLM plugin lives in the same space cloudllm occupies, but cloudllm has no persistence layer, no cross-run state, and no trigger model. They are complementary rather than competitive on most axes.

**Comparison with tianshu's `LlmProvider` trait:**

cloudllm's `ClientWrapper` trait (`src/cloudllm/client_wrapper.rs:186`) plays the same role as tianshu-style `LlmProvider` — a multi-provider abstraction. cloudllm goes further with `Agent` identity, `Orchestration` modes, and `ContextStrategy`, making it closer to an agent SDK than a bare LLM client. LangChain.rs occupies similar territory; async-openai is lower-level (no agents or orchestration).

---

## 2. Workspace structure [A1]

cloudllm uses a minimal 2-crate workspace (`Cargo.toml:1-3`):

1. **`cloudllm`** (root) — primary library; all agent, session, orchestration, and tool logic.
2. **`cloudllm_mcp`** (path `mcp/`) — MCP protocol layer; provides `ToolProtocol`, `ToolRegistry`, `ToolDefinition`, `McpClientProtocol`, and `MCPServer`.

There is no layering by domain concern (credentials, resources, resilience, tenancy all absent). The single `src/cloudllm/` module tree organizes everything. Feature flags: only `mcp-server` (gates axum + tower, enables `MCPServerBuilder`; `Cargo.toml:44-46`).

**vs. Nebula:** Nebula uses 26 crates with strict layering (error → resilience → credential → resource → action → engine). cloudllm's 2-crate approach trades isolation for simplicity — appropriate for a library (no multi-tenant server concerns) but would not scale to Nebula's deployment model.

---

## 3. Core abstractions [A3, A17] — DEEP

### A3.1 Trait shape

The central trait is `ClientWrapper` (`src/cloudllm/client_wrapper.rs:186`):

```rust
#[async_trait]
pub trait ClientWrapper: Send + Sync {
    async fn send_message(
        &self,
        messages: &[Message],
        tools: Option<Vec<ToolDefinition>>,
    ) -> Result<Message, Box<dyn Error>>;

    fn send_message_stream<'a>(
        &'a self,
        _messages: &'a [Message],
        _tools: Option<Vec<ToolDefinition>>,
    ) -> MessageStreamFuture<'a> { Box::pin(async { Ok(None) }) }

    fn model_name(&self) -> &str;
    async fn get_last_usage(&self) -> Option<TokenUsage>;
    fn usage_slot(&self) -> Option<&Mutex<Option<TokenUsage>>> { None }
}
```

- **Open trait** — any downstream crate can implement it. Not sealed.
- **`dyn`-compatible** — used as `Arc<dyn ClientWrapper>` throughout (`agent.rs:147`).
- **Associated types:** none — all types are concrete (`Message`, `TokenUsage`) or erased (`Box<dyn Error>`).
- **No GATs, no HRTBs, no typestate** — flat, simple, maximum compatibility.
- **Default methods:** `send_message_stream` (no-op), `get_last_usage` (delegates to `usage_slot`), `usage_slot` (returns None).
- **async-trait macro** (`async-trait = "0.1.89"`) — still using async-trait (pre-AFIT), which generates `Pin<Box<dyn Future>>` internally.

`ToolProtocol` (`mcp/src/protocol.rs:400`):

```rust
#[async_trait]
pub trait ToolProtocol: Send + Sync {
    async fn execute(&self, tool_name: &str, parameters: serde_json::Value)
        -> Result<ToolResult, Box<dyn Error + Send + Sync>>;
    async fn list_tools(&self) -> Result<Vec<ToolMetadata>, Box<dyn Error + Send + Sync>>;
    async fn get_tool_metadata(&self, name: &str) -> Result<ToolMetadata, Box<dyn Error + Send + Sync>>;
    fn protocol_name(&self) -> &str;
    async fn initialize(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> { Ok(()) }
    async fn shutdown(&mut self) -> Result<(), Box<dyn Error + Send + Sync>> { Ok(()) }
    async fn list_resources(&self) -> Result<Vec<ResourceMetadata>, Box<dyn Error + Send + Sync>> { Ok(vec![]) }
    async fn read_resource(&self, uri: &str) -> Result<String, Box<dyn Error + Send + Sync>> { ... }
    fn supports_resources(&self) -> bool { false }
}
```

`ContextStrategy` (`src/cloudllm/context_strategy.rs:51`): open async trait with `should_compact(&session) -> bool` and `compact(&mut session, chain, id)`.

`EventHandler` (`src/cloudllm/event.rs`): open async trait with `on_agent_event(&AgentEvent)` and `on_orchestration_event(&OrchestrationEvent)`, both with default no-op impls.

`Planner` (`src/cloudllm/planner.rs`): open async trait; `BasicPlanner` is the only bundled impl.

### A3.2 I/O shape

`Message` carries `role: Role`, `content: Arc<str>` (arena-backed), and `tool_calls: Vec<NativeToolCall>`. All messages use `Arc<str>` to enable cheap cloning across session history. Parameters and tool results are `serde_json::Value` — fully type-erased. No compile-time input/output type constraints exist.

`NativeToolCall` (`client_wrapper.rs:91`): `{ id: String, name: String, arguments: serde_json::Value }`.

Streaming output: `MessageChunkStream = Pin<Box<dyn Stream<Item = Result<MessageChunk, Box<dyn Error>>> + Send>>`. Streaming is provider-optional (default returns `Ok(None)`). OpenAI and Grok provide implementations; Claude and Gemini use the default no-op.

### A3.3 Versioning

No versioning concept. Actions/agents are identified by string `id` and `name` only. There is no `#[deprecated]` on `Agent` or `ClientWrapper`; deprecation appears only on concrete `Model` enum variants (e.g., `ClaudeOpus46` deprecated in favor of `ClaudeOpus47`, `clients/claude.rs:58`). No v1/v2 action registration, no migration path.

### A3.4 Lifecycle hooks

`Agent` has no explicit pre/post/cleanup hooks. The `Planner` trait (`planner.rs`) abstracts a single `plan(...)` turn with policy check, tool loop, and streaming output — no named lifecycle phases. `ToolProtocol` has optional `initialize()` and `shutdown()` with default no-ops. No idempotency key concept.

Cancellation: none. No `CancellationToken`, no interrupt signal. If the tokio task is dropped, the HTTP call is abandoned, but there is no cooperative cancellation in the API.

### A3.5 Resource and credential deps

Agents receive credentials as plain `String` API keys passed to client constructors (`clients/openai.rs:306`: `api_key: secret_key.to_string()`). There is no mechanism for an agent to declare "I need credential X" — keys are constructor-injected, stored as plain `String` on the client struct. No compile-time check of credential availability.

### A3.6 Retry/resilience attachment

No built-in retry logic, circuit breaker, or timeout. The HTTP clients use bare `reqwest` calls with no retry policy (`clients/common.rs:406`). If a provider returns a 429 or 500, the error propagates immediately as `Box<dyn Error>`. No `ErrorClassifier` concept.

### A3.7 Authoring DX

A minimal agent requires ~5 lines:

```rust
let client = Arc::new(OpenAIClient::new_with_model_enum(&key, Model::GPT41Mini));
let agent = Agent::new("id", "Name", client);
let response = agent.send("system", "hello", &[]).await?;
```

No derive macros, no builders (well, builder-style `with_*` methods exist on `Agent`). "Hello world" is concise. IDE support is standard — no proc macros, all types are concrete or `Box<dyn Trait>`.

### A3.8 Metadata

`Agent` has public fields: `id: String`, `name: String`, `expertise: Option<String>`, `personality: Option<String>`, `metadata: HashMap<String, String>` (`agent.rs:106-134`). No i18n, no icon/category, no compile-time metadata. All runtime strings.

### A3.9 vs Nebula

Nebula has 5 sealed action kinds (Process/Supply/Trigger/Event/Schedule) each with associated `Input`/`Output`/`Error` types enforced at compile time. cloudllm has no equivalent: instead of sealed polymorphism it has a single open `ClientWrapper` trait plus the `ToolProtocol` trait for extensions. cloudllm's abstraction is shallow (type-erased JSON in/out, no GATs) but far simpler to implement extensions for. The trade-off: cloudllm gains DX and open extensibility; Nebula gains compile-time correctness and type-safe port wiring.

---

## 4. DAG / execution graph [A2, A9, A10]

**No DAG.** cloudllm has no directed-acyclic-graph workflow model. Grep evidence:

- Search: `dag`, `DAG`, `petgraph`, `graph`, `Graph` in `src/` — results: only `agent.rs` has the word "graph" in a doc comment context (`mcp/src/protocol.rs` never), and `orchestration.rs` references a dependency graph only in comments (`orchestration.rs:35`). No petgraph import in `Cargo.toml`.
- Search: `workflow` in `src/` — appears only in comments/doc strings (e.g., `agent.rs:10`: "In custom workflows for specialized use cases"), never as a first-class type.

Orchestration is a flat linear conversation loop, not a graph. `Orchestration::run(prompt, rounds)` iterates agents sequentially or in parallel for a fixed number of rounds (`orchestration.rs:1327`).

**Concurrency:** `tokio::spawn` for Parallel mode (`orchestration.rs:433` comment). `Arc<RwLock<ToolRegistry>>` for runtime tool mutations (`agent.rs:122`). `Arena/bumpalo` for message bodies (`llm_session.rs:76`). No frontier scheduler, no work-stealing.

**vs. Nebula:** Nebula has TypeDAG with 4 levels of type safety. cloudllm has none of this — it is not a DAG engine.

---

## 5. Persistence and recovery [A8, A9]

**No workflow persistence layer.** There is no database, no checkpoint, no append-only log. Each `LLMSession` holds conversation history in memory as `Vec<Message>` (`llm_session.rs:64`). If the process restarts, history is lost.

**MentisDB** (`mentisdb = "0.4"` dependency) provides persistent agent memory: SHA-256 hash-chained `Thought` objects with a git-like registry, stored to disk. This is a semantic memory primitive (facts/decisions) not a workflow execution log. An agent can commit thoughts to MentisDB (`agent.rs:56`: `use mentisdb::{MentisDb, Thought, ThoughtType}`).

Grep evidence of absent persistence primitives:
- Search `sqlx`, `postgres`, `Postgres`, `PgPool`, `migration` in all `*.rs` and `*.toml` — 0 results.
- Search `checkpoint`, `replay`, `frontier`, `append_only` in `src/` — 0 results.

**vs. Nebula:** Nebula has sqlx + PgPool + frontier-based checkpointing + append-only execution log. cloudllm has none of these — it is a stateless-per-run library.

---

## 6. Credentials / secrets [A4] — DEEP

**A4.1 Existence:** No separate credential layer exists. API keys are passed as plain `&str` / `String` to client constructors and stored as `String` fields on client structs.

Grep evidence:
- Search `credential`, `Credential`, `CredentialOps`, `LiveCredential` in all `*.rs` — 0 source results (only appears in `mcp/src/builder_utils.rs:198` as a comment: "subtle::ConstantTimeEq prevents timing oracle on credentials").
- Search `secrecy`, `zeroize`, `Zeroize` in `Cargo.toml` and `*.rs` — 0 results.
- Search `Secret<T>`, `secrecy` crate import — 0 results.

**A4.2 Storage:** None. API keys live in memory as plain `String` for the lifetime of the client struct. No encrypted storage, no vault integration, no OS keychain.

**A4.3 In-memory protection:** None. Keys are plain `String`. `subtle = "2"` is imported in `Cargo.toml:34` and used only for constant-time comparison in MCP server authentication (`mcp/src/builder_utils.rs:198`) — not for API key protection.

**A4.4 Lifecycle:** No CRUD for credentials. No refresh, no expiry detection, no revocation. A key is provided at construction and stays for the process lifetime.

**A4.5 OAuth2/OIDC:** None. No OAuth2 flow, no PKCE, no refresh token handling.

**A4.6–A4.8:** N/A — no credential abstraction exists.

**A4.9 vs Nebula:** Nebula has State/Material split, LiveCredential with `watch()` for blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter for type erasure. cloudllm has none of these — credentials are opaque strings injected at startup.

This is an expected omission: cloudllm is a library for interactive agent sessions (typically short-lived) rather than a long-running server managing many tenants' credentials. However, it means cloudllm cannot safely be embedded in multi-tenant server deployments without external credential management.

---

## 7. Resource management [A5] — DEEP

**A5.1 Existence:** No explicit resource abstraction. DB pools, HTTP clients, and caches are not managed as first-class resources. Each `ClientWrapper` struct owns its own `reqwest::Client` internally (`clients/openai.rs:281`: `struct OpenAIClient { client: reqwest::Client, ... }`).

Grep evidence:
- Search `Resource`, `ResourceLifecycle`, `ReloadOutcome`, `generation` (resource context) in `src/` — 0 results (MCP `ResourceMetadata` exists but is purely about MCP protocol resources, not infrastructure resources).
- Search `pool`, `Pool`, `ConnectionPool` in `src/` — 0 results in source (only appears in test file `tests/connection_pooling_test.rs` which tests a custom implementation).

**A5.2–A5.8:** N/A — no resource scoping, no lifecycle hooks, no hot-reload, no backpressure. Each agent creates its own HTTP client; no sharing of infrastructure resources.

**A5.8 vs Nebula:** Nebula has 4 scope levels, `ReloadOutcome` enum, generation tracking, `on_credential_refresh`. cloudllm has none of these.

---

## 8. Resilience [A6, A18]

No resilience primitives. Grep evidence:
- Search `retry`, `Retry`, `circuit_breaker`, `CircuitBreaker`, `bulkhead`, `Bulkhead`, `timeout` in `src/` — 0 results (timeout appears in HTTP examples only).
- Search `ErrorClass`, `ErrorClassifier`, `transient`, `permanent` in `src/` — 0 results.

Error type: `Box<dyn Error>` / `Box<dyn Error + Send + Sync>` throughout. No custom error enum with classification. `planner.rs:78`: `type PlannerResult<T> = Result<T, Box<dyn Error + Send + Sync>>`. Function `map_session_error` (`planner.rs:1323`) converts between error types but adds no classification.

`OrchestrationError` (`orchestration.rs:936`) is a small typed enum covering orchestration-specific failures (agent-not-found, mode-mismatch), not a general error taxonomy.

**vs. Nebula:** Nebula has `nebula-resilience` crate with retry/CB/bulkhead/timeout/hedging and unified `ErrorClassifier`. cloudllm has none of this.

---

## 9. Expression / data routing [A7]

`evalexpr = "13.0.0"` is a dependency (`Cargo.toml:32`) but it is used internally in a limited capacity (not exposed as a user-facing expression DSL). Grep: `evalexpr` appears in `Cargo.toml` but no grep match in source `*.rs` files for `evalexpr::*` — it may be used in tools or session token estimation.

There is no `$nodes.foo.result.email` style expression language. Data flows through `serde_json::Value` at every tool call boundary; routing between agents in orchestration is positional (message order) not expression-driven.

**vs. Nebula:** Nebula has a 60+ function expression engine with type inference and sandboxed eval. cloudllm has no equivalent.

---

## 10. Plugin / extension system [A11] — DEEP (BUILD + EXEC)

### 10.A — Plugin BUILD process

**A11.1 Format:** No plugin binary format. cloudllm uses no WASM, no dynamic libraries, no separate plugin packages. Extensions are Rust code that implements `ToolProtocol` or `ClientWrapper` and are compiled into the same binary.

Grep evidence:
- Search `wasm`, `WASM`, `wasmtime`, `wasmer`, `wasmi` in all files — 0 results.
- Search `dlopen`, `libloading`, `dynamic` (lib context) in `*.rs` — 0 results.
- Search `plugin`, `Plugin` in `src/` — 0 results.

**A11.2 Toolchain:** No separate compile step. Tools and protocol implementations are normal Rust structs in the same binary.

**A11.3 Manifest:** None.

**A11.4 Registry/discovery:** `ToolRegistry::discover_tools_from_primary()` (`mcp/src/protocol.rs:803`) performs runtime discovery by calling `list_tools()` on registered `ToolProtocol` implementations. Remote MCP servers are discovered via HTTP. No signing, no versioning, no OCI registry.

### 10.B — Plugin EXECUTION sandbox

**A11.5 Sandbox type:** None. Tools execute in the same process and same thread. `CustomToolProtocol` takes closures (`Fn(serde_json::Value) -> Result<ToolResult, ...> + Send + Sync`); there is no isolation boundary. Remote MCP servers are called over HTTP (that is the only "sandbox" — a separate OS process).

**A11.6 Trust boundary:** No capability system. Local tools have full process access. HTTP MCP servers are trusted by default once connected.

**A11.7 Host-plugin calls:** Via `ToolProtocol::execute(name, serde_json::Value)` — JSON marshaling only.

**A11.8 Lifecycle:** `initialize()` and `shutdown()` default no-ops on `ToolProtocol`. Hot reload: `Agent::add_protocol()` and `remove_protocol()` at runtime via `Arc<RwLock<ToolRegistry>>` (`agent.rs:122`).

**A11.9 vs Nebula:** Nebula targets WASM + capability security + Plugin Fund commercial model. cloudllm has no plugin system — only in-process Rust trait implementations and remote HTTP MCP. No commercial monetization model.

---

## 11. Trigger / event model [A12] — DEEP

**No trigger/event model.** cloudllm has no webhook handling, no cron scheduler, no queue consumer, no source normalization.

Grep evidence:
- Search `webhook`, `Webhook` in `src/` — 0 results.
- Search `cron`, `schedule`, `Schedule` (as a concept, not `tokio::time`) in `src/` — 0 results.
- Search `kafka`, `Kafka`, `rabbitmq`, `nats`, `NATS`, `pubsub` in all files — 0 results.
- Search `TriggerAction`, `Source`, `EventAction` in `src/` — 0 results.

cloudllm agents are invoked programmatically (`agent.send(...)`, `orchestration.run(...)`). No inbound event routing exists.

**A12.7 vs Nebula:** Nebula's `TriggerAction` with `Input = Config` / `Output = Event` plus the `Source` trait for a 2-stage normalization pipeline has no equivalent in cloudllm. cloudllm is a library called synchronously, not a server that listens for inbound events.

---

## 12. Multi-tenancy [A14]

No multi-tenancy. No RBAC, no SSO, no schema/RLS isolation. MCP server authentication uses a static bearer token with `subtle::ConstantTimeEq` for constant-time comparison (`mcp/src/builder_utils.rs:198`). This is single-tenant at the process level.

**vs. Nebula:** Nebula has `nebula-tenant` crate with three isolation modes. N/A for cloudllm.

---

## 13. Observability [A15]

No OpenTelemetry integration. The observability model is the `EventHandler` callback system (`event.rs`): an `Arc<dyn EventHandler>` receives `AgentEvent` and `OrchestrationEvent` variants synchronously during execution. Events cover:
- `LLMCallStarted` / `LLMCallCompleted` (with `TokenUsage`)
- `ToolCallDetected` / `ToolExecutionCompleted` / `ToolCallFailed`
- `SendStarted` / `SendCompleted`
- Orchestration: `RunStarted` / `RoundCompleted` / `RunCompleted`
- MCP: `ServerHttpRequest` / `ToolCallRouted` / `AuthRejection`

This is a synchronous in-process pub/sub — no trace IDs, no span propagation, no metrics export.

**vs. Nebula:** Nebula exports structured OpenTelemetry spans per execution. cloudllm's event system is a simpler callback-based approach better suited to CLI/interactive tools.

---

## 14. API surface [A16]

The public API is purely Rust library (no HTTP endpoint of its own). Usage is `cloudllm = "0.15.1"` as a crate dependency. No OpenAPI spec, no REST API, no gRPC. The `MCPServerBuilder` allows deploying a tool server that exposes MCP over HTTP, but that is exposing tools, not a management API.

**vs. Nebula:** Nebula has a REST API surface with planned GraphQL/gRPC. cloudllm is library-only.

---

## 15. Testing infrastructure [A19]

14 integration test files in `tests/`. Tests are end-to-end by design — they call real (or mocked) LLM providers. No `nebula-testing`-style public testing utilities. No contract tests for `ClientWrapper` or `ToolProtocol` implementors. No `insta` snapshot testing, no `wiremock`. Some tests use `tempfile = "3.23"` for filesystem tool tests.

**vs. Nebula:** Nebula has a dedicated `nebula-testing` crate with contract tests. cloudllm relies entirely on integration tests.

---

## 16. AI / LLM integration [A21] — DEEP

This is cloudllm's primary purpose. All 13 sub-questions answered:

### A21.1 Existence

AI/LLM integration is the **central, defining feature** of cloudllm, not an add-on. Every public API is oriented around LLM calls. There is no non-AI usage path.

### A21.2 Provider abstraction

Multi-provider. `ClientWrapper` trait with 4 concrete implementations:

| Provider | File | Mechanism |
|----------|------|-----------|
| OpenAI | `clients/openai.rs` | `openai-rust2` crate (author's fork) |
| Anthropic Claude | `clients/claude.rs` | Delegates to `OpenAIClient` via OpenAI-compat API at `https://api.anthropic.com/v1` |
| Google Gemini | `clients/gemini.rs` | Native HTTP via `reqwest`, Gemini-specific JSON |
| xAI Grok | `clients/grok.rs` | Native HTTP via `reqwest`, OpenAI-compatible endpoint |

Custom BYOL endpoint: `OpenAIClient::new_with_custom_endpoint(api_key, base_url, model)` allows pointing to any OpenAI-compatible endpoint (`clients/openai.rs:324`). This enables local model access via Ollama, LM Studio, or similar.

Local model support: No direct llama.cpp/candle/mistral.rs integration — but the custom endpoint mechanism covers Ollama endpoints.

Model enumerations: All providers expose typed `Model` enums with deprecation attributes for discontinued models (e.g., `claude.rs:58-79` deprecates `ClaudeOpus46` in favor of `ClaudeOpus47`).

### A21.3 Prompt management

System prompt set via `LLMSession::new(client, system_prompt, max_tokens)` or `Agent::set_system_prompt(...)`. No standalone prompt template engine. No few-shot example management, no prompt versioning, no prompts checked into workflow definitions.

Agent identity attributes (`expertise`, `personality`) are embedded into the system prompt string at send time — simple string interpolation, not a structured template system.

### A21.4 Structured output

No built-in JSON schema enforcement. Tool calling (see A21.5) returns `serde_json::Value`; callers are responsible for their own validation. No re-prompting on validation failure. No Pydantic-style schema enforcement.

`evalexpr` is present in dependencies but not connected to output validation.

### A21.5 Tool calling

Two parallel mechanisms:

1. **Native function calling** (`NativeToolCall`, `client_wrapper.rs:91`): `send_message` with `tools: Some(Vec<ToolDefinition>)` forwards tool definitions to the provider's function-calling API. The LLM returns `NativeToolCall` instances in the response message. The agent's send loop (`agent.rs`) detects these, executes them via `ToolRegistry`, and re-submits results as `Role::Tool { call_id }` messages. Multi-turn feedback loop: yes. Parallel tool execution: not documented as parallel — sequential by default.

2. **Text-parsed fallback**: For providers lacking native tool calling, the agent can parse `[TOOL_CALL: name(params)]` markers from assistant text (older mechanism, still present). Results injected as `Role::User` messages rather than `Role::Tool`.

Tool definition format: `ToolDefinition { name, description, parameters_schema: serde_json::Value }` (JSON Schema object), `mcp/src/protocol.rs:90-97`.

Execution sandbox: same process (no isolation).

### A21.6 Streaming

SSE/chunked via `MessageChunkStream = Pin<Box<dyn Stream<Item = Result<MessageChunk, Box<dyn Error>>> + Send>>` (`client_wrapper.rs:176`). OpenAI and Grok provide streaming implementations. Claude uses the default no-op returning `Ok(None)`.

`LLMSession::send_message_stream(...)` wraps provider streaming. `Planner` also supports streaming output via a `Streamer` trait (noop by default).

Backpressure: standard tokio stream backpressure — the consumer polls the stream. No explicit backpressure policy.

**Streaming into workflow nodes:** N/A — no workflow node concept exists.

### A21.7 Multi-agent orchestration

This is cloudllm's differentiating feature over simple LLM clients. 7 collaboration modes (`orchestration.rs:430-end`):

| Mode | Pattern | Termination |
|------|---------|-------------|
| `Parallel` | All agents respond simultaneously via `tokio::spawn` | Fixed rounds |
| `RoundRobin` | Sequential turn-taking | Fixed rounds |
| `Moderated` | Moderator selects next speaker | Fixed rounds |
| `Hierarchical` | Layer-by-layer (lead → specialists) | All layers done |
| `Debate { max_rounds, convergence_threshold }` | Agents challenge each other; convergence detected | Convergence or max rounds |
| `Ralph { tasks, max_iterations }` | PRD checklist; agents signal `[TASK_COMPLETE:id]` | All tasks done or max iterations |
| `AnthropicAgentTeams { pool_id, tasks, max_iterations }` | Decentralized task claiming via shared Memory | All tasks done or max iterations |

Hand-off: via message injection — `Agent::receive_message(...)` inserts prior agent outputs into the target agent's session history. Per-agent cursors track which messages each agent has already seen (`orchestration.rs:55-56`).

Shared memory: `Memory` tool (in-process `Arc<Mutex<HashMap>>` with TTL) shared across agents in an `Orchestration`. `AnthropicAgentTeams` mode uses this for task coordination — agents claim tasks by writing to shared memory.

Termination: fixed rounds (most modes) or task-completion detection.

**vs. Nebula/Surge:** Nebula has no first-class LLM orchestration; Surge handles agent orchestration on ACP. cloudllm's `Orchestration` is the most directly comparable component to Surge, but cloudllm's model is simpler (no ACP, no cross-service handoff).

### A21.8 RAG / vector store integration

No embeddings, no vector store integration. Grep evidence:
- Search `embed`, `embedding`, `vector`, `qdrant`, `pgvector`, `weaviate`, `faiss`, `RAG` in all `*.rs` — 0 results.
- Search `retrieval` in all files — 0 results.

**MentisDB** (`mentisdb = "0.4"`) provides semantic graph-based memory with hash-chained persistence, but it is not a vector database — it stores `Thought` objects with typed `ThoughtType` values and resolves context via a graph traversal, not embedding similarity search.

### A21.9 Memory / context management

Two-layer memory model:

1. **`LLMSession` rolling history** (`llm_session.rs:64`): In-memory `Vec<Message>` with oldest-first trimming when `estimated_history_tokens` approaches `max_tokens`. Token estimation is heuristic (not actual tokenizer), cached per-message for efficiency (`llm_session.rs:67`). Context window managed by `ContextStrategy`.

2. **MentisDB long-term memory** (`agent.rs:131`, optional): SHA-256 hash-chained `Thought` objects persisted to disk. Agents can `commit()` thoughts. Context strategies can serialize session state to MentisDB and reload it (`SelfCompressionStrategy`).

Three `ContextStrategy` implementations (`context_strategy.rs`):
- `TrimStrategy` (default): oldest-first trim, no-op `compact()`.
- `SelfCompressionStrategy`: asks the LLM itself to write a structured save-file, persists to MentisDB, then reloads as bootstrap prompt.
- `NoveltyAwareStrategy`: wraps another strategy; uses entropy heuristic (unique n-gram ratio) to skip compression when content is still novel.

Conversation memory is per-session (per `Agent` instance). No cross-execution sharing without MentisDB.

### A21.10 Cost / tokens

Per-call token accounting via `TokenUsage { input_tokens, output_tokens, total_tokens }` (`client_wrapper.rs:136`). Providers that expose usage data populate `usage_slot()` on their client struct.

`LLMSession::token_usage()` aggregates across the session. `AgentEvent::SendCompleted` carries cumulative `tokens_used`.

**No cost calculator** (dollar amounts). No per-provider cost rates. No budget circuit breakers. No per-tenant attribution. The `ORCHESTRATION_TUTORIAL.md` documents cost estimates manually (e.g., "Debate: $0.60-$2.00 for 4 agents") but there is no runtime enforcement.

### A21.11 Observability

`EventHandler` callbacks fire for every LLM round-trip (`LLMCallStarted`/`Completed`), every tool call, and every orchestration lifecycle event. `TokenUsage` is surfaced in `LLMCallCompleted` events.

**No structured tracing, no OpenTelemetry.** Prompt and response content are not logged by default. There are no PII-safe logging controls. No LLM-as-judge eval hooks.

### A21.12 Safety

No built-in content filtering, no prompt injection mitigations, no output validation pipeline. The Gemini `Model` enum lists `TextBisonSafetyOff` and `TextBisonSafetyRecitationOff` variants (`clients/gemini.rs:245-248`), indicating safety controls are delegated to the provider level entirely.

No input sanitization before prompt construction. System prompts and user messages are concatenated as-is.

### A21.13 vs Nebula + Surge

| Dimension | cloudllm | Nebula | Surge |
|-----------|---------|--------|-------|
| LLM abstraction | `ClientWrapper` trait, 4 providers | None (planned plugin) | Via ACP |
| Multi-agent | 7 `OrchestrationMode` variants, full in-library | None | Core feature |
| Persistence | MentisDB (semantic memory) + in-memory history | Frontier + PgPool + append-only log | Not known |
| Tools | `ToolProtocol` + MCP + built-ins | Generic actions (type-safe) | ACP tools |
| Workflow DAG | Not present | TypeDAG L1-L4 | Not known |
| Credentials | Plain string API keys | State/Material/LiveCredential | Not known |

**cloudllm is first-class AI, working today.** Nebula's bet is that AI = generic actions + plugin; cloudllm proves the tradeoff: you get excellent LLM DX, multi-agent orchestration, and streaming, at the cost of no workflow engine, no persistence, no credentials, and no DAG. cloudllm is not over-coupled — it avoids coupling by having no non-AI features at all.

---

## 17. Notable design decisions

**D1: `ClientWrapper` as open trait with `Box<dyn Error>`.**
Simple, pragmatic, maximally compatible. The choice of `Box<dyn Error>` instead of `thiserror`/`anyhow` means callsites cannot pattern-match on error kinds for retry logic. This was a deliberate simplicity choice; the trade-off is that retry strategies must be layered externally.

**D2: Arena allocation for message bodies (bumpalo).**
Issue #26 ("Arena/bump allocation for message bodies") was explicitly filed and closed. `LLMSession` uses a `bumpalo::Bump` arena (`llm_session.rs:77`) to avoid per-message heap allocations. This is an unusual optimization for an LLM client library — shows the author is performance-conscious.

**D3: Claude via OpenAI-compat layer.**
`ClaudeClient` delegates entirely to `OpenAIClient` pointed at Anthropic's OpenAI-compatible endpoint (`clients/claude.rs:43-48`). This means Claude and OpenAI share code, but it also means Claude-specific features (extended thinking, vision with PDFs) unavailable in the compat layer are not accessible through cloudllm.

**D4: `AnthropicAgentTeams` decentralized coordination via shared Memory.**
Rather than a central orchestrator assigning tasks, agents discover, claim, and complete tasks from a shared `Memory` (in-process HashMap). This mirrors Anthropic's own agent team architecture. The result: no single point of failure, agents can be added/removed dynamically, but debugging is harder (no single trace of task assignment).

**D5: Two-tier memory (session history + MentisDB).**
Separating rolling context (cheap, fast, in-memory trim) from long-term semantic memory (hash-chained, persistent, graph-resolved) is a sound decomposition. The `SelfCompressionStrategy` that uses the LLM itself to write its own save-file is novel — the agent generates structured state to persist, not just raw message history.

**D6: Runtime hot-swapping of tool protocols.**
`Agent::add_protocol()` / `remove_protocol()` via `Arc<RwLock<ToolRegistry>>` allows tools to be registered/deregistered while the agent is running. This is useful for agentic systems that discover tools dynamically (e.g., connecting to a new MCP server mid-session).

**D7: No async-fn-in-trait (still uses async-trait macro).**
All traits use `#[async_trait]` macro (`Cargo.toml:23`: `async-trait = "0.1.89"`). Rust 1.75+ supports async fns in traits without the macro. This is a minor idiom lag (edition 2018 + async-trait is a known friction pair in 1.75+ codebases).

---

## 18. Known limitations / pain points

Based on GitHub issues (only 26 total; most are implementation tasks, not user pain points):

- **Issue #54** (OPEN): "Github Copilot provider" — request for a Copilot client. Indicates the multi-provider model is valued; Copilot's non-standard auth is likely the blocker.
- **Issue #3** (CLOSED, 2025-10-26): "Groq Integration" — Groq (not Grok) is not yet supported. Fast inference providers are a gap.
- **Issues #9, #10, #12, #14–16, #22, #24, #26, #28, #30** (all CLOSED, batch in 2025-10): A focused performance audit round — arena allocation, dedup clone, async-friendly locks, token cache, buffer reuse, pre-transmission trim. This shows the author is engineering-quality conscious but that the initial implementation had non-trivial allocation overhead in hot paths.

**Structural limitations (from code analysis, not issues):**
- No error classification → no retry logic → network transience is the caller's problem.
- No credential lifecycle → API keys are stale if rotated mid-session.
- No streaming for Claude/Gemini (default no-op implementation).
- Token estimation is heuristic (character-based approximation), not a real tokenizer — context trimming may be inaccurate for non-English or special-token-heavy prompts.
- `OrchestrationMode::Debate` cost warning in tutorial: "VERY HIGH — exponential with rounds" (ORCHESTRATION_TUTORIAL.md:17). No built-in guard against runaway cost.

---

## 19. Bus factor / sustainability

- **Maintainer:** 1 (Angel Leon, gubatron@gmail.com). All 20 visible commits from a single author.
- **Stars:** 26. **Forks:** 3.
- **Commit cadence:** Active — last push 2026-04-23; multiple version bumps in recent history (0.14.0, 0.15.0, 0.15.1 all in recent commits).
- **Issues:** 26 total; 25 closed, 1 open. Very low issue volume — either low usage or the author self-assigns and closes quickly.
- **Release age:** 0.15.1 is effectively current (bumped recently per git log).
- **Bus factor: 1.** If the sole maintainer stops contributing, the project stalls. No governance document, no contributing guide, no CI configured (no `.github/workflows/` found in the directory tree).

---

## 20. Final scorecard vs Nebula

| Axis | cloudllm approach | Nebula approach | Verdict | Borrow? |
|------|------------------|-----------------|---------|---------|
| A1 Workspace | 2-crate workspace (root + mcp sub-crate), no layering, Edition 2018 | 26 crates, layered: nebula-error / nebula-resilience / ... Edition 2024 | Nebula deeper; cloudllm simpler for a library | no — different goals |
| A2 DAG | No DAG. Flat conversation loops. No petgraph. | TypeDAG L1-L4 | Nebula deeper; cloudllm N/A (not an engine) | no — different goals |
| A3 Action | Open `ClientWrapper` trait (`dyn`-compatible, no associated types, `Box<dyn Error>`, async-trait); `ToolProtocol` for extensions | 5 sealed action kinds, sealed traits, associated Input/Output/Error, GATs, derive macros | Different decomposition — cloudllm maximizes DX and openness; Nebula maximizes compile-time correctness | refine — cloudllm's `ContextStrategy` pattern (pluggable strategy object) is borrowable for Nebula plugin context hooks |
| A11 Plugin BUILD | No build process — in-process Rust trait implementations only; remote MCP over HTTP | WASM sandbox planned (wasmtime), plugin-v2 spec | Nebula deeper (planned); cloudllm pragmatic | no — Nebula's approach is correct for sandboxing |
| A11 Plugin EXEC | No sandbox. Same-process closures or remote HTTP MCP. `ToolProtocol::execute(name, Value)`. | WASM + capability security | Nebula deeper; cloudllm N/A | no |
| A18 Errors | `Box<dyn Error>` / `Box<dyn Error + Send + Sync>` throughout; small `OrchestrationError` enum; no classification | `nebula-error` crate + `ErrorClass` enum + `ErrorClassifier` | Nebula deeper; cloudllm's approach prevents retry and classification | no — Nebula's approach is correct |
| A21 AI/LLM | **Central feature.** `ClientWrapper` trait (4 providers: OpenAI/Claude/Gemini/Grok + BYOL endpoint). `Agent` + `LLMSession` + `Orchestration` (7 modes). MentisDB memory. 3 context strategies. Native + text-parsed tool calling. Streaming (OpenAI/Grok). Token tracking. EventHandler callbacks. | No first-class LLM (planned: generic actions + plugin LLM client) | Competitor deeper on AI axis; Nebula not yet in this space | yes — borrow: `ClientWrapper` multi-provider abstraction pattern; `ContextStrategy` pluggable strategy; `AnthropicAgentTeams` decentralized task coordination pattern |

---

## Appendix — Negative grep evidence (LLM-client-only confirmation)

The following searches confirm cloudllm is an LLM client/agent SDK, not a workflow engine:

| Concept | Search terms | Result |
|---------|-------------|--------|
| DAG/graph engine | `petgraph`, `dag`, `DAG`, `TypeDAG` in `*.rs` | 0 hits in source |
| Credential system | `credential`, `Credential`, `secrecy`, `Zeroize`, `LiveCredential` in `src/**` | 0 hits in source |
| Resource lifecycle | `ReloadOutcome`, `ResourceLifecycle`, `ConnectionPool` in `src/**` | 0 hits |
| Resilience | `retry`, `circuit_breaker`, `bulkhead`, `ErrorClassifier` in `src/**` | 0 hits |
| Persistence/storage | `sqlx`, `postgres`, `checkpoint`, `frontier`, `append_only` in all files | 0 hits |
| Trigger/webhook | `webhook`, `cron`, `kafka`, `nats`, `TriggerAction` in `src/**` | 0 hits |
| Multi-tenancy | `tenant`, `Tenant`, `rbac`, `RBAC`, `SSO` in `src/**` | 0 hits |
| Plugin system | `wasm`, `WASM`, `wasmtime`, `dlopen`, `libloading` in all files | 0 hits |
