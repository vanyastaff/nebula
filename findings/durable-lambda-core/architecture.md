# durable-lambda-core — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/pgdad/durable-rust
- **Stars:** 2 (as of 2026-04-26)
- **Forks:** 0
- **License:** MIT OR Apache-2.0
- **Last commit:** 2026-04-01 (tag v1.2.0)
- **Created:** 2026-03-15 — project is ~6 weeks old
- **Governance:** Solo author (pgdad). No issue tracker activity (0 open issues, 0 closed issues found via `gh issue list --repo pgdad/durable-rust --state all --limit 30` — returned empty array).
- **Published crates:** 6 crates published to crates.io at v1.2.0 (per commit `cfeb957`: "complete v1.2 Crates.io Publishing milestone").

---

## 1. Concept positioning [A1, A13, A20]

**Author's own description (README.md line 1):**
> "An idiomatic Rust SDK for AWS Lambda Durable Execution, providing full feature parity with the official AWS Python Durable Lambda SDK."

**Own assessment after reading code:**
A single-purpose Rust SDK that wraps the AWS Lambda Durable Execution managed service — a feature introduced by AWS that lets Lambda functions pause, checkpoint, and resume across cold starts. The library does not implement its own orchestration engine; durability is fully delegated to AWS. The SDK's job is to provide an ergonomic Rust API over two AWS APIs (`checkpoint_durable_execution` and `get_durable_execution_state`) and to replicate the replay semantics of the official Python SDK.

**Comparison with Nebula:**
Nebula is a full workflow orchestration engine with its own DAG scheduler, credential subsystem, resource lifecycle, resilience layer, expression engine, plugin model, and multi-tenancy. durable-lambda-core is none of those things: it is a thin, Lambda-specific adapter for a managed AWS service. The projects do not compete architecturally. The only overlap is that both implement a form of durable step execution — but Nebula owns the durability infrastructure while durable-lambda-core outsources it entirely to AWS Lambda Durable Execution.

---

## 2. Workspace structure [A1]

Six library crates plus examples and tests in a single Cargo workspace (`Cargo.toml`, resolver = "2"):

```
crates/
  durable-lambda-core       # Replay engine, all 8 operations, DurableBackend trait, error types
  durable-lambda-macro      # #[durable_execution] proc-macro
  durable-lambda-trait      # DurableHandler trait + TraitContext wrapper
  durable-lambda-closure    # ClosureContext wrapper (recommended default)
  durable-lambda-builder    # BuilderContext wrapper + DurableHandlerBuilder
  durable-lambda-testing    # MockDurableContext + assertion helpers
examples/
  closure-style / macro-style / trait-style / builder-style
tests/
  e2e / parity
compliance/
  python / rust / tests/fixtures  (Python–Rust parity suite)
```

**Dependency graph** (from README.md "Crate Dependency Graph" section):
```
durable-lambda-closure ─┐
durable-lambda-macro  ──┤
durable-lambda-trait  ──┼── durable-lambda-core
durable-lambda-builder ─┤
durable-lambda-testing ─┘
```

All approach crates depend solely on `durable-lambda-core`. No circular or cross-approach dependencies. The workspace uses `[workspace.dependencies]` for all version pins — individual crate `Cargo.toml` files have no `[dependencies]` section of their own (enforced by CLAUDE.md rule).

**Feature flags:** None observed. No optional feature gates in any `Cargo.toml`.

**Comparison with Nebula:** Nebula has 26 crates across 6 conceptual layers (infra → domain → integration → API → tenancy → tooling). durable-lambda-core has 6 crates in a flat fan-out pattern: one core + four ergonomic wrappers + one testing utility. The design reflects the project's narrow scope — there is no need for separate credential, resource, resilience, expression, or DAG crates because those concerns either do not exist or belong to AWS.

---

## 3. Core abstractions [A3, A17] — DEEP

### A3.1 Trait shape

There is no "action" or "node" trait in the workflow sense. The project's unit of work abstraction is the **`DurableContextOps` trait** (`crates/durable-lambda-core/src/ops_trait.rs:50`):

