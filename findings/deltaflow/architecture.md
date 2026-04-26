# deltaflow — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/mavdi/deltaflow
- **Stars/forks:** Not indexed by DeepWiki; community traction minimal (zero GitHub issues)
- **Last activity:** v0.6.0 released (commit `5907b0c`); CHANGELOG shows active work from 2025-12-08 to latest
- **License:** MIT (`LICENSE` in root)
- **Governance:** Solo project (author: mavdi). No governance documents, no contributor guide.
- **Maintainers:** Single maintainer — mavdi
- **Crates.io keywords:** workflow, pipeline, async, jobs, tasks (`Cargo.toml` lines 13–14)
- **Edition:** Rust 2021 (`Cargo.toml` line 8)
- **Status:** Explicitly experimental: "Warning: This is an experimental project under active development. APIs may change without notice. Not recommended for production use." (`README.md` lines 1–4)

---

## 1. Concept positioning [A1, A13, A20]

**Author's own sentence (README.md line 6):** "The embeddable workflow engine."

**My characterization (after reading source):** A single-process, in-memory pipeline library with an optional SQLite task queue and periodic scheduler, designed to be embedded as a Rust library rather than deployed as a separate service. Pipelines are linear chains of typed steps with branching (fork/fan-out/emit) routing output to other named pipelines via a SQLite-backed task store.

**Comparison with Nebula:** Deltaflow is a library, not an engine. It makes an explicit design choice to forbid distributed execution and DAG dependencies (README.md "Not planned by design" section). Nebula is a full workflow orchestration engine with credential management, multi-tenancy, resilience crate, plugin spec, and a 4-level TypeDAG. Deltaflow is roughly the equivalent of only Nebula's core pipeline execution path stripped of almost all surrounding subsystems.

---

## 2. Workspace structure [A1]

**Crate count:** 2 workspace members (`Cargo.toml` lines 1–3):
- `deltaflow` (root) — core library: pipeline, step, retry, recorder, runner (sqlite-gated), scheduler (sqlite-gated)
- `deltaflow-harness` — dev-dependency companion: web visualizer serving a React frontend over axum

**Feature flags:**
- `default = []` — core library with no storage
- `sqlite` — gates `runner`, `scheduler`, and `SqliteRecorder` modules (`Cargo.toml` lines 16–17; `src/lib.rs` lines 77–96)

**Layer separation:** Minimal. No crate-level layering by domain. The single crate covers all concerns: trait definitions, pipeline builder, retry policy, recorder trait, task store trait, SQLite implementations, and scheduler. `deltaflow-harness` is purely a dev/visualization tool.

**Comparison with Nebula:** Nebula has 26 crates with layered separation (nebula-error → nebula-resilience → nebula-credential → nebula-resource → nebula-action → nebula-engine). Deltaflow has 2 crates with no domain boundary enforcement.

---

## 3. Core abstractions [A3, A17] — DEEP

### A3.1 Trait shape

The core abstraction is the `Step` trait, defined in `src/step.rs`:

```
src/step.rs:37-49
pub trait Step: Send + Sync {
    type Input: Send + Sync + Clone;
    type Output: Send + Sync;
    fn name(&self) -> &'static str;
    async fn execute(&self, input: Self::Input) -> Result<Self::Output, StepError>;
}
```

**Sealed/open:** Open — any external crate can implement `Step`. No sealed trait mechanism exists. No `#[doc(hidden)]` module trick or private supertrait.

**`dyn` compatibility:** The `Step` trait itself is NOT intended for `dyn` use due to associated types. Instead, an internal `BoxedStep<I, O>` trait is used for type-erased storage inside pipeline chains (`src/pipeline.rs:168-172`). The public-facing pipeline uses typestate generics, not trait objects, for the chain.

**Associated types count:** 2 — `Input` and `Output`. No `Error`, `Config`, `Context`, or `State` associated types. Error is fixed to `StepError` (the concrete enum from `src/step.rs`).

**GATs:** None. The trait uses simple associated types with lifetime-free bounds.

**HRTBs:** None found in the trait or its blanket implementations.

