# kotoba-workflow — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/com-junkawasaki/kotoba
- **Stars:** 2 | **Forks:** 0
- **License:** Apache 2.0 (declared in Cargo.toml; no LICENSE file found in shallow clone)
- **Last push:** 2025-12-18 (UTC)
- **Primary language:** Rust (Cargo.lock confirms; workspace root `[package] name = "eaf-ipg-runtime"`)
- **Author:** Jun Kawasaki (`jun784@example.com` — `Cargo.toml` root package)
- **Version:** workspace `0.1.22`, root package `0.2.0`
- **Governance:** Solo maintainer, no release tag found, 1 open issue, 0 closed issues

**Multi-component note:** The repo contains a large aspirational layer plan (13 layers, ~80 planned crates) but only a small subset actually exist as Rust source. Only the following crates have `Cargo.toml` and Rust code:
- `crates/010-logic/019-kotoba-jsonld` — JSON-LD processing
- `crates/010-logic/020-kotoba-os` — Kernel + Actor + Mediator (the workflow-adjacent core)
- `crates/010-logic/021-kotoba-phonosemantic` — phoneme/semantic mapping
- `crates/010-logic/022-kotoba-owl-reasoner` — OWL RDFS/Lite/DL inference
- `crates/030-storage/039-kotoba-storage-fcdb` — FCDB content-addressable storage adapter
- `crates/engidb` — sled-based Merkle DAG store
- `crates/kotoba-types` — core graph IR types (Node/Edge/Graph/ExecDag)
- `crates/kotobas-tamaki-holochain` — Holochain DHT integration experiment

The `050-workflow` layer referenced in documentation (`kotoba-workflow-core`, `kotoba-workflow`, etc.) does **not exist** as buildable crates in the current HEAD. It only exists in `_archive/251006/`. This analysis covers the actual buildable code plus references to the archived workflow design.

---

## 1. Concept positioning [A1, A13, A20]

**Author's own description (README.md line 4):**
> "Kotoba is a phonosemantic digital computing system where all computing, operating system, datastore, and self-evolution mechanisms are represented, reasoned, and executed using JSON-LD with OWL inference."

**Analyst's description (after reading code):**
Kotoba is an experimental knowledge-representation compute engine that uses JSON-LD + OWL ontologies as the universal intermediate representation for programs, processes, and data. The "workflow" component (`kotoba-os`) implements a Kernel + Actor + Mediator orchestration pattern where processes are described as JSON-LD graphs and dispatched to capability-matched actors with PROV-O provenance recording. The wider system includes a sled/FCDB storage backend, an OWL reasoning engine (via the external `fukurow` project), and ambitious aspirational layers that exist only in documentation or an archived code directory.

**Comparison with Nebula:**
Nebula is a production-targeting, strongly-typed Rust workflow engine oriented toward n8n + Temporal use cases with 26 deployable crates, sealed traits, typed DAGs, and credential management. Kotoba is an early-stage, research-oriented system targeting semantic computing: its "workflow" is actually a process-network interpreter driven by OWL ontology and JSON-LD IRs, not a DAG of typed Rust actions. The scope overlap is narrow — both execute graphs of units of work — but the abstraction levels are orthogonal.

---

## 2. Workspace structure [A1]

Workspace file: `Cargo.toml` (root), lines 2–54.

**Active workspace members (confirmed have Cargo.toml + Rust source):**
| Crate | Layer | Role |
|-------|-------|------|
| `kotoba-os` | 010/015 | Kernel + Actor + Mediator (main process-network executor) |
| `kotoba-jsonld` | 010 | JSON-LD utilities |
| `kotoba-phonosemantic` | 010 | Phoneme/semantic mapping |
| `kotoba-owl-reasoner` | 010 | OWL RDFS/Lite/DL inference (fukurow bindings) |
| `kotoba-storage-fcdb` | 030 | FCDB content-addressable storage adapter |
| `engidb` | root/crates | sled-based Merkle DAG with CID + IPLD blocks |
| `kotoba-types` | root/crates | Core IR types (Node, Edge, Graph, ExecDag) |
| `kotobas-tamaki-holochain` | experimental | Holochain DHT zome integration |
| `kotoba-cli` | 090 | CLI (archived, commented out of active build) |

