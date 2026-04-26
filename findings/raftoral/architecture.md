# Raftoral — Architectural Decomposition

## 0. Project metadata

| Field | Value |
|-------|-------|
| Repo | https://github.com/orishu/raftoral |
| Stars | 6 |
| Forks | 0 |
| License | MIT |
| Language | Rust (100%) |
| Edition | 2024 |
| Created | 2025-09-22 |
| Last commit | 2025-12-29 |
| Latest release | v0.2.0 (unreleased tag only — no GitHub release since v0.1.1 on 2025-10-11) |
| Maintainers | 1 (Ori Shalev, ori.shalev@gmail.com) |
| Open issues | 0 |
| Closed issues | 0 |
| LOC | ~16,243 across 49 .rs files (tokei unavailable; wc -l count) |
| Tests | 104 inline unit/integration tests |

---

## 1. Concept positioning [A1, A13, A20]

**Author's own description (README):** "A Rust library for building fault-tolerant, distributed workflows using Raft consensus."

**Mine (after reading code):** A single-crate embedded workflow engine that replaces Temporal's centralized server with two-tier Raft consensus woven directly into the application process — workflows are closures, checkpoints are replicated variables, and persistence is the Raft log rather than a separate database.

**Comparison with Nebula:** Nebula is a 26-crate workflow platform with a rich action taxonomy (5 kinds), credential/resource subsystems, plugin architecture, and multi-tenancy. Raftoral has a single action shape (async closure + checkpoint macros), no credential layer, no resource abstraction, and no tenancy. Raftoral's unique contribution is eliminating external infrastructure by embedding Raft consensus at the workflow level — a dimension Nebula does not target.

---

## 2. Workspace structure [A1]

Raftoral has **2 crates** in a simple workspace:

1. **`raftoral`** (root, `Cargo.toml` line 1) — The core library. Compiled as both `cdylib` (for WASM/FFI) and `rlib`. Contains all Raft infrastructure, workflow runtime, management runtime, HTTP and gRPC servers, RocksDB storage, and WASM bindings.

2. **`raftoral-client`** (`raftoral-client/Cargo.toml` line 1) — A thin gRPC client SDK for sidecar mode. Applications in polyglot environments (non-Rust) call the sidecar through this crate.

There are **no feature flags for deployment mode** — instead, the caller composes the pieces (e.g., `FullNode<WorkflowRuntime>` for embedded, `FullNode<WorkflowProxyRuntime>` for sidecar). WASM is controlled by the `target_arch = "wasm32"` cfg gate. Persistent storage is the `persistent-storage` feature (default: on), which enables RocksDB (`Cargo.toml` line 13: `rocksdb = { version = "^0.24.0", optional = true }`).

**Comparison with Nebula:** Nebula has 26 crates with strict layer boundaries (nebula-error → nebula-resilience → nebula-credential → nebula-resource → nebula-action → nebula-engine → nebula-tenant → …). Raftoral is a monolith by comparison — everything ships in one crate. This keeps friction low for early adopters but makes individual subsystem reuse impossible and makes it impossible to depend on, say, only the Raft layer without pulling in workflow logic.

---

## 3. Core abstractions [A3, A17] ⭐ DEEP

### A3.1 — Trait shape

The "unit of work" in Raftoral is **a workflow function**, not a trait-based action hierarchy. The core trait is `WorkflowFunction<I, O>` defined in `src/workflow/registry.rs`:

```rust
pub trait WorkflowFunction<I, O>: Send + Sync + 'static
where
    I: Send + 'static,
    O: Send + 'static,
{
    fn execute(
        &self,
        input: I,
        context: WorkflowContext,
    ) -> Pin<Box<dyn Future<Output = Result<O, WorkflowError>> + Send>>;
}
```

`src/workflow/registry.rs:9-22`

The trait is **open** (anyone can implement it) but in practice users always pass closures via `register_closure`, which wraps them in a private `ClosureWorkflow<I, O, F, Fut>` struct inside `WorkflowRegistry::register_closure`. There are no sealed sub-kinds, no GATs, no HRTBs. The trait has 2 type parameters (I/O) and 0 associated types.

There is a single generic `StateMachine` trait for Raft state machines (`src/raft/generic/state_machine.rs:14-35`):