**Typestate:** The pipeline builder uses typestate via generic type parameters (`Pipeline<I, O, Chain>`) to enforce at compile time that each step's `Output` matches the next step's `Input` (`src/pipeline.rs:406-411`). This is zero-cost typestate via `PhantomData` and generic constraints, not a `Validated<T>`-style proof token.

**Default methods:** None on `Step`. The `name()` method must be implemented manually.

### A3.2 I/O shape

**Input:** `type Input: Send + Sync + Clone`. Clone is required because the retry loop at `src/pipeline.rs:272` calls `input.clone()` before each retry attempt. No `Serialize`/`Deserialize` bound on `Step::Input` at the trait level — that constraint is imposed by `BuiltPipeline::run()` which requires `I: HasEntityId` (`src/pipeline.rs:1190-1193`) and by `ErasedPipeline` which requires `I: DeserializeOwned + Serialize` (`src/runner/erased.rs:33-35`).

**Output:** `type Output: Send + Sync`. No serialization bound at the `Step` level; `Serialize` is required when calling `BuiltPipeline::run()` (`src/pipeline.rs:1193`) so the output can be passed to spawn rules.

**Type-erased input/output:** When dispatched through the Runner, tasks are stored and retrieved as `serde_json::Value` (`src/runner/store.rs:14`, `src/runner/erased.rs:20`). Serialization is done by `ErasedPipeline::run_erased()` (`src/runner/erased.rs:46-63`).

**Streaming output:** None. `execute()` returns a single `Result<Output, StepError>`, not a stream. No `async_stream`, no `futures::Stream`.

**Side-effects model:** No formal side-effect modeling. Steps are expected to carry any external clients/repositories as fields (struct injection at construction time). No dependency injection framework.

### A3.3 Versioning

**No versioning system.** Steps are referenced by string name returned from `fn name() -> &'static str`. The task queue stores pipeline name as a string (`src/runner/store.rs:15`). There is no `v1`/`v2` suffix, no version field in the task record, no migration support. The `#[deprecated]` attribute is used only at the method level for `spawn_from` and `get_spawned` (`src/pipeline.rs:652`, `src/pipeline.rs:1285`), not for step versioning.

### A3.4 Lifecycle hooks

**Single lifecycle method:** `execute()` only. No `pre_execute`, `post_execute`, `cleanup`, `on_failure`, or `on_cancellation` hooks.

**Async:** `execute()` is `async` (via `async_trait`).

**Cancellation:** No cancellation points or cancellation token. The `run()` loop in `Runner` (`src/runner/executor.rs:33`) is `async fn run(&self) -> !` — infinite loop with `tokio::time::sleep`. No graceful shutdown mechanism is defined.

**Idempotency key:** None. The `StoredTask` has an `id: TaskId(i64)` but no idempotency key field (`src/runner/store.rs:13-18`).

### A3.5 Resource and credential dependencies

Steps carry their dependencies as struct fields set at construction time. No declaration mechanism, no associated type for resources, no compile-time check. A step needing a database pool would hold `Arc<SqlitePool>` as a field and accept it in its constructor. There is no resource lifecycle or injection interface.

### A3.6 Retry/resilience attachment

**Pipeline-level only.** `RetryPolicy` is set once on the pipeline via `with_retry()` and applies uniformly to every step in the chain (`src/pipeline.rs:426-429`). No per-step retry policy (README explicitly lists "Per-step retry policies" as a planned future feature). The retry is a simple loop inside `ChainedStep::run()` and `ThenChain::run()` (`src/pipeline.rs:268-310`, `src/pipeline.rs:1116-1157`). No circuit breaker, no bulkhead, no timeout per step, no hedging.

### A3.7 Authoring DX

**No derive macros.** Steps must be manually implemented. Minimum "hello world" step requires implementing the `Step` trait with `name()` returning a `&'static str` and `execute()` returning a `Result`. Approximately 8 lines of code for the simplest possible step. No `#[derive(Step)]` macro available.

### A3.8 Metadata

