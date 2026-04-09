# nebula-sandbox
Plugin isolation and sandboxing — SandboxRunner trait and implementations.

## Invariants
- `SandboxRunner` is the common interface for all action execution.
- `InProcessSandbox` — trusted, in-process. For built-in actions.
- `ProcessSandbox` — child process, stdin/stdout JSON. For community plugins. Timeout + kill_on_drop.

## Key Decisions
- **Process isolation over WASM.** WASM rejected: most Rust I/O libs (tokio/reqwest/teloxide) don't compile to WASM. Process isolation lets plugins use any library.
- Plugin response protocol: `{"output": {...}}` or `{"error": "...", "code": "...", "retryable": bool}`.
- Permissions model in `permissions.rs`: network (domain allowlist), fs, env, credentials. OS enforcement (seccomp) planned.

## Traps
- `ProcessSandbox` spawns a new process per execution call. Pooling not implemented.
- `PluginResponse` uses `#[serde(untagged)]` — order of variants matters for deserialization.
- Permissions are defined but not yet enforced at OS level (seccomp). Currently advisory.

## Relations
- Depends on nebula-action. Used by nebula-runtime (re-export), nebula-engine (via runtime).

<!-- reviewed: 2026-04-09 — fixed Capability import behind cfg(linux) in os_sandbox.rs -->
