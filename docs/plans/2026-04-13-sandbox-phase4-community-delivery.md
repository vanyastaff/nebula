# Phase 4 — Community plugin delivery

**Parent roadmap:** [2026-04-13-sandbox-roadmap.md](2026-04-13-sandbox-roadmap.md)
**Status:** spec
**Estimated effort:** ~3-4 weeks (reduced — no permission grant flow)
**Blocks on:** Phase 1 (broker), Phase 2 (Linux) or Phase 3 (desktop)

## Goal

Turn a working sandbox into a shipping story. An external author must be able to write a plugin, sign it, publish it, and have an operator install, run, and uninstall it — with supply-chain integrity enforced at every step. The plugin runs under process isolation + OS jail + broker-mediated I/O (Phases 1-3); there is **no permission declaration in the manifest** (that model is deferred until requirements crystallize — see roadmap §D4).

## Non-goals

- Permission manifest and grant UI — **deferred**. Trust is all-or-nothing: you install a signed plugin or you don't. Scope enforcement is sandbox-wide (process isolation, anti-SSRF, audit log), not per-plugin-declared.
- Full Sigstore / Notary integration. Phase 4 ships basic ed25519 signing; richer attestation is a follow-up.
- Payment / monetization for the registry.
- Third-party auditing workflows (human reviews).
- Plugin hot-reload on running workflows. Restart-to-upgrade is acceptable for v1.

## Components

### 1. Plugin manifest v1

A signed TOML file co-located with the plugin binary. **Minimal by design**: only identity, SDK compatibility, and signing. Nothing else. Everything a plugin actually does (actions, parameter schemas, credential/resource slot declarations, resource limits) comes from `#[derive(Action)]` metadata delivered via the `__metadata__` gRPC call at register time, or from workflow-config / engine-config (resource limits, timeouts — set by `nebula-runtime`).

```toml
# plugin.toml
[plugin]
key         = "com.author.telegram"
version     = "1.2.3"
author      = "alice@example.com"
description = "Send messages through Telegram Bot API"
sdk-version = "1.2.0"

[signing]
algorithm     = "ed25519"
public-key    = "..."
signature     = "..."
binary-sha256 = "..."
```

**Invariants:**

1. **No `[permissions]` section.** Scope is sandbox-wide, not per-plugin-declared. See roadmap §D4.
2. **No `[runtime]` section.** Resource limits (memory, CPU, timeout) are set by `nebula-runtime` / workflow-config — operator, not plugin author, decides.
3. **No `[actions]`, `[[credentials]]`, `[[resources]]`.** Action metadata and slot declarations come from derive-macros via `__metadata__` at register time. Engine validates slot types against the credential / resource type registry.
4. **`sdk-version`** declares compile-time dependency on `nebula-plugin-sdk`. Host checks `SUPPORTED_SDK_RANGE.contains(manifest.sdk_version)` at register time; refuse-to-spawn on incompatibility (faster and cleaner error than failing in handshake).
5. **`binary-sha256`** is verified at spawn time — a signed manifest for binary `A` cannot be reused to run binary `B`.

### 2. Signing model

- **Key pair:** ed25519. Private key stays with the author; public key is embedded in the manifest and also registered with the registry.
- **Signature:** detached, over the canonical TOML serialization of the `[plugin]` section only (all other sections derived or separately verified). Key order alphabetical. `binary-sha256` is part of `[plugin]` so the signature binds manifest to binary.
- **Verification:** every load path (`discover_plugin`, registry install, runtime check) re-verifies before the plugin is usable. Mismatch → refuse to register, log `SandboxPluginSignatureInvalid { plugin, reason }`.
- **Trust anchors:** operators maintain a list of trusted public keys in `nebula-engine` config. Desktop users manage theirs via the desktop app's plugin-install dialog. Unknown-signer plugins either refuse to load (strict mode) or load with a warning (default mode) — configurable per deployment.
- **SDK-version compatibility check**: at register time, before signature verification, host checks manifest's `sdk-version` against its supported range. Incompatible → refuse with clear error.

Phase 4 ships basic ed25519 signing. Sigstore, Notary, or SBOM attestation are follow-up work.

### 3. Install flow

There is **no capability grant dialog** — install is a simpler decision: "do you trust this plugin author?".

```
nebula plugin install <url|path>
  1. Fetch manifest + binary
  2. Verify sdk-version compatibility
  3. Verify ed25519 signature over [plugin]
  4. Verify binary-sha256 matches fetched binary
  5. Check public key against trust anchors:
     - If in trusted list → install silently
     - If unknown + strict mode → refuse
     - If unknown + default mode → show confirmation dialog:
         "Install unsigned-by-known-key plugin <name> by <author>?
          Binary hash: <sha256>"
  6. Copy binary + manifest to plugin directory
  7. Register with engine (loads manifest, calls __metadata__ via gRPC)
  8. Actions become available in workflow UI
```

