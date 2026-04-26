# fluxus — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/lispking/fluxus
- **License:** Apache 2.0
- **Current version:** 0.2.0 (workspace)
- **Rust toolchain pin:** 1.88.0 (stable), `rust-toolchain.toml` line 2
- **WASM target in toolchain:** `wasm32-unknown-unknown` (compilation target, not plugin sandbox)
- **Commits (depth-50 clone):** 92 visible
- **Latest tags:** `fluxus-api-v0.1.3`, `fluxus-core-v0.1.3`, `fluxus-runtime-v0.1.3`, `fluxus-sinks-v0.1.3`, `fluxus-sources-v0.1.3`
- **Governance:** "Fluxus Team" — no named maintainers in Cargo.toml; GitHub org is `fluxus-labs` (dependabot PRs from that org). Solo or small team project.
- **Edition:** 2024 (workspace-level, `Cargo.toml` line 5)
- **Stars / forks:** Not available from git clone; crates.io badge present in README.

---

## 1. Concept positioning [A1, A13, A20]

**Author's own description (README.md line 14):** "Fluxus is a lightweight stream processing engine written in Rust, designed for efficient real-time data processing and analysis."

**My description after reading code:** Fluxus is a Flink/Kafka Streams-inspired streaming library that provides a linear Source→Operator→Sink pipeline model with tumbling/sliding/session windowing, backpressure, retry, and configurable parallelism — but with no DAG topology, no persistent state backend, no credential system, and no workflow orchestration semantics.

**Comparison with Nebula:** Fluxus and Nebula occupy almost non-overlapping problem spaces. Nebula is a *workflow orchestration engine* with a TypeDAG, credential subsystem, persistent execution log, multi-tenancy, and 5 action kinds covering n8n+Temporal use cases. Fluxus is a *streaming processing library* — closer to Apache Flink's Rust analogue. The shared surface area is narrow: both use async Rust with tokio, both have a notion of a data unit (Nebula: workflow event / Fluxus: `Record<T>`), and both model retry/backpressure. The conceptual distance is larger than the tooling overlap.

---

## 2. Workspace structure [A1]

Fluxus is organized as a Cargo workspace (`Cargo.toml` line 2: `members = ["crates/*", "examples/*"]`). There are **8 library crates** plus multiple example crates:

| Crate | Role |
|-------|------|
| `fluxus` | Umbrella crate; all sub-crates are optional features; `full` feature enables all |
| `fluxus-api` | High-level fluent API (`DataStream`, `WindowedStream`); depends on all other crates |
| `fluxus-core` | Core types: `Pipeline`, `RetryStrategy`, `BackpressureController`, `Metrics` |
| `fluxus-runtime` | `RuntimeContext` — parallel execution via tokio channels; `KeyedStateBackend`; `WatermarkTracker` |
| `fluxus-sources` | `Source<T>` trait + `CsvSource`, `GeneratorSource` |
| `fluxus-sinks` | `Sink<T>` trait + `ConsoleSink`, `FileSink`, `BufferedSink`, `DummySink` |
| `fluxus-transformers` | `Operator<In, Out>` trait + `MapOperator`, `FilterOperator`, `WindowReduceOperator`, `WindowAggregator`, etc. |
| `fluxus-utils` | `Record<T>`, `StreamError`, `StreamResult<T>`, `WindowConfig`, `WindowType`, time utils |

**Layer model:** Bottom-up: `fluxus-utils` has no internal deps → `fluxus-sources`, `fluxus-sinks` depend only on utils → `fluxus-transformers` depends on sources + utils → `fluxus-core` aggregates all leaf crates → `fluxus-runtime` depends on core + leaves → `fluxus-api` depends on core + runtime + all leaves. `fluxus` (umbrella) re-exports everything via feature flags.

**Feature flags:** The umbrella crate (`crates/fluxus/Cargo.toml`) has `default = []` (empty) and `full = [all sub-crates]`. Individual sub-crates have no feature flags of their own.

