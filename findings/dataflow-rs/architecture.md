# dataflow-rs — Architectural Decomposition

## 0. Project Metadata

- **Repo:** https://github.com/GoPlasmatic/dataflow-rs
- **Stars:** 37 | **Forks:** 4 | **Watchers:** 1
- **License:** Apache-2.0
- **Maintainer:** Plasmatic Engineering / shankar-gpio (primary, 46/50 commits); codetiger (4/50 commits)
- **Created:** 2025-01-25 | **Last release:** v2.1.5 (2026-04-11)
- **Version on crates.io:** 2.1.5 | **Total downloads:** 22.8K | **Recent downloads:** 3.4K
- **Rust MSRV:** 1.85+ (README badge) | **Edition:** 2024 (`Cargo.toml:4`)
- **Crates.io created:** 2025-04-12 — about 11 weeks on crates.io as of this analysis
- **Issue count:** 8 total, all closed; 0 open
- **Governance:** Solo-primary maintainer (shankar-gpio), open Apache-2.0 library, no commercial model, no Plugin Fund equivalent

---

## 1. Concept Positioning [A1, A13, A20]

**Author's description (`README.md:8`):**
> "A lightweight rules engine for building IFTTT-style automation and data processing pipelines in Rust. Define rules with JSONLogic conditions, execute actions, and chain workflows."

**My description after reading code:**
dataflow-rs is a library-first, compile-time-extended rules engine where JSON-defined workflows (IF condition THEN tasks) are pre-compiled at startup and evaluated against an in-memory `Message` struct via a tokio async runtime. It has no server, no visual editor backend, no trigger subsystem, and no persistence layer. Its primary deployment is as a Rust crate embedded into other applications.

**Comparison with Nebula:**
Both use tokio, JSON workflow definitions, and the IFTTT/automation framing. The comparison stops there. Nebula is a 26-crate orchestration platform with DAG execution, credential lifecycle, resource scoping, multi-tenancy, PostgreSQL persistence, and a deployment-binary model. dataflow-rs is a 2-crate embeddable library with no persistence, no credentials, no server, and no trigger layer. The "n8n-style workflow engine" framing from the brief is misleading — dataflow-rs is closer to a JSON rules evaluator (like json-rules-engine or cel-go) than to n8n or Temporal.

**Versus z8run (other n8n-style Tier 1 target):**
z8run is the true n8n-style competitor — it has a React visual editor, SQLite/PostgreSQL backend, WASM plugin sandbox (wasmtime), REST API server, 35+ built-in nodes, 10 AI/LLM nodes, and a JWT auth layer. dataflow-rs has none of these. z8run and Nebula are both server-mode workflow engines; dataflow-rs is a rules evaluation library that happens to be async. The two cannot be fairly compared on most axes.

---

## 2. Workspace Structure [A1]

**Crate inventory (`Cargo.toml:39-40`):**

```
dataflow-rs/
├── src/                    Library crate: dataflow-rs (core engine)
│   ├── lib.rs              Public API re-exports + type aliases
│   └── engine/             Engine module (all core logic)
│       ├── mod.rs          Engine struct, process_message*, with_new_workflows
│       ├── compiler.rs     LogicCompiler — startup JSONLogic pre-compilation
│       ├── executor.rs     InternalExecutor — built-in function dispatch
│       ├── workflow_executor.rs WorkflowExecutor — condition eval + task loop
│       ├── task_executor.rs TaskExecutor — function registry dispatch
│       ├── workflow.rs     Workflow struct + WorkflowStatus enum
│       ├── task.rs         Task struct
│       ├── message.rs      Message struct + AuditTrail + Change
│       ├── trace.rs        ExecutionTrace + ExecutionStep
│       ├── error.rs        DataflowError enum + ErrorInfo + ErrorInfoBuilder
│       ├── utils.rs        get_nested_value / set_nested_value
│       └── functions/
│           ├── mod.rs      AsyncFunctionHandler trait, builtins registry
│           ├── config.rs   FunctionConfig untagged enum (12 variants)
│           ├── map.rs      MapConfig/MapMapping
│           ├── validation.rs ValidationConfig/ValidationRule
│           ├── parse.rs    ParseConfig (JSON + XML)
│           ├── publish.rs  PublishConfig (JSON + XML)
│           ├── filter.rs   FilterConfig (halt/skip)
│           ├── log.rs      LogConfig
│           └── integration.rs HttpCallConfig/EnrichConfig/PublishKafkaConfig
├── wasm/                   cdylib crate: dataflow-wasm (browser bindings)
│   └── src/lib.rs          WasmEngine wrapper via wasm-bindgen
├── ui/                     @goplasmatic/dataflow-ui (React, not a Rust crate)
└── docs/                   mdBook documentation site
```

**Layer separation:** Single-crate for all domain logic. No separation of infrastructure, domain, application, or presentation layers. The entire engine is one module tree. This is appropriate for a library-first design with no I/O dependencies in the core, but it means adding any infrastructure concern (database, HTTP server, credential store) would require either polluting this crate or building a separate one.

**Feature flags (`Cargo.toml:15-17`):** Only one feature: `wasm-web`, which enables WASM-specific dependencies for browser targets. No feature flags for optional integrations.

**Umbrella crate:** None — `dataflow-rs` is the single consumer entry point. `dataflow-wasm` is a packaging artifact, not an independent API surface.

**Comparison with Nebula:** Nebula has 26 crates vs dataflow-rs's 2 (core + WASM). Nebula's separation (nebula-error / nebula-resilience / nebula-credential / nebula-resource / nebula-action / nebula-engine / nebula-tenant / etc.) reflects a system with server-mode deployment, PostgreSQL persistence, multi-tenancy, and plugin sandboxing. dataflow-rs's 2-crate design is appropriate for its scope (embeddable library) but would be entirely insufficient for Nebula's surface area.

---

## 3. Core Abstractions [A3, A17]

### A3.1 — Trait shape

The single extension trait is `AsyncFunctionHandler` (`src/engine/functions/mod.rs:60-78`):

```rust
#[async_trait]
pub trait AsyncFunctionHandler: Send + Sync {
    async fn execute(
        &self,
        message: &mut Message,
        config: &FunctionConfig,
        datalogic: Arc<DataLogic>,
    ) -> Result<(usize, Vec<Change>)>;
}
```

**Open or sealed?** Open — any external crate can implement `AsyncFunctionHandler`. No sealing mechanism.

**`dyn` compatible?** Yes — used as `Box<dyn AsyncFunctionHandler + Send + Sync>` in the registry `HashMap<String, Box<dyn AsyncFunctionHandler + Send + Sync>>` (`src/engine/task_executor.rs:27`).

**Associated types:** Zero. No `Input`, `Output`, `Error`, `Config`, `Context`, or `State` associated types. The input is always `&FunctionConfig` (an enum variant carrying `serde_json::Value`). The output is always `(usize, Vec<Change>)`. This is total type erasure.

