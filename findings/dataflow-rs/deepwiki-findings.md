# DeepWiki Findings — dataflow-rs

Repository indexed: YES (GoPlasmatic/dataflow-rs)

---

## Query 1: Core trait hierarchy for actions/nodes/activities

**Answer:** The core hierarchy is `Engine` → `Workflow` → `Task` → `FunctionConfig`. The `AsyncFunctionHandler` trait is the extension point:

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

`FunctionConfig` is an untagged enum with `Map`, `Validation`, `ParseJson`, `ParseXml`, `PublishJson`, `PublishXml`, `Filter`, `Log`, `HttpCall`, `Enrich`, `PublishKafka`, and `Custom` variants. Terms "actions", "nodes", "activities" are not explicit — the system uses "Workflows" and "Tasks".

---

## Query 2: Workflow state persistence and crash recovery

**Answer:** No built-in persistence or checkpoint mechanism. The engine processes individual `Message` objects. Workflow state is entirely in-memory within the `Message` struct (context JSON, audit_trail, errors). Any persistence must be implemented externally by serializing/deserializing `Message`. No database, no event log, no crash recovery.

---

## Query 3: Credential/secret management approach

**Answer:** No credential or secret management. The codebase has no modules for credential storage, API key management, encryption at rest, or secret rotation. Any secret would be passed inline in workflow JSON config or via environment variables in the calling application.

---

## Query 4: Plugin or extension implementation

**Answer:** No WASM plugin sandbox, no dynamic library loading, no subprocess execution. The only extension mechanism is compile-time Rust trait implementation: implement `AsyncFunctionHandler`, register in a `HashMap`, pass to `Engine::new()`. All custom logic is linked into the same binary at compile time. The WASM target (`wasm/` crate) is for compiling the engine itself to run in the browser — not a plugin sandbox.

---

## Query 5: Concurrency primitives and !Send handling

**Answer:** Tokio multi-thread runtime (`rt-multi-thread` feature). `Arc<T>` for shared state (DataLogic instance, compiled logic cache, function registry). `async-trait` for polymorphic async methods. `AsyncFunctionHandler` requires `Send + Sync` — `!Send` types cannot be registered as custom functions. No `!Send` isolation mechanism (no thread-local sandbox). Sequential workflow processing per message (no parallel task execution).

---

## Query 6: Triggers, webhooks, schedules, external events

**Answer:** No trigger subsystem at all. The engine is purely reactive — it processes `Message` objects passed explicitly to `Engine::process_message()`. There are no webhook listeners, no cron scheduler, no Kafka consumer, no event bus. The `publish_kafka` integration config in `FunctionConfig` covers *output* to Kafka, not *input* triggers. Integration with external event sources is entirely the caller's responsibility.

---

## Query 7: Built-in LLM/AI integration

**Answer:** No built-in LLM or AI integration. The words "openai", "anthropic", "llm", "gpt", "claude", "gemini", "embedding", "completion", "model" do not appear in the source (verified by grep). Custom functions could call an LLM via HTTP but no abstraction or provider trait exists.

---

## Query 8: Major architectural trade-offs

**Answer:** Four documented trade-offs:
1. **Startup time vs. runtime performance** — compile all JSONLogic at `Engine::new()`, zero overhead at process time
2. **Immutability vs. dynamic modification** — workflows immutable after creation; hot-reload requires constructing a new engine via `with_new_workflows()` which reuses the function registry Arc
3. **Memory usage vs. CPU overhead** — `Arc<CompiledLogic>` zero-copy sharing across threads; logic_cache Vec with O(1) index access
4. **CPU-bound vs I/O-bound** — CPU JSONLogic eval recommended to use `spawn_blocking`; custom functions are fully async

---

## Query 9: Known limitations and planned redesigns

**Answer:** Primary documented bug: `temp_data` root overwrite (issue #1, closed). Setting `path: "temp_data"` previously replaced the entire `temp_data` object instead of merging. Fixed in current codebase. No other documented planned redesigns or open issues (all 8 issues are closed as of 2026-04-26).

Total DeepWiki queries: 9/9