**Ghost crates** (referenced in `[workspace.dependencies]` but no Cargo.toml on disk — all commented out in workspace members): `kotoba-workflow-core`, `kotoba-workflow`, `kotoba-workflow-activities`, `kotoba-workflow-operator`, `kotoba-storage` (interface crate), most deployment/service/language crates.

**Feature flags:** The `reasoning` feature on `kotoba-os` conditionally activates OWL capability matching and SHACL validation via `kotoba-owl-reasoner`. The root package has a `fcdb` feature for `engidb`. No workspace-level feature flags observed.

**Layer count vs Nebula:** Nebula has 26 active crates in a flat-ish layer with strict dependency inversion. Kotoba plans 13 layers with ~80 crates but delivers only ~8 active ones. Layer separation is aspirational not enforced by Cargo.

---

## 3. Core abstractions [A3, A17] — DEEP

### A3.1 Trait shape

The central unit-of-work abstraction is the `ActorTrait` defined at `crates/010-logic/020-kotoba-os/src/actor.rs:136-167`:

```rust
#[async_trait]
pub trait ActorTrait: Send + Sync {
    async fn perform(&self, process: &Process) -> Result<Resource>;
    fn id(&self) -> &str;
    fn capability(&self) -> &str;
    #[cfg(feature = "reasoning")]
    fn shacl_shape(&self) -> Option<&Value> { None }
    #[cfg(feature = "reasoning")]
    async fn compatibility_score(&self, process: &Process) -> f64 { ... }
}
```

**A3.1 answer:** The trait is open (any crate can implement it). It is trait-object compatible (`dyn ActorTrait`) and is stored in `Arc<dyn ActorTrait>` by the `Mediator`. There are no associated types (`Input`/`Output`/`Error` — these are fixed as `Process` and `Resource` concrete types). No GATs, no HRTBs, no typestate. The trait uses `async_trait` macro (older pattern, not native async-trait from Rust 1.75+). Two default methods exist: `shacl_shape()` and `compatibility_score()`, both feature-gated.

Compared to Nebula's sealed trait with 5 action kinds and associated `Input`/`Output`/`Error` types, Kotoba's `ActorTrait` is much simpler: one method (`perform`), no associated types, no versioning, no sealing.

### A3.2 I/O shape

Input is always `&Process` (a concrete struct with JSON-LD fields). Output is always `Resource` (another concrete struct). Both are `serde_json::Value`-heavy (`HashMap<String, Value>` for `additional` properties). No generics. No streaming. The `Process` type is defined at `crates/010-logic/020-kotoba-os/src/types.rs:12-54` with fields: `@id`, `@type`, `kotoba:label`, `kotoba:performedBy`, `kotoba:used`, `kotoba:generated`, `kotoba:next`, plus `#[serde(flatten)] additional: HashMap<String, Value>`.

Side effects are realized through the `Resource` return value wrapping PROV-O provenance metadata (`actor.rs:120-132`).

### A3.3 Versioning

No versioning mechanism. Actors are identified by IRI string (`id: String`). There is no `#[deprecated]`, no v1/v2 distinction, no migration support.

### A3.4 Lifecycle hooks

The `Kernel` has optional `on_process_start` and `on_process_end` callbacks (`Box<dyn Fn(&Process) + Send + Sync>` — `kernel.rs:29-30`). The only mandatory lifecycle method is `ActorTrait::perform`. No pre/post/cleanup/on-failure hooks per actor. No cancellation tokens. No idempotency key.

### A3.5 Resource and credential deps

No mechanism. Actors declare no resource or credential dependencies. Resources are passed implicitly via `Process.used` (a `Vec<String>` of IRIs — `types.rs:37-39`), which is untyped.