**GAT, HRTB, typestate:** None. The trait is maximally simple — a single async method, no bounds beyond `Send + Sync`, no associated types, no generic parameters.

**`async-trait` macro:** Yes, still using `async-trait = "0.1"` crate rather than native async trait methods. This is notable given Edition 2024 (`Cargo.toml:4`) — native async fns in traits are stable on 1.75+, yet the crate uses MSRV 1.85+ and still pulls `async-trait`.

### A3.2 — I/O shape

**Input:** `&mut Message` — a single struct containing `context` (a `serde_json::Value` object with `data`, `metadata`, and `temp_data` fields). No type parameter. The function receives the full context tree and reads from it via JSON pointer paths.

**Output:** `Result<(usize, Vec<Change>)>` — a numeric HTTP-style status code and a vector of `Change` records for audit trail. No output payload type. Side effects are mutations to `message.context` in place.

**Type erasure:** Complete. There is no way to express "this function takes an email address and returns a normalized string" at the type level. All type information lives in the JSON config and is checked at runtime via `serde_json::Value::as_str()` calls.

**Streaming:** None. No streaming output. The only streaming-adjacent feature is the `ExecutionTrace` returned by `process_message_with_trace()`, which captures per-step message snapshots for debugging.

### A3.3 — Versioning

**Workflow versioning:** `Workflow` has a `version: u32` field (`src/engine/workflow.rs:41`, defaulting to 1). This is metadata only — the engine does not use `version` to select different code paths or migration logic. Two workflows with the same `id` but different `version` are treated as distinct workflows if both are loaded.

**Node/function versioning:** None. Functions are identified by string name only (`FunctionConfig::Custom { name: String, .. }`). There is no `v1`/`v2` distinction, no `#[deprecated]` attribute, no migration support. Removing a function type results in `DataflowError::FunctionNotFound` at runtime.

### A3.4 — Lifecycle hooks

The `Task` has no lifecycle hooks beyond the single `execute` method. No `pre_execute`, `post_execute`, `on_failure`, `cleanup`, or `on_cancel`. The `Workflow` has no hooks either. The `WorkflowExecutor` handles `continue_on_error` (`src/engine/workflow_executor.rs:90-111`) which allows error collection without stopping, but this is workflow-level configuration, not a hook.

**Cancellation:** No cancellation mechanism. No `CancellationToken`, no cooperative cancellation point. A long-running custom function cannot be cancelled.

**Idempotency key:** Not present at the engine level. The `Message.id` is a UUID generated at construction (`src/engine/message.rs:69`), not an idempotency key supplied by the caller.

### A3.5 — Resource and credential dependencies

**How does a function declare dependencies?** It does not. Functions access whatever they need via the `message.context` JSON tree or via closure-captured state in the implementing struct. There is no dependency declaration mechanism — no attribute, no associated type, no constructor injection pattern. A function wanting a database pool would capture `Arc<Pool>` in its struct fields.

**Compile-time check:** None. The engine has no awareness of what external resources a custom function needs.

### A3.6 — Retry/resilience attachment

**At the engine level:** The `DataflowError` enum has a `retryable()` method (`src/engine/error.rs:86-114`) that classifies errors as transient (5xx HTTP, timeout, IO) or permanent (4xx, validation, logic). However, **the engine itself does not retry anything**. `retryable()` is a utility for the caller — the engine surfaces the error, the caller decides whether to retry.

**`continue_on_error`:** Both `Workflow` and `Task` have `continue_on_error: bool` fields. When `true`, errors are appended to `message.errors` but execution continues. This is failure tolerance, not retry.

**Retry policy:** No retry configuration, no exponential backoff, no circuit breaker, no bulkhead, no hedging. This is entirely absent.

### A3.7 — Authoring DX

**No derive macro.** To add a custom function:
1. Create a struct
2. Implement `AsyncFunctionHandler` with `#[async_trait]` (~10-15 lines)
3. Register: `custom_functions.insert("name".to_string(), Box::new(MyFn))`
4. Pass to `Engine::new(workflows, Some(custom_functions))`

"Hello world" custom function: approximately 15 lines of Rust.

**No proc-macro for workflow generation.** Workflows are defined in JSON; there is no Rust DSL or builder for constructing them programmatically beyond `Workflow::rule()` and `Task::action()` convenience constructors.

### A3.8 — Metadata

**Function metadata:** None at the engine level. No display name, description, icon, category, or i18n. The only metadata is the `name: String` key in the registry.

**Workflow/Task metadata:** `Workflow` has `name`, `description: Option<String>`, `tags: Vec<String>` (all runtime, from JSON deserialization). No compile-time metadata, no icon, no category registration.

### A3.9 — vs Nebula

Nebula has 5 action kinds (Process / Supply / Trigger / Event / Schedule) with sealed traits and associated `Input`/`Output`/`Error` types giving compile-time type safety at port boundaries. dataflow-rs has 1 extension point (`AsyncFunctionHandler`) with zero associated types and complete type erasure via `serde_json::Value`. The FunctionConfig enum's 12 variants (Map, Validation, ParseJson, ParseXml, PublishJson, PublishXml, Filter, Log, HttpCall, Enrich, PublishKafka, Custom) represent the built-in action kinds, but all share the same `execute` signature — there is no specialization at the type level.

Nebula's 5-kind type hierarchy enables the compiler to verify that a TriggerAction's Output can be connected to a ProcessAction's Input. dataflow-rs's architecture makes this impossible — all type checking happens at runtime via JSON path access.

---

## 4. DAG / Execution Graph [A2, A9, A10]

### Graph model

dataflow-rs does NOT implement a DAG. The execution model is:

1. Workflows are sorted by `priority: u32` at `Engine::new()` (`src/engine/compiler.rs:101`)
2. For each message, workflows execute sequentially in priority order (`src/engine/mod.rs:253-256`)
3. Each workflow evaluates its JSONLogic `condition` against `message.context`
4. If condition is true, tasks within that workflow execute sequentially (`src/engine/workflow_executor.rs:177-209`)

This is a **flat priority-ordered rule chain**, not a DAG. There are no edges, no ports, no port-typing, no fan-out, no fan-in, no parallel branches, no cycles check. The "workflow chaining" advertised in documentation refers to sequential execution of multiple workflows on one message — not a graph.

**"THAT" chaining (`README.md:22-27`):** The THAT in "IF-THEN-THAT" means: after a workflow executes, subsequent higher-priority workflows (sorted at startup) also evaluate against the same message. It is not a graph edge — it is sequential list traversal.

### Port typing

Not applicable. No port concept exists. Data flows through the single `message.context` JSON object which is mutated in place by each task.

### Compile-time checks

