# aqueducts-utils — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/vigimite/aqueducts
- **Stars / forks:** small project (community Discord opened April 2025; no star count visible from clone)
- **Last activity:** commit `8ade0f5` (chore: update rand crate) — recent, actively maintained
- **Latest release:** `v0.11.1` (all crates tagged simultaneously)
- **License:** Apache-2.0 (`Cargo.toml` workspace line 22)
- **Governance:** solo maintainer (`vigimite@protonmail.com`; single author in workspace Cargo.toml line 14)
- **Edition:** 2021 (`Cargo.toml` workspace line 15)
- **Note on crate name:** The GitHub repo is `aqueducts`; the sub-crate under study is `aqueducts-utils` as assigned. In the actual workspace there is no crate named `aqueducts-utils` — the closest match is `aqueducts` (meta umbrella) and `aqueducts-core`. This report treats the whole workspace.

---

## 1. Concept positioning [A1, A13, A20]

**Author's own README sentence:** "Aqueducts is a framework to write and execute ETL data pipelines declaratively." (README.md line 4)

**Mine (after reading code):** Aqueducts is a thin orchestration shell over Apache DataFusion: it deserializes a declarative YAML/JSON/TOML pipeline spec, registers named sources into a DataFusion `SessionContext`, runs SQL stages (with optional parallelism), and writes the result to a destination — locally or via an axum/WebSocket remote executor.

**Comparison with Nebula:** Nebula is a general-purpose workflow orchestration engine modeled after n8n + Temporal + Airflow, with typed actions, credential lifecycle, resource management, multi-tenancy, and plugin architecture. Aqueducts is scoped entirely to data transformation (ETL): it has no workflow control flow (conditionals, loops, retries), no credentials layer, no triggers, no plugin system, and no AI surface. The design space barely overlaps — Aqueducts is closer to dbt-on-Rust than to Nebula.

---

## 2. Workspace structure [A1]

8 workspace members (`Cargo.toml` lines 2–11):

| Path | Crate | Layer |
|------|-------|-------|
| `aqueducts/schemas` | `aqueducts-schemas` | L0 types |
| `aqueducts/core` | `aqueducts-core` | L1 engine |
| `aqueducts/delta` | `aqueducts-delta` | L2 provider |
| `aqueducts/odbc` | `aqueducts-odbc` | L2 provider |
| `aqueducts/meta` | `aqueducts` | L3 umbrella |
| `aqueducts-cli` | `aqueducts-cli` | L4 binary |
| `aqueducts-executor` | `aqueducts-executor` | L4 binary |
| `tools/schema-generator` | _(internal)_ | tooling |

The schemas crate has zero internal deps and is the shared type layer; providers depend on schemas; core depends on schemas and optionally providers; the meta umbrella re-exports everything through feature flags; binaries depend on the umbrella.

Feature flags (`aqueducts-core/Cargo.toml` lines 14–24): `s3`, `gcs`, `azure`, `odbc`, `delta`, `json`, `yaml` (default), `toml`, `custom_udfs`. This is the entire extension mechanism — no runtime plugin system exists.

**Nebula comparison:** Nebula has 26 crates with explicit boundary enforcement (nebula-error / nebula-credential / nebula-resource / nebula-action / nebula-engine / nebula-tenant, etc.) and Edition 2024. Aqueducts has 8 crates, Edition 2021, and layer separation is shallower. Nebula deeper in boundary enforcement; Aqueducts simpler and appropriate for its narrower scope.

---

## 3. Core abstractions [A3, A17] — DEEP

### A3.1 — Trait shape

There is **no `Action`, `Node`, or `Activity` trait** in this codebase. A scan of all 47 `.rs` files finds no such trait. The only substantive public trait is `ProgressTracker` (`aqueducts-core/src/progress_tracker.rs:69`):

```rust
pub trait ProgressTracker: Send + Sync {
    fn on_progress(&self, event: ProgressEvent);
    fn on_output(
        &self,
        stage_name: &str,
        output_type: OutputType,
        schema: &DFSchema,
        batches: &[RecordBatch],
    );
}
```

