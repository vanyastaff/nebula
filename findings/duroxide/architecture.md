# duroxide — Architectural Decomposition

## 0. Project Metadata

- **Repo:** https://github.com/microsoft/duroxide
- **Stars / Forks:** Data not available via CLI (young project, very small community)
- **Crates.io:** v0.1.28, 1.9K total downloads, 1.4K recent — very early adoption
- **Created:** 2025-11-30 (less than 6 months old at time of analysis)
- **Last release:** 0.1.28 on 2026-04-23 (active, rapid release cadence)
- **License:** MIT
- **Governance:** Microsoft-owned repository; primary contributor `affandar` (github.com/affandar); solo development with occasional PR contributions
- **Description (author):** "A lightweight and embeddable durable execution runtime for Rust. Inspired by the Durable Task Framework and Temporal."
- **Ecosystem:** Companion crates `duroxide-pg` (PostgreSQL provider, separate repo `microsoft/duroxide-pg`) and `toygres` (sample app, `affandar/toygres`)

---

## 1. Concept Positioning [A1, A13, A20]

**Author's description:** "A lightweight and embeddable durable execution runtime for Rust" inspired by Durable Task Framework and Temporal. The core abstraction is deterministic replay: orchestrations are async Rust functions that get replayed from history after any crash or restart.

**Mine, after reading code:** duroxide is a single-crate embedded durable execution library whose design philosophy is "orchestrator function + activity function" — the DurableTask/Temporal pattern ported idiomatically to tokio async Rust. It prioritizes embeddability and storage-agnosticism (Provider trait) over feature breadth.

**Comparison with Nebula:** Nebula is a workflow *engine* targeting n8n + Temporal use cases in a 26-crate workspace with credential management, expression evaluation, multi-tenancy, and 5 action kinds as peer-level concerns. duroxide is a focused *durable execution library* — it handles replay, activity scheduling, and timers superbly, but deliberately leaves credentials, expressions, multi-tenancy, and triggers entirely out of scope. Nebula is a platform; duroxide is a runtime primitive.

---

## 2. Workspace Structure [A1]

**Crate count:** 2 crates total in the workspace (`Cargo.toml:50-53`):
- `duroxide` — the core framework (all runtime, providers, client)
- `sqlite-stress` — benchmark/stress tool (separate workspace member)

**Single-crate architecture.** All functionality — replay engine, dispatchers, provider trait, SQLite provider, client API, observability — lives in one crate. Internal module structure (`src/lib.rs:410-427`):
- `src/runtime/` — `mod.rs`, `dispatchers/`, `registry.rs`, `replay_engine.rs`, `execution.rs`, `observability.rs`, `limits.rs`
- `src/providers/` — `mod.rs`, `error.rs`, `sqlite.rs`, `instrumented.rs`, `management.rs`
- `src/client/mod.rs`
- `src/provider_validation/` — comprehensive provider conformance test suite
- `src/provider_stress_test/` — stress test infrastructure

**Feature flags** (`Cargo.toml:14-22`):
- `sqlite` — enables bundled SQLite provider (optional)
- `provider-test` — provider validation harness (generic, no provider dep)
- `test` — enables `provider-test` + `test-hooks`
- `test-hooks` — fault injection for integration tests
- `replay-version-test` — gates v2 event types for replay-engine extensibility verification

**Comparison with Nebula:** Nebula has 26 layered crates with hard SRP boundaries. duroxide concentrates everything in one crate with feature flags. The duroxide approach enables simpler embedding at the cost of all-or-nothing compilation. Nebula's multi-crate model provides finer dependency control and independent maturity tracking. The single-crate approach is justifiable for duroxide's scope, but would not scale to Nebula's ambitions.

---

## 3. Core Abstractions [A3, A17] — DEEP

### A3.1 — Trait Shape

duroxide expresses its core units of work through **two traits** and **two registration types**, not a sealed multi-kind system:

```rust
// src/runtime/mod.rs:424-426
#[async_trait]
pub trait OrchestrationHandler: Send + Sync {
    async fn invoke(&self, ctx: OrchestrationContext, input: String) -> Result<String, String>;
}

// src/runtime/mod.rs:447-449
#[async_trait]
pub trait ActivityHandler: Send + Sync {
    async fn invoke(&self, ctx: ActivityContext, input: String) -> Result<String, String>;
}
```

Both traits use `async_trait` (0.1.x bridge macro). They are **open traits** — any type implementing `Send + Sync` with the right signature can implement them. There is **no sealed-trait pattern** — external crates can freely implement `OrchestrationHandler` or `ActivityHandler`.

The public surface is however dominated by **function-based registration** via closures, not trait impls directly:

```rust
// src/runtime/registry.rs:286-299
impl OrchestrationRegistryBuilder {
    pub fn register<F, Fut>(mut self, name: impl Into<String>, f: F) -> Self
    where
        F: Fn(OrchestrationContext, String) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<String, String>> + Send + 'static,
    { ... }
```

**Associated type count:** Zero formal associated types. The trait has a single `invoke()` method with string-typed I/O. No GATs, no HRTBs in core traits. Type safety at the boundary is achieved through **wrapper functions** that call `serde_json::from_str` / `serde_json::to_string` internally via `register_typed<In, Out>()`.

**Trait object compatible:** Yes — both traits are used as `dyn OrchestrationHandler` / `dyn ActivityHandler` stored in `Arc<H>` inside the registry (`src/runtime/registry.rs:34`).

### A3.2 — I/O Shape

All I/O crosses the trait boundary as `String`. The `register_typed<In, Out>` builder methods add serde wrapping internally (`src/runtime/registry.rs:302-325`). There is **no streaming output**. I/O is not type-erased via `serde_json::Value` at the API boundary — the raw `String` is the erasure layer. Side effects live entirely in activity code (activities are where all non-deterministic work happens).

### A3.3 — Versioning