None for graph structure — the graph does not exist at the type level. JSONLogic expressions are validated at `Engine::new()` (compile-time analog for logic expressions), but there is no workflow-to-workflow connection validation.

### Concurrency

Workflows within a single `process_message()` call execute sequentially (no parallelism). Multiple concurrent `process_message()` calls are safe because `Engine` is `Arc`-cloned and messages are independent. Custom functions may internally use `tokio::spawn` or `spawn_blocking` but the engine does not parallelize task execution.

**`!Send` handling:** The `AsyncFunctionHandler: Send + Sync` bound means `!Send` types cannot be registered. No thread-local isolation mechanism exists.

**Comparison with Nebula:** Nebula has a TypeDAG with 4 levels (static generics → TypeId → refinement predicates → petgraph) enforcing type correctness at port boundaries. dataflow-rs has no DAG, no ports, and no type checking. Nebula's frontier scheduler enables parallel task execution within a workflow; dataflow-rs is strictly sequential per message.

---

## 5. Persistence and Recovery [A8, A9]

### Storage

**No storage layer.** dataflow-rs has no database dependency, no file persistence, no ORM, no migration system. The `Cargo.toml` has no `sqlx`, no `diesel`, no `sled`, no `rocksdb`.

Grep evidence: `grep -r "postgres\|sqlite\|mysql\|sqlx\|diesel\|database\|db\|persist\|checkpoint" src/ --include="*.rs"` — zero results.

### Persistence model

**No persistence model.** The `Message` struct is an in-memory struct. When `process_message()` returns, the message either lives in the caller's memory or is dropped. There is no append-only log, no event sourcing, no checkpoint, no execution record in any external store.

The `AuditTrail` (`src/engine/message.rs:152-159`) is an in-memory `Vec<AuditTrail>` on `Message`. It records which tasks ran, what changed, and timestamps. It is ephemeral — not persisted anywhere by the engine.

### Crash recovery

**Not provided.** If the calling process crashes, all in-flight messages are lost. Recovery semantics are entirely the caller's responsibility. The `Message` struct implements `Serialize`/`Deserialize` (`src/engine/message.rs:23-64`), which enables callers to checkpoint messages externally if desired, but the engine provides no built-in recovery.

**Comparison with Nebula:** Nebula has a frontier-based scheduler with checkpoint recovery and an append-only execution log enabling state reconstruction via replay. dataflow-rs has none of this — it is a stateless per-message processor with no durability guarantees.

---

## 6. Credentials / Secrets [A4]

### A4.1 — Existence

**No credential layer.** Explicit statement: dataflow-rs has no credential management abstraction of any kind.

Grep evidence: `grep -r "credential\|secret\|vault\|keychain\|encrypt\|zeroize\|secrecy" src/ --include="*.rs"` — zero results in src/ (only a documentation comment in `error.rs` mentions nothing credential-related).

### A4.2 — Storage

Not applicable. No credential storage.

### A4.3 — In-memory protection

Not applicable. No `Zeroize`, no `secrecy::Secret<T>`, no memory protection.

### A4.4 — Lifecycle

Not applicable. No CRUD for credentials, no refresh, no revocation.

### A4.5 — OAuth2/OIDC

Not applicable. No OAuth2 support.

### A4.6 — Composition

Not applicable.

### A4.7 — Scope

Not applicable.

### A4.8 — Type safety

Not applicable.

### A4.9 — vs Nebula

Nebula's credential subsystem (State/Material split, CredentialOps trait, LiveCredential with watch() for blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter for type erasure) has no equivalent in dataflow-rs. Any secret (API key, connection string) used by a custom function must be captured in the struct at construction time by the calling application, outside the engine's awareness. This is appropriate for a library but means the engine provides zero credential-safety guarantees.

**Design decision vs. omission:** Given the library-first scope and IFTTT framing, credential management is intentionally out of scope. The `HttpCallConfig` integration uses a `connector: String` reference field (`src/engine/functions/integration.rs:13`), but the connector resolution (including any credentials) is delegated entirely to the caller's `AsyncFunctionHandler` implementation.

---

## 7. Resource Management [A5]

### A5.1 — Existence

**No resource abstraction.** Explicit statement: dataflow-rs has no first-class resource lifecycle concept.

Grep evidence: `grep -r "resource\|pool\|connection\|reload\|generation\|scope" src/ --include="*.rs"` — zero results related to resource lifecycle.

### A5.2 — Scoping

Not applicable. No scope levels (Global/Workflow/Execution/Action). Custom functions capture resources in their struct fields at registration time; scope is determined by the caller's usage of the `Engine`.

### A5.3 — Lifecycle hooks

Not applicable. No init/shutdown/health-check hooks at the engine level. A custom function struct can implement `Drop` for cleanup, but the engine provides no lifecycle contract.

### A5.4 — Reload

**Partial.** The engine has `with_new_workflows()` (`src/engine/mod.rs:199-232`) for hot-reloading workflow definitions. This creates a new `Engine` instance with fresh compiled logic, reusing the existing function registry `Arc` via `Arc::clone`. The old engine remains valid for in-flight calls. However, this is workflow hot-reload, not resource hot-reload — registered `AsyncFunctionHandler` instances cannot be updated without constructing a new engine.

No `ReloadOutcome` enum, no generation tracking.

### A5.5 — Sharing

Custom function handlers are `Arc`-shared via the registry `Arc<HashMap<String, Box<dyn AsyncFunctionHandler + Send + Sync>>>`. Resources held by handlers (e.g., HTTP clients, DB pools) are shared as the implementing struct sees fit.

### A5.6 — Credential dependencies

Not applicable. No credential-resource interaction.

### A5.7 — Backpressure

Not applicable at the engine level.

### A5.8 — vs Nebula

Nebula has 4 scope levels (Global/Workflow/Execution/Action), `ReloadOutcome` enum, generation tracking for cache invalidation, and per-resource `on_credential_refresh` hooks. dataflow-rs has none of these. The comparison is not relevant — dataflow-rs's library scope does not require resource lifecycle management; the calling application provides all resources.

---

## 8. Resilience [A6, A18]

### Resilience patterns

dataflow-rs has **no retry policy, no circuit breaker, no bulkhead, no timeout, no hedging**.

What it does have:

**`DataflowError::retryable()` method** (`src/engine/error.rs:86-114`): Classifies HTTP 5xx, timeout, and IO errors as retryable; HTTP 4xx, validation, logic, deserialization, and unknown errors as non-retryable. This is a classification helper for callers — the engine itself does not act on it.

**`continue_on_error: bool`** on `Workflow` and `Task`: When `true`, errors are appended to `message.errors` and execution continues. The final message may contain both results and errors. This is failure tolerance, not retry.

