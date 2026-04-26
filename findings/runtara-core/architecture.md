# runtara-core — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/runtarahq/runtara
- **Stars/forks:** Not indexed by crates.io; public GitHub repo with early-public activity (4 issues total, first tagged release v1.0.21 through current v3.0.0).
- **Last activity:** April 2026 (active; v3.0.0 rename PR merged April 2026).
- **License:** AGPL-3.0-or-later. Commercial licensing contact: hello@syncmyorders.com (SyncMyOrders Sp. z o.o. — Polish startup).
- **Governance:** Single maintainer (volodymyrrudyi / Volodymyr Rudyi). No Plugin Fund equivalent.
- **Toolchain:** Rust 1.90.0 pinned (`rust-toolchain.toml`), edition 2024. WASM target: `wasm32-wasip2`.
- **Version:** `workspace.package.version = "3.0.0"` (`Cargo.toml:36`).

---

## 1. Concept positioning [A1, A13, A20]

**Author's one-sentence (README):**
> "Runtara is a Rust workspace for building and running durable workflows."

**Mine (after reading code):**
Runtara is a compile-to-WASM durable workflow engine: JSON-described workflows are compiled ahead-of-time into WASM binaries (via `runtara-workflows`), executed in wasmtime sandboxes by `runtara-environment`, and made crash-safe by a checkpoint/signal/sleep persistence engine in `runtara-core`. The `runtara-server` layer adds connections, OAuth2, multi-tenancy (single-process, one tenant per instance), MCP, and an AI-agent step type with built-in tool-calling loop.

**Comparison with Nebula:**
Both are Rust workflow engines targeting WASM execution; both have a persistence+checkpoint layer and planned multi-runner support. Key difference: Nebula compiles at build time via Rust traits and derive macros; Runtara compiles at runtime (server-side) from a JSON DSL into a Rust source file and then invokes rustc. Nebula has a richer type-safety story (sealed traits, GATs, typestate); Runtara trades compile-time safety for author ergonomics (visual JSON editor) and runtime AOT compilation.

---

## 2. Workspace structure [A1]

**17 crates** under `crates/`:

| Layer | Crates |
|-------|--------|
| Foundation | `runtara-core` (persistence), `runtara-sdk` + `runtara-sdk-macros` (instance-side API + `#[resilient]` macro) |
| Compiler | `runtara-dsl` (DSL types), `runtara-workflows` (JSON→Rust→WASM compiler), `runtara-agent-macro`, `runtara-agents`, `runtara-workflow-stdlib` |
| Runtime | `runtara-environment` (runner management), `runtara-management-sdk` (control-plane SDK) |
| Server | `runtara-server` (HTTP + MCP API + workers), `runtara-connections`, `runtara-object-store` |
| AI | `runtara-ai` (LLM completion abstraction) |
| Utilities | `runtara-http`, `runtara-text-parser`, `runtara-test-harness` |

Feature flags are present but limited: `runtara-core` has `server` (HTTP transport), `runtara-agents` has `native` / `wasi` / `wasm-js` / `integrations`, `runtara-sdk` has `embedded` / `http` / `wasi` / `wasm-js`, `runtara-server` has `embed-ui`.

No umbrella re-export crate. No `nebula-error`-style unified error crate. Layer separation is enforced by crate ownership only, not sealed traits.

**Comparison with Nebula:** Nebula has 26 crates vs 17 here. Nebula uses sealed traits across crate boundaries; Runtara uses `inventory`-based dynamic registration. Nebula has layered error taxonomy in `nebula-error`; Runtara uses `thiserror` per-crate with no cross-crate error classification.

---

## 3. Core abstractions [A3, A17] — DEEP

### A3.1 — Trait shape

There is no sealed `Action` / `Node` trait. The unit of work is the **capability**: a plain synchronous function annotated with `#[capability]` from `runtara-agent-macro`.

```rust
// crates/runtara-agents/src/agents/compression.rs:346
#[capability(
    module = "compression",
    display_name = "Create Archive",
    description = "Create an archive from one or more files",
    errors(
        permanent("ARCHIVE_NO_FILES", "At least one file is required..."),
    )
)]
pub fn create_archive(input: CreateArchiveInput) -> Result<FileData, AgentError> { ... }
```

There is no trait to implement. The macro auto-registers the function into an `inventory` registry keyed by `(agent_id, capability_id)`. Dispatch is:

```rust
// crates/runtara-agents/src/registry.rs:17
pub fn execute_capability(agent_id: &str, capability_id: &str, step_inputs: Value) -> Result<Value, String> {
    runtara_dsl::agent_meta::execute_capability(agent_id, capability_id, step_inputs)
}
```

No trait object (`dyn`). No associated types for `Input/Output/Error` at the trait level. Input and Output are plain structs with `#[derive(CapabilityInput)]` / `#[derive(CapabilityOutput)]` proc macros that register field metadata for the DSL schema generator.

The closest thing to a "step type" trait is the DSL `Step` enum (`crates/runtara-dsl/src/schema_types.rs:274`):

```rust
#[serde(tag = "stepType")]
pub enum Step {
    Finish(FinishStep),
    Agent(AgentStep),
    Conditional(ConditionalStep),
    Split(SplitStep),
    Switch(SwitchStep),
    EmbedWorkflow(EmbedWorkflowStep),
    While(WhileStep),
    Log(LogStep),
    Error(ErrorStep),
    Filter(FilterStep),
    GroupBy(GroupByStep),
    Delay(DelayStep),
    WaitForSignal(WaitForSignalStep),
    AiAgent(AiAgentStep),        // ← first-class AI step
}
```

**14 step variants** total. The `AiAgent` step is a first-class enum member — not a plugin.

### A3.2 — I/O shape

Capability inputs are deserialized from `serde_json::Value` at dispatch time via the `inventory`-registered executor. Inputs and outputs are plain Rust structs with `serde::Deserialize` / `serde::Serialize`. No streaming output. Side effects happen synchronously inside the capability function (no async).

