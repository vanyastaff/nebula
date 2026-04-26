# Tianshu-rs (天枢) — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/Desicool/Tianshu-rs
- **Stars:** 2 (very early-stage)
- **Forks:** 0
- **Open issues:** 2 (both are feature proposals)
- **Last activity:** 2026-04 (active development, 10+ commits in recent history)
- **License:** Apache-2.0
- **Governance:** Solo developer (Desicool), MIT-style license, no commercial model stated
- **Crates published:** Yes — `tianshu`, `tianshu-postgres`, `tianshu-llm-openai`, `tianshu-observe`, `tianshu-dashboard` on crates.io
- **Positioning:** AI-first workflow engine targeting "LangGraph alternative in Rust"
- **Documentation:** Bilingual — full English README + full Simplified Chinese README (README.zh.md). All source code and comments are in English. This appears to be a Chinese-origin project targeting international audiences, with explicit Chinese cloud integration (ByteDance Doubao endpoint shown in documentation).

---

## 1. Concept positioning [A1, A13, A20]

**Author's own description:** "A checkpoint-safe, coroutine-like workflow engine for building AI agent orchestration systems in Rust." (README.md)

**Observed description:** A sequential-code workflow engine where `ctx.step()` is the primitive — each call is automatically checkpointed and recovered on restart. Targeted explicitly at AI agent workloads, with a first-class LLM layer built in.

**Comparison with Nebula:** Tianshu and Nebula overlap at the orchestration layer but diverge fundamentally in model:
- Nebula uses a **DAG** (TypeDAG L1-L4): nodes are typed at compile time, edges express data flow, graphs are verified.
- Tianshu uses a **coroutine metaphor**: sequential async code, no DAG definition, no edge wiring. Workflows are functions that call `ctx.step()`.
- Tianshu has **built-in LLM as a first-class concern** from v0.1. Nebula defers LLM to actions + plugin.
- Tianshu is explicitly positioning against LangGraph (Python); Nebula positions against n8n + Temporal + Airflow.

---

## 2. Workspace structure [A1]

**Workspace members (6 total):**

| Crate | Published as | Role |
|-------|-------------|------|
| `crates/workflow_engine` | `tianshu` | Core: scheduler, traits, stores, LLM traits, tools, retry |
| `crates/workflow_engine_postgres` | `tianshu-postgres` | PostgreSQL adapters for CaseStore, StateStore, SessionStore |
| `crates/workflow_engine_llm_openai` | `tianshu-llm-openai` | OpenAI-compatible LLM adapter (also Ollama, Doubao, Azure) |
| `crates/workflow_engine_observe` | `tianshu-observe` | Observer implementations: InMemory, Jsonl, Composite, dataset export |
| `crates/workflow_engine_dashboard` | `tianshu-dashboard` | Axum-based HTTP dashboard + web UI |
| `examples/approval_workflow` | (local) | Full example with polling, stage transitions, PostgreSQL option |

**Workspace root file:** `Cargo.toml` — uses `resolver = "2"`, Edition 2021 (not 2024). Workspace dependencies are centralized.

**Layer separation:** The design is loosely layered — `tianshu` (core) → `tianshu-postgres` / `tianshu-llm-openai` / `tianshu-observe` (addons) — but there is no enforced layer boundary beyond crate dependency direction. The core crate imports no addon crates.

**Feature flags:** No workspace-level feature flags. The approval_workflow example uses a local `postgres` feature flag to optionally pull in tianshu-postgres.

**Comparison with Nebula (A1):** 6 crates vs Nebula's 26 crates. Tianshu has a much flatter workspace. Nebula enforces strict layer boundaries (nebula-error at the base, nebula-resilience, nebula-credential, nebula-resource each before nebula-action). Tianshu puts everything in one core crate and pulls optional adapters as separate crates. Less structured but simpler for a new contributor.

---

## 3. Core abstractions [A3, A17] — DEEP

### A3.1 Trait shape

The primary unit of work is `BaseWorkflow` (`crates/workflow_engine/src/workflow.rs:62`):

```rust
#[async_trait]
pub trait BaseWorkflow: Send + Sync {
    async fn run(&self, ctx: &mut WorkflowContext) -> Result<WorkflowResult>;
    fn route_listener(&self) -> Option<&RouteListener> { None }
    fn is_listening(&self, _ctx: &WorkflowContext) -> bool { false }
    fn on_route_matched(&self, _ctx: &mut WorkflowContext, _payload: Option<JsonValue>) {}
}
```

- **Sealed:** No. The trait is `pub` with no sealing mechanism. Any external crate can implement it.
- **Trait-object compatible:** Yes — `Box<dyn BaseWorkflow>` is used throughout (`registry.rs:12`).
- **Associated types:** None. The I/O model is entirely erased — inputs come from `ctx.case.resource_data: Option<JsonValue>` and outputs are returned via `ctx.finish(type, description)`.
- **GATs / HRTBs / typestate:** None. Tianshu makes no use of GATs, HRTBs, or typestate patterns.
- **Default methods:** Three default methods (route_listener, is_listening, on_route_matched) for the intent-routing subsystem.

