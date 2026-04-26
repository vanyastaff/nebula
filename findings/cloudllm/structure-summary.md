# cloudllm — Structure Summary

## Crate count

2 Rust crates in a minimal workspace:
- `cloudllm` (root, v0.15.1) — primary library crate
- `cloudllm_mcp` (path `mcp/`, v0.1.0) — MCP protocol layer extracted for reuse

No feature-gating between domain layers; only one optional feature gate: `mcp-server` (axum + tower, for HTTP MCP server builder).

## Source module tree (cloudllm root)

```
src/cloudllm/
├── agent.rs           (1969 LOC) — Agent struct, tool loop, fork, event emission
├── client_wrapper.rs  (269 LOC)  — ClientWrapper trait (core LLM abstraction)
├── clients/
│   ├── claude.rs      (~200 LOC) — ClaudeClient via OpenAI-compat layer
│   ├── gemini.rs      (~600 LOC) — GeminiClient native HTTP
│   ├── grok.rs        (~380 LOC) — GrokClient (xAI)
│   └── openai.rs      (~600 LOC) — OpenAIClient (primary implementation)
├── config.rs          — env-var based config helpers
├── context_strategy.rs — ContextStrategy trait + TrimStrategy/SelfCompression/NoveltyAware
├── event.rs           — AgentEvent / OrchestrationEvent / EventHandler trait
├── image_generation.rs — ImageGenerationClient trait + 3 providers
├── llm_session.rs     (~400 LOC) — LLMSession (history, trimming, token accounting)
├── mcp_http_adapter.rs — bridges cloudllm agent to MCP over HTTP
├── mcp_server.rs      — MCPServer struct
├── mcp_server_builder.rs  — MCPServerBuilder
├── orchestration.rs   (2746 LOC) — Orchestration + OrchestrationMode enum (7 modes)
├── planner.rs         (1325 LOC) — Planner trait + BasicPlanner
├── resource_protocol.rs — re-exports mcp::resources (MCP resource support)
├── tool_protocol.rs   — re-exports mcp::protocol (ToolProtocol, ToolRegistry, etc.)
├── tool_protocols.rs  (1533 LOC) — CustomToolProtocol, MCPClientProtocol, MemoryProtocol
└── tools/             — built-in tools: bash, calculator, filesystem, http_client, memory
```

## LOC

Total Rust LOC: 39,410 (all .rs files including tests and examples).
- Source (`src/`): ~12,000 LOC estimated
- Tests (`tests/`): ~5,000 LOC
- Examples (`examples/`): ~13,000 LOC
- MCP sub-crate (`mcp/src/`): ~4,000 LOC

## Key dependencies

| Dep | Purpose |
|-----|---------|
| `tokio 1.48` | async runtime |
| `async-trait 0.1` | dyn async traits |
| `openai-rust2 1.7.2` | author's fork of OpenAI Rust client |
| `reqwest 0.12` | HTTP for Gemini/Grok native clients |
| `serde / serde_json` | JSON serialization |
| `mentisdb 0.4` | persistent agent memory (author's own crate) |
| `bumpalo 3.19` | arena allocator for message bodies |
| `evalexpr 13` | expression evaluation |
| `axum 0.8` (optional) | MCP HTTP server |
| `sha2 + subtle` | SHA-256 hashing for MentisDB integrity |

## Edition and toolchain

Edition 2018 (not 2024 — unlike Nebula's pinned 2024 + 1.95.0). No `rust-toolchain.toml` present.

## Test density

Approximately 14 test files (integration-heavy). No `#[cfg(test)]` unit-test blocks evident in source—tests are in separate `tests/` files. No property-based or contract tests.
