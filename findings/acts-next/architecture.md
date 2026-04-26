# acts-next (luminvent/acts) — Architectural Decomposition

## 0. Project Metadata

| Field | Value |
|-------|-------|
| Repo | https://github.com/luminvent/acts |
| Fork of | https://github.com/yaojianpin/acts (forked 2025-03-31) |
| Latest version | v0.17.2 (version string in workspace Cargo.toml) |
| Stars | 2 (fork) / upstream yaojianpin/acts: 61 |
| Forks of upstream | 10 |
| License | Apache-2.0 |
| Organization | Luminvent (https://github.com/luminvent) |
| Issues | Disabled on fork; upstream has 9 total, 3 open |
| Fork created | 2025-03-31 |
| Last push | 2025-09-09 (luminvent fork) |
| Luminvent contributors | Marc-Antoine ARNAUD (primary fork contributor) |
| Rust edition | 2024 |
| Pinned toolchain | None (uses stable) |
| crates.io | `acts` crate — 24.4K total downloads, 531 recent (90d) — published by yaojianpin |

---

## 1. Concept Positioning [A1, A13, A20]

**Author's description (README.md:1):** "`acts` is a fast, tiny, extensiable workflow engine"

**Researcher's description (after reading code):** acts-next is the Luminvent organization's fork of yaojianpin/acts — a single-binary Rust library that executes YAML-defined workflows using a message bus for all state transitions, a JavaScript engine (QuickJS via rquickjs) for expressions, and an `inventory`-crate compile-time package registry for extensibility. Luminvent adds ergonomic API improvements (new EventAction variants for variable mutation, naming cleanup, bug fixes) but does not redesign the core architecture.

**Comparison with Nebula:**  
- acts-next occupies the same niche as the original acts: a lightweight embeddable workflow execution library without multi-tenancy, credential management, resource lifecycle, or advanced resilience.
- Nebula targets "n8n + Temporal + Airflow merged" with 26 specialized crates enforcing layer boundaries. acts-next targets simple embedded usage with a single core crate.
- The key differentiator acts brings that Nebula lacks: JavaScript as the expression language (QuickJS), gRPC multi-language SDKs via `acts-server`, and a human-in-the-loop IRQ pattern. Nebula's type system is far more sophisticated; acts prioritizes runtime simplicity and embeddability.

---

## 2. Workspace Structure [A1]

The workspace `Cargo.toml` (root) declares 7 members:

```
acts/                      — Core engine (the single published crate)
store/sqlite/              — acts-store-sqlite: SQLite persistence plugin
store/postgres/            — acts-store-postgres: PostgreSQL persistence plugin
plugins/state/             — acts-state: Redis state package plugin
plugins/http/              — acts-http: HTTP request package plugin
plugins/shell/             — acts-shell: Shell command plugin (bash/nushell/powershell)
examples/plugins/*/        — Example plugin code (excluded from default build)
```

**Crate count:** 7 workspace members (plus 3 example crates excluded). Only `acts` is published to crates.io.

**Feature flags:** `acts/Cargo.toml` shows `[features] default = []` — no feature flags in the core crate. All functionality ships unconditionally. Store plugins have a `bundled` feature for SQLite. No conditional compilation of engine features.

**Umbrella pattern:** None. Consumers use `acts` directly.

**Workspace package version:** All members inherit `version = "0.17.2"` from `[workspace.package]` (`Cargo.toml:18`).

**Resolver:** `resolver = "3"` (Cargo edition 2024 feature resolver) — same as Nebula's edition 2024.

**vs. Nebula:** Nebula has 26 crates with enforced layer boundaries (nebula-error → nebula-resilience → nebula-credential → nebula-resource → nebula-action → nebula-engine → nebula-tenant). acts-next has 1 core crate with all logic co-located. Boundary enforcement is absent in acts — a new feature can freely introduce cross-module dependencies within the `acts` crate. Nebula's crate structure prevents credential code from depending on engine code; acts has no such constraint.

---

## 3. Core Abstractions [A3, A17] — DEEP

### Trait Hierarchy

The unit-of-work abstraction is the **package** system. Three distinct public traits form the hierarchy:

**`ActPackage` trait** (`acts/src/package/mod.rs:26–28`):
```rust
pub trait ActPackage {
    fn meta() -> ActPackageMeta;
}
```
Pure metadata trait — one associated function. No object safety, no `dyn ActPackage`.

**`ActPackageFn` trait** (`acts/src/package/mod.rs:30–40`):
```rust
#[async_trait::async_trait]
pub trait ActPackageFn: Send + Sync {
    fn execute(&self, _ctx: &Context) -> Result<Option<Vars>> { Ok(None) }
    async fn start(&self, _rt: &Arc<Runtime>, _options: &Vars) -> Result<Option<Vars>> { Ok(None) }
}
```
Execution trait. Both methods have default no-op implementations. Packages override exactly one depending on `run_as` mode. Stored as `Box<dyn ActPackageFn>` (`acts/src/package/mod.rs:116`).

**`ActPlugin` trait** (`acts/src/plugin/mod.rs`):
```rust
#[async_trait::async_trait]
pub trait ActPlugin: Send + Sync {
    async fn on_init(&self, engine: &Engine) -> crate::Result<()>;
}
```
Single-method system-level hook. Stored as `Box<dyn ActPlugin>` in `EngineBuilder`.

**`ActTask` trait** (`acts/src/scheduler/mod.rs`):
```rust
pub trait ActTask: Clone + Send {
    fn init(&self, _ctx: &Context) -> Result<()> { Ok(()) }
    fn run(&self, _ctx: &Context) -> Result<()> { Ok(()) }
    fn next(&self, _ctx: &Context) -> Result<bool> { Ok(false) }
    fn review(&self, _ctx: &Context) -> Result<bool> { Ok(true) }
    fn error(&self, ctx: &Context) -> Result<()> { ctx.emit_error() }
}
```
Internal scheduler lifecycle. Implemented by node types (Workflow, Branch, Step, Act) to drive scheduler state progression.

**`ActUserVar` trait** (`acts/src/env/mod.rs:36–47`):
```rust
pub trait ActUserVar: Send + Sync {
    fn name(&self) -> String;
    fn default_data(&self) -> Option<Vars> { None }
}
```
New in v0.17.0. Allows registering named JavaScript globals accessible in expressions.

### A3.1 Trait Shape

- **Open or sealed?** Open — `ActPackageFn` and `ActPlugin` are public, implementable by any downstream crate. No `Sealed` supertrait.
- **`dyn` compatible?** `ActPackageFn` is `Box<dyn ActPackageFn>` (`acts/src/package/mod.rs`). `ActPlugin` is `Box<dyn ActPlugin>`.
- **Associated types:** Zero on `ActPackageFn`. No associated `Input`, `Output`, `Error`, or `Config` types. I/O is universally `Vars` (a newtype over `serde_json::Map<String, Value>`).
- **GATs:** None.
- **HRTBs:** None.
- **Typestate:** None.
- **Default methods:** Both lifecycle methods on `ActPackageFn` have no-op defaults.

### A3.2 I/O Shape

- **Input:** `Vars` — a newtype `struct Vars { inner: serde_json::Map<String, Value> }` (`acts/src/model/vars.rs`). All inputs are type-erased JSON. No generic type parameters on the trait.
- **Output:** `Result<Option<Vars>>` — same type-erased map, optional.
- **Streaming output:** None. All packages return a single `Option<Vars>`.
- **Side effects model:** Packages can call `ctx.task().expose()` to propagate output keys upstream, or set process-level variables via `ctx.proc.set_data()`.

### A3.3 Versioning

- **`Workflow.ver: i32`** exists (`acts/src/model/workflow.rs:24`) but is unused for routing or migration. Used only as a record field in the stored `Event` (trigger) records.
- **Package versions** are `&'static str` in `ActPackageMeta.version` — informational only.
- **No v1/v2 dispatch.** There is no mechanism to route packages by version or migrate workflow instances across versions. Issue #10 ("Add version on a Model/Package") remains open in upstream.
- **Deployment:** `executor.model().deploy(&workflow)` creates or updates the stored model record. Redeploying the same `id` overwrites. No migration hooks.

### A3.4 Lifecycle Hooks

- **`ActPackageFn::execute`** — sync, called with `&Context` for core packages.
- **`ActPackageFn::start`** — async, called with `&Arc<Runtime>` + `&Vars` for event trigger packages.
- **`ActTask::init/run/next/review/error`** — internal scheduler lifecycle for node types.
- **`ActEvent` enum** (`acts/src/model/mod.rs`) with variants `Created/Completed/BeforeUpdate/Updated/Step` — lifecycle events on acts and steps. Setup acts (declared in `act.on` or `step.setup`) run at these points.
- **No pre/post/cleanup/on-failure** at the `ActPackageFn` level. Error handling is via `catches` blocks in the YAML model.
- **Cancellation:** `EventAction::Cancel` and `EventAction::Abort` are available. No cancellation points inside package execution itself.
- **Idempotency key:** `Act.key` field (`acts/src/model/act.rs`) — user-supplied correlation string, not engine-enforced.

### A3.5 Resource and Credential Dependencies

Packages declare no typed dependency on DB pools, credentials, or external resources. Dependencies are injected via `ActPlugin::on_init` — the plugin initializes its client and captures it in a channel callback closure. No compile-time check that a package's runtime dependencies are satisfied.

### A3.6 Retry/Resilience Attachment

`Act` model has `retry: Retry { times: i32 }` (`acts/src/model/act/retry.rs`) and `timeout: Vec<Timeout>` (`acts/src/model/act/timeout.rs`). Retry is configured per-act in YAML as an integer count. No backoff, no jitter, no exponential increase. Timeout is expressed in human-readable form (`1d`, `2h`, `30m`, `60s`) and triggers a sub-steps sequence. No circuit breaker, no bulkhead.

### A3.7 Authoring DX

A custom package requires: implement `ActPackage::meta()` + `ActPackageFn::execute()` or `start()` + `inventory::submit!(ActPackageRegister::new::<T>())`. Approximately 35-40 lines for a minimal package. No derive macros from acts itself. No CLI scaffolding. JSON Schema validation of params is automatic via `ActPackageRegister::new::<T>()` which embeds the `jsonschema::validate` call in the `create` function.

### A3.8 Metadata

`ActPackageMeta` (`acts/src/package/mod.rs:68+`): `name: &'static str`, `desc: &'static str`, `icon: &'static str`, `doc: &'static str`, `version: &'static str`, `schema: serde_json::Value`, `run_as: ActRunAs`, `resources: Vec<ActResource>`, `catalog: ActPackageCatalog`. All compile-time static strings. No i18n. No runtime override. The `resources` field (name/desc/operations) targets a visual workflow editor's package browser.

`ActPackageCatalog` variants: `Core | Event | Transform | Form | Ai | App`. The `Ai` variant is a placeholder — no AI packages exist yet.

### A3.9 Comparison with Nebula

Nebula has **5 action kinds** (ProcessAction / SupplyAction / TriggerAction / EventAction / ScheduleAction) with **sealed traits**, **associated Input/Output/Error types**, **versioning via type identity**, and **derive macros via nebula-derive**. acts-next has **one execution trait** (`ActPackageFn`) with **type-erased Vars I/O**, **no versioning**, and **no derive macros**.

The `ActRunAs` enum (`Func/Irq/Msg`) provides a rough functional analog to Nebula's 5 action kinds but is far less expressive:
- `Irq` ≈ ProcessAction (interrupt, wait for client response)
- `Msg` ≈ EventAction (fire-and-forget notification)
- `Func` ≈ internal utility (no client visibility)

acts-next has no equivalent to Nebula's SupplyAction (data provision), TriggerAction (workflow start via typed Source), or ScheduleAction (cron/timer).

The fundamental design difference: Nebula's type system catches I/O mismatches at compile time; acts defers all type checking to JSON Schema validation at runtime registration.

### Notable New Abstractions vs. Original acts (Luminvent additions)

**`SetVars` / `SetProcessVars` EventAction** (`acts/src/event/mod.rs:EventAction`):
Two new variants added by Luminvent: `SetVars` (mutate task-level data without completing the task) and `SetProcessVars` (mutate process-level data without completing the task). Exposed via `executor.act().set_task_vars(pid, tid, &vars)` and `executor.act().set_process_vars(pid, tid, &vars)` (`acts/src/export/executor/act_executor.rs`). This fills a key gap: previously, callers could only mutate workflow state by completing or erroring a task, which forced state changes to also advance the workflow. Now callers can accumulate intermediate results on a long-running IRQ task.

**`ActPackageRegister::new::<T>()` const fn** (`acts/src/package/mod.rs`): The original acts used a dynamic `fn` pointer for package creation. The current code uses a const fn constructor pattern, enabling compile-time registration via `inventory::submit!`. (This was part of v0.16.0 refactoring by yaojianpin, adopted in the fork.)

---

## 4. DAG / Execution Graph [A2, A9, A10]

### Graph Description

acts-next workflows are **linear trees with branch divergence**, not general DAGs. The `NodeTree` (`acts/src/scheduler/tree/node_tree.rs`) holds a rooted tree where each `Node` has `children`, `prev`, `next`, and `parent` pointers. The node types form a strict hierarchy:

```
NodeKind (acts/src/scheduler/tree/node.rs):
  Workflow | Branch | Step | Act
```

- `Workflow` → has `Vec<Step>` or `Vec<Act>` (via `on`)
- `Step` → has `Vec<Branch>` and/or `Vec<Act>`
- `Branch` → has `Vec<Step>` and a `Vec<String> needs` field for join semantics
- `Act` → leaf nodes

`Branch.needs: Vec<String>` (`acts/src/model/branch.rs`) enables join semantics — a branch with `needs: [other_branch_id]` waits in `TaskState::Pending` until all listed branches complete (`acts/src/scheduler/process/task/branch.rs:12–15`). This is the only non-linear control flow construct — a limited form of DAG join without arbitrary edge connections.

`Step.next: Option<String>` allows explicit step sequencing by ID rather than positional ordering.

### Port Typing

None. There are no typed ports. Data flows through the global `Vars` map — all act inputs and outputs share the same key-value namespace. `inputs:` and `outputs:` declarations in YAML are conventions validated via `Outputs::check()` (`acts/src/model/output.rs`) which enforces required fields and basic type matching (`String/Bool/Number/Array/Object`). This is a minor improvement over untyped JSON but not compile-time type safety.

### Compile-time vs Runtime Checks

No compile-time checks on workflow structure. `NodeTree::load()` validates the tree at deploy time. Condition expressions (`if: value > 100`) are compiled and evaluated at runtime via rquickjs. JSON Schema validates package params at registration time, not at workflow definition time.

### Scheduler Model

Single-threaded event loop with a tokio mpsc channel. The `Scheduler` has a bounded channel (`acts/src/scheduler/queue/queue.rs`). `Runtime::event_loop()` is spawned as a separate tokio task. Each task is executed by calling `task.exec(ctx)` within that loop iteration.

`Process` instances are cached in a Moka LRU cache (default cap 1024 — `acts/src/config.rs:config.cache_cap()`). When a process is evicted, it is stored to the backing store. On `cache.restore()`, processes are reloaded and re-enqueued.

### Concurrency

Multiple processes run concurrently via `tokio::spawn` — `Runtime::launch()` spawns each process start. Within a process, tasks execute sequentially via the single scheduler loop. `acts.core.parallel` creates sibling Act nodes dynamically to achieve per-process parallelism. `acts.core.block` with `mode: parallel` or `mode: sequence` (`acts/src/package/core/block.rs`) is the generalized block-level parallelism primitive.

`tokio::task_local! static CONTEXT: Context` (`acts/src/scheduler/context.rs:15–16`) propagates the execution context implicitly within sync execution scope. Context does not flow across `await` points — it is scoped to the synchronous execution of `task.exec()`.

**vs Nebula:** Nebula has TypeDAG with 4 levels (static generics, TypeId, refinement predicates, petgraph soundness checks). acts-next has no compile-time DAG type safety and uses a pointer-linked tree traversal. Nebula's frontier-based scheduler with work-stealing semantics vs acts' single-queue sequential scheduler with explicit parallelism packages.

---

## 5. Persistence and Recovery [A8, A9]

### Storage Layer

The `DbCollection` trait (`acts/src/store/mod.rs`) defines a generic CRUD interface:
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

Six collection types: `Tasks`, `Procs`, `Models`, `Messages`, `Events`, `Packages` (`acts/src/store/mod.rs:StoreIden`). **New vs. original acts:** `Events` collection (for trigger event records) was added in v0.16.0.

Default backend: `MemStore` — `HashMap`-based in-memory store with no durability. Optional plugins: `acts-store-sqlite` (using `rusqlite` + `r2d2` + `sea-query`) and `acts-store-postgres` (using `sqlx` + async `sea-query`). The postgres plugin uses `sqlx::PgPool` with a connection pool.

`DbCollectionIden` trait (`acts/src/store/mod.rs`) is a new type marker introduced in v0.16.0 for type-safe collection identity, allowing `Extender::register_collection<DATA>()` to be generic over the stored type.

### Persistence Model

**Checkpointing**, not event sourcing. Each state transition updates the task/process record in-place. There is no append-only execution log.

Recovery:
1. `cache.restore()` queries the store for in-progress `Proc` records.
2. For each `Proc`, its serialized `NodeTree` (stored as JSON in `data::Proc.tree`) is deserialized.
3. Tasks with non-terminal states are re-enqueued.

`data::Task` (`acts/src/store/data/task.rs`) stores `node_data: String` (JSON of `NodeData`), `state: String`, `data: String` (JSON of `Vars`), `hooks: String` (JSON of lifecycle hooks). This is a snapshot-based approach — no full history.

### Migrations

No migration infrastructure. SQLite plugin uses `CREATE TABLE IF NOT EXISTS` at init time. Postgres plugin runs CREATE TABLE statements via `sea-query`. No versioned migrations, no rollback support.

**vs Nebula:** Nebula uses frontier-based checkpointing with an append-only execution log, enabling state reconstruction via replay. acts-next uses simpler in-place update checkpointing. Nebula has sqlx migrations with versioned SQL files; acts has no migration infrastructure.

---

## 6. Credentials / Secrets [A4] — DEEP

### A4.1 Existence

**No dedicated credential layer exists.** acts-next does not have a `Credential` type, `CredentialOps` trait, or credential storage.

**Grep evidence (negative findings):**
- Searched entire workspace for `credential`, `oauth`, `vault`, `keychain`, `token` in a credential sense:
  ```
  grep -r "credential\|oauth\|vault" --include="*.rs" -l
  ```
  Result: only `acts/src/env/moudle/vars/secrets.rs` (7-line `SecretsVar` that exposes a JS global) and incidental `token` appearances in string fields.
- No `Zeroize`, no `secrecy::Secret<T>` in Cargo.toml.
- No vault integration (HashiCorp Vault, AWS Secrets Manager, etc.).

The `ActUserVar` trait (`acts/src/env/mod.rs:36`) added in v0.17.0 provides a generic interface for named JS globals, of which `SecretsVar` is one implementation. This is the only "credential-adjacent" concept.

### A4.2 Storage

No credential storage. Secrets are passed as plain `Vars` at workflow start time.

### A4.3 In-memory Protection

No `Zeroize`, no `secrecy::Secret<T>`. Secrets passed via `Vars` (i.e., `serde_json::Map<String, Value>`) remain in heap memory as plain strings until GC.

### A4.4 Lifecycle

No lifecycle. Secrets are scoped to a single process invocation. No refresh, no rotation, no revocation.

### A4.5 OAuth2/OIDC

No OAuth2 support. Searched `Cargo.toml` files for `oauth`, `oidc`, `openid` — found nothing.

### A4.6 Composition

Single flat namespace. All secrets available as `secrets.FIELD` JS global.

### A4.7 Scope

Per-process invocation only. Secrets are provided at `executor.proc().start("model_id", &vars)` time.

### A4.8 Type Safety

No type safety for secrets. Everything is `serde_json::Value`.

### A4.9 vs Nebula

Nebula has: State/Material split, LiveCredential with `watch()` for blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter type erasure, encrypted storage, Zeroize protection. acts-next has: a 7-line `SecretsVar` that exposes a JavaScript global. This is the largest single capability gap vs Nebula.

**Luminvent delta:** No change to credential handling from original acts. The fork adds no credential layer.

---

## 7. Resource Management [A5] — DEEP

### A5.1 Existence

**No dedicated resource abstraction.** There is no `Resource` trait, no pool management, no lifecycle-managed DB or HTTP client as a first-class engine concept.

**Grep evidence (negative findings):**
- Searched `acts/src/` for `resource`, `pool`, `lifecycle`, `scope`, `reload`:
  ```
  grep -r "resource\|pool\|lifecycle\|scope\|reload" --include="*.rs" -l (acts/src/)
  ```
  The word "resources" only appears in `ActPackageMeta.resources: Vec<ActResource>` — UI metadata (name/description/operations for editor display), not runtime resource management.

### A5.2 Scoping

No scoping levels. Resource-like objects (e.g., a Redis client in the state plugin) are created once during `ActPlugin::on_init`, captured in closures registered on the channel, and shared via `Arc::clone`. Scope is effectively "global for the process lifetime."

### A5.3 Lifecycle Hooks

`ActPlugin::on_init` is the only lifecycle hook. No `on_shutdown`, no `health_check`. Init failure causes `EngineBuilder::build()` to return `Err`.

### A5.4 Reload

No hot-reload. No blue-green. No generation tracking. Reloading requires engine restart.

### A5.5 Sharing

Resources shared via `Arc::clone` inside closure capture. The SQLite store uses `r2d2` connection pool from `r2d2_sqlite` — this is the plugin's own concern, not a framework facility.

### A5.6 Credential Dependencies

Resources cannot declare credential dependencies — there are no credentials in the system.

### A5.7 Backpressure

No resource acquisition backpressure. The scheduler task queue channel is bounded (default cap `config.cache_cap()` → 1024 processes), providing implicit backpressure at the process level but not at the resource level.

### A5.8 vs Nebula

Nebula has: 4 scope levels (Global/Workflow/Execution/Action), ReloadOutcome enum, generation tracking, `on_credential_refresh` per-resource hook. acts-next has: none of these. Resources are implicit — plugin authors manage their own external connections.

---

## 8. Resilience [A6, A18]

### Error Handling

`ActError` (`acts/src/error.rs`) is a `thiserror::Error`-derived enum with 10 variants: `Config`, `Convert`, `Script`, `Exception { ecode, message }`, `Model`, `Runtime`, `Store`, `Action`, `IoError`, `Package`. No `ErrorClass` categorization (no transient/permanent/retryable distinction). All errors are treated equally.

`Error { ecode: String, message: String }` is the user-visible workflow error type.

### Retry

`Act.retry.times: i32` (`acts/src/model/act/retry.rs`) provides a per-act retry count. No backoff strategy, no jitter, no exponential increase. Zero retry by default.

Message delivery retry: `config.max_message_retry_times` (default 20) with `tick_interval_secs` (default 15s). The tick handler resends unacked messages. No per-message backoff.

### Circuit Breaker

No circuit breaker. Searched all Rust files for `circuit`, `breaker`, `bulkhead`, `hedg` — found nothing.

### Timeout

`Step.timeout: Vec<Timeout>` and `Act.timeout: Vec<Timeout>` — timeout rules in human-readable durations (`1d`, `2h`, `30m`, `60s`). When a timeout fires, a set of `steps` executes. Implementation is in the tick handler which checks elapsed time at each tick interval (`config.tick_interval_secs()`).

**vs Nebula:** Nebula has `nebula-resilience` crate: retry with backoff + jitter, circuit breaker, bulkhead, timeout, hedging, unified `ErrorClassifier`. acts-next has basic retry count, a tick-based timeout, and message re-delivery — no sophisticated resilience patterns.

---

## 9. Expression / Data Routing [A7]

### DSL

acts-next uses **JavaScript** as its expression and scripting language, embedded via `rquickjs` (QuickJS binding, `acts/Cargo.toml`). This is the primary differentiator compared to most Rust workflow engines.

**Expression contexts:**
- `if:` conditions on Step/Branch/Act — JavaScript boolean expression evaluated at runtime.
- `acts.transform.code` package — runs arbitrary JavaScript with access to all workflow variables.
- `{{ variable }}` template syntax in shell plugin parameters.

### JavaScript Runtime Architecture

`Enviroment` (`acts/src/env/mod.rs`) holds a list of `ActModule` implementations. On each `eval()` call, a new `JsRuntime` + `JsContext` are created fresh (`rquickjs::Runtime::new()` and `JsContext::full()`), modules are initialized, and the expression is evaluated. **No runtime reuse** — a fresh QuickJS context per evaluation call. This trades startup overhead for isolation.

Built-in JS globals exposed via modules (`acts/src/env/moudle/mod.rs`):
- `console` — logging
- Arrays utilities
- `$inputs()`, `$data()` — act input/data access functions
- `$get(name)`, `$set(name, val)` — variable get/set
- `$set_process_var(name, val)` — process-level variable mutation
- `$step` — per-step variable access (new in v0.17.0)
- `$env` — workflow environment variables
- `secrets.FIELD` — secrets (user-provided data)
- `os` — operating system information (new in v0.17.0)

**New in v0.17.0:** `{{ }}` template syntax replaces `${ }`. Direct variable name access in scripts instead of `$("var")`. Renamed `$act.inputs()` → `$inputs()`, `$act.data()` → `$data()`.

**`ActUserVar` trait** (`acts/src/env/mod.rs:36–47`): Introduced in v0.17.0 to allow plugin authors to register named JS global namespaces. Users call `engine.extender().register_var(&my_module)` to add a custom global. This generalizes the `SecretsVar` pattern into an extensible hook.

### Sandbox

rquickjs/QuickJS has limited filesystem/network access by default, but acts does not explicitly configure resource limits or a strict capability sandbox. CPU/memory limits are not set. `acts.transform.code` executes arbitrary JavaScript. `acts.app.shell` (added in v0.17.0) executes arbitrary shell commands (bash/nushell/powershell) — explicit escape hatch with no sandboxing.

**vs Nebula:** Nebula has a custom expression engine with 60+ functions, type inference, and JSONPath-like `$nodes.foo.result.email` syntax. acts-next uses JavaScript which is more powerful but less type-safe. Nebula's expression engine is sandboxed by design; acts' shell plugin bypasses all sandboxing.

---

## 10. Plugin / Extension System [A11] — DEEP

### 10.A — Plugin BUILD Process

**A11.1 Format:** Rust crates in the workspace. No binary artifact format (no .tar.gz, no OCI, no WASM blob). Plugins compile as regular Rust crates without `cdylib`. Manifest format: `Cargo.toml`. No custom plugin manifest schema beyond `ActPackageMeta`.

**A11.2 Toolchain:** Standard cargo build. All plugins compile in the same workspace, same Rust toolchain, same edition (2024). No cross-compilation required for local plugins. No SDK beyond implementing the `ActPlugin` and/or `ActPackageFn` traits.

**A11.3 Manifest Content:** `ActPackageMeta` (`acts/src/package/mod.rs`) is the "manifest" for package plugins:
- `name: &'static str` — dot-namespaced (e.g., `"acts.core.irq"`)
- `schema: serde_json::Value` — JSON Schema for parameters
- `run_as: ActRunAs` — `Func | Irq | Msg`
- `resources: Vec<ActResource>` — UI metadata for editor
- `catalog: ActPackageCatalog` — `Core | Event | Transform | Form | Ai | App`

No capability declarations, no permission grants, no network/fs/crypto requirements in the manifest.

**A11.4 Registry/Discovery:** Two mechanisms:
1. **Compile-time registration** via `inventory::submit!(ActPackageRegister::new::<T>())` (`acts/src/package/mod.rs`). The `inventory` crate uses linker sections to collect all registrations at startup. Built-in packages use this.
2. **Runtime registration** via `engine.extender().register_package(&meta)` (`acts/src/export/extender.rs`). External plugins call this from `ActPlugin::on_init`. Only registers metadata — execution logic is handled by the plugin listening on the channel.

No remote registry. No signing. No version pinning for plugin compatibility. `Extender::register_package()` enforces that `run_as != Func` (line: `if meta.run_as == ActRunAs::Func { return Err(...) }`) — Func packages are engine-internal only.

### 10.B — Plugin EXECUTION Sandbox

**A11.5 Sandbox Type:** **None.** Plugins execute in the same process and memory space as the engine. No process isolation, no WASM sandbox, no IPC. Built-in packages receive a `&Context` reference and execute directly on the tokio runtime. The external plugin pattern (http, state, shell) subscribes to channel messages and spawns its own tokio tasks:

```rust
// plugins/http/src/lib.rs
chan.on_message(move |e| {
    tokio::spawn(async move {
        // HTTP request executes here, same process
        executor.act().complete(&pid, &tid, &data).unwrap();
    });
});
```

**A11.6 Trust Boundary:** Plugins are fully trusted. No capability-based security. No CPU/memory/wall-time limits. A buggy plugin can crash or deadlock the engine.

**A11.7 Host↔plugin Calls:** For built-in packages: `execute(&ctx)` direct Rust function call. For external plugins: message passing via `Channel`. Plugin listens for `on_message`, executes logic, calls `executor.act().complete/error()` to return results. Marshaling is via `Vars` (JSON map). No protobuf, no WIT, no wit-bindgen. The channel is async (tokio mpsc internally) but the plugin callback is a sync fn pointer that spawns its own tokio task for async work.

**A11.8 Lifecycle:** `ActPlugin::on_init` is called once at `EngineBuilder::build()`. No explicit start/stop/reload. No crash recovery — if a plugin's tokio task panics, the panic is silently swallowed (tokio spawn default). No `on_shutdown`.

**A11.9 vs Nebula:** Nebula targets WASM sandbox with wasmtime, capability-based security, and a commercial Plugin Fund model (royalties to plugin authors). acts-next has no sandbox, no capability security, no commercial plugin model. acts' plugin system is simpler and pragmatic for trusted-environment use cases but unsuitable for a marketplace/fund model.

---

## 11. Trigger / Event Model [A12] — DEEP

### A12.1 Trigger Types

Triggers in acts-next are `Act` instances in `workflow.on: Vec<Act>` (`acts/src/model/workflow.rs:24`). Three built-in trigger types:

| Package name | Type | Status |
|---|---|---|
| `acts.event.manual` | Manual/programmatic start | Implemented |
| `acts.event.hook` | Hook (blocks caller until workflow complete) | Implemented |
| `acts.event.chat` | Chat event (starts workflow with string param) | Implemented |
| `acts.event.schedule` | Cron/interval | **Roadmap — not implemented** |

**Webhook:** Not natively supported. `acts-server` (separate repo) is the gateway.  
**External events (Kafka/RabbitMQ/NATS/Redis Streams):** Not supported.  
**FS watch, DB CDC, polling:** Not supported.  
**Internal events:** `acts.core.msg` (fire-and-forget), `setup` acts on node lifecycle events.  
**Manual:** `acts.event.manual` — starts the workflow programmatically.

### A12.2 Webhook

No webhook support built-in. `acts-server` (separate repo) provides a gRPC interface usable as a webhook gateway with an external HTTP adapter. No URL allocation, no HMAC verification, no idempotency key at the acts level.

### A12.3 Schedule

**Roadmap only.** Searched `acts/src/` for `cron`, `schedule`, `interval`, `timer` — found only timeout duration parsing (`acts/src/model/act/timeout.rs`) for step timeouts, not workflow scheduling. `README.md` roadmap shows `[ ] schedule` under event packages.

### A12.4 External Event

Not implemented. The `acts.event.hook` pattern is closest to an external event trigger — it blocks the caller until the triggered workflow completes via `Signal`. This is synchronous invocation, not async event ingestion from a broker.

**New `EventExecutor`** (`acts/src/export/executor/event_executor.rs`): Added in v0.16.0. Provides `list()`, `get()`, and `start(event_id, params)` methods for managing deployed trigger events. `start()` looks up the event record in the store, retrieves the registered package, and calls `package.start()`. This is a persistence layer for trigger registrations — events are stored as `data::Event` records tied to workflow IDs and versions.

### A12.5 Reactive vs Polling

Default: reactive via message channel. The engine emits messages on state transitions; clients subscribe via `engine.channel().on_message()`. No polling loop for triggers.

### A12.6 Trigger→Workflow Dispatch

1:1 mapping. Each `workflow.on` event starts one process. Fan-out requires multiple `on` declarations. Trigger metadata is passed as `params` to `ActPackageFn::start()` which becomes process inputs. No conditional triggers. No replay support.

### A12.7 Trigger as Action

Trigger packages (`acts.event.manual`, `acts.event.hook`, `acts.event.chat`) implement `ActPackageFn::start()` (async). Their `run_as: ActRunAs::Func` — they are Func-type packages that trigger workflow starts. They appear in `workflow.on: Vec<Act>` not in workflow steps. Lifecycle: fire-once on invocation. The `hook` variant blocks until the triggered workflow completes.

**New: `EventInfo` / `EventExecutor`** allows querying registered trigger events — `executor.evt().list(q)` returns `PageData<EventInfo>` with event `id`, `name`, `mid` (model id), `ver`, `uses`, and `params`. This enables UIs to discover what workflows can be triggered and how.

### A12.8 vs Nebula

Nebula has a 2-stage model: `Source` trait normalizes raw inbound (HTTP request, Kafka message, cron tick) → typed `Event` → `TriggerAction` (one of 5 action kinds with typed `Input = Config` / `Output = Event`). acts-next has a 1-stage model: trigger packages directly start workflow instances with untyped `Vars`. Nebula's Source separation enables decoupling external inbound format from internal workflow event; acts' approach is simpler but less composable. acts-next added `EventExecutor` for trigger discovery (list/get), which is a useful UX feature Nebula's trigger system lacks.

---

## 12. Multi-tenancy [A14]

**No multi-tenancy support.** acts-next is a single-tenant embedded library. There is no `Tenant` type, no schema isolation, no RLS, no RBAC, no SSO, no SCIM.

**Grep evidence:** Searched all workspace members for `tenant`, `rbac`, `permission`, `role`, `access control`, `schema isolation` — found nothing.

Workflows are identified by model ID and process ID. All processes share the same store collections. No user identity concept exists in the core engine — user identity is a convention implemented via `secrets.user_id` pattern at the application layer.

**vs Nebula:** Nebula has `nebula-tenant` crate with three isolation modes (schema/RLS/database), RBAC, planned SSO/SCIM. acts-next has no equivalent at any level.

---

## 13. Observability [A15]

### Tracing

acts-next uses the `tracing` crate (`acts/Cargo.toml`) for structured logging. `#[instrument]` decorates cache and key scheduler methods. `debug!`/`info!`/`error!` calls throughout.

**No OpenTelemetry.** Searched all Cargo.toml files for `opentelemetry`, `otel`, `prometheus`, `metrics` — found nothing. The `observability` plugin (`plugins/obs`) is on the roadmap as `[ ] observability (plugins/obs)`.

### Metrics

No metrics collection. No Prometheus counters, histograms, or gauges.

### Granularity

Tracing covers: process start/stop, task push/exec, cache operations, action dispatch. No per-act latency histograms, no error rate counters, no execution trace IDs.

**vs Nebula:** Nebula has OpenTelemetry with one trace per workflow execution, per-action latency/count/error metrics. acts-next has basic `tracing` logging only.

---

## 14. API Surface [A16]

### Programmatic (Library) API

The public API (`acts/src/export/`):
- `Engine` — create, start, close the engine
- `EngineBuilder` — builder pattern with plugin registration
- `Executor` — sub-executors: `msg()`, `act()`, `model()`, `proc()`, `task()`, `pack()`, `evt()`
- `Channel` / `ChannelOptions` — subscribe to workflow events (`on_start`, `on_message`, `on_complete`, `on_error`)
- `Extender` — `register_var()`, `register_package()`, `register_collection()`

**New executor methods added by Luminvent:**
- `executor.act().set_task_vars(pid, tid, &vars)` — SetVars action
- `executor.act().set_process_vars(pid, tid, &vars)` — SetProcessVars action
- `executor.act().do_action(pid, tid, action, &vars)` — raw action dispatch (exposed as public method)
- `executor.evt().list(q)` / `.get(id)` / `.start(event_id, params)` — event management

**New data types exposed:**
- `EventInfo` — trigger event record
- `PageData<T>` — pagination result (now public, previously internal)

### Network API

**No built-in REST or gRPC.** The `acts-server` is a separate repository providing gRPC. Client SDKs: Rust (`acts-channel`), Python (`acts-channel-py`), Go (`acts-channel-go`).

### Versioning

No API versioning strategy. The 0.x version range with breaking changes between minor versions. No stability guarantees documented.

**vs Nebula:** Nebula has REST API + planned GraphQL/gRPC with OpenAPI spec generation, OwnerId-aware per-tenant routing. acts-next has a library-only API plus a separate gRPC server. acts' multi-language gRPC clients are more mature than Nebula's planned gRPC transport.

---

## 15. Testing Infrastructure [A19]

Tests are co-located with source in `scheduler/tests/`, `model/tests/`, `store/tests/`, `cache/tests/`, `env/tests/`, `export/tests/`. ~65 test files totaling approximately 228 Rust source files. Tests use `tokio::test` for async and standard `#[test]` for sync.

`acts/src/utils/test.rs` — test utility functions. `Signal<T>` with `triple()` / `double()` patterns for async test synchronization.

No `acts-testing` crate. No public testing utilities. No contract tests for plugin implementors.

**vs Nebula:** Nebula has a dedicated `nebula-testing` crate with public contract tests for resource implementors. acts-next has no public testing utilities — plugin authors must write their own test infrastructure.

---

## 16. AI / LLM Integration [A21] — DEEP

### A21.1 Existence

**No AI/LLM integration exists.** `ActPackageCatalog::Ai` variant (`acts/src/package/mod.rs`) is a placeholder with comment "AI related for LLMs". `plugins/ai` is listed in the roadmap as `[ ] ai (plugins/ai)` — not yet created.

**Grep evidence (negative findings):**
- Searched entire workspace for `openai`, `anthropic`, `llm`, `gpt`, `claude`, `gemini`, `ollama`, `embedding`, `completion`, `langchain`, `vector`:
  ```
  grep -r "openai|anthropic|llm|gpt|claude|gemini|ollama|embedding|completion|langchain|vector" --include="*.rs" -l
  ```
  Result: only `acts/src/package/mod.rs:93` — the comment "AI related for LLMs" in the `ActPackageCatalog::Ai` variant. Zero actual AI API calls.
- The `acts.event.chat` package name suggests AI chat intent but its implementation (`acts/src/package/event/chat.rs`) simply starts a workflow with string parameters — no LLM calls, no AI API.

**Luminvent delta:** No AI additions. The fork adds no LLM capabilities.

### A21.2–A21.12

All nil. No provider abstraction, no prompt management, no structured output, no tool calling, no streaming, no multi-agent, no RAG/vector, no memory/context, no cost/token tracking, no observability, no safety features.

### A21.13 vs Nebula+Surge

Nebula's strategic position: "AI workflows realized through generic actions + plugin LLM client. Surge (separate project) handles agent orchestration on ACP." acts-next has a similar trajectory: `ActPackageCatalog::Ai` reserved, `plugins/ai` on roadmap. Neither acts-next nor Nebula has first-class LLM integration today. Nebula's Surge/ACP separation is architecturally more advanced; acts has no equivalent agent orchestration layer.

---

## 17. vs Original acts — Diff Analysis

This section is the core research value for acts-next. The luminvent fork diverged from yaojianpin/acts at commit 0.13.3 (2024-03-27, tagged in the Luminvent fork timeline). The following changes are unique to the luminvent fork, based on `git log --format="%as %an %s" | grep "Marc-Antoine\|luminvent"`:

### Confirmed Luminvent Changes

1. **`SetVars` / `SetProcessVars` EventAction** (Marc-Antoine ARNAUD, 2025-04-01 and 2025-08-29):
   - `EventAction::Update` (luminvent's original name) was later renamed `SetProcessVars` in yaojianpin/acts as well.
   - `SetVars` (mutate task-level data without completing) was added by Marc-Antoine ARNAUD on 2025-08-29 (`feat: add SetVars action`) — this is a Luminvent-only addition not yet in upstream.
   - Exposed via `executor.act().set_task_vars()` and `executor.act().set_process_vars()`.

2. **`do_action` public method** (`acts/src/export/executor/act_executor.rs:do_action`):
   - Marc-Antoine ARNAUD: "feat: expose do_action method" (2025-04-02). Allows callers to dispatch arbitrary `EventAction` variants without going through named convenience methods.

3. **`keep_processes` config flag** (`acts/src/config.rs:keep_processes`):
   - Marc-Antoine ARNAUD: "feat: allow to keep processes after completion" (2025-04-23). When `true`, process and task records are not deleted from the store after workflow completion — useful for inspection and debugging.

4. **Bug fix: Process state propagation** (Marc-Antoine ARNAUD, 2025-04-23):
   - "fix: set process state if task is completed and is root task" — addresses upstream Issue #12. Applied before yaojianpin accepted the same fix.

5. **API naming cleanup** (2025-03-30–04):
   - Renamed internal module `sch` → `scheduler`, `proc` → `process`.
   - Used `strum` for EventAction string conversion (`strum::AsRefStr`, `strum::EnumString`).
   - These are cosmetic/ergonomic but improve the `do_action` string-based dispatch path.

6. **`PageData<T>` public export** (2025-04-10):
   - "feat: expose PageData from store" — `PageData<T>` was internal; now public so external query results can be typed.

### What Was NOT Changed (Identical to Upstream)

The following critical architectural components are **identical** to yaojianpin/acts:
- Core trait hierarchy (`ActPackageFn`, `ActPackage`, `ActPlugin`, `ActTask`)
- Type-erased `Vars` I/O
- `inventory::submit!` compile-time package registration
- JavaScript expression engine (rquickjs/QuickJS)
- No credential layer (7-line `SecretsVar` JS global)
- No resource lifecycle
- No multi-tenancy
- No AI/LLM integration
- No resilience patterns beyond basic retry count
- No observability beyond `tracing` logging
- `acts-server` (separate gRPC repo) for network access
- Single-process scheduler, no distributed coordination

### Interpretation

The luminvent/acts fork is an **operational fork** — Luminvent is actively using acts as a dependency for their workflow needs and upstreaming ergonomic improvements and bug fixes back to yaojianpin/acts. The fork is not a redesign or successor in any architectural sense. Key evidence:
- Luminvent's commits were merged back to upstream: `git log | grep "Merge branch 'luminvent-main'"` shows two such merges in yaojianpin/acts.
- The `SetVars` addition (2025-08-29) is the most recent Luminvent-unique feature and addresses a practical gap: being able to update task data mid-workflow without forcing completion.
- Version string in the Luminvent fork remains `0.17.2` (same as upstream) — no independent release cadence.

**The naming "acts-next" (from the research assignment) appears to be a researcher-assigned label, not an official project name.** The repo is simply `luminvent/acts`.

---

## 18. Notable Design Decisions

### Decision 1: JavaScript as the Expression Language (Inherited from upstream)

acts-next embeds QuickJS (via `rquickjs`) for all expression evaluation and scripting. This gives workflow authors a familiar, capable language. Trade-off: a fresh QuickJS context per evaluation (no runtime reuse in `acts/src/env/mod.rs`) adds latency. JavaScript errors are runtime, not compile-time. The `acts.app.shell` plugin (added v0.17.0) is a direct escape hatch from any safety model — it executes arbitrary shell commands.

**Applicability to Nebula:** Nebula's custom expression engine avoids embedding overhead but has lower expressiveness. A "JavaScript expression plugin" for Nebula as an optional power-user backend would be valuable.

### Decision 2: SetVars/SetProcessVars — Mid-workflow State Mutation (Luminvent addition)

The addition of `SetVars` and `SetProcessVars` EventActions enables callers to update workflow variables on a running (non-completed) IRQ task. This solves the common pattern of accumulating results before a human decision (e.g., uploading supporting documents, adding approver comments) without forcing premature task completion.

**Applicability to Nebula:** Nebula's `ProcessAction` pattern (complete with typed Output) implicitly requires completion to return data. A parallel "accumulate partial data" operation on an in-progress pauseable action would be useful for multi-step human tasks.

### Decision 3: ActUserVar Trait — Extensible JS Global Namespace (v0.17.0)

The `ActUserVar` trait allows plugin authors to inject named JavaScript globals (like `secrets`, `my_app`) into the expression evaluation context. This is a clean extensibility hook that enables application-specific context without polluting the global namespace or hardcoding all globals in the engine.

**Applicability to Nebula:** Nebula's expression engine has a fixed function/variable set. A similar plugin hook for adding custom expression functions or context namespaces would enable embedding applications to provide domain-specific expression capabilities.

### Decision 4: EventExecutor + Event Store (v0.16.0)

Storing deployed trigger events (`data::Event`) as persistent records with their own collection enables `executor.evt().list()` queries — UIs can discover what triggers are registered and what parameters they expect. This is a first step toward a trigger catalog/registry.

**Applicability to Nebula:** Nebula's TriggerAction and Source abstractions are strongly typed but not easily queryable from external UIs. A trigger catalog endpoint that exposes registered triggers with their config schema would improve DX for workflow builders.

### Decision 5: keep_processes Config Flag (Luminvent addition)

Retaining process/task records post-completion (configurable via `acts.toml` `keep_processes = true`) enables post-run inspection without requiring a separate audit log or append-only store. Trade-off: store growth over time without cleanup.

**Applicability to Nebula:** Nebula's append-only execution log already supports full history. The `keep_processes` pattern is more relevant for acts' simpler checkpoint model where completed records would otherwise be deleted.

### Decision 6: inventory compile-time Package Registration

`inventory::submit!(ActPackageRegister::new::<T>())` allows packages to self-register using linker sections. Eliminates manual registration code — adding a new built-in package requires only adding the `inventory::submit!` line. Incompatible with WASM plugins (runtime loading).

**Applicability to Nebula:** Interesting for built-in action registration, but incompatible with Nebula's planned WASM plugin model.

---

## 19. Bus Factor / Sustainability

| Metric | Value |
|--------|-------|
| Upstream maintainer | 1 (yaojianpin — Yao) |
| Luminvent fork maintainer | Marc-Antoine ARNAUD (primary), Luminvent org |
| Commit cadence (Luminvent fork) | Occasional bursts — last commit 2025-08-29; prior major burst 2025-04-01 |
| Total commits (fork) | 172 (including inherited upstream commits) |
| Luminvent-authored commits | ~35 (Marc-Antoine ARNAUD) |
| Upstream stars | 61 |
| Fork stars | 2 |
| Issues | Disabled on fork; upstream has 9 total |
| Upstream crates.io downloads | 24.4K total |

**Bus factor: 2 effective** (yaojianpin for upstream, Marc-Antoine ARNAUD for Luminvent additions). The Luminvent fork appears to be an internal dependency fork with occasional upstream contributions rather than a standalone project with an independent user community. The fork's public positioning as a named project ("acts-next") appears to be research-assigned; the actual repo has no such branding.

---

## 20. Final Scorecard vs Nebula

| Axis | acts-next approach | Nebula approach | Verdict | Borrow? |
|------|--------------------|-----------------|---------|---------|
| A1 Workspace | 1 core crate + 6 plugin crates, no formal layers, Edition 2024 | 26 crates layered: nebula-error → nebula-resilience → … → nebula-tenant, Edition 2024 | **Nebula deeper** — crate boundaries enforce invariants acts cannot. acts simpler to build. | no — Nebula already better |
| A2 DAG | Linear tree (Workflow→Step→Branch→Act), `NodeTree` pointer-linked, `branch.needs` for join semantics, no typed ports | TypeDAG L1-L4 (generics → TypeId → predicates → petgraph) | **Nebula deeper** — acts has no compile-time DAG type safety. Branch.needs is a minimal join primitive. | no — Nebula already better |
| A3 Action | `ActPackageFn` with `Vars` I/O, open trait, `dyn` compatible, `run_as` (Func/Irq/Msg), `inventory` compile-time registration, `ActUserVar` extensible JS context | 5 action kinds, sealed traits, assoc Input/Output/Error, versioning, derive macros | **Nebula deeper** — type-erased Vars vs typed assoc types. | refine — `SetVars`/`SetProcessVars` pattern (mid-workflow state accumulation without task completion) is worth adopting for Nebula's pauseable actions |
| A4 Credential | None (7-line `SecretsVar` JS global only, `ActUserVar` trait extension point) | State/Material split, LiveCredential watch(), blue-green refresh, OAuth2Protocol, DynAdapter | **Nebula far deeper** — acts has no credential subsystem. | no — different goals |
| A5 Resource | None (plugin authors manage own connections; `ActPlugin::on_init` is the only hook) | 4 scope levels, ReloadOutcome, generation tracking, on_credential_refresh | **Nebula far deeper** — acts has no resource abstraction. | no — Nebula already better |
| A6 Resilience | `Retry { times: i32 }`, tick-based timeout, message re-delivery up to N times. No CB, bulkhead, backoff | nebula-resilience: retry/CB/bulkhead/timeout/hedging, ErrorClassifier | **Nebula far deeper** — acts' resilience is minimal. | no — Nebula already better |
| A7 Expression | JavaScript (rquickjs/QuickJS), fresh context per eval, `$get/$set/$inputs/$data`, `$step`, `$env`, `secrets.*`, `os`. `{{ }}` template syntax. `ActUserVar` extensible JS globals | 60+ funcs, type inference, sandboxed eval, JSONPath-like `$nodes.foo.result.email` | **Different decomposition** — acts uses full JS (more power, less safety/analyzability); Nebula uses custom sandboxed DSL | maybe — acts' `ActUserVar` pattern (pluggable JS global namespace) could inspire extensible context namespaces in Nebula's expression engine |
| A8 Storage | `DbCollection` trait, pluggable (MemStore default, SQLite/Postgres plugins), `sea-query` builder, no versioned migrations | sqlx + PgPool, Pg*Repo, SQL migrations, RLS | **Nebula deeper for production** — RLS, migrations, pgPool management. **acts simpler for testing** — in-memory default. | refine — acts' pluggable `DbCollection` interface (swap backend via plugin) is worth considering for Nebula test/dev modes |
| A9 Persistence | Checkpointing (in-place state updates), `data::Event` store for trigger registrations, `keep_processes` option. No event sourcing. | Frontier-based checkpoint, append-only log, replay-based reconstruction | **Nebula deeper** — replay capability vs simple checkpoint. | refine — acts' `data::Event` as stored trigger registration (enabling `evt().list()`) is a useful catalog pattern |
| A10 Concurrency | tokio, single scheduler event loop, `tokio::task_local!` for context, multiple processes via `tokio::spawn`, `acts.core.block` for block-level parallelism | tokio, frontier scheduler with work-stealing, `!Send` action support | **Different decomposition** — acts uses single-queue; Nebula uses work-stealing frontier. | maybe — `tokio::task_local!` context threading pattern is clean; `acts.core.block` sequence/parallel duality is a nice user-facing primitive |
| A11 Plugin BUILD | Rust crates, workspace members, `inventory::submit!` compile-time registration, `Cargo.toml` as manifest, no WASM | WASM sandbox planned, plugin-v2 spec, Plugin Fund commercial model | **Different goals** — acts chose simplicity (in-process Rust crates); Nebula chose isolation (WASM). | no — different goals |
| A11 Plugin EXEC | In-process, same memory space, no sandbox, channel-based external execution pattern | WASM sandbox, capability security | **Nebula more correct for platform** — acts trusts all plugins fully. | no — different goals |
| A12 Trigger | 3 event types (manual/hook/chat), no cron, no webhook, no external broker. `workflow.on: Vec<Act>`. `EventExecutor` for trigger discovery (list/get/start). `EventInfo` published type. | TriggerAction: Source→Event 2-stage, typed Input=Config/Output=Event | **Nebula deeper** — typed 2-stage Source→Event vs untyped 1-stage Vars. | refine — acts' `EventExecutor.list()` trigger catalog pattern is valuable UX; Nebula lacks a queryable trigger registry endpoint |
| A13 Deployment | Embedded library only. `acts-server` (separate repo) for gRPC + multi-language clients. No multi-mode binary. | 3 modes from one binary: desktop/self-hosted/cloud | **Nebula more complete** — acts is library-only. | no — different goals |
| A14 Multi-tenancy | None. Single-tenant embedded library. | nebula-tenant: schema/RLS/database isolation, RBAC, planned SSO/SCIM | **Nebula far deeper** — acts has no tenancy concept. | no — different goals |
| A15 Observability | `tracing` crate only, no OpenTelemetry, no metrics. `observability` plugin on roadmap. | OpenTelemetry per execution, per-action latency/count/error metrics | **Nebula deeper** — acts has logging only. | no — Nebula already better |
| A16 API | Rust library API (Executor/Channel/Extender), `acts-server` provides gRPC (separate repo), multi-language clients (Rust/Python/Go). New: `do_action()` public, `PageData<T>` public export. | REST + planned GraphQL/gRPC, OpenAPI spec, OwnerId-aware | **Different decomposition** — acts' gRPC-first remote API via `acts-server` with multi-language SDKs is more complete than Nebula's planned gRPC. | refine — acts' multi-language client SDK approach is worth modeling for Nebula's gRPC transport |
| A17 Type safety | Open traits, type-erased `Vars`, JSON Schema param validation at runtime, `Outputs::check()` basic type matching | Sealed traits, GATs, HRTBs, typestate, Validated<T> proof tokens | **Nebula far deeper** — acts relies on runtime JSON validation entirely. | no — Nebula already better |
| A18 Errors | `ActError` thiserror enum (10 variants, identical to upstream), no ErrorClass/transient-permanent distinction | nebula-error + ErrorClass (transient/permanent/cancelled) | **Nebula deeper** — ErrorClass enables intelligent retry policy. | refine — acts' `ActError::Exception { ecode, message }` (structured user-visible error with code + message) is a good pattern for workflow-level user errors |
| A19 Testing | ~65 test files co-located, `utils/test.rs` helpers, `Signal` sync primitive for async tests. No public testing crate. | nebula-testing crate, contract tests, insta+wiremock+mockall | **Nebula deeper for plugin authors** — no public testing utilities in acts. | no — Nebula already better |
| A20 Governance | Apache-2.0, upstream: solo maintainer; fork: Luminvent org + Marc-Antoine ARNAUD. No commercial model. Fork merges upstream changes. | Open core, Plugin Fund commercial model, planned SOC 2 | **Nebula more complete** — Plugin Fund differentiates. acts has no commercial model. | no — different trajectory |
| A21 AI/LLM | None. `ActPackageCatalog::Ai` placeholder. `plugins/ai` on roadmap. No providers, no abstractions. | None currently — strategic bet on generic actions + LLM plugin + Surge for agent orchestration | **Convergent** — both AI-absent today with "AI as plugin" roadmap direction. | maybe — acts' explicit `Ai` catalog category suggests taxonomy-aware package organization; Nebula should consider action catalog taxonomy for the editor UI |

**Total rows: 22 (A11 split into BUILD + EXEC as required)**

---

## Key Findings Summary

**What Luminvent changed from original acts:**
1. Added `SetVars` EventAction — the most impactful new feature, enabling mid-workflow state accumulation without forced task completion.
2. Exposed `do_action()` publicly — enables raw action dispatch for advanced integrators.
3. Added `keep_processes` config — operational convenience for inspection/debugging.
4. Fixed process state propagation bug (#12) before upstream acceptance.
5. Naming cleanup (strum conversions, module renames) — ergonomic improvements.
6. Exposed `PageData<T>` — minor API completeness.

**What was NOT changed:** The entire architecture — same traits, same type erasure, same `Vars` I/O, same QuickJS JavaScript engine, same inventory registration, same absence of credential/resource/resilience/AI layers.

**Verdict on "acts-next":** This is an operational fork with ergonomic additions, not a redesign. The label "acts-next" (research-assigned) overstates the extent of change. The primary research value is confirming that Luminvent (a commercial organization) has found acts useful enough to fork and maintain, and that the `SetVars`/`SetProcessVars` pattern for mid-workflow state accumulation is a real user need worth examining for Nebula.