### A3.2 I/O shape

- **Input:** `JsonValue` stored in `Case.resource_data` (`case.rs:44`). No type parameter. Fully erased.
- **Output:** A string pair `(finished_type: String, finished_description: String)` passed to `ctx.finish()` (`context.rs:444`). No type parameter.
- **Streaming output:** Not applicable — there's no streaming workflow output concept. Streaming is only for LLM responses (`StreamingLlmProvider` trait, `llm.rs:103`).
- **Side-effects model:** All side effects happen inside the closure passed to `ctx.step()`. There is no effect model or purity enforcement.

### A3.3 Versioning

No versioning mechanism exists. Workflows are registered by string code (`registry.rs:25`) and looked up by that code at runtime. There is no concept of v1/v2 workflow versions, no `#[deprecated]` on trait implementations, and no migration support. The `workflow_code` field in `Case` is a plain string.

### A3.4 Lifecycle hooks

- **Lifecycle:** Only `run()`. No pre/execute/post/cleanup/on-failure hooks.
- **Cancellation:** `lifecycle_state` field on `Case` supports `"pause"` and `"stop"` strings (`case.rs:66-70`), checked in the scheduler before executing. No cancellation token or async cancellation point.
- **Idempotency key:** `step_name` serves as the idempotency key — the checkpoint is keyed by `(case_key, step_name)` (`context.rs:80-84`).

### A3.5 Resource and credential dependencies

None. Workflows declare no dependencies. Resources (DB pools, HTTP clients) are constructed outside the engine and captured by closure. There is no injection mechanism.

### A3.6 Retry/resilience attachment

- Per-step via `ctx.step_with_retry(step_name, &policy, closure)` (`context.rs:337`).
- `RetryPolicy` is a user-constructed struct passed at the call site (`retry.rs:32`).
- No workflow-level policy or global override mechanism.

### A3.7 Authoring DX

No derive macros. No builder. A "hello world" workflow requires implementing `BaseWorkflow` manually — approximately 8 lines:

```rust
struct HelloWorkflow;
#[async_trait]
impl BaseWorkflow for HelloWorkflow {
    async fn run(&self, ctx: &mut WorkflowContext) -> anyhow::Result<WorkflowResult> {
        ctx.finish("success".into(), "done".into()).await?;
        Ok(WorkflowResult::Finished("success".into(), "done".into()))
    }
}
```

### A3.8 Metadata

No display name, description, icon, or category system. The only identifier is the workflow code string registered in `WorkflowRegistry`. No i18n support.

### A3.9 vs Nebula

Nebula has 5 action kinds (Process/Supply/Trigger/Event/Schedule), sealed traits, and associated Input/Output/Error types enforced at compile time. Tianshu has **1 workflow kind** (`BaseWorkflow`) with fully erased I/O. This is a fundamentally simpler abstraction that trades type safety for ergonomics. There is no equivalent of Nebula's TriggerAction (input=Config, output=Event) nor SupplyAction. Tianshu's `StageBase<S>` trait (`stage.rs:41`) provides a sub-pattern for stage machines within a workflow but is not an alternative to action kinds — it's a helper to structure a single workflow's run() body.

---

## 4. DAG / execution graph [A2, A9, A10]

### No DAG

Tianshu has **no graph model**. There is no graph library dependency, no port/edge/node concept, and no compile-time or runtime graph verification. The "graph" is the implicit call tree of `run()` methods.

The execution model is a **tick-based scheduler** (`SchedulerV2`, `engine.rs:162`) that drives a set of `Case` objects. In each tick (4 phases):

1. **Partition** (`engine.rs:219`): split cases into `Running` and `Waiting` by `ExecutionState` enum.
2. **Probe** (`engine.rs:236`): call `wf.run()` on `Waiting` cases to re-evaluate their `PollPredicate` list.
3. **Evaluate** (`engine.rs:321`): call `PollEvaluator` with the `ResourceFetcher` to see which predicates are satisfied.
4. **Execute** (`engine.rs:357`): call `wf.run()` on all ready cases (Sequential or Parallel mode, controlled by `ExecutionMode` enum).

**Concurrency within a tick:** `ExecutionMode::Parallel` uses `tokio::task::JoinSet` (`engine.rs:430`) to spawn one task per ready case. `ExecutionMode::Sequential` runs them in a loop. Default is sequential.

**Comparison with Nebula (A2/A10):** Nebula has TypeDAG (4 levels), static port typing, frontier-based work-stealing scheduler. Tianshu has no DAG and a simple 4-phase tick. Tianshu is simpler but cannot enforce data-flow types at compile time. No frontier scheduler, no work-stealing.

---

## 5. Persistence & recovery [A8, A9]

### Storage layer

Three traits (`store.rs`):

- `CaseStore` (`store.rs:22`): upsert/get/list workflow case records.
- `StateStore` (`store.rs:87`): save/get checkpoint data keyed by `(case_key, step_name)`.
- `SessionStore` (`store.rs:62`): upsert/get/delete session records.

