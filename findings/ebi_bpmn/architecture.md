# ebi_bpmn — Architectural Decomposition

## 0. Project metadata

- **Repo:** https://github.com/BPM-Research-Group/Ebi_BPMN
- **Stars:** 2 (as of 2026-04-26)
- **Forks:** 0
- **Last push:** 2026-04-13
- **Created:** 2026-02-24 (young project — ~2 months old)
- **License:** MIT OR Apache-2.0
- **Maintainers:** 1 (sleemans — single contributor per `gh api contributors`)
- **Governance:** academic / research group (BPM-Research-Group on GitHub)
- **Published version:** 0.0.46 on crates.io (pre-release versioning)
- **Homepage:** https://ebitools.org
- **GitHub issues:** 0 open, no closed issues found (gh issue list returned nothing)

## 1. Concept positioning [A1, A13, A20]

**Author's own description (README.md line 1–4):**
> "A BPMN library for Rust. Contains a parser, a data structure and a writer. For now, this crate focuses on the behaviour of BPMN models; not on the data or resource perspectives."

**My description (after reading code):**
A research-grade Rust library for parsing, constructing, structurally validating, and executing the behavioral semantics of BPMN 2.0 models, with an extension for stochastic BPMN (weighted probabilistic transition selection). It is a library crate, not a runtime orchestrator — it models BPMN execution as a token-game state-space traversal, not as a production workflow engine.

**Comparison with Nebula:**
Ebi_BPMN is not a workflow engine at all — it is a behavioral model library. It has no scheduler, no persistence, no credential layer, no tenancy, no API surface, and no deployment modes. Its purpose is to enable process-mining algorithms (in the parent `ebi` crate) to reason about BPMN model behavior mathematically. Nebula is a full production orchestration engine that draws inspiration from BPMN semantics; Ebi_BPMN implements those semantics formally and directly. The overlap is conceptual, not architectural.

## 2. Workspace structure [A1]

Ebi_BPMN is a **single-crate** Rust library (`Cargo.toml` root, no workspace). There is one library target and no separate workspace members. The crate declares `crate-type = ["cdylib", "lib"]` (`Cargo.toml` line 21), which enables it to be loaded as a C dynamic library from other runtimes (e.g., for Java interop through JNI or for use in a web context).

**Source module layout:**

| Module | Role |
|--------|------|
| `business_process_model_and_notation.rs` | Root struct `BusinessProcessModelAndNotation` |
| `stochastic_business_process_model_and_notation.rs` | Wrapper adding stochastic semantics |
| `element.rs` | `BPMNElement` enum — 22 variants |
| `elements/` | Per-element structs (task, gateway, event, etc.) |
| `traits/` | Core traits: `BPMNObject`, `Processable`, `Searchable`, `Startable`, `Transitionable`, `Writable` |
| `marking.rs` | Token-based marking state: `BPMNMarking`, `BPMNRootMarking`, `BPMNSubMarking`, `Token` |
| `semantics.rs` | `execute_transition`, `get_enabled_transitions`, `get_initial_marking` |
| `parser/` | XML parser (quick-xml) — 22 tag-specific modules |
| `writer/` | XML writer (quick-xml) — 23 element-specific modules |
| `creator.rs` | Programmatic BPMN model builder `BPMNCreator` |
| `structure_checker.rs` | Structural validation of BPMN models |
| `partially_ordered_run.rs` | Hypergraph partial-order semantics for SBPMN |

**External dependencies (Cargo.toml):**
- `ebi_derive` (0.2.10) — derive macros from the broader Ebi ecosystem
- `ebi_activity_key` (0.0.2) — activity label interning
- `ebi_arithmetic` (0.3.11) — arithmetic with exact/approximate modes (feature flags)
- `anyhow` (1.0.102) — error propagation
- `strum` / `strum_macros` (0.28) — enum string utilities
- `quick-xml` (0.39.2) — XML parse/write
- `bitvec` (1.0.1) — bit-level transition enablement vectors
- `layout-rs` (0.1.3) — visualization layout for partial-order runs
- `itertools` (0.14)

