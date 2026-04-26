# acts — Architectural Decomposition

## 0. Project Metadata

| Field | Value |
|-------|-------|
| Repo | https://github.com/yaojianpin/acts |
| Latest version | 0.17.2 |
| Stars | ~650 (as of research date) |
| Total crates.io downloads | 24.4K |
| Recent downloads (last 90d) | 531 |
| Created | 2023-01-05 |
| Last crates.io push | 2025-06-06 |
| License | Apache-2.0 |
| Governance | Solo maintainer (yaojianpin — Yao) |
| Language | Rust, Edition 2024, no pinned toolchain (uses stable) |
| Open issues | 2 |
| Closed issues | 7 |

---

## 1. Concept Positioning [A1, A13, A20]

**Author's description (README.md line 1):** "Acts is a fast, lightweight, extensiable workflow engine that executes workflows defined in YAML format. Unlike traditional workflow engines (such as BPMN). Acts uses a message-driven architecture to execute and distribute messages."

**Researcher's description (after reading code):** Acts is a single-binary embeddable Rust library that executes YAML-defined workflows using a message bus for all state transitions, a JavaScript engine (QuickJS via rquickjs) for expressions and conditions, and an `inventory`-based compile-time package registry for extensibility. It targets integration with external systems via a gRPC server companion (`acts-server`, separate repo) but ships as a pure library crate.

**Comparison with Nebula:**
- Acts occupies a narrower niche than Nebula: it is a workflow execution library without multi-tenancy, credential management, AI integration, or advanced resilience. It is closer to an n8n-lite or a process-orchestrator SDK.
- Nebula targets "n8n + Temporal + Airflow merged" with 26 specialized crates; acts targets a lightweight embeddable engine with a minimal dependency surface.
- Both use tokio. Both define workflows in a structured format (YAML vs Rust type-safe DSL). Both support branching and steps. Nebula's type system is far more sophisticated; acts prioritizes runtime simplicity.

---

## 2. Workspace Structure [A1]

The workspace `Cargo.toml` (`Cargo.toml:1–15`) declares 7 members across 4 directories:

```
acts/                      # Core engine (THE main crate)
store/sqlite/              # acts-store-sqlite plugin
store/postgres/            # acts-store-postgres plugin
plugins/state/             # Redis state package plugin
plugins/http/              # HTTP request package plugin
plugins/shell/             # Shell command package plugin
examples/plugins/*/        # Example plugin code (excluded from build)
```

**Crate count: 7 workspace members** (plus 3 example crates excluded from default build).

**Feature flags:** `acts/Cargo.toml:62` shows `[features] default = []` — no feature flags in the core crate. All functionality ships unconditionally. Store plugins have a `bundled` feature for SQLite. No conditional compilation of engine features.

**Umbrella pattern:** No umbrella re-export crate. Consumers use `acts` directly. Plugin crates (`acts-store-sqlite`, etc.) are optional add-ons referenced by name.

**Vs Nebula:** Nebula has 26 crates with enforced layer boundaries (nebula-error → nebula-resilience → nebula-credential → nebula-resource → nebula-action → nebula-engine → nebula-tenant). Acts has 1 core crate with all logic co-located. The difference is boundary enforcement: Nebula's crate structure prevents credential code from depending on engine code; acts has no such constraint.

**Edition:** 2024 (matching Nebula).

---

## 3. Core Abstractions [A3, A17] ⭐ DEEP

### Trait Hierarchy

The unit-of-work abstraction in acts is the **package** system, not a generic "action" trait. There are two distinct traits:

**`ActPackage` trait** (`acts/src/package/mod.rs:26–28`):
```rust
pub trait ActPackage {
    fn meta() -> ActPackageMeta;
}
```
This is a pure metadata trait — one associated function returning metadata. No object safety, no `dyn ActPackage`.

**`ActPackageFn` trait** (`acts/src/package/mod.rs:30–40`):
```rust
#[async_trait::async_trait]
pub trait ActPackageFn: Send + Sync {
    fn execute(&self, _ctx: &Context) -> Result<Option<Vars>> {
        Ok(None)
    }
    async fn start(&self, _rt: &Arc<Runtime>, _options: &Vars) -> Result<Option<Vars>> {
        Ok(None)
    }
}
```
This is the execution trait. Both methods have default no-op implementations. Packages override exactly one depending on their `run_as` mode.

**`ActTask` trait** (`acts/src/scheduler/mod.rs:18–35`) — internal scheduler interface:
```rust
pub trait ActTask: Clone + Send {
    fn init(&self, _ctx: &Context) -> Result<()> { Ok(()) }
    fn run(&self, _ctx: &Context) -> Result<()> { Ok(()) }
    fn next(&self, _ctx: &Context) -> Result<bool> { Ok(false) }
    fn review(&self, _ctx: &Context) -> Result<bool> { Ok(true) }
    fn error(&self, ctx: &Context) -> Result<()> { ctx.emit_error() }
}
```
This is implemented by the four node types (Workflow, Branch, Step, Act tasks) and drives scheduler progression.

**`ActPlugin` trait** (`acts/src/plugin/mod.rs:24–26`):
```rust
#[async_trait::async_trait]
pub trait ActPlugin: Send + Sync {
    async fn on_init(&self, engine: &Engine) -> crate::Result<()>;
}
```
Single-method system-level hook called once at `EngineBuilder::build()`.

### A3.1 Trait Shape

- **Open or sealed?** Open — `ActPackageFn` and `ActPlugin` are public traits implementable by any downstream crate. No `Sealed` supertrait, no `private::Impl` trick.
- **`dyn` compatible?** `ActPackageFn` is used as `Box<dyn ActPackageFn>` (`acts/src/package/mod.rs:116`). `ActPlugin` is stored as `Box<dyn ActPlugin>` in `EngineBuilder` (`acts/src/builder.rs:8`).
- **Associated types count:** Zero. No associated `Input`, `Output`, `Error`, or `Config` types. I/O is universally `Vars` (a `serde_json::Map<String, Value>` type alias).
- **GATs:** None.
- **HRTBs:** None.
- **Typestate:** None.
- **Default methods:** Both lifecycle methods on `ActPackageFn` have defaults (do nothing).

### A3.2 I/O Shape

- **Input:** `Vars = serde_json::Map<String, serde_json::Value>` (`acts/src/lib.rs` — `serde_json::Value` map). All inputs are type-erased JSON. No generic type parameters on the trait.
- **Output:** `Result<Option<Vars>>` — same type-erased map, optional.
- **Streaming output:** None. All packages return a single `Option<Vars>`.
- **Side effects model:** Packages call `executor.act().complete()` / `.error()` to signal completion, or directly mutate context via `ctx.task().update_data()`. Context is thread-local (`tokio::task_local!` in `acts/src/scheduler/context.rs:15–17`).

