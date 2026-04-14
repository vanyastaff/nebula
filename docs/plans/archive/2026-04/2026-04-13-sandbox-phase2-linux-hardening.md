# Phase 2 — Linux hardening

**Parent roadmap:** [2026-04-13-sandbox-roadmap.md](2026-04-13-sandbox-roadmap.md)
**Status:** spec
**Estimated effort:** ~3-4 weeks
**Blocks on:** Phase 1
**Blocks:** Phase 4 (community plugins on Tier 1)

## Goal

Turn the Phase 1 broker from "correct" into "safe to run arbitrary code". On Linux we want a multi-layer jail: broker (app-layer), landlock (FS), seccomp-bpf (syscalls), user+net+pid namespaces (isolation), cgroups v2 (CPU/memory/pids/io), rlimits (backstop). A plugin that tries to read `/etc/passwd`, connect to `8.8.8.8`, fork a shell, or OOM the host must be killed or denied in every single case, with loud observability.

This phase only targets Linux (Tier 1). macOS and Windows happen in Phase 3.

## Non-goals

- WASI experiment (separate spike, non-blocking).
- Non-Linux OS enforcement (Phase 3).
- Plugin signing / registry / grant flow (Phase 4).

## Enforcement layers (target)

```
┌──────────────────────────────────────────────────────────────┐
│ CapabilityBroker (app-layer policy, Phase 1)                 │
│   allowlisted domains, credentials, fs paths, metered bytes  │
├──────────────────────────────────────────────────────────────┤
│ seccomp-bpf (Phase 2)                                        │
│   syscall allowlist: read/write/poll/epoll/futex/nanosleep/  │
│   sigreturn/exit/exit_group/brk/mmap(anon only)/munmap/      │
│   mprotect/clock_gettime/prctl(PR_SET_*)/getrandom. DENY     │
│   open/openat(*)/connect/socket/fork/clone/execve/ptrace/... │
├──────────────────────────────────────────────────────────────┤
│ landlock (Phase 0/1, extended in Phase 2)                    │
│   RO: plugin binary dir, tmpdir, read paths from capabilities│
│   RW: write paths from capabilities                          │
│   else: deny                                                 │
├──────────────────────────────────────────────────────────────┤
│ namespaces (Phase 2)                                         │
│   CLONE_NEWUSER — unprivileged user ns                       │
│   CLONE_NEWNET  — empty net ns (no interfaces except lo)     │
│   CLONE_NEWPID  — plugin is PID 1 in its ns                  │
│   CLONE_NEWIPC  — isolated System V IPC                      │
│   CLONE_NEWUTS  — hostname isolation                         │
├──────────────────────────────────────────────────────────────┤
│ cgroups v2 (Phase 2)                                         │
│   memory.max, memory.swap.max, cpu.max, pids.max, io.max     │
├──────────────────────────────────────────────────────────────┤
│ rlimits (Phase 0, tightened in Phase 2)                      │
│   RLIMIT_CPU, RLIMIT_AS, RLIMIT_NOFILE, RLIMIT_NPROC,        │
│   RLIMIT_FSIZE, RLIMIT_CORE=0                                │
└──────────────────────────────────────────────────────────────┘
```

Design principle: **every call the plugin makes must fail in at least two layers if it ever gets past one**.

## Confirmed Rust dependencies

From `.project/context/research/sandbox-prior-art.md`:

| Crate | Version | Role |
|---|---|---|
| `landlock` | 0.4.4 | FS LSM (5.13+) and net LSM (6.7+) |
| `seccompiler` | 0.5.0 | BPF filter compiler — Firecracker's, the 2025 standard |
| `cgroups-rs` | 0.5.0 | cgroup v2 — kata-containers-owned |
| `rustix` | 1.1.4 | `unshare`, `pivot_root`, `setrlimit`, `prctl`, `pidfd`, `mount` |
| `caps` | 0.5.6 | drop Linux capabilities pre-exec |

**Reference reading (not dependencies):** `hakoniwa` 1.5.0 and `youki/libcontainer` — both get the Linux setup ordering right (`unshare` → `pivot_root` → mount plan → cgroup attach → landlock → seccomp → caps drop → exec). We implement our own wiring against the same primitives, matching our policy format exactly, but we follow their order of operations. Reserve the right to pivot to `hakoniwa` as a direct dependency if our wiring grows bug-laden.

## Key design decisions

### D1. seccomp-bpf default is deny-all
Plugin gets a whitelist of ~30 syscalls, built from the actual set tokio + mio + reqwest's rustls/hyper backend need for stdio-only operation. Anything else → `SIGSYS`. Tested via an adversarial plugin that tries every blocked syscall.

### D2. Network namespace is empty
Plugin has **only `lo`**. All outbound HTTP goes through the broker (Phase 1). This makes SSRF structurally impossible at the OS layer, not just policy-checked.