**Feature flags (Cargo.toml lines 11–21):**
- `exactarithmetic` / `eexactarithmetic` — enables exact rational arithmetic via `ebi_arithmetic`
- `approximatearithmetic` / `eapproximatearithmetic` — floating-point approximation path
- `testactivities` — enables `TestActivityKey` trait for tests

**Nebula comparison:** Nebula uses a 26-crate layered workspace with strict domain separation. Ebi_BPMN is a single-crate library with no deployment separation. No feature-flag-controlled execution modes analogous to Nebula's desktop/serve/cloud split.

## 3. Core abstractions [A3, A17] — DEEP (A3.1–A3.9)

### A3.1 — Trait shape

The unit of behavior is not an "action" or "node" in the Nebula sense — it is a BPMN element type. The trait system has six traits (`src/traits/`):

1. **`BPMNObject`** (`src/traits/objectable.rs:8`) — base identity trait. Methods: `global_index()`, `local_index()`, `id()`, `activity()`, `is_end_event()`, `incoming_sequence_flows()`, `outgoing_sequence_flows()`, `incoming_message_flows()`, `outgoing_message_flows()`, `can_start_process_instance()`, `outgoing_message_flows_always_have_tokens()`, `outgoing_messages_cannot_be_removed()`, `incoming_messages_are_ignored()`, `can_have_incoming_sequence_flows()`, `can_have_outgoing_sequence_flows()`. **No associated types.** Object-safe (`&dyn Processable` used throughout, which supertraits `BPMNObject`).

2. **`Transitionable`** (`src/traits/transitionable.rs:15`) — behavioral semantics trait. Methods: `number_of_transitions()`, `enabled_transitions()`, `execute_transition()`, `transition_activity()`, `transition_debug()`, `transition_probabilistic_penalty()`, `transition_2_consumed_tokens()`, `transition_2_produced_tokens()`. **No associated types.** Not sealed — public trait. Returns `anyhow::Result<T>` throughout.

3. **`Processable`** (`src/traits/processable.rs:12`) — container trait for elements that hold child elements (processes, sub-processes). Supertraits: `BPMNObject + Debug`. Methods: `elements_non_recursive()`, `sequence_flows_non_recursive()`, `to_sub_marking()`, `is_sub_process()`, plus default methods `sequence_flow_index_2_source()` and `sequence_flow_index_2_target()`. Object-safe: used as `&dyn Processable` in many call sites.

4. **`Searchable`** (`src/traits/searchable.rs`) — recursive lookup (not shown in detail but used as impl on `Vec<BPMNElement>`).

5. **`Startable`** (`src/traits/startable.rs:9`) — determines how a process initiates (`unconstrained_start_events_without_recursing`, `start_elements_without_recursing`, `end_events_without_recursing`, `initiation_mode`).

6. **`Writable`** (`src/traits/writable.rs`) — XML serialization.

**Sealed?** No. All traits are `pub` and can be implemented by external crates. There is no sealing mechanism equivalent to Nebula's sealed action kinds.

**GAT / HRTB / typestate?** None. The trait design is straightforward object-safe Rust without GATs, HRTBs, or typestate patterns.

**dyn compatibility:** Yes — `Processable` and `BPMNObject` are used as trait objects in many call sites (e.g., `parent: &dyn Processable`, `bpmn: &BusinessProcessModelAndNotation` which implements `Processable`).

### A3.2 — I/O shape

There is no Input/Output abstraction in the sense of Nebula's `ProcessAction::Input` / `ProcessAction::Output`. Ebi_BPMN does not execute "actions" that consume typed inputs and produce typed outputs. Instead, it executes transitions on a `BPMNMarking` (token state). The "input" to a transition is the current marking (borrowed); the "output" is the mutated marking plus lists of consumed and produced `Token` values (`src/marking.rs:314`). There is no streaming output, no side-effects model beyond marking mutation, and no type-erased data payload — BPMN elements do not carry data variables at all (the README explicitly states "this crate focuses on the behaviour of BPMN models; not on the data or resource perspectives").

