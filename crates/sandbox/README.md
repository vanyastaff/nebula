---
name: nebula-sandbox
role: Process Sandboxing (Correctness Boundary)
status: partial
last-reviewed: 2026-05-15
canon-invariants: [L1-4.5, L1-7.1, L1-12.6]
related: [nebula-plugin-sdk, nebula-plugin, nebula-runtime]
---

# nebula-sandbox

## Purpose

A workflow engine that dispatches to community plugins needs an isolation boundary between the
engine host and plugin code. That boundary must be honest about what it actually provides:
in-process execution for trusted built-in actions (cooperative cancellation only, not OS-level
isolation) and child-process execution for community plugins over a duplex JSON envelope
protocol (the trust model canon §12.6 names explicitly). `nebula-sandbox` defines and owns both
execution modes, the plugin discovery path, and the Linux OS-level hardening primitives. There
is **no per-plugin capability/scope model** — egress, credential, and filesystem mediation is
the broker's responsibility (ADR-0025), not this crate — and **this is not a security boundary
against malicious native code**.

## Role

*Process Sandboxing (Correctness Boundary).* Provides `InProcessSandbox` (trusted dispatch
with cooperative cancellation) and `ProcessSandbox` (child-process JSON envelope broker per
ADR 0006). Correctness and least privilege for accidental misuse — not attacker-grade isolation
against malicious native code. Canon §12.6 is the normative statement.

## Public API

- `InProcessSandbox` — trusted in-process execution for built-in actions. No OS isolation;
  cooperative cancellation via `SandboxedContext::check_cancelled`. Correctness boundary only.
- `ProcessSandbox` — child-process execution over a duplex line-delimited JSON envelope
  (ADR 0006 Phase 1). Long-lived plugin process; spawn cost paid once. Sequential dispatch
  within a single plugin process today.
- `ProcessSandboxHandler` — bridges `ProcessSandbox` into `ActionRegistry` so the runtime
  sees a unified `ActionExecutor`.
- `SandboxRunner`, `ActionExecutor`, `ActionExecutorFuture`, `SandboxedContext` — core sandbox
  runner abstraction used by the engine runtime.
- `discovery` module — scans directories for plugin binaries via `plugin.toml` markers.
- `os_sandbox` module — Linux Landlock (fixed system paths, best-effort, fail-closed) plus
  `setrlimit` child caps, applied fork-safely via `PreparedSandbox`. No-op on non-Linux.
- `SandboxError` — typed error.

## Contract

- **[L1-§12.6]** In-process execution provides **correctness and cooperative cancellation**,
  not a security boundary against malicious native code. `InProcessSandbox` is pure dispatch.

- **[L1-§12.6]** Plugin IPC today is **sequential dispatch over a JSON envelope to a child
  process** (ADR 0006 slices 1a–1c). That is the trust model. Do not describe it as sandboxed
  execution of untrusted native code.

- **[L1-§12.6]** **WASM / WASI is an explicit non-goal.** The Rust plugin ecosystem
  (`redis`, `sqlx`, `rdkafka`, `tonic`, `*-sys` crates) does not compile to
  `wasm32-wasip2`. Offering WASM as "the future sandbox" is a §4.5 false capability and a §4.4
  DX regression. It must not appear as `planned` in any README or `lib.rs`.

- **[L1-§4.5]** No per-plugin capability/scope surface is advertised. The previously-shipped
  `PluginCapabilities` enum was unenforced on the dispatch path and has been removed (roadmap
  §Delete + ADR-0025 D4). Egress, credential, and filesystem mediation is the broker's
  (ADR-0025), introduced when slice 1d lands — not a passthrough allowlist here.

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

- API stability: `partial` — `InProcessSandbox` and `ProcessSandbox` are in active use; the
  broker (egress / credential / scope mediation) is not yet built (see Appendix).
- No per-plugin capability/scope surface (removed; the broker owns scope per ADR-0025).
- `os_sandbox` is Linux-only: Landlock fixed system paths + `setrlimit`, best-effort and
  fail-closed, applied fork-safely. No macOS/Windows OS confinement — `is_available()`
  reports this honestly.
- ADR 0006 slice 1d (broker RPC, `PluginSupervisor`, reattach) is `proposed` / not yet landed.
- 1 integration test (`discovery_schema_roundtrip`, `#[ignore]`-gated — requires pre-built
  fixture); cancel path and protocol envelope covered only by unit tests.

## Related

- Canon: `docs/PRODUCT_CANON.md` §4.5, §7.1, §12.6.
- ADR: `docs/adr/0006-sandbox-phase1-broker.md` — duplex JSON-RPC over UDS / Named Pipe.
- Plugin model: `docs/INTEGRATION_MODEL.md` §7.
- Glossary: `docs/GLOSSARY.md` §4 (sandbox / resource).
- Siblings: `nebula-plugin-sdk` (plugin-author side / wire protocol), `nebula-plugin`
  (host-side registry), `nebula-runtime` (dispatches through sandbox runners).

## Appendix

### Real isolation roadmap (priority order, replacing any historical WASM language)

1. **Broker scope model.** There is no per-plugin capability enum. Egress,
   credential, and filesystem mediation is the host-side broker's
   responsibility, keyed by the workflow-config credential-scope hash per
   [ADR-0025](../../docs/adr/0025-sandbox-broker-rpc-surface.md)
   (D4 + §2 / §3 / §6). The broker module is slice 1d work, not yet built.
2. **`plugin.toml` signing verification** — canon §7.1; tooling (`cargo-nebula` or equivalent)
   verifies signatures before the host trusts a plugin's `plugin.toml`.
3. **`os_sandbox` per-platform backends** — seccomp-bpf + landlock (Linux), `sandbox_init`
   (macOS), `AppContainer` / job objects (Windows). Ship per-platform as they stabilize.
4. **`ProcessSandbox` parallelism** — sequential dispatch (ADR 0006 slice 1c) is the current
   §12.6 reality. Bounded parallel dispatch per plugin (with a fair scheduler across plugins)
   is the §4.1 throughput win for real workloads.
5. **Integration tests for cancel path and protocol envelope** — canon §13 step 5 must be
   green end-to-end against `ProcessSandbox`, not only against `InProcessSandbox`.

### Discovery TODO (partially closed by slice B)

Slice B of the plugin load-path stabilization closed the `plugin.toml`
parsing gap: discovery now reads `[nebula].sdk` + `[plugin].id` before
spawning the binary, enforces the SDK semver constraint, and honors the
optional id override. The broker scope model (item 1 of the roadmap
above) is the remaining piece, tracked under ADR-0025 slice 1d.

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
