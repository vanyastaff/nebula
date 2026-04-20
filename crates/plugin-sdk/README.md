---
name: nebula-plugin-sdk
role: Plugin Author SDK (Duplex Broker Client)
status: partial
last-reviewed: 2026-04-17
canon-invariants: [L1-7.1, L1-12.6]
related: [nebula-sandbox, nebula-plugin]
---

# nebula-plugin-sdk

## Purpose

A community plugin author writes a Rust binary that the engine spawns as a child process.
That binary needs to speak the duplex JSON envelope wire protocol, handle the handshake,
dispatch incoming action invocations, and eventually call back into the host for credentials,
network, or logging via broker RPCs. Without an SDK, every plugin author would re-implement
the framing logic and envelope types — and diverge in incompatible ways. `nebula-plugin-sdk`
is the plugin-author-side SDK: implement `PluginHandler`, call `run_duplex` from `main`, and
the SDK handles the rest. The host side (`nebula-sandbox`) imports the wire envelope types from
this crate to speak the same protocol.

## Role

*Plugin Author SDK (Duplex Broker Client).* The plugin-process counterpart to
`nebula-sandbox`'s `ProcessSandbox`. Plugin authors implement `PluginHandler` and call
`run_duplex`; the SDK owns the handshake, transport binding, line framing, and dispatch loop.
Wire protocol: duplex line-delimited JSON over OS-native transport (UDS on Unix, Named Pipe on
Windows) per ADR 0006.

## Public API

- `PluginHandler` — trait plugin authors implement: `manifest() -> &PluginManifest`,
  `actions() -> &[ActionDescriptor]`, and
  `execute(ctx, action_key, input) -> Result<Value, PluginError>`. The manifest type
  is [`nebula_metadata::PluginManifest`] (Core-layer dep, see §7.1 below); the
  descriptor type lives in the `protocol` submodule.
- `PluginCtx` — execution context passed into `PluginHandler::execute`. Placeholder in
  slice 1c; future slices add broker RPC accessors.
- `PluginError` — typed error crossing the protocol boundary: `fatal` and `retryable`
  constructors.
- `run_duplex` — `main`-callable async entry point: binds transport, emits handshake,
  accepts one host connection, runs the dispatch loop.
- `protocol` submodule — `HostToPlugin`, `PluginToHost` envelope types,
  `DUPLEX_PROTOCOL_VERSION`. Host imports these; plugin authors do not touch them directly.
- `transport` module — `bind_listener`, `PluginListener`, `PluginStream` — stdio handshake
  + UDS / Named Pipe binding.

## Contract

- **[L1-§12.6]** Plugin IPC today is **sequential dispatch over a JSON envelope to a child
  process**. That is the trust model. The SDK must not describe itself as providing attacker-
  grade isolation. Parallelism within a plugin process is `planned` (ADR 0006 slice 1d).

- **[L1-§7.1]** Plugin is the unit of registration. **One Core-layer
  exception to intra-workspace deps:** `nebula-metadata` (for
  `PluginManifest`) and `nebula-schema` (for `ValidSchema` on wire).
  Both are Core-layer crates, not engine-side infrastructure, so the
  plugin-side binary stays free of engine coupling. Any other
  cross-imports must be questioned hard. Wire envelope types live here
  (not in `nebula-plugin`) because plugin authors link against them.

- **[L1-§7.2]** Protocol versioning (`DUPLEX_PROTOCOL_VERSION`): cross-version compatibility
  of the wire envelope is not yet a tested contract. Breaking the envelope requires migration
  guidance per canon §7.2 and `docs/UPGRADE_COMPAT.md`.

## Non-goals

- Not the host-side sandbox — see `nebula-sandbox` (`ProcessSandbox`, `ProcessSandboxHandler`).
- Not the host-side plugin registry or trait — see `nebula-plugin`.
- Not the integration author SDK — see `nebula-sdk` (for writing actions backed by external
  services; this crate is for the plugin binary entry point).
- Not a multi-runtime crate — async runtime is `tokio` only; by design for the initial release.

## Maturity

See `docs/MATURITY.md` row for `nebula-plugin-sdk`.

- API stability: `partial` — `PluginHandler`, `PluginCtx`, `run_duplex`, and the wire
  protocol are in active use (ADR 0006 slices 1a–1c landed); slice 1d adds `PluginCtx` broker
  RPC accessors and `PluginSupervisor`.
- `PluginCtx` is a placeholder (no methods); broker RPC verbs land in slice 1d.
- No capability negotiation in the handshake yet (related to `nebula-sandbox` capability TODO).
- Protocol versioning not yet a tested contract.
- 1 panic site — candidate for typed error.
- 2 unit test markers + 1 integration test — coverage is light; handshake path is the most
  exercised surface.

## Related

- Canon: `docs/PRODUCT_CANON.md` §7.1, §12.6.
- ADR: `docs/adr/0006-sandbox-phase1-broker.md` — duplex JSON-RPC over UDS / Named Pipe;
  slices 1a/1b/1c landed, slice 1d proposed.
- Plugin model: `docs/INTEGRATION_MODEL.md` §7.
- Upgrade compatibility: `docs/UPGRADE_COMPAT.md`.
- Siblings: `nebula-sandbox` (host side / `ProcessSandbox`), `nebula-plugin` (host-side
  registry and trait).
