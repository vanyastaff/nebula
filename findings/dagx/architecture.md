# dagx — Architectural Decomposition

## 0. Project metadata

| Field | Value |
|-------|-------|
| Repo | https://github.com/swaits/dagx |
| Version | 0.3.1 (latest release 2025-10-10) |
| Stars | 19 |
| Forks | 1 |
| Open issues | 0 |
| License | MIT |
| Maintainer | Stephen Waits (solo, `steve@waits.net`) |
| Created | 2025-10-07 |
| Last activity | 2026-04-23 (repository push) |
| MSRV | Rust 1.81.0 |
| Edition | 2021 |

---

## 1. Concept positioning [A1, A13, A20]

**Author's description (README.md line 9):** "A minimal, type-safe, runtime-agnostic async DAG (Directed Acyclic Graph) executor with compile-time cycle prevention and true parallel execution."

**Analyst's description:** dagx is a pure execution-time DAG task scheduler — a library primitive for running typed async tasks in dependency order with zero persistence, zero credential handling, zero plugin system, and no workflow orchestration semantics. It occupies the bottom-most layer of the workflow stack: "I have N async computations with typed I/O edges; run them in topological order."

**Comparison with Nebula:** Nebula is a full workflow orchestration engine (n8n + Temporal + Airflow merged). dagx has no overlap with Nebula's credential layer, resource lifecycle, trigger model, multi-tenancy, persistence, observability stack, or plugin system. The only overlapping concern is the DAG execution primitive — and even there dagx is pure in-process while Nebula's execution is backed by persistent state.

---

## 2. Workspace structure [A1]

