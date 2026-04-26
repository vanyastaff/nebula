# orka — Architectural Decomposition

## 0. Project Metadata

- **Repo:** https://github.com/excsn/orka
- **crates.io:** https://crates.io/crates/orka v0.1.0
- **Stars:** 5 / **Forks:** 4 (as of 2026-04-26)
- **Last commit:** 2025-12-10 (`963db80 should use orkaresult more`)
- **First commit:** 2025-05-17 (`235551a First Commit`)
- **License:** MPL-2.0 (`core/LICENSE`)
- **Author:** Excerion Sun / normano (`dev@excsn.com`)
- **Maintainer count:** 1 (single contributor in GitHub contributor API)
- **Editions:** Rust 2021 (`core/Cargo.toml:6`)
- **Downloads:** 665 total, 96 recent

---

## 1. Concept Positioning [A1, A13, A20]

**Author's own description (core/README.md:6):**
> "An asynchronous, pluggable, and type-safe workflow engine for Rust, designed to orchestrate complex multi-step business processes with robust context management and conditional logic."

**One-sentence mine, after reading the code:**
Orka is a sequential, generics-based in-process pipeline framework for Rust: a `Pipeline<TData, Err>` runs named steps in order, each step can have `before`/`on`/`after` hook phases, and a `ConditionalScopeBuilder` allows runtime dispatch into typed sub-pipelines; there is no persistence layer, no scheduler, no credential management, and no network API.

**Comparison with Nebula:**
Orka and Nebula share an emphasis on type safety and Tokio-based async execution, but their scope diverges sharply. Nebula is a full workflow *engine* targeting n8n+Temporal use cases with credential management, persistence, multi-tenancy, plugin sandboxing, and event triggers. Orka is a workflow *library* — it handles only in-process, in-memory pipeline definition and execution. The "typed workflow" claim refers to its generics-based pipeline type (`Pipeline<TData, Err>`) rather than a DAG with type-safe port connections.

---

## 2. Workspace Structure [A1]

The workspace root is `Cargo.toml` with two members:
```
members = ["core", "examples/ecommerce_app"]
```
- `core` — the publishable library crate (`orka` on crates.io)
- `examples/ecommerce_app` — a binary example (not published)

**Layer separation:** There is no explicit layering beyond library vs. example. The `core` crate is a flat single-crate library with five modules: `core`, `pipeline`, `conditional`, `registry`, `error`. No feature flags are declared in `core/Cargo.toml`.

**Nebula comparison:** Nebula has 26 crates in a strictly layered monorepo (foundational → domain → engine → infrastructure). Orka has 1 library crate. This is 25 fewer crates, which is appropriate for Orka's narrower scope, but means there is no isolation between concerns (context management, error types, pipeline definition, and registry are all in one crate).

---

## 3. Core Abstractions [A3, A17] — DEEP

### A3.1 Trait shape

Orka does **not** use a sealed `Action` trait or equivalent. The unit of work is not a trait at all — it is a **type alias** for an async closure:

```rust
// core/src/core/context.rs:32
pub type Handler<TData, Err> = Box<
  dyn Fn(ContextData<TData>) -> Pin<Box<dyn Future<Output = Result<PipelineControl, Err>> + Send>>
    + Send + Sync,
>;
```

A handler takes `ContextData<TData>` (an `Arc<RwLock<TData>>` wrapper) and returns a pinned future that resolves to `Result<PipelineControl, Err>`. There is no trait that handlers must implement — they are anonymous async closures boxed at registration time.

The `Pipeline<TData, Err>` struct is the container:
```rust
// core/src/pipeline/definition.rs:23
pub struct Pipeline<TData, Err>
where
  TData: 'static + Send + Sync,
  Err: std::error::Error + From<crate::error::OrkaError> + Send + Sync + 'static,
```

The only "trait for a pipeline" is `AnyPipeline<E>` used for type erasure in the registry:
```rust
// core/src/core/pipeline_trait.rs:16
#[async_trait]
pub trait AnyPipeline<E>: Send + Sync
where
  E: std::error::Error + From<OrkaError> + Send + Sync + 'static,
{
  async fn run_any_erased(&self, ctx: &mut dyn Any) -> Result<PipelineResult, E>;
}
```

**Summary A3.1:** Not sealed (users can implement `PipelineProvider` freely). Not `dyn Handler` in the traditional sense — handlers are type-erased closures stored as `Box<dyn Fn...>`. No associated types (no `Input`/`Output`/`Error` type aliases on a trait). No GATs. No HRTBs. No typestate on the action/handler itself. The only typestate constraint is `Err: From<OrkaError>` at the `Pipeline<TData,Err>` struct level.

### A3.2 I/O shape

All handlers operate on `ContextData<TData>` — a shared `Arc<parking_lot::RwLock<TData>>`. There is no separate typed `Input` and `Output` for each step. The shared context is the only I/O channel between steps. This means all steps share one mutable blob of state; there are no typed ports connecting steps.

```rust
// core/src/core/context_data.rs:17
pub struct ContextData<T: Send + Sync + 'static>(Arc<RwLock<T>>);
```

Sub-contexts (`SData`) can be extracted from `TData` via `set_extractor` / `on<SData>`, but this creates a new independent `ContextData<SData>` (a clone or derived value), not a reference into the parent context. The documentation explicitly notes that sub-handler modifications to a cloned `SData` are **not** reflected back in the parent `TData` (see `core/tests/pipeline_execution_tests.rs:266`), making the extractor pattern primarily useful for read-only dispatch, not write-through.

**No streaming output.** No side-effects model beyond mutations on `ContextData`.

### A3.3 Versioning

No versioning of handlers or pipelines. Steps are addressed by string name only (e.g., `pipeline.on_root("step1", ...)` at `core/src/pipeline/hooks.rs:65`). No `#[deprecated]`. No version tagging in step definitions. Steps can be inserted/removed at build time via `insert_before_step`, `insert_after_step`, `remove_step` (`core/src/pipeline/definition.rs:98-154`), but there is no migration support.

