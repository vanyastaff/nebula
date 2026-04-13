# Sandbox & Community Plugin Execution — Roadmap

**Status:** proposal
**Scope:** `nebula-sandbox`, `nebula-plugin-protocol`, `nebula-runtime`, touches on `nebula-action`, `nebula-plugin`, `nebula-credential`, `nebula-resource`, `nebula-engine`, `apps/desktop`
**Authors:** Claude (with architecture input from `.project/context/crates/{runtime,sandbox}.md`)

## 1. Why this document exists

Nebula's goal is to run **community / third-party actions** — code we don't control — inside the workflow engine. Today:

- `nebula-runtime` **fail-closes** on any `IsolationLevel != None` (`runtime.rs:245-250, 266-270`) because there is nothing to dispatch to.
- `nebula-sandbox::InProcessSandbox` is effectively a pass-through: only checks cancellation (`in_process.rs:31-47`).
- `nebula-sandbox::ProcessSandbox` is the most real thing we have — `env_clear` + landlock + rlimits + stdout cap + `kill_on_drop` — but it is **not wired into `ActionRuntime`** and its protocol is **one-shot**: the plugin process reads one `PluginRequest{action_key,input}`, writes one `PluginResponse`, exits. There is **no way** to pass `ActionContext` (credentials, resources, logger) to the plugin, and **no callback channel** from plugin back to parent.
- `PluginCapabilities` is a rich, well-tested model (`capabilities.rs:13-173`) but it is **advisory**: no runtime enforcement except via landlock/rlimits at spawn time.

Net effect: **community plugins cannot actually do anything useful today** (they can't receive secrets), and **even if they could, we have no enforcement story** beyond Linux landlock.

## 2. Architectural direction

We commit to the following decisions up front. All validated against `.project/context/research/sandbox-prior-art.md`:

### D1. Broker over raw syscalls
Plugin processes **must not** do their own network, FS, credentials, or resource I/O. Everything capability-relevant goes through a **duplex JSON-RPC broker** between parent and plugin. OS jails (landlock, seccomp, cgroups, AppContainer, sandbox-exec) exist as **defense-in-depth**, not as the primary enforcement.

**Rationale:**
- Cross-platform. OS jails vary wildly; broker is identical on Linux/macOS/Windows.
- Enforceable. Every network request / file read / credential fetch is a message we can policy-check, log, meter, and cancel.
- Secret-safe. Plugin never sees raw credentials — it asks the broker "make an HTTPS POST to `api.slack.com` with the `slack_bot_token` credential" and the broker injects the header.
- Auditable. Broker becomes a single chokepoint for "what did this plugin do" — metrics, event bus, violation events.
- Compatible with future WASI. The broker interface is the same whether the plugin runs as a child process, a WASI module, or (one day) an in-process WASM module.

**Cost:** breaks the current plugin protocol. Plugins must use a new SDK (`nebula-plugin-sdk`) that wraps broker RPC. We own both sides, so that's acceptable — nobody ships community plugins yet.

### D2. Process isolation first, WASI later
Keep the existing "plugin = OS child process" decision from `sandbox.md:10`. Plugins get to use `tokio`, `reqwest`, `teloxide`, anything. WASI stays as a separate, non-blocking experimental track.

### D3. Linux is Tier 1, macOS/Windows are Tier 2
We ship **production-grade** guarantees only on Linux (landlock + seccomp-bpf + cgroups v2 + user/network namespaces). macOS and Windows get the broker plus best-effort OS jails (`sandbox-exec`, AppContainer + Job Object) and a clearly-documented "defense in depth only, not a hard boundary" label. The desktop app (`apps/desktop`) is Tier 2 by definition and must refuse to run un-audited community plugins unless the user explicitly grants it.

### D4. No permission manifest (deferred)
We ship **no `[permissions]` declaration** in `plugin.toml` for Phase 1–4. The speculative "Cargo-style capability scope" design (fully researched, see `.project/context/research/sandbox-permission-formats.md`) is deferred until real community plugins and operator feedback crystallize the requirements. What we ship instead is strictly more practical and already dramatically better than n8n's in-process model:

1. **Process isolation** — plugin cannot touch host FS, spawn children, or speak raw network by construction.
2. **Broker gRPC as sole exit** — every outside-world call goes through host-mediated RPC.
3. **Anti-SSRF + private-IP blocklist** — broker refuses on resolve, always on.
4. **Audit log** — every broker RPC emitted to EventBus + metrics; operator sees what the plugin did post-hoc.
5. **Signed manifest** — supply-chain integrity at install time (Phase 4).
6. **OS jail** — seccomp + landlock + cgroups on Linux, sandbox-exec + disclaim on macOS, AppContainer + Job Object + WFP on Windows. Defense in depth even against broker bugs.

The plugin manifest carries **identity and signing only**. Trust is all-or-nothing: you install a signed plugin or you don't. Scope enforcement is sandbox-wide, not per-plugin-declared. Revisit only when a real operator says "plugin X must only talk to `*.company.internal`" and nothing in the audit-log-based workflow is sufficient.

### D5. Transport: gRPC over UDS/TCP-loopback with go-plugin-style handshake
Stdio is for the **handshake line only**. Plugin writes one line to stdout: `NEBULA-PROTO-1 | PLUGIN-VER-N | unix|tcp | addr | grpc`. Host parses, dials. gRPC with **AutoMTLS** on both sides (one-shot self-signed certs, only the launching host can speak to the running plugin). UDS on Linux/macOS, TCP loopback on Windows (Windows UDS is supported since 1803 but tooling is uneven; go-plugin uses TCP loopback in production). Plugin-SDK hides all this behind `run_duplex(handler)` — plugin authors see a simple async API. 10 years of Terraform/Vault production validation.

### D6. Lifecycle: long-lived per `(plugin_key, credential_scope)` with Reattach
**Not per-invocation.** For workflow executions running hours to days, spawn-per-call is prohibitive. Plugin process is long-lived and scoped by `(plugin_key, credential_scope)` tuple — same plugin with different credential bindings runs as different processes (prevents credential leakage). **Reattach** means the engine can restart without killing in-flight workflows. This is the Nomad shape of go-plugin, not the Terraform shape.

### D7. OS permission granted once; sandbox enforces the rest
OS permission (macOS TCC, Windows Privacy, Linux portal) is granted **once to nebula-desktop** as an application via the usual OS consent prompts. There is **no second layer of plugin-declared permission** — the broker is the sole runtime gatekeeper, default-allow with audit log. **`responsibility_spawnattrs_setdisclaim(1)` on macOS** ensures plugin children cannot inherit host TCC grants — a plugin that escapes its sandbox still cannot access camera/mic/screen directly, because the OS refuses the disclaimed child. The plugin's only path to TCC-gated devices is via broker RPC, and the broker runs in the host process (which holds the OS grant). Same structural guarantee via AppContainer on Windows (children don't inherit user-level privacy grants) and empty netns on Linux (plugin has no network stack at all, must go through broker UDS).

