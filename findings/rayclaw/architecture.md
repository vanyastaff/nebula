# rayclaw — Architectural Decomposition

## 0. Project metadata

- **Repo**: https://github.com/rayclaw/rayclaw
- **Version**: 0.2.5 (latest tag at analysis time: `v0.2.5`)
- **License**: MIT
- **Language**: Rust 2021 edition (no pinned toolchain file found)
- **crates.io**: published as `rayclaw`
- **GitHub issues**: only 1 closed issue found (`#3`, "About Claude Code", 2026-03-23). Very low issue traffic for a new project.
- **Governance**: solo maintainer (rayclaw GitHub org), active commit history (v0.2.x cadence, Rust 1.95 Clippy fix already in)
- **Related**: rayclaw-desktop (Tauri 2 + React) at https://github.com/rayclaw/rayclaw-desktop

---

## 1. Concept positioning [A1, A13, A20]

**Author's own description** (README.md line 21): "RayClaw is a multi-channel agentic AI runtime written in Rust. It connects to Telegram, Discord, Slack, Feishu/Lark, and a built-in Web UI through a unified agent engine. Every conversation flows through the same tool-calling loop — shell commands, file operations, web search, background scheduling, and persistent memory — regardless of which channel it arrives on."

**My description (after reading code)**: RayClaw is a single-binary AI assistant runtime that routes chat messages from multiple IM platforms through a shared agentic loop backed by an LLM provider abstraction; tools are the primitive unit of behavior (not a DAG of nodes), and the "workflow" is the unstructured tool-calling conversation.

**Comparison with Nebula**: Nebula is a workflow orchestration engine — its unit is a typed, versioned Action in a static DAG evaluated by a frontier scheduler. RayClaw's unit is a `Box<dyn Tool>` in an LLM-driven conversation loop with no predefined graph. These are fundamentally different categories: Nebula = deterministic workflow engine with optional AI actions; RayClaw = AI-first agent runtime with optional structured tasks (cron/todos). They do not compete on the same axis except that both support scheduled tasks.

---

## 2. Workspace structure [A1]

Single-crate workspace (`Cargo.toml` root, no `[workspace]` members beyond the root). Feature flags gate channel adapters:

| Feature | Default | Description |
|---------|---------|-------------|
| `telegram` | Yes | teloxide 0.17 |
| `discord` | Yes | serenity 0.12 |
| `slack` | Yes | pure async WebSocket |
| `feishu` | Yes | Feishu/Lark WebSocket + webhook |
| `weixin` | Yes | iLink Bot protocol |
| `web` | **No** | axum 0.7 + embedded React UI (`include_dir!`) |
| `sqlite-vec` | No | sqlite-vec for semantic memory |

Source tree (`src/`): ~37 Rust files, approximately 39,000 lines of source. No sub-crates. Contrast with Nebula's 26-crate layered workspace.

**Comparison with Nebula**: Nebula uses strong physical boundaries (26 crates) for dependency enforcement. RayClaw is a single monolith with feature-flag boundaries. Simpler build, weaker layering guarantees.

---

## 3. Core abstractions [A3, A17]

### A3.1 Trait shape

`Tool` trait, defined at `src/tools/mod.rs:221`:

```rust
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, input: serde_json::Value) -> ToolResult;
}
```

- **Open** trait (not sealed) — any downstream crate can implement it; `ToolRegistry::add_tool()` is `pub` (`src/tools/mod.rs:509`).
- **Trait-object compatible** — stored as `Vec<Box<dyn Tool>>` (`src/tools/mod.rs:228`).
- **No associated types** — input and output are both `serde_json::Value` / `ToolResult` structs.
- **No GATs, no HRTBs, no typestate** — the trait is intentionally simple.
- **Default method**: `send_message_stream` on `LlmProvider` has a default fallback implementation (`src/llm.rs:212-228`), but `Tool` itself has no defaults.

### A3.2 I/O shape

- **Input**: `serde_json::Value` — fully dynamic JSON, no compile-time schema enforcement.
- **Output**: `ToolResult { content: String, is_error: bool, status_code: Option<i32>, bytes: usize, duration_ms: Option<u128>, error_type: Option<String> }` (`src/tools/mod.rs:37`).
- **No streaming output from tools** — streaming exists at the LLM layer (`send_message_stream` via SSE, `src/llm.rs:212`), not at the tool level.

### A3.3 Versioning