Versioning is first-class. The registry stores handlers in a `BTreeMap<semver::Version, Arc<H>>` per name (`src/runtime/registry.rs:34`). Version policy is either `Latest` (always pick highest) or `Exact(Version)` (`src/runtime/registry.rs:24-27`). Orchestrations can be registered at explicit versions via `register_versioned()` / `register_versioned_typed()`. Activities are always stored at version `1.0.0` with `Latest` policy (hardcoded — activities do not version independently). Version strings are persisted into event history via `OrchestrationStarted { version }` events (`src/lib.rs:1094-1101`), enabling future mixed-version cluster routing. A `SemverRange` / `DispatcherCapabilityFilter` mechanism allows dispatchers to filter work items by the duroxide version that created them (`src/providers/mod.rs:161-188`).

### A3.4 — Lifecycle Hooks

The orchestration model has **no pre/post/cleanup hooks**. Lifecycle is expressed implicitly:
- **Start:** orchestration function is called with empty history
- **Each turn:** full replay from history up to current point, then continue
- **Completion/failure:** `TurnResult::Completed` / `TurnResult::Failed` returned by replay engine (`src/runtime/replay_engine.rs:18-29`)
- **Cancellation:** `CancelInstance` work item causes `TurnResult::Cancelled`

Activities receive a `CancellationToken` (via `tokio-util`) and an `ActivityContext`. There is **no explicit pre/post hook** for activities. Idempotency is the user's responsibility.

### A3.5 — Resource and Credential Dependencies

**None declared.** Activities receive an `ActivityContext` which provides access to `get_client()` (returns a `Client` for KV access and sub-orchestration introspection). There is **no mechanism for declaring resource or credential dependencies** at registration time. Resource acquisition is the activity's own responsibility. This is a deliberate simplicity choice — duroxide is not a dependency injection framework.

### A3.6 — Retry/Resilience Attachment

Per-activity retry is **declared at the call site**, not at registration time. The orchestration code calls `ctx.schedule_activity_with_retry(name, input, policy)` with a `RetryPolicy` struct (`src/lib.rs:1404-1487`). `BackoffStrategy` supports `None`, `Fixed`, `Linear`, and `Exponential` (`src/lib.rs:1346-1401`). Retry is fully orchestrator-side — implemented as a loop + timer in orchestration code, not a worker-side feature. There is no per-activity global default policy.

### A3.7 — Authoring DX

Authoring is **closure-based** with the builder pattern. A minimal "hello world" activity in 1 line:

```rust
.register("Hello", |ctx: ActivityContext, name: String| async move { Ok(format!("Hello, {name}!")) })
```

No derive macros. No attribute macros. No code generation. The DX is pure idiomatic Rust closures. IDE support is full (standard Rust closures with type inference).

### A3.8 — Metadata

**No display name, description, icon, or category metadata.** Activities and orchestrations are identified by string names only. There is no i18n, no compile-time metadata, no catalog/schema. This is a deliberate minimal approach.

### A3.9 — vs Nebula

Nebula has 5 sealed action kinds (`ProcessAction`, `SupplyAction`, `TriggerAction`, `EventAction`, `ScheduleAction`) with associated `Input`/`Output`/`Error` types, derive macros, and metadata. duroxide has 2 open function traits (`OrchestrationHandler`, `ActivityHandler`) with string-typed I/O and no metadata. duroxide's model is closer to Temporal's "orchestration function + activity function" decomposition. The key missing piece vs Nebula is **kind-level semantics**: duroxide cannot express "this is a trigger that produces events" vs "this is a supply that produces resources". All behaviors must be encoded in orchestration logic.

---

## 4. DAG / Execution Graph [A2, A9, A10]

### Graph Model

duroxide does **not use a static DAG**. There is no graph DSL, no petgraph, no port typing. Instead, the "graph" is the **implicit execution tree** formed by orchestration functions calling `schedule_activity()`, `schedule_sub_orchestration()`, and `schedule_timer()`. The graph is **dynamic and built at runtime** through replay.

The execution structure is:
- **Sequential:** `let a = ctx.schedule_activity("A", input).await?; let b = ctx.schedule_activity("B", a).await?`
- **Parallel (fan-out):** `ctx.join(vec![f1, f2, f3]).await`
- **Race (select):** `ctx.select2(activity, timer).await` returns `Either2<A, B>`
- **Sub-trees:** `ctx.schedule_sub_orchestration("Sub", input).await` creates child orchestrations

**Compile-time safety:** None beyond Rust's async type system. Port types are not checked — activities always emit `String` / `Result<String, String>`. **No generics-based compile-time DAG checking** (unlike Nebula's TypeDAG L1-L4).

### Concurrency Model

Two tokio task pools with configurable concurrency (`src/runtime/mod.rs:155-182`):
- `OrchestrationDispatcher` — N concurrent workers, each processing one orchestration turn at a time
- `WorkDispatcher` — M concurrent workers, each executing one activity at a time

Lock semantics: peek-lock on both queues. Orchestration items have a short lock timeout (default 5s); activities have a longer one (default 30s) with automatic renewal. The key insight: orchestration turns are fast (milliseconds) — they just replay history and emit new actions. Heavy work happens in activities.

**!Send handling:** The orchestration function is `Send` (`OrchestrationHandler: Send + Sync`). Activities are also `Send`. There is no `!Send` path in duroxide — the single-threaded replay model requires `Send` to dispatch across tokio worker threads.

### Deterministic Resolution

`join()` and `select()` / `select2()` are **history-ordered**, not poll-ordered. The replay engine delivers completions in the order they appear in history, ensuring deterministic behavior across replays (`docs/execution-model.md:101-127`).

**Comparison with Nebula:** Nebula's TypeDAG (L1-L4) provides compile-time guarantees on port connectivity, type matching, and graph soundness. duroxide relies entirely on runtime correctness through replay determinism. Nebula is safer for complex DAGs; duroxide is simpler to use for sequential/parallel patterns that don't need compile-time enforcement.

---

## 5. Persistence and Recovery [A8, A9]

### Storage Schema

duroxide uses SQLite (bundled, optional feature) or a custom `Provider` implementation. The schema (`migrations/20240101000000_initial_schema.sql`) defines:

