# aofctl (agenticdevops/aof) — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/agenticdevops/aof
- **Stars:** ~800 (GitHub badge shows badge, repo is young)
- **License:** Apache 2.0
- **Latest tag:** `v0.4.0-beta` (at depth-50 clone)
- **Maintainer:** Gourav Shah (`gjs@opsflow.sh`), single primary maintainer
- **Workspace edition:** Rust 2021; `rust-version = "1.75"` (`Cargo.toml` line 8)
- **Version:** `0.4.0-beta`
- **Governance:** Solo open-source; companion closed-source products KubePilot and OpsPilot import AOF crates
- **Activity:** 101 issues as of clone date; PRs active (most recent commit: remove hardcoded Slack tokens); commits daily cadence from mid-2025 to early 2026

---

## 1. Concept positioning [A1, A13, A20]

**Author's own README (line 11):**
> "n8n for Agentic Ops — Build AI agents with Kubernetes-style YAML. No Python required."

**Independent assessment after reading code:**
AOF is a YAML-first, CLI-driven AI agent orchestration framework targeting DevOps/SRE workflows. Its primary differentiator is the `kubectl`-style `aofctl` CLI combined with K8s CRD-style resource definitions (`kind: Agent`, `kind: AgentFleet`, `kind: AgentFlow`, `kind: Trigger`). The AI angle is not decorative — LLM providers, multi-turn tool execution, structured output enforcement, and multi-agent fleet coordination are all first-class features implemented in Rust.

**Comparison with Nebula:**
Nebula is a workflow orchestration engine where AI is explicitly a future concern (bet: AI = generic actions + plugin LLM). AOF inverts this: AI/LLM is the central primitive. AOF lacks Nebula's depth in DAG type-safety, credential lifecycle, resource scoping, and persistence; but AOF ships working multi-provider LLM integration today, where Nebula has nothing equivalent.

---

## 2. Workspace structure [A1]

**Crate count:** 17 workspace members (`Cargo.toml` lines 4-20)

| Crate | Purpose |
|-------|---------|
| `aof-core` | Foundational traits/types: `Agent`, `Model`, `Tool`, `Workflow`, `AgentFlow`, `AgentFleet`, `Trigger`, `Memory`, `AofError` |
| `aof-llm` | LLM provider adapter layer: Anthropic, OpenAI, Google, Groq, Ollama, Bedrock (feature-gated) |
| `aof-runtime` | Execution engine: `AgentExecutor`, `AgentFlowExecutor`, `WorkflowExecutor`, `FleetCoordinator`, resilience, sandbox, credential audit |
| `aof-mcp` | Model Context Protocol client (stdio, SSE, HTTP transports) |
| `aof-memory` | Memory backends: in-memory, file-based |
| `aof-triggers` | Platform trigger implementations: Slack, Telegram, Discord, GitHub, PagerDuty, Opsgenie, HTTP, Schedule |
| `aof-tools` | Built-in tools: shell, kubectl, Grafana, Datadog, AWS, GCP, Azure, Vault, Snyk, OPA, SonarQube, Trivy |
| `aof-skills` | Skill library (pre-built agent behaviors) |
| `aof-coordination` | Multi-agent coordination protocol, event broadcaster, decision log, session state |
| `aof-coordination-protocols` | Consensus algorithms, heartbeat, standup, metrics overhead tracking |
| `aof-conversational` | Template library (pre-built fleet templates), input sanitizer, persona persistence |
| `aof-gateway` | HTTP gateway (REST API exposure of agents/flows/fleets) |
| `aof-personas` | Agent persona management and metrics |
| `aofctl` | CLI binary (`aofctl run agent`, `aofctl run flow`, etc.) |
| `aof-viz` | Visualization utilities |
| `smoke-test-mcp` | MCP smoke tests |
| `test-trigger-server` | Test HTTP server for trigger tests |

**Layer separation:** Partial. `aof-core` is pure traits/types. `aof-runtime` depends on `aof-core`, `aof-coordination`, `aof-llm` implicitly (via config). `aof-triggers` and `aof-tools` are roughly parallel leaves. No strict numerical layering; dependencies form a diamond with `aof-core` at the root.

**Feature flags:** `bedrock` feature flag in `aof-llm` gates AWS Bedrock provider. No other significant feature flags found.

**vs Nebula:** Nebula has 26 crates with explicit numeric layering (L0 error/glue, L1 traits, L2 implementations, L3 engine). AOF has 17 crates with a looser structure and no formal layer contract. Nebula is deeper; AOF is wider in AI/LLM.

---

## 3. Core abstractions [A3, A17] — DEEP

### A3.1 Trait shape

The primary unit of work is the `Agent` trait defined in `crates/aof-core/src/agent.rs:442-464`:

```rust
#[async_trait]
pub trait Agent: Send + Sync {
    async fn execute(&self, ctx: &mut AgentContext) -> AofResult<String>;
    fn metadata(&self) -> &AgentMetadata;
    async fn init(&mut self) -> AofResult<()> { Ok(()) }
    async fn cleanup(&mut self) -> AofResult<()> { Ok(()) }
    fn validate(&self) -> AofResult<()> { Ok(()) }
}
```

**Sealed?** No. The trait is fully open — any crate in the ecosystem can implement it. No `sealed::Sealed` supertrait or private module guard.

**`dyn Agent` compatible?** Yes. `AgentRef = Arc<dyn Agent>` is declared at `crates/aof-core/src/agent.rs:992`. The trait uses `async_trait` macro rather than native async fn in traits (Rust 2021 edition, predates stable AFIT).

**Associated types:** None. The trait has no associated `Input` / `Output` / `Error` / `Config` types. Input is passed via the `AgentContext` struct; output is a plain `String`. This is fundamentally different from Nebula's 5-kind action system where each action kind carries associated `Input`, `Output`, and `Error` types enforced at compile time.

