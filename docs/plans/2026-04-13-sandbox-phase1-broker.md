# Phase 1 — Broker protocol & plugin SDK

**Parent roadmap:** [2026-04-13-sandbox-roadmap.md](2026-04-13-sandbox-roadmap.md)
**Status:** spec
**Estimated effort:** ~3-4 weeks
**Blocks on:** Phase 0
**Blocks:** Phase 2, 3, 4

## Goal

Replace the one-shot `PluginRequest/Response` protocol with a **duplex, long-lived JSON-RPC broker** so that plugins can call back into the parent for network, filesystem, credentials, resources, logging, and metrics — all policy-checked against `PluginCapabilities` on every call. This is the phase where `nebula-sandbox` stops being a name-only placeholder and becomes the real chokepoint for untrusted code.

At the end of Phase 1, a community plugin can:
- Receive structured input from the host.
- Ask the host to perform an HTTPS call with an allowlisted credential; the plugin never sees the token.
- Emit logs and metrics that land in the host's observability stack.
- Be cancelled cooperatively.
- Be killed unconditionally on timeout, OOM (rlimit), or capability violation.

## Non-goals

- seccomp / cgroups v2 / namespaces / adversarial test suite — all Phase 2.
- macOS / Windows enforcement — Phase 3.
- Plugin signing, manifest distribution, grant UI — Phase 4.
- Stateful action broker semantics beyond what's needed to run them.

## The protocol

### Transport — go-plugin style

Decision validated against `.project/context/research/sandbox-prior-art.md`. HashiCorp go-plugin (Terraform/Vault/Nomad, ~10 years in production) has already solved this problem; we adopt their wire pattern wholesale.

**Handshake:** plugin writes exactly one line to stdout at startup:

```
NEBULA-PROTO-1|PLUGIN-VER-N|unix|/tmp/nebula-plugin-<uuid>.sock|grpc
```

Five pipe-separated fields: core protocol version, plugin protocol version, transport (`unix` or `tcp`), address (UDS path or `127.0.0.1:port`), wire format (`grpc`). Host parses this line and dials.

**Main channel:** **gRPC** over the negotiated transport.
- Linux / macOS: Unix domain socket at `/tmp/nebula-plugin-<uuid>.sock`.
- Windows: TCP loopback at `127.0.0.1:<port>` from the range `NEBULA_PLUGIN_MIN_PORT..NEBULA_PLUGIN_MAX_PORT` (env vars, default 10000..25000). Windows UDS is supported since 1803 but tooling is uneven; go-plugin uses TCP loopback in production — we match.

**AutoMTLS:** one-shot self-signed certs in both directions. Broker generates a keypair + cert at spawn; passes the public cert to the plugin via an env var (`NEBULA_PLUGIN_CLIENT_CERT`); receives the plugin's public cert over the first gRPC handshake message. Both sides pin. Only the launching broker can speak to the running plugin instance — even if another process on the box can connect to the UDS or loopback port, the mTLS handshake fails.

**Reattach:** the broker persists `{plugin_pid, transport, addr, client_cert, server_cert}` to a state file. On engine restart, the broker reads this file and calls `Client::reattach(info)` — no plugin restart. Plugins keep running across host restarts. Critical for nebula workflow executions that run hours or days.

**stderr:** reserved for logs. Plugin writes structured JSON lines; host task parses each line as `nebula-log` structured log; falls back to verbatim with `plugin:<name>` prefix on parse failure.

**stdin:** unused after the handshake line. Kept open so the plugin can use it as a "parent is alive" signal — closing stdin on plugin side means "graceful shutdown".

### Message shape

All RPC happens over gRPC streaming services. Plugin-SDK exposes them as async Rust traits; plugin authors implement a single trait (`PluginHandler`) and call `nebula_plugin_sdk::run_duplex(handler)` from `main`. The SDK hides all transport, handshake, and mTLS details.