### A3.3 — Versioning

No versioning concept. Element types are identified by the `BPMNElement` enum variants (`src/element.rs:34`), not by name+version or type-tag. No `#[deprecated]` annotations visible. The crate version (0.0.46) advances rapidly (46 patch versions in ~2 months) but there is no migration path for workflow definitions — the format is always current-version BPMN 2.0 XML.

### A3.4 — Lifecycle hooks

The "lifecycle" of a BPMN element in this library consists of: parse (via XML importer), structural-correctness check, and then repeated `enabled_transitions()` + `execute_transition()` calls. There is no pre/post/cleanup/on-failure hook system. Execution is fully synchronous — no async anywhere in the library (no `async fn` or Tokio). Cancellation points: none. Idempotency: the library itself is stateless in the sense that the user manages the `BPMNMarking` and calls transition execution manually.

### A3.5 — Resource and credential dependencies

None. There is no mechanism to declare that an element needs a DB pool, credential, HTTP client, or any runtime resource. The README explicitly says the crate does not address the data or resource perspectives of BPMN.

### A3.6 — Retry and resilience attachment

None. No retry logic, circuit breaker, or timeout mechanism. Errors are propagated via `anyhow::Result<T>` but there is no error classification or resilience layer.

### A3.7 — Authoring DX

Element construction is done programmatically through `BPMNCreator` (`src/creator.rs`). A "hello world" BPMN model requires calling `BPMNCreator::new()`, `add_process()`, then element-specific `add_*` methods, and finally `to_bpmn()`. Alternatively, BPMN XML files are parsed via `BusinessProcessModelAndNotation::import_from_reader()`. No derive macro for user-defined element types; the `ebi_derive` crate provides `#[derive(ActivityKey)]` for the internal `ActivityKey` interning only. No IDE plugin or code generation tool exists.

### A3.8 — Metadata