```rust
pub trait DurableContextOps {
    fn step<T, E, F, Fut>(&mut self, name: &str, f: F)
        -> impl Future<Output = Result<Result<T, E>, DurableError>> + Send
    where
        T: Serialize + DeserializeOwned + Send + 'static,
        E: Serialize + DeserializeOwned + Send + 'static,
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<T, E>> + Send + 'static;
    // ... (step_with_options, wait, create_callback, invoke, parallel, child_context, map, etc.)
}
```

**Sealed/open:** The trait is **open** — any struct can implement `DurableContextOps`. However, the doc comment at `ops_trait.rs:7–9` explicitly states: "This trait is designed for **static dispatch only** — never use it as `dyn DurableContextOps`." It is not object-safe in practice due to `-> impl Future<...>` return positions (RPITIT — return position impl Trait in trait, stabilized in Rust 1.75).

**Associated types count:** Zero associated types. All generic parameters are on individual methods, not on the trait itself. No GATs, no HRTBs on the trait definition.

**Typestate:** None. `DurableContext` does not use typestate — it is a single concrete struct that delegates to the replay engine.

**Default methods:** None defined on `DurableContextOps`. The `DurableBackend` trait (`backend.rs:38`) has one default method (`batch_checkpoint` at line 82) that delegates to `checkpoint`.

### A3.2 I/O shape

Step inputs and outputs flow through generic bounds: `T: Serialize + DeserializeOwned + Send + 'static` and `E: Serialize + DeserializeOwned + Send + 'static` (ops_trait.rs:65–68). Internally everything is serialized to/from `serde_json::Value` at checkpoint time. There is no streaming output — all step results are fully materialized before checkpointing. Side effects are modeled by placing them inside the step closure.

The double-Result pattern is a deliberate design: `Result<Result<T, E>, DurableError>` where the outer layer is SDK infrastructure (replay mismatch, checkpoint failure, AWS error) and the inner is the user's business result (both arms checkpointed identically).

### A3.3 Versioning

No versioning. Operations are referenced by string name only — a positional, counter-based identity (`blake2b("{counter}")` at `operation_id.rs:84–91`). The name is display-only; the operation ID is deterministic from position. **Reordering operations between deployments breaks in-flight replay** (README.md "Safety checklist"). No `#[deprecated]`, no migration support, no v1/v2 distinction.

### A3.4 Lifecycle hooks

No pre/post/cleanup/on-failure hooks. Each operation has exactly one `execute` phase (the closure). Cancellation is modeled by propagating `DurableError::WaitSuspended`, `CallbackSuspended`, `InvokeSuspended`, etc. as signals that the Lambda function should exit — the AWS service handles re-invocation. No idempotency key beyond the operation ID derived from position counter.

### A3.5 Resource and credential deps

None. There is no mechanism for an operation to declare "I need DB pool X + credential Y." AWS credential handling for the SDK itself uses `aws_config::load_defaults` at Lambda startup (`handler.rs:118`). User business logic credentials are out of scope — the SDK assumes the Lambda execution role has the necessary IAM permissions.

### A3.6 Retry/resilience attachment

Per-step via `StepOptions` (`types.rs:182`):
```rust
pub struct StepOptions {
    retries: Option<u32>,
    backoff_seconds: Option<i32>,
    timeout_seconds: Option<u64>,
    retry_if: Option<RetryPredicate>,  // Arc<dyn Fn(&dyn Any) -> bool + Send + Sync>
}
```
Retry is delegated to the AWS service — the SDK sends a RETRY checkpoint (`DurableError::StepRetryScheduled`), exits the function, and the service re-invokes after the configured delay. No circuit breaker, bulkhead, or hedging. The `retry_if` predicate is a runtime closure, not a compile-time constraint.

### A3.7 Authoring DX

Four API styles, all identical in behavior:
- **Closure:** pass a closure to `durable_lambda_closure::run(handler_fn)` — ~5 lines for hello-world
- **Macro:** `#[durable_execution]` attribute on an async fn — main() is generated (lib.rs:37–45)
- **Trait:** implement `DurableHandler` on a struct + call `durable_lambda_trait::run(MyHandler)` — ~12 lines
- **Builder:** `durable_lambda_builder::handler(|event, ctx| async move { ... }).run().await` — ~8 lines