The `AgentStep` in the DSL declares `input_mapping: Option<InputMapping>` where `InputMapping = HashMap<String, MappingValue>` and `MappingValue` is an enum (`reference` / `immediate` / `composite` / `template`). At compile time, the workflow compiler (`runtara-workflows`) code-generates Rust that resolves these mappings against the execution context before calling the capability.

### A3.3 — Versioning

No explicit action versioning system. Capabilities are identified by `(agent_id, capability_id)` string pair with no version suffix. The DSL has `DSL_VERSION: &str = "3.0.0"` (`crates/runtara-dsl/src/schema_types.rs:18`) for schema versioning, not action versioning. The CHANGELOG records a breaking v3.0.0 rename of `Scenario` → `Workflow` and `StartScenario` → `EmbedWorkflow` with no backward shims (every path, table, and enum variant changed atomically).

### A3.4 — Lifecycle hooks

No pre/execute/post/cleanup hooks. Capability functions are single-call synchronous functions. The `#[resilient]` SDK macro (`crates/runtara-sdk-macros/src/lib.rs:91`) wraps capability call sites (in generated code) with retry + checkpoint + durable-sleep. The macro is a code-generation concern, not a lifecycle trait.

Cancellation: the SDK polls for cancel/pause signals at checkpoint calls. The `checkpoint()` return value exposes `should_cancel()` / `should_pause()` (`crates/runtara-sdk/src/lib.rs` API). This is reactive, not a hook.

No idempotency key field. The checkpoint ID serves as idempotency marker.

### A3.5 — Resource and credential deps

An `AgentStep` declares an optional `connection_id: Option<String>` (`crates/runtara-dsl/src/schema_types.rs:395`). The generated workflow code fetches connection parameters at runtime via the connection service API. There is no compile-time connection type checking. The capability function receives a `RawConnection` or connection parameters JSON; it does not declare "I need connection type X" in its signature.

### A3.6 — Retry/resilience attachment

Per-capability (per-step) configuration: `AgentStep` has `max_retries: Option<u32>` (default 3 for normal, 5 for rate-limited) and `retry_delay: Option<u64>` (`crates/runtara-dsl/src/schema_types.rs:402-412`). Step timeout: `timeout: Option<u64>`. The `#[resilient]` macro handles exponential backoff + `AUTO_RETRY_ON_429`. No circuit breaker, no bulkhead, no hedging.

### A3.7 — Authoring DX

The author writes a plain Rust function in their agent crate with `#[capability]` and `#[derive(CapabilityInput)]` / `#[derive(CapabilityOutput)]` on input/output structs. Steps are wired in JSON using `agentId` and `capabilityId`. No derive-generated step builder for users. The `runtara-workflows` compiler handles all the boilerplate from the JSON. For built-in agents the "hello world" capability is ~20 lines.

### A3.8 — Metadata

Display name and description are declared in the `#[capability]` macro attributes and collected via `inventory` into `CapabilityMeta` structs (defined in `runtara-dsl::agent_meta`). No i18n. Metadata is runtime-queryable (`get_all_capabilities()`, `get_all_step_types()`). Icons: not present in source.

### A3.9 — vs Nebula

Nebula has **5 sealed action kinds** (ProcessAction, SupplyAction, TriggerAction, EventAction, ScheduleAction) with associated types and versioning. Runtara has **14 DSL step variants** (none are user-extensible without modifying the DSL crate) plus an open-ended agent/capability system that IS user-extensible (write a function, annotate with `#[capability]`, link into stdlib). The extensibility model is entirely different: Nebula's extension point is implementing `ProcessAction`; Runtara's is writing a capability function and registering it. Neither has compile-time I/O type checking between steps.

**Type safety verdict:** Runtara's step I/O is fully type-erased at the workflow level (all step outputs are `serde_json::Value`). Nebula's is also type-erased at the DAG graph level for dynamic nodes, but has sealed trait enforcement for action kinds.

---

## 4. DAG / execution graph [A2, A9, A10]

### Graph model

The execution graph (`ExecutionGraph`, `crates/runtara-dsl/src/schema_types.rs:73`) is a JSON flat-map of step IDs to `Step` variants plus an ordered `Vec<ExecutionPlanEdge>`. There is no compile-time port typing. Edges carry an optional `condition: Option<ConditionExpression>` and `priority: Option<i32>` for conditional routing. Fan-out (parallel execution) is modeled by multiple unlabeled edges from the same step.

There is **no petgraph or DAG library**. The graph is a flat HashMap + Vec; validation in `runtara-workflows/src/validation.rs` (6430 lines) performs cycle detection and reachability analysis at compile time but uses simple DFS, not a graph library.

**Compile-time vs runtime checking:** Graph structural validity is checked when the workflow is compiled (server-side). At execution time, the compiled WASM binary executes a linear sequence of function calls generated by the compiler — there is no runtime graph interpreter.

### Concurrency

`Split` step (`crates/runtara-dsl/src/schema_types.rs:451`) iterates over an array. Parallelism is configured via `SplitConfig`. Inside the WASM process the execution is single-threaded (synchronous). The management plane runs concurrent instances as separate processes (WASM modules in separate wasmtime invocations).

**Comparison with Nebula:** Nebula has TypeDAG with 4 correctness levels (generics, TypeId, predicates, petgraph). Runtara has no type-safe DAG; validation is JSON-level at compile time with no static typing of step I/O ports.

---

## 5. Persistence and recovery [A8, A9]

### Storage

Three storage layers:
1. **runtara-core**: PostgreSQL or SQLite via sqlx (`crates/runtara-core/src/persistence/mod.rs`). Tables: `instances`, `checkpoints`, `events`, `signals`, `step_summaries`, `error_history`. Migrations in `crates/runtara-core/migrations/postgresql/` and `migrations/sqlite/`.
2. **runtara-server**: PostgreSQL for workflows, executions, triggers, connections, metrics. Migrations in `crates/runtara-server/migrations/`. Separate `OBJECT_MODEL_DATABASE_URL` for the object-store database.
3. **Valkey/Redis**: Required for checkpoint storage during workflow execution at the `runtara-server` layer (`VALKEY_HOST` env var). `CHECKPOINT_TTL_HOURS` defaults to 48h.