### A3.3 Versioning

- **`Workflow.ver: i32`** exists (`acts/src/model/workflow.rs:32`) but is not used for routing or migration.
- **Package versions** are `&'static str` in `ActPackageMeta.version` — informational only.
- **No v1/v2 dispatch:** There is no mechanism to run two versions of a package simultaneously or migrate workflow instances across versions. Issue #10 ("Add version on a Model/Package") is open and unresolved.
- **Deployment model:** A workflow model is deployed by calling `executor.model().deploy(&workflow)`. Redeploying the same `id` overwrites the stored YAML. No migration hooks.

### A3.4 Lifecycle Hooks

- **`ActPackageFn::execute`** — sync, called with `&Context` for core packages.
- **`ActPackageFn::start`** — async, called with `&Arc<Runtime>` + `&Vars` for event trigger packages.
- **`ActTask::init/run/next/review/error`** — internal scheduler lifecycle. `init` prepares context variables; `run` executes the node; `next` determines if the next node should execute; `review` checks completion conditions.
- **No pre/post/cleanup/on-failure** lifecycle at the `ActPackageFn` level. Error handling is via `catches` blocks in the YAML model.
- **Cancellation:** `EventAction::Abort` and `EventAction::Cancel` are available to clients but there are no cancellation points inside package execution.
- **Idempotency key:** `Act.key` field (`acts/src/model/act.rs:29`) — a user-supplied string used to correlate messages. Not an engine-enforced idempotency guarantee.

### A3.5 Resource and Credential Dependencies

Packages declare no typed dependency on DB pools, credentials, or external resources. Dependencies are injected via the `ActPlugin::on_init` pattern — the plugin initializes its external client (e.g., Redis in `plugins/state/src/lib.rs:18–40`) and captures it in a closure registered on the channel. There is no compile-time check that a package's runtime dependencies are satisfied.

### A3.6 Retry/Resilience Attachment

`Act` model has a `Retry { times: i32 }` field (`acts/src/model/act/retry.rs:1–18`) and a `Timeout { on: String, steps: Vec<Step> }` field (`acts/src/model/act/timeout.rs:1–15`). Retry is configured per-act in YAML. No circuit breaker, no bulkhead, no exponential backoff, no jitter. The retry count is a simple integer. Timeout is expressed in human-readable form (`1d`, `2h`, `30m`, `60s`) and triggers a sub-steps sequence when exceeded.

Message delivery retry (for ACK-required channels) is separate: `config.max_message_retry_times` (default 20) with a tick interval of 15 seconds (`acts/src/config.rs:78–80`).

### A3.7 Authoring DX

Minimal. A "hello world" custom package in Rust requires:
1. Define a struct, derive `Serialize/Deserialize`.
2. Implement `ActPackage::meta()` with name, description, version, JSON schema.
3. Implement `ActPackageFn::execute()` or `start()`.
4. Call `inventory::submit!(ActPackageRegister::new::<MyPackage>())`.

No derive macros from acts itself. No CLI scaffolding. Approximately 40 lines for a minimal package. The schema validation is via `jsonschema` at registration time — parameters are validated against the declared JSON Schema before `execute()` is called.

### A3.8 Metadata

`ActPackageMeta` (`acts/src/package/mod.rs:68–95`): `name: &'static str`, `desc: &'static str`, `icon: &'static str`, `doc: &'static str`, `version: &'static str`, `schema: serde_json::Value`, `run_as: ActRunAs`, `resources: Vec<ActResource>`, `catalog: ActPackageCatalog`. All compile-time static strings. No i18n. No runtime override.

The `resources` and `catalog` fields suggest an intent to support a visual workflow editor that can browse and select packages — consistent with the roadmap item `plugins/form`.

### A3.9 Comparison with Nebula

Nebula has **5 action kinds** (ProcessAction / SupplyAction / TriggerAction / EventAction / ScheduleAction) with **sealed traits**, **associated Input/Output/Error types**, **versioning via type identity**, and **derive macros via nebula-derive**. Acts has **one execution trait** (`ActPackageFn`) with **type-erased Vars I/O**, **no versioning**, and **no derive macros**. The fundamental design difference: Nebula's type system catches mismatches at compile time; acts defers all type checking to JSON Schema validation at runtime registration.

Acts' `ActRunAs` enum (`acts/src/package/mod.rs:43–62`) with variants `Func/Irq/Msg` provides a rough functional analog to Nebula's 5 action kinds but is far less expressive:
- `Irq` ≈ ProcessAction (interrupt, wait for response)
- `Msg` ≈ EventAction (fire-and-forget notification)
- `Func` ≈ internal utility (no client visibility)

Acts has no equivalent to Nebula's SupplyAction (data provision), TriggerAction (workflow start), or ScheduleAction (cron/timer).

---

## 4. DAG / Execution Graph [A2, A9, A10]

### Graph Description

Acts workflows are **linear trees**, not general DAGs. The `NodeTree` (`acts/src/scheduler/tree/node_tree.rs`) holds a rooted tree where each `Node` has `children`, `prev`, `next`, and `parent` pointers. The node types form a strict hierarchy: `Workflow → Step → (Branch →)* Step → Act`.

```
NodeKind enum (acts/src/scheduler/tree/node.rs:21–27):
  Workflow | Branch | Step | Act
```

Branching is explicit via `Branch` nodes (if/else conditions evaluated in JavaScript). Steps execute sequentially. Acts within a step execute sequentially unless wrapped in `acts.core.parallel`. There is no cycle support, no arbitrary edge connections, and no DAG-level port typing.

### Port Typing

None. There are no typed ports. Data flows through the global `Vars` map — all act inputs and outputs share the same key-value namespace. The `inputs:` and `outputs:` declarations in YAML are conventions, not type constraints (`acts/src/model/act.rs:44–52`).

### Compile-time vs Runtime Checks

There are no compile-time checks on workflow structure. `NodeTree::load()` (`acts/src/scheduler/tree/node_tree.rs:25–50`) validates the tree at deploy time by walking nodes and checking for duplicate IDs. Condition expressions (`if: value > 100`) are compiled and evaluated at runtime via rquickjs.

### Scheduler Model

Single-threaded event loop with a tokio mpsc channel as the task queue. The `Scheduler` (`acts/src/scheduler/scheduler.rs`) has a bounded channel of size 100 (`acts/src/scheduler/queue/queue.rs:8`). `Scheduler::next()` is called in a loop by `Runtime::event_loop()` spawned as a separate tokio task. Each task is executed by calling `task.exec(ctx)` synchronously within that loop iteration.