**Comparison with Nebula:** Nebula has 26 crates vs. Fluxus 8. Nebula's crate boundaries encode domain concepts (credential, resource, action, engine, tenant). Fluxus's crate boundaries encode *layer* (api/core/runtime/transforms/sources/sinks/utils) — a classic library architecture without domain decomposition. Neither is strictly better; they reflect different problem decompositions.

---

## 3. Core abstractions [A3, A17] — DEEP

This section answers all A3.1–A3.9 questions with code citations.

### A3.1 — Trait shape

The core trait is `Operator<In, Out>` defined at `crates/fluxus-transformers/src/operator/mod.rs:12–35`:

```rust
#[async_trait]
pub trait Operator<In, Out>: Send {
    async fn init(&mut self) -> StreamResult<()> { Ok(()) }
    async fn process(&mut self, record: Record<In>) -> StreamResult<Vec<Record<Out>>>;
    async fn on_window_trigger(&mut self) -> StreamResult<Vec<Record<Out>>> { Ok(Vec::new()) }
    async fn close(&mut self) -> StreamResult<()> { Ok(()) }
}
```

Traits are **open** — any external crate can implement `Operator<In, Out>`, `Source<T>`, or `Sink<T>`. There are no sealed-trait mechanisms (no private supertrait trick, no module-private bound). All three traits are **object-safe** via `dyn` boxing: `crates/fluxus-transformers/src/lib.rs:14` defines `pub type InnerOperator<T, R> = dyn Operator<T, R> + Send + Sync` and `InnerSource<T> = dyn Source<T> + Send + Sync`. Associated type count: **0** — the type parameters (`In`, `Out`, `T`) are generic parameters not associated types. No GATs, no HRTBs, no typestate. The `async_trait` macro (external crate) is required for all three traits.

### A3.2 — I/O shape

The data unit is `Record<T>` (`crates/fluxus-utils/src/models.rs:7–25`): a generic struct with `pub data: T` and `pub timestamp: i64`. `T` is a *fully generic type parameter* — no serde requirement, no type erasure. `Operator::process` takes `Record<In>` and returns `StreamResult<Vec<Record<Out>>>` — meaning zero, one, or multiple output records per input. This is a clean flat-map semantics. There is no streaming output (no async generator / `Stream` impl / yielded values); output is synchronous `Vec`. Side effects model: implicit — operators can do anything in their `&mut self` state.

### A3.3 — Versioning

No versioning mechanism. Operators are plain Rust structs. No `#[deprecated]`, no version field in any manifest, no "v1 vs v2" distinction in the API. Operators are referenced entirely by Rust type at compile time. There is no workflow definition format (no YAML/JSON/TOML workflow spec), so the question of "name+version reference" in a workflow definition is N/A.

### A3.4 — Lifecycle hooks

Three lifecycle hooks: `init()`, `process()` (per-record), `on_window_trigger()` (window emit), `close()`. All are `async`. No `pre`/`post`/`on-failure` hooks. No cancellation token, no idempotency key. `Pipeline::execute` in `fluxus-core` calls `init()` on all sources, operators, and sinks before the main loop, then calls `flush()` and `close()` on the sink at completion (`crates/fluxus-core/src/pipeline/processor.rs:191–279`).

### A3.5 — Resource and credential deps

None. Operators have no mechanism to declare dependencies on DB pools, HTTP clients, or credentials. Each operator manages its own state via `&mut self`. There is no dependency injection, no resource registry, no credential binding. Compared to Nebula (which has separate `nebula-resource` and `nebula-credential` crates with 4-scope lifecycle and on_credential_refresh hooks), Fluxus has zero infrastructure here.

### A3.6 — Retry/resilience attachment