### A3.4 Lifecycle hooks

Three hook phases per step: `before_root`, `on_root`, `after_root` (`core/src/pipeline/hooks.rs:42, 64, 81`). All are async. Each phase can hold multiple handlers executed in registration order. A handler returning `PipelineControl::Stop` halts the pipeline immediately (`core/src/pipeline/execution.rs:94`). There is no `cleanup` or `on-failure` hook. There are no explicit cancellation points (tokio cancellation is not wired in). No idempotency key concept.

### A3.5 Resource & credential deps

No mechanism. Handlers close over application state (e.g., a DB pool from `AppState`) via Rust closures. There is no framework-level way to declare "this handler needs resource X or credential Y". The example `ecommerce_app` injects `AppState` into `ContextData<CheckoutCtxData>` (which has an `app_state: Arc<AppState>` field), but this is a user pattern, not an orka-provided abstraction.

### A3.6 Retry / resilience attachment

No retry, circuit-breaker, backoff, or timeout at the framework level. Searched `targets/orka/core/src/` for `retry`, `circuit`, `bulkhead`, `timeout`, `backoff`, `hedge` — found zero matches. Resilience must be implemented inside individual handler closures.

### A3.7 Authoring DX

Handlers are plain async closures wrapped in `Box::pin(async move { ... })`:
```rust
pipeline.on_root("step1", |ctx: ContextData<MyCtx>| Box::pin(async move {
    ctx.write().count += 1;
    Ok(PipelineControl::Continue)
}));
```
Approximately 4 lines for a "hello world" step. No derive macros, no builder macros. IDE support is standard Rust trait/closure completion. The `async-trait` crate is used for `PipelineProvider` and `AnyConditionalScope` traits, which can hinder IDE inference on those boundaries.

### A3.8 Metadata

No display name, description, icon, or category metadata on steps. Steps have only a `String` name, `bool optional`, and `Option<SkipCondition<TData>>` (see `core/src/core/step.rs:16`). No i18n.

### A3.9 vs Nebula

| Dimension | orka | Nebula |
|-----------|------|--------|
| Action kinds | 1 (generic handler closure) | 5 (Process/Supply/Trigger/Event/Schedule) |
| Trait shape | no sealed trait; `Box<dyn Fn...>` type alias | sealed `Action` trait with assoc `Input`/`Output`/`Error` |
| I/O model | shared `ContextData<TData>` blob | typed `Input` → `Output` per action kind |
| Versioning | none | type identity |
| Derive macros | none | `nebula-derive` |
| Hook phases | before/on/after | pre/execute/post (per ProcessAction) |

Nebula's 5-action taxonomy maps different workflow concerns (triggers, events, schedules) to different trait shapes. Orka collapses everything into one generic handler function. This is simpler to learn but loses compile-time guarantees about what a given step produces or consumes.

---

## 4. DAG / Execution Graph [A2, A9, A10]

### Graph model

Orka is **not a DAG**. It is a **sequential pipeline** — an ordered `Vec<StepDef<T>>`. Steps execute one at a time in registration order. There is no branching in the graph topology (only conditional sub-pipeline dispatch within a step, which is itself sequential). There are no parallel branches, no join nodes, no fan-out, no fan-in.

```rust
// core/src/pipeline/execution.rs:44
for (step_idx, step_def) in self.steps.iter().enumerate() {
    // ... execute before/on/after handlers
}
```

### Port typing

Orka has no port concept. Steps share a single `ContextData<TData>` — a single typed but opaque blob. The `ConditionalScopeBuilder` does allow type-specialized sub-contexts (`Pipeline<SData, Err>`) accessed via user-written extractor functions, providing a limited form of type narrowing. However, the connection between parent context and child context is not verified at compile time beyond the extractor function's return type.

**Compile-time vs runtime:** The pipeline's context type `TData` is fixed at compile time. Sub-contexts `SData` are also compile-time typed (the extractor function is generic over `SData`). Registry dispatch is **runtime** — `Orka` uses `TypeId::of::<TData>()` as key (`core/src/registry.rs:141`). If the wrong `TData` is passed to `orka.run()`, a `TypeMismatch` runtime error results.

**Nebula comparison:** Nebula's TypeDAG L1-L4 enforces port types at compile time (L1), uses TypeId for dynamic registration (L2), adds refinement predicates (L3), and petgraph structural soundness checks (L4). Orka achieves only L1 for the primary context type, with no port-level typing, no predicate constraints, and no graph-level soundness checks.

### Concurrency model

Execution is sequential within a pipeline. Multiple pipelines or multiple pipeline invocations can be concurrent (Tokio-based), but a single `Pipeline::run()` call executes all steps sequentially. There is no work-stealing scheduler, no frontier model, no parallel step execution within a single pipeline run.

`ContextData<T>` uses `parking_lot::RwLock` (not `tokio::sync::RwLock`), and the documentation explicitly warns that lock guards must not be held across `.await` points (`core/src/core/context_data.rs:15-16`). This is a potential deadlock risk if a handler forgets to drop a guard before awaiting.

**`!Send` handling:** No special handling. `TData: Send + Sync` is required (`core/src/pipeline/definition.rs:25`). `!Send` types cannot be used as context data.

---

## 5. Persistence & Recovery [A8, A9]

**No persistence layer exists in orka.**

Searched `core/src/` for: `persist`, `snapshot`, `checkpoint`, `journal`, `event_sourcing`, `durable`, `database`, `sqlx`, `serde`, `deserialize` — zero matches in library code.

Orka is purely in-memory. Pipeline state (`ContextData<TData>`) lives on the heap for the duration of a `pipeline.run()` call. If the process crashes, all state is lost. There is no recovery mechanism, no durable journal, no append-only log, no checkpoint model.