No versioning at all — tools are identified by name string only (`src/tools/mod.rs:521`). No `#[deprecated]`, no v1/v2 distinction, no migration support.

### A3.4 Lifecycle hooks

Single `execute()` method. No pre/post/cleanup/on-failure hooks. Cancellation is implicit (the agent loop has a `max_tool_iterations` guard, `src/agent_engine.rs:904`). No idempotency key.

### A3.5 Resource & credential deps

No formal resource declaration. Tools receive `Arc<Database>` and other dependencies through constructors at `ToolRegistry::new()` time (`src/tools/mod.rs:288-385`). No compile-time check — ad-hoc constructor injection.

### A3.6 Retry/resilience attachment

LLM-level retry lives in `src/error_classifier.rs` (exponential backoff for 429/5xx). Tool execution has no per-tool retry policy. The agent loop can retry a tool call sequence if context overflow occurs (`src/agent_engine.rs:937-953`).

### A3.7 Authoring DX

Manual `impl Tool for MyTool`. No derive macro. A "hello world" tool requires about 30-40 lines: struct definition, `new()`, `name()`, `definition()` (JSON Schema literal), `execute()`. See `src/tools/web_fetch.rs` (~128 lines) for a minimal real example.

### A3.8 Metadata

Tool `ToolDefinition` has `name: String`, `description: String`, `input_schema: serde_json::Value` (`src/llm_types.rs:4`). No icon, no category, no i18n. Metadata is entirely runtime JSON.

### A3.9 vs Nebula

Nebula has 5 sealed action kinds (Process/Supply/Trigger/Event/Schedule) with associated `Input`/`Output`/`Error` types, derive macros, and a versioning scheme. RayClaw has one open `Tool` trait with fully dynamic JSON I/O. Nebula's approach is richer for type-safe workflow composition; RayClaw's is simpler for AI-driven tool dispatch where the LLM handles schema validation.

**AgentEngine trait** (`src/agent_engine.rs:118`):
```rust
#[async_trait]
pub trait AgentEngine: Send + Sync {
    async fn process(&self, state: &AppState, context: AgentRequestContext<'_>,
        override_prompt: Option<&str>, image_data: Option<(String, String)>)
        -> anyhow::Result<String>;
    async fn process_with_events(&self, ..., event_tx: Option<&UnboundedSender<AgentEvent>>)
        -> anyhow::Result<String>;
}
```
This is the top-level "orchestrator" abstraction — one implementation (`DefaultAgentEngine`). No sub-class hierarchy.

---

## 4. DAG / execution graph [A2, A9, A10]

**No DAG model.** RayClaw has no static workflow graph. The "graph" is the sequence of LLM tool calls in a conversation session — dynamic and LLM-directed.

Negative grep evidence:
- Searched `petgraph`, `dag`, `DAG`, `workflow_graph`, `node_graph` in `src/` — **no matches**.

The scheduler (`src/scheduler.rs`) runs a 60-second poll loop for cron-based tasks, which is linear task dispatch, not a graph.

**Concurrency**: tokio runtime, single-threaded tool execution per chat (per-chat mutex `ChatLocks` in `src/agent_engine.rs`). Sub-agents run in parallel tokio tasks but the `sub_agent` tool limits them to `MAX_SUB_AGENT_ITERATIONS = 10` (`src/tools/sub_agent.rs:15`).

**Comparison with Nebula**: Nebula TypeDAG L1-L4 with petgraph soundness checks — deeply different. RayClaw uses no graph structure.

---

## 5. Persistence & recovery [A8, A9]

- **Storage**: SQLite via `rusqlite` with WAL mode. Single `Mutex<Connection>` shared as `Arc<Database>` (`src/db.rs:11`). No async SQLite driver.
- **Schema**: versioned via `db_meta` + `schema_migrations` tables (`src/db.rs:421`). Tables: `chats`, `messages`, `scheduled_tasks`, `sessions`, `memories`, `llm_usage_logs`, `memory_injection_logs`, `skill_health`, `tool_call_sequences`.
- **Session persistence**: full `Vec<Message>` (including tool_use/tool_result blocks) serialized to JSON in `sessions` table. Resume = load session + append.
- **Recovery**: no checkpoint/replay. On restart, in-flight sessions are lost but persisted sessions resume on next user message. No frontier-based scheduler.