- `instances` — per-instance metadata (orchestration name/version, current execution ID)
- `executions` — per-execution records (status, output, timestamps) — supports ContinueAsNew multi-execution
- `history` — append-only event log keyed by `(instance_id, execution_id, event_id)`, storing JSON-serialized `Event` objects
- `orchestrator_queue` — work items for the orchestration dispatcher (with visibility, lock, attempt_count)
- `worker_queue` — work items for the worker dispatcher (activities)
- `instance_locks` — per-instance advisory locks for peek-lock semantics
- `sessions` — session affinity tracking (heartbeat lease, idle timeout, crash recovery)
- `kv_store` — materialized KV state (written at execution completion boundaries)
- `kv_delta` — KV mutations during current execution (merged at completion; seeded from `kv_store` at fetch time to avoid replay poisoning)

Total migrations: 12, from initial schema to `add_kv_delta` in v0.1.26.

### Persistence Model

**Event sourcing / append-only history.** Each orchestration turn appends `history_delta` events atomically via `ack_orchestration_item()`. The state of an orchestration is fully derivable by replaying its history. No mutable state is stored except for KV store and custom status (which are also replayed from `KeyValueSet`/`CustomStatusUpdated` events).

**Replay-based recovery:** After a crash, the runtime re-reads history from the provider and re-executes the orchestration function from the beginning. The replay engine (`src/runtime/replay_engine.rs`) delivers completions from persisted history events, allowing the function to fast-forward to the current suspension point without re-executing activities.

**History cap:** Default 1024 events per execution. If exceeded, the provider returns an error and the runtime terminates the orchestration rather than truncating (determinism preservation). `continue_as_new` is the pattern for long-running workflows that would exceed the cap.

**Atomic commit:** `ack_orchestration_item(lock_token, execution_id, history_delta, orchestrator_items, worker_items, metadata, cancelled_activities, ...)` is a single atomic operation. History append, queue enqueue/dequeue, and metadata update all happen in one transaction.

**Comparison with Nebula:** Nebula uses a **frontier-based scheduler** with checkpoint recovery and an append-only execution log — conceptually similar. The key differences: (1) duroxide explicitly surfaces the replay model to the user through the orchestration function pattern; (2) Nebula's frontier model is more opaque but allows richer scheduling semantics; (3) duroxide's KV delta table (v0.1.26) is an interesting innovation to solve the read-modify-write replay poisoning problem — a problem Nebula does not yet publicly surface a solution for.

---

## 6. Credentials / Secrets [A4] — DEEP

### A4.1 — Existence

**No credential layer exists in duroxide.** This is an explicit design omission — duroxide is a durable execution runtime, not a secrets manager or integration platform.

**Grep evidence (searched in `src/` directory):**
- `grep -r "credential" --include="*.rs" src/` — 0 matches for the concept; all matches are for `lock_token` / `CancellationToken` (unrelated)
- `grep -r "secret\|zeroize\|secrecy\|oauth\|vault" --include="*.rs" src/` — 0 matches
- `grep -r "credential\|oauth\|oidc" docs/` — 0 matches in all documentation files

### A4.2-A4.9 — All Sub-Axes

All A4 sub-axes are **not applicable / not implemented:**
- A4.2 Storage: No credential storage, no encryption
- A4.3 In-memory protection: No `Zeroize`, no `secrecy::Secret<T>`
- A4.4 Lifecycle: No credential CRUD, no refresh model
- A4.5 OAuth2/OIDC: Not present
- A4.6 Composition: Not applicable
- A4.7 Scope: Not applicable
- A4.8 Type safety: Not applicable
- A4.9 vs Nebula: duroxide has none of Nebula's State/Material split, LiveCredential, watch(), blue-green refresh, OAuth2Protocol, or DynAdapter

**Design philosophy:** Credentials are the user's concern. In duroxide, activity code calls external APIs using whatever credentials the host process injects (environment variables, config files, secrets from vault). The framework makes no attempt to manage credentials. This contrasts sharply with Nebula's deep credential subsystem.

---

## 7. Resource Management [A5] — DEEP

### A5.1 — Existence

**No first-class resource abstraction.** duroxide does not provide a `Resource` trait, scope levels, or lifecycle hooks for database pools, HTTP clients, or caches.

**Grep evidence:**
- `grep -r "Resource\|resource_lifecycle\|ReloadOutcome\|on_credential_refresh" --include="*.rs" src/` — 0 matches relevant to resource management
- `grep -r "resource" --include="*.rs" src/` — only matches `resource` as a field in `ConfigErrorKind::Nondeterminism { resource }` (the orchestration name) at `src/lib.rs:961`

### A5.2-A5.8 — All Sub-Axes

All A5 sub-axes are **not applicable / not implemented:**
- A5.1-A5.8: Not present; no scope levels, no lifecycle hooks, no reload model, no generation tracking, no backpressure for resource acquisition

**Design philosophy:** Resources are the user's responsibility. Activities receive an `ActivityContext` that provides `get_client()` for sub-orchestration introspection, but no resource handles. The user creates their own `Arc<Pool>` or `Arc<Client>` and captures it in activity closures:

```rust
let pool = Arc::new(PgPool::connect(&db_url).await?);
let pool_clone = pool.clone();
activities.register("QueryDB", move |ctx: ActivityContext, query: String| {
    let pool = pool_clone.clone();
    async move { /* use pool */ }
});
```

This is a Rust-native pattern that works well but lacks the observability, lifecycle management, and credential-rotation features that Nebula's resource system provides.

---

## 8. Resilience [A6, A18]

### Retry

Per-activity retry via `RetryPolicy` (`src/lib.rs:1425-1487`). Retry is **implemented entirely in orchestration code** using existing primitives — loops, timers, and `schedule_activity` calls. The implementation at the framework level generates a sequence of `ActivityScheduled`/`ActivityCompleted`/`ActivityFailed`/`TimerCreated`/`TimerFired` events in history, making retry deterministic and replayable.

