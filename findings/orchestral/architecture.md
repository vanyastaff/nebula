# orchestral — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/sizzlecar/orchestral
- **Latest tag:** v0.2.0
- **License:** MIT (`LICENSE` file)
- **Rust edition:** 2021 (`Cargo.toml` workspace.package.edition)
- **Pinned toolchain:** Rust 1.91.0 stable (`rust-toolchain.toml`)
- **Governance:** Solo maintainer (sizzlecar), open source, no commercial model mentioned
- **Issues:** 0 open issues at time of research (gh issue list returned empty)
- **Stars/forks:** Not retrieved (GitHub API not queried)
- **Published crates:** orchestral and orchestral-cli published to crates.io (v0.2.0)

---

## 1. Concept positioning [A1, A13, A20]

**From the author's README:** "Workflow orchestration for grounded agents. Orchestrates stateful workflows, not one-off tool calls. Executes typed actions with an agent loop and mini-DAG. Replans from real state and verifies before finishing."

**My one-sentence after reading code:** orchestral is an LLM-as-scheduler runtime where a multi-provider planner (OpenAI / Anthropic / Gemini / Ollama / OpenRouter / etc.) generates and executes dynamic DAG plans from natural-language intent, with an agent loop for self-correction, MCP bridge for external tools, and a Skill system for domain knowledge injection.

**Comparison with Nebula:** Nebula is a developer-built workflow DAG engine where humans define typed action pipelines statically (5 action kinds, sealed traits, strong compile-time checks). orchestral inverts this: the LLM is the planner and the developer registers atomic capabilities (Action impls); the LLM decides which actions to call and in what order. Nebula targets n8n+Temporal use cases; orchestral targets Copilot/agent-shell use cases.

---

## 2. Workspace structure [A1]

**Crate count:** 6 workspace members across 3 layers.

```
core/orchestral-core     — pure abstractions (no I/O): Intent/Plan/Step/Task, Action/Planner/Executor/Normalizer traits, stores, DAG
core/orchestral-runtime  — stateful runtime: LLM planners, built-in actions, MCP bridge, skill system, orchestrator, agent loop
core/orchestral          — facade: re-exports core + runtime
apps/orchestral-cli      — CLI + TUI (ratatui/clap), scenario runner
apps/orchestral-telegram — Telegram bot adapter
examples                 — runnable demos
```

**Source:** `Cargo.toml` lines 3-15.

**Dependency rule enforced by CI** (`CLAUDE.md`): "core crates must never depend on `platform/` or `apps/`".

**Feature flags:** none observed in workspace `Cargo.toml`. No umbrella feature-flag configuration.

**Key workspace deps:** tokio 1 full, async-trait 0.1, serde 1 + serde_json, thiserror 1, tracing 0.1, reqwest 0.12, uuid 1, chrono 0.4, serde_yaml 0.9.

**Comparison with Nebula:** Nebula has 26 crates with deep layering (nebula-error / nebula-resilience / nebula-credential / nebula-resource / nebula-action / nebula-engine / nebula-tenant etc.). orchestral has 6, meaning far less domain decomposition — no separate crates for errors, resilience, credentials, resources, multi-tenancy. This is appropriate for a smaller-scope tool but would not scale to enterprise workflow engine needs.

---

## 3. Core abstractions [A3, A17] — DEEP

### A3.1 Trait shape

The `Action` trait is defined in `core/orchestral-core/src/action/mod.rs:30-44`:

```rust
#[async_trait]
pub trait Action: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn metadata(&self) -> ActionMeta { ActionMeta::new(self.name(), self.description()) }
    async fn run(&self, input: ActionInput, ctx: ActionContext) -> ActionResult;
}
```

**Open, not sealed.** Any downstream crate can implement `Action`. No sealing mechanism via `pub(crate)` supertrait or similar. Trait-object compatible: `dyn Action` is used throughout (e.g., `Vec<Arc<dyn Action>>` in `sdk.rs:57`).

**Associated types: none.** The trait has no associated types — `Input` is always `ActionInput` (a newtype over `serde_json::Value`), `Output` is always `ActionResult` (an enum). No GATs, no HRTBs, no typestate on the Action trait itself.

**Default method:** `metadata()` has a default implementation returning an `ActionMeta` built from `name()` and `description()`.

### A3.2 I/O shape

`ActionInput` (`core/orchestral-core/src/action/input.rs:8-12`):
```rust
pub struct ActionInput {
    pub params: Value,  // serde_json::Value — completely type-erased
}
```

`ActionResult` (`core/orchestral-core/src/action/result.rs:17-52`) is an enum:
```rust
pub enum ActionResult {
    Success { exports: HashMap<String, Value> },
    NeedClarification { question: String },
    NeedApproval { request: ApprovalRequest },
    RetryableError { message: String, retry_after: Option<Duration>, attempt: u32 },
    Error { message: String },
}
```