### A3.6 Retry/resilience attachment

Retry is wired globally at the `Kernel` level via `RetryExecutor` (`error.rs:134-221`). Config: `max_retries: 3`, `initial_delay_secs: 1`, `backoff_multiplier: 2.0`. There is no per-actor or per-process retry policy, no circuit breaker, no bulkhead, no timeout beyond implicit async timeout.

### A3.7 Authoring DX

No derive macros. Manual `impl ActorTrait`. The `DefaultActor` in `actor.rs:170-247` is the built-in implementation. A "hello world actor" requires `impl ActorTrait` with three methods (~15 lines). No IDE-specific support beyond standard Rust tooling.

### A3.8 Metadata

No display name, icon, or category system. Actors carry `id: String` and `capability: String` only. No i18n.

### A3.9 vs Nebula

Nebula has 5 action kinds (Process/Supply/Trigger/Event/Schedule), sealed traits, and rich associated types. Kotoba has a single `ActorTrait` variant with fixed concrete types, no sealing, no type parameters. Nebula is significantly deeper here.

---

## 4. DAG / execution graph [A2, A9, A10]

**Graph model:** Kotoba has two distinct graph models:

1. **OS/workflow level (`kotoba-os`):** Processes in a JSON-LD `Story` are linked via `kotoba:next` IRI pointers (a singly-linked list, not a DAG). `ProcessHandler::get_process_chain()` (`process_handler.rs:39-82`) resolves execution order by finding the "initial" process (one not referenced by any `next`) and following the chain. No branching or merging is supported at this level.

2. **VM/IR level (`kotoba-types` + `runtime.rs`):** The `ExecDag` type (`kotoba-types/src/lib.rs:106-112`) represents a proper DAG for low-level program execution. `lower_to_exec_dag()` in `src/runtime.rs` builds data/control/memory/time edges from a multi-layer `Graph` and executes it using Kahn's algorithm. This is the internal VM layer, not the workflow layer.

**Port typing:** None at workflow level. The low-level DAG uses `ExecEdgeKind` enum (`Data/Control/Memory/Enable/Time`) but no typed port system.

**Compile-time checks:** None. Process graph is assembled at runtime from JSON-LD deserialization.

**Scheduler:** Linear chain execution in `Kernel::start()` — sequential `for process in process_chain`. No parallel branch scheduling at OS level.

**Concurrency:** `tokio` runtime. `Arc<dyn ActorTrait>` for shared actor refs. `RwLock` for FCDB storage state. No `!Send` handling documented — all traits require `Send + Sync`.

**Comparison with Nebula:** Nebula uses petgraph with 4-level TypeDAG for compile-time and runtime soundness. Kotoba uses a linked list (`kotoba:next`) for workflow ordering and a separate low-level DAG for VM execution. No unified workflow DAG.

---

## 5. Persistence and recovery [A8, A9]

**Storage trait:** `StorageEngine` (in referenced but ghost `kotoba-storage` crate; its interface is used by `kotoba-os` and `kotoba-storage-fcdb`). Operations: `Get/Put/Delete/Exists/List/Batch` via `StoragePlan`.

**Backends:**
- **FCDB** (`kotoba-storage-fcdb/src/lib.rs`): Content-addressable `GraphDB` built on `fcdb-core`/`fcdb-cas`/`fcdb-graph`. RID-keyed node storage. Data stored as JSON-LD bytes. No native transactions (batch ops sequential).
- **EngiDB** (`engidb/src/lib.rs`): sled-backed Merkle DAG with IPLD blocks, CID addressing, branch/commit tracking.
- **Redis** adapter: Referenced in workspace deps but no active crate present.
- **RocksDB** adapter: Referenced but no active crate.

**Persistence model:** Provenance is persisted immediately on each process completion (`provenance.rs:111-128` — `persist_event()`). On `Kernel::start()` it loads saved events from storage (`provenance.rs:32-66`). No checkpoint/snapshot model. No append-only execution log or frontier-based scheduling.

