# dag_exec — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/reymom/rust-dag-executor
- **Crate name on crates.io:** `dag_exec`
- **Current version:** 0.1.1 (published; API pre-1.0, breaking changes expected)
- **License:** MIT OR Apache-2.0
- **Rust edition:** 2024
- **Minimum supported Rust version:** 1.85
- **Stars / Forks:** small personal project, minimal stars at research date (2026-04-26)
- **Last meaningful commit:** 2026-02-26 (docs polish / README), feature work ended at `feat: add feature-gated execution tracing` (2026-02-26)
- **Tags:** `v0.1.1` (latest), `v0.1.0`
- **Maintainers:** single maintainer (reymom); solo project
- **Zero runtime dependencies.** `[dependencies]` is empty. Only dev-dependency is `criterion = "0.8.2"`.

---

## 1. Concept positioning [A1, A13, A20]

**Author's own description (README.md line 3):**
> "Sync DAG executor for CPU-heavy pipelines: bounded parallelism + partial evaluation (std-only)."

**My description after reading code:**
A minimal, zero-dependency, synchronous DAG executor library that uses Kahn's topological sort to schedule `Source`/`Task` nodes (plain closures), supports partial evaluation (prunes unneeded nodes), runs tasks in a bounded std-thread worker pool, and detects cycles at execution time via leftover positive in-degree after processing.

**Comparison with Nebula:**
dag_exec is a utility library, not a workflow orchestration engine. It occupies the bottom of the compute stack — think of it as a reusable scheduler primitive that Nebula could theoretically embed inside its engine, but dag_exec itself has no credentials, no persistence, no tenancy, no triggers, no expressions, no plugins, and no network layer. The positioning overlap is only at the abstract idea of "tasks with dependencies"; architecturally these are different problem classes.

---

## 2. Workspace structure [A1]

dag_exec is a **single-crate library**, not a workspace.

- One `Cargo.toml` at root; `name = "dag_exec"`, `version = "0.1.1"`, `edition = "2024"` (`Cargo.toml` line 1–4)
- **No workspace members, no sub-crates**
- Source layout:
  - `src/lib.rs` — public re-exports only
  - `src/graph.rs` — `Dag<K,O,E>`, `NodeId`, `NodeKind`, `ExecutorConfig`
  - `src/builder.rs` — `DagBuilder<K,O,E>`
  - `src/error.rs` — `BuildError<K>`, `ExecError<K,E>`
  - `src/exec.rs` — `Executor` struct (facade); delegates to sub-modules
  - `src/exec/common.rs` — `mark_needed`, `build_kahn_metadata`, `collect_outputs`
  - `src/exec/sequential.rs` — sequential Kahn scheduler
  - `src/exec/parallel.rs` — parallel scheduler with bounded worker pool
  - `src/exec/observe.rs` — `ExecObserver` trait + `NoopObserver`
  - `src/trace.rs` — feature-gated `ExecutionTrace`, `NodeTrace`, `TracedExecution`, `TraceObserver`
- **One feature flag:** `execution-trace` (`Cargo.toml` line 24); guards all trace machinery in `src/trace.rs`
- **19 Rust source files total, ~2,744 lines** (including tests, benches, examples)
- **Zero production dependencies** (strictly std-only)

**vs Nebula:** Nebula uses a 26-crate layered workspace (nebula-error / nebula-resilience / nebula-credential / nebula-resource / nebula-action / nebula-engine / nebula-tenant / etc.). dag_exec is a 1-crate library occupying 1/26th of Nebula's scope — roughly equivalent to a stripped-down version of what might live inside `nebula-engine`'s scheduler.

---

## 3. Core abstractions [A3, A17] — DEEP

### A3.1 Trait shape

There is **no user-facing trait** for defining a node. The "action" abstraction is a raw closure:

```rust
// src/graph.rs:29
pub(crate) type TaskFn<O, E> = dyn Fn(&[Arc<O>]) -> Result<O, E> + Send + Sync + 'static;
```

Users pass closures to `DagBuilder::add_task`. There is no `Action` trait, no sealed trait, no assoc types for Input/Output/Error on a trait implementor. The type parameter tripling `<K, O, E>` on `Dag`, `DagBuilder`, and `Executor` means:
- `K`: node key type (any `Eq + Hash + Clone`)
- `O`: the single shared output/input value type for all nodes in a given DAG
- `E`: the error type tasks can produce

