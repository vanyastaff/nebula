---
name: nebula-sandbox
role: Process Sandboxing (Correctness Boundary)
status: partial
last-reviewed: 2026-04-17
canon-invariants: [L1-4.5, L1-7.1, L1-12.6]
related: [nebula-plugin-sdk, nebula-plugin, nebula-runtime]
---

# nebula-sandbox

## Purpose

A workflow engine that dispatches to community plugins needs an isolation boundary between the
engine host and plugin code. That boundary must be honest about what it actually provides:
in-process execution for trusted built-in actions (cooperative cancellation and capability
checks, not OS-level isolation) and child-process execution for community plugins over a duplex
JSON envelope protocol (the trust model canon §12.6 names explicitly). `nebula-sandbox` defines
and owns both execution modes, the capability declaration model, the plugin discovery path, and
the OS-level hardening primitives — while being clear that **this is not a security boundary
against malicious native code**.

## Role

*Process Sandboxing (Correctness Boundary).* Provides `InProcessSandbox` (trusted dispatch
with cooperative cancellation) and `ProcessSandbox` (child-process JSON envelope broker per
ADR 0006). Correctness and least privilege for accidental misuse — not attacker-grade isolation
against malicious native code. Canon §12.6 is the normative statement.

## Public API

- `InProcessSandbox` — trusted in-process execution for built-in actions. No OS isolation.
  Capability checks via `SandboxedContext::check_cancelled`. Correctness boundary only.
- `ProcessSandbox` — child-process execution over a duplex line-delimited JSON envelope
  (ADR 0006 Phase 1). Long-lived plugin process; spawn cost paid once. Sequential dispatch
  within a single plugin process today.
- `ProcessSandboxHandler` — bridges `ProcessSandbox` into `ActionRegistry` so the runtime
  sees a unified `ActionExecutor`.
- `SandboxRunner`, `ActionExecutor`, `ActionExecutorFuture`, `SandboxedContext` — core sandbox
  runner abstraction used by `nebula-runtime`.
- `capabilities::PluginCapabilities` — iOS-style per-plugin capability declarations
  (network / filesystem / env allowlists). Defined but not yet wired from `plugin.toml` through
  discovery (see Appendix).
- `discovery` module — scans directories for plugin binaries via `plugin.toml` markers.
- `os_sandbox` module — OS-level hardening primitives (best-effort; per-platform status in
  Appendix).
- `SandboxError` — typed error.

## Contract

- **[L1-§12.6]** In-process sandbox / capability checks provide **correctness and least
  privilege for accidental misuse**, not a security boundary against malicious native code.
  `InProcessSandbox` is pure dispatch with cooperative cancellation.

- **[L1-§12.6]** Plugin IPC today is **sequential dispatch over a JSON envelope to a child
  process** (ADR 0006 slices 1a–1c). That is the trust model. Do not describe it as sandboxed
  execution of untrusted native code.

- **[L1-§12.6]** **WASM / WASI is an explicit non-goal.** The Rust plugin ecosystem
  (`redis`, `sqlx`, `rdkafka`, `tonic`, `*-sys` crates) does not compile to
  `wasm32-wasip2`. Offering WASM as "the future sandbox" is a §4.5 false capability and a §4.4
  DX regression. It must not appear as `planned` in any README or `lib.rs`.

- **[L1-§4.5]** `PluginCapabilities` enforcement from `plugin.toml` through discovery is a
  `false capability` until the `discovery.rs` TODO is closed — the allowlist is defined but
  unenforced today.

- **[L1-§7.1]** Plugin is the unit of registration. `ProcessSandbox` hosts the duplex broker;
  `nebula-plugin-sdk` is the plugin-author side. Wire protocol types live in the SDK because
  plugin authors link against them; the sandbox imports them back to speak the same protocol.

## Non-goals

- Not an attacker-grade isolation boundary against malicious native code.
- Not a WASM / WASI runtime — see §12.6 rationale.
- Not the action dispatcher — see `nebula-runtime` (drives this crate).
- Not the plugin trait / registry — see `nebula-plugin`.
- Not the plugin-author SDK — see `nebula-plugin-sdk`.

## Maturity

See `docs/MATURITY.md` row for `nebula-sandbox`.

