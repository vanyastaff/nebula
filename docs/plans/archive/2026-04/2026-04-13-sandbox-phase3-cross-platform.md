# Phase 3 — Cross-platform sandboxing

**Parent roadmap:** [2026-04-13-sandbox-roadmap.md](2026-04-13-sandbox-roadmap.md)
**Status:** spec
**Estimated effort:** ~2-3 weeks
**Blocks on:** Phase 1 (broker), benefits from Phase 2 (Linux as reference)
**Blocks:** desktop community plugins

## Goal

Extend sandboxed plugin execution to macOS and Windows. The broker (Phase 1) works unmodified on every platform — it's pure user-space IPC. What this phase adds is **per-platform OS jails as defense-in-depth**, with honest tiering so operators know what they're getting.

The desktop app (`apps/desktop`) is the primary driver: a user on macOS or Windows must be able to install a community plugin and run it with comparable safety to Linux, or at least with a clear warning about the reduced guarantees.

## Tiering

| Tier    | Platforms                             | Enforcement                                                  |
|---------|---------------------------------------|--------------------------------------------------------------|
| Tier 1  | Linux ≥ 5.13 with cgroups v2          | Full stack from Phase 2 (seccomp + landlock + ns + cgroups)  |
| Tier 2a | macOS ≥ 12                            | Broker + `sandbox-exec` profile + Mach job limits            |
| Tier 2b | Windows ≥ 10                          | Broker + AppContainer + Job Object + restricted token        |
| Tier 3  | other (older kernels, BSDs, etc.)     | Broker only, loud warning, opt-in via config flag            |

Community plugins from untrusted sources are **Tier 1 only** by default. Desktop users can opt into Tier 2 per plugin with an explicit grant. Tier 3 is never used for untrusted code.

## Non-goals

- WASI experiment — still a separate track; not blocking this phase.
- Production-grade Windows parity with Linux — AppContainer has well-known gaps and we don't try to close them.
- Mobile (iOS, Android) plugin execution — out of scope.

## macOS (Tier 2a)

### Mechanism
- **Primary:** `sandbox-exec` with a Scheme profile derived from `PluginCapabilities`. Confirmed functional on macOS 15 Sequoia 15.4; deprecation warning has been there since 10.8 (2012), Apple uses it internally — not going anywhere in the 15.x line. Apple's official replacement is App Sandbox via entitlements, which **does not apply to child processes we spawn** (it's tied to code-signed bundle at sign time) so it's not an option.
- **Critical addition — `responsibility_spawnattrs_setdisclaim(1)`**: must be called before `posix_spawn` on every plugin child. Without this, a plugin child inherits nebula-desktop's TCC grants (Camera, Microphone, Screen Recording, Accessibility) — meaning a sandbox-escaped plugin would have unmediated device access. Private API but stable since 10.14 and used by Chrome/Electron. **Non-negotiable for the two-layer permission model.** See `.project/context/research/sandbox-prior-art.md` → "macOS" for details.
- **Backup:** `posix_spawn` with `RLIMIT_*` + `setrlimit` + `POSIX_SPAWN_CLOEXEC_DEFAULT` + scratch dir under `/tmp/nebula-<uuid>`.
- **Broker:** primary enforcement for network, structured FS, devices.
- **Rejected:** Endpoint Security framework. Requires `com.apple.developer.endpoint-security.client` entitlement which Apple issues only to vetted security-product vendors after manual review. Not viable for OSS distribution. Removed from consideration.

### sandbox-exec profile template
Generated at spawn time from the plugin's granted capabilities:

```scheme
(version 1)
(deny default)
(allow process-fork (literal "/path/to/plugin/binary"))
(allow file-read* (literal "/path/to/plugin/dir"))
(allow file-read* (subpath "/tmp/nebula-plugin-<uuid>"))
(allow file-write* (subpath "/tmp/nebula-plugin-<uuid>"))
; network fully denied — broker handles it
(deny network*)
(allow mach-lookup)   ; necessary for dyld
(allow signal (target self))
```

Network is **always denied** at the OS layer; all outbound goes through the broker. This is the same structural guarantee as Linux netns.

### Limits
- CPU / memory via `setrlimit`.
- Wall-clock timeout via broker (same as Linux).
- No per-plugin FS quota on macOS — scratch dir is created with a fixed size cap and mounted on an APFS subvolume (best-effort).

### Risks
- `sandbox-exec` may be removed in a future macOS version. Accept the risk, document it, and plan a follow-up to move to a launchd-based app-container if Apple removes it.
- Code signing — community plugins are unsigned on macOS. Gatekeeper will block them unless the user explicitly allows; document this in the Phase 4 delivery story.
- `deny default` breaks many things; expect significant profile iteration during implementation.

## Windows (Tier 2b)