The example `ecommerce_app` has placeholder comments like `// Placeholder for DB op: sqlx::query!(...)...` (e.g., `examples/ecommerce_app/src/pipelines/checkout_pipeline.rs:53`) that indicate intent to add persistence, but this is user-space concern, not an orka framework feature.

**Storage layer:** None. No sqlx, no migrations, no Postgres.

**Nebula comparison:** Nebula has frontier-based checkpoint recovery, append-only execution log, and state reconstruction via replay. Orka has none of these. Orka is not a durable workflow engine — it is a helper for organizing in-process logic, not for running long-lived durable workflows.

---

## 6. Credentials / Secrets [A4] — DEEP

**No credential layer exists in orka.**

Grepped `targets/orka/core/src/` for: `credential`, `secret`, `token`, `auth`, `OAuth`, `oauth`, `password`, `key_` — zero matches. This is confirmed.

### A4.1 Existence

No separate credential layer. Application credentials (API keys, DB passwords, OAuth tokens) must be managed entirely outside orka and passed into pipeline context data by the application.

### A4.2 Storage

N/A — no at-rest encryption, no backend, no key rotation.

### A4.3 In-memory protection

N/A — no `Zeroize`, no `secrecy::Secret<T>`, no lifetime limits on sensitive data.

### A4.4 Lifecycle

N/A — no CRUD, no refresh model, no expiry detection.

### A4.5 OAuth2/OIDC

N/A — no OAuth2 support.

### A4.6 Composition

N/A.

### A4.7 Scope

N/A.

### A4.8 Type safety

N/A.

### A4.9 vs Nebula

Nebula has: State/Material split (typed state + opaque material), `CredentialOps` trait, `LiveCredential` with `watch()` for blue-green refresh, `OAuth2Protocol` blanket adapter, `DynAdapter` for type erasure.

Orka has: none of these. This is an intentional omission — orka's README explicitly states: "Application-specific configuration (like database URLs, API keys for services called by handlers) is managed by the application using Orka, not by Orka itself." (`core/README.USAGE.md:286`). Orka treats credentials as out-of-scope.

---

## 7. Resource Management [A5] — DEEP

**No resource lifecycle abstraction exists in orka.**

Grepped `core/src/` for: `pool`, `resource`, `lifecycle`, `init`, `shutdown`, `health_check`, `reload`, `generation`, `backpressure` — zero framework matches.

### A5.1 Existence

No first-class resource abstraction. DB pools, HTTP clients, caches are injected as fields in the user's `TData` struct, typically via `Arc<T>`. The example app injects `AppState` (which contains a `db_pool`) into `CheckoutCtxData.app_state` (`examples/ecommerce_app/src/pipelines/checkout_pipeline.rs:72`).

### A5.2 Scoping

No scoping model. All resources are effectively global — they live in the application and are passed into the initial `ContextData`.

### A5.3 Lifecycle hooks

No framework lifecycle hooks for resources. Application code must manage init/shutdown.

### A5.4 Reload

No hot-reload, no blue-green swap, no `ReloadOutcome` enum, no generation tracking.

### A5.5 Sharing

Resources are shared via `Arc<T>` in user code.

### A5.6 Credential deps

No framework mechanism for a resource to declare credential dependencies or be notified of credential rotation.

### A5.7 Backpressure

No acquire timeout, no bounded queue.

### A5.8 vs Nebula

Nebula has 4 scope levels (`Global / Workflow / Execution / Action`), `ReloadOutcome` enum (`Reloaded / NoChange / Failed`), generation tracking for cache invalidation, and `on_credential_refresh` per-resource hook. Orka has none of these. Again, this is an intentional scope difference — orka is a pipeline orchestrator, not a resource lifecycle manager.

---

## 8. Resilience [A6, A18]

### Resilience (A6)

No resilience patterns exist in orka core. Grepped `core/src/` for `retry`, `circuit_breaker`, `bulkhead`, `timeout`, `backoff`, `hedging`, `classify` — zero matches.

The only resilience-related behavior is the `optional` flag on steps: if `optional: true`, a step with no handlers or a failing `AnyConditionalScope` with `is_step_optional_captured == true` (`core/src/conditional/builder.rs:143`) will be silently skipped/continued rather than failing the pipeline. This is a coarse degradation knob, not a retry or circuit-breaker.

**Nebula comparison:** Nebula has a dedicated `nebula-resilience` crate with retry / circuit breaker / bulkhead / timeout / hedging and a unified `ErrorClassifier` categorizing transient vs permanent failures. Orka has none of this — users must implement retry logic inside their handler closures.

### Errors (A18)

orka defines `OrkaError` as a `thiserror`-derived enum (`core/src/error.rs:6`):
```rust
pub enum OrkaError {
    StepNotFound { step_name: String },
    HandlerMissing { step_name: String },
    ExtractorFailure { step_name: String, source: AnyhowError },
    PipelineProviderFailure { step_name: String, source: AnyhowError },
    TypeMismatch { step_name: String, expected_type: String },
    HandlerError { source: AnyhowError },
    ConfigurationError { step_name: String, message: String },
    Internal(String),
    NoConditionalScopeMatched { step_name: String },
}
```

This is a reasonable framework error type. Wrapping `anyhow::Error` as `source` preserves context. The `From<AnyhowError>` impl is provided for external error wrapping. No `ErrorClass` or transient/permanent classification.

**Nebula comparison:** Nebula's `nebula-error` crate has `ErrorClass` enum (transient/permanent/cancelled/etc.) used by `ErrorClassifier` in resilience. Orka's `OrkaError` has no such classification — all errors are fatal to the current pipeline execution.

---

## 9. Expression / Data Routing [A7]

**No expression engine exists in orka.**

Searched `core/src/` for: `expression`, `eval`, `jmespath`, `jsonpath`, `template`, `sandbox`, `$nodes` — zero matches.