**`ErrorInfo` struct** (`src/engine/error.rs:121-151`): Structured error tracking with `retry_attempted: Option<bool>` and `retry_count: Option<u32>` fields. These are informational fields populated by the caller; the engine sets them to `Some(false)` and `Some(0)` at creation time.

### Error classification

The `DataflowError` enum has 10 variants: `Validation`, `FunctionExecution`, `Workflow`, `Task`, `FunctionNotFound`, `Deserialization`, `Io`, `LogicEvaluation`, `Http { status, message }`, `Timeout`, `Unknown`. The HTTP variant's status code is used in `retryable()` to distinguish 5xx (retryable) from 4xx (non-retryable).

**Comparison with Nebula:** Nebula has `nebula-resilience` as a separate crate providing retry, circuit breaker, bulkhead, timeout, and hedging with unified `ErrorClassifier`. dataflow-rs has `retryable()` classification only — no execution policy. The classification is sound but not actionable within the engine.

---

## 9. Expression / Data Routing [A7]

### DSL existence and syntax

dataflow-rs uses **JSONLogic** (via `datalogic-rs = "4.0"`, a GoPlasmatic-maintained Rust implementation) as its expression language.

JSONLogic syntax (`README.md:65-78`):
```json
{
    "condition": {">=": [{"var": "data.order.total"}, 1000]},
    "mappings": [
        {
            "path": "data.order.discount",
            "logic": {"*": [{"var": "data.order.total"}, 0.1]}
        }
    ]
}
```

- `{"var": "data.order.total"}` — field access via dot-path
- `{">=": [...]}`, `{"*": [...]}`, `{"-": [...]}` — operators
- `{"cat": [...]}` — string concatenation
- `{"!!": [...]}` — boolean coercion (truthy check)
- `true` / `false` — literals

JSONLogic is a JSON-native expression language with ~20 operators. It is simpler than Nebula's 60+ function expression engine but has the advantage of being a portable standard (implementations exist in JavaScript, Python, Go, etc.).

**Context access:** Three namespaces accessible from any expression: `data.*`, `metadata.*`, `temp_data.*` (`src/engine/message.rs:71-79`).

### Pre-compilation

All JSONLogic expressions are compiled once at `Engine::new()` via `LogicCompiler::compile_workflows()` (`src/engine/compiler.rs:64-103`). Compiled expressions are stored as `Arc<CompiledLogic>` in `logic_cache: Vec<Arc<CompiledLogic>>`. Tasks/mappings store the index into this Vec (`condition_index: Option<usize>`, `logic_index: Option<usize>`). At evaluation time, the engine accesses `logic_cache[index]` for O(1) compiled logic lookup.

### Sandbox

No sandbox. JSONLogic expressions evaluate against a JSON context tree. The `DataLogic` instance is configured with `DataLogic::with_preserve_structure()` (`src/engine/compiler.rs:43`) to maintain object structure through operations. JSONLogic has no side-effect capabilities (no file access, no network, no system calls) by design — the expression language is safe by construction.

### Channel routing

The engine has O(1) channel-based workflow dispatch via `channel_index: HashMap<String, Vec<usize>>` (`src/engine/mod.rs:113`). Workflows declare a `channel: String` field (default `"default"`); callers can call `process_message_for_channel("orders", &mut message)` to run only workflows registered on that channel.

**Comparison with Nebula:** Nebula's expression engine has 60+ functions, type inference, and the `$nodes.foo.result.email` syntax for referencing outputs of named nodes. dataflow-rs uses JSONLogic's ~20 operators with `{"var": "data.field"}` syntax. Nebula's expression engine is richer; dataflow-rs's is a portable standard. JSONLogic lacks date functions, string regex, and the pipeline-data-reference syntax that Nebula's engine supports.

---

## 10. Plugin / Extension System [A11]

### 10.A — Plugin BUILD process

**There is no plugin build process.** The extension mechanism is compile-time Rust trait implementation — not a plugin in any architectural sense.

**A11.1 — Format:** No format. Custom functions are Rust structs implementing `AsyncFunctionHandler`, compiled into the same binary as the engine.

**A11.2 — Toolchain:** The caller's own Cargo workspace. No separate SDK, no CLI scaffolding, no cross-compilation requirement.

**A11.3 — Manifest content:** No manifest. The function name is a `String` key in a `HashMap`. No capability declaration, no permission grants, no dependency declarations.

**A11.4 — Registry/discovery:** Local `HashMap<String, Box<dyn AsyncFunctionHandler + Send + Sync>>` passed to `Engine::new()`. No remote registry, no OCI, no signing, no versioning.

### 10.B — Plugin EXECUTION sandbox

**There is no execution sandbox.**

**A11.5 — Sandbox type:** Not applicable. Custom functions run in-process, in the same memory space, on the same tokio executor as the engine itself. There is no memory isolation, no CPU/memory limits, no capability restrictions.

**A11.6 — Trust boundary:** None. A custom function has full access to the process's memory, filesystem, network, and system calls. There is zero sandbox enforcement.

**A11.7 — Host↔plugin calls:** The "plugin" is the calling application's own code. The marshaling is direct Rust method dispatch — `handler.execute(message, config, datalogic).await`. No IPC, no serialization, no async crossing (it's the same async task).

**A11.8 — Lifecycle:** No start/stop/reload of individual functions. The function registry is replaced only by constructing a new `Engine`.

**A11.9 — vs Nebula:**

Nebula targets WASM sandbox with capability-based security, Plugin Fund commercial model, and the plugin-v2 spec. dataflow-rs has no WASM plugin sandbox (the WASM target compiles the whole engine to run in a browser, not a host-side sandbox for plugins). The extension model is compile-time Rust trait implementation — the most basic form of extensibility in Rust, not a plugin system.

Grep evidence for absence: `grep -r "wasm\|wasmtime\|wasmer\|wasmi\|plugin\|libloading\|dlopen\|sandbox" src/ --include="*.rs"` — zero results in the core engine src. The wasm/ crate uses wasm-bindgen for browser deployment, which is orthogonal to plugin sandboxing.

---

## 11. Trigger / Event Model [A12]

### A12.1 — Trigger types

**None.** dataflow-rs has no trigger subsystem. Explicit statement: the engine is a pure message processor — it processes `Message` objects passed to `Engine::process_message()`. Nothing inside the engine listens for webhooks, runs on cron, connects to Kafka, polls databases, or watches filesystems.

Grep evidence: `grep -r "trigger\|webhook\|cron\|schedule\|interval\|kafka consumer\|rabbitmq\|nats\|pubsub\|listen\|poll" src/ --include="*.rs"` — the only Kafka-related hits are `publish_kafka` (output, not input trigger), `src/engine/compiler.rs` and `src/engine/functions/integration.rs`.

### A12.2 — Webhook

Not applicable.

### A12.3 — Schedule

Not applicable.

### A12.4 — External event integration