`BackoffStrategy` variants: `None`, `Fixed { delay }`, `Linear { base, max }`, `Exponential { base, multiplier, max }` (`src/lib.rs:1346-1401`). Default is exponential (100ms base, 2.0×, 30s max).

Per-attempt timeout via `RetryPolicy::with_timeout()` — races each activity attempt against a timer (`RetryPolicy::timeout` field, `src/lib.rs:1430-1435`).

### Error Classification

Three-tier classification (`src/lib.rs:888-972`):
- `ErrorDetails::Infrastructure { operation, message, retryable }` — provider failures, data corruption. Abort the turn; never reach user code
- `ErrorDetails::Configuration { kind: Nondeterminism, resource }` — unregistered handler, replay mismatch. Abort turn; require deployment fix
- `ErrorDetails::Application { kind, message, retryable }` — business logic failures; propagate normally through orchestration code
- `ErrorDetails::Poison { attempt_count, max_attempts, message_type }` — message exceeded max fetch attempts

`ProviderError` mirrors this with `retryable: bool` flag (`src/providers/error.rs:37-44`).

### What is NOT present

**No circuit breaker** (grep: `grep -r "circuit\|breaker\|bulkhead\|hedg" --include="*.rs" src/` — 0 results). No bulkhead, no hedging, no adaptive timeout. The runtime uses unregistered-handler exponential backoff (`UnregisteredBackoffConfig`, `src/runtime/mod.rs:66-129`) — but this is not a user-facing resilience API.

**Comparison with Nebula:** Nebula's `nebula-resilience` crate provides a full resilience palette (retry, CB, bulkhead, timeout, hedging) with `ErrorClassifier` separating transient from permanent. duroxide provides per-activity retry only. Nebula is significantly deeper here.

---

## 9. Expression / Data Routing [A7]

**No expression engine exists in duroxide.**

**Grep evidence:**
- `grep -r "expression\|jmespath\|jsonpath\|sandbox\|eval" --include="*.rs" src/` — 0 matches
- No `$nodes.foo.result` syntax, no expression language, no computed data routing DSL

Data routing between activities is done via standard Rust — orchestration code captures outputs in variables and passes them as inputs to subsequent `schedule_activity()` calls. This is a deliberate choice: Rust's type system and async/await syntax subsume the need for a DSL.

**Comparison with Nebula:** Nebula has a 60+ function expression engine with type inference and sandboxed evaluation, supporting `$nodes.foo.result.email` syntax for connecting node outputs to inputs. duroxide has no analog. For duroxide's embedded-library use case, this is reasonable; for n8n-style "build workflows visually", it would be a hard gap.

---

## 10. Plugin / Extension System [A11] — DEEP

### 10.A — Plugin BUILD Process

**No plugin system exists in duroxide.**

**A11.1 Format:** No plugin manifest format, no .tar.gz, OCI, WASM, or dynamic library loading.

**A11.2 Toolchain:** No separate plugin toolchain. Extension is done by Rust trait implementation at compile time.

**A11.3 Manifest:** Not applicable.

**A11.4 Registry/discovery:** Not applicable. All orchestrations and activities are registered in-process before runtime start.

**Grep evidence:**
- `grep -r "plugin\|wasm\|dlopen\|libloading\|wasmtime\|wasmer" --include="*.rs" src/` — 0 matches
- `grep -r "plugin" docs/` — 0 matches

**Extension model for BUILD:** Implement the `Provider` trait for new storage backends, then pass an `Arc<dyn Provider>` to `Runtime::start_with_store()`. This is the only designed extension point at build time. There is no third-party plugin fund, no sandboxing, no capability-based security.

### 10.B — Plugin EXECUTION Sandbox

**No plugin execution sandbox.**

**A11.5 Sandbox type:** Not applicable. Activities run in the same process as the runtime.

**A11.6 Trust boundary:** No boundary. Activity closures are trusted Rust code.

**A11.7 Host↔plugin calls:** Activities are plain async Rust closures — no marshaling layer beyond serde_json for I/O serialization.

**A11.8 Lifecycle:** Activities start and stop with the tokio task lifecycle. No hot reload.

**A11.9 vs Nebula:** Nebula targets WASM sandbox (wasmtime), capability-based security, and a Plugin Fund commercial model. duroxide has none of this. The design philosophy is different: duroxide is embedded infrastructure, not an app store. WASM sandboxing would conflict with duroxide's use case (embedding into a Rust application that brings its own activities).

---

## 11. Trigger / Event Model [A12] — DEEP

### A12.1 — Trigger Types

duroxide supports the following inbound event types:

- **External events (one-shot):** `Runtime::raise_event(instance, name, data)` / `ctx.schedule_wait(name)` — ephemeral signal pattern. Used for human-in-the-loop, webhook callbacks, approvals.
- **External events (queue/FIFO):** `client.enqueue_event(instance, queue, data)` / `ctx.dequeue_event(queue)` — persistent mailbox semantics. Survives ContinueAsNew.
- **Durable timers:** `ctx.schedule_timer(Duration)` — orchestration-level delays. Backed by `TimerFired` work items with `visible_at = fire_at_ms` for delayed visibility.

**No built-in webhook registration.** No schedule/cron trigger. No Kafka/RabbitMQ/NATS integration. No polling model. No database CDC. No filesystem watcher.

### A12.2 — Webhook

Not implemented. Webhooks must be handled at the application layer: the host HTTP server receives the request and calls `client.raise_event(instance, name, data)` or `client.enqueue_event(instance, queue, data)`.

### A12.3 — Schedule / Cron

Not implemented. Timers are duration-based (`ctx.schedule_timer(Duration)`), not cron-based. There is no cron syntax, no timezone support, no missed-schedule recovery beyond timer determinism. For periodic orchestrations, the pattern is `ctx.continue_as_new()` at the end of each execution with a timer at the start of the next.

### A12.4 — External Event Integration