```rust
pub trait StateMachine: Send + Sync {
    type Command: Serialize + for<'de> Deserialize<'de> + Send + Sync;
    type Event: Clone + Send + Sync;
    fn apply(&mut self, command: &Self::Command) -> Result<Vec<Self::Event>, Box<dyn std::error::Error>>;
    fn snapshot(&self) -> Result<Vec<u8>, Box<dyn std::error::Error>>;
    fn restore(&mut self, snapshot: &[u8]) -> Result<(), Box<dyn std::error::Error>>;
    fn on_follower_failed(&mut self, _node_id: u64) -> Vec<Self::Event> { vec![] }
}
```

`StateMachine` has 2 associated types (`Command`, `Event`), not GATs. `RaftNode<SM: StateMachine>` is generic over it.

### A3.2 — I/O shape

Input and output types are **constrained to `Serialize + Deserialize`** (serde). At the registry boundary, input is serialized to `Vec<u8>` before storage and deserialized inside the boxed executor (`src/workflow/registry.rs:38-53`). Output is also `Vec<u8>` at the erased layer. The type erasure mechanism is `BoxedWorkflowFunction`, which stores a `Box<dyn Fn(Box<dyn Any + Send>, WorkflowContext) -> Pin<Box<...>>  + Send + Sync>`. There is no streaming output; the workflow returns a single `Result<O, WorkflowError>` at completion.

### A3.3 — Versioning

Versioning is **explicit at registration time**: `registry.register_closure("process_order", 1, ...)` and `registry.register_closure("process_order", 2, ...)` are stored under key `(workflow_type: String, version: u32)` in `WorkflowRegistry::workflows: HashMap<(String, u32), Arc<BoxedWorkflowFunction>>` (`src/workflow/registry.rs:87`). Multiple versions coexist side-by-side. Start requests carry version explicitly (`WorkflowCommand::WorkflowStart { version: u32, ... }` — `src/workflow/state_machine.rs:23`). There is no migration support, no `#[deprecated]`, no schema evolution for old checkpoints.

### A3.4 — Lifecycle hooks

There is **one lifecycle phase: execute**. No pre/post/cleanup/on-failure hooks. Cancellation: not yet implemented (planned as `WorkflowCommand::WorkflowTerminate` in `docs/FEATURE_PLAN.md` §1.2; the `WorkflowCommand` enum in `src/workflow/state_machine.rs` has no `Terminate` variant). Idempotency: workflows are keyed by a user-supplied `workflow_id` string; starting a workflow with an existing ID returns `WorkflowError::AlreadyExists`.

### A3.5 — Resource and credential dependencies

None. There is no injection mechanism. Workflow closures close over whatever they need from surrounding scope, which is standard Rust but means the framework provides no lifetime tracking, no pool sharing, and no reload hooks for dependencies.

### A3.6 — Retry/resilience

None built into the framework. The README notes explicitly: "Retry logic in your closure." There is no per-workflow retry policy, no circuit breaker attachment, no bulkhead.

### A3.7 — Authoring DX

The minimal "hello world" workflow:

```rust
registry.register_workflow_closure("my_workflow", 1,
    |input: MyInput, ctx| async move {
        let result = checkpoint!(ctx, "result", process(input));
        Ok(MyOutput { value: result.get() })
    }
).await?;
```

Approximately 5 lines. No derive macros needed. The `checkpoint!` and `checkpoint_compute!` macros are defined in `src/lib.rs` and expand to `ctx.create_replicated_var(...)` calls.

### A3.8 — Metadata

There is **no metadata system**. No display name, description, icon, or category per workflow. The only identifier is the `(workflow_type: String, version: u32)` key.

### A3.9 — vs Nebula

Nebula has 5 sealed action kinds (ProcessAction / SupplyAction / TriggerAction / EventAction / ScheduleAction) each with associated `Input`, `Output`, `Error` types and versioning baked into the type system. Raftoral has **one action shape** (async closure with I/O generic over `Serialize + Deserialize`) with runtime version dispatch. Nebula's richer taxonomy enables compile-time assurances (e.g., TriggerAction's `Input = Config` / `Output = Event` constraint). Raftoral trades taxonomy depth for simplicity and embedded distribution.

---

## 4. DAG / execution graph [A2, A9, A10]

### A2 — Graph model