**Comparison with Nebula**: Nebula uses PostgreSQL + PgPool + RLS + append-only execution log with frontier-based checkpoint recovery. RayClaw uses SQLite + full message JSON blob — simpler, single-user oriented.

---

## 6. Credentials / secrets [A4]

**A4.1 Existence**: No dedicated credential layer. API keys and tokens live directly in `rayclaw.config.yaml` as plain config fields (`src/config.rs:136-229`): `api_key`, `telegram_bot_token`, `discord_bot_token`, `web_auth_token`, `embedding_api_key`, `aws_access_key_id`, `aws_secret_access_key`.

**A4.2 Storage**: Config file on disk, read at startup. No at-rest encryption, no vault, no OS keychain.

**A4.3 In-memory protection**: No `zeroize`, no `secrecy::Secret<T>`. Secrets are plain `String` fields in the `Config` struct.

Negative grep evidence: searched `zeroize`, `secrecy`, `encrypt`, `vault`, `keychain` in `src/` — found only Feishu `app_secret` (a field name, not the library) and AES cipher (`aes`/`cipher` crates used for WeChat message body encryption per iLink protocol, not credential storage).

**A4.4 Lifecycle**: CRUD not applicable. No refresh model, no revocation. AWS credentials optionally refreshed via SigV4 signing at call time (`src/llm_bedrock.rs:8`).

**A4.5 OAuth2/OIDC**: Not present. OAuth2 crate not in `Cargo.toml`.

**A4.6-A4.9**: Not applicable. No credential trait, no State/Material split, no LiveCredential, no blue-green refresh.

**Comparison with Nebula**: Nebula's credential subsystem (State/Material split, CredentialOps, LiveCredential, blue-green refresh, OAuth2Protocol) is vastly deeper. RayClaw stores keys in a plaintext YAML file. This is a conscious design choice for a personal/self-hosted assistant, not an oversight for a multi-tenant platform.

---

## 7. Resource management [A5]

**A5.1 Existence**: No separate resource abstraction. DB pool, HTTP client, LLM provider, and channel adapters are wired at `AppState` construction in `src/runtime.rs`. No `Resource` trait.

Negative grep evidence: searched `ReloadOutcome`, `resource_lifecycle`, `resource_scope`, `on_credential_refresh` in `src/` — no matches.

**A5.2-A5.8**: Not applicable. No scope levels, no hot-reload, no generation tracking, no backpressure on resource acquisition.

**`AppState`** holds: `Arc<Database>`, `Arc<dyn LlmProvider>`, `Arc<ToolRegistry>`, `Arc<ChannelRegistry>`, `Arc<dyn EmbeddingProvider>`, `Config`. All shared via `Arc`, no pooling abstraction.

**Comparison with Nebula**: Nebula has 4 scope levels (Global/Workflow/Execution/Action), `ReloadOutcome` enum, and `on_credential_refresh` hooks. RayClaw has no resource lifecycle concept — resources are singletons wired at startup.

---

## 8. Resilience [A6, A18]

LLM-specific error classification in `src/error_classifier.rs`:
- `LlmErrorCategory` enum: `Transient`, `RateLimit`, `ContextOverflow`, `Auth`, `Permanent` (`src/error_classifier.rs:8`).
- `classify_http(status, body)` and `classify_anthropic(status, error_type, message)` (`src/error_classifier.rs:118-213`).
- Exponential backoff: RateLimit = `2^(attempt+1)s`, Transient = `2^attempt s`, capped at 60s with server `Retry-After` header support (`src/error_classifier.rs:77-96`).
- **No circuit breaker, no bulkhead, no timeout, no hedging**.
- Context overflow triggers progressive compaction (`src/agent_engine.rs:937-953`).
- Loop detector (`src/agent_engine.rs:29-83`): detects repeating tool call patterns (n-gram repetition at configurable threshold).

---

## 9. Expression / data routing [A7]

**No expression engine.** No DSL for data routing between workflow nodes — there are no nodes to route between. Data flows as plain strings/JSON through the LLM conversation context.

Negative grep evidence: searched `expression`, `$nodes`, `jmespath`, `jsonpath`, `eval_expr` in `src/` — no matches.

---

## 10. Plugin / extension system [A11]

### 10.A — Plugin BUILD process (A11.1-A11.4)

**Skills as "plugins"**: Skills are markdown files (`SKILL.md`) in `rayclaw.data/skills/<name>/`. No compilation. Manifest is YAML frontmatter inside `SKILL.md` with fields: `name`, `description`, `platforms`, `deps`, `source`, `version`, `trust_level` (`src/skills.rs:38`).