No direct broker integration. External systems must call the `Client` API to inject events. The `docs/proposals/durable-copilot-sdk-features.md` describes a real-world use case where GitHub Copilot SDK events are forwarded to duroxide via `raise_event`.

### A12.5 — Reactive vs Polling

The runtime is **event-driven** (message-driven). Dispatchers poll the provider queues (`fetch_orchestration_item`, `fetch_work_item`) in a loop with configurable `dispatcher_min_poll_interval` (default 100ms) and optional long polling (`dispatcher_long_poll_timeout`, default 30s). Long polling is provider-dependent — SQLite provider supports it.

### A12.6 — Trigger → Workflow Dispatch

External events are named and matched to waiting subscriptions in history. One-shot events (`raise_event`) match positionally (Nth raise to Nth wait for the same name). Queue events (`enqueue_event`) match FIFO against `dequeue_event` subscriptions. There is no fan-out; events are 1:1 per orchestration instance.

### A12.7 — Trigger as Action

External events are not modeled as action "kinds" like Nebula's `TriggerAction`. They are **orchestration primitives** — `Action::WaitExternal`, `Action::DequeueEvent` — emitted by the orchestration context and consumed by the replay engine. There is no separate trigger lifecycle; waiting for an event is simply a suspension point in orchestration code.

### A12.8 — vs Nebula

Nebula has a 2-stage `Source → Event → TriggerAction` model where `TriggerAction` is a first-class action kind with `Input = Config` (startup registration) and `Output = Event` (typed payload). duroxide uses a simpler inline model: `ctx.schedule_wait("name")` is the trigger, and external code calls `client.raise_event()` to signal. There is no 2-stage separation, no Source trait, no Event normalization layer. For embedded use cases this is sufficient. For a platform like Nebula that needs to normalize across HTTP/Kafka/cron sources, duroxide's model would require significant extension.

---

## 12. Multi-tenancy [A14]

**No multi-tenancy support.**

**Grep evidence:**
- `grep -r "tenant\|multi.tenant\|rbac\|sso\|scim\|schema_isolation" --include="*.rs" src/` — 0 matches
- No `tenant_id` field in any struct; no row-level security (RLS) configuration; no RBAC

The `instances` table uses string `instance_id` with no namespace/tenant partitioning. All orchestrations share the same SQLite database. Isolation between workloads is the user's responsibility (separate databases, separate Provider instances, separate processes).

**Comparison with Nebula:** Nebula has a dedicated `nebula-tenant` crate with three isolation modes (schema / RLS / database), RBAC, and SSO/SCIM roadmap. duroxide has no analog. This is a fundamental scope difference — duroxide is designed for single-tenant embedded use.

---

## 13. Observability [A15]

### Tracing

Structured logging via `tracing` + `tracing-subscriber` (`Cargo.toml:29-30`). Three log formats: `Compact`, `Pretty`, `Json` (`src/runtime/observability.rs:36-43`). All logs include correlation fields: `instance_id`, `execution_id`, `orchestration_name`, `orchestration_version`, `activity_name`, `worker_id`.

Replay-safe logging: `ctx.trace_info()`, `ctx.trace_warn()`, `ctx.trace_error()` are implemented as **deterministic system activities** (`SYSCALL_ACTIVITY_UTC_NOW_MS`, etc., `src/runtime/mod.rs:26-62`). Logs are emitted at completion time, not during replay, preventing duplicate log output.

### Metrics

Metrics via the `metrics` crate facade (version 0.24, `Cargo.toml:33-34`). Zero-cost no-ops if no recorder is installed. Users install their preferred exporter: Prometheus, OpenTelemetry, CloudWatch, Azure Monitor. Tracked metrics include:
- `duroxide_active_orchestrations` (gauge)
- `duroxide_orchestrator_queue_depth` / `duroxide_worker_queue_depth` (gauge, polled from provider)
- `duroxide_orchestration_started_total` / `duroxide_orchestration_completed_total` (counters)
- `duroxide_orchestration_duration_seconds` (histogram)
- `duroxide_activity_execution_duration_seconds` (histogram, with labels: activity_name, outcome, retry_attempt, tag)
- Poison, infra error, config error counters

**No built-in OpenTelemetry integration.** The `metrics` facade is OTel-compatible but requires user-provided bridge. There is no trace span per execution — logging is structured text, not distributed traces.

**Comparison with Nebula:** Nebula uses OpenTelemetry directly with one trace = one workflow run and per-action spans. duroxide uses the `metrics` facade (more flexible, less opinionated) and `tracing` for structured text logs. Both correlate by execution/instance ID. duroxide's approach is more embeddable; Nebula's gives richer distributed trace integration.

---

## 14. API Surface [A16]

**Programmatic Rust API only.** No HTTP REST API, no GraphQL, no gRPC, no OpenAPI spec.

The `Client` struct (`src/client/mod.rs`) provides:
- `start_orchestration(instance, name, input)` → `Result<(), ClientError>`
- `wait_for_orchestration(instance, timeout)` → `Result<OrchestrationStatus, ClientError>`
- `get_orchestration_status(instance)` → `Result<OrchestrationStatus, ClientError>`
- `cancel_orchestration(instance)` → `Result<(), ClientError>`
- `raise_event(instance, name, data)` → `Result<(), ClientError>`
- `enqueue_event(instance, queue, data)` → `Result<(), ClientError>`
- `delete_instance(instance, force)` → `Result<DeleteInstanceResult, ClientError>`
- `delete_instance_bulk(filter)`, `prune_executions()`, `get_instance_tree()`
- `get_kv_value(instance, key)` → KV read
- `get_orchestration_stats(instance)` → `SystemStats` (v0.1.27)

`ProviderAdmin` trait (`src/providers/management.rs`) exposes:
- `list_instances()`, `list_executions()`, `get_system_metrics()`, `get_queue_depths()`

**Comparison with Nebula:** Nebula has a REST API, OpenAPI spec, and planned GraphQL/gRPC. duroxide's Rust-only API is appropriate for its embedded use case but would need a network layer for SaaS deployment.

---

## 15. Testing Infrastructure [A19]