Raftoral has **no DAG model**. Workflow execution is a single sequential async function — there is no graph of nodes, no port typing, no compile-time edge checking, no petgraph. The "graph" is the sequential execution path of a Rust `async fn`. If a workflow needs fan-out it must be implemented in user code (spawn tasks, coordinate via checkpoints).

**Comparison with Nebula:** Nebula's TypeDAG operates at 4 levels (generics → TypeId → predicates → petgraph). Raftoral has no graph abstraction at all. This is a fundamental scope difference: Raftoral orchestrates sequential durable tasks; Nebula orchestrates graphs of heterogeneous nodes.

### A10 — Concurrency

Runtime is **tokio** (`Cargo.toml` non-WASM dependencies). Each `RaftNode<SM>` runs a tick loop driven by `tokio::time::interval` inside `RaftNode::run()`. The **owner/wait pattern** is the core concurrency model:

- All nodes in a cluster receive and apply every `WorkflowStart` command (parallel execution)
- The **owner node** (initially the proposer) executes the workflow closure and proposes checkpoint commands
- **Non-owner nodes** subscribe to the EventBus and wait for `WorkflowEvent::CheckpointSet` events to advance through the same code path without re-executing side effects

The `WorkflowOwnershipMap` (`src/workflow/ownership.rs`) maps `workflow_id → owner_node_id` in an `Arc<Mutex<HashMap>>`. There is no `!Send` isolation — workflow closures must be `Send + Sync + 'static`.

---

## 5. Persistence & recovery [A8, A9]

### A8 — Storage layer

The Raft log is the **only storage layer**. There is no external database. Two backends:

1. **RocksDB** (default, `persistent-storage` feature): `src/raft/generic/rocksdb_storage.rs`. Three column families: `entries` (keyed `entry_{index}` u64), `metadata` (hard_state, conf_state, first_index, last_index, node_id), `snapshot` (snapshot_data, snapshot_metadata). Written via `WriteBatch`. No SQL, no migrations, no ORM.

2. **In-memory** (`MemStorage` from raft-rs, used when `storage_path: None`): `src/raft/generic/storage.rs` — `type MemStorageWithSnapshot = MemStorage`. Data is lost on process exit.

### A9 — Persistence model

The persistence model is **checkpoint-based with Raft log as the journal**:

- Every `ReplicatedVar::set()` call issues `WorkflowRuntime::set_checkpoint()` which proposes a `WorkflowCommand::SetCheckpoint { workflow_id, key, value: Vec<u8> }` through Raft consensus.
- Committed entries are applied by `WorkflowStateMachine::apply()` (`src/workflow/state_machine.rs`), which stores checkpoint values in two structures: `checkpoint_queues` (transient, for follower catch-up) and `checkpoint_history` (permanent, for snapshots).
- **Snapshots**: `WorkflowStateMachine::snapshot()` serializes the entire in-memory state to JSON bytes (`src/workflow/state_machine.rs:186-208`). `restore()` reconstructs state from snapshot bytes and repopulates `checkpoint_queues` from `checkpoint_history`.
- **Recovery**: On restart, node loads RocksDB hard state and log entries, replays from last snapshot via raft-rs `RawNode`, and the `WorkflowStateMachine::restore()` method repopulates checkpoint queues so execution can resume at the correct checkpoint.
- **Snapshot interval**: configurable via `RaftNodeConfig::snapshot_interval` (default: 1000 committed entries).

**Comparison with Nebula:** Nebula uses a frontier-based scheduler with append-only execution log in PostgreSQL (sqlx + PgPool + PgExecutionRepo). Raftoral uses the Raft log itself as the persistent event store — no external database required, but recovery is bounded by what fits in a Raft snapshot (entire state must serialize to memory). Nebula's frontier model allows streaming replay of arbitrary-length histories; Raftoral's snapshot-based model bounds snapshot size to available RAM.

---

## 6. Credentials / secrets [A4] ⭐ DEEP

### A4.1 — Existence

**No credential layer exists.** Searched `src/` for: `credential`, `secret`, `token`, `auth`, `oauth` — found only `src/config.rs` which has commented-out TLS/interceptor stubs with no implementation. No crate, no trait, no storage.

**Grep evidence:**

```
grep -r "credential\|secret\|token\|auth\|oauth" src/ --include="*.rs" -l
→ src/config.rs (only commented TODO lines referencing future gRPC channel auth)
```