Both input and output are fully type-erased via `serde_json::Value`. No compile-time I/O typing. The `ActionResult::Success.exports` is a `HashMap<String, Value>` fed into the `WorkingSet` for inter-step data passing.

**Streaming output:** `complete_stream` exists on `LlmClient` (`planner/llm.rs:70-82`) with a `StreamChunkCallback = Arc<dyn Fn(String) + Send + Sync>`. This is streaming for LLM text responses back to the UI, not for action outputs as workflow nodes.

**Side-effects model:** actions can do anything; there is no capability declaration enforced at runtime (capabilities in `ActionMeta` are advisory strings).

### A3.3 Versioning

No versioning system. Actions are identified by name string only (`fn name(&self) -> &str`). No `#[deprecated]`, no version field, no v1/v2 distinction. The `ActionMeta` struct has no version field.

**Negative evidence:** grep for "version" in `core/orchestral-core/src/action/` returns only `ActionMeta::new` boilerplate — no versioning design.

### A3.4 Lifecycle hooks

The `LifecycleHook` trait (`core/orchestral-core/src/spi/lifecycle.rs:120-141`) provides:
- `on_turn_start` — before planner runs
- `on_plan_created` — after planner generates plan, before normalization (plan can be mutated)
- `before_step` — returns `StepDecision::Continue | Skip`
- `after_step` — after step (success or failure)
- `on_execution_complete` — after full DAG finishes
- `on_turn_end` — after final output determined

All are `async`. No pre-execute / cleanup / on-failure per-action hooks. Cancellation: `CancellationToken` (from `tokio-util`) is re-exported in `action/mod.rs:20` but is not a required `Action::run` parameter — cancellation is implicit.

**Idempotency key:** no built-in idempotency key on `Action` or `Step`.

### A3.5 Resource and credential deps

No formal resource or credential declaration mechanism on `Action`. Actions access the environment via `ActionContext` (`core/orchestral-core/src/action/context.rs`) which provides access to stores, working set, and cancellation token. There is no typed resource injection ("I need DB pool X + credential Y"). Configuration is pulled via environment variables at startup.

### A3.6 Retry/resilience attachment

`ActionResult::RetryableError { message, retry_after, attempt }` signals to the executor that the action can be retried. The executor handles the retry loop. There is no per-action retry policy declaration — no `#[retry(max=3)]` or equivalent. The Step has no retry policy field.

### A3.7 Authoring DX

**No derive macro.** Authors implement `Action` manually. A minimal "hello world" action requires:
1. `impl Action for MyAction` — name, description, run (6-10 lines)
2. Register via `Orchestral::builder().action(MyAction::new())`

Line count for hello-world: ~12 lines (struct definition + impl block). See `examples/sdk_quickstart.rs`.

### A3.8 Metadata

`ActionMeta` (`core/orchestral-core/src/action/mod.rs:47-65`) carries: `name`, `description`, `category: Option<String>`, `input_schema: serde_json::Value`, `output_schema: serde_json::Value`, `capabilities: Vec<String>`, `input_kinds: Vec<String>`, `output_kinds: Vec<String>`.

No i18n support. All metadata is runtime strings, not compile-time constants. The schemas are JSON Schema values used for LLM planning hints, not for validation.

### A3.9 vs Nebula

| | orchestral | Nebula |
|---|---|---|
| Action kinds | 1 (Action trait) + StepKind variants (Action/WaitUser/WaitEvent/System/Agent) | 5 sealed kinds (Process/Supply/Trigger/Event/Schedule) |
| Sealing | Open trait (any crate can implement) | Sealed (only nebula-owned crates can add kinds) |
| I/O typing | Fully type-erased (Value in, HashMap<String,Value> out) | Associated Input/Output/Error types per action |
| Versioning | None | Type identity (implicit) |
| Derive macro | None | nebula-derive |
| Compile-time safety | None on action boundary | Strong (sealed traits, assoc types) |

orchestral's Action trait is simpler and more open. Nebula's 5 action kinds with sealed traits and associated types give stronger compile-time guarantees but require more scaffolding.

---

## 4. DAG / execution graph [A2, A9, A10]

### Graph model

`ExecutionDag` (`core/orchestral-core/src/executor/dag.rs:68-75`) is a runtime `HashMap<String, DagNode>` with explicit `ready_nodes: Vec<String>`. DAG is constructed from a `Plan` (list of `Step`s with `depends_on: Vec<StepId>`). Dynamic modification is supported via `ExecutionDag::add_node` when `dynamic == true`.

**Step types** (`core/orchestral-core/src/types/step.rs:69-81`):
- `Action` — normal action execution
- `WaitUser` — pause for user input
- `WaitEvent` — pause for external event
- `System` — built-in system steps (e.g., resolve_reference)
- `Agent` — constrained internal LLM loop for iterative local exploration (leaf agent)

