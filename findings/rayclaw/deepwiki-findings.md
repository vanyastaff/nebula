# DeepWiki Findings — rayclaw/rayclaw

Queries executed: 4/4 (all succeeded)

---

## Query 1: "What is the core trait hierarchy for actions/nodes/activities?"

Result: The core trait is `Tool` in `src/tools/mod.rs`. Methods: `name()`, `definition()` (returns `ToolDefinition` with name/description/JSON Schema), `execute(serde_json::Value) -> ToolResult`. No associated types beyond the concrete return types. `ToolRegistry` manages a `Vec<Box<dyn Tool>>`. `ToolDefinition` uses `serde_json::Value` for input_schema — fully dynamic, no compile-time generics.

---

## Query 4: "How are plugins or extensions implemented (WASM/dynamic/static)?"

Result: No WASM sandbox. Two extension mechanisms:
1. **Skills** — markdown files (`SKILL.md`) in `rayclaw.data/skills/`, activated via `activate_skill` tool, instructions injected into context.
2. **ACP (Agent Client Protocol)** — external agents spawned as subprocesses via `build_spawn_command` in `src/acp.rs`. JSON-RPC/stdio. Example: `npx @anthropic-ai/claude-code`.
`Cargo.lock` shows `wasm-bindgen`/`wasi` deps but these are transitive, not used for plugin execution.

---

## Query 7: "Is there built-in LLM or AI agent integration? What providers and abstractions are supported?"

Result: `LlmProvider` trait in `src/llm.rs` — `send_message()` + `send_message_stream()`. Three concrete providers: `AnthropicProvider`, `OpenAiProvider` (covers OpenAI, OpenRouter, DeepSeek, Groq, Ollama, Azure, Bedrock, Zhipu, Moonshot, Mistral, Together, Tencent, XAI, Huggingface, Cohere, Minimax, Alibaba), `BedrockProvider` (SigV4 auth). `create_provider(config)` selects at runtime. Core agent loop: per-chat mutex → build context → LLM call → tool dispatch → persist to SQLite → repeat.

---

## Query 9: "What known limitations or planned redesigns are documented?"

Result:
1. **Hot-reload plan** (`RELOAD.md`): design doc for self-healing bot; not yet implemented. Missing: Feishu→restart trigger, auto-rollback, ACP integration.
2. **MCP SDK migration** (`docs/mcp-sdk-evaluation.md`): current custom `src/mcp.rs` may migrate to official Rust MCP SDK behind opt-in flag.
Key trade-offs: session-resume via full message serialization, SQLite WAL for concurrent reads, `LlmProvider` abstraction for provider flexibility, channel-agnostic core, dual memory system (file + SQLite).
