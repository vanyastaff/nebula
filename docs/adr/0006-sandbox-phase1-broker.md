---
id: 0006
title: sandbox-phase1-broker
status: accepted
date: 2026-04-17
supersedes: []
superseded_by: []
tags: [sandbox, plugin, protocol, security]
related: [crates/sandbox/src/process.rs, crates/plugin-sdk/src/transport.rs, docs/plans/2026-04-13-sandbox-phase1-broker.md]
---

# 0006. Sandbox Phase 1 broker — duplex JSON-RPC over UDS / Named Pipe

## Context

Before Phase 1, `ProcessSandbox` used a one-shot protocol: spawn the plugin
binary, send one `PluginRequest` envelope on stdin, read one `PluginResponse`
from stdout, kill the process. Plugins could not call back into the host for
credentials, network, logging, or metrics. There was no durable transport:
every invocation paid the full spawn cost and plugins could not maintain state
across calls.

The Phase 1 plan (`docs/plans/2026-04-13-sandbox-phase1-broker.md`) identified
two related gaps: (a) plugins need a callback mechanism for host services, and
(b) the one-shot lifecycle prevents long-lived plugin processes that amortise
startup cost. Both require a duplex, long-lived transport.

Slices 1a/1b/1c have landed. Slice 1d (full broker with RPC verbs, `PluginSupervisor`,
reattach) is the remaining proposed portion of this ADR.

## Decision

Replace the one-shot protocol with a **duplex line-delimited JSON envelope
stream** over OS-native transports:

- **Unix / macOS**: Unix domain socket under a per-plugin `0700` directory
  (`/tmp/nebula-plugin-{pid}/sock`); directory mode gates access before socket
  permissions apply.
- **Windows**: Named Pipe under `\\.\pipe\LOCAL\nebula-plugin-{pid}` (session
  namespace, invisible to other logon sessions); first `ConnectNamedPipe` wins.

No TLS, no cert generation. Security boundary is the OS filesystem/object
namespace ACL — the same primitive used by SSH agent, Docker socket, and LSP
servers.

Plugin announces transport via a single handshake line on stdout:
`NEBULA-PROTO-2|unix|<path>` or `NEBULA-PROTO-2|pipe|<name>`.
Host parses with a 3 s timeout, then dials. `ProcessSandbox` caches the
resulting `PluginHandle` (`Mutex<Option<PluginHandle>>`); subsequent calls
reuse it without respawning.

The broker RPC (planned slice 1d): a host-side `Broker` in
`crates/sandbox/src/broker/` will handle inbound `rpc_call` envelopes from the
plugin for `log.emit`, `credentials.get`, `network.http_request`, `time.now`,
`rand.bytes`, `cancel.check`, `env.get`, `metrics.emit`. All verbs are
default-allow with audit log; no manifest-declared scope enforcement until
Phase 2.

## Consequences

Positive (landed):

- Long-lived plugin processes: spawn cost paid once per binary; subsequent
  calls reuse the handle — no fork/exec overhead.
- Bidirectional stream: plugin can send log envelopes asynchronously; host can
  send cooperative `cancel` envelopes.
- Platform-native security boundary with no external crypto dependencies.

Negative:

- Broker RPC verbs (slice 1d) not yet landed; `credentials.get`,
  `network.http_request`, etc. are spec only — plugins cannot call host
  services yet.
- `PluginSupervisor` (multi-process lifecycle, reattach on engine restart,
  credential-scope isolation) also slice 1d / future.
- "Default-allow with audit log" defers per-plugin capability enforcement to
  Phase 2; operators cannot restrict plugin-to-host calls in Phase 1.

Follow-up:

- Slice 1d: broker module, RPC verbs, PluginSupervisor, reattach.
- Phase 2: seccomp / cgroups v2 / namespaces; macOS responsibility disclaim.
- Phase 3: macOS / Windows kernel-level enforcement.
- Phase 4: plugin signing, manifest distribution.

## Alternatives considered

- **gRPC / protobuf over TCP**: rejected before slice 1a. Adds tonic/prost
  dependency, TLS cert generation, and cross-language interop overhead that
  Rust-only plugins do not need. Prior art is LSP/DAP, not go-plugin.
- **Spawn per invocation (keep one-shot)**: rejected. Long-lived processes
  are required for the broker callback model; one-shot can't hold a pending-
  call table for concurrent in-flight RPC responses.
- **Shared memory**: more complex, harder to audit, unnecessary given the
  throughput requirements of workflow automation.

## Seam / verification

Landed seams:
- `crates/sandbox/src/process.rs` — `ProcessSandbox` with `PluginHandle`,
  `spawn_and_dial`, long-lived handle cache.
- `crates/plugin-sdk/src/transport.rs` — `bind_listener`, `PluginListener::accept`,
  `dial` for both UDS and Named Pipe.
- `crates/plugin-sdk/src/protocol.rs` — `HostToPlugin`, `PluginToHost` duplex
  envelope types; `DUPLEX_PROTOCOL_VERSION = 2`.

Proposed seams (slice 1d):
- `crates/sandbox/src/broker/` — broker module (not yet created).
- `crates/sandbox/src/supervisor.rs` — `PluginSupervisor` (not yet created).

Slices landed: 1a (`c6b9d531`), 1b (`f3b6701b`), 1c (`b5723f28`).