### Checkpoint model

Workflows call `sdk.checkpoint(id, state_bytes)` at each significant step. The `Persistence` trait (`crates/runtara-core/src/persistence/mod.rs:416`) provides `save_checkpoint` / `load_checkpoint`. On resume, the SDK reads the checkpoint and skips to the next un-executed step. The `#[resilient]` macro wraps capability calls: on fresh execution it calls the capability and saves a checkpoint; on resume it reads the existing checkpoint result and returns it without re-executing.

```
// Persistence trait (crates/runtara-core/src/persistence/mod.rs:416)
pub trait Persistence: Send + Sync {
    async fn save_checkpoint(&self, ...) -> Result<(), CoreError>;
    async fn load_checkpoint(&self, ...) -> Result<Option<CheckpointRecord>, CoreError>;
    async fn register_instance(&self, ...) -> Result<(), CoreError>;
    async fn set_instance_sleep(&self, ...) -> Result<(), CoreError>;
    // ... ~30 methods total
}
```

### Saga/compensation

`AgentStep` supports `compensation: Option<CompensationConfig>` (`crates/runtara-dsl/src/schema_types.rs:413`). The `CheckpointRecord` has `is_compensatable`, `compensation_step_id`, `compensation_state` fields. `crates/runtara-core/src/compensation.rs` implements the saga rollback logic.

**Comparison with Nebula:** Nebula uses a frontier-based scheduler with append-only execution log and state reconstruction by replay. Runtara uses direct checkpoint save/load without replay semantics — simpler but not event-sourced. Runtara adds saga compensation which Nebula does not have yet.

---

## 6. Credentials and secrets [A4] — DEEP

### A4.1 — Existence

Yes, dedicated credential layer in `runtara-connections`. Connections are the credential primitive: named, typed (by `integration_id`), and stored with encrypted parameters.

### A4.2 — Storage

At-rest encryption: **AES-256-GCM** with Zeroizing key (`crates/runtara-connections/src/crypto/aes_gcm.rs:1`). The cipher accepts a base64-encoded 32-byte key (`CONNECTIONS_ENCRYPTION_KEY` env var via `crypto/factory.rs`). Each encrypt call generates a fresh 96-bit OsRng nonce. Output is a JSON envelope: `{v, alg, kid, nonce, ct}`. Backend: PostgreSQL via `ConnectionRepository`. No external vault integration.

### A4.3 — In-memory protection

Key is held as `Zeroizing<Vec<u8>>` (`crates/runtara-connections/src/crypto/aes_gcm.rs:18`) — zeroed on drop. No `secrecy::Secret<T>` wrapper. Plaintext connection parameters are decrypted on demand and not pinned after use (no explicit lifetime limits).

### A4.4 — Lifecycle

Full CRUD via `ConnectionService` (`crates/runtara-connections/src/service/connections.rs`). Refresh model: OAuth2 refresh tokens are stored and the `OAuthService` (`crates/runtara-connections/src/service/oauth.rs`) handles token exchange and storage. No background refresh loop; tokens are fetched/refreshed on demand via `provider_auth.rs`. Token caching in Redis/dashmap via `token_cache.rs`. Revocation: soft delete via `valid_until` field or deletion.

### A4.5 — OAuth2/OIDC

Authorization code flow: `OAuthService::generate_authorization_url` + `OAuthService::handle_callback` (`crates/runtara-connections/src/service/oauth.rs`). State token: cryptographically random, stored in DB, consumed atomically on callback. PKCE: not present in source. Multi-provider: yes (each integration type has an `OAuthConfig` in `agent_meta`). Client credentials: yes, used for Shopify, HubSpot, etc. Refresh: stored `refresh_token` re-exchanged on expiry (`provider_auth.rs:286`).

### A4.6 — Composition

One `connection_id` per `AgentStep`. No multi-credential per step. No delegation or SSO patterns at the action level.

### A4.7 — Scope

Per-tenant (single-tenant process; `tenant_id` column in every connection row). Per-workflow sharing: yes (connection_id in step definition is reused across executions).

### A4.8 — Type safety

No typestate (Validated/Unvalidated). No phantom types per credential kind. Connection parameters are `serde_json::Value`; type checking is done at connection creation via integration metadata.

### A4.9 — vs Nebula

| Feature | Runtara | Nebula |
|---------|---------|--------|
| State/Material split | No — single encrypted JSON blob | Yes — typed State + opaque Material |
| LiveCredential watch() | No — on-demand fetch | Yes — reactive blue-green |
| OAuth2 flow | Yes (auth code + refresh) | Yes (OAuth2Protocol blanket adapter) |
| AES-256-GCM at rest | Yes | Yes |
| Zeroize | Yes (key only) | Yes (secrecy::Secret<T> for material) |
| Typestate (Validated/Unvalidated) | No | Yes |
| DynAdapter erasure | No | Yes |
| Per-process isolation | Yes (single tenant per process) | Via nebula-tenant schema/RLS |

**Verdict:** Runtara's connection management is solid (AES-256-GCM, zeroize, OAuth2 flows, token cache) but simpler than Nebula's. Nebula's State/Material split and LiveCredential reactive refresh are deeper.

---

## 7. Resource management [A5] — DEEP

### A5.1 — Existence

No dedicated resource abstraction equivalent to Nebula's `Resource` trait or `ResourceScope`. Capabilities directly manage their own connections (HTTP clients are created per-call in `runtara-http`). DB pools exist implicitly in `runtara-core` (`sqlx::PgPool`) and `runtara-server`.

Evidence of absence: searched for `trait Resource`, `ResourceScope`, `ReloadOutcome`, `on_credential_refresh`, `ResourceLifecycle` across all crates — not found.

```
# Grep evidence
grep -rn "trait Resource\|ResourceScope\|ReloadOutcome\|on_credential_refresh" crates/ 
# → 0 results
```