`TrustLevel` enum: `Archived` < `Candidate` < `Verified` < `Official` (`src/skills.rs:7`). Skills can be auto-generated by the skill evolution system (`src/skill_evolution.rs`), validated, and promoted through the trust hierarchy.

No formal registry, no signing, no OCI. Discovery is local directory scan.

**ACP agents**: Not plugins in a build sense. Configured in `rayclaw.data/acp.json` with `command`, `args`, `env`. Launched on demand as subprocesses.

### 10.B — Plugin EXECUTION sandbox (A11.5-A11.9)

**No WASM sandbox.** Extension execution is:
1. Skill: instructions injected into LLM context via `activate_skill` tool (`src/tools/activate_skill.rs`). The LLM then calls other tools to execute the skill's steps.
2. ACP: subprocess spawned via `tokio::process::Command` with stdin/stdout/stderr pipes (`src/acp.rs:234`). JSON-RPC over stdio (protocol version `2025-11-05`, `src/mcp.rs:12`).
3. MCP tools: child processes per MCP server config, tools federated into `ToolRegistry` (`src/tools/mcp.rs`).

No memory isolation, no CPU/memory limits, no capability-based security beyond tool authorization (High/Medium/Low risk levels with approval tokens for High-risk tools on web/control-chat, `src/tools/mod.rs:82-116`).

**Lifecycle**: ACP agents started on demand, terminated via `acp_end_session`. No hot-reload of Rust code at runtime.

**A11.9 vs Nebula**: Nebula targets WASM + capability security + commercial Plugin Fund. RayClaw uses subprocess + markdown injection — no sandboxing, no commercial monetization model for extensions.

---

## 11. Trigger / event model [A12]

**A12.1 Trigger types**:
- **Schedule**: cron (6-field format: sec min hour dom month dow via `cron` crate, `src/tools/schedule.rs`) with timezone support (`chrono-tz`).
- **Webhook**: not applicable — RayClaw receives via long-poll/WebSocket adapters, not outbound webhooks.
- **External event**: not present — no Kafka, RabbitMQ, NATS, Redis streams.
- **Channel messages**: Telegram (long-poll via teloxide), Discord (gateway WebSocket via serenity), Slack (Socket Mode WebSocket), Feishu (WebSocket or webhook), WeChat (iLink Bot WebSocket).
- **Manual**: user message in any channel.

**A12.2 Webhooks**: RayClaw does not expose webhook endpoints that external services call. It acts as a webhook consumer only for Feishu (inbound webhook mode as alternative to WebSocket).

**A12.3 Schedule**: 6-field cron, timezone-aware (chrono-tz), 60-second poll granularity. Missed schedule recovery: tasks are run on next poll if overdue (no double-fire guard beyond the poll interval).

**A12.7 Trigger as Action**: Triggers are not typed action kinds. The scheduler in `src/scheduler.rs` calls `process_with_agent` — the same agent loop entry point as channel messages. No separate `TriggerAction` abstraction.

**A12.8 vs Nebula**: Nebula's `TriggerAction` with `Input = Config` (registration) and `Output = Event` (typed payload) + `Source` trait normalizing inbound events — a 2-stage typed pipeline. RayClaw has no such abstraction: triggers are string-typed cron tasks dispatched to the shared agent loop.

---

## 12. Multi-tenancy [A14]

No multi-tenancy. Single deployment serves a single agent configuration. Per-chat isolation is via `chat_id` scoping in the database, but all chats share the same LLM credentials and config. No RBAC beyond control-chat permission model (a list of `control_chat_ids` that can run High-risk tools, `src/tools/mod.rs:149`).

---

## 13. Observability [A15]

- **Tracing**: `tracing` + `tracing-subscriber` with `EnvFilter`. Structured spans: `info!`, `warn!`, `error!` throughout. No OpenTelemetry export.
- **Metrics**: in-process `SessionMetrics` struct (`src/metrics.rs:14`) tracks per-session tool calls, token counts, error categories, loop detection, context overflow, user feedback signals.
- **LLM usage logging**: `llm_usage_logs` SQLite table with input/output tokens, model, provider, cost estimate (`src/db.rs:606`, `src/db.rs:1647`).
- **Memory injection logs**: separate table tracking per-chat memory retrieval events.
- **No OpenTelemetry, no Prometheus, no distributed tracing**.

