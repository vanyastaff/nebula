# Sandbox prior art — research digest

Research done 2026-04-13 for nebula-sandbox design. Three parallel research passes covered: Rust crate ecosystem, non-Rust plugin architectures, macOS/Windows sandbox surfaces. This file is the digest; full reports were ephemeral.

**Confirmed architectural decisions at the end.** Future sessions should read this before touching sandbox plan docs.

---

## Rust crates — dependency verdicts

### Dependencies we take

| Crate | Version | Role | License | Status |
|---|---|---|---|---|
| `landlock` | 0.4.4 | Linux FS LSM | MIT/Apache | 7.3M dl, 52 rdeps, LSM team owns |
| `seccompiler` | 0.5.0 | Seccomp-bpf compiler | Apache-2.0 | 11.2M dl, Firecracker owns; standard in 2025 |
| `cgroups-rs` | 0.5.0 | cgroup v1/v2 | MIT/Apache | 4.4M dl, kata-containers owns |
| `rustix` | 1.1.4 | syscall wrappers | Apache/MIT/BSD | **718M dl** — overtook `nix`, BCA owns |
| `caps` | 0.5.6 | Linux capabilities | MIT/Apache | 11.7M dl, small, pure-Rust |
| `win32job` | 2.0.3 | Win Job Objects | MIT | 648K dl, mature, safe API |
| `rappct` | 0.13.3 | Win AppContainer/LPAC | MIT | **Vendor it** — single author, new (2025-10), no alternative |
| `interprocess` | 2.4.0 | Cross-platform UDS + named pipes | 0BSD | 8.2M dl, 106 rdeps, sync + tokio |
| `memmap2` | 0.9.10 | mmap | MIT/Apache | 220M dl, replacement for `memmap` |
| `shared_memory` | 0.12.4 | POSIX shm + Win FileMapping | MIT | **Pin, fork-ready** — stagnant since 2022 but no alternative |
| `raw_sync` | 0.1.5 | Cross-process sync primitives | MIT | **Pin, fork-ready** — stagnant since 2020 |
| `postcard` | 1.1.3 | Binary wire format | MIT/Apache | 27.8M dl, COBS-framed, no_std clean, stable 1.x |
| `rkyv` | 0.8.15 | Zero-copy archive | MIT | 97M dl, for device-ring hot path only |
| `cap-std` | 4.0.2 | Capability-based std | Apache-2.0 | 9.6M dl, BCA owns. **Host-side only** — makes path-traversal bugs in our broker code structurally impossible. Not an enforcement layer. |
| ~~`tonic` + `prost` + `rustls` + `rcgen`~~ | ~~latest~~ | ~~gRPC transport~~ | ~~MIT/Apache~~ | **Rejected 2026-04-13** — Rust-only plugin constraint means no cross-language interop need; ~65 transitive crates not justified. Slice 1c ships UDS / Named Pipe via tokio `net` feature alone. See D1 above. |
| `libloading` | 0.9.0 | dlopen/LoadLibrary | ISC | 341M dl — **only if** we ever ship in-process plugins (currently rejected) |

### Crates we study but don't depend on

| Crate | Why study | Why skip dep |
|---|---|---|
| `hakoniwa` | Full Linux stack in one crate (namespaces + pivot_root + cgroup v2 + landlock + seccompiler + uid/gid maps). 28K dl, active 2026-04-11. **Closest existing thing to nebula-sandbox Linux backend.** | External dep in security-critical path. **Read as reference for ordering** (it gets the `unshare` → `pivot_root` → mount plan → cgroup attach → landlock → seccomp sequence right), wire primitives ourselves. |
| `nono` | Closest to "modern gaol". Linux+macOS. Apache-2.0. Active 2026-04-12. | No Windows. Not cross-platform enough for our Tier 2b. |
| `extrasafe` | Best policy-builder DSL in Rust seccomp space. Type-driven composability (`SafetyContext::new().enable(Networking::nothing().allow_running_tcp_clients())`). | Linux only. **Vendor if we want the DSL shape** — it's small enough. |
| `birdcage` | Phylum's cross-platform sandbox for package manager postinstall. Linux+macOS. Clean `Exception` API. | **GPL-3.0 — license blocker.** Copy the API shape, don't depend. |
| `gaol` | Historical broker/child split pattern. Where the pattern was first articulated in Rust. | **Dead since 2019-10.** Learn, don't depend. |
| `youki`'s `libcontainer` / `libcgroups` sub-crates | Production-grade Linux primitive ordering | Part of a full OCI runtime; we only need the wiring pattern. |

