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

<!-- reviewed: 2026-04-09 — Phase 7.5: ProcessSandboxHandler migrated from InternalHandler to StatelessHandler, discovery returns ActionHandler tuples -->
<!-- reviewed: 2026-04-11 — Import paths migrated off `nebula_action::handler::X` (aliases deleted upstream in action crate post-audit). `discovery.rs` and `handler.rs` now import `ActionHandler`, `ActionMetadata`, `StatelessHandler` from the `nebula_action` crate root. Zero behavior change. -->
<!-- reviewed: 2026-04-11 — `capabilities::path_under` rewritten: was string-concat on `canonicalize().unwrap_or_else(|_| str::to_owned)`, which broke on Windows because `canonicalize` returned `\\?\C:\tmp` while the fallback kept POSIX `/tmp`, and component containment was faked with `format!("{base}/")`. New impl uses `std::path::Path` + component-wise `starts_with`. If both paths canonicalize, OS-resolved forms are compared (traversal-safe via `/tmp/../etc` → `/etc`). If either fails, both are run through a lexical `normalize_lex` that drops `CurDir` and pops on `ParentDir`, then component `starts_with` — symmetric, sep-agnostic, still traversal-safe, works on Windows where `/tmp` isn't a real path. Two previously-flaky tests (`filesystem_read_only`, `filesystem_write_implies_read`) now pass on both targets. -->