**GAT/HRTB:** None found. The type system is entirely runtime-driven.

**Typestate:** None. No `Validated<T>` or compile-time state distinction.

**Default methods:** `init`, `cleanup`, `validate` all have empty default bodies.

### A3.2 I/O shape

**Input:** `AgentContext` struct (`crates/aof-core/src/agent.rs:468-488`) containing: `input: String`, `messages: Vec<Message>`, `state: HashMap<String, serde_json::Value>`, `tool_results: Vec<ToolResult>`, `metadata: ExecutionMetadata`, `output_schema: Option<OutputSchema>`, `input_schema: Option<InputSchema>`.

All structured intermediate state is `serde_json::Value` — fully type-erased. No compile-time type enforcement.

**Output:** `AofResult<String>` — a plain string. Structured output is requested via `OutputSchemaSpec` in `AgentConfig` which instructs the LLM via prompt injection ("respond with valid JSON matching this schema"). Validation is done at runtime by parsing the returned string against the JSON Schema. There is no compile-time output type.

**Streaming:** Supported via `StreamEvent` enum in `crates/aof-runtime/src/executor/agent_executor.rs:~28-90`. The enum has variants: `TextDelta`, `ToolCallStart`, `ToolCallComplete`, `Thinking`, `IterationStart`, `IterationComplete`, `Done`, `Error`. Streaming is SSE/channel-based using tokio `mpsc`.

**Side-effects model:** Tool calls executed in the `AgentExecutor` tool-use loop. The executor sends a `ModelRequest`, receives a `ModelResponse` with `tool_calls`, executes them via `ToolExecutor`, and feeds results back. Multi-turn feedback loop is the primary execution pattern.

### A3.3 Versioning

No versioning mechanism. Agents are referenced by name only (via `AgentConfig.name`). No `#[deprecated]`, no version field in `AgentMetadata`, no v1/v2 distinctions for agent definitions. The YAML `apiVersion: aof.dev/v1` is a K8s-style resource version but does not control agent behavior versioning.

### A3.4 Lifecycle hooks

`init`, `cleanup`, `validate` are defined in the `Agent` trait with empty defaults. There is no `pre/post/on-failure/cleanup` hook beyond these three. No idempotency key mechanism.

### A3.5 Resource & credential deps

Agents declare tools via `tools: Vec<ToolSpec>` in `AgentConfig` and MCP servers via `mcp_servers: Vec<McpServerConfig>`. There is no resource abstraction layer — each agent creates its own HTTP client or DB pool. Credential dependencies are declared only implicitly: the `StandaloneTriggerConfig` struct holds `bot_token`, `signing_secret`, etc. as plain strings (resolved from env vars at runtime via `expand_env_var()`). No compile-time check that "this agent needs credential Y."

### A3.6 Retry/resilience attachment

`WorkflowSpec` has a global `retry: Option<RetryConfig>` field. Per-step retry is available via `StepConfig.retry`. The `RetryPolicy` in `crates/aof-runtime/src/resilience/retry.rs` implements exponential backoff with jitter. However, the per-agent `Agent` trait has no retry declaration; retry is applied at the workflow/executor layer.

### A3.7 Authoring DX

Agents are defined in YAML. Minimal Rust coding required. A "hello world" agent:

```yaml
apiVersion: aof.dev/v1
kind: Agent
metadata:
  name: k8s-helper
spec:
  model: openai:gpt-4
  instructions: You are a helpful Kubernetes expert.
  tools:
    - shell
```

No derive macro or builder pattern needed. Running `aofctl run agent my-agent.yaml` is the CLI entry point. This is dramatically simpler than implementing a Nebula `ProcessAction` trait with associated types.

### A3.8 Metadata

`AgentMetadata` has `name`, `description`, `version`, `capabilities: Vec<String>`, `extra: HashMap<String, serde_json::Value>`. No i18n support. All runtime, no compile-time metadata baking.

### A3.9 vs Nebula

Nebula has **5 action kinds** (Process / Supply / Trigger / Event / Schedule), each a sealed trait with associated `Input` / `Output` / `Error` types enforced at compile time. AOF has **1 agent kind** (`Agent` trait, open, no associated types). AOF additionally has AgentFlow (DAG of nodes), AgentFleet (multi-agent team), and Workflow (step-based FSM) — these are configuration structs, not separate trait hierarchies. The architecture is fundamentally **runtime-erased** where Nebula is **compile-time typed**.

---

## 4. DAG / execution graph [A2, A9, A10]

**Graph description:** `AgentFlow` defines a DAG via `nodes: Vec<FlowNode>` and `connections: Vec<FlowConnection>` in `crates/aof-core/src/agentflow.rs`. Nodes have types: `Agent`, `Transform`, `Conditional`, `Parallel`, `Join`, `Approval`, `Wait`, `Loop`, `Script`, `Slack` (inline action). Connections are string-keyed `from`/`to` pairs.

**Port typing:** No port typing. Connections are untyped string identifiers. No compile-time check that `from: diagnose.output` matches `to: auto-fix.input` in terms of schema.

**Compile-time checks:** None. The DAG is parsed from YAML at runtime.