**Trait count: zero user-facing traits.** The only internal trait is `ExecObserver` (`src/exec/observe.rs:4`) which is sealed in the sense that it lives in a `pub(crate)` module.

**No GATs, no HRTBs, no typestate, no sealed-trait pattern.** Type safety comes purely from the generic triple `<K, O, E>` and the `Send + Sync + 'static` closure bounds.

### A3.2 I/O shape

All nodes in a single DAG share the **same output type `O`** (`src/graph.rs:29`). Input to a task node is `&[Arc<O>]` — a slice of Arc-wrapped outputs from dependency nodes. There is no per-node typed I/O differentiation; a DAG cannot mix nodes with heterogeneous value types without newtype wrapping.

Sources inject pre-computed values: `add_source(key, value: O)` wraps with `Arc::new(value)` (`src/builder.rs:39`).

**No streaming output. No side-effects model.** Return is `Result<O, E>`.

### A3.3 Versioning

No versioning. Nodes are identified by keys of type `K` (any `Eq + Hash + Clone`). No v1/v2 concept, no `#[deprecated]`, no migration support.

### A3.4 Lifecycle hooks

Single lifecycle: the closure itself is executed. There is no pre/post/cleanup/on-failure hook. The task closure signature is `Fn(&[Arc<O>]) -> Result<O, E>` (`src/graph.rs:29`). On `Err(e)`, execution propagates `ExecError::TaskFailed` and halts the run.

**Cancellation:** none implemented. No cancellation token, no cooperative cancellation points.

**Idempotency:** not supported as a framework concept. Each run recomputes all needed nodes from scratch.

### A3.5 Resource & credential deps

**None.** Tasks are pure closures. There is no mechanism to declare "I need DB pool X" or "I need credential Y". Any external resources must be captured in the closure at DAG-build time via `Arc<...>` captures.

### A3.6 Retry / resilience attachment

**None.** There is no retry logic, circuit breaker, timeout, or per-task policy. On task failure (`Err(e)`), the executor immediately returns `ExecError::TaskFailed { task, error }` (`src/error.rs:28`). No wrapper, no backoff.

### A3.7 Authoring DX

Minimal builder API. A "hello world action" requires:
1. `DagBuilder::<K, O, E>::new()` (`src/builder.rs:33`)
2. `b.add_source(key, value)` (`src/builder.rs:39`)
3. `b.add_task(key, deps, closure)` (`src/builder.rs:43`)
4. `b.build()` (`src/builder.rs:65`)

Line count for minimal working example: ~6 lines (README.md lines 22–29).

No derive macros, no proc macros, no code generation. Authoring is manual closure-based; IDE support is whatever Rust's type inference can provide.

### A3.8 Metadata

No display name, no description, no icon, no category, no i18n. Node identity is purely the user-supplied key `K`. No compile-time vs runtime registry.

### A3.9 vs Nebula

Nebula has 5 action kinds (Process / Supply / Trigger / Event / Schedule) as sealed traits with associated `Input` / `Output` / `Error` types, derive macros, and versioning by type identity.

dag_exec has **zero action kinds**. The entire "action" model is a single `Fn(&[Arc<O>]) -> Result<O, E>` closure. There is no differentiation between process actions, supply actions, trigger actions, events, or schedules. All nodes are either static sources or synchronous computation closures. This is a fundamental difference in architectural scope: dag_exec implements just the compute graph primitive; Nebula wraps that with a full action taxonomy.

---

## 4. DAG / execution graph [A2, A9, A10]

### DAG model

dag_exec implements a **real runtime DAG** — no compile-time topology guarantees, but genuine cycle detection at execution time via Kahn's algorithm.

**Data structure:** custom adjacency representation using dense `Vec<Node<K,O,E>>` indexed by `NodeId(usize)`, plus a `HashMap<K, NodeId>` for key-to-index lookup (`src/graph.rs:43–46`). **Not petgraph.** The author wrote a lightweight, purpose-built structure.

**Cycle detection mechanism:** Kahn's topological sort with in-degree counting:
- At build time, missing dependencies are detected (`src/builder.rs:82–88`)
- At execution time, if `processed != needed_count` after the queue drains, the remaining nodes with positive in-degree form a cycle — returned as `ExecError::Cycle { remaining: Vec<K> }` (`src/exec/sequential.rs:90–98`, `src/exec/parallel.rs:272–280`)