Desktop install is the same flow with a GUI confirmation dialog instead of CLI.

### 4. Plugin registry (`nebula-plugin-registry`)

- **Scope:** a directory service, not a marketplace. Stores manifests, public keys, binary blob references, and basic metadata (downloads, declared SDK version, author).
- **API:** HTTP, lives under `nebula-api`. Endpoints:
  - `GET /plugins` — search/list
  - `GET /plugins/{key}/versions` — version list
  - `GET /plugins/{key}/{version}/manifest` — `plugin.toml`
  - `GET /plugins/{key}/{version}/binary` — signed binary stream
  - `POST /plugins` — authenticated publish
- **Client:** CLI + desktop app.
- **Not in Phase 4:** rating, comments, curation, payment, search ranking beyond simple text match.

### 5. Desktop install dialog

Tauri surface (`apps/desktop`). On plugin install, shows identity + signing status — **no capability list** because there isn't one:

```
┌──────────────────────────────────────────────┐
│ Install plugin: Telegram                     │
│ by alice@example.com                         │
│ version 1.2.3 — built with nebula-sdk 1.2.0  │
│                                              │
│ ✓ Signature verified (trusted key)           │
│ ✓ Binary hash matches                        │
│ ✓ SDK compatible with this nebula (1.0–1.5)  │
│                                              │
│ This plugin runs in a sandboxed process      │
│ and can make network requests, read/write    │
│ its scratch directory, and use credentials   │
│ you assign to it in workflows. All actions   │
│ are logged and can be reviewed later.        │
│                                              │
│            [Cancel]  [Install]               │
└──────────────────────────────────────────────┘
```

After install, plugin-management view lists installed plugins with version, author, signature status, install date, and uninstall button. **There is no per-plugin revoke** — you uninstall, which tears down the plugin's supervisor process entirely.

### 6. SBOM + dependency audit

- **On publish:** author submits plugin binary with an SBOM (CycloneDX JSON). Registry runs `cargo audit` / OSV.dev lookup at publish time, flags known CVEs on the plugin's detail page.
- **On install:** client re-verifies the SBOM signature and re-runs the audit locally for fresh data.
- **Not enforced:** publishing with CVEs is allowed, just labeled. Enforcement of CVE-free builds is a future policy decision.

### 7. macOS quarantine handling

- Downloaded plugin binaries hit `com.apple.quarantine` xattr → Gatekeeper blocks first launch.
- After manifest signature verification passes, `nebula plugin install` strips the xattr via `xattr -d com.apple.quarantine <binary>`. The nebula signature is the trust anchor, not Gatekeeper.
- Document this clearly — users who see plugins failing with "damaged and can't be opened" are hitting Gatekeeper, not nebula.

### 8. Windows install-time admin requirement

- The WFP firewall provider from Phase 3 needs admin registration **once**. The desktop installer handles it via a separate signed helper `nebula-firewall-helper.exe` that elevates with UAC prompt.
- Operators on Windows Server / domain-managed machines may need an MSI with explicit WDAC allowlist entries — document.
- **Smart App Control (SAC)** on fresh Windows 11 Home blocks unsigned plugin binaries outright via cloud reputation. Document as known incompatibility: operators must either add nebula's plugin directory to SAC exclusions (requires elevation) or disable SAC.

### 9. Observability for plugin lifecycle

- Metrics: `sandbox_plugin_installs_total{plugin, version, signature=trusted|unknown|invalid}`, `sandbox_plugin_uninstalls_total{plugin}`, `sandbox_plugin_load_failures_total{plugin, reason=sdk_incompat|sig_invalid|hash_mismatch|spawn_failed}`.
- Events on `EventBus`: `PluginInstalled`, `PluginUninstalled`, `PluginSignatureInvalid`, `PluginSdkIncompat`, `PluginSpawnFailed`.
- All broker RPCs are already logged from Phase 1 — operators can audit per-plugin activity post-hoc without grant machinery.

## Work breakdown

1. **Manifest schema + parser** — `nebula-sandbox::manifest` module with `semver` for sdk-version checks. 2 days.
2. **ed25519 signing library wiring** — `ed25519-dalek` crate; canonical TOML serialization helper. 2 days.
3. **Manifest verification integrated into `discovery::discover_plugin`** — replace Phase 0's minimal loader. 1 day.
4. **Binary hash verification** — checked at spawn time, not just install time. 1 day.
5. **`nebula-plugin-registry` service** — HTTP endpoints in `nebula-api`, storage backend (start with filesystem, plan for S3-compatible later). 5-7 days.
6. **CLI: `nebula plugin install`, `nebula plugin list`, `nebula plugin uninstall`, `nebula plugin info <key>`, `nebula plugin sign <path>`, `nebula plugin verify <path>`** — 3 days.
7. **Desktop install dialog** (Tauri) — 3-4 days (simpler than the deferred grant UI).
8. **macOS quarantine strip** — 1 day.
9. **Windows `nebula-firewall-helper.exe`** — elevated installer that registers WFP provider once. 2-3 days.
10. **SBOM generation + audit hook at publish** — 2-3 days.
11. **End-to-end integration test** — publish → install → run → uninstall, on Linux CI. Adversarial cases: invalid signature, sdk-mismatch, hash mismatch, corrupted manifest. 2 days.
12. **Docs** — plugin author guide, operator guide, desktop user guide. 2 days.