Steps can carry optional `description` and `tags: HashMap<String, String>` via `Metadata` struct (`src/pipeline.rs:25-50`), attached via builder pattern (`.desc()`, `.tag()`) after each step-adding call (`src/pipeline.rs:495-508`). Metadata is runtime-only (not compile-time). Used exclusively for visualization output via `to_graph()`. No i18n support.

### A3.9 vs Nebula

Nebula has 5 action kinds (Process / Supply / Trigger / Event / Schedule), each a sealed trait kind with distinct assoc type sets, derive macros, and a clear role taxonomy. Deltaflow has exactly **1** concept: `Step`. There are no analogues to Supply (resource provision), Trigger (event source), Event (normalized payload), or Schedule (cron-driven). The single `Step` type covers what Nebula calls ProcessAction only. Everything is manual implementation — no derive macros, no kind taxonomy, no versioning.

---

## 4. DAG / execution graph [A2, A9, A10]

**Graph model:** Deltaflow explicitly rejects DAG. README.md (line 248): "DAG dependencies (pipelines are linear) — Not planned (by design)." Each pipeline is a strictly linear chain of steps: `I → Step1 → Step2 → ... → StepN → O`.

**Inter-pipeline routing** (fork/fan-out/emit) is not a DAG; it is a task-queue dispatch model. After a pipeline completes, its output is serialized to JSON and inserted as new tasks for target pipelines (`src/runner/executor.rs:91-100`). There is no shared execution context between pipelines. Cycles are possible in the task queue sense (pipeline A forks to B, B forks back to A via `emit`) but are not modeled as graph edges with cycle detection.

**Port typing:** None. There are no "ports" — the Step chain is a type-level linked list where each step's `Output` must match the next step's `Input` at compile time. The cross-pipeline data flow is type-erased through `serde_json::Value`.

**Compile-time checks:** Within a single pipeline, the type system enforces I→O→... matching via generics. Across pipelines, type safety is lost at the `serde_json::Value` boundary.

**Scheduler / concurrency model:** The `Runner` uses a tokio semaphore for global concurrency control (`src/runner/executor.rs:37`) and per-pipeline semaphores for rate-limited pipelines (`src/runner/executor.rs:17`, `src/runner/executor.rs:44-55`). Poll-based: every `poll_interval` (default 1 second), the runner claims available tasks and spawns `tokio::spawn` futures.

**Comparison with Nebula:** Nebula's TypeDAG has 4 levels (compile-time generics → TypeId → refinement predicates → petgraph soundness checks). Deltaflow has no DAG — linear only, with type safety deliberately dropped at inter-pipeline boundaries.

---

## 5. Persistence and recovery [A8, A9]

**Storage:** SQLite via sqlx (`Cargo.toml` lines 31-34). Two tables:
- `delta_tasks` — task queue with `status` column (pending/running/completed/failed), `scheduled_for` timestamp (`src/runner/sqlite_store.rs:24-45`)
- `delta_runs` + `delta_steps` — execution history for the `SqliteRecorder` (`src/sqlite.rs:9-33`)

**Schema management:** Inline DDL strings, not external migration files. `run_migrations()` splits by `;` and executes each statement (`src/sqlite.rs:49-57`; `src/runner/sqlite_store.rs:22-73`).

**Recovery:** `recover_orphans()` resets tasks stuck in `running` state back to `pending` on startup (`src/runner/sqlite_store.rs:310-325`). This is crash recovery for the single-process case.

**No checkpoint/frontier model.** There is no append-only execution log, no event sourcing, no replay-based state reconstruction. Recovery is purely "reset stuck tasks to pending and re-execute."

**Comparison with Nebula:** Nebula uses PostgreSQL with RLS, migration-managed schema (`migrations/`), and a frontier-based append-only execution log with replay. Deltaflow uses SQLite with inline DDL and at-most-once crash recovery with no replay guarantee.

---

## 6. Credentials / secrets [A4] — DEEP

### A4.1 Existence

**No credential layer.** Negative finding confirmed by exhaustive grep:

```
grep -ri "credential" --include="*.rs" --include="*.toml" .   → 0 results
grep -ri "secret"     --include="*.rs" --include="*.toml" .   → 0 results
grep -ri "token"      --include="*.rs" --include="*.toml" .   → 0 results
grep -ri "auth"       --include="*.rs" --include="*.toml" .   → 0 results
```

No dependencies for `secrecy`, `keyring`, `aws-secretsmanager-client`, `vault`, or any credential management library appear in `Cargo.toml`. This is an explicit omission — the project is a library that expects callers to provide any secrets through struct constructors on their step implementations.

### A4.2–A4.9

All absent. No at-rest encryption, no in-memory protection (`Zeroize`, `secrecy::Secret<T>`), no lifecycle, no OAuth2/OIDC, no scope model, no type safety around secrets.

**vs Nebula:** Nebula has a full credential subsystem (State/Material split, LiveCredential with watch(), blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter type erasure). Deltaflow has none of this — not even env-var helpers.

---

## 7. Resource management [A5] — DEEP

### A5.1 Existence

**No resource abstraction.** Negative finding confirmed by exhaustive grep:

```
grep -ri "resource" --include="*.rs" --include="*.toml" .   → 0 results
```

Steps carry their own external dependencies (database pools, HTTP clients, etc.) as struct fields. No shared pool management, no lifecycle interface, no scope levels.

### A5.2–A5.8

All absent. No scope levels, no init/shutdown hooks, no hot-reload, no generation tracking, no credential-refresh notification, no backpressure on resource acquisition.

**vs Nebula:** Nebula has 4 scope levels (Global/Workflow/Execution/Action), `ReloadOutcome` enum, generation tracking, and `on_credential_refresh` per-resource hook. Deltaflow has none of this.

---

## 8. Resilience [A6, A18]

**Retry:** Pipeline-level `RetryPolicy` with 3 variants: `None`, `Fixed { max_attempts, delay }`, `Exponential { max_attempts, initial_delay, max_delay }` (`src/retry.rs:7-29`). The retry logic is inlined in `ChainedStep::run()` and `ThenChain::run()` with per-attempt `tokio::time::sleep`.

**Error classification:** Two-variant `StepError` enum (`src/step.rs:8-16`): `Retryable(anyhow::Error)` and `Permanent(anyhow::Error)`. The step author decides at error creation time whether the error is retryable. The retry loop respects this classification (`src/pipeline.rs:289`).

**No circuit breaker, no bulkhead, no timeout, no hedging.** Confirmed by grep: no `circuit_breaker`, `bulkhead`, `timeout` in any `.rs` file.

**vs Nebula:** Nebula has a dedicated `nebula-resilience` crate with retry/CB/bulkhead/timeout/hedging and a unified `ErrorClassifier`. Deltaflow has inline retry with manual classification — no separate resilience crate, no circuit breaker.

---

## 9. Expression / data routing [A7]

**No expression engine.** There is no DSL, no `$nodes.foo.result` syntax, no templating, no type inference engine, no sandboxed evaluator. Data routing decisions are made by Rust closures passed to `fork_when()` (`src/pipeline.rs:442-453`) — these are plain `Fn(&Result<O, PipelineError>) -> bool` predicates compiled into the binary.

**Comparison with Nebula:** Nebula has 60+ functions, JSONPath-like expression syntax (`$nodes.foo.result.email`), type inference, and sandboxed eval. Deltaflow's equivalent is a Rust closure — more type-safe but requires recompilation to change routing logic.

---

## 10. Plugin / extension system [A11] — DEEP

### 10.A — Plugin BUILD process

**No plugin system.** Negative finding confirmed:

```
grep -ri "plugin" --include="*.rs" --include="*.toml" .   → 0 results
grep -ri "wasm"   --include="*.rs" --include="*.toml" .   → 0 results
```

No manifest format, no build toolchain, no registry, no discovery mechanism. The `Step` trait is open — users add steps by implementing the trait directly in their own Rust code and linking it into their binary. This is extension by library composition, not a plugin system.

**A11.1–A11.4:** All absent.

### 10.B — Plugin EXECUTION sandbox