### D3. cgroups v2 only, no v1 fallback
Kernel 5.13+ is already required by landlock. cgroups v2 has been default since kernel 5.8. If the host lacks cgroup v2 delegation (e.g., a systemd slice isn't set up), engine startup fails loudly with a remediation hint. No silent downgrade.

### D4. Process pool, not per-call spawn
A plugin's cold-start cost (spawn + `pre_exec` + seccomp load + ns setup) is measured in tens of milliseconds. Per-call is fine for batch, not for user-facing sync workflows. Pool design:
- One `Supervisor` per plugin binary + capability tuple.
- Supervisor keeps N warm processes (configurable, default 2).
- Each warm process is a fresh jail — the supervisor doesn't reuse a jail across *different* invocations. Instead, on a pool miss we spawn a new one; on a pool hit we hand out a warm one that has **never been used**. After one use the process exits and a replacement is spawned.
- This trades memory for cold-start; revisit if memory pressure is real.
- **Explicitly not implemented:** shared long-lived plugin processes that service multiple invocations. Too risky (state leaks between invocations).

### D5. Kernel capability detection is upfront
Engine boot runs a capability probe: landlock available? seccomp available? user namespaces unprivileged? cgroup v2 delegated? If any required capability is missing, startup fails with a clear message. No per-spawn "maybe it works" — that's how you ship broken guarantees.

## Work breakdown

### Step 2.1 — seccomp-bpf filter
- **Crate:** `seccompiler` (maintained, used by firecracker). Add to `[target.'cfg(target_os = "linux")'.dependencies]`.
- **Where:** `crates/sandbox/src/os_sandbox.rs`, extend `apply_sandbox`.
- **Filter build:** allowlist-based `SeccompFilter` targeting the minimum set of syscalls tokio + broker stdio need. Start from a known-good firecracker profile and strip.
- **Loaded at:** `pre_exec`, after landlock, before exec.
- **Failure mode:** `SIGSYS` (kill). Host observes via `waitpid` and emits `SandboxPluginKilled { reason: seccomp_violation }`.
- **Tests:** adversarial plugin calling `open`, `connect`, `fork`, `execve`, `ptrace`, `socket`. Assert all killed with the right signal.

### Step 2.2 — Namespaces
- **Crate:** `nix` (already present).
- **Where:** `os_sandbox::apply_sandbox`, wrap the whole thing in `unshare(CLONE_NEWUSER | CLONE_NEWNET | CLONE_NEWPID | CLONE_NEWIPC | CLONE_NEWUTS)` during `pre_exec`.
- **Failure mode:** plugin binary can't resolve names, can't connect to anything, can't see other PIDs.
- **Complication:** `CLONE_NEWUSER` must happen before the others (unprivileged case). Requires careful uid_map/gid_map setup. Reference: `user_namespaces(7)`.
- **Tests:** adversarial plugin tries `hostname`, listing `/proc/*/status`, binding to a socket. Assert all fail.

### Step 2.3 — cgroups v2
- **Crate:** `cgroups-rs` (or raw `cgroup2` via direct file writes to `/sys/fs/cgroup`).
- **Where:** new module `crates/sandbox/src/cgroups.rs`.
- **Lifecycle:** before spawn, create a cgroup under `/sys/fs/cgroup/nebula/<plugin_uuid>/`. Write `memory.max`, `cpu.max`, `pids.max`, `io.max`. Move the child into it immediately after spawn (there is a small race window between `fork` and `cgroup attach`; mitigated by having seccomp + rlimits as backstops).
- **Limits (defaults):** 256 MiB memory, 50% of one core, 16 pids, 10 MB/s io write.
- **Teardown:** on child exit, remove the cgroup.
- **Failure mode:** OOM → cgroup kills, emits `SandboxPluginKilled { reason: oom }`.
- **Tests:** adversarial plugin allocates until OOM. Assert it's killed cleanly and the host survives.

### Step 2.4 — Landlock path set expansion
- **Where:** `os_sandbox.rs` existing landlock setup.
- **What:** currently limited; expand RO set to include plugin binary dir, `/tmp` via fresh `tmpfs` mount (namespace-level), RW limited to paths in capabilities.
- **Tmpfs:** mount `tmpfs` at `/tmp` inside the mount namespace — each plugin gets a fresh, size-capped scratch space that disappears on exit. Requires mount namespace (CLONE_NEWNS).
- **Tests:** plugin tries `/etc/passwd`, `/proc/self/maps`, `/var/log`. Assert denied.

### Step 2.5 — Kernel capability probe at boot
- **Where:** new `crates/sandbox/src/probe.rs`, called by `ActionRuntime::new` (or engine boot).
- **Checks:** landlock version, seccomp availability, unprivileged user namespaces (`/proc/sys/kernel/unprivileged_userns_clone`), cgroup v2 delegation.
- **Fail mode:** return `SandboxSupportError { missing: Vec<String>, remediation: String }`. Engine startup converts to a fatal startup error.

### Step 2.6 — Process pool
- **Where:** new `crates/sandbox/src/pool.rs`.
- **Semantics:** `Supervisor` per (plugin_key, capabilities_hash). Warm count configurable. `acquire()` returns a fresh process; `release()` discards it. Processes never outlive one invocation. This is not traditional pooling — it's a prewarmed cold-start mitigation.
- **Alternative considered:** long-lived plugin processes servicing many invocations. Rejected — too much shared state risk. Reconsider only if pool-based approach proves prohibitively expensive.

### Step 2.7 — Adversarial test suite
- **Where:** `crates/sandbox/tests/adversarial_linux.rs` (gated `#[cfg(target_os = "linux")]`).
- **Binary:** a single adversarial plugin at `examples/sandbox-adversary/` that takes an `attack` parameter and tries each attack in turn:
  - `read_etc_passwd` — expect seccomp+landlock deny
  - `connect_public_ip` — expect seccomp+netns deny
  - `fork_shell` — expect seccomp+pids deny
  - `oom_allocate` — expect cgroups OOM kill
  - `cpu_spin` — expect cpu.max throttle + eventual timeout
  - `fd_exhaustion` — expect RLIMIT_NOFILE
  - `dns_rebinding` — expect broker-level pinned resolution
  - `credential_exfil_via_stderr` — expect stderr size cap + sanitization
- **Test matrix:** runs each attack and asserts the host stays healthy, the right kill reason is observed, and metrics increment.

### Step 2.8 — Metrics + events for all the above
- Wire `SandboxPluginKilled { reason }` into the `EventBus` for every kill path.
- Ensure `sandbox_plugin_oom_total`, `sandbox_plugin_timeout_total`, `sandbox_plugin_killed_total{reason}` all populated.

### Step 2.9 — Extended RPC verbs
Finish the Phase 1 deferred verbs under the broker:
- `fs.read { path }`, `fs.write { path, bytes }` — policy-checked against `check_fs_read` / `check_fs_write`, then executed **by the broker** (landlock RO paths are defense-in-depth; the primary check is the broker). File handles never leak to the plugin.
- `resource.acquire { key }` — returns a `ResourceRef` handle (same pattern as `CredentialRef`).
- `event.emit` — emits to `EventBus` with plugin-key namespace prefix enforced by broker.
- `system.info` — returns a heavily-redacted struct (no IPs, no hostnames, just a platform name).

## Acceptance criteria

- [ ] Every adversarial attack in the test suite is killed or denied on Linux kernel 5.13+ with the expected reason.
- [ ] Engine startup fails cleanly on a host without cgroup v2 delegation, with a remediation hint.
- [ ] Process pool reduces cold-start from ~30ms to <5ms for a hot plugin (benchmark).
- [ ] `tmpfs` `/tmp` is fresh per plugin and capped in size.
- [ ] Metrics dashboard (manual check with Jaeger/OTEL via `task obs:up`) shows per-plugin RPC call counts, byte budgets, and kill reasons.
- [ ] `fs.read`/`fs.write`/`resource.acquire`/`event.emit` RPC verbs implemented and tested.
- [ ] `cargo nextest run --workspace` green on Linux CI, `cargo clippy --workspace -- -D warnings` clean, `cargo deny check` clean.
- [ ] `.project/context/crates/sandbox.md` updated with Tier 1 guarantees table.

## Risks

| Risk | Mitigation |
|------|------------|
| seccomp filter too tight, breaks tokio runtime | Build filter from a real, profiled tokio/reqwest plugin; add strace-based test |
| User namespaces disabled on the host (some hardened kernels) | Probe at boot, fail loudly, document remediation |
| cgroups v2 delegation missing in non-systemd environments | Probe at boot, document systemd slice config |
| Process pool memory pressure with many plugins | Bound pool size globally, eager drop on idle |
| Adversarial tests flake in CI | Single-threaded, longer timeouts, isolated in their own nextest profile |
| Landlock breaking changes between kernels | Pin `landlock` crate version, CI on multiple kernel versions |
| Namespace setup interferes with reqwest DNS | Broker is host-side — the plugin doesn't need DNS — netns is empty on purpose |

## Tier 1 guarantees statement (to document after landing)

On Linux ≥ 5.13 with cgroup v2, unprivileged user namespaces, and seccomp enabled, a nebula-sandboxed plugin **cannot**:

1. Read any file outside its scratch dir or broker-mediated `fs.read` call (defended by: landlock, seccomp, broker sanity checks).
2. Connect to any private / localhost / link-local address (defended by: broker anti-SSRF blocklist, netns, seccomp). Note: connecting to **public internet hosts** is allowed by default — scope restriction is deferred (see roadmap §D4). Protection is audit log + anti-SSRF + process isolation, not per-host allowlist.
3. Fork, exec, or spawn any process (defended by: seccomp, RLIMIT_NPROC, pids.max).
4. Exhaust host memory (defended by: cgroup memory.max, RLIMIT_AS).
5. Consume more than its CPU share (defended by: cgroup cpu.max, RLIMIT_CPU).
6. See other processes on the host (defended by: PID namespace).
7. Observe or modify hostnames, UTS, IPC (defended by: namespaces).
8. Read credentials in raw form (defended by: broker `CredentialRef` indirection).
9. Escape its filesystem scratch space (defended by: tmpfs in mount ns + landlock).
10. Survive cooperative cancellation + kill window (defended by: broker cancel, SIGKILL, kill_on_drop).

Any counterexample is a P0 security bug.