**Recovery:** Partial — provenance events reload from storage, but there is no concept of "replay from checkpoint." If a process fails mid-chain, restarting `Kernel::start()` re-executes from the first process (no idempotency protection).

**Comparison with Nebula:** Nebula uses frontier-based scheduler with checkpoint recovery and append-only execution log. Kotoba has simpler CRUD-style provenance persistence with no crash recovery guarantee.

---

## 6. Credentials / secrets [A4] — DEEP

**A4.1 Existence:** There is no dedicated credential layer in the active codebase. The keyword `credential` does not appear in any non-archive Rust file.

**Grep evidence:**
```
grep -r "credential\|secret\|token\|password\|api_key" --include="*.rs" crates/ src/
# Result: 0 matches in active (non-archive) crates
```

The archived `_archive/251006/crates/020-language/023-kotoba-kotobas/src/ai_models.rs` contains `pub api_key: String` stored plainly in `AiModelConfig`, passed via `reqwest` bearer header. This is the only credential-adjacent code found, and it is in the archive, not built.

The archived `_archive/251006/crates/010-logic/015-kotoba-auth/src/lib.rs` provides an RBAC/ABAC/ReBAC authorization engine (policy evaluation, relation tuples) but no secret storage.

**A4.2-A4.9:** Not applicable. There is no at-rest encryption, no `secrecy::Secret<T>`, no zeroize, no OAuth2 support, no blue-green refresh, no State/Material split. All absent.

**Comparison with Nebula:** Nebula has a dedicated `nebula-credential` crate with State/Material split, LiveCredential `watch()`, blue-green refresh, and `OAuth2Protocol` blanket adapter. Kotoba has nothing comparable in active code.

---

## 7. Resource management [A5] — DEEP

**A5.1 Existence:** No dedicated resource abstraction in active crates. The `Resource` type in `kotoba-os/src/types.rs:61-85` is a JSON-LD data object (a named entity with IRI and properties), not a pooled infrastructure resource (e.g., DB connection pool, HTTP client).

**Grep evidence:**
```
grep -r "pool\|Pool\|ResourceManager\|resource_manager\|lifecycle" --include="*.rs" crates/ src/
# Result: 0 matches in active (non-archive) crates
```

**A5.2-A5.8:** All absent. No scope levels, no init/shutdown hooks, no hot-reload, no generation tracking, no credential notification. Resources are ephemeral result containers, not managed infrastructure objects.

**Comparison with Nebula:** Nebula has `nebula-resource` with 4 scope levels (Global/Workflow/Execution/Action), `ReloadOutcome` enum, and `on_credential_refresh` hook. Kotoba has no resource management layer at all.

---

## 8. Resilience [A6, A18]

**Retry:** `RetryExecutor` in `error.rs:134-221`. Configurable via `RetryConfig` (`max_retries`, `initial_delay_secs`, `max_delay_secs`, `backoff_multiplier`). Default: 3 retries, 1s initial, 2x backoff, 60s cap.

**Error classification:** `ErrorCategory` enum at `error.rs:13-24`: `Transient/Permanent/System/Validation/Network`. `ErrorContext::from_error()` maps `KotobaOsError` variants to categories. Only `Transient` and `Network` categories are marked retryable by default.

**Escalation:** `ErrorEscalator` (`error.rs:237-310`) escalates based on retry count: `None → Warning → Error → Critical`. Critical level logs but does not trigger external alerts (comment: "In a production system, this would trigger alerts/notifications").

**Missing:** No circuit breaker, no bulkhead, no timeout mechanism, no hedging, no `ErrorClassifier` abstraction comparable to Nebula's.

**Error types:** `KotobaOsError` uses `thiserror` (`lib.rs:83-111`). Flat enum with 7 variants. No `ErrorClass` categorization trait. Root package uses `anyhow` directly.