The macro validates at compile time that the second parameter is `DurableContext` and the return type is `Result<_, DurableError>` (`expand.rs` validates this; trybuild tests confirm in `crates/durable-lambda-macro/tests/ui/`).

### A3.8 Metadata

None. No display name, description, icon, category, or i18n. Operations are identified only by user-supplied string names.

### A3.9 vs Nebula

Nebula defines 5 **action kinds** as sealed traits with associated `Input`/`Output`/`Error` types: `ProcessAction`, `SupplyAction`, `TriggerAction`, `EventAction`, `ScheduleAction`. Each kind imposes compile-time constraints and carries version identity.

durable-lambda-core defines **0 action kinds** as distinct traits. Instead it defines 8 **operation types** as a runtime enum (`OperationType` at `types.rs:60–78`): `Step`, `Wait`, `Callback`, `Invoke`, `Parallel`, `Map`, `ChildContext`, `Log`. These are not types that users implement — they are discriminators for checkpoint records. Users always implement the same pattern (a closure passed to a context method) regardless of which operation type they are invoking.

The conceptual gap is large: Nebula treats each action kind as a first-class type with associated constraints; durable-lambda-core treats all operations as closures dispatched through a single context object.

---

## 4. DAG / execution graph [A2, A9, A10]

**No DAG model.** This project has no workflow graph. Execution flow is sequential Rust code inside a single handler function. Parallelism is achieved through `ctx.parallel(...)` and `ctx.map(...)`, but these are not nodes in a graph — they are async combinator calls. There is no compile-time or runtime graph, no petgraph dependency, no port typing, no topological sort.

Grep evidence: search for `petgraph`, `dag`, `DAG`, `graph`, `node`, `edge`, `topology` across all `.rs` files — only `backend.rs` and test files match, and those matches are unrelated to graph modeling (they reference AWS types and test helper function names).

**Concurrency model:** tokio runtime. `ctx.parallel()` spawns child tasks via `tokio::spawn` inside `crates/durable-lambda-core/src/operations/parallel.rs`. Each branch receives an owned `DurableContext` created by `create_child_context()`. The `Send + 'static` requirement is enforced by tokio's spawn boundary. There is no frontier scheduler, no work-stealing semantics, no `!Send` isolation.

**Comparison with Nebula:** Nebula's TypeDAG (L1-L4) is the engine's core; type-safe port connections at L1 guarantee at compile time that incompatible outputs cannot be wired to incompatible inputs. durable-lambda-core has no such concept — every step produces `serde_json::Value` at the checkpoint level and relies on the user to maintain type correctness across invocations via Rust's generic type inference.

---

## 5. Persistence and recovery [A8, A9]

**No owned storage layer.** Persistence is fully delegated to the AWS Lambda Durable Execution service. The SDK calls two AWS APIs (via `DurableBackend` at `backend.rs:38`):
- `checkpoint_durable_execution` — appends operation updates
- `get_durable_execution_state` — loads the full paginated operation history on each invocation

The **ReplayEngine** (`replay.rs:42`) holds an in-memory `HashMap<String, Operation>` keyed by operation ID. On each Lambda invocation, the context calls `get_execution_state` (paginated), loads all completed operations into the map, and determines whether to start in `Replaying` or `Executing` mode (`replay.rs:100–105`). Recovery semantics: if all completed operations match the current code path (same operation sequence, same IDs), replay returns cached results without re-executing closures. If history is empty, execution proceeds normally.

**Checkpoint protocol:** Every operation sends START then SUCCEED/FAIL. Parallel/map/child_context use `OperationType::Context` with `sub_type` discriminator. Batch checkpoint mode (`enable_batch_mode`) reduces checkpoint calls for sequences of independent steps.

**Operation ID generation:** `blake2b("{counter}")` for root operations; `blake2b("{parent_id}-{counter}")` for children (`operation_id.rs:84–91`). Must match Python SDK exactly — divergence breaks replay for in-flight workflows.