Data routing between steps is done entirely in handler closures via direct field access on `ContextData<TData>.read()` / `.write()`. There is no DSL, no template syntax, no `$nodes.foo.result.email`-style reference. Computed values must be explicitly set on the context struct by each handler.

**Nebula comparison:** Nebula has 60+ expression functions, type inference, a sandboxed evaluator, and a `$nodes.foo.result.email` syntax for referencing node outputs in workflow definitions. Orka has none of this — it is purely a code-based pipeline definition system.

---

## 10. Plugin / Extension System [A11] — DEEP

### 10.A — Plugin BUILD process

**No plugin system exists in orka.**

Searched `core/src/` for: `plugin`, `wasm`, `wasmtime`, `wasmer`, `wasmi`, `dlopen`, `libloading`, `dynamic_lib`, `cdylib`, `manifest` — zero matches.

**A11.1 Format:** None. No plugin package format, no manifest.

**A11.2 Toolchain:** None. No plugin SDK, no cargo extension, no scaffolding.

**A11.3 Manifest content:** None.

**A11.4 Registry/discovery:** None. No plugin registry.

The `ConditionalScopeBuilder` could be considered a "strategy pattern" or "soft plugin architecture" (as the README itself notes: `core/README.USAGE.md:338`: "This is useful for scenarios like selecting different payment gateways..."). However, this is compile-time Rust code composition, not a runtime plugin system with separate compilation, loading, or sandboxing.

### 10.B — Plugin EXECUTION sandbox

**A11.5 Sandbox type:** None. No WASM, no subprocess, no RPC. All "extension" code runs in-process as Rust closures.

**A11.6 Trust boundary:** None. All handlers run with full process trust.

**A11.7 Host↔plugin calls:** N/A — handlers are Rust closures called directly.

**A11.8 Lifecycle:** None beyond step execution.

**A11.9 vs Nebula:** Nebula targets WASM + capability security + Plugin Fund commercial model. Orka uses a different paradigm entirely: extensibility is achieved through Rust's type system (generics, closures) rather than dynamic loading. There is no commercial model for orka. The tradeoff is that orka's "plugins" are compile-time linked (safe, fast, no ABI concerns), while Nebula's WASM plugins are dynamic (runtime loading, isolation, independent versioning).

---

## 11. Trigger / Event Model [A12] — DEEP

**No trigger or event system exists in orka core.**

Searched `core/src/` for: `trigger`, `webhook`, `cron`, `schedule`, `interval`, `kafka`, `rabbitmq`, `nats`, `event_bus`, `pubsub`, `redis_stream`, `listen_notify`, `polling` — zero matches.

### A12.1 Trigger types

None. Pipelines are invoked programmatically via `pipeline.run(ctx_data).await` or `orka_registry.run(ctx_data).await`. There is no built-in mechanism to trigger a pipeline from a webhook, a cron schedule, an external event broker, a filesystem watch, or a DB change event.

### A12.2 Webhook

None. The `ecommerce_app` example has a `webhook_pipeline.rs` that shows how a user could *handle* a webhook (i.e., parse and route the payload inside a pipeline step), but the HTTP server that receives the webhook and invokes the pipeline is the user's responsibility (the example uses Actix-web). Orka provides no URL allocation, no HMAC verification, no idempotency key at the framework level.

### A12.3 Schedule

None. No cron support, no interval-based execution, no distributed locking for schedule deduplication.

### A12.4 External event

None. No broker integration.

### A12.5 Reactive vs polling

Not applicable — Orka has no event model.

### A12.6 Trigger→workflow dispatch

Not applicable.

### A12.7 Trigger as Action

Not applicable. Orka has no concept of a "Trigger" action kind.

### A12.8 vs Nebula

Nebula has `TriggerAction` with `Input = Config` (registration) and `Output = Event` (typed payload), a `Source` trait that normalizes raw inbound (HTTP request / Kafka message / cron tick) into a typed `Event`, and a 2-stage architecture. Orka has none of this. Triggers are entirely outside orka's scope — the user integrates with whatever event system they choose (Axum, Tokio tasks, cron crates, etc.) and then calls `pipeline.run()`.

---

## 12. Multi-tenancy [A14]

No multi-tenancy support. Searched for: `tenant`, `rbac`, `role`, `permission`, `schema_isolation`, `rls`, `scim`, `sso` — zero matches in any orka code.

The `Orka<ApplicationError>` registry has no tenant context. The `Pipeline<TData, Err>` type has no `OwnerId` or tenant discriminator.

**Nebula comparison:** Nebula has a `nebula-tenant` crate with three isolation modes (schema/RLS/database), RBAC, planned SSO/SCIM. Orka has none — it is a library for in-process orchestration with no multi-user or multi-tenant concept.

---

## 13. Observability [A15]

Orka uses the `tracing` crate (`tracing = "^0"` in `core/Cargo.toml:27`) for structured logging. All pipeline execution paths emit `tracing` events:

- `Pipeline::run()` has `#[instrument]` and emits events at `DEBUG`, `INFO`, `ERROR`, `TRACE` levels for each step phase (`core/src/pipeline/execution.rs:29-171`)
- `AnyConditionalScope::execute_scoped_pipeline` has `#[instrument]` (`core/src/conditional/scope.rs:80`)
- `ConditionalScopeBuilder::finalize_conditional_step` has `#[instrument]` (`core/src/conditional/builder.rs:111`)
- `PipelineWrapper::run_any_erased_with_owned_ctx` in registry has `#[instrument]` (`core/src/registry.rs:56`)

The `tracing::instrument` macro produces spans with fields like `pipeline_context_data_type`, `num_steps`, `step_name`, `step_index`, `optional`. These are consumer-configurable via `tracing-subscriber`.

**No OpenTelemetry integration.** No metrics (latency histograms, counters). No dedicated per-execution trace correlation. No structured log schema beyond what `tracing`'s default fields provide.