**Comparison with Nebula:** Nebula has `nebula-resilience` with retry/CB/bulkhead/timeout/hedging + `ErrorClassifier`. Kotoba has retry + simple escalation only; no circuit breaker or bulkhead.

---

## 9. Expression / data routing [A7]

**Active crates:** No expression DSL exists in active code. `src/dsl.rs` exists in the root package but its content is HTMX/UI-oriented (Kotoba JSON-LD UI IR), not a data routing DSL.

**Jsonnet:** `crates/020-language/023-kotoba-jsonnet` is referenced in `LAYER_ARCHITECTURE.md` and workspace deps but the corresponding crate directory under `crates/020-language/` is not present in the shallow clone or active workspace.

**KotobaScript (kotobas):** Planned in Layer 020 but not buildable. The archive (`_archive/251006/crates/020-language/023-kotoba-kotobas/`) contains an AI-oriented DSL with `ai_chains.rs`, `ai_models.rs`, `ai_parser.rs`, but it is not part of the active workspace.

**DSL for workflow:** The "expression language" for the workflow layer is effectively JSON-LD IRIs and SPARQL fragments used in the `EvolutionEngine` (`evolution.rs:152-215`). These are raw strings, not a typed DSL.

**Comparison with Nebula:** Nebula has a 60+ function expression engine with type inference and sandbox. Kotoba has no comparable expression engine in active code.

---

## 10. Plugin / extension system [A11] — DEEP

### 10.A — Plugin BUILD process

**A11.1-A11.4:** No plugin system exists in active code.

**Grep evidence:**
```
grep -r "plugin\|Plugin" --include="*.rs" crates/ src/ -l
# Result: 0 matches in active (non-archive) crates
```

No manifest format, no toolchain, no registry, no capability declaration for plugins.

**WASM:** `wasmtime = "0.35"` appears in workspace `[workspace.dependencies]` (`Cargo.toml:46`) but no crate in the active workspace uses it. The `wasm-bindgen` and `wasm-bindgen-futures` are used in the root package binary for browser-targeting (HTMX UI transpilation), not for plugin sandboxing.

`kotobas-tamaki-holochain` uses WASM through the Holochain HDK (`wasm_error!` macro — `zome.rs:20-118`), but this is Holochain zome compilation, not a plugin extension system.

### 10.B — Plugin EXECUTION sandbox

**A11.5-A11.9:** No sandbox. No plugin loader. No capability enforcement for external code.

**Comparison with Nebula:** Nebula has a WASM sandbox target (wasmtime), plugin-v2 spec, and Plugin Fund commercial model. Kotoba has no plugin system.

---

## 11. Trigger / event model [A12] — DEEP

**A12.1 Trigger types:** No trigger system in active code. The archived workflow (`_archive/251006/crates/010-logic/010-kotoba-types/crates/300-workflow/kotoba-workflow/src/lib.rs:105-113`) defines a `Trigger` enum with three variants:
```rust
pub enum Trigger {
    Schedule(String), // cron expression
    Event(String),    // event name
    Manual,
}
```
This is archived code, not built.

**Grep evidence (active code):**
```
grep -r "trigger\|webhook\|schedule\|cron" --include="*.rs" crates/ src/ -l | grep -v _archive
# Found: src/main.rs (HTMX hx-trigger for UI only), src/realtime.rs (HTMX event helpers)
```
No workflow trigger machinery exists in active crates.

**A12.2 Webhook:** Not implemented. No URL allocation, no HMAC verification.

**A12.3 Schedule:** Not implemented. Cron string present in archived `Trigger::Schedule(String)` but no executor.

**A12.4 External events:** Not implemented.

**A12.5-A12.8:** All absent. No reactive model, no fan-out, no two-stage Source→Event pattern.

**Comparison with Nebula:** Nebula has `TriggerAction` with `Input = Config` / `Output = Event`, two-stage Source normalization, and a structured trigger dispatch model. Kotoba has a planned but unimplemented trigger enum in archived code.

---

## 12. Multi-tenancy [A14]

