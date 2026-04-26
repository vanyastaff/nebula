# Treadle — Architectural Decomposition

## 0. Project Metadata

- **Repo:** https://github.com/oxur/treadle
- **Stars:** 2 (as of 2026-04-26)
- **Forks:** 1
- **Last push:** 2026-02-08 (created 2026-02-08 — project is ~2.5 months old)
- **Crates.io version:** 0.2.0 (37 total downloads)
- **License:** MIT OR Apache-2.0
- **Governance:** Solo project (maintainer handle not yet disclosed in README; the CLAUDE.md references "Taproot" which is likely the internal codename for the author's tooling project suite `oxur`)
- **Open issues:** 0 (GitHub confirmed empty issue list)
- **Rust edition:** 2021; `rust-version = "1.75"`

---

## 1. Concept Positioning [A1, A13, A20]

**Author's own README sentence:**
> "A persistent, resumable, human-in-the-loop workflow engine backed by a petgraph DAG"

**After reading code:**
Treadle is a single-crate, embeddable Rust library (not a service) for running work items through a DAG of stages with SQLite-persisted state, human review gates, fan-out subtask execution, and a tokio broadcast event stream — designed for local CLI tools and single-process AI-agent pipelines, with a documented v2 roadmap for quality-gate-driven iterative retry loops.

**Comparison with Nebula:**
Treadle is architecturally far narrower than Nebula. Where Nebula is a 26-crate workflow *platform* (multi-tenant, cloud-deployable, credential-aware, plugin-first), Treadle is a focused workflow *library* for single-process use. Treadle explicitly positions itself in the gap between "single-shot DAG executors" and "distributed workflow engines" (docs/related-projects.md); Nebula occupies the high end of that spectrum.

---

## 2. Workspace Structure [A1]

**Single crate, no workspace.** `Cargo.toml` (root, lines 1–30) defines one package `treadle = "0.2.0"` with no `[workspace]` section.

Crate layout:
- `src/lib.rs` — public re-export hub
- `src/stage.rs` — `Stage`, `StageOutcome`, `StageState`, `StageStatus`, `SubTask`, `StageContext`, `ReviewData`
- `src/workflow.rs` — `Workflow`, `WorkflowBuilder`, petgraph-backed DAG, executor
- `src/work_item.rs` — `WorkItem` trait
- `src/event.rs` — `WorkflowEvent` enum (11 variants)
- `src/error.rs` — `TreadleError`, `Result<T>`
- `src/status.rs` — `PipelineStatus`, `StageStatusEntry`
- `src/state_store/mod.rs` — `StateStore` trait
- `src/state_store/memory.rs` — `MemoryStateStore`
- `src/state_store/sqlite.rs` — `SqliteStateStore` (feature-gated)

Feature flags: one feature, `sqlite` (default), gates rusqlite dependency (`Cargo.toml:23-24`).

Total source lines: ~5,824 (excluding tests inline; tests are inside the same files). Integration tests: `tests/integration.rs` (448 lines). One example: `examples/basic_pipeline.rs`.

**Comparison:** Nebula has 26 crates with strict layering (nebula-error / nebula-resilience / nebula-credential / nebula-resource / nebula-action / nebula-engine / etc.). Treadle is a single flat crate — appropriate for its scope but unextendable as a platform.

---

## 3. Core Abstractions [A3, A17] — DEEP

### A3.1 — Trait shape

The primary abstraction is `Stage`, defined in `src/stage.rs:334`:

```rust
#[async_trait]
pub trait Stage: Debug + Send + Sync {
    async fn execute(&self, item: &dyn WorkItem, context: &mut StageContext) -> Result<StageOutcome>;
    fn name(&self) -> &str;
    async fn before_execute(&self, _item: &dyn WorkItem, _context: &StageContext) -> Result<()> { Ok(()) }
    async fn after_execute(&self, _item: &dyn WorkItem, _context: &StageContext, _outcome: &StageOutcome) -> Result<()> { Ok(()) }
}
```

- **Open trait** — not sealed. Any type in any crate can implement `Stage`.
- **`dyn Stage` compatible** — the codebase stores stages as `Arc<dyn Stage>` (see `workflow.rs:23`).
- **Associated types:** zero. Unlike Nebula's typed `Input`/`Output`/`Error` associated types, Treadle's `Stage` has no associated types. Input is `&dyn WorkItem` (type-erased), output is `StageOutcome` (an enum), errors fold into `Result<StageOutcome>` via `TreadleError`.
- **No GATs, no HRTBs, no typestate.** The trait is deliberately simple for object safety.
- **`async_trait` crate** used rather than native async-in-trait (Rust edition 2021 does not have stable `async fn in trait`; `rust-version = "1.75"` predates the stable async-fn-in-trait stabilization that would enable it without the macro).

### A3.2 — I/O shape

- **Input:** `&dyn WorkItem` — fully type-erased. `WorkItem` trait (`src/work_item.rs:45`) requires only `fn id(&self) -> &str`, `Debug`, `Send`, `Sync`. The engine never inspects work item internals; stages cast via `&dyn WorkItem` and must downcast themselves if they need typed access.
- **Output:** `StageOutcome` enum (`src/stage.rs:227`) with five variants: `Complete`, `NeedsReview`, `Retry`, `Failed`, `FanOut(Vec<SubTask>)`.
- **Side-effect model:** stages perform all side effects themselves; the engine only sees the outcome. No streaming output; output is complete-or-not.

### A3.3 — Versioning

No versioning mechanism. Stages are referenced by string name only (`Workflow::builder().stage("scan", ScanStage)`). There is no v1 vs v2 type-tag, no `#[deprecated]`, no name+version identifier. The v2 design (Phase 6 design doc) plans to change `Stage::execute` return type from `Result<StageOutcome>` to `Result<StageOutput>` with a `From<StageOutcome> for StageOutput` backward-compat adapter.

### A3.4 — Lifecycle hooks

Three hooks: `before_execute` (pre), `execute` (main), `after_execute` (post). All async. No cleanup or on-failure hook. No cancellation points in the trait definition. No explicit idempotency key at the trait level.

### A3.5 — Resource and credential dependencies

None. Stages receive only `&dyn WorkItem` and `&mut StageContext`. No mechanism for a stage to declare "I need DB pool X or credential Y." Stages must manage their own dependencies via closures, constructors, or `Arc`-wrapped state stored in the stage struct.

### A3.6 — Retry/resilience attachment

Manual. The `StageOutcome::Retry` variant marks a stage as needing retry, which currently (v1) just marks it `Paused` with an incremented retry counter (`workflow.rs:614-624`). The comment at `workflow.rs:466` states: "For now, treat retry as needing review // Full retry logic will be in Milestone 4.4." The v2 design (Phase 8) adds `RetryBudget` configured at workflow builder level — not inside the `Stage` trait.

### A3.7 — Authoring DX

Manual impl, no derive macros. A "hello world" stage:

```rust
#[derive(Debug)]
struct ScanStage;

#[async_trait]
impl Stage for ScanStage {
    fn name(&self) -> &str { "scan" }
    async fn execute(&self, _item: &dyn WorkItem, _ctx: &mut StageContext) -> Result<StageOutcome> {
        Ok(StageOutcome::Complete)
    }
}
```

That is ~10 lines with the proc-macro import. No derive macros, no builder DSL.

### A3.8 — Metadata

Stage name is the only metadata, returned by `fn name(&self) -> &str`. No description, icon, category, or i18n. The name is runtime (not compile-time); it is a string used for state store keying.

### A3.9 — vs Nebula comparison

Nebula has **5 sealed action kinds** (Process/Supply/Trigger/Event/Schedule) each with typed `Input`/`Output`/`Error` associated types and derive macros. Treadle has **1 open trait** with zero associated types and a 5-variant outcome enum. Treadle is simpler to onboard but loses all compile-time type safety: the engine cannot verify at compile time that output from stage A is valid input for stage B.

---

## 4. DAG / Execution Graph [A2, A9, A10]

### Graph implementation

`petgraph::graph::DiGraph<RegisteredStage, ()>` backing (`workflow.rs:72`). Cycle detection runs at `WorkflowBuilder::build()` time via `petgraph::algo::is_cyclic_directed` (producing `TreadleError::DagCycle` if a cycle is found). Topological order is computed once at build time using `petgraph::algo::toposort` and cached in `topo_order: Vec<String>` (`workflow.rs:77`).

**Port typing:** none. Edges carry no type information `DiGraph<_, ()>`. There is no equivalent of Nebula's TypeDAG L1–L4 compile-time port type enforcement.

**Compile-time checks:** only that stage names used in `dependency()` calls exist in the graph (name lookup; runtime error if not found). No type checking between stages.

### Scheduler model

Single-process, sequential within a recursive `advance_internal` call. The method identifies ready stages (deps satisfied, status Pending) then executes them one by one in a `for stage_name in ready_stages` loop (`workflow.rs:454`). After each round, if progress was made, it recurses (depth-limited to 100, `workflow.rs:427`). Fan-out subtasks within a single fan-out stage are also executed sequentially in a for-loop (`workflow.rs:692`), not via `tokio::spawn`. There is no work-stealing or true parallel scheduling.

### Concurrency

Single tokio task. The `advance` method is called by the user and blocks until the workflow stalls. The `Workflow` struct is `Send + Sync` (it wraps `Arc<dyn Stage>` and a `broadcast::Sender`). Fan-out does not spawn parallel tasks; subtask execution is sequential.

**Comparison with Nebula:** Nebula uses a frontier-based scheduler with work-stealing semantics and explicit `!Send` isolation. Treadle executes ready stages sequentially within the advance call.

---

## 5. Persistence & Recovery [A8, A9]

### Storage

Two backends:
1. **`MemoryStateStore`** — `Arc<tokio::sync::RwLock<HashMap<...>>>` in-memory, no persistence.
2. **`SqliteStateStore`** — `rusqlite::Connection` wrapped in `Arc<tokio::sync::Mutex<Connection>>`, file-backed (`src/state_store/sqlite.rs:75-77`). Uses `tokio::task::spawn_blocking` for all DB operations.

**Schema** (inlined SQL in `sqlite.rs:21-51`):
- `stage_states(work_item_id, stage_name, state_json, updated_at)` — PK is `(work_item_id, stage_name)`
- `work_items(work_item_id, data_json, updated_at)`
- `schema_version(version)` with version = 1

State is stored as JSON (`serde_json` serialization of `StageState`). There are no SQL migrations beyond schema v1 creation.

### Recovery model

On restart, the caller reopens `SqliteStateStore::open("workflow.db")`, rebuilds the `Workflow` struct from code, and calls `workflow.advance(&item, &mut store)`. The executor reads existing stage states and skips stages that are already `Complete`. This is a "checkpoint-at-stage-completion" model, not event sourcing or journaling.

**Comparison with Nebula:** Nebula uses append-only execution log + frontier-based checkpoint recovery with replay semantics. Treadle uses direct state overwrite in SQLite — simpler but not append-only, no replay capability.

---

## 6. Credentials / Secrets [A4] — DEEP

**A4.1 Existence:** No credential layer exists. Treadle has no concept of credentials, secrets, tokens, or authentication.

**Grep evidence (src/ directory):**
```
grep -ri "credential|secret|token|auth|oauth|password" targets/treadle/src/
→ Found 0 total occurrences across 0 files.
```

**A4.2 through A4.9:** Not applicable. No storage, no in-memory protection, no lifecycle, no OAuth2, no composition, no scope, no type safety for credentials. This is an intentional omission consistent with the library's embedded/local-CLI scope — it delegates credential management to the embedding application.

**vs Nebula:** Nebula has an entire `nebula-credential` crate with State/Material split, `LiveCredential` watch(), blue-green refresh, OAuth2Protocol blanket adapter, and DynAdapter type erasure. Treadle has nothing analogous.

---

## 7. Resource Management [A5] — DEEP

**A5.1 Existence:** No resource abstraction exists. There are no DB pools, HTTP clients, or caches managed by the Treadle engine.

**Grep evidence (src/ directory):**
```
grep -ri "resource|pool|client|cache|reload|generation" targets/treadle/src/
→ 0 matches (excluding "resource" appearing in Cargo keywords/categories).
```

**A5.2 through A5.8:** Not applicable. Stages manage their own dependencies via constructor injection. The `StageContext` carries `metadata: HashMap<String, serde_json::Value>` for passing data between stages, but this is not a resource management layer.

**vs Nebula:** Nebula has 4 scope levels, `ReloadOutcome` enum, generation tracking for cache invalidation, and `on_credential_refresh` per-resource hooks. Treadle has none of these.

---

## 8. Resilience [A6, A18]

**Retry:** Manual via `workflow.retry_stage(item_id, "stage_name", store)` (`workflow.rs` — exists in the 2418-line file). `StageOutcome::Retry` exists but is currently treated as "paused" in v1. The v2 design (Phase 8-9) adds `RetryBudget` with configurable `max_attempts`, `attempt_timeout`, and `ExhaustedAction` (Fail or Escalate).

**Circuit breaker / bulkhead / hedging / timeout:** None in v1. Phase 9 adds `attempt_timeout: Option<Duration>` per stage. No circuit breaker or bulkhead concept planned.

**Error classification:** None. `TreadleError` (`src/error.rs:14-56`) has 8 structural variants (`StateStore`, `StageExecution`, `InvalidWorkflow`, `WorkItemNotFound`, `StageNotFound`, `DuplicateStage`, `DagCycle`, `Serialization`, `Io`, `Database`) but no transient/permanent classification (no equivalent of Nebula's `ErrorClass` enum or `ErrorClassifier`).

**vs Nebula:** Nebula has a dedicated `nebula-resilience` crate with retry/CB/bulkhead/timeout/hedging and unified `ErrorClassifier`. Treadle is significantly shallower.

---

## 9. Expression / Data Routing [A7]

**No expression engine exists.** Treadle has no DSL, no `$nodes.foo.result.email`-style templating, no type inference, no sandboxed eval.

**Grep evidence:**
```
grep -ri "expression|template|dsl|eval|sandbox" targets/treadle/src/
→ 0 matches
```

Data flows between stages only via explicit stage logic. Stages can read `StageContext::metadata` (a `HashMap<String, serde_json::Value>`) but this is a free-form side channel, not a typed routing layer.

**vs Nebula:** Nebula has a 60+ function expression engine with type inference and `$nodes.foo.result.email` syntax. Treadle has no equivalent and none is planned.

---

## 10. Plugin / Extension System [A11] — DEEP

### 10.A — Plugin BUILD process

**No plugin system exists.** There is no plugin concept, no manifest format, no registry, no build SDK.

**Grep evidence:**
```
grep -ri "plugin|wasm|extension" targets/treadle/src/
→ 0 matches
```

The extension mechanism is implementing the open `Stage` trait and adding it to the workflow via `WorkflowBuilder::stage()`.

**A11.1 through A11.4:** Not applicable.

### 10.B — Plugin EXECUTION sandbox

**No sandbox exists.** All stages execute in-process.

**A11.5 through A11.9:** Not applicable. There is no WASM runtime, no subprocess model, no RPC, no trust boundary, no capability model.

**vs Nebula:** Nebula targets WASM sandbox + capability security + Plugin Fund commercial model. Treadle has no plugin architecture at all — stages are first-class Rust code compiled into the binary.

---

## 11. Trigger / Event Model [A12] — DEEP

**A12.1 Trigger types:** Treadle has no external trigger model. There is no webhook, cron, Kafka, polling, or FS watch. Work items enter the pipeline when the user calls `workflow.advance(&item, &mut store)`.

**Grep evidence:**
```
grep -ri "webhook|cron|schedule|trigger|kafka|pubsub|polling" targets/treadle/src/
→ 0 matches
```

**A12.2 Webhook:** Not applicable.

**A12.3 Schedule:** Not applicable.

**A12.4 External event:** Not applicable.

**A12.5 Reactive vs polling:** Not applicable. The trigger model is entirely caller-driven — no internal polling or reactivity.

**A12.6 Trigger→workflow dispatch:** Items are submitted explicitly; no fan-out at the trigger level.

**A12.7 Trigger as Action:** Treadle has no equivalent of Nebula's `TriggerAction` kind. Triggers are external to the library.

**A12.8 vs Nebula:** Nebula models triggers as a first-class action kind (`TriggerAction`, with `Input = Config` and `Output = Event`) via a 2-stage Source → Event normalization pipeline. Treadle has no equivalent. The entire Nebula trigger/event model is absent.

**The event system Treadle does have** is an *observation* stream (not an ingestion model): `WorkflowEvent` (11 variants, `src/event.rs`) broadcast via `tokio::sync::broadcast::Sender<WorkflowEvent>`. This is for monitoring (TUI/CLI observers) not for ingesting external triggers.

---

## 12. Multi-tenancy [A14]

**Not applicable / not present.** Treadle is a single-user embedded library. There is no tenant concept, no RBAC, no SSO, no SCIM, no schema isolation.

---

## 13. Observability [A15]

**Tracing:** `tracing` crate (`Cargo.toml:21`). The executor uses structured spans: `info_span!("advance", item_id)`, `info_span!("stage", stage)`, `info_span!("fanout", stage, subtask_count)`. Debug/info/warn events throughout. Instrumented via `.instrument(span)` pattern (`workflow.rs:414-416`).

**Metrics:** None. No OpenTelemetry, no metrics.

**Event stream:** `WorkflowEvent` enum (11 variants) broadcast via `tokio::sync::broadcast` channel with capacity 256 (`workflow.rs:16`). Events cover stage started/completed/failed, review required, retry, fan-out started, subtask started/completed/failed, and workflow completed.

**Pipeline status:** `PipelineStatus` with per-stage `StageStatusEntry` including timing, retry count, error, and subtask details (`src/status.rs`). Progress percentage calculation via `progress_percent()`.

**vs Nebula:** Nebula uses OpenTelemetry with one trace per workflow execution. Treadle uses the `tracing` crate directly (compatible with OTel subscribers, but the integration is the user's responsibility).

---

## 14. API Surface [A16]

**Programmatic (library) API only.** Treadle is a Rust library, not a service. No HTTP API, no GraphQL, no gRPC, no OpenAPI spec.

The public API surface (`src/lib.rs:241-251`):
- `TreadleError`, `Result`
- `WorkflowEvent`
- `Stage`, `StageContext`, `StageOutcome`, `StageState`, `StageStatus`, `SubTask`, `ReviewData`
- `StateStore`, `MemoryStateStore`, `SqliteStateStore` (feature-gated)
- `PipelineStatus`, `StageStatusEntry`
- `WorkItem`
- `Workflow`, `WorkflowBuilder`
- `version() -> &'static str`

No versioning scheme for the API beyond semver on the crate.

---

## 15. Testing Infrastructure [A19]

**Unit tests:** Inline in each source file. 149 unit tests (`CHANGELOG.md:35`). Cover: `StageState`, `SubTask`, `ReviewData`, `StageOutcome`, `StageContext`, `WorkItem`, `TreadleError`, `WorkflowEvent`, `PipelineStatus`, `Workflow` builder and executor.

**Integration tests:** `tests/integration.rs` (448 lines, 8 tests). Tests include: basic pipeline execution, review gate behavior, fan-out execution, retry handling, and event streaming.

**Doc tests:** 9 doc tests in `lib.rs` (`CHANGELOG.md:37`).

**No public testing utilities.** There is no equivalent of Nebula's `nebula-testing` crate or resource-author contract tests. The `MemoryStateStore` is the main testing affordance.

**Total tests:** 166 (CHANGELOG.md:34: "166 total tests").

---

## 16. AI / LLM Integration [A21] — DEEP

**A21.1 Existence:** No built-in LLM integration in the v1 codebase. However, the v2 design document (`docs/design/02-under-review/0002-treadle-v2-design-document.md`) explicitly frames one of the two main use cases as "AI Document Processing" using Claude Code via subprocess invocations. The v2 design adds quality gates and retry-with-feedback to support AI pipeline patterns.

**Grep evidence for built-in LLM support:**
```
grep -ri "openai|anthropic|llm|embedding|completion|gpt|claude" targets/treadle/src/
→ 0 matches (source files only)
```

The two hits in docs are: `0002-treadle-v2-design-document.md` references `generate_embeddings` as a stage name example, and mentions "Claude Code processes a corpus of PDFs" as the motivating use case B.

**A21.2 Provider abstraction:** None. No provider trait, no BYOL endpoint.

**A21.3 Prompt management:** None built-in.

**A21.4 Structured output:** The v2 `QualityGate` trait (Phase 7 design docs) and `QualityVerdict` enum provide a mechanism for evaluating structured output, but this is quality evaluation infrastructure, not LLM output parsing.

**A21.5 Tool calling:** None.

**A21.6 Streaming:** None.

**A21.7 Multi-agent:** The v2 design mentions "Claude Code is called via an API or subprocess" — stages are thin wrappers around external tool invocations. There is no native multi-agent orchestration.

**A21.8 RAG/vector:** The v2 motivating use case B mentions generating embeddings and vector DB entries as pipeline stages, but no built-in vector store integration exists or is planned.

**A21.9 through A21.12:** None.

**A21.13 vs Nebula+Surge:** Treadle's v2 design essentially operationalizes the same pattern Nebula bets on — AI workflows as generic stages + external LLM calls — but Treadle's quality-gate and retry-with-feedback layer is specifically tailored to LLM output quality evaluation. This is an interesting differentiated approach: Treadle handles the "retry loop when LLM output is poor quality" problem as a first-class engine concern, whereas Nebula treats that as application-level logic. Treadle does not yet implement this (it is a design document); Nebula has not addressed it at all.

---

## 17. Notable Design Decisions

### 17.1 Single-crate simplicity

Treadle is one crate with zero internal layering. The entire runtime, state management, and DAG engine are co-located. This makes it trivially embeddable (one `Cargo.toml` line) but unextendable as a platform. The upside is zero dependency friction for CLI tool authors. The downside is that the single crate violates SRP: persistence, concurrency, graph execution, event streaming, and error types all live in `src/workflow.rs` (2,418 lines). As the v2 roadmap adds quality gates, retry budgets, and review policies, this file will grow substantially.

**Trade-off:** Nebula's 26-crate workspace has the opposite problem — high navigation cost, reuse-through-composition design — but is designed for platform evolution.

### 17.2 Human-in-the-loop as first-class engine concern

Most workflow engines treat human review as an external signal (a webhook callback, a database flag polled by the engine). Treadle makes `StageOutcome::NeedsReview` / `StageOutcome::Paused` / `workflow.approve_review()` / `workflow.reject_review()` first-class engine API. This is the defining design decision of the project and is directly motivated by the musicological cataloguing tool use case.

**Borrow signal for Nebula:** Nebula has no explicit human-in-the-loop gate concept. Review gates at the stage level — as a first-class outcome enum variant — could be borrowed, particularly for AI agent pipelines where human oversight is required.

### 17.3 v2 Quality Gate / Retry-with-Feedback separation

The v2 design (Phase 7 design doc) separates three concerns that `Stage::execute` currently conflates: **doing the work** (the stage), **judging the result** (a `QualityGate` trait), and **deciding what to do next** (a `ReviewPolicy` configuration). This is architecturally sound and resembles a judge/executor separation pattern. The `QualityGate::evaluate(artefact, context) -> QualityVerdict` signature keeps quality evaluation separate from work execution.

**Borrow signal for Nebula:** This three-way separation (Stage / QualityGate / ReviewPolicy) is potentially applicable to Nebula's AI-pipeline scenarios.

### 17.4 `async_trait` macro dependency

The crate uses `async_trait = "0.1"` on both `Stage` and `StateStore` traits. With `rust-version = "1.75"`, stable `async fn in trait` (AFIT, stabilized in Rust 1.75.0) should be available without the macro for object-safe traits. The project's CLAUDE.md references a Taproot coding standards file suggesting they track anti-patterns; this may be a deliberate choice for object safety (the `async_trait` macro generates box pinned futures which work with `dyn Stage`, whereas naive AFIT requires `use<>` bounds that limit dyn compatibility). This is worth investigating as a potential technical debt item.

### 17.5 SQLite state store with schema_version = 1

The SQLite schema (`sqlite.rs:21-51`) uses a fixed version 1 with no migration framework. The v2 design (Phase 6.5) plans to `ALTER TABLE stage_statuses ADD COLUMN artefact_summary TEXT` for schema v2. This will require a migration path that the current schema_version table is presumably designed to support, but no migration runner exists yet.

---

## 18. Known Limitations / Pain Points

The repository has **0 GitHub issues** (confirmed by `gh issue list` returning `[]`). No closed issues. The project is 2.5 months old with 2 stars, so no community pain points are documented externally.

From internal documentation and code comments:

1. **Retry semantics incomplete in v1** — `workflow.rs:466`: "For now, treat retry as needing review // Full retry logic will be in Milestone 4.4." The `StageOutcome::Retry` variant exists but behaves identically to `NeedsReview` in the current implementation. This is a documented gap.

2. **Fan-out is sequential** — `workflow.rs:692-760`: subtasks in a fan-out stage are executed in a sequential `for` loop, not via `tokio::spawn`. The README implies parallelism ("concurrent subtasks") but the implementation is sequential. This would be a correctness/performance issue for large fan-outs.

3. **`advance_internal` recursion** — the recursion depth limit of 100 (`workflow.rs:427`) is a defensive guard but signals the absence of a proper task queue. Deep pipelines could hit this limit.

4. **No stage output passing** — in v1, stages cannot receive typed output from upstream stages. A downstream stage cannot access what the previous stage computed. This is the primary motivation for the v2 `Artefact` trait / `StageOutput` redesign.

5. **`arc::dyn Stage` + `async_trait` box overhead** — each stage execution allocates a `Box<dyn Future>` via the `async_trait` macro. In high-throughput scenarios this overhead may be measurable.

---

## 19. Bus Factor / Sustainability

- **Maintainers:** 1 (solo project, oxur organization — the same maintainer behind `dagrs`-adjacent tooling)
- **Commit cadence:** 20 commits in ~2 days (2026-02-07 to 2026-02-08), then no activity. The project was essentially completed as a sprint.
- **Issue ratio:** 0 open, 0 closed. No community engagement yet.
- **Downloads:** 37 total on crates.io (2.5 months post-publish).
- **Last release:** 0.2.0 on 2026-02-08. No release since.
- **Bus factor:** 1. If the maintainer stops, the project stalls.
- **v2 roadmap:** 11 detailed phase documents (Phases 1–11) in `docs/dev/`, totalling ~17,918 lines. This implies the maintainer has a detailed plan but has not yet implemented Phases 6–11.

---

## 20. Final Scorecard vs Nebula

| Axis | Treadle approach | Nebula approach | Who's deeper / simpler / more correct | Borrow? |
|------|-----------------|-----------------|---------------------------------------|---------|
| A1 Workspace | Single crate, ~5.8K LOC, Edition 2021, `rust-version = "1.75"` | 26 crates, layered, Edition 2024, stable 1.95.0 | Nebula deeper (platform scale); Treadle simpler (library) — different goals | no — different goals |
| A2 DAG | `petgraph::DiGraph`, cycle check at build, topo-sort cached, no port typing, no compile-time type checks between stages | TypeDAG L1–L4 (static generics → TypeId → predicates → petgraph) | Nebula deeper (typed ports); Treadle simpler (one library) — different decomposition | refine: cycle detection and topo-sort approach is convergent |
| A3 Action | Open `Stage` trait, 1 associated item (name), type-erased `&dyn WorkItem` input, `StageOutcome` enum output, `async_trait` macro, no derive macros, no versioning | 5 sealed action kinds, assoc `Input/Output/Error`, derive macros, versioning via type identity | Nebula deeper (type safety, 5 kinds, sealed, GATs) | no — Nebula's already better |
| A4 Credential | None (confirmed: 0 grep matches in src/) | State/Material split, LiveCredential, blue-green refresh, OAuth2Protocol | Nebula deeper | no — different goals (embedded library) |
| A5 Resource | None (confirmed: 0 grep matches in src/) | 4 scope levels, ReloadOutcome, generation tracking, on_credential_refresh | Nebula deeper | no — different goals |
| A6 Resilience | Manual retry (StageOutcome::Retry, currently paused), no CB/bulkhead/hedging/timeout in v1; v2 adds RetryBudget | nebula-resilience crate: retry/CB/bulkhead/timeout/hedging + ErrorClassifier | Nebula deeper | maybe — v2 RetryBudget + QualityGate pattern is novel; worth watching |
| A7 Expression | None (confirmed: 0 grep matches) | 60+ funcs, type inference, sandboxed eval, $nodes.foo.result.email | Nebula deeper | no — different goals |
| A8 Storage | rusqlite SQLite (feature-gated) + MemoryStateStore, state as JSON blobs, schema v1, no migration runner | sqlx + PgPool, Pg*Repo, SQL migrations, RLS | Different decomposition — Nebula PostgreSQL for platform; Treadle SQLite for embedded | no — different goals |
| A9 Persistence | Checkpoint-at-stage-completion, direct state overwrite in SQLite, no append-only log, no replay | Frontier-based + checkpoint + append-only execution log + replay | Nebula deeper (durability semantics); Treadle simpler (sufficient for single-process) | no — different goals |
| A10 Concurrency | tokio runtime, single-task sequential execution within `advance()`, fan-out is sequential, no parallel scheduling | tokio + frontier scheduler + work-stealing, `!Send` isolation | Nebula deeper | no — sequential is correct for Treadle's scope |
| A11 Plugin BUILD | None (confirmed: 0 grep matches) | WASM + plugin-v2 spec + Plugin Fund | Nebula deeper | no — different goals |
| A11 Plugin EXEC | None — stages are compiled-in Rust code | WASM sandbox + capability security | Nebula deeper | no — different goals |
| A12 Trigger | None (confirmed: 0 grep matches). Execution is caller-driven only. The event system is for observation (11-variant WorkflowEvent broadcast), not ingestion. | TriggerAction with Input=Config / Output=Event; Source → Event 2-stage normalization | Nebula deeper (trigger model); Treadle omits this intentionally | no — different goals |
| A13 Deployment | Single embedded library, one binary mode | 3 modes from one binary (desktop/serve/cloud) | Different decomposition — not comparable | no — different goals |
| A21 AI/LLM | No built-in LLM; v2 design explicitly targets LLM pipelines via QualityGate + retry-with-feedback pattern (design doc only, not implemented) | No first-class LLM yet; bet: generic actions + LLM plugin | Convergent bet; Treadle's QualityGate/RetryBudget approach for LLM quality evaluation is novel and more specific | yes — borrow QualityGate / retry-with-feedback pattern as Nebula approaches LLM pipeline use cases |

---

## Summary Assessment

Treadle is a well-scoped, well-documented single-crate library for a specific problem: persistent DAG pipelines with human review gates in single-process Rust applications. It is not a competitor to Nebula — it occupies a completely different level of the stack (embedded library vs. workflow platform). The project is authored by an experienced Rust developer who has produced detailed implementation plans (11 phases, ~18K lines of design docs) but has only completed Phases 1–5 of the v2 roadmap.

The two genuinely novel ideas Treadle has that Nebula lacks:

1. **Human review gate as a first-class engine outcome** (`StageOutcome::NeedsReview` / `approve_review` / `reject_review`) — Nebula has no equivalent.
2. **Quality gate + retry-with-feedback loop** (v2 design: `QualityGate` trait + `RetryBudget` + structured `QualityFeedback` threading between attempts) — specifically designed for AI agent pipelines where LLM output may need iterative refinement. Nebula has no equivalent and has explicitly noted AI pipelines as a future concern.

Both are borrowable ideas for Nebula, particularly for AI-pipeline features.
