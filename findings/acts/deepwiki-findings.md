# acts — DeepWiki Findings

All 9 queries executed against `yaojianpin/acts`.

---

## Query 1: Core trait hierarchy for actions/nodes/activities

**Answer summary:** The core trait hierarchy is: `NodeKind` enum (Workflow/Branch/Step/Act) + `NodeContent` enum wrapping those types + `ActTask` trait with `init/run/next/review/error` methods dispatched by the scheduler. `ActPackageFn` trait has two lifecycle methods: `execute(&self, ctx: &Context) -> Result<Option<Vars>>` for sync core packages, and `async fn start(&self, rt: &Arc<Runtime>, options: &Vars) -> Result<Option<Vars>>` for event packages. Packages register via `inventory::submit!()` at compile-time. No associated types — I/O uses `Vars` (type alias for `serde_json::Map<String, Value>`).

---

## Query 2: Workflow state persistence and recovery

**Answer summary:** Checkpointing model. Five persisted collections: `Models`, `Procs`, `Tasks`, `Messages`, `Events`. State is written to store on each transition. Recovery via cache restore: `cache.restore()` reloads `Proc` + `Task` records from store on startup. Default backend is in-memory (`MemStore`). Pluggable via `ActPlugin` → `engine.extender().register_collection()`. SQLite and Postgres backends available as separate crates. No event sourcing, no append-only log, no replay.

---

## Query 3: Credential/secret management

**Answer summary:** No dedicated credential layer. `SecretsVar` implements `ActUserVar` and exposes a `secrets` JavaScript global in the expression sandbox — the data is passed in as `Vars` at workflow start and read via `secrets.TOKEN` in expressions. No encryption at rest, no Zeroize, no OAuth2, no refresh lifecycle. Grep evidence: only `acts/src/env/moudle/vars/secrets.rs` matches — 7 lines defining a name accessor.

---

## Query 4: Plugin architecture (ActPlugin vs ActPackage, compile/exec location)

**Answer summary:** Two distinct extension points:
- `ActPlugin` trait (system-level, `on_init(&Engine)` async) — store backends, external service init
- `ActPackage` trait (workflow functionality) + `ActPackageFn` trait — packages that can be referenced in workflow YAML via `uses: acts.xxx.yyy`

Both compile as Rust crates in the workspace. No WASM, no dynamic library loading. Packages registered via `inventory::submit!(ActPackageRegister::new::<T>())` — compile-time only. External plugins (http, state, shell) are Rust crates that implement `ActPlugin`, register their package via `engine.extender().register_package()`, and subscribe to engine messages via `engine.channel_with_options()` to handle execution.

---

## Query 5: Concurrency primitives and !Send handling

**Answer summary:** tokio async runtime. `ShareLock<T> = Arc<RwLock<T>>` used throughout for Task/Process fields. Scheduler uses a bounded `tokio::sync::mpsc::channel(100)` as task queue. `tokio::task_local!` macro used for `Context` (the execution context is thread-local, not passed through call stacks). `Handle::current().spawn()` for event dispatch. All event handler closures require `Fn(...) + Send + Sync + 'static`. No `!Send` isolation — all types must be `Send`. Uses `unsafe impl Send for Enviroment` to satisfy the constraint.

---

## Query 6: Trigger/event modeling

**Answer summary:** Triggers are `Act` instances in `workflow.on: Vec<Act>`. Three event packages: `acts.event.manual` (explicit start), `acts.event.hook` (hook-triggered, waits for completion signal), `acts.event.chat` (chat event trigger). `schedule` is on the roadmap but not implemented. No webhook, no cron, no Kafka/queue, no DB change-data-capture triggers. Trigger packages implement `ActPackageFn::start()` async method — they start a new workflow process.

---

## Query 7: LLM/AI integration

**Answer summary:** No built-in LLM integration. `ActPackageCatalog::Ai` variant exists as a placeholder category. Roadmap shows `plugins/ai` as planned. No openai/anthropic/llm code found anywhere in the codebase. The `acts.event.chat` package name suggests intent but its implementation only starts a workflow from a string parameter — no LLM calls.

---

## Query 8: Major architectural trade-offs

**Answer summary:** The explicit design choice is message-driven architecture vs. traditional BPMN sequential flow. Key trade-off: acts uses pub-sub event emission at every state transition (every step/act creates/completes generates a message to channels) enabling loose coupling between engine and business logic. BPMN has tighter coupling between tasks and their service implementations. The IRQ pattern (interrupt request) is acts' mechanism for human-in-the-loop — workflow pauses at `acts.core.irq`, emits a message, and resumes only when client sends `Next/Submit/Error/etc` action. This differs from BPMN's UserTask but achieves the same goal.

---

## Query 9: Known limitations and planned redesigns

**Answer summary:** From roadmap: `schedule` event trigger not implemented; `form`, `ai`, `pubsub`, `observability`, `database`, `mail` package plugins planned but not built. Issue #16 "Cache learning" open. Issue #10 "Add version on a Model/Package" open. Major past redesigns: v0.12.0 completely changed act YAML format, v0.16.0 refactored package system, v0.17.0 refactored env module and changed expression syntax from `${ }` to `{{ }}`. High churn on public API suggests pre-1.0 stability concerns.