`Process` instances are cached in a Moka LRU cache (default cap 1024). When a process is evicted from cache, it is stored to the backing store. On `cache.restore()`, processes are reloaded from store and re-enqueued.

### Concurrency

Multiple processes can run concurrently via `tokio::spawn` — `Runtime::launch()` spawns each process start. Within a process, tasks execute sequentially via the single scheduler loop. The `acts.core.parallel` package creates sibling Act nodes dynamically and they enqueue independently, achieving per-process parallelism.

`tokio::task_local! static CONTEXT: Context` (`acts/src/scheduler/context.rs:15`) means the execution context is thread-local and propagated implicitly. This is the `!Send` analogue: context does not flow across awaits or spawns — it is scoped to the synchronous execution of `task.exec()`.

**Vs Nebula:** Nebula has TypeDAG with 4 levels (L1 static generics, L2 TypeId, L3 refinement predicates, L4 petgraph soundness checks). Acts has no compile-time DAG type safety and uses a simple linked-list tree traversal. Nebula's frontier-based scheduler vs acts' single-queue sequential scheduler — acts does not have work-stealing semantics.

---

## 5. Persistence and Recovery [A8, A9]

### Storage Layer

The `DbCollection` trait (`acts/src/store/mod.rs:52–61`) defines a generic CRUD interface:
```rust
pub trait DbCollection: Send + Sync {
    type Item;
    fn exists(&self, id: &str) -> Result<bool>;
    fn find(&self, id: &str) -> Result<Self::Item>;
    fn query(&self, query: &Query) -> Result<PageData<Self::Item>>;
    fn create(&self, data: &Self::Item) -> Result<bool>;
    fn update(&self, data: &Self::Item) -> Result<bool>;
    fn delete(&self, id: &str) -> Result<bool>;
}
```

Six collection types: `Tasks`, `Procs`, `Models`, `Messages`, `Events`, `Packages` (`acts/src/store/mod.rs:28–43`).

Default backend: `MemStore` — a `HashMap`-based in-memory store with no durability. Optional plugins: `acts-store-sqlite` (using `rusqlite` + `r2d2` connection pool + `sea-query` query builder) and `acts-store-postgres` (using `sqlx` + async `sea-query`). Plugins register collections via `engine.extender().register_collection()`.

### Persistence Model

**Checkpointing**, not event sourcing. Each state transition updates the task/process record in-place. There is no append-only execution log. On recovery:
1. `cache.restore()` (`acts/src/cache/cache.rs:~90`) queries the store for in-progress `Proc` records.
2. For each `Proc`, its serialized `NodeTree` (stored as JSON in `data::Proc.tree`) is deserialized.
3. Tasks with non-terminal states are re-enqueued.

`data::Task` (`acts/src/store/data/task.rs`) stores `node_data: String` (JSON of `NodeData`), `state: String`, `data: String` (JSON of `Vars`), `hooks: String` (JSON of lifecycle hooks). This is a snapshot-based approach.

**Vs Nebula:** Nebula uses frontier-based checkpointing with an append-only execution log, enabling state reconstruction via replay. Acts uses simpler in-place update checkpointing with no replay semantics. Acts cannot reconstruct the full execution history; Nebula can.

### Migrations

No migration infrastructure. SQLite plugin uses `CREATE TABLE IF NOT EXISTS` at init time. No versioned migrations, no rollback support.

---

## 6. Credentials / Secrets [A4] ⭐ DEEP

### A4.1 Existence

**No dedicated credential layer exists.** Acts does not have a `Credential` type, `CredentialOps` trait, or credential storage.

**Grep evidence:**
- Searched `acts/src/` for `credential`, `secret`, `token`, `oauth`, `auth`: only `acts/src/env/moudle/vars/secrets.rs` matches.
- `secrets.rs` is 7 lines: it implements `ActUserVar` to expose a `secrets` JavaScript global (`acts/src/env/moudle/vars/secrets.rs:1–7`).
- No encryption, no vault integration, no key management.

### A4.2 Storage

No credential storage. Secrets are passed as plain `Vars` at workflow start time. There is no persistence of credentials.

### A4.3 In-memory Protection

No `Zeroize`, no `secrecy::Secret<T>`. Secrets passed via `Vars` (i.e., `serde_json::Map<String, Value>`) remain in heap memory as plain strings until garbage collected.

### A4.4 Lifecycle

No lifecycle. Secrets are scoped to a single workflow process invocation. No refresh, no rotation, no revocation.

### A4.5 OAuth2/OIDC

No OAuth2 support of any kind. Searched `Cargo.toml` for `oauth`, `oidc`, `openid` — found nothing.

### A4.6 Composition

Single flat namespace. All secrets available in the `secrets.FIELD` JS global.

### A4.7 Scope

Per-process invocation only. Secrets are provided at `executor.proc().start("model_id", &vars)` and stored in the process environment. No workspace-level or global secrets store.

### A4.8 Type Safety

No type safety for secrets. Everything is `serde_json::Value`.

### A4.9 vs Nebula

Nebula has: State/Material split, LiveCredential with watch() for blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter type erasure, encrypted storage, Zeroize protection. Acts has: a 7-line `SecretsVar` that exposes a JavaScript global. Acts has essentially no credential system. This is the largest single capability gap vs Nebula.

---

## 7. Resource Management [A5] ⭐ DEEP

### A5.1 Existence

**No dedicated resource abstraction.** There is no `Resource` trait, no pool management, no lifecycle-managed DB or HTTP client as a first-class engine concept.

**Grep evidence:** Searched `acts/src/` for `resource`, `pool`, `lifecycle`, `scope`, `reload` — no resource management concepts found. The word "resources" appears only in `ActPackageMeta.resources: Vec<ActResource>` which is UI metadata (name/description strings for editor display), not runtime resource management.

### A5.2 Scoping

No scoping levels. Resource-like objects (e.g., a Redis client in the state plugin) are created once during `ActPlugin::on_init`, captured in closures registered on the channel, and shared via clone (the Redis client in `plugins/state/src/lib.rs:22` is cloned into the closure). The scope is effectively "global for the process lifetime" with no per-workflow or per-execution scoping.

### A5.3 Lifecycle Hooks

`ActPlugin::on_init` is the only lifecycle hook available. No `on_shutdown`, no `health_check`. If init fails, the `EngineBuilder::build()` returns an `Err` and the engine does not start.