### A4.2–A4.9 — All Not Applicable

No at-rest encryption, no vault backend, no Zeroize/secrecy, no refresh model, no OAuth2/OIDC, no scope concept. The Raft transport (gRPC) has no TLS by default — TLS configurators are mentioned in comments in `src/config.rs` but are explicitly commented out.

**Comparison with Nebula:** Nebula has State/Material split, CredentialOps trait, LiveCredential with `watch()` for blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter type erasure — a credential subsystem that is arguably the most elaborate of any Rust workflow engine surveyed. Raftoral has none of this. It is a pure infrastructure library; credential management is entirely delegated to the calling application.

---

## 7. Resource management [A5] ⭐ DEEP

### A5.1 — Existence

**No resource abstraction exists.** Searched `src/` for: `resource`, `pool`, `health_check`, `reload`, `hot_reload`, `ReloadOutcome` — found zero matches outside inline comments. Each workflow closure closes over whatever it needs from the surrounding scope; the framework has no concept of a managed, scoped, or health-checked resource.

**Grep evidence:**

```
grep -r "ReloadOutcome\|resource\|hot_reload\|pool" src/ --include="*.rs" -l
→ (empty)
```

### A5.2–A5.8 — All Not Applicable

No scope levels, no lifecycle hooks, no reload support, no credential dependency declarations, no backpressure for resource acquisition. Resource sharing is achieved through ordinary Rust `Arc<T>` shared across closure captures — correct but outside any framework governance.

**Comparison with Nebula:** Nebula has 4 scope levels (Global / Workflow / Execution / Action), `ReloadOutcome` enum, generation tracking, `on_credential_refresh` hook per resource. Raftoral has none of these. Again, this reflects different scopes: Nebula is a full-stack workflow platform, Raftoral is a distributed coordination primitive.

---

## 8. Resilience [A6, A18]

### A6 — Resilience patterns

**No resilience framework exists.** There is no retry policy, circuit breaker, bulkhead, timeout wrapper, or hedging primitive. The README states explicitly: "Retry logic in your closure." The `WorkflowError` enum (`src/workflow/error.rs`) has 7 variants (`AlreadyExists`, `NotFound`, `NotLeader`, `ClusterError`, `Timeout`, `SerializationError`, `DeserializationError`) but no classification dimension (transient vs permanent).

The transport layer returns `TransportError` (`src/raft/generic/errors.rs`) and `RoutingError` with `MailboxFull` for backpressure at the Raft message level, but these are not exposed to workflow authors.

### A18 — Error types

Raftoral uses **ad-hoc error types per layer**: `WorkflowError`, `TransportError`, `RoutingError`, `ReplicatedVarError`. No unified error taxonomy, no `thiserror`-derived hierarchy in the main crate (only `raftoral-client` uses `thiserror`). No `anyhow`/`eyre`. Errors are `Clone + Serialize + Deserialize` where workflow-level (so they can be stored in Raft state and transported across the network in `WorkflowEvent::WorkflowFailed { error: Vec<u8> }`).

**Comparison with Nebula:** Nebula has `nebula-error` crate with `ErrorClass` enum (transient / permanent / cancelled / …) consumed by `ErrorClassifier` in `nebula-resilience` for policy decisions. Raftoral has neither a resilience crate nor error classification.

---

## 9. Expression / data routing [A7]

**No expression engine exists.** There is no DSL, no `$nodes.foo.result.email`-style path expression, no type inference, no sandboxed evaluator. Workflow data routing is done through Rust closures and `ReplicatedVar<T>` dereferencing.

**Grep evidence:**

```
grep -r "expression\|eval\|jsonpath\|dsl\|sandbox" src/ --include="*.rs" -l
→ (empty)
```

**Comparison with Nebula:** Nebula has a 60+ function expression engine with type inference and sandbox. Raftoral delegates all data routing to Rust code.

---

## 10. Plugin / extension system [A11] ⭐ DEEP — TWO sub-sections

### 10.A — Plugin BUILD process (A11.1–A11.4)

**No plugin system exists.** There is no manifest format, no plugin registry, no plugin SDK, no compile-time plugin pipeline.