**Port typing:** None. All inter-step data flows via `WorkingSet: HashMap<String, Value>` with `StepIoBinding` records (`from: String, to: String`). No compile-time port typing.

**Compile-time DAG checks:** None. DAG validation is runtime-only via `PlanValidator / PlanFixer` in `normalizer/mod.rs`. The normalizer checks dependency existence and cycles.

**Agent loop:** Up to 6 planner iterations (configurable). After each execution the result becomes an observation; the planner replans if not terminal. This is the key "grounded" loop (`CLAUDE.md`: "Agent Loop (multi-iteration): Planner generates PlannerOutput — if not terminal, execution result becomes observation; loop continues").

**Comparison with Nebula:** Nebula's TypeDAG has 4 levels (static generics → TypeId → refinement predicates → petgraph). orchestral uses a plain `HashMap` with no petgraph, no compile-time checks. Nebula is deeper for safety; orchestral is lighter and dynamically modified at LLM discretion.

---

## 5. Persistence and recovery [A8, A9]

**Storage:** Two store traits in `core/orchestral-core/src/store/`:
- `EventStore` — append-only event journal (`event_store.rs`)
- `TaskStore` — mutable task state (`task_store.rs`)

Default impls are in-memory: `InMemoryEventStore` and `InMemoryTaskStore`.

**Backend config** (`core/orchestral-core/src/config/mod.rs`: `StoresConfig`): SQLite and PostgreSQL options are referenced in config structs (`default_store_backend() = "sqlite"`). Blob storage: local filesystem and S3 (`S3BlobsConfig` with `access_key_env`, `secret_key_env`).

**Persistence model:** Event-sourced append-only log. Task state reconstructed from events. No explicit frontier/checkpoint language — the concept is present via `restore_checkpoint` in `orchestrator/state.rs`.

**Recovery:** `restore_checkpoint` function exists in `core/orchestral-runtime/src/orchestrator/state.rs`. The orchestrator has a `recovery` module (`orchestrator.rs:14: #[cfg(test)] mod recovery`).

**Comparison with Nebula:** Nebula has frontier-based scheduler with explicit checkpoint recovery, sqlx + PgPool with migrations and RLS. orchestral has similar concepts (event store + task store) but no production-grade PostgreSQL layer visible in the public crates, defaulting to in-memory for both.

---

## 6. Credentials / secrets [A4] — DEEP

### A4.1 Existence

No separate credential layer. API keys are managed via environment variable names stored in config structs.

**Grep evidence:**
- Search for `credential` in `*.rs`: 0 results
- Search for `zeroize` in `*.rs`: 0 results
- Search for `secrecy` in `*.rs`: 0 results
- S3 config has `access_key_env: Option<String>` and `secret_key_env: Option<String>` (`config/mod.rs:426-428`) — these store env var *names*, not the values themselves

### A4.2 Storage

No at-rest encryption. API keys are read from environment variables at startup (`factory.rs:107-121`). No vault integration, no OS keychain.

### A4.3 In-memory protection

None. No Zeroize, no `secrecy::Secret<T>`. API keys stored as plain `String`.

### A4.4 Lifecycle

CRUD not applicable — keys are env vars. No refresh model. No revocation. No expiry detection.

### A4.5 OAuth2/OIDC

No OAuth2 support. Bearer token support exists only for MCP server connections (`McpServerSpec.bearer_token_env_var: Option<String>`, `config/mod.rs:543`) — this is a static env var lookup, not an OAuth2 flow.

### A4.6–A4.9

Not applicable. No credential composition, no type safety on credentials, no LiveCredential watch(), no blue-green refresh.

**A4.9 vs Nebula:** orchestral has none of Nebula's credential subsystem features (State/Material split, LiveCredential watch(), blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter erasure). This is expected — orchestral targets developer-on-laptop use cases where env vars suffice.

---

## 7. Resource management [A5] — DEEP

### A5.1 Existence

No separate resource abstraction. Actions create and manage their own connections (e.g., `McpServerAction` manages its own stdio/HTTP connections per invocation in `action/mcp.rs`).

**Grep evidence:**
- Search for `Resource` trait in `*.rs`: 0 results matching a resource lifecycle trait
- Search for `ReloadOutcome` in `*.rs`: 0 results
- Search for `pool` in `*.rs`: 0 results matching connection pool management

### A5.2–A5.8

Not applicable. No scope levels, no lifecycle hooks (init/shutdown/health-check), no hot-reload, no generation tracking, no credential-to-resource notification, no backpressure.

**A5.8 vs Nebula:** orchestral has none of Nebula's 4 scope levels, ReloadOutcome enum, generation tracking, or on_credential_refresh. Actions in orchestral are stateless executors that open connections on demand.

---

## 8. Resilience [A6, A18]

**Retry:** `ActionResult::RetryableError { retry_after, attempt }` signals retryability. The executor in `executor/run.rs:538` handles retry loops. No separate resilience crate.