### A5.4 Reload

No hot-reload. No blue-green. No generation tracking. Reloading requires stopping and restarting the engine.

### A5.5 Sharing

Resources are shared via `Arc::clone` and closure capture. The Redis client in the state plugin is owned by the closure — one connection per channel handler. No pooling infrastructure provided by acts itself (the SQLite store uses `r2d2` connection pool from `r2d2_sqlite`, but this is the plugin's own concern).

### A5.6 Credential Dependencies

Resources cannot declare credential dependencies — there are no credentials in the system.

### A5.7 Backpressure

No backpressure for resource acquisition. The task queue channel has a hard cap of 100 items (`acts/src/scheduler/queue/queue.rs:8`), which provides implicit backpressure at the scheduler level but not at the resource level.

### A5.8 vs Nebula

Nebula has: 4 scope levels (Global/Workflow/Execution/Action), ReloadOutcome enum, generation tracking, on_credential_refresh per-resource hook. Acts has: none of these. Resources are implicit — plugin authors manage their own external connections with no framework assistance.

---

## 8. Resilience [A6, A18]

### Error Handling

`ActError` (`acts/src/error.rs:8–33`) is a `thiserror::Error`-derived enum with variants: `Config`, `Convert`, `Script`, `Exception { ecode, message }`, `Model`, `Runtime`, `Store`, `Action`, `IoError`, `Package`. There is no `ErrorClass` categorization (no transient/permanent/retryable distinction). All errors are treated equally — a `Store` error is not distinguished from a transient network timeout.

The `Error` struct (`acts/src/error.rs:35–44`) with `ecode: String` and `message: String` is the user-visible error type propagated to workflow catches.

### Retry

`Act.retry.times: i32` (`acts/src/model/act/retry.rs`) provides a retry count per act. The scheduler task implementation retries based on this count. No backoff strategy, no jitter, no exponential increase. Zero retry by default.

Message delivery retry: `config.max_message_retry_times` (default 20) with `config.tick_interval_secs` (default 15s) tick interval. The tick handler resends unacked messages (`acts/src/scheduler/runtime.rs:~170`). No per-message backoff.

### Circuit Breaker

No circuit breaker. Searched for `circuit`, `breaker`, `bulkhead`, `hedg` — found nothing.

### Timeout

`Step.timeout: Vec<Timeout>` (`acts/src/model/step.rs:21`) and `Act.timeout: Vec<Timeout>` (`acts/src/model/act.rs:47`) — timeout rules expressed in human-readable durations (`1d`, `2h`, `30m`, `60s`). When a timeout fires, a set of `steps` executes (typically containing an `irq` to notify the operator). Implementation is in the tick handler which checks elapsed time at each tick interval.

### Vs Nebula

Nebula has `nebula-resilience` crate: retry with backoff + jitter, circuit breaker, bulkhead, timeout, hedging, unified `ErrorClassifier`. Acts has: basic retry count, a tick-based timeout, message re-delivery — no sophisticated resilience patterns.

---

## 9. Expression / Data Routing [A7]

### DSL

Acts uses **JavaScript** as its expression and scripting language, embedded via `rquickjs` (QuickJS binding, `acts/Cargo.toml:16`). This is the primary differentiator compared to most Rust workflow engines.

**Expression contexts:**
- `if:` conditions on Step/Branch/Act — JavaScript boolean expression evaluated at runtime.
- `acts.transform.code` package — runs arbitrary JavaScript with access to all workflow variables.
- `{{ variable }}` template syntax in shell plugin parameters (`acts/src/utils/consts.rs`).

### JavaScript Runtime Architecture

`Enviroment` (`acts/src/env/mod.rs:56`) holds a list of `ActModule` implementations. On each evaluation, a `JsRuntime` + `JsContext` are created fresh (from `rquickjs::Runtime::new()` and `JsContext::full()`), modules are initialized, and the expression is evaluated. **No sandbox reuse** — a fresh QuickJS context per evaluation call.

Built-in JS globals exposed via modules (`acts/src/env/moudle/mod.rs:14–22`):
- `console` — logging
- Arrays utilities
- `$act` module: `$get(name)`, `$set(name, val)`, `$inputs()`, `$data()`, `$set_process_var(name, val)`
- `$step` — per-step variable access
- `$env` — workflow environment variables
- `secrets.FIELD` — secrets (user-provided data)
- `os` — operating system information

**No custom function library** (unlike Nebula's 60+ built-in functions). Workflow authors can call standard JavaScript functions (`Math`, `JSON`, string methods, etc.) plus the acts-specific globals.

### Sandbox

rquickjs/QuickJS has limited filesystem/network access by default, but acts does not explicitly configure resource limits or a strict capability sandbox. CPU/memory limits are not set. `acts.transform.code` executes arbitrary JavaScript. `acts.app.shell` executes arbitrary shell commands — this is an explicit escape hatch with no sandboxing.

**Vs Nebula:** Nebula has a custom expression engine with 60+ functions, type inference, and JSONPath-like `$nodes.foo.result.email` syntax. Acts uses JavaScript which is more powerful but less type-safe and harder to analyze statically. Nebula's expression engine is sandboxed by design; acts' JavaScript is sandboxed only by QuickJS's default constraints (no Node.js APIs) but the shell plugin bypasses all sandboxing.

---

## 10. Plugin / Extension System [A11] ⭐ DEEP

### 10.A — Plugin BUILD Process

**A11.1 Format:** Rust crates in the workspace. No binary artifact format (no .tar.gz, no OCI, no WASM blob). Plugins are compiled as regular `cdylib`-free Rust crates. Manifest format: `Cargo.toml`. No custom plugin manifest schema.

**A11.2 Toolchain:** Standard cargo build. All plugins compile in the same workspace, same Rust toolchain, same edition (2024). No cross-compilation required for local plugins. No SDK beyond implementing the `ActPlugin` and/or `ActPackageFn` traits.

**A11.3 Manifest Content:** `ActPackageMeta` (`acts/src/package/mod.rs:68–95`) is the "manifest" for package plugins:
```rust
pub struct ActPackageMeta {
    pub name: &'static str,      // dot-namespaced: "acts.core.irq"
    pub desc: &'static str,
    pub icon: &'static str,
    pub doc: &'static str,
    pub version: &'static str,
    pub schema: serde_json::Value,  // JSON Schema for parameters
    pub run_as: ActRunAs,           // Func | Irq | Msg
    pub resources: Vec<ActResource>, // UI metadata
    pub catalog: ActPackageCatalog, // Core | Event | Transform | Form | Ai | App
}
```
No capability declarations, no permission grants, no network/fs/crypto capability requirements.

**A11.4 Registry/Discovery:** Two mechanisms:
1. **Compile-time registration** via `inventory::submit!(ActPackageRegister::new::<T>())` (`acts/src/package/mod.rs:190`). The `inventory` crate uses linker sections to collect all registrations at startup. Built-in packages use this.
2. **Runtime registration** via `engine.extender().register_package(&meta)` (`acts/src/export/extender.rs:50–63`). External plugins call this from `ActPlugin::on_init`. This only registers the metadata/schema — execution logic is handled by the plugin listening on a channel.

No remote registry. No signing. No version pinning for plugin compatibility.

### 10.B — Plugin EXECUTION Sandbox

**A11.5 Sandbox Type:** **None.** Plugins execute in the same process, in the same memory space as the engine. There is no process isolation, no WASM sandbox, no IPC. Plugin packages receive a `Context` reference or `Arc<Runtime>` and execute directly on the tokio runtime. The external plugin pattern (http, state, shell) is: subscribe to engine messages, execute logic in the plugin's own tokio task spawned via the channel callback.

```rust
// plugins/http/src/lib.rs:12–42
chan.on_message(move |e| {
    tokio::spawn(async move {
        let pack = package::HttpPackage::create(&inputs);
        // HTTP request executes here, in the same process
        match pack.run().await {
            Ok(data) => executor.act().complete(&pid, &tid, &data).unwrap(),
            Err(err) => executor.act().error(&pid, &tid, &err.into()).unwrap(),
        };
    });
});
```

**A11.6 Trust Boundary:** Plugins are fully trusted. No capability-based security. CPU/memory/wall-time limits are not enforced. A buggy plugin can crash or deadlock the engine.

**A11.7 Host↔plugin Calls:** For built-in packages: `execute(&ctx)` direct Rust function call. For external plugins (the "external execution" pattern): message passing via `Channel`. Plugin listens for `on_message`, executes its logic, calls `executor.act().complete/error` to return results. Marshaling is via `Vars` (JSON map). No protobuf, no WIT, no wit-bindgen. The channel is async (tokio mpsc internally) but the plugin callback is sync (fn pointer, spawns its own tokio task for async work).

**A11.8 Lifecycle:** No explicit start/stop/reload for packages. `ActPlugin::on_init` is called once. No crash recovery — if a plugin's tokio task panics, the panic is lost (tokio spawn default behavior). No `on_shutdown`.

**A11.9 vs Nebula:** Nebula targets WASM sandbox with wasmtime, capability-based security, and a commercial Plugin Fund model. Acts has no sandbox, no capability security, no commercial model for plugins. Acts' plugin system is simpler and pragmatic for trusted-environment use cases (same-process, same-crate compilation) but unsuitable for a marketplace/fund model where plugins must be isolated.

---

## 11. Trigger / Event Model [A12] ⭐ DEEP

### A12.1 Trigger Types

Triggers in acts are `Act` instances in the `workflow.on: Vec<Act>` field (`acts/src/model/workflow.rs:26`). Three built-in trigger types:

| Package name | Type | Status |
|---|---|---|
| `acts.event.manual` | Manual/programmatic start | Implemented |
| `acts.event.hook` | Hook (waits for workflow completion) | Implemented |
| `acts.event.chat` | Chat event | Implemented |
| `acts.event.schedule` | Cron/interval | **Roadmap — not implemented** |

**Webhook:** Not natively supported. Would require `acts-server` (separate gRPC server) as a gateway.
**External events (Kafka/RabbitMQ/NATS/Redis Streams):** Not natively supported. `pubsub` is on roadmap.
**FS watch:** Not supported.
**DB CDC/LISTEN-NOTIFY:** Not supported.
**Polling:** Not supported natively.
**Internal events:** `acts.core.msg` (fire-and-forget notification to channel subscribers), `setup` acts (hooks on `created/completed/before_update/updated` node lifecycle events).
**Manual:** `acts.event.manual` — starts the workflow when called programmatically.

### A12.2 Webhook

No webhook support built-in. `acts-server` (separate repo) provides a gRPC interface that could serve as a webhook gateway with an external HTTP adapter. No URL allocation, no HMAC verification, no idempotency key at the acts level.

### A12.3 Schedule

**Roadmap only.** `schedule` appears in the README roadmap as `[ ] schedule` under the event packages section. No implementation exists. Searched `acts/src/` for `cron`, `schedule`, `interval`, `timer` — found only timeout duration parsing (`acts/src/model/act/timeout.rs`) for step timeouts, not workflow scheduling.

### A12.4 External Event

Not implemented. The `acts.event.hook` (`acts/src/package/event/hook.rs`) pattern is closest to an external event trigger — it starts a workflow and blocks the caller until the workflow completes using `Signal`. This is synchronous invocation, not async event ingestion from a broker.

### A12.5 Reactive vs Polling

Default: reactive via message channel. The engine emits messages on state transitions; clients subscribe via `engine.channel().on_message()`. There is no polling loop for triggers — triggers are invoked programmatically or via acts-server.

### A12.6 Trigger→Workflow Dispatch

1:1 mapping. Each `workflow.on` event starts one process. Fan-out would require multiple `on` declarations or a custom package. Trigger metadata is passed as `params` to `ActPackageFn::start()`, which becomes process inputs. No conditional triggers (conditions are per-act, not per-trigger). No replay support.

### A12.7 Trigger as Action

Trigger packages (`acts.event.manual`, `acts.event.hook`, `acts.event.chat`) implement `ActPackageFn::start()` (async), not `execute()`. Their `run_as: ActRunAs::Func` — they are "Func" type packages that trigger workflow starts but do not themselves appear in workflow steps. They are more like "event sources" than workflow steps. Lifecycle: they fire once and complete. The `hook` variant blocks until the triggered workflow completes.

### A12.8 vs Nebula

Nebula has a 2-stage model: `Source` trait normalizes raw inbound (HTTP request, Kafka message, cron tick) → typed `Event` → `TriggerAction` (which is itself one of the 5 action kinds). Acts has a 1-stage model: trigger packages directly start workflow instances. No normalized event type, no Source trait. Nebula's TriggerAction has typed `Input = Config` (registration) and `Output = Event` (typed payload) — acts has untyped `Vars`. Nebula has webhook, cron, and future event bus triggers; acts has manual/hook/chat only.

---

## 12. Multi-tenancy [A14]

**No multi-tenancy support.** Acts is a single-tenant embedded library. There is no `Tenant` type, no schema isolation, no RLS, no RBAC, no SSO, no SCIM.

**Grep evidence:** Searched `acts/src/` and all workspace members for `tenant`, `rbac`, `permission`, `role`, `access control`, `schema isolation` — found nothing.

Workflows are identified by model ID and process ID. All processes share the same store collections. No user identity concept exists in the core engine — user identity is a convention implemented via `secrets.user_id` pattern by the application layer.

**Vs Nebula:** Nebula has `nebula-tenant` crate with three isolation modes (schema/RLS/database), RBAC, planned SSO/SCIM. Acts has no equivalent at any level.

---

## 13. Observability [A15]

### Tracing

Acts uses the `tracing` crate (`acts/Cargo.toml:27`) for structured logging. There are 237 tracing calls in `acts/src/` (debug/info/error/instrument). Instrumentation uses `#[instrument]` on cache methods and `debug!` throughout the scheduler.

**No OpenTelemetry.** Searched all Cargo.toml files for `opentelemetry`, `otel`, `prometheus`, `metrics` — found nothing. The `tracing` output goes to stdout/file via the application's subscriber configuration.

### Metrics

No metrics collection. No Prometheus counters, histograms, or gauges.

### Granularity

Tracing covers: process start/stop, task push/exec, cache operations, action dispatch. No per-act latency histograms, no error rate counters, no execution trace IDs.

### Roadmap

`observability` package listed in roadmap (`README.md:560`) as `[ ] observability (plugins/obs)` — confirming this is a known gap.

**Vs Nebula:** Nebula has OpenTelemetry with one trace per workflow execution, per-action latency/count/error metrics. Acts has basic `tracing` logging only. The observability gap is significant for production use.

---

## 14. API Surface [A16]

### Programmatic (Library) API

The public API is exposed via `acts/src/export/`:
- `Engine` — create, start, close the engine
- `EngineBuilder` — builder pattern with plugin registration
- `Executor` — sub-executors for model/proc/act/task/msg/event/pack
- `Channel` / `ChannelOptions` — subscribe to workflow events
- `Extender` — register packages, collections, user vars

Key executor sub-APIs:
- `executor.model().deploy(&workflow)` — register a workflow
- `executor.proc().start("model_id", &vars)` — start a workflow instance
- `executor.act().complete/error/submit/abort/skip/back(pid, tid, &vars)` — respond to irq tasks
- `executor.msg().list/ack()` — message management

### Network API

**No built-in REST or gRPC.** The `acts-server` is a separate repository (`https://github.com/yaojianpin/acts-server`) providing gRPC. Client SDKs: Rust (`acts-channel`), Python (`acts-channel-py`), Go (`acts-channel-go`).

There is no OpenAPI spec, no HTTP server, no REST endpoints in the `acts` crate.

### Versioning

No API versioning strategy. The 0.x version range with breaking changes between minor versions (0.12.0, 0.16.0, 0.17.0 all had breaking YAML or API changes). No stability guarantees documented.

**Vs Nebula:** Nebula has REST API + planned GraphQL/gRPC with OpenAPI spec generation, OwnerId-aware per-tenant routing. Acts has a library-only API plus a separate gRPC server. Acts' gRPC approach is closer to Nebula's planned gRPC transport but is more mature (already exists, with multi-language clients).

---

## 15. Testing Infrastructure [A19]

### Unit Tests

Tests are co-located with source in `scheduler/tests/`, `model/tests/`, `store/tests/`, `cache/tests/`, `env/tests/`, `export/tests/`. 55 test files totaling a substantial test surface. Tests use `tokio::test` for async tests and standard `#[test]` for sync.

### Test Helpers

`acts/src/utils/test.rs` — contains test utility functions. `engine.signal()` pattern for test synchronization (creates a `Signal<T>` with `triple()` or `double()` for send/receive coordination in async tests).

### No Public Testing Utilities

There is no `acts-testing` crate. Test infrastructure is internal. No contract tests for plugin implementors. No integration test harness for end-to-end workflow testing.

### Coverage

The test suite covers: scheduler state machine, model parsing, store CRUD, cache behavior, export executor API, branch/step/act execution. No performance regression tests in CI (benchmark harness exists but is not required to pass in CI).

**Vs Nebula:** Nebula has a dedicated `nebula-testing` crate with public contract tests for resource implementors. Acts has no public testing utilities — plugin authors must write their own test infrastructure.

---

## 16. AI / LLM Integration [A21] ⭐ DEEP

### A21.1 Existence

**No AI/LLM integration exists.** `ActPackageCatalog::Ai` variant (`acts/src/package/mod.rs:94`) is a placeholder enum variant for future categorization. `plugins/ai` is listed in the roadmap as `[ ] ai (plugins/ai)` — not yet created.

**Grep evidence (negative findings):**
- Searched entire `acts/` workspace for `openai`, `anthropic`, `llm`, `gpt`, `claude`, `gemini`, `ollama`, `embedding`, `completion`, `langchain`, `vector` — found zero matches.
- The `acts.event.chat` package name suggests AI chat intent but its implementation (`acts/src/package/event/chat.rs`) simply starts a workflow with a string parameter — no LLM calls, no AI API.

### A21.2 Provider Abstraction

None. No provider trait.

### A21.3 Prompt Management

None.

### A21.4 Structured Output

None.

### A21.5 Tool Calling

None.

### A21.6 Streaming

None.

### A21.7 Multi-agent

None.

### A21.8 RAG/Vector

None.

### A21.9 Memory/Context

None.

### A21.10 Cost/Tokens

None.

### A21.11 Observability

None.

### A21.12 Safety

None.

### A21.13 vs Nebula+Surge

Nebula's strategic position: "AI workflows realized through generic actions + plugin LLM client. Surge (separate project) handles agent orchestration on ACP." Acts has a similar trajectory: `ActPackageCatalog::Ai` reserved, `plugins/ai` on roadmap. Neither acts nor Nebula has first-class LLM integration today. Nebula's Surge/ACP separation is architecturally more advanced; acts has no equivalent agent orchestration layer.

---

## 17. Notable Design Decisions

### Decision 1: JavaScript as the Expression Language

Acts embeds QuickJS (via `rquickjs`) for all expression evaluation and scripting. This is a strong differentiator — most Rust workflow engines use simpler DSLs or Lua. JavaScript gives workflow authors a familiar, capable language. Trade-off: QuickJS is ~1MB of compiled code, creates a new runtime per eval call (no runtime reuse observed in `acts/src/env/mod.rs:104–118`), and JavaScript evaluation adds latency compared to compiled predicate functions. Also: JavaScript errors are runtime, not compile-time — type mismatches surface only at execution.

**Applicability to Nebula:** Nebula's custom expression engine avoids the embedding overhead but has lower expressiveness. A JavaScript plugin package for Nebula's expression layer would be worth exploring as an optional "power user" expression backend.

### Decision 2: inventory-Based Compile-time Package Registration

`inventory::submit!(ActPackageRegister::new::<T>())` allows packages to self-register using linker sections. This eliminates manual registration code — adding a new built-in package is adding a `use` + the `inventory::submit!` line. Trade-off: requires compile-time knowledge of all packages; no runtime dynamic loading; increases link time marginally.

**Applicability to Nebula:** Nebula's action registration uses a different mechanism. The `inventory` pattern is elegant for built-in packages but incompatible with WASM plugins which load at runtime.

### Decision 3: Message-Driven IRQ Pattern for Human-in-the-Loop

The `acts.core.irq` pattern (interrupt request) is acts' primary mechanism for human tasks. When a workflow hits an `irq` act, it emits a message to all subscribed channels with `state: created` and pauses. External clients respond via `executor.act().complete()`. This is a clean decoupling: the workflow engine doesn't know how the human task is served. Trade-off: complexity for the client — it must track PIDs and TIDs, subscribe to the right channel pattern, and call the right executor method. No timeout-safe completion by default.

**Applicability to Nebula:** Nebula's ProcessAction with Input/Output types is more structured but less flexible for external client interaction. Acts' IRQ pattern could inspire a "pauseable action" concept for Nebula that emits typed events to external channels.

### Decision 4: Single-Crate Core

All engine logic in one crate with no intra-workspace API boundaries. This simplifies development and reduces compilation overhead. Trade-off: no enforcement of layering invariants (a new feature can accidentally introduce circular dependencies within the crate), no separate versioning for engine subsystems.

**Applicability to Nebula:** Nebula's multi-crate approach is more correct for a platform with external plugin authors. Acts' approach is only viable because acts is not yet a platform.

### Decision 5: In-Memory Default Store with Plugin Backends

The default store is `MemStore` with no durability. Users add `acts-store-sqlite` or `acts-store-postgres` plugins for persistence. This makes the engine "zero-config" for testing and development. Trade-off: easy to forget to add the persistence plugin in production. Crash without a plugin = all workflow state lost.

**Applicability to Nebula:** Nebula hard-requires PostgreSQL (via `sqlx + PgPool + PgRepo`). Acts' pluggable store is more flexible but more footgun-prone. A "dev mode" memory store for Nebula test/CI that implements the same DbCollection interface would be useful.

---

## 18. Known Limitations and Pain Points

### Issue #10 — Model Versioning (Open)
**URL:** https://github.com/yaojianpin/acts/issues/10
**Summary:** No versioning story for workflow models. Deploying a new model version while instances of the old version are running is undefined behavior. The `Workflow.ver: i32` field exists but is unused for routing or migration. This blocks production use in any system that evolves its workflows.

### Issue #12 — Process State Bug (Closed, v0.15.0)
**URL:** https://github.com/yaojianpin/acts/issues/12
**Summary:** Process state was not updated when root task completed. Bug in state machine propagation. Suggests the hand-rolled state machine has correctness risks.

### Issue #8 — Input/Output Documentation Gap (Closed)
**URL:** https://github.com/yaojianpin/acts/issues/8
**Summary:** The distinction between `inputs`, `outputs`, `params`, `options`, and `data` in act definitions is confusing. Multiple issues and changelog entries show this API has been refactored multiple times.

### Breaking API Changes (CHANGELOG observations)
- v0.12.0: YAML act format changed completely — all existing workflows needed updates.
- v0.16.0: Package system refactored — `ActPlugin`, `ActPackage`, `ActPackageFn` separation introduced.
- v0.17.0: Expression syntax changed from `${ }` to `{{ }}`; variable access functions renamed (`$act.inputs()` → `$inputs()`).

Three major breaking changes in 5 version increments signals pre-production maturity level. Acts is explicitly pre-1.0 with an unstable API.

### Missing Scheduled Triggers
The roadmap shows `schedule` event as unimplemented. Any time-based workflow (daily reports, retry queues, cleanup jobs) requires an external scheduler calling `executor.proc().start()`.

### No Distributed Safety
The scheduler is single-process. Running multiple acts instances against the same database store has no distributed coordination — double-fire and race conditions are expected. No leader election, no distributed lock, no message deduplication across instances.

---

## 19. Bus Factor / Sustainability

| Metric | Value |
|--------|-------|
| Maintainers | 1 (yaojianpin) |
| Commit cadence | Active — multiple commits per month through 2025 |
| Last release | v0.17.2 (2025) |
| Total issues | 9 (7 closed, 2 open) |
| Open/total ratio | 22% |
| Crates.io downloads | 24.4K total, 531 recent |
| Stars | ~650 |

**Bus factor: 1.** Single maintainer, no contributor list in CHANGELOG. Low issue volume suggests minimal community adoption relative to GitHub stars. The project is healthy for a solo open-source project but carries single-maintainer risk for production adoption.

---

## 20. Final Scorecard vs Nebula

| Axis | acts approach | Nebula approach | Verdict | Borrow? |
|------|--------------|-----------------|---------|---------|
| A1 Workspace | 1 core crate + 6 plugin crates, 10 Cargo.toml, no formal layers | 26 crates layered: nebula-error → nebula-resilience → … → nebula-tenant, Edition 2024 | **Nebula deeper** — crate boundaries enforce invariants acts cannot. Acts simpler to build. | no — Nebula already better |
| A2 DAG | Linear tree (Workflow→Step→Branch→Act), `NodeTree` pointer-linked, no port typing, no compile-time checks | TypeDAG L1-L4 (generics → TypeId → predicates → petgraph) | **Nebula deeper** — acts has no type safety on graph connections. | no — Nebula already better |
| A3 Action | `ActPackageFn` with `Vars` I/O, open trait, `dyn` compatible, `run_as` (Func/Irq/Msg), compile-time `inventory` registration | 5 action kinds, sealed traits, assoc Input/Output/Error, versioning, derive macros | **Nebula deeper** — type-erased Vars vs typed assoc types is fundamental difference. | refine — acts' `inventory` compile-time registration approach is elegant for built-ins. |
| A4 Credential | None (7-line `SecretsVar` JS global only) | State/Material split, LiveCredential watch(), blue-green refresh, OAuth2Protocol, DynAdapter | **Nebula far deeper** — acts has no credential subsystem. | no — different goals (acts is library, not platform) |
| A5 Resource | None (plugin authors manage own connections) | 4 scope levels, ReloadOutcome, generation tracking, on_credential_refresh | **Nebula far deeper** — acts has no resource abstraction. | no — Nebula already better |
| A6 Resilience | `Retry { times: i32 }`, tick-based timeout, message re-delivery up to N times. No CB, no bulkhead, no backoff | nebula-resilience: retry/CB/bulkhead/timeout/hedging, ErrorClassifier | **Nebula far deeper** — acts' resilience is minimal. | no — Nebula already better |
| A7 Expression | JavaScript (rquickjs/QuickJS), fresh context per eval, `$get/$set/$inputs/$data`, `secrets.*`, `$env`, `os`. Template `{{ }}` syntax | 60+ funcs, type inference, sandboxed eval, JSONPath-like `$nodes.foo.result.email` | **Different decomposition** — acts uses full JS (more power, less safety); Nebula uses custom sandboxed DSL (safer, more opinionated). | maybe — acts' QuickJS approach could inspire a "JS expression plugin" for power users in Nebula |
| A8 Storage | `DbCollection` trait, pluggable (MemStore default, SQLite/Postgres plugins), `sea-query` builder, no migrations | sqlx + PgPool, Pg*Repo, SQL migrations, RLS | **Nebula deeper for production** — RLS, migrations, pgPool management. **Acts simpler for testing** — in-memory default. | refine — acts' pluggable DbCollection interface (swap out persistence backend via ActPlugin) is worth borrowing for Nebula test/dev modes |
| A9 Persistence | Checkpointing (in-place state updates), no event sourcing, no replay. Recovery by querying store for non-terminal tasks. | Frontier-based checkpoint, append-only log, replay-based reconstruction | **Nebula deeper** — replay capability vs simple checkpoint. | no — Nebula already better |
| A10 Concurrency | tokio, single scheduler event loop (mpsc ch=100), `tokio::task_local!` for context, multiple processes via `tokio::spawn`, parallel via dynamic act injection | tokio, frontier scheduler with work-stealing, `!Send` action support | **Different decomposition** — acts uses simpler single-queue; Nebula uses work-stealing frontier. Acts' `task_local!` context propagation is interesting. | maybe — `tokio::task_local!` context threading pattern is clean; worth evaluating vs Nebula's explicit context passing |
| A11 Plugin BUILD | Rust crates, workspace members, `inventory::submit!` compile-time registration, `Cargo.toml` as manifest, no WASM | WASM sandbox planned, plugin-v2 spec, Plugin Fund commercial model | **Different goals** — acts chose simplicity (in-process Rust crates). Nebula chose isolation (WASM). | no — different goals |
| A11 Plugin EXEC | In-process, same memory space, no sandbox, message channel for external execution pattern | WASM sandbox, capability security | **Nebula more correct for platform** — acts trusts all plugins. | no — different goals |
| A12 Trigger | 3 event types (manual/hook/chat), no cron, no webhook, no external broker. Triggers are `workflow.on: Vec<Act>`. | TriggerAction: Source→Event 2-stage, typed Input=Config/Output=Event | **Nebula deeper** — 2-stage normalized Source→Event enables decoupling external inbound format from internal workflow event. Acts has 1:1 package→process dispatch. | refine — acts' `workflow.on` inline trigger declaration in YAML is elegant UX; Nebula's Source separation is more correct but has higher learning curve |
| A13 Deployment | Embedded library only. `acts-server` (separate repo) for gRPC. No multi-mode binary. | 3 modes from one binary: desktop/self-hosted/cloud | **Nebula more complete** — acts is library-only; full deployment is delegated to `acts-server`. | no — different goals |
| A14 Multi-tenancy | None. Single-tenant embedded library. | nebula-tenant: schema/RLS/database isolation, RBAC, planned SSO/SCIM | **Nebula far deeper** — acts has no tenancy concept. | no — different goals |
| A15 Observability | `tracing` crate only, no OpenTelemetry, no metrics, no per-execution trace IDs. `observability` plugin on roadmap. | OpenTelemetry per execution, per-action latency/count/error metrics | **Nebula deeper** — acts has logging only. | no — Nebula already better |
| A16 API | Rust library API (Executor/Channel/Extender), `acts-server` provides gRPC (separate repo), multi-language clients (Rust/Python/Go) | REST + planned GraphQL/gRPC, OpenAPI spec, OwnerId-aware | **Different decomposition** — acts' gRPC-first remote API via `acts-server` with multi-language SDKs is more complete than Nebula's planned gRPC. | refine — acts' multi-language client SDK approach (acts-channel-go, acts-channel-py) is worth borrowing for Nebula's gRPC transport layer |
| A17 Type safety | Open traits, type-erased `Vars`, JSON Schema parameter validation at runtime, `strum` enums | Sealed traits, GATs, HRTBs, typestate, Validated<T> proof tokens | **Nebula far deeper** — acts relies entirely on runtime JSON validation. | no — Nebula already better |
| A18 Errors | `ActError` thiserror enum (9 variants), no ErrorClass/transient-permanent distinction, `Error { ecode, message }` for user-visible errors | nebula-error + ErrorClass (transient/permanent/cancelled) | **Nebula deeper** — ErrorClass enables intelligent retry policy. Acts has no error classification. | refine — acts' `ActError::Exception { ecode, message }` pattern (structured user-visible error with code + message) is a good pattern for workflow-level errors |
| A19 Testing | 55 test files co-located, `utils/test.rs` helpers, `Signal` sync primitive for async tests. No public testing crate. | nebula-testing crate, contract tests, insta+wiremock+mockall | **Nebula deeper for plugin authors** — no public testing utilities in acts. | no — Nebula already better |
| A20 Governance | Apache-2.0, solo maintainer, no commercial model, roadmap in README, pre-1.0 API stability | Open core, Plugin Fund commercial model, planned SOC 2, solo maintainer | **Nebula more complete** — Plugin Fund is a differentiated commercial story. Acts has no commercial model. | no — different trajectory |
| A21 AI/LLM | None. `ActPackageCatalog::Ai` placeholder, `plugins/ai` on roadmap. `acts.event.chat` is naming convention only (no LLM calls). | None currently — strategic bet on generic actions + LLM plugin + Surge for agent orchestration | **Convergent** — both are AI-absent today; both have "AI as plugin" as the roadmap direction. Acts has a named catalog entry; Nebula has Surge/ACP for agent orchestration at a higher level. | maybe — acts' explicit `Ai` catalog category suggests UI-facing package organization; Nebula should consider action catalog taxonomy for the editor UI |

---

*Total rows: 22 (A11 split into BUILD + EXEC as required)*