### Crates explicitly rejected

| Crate | Reason |
|---|---|
| `tarpc` | Bakes its own framing/serde; would fight it for stdio+length-prefix+JSON-RPC semantics. Plugin authors shouldn't need to learn tarpc. |
| `abi_stable` | Stale since 2023-10. Confirms decision to go child-process instead of in-process dylib. |
| `libseccomp` | 1.4M dl vs seccompiler 11.2M. Requires C lib dep. Use seccompiler unless we need syscall-name resolver. |
| `nix` | Still maintained (514M dl) but `rustix` (718M) has overtaken it. rustix is faster on Linux (linux_raw backend), covers more platforms, BCA-owned. Use `nix` only for the handful of ioctls `rustix` lacks. |
| `ipmpsc` | Dead 2021. Build SPSC ring on `memmap2 + raw_sync` directly. |
| Endpoint Security bindings (`endpoint-sec`, `endpointsecurity-rs`) | Bindings fine; **Apple entitlement is the blocker**. `com.apple.developer.endpoint-security.client` is issued only after manual Apple review for security-product use cases. Not viable for OSS. |
| `bubblewrap` / `minijail` Rust bindings | No serious bindings exist. If we ever want bwrap, we exec it, not link it. Skip. |

---

## Non-Rust prior art — lessons

### HashiCorp go-plugin (Terraform, Vault, Nomad, Packer)

The reference for "long-lived plugin subprocess with RPC", ~10 years in production.

**Steal:**
- **Handshake line**: plugin writes one line to stdout → `CORE-PROTO | APP-PROTO | unix|tcp | addr | grpc|netrpc`. Host parses, dials.
- **AutoMTLS**: one-shot self-signed certs in both directions; only the launching host can speak to the running plugin instance. Critical when TCP loopback is the transport (Windows case).
- **Reattach**: host can restart without killing long-running plugins; plugin is told its ID and connection info, keeps running, new host dials in.
- **Transport**: gRPC over UDS on Linux/macOS, **TCP loopback** on Windows (Windows UDS is supported since 1803 but tooling is uneven; go-plugin uses TCP loopback in production with `PLUGIN_MIN_PORT`/`PLUGIN_MAX_PORT` env vars).
- **MagicCookie**: env var checked at plugin startup, documented as UX (stop randos from running a Vault plugin standalone), NOT security.
- **stderr is logs, never protocol.** Host has a task that tries to parse each line as structured log; falls back to verbatim with `plugin:<name>` prefix. gRPC mode has a dedicated `GRPCStdio` streaming service for the same purpose.
- **GRPCBroker for bidirectional callbacks**: host passes an interface to plugin, plugin calls it back. Implemented by broker handing out stream IDs over the main gRPC channel, either side calls `Accept(id)` to get a listener, serves another gRPC server on top of the multiplexed stream. This is how Terraform providers can ask the host to render UI prompts, etc.

**Ignore:**
- **Sandboxing**. go-plugin does none. It's a protocol library; OS-level sandbox is explicitly left to the embedder. nebula does what go-plugin punted on.

### Nomad shape vs Terraform shape (both use go-plugin)

Same library, two very different lifecycles:

- **Terraform provider**: short-lived, one instance per provider per `plan/apply` command, dies when command ends. Schema-driven CRUD.
- **Nomad driver**: **long-lived**, one per driver per client node, survives client restart via `Reattach`. Implements `StartTask/StopTask/InspectTask/SignalTask`. Declares `Capabilities` (fs isolation, network isolation, can-signal) enforced by the framework.

**nebula is Nomad-shape.** Workflow executions run hours to days; we want long-lived plugin processes with reattach, declared-capabilities contract enforced by host, engine can restart without killing in-flight work.

### Zapier Platform / Zapier CLI apps

**Steal:**
- **Brokered credentials**: plugin never sees long-lived secrets. Integration receives `bundle.authData` for one call only. Zapier holds OAuth refresh, plugin sees only access token per-call. This is our `CredentialRef` design, validated.
- **Per-invocation wall-clock and RSS limits**: copied from Lambda. Every plugin RPC has a ceiling.