**Comparison with Nebula:** Nebula owns its PostgreSQL schema (sqlx + PgPool + RLS + migrations). Recovery is frontier-based with append-only execution log and checkpoint replay. durable-lambda-core has zero owned storage — AWS is the database.

---

## 6. Credentials / secrets [A4] — DEEP

### A4.1 Existence

**No credential layer exists.** The SDK does not define any credential abstraction, storage mechanism, or lifecycle management.

Grep evidence: search for `credential`, `secret`, `oauth`, `vault`, `keychain`, `zeroize`, `secrecy` across all `.rs` files. The only hit is the phrase "credential-free testing" in a doc comment at `backend.rs:8` — referring to the fact that `MockBackend` does not require AWS credentials, not to any credential management feature.

### A4.2–A4.9

All absent. AWS SDK credentials for the Lambda execution role are loaded by `aws_config::load_defaults(aws_config::BehaviorVersion::latest())` at handler startup (`handler.rs:118`) — this delegates entirely to the AWS SDK's credential provider chain (IAM role, environment variables, instance metadata). No at-rest encryption, no key rotation, no OAuth2, no State/Material split, no LiveCredential, no blue-green refresh.

**vs Nebula:** Nebula has one of the deepest credential subsystems in the Rust workflow engine ecosystem: State/Material split, typed state distinction (Unvalidated/Validated), CredentialOps trait, LiveCredential with watch() for blue-green rotation, OAuth2Protocol blanket adapter, DynAdapter type erasure. durable-lambda-core has none of this — the entire credential concern is outsourced to IAM.

---

## 7. Resource management [A5] — DEEP

### A5.1 Existence

**No resource abstraction.** There are no concepts of DB pools, HTTP clients, or caches as first-class managed resources. Each Lambda invocation creates a fresh `DurableContext` via `DurableContext::new(backend, arn, token, operations, next_marker)` (`handler.rs:135`). The `RealBackend` (wrapping `aws_sdk_lambda::Client`) is shared across invocations via `Arc<RealBackend>` created once at startup — but this is not a lifecycle-managed resource, it is a standard Lambda handler pattern.

### A5.2–A5.8

All absent. No scope levels, no lifecycle hooks, no hot-reload, no generation tracking, no credential dependency declaration, no backpressure. Resources are user responsibility.

Grep evidence: search for `ReloadOutcome`, `resource`, `lifecycle`, `init_hook`, `shutdown`, `health_check` across all `.rs` files — no matches.

**vs Nebula:** Nebula's resource subsystem (4 scope levels, ReloadOutcome enum, generation tracking, on_credential_refresh per-resource hook) has no analog here. durable-lambda-core relies on the Lambda execution model (cold start → warm reuse) as its resource lifecycle.

---

## 8. Resilience [A6, A18]

The only resilience mechanism is step-level retry via `StepOptions::retries(n).backoff_seconds(s)` and per-step `retry_if` predicate. The retry loop is not implemented in the SDK — it sends a RETRY checkpoint to AWS and exits; AWS is responsible for re-invocation timing.

For AWS API calls (checkpointing), `RealBackend` implements a manual retry loop (`backend.rs:136–184`) with exponential backoff + full jitter (3 retries max, base 100ms, cap 2000ms). Retryable conditions: throttling, rate exceeded, service unavailable, internal server error, timeout (detected by string-scanning the error message).

**No circuit breaker, bulkhead, hedging, or unified error classifier.** There is no `ErrorClassifier` trait.

The `DurableError` enum (`error.rs:26`) uses `thiserror::Error` and has 17 variants with stable `.code()` string identifiers. It is a `#[non_exhaustive]` enum. Errors are typed per failure mode (ReplayMismatch, CheckpointFailed, StepRetryScheduled, WaitSuspended, etc.) rather than classified by transient/permanent axis.

**vs Nebula:** nebula-resilience provides retry / CB / bulkhead / timeout / hedging with unified `ErrorClassifier`. durable-lambda-core has point retry only, delegating the actual retry scheduling to AWS.

---

## 9. Expression / data routing [A7]