**Partial for output only.** The `PublishKafkaConfig` (`src/engine/functions/integration.rs:123-148`) provides typed config for publishing messages to Kafka topics as a workflow action. The `HttpCallConfig` (`src/engine/functions/integration.rs:6-53`) provides typed config for making HTTP requests. Both are output integrations — they send data out, they do not receive events in.

### A12.5 — Reactive vs polling

Not applicable. The engine is driven by the caller pushing messages.

### A12.6 — Trigger→workflow dispatch

The "dispatch" in dataflow-rs is the caller calling `engine.process_message(&mut msg).await`. No fan-out, no conditional dispatch beyond the built-in JSONLogic condition evaluation on each workflow.

### A12.7 — Trigger as Action kind

Not applicable. There is no trigger kind in the action system.

### A12.8 — vs Nebula

Nebula's TriggerAction has `Input = Config` (registration phase) and `Output = Event` (typed payload), with the Source trait normalizing raw inbound (HTTP req / Kafka msg / cron tick) into a typed Event — a 2-stage model. dataflow-rs has no equivalent architecture. The trigger problem is entirely externalized in dataflow-rs.

This is the sharpest capability gap between dataflow-rs and both Nebula and z8run. z8run has webhook listeners, cron triggers (albeit broken per the z8run analysis), and WebSocket-based real-time execution. dataflow-rs leaves all of this to the embedding application.

---

## 12. Multi-tenancy [A14]

**No multi-tenancy.**

Grep evidence: `grep -r "tenant\|rbac\|scope\|workspace\|user_id\|organization\|schema\|rls" src/ --include="*.rs"` — zero results.

The `Engine` has no concept of owner, user, tenant, workspace, or permissions. Isolation between tenants would require the embedding application to maintain separate `Engine` instances (one per tenant) or route messages through tenant-specific channel groups. There is no RBAC, no SSO, no SCIM.

**Comparison with Nebula:** Nebula has `nebula-tenant` with three isolation modes (schema/RLS/database), RBAC, and planned SSO/SCIM. This axis is not applicable to dataflow-rs's scope.

---

## 13. Observability [A15]

### Tracing framework

**No OpenTelemetry, no structured tracing, no metrics.** The `log = "0.4"` crate is used throughout (`src/engine/workflow_executor.rs:13`): `debug!`, `info!`, `warn!`, `error!` macros for standard log-level output. No trace spans, no trace IDs, no metrics counters.

Grep evidence: `grep -r "tracing\|opentelemetry\|metrics\|prometheus\|span\|trace_id" src/ --include="*.rs"` — the only hits are in `src/engine/mod.rs` (the `trace` module import for `ExecutionTrace`, which is a debug-mode execution snapshot, not an observability trace), and `src/engine/trace.rs` (same).

**ExecutionTrace (A15 substitute):** `process_message_with_trace()` (`src/engine/mod.rs:270-291`) returns an `ExecutionTrace` containing per-step `ExecutionStep` records with message snapshots. This is a **debugging tool**, not an observability pipeline. It captures: workflow_id, task_id, step result (Executed/Skipped), full message state at each step, and per-mapping context snapshots for `map` tasks.

### Audit trail

`AuditTrail` records on `Message` track: workflow_id, task_id, timestamp, status code, and `Vec<Change>` (path, old_value, new_value). This is the primary observability artifact for a processed message.

**Comparison with Nebula:** Nebula uses OpenTelemetry with structured per-execution traces (one trace = one workflow run) and per-action metrics (latency/count/errors). dataflow-rs has no structured telemetry integration; its "trace" is a debug-mode snapshot list, not a proper tracing backend.

---

## 14. API Surface [A16]

### Programmatic API

The public Rust API (`src/lib.rs:192-212`):
- `Engine` — core struct with `new()`, `process_message()`, `process_message_with_trace()`, `process_message_for_channel()`, `with_new_workflows()`, `workflow_by_id()`
- `Workflow` / `Rule` (type alias) — `from_json()`, `from_file()`, `validate()`, `rule()` constructor
- `Task` / `Action` (type alias) — `action()` constructor
- `Message` / `AuditTrail` / `Change` — message access API
- `AsyncFunctionHandler` — the extension trait
- `FunctionConfig` and all typed config variants
- `DataflowError` / `ErrorInfo` / `Result` — error types
- `ExecutionTrace` / `ExecutionStep` / `StepResult` — tracing types

### Network API

**None.** dataflow-rs is a library. It exposes no HTTP server, no gRPC, no WebSocket, no REST API. There is no OpenAPI spec.

### WebAssembly API

`dataflow-wasm` exposes a JavaScript API via wasm-bindgen (`wasm/src/lib.rs`):
- `WasmEngine::new(workflows_json: &str)` — create engine from JSON
- `WasmEngine::process(payload: &str) -> Promise<string>` — async process
- `WasmEngine::process_with_trace(payload: &str) -> Promise<string>` — debug mode
- `WasmEngine::workflow_count() -> usize`
- `WasmEngine::workflow_ids() -> String`
- Free function: `process_message(workflows_json, payload) -> Promise` for one-shot use

Available as `@goplasmatic/dataflow-wasm` npm package.

### Versioning

The crate follows semver. Public API is `pub use` re-exports from `src/lib.rs`. No endpoint versioning (no network API to version).

**Comparison with Nebula:** Nebula has a REST API, planned GraphQL+gRPC, OpenAPI spec generation, and OwnerId-aware routing. dataflow-rs is a library with no network surface — not applicable to the same axes.

---

## 15. Testing Infrastructure [A19]

### Test count and distribution

91 `#[test]` / `#[tokio::test]` assertions across:
- `src/engine/error.rs` — unit tests for error classification, builder, conversions (~35 tests)
- `src/engine/trace.rs` — unit tests for ExecutionTrace, ExecutionStep serialization (~10 tests)
- `src/engine/workflow_executor.rs` — integration tests for workflow condition skip / execute (~6 tests)
- `src/engine/task_executor.rs` — unit tests for function registry / has_function (~4 tests)
- `tests/workflow_engine_test.rs` — integration tests for full engine pipeline

### Testing patterns

- Direct unit tests on data structures (error, trace)
- Integration tests using `Engine::new()` with full workflow JSON strings
- Custom `MockAsyncFunction` structs implementing `AsyncFunctionHandler` for test injection
- `tokio::test` for async tests
- No `insta` snapshot testing, no `wiremock`, no `mockall`

### Public testing utilities

None. dataflow-rs does not publish a `dataflow-rs-testing` crate or testing helpers for consumers. Contract tests for `AsyncFunctionHandler` implementors are not documented.

**Comparison with Nebula:** Nebula has `nebula-testing` crate with contract tests for resource implementors, `insta` + `wiremock` + `mockall`. dataflow-rs tests are functional but not published as a testing infrastructure for third-party implementors.