dagx is a 3-crate workspace (vs Nebula's 26):

| Crate | Purpose | LOC |
|-------|---------|-----|
| `dagx` | Core library: trait definitions, scheduler, builder, error types | ~1641 |
| `dagx-macros` | Proc-macro: `#[task]` attribute that generates `Task` impls | 307 |
| `dagx-test` | Internal test helper: `task_fn` closure factory, `ExtractInput` trait | 210 |

No layer separation; all crates are at the same conceptual level. `dagx-test` is not published (`#![cfg(not(tarpaulin_include))]`). No umbrella re-export crate. No feature flags for deployment modes.

Feature flags in `dagx`:
- `derive` (default on): enables `#[task]` macro via `dagx-macros`
- `tracing` (default off): structured tracing events via `tracing 0.1`

**vs Nebula:** Nebula's 26-crate layered architecture (nebula-error → nebula-resilience → nebula-credential → nebula-resource → nebula-action → nebula-engine → nebula-tenant) reflects the full production service concern. dagx has no such layering — it is a single library with a helper macro crate.

---

## 3. Core abstractions [A3, A17] — DEEP

### A3.1 — Trait shape

The single core trait is `Task<Input>` defined at `src/task.rs:33`:

```rust
pub trait Task<Input>: Send
where
    Input: Send + Sync + 'static,
{
    type Output: Send + Sync + 'static;

    fn run(self, input: TaskInput<Input>) -> impl Future<Output = Self::Output> + Send;
}
```

This is an **open trait** — any type in any crate can implement it. There is no sealing mechanism. It is **not** `dyn`-compatible (uses `impl Future` in return position and takes `self` by value). Internally the library uses type erasure via `ExecutableNode` (a `dyn`-safe object-safe internal trait at `src/node.rs:30`) to store heterogeneous tasks.

Associated type count: **1** — `Output` only. The input type is a generic parameter `Input`, not an associated type. This is a deliberate choice: `Task<(i32, String)>` and `Task<(bool,)>` are distinct trait impls on the same struct, making tasks polymorphic over input arities without associated type proliferation.

No GATs. No HRTBs. No typestate on the trait itself (typestate is used on `TaskBuilder`, not `Task`). No default methods.

The `Input` type parameter encodes the full input tuple: `Task<()>` = no deps; `Task<(A,)>` = single dep of type A; `Task<(A,B)>` = two deps, etc., up to 8 (`src/task.rs:81-88`).

### A3.2 — I/O shape

**Input:** `TaskInput<'inputs, Input>` is a linear type wrapping an iterator over `Arc<dyn Any + Send + Sync>` (`src/task.rs:50-62`). The macro generates inline type-specific extraction via `input.next()` which downcasts `Arc<dyn Any>` to the concrete `&T`. Types are NOT required to be serializable — only `Send + Sync + 'static`. No `serde_json::Value` or `Box<dyn Any>` in the public-facing input API.

**Output:** Typed `Self::Output: Send + Sync + 'static`. Not serializable by default. The framework internally wraps output in `Arc<T>` (`src/node.rs:91`) for fan-out sharing, but the user writes `type Output = Vec<String>` — transparent.

**Streaming:** No streaming output. Tasks produce a single value.

**Side effects:** Entirely user-managed. The framework imposes no constraint on what a task does (I/O, network, DB). No side-effect model.

### A3.3 — Versioning

None. Tasks are referenced entirely by Rust type identity (the struct name). No name+version addressing, no `#[deprecated]`, no v1/v2 distinguishable in the DAG definition. The "workflow definition" is Rust source code — versioning is source-level.

### A3.4 — Lifecycle hooks

One lifecycle point: `run(self, input)`. Tasks are consumed (moved) during execution — no separate pre/post/cleanup/on-failure hooks. Panics are caught via `FutureExt::catch_unwind()` at `src/runner.rs:329` and converted to `DagError::TaskPanicked`. No cancellation points. No idempotency key.

### A3.5 — Resource and credential dependencies

No declaration mechanism. Tasks have no way to declare external dependencies (DB pool, credential, HTTP client). All external resources must be baked into the task struct at construction time (struct fields) or obtained via global/ambient context. There is no compile-time check for resource/credential availability.

Example pattern (manual, no framework support):
```rust
struct FetchUser { db: Arc<DbPool>, cred: ApiKey }
#[task]
impl FetchUser {
    async fn run(&self, user_id: &UserId) -> User { /* use self.db, self.cred */ }
}
```

### A3.6 — Retry/resilience attachment

No framework-level retry or resilience policy. The `circuit_breaker.rs` example at `examples/circuit_breaker.rs` implements a circuit breaker entirely in user space via a shared `Arc<Mutex<CircuitState>>` inside task structs. There is no per-task policy, no attribute, no runtime config. No retry loop, no backoff, no bulkhead.

### A3.7 — Authoring DX

"Hello world" action with the macro:
```rust
struct Add;
#[task]
impl Add {
    async fn run(a: &i32, b: &i32) -> i32 { a + b }
}
```
5 lines. No trait imports needed when using the macro (macro derives everything). Manual impl requires `use dagx::Task;` and explicit `Input` generic parameter. The macro is enabled by default via the `derive` feature. IDE support: standard Rust autocompletion on the generated impl.

### A3.8 — Metadata

None. No display name, description, icon, or category. No i18n. No runtime type registry. Tasks are anonymous Rust types.

### A3.9 — vs Nebula

Nebula has **5 action kinds** (Process / Supply / Trigger / Event / Schedule) — a sealed taxonomy with distinct semantics for each kind. dagx has **1 task kind** — a generic async computation unit with no semantic differentiation. Nebula uses associated types for `Input`, `Output`, `Error`, and `Config`; dagx uses one associated type (`Output`) and a generic parameter for input arity. Nebula seals its trait (external crates cannot implement new action kinds); dagx's `Task` is fully open. Nebula uses derive macros for boilerplate reduction; dagx uses the `#[task]` attribute macro with a cleaner ergonomic (no explicit `impl ActionTrait for ...` needed).

**Conclusion on A3:** dagx's single-trait design is radically simpler, intentionally so. It is correct for "run typed computations in order" but has no semantics for triggers, credentials, scheduling, or stateful workflows.

---

## 4. DAG / execution graph [A2, A9, A10]

### Graph representation

dagx does NOT use petgraph or any external graph library. The graph is two plain `HashMap<NodeId, Vec<NodeId>>` adjacency lists (`src/runner.rs:107-109`):

```rust
pub(crate) nodes: Vec<Option<Box<dyn ExecutableNode + Sync>>>,
pub(crate) edges: PassThroughHashMap<NodeId, Vec<NodeId>>,        // node → dependencies
pub(crate) dependents: PassThroughHashMap<NodeId, Vec<NodeId>>,   // node → tasks depending on it
```

`NodeId` is `u32` (`src/builder.rs:170`). The `PassThroughHasher` (`src/runner.rs:26-52`) is a custom identity hasher for `NodeId` — avoids hashing overhead since IDs are already unique integers.

### Compile-time cycle prevention

The core DAG correctness mechanism is a **typestate pattern** on `TaskBuilder`/`TaskHandle`:

- `TaskBuilder<'a, Input, Tk>` (`src/builder.rs:61`) — mutable, holds a `&'a mut DagRunner` borrow. Has `depends_on()` method. Is consumed (moved) by `depends_on()`.
- `TaskHandle<T>` (`src/builder.rs:199`) — immutable token. Has `id` and `PhantomData<fn() -> T>`. Has **no** `depends_on()` method.

The `TaskWire` trait (`src/builder.rs:209`) dispatches between the two return types at compile time: tasks with `Input = ()` return `TaskHandle` directly; tasks with non-unit `Input` return `TaskBuilder`. This enforces that you cannot wire dependencies onto a task that has already been finalized, making cycles structurally impossible to express.

Proved via `compile_fail` tests (`tests/cycle_prevention.rs`). Runtime cycle detection code was **removed** in v0.3.0 (24 lines deleted from `runner.rs` per CHANGELOG.md).

### Compile-time vs runtime graph validation

- **Compile-time:** port types and cycle prevention — complete.
- **Runtime:** topological sort via Kahn's algorithm (`src/runner.rs:450-520`) — required only for scheduling, not for validation. The sort cannot fail by design (cycles impossible).

### Type-safe ports

Output type is carried in `TaskHandle<T>`: `TaskHandle<i32>` can only be passed to a task whose `run()` takes `&i32`. The `DepsTuple` trait (`src/deps.rs:8`) enforces type correspondence: `DepsTuple<(A, B)>` only accepts `(&TaskHandle<A>, &TaskHandle<B>)`. Type mismatch is a compile error.

### Scheduling model

Kahn's algorithm layers the graph topologically (`src/runner.rs:450`). Each layer executes as:
- **Layer size = 1:** inline execution (no spawn) — `AssertUnwindSafe(node.execute_with_deps(...)).catch_unwind().await` (`src/runner.rs:328`).
- **Layer size > 1:** all tasks in the layer are submitted to the user's spawner function via `FuturesUnordered` (`src/runner.rs:367-421`), enabling true parallelism.

No work-stealing. No frontier scheduler. The scheduler is a simple layer iteration — O(nodes + edges) setup, O(max_layer_width) concurrency per layer.

### Concurrency

Runtime-agnostic: `run(spawner)` accepts any `Fn(BoxFuture<'static, ...>) -> F`. Tested runtimes: tokio, smol, async-executor, pollster, futures-executor (`tests/runtimes/mod.rs`). No `!Send` support — all tasks must implement `Send`. Internally uses `Arc<dyn Any + Send + Sync>` for type-erased output sharing.

**vs Nebula:** Nebula uses TypeDAG with L1-L4 (generics → TypeId → refinement predicates → petgraph soundness checks). dagx's DAG is L1 only — compile-time generic port types with no TypeId/predicate/petgraph layers. Nebula has a frontier-based scheduler with work-stealing; dagx has a flat layer iteration. Nebula supports `!Send` isolation via thread-local sandboxing; dagx requires `Send` everywhere.

---

## 5. Persistence and recovery [A8, A9]

**No persistence mechanism exists.**

Grep evidence:
- Searched: `grep -rn "persistence\|checkpoint\|storage\|database\|state.*save\|serializ" --include="*.rs"` — found only: `src/node.rs:41` ("node storage" in a comment about in-memory struct layout) and `src/node.rs:93` ("Return Arc-wrapped output (for storage by runner)"). No external storage, no DB, no checkpoint.
- Searched: `grep -rn "serde\|bincode\|ron\|json\|toml\|yaml" --include="*.rs"` — found nothing in source (only benchmark comparison code referencing dagrs which uses YAML).

`DagRunner::run()` consumes the `DagRunner` and returns `DagOutput` — all state lives in memory for the duration of one `run()` call. There is no re-run, no partial recovery, no crash-resume.

**vs Nebula:** Nebula has frontier-based scheduler with checkpoint recovery, append-only execution log, and state reconstruction via replay. dagx has none of this — it is a one-shot in-process executor.

---

## 6. Credentials / secrets [A4] — DEEP

**No credential layer exists.**

### A4.1 — Existence

There is no credential or secret management system in dagx. Tasks that need credentials carry them in struct fields (plain values, user's choice of type).

Grep evidence:
- `grep -r "credential\|secret\|token\|auth\|oauth\|password" --include="*.rs" -l` → returns only `src/builder.rs`, where the match is `token` in doc comment: "Opaque, typed **token** for a node's output" (referring to `TaskHandle`). Zero credential-domain hits.

### A4.2 — Storage
Not applicable. No at-rest encryption, no vault integration.

### A4.3 — In-memory protection
Not applicable. No `Zeroize`, no `secrecy::Secret<T>`.

### A4.4 — Lifecycle
Not applicable.

### A4.5 — OAuth2/OIDC
Not applicable.

### A4.6 — Composition
Not applicable.

### A4.7 — Scope
Not applicable.

### A4.8 — Type safety
Not applicable.

### A4.9 — vs Nebula
Nebula has State/Material split, LiveCredential with `watch()`, blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter erasure. dagx has none of these. The delta is complete — no overlap.

---

## 7. Resource management [A5] — DEEP

**No resource abstraction exists.**

### A5.1 — Existence

There is no resource lifecycle system. DB pools, HTTP clients, caches, or other shared resources are user-managed and must be injected via task struct fields at construction time.

Grep evidence:
- `grep -r "resource\|pool\|lifecycle\|reload\|scope" --include="*.rs" -l` → returns `examples/circuit_breaker.rs` (comment: "Preventing resource exhaustion from slow/down services") and `benches/patterns/breakdown.rs` (comment: "Full lifecycle: add 10k tasks + run them"). Neither refers to a resource management layer.

### A5.2 — Scoping
Not applicable. No scope levels.

### A5.3 — Lifecycle hooks
Not applicable.

### A5.4 — Reload
Not applicable.

### A5.5 — Sharing
User-space `Arc<T>` in task struct fields. No pooling.

### A5.6 — Credential deps
Not applicable.

### A5.7 — Backpressure
Not applicable.

### A5.8 — vs Nebula
Nebula has 4 scope levels (Global/Workflow/Execution/Action), `ReloadOutcome` enum, generation tracking, and `on_credential_refresh`. dagx has none of this. Complete delta.

---

## 8. Resilience [A6, A18]

**No framework-level resilience.**

The `circuit_breaker.rs` example (`examples/circuit_breaker.rs:168-466`) implements a three-state circuit breaker (Closed/Open/Half-Open) as a demo **of what users can build**, not a framework feature. The implementation uses a shared `Arc<Mutex<CircuitState>>` injected into task structs. This is user code, not a library abstraction.

Grep evidence for library-level resilience:
- `grep -rn "retry\|circuit.break\|backoff\|timeout\|bulkhead\|resilience\|rate.limit" --include="*.rs"` → all hits are in `examples/circuit_breaker.rs` (user code) and `tests/errors/recovery.rs` (test demonstrating user-space retry patterns). No hits in `src/`.

**Error type:** `DagError` enum (`src/error.rs:8`) has a single variant:
```rust
#[non_exhaustive]
pub enum DagError {
    TaskPanicked { task_id: u32, panic_message: String },
}
```
No error classification (transient/permanent), no ErrorClass enum. Task-level errors (network timeout, DB error, etc.) are expected to propagate as panics or be handled inside the task before returning.

**vs Nebula:** Nebula's `nebula-resilience` crate provides retry/CB/bulkhead/timeout/hedging with a unified `ErrorClassifier`. dagx has none of this — resilience is fully user-space.

---

## 9. Expression / data routing [A7]

**No expression engine or data routing DSL.**

Grep evidence:
- Searched for `expr\|expression\|dsl\|jsonpath\|template\|evaluate\|sandbox` in `src/` — zero hits.

Data routing is pure Rust: `TaskHandle<T>` acts as a typed wire; the type system enforces that only the correct type can be passed between tasks. There is no `$nodes.foo.result.email` expression syntax or dynamic routing.

**vs Nebula:** Nebula has a 60+ function expression engine with type inference, sandbox evaluation, and JSONPath-like syntax. dagx's "expression engine" is Rust generics — compile-time correct but not dynamically composable.

---

## 10. Plugin / extension system [A11] — DEEP

**No plugin system exists.**

### 10.A — Plugin BUILD process

**A11.1 — Format:** None. No plugin manifest, no .tar.gz, no OCI image, no WASM blob.

**A11.2 — Toolchain:** None.

**A11.3 — Manifest content:** None.

**A11.4 — Registry/discovery:** None.

Grep evidence:
- `grep -r "plugin\|wasm\|extension\|dylib\|dynamic" --include="*.rs" -l` → returns only `src/node.rs`, where the hit is `node.rs:26`: "need **dynamic** dispatch to execute tasks with different input/output types" — this refers to `dyn ExecutableNode`, an internal type erasure mechanism, not an extension point.

### 10.B — Plugin EXECUTION sandbox

**A11.5 — Sandbox type:** None. No WASM runtime, no dynamic library loading, no subprocess IPC.

**A11.6 — Trust boundary:** Not applicable.

**A11.7 — Host-plugin calls:** Not applicable.

**A11.8 — Lifecycle:** Not applicable.

**A11.9 — vs Nebula:** Nebula targets WASM sandbox via wasmtime, capability security, and a Plugin Fund commercial model. dagx has no plugin system and no commercial model. The extension model is pure Rust trait impl — "your crate, your task struct."

---

## 11. Trigger / event model [A12] — DEEP

**No trigger or event system exists.**

### A12.1 — Trigger types

None of: webhook, schedule/cron, external event (Kafka/NATS/Redis), FS watch, DB change, polling, internal event, manual trigger. dagx is an in-process executor; "triggering" a DAG run means calling `dag.run(spawner).await` from application code.

Grep evidence:
- `grep -rn "trigger\|webhook\|schedule\|cron\|event" --include="*.rs"` in `src/` and library code → all hits are in test files and examples where "event" is used in general English (e.g., "trigger open" in circuit breaker comment) or in `src/lib.rs` doc comment for "trigger" in an unrelated sentence. Zero trigger-domain API hits.

### A12.2 — Webhook
Not applicable.

### A12.3 — Schedule
Not applicable.

### A12.4 — External event
Not applicable.

### A12.5 — Reactive vs polling
Not applicable.

### A12.6 — Trigger→workflow dispatch
Not applicable.

### A12.7 — Trigger as Action
Not applicable. The concept does not exist in dagx.

### A12.8 — vs Nebula
Nebula models triggers as `TriggerAction` with `Input = Config` (registration) and `Output = Event` (typed payload), using a 2-stage Source → Event architecture. dagx has no trigger concept — the entire lifecycle of triggering, persisting, and dispatching a workflow run is out of scope.

---

## 12. Multi-tenancy [A14]

**No multi-tenancy.**

Grep evidence:
- `grep -rn "multi.*tenant\|tenant\|rbac\|sso\|scim" --include="*.rs"` → zero hits.

dagx is a pure computation primitive — no user concept, no workspace, no isolation boundary.

**vs Nebula:** Nebula has `nebula-tenant` crate with three isolation modes (schema/RLS/database), RBAC, planned SSO, planned SCIM. Complete delta.

---

## 13. Observability [A15]

**Optional tracing support via feature flag.**

When the `tracing` feature is enabled (`Cargo.toml` line 10), dagx emits structured events via the `tracing` crate at four log levels (README.md, tracing section):

- **INFO:** DAG execution start/completion
- **DEBUG:** task additions, dependency wiring, layer computation
- **TRACE:** individual task execution (inline vs spawned), detailed execution flow
- **ERROR:** task panics, concurrent execution attempts

Implementation: `#[cfg(feature = "tracing")]` guards throughout `src/runner.rs` and `src/builder.rs`. The `run()` method is instrumented with `#[cfg_attr(feature = "tracing", tracing::instrument(skip(self, spawner)))]` (`src/runner.rs:266`).

No OpenTelemetry integration. No metrics (counters, histograms). No structured tracing of individual task latency per execution. Zero-cost when disabled ("literally 0ns overhead — code removed at compile time", README.md).

**vs Nebula:** Nebula uses OpenTelemetry with structured tracing per execution (one trace = one workflow run) and metrics per action (latency/count/errors). dagx's observability is debug-level logging only, not production telemetry.

---

## 14. API surface [A16]

**Library API only — no network API.**

The public API (re-exported from `src/lib.rs:483-491`):
```rust
pub use builder::{TaskBuilder, TaskHandle};
pub use error::{DagError, DagResult};
pub use output::DagOutput;
pub use runner::DagRunner;
pub use task::{Task, TaskInput};
#[cfg(feature = "derive")]
pub use dagx_macros::task;
```

7 public items. No REST API, no GraphQL, no gRPC, no HTTP server, no OpenAPI spec. The library is embedded in the user's binary — no network protocol.

**API stability:** Version 0.3.1. `DagError` is `#[non_exhaustive]`. Three breaking changes across 4 minor versions (MSRV bump, `DagRunner` single-threaded, `TaskHandle` removal of Clone/Copy). No stability guarantees documented.

**vs Nebula:** Nebula has a REST API, planned GraphQL/gRPC, generated OpenAPI spec, and OwnerId-aware per-tenant routing. dagx is an embedded library with no network surface.

---

## 15. Testing infrastructure [A19]

**Strong unit + integration test coverage (~83% per CI configuration).**

Test organization (22 integration test files, `tests/`):
- `cycle_prevention.rs` — `compile_fail` tests proving cycle impossibility
- `custom_type_test.rs` — regression tests for v0.3.1 custom type support
- `boundaries/` — deep chains, zero tasks, type limits
- `dependencies/` — tuple arities, resolution
- `errors/` — propagation, user-space retry/recovery patterns
- `execution/` — basic execution correctness
- `interleaving/` — multi-layer execution order
- `parallelism/` — spawning, threads, timing proofs
- `runtimes/` — tokio, smol, async-executor, pollster, futures-executor compatibility
- `tracing/` — with/without feature flag

CI configuration (`ci.yml`): 18-run matrix (3 platforms × 2 Rust versions × 3 feature combos). Coverage threshold: 80% (enforced via tarpaulin). `#[cfg_attr(tarpaulin, ignore)]` on timing-sensitive and resource-intensive tests.

**No public testing utilities** — `dagx-test` is explicitly not meant for public use (crate doc: "This crate is not meant for public use and offers no stability guarantees").

**vs Nebula:** Nebula has a `nebula-testing` crate with public testing utilities, contract tests for resource implementors (resource-author-contracts.md), insta snapshots, wiremock, and mockall. dagx's test infra is more focused (no contract testing, no public utilities) but has broader runtime matrix coverage.

---

## 16. AI / LLM integration [A21] — DEEP

**No AI or LLM integration.**

### A21.1 — Existence

None. There is no built-in LLM integration, no separate AI crate, no community AI plugin.

Grep evidence:
- `grep -r "llm\|openai\|anthropic\|embedding\|completion\|ai\|gpt\|claude\|mistral\|ollama" --include="*.rs"` → zero hits across all source files, examples, tests, and benchmarks.

### A21.2–A21.13

All not applicable. dagx has no provider abstraction, no prompt management, no structured output, no tool calling, no streaming LLM integration, no multi-agent pattern, no RAG/vector search, no conversation memory, no token counting, no LLM-specific observability, and no safety/filtering layer.

### A21.13 — vs Nebula+Surge

Nebula's bet is "AI = generic actions + plugin LLM client" with Surge as a separate agent orchestrator on ACP. dagx is compatible with this bet — LLM calls can be wrapped as dagx tasks. However dagx has no opinion on AI architecture and provides no accelerators.

---

## 17. Notable design decisions

### 1. Typestate cycle prevention vs runtime detection

**Decision:** Make cycles impossible to express in the type system by consuming `TaskBuilder` on wiring and giving `TaskHandle` no `depends_on()` method.

**Trade-off:** Zero runtime overhead, compiler-verified. Cost: up to 8 deps per task (hard limit from tuple implementations); no dynamic DAG construction; DAG must be fully wired before `run()`.

**Applicability to Nebula:** Nebula's TypeDAG already uses compile-time generics at L1. The dagx pattern (typestate + consumption) is a clean complement. Nebula could use this for workflow builder APIs that need to prevent non-DAG configurations at source-code level.

### 2. Runtime-agnostic spawner closure

**Decision:** `run(spawner: S)` where `S: Fn(BoxFuture) -> F` lets the user inject any async runtime.

**Trade-off:** Works with tokio, smol, pollster, any executor. Cost: function-pointer overhead per spawn, slightly complex signature, user must understand futures.

**Applicability to Nebula:** Nebula is tokio-only. Runtime agnosticism is not a goal — but the spawner abstraction pattern could be useful for testing (inject a synchronous executor for deterministic test runs).

### 3. Inline fast-path for single-task layers

**Decision:** If a topological layer has exactly one task, execute it inline (no spawn). See `src/runner.rs:308`.

**Trade-off:** 10-100x speedup for sequential chains. No behavioral difference — panics caught identically via `catch_unwind`. Cost: bifurcated code path (inline vs spawned).

**Applicability to Nebula:** Nebula's engine could benefit from an analogous optimization for single-active-node frontier states, avoiding unnecessary scheduling overhead in linear workflow branches.

### 4. `Arc<T>` output wrapping for O(1) fan-out

**Decision:** All task outputs are internally wrapped in `Arc<T>` before being stored in the output map (`src/node.rs:91`). Fan-out (1→N) clones the Arc N times rather than cloning T.

**Trade-off:** Eliminates `Clone` requirement on task output types (removed in PR #7). Cost: one extra atomic refcount per output. For small types (int, bool) Arc overhead exceeds copy cost.

**Applicability to Nebula:** Nebula already uses similar patterns in its data flow. The insight that user-visible types don't need `Clone` by having the framework own Arc wrapping is clean API design.

### 5. Panic-as-error boundary

**Decision:** Both inline and spawned tasks are wrapped in `AssertUnwindSafe(...).catch_unwind()` (`src/runner.rs:328, 390`). Panics become `DagError::TaskPanicked`.

**Trade-off:** Consistent behavior across runtimes (tokio propagates panics as `JoinError`; smol drops them differently). Cost: `AssertUnwindSafe` can mask UB if tasks violate it (user responsibility). DAG aborts after first panicking layer.

**Applicability to Nebula:** Nebula uses explicit `Result` propagation and typed `DagError`/`ErrorClass`. The catch-unwind approach is useful for "user code in library" contexts; Nebula could apply this at action-execution boundaries as a defense layer.

### 6. No petgraph dependency

**Decision:** Represent the DAG as two plain `HashMap<NodeId, Vec<NodeId>>` adjacency lists rather than using petgraph.

**Trade-off:** Zero external graph dependency, simpler build. The passthrough hasher (`src/runner.rs:26`) removes even the hashing overhead. Cost: only Kahn's algorithm is implemented — no DFS, no SCC, no graph traversal queries. No `NodeIndex` type-level graph proofs.

**Applicability to Nebula:** Nebula uses petgraph at L4 for soundness checks. dagx shows that for simple execution scheduling, petgraph is not required. If Nebula's petgraph usage is limited to topological sort, the dependency could be replaced with a simpler implementation.

---

## 18. Known limitations and pain points

dagx has only 3 GitHub issues total (all closed), none with reactions > 0. This indicates very early-stage adoption. Limitations identified from code analysis and CHANGELOG:

**Hard dependency limit of 8 per task:** Enforced by macro expansion and tuple impls (`src/deps.rs:51-58`, `src/task.rs:81-88`). The README acknowledges this: "If you need more than 8 dependencies: 1. Group related inputs into a struct 2. Use intermediate aggregation tasks 3. Consider if 8+ dependencies indicates a design issue."

**No dynamic DAG:** The DAG must be fully constructed before `run()`. No conditional branching, no loops, no runtime graph modification. The COMPARISONS.md doc explicitly acknowledges: "dagx is not the right choice if you need: Cyclic graphs or dynamic flow control (loops, conditions) → Consider dagrs or tasksitter."

**No error recovery across layers:** On the first task failure (panic), execution aborts after the current layer completes (`src/runner.rs:437`). There is no `rescue`, no partial result retrieval, no error isolation per branch.

**No workflow persistence:** A completed `DagRunner` is consumed. There is no snapshot, no resume-from-checkpoint, no persistent execution history.

**PR #14 (unreleased):** `TaskHandle` lost `Clone` and `Copy`. This is a breaking change from the perspective of users who saved handles for later output retrieval — `DagOutput::get()` now consumes the handle, so each handle can only be retrieved once.

---

## 19. Bus factor / sustainability

**Bus factor: 1.** Single maintainer (Stephen Waits). Only 1 fork. All 12 PRs authored by `TechnoPorg` (a contributor) but all merged by the maintainer.

**Commit cadence:** Active development in October 2025 (v0.1.0 to v0.3.1 in 3 days). Then quiet until April 2026 (repository updated but no release). Issue #15 "Release v0.4" is open, indicating planned future work.

**Stars:** 19 — minimal community adoption.

**Issues ratio:** 0 open / 3 total. No community feedback or bug reports beyond the maintainer's own roadmap items.

**Risk:** High bus factor risk for production adoption. However, as a library primitive with a minimal, stable API surface, community adoption risk is lower than for a framework.

---

## 20. Final scorecard vs Nebula

| Axis | dagx approach | Nebula approach | Verdict | Borrow? |
|------|--------------|-----------------|---------|---------|
| A1 Workspace | 3 crates (dagx + dagx-macros + dagx-test), flat | 26 crates, layered nebula-error → nebula-engine → nebula-tenant | Competitor simpler, Nebula richer — different goals | no — different goals |
| A2 DAG | Typestate cycle prevention at L1; runtime Kahn BFS scheduling; no petgraph; compile-time port types | TypeDAG L1-L4 (generics → TypeId → predicates → petgraph) | dagx has better compile-time cycle prevention (typestate); Nebula has deeper graph semantics (petgraph, L4) | refine — typestate pattern for builder API |
| A3 Action | Single open `Task<Input>` trait; 1 assoc type (`Output`); input = generic tuple param; `#[task]` macro; no versioning, no hooks, no lifecycle | 5 action kinds (Process/Supply/Trigger/Event/Schedule), sealed trait, assoc Input/Output/Error, derive macros | Competitor simpler (correct for its scope); Nebula deeper (correct for orchestration) | no — different goals |
| A4 Credential | None — not applicable | State/Material split, LiveCredential, blue-green refresh, OAuth2Protocol | Nebula deeper (complete credential subsystem) | no — different goals |
| A5 Resource | None — not applicable | 4 scope levels, ReloadOutcome, generation tracking, on_credential_refresh | Nebula deeper | no — different goals |
| A6 Resilience | User-space only (circuit breaker example); no framework retry/CB/timeout | nebula-resilience: retry/CB/bulkhead/timeout/hedging, ErrorClassifier | Nebula deeper | no — different goals |
| A7 Expression | None — pure Rust type wiring | 60+ functions, type inference, sandboxed eval, `$nodes.foo.result.email` | Nebula deeper | no — different goals |
| A8 Storage | None — in-memory only | sqlx + PgPool + RLS, Pg*Repo, SQL migrations | Nebula deeper | no — different goals |
| A9 Persistence | None — one-shot in-process executor | Frontier + checkpoint + append-only log + replay | Nebula deeper | no — different goals |
| A10 Concurrency | tokio/smol/any runtime via spawner closure; Kahn BFS layer scheduler; Send required | tokio + frontier scheduler + work-stealing + !Send isolation | Different decomposition — dagx runtime-agnostic; Nebula deeper scheduling | refine — inline single-task fast-path |
| A11 Plugin BUILD | None | WASM + plugin-v2 spec + Plugin Fund | Nebula deeper | no — different goals |
| A11 Plugin EXEC | None | WASM sandbox (wasmtime) + capability security | Nebula deeper | no — different goals |
| A12 Trigger | None — pure library, no trigger concept | TriggerAction Source→Event 2-stage | Nebula deeper | no — different goals |
| A13 Deployment | Embedded library (no deployment modes) | 3 modes from one binary (desktop/serve/cloud) | Different decomposition | no — different goals |
| A14 Multi-tenancy | None | nebula-tenant: schema/RLS/db, RBAC, SSO planned | Nebula deeper | no — different goals |
| A15 Observability | Optional tracing feature (debug logging only) | OpenTelemetry per execution, metrics per action | Nebula deeper | no — different goals |
| A16 API | Library API only (7 public items); no network | REST + planned GraphQL/gRPC, OpenAPI | Nebula deeper | no — different goals |
| A17 Type safety | Typestate on TaskBuilder/TaskHandle; compile-time port types; no GATs/HRTBs/Validated<T> | Sealed traits, GATs, HRTBs, typestate, Validated<T> proof tokens | Nebula deeper (more advanced type system) | refine — TaskHandle/TaskBuilder typestate pattern |
| A18 Errors | Single-enum `DagError` with `TaskPanicked` variant; `#[non_exhaustive]`; no classification | nebula-error + ErrorClass enum (transient/permanent/cancelled) | Nebula deeper — dagx intentionally minimal | no — Nebula's already better |
| A19 Testing | 22 integration test files, 83% coverage, 18-run CI matrix (platform × rust × features); no public test utils | nebula-testing crate, contract tests, insta/wiremock/mockall | Different — dagx has better runtime matrix breadth; Nebula has better test utilities | refine — runtime compatibility matrix testing |
| A20 Governance | MIT, solo maintainer, no commercial model | Open core, Plugin Fund, planned SOC 2, solo maintainer | Different — Nebula has clearer commercial story | no — different goals |
| A21 AI/LLM | None — no AI integration whatsoever | None yet — generic actions + plugin LLM bet | Convergent (both absent) | no — Nebula's strategy already correct |
