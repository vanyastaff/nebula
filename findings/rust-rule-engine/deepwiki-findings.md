# DeepWiki Findings — rust-rule-engine

## Query 1: Core trait hierarchy for actions/nodes/activities

**Question:** "What is the core trait hierarchy for actions/nodes/activities? What is the RulePlugin trait shape, its associated types, and how do custom functions/actions get registered?"

**Response summary:**
- `ActionType` enum in `src/types.rs` defines action kinds: Set, Log, MethodCall, Retract, Custom, ActivateAgendaGroup, ScheduleRule, CompleteWorkflow, SetWorkflowData, Append.
- `RulePlugin` trait: `get_metadata()`, `register_actions(engine)`, `register_functions(engine)`, `unload()`, `health_check()` — no associated types.
- Custom functions registered via `engine.register_function(name, closure)` where closure is `Fn(&[Value], &Facts) -> Result<Value> + Send + Sync + 'static`.
- Custom action handlers registered via `engine.register_action_handler(action_type, closure)`.
- Both `RustRuleEngine` and `IncrementalEngine` have separate `register_function` methods with different signatures.
- DeepWiki flagged that RETE_ARCHITECTURE.md does not detail a trait hierarchy for Alpha/Beta nodes — they are internal implementation structs, not user-facing traits.

**Raw result:** https://deepwiki.com/search/what-is-the-core-trait-hierarc_151d714d-9afa-4ac7-aad3-9d1de6ace8a2

---

## Query 4: Plugin/extension implementation (WASM/dynamic/static)

**Question:** "How are plugins or extensions implemented? Where do plugins compile and where do they execute? Is there any WASM, dynamic library, or subprocess sandbox involved?"

**Response summary:**
- Plugins are Rust traits compiled into the same binary as the main application — no separate compilation step.
- `RulePlugin` trait + `PluginManager` provides load/unload/hot-reload/health-check lifecycle.
- Plugins execute directly in the same process and memory space — no WASM, no dynamic libraries (.so/.dll), no subprocess sandboxing.
- "The codebase does not contain any references to WebAssembly (WASM), dynamic libraries (like .so or .dll files), or subprocess sandboxing mechanisms for plugins."
- Examples are in Makefile: `advanced_plugins_showcase`, `builtin_plugins_demo`, `plugin_system_demo`.

**Raw result:** https://deepwiki.com/search/how-are-plugins-or-extensions_c8836b8b-f2cf-4b77-b329-3d80f369c16b

---

## Query 7: Built-in LLM or AI agent integration

**Question:** "Is there built-in LLM or AI agent integration? What providers and abstractions are supported? Is there any OpenAI, Anthropic, embedding, or vector store integration?"

**Response summary:**
- DeepWiki cited `.env.example` and `docs/examples/AI_INTEGRATION.md` as evidence of AI integration, claiming "the system is configured to work with OpenAI and Anthropic."
- IMPORTANT: This is documentation-only guidance. No actual `reqwest`, OpenAI SDK, or Anthropic SDK exists in `Cargo.toml` or any `.rs` source file. DeepWiki correctly noted "The Cargo.toml file lists reqwest with the json feature in dev-dependencies" — but this was in an older version; current Cargo.toml shows reqwest is NOT a dependency.
- "There is no explicit built-in integration for embedding generation or vector store functionalities."
- The actual AI "integration" is a pattern description: implement `CustomFunction` as a closure that calls an HTTP endpoint.
- Keywords in Cargo.toml include "ai" and "ml" — this is positioning, not capability.

**Assessment:** DeepWiki's response overstated the AI integration by treating documentation examples as shipped features. Negative grep in actual source code confirms zero AI SDK dependencies.

**Raw result:** https://deepwiki.com/search/is-there-builtin-llm-or-ai-age_7a5017d8-3ce7-4a2c-83fe-d48a78134a6b

---

## Query 9: Known limitations or planned redesigns

**Question:** "What known limitations or planned redesigns are documented?"

**Response summary:**
- Not thread-safe by default — requires `Arc<Mutex<>>` wrapping.
- Alpha Memory Indexing: writes 5-10% slower due to index maintenance.
- Node Sharing: setup 2x slower.
- Alpha Compaction: 5-10% slower insertion.
- Token Pooling: rarely beneficial for typical workloads.
- Concurrency contradiction: RETE_ARCHITECTURE.md lists both "Single-threaded execution (parallel RETE not implemented)" AND "Parallel RETE - Multi-threaded evaluation (IMPLEMENTED)" — DeepWiki correctly flagged this as a discrepancy.
- No persistent storage (in-memory only).
- ~95% CLIPS compatibility.
- No GUI rule builder.
- Basic infinite loop detection (max iterations only).
- Planned for v1.2.0: Persistent Storage, JIT rule compilation, REST API, Distributed Execution.
- Bus factor: effectively 1 primary maintainer.

**Raw result:** https://deepwiki.com/search/what-known-limitations-or-plan_1bc8fc8d-397a-41f9-9586-60a5a15a6e3d

---

## Query fail-stop check

All 4 queries returned substantive responses. No 3-fail-stop triggered. DeepWiki queries: 4/4 completed.