**Circuit breaker:** No circuit breaker. Grep for `circuit_breaker` / `CircuitBreaker` returns nothing.

**Bulkhead:** No bulkhead. Grep for `bulkhead` returns nothing.

**Timeout:** MCP tool calls have `tool_timeout_ms` (`McpServerSpec`, `config/mod.rs`). No generic timeout policy.

**Hedging:** No hedging.

**Error classification:** Implicit via `ActionResult::RetryableError` vs `ActionResult::Error` (transient vs permanent). No `ErrorClassifier` abstraction.

**Error type (A18):** `thiserror` is used throughout. Each module defines its own error enum with `#[derive(thiserror::Error)]`. Examples: `PlanError` in `planner/mod.rs:48`, `LlmError` in `planner/llm.rs:109`, `ConfigError` in config loader, `ActionBuildError` in action factory, `NormalizeError` in normalizer, `StoreError` in stores. No unified `nebula-error` equivalent — errors are local per-module.

**Comparison with Nebula:** Nebula has a dedicated `nebula-resilience` crate with retry/CB/bulkhead/timeout/hedging and a unified `ErrorClassifier`. orchestral's resilience is minimal and embedded in executor logic.

---

## 9. Expression / data routing [A7]

No expression engine or DSL. Inter-step data flows via `WorkingSet: HashMap<String, Value>` with `StepIoBinding` records. LLM-generated plans reference binding keys as string names. No `$nodes.foo.result.email` syntax or similar. No sandboxed eval.

**Negative evidence:** Grep for `expression\|eval\|jinja\|jsonpath\|handlebars` in `*.rs` returns nothing in core/runtime. The `interpreter` module (`core/orchestral-core/src/interpreter/mod.rs`) exists but appears to be a simple binding resolver, not a full expression evaluator.

---

## 10. Plugin / extension system [A11] — DEEP (BUILD + EXEC)

### 10.A — Plugin BUILD process (A11.1–A11.4)

**A11.1 Format:** No WASM, no dynamic library, no OCI container. Extensions are either:
1. MCP servers — separate processes (any language) invoked via stdio or HTTP
2. Skills — `SKILL.md` markdown files auto-discovered from `.claude/skills/`, `.codex/skills/`, `skills/` dirs
3. `RuntimeExtensionSpec` in YAML config — a named extension with `options: Value` (reserved for future use, `config/mod.rs:582-594`)

**A11.2 Toolchain:** MCP servers compile separately (any language). Skills are markdown documents, no compilation. No SDK for writing extensions in Rust.

**A11.3 Manifest content:** MCP server config in `McpServerSpec` (`config/mod.rs:526-551`) includes: `name, enabled, required, command, args, env, url, headers, bearer_token_env_var, startup_timeout_ms, tool_timeout_ms, enabled_tools, disabled_tools`. No capability declaration (network/fs/crypto), no permission grants.

**A11.4 Registry/discovery:** MCP servers auto-discovered from `.mcp.json` files in the filesystem at startup (`CLAUDE.md`). Skills auto-discovered from skill directories. No remote registry, no signing, no version pinning for MCP tools.

### 10.B — Plugin EXECUTION sandbox (A11.5–A11.9)

**A11.5 Sandbox type:** MCP servers run as subprocesses (stdio pipe: `StdioMcpSession` in `action/mcp.rs`) or HTTP endpoints. The `McpServerAction` uses `tokio::process::Command` + `BufReader` on stdio. No WASM sandbox, no memory isolation beyond OS process boundaries.

**A11.6 Trust boundary:** MCP server processes are trusted (no capability-based security). No CPU/memory/wall-time limits enforced by orchestral. `tool_timeout_ms` gives a per-call deadline only.

**A11.7 Host-plugin calls:** JSON-RPC over stdio (MCP protocol). Marshaling via `serde_json`. No async protocol crossing — requests are serial within a session (`session.request()` is awaited). Error propagation via JSON error field.

**A11.8 Lifecycle:** MCP sessions initialized on demand per action invocation (`StdioMcpSession::connect()` on each run, or pooled — see `mcp.rs`). No hot reload, no crash recovery.

**A11.9 vs Nebula:** Nebula targets WASM sandbox + capability security + Plugin Fund commercial model. orchestral uses OS process (MCP server) as extension boundary with no capability security. No commercial monetization model. MCP is already a defacto standard and orchestral rides it cleanly, which is practically stronger than Nebula's yet-unimplemented WASM plan.

---

## 11. Trigger / event model [A12] — (Tier 3: answered for context, not required deep)

Orchestral's entry point is a `UserInput` event passed to `ThreadRuntime`. Events are: `UserInput, AssistantOutput, Artifact, ExternalEvent, SystemTrace, ResumeAfterWait` (`CLAUDE.md`). The `ConcurrencyPolicy` trait (`concurrency.rs`) decides what to do when a new event arrives: Interrupt / Queue / Parallel / Merge / Reject.

