# nebula-plugin-sdk — Claude Code orientation
> Agent quick-map for `crates/plugin-sdk/`. Full design: `README.md`. Repo-wide rules: root `CLAUDE.md`.

**Purpose:** Plugin-author-side SDK — implement `PluginHandler` + call `run_duplex` from `main` to speak the duplex line-delimited JSON envelope protocol to the host (`nebula-sandbox`).
**Layer:** Plugin-Proto — depends only on Core (`nebula-metadata`, `nebula-schema`) + tokio/serde; no engine-side deps (root CLAUDE.md -> Layered Dependency Map).

## Commands
- `cargo check -p nebula-plugin-sdk`
- `cargo nextest run -p nebula-plugin-sdk`  ·  doctests: `cargo test -p nebula-plugin-sdk --doc`
- Four `[[bin]]` fixtures (`echo`/`counter`/`schema`/`resend`) drive `nebula-sandbox` integration tests — keep them building.
- `NEBULA_PLUGIN_MAX_FRAME_BYTES` caps inbound host frame size (default 1 MiB).

## Key files
- `src/lib.rs` — `PluginHandler` trait, `PluginCtx`, `PluginError`, `run_duplex` entry point + sequential dispatch event loop.
- `src/protocol.rs` — wire envelopes `HostToPlugin`/`PluginToHost`, `DUPLEX_PROTOCOL_VERSION` (=3), `SDK_VERSION`; host imports these.
- `src/transport.rs` — `bind_listener`, `PluginListener`, `PluginStream` (UDS on Unix / Named Pipe on Windows) + stdout handshake line.
- `src/bin/*_fixture.rs` — test-plugin binaries for the host-side sandbox harness.

## Conventions & never-do
- **Dispatch is sequential (slice 1c)** — one action at a time, head-of-line blocking. `Cancel`/`RpcResponse*` envelopes are intentional no-ops until slice 1d; do not pretend concurrency or broker RPC exist yet.
- **Not isolation.** Trust model is sequential JSON dispatch to a child process (canon §12.6); never describe this as attacker-grade sandboxing. WASM is an explicit non-goal.
- **Core-layer dep exception (§7.1):** only `nebula-metadata` + `nebula-schema` are allowed for `PluginManifest`/`ValidSchema` on the wire. Any other cross-import is a layer violation — question it hard. Wire types live here (not `nebula-plugin`) because authors link against them.
- Envelopes must serialize to a single line — never emit raw newlines; the framing invariant is test-locked.
- Cross-crate calls go through `nebula-eventbus`, not direct sibling imports.
- Library code uses typed `thiserror`/`NebulaError`; no panicking unwrap/expect/panic in lib code.

## See also
- `README.md` — full design · ADR-0006 (`docs/adr/HISTORICAL.md`, duplex JSON-RPC over UDS/Named Pipe) · `docs/INTEGRATION_MODEL.md` §7 · `docs/PRODUCT_CANON.md` §7.1/§12.6 · siblings `nebula-sandbox` (host), `nebula-plugin` (registry).