This is open (any external crate can implement it), not sealed, and uses no associated types. It is `dyn`-compatible and held behind `Arc<dyn ProgressTracker>` in `run_pipeline` (`aqueducts-core/src/lib.rs:71`). No GATs, no HRTBs, no typestate.

Pipeline steps are plain **data structs**, not trait implementations:
- `Stage` — `aqueducts-schemas/src/stages.rs:35` — `{ name: String, query: String, show: Option<usize>, explain: bool, explain_analyze: bool, print_schema: bool }`
- `Source` — enum, `aqueducts-schemas/src/sources.rs:38` — variants: `InMemory`, `File`, `Directory`, `Odbc`, `Delta`
- `Destination` — enum, `aqueducts-schemas/src/destinations.rs:36` — variants: `InMemory`, `File`, `Odbc`, `Delta`
- `Aqueduct` — top-level pipeline struct, `aqueducts-schemas/src/lib.rs:86` — `{ version: String, sources: Vec<Source>, stages: Vec<Vec<Stage>>, destination: Option<Destination> }`

### A3.2 — I/O shape

Input is always a named SQL table (the result of deserialization into a `Source` variant, registered into DataFusion's `SessionContext`). Output is always another named SQL table produced by the stage's `query` string. Both are type-erased at the SQL level — DataFusion handles Arrow type inference. No generics for Input/Output; no streaming output from stages; side effects are limited to writes at the `Destination` step.

### A3.3 — Versioning

A `version` field exists on `Aqueduct` (defaults to `"v2"`, `aqueducts-schemas/src/lib.rs:89`). There is no structured v1→v2 migration code; the `version` field is stored but not acted on programmatically (no match arm on it found in any `.rs` file). Steps are not individually versioned. No `#[deprecated]`.

### A3.4 — Lifecycle hooks

`run_pipeline` is the single entry point (`aqueducts-core/src/lib.rs:68`). Execution model:

1. Register destination.
2. Register all sources in parallel (`tokio::spawn` per source, `aqueducts-core/src/lib.rs:93–110`).
3. For each sequential stage row, run parallel sub-stages (`tokio::spawn` per sub-stage, lines 125–173).
4. Write destination from the last stage's result.

No per-stage pre/post/cleanup hooks. No idempotency key. No cancellation point at the stage level (cancellation is at the executor level via `CancellationToken`, `aqueducts-executor/src/executor/manager.rs:44`).

### A3.5 — Resource and credential deps

Stages do not declare resource dependencies. All SQL context is implicit in the shared `Arc<SessionContext>`. Cloud storage credentials are injected via `storage_config: HashMap<String, String>` per source/destination (e.g., `aqueducts-schemas/src/sources.rs:98`). No compile-time resource declaration; no constructor injection.

### A3.6 — Retry/resilience attachment

No retry, circuit breaker, or timeout per stage. The only resilience surface is the executor's memory limit via DataFusion's runtime environment. No per-pipeline or per-stage retry policy.

### A3.7 — Authoring DX

Pipeline definition is YAML/JSON/TOML deserialization, not Rust code. A minimal hello-world stage in YAML is 5 lines. The Rust API uses `bon` builder macros for struct construction. There is no derive macro for creating new stage types — the schema is fixed.

### A3.8 — Metadata

Display name exists as `name: String` on `Stage`. No icon, no category, no i18n. Names are runtime strings.

### A3.9 — vs Nebula

Nebula has 5 sealed action kinds (Process/Supply/Trigger/Event/Schedule) with associated types `Input`/`Output`/`Error`, derive macros, versioning, and lifecycle hooks. Aqueducts has **one kind of step** (SQL Stage) modeled as a plain data struct, with zero trait hierarchy. The gap is categorical: Nebula supports arbitrary Rust logic per action; Aqueducts constrains all logic to SQL inside a fixed DataFusion context.

---

## 4. DAG / execution graph [A2, A9, A10]

### Graph model

The pipeline is a **linear sequence of stage rows**, where each row may contain multiple parallel sub-stages. This is a simplified 2D grid, not a general DAG. There is no petgraph or graph library. The structure is `Vec<Vec<Stage>>` where the outer Vec is sequential and the inner Vec is parallel.

Stage-level data dependency is implicit: each stage can reference any previously registered table by name in its SQL query. There is no static compile-time edge declaration. Dependency is inferred at runtime by DataFusion's SQL planner.

A TTL (time-to-live) mechanism garbage-collects stage results from the `SessionContext` after no further stages reference them (`aqueducts-core/src/lib.rs:222–264`): it scans forward stages using a regex match for the stage name in downstream queries.

### Compile-time vs runtime checks

No compile-time checks on graph topology. Schema validation happens at DataFusion planning time (runtime). Port typing does not exist — data flows as Arrow `RecordBatch` through the SQL context.

### Scheduler model

tokio task spawning per source and per parallel stage row. No work-stealing, no frontier-based scheduling. The executor enforces single-job concurrency via a `Semaphore(1)` (`aqueducts-executor/src/executor/manager.rs:14`).

**Nebula comparison:** Nebula's TypeDAG (L1 static generics → L2 TypeId → L3 predicates → L4 petgraph) is fundamentally more powerful. Aqueducts' 2D grid is adequate for linear ETL but cannot express non-linear DAGs, conditional branching, or fan-out/fan-in. Different decomposition, neither dominates for their respective use cases.

---

## 5. Persistence and recovery [A8, A9]

No persistence layer. There is no database dependency (`sqlx`, `diesel`, or any query builder) in the workspace. Pipeline state is held in memory in DataFusion's `SessionContext` for the duration of one run. If the executor process crashes, the run is lost and must be restarted manually.

There is no checkpoint, no append-only execution log, no state reconstruction. The `version: "v2"` on `Aqueduct` is a schema migration marker, not a persistence mechanism.

**Nebula comparison:** Nebula has `sqlx + PgPool`, `Pg*Repo` per aggregate, append-only execution log, and frontier-based checkpoint recovery. Aqueducts has none of this — stateless by design (ETL run = ephemeral compute).

---

## 6. Credentials / secrets [A4] — DEEP

**A4.1 — Existence:** There is no credential layer. Cloud storage credentials are passed as plain `HashMap<String, String>` in the pipeline config file under `storage_config` (e.g., `aqueducts-schemas/src/sources.rs:98`). The executor API key is a plain string compared in middleware (`aqueducts-executor/src/api/auth.rs:26`).

**Grep evidence:**
- `grep -r "credential" --include="*.rs"` returns only files in `aqueducts/core/src/store/` — these are the `S3Provider`, `GcsProvider`, `AzureProvider` implementations where AWS credentials are passed as config keys like `aws_access_key_id` and `aws_secret_access_key` (string fields in a HashMap).
- `grep -r "secret" --include="*.rs"` returns `azure.rs` and `s3.rs` — both reference `aws_secret_access_key` and `azure_client_secret` as string keys in a HashMap.
- No `secrecy` crate, no `zeroize`, no `Secret<T>` wrapper anywhere in the workspace.

**A4.2 — Storage:** No at-rest encryption. Credentials are stored in the YAML/JSON pipeline definition file, or passed via env vars (`S3Provider::create_store` calls `AmazonS3Builder::from_env()` first — `aqueducts-core/src/store/s3.rs:60`). No vault integration.

**A4.3 — In-memory protection:** None. `HashMap<String, String>` holds credentials as plain `String`. No `Zeroize`, no `secrecy::Secret<T>`.

**A4.4 — Lifecycle:** No CRUD, no revocation, no refresh. Credentials are loaded once at pipeline start and used for the duration of the run.

**A4.5 — OAuth2/OIDC:** None. AWS session tokens are supported as strings (`aws_session_token` key in `s3.rs:75`), but there is no OAuth2 flow.

**A4.6 — Composition:** One storage_config per source/destination. No credential sharing or delegation.

**A4.7 — Scope:** Credentials scope to a single pipeline invocation. No cross-execution or workspace-level sharing.

**A4.8 — Type safety:** None. Credentials are untyped strings.

**A4.9 — vs Nebula:** Nebula's State/Material split, `LiveCredential` with `watch()`, blue-green refresh, `OAuth2Protocol` blanket adapter, and `DynAdapter` type erasure have no equivalent in Aqueducts. Aqueducts has no credential layer at all — just string key-value config.

---

## 7. Resource management [A5] — DEEP

**A5.1 — Existence:** No separate resource abstraction. The `Arc<SessionContext>` is the only shared runtime object. DB connections (ODBC) are created per-pipeline-invocation within the provider crate.

**Grep evidence:** `grep -r "resource\|Resource" --include="*.rs"` finds only narrative comments ("cloud storage resources") and `object_store` usage — no `Resource` trait, no pool abstraction, no lifecycle management.

**A5.2 — Scoping:** No scope levels. The `SessionContext` lives for the duration of `run_pipeline`.

**A5.3 — Lifecycle hooks:** No `init`, `shutdown`, or `health-check`. DataFusion session creation is the user's responsibility before calling `run_pipeline`.

**A5.4 — Reload:** No hot-reload, no blue-green, no `ReloadOutcome` enum.

**A5.5 — Sharing:** The `Arc<SessionContext>` is shared across all parallel source registrations and stage executions within a single pipeline run.

**A5.6 — Credential deps:** Resources do not declare credential dependencies. Credentials flow in via `storage_config` HashMap at pipeline config load time.

**A5.7 — Backpressure:** DataFusion memory limits via `--max-memory` flag (GB, `aqueducts-executor/src/config.rs`). No acquire timeout, no bounded queue for resources.

**A5.8 — vs Nebula:** Nebula has 4 scope levels (Global/Workflow/Execution/Action), `ReloadOutcome`, generation tracking, `on_credential_refresh` per-resource hook. Aqueducts has none of these — resource lifecycle is fully delegated to the DataFusion runtime.

---

## 8. Resilience [A6, A18]

No retry, circuit breaker, bulkhead, timeout, or hedging. The executor enforces memory limits via DataFusion's runtime environment. Stage cancellation is supported via `tokio_util::sync::CancellationToken` at the executor level (`aqueducts-executor/src/executor/manager.rs:44`), but this is cancellation of the entire pipeline run, not per-stage.

Error types use `thiserror = "2"` and `miette = "7.6"` (added in commit `93dc151` "Integrate miette diagnostics"). `miette` adds rich diagnostic context (code snippets, suggestions) but does not provide an `ErrorClass` abstraction or transient/permanent classification.

**Nebula comparison:** Nebula has a dedicated `nebula-resilience` crate with retry/CB/bulkhead/timeout/hedging and a unified `ErrorClassifier`. Aqueducts has no resilience primitives; this is a direct gap.

---

## 9. Expression / data routing [A7]

No expression engine or DSL for data routing. All data transformation is SQL (DataFusion SQL dialect). Template parameter substitution uses `${variable}` syntax in the pipeline definition file, handled by a simple regex replacer in `aqueducts-core/src/templating.rs`. Custom UDFs are possible through the `custom_udfs` feature flag (DataFusion JSON functions), but this is not a general expression engine — it is SQL + registered functions.

**Nebula comparison:** Nebula has a 60+ function expression engine with type inference, sandboxed eval, and `$nodes.foo.result.email` syntax for inter-node data routing. Aqueducts uses SQL for all transformations. Different decomposition — SQL is more expressive for tabular data but not general-purpose for workflow data routing.

---

## 10. Plugin / extension system [A11] — BUILD + EXEC

### 10.A — Plugin BUILD process (A11.1–A11.4)

**A11.1 — Format:** No plugin format. Extensions are Rust crates compiled statically into the binary.

**A11.2 — Toolchain:** Cargo workspace feature flags are the only mechanism. To add a new source type, a developer must fork the repo and add a new crate + feature flag. No plugin SDK, no scaffolding tool, no cross-compilation target. The schema generator tool (`tools/schema-generator`) generates JSON Schema from the `schemars` derives — it is not a plugin build tool.

**A11.3 — Manifest content:** No plugin manifest. There is no capability declaration, no permission grant system, no plugin dependency declaration. Everything is `Cargo.toml` features.

**A11.4 — Registry/discovery:** No registry. Feature-gated static linking only.

**Grep evidence:** `grep -r "\bplugin\b" --include="*.rs"` returned zero results across all 47 `.rs` files.

### 10.B — Plugin EXECUTION sandbox (A11.5–A11.9)

**A11.5 — Sandbox type:** Not applicable. There is no runtime sandbox. Provider crates run in-process with zero isolation from the host.

**A11.6 — Trust boundary:** No trust model. All code is statically linked and fully trusted.

**A11.7 — Host↔plugin calls:** Not applicable. Standard Rust function calls within the same binary.

**A11.8 — Lifecycle:** Static linkage; no hot-reload, no crash recovery per provider.

**A11.9 — vs Nebula:** Nebula targets WASM sandbox with capability-based security and a Plugin Fund commercial model. Aqueducts has no plugin runtime at all — static Cargo features are a compile-time composition mechanism, not a plugin system. An end-user cannot add a new source type without recompiling from source.

---

## 11. Trigger / event model [A12]

No trigger or event model exists. Aqueducts pipelines are single-shot, on-demand executions. There is no webhook receiver, no cron scheduler, no message queue integration, no FS watcher, no DB change listener, and no internal event bus.

**Grep evidence (negative):**
- `grep -r "webhook\|cron\|schedule\|trigger\|event.bus\|kafka\|rabbitmq\|nats" --include="*.rs"` returned zero results for all of these terms in any pipeline-related file. The only event-related file is `aqueducts-schemas/src/progress.rs` which defines `ProgressEvent` — a simple status notification enum for in-process progress tracking, not an external event system.

The executor accepts one pipeline definition via HTTP POST and executes it; the client connects via WebSocket to stream progress events back. This is request/response, not event-driven orchestration.

**Nebula comparison:** Nebula has `TriggerAction` with `Input = Config` (registration) and `Output = Event` (typed payload), a `Source` trait normalizing HTTP/Kafka/cron, and a 2-stage dispatch model. Aqueducts has none of this — it is a computation library, not an orchestration platform.

---

## 12. Multi-tenancy [A14]

No multi-tenancy. There is no RBAC, no SSO, no SCIM, no schema isolation. The executor has a single API key (`aqueducts-executor/src/api/auth.rs`) — all clients with the key have equal access. No user or tenant concept exists in the data model.

---

## 13. Observability [A15]

Uses `tracing = "0.1"` throughout. All significant execution points are instrumented with `#[instrument]` or manual `debug!`/`info!`/`warn!`/`error!` calls. The `LoggingProgressTracker` in `aqueducts-core/src/progress_tracker.rs` provides structured log output for pipeline lifecycle events.

No OpenTelemetry integration. No metrics exporter (no `prometheus` or `opentelemetry-prometheus` in `Cargo.toml`). No per-action latency histogram. Tracing output is configured by the binary consumer via `tracing-subscriber` with env-filter.

**Nebula comparison:** Nebula uses OpenTelemetry with structured tracing per execution (one trace = one workflow run) and per-action latency/count/error metrics. Aqueducts uses only `tracing` logging; no OTel, no metrics exporters.

---

## 14. API surface [A16]

**Programmatic API:** `run_pipeline(ctx: Arc<SessionContext>, aqueduct: Aqueduct, progress_tracker: Option<Arc<dyn ProgressTracker>>) -> Result<Arc<SessionContext>>` — single function. `Aqueduct::from_file` and `Aqueduct::from_str` for loading.

**Network API:** The executor exposes an axum HTTP server. API key auth via `X-API-Key` header. WebSocket/SSE for streaming progress events back to the CLI. No OpenAPI spec generated. No versioning on the HTTP API.

**CLI:** `aqueducts run --file <path> --param key=value [--executor <url>] [--api-key <key>]`

---

## 15. Testing infrastructure [A19]

Integration tests in `aqueducts/core/tests/integration.rs`, `aqueducts/delta/tests/integration.rs`, `aqueducts/schemas/tests/integration.rs`. Unit tests inline in `aqueducts-executor/src/executor/manager.rs` (4 tests covering queue submission, sequential execution, progress streaming, and cancellation). `tracing-test = "0.2"` is in dev-deps for tracing output in tests. No `insta`, no `wiremock`, no `mockall` in the workspace.

No public testing utilities crate. No contract tests for extending sources or destinations.

**Nebula comparison:** Nebula has a dedicated `nebula-testing` crate with contract tests for resource implementors, `insta` + `wiremock` + `mockall`. Aqueducts has basic integration tests only.

---

## 16. AI / LLM integration [A21] — DEEP

**A21.1 — Existence:** None. No built-in, no separate crate, no community plugin.

**Grep evidence (mandatory):**
- `grep -r "llm\|openai\|anthropic\|LLM\|language.model\|completion\|embedding" --include="*.rs"` returned **zero results** across all 47 `.rs` files.
- `grep -r "wasm\|wasmtime\|wasmer" --include="*.rs"` returned **zero results**.
- `grep -r "\bplugin\b" --include="*.rs"` returned **zero results**.

**A21.2 — Provider abstraction:** None.
**A21.3 — Prompt management:** None.
**A21.4 — Structured output:** None.
**A21.5 — Tool calling:** None.
**A21.6 — Streaming:** None (the WebSocket streaming exists only for pipeline progress events, not LLM token streaming).
**A21.7 — Multi-agent:** None.
**A21.8 — RAG/vector:** None.
**A21.9 — Memory/context:** None.
**A21.10 — Cost/tokens:** None.
**A21.11 — Observability:** None.
**A21.12 — Safety:** None.
**A21.13 — vs Nebula + Surge:** Nebula also has no first-class LLM abstraction (bet: AI = generic actions + plugin LLM client). Surge is the separate agent orchestrator on ACP. Aqueducts has no AI surface at all and no roadmap item for it. Convergent (both frameworks have no built-in LLM), but for different strategic reasons: Nebula defers to plugin model; Aqueducts simply targets ETL not AI.

---

## 17. Notable design decisions

**D1 — DataFusion as the only execution model.** All transformation logic is SQL. This eliminates the need for a custom expression engine, type system, or operator graph — DataFusion provides all of that. The tradeoff: pipelines cannot execute arbitrary Rust logic (only what DataFusion SQL supports), and adding new operators requires upstream DataFusion contributions or UDF registration.

**D2 — Vec\<Vec\<Stage\>\> as the concurrency model.** Nested arrays are a simple serialization-friendly representation of "sequential rows of parallel stages." It avoids a full DAG library but limits expressibility to a linear pipeline with optional width. No conditional branching, no fan-out, no join synchronization beyond what DataFusion's SQL engine handles internally.

**D3 — Static feature flags as the entire plugin model.** Provider crates (`aqueducts-delta`, `aqueducts-odbc`) are Cargo workspace members enabled at compile time. This yields zero runtime overhead and eliminates versioning/ABI concerns, but it means end-users cannot extend the framework without forking and recompiling. The roadmap acknowledges this will not scale once a web server / catalog is added.

**D4 — Credentials as plain HashMap\<String, String\>.** Cloud credentials are user-supplied config keys, delegated entirely to `object_store` and `delta-rs` builders. This keeps the framework small but means secrets appear in plain text in pipeline definition files. There is no Secret<T> wrapping, no zeroize, and no lifecycle management.

**D5 — Single-concurrency executor by design.** The executor enforces `Semaphore(1)` so that only one pipeline runs at a time. The ARCHITECTURE.md explicitly states: "Each execution is exclusive on the executor which means that only one Aqueduct can be run at a time." This is a correct choice for memory-intensive DataFusion queries but limits throughput.

**D6 — miette for error diagnostics.** Adding `miette` (commit `93dc151`) provides rich source-context errors with code snippets and suggestions. This is an unusual but effective choice for a framework whose users write pipeline config files — parse errors in YAML/SQL can now show the exact failing span.

---

## 18. Known limitations / pain points

From GitHub issues (7 total: 5 closed, 2 open) — small issue tracker:

- **Issue #9 — Feature: python bindings** (open, 2024-07-28): No Python API; all usage must be through the CLI or the Rust library. URL: https://github.com/vigimite/aqueducts/issues/9
- **Issue #8 — Add more tests** (open, labeled `good first issue`, 2024-06-02): Test coverage is acknowledged as insufficient. URL: https://github.com/vigimite/aqueducts/issues/8
- **Issue #36 — MySQL/MariaDB support** (closed): ODBC is the workaround; no native MySQL driver.
- **Issue #37 — Docker setup** (closed): Docker Compose setup added.
- **Issue #63 — Prebuilt binaries** (closed): cargo-dist added for binary releases.

From roadmap (README.md lines 265–275): web server orchestration, Apache Iceberg, data catalog — all unimplemented.

From CHANGELOG.md and git log: breaking changes occurred at v0.10 (rename of `file_type` to `format` in source/dest configs) and DataFusion major version bumps (v48 → v51 in recent commits) causing dependency churn.

Key architectural limitation: **no workflow persistence**. A failed pipeline run leaves no checkpoint; re-execution reruns the entire pipeline from scratch. For large ETL jobs this can be expensive.

---

## 19. Bus factor / sustainability

- **Maintainer count:** 1 (vigimite)
- **Commit cadence:** Active — multiple commits in last 2 months (dependency updates, bug fixes)
- **Issue ratio:** 7 total issues (5 closed, 2 open) — very low volume; community is small
- **Last release:** v0.11.1 (recent)
- **Discord:** opened April 2025 — community building just started
- **Risk:** Single maintainer; no community contributors visible in git log

---

## 20. Final scorecard vs Nebula

| Axis | Their approach | Nebula approach | Who's deeper / simpler / more correct | Borrow? |
|------|---------------|-----------------|---------------------------------------|---------|
| A1 Workspace | 8 crates, Edition 2021, Cargo feature flags as extension mechanism | 26 crates layered, Edition 2024, strong boundary enforcement | Nebula deeper on boundary isolation; Aqueducts simpler and proportionate to scope | no — Nebula's is already better for Nebula's problem |
| A2 DAG | Vec\<Vec\<Stage\>\> 2D grid — sequential rows of parallel SQL stages; no graph library; implicit SQL-level deps | TypeDAG L1-L4 static generics → TypeId → predicates → petgraph | Different decomposition — SQL implicit deps vs typed explicit edges; Nebula richer for general workflow; Aqueducts adequate for ETL | no — different goals |
| A3 Action | No action trait — pipeline steps are plain `Stage { name, query }` data structs; one kind of step (SQL); no lifecycle hooks | 5 action kinds, sealed traits, assoc Input/Output/Error, derive macros | Competitor simpler by design (ETL only); Nebula richer (general workflows) | no — different goals |
| A11 Plugin BUILD | Cargo feature flags; provider crates statically linked; no plugin SDK, no manifest, no registry | WASM + plugin-v2 spec + Plugin Fund; capability-based security | Nebula deeper; Aqueducts has no plugin system — static linking is compile-time composition | no — Nebula's already better |
| A11 Plugin EXEC | In-process static linkage; zero sandbox; no isolation | WASM sandbox (wasmtime), capability grants, crash recovery | Nebula deeper — correctness and security isolation | no — Nebula's already better |
| A18 Errors | `thiserror = "2"` + `miette = "7.6"` per-crate enum errors; no ErrorClass/transient distinction; miette adds diagnostic context (code spans) | nebula-error crate + ErrorClass enum (transient/permanent/cancelled) + ErrorClassifier in resilience | Different: `miette` diagnostic richness is worth noting; Nebula has structured classification Aqueducts lacks | refine — adopt `miette` diagnostic integration for user-facing parse/config errors |
| A21 AI/LLM | None — zero LLM surface; no roadmap item for AI | None first-class — bet on generic actions + plugin LLM client (Surge for agent orchestration) | Convergent absence; neither has built-in LLM — different strategic reasons | no — different goals |