**Partial evaluation (subgraph pruning):** the `mark_needed` function (`src/exec/common.rs:12–41`) performs a DFS backward from requested output keys, marking only reachable nodes. The scheduler only processes marked nodes.

**Port typing:** **none.** The uniform type `O` for all node outputs means there is no port-level type differentiation. Any type mismatch between logically distinct node outputs must be handled by the user via newtype enums.

**vs Nebula TypeDAG:** Nebula's TypeDAG uses four levels (L1 static generics, L2 TypeId dynamic registration, L3 refinement predicates, L4 petgraph soundness). dag_exec has one level: runtime Kahn with `<K,O,E>` uniform typing. Nebula's approach is architecturally deeper; dag_exec's is simpler and appropriate for its scope.

### Scheduler model [A10]

Two schedulers are provided:

**Sequential (`src/exec/sequential.rs`):** single-threaded Kahn BFS over a `VecDeque<NodeId>`. No async, no threads, no tokio. Pure `std`.

**Parallel (`src/exec/parallel.rs`):** bounded std-thread worker pool. Each worker has an independent `mpsc::sync_channel<Task<O,E>>` with configurable capacity `worker_queue_cap`. The scheduler thread dispatches tasks round-robin via `try_send`; if all queues are full (`DispatchOutcome::AllFull`), it pauses dispatch and blocks on `res_rx.recv()` to drain at least one result. Global in-flight cap is `min(max_in_flight, n_workers * (worker_queue_cap + 1))` (`src/exec/parallel.rs:194–195`).

**RAII shutdown:** `WorkerPool` implements `Drop` by taking the `Option<Vec<...>>` of senders (dropping them closes the channels) then joining all threads (`src/exec/parallel.rs:87–97`).

**No async runtime.** `#![forbid(unsafe_code)]` (`src/lib.rs:1`). The entire library is `std`-only.

**vs Nebula A10:** Nebula uses tokio with a frontier scheduler supporting work-stealing and `!Send` action isolation. dag_exec uses std threads with bounded sync channels. dag_exec is intentionally synchronous to target CPU-heavy workloads without async overhead.

---

## 5. Persistence & recovery [A8, A9]

**No persistence layer.** dag_exec holds no state between runs. There is no database, no storage backend, no append-only log, no checkpoint, no frontier-based recovery.

Each call to `exec.run_sequential` or `exec.run_parallel` is a stateless, one-shot computation. The `Dag<K,O,E>` struct is immutable after `build()` and can be reused across multiple runs, but results are not stored.

**vs Nebula A8/A9:** Nebula uses `sqlx + PgPool`, `Pg*Repo` aggregates, SQL migrations, and a frontier-based scheduler with checkpoint recovery and append-only execution log. dag_exec has none of this; it is purely in-memory and stateless.

---

## 6. Credentials / secrets [A4] — DEEP

**No credential layer exists.**

Grep evidence:
- Searched: `credential` in all `*.rs`, `*.toml`, `*.md` files — **zero matches** (verified by `grep -r "credential" . --include="*.rs" --include="*.toml" --include="*.md"` returning empty output)
- Searched: `secret` — **zero matches**
- Searched: `token`, `auth`, `oauth`, `jwt` (case-insensitive) — **zero matches**
- Searched: `encrypt`, `decrypt`, `zeroize`, `secrecy` — **zero matches**

**A4.1–A4.9:** Not applicable. No separate credential layer, no env-var convention for secrets, no documented design decision about credentials — credentials are entirely out of scope for this library.

---

## 7. Resource management [A5] — DEEP

**No resource abstraction exists.**

Tasks capture external resources (DB pools, HTTP clients, etc.) as `Arc<...>` captures at DAG-build time inside closures. There is no Resource trait, no scope levels, no lifecycle hooks, no reload mechanism.

Grep evidence:
- Searched: `Resource`, `resource`, `pool`, `lifecycle`, `ReloadOutcome`, `generation` in `*.rs` — no framework-level matches
- The word "pool" appears only in the context of the worker thread pool (`WorkerPool` struct, `src/exec/parallel.rs:31`)

**A5.1–A5.8:** Not applicable.

---

## 8. Resilience [A6, A18]

**No resilience primitives.** No retry, no circuit breaker, no bulkhead, no timeout, no hedging, no `ErrorClassifier`.