No webhook registration. No cron scheduler. No Kafka/RabbitMQ integration. No FS watch. External events (`ExternalEvent`) are a type in the event model but no connectors are provided in the current codebase.

---

## 12. Multi-tenancy [A14]

Not applicable. No tenant isolation, no RBAC, no SSO, no SCIM. Single-user CLI/bot tool.

---

## 13. Observability [A15]

**Tracing:** `tracing` crate (0.1) is used (`Cargo.toml` workspace.dependencies). `tracing-subscriber` with `env-filter` feature for log output. Usage is scattered `tracing::debug!` / `tracing::warn!` calls in CLI event loop (`apps/orchestral-cli/src/tui/event_loop.rs`).

No OpenTelemetry. No structured per-execution trace. No metrics crate (no `metrics` or `prometheus` in workspace deps).

**Comparison with Nebula:** Nebula uses full OpenTelemetry with structured tracing per execution (one trace = one workflow run). orchestral has basic `tracing` debug logging only.

---

## 14. API surface [A16]

**Programmatic API:** Builder API via `Orchestral::builder()` in `core/orchestral-runtime/src/sdk.rs`. Documented in README and CLAUDE.md.

**Network API:** None. orchestral is a library/CLI, not a server. No REST, no GraphQL, no gRPC.

**CLI:** `orchestral-cli` with `run` and `scenario` subcommands.

**Bot adapter:** `orchestral-telegram` as a separate app crate.

---

## 15. Testing infrastructure [A19]

No dedicated testing crate. Tests are inline in source files. `#[cfg(test)]` modules throughout.

From `REFACTOR_PLAN.md`: "cargo test passes 167 tests". Scenario smoke tests in `configs/scenarios/*.smoke.yaml` run via `orchestral-cli scenario` command.

No public testing utilities, no contract tests, no wiremock/mockall usage visible in workspace deps.

---

## 16. AI / LLM integration [A21] — FULL DEEP (A21.1–A21.13)

### A21.1 Existence

**Built-in and central.** LLM integration is the core feature of orchestral. The LLM is the planner — without it the system cannot generate execution plans. The `LlmClient` trait and `LlmPlanner` are in `core/orchestral-runtime/src/planner/`.

### A21.2 Provider abstraction

**Multi-provider.** The `LlmClient` trait (`planner/llm.rs:55-82`):
```rust
#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn complete(&self, request: LlmRequest) -> Result<String, LlmError>;
    async fn complete_with_tools(...) -> Result<LlmResponse, LlmError>;
    async fn complete_stream(...) -> Result<String, LlmError>;
}
```

**Providers supported** (via `graniet/llm` SDK, `factory.rs:136-148`):
- `openai`
- `google` / `gemini`
- `anthropic` / `claude`
- `deepseek`
- `groq`
- `xai`
- `mistral`
- `cohere`
- `openrouter` (meta-provider for all above)
- `ollama` (local model support)

**BYOL endpoint:** `BackendSpec.endpoint: Option<String>` allows custom base URL override (`factory.rs:96`).

**Local model support:** `ollama` backend is explicitly handled (`bootstrap/runtime_builder.rs:280`, `factory.rs:169`).

There is also a separate direct Gemini client implementation in `planner/gemini.rs` (does not go through the `graniet/llm` SDK) for compatibility.

### A21.3 Prompt management

**System prompt:** `LlmPlannerConfig.system_prompt: String` injected into every request. Prompt templates are files in `core/orchestral-runtime/src/prompts/` (constitution, execution rules, JSON examples — per `CLAUDE.md`).

**Conversation structure:** `PlannerContext.history: Vec<HistoryItem>` where each item has `role: String` (user/assistant/system), `content: String`, `timestamp`. Max history: `LlmPlannerConfig.max_history = 20` items (`planner/llm.rs:133`).

**Skills injection:** `PlannerContext.skill_instructions: Vec<SkillInstruction>` — keyword-matched `SKILL.md` files injected into the planning prompt for domain knowledge.

**Versioning of prompts:** Prompt files in `prompts/` directory. No version pinning or migration support. Prompts are not checked into workflow definition (they are always the current prompts).

**Few-shot:** Not formalized — history items include prior plan/result observations that serve as implicit few-shot.

### A21.4 Structured output

The planner requests structured JSON output (the LLM must emit a `PlannerOutput` in JSON format). Parsing in `planner/llm/parsing.rs`. `extract_json` function extracts JSON from LLM text. `parse_planner_output` deserializes it.

**Re-prompting on validation fail:** `PlanNormalizer` validates and fixes plans post-LLM. `PlanFixer` trait exists for auto-repair. No explicit "re-prompt the LLM" on parse failure — invalid output → normalizer tries to fix → error escalation.

**Schema enforcement:** Input schemas in `ActionMeta.input_schema: serde_json::Value` are JSON Schema values provided to the planner as hints, not enforced at runtime against action inputs.