No multi-tenancy in active code. No tenant isolation, RBAC, SSO, or SCIM. The archived `kotoba-auth` (`_archive/251006/crates/010-logic/015-kotoba-auth/`) implements an ABAC/ReBAC policy engine (allow/deny decisions, relation tuples), but it is not active.

---

## 13. Observability [A15]

**Tracing:** `tracing` crate used throughout (`kotoba-os/src/kernel.rs`, `mediator.rs`, `provenance.rs`). `info!/warn!/error!` macros at key lifecycle points (process start, actor selection, provenance persistence). No structured span attributes or trace IDs tied to executions.

**Metrics:** None. No counters, histograms, or latency metrics exposed.

**OpenTelemetry:** Not configured. `tracing-subscriber` in workspace deps but only `fmt` subscriber used.

**Comparison with Nebula:** Nebula uses OpenTelemetry with one trace per execution. Kotoba uses basic `tracing` logs with no metrics or OTel integration.

---

## 14. API surface [A16]

**Programmatic API:** `kotoba-os` crate exports `Kernel`, `ActorTrait`, `DefaultActor`, `Mediator`, `Provenance`, etc.

**Network API:** The root package binary (`src/server.rs`) embeds an `axum` HTTP server for the TODO demo app. No workflow API endpoints. No OpenAPI spec. No gRPC.

**Versioning:** None. Single `0.1.22` workspace version.

**Comparison with Nebula:** Nebula has REST API + planned GraphQL/gRPC with OpenAPI. Kotoba has a demo HTTP server with no workflow network API.

---

## 15. Testing infrastructure [A19]

**Test count (active crates):** `kotoba-os/src/lib.rs` has 4 integration-style `#[tokio::test]` tests. `kotoba-os/src/error.rs` has 3 unit tests. `kotoba-storage-fcdb/src/lib.rs` has 2 integration tests. `kotobas-tamaki-holochain/tests/` has `agent_communication_tests.rs`, `integration_tests.rs`, `unit_tests.rs`. Total: ~15-20 tests across active crates.

**No public testing utilities crate.** No contract tests. No insta snapshots. No wiremock.

**Dev dependencies:** `criterion` for benchmarks (disabled), `proptest`, `quickcheck`, `rand`, `uuid` in root.

**Comparison with Nebula:** Nebula has `nebula-testing` crate with contract tests and public test utilities. Kotoba has modest inline tests.

---

## 16. AI / LLM integration [A21] — DEEP

**A21.1 Existence in active code:** No AI/LLM integration in active crates.

**Grep evidence:**
```
grep -r "llm\|openai\|anthropic\|embedding\|completion\|LLM" --include="*.rs" crates/ src/ | grep -v _archive
# Result: src/main.rs:514 — "TODO: Implement completion logic when full query support is available"
# (refers to Todo item completion, not LLM completion)
```

**A21.1-A21.13:** All absent in active code. No provider abstraction, no prompt management, no structured output, no tool calling, no streaming, no multi-agent patterns, no RAG/vector, no memory management, no cost tracking, no observability for LLM calls, no safety filtering.

**Archived AI integration (`_archive/251006/`):** The archive contains `kotoba-kotobas` with dedicated AI modules:
- `ai_models.rs`: `AiModels` manager, `AiProvider` enum (OpenAI/Anthropic/Google), `AiModelConfig` with plaintext `api_key: String`, `AiMessage`/`AiResponse`/`AiUsage` types, `call_openai()` implementation (functional), `call_anthropic()` stub (returns `Err("not yet implemented")`), `call_google()` stub.
- `ai_chains.rs`: Referenced but not read in detail.
- `ai_parser.rs`: Referenced but not read in detail.

The archived implementation stores API keys in plaintext struct fields with no secrecy crate, no zeroize, no vault integration. Token counting is tracked via `AiUsage` struct. No streaming, no RAG, no multi-agent.