Reference implementations: `InMemoryCaseStore`, `InMemoryStateStore`, `InMemorySessionStore` (all in `store.rs`). PostgreSQL implementations in `tianshu-postgres` using `deadpool-postgres` pool + raw `tokio_postgres` queries. No sqlx, no ORM.

**Schema:** 4 SQL tables (`migrations/`):
- `wf_cases` — workflow case lifecycle (`001_create_wf_cases.sql`)
- `wf_state` — per-step checkpoint data as TEXT strings (`002_create_wf_state.sql`)
- `wf_sessions` — session metadata as JSONB (`003_create_wf_sessions.sql`)
- `wf_session_state` — cross-case session-scoped variables (`004_create_wf_session_state.sql`)

### Recovery model

Each `ctx.step()` call (`context.rs:156`) first checks `get_checkpoint(step_name)`. If a non-null value exists, it deserializes and returns it without executing the closure. This is the full recovery mechanism: on process restart, the workflow's `run()` replays from the top, and every completed step is served from the checkpoint store. Incomplete steps (crashed mid-execution) are re-executed. This is the "coroutine replay" model described in the README.

**In-memory cache:** `WorkflowContext` has a `HashMap<String, JsonValue>` cache (`context.rs:33`) to avoid repeated store reads within a tick.

**No append-only log, no frontier-based checkpoint:** Unlike Nebula's append-only execution log + frontier scheduler, Tianshu uses last-write-wins upserts. Recovery is via replay, not log replay. No generation tracking, no checkpoint compaction.

**Comparison with Nebula (A8/A9):** Nebula uses sqlx + PgPool + Pg*Repo pattern, frontier-based scheduler with checkpoint recovery, append-only execution log. Tianshu uses deadpool + raw tokio_postgres queries, replay-based recovery, upsert model. Nebula's model is more production-hardened; Tianshu's model is simpler to reason about.

---

## 6. Credentials / secrets [A4] — DEEP

### A4.1 Existence

**No separate credential layer exists.** The only credential handling in the codebase is `api_key: String` stored in the `OpenAiProvider` struct (`workflow_engine_llm_openai/src/lib.rs:121`).

Grep evidence (searched all `.rs` files):
- `grep -r "credential" --include="*.rs"` — zero matches in non-test, non-doc code
- `grep -r "zeroize\|secrecy\|Secret<\|Zeroize" --include="*.rs"` — zero matches
- `grep -r "vault\|keyring\|encrypt\|AES\|ChaCha" --include="*.rs"` — zero matches

### A4.2 Storage

None. API keys are passed by the user as plain `String` at construction time. There is no at-rest encryption, no vault backend, no OS keychain integration, no key rotation mechanism.

### A4.3 In-memory protection

None. `api_key: String` in `OpenAiProvider` has no zeroize, no `secrecy::Secret<T>` wrapping, and no lifetime limits.

### A4.4 Lifecycle

None. No CRUD for credentials, no refresh, no revocation, no expiry detection.

### A4.5 OAuth2/OIDC

None. No OAuth2, no OIDC, no PKCE.

### A4.6-A4.9

None of Nebula's credential capabilities are present: no State/Material split, no LiveCredential, no watch() for blue-green refresh, no OAuth2Protocol adapter, no DynAdapter type erasure, no phantom types per credential kind.

**Design decision (inferred):** Tianshu's README shows `OpenAiProvider::new("sk-...", "gpt-4o")` — the API key is passed inline at construction time, expected to come from env vars managed by the user. This is a deliberate simplification, not an oversight — the project's focus is on the workflow execution primitive, not secrets management.

---

## 7. Resource management [A5] — DEEP

### A5.1 Existence

**No separate resource abstraction exists.** Each workflow manages its own resources by closure capture. No `Resource` trait, no pool management, no lifecycle coordination by the engine.

Grep evidence:
- `grep -r "Resource\b" --include="*.rs"` in source files returns only `resource_type`, `resource_id`, `resource_data`, `ResourceFetcher` — all unrelated to Nebula's Resource concept.
- `grep -r "pool\b" --include="*.rs"` matches only `deadpool_postgres::Pool` in the postgres adapter — no engine-level pool abstraction.

### A5.2-A5.8

None of Nebula's resource lifecycle exists: no scope levels (Global/Workflow/Execution/Action), no `ReloadOutcome` enum, no hot-reload, no generation tracking, no `on_credential_refresh` hook, no acquire timeout, no backpressure.

The `ResourceFetcher` trait (`poll.rs:18`) is named similarly but is a different concept: it is a poll-predicate evaluator that fetches external event data to determine if a waiting workflow should wake up. It is not a resource pool abstraction.

**Comparison with Nebula (A5):** Nebula has 4 scope levels, ReloadOutcome, generation tracking, on_credential_refresh — Tianshu has none of these. This is a large gap for enterprise production use cases requiring resource lifecycle management.

---

## 8. Resilience [A6, A18]

### Retry