## 3. Phases

```
Phase 0  — Wire existing ProcessSandbox into runtime, stop fail-closing
  │        Tiny, unblocks any further work. ~1 week.
  ▼
Phase 1  — Broker protocol + nebula-plugin-sdk
  │        The real architectural shift. ~3-4 weeks.
  ▼
Phase 2  — Linux hardening (seccomp, cgroups, namespaces, pool, adversarial tests)
  │        Turns "it runs" into "it's safe to run untrusted code". ~3-4 weeks.
  ▼
Phase 3  — Cross-platform (macOS sandbox-exec, Windows AppContainer)
  │        Broker is platform-independent; OS jails are the delta. ~2-3 weeks.
  ▼
Phase 4  — Community delivery (signed manifests, grant flow, registry, desktop UI)
  │        Shipping story. ~4-6 weeks.
  ▼
Phase 5  — Input devices (camera/mic/clipboard/shortcuts/keyboard, Tauri delegation)
           Desktop device story. Data plane over shared memory. ~4-5 weeks.
```

Each phase has its own spec under `docs/plans/2026-04-13-sandbox-phaseN-*.md`.

## 4. What we keep / change / delete

### Keep
- `SandboxRunner` trait shape (`runner.rs:42-50`). Minimal and object-safe.
- `ProcessSandbox::call`'s spawn hardening: `env_clear`, `kill_on_drop`, stdout size cap, stderr sanitization, landlock/rlimits `pre_exec`. All valuable.
- `os_sandbox::apply_sandbox` (landlock + rlimits) — extend, don't replace.
- `discovery.rs` plugin discovery — extend with signed manifest loading (Phase 4).

