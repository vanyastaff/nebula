# DeepWiki Findings — fluxus

## Query 1: Core trait hierarchy for actions/nodes/activities

**Question:** What is the core trait hierarchy for actions/nodes/activities? How are Source, Sink, and Operator traits defined and composed?

**Answer (summarized):**
Three `async_trait`-based traits: `Source<T>` (init/next/close), `Sink<T>` (init/write/flush/close), `Operator<In, Out>` (init/process/on_window_trigger/close). `DataStream<T>` provides the fluent builder API. `TransformSourceWithOperator` composes operators lazily into a new source for pull-based evaluation. `RuntimeContext` drives parallel execution via tokio::spawn + mpsc channels. All three use `async_trait` macro because Rust lacks native async-in-traits (note: project uses 1.88.0 where this is improving but `async_trait` still in use).

---

## Query 4: Plugin system / WASM / dynamic loading

**Question:** How are plugins or extensions implemented? Is there a plugin system, WASM sandbox, or dynamic loading mechanism?

**Answer (summarized):**
No dedicated plugin system, no WASM sandbox, no dynamic loading. Extensibility is compile-time only: separate crates in the Cargo workspace, selectable via feature flags on the umbrella `fluxus` crate. The `rust-toolchain.toml` includes `wasm32-unknown-unknown` as a *compilation target* (for running the engine itself in a browser context), not as a plugin sandbox. No `wasmtime`, `wasmer`, `libloading`, or equivalent dependency found in any `Cargo.toml`.

---

## Query 7: LLM / AI agent integration

**Question:** Is there built-in LLM or AI agent integration? What AI providers or abstractions are supported?

**Answer (summarized):**
No LLM or AI agent integration whatsoever. Project focuses on general-purpose stream processing. No references to OpenAI, Anthropic, Hugging Face, embeddings, completions, or related libraries in codebase or Cargo.lock. AI functionality could be added as a custom `Operator` implementation but the framework provides no native abstractions.

---

## Query 9: Known limitations / planned redesigns

**Question:** What known limitations or planned redesigns are documented? What is on the roadmap?

**Answer (summarized):**
Two explicitly planned features:
1. **State checkpointing** — planned for `fluxus-core` state management
2. **Runtime diagnostics** — planned for `fluxus-runtime` monitoring

Issues reveal community interest in: stateless/stateful operator classification (#89), `cargo fluxus-init` scaffolding tool (#81), `fluxus-console` real-time monitoring (#77), PostgreSQL/Cassandra/ClickHouse sources (#33, #32, #49), blockchain data sources (#64, #65, #66), and Python DataFrame integration (#50).