**No expression engine.** There is no DSL, no expression syntax, no `$nodes.foo.result.email` style routing. Data flows as owned Rust values (typed generics serialized to/from `serde_json::Value` at checkpoint boundaries). Routing logic is plain Rust code inside handler closures.

Grep evidence: search for `expression`, `expr`, `DSL`, `sandbox`, `eval` across all `.rs` files — no matches.

---

## 10. Plugin / extension system [A11] — DEEP (BUILD + EXEC)

### 10.A — Plugin BUILD process (A11.1–A11.4)

**No plugin system exists.** There is no plugin format, manifest, toolchain, registry, or build process.

Grep evidence: search for `plugin`, `wasm`, `wasmtime`, `wasmer`, `libloading`, `dylib`, `plugin_host` across all `.rs` files — zero matches.

### 10.B — Plugin EXECUTION sandbox (A11.5–A11.9)

**No plugin execution sandbox.** There is no dynamic loading, WASM runtime, subprocess IPC, or capability-based permission system.

**vs Nebula:** Nebula targets WASM + capability security + Plugin Fund commercial model (royalties to plugin authors). durable-lambda-core has no extension model — the "plugin" is literally writing a Lambda function that calls into the SDK.

---

## 11. Trigger / event model [A12] — DEEP

### A12.1 Trigger types

**No trigger abstraction.** durable-lambda-core does not implement triggers. It is invoked by the AWS Lambda service when the durable execution service re-invokes the function. How the initial invocation is triggered (API Gateway, EventBridge, SQS, manual) is entirely outside the scope of this SDK.

Grep evidence: search for `trigger`, `webhook`, `cron`, `schedule`, `kafka`, `event_source`, `TriggerAction` across all `.rs` files. Matches for "event" resolve to `crates/durable-lambda-core/src/event.rs`, which is the Lambda invocation event parser (`parse_invocation`) — not a trigger model.

### A12.2–A12.8

All absent for the same reason: trigger modeling is the AWS service's responsibility, not this SDK's.

**vs Nebula:** Nebula's `TriggerAction` (Input = Config for registration, Output = Event for typed payload) + `Source` trait for 2-stage normalization has no equivalent here.

---

## 12. Multi-tenancy [A14]

**None.** No tenant isolation, RBAC, SSO, or SCIM.

Grep evidence: search for `tenant`, `rbac`, `sso`, `scim`, `schema` across all `.rs` files — matches for `schema` in test files resolve to `aws_sdk_lambda::types::OperationType::Schema` (AWS type, unrelated). No multi-tenancy concepts exist in the codebase.

Tenancy in a durable-lambda-core deployment is handled by Lambda function-level IAM policies and account/region boundaries — AWS infrastructure, not the SDK.

---

## 13. Observability [A15]

Structured logging via `tracing` crate. All log operations are **replay-safe** — they no-op during replay mode to avoid duplicate log output on re-invocation (`operations/log.rs`). Log methods on `DurableContextOps` (`ops_trait.rs:249–270`): `log`, `log_with_data`, `log_debug`, `log_warn`, `log_error`, each with a `_with_data` variant for `serde_json::Value` structured data.

No OpenTelemetry, no metrics, no per-operation tracing spans. The SDK does not emit spans for checkpoint calls or replay transitions. Users can add their own `tracing-subscriber` setup via the builder API (`.with_tracing(subscriber)`).

**vs Nebula:** Nebula targets OpenTelemetry with one trace per execution and per-action latency/count/error metrics. durable-lambda-core has no OTel — only basic `tracing` log emission.

---

## 14. API surface [A16]

**Programmatic API only.** No REST, GraphQL, gRPC, or OpenAPI. The SDK is consumed as a Rust library. The four ergonomic API surfaces (closure / macro / trait / builder) are all library-level.

**Versioning:** The workspace is at v1.2.0. No runtime API versioning; semver only.

---

## 15. Testing infrastructure [A19]