### A21.5 Tool calling

**Definition format:** Each action registered as an `ActionMeta` with `name`, `description`, `input_schema` (JSON Schema). These are presented to the LLM as the tool catalog.

**`complete_with_tools`** on `LlmClient` (`planner/llm.rs:62-68`) accepts `&[ToolDefinition]` for native LLM function calling (the LLM emits a tool call response). The default impl falls back to text completion.

**MCP deferred schema loading:** The planner catalog shows MCP tool names and descriptions only. The planner calls `tool_lookup` action to get the full input schema before invoking an MCP tool — this avoids loading all schemas at startup.

**Multi-tools per call:** The planner can generate a `MiniPlan` with multiple steps in one turn. Within a single `complete_with_tools` call, only one tool is called (LLM selects one); parallelism comes from the DAG executor running multiple ready steps concurrently.

**Execution sandbox:** Tool actions execute in the same process (builtin) or as subprocess (MCP). No WASM sandbox.

**Feedback loop (multi-turn):** The agent loop (`orchestrator/agent_loop.rs`) feeds execution results back as observations for the next planner iteration — up to 6 iterations (`sdk.rs:79: max_planner_iterations: 6`).

### A21.6 Streaming

`LlmClient::complete_stream` (`planner/llm.rs:70-82`) accepts a `StreamChunkCallback = Arc<dyn Fn(String) + Send + Sync>`. This streams LLM text responses to the UI (TUI or Telegram bot).

**Streaming into workflow nodes:** Not applicable — the planner generates a full plan (or single action) before execution begins. Streaming is UI feedback only, not a DAG data source.

**Backpressure:** None. The callback is called synchronously on each chunk.

### A21.7 Multi-agent

**`StepKind::Agent`** (`types/step.rs:80-81`): "Constrained internal LLM loop for iterative local exploration." The executor has an `AgentStepExecutor` trait (`executor/mod.rs:111`) for executing agent steps.

**`leaf_agent`** step (`types/step.rs:199-212`): Agent step with `mode: "leaf"` param — bounded exploration.

**Hand-off:** Not formalized. The outer agent loop feeds results back to the planner. There is no typed hand-off protocol between agents.

**Shared memory:** `WorkingSet: HashMap<String, Value>` serves as shared state across steps within a plan turn.

**Termination conditions:** `max_iterations: 6` in the outer loop. The planner outputs `Done(String)` or `NeedInput(String)` as terminal signals.

**Comparison with other AI-first competitors:** aofctl has 5 named fleet coordination modes (Hierarchical/Peer/Swarm/Pipeline/Tiered). orchestral has a single loop-and-replan model plus optional leaf-agent sub-steps. Less structured than aofctl but simpler and more practical for single-model agentic tasks.

### A21.8 RAG / vector

**No RAG.** Grep for `qdrant`, `pinecone`, `pgvector`, `weaviate`, `embedding` in `*.rs` returns no hits in the core/runtime source (some UI strings in `apps/orchestral-cli/src/tui/ui.rs` appear but are unrelated).

**Retrieval as workflow node:** The skill system provides domain knowledge via `SKILL.md` injection (keyword matching → inject into prompt). This is text-based retrieval, not vector similarity.

### A21.9 Memory / context

**Conversation memory:** `Thread` struct (`thread.rs`) holds per-thread (per-session) interaction history. `PlannerContext.history: Vec<HistoryItem>` with `max_history = 20` configurable truncation.

**Context window management:** `BasicContextBuilder` and `TokenBudget` in `core/orchestral-runtime/src/context/`. `TokenBudget` (`context/mod.rs:32-64`) tracks estimated token usage. Budget: `max_context_tokens` (default configurable). Truncation strategy: last N history items.

**Long-term memory:** No persistent memory across sessions. Memory is in-process and lost on restart (default in-memory stores).

**Working set preview:** `PlannerLoopContext.working_set_preview: Option<String>` — a string summary of the current working set passed to the planner for context within the agent loop (`planner/mod.rs:113`).

### A21.10 Cost / tokens

**Token estimation:** `TokenBudget::estimate_tokens(&self, content: &str) -> usize` using a `chars_per_token` ratio (default 4.0 chars/token, `context/mod.rs:46`). This is a rough heuristic, not an exact API tokenizer.

**No cost calculation.** No per-provider cost table, no per-token pricing, no budget circuit breakers, no per-tenant attribution.

**Actual API usage stats:** Not tracked. `LlmResponse` has no token count field. `complete()` returns `String` only.

### A21.11 Observability

**Prompt logging:** `LlmPlannerConfig.log_full_prompts: bool` (default `false`, `planner/llm.rs:138`). When enabled, full prompts are logged via `tracing::debug!` with truncation at `MAX_PROMPT_LOG_CHARS = 4_000` chars and `MAX_LLM_OUTPUT_LOG_CHARS = 8_000` chars.