`RetryPolicy` struct (`retry.rs:32`) with fields:
- `max_attempts: u32`
- `base_delay: Duration`
- `max_delay: Duration`
- `backoff_factor: f64`
- `classify: Arc<dyn Fn(&anyhow::Error) -> ErrorClass + Send + Sync>` — user-provided error classifier

`ErrorClass` enum (`retry.rs:14`): `Transient`, `PromptTooLong`, `MaxOutputTokens`, `ProviderOverloaded`, `Fatal`.

`with_retry()` free function (`retry.rs:54`): exponential backoff loop, stops on `Fatal` or exhausted attempts.

`ctx.step_with_retry()` (`context.rs:337`): checkpoint-aware retry at step level.

### Resilient LLM Provider

`ResilientLlmProvider` (`llm_resilient.rs`) wraps any `LlmProvider` with:
- Per-attempt fallback providers (for `ProviderOverloaded`)
- `max_tokens` doubling on `MaxOutputTokens` error (up to 128k ceiling)
- `on_prompt_too_long` handler callback for `PromptTooLong` error

### Missing resilience patterns

No circuit breaker, no bulkhead, no timeout wrapper, no hedging. Tianshu's resilience layer is focused entirely on LLM provider failures. Nebula has all five patterns as a separate `nebula-resilience` crate.

### Error handling [A18]

`anyhow::Error` is used throughout (`anyhow = "1.0"`). No custom error type, no error classification hierarchy comparable to Nebula's `ErrorClass` in `nebula-error`. The `ErrorClass` in `retry.rs` is specifically for LLM error classification only.

**Comparison with Nebula (A6/A18):** Nebula has retry + CB + bulkhead + timeout + hedging with a unified `ErrorClassifier`. Tianshu has retry + basic fallback for LLM calls only. Error handling is `anyhow` throughout vs Nebula's typed `nebula-error`. Tianshu simpler but much less complete for non-LLM resilience needs.

---

## 9. Expression / data routing [A7]

**No expression engine exists.** Tianshu has no DSL, no expression language, no `$nodes.foo.result.email` syntax, no JSONPath evaluation, no sandbox. Data flows through `JsonValue` passed via closures — routing is entirely imperative Rust code.

Grep evidence:
- `grep -r "expression\|jsonpath\|jmespath\|cel\|script\|eval" --include="*.rs"` — zero matches

The `IntentRouterV2` (`poll.rs:76`) provides LLM-based intent routing for message-driven workflows, but this is a semantic router (LLM classifies user messages), not a data-routing expression engine.

---

## 10. Plugin / extension system [A11] — DEEP

### 10.A Plugin BUILD process

**No plugin system exists.** Tianshu has no plugin format, no manifest, no registry, no SDK.

Grep evidence:
- `grep -r "plugin\|wasm\|wasmtime\|wasmer\|dylib\|libloading" --include="*.rs"` — zero matches
- `grep -r "manifest\|registry.*plugin" --include="*.rs"` — zero matches

Extension points in Tianshu are traits: `BaseWorkflow`, `Tool`, `LlmProvider`, `StateStore`, `CaseStore`, `Observer` — all implemented statically at compile time and registered at startup. There is no dynamic loading.

**A11.1-A11.4:** Not applicable — no plugin system.

### 10.B Plugin EXECUTION sandbox

**Not applicable.** No WASM runtime, no subprocess isolation, no capability system.

**A11.5-A11.9:** Not applicable.

**Comparison with Nebula (A11):** Nebula has a WASM sandbox specification (planned), capability-based security, and a Plugin Fund commercial model. Tianshu has no plugin layer — it is a library where extensions are Rust trait implementations linked at compile time. This makes Tianshu's extension story much simpler but means users cannot ship plugins separately from the main binary.

---

## 11. Trigger / event model [A12] — DEEP

### A12.1 Trigger types

Tianshu has a **polling model** only. There is no webhook receiver, no cron scheduler, no external message broker integration. The `PollPredicate` struct (`workflow.rs:20`) declares what external resource a workflow is waiting for:

```rust
pub struct PollPredicate {
    pub resource_type: String,
    pub resource_id: String,
    pub step_name: String,
    pub intent_desc: Option<String>,
}
```

The `ResourceFetcher` trait (`poll.rs:18`) is the hook for the user to implement polling logic. The scheduler calls this on every tick for waiting workflows.

### A12.2 Webhook

None. No webhook receiver, no URL allocation, no HMAC verification, no registration mechanism.

### A12.3 Schedule

None. No cron scheduler, no one-shot scheduling, no interval triggers.

### A12.4 External events

No direct broker integration (Kafka, RabbitMQ, NATS, Redis Streams). External events are received via the `ResourceFetcher` polling abstraction — the user's implementation of `ResourceFetcher` can poll any backend.

### A12.5 Reactive vs polling

**Polling only.** The scheduler re-runs `wf.run()` on waiting workflows every tick and consults the `ResourceFetcher`. There is no reactive/push mechanism. The tick interval is controlled by the user.

### A12.6 Trigger → workflow dispatch