### Provider Validation Suite

duroxide ships an impressive **generic provider conformance test suite** (`src/provider_validation/`) gated behind the `provider-test` feature. This is a concrete advantage over Nebula.

Test modules cover:
- `atomicity`, `bulk_deletion`, `cancellation`, `capability_filtering`
- `custom_status`, `deletion`, `error_handling`, `instance_creation`
- `instance_locking`, `kv_store`, `lock_expiration`, `long_polling`
- `management`, `multi_execution`, `poison_message`, `prune`, `queue_semantics`, `sessions`, `tag_filtering`

Any `Provider` implementation can run this suite by enabling `features = ["provider-test"]` and calling the test harness. The docs (`docs/provider-testing-guide.md`) provide a comprehensive guide for custom provider authors.

### Integration Tests

`tests/e2e_samples.rs` — comprehensive end-to-end samples covering all major patterns. `tests/unit_tests.rs` — unit tests. `tests/replay_engine/` — dedicated replay engine test suite including `tests/scenarios/`.

### Stress Tests

`sqlite-stress/` crate — standalone stress tool. `./run-tests.sh` two-pass test suite.

**Comparison with Nebula:** Nebula has `nebula-testing` crate + `resource-author-contracts.md` (contract tests for resource implementors) + insta/wiremock/mockall. duroxide's provider validation suite is actually deeper and more systematically organized than Nebula's resource contract testing.

---

## 16. AI / LLM Integration [A21] — DEEP

### A21.1 — Existence

**No built-in AI/LLM integration is implemented.** There are detailed design proposals.

**Grep evidence in `src/` (runtime code):**
- `grep -r "openai\|anthropic\|llm\|embedding\|completion\|gpt\|claude" --include="*.rs" src/` — 0 matches
- `grep -r "llm\|openai\|ai" --include="*.rs" src/` — 0 matches

**In documentation proposals only:**
- `docs/proposals/llm-integration.md` — detailed design for an LLM provider and dynamic orchestration construction
- `docs/proposals/durable-copilot-sdk-features.md` — feature requests from building `durable-copilot-sdk` (a real external use case)

### A21.2 — Provider Abstraction (Proposed)

The `docs/proposals/llm-integration.md` designs an `LlmProvider` trait:
```rust
pub trait LlmProvider: Send + Sync {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError>;
    fn name(&self) -> &str;
    fn model(&self) -> &str;
}
```
Planned implementations: `OpenAiProvider`, `AzureOpenAiProvider`, `AnthropicProvider`, `OllamaProvider`, `MockLlmProvider`. Multi-provider support is designed from the start.

### A21.3-A21.13 — All Sub-Axes

**None are implemented.** The proposals cover:
- A21.3 Prompt mgmt: designed in `docs/proposals/llm-integration.md` with `generate_with_system()`, `classify()`, `extract_features()`
- A21.4 Structured output: `generate_structured<T>()` with JSON Schema
- A21.5 Tool calling: `describe_tools()` returns activity descriptions as tool schemas
- A21.6 Streaming: Not mentioned in proposals
- A21.7 Multi-agent: Dynamic orchestration construction via LLM planning loop (`docs/proposals/llm-integration.md:210-279`)
- A21.8 RAG/vector: Not mentioned
- A21.9 Memory/context: `LlmOptions { include_history, include_activity_results }` proposed
- A21.10 Cost/tokens: `tokens_used` field in `LlmCompleted` event proposed
- A21.11-A21.12: Not mentioned in proposals

**GitHub issues:** Issues #21, #22, #23 are referenced in the LLM proposal but not present in the current issue list (suggesting they may have been created in the companion `duroxide-pg` or `toygres` repos, or not yet filed).

### A21.13 — vs Nebula + Surge

Nebula's current position is "no first-class LLM yet — AI = generic actions + plugin LLM client". duroxide's current position is identical: no implementation, only proposals. However, duroxide's proposals are **remarkably detailed** and suggest the maintainer has a concrete vision for making LLM calls first-class replay-safe orchestration primitives (recording `LlmRequested`/`LlmCompleted` events in history). This is architecturally interesting — it would make LLM calls durable, retryable, and debuggable through the same replay mechanism used for activities.

The `durable-copilot-sdk` use case indicates a real external party has already built an LLM agent framework on top of duroxide manually, validating the approach.

---

## 17. Notable Design Decisions

### DD1: Replay-Based Execution vs Checkpoint-Based

duroxide chose **full history replay** rather than checkpoint/snapshot-based recovery. On each turn, the orchestration function executes from line 1 with all historical completions pre-loaded. This requires the orchestration code to be deterministic (no random, no current time, no direct I/O) but eliminates the need for a serialization format for orchestration state. The `ctx.new_guid()` and `ctx.utc_now_ms()` syscall activities ensure determinism for "random" values. **Trade-off:** O(history) replay cost per turn (mitigated by history cap of 1024 events); **benefit:** no state serialization complexity, any async Rust function can be an orchestration.

### DD2: KV Delta Table (v0.1.26)

The two-table KV model (`kv_store` + `kv_delta`) solves a subtle replay poisoning bug: if orchestration code reads the KV state, modifies it, and writes it back across multiple turns, naive snapshot-at-fetch would seed the KV with the accumulated current-execution value, making replay see values that were only computed during that execution. The delta table isolates in-flight mutations from the baseline, ensuring replay seeds from prior-execution state only. **Borrow consideration for Nebula:** This pattern is directly applicable if Nebula ever adds per-execution KV-style mutable state.

### DD3: Provider as Pure Storage, Runtime as Intelligence

The Provider trait (`src/providers/mod.rs`) is explicitly designed as "dumb storage". The runtime computes execution semantics (ContinueAsNew boundaries, execution IDs, status transitions) and passes them to the provider as explicit instructions via `ExecutionMetadata`. Providers do not inspect event content. **Trade-off:** Heavier RTT for each turn (provider cannot shortcut any logic); **benefit:** providers are interchangeable and testable via the generic validation suite.