**Nebula comparison:** Nebula uses OpenTelemetry with structured per-execution tracing (one trace = one workflow run) and per-action metrics (latency/count/errors). Orka uses `tracing` event macros only — no metrics, no OTel, no trace correlation across distributed execution. For a library operating in a single process, `tracing` is appropriate; for a distributed workflow engine, it would be insufficient.

---

## 14. API Surface [A16]

**Programmatic Rust API only.** No network API (REST, GraphQL, gRPC). No OpenAPI spec. No CLI.

The public API surface is intentionally minimal:
- `Pipeline<TData, Err>` — construct, add handlers, add conditional scopes, run
- `ContextData<T>` — `Arc<RwLock<T>>` wrapper
- `Orka<ApplicationError>` — type-keyed registry for multiple pipelines
- `ConditionalScopeBuilder` / `ConditionalScopeConfigurator` — fluent builder
- `PipelineProvider` trait — for sourcing scoped pipelines dynamically
- `OrkaError`, `OrkaResult` — error types
- `PipelineControl`, `PipelineResult` — flow control enums

All re-exported from `core/src/lib.rs:25-41`.

The API is versioned only via crates.io semver (currently v0.1.0). No `#[deprecated]` annotations. No stability guarantees.

**Nebula comparison:** Nebula has a REST API with planned GraphQL/gRPC, OpenAPI spec generation, and `OwnerId`-aware per-tenant routing. Orka is a library — API concerns are out of scope.

---

## 15. Testing Infrastructure [A19]

orka has a reasonable testing foundation for a small library:

**Integration tests** (5 files in `core/tests/`):
- `pipeline_execution_tests.rs` — basic sequential execution, stop signal, error propagation, skip conditions, optional steps, before/on/after ordering, sub-context extraction
- `conditional_scope_tests.rs` — static and dynamic scopes, condition matching, no-match behavior
- `context_management_tests.rs` — lock discipline tests
- `error_handling_tests.rs` — OrkaError variants
- `registry_tests.rs` — `Orka<E>` registration and dispatch

**Benchmark** (`core/benches/orka_benchmarks.rs`): Criterion benchmarks with `tokio` async runtime.