---

## 16. AI / LLM Integration [A21]

### A21.1 — Existence

**No built-in LLM or AI integration.** Explicit statement with grep evidence.

Grep: `grep -r "openai\|anthropic\|llm\|embedding\|completion\|gpt\|claude\|gemini\|model\|langchain\|ollama\|candle" src/ wasm/ --include="*.rs"` — zero results in any source file.

### A21.2 — Provider abstraction

Not applicable.

### A21.3 — Prompt management

Not applicable.

### A21.4 — Structured output

Not applicable.

### A21.5 — Tool calling

Not applicable.

### A21.6 — Streaming

Not applicable.

### A21.7 — Multi-agent

Not applicable.

### A21.8 — RAG/vector

Not applicable.

### A21.9 — Memory/context

Not applicable. The `message.context["temp_data"]` namespace is a per-message scratchpad, not a multi-turn memory system.

### A21.10 — Cost/tokens

Not applicable.

### A21.11 — Observability

Not applicable.

### A21.12 — Safety

Not applicable.

### A21.13 — vs Nebula + Surge

Nebula's position: no first-class LLM abstraction yet; strategic bet is AI = generic actions + plugin LLM client; Surge handles agent orchestration on ACP. dataflow-rs has the same position by default (no LLM built in), but without a strategic plan. A user wanting LLM integration would implement `AsyncFunctionHandler` calling an HTTP endpoint (OpenAI, etc.) — functional but zero framework support.

**Comparison with z8run:** z8run has 10 built-in AI nodes (LLM, embeddings, vector store, AI agent, etc.) — a massive gap. dataflow-rs and Nebula are aligned in their non-LLM-first stance; z8run is the outlier that ships LLM capabilities today.

---

## 17. Notable Design Decisions

### Decision 1: Pre-compile JSONLogic at startup, zero runtime overhead

**Decision:** All JSONLogic expressions are compiled to `Arc<CompiledLogic>` at `Engine::new()` time. Runtime evaluation uses index-based lookup into a `Vec<Arc<CompiledLogic>>`. No runtime parsing, no runtime validation.

**Trade-off:** Startup time increases with workflow count and complexity. Dynamic workflow addition requires constructing a new engine (via `with_new_workflows()`) rather than adding to a running instance. Early failure detection is a benefit (invalid logic caught at startup). Predictable sub-millisecond latency per message is the main payoff.

**Applicability to Nebula:** Nebula's expression engine also performs compilation, but the compilation model is not as rigidly "compile-all-at-startup". The Arc-wrapped compiled logic cache pattern is worth inspecting for Nebula's expression caching strategy.

### Decision 2: Library-first, no server, no persistence

**Decision:** dataflow-rs deliberately has no HTTP server, no database, no persistence layer. The crate is an embeddable rules engine — the embedding application provides all I/O.

**Trade-off:** Maximum flexibility for embedding use cases (serverless, WASM, embedded systems). Zero constraints imposed on the calling application. But: no standalone deployment, no multi-tenant sharing, no workflow history, no dashboard.

**Applicability to Nebula:** Nebula's 3-mode deployment (desktop/self-hosted/cloud from one codebase) is the inverse — opinionated and deployment-aware. Neither is wrong; they serve different integration patterns.

### Decision 3: Single `Message` struct as universal data context

**Decision:** All data (input, intermediate, output) lives in a single `serde_json::Value` tree structured as `{"data": {}, "metadata": {}, "temp_data": {}}`. JSONLogic expressions access any field from any of these namespaces. There is no per-action input/output typing.

**Trade-off:** Extremely flexible — any task can read/write any field at any time. Easy to understand and debug. The audit trail (per-change records) compensates for lack of explicit data flow edges. Trade-off: no compile-time verification of data dependencies, no port-level type safety, no detection of "task B depends on data produced by task A" at design time.

**Applicability to Nebula:** Nebula's typed DAG with associated Input/Output types is the polar opposite and superior for correctness guarantees. The single-context approach may be worth noting as a simplicity benchmark — it shows what can be achieved without type machinery, useful when evaluating the cost/benefit of Nebula's type complexity.

### Decision 4: `continue_on_error` as soft-failure mode

**Decision:** Both `Workflow` and `Task` have a boolean `continue_on_error` flag. When `true`, errors are collected into `message.errors` but execution continues. The message can carry both results and errors simultaneously.

**Trade-off:** Enables partial-success semantics appropriate for data pipelines where validation failures should not abort transformation. The caller must check `message.has_errors()` after processing. Risk: silent data corruption if a failed task was a prerequisite for a later one.

**Applicability to Nebula:** Nebula's error handling uses `ErrorClass` (transient/permanent/cancelled) and action-level retry policies for more granular control. The `continue_on_error` boolean is simpler; Nebula's approach scales better but is more complex.

### Decision 5: IFTTT reposition in v2.1.0 (commit: `feat: Reposition as IFTTT-style rules engine`)

**Decision:** The 2026-02-21 commit repositions the project from "workflow engine" to "IFTTT-style rules engine" — adding type aliases (`Rule = Workflow`, `Action = Task`, `RulesEngine = Engine`), dual-naming documentation, and the IF→THEN→THAT marketing framing.

**Trade-off:** Broadens appeal beyond Rust workflow developers to users familiar with rules engines and IFTTT. Avoids competing directly with heavier workflow engines (Temporal, Nebula). The rename is additive (aliases, not breaking) but reflects a strategic pivot away from "n8n in Rust" positioning.

**Applicability to Nebula:** None directly. This is a marketing decision. Nebula should note that dataflow-rs self-repositioned away from direct competition.

### Decision 6: wasm-bindgen target for browser execution

**Decision:** The WASM target (`wasm/` crate) compiles the entire rules engine to run in the browser. JSON workflows defined server-side can be evaluated client-side with zero server round-trip.

**Trade-off:** Enables offline rule evaluation, client-side data validation, and edge deployment. The `wasm-web` feature gate isolates WASM-specific dependencies (chrono/wasmbind, getrandom/wasm_js, uuid/js). The browser engine cannot use `rt-multi-thread` Tokio features, using `rt` only.

**Applicability to Nebula:** Nebula's WASM target is for plugins (sandbox), not for running the engine in the browser. Different use case. Worth noting as an alternative WASM deployment pattern.

---

## 18. Known Limitations / Pain Points

### Limitation 1: No trigger subsystem (critical gap for IFTTT framing)

The IFTTT model requires triggers. dataflow-rs has no webhook listener, no cron scheduler, no event bus consumer. Despite the IFTTT branding, users must build their own trigger layer. This is the most frequently implicit gap — every integration example shows the caller creating a `Message` and calling `process_message()`, which requires an external event pump.