Retry is configured **per-pipeline**, not per-operator. `Pipeline::with_retry_strategy(strategy: RetryStrategy)` sets a single `ErrorHandler` (`crates/fluxus-core/src/pipeline/processor.rs:110–113`). `RetryStrategy` is an enum with `NoRetry`, `Fixed { delay, max_attempts }`, `ExponentialBackoff { initial_delay, max_delay, max_attempts, multiplier }` (`crates/fluxus-core/src/error_handling/retry_strategy.rs:5–20`). The default strategy in `Pipeline::source()` is exponential backoff with 100ms initial, 10s max, 3 attempts, 2x multiplier (`crates/fluxus-core/src/pipeline/processor.rs:71–76`). There is no per-operator override, no circuit breaker, no bulkhead, no timeout, no hedging.

### A3.7 — Authoring DX

No derive macro, no code generation. Users implement the `Operator`, `Source`, or `Sink` trait directly. Minimal "hello world" operator:

```rust
struct MyOp;

#[async_trait]
impl Operator<String, String> for MyOp {
    async fn process(&mut self, record: Record<String>) -> StreamResult<Vec<Record<String>>> {
        Ok(vec![record])
    }
}
```

Approximately 8 lines including `async_trait` and `use` statements. The fluent builder on `DataStream` means pipeline construction is concise:

```rust
DataStream::new(source).map(|x| x * 2).filter(|x| *x > 5).sink(sink).await?;
```