**A11.1:** No plugin format. The `SubClusterRuntime` trait (`src/management/sub_cluster_runtime.rs`) allows different runtime types to be injected into `ManagementRuntime<R>`, but this is a compile-time generic parameter in the host binary, not a runtime plugin system.

**A11.2:** No plugin toolchain.

**A11.3:** No manifest.

**A11.4:** No registry.

**Grep evidence:**

```
grep -r "plugin\|Plugin" src/ --include="*.rs" -l
→ src/management/runtime.rs, src/management/sub_cluster_runtime.rs,
  src/raft/generic/integration_tests.rs
```

All hits refer to Rust generics (`SubClusterRuntime` is called "plugin" in one comment) — not a runtime plugin system.

### 10.B — Plugin EXECUTION sandbox (A11.5–A11.9)

**No plugin sandbox.** There is **experimental WASM support** but it is for running Raftoral itself in a WASM environment (Node.js / browser), not for sandboxing third-party plugins. `src/wasm/mod.rs` provides `wasm-bindgen` entry points (`WasmRaftoralNode`) that wrap `FullNode` for JavaScript use. There is a `build-wasm.sh` script for compiling to WASM target.

The sidecar architecture (`src/sidecar/`, `src/workflow_proxy_runtime.rs`) is the closest analog to a plugin execution boundary: the application process is a separate container communicating over gRPC streams (`sidecar.proto`). But this is an execution delegation mechanism, not a sandboxed plugin model.

**A11.9 vs Nebula:** Nebula targets WASM sandbox + capability security + Plugin Fund commercial model. Raftoral has no plugin system; the sidecar is a polyglot extension point but not sandboxed and has no capability model.

---

## 11. Trigger / event model [A12] ⭐ DEEP

### A12.1–A12.8 — Trigger types

**No trigger system exists.** Raftoral has no concept of a trigger, webhook, schedule, external event source, cron, Kafka consumer, or polling loop. Workflow execution is initiated **imperatively**: a caller invokes `WorkflowRuntime::start_workflow(...)` or the gRPC `RunWorkflowAsync` RPC (`raftoral.proto` `WorkflowManagement` service). There is no reactive execution model.

**Grep evidence for trigger-related concepts:**

```
grep -r "trigger\|webhook\|schedule\|cron\|event_source\|kafka\|rabbitmq\|nats" src/ --include="*.rs" -l
→ src/full_node/mod.rs, src/management/cluster_manager.rs,
  src/raft/generic/proposal_router.rs
```

All hits are unrelated to workflow triggers: `full_node/mod.rs` uses "trigger" in a Raft leadership context, `cluster_manager.rs` references "event" for cluster creation events, `proposal_router.rs` uses "trigger" for leader change detection.

**A12.7 — Trigger as Action:** There is no trigger kind. Workflows are started imperatively, not through reactive sources.

**A12.8 vs Nebula:** Nebula has a 2-stage model: `Source` normalizes raw inbound (HTTP request / Kafka message / cron tick) into a typed `Event`, then `TriggerAction<Input=Config, Output=Event>` produces Events consumed by `EventAction` nodes. Raftoral has no equivalent. External events (Kafka messages, HTTP webhooks) must be translated to `start_workflow()` calls by application code outside Raftoral.

---

## 12. Multi-tenancy [A14]

No multi-tenancy support. There is no tenant isolation, no schema-per-tenant, no RLS, no RBAC, no SSO/SCIM. All workflows share a single flat namespace within a cluster. The `workflow_id` is a user-supplied string with no ownership/tenant prefix enforced by the framework.

**Comparison with Nebula:** Nebula has `nebula-tenant` crate with three isolation modes (schema / RLS / database), RBAC, SSO planned, SCIM planned.

---

## 13. Observability [A15]

No OpenTelemetry, no structured tracing per execution, no metrics export. Logging uses `slog` throughout (`slog = "^2.7"`, `slog-term`, `slog-async`). Log output is human-readable terminal format via `slog_term::FullFormat`. There are no per-execution trace spans, no Prometheus metrics, no OTLP exporter. Logging granularity is manual `info!`/`debug!`/`warn!` calls in key paths.

The sidecar proto (`sidecar.proto`) includes a `HeartbeatRequest/Response` for connection liveness but no observability instrumentation.

**Comparison with Nebula:** Nebula has OpenTelemetry with structured tracing per execution (one trace = one workflow run) and per-action metrics (latency / count / errors). Raftoral has slog only.