**Ignore:**
- Hosting plugins in AWS Lambda. Nebula is self-hosted; we need to build the equivalent jail ourselves.

### Deno permissions / ops

**Steal:**
- **Runtime permission check at the op boundary**: every `Deno.fetch`, `Deno.readFile`, etc. is a Rust op that calls `state.borrow::<Permissions>().check_*()` at runtime. Narrow, auditable, no compile-time magic. Matches our broker RPC design.
- **`query / request / revoke` API shape**: clean triple for runtime capability management. Consider for nebula's `CapabilityBroker` public API.

**Avoid (documented footgun):**
- **"Allow once, allow forever" for streams**: in Deno, `--allow-net=api.foo.com` lets you fetch any amount of data for the process lifetime. For long-lived nebula workflows, this is wrong — we need per-call byte caps and per-capability wall-clock budgets, checked on every RPC.

### Figma plugin sandbox

**Steal:**
- **Two-realm split**: plugin logic runs in QuickJS realm with zero browser APIs — no `fetch`, no DOM, no `setTimeout`. Only scene-manipulation API. A separate sandboxed `<iframe>` handles network/credentials and communicates via `postMessage`. Mirror this: split plugin work into "logic that touches data model" (heavily restricted) and "logic that touches outside world" (declared egress allowlist, host-enforced).
- **`manifest.json` with `networkAccess.allowedDomains`**: declarative, enforced by CSP + review process.

### VS Code extension host

**Cautionary tale:**
- Extensions share one Node process per window, monkey-patch each other's globals. Don't do this. The reason we have process-per-plugin.
- **Workspace Trust** (`capabilities.untrustedWorkspaces.supported: true|false|"limited"` in `package.json`): extensions self-declare their trust. **Anti-pattern.** The plugin doesn't tell us how trusted it is — the user grants scopes, the host enforces, the plugin only declares what it needs.

### Obsidian / n8n / HACS

**All three: in-process loading of community code → no security story.** Cautionary tales confirming the child-process decision.

- Obsidian: Electron renderer loads arbitrary Node modules. Full `fs`/`child_process`/`net` access. No sandbox.
- n8n community nodes: npm packages in the same Node process. Docs explicit: *"community nodes have full access to the machine that n8n runs on, and can do anything, including malicious actions."*
- HACS / Home Assistant custom components: Python packages in same process. HA docs: "not reviewed or tested, may negatively impact stability."

**Lesson**: "we'll review on submission" doesn't scale and doesn't survive update-time supply-chain attacks. Make plugins *structurally incapable* of the bad thing.

### Discord / Slack apps

**Steal:**
- **Scope-based consent UI**: user installs an app, sees a consent screen ("this app can read messages, post, manage channels"), grants scoped tokens. Copy for nebula install-time grant: "this plugin wants `network:api.stripe.com`, `camera:capture`" → user accepts once → scope recorded → runtime enforces.