There is a planned `cargo fluxus-init` scaffolding tool (GitHub issue #81) that has not been implemented.

### A3.8 — Metadata

No metadata system. No display name, description, icon, category, or i18n fields. Operators are plain Rust structs with no annotation mechanism. There is no registry of available operators and no catalog/marketplace infrastructure.

### A3.9 — vs Nebula

Nebula has 5 action kinds (Process / Supply / Trigger / Event / Schedule) with a sealed trait (external crates cannot implement core kinds), associated `Input`/`Output`/`Error` types, versioning via type identity, and a derive macro (`nebula-derive`). Fluxus has **1 "kind"** — `Operator<In, Out>` — an open, generic trait with no sealing, no associated type axioms beyond the generic parameters, no versioning, and no derive macro. Nebula's design is vastly richer and more constrained; Fluxus's is simpler and more flexible at the cost of fewer compile-time guarantees. The `Operator` trait does not distinguish stateful vs. stateless operators at the type level (GitHub issue #89 requests this classification).

---

## 4. DAG / execution graph [A2, A9, A10]

**A2 — DAG model:** Fluxus has **no DAG**. The execution model is strictly linear: one source → zero or more operators → one sink. `DataStream::transform` wraps the current source + operator list into a `TransformSourceWithOperator` (`crates/fluxus-api/src/stream/datastream.rs:96–107`), producing a new `DataStream`. There is no branching, no fan-out, no fan-in, no merge, no join, no cycle detection. `petgraph` is not a dependency. There are no compile-time topology checks. This is strictly a *pipeline* model, not a DAG model.

**A10 — Concurrency:** Two concurrency models coexist:

1. **`Pipeline` model** (`fluxus-core`): single-threaded event loop using `tokio::select!` over source reads and a watermark interval tick (`crates/fluxus-core/src/pipeline/processor.rs:208–276`). Backpressure is applied by busy-waiting with `time::sleep`.

2. **`RuntimeContext` model** (`fluxus-runtime`): parallel execution via `tokio::spawn` + `tokio::sync::mpsc` channels (`crates/fluxus-runtime/src/runtime.rs:29–67`). One task per source, `parallelism` tasks per operator, one task per sink. Operators share an `Arc<Mutex<dyn Operator>>` — contended. `parking_lot::RwLock` used in `KeyedStateBackend` and `WatermarkTracker`.

**Unsafe code note:** `crates/fluxus-transformers/src/transform_base.rs:32–68` uses `unsafe { &mut *(Arc::as_ptr(&operator) as *mut ...) }` to bypass Rust's aliasing rules, with a comment claiming "exclusive access through &mut self". This is a **soundness concern**: `Arc` does not guarantee exclusive access; the comment's reasoning is incorrect in principle. This pattern also appears in `transform_source_with_operator.rs:72`.

---

## 5. Persistence & recovery [A8, A9]

**No persistence.** Fluxus is an in-memory streaming library. There is no database, no SQL, no migrations, no checkpoint/recovery mechanism. `KeyedStateBackend<K, V>` (`crates/fluxus-runtime/src/state.rs`) is a `HashMap` wrapped in `parking_lot::RwLock` — entirely in-memory with no durability. `WatermarkTracker` (`crates/fluxus-runtime/src/watermark.rs`) is an in-memory `RwLock<SystemTime>`. DeepWiki query 9 confirmed state checkpointing is *planned* but not implemented. There is no append-only log, no frontier-based scheduling, no execution replay.

**Comparison with Nebula:** Nebula has `sqlx + PgPool`, per-aggregate Pg*Repo, SQL migrations, frontier-based checkpoint recovery, and an append-only execution log. Fluxus has none of these. Different category of system — streaming library vs. durable workflow engine.

---

## 6. Credentials / secrets [A4] — Deep (Negative)

**No credential layer exists.** Grep evidence:

- `grep -rn "credential\|secret\|token\|auth\|oauth\|apikey\|api_key" --include="*.rs"` → **zero results** across all 8 crates
- `grep -rn "credential\|secret\|token\|auth\|oauth" --include="*.toml"` → only `authors` field matches, no auth/credential deps

**A4.1:** No credential layer. Not a documented design decision — simply out of scope for a streaming library.
**A4.2:** No storage. No encryption. No backend.
**A4.3:** No in-memory protection. No `zeroize`, no `secrecy::Secret<T>`.
**A4.4:** No lifecycle, no refresh, no revocation.
**A4.5:** No OAuth2/OIDC.
**A4.6:** N/A.
**A4.7:** N/A.
**A4.8:** No type-safe credential distinction.
**A4.9 vs Nebula:** Nebula has State/Material split, LiveCredential with watch(), blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter type erasure. Fluxus has **none** of these — different project category.

---

## 7. Resource management [A5] — Deep (Negative)

**No resource abstraction exists.** Grep evidence:

- `grep -rn "resource\|Resource\|pool\|Pool\|client\|Client" --include="*.rs"` → only `PipelineStatus::Running` and `Record` field names; no resource registry, no pool abstraction
- No `deadpool`, `bb8`, `sqlx::Pool`, `reqwest::Client` in any `Cargo.toml`

**A5.1:** No resource abstraction. Each operator allocates its own state via `&mut self`.
**A5.2:** No scope levels. No Global/Workflow/Execution/Action concept.
**A5.3:** Lifecycle hooks exist only on `Operator`/`Source`/`Sink` (`init`/`close`) — not on external resources.
**A5.4:** No hot-reload, no `ReloadOutcome`, no generation counter.
**A5.5:** `Arc<Mutex<dyn Operator>>` is used for sharing in `RuntimeContext` — a minimal form of sharing, not a pool.
**A5.6:** No credential deps on resources.
**A5.7:** Backpressure exists at pipeline level (`BackpressureStrategy`) but is not a resource-acquire mechanism.
**A5.8 vs Nebula:** Nebula has 4 scope levels, `ReloadOutcome`, generation tracking, `on_credential_refresh`. Fluxus has none — different project category.

---

## 8. Resilience [A6, A18]

**Retry:** `RetryStrategy` enum in `fluxus-core` with `NoRetry`, `Fixed`, `ExponentialBackoff` (`crates/fluxus-core/src/error_handling/retry_strategy.rs`). Applied per-pipeline via `ErrorHandler::retry()` (`crates/fluxus-core/src/error_handling/mod.rs:22–52`). Retry is applied to both operator processing and sink writes with `block_on()` inside an async retry loop — a problematic pattern (blocking inside async executor).

**Backpressure:** `BackpressureStrategy` enum with `Block`, `DropOldest`, `DropNewest`, `Throttle { high_watermark, low_watermark, backoff }` (`crates/fluxus-core/src/error_handling/backpressure.rs`). `Pipeline` applies throttle via `time::sleep` in the main loop.

**No circuit breaker.** No bulkhead. No timeout wrapper. No hedging. No `ErrorClassifier` equivalent.

**Errors [A18]:** Defined in `crates/fluxus-utils/src/models.rs:29–50` as `StreamError` (a `thiserror`-derived enum):
```rust
pub enum StreamError {
    Io(#[from] std::io::Error),
    Serialization(String),
    Config(String),
    Runtime(String),
    EOF,
    Wait(u64),
}
```
`StreamResult<T> = Result<T, StreamError>`. Simple flat enum — no error classification taxonomy, no transient/permanent distinction, no `ErrorClass` equivalent. `anyhow` is in dependencies of `fluxus-api` and `fluxus-core` but not used in the error type itself.

**Comparison with Nebula:** Nebula has `nebula-error` + `ErrorClass` (transient/permanent/cancelled) used by `ErrorClassifier` to route retry decisions. Fluxus has a simpler flat error enum with retry hardcoded as exponential backoff without classifying whether an error is retriable.

---

## 9. Expression / data routing [A7]

**No expression engine.** No DSL, no `$nodes.foo.result` syntax, no JSON path evaluation, no sandboxed eval. Data routing is handled entirely in Rust closures passed to `map()`, `filter()`, `flat_map()`. This is appropriate for a library (not a configuration-driven workflow engine). Operators can embed arbitrary Rust logic.

---

## 10. Plugin / extension system [A11] — TWO sub-sections

### 10.A — Plugin BUILD process

**A11.1 — Format:** No plugin format. No manifest. No `.tar.gz`, OCI image, WASM blob, or dynamic library concept. Extension is done by adding a Rust crate to the workspace or implementing the `Operator`/`Source`/`Sink` traits in user code.

**A11.2 — Toolchain:** No plugin SDK. No `cargo fluxus-init` (planned, GitHub issue #81). Users write standard Rust crates and depend on the relevant Fluxus crates.

**A11.3 — Manifest content:** N/A.

**A11.4 — Registry/discovery:** No registry. No search mechanism. The umbrella `fluxus` crate uses Cargo feature flags to enable sub-crates — this is the only "discovery" mechanism.

### 10.B — Plugin EXECUTION sandbox

**A11.5 — Sandbox type:** No sandbox. `wasm32-unknown-unknown` in `rust-toolchain.toml` is a *build target* for running Fluxus itself in a browser, not for isolating plugin code. No `wasmtime`, `wasmer`, `libloading`, or subprocess IPC.

**A11.6 — Trust boundary:** N/A — no plugin isolation concept.

**A11.7 — Host↔plugin calls:** N/A.

**A11.8 — Lifecycle:** N/A.

**A11.9 — vs Nebula:** Nebula targets WASM + capability security + Plugin Fund commercial model. Fluxus has **no plugin system** — extension is compile-time Rust trait implementation with no runtime isolation, no capability model, and no commercial monetization story.

---

## 11. Trigger / event model [A12]

Fluxus has **no trigger/event model** in the workflow-orchestration sense. There is no webhook registration, no cron/schedule abstraction, no Kafka/NATS consumer, no FS watch, no DB change stream. The `Source<T>` trait is the closest analog — it reads records from wherever it is implemented — but it is a pull-based polling abstraction, not an event-push trigger. Sources in the codebase: `CsvSource` (reads a CSV file), `GeneratorSource` (generates synthetic records from a `Vec`).

Grep evidence:
- `grep -rn "Kafka\|kafka\|rabbitmq\|nats\|redis\|pubsub\|mqtt" --include="*.rs"` → **zero results**
- `grep -rn "webhook\|cron\|schedule\|trigger" --include="*.rs"` → zero results (outside comments)

GitHub issues suggest future blockchain data sources (Solana #65, TON #66), PostgreSQL #33, Cassandra #32, ClickHouse #49, and social platform sinks (Twitter #60, Discord #59, Telegram closed #57). None are implemented yet.

**Comparison with Nebula:** Nebula has `TriggerAction` with `Input = Config` (registration) and `Output = Event` (typed payload), plus a `Source` trait that normalizes HTTP req / Kafka msg / cron tick into typed events. This is a 2-stage source-normalization model with backpressure. Fluxus has a single-stage `Source<T>` pull model with no trigger semantics.

---

## 12. Multi-tenancy [A14]

No multi-tenancy. No tenant ID concept, no RBAC, no SSO, no SCIM, no database isolation modes. Fluxus is a single-process streaming library; tenancy is not a concern.

---

## 13. Observability [A15]

Basic in-process metrics only:

- `Metrics` struct (`crates/fluxus-core/src/metrics.rs`) with `Counter` and `Timer` primitives, stored in a `HashMap<String, MetricValue>`. All in-memory.
- `Pipeline` tracks `records_processed`, `records_failed`, and `process_time` counters/timers via `Arc<Counter>` and `Arc<Timer>`.
- `tracing` crate is used for structured logging in `Pipeline::execute` (`tracing::warn`, `tracing::error`, `tracing::debug`).
- No `opentelemetry`, no Prometheus exporter, no Jaeger, no distributed trace correlation, no per-execution trace context.
- GitHub issue #77 requests a `fluxus-console` for real-time metrics visualization — not yet implemented.

**Comparison with Nebula:** Nebula has OpenTelemetry with one trace per workflow run, per-action latency/count/error metrics. Fluxus has primitive in-memory counters with `tracing` for log output — a much lighter approach appropriate for a library.

---

## 14. API surface [A16]

Fluxus is a **library API only** — there is no network API (no REST, no gRPC, no WebSocket server). The public surface is the `DataStream<T>` fluent builder (`fluxus-api`) and the `Operator`/`Source`/`Sink` traits. No OpenAPI spec, no versioning beyond semver.

---

## 15. Testing infrastructure [A19]

No dedicated testing crate. Tests are inlined as `#[cfg(test)]` modules in source files. `dev-dependencies` include `tokio-test = "0.4.4"` (in `fluxus-api`), `criterion = "0.6"` (benchmarks in `fluxus-runtime`). `cargo-husky` is a dev dependency in `fluxus-core` for pre-commit hooks. GitHub issue #52 (integration tests) was closed, suggesting some integration tests were added, but there is no `nebula-testing`-equivalent public testing utility crate.

---

## 16. AI / LLM integration [A21] — Deep (Negative)

**No AI/LLM integration exists.** Full grep evidence:

- `grep -rn "llm\|openai\|anthropic\|claude\|gpt\|embedding\|completion\|langchain\|wasm\|plugin" --include="*.rs"` → **zero results**
- `grep -rn "llm\|openai\|anthropic\|embedding\|completion" --include="*.toml"` → **zero results** across all Cargo.tomls and Cargo.lock (verified)

**A21.1 — Existence:** No built-in LLM integration, no separate crate, no community plugin infrastructure. Not a feature of fluxus.

**A21.2 — Provider abstraction:** None.

**A21.3 — Prompt management:** None.

**A21.4 — Structured output:** None.

**A21.5 — Tool calling:** None.

**A21.6 — Streaming:** Fluxus does stream data, but `Source<T>` produces `Record<T>` — not LLM token streams. No SSE adapter, no chunked LLM response model.

**A21.7 — Multi-agent:** None.

**A21.8 — RAG/vector:** None.

**A21.9 — Memory/context:** None.

**A21.10 — Cost/tokens:** None.

**A21.11 — Observability:** None.

**A21.12 — Safety:** None.

**A21.13 — vs Nebula+Surge:** Nebula has no first-class LLM abstraction either (strategic bet: AI = generic actions + plugin LLM client). Fluxus is similarly absent. However, Fluxus's `Operator<In, Out>` trait *could* wrap an LLM call as a custom operator (the open-trait design enables this), but the framework provides nothing to help. Nebula's planned plugin model (WASM + capability security) would provide a safer execution boundary for untrusted LLM-calling code than Fluxus's no-isolation model.

---

## 17. Notable design decisions

### 17.1 — Pull-based lazy evaluation via `TransformSourceWithOperator`

`DataStream::map/filter/transform` do not eagerly compute — they wrap the existing source and operator list into a new `TransformSourceWithOperator` that becomes the source for the next stage. This is a pull model: data flows backward from the terminal `.sink()` call. This enables zero-copy lazy composition but makes fan-out topologies (DAGs) architecturally impossible without a fundamental redesign.

**Trade-off:** Simple, composable, easy to reason about. Incompatible with DAG semantics (join, branch, merge). Not suitable for Nebula's use case.

### 17.2 — Two parallel execution models with incompatible semantics

`Pipeline` (fluxus-core) and `RuntimeContext` (fluxus-runtime) are separate execution models. `Pipeline` is a single-threaded select loop with retry and backpressure. `RuntimeContext` spawns parallel tokio tasks with mpsc channels. These are not unified — users must choose one. The dual-model creates confusion and API surface duplication.

**Trade-off:** `RuntimeContext` enables higher throughput; `Pipeline` is simpler to debug. The split also means retry logic in `Pipeline` does not carry over to `RuntimeContext`.

### 17.3 — `unsafe` Arc pointer casting in `TransformBase`

`crates/fluxus-transformers/src/transform_base.rs:32–34` uses `unsafe { &mut *(Arc::as_ptr(&operator) as *mut ...) }` claiming "safe because we have exclusive access through &mut self". This reasoning is incorrect: `Arc` does not enforce exclusive ownership; if another `Arc` clone exists elsewhere, this creates undefined behavior. This is a latent soundness bug.

**Trade-off:** Avoids `Arc<Mutex<T>>` overhead for the common (single-owner) case, but does so unsafely. Correct approach would be `Arc::get_mut` (which returns `None` if not uniquely owned) or restructuring to take exclusive ownership.

### 17.4 — Open traits with no sealing

All three core traits (`Operator`, `Source`, `Sink`) are fully open for external implementation. This maximizes extensibility but provides no ecosystem coherence guarantees. There is no "official operator" vs. "community operator" distinction, no version compatibility declaration, no interface stability promise beyond semver.

**Trade-off:** Easy DX for adding custom operators. No ability to add sealed variants (e.g., a new `Operator::on_checkpoint()` method breaks all existing implementations — the `#[async_trait]` default methods partially mitigate this). Nebula's sealed trait approach makes backward-compatible evolution of the core action kinds possible without breaking implementors.

### 17.5 — `async_trait` for object safety

All three core traits use `#[async_trait]` for `dyn` compatibility. On Rust 1.88.0 (the pinned version), native `async fn in trait` is stabilized but RPITIT is not fully object-safe for `dyn`. Using `async_trait` is the pragmatic choice for `dyn Operator + Send + Sync` but adds boxing overhead per async call.

---

## 18. Known limitations / pain points

From GitHub issues (fetched via `gh issue list --repo lispking/fluxus --state all --limit 30`):

- **#89** (OPEN) — "Classify operators as stateless and stateful" — the type system does not distinguish stateful from stateless operators. No reactions visible.
- **#81** (OPEN, `enhancement, help wanted`) — `cargo fluxus-init` scaffolding tool not yet built. Community wants it.
- **#77** (OPEN) — `fluxus-console` monitoring UI not yet built.
- **#68** (OPEN, `enhancement, help wanted`) — `KeyedStateBackend` still uses plain `HashMap + RwLock` rather than `DashMap`; concurrent access is a known bottleneck.
- **#46** (CLOSED, `bug`) — "Filter operator doesn't work" — was a bug, now closed. Suggests early-stage quality.
- **#84** (CLOSED) — "Replace `tokio::mpsc` with `tokio-mpmc`" — the current mpsc-based parallelism (one receiver for all parallel operator workers sharing via `Arc<Mutex<Receiver>>`) is a known bottleneck.
- **State checkpointing** — documented as planned but not implemented (DeepWiki query 9).

---

## 19. Bus factor / sustainability

- **Maintainer count:** Appears to be 1 primary author (`lispking`) with dependabot from `fluxus-labs` org. 92 total commits in depth-50 clone. Low bus factor.
- **Commit cadence:** Most recent commits are dependabot security bumps (openssl, rustls-webpki, bytes). Last feature commit `a623278` (add user-behavior examples) and `eff0787` (add runtime_benchmark) appear to be the most recent non-dependabot work.
- **Issues ratio:** 30 issues visible; mix of open enhancements and closed bugs. Healthy engagement pattern for a small project.
- **Version:** 0.2.0 — pre-1.0; API instability expected.
- **crates.io:** Published (`fluxus-core` badge in README), but download count not checked from clone.

---

## 20. Final scorecard vs Nebula

| Axis | Their approach | Nebula approach | Who's deeper / simpler / more correct | Borrow? |
|------|---------------|-----------------|---------------------------------------|---------|
| A1 Workspace | 8 crates (api/core/runtime/transforms/sources/sinks/utils/umbrella), layered by abstraction level, feature-gated umbrella | 26 crates, domain-layered (error/resilience/credential/resource/action/engine/tenant/…). Edition 2024. | Different decomposition: Fluxus = library layers, Nebula = domain boundaries. Neither dominates. | no — different goals |
| A2 DAG | No DAG. Linear pipeline only: Source → [Operators] → Sink. No petgraph, no fan-out, no join. | TypeDAG L1-L4 (static generics → TypeId → refinement predicates → petgraph) | Nebula deeper — Fluxus is not a DAG engine | no — different goals |
| A3 Action/Node | 1 open generic trait `Operator<In, Out>` with 4 lifecycle methods. No sealed trait, no associated types, no derive macro, no versioning, no metadata. | 5 action kinds (Process/Supply/Trigger/Event/Schedule), sealed trait, associated Input/Output/Error, derive macros | Nebula deeper — sealed kinds + associated types + derive macro vs. single open generic trait | refine — Fluxus's simpler open-trait is appropriate for a library; Nebula's richer model is needed for a platform |
| A11 Plugin BUILD | No plugin system. Extension = implement trait in user Rust crate. No manifest, no SDK, no registry. | WASM sandbox planned, plugin-v2 spec, Plugin Fund commercial model | Nebula richer (planned) — Fluxus has no isolation concept | no — different goals |
| A11 Plugin EXEC | No sandbox. `wasm32-unknown-unknown` is build target, not runtime sandbox. No dynamic loading. | WASM sandbox + capability security (planned, wasmtime) | Nebula richer (planned) | no — different goals |
| A18 Errors | `StreamError` enum via `thiserror`, 6 variants (Io/Serialization/Config/Runtime/EOF/Wait). No error classification. `anyhow` in deps but not used in error type. | nebula-error crate, `ErrorClass` enum (transient/permanent/cancelled), `ErrorClassifier` in resilience | Nebula deeper — transient/permanent classification enables smarter retry decisions; Fluxus flat enum with no classification | refine — `StreamError::Wait(u64)` is a clever variant for backpressure signaling; could inspire Nebula's backpressure error path |
| A21 AI/LLM | No LLM integration. Zero grep matches for openai/anthropic/llm/embedding across all code. Custom `Operator` impl could call LLM but no framework support. | No first-class LLM yet; strategic bet = generic actions + plugin LLM client | Convergent absence — both projects have no LLM layer; Fluxus's `Operator<In, Out>` is a natural extension point for LLM calls | no — different goals |