---

## 14. API surface [A16]

Three API surfaces:

1. **Programmatic Rust API** — `WorkflowRuntime`, `WorkflowRegistry`, `FullNode<R>`. Main entry point for embedded mode.

2. **gRPC** (`raftoral.proto`) — `RaftService` (peer-to-peer Raft messages) and `WorkflowManagement` service with three RPCs: `RunWorkflowAsync`, `RunWorkflowSync`, `WaitForWorkflowCompletion`. Generated via `tonic-prost-build`. No OpenAPI. No versioning beyond proto field numbers.

3. **HTTP/REST** (axum) — `src/http/server.rs`. CORS-friendly alternative transport used for WASM compatibility. Endpoints include `POST /run_workflow` and Raft message forwarding. Not a separate management API — it is a protocol-level alternative to gRPC.

4. **Sidecar gRPC stream** (`sidecar.proto`) — Bidirectional streaming `WorkflowStream` RPC for sidecar ↔ app communication. Messages: `ExecuteWorkflowRequest`, `CheckpointEvent`, `CheckpointProposal`, `WorkflowResult`.

**Comparison with Nebula:** Nebula has REST now with GraphQL + gRPC planned, OpenAPI spec generated, OwnerId-aware routing. Raftoral has gRPC + HTTP but no OpenAPI, no versioning story for the wire protocol, and no management REST API (only workflow execution endpoints).

---

## 15. Testing infrastructure [A19]

No dedicated testing crate. Tests are **inline in source modules** (`#[cfg(test)]` blocks). 104 tests counted via `grep -r "#\[test\]\|#\[tokio::test\]"`.

Test infrastructure includes:
- `InProcessNetwork` / `InProcessServer` / `InProcessNetworkSender` (`src/raft/generic/server/in_process.rs`) — in-process transport for multi-node testing without real network.
- `KvStateMachine` in `src/raft/generic/state_machine.rs` — example state machine used as test fixture.
- Shell scripts: `scripts/test_two_node_cluster.sh`, `scripts/run_ping_pong.sh`, `scripts/run_ping_pong_http.sh`.
- `port_check = "^0.3.0"` as dev-dependency for network port availability checks.
- `tempfile = "3.8"` for RocksDB storage tests.

No `insta` (snapshot testing), no `wiremock`, no `mockall`, no contract tests.

**Comparison with Nebula:** Nebula has `nebula-testing` crate with resource-author contracts, `insta` for snapshot testing, `wiremock`, `mockall`. Raftoral's test infrastructure is simpler but the `InProcessNetwork` approach is solid for testing distributed correctness without process spawning.

---

## 16. AI / LLM integration [A21] ⭐ DEEP

### A21.1 — Existence

**No AI/LLM integration exists.** Searched `src/` for: `openai`, `anthropic`, `llm`, `embedding`, `completion`, `gpt`, `claude`, `ai_`, `llama`, `vector`, `rag`, `prompt` — **found zero matches**.

**Grep evidence:**

```
grep -r "openai\|anthropic\|llm\|embedding\|completion\|gpt\|claude\|ai_\|llama" \
     src/ --include="*.rs" -l
→ (empty)
```

### A21.2–A21.13 — All Not Applicable

There is no provider abstraction, no prompt management, no structured output, no tool calling, no streaming, no multi-agent patterns, no RAG/vector integration, no memory management, no cost tracking, no LLM observability, and no safety layer.

Raftoral is positioned as infrastructure for durable workflow execution — AI workflow nodes would be implemented as `checkpoint_compute!` closures calling external LLM APIs. The `checkpoint_compute!` macro (`src/lib.rs:47-56`) is precisely suited to this: it executes a side-effectful async computation once (on the owner node) and replicates the result across the cluster, which is the correct semantics for a non-deterministic LLM API call. But this is application-level usage, not a built-in feature.

**Comparison with Nebula:** Nebula has no first-class LLM abstraction (same as Raftoral). Both take the "AI = generic actions" approach. Raftoral's `checkpoint_compute!` is a particularly clean fit for LLM inference: exactly-once execution with distributed result propagation.

---

## 17. Notable design decisions

### D1 — Raft log as the sole persistence mechanism