**Issue evidence:** No explicit issue filed about this. The IFTTT reposition (v2.1.0, 2026-02-21) added dual naming but no trigger infrastructure. The gap exists by design but is not explicitly documented as a limitation.

### Limitation 2: No persistence or crash recovery

In-memory only. Any in-flight message is lost on process crash. This makes the engine unsuitable for long-running workflows, durable task queues, or compliance-audit-requiring pipelines without external infrastructure. The `Message`'s `Serialize`/`Deserialize` is a hint toward external checkpointing but the engine provides no tooling.

### Limitation 3: No credential/secret management

API keys, connection strings, and tokens must be handled entirely outside the engine. The `HttpCallConfig.connector` field (`src/engine/functions/integration.rs:13`) is a connector name string, but the connector resolution — including credentials — is delegated to the caller. This is intentional for a library but creates a gap for any integration requiring rotation or secure storage.

### Limitation 4: Sequential execution only, no parallelism within a workflow

Tasks within a workflow execute sequentially. There is no mechanism to declare "tasks A and B can run in parallel, then task C depends on both". For CPU-bound workflows with independent tasks, this leaves throughput on the table.

### Limitation 5: Issue #1 — temp_data root overwrite (resolved)

**URL:** https://github.com/GoPlasmatic/dataflow-rs/issues/1
Setting `path: "temp_data"` overwrote the entire `temp_data` object instead of merging. This caused data loss when multiple tasks wrote to `temp_data` root. Fixed in current version. The fix required explicit object merging logic in `set_nested_value`.

### Limitation 6: `async-trait` instead of native async fn in traits

Despite Edition 2024 and MSRV 1.85+, `AsyncFunctionHandler` uses `async-trait = "0.1"` for `async fn` in traits. Native async fn in traits (stable since 1.75) would eliminate the `Box<dyn Future>` boxing overhead and clarify error messages. This is a minor ergonomics debt.

---

## 19. Bus Factor / Sustainability

### Maintainer count

2 contributors in commit history: `shankar-gpio` (46/50 commits, primary), `codetiger` (4/50 commits). Effectively a solo project. No maintainer org structure, no governance doc, no CODEOWNERS file.

### Commit cadence

Active in the analysis window (2025-01-25 to 2026-04-11). All 50 commits in the depth-50 clone span 15 months. Recent activity: 3 releases in a single day (2026-04-11: v2.1.3, v2.1.4, v2.1.5) for build/CI fixes. Prior to that: last code commit was 2026-03-13 (v2.1.2). Gap between v2.1.2 and v2.1.3 was ~1 month.

### Issue ratio

8 total issues, all closed, 0 open. Either very low community engagement or all problems are solved immediately. More likely: very few external users testing in production.

### Downloads

22.8K total downloads, 3.4K recent (as reported by crates.io). For context: first appeared on crates.io 2025-04-12. Download count includes CI bots and automated dependency scanners. 3.4K recent downloads is modest for a rules engine library.

### Related package GoPlasmatic/datalogic-rs

The key dependency (`datalogic-rs = "4.0"`) is also maintained by GoPlasmatic. The engine's core expression power depends on this package. Any breaking change in `datalogic-rs` would cascade. Both packages are maintained by the same team.

### Assessment

Bus factor: 1. Primary risk is maintainer abandonment. The project is young (created 2025-01-25) with active development patterns but low community adoption. The v2.1.0 IFTTT reposition suggests the maintainer is still exploring the right positioning. No SOC 2, no commercial backing, no Plugin Fund, no foundation governance.

---

## 20. Final Scorecard vs Nebula