On task failure, `ExecError::TaskFailed { task: K, error: E }` is returned immediately (`src/error.rs:32`). The executor does not attempt to continue running other branches (fail-fast semantics); the entire run is aborted on first task error.

The `ExecError::InternalInvariant(&'static str)` variant (`src/error.rs:33`) covers detected invariant violations (e.g., dep missing during exec, channel disconnect). This is a safety net for internal bugs, not a user-facing resilience boundary.

---

## 9. Expression / data routing [A7]

**No expression engine.** There is no DSL, no expression evaluator, no `$nodes.foo.result.email`-style syntax, no type inference for data routing. Data flows by explicit dependency declaration: `add_task(key, deps: Vec<K>, closure)`. Routing is hardcoded at build time.

---

## 10. Plugin / extension system [A11] — DEEP

### 10.A — Plugin BUILD process

**No plugin system.**

Grep evidence:
- Searched: `plugin`, `wasm`, `wasmtime`, `wasmer`, `wasmi`, `dynamic`, `manifest` (case-insensitive) in all `*.rs`, `*.toml`, `*.md` — **zero matches**
- There is no plugin manifest format, no build SDK, no registry, no toolchain integration for external extensions.

**A11.1–A11.4:** Not applicable.

### 10.B — Plugin EXECUTION sandbox

**No execution sandbox.**

There is no WASM runtime, no subprocess IPC, no dynamic library loading (`libloading` not in deps), no capability security model.

**A11.5–A11.9:** Not applicable.

**vs Nebula A11:** Nebula targets WASM sandbox (wasmtime) with capability-based security and a Plugin Fund commercial model. dag_exec has no extension mechanism whatsoever — the "extension" model is simply writing a Rust closure and capturing what you need.

---

## 11. Trigger / event model [A12]

**No trigger or event model.**

dag_exec has no concept of workflow scheduling, event-driven execution, webhooks, cron, external message queues, or polling. Execution is initiated by a direct function call: `executor.run_sequential(dag, outputs)` or `executor.run_parallel(dag, outputs)`.

Grep evidence:
- Searched: `trigger`, `webhook`, `cron`, `schedule`, `event`, `kafka`, `rabbitmq`, `nats`, `pubsub` (case-insensitive) — **zero matches**

---

## 12. Multi-tenancy [A14]

None. No concept of users, tenants, RBAC, SSO, or SCIM. The library is purely a computation primitive.

---

## 13. Observability [A15]

**Optional execution tracing via feature flag.** Feature `execution-trace` (feature flag in `Cargo.toml` line 24) gates a lightweight instrumentation subsystem:

- `ExecObserver` trait (`src/exec/observe.rs:4`): three hooks — `mark_ready(id, frontier_width)`, `mark_start(id, worker_id)`, `mark_finish(id)` — all with default no-op impls
- `NoopObserver` struct (`src/exec/observe.rs:13`): zero-cost default
- `TraceObserver` (`src/trace.rs:53`): records `Instant`-based timestamps per node; computes critical-path time via DP over `longest_path` array (`src/trace.rs:103–109`); tracks `max_frontier_width`
- `ExecutionTrace<K>` (`src/trace.rs:36`): stores `wall_time`, `critical_path_time`, `max_frontier_width`, `executed_nodes`, per-node `NodeTrace` with `ready_at`, `started_at`, `finished_at`, `run_duration`, `worker_id`

**No OpenTelemetry, no metrics export, no structured logging.** The tracing is internal diagnostic data; there is no integration with any telemetry framework.

**vs Nebula A15:** Nebula uses OpenTelemetry with structured tracing per execution and per-action metrics. dag_exec has a bespoke, self-contained trace struct for benchmarking/diagnostics — useful for performance analysis, not production telemetry.

---

## 14. API surface [A16]

The crate's entire public API is:

```rust
// src/lib.rs
pub use builder::DagBuilder;
pub use error::{BuildError, ExecError};
pub use exec::Executor;
pub use graph::{Dag, ExecutorConfig, NodeId};
// feature = "execution-trace":
pub use trace::{ExecutionTrace, NodeTrace, TraceNodeKind, TracedExecution};
```

**No network API.** No REST, no gRPC, no GraphQL, no HTTP server. This is a library crate, not a server.

---

## 15. Testing infrastructure [A19]

