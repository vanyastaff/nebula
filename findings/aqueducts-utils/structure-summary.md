# aqueducts-utils — Structure Summary

## Crate count and names

8 workspace members:

| Path | Crate name | Role |
|------|-----------|------|
| `aqueducts/meta` | `aqueducts` | Umbrella / thin re-export with feature flags |
| `aqueducts/core` | `aqueducts-core` | Pipeline execution engine |
| `aqueducts/schemas` | `aqueducts-schemas` | Config types and validation |
| `aqueducts/delta` | `aqueducts-delta` | Delta Lake integration (optional) |
| `aqueducts/odbc` | `aqueducts-odbc` | ODBC integration (optional) |
| `aqueducts-cli` | `aqueducts-cli` | CLI binary |
| `aqueducts-executor` | `aqueducts-executor` | Remote executor server binary |
| `tools/schema-generator` | _(internal tool)_ | JSON Schema generation |

## Layer separation

- Layer 0 (schemas): `aqueducts-schemas` — pure data types, no I/O, serde/schemars derives, no circular deps.
- Layer 1 (core engine): `aqueducts-core` — SQL execution against DataFusion SessionContext; registers sources, processes stages, writes destinations.
- Layer 2 (providers): `aqueducts-delta`, `aqueducts-odbc` — optional provider crates enabled by feature flags.
- Layer 3 (umbrella): `aqueducts` meta crate — re-exports core + providers through a unified `prelude`.
- Layer 4 (binaries): `aqueducts-cli`, `aqueducts-executor` — user-facing executables.

## Feature flags

Core flags (aqueducts-core): `s3`, `gcs`, `azure`, `odbc`, `delta`, `json`, `yaml` (default), `toml`, `custom_udfs`.
Umbrella (aqueducts meta): all of the above plus `schema_gen`, `protocol`.

## Top-level dependencies

- `datafusion = "51"` — SQL engine (dominant dep)
- `deltalake` — git-pinned Delta Lake
- `arrow-odbc = "21"` — ODBC Arrow bridge
- `tokio = "1"` — async runtime
- `axum = "0.8.7"` — executor HTTP server
- `tokio-tungstenite = "0.26"` — WebSocket streaming
- `serde` / `serde_json` / `serde_yml` / `toml` — multi-format deserialization
- `thiserror = "2"` + `miette = "7.6"` — error handling
- `bon = "3.8"` — builder derives
- `schemars = "0.8"` — JSON Schema generation

## LOC

47 `.rs` files, ~9,120 total lines (excluding `target/`). The codebase is compact.

## Test count

Integration test files: `aqueducts/core/tests/integration.rs`, `aqueducts/delta/tests/integration.rs`, `aqueducts/schemas/tests/integration.rs`. Unit tests inline in `executor/manager.rs` (4 tests). Total tests: ~20-30 test cases across crates.