### A5.2 — A5.8

Not applicable — no resource abstraction layer exists. Each capability creates its own HTTP client or uses the injected connection parameters. There is no pooling, no hot-reload, no generation tracking, no backpressure on resource acquisition.

**Comparison with Nebula:** Nebula has 4 scope levels, `ReloadOutcome` enum, generation tracking, and `on_credential_refresh` hooks. Runtara has none of this — each agent capability is responsible for its own resource lifecycle. This is simpler but means no coordinated credential rotation across long-running capabilities.

---

## 8. Resilience [A6, A18]

### Retry model

The `#[resilient]` proc macro (`crates/runtara-sdk-macros/src/lib.rs:91`) wraps capability call sites in generated workflow code. Attributes:

```rust
#[resilient(durable = true, max_retries = 3, strategy = ExponentialBackoff, delay = 1000)]
```

- Exponential backoff with configurable base delay.
- `AUTO_RETRY_ON_429`: automatic durable-sleep on 429 responses, respects `Retry-After` header.
- `rate_limit_budget_ms`: per-workflow budget cap on cumulative retry sleep time (default 60,000ms).
- When `durable = false`: all backoff uses `std::thread::sleep` instead of `sdk.sleep`.

**No circuit breaker. No bulkhead. No hedging. No timeout at the resilience layer** (timeout is a per-step DSL field, not the macro).

### Error classification

`AgentError` in `runtara-dsl` (`crates/runtara-agents/src/types.rs`) has `permanent` vs `transient` classification matching the capability's declared `errors(...)`. The `#[resilient]` macro inspects the error JSON `category` field to decide whether to retry. Error conditions can be routed via `onError`-labeled edges in the execution plan.

No unified cross-crate error taxonomy (`ErrorClass` enum) equivalent to Nebula's `nebula-error`.

**Comparison with Nebula:** Nebula has a dedicated `nebula-resilience` crate with retry / circuit breaker / bulkhead / timeout / hedging plus a unified `ErrorClassifier`. Runtara's resilience is entirely in a proc macro — no CB, no bulkhead, no hedging. Runtara's approach is lighter-weight and sufficient for WASM-per-process execution but would not scale for in-process concurrent capability execution.

---

## 9. Expression and data routing [A7]

### DSL mapping engine

Data routing is handled by the `MappingValue` enum (`crates/runtara-dsl/src/schema_types.rs:1226`):

```rust
#[serde(tag = "valueType", rename_all = "lowercase")]
pub enum MappingValue {
    Reference(ReferenceValue),   // dot-path: "steps.fetch.outputs.items"
    Immediate(ImmediateValue),   // literal JSON value
    Composite(CompositeValue),   // nested object/array of MappingValues
    Template(TemplateValue),     // minijinja template string
}
```

Path references use dot notation: `data.user.name`, `steps.<id>.outputs.<field>`, `variables.<name>`, `__error.category`. No JSONPath wildcards or computed expressions in the mapping layer.

Template strings use **minijinja 2.5** with the full execution context available as template variables.

Conditions (`ConditionExpression`) support: `EQ`, `AND`, `OR`, `STARTS_WITH`, `CONTAINS`, and other operators. They are evaluated at runtime inside the compiled WASM code.

No sandbox: the minijinja template engine runs inside the WASM process (which itself is sandboxed by wasmtime WASI).

**Comparison with Nebula:** Nebula has a dedicated expression engine with 60+ functions, type inference, and a sandboxed evaluator. Runtara's mapping is simpler: 4 value types, conditions with a set of operators, and minijinja for templates. No built-in function library. Different design point — Runtara compiles conditions into Rust code; Nebula interprets expressions at runtime.

---

## 10. Plugin and extension system [A11] — DEEP (BUILD + EXEC)

### 10.A — Plugin BUILD process

**A11.1 — Format:** There is no external plugin format. Extensions are native Rust crates linked into `runtara-workflow-stdlib`. The format is "implement a capability function with `#[capability]` and link it in." No WASM blob plugin format, no manifest file, no OCI plugin package.

Evidence: searched for `plugin.toml`, `plugin.json`, `PluginManifest`, `PluginRegistry` in all crates — not found.

**A11.2 — Toolchain:** Capability authors write Rust, compile it with cargo as part of the `runtara-agents` crate (or a downstream crate that links into stdlib). No separate compilation step for capabilities themselves; they compile along with the standard library.

**A11.3 — Manifest:** No manifest. Capabilities self-register via `inventory::submit!` at link time (generated by `#[capability]` macro).

**A11.4 — Registry/discovery:** The `inventory` crate provides compile-time-collected global registries for `CapabilityMeta` and `CapabilityExecutor`. Discovery is in-process via `runtara_dsl::agent_meta::get_all_capabilities()`. No remote registry.

**Comparison with Nebula plugin BUILD:** Nebula targets WASM blob packages with capability declarations; Runtara uses native static linking. Runtara's approach is simpler for stdlib capabilities but does not allow third-party runtime plugin loading.

### 10.B — Plugin EXECUTION sandbox

**A11.5 — Sandbox type:** The workflow itself (compiled to WASM) runs inside **wasmtime** with WASI support (`crates/runtara-environment/src/runner/wasm.rs`). Capabilities are statically linked into the WASM binary — they run inside the same WASM process, not in separate sandboxes. There is no inter-plugin isolation. The WASM sandbox boundary is at the workflow process level, not the capability level.

```rust
// crates/runtara-environment/src/runner/wasm.rs:42
// Runner invokes wasmtime CLI with WASI HTTP and network support
```

**A11.6 — Trust boundary:** The WASM process is isolated from the host by WASI capabilities (network via WASI HTTP, no direct filesystem access). Capabilities inside the WASM module are mutually trusted (no per-capability sandboxing).

**A11.7 — Host↔plugin calls:** No host-provided function calls to capabilities. All capability I/O goes through JSON deserialization. Network calls use `runtara-http` WASI backend.