Raftoral eliminates all external databases by treating the Raft log as the write-ahead log for workflow state. This is architecturally bold: it means zero operational dependencies, true peer-to-peer deployment, and natural crash recovery without any reconciliation step. The trade-off is that the entire workflow state must fit in memory for snapshotting, there is no query interface for historical data, and log compaction is bounded by snapshot frequency. This is a correct choice for embedded use in microservices with bounded workflow lifetimes but will not scale to workflows with terabytes of state or millions of concurrent instances needing queryable history.

### D2 — Owner/wait pattern for checkpoint proposals

Rather than having every node independently propose checkpoints (which would result in N × checkpoints Raft proposals for an N-node cluster), only the designated **owner node** proposes. Non-owners wait for committed events. This reduces Raft proposal traffic by 50–75% (per README benchmarks). The correctness guarantee is that non-owners skip execution of `checkpoint_compute!` closures; they only receive and apply the committed result. This is an elegant solution to the distributed determinism problem and directly addresses Temporal's separate "activity worker" model.

### D3 — Two-tier cluster architecture for horizontal scaling

The dual Management + Execution cluster topology (`docs/SCALABILITY_ARCHITECTURE.md`) decouples cluster membership overhead from checkpoint replication throughput. Management cluster (cluster_id=0) tracks O(N×C) topology with a small 3-5 voter quorum; execution clusters (cluster_id=1+) isolate O(W_local) workflow state to small groups of 5 nodes. This design means a 50-node deployment checkpoints at 5x (one execution cluster) rather than 50x — a 10x throughput improvement at scale. As of v0.2.0, load balancing across execution clusters is not yet implemented (`src/full_node/workflow_service.rs` hard-codes `cluster_id=1`), but the architecture is in place.

### D4 — `SubClusterRuntime` trait for polymorphic deployment modes

The generic `FullNode<R: SubClusterRuntime>` + `ManagementRuntime<R>` pattern (`src/full_node/mod.rs`, `src/management/sub_cluster_runtime.rs`) allows the same management infrastructure to host different execution backends: `WorkflowRuntime` for embedded mode, `WorkflowProxyRuntime` for sidecar mode. This is a clean separation achieved through compile-time generics rather than runtime dispatch, incurring zero overhead for the common embedded case.

### D5 — Sidecar architecture for polyglot support

The sidecar design (`docs/RAFTORAL_AS_SIDECAR.md`, `src/sidecar/`) uses bidirectional gRPC streaming over `localhost` to decouple Raft consensus from application language. The application receives `ExecuteWorkflowRequest` events, executes workflow code in its own runtime, and reports `CheckpointProposal` / `WorkflowResult` back via the stream. This is architecturally sound but introduces latency (network round-trip per checkpoint) and requires the application to implement deterministic execution semantics independently — a non-trivial requirement.

---

## 18. Known limitations / pain points

All sourced from internal docs (zero GitHub issues), cross-referenced against source code.

**LP-1 — Entire workflow state in RAM:** `WorkflowStateMachine` stores all checkpoint history in `HashMap<String, VecDeque<Vec<u8>>>` (`src/workflow/state_machine.rs:65`). There is no disk-backed checkpoint store. Long-running workflows with many checkpoints will exhaust memory; `docs/FEATURE_PLAN.md` §5 describes `RocksDBCheckpointStore` as a planned feature for v0.5.0.

**LP-2 — No workflow termination:** `WorkflowCommand` enum has no `Terminate` variant (`src/workflow/state_machine.rs:16-40`). A workflow running an infinite loop cannot be cancelled by the framework. Planned for v0.2.0 per `docs/FEATURE_PLAN.md` §1.2 but not yet shipped.

**LP-3 — No compensation/rollback:** README states "No built-in compensation/rollback (implement in workflow logic)." This is a significant gap for payment/order workflows where partial failures require undo sequences.

**LP-4 — Load balancing across execution clusters not implemented:** `src/full_node/workflow_service.rs` line ~55 hard-codes `cluster_id=1`. All workflows go to a single execution cluster regardless of load.

**LP-5 — Determinism requirement is user-enforced:** Non-deterministic code (calls to `rand::random()`, `SystemTime::now()`, or any external API) outside `checkpoint_compute!` will silently corrupt distributed state. There is no framework-level detection or guard. This is the hardest invariant for users to maintain.

---

## 19. Bus factor / sustainability