**No dedicated testing utility crate** (unlike Nebula's `nebula-testing`). No wiremock, no mockall, no insta snapshot testing. No contract tests for user-implemented traits like `PipelineProvider`.

A notable test limitation acknowledged in the test file itself: sub-context extraction via clone means sub-handler mutations are not reflected back in the parent context, limiting the ability to test integrated state mutation (`core/tests/pipeline_execution_tests.rs:266`: `assert_eq!(guard.sub_data_container.processed, false); // Because sub-handler worked on a clone`).

**Nebula comparison:** Nebula has a `nebula-testing` crate with resource-author contract tests, insta + wiremock + mockall. Orka has standard Rust integration tests only.

---

## 16. AI / LLM Integration [A21] — DEEP

**No AI or LLM integration in orka.**

Grepped all orka source files for: `openai`, `anthropic`, `llm`, `embedding`, `completion`, `gpt`, `claude`, `llama`, `ollama`, `mistral`, `candle`, `rag`, `vector`, `prompt` — zero matches in library code. The only hit was the word "completion" in a comment about a signup form, which is unrelated (`examples/ecommerce_app/src/web/handlers/auth_handlers.rs:82`).

### A21.1-A21.13 All null

All A21 sub-questions are answered: No. This is expected for a low-level pipeline orchestration library. There is no built-in provider abstraction, no prompt management, no structured output, no tool calling, no streaming, no multi-agent patterns, no RAG/vector integration, no memory/context management, no cost tracking, no observability for LLM calls, and no content filtering.

**A21.13 vs Nebula:** Nebula's strategic bet is that AI workflows are realized as generic actions + plugin LLM client (no first-class LLM abstraction yet). Orka takes the same position by omission — LLM calls could be made inside handler closures, but the framework provides no abstractions for this. Neither project has built-in LLM support; the difference is that Nebula has an explicit strategic rationale documented, whereas orka simply does not address the topic.

---

## 17. Notable Design Decisions

### 17.1 Handler as closure, not trait impl

orka chose `Box<dyn Fn(ContextData<TData>) -> Pin<Box<dyn Future...>>` over a named trait that users implement. This reduces boilerplate dramatically (a 4-line closure vs. a 20-line struct with trait impl), but it also means:
- No associated types for typed I/O
- No compile-time discovery of what a step reads or writes
- No metadata (name, description) associated with the handler itself
- All type information is erased at registration time

**Trade-off:** Better ergonomics for small pipelines; worse type safety for large ones. Orka's DX is notably lighter than Nebula's but the trade-off is less structural guarantees.

### 17.2 ConditionalScope for typed branching

The `ConditionalScopeBuilder` is orka's most architecturally interesting mechanism. It allows a step to conditionally dispatch to `Pipeline<SData, Err>` instances — full pipelines with their own typed context. The extractor pattern (`Fn(ContextData<TData>) -> Result<ContextData<SData>, OrkaError>`) provides a typed narrowing from the parent context to the sub-context.

However, the sub-context is a *separate* allocation (either a clone or a newly created struct), not a typed view into the parent. This means sub-pipeline changes must be explicitly merged back into the parent context by a subsequent step, requiring coordination logic in the parent pipeline. The checkout example handles this via the `after_root` hook on the conditional step (`examples/ecommerce_app/src/pipelines/checkout_pipeline.rs:201`).

### 17.3 parking_lot::RwLock instead of tokio::sync::RwLock

orka uses `parking_lot::RwLock` for `ContextData<T>` rather than `tokio::sync::RwLock`. This means lock acquisition blocks the async thread, not just the task. The documentation repeatedly warns about this: "Lock guards obtained from this struct are blocking and MUST NOT be held across `.await` suspension points" (`core/src/core/context_data.rs:14`).

The choice appears motivated by API simplicity (parking_lot's `read()`/`write()` return guards directly without `await`), but it creates a correctness trap for users unfamiliar with async Rust: forgetting to drop a guard before an await point will deadlock or starve the thread pool. This is a significant footgun.

### 17.4 Type-keyed registry with TypeId dispatch

The `Orka<ApplicationError>` registry uses `TypeId::of::<TData>()` as the pipeline key (`core/src/registry.rs:141`). This means exactly one pipeline can be registered per `TData` type. This is a strict limitation: if you want two different pipelines over the same context type (e.g., an "order_checkout" pipeline and an "order_refund" pipeline over `OrderContext`), you must use distinct type wrappers.

### 17.5 Error type erasure via From<OrkaError>

The `Err: From<OrkaError>` bound on `Pipeline<TData, Err>` is a clean integration point. Users define their own application error enum, implement `From<OrkaError>` for it, and get transparent error conversion throughout the pipeline. This is idiomatic Rust error handling.

### 17.6 MPL-2.0 licensing

The Mozilla Public License 2.0 is a weak copyleft license — compatible with Apache 2.0 and MIT for linking, but modifications to orka's source files must be shared. This is distinct from MIT/Apache (permissive) and GPL (strong copyleft). For Nebula's competitor analysis, this means adopting orka patterns is legally fine but contributing changes to orka would require publishing those changes.

---

## 18. Known Limitations / Pain Points

### Issues sweep

`gh issue list --repo excsn/orka --state open --limit 100` and `--state closed --limit 50` both returned **empty arrays** (`[]`). The repository has zero filed issues (open or closed).

Given zero issues, the ≥3 issue citation requirement from the Worker Brief cannot be met — the repo has no issue tracker activity. This finding is itself informative: the project is too new (May 2025) to have accumulated community feedback.

The commit message `963db80 should use orkaresult more` suggests the author identified an internal API consistency problem (handler functions not consistently returning `OrkaResult`) and fixed it. This aligns with the acknowledged limitation in the codebase.

### Design limitations observed from code

1. **Sub-context write-through is broken** (`core/tests/pipeline_execution_tests.rs:266`): The test explicitly asserts that `sub_data_container.processed == false` even after the sub-handler set it to `true`, because the extractor creates a clone. This is a known limitation acknowledged in comments. Sub-pipelines cannot mutate parent context fields without explicit merge logic.

2. **Single pipeline per TData type** (`core/src/registry.rs:141`): `TypeId`-keyed registry prevents having multiple named workflows over the same context type.

3. **parking_lot deadlock risk** (`core/src/core/context_data.rs:14`): Blocking locks in async context are a user footgun.

4. **No persistence or durability**: For long-running business processes (the stated use case), losing state on process restart is a critical limitation. The README's use-case list includes "Financial Services: Loan application processing, trade execution workflows" — these cannot afford to lose state on crash.

5. **Erased types in handler storage**: `Box<dyn Fn...>` means no inspection of what handlers are registered (no `list_steps_with_handlers()`, no debugging of the pipeline shape beyond step names).

---

## 19. Bus Factor / Sustainability

- **Maintainer count:** 1 (normano / Excerion Sun)
- **Commits:** 2 total (as of depth-50 clone)
- **Issues:** 0 open, 0 closed
- **crates.io downloads:** 665 total (first published 2025-05-18)
- **Age:** ~11 months (May 2025 to April 2026)
- **Last update:** 2025-12-10 (~4 months before analysis date)

Bus factor: 1. This is a solo early-stage project. The low commit count and zero issue activity suggest either very early development stage or very low external adoption. The 4-month gap since last commit suggests the project may be in a dormant phase.

---

## 20. Final Scorecard vs Nebula

| Axis | orka approach | Nebula approach | Who's deeper / simpler / more correct | Borrow? |
|------|--------------|-----------------|---------------------------------------|---------|
| A1 Workspace | 1 library crate (core) + 1 example binary | 26 crates, layered: nebula-error / nebula-resilience / nebula-credential / nebula-resource / nebula-action / nebula-engine / nebula-tenant / nebula-eventbus / etc. Edition 2024. | Nebula deeper — crate boundaries enforce layer contracts. Orka's single crate is appropriate for its scope. | no — different goals |
| A2 DAG | Sequential `Vec<StepDef>`, no DAG; TypeId registry dispatch at runtime; no port typing; sub-context via typed extractor closures | TypeDAG: L1 = static generics enforce port types at compile time; L2 = TypeId for dynamic registration; L3 = refinement predicates; L4 = petgraph soundness checks. | Nebula deeper — full DAG with port typing vs sequential list. Orka's sub-context extractor idea (typed narrowing from TData to SData) is interesting but incomplete. | refine — the extractor pattern (typed sub-context) could inform Nebula's resource scoping |
| A3 Action | No sealed trait; `Box<dyn Fn(ContextData<TData>) -> ...>` type alias; before/on/after hooks; no Input/Output/Error assoc types; no metadata; no versioning | 5 action kinds (Process / Supply / Trigger / Event / Schedule). Sealed trait. Associated Input / Output / Error. Versioning via type identity. Derive macros via nebula-derive. | Nebula deeper — 5-kind taxonomy + sealed trait + assoc types vs single generic closure. Orka simpler for small use cases. | no — Nebula's already better |
| A4 Credential | None — explicitly out of scope; must be injected via user-defined ContextData struct | State/Material split (typed state + opaque material). CredentialOps trait. LiveCredential with watch() for blue-green refresh. OAuth2Protocol blanket adapter. DynAdapter for type erasure. | Nebula deeper — comprehensive credential subsystem vs zero | no — different goals |
| A5 Resource | None — resources injected as Arc<T> fields in user's TData; no scoping, no reload, no generation tracking | 4 scope levels (Global / Workflow / Execution / Action). ReloadOutcome enum (Reloaded / NoChange / Failed). Generation tracking. on_credential_refresh per-resource hook. | Nebula deeper — full resource lifecycle vs user-space injection | no — different goals |
| A6 Resilience | None — step `optional` flag is the only degradation knob; no retry/CB/timeout/backoff | nebula-resilience crate: retry / circuit breaker / bulkhead / timeout / hedging. Unified ErrorClassifier categorizing transient vs permanent. | Nebula deeper — no resilience primitives in orka at all | no — Nebula's already better |
| A7 Expression | None — data routing via direct Rust field access on ContextData | 60+ functions, type inference, sandboxed eval. Syntax: `$nodes.foo.result.email`. JSONPath-like + computed expressions. | Nebula deeper — full expression engine vs zero | no — different goals |
| A8 Storage | None | sqlx + PgPool. Pg*Repo per aggregate. SQL migrations. PostgreSQL RLS for tenancy. | Nebula deeper — production-grade Postgres layer vs nothing | no — different goals |
| A9 Persistence | None — pure in-memory; state lost on crash | Frontier-based scheduler with checkpoint recovery. Append-only execution log. State reconstruction via replay. | Nebula deeper — durable workflow engine vs ephemeral in-process pipeline | no — different goals |
| A10 Concurrency | tokio-based; sequential within a pipeline run; parking_lot::RwLock (blocking in async — user footgun); TData: Send + Sync required; no !Send isolation | tokio runtime. Frontier scheduler with work-stealing semantics. !Send action support via thread-local sandbox isolation. | Nebula deeper — frontier scheduler + !Send isolation vs sequential steps + parking_lot blocking lock | maybe — parking_lot::RwLock's map_read/map_write mapped guards are ergonomic; Nebula could expose similar |
| A11 Plugin BUILD | None — "plugins" are compile-time Rust closures; no manifest, no SDK, no registry | WASM sandbox planned (wasmtime). plugin-v2 spec doc. Plugin Fund commercial model. Capability-based security. | Nebula deeper — full WASM plugin spec (planned) vs zero | no — different goals |
| A11 Plugin EXEC | None — handlers run in-process with full process trust | WASM sandbox + capability security (planned) | Nebula deeper (planned); orka simpler (in-process closures are fast and safe for trusted code) | no — different goals |
| A12 Trigger | None — pipelines invoked programmatically; webhook example shows user-space handler pattern only | TriggerAction with Input = Config (registration) and Output = Event (typed payload). Source trait normalizes raw inbound. 2-stage. | Nebula deeper — full trigger model vs zero | no — different goals |
| A13 Deployment | Library only — no deployment mode; user embeds in their binary | 3 modes from one binary: nebula desktop (single-user GUI), nebula serve (self-hosted), cloud (managed). | Nebula deeper — 3 deployment modes; orka is a library, not an engine with deployment modes | no — different goals |
| A14 Multi-tenancy | None | nebula-tenant crate. Three isolation modes (schema / RLS / database). RBAC. SSO planned. SCIM planned. | Nebula deeper — comprehensive multi-tenancy vs none | no — different goals |
| A15 Observability | tracing events (DEBUG/INFO/ERROR/TRACE) + #[instrument] spans on pipeline execution; no metrics; no OTel | OpenTelemetry. Structured tracing per execution (one trace = one workflow run). Metrics per action (latency / count / errors). | Nebula deeper — OTel + metrics vs tracing events only. orka's span field choices (pipeline_context_data_type, step_name, step_index) are clean. | refine — orka's span field naming convention is clean; Nebula could adopt similar structured fields |
| A16 API | Programmatic Rust API only; no REST/GraphQL/gRPC; no OpenAPI | REST API now. GraphQL + gRPC planned. OpenAPI spec generated. OwnerId-aware (per-tenant). | Nebula deeper — network API vs library API | no — different goals |
| A17 Type safety | Pipeline<TData,Err> compile-time generics for context + error type; Err: From<OrkaError> bound; TypeId registry dispatch; no sealed traits, no GATs, no HRTBs, no typestate, no Validated<T> | Sealed traits (extern crates can't impl core kinds). GATs for resource handles. HRTBs for lifetime polymorphism. typestate (Validated/Unvalidated). Validated<T> proof tokens. | Nebula deeper — full type-safety stack vs basic generics + TypeId. Orka's claim of "type-safe" is modest — it means TData is compile-time typed, not that ports/I/O are typed. | no — Nebula's already better |
| A18 Errors | OrkaError (thiserror enum, 9 variants); wraps anyhow::Error for sources; From<AnyhowError> impl; no error classification | nebula-error crate. Contextual errors. ErrorClass enum (transient / permanent / cancelled / etc.). Used by ErrorClassifier in resilience. | Nebula deeper — ErrorClass + ErrorClassifier vs flat enum with no classification | refine — orka's `From<AnyhowError>` impl on OrkaError is clean; the double-wrap detection code (checking if `err` is already an `OrkaError`) is a nice pattern |
| A19 Testing | 5 integration test files; criterion benchmarks; no testing utility crate; no contract tests; sub-context write-through limitation acknowledged in test comments | nebula-testing crate. resource-author-contracts.md (contract tests for resource implementors). insta + wiremock + mockall. | Nebula deeper — dedicated testing crate + contract tests + snapshot testing vs standard Rust tests | no — Nebula's already better |
| A20 Governance | Open source (MPL-2.0); solo maintainer; v0.1.0; no commercial model | Open core. Plugin Fund (commercial model for plugin authors). Planned SOC 2 (2-3 year horizon). Solo maintainer (Vanya). | Different decomposition — both solo; Nebula has commercial model + SOC 2 roadmap; orka is a pure open-source library | no — different goals |
| A21 AI/LLM | None — grepped for openai/anthropic/llm/embedding/completion/gpt/claude — zero matches | (none yet — generic actions + LLM plugin) | Convergent — neither has first-class LLM abstraction | no — different goals |

---

## Summary

orka is a well-structured, ergonomically clean in-process pipeline library that achieves its stated goal: making sequential multi-step async processes more organized in Rust. Its `Pipeline<TData, Err>` generics approach provides compile-time safety for the context and error types, and the `ConditionalScopeBuilder` is an interesting mechanism for typed dispatch to sub-pipelines.

However, the "typed workflow engine" marketing claims more than the implementation delivers relative to Nebula. The typing is limited to the TData blob type — there are no typed ports, no per-step I/O types, no sealed action taxonomy, and no graph-level soundness checks. The framework has no persistence, no credentials, no resources, no resilience, no triggers, no plugins, no multi-tenancy, and no network API. These are not deficiencies — they are deliberate scope limits for a library that positions itself as an in-process orchestration helper rather than a full workflow engine.

For Nebula's design, the most interesting borrowable ideas are:
1. The `ConditionalScopeBuilder` pattern for typed sub-context dispatch (refine for resource scoping)
2. The clean `Err: From<OrkaError>` composition boundary (already similar in Nebula's design)
3. The structured `tracing::instrument` field naming for pipeline spans

---

## 21. Deep Question Answers — Explicit Summary

This section explicitly answers all mandatory Deep Questions for axes A3, A4, A5, A11, A12, A21 per Worker Brief §1.5, with grep evidence for negative findings.

### A3 — Action/Node structure

**A3.1** No sealed trait. No trait-object-dispatched `dyn Action`. Handler is a type alias `Box<dyn Fn(ContextData<TData>) -> Pin<Box<dyn Future<Output = Result<PipelineControl, Err>> + Send>> + Send + Sync>` (`core/src/core/context.rs:32`). Zero associated types on any "action trait". No GATs. No HRTBs in handler types. No typestate.

**A3.2** Input: `ContextData<TData>` (shared `Arc<RwLock<TData>>`). Output: `PipelineControl::Continue | Stop`. No separate typed Input/Output per step. No streaming. No serde requirement on TData (left to user). Side effects via `ctx_data.write()` mutations.

**A3.3** No versioning. Steps addressed by string name only. No `#[deprecated]`. No migration support.

**A3.4** Three hook phases: `before_root`, `on_root`, `after_root` (`core/src/pipeline/hooks.rs:42, 64, 81`). All async. No cleanup hook. No on-failure hook. No cancellation point integration. No idempotency key.

**A3.5** No framework-level resource/credential dependency declaration. Handlers close over application state via Rust captures (e.g., `Arc<AppState>` inside `TData`).

**A3.6** No retry or resilience at framework level. Grep: `grep -r "retry\|circuit\|bulkhead\|timeout\|backoff" targets/orka/core/src/` — zero matches.

**A3.7** Plain async closures via `Box::pin(async move { ... })`. Approximately 4 lines for minimal step. No derive macros, no builder macros.

**A3.8** No display name, description, icon, or category. Step metadata is `{name: String, optional: bool, skip_if: Option<SkipCondition>}` only (`core/src/core/step.rs:16`). No i18n.

**A3.9** orka has 1 action kind (generic handler). Nebula has 5 (Process/Supply/Trigger/Event/Schedule). Not sealed. No assoc types.

### A4 — Credentials

**A4.1** No credential layer. Grep: `grep -r "credential\|secret\|token\|auth\|OAuth\|password\|key_" targets/orka/core/src/ --include="*.rs"` — zero matches. Documented as intentional scope exclusion in `core/README.USAGE.md:286`.

**A4.2-A4.9** All null — no storage, no in-memory protection, no lifecycle, no OAuth2, no composition, no scoping, no type safety for credentials, no comparison applicable.

### A5 — Resource lifecycle

**A5.1** No resource abstraction. Grep: `grep -r "pool\|resource\|lifecycle\|reload\|generation" targets/orka/core/src/ --include="*.rs"` — zero matches. Resources injected as fields in user's `TData` via `Arc<T>`.

**A5.2-A5.8** All null — no scoping, no lifecycle hooks, no reload, no sharing model, no credential deps, no backpressure.

### A11 — Plugin system

**A11.1-A11.4 (BUILD)** No plugin system. Grep: `grep -r "plugin\|wasm\|wasmtime\|wasmer\|dlopen\|libloading\|manifest" targets/orka/ --include="*.rs"` — zero matches. No plugin format, toolchain, manifest, or registry.

**A11.5-A11.9 (EXEC)** No sandbox. All code runs in-process as Rust closures. No WASM, no subprocess, no RPC, no trust boundary, no host↔plugin marshaling.

### A12 — Trigger/Event model

**A12.1** No trigger types. Grep: `grep -r "trigger\|webhook\|cron\|schedule\|kafka\|rabbitmq\|nats\|event_bus\|pubsub" targets/orka/core/src/ --include="*.rs"` — zero matches. Pipelines invoked programmatically via `pipeline.run()` or `orka.run()`.

**A12.2-A12.8** All null — no webhook, no schedule, no external event integration, no reactive model, no trigger→dispatch, no TriggerAction analogue.

### A21 — AI/LLM integration

**A21.1** No AI/LLM integration. Grep: `grep -r "openai\|anthropic\|llm\|embedding\|completion\|gpt\|claude\|llama\|ollama\|mistral\|rag\|vector\|prompt" targets/orka/ --include="*.rs" -i` — zero matches in library code. The word "completion" appeared once in an example file in a comment about a signup form (`examples/ecommerce_app/src/web/handlers/auth_handlers.rs:82`) — unrelated to AI.

**A21.2-A21.12** All null. No provider abstraction, no prompt management, no structured output, no tool calling, no streaming, no multi-agent patterns, no RAG/vector, no memory management, no cost tracking, no LLM observability, no content filtering.

**A21.13** Neither orka nor Nebula has first-class LLM support. Convergent null. Nebula has an explicit strategic bet (AI = generic actions + plugin LLM client); orka simply doesn't address the topic.