**A21.13 vs Nebula+Surge:** Nebula has no first-class LLM (strategic bet: AI = generic actions + plugin LLM client). Kotoba attempted a first-class multi-provider LLM client (archived), but abandoned it before completing. The archived design is over-coupled: API keys in plain structs, only OpenAI actually wired up, Anthropic/Google stubbed.

---

## 17. Notable design decisions

**D1: JSON-LD as universal IR**
All process definitions, actor registrations, provenance records, and capability declarations use JSON-LD (`serde_json::Value` with `@context`/`@type`/`@id` fields). This enables semantic interoperability across the system but at the cost of Rust's type system — everything flowing through `Value` loses compile-time guarantees. Nebula's approach (sealed traits + associated types) is the opposite end of this spectrum.

**D2: OWL reasoning as capability matching**
The `Mediator` has three selection strategies: `Direct` (IRI match), `Capability` (string match), and `ShaclSemantic` (OWL subsumption via fukurow). The reasoning path is optional (`feature = "reasoning"`) and its implementation admits it is incomplete: `capability.rs:281-283` contains `// TODO: Implement OWL subsumption check`. The OWL integration is designed but not operative.

**D3: Provenance as first-class concern**
Every process execution records a `ProvenanceEvent` in PROV-O format with `prov:wasGeneratedBy`, `prov:wasAssociatedWith`, `prov:used`, `prov:generated`. This is immediately persisted to storage. This is a differentiating design choice that Nebula does not have as a first-class layer.

**D4: Self-evolution loop**
`evolution.rs` implements a `Shape → Process → Provenance → Pattern Discovery → Shape Refinement` loop using SPARQL queries over provenance events to discover co-occurrence patterns and actor performance trends, then refine SHACL shapes. This is ambitious but incomplete (SPARQL execution relies on unimplemented features in `execute_sparql`).

**D5: Content-addressable storage everywhere**
`engidb` uses sled + Blake3 + CID (IPLD/CBOR) for all data. `kotoba-storage-fcdb` wraps fcdb's CAS+GraphDB. This aligns with the Holochain influence and provides deduplication and verifiability but complicates simple CRUD patterns.

**D6: Aspirational architecture gap**
The `LAYER_ARCHITECTURE.md` and `README.md` describe 13 layers and ~80 crates, but only 8 crates exist. The workspace `Cargo.toml` comments out over 30 planned members with `# No Cargo.toml`. This creates a significant gap between documented design and actual implementation.

---

## 18. Known limitations / pain points

**GitHub issue count:** 1 open, 0 closed. Far below the 100-issue threshold for mandatory citation.

- **Issue #2 (open):** "Implement 'eval' subcommand in 'kotoba-cli' and complete 'kotoba-jsonnet' evaluation features" — the CLI and Jsonnet evaluator are blocked by missing crate implementations.

**From code comments:**
- `mediator.rs:181`: `// TODO: Implement SHACL-based semantic matching` — capability matching falls back to string equality.
- `capability.rs:281`: `// TODO: Implement OWL subsumption check` — OWL reasoning for capability matching not operative.
- `kernel.rs:137`: `// In strict mode, this would return an error` — SHACL validation warnings are swallowed.
- `fcdb/src/lib.rs:235`: `// Simplified - full implementation would handle all conditions` — query conditions mostly unimplemented.
- All benchmarks in root `Cargo.toml` are commented out due to compilation issues (conflicting indexmap versions).

**Sustainability:** Solo maintainer, 2 stars, no release tags, extensive archived code base (the `_archive` directory is larger than the active code). Commit velocity shows multi-month gaps between meaningful changes.

---

## 19. Bus factor / sustainability

- **Maintainers:** 1 (Jun Kawasaki)
- **Stars:** 2 | **Forks:** 0
- **Open issues:** 1 | **Closed issues:** 0
- **Last activity:** December 2025 (per `pushedAt` field)
- **Releases:** 0
- **Architecture:** Solo-designed, no external contributors in recent history
- **Risk:** High. No documentation of external users, no release cadence, extensive aspirational-only code. The `_archive` directory suggests significant architectural pivot already occurred. Bus factor = 1.