---

## 14. API surface [A16]

- **Web API**: axum 0.7 (feature-gated), exposes REST endpoints for chat, SSE stream for agent events (`src/web.rs`). `Bearer <web_auth_token>` authentication.
- **SDK mode**: `RayClawAgent` struct in `src/sdk.rs` for library embedding — no channel adapters, no background tasks.
- **ACP**: JSON-RPC over stdio for external coding agent integration (`src/acp.rs`).
- No GraphQL, no gRPC, no OpenAPI spec generated.

---

## 15. Testing infrastructure [A19]

Inline `#[cfg(test)]` modules throughout. ~500 unit tests estimated from file inspection. Notably thorough tests in `src/llm.rs` (provider round-trips with mock HTTP), `src/tools/mod.rs` (approval flow, auth context), `src/error_classifier.rs` (all error cases), `src/metrics.rs`, `src/memory_quality.rs`. Integration tests in `tests/` directory. No public testing utilities crate.

---

## 16. AI / LLM integration [A21] — FULL DEPTH

This is the central architectural axis for rayclaw.

### A21.1 Existence

LLM integration is the **core feature**, not an add-on. Every user interaction routes through `LlmProvider`. The agent loop is the main execution primitive. Without an LLM configured, the system does not function.

### A21.2 Provider abstraction

`LlmProvider` trait at `src/llm.rs:204`:
```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn send_message(&self, system: &str, messages: Vec<Message>,
        tools: Option<Vec<ToolDefinition>>) -> Result<MessagesResponse, RayClawError>;
    async fn send_message_stream(&self, ..., text_tx: Option<&UnboundedSender<String>>)
        -> Result<MessagesResponse, RayClawError>;
}
```

Three concrete implementations:
1. `AnthropicProvider` — native Anthropic Messages API with optional prompt caching (`cache_control: {"type": "ephemeral"}` on system prompt, `src/llm.rs:277-291`).
2. `OpenAiProvider` — covers OpenAI, OpenRouter, DeepSeek, Groq, Ollama, Azure, Bedrock via proxy, Zhipu, Moonshot, Mistral, Together, Tencent, XAI, Huggingface, Cohere, Minimax, Alibaba. Base URL resolved per provider name (`src/llm.rs:868`).
3. `BedrockProvider` (`src/llm_bedrock.rs`) — AWS SigV4 signing for Bedrock-hosted models.

**BYOL endpoint**: Yes — `custom` provider type with configurable `base_url` in `rayclaw.config.yaml`.

**Local model**: Ollama supported via OpenAI-compatible endpoint (`src/llm.rs:868-888`). No candle/llama.cpp/mistral.rs.

`create_provider(config)` at `src/llm.rs:231` dispatches on `config.llm_provider` string at runtime.

### A21.3 Prompt management

System prompt built dynamically in `build_system_prompt()` (`src/agent_engine.rs:1604`). Structure:
- Optional `<soul>` XML block from SOUL.md personality file.
- Channel/chat context.
- `<memory>` block from dual memory system.
- Tool instructions section listing all 30+ available tools with guidance.
- Dynamic capability blocks (scheduling, sub-agent, ACP, MCP, skills).

No templating engine — plain Rust string formatting. No prompt version control. System prompt re-generated on every call. Few-shot examples are hardcoded in tool descriptions.

**Prompt caching**: Anthropic `cache_control: {"type": "ephemeral"}` applied to system prompt when `prompt_cache_ttl` config is not `"none"` (`src/llm.rs:277`). TTL: `"5m"` or `"1h"` options.

### A21.4 Structured output

No JSON mode, no JSON Schema enforcement on LLM output. Tool call inputs are validated only by the tool's own `execute()` method, not at the schema level. The skill evolution system uses LLM to generate `SKILL.md` content, validated by `validate_candidate_content()` (`src/skill_evolution.rs:488`) — a string-level check, not JSON Schema.

No Pydantic-style type enforcement. No re-prompting on validation failure (there is overflow recovery but not schema re-prompting).

### A21.5 Tool calling

Tool definitions exported as JSON Schema via `ToolDefinition.input_schema: serde_json::Value`. Tools are passed to the LLM as `tools: Option<Vec<ToolDefinition>>` in each request (`src/llm_types.rs:58`).

**Multi-tools per call**: yes — the agent loop processes all `ToolUse` blocks in a single response before the next LLM call (`src/agent_engine.rs:904+`).