The `IntentRouterV2` (`poll.rs:76`) provides LLM-based message routing: given a user message and a set of `(step_name, intent_description)` pairs, it calls the LLM to classify the message and returns the matched step name. This is used for message-driven workflows (e.g., CRM chatbots). Fan-out is not explicitly supported — one message routes to one workflow.

### A12.7 Trigger as Action

No. Tianshu has no trigger-as-action concept. The `WorkflowResult::Waiting(polls)` return value is the suspension mechanism, and `PollPredicate` declares what the workflow waits for. This is a different decomposition from Nebula's TriggerAction.

### A12.8 vs Nebula

Nebula's Source → Event → TriggerAction 2-stage model normalizes raw inbound data (HTTP req / Kafka msg / cron tick) into typed Events, then routes to TriggerAction. Tianshu has no 2-stage model — the raw event data flows through `ResourceFetcher.fetch()` returning `JsonValue`. There is no normalization layer. Tianshu's polling is simpler to implement but lacks the type safety and backpressure of Nebula's trigger model.

---

## 12. Multi-tenancy [A14]

No multi-tenancy layer. No RBAC, no SSO, no SCIM, no tenant isolation, no schema-per-tenant, no RLS.

Grep evidence:
- `grep -r "tenant\|rbac\|sso\|scim\|schema.*tenant\|rls" --include="*.rs"` — zero matches (excluding LLM message role strings)

The `session_id` field provides logical grouping of cases but has no isolation semantics. There is no authorization model.

---

## 13. Observability [A15]

### Tracing

Uses the `tracing` crate. All scheduler phases, workflow state changes, checkpoint saves/restores, and DB operations emit structured `info!` / `debug!` / `warn!` / `error!` events with field values. Example from `engine.rs:411`: `info!("Workflow finished: case_key='{}', type='{}', desc='{}'", ...)`.

### Observer pattern

`Observer` trait (`observe.rs:153`) with 7 callbacks:
- `on_step()` — per-step execution (cached or fresh)
- `on_workflow_complete()` — workflow finish
- `on_llm_call()` — every LLM call
- `on_tool_call()` — every tool call in a tool loop
- `on_retry()` — retry attempts
- `on_probe()` — waiting-workflow probe in scheduler tick
- `flush()` — drain buffered writes

Three implementations in `tianshu-observe`: `InMemoryObserver`, `JsonlObserver` (async JSONL append), `CompositeObserver` (fan-out to multiple observers).

### What's missing (stated in README)

The README explicitly documents gaps: no step-level timing spans (only total tick duration), no Prometheus/metrics counters, no OpenTelemetry/distributed tracing.

**Comparison with Nebula (A15):** Nebula has OpenTelemetry integration with one trace per execution, per-action metrics. Tianshu has tracing crate structured logs + custom Observer events but no OTel, no distributed trace context propagation, no metrics. Tianshu's observability is functional but not production-grade for distributed monitoring.

---

## 14. API surface [A16]

No network API is provided by the core library. The `tianshu-dashboard` crate (`workflow_engine_dashboard/`) provides an Axum-based HTTP API and web UI for inspecting workflow state. This is an optional addon, not the primary interface.

The primary API is the Rust library API: `SchedulerV2::tick()`, `WorkflowRegistry::register()`, `WorkflowContext::step()`, etc.

No REST API for workflow management (create/pause/cancel), no GraphQL, no gRPC, no OpenAPI spec.

---

## 15. Testing infrastructure [A19]

- **75 `#[test]` functions** across 15 test files (inline `#[cfg(test)]` modules and `tests/` directories).
- **Test style:** unit tests are inline in each module; integration tests live in `crates/*/tests/`.
- **No public testing utilities.** There is no `tianshu-testing` crate or equivalent to Nebula's `nebula-testing`.
- **Postgres tests:** Tagged with `#[ignore]` and require `DATABASE_URL` env var. The `InMemory*` stores make most tests runnable without a database.
- **Test framework:** Standard `tokio::test` + `#[test]` — no `insta` snapshot testing, no `wiremock`, no `mockall`.
- **Coverage:** Good coverage of the core scheduler (`engine.rs` has ~22 tests), context checkpoint logic, store implementations, retry logic, tool loop, and observer events.

---

## 16. AI / LLM integration [A21] — DEEP

### A21.1 Existence

**Built-in — central feature.** LLM integration is the primary value proposition of Tianshu. The core `tianshu` crate includes `LlmProvider`, `StreamingLlmProvider`, `ToolRegistry`, `Tool`, `ManagedConversation`, `ResilientLlmProvider`, and `IntentRouterV2`. An OpenAI-compatible adapter is a separate crate but published alongside core.

### A21.2 Provider abstraction

`LlmProvider` trait (`llm.rs:100`): single method `async fn complete(&self, request: LlmRequest) -> Result<LlmResponse>`. `StreamingLlmProvider` extends it with `async fn stream(&self, request: LlmRequest, tx: mpsc::Sender<LlmStreamEvent>) -> Result<()>` (`llm.rs:112`).

