# Structure Summary — emergent-engine

## Repository Metadata

- **Repo**: https://github.com/govcraft/emergent
- **Stars**: 18 | **Forks**: 0 | **Open issues**: 13
- **Created**: 2026-01-06 | **Last activity**: 2026-04-26
- **License**: MIT OR Apache-2.0
- **Author**: Roland Rodriguez (roland@govcraft.ai)
- **Latest tag**: v0.10.9 (engine), sdks/*/v0.13.1

## Workspace Structure (7 Rust crates in workspace)

| Crate | Path | Purpose |
|-------|------|---------|
| `emergent-engine` | `emergent-engine/` | Core binary: process manager, IPC broker, event store, scaffold, marketplace |
| `emergent-client` | `sdks/rust/` | Rust SDK for building Source/Handler/Sink primitives |
| `timer` | `examples/sources/timer/` | Example source primitive |
| `filter` | `examples/handlers/filter/` | Example handler primitive |
| `exec-handler` | `examples/handlers/exec/` | Exec handler (stdin/stdout pipe wrapper) |
| `log` | `examples/sinks/log/` | Example log sink |
| `console` | `examples/sinks/console/` | Example console sink |

**Non-Rust SDKs** (outside workspace): TypeScript/Deno (`sdks/ts/`), Python 3.11+ (`sdks/py/`), Go 1.23+ (`sdks/go/`)

## Dependency Summary (emergent-engine)

**Core**:
- `acton-reactive = "8.1.1"` (features: ipc, ipc-messagepack) — actor framework, IPC, message routing
- `tokio = "1"` (full) — async runtime
- `axum = "0.8.8"` — HTTP API server
- `rusqlite = "0.32"` (bundled) — SQLite event store

**Serialization**: `serde`, `serde_json`, `rmp-serde`
**Config**: `toml = "0.8"`
**Errors**: `thiserror = "2"`, `anyhow = "1"`
**CLI**: `clap = "4.5.53"` (derive)
**Scaffold/DX**: `minijinja = "2"`, `dialoguer = "0.11"`, `heck = "0.5"`
**Marketplace**: `reqwest`, `flate2`, `tar`, `zip`, `sha2`, `indicatif`, `tempfile`
**System**: `nix = "0.30.1"` (signal), `directories = "5"`, `which = "8.0.2"`, `chrono = "0.4"`

## LOC Estimate

tokei not available in path. Estimated from file sizes:
- `emergent-engine/src/`: ~3,500 Rust lines (config.rs ~1,045, process_manager.rs ~463, primitive_actor.rs ~500+, marketplace/* ~1,000+)
- `sdks/rust/src/`: ~1,700 Rust lines (connection.rs ~1,346, message.rs, types/*)
- Total Rust (workspace): ~7,500–9,000 lines
- TypeScript SDK: ~600 lines
- Python SDK: ~700 lines
- Go SDK: ~800 lines

## Test Count (approximate)

- `emergent-engine`: ~40 unit tests (config.rs has 20+, event_store ~8, primitives ~3, process_manager ~2)
- `sdks/rust`: ~20 integration tests
- `sdks/py/tests/`: ~50 Python tests
- `sdks/go/`: ~30 Go tests