**Scheduler model:** Topological sort is done at runtime by the `AgentFlowExecutor`. The ROADMAP notes "Future: Horizontal scaling via Redis/NATS" (#47 open), indicating the current model is single-process, non-distributed.

**Concurrency:** tokio runtime. Parallel branches in AgentFlow are executed with `tokio::task::JoinSet` (seen in `crates/aof-runtime/src/executor/agent_executor.rs`). `AgentFleet` has explicit coordination modes: `Hierarchical`, `Peer`, `Swarm`, `Pipeline`, `Tiered`.

**vs Nebula:** Nebula has a 4-level TypeDAG (L1 generics → L2 TypeId → L3 predicates → L4 petgraph). AOF has a single runtime string-referenced adjacency list. Nebula is radically deeper on graph type-safety; AOF is more approachable for YAML authors.

---

## 5. Persistence & recovery [A8, A9]

**Storage:** `WorkflowSpec` has a `checkpointing: Option<CheckpointConfig>` field (`crates/aof-core/src/workflow.rs`). `CheckpointConfig` references a `CheckpointBackend` enum. However, no PostgreSQL or database dependency appears in the workspace `Cargo.toml`. Memory backends are in-memory or file-based (`crates/aof-memory/src/`). No `sqlx`, no `PgPool`, no `pgpool` found in dependencies.

**Persistence model:** Primarily in-memory or file-backed. The Workflow's checkpoint capability is defined as a configuration struct but actual backend implementations appear to be incomplete (no `sqlx` dependency). The ROADMAP cites "Horizontal scaling via Redis/NATS" as P1 planned, implying current state does not have durable distributed persistence.

**Recovery semantics:** `RecoveryConfig` struct exists in `aof-core/src/workflow.rs`. Beyond the struct definition, runtime recovery implementation was not found in the source at depth-50 clone.

**vs Nebula:** Nebula has frontier-based checkpoint recovery, append-only execution log, and `sqlx + PgPool` for durable storage. AOF has the types but not the implementation; this is a major gap.

---

## 6. Credentials / secrets [A4] — DEEP

### A4.1 Existence

AOF has **no separate credential abstraction layer** in the Nebula sense (no `CredentialOps` trait, no `LiveCredential`). Credential handling is split across:

1. **Trigger-level secrets:** `StandaloneTriggerConfig` holds `bot_token`, `signing_secret`, `api_key` etc. as `Option<String>`, resolved via `expand_env_var()` pattern (`crates/aof-core/src/trigger.rs:362-385`)
2. **Context-level secrets:** `ContextSpec` has `secrets: Vec<SecretRef>` where `SecretRef` holds a reference to an env var or Kubernetes secret
3. **Audit layer:** `CredentialAccessEvent` and `CredentialAccessAnomaly` in `crates/aof-core/src/credential.rs` — an audit/anomaly detection subsystem tracking file-based credential access (kubeconfig, AWS creds, etc.)

**Negative grep — searched `crates/` for Nebula-style patterns:**
- `LiveCredential` — **not found**
- `CredentialOps` — **not found**
- `secrecy::Secret` — **not found**
- `zeroize` — **not found**
- `State.*split` (State/Material split) — **not found**
- `blue.green` in credential context — **not found** (only found in deployment strategy string)

### A4.2 Storage

No at-rest encryption for secrets. Credentials are resolved from environment variables at runtime. No Vault integration as a credential backend (Vault is a tool that agents can call, not a secret store for the framework itself).

### A4.3 In-memory protection

No `zeroize` or `secrecy::Secret<T>`. Plain `String` fields.

### A4.4 Lifecycle

No refresh, revocation, or expiry model. Credentials are read once from env vars.

### A4.5 OAuth2/OIDC

Not implemented at the framework level. OAuth2 tokens are passed in as raw strings via environment variables.

### A4.6 Composition

One credential per platform trigger, no delegation model.

### A4.7 Scope

Credentials are effectively global (env var scope). No per-workspace or per-tenant isolation. Issue #46 (Multi-org support / per-org credentials) is open P1.

### A4.8 Type safety

No phantom types or compile-time credential kind safety. All credentials are `Option<String>`.

### A4.9 vs Nebula

Nebula has: State/Material split (typed state + opaque material), `CredentialOps` trait, `LiveCredential` with `watch()` for blue-green refresh, `OAuth2Protocol` blanket adapter, `DynAdapter` type erasure, `zeroize`, and `secrecy::Secret`. AOF has: none of these. AOF's credential audit subsystem (`CredentialAccessEvent`, `AnomalyDetector`) is a runtime monitoring layer, not a lifecycle management layer. **Different goals — Nebula deeper here.**

---

## 7. Resource management [A5] — DEEP

### A5.1 Existence

No separate resource abstraction. Each tool/agent creates its own HTTP client via `reqwest::Client`. There is no `Resource` trait, no resource scoping system, no pooling layer.

**Negative grep searched for:**
- `ReloadOutcome` — **not found**
- `Resource.*trait\|impl.*Resource` in resource-management sense — **not found**
- `on_credential_refresh` — **not found**
- `generation.*counter\|gen_counter` — **not found**

### A5.2 Scoping

No scope levels. Resources (HTTP clients, parsed configs) are scoped to their creating task's lifetime. The sandbox module (`crates/aof-runtime/src/sandbox/`) provides Linux capability dropping and seccomp profiles for Docker-based tool execution, which is a form of resource isolation, but not a resource lifecycle API.

### A5.3 Lifecycle hooks

No `init` / `shutdown` / `health-check` pattern for resources. The `Agent` trait has `init` and `cleanup` but these are per-agent, not per-shared-resource.

### A5.4 Reload

No hot-reload. Issue #22 (Config hot-reload) is open P2. No `ReloadOutcome` enum.

### A5.5 Sharing

No pooling. `Arc<dyn Model>` (`ModelRef = Arc<dyn Model>` at `crates/aof-core/src/model.rs:206`) allows sharing a model client. MCP executor instances are created per-agent-execution (seen in `crates/aof-runtime/src/executor/agent_executor.rs`).

### A5.6 Credential deps

No formal mechanism. Resources do not declare credential dependencies.

### A5.7 Backpressure

`Bulkhead` in `crates/aof-runtime/src/resilience/bulkhead.rs` provides an agent-slot semaphore (`acquire_agent_slot`, `try_acquire_agent_slot`). This is the only backpressure mechanism found.

### A5.8 vs Nebula

Nebula has 4 scope levels (Global/Workflow/Execution/Action), `ReloadOutcome` enum with generation tracking, `on_credential_refresh` per-resource hook. AOF has none of these. AOF has a `Bulkhead` that Nebula lacks as a named primitive (Nebula uses its resilience crate for similar effects). **Nebula significantly deeper on resource management.**

---

## 8. Resilience [A6, A18]

AOF has a dedicated resilience module at `crates/aof-runtime/src/resilience/` with five files:

| File | Pattern |
|------|---------|
| `retry.rs` | `RetryPolicy`: exponential backoff, jitter, configurable `max_attempts`/`base_delay`/`max_delay` |
| `circuit_breaker.rs` | `CircuitBreaker` with states (Closed/Open/HalfOpen) |
| `bulkhead.rs` | `Bulkhead`: semaphore-based concurrency limiting |
| `supervisor.rs` | `Supervisor`: agent restart / crash recovery |
| `degradation.rs` | `DegradationPolicy`: graceful degradation on failures |

These are separate Rust implementations (not wrapping `tokio-retry` or `failsafe`). The `RetryPolicy` uses `2^attempt` exponential backoff with optional jitter (`crates/aof-runtime/src/resilience/retry.rs:43-57`).

No unified `ErrorClassifier` equivalent to Nebula's transient/permanent distinction. The executor has a private `ErrorCategory` enum (`crates/aof-runtime/src/executor/agent_executor.rs`) with `Retryable`/`NonRetryable` discrimination, but it is not exported.

**vs Nebula:** Both have retry/CB/bulkhead. Nebula adds hedging and a unified public `ErrorClassifier` with `ErrorClass` enum exported from `nebula-error`. AOF's error classification is executor-private. AOF adds a `Supervisor` (agent restart) that Nebula does not currently have. **Comparable breadth, different integration depth.**

---

## 9. Expression / data routing [A7]

No expression engine. There is no `$nodes.foo.result.email` or JSONPath-style DSL. Data flows between flow nodes via `AgentContext.state: HashMap<String, serde_json::Value>`. Conditional routing in `AgentFlow` uses `conditions: Vec<NodeCondition>` where `NodeCondition` is a plain string expression (e.g., `"severity != \"critical\""` from the README). No sandboxed evaluation, no 60+ built-in functions.

**Negative grep:** Searched for `expression\|expr_eval\|jsonpath\|jinja\|template_engine` in `crates/` — found only template strings in `aof-conversational` and no evaluation engine.

**vs Nebula:** Nebula has a full expression engine (60+ functions, type inference, sandboxed eval, `$nodes.foo.result.email` syntax). AOF has inline string conditions that are matched at runtime by simple string comparison. **Nebula significantly deeper.**

---

## 10. Plugin / extension system [A11] — DEEP

### 10.A Plugin BUILD process

**A11.1 Format:** No formal plugin format. Extension is accomplished at two levels: (1) implementing the `Agent` trait in Rust and contributing to the workspace, or (2) using MCP servers as the integration point. MCP servers are external processes started via stdio/SSE/HTTP. There is no `.tar.gz`, OCI, or WASM blob plugin artifact.

**A11.2 Toolchain:** No dedicated plugin SDK, no cross-compilation tooling, no scaffolding CLI. New tools are added as Rust modules in `crates/aof-tools/src/tools/`.

**A11.3 Manifest content:** No plugin manifest. MCP server configs are declared inline in agent YAML (`mcp_servers:` field).

**A11.4 Registry/discovery:** Issue #71 "MCP Server Catalog" is open P0 — this is the planned discovery mechanism but not yet implemented. Currently, users specify MCP server commands directly in YAML.

**Negative grep for `plugin_manifest\|plugin_registry\|plugin_load\|dlopen\|libloading`:** **Not found.**

### 10.B Plugin EXECUTION sandbox

**A11.5 Sandbox type:** Two sandboxing mechanisms:

1. **MCP subprocess:** MCP tools run as external processes (stdio/SSE/HTTP via `aof-mcp` crate). This is the primary extension mechanism. Each MCP server is a separate process; communication via JSON-RPC over stdio or HTTP.

2. **Docker sandbox:** `crates/aof-runtime/src/sandbox/` implements Docker container isolation for shell tool execution. Uses `bollard` crate (Docker API). Capability config (`capabilities.rs`) drops Linux capabilities; `seccomp.rs` implements seccomp profiles per tool type (shell, kubectl, file ops, network, K8s API).

**A11.6 Trust boundary:** Docker sandbox provides memory/CPU limits, read-only root filesystem, dropped capabilities. MCP subprocess has no explicit capability-based trust model — the subprocess inherits the environment.

**A11.7 Host↔plugin calls:** MCP: JSON-RPC 2.0 over stdio/SSE/HTTP. Tool definitions are exposed as JSON Schema. Marshaling is `serde_json`. No async crossing issue (separate processes).

**A11.8 Lifecycle:** MCP servers are started at agent init and stopped at cleanup. No hot-reload. The `aof-mcp` crate has `auto_reconnect: true` flag for MCP server configs.

**A11.9 vs Nebula:** Nebula targets WASM + capability security + Plugin Fund commercial model. AOF uses MCP subprocess + Docker container isolation. Both are "trust boundary via process isolation" approaches, but via different mechanisms. AOF's MCP-first approach aligns with the emerging MCP ecosystem standard. Nebula's WASM target is more hermetic (memory isolation, no process boundary needed) but is planned, not shipped. **Different decomposition; AOF has working external tooling now, Nebula has deeper planned architecture.**

---

## 11. Trigger / event model [A12] — DEEP

### A12.1 Trigger types

Defined in `crates/aof-core/src/trigger.rs:124-146` as `StandaloneTriggerType` enum:
- `Slack`, `Telegram`, `Discord`, `WhatsApp` (chat platforms)
- `HTTP` (generic webhook)
- `Schedule` (cron-based)
- `PagerDuty`, `GitHub`, `Jira`, `Opsgenie` (DevOps integrations)
- `Manual` (CLI invocation)

Implementations in `crates/aof-triggers/src/platforms/` (one module per platform).

### A12.2 Webhook

HTTP trigger (`StandaloneTriggerType::HTTP`) uses `path`, `methods`, `required_headers`, `webhook_secret` fields. URL is user-configurable via `path` field and bound on a configurable `port`. HMAC verification via `webhook_secret` field. No idempotency key mechanism.

### A12.3 Schedule

Cron via `StandaloneTriggerType::Schedule` with `cron: Option<String>` and `timezone: Option<String>` fields. The cron library used was not directly verified in sources read, but a standard Rust cron crate is expected. No DST handling or missed schedule recovery documented in source.

### A12.4 External event

Kafka and SQS listed as P3 planned in ROADMAP. Current external event triggers are chat platforms (polling or webhook-based) and HTTP. No direct Kafka/NATS consumer groups. Issue #47 (Horizontal scaling / Redis/NATS) is P1 but refers to scaling, not trigger ingestion.

### A12.5 Reactive vs polling

Mixed. Chat platforms (Slack, Telegram, Discord) use platform SDKs/webhooks. Schedule uses cron polling. HTTP trigger is reactive (webhook server). Default model is reactive where platform allows.

### A12.6 Trigger→workflow dispatch

`CommandBinding` struct in `crates/aof-core/src/trigger.rs:99-117` maps slash commands to `agent`, `fleet`, or `flow`. A `default_agent` field handles @mentions. 1:1 or 1:N dispatch (one trigger can route to multiple agents via commands map). Trigger metadata passed as message context.

### A12.7 Trigger as Action

Triggers are **not** a kind of `Agent`. `Trigger` is a separate Kubernetes-style resource (`kind: Trigger`). The `Trigger` struct and the `Agent` trait are distinct top-level constructs. A trigger's lifecycle is: start (register handlers) → listen (forever) → dispatch to agent/fleet/flow on event. The `FlowBinding` CRD (`crates/aof-core/src/binding.rs`) decouples trigger routing from flow definitions.

### A12.8 vs Nebula

Nebula: `Source` → `Event` → `TriggerAction` 2-stage with `Input = Config` (registration) / `Output = Event` (typed payload); backpressure via tokio bounded channels. AOF: `Trigger` CRD → `CommandBinding` → agent/fleet/flow 1-stage routing; no typed event payload (`StandaloneTriggerConfig` is a flat config struct, not a typed domain event). **Nebula's 2-stage model is architecturally cleaner and type-safer; AOF's routing table approach is simpler to configure but loses type information.**

---

## 12. Multi-tenancy [A14]

No formal multi-tenancy. The ROADMAP lists "Multi-org support — Per-org credentials" as issue #46 (P1 open). Current approach to multi-tenant routing: multiple `Trigger` CRDs each with channel/user filters (e.g., `multi-tenant/slack-prod-k8s-bot.yaml` routes to production agents, `multi-tenant/slack-staging-k8s-bot.yaml` routes to staging). This is namespace simulation via YAML routing, not schema/RLS/database isolation.

No RBAC layer. No SSO. No SCIM.

**vs Nebula:** Nebula has `nebula-tenant` crate with three isolation modes (schema/RLS/database), RBAC, planned SSO/SCIM. AOF has routing-based channel filtering. **Nebula significantly deeper.**

---

## 13. Observability [A15]

Observability is implemented via two mechanisms:

1. **Structured logging:** `tracing` and `tracing-subscriber` are workspace dependencies (`Cargo.toml`). The `aofctl` CLI initializes a `tracing_subscriber` with JSON format and env-filter (`crates/aofctl/src/commands/run.rs:1005-1015`). `prometheus` crate is a workspace dependency for metrics.

2. **Activity events:** `ActivityEvent` / `ActivityLogger` / `TokenCount` types in `crates/aof-core/src/activity.rs` emit per-step events (LlmCall, ToolExecuting, MemoryOp, etc.). These feed the TUI live display.

3. **Coordination events:** `CoordinationEvent` / `DecisionLogEntry` for multi-agent sessions.

No OpenTelemetry exporter found. No per-execution trace spanning (no `TraceId` or `SpanId` propagation found in sources read). Prometheus metrics are declared but not a full OTel integration.

**vs Nebula:** Nebula uses OpenTelemetry with structured tracing per execution (one trace = one workflow run). AOF uses `tracing` + `prometheus` but not OTel exporters. **Nebula deeper on observability integration.**

---

## 14. API surface [A16]

`aof-gateway` crate exposes agents/flows/fleets over HTTP REST. `aof-gateway` uses `hyper 1.0` + `tower` + `tower-http`. No OpenAPI spec generation found. No GraphQL. No gRPC. REST paths are handler-defined.

`aofctl` CLI provides the primary programmatic interface: `aofctl run agent`, `aofctl run flow`, `aofctl run fleet`, `aofctl run workflow`.

---

## 15. Testing infrastructure [A19]

No dedicated testing crate equivalent to Nebula's `nebula-testing`. Tests are co-located in each crate (`tests/` subdirectories and `#[cfg(test)]` modules).

`crates/aofctl/tests/TEST_STRATEGY.md` documents the test approach. Test libraries: standard `#[tokio::test]`, no `insta` snapshots, no `wiremock` found in workspace dependencies. `tempfile` is present for temp dir tests.

`crates/aof-personas/tests/` has extensive integration/e2e tests (e.g., `integration_e2e_test.rs` with 900+ lines, `metrics_computation_test.rs`).

---

## 16. AI / LLM integration [A21] — DEEP

### A21.1 Existence

**LLM integration is the central feature of AOF.** It is not a plugin or optional module — the `aof-llm` crate is a core workspace member and `Model` trait is re-exported from `aof-core`. Every agent requires a `model` field pointing to an LLM. This is the most LLM-native framework in the competitor cohort.

### A21.2 Provider abstraction

**Multi-provider.** The `ModelProvider` enum in `crates/aof-core/src/model.rs:37-48`:

```rust
pub enum ModelProvider {
    Anthropic, OpenAI, Google, Groq, Bedrock, Azure, Ollama, Custom,
}
```

Providers in `crates/aof-llm/src/provider/`:
- `anthropic.rs` — Anthropic Claude (direct API)
- `openai.rs` — OpenAI (also used for Groq and Ollama via endpoint override, `crates/aof-llm/src/provider.rs:23-57`)
- `google.rs` — Google Gemini
- `bedrock.rs` — AWS Bedrock (feature-gated: `#[cfg(feature = "bedrock")]`)

**BYOL endpoint:** Yes. `ModelConfig.endpoint: Option<String>` allows custom base URL. This enables any OpenAI-compatible server (Groq uses `https://api.groq.com/openai/v1`; Ollama uses `http://localhost:11434/v1`).

**Local models:** Ollama supported natively. `llama.cpp`, `candle`, `mistral.rs` not directly supported but would work via OpenAI-compatible endpoint.

**Provider trait shape:** `LlmProvider` trait in `crates/aof-llm/src/provider.rs:11-13`:
```rust
pub trait LlmProvider {
    fn create(config: ModelConfig) -> AofResult<Box<dyn Model>>;
}
```
Factory pattern returning `Box<dyn Model>`. The `Model` trait is the runtime abstraction.

### A21.3 Prompt management

**System/user/assistant structure:** Fully supported. `ModelRequest` has `system: Option<String>` and `messages: Vec<RequestMessage>`. `MessageRole` enum: `User`, `Assistant`, `System`, `Tool` (`crates/aof-core/src/model.rs:140-147`).

**Instructions alias:** `AgentConfig` accepts both `system_prompt` and `instructions` as aliases (`crates/aof-core/src/agent.rs:919`).

**Templating:** No dedicated template engine. System prompts are static strings in YAML. The `aof-conversational` crate has template library for pre-built fleet system prompts but no runtime variable interpolation.

**Few-shot:** Not explicitly supported. Users can add few-shot examples to `system_prompt` text.

**Versioning:** No prompt versioning. Prompts are inline strings in YAML.

**Prompts in workflow definition:** Yes — prompts are part of the YAML agent definition and version-controlled alongside the spec.

### A21.4 Structured output

Fully implemented. `OutputSchemaSpec` in `crates/aof-core/src/agent.rs:16-121` provides JSON Schema-based output validation:

- `validation_mode`: `strict` (default), `lenient`, `coerce`
- `on_validation_error`: `fail` (default), `retry`, `passthrough`
- `max_retries: Option<u32>` for retry-on-validation-fail

`OutputSchemaSpec.to_instructions()` generates the LLM prompt injection: "You MUST respond with valid JSON matching this schema..." (`crates/aof-core/src/agent.rs:114-121`).

The approach is **prompt-injection structured output** (not native JSON mode / function calling for all providers). Native tool calling is also supported for providers that support it (via `ModelRequest.tools`).

### A21.5 Tool calling

**Tool calling is the primary execution mechanism.** The `AgentExecutor` implements a multi-turn tool-use loop: call model → if `stop_reason == ToolUse` → execute tools → feed results back → repeat until `EndTurn` or max iterations.

Tool definition format: `ToolDefinition { name: String, description: String, parameters: serde_json::Value }` (JSON Schema for parameters). Multi-tools per call supported (model can request multiple tool calls; `ModelResponse.tool_calls: Vec<ToolCall>`).

**Execution sandbox:** Tool execution is in-process (via `ToolExecutor` trait) or via Docker container (shell/kubectl via bollard) or via MCP subprocess. No WASM sandbox for tools.

**Feedback loop:** Multi-turn, implemented in `AgentExecutor`. Tools are called, results fed back as `MessageRole::Tool` messages, model continues.

**Parallel exec:** `JoinSet` used for parallel tool execution when multiple tool calls are requested simultaneously.

### A21.6 Streaming

SSE-style streaming implemented. `Model.generate_stream()` returns `Pin<Box<dyn Stream<Item = AofResult<StreamChunk>> + Send>>` (`crates/aof-core/src/model.rs:21-24`). `StreamChunk` enum: `ContentDelta { delta: String }`, `ToolCall { tool_call }`, `Done { usage, stop_reason }`.

The `AgentExecutor` exposes `StreamEvent` via an `mpsc` channel for real-time UI updates. The TUI in `aofctl` consumes these events.

**Backpressure:** `mpsc` bounded channel provides implicit backpressure. No explicit `Semaphore` for streaming.

### A21.7 Multi-agent

**First-class multi-agent support via `AgentFleet`.** Coordination modes in `crates/aof-core/src/fleet.rs:10`:
- `Hierarchical` — manager agent coordinates workers
- `Peer` — all agents coordinate as equals (consensus)
- `Swarm` — dynamic self-organizing
- `Pipeline` — sequential handoff
- `Tiered` — tier-based parallel execution (for multi-model RCA)

`ConsensusConfig` and `ConsensusAlgorithm` types are present for the Peer mode. Issue #77 noted that peer mode incorrectly applies consensus when agent outputs are complementary rather than competing.

**Hand-off:** `FleetCoordinator` manages task distribution. `AgentCoordination` event stream via `aof-coordination` crate's `EventBroadcaster`.

**Shared memory:** `SharedMemoryConfig` in `FleetSpec.shared` with `SharedMemoryType` variants.

**Termination conditions:** `FleetSpec` has `max_iterations` safety limit.

### A21.8 RAG/vector

No native vector store integration. No embeddings API call found. The `error_tracker.rs` in `aof-core` is described as "Local Error Knowledge Base (RAG system)" but uses simple keyword matching, not actual embeddings. A comment in `aof-coordination/src/decision_log.rs` says "Future: Replace with embeddings-based semantic search." The `aof-coordination/src/persistence.rs` mentions "vector of session IDs" (Rust `Vec`, not a vector store).

**Negative grep for `qdrant\|pgvector\|pinecone\|weaviate\|embedding\|embed_text`:** Only found the `error_tracker.rs` RAG comment and `fleet/consensus.rs` "In production, could use embeddings" comment. **No vector store integration present.**

### A21.9 Memory/context

**Per-execution memory via `aof-memory` crate.** Memory backends:
- `InMemory` — `Vec<MemoryEntry>` (session-scoped)
- `File` — JSON file persistence (cross-execution session memory)
- SQLite and Redis listed in ROADMAP

`AgentConfig.max_context_messages: usize` (default 10) limits conversation history sent to LLM — a simple sliding window truncation. No summarization strategy.

`MemorySpec` in agent YAML supports: `"in_memory"`, `"file:./path.json"`, or structured `{type: File, config: {path: ..., max_messages: 50}}` (`crates/aof-core/src/agent.rs:150-224`).

**Long-term memory:** File backend provides cross-session persistence. No semantic retrieval.

### A21.10 Cost/tokens

`AgentContext.metadata: ExecutionMetadata` tracks `input_tokens` and `output_tokens` per execution. `Usage { input_tokens, output_tokens }` is returned by all providers in `ModelResponse`. `ContextSpec.limits.max_cost_per_day: Option<f64>` field exists (`crates/aof-core/src/context.rs:251`) for daily cost cap. `ActivityDetails` includes `TokenCount`.

No per-provider cost calculation or per-tenant attribution. No circuit breaker for budget exhaustion. The `max_cost_per_day` field appears to be a config field without a runtime enforcement implementation visible in the sources read.

### A21.11 Observability

`ActivityEvent` system tracks every LLM call (`LlmCall`, `LlmWaiting`, `LlmResponse` activity types). Token counts are included in `ActivityDetails`. Tool calls are tracked (`ToolCallStart`, `ToolCallComplete` in `StreamEvent`). `CoordinationEvent` and `DecisionLogEntry` for multi-agent sessions.

No prompt+response logging with PII-safe filtering. No LLM-as-judge eval hooks. Basic per-call tracking present.

### A21.12 Safety

**Prompt injection mitigation:** `crates/aof-conversational/src/sanitize.rs` implements `sanitize_user_input()` with regex-based injection pattern detection (e.g., "ignore previous instructions", "act as", "bypass safety rules"). Maximum input length of 5000 characters.

**Risk classification:** `RiskLevel` enum (Low/Medium/High/Critical) and `AnomalyAction` enum for credential access monitoring. Shell tool risk levels are assessed per operation.

**Content filtering:** `StopReason::ContentFilter` variant handled in Bedrock and Google providers. No pre-call content filter applied by the framework.

**Output validation:** JSON Schema validation on structured outputs. `on_validation_error: "retry"` for re-prompting on schema mismatch.

### A21.13 vs Nebula + Surge

Nebula has no first-class LLM abstraction (bet: AI = generic actions + LLM plugin). Surge is a separate agent orchestrator on ACP. AOF makes LLM the central primitive with working multi-provider support, multi-turn tool execution, fleet coordination, and streaming — all shipping today.

**AOF is first-class and working.** Nebula is betting on a clean architecture play (generic actions = LLM actions). AOF's architecture is less type-safe but delivers end-to-end working AI agent orchestration. The "over-coupled" concern from the project hint is partially warranted: the `AgentExecutor` couples LLM I/O, tool execution, memory management, and streaming in a single 1000+ line file, which will resist refactoring. But it works. Surge+Nebula's decoupled approach will be cleaner but ships nothing today.

---

## 17. Notable design decisions

**1. YAML-first over code-first.** All agent behavior is declared in YAML with K8s-style CRDs. This is a deliberate DX choice that eliminates the barrier for DevOps/SRE engineers who are not Rust developers. Trade-off: YAML has no compile-time type checking; configuration errors surface at runtime.

**2. MCP as the extension boundary.** Rather than implementing a WASM plugin system, AOF uses the MCP protocol (JSON-RPC over stdio/SSE/HTTP) for all tool extensions. This aligns with the emerging Claude/Anthropic ecosystem standard and allows reuse of the growing MCP server ecosystem. Trade-off: subprocess overhead, no memory isolation.

**3. Agent trait over action kinds.** A single open `Agent` trait rather than Nebula's 5 sealed action kinds. Simpler to explain and implement, but loses compile-time guarantees about action composition, credential dependencies, and I/O types.

**4. LLM-central architecture.** Every agent has exactly one LLM. Multi-agent coordination is built by composing LLM-backed agents. This means every workflow node incurs LLM API costs and latency. No "pure logic" actions that run without a model. Trade-off: simple mental model, expensive for high-throughput automation.

**5. Runtime over compile-time.** All type checking (tool schemas, output schemas, agent connectivity) is runtime JSON Schema validation. Provides YAML ergonomics but loses compile-time safety guarantees that Nebula prioritizes.

---

## 18. Known limitations / pain points

Based on GitHub issues (total ~101 issues, mostly closed):

1. **No horizontal scaling** — Single process, no distributed coordination. Issue [#47](https://github.com/agenticdevops/aof/issues/47) P1 open.
2. **No multi-org credentials** — Issue [#46](https://github.com/agenticdevops/aof/issues/46) P1 open. Single credential namespace.
3. **Peer mode consensus over-applies** — Issue [#77](https://github.com/agenticdevops/aof/issues/77) closed. Peer fleet applies consensus even when agent outputs are complementary not competing.
4. **YAML parsing edge cases** — Issue [#84](https://github.com/agenticdevops/aof/issues/84) (memory field type mismatch), [#89](https://github.com/agenticdevops/aof/issues/89) (non-Agent YAML files break daemon), [#95](https://github.com/agenticdevops/aof/issues/95) (library:// URI path resolution). Complex YAML deserialization with untagged enum unions is fragile.
5. **No config hot-reload** — Issue [#22](https://github.com/agenticdevops/aof/issues/22) P2 open.
6. **No MCP server catalog/discovery** — Issue [#71](https://github.com/agenticdevops/aof/issues/71) P0 open.
7. **Hardcoded secrets in code** — PR #101 fixed hardcoded Slack tokens in `aof-gateway`. Indicates the credential handling model is immature.
8. **AgentExecutor monolith** — Based on source structure, all LLM + tool + memory logic is in `executor/agent_executor.rs`. This single file handles streaming, multi-turn loops, schema validation, and error recovery — a coupling concern that will limit testability as the project grows.

---

## 19. Bus factor / sustainability

- **Maintainer count:** 1 primary (Gourav Shah, `gjs@opsflow.sh`). The CLAUDE.md lists the project as part of an "OpsFlow ecosystem" with companion closed-source products (KubePilot, OpsPilot), providing commercial incentive.
- **Commit cadence:** Active. Commits from mid-2025 through early 2026. 20 commits in the last 50 visible at depth-50 clone.
- **Issues ratio:** 101 total issues; approximately 80% closed — high throughput but also indicates rapid pace with quality debt.
- **Last release:** `v0.4.0-beta` — still in beta phase.
- **Bus factor:** High risk (1 primary maintainer). Commercial backing via closed-source products partially mitigates.

---

## 20. Final scorecard vs Nebula

| Axis | AOF (aofctl) approach | Nebula approach | Verdict | Borrow? |
|------|-----------------------|-----------------|---------|---------|
| **A1 Workspace** | 17 crates, loose layering, edition 2021, rust-version 1.75 | 26 crates, strict numeric layers, edition 2024, pinned 1.95 | Nebula more mature structurally | no |
| **A2 DAG** | Runtime string adjacency list in AgentFlow; no port typing; no compile-time checks | TypeDAG L1-L4: generics→TypeId→predicates→petgraph | Nebula radically deeper | no |
| **A3 Action** | 1 open `Agent` trait, no assoc types, type-erased I/O (`String`/`HashMap<String, Value>`) | 5 sealed action kinds, assoc Input/Output/Error, derive macros | Nebula deeper on type safety; AOF simpler to author | refine |
| **A4 Credential** | Env-var strings, no lifecycle, audit anomaly detection only | State/Material split, LiveCredential, blue-green refresh, OAuth2 | Nebula significantly deeper | no |
| **A5 Resource** | No resource abstraction; `Bulkhead` semaphore only | 4 scope levels, ReloadOutcome, generation tracking | Nebula significantly deeper | no |
| **A6 Resilience** | retry/CB/bulkhead/supervisor/degradation in `aof-runtime/resilience/` | retry/CB/bulkhead/timeout/hedging + unified ErrorClassifier | Comparable breadth; AOF adds Supervisor | refine (adopt Supervisor pattern) |
| **A7 Expression** | No DSL; inline string conditions only | 60+ funcs, type inference, sandbox, `$nodes.foo.result.email` | Nebula significantly deeper | no |
| **A18 Errors** | `AofError` enum via `thiserror`, 20+ variants, no error class taxonomy | `nebula-error` + `ErrorClass` enum (transient/permanent/etc.) | Different decomposition; AOF has more domain variants (sandbox, docker, lock); Nebula has classification | refine |
| **A11 Plugin BUILD** | MCP servers (external processes) as extension model; no plugin manifest | WASM planned, plugin-v2 spec | Different decomposition — AOF ships, Nebula plans | maybe |
| **A11 Plugin EXEC** | Docker container sandbox (bollard) + MCP subprocess JSON-RPC | WASM sandbox + capability security (planned) | AOF ships Docker isolation today; Nebula's WASM is planned | maybe |
| **A21 AI/LLM** | First-class, central feature: multi-provider (Anthropic/OpenAI/Google/Groq/Ollama/Bedrock), tool calling, streaming, multi-agent fleet, structured output, safety/injection mitigation | None first-class; bet on generic actions + LLM plugin | AOF first-class and working; Nebula has cleaner architecture bet | yes — validate multi-provider abstraction shape for Nebula's future LLM plugin |

---

## Key Nebula insights

1. **AOF's `OutputSchemaSpec` + retry-on-validation pattern** is a pragmatic approach to structured LLM output that Nebula could adopt when building its LLM plugin. The prompt-injection + JSON Schema validation loop is simple and provider-agnostic.

2. **AOF's `Supervisor` pattern** in `aof-runtime/resilience/supervisor.rs` for agent crash recovery is absent from Nebula's resilience crate. Worth evaluating for Nebula's long-running action execution.

3. **AOF's MCP-first extension model** represents a real-world validation that "subprocess + JSON-RPC" is sufficient for tool isolation in devops workflows. Nebula's WASM target is more hermetic but MCP would be cheaper to ship faster.

4. **AOF's fleet coordination modes** (Hierarchical/Peer/Swarm/Pipeline/Tiered) map cleanly to Surge's agent orchestration concerns. The `ConsensusConfig` / `ConsensusAlgorithm` types are worth studying for Surge's ACP design.

5. **AOF confirms the YAMLification anti-pattern**: YAML-first with no compile-time type safety leads to runtime surprises (issues #84, #89, #95). Nebula's type-safe DAG is the correct long-term choice despite higher authoring friction.