**No execution sandbox.** No WASM runtime, no subprocess model, no IPC, no capability security. All steps execute in the same process, in the same memory space, with no isolation.

**A11.5–A11.9:** All absent.

**vs Nebula:** Nebula targets WASM sandbox with wasmtime, plugin-v2 spec, and Plugin Fund commercial model. Deltaflow's extension model is simply "implement the `Step` trait in your crate."

---

## 11. Trigger / event model [A12]

### A12.1 Trigger types

**Interval-based polling only.** The `PeriodicScheduler` supports interval-based triggers (`src/scheduler/builder.rs:82-91`). No webhook, no cron syntax, no external broker (Kafka/RabbitMQ/NATS), no filesystem watch, no database CDC, no manual trigger via API call.

### A12.2 Webhook

**None.** No webhook registration, no URL allocation, no HMAC verification.

### A12.3 Schedule

Interval-based only (`Duration`). No cron expression, no timezone handling, no DST awareness, no missed-schedule recovery beyond orphan reset. The `trigger()` method on `SchedulerBuilder` emits `DateTime<Utc>` to a target pipeline at each interval (`src/scheduler/builder.rs:82-91`). `run_on_start: bool` controls immediate first execution.

### A12.4 External events

**None.** No Kafka, RabbitMQ, NATS, Redis Streams, or any broker integration. No pubsub.

### A12.5 Reactive vs polling

Default and only model: polling. The `Runner` polls SQLite on a fixed `poll_interval` (default 1 second). The `PeriodicScheduler` uses `tokio::time::sleep` between firings.

### A12.6 Trigger to pipeline dispatch

1:1 — each trigger fires to exactly one named pipeline. No fan-out from trigger. The enqueued task carries serialized `serde_json::Value` input.

### A12.7 Trigger as action

**Triggers are not a kind of Step.** `TriggerNode` is a visualization struct only (`src/scheduler/graph.rs`). `PeriodicScheduler` is a standalone struct, not a pipeline step. It enqueues tasks to the `TaskStore` at intervals — it does not participate in the step chain.

### A12.8 vs Nebula

Nebula has a 2-stage model: Source → Event → TriggerAction, with the `TriggerAction` trait having `Input = Config` (registration config) and `Output = Event` (typed payload). Deltaflow has a 1-stage flat model: scheduler fires → enqueue JSON to task queue. No typed event envelope, no Source normalization layer.

---

## 12. Multi-tenancy [A14]

**None.** No tenant concept, no RBAC, no SSO, no SCIM. The library is single-process single-user by design.

---

## 13. Observability [A15]

**Basic tracing.** `tracing = "0.1"` is a dependency (`Cargo.toml` line 29). It is used only in `src/scheduler/job.rs` with `debug!`, `error!`, and `info!` macros for scheduler lifecycle events. No structured spans, no OpenTelemetry, no metrics.

The `Recorder` trait (`src/recorder.rs:33-50`) captures pipeline run and step start/complete events with timestamps. The `SqliteRecorder` persists these to the `delta_runs` and `delta_steps` tables. No trace IDs, no parent span propagation, no distributed tracing.

---

## 14. API surface [A16]

**Library only.** No network API. The public API surface is the Rust crate API: `Step`, `Pipeline`, `BuiltPipeline`, `Runner`, `RunnerBuilder`, `SchedulerBuilder`, `RetryPolicy`, `Recorder`, `TaskStore`. No REST, no GraphQL, no gRPC, no OpenAPI spec.

---

## 15. Testing infrastructure [A19]

No dedicated testing crate. `src/retry.rs:96-142` contains unit tests for `RetryPolicy`. Integration tests are in `tests/` (not read, but present in directory listing). No contract test framework, no `insta`, no `wiremock`, no `mockall` in dependencies.

---

## 16. AI / LLM integration [A21] — DEEP

### A21.1 Existence

**No AI/LLM integration.** Negative finding confirmed:

```
grep -ri "openai"     --include="*.rs" --include="*.toml" .   → 0 results
grep -ri "anthropic"  --include="*.rs" --include="*.toml" .   → 0 results
grep -ri "llm"        --include="*.rs" --include="*.toml" .   → 0 results
grep -ri "embedding"  --include="*.rs" --include="*.toml" .   → 0 results
grep -ri "completion" --include="*.rs" --include="*.toml" .   → 0 results (only in comments about "retry cycle")
```