**Per-LLM-call tracing:** Basic `tracing::debug!` calls in the planner path (not structured spans).

**Eval hooks (LLM-as-judge):** None.

**PII-safe logging:** No PII detection or redaction. Prompt logging truncation reduces exposure but is not PII-aware.

### A21.12 Safety

**Content filtering:** None. No pre/post content filtering.

**Prompt injection mitigations:** None explicit. The `constitution` prompt file in `prompts/` likely contains behavioral guidelines, but there is no programmatic injection mitigation.

**Output validation:** `PlanNormalizer` validates plan structure (DAG correctness, missing deps). No semantic validation of LLM output content.

**Approval gate:** `ActionResult::NeedApproval` allows actions to request explicit user approval before executing destructive operations (e.g., shell commands). The TUI shows an approval modal (`apps/orchestral-cli/src/tui/bottom_pane/modal.rs`).

### A21.13 vs Nebula + Surge

**Nebula** has no first-class LLM abstraction. Nebula's bet: AI workflows = generic actions + plugin LLM client. Surge = separate agent orchestrator on ACP.

**orchestral is the inverse:** LLM is mandatory and central (you must supply an API key). The developer builds capabilities (actions) and the LLM decides the workflow. This is a fundamentally different architecture — orchestral is an agent runtime, not a workflow engine.

**Working vs over-coupled:** orchestral's LLM integration is working (the system is functional, has tests, is published to crates.io). The coupling is intentional: the system cannot function without an LLM. This is not over-engineering but rather the core design thesis. The graniet/llm SDK dependency adds another abstraction layer but the default factory hides it cleanly.

**Key differentiators vs other AI-first crates:**
- vs aofctl: orchestral has richer agent loop / MCP bridge; aofctl has more fleet coordination modes
- vs cloudllm: cloudllm focuses on multi-provider chat API; orchestral adds DAG execution, skill system, and agent loop
- vs runtara-core: runtara-core has MCP server built-in; orchestral has MCP as extension
- vs rayclaw: rayclaw uses LLM-as-scheduler for pipeline routing; orchestral uses LLM-as-scheduler for full DAG planning with replanning

---

## 17. Notable design decisions

**1. LLM-as-planner with replanning loop.** The central bet: LLM generates a DAG plan, executor runs it, observations feed back to LLM for correction. Up to 6 iterations. This makes orchestral robust to imperfect initial plans at the cost of LLM API calls. Applicable to Nebula: Surge project already covers this; Nebula core is rightly statically-planned.

**2. MCP bridge as the plugin model.** Instead of a custom plugin ABI (WASM, dynamic library), orchestral uses MCP (Model Context Protocol) servers as the extension point. Any language, any process. This is pragmatic and aligns with emerging AI tooling ecosystem norms. Nebula's WASM plan is more ambitious but unimplemented; MCP is shipping today.

**3. Type erasure everywhere.** `ActionInput = Value`, `ActionResult::Success.exports = HashMap<String, Value>`. Zero compile-time checking on action I/O. This enables fully dynamic LLM-driven workflows but sacrifices the safety that makes Nebula's design distinctive. The trade-off is practical for an LLM-driven system where the planner (not the developer) defines parameter names.

**4. Skills as text documents (SKILL.md).** Domain knowledge injected via markdown files rather than typed skill objects. Simple to author, zero Rust code required. The keyword-matching injection is heuristic (scored by string similarity). Risk: irrelevant skills accidentally injected if keywords collide.

**5. Action selector pre-filter.** When action count >= 30, a two-stage planning is used: first LLM call selects relevant actions (`build_action_selector_prompt`), second call generates the full plan with only those actions in context. This manages context window size as action catalog grows (`planner/llm.rs:175-179`).

**6. Open Action trait.** Any crate can implement `Action`. This is the correct choice for an SDK/extension-first tool. The trade-off vs Nebula's sealed traits: no invariant enforcement at compile time, but easier community contribution.

**7. Concurrency policy as a pluggable trait.** Five concrete policies provided: Default (interrupt), Queue (partially implemented — rejects when busy with explicit comment), Parallel (bounded), Merge (chat-bot style), Reject. The `Queue` policy has a notable honest comment: "Queue policy is not implemented; use interrupt/parallel/reject policy" (`concurrency.rs:113`).

---

## 18. Known limitations / pain points

**No GitHub issues** were found (gh issue list returned empty array). The following limitations come from code inspection and documentation:

1. **Queue concurrency policy unimplemented.** `concurrency.rs:111-115` — the `QueueConcurrencyPolicy` explicitly rejects when busy with a comment noting it is not implemented.

2. **REFACTOR_PLAN.md** (root): Written in Chinese, documents a 5-phase refactor from Reactor/Recipe/Skeleton architecture to the current agent loop + mini-DAG model. Phases 1-2 are complete (removing ~6,400 lines of old code). Phases 3-5 pending.

