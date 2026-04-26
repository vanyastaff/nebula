# aofctl (agenticdevops/aof) — Structure Summary

## Workspace composition

- **Total crates:** 17 workspace members
- **Primary binary:** `aofctl` (CLI)
- **Core library crates:** `aof-core`, `aof-llm`, `aof-runtime`, `aof-mcp`, `aof-memory`, `aof-triggers`, `aof-tools`, `aof-coordination`, `aof-coordination-protocols`, `aof-conversational`, `aof-skills`, `aof-personas`, `aof-gateway`, `aof-viz`
- **Test/infra crates:** `smoke-test-mcp`, `test-trigger-server`

## Dependency graph (key edges)

```
aofctl → aof-runtime → aof-core
aof-runtime → aof-coordination
aof-runtime → aof-memory
aof-runtime → aof-llm (via config, not explicit dep? — resolved at aofctl level)
aof-llm → aof-core
aof-triggers → aof-core
aof-tools → aof-core
aof-mcp → aof-core
aof-gateway → aof-runtime, aof-core
```

## Top-10 workspace dependencies

| Dependency | Purpose |
|------------|---------|
| `tokio` (full) | Async runtime |
| `serde` + `serde_json` + `serde_yaml` | YAML/JSON serialization |
| `reqwest` (json, stream) | HTTP client for LLM API calls |
| `async-trait` | Async trait methods (Rust 2021 era) |
| `thiserror` | Error enum derivation |
| `tracing` + `tracing-subscriber` | Structured logging |
| `clap` (derive) | CLI argument parsing |
| `anyhow` | Error handling in examples/tests |
| `prometheus` | Metrics |
| `bollard` | Docker API (sandbox isolation) |

## LOC estimate

tokei was not available in PATH. Rough estimate from crate count and file sizes:
- `aof-core/src/`: ~7 source files × ~400 lines avg = ~2800 lines
- `aof-runtime/src/`: ~25 source files × ~300 lines avg = ~7500 lines
- `aof-llm/src/`: ~5 source files × ~300 lines = ~1500 lines
- `aof-triggers/src/`: ~20 source files × ~200 lines = ~4000 lines
- `aof-tools/src/`: ~20 source files × ~300 lines = ~6000 lines
- Other crates: ~5000 lines
- **Estimated total: ~30K lines of Rust**

## Test count

Tests are co-located in each crate. Significant test files:
- `aof-core/src/*.rs` — inline `#[cfg(test)]` modules in every file
- `aof-personas/tests/` — integration + e2e (900+ line test files)
- `aof-conversational/tests/` — persistence tests
- `aofctl/tests/` — CLI integration tests
- Estimated: ~200-400 test functions workspace-wide

## Key observations

1. `rust-version = "1.75"` — significantly older than Nebula's pinned 1.95. `async_trait` usage confirms pre-stable AFIT development.
2. `async-trait` in workspace deps indicates trait objects over native async fn in traits.
3. No `sqlx`, no `pg`, no database dependency — persistence is in-memory or file.
4. `bollard` dependency indicates Docker API integration for sandbox.
5. `redis` dependency in workspace (for planned horizontal scaling, not yet wired to all features).