| Axis | dataflow-rs approach | Nebula approach | Who's deeper / simpler / more correct | Borrow? |
|------|---------------------|-----------------|---------------------------------------|---------|
| A1 Workspace | 2 crates (library + WASM binding). Single-module engine. No layering. | 26 crates layered: nebula-error / nebula-resilience / nebula-credential / nebula-resource / nebula-action / nebula-engine / nebula-tenant / nebula-eventbus / etc. Edition 2024. | **Nebula deeper** — 26 crates vs 2 reflects actual system complexity. dataflow-rs's 2-crate design is appropriate for its scope (embeddable library), not for a deployment platform. | no — different goals |
| A2 DAG | No DAG. Priority-sorted sequential rule list. No ports, no edges, no port typing, no compile-time graph validation. | TypeDAG: L1 = static generics; L2 = TypeId; L3 = refinement predicates; L4 = petgraph soundness. | **Nebula deeper** — TypeDAG L1-L4 provides compile-time and runtime graph correctness. dataflow-rs has no graph model. | no — different goals |
| A3 Action | Single open trait `AsyncFunctionHandler` with zero associated types. Total type erasure via serde_json::Value I/O. 12 built-in FunctionConfig variants. | 5 action kinds (Process/Supply/Trigger/Event/Schedule). Sealed trait. Assoc `Input`/`Output`/`Error`. Versioning via type identity. Derive macros via nebula-derive. | **Nebula deeper** — sealed traits + associated types give compile-time port-level safety. dataflow-rs's type erasure is simpler but loses correctness guarantees. | no — Nebula already better |
| A4 Credential | None. Explicit grep evidence: zero results for credential/secret/vault/zeroize/secrecy. | State/Material split, CredentialOps trait, LiveCredential with watch() for blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter erasure. | **Nebula deeper** — complete credential lifecycle vs zero. Not applicable to dataflow-rs library scope. | no — different goals |
| A5 Resource | None. No resource abstraction. Custom functions capture resources in struct fields. Hot-reload via `with_new_workflows()` reuses function registry Arc. | 4 scope levels (Global/Workflow/Execution/Action). ReloadOutcome enum. Generation tracking. on_credential_refresh hook. | **Nebula deeper** — dataflow-rs delegates all resource management to the embedding application. | no — different goals |
| A6 Resilience | `retryable()` classification method on DataflowError. `continue_on_error` boolean for soft failure. No retry policy, no circuit breaker, no bulkhead, no timeout, no hedging. | nebula-resilience crate: retry/CB/bulkhead/timeout/hedging. Unified ErrorClassifier. | **Nebula deeper** — nebula-resilience is a full resilience crate; dataflow-rs has classification only. `continue_on_error` pattern is worth noting. | refine — `continue_on_error` per-task is simpler than per-action retry, trade-offs differ |
| A7 Expression | JSONLogic via datalogic-rs 4.0. ~20 operators. Syntax: `{"var": "data.field"}`. Pre-compiled to Arc<CompiledLogic> at startup. O(1) index lookup. Three-namespace context (data/metadata/temp_data). | 60+ functions, type inference, sandbox. Syntax: `$nodes.foo.result.email`. JSONPath-like + computed. | **Different decomposition** — JSONLogic is a portable standard; Nebula's engine is richer but proprietary. Pre-compilation pattern is convergent. | refine — Arc<CompiledLogic> Vec cache with index access is worth comparing to Nebula's expression caching |
| A8 Storage | None. No database, no ORM, no migrations. | sqlx + PgPool. Pg*Repo per aggregate. SQL migrations. PostgreSQL RLS for tenancy. | **Nebula deeper** — not applicable to library scope. | no — different goals |
| A9 Persistence | None. In-memory only. AuditTrail is ephemeral on Message. No crash recovery. | Frontier-based scheduler with checkpoint recovery. Append-only execution log. State reconstruction via replay. | **Nebula deeper** — not applicable to library scope. | no — different goals |
| A10 Concurrency | Tokio multi-thread. Sequential task execution per message. Arc<DataLogic> + Arc<logic_cache> shared across async tasks. AsyncFunctionHandler: Send + Sync required. No !Send support. | tokio runtime. Frontier scheduler with work-stealing. !Send action support via thread-local sandbox isolation. | **Nebula deeper** — Nebula's frontier scheduler enables parallel task execution; dataflow-rs is sequential per message. | no — different goals |
| A11 Plugin BUILD | No plugin build process. Compile-time Rust trait implementation. Functions linked into same binary. | WASM, plugin-v2 spec. Capability security. | **Nebula deeper** — WASM sandbox with capability model vs compile-time same-binary extension. | no — different goals |
| A11 Plugin EXEC | No execution sandbox. In-process, full process access, no isolation. | WASM sandbox + capability security. | **Nebula deeper** — zero sandbox enforcement in dataflow-rs. | no — different goals |
| A12 Trigger | None. No webhook, no cron, no external event source. Engine processes messages pushed by caller only. | TriggerAction: Input=Config (registration), Output=Event (typed payload). Source trait normalizes raw inbound. 2-stage. | **Nebula deeper** — 2-stage Source→Event model vs zero trigger infrastructure. Critical gap for IFTTT branding. | no — different goals |
| A13 Deployment | Library only. No binary. Embedded in caller's binary. WASM target for browser. | 3 modes from one binary: desktop / self-hosted / cloud. | **Different decomposition** — library vs deployment platform. Both valid. | no — different goals |
| A14 Multi-tenancy | None. No tenant concept. Isolation via separate Engine instances (caller's responsibility). | nebula-tenant: schema/RLS/database isolation. RBAC. SSO planned. SCIM planned. | **Nebula deeper** — not applicable to library scope. | no — different goals |
| A15 Observability | `log` crate (debug/info/warn/error). AuditTrail per-message (ephemeral). ExecutionTrace (debug snapshots). No OpenTelemetry, no metrics, no spans. | OpenTelemetry per execution. Structured tracing. Per-action metrics (latency/count/errors). | **Nebula deeper** — OTel vs log crate. ExecutionTrace is a useful debug tool but not production observability. | maybe — ExecutionTrace's per-step message snapshot pattern could complement Nebula's span model for step-level debugging |
| A16 API | Rust library API only. JavaScript WASM API (wasm-bindgen). No network API, no OpenAPI. | REST API now. GraphQL + gRPC planned. OpenAPI spec generated. OwnerId-aware. | **Different decomposition** — library vs server. | no — different goals |
| A17 Type safety | No sealed traits, no GATs, no HRTBs, no typestate, no Validated<T>. Complete type erasure via serde_json::Value at all task boundaries. | Sealed traits, GATs, HRTBs, typestate, Validated<T>. | **Nebula deeper** — dataflow-rs's design deliberately sacrifices type safety for simplicity. | no — Nebula already better |
| A18 Errors | `DataflowError` enum (10 variants) with `retryable()` method. `ErrorInfo` struct with `code`/`message`/`path`/`workflow_id`/`task_id`/`timestamp`. `thiserror = "2.0"`. | nebula-error crate. Contextual errors. ErrorClass enum (transient/permanent/cancelled). Used by ErrorClassifier in resilience. | **Nebula deeper** — ErrorClass enum + resilience integration vs ad-hoc retryable() method. Both use thiserror. | refine — `ErrorInfo.code` as a typed string code is a useful pattern for structured error reporting |
| A19 Testing | 91 unit/integration tests. No testing crate for consumers. No insta/wiremock/mockall. | nebula-testing crate. Contract tests. insta + wiremock + mockall. | **Nebula deeper** — published testing crate vs none. dataflow-rs's MockAsyncFunction pattern for test injection is clean. | no — Nebula already better |
| A20 Governance | Apache-2.0. Solo primary maintainer. No commercial model. No Plugin Fund. No SOC 2. 37 GitHub stars. | Open core. Plugin Fund commercial model. Planned SOC 2. Solo maintainer (Vanya). | **Convergent** — both are solo-maintainer open source. Nebula has a commercial strategy (Plugin Fund); dataflow-rs does not. | no — different goals |
| A21 AI/LLM | None. Zero LLM-related code. grep evidence: "openai\|anthropic\|llm\|embedding" — zero results in all source. | No first-class LLM abstraction yet. Generic actions + plugin LLM client plan. Surge = agent orchestrator on ACP. | **Convergent** — neither has first-class LLM. Both leave AI to the embedding application / custom functions. z8run is the outlier with 10 built-in AI nodes. | no — both aligned on "no first-class LLM" |

---

## Appendix: Key code locations

| Concept | File | Lines |
|---------|------|-------|
| `AsyncFunctionHandler` trait | `src/engine/functions/mod.rs` | 59-78 |
| `FunctionConfig` enum (12 variants) | `src/engine/functions/config.rs` | 15-65 |
| `Engine::new()` | `src/engine/mod.rs` | 146-188 |
| `Engine::with_new_workflows()` | `src/engine/mod.rs` | 199-232 |
| `Engine::process_message()` | `src/engine/mod.rs` | 246-258 |
| `Engine::process_message_for_channel()` | `src/engine/mod.rs` | 301-320 |
| `Workflow` struct | `src/engine/workflow.rs` | 22-54 |
| `WorkflowStatus` enum | `src/engine/workflow.rs` | 10-17 |
| `Task` struct | `src/engine/task.rs` | 32-63 |
| `Message` struct | `src/engine/message.rs` | 9-20 |
| `DataflowError::retryable()` | `src/engine/error.rs` | 86-114 |
| `LogicCompiler::compile_workflows()` | `src/engine/compiler.rs` | 64-103 |
| `WorkflowExecutor::execute()` | `src/engine/workflow_executor.rs` | 71-111 |
| `HttpCallConfig` | `src/engine/functions/integration.rs` | 6-53 |
| `WasmEngine::process()` | `wasm/src/lib.rs` | 132-147 |
| `build_channel_index()` | `src/engine/mod.rs` | 119-127 |
| `FILTER_STATUS_HALT` / `FILTER_STATUS_SKIP` | `src/engine/functions/filter.rs` | (exported in mod.rs:23) |