`durable-lambda-testing` crate provides `MockDurableContext` with a builder API (`mock_context.rs`):
```rust
MockDurableContext::new()
    .with_step_result("name", r#"json"#)
    .with_step_error("name", "ErrorType", r#"json"#)
    .with_wait("name")
    .with_callback("name", "cb-id", r#"json"#)
    .with_invoke("name", r#"json_result"#)
    .build()
    .await
// Returns: (DurableContext, CheckpointRecorder, OperationRecorder)
```

Assertion helpers (`assertions.rs`): `assert_no_checkpoints`, `assert_checkpoint_count`, `assert_operations`, `assert_operation_names`, `assert_operation_count`. All return `()` and panic on failure.

Test suites: `e2e` (28 end-to-end workflow tests), `parity` (cross-approach behavioral parity tests), `compliance` (Python–Rust fixture-based parity). Unit tests are embedded in each source file.

**vs Nebula:** nebula-testing provides contract tests for resource implementors (resource-author-contracts.md), wiremock integration, mockall. durable-lambda-core's testing crate is narrower but well-suited for its purpose: the mock builder pattern is ergonomic and the assertion helpers are production-quality.

---

## 16. AI / LLM integration [A21] — DEEP

### A21.1 Existence

**None.** There is no AI or LLM integration in this SDK.

Grep evidence: search for `llm`, `openai`, `anthropic`, `gpt`, `claude`, `embedding`, `completion`, `ai_agent` (case-insensitive) across all `.rs` files — zero matches. The search for `ai` returned matches only in AWS type names (`aws_sdk_lambda`, `aws_smithy_types`) and function/variable names unrelated to AI (e.g., `await`).

### A21.2–A21.13

All absent. The project is not AI-first and has no stated plans for LLM integration. Its positioning is entirely AWS Lambda Durable Execution — a managed infrastructure primitive, not an AI orchestration layer.

**vs Nebula + Surge:** Nebula has no first-class LLM abstraction yet (strategic bet: AI = generic actions + plugin LLM client). Surge is the separate agent orchestrator on ACP. durable-lambda-core similarly has no LLM — it is even further from AI than Nebula, as it does not have Nebula's generic action model that could host an LLM step.

---

## 17. Notable design decisions

### D1 — Server-owned durability

The most fundamental decision: delegate the entire durability problem (state storage, invocation scheduling, retry orchestration, parallel branch coordination) to AWS Lambda Durable Execution. The SDK never stores state — it only serializes operations to/from the AWS APIs (`DurableBackend` at `backend.rs:38`). This eliminates entire subsystems (no DB, no scheduler, no distributed coordination) at the cost of AWS vendor lock-in and the requirement that the target AWS account has Lambda Durable Execution enabled.

**Trade-off:** Maximum operational simplicity for AWS-native users; zero portability to non-AWS environments.

**Applicability to Nebula:** Not applicable. Nebula's value proposition includes self-hosted and desktop deployment modes — it cannot delegate durability to any single cloud provider.

### D2 — Replay via position-based operation IDs

Operation identity is positional: `blake2b("{counter}")` for root, `blake2b("{parent_id}-{counter}")` for children (`operation_id.rs:84–91`). This matches the Python SDK exactly — the compliance test suite verifies byte-level parity (`compliance/` directory). The consequence is that **operation order is a compatibility contract**: reordering or inserting operations between deployments breaks in-flight workflow replay.

**Trade-off:** Simple implementation, Python SDK parity, no need for operation name uniqueness enforcement. Fragile to code changes during active deployments.

**vs duroxide:** duroxide (MS DurableTask) also uses history replay but names activities by registered function name + sequence number. durable-lambda-core uses pure counter + blake2b — more opaque but matches its target AWS service's behavior.

### D3 — Four identical API surfaces

The same 8 operations are exposed via closure, macro, trait, and builder styles — all delegating to `DurableContext` (`handler.rs`, `context.rs` in each wrapper crate). The behavioral parity is verified by the `tests/parity` suite. This is unusual: most SDKs pick one style. The decision reflects an ergonomic choice to let teams adopt their preferred Rust style without behavioral penalty.

**Trade-off:** 4x documentation surface; risk of wrapper crates drifting if core changes. Mitigated by the ops_trait.rs delegation pattern and parity tests.