No LLM client library, no prompt management, no structured output, no tool calling, no streaming, no multi-agent patterns, no RAG, no vector store integration, no token counting, no content filtering.

### A21.2–A21.13

All absent. No provider abstraction, no prompt templating, no JSON schema enforcement, no function/tool calling definition format, no SSE streaming, no multi-agent hand-off, no embeddings, no context window management, no cost tracking, no per-LLM-call tracing, no content filtering.

**vs Nebula+Surge:** Nebula's strategic bet is "AI = generic actions + plugin LLM client." Deltaflow does not even reach that level — there is no plugin system and no LLM-specific action. A user could implement an `LlmStep` using the `Step` trait with an OpenAI client held as a struct field, but the framework provides zero AI-specific scaffolding. Surge (agent orchestrator on ACP) has no equivalent here.

---

## 17. Notable design decisions

**1. Single-process by design.** The explicit rejection of distributed execution and DAG dependencies (`README.md` lines 245-251) is a deliberate scope constraint. This trades capability for simplicity and embeddability. Trade-off: the library cannot scale horizontally; the SQLite backend is a single-file bottleneck. Applicability to Nebula: Nebula's multi-process/multi-tenant goals mean this tradeoff is not transferable, but the embedded-first framing may be useful for Nebula's "desktop mode."

**2. Typestate builder for type-safe linear chains.** The `Pipeline<I, O, Chain>` typestate with `ChainedStep` and `ThenChain` generics (`src/pipeline.rs:238-246`, `src/pipeline.rs:1077-1085`) encodes the step chain's I/O types at the type level, so `then(S)` requires `S::Input = O`. This gives compile-time I/O safety within a pipeline without any runtime overhead. Trade-off: complex type signatures, hard to serialize the full pipeline type. Applicability: Nebula already does this more elaborately with its 4-level TypeDAG.

**3. Type erasure at inter-pipeline boundary.** Cross-pipeline data flow is serialized to `serde_json::Value`. This is the escape hatch that allows the typed pipeline system to interoperate with a string-keyed task queue. Trade-off: all type safety is lost at fork/fan-out/emit boundaries; a malformed JSON payload will cause runtime deserialization errors, not compile-time errors. Applicability: Nebula's approach of typed ports through the full DAG is architecturally superior; this is a known weakness deltaflow accepts.

**4. SQLite as the only storage backend.** The `sqlite` feature flag gates the entire persistence story. No PostgreSQL, no Redis, no in-memory alternative. Trade-off: zero external infrastructure for development/testing but hard ceiling on throughput and no multi-process sharing. Applicability to Nebula: Nebula's SQLite-for-dev mode could be interesting, but the production story requires PostgreSQL.

**5. Open trait + manual impl as "plugin system."** There are no plugins; extension is by library composition. This is the simplest possible extension model but means all steps must be compiled in. No dynamic loading, no runtime extension. Trade-off: maximum type safety, zero runtime overhead, but no third-party plugin ecosystem possible. Nebula's WASM-based plugin plan is fundamentally different.

---

## 18. Known limitations / pain points

**GitHub Issues:** Zero issues — no community pain points captured on the tracker.

**README's own "Not planned" list** (README.md lines 245-251):
- Distributed execution is explicitly out of scope
- DAG dependencies are explicitly out of scope

**README's own "What's coming" list** (README.md lines 239-244):
- Per-step retry policies (current retry is pipeline-wide only)
- Task priorities (no priority queue in SQLite store)
- More storage backends (SQLite only currently)

**CHANGELOG breaking changes:**
- v0.5.0 (2025-12-18): `fork_when_desc()` removed; `spawn_from()` removed — consumer-facing API churn in a single release cycle.
- v0.6.0: `get_spawned()` deprecated in favor of `get_spawned_from_result()` — API still evolving.

---

## 19. Bus factor / sustainability