---

## 20. Final scorecard vs Nebula

| Axis | Kotoba approach | Nebula approach | Verdict | Borrow? |
|------|----------------|-----------------|---------|---------|
| A1 Workspace | 8 active crates out of ~80 planned; 13-layer aspirational architecture; multiple ghost crates | 26 crates, layered, all buildable | Nebula deeper — Kotoba has high aspiration/delivery gap | no |
| A2 DAG | Linked list via `kotoba:next` for workflow; separate ExecDag for low-level VM; no typed ports | TypeDAG L1-L4 with compile-time checks | Nebula deeper — Kotoba lacks typed DAG at workflow level | no |
| A3 Action | Single `ActorTrait` (open, concrete types, no associated types, no sealing, no versioning); DefaultActor; async_trait macro | 5 action kinds, sealed traits, associated Input/Output/Error, versioning, derive macros | Nebula significantly deeper | no |
| A4 Credential | Absent (no active credential layer; archived ai_models.rs has plaintext api_key) | State/Material split, LiveCredential, blue-green refresh, OAuth2Protocol | Nebula deeper | no |
| A5 Resource | Absent (Resource = JSON-LD data object, not pooled infrastructure) | 4 scope levels, ReloadOutcome, generation tracking, on_credential_refresh | Nebula deeper | no |
| A6 Resilience | Retry with exponential backoff + ErrorCategory + ErrorEscalator; no CB/bulkhead/timeout | retry/CB/bulkhead/timeout/hedging + ErrorClassifier | Nebula deeper | no |
| A7 Expression | Absent in active code; archived KotobaScript/Jsonnet planned | 60+ funcs, type inference, sandbox | Nebula deeper | no |
| A8 Storage | FCDB (CAS+GraphDB) + sled/EngiDB; Port/Adapter pattern; no SQL | sqlx + PgPool + RLS + Pg*Repo | Different decomposition — Kotoba content-addressable (interesting), Nebula relational | refine — CAS provenance idea worth exploring |
| A9 Persistence | Provenance event CRUD + PROV-O format; no checkpoint/frontier; no crash recovery | Frontier + checkpoint + append-only | Nebula deeper for recovery; Kotoba's PROV-O provenance is richer | maybe — PROV-O provenance recording idea |
| A10 Concurrency | tokio + Arc<dyn ActorTrait> + RwLock; sequential chain execution; no parallel scheduling | tokio + frontier scheduler + !Send isolation | Nebula deeper; Kotoba sequential only at workflow level | no |
| A11 Plugin BUILD | Absent; WASM in deps but unused for plugins | WASM sandbox planned, plugin-v2 spec | Nebula ahead (even if also planned) | no |
| A11 Plugin EXEC | Absent; no sandbox loader | WASM sandbox + capability security | Nebula ahead | no |
| A12 Trigger | Absent in active code; `Trigger` enum archived (Schedule/Event/Manual) | TriggerAction Source→Event 2-stage | Nebula deeper | no |
| A21 AI/LLM | Absent in active code; archived `AiModels` manager (OpenAI functional, Anthropic/Google stubbed; plaintext API key) | No first-class LLM yet; generic actions + plugin bet | Different decomposition — neither has production AI integration; Kotoba attempted and abandoned | no |

---

## Appendix: DeepWiki augmentation

Three DeepWiki queries were attempted for `com-junkawasaki/kotoba`:
1. "What is the core trait hierarchy for actions/nodes/activities?" — **Error: Repository not found. Visit https://deepwiki.com to index it.**
2. "How is workflow state persisted and recovered after crash?" — **Error: Repository not found.**
3. "What is the credential or secret management approach?" — **Error: Repository not found.**

Three consecutive failures — DeepWiki augmentation stopped per §3.6 3-fail-stop protocol. All architectural claims in this document are based on direct code inspection.