**Total:** ~25-32 working days. Reduced from original 30-40 by dropping permission grant flow.

## Acceptance criteria

- [ ] A plugin can be published to the registry with a valid ed25519 signature.
- [ ] `nebula plugin install` verifies sdk-version, signature, and binary hash; refuses on any mismatch with a clear error.
- [ ] Unknown-signer plugins in default mode show a confirmation dialog; in strict mode they're refused outright.
- [ ] Binary hash is re-verified on every plugin spawn (not just install).
- [ ] Desktop install dialog shows identity + signing status; no capability list.
- [ ] macOS install strips `com.apple.quarantine` after signature verification passes.
- [ ] Windows install registers the WFP provider once via elevated helper.
- [ ] SBOM + CVE report is attached to each plugin version.
- [ ] Metrics and events for install/uninstall/signature-failure/sdk-incompat/spawn-failure all wired.
- [ ] End-to-end integration test green on Linux CI (covers adversarial cases).
- [ ] Author, operator, and desktop user docs published under `docs/`.

## Risks

| Risk | Mitigation |
|---|---|
| Signing UX is painful for plugin authors | Ship `nebula plugin sign <path>` CLI that handles canonicalization + signature; provide a `new-plugin` template with signing pre-wired |
| Registry becomes a trust-shaped target | Minimal v1: we don't host binaries globally, just manifest directory + checksums; binary hosting is per-deployment |
| SDK-version churn breaks old plugins | Maintain a 6-month compat window: `SUPPORTED_SDK_RANGE` covers 3 minor versions back minimum. Document in SDK release notes. |
| CVE churn floods operators with alerts | Audit on publish + install, not continuously; operators opt-in to background re-audit |
| Install-time elevated helper (Windows) triggers UAC fatigue | One-time install, not per-plugin. Document clearly. |
| Without permission model, a compromised plugin can do anything within the sandbox | Scope restriction is sandbox-wide, not per-plugin. Defense is: (a) process isolation blocks FS/network/devices except through broker, (b) broker has anti-SSRF and audit log, (c) signed manifests verify supply chain, (d) uninstall is fast. If per-plugin scope becomes a real operator requirement, revisit in a later phase. |

## Out of scope (candidate future phases)

- **Permission manifest model** — deferred until real requirements (see roadmap §D4).
- Sigstore / transparency log.
- Plugin marketplace UI with ratings, search, featured plugins.
- Paid plugins / license enforcement.
- Org-level policy: "all plugins used by org X must be signed by keys in set Y".
- Runtime attestation ("the running plugin binary matches the signed manifest") — partially covered by `binary-sha256` check; full attestation is a future initiative.

## Final shipping gate

Before marking Phase 4 done, a **malicious plugin drill** is run by the security lead:

- Build a plugin that tries to (a) exfiltrate credentials to an attacker-controlled host, (b) read `/etc/passwd`, (c) fork-bomb the host, (d) OOM the host, (e) spawn a shell, (f) tamper with its own binary on disk to invalidate future signature checks.
- Sign it with a throwaway key added to trust anchors.
- Install it through the normal flow, run it against a sample workflow.
- Expected:
  - Credential exfiltration: anti-SSRF blocklist catches internal-IP targets; all attempts visible in audit log; public-IP exfiltration is possible (no scope enforcement) but **operator sees every RPC in audit log** within seconds.
  - `/etc/passwd` read: denied by landlock (Linux) / sandbox-exec (macOS) / AppContainer (Windows). SIGSYS or EACCES.
  - Fork-bomb: denied by seccomp + pids.max cgroup (Linux) / `process-fork` deny in sandbox-exec profile (macOS) / Job Object active-process limit (Windows).
  - OOM: cgroup memory.max kills plugin cleanly (Linux) / RLIMIT_AS (macOS) / Job Object memory limit (Windows). Host survives.
  - Shell spawn: denied by seccomp (`execve` not in allowlist) / sandbox-exec `process-exec` deny / AppContainer restricted token.
  - Binary tamper: detected at next spawn via `binary-sha256` mismatch; plugin fails to load.

If any of these fails, Phase 4 is not done — regardless of how many criteria above are checked. **Exfiltration-to-public-IP is a known limitation** and documented; operators who need stricter scope wait for a future permission-manifest phase.
