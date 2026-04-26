# rust-rule-engine — Architectural Decomposition

## 0. Project Metadata

- **Repo:** https://github.com/KSD-CO/rust-rule-engine
- **Stars:** ~35 (low community adoption at time of analysis)
- **License:** MIT
- **Maintainers:** ttvuhm (tonthatvu, 34/50 commits primary), tonthatvu GitHub bot alias (15/50 commits), nghiaphamln (3 commits — contributor for PR #66 performance work). Dependabot (6 commits, automated).
- **Total contributors (human):** 2 active humans.
- **Created:** 2025-10-01 (crates.io) | **Latest release:** v1.20.1
- **Rust Edition:** 2021 (`Cargo.toml:4`)
- **Total downloads:** 8.6K | **Recent downloads:** 2.7K
- **Test count:** 605 `#[test]` / `#[tokio::test]` annotations across `src/` and `tests/`
- **Open issues:** 0 (GitHub issues list returned empty — no issues filed at all)
- **Governance:** MIT open source, no commercial model, no Plugin Fund, no SOC 2 mention.

---

## 1. Concept Positioning [A1, A13, A20]

**Author's description (README.md:8):**
> "A blazing-fast production-ready rule engine for Rust supporting both Forward and Backward Chaining. Features RETE-UL algorithm with Alpha Memory Indexing and Beta Memory Indexing, parallel execution, goal-driven reasoning, and GRL (Grule Rule Language) syntax."

**My description after reading code:**
rust-rule-engine is a single-crate library-first rule engine offering three reasoning modes — forward chaining (native engine + RETE-UL), backward chaining (DFS/BFS/iterative deepening with unification), and stream processing (CEP with time windows via `streaming` feature) — all driven by GRL (Grule Rule Language) syntax parsed with nom. Its extension model is compile-time Rust trait implementation via `RulePlugin`, with no WASM or dynamic library sandbox. It has no persistence layer, no credential system, no multi-tenancy, and no network API.

**Comparison with Nebula:**
rust-rule-engine and Nebula both target automation/decision logic in Rust, but occupy entirely different positions. Nebula is a 26-crate deployment platform with PostgreSQL persistence, credential lifecycle, multi-tenancy, plugin sandboxing, and a 3-mode deployment binary. rust-rule-engine is an embeddable reasoning library. The key differentiation is the reasoning model: rust-rule-engine provides RETE-UL forward chaining and Prolog-style backward chaining — capabilities Nebula has no analogue for. Nebula is a DAG-based orchestrator; rust-rule-engine is a classic production rule system (PRS).

**vs. dataflow-rs:**
Both are library-first, single-crate, embeddable rule libraries. Key differences: dataflow-rs uses JSONLogic as its condition language; rust-rule-engine uses GRL (a textual DSL). dataflow-rs is forward-only (priority-sorted rule list, sequential); rust-rule-engine adds RETE-UL network (O(1) alpha indexing), backward chaining (goal-driven with unification), and stream processing. rust-rule-engine is significantly more sophisticated on the reasoning axis; dataflow-rs is simpler for pure IFTTT-style cases. Neither has persistence, credentials, or a server.

---

## 2. Workspace Structure [A1]

**Crate inventory:** Single crate — `rust-rule-engine` (Edition 2021). No workspace, no sub-crates.

**Module tree (`src/`):**
```
src/
├── lib.rs                   Public API re-exports; RuleEngineBuilder
├── types.rs                 ActionType enum, Value enum, Operator enum, LogicalOperator
├── errors.rs                RuleEngineError enum (13 variants) + Result alias
├── expression.rs            Arithmetic expression evaluator
├── main.rs                  CLI entry point (feature placeholder)
├── engine/
│   ├── engine.rs            RustRuleEngine — forward chaining engine (primary)
│   ├── rule.rs              Rule, Condition, ConditionGroup, ConditionExpression
│   ├── facts.rs             Facts struct (HashMap<String,Value>), FactHelper
│   ├── knowledge_base.rs    KnowledgeBase — rule store with salience sorting
│   ├── plugin.rs            RulePlugin trait, PluginManager, PluginMetadata
│   ├── agenda.rs            AgendaManager, ActivationGroupManager
│   ├── workflow.rs          WorkflowEngine, WorkflowState, ScheduledTask
│   ├── parallel.rs          Parallel evaluation for forward chaining
│   ├── safe_parallel.rs     Thread-safe parallel variant
│   ├── analytics.rs         RuleAnalytics — in-memory execution metrics
│   ├── coverage.rs          RuleCoverage — test coverage reporting
│   ├── dependency.rs        Rule dependency graph
│   ├── module.rs            Module system (import management)
│   ├── template.rs          Rule templates
│   ├── condition_evaluator.rs Condition evaluation helpers
│   └── pattern_matcher.rs  Exists/Forall pattern matching
├── parser/
│   ├── grl.rs               GRLParser — nom-based GRL parser
│   ├── grl_no_regex.rs      Alternative parser (no regex backend)
│   ├── grl_helpers.rs       Parser helper utilities
│   ├── grl/stream_syntax.rs Stream pattern GRL extensions
│   ├── parallel.rs          Parallel parsing
│   └── zero_copy.rs         Zero-copy parsing helpers
├── rete/                    RETE-UL algorithm implementation
│   ├── alpha.rs / alpha_memory_index.rs  Alpha nodes + indexed alpha memory
│   ├── beta.rs              Beta nodes + join operations
│   ├── network.rs           RETE-UL network construction
│   ├── grl_loader.rs        GRL → RETE network loader
│   ├── facts.rs             TypedFacts / FactValue (typed fact system for RETE)
│   ├── working_memory.rs    WorkingMemory — fact store for RETE engine
│   ├── tms.rs               Truth Maintenance System
│   ├── optimization.rs      Token pooling, beta indexing, compaction
│   ├── memoization.rs       Memoized condition evaluation
│   ├── accumulate.rs        SUM/COUNT/AVG/MIN/MAX accumulate functions
│   ├── stream_alpha_node.rs StreamAlphaNode (streaming feature)
│   ├── stream_beta_node.rs  StreamBetaNode (streaming feature)
│   └── stream_join_node.rs  StreamJoinNode (streaming feature)
├── backward/                Backward chaining (backward-chaining feature)
│   ├── backward_engine.rs   BackwardEngine + BackwardConfig
│   ├── search.rs            SearchStrategy enum, DFS/BFS/IterativeDeepening
│   ├── unification.rs       Prolog-style unification
│   ├── proof_tree.rs        ProofTree, ProofTrace
│   ├── proof_graph.rs       ProofGraph — TMS-integrated proof cache
│   ├── goal.rs              Goal, GoalManager, GoalStatus
│   ├── query.rs             QueryParser, QueryResult
│   ├── aggregation.rs       Backward aggregation (COUNT/SUM/AVG/MIN/MAX)
│   ├── optimizer.rs         Goal reordering for 10-100x speedup
│   ├── disjunction.rs       OR-pattern support
│   ├── nested.rs            Nested subqueries with shared variables
│   └── explanation.rs       Proof tree export (JSON/MD/HTML)
├── plugins/                 Built-in plugin suite
│   ├── string_utils.rs      String actions + functions (13+7)
│   ├── math_utils.rs        Math actions + functions (12+8)
│   ├── date_utils.rs        Date/Time actions + functions (8+6)
│   ├── validation.rs        Validation actions + functions (6+6)
│   └── collection_utils.rs  Collection actions + functions (7+6)
└── streaming/               Stream processing (streaming feature)
    ├── engine.rs            StreamingEngine
    ├── window.rs            WindowType: Sliding/Tumbling/Session
    └── state.rs             Stateful operators, file-based checkpoint
```

**Feature flags (`Cargo.toml:34-38`):**
- `streaming` — enables tokio + stream processing nodes
- `streaming-redis` — adds Redis state backend for distributed streaming
- `backward-chaining` — enables the `backward` module

**Umbrella crate:** None — `rust-rule-engine` is the single consumer entry point.

**Comparison with Nebula:** 1 crate vs 26 crates. The single-crate design is appropriate for an embeddable reasoning library. Nebula's layered crate separation (nebula-error / nebula-resilience / nebula-credential / nebula-resource / nebula-action / nebula-engine / nebula-tenant) reflects deployment platform concerns that are out of scope here.

---

## 3. Core Abstractions [A3, A17] — DEEP

This section answers all A3.1-A3.9 questions with code citations.

### A3.1 — Trait shape

There is no single "action trait". rust-rule-engine uses two complementary models:

**1. `ActionType` enum** (`src/types.rs`) — the closed set of action kinds a rule can fire:
```
ActionType::Set         { field: String, value: Value }
ActionType::Log         { message: String }
ActionType::MethodCall  { object: String, method: String, args: Vec<Value> }
ActionType::Retract     { object: String }
ActionType::Custom      { action_type: String, params: HashMap<String,Value> }
ActionType::ActivateAgendaGroup { group: String }
ActionType::ScheduleRule        { rule_name: String, delay_ms: u64 }
ActionType::CompleteWorkflow    { workflow_name: String }
ActionType::SetWorkflowData     { key: String, value: Value }
ActionType::Append      { field: String, value: Value }
```
This is an enum, not a sealed trait — it is a closed set. No external crate can add an `ActionType` variant without modifying this file.

**2. `RulePlugin` trait** (`src/engine/plugin.rs:48-70`) — the open extension point for custom functions and action handlers:
```rust
pub trait RulePlugin: Send + Sync {
    fn get_metadata(&self) -> &PluginMetadata;
    fn register_actions(&self, engine: &mut RustRuleEngine) -> Result<()>;
    fn register_functions(&self, _engine: &mut RustRuleEngine) -> Result<()> { Ok(()) }
    fn unload(&mut self) -> Result<()> { Ok(()) }
    fn health_check(&mut self) -> PluginHealth { PluginHealth::Healthy }
}
```

**Open or sealed?** Open — any external crate can implement `RulePlugin`. No sealing mechanism.

**`dyn` compatible?** Yes — stored as `Arc<dyn RulePlugin>` in `PluginManager.plugins: HashMap<String, Arc<dyn RulePlugin>>` (`src/engine/plugin.rs:103`).

**Associated types:** Zero on `RulePlugin`. No `Input`, `Output`, `Error`, `Config`, `State` associated types. The custom function signature is a bare closure: `Fn(&[Value], &Facts) -> Result<Value> + Send + Sync + 'static` (`src/engine/engine.rs:17`). The custom action handler is: `Fn(&HashMap<String, Value>, &Facts) -> Result<()> + Send + Sync + 'static` (`src/engine/engine.rs:20`). Both are type-erased function pointers.

**GAT, HRTB, typestate:** None. The extension model is maximally simple — no advanced type system features.

**`dyn Action` pattern:** Not used in the rule sense. The engine dispatches actions via the `ActionType` enum match in `RustRuleEngine::execute_action()` (`src/engine/engine.rs:1202-1352`), not via trait objects.

### A3.2 — I/O shape

**Facts:** Both the native forward engine and the RETE engine share a flat key-value store:
- `Facts` struct (`src/engine/facts.rs`): `HashMap<String, Value>` under `RwLock<>`. Fields accessed as dot-paths (`"Customer.TotalSpent"`). No associated Input type per action.
- `TypedFacts` / `FactValue` (`src/rete/facts.rs`): Typed variant for the RETE engine with variants `FactValue::String`, `FactValue::Integer`, `FactValue::Float`, `FactValue::Boolean`, `FactValue::Array`, `FactValue::Null`.

**Output:** Actions mutate `Facts` in place (`facts.set(field, value)`). No typed output. Side effects are direct mutations to the shared fact store via `RwLock`.

**Type erasure:** Complete — all inter-rule data flows through `HashMap<String, Value>`. The `Value` enum has variants: `String`, `Number(f64)`, `Integer(i64)`, `Boolean`, `Array`, `Object(HashMap)`, `Null`, `Expression(String)`.

**Streaming:** The `streaming` feature adds `StreamAlphaNode` for CEP; events flow into `WorkingMemory` and become available as facts for rule evaluation. No backpressure mechanism beyond tokio's channel capacity.

### A3.3 — Versioning

**Rule versioning:** `Rule` struct has no `version` field. Rules are identified by `name: String` only (`src/engine/rule.rs`, `KnowledgeBase`). No `v1`/`v2` distinction, no `#[deprecated]` on rules, no migration support.

**Function versioning:** None. Custom functions registered by string name only. Name collision silently overwrites the previous entry in the `HashMap<String, CustomFunction>`.

**`enabled` field on Rule:** `Rule.enabled: bool` allows disabling rules without removal (`src/engine/engine.rs:495`). Not versioning, but provides an on/off toggle.

**Date-effective/expires:** `Rule.date_effective: Option<DateTime<Utc>>` and `Rule.date_expires: Option<DateTime<Utc>>` (`src/engine/rule.rs`) enable time-bounded rules — checked in `execute_at_time()` (`src/engine/engine.rs:502`). This is a form of temporal versioning.

### A3.4 — Lifecycle hooks

`RulePlugin` has `unload()` and `health_check()` hooks (`src/engine/plugin.rs:62-69`). No `pre_execute`, `post_execute`, `on_failure`, `cleanup`, or `on_cancel` hooks at the rule level.

**Cancellation:** No cancellation mechanism. The engine has a configurable `max_cycles: usize` (default 100) and `timeout: Option<Duration>` (default 30s, `src/engine/engine.rs:39`). Timeout is checked per-cycle against `start_time.elapsed()` — not per-action.

**Idempotency:** `no_loop: bool` on `Rule` prevents re-firing once fired within an execution (`src/engine/engine.rs:517`). Global `fired_rules_global: HashSet<String>` tracks this across cycles. This is loop prevention, not idempotency.

### A3.5 — Resource and credential dependencies

No dependency declaration mechanism. Custom functions capture external resources (DB pools, HTTP clients) in their closure scope. The engine has no awareness of what external resources a function needs. No compile-time check.

### A3.6 — Retry/resilience attachment

No retry policy, circuit breaker, bulkhead, or hedging at any level. `EngineConfig.timeout: Option<Duration>` is a global execution timeout, not per-action retry. The `RuleEngineError` enum has no `retryable()` classifier analogous to dataflow-rs.

### A3.7 — Authoring DX

**Builder pattern:** `RuleEngineBuilder` (`src/lib.rs:194-258`) provides `with_inline_grl(grl: &str)`, `with_rule_file(path)`, `with_config(config)`, `build()`. This is ergonomic for GRL-based usage.

**Custom function DX:**
```rust
engine.register_function("myFunc", |args, facts| {
    Ok(Value::Number(42.0))
});
```
Approximately 3 lines. Very low ceremony.

**Custom plugin DX:** Implement `RulePlugin`, provide `PluginMetadata`, implement `register_actions` and optionally `register_functions`. Approximately 30-40 lines for a minimal plugin.

**No proc-macro** for rule or action generation.

### A3.8 — Metadata

`PluginMetadata` (`src/engine/plugin.rs:25-35`) carries: `name`, `version`, `description`, `author`, `state: PluginState`, `health: PluginHealth`, `actions: Vec<String>`, `functions: Vec<String>`, `dependencies: Vec<String>`. All runtime data, from plugin struct construction. No compile-time metadata, no icon, no i18n.

`Rule` has: `name: String`, `salience: i32`, `description: Option<String>`, `tags: Vec<String>`, `enabled: bool`, `no_loop: bool`, `agenda_group: Option<String>`, `activation_group: Option<String>`, `date_effective`, `date_expires`.

### A3.9 — vs Nebula

Nebula has 5 sealed action kinds (Process/Supply/Trigger/Event/Schedule) with associated `Input`/`Output`/`Error` types enforcing compile-time port-level type safety across DAG edges. rust-rule-engine has a 10-variant `ActionType` enum (closed to extension without source modification) plus an open `RulePlugin` trait for custom functions/actions. All data flows through `HashMap<String, Value>` — no compile-time type checking at rule I/O boundaries.

rust-rule-engine's strength vs Nebula: it has a far richer reasoning model — RETE-UL forward chaining with O(1) alpha indexing, backward chaining with unification and proof trees, and CEP stream processing. Nebula has no analogue for any of these. Nebula's strength: 5-kind sealed trait system with compile-time port safety, DAG structure, credential lifecycle, persistence, and multi-tenancy.

**How many action kinds?** 10 in `ActionType` enum, not counting the open `RulePlugin` extension. Compare: dataflow-rs has 12 `FunctionConfig` variants. Nebula has 5 sealed kinds. rust-rule-engine's 10-variant enum sits between them but includes workflow-control actions (ActivateAgendaGroup, ScheduleRule, CompleteWorkflow) that have no equivalent in dataflow-rs.

---

## 4. DAG / Execution Graph [A2, A9, A10]

### Graph model

rust-rule-engine does NOT implement a DAG at the workflow level. The execution model for the native forward engine (`RustRuleEngine::execute()`) is:

1. Rules are sorted by `salience: i32` (highest first) at each cycle via `knowledge_base.get_rules_by_salience()` (`src/engine/engine.rs:488`).
2. Per cycle: iterate rules by salience, evaluate each condition, fire matching rules, collect actions.
3. Continue until no rule fires in a cycle or `max_cycles` is reached.

This is a **match-resolve-act** cycle (Rete-style inference loop), not a DAG. There are no explicit edges, no ports, no port-typing.

**RETE-UL network** (`src/rete/`): The `IncrementalEngine` uses the actual RETE algorithm — Alpha nodes filter single-fact conditions, Beta nodes join multiple facts, the network propagates token activations to production rules. This is a proper RETE network, not just a sorted list. However, the RETE network is an optimization for the forward chaining inference loop — not a user-visible DAG workflow.

**WorkflowEngine** (`src/engine/workflow.rs`): The engine has a `WorkflowEngine` that supports agenda-group-based workflow stepping (`execute_workflow()`, `execute_workflow_step()`). Rules can fire `ActionType::ActivateAgendaGroup` to transfer control to another group. This models sequential workflow stages but still lacks DAG structure — it is agenda group activation chains.

**No port typing, no compile-time graph validation.**

### Concurrency

`EngineConfig` has no parallelism settings at the knowledge-base level. The `streaming` feature uses `tokio::spawn` for async event processing (`src/streaming/engine.rs`). The `engine/parallel.rs` and `engine/safe_parallel.rs` modules exist but their use in the main `execute()` path is not the default — salience-ordered sequential evaluation is the default. The `RulePlugin` bound requires `Send + Sync`, enabling multi-threaded plugin loading but not automatic parallel rule evaluation.

`BackwardEngine` is explicitly not thread-safe by default (documented in `tests/backward_thread_safety.rs` which wraps it in `Arc<Mutex<>>`).

**`!Send` handling:** `RulePlugin: Send + Sync` required. Custom function closures must also be `Send + Sync + 'static`. No `!Send` support, no thread-local isolation.

**Comparison with Nebula:** Nebula's TypeDAG (L1 static generics → L2 TypeId → L3 refinement predicates → L4 petgraph) enforces port-level type correctness at compile time. rust-rule-engine has no DAG, no port types. Nebula's frontier scheduler enables parallel task execution within a workflow; rust-rule-engine's default forward chaining is cycle-sequential. The RETE network handles multi-fact joins internally through alpha/beta node propagation but this is not user-programmable parallelism.

---

## 5. Persistence and Recovery [A8, A9]

### Storage

**No database persistence.** The engine's `Cargo.toml` has no `sqlx`, `diesel`, `sled`, or any database dependency. The `streaming/state.rs` module documents `StateBackend::File` for file-based checkpoint state in stream processing, but this is not a general persistence model for rules or execution state.

Grep evidence: `grep -r "postgres\|sqlite\|mysql\|sqlx\|diesel\|rocksdb\|sled" src/ --include="*.rs"` — zero results.

### Persistence model

**In-memory only.** The `Facts` struct is a `RwLock<HashMap<String, Value>>` — ephemeral per engine session. No append-only log, no checkpoint, no event sourcing, no crash recovery for the forward/backward engines.

`streaming/state.rs` mentions `auto_checkpoint: bool` and file-based state as a future capability (`StateBackend::File` variant documented with "File-based state (persistent)" comment), but the streaming state module is described as "in-memory state (not persistent across restarts)" for the current default backend (`src/streaming/state.rs`).

**Comparison with Nebula:** Nebula has a frontier-based scheduler with checkpoint recovery, append-only execution log, and state reconstruction via replay — none of which exist in rust-rule-engine.

---

## 6. Credentials / Secrets [A4]

### A4.1 — Existence

**No credential layer.** Explicit statement: rust-rule-engine has no credential management abstraction of any kind.

Grep evidence: `grep -r "credential\|secret\|vault\|keychain\|encrypt\|zeroize\|secrecy\|oauth\|token" src/ --include="*.rs"` — zero results in source files (the only "token" hits are for the RETE Token struct, not credentials).

### A4.2-A4.8

Not applicable. No credential storage, no in-memory protection, no lifecycle, no OAuth2, no composition, no scoping, no type safety for credentials.

### A4.9 — vs Nebula

Nebula's credential subsystem (State/Material split, CredentialOps trait, LiveCredential with watch() for blue-green refresh, OAuth2Protocol blanket adapter, DynAdapter) has no equivalent in rust-rule-engine. Any API key or connection string used by a custom function must be captured in the closure scope at registration time by the calling application. The `.env.example` file documents environment variable names for AI API keys (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, etc.) but these are documentation-only — no engine code reads them. The documented "AI Integration" is a pattern for embedding application code, not an engine facility.

---

## 7. Resource Management [A5]

### A5.1 — Existence

**No resource abstraction.** Explicit statement: rust-rule-engine has no first-class resource lifecycle concept.

Grep evidence: `grep -r "resource\|pool\|connection\|reload\|generation\|scope" src/ --include="*.rs"` — zero results related to resource lifecycle management (the "reload" hits are for `hot_reload_plugin` in the plugin system, not resource reload).

### A5.2-A5.7

Not applicable. No scope levels, no lifecycle hooks, no hot-reload for resources, no sharing semantics beyond what the calling application manages, no credential-resource interaction, no backpressure.

### A5.8 — vs Nebula

Nebula has 4 scope levels (Global/Workflow/Execution/Action), `ReloadOutcome` enum, generation tracking, and per-resource `on_credential_refresh` hooks. rust-rule-engine delegates all resource management to the embedding application, consistent with its library-first scope.

---

## 8. Resilience [A6, A18]

### A18 — Error types

`RuleEngineError` enum (`src/errors.rs:5-101`) using `thiserror = "2.0"`, 13 variants:
```
ParseError { message: String }
EvaluationError { message: String }
FieldNotFound { field: String }
IoError(#[from] std::io::Error)
TypeMismatch { expected: String, actual: String }
InvalidOperator { operator: String }
InvalidLogicalOperator { operator: String }
RegexError { message: String }
ActionError { message: String }
ExecutionError(String)
SerializationError { message: String }
PluginError { message: String }
FeatureNotEnabled { feature: String, message: String }
ModuleError { message: String }
```

No `ErrorClass` enum, no `retryable()` method, no error classification for resilience decisions. `FeatureNotEnabled` is notable — correctly surfaces feature-flag-gated errors at runtime rather than compile-time misconfig.

### Resilience patterns

No retry policy, circuit breaker, bulkhead, timeout (at action level), or hedging. The engine has a global execution timeout (`EngineConfig.timeout: Option<Duration>`, default 30s) and a maximum cycle count (`max_cycles: usize`, default 100), both checked in the main execution loop. These are safety bounds, not resilience policies.

**Comparison with Nebula:** Nebula has `nebula-resilience` as a separate crate providing retry/CB/bulkhead/timeout/hedging with `ErrorClassifier` categorizing transient vs permanent errors. rust-rule-engine has no resilience layer; errors surface to the caller immediately.

---

## 9. Expression / Data Routing [A7]

### DSL: GRL (Grule Rule Language)

rust-rule-engine uses GRL syntax inspired by the Go-based `grule-rule-engine`. Rules are parsed by `GRLParser` (`src/parser/grl.rs`) using the `nom = "8.0"` parser combinator.

**GRL syntax:**
```grl
rule "VIP Discount" salience 10 no-loop {
    when
        Customer.TotalSpent > 10000 &&
        Customer.active == true
    then
        Customer.Discount = 0.15;
        Log("VIP discount applied");
}
```

**Operators:** `>`, `<`, `>=`, `<=`, `==`, `!=`, `contains`, `startsWith`, `endsWith`, `matches` (wildcard), `in` (array membership). Logical: `&&`, `||`, `!`.

**Expressions in conditions:**
- Field references: `Customer.TotalSpent`
- Function calls: `aiSentiment(User.text) > 0.5`
- Arithmetic expressions in actions: `order.total * 0.1`
- Array literal: `User.role in ["admin", "vip"]`
- Date-effective/expires fields on rules

**Comparison with Nebula:** Nebula's expression engine has 60+ functions, type inference, and JSONPath-like `$nodes.foo.result.email` syntax for cross-node data references. GRL is a rules-oriented language optimized for condition-action pattern matching; Nebula's expression engine is optimized for data transformation pipelines. GRL is richer on the reasoning side (backward chaining queries, accumulate functions, unification variables `?x`); Nebula's is richer on the data routing side. Neither is a strict superset.

**Comparison with dataflow-rs:** dataflow-rs uses JSONLogic (JSON-native, ~20 operators). rust-rule-engine uses GRL (text DSL, richer syntax, backward chaining queries). GRL is more expressive; JSONLogic is more portable.

---

## 10. Plugin / Extension System [A11] — DEEP

### 10.A — Plugin BUILD process

**A11.1 — Format:** No external plugin format. Plugins are Rust structs implementing `RulePlugin` trait, compiled into the same binary as the engine. No `.tar.gz`, no OCI, no WASM blob, no manifest file.

**A11.2 — Toolchain:** The caller's own Cargo project. No separate SDK, no CLI scaffolding, no cross-compilation requirement. The built-in plugin suite (`src/plugins/`) is pre-compiled as part of the crate.

**A11.3 — Manifest content:** `PluginMetadata` struct (`src/engine/plugin.rs:25-35`) carries: `name`, `version`, `description`, `author`, `state`, `health`, `actions: Vec<String>`, `functions: Vec<String>`, `dependencies: Vec<String>`. Dependencies are validated before loading (`PluginManager::validate_dependencies()`, `src/engine/plugin.rs:215-222`) — if a named dependency plugin is not yet loaded, the load fails. This is the closest thing to capability declaration; it is runtime string-based, not capability-permission-based.

**A11.4 — Registry/discovery:** `PluginManager.plugins: HashMap<String, Arc<dyn RulePlugin>>` in-memory registry. No remote registry, no OCI, no signing, no version pinning. Load order tracked in `load_order: Vec<String>`.

### 10.B — Plugin EXECUTION sandbox

**A11.5 — Sandbox type:** None. Plugins execute in-process, in the same memory space, on the same thread as the rule engine. No WASM runtime, no dynamic library loading, no subprocess isolation.

Grep evidence: `grep -r "wasm\|wasmtime\|wasmer\|wasmi\|libloading\|dlopen\|sandbox" src/ --include="*.rs"` — zero results.

**A11.6 — Trust boundary:** None. A plugin has full access to the process's memory, filesystem, network, and system calls. Zero sandbox enforcement.

**A11.7 — Host↔plugin calls:** Direct Rust method dispatch — `plugin.register_actions(engine)` → `engine.register_action_handler(name, closure)`. No IPC, no serialization, no async crossing. Plugins call `engine.register_function(name, closure)` and `engine.register_action_handler(type, closure)` during load (`src/engine/engine.rs:196-211`).

**A11.8 — Lifecycle:** `load_plugin(plugin: Arc<dyn RulePlugin>)` / `unload_plugin(name)` / `hot_reload_plugin(name, new_plugin)` on `RustRuleEngine`. Hot-reload is implemented as: unload old plugin (remove from `PluginManager.plugins`), re-register new plugin actions/functions, load new plugin. Note: hot-reload does NOT evict previously registered closures from the engine's `custom_functions` or `action_handlers` HashMaps — the old closures remain registered by name until they are overwritten by the new plugin's `register_actions`. This is a correctness gap: unloading a plugin does not unregister its functions.

**A11.9 — vs Nebula:**

Nebula targets WASM sandbox with capability-based security, Plugin Fund commercial model, and plugin-v2 spec. rust-rule-engine has no WASM sandbox — the extension model is compile-time same-binary Rust trait implementation. This is the same assessment as dataflow-rs: maximum simplicity, zero security isolation. The `PluginConfig.safety_checks: bool` field (`src/engine/plugin.rs:86`) and `enable_hot_reload: bool` sound sophisticated, but `safety_checks` only validates the `dependencies: Vec<String>` list — it does not sandbox plugin execution.

---

## 11. Trigger / Event Model [A12]

### A12.1 — Trigger types

The base forward and backward chaining engines have no trigger subsystem. The `streaming` feature adds stream-based event processing.

**Explicit trigger types:**
- **Stream events** (`streaming` feature): `StreamAlphaNode` processes `WorkingMemory` events matching filters and time windows. Events are pushed by the embedding application via `StreamAlphaNode::process_event()` (`src/rete/stream_alpha_node.rs`).
- **Scheduled tasks** (`WorkflowEngine::schedule_rule(rule_name, delay_ms, None)` — rules can self-schedule via `ActionType::ScheduleRule`). This is an in-process delay scheduler, not a persistent cron.

**No webhook, no external cron, no Kafka consumer, no database CDC.** Grep evidence: `grep -r "webhook\|http.*listen\|cron\|schedule.*cron\|kafka.*consumer\|rabbitmq\|nats\|pubsub" src/ --include="*.rs"` — zero results for external trigger mechanisms.

### Stream processing windows

`WindowType` enum (`src/streaming/window.rs`):
- `Sliding` — continuously moving window
- `Tumbling` — non-overlapping fixed-interval window
- `Session { timeout: Duration }` — gap-based session window

Multi-stream joins are supported via `StreamJoinNode` (`src/rete/stream_join_node.rs`). GRL stream syntax:
```grl
login: LoginEvent from stream("logins") over window(10 min, sliding) &&
purchase: PurchaseEvent from stream("purchases") over window(10 min, sliding) &&
login.user_id == purchase.user_id
```

### A12.8 — vs Nebula

Nebula's `TriggerAction` has `Input = Config` (registration phase) and `Output = Event` (typed payload), with the `Source` trait normalizing raw inbound (HTTP req / Kafka msg / cron tick) into a typed `Event` — a 2-stage model with type-safe payload propagation into the DAG. rust-rule-engine's stream events flow into `WorkingMemory` as facts, losing their type identity. The integration boundary is explicit in Nebula (Source → Event → TriggerAction); implicit in rust-rule-engine (push event → working memory → any matching rule fires). Nebula's model is more suitable for typed workflow orchestration; rust-rule-engine's is more suitable for real-time CEP pattern matching.

---

## 12. Multi-tenancy [A14]

**No multi-tenancy.** Grep evidence: `grep -r "tenant\|rbac\|workspace\|organization\|rls" src/ --include="*.rs"` — zero results.

The `KnowledgeBase` struct (`src/engine/knowledge_base.rs`) has a `name: String` field for labeling, not tenant isolation. No RBAC, no SSO, no SCIM.

---

## 13. Observability [A15]

**No OpenTelemetry, no structured tracing, no metrics export.** The `log` crate is used throughout for debug/info/warn/error output. Grep evidence: `grep -r "opentelemetry\|tracing\|prometheus\|metrics" src/ --include="*.rs"` — zero results in source (the `tracing` hits in the README changelog are documentation mentions, not code imports).

**Built-in analytics:** `RuleAnalytics` (`src/engine/analytics.rs`) — in-memory rule execution metrics: per-rule execution count, total time, average time, success rate, failure modes. Accessible via `engine.analytics()`. All in-memory, not exported to any telemetry backend.

**Rule coverage:** `RuleCoverage` (`src/engine/coverage.rs`) — tracks which rules have been evaluated in tests, generates coverage report strings. Useful for test completeness, not for production observability.

---

## 14. API Surface [A16]

### Programmatic API

Public API (`src/lib.rs:161-265`):
- `RuleEngineBuilder` — builder for rule engine configuration
- `RustRuleEngine` — primary forward chaining engine
- `KnowledgeBase` — rule store
- `Facts` / `FactHelper` — fact manipulation
- `Rule` / `Condition` / `ConditionGroup` — rule structure
- `GRLParser` — parse GRL strings to `Vec<Rule>`
- Feature-gated: `backward::BackwardEngine`, `rete::IncrementalEngine`, `streaming::StreamingEngine`

### Network API

**None.** No HTTP server, no gRPC, no WebSocket, no REST API. RETE_ARCHITECTURE.md plans a REST API for v1.2.0 but it does not exist in the codebase.

### Versioning

Semver via `version = "1.20.1"` in `Cargo.toml`. Public API follows Rust semver conventions.

---

## 15. Testing Infrastructure [A19]

**605 total `#[test]` / `#[tokio::test]` annotations** across `src/` and `tests/`.

Test files in `tests/`:
- `grl_harness.rs` + `grl_cases.yml` — data-driven GRL test harness
- `backward_comprehensive_tests.rs` — backward chaining tests
- `backward_thread_safety.rs` — concurrency tests with `Arc<Mutex<BackwardEngine>>`
- `backward_tms_integration.rs` — TMS integration tests
- `proof_graph_integration_test.rs` — proof graph cache tests
- `rete_performance_test.rs` — RETE performance regression tests
- `tms_test.rs` — Truth Maintenance System tests
- `test_module_system.rs` — module system tests

No public testing crate, no `insta` snapshot testing, no `wiremock`, no `mockall`. Tests are direct unit/integration tests with manual assertion patterns.

**Comparison with Nebula:** Nebula has `nebula-testing` crate with published contract tests for resource implementors, `insta` + `wiremock` + `mockall`. rust-rule-engine has a more comprehensive test count (605 vs Nebula's comparable scope) but no published testing infrastructure for consumers.

---

## 16. AI / LLM Integration [A21] — DEEP

This section answers all A21.1-A21.13 questions.

### A21.1 — Existence

**No built-in LLM integration in shipped Rust source code.** The `.env.example` and `docs/examples/AI_INTEGRATION.md` document how an embedding application *could* call OpenAI/Anthropic/HuggingFace APIs via custom functions, but this guidance is documentation-only. No `reqwest`, `openai`, `anthropic`, or any AI SDK appears in `Cargo.toml` dependencies or any `.rs` source file.

Grep evidence:
- `grep -r "openai\|anthropic\|llm\|embedding\|completion\|gpt\|claude\|gemini\|langchain\|ollama\|candle\|mistral" src/ --include="*.rs"` — zero results.
- `grep -r "reqwest\|http_client\|AI_INTEGRATION" src/ examples/ --include="*.rs"` — zero results in any compiled source.
- `Cargo.toml` dependencies: `serde`, `serde_json`, `rexile`, `thiserror`, `chrono`, `log`, `tokio` (optional), `nom`, `redis` (optional). No AI SDKs.

The documented AI integration is a pattern where a user implements `CustomFunction` as a closure calling an HTTP endpoint. The `.env.example` provides example env var names. This is a guide, not a feature.

### A21.2 — Provider abstraction

Not applicable — no built-in provider abstraction exists.

### A21.3 — Prompt management

Not applicable.

### A21.4 — Structured output

Not applicable.

### A21.5 — Tool calling

Not applicable.

### A21.6 — Streaming

Not applicable for LLM streaming. (The `streaming` feature is for CEP/stream processing, not LLM token streaming.)

### A21.7 — Multi-agent

Not applicable.

### A21.8 — RAG/vector

Not applicable. No vector store integration.

### A21.9 — Memory/context

Not applicable. The `Facts` store is per-engine-session in-memory; no multi-turn conversation memory.

### A21.10 — Cost/tokens

Not applicable.

### A21.11 — Observability

Not applicable.

### A21.12 — Safety

Not applicable.

### A21.13 — vs Nebula + Surge

Nebula has no first-class LLM abstraction (strategic bet: AI = generic actions + plugin LLM client; Surge handles agent orchestration on ACP). rust-rule-engine has the same position by default (no LLM built-in), but with a documentation-level gesture toward AI integration via `.env.example` and `AI_INTEGRATION.md`.

The `Cargo.toml:8` keywords include `"ai"` and `"ml"` — this is positioning metadata, not capability. The actual AI integration guidance describes the same pattern any Rust crate would use: capture an HTTP client in a closure, register as a custom function. No SDK abstractions, no tool calling, no structured output, no multi-agent patterns.

**Key contrast with z8run** (full workflow engine with 10 built-in AI nodes): rust-rule-engine is at the opposite end — the `.env.example` is the entirety of AI "support". The keyword `"ai"` on crates.io is aspirational.

---

## 17. Notable Design Decisions

### Decision 1: Dual reasoning mode — forward chaining + backward chaining in one library

**Decision:** The library ships both forward chaining (native match-resolve-act cycle + RETE-UL network) and backward chaining (goal-driven with Prolog-style unification, DFS/BFS/IDS strategies) as separate engines sharing the same `KnowledgeBase` and `Facts` types. The two modes can be combined — the `DepthFirstSearch::new_with_engine()` variant (`src/backward/search.rs`) accepts a `RETE IncrementalEngine` reference to reuse RETE's working memory during backward search.

**Trade-off:** Very broad capability spectrum for a single library. Expert systems requiring both data-driven and goal-driven reasoning get both. Cost: significant complexity (the `backward/` module is 17 files), potential for confusion about which engine to use, and the two engines having different fact models (`Facts` vs `TypedFacts`). The `BackwardEngine` is not thread-safe by default (documented in `tests/backward_thread_safety.rs`).

**vs dataflow-rs:** dataflow-rs is forward-only (priority-sorted sequential chain). rust-rule-engine is the richer choice for any use case involving reasoning about goals, constraints, or proof chains.

**Applicability to Nebula:** Nebula has no backward chaining and no goal-driven reasoning. This is a legitimate capability gap for any workflow orchestration needing "can this order be approved?" style proof queries. However, backward chaining in a production rules engine is a different paradigm than Nebula's action DAG. Not a direct borrow — would require a new crate (`nebula-reasoning` or similar).

### Decision 2: RETE-UL with Alpha Memory Indexing for O(1) pattern matching

**Decision:** The `IncrementalEngine` (`src/rete/`) implements RETE-UL with indexed alpha memory (`alpha_memory_index.rs`), beta memory indexing (`optimization.rs`), token pooling, and memoization. Alpha memory indexing reduces the cost of activating matching rules from O(N) linear scan to O(1) hash lookup per fact field.

**Trade-off:** 122x speedup in alpha evaluation benchmarks (README). Trade-offs: alpha indexing makes writes 5-10% slower (index maintenance), node sharing makes setup 2x slower, and token pooling rarely benefits typical workloads. The optimizations are composable and can be applied selectively after profiling.

**vs dataflow-rs:** dataflow-rs pre-compiles JSONLogic expressions at startup (O(1) index lookup via Vec), which is conceptually similar but much simpler (no RETE network, no join nodes). RETE-UL handles multi-fact conditions with joins; dataflow-rs evaluates conditions against a single message context.

**Applicability to Nebula:** Nebula's expression engine pre-compiles expressions but has no RETE network. If Nebula adds complex multi-fact condition evaluation, the RETE alpha/beta indexing pattern is worth studying.

### Decision 3: GRL text DSL vs JSON-based condition languages

**Decision:** rust-rule-engine chose GRL (a text DSL inspired by grule-rule-engine for Go) rather than a JSON-based language like JSONLogic (dataflow-rs) or YAML. GRL is parsed by nom and supports conditions, actions, method calls, function calls, aggregates, and backward chaining queries in a readable, human-authored format.

**Trade-off:** GRL is more readable and expressive than JSONLogic for complex rule conditions. Trade-offs: parser complexity (multiple parser implementations — `grl.rs`, `grl_no_regex.rs`, `grl_helpers.rs`), parser bugs (v1.19.3 fixed 16 critical `.unwrap()` panics in the parser), and lack of portability (JSONLogic has implementations in 20+ languages; GRL is Rust-specific). The parser's use of `rexile` (custom regex library) rather than the standard `regex` crate adds another dependency.

**Applicability to Nebula:** Nebula's expression engine uses its own syntax (`$nodes.foo.result.email`). No direct borrow. The GRL approach of separating conditions and actions in a named, salience-ordered rule structure is a clean DSL design pattern for business rules — relevant if Nebula adds a business rules layer.

### Decision 4: ProofGraph — TMS-integrated proof caching for backward chaining

**Decision:** `ProofGraph` (`src/backward/proof_graph.rs`, 520 lines) provides a global cache for proven facts with dependency tracking and Truth Maintenance System (TMS) integration. When a fact that was used as a premise is retracted, cascading invalidation propagates through the dependency graph automatically.

**Trade-off:** O(1) cache lookup for subsequent identical queries — significant speedup for repeated backward queries against the same fact base. TMS integration means the cache stays consistent when facts change. Trade-off: memory overhead for storing proof justifications; complexity for reasoning about invalidation correctness.

**Applicability to Nebula:** Nebula's `nebula-resource` has generation tracking for cache invalidation — a simpler version of this concept. The ProofGraph's cascading invalidation through a dependency graph is a more sophisticated pattern, applicable if Nebula adds memoized computations with reactive invalidation.

### Decision 5: Plugin system with same-process execution and no sandbox

**Decision:** The `RulePlugin` trait and `PluginManager` provide lifecycle management (load/unload/hot-reload/health-check) and dependency validation, but plugins execute in-process with no isolation.

**Trade-off:** Zero overhead, zero marshaling, direct Rust method dispatch. Trade-offs: zero security isolation — a malicious or buggy plugin can crash or corrupt the process. The documentation gestures at sandbox safety (`safety_checks: bool` in `PluginConfig`) but this only validates plugin dependency names, not capabilities.

**vs Nebula:** Nebula targets WASM sandbox with capability-based security for plugin isolation. rust-rule-engine's approach is appropriate for trusted, same-team plugins; Nebula's is appropriate for third-party plugin marketplace scenarios (Plugin Fund). Different goals.

### Decision 6: Stream processing via RETE integration (StreamAlphaNode)

**Decision:** Stream processing is implemented as RETE network extensions — `StreamAlphaNode`, `StreamBetaNode`, `StreamJoinNode` — that integrate with `WorkingMemory`. Events flow into alpha nodes with windowing logic, and matching events become facts in working memory for rule evaluation. GRL stream syntax extends the standard rule language.

**Trade-off:** Tight integration with the RETE network enables stream events to trigger the same rules as regular facts, without a separate streaming pipeline. Trade-offs: the `streaming` feature requires `tokio`, making the crate async-optional; the Redis state backend (`streaming-redis` feature) adds a production-grade distributed state option. Session windows (gap-based) add complexity but enable natural user session modeling.

**vs Nebula:** Nebula has no CEP/stream processing layer. This is an entirely new capability class. Nebula's TriggerAction subsystem handles inbound events but does not correlate them across time windows.

### Decision 7 (vs dataflow-rs comparison): Different DSL, different reasoning, shared library-first philosophy

Both rust-rule-engine and dataflow-rs are single-crate embeddable Rust libraries with no persistence or credential layer. The divergence is at the reasoning level: dataflow-rs is a priority-sorted sequential rule chain with JSONLogic conditions; rust-rule-engine is a proper production rules system (PRS) with RETE-UL, backward chaining, and stream processing. rust-rule-engine is 10x more complex and capable on the reasoning axis; dataflow-rs is simpler for pure IFTTT-style use cases. Neither has a server, neither has credentials, neither has multi-tenancy — they share the same class of "omissions" relative to Nebula.

---

## 18. Known Limitations / Pain Points

Data from DeepWiki query 9, RETE_ARCHITECTURE.md, CHANGELOG, and code analysis.

**Limitation 1: No thread safety by default (documented)**
The core `RustRuleEngine` and `BackwardEngine` are not thread-safe — users must wrap in `Arc<Mutex<>>` for concurrent access. This is explicitly tested in `tests/backward_thread_safety.rs`. RETE_ARCHITECTURE.md notes "Single-threaded execution (parallel RETE not implemented)" as a current limitation — despite `engine/parallel.rs` existing, the default execution path is sequential.

**Limitation 2: Parser robustness (v1.19.3 — 16 critical `.unwrap()` panics fixed)**
The GRL parser had 16 locations in `src/parser/grl.rs` and `src/parser/grl_no_regex.rs` where `.unwrap()` calls could panic on malformed input — fixed in v1.19.3 (2026-01-15). CHANGELOG entry: "This release makes the parser production-ready for handling untrusted or malformed GRL input without panicking." This indicates the parser was not production-hardened until v1.19.3 — recent for a library claiming "production-ready" since earlier versions.

**Limitation 3: Hot-reload plugin does not purge old closures**
`unload_plugin()` removes the plugin from `PluginManager.plugins` but does not remove the previously registered closures from `RustRuleEngine.custom_functions` or `action_handlers` HashMaps (`src/engine/engine.rs:1974-1993`). After a hot-reload, the old closure remains callable under its registered name until overwritten by the new plugin. This is a correctness gap for plugin hot-reload scenarios.

**Limitation 4: No persistent storage (documented planned feature)**
All state is in-memory. RETE_ARCHITECTURE.md lists "Persistent Storage" as a planned v1.2.0 feature. The `streaming/state.rs` module documents a `StateBackend::File` variant but describes it as "File-based state (persistent)" with the caveat that the default in-memory backend is "not persistent across restarts." No timeline for implementation.

**Limitation 5: `~95% CLIPS compatibility` (documented)**
RETE_ARCHITECTURE.md states "~95% compatibility with CLIPS, not 100%." Missing CLIPS features are not enumerated in the available documentation. This is relevant for users migrating from CLIPS-based expert systems.

**Limitation 6: Edition 2021 (not 2024)**
The crate uses `edition = "2021"` (`Cargo.toml:4`) — not Rust 2024 edition, unlike Nebula (2024). This means the crate doesn't benefit from the 2024 edition's `async`-in-trait improvements, which may explain why some async patterns in the `streaming` module use `tokio::spawn` rather than native async traits.

**Issue evidence:** GitHub issues list returned empty (zero issues filed). This may indicate very low community usage rather than a problem-free codebase. The absence of any filed issues on a crate with non-trivial complexity is statistically unusual.

---

## 19. Bus Factor / Sustainability

**Maintainers:** 2 active humans: `ttvuhm` (primary, 34/50 commits) and a GitHub alias identity (15 commits — same person via web interface). `nghiaphamln` contributed performance improvements in PR #66 (3 commits) and PR #67. Effectively bus factor 1.

**Commit cadence:** 50 commits span the depth-50 clone. The most recent cluster shows active development: v1.20.0 (performance optimization), v1.19.3 (parser hardening), v1.19.0 (in-operator), v1.18.28 (Unicode fix). The release cadence is ~2-3 releases per month, suggesting active development.

**Crates.io stats:** 8.6K total downloads, 2.7K recent. Created 2025-10-01 — 7 months old at analysis date. Download count includes CI bots. For a 7-month-old specialized library, 8.6K is modest.

**Assessment:** Bus factor 1. Young crate (7 months) with active single-maintainer development. No commercial backing, no foundation governance, no SOC 2. The rapid version cadence (v1.15-v1.20 in 7 months) suggests strong single-maintainer velocity but also potential for API churn. The zero-issue count on GitHub suggests minimal external community testing.

---

## 20. Final Scorecard vs Nebula

| Axis | rust-rule-engine approach | Nebula approach | Who's deeper / simpler / more correct | Borrow? |
|------|--------------------------|-----------------|---------------------------------------|---------|
| A1 Workspace | Single crate. Edition 2021. Feature flags: `streaming`, `streaming-redis`, `backward-chaining`. No layering. | 26 crates layered: nebula-error / nebula-resilience / nebula-credential / nebula-resource / nebula-action / nebula-engine / nebula-tenant / etc. Edition 2024. | **Nebula deeper** — 26-crate layering reflects deployment platform complexity. Single-crate is appropriate for a reasoning library. Different goals. | no — different goals |
| A2 DAG | No DAG at workflow level. Match-resolve-act cycle (RETE-style inference loop). RETE-UL network handles multi-fact joins. WorkflowEngine provides agenda-group-based control flow. | TypeDAG: L1 = static generics; L2 = TypeId; L3 = refinement predicates; L4 = petgraph soundness. | **Different decomposition** — RETE network is a join graph for fact pattern matching, not a user-visible workflow DAG. Neither dominates for their respective use cases. | no — different goals |
| A3 Action | `ActionType` enum (10 variants, closed). `RulePlugin` open trait for custom functions/actions. Zero associated types. Type erasure via `Value` and closure types. | 5 action kinds (Process/Supply/Trigger/Event/Schedule). Sealed trait. Assoc `Input`/`Output`/`Error`. Versioning via type identity. Derive macros via nebula-derive. | **Nebula deeper** — sealed traits + associated types give compile-time port-level safety. rust-rule-engine's type erasure is simpler but loses correctness guarantees across rule boundaries. | no — Nebula already better |
| A11 Plugin BUILD | No external build. Compile-time Rust trait implementation in same binary. `PluginMetadata` with string dependency validation. | WASM, plugin-v2 spec. Capability security. | **Nebula deeper** — WASM sandbox with capability model vs same-binary extension. rust-rule-engine's dependency validation is a step above dataflow-rs's zero metadata. | refine — `PluginMetadata.dependencies` validation pattern is worth preserving in Nebula's build-time manifest |
| A11 Plugin EXEC | In-process, no isolation, no WASM. `safety_checks` validates dependency names only. Hot-reload correctness gap (old closures not purged). | WASM sandbox + capability security. | **Nebula deeper** — zero sandbox enforcement in rust-rule-engine. Hot-reload correctness gap is a real bug. | no — different goals |
| A18 Errors | `RuleEngineError` enum (13 variants) with `thiserror = "2.0"`. `FeatureNotEnabled` variant for feature-flag errors. No `retryable()` classifier. | nebula-error crate. Contextual errors. `ErrorClass` enum (transient/permanent/cancelled). Used by ErrorClassifier in resilience. | **Nebula deeper** — ErrorClass + resilience integration vs unclassified error enum. `FeatureNotEnabled` variant is a clean pattern for feature-gated APIs. | refine — `FeatureNotEnabled { feature, message }` variant pattern is worth adopting in Nebula for feature-flag-gated errors |
| A21 AI/LLM | None in source. `.env.example` documents AI API env vars. `AI_INTEGRATION.md` shows integration pattern. Cargo.toml keywords include "ai", "ml". No SDK dependency. | No first-class LLM abstraction yet. Generic actions + plugin LLM client plan. Surge = agent orchestrator on ACP. | **Convergent** — neither has first-class LLM. Both leave AI to the embedding application / custom functions. rust-rule-engine's documentation gesture is ahead of Nebula's in explicitness of guidance, but has no code substance. | no — both aligned on "no first-class LLM" |

---

## Appendix A: Key code locations

| Concept | File | Lines |
|---------|------|-------|
| `ActionType` enum | `src/types.rs` | (primary) |
| `RulePlugin` trait | `src/engine/plugin.rs` | 48–70 |
| `PluginManager` | `src/engine/plugin.rs` | 102–248 |
| `RustRuleEngine::execute()` | `src/engine/engine.rs` | 444–625 |
| `RustRuleEngine::load_plugin()` | `src/engine/engine.rs` | 1962–1972 |
| `RustRuleEngine::hot_reload_plugin()` | `src/engine/engine.rs` | 1978–1993 |
| `RuleEngineError` enum | `src/errors.rs` | 5–101 |
| `CustomFunction` type alias | `src/engine/engine.rs` | 17 |
| `ActionHandler` type alias | `src/engine/engine.rs` | 20 |
| `ConditionExpression` enum | `src/engine/rule.rs` | 34–68 |
| `BackwardEngine` struct | `src/backward/backward_engine.rs` | 41–47 |
| `ProofGraph` | `src/backward/proof_graph.rs` | (520 lines) |
| `StreamAlphaNode` | `src/rete/stream_alpha_node.rs` | (main streaming node) |
| `WindowType` enum | `src/streaming/window.rs` | — |
| `RuleEngineBuilder` | `src/lib.rs` | 194–258 |

## Appendix B: Negative grep evidence

| Searched for | Result |
|-------------|--------|
| `credential\|secret\|vault\|zeroize\|secrecy\|oauth` in `src/` | zero results |
| `openai\|anthropic\|llm\|embedding\|gpt\|claude\|ollama` in `src/` | zero results |
| `wasm\|wasmtime\|wasmer\|wasmi\|libloading\|dlopen\|sandbox` in `src/` | zero results |
| `postgres\|sqlite\|mysql\|sqlx\|diesel\|rocksdb\|sled` in `src/` | zero results |
| `opentelemetry\|prometheus\|tracing` (as import) in `src/` | zero results |
| `tenant\|rbac\|workspace\|organization\|rls` in `src/` | zero results |