**A11.8 — Lifecycle:** One WASM process per workflow invocation. No hot-reload. Crash recovery via checkpoint resume (the crashed process's last checkpoint is resumed by re-launching the WASM module with `RUNTARA_CHECKPOINT_ID`).

**A11.9 — vs Nebula:** Nebula targets WASM + capability security model + Plugin Fund commercial monetization (royalties to plugin authors). Runtara uses WASM as an execution sandbox but has no per-capability security model, no plugin distribution system, and no commercial monetization of extensions. Runtara's model is closer to "compile your agents into the stdlib binary" than "load external plugins at runtime."

---

## 11. Trigger and event model [A12] — DEEP

### A12.1 — Trigger types

Five trigger types (`crates/runtara-server/src/api/dto/triggers.rs:11`):

```rust
pub enum TriggerType {
    Http,          // webhook
    Cron,          // cron schedule
    Email,         // incoming email
    Application,   // external system via connection
    Channel,       // conversational (Slack, Teams, Telegram) — session-based
}
```

### A12.2 — Webhook

HTTP triggers registered in the `invocation_triggers` table. URL allocation: stable, determined by tenant and workflow. HMAC verification: present for channel integrations (Slack, Teams) but not generic HTTP triggers. Rate limiting via connections layer.

### A12.3 — Schedule (Cron)

Cron scheduler (`crates/runtara-server/src/workers/cron_scheduler.rs`): polls active CRON triggers in the DB at configurable intervals (default 60s). Uses `croner 2` crate for cron expression parsing. Timezone support: configurable in trigger `configuration` JSON. Missed-schedule recovery: DB `last_run` timestamp; checks if `now >= next_scheduled(last_run)`. No distributed double-fire prevention beyond single-tenant architecture.

### A12.4 — External events

`Application` triggers hook into external systems via connections. Trigger stream: Redis/Valkey streams. The `TriggerStreamPublisher` publishes events to a Valkey stream key; the `TriggerWorker` consumes via `XREADGROUP` / `XAUTOCLAIM` (`crates/runtara-server/src/workers/trigger_worker.rs`). Consumer group semantics prevent double-fire within a tenant.

### A12.5 — Reactive vs polling

Cron: polling (DB poll loop). HTTP/channel: reactive (HTTP callback). Valkey stream: reactive (XREADGROUP blocking read with configurable `block_timeout_ms`).

### A12.6 — Trigger→workflow dispatch

1:1 by default. `single_instance: bool` field on `InvocationTrigger` prevents concurrent execution of the same trigger. Trigger metadata (payload, source) is passed as workflow input JSON.

### A12.7 — Trigger as Action

No. Triggers are infrastructure records in the DB + workers, not workflow step types. There is no `TriggerAction` equivalent to Nebula's. The workflow's entry point (`entryPoint` field in `ExecutionGraph`) receives trigger payload as `data.input`. Triggers are forever-running database records that fire the workflow; the workflow itself has no awareness of trigger lifecycle.

### A12.8 — vs Nebula

Nebula: `TriggerAction` is a first-class action kind with `Input = Config` (registration) and `Output = Event` (typed payload). Source trait normalizes inbound (HTTP req / Kafka msg / cron tick) into `Event`. 2-stage (Source → TriggerAction → workflow).

Runtara: flat model. Triggers are DB rows processed by workers. No 2-stage abstraction. No typed event payload — trigger data arrives as raw JSON in `data.input`. No backpressure model beyond Valkey stream consumer groups.

---

## 12. Multi-tenancy [A14]

Single-tenant per process model (`TENANT_ID` required env var; server panics at startup if unset). Auth modes: `oidc` (JWKS-based JWT validation), `trust_proxy` (reverse-proxy header injection), `local` (no auth). Documented in `docs/deployment/auth-modes.md`.

Tenant isolation at the data layer: every table has a `tenant_id` column with query-level filtering. No PostgreSQL RLS, no schema-per-tenant. JWT claims: `org_id` claim must match `TENANT_ID`; mismatch returns 403.

No RBAC, no SCIM, no SAML. SSO is delegated to an external IdP (Okta, Auth0, Entra ID, Keycloak, Zitadel) via OIDC discovery.

Evidence of absence:
```
grep -rn "RLS\|row.level security\|RBAC\|SCIM\|SAML" crates/ --include="*.rs"
# → 0 results relevant to multi-tenancy features
```

**Comparison with Nebula:** Nebula's `nebula-tenant` has three isolation modes (schema / RLS / database) plus planned RBAC, SSO, and SCIM. Runtara is single-tenant per process with OIDC delegation — simpler but limiting for SaaS multi-tenant deployments. Runtara's model is more of an "embed into your own SaaS" pattern than a "run a shared multi-tenant cluster" pattern.

---

## 13. Observability [A15]

OpenTelemetry stack via `opentelemetry 0.31` + `tracing-opentelemetry 0.32` + OTLP exporter in `runtara-server`. Metrics instruments defined in `crates/runtara-server/src/observability/mod.rs`:

- `runtara.worker.executions.total` / `active` / `duration`
- `runtara.compilation.total` / `active` / `duration` / `queue.size`
- `runtara.trigger.events.total` / `failed` / `processing.duration`
- `runtara.http.requests.total` / `duration`
- `runtara.db.queries.total` / `duration` / `pool.connections.active`

Step-level events: `track_events: Option<bool>` field on `ExecutionGraph`. When enabled, step start/end events are recorded as `step_events` in DB. Step summaries queryable via API.

Structured logging via `tracing` with env-filter. JSON log format available.

**Comparison with Nebula:** Nebula has OpenTelemetry per execution (one trace = one workflow run) with metrics per action. Runtara has OpenTelemetry at the server level with step-event recording for debug mode. Both use the same telemetry stack; Runtara's step_events are opt-in per workflow version vs Nebula's always-on per-execution tracing.

---

## 14. API surface [A16]

REST HTTP API on `:7001` via Axum 0.8. OpenAPI via `utoipa 5` with `#[utoipa::path]` on handlers. All DTOs derive `utoipa::ToSchema`. OpenAPI spec is served at runtime.

MCP (Model Context Protocol) server: `rmcp 1.2` with streamable HTTP transport (`crates/runtara-server/src/mcp/`). MCP tools expose workflow management, executions, connections, and object model to AI agents.

Internal API on `:7002` for service-to-service communication.

No GraphQL. No gRPC. No versioned API prefix.

**Comparison with Nebula:** Nebula has REST now, with GraphQL + gRPC planned, plus OpenAPI generation. Runtara has REST + MCP (Nebula does not have MCP yet). Runtara's MCP integration is a differentiator for AI-agent control of workflows.

---

## 15. Testing infrastructure [A19]

- `runtara-test-harness`: isolated binary for executing capabilities in isolation (`crates/runtara-test-harness/`).
- testcontainers: PostgreSQL in integration tests (`testcontainers-modules 0.11`, `testcontainers 0.23`).
- parity harness: `crates/runtara-core/src/persistence/common/ops/parity_harness.rs` — runs identical operations against both Postgres and SQLite backends.
- wiremock 0.6: mock HTTP in server tests.
- e2e: shell-based tests in `e2e/` with sample workflows.
- `runtara-agents/tests/error_introspection_test.rs`, `custom_module_registration_test.rs`.

No public testing utilities crate for downstream consumers. No contract tests equivalent to Nebula's `resource-author-contracts.md`.

---

## 16. AI / LLM integration [A21] — DEEP

### A21.1 — Existence

**First-class.** `AiAgent` is a built-in `Step` enum variant (`crates/runtara-dsl/src/schema_types.rs:315`). There is a dedicated `runtara-ai` crate. AI integration is central, not peripheral.

### A21.2 — Provider abstraction

The `CompletionModel` trait (`crates/runtara-ai/src/completion.rs:50`):

```rust
pub trait CompletionModel {
    fn completion_request(&self, prompt: Message) -> CompletionRequestBuilder;
    fn completion(&self, request: CompletionRequest) -> Result<CompletionResponse, CompletionError>;
}
```

Synchronous (no async). Providers: one concrete provider in `runtara-ai`: `openai::OpenAICompletionModel` which supports any OpenAI-compatible API (OpenAI, Azure OpenAI, vLLM, Ollama). Anthropic and Bedrock are accessible via the `runtara-agents` integration agents (`openai.rs`, `bedrock.rs`) — they use the capability/connection system, not the `CompletionModel` trait. The `provider.rs` dispatcher (`crates/runtara-ai/src/provider.rs`) currently only creates OpenAI-compatible models.

```rust
// crates/runtara-ai/src/provider.rs:137
"openai_api_key" => {
    let m = create_openai_model_with_connection(parameters, model, connection_id)?;
    // ...
}
// "anthropic_api_key" handled for structured_output_params only, not CompletionModel
```

BYOL endpoint: yes, via `base_url` override or proxy connection_id. Local models: yes, via Ollama (OpenAI-compatible).

### A21.3 — Prompt management

`AiAgentConfig.system_prompt: MappingValue` and `user_prompt: MappingValue` (`crates/runtara-dsl/src/schema_types.rs:1094`). Both can be references to workflow data, immediate literals, or minijinja templates. System/user/assistant structure: yes (mapped to OpenAI message array). Few-shot: via `chat_history: Vec<Message>` in the request. No versioning of prompts; they are inline in the workflow JSON definition.

### A21.4 — Structured output

`AiAgentConfig.output_schema: Option<HashMap<String, SchemaField>>` (`crates/runtara-dsl/src/schema_types.rs:output_schema`). When set, the provider's structured output feature is invoked:
- OpenAI: `response_format: { type: "json_schema", json_schema: { strict: true, schema: ... } }`
- Anthropic: `response_format: { type: "json", schema: ... }` (`crates/runtara-ai/src/provider.rs:82-96`)

Uses DSL `SchemaField` format. Output `response` field is a parsed JSON object. No re-prompting on validation failure (schema enforcement is provider-side).

### A21.5 — Tool calling

The `AiAgent` step uses labeled edges as tools. Each `"tool-name"` labeled edge in `ExecutionPlanEdge` pointing to an `Agent`, `EmbedWorkflow`, or `WaitForSignal` step is exposed as a tool to the LLM (`crates/runtara-workflows/src/codegen/ast/steps/ai_agent.rs:55`). Tool definitions are built from capability metadata (input schema, description). The agentic loop:
1. Send prompt + tools + history to LLM.
2. If tool call: dispatch to matching edge target via generated code, collect result, loop.
3. If text response: store as output, continue to next step.
4. If `max_iterations` reached (default 10): stop with current response.

Multi-tools per call: yes (LLM may call multiple tools per turn). Parallel execution: each tool call is wrapped with `#[resilient]` for checkpoint-based crash recovery. Multi-turn: yes (full conversation history maintained in chat_history). Execution sandbox: same WASM process.

### A21.6 — Streaming

No streaming. `runtara-ai/README.md:16`: "no streaming." The `CompletionModel::completion` is synchronous. No SSE, no chunked response.

### A21.7 — Multi-agent

Agents calling agents: yes via `EmbedWorkflow` as a tool in `AiAgent`. The `AiAgent` step can have a `EmbedWorkflow` step as a tool, which invokes a child workflow. No explicit agent-to-agent coordination pattern beyond this. No shared memory between parallel agent executions. No termination conditions beyond `max_iterations`.

### A21.8 — RAG / vector

No built-in vector store integration. OpenAI embeddings are available as a capability (`openai-create-embedding` in `crates/runtara-agents/src/agents/integrations/openai.rs:1134`) but there is no workflow step type for retrieval, no pgvector, Qdrant, Pinecone, or Weaviate integration.

Evidence:
```
grep -rn "qdrant\|pgvector\|pinecone\|weaviate\|chromadb\|vector_store" crates/ --include="*.rs"
# → 0 results
```

### A21.9 — Memory / context

Conversation memory via `AiAgentMemory` (`crates/runtara-dsl/src/schema_types.rs:1155`):
- Keyed by `conversation_id` (can be a reference to `data.sessionId`).
- Storage delegated to a memory provider agent connected via "memory" labeled edge.
- Compaction: `SlidingWindow` (drop oldest) or `Summarize` (LLM-summarize old messages + replace).
- Max messages threshold configurable (default 50).
- Cross-execution memory: yes (if conversation_id is stable across executions).

Context window management: handled by the compaction strategy. No explicit token counting for truncation.

### A21.10 — Cost / tokens

`CompletionResponse.usage: Option<Usage>` with `prompt_tokens`, `completion_tokens`, `total_tokens` (`crates/runtara-ai/src/completion.rs:97`). No per-provider cost calculation, no budget circuit breakers, no per-tenant token attribution.

### A21.11 — Observability

Tool calls emit `step_debug_start`/`step_debug_end` events (same step events as regular steps) so they appear in the execution trace (`crates/runtara-workflows/src/codegen/ast/steps/ai_agent.rs:25`). No dedicated LLM-call tracing, no prompt+response logging (PII-safe or otherwise), no LLM-as-judge eval hooks.

### A21.12 — Safety

No content filtering pre/post. No prompt injection mitigations documented. Output validation is schema-enforcement only (via provider's structured output feature). No output content classification.

### A21.13 — vs Nebula + Surge

Nebula has no first-class LLM abstraction. Runtara has a first-class `AiAgent` step with tool calling, conversation memory, structured output, multi-provider support (OpenAI-compatible + Bedrock integrations), and MCP server for AI agent control. This is a significant differentiator.

However, the implementation is synchronous (no streaming), provider support is OpenAI-compatible only in the `CompletionModel` trait (Anthropic/Bedrock via separate capability agents), and there is no RAG/vector layer, no token budgeting, and no safety layer. The AI integration is real and working, not just planned.

---

## 17. Notable design decisions

**1. AOT compilation of JSON workflows to WASM**

Workflows are described in JSON and compiled server-side into Rust source code, then compiled to WASM via rustc (`RUNTARA_COMPILE_TARGET = wasm32-wasip2` by default). This means: (a) no runtime interpreter overhead; (b) type errors in workflow logic are caught at compile time (partially — type-erased JSON at step boundaries); (c) compilation latency on first workflow save; (d) users author in JSON/visual editor and never write Rust. Trade-off vs Nebula: Nebula requires users to write Rust; Runtara allows non-Rust developers to author workflows at the cost of a server-side rustc dependency.

**2. Single-tenant per process**

`TENANT_ID` is a required env var; there is no multi-tenant routing within one server process. This simplifies auth, removes cross-tenant data isolation bugs, but requires orchestration to run N tenants. Appropriate for a SaaS platform where each customer gets an isolated container. Nebula's `nebula-tenant` supports multiple tenants in one process with schema/RLS/db isolation.

**3. WASM-first execution with runner fallbacks**

Default runner is WASM (wasmtime CLI) with OCI, Native, and Mock as alternatives. WASM provides sandboxing and portability without container runtime overhead. The workflow binary is compiled once and run many times in isolated wasmtime processes. The OCI runner adds cgroup isolation and metrics for production use with container orchestrators.

**4. Valkey (Redis) as checkpoint transport**

The `runtara-server` layer uses Valkey (Redis-compatible) for checkpoint storage during execution, with a TTL (`CHECKPOINT_TTL_HOURS = 48`). This is distinct from the core persistence layer (PostgreSQL). The choice decouples hot-path checkpoint I/O from the relational DB. Risk: checkpoint data is ephemeral; if Valkey loses data before TTL, recovery would fail. The `runtara-core` PostgreSQL layer still stores instance state; only the in-progress checkpoint blobs go to Valkey.

**5. `inventory`-based capability registration**

Agent capabilities are self-registering at link time via `inventory::submit!` (generated by `#[capability]`). This allows capabilities to be added without modifying any registry file. The tradeoff: all capabilities are compiled into the stdlib binary; there is no runtime plugin loading. Adding a new capability requires recompiling the stdlib.

**6. Non-durable mode as optimization**

The `durable: Option<bool>` flag on workflow and per-step disables checkpoint I/O, enabling short-lived workflows to run without database overhead. The `#[resilient]` macro compiles to different code paths based on this flag. This is a good design for workflows that are fast enough to re-run from scratch on failure.

**7. MCP as first-class API surface**

The `rmcp 1.2` integration makes Runtara natively controllable by AI agents via Model Context Protocol. This is the only competitor in this analysis set with first-class MCP support. It enables LLM agents (Claude, GPT, etc.) to manage workflows as tools without custom API integrations.

---

## 18. Known limitations / pain points

1. **Single-tenant per process** (from `docs/deployment/auth-modes.md`): cannot run multiple tenants in one binary; requires separate deployment per tenant.

2. **Breaking v3.0.0 rename** (CHANGELOG): `Scenario → Workflow` was a hard breaking change with no backward-compat shims, requiring coordinated migration of REST clients, frontend, SDK, and database.

3. **No streaming LLM** (from `runtara-ai/README.md:16`): completions are synchronous and blocking; long-generation requests hold the WASM thread.

4. **No circuit breaker or bulkhead**: resilience is retry-only; no circuit breaker to stop hammering a failing dependency. Large fan-out workflows can exhaust connection pools.

5. **Valkey required for server** (`README.md` configuration table): adds operational complexity vs a PostgreSQL-only deployment.

6. **Time-travel debugger not yet implemented** (Issue #2, open): debugging durable workflows requires raw database queries; no SDK-level time-travel or breakpoint API.

7. **Compilation latency**: the first execution of a workflow triggers a server-side rustc compilation (AOT). Compilation queue introduced in v1.8.0 serializes concurrent compilations but does not reduce first-compile latency.

---

## 19. Bus factor / sustainability

- **Maintainer count:** 1 (volodymyrrudyi — Volodymyr Rudyi). Solo maintainer, similar to Nebula.
- **Commercial entity:** SyncMyOrders Sp. z o.o. (Poland). AGPL with commercial licensing.
- **Commit cadence:** Active April 2026 — daily commits observed in recent git log.
- **Issues ratio:** 4 total (3 closed, 1 open). Tiny issue tracker suggests most work is direct-push or privately tracked.
- **Last release:** v3.0.0 (current). Previous: v1.8.0 (2026-04-13).
- **Risk:** Solo maintainer with a broad codebase (~202K LOC). Sustainability risk similar to Nebula.

---

## 20. Final scorecard vs Nebula

| Axis | Runtara approach | Nebula approach | Verdict | Borrow? |
|------|-----------------|-----------------|---------|---------|
| A1 Workspace | 17 crates, 3 embedded layers (core/env/server), inventory-based registration, edition 2024, Rust 1.90.0 | 26 crates, layered (nebula-error up to nebula-engine/tenant), sealed traits, edition 2024, Rust 1.95.0 | Different decomposition; Nebula has stronger type guarantees between crates | no — different goals |
| A2 DAG | Flat HashMap+Vec ExecutionGraph, DFS validation at compile time, no graph library, type-erased step I/O | TypeDAG L1-L4 (static generics → TypeId → predicates → petgraph) | Nebula deeper — type-safe ports vs JSON-erased; Runtara's JSON authoring is more accessible | refine — Runtara's visual JSON + compile-time validation is worth studying for tooling |
| A3 Action | inventory-registered capability functions, 14 DSL step variants (incl. AiAgent), no sealed trait, I/O as serde_json::Value | 5 action kinds (Process/Supply/Trigger/Event/Schedule), sealed traits, assoc Input/Output/Error | Different decomposition — Nebula richer type safety, Runtara more extensible without Rust knowledge | refine — AiAgent as first-class step is borrowable; tool-call edge labeling pattern is elegant |
| A4 Credential | AES-256-GCM at rest, Zeroize key, OAuth2 auth code flow, token cache (Redis+dashmap), per-tenant JSON blob | State/Material split, LiveCredential watch(), blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter erasure | Nebula deeper — reactive refresh vs on-demand; typestate prevents credential leaks | no — Nebula's already better |
| A5 Resource | None — each capability manages its own resources; no pooling, no scoping, no hot-reload | 4 scope levels (Global/Workflow/Execution/Action), ReloadOutcome enum, generation tracking, on_credential_refresh | Nebula deeper — explicit resource lifecycle vs implicit | no — Nebula's already better |
| A6 Resilience | `#[resilient]` proc macro: retry + exp backoff + AUTO_RETRY_ON_429 + per-step config; no CB/bulkhead/hedging | nebula-resilience crate: retry/CB/bulkhead/timeout/hedging, unified ErrorClassifier | Nebula deeper — full resilience primitives vs retry-only | no — Nebula's already better |
| A7 Expression | MappingValue enum (reference/immediate/composite/template), minijinja templates, condition operators compiled to Rust | 60+ funcs, type inference, sandboxed eval, $nodes.foo.result.email JSONPath-like | Different decomposition — Runtara compiles conditions to Rust (no runtime interpreter); Nebula has richer function library | refine — compile-to-code approach avoids interpreter overhead |
| A8 Storage | sqlx + PgPool + SQLite fallback (core), PostgreSQL (env/server), Valkey for hot checkpoints, sqlx migrations | sqlx + PgPool, Pg*Repo per aggregate, PostgreSQL RLS, SQL migrations | Convergent — both sqlx + PgPool; Runtara adds SQLite embedded option and Valkey for hot-path | refine — SQLite embedded mode is worth noting for desktop deployments |
| A9 Persistence | Checkpoint save/load (direct, not replay), saga compensation with compensation_state enum, Valkey TTL risk | Frontier-based scheduler, checkpoint, append-only execution log, state reconstruction via replay | Different decomposition — Runtara's saga compensation is ahead of Nebula; Nebula's replay model is more durable | refine — saga compensation (CompensationConfig) is borrowable for Nebula |
| A10 Concurrency | tokio async management plane, synchronous WASM execution (single-threaded per process), separate process per instance | tokio runtime, frontier scheduler with work-stealing semantics, !Send action support | Different decomposition — Runtara trades in-process concurrency for process-level isolation; Nebula runs concurrently in-process | no — different goals |
| A11 Plugin BUILD | No external plugin format — capabilities are statically linked Rust functions registered via inventory | WASM blob planned, plugin-v2 spec, Plugin Fund commercial model, capability declaration | Nebula richer — dynamic plugin loading vs static linking | maybe — static inventory registration is simpler; could coexist with Nebula's WASM plugins |
| A11 Plugin EXEC | WASM sandbox at the workflow process level (wasmtime CLI), capabilities inside sandbox, no per-capability isolation | WASM sandbox planned (wasmtime), capability-based security, per-plugin isolation | Nebula richer (planned) — per-capability security vs whole-process sandbox | maybe — Runtara's wasmtime CLI runner is a working reference implementation |
| A12 Trigger | 5 trigger types (HTTP/Cron/Email/Application/Channel), DB-polled cron worker, Valkey stream for events, no TriggerAction step | TriggerAction with Input=Config, Output=Event, Source trait 2-stage, type-safe event payload | Different decomposition — Runtara's Channel/Email triggers are more diverse; Nebula's TriggerAction is more type-safe | refine — Channel trigger (Slack/Teams/Telegram conversational) is borrowable |
| A21 AI/LLM | First-class AiAgent step, CompletionModel trait, OpenAI-compatible providers, tool-calling via edge labels, conversation memory, structured output, MCP server | No first-class LLM — generic actions + plugin LLM client strategy; Surge for agent orchestration | Competitor deeper — Runtara has working first-class AI integration; Nebula has strategic bet only | yes — AiAgent step pattern, tool-call-as-edge, conversation memory with compaction, MCP server all borrowable |

---