### Change
- **`nebula-plugin-protocol`**: one-shot `PluginRequest/Response` → **gRPC over UDS/TCP-loopback with go-plugin-style handshake line and AutoMTLS** (see D5). Versioned (`PROTOCOL_VERSION: 1` → `2`). Protobuf services `NebulaPlugin` and `NebulaBroker`. Bidirectional `Execute` stream. Plugin SDK exposes a thin `run_duplex(handler)` façade — plugin authors never see tonic/prost.
- **Plugin manifest**: minimal 9-line TOML with `[plugin]` (key, version, author, description, sdk-version, binary-sha256) and `[signing]` (algorithm, public-key, signature). **No `[permissions]`, no `[runtime]`, no `[actions]`** — see D4. Resource limits come from `nebula-runtime` / workflow-config. Actions come from derive-macros via `__metadata__`.
- **`InProcessSandbox`**: becomes a real in-process runner wiring the broker into a same-process plugin handler — useful for first-party actions that want the same API without spawn cost, and for deterministic broker testing. Drop the current pure pass-through behaviour; keep the name.
- **`ActionRuntime::execute_stateless` / `execute_stateful`**: stop fail-closing on non-`None` isolation. Dispatch through `self.sandbox` (`runtime.rs:245-250, 266-270`).
- **Lifecycle**: from per-invocation spawn to **long-lived per `(plugin_key, credential_scope)` via `PluginSupervisor`** with Reattach (see D6).
- **`SandboxedContext`**: keeps the `ActionContext` wrapping role; loses the "holds PluginCapabilities" responsibility because there's no PluginCapabilities anymore. Just holds credentials accessor, resource accessor, logger, cancellation.
- **`discover_plugin`** (`discovery.rs:87`): currently hardcodes `PluginCapabilities::none()`. Replaced by `verify_and_load_manifest(path)` in Phase 4, which reads the signed `plugin.toml`, verifies ed25519 signature over `[plugin]`, checks `sdk-version` against supported range, verifies `binary-sha256` against the binary.
- **Examples**: any runnable plugin examples live under the root `examples/` workspace member, not per-crate (per project convention).

### Delete
- The current one-shot plugin main loop (`plugin-protocol/src/lib.rs:228-248`). Replaced by gRPC-based `run_duplex(handler)`.
- The `#[ignore = "Sandboxed dispatch is Phase 7.6 — currently bypassed"]` test at `runtime.rs:648-679` — un-ignore as Phase 0 acceptance criterion.
- Any `TODO: load from config` comments (`discovery.rs:87`) once Phase 4 manifest loading lands.
- The flat `Capability` enum in `nebula-sandbox::capabilities` — no replacement, permission model is deferred (see D4). The path-normalization logic at `capabilities.rs:194-220` stays as a host-side utility but is no longer tied to any user-facing scope concept.

## 5. Broker RPC surface (target)

All plugin↔host interaction goes through broker gRPC. **There is no scope declaration** — every verb is default-allow with audit log, subject to always-on sanity checks (anti-SSRF blocklist, per-call timeout, byte caps). See D4 for the full rationale.

| Verb | Source | Sanity checks | Phase |
|---|---|---|---|
| `network.http_request` | `reqwest::Client` in broker | domain resolves, private-IP blocklist, byte cap, timeout, audit log | 1 |
| `net.tcp_connect` | `tokio::net::TcpStream` in broker, bridged to `BrokerStream` | private-IP blocklist, TLS validation, audit log | 2 |
| `fs.read` / `fs.write` | host FS within plugin scratch dir; explicit absolute paths audited | path normalization, `/etc/shadow`-style blocklist, size cap, audit log | 2 |
| `env.get` | `std::env::var` in broker | audit log; operator can set engine-wide deny-list | 1 |
| `event.emit` | `EventBus` | per-plugin namespace prefix on event name | 2 |
| `metrics.emit` | `MetricsRegistry` | per-plugin namespace prefix on metric name | 2 |
| `media.camera.open` | OS camera API (Tauri-delegated on desktop, PipeWire on Linux) | must have OS TCC / Windows Privacy / portal grant for nebula-desktop | 5 |
| `media.microphone.open` | same shape as camera | same | 5 |
| `clipboard.read` / `clipboard.write` | OS clipboard API | audit log | 5 |
| `global-shortcut.register` | OS hotkey API | audit log; tray indicator | 5 |
| `geolocation.query` | OS location API | OS grant required | 5 |
| `credentials.get {slot}` | `ActionContext::credentials` → `CredentialRef` | slot must be bound at workflow-config; raw secret never crosses IPC | 1 |
| `resource.acquire {slot}` | `ActionContext::resources` → `ResourceRef` | slot must be bound at workflow-config | 2 |
| `log.emit` | `ActionContext::logger` | — | 1 |
| `time.now`, `rand.bytes`, `cancel.check` | host primitives | — | 1 |

**Invariants:**