**Execution**: same process, via `ToolRegistry::execute_with_auth()` (`src/tools/mod.rs:539`). No sandbox for tool execution — bash tool runs shell commands in the host OS (`src/tools/bash.rs`).

**Feedback loop**: multi-turn tool use — tool results appended to message history as `ToolResult` content blocks, loop continues until `end_turn` or `max_tool_iterations` (`src/agent_engine.rs:904-1080`).

**Parallel execution**: the agent processes all tool calls from one LLM response in sequence, not in parallel (one `for` loop in the agent_engine). Sub-agents run in parallel tokio tasks but are spawned by an explicit `sub_agent` tool call.

**High-risk approval**: `bash`, `acp_prompt`, `acp_submit_job`, `acp_coding` require a 2-step approval token on web channel and control chats (`src/tools/mod.rs:99-115`).

### A21.6 Streaming

SSE streaming from LLM to web channel via `send_message_stream()` with `text_tx: Option<&UnboundedSender<String>>` (`src/llm.rs:212`). SSE parser hand-written (`SseEventParser`, `src/llm.rs:125-196`). `AgentEvent::TextDelta` events forwarded to web UI (`src/agent_engine.rs:910-929`).

Streaming is for display only — the full response is assembled before tool dispatch. No backpressure mechanism on the streaming side.

### A21.7 Multi-agent

Limited multi-agent via two mechanisms:
1. **Sub-agent tool** (`src/tools/sub_agent.rs`): spawns a nested agent loop with restricted tool set (`MAX_SUB_AGENT_ITERATIONS = 10`). No message passing between parent and sub-agent during execution — task/result only. No shared memory between agents.
2. **ACP (Agent Client Protocol)** (`src/acp.rs`): spawns external coding agents (e.g., Claude Code) as subprocesses. Bidirectional JSON-RPC. Parent agent sends prompts, receives completions. External agent has its own tool loop.

No formal multi-agent topology — no explicit hand-off protocol, no shared memory between agents at the protocol level, no termination conditions beyond `end_turn`.

**Comparison with z8run (10 AI nodes — node-based)**: z8run has explicit graph nodes. RayClaw has a flat "main agent + optional sub-agent" topology, not a graph.

**Comparison with runtara-core (AiAgent step + MCP server)**: runtara-core uses MCP as the agent boundary. RayClaw also integrates MCP but as tool federation, not agent isolation.

### A21.8 RAG / vector

Optional semantic memory via `sqlite-vec` feature flag. `EmbeddingProvider` trait at `src/embedding.rs:10`:
```rust
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn model(&self) -> &str;
    fn dimension(&self) -> usize;
}
```

Two implementations: `OpenAIEmbeddingProvider` and `OllamaEmbeddingProvider` (`src/embedding.rs:16-28`). Embeddings stored in SQLite via `sqlite-vec` extension. Retrieval via KNN in `build_db_memory_context()` (`src/agent_engine.rs:1441-1463`).

Without `sqlite-vec`, falls back to keyword search (`retrieval_method = "keyword"`, `src/agent_engine.rs:1441`).

No external vector store (no Qdrant, Pinecone, pgvector, Weaviate). Retrieval as implicit memory injection, not as an explicit workflow node.

### A21.9 Memory / context

**Dual memory system**:
1. **File memory** (`src/memory.rs`): per-chat `AGENTS.md` and global `AGENTS.md` under `rayclaw.data/`. Human-readable markdown. Injected into system prompt as `<memory>` block.
2. **Structured memory** (`src/tools/structured_memory.rs`): SQLite `memories` table with category, content, embedding (if sqlite-vec). CRUD tools: `structured_memory_search`, `structured_memory_delete`, `structured_memory_update`.

**Context window management**: `max_session_messages` config field (default 40). When exceeded, older messages summarized via LLM (`compact_messages()`, `src/agent_engine.rs:1936`), keeping `compact_keep_recent` (default 20) recent messages verbatim. Full conversation archived to markdown before compaction (`src/agent_engine.rs:1825`).

**Memory reflector** (`src/scheduler.rs:362-728`): background 60s poll extracts long-term facts from conversation, deduplicates, promotes to structured memories.

**Skill evolution** (`src/skill_evolution.rs`): detects repeated tool call patterns (n-gram analysis), auto-generates `SKILL.md` files, validates, promotes through TrustLevel hierarchy.