3. **Temporary `DerivationPolicy` enum.** `REFACTOR_PLAN.md` notes that `action/document/assess.rs`, `action/spreadsheet/assess.rs`, `action/structured/assess.rs` have a local `DerivationPolicy` enum as a "临时方案" (temporary solution) pending Phase 5 cleanup.

4. **No persistent memory.** Long-term memory across sessions is not implemented. In-memory stores are the default.

5. **Token estimation is heuristic.** `chars / 4.0` approximation — inaccurate for non-Latin scripts and code.

6. **No PII protection in prompt logging.** When `log_full_prompts = true`, user intent and full conversation history are logged to stdout/tracing.

7. **No production store backend.** SQLite and PostgreSQL are referenced in config but the production implementations are not visible in the published crates (likely in `plugins/` per `CLAUDE.md`: "Concrete infra implementations (S3/PG/Redis) go in `plugins/`").

---

## 19. Bus factor / sustainability

- **Maintainer count:** 1 (sizzlecar — single author inferred from commit history)
- **Recent commit cadence:** Active — 20 commits visible in `--depth 50` clone, including MR merges for CLI UX, Telegram adapter, and crates.io publishing
- **Latest tag:** v0.2.0 (recent)
- **Issues:** 0 open issues (either very stable or no community yet)
- **Published to crates.io:** yes (orchestral, orchestral-cli at 0.2.0)
- **Language:** Rust 1.91.0

Bus factor = 1. Very new project (v0.2.0). No community infrastructure visible. Solo developer with REFACTOR_PLAN indicating active architectural iteration.

---

## 20. Final scorecard vs Nebula

| Axis | orchestral approach | Nebula approach | Verdict | Borrow? |
|------|---------------------|-----------------|---------|---------|
| A1 Workspace | 6 crates, 3 layers (core/runtime/facade + 2 apps + examples) | 26 crates, layered (error/resilience/credential/resource/action/engine/tenant/eventbus etc.) | Nebula deeper — orchestral appropriate for its scope | no |
| A2 DAG | Runtime `HashMap<String, DagNode>`, no compile-time checks, dynamic modification, replanning via agent loop | TypeDAG L1-L4 (static generics → TypeId → refinement predicates → petgraph) | Nebula deeper for safety; orchestral simpler and LLM-friendly | no |
| A3 Action | Open `trait Action: Send + Sync`, type-erased Value I/O, no assoc types, no versioning, no derive macro | 5 sealed kinds, assoc Input/Output/Error types, versioning via type identity, nebula-derive | Nebula deeper (compile-time); orchestral simpler (open trait, good for SDK/LLM use) | refine — open trait idea worth considering for SDK ergonomics |
| A11 Plugin BUILD | MCP server (subprocess/HTTP) + SKILL.md docs, auto-discovered from .mcp.json/.claude/skills/ | WASM planned, plugin-v2 spec, Plugin Fund | Different decomposition — orchestral ships MCP today; Nebula targets WASM sandbox (unimplemented) | yes — MCP bridge pattern worth borrowing as Nebula plugin transport |
| A11 Plugin EXEC | OS process sandbox (stdio/HTTP), JSON-RPC, no capability security | WASM sandbox + capability security (planned) | Different goals — orchestral practical, Nebula more secure (when implemented) | no |
| A18 Errors | `thiserror` per-module enums (PlanError, LlmError, ConfigError, etc.), no unified error type | nebula-error crate, ErrorClass enum (transient/permanent/cancelled), used by ErrorClassifier | Nebula deeper — unified error classification enables resilience engine | no |
| A21 AI/LLM | Full built-in: multi-provider LlmClient trait, LlmPlanner, agent loop (6 iters), MCP bridge, skill system, streaming, tool calling, agent step kind, token budget estimation | No first-class LLM; bet = generic actions + plugin LLM client | Competitor deeper — orchestral is an AI-first runtime; Nebula intentionally defers LLM to plugin layer | maybe — LlmClient trait shape and action-selector pre-filter are worth examining for Nebula's Surge integration |

---

## Negative grep evidence summary

The following patterns were searched and **not found** in `*.rs` source files:

| Pattern | Result |
|---------|--------|
| `credential` | 0 results in `*.rs` |
| `zeroize` | 0 results |
| `secrecy` | 0 results |
| `oauth` (lowercase) | 0 results in `*.rs` |
| `wasm` / `wasmtime` / `wasmer` | 0 results in `*.rs` core/runtime |
| `libloading` | 0 results |
| `pgvector` / `qdrant` / `pinecone` / `weaviate` | 0 results |
| `embedding` | 0 results in core/runtime |
| `circuit_breaker` / `CircuitBreaker` | 0 results |
| `bulkhead` | 0 results |
| `opentelemetry` | 0 results |
| `prometheus` / `metrics` crate | 0 results in workspace deps |
