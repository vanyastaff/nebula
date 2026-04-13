# nebula-sandbox
Plugin isolation and sandboxing — SandboxRunner trait and implementations.

## Invariants
- `SandboxRunner` is the common interface for all action execution.
- `InProcessSandbox` — trusted, in-process. For built-in actions.
- `ProcessSandbox` — child process, stdin/stdout JSON. For community plugins. Timeout + kill_on_drop.
- **Phase 0 dispatch (2026-04-13)**: `ActionRuntime::execute_stateless` now routes `CapabilityGated`/`Isolated` through `self.sandbox.execute()`. Stateful isolation still fail-closes — needs the Phase 1 broker loop. 3177 workspace tests green.
- **Permission manifest model deferred**: see `docs/plans/2026-04-13-sandbox-roadmap.md` §D4. `plugin.toml` in its final form is only `[plugin]` + `[signing]` (Phase 4). For Phase 0–1 the engine wires plugins via code. Defense is: process isolation, broker RPC with anti-SSRF + audit log, OS jail (Phase 2+), signed manifest (Phase 4).

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

<!-- reviewed: 2026-04-13 — Phase 0 dispatch landed -->

## Sandbox roadmap (target state)
- **Phase 0** ✅ — Stateless dispatch unblocked.
- **Phase 1** — gRPC broker over UDS/TCP-loopback, AutoMTLS, Reattach, `PluginSupervisor` long-lived per `(ActionKey, credential_scope)`, `nebula-plugin-protocol` v2 (protobuf + tonic). Replaces current one-shot stdio JSON. Spec: `docs/plans/2026-04-13-sandbox-phase1-broker.md`.
- **Phase 2** — Linux Tier 1 hardening (seccompiler + landlock + cgroups-rs + rustix + caps).
- **Phase 3** — macOS (sandbox-exec + `responsibility_spawnattrs_setdisclaim`) + Windows (AppContainer via vendored `rappct`, `win32job`, WFP provider with admin install).
- **Phase 4** — ed25519 signed minimal manifest (`sdk-version` + `binary-sha256`), registry, desktop install dialog, no capability grant flow.
- **Phase 5** — Device broker (camera/mic/clipboard/shortcut/keyboard) via shared memory rings, Tauri delegation on desktop.
- Full research & decisions: `.project/context/research/sandbox-prior-art.md`.