Provider implementations:
- `OpenAiProvider` in `tianshu-llm-openai` — OpenAI API by default, configurable `base_url` for Ollama (`http://localhost:11434/v1`), ByteDance Doubao (`https://ark.cn-beijing.volces.com/api/v3`), Azure OpenAI. This makes it an OpenAI-compatible API adapter, not a multi-vendor trait with separate backends per vendor.
- Anthropic (Claude) adapter is on the roadmap but not yet implemented (README roadmap item: `tianshu-llm-anthropic`).
- Local models via Ollama using the OpenAI-compatible `/v1/chat/completions` endpoint.

**BYOL endpoint:** Yes — `OpenAiProvider::builder(...).base_url("...")` pattern (`lib.rs:90`).

### A21.3 Prompt management

`LlmRequest` (`llm.rs:71`) has `system_prompt: Option<String>`, `messages: Vec<LlmMessage>`. The message format follows OpenAI's role-based model (system/user/assistant/tool). No templating engine, no versioned prompt storage, no prompt-checked-into-workflow-definition mechanism.

### A21.4 Structured output

No JSON mode, no JSON Schema enforcement, no function-calling schema validation, no re-prompting on validation failure. Tool calls use `parameters_schema: JsonValue` in `LlmTool` (`llm.rs:19`) but there is no schema validation of inputs or outputs.

### A21.5 Tool calling

`Tool` trait (`tool.rs:21`):
```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn safety(&self) -> ToolSafety;
    fn parameters_schema(&self) -> JsonValue;
    async fn execute(&self, input: JsonValue) -> Result<String>;
}
```

`ToolSafety` enum (`tool.rs:13`): `ReadOnly` (safe to run in parallel) or `Exclusive` (must run alone). `ToolRegistry::execute_with_concurrency()` (`tool.rs:100`) partitions tool calls by safety level and runs `ReadOnly` calls in parallel using `JoinSet` + `Semaphore` with configurable `max_concurrency`.

`run_tool_loop()` (`tool_loop.rs:44`) drives the multi-turn tool loop: LLM call → tool execution → append tool results → repeat until no tool calls or `max_rounds` exceeded.

`ctx.tool_step(step_name, llm, tools, request, config)` (`context.rs:369`): checkpoint-aware tool loop step.

### A21.6 Streaming

`StreamingLlmProvider` trait (`llm.rs:112`): events delivered via `mpsc::Sender<LlmStreamEvent>`. `LlmStreamEvent` variants: `TextDelta(String)`, `ToolUse(ToolCall)`, `Usage(LlmUsage)`, `Done(String)`, `Error(String)`. The `OpenAiProvider` in `tianshu-llm-openai` implements SSE streaming (`sse.rs`, `streaming.rs`). Streaming is first-class, not a bolt-on.

No backpressure mechanism — the channel is unbounded (`tokio::sync::mpsc`).

### A21.7 Multi-agent

**Sub-workflow spawning** provides multi-agent patterns. `ctx.spawn_child(config)` and `ctx.spawn_children(configs)` (`context.rs:294`, `context.rs:306`) create child `Case` objects in `Running` state. `ctx.await_children(handles)` (`context.rs:320`) returns `ChildrenResult::Pending(n)` or `ChildrenResult::AllDone(statuses)`. The parent suspends as `Waiting` while children execute.

**Issue #2** (open feature request) describes planned multi-agent examples: ReAct, plan-and-execute, multi-agent swarm, conversation agent.

**Issue #8** (open feature request) proposes recursive depth limiting (`max_depth`) for sub-process spawning to prevent runaway agent recursion — a safety concern that is unresolved.

Hand-off: informal via `Case.resource_data` passing data between parent and children. No typed hand-off protocol.

Shared memory: `ctx.set_session_state()` / `ctx.get_session_state()` (`context.rs:207`, `context.rs:181`) for cross-case variables within a session. **Explicitly documented as last-write-wins with no locking** (`context.rs:200`: "No engine-level locking is provided").

### A21.8 RAG / vector

No built-in embeddings, no vector store integration. No retrieval-as-workflow-node.

Grep evidence:
- `grep -r "embedding\|vector.*store\|qdrant\|pinecone\|pgvector\|weaviate" --include="*.rs"` — zero matches

### A21.9 Memory / context

`ManagedConversation` (`compact.rs:131`) provides per-workflow context compaction:
- Tracks `Vec<LlmMessage>` with a `TokenCounter`
- Auto-compacts when `estimated_tokens > max_input_tokens * compact_threshold`
- Two compaction strategies: `TruncationCompaction` (drop oldest preserving N recent, `compact.rs:22`) and `LlmSummaryCompaction` (LLM call to summarize dropped prefix, `compact.rs:51`)
- `ContextConfig` defaults: 128k input tokens, 85% compact threshold (`token.rs:27`)

No session-level or user-level long-term memory. No external memory store.

### A21.10 Cost / tokens

`LlmUsage` struct (`llm.rs:86`) tracks `prompt_tokens` and `completion_tokens`. No cost calculation, no budget circuit breakers, no per-tenant attribution.