- **Maintainer count:** 1 (mavdi)
- **Commit cadence:** Active development from 2025-12-08 (v0.2.0) to v0.6.0; approximately 4 releases in ~6 weeks
- **Issues:** Zero open, zero closed — no community engagement
- **Stars/forks:** Not publicly visible from this analysis; DeepWiki confirms not indexed
- **Last release age:** v0.6.0 is most recent (within last 50 commits)
- **Risk:** High bus factor (1). No test suite visible in CI. Explicitly experimental. Breaking API changes in every minor version.

---

## 20. Final scorecard vs Nebula

| Axis | deltaflow approach | Nebula approach | Who's deeper / simpler / more correct | Borrow? |
|------|--------------------|-----------------|---------------------------------------|---------|
| A1 Workspace | 2 crates (deltaflow + harness), `sqlite` feature flag gates persistence | 26 crates layered: nebula-error / nebula-resilience / nebula-credential / nebula-resource / nebula-action / nebula-engine / nebula-tenant / etc. Edition 2024 | Nebula deeper — domain boundary enforcement, SRP. Deltaflow is one flat crate. | no — Nebula already has better structure |
| A2 DAG | No DAG by design. Linear pipelines only. Type-safe within pipeline via typestate generics; type-erased at inter-pipeline boundary via `serde_json::Value` (`src/pipeline.rs:1238-1278`). Cross-pipeline routing via task queue. | TypeDAG: L1 static generics, L2 TypeId, L3 refinement predicates, L4 petgraph soundness checks | Nebula deeper — full DAG model with 4 levels. Deltaflow's linear model is simpler but intentionally scoped. | no — different goals |
| A3 Action | 1 kind: `Step` trait. Open (any crate can implement). 2 assoc types (Input/Output). No GAT/HRTB. No lifecycle hooks except `execute()`. No derive macros. Pipeline-level retry only. No versioning. (`src/step.rs:37-49`) | 5 action kinds (Process/Supply/Trigger/Event/Schedule). Sealed trait. Input/Output/Error assoc types. Versioning. nebula-derive macros. | Nebula deeper — richer taxonomy, sealed, macro-driven, versioned. Deltaflow's single-kind approach is simpler DX but lacks expressiveness for complex orchestration. | no — Nebula already richer |
| A11 Plugin BUILD | No plugin system. Extension by implementing `Step` in-process. No manifest, no registry, no build toolchain. (grep: 0 results for "plugin", "wasm") | WASM sandbox planned (wasmtime), plugin-v2 spec, Plugin Fund commercial model, capability-based security | Nebula deeper (planned) — WASM isolation is a fundamentally different safety model. | no — different goals |
| A11 Plugin EXEC | No execution sandbox. All steps in same process/memory space. No capability enforcement. | WASM sandbox + capability security (planned) | Nebula deeper — isolation model. | no |
| A18 Errors | `StepError` enum (Retryable/Permanent wrapping `anyhow::Error`). `PipelineError` enum (StepFailed/RetriesExhausted/RecorderError). `TaskError` enum in runner. All use `thiserror`. (`src/step.rs:8-16`, `src/pipeline.rs:122-144`) | `nebula-error` crate, `ErrorClass` enum (transient/permanent/cancelled/etc.), used by `ErrorClassifier` in resilience crate | Different decomposition — deltaflow's 2-variant `StepError` maps roughly to Nebula's transient/permanent distinction. Deltaflow's approach is simpler (2 variants vs full `ErrorClass`); Nebula's is richer with classification used by CB/bulkhead/hedging. | refine — deltaflow's `Retryable`/`Permanent` naming is cleaner than "transient"/"permanent" — worth considering as alias |
| A21 AI/LLM | None. No LLM client, no prompt management, no structured output, no tool calling, no streaming, no RAG. Zero grep results for openai/anthropic/llm/embedding/completion. | No first-class LLM abstraction. Strategic bet: AI workflows via generic actions + plugin LLM client. Surge handles agent orchestration on ACP. | Convergent absence — both have no first-class AI integration; both bet on external integration. Neither is more correct here. | no — different goals |