- `process.spawn` has **no verb** — a sandboxed plugin spawning its own child processes defeats the model. Anything a plugin needs that requires a helper binary is a *nebula resource* exposed through the broker.
- `credentials.get` / `resource.acquire` use **slot names from action metadata** (generated by `#[derive(Action)]` macros, delivered via `__metadata__` at register time). Slot→value binding happens at workflow-config time. Plugin never sees a credential ID or raw value.
- All RPCs emit `sandbox_rpc_calls_total{plugin, verb, outcome}` and `sandbox_rpc_bytes_total{plugin, verb, direction}` metrics. Operators audit post-hoc via `nebula plugin logs <plugin>`.

## 6. Observability contract

The broker emits, at minimum:

- **Metrics** (per plugin key, per verb):
  - `sandbox_rpc_calls_total{verb, outcome=allowed|denied|error}`
  - `sandbox_rpc_bytes_total{verb, direction=up|down}`
  - `sandbox_rpc_duration_seconds{verb}`
  - `sandbox_capability_denials_total{capability}`
  - `sandbox_plugin_spawn_total{plugin, outcome}`
  - `sandbox_plugin_oom_total{plugin}`
  - `sandbox_plugin_timeout_total{plugin}`

- **Events** on `EventBus`:
  - `SandboxCapabilityDenied { plugin, verb, reason, capability }`
  - `SandboxPluginKilled { plugin, reason: oom|timeout|stderr_limit|protocol_error }`
  - `SandboxPluginSpawnFailed { plugin, error }`

- **Structured logs**: every denied verb logs at `warn` with plugin key, verb, and reason. Plugin stderr is already sanitized and logged (`process.rs:256-261`) — keep.

## 7. Out of scope (for now)

- In-process WASM sandbox (cranelift/wasmtime). Tracked as a separate spike in Phase 3 — no dependency on main roadmap.
- Network metering at the packet level (we meter at the RPC byte level, which is enough for policy but not for QoS).
- GPU / camera / microphone / notifications — these are broker verbs exposed in Phase 5, gated only by the host app's OS-level TCC/Privacy grant. Phase 4 deals with plugin install/uninstall in the desktop app; finer-grained per-plugin permission model is deferred (§D4).
- Full supply-chain signing (Sigstore / Notary). Phase 4 covers manifest signing as a starting point; full attestation is a separate initiative.
- Multi-tenant credential scoping (beyond the existing `CredentialAccessor` abstraction).

## 8. Dependencies and blocking relationships

| Phase | Blocks on                                          | Blocks                                  |
|-------|----------------------------------------------------|-----------------------------------------|
| 0     | nothing                                            | all following phases                    |
| 1     | Phase 0                                            | 2, 3, 4                                 |
| 2     | Phase 1 (needs RPC traffic to policy-check)        | 4 (community plugins on Linux)          |
| 3     | Phase 1                                            | desktop community plugins               |
| 4     | Phase 1, 2 (Linux) or 3 (desktop)                  | "community plugin marketplace" feature  |

## 9. Success criteria for the roadmap as a whole

1. A community author can write a plugin with `nebula-plugin-sdk`, declare capabilities in a manifest, and have it run under `ActionRuntime` with isolation enforced both at the broker level and the OS level (Tier 1: Linux).
2. An operator can grant/revoke capabilities without restarting the engine, and see every denial as a metric + event.
3. A malicious plugin attempting to (a) read `/etc/passwd`, (b) connect to an un-allowlisted host, (c) fork a shell, (d) OOM the host, (e) spin a CPU-hot loop — is **killed or denied in every case** on Tier 1, with a loud observability signal, and the engine keeps running.
4. The `ProcessSandbox` path is the same code path as the `InProcessSandbox` path from the caller's point of view — `ActionRuntime` does not special-case.
5. All adversarial cases above are covered by integration tests that run in CI on Linux.

## 10. Phase specs

- [Phase 0 — Wire existing ProcessSandbox into ActionRuntime](2026-04-13-sandbox-phase0-wire-existing.md)
- [Phase 1 — Broker protocol & plugin SDK](2026-04-13-sandbox-phase1-broker.md)
- [Phase 2 — Linux hardening](2026-04-13-sandbox-phase2-linux-hardening.md)
- [Phase 3 — Cross-platform sandboxing](2026-04-13-sandbox-phase3-cross-platform.md)
- [Phase 4 — Community plugin delivery](2026-04-13-sandbox-phase4-community-delivery.md)
- [Phase 5 — Input devices (desktop)](2026-04-13-sandbox-phase5-devices.md)