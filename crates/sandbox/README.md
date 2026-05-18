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

A workflow engine that dispatches to community plugins needs a transport between the engine
host and plugin code that is honest about what it actually provides: child-process execution
for community plugins over a duplex JSON envelope protocol (the trust model canon §12.6 names
explicitly). `nebula-sandbox` is a **transport-only Plugin-Proto leaf**: it owns
`ProcessSandbox` (the duplex envelope transport), the pure credential-scope identity
(`ScopeHash`), and the Linux OS-level hardening primitives. It does **not** own discovery, the
registry adapters, the runner abstraction, or any in-process execution path — those were
relocated (see "Relocation" below). There is **no per-plugin capability/scope model** —
egress, credential, and filesystem mediation is the broker's responsibility (ADR-0025), not
this crate — and **this is not a security boundary against malicious native code**.

## Role

*Process Sandbox Transport (Correctness Boundary).* Provides `ProcessSandbox` — the host side
of the child-process duplex JSON-envelope transport per ADR 0006. Correctness and least
privilege for accidental misuse — not attacker-grade isolation against malicious native code.
Canon §12.6 is the normative statement. A leaf crate with no Business-tier dependency.

## Public API

- `ProcessSandbox` — child-process execution over a duplex line-delimited JSON envelope
  (ADR 0006 Phase 1). Long-lived plugin process; spawn cost paid once (lazily, on the first
  envelope round-trip). Sequential dispatch within a single plugin process today. Transport
  methods return `SandboxError`.
- `scope::{ScopeHash, scope_hash}` — pure credential-scope identity (ADR-0025 §2), computed
  from caller-supplied slot-name strings only. The engine owns the process pool that keys on
  it; this crate never sees a workflow node.
- `os_sandbox` module — Linux Landlock (fixed system paths, best-effort, fail-closed) plus
  `setrlimit` child caps, applied fork-safely via `PreparedSandbox`. No-op on non-Linux.
- `SandboxError` — typed transport error.

## Relocation

These were **moved out of this crate** (host-registry population and dispatch belong with
their owners; the leaf stays transport-only):

- Plugin **discovery** path, the `RemoteAction` / `ProcessSandboxHandler` registry adapters,
  and the `SandboxError` → `ActionError` mapping (`sandbox_bridge`) → **`nebula-plugin`**.
- `SandboxRunner`, `SandboxedContext`, and `InProcessSandbox` (the in-process trusted-dispatch
  path and the runner abstraction the dispatcher owns) → **`nebula-engine`**.

Migration path: code that constructed an `InProcessSandbox` or named `SandboxRunner` /
`SandboxedContext` now imports them from `nebula-engine`; code that discovered plugins or
referenced `ProcessSandboxHandler` / `RemoteAction` now imports them from `nebula-plugin`.
`ProcessSandbox`, `SandboxError`, and `scope::*` continue to come from `nebula-sandbox`.

## Contract

- **[L1-§12.6]** This crate is **transport only**: child-process execution provides
  **correctness and cooperative cancellation**, not a security boundary against malicious
  native code. The in-process trusted-dispatch path (`InProcessSandbox`) lives in
  `nebula-engine`, not here.

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

- **[L1-§7.1]** Plugin is the unit of registration. `ProcessSandbox` hosts the duplex
  transport; `nebula-plugin-sdk` is the plugin-author side. Wire protocol types live in the
  SDK because plugin authors link against them; the sandbox imports them back to speak the
  same protocol. Host-side registration of discovered actions lives in `nebula-plugin`, not
  here.

## Non-goals

- Not an attacker-grade isolation boundary against malicious native code.
- Not a WASM / WASI runtime — see §12.6 rationale.
- Not the action dispatcher / runner abstraction — `SandboxRunner` lives in `nebula-engine`.
- Not the in-process execution path — `InProcessSandbox` lives in `nebula-engine`.
- Not the plugin discovery path or registry adapters — see `nebula-plugin`.
- Not the plugin trait / registry — see `nebula-plugin`.
- Not the plugin-author SDK — see `nebula-plugin-sdk`.

## Maturity

See `docs/MATURITY.md` row for `nebula-sandbox`.

- API stability: `partial` — `ProcessSandbox` is in active use; the broker (egress /
  credential / scope mediation) is not yet built (see Appendix).
- No per-plugin capability/scope surface (removed; the broker owns scope per ADR-0025).
- `os_sandbox` is Linux-only: Landlock fixed system paths + `setrlimit`, best-effort and
  fail-closed, applied fork-safely. No macOS/Windows OS confinement — `is_available()`
  reports this honestly.
- ADR 0006 slice 1d (broker RPC, `PluginSupervisor`, reattach) is `proposed` / not yet landed.
- 1 integration test (`discovery_schema_roundtrip`, `#[ignore]`-gated — requires pre-built
  fixture); cancel path and protocol envelope covered only by unit tests.

## Related

- Canon: `docs/PRODUCT_CANON.md` §4.5, §7.1, §12.6.
- ADR: ADR-0006 (historical — `docs/adr/HISTORICAL.md`) — duplex JSON-RPC over UDS / Named Pipe.
- Plugin model: `docs/INTEGRATION_MODEL.md` §7.
- Glossary: `docs/GLOSSARY.md` §4 (sandbox / resource).
- Siblings: `nebula-plugin-sdk` (plugin-author side / wire protocol), `nebula-plugin`
  (host-side registry + discovery + `ProcessSandboxHandler`/`RemoteAction` + the
  `SandboxError` → `ActionError` mapping), `nebula-engine` (owns `SandboxRunner`,
  `SandboxedContext`, `InProcessSandbox`, and dispatches through this transport).

## Appendix

### Real isolation roadmap (priority order, replacing any historical WASM language)

1. **Broker scope model.** There is no per-plugin capability enum. Egress,
   credential, and filesystem mediation is the host-side broker's
   responsibility, keyed by the workflow-config credential-scope hash per
   [ADR-0025](../../docs/adr/HISTORICAL.md)
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

ADR 0006 (historical — `docs/adr/HISTORICAL.md`) covers the Phase 1 duplex broker:

- Slices 1a (`c6b9d531`), 1b (`f3b6701b`), 1c (`b5723f28`) — **landed**: long-lived plugin
  process, duplex line-delimited JSON envelope over UDS / Named Pipe, cooperative cancel.
- Slice 1d — **proposed**: broker module (`crates/sandbox/src/broker/`), `PluginSupervisor`,
  RPC verbs (`credentials.get`, `network.http_request`, etc.), reattach on engine restart.

Until slice 1d lands, plugins cannot call back into the host for credentials, network, or
logging via the broker RPC. The `PluginCtx` in `nebula-plugin-sdk` is a placeholder.

### Architecture notes

- `SandboxRunner`, `InProcessSandbox`, and `SandboxedContext` are **not** owned here — they
  live in `nebula-engine` (the dispatcher that owns the runner abstraction). This crate
  exports only the transport (`ProcessSandbox`), the typed `SandboxError`, the credential-
  scope identity (`scope::{ScopeHash, scope_hash}`), and the `os_sandbox` hardening
  primitives.
- The `SandboxError` → `ActionError` mapping, plugin discovery, and the
  `ProcessSandboxHandler` / `RemoteAction` registry adapters live in `nebula-plugin` (the
  host-registry crate), keeping this leaf free of any Business-tier dependency.
- Dependency on `nebula-plugin-sdk` (wire protocol types) is correct: this crate is the
  **host** of the duplex transport; the SDK is the **plugin** side.