Integration tests in `tests/`:
- `tests/sequential.rs` — correctness, pruning, cycle detection, failure propagation for sequential executor
- `tests/parallel.rs` — same plus backpressure invariant, RAII shutdown, concurrent correctness
- `tests/trace.rs` — feature-gated traced execution invariants (output match, critical path, wall_time, frontier width)
- `tests/common/mod.rs` — shared fixtures: `build_add_chain_dag`, `build_cycle_dag`, `build_failing_dag`

No dedicated testing crate (contrast Nebula's `nebula-testing`). No mock framework, no snapshot testing (insta), no contract tests.

---

## 16. AI / LLM integration [A21] — DEEP

**No AI or LLM integration of any kind.**

Grep evidence (all case-insensitive, searched across all `*.rs`, `*.toml`, `*.md` files):
- `openai` — **zero matches**
- `anthropic` — **zero matches**
- `llm` — **zero matches**
- `embedding` — **zero matches**
- `completion` — **zero matches**
- `gpt`, `claude`, `gemini`, `mistral` — **zero matches**
- `rag`, `vector`, `prompt` — **zero matches**

**A21.1–A21.13:** Not applicable. dag_exec is a pure compute-graph scheduler with no LLM awareness, no provider abstraction, no prompt management, no streaming, no multi-agent patterns, no RAG, no memory/context management, no token counting, no content filtering.

**vs Nebula A21:** Nebula's strategic bet is that AI workflows are realized through generic actions + a plugin LLM client. dag_exec is even further removed — it does not model LLM integration even as a future concern.

---

## 17. Notable design decisions

### 17.1 Zero dependencies

The `[dependencies]` table in `Cargo.toml` is empty. Only `criterion` appears in `[dev-dependencies]`. This is a deliberate positioning choice (README.md: "std-only") that maximizes portability (no feature flag version conflicts, no supply chain exposure) but also means users must bring all external integrations themselves. Every competitor that uses `petgraph`, `tokio`, or `rayon` accepts a transitive dependency on those crates' ecosystems. dag_exec does not.

**Trade-off:** Zero compilation time for deps; extremely light binary impact. The cost is that users cannot share scheduling primitives with async runtimes they already have.

**Applicability to Nebula:** Nebula's engine crate should not use dag_exec as an internal scheduler (async mismatch), but the zero-dep discipline of the library-facing API surface is worth noting.

### 17.2 Uniform output type `O`

The design choice `TaskFn<O, E> = dyn Fn(&[Arc<O>]) -> Result<O, E>` means every node in a DAG must produce the same `O` type. This enforces a constraint that limits expressiveness — real pipelines often mix `Vec<u8>`, `u64`, and `String` results. Users must use an enum or `Box<dyn Any>` wrapper.

**Trade-off:** Simplicity and zero unsafe code; no type-erased dispatch needed. The parallel scheduler can hold a single `Vec<Option<Arc<O>>>` indexed by `NodeId`. Cost: real heterogeneous DAGs require user-side ceremony.

**vs Nebula:** Nebula's `ProcessAction` with associated `Input`/`Output` types allows fully heterogeneous pipelines. dag_exec's uniform `O` is a significant limitation for workflow-style use cases.

### 17.3 Kahn's algorithm without petgraph

dag_exec builds its own adjacency list (`Vec<Node<K,O,E>>` + `HashMap<K,NodeId>`) instead of depending on petgraph. Cycle detection is implicitly by-product of Kahn's BFS (`processed != needed_count` check). This is correct, complete, and simpler than using petgraph's `is_cyclic_directed()`.

**Trade-off vs dagx:** dagx (Tier 2 already analyzed) prevents cycles at compile time via typestate. dag_exec detects cycles at runtime — less safe for static workflows, but handles dynamic graph construction.

### 17.4 Observer pattern for optional telemetry

The `ExecObserver` trait with default no-op methods and `NoopObserver` struct ensures zero overhead for the common case while enabling `TraceObserver` to collect timing data (`src/exec/observe.rs`). The feature flag `execution-trace` further gates the `TraceObserver` at compile time.

**Trade-off:** Correct and ergonomic. The default path has no overhead; the trace path incurs a `Vec<Option<Duration>>` per-node plus an `Instant` comparison at each scheduler event. This is the right pattern.

**Applicability to Nebula:** Nebula already has OpenTelemetry spans per action, which is a superset of this. The `ExecObserver` pattern could be a useful lightweight alternative inside benchmarks or testing where full OTel is not needed.

### 17.5 Backpressure via bounded sync_channel

The parallel scheduler uses `std::sync::mpsc::sync_channel` with configurable `worker_queue_cap` per worker, plus a global `max_in_flight` soft cap. Backpressure is enforced by detecting `TrySendError::Full` and parking the pending task until a result arrives (`src/exec/parallel.rs:212–224`).

**Trade-off:** Avoids unbounded task queuing (which could OOM on large DAGs with many ready nodes). The two-level cap (`max_in_flight` global + `worker_queue_cap * n_workers` physical) is clearly documented and tested (`tests/parallel.rs`: `backpressure_invariant`). The cost is that a mis-configured `max_in_flight = 0` panics (caught as `InternalInvariant` error, `src/exec/parallel.rs:163–165`).

### 17.6 Partial evaluation as first-class feature

`mark_needed` (`src/exec/common.rs:12–41`) is a DFS backward from requested output keys. This makes subgraph pruning a core invariant, not a feature. The examples (`pipeline.rs`) demonstrate that requesting only `commitment` skips all fee/risk/receipt branches entirely.

**vs Nebula:** Nebula does not expose partial evaluation as a user-facing primitive (workflow definitions run all scheduled nodes). For CPU-bound batch pipelines where only a subset of outputs are needed, dag_exec's model is more efficient. This is a genuine design idea worth noting, though it maps more naturally to reactive/demand-driven systems than orchestration engines.

---

## 17.B vs dagx subsection

**dagx** (Tier 2): implements cycle prevention at **compile time** via typestate pattern — edges are only addable between type-checked port types, so a malformed DAG is a type error. The runtime never needs to do cycle detection.

**dag_exec**: implements cycle detection at **runtime** via Kahn's BFS. A cyclic DAG passes `build()` successfully (the builder only checks for missing dependencies, not cycles — `src/builder.rs:65–107`), and the cycle is reported as `ExecError::Cycle` only when `run_sequential` or `run_parallel` is called.

**Key contrast:** dagx's compile-time safety is stronger but requires a more complex API (typestate builder pattern, type-parameterized graph). dag_exec's runtime approach allows dynamic DAG construction (any `K: Eq + Hash + Clone`) and is simpler to use but pushes cycle errors to runtime. For workflows that are statically known (e.g., a fixed processing pipeline), dagx's typestate model is more correct. For dynamically assembled compute graphs (e.g., constructed from config at startup), dag_exec's runtime approach is more flexible.

**orka** (Tier 1): is a sequential pipeline with typed stages, not a real DAG — fork/join is not supported. dag_exec supports arbitrary DAG topology with fan-out and fan-in, making it more general than orka's sequential chain model.

---

## 18. Known limitations / pain points

Only 6 issues total (all GitHub issues for the repo):

- **Issue #6 (OPEN):** "Error hardening & invariants cleanup" — `enhancement` label, 2026-02-17. Directly cited in README "Next" section. Covers: better error messages, invariant documentation, `max_in_flight=0` validation improvements.
  URL: https://github.com/reymom/rust-dag-executor/issues/6

- **Issue #1 (CLOSED):** "parallel: enforce max_in_flight via sync_channel + try_send" — `enhancement` label, closed 2026-02-20. This was the backpressure design issue. The solution landed via bounded sync_channel approach.
  URL: https://github.com/reymom/rust-dag-executor/issues/1

Reactions: No reaction counts visible via `gh issue list` (small project, minimal community engagement). The project has 6 issues total — well below the Tier 1/2 threshold of 100 closed issues, so the "cite ≥ 3 issues" rule does not apply per the Tier 3 spec.

**Structural limitations identified by code analysis:**
1. **Uniform output type `O`** prevents heterogeneous pipelines without user-side newtype wrapping.
2. **Cycle detection is runtime-only** — a cyclic `Dag` is valid until `run_*` is called.
3. **No async support** — the library cannot be used with tokio-based tasks without a `block_in_place` or `spawn_blocking` wrapper on the caller side.
4. **No persistence** — no way to resume a partially-executed DAG after process crash.
5. **No cancellation** — a running parallel execution cannot be aborted mid-flight.
6. **No error recovery** — first task failure aborts the entire run; no "continue other branches" mode.
7. **Pre-1.0 API** — README explicitly warns "API may change while pre-1.0."

---

## 19. Bus factor / sustainability

- **Single maintainer** (reymom). Bus factor = 1.
- **Commit cadence:** all substantive work was done in a 2-week burst (2026-02-17 to 2026-02-26). No commits after that date in the 50-commit clone.
- **Open issues:** 1 open (`#6`). No pull requests visible.
- **Published on crates.io** as v0.1.1, but no download stats gathered (tool not invoked for this tier).
- **Zero community engagement** at research date — no stars visible, no discussion threads, no third-party contributions.
- **Verdict:** Low sustainability risk for users because it is a small, self-contained library — the code is understandable enough to fork or maintain. But the project shows no evidence of active development or community adoption.

---

## 20. Final scorecard vs Nebula

| Axis | dag_exec approach | Nebula approach | Who's deeper / simpler / more correct | Borrow? |
|------|-------------------|-----------------|---------------------------------------|---------|
| A1 Workspace | Single crate, 0 deps, 19 `.rs` files, `std`-only | 26 crates layered: nebula-error / nebula-resilience / nebula-credential / nebula-resource / nebula-action / nebula-engine / nebula-tenant / etc. Edition 2024 | Nebula deeper — dag_exec is a compute primitive, not an orchestration engine | no — different goals |
| A2 DAG | Runtime Kahn BFS over custom `Vec<Node>` + `HashMap<K,NodeId>`; cycle detection by `processed!=needed_count`; partial eval via backward DFS | TypeDAG: L1 static generics, L2 TypeId, L3 predicates, L4 petgraph | Different decomposition: dag_exec runtime dynamic; Nebula compile+runtime multi-layer | refine — partial evaluation concept worth noting |
| A3 Action | No trait. Single closure type `Fn(&[Arc<O>]) -> Result<O, E>`. Uniform `O` for all nodes. No lifecycle, no versioning, no sealed trait. | 5 action kinds (Process/Supply/Trigger/Event/Schedule). Sealed trait. Assoc `Input`/`Output`/`Error`. Derive macros. | Nebula deeper — dag_exec closure model is minimal and inflexible for orchestration | no — Nebula's already better |
| A11 Plugin BUILD | None. No manifest, no SDK, no registry. | WASM sandbox planned; plugin-v2 spec; Plugin Fund commercial model | Nebula deeper — dag_exec has no plugin concept | no — different goals |
| A11 Plugin EXEC | None. No sandbox, no runtime isolation, no capability security. | WASM sandbox (wasmtime target) + capability-based security | Nebula deeper | no — different goals |
| A18 Errors | Two enums: `BuildError<K>` (4 variants: DuplicateKey, MissingKey, MissingDependency, EmptyGraph) and `ExecError<K,E>` (5 variants: Build, Cycle, TaskFailed, OutputMissing, InternalInvariant). Manual `Display` + `std::error::Error`. Zero dependencies (no `thiserror`, no `anyhow`). | `nebula-error` crate; contextual errors; `ErrorClass` enum (transient/permanent/cancelled/etc.); used by `ErrorClassifier` in resilience | Competitor simpler, Nebula richer — dag_exec's error design is appropriate for its scope; Nebula's ErrorClass enables smart resilience policies | no — Nebula's already better for orchestration; dag_exec's zero-dep approach correct for a library |
| A21 AI/LLM | None. Searched: openai, anthropic, llm, gpt, embedding, completion, prompt — zero matches. | No first-class LLM abstraction; strategic bet: AI = generic actions + plugin LLM client; Surge (separate) handles agent orchestration on ACP | Convergent at "not yet" — both have no LLM layer; Nebula has strategic intent, dag_exec has none | no — different goals |

---

## Appendix: DeepWiki query log

| Query # | Question | Result |
|---------|----------|--------|
| 1 | "What is the core trait hierarchy for actions/nodes/activities?" | FAIL — "Repository not found. Visit https://deepwiki.com to index it." |
| 4 | "How are plugins or extensions implemented (WASM/dynamic/static)?" | FAIL — "Repository not found." |
| 7 | "Is there built-in LLM or AI agent integration?" | FAIL — "Repository not found." |
| 9 | (not sent) | 3-fail-stop triggered per protocol |

The `reymom/rust-dag-executor` repository is not indexed by DeepWiki. All 4 assigned queries (1, 4, 7, 9) encountered the not-found error; the 3-fail-stop rule was triggered after query 7.
