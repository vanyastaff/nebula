# flowlang — Structure Summary

## Workspace layout

Single-crate project (not a workspace). One `Cargo.toml` at root.
No sub-crates; extension points compiled as optional features or as
separate Cargo workspaces that users create alongside.

## Crate count
1 main crate (`flowlang`), with one companion dependency (`ndata`) for
the internal heap.

## Binaries
- `flow` — CLI interpreter (`src/main.rs`)
- `flowb` — Builder / code-generator (`src/build.rs`)
- `flowmcp` — MCP stdio server (`src/flowmcp.rs`)

## Key modules inside `src/`
| Module | Purpose |
|--------|---------|
| `datastore` | File-system JSON store + global ndata heap |
| `command` | Command dispatch (flow / rust / python / java / js) |
| `code` | Flow graph interpreter (`Code::execute`) |
| `case` | Data structs: Case / Operation / Connection / Node |
| `primitives` | ~50 built-in primitive operations |
| `rustcmd` | Function-pointer registry for Rust commands |
| `pycmd` / `pyenv` / `pywrapper` | Python runtime bridge (pyo3) |
| `jscmd` | JavaScript runtime bridge (deno_core) |
| `javacmd` | Java bridge (jni) |
| `appserver` | HTTP server + event/timer loops |
| `builder/` | Code-generation system for Rust/Python commands |
| `mcp/` | MCP JSON-RPC server (tools/list, tools/call) |
| `x25519` | Hand-rolled X25519 DH (no external crypto dep) |

## Top-10 dependencies (from Cargo.toml)
1. `ndata` 0.3.16 — internal dynamic heap (always)
2. `libc` 0.2 — FFI / dlopen (always)
3. `pyo3` 0.21.2 — Python embedding (feature: python_runtime)
4. `deno_core` 0.249.0 — JS/V8 embedding (feature: javascript_runtime)
5. `serde_v8` 0.158.0 — serde bridge for V8 (feature: javascript_runtime)
6. `jni` 0.21.1 — JVM embedding (feature: java_runtime)
7. `serde` 1 — serialization (feature: serde_support)
8. `serde_json` 1 — JSON (feature: serde_support)
9. `gag` 1.0 — stdout suppression for MCP mode (feature: gag)
10. (none after 9 — very lean deps)

## LOC
Total Rust source: ~11,244 lines across 122 .rs files.
Root-level .rs files: ~6,001 lines.

## Test count
No test files or `#[test]` attributes found in source scan.
The `data/testflow` directory contains JSON-defined flow programs that
serve as functional test data.

## Notable: no async runtime
flowlang uses `std::thread::spawn` directly; no tokio / async-std.
