# nebula-sandbox
Plugin isolation and sandboxing — SandboxRunner trait and implementations.

## Invariants
- `SandboxRunner` is the common interface for action execution within isolation boundaries.
- `InProcessSandbox` — trusted, in-process execution for built-in actions. No real isolation.
- All sandbox types are `Send + Sync`. Async via `async_trait`.

## Key Decisions
- Extracted from `nebula-runtime` to isolate sandbox concerns. Runtime re-exports for backward compat.
- Two planned implementations: `InProcessSandbox` (built-in, trusted) and `WasmSandbox` (community, wasmtime, feature-gated `wasm`).
- WASM chosen over native FFI (stabby/libloading) — sandboxing built-in, portable `.wasm` files, no ABI issues.
- `SandboxedContext` wraps `ActionContext` with capability checks (cancellation).

## Traps
- `ActionExecutor` is a type alias for a boxed closure, not a trait — used by `InProcessSandbox` only.
- WASM feature not yet implemented — `wasm` feature gate reserved in Cargo.toml.

## Relations
- Depends on nebula-action. Used by nebula-runtime (re-export), nebula-engine (via runtime).

<!-- created: 2026-04-08 -->