### A21.10 Cost / tokens

`llm_usage_logs` table (`src/db.rs:606`): stores `input_tokens`, `output_tokens`, `model`, `provider`, `estimated_cost_usd` per request. `Config::estimate_cost_usd()` (`src/config.rs:576`) calculates cost from provider/model-specific per-token rates. Per-chat and global summaries queryable via `get_llm_usage_summary()` / `get_llm_usage_by_model()` (`src/db.rs:1679-1798`). Displayed in web dashboard.

No budget circuit breaker (no hard cap that stops the agent on cost overrun). No per-tenant attribution (single-tenant).

### A21.11 Observability

`SessionMetrics` per agent invocation (`src/metrics.rs:14`): tracks `total_iterations`, `tool_calls[]`, `llm_input_tokens`, `llm_output_tokens`, `error_count`, `error_categories`, `loop_detected`, `overflow_recovered`, `user_corrections`, `user_positive_signals`, `session_duration_ms`.

`ToolCallMetric` records per-tool: `tool_name`, `success`, `duration_ms`, `timestamp` (`src/metrics.rs:5`).

User feedback detection (`src/metrics.rs:83`): keyword-based detection of correction/positive signals in Chinese and English.

**LLM call tracing**: per-call token logging with model/provider/cost attribution. No prompt/response full-text logging (PII safety implicit). No eval hooks / LLM-as-judge.

**No OpenTelemetry**. Tracing via `tracing` crate with stdout/file appenders (`src/logging.rs`).

### A21.12 Safety

- **Path guard** (`src/tools/path_guard.rs`): blocklist preventing file tools from accessing `.ssh`, `.aws`, `.env`, credentials paths.
- **High-risk tool approval** (`src/tools/mod.rs:543-577`): bash, acp tools require 2-step approval token on web/control-chat channels.
- **XML sanitization** (`src/agent_engine.rs:200-212`): user message content sanitized before injection into XML-tagged system prompt to prevent prompt injection via XML confusion.
- **Working directory isolation** (`WorkingDirIsolation::Chat` vs `Shared`): per-chat isolated working directories.
- **No content filtering pre/post LLM call** — no moderation API calls, no blocked-word lists.
- **No prompt injection detection** beyond XML sanitization.

### A21.13 vs Nebula + Surge

**Nebula**: no first-class LLM abstraction; strategic bet is AI = generic actions + plugin LLM client. Surge = separate agent orchestrator on ACP. Nebula is a workflow engine where LLM *could* be one action.

**RayClaw**: LLM is the entire runtime. The "workflow" is the unstructured tool-calling conversation. AI-first, not workflow-first.

**Assessment**: RayClaw is working and well-integrated — the LLM layer is not over-coupled; the `LlmProvider` abstraction is clean, swap-able, well-tested. The tool-calling loop is production-quality (loop detection, context overflow recovery, backoff, streaming). RayClaw is a *peer platform* to Surge, not to Nebula. It answers "how do you run an AI assistant"; Nebula answers "how do you build deterministic multi-step business workflows."

For Nebula, the most borrowable ideas are:
- **LLM error classification** (`src/error_classifier.rs`) — the 5-category enum with provider-specific classifiers and backoff is clean and reusable.
- **Dual memory (file + structured DB)** — the `EmbeddingProvider` abstraction with optional semantic memory is a good pattern for an AI action in Nebula.
- **Prompt caching integration** — `cache_control: ephemeral` on system prompt with configurable TTL (`"5m"`, `"1h"`, `"none"`) is a practical optimization worth adopting in any Nebula LLM plugin action.
- **Skill evolution** — the pattern of detecting repeated tool call sequences and auto-generating skill files is novel and potentially relevant to Nebula's Plugin Fund if Nebula builds an LLM orchestration layer.

---

## 17. Notable design decisions

1. **Single-crate monolith with feature flags**: simplifies distribution (one binary covers all channels) at the cost of boundary enforcement. Works for a single-author personal agent.

2. **SQLite as the only persistence layer**: eliminates operational dependencies (no Postgres required), supports embedded/desktop use. WAL mode provides concurrent read performance. Downside: no horizontal scaling, no RLS, no advanced query optimization.

3. **LLM = orchestrator, not tool**: unlike most workflow engines where LLM is one node in a graph, RayClaw makes LLM the scheduler. This is expressive but non-deterministic — the same input can produce different tool call sequences.

