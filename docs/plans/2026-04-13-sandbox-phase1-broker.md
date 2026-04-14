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

Decision validated against `.project/context/research/sandbox-prior-art.md` and the Rust-only plugin constraint (see roadmap §D5). **No gRPC, no protobuf, no TLS** — we're not paying for cross-language interop we don't need. Prior art is LSP / DAP, not go-plugin.

**Handshake:** plugin writes exactly one line to stdout at startup:

```
NEBULA-PROTO-2|unix|/tmp/nebula-plugin-<uuid>.sock
NEBULA-PROTO-2|pipe|\\.\pipe\LOCAL\nebula-plugin-<uuid>
```

Three pipe-separated fields: protocol version, transport kind (`unix` or `pipe`), address (absolute UDS path on Unix, `\\.\pipe\LOCAL\...` on Windows). Host parses this line with a 3 s timeout, then dials the address. The plugin accepts exactly one connection on the listener, then drops the listener; any subsequent `connect()` from another process fails.

**Main channel:** bidirectional line-delimited JSON envelope stream (same shape as slices 1a/1b), carried over:
- **Linux / macOS**: Unix domain socket. Plugin creates a parent directory with mode `0700` owned by the current user, then binds the socket at `<dir>/sock` so the socket's reachability is gated by both directory and socket permissions.
- **Windows**: named pipe under `\\.\pipe\LOCAL\` (session-scoped namespace — invisible to other logon sessions). A `SECURITY_ATTRIBUTES` DACL restricts the pipe to the creating user's SID. The first `ConnectNamedPipe` wins; subsequent attempts fail.

No TLS, no cert generation, no AutoMTLS. The security boundary is the OS-level filesystem / object namespace ACL. Same primitive the SSH agent, systemd, dbus, Docker socket, X11 abstract sockets, and LSP servers all rely on.

**Reattach:** supervisor persists `{pid, transport, address, binary_path}` to a state file under `~/.local/state/nebula/plugins/` (or platform equivalent). On engine restart, supervisor reads the file, checks the PID is alive, dials the address. If the dial succeeds the plugin is reattached — no spawn, no handshake. If the dial fails (stale socket / dead process), the entry is dropped and the next request respawns fresh.

**stderr:** reserved for logs. Plugin writes structured JSON lines there; host task parses each line as structured log, falls back to verbatim with `plugin:<name>` prefix on parse failure. stdout is closed by the plugin after the handshake line (it's never used again).

**stdin:** closed by the host after spawn. The socket is the only live channel.

### Message shape

Every message is a tagged envelope (unchanged from slice 1a):

```json
{"kind": "action_invoke", "id": 42, "action_key": "...", "input": {...}}
```

`kind` is the discriminator; the rest of the fields depend on `kind`. Slice 1a defines both enum sets (`HostToPlugin`, `PluginToHost`) with flat variants — no nested flattening.

### Message kinds

Host → plugin (`HostToPlugin`):

| Kind                  | Semantics                                                 |
|-----------------------|-----------------------------------------------------------|
| `action_invoke`       | Invoke one action with correlation id                     |
| `cancel`              | Cooperative cancel for an in-flight action id             |
| `rpc_response_ok`     | Success response to a plugin-initiated `rpc_call`         |
| `rpc_response_error`  | Error response to a plugin-initiated `rpc_call`           |
| `metadata_request`    | Request plugin metadata                                   |
| `shutdown`            | Graceful shutdown signal                                  |

Plugin → host (`PluginToHost`):

| Kind                  | Semantics                                                 |
|-----------------------|-----------------------------------------------------------|
| `action_result_ok`    | Successful action result with correlation id              |
| `action_result_error` | Failed action result (code, message, retryable)           |
| `rpc_call`            | Plugin-initiated RPC into the host broker                 |
| `log`                 | One-way structured log (level, message, fields)           |
| `metadata_response`   | Reply to `metadata_request`                               |

No `hello` / `hello_ack` — version check happens at compile time (both sides share `DUPLEX_PROTOCOL_VERSION = 2`). No `progress` / `metric` / `event` yet; added in slice 1d alongside broker verbs.

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

### 1. `nebula-plugin-protocol`
- **Status**: done in slices 1a / 1b. `duplex` module ships line-delimited JSON envelope types (`HostToPlugin`, `PluginToHost`, `LogLevel`, `ActionDescriptor`, `DUPLEX_PROTOCOL_VERSION = 2`). Legacy v1 removed.
- **No new dependencies** at any slice of Phase 1. The crate stays on `serde` + `serde_json`. No prost, no tonic.

### 2. `nebula-plugin-sdk`
- **Status**: scaffolding done in slice 1a. `PluginHandler` async trait, `PluginCtx` placeholder, `PluginMeta` / `PluginError`, `run_duplex(handler)` over stdio (slice 1a) → over UDS / Named Pipe (slice 1c).
- Slice 1c extends `run_duplex` to bind a platform-specific transport (UDS on Unix via `tokio::net::UnixListener`, Named Pipe on Windows via `tokio::net::windows::named_pipe::ServerOptions`), print the handshake line, accept one connection, run the event loop over the stream.
- Slice 1d adds broker RPC accessors to `PluginCtx` (`ctx.network().http(...)`, `ctx.credentials().get(...)`, etc.) via outbound `RpcCall` envelopes + pending-call tables.
- **Dependencies**: `nebula-plugin-protocol`, `async-trait`, `serde`, `serde_json`, `tokio` (io-std, io-util, sync, **+ `net` feature in slice 1c**), `thiserror`, `tracing`. Zero transport libraries beyond tokio.

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
- Public entry point: `async fn run(self, stream: PluginStream, invocation: Invocation) -> Result<ActionResult<Value>, ActionError>` where `PluginStream` is an async `AsyncRead + AsyncWrite` over the UDS / Named Pipe connection.
  - Drives the bidirectional stream loop.
  - Dispatches each `rpc_call` to the appropriate handler method.
  - Runs sanity checks on every call (private-IP blocklist, size cap, timeout).
  - Emits audit events to `EventBus` and metrics to `MetricsRegistry` for every call.
  - Returns when `action_result` arrives, the child exits, the timeout expires, or the cancellation token fires.

**No policy engine** — the broker does not check manifest-declared scope. It only enforces always-on sanity checks and surfaces everything to the audit log. See roadmap §D4.

### 4. `ProcessSandbox` + `PluginSupervisor`
- Keeps `env_clear`, `kill_on_drop`, landlock/rlimits pre_exec, stderr sanitization from slice 1b.
- **Lifecycle shift**: from "spawn per invocation" (slice 1b one-shot) to "long-lived per `(binary_path, credential_scope_hash)`". A new `PluginSupervisor` (`crates/sandbox/src/supervisor.rs`) owns the set of running plugin processes keyed by that tuple. `ProcessSandbox::execute(ctx, metadata, input)` becomes a thin wrapper that asks the supervisor for a handle and sends one envelope down it.
- **Crash recovery**: on plugin crash or connection drop, supervisor discards the handle, respawns the plugin on the next request, and retries the in-flight call once (fatal on second failure).
- **Reattach support**: supervisor persists `{pid, transport, address, binary_path}` per running plugin to a state file under `~/.local/state/nebula/plugins/` (or platform equivalent). On engine start, supervisor reads the file, checks each PID is still alive, attempts to dial the address. On success → reattach. On failure → drop the entry, respawn fresh on next request.
- **Spawn path**:
  1. Build argv, env (clear by default; env vars from capabilities only).
  2. `tokio::process::Command::spawn` with stdin closed, stdout piped for handshake, stderr piped for logs, `kill_on_drop(true)`.
  3. (macOS — Phase 3) Apply `responsibility_spawnattrs_setdisclaim(1)` via `pre_exec`. Phase 1 leaves a TODO marker.
  4. (Linux — Phase 0 carry-over) Apply landlock + rlimits via existing `pre_exec`.
  5. Read handshake line from plugin stdout with 3 s timeout. Parse `NEBULA-PROTO-2|unix|<path>` or `NEBULA-PROTO-2|pipe|<name>`.
  6. Dial the UDS / Named Pipe. The plugin has already bound and is `accept()`-ing.
  7. Store `PluginHandle { process, connection, writer_tx, reader_rx, next_id }` in the supervisor map.
- **Per-call shape**: supervisor.acquire_or_spawn(binary_path, scope_hash) → PluginHandle → handle.invoke(action_key, input) → assigns new correlation id, sends `ActionInvoke` on writer task, awaits `ActionResult*` on pending-call map keyed by id, returns.
- **Concurrent dispatch**: multiple invocations against the same plugin handle run concurrently; correlation ids in the envelope disambiguate responses. Plugin-side `run_duplex` also becomes concurrent in slice 1c (spawn task per `ActionInvoke`, shared `Arc<Handler>`, writer channel).
- **Credential-scope boundary**: if a request arrives for the same binary with a different credential hash, supervisor spawns a separate process. Plugins never see credentials from another scope.

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

## Work breakdown (sliced)

Phase 1 is delivered in 5 incremental slices, each a separate commit with its own tests:

- **Slice 1a** ✅ (done) — duplex envelope types + `nebula-plugin-sdk` + stdio `run_duplex` + echo fixture + 8 integration tests. Zero new workspace deps.
- **Slice 1b** ✅ (done) — `ProcessSandbox` migrated to duplex v2. Legacy v1 (`PluginRequest`/`PluginResponse`/`run()`) deleted.
- **Slice 1c** (next, ~4-5 days) — UDS (Unix) / Named Pipe (Windows) transport + `PluginSupervisor` (long-lived per `(binary_path, scope_hash)`) + concurrent multiplexed dispatch + Reattach via state file. Plugin `run_duplex` spawns task per `ActionInvoke` with shared `Arc<Handler>`. `tokio` feature `net` added to `plugin-sdk` and `sandbox`. Zero new workspace deps.
- **Slice 1d** (~5-7 days) — `CapabilityBroker` skeleton + broker verbs: `network.http_request` (reqwest + anti-SSRF blocklist + per-call budgets), `credentials.get { slot }` with `CredentialRef` indirection, `env.get`, `log.emit`, `metrics.emit`, `time.now`, `rand.bytes`, `cancel.check`. `PluginCtx` gets the accessor methods. Pending-call tables on both sides wire `RpcCall` ↔ `RpcResponse*`.
- **Slice 1e** (~2-3 days) — unblock `ActionRuntime::execute_stateful` for non-None isolation through the supervisor. Multi-iteration stateful actions.

**Total remaining after 1a+1b**: ~11-15 working days (slice 1c + 1d + 1e). Tight bound because we avoided the tonic+prost+rustls+rcgen dependency stack; every component is small and testable in isolation.

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
| JSON line-framing bugs cause silent data loss | Serde `single_line_serialization` invariant test in slice 1a; framing enforced by `writeln!` + `read_line` on both sides |
| UDS path collisions on shared tmpdir | UUID-suffixed socket path inside a per-plugin directory created with mode `0700` |
| Windows named pipe DACL bypass | Use `\\.\pipe\LOCAL\` session-scoped namespace as primary; explicit user-SID DACL as defense-in-depth |
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