**Ignore:**
- Architecture (third-party code runs on the developer's own infra). Different product shape.

---

## macOS — surface analysis

### sandbox-exec (Seatbelt)
**Still functional on macOS 15 Sequoia (15.4 confirmed).** Deprecation warning has been present since macOS 10.8 (2012). Apple themselves use it to launch Safari, Mail, many system services. Not going anywhere in the 15.x line. Profile language (`file-read*`, `network*`, `process-fork`, `mach-lookup`, `iokit-open`, etc.) is expressive. **Our Tier 2a primary.**

Rust wrappers are all thin shells around `sandbox-exec` command. Write 50 lines around `Command::new("sandbox-exec")` ourselves.

### App Sandbox via entitlements
Baked into signed app bundle at sign time. **Cannot be applied to arbitrary child process we spawn.** Useless for our case.

### Endpoint Security framework
Powerful (observe/deny exec/fork/open/mmap/connect since macOS 14). **But**: requires `com.apple.developer.endpoint-security.client` entitlement, issued only after Apple manual review for security-product use cases. **Unusable for OSS distribution.** Remove from plans.

### TCC and `responsibility_spawnattrs_setdisclaim(1)` — CRITICAL

**Default behavior**: child process inherits "responsible process" = first ancestor the user actually launched. If nebula-desktop has Screen Recording grant, plugin child piggybacks on that grant automatically.

**This is a zero-click privacy breach vector.** A plugin with a sandbox escape gets unmediated camera/mic/screen access.

**Mitigation**: call `responsibility_spawnattrs_setdisclaim(attr, 1)` before `posix_spawn`. Child becomes its own responsible process. Plugin trying to open device directly → OS denies (no grant).

Private API but **stable since macOS 10.14** and used by Chrome and Electron. Safe to depend on.

**Net effect with disclaim**:
- nebula-desktop holds TCC grants (user granted once).
- Plugin child is disclaimed → has no TCC grants.
- Plugin tries `/dev/video0` / AVCaptureDevice → OS blocks.
- Plugin must go through broker RPC → broker in host process (with TCC) opens device → broker writes frames to shared memory ring → plugin reads.
- Even if plugin escapes sandbox → cannot access camera without going through broker, because it's disclaimed at the OS level.

This is **defense in depth for the two-layer permission model**: OS grant goes to app; plugin permission system gates which plugins use it; disclaim prevents bypass.

### Gatekeeper / quarantine / code signing
Unsigned plugin binaries downloaded from the internet hit Gatekeeper quarantine on first launch. Plugin does **not** need to be Apple-signed to run inside sandbox-exec. **Phase 4 install flow must strip `com.apple.quarantine` xattr after verifying our own checksum.**

### Recommended macOS stack
- **Spawn**: `posix_spawn` + `responsibility_spawnattrs_setdisclaim(1)` + `setrlimit` + `POSIX_SPAWN_CLOEXEC_DEFAULT` + scratch dir under `/tmp/nebula-<uuid>`.
- **Sandbox**: `sandbox-exec -f <generated.sb>` wrapping the call. Generated profile: `(version 1) (deny default) (allow process-fork) (allow file-read* (subpath "/tmp/nebula-<uuid>")) (allow file-write* (subpath "/tmp/nebula-<uuid>")) (deny network*) (allow mach-lookup) (allow signal (target self))` plus dyld carve-outs.
- **Broker**: primary enforcement for network, structured FS, devices.
- **Fallback if Apple removes sandbox-exec**: drop to `posix_spawn` + rlimits + disclaim only; broker-only enforcement = Tier 3.

---

## Windows — surface analysis

### AppContainer
The right primitive. Token with AppContainer SID, fresh per-instance object namespace, no user profile access by default, structural network deny unless `internetClient` capability granted.

**Classic Win32 binary friction is real**: CRT touches `%TEMP%`, `%LOCALAPPDATA%`, `%PROGRAMDATA%` paths the AppContainer can't see. Sqlite creates `-journal`, OpenSSL looks for `cacert.pem`. **Mitigation**: pre-staged per-AppContainer directories with AppContainer SID explicit ACLs. Ship "bring your own runtime" plugin template that uses broker FS API, not libc.

Rust crate `rappct` 0.13.3 (single maintainer, October 2025, 3.6K dl). **Vendor, not depend directly.**

### Job Objects
Mature, perfect for memory/CPU/process-count/UI restrictions. `win32job` 2.0.3 (648K dl) is the take.

**Combine with AppContainer**: assign AppContainer-launched process to job after `ResumeThread`. Nested jobs supported since Win8.

**Breakaway**: child escapes only if **both** job has `JOB_OBJECT_LIMIT_BREAKAWAY_OK` AND child created with `CREATE_BREAKAWAY_FROM_JOB`. **Don't set breakaway** on the job → child stays put regardless of what it tries.

Set `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | _DIE_ON_UNHANDLED_EXCEPTION | _ACTIVE_PROCESS | _PROCESS_MEMORY | _JOB_TIME`.

### WFP (Windows Filtering Platform)
For blocking network at kernel granularity per AppContainer SID. Install filter with `FwpmFilterAdd0`, condition `FWPM_CONDITION_ALE_PACKAGE_ID = <package SID>`, action BLOCK.

**Admin install requirement**: registering the WFP provider with a stable GUID requires admin **once at install time**. Per-spawn dynamic filters are non-elevated after that. **Ship a `nebula-firewall-helper.exe` as a separate signed binary** that does the install-time provider registration.

Rust crates (`windows-wfp`, `wfp`) are new and low-adoption — probably need to write our own thin wrapper for AppContainer-SID-conditioned filters.

### Smart App Control (SAC)
On by default on fresh Windows 11 Home installs. Blocks unsigned binaries from the internet outright based on cloud reputation. **Documented incompatibility**: operators with SAC on must either add nebula's plugin directory to exclusions (requires elevation) or disable SAC.

### Windows Sandbox (`.wsb`)
Hyper-V-backed full disposable VM. Too heavy for per-plugin. No library API. **Skip.** Document as "paranoid users run entire nebula-desktop inside one."

### SetWindowsHookEx and AV telemetry
Global keyboard hooks trigger AV flagging. **Don't use SetWindowsHookEx** for anything in broker. For Phase 5 global keyboard, use raw input or UI Automation.

### Recommended Windows stack
- **Token**: AppContainer profile via `CreateAppContainerProfile`, no network caps, no documentsLibrary, no picturesLibrary.
- **Spawn**: `CreateProcessAsUser` with `EXTENDED_STARTUPINFO_PRESENT` + `PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`, suspended.
- **Job**: `win32job::Job` with kill-on-close/die-on-exception, no breakaway, UI restrictions enabled, assign before `ResumeThread`.
- **Network**: WFP filter per AppContainer SID, installed by `nebula-firewall-helper.exe` at app install.
- **Filesystem**: scratch dir under `%LOCALAPPDATA%\nebula\plugins\<uuid>` ACL'd to AppContainer SID.
- **Dependencies**: `rappct` (vendored), `win32job`, `windows` crate for raw WFP + security calls.

---

## Patterns we steal (summary)

1. **go-plugin handshake + AutoMTLS + Reattach** — wire format and lifecycle for Phase 1.
2. **Nomad-shape long-lived subprocess** per `(plugin, credential-scope)` tuple.
3. **Zapier-style brokered credentials** — `CredentialRef` indirection, raw secret never crosses IPC.
4. **Figma two-realm split** — plugin logic realm (no outside I/O) + broker realm (policy-checked outside I/O).
5. **Discord/Slack scope-based consent UI** — install-time manifest review, user grants, host enforces.
6. **Runtime op-boundary checks** (Deno) — every broker RPC is a policy check.

## Anti-patterns we avoid

1. **In-process loading of community code** (n8n, Obsidian, HACS). No security story possible.
2. **Self-declared trust tiers** (VS Code Workspace Trust). Plugin doesn't say how trusted it is; user grants, host enforces.
3. **"Stream allowed once = allowed forever"** (Deno footgun). Per-call wall-clock + byte caps.
4. **Shared process per plugin container** (VS Code extension host). Process-per-plugin.
5. **Runtime-only enforcement** (go-plugin). We do runtime enforcement **plus** OS sandbox as defense in depth.

---

## Single-layer OS permission model (confirmed after deferring plugin layer)

```
OS permission (app-level, granted once to nebula-desktop)
├── macOS: TCC (Camera, Microphone, Screen Recording, Accessibility, Location)
├── Windows: Privacy settings + AppContainer capability SIDs + UAC
└── Linux (desktop): xdg-desktop-portal (camera, microphone, location, global-shortcuts)

Granted ONCE to nebula-desktop (the signed app bundle) by the user via OS prompts.
No second per-plugin layer — see "Deferred decisions" above.
```

**Why this is safe without a plugin-declared layer:**

- `responsibility_spawnattrs_setdisclaim(1)` on macOS makes the plugin child its own TCC responsible process — it does **not** inherit nebula-desktop's grants. Plugin cannot open camera/mic/screen directly.
- AppContainer on Windows: children get a fresh security principal, don't inherit parent's privacy grants.
- Empty netns + no portal session on Linux: plugin child literally cannot reach `xdg-desktop-portal`.

Plugin's only path to devices is **through broker RPC**. Broker runs in the host process (which holds the OS grant), opens the device via OS API, streams frames to plugin via shared memory ring. Audit log records which plugin used which device.

**How it works in practice:**

1. User installs nebula-desktop. On first use of a device, OS prompts: "nebula wants to use Camera". User allows. Host now has OS-level grant.
2. User installs a plugin (signed, verified). Desktop dialog: "Install Telegram plugin by alice@example.com? ✓ Signature verified". User confirms install — **no capability grant dialog**.
3. At workflow runtime: plugin child is spawned with disclaim. Plugin has no OS grants. Plugin calls `media.camera.open` over gRPC broker.
4. Broker (running in host process, has TCC) opens the camera via OS API, sets up shm ring, returns `MediaStreamRef`. Audit event `SandboxDeviceOpen { plugin, device }` emitted.
5. Tray indicator appears: "Camera in use by Telegram plugin".
6. Plugin reads frames from shm.
7. If plugin escapes sandbox and tries to call `AVCaptureDevice` directly → OS denies (no TCC on disclaimed child).

Operator reviews audit log post-hoc via `nebula plugin logs <plugin>` or desktop UI — if a plugin used a device it shouldn't have, operator uninstalls it.

This is the core invariant: **OS grant to host, broker mediates, plugin is structurally incapable of bypassing broker**. Fine-grained per-plugin scope (if it ever becomes a real requirement) layers on top later.

---

## Deferred decisions

### Permission manifest model (deferred 2026-04-13)

The entire `[permissions]` table in `plugin.toml` — researched extensively against Tauri, Cargo, Deno, Chrome MV3, WASI, Flatpak, GH Actions in `sandbox-permission-formats.md` — is **deferred out of Phase 1–4**. The design was speculative: we had no operator requirements, no community-plugin requirements, and no real use cases that couldn't be satisfied by the process-isolation + broker + audit-log stack. Trying to ship it anyway risked building the wrong thing.

Instead, Phase 1–4 ship:

1. Process isolation (plugin cannot touch FS/network/devices by construction).
2. Broker gRPC as sole exit (every outside-world call host-mediated).
3. Anti-SSRF + private-IP blocklist (always on).
4. Audit log of every broker RPC (metrics + EventBus).
5. Signed manifest (identity + supply-chain).
6. OS jail (seccomp/landlock/cgroups on Linux; sandbox-exec + disclaim on macOS; AppContainer + Job Object + WFP on Windows).

This is strictly better than n8n's in-process model (no isolation at all) without the speculative design burden.

**When to revisit**: a real operator says "plugin X must only talk to `*.company.internal`, and audit log is not enough — we need it to be enforced at the sandbox layer". Until then, defer. When revisiting, the full research is preserved in `sandbox-permission-formats.md`.

**What remains simple**:
- Plugin manifest is 9 lines: `[plugin]` (identity + `sdk-version` + `binary-sha256`) and `[signing]` (ed25519).
- Resource limits come from `nebula-runtime` / workflow-config, not the plugin author.
- Actions, parameters, credential slots, resource slots come from `#[derive(Action)]` via `__metadata__` gRPC at register time.

## Confirmed architectural decisions

### D1 — Transport: plain UDS / Windows Named Pipe + line-delimited JSON
**No gRPC, no protobuf, no TLS.** Initially we considered the go-plugin stack (tonic + prost + rustls + rcgen, ~65 transitive crates) but that pattern is designed for cross-language plugin ecosystems — Terraform plugins can be written in any language that speaks gRPC. nebula is deliberately **Rust-only** (the user rejected JS/Python plugins on security grounds; see roadmap §D5), so we don't pay for cross-language interop we don't need.

**Handshake line** printed by plugin to stdout: `NEBULA-PROTO-2|unix|/tmp/nebula-plugin-<uuid>.sock` or `NEBULA-PROTO-2|pipe|\\.\pipe\LOCAL\nebula-plugin-<uuid>`. Host dials the socket/pipe directly.

**Auth** is filesystem / pipe object namespace ACLs — UDS mode `0600` inside a parent dir with mode `0700` (Linux/macOS); session-scoped named pipe with user-SID DACL (Windows). Same security primitive as SSH agent, systemd, dbus, Docker socket, LSP servers.

**Prior art**: LSP (Language Server Protocol — rust-analyzer, clangd, gopls) and DAP (Debug Adapter Protocol — every serious debugger UI). Plain bidirectional JSON-RPC over stdio or sockets. 10+ years in production across millions of users and every platform. Zero gRPC. Proves the light path is production-grade.

**What we keep from go-plugin**: the *lifecycle pattern* (long-lived subprocess, Reattach, supervisor model) — just without the transport baggage. Slice 1c ships this.

Plugin-SDK hides all transport details behind `run_duplex(handler)`; plugin authors see a simple async trait.

### D2 — Lifecycle: long-lived per `(plugin_key, credential_scope)` with Reattach
Not per-invocation. Not per-engine-instance. Per credential-scope — same plugin with different credentials = different process (prevents credential leak). Reattach means engine can restart without killing in-flight workflows.

### D3 — Linux backend: manual wiring with hakoniwa/youki as reference
Depend on `landlock + seccompiler + cgroups-rs + rustix + caps` directly. Read hakoniwa source for correct setup order. Not depending on hakoniwa lets us match our policy format exactly and avoids an external dep in security-critical code. Reserve right to pivot to hakoniwa if our wiring grows bug-laden.

### D4 — macOS sandbox: `sandbox-exec` + disclaim + broker
Tier 2a stack is `posix_spawn` + `responsibility_spawnattrs_setdisclaim(1)` + `setrlimit` + `sandbox-exec` profile + broker. No Endpoint Security (entitlement blocker). Broker is primary enforcement; sandbox-exec + disclaim is defense in depth.

### D5 — Windows sandbox: AppContainer + Job Object + WFP
Tier 2b stack is AppContainer (via vendored `rappct`) + Job Object (`win32job`) + WFP filter per AppContainer SID (custom wrapper). Admin install required **once** for WFP provider registration. CRT friction is a real implementation cost — plan for pre-staged directories.

### D6 — Two-layer permission model
OS permission granted to nebula-desktop app once; plugin permission granted per-plugin via nebula's own grant UI. `responsibility_spawnattrs_setdisclaim(1)` on macOS prevents plugin children from inheriting host OS grants, forcing all device access through the broker. Same principle applies on all platforms (AppContainer on Windows naturally doesn't inherit grants; Linux uses portal tokens).