### DD4: Activity Tags for Worker Specialization

Worker specialization via `TagFilter` and `with_tag()` on `DurableFuture` (`src/providers/mod.rs:8-121`, `src/lib.rs:630-662`). Workers declare which tags they accept via `RuntimeOptions::worker_tag_filter`. Tagged activities only route to matching workers. This is critical for real-world use cases like GPU workloads, in-memory session state, or specialized hardware — cases the `durable-copilot-sdk` use case drove. **Borrow consideration for Nebula:** Nebula could adopt tagged routing as a lighter-weight alternative to explicit resource-scoped dispatchers.

### DD5: Session Affinity

Session-pinned activities (`schedule_activity_on_session(name, input, session_id)`) route to the worker owning a session. Sessions have heartbeat leases, idle timeout, and crash recovery. The Session Manager background task handles renewal and cleanup (`src/runtime/mod.rs:306-330`). This enables "durable actor" patterns — stateful in-memory entities that survive crashes via replay. **Comparison with Nebula:** Nebula has no session affinity concept; activity execution is stateless per turn.

### DD6: Versioned Orchestrations with Capability Filtering

The `SemverRange` / `DispatcherCapabilityFilter` mechanism allows mixed-version cluster deployments. Dispatchers only fetch work items pinned at semver versions they can process (`src/providers/mod.rs:161-188`). This enables rolling deployments without downtime. **Borrow consideration for Nebula:** Nebula's version pinning strategy (if any) is less explicit.

---

## 18. Known Limitations / Pain Points

1. **No credential/secrets management.** Activities handle their own secrets. This limits out-of-the-box security posture for multi-tenant or enterprise use cases. (Design decision, not a bug.)