### A21.11 Observability

`LlmCallRecord` (`observe.rs:94`) captures per-call: `model`, `request`, `response_content`, `usage`, `duration_ms`, `error`. `ToolCallRecord` (`observe.rs:76`) captures per-tool-call: `tool_name`, `input`, `output`, `is_error`, `duration_ms`.

`ObservedLlmProvider` wrapper (`observe.rs:280`) for transparent instrumentation — wrap any `LlmProvider` to automatically record all calls.

`tianshu-observe` provides `JsonlObserver` for RLHF dataset building (README explicitly mentions this use case) and a `step_dataset()` / `llm_dataset()` / `workflow_dataset()` function in `dataset.rs` for extracting fine-tuning data.

No PII filtering, no content masking, no eval hooks (LLM-as-judge).

### A21.12 Safety

No content filtering, no prompt injection mitigations, no output validation.

### A21.13 vs Nebula + Surge

Tianshu has **first-class LLM integration** — it is the central value proposition, not an add-on. It implements a working, usable LLM layer (provider trait, tool calling, streaming, context compaction, resilient provider) in v0.1. Nebula has no LLM abstraction (strategic bet: AI = generic actions + plugin LLM client). Tianshu validates that an integrated LLM layer in a Rust workflow engine is practical and functional. Key gaps vs an enterprise standard: no structured output validation, no RAG, no cost tracking, no safety layer, shallow streaming (no backpressure), race condition in session-scoped shared memory.

---

## 17. Notable design decisions

**1. Coroutine-replay instead of DAG.** The `ctx.step()` + checkpoint replay model is genuinely novel for a Rust workflow engine. It eliminates the graph-wiring ceremony that makes Temporal and n8n harder to onboard to. Trade-off: no compile-time verification of data flow, harder to visualize as a graph, replay executes all prior steps on each tick (for waiting → running transitions this means re-executing the whole workflow up to the wait point).

**2. LLM as a first-class primitive at v0.1.** The decision to ship `LlmProvider`, `Tool`, `ToolRegistry`, `run_tool_loop`, `ManagedConversation`, `ResilientLlmProvider` in the initial release is aggressive. Nebula's opposite bet (defer LLM to plugins) is safer for API stability but slower to deliver value to AI-first users.

**3. Session/Case as primary execution units.** The Session → Case → Step hierarchy maps naturally to conversational/agent workflows (one session = one user conversation, one case = one task being processed). Nebula's Session concept is more generic. Tianshu's is opinionated toward agent/chat use cases.