### D4 — `DurableContextOps` as static-dispatch-only trait

`ops_trait.rs:7–9` explicitly prohibits `dyn DurableContextOps`. The trait uses RPITIT (`-> impl Future<...>`), which is not object-safe. This enables generic handler functions (`async fn process<C: DurableContextOps>`) without boxing, preserving zero-cost abstractions.

**Trade-off:** Cannot store `Box<dyn DurableContextOps>` for late-binding composition. Acceptable given the project's focused scope.

**Comparison with Nebula:** Nebula uses `DynAdapter` for type erasure where needed; Nebula's action traits are designed for both static and dynamic dispatch. durable-lambda-core's choice is simpler and adequate for its use case.

### D5 — Dual-layer Result for step outcomes

`Result<Result<T, E>, DurableError>` is the canonical step return type. Both `Ok(T)` and `Err(E)` inside the inner Result are checkpointed identically. This cleanly separates SDK infrastructure errors (outer) from user business errors (inner) — a semantically precise design that avoids conflating replay-safe business failures with non-deterministic infrastructure failures.

**Applicability to Nebula:** Nebula already has a similar separation via `nebula-error` + `ErrorClass` (transient/permanent classification). The dual-Result pattern could be worth borrowing for ProcessAction's error surface.

### D6 — Compliance suite as first-class artifact

The `compliance/` directory contains Python reference workflows and Rust equivalents with shared JSON fixtures. The CI verifies that Rust and Python produce identical operation sequences for the same logical workflow. This is a strong correctness guarantee for a Python parity SDK.

**vs Nebula:** Nebula has no equivalent compliance layer against a reference SDK. Not applicable (Nebula has no reference implementation to comply with).

### D7 — BMAD process artifacts in `.claude/skills/`

The repo contains 38+ BMAD (AI-assisted development methodology) skill files in `.claude/skills/`. This is evidence that the SDK was built using AI-assisted development with an explicit methodology (BMAD = "Build More, Argue Less with the Developers"). The `_bmad-output/project-context.md` referenced in the README documents 38 implementation rules. This is architecturally neutral but noteworthy as a development practice.

---

## 18. Known limitations / pain points

**No open GitHub issues found.** `gh issue list --repo pgdad/durable-rust --state all --limit 30` returned an empty array. The project has 0 stars and 0 forks from external users, so there are no community-reported pain points.

Based on code analysis:

1. **Determinism discipline is entirely user-enforced.** The SDK cannot detect non-deterministic code outside step closures at compile time or runtime (no static analysis, no checker). The README documents the "safety checklist" (no `Utc::now()`, no `Uuid::new_v4()` outside steps) but violation silently produces replay mismatches that are hard to diagnose.

2. **Parallel/map closure syntax is verbose.** The `BranchFn` type alias pattern (`Box<dyn FnOnce(DurableContext) -> Pin<Box<dyn Future<...> + Send>> + Send>`) is necessary but ergonomically heavy. The README devotes a troubleshooting section to `Send + 'static` errors on parallel closures.

3. **Step results require explicit type annotations.** `let result: Result<T, E> = ctx.step(...)` — the compiler cannot infer `T` and `E` from the closure return type alone (README troubleshooting section). This is a known limitation of Rust's type inference with serde deserialization.

4. **AWS vendor lock-in is total.** No abstraction allows swapping out the AWS backend for a different durable execution provider. The `DurableBackend` trait exists but its interface mirrors the AWS API exactly (`checkpoint_durable_execution`, `get_durable_execution_state`).

5. **No operation reordering safety.** Deploying a new Lambda version with reordered operations while in-flight workflows exist will silently corrupt replay. There is no version guard or migration mechanism.

---

## 19. Bus factor / sustainability

- **Maintainers:** 1 (pgdad)
- **Commit cadence:** 20 commits sampled from `git log --oneline -20`, spanning approximately 2 weeks of active development (project created 2026-03-15, last tagged 2026-04-01)
- **Stars / forks:** 2 stars, 0 forks — minimal external adoption
- **Issues:** 0 open, 0 closed — no community engagement
- **Release:** v1.2.0 is the only published release (6 crates on crates.io per commit message)
- **Bus factor:** 1 — if the sole author stops, the project stops