### D7 — Shared memory primitives
`memmap2 + shared_memory + raw_sync`, pinned and fork-ready. No alternative exists. Linux prefers `memfd_create` with `F_ADD_SEALS` for device rings.

### D8 — Credential indirection via `CredentialRef`
Raw credentials never cross the IPC boundary. Plugin asks for `credentials.get { slot }` (slot name from action metadata, not plugin.toml). Broker returns an opaque nonce; plugin passes nonce into `network.http_request { auth: CredentialRef }`; broker resolves and injects header host-side. Valid for one invocation only.

---

## Links and references

### Rust crates (all as of 2026-04-13)
- `landlock` — https://crates.io/crates/landlock (0.4.4)
- `seccompiler` — https://crates.io/crates/seccompiler (0.5.0)
- `cgroups-rs` — https://crates.io/crates/cgroups-rs (0.5.0)
- `rustix` — https://crates.io/crates/rustix (1.1.4)
- `interprocess` — https://crates.io/crates/interprocess (2.4.0)
- `win32job` — https://crates.io/crates/win32job (2.0.3)
- `rappct` — https://crates.io/crates/rappct (0.13.3)
- `hakoniwa` — https://crates.io/crates/hakoniwa (1.5.0)
- `extrasafe` — https://crates.io/crates/extrasafe (0.5.1)
- `cap-std` — https://crates.io/crates/cap-std (4.0.2)

### Non-Rust prior art
- go-plugin: https://github.com/hashicorp/go-plugin
- Deno permissions: https://docs.deno.com/runtime/fundamentals/security
- Zapier Platform: https://platform.zapier.com/
- Figma plugin API: https://www.figma.com/plugin-docs/
- VS Code Workspace Trust: https://code.visualstudio.com/api/extension-guides/workspace-trust

### OS-specific docs
- macOS TCC deep dive: https://angelica.gitbook.io/hacktricks/macos-hardening/macos-security-and-privilege-escalation/macos-security-protections/macos-tcc
- `responsibility_spawnattrs_setdisclaim`: https://www.qt.io/blog/the-curious-case-of-the-responsible-process
- sandbox-exec SBPL: https://theapplewiki.com/wiki/Dev:Seatbelt
- Windows Job Objects: https://learn.microsoft.com/en-us/windows/win32/procthread/job-objects
- Windows AppContainer + WFP: https://projectzero.google/2021/08/understanding-network-access-windows-app.html
- WFP defensive techniques: https://textslashplain.com/2025/03/31/defensive-technology-windows-filtering-platform/

---

**End of digest. Update this file only when decisions change. For ephemeral exploration, use throwaway research.**