### Mechanism
- **Primary:** **AppContainer** via vendored `rappct` 0.13.3 crate — process runs with a constrained token, its own object namespace, no registry write access outside the package, no network unless `internetClient` capability granted. **Classic Win32 binary friction is a real implementation cost** — CRT calls in Rust dependencies (sqlite, rustls, OpenSSL) silently touch `%TEMP%`, `%LOCALAPPDATA%`, `%PROGRAMDATA%` and hit "access denied" opacity. Mitigation: **pre-staged per-AppContainer directories** with AppContainer SID ACLs; ship "bring your own runtime" plugin template that does FS via broker only.
- **Backup:** **Job Object** via `win32job` 2.0.3 crate. Flags: `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | _DIE_ON_UNHANDLED_EXCEPTION | _ACTIVE_PROCESS | _PROCESS_MEMORY | _JOB_TIME`. **Breakaway is NOT enabled** — without `JOB_OBJECT_LIMIT_BREAKAWAY_OK` on the job, a child's `CREATE_BREAKAWAY_FROM_JOB` is ignored and the child stays put.
- **Token:** no restricted-token step needed — AppContainer's token is already more restricted than any we'd build by hand via `CreateRestrictedToken`. Keep restricted tokens only as a fallback if `CreateAppContainerProfile` fails.
- **Network:** WFP filter conditioned on `FWPM_CONDITION_ALE_PACKAGE_ID = <AppContainer SID>`, action BLOCK. **Requires admin-install once** for WFP provider registration — ship `nebula-firewall-helper.exe` as a separate signed binary that registers the provider with a stable GUID at desktop-app install time. Per-spawn dynamic filters are non-elevated after that. Rust bindings thin — write our own wrapper around `windows` crate's WFP FFI.
- **Smart App Control (SAC)** is a **documented incompatibility** on fresh Windows 11 Home. Unsigned plugin binaries from the internet are blocked by cloud reputation. Operators must either whitelist the plugin directory (requires elevation) or disable SAC. Document explicitly in Phase 4.

### Architecture
- Plugin binary spawned via `CreateProcessAsUser` with the restricted token.
- Assigned to a Job Object with:
  - `JobObjectExtendedLimitInformation`: memory cap, active process cap, UI restrictions.
  - `JobObjectBasicLimitInformation`: CPU time cap.
  - `JobObjectCpuRateControlInformation`: CPU rate cap.
- AppContainer created per plugin with a fresh SID; lifetime-bound to the invocation.
- Network access **denied** at the Windows Filtering Platform (WFP) layer via a firewall rule tied to the AppContainer SID. Again: all network goes through the broker.

### Crate choices
- `windows` crate (official Microsoft bindings) for Job Object + token APIs.
- `wfp` bindings — may need our own thin wrapper; evaluate `windivert` or direct FFI.

### Limits
- Memory, CPU, process count, UI-access — all via Job Object.
- Filesystem — AppContainer's isolated object namespace + broker-mediated `fs.read`/`fs.write`.
- Network — broker-mediated, WFP-blocked at OS layer.

### Risks
- AppContainer is originally designed for UWP apps; running classic Win32 binaries inside it is supported but quirky. Expect iteration.
- Some DLLs expect access to user profile paths — document required carve-outs.
- WFP rules per-AppContainer require admin rights on first install. Acceptable for desktop app context.

## Broker additions for Phase 3

- **Platform detection:** `CapabilityBroker` learns its tier at construction time and records it in metrics (`sandbox_tier_info{tier=1|2a|2b|3}`).
- **Tier-aware fail-safes:** on Tier 2/3, the broker **tightens** its own policy (shorter default timeouts, smaller byte budgets, stricter domain allowlists) because the OS jail is weaker.
- **Tier 3 warning path:** at engine boot, if no supported OS jail is available, log a `warn` every 60s for the first 10 minutes: "running plugins in Tier 3 — broker-only enforcement". Refuse to run any plugin flagged `untrusted: true` in its manifest.

## Work breakdown

1. **Platform abstraction** — new trait `OsJail` in `os_sandbox.rs` with impls `LinuxJail`, `MacJail`, `WindowsJail`, `NoopJail`. 2 days.
2. **macOS sandbox-exec profile generator** — from `PluginCapabilities` to Scheme. 2-3 days.
3. **macOS spawn path** — `pre_exec` equivalent using `posix_spawn_file_actions`. 2 days.
4. **macOS adversarial suite** — same attacks as Linux, adjusted expectations per tier. 2 days.
5. **Windows AppContainer + Job Object spawn** — heavy lifting, expect 4-5 days.
6. **Windows WFP network block** — 2-3 days (plus admin setup documentation).
7. **Windows adversarial suite** — 2 days.
8. **Broker tier detection + tightened policy on Tier 2/3** — 1 day.
9. **Context docs + tier guarantees document** — 1 day.
10. **CI matrix** — GitHub Actions runners for Linux (Tier 1), macOS (Tier 2a), Windows (Tier 2b). 1 day.

## Acceptance criteria

- [ ] The platform abstraction compiles and tests pass on Linux, macOS, and Windows runners.
- [ ] Adversarial test suite runs on macOS: file read outside scope, network connect outside broker → denied.
- [ ] Adversarial test suite runs on Windows: same.
- [ ] Tier is reported in metrics and surfaced in the desktop app's plugin-install UI.
- [ ] Tier 3 fallback refuses to run plugins flagged `untrusted: true`.
- [ ] Documentation clearly states what each tier guarantees and does not guarantee.
- [ ] `cargo nextest run --workspace` green on the CI matrix.

## Risks

| Risk | Mitigation |
|------|------------|
| macOS `sandbox-exec` deprecation | Flagged in docs; fallback plan to app-container with launchd |
| Windows AppContainer + Win32 binary incompatibility | Start with a simple plugin; iterate profiles; accept some capability carve-outs |
| WFP firewall rule requires admin install | Desktop installer handles it; CLI doc notes the requirement |
| CI runners lack the kernel features | Pin runner versions; use self-hosted for Linux where needed |
| Divergence between platforms silently weakens guarantees | Tier is a first-class concept in metrics, desktop UI, and docs |

## Deliverables

- `crates/sandbox/src/os_sandbox.rs` refactored into platform-specific modules (`linux/`, `mac/`, `windows/`, `noop/`).
- A `tier_guarantees.md` document in `.project/context/crates/` listing what each tier protects against and what it does not.
- Desktop app (Tauri) surfaces the tier to the user when installing or running a plugin.
- CI matrix green on all three platforms for the adversarial suite (with tier-adjusted expectations).