4. **Skill evolution via LLM-generated code**: the system observes repeated tool patterns and asks the LLM to synthesize a reusable skill. This is a genuine self-improvement loop, though security depends entirely on trust levels and the path guard.

5. **ACP as multi-agent boundary**: using JSON-RPC/stdio as the agent-to-agent protocol (the same protocol as Claude Code) means RayClaw can orchestrate other AI assistants without a shared type system. Practical but opaque.

6. **Context compaction via LLM summarization**: when session history is too long, older messages are summarized by the LLM before being dropped. This is the standard approach but loses exact tool call history — incompatible with strict audit requirements.

---

## 18. Known limitations / pain points

Only 1 GitHub issue found (no public issue tracker activity). From code comments and docs:

1. **Hot-reload not implemented** (`RELOAD.md`): the self-healing bot concept is aspirational. Missing pieces: trigger-from-chat mechanism, auto-rollback, explicit state persistence verification.

2. **MCP SDK migration under evaluation** (`docs/mcp-sdk-evaluation.md`): current `src/mcp.rs` is a hand-rolled JSON-RPC client; evaluating migration to official Rust MCP SDK.

3. **SQLite `Mutex<Connection>` bottleneck** (`src/db.rs:11`): all DB access is serialized through one mutex. Under high concurrency (many active chats), this is a write bottleneck.

4. **No structured output enforcement**: tool inputs validated at runtime only, no compile-time or schema-level guarantees. Schema drift between `definition()` and `execute()` is possible.

5. **Sub-agent limited to 10 iterations** (`src/tools/sub_agent.rs:15`): may be insufficient for complex tasks.

---

## 19. Bus factor / sustainability

- **Maintainer count**: 1 (rayclaw GitHub org, single author visible from commits).
- **Commit cadence**: active — v0.2.1 → v0.2.5 with feature additions (Feishu phase 2, WeChat, skill evolution, error classifier) in recent months.
- **Issues**: 1 closed issue. Low community engagement (no starred-issue analysis possible).
- **Rust 1.95 compliance**: Clippy fix for `while_let_loop` already merged (`a08e49a`), showing active maintenance.
- **Desktop companion**: `rayclaw-desktop` (separate repo) signals product ambitions beyond CLI.

---

## 20. Final scorecard vs Nebula

| Axis | RayClaw approach | Nebula approach | Verdict | Borrow? |
|------|-----------------|-----------------|---------|---------|
| A1 Workspace | Single crate, feature flags, ~39K LOC | 26 crates layered, Edition 2024 | Different goals — RayClaw simple binary, Nebula enforced modularity | no — different goals |
| A2 DAG | No DAG — LLM-directed tool call sequence | TypeDAG L1-L4 petgraph | Different decomposition — RayClaw is AI-driven, Nebula is deterministic | no — different goals |
| A3 Action | Open `Tool` trait, `Box<dyn Tool>`, JSON I/O, no versioning, no lifecycle hooks | 5 sealed action kinds, assoc types, versioning, derive macros | Nebula deeper for workflow; RayClaw simpler for AI dispatch | refine — borrow simple tool registry pattern for AI-action plugin SDK |
| A11 Plugin BUILD | Markdown SKILL.md, no compilation, TrustLevel enum | WASM, plugin-v2 spec planned | Different — RayClaw's skills are prompt-injected docs, not compiled code | maybe — skill trust level concept applicable to Nebula plugin review |
| A11 Plugin EXEC | Subprocess (ACP/MCP), no sandbox, approval token for high-risk tools | WASM sandbox + capability security | Nebula deeper — RayClaw has no memory isolation | no — Nebula's WASM model is correct for production |
| A18 Errors | `RayClawError` via thiserror, `LlmErrorCategory` 5-category classifier, exponential backoff | nebula-error + ErrorClass + ErrorClassifier in nebula-resilience | Convergent — both have classified errors; RayClaw's LLM-specific classifier is complementary | refine — adopt LLM error category names in Nebula's resilience crate for LLM plugin |
| A21 AI/LLM | **Full — LlmProvider trait, 3 backends + BYOL, streaming, sub-agent, ACP multi-agent, vector memory, token tracking, error classification, prompt caching** | None yet — bet: AI = generic actions + LLM plugin | Competitor deeper (by design) — this is RayClaw's core vs Nebula's non-feature | yes — borrow error classifier, embedding provider abstraction, prompt cache TTL pattern for future Nebula LLM action |