2. **No built-in trigger/schedule model.** Cron jobs, webhooks, and message queue consumers require external wiring. (GitHub issues #8, #9 hint at upcoming event model improvements.)

3. **Orphan queue message handling** (GitHub issues #4, #5). Messages for deleted/non-existent instances fall into the unregistered-handler backoff loop instead of clean discard. Fix planned but not merged.

4. **Stale activity cleanup** (GitHub issue #3). Activities for cancelled/deleted instances remain in the worker queue indefinitely. No TTL or background sweep.

5. **PostgreSQL provider races on concurrent startup** (GitHub issues #7, #10). The ecosystem `duroxide-pg` provider is not yet production-hardened for multi-node concurrent startup.

6. **History cap of 1024 events** forces use of `continue_as_new` for long-running workflows. This is an architectural constraint visible to users. The cap is configurable in tests but hardcoded in the runtime limits (`src/runtime/limits.rs`).

7. **String-typed I/O boundary.** All activities and orchestrations use `Result<String, String>` across the trait boundary. The typed wrappers (`register_typed`) add a serde layer but the error type is always `String` — no structured error propagation at the framework level.

8. **No REST API.** Embedding is the only deployment model. For managed/SaaS deployment, the user must build an HTTP layer themselves.

9. **No circuit breaker or adaptive retry.** Only fixed retry policies. A long-running activity that repeatedly fails will exhaust retry attempts with no circuit-breaking behavior.

---

## 19. Bus Factor / Sustainability

- **Maintainers:** 1 primary (`affandar`). Microsoft ownership provides organizational backing but development appears solo.
- **Commit cadence:** Very rapid — 0.1.20 → 0.1.28 in approximately 4 months. 28 patch versions in 5 months indicates active iteration.
- **Crates.io downloads:** 1.9K total, 1.4K recent — very early adoption; not yet widely used.
- **Documentation quality:** Exceptional for a young project. Detailed architecture docs, proposals, provider guides, observability guides, and a comprehensive orchestration guide (`docs/ORCHESTRATION-GUIDE.md` at 93KB).
- **Risk:** Solo maintainer with no declared succession plan. The Microsoft org affiliation may provide continuity.
- **Issue velocity:** Only 10 total issues in ~5 months — either low adoption or issues tracked elsewhere.
- **Test coverage infrastructure:** Strong (provider validation suite, e2e samples, stress tests) — suggests serious engineering investment.

---

## 20. Final Scorecard vs Nebula

| Axis | duroxide Approach | Nebula Approach | Verdict | Borrow? |
|------|-------------------|-----------------|---------|---------|
| A1 Workspace | 1 crate + 1 stress tool; all in `duroxide` with feature flags | 26 crates layered: nebula-error / nebula-resilience / nebula-credential / etc. Edition 2024 | Different decomposition — duroxide optimizes for embeddability; Nebula for modularity. Neither dominates | no — different goals |
| A2 DAG | Implicit execution tree via async/await; `join`/`select2` compositors; no compile-time DAG; no port typing | TypeDAG: L1 = static generics; L2 = TypeId; L3 = refinement predicates; L4 = petgraph soundness checks | Nebula deeper for complex graph safety; duroxide simpler to use for sequential/parallel patterns | no — Nebula's already better |
| A3 Action | 2 open traits (`OrchestrationHandler`, `ActivityHandler`); function closures; string I/O; versioned registry; no sealed kinds; no derive macros | 5 sealed action kinds (Process/Supply/Trigger/Event/Schedule); assoc `Input`/`Output`/`Error`; versioning via type identity; derive macros | Different decomposition — duroxide DurableTask-style vs Nebula workflow-engine-style | refine — versioned registry + capability filtering pattern is worth borrowing |
| A4 Credential | None — explicit omission; activities handle own secrets | State/Material split; CredentialOps trait; LiveCredential with watch(); blue-green refresh; OAuth2Protocol | Nebula deeper — comprehensive credential subsystem | no — Nebula's already better |
| A5 Resource | None — explicit omission; users manage their own pools/clients | 4 scope levels (Global/Workflow/Execution/Action); ReloadOutcome enum; generation tracking; on_credential_refresh | Nebula deeper — first-class resource lifecycle | no — Nebula's already better |
| A6 Resilience | Per-activity RetryPolicy (fixed/linear/exponential); error classification (Infrastructure/Configuration/Application/Poison); no CB/bulkhead/hedging | nebula-resilience: retry/CB/bulkhead/timeout/hedging; unified ErrorClassifier | Nebula deeper — full resilience palette; duroxide has retry only | no — Nebula's already better |
| A7 Expression | None — pure Rust closures, no DSL | 60+ functions; type inference; sandboxed eval; `$nodes.foo.result` syntax | Nebula deeper — expression engine required for visual workflow builder | no — different goals |
| A8 Storage | SQLite (bundled optional) + `Provider` trait for custom backends (PG via separate repo); 12 migration files; two-table KV model | sqlx + PgPool; Pg*Repo per aggregate; migrations/; PostgreSQL RLS for tenancy | Different decomposition — duroxide storage-agnostic with Provider trait; Nebula PG-primary | refine — Provider trait + generic validation suite pattern is excellent |
| A9 Persistence | Append-only event log; full replay per turn; history cap 1024; KV delta table for RMW isolation; `ack_orchestration_item` atomic commit | Frontier-based scheduler + checkpoint recovery; append-only execution log; state reconstruction via replay | Convergent — both use event sourcing/replay; duroxide is more explicit about the replay model | refine — KV delta table for RMW isolation is a concrete technique worth studying |
| A10 Concurrency | tokio runtime; two dispatchers (OrchestrationDispatcher N concurrent, WorkDispatcher M concurrent); peek-lock semantics; `Send + Sync` required throughout | tokio runtime; frontier scheduler with work-stealing semantics; `!Send` action support via thread-local sandbox | Competitor simpler, Nebula richer — Nebula supports `!Send` actors; duroxide requires `Send` everywhere | no — Nebula's already better |
| A11 Plugin BUILD | None — all activities compiled in-process; `Provider` trait for storage extension only | WASM sandbox planned; plugin-v2 spec; Plugin Fund commercial model; capability-based security | Nebula richer roadmap; duroxide has no plugin model | no — different goals |
| A11 Plugin EXEC | None — activities run in same process as runtime; no sandbox, no isolation | WASM + capability security (planned); wasmtime target | Nebula richer roadmap | no — different goals |
| A12 Trigger | External events (one-shot + FIFO queue); durable timers (duration); no webhook, no cron, no broker integration | TriggerAction with `Input = Config` (registration) + `Output = Event` (typed payload); Source → Event 2-stage | Nebula deeper — full trigger taxonomy; duroxide has primitives only | refine — FIFO queue semantics (persistent mailbox) is clean; study for Nebula's queue trigger implementation |
| A13 Deployment | Embedded library (single binary, in-process); no standalone server; no Docker image; no management UI | 3 modes from one binary: `nebula desktop`/`nebula serve`/cloud | Different decomposition — duroxide is a library, Nebula is an application | no — different goals |
| A14 Multi-tenancy | None — single database, no namespace/tenant isolation | nebula-tenant: schema/RLS/database isolation; RBAC; SSO/SCIM planned | Nebula deeper — enterprise multi-tenancy; duroxide single-tenant only | no — different goals |
| A15 Observability | `metrics` facade (zero-cost, user-installed exporter); structured logging via `tracing`; 3 log formats; correlation fields; replay-safe `ctx.trace_*` system activities | OpenTelemetry; structured tracing per execution (one trace = one workflow run); metrics per action | Different decomposition — duroxide more flexible (any metrics backend); Nebula richer (distributed traces) | refine — metrics facade pattern (zero-cost, pluggable exporter) is worth adopting |
| A16 API | Rust-only `Client` API; no HTTP; no REST/GraphQL/gRPC | REST API; OpenAPI spec; planned GraphQL/gRPC; OwnerId-aware | Nebula deeper for network API; duroxide appropriate for embedded | no — different goals |
| A17 Type safety | No GATs, no HRTBs, no typestate in core traits; type erasure through String; `register_typed` adds serde wrapper; versioned registry via `BTreeMap<semver::Version, Arc<dyn H>>` | Sealed traits; GATs for resource handles; HRTBs; typestate (Validated/Unvalidated); Validated<T> proof tokens | Nebula deeper — rich type system features; duroxide simpler/more accessible | no — Nebula's already better |
| A18 Errors | `ErrorDetails` enum with 4 variants (Infrastructure/Configuration/Application/Poison); `ProviderError { retryable: bool }` | nebula-error crate; ErrorClass enum; contextual errors | Convergent — both have structured error classification with retryability; duroxide's Poison variant is unique | refine — Poison message detection pattern (escalating attempt_count with explicit detection) is worth adopting |
| A19 Testing | Generic provider validation suite (20 modules); e2e samples; replay engine tests; stress tests; no separate testing crate | nebula-testing crate; resource-author-contracts.md; insta/wiremock/mockall | Competitor deeper for provider conformance testing; Nebula richer for action/credential mocking | yes — generic provider validation suite design is stronger than Nebula's resource contracts |
| A20 Governance | MIT license; Microsoft org; solo maintainer (`affandar`); no commercial model; alpha quality | Open core; Plugin Fund; planned SOC 2; solo maintainer (Vanya) | Convergent — both solo maintainer; Nebula has a commercial story; duroxide does not yet | no — different goals |
| A21 AI/LLM | No implementation; detailed proposal in `docs/proposals/llm-integration.md` for replay-safe LLM activities with history events (`LlmRequested`/`LlmCompleted`); `durable-copilot-sdk` external use case | No first-class LLM yet — strategic bet: AI = generic actions + plugin LLM client; Surge handles agent orchestration on ACP | Convergent — both recognize LLM integration as a future priority with no current implementation | refine — duroxide's proposed `LlmRequested`/`LlmCompleted` event approach is worth studying; replay-safe LLM calls are architecturally sound |

---

*Analysis performed 2026-04-26 against duroxide v0.1.28 (commit `283dcdc`). Source code at `C:/Users/vanya/RustroverProjects/nebula/.worktrees/nebula/hopeful-shirley-08de69/targets/duroxide/`.*