The project is very young (6 weeks) and has minimal external adoption. Its value depends entirely on the continued existence and stability of the AWS Lambda Durable Execution service, which is itself a relatively new AWS feature.

---

## 20. Final scorecard vs Nebula

| Axis | Their approach | Nebula approach | Verdict | Borrow? |
|------|---------------|-----------------|---------|---------|
| A1 Workspace | 6 crates: 1 core + 4 wrappers + 1 testing; flat fan-out pattern; no feature flags | 26 crates layered: nebula-error / nebula-resilience / nebula-credential / nebula-resource / nebula-action / nebula-engine / nebula-tenant / nebula-eventbus / etc. Edition 2024 | Nebula deeper — durable-lambda-core's scope is narrow by design | no — different goals |
| A2 DAG | No DAG model. Sequential Rust code; parallel/map as async combinators | TypeDAG: L1=static generics enforce port types at compile time; L2=TypeId; L3=refinement predicates; L4=petgraph soundness checks | Nebula deeper — durable-lambda-core has no graph concept | no — different goals |
| A3 Action | No action kinds. 8 operation types as enum discriminators; closures via single `DurableContextOps` trait; RPITIT, no GATs, no associated types, no sealing | 5 action kinds (Process/Supply/Trigger/Event/Schedule). Sealed trait. Associated Input/Output/Error. Versioning. Derive macros | Nebula deeper — dual-Result pattern is worth noting | refine — dual-Result error separation is a borrowable idea |
| A11 Plugin BUILD | None — no plugin system exists | WASM sandbox planned (wasmtime), plugin-v2 spec, Plugin Fund commercial model | Nebula deeper (planned) | no — different goals |
| A11 Plugin EXEC | None — no plugin execution sandbox | WASM sandbox + capability security | Nebula deeper (planned) | no — different goals |
| A18 Errors | `DurableError` enum with 17 variants via `thiserror`, `.code()` stable identifiers, `#[non_exhaustive]`, no ErrorClass axis | nebula-error crate, ErrorClass enum (transient/permanent/cancelled/etc.), used by ErrorClassifier in resilience | Different decomposition — durable-lambda-core's per-variant `.code()` stable string IDs are a clean pattern | refine — stable `.code()` string identifiers per variant is worth adopting in nebula-error |
| A21 AI/LLM | None — zero AI/LLM integration; grep confirms no openai/anthropic/llm/embedding/completion references | No first-class LLM abstraction yet — strategic bet: AI = generic actions + plugin LLM client. Surge handles agent orchestration on ACP | Convergent — both projects have no LLM integration | no — different goals |

---

## §17 subsection — vs duroxide

duroxide (MS DurableTask Rust binding) and durable-lambda-core both implement history-replay-based durable execution, but at different levels:

**Replay mechanism:** Both use a position-keyed operation history. duroxide replays named activities/orchestrations registered with the DurableTask sidecar. durable-lambda-core uses `blake2b(counter)` IDs matching the Python SDK — more opaque but AWS-service-compatible.

**Infrastructure ownership:** duroxide requires a DurableTask sidecar (dapr or standalone), giving it runtime portability across Azure/AWS/GCP/self-hosted. durable-lambda-core requires AWS Lambda Durable Execution — zero portability.

**Abstraction style:** duroxide exposes activities as registered async functions (`register_activity!()`). durable-lambda-core exposes operations as context method calls (`ctx.step("name", || async { ... })`). Neither uses a sealed trait hierarchy like Nebula's 5 action kinds.

**Scope:** duroxide is a protocol adapter for the DurableTask protocol. durable-lambda-core is a full SDK covering 8 operation types, 4 API styles, testing utilities, and a compliance suite. durable-lambda-core is the more complete developer-facing product despite being younger.

**Key differentiator:** durable-lambda-core's compliance suite against Python reference workflows is a stronger correctness guarantee than anything duroxide ships. duroxide relies on the DurableTask sidecar's correctness; durable-lambda-core verifies Rust↔Python behavioral parity independently.