### gRPC services (Phase 1)

Defined in `crates/plugin-protocol/proto/nebula.proto`:

```protobuf
service NebulaPlugin {
  // Plugin-exposed: handle one action invocation (long-running, bidirectional for stateful)
  rpc Execute(stream HostToPlugin) returns (stream PluginToHost);

  // Plugin-exposed: metadata query (replaces the old __metadata__ one-shot)
  rpc Metadata(MetadataRequest) returns (MetadataResponse);

  // Plugin-exposed: graceful shutdown
  rpc Shutdown(ShutdownRequest) returns (ShutdownResponse);
}

service NebulaBroker {
  // Broker-exposed: the plugin calls these
  rpc RpcCall(RpcRequest) returns (RpcResponse);
  rpc LogEmit(stream LogRecord) returns (LogEmitResponse);
  rpc MetricEmit(stream MetricRecord) returns (MetricEmitResponse);
}
```

The bidirectional `Execute` stream carries `HostToPlugin { action_invoke, cancel, rpc_response }` and `PluginToHost { action_result, rpc_call, progress }` as oneof variants — same logical envelope as the earlier stdio JSON-RPC design, now carried by gRPC streams. `RpcCall` from plugin to broker is how `credentials.get`, `network.http_request`, etc. are invoked.

### Message shape
Every message is a tagged envelope:

```json
{ "v": 2, "id": <u64|null>, "kind": "...", "payload": {...} }
```

- `v` — protocol version, `2` for this phase (bumped from current `PROTOCOL_VERSION: 1`).
- `id` — correlation ID for request/response pairs. `null` for one-way messages (logs, metrics, events).
- `kind` — discriminator. See table below.
- `payload` — kind-specific body.

### Message kinds (MVP)

Host → plugin:

| Kind                  | Direction   | Semantics                                               |
|-----------------------|-------------|---------------------------------------------------------|
| `hello`               | H → P       | Protocol version, plugin key, action key, initial input |
| `action_invoke`       | H → P       | For stateful: next iteration                            |
| `rpc_response`        | H → P       | Response to a plugin-initiated RPC                      |
| `cancel`              | H → P       | Cooperative cancel signal                               |
| `shutdown`            | H → P       | Graceful shutdown, plugin should exit                   |

Plugin → host:

| Kind           | Direction | Semantics                                                                                  |
|----------------|-----------|--------------------------------------------------------------------------------------------|
| `hello_ack`    | P → H     | Plugin confirms protocol version match                                                     |
| `action_result`| P → H     | Final (or iteration) result                                                                |
| `rpc_call`     | P → H     | Plugin-initiated capability request (network/fs/credentials/etc.)                          |
| `log`          | P → H     | Structured log event                                                                       |
| `metric`       | P → H     | Metric sample                                                                              |
| `event`        | P → H     | EventBus event (requires `EventEmit` capability)                                           |
| `progress`     | P → H     | Progress heartbeat (resets broker's idle timeout)                                          |

### RPC verbs (initial set)

See `2026-04-13-sandbox-roadmap.md` §5 for the full target surface. **All verbs are default-allow with audit log** — see roadmap §D4 for why. Phase 1 ships:

- `log.emit` → `ActionContext::logger`
- `time.now`, `rand.bytes`, `cancel.check` → host primitives
- `credentials.get { slot }` → `ActionContext::credentials`. Slot name comes from action metadata (derive-macro). **Does not return the secret**; returns an opaque handle `CredentialRef` that can be passed to `network.http_request` as `auth` for host-side injection.
- `network.http_request { method, url, headers, body, auth: Option<CredentialRef>, timeout_ms }` → `{ status, headers, body }`. Sanity checks: domain resolves, private-IP blocklist, size cap (default 10 MiB), per-call timeout, bytes metered, audit-logged. **No manifest-declared scope**.
- `env.get { key }` → returns value. Audit-logged. Operator can set engine-wide deny-list (not plugin-declared).
- `metrics.emit { name, value, labels }` → plugin-namespaced metric.

`fs.read` / `fs.write`, `net.tcp_connect`, `event.emit`, `resource.acquire`, device verbs (`media.camera.*`, `media.microphone.*`, `clipboard.*`, `global-shortcut.*`, `geolocation.*`) land in Phase 2/5.

### Error semantics
- Every `rpc_call` gets exactly one `rpc_response` (matched by `id`).
- On sanity-check failure the response is `{ "error": { "code": "SANITY_CHECK_FAILED", "reason": "private_ip_blocked"|"size_cap_exceeded"|"timeout"|"dns_failure", "details": "..." } }`.
- On slot-not-bound (`credentials.get` / `resource.acquire` with an unbound slot) the response is `{ "error": { "code": "SLOT_NOT_BOUND", "slot": "..." } }`. This is a workflow-config problem, not a security violation — the plugin can surface it to the user as "please configure credential X in your workflow node".
- Protocol errors (unknown message, invalid framing, version mismatch) cause the host to send `shutdown` and then SIGKILL after a 500 ms grace window. Observability event: `SandboxPluginKilled { reason: protocol_error }`.

## Architecture components

### 1. `nebula-plugin-protocol` v2
- Keep the crate, bump `PROTOCOL_VERSION` to `2`.
- **Delete** the one-shot `run()` function and any stdio JSON framing.
- **Add** protobuf definitions under `proto/nebula.proto`; generate Rust types via `prost` at build time. Services `NebulaPlugin` and `NebulaBroker` as defined in the Protocol section above.
- New public types (prost-generated): `HostToPlugin`, `PluginToHost`, `RpcRequest`, `RpcResponse`, `LogRecord`, `MetricRecord`, `CredentialRef`, `ResourceRef`, `MetadataRequest`, `MetadataResponse`, `ShutdownRequest`.
- `nebula-plugin-sdk` (thin façade over protocol): `PluginHandler` trait (`async fn execute(&self, ctx: &PluginCtx, input: Value) -> PluginResult`), `PluginCtx` (exposes `ctx.network().http(...).await`, `ctx.credentials().get("bot_token").await`, `ctx.log().info("...")`, etc.), `run_duplex(handler)` entry point that prints the handshake line and starts the gRPC server with AutoMTLS.
- Plugin authors never see protobuf or tonic types directly — the SDK wraps everything.
- Backwards compatibility: `v1` (one-shot stdio JSON) is **dropped**. No plugins in the wild yet.

Dependencies: `tonic`, `prost`, `rustls`, `rcgen` (AutoMTLS cert generation), `tokio`, `tower`.

### 2. `nebula-plugin-sdk` (new crate, optional)
- Thin wrapper around `nebula-plugin-protocol` that gives plugin authors an ergonomic API: macros, builder patterns, typed RPC helpers, panic→`ActionError` conversion.
- Depends only on `nebula-plugin-protocol` + `serde` + `tokio`. No `nebula-action` (plugins must not accidentally import host types).
- Alternative: roll everything into `nebula-plugin-protocol` and skip the SDK crate. Decide during implementation based on whether the ergonomic layer stays small.

### 3. Host-side `Broker`
- New module `crates/sandbox/src/broker/`. Submodules: `mod.rs`, `network.rs`, `credentials.rs`, `envelope.rs`, `audit.rs`.
- `Broker` owns:
  - `Arc<dyn CredentialAccessor>` (from `SandboxedContext`)
  - `Arc<dyn ResourceAccessor>` (from `SandboxedContext`)
  - `Arc<dyn ActionLogger>` (from `SandboxedContext`)
  - `MetricsRegistry` handle
  - `EventBus` handle for audit events
  - A shared `reqwest::Client` with a custom DNS resolver (anti-SSRF private-IP blocklist)
  - `CancellationToken`
  - Per-call budgets (timeout, byte cap, duration) — configurable per invocation from workflow/engine config
- Public entry point: `async fn run(self, gRPC_stream, invocation: Invocation) -> Result<ActionResult<Value>, ActionError>`.
  - Drives the bidirectional stream loop.
  - Dispatches each `rpc_call` to the appropriate handler method.
  - Runs sanity checks on every call (private-IP blocklist, size cap, timeout).
  - Emits audit events to `EventBus` and metrics to `MetricsRegistry` for every call.
  - Returns when `action_result` arrives, the child exits, the timeout expires, or the cancellation token fires.

**No policy engine** — the broker does not check manifest-declared scope. It only enforces always-on sanity checks and surfaces everything to the audit log. See roadmap §D4.

### 4. `ProcessSandbox` v2 + `PluginSupervisor`
- Keeps `env_clear`, `kill_on_drop`, landlock/rlimits pre_exec, stderr sanitization.
- **Lifecycle shift**: from "spawn per invocation" to "long-lived per `(plugin_key, credential_scope)`". A new `PluginSupervisor` (`crates/sandbox/src/supervisor.rs`) owns the set of running plugin processes keyed by `(ActionKey, CredentialScopeHash)`. `execute(context, metadata, input)` routes through the supervisor: `supervisor.acquire_or_spawn(key, scope)` → gRPC client handle → `client.execute(...)` → result.
- **Crash recovery**: on plugin crash, supervisor relaunches and retries the in-flight call once (fatal on second failure).
- **Reattach support**: supervisor persists `{pid, transport, addr, server_cert, client_cert}` for each running plugin to a state file under `~/.local/state/nebula/plugins/` (or platform equivalent). On engine start, supervisor reads this file and reattaches instead of respawning. Lets the engine restart without killing in-flight workflows — critical for long-running nebula executions.
- **Spawn path**:
  1. Build argv, env (clear by default; pass `NEBULA_PLUGIN_CLIENT_CERT`, `NEBULA_PLUGIN_MAGIC_COOKIE`, `NEBULA_PROTOCOL=1`, `NEBULA_PLUGIN_TRANSPORT`, `NEBULA_PLUGIN_ADDR`).
  2. `tokio::process::Command::spawn` with stdin/stdout/stderr piped + `kill_on_drop(true)`.
  3. (macOS — Phase 3) Apply `responsibility_spawnattrs_setdisclaim(1)` via `pre_exec`. Phase 1 leaves a TODO marker.
  4. (Linux — Phase 0 carry-over) Apply landlock + rlimits via existing `pre_exec`.
  5. Read handshake line from plugin stdout with 500 ms timeout. Parse, dial gRPC, complete mTLS.
  6. Plugin is now registered in supervisor; gRPC client handle returned to caller.
- **Per-call shape**: `execute(context, metadata, input)` acquires supervisor entry, opens a bidirectional `Execute` gRPC stream, sends `HostToPlugin::action_invoke`, relays `PluginToHost::rpc_call` through the broker (which runs always-on sanity checks: anti-SSRF, size cap, timeout, audit log — no manifest-declared scope), relays `rpc_response` back, awaits `action_result`. Timeouts and byte caps enforced per call.
- **Credential-scope boundary**: if a second invocation arrives for the same `ActionKey` with a different credential binding, supervisor spawns a **separate** process. Plugins never see credentials from another scope.

### 5. `InProcessSandbox` v2
- Stops being a pass-through. Becomes a runner that wires the broker into a **same-process** plugin handler — useful for first-party actions that want the same API surface without the spawn cost, and useful for deterministic unit testing of the broker itself without touching the OS.
- Executes via an in-memory channel pair instead of stdio.

### 6. `ActionRuntime` changes
- `execute_stateful` stops fail-closing for non-`None` isolation. The broker's long-lived loop naturally supports stateful iteration — the runtime just pumps `ActionInvoke` messages and reads `ActionResult{iteration}` envelopes.
- Retain the `MAX_ITERATIONS = 10_000` cap.
- Cancellation propagates: if the runtime's `CancellationToken` fires, the broker sends `cancel` immediately and schedules a hard kill after the configured grace window.

### 7. Network broker implementation
- Single shared `reqwest::Client` per broker with a custom DNS resolver that **rejects any response where any resolved IP falls in private ranges**: `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`, `127.0.0.0/8` (except explicit `localhost` opt-in), `::1`, `fe80::/10`, `fd00::/8`. Anti-SSRF is always on; plugins cannot reach cloud-metadata endpoints or internal services.
- Host-side injection of auth headers from `CredentialRef` handles. The plugin can never see the raw secret — the broker resolves the ref, attaches the header, drops the secret from memory before the response is returned.
- Response size cap (configurable, default 10 MiB). Status + headers + body returned as bytes.
- Every request emits `sandbox_rpc_calls_total{plugin, verb="network.http_request", outcome}`, `sandbox_rpc_bytes_total{plugin, verb, direction}`, `sandbox_rpc_duration_seconds{plugin, verb}`.
- Audit event on `EventBus`: `SandboxNetworkCall { plugin, method, host, port, status, bytes_up, bytes_down, duration_ms }` — operator reviews post-hoc.
- Metered: `sandbox_rpc_bytes_total{verb="network.http_request", direction=up|down}`.
- **Private IP blocklist by default** (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, ::1, fe80::/10, fd00::/8). Opt-in to reach private addresses via an explicit capability (`NetworkAllPrivate`) — not added in Phase 1.

### 8. Credential / resource broker implementation

**Not a sandbox-policy gate.** `credentials.get` and `resource.acquire` are engine-provided verbs — the broker proxies directly to `ActionContext::{credentials, resources}`. The policy of "which credentials this plugin node can see" lives in workflow-node configuration (bound at engine level), not in `plugin.toml`.

- `credentials.get { slot }` — `slot` is a string from action metadata, declared in `#[derive(Action)]` annotations and delivered via the `__metadata__` RPC at register time. Broker calls `ctx.credentials.get(slot)` on the current invocation's `ActionContext`.
- If the slot is not bound at the engine level → `SLOT_NOT_BOUND` error (workflow-config problem, not a capability violation).
- If bound, broker creates a `CredentialRef` — a short-lived nonce in a per-invocation map pointing at the real credential handle. Returns the ref to the plugin.
- **The raw credential value never crosses the stdio boundary.** The plugin uses `CredentialRef` only as an indirect handle, passing it into `network.http_request { auth: CredentialRef }`, where the broker resolves it host-side and injects the header.
- `CredentialRef` is valid only inside the current invocation. Cleared on `ActionResult`, cancel, or panic.
- **Slot-type mismatch** is a register-time failure. If the action declares `slot "bot_token": TelegramBotToken` but the engine's credential-type registry has no `TelegramBotToken`, the plugin never registers. No runtime discovery.

The same code path handles `resource.acquire { slot }` — engine-provided, slot-driven, wraps the real resource handle in a `ResourceRef`, cleared at invocation end.

## Work breakdown

1. **Protocol v2 spec** — finalize the envelope schema, freeze `PROTOCOL_VERSION = 2`. One day.
2. **`nebula-plugin-protocol` rewrite** — implement `Envelope`, `Kind`, `run_duplex`, `PluginCtx`. Unit tests for serde round-trips and framing. 3-4 days.
3. **`CapabilityBroker` skeleton** — envelope loop, dispatch table, cancellation integration, deny-everything default. 2-3 days.
4. **Network broker** — reqwest client, domain allowlist, private-IP blocklist, credential ref injection. 2-3 days.
5. **Credential broker** — ref table, host-side lookup, enforcement. 1-2 days.
6. **`ProcessSandbox` v2 rewrite** — swap one-shot `call` for broker-driven `run`. 2 days.
7. **`InProcessSandbox` v2 rewrite** — in-memory channel pair + broker loop, for test + trusted action paths. 2 days.
8. **`ActionRuntime::execute_stateful` unlock** — pump iterations through the broker loop. 2 days.
9. **Examples plugin** — under `examples/sandbox-http-fetch-plugin/`, demonstrates credential use + HTTP request + log emission. 1 day.
10. **Integration tests** — under `crates/sandbox/tests/`:
    - Broker happy path (network + credentials) — 1 day.
    - Capability denial (unknown domain, unknown credential) — 1 day.
    - Protocol mismatch handling (plugin sends `v: 1`) — 1 day.
    - Stateful iteration via broker — 1 day.
    - Cancellation propagation — 1 day.
11. **Metrics wiring** — all RPC verbs. 1 day.
12. **Context docs** — update `sandbox.md`, `runtime.md`, `pitfalls.md`, `decisions.md`. 1 day.

**Total:** ~18-22 working days of focused work. Add buffer for review and rework: budget 3-4 weeks.

## Acceptance criteria

- [ ] `nebula-plugin-protocol` exposes `run_duplex` and the v2 envelope types; v1 `run` removed.
- [ ] `CapabilityBroker` policy-checks `network.http_request` against `check_domain`, with integration test covering allowed, denied-domain, and private-IP rejection.
- [ ] `credentials.get` returns a `CredentialRef`, never a raw secret; integration test asserts raw secret is not observable in any envelope bytes.
- [ ] `ActionRuntime::execute_stateless` and `execute_stateful` both dispatch non-`None` isolation through the broker (the latter's fail-closed branch from Phase 0 is removed).
- [ ] Example plugin at `examples/sandbox-http-fetch-plugin/` performs a real HTTPS call through the broker end-to-end.
- [ ] Metrics for `sandbox_rpc_calls_total`, `sandbox_rpc_bytes_total`, `sandbox_capability_denials_total` increment correctly in tests.
- [ ] `cargo nextest run --workspace` green, `cargo test --doc` green, `cargo clippy --workspace -- -D warnings` clean, `cargo deny check` clean.
- [ ] Context files updated with "Phase 1 landed — broker active".

## Risks

| Risk | Mitigation |
|------|------------|
| Plugin authors find the SDK awkward | Iterate with a couple of real plugin rewrites in Phase 4; broker surface is version-gated so we can extend without breaks |
| Stdio framing bugs cause silent data loss | Newline framing + strict length bounds + property-based roundtrip tests |
| Host-side HTTP client allocates unboundedly on large responses | Enforce per-response byte cap and stream into a `bytes::BytesMut` with a hard limit |
| Credential ref table leaks refs between invocations | Broker owns the table; dropped on `ActionResult` / cancel / panic |
| Cancellation races (broker kills before plugin flushes `action_result`) | Configurable grace window, default 250ms, then SIGKILL (plus `kill_on_drop` as backstop) |
| SSRF via DNS rebinding | Resolve once at the start of the connection, pin the IP, reject if not in public range |
| Stateful loop tails → plugin keeps calling RPC forever | Broker enforces `MAX_RPC_CALLS_PER_INVOCATION` (configurable) and `MAX_RPC_BYTES_PER_INVOCATION`; both default tight |

## What this phase does **not** solve

- Plugin spawning its own threads and CPU-pinning them → needs cgroups (Phase 2).
- Plugin reading files via syscalls → needs seccomp + landlock (Phase 2).
- Plugin forking → needs seccomp (Phase 2).
- Plugin exhausting host memory → needs cgroups + rlimits tightening (Phase 2).
- macOS / Windows plugin authors → broker works, OS jail doesn't (Phase 3).

Phase 1 is the **correctness** phase. Phase 2 is the **safety** phase.