Each `BPMNElement` carries `id: String` (XML id), `global_index: GlobalIndex` (a `(usize, ())` tuple used as a unique address), and `local_index: usize` (position within parent's child list). Named activities (`Activity` from `ebi_activity_key`) are carried only on task-like elements. There is no display name, icon, category, description, or i18n support. Metadata is compile-time only (struct fields on concrete element types), not queryable at runtime beyond what `BPMNObject` exposes.

### A3.9 — Comparison with Nebula's 5 action kinds

Nebula distinguishes 5 sealed action kinds (Process/Supply/Trigger/Event/Schedule) with associated `Input`/`Output`/`Error` types and derive macros for authoring. Ebi_BPMN has a flat `BPMNElement` enum with 22 variants covering all BPMN 2.0 element types but no kind-level abstraction analogous to Nebula's. The abstraction level is different: Nebula's "actions" are opaque user-defined execution units; Ebi_BPMN's elements are fixed BPMN-spec types. Ebi_BPMN has no user-extensible element abstraction.

## 4. DAG / execution graph [A2, A9, A10]

### Graph model

The BPMN process is not modeled as a DAG (directed acyclic graph). BPMN processes are directed graphs that allow cycles (loops), which is why the execution model is a token-game / Petri-net analogy. The structure is:

- `BusinessProcessModelAndNotation.elements: Vec<BPMNElement>` — flat list of elements (processes, gateways, tasks, events) at the top level (`src/business_process_model_and_notation.rs:53`)
- `BPMNProcess.elements: Vec<BPMNElement>` — child elements within a process or sub-process
- `BPMNProcess.sequence_flows: Vec<BPMNSequenceFlow>` — directed arcs between child elements
- `BPMNRootMarking.message_flow_2_tokens: Vec<u64>` — inter-pool message flow tokens (`src/marking.rs:177`)

Connectivity is resolved by local index (position in parent's `elements` Vec) and global index (unique across entire model). There is no petgraph or other graph library used — the graph is implicit in the sequence flow `source_local_index` / `target_local_index` fields.

### Port typing

No port typing. Sequence flows connect elements by index without type constraints. There is structural validation (`is_structurally_correct()`) checking basic invariants (e.g., exclusive gateways need at least one incoming arc, message flows must cross pools) but no compile-time type-safe connectivity.

### Compile-time checks

None for graph topology. All validity is checked at parse time or via explicit `is_structurally_correct()` calls.

### Concurrency model

All execution is synchronous and single-threaded. The "concurrent" aspects of BPMN (parallel gateways producing multiple tokens, inter-pool message flows) are modeled through the marking's token counters, not through OS threads or async tasks. The `PartiallyOrderedRun` (`src/partially_ordered_run.rs`) computes a partial-order hypergraph over an execution trace, enabling formal reasoning about concurrency without actual parallel execution.

**Nebula comparison:** Nebula's TypeDAG has four levels of type-safety enforcement (L1 static generics → L2 TypeId → L3 predicates → L4 petgraph). Ebi_BPMN has no type-safe DAG — connections are runtime index-based, with structural validation only. Nebula's frontier scheduler runs on tokio with work-stealing. Ebi_BPMN has no scheduler and no async runtime.

## 5. Persistence and recovery [A8, A9]

**No persistence layer exists.** There is no database, no storage dependency, no migration system, and no serialization format beyond BPMN XML import/export. The library is purely in-memory: the caller creates a `BusinessProcessModelAndNotation`, obtains an initial `BPMNMarking` via `get_initial_marking()`, and drives execution by calling `execute_transition()` repeatedly. If the process crashes, no state is saved. The `PartiallyOrderedRun` struct records a trace of executed transitions in memory but does not checkpoint it.

Grepped for persistence keywords: searched `persistence`, `checkpoint`, `journal`, `replay`, `sqlx`, `postgres`, `database` — found nothing in `src/`.

**Nebula comparison:** Nebula uses frontier-based checkpointing with an append-only execution log backed by PostgreSQL (sqlx + PgPool). Ebi_BPMN has no equivalent.

## 6. Credentials / secrets [A4] — DEEP (A4.1–A4.9)

### A4.1 — Existence
**No credential layer.** Ebi_BPMN does not handle credentials, secrets, or authentication of any kind.

**Grep evidence:** Searched `src/` for `credential`, `secret`, `auth`, `oauth`, `password` (case-insensitive) — zero matches in any source file (grep returned no matches).

### A4.2–A4.9
All A4 sub-axes: not applicable. No storage, no in-memory protection, no lifecycle, no OAuth2, no composition, no scope, no type safety for credentials. This is a pure behavioral modeling library.

**Nebula comparison:** Nebula has a dedicated `nebula-credential` crate with State/Material split, LiveCredential watch(), blue-green refresh, and OAuth2Protocol blanket adapter. Ebi_BPMN has nothing analogous.

## 7. Resource management [A5] — DEEP (A5.1–A5.8)

### A5.1 — Existence
**No resource abstraction.** The library does not manage DB pools, HTTP clients, caches, or any external resource.

**Grep evidence:** Searched `src/` for `resource`, `pool`, `client`, `cache`, `handle`, `scope` — none returned relevant matches related to resource management.

### A5.2–A5.8
All A5 sub-axes: not applicable. The only "resource" in the codebase is the `BusinessProcessModelAndNotation` struct itself, which is immutable after construction (transitions only mutate the `BPMNMarking` passed by the caller). No scoping, no hot-reload, no sharing, no backpressure.

**Nebula comparison:** Nebula has a dedicated `nebula-resource` crate with 4 scope levels, `ReloadOutcome` enum, generation tracking, and credential-refresh hooks. Ebi_BPMN has nothing analogous.

## 8. Resilience [A6, A18]

**No resilience layer.** There is no retry logic, circuit breaker, timeout, bulkhead, or hedging. Error classification does not exist — all errors are `anyhow::Error` (see §18 for detail on error handling).

Grepped for `retry`, `backoff`, `circuit`, `timeout`, `bulkhead` in `src/` — zero matches.

**Nebula comparison:** Nebula has a dedicated `nebula-resilience` crate with all five patterns plus `ErrorClassifier`. Ebi_BPMN has none.

## 9. Expression / data routing [A7]

**No expression engine.** BPMN 2.0 defines conditional sequence flows (e.g., XOR gateway outgoing arcs have conditions). This library ignores all conditions — the README states "For now, this crate focuses on the behaviour of BPMN models; not on the data or resource perspectives." Exclusive gateway branches are treated as non-deterministic choices enumerated by the transition system, leaving selection to the caller (or to probabilistic weights in the stochastic extension).

Grepped for `expression`, `condition`, `eval`, `jexl`, `jsonpath` in `src/` — zero matches.

**Nebula comparison:** Nebula has a 60+ function expression engine with a sandboxed DSL and type inference. Ebi_BPMN deliberately omits this as outside its behavioral-semantics scope.

## 10. Plugin / extension system [A11] — BUILD and EXEC

### 10.A — Plugin BUILD process (A11.1–A11.4)

**No plugin system.** There is no plugin architecture, no manifest format, no build toolchain, and no registry.

**Grep evidence:** Searched for `plugin`, `extension`, `manifest` in `src/` and `Cargo.toml` — zero matches.

The `cdylib` crate type in `Cargo.toml:21` is not a plugin mechanism — it enables the crate itself to be loaded as a dynamic library from non-Rust code (e.g., from Java via JNI). It does not allow users to author plugins for Ebi_BPMN.

The `Cargo.lock` file includes `wasm-bindgen` transitive dependencies — these come from the `layout-rs` crate (visualization dependency), not from any WASM plugin system in Ebi_BPMN itself.

**A11.1–A11.4:** Not applicable.

### 10.B — Plugin EXECUTION sandbox (A11.5–A11.9)

Not applicable for the same reasons — no plugin execution framework exists.

**A11.9 vs Nebula:** Nebula targets a WASM sandbox with capability-based security and a Plugin Fund commercial model. Ebi_BPMN has no plugin system of any kind.

## 11. Trigger / event model [A12] — DEEP (A12.1–A12.8)

Ebi_BPMN implements the BPMN 2.0 event model at the specification level, not as a runtime trigger system. The supported BPMN event elements are (from `src/element.rs:34-57`):

| BPMN event type | Implementation |
|-----------------|----------------|
| `StartEvent` (none) | `BPMNStartEvent` |
| `StartEvent` (message) | `BPMNMessageStartEvent` |
| `StartEvent` (timer) | `BPMNTimerStartEvent` |
| `IntermediateCatchEvent` (none) | `BPMNIntermediateCatchEvent` |
| `IntermediateCatchEvent` (message) | `BPMNMessageIntermediateCatchEvent` |
| `IntermediateCatchEvent` (timer) | `BPMNTimerIntermediateCatchEvent` |
| `IntermediateThrowEvent` (none) | `BPMNIntermediateThrowEvent` |
| `IntermediateThrowEvent` (message) | `BPMNMessageIntermediateThrowEvent` |
| `EndEvent` (none) | `BPMNEndEvent` |
| `EndEvent` (message) | `BPMNMessageEndEvent` |
| `EventBasedGateway` | `BPMNEventBasedGateway` |

### A12.1 — Trigger types in operational sense
None. Ebi_BPMN does not connect to external systems. Timer events are modeled as transitions in the state space but do not fire based on wall-clock time. Message events are modeled as token flows (`BPMNMessageFlow`, `BPMNRootMarking.message_flow_2_tokens`) but do not integrate with real message brokers. Webhooks, cron, Kafka, Redis streams, DB change, FS watch — none.

### A12.2–A12.8
Not applicable in the operational sense. The "trigger model" is purely formal: a start event that can fire is one where `is_unconstrained_start_event()` returns `true`, which enables the initial token placement (`semantics.rs:get_initial_marking`). There is no webhook registration, cron expression, or backpressure model.

**A12.7 — Trigger as Action vs separate:** BPMN start and intermediate events in this library are element types within the `BPMNElement` enum, not separate trigger actions. There is no 2-stage Source → Event model analogous to Nebula's `TriggerAction`.

**A12.8 vs Nebula:** Nebula's `TriggerAction` has a 2-stage Source → Event pipeline with backpressure. Ebi_BPMN has a formal state-space model for BPMN events with no operational trigger infrastructure.

## 12. Multi-tenancy [A14]

No multi-tenancy. The library has no concept of users, workspaces, tenants, RBAC, SSO, or SCIM. It is a pure Rust library with no network API and no user identity model.

## 13. Observability [A15]

No observability infrastructure. There is no OpenTelemetry integration, no structured tracing, no metrics, and no logging framework. Debug strings are provided through `transition_debug()` on `Transitionable` (`src/traits/transitionable.rs:45`) for human-readable transition descriptions, but this is not structured telemetry.

Grepped for `tracing`, `opentelemetry`, `metrics`, `log`, `slog` in `src/` — no matches (only `anyhow` for error propagation).

## 14. API surface [A16]

No network API. Ebi_BPMN is a library crate with a Rust public API. The public surface (re-exported from `src/lib.rs:141-153`):

- `BusinessProcessModelAndNotation` — parse, traverse, execute
- `StochasticBusinessProcessModelAndNotation` — stochastic execution
- `BPMNCreator`, `Container`, `EndEventType`, `GatewayType`, `IntermediateEventType`, `StartEventType` — programmatic construction
- `BPMNMarking`, `Token` — marking state
- `BPMNMessageFlow`, `BPMNSequenceFlow` — flow types
- `GlobalIndex` — element addressing
- All element types via `pub mod elements`
- All trait types via `pub mod traits`

No REST, no GraphQL, no gRPC, no OpenAPI.

## 15. Testing infrastructure [A19]

**Test files:** 23 BPMN files in `testfiles/` covering standard patterns (credit scoring, dispatch of goods, OR loops, event-based gateways, message flows, stochastic models). Unit tests are inline in source files (`#[cfg(test)]` blocks). One test observed: `bpmn_pool_translate` in `business_process_model_and_notation.rs:212`.

**No dedicated testing crate.** No public testing utilities for users of the library. The `testactivities` feature flag enables `TestActivityKey` trait for verifying activity key consistency in tests.

**Nebula comparison:** Nebula has a dedicated `nebula-testing` crate with resource-author contracts and integration testing utilities. Ebi_BPMN relies on inline tests and BPMN fixture files only.

## 16. AI / LLM integration [A21] — DEEP (A21.1–A21.13)

### A21.1 — Existence
**No AI or LLM integration.** Confirmed by exhaustive grep.

**Grep evidence:** Searched the entire repository (including test files and Cargo.toml) for: `openai`, `anthropic`, `llm`, `embedding`, `completion`, `ai_`, `llama`, `mistral`, `gpt`, `chatgpt`, `claude` — zero matches in any source file. The only matches for these terms were in BPMN process model testfiles (e.g., `schufa` credit scoring model uses the word "completion" as a natural-language label in a BPMN task name).

### A21.2–A21.13
All not applicable. Ebi_BPMN has no provider abstraction, no prompt management, no structured output, no tool calling, no streaming, no multi-agent, no RAG/vector, no memory/context management, no cost tracking, no observability hooks, and no safety filtering for AI/LLM.

**A21.13 vs Nebula:** Nebula bets that AI = generic actions + plugin LLM client (Surge handles agent orchestration separately). Ebi_BPMN makes the same de-facto bet by omission — it is a behavioral modeling library and AI is entirely outside its scope. The difference is that Nebula's omission is a deliberate strategic choice documented in the product canon, while Ebi_BPMN's omission is a consequence of its narrow research focus.

## 17. Notable design decisions

### D1 — Token-game semantics as the execution model

The library implements BPMN execution as a Petri-net-style token game (`BPMNMarking` with `sequence_flow_2_tokens`, `element_index_2_tokens`, `message_flow_2_tokens` — `marking.rs:207–213`). This is the formal approach used in process-mining research (van der Aalst's token replay, conformance checking). It enables formal reasoning (state-space exploration, trace generation) but does not map to practical workflow execution where tasks have side effects, data, and external integrations.

**Trade-off:** Formally correct and mathematically tractable; practically useless for production orchestration. Nebula does not implement formal token-game semantics — it uses a frontier-based scheduler for production execution.

**Applicability to Nebula:** Could borrow the structural validator approach — Ebi_BPMN's `is_structurally_correct()` pattern is a clean separation of graph validity checking from execution. Nebula's DAG validation could be similarly decoupled.

### D2 — Stochastic BPMN as a first-class concern

`StochasticBusinessProcessModelAndNotation` wraps `BusinessProcessModelAndNotation` and adds probabilistic weights to transitions (`ebi_arithmetic::Fraction`). The `PartiallyOrderedRun` struct computes random execution traces by sampling weighted transitions (`partially_ordered_run.rs:31–40`). This is used in process mining for simulation and probability analysis — not in any production workflow engine.

**Trade-off:** Enables quantitative process analysis (conformance checking with fitness measures); adds complexity to the element model (every transition carries an optional `Fraction` weight). Nebula has no stochastic execution model.

### D3 — Deviation from BPMN 2.0.2 standard explicitly documented

The README explicitly lists four semantic deviations from BPMN 2.0.2 (OR join semantics, deadlock = termination, empty model interpretation, task-after-event-based-gateway as receive task). This academic honesty about spec conformance is uncommon in open-source projects and reflects the research context.

**Trade-off:** Makes the library unsuitable as a compliance-grade BPMN engine; makes it tractable for formal analysis. A production engine like Nebula would need to be closer to the spec if advertising BPMN compatibility.

### D4 — `cdylib` crate type for cross-language embedding

The crate compiles as both a Rust `lib` and a C dynamic library (`Cargo.toml:21`). This is likely to support the parent `ebi` Java tool (https://ebitools.org uses JVM-based tooling). The `cdylib` target imposes constraints on the public API (no Rust-specific types can cross the FFI boundary without wrapping).

**Trade-off:** Enables embedding in JVM tooling; constrains API evolution. Nebula does not need cross-language embedding as it targets native Rust deployment only.

### D5 — `anyhow::Error` throughout — no domain error taxonomy

Every fallible function returns `anyhow::Result<T>`. There is no custom error type, no error classification, and no structured error codes. Errors are human-readable strings (`anyhow!("transition not found")`, `anyhow!("message flow with id `{}` is intra-pool")`). This is appropriate for a library where callers are responsible for handling all errors, but provides no machine-processable error classification.

**Trade-off:** Minimal boilerplate; no type-safe error handling for callers. Nebula uses a dedicated `nebula-error` crate with `ErrorClass` enum for classification.

### D6 — Macro-driven enum dispatch without dynamic dispatch overhead

The `enums!` macro in `element.rs:80–111` generates match arms for all 22 `BPMNElement` variants, routing every trait method call without virtual dispatch. This is a common pattern in Rust for avoiding vtable overhead when the variant set is closed.

**Trade-off:** Efficient; brittle (adding a new element type requires updating every match arm). Nebula uses sealed traits for the same closed-set guarantee but at the trait-impl level rather than enum dispatch.

## 18. Known limitations / pain points

GitHub issues: **0 open, 0 closed** — the repository has no issue tracker activity. This is consistent with it being a brand-new research library (created 2026-02-24, pushed 2026-04-13 = ~7 weeks old).

Known limitations documented in README:
- "The crate does not currently consider or export layouting (bpmndi) information." (README, Limitations section)
- "There is a maximum number of outgoing sequence flows of an inclusive gateway of 64 (on 64-bits system) or 32 (on 32-bit systems)." (README) — this is a `bitvec`-backed bitmask limitation.
- "expanded sub-processes are not supported [in stochastic mode]" (README, Stochastic section)
- "support for inter-pool communication is limited" (README, Stochastic section)
- Sub-process support in general is incomplete: `marking.rs:78` returns `Err(anyhow!("Sub-processes are not supported here for now."))` and `marking.rs:125` does the same
- Commit history shows active churn on OR gateway semantics ("expand support for OR joins in loops" commit e56578d) and partially-ordered run semantics (multiple "does not compile" commits: e0b5f54, 7d4b717, 22aa62e) — indicating active development with unstable features

## 19. Bus factor / sustainability

- **Maintainers:** 1 (sleemans — only contributor per `gh api contributors`)
- **Commit cadence:** Active — 20 commits visible in depth-50 clone, including multiple commits per day in some periods; last push 2026-04-13
- **Organization:** BPM-Research-Group — academic research group (likely TU/e or similar European BPM university group)
- **Version:** 0.0.46 — pre-release versioning; rapid patch increments suggest active development without semantic versioning discipline
- **Issue ratio:** N/A — 0 issues (no community yet)
- **Bus factor:** 1 — extreme risk; single-maintainer academic project
- **Sustainability concern:** Academic projects typically stall when the researcher moves on (graduation, job change). The `ebi` parent project appears more established (https://ebitools.org) but Ebi_BPMN itself is the youngest component.

## 20. Final scorecard vs Nebula

| Axis | ebi_bpmn approach | Nebula approach | Assessment | Borrow? |
|------|-------------------|-----------------|------------|---------|
| A1 Workspace | Single crate, `cdylib + lib`, 6 feature flags | 26 crates, layered, Edition 2024 | Nebula deeper (production-grade separation); ebi_bpmn appropriately minimal for library | no — different goals |
| A2 DAG | No DAG: flat `Vec<BPMNElement>` + sequence flow indices; token-game execution; cycles allowed | TypeDAG L1-L4 (static generics → TypeId → predicates → petgraph) | Nebula deeper for type-safe connectivity; ebi_bpmn correct for formal semantics | no — different goals |
| A3 Action | 22-variant `BPMNElement` enum + 6 open traits (BPMNObject, Transitionable, Processable, Startable, Searchable, Writable); no assoc types; no sealing; no I/O shape; synchronous only | 5 action kinds, sealed traits, assoc Input/Output/Error, derive macros | Different decomposition — ebi_bpmn models BPMN spec elements; Nebula models user-defined execution units | no — different goals |
| A11 BUILD | No plugin system | WASM sandbox planned, plugin-v2 spec, Plugin Fund | Nebula deeper (commercial intent); ebi_bpmn not applicable | no — different goals |
| A11 EXEC | No plugin execution | WASM + capability security | Nebula deeper | no — different goals |
| A18 Errors | `anyhow::Error` throughout; no custom type; no classification | nebula-error crate; ErrorClass enum; used by ErrorClassifier in resilience | Nebula deeper; ebi_bpmn idiomatic for library use but unclassified | refine — anyhow is correct for library-level errors; Nebula's ErrorClass adds value at engine boundaries |
| A21 AI/LLM | None — not applicable to scope | None — strategic bet: AI = generic actions + plugin LLM | Convergent by omission; different strategic framing | no — different goals |