- API stability: `partial` — `InProcessSandbox` and `ProcessSandbox` are in active use;
  capability enforcement and OS hardening backends are incomplete (see Appendix).
- `PluginCapabilities` allowlist is defined but unenforced — `false capability` (§4.5) until
  discovery wiring closes.
- `os_sandbox` per-platform backends are partial — check `src/os_sandbox.rs` before claiming
  any platform-specific hardening.
- ADR 0006 slice 1d (broker RPC, `PluginSupervisor`, reattach) is `proposed` / not yet landed.
- 3 panic sites — candidates for typed `SandboxError`.
- 0 integration tests; cancel path and protocol envelope covered only by unit tests.

## Related

- Canon: `docs/PRODUCT_CANON.md` §4.5, §7.1, §12.6.
- ADR: `docs/adr/0006-sandbox-phase1-broker.md` — duplex JSON-RPC over UDS / Named Pipe.
- Plugin model: `docs/INTEGRATION_MODEL.md` §7.
- Glossary: `docs/GLOSSARY.md` §4 (sandbox / resource).
- Siblings: `nebula-plugin-sdk` (plugin-author side / wire protocol), `nebula-plugin`
  (host-side registry), `nebula-runtime` (dispatches through sandbox runners).

## Appendix

### Discovery TODO (evicted from PRODUCT_CANON.md §12.6)

The `PluginCapabilities` enforcement path from `plugin.toml` through discovery is currently
marked TODO in `src/discovery.rs` (`discovery.rs:117` hardcodes
`PluginCapabilities::none()`). Closing this TODO is part of the isolation roadmap (canon §12.6)
and requires:

1. Parse capabilities from `plugin.toml` at plugin load.
2. Enforce capabilities in the broker on each dispatched call.
3. Test coverage proving a capability-deny is actually rejected end-to-end.

Until this lands, the capability allowlist is advertised but unenforced — a `false capability`
per canon §4.5. Consider surfacing `PluginCapabilities` as `experimental` in its doc comment
until the discovery-path TODO is closed.

### Real isolation roadmap (priority order, replacing any historical WASM language)

1. **Capability wiring** — close `discovery.rs:117` so `PluginCapabilities` is loaded from
   `plugin.toml` and enforced at `ProcessSandbox` boundaries.
2. **`plugin.toml` signing verification** — canon §7.1; tooling (`cargo-nebula` or equivalent)
   verifies signatures before the host trusts a plugin's `plugin.toml`.
3. **`os_sandbox` per-platform backends** — seccomp-bpf + landlock (Linux), `sandbox_init`
   (macOS), `AppContainer` / job objects (Windows). Ship per-platform as they stabilize.
4. **`ProcessSandbox` parallelism** — sequential dispatch (ADR 0006 slice 1c) is the current
   §12.6 reality. Bounded parallel dispatch per plugin (with a fair scheduler across plugins)
   is the §4.1 throughput win for real workloads.
5. **Integration tests for cancel path and protocol envelope** — canon §13 step 5 must be
   green end-to-end against `ProcessSandbox`, not only against `InProcessSandbox`.

### ADR 0006 status

ADR 0006 (`docs/adr/0006-sandbox-phase1-broker.md`) covers the Phase 1 duplex broker:

- Slices 1a (`c6b9d531`), 1b (`f3b6701b`), 1c (`b5723f28`) — **landed**: long-lived plugin
  process, duplex line-delimited JSON envelope over UDS / Named Pipe, cooperative cancel.
- Slice 1d — **proposed**: broker module (`crates/sandbox/src/broker/`), `PluginSupervisor`,
  RPC verbs (`credentials.get`, `network.http_request`, etc.), reattach on engine restart.

Until slice 1d lands, plugins cannot call back into the host for credentials, network, or
logging via the broker RPC. The `PluginCtx` in `nebula-plugin-sdk` is a placeholder.

### Architecture notes

- `ActionExecutor`, `SandboxRunner`, `InProcessSandbox`, `SandboxedContext` are owned here
  and re-exported by `nebula-runtime` via `pub use nebula_sandbox::...`. The legacy
  `nebula-runtime/src/sandbox.rs` shim was deleted in commit `eae0b54e`.
- Dependency on `nebula-plugin-sdk` (wire protocol types) is correct: this crate is the
  **host** of the duplex broker; the SDK is the **plugin** side.