**4. Bilingual documentation.** The full README in both English and Simplified Chinese with explicit Doubao (ByteDance's LLM) and Azure OpenAI support in the adapter suggests intentional targeting of both Chinese and international markets.

**5. No resource/credential abstraction by design.** The README's quick-start example hardcodes API keys as strings — this is a DX choice to minimize setup friction for small projects. It is not scalable to multi-tenant or enterprise deployments.

**6. ToolSafety and concurrent tool execution.** The `ReadOnly`/`Exclusive` safety classification on tools is a clean primitive for LLM tool orchestration. Running all ReadOnly tools in parallel within a tool loop round is a meaningful latency optimization.

---

## 18. Known limitations / pain points

**Issue #8 (open):** No recursive sub-process depth limiting — a workflow spawning children that spawn children can recurse unboundedly. Proposed fix requires a breaking API change (`spawn_child()` → returns `SpawnResult` enum). URL: https://github.com/Desicool/Tianshu-rs/issues/8

**Issue #2 (open):** No built-in examples for ReAct, plan-and-execute, multi-agent swarm, or conversation agent patterns. These are foundational patterns for the target use case. URL: https://github.com/Desicool/Tianshu-rs/issues/2

**Documented gaps (README "What's not yet implemented"):**
- No step-level timing/duration spans (only total workflow duration)
- No Prometheus/metrics counters
- No OpenTelemetry/distributed tracing integration

**Session-state race condition (documented in code):** `ctx.set_session_state()` (`context.rs:200`) explicitly warns "No engine-level locking is provided." Concurrent cases in the same session writing to the same session variable will silently overwrite each other. This is a correctness hazard in multi-agent workflows.

**Coroutine replay cost:** On each tick, a `Waiting` workflow is probed by executing `wf.run()` and replaying all completed steps (each step hit is served from checkpoint cache but still executed up to the wait point). For workflows with 100+ completed steps, this is O(n) deserialization work per tick.

**No DAG visualization:** The coroutine model makes workflow visualization harder — there is no graph structure to render.

**No deletion from StateStore:** The `StateStore` trait has no per-key delete (`context.rs:145` comment: "Because StateStore has no per-key delete, we save an empty sentinel"). This requires a `JsonValue::Null` sentinel pattern for cleared steps, which is fragile.

---

## 19. Bus factor / sustainability

- **Maintainer count:** 1 (Desicool)
- **Stars:** 2 — extremely early-stage, not yet publicly discovered
- **Commit cadence:** Active — 10+ commits in the most recent period, features being shipped (dashboard, examples, streaming)
- **Issues ratio:** 2 open / 0 closed — all issues are feature proposals, no bug reports yet (likely because no external users yet)
- **Last release:** 0.1.0 published on crates.io — initial release
- **Risk:** Single maintainer, very early, no community yet. High bus factor risk. The project uses a Beads issue tracker and Claude Code agent definitions suggesting developer tooling investment, but the project is pre-adoption.

---

## 20. Final scorecard vs Nebula

| Axis | Tianshu approach | Nebula approach | Who's deeper / simpler / more correct | Borrow? |
|------|-----------------|-----------------|---------------------------------------|---------|
| A1 Workspace | 6 crates, flat, Edition 2021, no feature-flag layers | 26 crates layered, Edition 2024, strict boundary enforcement | Different decomposition. Tianshu simpler to navigate; Nebula more structured for a large team. | no — different goals |
| A2 DAG | No DAG. Sequential coroutine replay model. No graph library. | TypeDAG L1-L4: static generics → TypeId → predicates → petgraph | Different decomposition, neither dominates. Tianshu's model is easier to adopt but loses compile-time data-flow safety. | no — different goals |
| A3 Action | 1 kind (BaseWorkflow), open trait, erased I/O (JsonValue), no versioning, no macros | 5 action kinds, sealed traits, associated Input/Output/Error, derive macros | Nebula deeper — typed I/O and sealed traits catch errors at compile time | no — Nebula's already better |
| A4 Credential | None. api_key: String passed at construction. | State/Material split, LiveCredential, blue-green refresh, OAuth2Protocol | Nebula deeper — Tianshu has no credential layer | no — Nebula's already better |
| A5 Resource | None. Resources captured by closure. | 4 scope levels, ReloadOutcome, generation tracking, on_credential_refresh | Nebula deeper — Tianshu has no resource abstraction | no — Nebula's already better |
| A6 Resilience | Retry + fallback for LLM calls only. No CB, bulkhead, timeout, hedging. | retry/CB/bulkhead/timeout/hedging in nebula-resilience crate | Nebula deeper — Tianshu resilience is LLM-specific only | no — Nebula's already better |
| A7 Expression | None. Imperative Rust code only. | 60+ functions, type inference, sandboxed eval, JSONPath-like syntax | Nebula deeper — Tianshu has no expression engine | no — different goals |
| A8 Storage | deadpool-postgres + raw tokio_postgres. 4 SQL tables. No ORM. | sqlx + PgPool, Pg*Repo pattern, RLS | Different decomposition. Both are pragmatic; Nebula more idiomatic Rust (sqlx). | maybe — sqlx migration |
| A9 Persistence | Upsert-based checkpoint replay. No append-only log. | Frontier-based scheduler, append-only execution log, state reconstruction via replay | Nebula deeper — append-only log gives better auditability | no — Nebula's already better |
| A10 Concurrency | tokio, JoinSet for parallel case execution, Sequential default | tokio, frontier scheduler, !Send action support | Nebula deeper — frontier work-stealing, !Send isolation. Tianshu simpler. | no — Nebula's already better |
| A11 Plugin BUILD | None — trait impls linked at compile time | WASM planned, plugin-v2 spec, Plugin Fund model | Nebula deeper — plugin ecosystem strategy. Tianshu has no plugin model. | no — Nebula's already better |
| A11 Plugin EXEC | None — no runtime loading | WASM sandbox + capability security (planned) | Nebula deeper — safety boundary design | no — Nebula's already better |
| A12 Trigger | Polling only via ResourceFetcher + PollPredicate. No webhook, cron, broker. IntentRouterV2 for LLM-based routing. | TriggerAction Source→Event 2-stage, webhook/cron/kafka | Nebula deeper — typed 2-stage trigger model. Tianshu's polling is simpler to implement. | refine — IntentRouterV2 LLM routing concept is interesting |
| A21 AI/LLM | First-class: LlmProvider, Tool+ToolRegistry, streaming, ManagedConversation, ResilientLlmProvider. OpenAI-compat adapter ships now. Anthropic on roadmap. | No first-class LLM. Strategic bet: AI = generic actions + plugin LLM client. | Competitor deeper — Tianshu proves the LLM-integrated model is viable and ergonomic. Nebula's bet is safer for API stability but slower for AI-first users. | yes — LlmProvider trait shape, ToolSafety ReadOnly/Exclusive pattern, ManagedConversation compaction strategy, ObservedLlmProvider wrapper |

---

**Summary verdict:** Tianshu is an AI-first Rust workflow engine that makes a bold, successful bet on coroutine-replay over DAGs and on LLM-first over plugin-deferred. Its LLM abstractions — particularly `ToolSafety` concurrency classification, `ManagedConversation` context compaction, and `ObservedLlmProvider` — are well-designed and immediately borrowable by Nebula. Its credential, resource, resilience, and trigger stories are either absent or minimal. It is better thought of as a prototype of what an LLM-first workflow engine looks like than as a production competitor to Nebula's full feature set.
