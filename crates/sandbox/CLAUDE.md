# nebula-sandbox — Claude Code orientation
> Agent quick-map for `crates/sandbox/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** Host side of the duplex JSON-envelope child-process transport to community plugins (`ProcessSandbox`), plus the credential-scope identity (`ScopeHash`) and Linux OS-hardening primitives.
**Layer:** Plugin-Proto (leaf) — depends only on Core (`nebula-metadata`) + the plugin protocol (`nebula-plugin-sdk`); no Business/Exec dependency (root CLAUDE.md -> Layered Dependency Map).

## Commands
- `cargo check -p nebula-sandbox`
- `cargo nextest run -p nebula-sandbox`  ·  doctests: `cargo test -p nebula-sandbox --doc`
- `discovery_schema_roundtrip` is `#[ignore]`-gated (needs a pre-built plugin fixture); `os_sandbox` Landlock/rlimit code is `cfg(target_os = "linux")` only.

## Key files
- `src/lib.rs` — `#![deny(unsafe_code)]`, `#![warn(missing_docs)]`; re-exports `ProcessSandbox`, `SandboxError`, `scope::{ScopeHash, scope_hash}`.
- `src/dispatch.rs` — `ProcessSandbox`: lazy spawn + duplex envelope round-trip with per-call timeout + cancel race.
- `src/scope.rs` — pure `ScopeHash` / `scope_hash` from caller slot-name strings (ADR-0025 §2); never sees a workflow node.
- `src/os_sandbox.rs` — Linux Landlock (fixed system paths, fail-closed) + `setrlimit`, applied fork-safely via `PreparedSandbox`; no-op elsewhere.
- `src/spawn.rs` · `src/handshake.rs` · `src/codec.rs` — process spawn, socket dial/handshake, line-delimited JSON framing.
- `src/error.rs` — typed `SandboxError`.

## Conventions & never-do
- **Transport only, NOT a security boundary** against malicious native code (canon §12.6): provides correctness + cooperative cancel, not attacker-grade isolation.
- **WASM / WASI is an explicit non-goal** (§12.6) — never list it as `planned` in README or `lib.rs`.
- **No per-plugin capability/scope surface** — egress/credential/filesystem mediation is the broker's job (ADR-0025 slice 1d, unbuilt). Do not re-add the removed `PluginCapabilities`.
- Do NOT own these — they live elsewhere: discovery + `RemoteAction`/`ProcessSandboxHandler` + `SandboxError`→`ActionError` map = `nebula-plugin`; `SandboxRunner`/`SandboxedContext`/`InProcessSandbox` = `nebula-engine`.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design, isolation roadmap, ADR 0006 status · ADR-0006 / ADR-0025 (`docs/adr/HISTORICAL.md`) · canon `docs/PRODUCT_CANON.md` §4.5/§7.1/§12.6 · `docs/INTEGRATION_MODEL.md` §7.