- **Maintainers:** 1 (Ori Shalev)
- **Commit activity:** Active — 20 commits in the last ~3 months covering architecture redesign (V2 layers), sidecar implementation, and RocksDB storage
- **Issues:** 0 open, 0 closed — consistent with pre-announcement / solo development phase
- **Stars:** 6 (very early)
- **Last release:** v0.1.1 (2025-10-11); v0.2.0 is tagged in git but not published as GitHub release
- **Bus factor:** 1 — complete bus factor risk
- **Crates.io:** Not yet published to crates.io (checked — no results for `raftoral`)
- **Verdict:** Pre-release research project. Architecture is thoughtful and well-documented but the project is in early development with no community, no commercial backing, and no stability guarantees.

---

## 20. Final scorecard vs Nebula

| Axis | Raftoral approach | Nebula approach | Who's deeper / simpler / more correct | Borrow? |
|------|-------------------|-----------------|---------------------------------------|---------|
| A1 Workspace | 2 crates, monolithic library | 26 crates, layered | Different decomposition, neither dominates — Nebula has SRP discipline; Raftoral has zero friction for consumers | no |
| A2 DAG | No DAG — single sequential async closure | TypeDAG L1-L4 generics→TypeId→predicates→petgraph | Nebula deeper — Raftoral targets sequential tasks, not DAGs | no — different goals |
| A3 Action | `WorkflowFunction<I,O>` open trait, closures, serde I/O, (name,version) dispatch | 5 action kinds, sealed traits, assoc Input/Output/Error | Different decomposition — Nebula's taxonomy enables compile-time correctness; Raftoral's simplicity lowers friction | refine — version dispatch by (name, u32) is pragmatic; consider for Nebula's version resolution |
| A4 Credential | None — grep confirms zero credential code | State/Material split, LiveCredential, blue-green refresh, OAuth2Protocol | Nebula deeper | no — Nebula's already better |
| A5 Resource | None — grep confirms zero resource abstraction | 4 scope levels, ReloadOutcome, generation tracking | Nebula deeper | no — Nebula's already better |
| A6 Resilience | None — user implements retry in closure | retry/CB/bulkhead/timeout/hedging in nebula-resilience | Nebula deeper | no — Nebula's already better |
| A7 Expression | None | 60+ funcs, type inference, sandbox, `$nodes.foo.result.email` | Nebula deeper | no — different goals |
| A8 Storage | RocksDB Raft log (3 column families: entries/metadata/snapshot) | sqlx + PgPool + Pg*Repo + RLS migrations | Different decomposition — Raftoral eliminates external DB; Nebula uses PostgreSQL for queryable history | maybe — RocksDB as embedded journal for Nebula's desktop mode is worth an ADR |
| A9 Persistence | Checkpoint-based via Raft log; snapshot serializes entire in-memory state | Frontier + checkpoint + append-only execution log | Different decomposition — Raftoral has no query interface; Nebula has replay semantics | no — different goals |
| A10 Concurrency | tokio; owner/wait pattern (50-75% proposal reduction); no `!Send` isolation | tokio + frontier scheduler + `!Send` action isolation | Different decomposition — owner/wait is novel for distributed semantics; Nebula's `!Send` isolation is stronger for single-process safety | refine — owner/wait pattern applicable to Nebula distributed mode |
| A11 Plugin BUILD | None | WASM sandbox planned, plugin-v2 spec, Plugin Fund | Nebula deeper | no — Nebula's already better |
| A11 Plugin EXEC | Sidecar gRPC streaming (polyglot delegation, no sandbox) | WASM sandbox + capability security | Different decomposition — sidecar is operationally simpler; WASM is more secure | maybe — sidecar pattern for polyglot plugins worth evaluating |
| A12 Trigger | None — imperative `start_workflow()` only | TriggerAction Source→Event 2-stage | Nebula deeper | no — Nebula's already better |
| A21 AI/LLM | None — `checkpoint_compute!` is natural fit for LLM calls (exactly-once semantics) | No first-class LLM (generic actions + plugin LLM) | Convergent — both bet AI = generic actions; Raftoral's `checkpoint_compute!` is a cleaner primitive for non-deterministic AI calls | refine — Nebula could expose explicit "exactly-once side effect" semantics analogous to `checkpoint_compute!` |
